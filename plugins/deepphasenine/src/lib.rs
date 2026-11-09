//! Deep Phase Nine — the 24-notch phaser of Audio Ease's Nautilus Bundle
//! (2000), rebuilt from the original binary.
//!
//! The engine (`engine.rs`) is a translation of the i386 slice of
//! `DeepPhaseNine.vst` 2.6.4 and renders bit-identical output to that machine
//! code (see `reverse_engineering/deepphasenine/`).

pub mod engine;
pub mod params;
pub mod ui;

use engine::{DeepPhaseNine, HostTime, Layout};
use mission::agc::LOOKAHEAD;
use params::{DpnParams, DpnParamsParamId as P};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use truce::prelude::*;
use truce_egui::EguiEditor;

/// The limiter's look-ahead, reported like `GetLatency` @ 0x1c77a.
pub const LATENCY: u32 = LOOKAHEAD as u32;

pub struct DpnDsp {
    engine: Box<DeepPhaseNine>,
    sample_rate: f64,
    last_playing: bool,
    l: Vec<f32>,
    r: Vec<f32>,
    /// Notch count set by a MIDI note, valid until the parameter moves.
    notes: Option<(i32, i64)>,
}

impl Default for DpnDsp {
    fn default() -> Self {
        Self {
            engine: Box::new(DeepPhaseNine::new(44100.0, Layout::Stereo)),
            sample_rate: 44100.0,
            last_playing: false,
            l: vec![0.0; 4096],
            r: vec![0.0; 4096],
            notes: None,
        }
    }
}

/// `DenormalFlush` for the duration of a block.
struct Ftz(bool);
impl Ftz {
    fn on() -> Self {
        Ftz(engine::flush_denormals(true))
    }
}
impl Drop for Ftz {
    fn drop(&mut self) {
        engine::flush_denormals(self.0);
    }
}

pub struct DpnPlugin;

impl PluginLogic for DpnPlugin {
    type Params = DpnParams;
    type DspState = DpnDsp;

    fn bus_layouts() -> Vec<BusLayout> {
        vec![
            BusLayout::stereo(),
            BusLayout::new().with_input("Main", ChannelConfig::Mono).with_output("Main", ChannelConfig::Stereo),
            BusLayout::mono(),
        ]
    }

    fn init(_params: &Self::Params, _cx: &InitContext) -> Self::DspState {
        DpnDsp::default()
    }

    fn reset(state: &mut Self::DspState, params: &Self::Params, config: &AudioConfig) {
        state.sample_rate = config.sample_rate;
        state.engine = Box::new(DeepPhaseNine::new(config.sample_rate, state.engine.layout));
        let n = config.max_block_size.max(64);
        state.l = vec![0.0; n];
        state.r = vec![0.0; n];
        params.status.sample_rate.store(config.sample_rate as u32, Ordering::Relaxed);
    }

    fn latency(_state: &Self::DspState) -> u32 {
        LATENCY
    }

    fn process(
        state: &mut Self::DspState,
        params: &Self::Params,
        buffer: &mut AudioBuffer,
        events: &EventList,
        context: &mut ProcessContext,
    ) -> ProcessStatus {
        let n = buffer.num_samples();
        if n == 0 {
            return ProcessStatus::Normal;
        }
        let _ftz = Ftz::on();
        let nin = buffer.num_input_channels();
        let nout = buffer.num_output_channels();
        let layout = match (nin, nout) {
            (_, 1) => Layout::Mono,
            (1, _) => Layout::MonoToStereo,
            _ => Layout::Stereo,
        };
        if layout != state.engine.layout {
            // Only on a layout change (the original had one process per variant).
            state.engine = Box::new(DeepPhaseNine::new(state.sample_rate, layout));
        }

        // MIDI as in 2.0.3 (`ReceiveMidi` @ 0x1c5a6): notes pick the notch
        // count; the CCs are left to the host's MIDI learn.
        for ev in events.iter() {
            if let EventBody::NoteOn { note, velocity, .. } = ev.body
                && velocity > 0
            {
                state.notes = Some((note as i32 % 24 + 1, params.notches.value()));
            }
        }

        // The editor holding the ball.
        let st = &params.status;
        let pos = f32::from_bits(st.drag_pos.load(Ordering::Relaxed));
        state.engine.hold(if pos >= 0.0 { Some((pos, st.drag_dir.load(Ordering::Relaxed))) } else { None });

        let tr = context.transport;
        let host = HostTime {
            playing: tr.playing,
            bpm: tr.tempo,
            ppq: tr.position_beats,
            transport_changed: tr.playing != state.last_playing,
        };
        state.last_playing = tr.playing;
        let mut c = params.controls();
        match state.notes {
            Some((nn, at)) if at == params.notches.value() => c.notches = nn,
            _ => state.notes = None,
        }

        if state.l.len() < n {
            state.l.resize(n, 0.0);
            state.r.resize(n, 0.0);
        }
        let (l, r) = (&mut state.l[..n], &mut state.r[..n]);
        let a = if nin > 0 { buffer.input(0) } else { &[][..] };
        for i in 0..n {
            l[i] = a.get(i).copied().unwrap_or(0.0);
        }
        if nin > 1 {
            r.copy_from_slice(buffer.input(1));
        } else {
            r.copy_from_slice(l);
        }
        state.engine.process(&c, &host, l, r);

        for i in 0..n {
            let g = 10f32.powf(params.output.read() / 20.0);
            buffer.output(0)[i] = l[i] * g;
            if nout > 1 {
                buffer.output(1)[i] = r[i] * g;
            }
        }

        let e = &state.engine;
        st.phase.store(e.store.lfo_pos.to_bits(), Ordering::Relaxed);
        st.dir.store(e.phaser.dir, Ordering::Relaxed);
        st.freq_l.store(e.phaser.freq.to_bits(), Ordering::Relaxed);
        st.freq_r.store(e.phaser.freq_right(&e.store).to_bits(), Ordering::Relaxed);
        st.limiter.store(e.limiter_gain().to_bits(), Ordering::Relaxed);

        context.set_meter(P::MeterL, buffer.output_peak(0));
        context.set_meter(P::MeterR, buffer.output_peak(if nout > 1 { 1 } else { 0 }));
        ProcessStatus::Normal
    }

    fn editor(params: Arc<Self::Params>) -> Box<dyn Editor> {
        EguiEditor::new(params.clone(), (ui::WINDOW_W, ui::WINDOW_H), ui::dpn_ui)
            .with_visuals(truce_egui::theme::dark())
            .resizable(true)
            .min_size((ui::MIN_W, ui::MIN_H))
            .max_size((ui::MAX_W, ui::MAX_H))
            .into_editor()
    }
}

truce::plugin! {
    logic: DpnPlugin,
    params: DpnParams,
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
    fn phases_a_signal() {
        let result = driver!(Plugin)
            .sample_rate(48_000.0)
            .channels(2)
            .block_size(333)
            .duration(Duration::from_millis(1500))
            .input(InputSource::Generator(Box::new(|i, sr| (i as f64 * 440.0 * std::f64::consts::TAU / sr).sin() as f32 * 0.5)))
            .run();
        assertions::assert_no_nans(&result);
        assertions::assert_nonzero(&result);
    }
}
