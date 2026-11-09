//! `CVecLibFFT`: the real FFT of the Mission framework, a thin wrapper over
//! Apple's vecLib (`create_fftsetup`, `ctoz`, `fft_zrip`, `ztoc`).
//!
//! Data layout ("packed real"): `n` floats, `x[0]` = DC, `x[1]` = Nyquist,
//! then interleaved `re, im` for bins 1 … n/2−1. The forward transform is
//! vDSP's: twice the mathematical DFT. The inverse scales by `0.5 / n`, so a
//! round trip returns the input (`CVecLibFFT::vbibiy…` in PeriScope @ 0x1d8c6).
//!
//! On macOS this calls the very same vDSP routines through the Accelerate
//! framework. Elsewhere a portable radix-2 implementation with the same
//! layout and scaling is used (equal to vDSP within float rounding).

/// vDSP chooses its kernel by the alignment of the split buffers, and the
/// kernels round differently; the split buffers are therefore always
/// 64-byte aligned, so results never depend on the allocator.
#[repr(C, align(64))]
#[derive(Clone, Copy)]
struct Line([f32; 16]);

struct Aligned(Vec<Line>, usize);

impl Aligned {
    fn new(n: usize) -> Self {
        Aligned(vec![Line([0.0; 16]); n.div_ceil(16)], n)
    }
}

impl std::ops::Deref for Aligned {
    type Target = [f32];
    fn deref(&self) -> &[f32] {
        unsafe { std::slice::from_raw_parts(self.0.as_ptr() as *const f32, self.1) }
    }
}

impl std::ops::DerefMut for Aligned {
    fn deref_mut(&mut self) -> &mut [f32] {
        unsafe { std::slice::from_raw_parts_mut(self.0.as_mut_ptr() as *mut f32, self.1) }
    }
}

pub struct RealFft {
    n: usize,
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))]
    log2n: u32,
    re: Aligned,
    im: Aligned,
    #[cfg(target_os = "macos")]
    setup: *mut std::ffi::c_void,
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    twiddle: Vec<(f32, f32)>,
}

// The setup is immutable after creation and only used through &mut self.
unsafe impl Send for RealFft {}

#[cfg(target_os = "macos")]
mod vdsp {
    use std::ffi::c_void;
    #[repr(C)]
    pub struct Split {
        pub realp: *mut f32,
        pub imagp: *mut f32,
    }
    #[link(name = "Accelerate", kind = "framework")]
    unsafe extern "C" {
        pub fn vDSP_create_fftsetup(log2n: usize, radix: i32) -> *mut c_void;
        pub fn vDSP_destroy_fftsetup(setup: *mut c_void);
        pub fn vDSP_fft_zrip(setup: *mut c_void, c: *const Split, stride: isize, log2n: usize, dir: i32);
        pub fn vDSP_ctoz(c: *const f32, ic: isize, z: *const Split, iz: isize, n: usize);
        pub fn vDSP_ztoc(z: *const Split, iz: isize, c: *mut f32, ic: isize, n: usize);
    }
}

impl RealFft {
    /// `CVecLibFFT::Init(n)` @ 0x1d7b0 (`n` a power of two).
    pub fn new(n: usize) -> Self {
        assert!(n.is_power_of_two() && n >= 4);
        let log2n = n.trailing_zeros();
        let h = n / 2;
        let twiddle = (0..h)
            .map(|k| {
                let a = -2.0 * std::f64::consts::PI * k as f64 / n as f64;
                (a.cos() as f32, a.sin() as f32)
            })
            .collect();
        #[cfg(target_os = "macos")]
        {
            let setup = unsafe { vdsp::vDSP_create_fftsetup(log2n as usize, 0) };
            assert!(!setup.is_null(), "vDSP_create_fftsetup failed");
            Self { n, log2n, re: Aligned::new(h), im: Aligned::new(h), setup, twiddle }
        }
        #[cfg(not(target_os = "macos"))]
        {
            Self { n, log2n, re: Aligned::new(h), im: Aligned::new(h), twiddle }
        }
    }

    pub fn len(&self) -> usize {
        self.n
    }

    /// Forward, in place (`gvtcotu…` @ 0x1d83a).
    pub fn forward(&mut self, x: &mut [f32]) {
        self.transform(x, 1);
    }

    /// Inverse, in place, then `x *= 0.5 / n` (`vbibiy…` @ 0x1d8c6).
    pub fn inverse(&mut self, x: &mut [f32]) {
        self.transform(x, -1);
        let s = 0.5 / ((self.n as u32 >> 16) as f32 * 65536.0 + (self.n as u32 & 0xffff) as f32);
        for v in x.iter_mut().take(self.n) {
            *v = s * *v;
        }
    }

    #[cfg(target_os = "macos")]
    fn transform(&mut self, x: &mut [f32], dir: i32) {
        let h = self.n / 2;
        let split = vdsp::Split { realp: self.re.as_mut_ptr(), imagp: self.im.as_mut_ptr() };
        unsafe {
            vdsp::vDSP_ctoz(x.as_ptr(), 2, &split, 1, h);
            vdsp::vDSP_fft_zrip(self.setup, &split, 1, self.log2n as usize, dir);
            vdsp::vDSP_ztoc(&split, 1, x.as_mut_ptr(), 2, h);
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn transform(&mut self, x: &mut [f32], dir: i32) {
        self.transform_portable(x, dir);
    }

    /// Portable version of vDSP's packed real FFT (Linux, Windows).
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    fn transform_portable(&mut self, x: &mut [f32], dir: i32) {
        let n = self.n;
        let h = n / 2;
        let w = |k: usize, tw: &[(f32, f32)]| {
            let (c, s) = tw[k % h];
            if dir > 0 { (c, s) } else { (c, -s) }
        };
        if dir > 0 {
            // z[k] = x[2k] + i x[2k+1]; complex FFT of size h
            for k in 0..h {
                self.re[k] = x[2 * k];
                self.im[k] = x[2 * k + 1];
            }
            complex_fft(&mut self.re, &mut self.im, false);
            // Unpack: X[k] = (Z[k] + conj Z[h−k])/2 − i/2 W^k (Z[k] − conj Z[h−k]); vDSP gives 2·X.
            let (r0, i0) = (self.re[0], self.im[0]);
            x[0] = 2.0 * (r0 + i0);
            x[1] = 2.0 * (r0 - i0);
            for k in 1..h {
                let (zr, zi) = (self.re[k], self.im[k]);
                let (cr, ci) = (self.re[h - k], -self.im[h - k]);
                let (er, ei) = (zr + cr, zi + ci);
                let (or_, oi) = (zr - cr, zi - ci);
                let (c, s) = w(k, &self.twiddle);
                // −i·W·(Z − conj Z')
                let (tr, ti) = (or_ * c - oi * s, or_ * s + oi * c);
                x[2 * k] = er + ti;
                x[2 * k + 1] = ei - tr;
            }
        } else {
            // vDSP's unscaled inverse: x[n] = Σ_k Y[k]·e^{+2πikn/N} over the
            // Hermitian spectrum Y. With z[m] = x[2m] + i·x[2m+1]:
            // Z[k] = (Y[k] + conj Y[h−k]) + i·(Y[k] − conj Y[h−k])·e^{+2πik/N},
            // z = the unscaled inverse complex FFT of Z (size h).
            let (dc, ny) = (x[0], x[1]);
            for k in 0..h {
                let (yr, yi) = if k == 0 { (dc, 0.0) } else { (x[2 * k], x[2 * k + 1]) };
                let j = h - k;
                let (cr, ci) = if j == h { (ny, 0.0) } else { (x[2 * j], -x[2 * j + 1]) };
                let (er, ei) = (yr + cr, yi + ci);
                let (dr, di) = (yr - cr, yi - ci);
                let (c, s) = self.twiddle[k];
                // e^{+2πik/N} = (c, −s)
                let (or_, oi) = (dr * c + di * s, di * c - dr * s);
                self.re[k] = er - oi;
                self.im[k] = ei + or_;
            }
            complex_fft(&mut self.re, &mut self.im, true);
            for k in 0..h {
                x[2 * k] = self.re[k];
                x[2 * k + 1] = self.im[k];
            }
        }
    }
}

#[cfg(target_os = "macos")]
impl Drop for RealFft {
    fn drop(&mut self) {
        unsafe { vdsp::vDSP_destroy_fftsetup(self.setup) };
    }
}

/// In-place iterative radix-2 complex FFT (unscaled).
#[cfg_attr(target_os = "macos", allow(dead_code))]
fn complex_fft(re: &mut [f32], im: &mut [f32], inverse: bool) {
    let n = re.len();
    let mut j = 0;
    for i in 1..n {
        let mut bit = n >> 1;
        while j & bit != 0 {
            j ^= bit;
            bit >>= 1;
        }
        j |= bit;
        if i < j {
            re.swap(i, j);
            im.swap(i, j);
        }
    }
    let mut len = 2;
    while len <= n {
        let a = if inverse { 2.0 } else { -2.0 } * std::f64::consts::PI / len as f64;
        for start in (0..n).step_by(len) {
            for k in 0..len / 2 {
                let (s, c) = (a * k as f64).sin_cos();
                let (c, s) = (c as f32, s as f32);
                let (ur, ui) = (re[start + k], im[start + k]);
                let (vr0, vi0) = (re[start + k + len / 2], im[start + k + len / 2]);
                let (vr, vi) = (vr0 * c - vi0 * s, vr0 * s + vi0 * c);
                re[start + k] = ur + vr;
                im[start + k] = ui + vi;
                re[start + k + len / 2] = ur - vr;
                im[start + k + len / 2] = ui - vi;
            }
        }
        len <<= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_and_scaling() {
        let n = 64;
        let mut f = RealFft::new(n);
        let x: Vec<f32> = (0..n).map(|i| ((i * 7 % 13) as f32 - 6.0) * 0.1).collect();
        let mut y = x.clone();
        f.forward(&mut y);
        // DC (packed in y[0]) is twice the sum
        let sum: f32 = x.iter().sum();
        assert!((y[0] - 2.0 * sum).abs() < 1e-4);
        // bin 1 against a direct DFT
        let (mut br, mut bi) = (0.0f64, 0.0f64);
        for (i, v) in x.iter().enumerate() {
            let a = -2.0 * std::f64::consts::PI * i as f64 / n as f64;
            br += *v as f64 * a.cos();
            bi += *v as f64 * a.sin();
        }
        assert!((y[2] as f64 - 2.0 * br).abs() < 1e-3 && (y[3] as f64 - 2.0 * bi).abs() < 1e-3);
        f.inverse(&mut y);
        for (a, b) in x.iter().zip(&y) {
            assert!((a - b).abs() < 1e-5);
        }
    }

    /// The portable transform (what Linux and Windows run) against vDSP.
    #[cfg(target_os = "macos")]
    #[test]
    fn portable_matches_vdsp() {
        for &n in &[16usize, 256, 4096] {
            let mut f = RealFft::new(n);
            let x: Vec<f32> = (0..n).map(|i| ((i * 7919 % 101) as f32 - 50.0) * 0.013 + (i as f32 * 0.05).sin()).collect();
            for dir in [1, -1] {
                let (mut a, mut b) = (x.clone(), x.clone());
                f.transform(&mut a, dir);
                f.transform_portable(&mut b, dir);
                let peak = a.iter().fold(0.0f32, |m, v| m.max(v.abs()));
                for (i, (p, q)) in a.iter().zip(&b).enumerate() {
                    assert!((p - q).abs() <= 1e-5 * peak, "n {n} dir {dir} [{i}]: vDSP {p} portable {q}");
                }
            }
        }
    }
}
