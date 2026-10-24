//! Protected types (DESIGN.md "Composites", ir.md 1.6): an object is a handle to a heap
//! record of its variables (the state); a method is a subprogram whose first parameter is
//! `self`, called with `CallProt`. Inside a method the state variables are frame slots
//! holding their addresses in the state, so names resolve like any other object.

use yavs_diag::Span;
use yavs_ir::{Dir, Inst, Mode, ParamClass, Reg, RegTy, RootKind, Ty, TyId, Width};
use yavs_sem::decl::{Body, DeclKind, ObjClass, ObjRole};
use yavs_sem::hir::Arg;
use yavs_sem::types::TypeKind;
use yavs_sem::{Db, DeclId, ExprId, ScopeId, SubtypeId, TypeId};

use crate::composite::scalar_w;
use crate::expr::Conv;
use crate::expr::V;
use crate::func::{Fx, func_of};
use crate::{Obj, R, Space};

/// The protected type whose method `d` is: declared in the protected type declaration
/// (`Db::method_of`) or only in its body (a private method, which also sees the object).
pub(crate) fn method_type(db: &Db, d: DeclId) -> Option<TypeId> {
    if let Some(t) = db.method_of(d) {
        return Some(t);
    }
    let d = db.entity(d);
    // Implicit operations of types declared in the body are not methods.
    if !matches!(db.decls[d].kind.subprogram()?.body, Body::Hir { .. }) {
        return None;
    }
    let s = db.decls[d].scope;
    if db.scopes[s].kind != yavs_sem::scope::ScopeKind::ProtectedBody {
        return None;
    }
    db.protected_of_region(db.scopes[s].parent?)
}

/// The variables of protected type `t` in state order: 2019 private variables, then the
/// body's variables and constants.
pub(crate) fn state_vars(db: &Db, t: TypeId) -> Vec<DeclId> {
    let TypeKind::Protected {
        decl_scope, body, ..
    } = &db.types[t].kind
    else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for s in [Some(*decl_scope), *body].into_iter().flatten() {
        for &d in &db.scopes[s].order {
            if let DeclKind::Object(o) = &db.decls[d].kind
                && o.role != ObjRole::Param
                && matches!(
                    o.class,
                    ObjClass::Variable | ObjClass::Constant | ObjClass::File
                )
            {
                out.push(d);
            }
        }
    }
    out
}

impl Fx<'_, '_> {
    /// The IR state record of protected type `t`.
    fn state_of(&mut self, t: TypeId, span: Span) -> R<TyId> {
        let tid = self.ty(t, span)?;
        match self.fb.program().types().get(tid) {
            Some(Ty::Protected { state }) => Ok(*state),
            _ => Err(self.cx.ice(span, "protected type without a state record")),
        }
    }

    /// Method prologue (before the other parameters): the `self` parameter and a slot per
    /// state variable holding its address (or `[data, desc]` for composites). A subprogram
    /// declared (at any depth) in a method takes that method's `self` through the static
    /// link and binds the state in its own frame (LRM 5.6.2, #221).
    pub fn method_prologue(&mut self, d: DeclId, scope: ScopeId, depth: u8, span: Span) -> R<()> {
        let db = self.db;
        let (t, sd, so) = match method_type(db, d) {
            Some(t) => {
                let off = self
                    .fb
                    .param("self", Width::B8, ParamClass::Value, Mode::In, None);
                self.fb.root(off, RootKind::Handle);
                self.cx.method_self.insert(scope, (t, depth, off));
                (t, depth, off)
            }
            None => {
                let mut s = Some(db.decls[d].scope);
                loop {
                    let Some(sc) = s else { return Ok(()) };
                    if let Some(&m) = self.cx.method_self.get(&sc) {
                        break m;
                    }
                    s = db.scopes[sc].parent;
                }
            }
        };
        self.prot_self = Some((sd, so));
        let state = self.state_of(t, span)?;
        let p = self.deref_self(sd, so);
        self.bind_state(t, state, p, depth, span)
    }

    /// The state address of the object whose handle is in frame slot `(depth, off)`.
    fn deref_self(&mut self, depth: u8, off: u32) -> Reg {
        let s = self.addr(Space::Frame(depth), off);
        let h = self.load(s, Width::B8, RegTy::Handle);
        let what = self.msg("protected object", Dir::To);
        let p = self.reg(RegTy::Ptr);
        self.emit(Inst::Deref {
            dst: p,
            handle: h,
            what,
        });
        p
    }

    /// A slot per state variable of protected type `t` holding its address in the state at
    /// `p` (or `[data, desc]` for composites), so that names of the variables resolve.
    fn bind_state(&mut self, t: TypeId, state: TyId, p: Reg, depth: u8, span: Span) -> R<()> {
        let db = self.db;
        let ds = self.desc_size(state);
        let base = self.offset(p, ds);
        let Some(Ty::Record { fields }) = self.fb.program().types().get(state).cloned() else {
            return Err(self.cx.ice(span, "protected state is not a record"));
        };
        for (n, v) in state_vars(db, t).into_iter().enumerate() {
            let (a, fd) = self.field_addr(state, base, p, n as u32, span)?;
            let obj = match scalar_w(self.fb.program(), fields[n].ty) {
                Some(w) => {
                    let o = self.fb.slot(8, 8);
                    let sa = self.addr(Space::Frame(depth), o);
                    self.store(sa, a, Width::B8);
                    Obj::ScalarRef {
                        space: Space::Frame(depth),
                        off: o,
                        w,
                    }
                }
                None => {
                    let o = self.fb.slot(16, 8);
                    let sa = self.addr(Space::Frame(depth), o);
                    self.store(sa, a, Width::B8);
                    let sd = self.offset(sa, 8);
                    self.store(sd, fd, Width::B8);
                    Obj::Array {
                        space: Space::Frame(depth),
                        off: o,
                    }
                }
            };
            self.cx.objs.insert(v, obj);
        }
        Ok(())
    }

    /// The handle of private variable `var` of protected object `object` (2019 5.6.2: the
    /// object of a call through a forwarding method alias).
    pub fn private_var(&mut self, object: ExprId, var: DeclId, span: Span) -> R<V> {
        let db = self.db;
        let t = db.base(db.exprs[object].subtype);
        let Some(n) = state_vars(db, t).iter().position(|&v| v == var) else {
            return Err(self
                .cx
                .ice(span, "private variable outside its protected type's state"));
        };
        let h = self.scalar_expr(object)?;
        let what = self.msg("protected object", Dir::To);
        let p = self.reg(RegTy::Ptr);
        self.emit(Inst::Deref {
            dst: p,
            handle: h,
            what,
        });
        let state = self.state_of(t, span)?;
        let ds = self.desc_size(state);
        let base = self.offset(p, ds);
        let (a, _) = self.field_addr(state, base, p, n as u32, span)?;
        Ok(V::S(self.load(a, Width::B8, RegTy::Handle)))
    }

    /// A call of method `callee` on `obj` (or, inside a method, on `self`).
    pub fn method_call(
        &mut self,
        callee: DeclId,
        args: &[Arg],
        obj: Option<ExprId>,
        res: Option<SubtypeId>,
        span: Span,
    ) -> R<Option<V>> {
        let db = self.db;
        let Some(sp) = db.decls[callee].kind.subprogram() else {
            return Err(self.cx.ice(span, "method is not a subprogram"));
        };
        let Body::Hir { scope, .. } = sp.body else {
            return Err(self.cx.unsupported(span, "methods without a VHDL body"));
        };
        let f = func_of(db, self.fb.program(), self.cx, callee, span)?;
        let h = match obj {
            Some(e) => self.scalar_expr(e)?,
            None => {
                let Some((depth, off)) = self.prot_self else {
                    // Only the initial value of a variable whose constraint it gives runs
                    // before its object exists (`prot_new`).
                    return Err(self.cx.unsupported(
                        span,
                        "a method call in the initial value of an unconstrained variable of a protected body",
                    ));
                };
                let a = self.addr(Space::Frame(depth), off);
                self.load(a, Width::B8, RegTy::Handle)
            }
        };
        let mut cells = self.cells(sp, args, span, Conv::Body)?;
        self.named_return_cells(callee, scope, res, span, &mut cells)?;
        let args = self.fb.regs(&cells);
        let Some(ret) = sp.ret else {
            self.emit(Inst::CallProt {
                func: f,
                obj: h,
                args,
                dst: None,
            });
            self.copy_out(span)?;
            return Ok(None);
        };
        let t = db.base(ret);
        let comp = self.is_comp(t);
        let rt = if comp { RegTy::Ptr } else { self.rt_of(t) };
        let dst = self.reg(rt);
        self.emit(Inst::CallProt {
            func: f,
            obj: h,
            args,
            dst: Some(dst),
        });
        if comp {
            return Ok(Some(self.fat_value(dst)));
        }
        Ok(Some(V::S(dst)))
    }

    /// The slot of non-protected shared variable `d` (`--relaxed`, W1904) of type `t`, created in
    /// the shared heap and initialised: its shape and the address of the slot.
    pub fn new_shared_slot(
        &mut self,
        d: DeclId,
        t: TypeId,
        name: &str,
        span: Span,
    ) -> R<(crate::VarShape, Reg)> {
        let (shape, tid) = self.shared_shape(d, t, span)?;
        let buf = self.alloc_n(shape.size());
        let size = self.int(i64::from(shape.size()));
        let h = self.reg(yavs_ir::RegTy::Handle);
        self.emit(yavs_ir::Inst::New {
            dst: h,
            ty: tid,
            init: buf,
            size,
            shared: true,
        });
        let what = self.msg(name, yavs_ir::Dir::To);
        let p = self.reg(yavs_ir::RegTy::Ptr);
        self.emit(yavs_ir::Inst::Deref {
            dst: p,
            handle: h,
            what,
        });
        self.var_init(d, shape, p, true)?;
        Ok((shape, p))
    }

    /// A new protected object of type `t`: its state initialised from the variables'
    /// declarations, in the shared heap for a shared variable. The initialisers' allocators
    /// follow a shared object there because the elaborator runs them with the shared state
    /// (yavs-interp `Frame::shared_alloc`, #246).
    pub fn prot_new(&mut self, t: TypeId, shared: bool, span: Span) -> R<Reg> {
        let db = self.db;
        let state = self.state_of(t, span)?;
        let Some(Ty::Record { fields }) = self.fb.program().types().get(state).cloned() else {
            return Err(self.cx.ice(span, "protected state is not a record"));
        };
        let vars = state_vars(db, t);
        // The state's descriptor: the constraints of its variables (rule 1). A variable whose
        // constraint comes from its initial value is initialised before the object exists.
        let outer_self = self.prot_self.take();
        let ds = self.desc_size(state);
        let desc = self.alloc_n(ds.max(1));
        let mut off = 0;
        let mut early = Vec::new();
        for (f, &v) in fields.iter().zip(&vars) {
            let DeclKind::Object(o) = &db.decls[v].kind else {
                return Err(self
                    .cx
                    .ice(span, "protected state variable is not an object"));
            };
            let mut init = None;
            if f.desc.is_none() && Ty::needs_desc(self.fb.program().types(), f.ty) {
                init = match o.default {
                    Some(e) => Some(self.expr(e)?),
                    None => None,
                };
                let vd = match init {
                    Some(V::A(_, vd)) => Some(vd),
                    _ => None,
                };
                let Some(d) = self.desc_of(o.subtype, vd, span)? else {
                    return Err(self.cx.ice(span, "unconstrained protected variable"));
                };
                let n = self.desc_size(f.ty);
                let at = self.offset(desc, off);
                let b = self.int(i64::from(n));
                self.emit(Inst::Copy {
                    dst: at,
                    src: d,
                    bytes: b,
                });
                off += n;
            }
            early.push(init);
        }
        // The object with default values, so that methods called by the initialisers below
        // see it (LRM 5.6.3: their implicit object is the one being elaborated).
        let size = self.size_of(state, desc);
        let dsr = self.int(i64::from(ds));
        let total = self.bin(yavs_ir::IntOp::Add, size, dsr);
        let buf = self.alloc(total);
        self.emit(Inst::Copy {
            dst: buf,
            src: desc,
            bytes: dsr,
        });
        let base = self.offset(buf, ds);
        self.temp_root(base, state, Some(buf));
        let mut late = Vec::new();
        for (n, (&v, init)) in vars.iter().zip(early).enumerate() {
            let (a, fd) = self.field_addr(state, base, buf, n as u32, span)?;
            late.push(init.is_none());
            match init {
                Some(val) => self.store_init(v, fields[n].ty, a, val, span)?,
                None => {
                    let DeclKind::Object(o) = &db.decls[v].kind else {
                        return Err(self
                            .cx
                            .ice(span, "protected state variable is not an object"));
                    };
                    let c = db.subtypes[o.subtype].constraint.clone();
                    let saved = std::mem::replace(&mut self.prot_defaults, false);
                    let r = self.default_into(o.subtype, &c, a, fd, span);
                    self.prot_defaults = saved;
                    r?;
                }
            }
        }
        let h = self.reg(RegTy::Handle);
        self.emit(Inst::New {
            dst: h,
            ty: state,
            init: buf,
            size: total,
            shared,
        });
        // The object's only reference while its initialisers run (rule 6).
        let so = self.fb.slot(8, 8);
        self.fb.root(so, RootKind::Handle);
        let sa = self.addr(Space::Frame(self.depth), so);
        self.store(sa, h, Width::B8);
        self.prot_self = Some((self.depth, so));
        // The initialisers also name the variables declared before them; other objects of
        // this type being lowered keep their own bindings.
        let saved: Vec<_> = vars.iter().map(|v| self.cx.objs.remove(v)).collect();
        let r = self.prot_init_late(t, state, &late, so, shared, span);
        for (v, o) in vars.iter().zip(saved) {
            match o {
                Some(o) => self.cx.objs.insert(*v, o),
                None => self.cx.objs.remove(v),
            };
        }
        self.prot_self = outer_self;
        r?;
        Ok(h)
    }

    /// The initialisers of the state variables not initialised by `prot_new`, in
    /// declaration order, on the object whose handle is in frame slot `so`.
    fn prot_init_late(
        &mut self,
        t: TypeId,
        state: TyId,
        late: &[bool],
        so: u32,
        shared: bool,
        span: Span,
    ) -> R<()> {
        let db = self.db;
        let Some(Ty::Record { fields }) = self.fb.program().types().get(state).cloned() else {
            return Err(self.cx.ice(span, "protected state is not a record"));
        };
        let p = self.deref_self(self.depth, so);
        self.bind_state(t, state, p, self.depth, span)?;
        let ds = self.desc_size(state);
        let base = self.offset(p, ds);
        for (n, v) in state_vars(db, t).into_iter().enumerate() {
            if !late[n] {
                continue;
            }
            let DeclKind::Object(o) = &db.decls[v].kind else {
                return Err(self
                    .cx
                    .ice(span, "protected state variable is not an object"));
            };
            let ft = db.base(o.subtype);
            if o.class == ObjClass::File {
                // The cell is zero from `default_into`; open it when the declaration says so.
                if o.file_open.is_some() {
                    let (a, _) = self.field_addr(state, base, p, n as u32, span)?;
                    self.open_file_at(v, a)?;
                }
                continue;
            }
            let val = match o.default {
                Some(e) => self.expr(e)?,
                None if matches!(db.types[ft].kind, TypeKind::Protected { .. }) => {
                    V::S(self.prot_new(ft, shared, span)?)
                }
                // 2019 composites of protected types (LCS-2016-014): new objects now.
                None if db.type_contains_protected(ft) => {
                    let (a, fd) = self.field_addr(state, base, p, n as u32, span)?;
                    let c = db.subtypes[o.subtype].constraint.clone();
                    self.default_into(o.subtype, &c, a, fd, span)?;
                    continue;
                }
                None => continue,
            };
            let (a, _) = self.field_addr(state, base, p, n as u32, span)?;
            self.store_init(v, fields[n].ty, a, val, span)?;
        }
        Ok(())
    }

    /// Store the initial value `val` of state variable `v` (field type `ft`) at `a`.
    fn store_init(&mut self, v: DeclId, ft: TyId, a: Reg, val: V, span: Span) -> R<()> {
        let db = self.db;
        let DeclKind::Object(o) = &db.decls[v].kind else {
            return Err(self
                .cx
                .ice(span, "protected state variable is not an object"));
        };
        if let (V::S(r), Some(e)) = (&val, o.default) {
            self.check_scalar(*r, Some(db.exprs[e].subtype), o.subtype, "", span)?;
        }
        self.store_at(ft, a, val, span)
    }

    /// A process-local variable of a protected type.
    pub fn local_prot(&mut self, d: DeclId, t: TypeId, span: Span) -> R<()> {
        let tid = self.ty(t, span)?;
        let name = self.db.decls[d].name.to_string();
        let off = self.fb.local(&name, tid, 8, 8);
        self.fb.root(off, RootKind::Handle);
        let h = self.prot_new(t, false, span)?;
        let a = self.addr(Space::Frame(self.depth), off);
        self.store(a, h, Width::B8);
        self.cx.objs.insert(
            d,
            Obj::Scalar {
                space: Space::Frame(self.depth),
                off,
                w: Width::B8,
            },
        );
        Ok(())
    }
}

/// A shared variable of a protected type in an architecture: an instance cell holding the
/// object's handle, created at elaboration in the shared heap (the elaborator runs the
/// thunk with `call_shared`).
pub(crate) fn shared_var(
    db: &Db,
    pb: &mut yavs_ir::ProgramBuilder,
    cx: &mut crate::Cx,
    ir: &mut crate::UnitIr,
    d: DeclId,
) -> R<()> {
    let decl = &db.decls[d];
    let DeclKind::Object(o) = &decl.kind else {
        return Err(cx.ice(decl.span, "not an object"));
    };
    let t = db.base(o.subtype);
    let name = decl.name.to_string();
    let span = decl.span;
    let off = cx.inst_alloc(8);
    let init = if matches!(db.types[t].kind, TypeKind::Protected { .. }) {
        cx.objs.insert(
            d,
            Obj::Scalar {
                space: Space::Inst,
                off,
                w: Width::B8,
            },
        );
        crate::func::thunk(db, pb, cx, &name, span, |fx| fx.prot_new(t, true, span))?
    } else {
        // `--relaxed` (W1904): the thunk creates the variable's slot in the shared heap and
        // returns its address; names of it reach it through the cell (`Space::Region`).
        let mut obj = None;
        let f = crate::func::thunk(db, pb, cx, &name, span, |fx| {
            let (shape, p) = fx.new_shared_slot(d, t, &name, span)?;
            obj = Some(shape.obj(Space::Region(off), 0));
            Ok(p)
        })?;
        let Some(obj) = obj else {
            return Err(cx.ice(span, "shared variable without a slot"));
        };
        cx.objs.insert(d, obj);
        let ty = crate::ty::lower(db, cx, pb, t, span)?;
        cx.shared_vars.push((d, ty));
        f
    };
    ir.consts.push(crate::InstConst {
        decl: d,
        name,
        off,
        w: Width::B8,
        comp: None,
        init,
        scope: crate::func::elab_region(db, decl.scope),
        instance_level: cx.arch_scopes.contains(&decl.scope),
    });
    Ok(())
}
