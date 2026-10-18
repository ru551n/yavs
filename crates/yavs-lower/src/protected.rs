//! Protected types (DESIGN.md "Composites", ir.md 1.6): an object is a handle to a heap
//! record of its variables (the state); a method is a subprogram whose first parameter is
//! `self`, called with `CallProt`. Inside a method the state variables are frame slots
//! holding their addresses in the state, so names resolve like any other object.

use yavs_diag::Span;
use yavs_ir::{Dir, Inst, Mode, ParamClass, Reg, RegTy, RootKind, Ty, TyId, Width};
use yavs_sem::decl::{Body, DeclKind, ObjClass, ObjRole};
use yavs_sem::hir::Arg;
use yavs_sem::types::TypeKind;
use yavs_sem::{Db, DeclId, ExprId, TypeId};

use crate::composite::scalar_w;
use crate::expr::V;
use crate::func::{Fx, func_of};
use crate::{Obj, R, Space, ty};

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
                && matches!(o.class, ObjClass::Variable | ObjClass::Constant)
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
    /// state variable holding its address (or `[data, desc]` for composites).
    pub fn method_prologue(&mut self, d: DeclId, depth: u8, span: Span) -> R<()> {
        let db = self.db;
        let Some(t) = db.method_of(d) else {
            return Ok(());
        };
        let off = self
            .fb
            .param("self", Width::B8, ParamClass::Value, Mode::In, None);
        self.prot_self = Some((depth, off));
        let state = self.state_of(t, span)?;
        let ds = self.desc_size(state);
        let s = self.addr(Space::Frame(depth), off);
        let h = self.load(s, Width::B8, RegTy::I64);
        let what = self.msg("protected object", Dir::To);
        let p = self.reg(RegTy::Ptr);
        self.emit(Inst::Deref {
            dst: p,
            handle: h,
            what,
        });
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

    /// A call of method `callee` on `obj` (or, inside a method, on `self`).
    pub fn method_call(
        &mut self,
        callee: DeclId,
        args: &[Arg],
        obj: Option<ExprId>,
        span: Span,
    ) -> R<Option<V>> {
        let db = self.db;
        let Some(sp) = db.decls[callee].kind.subprogram() else {
            return Err(self.cx.ice(span, "method is not a subprogram"));
        };
        if !matches!(sp.body, Body::Hir { .. }) {
            return Err(self.cx.unsupported(span, "methods without a VHDL body"));
        }
        let f = func_of(db, self.fb.program(), self.cx, callee, span)?;
        let h = match obj {
            Some(e) => self.scalar_expr(e)?,
            None => {
                let Some((depth, off)) = self.prot_self else {
                    return Err(self.cx.ice(span, "method call without an object"));
                };
                let a = self.addr(Space::Frame(depth), off);
                self.load(a, Width::B8, RegTy::I64)
            }
        };
        let cells = self.cells(sp, args, span, false)?;
        let args = self.fb.regs(&cells);
        let Some(ret) = sp.ret else {
            self.emit(Inst::CallProt {
                func: f,
                obj: h,
                args,
                dst: None,
            });
            return Ok(None);
        };
        let t = db.base(ret);
        let comp = self.is_comp(t);
        let rt = if comp {
            RegTy::Ptr
        } else if ty::is_real(db, t) {
            RegTy::F64
        } else {
            RegTy::I64
        };
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

    /// A new protected object of type `t`: its state initialised from the variables'
    /// declarations, in the shared heap for a shared variable.
    pub fn prot_new(&mut self, t: TypeId, shared: bool, span: Span) -> R<Reg> {
        let db = self.db;
        let state = self.state_of(t, span)?;
        let Some(Ty::Record { fields }) = self.fb.program().types().get(state).cloned() else {
            return Err(self.cx.ice(span, "protected state is not a record"));
        };
        let vars = state_vars(db, t);
        // The state's descriptor: the constraints of its variables (rule 1).
        let ds = self.desc_size(state);
        let desc = self.alloc_n(ds.max(1));
        let mut off = 0;
        let mut inits = Vec::new();
        for (f, &v) in fields.iter().zip(&vars) {
            let DeclKind::Object(o) = &db.decls[v].kind else {
                continue;
            };
            let init = match o.default {
                Some(e) => Some(self.expr(e)?),
                None => None,
            };
            if f.desc.is_none() && Ty::needs_desc(self.fb.program().types(), f.ty) {
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
            inits.push((o.subtype, o.default, init));
        }
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
        for (n, (st, e, init)) in inits.into_iter().enumerate() {
            let (a, fd) = self.field_addr(state, base, buf, n as u32, span)?;
            match init {
                Some(V::S(r)) => {
                    if let Some(e) = e {
                        self.check_scalar(r, Some(db.exprs[e].subtype), st, "", span)?;
                    }
                    self.store_at(fields[n].ty, a, V::S(r), span)?;
                }
                Some(v) => self.store_at(fields[n].ty, a, v, span)?,
                None => {
                    let ft = db.base(st);
                    if let TypeKind::Protected { .. } = db.types[ft].kind {
                        let h = self.prot_new(ft, shared, span)?;
                        self.store(a, h, Width::B8);
                    } else {
                        let c = db.subtypes[st].constraint.clone();
                        self.default_into(st, &c, a, fd, span)?;
                    }
                }
            }
        }
        let h = self.reg(RegTy::I64);
        self.emit(Inst::New {
            dst: h,
            ty: state,
            init: buf,
            size: total,
            shared,
        });
        Ok(h)
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
    if !matches!(db.types[t].kind, TypeKind::Protected { .. }) {
        return Err(cx.unsupported(decl.span, "shared variables of non-protected types"));
    }
    let name = decl.name.to_string();
    let span = decl.span;
    let init = crate::func::thunk(db, pb, cx, &name, span, |fx| fx.prot_new(t, true, span))?;
    let off = ir.inst_size.next_multiple_of(8);
    ir.inst_size = off + 8;
    cx.objs.insert(
        d,
        Obj::Scalar {
            space: Space::Inst,
            off,
            w: Width::B8,
        },
    );
    ir.consts.push(crate::InstConst {
        decl: d,
        name,
        off,
        w: Width::B8,
        init,
    });
    Ok(())
}
