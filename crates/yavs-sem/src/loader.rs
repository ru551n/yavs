//! Libraries, the unit graph and the on-demand unit loader (sem.md 1.2).
//!
//! `Db::load_unit` returns a unit at once when it is analyzed; otherwise it
//! finds the unit's file (library manifest, or the `lib/` tree for std and
//! ieee), parses the whole file and analyzes only the requested unit. The
//! other units of the file keep their AST in the file cache until asked for.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use yavs_diag::{Diagnostic, FileId, Span};
use yavs_lib::{Manifest, UnitKind as LibKind, UnitRecord, UnitRef};
use yavs_syntax::Symbol;
use yavs_syntax::ast::{self, Ident};

use crate::LangRev;
use crate::db::{Db, Reported};
use crate::decl::{Decl, DeclKind, Designator};
use crate::diag;
use crate::hir::UnitHir;
use crate::ids::{DeclId, LibId, ScopeId, UnitId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum UnitKey {
    /// Primary units share one namespace per library.
    Primary(Symbol),
    /// `(primary, architecture)` or `(package, "body")`.
    Secondary(Symbol, Symbol),
}

impl UnitKey {
    pub fn body(pkg: Symbol) -> Self {
        UnitKey::Secondary(pkg, Symbol::intern("body"))
    }

    fn of(u: &ast::LibraryUnit) -> UnitKey {
        use ast::LibraryUnit as L;
        match u {
            L::Entity(e) => UnitKey::Primary(e.name.key),
            L::Architecture(a) => UnitKey::Secondary(a.entity.key, a.name.key),
            L::Package(p) => UnitKey::Primary(p.name.key),
            L::PackageBody(b) => UnitKey::body(b.name.key),
            L::PackageInstance(p) => UnitKey::Primary(p.name.key),
            L::Configuration(c) => UnitKey::Primary(c.name.key),
            L::Context(c) => UnitKey::Primary(c.name.key),
        }
    }

    fn describe(&self) -> String {
        match self {
            UnitKey::Primary(k) => format!("`{k}`"),
            UnitKey::Secondary(p, s) if s.as_str() == "body" => format!("package body `{p}`"),
            UnitKey::Secondary(p, s) => format!("architecture `{s}` of `{p}`"),
        }
    }
}

pub enum UnitSlot {
    NotLoaded {
        file: PathBuf,
        rev: LangRev,
    },
    InProgress(UnitId),
    Done(UnitId),
    /// Its errors were reported; dependents stay silent (8.2).
    Failed,
}

pub enum LibSource {
    /// A library directory with `yavs-lib.toml`.
    Manifest(Manifest),
    /// The vendored `lib/<name>/<rev>` tree: package `p` is in `p.vhdl`, its
    /// body in `p-body.vhdl`.
    // ponytail: file-name convention instead of an index of the tree; add an
    // index if a vendored file ever holds a unit under another name.
    StdDir(PathBuf),
    /// Only what `analyze_file` registers (a fresh work library).
    Files,
}

pub struct Library {
    pub name: Ident,
    /// The library's own `DeclKind::Library` entity.
    pub decl: DeclId,
    pub source: LibSource,
    pub units: HashMap<UnitKey, UnitSlot>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum UnitKind {
    Entity,
    Architecture,
    Package,
    PackageBody,
    PackageInst,
    Context,
    Configuration,
}

pub struct Unit {
    pub lib: LibId,
    pub key: UnitKey,
    pub kind: UnitKind,
    pub name: Ident,
    /// The unit's `DeclKind::Unit` entity.
    pub decl: DeclId,
    /// The unit's declarative region.
    pub scope: ScopeId,
    /// Its context clause region (library/use/context items).
    pub context: ScopeId,
    /// Units named by its context clause or by selected names, and its primary.
    pub deps: Vec<UnitId>,
    /// Units named but not analyzed for this one (an instance's architecture, a
    /// default-bound entity): manifest dependencies only (unit.rs).
    pub refs: Vec<UnitRef>,
    pub file: FileId,
    pub span: Span,
    /// The file's own revision `F`.
    pub rev: LangRev,
    pub hir: UnitHir,
}

pub(crate) struct FileCache {
    pub file: FileId,
    /// `None` once the parse failed.
    pub units: Option<Vec<(UnitKey, Option<ast::DesignUnit>)>>,
}

fn parse_rev(s: &str) -> Option<LangRev> {
    match s {
        "2008" => Some(LangRev::Vhdl2008),
        "2019" => Some(LangRev::Vhdl2019),
        _ => None,
    }
}

fn ident(text: &str) -> Ident {
    Ident {
        text: Symbol::intern(text),
        key: Symbol::intern(&yavs_lib::fold(text)),
        span: Span::default(),
    }
}

impl Db {
    /// The library named `key` (folded), loading its manifest on first use.
    pub(crate) fn library(&mut self, name: Ident) -> Result<LibId, Reported> {
        if let Some(&id) = self.lib_by_key.get(&name.key) {
            return id.ok_or(Reported);
        }
        let key = name.key.as_str();
        // A mapped `std`/`ieee` (only possible with `yavs analyze --bootstrap`) replaces
        // the vendored tree; a library without a manifest is an empty library
        // (cli-contract section 3).
        let source = match self.opts.libraries.get(key) {
            Some(l) if !l.has_manifest() => Some(Ok(LibSource::Files)),
            Some(l) => Some(l.read().map(LibSource::Manifest)),
            None if key == "std" || key == "ieee" => Some(Ok(LibSource::StdDir(
                self.opts.stdlib.join(key).join(self.rev.as_str()),
            ))),
            None => None,
        };
        let r = match source {
            Some(Ok(source)) => Ok(self.add_library(name, source)),
            Some(Err(d)) => {
                let mut d = *d;
                if name.span != Span::default() {
                    d = d.with_primary(name.span, "library used here");
                }
                Err(self.error(d))
            }
            None => {
                let mut known: Vec<String> =
                    self.opts.libraries.names().map(String::from).collect();
                known.extend(["std".into(), "ieee".into()]);
                known.sort();
                let mut d = Diagnostic::error(
                    diag::UNKNOWN_LIBRARY,
                    format!("unknown library `{}`", name.text),
                );
                if name.span != Span::default() {
                    d = d.with_primary(name.span, "not a mapped library");
                }
                d = d.with_note(format!("mapped libraries: {}", known.join(", ")));
                if let Some(best) = crate::suggest::closest(key, known.iter().map(String::as_str)) {
                    d = d.with_help(format!("did you mean `{best}`?"));
                } else {
                    d = d.with_help(format!("map it with `{key}:PATH`"));
                }
                Err(self.error(d))
            }
        };
        self.lib_by_key.insert(name.key, r.ok());
        r
    }

    fn add_library(&mut self, name: Ident, source: LibSource) -> LibId {
        let id = self.libs.next_id();
        let decl = self.decls.push(Decl {
            name: Designator::Ident(name),
            kind: DeclKind::Library(id),
            scope: self.root,
            span: name.span,
            implicit: false,
            since: LangRev::Vhdl2008,
        });
        self.libs.push(Library {
            name,
            decl,
            source,
            units: HashMap::new(),
        })
    }

    /// Where the unit's source is, from the slot or the library source.
    pub(crate) fn locate(&self, lib: LibId, key: UnitKey) -> Option<(PathBuf, LangRev)> {
        if let Some(UnitSlot::NotLoaded { file, rev }) = self.libs[lib].units.get(&key) {
            return Some((file.clone(), *rev));
        }
        match &self.libs[lib].source {
            LibSource::Files => None,
            LibSource::StdDir(dir) => {
                let file = match key {
                    UnitKey::Primary(k) => dir.join(format!("{k}.vhdl")),
                    UnitKey::Secondary(p, _) => dir.join(format!("{p}-body.vhdl")),
                };
                file.is_file().then_some((file, self.rev))
            }
            LibSource::Manifest(m) => {
                let rec = match key {
                    UnitKey::Primary(k) => [
                        LibKind::Entity,
                        LibKind::Package,
                        LibKind::PackageInstance,
                        LibKind::Configuration,
                        LibKind::Context,
                    ]
                    .iter()
                    .find_map(|&kind| m.find(kind, k.as_str())),
                    UnitKey::Secondary(p, s) if s.as_str() == "body" => {
                        m.find(LibKind::PackageBody, p.as_str())
                    }
                    UnitKey::Secondary(e, a) => m.find(LibKind::Architecture, &format!("{e}/{a}")),
                };
                rec.and_then(|r| Some((r.file.clone(), parse_rev(&r.std)?)))
            }
        }
    }

    /// Parse a file into the cache (all its units). Errors are reported once.
    pub(crate) fn parse_into_cache(
        &mut self,
        path: &Path,
        rev: LangRev,
        at: Option<Span>,
    ) -> Result<(), Reported> {
        if let Some(fc) = self.files.get(path) {
            return if fc.units.is_some() {
                Ok(())
            } else {
                Err(Reported)
            };
        }
        let bytes = match std::fs::read(path) {
            Ok(b) => b,
            Err(e) => {
                let mut d = Diagnostic::error(
                    diag::SOURCE_UNREADABLE,
                    format!("cannot read source file `{}`", path.display()),
                )
                .with_note(e.to_string());
                if let Some(at) = at {
                    d = d.with_primary(at, "needed here");
                }
                let file = self.sources.add(path.display().to_string(), String::new());
                self.files
                    .insert(path.to_path_buf(), FileCache { file, units: None });
                return Err(self.error(d));
            }
        };
        // Auto never reports a diagnostic; `--encoding` reaches here with the CLI.
        let (file, _) = yavs_syntax::load_source(
            &mut self.sources,
            path.display().to_string(),
            bytes,
            yavs_syntax::Encoding::Auto,
        );
        let opts = yavs_syntax::Options {
            std: rev,
            defines: self.opts.defines.clone(),
        };
        let parsed = yavs_syntax::parse_file(file, self.sources.get(file), &opts);
        for d in parsed.diags {
            self.report(d);
        }
        let units = parsed.design.map(|d| {
            d.units
                .into_iter()
                .map(|u| (UnitKey::of(&u.unit), Some(u)))
                .collect()
        });
        let ok = units.is_some();
        self.files
            .insert(path.to_path_buf(), FileCache { file, units });
        if ok { Ok(()) } else { Err(Reported) }
    }

    /// The unit `key` of `lib`, analyzing it on first use. `at` is the
    /// reference that needs it (for diagnostics).
    pub(crate) fn load_unit(
        &mut self,
        lib: LibId,
        key: UnitKey,
        at: Option<Span>,
    ) -> Result<UnitId, Reported> {
        match self.libs[lib].units.get(&key) {
            Some(UnitSlot::Done(u)) => return Ok(*u),
            Some(UnitSlot::Failed) => return Err(Reported),
            Some(UnitSlot::InProgress(u)) => {
                let u = *u;
                return Err(self.cycle(u, at));
            }
            _ => {}
        }
        let Some((path, rev)) = self.locate(lib, key) else {
            return Err(self.not_found(lib, key, at));
        };
        if self.parse_into_cache(&path, rev, at).is_err() {
            self.libs[lib].units.insert(key, UnitSlot::Failed);
            return Err(Reported);
        }
        let fc = self.files.get_mut(&path).expect("parsed above");
        let file = fc.file;
        let ast = fc
            .units
            .as_mut()
            .and_then(|us| us.iter_mut().find(|(k, u)| *k == key && u.is_some()))
            .and_then(|(_, u)| u.take());
        let Some(ast) = ast else {
            return Err(self.not_found(lib, key, at));
        };
        let r = self.analyze_unit(lib, key, ast, file, rev);
        if r.is_err() {
            self.libs[lib].units.insert(key, UnitSlot::Failed);
        }
        r
    }

    /// `load_unit` from a reference in the unit being analyzed: a failed
    /// dependency taints it, so its later undeclared-name errors are cascades
    /// and stay silent (8.2).
    pub(crate) fn load_dep(
        &mut self,
        lib: LibId,
        key: UnitKey,
        at: Option<Span>,
    ) -> Result<UnitId, Reported> {
        let r = self.load_unit(lib, key, at);
        match r {
            Ok(u) => self.note_dep(u),
            Err(_) => {
                if let Some(c) = self.cur() {
                    self.tainted.insert(c.unit);
                }
            }
        }
        r
    }

    fn not_found(&mut self, lib: LibId, key: UnitKey, at: Option<Span>) -> Reported {
        let l = &self.libs[lib];
        let looked = match &l.source {
            LibSource::Manifest(_) => match self.opts.libraries.get(l.name.key.as_str()) {
                Some(dir) => format!("looked in the manifest of `{}`", dir.dir.display()),
                None => "looked in the library manifest".into(),
            },
            LibSource::StdDir(d) => format!("looked in `{}`", d.display()),
            LibSource::Files => {
                "the library has only the units of the files analyzed in this run".into()
            }
        };
        let mut d = Diagnostic::error(
            diag::UNIT_NOT_FOUND,
            format!(
                "design unit {} not found in library `{}`",
                key.describe(),
                l.name.text
            ),
        )
        .with_note(looked)
        .with_help("analyze the file that defines it first, or check the name");
        if let Some(at) = at {
            d = d.with_primary(at, "needed here");
        }
        self.error(d)
    }

    fn cycle(&mut self, unit: UnitId, at: Option<Span>) -> Reported {
        let name = self.units[unit].name;
        let mut d = Diagnostic::error(
            diag::DEPENDENCY_CYCLE,
            format!("design unit `{}` depends on itself", name.text),
        );
        if let Some(at) = at {
            d = d.with_primary(at, "needed here, while it is being analyzed");
        }
        let start = self.stack.iter().position(|c| c.unit == unit).unwrap_or(0);
        for c in &self.stack[start..] {
            let u = &self.units[c.unit];
            d = d.with_secondary(u.span, format!("`{}` is being analyzed", u.name.text));
        }
        self.error(d.with_help(
            "break the cycle, for example by moving shared declarations into a separate package",
        ))
    }

    /// Analyze every unit of `path` into library `work`, in textual order
    /// (`yavs analyze --work L f.vhd`). A later unit sees earlier ones through
    /// `work`. Returns the records for the library manifest.
    pub fn analyze_file(
        &mut self,
        work: &str,
        path: &Path,
        rev: LangRev,
    ) -> Result<Vec<UnitRecord>, Reported> {
        let name = ident(work);
        let lib =
            if self.opts.libraries.get(work).is_some() || self.lib_by_key.contains_key(&name.key) {
                self.library(name)?
            } else {
                // An unmapped work library holds only what this session analyzes.
                let l = self.add_library(name, LibSource::Files);
                self.lib_by_key.insert(name.key, Some(l));
                l
            };
        self.parse_into_cache(path, rev, None)?;
        let keys: Vec<UnitKey> = self.files[path]
            .units
            .as_ref()
            .expect("parsed")
            .iter()
            .map(|(k, _)| *k)
            .collect();
        for &k in &keys {
            self.libs[lib].units.insert(
                k,
                UnitSlot::NotLoaded {
                    file: path.to_path_buf(),
                    rev,
                },
            );
        }
        let before = self.error_count();
        let mut ids = Vec::new();
        for &k in &keys {
            if let Ok(u) = self.load_unit(lib, k, None) {
                ids.push(u);
            }
        }
        // A unit that failed, or needed one that failed, without a new error
        // depends on a unit that failed earlier in this session (8.2: silent);
        // the file still failed. A failed file is not recorded, so none of its
        // units is visible to later files, as with one `yavs analyze` per file.
        if self.error_count() > before
            || ids.len() < keys.len()
            || ids.iter().any(|u| self.tainted.contains(u))
        {
            for &k in &keys {
                self.libs[lib].units.insert(k, UnitSlot::Failed);
            }
            return Err(Reported);
        }
        // HIR invariants (sem.md 7.3) in debug builds.
        #[cfg(debug_assertions)]
        if crate::validate::validate(self) > 0 {
            return Err(Reported);
        }
        Ok(ids.into_iter().map(|u| self.unit_record(u)).collect())
    }

    /// Analyze a whole closure in compile order (`yavs_lib::closure`).
    pub fn analyze_closure(&mut self, files: &[yavs_lib::ClosureFile]) -> Result<(), Reported> {
        let mut r = Ok(());
        for f in files {
            let Some(rev) = parse_rev(&f.std) else {
                r = Err(self.error(Diagnostic::error(
                    diag::NOT_IN_REVISION,
                    format!(
                        "file `{}` has unknown standard `{}`",
                        f.path.display(),
                        f.std
                    ),
                )));
                continue;
            };
            if self.analyze_file(&f.library, &f.path, rev).is_err() {
                r = Err(Reported);
            }
        }
        r
    }

    /// Load a primary unit by name, e.g. `("ieee", "numeric_std")`.
    pub fn load_primary(&mut self, lib: &str, name: &str) -> Result<UnitId, Reported> {
        let lib = self.library(ident(lib))?;
        self.load_unit(
            lib,
            UnitKey::Primary(Symbol::intern(&yavs_lib::fold(name))),
            None,
        )
    }

    /// Load a package body.
    pub fn load_body(&mut self, lib: &str, pkg: &str) -> Result<UnitId, Reported> {
        let lib = self.library(ident(lib))?;
        self.load_unit(
            lib,
            UnitKey::body(Symbol::intern(&yavs_lib::fold(pkg))),
            None,
        )
    }

    /// Load secondary unit `secondary` of primary `primary` in `lib`, e.g. an
    /// architecture named by an instance or a configuration (yavs-elab).
    pub fn load_secondary(
        &mut self,
        lib: LibId,
        primary: Symbol,
        secondary: Symbol,
        at: Option<Span>,
    ) -> Result<UnitId, Reported> {
        self.load_unit(lib, UnitKey::Secondary(primary, secondary), at)
    }

    /// The most recently analyzed architecture of `entity` (LRM 7.3.3): the last
    /// one in manifest order, or in analysis order for a library of this session.
    pub fn last_architecture(&self, entity: UnitId) -> Option<Symbol> {
        let (lib, key) = (self.units[entity].lib, self.units[entity].name.key);
        match &self.libs[lib].source {
            LibSource::Manifest(m) => m
                .units()
                .filter(|u| {
                    u.kind == LibKind::Architecture && u.primary.as_deref() == Some(key.as_str())
                })
                .last()
                .and_then(|u| u.key.split_once('/').map(|(_, a)| Symbol::intern(a))),
            _ => self
                .units
                .iter()
                .filter(|(_, u)| {
                    u.lib == lib
                        && u.kind == UnitKind::Architecture
                        && matches!(u.key, UnitKey::Secondary(e, _) if e == key)
                })
                .last()
                .map(|(_, u)| u.name.key),
        }
    }

    /// The manifest record of an analyzed unit. Dependencies on the vendored
    /// std/ieee tree are left out: they are not manifest libraries.
    pub fn unit_record(&self, u: UnitId) -> UnitRecord {
        let unit = &self.units[u];
        let rev = unit.rev.as_str();
        let name = unit.name.text.as_str();
        let mut rec = match (unit.kind, unit.key) {
            (UnitKind::Architecture, UnitKey::Secondary(e, _)) => {
                UnitRecord::architecture(e.as_str(), name, rev)
            }
            (UnitKind::PackageBody, _) => UnitRecord::package_body(name, rev),
            (k, _) => UnitRecord::new(lib_kind(k), name, rev),
        };
        for &d in &unit.deps {
            let du = &self.units[d];
            if matches!(self.libs[du.lib].source, LibSource::StdDir(_)) {
                continue;
            }
            let key = match du.key {
                UnitKey::Primary(k) => k.to_string(),
                UnitKey::Secondary(p, s) if du.kind == UnitKind::Architecture => format!("{p}/{s}"),
                UnitKey::Secondary(p, _) => p.to_string(),
            };
            let r = UnitRef {
                library: self.libs[du.lib].name.key.to_string(),
                kind: lib_kind(du.kind),
                key,
            };
            if !rec.depends_on.contains(&r) {
                rec.depends_on.push(r);
            }
        }
        for r in &unit.refs {
            if !rec.depends_on.contains(r) {
                rec.depends_on.push(r.clone());
            }
        }
        rec
    }
}

fn lib_kind(k: UnitKind) -> LibKind {
    match k {
        UnitKind::Entity => LibKind::Entity,
        UnitKind::Architecture => LibKind::Architecture,
        UnitKind::Package => LibKind::Package,
        UnitKind::PackageBody => LibKind::PackageBody,
        UnitKind::PackageInst => LibKind::PackageInstance,
        UnitKind::Context => LibKind::Context,
        UnitKind::Configuration => LibKind::Configuration,
    }
}
