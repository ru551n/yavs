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

    /// Where the LLVM library is looked for: `lib/` next to the executable (the release archives),
    /// the dynamic loader's search, the path this build linked against.
    fn candidates(exe: Option<&std::path::Path>) -> Vec<std::path::PathBuf> {
        let mut c = Vec::new();
        if let Some(dir) = exe.and_then(|e| e.parent()) {
            c.push(dir.join("lib").join(SONAME));
        }
        c.extend([SONAME.into(), PATH.into()]);
        c
    }

    pub(super) fn load() -> Result<(), String> {
        let mut failed = Vec::new();
        // SAFETY: opening the library runs its initialisers, as the dynamic loader would have.
        let lib = candidates(std::env::current_exe().ok().as_deref())
            .iter()
            .find_map(|p| match unsafe { libloading::Library::new(p) } {
                Ok(l) => Some(l),
                Err(e) => {
                    let why = if p.is_absolute() && !p.exists() {
                        "not found".into()
                    } else {
                        e.to_string()
                    };
                    failed.push(format!("{}: {why}", p.display()));
                    None
                }
            })
            .ok_or_else(|| {
                format!(
                    "cannot load the LLVM library {SONAME} (looked in {})",
                    failed.join("; ")
                )
            })?;
        let mut slots = [0usize; COUNT];
        for (slot, name) in slots.iter_mut().zip(NAMES.split_inclusive('\0')) {
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

    #[cfg(test)]
    mod tests {
        use super::*;
        use std::path::Path;

        #[test]
        fn the_bundled_copy_is_tried_first() {
            let c = candidates(Some(Path::new("/opt/yavs/bin/yavs")));
            assert_eq!(c.len(), 3);
            assert_eq!(c[0], Path::new("/opt/yavs/bin/lib").join(SONAME));
            assert_eq!(c[1], Path::new(SONAME));
            assert_eq!(candidates(None).len(), 2);
        }
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
