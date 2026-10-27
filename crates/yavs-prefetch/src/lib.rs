//! A prefetch hint (#766, `docs/decisions.md` "Prefetch of the next process in a batch").
//!
//! A prefetch never faults and changes no program state, whatever the address: [`lines`] is safe
//! to call on any reference. On a target without a prefetch instruction it does nothing.

/// Cache line size the hint steps by.
pub const LINE: usize = 64;

/// Ask the CPU to load the first `n` cache lines of `*p` into the cache, without waiting.
#[inline(always)]
pub fn lines<T: ?Sized>(p: &T, n: usize) {
    let base = std::ptr::from_ref(p).cast::<u8>();
    for k in 0..n {
        hint(base.wrapping_add(k * LINE));
    }
}

#[inline(always)]
#[allow(unsafe_code)]
fn hint(p: *const u8) {
    #[cfg(target_arch = "x86_64")]
    // SAFETY: a prefetch is a hint: it cannot fault or change memory, for any address.
    unsafe {
        core::arch::x86_64::_mm_prefetch::<{ core::arch::x86_64::_MM_HINT_T0 }>(p.cast());
    }
    #[cfg(not(target_arch = "x86_64"))]
    let _ = p;
}
