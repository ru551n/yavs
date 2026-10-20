//! Reading an entry file: mapped into memory when the system allows it, else read into a buffer.
//!
//! The only `unsafe` in the crate (the workspace forbids it; this crate denies it and allows it
//! here), for the one call that maps a file.

use std::fs;
use std::io::Read as _;
use std::path::Path;

/// The bytes of an entry file.
pub(crate) enum Data {
    Mapped(memmap2::Mmap),
    Read(Vec<u8>),
}

impl std::fmt::Debug for Data {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} bytes", self.len())
    }
}

impl std::ops::Deref for Data {
    type Target = [u8];

    fn deref(&self) -> &[u8] {
        match self {
            Data::Mapped(m) => m,
            Data::Read(v) => v,
        }
    }
}

/// The file's bytes, or `None` when it cannot be opened. A file that cannot be mapped (an empty
/// one, a file system without mapping) is read instead.
pub(crate) fn open(path: &Path) -> Option<Data> {
    let mut f = fs::File::open(path).ok()?;
    // SAFETY: entries are never written in place: a writer fills a temporary file and renames it
    // over the entry, a prune or clean removes the name, and an open mapping keeps its pages
    // either way (Unix; on Windows removing a mapped file fails and is ignored). The bytes a
    // mapping shows therefore do not change while it lives; only something outside yavs that
    // truncates an entry in place could invalidate them (a signal on access, the same
    // exposure any mapped file has). `Entry::get` verifies the length and the checksum of what
    // it maps before anything is decoded from it.
    if let Ok(m) = unsafe { memmap2::MmapOptions::new().populate().map(&f) } {
        return Some(Data::Mapped(m));
    }
    let mut v = Vec::new();
    f.read_to_end(&mut v).ok()?;
    Some(Data::Read(v))
}
