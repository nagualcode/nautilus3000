//! The output gain control, `yruwg…::AGC(float**, unsigned long)` @ 0x21db0,
//! initialised in `InitBuffers` with `InitAGC(1.0, 0.3, down, up, 0.15, 300)`.
//!
//! It is a 300-sample look-ahead: the peak of the *incoming* sample decides the
//! gain applied to the sample leaving the delay line. The gain falls at
//! −24 dB/s while `gain · peak > 0.15` and recovers at +6 dB/s, but only while
//! it is below 0.3 — so after the first loud moment it settles around 0.3 and
//! never returns to unity. That is how the original behaves; the plug-in
//! reports the 300 samples as latency (`GetLatency` @ 0x20022).

pub const LOOKAHEAD: usize = 300;

#[derive(Clone)]
pub struct Agc {
    pub gain: f32,
    pub last_peak: f32,
    floor: f32,
    down: f32,
    up: f32,
    threshold: f32,
    delay: [Vec<f32>; 2],
    pos: usize,
}

impl Agc {
    pub fn new(sample_rate: f64) -> Self {
        let d = sample_rate * 0.5 * 20.0;
        Self {
            gain: 1.0,
            last_peak: 0.0,
            floor: 0.3,
            down: 10f64.powf(-12.0 / d) as f32,
            up: 10f64.powf(3.0 / d) as f32,
            threshold: 0.15,
            delay: [vec![0.0; LOOKAHEAD], vec![0.0; LOOKAHEAD]],
            pos: 0,
        }
    }

    /// In-place on two channels. Bit-identical to the original's block-shuffling
    /// implementation for any block size.
    pub fn process(&mut self, l: &mut [f32], r: &mut [f32]) {
        let mut g = self.gain;
        let mut pk = self.last_peak;
        for i in 0..l.len() {
            if self.threshold < pk {
                g *= self.down;
            }
            if g < self.floor {
                g *= self.up;
            }
            let (xl, xr) = (l[i], r[i]);
            let p = self.pos;
            l[i] = g * self.delay[0][p];
            r[i] = g * self.delay[1][p];
            self.delay[0][p] = xl;
            self.delay[1][p] = xr;
            self.pos = if p + 1 == LOOKAHEAD { 0 } else { p + 1 };
            pk = g * fabs_max(xl, xr);
        }
        self.gain = g;
        self.last_peak = pk;
    }
}

/// `FabsMaxOfBuffersAt` @ 0x21d48.
#[inline]
fn fabs_max(a: f32, b: f32) -> f32 {
    let mut m = 0.0f32;
    for v in [a, b] {
        let av = if v < 0.0 { -v } else { v };
        if m < av {
            m = av;
        }
    }
    m
}
