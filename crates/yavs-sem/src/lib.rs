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
pub mod edge;
pub mod expr;
pub mod external;
pub mod generic;
mod group;
pub mod hir;
pub mod ids;
pub mod image;
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
pub mod tracked;
pub mod types;
pub mod unit;
pub mod units;
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
    /// Where later stages keep what they derive from this analysis (`StageCache`); set by the
    /// CLI after analysis, `None` for no caching.
    pub stages: Option<std::sync::Arc<dyn StageCache>>,
    /// The analysed-unit cache (`units.rs`): units are read from it when their entry holds, and
    /// written to it when it is writable. `None`: every unit is analysed from source.
    pub units: Option<std::sync::Arc<dyn UnitStore>>,
    /// Compare every table of `Db` before and after each unit recorded for the analysed-unit cache
    /// and fail when that finds a change the write log of the table missed (`units.rs`
    /// `Db::diff`). Default: debug builds; `YAVS_UNIT_CHECK=1` sets it for `yavs analyze`.
    pub unit_audit: bool,
}

/// The store of the analysed-unit cache (`docs/decisions.md` "Analysed-unit cache"): entries by
/// library name and identity (`units.rs`). A read that fails is a miss, a write that fails is
/// silent.
pub trait UnitStore: Send + Sync + std::fmt::Debug {
    fn get(&self, lib: &str, identity: &[u8; 32]) -> Option<yavs_diag::Buffer>;
    /// Whether an entry is there, without reading it (its content is not checked).
    fn has(&self, lib: &str, identity: &[u8; 32]) -> bool {
        self.get(lib, identity).is_some()
    }
    fn put(&self, lib: &str, identity: &[u8; 32], bytes: &[u8]);
    /// Whether units analysed from source are written (`yavs analyze`; never `yavs run`).
    fn writable(&self) -> bool;
    /// A hit, a miss and why, for `YAVS_LOG=debug` (the message is made only when it is shown).
    fn note(&self, _msg: &dyn Fn() -> String) {}
}

/// A store for what later stages derive from an analyzed `Db` (#407): the lowered run-time
/// packages. Entries belong to one analysis, whose key the implementation holds; `extent` is
/// [`Db::stage_extent`] of the `Db` the stage ran on: an entry holds for a `Db` of the same
/// size made from the same files, however far elaboration has grown it since the analysis
/// (instantiation, units loaded late, #671). A write failure is silent.
pub trait StageCache: Send + Sync + std::fmt::Debug {
    fn get(&self, extent: &[usize], kind: &str, part: &[u8]) -> Option<yavs_diag::Buffer>;
    fn put(&self, extent: &[usize], kind: &str, part: &[u8], bytes: &[u8]);
    /// A reason an entry was not made, for `YAVS_LOG=debug`.
    fn note(&self, _msg: &str) {}

    /// A name of the entry `get` and `put` use for these arguments, the same in every run that
    /// would read it (`None`: no such entry is kept).
    fn identity(&self, _extent: &[usize], _kind: &str, _part: &[u8]) -> Option<String> {
        None
    }

    /// Whether the closure has entries of its own (it was analyzed before: the images of a
    /// closure are made on its second run). Without them only the library prefixes serve.
    fn closure(&self) -> bool {
        true
    }

    /// The [`Db::extent`] the analysis ended with: the library prefix entries are valid for a
    /// `Db` that has not grown since.
    fn analysed(&self) -> Vec<usize> {
        Vec::new()
    }

    /// The library prefixes of the closure the analysis went through, longest first.
    fn prefixes(&self) -> Vec<Prefix> {
        Vec::new()
    }

    /// Entries of library prefix `level` ([`StageCache::prefixes`]): shared by every closure
    /// that starts with the same files. `part` names what the stage was asked for.
    fn prefix_get(&self, _level: usize, _kind: &str, _part: &[u8]) -> Option<yavs_diag::Buffer> {
        None
    }

    fn prefix_put(&self, _level: usize, _kind: &str, _part: &[u8], _bytes: &[u8]) {}

    /// How many different closures (this one included) have asked for the entry of this prefix and
    /// not found it: the request is remembered. A stage that costs a fraction of a second to make
    /// is made once enough closures would have used it.
    fn prefix_sightings(&self, _level: usize, _kind: &str, _part: &[u8]) -> usize {
        usize::MAX
    }

    /// Whether this run may make the entry: `false` while another run is making it (the claim
    /// ends with [`StageCache::prefix_release`] or after a while).
    fn prefix_claim(&self, _level: usize, _kind: &str, _part: &[u8]) -> bool {
        true
    }

    /// End a [`StageCache::prefix_claim`].
    fn prefix_release(&self, _level: usize, _kind: &str, _part: &[u8]) {}

    /// The `Db` of library prefix `level` as it was when the prefix ended, read from its image:
    /// what a prefix stage lowers from, so that its result depends on the prefix alone.
    fn prefix_db(&self, _level: usize) -> Option<Db> {
        None
    }
}

/// A leading part of the files of a closure whose analysis is stored on its own and shared by
/// the closures that start with the same files (`StageCache::prefixes`).
#[derive(Clone, Debug)]
pub struct Prefix {
    /// The number of units the `Db` held when the prefix ended: a unit with a smaller index
    /// belongs to it.
    pub units: usize,
    /// [`Db::extent`] when the prefix ended.
    pub extent: Vec<usize>,
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
            unit_audit: cfg!(debug_assertions),
            stages: None,
            units: None,
        }
    }
}

/// The std/ieee tree, the one place that decides it: `$YAVS_STDLIB` if set, else the copy
/// built into the binary ([`stdlib::EMBEDDED`]), so a release is one executable.
pub fn default_stdlib() -> PathBuf {
    std::env::var_os("YAVS_STDLIB").map_or_else(|| PathBuf::from(stdlib::EMBEDDED), PathBuf::from)
}
