//! Library directories for yavs: an index per library and a record per source file, guarded by an
//! advisory lock file (plan.md, design rule 14), plus the on-demand closure
//! query that turns a top unit into an ordered list of source files.
//!
//! There is no parser dependency: the analyzer hands in [`UnitRecord`]s.

mod closure;
mod error;
mod library;
mod manifest;
mod record;
mod text;

pub use closure::{ClosureFile, Libraries, closure};
pub use library::{Library, Session};
pub use manifest::{
    FileRef, LOCK_FILE, MANIFEST_FILE, Manifest, RECORD_DIR, Stamp, UnitKind, UnitLoc, UnitRecord,
    UnitRef, fold, hash_content,
};

/// Every error is a ready-to-render diagnostic (no source spans).
pub type Result<T> = std::result::Result<T, Box<yavs_diag::Diagnostic>>;

/// Render a span-less diagnostic the way rustc would (used by tests and the CLI).
pub fn render(diag: &yavs_diag::Diagnostic, color: bool) -> String {
    yavs_diag::render(
        &yavs_diag::SourceMap::default(),
        diag,
        yavs_diag::MessageFormat::Human,
        color,
    )
}
