//! The circular buffer of the Mission framework (class `ekmdvrkalxdgqoeajnnmnqbvjhvlvgw` in
//! every obfuscated Nautilus binary; Riverrun's methods at 0x27ba–0x3580).
//!
//! It has an independent put and get position. Every method re-normalises its
//! position the way the original does: positions below the length are pulled up
//! by adding the length, positions at or above it are reduced with `%`. Those two
//! paths are not the same for out-of-range values, so they are kept separately.

/// `pos` normalised exactly as the original (`if pos < len {while pos<0 {pos+=len}} else {pos%=len}`).
#[inline]
pub fn wrap(mut pos: i32, len: i32) -> i32 {
    if pos < len {
        while pos < 0 {
            pos += len;
        }
        pos
    } else {
        pos % len
    }
}

#[derive(Clone, Debug)]
pub struct Ring {
    pub buf: Vec<f32>,
    pub put: i32,
    pub get: i32,
    pub len: i32,
}

impl Ring {
    /// `Init(len, blocksize)` @ 0x2a7c: zeroed buffer, both positions at 0.
    pub fn new(len: usize) -> Self {
        Self { buf: vec![0.0; len], put: 0, get: 0, len: len as i32 }
    }

    pub fn clear(&mut self) {
        self.buf.iter_mut().for_each(|s| *s = 0.0);
    }

    /// `SetPutPos` @ 0x2814.
    #[inline]
    pub fn set_put(&mut self, pos: i32) {
        self.put = wrap(pos, self.len);
    }

    /// `OffsetPutPos` @ 0x2848.
    #[inline]
    pub fn offset_put(&mut self, delta: i32) {
        self.put = wrap(self.put.wrapping_add(delta), self.len);
    }

    /// `SetGetPos` @ 0x28a6.
    #[inline]
    pub fn set_get(&mut self, pos: i32) {
        self.get = wrap(pos, self.len);
    }

    /// `OffsetGetPos`.
    #[inline]
    pub fn offset_get(&mut self, delta: i32) {
        self.get = wrap(self.get.wrapping_add(delta), self.len);
    }

    /// `AddFast(float*, long)`: sum in at the put position, advance it
    /// (the same as [`Ring::add_grain`]).
    pub fn add_fast(&mut self, src: &[f32]) {
        self.add_grain(src);
    }

    /// `Put(float*, long)` @ 0x2cb0: copy in at the put position, advance it.
    pub fn put(&mut self, src: &[f32]) {
        self.put = wrap(self.put, self.len);
        let n = src.len() as i32;
        let first = self.len - self.put;
        if first < n {
            let p = self.put as usize;
            self.buf[p..p + first as usize].copy_from_slice(&src[..first as usize]);
            let rest = (n - first) as usize;
            self.buf[..rest].copy_from_slice(&src[first as usize..]);
        } else {
            let p = self.put as usize;
            self.buf[p..p + n as usize].copy_from_slice(src);
        }
        self.put = wrap(self.put + n, self.len);
    }

    /// `Get(float*, long)` @ 0x3092: copy out from the get position, advance it.
    pub fn get_into(&mut self, dst: &mut [f32]) {
        self.get = wrap(self.get, self.len);
        let n = dst.len() as i32;
        let first = self.len - self.get;
        let g = self.get as usize;
        if first < n {
            dst[..first as usize].copy_from_slice(&self.buf[g..g + first as usize]);
            let rest = (n - first) as usize;
            dst[first as usize..].copy_from_slice(&self.buf[..rest]);
        } else {
            dst.copy_from_slice(&self.buf[g..g + n as usize]);
        }
        self.get = wrap(self.get + n, self.len);
    }

    /// `GetAndClear(float*, long)` @ 0x34ee: read and zero `dst.len()` samples.
    /// (Unlike the other methods it does not re-normalise `get` first.)
    pub fn get_and_clear(&mut self, dst: &mut [f32]) {
        let n = dst.len() as i32;
        let g = self.get as usize;
        let first = self.len - self.get;
        if n < first {
            dst.copy_from_slice(&self.buf[g..g + n as usize]);
            self.buf[g..g + n as usize].iter_mut().for_each(|s| *s = 0.0);
        } else {
            let f = first as usize;
            dst[..f].copy_from_slice(&self.buf[g..g + f]);
            self.buf[g..g + f].iter_mut().for_each(|s| *s = 0.0);
            let rest = (n - first) as usize;
            dst[f..].copy_from_slice(&self.buf[..rest]);
            self.buf[..rest].iter_mut().for_each(|s| *s = 0.0);
        }
        self.get = wrap(self.get + n, self.len);
    }

    /// `hgiovh…::AddGrain(float*, long)` @ 0x1c4aa: sum a grain in at the put
    /// position (plain float adds, unrolled by 4 in the original — same result).
    pub fn add_grain(&mut self, grain: &[f32]) {
        self.set_put(self.put);
        let n = grain.len() as i32;
        let p = self.put as usize;
        let first = self.len - self.put;
        if n < first {
            for (d, s) in self.buf[p..p + n as usize].iter_mut().zip(grain) {
                *d += *s;
            }
        } else {
            let f = first as usize;
            for (d, s) in self.buf[p..p + f].iter_mut().zip(&grain[..f]) {
                *d += *s;
            }
            let rest = (n - first) as usize;
            for (d, s) in self.buf[..rest].iter_mut().zip(&grain[f..]) {
                *d += *s;
            }
        }
        self.offset_put(n);
    }

    /// `GetPitched(float*, float ratio, long n)` @ 0x3166: read `n` samples from the
    /// get position, stepping by `ratio` with linear interpolation.
    ///
    /// The read position is accumulated in **single precision**, like the
    /// original. Far into a 4.65 s buffer the float step loses a few bits, which
    /// is part of Riverrun's sound and is kept on purpose.
    pub fn get_pitched(&mut self, dst: &mut [f32], ratio: f32, n: usize) {
        self.get = wrap(self.get, self.len);
        let len = self.len;
        let mut pos = self.get as f32;
        for out in dst.iter_mut().take(n) {
            let i = pos as i32; // cvttss2si
            let frac = pos - i as f32;
            let a = if i >= len { i - len } else { i };
            let mut b = a + 1;
            if b >= len {
                b -= len;
            }
            pos += ratio;
            *out = (1.0 - frac) * self.buf[a as usize] + frac * self.buf[b as usize];
        }
        let adv = (n as i32 as f32 * ratio) as i32;
        self.get = wrap(adv.wrapping_add(self.get), len);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_matches_original_paths() {
        assert_eq!(wrap(-1, 10), 9);
        assert_eq!(wrap(-21, 10), 9);
        assert_eq!(wrap(25, 10), 5);
        assert_eq!(wrap(3, 10), 3);
    }

    #[test]
    fn put_get_round_trip_across_the_end() {
        let mut r = Ring::new(8);
        r.set_put(6);
        r.put(&[1.0, 2.0, 3.0, 4.0]);
        assert_eq!(r.put, 2);
        r.set_get(6);
        let mut out = [0.0; 4];
        r.get_into(&mut out);
        assert_eq!(out, [1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn get_and_clear_zeroes() {
        let mut r = Ring::new(4);
        r.add_grain(&[1.0, 1.0, 1.0]);
        let mut o = [0.0; 3];
        r.get_and_clear(&mut o);
        assert_eq!(o, [1.0; 3]);
        assert!(r.buf.iter().all(|&s| s == 0.0));
    }
}
