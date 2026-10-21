//! Loads the shared LLVM library on first use (see `build.rs`).
//!
//! The executable holds a stub per exported LLVM C API function, all jumping through
//! `yavs_llvm_slots`. [`ensure`] opens the library and fills the table, once. Every way into
//! LLVM starts with it (`Module::new`, `Jit::new`, `compile`, `AutoTier::start`); a stub reached
//! earlier would be a bug in this crate and ends the process with a message.

use crate::Error;

#[cfg(lazy_llvm)]
mod stubs {
    include!(concat!(env!("OUT_DIR"), "/stubs.rs"));

    unsafe extern "C" {
        #[allow(improper_ctypes)]
        pub(super) static mut yavs_llvm_slots: [usize; COUNT];
    }

    #[unsafe(no_mangle)]
    extern "C" fn yavs_llvm_unloaded() -> ! {
        eprintln!(
            "internal error: an LLVM function was called before the library was loaded (a bug in yavs)"
        );
        std::process::abort()
    }

    pub(super) fn load() -> Result<(), String> {
        // SAFETY: opening the library runs its initialisers, as the dynamic loader would have.
        let lib = unsafe { libloading::Library::new(SONAME) }
            .or_else(|_| unsafe { libloading::Library::new(PATH) })
            .map_err(|e| format!("cannot load the LLVM library {SONAME}: {e}"))?;
        let mut slots = [0usize; COUNT];
        for (slot, name) in slots.iter_mut().zip(NAMES) {
            // SAFETY: only the address is taken; the stubs call it with the C API's signature.
            let f: libloading::Symbol<'_, unsafe extern "C" fn()> =
                unsafe { lib.get(name.as_bytes()) }.map_err(|e| {
                    format!(
                        "the LLVM library {SONAME} lacks {}: {e}",
                        name.trim_end_matches('\0')
                    )
                })?;
            *slot = *f as usize;
        }
        // The library stays loaded for the life of the process.
        std::mem::forget(lib);
        // SAFETY: nothing calls through the table until `ensure` returns (it is the only caller
        // of `load`, once, and the callers of LLVM wait for it).
        unsafe { std::ptr::addr_of_mut!(yavs_llvm_slots).write(slots) };
        Ok(())
    }
}

/// Makes the LLVM library available. Cheap after the first call.
#[cfg(lazy_llvm)]
pub fn ensure() -> Result<(), Error> {
    use std::sync::OnceLock;
    static LOADED: OnceLock<Result<(), String>> = OnceLock::new();
    LOADED.get_or_init(stubs::load).clone().map_err(Error)
}

/// Makes the LLVM library available (linked normally on this target).
#[cfg(not(lazy_llvm))]
pub fn ensure() -> Result<(), Error> {
    Ok(())
}
