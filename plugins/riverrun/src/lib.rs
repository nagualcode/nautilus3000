//! Riverrun — the real-time granular synthesizer of Audio Ease's Nautilus
//! Bundle (2000), rebuilt from the original binary.
//!
//! The engine (`engine/`) is a translation of the i386 slice of
//! `Riverrun.vst` 2.6.4 and renders bit-identical output to that machine code
//! (see `reverse_engineering/riverrun/`). This file is the plug-in glue:
//! parameters → engine controls, the fixed "grain clock", MIDI, the snippet
//! state, dry/wet and the status the editor draws.

pub mod audio;
pub mod demo;
pub mod engine;
pub mod params;
pub mod presets;
pub mod snippet;
pub mod ui;

use engine::agc::LOOKAHEAD;
use engine::{BeatSync, Controls, Engine, HostTime};
use params::{GrainMode, RiverrunParams, RiverrunParamsParamId as P, GRAIN_TRAIL};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use truce::prelude::*;
use truce_egui::EguiEditor;

/// The plug-in reports the AGC look-ahead as latency, like `GetLatency` @ 0x20022.
pub const LATENCY: u32 = LOOKAHEAD as u32;

/// Snapshot of the input screen published for the editor/project every this
/// many samples while recording.
const MIRROR_EVERY: usize = 4096;

/// A value set from MIDI, valid until the parameter itself moves.
#[derive(Clone, Copy)]
struct Override {
    value: f32,
    param_at: f32,
}

fn take(o: &mut Option<Override>, param: f32) -> f32 {
    match *o {
        Some(ov) if ov.param_at == param => ov.value,
        _ => {
            *o = None;
            param
        }
    }
}

pub struct RiverrunDsp {
    engine: Box<Engine>,
    sample_rate: f64,
    cell: usize,
    cell_left: usize,
    wet_l: Vec<f32>,
    wet_r: Vec<f32>,
    dry: [Vec<f32>; 2],
    dry_pos: usize,
    last_glass_param: f32,
    glass_seq: u32,
    clear_seq: u32,
    snippet_gen: u64,
    was_recording: bool,
    since_mirror: usize,
    last_playing: bool,
    cc: [Option<Override>; 9],
    envelope_override: Option<(i32, u32)>,
}

impl Default for RiverrunDsp {
    fn default() -> Self {
        Self::new(44100.0, 4096)
    }
}

impl RiverrunDsp {
    fn new(sample_rate: f64, max_block: usize) -> Self {
        Self {
            engine: Box::new(Engine::new(sample_rate)),
            sample_rate,
            cell: 512,
            cell_left: 0,
            wet_l: vec![0.0; max_block],
            wet_r: vec![0.0; max_block],
            dry: [vec![0.0; LOOKAHEAD], vec![0.0; LOOKAHEAD]],
            dry_pos: 0,
            last_glass_param: f32::NAN,
            glass_seq: 0,
            clear_seq: 0,
            snippet_gen: u64::MAX,
            was_recording: false,
            since_mirror: 0,
            last_playing: false,
            cc: [None; 9],
            envelope_override: None,
        }
    }

    /// The parameters, with MIDI overrides, in the original's units.
    fn controls(&mut self, p: &RiverrunParams) -> Controls {
        let envelope = p.envelope.index() as i32 + 1;
        let envelope = match self.envelope_override {
            Some((v, at)) if at == p.envelope.index() => v,
            _ => {
                self.envelope_override = None;
                envelope
            }
        };
        Controls {
            mono_stereo: take(&mut self.cc[0], p.mono_stereo.value()),
            needle_l: take(&mut self.cc[1], p.needle_l.value()),
            needle_r: take(&mut self.cc[2], p.needle_r.value()),
            walk: take(&mut self.cc[3], p.walk.value()),
            speed: take(&mut self.cc[4], p.speed.value()),
            pitch: take(&mut self.cc[5], p.pitch.value()),
            random: take(&mut self.cc[6], p.random.value()),
            speed_random: take(&mut self.cc[7], p.speed_random.value()),
            length: take(&mut self.cc[8], p.length.value()),
            record: p.record.value(),
            envelope,
            grid: p.grid.index() as i32 + 1,
            rhythmic: p.mode.value() == GrainMode::Rhythmic,
            fine: p.fine.value(),
        }
    }

    /// `ReceiveMidi` @ 0x1fbf2: CC 1, 11–18 set a control to
    /// `min + cc·(max−min)/128`; a note moves the Grain Glass to
    /// `((note − 36) mod 60) / 60` of the screen; a program change picks the
    /// envelope. (CC 19, Record, is left to the host's own MIDI learn.)
    fn midi(&mut self, p: &RiverrunParams, events: &EventList) {
        for ev in events.iter() {
            match ev.body {
                EventBody::ControlChange { cc, value, .. } => {
                    let (slot, param, lo, hi) = match cc {
                        1 => (0, p.mono_stereo.value(), 0.0, 93.0),
                        11 => (1, p.needle_l.value(), 0.0, 472.0),
                        12 => (2, p.needle_r.value(), 0.0, 472.0),
                        13 => (3, p.walk.value(), -5.0, 5.0),
                        14 => (4, p.speed.value(), 1.0, 286.0),
                        15 => (5, p.pitch.value(), 0.0, 1000.0),
                        16 => (6, p.random.value(), 0.0, 100.0),
                        17 => (7, p.speed_random.value(), 0.0, 286.0),
                        18 => (8, p.length.value(), 1.0, 286.0),
                        _ => continue,
                    };
                    let v = (lo as f64 + ((value as f32 * (hi - lo)) as f64) * 0.0078125) as f32;
                    self.cc[slot] = Some(Override { value: v, param_at: param });
                }
                EventBody::NoteOn { note, velocity, .. } if velocity > 0 => {
                    let k = ((note as i32 - 36) % 60) as i16 as f64;
                    let v = (0.0 + (472.0f32 - 0.0) as f64 * (k / 60.0)) as f32;
                    self.engine.set_glass(v);
                }
                EventBody::ProgramChange { program, .. } => {
                    self.envelope_override = Some(((program & 3) as i32 + 1, p.envelope.index()));
                }
                _ => {}
            }
        }
    }

    fn requests(&mut self, p: &RiverrunParams) {
        let slot = &p.snippet;
        let generation = slot.generation();
        if generation != self.snippet_gen {
            if let Some(req) = slot.try_request() {
                self.snippet_gen = generation;
                if let Some(data) = req {
                    self.engine.load_snippet_at(&data.samples, data.sample_rate);
                    self.publish_mirror(p);
                }
            }
        }
        let st = &p.status;
        let clear = st.clear_req.load(Ordering::Acquire);
        if clear != self.clear_seq {
            self.clear_seq = clear;
            self.engine.clear_snippet();
            self.publish_mirror(p);
        }
        let seq = st.glass_req_seq.load(Ordering::Acquire);
        if seq != self.glass_seq {
            self.glass_seq = seq;
            self.engine.set_glass(st.glass_req.load(Ordering::Relaxed) as f32 / 100.0);
        }
        let gp = p.glass.value();
        if gp != self.last_glass_param {
            if !self.last_glass_param.is_nan() {
                self.engine.set_glass(gp);
            }
            self.last_glass_param = gp;
        }
    }

    /// Copies the input screen for the editor and the project (`try_lock`:
    /// skipped if the editor is reading it right now).
    fn publish_mirror(&mut self, p: &RiverrunParams) {
        if let Ok(mut m) = p.snippet.mirror.try_lock() {
            if m.samples.capacity() >= self.engine.snapshot_len() {
                self.engine.copy_snippet_into(&mut m.samples);
                m.sample_rate = self.sample_rate;
                drop(m);
                p.snippet.bump_mirror();
                self.since_mirror = 0;
            }
        }
    }

    fn publish(&self, p: &RiverrunParams) {
        let st = &p.status;
        let e = &self.engine;
        st.glass_px.store((e.glass.max(0.0) * 100.0) as u32, Ordering::Relaxed);
        st.glass_w_px.store((e.glass_width_px.max(0) * 100) as u32, Ordering::Relaxed);
        st.agc_gain.store(e.agc_gain().to_bits(), Ordering::Relaxed);
        st.grains_total.store(e.grain_count, Ordering::Relaxed);
        if !e.recent.is_empty() {
            let mut seq = st.grain_seq.load(Ordering::Relaxed);
            for g in &e.recent {
                st.grains[seq as usize % GRAIN_TRAIL].store(params::pack_grain(g.pos, g.ratio, g.bus), Ordering::Relaxed);
                seq = seq.wrapping_add(1);
            }
            st.grain_seq.store(seq, Ordering::Release);
        }
    }
}

pub struct RiverrunPlugin;

impl PluginLogic for RiverrunPlugin {
    type Params = RiverrunParams;
    type DspState = RiverrunDsp;

    fn bus_layouts() -> Vec<BusLayout> {
        vec![
            BusLayout::stereo(),
            BusLayout::new()
                .with_input("Main", ChannelConfig::Mono)
                .with_output("Main", ChannelConfig::Stereo),
            BusLayout::mono(),
        ]
    }

    fn init(_params: &Self::Params, _cx: &InitContext) -> Self::DspState {
        RiverrunDsp::default()
    }

    fn reset(state: &mut Self::DspState, params: &Self::Params, config: &AudioConfig) {
        let max = config.max_block_size.max(64);
        let keep = state.engine.snippet_linear();
        let old_sr = state.sample_rate;
        let snippet_gen = state.snippet_gen;
        *state = RiverrunDsp::new(config.sample_rate, max);
        state.snippet_gen = snippet_gen;
        params.status.sample_rate.store(config.sample_rate as u32, Ordering::Relaxed);
        if let Ok(mut m) = params.snippet.mirror.lock() {
            let need = state.engine.snapshot_len();
            let len = m.samples.len();
            if m.samples.capacity() < need {
                m.samples.reserve(need - len);
            }
        }
        // A sample-rate change keeps what was on the screen.
        if keep.iter().any(|&s| s != 0.0) {
            state.engine.load_snippet_at(&keep, old_sr);
            state.publish_mirror(params);
        }
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
        state.requests(params);
        state.midi(params, events);
        let ctl = state.controls(params);
        state.engine.beat_sync = match params.beat_sync.value() {
            params::BeatSyncMode::Original => BeatSync::Original,
            params::BeatSyncMode::Tight => BeatSync::Tight,
        };
        let cell = params.clock.value().samples();
        if cell != state.cell {
            state.cell = cell;
            state.cell_left = 0;
        }

        let tr = context.transport;
        let spq = if tr.tempo > 0.0 { state.sample_rate * 60.0 / tr.tempo } else { 0.0 };
        let changed = tr.playing != state.last_playing;
        state.last_playing = tr.playing;
        let nin = buffer.num_input_channels();
        let stereo_in = nin >= 2;

        if state.wet_l.len() < n {
            // Hosts are not supposed to exceed max_block_size; stay safe anyway.
            state.wet_l.resize(n, 0.0);
            state.wet_r.resize(n, 0.0);
        }
        let mut rec_upto = 0usize;
        let mut done = 0usize;
        let mut first_cell = true;
        while done < n {
            if state.cell_left == 0 {
                let end = (done + state.cell).min(n);
                if ctl.record && nin > 0 && end > rec_upto {
                    let l = &buffer.input(0)[rec_upto..end];
                    let r = stereo_in.then(|| &buffer.input(1)[rec_upto..end]);
                    state.engine.record(l, r);
                    rec_upto = end;
                }
                let host = HostTime {
                    playing: tr.playing,
                    bpm: tr.tempo,
                    ppq: tr.position_beats + if spq > 0.0 { done as f64 / spq } else { 0.0 },
                    sample_pos: tr.position_samples + done as i64,
                    transport_changed: changed && first_cell,
                };
                first_cell = false;
                state.engine.begin_block(&ctl, &host, state.cell);
                state.publish(params);
                state.cell_left = state.cell;
            }
            let m = state.cell_left.min(n - done);
            let (l, r) = (&mut state.wet_l[done..done + m], &mut state.wet_r[done..done + m]);
            state.engine.render(l, r);
            state.cell_left -= m;
            done += m;
        }
        if ctl.record && nin > 0 && n > rec_upto {
            let l = &buffer.input(0)[rec_upto..n];
            let r = stereo_in.then(|| &buffer.input(1)[rec_upto..n]);
            state.engine.record(l, r);
        }

        // Snapshot the screen while recording, and once when Record goes off.
        if ctl.record {
            state.since_mirror += n;
            if state.since_mirror >= MIRROR_EVERY {
                state.publish_mirror(params);
            }
        } else if state.was_recording {
            state.publish_mirror(params);
        }
        state.was_recording = ctl.record;
        params.status.recording.store(ctl.record, Ordering::Relaxed);

        // Dry/wet (the dry path is delayed by the latency) and output gain.
        let nout = buffer.num_output_channels();
        for i in 0..n {
            let mix = params.mix.read() * 0.01;
            let gain = 10f32.powf(params.output.read() / 20.0);
            let p = state.dry_pos;
            let din = [
                if nin > 0 { buffer.input(0)[i] } else { 0.0 },
                if nin > 1 { buffer.input(1)[i] } else if nin > 0 { buffer.input(0)[i] } else { 0.0 },
            ];
            let dl = std::mem::replace(&mut state.dry[0][p], din[0]);
            let dr = std::mem::replace(&mut state.dry[1][p], din[1]);
            state.dry_pos = if p + 1 == LOOKAHEAD { 0 } else { p + 1 };
            let (wl, wr) = (state.wet_l[i], state.wet_r[i]);
            let (ol, or) = if mix >= 1.0 {
                (wl, wr)
            } else {
                (dl + (wl - dl) * mix, dr + (wr - dr) * mix)
            };
            if nout >= 2 {
                buffer.output(0)[i] = ol * gain;
                buffer.output(1)[i] = or * gain;
            } else if nout == 1 {
                buffer.output(0)[i] = 0.5 * (ol + or) * gain;
            }
        }

        if nout >= 1 {
            context.set_meter(P::MeterL, buffer.output_peak(0));
        }
        if nout >= 2 {
            context.set_meter(P::MeterR, buffer.output_peak(1));
        } else if nout == 1 {
            context.set_meter(P::MeterR, buffer.output_peak(0));
        }
        ProcessStatus::Normal
    }

    fn editor(params: Arc<Self::Params>) -> Box<dyn Editor> {
        EguiEditor::new(params.clone(), (ui::WINDOW_W, ui::WINDOW_H), ui::riverrun_ui)
            .with_visuals(truce_egui::theme::dark())
            .resizable(true)
            .min_size((ui::MIN_W, ui::MIN_H))
            .max_size((ui::MAX_W, ui::MAX_H))
            .into_editor()
    }
}

truce::plugin! {
    logic: RiverrunPlugin,
    params: RiverrunParams,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use truce_test::{assertions, driver, InputSource};

    #[test]
    fn starts_silent() {
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

    /// With a fixed grain clock the output no longer depends on the host's
    /// buffer size (in the original it did: one grain per host buffer).
    #[test]
    fn host_buffer_size_does_not_change_the_sound() {
        let run = |block: usize| {
            driver!(Plugin)
                .sample_rate(44_100.0)
                .channels(2)
                .block_size(block)
                .duration(Duration::from_millis(2000))
                .input(InputSource::Silence)
                .setup(|plugin, _| {
                    plugin.params().snippet.load(demo::snippet(44_100.0));
                    plugin.params().random.set_value(60.0);
                })
                .run()
        };
        let a = run(512);
        let b = run(64);
        let c = run(1000);
        assert_eq!(a.output, b.output);
        assert_eq!(a.output, c.output);
    }

    #[test]
    fn granulates_the_demo_snippet() {
        let result = driver!(Plugin)
            .sample_rate(48_000.0)
            .channels(2)
            .block_size(333)
            .duration(Duration::from_millis(1500))
            .input(InputSource::Silence)
            .setup(|plugin, _| {
                plugin.params().snippet.load(demo::snippet(48_000.0));
            })
            .run();
        assertions::assert_no_nans(&result);
        assertions::assert_nonzero(&result);
    }
}
