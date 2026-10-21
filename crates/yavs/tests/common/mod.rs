//! Shared by the integration tests.

use std::ops::Deref;
use std::path::Path;

/// A scratch directory under the system temp dir, removed when dropped (also when the test
/// panics). `name` only makes a leftover after a killed run recognisable.
pub struct Tmp(tempfile::TempDir);

pub fn tempdir(name: &str) -> Tmp {
    Tmp(tempfile::Builder::new()
        .prefix(&format!("yavs-{name}-"))
        .tempdir()
        .expect("tmp"))
}

impl Deref for Tmp {
    type Target = Path;
    fn deref(&self) -> &Path {
        self.0.path()
    }
}

impl AsRef<Path> for Tmp {
    fn as_ref(&self) -> &Path {
        self.0.path()
    }
}
