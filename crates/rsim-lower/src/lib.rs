//! Lowering of rsim-sem's typed HIR into rsim-ir (`DESIGN.md` in this crate).
//!
//! [`Lowerer`] turns analyzed design units into one [`rsim_ir::Program`]: one `Func` per
//! process and subprogram body, shared by every instance of its design unit (ir.md 2), and a
//! [`UnitIr`] per architecture describing its instance data, signals and processes for the
//! elaborator. Subprograms are lowered on first reference, so the std and ieee bodies a design
//! never calls are not lowered. Every instruction carries the statement it belongs to.
//!
//! Fail early: a construct this crate does not lower yet is an `E2001` diagnostic naming it,
//! never a guessed value; a broken HIR invariant is `E9999`. Any error means no program.

mod expr;
mod func;
mod stmt;
mod ty;

use std::collections::HashMap;

use rsim_diag::{Diagnostic, Span};
use rsim_ir::{
    CovInfo, CovItem, CovKind, FuncId, FuncKind, NativeId, Program, ProgramBuilder, TyId,
    VerifyError, Width,
};
use rsim_sem::decl::DeclKind;
use rsim_sem::hir::UnitHir;
use rsim_sem::{Db, DeclId, ScopeId, StmtId, TypeId, UnitId};

pub use ty::signal_elem;

/// A construct the lowering does not handle yet (rsim-diag registry: `E2000`-`E2099`).
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
    /// Instance constants in declaration order. The elaborator runs each thunk (it may read
    /// the constants before it) and stores the returned cell at `off` with width `w`.
    pub consts: Vec<InstConst>,
    /// Signals, indexed by `Inst::SigVal::sig`.
    pub signals: Vec<SignalIr>,
    /// Processes in elaboration order, concurrent statements included.
    pub processes: Vec<ProcessIr>,
}

#[derive(Clone, Debug)]
pub struct InstConst {
    pub decl: DeclId,
    pub name: String,
    pub off: u32,
    pub w: Width,
    /// `FuncKind::Thunk` returning the value.
    pub init: FuncId,
}

#[derive(Clone, Debug)]
pub struct SignalIr {
    pub decl: DeclId,
    /// Original spelling (rule 8).
    pub name: String,
    pub ty: TyId,
    /// `FuncKind::Thunk` returning the initial value (a scalar cell).
    pub init: FuncId,
}

#[derive(Clone, Debug)]
pub struct ProcessIr {
    /// The HIR statement (a process or a concurrent statement).
    pub stmt: StmtId,
    pub func: FuncId,
    pub label: Option<String>,
    pub postponed: bool,
    /// Driver slots: the instance signal each `Drive::driver` index drives (whole signal).
    pub drivers: Vec<u32>,
}

#[derive(Debug)]
pub struct Lowered {
    pub program: Program,
    pub units: Vec<UnitIr>,
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
    /// A signal parameter: the slot holds a signal reference (two cells).
    SignalRef { space: Space, off: u32 },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Space {
    /// The frame of the function at this nesting depth.
    Frame(u8),
    Inst,
}

/// State shared by the functions being lowered.
pub(crate) struct Cx {
    pub opts: LowerOptions,
    pub diags: Vec<Diagnostic>,
    pub tys: HashMap<TypeId, TyId>,
    pub natives: HashMap<(String, Option<TyId>), NativeId>,
    pub funcs: HashMap<DeclId, FuncId>,
    /// Subprograms declared but not lowered yet, with their nesting depth.
    pub queue: Vec<(DeclId, u8)>,
    /// Function and depth of each subprogram or process region.
    pub scope_func: HashMap<ScopeId, (FuncId, u8)>,
    pub objs: HashMap<DeclId, Obj>,
}

impl Cx {
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
            format!("internal error: {what}; this is a bug in rsim"),
        )
    }

    pub fn cover(
        &mut self,
        pb: &mut ProgramBuilder,
        stmt: rsim_ir::StmtId,
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
                funcs: HashMap::new(),
                queue: Vec::new(),
                scope_func: HashMap::new(),
                objs: HashMap::new(),
            },
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

    /// Lower a subprogram (and everything it calls), returning its function.
    pub fn subprogram(&mut self, decl: DeclId) -> Result<FuncId, Vec<Diagnostic>> {
        let f = func::func_of(self.db, &mut self.pb, &mut self.cx, decl, Span::default());
        self.drain();
        match f {
            Ok(f) if self.cx.diags.is_empty() => Ok(f),
            _ => Err(std::mem::take(&mut self.cx.diags)),
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
            .filter(|&d| matches!(&self.db.decls[d].kind, DeclKind::Subprogram(s) if matches!(s.body, rsim_sem::decl::Body::Hir { .. })))
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
            }),
            Err(errs) => Err(errs.iter().map(verify_ice).collect()),
        }
    }
}

fn verify_ice(e: &VerifyError) -> Diagnostic {
    Diagnostic::error(
        INTERNAL,
        format!("internal error: {e}; this is a bug in rsim"),
    )
}

/// The architecture of entity `top` to elaborate: the last one analyzed (LRM default binding
/// as rsim applies it, sem.md Q4).
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
        Some(rsim_sem::decl::SubprogramKind::Procedure) => FuncKind::Procedure,
        _ => FuncKind::Function,
    }
}
