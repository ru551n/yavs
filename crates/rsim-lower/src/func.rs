//! Functions: frames, parameters, declarations, subprogram and process bodies, architectures.

use rsim_diag::Span;
use rsim_ir::{
    AddrSpace, BlockId, FuncBuilder, FuncFlags, FuncId, FuncKind, Inst, Mode, ParamClass,
    ProgramBuilder, Reg, RegTy, RootKind, StmtKind, TyId, Width,
};
use rsim_sem::decl::{Body, DeclKind, ObjClass, ObjRole, Param, SubprogramKind};
use rsim_sem::hir::{self, Mode as HMode, Sensitivity, StmtKind as HStmt, UnitHir};
use rsim_sem::scope::ScopeKind;
use rsim_sem::{Db, DeclId, ScopeId, StmtId, SubtypeId, UnitId};

use crate::{Cx, Failed, InstConst, Obj, ProcessIr, R, SignalIr, Space, UnitIr, ty};

/// How a parameter is passed (DESIGN.md "Subprograms").
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Shape {
    /// Scalar by value: one cell.
    Val,
    /// Scalar `out`/`inout` variable: the address of the actual.
    Ref,
    /// Array: data address and descriptor address, two cells.
    Arr,
}

pub(crate) fn shape(db: &Db, cx: &mut Cx, p: &Param, span: Span) -> R<Shape> {
    match p.class {
        ObjClass::Signal => return Err(cx.unsupported(span, "signal parameters")),
        ObjClass::File => return Err(cx.unsupported(span, "file parameters (M3)")),
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
        _ => Err(cx.unsupported(span, "buffer, linkage and view parameters")),
    }
}

trait ScalarKind {
    fn kind_is_scalar(&self) -> bool;
}

impl ScalarKind for rsim_sem::types::TypeDef {
    fn kind_is_scalar(&self) -> bool {
        use rsim_sem::types::TypeKind::*;
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
    /// Whether the current block can still take instructions (no terminator yet).
    pub live: bool,
    /// Enclosing loops: (loop statement, `next` target, `exit` target).
    pub loops: Vec<(StmtId, BlockId, BlockId)>,
    /// The result subtype of a function.
    pub ret: Option<SubtypeId>,
    pub name: String,
    /// Driver slots of a process: the signal declaration each slot drives.
    pub drivers: Vec<DeclId>,
    /// The IR statement being lowered.
    pub cur: Option<rsim_ir::StmtId>,
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
            live: true,
            loops: Vec::new(),
            ret: None,
            name,
            drivers: Vec::new(),
            cur: None,
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

    pub fn msg(&mut self, what: &str, dir: rsim_ir::Dir) -> rsim_ir::CheckMsgId {
        self.fb.program().check_msg(what, dir)
    }

    pub fn ty(&mut self, t: rsim_sem::TypeId, span: Span) -> R<TyId> {
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
        let rt = if ty::is_real(self.db, t) {
            RegTy::F64
        } else {
            RegTy::I64
        };
        Ok((id, rt, w))
    }

    pub fn stmt_start(&mut self, kind: StmtKind, span: Span) -> rsim_ir::StmtId {
        if !self.live {
            let b = self.fb.block();
            self.fb.at(b);
        }
        let id = self.fb.stmt(kind, span);
        self.live = true;
        self.cur = Some(id);
        if let Some(item) = self
            .cx
            .cover(self.fb.program(), id, rsim_ir::CovKind::Statement)
        {
            self.emit(Inst::Cover { item });
        }
        id
    }

    /// A branch counter at the head of a branch arm.
    pub fn cover_branch(&mut self, span: Span) {
        let _ = span;
        let Some(stmt) = self.cur_stmt() else {
            return;
        };
        if let Some(item) = self
            .cx
            .cover(self.fb.program(), stmt, rsim_ir::CovKind::Branch)
        {
            self.emit(Inst::Cover { item });
        }
    }

    fn cur_stmt(&mut self) -> Option<rsim_ir::StmtId> {
        self.cur
    }

    // ---- declarations ----

    /// Objects declared in `scope` (variables, constants), initialised in order.
    pub fn decls(&mut self, scope: ScopeId) -> R<()> {
        let mut ok = true;
        for &d in &self.db.scopes[scope].order {
            let decl = &self.db.decls[d];
            let DeclKind::Object(o) = &decl.kind else {
                continue;
            };
            if matches!(o.role, ObjRole::Param | ObjRole::LoopParam) {
                continue;
            }
            let r = match o.class {
                ObjClass::Variable | ObjClass::Constant => self.local(d),
                ObjClass::SharedVariable => {
                    Err(self.cx.unsupported(decl.span, "shared variables (M3)"))
                }
                ObjClass::File => Err(self.cx.unsupported(decl.span, "file objects (M3)")),
                ObjClass::Signal => Err(self.cx.ice(decl.span, "signal in a sequential region")),
            };
            ok &= r.is_ok();
        }
        if ok { Ok(()) } else { Err(Failed) }
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
        if ty::is_array(db, t) {
            let tid = self.ty(t, decl.span)?;
            let (_, dims) = ty::array_of(db, t).ok_or(Failed)?;
            let dope_bytes = 24 * u32::from(dims);
            let off = self.fb.local(&name, tid, 24 + dope_bytes, 8);
            self.fb.root(off + 16, RootKind::Handle);
            let init = o.default.map(|e| self.expr(e)).transpose()?;
            let init = init.map(|v| self.arr(v, decl.span)).transpose()?;
            let dope = match (self.subtype_dope(o.subtype, decl.span)?, init) {
                (Some(dope), Some((_, vd))) => {
                    let what = self.msg(&name, rsim_ir::Dir::To);
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
            let data = match init {
                Some((data, _)) => data,
                None => self.default_array(o.subtype, size, count, decl.span)?,
            };
            let h = self.reg(RegTy::I64);
            self.emit(Inst::New {
                dst: h,
                ty: tid,
                init: data,
                size,
                shared: false,
            });
            let slot = self.addr(Space::Frame(self.depth), off);
            let hslot = self.offset(slot, 16);
            self.store(hslot, h, Width::B8);
            let what = self.msg(&name, rsim_ir::Dir::To);
            let p = self.reg(RegTy::Ptr);
            self.emit(Inst::Deref {
                dst: p,
                handle: h,
                what,
            });
            self.store(slot, p, Width::B8);
            let dslot = self.offset(slot, 24);
            let n = self.int(i64::from(dope_bytes));
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
            return Ok(());
        }
        let (tid, _, w) = self.scalar(o.subtype, decl.span)?;
        let off = self.fb.local(&name, tid, w.bytes(), w.bytes());
        let v = match o.default {
            Some(e) => {
                let v = self.scalar_expr(e)?;
                self.check_scalar(v, Some(db.exprs[e].subtype), o.subtype, &name, decl.span)?;
                v
            }
            None => self.scalar_left(o.subtype, decl.span)?,
        };
        let a = self.addr(Space::Frame(self.depth), off);
        self.store(a, v, w);
        self.cx.objs.insert(
            d,
            Obj::Scalar {
                space: Space::Frame(self.depth),
                off,
                w,
            },
        );
        Ok(())
    }

    /// `size` bytes of `count` elements, each the element subtype's left bound.
    fn default_array(&mut self, st: SubtypeId, size: Reg, count: Reg, span: Span) -> R<Reg> {
        let db = self.db;
        let (elem, _) = ty::array_of(db, db.base(st)).ok_or(Failed)?;
        let data = self.alloc(size);
        let left = db
            .static_bounds(elem)
            .and_then(|(l, _, _)| ty::cell(&l))
            .filter(|&c| c == 0);
        if left.is_some() {
            return Ok(data); // temporaries are zeroed
        }
        let etid = self.ty(db.base(elem), span)?;
        if rsim_ir::Ty::logic_leaf(self.fb.program().types(), etid).is_some() {
            return Err(self
                .cx
                .unsupported(span, "logic arrays whose element left bound is not code 0"));
        }
        let (_, _, w) = self.scalar(elem, span)?;
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
        });
        let size = self.load(l, Width::B8, RegTy::I64);
        let c = self.offset(l, 24);
        let count = self.load(c, Width::B8, RegTy::I64);
        (size, count)
    }

    /// Finish the body: a function falling off its end is a runtime error.
    pub fn end(mut self, span: Span) {
        if self.live {
            if self.kind == FuncKind::Function {
                let what = format!("function `{}` ended without a return statement", self.name);
                let what = self.msg(&what, rsim_ir::Dir::To);
                let _ = span;
                self.term(Inst::Unreachable { what });
            } else {
                self.term(Inst::Ret { v: None });
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
    let mut fx = Fx::new(db, cx, pb.define(f), depth, kind, decl.name.to_string());
    if let SubprogramKind::Function { pure: true } = sp.kind {
        fx.fb.flag(FuncFlags::PURE);
    }
    fx.ret = sp.ret;
    fx.stmt_start(StmtKind::Subprogram, decl.span);
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
                Obj::Scalar {
                    space: Space::Frame(depth),
                    off,
                    w: Width::B8,
                }
            }
            Shape::Ref => {
                let (_, _, w) = fx.scalar(p.subtype, decl.span)?;
                let off = fx.fb.param(&name, Width::B8, ParamClass::Value, mode, None);
                Obj::ScalarRef {
                    space: Space::Frame(depth),
                    off,
                    w,
                }
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
                Obj::Array {
                    space: Space::Frame(depth),
                    off,
                }
            }
        };
        fx.cx.objs.insert(pd, obj);
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
    if let UnitHir::Entity {
        generics,
        ports,
        stmts: estmts,
    } = &e.hir
    {
        if !generics.is_empty() || !ports.is_empty() {
            return Err(cx.unsupported(e.span, "entity generics and ports (M2 slice 3)"));
        }
        if !estmts.is_empty() {
            return Err(cx.unsupported(e.span, "entity statements"));
        }
    }
    let mut ir = UnitIr {
        unit,
        inst_size: 0,
        consts: Vec::new(),
        signals: Vec::new(),
        processes: Vec::new(),
    };
    let mut ok = true;
    for scope in [e.scope, u.scope] {
        for &d in &db.scopes[scope].order {
            ok &= inst_object(db, pb, cx, &mut ir, d).is_ok();
        }
    }
    for &s in stmts.iter() {
        ok &= process(db, pb, cx, &mut ir, s).is_ok();
    }
    if ok { Ok(ir) } else { Err(Failed) }
}

/// A thunk evaluating one cell (an initial value), for the elaborator.
fn thunk(
    db: &Db,
    pb: &mut ProgramBuilder,
    cx: &mut Cx,
    name: &str,
    span: Span,
    f: impl FnOnce(&mut Fx<'_, '_>) -> R<Reg>,
) -> R<rsim_ir::FuncId> {
    // ponytail: a pure function rather than `FuncKind::Thunk`, whose `Ret` the verifier
    // requires to be empty; a thunk once rsim-ir lets thunks return their value.
    let id = pb.declare(name, FuncKind::Function, None);
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

fn inst_object(db: &Db, pb: &mut ProgramBuilder, cx: &mut Cx, ir: &mut UnitIr, d: DeclId) -> R<()> {
    let decl = &db.decls[d];
    let DeclKind::Object(o) = &decl.kind else {
        return Ok(());
    };
    let name = decl.name.to_string();
    if ty::is_array(db, db.base(o.subtype)) {
        return Err(cx.unsupported(
            decl.span,
            "array signals and constants in an architecture (M2 slice 2)",
        ));
    }
    match o.class {
        ObjClass::Signal => {
            let t = ty::lower(db, cx, pb, db.base(o.subtype), decl.span)?;
            let (st, def) = (o.subtype, o.default);
            let init = thunk(
                db,
                pb,
                cx,
                &format!("{name}'init"),
                decl.span,
                |fx| match def {
                    Some(e) => {
                        let v = fx.scalar_expr(e)?;
                        fx.check_scalar(v, Some(db.exprs[e].subtype), st, &name, decl.span)?;
                        Ok(v)
                    }
                    None => fx.scalar_left(st, decl.span),
                },
            )?;
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
            });
            Ok(())
        }
        ObjClass::Constant => {
            let t = ty::lower(db, cx, pb, db.base(o.subtype), decl.span)?;
            let w = pb.types().get(t).map_or(Width::B8, ty::width);
            let Some(def) = o.default else {
                return Err(cx.ice(decl.span, "constant without a value"));
            };
            let st = o.subtype;
            let init = thunk(db, pb, cx, &name, decl.span, |fx| {
                let v = fx.scalar_expr(def)?;
                fx.check_scalar(v, Some(db.exprs[def].subtype), st, &name, decl.span)?;
                Ok(v)
            })?;
            let off = ir.inst_size.next_multiple_of(8);
            ir.inst_size = off + 8;
            cx.objs.insert(
                d,
                Obj::Scalar {
                    space: Space::Inst,
                    off,
                    w,
                },
            );
            ir.consts.push(InstConst {
                decl: d,
                name,
                off,
                w,
                init,
            });
            Ok(())
        }
        _ => Err(cx.unsupported(
            decl.span,
            "shared variables and files in an architecture (M3)",
        )),
    }
}

/// A process, or the equivalent process of a concurrent statement (LRM 11.3 to 11.6).
fn process(db: &Db, pb: &mut ProgramBuilder, cx: &mut Cx, ir: &mut UnitIr, s: StmtId) -> R<()> {
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
            info,
            ..
        } => {
            if *guarded {
                return Err(cx.unsupported(st.span, "guarded signal assignments"));
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
    let mut drivers = Vec::new();
    let mut sigs = Vec::new();
    for l in &info.drivers {
        let hir::ObjRoot::Decl(d) = l.root else {
            return Err(cx.unsupported(l.span, "drivers of external names and implicit signals"));
        };
        let Some(&Obj::Signal { sig }) = cx.objs.get(&d) else {
            return Err(cx.unsupported(l.span, "drivers of ports and outer signals (M2 slice 3)"));
        };
        if !drivers.contains(&d) {
            drivers.push(d);
            sigs.push(sig);
        }
    }
    let mut fx = Fx::new(db, cx, pb.define(f), 0, FuncKind::Process, name.clone());
    if implicit {
        fx.fb.flag(FuncFlags::IMPLICIT);
    }
    fx.drivers = drivers;
    fx.stmt_start(StmtKind::Process, st.span);
    let mut r = Ok(());
    if let Some(sc) = scope {
        r = fx.decls(sc);
    }
    let top = fx.block();
    fx.jump(top);
    fx.at(top);
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
    });
    Ok(())
}
