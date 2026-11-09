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
        #[cfg(target_arch = "x86")]
        use std::arch::x86::{_mm_getcsr, _mm_setcsr};
        #[cfg(target_arch = "x86_64")]
        use std::arch::x86_64::{_mm_getcsr, _mm_setcsr};
        #[allow(deprecated)]
        let csr = _mm_getcsr();
        let was = csr & 0x8040 != 0;
        #[allow(deprecated)]
        _mm_setcsr(if on { csr | 0x8040 } else { csr & !0x8040 });
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
