//! Functions: frames, parameters, declarations, subprogram and process bodies, architectures.

use yavs_diag::Span;
use yavs_ir::{
    AddrSpace, BlockId, FuncBuilder, FuncFlags, FuncId, FuncKind, Inst, Mode, ParamClass,
    ProgramBuilder, Reg, RegTy, RootKind, StmtKind, Ty, TyId, Width,
};
use yavs_sem::decl::{Alias, Body, DeclKind, ObjClass, ObjRole, Param, SubprogramKind};
use yavs_sem::hir::{self, Mode as HMode, Sensitivity, StmtKind as HStmt, UnitHir};
use yavs_sem::scope::ScopeKind;
use yavs_sem::types::RangeC;
use yavs_sem::{Db, DeclId, ScopeId, StmtId, SubtypeId, UnitId};

use crate::expr::Place;
use crate::{
    Cx, Failed, InstConst, InstValue, Obj, ProcessIr, R, SignalIr, Space, UnitIr, VarShape, ty,
};

/// How a parameter is passed (DESIGN.md "Subprograms").
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shape {
    /// Scalar by value: one cell.
    Val,
    /// Scalar `out`/`inout` variable: the address of the actual.
    Ref,
    /// Array: data address and descriptor address, two cells.
    Arr,
    /// Scalar `in` signal: a signal reference (two cells).
    Sig,
    /// Scalar `out`/`inout` signal: a signal reference and the caller's driver slot.
    SigOut,
    /// Signal with records in its type: a reference per leaf (`RecParam`); `true` for
    /// `out`/`inout`.
    RecSig(bool),
}

pub(crate) fn shape(db: &Db, cx: &mut Cx, p: &Param, span: Span) -> R<Shape> {
    match p.class {
        ObjClass::Signal if ty::has_record(db, db.base(p.subtype)) => match p.mode {
            HMode::In => return Ok(Shape::RecSig(false)),
            // A mode view drives only its `out`, `inout` and `buffer` leaves, each through
            // the slot its reference carries (`Fx::view_leaves`).
            HMode::Out | HMode::Inout | HMode::View { .. } => return Ok(Shape::RecSig(true)),
            HMode::Buffer | HMode::Linkage => {
                return Err(cx.ice(span, "buffer or linkage parameter (sem reports E1236)"));
            }
        },
        ObjClass::Signal
            if !ty::is_array(db, db.base(p.subtype))
                && !db.types[db.base(p.subtype)].kind_is_scalar() =>
        {
            return Err(cx.unsupported(span, "signal parameters of this type"));
        }
        ObjClass::Signal if p.mode == HMode::In => return Ok(Shape::Sig),
        ObjClass::Signal if matches!(p.mode, HMode::Out | HMode::Inout) => {
            return Ok(Shape::SigOut);
        }
        ObjClass::Signal => {
            return Err(cx.ice(
                span,
                "buffer, linkage or view parameter of a scalar or array",
            ));
        }
        // A file object is passed as the address of its cell (yavs-ir README, natives).
        ObjClass::File => return Ok(Shape::Ref),
        _ => {}
    }
    match &db.types[db.base(p.subtype)].kind {
        yavs_sem::types::TypeKind::Record { .. } => return Ok(Shape::Arr),
        // A protected object is its handle; the state is shared by reference.
        yavs_sem::types::TypeKind::Protected { .. } => return Ok(Shape::Val),
        yavs_sem::types::TypeKind::Access { .. } => {
            return Ok(if p.mode == HMode::In {
                Shape::Val
            } else {
                Shape::Ref
            });
        }
        _ => {}
    }
    if ty::is_array(db, db.base(p.subtype)) {
        return Ok(Shape::Arr);
    }
    if !db.types[db.base(p.subtype)].kind_is_scalar() {
        return Err(cx.unsupported(span, "record, access and protected parameters (M3)"));
    }
    match p.mode {
        HMode::In => Ok(Shape::Val),
        HMode::Out | HMode::Inout => Ok(Shape::Ref),
        _ => Err(cx.ice(span, "buffer or linkage parameter (sem reports E1236)")),
    }
}

trait ScalarKind {
    fn kind_is_scalar(&self) -> bool;
}

impl ScalarKind for yavs_sem::types::TypeDef {
    fn kind_is_scalar(&self) -> bool {
        use yavs_sem::types::TypeKind::*;
        matches!(
            self.kind,
            Enum { .. } | Integer | Floating | Physical { .. } | UniversalInteger | UniversalReal
        )
    }
}

/// The function of subprogram `d`, declared (and queued for lowering) on first use.
pub(crate) fn func_of(
    db: &Db,
    pb: &mut ProgramBuilder,
    cx: &mut Cx,
    d: DeclId,
    span: Span,
) -> R<FuncId> {
    if let Some(&f) = cx.funcs.get(&d) {
        return Ok(f);
    }
    let decl = &db.decls[d];
    let Some(sp) = decl.kind.subprogram() else {
        return Err(cx.ice(span, "call of a declaration that is not a subprogram"));
    };
    if !matches!(sp.body, Body::Hir { .. }) {
        return Err(cx.ice(span, "lowering a subprogram without a VHDL body"));
    }
    // The enclosing subprogram or process, for the static link.
    let mut s = Some(decl.scope);
    let mut parent = None;
    while let Some(sc) = s {
        if let Some(&p) = cx.scope_func.get(&sc) {
            parent = Some(p);
            break;
        }
        if matches!(
            db.scopes[sc].kind,
            ScopeKind::Subprogram | ScopeKind::Process
        ) {
            return Err(cx.ice(span, "nested subprogram referenced before its parent"));
        }
        s = db.scopes[sc].parent;
    }
    let kind = crate::func_kind(db, d);
    let f = pb.declare(&decl.name.to_string(), kind, parent.map(|p| p.0));
    let depth = parent.map_or(0, |p| p.1 + 1);
    cx.funcs.insert(d, f);
    cx.queue.push((d, depth));
    Ok(f)
}

/// Lowering state of one function.
pub(crate) struct Fx<'a, 'b> {
    pub db: &'a Db,
    pub cx: &'b mut Cx,
    pub fb: FuncBuilder<'b>,
    pub depth: u8,
    pub kind: FuncKind,
    /// A postponed process (`FuncKind::Process` only).
    pub postponed: bool,
    /// Whether the current block can still take instructions (no terminator yet).
    pub live: bool,
    /// Enclosing loops: (loop statement, `next` target, `exit` target).
    pub loops: Vec<(StmtId, BlockId, BlockId)>,
    /// The result subtype of a function.
    pub ret: Option<SubtypeId>,
    pub name: String,
    /// First driver slot of each `drivers` entry (a record LSP covers several).
    pub driver_base: Vec<u32>,
    /// Driver slots of a process: the signal declaration each slot drives.
    pub drivers: Vec<hir::Lsp>,
    /// The IR statement being lowered, and its span.
    pub cur: Option<(yavs_ir::StmtId, Span)>,
    /// Emit coverage counters (off in the implicit processes of port actuals and guards).
    pub cov: bool,
    /// The inputs of the conditions being lowered (`covcond.rs`): the state register, the
    /// shift of the input in it and whether it is a `std_ulogic`.
    pub cov_inputs: std::collections::HashMap<yavs_sem::ExprId, (Reg, u8, bool)>,
    /// In a protected type method: the frame depth and slot of `self`.
    pub prot_self: Option<(u8, u32)>,
    /// `default_into` gives protected subelements new objects (2019 composites of
    /// protected types, LCS-2016-014); off while `prot_new` lays out a state whose
    /// variables get theirs later, in declaration order.
    pub prot_defaults: bool,
    /// The `GUARD` signal of the enclosing guarded block (guarded assignments).
    pub guard: Option<DeclId>,
    /// Copy-out after the call being lowered: (frame slot of the temporary, actual,
    /// subtype of the formal) for `out`/`inout` actuals passed through a temporary.
    pub writeback: Vec<(u32, Option<yavs_sem::ExprId>, SubtypeId)>,
    /// Copy-out of individually associated or converted `out`/`inout` formals: (the
    /// temporary formal, the formal, its parts; a converted formal is one part whose
    /// formal part is the conversion).
    pub writeback_parts: Vec<(crate::Obj, DeclId, Box<[yavs_sem::hir::IndividualPart]>)>,
    /// Files declared in this subprogram (frame offset of the cell, `file_close` symbol and
    /// type), closed when it returns (LRM 14.6).
    pub local_files: Vec<(u32, String, TyId)>,
    /// A process with a sensitivity list: its whole-value reads of composite signals (#630).
    pub sigcopy: Option<SigCopies>,
    /// The bounds `(left, right, dir)` a one-dimensional descriptor was made from in a block
    /// (`Inst::MakeDope`; only valid in that block), so a native `ieee` call takes them as values (#697).
    pub made_dopes: Vec<(BlockId, Reg, (Reg, Reg, Reg))>,
}

/// The whole-value reads of record signals in a process with a sensitivity list. Such a process
/// runs between two suspensions without a wait, so a signal has one value for the whole
/// activation: a signal the body reads as a whole in two places or more has a copy that the
/// process keeps (made once in the declarative part, filled at the start of each activation)
/// instead of reading every leaf again at each read. `iu3` of GRLIB passes its register record
/// to dozens of functions; each call read about a thousand leaves.
pub(crate) struct SigCopies {
    /// The block that ends the declarative part, and the block of an activation's start.
    pub decl_end: BlockId,
    pub top: BlockId,
    /// The process statement, which the instructions of the copies belong to.
    pub stmt: (yavs_ir::StmtId, Span),
    /// The record signals the body reads as a whole in two places or more (`Fx::whole_reads`).
    pub wanted: Vec<DeclId>,
    /// The copy of a signal read twice: the frame slot of its `[data, desc, handle]`.
    pub copies: Vec<(DeclId, u32)>,
}

impl<'a, 'b> Fx<'a, 'b> {
    pub fn new(
        db: &'a Db,
        cx: &'b mut Cx,
        fb: FuncBuilder<'b>,
        depth: u8,
        kind: FuncKind,
        name: String,
    ) -> Self {
        Fx {
            db,
            cx,
            fb,
            depth,
            kind,
            postponed: false,
            live: true,
            loops: Vec::new(),
            ret: None,
            name,
            drivers: Vec::new(),
            driver_base: Vec::new(),
            cur: None,
            cov: true,
            cov_inputs: std::collections::HashMap::new(),
            prot_self: None,
            prot_defaults: true,
            guard: None,
            writeback: Vec::new(),
            writeback_parts: Vec::new(),
            local_files: Vec::new(),
            sigcopy: None,
            made_dopes: Vec::new(),
        }
    }

    // ---- emission ----

    pub fn emit(&mut self, i: Inst) {
        if !self.live {
            // Code after a terminator (after `return`, say) goes to a fresh unreachable block.
            let b = self.fb.block();
            self.fb.at(b);
            self.live = true;
        }
        match i {
            Inst::MakeDope { at, parts } if parts.len == 3 => {
                if let Some(&[l, r, d]) = self.fb.list(parts) {
                    let here = self.fb.current();
                    self.made_dopes.retain(|e| e.0 == here);
                    self.made_dopes.push((here, at, (l, r, d)));
                }
            }
            // A register the bounds were read from may change.
            Inst::Move { .. } => self.made_dopes.clear(),
            _ => {}
        }
        self.fb.emit(i);
    }

    /// Emit a terminator; the current block is closed.
    pub fn term(&mut self, i: Inst) {
        self.emit(i);
        self.live = false;
    }

    pub fn jump(&mut self, to: BlockId) {
        self.term(Inst::Br { to });
    }

    pub fn branch(&mut self, c: Reg, t: BlockId, f: BlockId) {
        self.term(Inst::CondBr { c, t, f });
    }

    pub fn block(&mut self) -> BlockId {
        self.fb.block()
    }

    pub fn at(&mut self, b: BlockId) {
        self.fb.at(b);
        self.live = true;
    }

    pub fn reg(&mut self, t: RegTy) -> Reg {
        self.fb.reg(t)
    }

    pub fn int(&mut self, v: i64) -> Reg {
        let dst = self.reg(RegTy::I64);
        self.emit(Inst::Const { dst, v });
        dst
    }

    pub fn addr(&mut self, space: Space, off: u32) -> Reg {
        let space = match space {
            Space::Frame(d) if d == self.depth => AddrSpace::Frame,
            Space::Frame(d) => AddrSpace::Up(self.depth - d),
            Space::Inst => AddrSpace::Inst,
            Space::Pkg(p) => AddrSpace::Pkg(p),
            Space::Shared => AddrSpace::Shared,
            Space::Region(cell) => {
                // The kernel runs this function's processes serially (ir.md 3.5).
                self.fb.flag(yavs_ir::FuncFlags::TOUCHES_SHARED);
                let c = self.addr(Space::Inst, cell);
                let p = self.load(c, Width::B8, RegTy::Ptr);
                return self.offset(p, off);
            }
        };
        let dst = self.reg(RegTy::Ptr);
        self.emit(Inst::AddrOf { dst, space, off });
        dst
    }

    pub fn load(&mut self, addr: Reg, w: Width, t: RegTy) -> Reg {
        let dst = self.reg(t);
        self.emit(Inst::Load { dst, addr, w });
        dst
    }

    pub fn store(&mut self, addr: Reg, src: Reg, w: Width) {
        self.emit(Inst::Store { addr, src, w });
    }

    /// `base + n` bytes.
    pub fn offset(&mut self, base: Reg, n: u32) -> Reg {
        if n == 0 {
            return base;
        }
        let pos = self.int(i64::from(n));
        let dst = self.reg(RegTy::Ptr);
        self.emit(Inst::Index {
            dst,
            base,
            pos,
            stride: 1,
        });
        dst
    }

    pub fn alloc(&mut self, size: Reg) -> Reg {
        let dst = self.reg(RegTy::Ptr);
        self.emit(Inst::Alloc { dst, size });
        dst
    }

    pub fn alloc_n(&mut self, n: u32) -> Reg {
        let s = self.int(i64::from(n));
        self.alloc(s)
    }

    /// `size` bytes of temporary memory for a value of `tid` constrained by the descriptor
    /// at `desc`, recorded for the GC when such a value can hold handles ([`Fx::temp_root`]).
    pub fn alloc_val(&mut self, size: Reg, tid: TyId, desc: Option<Reg>) -> Reg {
        let dst = self.alloc(size);
        self.temp_root(dst, tid, desc);
        dst
    }

    /// Record the temporary value of `tid` at `addr` (descriptor at `desc`, which must be
    /// filled already) for the GC when values of `tid` can hold handles: the handles in it
    /// are roots until the statement ends (rule 6, #422).
    pub fn temp_root(&mut self, addr: Reg, tid: TyId, desc: Option<Reg>) {
        let types = self.fb.program().types();
        if Ty::has_handles(types, tid) {
            let ds = Ty::desc_size(types, tid);
            let desc = desc.filter(|_| ds.is_some_and(|n| n > 0));
            self.emit(Inst::TempRoot {
                addr,
                desc,
                ty: Some(tid),
            });
        }
    }

    /// A fat pair `[data, desc]` in temporary memory with its words stored: the heap objects
    /// they point into stay until the statement ends (rule 6, #422).
    pub fn fat_root(&mut self, fat: Reg) {
        self.emit(Inst::TempRoot {
            addr: fat,
            desc: None,
            ty: None,
        });
    }

    pub fn msg(&mut self, what: &str, dir: yavs_ir::Dir) -> yavs_ir::CheckMsgId {
        self.fb.program().check_msg(what, dir)
    }

    pub fn ty(&mut self, t: yavs_sem::TypeId, span: Span) -> R<TyId> {
        ty::lower(self.db, self.cx, self.fb.program(), t, span)
    }

    /// The IR type of a scalar's subtype and its register type and width.
    pub fn scalar(&mut self, st: SubtypeId, span: Span) -> R<(TyId, RegTy, Width)> {
        let t = self.db.base(st);
        let id = self.ty(t, span)?;
        let w = match self.fb.program().types().get(id) {
            Some(t) if t.scalar_size().is_some() => ty::width(t),
            _ => return Err(self.cx.unsupported(span, "composite values here")),
        };
        Ok((id, self.rt_of(t), w))
    }

    /// [`Fx::scalar`] for an array element of subtype `st`: its width is the element width
    /// ([`ty::elem_width`]).
    pub fn elem_scalar(&mut self, st: SubtypeId, span: Span) -> R<(TyId, RegTy, Width)> {
        let (id, rt, _) = self.scalar(st, span)?;
        let w = self
            .fb
            .program()
            .types()
            .get(id)
            .map_or(Width::B8, ty::elem_width);
        Ok((id, rt, w))
    }

    /// [`Fx::scalar`] for reading a scalar signal value the kernel wrote into memory: the
    /// kernel holds a narrow integer as 4 bytes when it is an array element and as 8 when it
    /// is a scalar, and a 4-byte load reads either (the value fits in 32 bits).
    pub fn sig_scalar(&mut self, st: SubtypeId, span: Span) -> R<(TyId, RegTy, Width)> {
        let (id, rt, w) = self.scalar(st, span)?;
        let ew = self.fb.program().types().get(id).map_or(w, ty::elem_width);
        Ok((id, rt, if ew == Width::B4 { ew } else { w }))
    }

    pub fn stmt_start(&mut self, kind: StmtKind, span: Span) -> yavs_ir::StmtId {
        if !self.live {
            let b = self.fb.block();
            self.fb.at(b);
        }
        let id = self.fb.stmt(kind, span);
        self.live = true;
        self.cur = Some((id, span));
        // Statement coverage counts the statements of the source, not declarations or the
        // markers of processes, subprograms and implicit waits.
        if self.cov
            && !matches!(
                kind,
                StmtKind::Decl | StmtKind::Process | StmtKind::Subprogram | StmtKind::Implicit
            )
            && let Some(item) =
                self.cx
                    .cover(self.fb.program(), id, span, yavs_ir::CovKind::Statement)
        {
            self.emit(Inst::Cover { item });
        }
        id
    }

    /// A counter for arm `arm` of branching statement `stmt` (the statement being lowered when
    /// the caller started it: arms lower nested statements).
    pub fn cover_branch(&mut self, stmt: Option<(yavs_ir::StmtId, Span)>, arm: u32) {
        let Some((stmt, span)) = stmt else {
            return;
        };
        if self.cov
            && let Some(item) =
                self.cx
                    .cover(self.fb.program(), stmt, span, yavs_ir::CovKind::Branch(arm))
        {
            self.emit(Inst::Cover { item });
        }
    }

    // ---- declarations ----

    /// Objects declared in `scope` (variables, constants), initialised in order.
    /// The record signals the statements `body` read as a whole in at least two places (a read
    /// in a loop counts twice): a name that selects a field, an element or an attribute reads
    /// less, and is no whole read of its prefix.
    fn whole_reads(&self, body: &[StmtId]) -> Vec<DeclId> {
        use yavs_sem::hir::{AttrPrefix, ExprKind};
        use yavs_sem::lsp::Use;
        fn walk(fx: &Fx<'_, '_>, body: &[StmtId], weight: u32, counts: &mut Vec<(DeclId, u32)>) {
            for &s in body {
                fx.db.stmt_exprs(s, &mut |e, u| {
                    if u == Use::Target {
                        return;
                    }
                    let mut parts: Vec<yavs_sem::ExprId> = Vec::new();
                    fx.db.walk_expr(e, &mut |x| match &fx.db.exprs[x].kind {
                        ExprKind::Field { prefix, .. }
                        | ExprKind::Index { prefix, .. }
                        | ExprKind::Slice { prefix, .. }
                        | ExprKind::Attr {
                            prefix: AttrPrefix::Expr(prefix),
                            ..
                        } => parts.push(*prefix),
                        ExprKind::Obj(d)
                            if !parts.contains(&x)
                                && matches!(fx.cx.objs.get(d), Some(Obj::RecSignal(_))) =>
                        {
                            match counts.iter_mut().find(|c| c.0 == *d) {
                                Some(c) => c.1 += weight,
                                None => counts.push((*d, weight)),
                            }
                        }
                        _ => {}
                    });
                });
                let w = if matches!(fx.db.stmts[s].kind, HStmt::Loop { .. }) {
                    2
                } else {
                    weight
                };
                walk(fx, &fx.db.stmt_children(s), w, counts);
            }
        }
        let mut counts = Vec::new();
        walk(self, body, 1, &mut counts);
        counts
            .into_iter()
            .filter(|c| c.1 >= 2)
            .map(|c| c.0)
            .collect()
    }

    pub fn decls(&mut self, scope: ScopeId) -> R<()> {
        let mut ok = true;
        for d in self.db.decls_with_instances(scope) {
            let decl = &self.db.decls[d];
            if let DeclKind::Alias(Alias::Object { target, subtype }) = &decl.kind {
                // A signal alias has no storage: its uses lower the aliased name.
                let signal = matches!(self.db.name_root(*target),
                    Some(yavs_sem::lsp::NameRoot::Obj(r)) if self.db.is_signal(r));
                // An alias of a whole external name is an object of the region
                // (`external_alias`).
                let region = self.db.external_alias(d).is_some() && self.cx.objs.contains_key(&d);
                if !signal && !region {
                    ok &= self.alias(d, *target, *subtype).is_ok();
                }
                continue;
            }
            if let DeclKind::Type(_) | DeclKind::Subtype(_) = decl.kind {
                ok &= self.declared_ranges(d).is_ok();
                continue;
            }
            let DeclKind::Object(o) = &decl.kind else {
                continue;
            };
            if matches!(
                o.role,
                ObjRole::Param | ObjRole::LoopParam | ObjRole::ImplicitSignal
            ) {
                continue;
            }
            let r = match o.class {
                ObjClass::Variable | ObjClass::Constant => self.local(d),
                ObjClass::SharedVariable => {
                    Err(self.cx.unsupported(decl.span, "shared variables (M3)"))
                }
                ObjClass::File => {
                    self.stmt_start(StmtKind::Decl, decl.span);
                    self.local_file(d)
                }
                ObjClass::Signal => Err(self.cx.ice(decl.span, "signal in a sequential region")),
            };
            ok &= r.is_ok();
        }
        if ok { Ok(()) } else { Err(Failed) }
    }

    /// The non-static ranges of type or subtype declaration `d` are evaluated here, once, into
    /// frame slots that every later use reads (`Fx::range`): a bound that reads a variable or
    /// calls an impure function keeps its value at the declaration (LRM 14.4.2.1, #612).
    fn declared_ranges(&mut self, d: DeclId) -> R<()> {
        let db = self.db;
        let ranges = db.declared_ranges(d);
        let span = db.decls[d].span;
        for rc in ranges {
            let RangeC::Explicit { left, right, .. } = rc else {
                continue;
            };
            if db.value(*left).is_some() && db.value(*right).is_some() {
                continue;
            }
            self.stmt_start(StmtKind::Decl, span);
            // A slot of an earlier lowering of this function is stale.
            self.cx.decl_ranges.remove(left);
            let r = self.range(rc, span)?;
            let off = self.fb.slot(16, 8);
            let a = self.addr(Space::Frame(self.depth), off);
            self.store(a, r.left, Width::B8);
            let b = self.offset(a, 8);
            self.store(b, r.right, Width::B8);
            self.cx
                .decl_ranges
                .insert(*left, (Space::Frame(self.depth), off));
        }
        Ok(())
    }

    /// An object alias (LRM 6.6.2): the target's storage seen under the alias's subtype.
    fn alias(&mut self, d: DeclId, target: yavs_sem::ExprId, subtype: Option<SubtypeId>) -> R<()> {
        let db = self.db;
        let span = db.decls[d].span;
        // An alias of a whole object without a new subtype is that object.
        if let (yavs_sem::hir::ExprKind::Obj(t), None) = (&db.exprs[target].kind, subtype)
            && let Some(o) = self.cx.objs.get(t).copied()
        {
            self.cx.objs.insert(d, o);
            return Ok(());
        }
        // A locally static constant has no slot (its folded value is used): the alias has no
        // storage either and its uses lower the aliased name (`alias_target`, #300).
        if let Some(yavs_sem::lsp::NameRoot::Obj(yavs_sem::hir::ObjRoot::Decl(t))) =
            db.name_root(target)
            && !self.cx.objs.contains_key(&t)
            && matches!(&db.decls[t].kind, DeclKind::Object(o) if o.class == ObjClass::Constant)
        {
            return Ok(());
        }
        self.stmt_start(StmtKind::Decl, span);
        let name = db.decls[d].name.to_string();
        match self.place(target)? {
            Place::Scalar { addr, w, .. } => {
                let off = self.fb.slot(8, 8);
                self.fb.root(off, RootKind::Addr);
                let a = self.addr(Space::Frame(self.depth), off);
                self.store(a, addr, Width::B8);
                self.cx.objs.insert(
                    d,
                    Obj::ScalarRef {
                        space: Space::Frame(self.depth),
                        off,
                        w,
                    },
                );
            }
            Place::Array {
                data,
                dope,
                off: None,
                tid,
                ..
            } => {
                // A subtype that constrains only the index (`arr(0 to n)` of an array of open
                // arrays) takes the element bounds from the aliased object.
                let want = match subtype {
                    Some(st) => match self.subtype_dope(st, span)? {
                        Some(w) => Some(w),
                        None => self.desc_of(st, Some(dope), span)?,
                    },
                    None => None,
                };
                let dope = match want {
                    Some(want) => {
                        let t = db.base(db.exprs[target].subtype);
                        self.check_desc(t, tid, dope, want, &ty::obj_desc(db, d), span)?;
                        want
                    }
                    None => dope,
                };
                // The whole descriptor: a record's or an element's bounds follow the
                // dimensions (#315).
                let bytes = self.desc_size(tid);
                let off = self.fb.local(&name, tid, 24 + bytes, 8);
                self.fb.root(off, RootKind::Addr);
                let slot = self.addr(Space::Frame(self.depth), off);
                self.store(slot, data, Width::B8);
                let dslot = self.offset(slot, 24);
                let n = self.int(i64::from(bytes));
                self.emit(Inst::Copy {
                    dst: dslot,
                    src: dope,
                    bytes: n,
                });
                let dp = self.offset(slot, 8);
                self.store(dp, dslot, Width::B8);
                self.cx.objs.insert(
                    d,
                    Obj::Array {
                        space: Space::Frame(self.depth),
                        off,
                    },
                );
            }
            // A logic element or slice is not addressable. When its name is static it denotes
            // the same part at every use, so the alias has no storage and its uses lower the
            // aliased name (`alias_target`, LRM 6.6.2), as for signals.
            _ if db.is_static_object_name(target) => {}
            _ => {
                return Err(self.cx.unsupported(
                    span,
                    "aliases of logic array elements and slices with a non-static name",
                ));
            }
        }
        Ok(())
    }

    fn local(&mut self, d: DeclId) -> R<()> {
        let db = self.db;
        let decl = &db.decls[d];
        let DeclKind::Object(o) = &decl.kind else {
            return Err(self.cx.ice(decl.span, "not an object"));
        };
        self.stmt_start(StmtKind::Decl, decl.span);
        let name = decl.name.to_string();
        let t = db.base(o.subtype);
        match &db.types[t].kind {
            yavs_sem::types::TypeKind::Protected { .. } => {
                return self.local_prot(d, t, decl.span);
            }
            yavs_sem::types::TypeKind::Access { .. } => return self.local_access(d),
            _ => {}
        }
        self.check_subtype_compat(o.subtype, decl.span)?;
        let tid = self.ty(t, decl.span)?;
        let shape = self.var_shape(d)?;
        let off = match shape {
            VarShape::Scalar(w) => self.fb.local(&name, tid, w.bytes(), w.bytes()),
            _ => {
                let off = self.fb.local(&name, tid, shape.size(), 8);
                self.fb.root(off + 16, RootKind::Handle);
                off
            }
        };
        let slot = self.addr(Space::Frame(self.depth), off);
        self.var_init(d, shape, slot, false)?;
        self.cx
            .objs
            .insert(d, shape.obj(Space::Frame(self.depth), off));
        Ok(())
    }

    /// How variable `d` (not of a protected or access type) is stored: its slot's shape.
    pub fn var_shape(&mut self, d: DeclId) -> R<VarShape> {
        let db = self.db;
        let decl = &db.decls[d];
        let DeclKind::Object(o) = &decl.kind else {
            return Err(self.cx.ice(decl.span, "not an object"));
        };
        let t = db.base(o.subtype);
        if let yavs_sem::types::TypeKind::Access { .. } = db.types[t].kind {
            return Ok(VarShape::Scalar(Width::B8)); // a shared variable's handle (#303)
        }
        if self.is_general(t, decl.span)? {
            return Ok(VarShape::Comp);
        }
        if ty::is_array(db, t) {
            let (_, dims) = ty::array_of(db, t).ok_or(Failed)?;
            return Ok(VarShape::Array(dims));
        }
        let (_, _, w) = self.scalar(o.subtype, decl.span)?;
        Ok(VarShape::Scalar(w))
    }

    /// The shape of non-protected shared variable `d` of type `t` and the IR type of the
    /// shared-heap block holding its slot in a region (`Space::Region`), for the GC (#303):
    /// `t` for a scalar, else a record whose third word is the handle of the variable's
    /// object (`[data, desc, handle]`, `[data, dope, handle, descriptor]`).
    pub fn shared_shape(
        &mut self,
        d: DeclId,
        t: yavs_sem::TypeId,
        span: Span,
    ) -> R<(VarShape, TyId)> {
        let tid = self.ty(t, span)?;
        let shape = self.var_shape(d)?;
        if let VarShape::Scalar(_) = shape {
            return Ok((shape, tid));
        }
        let pb = self.fb.program();
        let word = pb.ty(Ty::Int {
            lo: i64::MIN,
            hi: i64::MAX,
        });
        let handle = pb.ty(Ty::Access { target: tid });
        let field = |name: &str, ty| yavs_ir::Field {
            name: name.into(),
            ty,
            desc: None,
        };
        let slot = pb.ty(Ty::Record {
            fields: vec![
                field("data", word),
                field("desc", word),
                field("handle", handle),
            ],
        });
        Ok((shape, slot))
    }

    /// Initialise variable `d` in its slot at `slot` (`shape.size()` bytes): a scalar's value;
    /// for an array `[data, dope, handle, descriptor]` over a heap object (rule 6), for a
    /// record or an array of composites `[data, desc, handle]`. With `shared` the heap object
    /// is in the shared heap (shared variables, ir.md 3.5).
    pub fn var_init(&mut self, d: DeclId, shape: VarShape, slot: Reg, shared: bool) -> R<()> {
        let db = self.db;
        let decl = &db.decls[d];
        let DeclKind::Object(o) = &decl.kind else {
            return Err(self.cx.ice(decl.span, "not an object"));
        };
        let name = decl.name.to_string();
        let t = db.base(o.subtype);
        match shape {
            VarShape::Comp => self.comp_init(d, slot, shared),
            VarShape::Array(dims) => {
                let tid = self.ty(t, decl.span)?;
                let dope_bytes = 24 * u32::from(dims);
                let init = o.default.map(|e| self.expr(e)).transpose()?;
                let init = init.map(|v| self.arr(v, decl.span)).transpose()?;
                // Without an initial value, bounds known at run time are made in the
                // variable's own descriptor: no temporary and no copy (#771).
                let dslot = self.offset(slot, 24);
                let into = if init.is_none() && !shared {
                    Some(dslot)
                } else {
                    None
                };
                let dope = match (self.subtype_dope_in(o.subtype, decl.span, into)?, init) {
                    (Some(dope), Some((_, vd))) => {
                        let what = self.msg(&name, yavs_ir::Dir::To);
                        self.emit(Inst::SubtypeCheck {
                            dope: vd,
                            want: dope,
                            dims,
                            what,
                        });
                        dope
                    }
                    (Some(dope), None) => dope,
                    (None, Some((_, vd))) => vd,
                    (None, None) => {
                        return Err(self
                            .cx
                            .ice(decl.span, "unconstrained object without bounds"));
                    }
                };
                let (size, count) = self.layout(tid, dope);
                let h = self.reg(RegTy::Handle);
                if init.is_none() && self.zero_default(o.subtype, tid, decl.span)? {
                    // All-zero default: allocated in place, no temporary to copy from.
                    let z = self.bytes_addr(vec![0]);
                    self.emit(Inst::NewFill {
                        dst: h,
                        ty: tid,
                        desc: dope,
                        dsize: 0,
                        data: z,
                        size,
                        fill: 1,
                        shared,
                    });
                } else {
                    let data = match init {
                        Some((data, _)) => data,
                        None => {
                            self.default_array(o.subtype, (tid, dope), size, count, decl.span)?
                        }
                    };
                    self.emit(Inst::New {
                        dst: h,
                        ty: tid,
                        init: data,
                        size,
                        shared,
                    });
                }
                let hslot = self.offset(slot, 16);
                self.store(hslot, h, Width::B8);
                let what = self.msg(&name, yavs_ir::Dir::To);
                let p = self.reg(RegTy::Ptr);
                self.emit(Inst::Deref {
                    dst: p,
                    handle: h,
                    what,
                });
                self.store(slot, p, Width::B8);
                if dope != dslot {
                    let n = self.int(i64::from(dope_bytes));
                    self.emit(Inst::Copy {
                        dst: dslot,
                        src: dope,
                        bytes: n,
                    });
                }
                let dp = self.offset(slot, 8);
                self.store(dp, dslot, Width::B8);
                let c = &db.subtypes[o.subtype].constraint;
                if let Some(b) = crate::composite::static_desc(db, self.fb.program(), t, c, tid)
                    && b.len() == dope_bytes as usize
                {
                    let k = self.fb.program().konst(yavs_ir::ConstData::Bytes(b));
                    self.cx.const_dope.insert(d, k);
                }
                Ok(())
            }
            VarShape::Scalar(w) => {
                let access = matches!(db.types[t].kind, yavs_sem::types::TypeKind::Access { .. });
                let v = match o.default {
                    Some(e) if access => self.scalar_expr(e)?,
                    None if access => self.int(0),
                    Some(e) => {
                        let v = self.scalar_expr(e)?;
                        let what = ty::obj_desc(db, d);
                        self.check_scalar(
                            v,
                            Some(db.exprs[e].subtype),
                            o.subtype,
                            &what,
                            decl.span,
                        )?;
                        v
                    }
                    None => self.scalar_left(o.subtype, decl.span)?,
                };
                self.store(slot, v, w);
                Ok(())
            }
        }
    }

    /// `size` bytes of `count` elements, each the element subtype's left bound.
    fn default_array(
        &mut self,
        st: SubtypeId,
        (tid, dope): (TyId, Reg),
        size: Reg,
        count: Reg,
        span: Span,
    ) -> R<Reg> {
        let db = self.db;
        let (elem, _) = ty::array_of(db, db.base(st)).ok_or(Failed)?;
        let data = self.alloc_val(size, tid, Some(dope));
        // 2019 arrays of a protected type: one new object per element (LCS-2016-014).
        let pt = db.base(elem);
        if matches!(
            db.types[pt].kind,
            yavs_sem::types::TypeKind::Protected { .. }
        ) {
            self.for_n(count, |fx, i| {
                let h = fx.prot_new(pt, false, span)?;
                let eight = fx.int(8);
                let off = fx.bin(yavs_ir::IntOp::Mul, i, eight);
                let a = fx.bin(yavs_ir::IntOp::Add, data, off);
                fx.store(a, h, Width::B8);
                Ok(())
            })?;
            return Ok(data);
        }
        let left = db
            .static_bounds(elem)
            .and_then(|(l, _, _)| ty::cell(&l))
            .filter(|&c| c == 0);
        let etid = self.ty(db.base(elem), span)?;
        let logic = yavs_ir::Ty::logic_leaf(self.fb.program().types(), etid).is_some();
        let bias = self.code_bias(etid);
        if left.is_some() && bias == 0 {
            return Ok(data); // temporaries are zeroed
        }
        if logic {
            let v = self.scalar_left(elem, span)?;
            let code = self.enc(v, bias);
            let zero = self.int(0);
            self.emit(Inst::LFill {
                base: data,
                idx: zero,
                n: count,
                v: code,
            });
            return Ok(data);
        }
        let (_, _, w) = self.elem_scalar(elem, span)?;
        let v = self.scalar_left(elem, span)?;
        let e = self.alloc_n(8);
        self.store(e, v, w);
        self.emit(Inst::Fill {
            dst: data,
            elem: e,
            n: count,
            size: w.bytes(),
        });
        Ok(data)
    }

    /// Size and element count of an array of type `tid` with descriptor `dope`.
    pub fn layout(&mut self, tid: TyId, dope: Reg) -> (Reg, Reg) {
        let l = self.reg(RegTy::Ptr);
        self.emit(Inst::LayoutOf {
            dst: l,
            ty: tid,
            desc: dope,
            cache: None,
        });
        let size = self.load(l, Width::B8, RegTy::I64);
        // A record layout has its field offsets where an array has its element count (and may
        // be shorter), so its count is 0.
        if !matches!(
            self.fb.program().types().get(tid),
            Some(yavs_ir::Ty::Array { .. })
        ) {
            let count = self.int(0);
            return (size, count);
        }
        let count = self.layout_count(tid, dope);
        (size, count)
    }

    /// The element count of an array of type `tid` with descriptor `dope` (the fourth word of
    /// its layout), computed from the lengths in the descriptor: the product of the lengths of
    /// the dimensions, times the element count of a logic array's elements (all leaves are in
    /// one plane vector). No `LayoutOf`: a layout is a run-time walk of the descriptor, and
    /// the same count asked inside a loop is a few cheap instructions the optimiser can share
    /// and fold.
    pub fn layout_count(&mut self, tid: TyId, dope: Reg) -> Reg {
        let types = self.fb.program().types();
        let Some(&yavs_ir::Ty::Array {
            elem,
            elem_desc,
            dims,
        }) = types.get(tid)
        else {
            return self.layout(tid, dope).1;
        };
        let nested = yavs_ir::Ty::logic_leaf(types, tid).is_some()
            && matches!(types.get(elem), Some(yavs_ir::Ty::Array { .. }));
        let mut count = self.dope_len(dope);
        for dim in 1..dims {
            let dst = self.reg(RegTy::I64);
            self.emit(Inst::DopeLen { dst, dope, dim });
            count = self.bin(yavs_ir::IntOp::Mul, count, dst);
        }
        if nested {
            let ed = match elem_desc {
                Some(c) => self.const_addr(c),
                None => self.offset(dope, 24 * u32::from(dims)),
            };
            let m = self.layout_count(elem, ed);
            count = self.bin(yavs_ir::IntOp::Mul, count, m);
        }
        count
    }

    /// Return `v`, closing the files this subprogram declared first.
    pub fn ret_term(&mut self, v: Option<Reg>) {
        for (off, sym, tid) in std::mem::take(&mut self.local_files) {
            let a = self.addr(crate::Space::Frame(self.depth), off);
            // A failure was reported by `native`; the return goes on.
            let _ = self.native(&sym, Some(tid), &[a], None, Span::default());
            self.local_files.push((off, sym, tid));
        }
        self.term(Inst::Ret { v });
    }

    /// Finish the body: a function falling off its end is a runtime error.
    pub fn end(mut self, span: Span) {
        if self.live {
            if self.kind == FuncKind::Function {
                let what = format!("function `{}` ended without a return statement", self.name);
                let what = self.msg(&what, yavs_ir::Dir::To);
                let _ = span;
                self.term(Inst::Unreachable { what });
            } else {
                self.ret_term(None);
            }
        }
        self.fb.finish();
    }
}

/// Lower subprogram `d` (declared by `func_of`).
pub(crate) fn subprogram(
    db: &Db,
    pb: &mut ProgramBuilder,
    cx: &mut Cx,
    d: DeclId,
    depth: u8,
) -> R<()> {
    let decl = &db.decls[d];
    let Some(sp) = decl.kind.subprogram() else {
        return Err(cx.ice(decl.span, "not a subprogram"));
    };
    let Body::Hir { scope, stmts } = &sp.body else {
        return Err(cx.ice(decl.span, "subprogram without a body"));
    };
    let Some(&f) = cx.funcs.get(&d) else {
        return Err(cx.ice(decl.span, "subprogram was not declared"));
    };
    cx.scope_func.insert(*scope, (f, depth));
    let kind = crate::func_kind(db, d);
    // A subprogram declared in a process (at any depth) runs only in that process and drives
    // through its driver slots (LRM 14.7.2).
    let mut s = Some(decl.scope);
    let mut drivers = None;
    while let Some(sc) = s {
        if let Some(d) = cx.proc_drivers.get(&sc) {
            drivers = Some(d.clone());
            break;
        }
        s = db.scopes[sc].parent;
    }
    let mut fx = Fx::new(db, cx, pb.define(f), depth, kind, decl.name.to_string());
    if let Some((d, b)) = drivers {
        fx.drivers = d;
        fx.driver_base = b;
    }
    if let SubprogramKind::Function { pure: true } = sp.kind {
        fx.fb.flag(FuncFlags::PURE);
    }
    fx.ret = sp.ret;
    if let (SubprogramKind::Function { .. }, Some(r)) = (&sp.kind, sp.ret) {
        let t = fx.ty(db.base(r), decl.span)?;
        fx.fb.ret_ty(t);
    }
    fx.stmt_start(StmtKind::Subprogram, decl.span);
    fx.method_prologue(d, *scope, depth, decl.span)?;
    // The body's own parameter declarations (a body completing a specification has its own
    // region, sem.md 1.3), in order.
    let body_params: Vec<DeclId> = db.scopes[*scope]
        .order
        .iter()
        .copied()
        .filter(|&d| matches!(&db.decls[d].kind, DeclKind::Object(o) if o.role == ObjRole::Param))
        .collect();
    if body_params.len() != sp.params.len() {
        return Err(fx
            .cx
            .ice(decl.span, "parameter declarations do not match the profile"));
    }
    for (p, &pd) in sp.params.iter().zip(&body_params) {
        let sh = shape(db, fx.cx, p, decl.span)?;
        let name = p.name.text.to_string();
        let mode = match p.mode {
            HMode::In => Mode::In,
            HMode::Out => Mode::Out,
            _ => Mode::Inout,
        };
        let obj = match sh {
            Shape::Val => {
                // The 8-byte cell holds the value: little-endian, so debuggers read it as the type.
                let (t, _, _) = fx.scalar(p.subtype, decl.span)?;
                let off = fx
                    .fb
                    .param(&name, Width::B8, ParamClass::Value, mode, Some(t));
                if fx.rt_of(db.base(p.subtype)) == RegTy::Handle {
                    fx.fb.root(off, RootKind::Handle);
                }
                Obj::Scalar {
                    space: Space::Frame(depth),
                    off,
                    w: Width::B8,
                }
            }
            Shape::Ref => {
                let (_, _, w) = fx.scalar(p.subtype, decl.span)?;
                let off = fx.fb.param(&name, Width::B8, ParamClass::Value, mode, None);
                fx.fb.root(off, RootKind::Addr);
                Obj::ScalarRef {
                    space: Space::Frame(depth),
                    off,
                    w,
                }
            }
            Shape::Sig => {
                let off = fx
                    .fb
                    .param(&name, Width::Sig, ParamClass::Signal, mode, None);
                Obj::SignalRef {
                    space: Space::Frame(depth),
                    off,
                    out: false,
                }
            }
            Shape::SigOut => {
                let off = fx
                    .fb
                    .param(&name, Width::Sig, ParamClass::Signal, mode, None);
                let slot = fx.fb.param(
                    &format!("{name}'driver"),
                    Width::B8,
                    ParamClass::Value,
                    Mode::In,
                    None,
                );
                if slot != off + 16 {
                    return Err(fx.cx.ice(decl.span, "signal parameter slots not adjacent"));
                }
                Obj::SignalRef {
                    space: Space::Frame(depth),
                    off,
                    out: true,
                }
            }
            Shape::RecSig(out) => {
                let leaves = ty::leaves(db, p.subtype);
                let mut at = Vec::new();
                for k in 0..leaves.len() {
                    let n = format!("{name}'{k}");
                    at.push(fx.fb.param(&n, Width::Sig, ParamClass::Signal, mode, None));
                }
                if out {
                    let n = format!("{name}'driver");
                    at.push(
                        fx.fb
                            .param(&n, Width::B8, ParamClass::Value, Mode::In, None),
                    );
                }
                let t = fx.ty(db.base(p.subtype), decl.span)?;
                let desc = yavs_ir::Ty::needs_desc(fx.fb.program().types(), t);
                if desc {
                    let n = format!("{name}'dope");
                    at.push(
                        fx.fb
                            .param(&n, Width::B8, ParamClass::Value, Mode::In, None),
                    );
                }
                let Some(&off) = at.first() else {
                    return Err(fx
                        .cx
                        .ice(decl.span, "record signal parameter without leaves"));
                };
                let want = (0..leaves.len() as u32).map(|k| off + 16 * k);
                let tail =
                    (0..(out as u32 + desc as u32)).map(|k| off + 16 * leaves.len() as u32 + 8 * k);
                if !at.iter().copied().eq(want.chain(tail)) {
                    return Err(fx.cx.ice(decl.span, "signal parameter slots not adjacent"));
                }
                let rec = leaves
                    .into_iter()
                    .enumerate()
                    .map(|(k, (path, st))| crate::RecLeaf {
                        path,
                        st,
                        sig: k as u32,
                    })
                    .collect::<Vec<_>>();
                let param = crate::RecParam {
                    space: Space::Frame(depth),
                    off,
                    leaves: rec.len() as u32,
                    out,
                    desc,
                };
                fx.cx.rec_leaves.insert(pd, rec);
                Obj::RecSignal(Some(param))
            }
            Shape::Arr => {
                let off = fx.fb.param(&name, Width::B8, ParamClass::Value, mode, None);
                let dope = fx.fb.param(
                    &format!("{name}'dope"),
                    Width::B8,
                    ParamClass::Value,
                    mode,
                    None,
                );
                if dope != off + 8 {
                    return Err(fx.cx.ice(decl.span, "array parameter slots not adjacent"));
                }
                let t = fx.ty(db.base(p.subtype), decl.span)?;
                fx.fb.param_array(off, t);
                fx.fb.root(off, RootKind::Addr);
                fx.fb.root(dope, RootKind::Addr);
                Obj::Array {
                    space: Space::Frame(depth),
                    off,
                }
            }
        };
        if let Obj::SignalRef { off, out, .. } = obj
            && ty::is_array(db, db.base(p.subtype))
        {
            // An array signal parameter: the actual's descriptor follows (DESIGN.md).
            let dope = fx.fb.param(
                &format!("{name}'dope"),
                Width::B8,
                ParamClass::Value,
                Mode::In,
                None,
            );
            if dope != off + if out { 24 } else { 16 } {
                return Err(fx.cx.ice(decl.span, "signal parameter slots not adjacent"));
            }
        }
        fx.cx.objs.insert(pd, obj);
    }
    // A named return value (2019): the caller passes the result subtype's descriptor last.
    if let Some(&(hidden, _)) = db.ret_names.get(scope) {
        let name = db.decls[hidden].name.to_string();
        let off = fx
            .fb
            .param(&name, Width::B8, ParamClass::Value, Mode::In, None);
        let dope = fx.fb.param(
            &format!("{name}'dope"),
            Width::B8,
            ParamClass::Value,
            Mode::In,
            None,
        );
        if dope != off + 8 {
            return Err(fx.cx.ice(decl.span, "array parameter slots not adjacent"));
        }
        fx.cx.objs.insert(
            hidden,
            Obj::Array {
                space: Space::Frame(depth),
                off,
            },
        );
    }
    // An external name it uses that is not bound makes the call fail (LRM 8.7: the name is
    // evaluated when the subprogram runs, #255).
    let unbound = db
        .external_uses(yavs_sem::external::ExtOwner::Sub(d))
        .iter()
        .find_map(|&h| match &fx.cx.ext_unbound {
            Some(m) => m.get(&h).cloned(),
            None => Some(format!(
                "`{}` uses external name `{}`, which is bound only once the design hierarchy \
                 is elaborated: it cannot be called during elaboration",
                decl.name, db.decls[h].name
            )),
        });
    if let Some(m) = unbound {
        fx.cx.ext_early |= fx.cx.ext_unbound.is_none();
        if fx.cx.ext_unbound.is_none() {
            fx.cx.context_funcs.push(fx.fb.id());
        }
        let what = fx.msg(&m, yavs_ir::Dir::To);
        fx.term(Inst::Unreachable { what });
        let b = fx.block();
        fx.at(b);
    }
    let decls = fx.decls(*scope);
    let body = fx.stmts(stmts);
    fx.end(decl.span);
    decls.and(body)
}

/// Lower an architecture and the processes of its statement part.
pub(crate) fn architecture(
    db: &Db,
    pb: &mut ProgramBuilder,
    cx: &mut Cx,
    unit: UnitId,
) -> R<UnitIr> {
    let u = &db.units[unit];
    let UnitHir::Architecture { entity, stmts } = &u.hir else {
        return Err(cx.ice(u.span, "not an architecture"));
    };
    let e = &db.units[*entity];
    let mut iface: Vec<DeclId> = Vec::new();
    let mut estmts: Vec<StmtId> = Vec::new();
    if let UnitHir::Entity {
        generics,
        ports,
        stmts: es,
    } = &e.hir
    {
        estmts = es.to_vec();
        for &g in generics.iter() {
            if !matches!(db.decls[g].kind, DeclKind::Object(_)) {
                return Err(
                    cx.unsupported(db.decls[g].span, "generic types, subprograms and packages")
                );
            }
        }
        iface.extend(generics.iter().chain(ports.iter()));
    }
    let mut ir = UnitIr {
        unit,
        inst_size: 0,
        values: Vec::new(),
        consts: Vec::new(),
        // Package signals come first, at the same index in every unit.
        signals: cx.pkg_sigs.clone(),
        processes: Vec::new(),
        names: Vec::new(),
        var_ports: Vec::new(),
        ext_vars: Vec::new(),
        ranges: Vec::new(),
    };
    cx.inst_size = 0;
    cx.names.clear();
    cx.arch_scopes = vec![e.scope, u.scope];
    let mut ok = true;
    let scopes = [e.scope, u.scope];
    let decls = iface.iter().copied().chain(
        scopes
            .iter()
            .flat_map(|&s| db.decls_with_instances(s))
            .filter(|d| !iface.contains(d)),
    );
    for d in decls {
        ok &= inst_object(db, pb, cx, &mut ir, d).is_ok();
    }
    // The entity's statements (LRM 3.2.4) come first, as in the elaborator's walk.
    estmts.extend(stmts.iter().copied());
    ok &= region(db, pb, cx, &mut ir, &estmts, None).is_ok();
    // Subprograms declared in the architecture use its instance data too ('path_name).
    while let Some((d, depth)) = cx.queue.pop() {
        ok &= subprogram(db, pb, cx, d, depth).is_ok();
    }
    ir.inst_size = cx.inst_size;
    ir.names = std::mem::take(&mut cx.names);
    if ok { Ok(ir) } else { Err(Failed) }
}

/// The concurrent statements of a region: processes and the equivalent processes of
/// concurrent statements, recursing into blocks and generates (whose declarations and
/// generate parameters become instance data too; yavs-elab builds one instance data per
/// generate iteration). Component and entity instances belong to the elaborator.
fn region(
    db: &Db,
    pb: &mut ProgramBuilder,
    cx: &mut Cx,
    ir: &mut UnitIr,
    stmts: &[StmtId],
    outer: Option<DeclId>,
) -> R<()> {
    let mut ok = true;
    let decls =
        |cx: &mut Cx, pb: &mut ProgramBuilder, ir: &mut UnitIr, scope: yavs_sem::ScopeId| {
            let mut ok = true;
            for d in db.decls_with_instances(scope) {
                ok &= inst_object(db, pb, cx, ir, d).is_ok();
            }
            ok
        };
    for &s in stmts {
        let st = &db.stmts[s];
        match &st.kind {
            HStmt::Instance {
                port_map, target, ..
            } => {
                ok &= actuals(db, pb, cx, ir, s, port_map, Some(target)).is_ok();
            }
            HStmt::Block {
                scope,
                guard,
                generics,
                ports,
                port_map,
                body,
                ..
            } => {
                ok &= actuals(db, pb, cx, ir, s, port_map, None).is_ok();
                // Block generics and ports are values and signals of the block's region.
                for &d in generics.iter().chain(ports.iter()) {
                    ok &= inst_object(db, pb, cx, ir, d).is_ok();
                }
                ok &= decls(cx, pb, ir, *scope);
                if let Some(g) = guard {
                    ok &= guard_process(db, pb, cx, ir, s, *g).is_ok();
                }
                ok &= region(db, pb, cx, ir, body, (*guard).or(outer)).is_ok();
            }
            HStmt::ForGenerate {
                param, scope, body, ..
            } => {
                ok &= inst_object(db, pb, cx, ir, *param).is_ok();
                ok &= decls(cx, pb, ir, *scope);
                ok &= region(db, pb, cx, ir, body, outer).is_ok();
            }
            HStmt::IfGenerate { alts } => {
                for a in alts.iter() {
                    ok &= decls(cx, pb, ir, a.scope);
                    ok &= region(db, pb, cx, ir, &a.body, outer).is_ok();
                }
            }
            HStmt::CaseGenerate { alts, .. } => {
                for (_, a) in alts.iter() {
                    ok &= decls(cx, pb, ir, a.scope);
                    ok &= region(db, pb, cx, ir, &a.body, outer).is_ok();
                }
            }
            HStmt::Process { scope, .. } => {
                // Implicit signals named in the process (or a subprogram of it) belong to the
                // enclosing region (LRM 16.2.4): instance signals bound by the elaborator.
                for &d in &db.scopes[*scope].order {
                    if let DeclKind::Object(o) = &db.decls[d].kind
                        && o.role == ObjRole::ImplicitSignal
                    {
                        ok &= inst_object(db, pb, cx, ir, d).is_ok();
                    }
                }
                ok &= process(db, pb, cx, ir, s, outer).is_ok();
            }
            _ => ok &= process(db, pb, cx, ir, s, outer).is_ok(),
        }
    }
    if ok { Ok(()) } else { Err(Failed) }
}

/// An alias `d` of external name `x` (declared, or implicit for a direct use, #93), which the
/// elaborator binds to the external name's target (LRM 8.7): for a signal an instance
/// signal of the region (one per leaf with records), for a constant an instance value, for a
/// shared variable an instance cell holding its object. Uses of the alias and LSPs rooted at
/// `x` go through it.
fn external_alias(
    db: &Db,
    pb: &mut ProgramBuilder,
    cx: &mut Cx,
    ir: &mut UnitIr,
    d: DeclId,
    x: yavs_sem::ExternalId,
) -> R<()> {
    use yavs_syntax::ast::ExternalClass;
    let decl = &db.decls[d];
    let name = decl.name.to_string();
    let st = db.externals[x].subtype;
    let t = db.base(st);
    let tid = ty::lower(db, cx, pb, t, decl.span)?;
    cx.ext_sigs.insert(x, d);
    match db.externals[x].class {
        ExternalClass::Constant => {
            let comp = matches!(
                db.types[t].kind,
                yavs_sem::types::TypeKind::Array { .. } | yavs_sem::types::TypeKind::Record { .. }
            );
            let off = cx.inst_alloc(if comp { 16 } else { 8 });
            ir.values.push(InstValue {
                decl: d,
                name,
                off,
                ty: tid,
            });
            let w = pb.types().get(tid).map_or(Width::B8, ty::width);
            let obj = if comp {
                Obj::InstArray {
                    space: Space::Inst,
                    off,
                }
            } else {
                Obj::Scalar {
                    space: Space::Inst,
                    off,
                    w,
                }
            };
            cx.objs.insert(d, obj);
            return Ok(());
        }
        ExternalClass::Variable => {
            if !matches!(
                db.types[t].kind,
                yavs_sem::types::TypeKind::Protected { .. }
            ) {
                return Err(cx.unsupported(decl.span, "external variables of non-protected types"));
            }
            let off = cx.inst_alloc(8);
            ir.ext_vars.push(InstValue {
                decl: d,
                name,
                off,
                ty: tid,
            });
            cx.objs.insert(
                d,
                Obj::Scalar {
                    space: Space::Inst,
                    off,
                    w: Width::B8,
                },
            );
            return Ok(());
        }
        ExternalClass::Signal => {}
    }
    let dope = if yavs_ir::Ty::needs_desc(pb.types(), tid) {
        let Some(size) = yavs_ir::Ty::desc_size(pb.types(), tid) else {
            return Err(cx.ice(decl.span, "composite type without a descriptor size"));
        };
        let off = cx.inst_alloc(size as u32);
        cx.sig_dopes.insert(d, (Space::Inst, off));
        Some((off, tid))
    } else {
        None
    };
    if ty::has_record(db, t) {
        // One instance signal per leaf of the target, as for a declared record signal.
        let mut dope = dope;
        let mut out = Vec::new();
        for (path, lst) in ty::leaves(db, st) {
            let lt = ty::lower(db, cx, pb, db.base(lst), decl.span)?;
            let sig = ir.signals.len() as u32;
            ir.signals.push(SignalIr {
                decl: d,
                name: name.clone(),
                ty: lt,
                init: None,
                path: path.clone(),
                dope: dope.take(),
            });
            out.push(crate::RecLeaf { path, st: lst, sig });
        }
        cx.rec_leaves.insert(d, out);
        cx.objs.insert(d, Obj::RecSignal(None));
        return Ok(());
    }
    cx.objs.insert(
        d,
        Obj::Signal {
            sig: ir.signals.len() as u32,
        },
    );
    ir.signals.push(SignalIr {
        decl: d,
        name,
        ty: tid,
        init: None,
        path: Vec::new(),
        dope,
    });
    Ok(())
}

/// The external aliases declaration `d` brings: itself when it aliases a whole external name,
/// the implicit aliases of an alias of a part of one (#93), those of a subprogram or of the
/// methods of a protected body (#255).
fn decl_externals(db: &Db, d: DeclId) -> Vec<(DeclId, yavs_sem::ExternalId)> {
    use yavs_sem::external::ExtOwner;
    if let Some(x) = db.external_alias(d) {
        return vec![(d, x)];
    }
    let mut owners: Vec<ExtOwner> = db
        .decl_subprograms(d)
        .into_iter()
        .map(ExtOwner::Sub)
        .collect();
    owners.push(ExtOwner::Alias(d));
    owners
        .into_iter()
        .flat_map(|o| db.external_uses(o))
        .filter_map(|&h| Some((h, db.external_alias(h)?)))
        .collect()
}

/// A thunk evaluating one cell (an initial value), for the elaborator.
pub(crate) fn thunk(
    db: &Db,
    pb: &mut ProgramBuilder,
    cx: &mut Cx,
    name: &str,
    span: Span,
    f: impl FnOnce(&mut Fx<'_, '_>) -> R<Reg>,
) -> R<yavs_ir::FuncId> {
    let id = pb.declare(name, FuncKind::Thunk, None);
    let mut fx = Fx::new(
        db,
        cx,
        pb.define(id),
        0,
        FuncKind::Function,
        name.to_string(),
    );
    fx.stmt_start(StmtKind::Decl, span);
    let v = f(&mut fx);
    if let Ok(v) = v {
        fx.term(Inst::Ret { v: Some(v) });
    }
    fx.end(span);
    v.map(|_| id)
}

/// The thunk that creates the object of shared variable `d` in the shared heap and
/// returns its handle (protected) or the address of its slot (`Lowerer::region_var_init`).
pub(crate) fn region_var_init(
    db: &Db,
    pb: &mut ProgramBuilder,
    cx: &mut Cx,
    d: DeclId,
) -> R<yavs_ir::FuncId> {
    let decl = &db.decls[d];
    let DeclKind::Object(o) = &decl.kind else {
        return Err(cx.ice(decl.span, "not an object"));
    };
    let (t, span, name) = (db.base(o.subtype), decl.span, decl.name.to_string());
    let protected = matches!(
        db.types[t].kind,
        yavs_sem::types::TypeKind::Protected { .. }
    );
    thunk(db, pb, cx, &name, span, |fx| {
        if protected {
            fx.prot_new(t, true, span)
        } else {
            Ok(fx.new_shared_slot(d, t, &name, span)?.1)
        }
    })
}

fn inst_object(db: &Db, pb: &mut ProgramBuilder, cx: &mut Cx, ir: &mut UnitIr, d: DeclId) -> R<()> {
    let decl = &db.decls[d];
    if let DeclKind::Type(_) | DeclKind::Subtype(_) = decl.kind {
        for left in db
            .declared_ranges(d)
            .into_iter()
            .filter_map(|r| db.dynamic_range(r))
        {
            let off = cx.inst_alloc(16);
            cx.decl_ranges.insert(left, (Space::Inst, off));
            ir.ranges.push(crate::InstRange {
                left,
                off,
                scope: elab_region(db, decl.scope),
            });
        }
    }
    if let DeclKind::Alias(_) | DeclKind::Subprogram(_) | DeclKind::Type(_) = decl.kind {
        for (h, x) in decl_externals(db, d) {
            external_alias(db, pb, cx, ir, h, x)?;
        }
        return Ok(());
    }
    let DeclKind::Object(o) = &decl.kind else {
        return Ok(());
    };
    let name = decl.name.to_string();
    let array = ty::is_array(db, db.base(o.subtype));
    match o.class {
        ObjClass::Signal if ty::has_record(db, db.base(o.subtype)) => {
            // One kernel signal per leaf (field), in leaf order; an array of records has one
            // per field over all its elements. The descriptor of the whole signal is in
            // instance data (bounds from the initial value or the actual, rule 1).
            let whole = ty::lower(db, cx, pb, db.base(o.subtype), decl.span)?;
            let mut dope = if yavs_ir::Ty::needs_desc(pb.types(), whole) {
                let Some(size) = yavs_ir::Ty::desc_size(pb.types(), whole) else {
                    return Err(cx.ice(decl.span, "composite type without a descriptor size"));
                };
                let off = cx.inst_alloc(size as u32);
                cx.sig_dopes.insert(d, (Space::Inst, off));
                Some((off, whole))
            } else {
                None
            };
            let mut out = Vec::new();
            for (path, st) in ty::leaves(db, o.subtype) {
                let t = ty::lower(db, cx, pb, db.base(st), decl.span)?;
                let sig = ir.signals.len() as u32;
                ir.signals.push(SignalIr {
                    decl: d,
                    name: name.clone(),
                    ty: t,
                    init: None,
                    path: path.clone(),
                    dope: dope.take(),
                });
                out.push(crate::RecLeaf { path, st, sig });
            }
            cx.rec_leaves.insert(d, out);
            cx.objs.insert(d, Obj::RecSignal(None));
            Ok(())
        }
        ObjClass::Signal => {
            let t = ty::lower(db, cx, pb, db.base(o.subtype), decl.span)?;
            let dope = if array {
                let Some(size) = yavs_ir::Ty::desc_size(pb.types(), t) else {
                    return Err(cx.ice(decl.span, "array type without a descriptor size"));
                };
                let off = cx.inst_alloc(size as u32);
                cx.sig_dopes.insert(d, (Space::Inst, off));
                Some((off, t))
            } else {
                None
            };
            let (st, def) = (o.subtype, o.default);
            let init = if array
                || matches!(
                    o.role,
                    ObjRole::Port | ObjRole::Guard | ObjRole::ImplicitSignal
                ) {
                None
            } else {
                Some(thunk(
                    db,
                    pb,
                    cx,
                    &format!("{name}'init"),
                    decl.span,
                    |fx| match def {
                        Some(e) => {
                            let v = fx.scalar_expr(e)?;
                            let what = ty::obj_desc(db, d);
                            fx.check_scalar(v, Some(db.exprs[e].subtype), st, &what, decl.span)?;
                            Ok(v)
                        }
                        None => fx.scalar_left(st, decl.span),
                    },
                )?)
            };
            cx.objs.insert(
                d,
                Obj::Signal {
                    sig: ir.signals.len() as u32,
                },
            );
            ir.signals.push(SignalIr {
                decl: d,
                name,
                ty: t,
                init,
                path: Vec::new(),
                dope,
            });
            Ok(())
        }
        ObjClass::Constant => {
            // A locally static constant is read through its folded initialiser (`read_obj`).
            let given = matches!(o.role, ObjRole::Generic | ObjRole::GenerateParam);
            if !given && o.default.is_some_and(|e| db.value(e).is_some()) {
                return Ok(());
            }
            let t = ty::lower(db, cx, pb, db.base(o.subtype), decl.span)?;
            let comp = matches!(
                db.types[db.base(o.subtype)].kind,
                yavs_sem::types::TypeKind::Array { .. } | yavs_sem::types::TypeKind::Record { .. }
            );
            let off = cx.inst_alloc(if comp { 16 } else { 8 });
            ir.values.push(InstValue {
                decl: d,
                name: name.clone(),
                off,
                ty: t,
            });
            if comp {
                cx.objs.insert(
                    d,
                    Obj::InstArray {
                        space: Space::Inst,
                        off,
                    },
                );
                // The bounds of an array constant of a static subtype are the same in every
                // instance: its descriptor is a constant of the pool, which the optimiser reads
                // (loop bounds and directions, positions), not a cell of the instance's data.
                if matches!(pb.types().get(t), Some(yavs_ir::Ty::Array { .. }))
                    && let Some(b) = crate::composite::static_desc(
                        db,
                        pb,
                        db.base(o.subtype),
                        &db.subtypes[o.subtype].constraint,
                        t,
                    )
                    && yavs_ir::Ty::desc_size(pb.types(), t) == Some(b.len() as u64)
                    && !b.is_empty()
                {
                    let k = pb.konst(yavs_ir::ConstData::Bytes(b));
                    cx.const_dope.insert(d, k);
                }
                // A composite constant elaboration did not fold is computed once per region
                // by a thunk returning the address of its fat pair (#73); one initialised from
                // an external name always has its elaborated value.
                if let (Some(def), false) = (o.default, given)
                    && !db.uses_external(def)
                {
                    let (st, span) = (o.subtype, decl.span);
                    let init = thunk(db, pb, cx, &name, span, |fx| {
                        let (data, desc, _) = fx.const_comp(d, st, def, span)?;
                        let fat = fx.alloc_n(16);
                        fx.store(fat, data, Width::B8);
                        let dp = fx.offset(fat, 8);
                        fx.store(dp, desc, Width::B8);
                        fx.fat_root(fat);
                        Ok(fat)
                    })?;
                    ir.consts.push(InstConst {
                        decl: d,
                        name,
                        off,
                        w: Width::B8,
                        comp: Some(t),
                        init,
                        scope: elab_region(db, decl.scope),
                        instance_level: cx.arch_scopes.contains(&decl.scope),
                    });
                }
                return Ok(());
            }
            let w = pb.types().get(t).map_or(Width::B8, ty::width);
            cx.objs.insert(
                d,
                Obj::Scalar {
                    space: Space::Inst,
                    off,
                    w,
                },
            );
            // Scalar constants the elaborator cannot fold are computed by a thunk.
            if let (Some(def), false) = (o.default, given)
                && !db.uses_external(def)
            {
                let st = o.subtype;
                let init = thunk(db, pb, cx, &name, decl.span, |fx| {
                    let v = fx.scalar_expr(def)?;
                    let what = ty::obj_desc(db, d);
                    fx.check_scalar(v, Some(db.exprs[def].subtype), st, &what, decl.span)?;
                    Ok(v)
                })?;
                ir.consts.push(InstConst {
                    decl: d,
                    name,
                    off,
                    w,
                    comp: None,
                    init,
                    scope: elab_region(db, decl.scope),
                    instance_level: cx.arch_scopes.contains(&decl.scope),
                });
            }
            Ok(())
        }
        ObjClass::SharedVariable => crate::protected::shared_var(db, pb, cx, ir, d),
        // A file of an architecture: a cell in the shared heap (instance data is read-only
        // while processes run), made and opened by a thunk at elaboration; its instance cell
        // holds the address (`Space::Region`, serial processes). One per instance only: a
        // block or generate would need one per declaring region (#249).
        ObjClass::File if cx.arch_scopes.contains(&decl.scope) => {
            let off = cx.inst_alloc(8);
            cx.objs.insert(
                d,
                Obj::Scalar {
                    space: Space::Region(off),
                    off: 0,
                    w: Width::B4,
                },
            );
            let init = thunk(db, pb, cx, &name, decl.span, |fx| fx.file_region(d))?;
            ir.consts.push(InstConst {
                decl: d,
                name,
                off,
                w: Width::B8,
                comp: None,
                init,
                scope: elab_region(db, decl.scope),
                instance_level: true,
            });
            Ok(())
        }
        // A 2019 variable port: a cell holding the handle of its actual's protected object,
        // which the elaborator writes (`UnitIr::var_ports`, #188).
        ObjClass::Variable if o.role == ObjRole::Port => {
            let off = cx.inst_alloc(8);
            cx.objs.insert(
                d,
                Obj::Scalar {
                    space: Space::Inst,
                    off,
                    w: Width::B8,
                },
            );
            ir.var_ports.push((d, off));
            Ok(())
        }
        _ => Err(cx.unsupported(
            decl.span,
            "shared variables and files in an architecture (M3)",
        )),
    }
}

/// A process, or the equivalent process of a concurrent statement (LRM 11.3 to 11.6).
fn process(
    db: &Db,
    pb: &mut ProgramBuilder,
    cx: &mut Cx,
    ir: &mut UnitIr,
    s: StmtId,
    mut guard: Option<DeclId>,
) -> R<()> {
    let st = &db.stmts[s];
    let (postponed, info, scope, implicit) = match &st.kind {
        HStmt::Process {
            postponed,
            info,
            scope,
            ..
        } => (*postponed, info, Some(*scope), false),
        HStmt::ConcSignalAssign {
            postponed,
            guarded,
            guard: assign_guard,
            info,
            ..
        } => {
            // The GUARD signal sem found at the statement: the block's own, or a signal
            // declared with that name (11.6).
            if assign_guard.is_some() {
                guard = *assign_guard;
            }
            if *guarded && guard.is_none() {
                return Err(cx.ice(st.span, "guarded assignment outside a guarded block"));
            }
            (*postponed, info, None, true)
        }
        HStmt::ConcAssert {
            postponed, info, ..
        }
        | HStmt::ConcProcCall {
            postponed, info, ..
        } => (*postponed, info, None, true),
        _ => {
            return Err(cx.unsupported(
                st.span,
                "blocks, instances and generate statements (M2 slice 3)",
            ));
        }
    };
    let label = st.label.map(|l| l.text.to_string());
    let name = label
        .clone()
        .unwrap_or_else(|| format!("_p{}", ir.processes.len()));
    let f = pb.declare(&name, FuncKind::Process, None);
    if let Some(sc) = scope {
        cx.scope_func.insert(sc, (f, 0));
    }
    // Aliases of external names declared in the process, and the implicit aliases of those
    // its statements use directly, belong to its region.
    if let Some(sc) = scope {
        for &d in &db.scopes[sc].order {
            for (h, x) in decl_externals(db, d) {
                external_alias(db, pb, cx, ir, h, x)?;
            }
        }
    }
    for &h in db.external_uses(yavs_sem::external::ExtOwner::Stmt(s)) {
        let x = db.implicit_external(h).ok_or(Failed)?;
        external_alias(db, pb, cx, ir, h, x)?;
    }
    // Driver slots: per longest static prefix, in `ProcessInfo::drivers` order, one per kernel
    // signal it covers (one, or the leaves of a record signal below its fields).
    let mut sigs = Vec::new();
    let mut bases = Vec::new();
    for l in &info.drivers {
        let Some(d) = cx.root_decl(db, l.root) else {
            return Err(cx.unsupported(l.span, "drivers of external names and implicit signals"));
        };
        bases.push(sigs.len() as u32);
        match cx.objs.get(&d) {
            Some(&Obj::Signal { sig }) => sigs.push(sig),
            Some(Obj::RecSignal(_)) => {
                let path = lsp_fields(l);
                let leaves = cx.rec_leaves.get(&d).cloned().unwrap_or_default();
                sigs.extend(
                    leaves
                        .iter()
                        .filter(|x| x.path.starts_with(&path))
                        .map(|x| x.sig),
                );
            }
            _ => {
                return Err(cx.unsupported(l.span, "drivers of signals outside the architecture"));
            }
        }
    }
    let drivers = info.drivers.clone();
    if let Some(sc) = scope {
        cx.proc_drivers.insert(sc, (drivers.clone(), bases.clone()));
    }
    let mut fx = Fx::new(db, cx, pb.define(f), 0, FuncKind::Process, name.clone());
    if implicit {
        fx.fb.flag(FuncFlags::IMPLICIT);
    }
    fx.drivers = drivers;
    fx.driver_base = bases;
    fx.guard = guard;
    fx.postponed = postponed;
    let pstmt = fx.stmt_start(StmtKind::Process, st.span);
    let mut r = Ok(());
    if let Some(sc) = scope {
        r = fx.decls(sc);
    }
    let top = fx.block();
    if scope.is_some() {
        fx.fb.body(top);
    }
    let decl_end = fx.fb.current();
    fx.jump(top);
    fx.at(top);
    if matches!(
        &st.kind,
        HStmt::Process {
            sensitivity: Sensitivity::List(_) | Sensitivity::All,
            ..
        }
    ) {
        fx.fb.flag(FuncFlags::SENS_LIST);
        if let (Some(_), HStmt::Process { body, .. }) = (scope, &st.kind) {
            let wanted = fx.whole_reads(body);
            if !wanted.is_empty() {
                fx.sigcopy = Some(SigCopies {
                    decl_end,
                    top,
                    stmt: (pstmt, st.span),
                    wanted,
                    copies: Vec::new(),
                });
            }
        }
    }
    let body = match &st.kind {
        HStmt::Process { body, .. } => fx.stmts(body),
        _ => fx.stmt(s),
    };
    let sens: Vec<hir::Lsp> = match &st.kind {
        HStmt::Process { sensitivity, .. } => match sensitivity {
            Sensitivity::None => Vec::new(),
            Sensitivity::List(l) => l.to_vec(),
            Sensitivity::All => info.reads_all.clone().unwrap_or_default(),
        },
        _ => info.reads_all.clone().unwrap_or_default(),
    };
    let has_wait = !matches!(
        &st.kind,
        HStmt::Process {
            sensitivity: Sensitivity::None,
            ..
        }
    );
    let w = if has_wait {
        fx.stmt_start(StmtKind::Implicit, st.span);
        fx.wait_on(&sens)
            .map(|()| fx.emit(Inst::Suspend { resume: 0 }))
    } else {
        Ok(())
    };
    fx.jump(top);
    fx.end(st.span);
    r.and(body).and(w)?;
    ir.processes.push(ProcessIr {
        stmt: s,
        func: f,
        label,
        postponed,
        drivers: sigs,
        actual: None,
    });
    Ok(())
}

/// The equivalent processes `formal <= expr` of the expression actuals of instance (or
/// block) statement `s` that the elaborator asked for (`Cx::actuals`, LRM 6.5.6.3), from its
/// port map `map` or from the binding indication of component instance `target`. An actual
/// of a binding indication reads the component's ports and generics: they become objects of
/// this unit, which the elaborator binds in the component's region.
fn actuals(
    db: &Db,
    pb: &mut ProgramBuilder,
    cx: &mut Cx,
    ir: &mut UnitIr,
    s: StmtId,
    map: &[hir::Arg],
    target: Option<&hir::InstTarget>,
) -> R<()> {
    let mut asked: Vec<_> = cx
        .actuals
        .iter()
        .filter(|(_, a)| a.0 == s)
        .map(|(&e, &(_, formal, port))| (e, formal, port))
        .collect();
    asked.sort_by_key(|a| a.0);
    let own = |e: yavs_sem::ExprId| {
        map.iter().any(|a| match &a.actual {
            hir::Actual::Expr(x) => *x == e,
            hir::Actual::Individual(parts) => parts.iter().any(|(_, x, ..)| *x == e),
            _ => false,
        })
    };
    let mut ok = true;
    if let Some(hir::InstTarget::Component(c)) = target
        && asked.iter().any(|a| !own(a.0))
        && let DeclKind::Component(comp) = &db.decls[*c].kind
    {
        for &d in comp.generics.iter().chain(comp.ports.iter()) {
            let known = ir.signals.iter().any(|x| x.decl == d)
                || ir.values.iter().any(|x| x.decl == d)
                || ir.consts.iter().any(|x| x.decl == d);
            if !known {
                ok &= inst_object(db, pb, cx, ir, d).is_ok();
            }
        }
    }
    for (e, formal, port) in asked {
        ok &= actual_process(db, pb, cx, ir, s, e, (formal, port)).is_ok();
    }
    if ok { Ok(()) } else { Err(Failed) }
}

fn actual_process(
    db: &Db,
    pb: &mut ProgramBuilder,
    cx: &mut Cx,
    ir: &mut UnitIr,
    s: StmtId,
    e: yavs_sem::ExprId,
    (formal, port): (Option<SubtypeId>, DeclId),
) -> R<()> {
    let span = db.exprs[e].span;
    let base = db.base(db.exprs[e].subtype);
    let t = ty::lower(db, cx, pb, base, span)?;
    // Arrays and composites with records that need one get the formal's descriptor.
    let dope = if ty::is_array(db, base)
        || (ty::has_record(db, base) && yavs_ir::Ty::needs_desc(pb.types(), t))
    {
        let Some(size) = yavs_ir::Ty::desc_size(pb.types(), t) else {
            return Err(cx.ice(span, "array type without a descriptor size"));
        };
        Some(cx.inst_alloc(size as u32))
    } else {
        None
    };
    for &h in db.external_uses(yavs_sem::external::ExtOwner::Expr(e)) {
        let x = db.implicit_external(h).ok_or(Failed)?;
        external_alias(db, pb, cx, ir, h, x)?;
    }
    // A scalar formal whose range is not static (the instance's generics): the elaborator
    // writes `[left, right, dir]` here.
    let bounds = match formal {
        Some(f) if db.static_bounds(f).is_none() && db.types[db.base(f)].kind.is_scalar() => {
            Some(cx.inst_alloc(24))
        }
        _ => None,
    };
    let name = "_actual".to_string();
    let f = pb.declare(&name, FuncKind::Process, None);
    let mut fx = Fx::new(db, cx, pb.define(f), 0, FuncKind::Process, name);
    fx.fb.flag(FuncFlags::IMPLICIT);
    fx.cov = false;
    fx.stmt_start(StmtKind::Process, span);
    let top = fx.block();
    fx.jump(top);
    fx.at(top);
    fx.stmt_start(StmtKind::SigAssign, span);
    let r = fx.drive_actual(e, (formal, port), dope, bounds, span);
    fx.stmt_start(StmtKind::Implicit, span);
    let mut sens = Vec::new();
    fx.signals_in(e, &mut sens);
    let w = fx
        .wait_on(&sens)
        .map(|()| fx.emit(Inst::Suspend { resume: 0 }));
    fx.jump(top);
    fx.end(span);
    r.and(w)?;
    ir.processes.push(ProcessIr {
        stmt: s,
        func: f,
        label: None,
        postponed: false,
        drivers: Vec::new(),
        actual: Some(crate::PortActual {
            expr: e,
            ty: t,
            dope,
            bounds,
        }),
    });
    Ok(())
}

/// The guard process of a guarded block, `GUARD <= guard_condition` sensitive to the
/// condition's signals (LRM 11.2); yavs-elab names it `_guard` and gives it GUARD's driver.
fn guard_process(
    db: &Db,
    pb: &mut ProgramBuilder,
    cx: &mut Cx,
    ir: &mut UnitIr,
    s: StmtId,
    g: DeclId,
) -> R<()> {
    let st = &db.stmts[s];
    let DeclKind::Object(o) = &db.decls[g].kind else {
        return Err(cx.ice(st.span, "GUARD is not an object"));
    };
    let Some(cond) = o.default else {
        return Err(cx.ice(st.span, "GUARD without its condition"));
    };
    let Some(&Obj::Signal { sig }) = cx.objs.get(&g) else {
        return Err(cx.ice(st.span, "GUARD was not lowered as a signal"));
    };
    let f = pb.declare("_guard", FuncKind::Process, None);
    let mut fx = Fx::new(db, cx, pb.define(f), 0, FuncKind::Process, "_guard".into());
    fx.fb.flag(FuncFlags::IMPLICIT);
    fx.cov = false;
    let lsp = hir::Lsp {
        root: hir::ObjRoot::Decl(g),
        path: Box::new([]),
        stat: yavs_sem::stat::Staticness::Locally,
        span: st.span,
    };
    fx.drivers = vec![lsp];
    fx.stmt_start(StmtKind::Process, st.span);
    let top = fx.block();
    fx.jump(top);
    fx.at(top);
    fx.stmt_start(StmtKind::SigAssign, db.exprs[cond].span);
    let r = fx.drive_guard(g, cond, st.span);
    fx.stmt_start(StmtKind::Implicit, st.span);
    let mut sens = Vec::new();
    fx.signals_in(cond, &mut sens);
    let w = fx
        .wait_on(&sens)
        .map(|()| fx.emit(Inst::Suspend { resume: 0 }));
    fx.jump(top);
    fx.end(st.span);
    r.and(w)?;
    ir.processes.push(ProcessIr {
        stmt: s,
        func: f,
        label: Some("_guard".into()),
        postponed: false,
        drivers: vec![sig],
        actual: None,
    });
    Ok(())
}

/// The run-time objects of a package and its body (LRM 14.4.2): signals, and an initialisation
/// thunk that evaluates the constants that are not locally static once and creates the shared
/// variables, in declaration order, in package data `AddrSpace::Pkg(index)`.
pub(crate) fn package(
    db: &Db,
    pb: &mut ProgramBuilder,
    cx: &mut Cx,
    unit: UnitId,
    index: u32,
) -> R<crate::PackageIr> {
    let u = &db.units[unit];
    let Some(scopes) = db.package_regions(unit) else {
        return Err(cx.ice(u.span, "not a package with objects"));
    };
    let name = format!("{}'init", u.name.text);
    let f = pb.declare(&name, FuncKind::Thunk, None);
    let mut fx = Fx::new(db, cx, pb.define(f), 0, FuncKind::Thunk, name);
    fx.stmt_start(StmtKind::Decl, u.span);
    let mut size = 0u32;
    let mut signals = Vec::new();
    let mut ok = true;
    for &s in &scopes {
        for d in db.decls_with_instances(s) {
            ok &= fx.pkg_object(d, index, &mut size, &mut signals).is_ok();
        }
    }
    // The aliases of the external names its subprograms use (#255).
    let (mut ext_consts, mut ext_vars) = (Vec::new(), Vec::new());
    for &s in &scopes {
        for d in db.decls_with_instances(s) {
            for m in db.decl_subprograms(d) {
                for &h in db.external_uses(yavs_sem::external::ExtOwner::Sub(m)) {
                    ok &= fx
                        .pkg_external(
                            h,
                            index,
                            &mut size,
                            &mut signals,
                            &mut ext_consts,
                            &mut ext_vars,
                        )
                        .is_ok();
                }
            }
        }
    }
    fx.end(u.span);
    if !ok {
        return Err(Failed);
    }
    Ok(crate::PackageIr {
        unit,
        index,
        size,
        init: f,
        signals,
        ext_consts,
        ext_vars,
    })
}

impl Fx<'_, '_> {
    /// One package object (see [`package`]); `size` is the package data allocated so far.
    fn pkg_object(
        &mut self,
        d: DeclId,
        p: u32,
        size: &mut u32,
        signals: &mut Vec<SignalIr>,
    ) -> R<()> {
        let db = self.db;
        let decl = &db.decls[d];
        if let DeclKind::Type(_) | DeclKind::Subtype(_) = decl.kind {
            return self.pkg_ranges(d, p, size);
        }
        let DeclKind::Object(o) = &decl.kind else {
            return Ok(());
        };
        if self.cx.objs.contains_key(&d) {
            return Ok(()); // a deferred constant seen again in the body
        }
        let span = decl.span;
        let t = db.base(o.subtype);
        let mut alloc = |n: u32| {
            let off = size.next_multiple_of(8);
            *size = off + n;
            off
        };
        match o.class {
            ObjClass::Signal => self.pkg_signal(d, o.subtype, p, size, signals),
            ObjClass::Constant => {
                // A deferred constant without its body, and locally static constants (read
                // through their folded value), have no slot.
                let Some(def) = o.default else { return Ok(()) };
                if db.value(def).is_some() {
                    return Ok(());
                }
                self.stmt_start(StmtKind::Decl, span);
                if self.is_comp(t) {
                    let tid = self.ty(t, span)?;
                    let Some(desc) = self.static_desc(o.subtype, def) else {
                        // Bounds known only at run time: the elements go to package data
                        // appended at initialisation, the fat pair and descriptor in place.
                        let n = yavs_ir::Ty::desc_size(self.fb.program().types(), tid)
                            .ok_or_else(|| self.cx.ice(span, "composite without a descriptor"))?;
                        let (data, vdesc, bytes) = self.const_comp(d, o.subtype, def, span)?;
                        let off = alloc(16 + n as u32);
                        let slot = self.addr(Space::Pkg(p), off);
                        let dst = self.reg(RegTy::Ptr);
                        self.emit(Inst::PkgAlloc { dst, size: bytes });
                        self.emit(Inst::Copy {
                            dst,
                            src: data,
                            bytes,
                        });
                        self.store(slot, dst, Width::B8);
                        let ds = self.offset(slot, 16);
                        let nb = self.int(n as i64);
                        self.emit(Inst::Copy {
                            dst: ds,
                            src: vdesc,
                            bytes: nb,
                        });
                        let dp = self.offset(slot, 8);
                        self.store(dp, ds, Width::B8);
                        self.cx.objs.insert(
                            d,
                            Obj::Array {
                                space: Space::Pkg(p),
                                off,
                            },
                        );
                        return Ok(());
                    };
                    let lsize = match self.fb.program().layout(tid, &desc) {
                        Ok((l, _)) => l.size as u32,
                        Err(e) => return Err(self.cx.ice(span, &e.to_string())),
                    };
                    let v = self.expr(def)?;
                    let (data, vdope) = self.arr(v, span)?;
                    let off = alloc(lsize.next_multiple_of(8) + desc.len() as u32);
                    let dslot = off + lsize.next_multiple_of(8);
                    let c = self
                        .fb
                        .program()
                        .konst(yavs_ir::ConstData::Bytes(desc.clone()));
                    let da = self.addr(Space::Pkg(p), dslot);
                    if !desc.is_empty() {
                        self.emit(Inst::CopyConst { dst: da, c });
                        if ty::is_array(db, t) {
                            let dims = ty::array_of(db, t).map_or(1, |x| x.1);
                            let what = self.msg(&ty::obj_desc(db, d), yavs_ir::Dir::To);
                            self.emit(Inst::SubtypeCheck {
                                dope: vdope,
                                want: da,
                                dims,
                                what,
                            });
                        }
                    }
                    let dst = self.addr(Space::Pkg(p), off);
                    let n = self.int(i64::from(lsize));
                    self.emit(Inst::Copy {
                        dst,
                        src: data,
                        bytes: n,
                    });
                    self.cx.objs.insert(
                        d,
                        Obj::Fixed {
                            space: Space::Pkg(p),
                            off,
                            size: lsize.next_multiple_of(8),
                        },
                    );
                    return Ok(());
                }
                let (_, _, w) = self.scalar(o.subtype, span)?;
                let v = self.scalar_expr(def)?;
                let what = ty::obj_desc(db, d);
                self.check_scalar(v, Some(db.exprs[def].subtype), o.subtype, &what, span)?;
                let off = alloc(8);
                let a = self.addr(Space::Pkg(p), off);
                self.store(a, v, w);
                self.cx.objs.insert(
                    d,
                    Obj::Scalar {
                        space: Space::Pkg(p),
                        off,
                        w,
                    },
                );
                Ok(())
            }
            ObjClass::SharedVariable => {
                if !matches!(
                    db.types[t].kind,
                    yavs_sem::types::TypeKind::Protected { .. }
                ) {
                    // `--relaxed` (W1904): one object in the shared variables area; the
                    // functions naming it touch shared state, so their processes are serial.
                    let (shape, _) = self.shared_shape(d, t, span)?;
                    self.stmt_start(StmtKind::Decl, span);
                    let off = self.cx.shared_size.next_multiple_of(8);
                    self.cx.shared_size = off + shape.size();
                    // The GC root: an access value, or the handle of the variable's object.
                    let root = match shape {
                        VarShape::Scalar(_) => {
                            matches!(db.types[t].kind, yavs_sem::types::TypeKind::Access { .. })
                                .then_some(off)
                        }
                        _ => Some(off + 16),
                    };
                    if let Some(r) = root {
                        self.fb.program().shared_root(r, RootKind::Handle);
                    }
                    let slot = self.addr(Space::Shared, off);
                    self.var_init(d, shape, slot, true)?;
                    self.cx.objs.insert(d, shape.obj(Space::Shared, off));
                    let ty = self.ty(t, span)?;
                    self.cx.shared_vars.push((d, ty));
                    return Ok(());
                }
                self.stmt_start(StmtKind::Decl, span);
                let h = self.prot_new(t, true, span)?;
                let off = alloc(8);
                let a = self.addr(Space::Pkg(p), off);
                self.store(a, h, Width::B8);
                self.cx.objs.insert(
                    d,
                    Obj::Scalar {
                        space: Space::Pkg(p),
                        off,
                        w: Width::B8,
                    },
                );
                Ok(())
            }
            ObjClass::File => {
                // A file of a package is one object for every process: its cell lives in the
                // shared variables area (serial processes, ir.md 3.5), opened here when its
                // declaration says so.
                let off = self.cx.shared_size.next_multiple_of(8);
                self.cx.shared_size = off + 8;
                self.cx.objs.insert(
                    d,
                    Obj::Scalar {
                        space: Space::Shared,
                        off,
                        w: Width::B4,
                    },
                );
                if o.file_open.is_some() {
                    self.stmt_start(StmtKind::Decl, span);
                    self.open_file(d, Space::Shared, off)?;
                }
                Ok(())
            }
            _ => Ok(()),
        }
    }

    /// The ranges of package type or subtype `d` with a bound that is not globally static,
    /// evaluated once into package data, one after the other (`Lowerer::package_range` reads
    /// them for the elaborator), for every use to read (LRM 14.4.2.1, #612).
    fn pkg_ranges(&mut self, d: DeclId, p: u32, size: &mut u32) -> R<()> {
        let db = self.db;
        let span = db.decls[d].span;
        let ranges: Vec<_> = db
            .declared_ranges(d)
            .into_iter()
            .filter(|r| db.dynamic_range(r).is_some())
            .collect();
        let mut off = size.next_multiple_of(8);
        *size = off + 16 * ranges.len() as u32;
        for rc in ranges {
            let RangeC::Explicit { left, .. } = rc else {
                continue;
            };
            self.stmt_start(StmtKind::Decl, span);
            self.cx.decl_ranges.remove(left);
            let r = self.range(rc, span)?;
            let a = self.addr(Space::Pkg(p), off);
            self.store(a, r.left, Width::B8);
            let b = self.offset(a, 8);
            self.store(b, r.right, Width::B8);
            self.cx.decl_ranges.insert(*left, (Space::Pkg(p), off));
            off += 16;
        }
        Ok(())
    }

    /// A package signal `d` of subtype `st` (a declared signal, or the alias of an external
    /// signal name in a subprogram, #255): one kernel signal per leaf, an array's or a
    /// composite's descriptor in package data.
    fn pkg_signal(
        &mut self,
        d: DeclId,
        st: SubtypeId,
        p: u32,
        size: &mut u32,
        signals: &mut Vec<SignalIr>,
    ) -> R<()> {
        let db = self.db;
        let decl = &db.decls[d];
        let span = decl.span;
        let t = db.base(st);
        let mut alloc = |n: u32| {
            let off = size.next_multiple_of(8);
            *size = off + n;
            off
        };
        if ty::has_record(db, t) {
            // One kernel signal per leaf, as in an architecture (`inst_object`); the whole
            // signal's descriptor in package data on the first leaf entry.
            let whole = self.ty(t, span)?;
            let types = self.fb.program().types();
            let mut dope = if yavs_ir::Ty::needs_desc(types, whole) {
                let n = yavs_ir::Ty::desc_size(types, whole)
                    .ok_or_else(|| self.cx.ice(span, "composite without a descriptor"))?;
                let off = alloc(n as u32);
                self.cx.sig_dopes.insert(d, (Space::Pkg(p), off));
                Some((off, whole))
            } else {
                None
            };
            let mut out = Vec::new();
            for (path, lst) in ty::leaves(db, st) {
                let ty = self.ty(db.base(lst), span)?;
                let sig = self.cx.pkg_sigs.len() as u32;
                let s = SignalIr {
                    decl: d,
                    name: decl.name.to_string(),
                    ty,
                    init: None,
                    path: path.clone(),
                    dope: None,
                };
                self.cx.pkg_sigs.push(s.clone());
                signals.push(SignalIr {
                    dope: dope.take(),
                    ..s
                });
                out.push(crate::RecLeaf { path, st: lst, sig });
            }
            self.cx.rec_leaves.insert(d, out);
            self.cx.objs.insert(d, Obj::RecSignal(None));
            return Ok(());
        }
        let ty = self.ty(t, span)?;
        let sig = self.cx.pkg_sigs.len() as u32;
        let s = SignalIr {
            decl: d,
            name: decl.name.to_string(),
            ty,
            init: None,
            path: Vec::new(),
            dope: None,
        };
        self.cx.pkg_sigs.push(s.clone());
        // An array signal's descriptor lives in package data, written by the elaborator from
        // the signal's bounds (a 2019 signal may take them from its initial value, LRM
        // 6.4.2.3).
        let dope = if ty::is_array(db, t) {
            let n = yavs_ir::Ty::desc_size(self.fb.program().types(), ty)
                .ok_or_else(|| self.cx.ice(span, "array type without a descriptor size"))?;
            let off = alloc(n as u32);
            self.cx.sig_dopes.insert(d, (Space::Pkg(p), off));
            Some((off, ty))
        } else {
            None
        };
        signals.push(SignalIr { dope, ..s });
        self.cx.objs.insert(d, Obj::Signal { sig });
        Ok(())
    }

    /// Alias `h` of an external name used in a subprogram of package `p` (#255): a signal is a
    /// package signal; a constant's value and a shared variable's object are cells in the
    /// shared variables area, written by the elaborator once the design is elaborated
    /// (package data is complete before the regions holding the targets exist).
    // ponytail: processes reading them run serially (ir.md 3.5); package data written after
    // the regions are built would keep them parallel.
    fn pkg_external(
        &mut self,
        h: DeclId,
        p: u32,
        size: &mut u32,
        signals: &mut Vec<SignalIr>,
        consts: &mut Vec<InstValue>,
        vars: &mut Vec<InstValue>,
    ) -> R<()> {
        use yavs_syntax::ast::ExternalClass;
        let db = self.db;
        if self.cx.objs.contains_key(&h) {
            return Ok(());
        }
        let span = db.decls[h].span;
        let Some(x) = db.external_alias(h) else {
            return Err(self.cx.ice(span, "not an external alias"));
        };
        let st = db.externals[x].subtype;
        let t = db.base(st);
        let name = db.decls[h].name.to_string();
        self.cx.ext_sigs.insert(x, h);
        let shared = |cx: &mut Cx, n: u32| {
            let off = cx.shared_size.next_multiple_of(8);
            cx.shared_size = off + n;
            off
        };
        match db.externals[x].class {
            ExternalClass::Signal => self.pkg_signal(h, st, p, size, signals),
            ExternalClass::Constant => {
                let ty = self.ty(t, span)?;
                let comp = matches!(
                    db.types[t].kind,
                    yavs_sem::types::TypeKind::Array { .. }
                        | yavs_sem::types::TypeKind::Record { .. }
                );
                let off = shared(self.cx, if comp { 16 } else { 8 });
                let obj = if comp {
                    Obj::InstArray {
                        space: Space::Shared,
                        off,
                    }
                } else {
                    let w = self
                        .fb
                        .program()
                        .types()
                        .get(ty)
                        .map_or(Width::B8, ty::width);
                    Obj::Scalar {
                        space: Space::Shared,
                        off,
                        w,
                    }
                };
                consts.push(InstValue {
                    decl: h,
                    name,
                    off,
                    ty,
                });
                self.cx.objs.insert(h, obj);
                Ok(())
            }
            ExternalClass::Variable => {
                if !matches!(
                    db.types[t].kind,
                    yavs_sem::types::TypeKind::Protected { .. }
                ) {
                    return Err(self
                        .cx
                        .unsupported(span, "external variables of non-protected types"));
                }
                let ty = self.ty(t, span)?;
                let off = shared(self.cx, 8);
                vars.push(InstValue {
                    decl: h,
                    name,
                    off,
                    ty,
                });
                self.cx.objs.insert(
                    h,
                    Obj::Scalar {
                        space: Space::Shared,
                        off,
                        w: Width::B8,
                    },
                );
                Ok(())
            }
        }
    }

    /// The descriptor of a composite subtype when it is known at lowering: none needed (a
    /// record of constrained fields), or static index bounds (from the subtype, else from a
    /// folded initial value).
    /// The value of composite constant `d` (subtype `st`, initialiser `def`) converted to its
    /// subtype: elements, descriptor and the elements' size in bytes.
    pub(crate) fn const_comp(
        &mut self,
        d: DeclId,
        st: SubtypeId,
        def: yavs_sem::ExprId,
        span: Span,
    ) -> R<(Reg, Reg, Reg)> {
        let db = self.db;
        let t = db.base(st);
        let tid = self.ty(t, span)?;
        let v = self.expr(def)?;
        let (data, vd) = self.arr(v, span)?;
        let Some(desc) = self.desc_of(st, Some(vd), span)? else {
            return Err(self.cx.ice(span, "unconstrained constant without bounds"));
        };
        self.check_desc(t, tid, vd, desc, &ty::obj_desc(db, d), span)?;
        let size = self.size_of(tid, desc);
        Ok((data, desc, size))
    }

    fn static_desc(&mut self, st: SubtypeId, def: yavs_sem::ExprId) -> Option<Vec<u8>> {
        let db = self.db;
        let tid = ty::lower(
            db,
            self.cx,
            self.fb.program(),
            db.base(st),
            db.exprs[def].span,
        )
        .ok()?;
        let n = yavs_ir::Ty::desc_size(self.fb.program().types(), tid)?;
        if n == 0 {
            return Some(Vec::new());
        }
        let mut out = Vec::new();
        if let yavs_sem::types::Constraint::Array {
            dims: Some(dims), ..
        } = &db.subtypes[st].constraint
        {
            for rc in dims.iter() {
                let yavs_sem::types::RangeC::Explicit { left, dir, right } = rc else {
                    return None;
                };
                let (l, r) = (db.value(*left)?.as_int()?, db.value(*right)?.as_int()?);
                let d = i64::from(*dir == yavs_sem::types::Direction::Downto);
                for x in [l, r, d] {
                    out.extend_from_slice(&x.to_le_bytes());
                }
            }
        }
        (out.len() as u64 == n).then_some(out)
    }
}

/// The field selections of a longest static prefix: the field path of the leaves it covers
/// (indices select elements, not leaves).
pub(crate) fn lsp_fields(l: &hir::Lsp) -> Vec<u32> {
    l.path
        .iter()
        .filter_map(|s| match s {
            hir::LspStep::Field(n) => Some(*n),
            _ => None,
        })
        .collect()
}

/// The block or generate (or entity or architecture) whose elaboration creates a constant
/// declared in `scope`: the nearest such region around it, `None` when there is none.
pub(crate) fn elab_region(db: &Db, scope: yavs_sem::ScopeId) -> Option<yavs_sem::ScopeId> {
    use yavs_sem::scope::ScopeKind as K;
    let mut cur = Some(scope);
    while let Some(s) = cur {
        if matches!(
            db.scopes[s].kind,
            K::Entity | K::Architecture | K::Block | K::Generate
        ) {
            return Some(s);
        }
        cur = db.scopes[s].parent;
    }
    None
}
