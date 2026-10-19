//! The compilation cache store (user decision 2026-10-19, `docs/decisions.md` "Compilation cache").
//!
//! This crate knows nothing about VHDL: it maps a [`Key`] (a hash of everything an entry depends
//! on) to a byte string in a directory, and never trusts what it reads back. An entry that is
//! missing, truncated, corrupt, from another format version or from another yavs build is a
//! miss; the caller then does the work from source. Every failure to write is silent (a
//! read-only library directory simply is not cached into), so the cache can only make a run
//! faster, never fail it.
//!
//! Writes are a temporary file plus an atomic rename, so concurrent readers and writers (parallel
//! test runs sharing a library) never see a half-written entry and need no lock.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use sha2::{Digest, Sha256};

/// Version of the entry file layout; part of every key and of the header.
pub const FORMAT: u32 = 1;

/// Name of the cache directory inside a library directory.
pub const DIR: &str = "yavs-cache";

/// Entries kept per directory; the least recently used ones are removed when it grows past this.
pub const MAX_ENTRIES: usize = 48;

const MAGIC: &[u8; 8] = b"YAVSIMG\0";
const HEADER: usize = 8 + 4 + 32 + 8 + 32;

/// Identifies the running build: the version, the executable's size and modification time. Any
/// rebuild changes it, so an entry never outlives the analyzer that made it.
pub fn build_id() -> &'static str {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| {
        let exe = std::env::current_exe()
            .and_then(fs::metadata)
            .map(|m| {
                let t = m
                    .modified()
                    .ok()
                    .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_nanos());
                format!("{}:{t}", m.len())
            })
            // ponytail: an unreadable executable gives a per-process id (never matches)
            .unwrap_or_else(|_| format!("unknown:{}", std::process::id()));
        format!("{}+{exe}", env!("CARGO_PKG_VERSION"))
    })
}

/// A hash of everything an entry depends on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Key([u8; 32]);

impl Key {
    pub fn hex(&self) -> String {
        self.0.iter().map(|b| format!("{b:02x}")).collect()
    }
}

/// Builds a [`Key`]; every part is length-prefixed so `("ab", "c")` and `("a", "bc")` differ.
pub struct KeyBuilder(Sha256);

impl KeyBuilder {
    /// `kind` names what is cached (`"sem"`); format and build are mixed in.
    pub fn new(kind: &str) -> Self {
        let mut k = KeyBuilder(Sha256::new());
        k.part(kind.as_bytes());
        k.part(&FORMAT.to_le_bytes());
        k.part(build_id().as_bytes());
        k
    }

    pub fn part(&mut self, bytes: &[u8]) -> &mut Self {
        self.0.update((bytes.len() as u64).to_le_bytes());
        self.0.update(bytes);
        self
    }

    pub fn text(&mut self, s: &str) -> &mut Self {
        self.part(s.as_bytes())
    }

    pub fn finish(self) -> Key {
        Key(self.0.finalize().into())
    }
}

/// A cache directory.
#[derive(Clone, Debug)]
pub struct Store {
    dir: PathBuf,
}

/// What [`Store::clean`] removed.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Cleaned {
    pub entries: usize,
    pub bytes: u64,
}

fn debug(msg: String) {
    if std::env::var("YAVS_LOG").is_ok_and(|v| v == "debug") {
        eprintln!("debug: cache: {msg}");
    }
}

impl Store {
    /// The store of library directory `lib`.
    pub fn of_library(lib: &Path) -> Store {
        Store { dir: lib.join(DIR) }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn file(&self, kind: &str, key: &Key) -> PathBuf {
        self.dir.join(format!("{kind}-{}.img", key.hex()))
    }

    /// The entry, or `None` for anything that is not a complete, intact entry for this key.
    pub fn get(&self, kind: &str, key: &Key) -> Option<Vec<u8>> {
        let path = self.file(kind, key);
        let bytes = fs::read(&path).ok()?;
        let payload = match check(&bytes, key) {
            Some(p) => p,
            None => {
                debug(format!("{} is not a valid entry", path.display()));
                return None;
            }
        };
        // Recently used entries survive pruning; failing to say so is harmless.
        if let Ok(f) = fs::File::options().write(true).open(&path) {
            let _ = f.set_modified(std::time::SystemTime::now());
        }
        Some(bytes[HEADER..HEADER + payload].to_vec())
    }

    /// Store `payload` atomically. Failures are only noted with `YAVS_LOG=debug`.
    pub fn put(&self, kind: &str, key: &Key, payload: &[u8]) {
        if let Err(e) = self.try_put(kind, key, payload) {
            debug(format!("cannot write into {}: {e}", self.dir.display()));
        }
    }

    fn try_put(&self, kind: &str, key: &Key, payload: &[u8]) -> std::io::Result<()> {
        fs::create_dir_all(&self.dir)?;
        let tmp = self.dir.join(format!(
            ".tmp-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        let write = || -> std::io::Result<()> {
            let mut f = fs::File::create(&tmp)?;
            f.write_all(MAGIC)?;
            f.write_all(&FORMAT.to_le_bytes())?;
            f.write_all(&key.0)?;
            f.write_all(&(payload.len() as u64).to_le_bytes())?;
            f.write_all(&Sha256::digest(payload))?;
            f.write_all(payload)?;
            f.flush()
        };
        if let Err(e) = write() {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
        // Replacing an entry another process just wrote is fine: same key, same content.
        if let Err(e) = fs::rename(&tmp, self.file(kind, key)) {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
        self.prune();
        Ok(())
    }

    /// Keep the [`MAX_ENTRIES`] most recently used entries.
    fn prune(&self) {
        let mut all = self.entries();
        if all.len() <= MAX_ENTRIES {
            return;
        }
        all.sort_by_key(|e| std::cmp::Reverse(e.1));
        for (p, _, _) in all.split_off(MAX_ENTRIES) {
            let _ = fs::remove_file(p);
        }
    }

    fn entries(&self) -> Vec<(PathBuf, std::time::SystemTime, u64)> {
        let Ok(rd) = fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        rd.filter_map(|e| {
            let e = e.ok()?;
            let name = e.file_name();
            let name = name.to_str()?;
            if !name.ends_with(".img") {
                return None;
            }
            let m = e.metadata().ok()?;
            Some((e.path(), m.modified().ok()?, m.len()))
        })
        .collect()
    }

    /// Remove every entry (and leftover temporary files) and the directory itself.
    pub fn clean(&self) -> std::io::Result<Cleaned> {
        let mut c = Cleaned::default();
        for (p, _, len) in self.entries() {
            fs::remove_file(p)?;
            c.entries += 1;
            c.bytes += len;
        }
        if let Ok(rd) = fs::read_dir(&self.dir) {
            for e in rd.flatten() {
                if e.file_name().to_string_lossy().starts_with(".tmp-") {
                    let _ = fs::remove_file(e.path());
                }
            }
        }
        match fs::remove_dir(&self.dir) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
            _ => Ok(c),
        }
    }
}

/// The payload length if `bytes` is a complete entry for `key` of this format.
fn check(bytes: &[u8], key: &Key) -> Option<usize> {
    if bytes.len() < HEADER || &bytes[..8] != MAGIC {
        return None;
    }
    if bytes[8..12] != FORMAT.to_le_bytes() || bytes[12..44] != key.0 {
        return None;
    }
    let len = u64::from_le_bytes(bytes[44..52].try_into().ok()?) as usize;
    if bytes.len() != HEADER.checked_add(len)? {
        return None;
    }
    (Sha256::digest(&bytes[HEADER..]).as_slice() == &bytes[52..84]).then_some(len)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(s: &str) -> Key {
        let mut k = KeyBuilder::new("t");
        k.text(s);
        k.finish()
    }

    #[test]
    fn round_trip_and_misses() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::of_library(d.path());
        assert_eq!(s.get("t", &key("a")), None);
        s.put("t", &key("a"), b"hello");
        assert_eq!(s.get("t", &key("a")).as_deref(), Some(&b"hello"[..]));
        assert_eq!(s.get("t", &key("b")), None);
        assert_eq!(s.get("u", &key("a")), None);
    }

    #[test]
    fn key_parts_do_not_run_together() {
        let mut a = KeyBuilder::new("t");
        a.text("ab").text("c");
        let mut b = KeyBuilder::new("t");
        b.text("a").text("bc");
        assert_ne!(a.finish(), b.finish());
    }

    #[test]
    fn corrupt_and_truncated_entries_are_misses() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::of_library(d.path());
        let k = key("a");
        s.put("t", &k, b"payload bytes");
        let p = s.file("t", &k);
        let good = fs::read(&p).unwrap();
        let mut bad = good.clone();
        *bad.last_mut().unwrap() ^= 1;
        fs::write(&p, &bad).unwrap();
        assert_eq!(s.get("t", &k), None);
        fs::write(&p, &good[..good.len() - 3]).unwrap();
        assert_eq!(s.get("t", &k), None);
        fs::write(&p, b"junk").unwrap();
        assert_eq!(s.get("t", &k), None);
        fs::write(&p, &good).unwrap();
        assert!(s.get("t", &k).is_some());
    }

    #[test]
    fn unwritable_directory_is_silent() {
        let d = tempfile::tempdir().unwrap();
        let f = d.path().join("file");
        fs::write(&f, b"x").unwrap();
        // the "library directory" is a file: nothing can be created below it
        let s = Store::of_library(&f);
        s.put("t", &key("a"), b"x");
        assert_eq!(s.get("t", &key("a")), None);
    }

    #[test]
    fn pruning_keeps_the_newest() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::of_library(d.path());
        for i in 0..MAX_ENTRIES + 5 {
            s.put("t", &key(&i.to_string()), b"x");
        }
        assert_eq!(s.entries().len(), MAX_ENTRIES);
        let c = s.clean().unwrap();
        assert_eq!(c.entries, MAX_ENTRIES);
        assert!(!s.dir().exists());
    }
}
