//! Library directories for rsim: a TOML manifest per library, guarded by an
//! advisory lock file (plan.md, design rule 14), plus the on-demand closure
//! query that turns a top unit into an ordered list of source files.
//!
//! There is no parser dependency: the analyzer hands in [`UnitRecord`]s.

mod closure;
mod error;
mod library;
mod manifest;

pub use closure::{ClosureFile, Libraries, closure};
pub use library::{Library, Session};
pub use manifest::{
    FileEntry, LOCK_FILE, MANIFEST_FILE, Manifest, UnitKind, UnitRecord, UnitRef, fold,
    hash_content,
};

/// Every error is a ready-to-render diagnostic (no source spans).
pub type Result<T> = std::result::Result<T, Box<rsim_diag::Diagnostic>>;

/// Render a span-less diagnostic the way rustc would (used by tests and the CLI).
pub fn render(diag: &rsim_diag::Diagnostic, color: bool) -> String {
    rsim_diag::render(
        &rsim_diag::SourceMap::default(),
        diag,
        rsim_diag::MessageFormat::Human,
        color,
    )
}
