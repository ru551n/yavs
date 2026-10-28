//! Libraries, the unit graph and the on-demand unit loader (sem.md 1.2).
//!
//! `Db::load_unit` returns a unit at once when it is analyzed; otherwise it
//! finds the unit's file (library manifest, or the `lib/` tree for std and
//! ieee), parses the whole file and analyzes only the requested unit. The
//! other units of the file keep their AST in the file cache until asked for.

use rustc_hash::FxHashMap as HashMap;
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

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
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

#[derive(serde::Serialize, serde::Deserialize)]
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

/// An image stores only which kind of source a library has; `Db::from_image` attaches the
/// current manifest or std tree again (image.rs).
mod source_tag {
    use super::LibSource;

    pub fn serialize<S: serde::Serializer>(v: &LibSource, s: S) -> Result<S::Ok, S::Error> {
        s.serialize_u8(match v {
            LibSource::Manifest(_) => 0,
            LibSource::StdDir(_) => 1,
            LibSource::Files => 2,
        })
    }

    pub fn deserialize<'de, D: serde::Deserializer<'de>>(d: D) -> Result<LibSource, D::Error> {
        Ok(match <u8 as serde::Deserialize>::deserialize(d)? {
            0 => LibSource::Manifest(std::sync::Arc::default()),
            1 => LibSource::StdDir(std::path::PathBuf::new()),
            2 => LibSource::Files,
            _ => return Err(serde::de::Error::custom("bad library source tag")),
        })
    }
}

#[derive(Clone)]
pub enum LibSource {
    /// A library directory with `yavs-lib.toml`.
    Manifest(std::sync::Arc<Manifest>),
    /// The vendored `lib/<name>/<rev>` tree: package `p` is in `p.vhdl`, its
    /// body in `p-body.vhdl`.
    // ponytail: file-name convention instead of an index of the tree; add an
    // index if a vendored file ever holds a unit under another name.
    StdDir(PathBuf),
    /// Only what `analyze_file` registers (a fresh work library).
    Files,
}

#[derive(serde::Serialize, serde::Deserialize)]
pub struct Library {
    pub name: Ident,
    /// The library's own `DeclKind::Library` entity.
    pub decl: DeclId,
    #[serde(with = "source_tag")]
    pub source: LibSource,
    pub units: HashMap<UnitKey, UnitSlot>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum UnitKind {
    Entity,
    Architecture,
    Package,
    PackageBody,
    PackageInst,
    Context,
    Configuration,
}

#[derive(serde::Serialize, serde::Deserialize)]
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
    /// `None` once the parse failed (or before it, see `parsed`).
    pub units: Option<Vec<(UnitKey, Option<ast::DesignUnit>)>>,
    /// `false`: registered by the analysed-unit cache, which read units of the file from their
    /// entries; the file is parsed when a unit of it is analysed from source after all.
    pub parsed: bool,
    /// The diagnostics of reading and parsing it (reported once, when it was registered).
    pub diags: Vec<Diagnostic>,
    /// Its content hash, when the analysed-unit cache is on (`units.rs`).
    pub hash: Option<String>,
}

fn parse_rev(s: &str) -> Option<LangRev> {
    match s {
        "1993" => Some(LangRev::Vhdl1993),
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
            if let Some(l) = id {
                self.lib_step(name, |_| l);
            }
            return id.ok_or(Reported);
        }
        let key = name.key.as_str();
        let source = self.lib_source(key);
        let r = match source {
            Some(Ok(source)) => Ok(self.lib_step(name, |db| db.add_library(name, source))),
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
                if let Some(h) = crate::suggest::suggest(
                    key,
                    "library",
                    known.iter().map(|k| (k.as_str(), k.as_str())),
                ) {
                    d = d.with_help(h);
                } else {
                    d = d.with_help(format!("map it with `{key}:PATH`"));
                }
                Err(self.error(d))
            }
        };
        self.lib_by_key.insert(name.key, r.ok());
        r
    }

    /// The source of library `key` (folded) as it is now: `None` for a library that is not
    /// mapped. A mapped `std`/`ieee` (only possible with `yavs analyze --bootstrap`) replaces the
    /// vendored tree; a library without a manifest is an empty library (cli-contract section 3).
    pub(crate) fn lib_source(
        &self,
        key: &str,
    ) -> Option<Result<LibSource, Box<yavs_diag::Diagnostic>>> {
        match self.opts.libraries.get(key) {
            Some(l) if !l.has_manifest() => Some(Ok(LibSource::Files)),
            // The index of a library is read once per session: a unit asks for the libraries it
            // depends on again for every file of a call (#748).
            Some(l) => {
                if let Some(m) = self.entry_cache.manifests.lock().unwrap().get(key) {
                    return Some(Ok(LibSource::Manifest(m.clone())));
                }
                let r = l.read();
                if let Ok(m) = &r {
                    self.entry_cache
                        .manifests
                        .lock()
                        .unwrap()
                        .insert(key.to_string(), m.clone());
                }
                Some(r.map(LibSource::Manifest))
            }
            None if key == "std" || key == "ieee" => Some(Ok(LibSource::StdDir(
                self.opts.stdlib.join(key).join(self.rev.as_str()),
            ))),
            None => None,
        }
    }

    fn add_library(&mut self, name: Ident, source: LibSource) -> LibId {
        let id = self.libs.next_id();
        let decl = self.decls.push(Decl {
            name: Designator::Ident(name),
            kind: DeclKind::Library(id),
            scope: self.root,
            span: name.span,
            implicit: false,
            since: LangRev::Vhdl1993,
        });
        self.libs.push(Library {
            name,
            decl,
            source,
            units: HashMap::default(),
        })
    }

    /// Where the unit's source is, from the slot or the library source.
    pub fn locate(&self, lib: LibId, key: UnitKey) -> Option<(PathBuf, LangRev)> {
        if let Some(UnitSlot::NotLoaded { file, rev }) = self.libs[lib].units.get(&key) {
            return Some((file.clone(), *rev));
        }
        self.locate_in(&self.libs[lib].source, key)
    }

    /// Where unit `key` of a library with this source is, by the source alone.
    pub(crate) fn locate_in(&self, source: &LibSource, key: UnitKey) -> Option<(PathBuf, LangRev)> {
        match source {
            LibSource::Files => None,
            LibSource::StdDir(dir) => {
                let file = match key {
                    UnitKey::Primary(k) => dir.join(format!("{k}.vhdl")),
                    UnitKey::Secondary(p, _) => dir.join(format!("{p}-body.vhdl")),
                };
                crate::stdlib::is_file(&file).then_some((file, self.rev))
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
                rec.and_then(|r| Some((r.file.to_path_buf(), parse_rev(r.std)?)))
            }
        }
    }

    /// Parse a file into the cache (all its units). Errors are reported once.
    pub(crate) fn parse_into_cache(
        &mut self,
        lib: LibId,
        path: &Path,
        rev: LangRev,
        at: Option<Span>,
    ) -> Result<(), Reported> {
        if self.frames.is_empty() {
            return self.parse_file(lib, path, rev, at);
        }
        self.file_step(lib, path, |db| db.parse_file(lib, path, rev, at))
    }

    fn parse_file(
        &mut self,
        lib: LibId,
        path: &Path,
        rev: LangRev,
        at: Option<Span>,
    ) -> Result<(), Reported> {
        let opts = yavs_syntax::Options {
            std: rev,
            defines: self.opts.defines.clone(),
            psl: self.opts.psl,
        };
        if let Some(fc) = self.files.get(&(lib, path.to_path_buf())) {
            if fc.parsed {
                return if fc.units.is_some() {
                    Ok(())
                } else {
                    Err(Reported)
                };
            }
            // Registered by the analysed-unit cache: its diagnostics were reported then.
            let file = fc.file;
            let units = match self.sources.get(file).try_text() {
                Ok(_) => yavs_syntax::parse_file(file, self.sources.get(file), &opts)
                    .design
                    .map(|d| {
                        d.units
                            .into_iter()
                            .map(|u| (UnitKey::of(&u.unit), Some(u)))
                            .collect()
                    }),
                Err(u) => {
                    let mut d = Diagnostic::error(
                        diag::SOURCE_UNREADABLE,
                        format!("cannot read source file `{}`", path.display()),
                    )
                    .with_note(u.note(&path.display().to_string()));
                    if let Some(at) = at {
                        d = d.with_primary(at, "needed here");
                    }
                    self.report(d);
                    None
                }
            };
            let fc = self
                .files
                .get_mut(&(lib, path.to_path_buf()))
                .expect("found above");
            fc.parsed = true;
            fc.units = units;
            return if fc.units.is_some() {
                Ok(())
            } else {
                Err(Reported)
            };
        }
        let before = self.diagnostics().len();
        let bytes = match crate::stdlib::read(path) {
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
                self.note_file(file);
                self.files.insert(
                    (lib, path.to_path_buf()),
                    FileCache {
                        file,
                        units: None,
                        parsed: true,
                        diags: Vec::new(),
                        hash: None,
                    },
                );
                return Err(self.error(d));
            }
        };
        let hash = self.units_on().then(|| {
            if matches!(self.libs[lib].source, LibSource::StdDir(_)) {
                format!("builtin:{}", path.display())
            } else {
                yavs_lib::hash_content(&bytes)
            }
        });
        let (file, bad) = yavs_syntax::load_source(
            &mut self.sources,
            path.display().to_string(),
            bytes,
            self.opts.encoding,
        );
        self.note_file(file);
        if let Some(d) = bad {
            self.files.insert(
                (lib, path.to_path_buf()),
                FileCache {
                    file,
                    units: None,
                    parsed: true,
                    diags: Vec::new(),
                    hash,
                },
            );
            return Err(self.error(d));
        }
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
        let diags = self.diagnostics()[before..].to_vec();
        self.files.insert(
            (lib, path.to_path_buf()),
            FileCache {
                file,
                units,
                parsed: true,
                diags,
                hash,
            },
        );
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
        let began = self.child_begin(lib, key);
        let r = self.load_unit_now(lib, key, at);
        self.child_end(began, lib, key);
        r
    }

    fn load_unit_now(
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
        // A package body is only needed at elaboration, so it may come later.
        if !matches!(key, UnitKey::Secondary(_, b) if b.as_str() == "body")
            && self.later_in_file.contains(&(lib, key))
        {
            return Err(self.declared_later(lib, key, at));
        }
        if self.units_on()
            && self.replaying == 0
            && !self.dup_in_file.contains(&(lib, key))
            && let Some(u) = self.try_replay(lib, key)
        {
            return Ok(u);
        }
        self.frame_begin(lib, key);
        let r = self.load_from_source(lib, key, at);
        self.frame_end(r.is_ok());
        r
    }

    fn load_from_source(
        &mut self,
        lib: LibId,
        key: UnitKey,
        at: Option<Span>,
    ) -> Result<UnitId, Reported> {
        let Some((path, rev)) = self.locate(lib, key) else {
            return Err(self.not_found(lib, key, at));
        };
        self.frame_located(&path, rev);
        if self.parse_into_cache(lib, &path, rev, at).is_err() {
            self.libs[lib].units.insert(key, UnitSlot::Failed);
            return Err(Reported);
        }
        let fc = self
            .files
            .get_mut(&(lib, path.clone()))
            .expect("parsed above");
        let file = fc.file;
        // A unit declared twice in its file: the file's own analysis takes the copies in
        // order; a later load takes the last, which replaced the others (13.5).
        let in_order = self.dup_in_file.contains(&(lib, key));
        let ast = fc
            .units
            .as_mut()
            .and_then(|us| {
                let mut it = us.iter_mut().filter(|(k, u)| *k == key && u.is_some());
                if in_order { it.next() } else { it.last() }
            })
            .and_then(|(_, u)| u.take());
        let Some(ast) = ast else {
            return Err(self.not_found(lib, key, at));
        };
        // sem.md 0.1: what the 2008 copy of a std/ieee package lacks gets `since: V2019`; under a
        // 2008 design a unit without a 1993 copy gets `since: V2008`.
        let std_primary = matches!(key, UnitKey::Primary(_))
            && matches!(self.libs[lib].source, LibSource::StdDir(_));
        let fresh = match self.rev {
            LangRev::Vhdl2019 if std_primary => Some((
                self.fresh_in_2019(lib, key, &path, &ast, file),
                LangRev::Vhdl2019,
            )),
            LangRev::Vhdl2008 if std_primary => {
                Some((self.fresh_in_2008(&path), LangRev::Vhdl2008))
            }
            _ => None,
        };
        let r = self.analyze_unit(lib, key, ast, file, rev);
        if let (Ok(u), Some((f, since))) = (&r, fresh) {
            self.mark_fresh(*u, f, since);
        }
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
            Ok(u) => {
                self.note_dep(u);
                if let Some(at) = at
                    && self.decls[self.units[u].decl].since > self.file_rev()
                {
                    let what = format!("{}.{}", self.libs[lib].name.text, self.span_text(at));
                    let since = self.decls[self.units[u].decl].since;
                    self.not_in_file_rev(what, at, since);
                }
            }
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
            LibSource::StdDir(d) => format!("looked in `{}`", crate::stdlib::display(d)),
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
        .with_note(looked);
        if let Some(at) = at {
            d = d.with_primary(at, "needed here");
        }
        let hint = match key {
            UnitKey::Primary(k) => {
                let names = self.unit_names(lib, None);
                crate::suggest::suggest(
                    k.as_str(),
                    "design unit",
                    names.iter().map(|n| (n.as_str(), n.as_str())),
                )
            }
            UnitKey::Secondary(..) => None,
        };
        d =
            d.with_help(hint.unwrap_or_else(|| {
                "analyze the file that defines it first, or check the name".into()
            }));
        self.error(d)
    }

    /// `E1002`: a unit needs a unit of its own file that comes after it (13.5, #810).
    fn declared_later(&mut self, lib: LibId, key: UnitKey, at: Option<Span>) -> Reported {
        let mut d = Diagnostic::error(
            diag::UNIT_NOT_FOUND,
            format!(
                "design unit {} of library `{}` is declared later in this file",
                key.describe(),
                self.libs[lib].name.text
            ),
        )
        .with_note("LRM 13.5: the units of a file are analyzed in their textual order, and a unit needs the units it depends on analyzed before it")
        .with_help("move it before the unit that needs it");
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
        self.analyze_file_hashed(work, path, rev, None)
    }

    /// [`Db::analyze_file`] of a file whose content hash the caller knows (`yavs_lib`
    /// `hash_content`), so that the analysed-unit cache need not read it to find its units.
    pub fn analyze_file_hashed(
        &mut self,
        work: &str,
        path: &Path,
        rev: LangRev,
        hash: Option<&str>,
    ) -> Result<Vec<UnitRecord>, Reported> {
        self.analyze_file_with(work, path, rev, hash, false)
    }

    /// [`Db::analyze_file_hashed`] for a caller that wants the records only (`yavs analyze`): a
    /// file whose units all have entries that hold and report nothing, and which the manifest
    /// records as they were, is registered but not loaded (`Db::warm_records`). The units load
    /// when a later file needs them.
    pub fn analyze_file_lazy(
        &mut self,
        work: &str,
        path: &Path,
        rev: LangRev,
        hash: Option<&str>,
    ) -> Result<Vec<UnitRecord>, Reported> {
        self.analyze_file_with(work, path, rev, hash, true)
    }

    fn analyze_file_with(
        &mut self,
        work: &str,
        path: &Path,
        rev: LangRev,
        hash: Option<&str>,
        lazy: bool,
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
        let keys = match self.warm_file(lib, path, rev, hash) {
            Some(keys) => keys,
            None => {
                self.parse_into_cache(lib, path, rev, None)?;
                self.files[&(lib, path.to_path_buf())]
                    .units
                    .as_ref()
                    .expect("parsed")
                    .iter()
                    .map(|(k, _)| *k)
                    .collect()
            }
        };
        for &k in &keys {
            self.libs[lib].units.insert(
                k,
                UnitSlot::NotLoaded {
                    file: path.to_path_buf(),
                    rev,
                },
            );
        }
        if lazy && let Some(records) = self.warm_records(lib, path, &keys) {
            return Ok(records);
        }
        let before = self.error_count();
        let mut ids = Vec::new();
        self.later_in_file = keys.iter().map(|&k| (lib, k)).collect();
        self.dup_in_file = (0..keys.len())
            .filter(|&i| keys[..i].contains(&keys[i]))
            .map(|i| (lib, keys[i]))
            .collect();
        for (i, &k) in keys.iter().enumerate() {
            self.later_in_file.retain(|&(_, x)| x != k);
            // A unit of the same name earlier in the file is replaced by this one (13.5).
            if keys[..i].contains(&k) {
                ids.retain(|&u| self.units[u].key != k);
                self.libs[lib].units.insert(
                    k,
                    UnitSlot::NotLoaded {
                        file: path.to_path_buf(),
                        rev,
                    },
                );
            }
            if let Ok(u) = self.load_unit(lib, k, None) {
                ids.push(u);
            }
        }
        self.later_in_file.clear();
        self.dup_in_file.clear();
        // A unit that failed, or needed one that failed, without a new error
        // depends on a unit that failed earlier in this session (8.2: silent);
        // the file still failed. A failed file is not recorded, so none of its
        // units is visible to later files, as with one `yavs analyze` per file.
        let distinct = (0..keys.len())
            .filter(|&i| !keys[..i].contains(&keys[i]))
            .count();
        if self.error_count() > before
            || ids.len() < distinct
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
            if self
                .analyze_file_hashed(&f.library, &f.path, rev, Some(&f.hash))
                .is_err()
            {
                r = Err(Reported);
            }
        }
        r
    }

    /// Whether `lib` has unit `key`, analyzed in this session or not (for the analysed-unit
    /// cache: the answer is part of the analysis of the unit that asks).
    pub(crate) fn unit_known(&self, lib: LibId, key: UnitKey) -> bool {
        let a = self.unit_known_now(lib, key);
        self.note_query(
            crate::units::Query::Known(self.libs[lib].name.key, key),
            crate::units::Answer::Bool(a),
        );
        a
    }

    /// Names for "did you mean": the primary units of `lib` or, with `entity`, the
    /// architectures of that entity, among the units the library knows now.
    pub(crate) fn unit_names(&self, lib: LibId, entity: Option<Symbol>) -> Vec<String> {
        let mut out: Vec<String> = self.libs[lib]
            .units
            .keys()
            .filter_map(|k| match (k, entity) {
                (UnitKey::Primary(p), None) => Some(p.to_string()),
                (UnitKey::Secondary(e, a), Some(want)) if *e == want && a.as_str() != "body" => {
                    Some(a.to_string())
                }
                _ => None,
            })
            .collect();
        match &self.libs[lib].source {
            LibSource::Manifest(m) => {
                out.extend(m.units().filter_map(|u| match (u.kind, entity) {
                    (
                        LibKind::Entity
                        | LibKind::Package
                        | LibKind::PackageInstance
                        | LibKind::Configuration
                        | LibKind::Context,
                        None,
                    ) => Some(u.key.to_string()),
                    (LibKind::Architecture, Some(e)) if u.primary == Some(e.as_str()) => {
                        u.key.rsplit('/').next().map(String::from)
                    }
                    _ => None,
                }))
            }
            LibSource::StdDir(dir) if entity.is_none() => {
                out.extend(crate::stdlib::unit_stems(dir))
            }
            _ => {}
        }
        out.sort();
        out.dedup();
        out
    }

    pub(crate) fn unit_known_now(&self, lib: LibId, key: UnitKey) -> bool {
        self.libs[lib].units.contains_key(&key) || self.locate(lib, key).is_some()
    }

    /// The units of `path` and an entry of the analysed-unit cache for each: when the manifest
    /// of `lib` records the file with this content and every unit of it has an entry made from
    /// it, the file is registered without being parsed (its units are read from their entries,
    /// or parsed after all when one does not hold).
    fn warm_file(
        &mut self,
        lib: LibId,
        path: &Path,
        rev: LangRev,
        hash: Option<&str>,
    ) -> Option<Vec<UnitKey>> {
        if !self.units_on() || self.files.contains_key(&(lib, path.to_path_buf())) {
            return None;
        }
        let LibSource::Manifest(m) = &self.libs[lib].source else {
            return None;
        };
        let fe = m.file(&std::path::absolute(path).ok()?)?;
        if fe.std != rev.as_str() || fe.units().next().is_none() || fe.partial {
            return None;
        }
        let hash = match hash {
            Some(h) => h.to_string(),
            None => yavs_lib::hash_content(&std::fs::read(path).ok()?),
        };
        if fe.hash != hash {
            return None;
        }
        let keys: Vec<UnitKey> = fe
            .units()
            .map(|u| match u.kind {
                LibKind::Architecture => {
                    let (e, a) = u.key.split_once('/')?;
                    Some(UnitKey::Secondary(Symbol::intern(e), Symbol::intern(a)))
                }
                LibKind::PackageBody => Some(UnitKey::body(Symbol::intern(u.key))),
                _ => Some(UnitKey::Primary(Symbol::intern(u.key))),
            })
            .collect::<Option<_>>()?;
        let stub = self.warm_stub(lib, &keys, path, &hash, rev)?;
        let file = self
            .sources
            .add_stub(stub, |p| crate::stdlib::read(Path::new(p)));
        self.note_file(file);
        self.files.insert(
            (lib, path.to_path_buf()),
            FileCache {
                file,
                units: None,
                parsed: false,
                diags: Vec::new(),
                hash: Some(hash),
            },
        );
        self.note_warm_file(path);
        Some(keys)
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
        let a = self.last_arch_now(lib, key);
        self.note_query(
            crate::units::Query::LastArch(self.libs[lib].name.key, key),
            crate::units::Answer::Name(a),
        );
        a
    }

    /// [`Db::last_architecture`] of entity `key` of `lib`, not recorded.
    pub(crate) fn last_arch_now(&self, lib: LibId, key: Symbol) -> Option<Symbol> {
        match &self.libs[lib].source {
            LibSource::Manifest(m) => m
                .units()
                .filter(|u| u.kind == LibKind::Architecture && u.primary == Some(key.as_str()))
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
