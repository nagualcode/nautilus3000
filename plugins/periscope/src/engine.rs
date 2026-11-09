//! The PeriScope engine, translated from the i386 slice of `Periscope.vst`
//! 2.6.4 (see `reverse_engineering/periscope/`).
//!
//! * [`Filter::update`] is `rzivejah…::UpdateFreqs` @ 0x1ac9a: needles →
//!   band frequencies, faders → gains, gains → a zero-phase spectrum
//!   (`thumbs2fftcoefsHerm` @ 0x16bb4) → impulse → centred, Hann-windowed,
//!   linear-phase FIR of 2048 taps (`halfShiftAndHanPad` @ 0x198ee) → its
//!   spectrum;
//! * [`Periscope::process`] is `Process` @ 0x1b674: per channel, FFT
//!   convolution by overlap-add (4096-point FFT, 2048-sample blocks), the
//!   channels' block boundaries staggered so the work spreads over buffers.

use mission::fft::RealFft;
use mission::ring::Ring;

/// `gFFTConvSize`.
pub const N: usize = 4096;
pub const HALF: usize = N / 2;
/// Faders (params 6 … 37).
pub const BANDS: usize = 32;
/// `GetLatency` @ 0x1a4da: a block of 2048 plus the FIR's centre at 1024.
/// (True for host buffers up to 2048 only; see [`Periscope::latency`].)
pub const LATENCY: u32 = 0xc00;

/// `gThumbBreakPoints`, built at load time by a static initialiser
/// (`__static_initialization_and_destruction_0` @ 0x1230):
/// fader position → linear gain, `(float)pow(10, dB/20)`.
pub const FADER_CURVE: [f32; 36] = [
    0.0, 0.0,
    3.0, f32::from_bits(0x3784_f352),   // −96 dB
    15.0, f32::from_bits(0x3b82_73a6),  // −48 dB
    35.0, f32::from_bits(0x3c23_d70a),  // −40 dB
    55.0, f32::from_bits(0x3ccd_c614),  // −32 dB
    75.0, f32::from_bits(0x3d81_3856),  // −24 dB
    95.0, f32::from_bits(0x3e22_4b06),  // −16 dB
    115.0, f32::from_bits(0x3ecb_d4b4), // −8 dB
    135.0, 1.0,                         // 0 dB
    150.0, f32::from_bits(0x3fca_ddc8), // +4 dB
    165.0, f32::from_bits(0x4020_c2bf), // +8 dB
    180.0, f32::from_bits(0x407e_c9e1), // +12 dB
    195.0, f32::from_bits(0x40c9_e807), // +16 dB
    210.0, 10.0,                        // +20 dB
    225.0, f32::from_bits(0x417d_953a), // +24 dB
    240.0, f32::from_bits(0x41c8_f36f), // +28 dB
    255.0, f32::from_bits(0x421f_3e2d), // +32 dB
    263.0, f32::from_bits(0x4248_7993), // +34 dB
];

/// The Mission framework's breakpoint interpolator
/// (`raprsrnunraeegybghbdzrvqypyotzm(short, float*, short)`).
pub fn breakpoint(x: i16, table: &[f32]) -> f32 {
    let n = table.len();
    let f = x as i32 as f32;
    if table[0] < f {
        if f < table[n - 2] {
            let mut s = 0;
            while s < n && table[s] <= f {
                s += 2;
            }
            ((f - table[s - 2]) / (table[s] - table[s - 2])) * (table[s + 1] - table[s - 1]) + table[s - 1]
        } else {
            table[n - 1]
        }
    } else {
        table[1]
    }
}

#[inline]
fn ln_f(x: f32) -> f32 {
    (x as f64).ln() as f32
}
#[inline]
fn exp_f(x: f32) -> f32 {
    (x as f64).exp() as f32
}
#[inline]
fn pow_f(x: f32, y: f32) -> f32 {
    (x as f64).powf(y as f64) as f32
}
#[inline]
fn cos_f(x: f32) -> f32 {
    (x as f64).cos() as f32
}

/// The controls (`wdncx…::GetParameter` @ 0x1c208).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Controls {
    /// Param 2 "loCutOff", 5 … 408 (418 steps from 18 Hz to Nyquist).
    pub low: f32,
    /// Param 3 "hiCutOff", 175 … 412.
    pub high: f32,
    /// Params 6 … 37, 0 … 265 (135 = 0 dB, 0 = the band removed).
    pub faders: [i32; BANDS],
}

impl Default for Controls {
    fn default() -> Self {
        Self { low: 40.0, high: 300.0, faders: [135; BANDS] }
    }
}

/// Needle position (0 … 418) → Hz.
pub fn needle_hz(pos: f32, sample_rate: f64) -> f32 {
    let nyq = (sample_rate * 0.5) as f32;
    let base = ((nyq as f64 / 18.0).powf(1.0 / 418.0)) as f32;
    nyq * pow_f(base, pos - 418.0)
}

/// Hz → needle position (inverse of [`needle_hz`]).
pub fn hz_needle(hz: f32, sample_rate: f64) -> f32 {
    let nyq = (sample_rate * 0.5) as f32;
    let base = ((nyq as f64 / 18.0).powf(1.0 / 418.0)) as f32;
    ln_f(hz / nyq) / ln_f(base) + 418.0
}

/// The equaliser curve: band layout and the convolution kernel's spectrum.
pub struct Filter {
    sr: f64,
    fft: RealFft,
    /// Private param 0x27 "freqs": the 32 band edges (the last is Nyquist).
    pub freqs: [f32; BANDS],
    /// Private param 0x28 "Conv Data": the kernel spectrum, packed.
    pub spectrum: Vec<f32>,
    /// 0x26 "Compare Thumbs", 0x2f / 0x30 "Old lo / hi freq".
    cache: [i16; BANDS],
    old_lo: f32,
    old_hi: f32,
    /// The needle positions after the original pushed them apart
    /// (`SetParamFloat(2/3)` in `UpdateFreqs`), for the editor.
    pub needles: (f32, f32),
}

impl Filter {
    pub fn new(sample_rate: f64) -> Self {
        Self {
            sr: sample_rate,
            fft: RealFft::new(N),
            freqs: [0.0; BANDS],
            spectrum: vec![0.0; N],
            cache: [-1; BANDS],
            old_lo: -1.0,
            old_hi: -1.0,
            needles: (40.0, 300.0),
        }
    }

    /// `UpdateFreqs`. Returns true when the kernel was rebuilt. When the
    /// needles are too close for 30 bands of at least one FFT bin, the one
    /// that moved is pushed back and written into `c` (the original's
    /// `SetParamFloat(2/3)`), as the host parameter would be.
    pub fn update(&mut self, c: &mut Controls) -> bool {
        let low_p = c.low.clamp(5.0, 408.0);
        let high_p = c.high.clamp(175.0, 412.0);
        let nyq = (self.sr * 0.5) as f32;
        let nyq_d = nyq as f64;
        let base = (nyq_d / 18.0).powf(f64::from_bits(0x3f63_991c_2c18_7f63)) as f32;
        let mut lo = nyq * pow_f(base, low_p - 418.0);
        let mut hi = nyq * pow_f(base, high_p - 418.0);
        self.needles = (low_p, high_p);
        let thumbs: [i16; BANDS] = std::array::from_fn(|k| c.faders[k].clamp(0, 0x109) as i16);
        if lo == self.old_lo && hi == self.old_hi && thumbs == self.cache {
            return false;
        }
        let gains: [f32; BANDS] = std::array::from_fn(|k| breakpoint(thumbs[k], &FADER_CURVE));

        // Bandwidth of one FFT bin, and the lowest/highest the needles may sit
        // so that 30 bands fit (constant width below 352 Hz, constant ratio above).
        let bin = ((nyq_d + nyq_d) / (N as f64)) as f32;
        let lo_min_hi = if lo <= 352.0 {
            ((((bin as f64 * 30.0 - 352.0) + lo as f64) / 352.0).exp() * 352.0) as f32
        } else {
            (lo as f64 * ((bin as f64 * 30.0) / lo as f64).exp()) as f32
        };
        let mut a0 = bin * -30.0 + hi;
        let mut f = a0;
        while ln_f(hi / f) * (f / 30.0) < bin {
            a0 = f;
            f *= 0.95;
        }
        while ln_f(hi / a0) * (a0 / 30.0) < bin {
            a0 *= 0.99;
        }
        if a0 < 352.0 {
            let l = ln_f(hi / 352.0);
            let k = (30.0 - l as f64 / ((bin / 352.0 + 1.0) as f64).ln()).ceil();
            a0 = 352.0 - bin * k as f32;
        }
        if a0 < lo || hi < lo_min_hi {
            if lo == self.old_lo || lo_min_hi <= hi {
                if hi != self.old_hi && a0 < lo {
                    self.needles.0 = (ln_f(a0 / nyq) / ln_f(base) + 418.0).clamp(5.0, 408.0);
                    c.low = self.needles.0;
                    lo = a0;
                }
            } else {
                self.needles.1 = (ln_f(lo_min_hi / nyq) / ln_f(base) + 418.0).clamp(175.0, 412.0);
                c.high = self.needles.1;
                hi = lo_min_hi;
            }
        }

        // Band edges.
        if lo <= 352.0 {
            let l = ln_f(hi / 352.0);
            let step = ((l + 1.0) * 352.0 - lo) * 0.033_333_335;
            let nlin = (352.0 - lo) / step;
            let lin = nlin.floor() as i32;
            let mut i = 0;
            // At low sample rates `lin` can pass 31: the original then writes
            // past its 32 edges (the 33rd float of "freqs" and beyond), which
            // nothing reads. Only the 32 edges are kept here.
            while i < lin {
                if (i as usize) < BANDS {
                    self.freqs[i as usize] = i as f32 * step + lo;
                }
                i += 1;
            }
            while i < 0x1f {
                self.freqs[i as usize] = exp_f((i as f32 - nlin) * (l / (30.0 - nlin))) * 352.0;
                i += 1;
            }
        } else {
            let ratio = pow_f(hi / lo, 0.033_333_335);
            let mut fr = lo;
            for k in 0..0x1f {
                self.freqs[k] = fr;
                fr *= ratio;
            }
        }
        self.freqs[0x1f] = nyq;

        // Gains → zero-phase spectrum → impulse → windowed linear-phase FIR
        // → its spectrum.
        thumbs_to_spectrum(&self.freqs, &gains, &mut self.spectrum);
        self.fft.inverse(&mut self.spectrum);
        half_shift_and_hann_pad(&mut self.spectrum);
        self.fft.forward(&mut self.spectrum);

        self.cache = thumbs;
        self.old_hi = hi;
        self.old_lo = lo;
        true
    }

    /// Kernel gain at a frequency (for the editor), from the packed spectrum
    /// of the FIR. (The staircase is drawn in vDSP's ×2 domain and the
    /// convolution multiplies vDSP spectra, so this is the actual gain.)
    pub fn gain_at(&self, hz: f32) -> f32 {
        let k = ((hz as f64 / (self.sr * 0.5)) * HALF as f64).round() as usize;
        let k = k.clamp(1, HALF - 1);
        let (re, im) = (self.spectrum[2 * k], self.spectrum[2 * k + 1]);
        (re * re + im * im).sqrt()
    }
}

/// `thumbs2fftcoefsHerm` @ 0x16bb4 (despite the name: a staircase — every
/// bin up to a band edge gets that band's gain, real parts only).
fn thumbs_to_spectrum(freqs: &[f32; BANDS], gains: &[f32; BANDS], out: &mut [f32]) {
    let nyq = freqs[0x1f];
    out.iter_mut().for_each(|v| *v = 0.0);
    let nf = (N as u32 >> 16) as f32 * 65536.0 + (N as u32 & 0xffff) as f32;
    let mut i = 0usize;
    let x = freqs[0] / nyq;
    loop {
        if 1.0 < x {
            break;
        }
        // (The original can write one float past the 4096 when a band edge
        // other than the last sits exactly at Nyquist; dropped here.)
        if i < out.len() {
            out[i] = gains[0];
        }
        i += 2;
        if !((i as f32) / nf < x) {
            break;
        }
    }
    for j in 0..0x1f {
        let x = freqs[j + 1] / nyq;
        if 1.0 < x {
            return;
        }
        loop {
            if i < out.len() {
                out[i] = gains[j + 1];
            }
            i += 2;
            if !((i as f32) / nf < x) {
                break;
            }
        }
    }
}

/// `hanning(float*, long)` @ 0x177b8: the angle step in single precision
/// (from a double 2π typed as 6.2831853056), the window and the product in
/// double.
fn hanning(x: &mut [f32]) {
    let n = x.len();
    let w = (6.283_185_305_6f64 / n as f32 as f64) as f32;
    for (i, v) in x.iter_mut().enumerate() {
        let c = cos_f(i as f32 * w) as f64;
        *v = ((c * -0.5 + 0.5) * *v as f64) as f32;
    }
}

/// `halfShiftAndHanPad` @ 0x198ee. The two `memmove`s count **bytes** equal
/// to `n`, i.e. n/4 floats: the impulse's last quarter goes in front of its
/// first quarter (a centred kernel of n/2), which is then Hann-windowed; the
/// second half is cleared.
fn half_shift_and_hann_pad(x: &mut [f32]) {
    let n = x.len();
    let q = n / 4;
    x.copy_within(0..q, q);
    x.copy_within(3 * q..n, 0);
    hanning(&mut x[..n / 2]);
    x[n / 2..].iter_mut().for_each(|v| *v = 0.0);
}

/// `pxyvzuffmgzhpzlobqearqbsnqaizyt` @ 0x16cc4: packed complex multiply.
fn multiply(x: &mut [f32], f: &[f32]) {
    x[0] *= f[0];
    x[1] *= f[1];
    for k in 1..x.len() / 2 {
        let (fr, fi) = (f[2 * k], f[2 * k + 1]);
        let (xr, xi) = (x[2 * k], x[2 * k + 1]);
        x[2 * k] = fr * xr - fi * xi;
        x[2 * k + 1] = fi * xr + fr * xi;
    }
}

pub struct Periscope {
    pub filter: Filter,
    fft: RealFft,
    inputs: Vec<Ring>,
    outputs: Vec<Ring>,
    pending: Vec<i32>,
    work: Vec<f32>,
    buffer: usize,
}

impl Periscope {
    /// `InitBuffers` @ 0x1a83e for `channels` in = out and a host buffer of
    /// `buffer` samples (the channel stagger depends on it).
    pub fn new(sample_rate: f64, channels: usize, buffer: usize) -> Self {
        let b = buffer as u32;
        let len = (2 * N as u32).max(b) * 4;
        let mut inputs = Vec::new();
        let mut outputs = Vec::new();
        let mut pending = Vec::new();
        for c in 0..channels {
            let offset = ((b * c as u32) % HALF as u32) as i32;
            pending.push(offset);
            let mut o = Ring::new(len as usize);
            o.offset_get(offset - (HALF as u32).max(b) as i32);
            outputs.push(o);
            let mut i = Ring::new(len as usize);
            i.offset_put(offset);
            inputs.push(i);
        }
        Self { filter: Filter::new(sample_rate), fft: RealFft::new(N), inputs, outputs, pending, work: vec![0.0; N], buffer }
    }

    pub fn channels(&self) -> usize {
        self.inputs.len()
    }

    /// The real delay. The output ring starts `max(N/2, buffer)` behind, so
    /// with host buffers above 2048 the signal comes `buffer − 2048` samples
    /// later than the 3072 the original reported to the host.
    pub fn latency(&self) -> u32 {
        LATENCY + (self.buffer as u32).saturating_sub(HALF as u32)
    }

    /// One `Process` call: `io[c]` holds channel `c`'s input and receives its output.
    pub fn process(&mut self, c: &mut Controls, io: &mut [&mut [f32]]) {
        self.filter.update(c);
        let n = io[0].len();
        for (ch, buf) in io.iter_mut().enumerate() {
            self.inputs[ch].put(buf);
            self.pending[ch] += n as i32;
            while HALF as i32 <= self.pending[ch] {
                self.inputs[ch].get_into(&mut self.work[..HALF]);
                self.pending[ch] -= HALF as i32;
                self.work[HALF..].iter_mut().for_each(|v| *v = 0.0);
                self.fft.forward(&mut self.work);
                multiply(&mut self.work, &self.filter.spectrum);
                self.fft.inverse(&mut self.work);
                let out = &mut self.outputs[ch];
                out.add_fast(&self.work[..HALF]);
                out.put(&self.work[HALF..]);
                out.offset_put(-(HALF as i32));
            }
        }
        for (ch, buf) in io.iter_mut().enumerate() {
            self.outputs[ch].get_into(buf);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flat_is_a_delay() {
        let mut p = Periscope::new(44100.0, 1, 512);
        let mut c = Controls::default();
        let mut out = vec![];
        for b in 0..20 {
            let mut x = vec![0.0f32; 512];
            if b == 0 {
                x[10] = 1.0;
            }
            p.process(&mut c, &mut [&mut x[..]]);
            out.extend_from_slice(&x);
        }
        let (k, v) = out.iter().enumerate().fold((0, 0.0f32), |a, (i, &v)| if v.abs() > a.1.abs() { (i, v) } else { a });
        assert_eq!(k, 10 + LATENCY as usize);
        assert!((v - 1.0).abs() < 0.02, "peak {v}");
    }

    #[test]
    fn latency_per_buffer_size() {
        for &b in &[64usize, 512, 2048, 3000, 4096] {
            let mut p = Periscope::new(44100.0, 3, b);
            let mut c = Controls::default();
            let mut outs = vec![vec![]; 3];
            for blk in 0..(16000 / b + 2) {
                let mut bufs: Vec<Vec<f32>> = (0..3).map(|_| vec![0.0f32; b]).collect();
                if blk == 0 {
                    bufs.iter_mut().for_each(|x| x[10] = 1.0);
                }
                let mut r: Vec<&mut [f32]> = bufs.iter_mut().map(|v| &mut v[..]).collect();
                p.process(&mut c, &mut r);
                for ch in 0..3 {
                    outs[ch].extend_from_slice(&bufs[ch]);
                }
            }
            for o in &outs {
                let k = o.iter().enumerate().fold((0, 0.0f32), |a, (i, &v)| if v.abs() > a.1 { (i, v.abs()) } else { a }).0;
                assert_eq!(k - 10, p.latency() as usize, "buffer {b}");
            }
        }
    }

    #[test]
    fn a_band_at_zero_removes_it() {
        let sr = 44100.0;
        let mut f = Filter::new(sr);
        let mut c = Controls::default();
        c.faders[16] = 0;
        f.update(&mut c);
        let mid = 0.5 * (f.freqs[15] + f.freqs[16]);
        let lo = 0.5 * (f.freqs[5] + f.freqs[6]);
        assert!(f.gain_at(mid) < 0.05, "{}", f.gain_at(mid));
        assert!((f.gain_at(lo) - 1.0).abs() < 0.05);
    }
}

#[cfg(test)]
mod low_rate {
    use super::*;
    #[test]
    fn needle_sweep() {
        for sr in [8000.0, 11025.0, 22050.0, 44100.0, 192000.0] {
            for lo in (5..=408).step_by(7) {
                for hi in (175..=412).step_by(5) {
                    let mut f = Filter::new(sr);
                    let mut c = Controls { low: lo as f32, high: hi as f32, faders: [100; BANDS] };
                    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| f.update(&mut c)));
                    assert!(r.is_ok(), "sr {sr} lo {lo} hi {hi}");
                }
            }
        }
    }
}
