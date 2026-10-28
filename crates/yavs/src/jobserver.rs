//! The token pool of `MAKEFLAGS` (`--jobserver-auth=R,W` or `fifo:PATH`): the threads of
//! `--threads` above the first run only on tokens taken from it (`docs/parallel.md`).

use std::sync::{Arc, Mutex, OnceLock, PoisonError};

/// The pool of this run, for the exit paths that must give its tokens back.
static POOL: OnceLock<Arc<Pool>> = OnceLock::new();

/// Tokens taken (the bytes read, written back as they were); `None` once closed.
pub(crate) struct Pool {
    state: Mutex<Option<(std::fs::File, Vec<u8>)>>,
}

impl yavs_kernel::Tokens for Pool {
    fn take(&self, want: usize) -> usize {
        use std::io::Read;
        let mut st = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        let Some((f, held)) = st.as_mut() else {
            return 0;
        };
        if held.len() < want {
            let mut buf = [0u8; 64];
            let n = (want - held.len()).min(buf.len());
            // Non-blocking: an empty pool (or any read error) is no token now, never a wait.
            if let Ok(got) = f.read(&mut buf[..n]) {
                held.extend_from_slice(&buf[..got]);
            }
        }
        held.len()
    }

    fn give(&self) {
        let mut st = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((f, held)) = st.as_mut() {
            give_back(f, held);
        }
    }
}

fn give_back(f: &mut std::fs::File, held: &mut Vec<u8>) {
    use std::io::Write;
    // A write that fails loses the tokens, as a killed process would.
    let _ = f.write_all(held);
    held.clear();
}

/// Give every token back and take none again: before any exit while the kernel may hold some.
pub(crate) fn close() {
    if let Some(p) = POOL.get() {
        let mut st = p.state.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((mut f, mut held)) = st.take() {
            give_back(&mut f, &mut held);
        }
    }
}

/// Whether a pool is in use (the signal handlers then give its tokens back before exiting).
pub(crate) fn active() -> bool {
    POOL.get().is_some()
}

/// The pool named in `MAKEFLAGS`, if any; `Err` is the note of error E0128. Call before the run opens any file: a descriptor
/// the parent closed may be reused by one.
pub(crate) fn connect() -> Result<Option<Arc<Pool>>, String> {
    let Some(flags) = std::env::var_os("MAKEFLAGS") else {
        return Ok(None);
    };
    let flags = flags.to_string_lossy();
    // The last one wins, as for make itself; `--jobserver-fds` is the older spelling.
    let Some(auth) = flags.split_whitespace().rev().find_map(|w| {
        w.strip_prefix("--jobserver-auth=")
            .or_else(|| w.strip_prefix("--jobserver-fds="))
    }) else {
        return Ok(None);
    };
    let f =
        open(auth).map_err(|why| format!("`MAKEFLAGS` names `--jobserver-auth={auth}`: {why}"))?;
    let p = Arc::new(Pool {
        state: Mutex::new(Some((f, Vec::new()))),
    });
    let _ = POOL.set(p.clone());
    Ok(Some(p))
}

#[cfg(unix)]
fn open(auth: &str) -> Result<std::fs::File, String> {
    use std::os::unix::fs::{FileTypeExt, OpenOptionsExt};
    let path = match auth.strip_prefix("fifo:") {
        Some(p) => p.to_string(),
        None => {
            let fd = auth
                .split_once(',')
                .and_then(|(r, w)| Some((r.parse::<u32>().ok()?, w.parse::<u32>().ok()?)))
                .ok_or("not `R,W` descriptors or `fifo:PATH`")?
                .0;
            // A new open file description of the pipe, so non-blocking reads leave the
            // parent's descriptor as it was. Elsewhere `/dev/fd` shares it: `fifo:PATH` only.
            if !cfg!(target_os = "linux") {
                return Err(
                    "descriptors are supported on Linux only, `fifo:PATH` elsewhere".into(),
                );
            }
            format!("/proc/self/fd/{fd}")
        }
    };
    // A closed descriptor, or one the parent did not mean for us, is not a pipe.
    match std::fs::metadata(&path) {
        Ok(m) if m.file_type().is_fifo() => {}
        Ok(_) => return Err(format!("`{path}` is not a pipe")),
        Err(e) => return Err(format!("`{path}`: {e}")),
    }
    std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(&path)
        .map_err(|e| format!("`{path}`: {e}"))
}

// ponytail: no Windows semaphore client yet; add one when a test runner passes one there.
#[cfg(not(unix))]
fn open(_: &str) -> Result<std::fs::File, String> {
    Err("a token pool is supported on Linux and other Unix systems only".into())
}
