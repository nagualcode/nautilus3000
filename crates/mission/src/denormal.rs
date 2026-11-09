//! `DenormalFlush`: the Mission framework sets the SSE unit to flush
//! denormals to zero (MXCSR FTZ | DAZ) for the duration of every `Process`.
//! Here: FTZ/DAZ on x86, FPCR.FZ on aarch64.

/// The original's `Process` runs inside `DenormalFlush` (flush-to-zero and
/// denormals-are-zero in MXCSR). The plug-in sets the same mode on the audio
/// thread; this sets it for offline use. Returns the previous state.
pub fn flush_denormals(on: bool) -> bool {
    #[cfg(target_arch = "aarch64")]
    unsafe {
        let mut fpcr: u64;
        std::arch::asm!("mrs {}, fpcr", out(reg) fpcr);
        let was = fpcr & (1 << 24) != 0;
        if on { fpcr |= 1 << 24 } else { fpcr &= !(1 << 24) }
        std::arch::asm!("msr fpcr, {}", in(reg) fpcr);
        was
    }
    #[cfg(any(target_arch = "x86_64", target_arch = "x86"))]
    unsafe {
        // MXCSR bit 15 = flush to zero, bit 6 = denormals are zero.
        let mut csr: u32 = 0;
        std::arch::asm!("stmxcsr [{}]", in(reg) &mut csr, options(nostack));
        let was = csr & 0x8040 != 0;
        let new = if on { csr | 0x8040 } else { csr & !0x8040 };
        std::arch::asm!("ldmxcsr [{}]", in(reg) &new, options(nostack));
        was
    }
    #[cfg(not(any(target_arch = "aarch64", target_arch = "x86_64", target_arch = "x86")))]
    {
        let _ = on;
        false
    }
}

/// RAII guard: flush on while alive, previous mode restored on drop.
pub struct DenormalFlush(bool);

impl DenormalFlush {
    pub fn on() -> Self {
        DenormalFlush(flush_denormals(true))
    }
}

impl Drop for DenormalFlush {
    fn drop(&mut self) {
        flush_denormals(self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flushes_and_restores() {
        let tiny = std::hint::black_box(1.0e-38f32);
        let was = flush_denormals(true);
        let flushed = std::hint::black_box(tiny) * std::hint::black_box(0.01f32);
        flush_denormals(false);
        let kept = std::hint::black_box(tiny) * std::hint::black_box(0.01f32);
        flush_denormals(was);
        assert_eq!(flushed, 0.0);
        assert!(kept > 0.0);
    }
}
