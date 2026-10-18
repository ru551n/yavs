//! Functions: frames, parameters, declarations, subprogram and process bodies, architectures.

use yavs_diag::Span;
use yavs_ir::{
    AddrSpace, BlockId, FuncBuilder, FuncFlags, FuncId, FuncKind, Inst, Mode, ParamClass,
    ProgramBuilder, Reg, RegTy, RootKind, StmtKind, TyId, Width,
};
use yavs_sem::decl::{Alias, Body, DeclKind, ObjClass, ObjRole, Param, SubprogramKind};
use yavs_sem::hir::{self, Mode as HMode, Sensitivity, StmtKind as HStmt, UnitHir};
use yavs_sem::scope::ScopeKind;
use yavs_sem::{Db, DeclId, ScopeId, StmtId, SubtypeId, UnitId};

use crate::expr::Place;
use crate::{Cx, Failed, InstConst, InstValue, Obj, ProcessIr, R, SignalIr, Space, UnitIr, ty};

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
}

pub(crate) fn shape(db: &Db, cx: &mut Cx, p: &Param, span: Span) -> R<Shape> {
    match p.class {
        ObjClass::Signal
            if !ty::is_array(db, db.base(p.subtype))
                && !db.types[db.base(p.subtype)].kind_is_scalar() =>
        {
            return Err(cx.unsupported(span, "record signal parameters"));
        }
        ObjClass::Signal if p.mode == HMode::In => return Ok(Shape::Sig),
        ObjClass::Signal if matches!(p.mode, HMode::Out | HMode::Inout) => {
            return Ok(Shape::SigOut);
        }
        ObjClass::Signal => {
            return Err(cx.unsupported(span, "buffer, linkage and view signal parameters"));
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
        _ => Err(cx.unsupported(span, "buffer, linkage and view parameters")),
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
    /// Whether the current block can still take instructions (no terminator yet).
    pub live: bool,
    /// Enclosing loops: (loop statement, `next` target, `exit` target).
    pub loops: Vec<(StmtId, BlockId, BlockId)>,
    /// The result subtype of a function.
    pub ret: Option<SubtypeId>,
    pub name: String,
    /// Driver slots of a process: the signal declaration each slot drives.
    pub drivers: Vec<hir::Lsp>,
    /// The IR statement being lowered.
    pub cur: Option<yavs_ir::StmtId>,
    /// In a protected type method: the frame depth and slot of `self`.
    pub prot_self: Option<(u8, u32)>,
    /// The `GUARD` signal of the enclosing guarded block (guarded assignments).
    pub guard: Option<DeclId>,
    /// Copy-out after the call being lowered: (frame slot of the temporary, actual,
    /// subtype of the formal) for `out`/`inout` actuals passed through a temporary.
    pub writeback: Vec<(u32, Option<yavs_sem::ExprId>, SubtypeId)>,
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
            prot_self: None,
            guard: None,
            writeback: Vec::new(),
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
            Space::Pkg(p) => AddrSpace::Pkg(p),
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
        let rt = if ty::is_real(self.db, t) {
            RegTy::F64
        } else {
            RegTy::I64
        };
        Ok((id, rt, w))
    }

    pub fn stmt_start(&mut self, kind: StmtKind, span: Span) -> yavs_ir::StmtId {
        if !self.live {
            let b = self.fb.block();
            self.fb.at(b);
        }
        let id = self.fb.stmt(kind, span);
        self.live = true;
        self.cur = Some(id);
        if let Some(item) = self
            .cx
            .cover(self.fb.program(), id, yavs_ir::CovKind::Statement)
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
            .cover(self.fb.program(), stmt, yavs_ir::CovKind::Branch)
        {
            self.emit(Inst::Cover { item });
        }
    }

    fn cur_stmt(&mut self) -> Option<yavs_ir::StmtId> {
        self.cur
    }

    // ---- declarations ----

    /// Objects declared in `scope` (variables, constants), initialised in order.
    pub fn decls(&mut self, scope: ScopeId) -> R<()> {
        let mut ok = true;
        for &d in &self.db.scopes[scope].order {
            let decl = &self.db.decls[d];
            if let DeclKind::Alias(Alias::Object { target, subtype }) = &decl.kind {
                ok &= self.alias(d, *target, *subtype).is_ok();
                continue;
            }
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
        self.stmt_start(StmtKind::Decl, span);
        let name = db.decls[d].name.to_string();
        match self.place(target)? {
            Place::Scalar { addr, w, .. } => {
                let off = self.fb.slot(8, 8);
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
                dims,
            } => {
                let dope = match subtype
                    .map(|st| self.subtype_dope(st, span))
                    .transpose()?
                    .flatten()
                {
                    Some(want) => {
                        let what = self.msg(&ty::obj_desc(db, d), yavs_ir::Dir::To);
                        self.emit(Inst::SubtypeCheck {
                            dope,
                            want,
                            dims,
                            what,
                        });
                        want
                    }
                    None => dope,
                };
                let bytes = 24 * u32::from(dims);
                let off = self.fb.local(&name, tid, 24 + bytes, 8);
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
            _ => {
                return Err(self
                    .cx
                    .unsupported(span, "aliases of logic array elements and slices"));
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
            _ if self.is_general(t, decl.span)? => return self.local_comp(d),
            _ => {}
        }
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
            let what = self.msg(&name, yavs_ir::Dir::To);
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
                let what = ty::obj_desc(db, d);
                self.check_scalar(v, Some(db.exprs[e].subtype), o.subtype, &what, decl.span)?;
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
        let etid = self.ty(db.base(elem), span)?;
        let logic = yavs_ir::Ty::logic_leaf(self.fb.program().types(), etid).is_some();
        let bias = self.code_bias(etid);
        if left.is_some() && bias == 0 {
            return Ok(data); // temporaries are zeroed
        }
        if logic {
            let v = self.scalar_left(elem, span)?;
            let code = self.enc(v, bias);
            self.for_n(count, |fx, i| {
                fx.emit(Inst::LSet {
                    base: data,
                    idx: i,
                    v: code,
                });
                Ok(())
            })?;
            return Ok(data);
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
                let what = self.msg(&what, yavs_ir::Dir::To);
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
    fx.method_prologue(d, depth, decl.span)?;
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
    if let UnitHir::Entity {
        generics,
        ports,
        stmts: estmts,
    } = &e.hir
    {
        for &g in generics.iter() {
            if !matches!(db.decls[g].kind, DeclKind::Object(_)) {
                return Err(
                    cx.unsupported(db.decls[g].span, "generic types, subprograms and packages")
                );
            }
        }
        iface.extend(generics.iter().chain(ports.iter()));
        if !estmts.is_empty() {
            return Err(cx.unsupported(e.span, "entity statements"));
        }
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
    };
    cx.inst_size = 0;
    cx.names.clear();
    cx.arch_scopes = vec![e.scope, u.scope];
    let mut ok = true;
    let scopes = [e.scope, u.scope];
    let decls = iface.iter().chain(
        scopes
            .iter()
            .flat_map(|&s| db.scopes[s].order.iter())
            .filter(|d| !iface.contains(d)),
    );
    for &d in decls {
        ok &= inst_object(db, pb, cx, &mut ir, d).is_ok();
    }
    ok &= region(db, pb, cx, &mut ir, stmts, None).is_ok();
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
            for &d in &db.scopes[scope].order {
                ok &= inst_object(db, pb, cx, ir, d).is_ok();
            }
            ok
        };
    for &s in stmts {
        let st = &db.stmts[s];
        match &st.kind {
            HStmt::Instance { .. } => {}
            HStmt::Block {
                scope,
                guard,
                generics,
                ports,
                body,
                ..
            } => {
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
            _ => ok &= process(db, pb, cx, ir, s, outer).is_ok(),
        }
    }
    if ok { Ok(()) } else { Err(Failed) }
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

fn inst_object(db: &Db, pb: &mut ProgramBuilder, cx: &mut Cx, ir: &mut UnitIr, d: DeclId) -> R<()> {
    let decl = &db.decls[d];
    let DeclKind::Object(o) = &decl.kind else {
        return Ok(());
    };
    let name = decl.name.to_string();
    if o.class == ObjClass::Constant
        && o.role != ObjRole::Generic
        && matches!(
            db.types[db.base(o.subtype)].kind,
            yavs_sem::types::TypeKind::Array { .. } | yavs_sem::types::TypeKind::Record { .. }
        )
    {
        // ponytail: like a package constant, a composite constant of an architecture is
        // re-evaluated from its initialiser at each use (`read_obj`); instance data once
        // elaboration folds every composite constant (#73).
        return Ok(());
    }
    let array = ty::is_array(db, db.base(o.subtype));
    match o.class {
        ObjClass::Signal => {
            let t = ty::lower(db, cx, pb, db.base(o.subtype), decl.span)?;
            let dope = if array {
                let Some(size) = yavs_ir::Ty::desc_size(pb.types(), t) else {
                    return Err(cx.ice(decl.span, "array type without a descriptor size"));
                };
                let off = cx.inst_alloc(size as u32);
                cx.sig_dopes.insert(d, off);
                Some(off)
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
            let off = cx.inst_alloc(if array { 16 } else { 8 });
            ir.values.push(InstValue {
                decl: d,
                name: name.clone(),
                off,
                ty: t,
            });
            if array {
                cx.objs.insert(d, Obj::InstArray { off });
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
            if let (Some(def), false) = (o.default, given) {
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
                    init,
                    instance_level: cx.arch_scopes.contains(&decl.scope),
                });
            }
            Ok(())
        }
        ObjClass::SharedVariable => crate::protected::shared_var(db, pb, cx, ir, d),
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
    guard: Option<DeclId>,
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
            info,
            ..
        } => {
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
    // One driver slot per longest static prefix, in `ProcessInfo::drivers` order.
    let mut sigs = Vec::new();
    for l in &info.drivers {
        let hir::ObjRoot::Decl(d) = l.root else {
            return Err(cx.unsupported(l.span, "drivers of external names and implicit signals"));
        };
        let Some(&Obj::Signal { sig }) = cx.objs.get(&d) else {
            return Err(cx.unsupported(l.span, "drivers of signals outside the architecture"));
        };
        sigs.push(sig);
    }
    let drivers = info.drivers.clone();
    let mut fx = Fx::new(db, cx, pb.define(f), 0, FuncKind::Process, name.clone());
    if implicit {
        fx.fb.flag(FuncFlags::IMPLICIT);
    }
    fx.drivers = drivers;
    fx.guard = guard;
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
    if !matches!(u.hir, UnitHir::Package { .. }) {
        return Err(cx.ice(u.span, "not a package"));
    }
    let mut scopes = vec![u.scope];
    scopes.extend(db.units.iter().find_map(|(_, b)| match b.hir {
        UnitHir::PackageBody { package } if package == unit => Some(b.scope),
        _ => None,
    }));
    let name = format!("{}'init", u.name.text);
    let f = pb.declare(&name, FuncKind::Thunk, None);
    let mut fx = Fx::new(db, cx, pb.define(f), 0, FuncKind::Thunk, name);
    fx.stmt_start(StmtKind::Decl, u.span);
    let mut size = 0u32;
    let mut signals = Vec::new();
    let mut ok = true;
    for s in scopes {
        for &d in &db.scopes[s].order {
            ok &= fx.pkg_object(d, index, &mut size, &mut signals).is_ok();
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
            ObjClass::Signal => {
                let ty = self.ty(t, span)?;
                let sig = self.cx.pkg_sigs.len() as u32;
                let s = SignalIr {
                    decl: d,
                    name: decl.name.to_string(),
                    ty,
                    init: None,
                    dope: None,
                };
                self.cx.pkg_sigs.push(s.clone());
                signals.push(s);
                self.cx.objs.insert(d, Obj::Signal { sig });
                Ok(())
            }
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
                        // ponytail: bounds known only at run time: the constant is still
                        // re-evaluated at each use (`read_obj`); a heap object in package data
                        // when such constants need evaluating once.
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
                    return Err(self
                        .cx
                        .unsupported(span, "shared variables of non-protected types"));
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
            // Files in packages are not registered: a use reports E2001 (M3).
            _ => Ok(()),
        }
    }

    /// The descriptor of a composite subtype when it is known at lowering: none needed (a
    /// record of constrained fields), or static index bounds (from the subtype, else from a
    /// folded initial value).
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
