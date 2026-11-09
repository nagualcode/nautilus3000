//! Grain envelopes: `ptoqca…::InitEnvelopes()` @ 0x1a616 builds one master
//! table of `gShapeLength` (= one second of samples) points, and `Run` resamples
//! it to each grain's length.

/// Shape codes stored at `engine + 0x52`.
pub const SHAPE_LINEAR: i16 = 0;
pub const SHAPE_LINEAR_B: i16 = 1;
pub const SHAPE_COSINE: i16 = 2;
pub const SHAPE_SHARP: i16 = 3;

/// The π literal in the original source (`0x400921fb54290a00`). It is
/// 3.1415926528, not 3.14159265358…: the last digits were mistyped. Kept.
pub const ORIGINAL_PI: f64 = f64::from_bits(0x4009_21fb_5429_0a00);

/// The fall of the Sharp attack from 1.0 to 0.2, as stored in the binary.
const SHARP_FALL: f64 = f64::from_bits(0x3fe9_9999_9800_0000);

/// The Envelope menu (param 7: 1 Triangle, 2 Square, 3 Sharp attack, 4 Cosine)
/// → `(shape, attack, decay)`, as set in `Process` @ 0x21200.
pub fn menu_to_shape(choice: i32) -> Option<(i16, f64, f64)> {
    match choice {
        1 => Some((SHAPE_LINEAR, 0.5, 0.5)),
        2 => Some((SHAPE_LINEAR_B, 0.05, 0.05)),
        3 => Some((SHAPE_SHARP, 0.05, 0.3)),
        4 => Some((SHAPE_COSINE, 0.5, 0.5)),
        _ => None,
    }
}

/// `(double)u` then saturate/truncate to `u32`, the GCC idiom for
/// `(unsigned long)(double)`.
#[inline]
pub fn d2u(v: f64) -> u32 {
    if v.is_nan() || v <= 0.0 {
        0
    } else if v >= 4_294_967_295.0 {
        u32::MAX
    } else {
        v as u32
    }
}

/// `(float)u` for an unsigned value, done by GCC as `hi*65536 + lo` in single
/// precision: exact below 2^24 and correctly rounded above.
#[inline]
pub fn u2f(v: u32) -> f32 {
    (v >> 16) as f32 * 65536.0 + (v & 0xffff) as f32
}

/// Fills `master` (length = gShapeLength) for `shape`, `attack` and `decay`
/// given as fractions of the grain.
pub fn init_master(master: &mut [f32], shape: i16, attack: f64, decay: f64) {
    let len = master.len() as u32;
    let a = d2u(len as f64 * attack);
    let d = d2u(len as f64 * decay);
    match shape {
        SHAPE_COSINE => {
            let mut i = 0u32;
            while i < a {
                let x = (u2f(i) / u2f(a)) as f64 * ORIGINAL_PI;
                master[i as usize] = (x.cos() * -0.5 + 0.5) as f32;
                i += 1;
            }
            while i < len.wrapping_sub(d) {
                master[i as usize] = 1.0;
                i += 1;
            }
            for j in 0..d {
                let x = (u2f(j) / u2f(d)) as f64 * ORIGINAL_PI;
                master[(len - 1 - j) as usize] = (x.cos() * -0.5 + 0.5) as f32;
            }
        }
        SHAPE_LINEAR | SHAPE_LINEAR_B => {
            let mut i = 0u32;
            while i < a {
                master[i as usize] = u2f(i) / u2f(a);
                i += 1;
            }
            while i < len.wrapping_sub(d) {
                master[i as usize] = 1.0;
                i += 1;
            }
            for j in 0..d {
                master[(len - 1 - j) as usize] = u2f(j) / u2f(d);
            }
        }
        SHAPE_SHARP => {
            // Rise over 40 % of the attack, fall to 0.2 over the rest of it,
            // hold 0.2, then fall by 0.2 over the decay. The value carries on
            // from wherever the float ramps ended (≈0.2), not from 0.2 exactly.
            let a40 = d2u(a as f64 * 0.4);
            let mut f = 0.0f32;
            let up = 1.0f32 / u2f(a40);
            let mut i = 0u32;
            while i < a40 {
                master[i as usize] = f;
                f += up;
                i += 1;
            }
            // The "0.8" in the binary is 0x3fe9999998000000 (0.79999999702):
            // neither the double nor the float nearest to 0.8. Kept bit for bit.
            let down = (0.0 - SHARP_FALL / u2f(a - a40) as f64) as f32;
            while i < a {
                master[i as usize] = f;
                f += down;
                i += 1;
            }
            let sustain_end = len.wrapping_sub(d);
            while i < sustain_end {
                master[i as usize] = 0.2;
                i += 1;
            }
            let rest = len - sustain_end;
            let fall = 0.0f32 - 0.2f32 / u2f(rest);
            while i < len {
                master[i as usize] = f;
                f += fall;
                i += 1;
            }
        }
        _ => {}
    }
}

/// Resamples the master table to a grain of `n` samples, the envelope copy in
/// `Run` @ 0x1b1c2. Index = `(int)((double)i * (double)(float)(shapeLen / n))`.
/// For shapes 0–2 only the attack and decay regions are copied; the middle is 1.
pub fn fill_grain_env(env: &mut [f32], master: &[f32], shape: i16, n: u32, a: u32, d: u32) {
    let scale = (u2f(master.len() as u32) / u2f(n)) as f64;
    let idx = |i: u32| ((i as f64) * scale) as i32 as usize;
    if shape == SHAPE_SHARP {
        for i in 0..n {
            env[i as usize] = master[idx(i)];
        }
    } else {
        let mut i = 0u32;
        while i < a {
            env[i as usize] = master[idx(i)];
            i += 1;
        }
        while i < n.wrapping_sub(d) {
            env[i as usize] = 1.0;
            i += 1;
        }
        while i < n {
            env[i as usize] = master[idx(i)];
            i += 1;
        }
    }
}

/// Applies the envelope to a grain, `Run` @ 0x1b323.
///
/// For shapes 0–2 the decay loop of the original walks down from index `n`
/// (one past the grain) and stops one sample early, so the sample at `n - d`
/// keeps full level and index `n` (outside the grain) is touched. Reproduced.
pub fn apply(grain: &mut [f32], env: &[f32], shape: i16, n: u32, a: u32, d: u32) {
    if shape == SHAPE_SHARP {
        for i in 0..n as usize {
            grain[i] *= env[i];
        }
    } else {
        for i in 0..a as usize {
            grain[i] *= env[i];
        }
        let mut k = n;
        let stop = n.wrapping_sub(d);
        let mut idx = n as usize;
        while k > stop {
            k -= 1;
            grain[idx] *= env[idx];
            idx = idx.wrapping_sub(1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn square_is_a_trapezoid() {
        let mut m = vec![0.0; 44100];
        init_master(&mut m, SHAPE_LINEAR_B, 0.05, 0.05);
        assert_eq!(m[0], 0.0);
        assert_eq!(m[22050], 1.0);
        assert_eq!(m[44099], 0.0);
        assert!((m[1102] - 1102.0 / 2205.0).abs() < 1e-7);
    }

    #[test]
    fn sharp_attack_shape() {
        let mut m = vec![0.0; 44100];
        init_master(&mut m, SHAPE_SHARP, 0.05, 0.3);
        let peak = m.iter().cloned().fold(0.0, f32::max);
        assert!((peak - 1.0).abs() < 1e-3);
        assert_eq!(m[20000], 0.2);
        assert!(m[44099].abs() < 1e-3);
    }

    #[test]
    fn cosine_uses_the_original_pi() {
        let mut m = vec![0.0; 1000];
        init_master(&mut m, SHAPE_COSINE, 0.5, 0.5);
        assert_eq!(m[0], 0.0);
        assert!(m[499] > 0.99);
    }
}
