//! Parameters, in the original's units and ranges
//! (`wdncx…::GetParameter` @ 0x1c208), and the lock-free status the audio
//! thread publishes for the editor.

use crate::engine::{breakpoint, needle_hz, Controls, BANDS, FADER_CURVE};
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::sync::RwLock;
use truce::prelude::*;

/// `PeriscopeEditMode`: what a drag in the fader screen does. The order and
/// the modifier keys are the original's (`PeriscopeEditMode::Idle` @ 0x19d40).
#[derive(ParamEnum, Default, Debug)]
pub enum EditMode {
    #[default]
    #[name = "Free draw"]
    FreeDraw,
    #[name = "Flat"]
    Flat,
    #[name = "Magnetic"]
    Magnetic,
    #[name = "Amplify"]
    Amplify,
    #[name = "Exaggerate"]
    Exaggerate,
}

#[derive(Params)]
pub struct PeriscopeParams {
    /// "loCutOff": the red needle, 418 steps from 18 Hz to Nyquist.
    #[param(name = "Low needle", short_name = "Low", range = "linear(5, 408)", default = 40.0, format = "fmt_needle")]
    pub low: FloatParam,

    /// "hiCutOff": the yellow needle.
    #[param(name = "High needle", short_name = "High", range = "linear(175, 412)", default = 300.0, format = "fmt_needle")]
    pub high: FloatParam,

    #[param(name = "Fader 1", short_name = "F1", range = "discrete(0, 265)", default = 135, format = "fmt_fader")]
    pub fader_1: IntParam,

    #[param(name = "Fader 2", short_name = "F2", range = "discrete(0, 265)", default = 135, format = "fmt_fader")]
    pub fader_2: IntParam,

    #[param(name = "Fader 3", short_name = "F3", range = "discrete(0, 265)", default = 135, format = "fmt_fader")]
    pub fader_3: IntParam,

    #[param(name = "Fader 4", short_name = "F4", range = "discrete(0, 265)", default = 135, format = "fmt_fader")]
    pub fader_4: IntParam,

    #[param(name = "Fader 5", short_name = "F5", range = "discrete(0, 265)", default = 135, format = "fmt_fader")]
    pub fader_5: IntParam,

    #[param(name = "Fader 6", short_name = "F6", range = "discrete(0, 265)", default = 135, format = "fmt_fader")]
    pub fader_6: IntParam,

    #[param(name = "Fader 7", short_name = "F7", range = "discrete(0, 265)", default = 135, format = "fmt_fader")]
    pub fader_7: IntParam,

    #[param(name = "Fader 8", short_name = "F8", range = "discrete(0, 265)", default = 135, format = "fmt_fader")]
    pub fader_8: IntParam,

    #[param(name = "Fader 9", short_name = "F9", range = "discrete(0, 265)", default = 135, format = "fmt_fader")]
    pub fader_9: IntParam,

    #[param(name = "Fader 10", short_name = "F10", range = "discrete(0, 265)", default = 135, format = "fmt_fader")]
    pub fader_10: IntParam,

    #[param(name = "Fader 11", short_name = "F11", range = "discrete(0, 265)", default = 135, format = "fmt_fader")]
    pub fader_11: IntParam,

    #[param(name = "Fader 12", short_name = "F12", range = "discrete(0, 265)", default = 135, format = "fmt_fader")]
    pub fader_12: IntParam,

    #[param(name = "Fader 13", short_name = "F13", range = "discrete(0, 265)", default = 135, format = "fmt_fader")]
    pub fader_13: IntParam,

    #[param(name = "Fader 14", short_name = "F14", range = "discrete(0, 265)", default = 135, format = "fmt_fader")]
    pub fader_14: IntParam,

    #[param(name = "Fader 15", short_name = "F15", range = "discrete(0, 265)", default = 135, format = "fmt_fader")]
    pub fader_15: IntParam,

    #[param(name = "Fader 16", short_name = "F16", range = "discrete(0, 265)", default = 135, format = "fmt_fader")]
    pub fader_16: IntParam,

    #[param(name = "Fader 17", short_name = "F17", range = "discrete(0, 265)", default = 135, format = "fmt_fader")]
    pub fader_17: IntParam,

    #[param(name = "Fader 18", short_name = "F18", range = "discrete(0, 265)", default = 135, format = "fmt_fader")]
    pub fader_18: IntParam,

    #[param(name = "Fader 19", short_name = "F19", range = "discrete(0, 265)", default = 135, format = "fmt_fader")]
    pub fader_19: IntParam,

    #[param(name = "Fader 20", short_name = "F20", range = "discrete(0, 265)", default = 135, format = "fmt_fader")]
    pub fader_20: IntParam,

    #[param(name = "Fader 21", short_name = "F21", range = "discrete(0, 265)", default = 135, format = "fmt_fader")]
    pub fader_21: IntParam,

    #[param(name = "Fader 22", short_name = "F22", range = "discrete(0, 265)", default = 135, format = "fmt_fader")]
    pub fader_22: IntParam,

    #[param(name = "Fader 23", short_name = "F23", range = "discrete(0, 265)", default = 135, format = "fmt_fader")]
    pub fader_23: IntParam,

    #[param(name = "Fader 24", short_name = "F24", range = "discrete(0, 265)", default = 135, format = "fmt_fader")]
    pub fader_24: IntParam,

    #[param(name = "Fader 25", short_name = "F25", range = "discrete(0, 265)", default = 135, format = "fmt_fader")]
    pub fader_25: IntParam,

    #[param(name = "Fader 26", short_name = "F26", range = "discrete(0, 265)", default = 135, format = "fmt_fader")]
    pub fader_26: IntParam,

    #[param(name = "Fader 27", short_name = "F27", range = "discrete(0, 265)", default = 135, format = "fmt_fader")]
    pub fader_27: IntParam,

    #[param(name = "Fader 28", short_name = "F28", range = "discrete(0, 265)", default = 135, format = "fmt_fader")]
    pub fader_28: IntParam,

    #[param(name = "Fader 29", short_name = "F29", range = "discrete(0, 265)", default = 135, format = "fmt_fader")]
    pub fader_29: IntParam,

    #[param(name = "Fader 30", short_name = "F30", range = "discrete(0, 265)", default = 135, format = "fmt_fader")]
    pub fader_30: IntParam,

    #[param(name = "Fader 31", short_name = "F31", range = "discrete(0, 265)", default = 135, format = "fmt_fader")]
    pub fader_31: IntParam,

    #[param(name = "Fader 32", short_name = "F32", range = "discrete(0, 265)", default = 135, format = "fmt_fader")]
    pub fader_32: IntParam,

    // ---- editor settings (not automatable in the original) ----
    #[param(name = "Fader edit mode", short_name = "Edit", default = 0)]
    pub edit_mode: EnumParam<EditMode>,

    /// "Gooeyness": the width, in faders, a Magnetic drag pulls along.
    #[param(name = "Magnetism", range = "linear(1, 32)", default = 3.0, format = "fmt_plain")]
    pub magnetism: FloatParam,

    /// "Curve Latency": how slowly the blue (average) graph follows.
    #[param(name = "Display release", short_name = "Release", range = "linear(0, 0.999)", default = 0.9, format = "fmt_plain")]
    pub release: FloatParam,

    #[param(name = "Brightness", range = "linear(0, 1)", default = 1.0, format = "fmt_percent")]
    pub brightness: FloatParam,

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

/// Length of the spectroscope's ring ("DrawBuf"; "draw fft size" ≤ 16384).
pub const SCOPE: usize = 16384;

/// Published by the audio thread (f32 bits in the u32s).
pub struct Status {
    pub sample_rate: AtomicU32,
    /// The needles in effect (MIDI, or pushed apart by the engine).
    pub low: AtomicU32,
    pub high: AtomicU32,
    /// Bumped whenever the engine or MIDI moved a needle or fader the host
    /// does not know about yet; the editor then writes it to the parameter.
    pub pending: AtomicU32,
    /// The faders in effect (MIDI notes / CCs override the parameters).
    pub faders: [AtomicU32; BANDS],
    /// Band edges in Hz (private parameter "freqs").
    pub freqs: [AtomicU32; BANDS],
    /// The output, all channels summed / √channels, for the spectroscope.
    pub scope: Box<[AtomicU32]>,
    pub scope_pos: AtomicUsize,
}

impl Default for Status {
    fn default() -> Self {
        Self {
            sample_rate: AtomicU32::new(44100),
            low: AtomicU32::new(40.0f32.to_bits()),
            high: AtomicU32::new(300.0f32.to_bits()),
            pending: AtomicU32::new(0),
            faders: std::array::from_fn(|_| AtomicU32::new(135)),
            freqs: std::array::from_fn(|_| AtomicU32::new(0)),
            scope: (0..SCOPE).map(|_| AtomicU32::new(0)).collect(),
            scope_pos: AtomicUsize::new(0),
        }
    }
}

pub fn fader_db(v: i64) -> f32 {
    20.0 * breakpoint(v.clamp(0, 265) as i16, &FADER_CURVE).log10()
}

/// `fukyq…::GetValueString` @ 0x1d0fc: "+12 dB", "-inf dB".
pub fn fader_label(v: i64) -> String {
    let db = fader_db(v);
    if db.is_infinite() {
        "-inf dB".into()
    } else if db > 0.0 {
        format!("+{db:.0} dB")
    } else {
        format!("{db:.0} dB")
    }
}

pub fn hz_label(hz: f64) -> String {
    if hz >= 1000.0 { format!("{:.2} kHz", hz / 1000.0) } else { format!("{hz:.1} Hz") }
}

impl PeriscopeParams {
    pub fn fader(&self, k: usize) -> &IntParam {
        [&self.fader_1, &self.fader_2, &self.fader_3, &self.fader_4, &self.fader_5, &self.fader_6, &self.fader_7, &self.fader_8, &self.fader_9, &self.fader_10, &self.fader_11, &self.fader_12, &self.fader_13, &self.fader_14, &self.fader_15, &self.fader_16, &self.fader_17, &self.fader_18, &self.fader_19, &self.fader_20, &self.fader_21, &self.fader_22, &self.fader_23, &self.fader_24, &self.fader_25, &self.fader_26, &self.fader_27, &self.fader_28, &self.fader_29, &self.fader_30, &self.fader_31, &self.fader_32][k]
    }

    pub fn controls(&self) -> Controls {
        Controls {
            low: self.low.value(),
            high: self.high.value(),
            faders: std::array::from_fn(|k| self.fader(k).value() as i32),
        }
    }

    pub fn sample_rate(&self) -> f64 {
        self.status.sample_rate.load(Ordering::Relaxed) as f64
    }

    fn fmt_needle(&self, v: f64) -> String {
        hz_label(needle_hz(v as f32, self.sample_rate()) as f64)
    }
    fn fmt_fader(&self, v: f64) -> String {
        fader_label(v.round() as i64)
    }
    fn fmt_plain(&self, v: f64) -> String {
        format!("{v:.2}")
    }
    fn fmt_percent(&self, v: f64) -> String {
        format!("{:.0} %", v * 100.0)
    }
}

pub use PeriscopeParamsParamId as P;

/// Parameter id of fader `k` (0-based).
pub fn fader_id(k: usize) -> P {
    [P::Fader1, P::Fader2, P::Fader3, P::Fader4, P::Fader5, P::Fader6, P::Fader7, P::Fader8, P::Fader9, P::Fader10, P::Fader11, P::Fader12, P::Fader13, P::Fader14, P::Fader15, P::Fader16, P::Fader17, P::Fader18, P::Fader19, P::Fader20, P::Fader21, P::Fader22, P::Fader23, P::Fader24, P::Fader25, P::Fader26, P::Fader27, P::Fader28, P::Fader29, P::Fader30, P::Fader31, P::Fader32][k]
}
