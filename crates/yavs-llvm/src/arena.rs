//! One contiguous arena for the machine code of every JIT of the process (#679).
//!
//! ORC's default linker maps the code of each module where the kernel puts it, in 4 KiB pages: a
//! large compiled process (hundreds of KiB to megabytes of straight-line code that every
//! activation runs through) then costs an instruction-TLB entry per page. Here the object layer
//! is RuntimeDyld with our own section allocator: code sections are bump-allocated one after the
//! other in a 2 MiB-aligned region (hot code dense in memory, in the order it was compiled), and
//! a 2 MiB chunk of it is asked to be a transparent huge page (`madvise`, then `MADV_COLLAPSE` of
//! what is there) once it holds 256 KiB, so a small run keeps its small pages. Data sections go
//! to a second zone of the same reservation (code and data stay within 2 GiB of each other), both
//! in `hugezone.rs`, shared with the Cranelift tier.
//! Linux x86_64 only (the instruction cache is coherent there); everywhere else, and when the
//! reservation fails or `YAVS_LLVM_ARENA=0`, LLJIT keeps its default linker.
//! The arena is also what lets the code be generated for the small code model (`jit.rs` `model`).
//! `YAVS_LLVM_HUGE=0` keeps the arena but leaves the huge pages out.

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod imp {
    use std::ffi::{c_char, c_int, c_uint, c_void};
    use std::sync::atomic::{AtomicUsize, Ordering};

    use crate::hugezone::{self, CHUNKS, FAILED, PROT_RW, PROT_RWX, Zone};
    use std::sync::{Mutex, OnceLock, PoisonError};

    use llvm_sys::orc2::ee::*;
    use llvm_sys::orc2::*;

    /// Code bytes placed, sections that did not fit the arena (all JITs of this process).
    static PLACED: AtomicUsize = AtomicUsize::new(0);
    static OVERFLOW: AtomicUsize = AtomicUsize::new(0);

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
                let z = hugezone::reserve()?;
                Some(Mutex::new(Arena {
                    code: z.code,
                    data: z.data,
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
        hugezone::map(size, prot)
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
