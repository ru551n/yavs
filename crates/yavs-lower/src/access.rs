//! Access values (DESIGN.md "Composites"): `null`, allocators, `.all` and implicit
//! dereference, `deallocate`. A value is a generational heap handle (rule 6); the object is
//! `[descriptor][value]`, so an access to an array keeps its bounds with the elements.

use yavs_diag::Span;
use yavs_ir::{Dir, Inst, RegTy};
use yavs_sem::hir::NewInit;
use yavs_sem::types::TypeKind;
use yavs_sem::{ExprId, SubtypeId};

use crate::R;
use crate::composite::scalar_w;
use crate::expr::{Place, V};
use crate::func::Fx;

impl Fx<'_, '_> {
    /// The designated subtype of access type `st`.
    fn designated(&mut self, st: SubtypeId, span: Span) -> R<SubtypeId> {
        match &self.db.types[self.db.base(st)].kind {
            TypeKind::Access { designated } => Ok(*designated),
            _ => Err(self.cx.ice(span, "dereference of a non-access value")),
        }
    }

    /// The object the access value `prefix` designates: its address and designated subtype.
    fn deref_obj(&mut self, prefix: ExprId, span: Span) -> R<(yavs_ir::Reg, SubtypeId)> {
        let des = self.designated(self.db.exprs[prefix].subtype, span)?;
        let h = self.scalar_expr(prefix)?;
        let what = self.msg("access value", Dir::To);
        let p = self.reg(RegTy::Ptr);
        self.emit(Inst::Deref {
            dst: p,
            handle: h,
            what,
        });
        Ok((p, des))
    }

    pub fn deref(&mut self, prefix: ExprId, span: Span) -> R<V> {
        let (p, des) = self.deref_obj(prefix, span)?;
        let tid = self.ty(self.db.base(des), span)?;
        let ds = self.desc_size(tid);
        let data = self.offset(p, ds);
        self.value_at(des, data, p, span)
    }

    pub fn deref_place(&mut self, prefix: ExprId, span: Span) -> R<Place> {
        let (p, des) = self.deref_obj(prefix, span)?;
        let tid = self.ty(self.db.base(des), span)?;
        let ds = self.desc_size(tid);
        let data = self.offset(p, ds);
        self.place_at(des, data, p, None, span)
    }

    /// `new T` / `new T'(v)` (LRM 9.3.7): a private heap object, its handle as the value.
    pub fn allocator(&mut self, init: &NewInit, st: SubtypeId, span: Span) -> R<V> {
        let db = self.db;
        let des = self.designated(st, span)?;
        let tid = self.ty(db.base(des), span)?;
        let (h, _, _) = match init {
            NewInit::Subtype(s) => {
                let Some(desc) = self.desc_of(*s, None, span)? else {
                    return Err(self.cx.ice(
                        span,
                        "allocator of an unconstrained subtype without a value",
                    ));
                };
                self.new_object(*s, tid, desc, None, false, span)?
            }
            NewInit::Value(e) => {
                let v = self.expr(*e)?;
                let desc = match v {
                    V::S(r) => {
                        self.check_scalar(r, Some(db.exprs[*e].subtype), des, "", span)?;
                        self.desc_of(des, None, span)?
                    }
                    V::A(_, vd) => {
                        let d = self.desc_of(des, Some(vd), span)?;
                        if let Some(d) = d {
                            self.check_desc(db.base(des), tid, vd, d, "allocator", span)?;
                        }
                        d
                    }
                };
                let Some(desc) = desc else {
                    return Err(self.cx.ice(span, "allocator without a descriptor"));
                };
                // A composite value whose descriptor is not in the object is copied with it.
                let v = match (v, scalar_w(self.fb.program(), tid)) {
                    (V::A(d, _), None) => V::A(d, desc),
                    (v, _) => v,
                };
                self.new_object(des, tid, desc, Some(v), false, span)?
            }
        };
        Ok(V::S(h))
    }

    /// `deallocate(p)`: frees the object and sets `p` to null.
    pub fn deallocate(&mut self, e: ExprId, span: Span) -> R<()> {
        let Place::Scalar { addr, .. } = self.place(e)? else {
            return Err(self.cx.ice(span, "deallocate of a composite"));
        };
        self.emit(Inst::Free { ptr: addr });
        Ok(())
    }

    /// A variable of an access type: an 8-byte handle slot, a GC root (rule 6), null by
    /// default.
    pub fn local_access(&mut self, d: yavs_sem::DeclId) -> R<()> {
        let db = self.db;
        let decl = &db.decls[d];
        let yavs_sem::decl::DeclKind::Object(o) = &decl.kind else {
            return Err(self.cx.ice(decl.span, "not an object"));
        };
        let tid = self.ty(db.base(o.subtype), decl.span)?;
        let off = self.fb.local(&decl.name.to_string(), tid, 8, 8);
        self.fb.root(off, yavs_ir::RootKind::Handle);
        let v = match o.default {
            Some(e) => self.scalar_expr(e)?,
            None => self.int(0),
        };
        let a = self.addr(crate::Space::Frame(self.depth), off);
        self.store(a, v, yavs_ir::Width::B8);
        self.cx.objs.insert(
            d,
            crate::Obj::Scalar {
                space: crate::Space::Frame(self.depth),
                off,
                w: yavs_ir::Width::B8,
            },
        );
        Ok(())
    }
}
