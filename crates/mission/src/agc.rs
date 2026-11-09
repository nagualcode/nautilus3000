//! `yruwg…::AGC(float**, unsigned long)` (Riverrun @ 0x21db0, Deep Phase Nine
//! has the same code) with `InitAGC(gain, floor, down, up, threshold, 300)`.
//!
//! A 300-sample look-ahead: the peak of the *incoming* sample (over all
//! channels) decides the gain applied to the sample leaving the delay line.
//! Per sample: if `gain · peak > threshold` the gain is multiplied by `down`;
//! if `gain < floor` it is multiplied by `up`. Each plug-in picks its own
//! numbers — see the constructors.

pub const LOOKAHEAD: usize = 300;

#[derive(Clone)]
pub struct Agc {
    pub gain: f32,
    pub last_peak: f32,
    floor: f32,
    down: f32,
    up: f32,
    threshold: f32,
    delay: Vec<Vec<f32>>,
    pos: usize,
}

impl Agc {
    pub fn new(channels: usize, gain: f32, floor: f32, down: f32, up: f32, threshold: f32) -> Self {
        Self {
            gain,
            last_peak: 0.0,
            floor,
            down,
            up,
            threshold,
            delay: vec![vec![0.0; LOOKAHEAD]; channels.max(1)],
            pos: 0,
        }
    }

    /// Riverrun: `InitAGC(1.0, 0.3, 10^(−12/(10·sr)), 10^(3/(10·sr)), 0.15, 300)`
    /// — falls at −24 dB/s above 0.15, recovers at +6 dB/s only below 0.3.
    pub fn riverrun(sample_rate: f64) -> Self {
        let d = sample_rate * 0.5 * 20.0;
        Self::new(2, 1.0, 0.3, 10f64.powf(-12.0 / d) as f32, 10f64.powf(3.0 / d) as f32, 0.15)
    }

    /// Deep Phase Nine: `InitAGC(1.0, 1.0, 10^−0.003, 10^(18/(10·sr)), 0.5, 300)`
    /// — a peak limiter at −6 dBFS: −0.06 dB per sample of attack, +36 dB/s
    /// release, never above unity.
    pub fn deep_phase_nine(channels: usize, sample_rate: f64) -> Self {
        let d = sample_rate * 0.5 * 20.0;
        Self::new(channels, 1.0, 1.0, 10f64.powf(-0.003) as f32, 10f64.powf(18.0 / d) as f32, 0.5)
    }

    pub fn channels(&self) -> usize {
        self.delay.len()
    }

    /// In place; `chans` must have `channels()` slices of equal length.
    /// Bit-identical to the original's block-shuffling implementation for any
    /// block size.
    pub fn process(&mut self, chans: &mut [&mut [f32]]) {
        let n = chans[0].len();
        let mut g = self.gain;
        let mut pk = self.last_peak;
        for i in 0..n {
            if self.threshold < pk {
                g *= self.down;
            }
            if g < self.floor {
                g *= self.up;
            }
            let p = self.pos;
            // FabsMaxOfBuffersAt @ 0x21d48
            let mut m = 0.0f32;
            for (ch, line) in chans.iter_mut().zip(self.delay.iter_mut()) {
                let x = ch[i];
                ch[i] = g * line[p];
                line[p] = x;
                let ax = if x < 0.0 { -x } else { x };
                if m < ax {
                    m = ax;
                }
            }
            self.pos = if p + 1 == LOOKAHEAD { 0 } else { p + 1 };
            pk = g * m;
        }
        self.gain = g;
        self.last_peak = pk;
    }

    /// Two-channel convenience.
    pub fn process_stereo(&mut self, l: &mut [f32], r: &mut [f32]) {
        self.process(&mut [l, r]);
    }
}
