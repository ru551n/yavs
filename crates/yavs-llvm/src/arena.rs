//! One contiguous arena for the machine code of every JIT of the process (#679).
//!
//! ORC's default linker maps the code of each module where the kernel puts it, in 4 KiB pages: a
//! large compiled process (hundreds of KiB to megabytes of straight-line code that every
//! activation runs through) then costs an instruction-TLB entry per page. Here the object layer
//! is RuntimeDyld with our own section allocator: code sections are bump-allocated one after the
//! other in a 2 MiB-aligned region (hot code dense in memory, in the order it was compiled), and
//! a 2 MiB chunk of it is asked to be a transparent huge page (`madvise`, then `MADV_COLLAPSE` of
//! what is there) once it holds 256 KiB, so a small run keeps its small pages. Data sections go
//! to a second zone of the same reservation (code and data stay within 2 GiB of each other).
//! Linux x86_64 only (the instruction cache is coherent there); everywhere else, and when the
//! reservation fails or `YAVS_LLVM_ARENA=0`, LLJIT keeps its default linker.
//! The arena is also what lets the code be generated for the small code model (`jit.rs` `model`).
//! `YAVS_LLVM_HUGE=0` keeps the arena but leaves the huge pages out.

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod imp {
    use std::ffi::{c_char, c_int, c_uint, c_void};
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::{Mutex, OnceLock, PoisonError};

    use llvm_sys::orc2::ee::*;
    use llvm_sys::orc2::*;

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
    }

    const PROT_RW: c_int = 3;
    const PROT_RWX: c_int = 7;
    const MAP_PRIVATE_ANON_NORESERVE: c_int = 0x2 | 0x20 | 0x4000;
    const MADV_HUGEPAGE: c_int = 14;
    const MADV_NOHUGEPAGE: c_int = 15;
    const MADV_COLLAPSE: c_int = 25;

    const HUGE: usize = 2 << 20;
    /// Bytes a chunk holds before it is made a huge page.
    const HUGE_FROM: usize = 256 << 10;
    /// Virtual sizes of the zones (nothing is committed until used).
    const CODE: usize = 1 << 30;
    const DATA: usize = 512 << 20;

    /// Code bytes placed, huge-page chunks made, collapses given up, sections that did not fit
    /// the arena (all JITs of this process).
    static PLACED: AtomicUsize = AtomicUsize::new(0);
    static CHUNKS: AtomicUsize = AtomicUsize::new(0);
    static FAILED: AtomicUsize = AtomicUsize::new(0);
    static OVERFLOW: AtomicUsize = AtomicUsize::new(0);

    struct Zone {
        start: usize,
        len: usize,
        used: usize,
        /// Per chunk (2 MiB): made a huge page (or given up on), and the fill it is tried at next.
        huge: Vec<(bool, usize)>,
    }

    impl Zone {
        fn alloc(&mut self, size: usize, align: usize, huge: bool) -> Option<usize> {
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
                            madvise(p, HUGE, MADV_HUGEPAGE) == 0
                                && madvise(p, HUGE, MADV_COLLAPSE) == 0
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

    struct Arena {
        code: Zone,
        data: Zone,
        huge: bool,
    }

    static ARENA: OnceLock<Option<Mutex<Arena>>> = OnceLock::new();

    fn arena() -> Option<&'static Mutex<Arena>> {
        ARENA
            .get_or_init(|| {
                if std::env::var("YAVS_LLVM_ARENA").is_ok_and(|v| v == "0") {
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
                let zone = |start, len| Zone {
                    start,
                    len,
                    used: 0,
                    huge: Vec::new(),
                };
                // SAFETY: protections and advice on the reservation made above.
                let ok = unsafe {
                    mprotect(base as *mut c_void, CODE, PROT_RWX) == 0
                        && mprotect((base + CODE) as *mut c_void, DATA, PROT_RW) == 0
                };
                if !ok {
                    return None;
                }
                // SAFETY: as above; small pages until a chunk is worth a huge one.
                unsafe { madvise(base as *mut c_void, CODE, MADV_NOHUGEPAGE) };
                Some(Mutex::new(Arena {
                    code: zone(base, CODE),
                    data: zone(base + CODE, DATA),
                    huge: !std::env::var("YAVS_LLVM_HUGE").is_ok_and(|v| v == "0"),
                }))
            })
            .as_ref()
    }

    /// `(code bytes, huge chunks, collapses given up, sections that did not fit)` so far, or
    /// `None` when the arena is not in use.
    pub fn stats() -> Option<[usize; 4]> {
        arena()?;
        Some([
            PLACED.load(Ordering::Relaxed),
            CHUNKS.load(Ordering::Relaxed),
            FAILED.load(Ordering::Relaxed),
            OVERFLOW.load(Ordering::Relaxed),
        ])
    }

    /// Whether the arena is in use (else LLJIT links as it always did).
    pub fn enabled() -> bool {
        arena().is_some()
    }

    /// A section that did not fit the arena: a mapping of its own.
    fn overflow(size: usize, prot: c_int) -> *mut u8 {
        OVERFLOW.fetch_add(1, Ordering::Relaxed);
        // The small code model reaches its data and stubs with 32-bit offsets: a section outside
        // the arena would link to wrong code, so this is a hard stop, not a fallback.
        if crate::jit::small_model() {
            eprintln!(
                "error: the code arena is full ({size} bytes more asked for); compiled code of this \
                 size needs YAVS_LLVM_CODEMODEL=large"
            );
            std::process::abort();
        }
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

    extern "C" fn create_context(_: *mut c_void) -> *mut c_void {
        // The arena is global: the per-layer context is only a non-null token.
        std::ptr::dangling_mut()
    }

    extern "C" fn notify_terminating(_: *mut c_void) {}

    extern "C" fn alloc_code(
        _: *mut c_void,
        size: usize,
        align: c_uint,
        _: c_uint,
        _: *const c_char,
    ) -> *mut u8 {
        let Some(a) = arena() else {
            return overflow(size, PROT_RWX);
        };
        let mut a = a.lock().unwrap_or_else(PoisonError::into_inner);
        let huge = a.huge;
        match a.code.alloc(size, align as usize, huge) {
            Some(p) => {
                PLACED.fetch_add(size, Ordering::Relaxed);
                p as *mut u8
            }
            None => overflow(size, PROT_RWX),
        }
    }

    extern "C" fn alloc_data(
        _: *mut c_void,
        size: usize,
        align: c_uint,
        _: c_uint,
        _: *const c_char,
        _: i32,
    ) -> *mut u8 {
        let Some(a) = arena() else {
            return overflow(size, PROT_RW);
        };
        let mut a = a.lock().unwrap_or_else(PoisonError::into_inner);
        match a.data.alloc(size, align as usize, false) {
            Some(p) => p as *mut u8,
            None => overflow(size, PROT_RW),
        }
    }

    // The code zone is writable and executable at once, so there is nothing to finalise (x86
    // keeps its instruction cache coherent). ponytail: memory of dropped JITs stays in the
    // arena (a run has a few JITs; a test process with hundreds uses megabytes); free chunks by
    // layer if that ever matters.
    extern "C" fn finalize(_: *mut c_void, _: *mut *mut c_char) -> i32 {
        0
    }

    extern "C" fn destroy(_: *mut c_void) {}

    /// The `ObjectLinkingLayerCreator` of the LLJIT builder.
    pub extern "C" fn create_layer(
        _: *mut c_void,
        es: LLVMOrcExecutionSessionRef,
        _: *const c_char,
    ) -> LLVMOrcObjectLayerRef {
        // SAFETY: the callbacks above live for the whole process; `es` is the session being built.
        unsafe {
            LLVMOrcCreateRTDyldObjectLinkingLayerWithMCJITMemoryManagerLikeCallbacks(
                es,
                std::ptr::null_mut(),
                create_context,
                notify_terminating,
                alloc_code,
                alloc_data,
                finalize,
                Some(destroy),
            )
        }
    }
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
mod imp {
    pub fn stats() -> Option<[usize; 4]> {
        None
    }

    #[allow(dead_code)]
    pub fn enabled() -> bool {
        false
    }
}

pub(crate) use imp::*;
