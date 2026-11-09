//! The reservation behind both code arenas (LLVM's `arena.rs`, Cranelift's `hugearena.rs`): code
//! bump-allocated in a 2 MiB-aligned zone, dense in compile order, a chunk made a transparent huge
//! page once it holds 256 KiB, so a small run keeps its small pages. A second zone holds data within
//! 2 GiB of the code. Linux x86_64 only (the instruction cache is coherent there).

use std::ffi::{c_int, c_void};
use std::sync::atomic::{AtomicUsize, Ordering};

unsafe extern "C" {
    fn mmap(
        addr: *mut c_void,
        len: usize,
        prot: c_int,
        flags: c_int,
        fd: c_int,
        off: i64,
    ) -> *mut c_void;
    fn mprotect(addr: *mut c_void, len: usize, prot: c_int) -> c_int;
    fn madvise(addr: *mut c_void, len: usize, advice: c_int) -> c_int;
    fn getrlimit(resource: c_int, rlim: *mut [u64; 2]) -> c_int;
}

pub const PROT_RW: c_int = 3;
pub const PROT_RWX: c_int = 7;
pub const MAP_PRIVATE_ANON_NORESERVE: c_int = 0x2 | 0x20 | 0x4000;
const MADV_HUGEPAGE: c_int = 14;
const MADV_NOHUGEPAGE: c_int = 15;
const MADV_COLLAPSE: c_int = 25;
const RLIMIT_AS: c_int = 9;
const RLIM_INFINITY: u64 = u64::MAX;

const HUGE: usize = 2 << 20;
/// Bytes a chunk holds before it is made a huge page.
const HUGE_FROM: usize = 256 << 10;
/// Virtual sizes of the zones (nothing is committed until used).
const CODE: usize = 1 << 30;
const DATA: usize = 512 << 20;

/// Huge-page chunks made, and collapses given up (all arenas of this process).
pub static CHUNKS: AtomicUsize = AtomicUsize::new(0);
pub static FAILED: AtomicUsize = AtomicUsize::new(0);

pub struct Zone {
    start: usize,
    len: usize,
    used: usize,
    /// Per chunk (2 MiB): made a huge page (or given up on), and the fill it is tried at next.
    huge: Vec<(bool, usize)>,
}

impl Zone {
    /// The address of `size` bytes, or `None` when the zone is full.
    pub fn alloc(&mut self, size: usize, align: usize, huge: bool) -> Option<usize> {
        let at = (self.start + self.used).next_multiple_of(align.max(16));
        let end = at + size.max(1);
        if end > self.start + self.len {
            return None;
        }
        self.used = end - self.start;
        if huge {
            let first = (at - self.start) / HUGE;
            let last = (end - 1 - self.start) / HUGE;
            for k in first..=last {
                let in_chunk = (end - self.start).min((k + 1) * HUGE) - k * HUGE;
                if self.huge.len() <= k {
                    self.huge.resize(k + 1, (false, HUGE_FROM));
                }
                // A chunk the allocation runs past is full: collapse it now too. A collapse
                // that fails (no free huge page right now) is tried again at twice the fill.
                let (done, next) = &mut self.huge[k];
                if !*done && (in_chunk >= *next || k < last) {
                    let p = (self.start + k * HUGE) as *mut c_void;
                    // SAFETY: advice on pages of our own reservation; no content changes.
                    let ok = unsafe {
                        madvise(p, HUGE, MADV_HUGEPAGE) == 0 && madvise(p, HUGE, MADV_COLLAPSE) == 0
                    };
                    if ok {
                        *done = true;
                        CHUNKS.fetch_add(1, Ordering::Relaxed);
                    } else if k < last || *next >= HUGE / 2 {
                        *done = true;
                        FAILED.fetch_add(1, Ordering::Relaxed);
                    } else {
                        *next *= 2;
                    }
                }
            }
        }
        Some(at)
    }
}

pub struct Zones {
    pub code: Zone,
    pub data: Zone,
}

/// A fresh reservation, or `None` where the host refuses it: no address space, writable and
/// executable memory not allowed, or a limit on the address space (1.5 GiB of it could then
/// make a later allocation of the run fail).
pub fn reserve() -> Option<Zones> {
    let mut lim = [0u64; 2];
    // SAFETY: `lim` is the two words `getrlimit` writes.
    if unsafe { getrlimit(RLIMIT_AS, &mut lim) } != 0 || lim[0] != RLIM_INFINITY {
        return None;
    }
    // SAFETY: a fresh anonymous reservation; the zones get their protection below.
    let raw = unsafe {
        mmap(
            std::ptr::null_mut(),
            CODE + DATA + HUGE,
            0,
            MAP_PRIVATE_ANON_NORESERVE,
            -1,
            0,
        )
    };
    if raw as isize == -1 {
        return None;
    }
    let base = (raw as usize).next_multiple_of(HUGE);
    // SAFETY: protections and advice on the reservation made above.
    let ok = unsafe {
        mprotect(base as *mut c_void, CODE, PROT_RWX) == 0
            && mprotect((base + CODE) as *mut c_void, DATA, PROT_RW) == 0
    };
    if !ok {
        return None;
    }
    // SAFETY: as above; small pages until a chunk is worth a huge one. Refused only where the
    // kernel has no huge pages, and then nothing is lost.
    unsafe { madvise(base as *mut c_void, CODE, MADV_NOHUGEPAGE) };
    let zone = |start, len| Zone {
        start,
        len,
        used: 0,
        huge: Vec::new(),
    };
    Some(Zones {
        code: zone(base, CODE),
        data: zone(base + CODE, DATA),
    })
}

/// A mapping for one allocation that did not fit its zone, null when refused.
pub fn map(size: usize, prot: c_int) -> *mut u8 {
    // SAFETY: a fresh anonymous mapping.
    let p = unsafe {
        mmap(
            std::ptr::null_mut(),
            size.max(1).next_multiple_of(4096),
            prot,
            MAP_PRIVATE_ANON_NORESERVE,
            -1,
            0,
        )
    };
    if p as isize == -1 {
        std::ptr::null_mut()
    } else {
        p.cast()
    }
}
