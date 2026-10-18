use crate::Result;
use crate::error::{debug, err, io, note};
use crate::manifest::{LOCK_FILE, MANIFEST_FILE, Manifest, UnitRecord};
use std::fs::{self, File, OpenOptions, TryLockError};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const LOCK_NOTE_AFTER: Duration = Duration::from_secs(3);

/// A library directory (`<dir>/rsim-lib.toml` + `<dir>/_RSIM_LIB`).
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
        let _guard = match File::open(&lock_path) {
            Ok(f) => match self.acquire(&f, false) {
                Ok(()) => Some(f),
                Err(e) => return Err(io("L0011", "lock", &lock_path, &e)),
            },
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
                Box::new(
                    rsim_diag::Diagnostic::error("L0001", format!("library `{}` not found", self.name)).with_help(format!(
                        "rsim looked for `{}`; analyze a file into this library first or fix the NAME:PATH mapping",
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
        let path = std::path::absolute(path).map_err(|e| io("L0002", "resolve", path, &e))?;
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
                    rsim_diag::Diagnostic::error(
                        "L0010",
                        format!("library `{}` is not writable", self.name),
                    )
                    .with_note(format!("{}: {e}", lock_path.display())),
                )
            })?;
        self.acquire(&lock, true)
            .map_err(|e| io("L0011", "lock", &lock_path, &e))?;

        let mp = self.manifest_path();
        let mut m = match fs::read_to_string(&mp) {
            Ok(t) => Manifest::parse(&t, &mp)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Manifest::new(&self.name, standard)
            }
            Err(e) => return Err(io("L0002", "read", &mp, &e)),
        };
        if m.standard != standard {
            return Err(Box::new(
                err(
                    "L0005",
                    format!(
                        "library `{}` holds VHDL-{} units, cannot analyze `{}` as VHDL-{standard}",
                        self.name,
                        m.standard,
                        path.display()
                    ),
                )
                .with_help(
                    "analyze with the library's standard, or use a separate library directory",
                ),
            ));
        }
        m.replace_file(path, hash.to_string(), units);

        let tmp = self
            .dir
            .join(format!("{MANIFEST_FILE}.{}.tmp", std::process::id()));
        let r = fs::write(&tmp, m.to_text())
            .and_then(|()| File::open(&tmp)?.sync_all())
            // std::fs::rename replaces an existing target on Windows too (MOVEFILE_REPLACE_EXISTING).
            .and_then(|()| fs::rename(&tmp, &mp));
        if let Err(e) = r {
            let _ = fs::remove_file(&tmp);
            return Err(io("L0002", "write", &mp, &e));
        }
        Ok(()) // lock released when `lock` drops
    }
}
