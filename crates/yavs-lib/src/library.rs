use crate::Result;
use crate::error::{debug, err, io, note};
use crate::manifest::{LOCK_FILE, MANIFEST_FILE, Manifest, UnitRecord};
use std::fs::{self, File, OpenOptions, TryLockError};
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const LOCK_NOTE_AFTER: Duration = Duration::from_secs(3);

/// The manifest name from before the project was renamed to yavs; never read.
const OLD_MANIFEST_FILE: &str = "rsim-lib.toml"; // rename: keep

/// A library directory (`<dir>/yavs-lib.toml` + `<dir>/_YAVS_LIB`).
#[derive(Clone, Debug)]
pub struct Library {
    pub name: String,
    pub dir: PathBuf,
}

impl Library {
    pub fn new(name: &str, dir: impl Into<PathBuf>) -> Self {
        Library {
            name: name.into(),
            dir: dir.into(),
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
        Manifest::parse(&text, &p)
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
        let path = std::path::absolute(path).map_err(|e| io("L0002", "resolve", path, &e))?;
        let mp = self.lib.manifest_path();
        let mut m = match fs::read_to_string(&mp) {
            Ok(t) => Manifest::parse(&t, &mp)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                self.lib.reject_old_manifest()?;
                Manifest::new(&self.lib.name)
            }
            Err(e) => return Err(io("L0002", "read", &mp, &e)),
        };
        m.replace_file(path, hash.to_string(), standard.to_string(), units);

        let tmp = self
            .lib
            .dir
            .join(format!("{MANIFEST_FILE}.{}.tmp", std::process::id()));
        // Synced through the writing handle: Windows refuses FlushFileBuffers on a read-only one.
        let r = File::create(&tmp)
            .and_then(|mut f| {
                f.write_all(m.to_text().as_bytes())?;
                f.sync_all()
            })
            // std::fs::rename replaces an existing target on Windows too (MOVEFILE_REPLACE_EXISTING).
            .and_then(|()| fs::rename(&tmp, &mp));
        if let Err(e) = r {
            let _ = fs::remove_file(&tmp);
            return Err(io("L0002", "write", &mp, &e));
        }
        Ok(())
    }
}
