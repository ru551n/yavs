//! The perf map (`/tmp/perf-<pid>.map`, one `address size name` line per function of generated
//! code) the compiled tier writes for `perf report` (`YAVS_CLIFT_PERFMAP`). One append-only
//! writer, so the code of several compiles shares the file. The file is named by the pid alone and
//! pids are reused, so [`start`] empties it once per process before anything is added (#1135).

use std::io::Write;
use std::sync::Mutex;

/// The symbol of generated code: `f12_m.u1.worker` (`kind` `f` frame-mode code, `n` native-frame
/// code, `c` its `CallFn` adapter, then the function id and its `Program::label`). The id keeps
/// it unique; the label is cleaned for the map, which is one line of text per function.
pub fn symbol(kind: &str, id: u32, label: &str) -> String {
    let clean: String = label
        .chars()
        .filter(|&c| c != '"')
        .map(|c| {
            if c.is_whitespace() || c.is_control() {
                '_'
            } else {
                c
            }
        })
        .collect();
    format!("{kind}{id}_{clean}")
}

/// The map path of this process.
fn path() -> String {
    format!("/tmp/perf-{}.map", std::process::id())
}

/// Empty the map of this process, once: a file left by an earlier process with the same pid (a
/// run that did not remove it, `perf record`) would otherwise name its code next to ours. Call it
/// before any function is listed, also when the JIT module lists them itself.
pub fn start() {
    static START: std::sync::Once = std::sync::Once::new();
    START.call_once(|| {
        if let Err(e) = empty(&path()) {
            eprintln!("warning: cannot empty the perf map {}: {e}", path());
        }
    });
}

/// Create `path` or cut it to zero length.
fn empty(path: &str) -> std::io::Result<()> {
    std::fs::File::create(path).map(drop)
}

/// Add the line of a function whose code is `size` bytes at `addr`.
pub fn append(addr: usize, size: usize, name: &str) {
    static FILE: Mutex<Option<std::fs::File>> = Mutex::new(None);
    let mut f = FILE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if f.is_none() {
        start();
        let path = path();
        match std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
        {
            Ok(file) => *f = Some(file),
            Err(e) => {
                static WARNED: std::sync::Once = std::sync::Once::new();
                WARNED.call_once(|| eprintln!("warning: cannot write the perf map {path}: {e}"));
            }
        }
    }
    if let Some(f) = f.as_mut() {
        let _ = writeln!(f, "{addr:x} {size:x} {name}");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn empty_cuts_a_stale_map_and_creates_a_missing_one() {
        let p = std::env::temp_dir().join(format!("perfmap_{}_empty.map", std::process::id()));
        let p = p.to_str().expect("utf-8 path");
        std::fs::write(p, "1000 10 stale\n").expect("write");
        super::empty(p).expect("empty");
        assert_eq!(std::fs::read_to_string(p).expect("read"), "");
        std::fs::remove_file(p).expect("remove");
        super::empty(p).expect("create");
        assert!(std::fs::read_to_string(p).expect("read").is_empty());
        std::fs::remove_file(p).expect("remove");
    }
}
