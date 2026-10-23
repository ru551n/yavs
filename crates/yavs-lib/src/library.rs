use crate::Result;
use crate::error::{debug, err, io, note};
use crate::manifest::{LOCK_FILE, MANIFEST_FILE, Manifest, Stamp, UnitRecord};
use std::fs::{self, File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const LOCK_NOTE_AFTER: Duration = Duration::from_secs(3);

/// The manifest name from before the project was renamed to yavs; never read.
const OLD_MANIFEST_FILE: &str = "rsim-lib.toml"; // rename: keep

/// A library directory (`<dir>/yavs-lib.toml` + `<dir>/_YAVS_LIB`).
#[derive(Clone, Debug)]
pub struct Library {
    pub name: String,
    pub dir: PathBuf,
    /// The manifest as last parsed by [`Library::read`], with the stamp of the file it came from
    /// (shared by clones): a run reads each library twice, once for the closure, once for analysis.
    parsed: Arc<Mutex<Option<(Stamp, Manifest)>>>,
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

    /// Read the manifest under a shared lock held only for the read. Never writes
    /// to the library directory, and works on read-only libraries.
    pub fn read(&self) -> Result<Manifest> {
        let lock_path = self.dir.join(LOCK_FILE);
        let held = std::path::absolute(&self.dir).is_ok_and(|k| HELD.lock().unwrap().contains(&k));
        let opened = if held {
            Ok(None)
        } else {
            File::open(&lock_path).map(Some)
        };
        let _guard = match opened {
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
        let p = self.manifest_path();
        // A manifest unmodified for two seconds that still has the size and time of the parsed one
        // is the same manifest (the library lock is held, so no writer is replacing it).
        let stamp = Stamp::of(&p);
        if let Some(st) = stamp
            && let Some((s, m)) = &*self.parsed.lock().unwrap()
            && *s == st
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
        let m = Manifest::parse(&text, &p)?;
        if let Some(st) = stamp {
            *self.parsed.lock().unwrap() = Some((st, m.clone()));
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
/// exclusive lock is held, so it is read and parsed once, not once per file) and the text
/// of the manifest file as last read or written (a record that changes nothing skips the write).
#[derive(Default)]
struct SessionState {
    manifest: Option<Manifest>,
    disk_text: Option<String>,
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
        let path = std::path::absolute(path).map_err(|e| io("L0002", "resolve", path, &e))?;
        let mp = self.lib.manifest_path();
        let mut st = self.state.lock().unwrap();
        if st.manifest.is_none() {
            let (m, text) = match fs::read_to_string(&mp) {
                Ok(t) => (Manifest::parse(&t, &mp)?, Some(t)),
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                    self.lib.reject_old_manifest()?;
                    (Manifest::new(&self.lib.name), None)
                }
                Err(e) => return Err(io("L0002", "read", &mp, &e)),
            };
            st.manifest = Some(m);
            st.disk_text = text;
        }
        let m = st.manifest.as_mut().unwrap();
        m.replace_file(path, hash.to_string(), stamp, standard.to_string(), units);

        // No fsync (user, 2026-10-23, #561): the rename leaves the old or the new manifest, never a
        // torn one; a power loss may lose the last analyze, which the next analyze redoes.
        let text = m.to_text();
        if st.disk_text.as_deref() == Some(text.as_str()) {
            return Ok(());
        }
        let tmp = self
            .lib
            .dir
            .join(format!("{MANIFEST_FILE}.{}.tmp", std::process::id()));
        let r = fs::write(&tmp, &text)
            // std::fs::rename replaces an existing target on Windows too (MOVEFILE_REPLACE_EXISTING).
            .and_then(|()| fs::rename(&tmp, &mp));
        if let Err(e) = r {
            let _ = fs::remove_file(&tmp);
            // The in-memory manifest has the file now; the next record must start from the disk.
            st.manifest = None;
            st.disk_text = None;
            return Err(io("L0002", "write", &mp, &e));
        }
        st.disk_text = Some(text);
        Ok(())
    }
}
