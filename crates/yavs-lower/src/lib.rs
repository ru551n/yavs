//! Lowering of yavs-sem's typed HIR into yavs-ir (`DESIGN.md` in this crate).
//!
//! [`Lowerer`] turns analyzed design units into one [`yavs_ir::Program`]: one `Func` per
//! process and subprogram body, shared by every instance of its design unit (ir.md 2), and a
//! [`UnitIr`] per architecture describing its instance data, signals and processes for the
//! elaborator. Subprograms are lowered on first reference, so the std and ieee bodies a design
//! never calls are not lowered. Every instruction carries the statement it belongs to.
//!
//! Fail early: a construct this crate does not lower yet is an `E2001` diagnostic naming it,
//! never a guessed value; a broken HIR invariant is `E9999`. Any error means no program.

mod access;
mod composite;
mod expr;
mod file;
mod force;
mod foreign;
mod func;
mod protected;
mod reflect;
mod stmt;
mod ty;

use std::collections::HashMap;

use yavs_diag::{Diagnostic, Span};
use yavs_ir::{
    CovInfo, CovItem, CovKind, FuncId, FuncKind, NativeId, Program, ProgramBuilder, TyId,
    VerifyError, Width,
};
use yavs_sem::decl::DeclKind;
use yavs_sem::hir::UnitHir;
use yavs_sem::{Db, DeclId, ScopeId, StmtId, TypeId, UnitId};

pub use ty::{encode_array, signal_elem};

/// A construct the lowering does not handle yet (yavs-diag registry: `E2000`-`E2099`).
pub const NOT_IMPLEMENTED: &str = "E2001";
/// Internal compiler error.
pub const INTERNAL: &str = "E9999";

#[derive(Clone, Copy, Debug, Default)]
pub struct LowerOptions {
    /// Emit statement and branch coverage counters (`Cover`, rule 10).
    pub coverage: bool,
}

/// The lowered form of an architecture: what the elaborator needs to build its instances.
#[derive(Clone, Debug)]
pub struct UnitIr {
    pub unit: UnitId,
    /// Bytes of instance data (`AddrSpace::Inst`).
    pub inst_size: u32,
    /// Generics and constants whose values the elaborator supplies (`Elaborated::value`),
    /// written into instance data with [`UnitIr::write_value`], in declaration order.
    pub values: Vec<InstValue>,
    /// Scalar constants the elaborator could not fold: it runs each thunk (it may read the
    /// values before it) and stores the returned cell at `off` with width `w`.
    pub consts: Vec<InstConst>,
    /// Signals, indexed by `Inst::SigVal::sig`.
    pub signals: Vec<SignalIr>,
    /// Processes in elaboration order, concurrent statements included.
    pub processes: Vec<ProcessIr>,
    /// `'path_name` and `'instance_name` strings the elaborator writes with
    /// [`UnitIr::write_string`] (LRM 16.2.5, lower case).
    pub names: Vec<InstName>,
}

/// A `'path_name` (`instance: false`) or `'instance_name` of named entity `decl`, as seen from
/// the region whose instance data is being built; an array slot at `off`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InstName {
    pub decl: DeclId,
    pub instance: bool,
    pub off: u32,
}

#[derive(Clone, Debug)]
pub struct InstConst {
    pub decl: DeclId,
    pub name: String,
    pub off: u32,
    pub w: Width,
    /// A composite constant's IR type: the thunk returns the address of its fat pair
    /// `[data, desc]`, written like a composite [`InstValue`] (`w` unused).
    pub comp: Option<TyId>,
    /// `FuncKind::Thunk` returning the value.
    pub init: FuncId,
    /// Declared in the entity or architecture itself (not in a block or generate): one value
    /// per instance, shared by the instance data of all its regions. A shared variable's
    /// thunk creates its object, so it must run once per instance (LRM 6.4.2.4).
    pub instance_level: bool,
}

#[derive(Clone, Debug)]
pub struct SignalIr {
    pub decl: DeclId,
    /// Original spelling (rule 8).
    pub name: String,
    pub ty: TyId,
    /// `FuncKind::Thunk` returning the initial value of a scalar signal (`None` for ports
    /// and arrays: the elaborator computes initial values itself).
    pub init: Option<FuncId>,
    /// The leaf of a record signal: field indices from the signal's root (empty for a
    /// signal that is one kernel signal). A record signal has one entry per leaf, in leaf
    /// order (docs/decisions.md "Elaboration": one leaf per field).
    pub path: Vec<u32>,
    /// An array signal's descriptor in instance data (`Ty::desc_size` bytes at this offset),
    /// written by the elaborator from the signal's bounds in each instance: ports take their
    /// actual's bounds.
    pub dope: Option<u32>,
}

/// A generic or constant in instance data.
#[derive(Clone, Debug)]
pub struct InstValue {
    pub decl: DeclId,
    pub name: String,
    pub off: u32,
    /// Its IR type: a scalar is one cell of the type's width at `off`; an array has the byte
    /// offsets of its elements and of its descriptor (both in instance data) at `off` and
    /// `off + 8`.
    pub ty: TyId,
}

impl UnitIr {
    /// Write string `s` (a STRING `1 to len`) into the array slot at `off` of `data`.
    pub fn write_string(off: u32, s: &str, data: &mut Vec<u8>) -> Result<(), String> {
        let off = off as usize;
        if data.len() < off + 16 {
            return Err("instance data too short for a name".into());
        }
        let mut put = |bytes: &[u8]| {
            let at = data.len().next_multiple_of(8);
            data.resize(at, 0);
            data.extend_from_slice(bytes);
            at as u64
        };
        let e = put(s.as_bytes());
        let dope: Vec<u8> = [1i64, s.len() as i64, 0]
            .iter()
            .flat_map(|x| x.to_le_bytes())
            .collect();
        let d = put(&dope);
        data[off..off + 8].copy_from_slice(&e.to_le_bytes());
        data[off + 8..off + 16].copy_from_slice(&d.to_le_bytes());
        Ok(())
    }

    /// Write value `v` of `iv` into instance data `data` (at least `inst_size` bytes): a
    /// scalar into its cell, an array appended (elements in the IR layout, bit-planes for
    /// logic arrays, then the descriptor) with the offsets stored at `iv.off`.
    pub fn write_value(
        prog: &Program,
        iv: &InstValue,
        v: &yavs_sem::stat::ConstValue,
        data: &mut Vec<u8>,
    ) -> Result<(), String> {
        let off = iv.off as usize;
        let t = prog.types.get(iv.ty).ok_or("missing type")?;
        if data.len() < off + t.scalar_size().map_or(16, |n| n as usize) {
            return Err(format!("instance data too short for `{}`", iv.name));
        }
        if let Some(n) = t.scalar_size() {
            let c = ty::cell(v).ok_or_else(|| format!("`{}`: not a scalar value", iv.name))?;
            data[off..off + n as usize].copy_from_slice(&c.to_le_bytes()[..n as usize]);
            return Ok(());
        }
        let (elems, dope) = ty::encode_array(prog, iv.ty, v)
            .ok_or_else(|| format!("`{}`: value does not fit its array type", iv.name))?;
        Self::write_composite(iv.off, &elems, &dope, data);
        Ok(())
    }

    /// Append a composite's elements and descriptor to `data` and write their offsets into
    /// the slot at `off` (an `InstValue` of composite type, which `data` must cover).
    pub fn write_composite(off: u32, elems: &[u8], desc: &[u8], data: &mut Vec<u8>) {
        let off = off as usize;
        let mut put = |bytes: &[u8]| {
            let at = data.len().next_multiple_of(8);
            data.resize(at, 0);
            data.extend_from_slice(bytes);
            at as u64
        };
        let e = put(elems);
        let d = put(desc);
        data[off..off + 8].copy_from_slice(&e.to_le_bytes());
        data[off + 8..off + 16].copy_from_slice(&d.to_le_bytes());
    }
}

#[derive(Clone, Debug)]
pub struct ProcessIr {
    /// The HIR statement (a process or a concurrent statement).
    pub stmt: StmtId,
    pub func: FuncId,
    pub label: Option<String>,
    pub postponed: bool,
    /// Driver slots, one per `ProcessInfo::drivers` entry in order: the instance signal it
    /// drives (the elaborator gives each slot the entry's kernel range).
    pub drivers: Vec<u32>,
}

#[derive(Debug)]
pub struct Lowered {
    pub program: Program,
    pub units: Vec<UnitIr>,
    /// Packages with run-time objects, in the order [`Lowerer::package`] was called; the
    /// `index` of each is its `AddrSpace::Pkg` number.
    pub packages: Vec<PackageIr>,
    /// Bytes of the shared variables area (`SharedState::vars`): file objects of packages.
    pub shared_size: u32,
}

/// A package's objects at run time (LRM 14.4.2). The elaborator initialises each package
/// once, in dependency order, with `yavs_interp::init_package(rt, init, index, size, shared)`
/// and binds the results as `Bindings::packages` (index order); signals are the first
/// entries of every `UnitIr::signals` (the same index in every instance).
#[derive(Clone, Debug)]
pub struct PackageIr {
    /// The package declaration (its body, if analyzed, is included).
    pub unit: UnitId,
    pub index: u32,
    /// Bytes of package data.
    pub size: u32,
    /// `FuncKind::Thunk`: evaluates the constants and creates the shared variables.
    pub init: FuncId,
    pub signals: Vec<SignalIr>,
}

/// Lowering failed; the diagnostics say why.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Failed;

pub(crate) type R<T> = Result<T, Failed>;

/// Where an object lives.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Obj {
    /// A scalar of width `w` at `off` in `space`.
    Scalar { space: Space, off: u32, w: Width },
    /// A scalar `out`/`inout` parameter: the slot holds the address of the actual.
    ScalarRef { space: Space, off: u32, w: Width },
    /// An array: a fat pair `[data, dope]` at `off`.
    Array { space: Space, off: u32 },
    /// Instance signal `sig`.
    Signal { sig: u32 },
    /// A record signal: one instance signal per leaf (`Cx::rec_leaves`).
    RecSignal,
    /// A composite of static layout stored in place: `size` bytes of elements at `off`, then
    /// its descriptor.
    Fixed { space: Space, off: u32, size: u32 },
    /// An array in instance data: byte offsets of its elements and descriptor at `off`.
    InstArray { off: u32 },
    /// A signal parameter: the slot holds a signal reference (two cells).
    /// With `out`: an `out`/`inout` signal parameter, the caller's driver slot in the cell
    /// at `off + 16`.
    SignalRef { space: Space, off: u32, out: bool },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Space {
    /// The frame of the function at this nesting depth.
    Frame(u8),
    Inst,
    /// Package data (`AddrSpace::Pkg`).
    Pkg(u32),
    /// The shared variables area (`AddrSpace::Shared`, serial processes only).
    Shared,
}

/// State shared by the functions being lowered.
pub(crate) struct Cx {
    pub opts: LowerOptions,
    pub diags: Vec<Diagnostic>,
    pub tys: HashMap<TypeId, TyId>,
    pub natives: HashMap<(String, Option<TyId>), NativeId>,
    pub foreigns: HashMap<DeclId, yavs_ir::ForeignId>,
    pub funcs: HashMap<DeclId, FuncId>,
    /// Subprograms declared but not lowered yet, with their nesting depth.
    pub queue: Vec<(DeclId, u8)>,
    /// Function and depth of each subprogram or process region.
    pub scope_func: HashMap<ScopeId, (FuncId, u8)>,
    /// Driver LSPs and slot bases of each process region: subprograms declared in a process
    /// drive through its drivers (LRM 14.7.2).
    pub proc_drivers: HashMap<ScopeId, (Vec<yavs_sem::hir::Lsp>, Vec<u32>)>,
    pub objs: HashMap<DeclId, Obj>,
    /// `std.reflection` entries per subtype (`reflect.rs`).
    pub refl: HashMap<yavs_sem::SubtypeId, yavs_ir::ReflId>,
    /// Instance data of the architecture being lowered: bytes allocated so far, and the
    /// `'path_name`/`'instance_name` slots its code reads.
    pub inst_size: u32,
    pub names: Vec<InstName>,
    /// Instance-data descriptor offsets of array signals (`SignalIr::dope`).
    pub sig_dopes: HashMap<DeclId, u32>,
    /// Signals of packages: the first entries of every unit's signal table.
    pub pkg_sigs: Vec<SignalIr>,
    /// The entity and architecture regions of the architecture being lowered.
    pub arch_scopes: Vec<ScopeId>,
    /// Bytes of the shared variables area allocated so far (package files).
    pub shared_size: u32,
    /// The leaves of record signals, in leaf order.
    pub rec_leaves: HashMap<DeclId, Vec<RecLeaf>>,
}

/// One kernel signal of a record signal.
#[derive(Clone, Debug)]
pub(crate) struct RecLeaf {
    pub path: Vec<u32>,
    pub st: yavs_sem::SubtypeId,
    pub sig: u32,
}

impl Cx {
    /// `size` bytes of instance data of the architecture being lowered (8-byte aligned).
    pub fn inst_alloc(&mut self, size: u32) -> u32 {
        let off = self.inst_size.next_multiple_of(8);
        self.inst_size = off + size;
        off
    }

    pub fn error(&mut self, code: &'static str, span: Span, msg: impl Into<String>) -> Failed {
        self.diags
            .push(Diagnostic::error(code, msg.into()).with_primary(span, ""));
        Failed
    }

    pub fn unsupported(&mut self, span: Span, what: &str) -> Failed {
        self.error(
            NOT_IMPLEMENTED,
            span,
            format!("{what} is not lowered to IR yet (M2)"),
        )
    }

    pub fn ice(&mut self, span: Span, what: &str) -> Failed {
        self.error(
            INTERNAL,
            span,
            format!("internal error: {what}; this is a bug in yavs"),
        )
    }

    pub fn cover(
        &mut self,
        pb: &mut ProgramBuilder,
        stmt: yavs_ir::StmtId,
        kind: CovKind,
    ) -> Option<CovItem> {
        self.opts.coverage.then(|| pb.cov(CovInfo { stmt, kind }))
    }
}

pub struct Lowerer<'a> {
    db: &'a Db,
    pb: ProgramBuilder,
    cx: Cx,
    units: Vec<UnitIr>,
    packages: Vec<PackageIr>,
}

impl<'a> Lowerer<'a> {
    /// `db` must hold no error diagnostics (fail early: nothing after a failed stage).
    pub fn new(db: &'a Db, opts: LowerOptions) -> Self {
        Lowerer {
            db,
            pb: ProgramBuilder::new(),
            cx: Cx {
                opts,
                diags: Vec::new(),
                tys: HashMap::new(),
                natives: HashMap::new(),
                foreigns: HashMap::new(),
                funcs: HashMap::new(),
                queue: Vec::new(),
                scope_func: HashMap::new(),
                proc_drivers: HashMap::new(),
                objs: HashMap::new(),
                refl: HashMap::new(),
                inst_size: 0,
                names: Vec::new(),
                sig_dopes: HashMap::new(),
                pkg_sigs: Vec::new(),
                arch_scopes: Vec::new(),
                shared_size: 0,
                rec_leaves: HashMap::new(),
            },
            packages: Vec::new(),
            units: Vec::new(),
        }
    }

    /// Lower an architecture: its signals, instance constants and processes.
    pub fn architecture(&mut self, unit: UnitId) -> Result<usize, Vec<Diagnostic>> {
        let r = func::architecture(self.db, &mut self.pb, &mut self.cx, unit);
        self.drain();
        match r {
            Ok(u) if self.cx.diags.is_empty() => {
                self.units.push(u);
                Ok(self.units.len() - 1)
            }
            _ => Err(std::mem::take(&mut self.cx.diags)),
        }
    }

    /// Lower the run-time objects of package `unit` (and of its body, when analyzed): call it
    /// for every package with objects before lowering the architectures that use them.
    pub fn package(&mut self, unit: UnitId) -> Result<usize, Vec<Diagnostic>> {
        let index = self.packages.len() as u32;
        let r = func::package(self.db, &mut self.pb, &mut self.cx, unit, index);
        self.drain();
        match r {
            Ok(p) if self.cx.diags.is_empty() => {
                self.packages.push(p);
                Ok(self.packages.len() - 1)
            }
            _ => Err(std::mem::take(&mut self.cx.diags)),
        }
    }

    /// Lower a subprogram (and everything it calls), returning its function.
    pub fn subprogram(&mut self, decl: DeclId) -> Result<FuncId, Vec<Diagnostic>> {
        let f = func::func_of(self.db, &mut self.pb, &mut self.cx, decl, Span::default());
        self.drain();
        match f {
            Ok(f) if self.cx.diags.is_empty() => Ok(f),
            _ => Err(std::mem::take(&mut self.cx.diags)),
        }
    }

    /// The IR type of `t` (lowered once), for callers marshalling values of it.
    pub fn ty(&mut self, t: yavs_sem::TypeId) -> Result<yavs_ir::TyId, Vec<Diagnostic>> {
        match ty::lower(self.db, &mut self.cx, &mut self.pb, t, Span::default()) {
            Ok(id) => Ok(id),
            Err(_) => Err(std::mem::take(&mut self.cx.diags)),
        }
    }

    /// Every subprogram body declared in the units' regions (a package body, say).
    pub fn subprograms_of(&mut self, unit: UnitId) -> Result<Vec<FuncId>, Vec<Diagnostic>> {
        let u = &self.db.units[unit];
        let scope = u.scope;
        let mut out = Vec::new();
        let decls: Vec<DeclId> = self.db.scopes[scope]
            .order
            .iter()
            .copied()
            .filter(|&d| matches!(&self.db.decls[d].kind, DeclKind::Subprogram(s) if matches!(s.body, yavs_sem::decl::Body::Hir { .. })))
            .collect();
        for d in decls {
            out.push(self.subprogram(d)?);
        }
        Ok(out)
    }

    fn drain(&mut self) {
        while let Some((d, depth)) = self.cx.queue.pop() {
            // Errors are collected in `cx.diags`.
            let _ = func::subprogram(self.db, &mut self.pb, &mut self.cx, d, depth);
        }
    }

    /// Verify and freeze the program. Verifier errors are internal errors of this crate.
    pub fn finish(mut self) -> Result<Lowered, Vec<Diagnostic>> {
        self.drain();
        if !self.cx.diags.is_empty() {
            return Err(self.cx.diags);
        }
        match self.pb.finish() {
            Ok(program) => Ok(Lowered {
                program,
                units: self.units,
                packages: self.packages,
                shared_size: self.cx.shared_size,
            }),
            Err(errs) => Err(errs.iter().map(verify_ice).collect()),
        }
    }
}

fn verify_ice(e: &VerifyError) -> Diagnostic {
    Diagnostic::error(
        INTERNAL,
        format!("internal error: {e}; this is a bug in yavs"),
    )
}

/// The architecture of entity `top` to elaborate: the last one analyzed (LRM default binding
/// as yavs applies it, sem.md Q4).
pub fn top_architecture(db: &Db, top: &str) -> Option<UnitId> {
    let key = top.to_ascii_lowercase();
    db.units
        .iter()
        .filter(|(_, u)| match &u.hir {
            UnitHir::Architecture { entity, .. } => db.units[*entity].name.key.as_str() == key,
            _ => false,
        })
        .map(|(id, _)| id)
        .last()
}

/// The kind of function a subprogram becomes.
pub(crate) fn func_kind(db: &Db, d: DeclId) -> FuncKind {
    match db.decls[d].kind.subprogram().map(|s| s.kind) {
        Some(yavs_sem::decl::SubprogramKind::Procedure) => FuncKind::Procedure,
        _ => FuncKind::Function,
    }
}
