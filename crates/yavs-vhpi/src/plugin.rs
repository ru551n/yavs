//! Loading VHPI plugins (`--vhpi <lib>[:entry]`).

use std::fmt;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use yavs_diag::Diagnostic;

use crate::sys::vhpiRegistrationFctT;

/// A plugin library and an optional bootstrap function name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PluginSpec {
    pub path: PathBuf,
    pub entry: Option<String>,
}

impl FromStr for PluginSpec {
    type Err = std::convert::Infallible;

    /// Parses `lib[:entry]`. The text after the last `:` is an entry only
    /// when it is a C identifier, so `C:\x\lib.dll` stays a path.
    fn from_str(s: &str) -> Result<Self, Self::Err> {
        let is_ident = |e: &str| {
            e.chars()
                .next()
                .is_some_and(|c| c == '_' || c.is_ascii_alphabetic())
                && e.chars().all(|c| c == '_' || c.is_ascii_alphanumeric())
        };
        Ok(match s.rsplit_once(':') {
            Some((path, entry)) if !path.is_empty() && is_ident(entry) => PluginSpec {
                path: path.into(),
                entry: Some(entry.into()),
            },
            _ => PluginSpec {
                path: s.into(),
                entry: None,
            },
        })
    }
}

/// A plugin that cannot be used. Loading stops at the first one, before
/// the simulation starts.
#[derive(Debug)]
pub enum LoadError {
    /// A path with a directory part names no file.
    NotFound { path: PathBuf, tried: PathBuf },
    /// The OS loader refused the library (bad format, missing dependency,
    /// unresolved `vhpi_*` import).
    Open {
        path: PathBuf,
        tried: String,
        os: String,
    },
    /// Neither the named entry nor `vhpi_startup_routines` was found.
    NoEntry { path: PathBuf, wanted: String },
}

impl LoadError {
    /// The yavs-diag form: E0901 (cannot load), E0902 (no entry point).
    pub fn to_diagnostic(&self) -> Diagnostic {
        match self {
            LoadError::NotFound { path, tried } => Diagnostic::error(
                "E0901",
                format!("cannot load VHPI plugin `{}`: no such file", path.display()),
            )
            .with_note(format!("looked for {}", tried.display())),
            LoadError::Open { path, tried, os } => Diagnostic::error(
                "E0901",
                format!("cannot load VHPI plugin `{}`", path.display()),
            )
            .with_note(format!("tried {tried}"))
            .with_note(format!("the OS loader said: {os}"))
            .with_help(
                "check that the plugin matches yavs's architecture, that its own dependencies \
                 are on the library search path, and that every vhpi_* symbol it imports is \
                 one yavs exports",
            ),
            LoadError::NoEntry { path, wanted } => Diagnostic::error(
                "E0902",
                format!("VHPI plugin `{}` has no {wanted}", path.display()),
            )
            .with_help(
                "export a NULL-terminated `vhpi_startup_routines` array, or name a bootstrap \
                 function with --vhpi LIB:ENTRY",
            ),
        }
    }
}

impl fmt::Display for LoadError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_diagnostic().message)
    }
}

impl std::error::Error for LoadError {}

/// Loads a plugin and runs its registration: the named entry if given,
/// otherwise every routine in its NULL-terminated `vhpi_startup_routines`.
/// The library stays loaded for the life of the process, since the
/// callbacks it registered point into it.
pub fn load(spec: &PluginSpec) -> Result<(), LoadError> {
    let path = &spec.path;
    let lib = open_path(path)?;
    match &spec.entry {
        Some(entry) => {
            // SAFETY: by convention a bootstrap entry is `void (*)(void)`.
            let f =
                unsafe { lib.get::<unsafe extern "C" fn()>(entry.as_bytes()) }.map_err(|_| {
                    LoadError::NoEntry {
                        path: path.clone(),
                        wanted: format!("function `{entry}`"),
                    }
                })?;
            // SAFETY: as above; the plugin runs its own registration code.
            unsafe { f() };
        }
        None => {
            // SAFETY: `vhpi_startup_routines` is a NULL-terminated array of
            // `void (*)(void)`; the symbol's address is its first element.
            let table = unsafe { lib.get::<*const vhpiRegistrationFctT>(b"vhpi_startup_routines") }
                .map_err(|_| LoadError::NoEntry {
                    path: path.clone(),
                    wanted: "`vhpi_startup_routines`".into(),
                })?;
            let mut p = *table;
            // SAFETY: walks the array up to its NULL terminator.
            unsafe {
                while let Some(routine) = *p {
                    routine();
                    p = p.add(1);
                }
            }
        }
    }
    std::mem::forget(lib);
    Ok(())
}

/// Opens `path`: a bare name through the OS library search path, anything else must exist.
fn open_path(path: &Path) -> Result<libloading::Library, LoadError> {
    let path = &path.to_path_buf();
    let tried = if path.components().count() > 1 || path.exists() {
        let abs = std::path::absolute(path).unwrap_or_else(|_| path.clone());
        if !abs.exists() {
            return Err(LoadError::NotFound {
                path: path.clone(),
                tried: abs,
            });
        }
        abs
    } else {
        path.clone()
    };
    open(&tried).map_err(|e| LoadError::Open {
        path: path.clone(),
        tried: if tried == *path {
            format!("`{}` on the OS library search path", path.display())
        } else {
            format!("`{}`", tried.display())
        },
        os: error_chain(&e),
    })
}

/// A shared library loaded with `--load` for VHPIDIRECT symbols. It stays loaded for the
/// life of the process, since bound functions point into it.
pub struct ForeignLib {
    pub path: PathBuf,
    lib: &'static libloading::Library,
}

/// Loads a `--load` library (no VHPI registration runs).
pub fn load_foreign(path: &Path) -> Result<ForeignLib, LoadError> {
    let lib = Box::leak(Box::new(open_path(path)?));
    Ok(ForeignLib {
        path: path.to_path_buf(),
        lib,
    })
}

/// Binds `symbol` with C signature `sig`: the first `--load` library in `libs` that defines
/// it, else the executable. `None` if nothing defines it.
pub fn bind_foreign(
    libs: &[ForeignLib],
    symbol: &str,
    sig: yavs_ffi::Signature,
) -> Option<Result<yavs_ffi::ForeignFn, yavs_ffi::FfiError>> {
    // SAFETY: the address is only used as a function pointer by the trampoline, with the
    // signature the VHDL declaration gives it (the VHPIDIRECT contract).
    let find = |l: &libloading::Library| unsafe {
        l.get::<*mut std::ffi::c_void>(symbol.as_bytes())
            .ok()
            .map(|p| *p)
    };
    let ptr = libs
        .iter()
        .find_map(|l| find(l.lib))
        .or_else(|| this().ok().and_then(|l| find(&l)))?;
    // SAFETY: as above; the libraries are never unloaded.
    Some(unsafe { yavs_ffi::ForeignFn::new(ptr, sig) })
}

#[cfg(unix)]
fn this() -> Result<libloading::Library, libloading::Error> {
    Ok(libloading::os::unix::Library::this().into())
}

#[cfg(windows)]
fn this() -> Result<libloading::Library, libloading::Error> {
    libloading::os::windows::Library::this().map(Into::into)
}

/// The error and its sources, so the OS error code is not lost (on
/// Windows libloading's own message is only "LoadLibraryExW failed").
fn error_chain(e: &dyn std::error::Error) -> String {
    let mut s = e.to_string();
    let mut cur = e.source();
    while let Some(src) = cur {
        s.push_str(": ");
        s.push_str(&src.to_string());
        cur = src.source();
    }
    s
}

#[cfg(unix)]
fn open(path: &Path) -> Result<libloading::Library, libloading::Error> {
    use libloading::os::unix::{Library, RTLD_GLOBAL, RTLD_NOW};
    // RTLD_NOW: a vhpi_* symbol yavs does not export fails here, not at
    // the first call. RTLD_GLOBAL: libraries the plugin loads later (for
    // cocotb, the GPI implementation) can see its symbols.
    // SAFETY: loading runs the library's initializers; that is the point.
    unsafe { Library::open(Some(path), RTLD_NOW | RTLD_GLOBAL) }.map(Into::into)
}

#[cfg(windows)]
fn open(path: &Path) -> Result<libloading::Library, libloading::Error> {
    // SAFETY: loading runs DllMain; that is the point.
    unsafe { libloading::Library::new(path) }
}

/// Give the caller a private duplicate of standard output and point file descriptor 1 (the C
/// runtime's `stdout`, which plugins and `vhpi_printf` write to) at standard error, so a
/// plugin's prints cannot corrupt a protocol stream on standard output (#389).
pub fn divert_stdout() -> std::io::Result<std::fs::File> {
    use std::io::Write;
    let mut out = std::io::stdout();
    out.flush()?;
    #[cfg(unix)]
    let own = {
        use std::os::fd::AsFd;
        unsafe extern "C" {
            fn dup2(old: i32, new: i32) -> i32;
        }
        let own = out.as_fd().try_clone_to_owned()?;
        // SAFETY: both descriptors are open; dup2 replaces descriptor 1 atomically.
        if unsafe { dup2(2, 1) } < 0 {
            return Err(std::io::Error::last_os_error());
        }
        own
    };
    #[cfg(windows)]
    let own = {
        use std::os::windows::io::{AsHandle, AsRawHandle};
        unsafe extern "C" {
            fn _dup2(old: i32, new: i32) -> i32;
        }
        unsafe extern "system" {
            fn SetStdHandle(which: u32, handle: *mut std::ffi::c_void) -> i32;
        }
        let own = out.as_handle().try_clone_to_owned()?;
        // SAFETY: both descriptors are open; the UCRT re-points descriptor 1, and the Win32
        // standard output handle (what Rust's `stdout` uses) follows explicitly.
        if unsafe { _dup2(2, 1) } < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let err = std::io::stderr().as_raw_handle();
        if unsafe { SetStdHandle(-11i32 as u32, err.cast()) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        own
    };
    Ok(own.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(s: &str) -> (String, Option<String>) {
        let p = s.parse::<PluginSpec>().unwrap();
        (p.path.to_string_lossy().into_owned(), p.entry)
    }

    #[test]
    fn parses_lib_and_entry() {
        assert_eq!(spec("lib.so"), ("lib.so".into(), None));
        assert_eq!(
            spec("/a/lib.so:boot_1"),
            ("/a/lib.so".into(), Some("boot_1".into()))
        );
        assert_eq!(spec(r"C:\x\lib.dll"), (r"C:\x\lib.dll".into(), None));
        assert_eq!(
            spec(r"C:\x\lib.dll:entry"),
            (r"C:\x\lib.dll".into(), Some("entry".into()))
        );
        assert_eq!(spec(":entry"), (":entry".into(), None));
        assert_eq!(spec("lib.so:"), ("lib.so:".into(), None));
    }
}
