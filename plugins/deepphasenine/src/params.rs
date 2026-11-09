//! Parameters (original units and ranges, `orjogk…::GetParameter` @ 0x19e56)
//! and the lock-free status the audio thread publishes for the editor.

use crate::engine::{Controls, SYNC_BEATS};
use std::sync::atomic::{AtomicI32, AtomicU32};
use std::sync::RwLock;
use truce::prelude::*;

#[derive(ParamEnum, Default, Debug)]
pub enum LfoShape {
    #[default]
    #[name = "Sine"]
    Sine,
    #[name = "Triangle"]
    Triangle,
    #[name = "Sample & Hold"]
    SampleHold,
    #[name = "OneShot Sine"]
    OneShotSine,
    #[name = "OneShot Triangle"]
    OneShotTriangle,
}

#[derive(ParamEnum, Default, Debug)]
pub enum OneShotDirection {
    #[default]
    #[name = "Down"]
    Down,
    #[name = "Stop"]
    Stop,
    #[name = "Up"]
    Up,
}

#[derive(Params)]
pub struct DpnParams {
    #[param(name = "Notch count", short_name = "Notches", range = "discrete(1, 24)", default = 3)]
    pub notches: IntParam,

    #[param(name = "Feedback", range = "linear(-0.95, 0.95)", default = 0.8, format = "fmt_feedback")]
    pub feedback: FloatParam,

    #[param(name = "Low", range = "log(0.01, 2000)", default = 100.0, format = "fmt_hz")]
    pub low: FloatParam,

    #[param(name = "High", range = "log(200, 20000)", default = 4000.0, format = "fmt_hz")]
    pub high: FloatParam,

    #[param(name = "Spread", range = "linear(0, 100)", default = 35.0, format = "fmt_spread")]
    pub spread: FloatParam,

    /// "WetDry" in the original: the level of the phased signal added to the
    /// (always present) dry one.
    #[param(name = "Mix", range = "linear(0, 1)", default = 1.0, format = "fmt_mix")]
    pub mix: FloatParam,

    #[param(name = "LFO Choice", short_name = "LFO")]
    pub lfo: EnumParam<LfoShape>,

    #[param(name = "LFO Speed", short_name = "Speed", range = "discrete(0, 70)", default = 21, format = "fmt_speed")]
    pub speed: IntParam,

    #[param(name = "One shot end", short_name = "Socket", range = "linear(0, 1)", default = 0.5, format = "fmt_socket")]
    pub oneshot_end: FloatParam,

    #[param(name = "One shot direction", short_name = "Direction")]
    pub oneshot_dir: EnumParam<OneShotDirection>,

    #[param(name = "Sync to beats", short_name = "Sync", default = 0)]
    pub sync: BoolParam,

    #[param(name = "Stereo offset", short_name = "Offset", range = "discrete(-180, 180)", default = 0, unit = "°")]
    pub offset: IntParam,

    // ---- not in the original ----
    #[param(name = "Output", range = "linear(-24, 12)", default = 0.0, unit = "dB", smooth = "exp(10)")]
    pub output: FloatParam,

    #[meter]
    pub meter_l: MeterSlot,
    #[meter]
    pub meter_r: MeterSlot,

    /// 0 = Modern, 1 = Classic.
    #[persist = "theme"]
    pub theme: RwLock<u8>,

    #[skip]
    pub status: Status,
}

/// Published by the audio thread (f32 bits in the u32s).
pub struct Status {
    pub sample_rate: AtomicU32,
    /// LFO value 0…1 (the notch sweep position, the ball's x) and direction.
    pub phase: AtomicU32,
    pub dir: AtomicI32,
    /// Second-order section parameters for the display: centre (fraction of
    /// Nyquist) of each channel, pole radius, feedback, depth, notches.
    pub freq_l: AtomicU32,
    pub freq_r: AtomicU32,
    pub limiter: AtomicU32,
    /// Editor → engine: the ball is held at `drag_pos` (−1 = not held).
    pub drag_pos: AtomicU32,
    pub drag_dir: AtomicI32,
    pub drag_seq: AtomicU32,
}

impl Default for Status {
    fn default() -> Self {
        Self {
            sample_rate: AtomicU32::new(44100),
            phase: AtomicU32::new(0),
            dir: AtomicI32::new(1),
            freq_l: AtomicU32::new(0),
            freq_r: AtomicU32::new(0),
            limiter: AtomicU32::new(1.0f32.to_bits()),
            drag_pos: AtomicU32::new((-1.0f32).to_bits()),
            drag_dir: AtomicI32::new(1),
            drag_seq: AtomicU32::new(0),
        }
    }
}

/// Note value of a sync setting, e.g. "1/4", "3/8", "2 bars".
pub fn beats_label(beats: f64) -> String {
    let b = beats;
    if b >= 4.0 && (b / 4.0).fract().abs() < 1e-9 {
        let bars = b / 4.0;
        return if bars == 1.0 { "1 bar".into() } else { format!("{bars:.0} bars") };
    }
    // Express as a fraction of a whole note: beats/4 = num/den
    let whole = b / 4.0;
    for den in [1u32, 2, 3, 4, 6, 7, 8, 9, 12, 14, 16, 18, 24, 28, 32, 48, 64, 96, 128] {
        let num = whole * den as f64;
        if (num - num.round()).abs() < 1e-6 && num.round() >= 1.0 {
            return format!("{}/{}", num.round() as u32, den);
        }
    }
    format!("{b:.3} beats")
}

impl DpnParams {
    pub fn controls(&self) -> Controls {
        Controls {
            notches: self.notches.value() as i32,
            feedback: self.feedback.value(),
            low: self.low.value(),
            high: self.high.value(),
            spread: self.spread.value(),
            wet_dry: self.mix.value(),
            lfo_choice: self.lfo.index() as i32 + 1,
            lfo_speed: self.speed.value() as i32,
            oneshot_end: self.oneshot_end.value(),
            oneshot_dir: self.oneshot_dir.index() as i32 - 1,
            sync: self.sync.value(),
            stereo_offset: self.offset.value() as i32,
        }
    }

    fn fmt_feedback(&self, v: f64) -> String {
        format!("{:+.0} %", v * 100.0 / 0.95)
    }
    fn fmt_hz(&self, v: f64) -> String {
        if v >= 1000.0 { format!("{:.2} kHz", v / 1000.0) } else if v >= 10.0 { format!("{v:.0} Hz") } else { format!("{v:.2} Hz") }
    }
    fn fmt_spread(&self, v: f64) -> String {
        format!("{v:.0}")
    }
    fn fmt_mix(&self, v: f64) -> String {
        format!("{:.0} %", v * 100.0)
    }
    fn fmt_socket(&self, v: f64) -> String {
        format!("{:.0} %", v * 100.0)
    }
    fn fmt_speed(&self, v: f64) -> String {
        let s = v.round() as i32;
        let sh = self.lfo.index() == 2;
        if self.sync.value() {
            let idx = if sh { (0x3f - s).max(0) } else { 0x54 - s } as usize;
            beats_label(SYNC_BEATS[idx.min(84)])
        } else {
            let hz = 2f64.powf(s as f64 / 5.0) * if sh { 0.03125 } else { 0.003_906_25 };
            if hz >= 1.0 { format!("{hz:.2} Hz") } else { format!("{:.1} s", 1.0 / hz) }
        }
    }
}
