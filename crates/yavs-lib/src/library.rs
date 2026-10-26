use crate::Result;
use crate::error::{debug, err, io, note};
use crate::manifest::{LOCK_FILE, MANIFEST_FILE, Manifest, Stamp, UnitRecord};
use crate::record;
use std::fs::{self, File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const LOCK_NOTE_AFTER: Duration = Duration::from_secs(3);

/// The manifest name from before the project was renamed to yavs; never read.
const OLD_MANIFEST_FILE: &str = "rsim-lib.toml"; // rename: keep

/// Why a parsed manifest is still the manifest on disk.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Fresh {
    /// The file has this size and time and was old enough to trust.
    Stamp(Stamp),
    /// Parsed under the process's own lock when `WRITES` had this value.
    Held(u64),
}

/// The manifest [`Library::read`] parsed last, with why it is still current.
type Parsed = Option<(Fresh, Arc<Manifest>)>;

/// Manifests written by sessions of this process (a parse under the lock is stale after one).
static WRITES: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// A library directory (`<dir>/yavs-lib.toml` + `<dir>/_YAVS_LIB`).
#[derive(Clone, Debug)]
pub struct Library {
    pub name: String,
    pub dir: PathBuf,
    /// The manifest as last parsed by [`Library::read`], with the stamp of the file it came from
    /// (shared by clones): a run reads each library twice, once for the closure, once for analysis.
    /// Parsed while this process held the library's lock: valid until any [`Session`] of the
    /// process writes a manifest (`WRITES`; nobody else can write while the lock is held).
    parsed: Arc<Mutex<Parsed>>,
}

impl Library {
    pub fn new(name: &str, dir: impl Into<PathBuf>) -> Self {
        Library {
            name: name.into(),
            dir: dir.into(),
            parsed: Arc::default(),
        }
    }

    fn manifest_path(&self) -> PathBuf {
        self.dir.join(MANIFEST_FILE)
    }

    /// The directory holds a manifest, possibly one under the old name that `read` rejects.
    pub fn has_manifest(&self) -> bool {
        self.manifest_path().exists() || self.dir.join(OLD_MANIFEST_FILE).exists()
    }

    /// Fail early on a library written before the rename to yavs.
    fn reject_old_manifest(&self) -> Result<()> {
        let old = self.dir.join(OLD_MANIFEST_FILE);
        if old.exists() {
            return Err(Box::new(
                err(
                    "L0004",
                    format!(
                        "library manifest `{}` was written before the rename to yavs",
                        old.display()
                    ),
                )
                .with_help("delete the library directory and re-analyze its files"),
            ));
        }
        Ok(())
    }

    /// Take a lock, noting after 3 s that we are waiting, then block.
    fn acquire(&self, f: &File, exclusive: bool) -> std::io::Result<()> {
        let start = Instant::now();
        loop {
            let r = if exclusive {
                f.try_lock()
            } else {
                f.try_lock_shared()
            };
            match r {
                Ok(()) => return Ok(()),
                Err(TryLockError::Error(e)) => return Err(e),
                Err(TryLockError::WouldBlock) if start.elapsed() < LOCK_NOTE_AFTER => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(TryLockError::WouldBlock) => {
                    note(format!(
                        "waiting for the lock on library `{}` ({})",
                        self.name,
                        self.dir.display()
                    ));
                    return if exclusive { f.lock() } else { f.lock_shared() };
                }
            }
        }
    }

    /// Take the shared lock for a read: the lock file (`None`: this process holds the exclusive
    /// lock, or the library cannot be locked) and whether this process holds the exclusive lock.
    fn lock_shared(&self) -> Result<(Option<File>, bool)> {
        let lock_path = self.dir.join(LOCK_FILE);
        let held = std::path::absolute(&self.dir).is_ok_and(|k| HELD.lock().unwrap().contains(&k));
        let opened = if held {
            Ok(None)
        } else {
            File::open(&lock_path).map(Some)
        };
        let guard = match opened {
            Ok(Some(f)) => match self.acquire(&f, false) {
                Ok(()) => Some(f),
                Err(e) => return Err(io("L0011", "lock", &lock_path, &e)),
            },
            Ok(None) => None, // this process holds the exclusive lock
            Err(e) => {
                debug(format!(
                    "reading library `{}` without a lock: {}: {e}",
                    self.name,
                    lock_path.display()
                ));
                None
            }
        };
        Ok((guard, held))
    }

    /// Read the index under a shared lock held only for the read. Never writes
    /// to the library directory, and works on read-only libraries. The records of its files
    /// are read later ([`Manifest::records`]); whoever does that holds the lock meanwhile
    /// ([`Library::read_locked`]).
    pub fn read(&self) -> Result<Arc<Manifest>> {
        let (guard, held) = self.lock_shared()?;
        let m = self.parse_index(held);
        drop(guard);
        m
    }

    /// [`Library::read`] that keeps the shared lock until the returned guard is dropped, so that
    /// no writer replaces a record while it is read.
    pub(crate) fn read_locked(&self) -> Result<(Arc<Manifest>, Option<File>)> {
        let (guard, held) = self.lock_shared()?;
        Ok((self.parse_index(held)?, guard))
    }

    fn parse_index(&self, held: bool) -> Result<Arc<Manifest>> {
        let p = self.manifest_path();
        // A manifest unmodified for two seconds that still has the size and time of the parsed one
        // is the same manifest (the library lock is held, so no writer is replacing it). One parsed
        // while this process held the lock is the same manifest too: a batch of `analyze` rewrites
        // it after every file, which is never two seconds old.
        let fresh = if held {
            Some(Fresh::Held(
                WRITES.load(std::sync::atomic::Ordering::SeqCst),
            ))
        } else {
            Stamp::of(&p).map(Fresh::Stamp)
        };
        if let Some(f) = fresh
            && let Some((s, m)) = &*self.parsed.lock().unwrap()
            && *s == f
        {
            return Ok(m.clone());
        }
        let text = fs::read_to_string(&p).map_err(|e| {
            if e.kind() == std::io::ErrorKind::NotFound {
                if let Err(d) = self.reject_old_manifest() {
                    return d;
                }
                Box::new(
                    yavs_diag::Diagnostic::error("L0001", format!("library `{}` not found", self.name)).with_help(format!(
                        "yavs looked for `{}`; analyze a file into this library first or fix the NAME:PATH mapping",
                        p.display()
                    )),
                )
            } else {
                io("L0002", "read", &p, &e)
            }
        })?;
        let m = Arc::new(Manifest::parse_owned(text, &p)?);
        if let Some(f) = fresh {
            *self.parsed.lock().unwrap() = Some((f, m.clone()));
        }
        Ok(m)
    }

    /// Record `path` (content hash `hash`) as defining `units`, replacing its earlier
    /// entry. Creates the library if needed. Holds the exclusive lock throughout and
    /// publishes the manifest with an atomic rename.
    pub fn update_file(
        &self,
        standard: &str,
        path: &Path,
        hash: &str,
        units: Vec<UnitRecord>,
    ) -> Result<()> {
        self.session()?.record(standard, path, hash, units)
    }

    /// Lock the library exclusively (creating it if needed) until the session drops;
    /// for a batch of `record`s. This process can still `read` the library meanwhile.
    pub fn session(&self) -> Result<Session> {
        fs::create_dir_all(&self.dir)
            .map_err(|e| io("L0010", "create library directory", &self.dir, &e))?;
        let lock_path = self.dir.join(LOCK_FILE);
        let lock = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&lock_path)
            .map_err(|e| {
                Box::new(
                    yavs_diag::Diagnostic::error(
                        "L0010",
                        format!("library `{}` is not writable", self.name),
                    )
                    .with_note(format!("{}: {e}", lock_path.display())),
                )
            })?;
        self.acquire(&lock, true)
            .map_err(|e| io("L0011", "lock", &lock_path, &e))?;
        let key = std::path::absolute(&self.dir).unwrap_or_else(|_| self.dir.clone());
        HELD.lock().unwrap().push(key.clone());
        // Parsed before the lock was ours: another process may have written since.
        *self.parsed.lock().unwrap() = None;
        Ok(Session {
            lib: self.clone(),
            _lock: lock,
            key,
            state: Mutex::default(),
        })
    }
}

/// Library directories exclusively locked by this process (see `Library::read`).
static HELD: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

/// An exclusive hold on a library; see `Library::session`.
pub struct Session {
    lib: Library,
    _lock: File,
    key: PathBuf,
    state: Mutex<SessionState>,
}

/// What a session keeps between records: the manifest (nobody else can write it while the
/// exclusive lock is held, so it is read and parsed once, not once per file), which also holds
/// the text of the manifest file as last read or written (a record that changes nothing skips
/// the write).
#[derive(Default)]
struct SessionState {
    manifest: Option<Manifest>,
    /// Records made since the index was last written, and the digests of the records they replaced.
    dirty: bool,
    dead: Vec<u128>,
}

impl Drop for Session {
    fn drop(&mut self) {
        let mut h = HELD.lock().unwrap();
        if let Some(i) = h.iter().position(|k| *k == self.key) {
            h.remove(i);
        }
    }
}

impl Session {
    /// Record one file now (atomic manifest rename), so it survives a later failure.
    pub fn record(
        &self,
        standard: &str,
        path: &Path,
        hash: &str,
        units: Vec<UnitRecord>,
    ) -> Result<()> {
        self.record_stamped(standard, path, hash, None, units)
    }

    /// [`Session::record`] with the stamp of the file taken before its content was read, so that
    /// an edit in between is never trusted (see [`Stamp`]).
    pub fn record_stamped(
        &self,
        standard: &str,
        path: &Path,
        hash: &str,
        stamp: Option<Stamp>,
        units: Vec<UnitRecord>,
    ) -> Result<()> {
        self.record_deferred(standard, path, hash, stamp, units)?;
        self.flush()
    }

    /// [`Session::record_stamped`] that leaves the index to [`Session::flush`]: the record of the
    /// file is written now, the index (rewritten whole) once for a batch, not once per file (#747).
    /// Nothing is recorded for a reader until the flush; a caller flushes before it returns and
    /// after a failure, so that the files before the failing one stay recorded.
    pub fn record_deferred(
        &self,
        standard: &str,
        path: &Path,
        hash: &str,
        stamp: Option<Stamp>,
        units: Vec<UnitRecord>,
    ) -> Result<()> {
        let path = std::path::absolute(path).map_err(|e| io("L0002", "resolve", path, &e))?;
        let Some(path_s) = path.to_str() else {
            return Err(Box::new(
                err(
                    "L0002",
                    format!("path `{}` is not valid UTF-8", path.display()),
                )
                .with_help(
                    "library files are recorded by path as text; rename the file or its directory",
                ),
            ));
        };
        let mp = self.lib.manifest_path();
        let mut st = self.state.lock().unwrap();
        if st.manifest.is_none() {
            // The index `read` parsed under this lock, if it did (once, not twice per call).
            let held = self.lib.parsed.lock().unwrap().take();
            let m = match held {
                Some((Fresh::Held(g), m))
                    if g == WRITES.load(std::sync::atomic::Ordering::SeqCst) =>
                {
                    Arc::unwrap_or_clone(m)
                }
                _ => match fs::read_to_string(&mp) {
                    Ok(t) => Manifest::parse_owned(t, &mp)?,
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                        self.lib.reject_old_manifest()?;
                        Manifest::new(&self.lib.name, &self.lib.dir)
                    }
                    Err(e) => return Err(io("L0002", "read", &mp, &e)),
                },
            };
            st.manifest = Some(m);
        }
        // No fsync (user, 2026-10-23, #561): the index is renamed into place, so a reader or a
        // crash sees it whole, old or new; a record goes first and nothing reads it before the
        // index names it (`write_record`), so the index never names one that is not there, and a
        // power loss may lose the last analyze, which the next analyze redoes.
        let m = st.manifest.as_ref().unwrap();
        let rdir = m.record_dir();
        let text = record::render(path_s, &units);
        let digest = record::digest(&text);
        let rp = rdir.join(record::file_name(digest));
        let intact = m.record_of(path_s) == Some(digest)
            && fs::metadata(&rp).is_ok_and(|md| md.len() == text.len() as u64);
        if !intact && let Err(e) = write_record(&rp, &text) {
            return Err(io("L0002", "write", &rp, &e));
        }
        let m = st.manifest.as_mut().unwrap();
        let mut dead: Vec<u128> = m
            .remove_file(path_s)
            .into_iter()
            .filter(|&d| d != digest)
            .collect();
        dead.extend(m.take_units(&units));
        m.push_file(path_s, hash, standard, stamp, digest, &units);
        st.dirty = true;
        st.dead.extend(dead);
        Ok(())
    }

    /// Write the index if records were made since it was last written (atomic rename), then
    /// remove the records nothing names any more.
    pub fn flush(&self) -> Result<()> {
        let mut st = self.state.lock().unwrap();
        if !std::mem::take(&mut st.dirty) {
            return Ok(());
        }
        let mp = self.lib.manifest_path();
        let m = st.manifest.as_ref().unwrap();
        let rendered = m.render();
        if rendered.text == m.src() {
            st.dead.clear();
            return Ok(());
        }
        // What `read` parsed (of this or another name of the directory) is the old index from here on:
        // bumped before the rename (so a parse of the old file made now is stale) and after it (so no
        // parse of the old file made in between stays current under the new value, #635).
        WRITES.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let r = write_new(&mp, &rendered.text);
        WRITES.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        if let Err(e) = r {
            // The in-memory index has the files now; the next record must start from the disk.
            st.manifest = None;
            st.dead.clear();
            return Err(io("L0002", "write", &mp, &e));
        }
        st.manifest.as_mut().unwrap().adopt(rendered);
        // Records nothing names any more (the files' earlier ones, files that lost every unit); not
        // one that a later record of the batch made again.
        let m = st.manifest.as_ref().unwrap();
        let rdir = m.record_dir();
        let live: std::collections::HashSet<u128> = m.record_digests().collect();
        for d in std::mem::take(&mut st.dead) {
            if !live.contains(&d) {
                let _ = fs::remove_file(rdir.join(record::file_name(d)));
            }
        }
        Ok(())
    }
}

/// Write a record under its own name, with no temporary file and rename (a record is named by the
/// digest of its text and nothing reads it before the index, written later, names it, so a reader
/// never meets it half written). A record of the same length already there is the same record; a
/// shorter one is a crashed call's, replaced whole.
fn write_record(path: &Path, text: &str) -> std::io::Result<()> {
    use std::io::Write as _;
    let mut opts = fs::OpenOptions::new();
    opts.write(true).create_new(true);
    let mut f = match opts.open(path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir_all(path.parent().expect("a directory"))?;
            opts.open(path)
        }
        r => r,
    };
    match &mut f {
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            if fs::metadata(path).is_ok_and(|m| m.len() == text.len() as u64) {
                return Ok(());
            }
            write_new(path, text)
        }
        Err(_) => f.map(drop),
        Ok(f) => f.write_all(text.as_bytes()).inspect_err(|_| {
            let _ = fs::remove_file(path);
        }),
    }
}

/// Write `text` to a temporary file next to `path` and rename it over `path` (the directory is
/// made when it is the record directory and missing). `std::fs::rename` replaces an existing
/// target on Windows too (MOVEFILE_REPLACE_EXISTING).
fn write_new(path: &Path, text: &str) -> std::io::Result<()> {
    let name = path.file_name().expect("a file name").to_string_lossy();
    let tmp = path.with_file_name(format!("{name}.{}.tmp", std::process::id()));
    let mut r = fs::write(&tmp, text);
    if r.as_ref()
        .is_err_and(|e| e.kind() == std::io::ErrorKind::NotFound)
    {
        r = fs::create_dir_all(path.parent().expect("a directory"))
            .and_then(|()| fs::write(&tmp, text));
    }
    r.and_then(|()| fs::rename(&tmp, path)).inspect_err(|_| {
        let _ = fs::remove_file(&tmp);
    })
}

#[cfg(test)]
mod tests {
    use super::write_record;

    #[test]
    fn a_cut_short_record_is_rewritten_and_an_intact_one_is_kept() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("records").join("r.toml");
        write_record(&p, "one two three\n").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "one two three\n");
        std::fs::write(&p, "one tw").unwrap();
        write_record(&p, "one two three\n").unwrap();
        assert_eq!(std::fs::read_to_string(&p).unwrap(), "one two three\n");
        let before = std::fs::metadata(&p).unwrap().modified().unwrap();
        write_record(&p, "one two three\n").unwrap();
        assert_eq!(std::fs::metadata(&p).unwrap().modified().unwrap(), before);
        assert!(
            std::fs::read_dir(d.path().join("records"))
                .unwrap()
                .all(|e| e.unwrap().file_name() == "r.toml"),
            "no temporary file is left"
        );
    }
}
