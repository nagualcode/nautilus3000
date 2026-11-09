//! The built-in demo snippet: a synthetic sung phrase, so Riverrun has
//! something to granulate before anything is recorded. Audio Ease's own demo
//! was a spoken voice; a voice is still the best way to hear the
//! "immense Gregorian monk choirs that can be created out of a single
//! utterance" of the manual.
//!
//! A glottal pulse train with vibrato runs through three formant resonators
//! that move from "aa" to "oo" to "ee", then a breathy "sh" at the end.

use crate::snippet::SnippetData;

/// The data fork of `reference/macos9/Riverrun Demo Song/Audio Files/Voice`, the spoken voice
/// Audio Ease used in the demo song that shipped with Nautilus Bundle 1.0
/// (Mac OS 9, 2000). Sound Designer II keeps its header in the resource fork,
/// which did not survive; the data is 16-bit big-endian mono, and its spectrum
/// and length (205 800 samples ≈ one full input screen) place it at 44.1 kHz.
const VOICE_SD2: &[u8] = include_bytes!("../assets/voice.sd2");

/// The Voice as the demo song's frozen screens would have held it: streamed in
/// from the right and frozen as the utterance ended, so the last spoken
/// syllable sits at the right edge (the presets park the Grain Glass there).
/// The trailing near-silence is rotated to the front. This alignment is an
/// inference — the song's own "RiverRun Temp Snap" files are lost.
pub fn voice_streamed() -> SnippetData {
    let mut v = voice();
    let snap = (44100.0f64 * 4.65) as usize;
    v.samples.truncate(snap);
    let thr = 10f32.powf(-34.0 / 20.0);
    let last = v.samples.iter().rposition(|s| s.abs() > thr).unwrap_or(snap - 1);
    let tail = snap - 1 - last;
    v.samples.rotate_right(tail.saturating_sub(64));
    v.name = "Voice (as streamed in the demo song)".into();
    v
}

/// The original demo voice.
pub fn voice() -> SnippetData {
    let samples = VOICE_SD2
        .chunks_exact(2)
        .map(|b| i16::from_be_bytes([b[0], b[1]]) as f32 / 32768.0)
        .collect();
    SnippetData { name: "Voice (Audio Ease demo, 2000)".into(), sample_rate: 44100.0, samples }
}
use std::f64::consts::TAU;

struct Reson {
    a1: f64,
    a2: f64,
    g: f64,
    y1: f64,
    y2: f64,
}

impl Reson {
    fn new() -> Self {
        Self { a1: 0.0, a2: 0.0, g: 0.0, y1: 0.0, y2: 0.0 }
    }
    fn set(&mut self, f: f64, bw: f64, sr: f64) {
        let r = (-std::f64::consts::PI * bw / sr).exp();
        self.a1 = 2.0 * r * (TAU * f / sr).cos();
        self.a2 = -r * r;
        self.g = 1.0 - r;
    }
    fn tick(&mut self, x: f64) -> f64 {
        let y = self.g * x + self.a1 * self.y1 + self.a2 * self.y2;
        self.y2 = self.y1;
        self.y1 = y;
        y
    }
}

fn lerp(a: f64, b: f64, t: f64) -> f64 {
    a + (b - a) * t.clamp(0.0, 1.0)
}

/// 4.6 s at `sample_rate`.
pub fn snippet(sample_rate: f64) -> SnippetData {
    let sr = sample_rate;
    let n = (sr * 4.6) as usize;
    let mut out = Vec::with_capacity(n);
    let mut f = [Reson::new(), Reson::new(), Reson::new()];
    // (F1, F2, F3) for aa, oo, ee (male voice)
    let vowels = [[730.0, 1090.0, 2440.0], [300.0, 870.0, 2240.0], [270.0, 2290.0, 3010.0]];
    let mut phase = 0.0f64;
    let mut noise = 0x1234_5678u32;
    for i in 0..n {
        let t = i as f64 / sr;
        // D3 → F3 → A2, gliding
        let note = if t < 1.4 { 50.0 } else if t < 2.8 { lerp(50.0, 53.0, (t - 1.4) * 8.0) } else { lerp(53.0, 45.0, (t - 2.8) * 6.0) };
        let vib = 0.25 * (TAU * 5.2 * t).sin() * lerp(0.0, 1.0, (t - 0.3) * 2.0);
        let f0 = 440.0 * 2f64.powf((note + vib - 69.0) / 12.0);
        phase += f0 / sr;
        if phase >= 1.0 {
            phase -= 1.0;
        }
        // Rosenberg-like glottal pulse
        let g = if phase < 0.4 { 0.5 - 0.5 * (std::f64::consts::PI * phase / 0.4).cos() } else if phase < 0.56 {
            (std::f64::consts::FRAC_PI_2 * (phase - 0.4) / 0.16).cos()
        } else {
            0.0
        };
        noise ^= noise << 13;
        noise ^= noise >> 17;
        noise ^= noise << 5;
        let white = noise as f64 / u32::MAX as f64 - 0.5;
        let k = (t / 4.0 * 2.0).min(2.0);
        let (a, b, w) = if k < 1.0 { (0, 1, k) } else { (1, 2, k - 1.0) };
        for (j, r) in f.iter_mut().enumerate() {
            let fr = lerp(vowels[a][j], vowels[b][j], w);
            r.set(fr, 60.0 + 40.0 * j as f64, sr);
        }
        let voiced = t < 4.0;
        let src = if voiced { g - 0.3 + 0.04 * white } else { 0.0 };
        let mut v = f[0].tick(src) * 1.0 + f[1].tick(src) * 0.6 + f[2].tick(src) * 0.3;
        let env = lerp(0.0, 1.0, t / 0.08) * lerp(1.0, 0.0, (t - 3.85) / 0.15);
        v *= env * 2.2;
        if (4.05..4.45).contains(&t) {
            let e = ((t - 4.05) / 0.4 * std::f64::consts::PI).sin();
            v += white * 0.35 * e;
        }
        out.push(v as f32);
    }
    let peak = out.iter().fold(0.0f32, |m, &x| m.max(x.abs())).max(1e-6);
    out.iter_mut().for_each(|x| *x *= 0.7 / peak);
    SnippetData { name: "Demo: sung phrase".into(), sample_rate: sr, samples: out }
}
