//! Semantic analysis for yavs (M1): libraries and design units, scopes and
//! visibility, the type system, overload resolution, static evaluation and
//! diagnostics. The output is a typed HIR (`hir`) in which every expression
//! carries its subtype and every call names its overload.
//!
//! The design is `docs/design/sem.md`; module ownership and conventions for
//! parallel work are in `crates/yavs-sem/CONTRIBUTING.md`.
//!
//! # Entry points
//!
//! ```no_run
//! # use yavs_sem::*;
//! let mut db = Db::new(Options::new(LangRev::Vhdl2008, default_stdlib()));
//! let records = db.analyze_file("work", "tb.vhd".as_ref(), LangRev::Vhdl2008);
//! for d in db.diagnostics() { /* render with yavs_diag::render(&db.sources, d, ..) */ }
//! ```
//!
//! - [`Db::analyze_file`]: `yavs analyze --work L f.vhd`; every unit of the
//!   file, in order; returns the `yavs_lib::UnitRecord`s for the manifest.
//! - [`Db::analyze_closure`]: a closure from `yavs_lib::closure`, in order.
//! - [`Db::load_primary`], [`Db::load_body`]: on-demand loading of one unit
//!   (everything it needs is loaded on the way).
//!
//! Fail early: any error-severity diagnostic means the result must not be
//! elaborated; the functions return `Err(Reported)` and the diagnostics are in
//! [`Db::diagnostics`].

pub mod alias;
pub mod alloc;
pub mod attr;
pub mod db;
pub mod decl;
pub mod diag;
pub mod expr;
pub mod external;
pub mod generic;
pub mod hir;
pub mod ids;
pub mod implicit;
pub mod loader;
pub mod lsp;
pub mod mangle;
pub mod protected;
pub mod reflect;
pub mod scope;
pub mod stat;
pub mod stmt;
pub mod suggest;
pub mod types;
pub mod unit;
pub mod validate;
pub mod view;

use std::path::PathBuf;

pub use db::{Db, Reported};
pub use ids::*;

/// A language revision. Each file is parsed with its own; a design is analyzed
/// against one design revision, the newest in its closure (sem.md 0.1).
pub type LangRev = yavs_syntax::Std;

#[derive(Clone, Debug)]
pub struct Options {
    /// The design revision `D`: std and ieee are loaded from `<stdlib>/{std,ieee}/<D>`.
    pub rev: LangRev,
    /// The vendored `lib/` tree.
    pub stdlib: PathBuf,
    /// Library name to directory mapping (`--work NAME:PATH`, `-L`, ...).
    pub libraries: yavs_lib::Libraries,
    /// `--relaxed` (sem.md 8.4).
    pub relaxed: bool,
    /// Conditional analysis identifiers, `(name, value)`.
    pub defines: Vec<(String, String)>,
}

impl Options {
    pub fn new(rev: LangRev, stdlib: impl Into<PathBuf>) -> Self {
        Options {
            rev,
            stdlib: stdlib.into(),
            libraries: yavs_lib::Libraries::new(),
            relaxed: false,
            defines: Vec::new(),
        }
    }
}

/// The `lib/` tree, the one place that decides it (#43): `$YAVS_STDLIB` if set, else `lib/` in
/// the executable's directory or up to three parents (an installed `yavs` next to `lib/`,
/// `bin/../lib`, `target/debug/../../lib` in a checkout). Debug builds then fall back to the
/// source checkout they were built from; a release build never does, and returns `lib/` next
/// to the executable so the "not found" diagnostic names the place it looked.
pub fn default_stdlib() -> PathBuf {
    if let Some(p) = std::env::var_os("YAVS_STDLIB") {
        return PathBuf::from(p);
    }
    let exe = std::env::current_exe().ok();
    if let Some(p) = exe
        .iter()
        .flat_map(|e| e.ancestors().skip(1).take(4))
        .map(|d| d.join("lib"))
        .find(|d| d.join("std").is_dir() && d.join("ieee").is_dir())
    {
        return p;
    }
    #[cfg(debug_assertions)]
    {
        let p = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../lib"));
        p.canonicalize().unwrap_or(p)
    }
    #[cfg(not(debug_assertions))]
    exe.as_deref()
        .and_then(std::path::Path::parent)
        .map_or_else(|| PathBuf::from("lib"), |d| d.join("lib"))
}
