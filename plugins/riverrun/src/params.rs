//! Parameters, persisted state and the lock-free status the audio thread
//! publishes for the editor.
//!
//! The fifteen original controls keep their original ranges and units (pixels
//! of the 2000 sliders, mostly): the engine truncates them to integers exactly
//! where the original did, so automation from old sessions maps one to one.
//! The MIDI CC numbers are those of manual 2.1 (page 14).

use crate::engine::tables::{self, breakpoint, to_short};
use crate::snippet::SnippetSlot;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::RwLock;
use truce::prelude::*;

/// Grains reported to the editor per block.
pub const GRAIN_TRAIL: usize = 64;
/// Columns of the input-screen overview (the original's 472 pixels).
pub const OVERVIEW: usize = 472;

#[derive(ParamEnum, Default, Debug)]
pub enum Envelope {
    #[name = "Triangle"]
    Triangle,
    #[default]
    #[name = "Square"]
    Square,
    #[name = "Sharp attack"]
    Sharp,
    #[name = "Cosine"]
    Cosine,
}

#[derive(ParamEnum, Default, Debug)]
pub enum PitchGrid {
    #[default]
    #[name = "seamless (no grid)"]
    Seamless,
    #[name = "minor 7th chord"]
    Min7,
    #[name = "major 7th chord"]
    Maj7,
    #[name = "dominant 7th chord"]
    Dom7,
    #[name = "min 5 chord 'deluxe'"]
    Min5,
    #[name = "maj 5 chord 'deluxe'"]
    Maj5,
    #[name = "min 4 chord 'deluxe'"]
    Min4,
    #[name = "maj 4 chord 'deluxe'"]
    Maj4,
    #[name = "min/maj 9th chord"]
    MinMaj9,
    #[name = "octaves and fifths"]
    OctFifths,
    #[name = "diatonic major"]
    Diatonic,
    #[name = "chromatic"]
    Chromatic,
    #[name = "whole note"]
    Whole,
    #[name = "minor thirds"]
    MinThirds,
    #[name = "major thirds"]
    MajThirds,
    #[name = "fourths"]
    Fourths,
    #[name = "diminished fifths"]
    DimFifths,
    #[name = "fifths"]
    Fifths,
    #[name = "minor sixths"]
    MinSixths,
    #[name = "major sixths"]
    MajSixths,
    #[name = "minor sevenths"]
    MinSevenths,
    #[name = "major sevenths"]
    MajSevenths,
    #[name = "octaves"]
    Octaves,
}

#[derive(ParamEnum, Default, Debug)]
pub enum GrainMode {
    #[default]
    #[name = "Flowing"]
    Flowing,
    #[name = "Rhythmic"]
    Rhythmic,
}

/// The original made one grain per host buffer; this fixes that buffer so the
/// sound no longer depends on the host's setting. 512 is the usual Mac OS X
/// host buffer of the time.
#[derive(ParamEnum, Default, Debug)]
pub enum GrainClock {
    #[name = "64"]
    B64,
    #[name = "128"]
    B128,
    #[name = "256"]
    B256,
    #[default]
    #[name = "512"]
    B512,
    #[name = "1024"]
    B1024,
}

/// How Rhythmic mode finds the beat (see `engine::BeatSync`).
#[derive(ParamEnum, Default, Debug)]
pub enum BeatSyncMode {
    #[default]
    #[name = "Original"]
    Original,
    #[name = "Tight"]
    Tight,
}

impl GrainClock {
    pub fn samples(self) -> usize {
        match self {
            GrainClock::B64 => 64,
            GrainClock::B128 => 128,
            GrainClock::B256 => 256,
            GrainClock::B512 => 512,
            GrainClock::B1024 => 1024,
        }
    }
}

#[derive(Params)]
pub struct RiverrunParams {
    // ---- Input screen ----
    #[param(name = "Record", default = 0)]
    pub record: BoolParam,

    #[param(name = "Walk", range = "linear(-5, 5)", default = 0.5, format = "fmt_walk")]
    pub walk: FloatParam,

    #[param(name = "Grain Glass Position", short_name = "Glass", range = "linear(0, 472)", default = 150.0, format = "fmt_px")]
    pub glass: FloatParam,

    #[param(name = "Left (red) Needle", short_name = "Red", range = "linear(0, 472)", default = 20.0, format = "fmt_px")]
    pub needle_l: FloatParam,

    #[param(name = "Right (yellow) Needle", short_name = "Yellow", range = "linear(0, 472)", default = 270.0, format = "fmt_px")]
    pub needle_r: FloatParam,

    #[param(name = "Shape", default = 1)]
    pub envelope: EnumParam<Envelope>,

    #[param(name = "Mono-stereo", range = "linear(0, 93)", default = 93.0, format = "fmt_stereo")]
    pub mono_stereo: FloatParam,

    // ---- Texture ----
    #[param(name = "Rhythmic or Flowing", short_name = "Mode")]
    pub mode: EnumParam<GrainMode>,

    #[param(name = "Grain speed", range = "linear(1, 286)", default = 125.0, format = "fmt_speed")]
    pub speed: FloatParam,

    #[param(name = "Grain speed random", short_name = "Speed rnd", range = "linear(0, 286)", default = 50.0, format = "fmt_jitter")]
    pub speed_random: FloatParam,

    #[param(name = "Grain length", range = "linear(1, 286)", default = 100.0, format = "fmt_length")]
    pub length: FloatParam,

    // ---- Transposition ----
    #[param(name = "Pitch grid")]
    pub grid: EnumParam<PitchGrid>,

    #[param(name = "Pitch", range = "linear(0, 1000)", default = 500.0, format = "fmt_pitch")]
    pub pitch: FloatParam,

    #[param(name = "Pitch Random", short_name = "Random", range = "linear(0, 100)", default = 0.0, format = "fmt_random")]
    pub random: FloatParam,

    #[param(name = "Fine tune", range = "linear(0.666666, 1.5)", default = 1.0, format = "fmt_fine")]
    pub fine: FloatParam,

    // ---- Not in the original ----
    #[param(name = "Mix", range = "linear(0, 100)", default = 100.0, format = "fmt_percent", smooth = "exp(10)")]
    pub mix: FloatParam,

    #[param(name = "Output", range = "linear(-24, 12)", default = 0.0, unit = "dB", smooth = "exp(10)")]
    pub output: FloatParam,

    #[param(name = "Grain clock", short_name = "Clock", default = 3)]
    pub clock: EnumParam<GrainClock>,

    #[param(name = "Beat sync")]
    pub beat_sync: EnumParam<BeatSyncMode>,

    #[meter]
    pub meter_l: MeterSlot,
    #[meter]
    pub meter_r: MeterSlot,

    /// The input screen, saved with the project.
    #[persist = "snippet"]
    pub snippet: SnippetSlot,

    /// 0 = Modern, 1 = Classic (the original's teal and red).
    #[persist = "theme"]
    pub theme: RwLock<u8>,

    #[skip]
    pub status: Status,
}

/// What the editor draws, all published lock-free by the audio thread.
pub struct Status {
    pub sample_rate: AtomicU32,
    /// Grain Glass left edge and width, screen pixels ×100.
    pub glass_px: AtomicU32,
    pub glass_w_px: AtomicU32,
    pub recording: AtomicBool,
    pub agc_gain: AtomicU32,
    pub grains_total: AtomicU64,
    /// Recent grains: `pos (16 bits) | log-ratio (8) | bus (8)`.
    pub grains: [AtomicU32; GRAIN_TRAIL],
    pub grain_seq: AtomicU32,
    /// The editor asks the audio thread to clear the input screen.
    pub clear_req: AtomicU32,
    /// The editor drags the Grain Glass: target pixel ×100 and a sequence.
    pub glass_req: AtomicU32,
    pub glass_req_seq: AtomicU32,
}

impl Default for Status {
    fn default() -> Self {
        Self {
            sample_rate: AtomicU32::new(44100),
            glass_px: AtomicU32::new(15000),
            glass_w_px: AtomicU32::new(0),
            recording: AtomicBool::new(false),
            agc_gain: AtomicU32::new(1.0f32.to_bits()),
            grains_total: AtomicU64::new(0),
            grains: std::array::from_fn(|_| AtomicU32::new(0)),
            grain_seq: AtomicU32::new(0),
            clear_req: AtomicU32::new(0),
            glass_req: AtomicU32::new(0),
            glass_req_seq: AtomicU32::new(0),
        }
    }
}

impl Status {
    pub fn sr(&self) -> f64 {
        self.sample_rate.load(Ordering::Relaxed) as f64
    }
}

/// Playback ratio → semitones.
pub fn ratio_to_st(r: f64) -> f64 {
    12.0 * r.log2()
}

impl RiverrunParams {
    fn fmt_walk(&self, v: f64) -> String {
        if v == 0.0 {
            "halt".into()
        } else {
            format!("{v:+.2}×")
        }
    }

    fn fmt_percent(&self, v: f64) -> String {
        format!("{v:.0} %")
    }

    fn fmt_px(&self, v: f64) -> String {
        format!("{:.0} px", v.trunc())
    }

    fn fmt_stereo(&self, v: f64) -> String {
        match v.trunc() as i32 {
            0 => "mono".into(),
            93 => "stereo".into(),
            p => format!("{:.0} %", p as f64 / 93.0 * 100.0),
        }
    }

    fn fmt_speed(&self, v: f64) -> String {
        if self.mode.value() == GrainMode::Rhythmic {
            let idx = ((v / 21.142_857_142_857_142 + 0.5) as i32).clamp(0, 13);
            tables::RHYTHM_LABELS[idx as usize].to_string()
        } else {
            let r = breakpoint(to_short(v as f32), &tables::FLOWING_MAP);
            if r >= 10.0 { format!("{r:.0} grains/s") } else { format!("{r:.1} grains/s") }
        }
    }

    fn fmt_jitter(&self, v: f64) -> String {
        let s = breakpoint(to_short(v as f32), &tables::TIMING_MAP) as f64 / 44100.0;
        format!("±{:.1} ms", s * 1000.0)
    }

    fn fmt_length(&self, v: f64) -> String {
        let s = breakpoint(to_short(v as f32), &tables::LENGTH_MAP) as i32 as f64 / 44100.0;
        format!("{:.1} ms", s * 1000.0)
    }

    fn fmt_pitch(&self, v: f64) -> String {
        let r = breakpoint(to_short(v as f32), &tables::PITCH_MAP) as f64;
        format!("{:+.1} st", ratio_to_st(r))
    }

    fn fmt_random(&self, v: f64) -> String {
        let r = breakpoint(to_short(v as f32), &tables::RANDOM_MAP);
        if r == 0.0 { "none".into() } else { format!("+{r:.2}×") }
    }

    fn fmt_fine(&self, v: f64) -> String {
        format!("{:+.0} ct", ratio_to_st(v) * 100.0)
    }
}

/// Packs a grain for the editor.
pub fn pack_grain(pos: f32, ratio: f32, bus: u8) -> u32 {
    let p = (pos.clamp(0.0, 1.0) * 65535.0) as u32;
    // log2 ratio −4…+5 → 0…255
    let lr = (((ratio.max(1e-3).log2() + 4.0) / 9.0).clamp(0.0, 1.0) * 255.0) as u32;
    p << 16 | lr << 8 | bus as u32
}

pub fn unpack_grain(v: u32) -> (f32, f32, u8) {
    let pos = (v >> 16) as f32 / 65535.0;
    let ratio = 2f32.powf(((v >> 8) & 0xff) as f32 / 255.0 * 9.0 - 4.0);
    (pos, ratio, (v & 0xff) as u8)
}
