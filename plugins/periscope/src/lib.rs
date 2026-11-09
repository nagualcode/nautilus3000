//! PeriScope 3K — the 32-band phase-correct zoomable equaliser of Audio
//! Ease's Nautilus Bundle (2000), rebuilt from the original binary.
//!
//! The engine (`engine.rs`) is a translation of the i386 slice of
//! `Periscope.vst` 2.6.4 and renders bit-identical output to that machine
//! code (see `reverse_engineering/periscope/`).

pub mod engine;
pub mod params;
pub mod ui;

use engine::{Periscope, BANDS};
use mission::denormal::DenormalFlush;
use params::{PeriscopeParams, P, SCOPE};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use truce::prelude::*;
use truce_egui::EguiEditor;

/// A value set by MIDI or by the engine, in force until the host parameter
/// moves away from what it was when the override was made.
#[derive(Clone, Copy)]
struct Override {
    value: f32,
    param: f64,
}

pub struct PeriscopeDsp {
    engine: Box<Periscope>,
    sample_rate: f64,
    buffer: usize,
    io: Vec<Vec<f32>>,
    /// 0…31 faders, 32 low needle, 33 high needle.
    ovr: [Option<Override>; BANDS + 2],
    /// Fader values saved by a note-on, restored by its note-off
    /// (`this + 0xac + note·4` in the original).
    saved: [f32; 128],
}

impl PeriscopeDsp {
    fn new(sample_rate: f64, channels: usize, buffer: usize) -> Self {
        Self {
            engine: Box::new(Periscope::new(sample_rate, channels, buffer)),
            sample_rate,
            buffer,
            io: vec![vec![0.0; buffer]; channels],
            ovr: [None; BANDS + 2],
            saved: [135.0; 128],
        }
    }

    fn param_value(params: &PeriscopeParams, i: usize) -> f64 {
        match i {
            32 => params.low.value() as f64,
            33 => params.high.value() as f64,
            k => params.fader(k).value() as f64,
        }
    }

    /// The value in force for slot `i`.
    fn value(&mut self, params: &PeriscopeParams, i: usize) -> f32 {
        let p = Self::param_value(params, i);
        match self.ovr[i] {
            Some(o) if o.param == p => o.value,
            _ => {
                self.ovr[i] = None;
                p as f32
            }
        }
    }

    fn set(&mut self, params: &PeriscopeParams, i: usize, v: f32) {
        let (lo, hi) = match i {
            32 => (5.0, 408.0),
            33 => (175.0, 412.0),
            _ => (0.0, 265.0),
        };
        let v = v.clamp(lo, hi);
        let v = if i < BANDS { v.trunc() } else { v };
        self.ovr[i] = Some(Override { value: v, param: Self::param_value(params, i) });
        params.status.pending.fetch_add(1, Ordering::Relaxed);
    }

    /// `ReceiveMidi` @ 0x1a4e4 (and the 2.0.2 manual).
    fn midi(&mut self, params: &PeriscopeParams, ev: &EventBody) {
        // Notes are folded into 60…95: one octave… three octaves of faders.
        let fold = |n: u8| {
            let mut n = n as i32;
            while n < 0x3c {
                n += 0x24;
            }
            while n > 0x5f {
                n -= 0x24;
            }
            (n - 0x3c) as usize
        };
        match *ev {
            EventBody::NoteOn { note, velocity, .. } if velocity > 0 => {
                let k = fold(note);
                if k < BANDS {
                    self.saved[note as usize & 0x7f] = self.value(params, k);
                    self.set(params, k, velocity_to_fader(velocity));
                }
            }
            EventBody::NoteOn { note, .. } | EventBody::NoteOff { note, .. } => {
                let k = fold(note);
                if k < BANDS {
                    self.set(params, k, self.saved[note as usize & 0x7f]);
                }
            }
            EventBody::ControlChange { cc, value, .. } => {
                let v = value as f32;
                match cc {
                    // CC 32…63: faders 1…32.
                    0x20..=0x3f => self.set(params, cc as usize - 0x20, (v * 265.0 / 127.0).round()),
                    // CC 64 / 65: the needles (`SetNormalized(value / 128)`).
                    0x40 => self.set(params, 32, 5.0 + v * 0.007_812_5 * 403.0),
                    0x41 => self.set(params, 33, 175.0 + v * 0.007_812_5 * 237.0),
                    _ => {}
                }
            }
            EventBody::PitchBend { value, .. } => {
                // Both needles shift as a pair, by the headroom in the
                // direction of the bend (each message moves them again).
                let f = (value as f32 - 8192.0) * 0.000_122_070_31;
                let (lo, hi) = (self.value(params, 32), self.value(params, 33));
                let room = if f < 0.0 { lo - 5.0 } else { 412.0 - hi };
                self.set(params, 32, lo + f * room);
                self.set(params, 33, f * room + hi);
            }
            _ => {}
        }
    }
}

/// A note's velocity as a fader position.
fn velocity_to_fader(v: u8) -> f32 {
    (v as f32 * 265.0 / 127.0).round()
}

impl Default for PeriscopeDsp {
    fn default() -> Self {
        Self::new(44100.0, 2, 512)
    }
}

pub struct PeriscopePlugin;

impl PluginLogic for PeriscopePlugin {
    type Params = PeriscopeParams;
    type DspState = PeriscopeDsp;

    fn bus_layouts() -> Vec<BusLayout> {
        // "always choose a channel configuration of Periscope that has as
        // many inputs as outputs" — the manual.
        vec![BusLayout::stereo(), BusLayout::mono()]
    }

    fn init(_params: &Self::Params, _cx: &InitContext) -> Self::DspState {
        PeriscopeDsp::default()
    }

    fn reset(state: &mut Self::DspState, params: &Self::Params, config: &AudioConfig) {
        let ch = state.io.len().max(1);
        *state = PeriscopeDsp::new(config.sample_rate, ch, config.max_block_size.max(32));
        params.status.sample_rate.store(config.sample_rate as u32, Ordering::Relaxed);
    }

    /// `GetLatency` said 3072 whatever the buffer; this is the real delay
    /// (3072 + buffer − 2048 for host buffers above 2048).
    fn latency(state: &Self::DspState) -> u32 {
        state.engine.latency()
    }

    fn process(
        state: &mut Self::DspState,
        params: &Self::Params,
        buffer: &mut AudioBuffer,
        events: &EventList,
        _context: &mut ProcessContext,
    ) -> ProcessStatus {
        let n = buffer.num_samples();
        if n == 0 {
            return ProcessStatus::Normal;
        }
        let _ftz = DenormalFlush::on();
        let ch = buffer.num_output_channels().max(1);
        if ch != state.engine.channels() || n > state.buffer {
            // Only on a layout change (the original had one process per variant).
            *state = PeriscopeDsp::new(state.sample_rate, ch, state.buffer.max(n));
        }
        for ev in events.iter() {
            state.midi(params, &ev.body);
        }

        let mut c = params.controls();
        for k in 0..BANDS {
            c.faders[k] = state.value(params, k) as i32;
        }
        c.low = state.value(params, 32);
        c.high = state.value(params, 33);
        let (lo, hi) = (c.low, c.high);

        let nin = buffer.num_input_channels();
        for (i, io) in state.io.iter_mut().enumerate() {
            let src = if nin == 0 { None } else { Some(buffer.input(i.min(nin - 1))) };
            for (k, v) in io[..n].iter_mut().enumerate() {
                *v = src.map_or(0.0, |s| s[k]);
            }
        }
        {
            let PeriscopeDsp { engine, io, .. } = state;
            let mut bufs: [&mut [f32]; 8] = Default::default();
            let mut m = 0;
            for v in io.iter_mut().take(8) {
                bufs[m] = &mut v[..n];
                m += 1;
            }
            engine.process(&mut c, &mut bufs[..m]);
        }
        // Needles pushed apart by `UpdateFreqs`: the original sends them to
        // the host as automation; here they stay in force until the editor
        // (or the host) writes them into the parameters.
        if c.low != lo {
            state.set(params, 32, c.low);
        }
        if c.high != hi {
            state.set(params, 33, c.high);
        }

        let st = &params.status;
        let scale = 1.0 / (ch as f32).sqrt();
        let pos = st.scope_pos.load(Ordering::Relaxed);
        for k in 0..n {
            let mut s = 0.0;
            for (i, io) in state.io.iter().enumerate() {
                buffer.output(i)[k] = io[k];
                s += io[k];
            }
            st.scope[(pos + k) % SCOPE].store((s * scale).to_bits(), Ordering::Relaxed);
        }
        st.scope_pos.store((pos + n) % SCOPE, Ordering::Relaxed);
        st.low.store(c.low.to_bits(), Ordering::Relaxed);
        st.high.store(c.high.to_bits(), Ordering::Relaxed);
        for k in 0..BANDS {
            st.faders[k].store(c.faders[k] as u32, Ordering::Relaxed);
            st.freqs[k].store(state.engine.filter.freqs[k].to_bits(), Ordering::Relaxed);
        }
        _context.set_meter(P::MeterL, buffer.output_peak(0));
        _context.set_meter(P::MeterR, buffer.output_peak(if ch > 1 { 1 } else { 0 }));
        ProcessStatus::Normal
    }

    fn editor(params: Arc<Self::Params>) -> Box<dyn Editor> {
        EguiEditor::new(params.clone(), (ui::WINDOW_W, ui::WINDOW_H), ui::periscope_ui)
            .with_visuals(truce_egui::theme::dark())
            .resizable(true)
            .min_size((ui::MIN_W, ui::MIN_H))
            .max_size((ui::MAX_W, ui::MAX_H))
            .into_editor()
    }
}

truce::plugin! {
    logic: PeriscopePlugin,
    params: PeriscopeParams,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use truce_test::{assertions, driver, InputSource};

    #[test]
    fn silence_in_silence_out() {
        let result = driver!(Plugin)
            .sample_rate(44_100.0)
            .channels(2)
            .block_size(256)
            .duration(Duration::from_millis(500))
            .input(InputSource::Silence)
            .run();
        assertions::assert_no_nans(&result);
        assertions::assert_silence(&result);
    }

    #[test]
    fn passes_a_signal_after_the_latency() {
        let result = driver!(Plugin)
            .sample_rate(48_000.0)
            .channels(2)
            .block_size(333)
            .duration(Duration::from_millis(1000))
            .input(InputSource::Generator(Box::new(|i, sr| (i as f64 * 440.0 * std::f64::consts::TAU / sr).sin() as f32 * 0.5)))
            .run();
        assertions::assert_no_nans(&result);
        assertions::assert_nonzero(&result);
    }
}
