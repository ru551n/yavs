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
mod group;
pub mod hir;
pub mod ids;
pub mod implicit;
pub mod loader;
pub mod lsp;
pub mod mangle;
mod package;
pub mod protected;
pub mod reflect;
pub mod rev;
pub mod scope;
pub mod stat;
pub mod stdlib;
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
    /// The std/ieee tree (`default_stdlib`).
    pub stdlib: PathBuf,
    /// Library name to directory mapping (`--work NAME:PATH`, `-L`, ...).
    pub libraries: yavs_lib::Libraries,
    /// `--relaxed` (sem.md 8.4).
    pub relaxed: bool,
    /// Conditional analysis identifiers, `(name, value)`.
    pub defines: Vec<(String, String)>,
    /// `--encoding`: how source files are read (decisions.md, source encoding).
    pub encoding: yavs_syntax::Encoding,
    /// `--psl`: read `-- psl` comments as PSL pragmas.
    pub psl: bool,
}

impl Options {
    pub fn new(rev: LangRev, stdlib: impl Into<PathBuf>) -> Self {
        Options {
            rev,
            stdlib: stdlib.into(),
            libraries: yavs_lib::Libraries::new(),
            relaxed: false,
            defines: Vec::new(),
            encoding: yavs_syntax::Encoding::Auto,
            psl: false,
        }
    }
}

/// The std/ieee tree, the one place that decides it: `$YAVS_STDLIB` if set, else the copy
/// built into the binary ([`stdlib::EMBEDDED`]), so a release is one executable.
pub fn default_stdlib() -> PathBuf {
    std::env::var_os("YAVS_STDLIB").map_or_else(|| PathBuf::from(stdlib::EMBEDDED), PathBuf::from)
}
