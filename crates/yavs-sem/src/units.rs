//! The analysed-unit cache (`docs/decisions.md` "Analysed-unit cache", #537, #539, #407).
//!
//! `yavs analyze` takes one file per call, and every call loaded the units the file depends on
//! from source again. Now each design unit analysed from source is written to the cache of its
//! library (`UnitStore`), and a later session that needs it reads it back instead.
//!
//! **What an entry is.** The analysis of one unit appends entries to the arenas of the `Db`,
//! changes a few tables, may write entries of other units (a package body completes the
//! subprograms of its package) and emits diagnostics. Units it needs are loaded in the middle
//! of that (on-demand loading nests). An entry records exactly that, in order: segments of the
//! unit's own arena entries and diagnostics, separated by the points where it loaded another
//! unit, registered a source file or created a library; then the writes to entries of other
//! units and the table changes, each with the value it found before. Reading the entry back
//! replays the same sequence; a unit loaded in between is loaded at the same point (from its
//! own entry), so the arenas end up exactly as an analysis from source leaves them, id for id.
//!
//! **Ids.** Ids index arenas shared by all units, so an entry writes each id as an owner (an
//! index into its table of the units it refers to: itself, a unit it loaded directly or through
//! another, a library) and the position among that owner's entries (`Reloc`). Every entry of
//! an arena knows its owner (`Arena::runs`), so this works whatever else the session loaded.
//! A unit that refers to anything else (an entry of a unit it did not load) is not stored.
//!
//! **Keys.** An entry is found by its identity (library, unit, path, design revision, options;
//! the store adds the build) and holds its *address* (the content hash and revision of its file)
//! and *full key* (the address, the full keys of the units it loaded, the libraries it made
//! and the answers it got about libraries). A reader computes the address of the unit it needs
//! and checks it; it compares the full key of every unit the entry loaded with the one that is
//! (or would be) loaded now. Anything that does not match is a miss: analysis from source.
//!
//! **Reading** first plans the whole replay (every unit it will load, where their entries will
//! land), so that a miss is found before anything changes. The few checks that need the
//! values of the session (an entry of another unit or a table key the unit changed must hold
//! what it held when the entry was made) run during the replay; a failed one undoes the
//! replay and the unit is analysed from source.

use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use rustc_hash::{FxHashMap, FxHashSet};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use yavs_diag::{Diagnostic, FileId, FileStub, Severity};
use yavs_syntax::Symbol;
use yavs_syntax::ast::Ident;

mod canon;

use crate::db::Db;
use crate::ids::*;
use crate::loader::{LibSource, UnitKey, UnitKind, UnitSlot};
use crate::tracked::{Touched, TrackedMap, TrackedSet};
use crate::{LangRev, UnitStore};

/// A sha256 digest.
pub type Hash = [u8; 32];

/// The layout of entries; part of every identity.
const FORMAT: u32 = 9;

/// Entries of `Db::new` (index 0 of these arenas): the same in every session.
const BASE: [u32; KINDS] = [0, 0, 1, 1, 1, 1, 1, 0, 0, 0, 0, 0, 0];

/// The owner number of base entries in an entry.
const BASE_OWNER: u32 = u32::MAX;

const ALL: [Kind; KINDS] = [
    Kind::Lib,
    Kind::Unit,
    Kind::Decl,
    Kind::Scope,
    Kind::Type,
    Kind::Subtype,
    Kind::Expr,
    Kind::Stmt,
    Kind::Const,
    Kind::Inst,
    Kind::View,
    Kind::External,
    Kind::AttrValue,
];

/// `$m!(field)` for the arena of kind `$k`.
macro_rules! by_kind {
    ($k:expr, $m:ident) => {
        match $k {
            Kind::Lib => $m!(libs),
            Kind::Unit => $m!(units),
            Kind::Decl => $m!(decls),
            Kind::Scope => $m!(scopes),
            Kind::Type => $m!(types),
            Kind::Subtype => $m!(subtypes),
            Kind::Expr => $m!(exprs),
            Kind::Stmt => $m!(stmts),
            Kind::Const => $m!(consts),
            Kind::Inst => $m!(insts),
            Kind::View => $m!(views),
            Kind::External => $m!(externals),
            Kind::AttrValue => $m!(attr_values),
        }
    };
}

/// One load of a unit in a session: analysed from source or read from the cache. Kept in the
/// `Db` (and its images), indexed by the owner number of the unit's arena entries.
#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct LoadRec {
    pub lib: LibId,
    pub key: UnitKey,
    pub unit: Option<UnitId>,
    /// `None`: the unit failed, or what it was made from is not known.
    pub full: Option<Hash>,
    /// The loads it asked for, in order, each once.
    pub deps: Vec<u32>,
    /// The files of its file steps, in order (registered by it or before).
    pub files: Vec<FileId>,
}

/// What a unit asked about the libraries: the answer is part of its full key and checked again
/// before the entry is used.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Query {
    /// Whether the library has the unit (in this session or in its manifest).
    Known(Symbol, UnitKey),
    /// Whether the library has an entity of that name.
    Entity(Symbol, Symbol),
    /// The most recently analysed architecture of an entity.
    LastArch(Symbol, Symbol),
    /// The session's `--relaxed`, asked only where it changes the analysis (`Db::relaxed`): an
    /// entry of a unit that never asked serves both settings.
    Relaxed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Answer {
    Bool(bool),
    Name(Option<Symbol>),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
enum Owner {
    Unit(Symbol, UnitKey),
    /// A library: its `LibId` (offset 0) and its declaration (offset 1).
    Lib(Symbol),
}

/// The plan part of an entry: what the replay does, in order, without the values.
#[derive(Serialize, Deserialize)]
enum PStep {
    /// Own entries per kind, diagnostics and TIME unit uses.
    Seg {
        sizes: [u32; KINDS],
        diags: u32,
        times: u32,
    },
    /// A file the unit reads: registered by the replay unless the session has it already.
    File {
        lib: Symbol,
        path: String,
        stub: FileStub,
        diags: u32,
    },
    /// A library the unit names, made by the replay unless it exists.
    Lib(Symbol),
    /// A unit the unit loads (`owners` index) and its full key.
    Load(u32, Hash),
}

#[derive(Serialize, Deserialize)]
struct Head {
    format: u32,
    address: Hash,
    full: Hash,
    kind: UnitKind,
    /// [`record_hash`] of the unit's manifest record: a file analysed again whose manifest says the
    /// same is not read at all (`Db::warm_records`).
    record: u128,
    owners: Vec<Owner>,
    steps: Vec<PStep>,
    queries: Vec<(Query, Answer)>,
}

impl Head {
    /// Reading the entry reports nothing: no diagnostic of the unit or of the files it registers.
    fn quiet(&self) -> bool {
        self.steps.iter().all(|s| {
            !matches!(
                s,
                PStep::Seg { diags: 1.., .. } | PStep::File { diags: 1.., .. }
            )
        })
    }
}

/// A hash of what the manifest records of a unit (its kind, names, standard and dependencies).
fn record_hash(r: &yavs_lib::UnitRecord) -> u128 {
    let b = postcard::to_stdvec(r)
        .unwrap_or_else(|e| panic!("internal error: cannot write a record: {e}"));
    twox_hash::XxHash3_128::oneshot(&b)
}

/// A step of a unit being analysed from source.
enum Step {
    Seg {
        start: [u32; KINDS],
        end: [u32; KINDS],
        diags: (usize, usize),
        times: (usize, usize),
    },
    File {
        lib: LibId,
        path: std::path::PathBuf,
        file: FileId,
    },
    /// A library the unit names (made by it or before), with the name as it wrote it.
    Lib(Ident, LibId),
    Load(u32),
}

/// The tables before a unit, to find what it changed: the position of each map's write log and the
/// bytes of each value table; with the audit also every map's content (`Options::unit_audit`).
struct Snap {
    tabs: Vec<TabSnap>,
    full: Option<Vec<TabSnap>>,
}

/// A table before a unit: the length of a map's write log (`tracked.rs`), a value table's bytes
/// or, for the audit, each key's hash with its value's (raw bytes, `hash_raw`).
enum TabSnap {
    Mark(usize),
    Map(FxHashMap<H, H>),
    Val(Vec<u8>),
}

/// A 128-bit hash of the raw bytes of a value (`hash_raw`): equal hashes are taken for equal
/// values.
pub(crate) type H = u128;

/// What a unit changed in a table: the key's hash (0 for a value table) and the value's hash
/// before and after; for a value table also its bytes before and after.
struct Change {
    t: u8,
    key: H,
    pre: Option<H>,
    post: Option<H>,
    val: Option<(Vec<u8>, Vec<u8>)>,
}

/// The hash of the raw bytes of `t`, written into `buf` (kept between calls).
pub(crate) fn hash_raw<T: Serialize + ?Sized>(t: &T, buf: &mut Vec<u8>) -> H {
    buf.clear();
    let b = std::mem::take(buf);
    let b = plain(|| yavs_syntax::raw_symbols(|| postcard::to_extend(t, b)))
        .unwrap_or_else(|e| panic!("internal error: cannot write a value: {e}"));
    let h = twox_hash::XxHash3_128::oneshot(&b);
    *buf = b;
    h
}

/// A unit being analysed from source (`Db::frames`).
pub(crate) struct Frame {
    serial: u32,
    lib: LibId,
    key: UnitKey,
    /// The unit's file and its revision, once located.
    path: Option<(std::path::PathBuf, LangRev)>,
    /// Write an entry at the end.
    capture: bool,
    /// Why no entry is written, if so.
    why_not: Option<String>,
    steps: Vec<Step>,
    seg_start: [u32; KINDS],
    seg_diags: usize,
    seg_times: usize,
    requested: FxHashSet<(LibId, UnitKey)>,
    queries: Vec<(Query, Answer)>,
    snap: Option<Snap>,
    /// Writes below the segment floor: (kind, index, bytes before), first per entry.
    touched: Vec<(Kind, u32, Vec<u8>)>,
    touched_set: FxHashSet<(Kind, u32)>,
    /// What units loaded inside this one (and changes it replayed) did to table keys, and the entries.
    nested_keys: FxHashMap<(u8, H), Nested>,
    nested_touched: FxHashSet<(Kind, u32)>,
    pending: [usize; 6],
}

/// What the units loaded inside a unit (and the changes it replayed) did to a table key: the hash of
/// the value before the first of them and after the last, and how many writes of the unit itself
/// came before the first (the length of the table's log then).
#[derive(Clone, Copy)]
struct Nested {
    pre: Option<H>,
    post: Option<H>,
    at: usize,
}

/// One change of a replay that `Db::rollback` undoes.
pub(crate) type Undo = Box<dyn FnOnce(&mut Db) + Send + Sync>;

/// `(first index, length, offset among the owner's entries)` of each run of a load's entries, per
/// kind.
pub(crate) type Runs = std::sync::Arc<[Vec<(u32, u32, u32)>; KINDS]>;

/// Where a replay started, to undo it.
pub(crate) struct Mark {
    lens: [u32; KINDS],
    sources: usize,
    file_owner: usize,
    loads: usize,
    diags: usize,
    times: usize,
    undo: usize,
}

// ---------------------------------------------------------------------------------------------
// Keys

fn sha(parts: &[&[u8]]) -> Hash {
    let mut h = Sha256::new();
    for p in parts {
        h.update((p.len() as u64).to_le_bytes());
        h.update(p);
    }
    h.finalize().into()
}

fn key_text(k: UnitKey) -> String {
    match k {
        UnitKey::Primary(p) => p.to_string(),
        UnitKey::Secondary(p, s) => format!("{p}/{s}"),
    }
}

/// The path as the key holds it: a relative path means another file from another directory.
fn path_text(p: &Path) -> String {
    if p.is_relative() && !p.starts_with(crate::stdlib::EMBEDDED) {
        let cwd = std::env::current_dir().unwrap_or_default();
        format!("{}\n{}", cwd.display(), p.display())
    } else {
        p.display().to_string()
    }
}

impl Db {
    /// Whether loads are tracked for the analysed-unit cache.
    pub(crate) fn units_on(&self) -> bool {
        self.opts.units.is_some()
    }

    fn store(&self) -> Option<std::sync::Arc<dyn UnitStore>> {
        self.opts.units.clone()
    }

    /// Whether units of `lib` have a place in the cache: std/ieee and the mapped libraries (a
    /// library directory without a manifest yet too: its first file).
    fn cacheable(&self, lib: LibId) -> bool {
        match self.libs[lib].source {
            LibSource::Manifest(_) | LibSource::StdDir(_) => true,
            LibSource::Files => self
                .opts
                .libraries
                .get(self.libs[lib].name.key.as_str())
                .is_some(),
        }
    }

    /// The identity of unit `key` of library `lib` read from `path`: what an entry is stored
    /// under. `relaxed` is the session's `--relaxed` for a unit whose analysis asked it
    /// (`Db::relaxed`), `None` for the others, whose one entry serves both settings.
    fn identity(&self, lib: Symbol, key: UnitKey, path: &Path, relaxed: Option<bool>) -> Hash {
        let mut defines = String::new();
        for (n, v) in &self.opts.defines {
            defines.push_str(&format!("{n}={v}\n"));
        }
        sha(&[
            b"unit",
            &FORMAT.to_le_bytes(),
            lib.as_str().as_bytes(),
            key_text(key).as_bytes(),
            path_text(path).as_bytes(),
            self.rev.as_str().as_bytes(),
            format!(
                "{relaxed:?} {} {:?} {}",
                self.opts.psl,
                self.opts.encoding,
                self.opts.stdlib.display()
            )
            .as_bytes(),
            defines.as_bytes(),
        ])
    }

    fn address(path: &Path, hash: &str, rev: LangRev) -> Hash {
        sha(&[
            b"address",
            path_text(path).as_bytes(),
            hash.as_bytes(),
            rev.as_str().as_bytes(),
        ])
    }

    /// The content hash of `path` of library `lib` as the session knows it, without parsing:
    /// the file cache, the manifest (trusted while the file keeps its size and time), the
    /// built-in std/ieee tree, else the file read now.
    fn content_hash(&self, lib: LibId, path: &Path) -> Option<String> {
        if let Some(fc) = self.files.get(&(lib, path.to_path_buf())) {
            return fc.hash.clone();
        }
        self.content_hash_in(&self.libs[lib].source, path)
    }

    fn content_hash_in(&self, source: &LibSource, path: &Path) -> Option<String> {
        match source {
            LibSource::StdDir(_) => Some(builtin_hash(path)),
            LibSource::Manifest(m) => match m.file(path) {
                // The size and time of a file are asked once per session, not once per unit that
                // depends on it (a call of a hundred files asks for the same hundred sources).
                Some(f) => {
                    if let Some(h) = self.entry_cache.hashes.lock().unwrap().get(path) {
                        return Some(h.clone());
                    }
                    if f.unchanged() {
                        let h = f.hash.to_string();
                        self.entry_cache
                            .hashes
                            .lock()
                            .unwrap()
                            .insert(path.to_path_buf(), h.clone());
                        return Some(h);
                    }
                    std::fs::read(path).ok().map(|b| yavs_lib::hash_content(&b))
                }
                None => std::fs::read(path).ok().map(|b| yavs_lib::hash_content(&b)),
            },
            LibSource::Files => None,
        }
    }

    // -----------------------------------------------------------------------------------------
    // Recording a unit analysed from source

    fn lens(&self) -> [u32; KINDS] {
        let mut out = [0u32; KINDS];
        for k in ALL {
            macro_rules! m {
                ($f:ident) => {
                    self.$f.len() as u32
                };
            }
            out[k as usize] = by_kind!(k, m);
        }
        out
    }

    fn set_owner(&mut self, owner: u32) {
        self.owner = owner;
        for k in ALL {
            macro_rules! m {
                ($f:ident) => {
                    self.$f.set_owner(owner)
                };
            }
            by_kind!(k, m);
        }
    }

    fn owner_now(&self) -> u32 {
        self.frames.last().map_or(SESSION, |f| f.serial)
    }

    fn watch_from(&mut self, lens: [u32; KINDS]) {
        for k in ALL {
            if k == Kind::Lib {
                continue;
            }
            macro_rules! m {
                ($f:ident) => {
                    self.$f.watch(lens[k as usize] as usize)
                };
            }
            by_kind!(k, m);
        }
    }

    /// End watching; the writes seen, in order.
    fn unwatch(&mut self) -> Vec<(Kind, u32, Vec<u8>)> {
        let mut out = Vec::new();
        for k in ALL {
            macro_rules! m {
                ($f:ident) => {
                    if let Some(w) = self.$f.unwatch() {
                        out.extend(w.touched.into_iter().map(|(i, b)| (k, i, b)));
                    }
                };
            }
            by_kind!(k, m);
        }
        out
    }

    fn pending_lens(&self) -> [usize; 6] {
        [
            self.pending_label_attrs.len(),
            self.pending_inst_checks.len(),
            self.pending_groups.len(),
            self.pending_implicit.len(),
            self.pending_specs.len(),
            self.stack.len(),
        ]
    }

    /// Close the open segment of the frame being recorded.
    fn seg_close(&mut self) {
        let lens = self.lens();
        let (diags, times) = (self.diags.len(), self.time_units.len());
        let touched = if self.frames.last().is_some_and(|f| f.capture) {
            self.unwatch()
        } else {
            Vec::new()
        };
        let log = std::mem::take(&mut *self.query_log.lock().unwrap_or_else(|e| e.into_inner()));
        let Some(f) = self.frames.last_mut() else {
            return;
        };
        for (q, a) in log {
            if !f.queries.iter().any(|(k, _)| *k == q) {
                f.queries.push((q, a));
            }
        }
        f.steps.push(Step::Seg {
            start: f.seg_start,
            end: lens,
            diags: (f.seg_diags, diags),
            times: (f.seg_times, times),
        });
        for (k, i, b) in touched {
            if f.touched_set.insert((k, i)) {
                f.touched.push((k, i, b));
            }
        }
    }

    /// Open the next segment of the frame being recorded.
    fn seg_open(&mut self) {
        let lens = self.lens();
        let (diags, times) = (self.diags.len(), self.time_units.len());
        let capture = match self.frames.last_mut() {
            Some(f) => {
                f.seg_start = lens;
                f.seg_diags = diags;
                f.seg_times = times;
                f.capture
            }
            None => return,
        };
        self.set_owner(self.owner_now());
        if capture {
            self.watch_from(lens);
        }
    }

    /// `load_unit` of `(lib, key)` begins inside the frame being recorded, if any: whether the
    /// frame's segment was closed for it (a unit asked for twice is a step once).
    pub(crate) fn child_begin(&mut self, lib: LibId, key: UnitKey) -> bool {
        if self.replaying > 0 {
            return false;
        }
        match self.frames.last_mut() {
            Some(f) => {
                if !f.requested.insert((lib, key)) {
                    return false;
                }
            }
            None => return false,
        }
        self.seg_close();
        true
    }

    /// The `load_unit` of [`Db::child_begin`] ended: the step names the load.
    pub(crate) fn child_end(&mut self, began: bool, lib: LibId, key: UnitKey) {
        if !began {
            return;
        }
        let serial = self.serial_of(lib, key);
        if let Some(f) = self.frames.last_mut() {
            match serial {
                Some(s) => f.steps.push(Step::Load(s)),
                None => {
                    f.why_not
                        .get_or_insert_with(|| format!("{} did not load", key_text(key)));
                }
            }
        }
        self.seg_open();
    }

    /// The load of a unit that is loaded now.
    fn serial_of(&self, lib: LibId, key: UnitKey) -> Option<u32> {
        let u = match self.libs[lib].units.get(&key) {
            Some(UnitSlot::Done(u)) => *u,
            _ => return None,
        };
        // Usually the last loads; a scan from the end finds them at once.
        self.loads
            .iter()
            .rposition(|r| r.unit == Some(u))
            .map(|i| i as u32)
    }

    /// A unit starts being analysed from source.
    pub(crate) fn frame_begin(&mut self, lib: LibId, key: UnitKey) {
        if !self.units_on() {
            return;
        }
        let serial = self.loads.len() as u32;
        self.loads.push(LoadRec {
            lib,
            key,
            unit: None,
            full: None,
            deps: Vec::new(),
            files: Vec::new(),
        });
        let cacheable = self.cacheable(lib);
        let capture = cacheable
            && self.store().is_some_and(|s| s.writable())
            && !self.dup_in_file.contains(&(lib, key));
        if capture {
            self.track(true);
        }
        let snap = capture.then(|| self.snapshot());
        let pending = self.pending_lens();
        self.frames.push(Frame {
            serial,
            lib,
            key,
            path: None,
            capture,
            why_not: None,
            steps: Vec::new(),
            seg_start: [0; KINDS],
            seg_diags: 0,
            seg_times: 0,
            requested: Default::default(),
            queries: Vec::new(),
            snap,
            touched: Vec::new(),
            touched_set: Default::default(),
            nested_keys: Default::default(),
            nested_touched: Default::default(),
            pending,
        });
        self.seg_open();
    }

    /// The unit being recorded was located at `path`.
    pub(crate) fn frame_located(&mut self, path: &Path, rev: LangRev) {
        if let Some(f) = self.frames.last_mut() {
            f.path = Some((path.to_path_buf(), rev));
        }
    }

    /// The unit being recorded is done (`ok`: analysed without errors).
    pub(crate) fn frame_end(&mut self, ok: bool) {
        if !self.units_on() || self.frames.is_empty() {
            return;
        }
        self.seg_close();
        let mut f = self.frames.pop().expect("checked");
        let parent = self.owner_now();
        self.set_owner(parent);
        let unit = match self.libs[f.lib].units.get(&f.key) {
            Some(UnitSlot::Done(u)) if ok => Some(*u),
            _ => None,
        };
        let deps: Vec<u32> = f
            .steps
            .iter()
            .filter_map(|s| match s {
                Step::Load(d) => Some(*d),
                _ => None,
            })
            .collect();
        let files: Vec<FileId> = f
            .steps
            .iter()
            .filter_map(|s| match s {
                Step::File { file, .. } => Some(*file),
                _ => None,
            })
            .collect();
        let address = unit.and_then(|_| {
            let (path, rev) = f.path.as_ref()?;
            let hash = self.content_hash(f.lib, path)?;
            Some(Self::address(path, &hash, *rev))
        });
        let full = address.and_then(|a| self.full_key(&f, a));
        {
            let r = &mut self.loads[f.serial as usize];
            r.unit = unit;
            r.full = full;
            r.deps = deps;
            r.files = files;
        }
        // What this unit changed itself (the units it loaded and the changes it replayed are in
        // `f.nested_keys`).
        let (changes, same) = match f.snap.as_ref().map(|s| self.diff(s, &f.nested_keys)) {
            Some((c, s)) => (Some(c), s),
            None => (None, Vec::new()),
        };
        if f.capture && unit.is_some() && (full.is_none() || address.is_none()) {
            self.note_miss(&f, "what it was made from is not known");
        }
        if let (Some(u), Some(full), Some(address), Some(changes)) = (unit, full, address, &changes)
        {
            if f.capture && f.why_not.is_none() {
                match self.encode(&f, u, address, full, changes) {
                    Ok((identity, bytes)) => {
                        if let Some(s) = self.store() {
                            s.put(self.libs[f.lib].name.key.as_str(), &identity, &bytes);
                            self.entry_cache.entries.lock().unwrap().remove(&identity);
                        }
                    }
                    Err(why) => self.note_miss(&f, &why),
                }
            } else if let Some(why) = &f.why_not {
                self.note_miss(&f, why);
            }
        }
        // The log holds the writes of the unit until its entry is made (a removed key is found
        // there); the frame around it is told them through `changes`.
        if let Some(s) = &f.snap {
            self.forget(s);
        }
        if self.frames.is_empty() {
            self.track(false);
        }
        // Where the unit sits among the writes of the frame around it.
        let at: Vec<usize> = tables().iter().map(|t| (t.loglen)(&*self)).collect();
        if let Some(p) = self.frames.last_mut() {
            if let Some(changes) = changes {
                // Everything below this unit, for the frame around it: its own changes after those
                // of the units it loaded. The larger map is kept and the smaller one added to it.
                let mut below = std::mem::take(&mut f.nested_keys);
                // A key the unit wrote and left as it was at its start is no change of it or of the
                // units loaded inside it; one it changed has the value before its first write.
                for k in same {
                    below.remove(&k);
                }
                for c in changes {
                    let n = Nested {
                        pre: c.pre,
                        post: c.post,
                        at: 0,
                    };
                    below.insert((c.t, c.key), n);
                }
                for ((t, _), n) in below.iter_mut() {
                    n.at = at[*t as usize];
                }
                // Older entries (siblings, replays) keep their value before and position, newer ones
                // their value after; a key back to where it was is dropped.
                let join = |older: Nested,
                            newer: Nested,
                            key: (u8, H),
                            into: &mut FxHashMap<(u8, H), Nested>| {
                    if older.pre == newer.post {
                        into.remove(&key);
                    } else {
                        into.insert(
                            key,
                            Nested {
                                post: newer.post,
                                ..older
                            },
                        );
                    }
                };
                if p.nested_keys.len() < below.len() {
                    std::mem::swap(&mut p.nested_keys, &mut below);
                    for (k, older) in below {
                        match p.nested_keys.get(&k).copied() {
                            Some(newer) => join(older, newer, k, &mut p.nested_keys),
                            None => {
                                p.nested_keys.insert(k, older);
                            }
                        }
                    }
                } else {
                    for (k, newer) in below {
                        match p.nested_keys.get(&k).copied() {
                            Some(older) => join(older, newer, k, &mut p.nested_keys),
                            None => {
                                p.nested_keys.insert(k, newer);
                            }
                        }
                    }
                }
            } else {
                // Not captured: the frame around it cannot tell its own changes apart.
                p.why_not
                    .get_or_insert_with(|| "a unit it loaded was not recorded".into());
            }
            p.nested_touched
                .extend(f.touched.iter().map(|t| (t.0, t.1)));
            p.nested_touched.extend(f.nested_touched.iter().copied());
        }
    }

    /// Say (debug log) that `path` was registered from the entries of its units, not parsed.
    pub(crate) fn note_warm_file(&self, path: &Path) {
        if let Some(s) = self.store() {
            s.note(&|| format!("file {} not parsed", path.display()));
        }
    }

    fn note_miss(&self, f: &Frame, why: &str) {
        if let Some(s) = self.store() {
            s.note(&|| {
                format!(
                    "unit {}.{} not stored: {why}",
                    self.libs[f.lib].name.key,
                    key_text(f.key)
                )
            });
        }
    }

    fn full_key(&self, f: &Frame, address: Hash) -> Option<Hash> {
        let mut parts: Vec<Vec<u8>> = vec![
            b"full".to_vec(),
            self.identity(self.libs[f.lib].name.key, f.key, &f.path.as_ref()?.0, None)
                .to_vec(),
            address.to_vec(),
        ];
        for s in &f.steps {
            match s {
                Step::Load(d) => parts.push(self.loads[*d as usize].full?.to_vec()),
                Step::Lib(_, l) => parts.push(self.libs[*l].name.key.as_str().as_bytes().to_vec()),
                Step::File { lib, path, .. } => {
                    // A file other than the unit's own (the 2008 copy of a std/ieee package)
                    // must be one the build holds.
                    if !matches!(self.libs[*lib].source, LibSource::StdDir(_))
                        && Some(path) != f.path.as_ref().map(|p| &p.0)
                    {
                        return None;
                    }
                    parts.push(path_text(path).into_bytes());
                }
                Step::Seg { .. } => {}
            }
        }
        parts.push(format!("{:?}", f.queries).into_bytes());
        let refs: Vec<&[u8]> = parts.iter().map(|p| &p[..]).collect();
        Some(sha(&refs))
    }

    /// The session's `--relaxed`, recorded as a question of the unit being analysed: ask it only
    /// where the answer changes the analysis, so other units' entries serve both settings.
    pub(crate) fn relaxed(&self) -> bool {
        self.note_query(Query::Relaxed, Answer::Bool(self.opts.relaxed));
        self.opts.relaxed
    }

    /// Record a question about the libraries asked by the unit being recorded.
    pub(crate) fn note_query(&self, q: Query, a: Answer) {
        if self.replaying == 0 && !self.frames.is_empty() {
            self.query_log
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .push((q, a));
        }
    }

    /// A file step of the unit being recorded: `f` registers the file (or finds it registered).
    pub(crate) fn file_step(
        &mut self,
        lib: LibId,
        path: &Path,
        f: impl FnOnce(&mut Db) -> Result<(), crate::Reported>,
    ) -> Result<(), crate::Reported> {
        let dup = self.replaying > 0
            || self.frames.last().is_none_or(|fr| {
                fr.steps.iter().any(
                    |s| matches!(s, Step::File { lib: l, path: p, .. } if *l == lib && p == path),
                )
            });
        if dup {
            return f(self);
        }
        self.seg_close();
        // The file belongs to the unit that registers it.
        let r = f(self);
        let file = self.files.get(&(lib, path.to_path_buf())).map(|fc| fc.file);
        if let Some(fr) = self.frames.last_mut() {
            match file {
                Some(file) => fr.steps.push(Step::File {
                    lib,
                    path: path.to_path_buf(),
                    file,
                }),
                None => {
                    fr.why_not.get_or_insert_with(|| "unreadable file".into());
                }
            }
        }
        self.seg_open();
        r
    }

    /// A unit being recorded names library `name`: `get` finds or makes it (owned by the
    /// session). A step of its own, once per unit: a replay makes the library when the session
    /// does not have it yet, as the analysis would.
    pub(crate) fn lib_step(&mut self, name: Ident, get: impl FnOnce(&mut Db) -> LibId) -> LibId {
        let dup = self.frames.last().is_none_or(|f| {
            f.steps
                .iter()
                .any(|s| matches!(s, Step::Lib(n, _) if n.key == name.key))
        });
        if dup || self.replaying > 0 {
            return get(self);
        }
        self.seg_close();
        self.set_owner(SESSION);
        let l = get(self);
        if let Some(f) = self.frames.last_mut() {
            f.steps.push(Step::Lib(name, l));
        }
        self.seg_open();
        l
    }

    /// The register of a new source file: which load it belongs to.
    pub(crate) fn note_file(&mut self, file: FileId) {
        let owner = self.owner;
        let i = file.0 as usize;
        if self.file_owner.len() <= i {
            self.file_owner.resize(i + 1, SESSION);
        }
        self.file_owner[i] = owner;
    }

    // -----------------------------------------------------------------------------------------
    // Tables

    /// Start or end logging the writes of the tables (`tracked.rs`); ending forgets the logs.
    fn track(&mut self, on: bool) {
        for t in tables() {
            (t.track)(self, on);
        }
    }

    /// Forget the writes the logs hold since `before`.
    fn forget(&mut self, before: &Snap) {
        for (t, tab) in tables().iter().zip(&before.tabs) {
            if let TabSnap::Mark(m) = tab {
                (t.forget)(self, *m);
            }
        }
    }

    fn snapshot(&self) -> Snap {
        Snap {
            tabs: tables().iter().map(|t| (t.mark)(self)).collect(),
            full: self
                .opts
                .unit_audit
                .then(|| tables().iter().map(|t| (t.snap)(self)).collect()),
        }
    }

    /// The table changes the unit made itself since `before`, in table order (and key hash order
    /// within a table), and the keys it wrote that are the same as at its start. The changes of a
    /// map are found from its log (a replayed change is not in it: it is in `nested`); the value of
    /// a key before the unit is the one before its first write unless a unit loaded inside wrote the
    /// key before that. The value tables are compared. With `Options::unit_audit` every table is
    /// compared too, and a change that comparison finds and is neither in the result nor in
    /// `nested` is an internal error (a write that bypassed the log).
    fn diff(
        &self,
        before: &Snap,
        nested: &FxHashMap<(u8, H), Nested>,
    ) -> (Vec<Change>, Vec<(u8, H)>) {
        let mut out = Vec::new();
        let mut same = Vec::new();
        for (i, t) in tables().iter().enumerate() {
            let t8 = i as u8;
            match &before.tabs[i] {
                TabSnap::Mark(m) => {
                    for (key, mut pre, post, first) in (t.touched)(self, *m) {
                        if let Some(n) = nested.get(&(t8, key))
                            && first >= n.at
                        {
                            pre = n.pre;
                        }
                        if pre == post {
                            same.push((t8, key));
                        } else {
                            out.push(Change {
                                t: t8,
                                key,
                                pre,
                                post,
                                val: None,
                            });
                        }
                    }
                }
                TabSnap::Val(b) => {
                    let TabSnap::Val(n) = (t.snap)(self) else {
                        unreachable!("a table keeps its shape")
                    };
                    if *b != n {
                        out.push(Change {
                            t: t8,
                            key: 0,
                            pre: Some(twox_hash::XxHash3_128::oneshot(b)),
                            post: Some(twox_hash::XxHash3_128::oneshot(&n)),
                            val: Some((b.clone(), n)),
                        });
                    }
                }
                TabSnap::Map(_) => unreachable!("a full snapshot is for the audit"),
            }
        }
        if let Some(full) = &before.full {
            self.audit(&out, &self.diff_full(full), nested);
        }
        (out, same)
    }

    /// Every change found by comparison (`found`) is a logged one with the same values before and
    /// after, or one of `nested` with the same value after; a logged change is found by comparison.
    fn audit(&self, logged: &[Change], found: &[Change], nested: &FxHashMap<(u8, H), Nested>) {
        let name = |c: &Change| tables()[c.t as usize].name;
        let mine: FxHashMap<(u8, H), &Change> = logged.iter().map(|c| ((c.t, c.key), c)).collect();
        for c in found {
            let ok = match mine.get(&(c.t, c.key)) {
                Some(l) => l.pre == c.pre && l.post == c.post && l.val == c.val,
                None => nested.get(&(c.t, c.key)).is_some_and(|n| n.post == c.post),
            };
            assert!(
                ok,
                "internal error: a unit changed table {} without logging the write (this is a bug in yavs)",
                name(c)
            );
        }
        let found: FxHashSet<(u8, H)> = found.iter().map(|c| (c.t, c.key)).collect();
        for c in logged {
            assert!(
                found.contains(&(c.t, c.key)),
                "internal error: a unit logged a change of table {} that comparison does not find (this is a bug in yavs)",
                name(c)
            );
        }
    }

    /// The table changes found by comparing every table with `before` (the audit of `diff`).
    fn diff_full(&self, before: &[TabSnap]) -> Vec<Change> {
        let mut out = Vec::new();
        for (i, t) in tables().iter().enumerate() {
            let t8 = i as u8;
            match (&before[i], (t.snap)(self)) {
                (TabSnap::Map(b), TabSnap::Map(n)) => {
                    let mut keys: Vec<H> = n.keys().chain(b.keys()).copied().collect();
                    keys.sort_unstable();
                    keys.dedup();
                    for k in keys {
                        let (pre, post) = (b.get(&k).copied(), n.get(&k).copied());
                        if pre != post {
                            out.push(Change {
                                t: t8,
                                key: k,
                                pre,
                                post,
                                val: None,
                            });
                        }
                    }
                }
                (TabSnap::Val(b), TabSnap::Val(n)) => {
                    if *b != n {
                        let mut buf = Vec::new();
                        out.push(Change {
                            t: t8,
                            key: 0,
                            pre: Some(twox_hash::XxHash3_128::oneshot(b)),
                            post: Some({
                                buf.extend_from_slice(&n);
                                twox_hash::XxHash3_128::oneshot(&buf)
                            }),
                            val: Some((b.clone(), n)),
                        });
                    }
                }
                _ => unreachable!("a table keeps its shape"),
            }
        }
        out
    }

    // -----------------------------------------------------------------------------------------
    // Writing an entry

    #[allow(clippy::type_complexity)]
    fn encode(
        &self,
        f: &Frame,
        unit: UnitId,
        address: Hash,
        full: Hash,
        changes: &[Change],
    ) -> Result<(Hash, Vec<u8>), String> {
        if f.pending != self.pending_lens() {
            return Err("analysis left work pending".into());
        }
        let (path, _) = f.path.as_ref().ok_or("no file")?;
        // The unit's own Unit entry is its first.
        let first_unit = self.units.runs_of(f.serial).first().map(|r| r.0);
        if first_unit != Some(unit.index() as u32) {
            return Err("the unit's entry is not its first".into());
        }
        let enc = Rc::new(Enc::new(self, f.serial));
        let reloc: Rc<dyn Reloc> = enc.clone();
        let map = enc.file_map();
        let (r, table) = yavs_syntax::encode_table(|| {
            let (head, body) = with_reloc(reloc, || {
                yavs_diag::map_file_ids(map, || self.encode_body(f, &enc, changes, address, full))
            });
            let mut head = head?;
            head.owners = enc.owners.list.borrow().clone();
            let head = postcard::to_stdvec(&head).map_err(|e| e.to_string())?;
            Ok::<_, String>((head, body?))
        });
        let (head, body) = r?;
        if let Some(why) = enc.failed.borrow().clone() {
            return Err(why);
        }
        let table = postcard::to_stdvec(&table).map_err(|e| e.to_string())?;
        let mut out = Vec::with_capacity(16 + table.len() + head.len() + body.len());
        out.extend_from_slice(&(table.len() as u64).to_le_bytes());
        out.extend_from_slice(&table);
        out.extend_from_slice(&(head.len() as u64).to_le_bytes());
        out.extend_from_slice(&head);
        out.extend_from_slice(&body);
        let relaxed = f
            .queries
            .iter()
            .any(|(q, _)| *q == Query::Relaxed)
            .then_some(self.opts.relaxed);
        Ok((
            self.identity(self.libs[f.lib].name.key, f.key, path, relaxed),
            out,
        ))
    }

    #[allow(clippy::type_complexity)]
    fn encode_body(
        &self,
        f: &Frame,
        enc: &Enc,
        changes: &[Change],
        address: Hash,
        full: Hash,
    ) -> (Result<Head, String>, Result<Vec<u8>, String>) {
        let mut steps = Vec::new();
        let mut body = Vec::new();
        let mut facts = crate::db::Facts::default();
        let r = (|| -> Result<(), String> {
            let put = |body: &mut Vec<u8>, v: &dyn erased::Ser| -> Result<(), String> {
                v.extend(body).map_err(|e| e.to_string())
            };
            for s in &f.steps {
                match s {
                    Step::Seg {
                        start,
                        end,
                        diags,
                        times,
                    } => {
                        let mut sizes = [0u32; KINDS];
                        for i in start[Kind::Expr as usize]..end[Kind::Expr as usize] {
                            self.note_expr(ExprId::from_index(i as usize), &mut facts);
                        }
                        for i in start[Kind::Stmt as usize]..end[Kind::Stmt as usize] {
                            self.note_stmt(StmtId::from_index(i as usize), &mut facts);
                        }
                        for k in ALL {
                            let (a, b) = (start[k as usize], end[k as usize]);
                            sizes[k as usize] = b - a;
                            if a == b {
                                continue;
                            }
                            if k == Kind::Lib {
                                return Err("library entries outside a library step".into());
                            }
                            // The length of each entry comes first, so that the reader can keep
                            // every entry as bytes until it is read (`Arena::push_run`).
                            let mut entries = Vec::new();
                            let mut lens = Vec::new();
                            macro_rules! m {
                                ($fl:ident) => {
                                    for i in a..b {
                                        let at = entries.len();
                                        put(&mut entries, self.$fl.get_at(i as usize))?;
                                        lens.push((entries.len() - at) as u32);
                                    }
                                };
                            }
                            by_kind!(k, m);
                            for l in lens {
                                put_varint(&mut body, l);
                            }
                            body.extend_from_slice(&entries);
                        }
                        for d in &self.diags[diags.0..diags.1] {
                            if d.severity == Severity::Error {
                                return Err("an error".into());
                            }
                            put(&mut body, d)?;
                        }
                        for t in &self.time_units[times.0..times.1] {
                            put(&mut body, t)?;
                        }
                        steps.push(PStep::Seg {
                            sizes,
                            diags: (diags.1 - diags.0) as u32,
                            times: (times.1 - times.0) as u32,
                        });
                    }
                    Step::File { lib, path, file } => {
                        let fc = self
                            .files
                            .get(&(*lib, path.clone()))
                            .ok_or("file cache cleared")?;
                        if fc.file != *file {
                            return Err("file registered twice".into());
                        }
                        for d in &fc.diags {
                            put(&mut body, d)?;
                        }
                        steps.push(PStep::File {
                            lib: self.libs[*lib].name.key,
                            path: path.display().to_string(),
                            stub: self.sources.get(*file).stub(),
                            diags: fc.diags.len() as u32,
                        });
                    }
                    Step::Lib(name, l) => {
                        put(&mut body, name)?;
                        steps.push(PStep::Lib(self.libs[*l].name.key));
                    }
                    Step::Load(d) => {
                        let r = &self.loads[*d as usize];
                        let o = enc.owner(Owner::Unit(self.libs[r.lib].name.key, r.key));
                        steps.push(PStep::Load(o, r.full.ok_or("a dependency without key")?));
                    }
                }
            }
            // Writes of entries of other units: (kind, id, before, after).
            let mut patches = Vec::new();
            for (k, i, before) in &f.touched {
                let owner = enc.owner_of(*k, *i);
                if owner == f.serial {
                    continue;
                }
                if *k == Kind::Lib {
                    continue;
                }
                if f.nested_touched.contains(&(*k, *i)) {
                    return Err("an entry written by the unit and by one it loaded".into());
                }
                let now = entry_raw(self, *k, *i as usize);
                if now == *before {
                    continue;
                }
                patches.push((*k, *i, before.clone()));
            }
            put(&mut body, &(patches.len() as u32))?;
            for (k, i, before) in patches {
                let pair = enc
                    .enc(k, i)
                    .ok_or("an entry written of a unit it did not load")?;
                put(&mut body, &(k as u8, pair))?;
                put(&mut body, &reenc_entry(self, k, &before)?)?;
                put(&mut body, &entry_rel(self, k, i as usize)?)?;
            }
            // Table changes not made by the units it loaded. A unit adds keys of its own and may
            // remove keys of a set (a package body, the generics its declaration made, whether the
            // declaration was loaded before the body or inside it: the removals are read from the
            // log, not from the net change); one that changes a key another unit made, or removes
            // one of a map, is not stored.
            let mut keys: FxHashMap<u8, FxHashSet<H>> = FxHashMap::default();
            let mut vals = Vec::new();
            for c in changes {
                let name = tables()[c.t as usize].name;
                if let Some(n) = f.nested_keys.get(&(c.t, c.key)) {
                    if n.post == c.post {
                        continue;
                    }
                    return Err(format!(
                        "table {name} key changed by the unit and by one it loaded"
                    ));
                }
                match &c.val {
                    Some(v) => vals.push((c.t, v)),
                    None if c.pre.is_some() && c.post.is_none() && tables()[c.t as usize].set => {}
                    None if c.pre.is_some() => {
                        return Err(format!("a key of table {name} that existed changed"));
                    }
                    None => {
                        keys.entry(c.t).or_default().insert(c.key);
                    }
                }
            }
            let mut n = keys.values().map(|k| k.len()).sum::<usize>() + vals.len();
            let mut tab = Vec::new();
            let mut ts: Vec<u8> = keys.keys().copied().collect();
            ts.sort_unstable();
            for t in ts {
                (tables()[t as usize].write)(self, t, Some(&keys[&t]), None, &mut tab)?;
            }
            for (t, d) in tables().iter().enumerate() {
                if let Some(TabSnap::Mark(m)) = f.snap.as_ref().map(|s| &s.tabs[t])
                    && d.set
                {
                    n += (d.write_removed)(self, t as u8, *m, &mut tab)?;
                }
            }
            for (t, v) in vals {
                (tables()[t as usize].write)(self, t, None, Some(v), &mut tab)?;
            }
            put(&mut body, &(n as u32))?;
            body.extend_from_slice(&tab);
            facts.settle_elem_cands();
            put(&mut body, &facts)?;
            Ok(())
        })();
        if let Err(e) = r {
            return (Err(e.clone()), Err(e));
        }
        let u = self.loads[f.serial as usize].unit.expect("done");
        let head = Head {
            format: FORMAT,
            address,
            full,
            kind: self.units[u].kind,
            record: record_hash(&self.unit_record(u)),
            owners: Vec::new(),
            steps,
            queries: f.queries.clone(),
        };
        (Ok(head), Ok(body))
    }
}

fn builtin_hash(path: &Path) -> String {
    format!("builtin:{}", path.display())
}

fn entry_raw(db: &Db, k: Kind, i: usize) -> Vec<u8> {
    macro_rules! m {
        ($f:ident) => {
            raw_bytes(db.$f.get_at(i))
        };
    }
    by_kind!(k, m)
}

/// Entry `i` of kind `k` written with the current `Reloc`.
fn entry_rel(db: &Db, k: Kind, i: usize) -> Result<Vec<u8>, String> {
    macro_rules! m {
        ($f:ident) => {
            postcard::to_stdvec(db.$f.get_at(i)).map_err(|e| e.to_string())
        };
    }
    by_kind!(k, m)
}

/// The raw bytes of an entry of kind `k` (this process) written again with the current `Reloc`.
fn reenc_entry(db: &Db, k: Kind, raw: &[u8]) -> Result<Vec<u8>, String> {
    macro_rules! m {
        ($f:ident) => {{
            let v = raw_like(&db.$f, raw)?;
            postcard::to_stdvec(&v).map_err(|e| e.to_string())
        }};
    }
    by_kind!(k, m)
}

fn raw_like<I, T: DeserializeOwned>(_: &Arena<I, T>, raw: &[u8]) -> Result<T, String> {
    raw_decode(raw)
}

fn rel_like<I, T: DeserializeOwned>(_: &Arena<I, T>, b: &[u8]) -> Result<T, String> {
    postcard::from_bytes(b).map_err(|e| e.to_string())
}

/// Decode bytes of [`raw_bytes`].
fn raw_decode<T: DeserializeOwned>(b: &[u8]) -> Result<T, String> {
    plain(|| yavs_syntax::raw_symbols(|| postcard::from_bytes(b))).map_err(|e| e.to_string())
}

/// `serde::Serialize` as an object, to write values of different types into one body.
mod erased {
    pub trait Ser {
        fn extend(&self, out: &mut Vec<u8>) -> postcard::Result<()>;
    }

    impl<T: serde::Serialize> Ser for T {
        fn extend(&self, out: &mut Vec<u8>) -> postcard::Result<()> {
            let v = postcard::to_extend(self, std::mem::take(out))?;
            *out = v;
            Ok(())
        }
    }
}

/// The owners an entry refers to, numbered in order of first use; [0] is the unit itself.
#[derive(Default)]
struct Owners {
    list: std::cell::RefCell<Vec<Owner>>,
    index: std::cell::RefCell<FxHashMap<Owner, u32>>,
}

impl Owners {
    fn get(&self, o: Owner) -> u32 {
        if let Some(&i) = self.index.borrow().get(&o) {
            return i;
        }
        let mut list = self.list.borrow_mut();
        let i = list.len() as u32;
        list.push(o.clone());
        self.index.borrow_mut().insert(o, i);
        i
    }
}

/// The `Reloc` of a unit being written: ids of its own entries, of the units it loaded (directly
/// or through others), of libraries and of `Db::new`. It holds copies of what it needs of the
/// `Db` (a `Reloc` lives in a thread-local while the entry is written).
struct Enc {
    /// Every arena's `(first index, owner)` runs and length.
    runs: [Vec<(u32, u32)>; KINDS],
    lens: [u32; KINDS],
    /// The owner of each allowed load.
    loads: FxHashMap<u32, Owner>,
    /// Library names by `LibId`, and the declarations of libraries.
    libs: Vec<Symbol>,
    lib_decls: FxHashMap<u32, usize>,
    owners: Rc<Owners>,
    /// Per load and kind: its owner number and its runs (`with_bases`), found once.
    #[allow(clippy::type_complexity)]
    by_owner: std::cell::RefCell<FxHashMap<(u32, Kind), Rc<(u32, Vec<(u32, u32, u32)>)>>>,
    /// Each file of an allowed load: the load's owner and the file's position among its steps.
    files: Rc<FxHashMap<FileId, (Owner, u32)>>,
    failed: Rc<std::cell::RefCell<Option<String>>>,
    /// Per kind the last run of one owner an id was found in: `(first index, end, owner number,
    /// offset of the first)`. Ids come in clusters, so most are in the run of the one before.
    hot: [std::cell::Cell<(u32, u32, u32, u32)>; KINDS],
}

/// `(first index, length, offset of the first among the owner's entries)` of each run.
fn with_bases(runs: Vec<(u32, u32)>) -> Vec<(u32, u32, u32)> {
    let mut base = 0;
    runs.into_iter()
        .map(|(s, l)| {
            let r = (s, l, base);
            base += l;
            r
        })
        .collect()
}

impl Enc {
    fn new(db: &Db, me: u32) -> Enc {
        let mut allowed = FxHashSet::default();
        let mut todo = vec![me];
        while let Some(s) = todo.pop() {
            if allowed.insert(s) {
                todo.extend(db.loads[s as usize].deps.iter().copied());
            }
        }
        let owner = |s: u32| {
            let r = &db.loads[s as usize];
            Owner::Unit(db.libs[r.lib].name.key, r.key)
        };
        let owners = Rc::new(Owners::default());
        owners.get(owner(me));
        let mut order: Vec<u32> = allowed.iter().copied().filter(|&s| s != me).collect();
        order.sort_unstable();
        order.insert(0, me);
        let mut files = FxHashMap::default();
        for &s in &order {
            for (k, &f) in db.loads[s as usize].files.iter().enumerate() {
                if k < 255 {
                    files.entry(f).or_insert_with(|| (owner(s), k as u32));
                }
            }
        }
        let runs = ALL.map(|k| {
            macro_rules! m {
                ($f:ident) => {
                    db.$f.runs_raw().to_vec()
                };
            }
            by_kind!(k, m)
        });
        let libs: Vec<Symbol> = db.libs.iter().map(|(_, l)| l.name.key).collect();
        let lib_decls = db
            .libs
            .iter()
            .map(|(i, l)| (l.decl.index() as u32, i.index()))
            .collect();
        Enc {
            runs,
            lens: db.lens(),
            loads: order.iter().map(|&s| (s, owner(s))).collect(),
            libs,
            lib_decls,
            owners,
            by_owner: Default::default(),
            files: Rc::new(files),
            failed: Default::default(),
            hot: std::array::from_fn(|_| std::cell::Cell::new((0, 0, 0, 0))),
        }
    }

    fn owner(&self, o: Owner) -> u32 {
        self.owners.get(o)
    }

    fn owner_of(&self, kind: Kind, i: u32) -> u32 {
        let runs = &self.runs[kind as usize];
        let k = runs.partition_point(|r| r.0 <= i);
        if k == 0 { SESSION } else { runs[k - 1].1 }
    }

    /// File ids as `owner << 8 | position`.
    fn file_map(&self) -> yavs_diag::FileIdMap {
        let (files, owners, failed) =
            (self.files.clone(), self.owners.clone(), self.failed.clone());
        let last = std::cell::Cell::new(None);
        Rc::new(move |id: FileId| {
            if let Some((from, to)) = last.get()
                && from == id
            {
                return to;
            }
            let to = match files.get(&id) {
                Some((o, k)) => {
                    let o = owners.get(o.clone());
                    (o < 1 << 24).then_some(FileId((o << 8) | k))
                }
                None => {
                    failed
                        .borrow_mut()
                        .get_or_insert_with(|| "a file of a unit it did not load".into());
                    None
                }
            };
            last.set(Some((id, to)));
            to
        })
    }

    fn fail(&self, why: String) -> Option<(u32, u32)> {
        self.failed.borrow_mut().get_or_insert(why);
        None
    }
}

impl Reloc for Enc {
    fn enc(&self, kind: Kind, index: u32) -> Option<(u32, u32)> {
        let hot = self.hot[kind as usize].get();
        if index.wrapping_sub(hot.0) < hot.1 - hot.0 {
            return Some((hot.2, hot.3 + index - hot.0));
        }
        let s = self.owner_of(kind, index);
        if s == SESSION {
            if index < BASE[kind as usize] {
                return Some((BASE_OWNER, index));
            }
            let lib = match kind {
                Kind::Lib => Some(index as usize),
                Kind::Decl => self.lib_decls.get(&index).copied(),
                _ => None,
            };
            return match lib.and_then(|l| self.libs.get(l)) {
                Some(&name) => Some((self.owner(Owner::Lib(name)), u32::from(kind == Kind::Decl))),
                None => self.fail(format!("a {kind:?} entry of the session")),
            };
        }
        let Some(o) = self.loads.get(&s) else {
            return self.fail(format!("a {kind:?} entry of a unit it did not load"));
        };
        let hit = self.by_owner.borrow().get(&(s, kind)).cloned();
        let found = hit.unwrap_or_else(|| {
            let o = self.owner(o.clone());
            let all = &self.runs[kind as usize];
            let len = self.lens[kind as usize];
            let mut mine = Vec::new();
            for (k, r) in all.iter().enumerate() {
                if r.1 == s {
                    let end = all.get(k + 1).map_or(len, |n| n.0);
                    if end > r.0 {
                        mine.push((r.0, end - r.0));
                    }
                }
            }
            let found = Rc::new((o, with_bases(mine)));
            self.by_owner.borrow_mut().insert((s, kind), found.clone());
            found
        });
        let (o, runs) = (found.0, &found.1);
        let k = runs.partition_point(|r| r.0 <= index);
        let (start, len, base) = *runs.get(k.checked_sub(1)?)?;
        if index >= start + len {
            return self.fail("an entry outside its owner's runs".into());
        }
        self.hot[kind as usize].set((start, start + len, o, base));
        Some((o, base + index - start))
    }

    fn dec(&self, _: Kind, _: u32, _: u32) -> Option<u32> {
        None
    }
}

// ---------------------------------------------------------------------------------------------
// Reading entries back

/// An entry read from the store and checked.
struct Entry {
    buf: yavs_diag::Buffer,
    syms: yavs_syntax::DecodeTable,
    head: Head,
    body: usize,
}

fn parse_entry(buf: yavs_diag::Buffer) -> Result<Entry, String> {
    let b = &buf[..];
    let n = |at: usize| -> Result<usize, String> {
        b.get(at..at + 8)
            .and_then(|x| <[u8; 8]>::try_from(x).ok())
            .map(|x| u64::from_le_bytes(x) as usize)
            .ok_or_else(|| "short entry".to_string())
    };
    let tn = n(0)?;
    let table: yavs_syntax::SymbolTable =
        postcard::from_bytes(b.get(8..8 + tn).ok_or("short table")?).map_err(|e| e.to_string())?;
    let hn = n(8 + tn)?;
    let at = 16 + tn;
    let syms = yavs_syntax::DecodeTable::new(table);
    let hb = b.get(at..at + hn).ok_or("short head")?;
    let head: Head = syms
        .run(|| postcard::from_bytes(hb))
        .map_err(|e| e.to_string())?;
    if head.format != FORMAT {
        return Err("format".into());
    }
    Ok(Entry {
        body: at + hn,
        buf,
        syms,
        head,
    })
}

/// What this session learnt from the store and the files that the next file of a call asks again
/// (#748): the files of a call that depend on the same hundred units would read, map and decode
/// each entry, and ask the size and time of each source, again for every file.
#[derive(Default)]
pub(crate) struct EntryCache {
    /// The entries read, by identity. An entry the session stores leaves the cache (the entry of
    /// that identity is another now), and it does not grow past [`EntryCache::MAX`] (each entry
    /// is a mapping of the process). `Db::release_analysis` drops it.
    entries: std::sync::Mutex<FxHashMap<Hash, Arc<Entry>>>,
    /// The content hash of each source the manifest knows and the file still has the size and
    /// time of.
    hashes: std::sync::Mutex<FxHashMap<std::path::PathBuf, String>>,
    /// The index of each library with a manifest, as read first.
    pub(crate) manifests: std::sync::Mutex<FxHashMap<String, Arc<yavs_lib::Manifest>>>,
}

impl EntryCache {
    const MAX: usize = 4096;
}

/// A unit the replay will load: where its entries go.
struct Planned {
    serial: u32,
    lib: Symbol,
    key: UnitKey,
    entry: Arc<Entry>,
    runs: Runs,
    files: Vec<FileId>,
}

/// The state of the session as the replay will leave it, worked out before it starts.
struct Plan {
    lens: [u32; KINDS],
    sources: u32,
    serials: u32,
    units: FxHashMap<(Symbol, UnitKey), Planned>,
    libs: FxHashMap<Symbol, (LibId, DeclId)>,
    files: FxHashMap<(Symbol, String), FileId>,
    entries: FxHashMap<(Symbol, UnitKey), Option<Arc<Entry>>>,
    /// The sources of the libraries the replay makes.
    srcs: FxHashMap<Symbol, LibSource>,
}

impl Db {
    fn lib_named(&self, name: Symbol) -> Option<LibId> {
        self.lib_by_key.get(&name).copied().flatten()
    }

    /// The entry of `(lib, key)` if it is one for the unit as it is now.
    fn entry_for(&self, lib: LibId, key: UnitKey) -> Result<Arc<Entry>, String> {
        let (path, rev) = self.locate(lib, key).ok_or("not found")?;
        let hash = self.content_hash(lib, &path).ok_or("no content hash")?;
        self.entry_at(lib, key, &path, &hash, rev)
    }

    /// Whether the unit has an entry, without reading it.
    fn entry_exists(&self, lib: LibId, key: UnitKey) -> bool {
        let Some((path, _)) = self.locate(lib, key) else {
            return false;
        };
        let name = self.libs[lib].name.key;
        self.store().is_some_and(|s| {
            [None, Some(self.opts.relaxed)]
                .into_iter()
                .any(|r| s.has(name.as_str(), &self.identity(name, key, &path, r)))
        })
    }

    fn entry_at(
        &self,
        lib: LibId,
        key: UnitKey,
        path: &Path,
        hash: &str,
        rev: LangRev,
    ) -> Result<Arc<Entry>, String> {
        self.entry_named(self.libs[lib].name.key, key, path, hash, rev)
    }

    fn entry_named(
        &self,
        name: Symbol,
        key: UnitKey,
        path: &Path,
        hash: &str,
        rev: LangRev,
    ) -> Result<Arc<Entry>, String> {
        let store = self.store().ok_or("no store")?;
        // The entry that serves both settings of `--relaxed`, else the one of this setting.
        let mut e = None;
        for relaxed in [None, Some(self.opts.relaxed)] {
            let identity = self.identity(name, key, path, relaxed);
            let cached = self
                .entry_cache
                .entries
                .lock()
                .unwrap()
                .get(&identity)
                .cloned();
            e = match cached {
                Some(e) => Some(e),
                None => match store.get(name.as_str(), &identity) {
                    Some(buf) => {
                        let e = Arc::new(parse_entry(buf)?);
                        let mut c = self.entry_cache.entries.lock().unwrap();
                        if c.len() < EntryCache::MAX {
                            c.insert(identity, e.clone());
                        }
                        Some(e)
                    }
                    None => None,
                },
            };
            if e.is_some() {
                break;
            }
        }
        let e = e.ok_or("no entry")?;
        if e.head.address != Self::address(path, hash, rev) {
            return Err("the source changed".into());
        }
        Ok(e)
    }

    /// The entry of unit `key` of library `lib`, which the replay makes.
    fn planned_entry(&self, p: &mut Plan, lib: Symbol, key: UnitKey) -> Option<Arc<Entry>> {
        if let std::collections::hash_map::Entry::Vacant(v) = p.srcs.entry(lib) {
            v.insert(self.lib_source(lib.as_str())?.ok()?);
        }
        let src = &p.srcs[&lib];
        if matches!(src, LibSource::Files) {
            return None;
        }
        let (path, rev) = self.locate_in(src, key)?;
        let hash = self.content_hash_in(src, &path)?;
        self.entry_named(lib, key, &path, &hash, rev).ok()
    }

    /// The stub of file `path` of `lib` (content hash `hash`) when every unit of `keys` has an
    /// entry made from it that reports nothing while reading it (`Db::warm_file`).
    pub(crate) fn warm_stub(
        &self,
        lib: LibId,
        keys: &[UnitKey],
        path: &Path,
        hash: &str,
        rev: LangRev,
    ) -> Option<FileStub> {
        let mut stub = None;
        let name = self.libs[lib].name.key;
        let text = path.display().to_string();
        for &k in keys {
            let e = self.entry_at(lib, k, path, hash, rev).ok()?;
            // The first segment of a unit is empty (nothing is made before its file is read).
            let first = e.head.steps.iter().find(|s| {
                !matches!(s, PStep::Seg { sizes, diags: 0, times: 0 } if sizes.iter().all(|&n| n == 0))
            });
            match first {
                Some(PStep::File {
                    lib: l,
                    path: p,
                    stub: s,
                    diags: 0,
                }) if *l == name && *p == text => {
                    stub.get_or_insert_with(|| s.clone());
                }
                _ => return None,
            }
        }
        stub
    }

    /// The manifest records of the units `keys` of file `path` (registered by `warm_file`) when
    /// reading them back would change and report nothing: every unit has an entry that holds,
    /// the entries of everything they load hold too, the units of the file have no diagnostic, and the
    /// manifest records the unit as it was when its entry was made. `yavs analyze` of such a
    /// file then loads none of it (a file that depends on 80 units no longer replays them to get
    /// back what the manifest says); a later file of the call loads a unit it needs the usual way.
    pub(crate) fn warm_records(
        &self,
        lib: LibId,
        path: &Path,
        keys: &[UnitKey],
    ) -> Option<Vec<yavs_lib::UnitRecord>> {
        let LibSource::Manifest(m) = &self.libs[lib].source else {
            return None;
        };
        let fe = m.file(&std::path::absolute(path).ok()?)?;
        // The record is only a hint here: a file that cannot be read back is analysed (and
        // recorded again) as for any file the cache does not know.
        let recs = m.records(&fe).ok()?;
        if recs.len() != keys.len() {
            return None;
        }
        let name = self.libs[lib].name.key;
        let mut p = Plan {
            lens: self.lens(),
            sources: self.sources.len() as u32,
            serials: self.loads.len() as u32,
            units: Default::default(),
            libs: Default::default(),
            files: Default::default(),
            entries: Default::default(),
            srcs: Default::default(),
        };
        for (&k, rec) in keys.iter().zip(&recs) {
            let e = self.entry_for(lib, k).ok()?;
            if e.head.record != record_hash(rec) {
                return None;
            }
            if p.units.contains_key(&(name, k)) {
                continue;
            }
            self.plan(&mut p, name, k, e).ok()?;
        }
        // Only the units of the file report (`yavs analyze` shows no warning of a unit it reads,
        // #584): what the units they load would print is not printed.
        if !keys.iter().all(|&k| {
            p.units
                .get(&(name, k))
                .is_some_and(|u| u.entry.head.quiet())
        }) {
            return None;
        }
        if let Some(s) = self.store() {
            s.note(&|| {
                format!(
                    "verified {name}.{} ({} units)",
                    key_text(keys[0]),
                    p.units.len()
                )
            });
        }
        Some(recs)
    }

    /// Plan the replay of `(lib, key)` from `entry` on top of `p`.
    fn plan(
        &self,
        p: &mut Plan,
        lib: Symbol,
        key: UnitKey,
        entry: Arc<Entry>,
    ) -> Result<(), String> {
        let serial = p.serials;
        p.serials += 1;
        let mut runs: [Vec<(u32, u32, u32)>; KINDS] = Default::default();
        let mut bases = [0u32; KINDS];
        let mut files = Vec::new();
        // Visible to its own Load steps (a cycle is no plan).
        p.units.insert(
            (lib, key),
            Planned {
                serial,
                lib,
                key,
                entry: entry.clone(),
                runs: Default::default(),
                files: Vec::new(),
            },
        );
        let head = &entry.head;
        for s in &head.steps {
            match s {
                PStep::Seg { sizes, .. } => {
                    for k in ALL {
                        let n = sizes[k as usize];
                        if n > 0 {
                            let i = k as usize;
                            runs[i].push((p.lens[i], n, bases[i]));
                            p.lens[i] += n;
                            bases[i] += n;
                        }
                    }
                }
                PStep::File { lib: l, path, .. } => {
                    let have = p.files.get(&(*l, path.clone())).copied().or_else(|| {
                        let lid = self.lib_named(*l)?;
                        self.files
                            .get(&(lid, std::path::PathBuf::from(path)))
                            .map(|fc| fc.file)
                    });
                    let f = match have {
                        Some(f) => f,
                        None => {
                            let f = FileId(p.sources);
                            p.sources += 1;
                            p.files.insert((*l, path.clone()), f);
                            f
                        }
                    };
                    files.push(f);
                }
                PStep::Lib(name) => {
                    if self.lib_named(*name).is_none() && !p.libs.contains_key(name) {
                        if self.lib_by_key.contains_key(name) {
                            return Err(format!("library {name} is unknown"));
                        }
                        let known = self.opts.libraries.get(name.as_str()).is_some()
                            || matches!(name.as_str(), "std" | "ieee");
                        if !known {
                            return Err(format!("library {name} is not mapped"));
                        }
                        let ids = (
                            LibId::from_index(p.lens[Kind::Lib as usize] as usize),
                            DeclId::from_index(p.lens[Kind::Decl as usize] as usize),
                        );
                        p.lens[Kind::Lib as usize] += 1;
                        p.lens[Kind::Decl as usize] += 1;
                        p.libs.insert(*name, ids);
                    }
                }
                PStep::Load(o, full) => {
                    let Some(Owner::Unit(dl, dk)) = head.owners.get(*o as usize) else {
                        return Err("bad owner".into());
                    };
                    let (dl, dk) = (*dl, *dk);
                    if let Some(pl) = p.units.get(&(dl, dk)) {
                        if pl.entry.head.full != *full {
                            return Err("a dependency changed".into());
                        }
                        continue;
                    }
                    let lid = self.lib_named(dl);
                    if let Some(lid) = lid {
                        match self.libs[lid].units.get(&dk) {
                            Some(UnitSlot::Done(_)) => {
                                let s = self.serial_of(lid, dk).ok_or("untracked dependency")?;
                                if self.loads[s as usize].full != Some(*full) {
                                    return Err(format!("{dl}.{} changed", key_text(dk)));
                                }
                                continue;
                            }
                            Some(UnitSlot::Failed | UnitSlot::InProgress(_)) => {
                                return Err("a dependency failed".into());
                            }
                            _ => {}
                        }
                    }
                    let e = match p.entries.get(&(dl, dk)) {
                        Some(e) => e.clone(),
                        None => {
                            let e = match lid {
                                Some(l) => self.entry_for(l, dk).ok(),
                                None => self.planned_entry(p, dl, dk),
                            };
                            p.entries.insert((dl, dk), e.clone());
                            e
                        }
                    };
                    let e = e.ok_or_else(|| format!("no entry for {}", key_text(dk)))?;
                    if e.head.full != *full {
                        return Err(format!("{dl}.{} has another entry", key_text(dk)));
                    }
                    self.plan(p, dl, dk, e)?;
                }
            }
        }
        for (q, a) in &head.queries {
            if self.answer(q, &mut *p) != *a {
                return Err(format!("{q:?} has another answer"));
            }
        }
        let pl = p.units.get_mut(&(lib, key)).expect("inserted above");
        pl.runs = std::sync::Arc::new(runs);
        pl.files = files;
        Ok(())
    }

    /// The answer to `q` in the session as `p` will leave it.
    fn answer(&self, q: &Query, p: &mut Plan) -> Answer {
        let l = match q {
            Query::Known(l, _) | Query::Entity(l, _) | Query::LastArch(l, _) => *l,
            Query::Relaxed => return Answer::Bool(self.opts.relaxed),
        };
        let lid = self.lib_named(l);
        // A library the replay makes: what its source says.
        if lid.is_none()
            && !p.srcs.contains_key(&l)
            && let Some(Ok(src)) = self.lib_source(l.as_str())
        {
            p.srcs.insert(l, src);
        }
        let src = p.srcs.get(&l);
        match q {
            Query::Known(_, k) => Answer::Bool(
                p.units.contains_key(&(l, *k))
                    || match lid {
                        Some(lid) => self.unit_known_now(lid, *k),
                        None => src.is_some_and(|s| self.locate_in(s, *k).is_some()),
                    },
            ),
            Query::Entity(_, k) => {
                let planned = p
                    .units
                    .get(&(l, UnitKey::Primary(*k)))
                    .map(|u| u.entry.head.kind == UnitKind::Entity);
                Answer::Bool(planned.unwrap_or_else(|| match lid {
                    Some(lid) => self.has_entity_now(lid, *k),
                    None => matches!(src, Some(LibSource::Manifest(m))
                        if m.find(yavs_lib::UnitKind::Entity, k.as_str()).is_some()),
                }))
            }
            Query::LastArch(_, k) => Answer::Name(match lid {
                Some(lid) => self.last_arch_now(lid, *k),
                None => match src {
                    Some(LibSource::Manifest(m)) => m
                        .units()
                        .filter(|u| {
                            u.kind == yavs_lib::UnitKind::Architecture
                                && u.primary == Some(k.as_str())
                        })
                        .last()
                        .and_then(|u| u.key.split_once('/').map(|(_, a)| Symbol::intern(a))),
                    _ => None,
                },
            }),
            Query::Relaxed => Answer::Bool(self.opts.relaxed),
        }
    }

    /// Read `(lib, key)` from the cache, if its entry holds and everything it loads can be
    /// loaded too. On a miss nothing changed.
    pub(crate) fn try_replay(&mut self, lib: LibId, key: UnitKey) -> Option<UnitId> {
        let store = self.store()?;
        if !self.cacheable(lib) {
            return None;
        }
        let name = self.libs[lib].name.key;
        let e = match self.entry_for(lib, key) {
            Ok(e) => e,
            Err(why) => {
                store.note(&|| format!("miss {name}.{}: {why}", key_text(key)));
                return None;
            }
        };
        let mut p = Plan {
            lens: self.lens(),
            sources: self.sources.len() as u32,
            serials: self.loads.len() as u32,
            units: Default::default(),
            libs: Default::default(),
            files: Default::default(),
            entries: Default::default(),
            srcs: Default::default(),
        };
        if let Err(why) = self.plan(&mut p, name, key, e) {
            store.note(&|| format!("miss {name}.{}: {why}", key_text(key)));
            return None;
        }
        let mark = self.mark();
        // Most names of the entries are new to this process.
        Symbol::reserve(p.units.values().map(|u| u.entry.syms.len()).sum::<usize>() / 2);
        let lens = self.lens();
        for k in ALL {
            let n = (p.lens[k as usize] - lens[k as usize]) as usize;
            macro_rules! m {
                ($f:ident) => {
                    self.$f.reserve(n)
                };
            }
            by_kind!(k, m);
        }
        match self.replay(&p, name, key) {
            Ok(u) => {
                self.undo.truncate(mark.undo);
                store.note(&|| {
                    format!(
                        "hit {name}.{} ({} units, {} bytes)",
                        key_text(key),
                        p.units.len(),
                        p.units.values().map(|u| u.entry.buf.len()).sum::<usize>()
                    )
                });
                Some(u)
            }
            Err(why) => {
                self.rollback(mark);
                store.note(&|| format!("undone {name}.{}: {why}", key_text(key)));
                None
            }
        }
    }

    fn mark(&self) -> Mark {
        Mark {
            lens: self.lens(),
            sources: self.sources.len(),
            file_owner: self.file_owner.len(),
            loads: self.loads.len(),
            diags: self.diags.len(),
            times: self.time_units.len(),
            undo: self.undo.len(),
        }
    }

    fn rollback(&mut self, m: Mark) {
        while self.undo.len() > m.undo {
            let u = self.undo.pop().expect("checked");
            u(self);
        }
        for k in ALL {
            let n = m.lens[k as usize] as usize;
            macro_rules! t {
                ($f:ident) => {
                    self.$f.truncate(n)
                };
            }
            by_kind!(k, t);
        }
        self.sources.truncate(m.sources);
        self.file_owner.truncate(m.file_owner);
        self.loads.truncate(m.loads);
        self.run_cache.retain(|&s, _| (s as usize) < m.loads);
        self.unit_facts.retain(|&s, _| (s as usize) < m.loads);
        self.diags.truncate(m.diags);
        self.time_units.truncate(m.times);
        self.set_owner(self.owner_now());
        self.bump_tracked();
    }

    fn bump_tracked(&mut self) {
        self.scopes.bump();
        self.decls.bump();
        self.subtypes.bump();
    }

    /// Replay planned unit `(lib, key)` and the units it loads.
    fn replay(&mut self, p: &Plan, lib: Symbol, key: UnitKey) -> Result<UnitId, String> {
        let pl = &p.units[&(lib, key)];
        let e = pl.entry.clone();
        let lid = self.lib_named(lib).ok_or("library missing")?;
        if self.loads.len() as u32 != pl.serial {
            return Err("plan out of step".into());
        }
        self.loads.push(LoadRec {
            lib: lid,
            key,
            unit: None,
            full: Some(e.head.full),
            deps: Vec::new(),
            files: Vec::new(),
        });
        let saved = self.owner;
        self.set_owner(pl.serial);
        let dec = match Dec::new(self, p, pl) {
            Ok(d) => std::sync::Arc::new(d),
            Err(e) => {
                self.set_owner(saved);
                return Err(e);
            }
        };
        let files = dec.files.clone();
        let shared: SharedFileMap = std::sync::Arc::new(move |id: FileId| {
            let (o, k) = (id.0 >> 8, (id.0 & 255) as usize);
            files.get(o as usize)?.as_ref()?.get(k).copied()
        });
        let ctx = PageCtx {
            syms: Some(e.syms.clone()),
            reloc: Some(dec.clone()),
            files: Some(shared),
        };
        self.replaying += 1;
        let r = ctx.run(|| self.replay_steps(p, pl, &e, &dec, &ctx));
        self.replaying -= 1;
        self.set_owner(saved);
        let u = r?;
        self.bump_tracked();
        Ok(u)
    }

    fn replay_steps(
        &mut self,
        p: &Plan,
        pl: &Planned,
        e: &Entry,
        dec: &Dec,
        ctx: &PageCtx,
    ) -> Result<UnitId, String> {
        let mut rest = &e.buf[e.body..];
        let serial = pl.serial as usize;
        let mut nfile = 0;
        let mut nrun = [0usize; KINDS];
        for s in &e.head.steps {
            match s {
                PStep::Seg {
                    sizes,
                    diags,
                    times,
                } => {
                    for k in ALL {
                        let n = sizes[k as usize];
                        if n > 0 {
                            let at = pl.runs[k as usize].get(nrun[k as usize]).map(|r| r.0);
                            nrun[k as usize] += 1;
                            macro_rules! len {
                                ($f:ident) => {
                                    self.$f.len() as u32
                                };
                            }
                            if at != Some(by_kind!(k, len)) {
                                return Err(format!("{k:?} entries out of step"));
                            }
                        }
                        if n == 0 {
                            continue;
                        }
                        // Every entry stays bytes until it is read.
                        let mut after = rest;
                        let mut total = 0usize;
                        for _ in 0..n {
                            total += take_varint(&mut after).ok_or("entry lengths")? as usize;
                        }
                        let lens = &rest[..rest.len() - after.len()];
                        if total > after.len() {
                            return Err("short entries".into());
                        }
                        let start = e.buf.len() - after.len();
                        let data = yavs_diag::Slice::of(&e.buf, start, start + total);
                        rest = &after[total..];
                        // Expressions and statements are read in runs (lowering takes a body whole);
                        // declarations, regions and types here and there.
                        let window = match k {
                            Kind::Expr | Kind::Stmt => PAGE,
                            _ => 1,
                        };
                        macro_rules! m {
                            ($f:ident) => {
                                self.$f
                                    .push_run(n as usize, lens, data, ctx.clone(), window)?
                            };
                        }
                        by_kind!(k, m);
                    }
                    for _ in 0..*diags {
                        let d: Diagnostic = take(&mut rest)?;
                        self.diags.push(d);
                    }
                    for _ in 0..*times {
                        let t = take(&mut rest)?;
                        self.time_units.push(t);
                    }
                }
                PStep::File {
                    lib,
                    path,
                    stub,
                    diags,
                } => {
                    let f = pl.files[nfile];
                    nfile += 1;
                    let mut ds = Vec::new();
                    for _ in 0..*diags {
                        ds.push(take::<Diagnostic>(&mut rest)?);
                    }
                    if (f.0 as usize) == self.sources.len() {
                        let lid = self.lib_named(*lib).ok_or("library missing")?;
                        let id = self
                            .sources
                            .add_stub(stub.clone(), |p| crate::stdlib::read(Path::new(p)));
                        self.note_file(id);
                        let k = (lid, std::path::PathBuf::from(path));
                        let hash = if matches!(self.libs[lid].source, LibSource::StdDir(_)) {
                            Some(builtin_hash(&k.1))
                        } else {
                            self.content_hash(lid, &k.1)
                        };
                        self.files.insert(
                            k.clone(),
                            crate::loader::FileCache {
                                file: id,
                                units: None,
                                parsed: false,
                                diags: ds.clone(),
                                hash,
                            },
                        );
                        self.undo.push(Box::new(move |db: &mut Db| {
                            db.files.remove(&k);
                        }));
                        self.diags.extend(ds);
                    }
                    self.loads[serial].files.push(f);
                }
                PStep::Lib(name) => {
                    let ident: Ident = take(&mut rest)?;
                    if self.lib_named(*name).is_none() {
                        self.set_owner(SESSION);
                        let r = self.library(ident);
                        self.set_owner(pl.serial);
                        let l = r.map_err(|_| "library failed")?;
                        let key = *name;
                        self.undo.push(Box::new(move |db: &mut Db| {
                            db.lib_by_key.remove(&key);
                        }));
                        if Some(&(l, self.libs[l].decl)) != p.libs.get(name) {
                            return Err("library out of step".into());
                        }
                    }
                }
                PStep::Load(o, _) => {
                    let Some(Owner::Unit(dl, dk)) = e.head.owners.get(*o as usize) else {
                        return Err("bad owner".into());
                    };
                    let lid = self.lib_named(*dl).ok_or("library missing")?;
                    let d = match self.libs[lid].units.get(dk) {
                        Some(UnitSlot::Done(_)) => {
                            self.serial_of(lid, *dk).ok_or("untracked dependency")?
                        }
                        _ => {
                            let s = self.loads.len() as u32;
                            self.replay(p, *dl, *dk)?;
                            s
                        }
                    };
                    self.loads[serial].deps.push(d);
                }
            }
        }
        // Writes of entries of other units.
        let n: u32 = take(&mut rest)?;
        for _ in 0..n {
            let (k, (o, off)): (u8, (u32, u32)) = take(&mut rest)?;
            let k = *ALL.get(k as usize).ok_or("bad kind")?;
            let i = dec.dec(k, o, off).ok_or("bad target")? as usize;
            let before: Vec<u8> = take(&mut rest)?;
            let after: Vec<u8> = take(&mut rest)?;
            macro_rules! m {
                ($f:ident) => {{
                    let want = rel_like(&self.$f, &before)?;
                    if !same_content(self.$f.get_at(i), &want)? {
                        return Err(format!("{k:?} entry changed since"));
                    }
                    let v = rel_like(&self.$f, &after)?;
                    let old = self.$f.replace(i, v);
                    self.undo.push(Box::new(move |db: &mut Db| {
                        db.$f.replace(i, old);
                    }));
                    if let Some(f) = self.frames.last_mut() {
                        f.nested_touched.insert((k, i as u32));
                    }
                }};
            }
            by_kind!(k, m);
        }
        // Table changes.
        let n: u32 = take(&mut rest)?;
        let raw = self.frames.last().is_some_and(|f| f.capture);
        for _ in 0..n {
            let t: u8 = take(&mut rest)?;
            let d = tables().get(t as usize).ok_or("bad table")?;
            let (undo, changed) = (d.replay)(self, &mut rest, raw)?;
            self.undo.push(undo);
            if let Some((key, pre, post)) = changed {
                let at = (d.loglen)(self);
                if let Some(f) = self.frames.last_mut() {
                    f.nested_keys.insert((t, key), Nested { pre, post, at });
                }
            }
        }
        // The facts of its entries (`Db::scan_facts`).
        let facts: crate::db::Facts = take(&mut rest)?;
        self.unit_facts.insert(pl.serial, facts);
        let u = UnitId::from_index(
            pl.runs[Kind::Unit as usize]
                .first()
                .ok_or("no unit entry")?
                .0 as usize,
        );
        let lid = self.lib_named(pl.lib).ok_or("library missing")?;
        if self.units[u].key != pl.key {
            return Err("unit entry out of step".into());
        }
        let old = self.libs[lid].units.insert(pl.key, UnitSlot::Done(u));
        let k = pl.key;
        self.undo.push(Box::new(move |db: &mut Db| match old {
            Some(o) => {
                db.libs[lid].units.insert(k, o);
            }
            None => {
                db.libs[lid].units.remove(&k);
            }
        }));
        self.loads[serial].unit = Some(u);
        self.run_cache.insert(pl.serial, pl.runs.clone());
        Ok(u)
    }
}

/// The `Reloc` of a unit being read: its owners' entries where the plan puts them.
struct Dec {
    owners: Vec<OwnerAt>,
    files: std::sync::Arc<Vec<Option<Vec<FileId>>>>,
}

enum OwnerAt {
    Unit(Runs),
    Lib(LibId, DeclId),
}

impl Db {
    /// The runs of finished load `s`.
    fn runs_of_load(&mut self, s: u32) -> Runs {
        if let Some(r) = self.run_cache.get(&s) {
            return r.clone();
        }
        let r: Runs = std::sync::Arc::new(ALL.map(|kind| {
            macro_rules! m {
                ($f:ident) => {
                    self.$f.runs_of(s)
                };
            }
            with_bases(by_kind!(kind, m))
        }));
        self.run_cache.insert(s, r.clone());
        r
    }
}

impl Dec {
    fn new(db: &mut Db, p: &Plan, pl: &Planned) -> Result<Dec, String> {
        let mut owners = Vec::new();
        let mut files = Vec::new();
        for o in &pl.entry.head.owners {
            match o {
                Owner::Lib(n) => {
                    let ids = match db.lib_named(*n) {
                        Some(l) => (l, db.libs[l].decl),
                        None => *p.libs.get(n).ok_or("library missing")?,
                    };
                    owners.push(OwnerAt::Lib(ids.0, ids.1));
                    files.push(None);
                }
                Owner::Unit(l, k) => {
                    if let Some(u) = p.units.get(&(*l, *k)) {
                        owners.push(OwnerAt::Unit(u.runs.clone()));
                        files.push(Some(u.files.clone()));
                        continue;
                    }
                    let lid = db.lib_named(*l).ok_or("library missing")?;
                    let s = db.serial_of(lid, *k).ok_or("owner not loaded")?;
                    owners.push(OwnerAt::Unit(db.runs_of_load(s)));
                    files.push(Some(db.loads[s as usize].files.clone()));
                }
            }
        }
        Ok(Dec {
            owners,
            files: std::sync::Arc::new(files),
        })
    }
}

impl Reloc for Dec {
    fn enc(&self, _: Kind, _: u32) -> Option<(u32, u32)> {
        None
    }

    fn dec(&self, kind: Kind, owner: u32, offset: u32) -> Option<u32> {
        if owner == BASE_OWNER {
            return (offset < BASE[kind as usize]).then_some(offset);
        }
        match self.owners.get(owner as usize)? {
            OwnerAt::Lib(l, d) => match (kind, offset) {
                (Kind::Lib, 0) => Some(l.index() as u32),
                (Kind::Decl, 1) => Some(d.index() as u32),
                _ => None,
            },
            OwnerAt::Unit(runs) => {
                let runs = &runs[kind as usize];
                if let [(start, len, _)] = runs[..] {
                    return (offset < len).then_some(start + offset);
                }
                let k = runs.partition_point(|r| r.2 <= offset);
                let (start, len, base) = *runs.get(k.checked_sub(1)?)?;
                (offset < base + len).then(|| start + offset - base)
            }
        }
    }
}

impl Db {
    /// The bodies of the `std`/`ieee` packages this session loaded that the cache has no entry
    /// for, as (library, package). `yavs analyze` never needs them, but every `elab` and `run`
    /// does: the CLI analyses them once in a session of their own so that they are stored.
    pub fn builtin_bodies_missing(&self) -> Vec<(String, String)> {
        if self.store().is_none_or(|s| !s.writable()) {
            return Vec::new();
        }
        let mut out = Vec::new();
        for (lid, lib) in self.libs.iter() {
            if !matches!(lib.source, LibSource::StdDir(_)) {
                continue;
            }
            for (k, slot) in &lib.units {
                let (UnitKey::Primary(p), UnitSlot::Done(u)) = (k, slot) else {
                    continue;
                };
                let body = UnitKey::body(*p);
                if self.units[*u].kind != UnitKind::Package
                    || lib.units.contains_key(&body)
                    || self.locate(lid, body).is_none()
                    || self.entry_exists(lid, body)
                {
                    continue;
                }
                out.push((lib.name.key.to_string(), p.to_string()));
            }
        }
        out.sort();
        out
    }

    /// Why this analysis differs from `other` (for tests and `YAVS_UNIT_CHECK`): every arena entry,
    /// table, source file, diagnostic and library unit by content. What only says how the
    /// session was built (the loads of the cache, the owners of entries) is not compared.
    #[doc(hidden)]
    pub fn differs_from(&self, other: &Db) -> Option<String> {
        let c = |v: Result<canon::V, String>| v.unwrap_or_else(|e| panic!("internal error: {e}"));
        for k in ALL {
            macro_rules! m {
                ($f:ident) => {{
                    if self.$f.len() != other.$f.len() {
                        return Some(format!(
                            "{k:?}: {} entries, {} expected",
                            self.$f.len(),
                            other.$f.len()
                        ));
                    }
                    for i in 0..self.$f.len() {
                        let (a, b) = (
                            c(canon::canon(self.$f.get_at(i))),
                            c(canon::canon(other.$f.get_at(i))),
                        );
                        if a != b {
                            return Some(format!(
                                "{k:?} entry {i} differs at{}",
                                canon::first_diff(&a, &b)
                            ));
                        }
                    }
                }};
            }
            by_kind!(k, m);
        }
        for (i, t) in tables().iter().enumerate() {
            let (a, b) = ((t.canon)(self), (t.canon)(other));
            if a != b {
                let detail = match (a, b) {
                    (Ok(a), Ok(b)) => canon::first_diff(&a, &b),
                    (a, b) => format!("{a:?} {b:?}"),
                };
                if std::env::var_os("YAVS_UNIT_DEBUG").is_some() {
                    for k in ALL {
                        macro_rules! m {
                            ($f:ident) => {
                                eprintln!("{k:?} runs {:?}", self.$f.runs_raw())
                            };
                        }
                        by_kind!(k, m);
                    }
                    for (i, l) in self.loads.iter().enumerate() {
                        eprintln!(
                            "load {i}: {} {:?} unit {:?} deps {:?}",
                            self.libs[l.lib].name.key,
                            key_text(l.key),
                            l.unit,
                            l.deps
                        );
                    }
                }
                return Some(format!("table {} (#{i}) differs:{detail}", t.name));
            }
        }
        if self.sources.len() != other.sources.len() {
            return Some("source files".into());
        }
        for i in 0..self.sources.len() {
            let f = FileId(i as u32);
            if self.sources.get(f).stub() != other.sources.get(f).stub() {
                return Some(format!("source file {i}"));
            }
        }
        if c(canon::canon(&self.diags)) != c(canon::canon(&other.diags)) {
            return Some("diagnostics".into());
        }
        if c(canon::canon(&self.time_units)) != c(canon::canon(&other.time_units)) {
            return Some("time units".into());
        }
        if c(canon::canon(&self.lib_by_key)) != c(canon::canon(&other.lib_by_key)) {
            return Some("libraries".into());
        }
        let (a, b) = (self.facts(), other.facts());
        if (a.reads_activity, &a.forces, &a.package_paths, a.history_all)
            != (b.reads_activity, &b.forces, &b.package_paths, b.history_all)
            || (&a.history_parts, &a.history_wholes) != (&b.history_parts, &b.history_wholes)
            || (&a.elem_reads, a.elem_reads_all) != (&b.elem_reads, b.elem_reads_all)
        {
            return Some("facts".into());
        }
        None
    }
}

// ---------------------------------------------------------------------------------------------
// The tables a unit may change, besides the arenas

/// What a replayed table change leaves for the frame around it: the hashes of the key and of the
/// value before and after.
type Changed = Option<(H, Option<H>, Option<H>)>;

/// One table of `Db`: how to compare it, write a change and replay it.
struct Desc {
    name: &'static str,
    /// The whole table (for the audit and value tables).
    snap: fn(&Db) -> TabSnap,
    /// What `diff` needs to find the changes: a map's log position, a value table's bytes.
    mark: fn(&Db) -> TabSnap,
    /// Log the writes of a map (or stop and forget).
    track: fn(&mut Db, bool),
    /// Forget the writes of a map since a log position.
    forget: fn(&mut Db, usize),
    /// The length of the log of a map (0 for a value table).
    loglen: fn(&Db) -> usize,
    /// The keys of a map written since a log position.
    touched: fn(&Db, usize) -> Vec<Touched>,
    /// Write changes under the current `Reloc`, each as the table number, the key, the value
    /// before and the value after: of a map, the keys with these hashes (added by the unit); of a
    /// value table, the raw bytes before and after.
    #[allow(clippy::type_complexity)]
    write: fn(
        &Db,
        u8,
        Option<&FxHashSet<H>>,
        Option<&(Vec<u8>, Vec<u8>)>,
        &mut Vec<u8>,
    ) -> Result<(), String>,
    /// Whether the table is a set: its removals are written by `write_removed`.
    set: bool,
    /// Write the removal of every key of a set that was there at a log position and is gone
    /// now, as `write` writes a change; how many.
    write_removed: fn(&Db, u8, usize, &mut Vec<u8>) -> Result<usize, String>,
    /// Read a change written by `write` and make it, if the table holds the value before; the
    /// undo, and the raw key and value set when asked (`raw`).
    #[allow(clippy::type_complexity)]
    replay: fn(&mut Db, &mut &[u8], bool) -> Result<(Undo, Changed), String>,
    /// The table by content.
    canon: fn(&Db) -> Result<canon::V, String>,
}

fn take<T: DeserializeOwned>(rest: &mut &[u8]) -> Result<T, String> {
    let (v, r) = postcard::take_from_bytes(rest).map_err(|e| e.to_string())?;
    *rest = r;
    Ok(v)
}

fn put<T: Serialize>(v: &T, out: &mut Vec<u8>) -> Result<(), String> {
    erased::Ser::extend(v, out).map_err(|e| e.to_string())
}

/// Whether `now` is `pre` by content.
fn same<V: Serialize>(now: Option<&V>, pre: Option<&V>) -> Result<bool, String> {
    Ok(match (now, pre) {
        (None, None) => true,
        (Some(a), Some(b)) => same_content(a, b)?,
        _ => false,
    })
}

/// Whether `a` and `b` have the same content: equal bytes say so at once, else the canonical
/// forms decide (they sort the keys of maps, whose order differs between equal maps).
fn same_content<T: Serialize>(a: &T, b: &T) -> Result<bool, String> {
    Ok(raw_bytes(a) == raw_bytes(b) || canon::canon(a)? == canon::canon(b)?)
}

trait MapLike: 'static {
    type K: Serialize + DeserializeOwned + Eq + std::hash::Hash + Clone + Send + Sync + 'static;
    type V: Serialize + DeserializeOwned + Send + Sync + 'static;
    fn entries(&self) -> Vec<(&Self::K, &Self::V)>;
    fn get_v(&self, k: &Self::K) -> Option<&Self::V>;
    /// A write of a replay, not logged when `quiet` (the frame records it another way).
    fn put(&mut self, k: Self::K, v: Self::V, quiet: bool) -> Option<Self::V>;
    fn del(&mut self, k: &Self::K, quiet: bool) -> Option<Self::V>;
    /// The keys of the table at log position `mark` that are gone now (sets only: a map's
    /// removal would have to record the value it removed), and the value of a set's key.
    fn removed_since(&self, mark: usize) -> Vec<Self::K>;
    fn removed_value() -> Option<Self::V>;
}

impl<K, V> MapLike for TrackedMap<K, V>
where
    K: Serialize + DeserializeOwned + Eq + std::hash::Hash + Clone + Send + Sync + 'static,
    V: Serialize + DeserializeOwned + Send + Sync + 'static,
{
    type K = K;
    type V = V;
    fn entries(&self) -> Vec<(&K, &V)> {
        self.iter().collect()
    }
    fn get_v(&self, k: &K) -> Option<&V> {
        self.get(k)
    }
    fn put(&mut self, k: K, v: V, quiet: bool) -> Option<V> {
        if quiet {
            self.insert_quiet(k, v)
        } else {
            self.insert(k, v)
        }
    }
    fn del(&mut self, k: &K, quiet: bool) -> Option<V> {
        if quiet {
            self.remove_quiet(k)
        } else {
            self.remove(k)
        }
    }
    fn removed_since(&self, _: usize) -> Vec<K> {
        Vec::new()
    }
    fn removed_value() -> Option<V> {
        None
    }
}

impl<K> MapLike for TrackedSet<K>
where
    K: Serialize + DeserializeOwned + Eq + std::hash::Hash + Clone + Send + Sync + 'static,
{
    type K = K;
    type V = ();
    fn entries(&self) -> Vec<(&K, &())> {
        self.iter().map(|k| (k, &())).collect()
    }
    fn get_v(&self, k: &K) -> Option<&()> {
        self.contains(k).then_some(&())
    }
    fn put(&mut self, k: K, _: (), quiet: bool) -> Option<()> {
        let new = if quiet {
            self.insert_quiet(k)
        } else {
            self.insert(k)
        };
        (!new).then_some(())
    }
    fn del(&mut self, k: &K, quiet: bool) -> Option<()> {
        let had = if quiet {
            self.remove_quiet(k)
        } else {
            self.remove(k)
        };
        had.then_some(())
    }
    fn removed_since(&self, mark: usize) -> Vec<K> {
        TrackedSet::removed_since(self, mark)
    }
    fn removed_value() -> Option<()> {
        Some(())
    }
}

/// Whether the table `acc` reaches is a set.
fn set_of<M: MapLike>(_: fn(&Db) -> &M) -> bool {
    M::removed_value().is_some()
}

fn map_snap<M: MapLike>(m: &M) -> TabSnap {
    let mut buf = Vec::new();
    TabSnap::Map(
        m.entries()
            .into_iter()
            .map(|(k, v)| (hash_raw(k, &mut buf), hash_raw(v, &mut buf)))
            .collect(),
    )
}

fn map_write<M: MapLike>(
    acc: fn(&Db) -> &M,
    db: &Db,
    t: u8,
    keys: Option<&FxHashSet<H>>,
    _: Option<&(Vec<u8>, Vec<u8>)>,
    out: &mut Vec<u8>,
) -> Result<(), String> {
    let keys = keys.ok_or("a map without keys")?;
    let mut buf = Vec::new();
    let mut found = 0;
    for (k, v) in acc(db).entries() {
        if keys.contains(&hash_raw(k, &mut buf)) {
            found += 1;
            put(&t, out)?;
            put(k, out)?;
            put(&None::<&M::V>, out)?;
            put(&Some(v), out)?;
        }
    }
    if found != keys.len() {
        return Err("a changed key is gone".into());
    }
    Ok(())
}

/// The keys of a set that were there at log position `mark` and are gone now: each as table,
/// key, the value it had and none. How many.
fn map_write_removed<M: MapLike>(
    acc: fn(&Db) -> &M,
    db: &Db,
    t: u8,
    mark: usize,
    out: &mut Vec<u8>,
) -> Result<usize, String> {
    let Some(v) = M::removed_value() else {
        return Ok(0);
    };
    let removed = acc(db).removed_since(mark);
    for k in &removed {
        put(&t, out)?;
        put(k, out)?;
        put(&Some(&v), out)?;
        put(&None::<&M::V>, out)?;
    }
    Ok(removed.len())
}

fn map_replay<M: MapLike>(
    acc: fn(&mut Db) -> &mut M,
    db: &mut Db,
    rest: &mut &[u8],
    raw: bool,
) -> Result<(Undo, Changed), String> {
    let key: M::K = take(rest)?;
    let pre: Option<M::V> = take(rest)?;
    let post: Option<M::V> = take(rest)?;
    if !same(acc(db).get_v(&key), pre.as_ref())? {
        return Err("a table changed since".into());
    }
    let changed = raw.then(|| {
        let mut buf = Vec::new();
        (
            hash_raw(&key, &mut buf),
            pre.as_ref().map(|v| hash_raw(v, &mut buf)),
            post.as_ref().map(|v| hash_raw(v, &mut buf)),
        )
    });
    // A frame that records (`raw`) is told the key and value hashes below, so it does not need the log.
    let old = match post {
        Some(v) => acc(db).put(key.clone(), v, raw),
        None => acc(db).del(&key, raw),
    };
    let undo: Undo = Box::new(move |db: &mut Db| {
        let m = acc(db);
        match old {
            Some(v) => {
                m.put(key, v, raw);
            }
            None => {
                m.del(&key, raw);
            }
        }
    });
    Ok((undo, changed))
}

fn map_canon<M: MapLike>(m: &M) -> Result<canon::V, String> {
    let mut v = m
        .entries()
        .into_iter()
        .map(|(k, v)| Ok((canon::canon(k)?, canon::canon(v)?)))
        .collect::<Result<Vec<_>, String>>()?;
    v.sort();
    Ok(canon::V::Map(v))
}

fn val_snap<T: Serialize>(v: &T) -> TabSnap {
    TabSnap::Val(raw_bytes(v))
}

fn val_write<T: Serialize + DeserializeOwned>(
    _: fn(&Db) -> &T,
    _: &Db,
    t: u8,
    _: Option<&FxHashSet<H>>,
    val: Option<&(Vec<u8>, Vec<u8>)>,
    out: &mut Vec<u8>,
) -> Result<(), String> {
    let (pre, post) = val.ok_or("a value table without value")?;
    let pre: T = raw_decode(pre)?;
    let post: T = raw_decode(post)?;
    put(&t, out)?;
    put(&pre, out)?;
    put(&post, out)
}

fn val_replay<T: Serialize + DeserializeOwned + Send + Sync + 'static>(
    acc: fn(&mut Db) -> &mut T,
    db: &mut Db,
    rest: &mut &[u8],
    raw: bool,
) -> Result<(Undo, Changed), String> {
    let pre: T = take(rest)?;
    let post: T = take(rest)?;
    if !same(Some(&*acc(db)), Some(&pre))? {
        return Err("a value changed since".into());
    }
    let changed = raw.then(|| {
        let b = raw_bytes(&post);
        (0, None, Some(twox_hash::XxHash3_128::oneshot(&b)))
    });
    let old = std::mem::replace(acc(db), post);
    let undo: Undo = Box::new(move |db: &mut Db| {
        *acc(db) = old;
    });
    Ok((undo, changed))
}

macro_rules! tables {
    ($($kind:ident $($f:ident).+;)*) => {
        fn make_tables() -> Vec<Desc> {
            vec![$(tables!(@one $kind $($f).+)),*]
        }
    };
    (@one map $($f:ident).+) => {
        Desc {
            name: stringify!($($f).+),
            snap: |db| map_snap(&db.$($f).+),
            mark: |db| TabSnap::Mark(db.$($f).+.mark()),
            track: |db, on| db.$($f).+.track(on),
            forget: |db, m| db.$($f).+.forget(m),
            loglen: |db| db.$($f).+.mark(),
            touched: |db, m| db.$($f).+.touched_since(m),
            write: |db, t, k, v, out| map_write(|db: &Db| &db.$($f).+, db, t, k, v, out),
            set: set_of(|db: &Db| &db.$($f).+),
            write_removed: |db, t, m, out| {
                map_write_removed(|db: &Db| &db.$($f).+, db, t, m, out)
            },
            replay: |db, rest, raw| map_replay(|db: &mut Db| &mut db.$($f).+, db, rest, raw),
            canon: |db| map_canon(&db.$($f).+),
        }
    };
    (@one val $($f:ident).+) => {
        Desc {
            name: stringify!($($f).+),
            snap: |db| val_snap(&db.$($f).+),
            mark: |db| val_snap(&db.$($f).+),
            track: |_, _| {},
            forget: |_, _| {},
            loglen: |_| 0,
            touched: |_, _| Vec::new(),
            write: |db, t, k, v, out| val_write(|db: &Db| &db.$($f).+, db, t, k, v, out),
            set: false,
            write_removed: |_, _, _, _| Ok(0),
            replay: |db, rest, raw| val_replay(|db: &mut Db| &mut db.$($f).+, db, rest, raw),
            canon: |db| canon::canon(&db.$($f).+),
        }
    };
}

// Every serialized field of `Db` that is not an arena, the source map, the diagnostics, the
// TIME unit uses (all four replayed in order), the libraries (`Step::Lib`, unit slots) or the
// loads of this cache. `tests::every_field_is_covered` checks the list.
tables! {
    map attr_index;
    map forwarding;
    map disconnect_times;
    map ret_names;
    map attr_closed;
    val well_known;
    map undeclared;
    map rev_reported;
    map tainted;
    map ext_uses;
    map pkg_targets;
    val validated;
    map config_specs;
    map range_records;
    map generic.defaults;
    map generic.lists;
    map generic.maps;
    val generic.reuse;
    map generic.implicit;
    map generic.formal_anon;
    map generic.implicit_insts;
    map generic.call_insts;
    map generic.protected;
    map generic.header;
    map generic.unused;
    map generic.wants;
    map generic.entities;
    map generic.unit_names;
    map generic.components;
    val generic.blocks;
    val facts;
    val errors;
}

fn tables() -> &'static [Desc] {
    static T: std::sync::OnceLock<Vec<Desc>> = std::sync::OnceLock::new();
    T.get_or_init(make_tables)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Field names of the top-level struct a value serializes as.
    fn field_names<T: Serialize>(v: &T) -> Vec<&'static str> {
        use serde::ser::*;
        struct Names(Vec<&'static str>);
        struct Fields<'a>(&'a mut Names);
        macro_rules! nope {
            ($($m:ident($($t:ty),*)),*) => {$(
                fn $m(self, $(_: $t),*) -> Result<(), Error> { Ok(()) }
            )*};
        }
        #[derive(Debug)]
        struct Error;
        impl std::fmt::Display for Error {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                f.write_str("e")
            }
        }
        impl std::error::Error for Error {}
        impl serde::ser::Error for Error {
            fn custom<M: std::fmt::Display>(_: M) -> Self {
                Error
            }
        }
        impl<'a> SerializeStruct for Fields<'a> {
            type Ok = ();
            type Error = Error;
            fn serialize_field<V: ?Sized + Serialize>(
                &mut self,
                key: &'static str,
                _: &V,
            ) -> Result<(), Error> {
                self.0.0.push(key);
                Ok(())
            }
            fn end(self) -> Result<(), Error> {
                Ok(())
            }
        }
        impl<'a> Serializer for &'a mut Names {
            type Ok = ();
            type Error = Error;
            type SerializeSeq = Impossible<(), Error>;
            type SerializeTuple = Impossible<(), Error>;
            type SerializeTupleStruct = Impossible<(), Error>;
            type SerializeTupleVariant = Impossible<(), Error>;
            type SerializeMap = Impossible<(), Error>;
            type SerializeStruct = Fields<'a>;
            type SerializeStructVariant = Impossible<(), Error>;
            nope!(
                serialize_bool(bool),
                serialize_i8(i8),
                serialize_i16(i16),
                serialize_i32(i32),
                serialize_i64(i64),
                serialize_u8(u8),
                serialize_u16(u16),
                serialize_u32(u32),
                serialize_u64(u64),
                serialize_f32(f32),
                serialize_f64(f64),
                serialize_char(char),
                serialize_str(&str),
                serialize_bytes(&[u8]),
                serialize_none(),
                serialize_unit(),
                serialize_unit_struct(&'static str),
                serialize_unit_variant(&'static str, u32, &'static str)
            );
            fn serialize_some<V: ?Sized + Serialize>(self, _: &V) -> Result<(), Error> {
                Ok(())
            }
            fn serialize_newtype_struct<V: ?Sized + Serialize>(
                self,
                _: &'static str,
                _: &V,
            ) -> Result<(), Error> {
                Ok(())
            }
            fn serialize_newtype_variant<V: ?Sized + Serialize>(
                self,
                _: &'static str,
                _: u32,
                _: &'static str,
                _: &V,
            ) -> Result<(), Error> {
                Ok(())
            }
            fn serialize_seq(self, _: Option<usize>) -> Result<Self::SerializeSeq, Error> {
                Err(Error)
            }
            fn serialize_tuple(self, _: usize) -> Result<Self::SerializeTuple, Error> {
                Err(Error)
            }
            fn serialize_tuple_struct(
                self,
                _: &'static str,
                _: usize,
            ) -> Result<Self::SerializeTupleStruct, Error> {
                Err(Error)
            }
            fn serialize_tuple_variant(
                self,
                _: &'static str,
                _: u32,
                _: &'static str,
                _: usize,
            ) -> Result<Self::SerializeTupleVariant, Error> {
                Err(Error)
            }
            fn serialize_map(self, _: Option<usize>) -> Result<Self::SerializeMap, Error> {
                Err(Error)
            }
            fn serialize_struct(
                self,
                _: &'static str,
                _: usize,
            ) -> Result<Self::SerializeStruct, Error> {
                Ok(Fields(self))
            }
            fn serialize_struct_variant(
                self,
                _: &'static str,
                _: u32,
                _: &'static str,
                _: usize,
            ) -> Result<Self::SerializeStructVariant, Error> {
                Err(Error)
            }
        }
        let mut n = Names(Vec::new());
        v.serialize(&mut n).expect("a struct");
        n.0
    }

    /// A new field of `Db` or `GenericState` must be replayed: listed in `tables!` or handled
    /// by the replay itself.
    #[test]
    fn every_field_is_covered() {
        let db = Db::new(crate::Options::new(LangRev::Vhdl2008, ""));
        let listed: Vec<&str> = tables().iter().map(|t| t.name).collect();
        let special = [
            "rev",
            "sources",
            "libs",
            "units",
            "decls",
            "scopes",
            "types",
            "subtypes",
            "exprs",
            "stmts",
            "consts",
            "insts",
            "views",
            "externals",
            "attr_values",
            "time_units",
            "poison",
            "root",
            "lib_by_key",
            "diags",
            "loads",
            "file_owner",
            // Made by `to_image` from the finished design, never by a unit (#426).
            "run_time_pkgs",
        ];
        for f in field_names(&db) {
            if f == "generic" {
                continue;
            }
            assert!(
                listed.contains(&f) || special.contains(&f),
                "Db::{f} is not covered by the analysed-unit cache"
            );
        }
        for f in field_names(&db.generic) {
            let name = format!("generic.{f}");
            assert!(
                listed.iter().any(|l| l.replace(' ', "") == name),
                "GenericState::{f} is not covered by the analysed-unit cache"
            );
        }
    }
}
