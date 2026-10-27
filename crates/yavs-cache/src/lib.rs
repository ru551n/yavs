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
//! test runs sharing a library) never see a half-written entry and need no lock. The analysed
//! units (`Store::units_of_library`) are created under their own name instead, to save the rename:
//! a reader that meets one half written finds its length or checksum wrong, which is a miss.

#[allow(unsafe_code)]
mod map;

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use sha2::{Digest, Sha256};

/// Version of the entry file layout; part of every key and of the header.
pub const FORMAT: u32 = 2;

/// Name of the cache directory inside a library directory.
pub const DIR: &str = "yavs-cache";

/// Entries kept per directory; the least recently used ones are removed when it grows past this.
pub const MAX_ENTRIES: usize = 96;

/// Bytes of entries kept per directory (the lowered library of a big verification library is
/// about 10 MB): the least recently used entries go first, but the newest is always kept.
pub const MAX_BYTES: u64 = 512 << 20;

/// The subdirectory of [`DIR`] that holds the compiled code of the LLVM tier
/// ([`Store::objects_of_library`]): many small entries, one per compiled module, under limits of
/// their own, so they never push the stage entries out.
pub const OBJ_DIR: &str = "obj";

/// Entries and bytes kept in [`OBJ_DIR`].
pub const OBJ_MAX_ENTRIES: usize = 4096;
pub const OBJ_MAX_BYTES: u64 = 256 << 20;

/// The subdirectory of [`DIR`] that holds the analysed-unit cache ([`Store::units_of_library`]):
/// one entry per design unit of the library (`yavs_sem::units`), under limits of their own.
pub const UNIT_DIR: &str = "units";

/// Entries and bytes kept in [`UNIT_DIR`]: room for every unit of a big verification library and
/// the std/ieee units it uses.
pub const UNIT_MAX_ENTRIES: usize = 16384;
pub const UNIT_MAX_BYTES: u64 = 512 << 20;

/// Notes ([`Store::note`]) kept per directory: the newest ones. Trimming asks the age of every
/// note (a system call each), so it waits until the directory is [`NOTES_SLACK`] past this.
pub const MAX_NOTES: usize = 512;
const NOTES_SLACK: usize = 64;

/// Whether a put of `added` bytes into a directory of `n` entries kept to `max_entries` and
/// `max_bytes` must look at them all (their size and age: 0.25 ms for 90 entries); `due` is that
/// the last time was `SWEEP_EVERY` ago.
fn needs_sweep(n: usize, added: u64, due: bool, (max_entries, max_bytes): (usize, u64)) -> bool {
    n > max_entries || added > max_bytes / FEW_ENTRIES as u64 || (n > FEW_ENTRIES && due)
}

/// The file whose modification time says when a directory was last pruned by size, and how
/// often that is done for a directory that is not over the entry limit: the byte limit is soft
/// by what is stored in between.
const SWEPT: &str = ".swept";
const SWEEP_EVERY: std::time::Duration = std::time::Duration::from_secs(5);

/// A directory of at most this many entries, each at most a sixteenth of its byte limit, cannot be
/// over that limit: a put into it needs no pruning.
const FEW_ENTRIES: usize = 16;

const MAGIC: &[u8; 8] = b"YAVSIMG\0";
const HEADER: usize = 8 + 4 + 32 + 8 + 8;

static SOURCES: OnceLock<String> = OnceLock::new();

/// Names the sources the running executable was built from (a hash made at build time, see
/// `crates/yavs/build.rs`). Call it once, before the first key is made; without it the build is
/// identified by the executable's modification time.
pub fn set_sources(hash: &str) {
    if !hash.is_empty() {
        let _ = SOURCES.set(hash.to_string());
    }
}

/// Identifies the running build: the version, the sources it was built from (or, without them,
/// the executable's modification time), the executable's size, and `YAVS_CACHE_SALT` (a way to
/// stand in for another build; tests). A copy of a build has the same id, another build never
/// does, so an entry never outlives the code that made it.
pub fn build_id() -> &'static str {
    static ID: OnceLock<String> = OnceLock::new();
    ID.get_or_init(|| {
        let exe = std::env::current_exe()
            .and_then(fs::metadata)
            .map(|m| {
                let t = match SOURCES.get() {
                    Some(h) => h.clone(),
                    None => m
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                        .map_or(0, |d| d.as_nanos())
                        .to_string(),
                };
                format!("{}:{t}", m.len())
            })
            // ponytail: an unreadable executable gives a per-process id (never matches)
            .unwrap_or_else(|_| format!("unknown:{}", std::process::id()));
        let salt = std::env::var("YAVS_CACHE_SALT").unwrap_or_default();
        format!("{}+{exe}+{salt}", env!("CARGO_PKG_VERSION"))
    })
}

/// The payload of an entry that was read: the file mapped into memory (or, where that is not
/// possible, read), not copied. It can be kept for as long as the values decoded from it need it.
#[derive(Debug)]
pub struct Entry {
    data: map::Data,
}

impl std::ops::Deref for Entry {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        &self.data[HEADER..]
    }
}

impl AsRef<[u8]> for Entry {
    fn as_ref(&self) -> &[u8] {
        self
    }
}

impl PartialEq for Entry {
    fn eq(&self, o: &Entry) -> bool {
        **self == **o
    }
}

/// The checksum of a payload: fast enough not to show next to reading the file (a damaged entry
/// is a miss; nothing here defends against an attacker, who could edit the sources as well).
fn checksum(payload: &[u8]) -> [u8; 8] {
    twox_hash::XxHash3_64::oneshot(payload).to_le_bytes()
}

/// Return memory the allocator keeps after a big, short-lived computation (the lowering of a
/// whole library) to the system, so that it does not stay in the process's resident size
/// through the rest of the run.
pub fn trim_heap() {
    map::trim_heap();
}

/// Ask the system for transparent huge pages on the heap (Linux, glibc, THP in `madvise` mode;
/// nothing elsewhere). Call once, first thing in `main`. `YAVS_HUGE_HEAP=0` switches it off.
pub fn huge_heap() {
    map::huge_heap();
}

/// A hash of everything an entry depends on.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Key([u8; 32]);

impl Key {
    pub fn hex(&self) -> String {
        String::from_utf8_lossy(&self.hex_bytes()).into_owned()
    }

    /// The key whose [`Key::hex`] is `s`, `None` for any other text.
    pub fn from_hex(s: &str) -> Option<Key> {
        let b = s.as_bytes();
        if b.len() != 64 || !b.iter().all(u8::is_ascii_hexdigit) {
            return None;
        }
        let mut k = [0; 32];
        for (i, out) in k.iter_mut().enumerate() {
            *out = u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()?;
        }
        Some(Key(k))
    }

    fn hex_bytes(&self) -> [u8; 64] {
        const DIGITS: &[u8; 16] = b"0123456789abcdef";
        let mut out = [0; 64];
        for (i, b) in self.0.iter().enumerate() {
            out[2 * i] = DIGITS[usize::from(b >> 4)];
            out[2 * i + 1] = DIGITS[usize::from(b & 15)];
        }
        out
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
    /// Entries and bytes kept ([`MAX_ENTRIES`] and [`MAX_BYTES`], or the `OBJ_*` limits).
    limits: (usize, u64),
    /// `get` refreshes the time of the entry it reads (not for the analysed units: a run reads
    /// a hundred of them, and a library keeps them all).
    touch: bool,
    /// `put` creates an entry under its own name (the analysed units: a hundred or more small
    /// entries per call), not through a temporary file and a rename.
    direct: bool,
    /// The `.img` entries of the directory as this process counts them: read once, then one per
    /// put (a batch of puts must not list the directory every time); [`UNKNOWN`] until the first
    /// put and after a sweep. Entries other processes add meanwhile are seen by the next sweep.
    count: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

/// [`Store::count`] before the directory was listed.
const UNKNOWN: usize = usize::MAX;

fn unknown() -> std::sync::Arc<std::sync::atomic::AtomicUsize> {
    std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(UNKNOWN))
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
        Store {
            dir: lib.join(DIR),
            limits: (MAX_ENTRIES, MAX_BYTES),
            touch: true,
            direct: false,
            count: unknown(),
        }
    }

    /// The store of the compiled code of library directory `lib` ([`OBJ_DIR`]); [`Store::clean`]
    /// of the library's store removes it as well.
    pub fn objects_of_library(lib: &Path) -> Store {
        Store {
            dir: lib.join(DIR).join(OBJ_DIR),
            limits: (OBJ_MAX_ENTRIES, OBJ_MAX_BYTES),
            touch: true,
            direct: false,
            count: unknown(),
        }
    }

    /// The store of the analysed units of library directory `lib` ([`UNIT_DIR`]);
    /// [`Store::clean`] of the library's store removes it as well.
    pub fn units_of_library(lib: &Path) -> Store {
        Store {
            dir: lib.join(DIR).join(UNIT_DIR),
            limits: (UNIT_MAX_ENTRIES, UNIT_MAX_BYTES),
            touch: false,
            direct: true,
            count: unknown(),
        }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// `dir/kind-<key>.img` in one allocation (a run asks for a hundred entries).
    fn file(&self, kind: &str, key: &Key) -> PathBuf {
        let name = |p: &mut std::ffi::OsString| {
            p.push(kind);
            p.push("-");
            p.push(std::str::from_utf8(&key.hex_bytes()).expect("hex digits"));
            p.push(".img");
        };
        let dir = self.dir.as_os_str();
        if dir.is_empty()
            || self
                .dir
                .to_str()
                .is_some_and(|d| d.ends_with(std::path::is_separator))
        {
            let mut n = std::ffi::OsString::new();
            name(&mut n);
            return self.dir.join(n);
        }
        let mut p = std::ffi::OsString::with_capacity(dir.len() + kind.len() + 72);
        p.push(dir);
        p.push(std::path::MAIN_SEPARATOR_STR);
        name(&mut p);
        PathBuf::from(p)
    }

    /// The entry, or `None` for anything that is not a complete, intact entry for this key.
    pub fn get(&self, kind: &str, key: &Key) -> Option<Entry> {
        let path = self.file(kind, key);
        let bytes = map::open(&path, self.touch)?;
        if !check(&bytes, key) {
            debug(format!("{} is not a valid entry", path.display()));
            return None;
        }
        // Recently used entries survive pruning; failing to say so is harmless. (Unix: `open`
        // did it, through the file it had open.)
        #[cfg(not(unix))]
        if self.touch
            && let Ok(f) = fs::File::options().write(true).open(&path)
        {
            let _ = f.set_modified(std::time::SystemTime::now());
        }
        Some(Entry { data: bytes })
    }

    /// The payload size of the entry, from its header and the file's size alone (the payload is not
    /// read or checked): for a caller that only asks whether a usable, non-empty entry is there. A
    /// file whose size is not the header's is not one (an entry another process is still writing,
    /// or a write that was cut short).
    pub fn payload_len(&self, kind: &str, key: &Key) -> Option<u64> {
        use std::io::Read as _;
        let mut f = fs::File::open(self.file(kind, key)).ok()?;
        let size = f.metadata().ok()?.len();
        let mut head = [0u8; HEADER];
        f.read_exact(&mut head).ok()?;
        let len = u64::from_le_bytes(head[HEADER - 16..HEADER - 8].try_into().ok()?);
        (size == HEADER as u64 + len).then_some(len)
    }

    /// Store `payload` atomically. Failures are only noted with `YAVS_LOG=debug`.
    pub fn put(&self, kind: &str, key: &Key, payload: &[u8]) {
        if let Err(e) = self.try_put(kind, key, payload) {
            debug(format!("cannot write into {}: {e}", self.dir.display()));
        }
    }

    fn try_put(&self, kind: &str, key: &Key, payload: &[u8]) -> std::io::Result<()> {
        if self.direct && self.try_put_new(kind, key, payload)? {
            self.prune(payload.len() as u64);
            return Ok(());
        }
        let tmp = self.dir.join(format!(
            ".tmp-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(0, |d| d.as_nanos())
        ));
        let write = || -> std::io::Result<()> {
            // The directory exists after the first put: made only when the create fails.
            let mut f = match fs::File::create(&tmp) {
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    fs::create_dir_all(&self.dir)?;
                    fs::File::create(&tmp)?
                }
                r => r?,
            };
            let mut head = Vec::with_capacity(HEADER);
            head.extend_from_slice(MAGIC);
            head.extend_from_slice(&FORMAT.to_le_bytes());
            head.extend_from_slice(&key.0);
            head.extend_from_slice(&(payload.len() as u64).to_le_bytes());
            head.extend_from_slice(&checksum(payload));
            f.write_all(&head)?;
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
        self.prune(payload.len() as u64);
        Ok(())
    }

    /// Creates the entry under its own name (no temporary file and rename: a rename costs a
    /// twentieth of a cold analysis). A reader that meets it half written finds the length or the
    /// checksum wrong, which is a miss. `false`: the name exists (a damaged or racing entry), the
    /// caller replaces it by rename.
    #[cfg(unix)]
    fn try_put_new(&self, kind: &str, key: &Key, payload: &[u8]) -> std::io::Result<bool> {
        let path = self.file(kind, key);
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create_new(true);
        let mut f = match opts.open(&path) {
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Ok(false),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                fs::create_dir_all(&self.dir)?;
                match opts.open(&path) {
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => return Ok(false),
                    r => r?,
                }
            }
            r => r?,
        };
        let mut head = Vec::with_capacity(HEADER);
        head.extend_from_slice(MAGIC);
        head.extend_from_slice(&FORMAT.to_le_bytes());
        head.extend_from_slice(&key.0);
        head.extend_from_slice(&(payload.len() as u64).to_le_bytes());
        head.extend_from_slice(&checksum(payload));
        if let Err(e) = f.write_all(&head).and_then(|()| f.write_all(payload)) {
            drop(f);
            let _ = fs::remove_file(&path);
            return Err(e);
        }
        Ok(true)
    }

    #[cfg(not(unix))]
    fn try_put_new(&self, _: &str, _: &Key, _: &[u8]) -> std::io::Result<bool> {
        Ok(false)
    }

    fn note_file(&self, kind: &str, key: &Key) -> PathBuf {
        self.dir.join(format!("{kind}-{}.seen", key.hex()))
    }

    /// Whether [`Store::note`] was called for this entry before.
    pub fn noted(&self, kind: &str, key: &Key) -> bool {
        self.note_file(kind, key).exists()
    }

    /// Remember that a run wanted this entry and did not find it, as an empty file that costs
    /// nothing to write: a caller that stores an entry only on the second request (a design
    /// that runs once should not pay for the image) asks [`Store::noted`] first. Failures are
    /// silent; the [`MAX_NOTES`] newest notes are kept.
    pub fn note(&self, kind: &str, key: &Key) {
        if fs::create_dir_all(&self.dir).is_err()
            || fs::File::create(self.note_file(kind, key)).is_err()
        {
            return;
        }
        self.trim_notes();
    }

    /// Record that `member` wanted an entry of `group` (empty `.seen` files, each named by both, the
    /// newest [`MAX_NOTES`] kept) and return how many different members have, this one included. A
    /// caller that makes a big entry only when enough different designs ask for it (each of them
    /// would have used it) asks this instead of [`Store::noted`].
    pub fn sightings(&self, kind: &str, group: &Key, member: &Key) -> usize {
        let name = |m: &str| format!("{kind}-{}.{m}.seen", group.hex());
        let mine = self.dir.join(name(&member.hex()[..16]));
        if !mine.exists() {
            if fs::create_dir_all(&self.dir).is_err() || fs::File::create(&mine).is_err() {
                return 1;
            }
            self.trim_notes();
        }
        let prefix = format!("{kind}-{}.", group.hex());
        self.names_ending(".seen")
            .iter()
            .filter(|p| {
                p.file_name()
                    .is_some_and(|n| n.to_string_lossy().starts_with(&prefix))
            })
            .count()
            .max(1)
    }

    fn claim_file(&self, kind: &str, key: &Key) -> PathBuf {
        self.dir.join(format!("{kind}-{}.making", key.hex()))
    }

    /// Say that this run is making the entry, so that runs that start meanwhile do not make it
    /// as well (an entry that costs a fifth of a second to make would otherwise be made by every
    /// test of a parallel suite that starts together). `false` when another run claimed it less
    /// than `ttl` ago; a claim that old is taken over (its run may have died). Call
    /// [`Store::release`] when the entry is stored.
    pub fn claim(&self, kind: &str, key: &Key, ttl: std::time::Duration) -> bool {
        let p = self.claim_file(kind, key);
        if fs::create_dir_all(&self.dir).is_err() {
            return false;
        }
        match fs::File::options().write(true).create_new(true).open(&p) {
            Ok(_) => true,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
                let fresh = fs::metadata(&p)
                    .and_then(|m| m.modified())
                    .ok()
                    .and_then(|t| t.elapsed().ok())
                    .is_some_and(|age| age < ttl);
                // Taking over: the claim is renewed for this run.
                !fresh && fs::File::create(&p).is_ok()
            }
            Err(_) => false,
        }
    }

    /// End a [`Store::claim`].
    pub fn release(&self, kind: &str, key: &Key) {
        let _ = fs::remove_file(self.claim_file(kind, key));
    }

    /// Keep the most recently used entries within the store's limits ([`MAX_ENTRIES`] and
    /// [`MAX_BYTES`] for a library's store),
    /// after a put of `added` bytes. The names are counted first (one directory read); the sizes
    /// and ages are asked for a directory over the entry limit, after a big entry, and at most
    /// every [`SWEEP_EVERY`] otherwise, not for every put of every process.
    fn prune(&self, added: u64) {
        use std::sync::atomic::Ordering::Relaxed;
        let n = match self.count.load(Relaxed) {
            UNKNOWN => self.names_ending(".img").len(),
            c => c + 1,
        };
        self.count.store(n, Relaxed);
        let marker = self.dir.join(SWEPT);
        let due = n > FEW_ENTRIES
            && fs::metadata(&marker)
                .and_then(|m| m.modified())
                .ok()
                .and_then(|t| t.elapsed().ok())
                .is_none_or(|age| age >= SWEEP_EVERY);
        if needs_sweep(n, added, due, self.limits) {
            let _ = fs::File::create(&marker);
            self.prune_to(self.limits.0, self.limits.1);
            self.count.store(UNKNOWN, Relaxed);
        }
    }

    /// Drop the oldest notes (`.seen` files) beyond [`MAX_NOTES`], once there are
    /// [`NOTES_SLACK`] more than that.
    fn trim_notes(&self) {
        if self.names_ending(".seen").len() > MAX_NOTES + NOTES_SLACK {
            let mut all = self.files_ending(".seen");
            all.sort_by_key(|e| std::cmp::Reverse(e.1));
            for (p, _, _) in all.split_off(MAX_NOTES) {
                let _ = fs::remove_file(p);
            }
        }
    }

    fn prune_to(&self, max_entries: usize, max_bytes: u64) {
        let mut all = self.entries();
        all.sort_by_key(|e| std::cmp::Reverse(e.1));
        let mut bytes = 0u64;
        let mut keep = 0;
        for (i, e) in all.iter().enumerate() {
            bytes += e.2;
            if i >= max_entries || (i > 0 && bytes > max_bytes) {
                break;
            }
            keep = i + 1;
        }
        for (p, _, _) in all.split_off(keep) {
            let _ = fs::remove_file(p);
        }
    }

    fn entries(&self) -> Vec<(PathBuf, std::time::SystemTime, u64)> {
        self.files_ending(".img")
    }

    /// The files whose names end in `ext`, without asking for their metadata.
    fn names_ending(&self, ext: &str) -> Vec<PathBuf> {
        let Ok(rd) = fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        rd.filter_map(|e| {
            let e = e.ok()?;
            e.file_name().to_str()?.ends_with(ext).then(|| e.path())
        })
        .collect()
    }

    fn files_ending(&self, ext: &str) -> Vec<(PathBuf, std::time::SystemTime, u64)> {
        let Ok(rd) = fs::read_dir(&self.dir) else {
            return Vec::new();
        };
        rd.filter_map(|e| {
            let e = e.ok()?;
            let name = e.file_name();
            let name = name.to_str()?;
            if !name.ends_with(ext) {
                return None;
            }
            let m = e.metadata().ok()?;
            Some((e.path(), m.modified().ok()?, m.len()))
        })
        .collect()
    }

    /// Remove every entry (and leftover temporary files), the compiled code in [`OBJ_DIR`], and
    /// the directory itself.
    pub fn clean(&self) -> std::io::Result<Cleaned> {
        let mut c = Cleaned::default();
        for (sub, limits) in [
            (OBJ_DIR, (OBJ_MAX_ENTRIES, OBJ_MAX_BYTES)),
            (UNIT_DIR, (UNIT_MAX_ENTRIES, UNIT_MAX_BYTES)),
        ] {
            let dir = self.dir.join(sub);
            if dir.is_dir() {
                let o = Store {
                    dir,
                    limits,
                    touch: true,
                    direct: false,
                    count: unknown(),
                }
                .clean()?;
                c.entries += o.entries;
                c.bytes += o.bytes;
            }
        }
        for (p, _, len) in self.entries() {
            fs::remove_file(p)?;
            c.entries += 1;
            c.bytes += len;
        }
        if let Ok(rd) = fs::read_dir(&self.dir) {
            for e in rd.flatten() {
                let n = e.file_name();
                let n = n.to_string_lossy();
                if n.starts_with(".tmp-")
                    || n.ends_with(".seen")
                    || n.ends_with(".making")
                    || n == SWEPT
                {
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

/// `bytes` is a complete entry for `key` of this format.
fn check(bytes: &[u8], key: &Key) -> bool {
    let ok = || -> Option<bool> {
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
        Some(checksum(&bytes[HEADER..]) == bytes[52..60])
    };
    ok() == Some(true)
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
    fn a_key_reads_back_from_its_hex() {
        let k = key("a");
        assert_eq!(Key::from_hex(&k.hex()), Some(k));
        for bad in ["", "ab", &k.hex()[1..], &format!("+{}", &k.hex()[1..])] {
            assert_eq!(Key::from_hex(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn payload_length_is_read_from_the_file_size() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::of_library(d.path());
        assert_eq!(s.payload_len("t", &key("a")), None);
        s.put("t", &key("a"), b"hello");
        s.put("t", &key("e"), b"");
        assert_eq!(s.payload_len("t", &key("a")), Some(5));
        assert_eq!(s.payload_len("t", &key("e")), Some(0));
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
        fs::write(&p, b"").unwrap();
        assert_eq!(s.get("t", &k), None);
        fs::write(&p, &good).unwrap();
        assert!(s.get("t", &k).is_some());
    }

    /// The units store creates entries under their own name: a cut-short file there is a miss, not
    /// an entry (`payload_len` agrees), and the next `put` replaces it; a complete one is read as
    /// before.
    #[test]
    fn a_cut_short_unit_entry_is_replaced() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::units_of_library(d.path());
        let k = key("a");
        s.put("unit", &k, b"payload bytes");
        assert_eq!(s.get("unit", &k).as_deref(), Some(&b"payload bytes"[..]));
        assert_eq!(s.payload_len("unit", &k), Some(13));
        let p = s.file("unit", &k);
        let good = fs::read(&p).unwrap();
        for cut in [0, 10, HEADER, good.len() - 1] {
            fs::write(&p, &good[..cut]).unwrap();
            assert_eq!(s.get("unit", &k), None, "cut at {cut}");
            assert_eq!(s.payload_len("unit", &k), None, "cut at {cut}");
            s.put("unit", &k, b"payload bytes");
            assert_eq!(fs::read(&p).unwrap(), good, "cut at {cut}");
        }
        assert!(
            fs::read_dir(s.dir()).unwrap().all(|e| !e
                .unwrap()
                .file_name()
                .to_string_lossy()
                .starts_with(".tmp")),
            "no temporary file is left"
        );
    }

    /// An entry that is replaced (a writer renames a new file over it) or removed while a run
    /// holds it keeps showing the bytes it was read with.
    #[test]
    fn a_held_entry_survives_replacement_and_removal() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::of_library(d.path());
        let k = key("a");
        s.put("t", &k, &[7u8; 100_000]);
        let held = s.get("t", &k).unwrap();
        s.put("t", &k, b"new");
        assert_eq!(&*held, &[7u8; 100_000][..]);
        assert_eq!(s.get("t", &k).as_deref(), Some(&b"new"[..]));
        let _ = s.clean();
        assert_eq!(&*held, &[7u8; 100_000][..]);
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
    fn notes_are_remembered_and_cleaned() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::of_library(d.path());
        assert!(!s.noted("t", &key("a")));
        s.note("t", &key("a"));
        assert!(s.noted("t", &key("a")));
        assert!(!s.noted("t", &key("b")));
        assert!(!s.noted("u", &key("a")));
        // Trimmed down to the limit once NOTES_SLACK past it, so at least the newest MAX_NOTES stay.
        for i in 0..MAX_NOTES + NOTES_SLACK + 5 {
            s.note("t", &key(&i.to_string()));
        }
        let kept = s.files_ending(".seen").len();
        assert!(
            (MAX_NOTES..=MAX_NOTES + NOTES_SLACK).contains(&kept),
            "{kept}"
        );
        assert!(s.noted("t", &key(&(MAX_NOTES + NOTES_SLACK + 4).to_string())));
        assert_eq!(s.entries().len(), 0);
        s.clean().unwrap();
        assert!(!s.dir().exists());
    }

    const LIB: (usize, u64) = (MAX_ENTRIES, MAX_BYTES);

    /// Compiled code has a store of its own inside the library's: its entries do not count
    /// against the library's limit, and cleaning the library removes them too.
    #[test]
    fn objects_have_limits_of_their_own_and_are_cleaned_with_the_library() {
        let d = tempfile::tempdir().unwrap();
        let lib = Store::of_library(d.path());
        let obj = Store::objects_of_library(d.path());
        assert!(obj.dir().starts_with(lib.dir()));
        lib.put("t", &key("a"), b"stage");
        for i in 0..MAX_ENTRIES + 5 {
            obj.put("obj", &key(&i.to_string()), b"code");
        }
        assert_eq!(obj.entries().len(), MAX_ENTRIES + 5);
        lib.put("t", &key("b"), b"stage");
        assert_eq!(lib.get("t", &key("a")).as_deref(), Some(&b"stage"[..]));
        let c = lib.clean().unwrap();
        assert_eq!(c.entries, MAX_ENTRIES + 5 + 2);
        assert!(!lib.dir().exists());
    }

    /// A store counts its puts after the first listing of the directory (#747) and still keeps
    /// to the entry limit; the newest entry is always kept.
    #[test]
    fn counted_puts_still_prune() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::of_library(d.path());
        let n = MAX_ENTRIES + MAX_ENTRIES / 2;
        for i in 0..n {
            s.put("t", &key(&i.to_string()), b"x");
        }
        assert_eq!(s.entries().len(), MAX_ENTRIES);
        assert!(s.get("t", &key(&(n - 1).to_string())).is_some());
    }

    /// Pruning asks the size and age of every entry: it waits for a reason.
    #[test]
    fn pruning_waits_for_a_reason() {
        assert!(
            !needs_sweep(FEW_ENTRIES, 100, true, LIB),
            "few entries cannot be over the limit"
        );
        assert!(
            needs_sweep(FEW_ENTRIES + 1, 100, true, LIB),
            "the first put of a process"
        );
        assert!(
            !needs_sweep(FEW_ENTRIES + 1, 100, false, LIB),
            "later puts of the process"
        );
        assert!(
            needs_sweep(MAX_ENTRIES + 1, 100, false, LIB),
            "over the entry limit"
        );
        assert!(
            needs_sweep(1, MAX_BYTES / 2, false, LIB),
            "an entry that can fill the byte limit"
        );
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

    #[test]
    fn pruning_keeps_to_the_byte_limit_but_the_newest() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::of_library(d.path());
        let t0 = std::time::SystemTime::now() - std::time::Duration::from_secs(100);
        for i in 0..4u64 {
            s.put("t", &key(&i.to_string()), &[7u8; 1000]);
            let f = fs::File::options()
                .write(true)
                .open(s.file("t", &key(&i.to_string())))
                .unwrap();
            f.set_modified(t0 + std::time::Duration::from_secs(i))
                .unwrap();
        }
        // Room for two of them.
        s.prune_to(96, 2 * (1000 + HEADER as u64) + 10);
        assert_eq!(s.entries().len(), 2);
        assert!(s.get("t", &key("3")).is_some() && s.get("t", &key("2")).is_some());
        assert!(s.get("t", &key("1")).is_none());
        // The newest stays even when it alone is over the limit.
        s.prune_to(96, 5);
        assert_eq!(s.entries().len(), 1);
    }

    /// A read refreshes the time of the entry (through the file it has open, Unix).
    #[cfg(unix)]
    #[test]
    fn a_read_refreshes_the_entry() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::of_library(d.path());
        s.put("t", &key("a"), b"x");
        let old = std::time::SystemTime::now() - std::time::Duration::from_secs(3600);
        fs::File::options()
            .write(true)
            .open(s.file("t", &key("a")))
            .unwrap()
            .set_modified(old)
            .unwrap();
        assert!(s.get("t", &key("a")).is_some());
        let t = fs::metadata(s.file("t", &key("a")))
            .unwrap()
            .modified()
            .unwrap();
        assert!(t.elapsed().unwrap() < std::time::Duration::from_secs(60));
    }

    #[test]
    fn a_claim_is_exclusive_until_released_or_old() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::of_library(d.path());
        let ttl = std::time::Duration::from_secs(60);
        assert!(s.claim("t", &key("a"), ttl));
        assert!(!s.claim("t", &key("a"), ttl), "someone is making it");
        assert!(s.claim("t", &key("b"), ttl), "another entry");
        s.release("t", &key("a"));
        assert!(s.claim("t", &key("a"), ttl));
        // A claim older than the limit is taken over.
        assert!(s.claim("t", &key("a"), std::time::Duration::ZERO));
        // Claims are not entries, and cleaning removes them.
        assert_eq!(s.entries().len(), 0);
        s.clean().unwrap();
        assert!(!s.dir().exists());
    }

    #[test]
    fn sightings_count_different_members_per_group() {
        let d = tempfile::tempdir().unwrap();
        let s = Store::of_library(d.path());
        let g = key("group");
        assert_eq!(s.sightings("t", &g, &key("a")), 1);
        assert_eq!(s.sightings("t", &g, &key("a")), 1, "the same member twice");
        assert_eq!(s.sightings("t", &g, &key("b")), 2);
        assert_eq!(s.sightings("t", &g, &key("c")), 3);
        assert_eq!(s.sightings("t", &key("other"), &key("a")), 1);
        assert_eq!(s.sightings("u", &g, &key("a")), 1, "another kind");
        assert_eq!(s.entries().len(), 0, "notes are not entries");
        s.clean().unwrap();
        assert!(!s.dir().exists());
    }
}
