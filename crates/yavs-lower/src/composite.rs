//! Composites (DESIGN.md "Composites"): record and composite-element array types, their
//! runtime descriptors (rule 1), records, arrays of composites, multi-dimensional aggregates,
//! composite constants and objects.
//!
//! A composite value is `V::A(data, desc)` for records and arrays alike: `desc` holds the
//! descriptor bytes of its type (`Ty::desc_size`; empty for a record of static fields). A
//! heap object of type `T` is `[descriptor][value]` (yavs-interp's convention for arrays,
//! value-only for statically laid out records).

use yavs_diag::Span;
use yavs_ir::{
    AddrSpace, CmpOp, ConstData, Dir, Field, Inst, IntOp, LogicKind, ProgramBuilder, Reg, RegTy,
    RootKind, Ty, TyId, Width,
};
use yavs_sem::decl::DeclKind;
use yavs_sem::hir::{AggAssoc, AggBounds, Choice, ExprKind};
use yavs_sem::stat::{ConstValue, Elems};
use yavs_sem::types::{Constraint, Direction, RangeC, TypeKind};
use yavs_sem::{Db, DeclId, ExprId, SubtypeId, TypeId};

use crate::expr::{Dr, Place, V};
use crate::func::Fx;
use crate::{Cx, Failed, Obj, R, Space, ty};

// ---- types ----

/// Record, access, file and protected types, and incomplete types through their completion.
pub(crate) fn lower_ty(
    db: &Db,
    cx: &mut Cx,
    pb: &mut ProgramBuilder,
    t: TypeId,
    span: Span,
) -> R<TyId> {
    match &db.types[t].kind {
        TypeKind::Incomplete {
            completion: Some(c),
        } => {
            let id = ty::lower(db, cx, pb, *c, span)?;
            cx.tys.insert(t, id);
            Ok(id)
        }
        TypeKind::Record { elems } => {
            // Reserved first: a field may designate this record through an access type.
            let id = pb.ty(Ty::Record { fields: Vec::new() });
            cx.tys.insert(t, id);
            let mut fields = Vec::new();
            for e in elems.iter() {
                fields.push(field(db, cx, pb, e.name.text.to_string(), e.subtype, span)?);
            }
            pb.set_ty(id, Ty::Record { fields });
            Ok(id)
        }
        TypeKind::Access { designated } => {
            let id = pb.ty(Ty::Access { target: TyId(0) });
            cx.tys.insert(t, id);
            let target = ty::lower(db, cx, pb, db.base(*designated), span)?;
            pb.set_ty(id, Ty::Access { target });
            Ok(id)
        }
        TypeKind::File { elem } => {
            let elem = ty::lower(db, cx, pb, db.base(*elem), span)?;
            let id = pb.ty(Ty::File { elem });
            cx.tys.insert(t, id);
            Ok(id)
        }
        TypeKind::Protected { .. } => {
            let state = pb.ty(Ty::Record { fields: Vec::new() });
            let id = pb.ty(Ty::Protected { state });
            cx.tys.insert(t, id);
            let mut fields = Vec::new();
            for d in crate::protected::state_vars(db, t) {
                let DeclKind::Object(o) = &db.decls[d].kind else {
                    continue;
                };
                fields.push(field(
                    db,
                    cx,
                    pb,
                    db.decls[d].name.to_string(),
                    o.subtype,
                    span,
                )?);
            }
            pb.set_ty(state, Ty::Record { fields });
            Ok(id)
        }
        _ => Err(cx.unsupported(span, "generic and incomplete types")),
    }
}

fn field(
    db: &Db,
    cx: &mut Cx,
    pb: &mut ProgramBuilder,
    name: String,
    st: SubtypeId,
    span: Span,
) -> R<Field> {
    let fty = ty::lower(db, cx, pb, db.base(st), span)?;
    let desc = if Ty::needs_desc(pb.types(), fty) {
        static_desc(db, pb, db.base(st), &db.subtypes[st].constraint, fty)
            .map(|b| pb.konst(ConstData::Bytes(b)))
    } else {
        None
    };
    Ok(Field {
        name,
        ty: fty,
        desc,
    })
}

/// An array type whose elements are composite (or need a descriptor).
pub(crate) fn array_ty(
    db: &Db,
    cx: &mut Cx,
    pb: &mut ProgramBuilder,
    t: TypeId,
    elem_st: SubtypeId,
    elem: TyId,
    _span: Span,
) -> R<TyId> {
    let TypeKind::Array { indexes, .. } = &db.types[t].kind else {
        return Err(cx.ice(_span, "array_ty of a non-array"));
    };
    let elem_desc = if Ty::needs_desc(pb.types(), elem) {
        static_desc(
            db,
            pb,
            db.base(elem_st),
            &db.subtypes[elem_st].constraint,
            elem,
        )
        .map(|b| pb.konst(ConstData::Bytes(b)))
    } else {
        None
    };
    let id = pb.ty(Ty::Array {
        elem,
        elem_desc,
        dims: indexes.len() as u8,
    });
    cx.tys.insert(t, id);
    Ok(id)
}

fn dir_cell(d: Direction) -> i64 {
    match d {
        Direction::To => 0,
        Direction::Downto => 1,
    }
}

fn static_range(db: &Db, rc: &RangeC) -> Option<[i64; 3]> {
    let RangeC::Explicit { left, dir, right } = rc else {
        return None;
    };
    Some([
        db.value(*left)?.as_int()?,
        db.value(*right)?.as_int()?,
        dir_cell(*dir),
    ])
}

/// The constraint of element `i` of record type `t` under `c`, else the element's own.
fn field_constraint(db: &Db, t: TypeId, c: &Constraint, i: usize) -> Constraint {
    if let Constraint::Record { elems } = c
        && let Some((_, fc)) = elems.iter().find(|x| x.0 as usize == i)
    {
        return fc.clone();
    }
    match &db.types[t].kind {
        TypeKind::Record { elems } => db.subtypes[elems[i].subtype].constraint.clone(),
        _ => Constraint::None,
    }
}

/// The element constraint of array type `t` under `c`, else the type's element subtype's.
/// Callers pass array types only; a non-array falls back to the type's first subtype.
fn elem_constraint(db: &Db, t: TypeId, c: &Constraint) -> (SubtypeId, Constraint) {
    let TypeKind::Array { elem, .. } = &db.types[t].kind else {
        return (db.types[t].first_subtype, Constraint::None);
    };
    match c {
        Constraint::Array { elem: ec, .. } if !matches!(**ec, Constraint::None) => {
            (*elem, (**ec).clone())
        }
        _ => (*elem, db.subtypes[*elem].constraint.clone()),
    }
}

/// The descriptor bytes of type `t` (IR `tid`) under constraint `c` when every part is
/// locally static.
pub(crate) fn static_desc(
    db: &Db,
    pb: &ProgramBuilder,
    t: TypeId,
    c: &Constraint,
    tid: TyId,
) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    match pb.types().get(tid)?.clone() {
        Ty::Array {
            elem, elem_desc, ..
        } => {
            let Constraint::Array { dims: Some(ds), .. } = c else {
                return None;
            };
            for rc in ds.iter() {
                for x in static_range(db, rc)? {
                    out.extend_from_slice(&x.to_le_bytes());
                }
            }
            if elem_desc.is_none() && Ty::needs_desc(pb.types(), elem) {
                let (est, ec) = elem_constraint(db, t, c);
                out.extend(static_desc(db, pb, db.base(est), &ec, elem)?);
            }
        }
        Ty::Record { fields } => {
            let TypeKind::Record { elems } = &db.types[t].kind else {
                return None;
            };
            for (i, f) in fields.iter().enumerate() {
                if f.desc.is_none() && Ty::needs_desc(pb.types(), f.ty) {
                    let fc = field_constraint(db, t, c, i);
                    out.extend(static_desc(db, pb, db.base(elems[i].subtype), &fc, f.ty)?);
                }
            }
        }
        _ => {}
    }
    Some(out)
}

/// Whether `c` constrains every part of a descriptor of `t` (no actual needed).
fn covers(db: &Db, pb: &ProgramBuilder, t: TypeId, c: &Constraint, tid: TyId) -> bool {
    match pb.types().get(tid).cloned() {
        Some(Ty::Array {
            elem, elem_desc, ..
        }) => {
            matches!(c, Constraint::Array { dims: Some(_), .. })
                && (elem_desc.is_some() || !Ty::needs_desc(pb.types(), elem) || {
                    let (est, ec) = elem_constraint(db, t, c);
                    covers(db, pb, db.base(est), &ec, elem)
                })
        }
        Some(Ty::Record { fields }) => {
            let TypeKind::Record { elems } = &db.types[t].kind else {
                return false;
            };
            fields.iter().enumerate().all(|(i, f)| {
                f.desc.is_some()
                    || !Ty::needs_desc(pb.types(), f.ty)
                    || covers(
                        db,
                        pb,
                        db.base(elems[i].subtype),
                        &field_constraint(db, t, c, i),
                        f.ty,
                    )
            })
        }
        _ => true,
    }
}

/// The scalar width of an IR type, `None` for composites.
pub(crate) fn scalar_w(pb: &ProgramBuilder, tid: TyId) -> Option<Width> {
    pb.types()
        .get(tid)
        .filter(|t| t.scalar_size().is_some())
        .map(ty::width)
}

impl Fx<'_, '_> {
    // ---- type facts ----

    pub fn is_comp(&self, t: TypeId) -> bool {
        matches!(
            self.db.types[t].kind,
            TypeKind::Array { .. } | TypeKind::Record { .. }
        )
    }

    /// Composite types the scalar-element array paths do not handle: records, arrays whose
    /// elements are composite or hold access values.
    pub fn is_general(&mut self, t: TypeId, span: Span) -> R<bool> {
        let db = self.db;
        Ok(match &db.types[t].kind {
            TypeKind::Record { .. } => true,
            TypeKind::Array { elem, .. } => {
                let et = self.ty(db.base(*elem), span)?;
                !matches!(
                    self.fb.program().types().get(et),
                    Some(Ty::Int { .. } | Ty::Physical { .. } | Ty::Enum { .. } | Ty::Real)
                )
            }
            _ => false,
        })
    }

    pub fn desc_size(&mut self, tid: TyId) -> u32 {
        Ty::desc_size(self.fb.program().types(), tid).unwrap_or(0) as u32
    }

    fn const_addr(&mut self, c: yavs_ir::ConstId) -> Reg {
        let dst = self.reg(RegTy::Ptr);
        self.emit(Inst::AddrOf {
            dst,
            space: AddrSpace::Const(c),
            off: 0,
        });
        dst
    }

    fn bytes_addr(&mut self, b: Vec<u8>) -> Reg {
        let c = self.fb.program().konst(ConstData::Bytes(b));
        self.const_addr(c)
    }

    // ---- descriptors ----

    /// The descriptor of subtype `st`, its unconstrained parts taken from `actual` (a
    /// descriptor of the same type); `None` when `st` leaves parts open and there is no
    /// actual.
    pub fn desc_of(&mut self, st: SubtypeId, actual: Option<Reg>, span: Span) -> R<Option<Reg>> {
        let db = self.db;
        let t = db.base(st);
        let tid = self.ty(t, span)?;
        let c = db.subtypes[st].constraint.clone();
        self.desc_c(t, &c, tid, actual, span)
    }

    pub fn desc_c(
        &mut self,
        t: TypeId,
        c: &Constraint,
        tid: TyId,
        actual: Option<Reg>,
        span: Span,
    ) -> R<Option<Reg>> {
        let ds = self.desc_size(tid);
        if ds == 0 {
            return Ok(Some(self.bytes_addr(Vec::new())));
        }
        if let Constraint::FromObject {
            expr,
            elem_depth: 0,
        } = c
        {
            return self.dope_of(*expr).map(Some);
        }
        if let Some(b) = static_desc(self.db, self.fb.program(), t, c, tid) {
            return Ok(Some(self.bytes_addr(b)));
        }
        if actual.is_none() && !covers(self.db, self.fb.program(), t, c, tid) {
            return Ok(None);
        }
        let dst = self.alloc_n(ds);
        self.fill_desc(t, c, tid, dst, actual, span)?;
        Ok(Some(dst))
    }

    fn fill_desc(
        &mut self,
        t: TypeId,
        c: &Constraint,
        tid: TyId,
        dst: Reg,
        actual: Option<Reg>,
        span: Span,
    ) -> R<()> {
        let db = self.db;
        let Some(ty) = self.fb.program().types().get(tid).cloned() else {
            return Err(self.cx.ice(span, "descriptor of a missing type"));
        };
        match ty {
            Ty::Array {
                elem,
                elem_desc,
                dims,
            } => {
                match c {
                    Constraint::Array { dims: Some(ds), .. } => {
                        for (k, rc) in ds.iter().enumerate() {
                            let r = self.range(rc, span)?;
                            let d = self.dir_reg(r.dir);
                            let at = self.offset(dst, 24 * k as u32);
                            self.store(at, r.left, Width::B8);
                            let at = self.offset(dst, 24 * k as u32 + 8);
                            self.store(at, r.right, Width::B8);
                            let at = self.offset(dst, 24 * k as u32 + 16);
                            self.store(at, d, Width::B8);
                        }
                    }
                    _ => {
                        let Some(a) = actual else {
                            return Err(self.cx.ice(span, "unconstrained object without bounds"));
                        };
                        let n = self.int(24 * i64::from(dims));
                        self.emit(Inst::Copy {
                            dst,
                            src: a,
                            bytes: n,
                        });
                    }
                }
                if elem_desc.is_none() && Ty::needs_desc(self.fb.program().types(), elem) {
                    let (est, ec) = elem_constraint(db, t, c);
                    let off = 24 * u32::from(dims);
                    let d2 = self.offset(dst, off);
                    let a2 = actual.map(|a| self.offset(a, off));
                    self.fill_desc(db.base(est), &ec, elem, d2, a2, span)?;
                }
            }
            Ty::Record { fields } => {
                let TypeKind::Record { elems } = &db.types[t].kind else {
                    return Err(self.cx.ice(span, "record descriptor of a non-record"));
                };
                let mut off = 0;
                for (i, f) in fields.iter().enumerate() {
                    if f.desc.is_some() || !Ty::needs_desc(self.fb.program().types(), f.ty) {
                        continue;
                    }
                    let fc = field_constraint(db, t, c, i);
                    let d2 = self.offset(dst, off);
                    let a2 = actual.map(|a| self.offset(a, off));
                    self.fill_desc(db.base(elems[i].subtype), &fc, f.ty, d2, a2, span)?;
                    off += self.desc_size(f.ty);
                }
            }
            _ => {}
        }
        Ok(())
    }

    /// A copy of the `desc_size(tid)` descriptor bytes at `desc` in temporary memory.
    pub fn copy_desc(&mut self, tid: TyId, desc: Reg) -> Reg {
        let ds = self.desc_size(tid);
        let dst = self.alloc_n(ds.max(1));
        let n = self.int(i64::from(ds));
        self.emit(Inst::Copy {
            dst,
            src: desc,
            bytes: n,
        });
        dst
    }

    /// The layout words `[size, align, stride | offsets...]` of `tid` under `desc`.
    pub fn lay(&mut self, tid: TyId, desc: Reg) -> Reg {
        let l = self.reg(RegTy::Ptr);
        self.emit(Inst::LayoutOf {
            dst: l,
            ty: tid,
            desc,
        });
        l
    }

    pub fn size_of(&mut self, tid: TyId, desc: Reg) -> Reg {
        let l = self.lay(tid, desc);
        self.load(l, Width::B8, RegTy::I64)
    }

    // ---- element and field access ----

    /// Element descriptor of an array of type `tid` with descriptor `dope`.
    pub(crate) fn elem_desc(&mut self, tid: TyId, dope: Reg) -> R<(TyId, Reg)> {
        match self.fb.program().types().get(tid).cloned() {
            Some(Ty::Array {
                elem,
                elem_desc,
                dims,
            }) => {
                let d = match elem_desc {
                    Some(c) => self.const_addr(c),
                    None => self.offset(dope, 24 * u32::from(dims)),
                };
                Ok((elem, d))
            }
            _ => Err(self.cx.ice(Span::default(), "element of a non-array")),
        }
    }

    /// The value of type `st` at `addr` (scalars loaded, composites by reference).
    pub fn value_at(&mut self, st: SubtypeId, addr: Reg, desc: Reg, span: Span) -> R<V> {
        let db = self.db;
        let t = db.base(st);
        let tid = self.ty(t, span)?;
        match scalar_w(self.fb.program(), tid) {
            Some(w) => {
                let rt = if ty::is_real(db, t) {
                    RegTy::F64
                } else {
                    RegTy::I64
                };
                Ok(V::S(self.load(addr, w, rt)))
            }
            None => Ok(V::A(addr, desc)),
        }
    }

    /// Store value `v` of type `tid` at `addr` (composites copied whole).
    pub fn store_at(&mut self, tid: TyId, addr: Reg, v: V, span: Span) -> R<()> {
        match (scalar_w(self.fb.program(), tid), v) {
            (Some(w), V::S(r)) => {
                self.store(addr, r, w);
                Ok(())
            }
            (None, V::A(data, desc)) => {
                let size = self.size_of(tid, desc);
                self.emit(Inst::Copy {
                    dst: addr,
                    src: data,
                    bytes: size,
                });
                Ok(())
            }
            _ => Err(self.cx.ice(span, "value of the wrong shape")),
        }
    }

    /// Record type facts of the record subtype of expression `e`.
    fn record_of(&mut self, e: ExprId, span: Span) -> R<(TypeId, TyId)> {
        let t = self.db.base(self.db.exprs[e].subtype);
        let tid = self.ty(t, span)?;
        Ok((t, tid))
    }

    /// Address and descriptor of field `n` of the record at `(base, desc)`.
    pub fn field_addr(
        &mut self,
        tid: TyId,
        base: Reg,
        desc: Reg,
        n: u32,
        span: Span,
    ) -> R<(Reg, Reg)> {
        let d = self.field_desc(tid, desc, n, span)?;
        let l = self.lay(tid, desc);
        let a = self.reg(RegTy::Ptr);
        self.emit(Inst::Field {
            dst: a,
            base,
            layout: l,
            n,
        });
        Ok((a, d))
    }

    /// Descriptor of field `n` of a record of type `tid` with descriptor `desc`.
    pub fn field_desc(&mut self, tid: TyId, desc: Reg, n: u32, span: Span) -> R<Reg> {
        let Some(Ty::Record { fields }) = self.fb.program().types().get(tid).cloned() else {
            return Err(self.cx.ice(span, "field of a non-record"));
        };
        let f = &fields[n as usize];
        Ok(match f.desc {
            Some(c) => self.const_addr(c),
            None => {
                let mut off = 0;
                for g in &fields[..n as usize] {
                    if g.desc.is_none() {
                        off += self.desc_size(g.ty);
                    }
                }
                self.offset(desc, off)
            }
        })
    }

    pub(crate) fn field_subtype(&mut self, t: TypeId, n: u32) -> R<SubtypeId> {
        match &self.db.types[t].kind {
            TypeKind::Record { elems } if (n as usize) < elems.len() => {
                Ok(elems[n as usize].subtype)
            }
            _ => Err(self.cx.ice(Span::default(), "field of a non-record type")),
        }
    }

    pub fn field(&mut self, prefix: ExprId, n: u32, span: Span) -> R<V> {
        let (t, tid) = self.record_of(prefix, span)?;
        let v = self.expr(prefix)?;
        let (base, desc) = self.arr(v, span)?;
        let (a, d) = self.field_addr(tid, base, desc, n, span)?;
        let st = self.field_subtype(t, n)?;
        self.value_at(st, a, d, span)
    }

    pub fn field_place(&mut self, prefix: ExprId, n: u32, span: Span) -> R<Place> {
        let (t, tid) = self.record_of(prefix, span)?;
        let Place::Array {
            data, dope: desc, ..
        } = self.place(prefix)?
        else {
            return Err(self.cx.ice(span, "field of a non-record place"));
        };
        let (a, d) = self.field_addr(tid, data, desc, n, span)?;
        let st = self.field_subtype(t, n)?;
        self.place_at(st, a, d, None, span)
    }

    /// The place of a value of subtype `st` at `addr`; `off` is a plane offset for logic
    /// arrays nested in a logic array.
    pub fn place_at(
        &mut self,
        st: SubtypeId,
        addr: Reg,
        desc: Reg,
        off: Option<Reg>,
        span: Span,
    ) -> R<Place> {
        let db = self.db;
        let t = db.base(st);
        let tid = self.ty(t, span)?;
        match scalar_w(self.fb.program(), tid) {
            Some(w) => Ok(Place::Scalar { addr, w, st }),
            None => Ok(Place::Array {
                data: addr,
                dope: desc,
                off,
                tid,
                dims: ty::array_of(db, t).map_or(0, |x| x.1),
            }),
        }
    }

    fn logic(&mut self, tid: TyId) -> bool {
        Ty::logic_leaf(self.fb.program().types(), tid).is_some()
    }

    /// Element `pos` of an array of composites (or of logic arrays, a row of planes).
    pub fn index_comp(&mut self, prefix: ExprId, indices: &[ExprId], span: Span) -> R<V> {
        let db = self.db;
        let t = db.base(db.exprs[prefix].subtype);
        let tid = self.ty(t, span)?;
        let what = self.prefix_name(prefix);
        let v = self.expr(prefix)?;
        let (data, dope) = self.arr(v, span)?;
        let pos = self.flat_pos(dope, indices, &what)?;
        let (etid, ed) = self.elem_desc(tid, dope)?;
        let est = ty::array_of(db, t).ok_or(Failed)?.0;
        if self.logic(tid) {
            let (_, total) = self.layout(tid, dope);
            let (size, m) = self.layout(etid, ed);
            let src_off = self.row_off(total, m, pos, None);
            let dst = self.alloc(size);
            let z = self.int(0);
            self.emit(Inst::LCopy {
                dst,
                dst_off: z,
                src: data,
                src_off,
                n: m,
            });
            return Ok(V::A(dst, ed));
        }
        let a = self.elem_addr(tid, data, dope, pos);
        self.value_at(est, a, ed, span)
    }

    /// Plane offset of row `pos` (from the left) of `m` elements in `total`.
    fn row_off(&mut self, total: Reg, m: Reg, pos: Reg, outer: Option<Reg>) -> Reg {
        let one = self.int(1);
        let p1 = self.bin(IntOp::Add, pos, one);
        let k = self.bin(IntOp::Mul, p1, m);
        let o = self.bin(IntOp::Sub, total, k);
        match outer {
            Some(b) => self.bin(IntOp::Add, o, b),
            None => o,
        }
    }

    pub(crate) fn elem_addr(&mut self, tid: TyId, data: Reg, dope: Reg, pos: Reg) -> Reg {
        let l = self.lay(tid, dope);
        let sa = self.offset(l, 16);
        let stride = self.load(sa, Width::B8, RegTy::I64);
        let off = self.bin(IntOp::Mul, pos, stride);
        let a = self.reg(RegTy::Ptr);
        self.emit(Inst::Index {
            dst: a,
            base: data,
            pos: off,
            stride: 1,
        });
        a
    }

    pub fn index_place_comp(&mut self, prefix: ExprId, indices: &[ExprId], span: Span) -> R<Place> {
        let db = self.db;
        let t = db.base(db.exprs[prefix].subtype);
        let what = self.prefix_name(prefix);
        let Place::Array {
            data,
            dope,
            off,
            tid,
            ..
        } = self.place(prefix)?
        else {
            return Err(self.cx.ice(span, "indexed non-array"));
        };
        let pos = self.flat_pos(dope, indices, &what)?;
        let (etid, ed) = self.elem_desc(tid, dope)?;
        let est = ty::array_of(db, t).ok_or(Failed)?.0;
        if self.logic(tid) {
            let (_, total) = self.layout(tid, dope);
            let (_, m) = self.layout(etid, ed);
            let o = self.row_off(total, m, pos, off);
            return self.place_at(est, data, ed, Some(o), span);
        }
        let a = self.elem_addr(tid, data, dope, pos);
        self.place_at(est, a, ed, None, span)
    }

    /// A slice of an array of composites: the descriptor keeps the element part.
    pub(crate) fn slice_desc(&mut self, tid: TyId, dope: Reg, nd: Reg) -> Reg {
        let ds = self.desc_size(tid);
        if ds <= 24 {
            return nd;
        }
        let d = self.alloc_n(ds);
        let n = self.int(24);
        self.emit(Inst::Copy {
            dst: d,
            src: nd,
            bytes: n,
        });
        let (src, dst) = (self.offset(dope, 24), self.offset(d, 24));
        let n = self.int(i64::from(ds - 24));
        self.emit(Inst::Copy { dst, src, bytes: n });
        d
    }

    pub fn slice_comp(&mut self, prefix: ExprId, range: &RangeC, span: Span) -> R<V> {
        let db = self.db;
        let t = db.base(db.exprs[prefix].subtype);
        let tid = self.ty(t, span)?;
        let what = self.prefix_name(prefix);
        let v = self.expr(prefix)?;
        let (data, dope) = self.arr(v, span)?;
        let (p, n, nd) = self.slice_of(dope, range, &what, span)?;
        let d = self.slice_desc(tid, dope, nd);
        if self.logic(tid) {
            let (etid, ed) = self.elem_desc(tid, dope)?;
            let (_, total) = self.layout(tid, dope);
            let (_, m) = self.layout(etid, ed);
            let last = self.bin(IntOp::Add, p, n);
            let one = self.int(1);
            let lm1 = self.bin(IntOp::Sub, last, one);
            let src_off = self.row_off(total, m, lm1, None);
            let (size, count) = self.layout(tid, d);
            let dst = self.alloc(size);
            let z = self.int(0);
            self.emit(Inst::LCopy {
                dst,
                dst_off: z,
                src: data,
                src_off,
                n: count,
            });
            return Ok(V::A(dst, d));
        }
        let a = self.elem_addr(tid, data, dope, p);
        Ok(V::A(a, d))
    }

    pub fn slice_place_comp(&mut self, prefix: ExprId, range: &RangeC, span: Span) -> R<Place> {
        let what = self.prefix_name(prefix);
        let Place::Array {
            data,
            dope,
            off,
            tid,
            dims,
        } = self.place(prefix)?
        else {
            return Err(self.cx.ice(span, "sliced non-array"));
        };
        let (p, n, nd) = self.slice_of(dope, range, &what, span)?;
        let d = self.slice_desc(tid, dope, nd);
        if self.logic(tid) {
            let (etid, ed) = self.elem_desc(tid, dope)?;
            let (_, total) = self.layout(tid, dope);
            let (_, m) = self.layout(etid, ed);
            let last = self.bin(IntOp::Add, p, n);
            let one = self.int(1);
            let lm1 = self.bin(IntOp::Sub, last, one);
            let o = self.row_off(total, m, lm1, off);
            return Ok(Place::Array {
                data,
                dope: d,
                off: Some(o),
                tid,
                dims,
            });
        }
        let a = self.elem_addr(tid, data, dope, p);
        Ok(Place::Array {
            data: a,
            dope: d,
            off: None,
            tid,
            dims,
        })
    }

    // ---- equality ----

    /// `a = b` for composites of type `t` (LRM 9.2.3: same length and equal elements).
    pub fn comp_eq(&mut self, t: TypeId, a: V, b: V, span: Span) -> R<Reg> {
        let tid = self.ty(t, span)?;
        let (ad, ap) = self.arr(a, span)?;
        let (bd, bp) = self.arr(b, span)?;
        let r = self.reg(RegTy::I64);
        self.emit(Inst::Const { dst: r, v: 1 });
        let done = self.block();
        self.eq_into(t, tid, (ad, ap), (bd, bp), r, done, span)?;
        if self.live {
            self.jump(done);
        }
        self.at(done);
        Ok(r)
    }

    /// Compare; on a difference set `r` to 0 and jump to `done`.
    #[allow(clippy::too_many_arguments)]
    fn eq_into(
        &mut self,
        t: TypeId,
        tid: TyId,
        a: (Reg, Reg),
        b: (Reg, Reg),
        r: Reg,
        done: yavs_ir::BlockId,
        span: Span,
    ) -> R<()> {
        let db = self.db;
        let ne = |fx: &mut Self, same: Reg| {
            let (diff, cont) = (fx.block(), fx.block());
            fx.branch(same, cont, diff);
            fx.at(diff);
            fx.emit(Inst::Const { dst: r, v: 0 });
            fx.jump(done);
            fx.at(cont);
        };
        match self.fb.program().types().get(tid).cloned() {
            Some(Ty::Record { fields }) => {
                for (n, f) in fields.iter().enumerate() {
                    let st = self.field_subtype(t, n as u32)?;
                    let (fa, fda) = self.field_addr(tid, a.0, a.1, n as u32, span)?;
                    let (fb, fdb) = self.field_addr(tid, b.0, b.1, n as u32, span)?;
                    match scalar_w(self.fb.program(), f.ty) {
                        Some(w) => {
                            let real = ty::is_real(db, db.base(st));
                            let rt = if real { RegTy::F64 } else { RegTy::I64 };
                            let x = self.load(fa, w, rt);
                            let y = self.load(fb, w, rt);
                            let s = self.reg(RegTy::I64);
                            if real {
                                self.emit(Inst::FCmp {
                                    op: CmpOp::Eq,
                                    dst: s,
                                    a: x,
                                    b: y,
                                });
                            } else {
                                self.emit(Inst::Cmp {
                                    op: CmpOp::Eq,
                                    dst: s,
                                    a: x,
                                    b: y,
                                });
                            }
                            ne(self, s);
                        }
                        None => {
                            self.eq_into(db.base(st), f.ty, (fa, fda), (fb, fdb), r, done, span)?
                        }
                    }
                }
            }
            Some(Ty::Array { dims, .. }) => {
                // Lengths per dimension, then the elements.
                for k in 0..dims {
                    let la = self.reg(RegTy::I64);
                    self.emit(Inst::DopeLen {
                        dst: la,
                        dope: a.1,
                        dim: k,
                    });
                    let lb = self.reg(RegTy::I64);
                    self.emit(Inst::DopeLen {
                        dst: lb,
                        dope: b.1,
                        dim: k,
                    });
                    let s = self.cmp(CmpOp::Eq, la, lb);
                    ne(self, s);
                }
                if self.logic(tid) {
                    let (_, n) = self.layout(tid, a.1);
                    let s = self.reg(RegTy::I64);
                    self.emit(Inst::LEq {
                        dst: s,
                        a: a.0,
                        b: b.0,
                        n,
                    });
                    ne(self, s);
                    return Ok(());
                }
                let (est, _) = ty::array_of(db, t).ok_or(Failed)?;
                let (etid, eda) = self.elem_desc(tid, a.1)?;
                let (_, edb) = self.elem_desc(tid, b.1)?;
                let (_, count) = self.layout(tid, a.1);
                let ew = scalar_w(self.fb.program(), etid);
                let real = ty::is_real(db, db.base(est));
                self.for_n(count, |fx, i| {
                    let xa = fx.elem_addr(tid, a.0, a.1, i);
                    let xb = fx.elem_addr(tid, b.0, b.1, i);
                    match ew {
                        Some(w) => {
                            let rt = if real { RegTy::F64 } else { RegTy::I64 };
                            let x = fx.load(xa, w, rt);
                            let y = fx.load(xb, w, rt);
                            let s = fx.reg(RegTy::I64);
                            if real {
                                fx.emit(Inst::FCmp {
                                    op: CmpOp::Eq,
                                    dst: s,
                                    a: x,
                                    b: y,
                                });
                            } else {
                                fx.emit(Inst::Cmp {
                                    op: CmpOp::Eq,
                                    dst: s,
                                    a: x,
                                    b: y,
                                });
                            }
                            ne(fx, s);
                            Ok(())
                        }
                        None => fx.eq_into(db.base(est), etid, (xa, eda), (xb, edb), r, done, span),
                    }
                })?;
            }
            _ => return Err(self.cx.ice(span, "composite equality of a scalar")),
        }
        Ok(())
    }

    // ---- default values ----

    /// Write the default value (LRM 6.4.2.3: `'left` of each scalar) of `st` at `addr`.
    pub fn default_into(
        &mut self,
        st: SubtypeId,
        c: &Constraint,
        addr: Reg,
        desc: Reg,
        span: Span,
    ) -> R<()> {
        let db = self.db;
        let t = db.base(st);
        let tid = self.ty(t, span)?;
        if let Some(w) = scalar_w(self.fb.program(), tid) {
            let v = match &db.types[t].kind {
                TypeKind::Access { .. } | TypeKind::Protected { .. } | TypeKind::File { .. } => {
                    self.int(0)
                }
                _ => self.scalar_left(st, span)?,
            };
            self.store(addr, v, w);
            return Ok(());
        }
        match self.fb.program().types().get(tid).cloned() {
            Some(Ty::Record { fields }) => {
                for (n, f) in fields.iter().enumerate() {
                    let fst = self.field_subtype(t, n as u32)?;
                    let fc = field_constraint(db, t, c, n);
                    let (a, d) = self.field_addr(tid, addr, desc, n as u32, span)?;
                    let _ = f;
                    self.default_into(fst, &fc, a, d, span)?;
                }
            }
            Some(Ty::Array { .. }) => {
                let (est, ec) = elem_constraint(db, t, c);
                let (etid, ed) = self.elem_desc(tid, desc)?;
                if let Some(leaf) = Ty::logic_leaf(self.fb.program().types(), tid) {
                    let (_, count) = self.layout(tid, desc);
                    let mut s = est;
                    while let Some((e, _)) = ty::array_of(db, db.base(s)) {
                        s = e;
                    }
                    let v = self.scalar_left(s, span)?;
                    let bias = if leaf == LogicKind::StdULogic { 0 } else { 2 };
                    let code = self.enc(v, bias);
                    self.for_n(count, |fx, i| {
                        fx.emit(Inst::LSet {
                            base: addr,
                            idx: i,
                            v: code,
                        });
                        Ok(())
                    })?;
                    return Ok(());
                }
                let (_, count) = self.layout(tid, desc);
                let _ = etid;
                self.for_n(count, |fx, i| {
                    let a = fx.elem_addr(tid, addr, desc, i);
                    fx.default_into(est, &ec, a, ed, span)
                })?;
            }
            _ => return Err(self.cx.ice(span, "default of an unknown type")),
        }
        Ok(())
    }

    // ---- objects ----

    /// A heap object `[descriptor][value]` holding a copy of `v` (or the default value of
    /// `st` when `v` is `None`); returns the handle, the data address and the descriptor
    /// address inside the object.
    pub fn new_object(
        &mut self,
        st: SubtypeId,
        tid: TyId,
        desc: Reg,
        v: Option<V>,
        shared: bool,
        span: Span,
    ) -> R<(Reg, Reg, Reg)> {
        let ds = self.desc_size(tid);
        let size = match scalar_w(self.fb.program(), tid) {
            Some(w) => self.int(i64::from(w.bytes())),
            None => self.size_of(tid, desc),
        };
        let dsr = self.int(i64::from(ds));
        let total = self.bin(IntOp::Add, size, dsr);
        let buf = self.alloc(total);
        self.emit(Inst::Copy {
            dst: buf,
            src: desc,
            bytes: dsr,
        });
        let vat = self.offset(buf, ds);
        match v {
            Some(V::A(d, _)) if self.logic(tid) && scalar_w(self.fb.program(), tid).is_none() => {
                let (_, count) = self.layout(tid, desc);
                let z = self.int(0);
                self.emit(Inst::LCopy {
                    dst: vat,
                    dst_off: z,
                    src: d,
                    src_off: z,
                    n: count,
                });
            }
            Some(v) => self.store_at(tid, vat, v, span)?,
            None => {
                let c = self.db.subtypes[st].constraint.clone();
                self.default_into(st, &c, vat, buf, span)?
            }
        }
        let h = self.reg(RegTy::I64);
        self.emit(Inst::New {
            dst: h,
            ty: tid,
            init: buf,
            size: total,
            shared,
        });
        let what = self.msg("object", Dir::To);
        let p = self.reg(RegTy::Ptr);
        self.emit(Inst::Deref {
            dst: p,
            handle: h,
            what,
        });
        let data = self.offset(p, ds);
        Ok((h, data, p))
    }

    /// A variable or constant of a record type or an array of composites: a frame slot
    /// `[data, desc, handle]` over a private heap object (rule 6).
    pub fn local_comp(&mut self, d: DeclId) -> R<()> {
        let db = self.db;
        let decl = &db.decls[d];
        let DeclKind::Object(o) = &decl.kind else {
            return Err(self.cx.ice(decl.span, "not an object"));
        };
        let span = decl.span;
        let name = decl.name.to_string();
        let t = db.base(o.subtype);
        let tid = self.ty(t, span)?;
        let off = self.fb.local(&name, tid, 24, 8);
        self.fb.root(off + 16, RootKind::Handle);
        let init = match o.default {
            Some(e) => Some(self.expr(e)?),
            None => None,
        };
        let vd = match init {
            Some(V::A(_, vd)) => Some(vd),
            Some(V::S(_)) => return Err(self.cx.ice(span, "scalar initial value of a composite")),
            None => None,
        };
        let Some(desc) = self.desc_of(o.subtype, vd, span)? else {
            return Err(self.cx.ice(span, "unconstrained object without bounds"));
        };
        if let Some(vd) = vd {
            self.check_desc(t, tid, vd, desc, &ty::obj_desc(db, d), span)?;
        }
        let (h, data, p) = self.new_object(o.subtype, tid, desc, init, false, span)?;
        let slot = self.addr(Space::Frame(self.depth), off);
        self.store(slot, data, Width::B8);
        let ds = self.offset(slot, 8);
        self.store(ds, p, Width::B8);
        let hs = self.offset(slot, 16);
        self.store(hs, h, Width::B8);
        self.cx.objs.insert(
            d,
            Obj::Array {
                space: Space::Frame(self.depth),
                off,
            },
        );
        Ok(())
    }

    /// Length checks of descriptor `vd` against `want` (implicit subtype conversion): the
    /// array dimensions, element by element for records.
    pub fn check_desc(
        &mut self,
        t: TypeId,
        tid: TyId,
        vd: Reg,
        want: Reg,
        what: &str,
        _span: Span,
    ) -> R<()> {
        let db = self.db;
        match self.fb.program().types().get(tid).cloned() {
            Some(Ty::Array {
                elem,
                elem_desc,
                dims,
            }) => {
                let m = self.msg(what, Dir::To);
                self.emit(Inst::SubtypeCheck {
                    dope: vd,
                    want,
                    dims,
                    what: m,
                });
                if elem_desc.is_none() && Ty::needs_desc(self.fb.program().types(), elem) {
                    let (est, _) = elem_constraint(db, t, &Constraint::None);
                    let o = 24 * u32::from(dims);
                    let (a, b) = (self.offset(vd, o), self.offset(want, o));
                    self.check_desc(db.base(est), elem, a, b, what, _span)?;
                }
            }
            Some(Ty::Record { fields }) => {
                let mut off = 0;
                for (n, f) in fields.iter().enumerate() {
                    if f.desc.is_some() || !Ty::needs_desc(self.fb.program().types(), f.ty) {
                        continue;
                    }
                    let fst = self.field_subtype(t, n as u32)?;
                    let (a, b) = (self.offset(vd, off), self.offset(want, off));
                    self.check_desc(db.base(fst), f.ty, a, b, what, _span)?;
                    off += self.desc_size(f.ty);
                }
            }
            _ => {}
        }
        Ok(())
    }

    // ---- aggregates ----

    /// Record aggregates, aggregates of composite elements and multi-dimensional ones.
    pub fn aggregate_comp(
        &mut self,
        assocs: &[AggAssoc],
        bounds: &AggBounds,
        st: SubtypeId,
        span: Span,
    ) -> R<V> {
        let db = self.db;
        let t = db.base(st);
        let tid = self.ty(t, span)?;
        if let TypeKind::Record { elems } = &db.types[t].kind {
            return self.record_agg(t, tid, elems.len(), assocs, st, span);
        }
        let TypeKind::Array { indexes, .. } = &db.types[t].kind else {
            return Err(self.cx.ice(span, "aggregate of a scalar type"));
        };
        let dims = indexes.len() as u8;
        if assocs.iter().any(|a| a.slice_piece) {
            return self.aggregate_pieces(assocs, bounds, st, span);
        }
        // Descriptor: the context subtype's, else from the bounds of each level.
        // Named bounds: sem's result subtype has the direction of a constrained context.
        let desc = match bounds {
            AggBounds::Context | AggBounds::Named { .. } => self.desc_of(st, None, span)?,
            _ => None,
        };
        let desc = match desc {
            Some(d) => d,
            None => self.agg_desc(t, tid, assocs, bounds, dims, span)?,
        };
        let (size, _) = self.layout(tid, desc);
        let data = self.alloc(size);
        self.agg_fill(t, tid, assocs, 0, dims, data, desc, None, span)?;
        Ok(V::A(data, desc))
    }

    /// Range of one aggregate level from its bounds (LRM 9.3.3.3).
    fn level_range(
        &mut self,
        bounds: &AggBounds,
        index: SubtypeId,
        span: Span,
    ) -> R<(Reg, Reg, Reg)> {
        let s = self.scalar_bounds(index, span)?;
        let Dr::Static(dir) = s.dir else {
            return Err(self.cx.ice(span, "index subtype with a runtime direction"));
        };
        Ok(match bounds {
            AggBounds::Positional { count } => {
                let n = self.scalar_expr(*count)?;
                let one = self.int(1);
                let last = self.bin(IntOp::Sub, n, one);
                let right = match dir {
                    Direction::To => self.bin(IntOp::Add, s.left, last),
                    Direction::Downto => self.bin(IntOp::Sub, s.left, last),
                };
                let d = self.int(dir_cell(dir));
                (s.left, right, d)
            }
            AggBounds::Named { low, high } => {
                let lo = self.scalar_expr(*low)?;
                let hi = self.scalar_expr(*high)?;
                let d = self.int(dir_cell(dir));
                match dir {
                    Direction::To => (lo, hi, d),
                    Direction::Downto => (hi, lo, d),
                }
            }
            _ => return Err(self.cx.unsupported(span, "this aggregate form")),
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn agg_desc(
        &mut self,
        t: TypeId,
        tid: TyId,
        assocs: &[AggAssoc],
        bounds: &AggBounds,
        dims: u8,
        span: Span,
    ) -> R<Reg> {
        let db = self.db;
        let TypeKind::Array { indexes, .. } = &db.types[t].kind else {
            return Err(self.cx.ice(span, "aggregate of a non-array"));
        };
        let indexes = indexes.clone();
        let ds = self.desc_size(tid);
        let desc = self.alloc_n(ds);
        let mut cur_bounds: Option<&AggBounds> = Some(bounds);
        let mut cur_assocs: &[AggAssoc] = assocs;
        for k in 0..dims as usize {
            let Some(b) = cur_bounds else {
                return Err(self
                    .cx
                    .ice(span, "multi-dimensional aggregate without rows"));
            };
            let (l, r, d) = if matches!(b, AggBounds::Context) {
                return Err(self
                    .cx
                    .unsupported(span, "`others` in a row of an unconstrained aggregate"));
            } else {
                self.level_range(b, indexes[k], span)?
            };
            for (j, x) in [l, r, d].into_iter().enumerate() {
                let at = self.offset(desc, (24 * k + 8 * j) as u32);
                self.store(at, x, Width::B8);
            }
            // The next level: the first row's aggregate.
            if k + 1 < dims as usize {
                let Some(first) = cur_assocs.first() else {
                    return Err(self
                        .cx
                        .unsupported(span, "null rows in a multi-dimensional aggregate"));
                };
                match &db.exprs[first.value].kind {
                    ExprKind::Aggregate { assocs, bounds } => {
                        cur_assocs = assocs;
                        cur_bounds = Some(bounds);
                    }
                    _ => {
                        return Err(self.cx.unsupported(
                            span,
                            "rows of a multi-dimensional aggregate that are not aggregates",
                        ));
                    }
                }
            }
        }
        // The element part of the descriptor: from the first element's value.
        let off = 24 * u32::from(dims);
        if ds > off {
            let Some(first) = cur_assocs.first() else {
                return Err(self
                    .cx
                    .unsupported(span, "null aggregate of unconstrained elements"));
            };
            let v = self.expr(first.value)?;
            let (_, ed) = self.arr(v, span)?;
            let at = self.offset(desc, off);
            let n = self.int(i64::from(ds - off));
            self.emit(Inst::Copy {
                dst: at,
                src: ed,
                bytes: n,
            });
        }
        Ok(desc)
    }

    /// Fill level `k` of an array aggregate; `base` is the flat position of this row.
    #[allow(clippy::too_many_arguments)]
    fn agg_fill(
        &mut self,
        t: TypeId,
        tid: TyId,
        assocs: &[AggAssoc],
        k: u8,
        dims: u8,
        data: Reg,
        desc: Reg,
        base: Option<Reg>,
        span: Span,
    ) -> R<()> {
        let db = self.db;
        let (est, _) = ty::array_of(db, t).ok_or(Failed)?;
        let len = self.reg(RegTy::I64);
        self.emit(Inst::DopeLen {
            dst: len,
            dope: desc,
            dim: k,
        });
        let flat = |fx: &mut Self, pos: Reg| -> Reg {
            match base {
                Some(b) => {
                    let m = fx.bin(IntOp::Mul, b, len);
                    fx.bin(IntOp::Add, m, pos)
                }
                None => pos,
            }
        };
        let last = k + 1 == dims;
        let m = self.msg("aggregate", Dir::To);
        let put = |fx: &mut Self, pos: Reg, e: ExprId| -> R<()> {
            let f = flat(fx, pos);
            if last {
                fx.agg_elem(tid, est, data, desc, f, e, span)
            } else {
                match &db.exprs[e].kind {
                    ExprKind::Aggregate { assocs, .. } if db.exprs[e].value.is_none() => {
                        fx.agg_fill(t, tid, assocs, k + 1, dims, data, desc, Some(f), span)
                    }
                    _ => Err(fx.cx.unsupported(
                        span,
                        "rows of a multi-dimensional aggregate that are not aggregates",
                    )),
                }
            }
        };
        for a in assocs
            .iter()
            .filter(|a| a.choices.iter().any(|c| matches!(c, Choice::Others)))
        {
            let e = a.value;
            self.for_n(len, |fx, i| put(fx, i, e))?;
        }
        let mut n = 0i64;
        for a in assocs.iter() {
            if a.choices.iter().any(|c| matches!(c, Choice::Others)) {
                continue;
            }
            if a.choices.is_empty() {
                let pos = self.int(n);
                n += 1;
                put(self, pos, a.value)?;
                continue;
            }
            for c in a.choices.iter() {
                match c {
                    Choice::Expr(c) => {
                        let i = self.scalar_expr(*c)?;
                        let pos = self.reg(RegTy::I64);
                        let dope = self.offset(desc, 24 * u32::from(k));
                        self.emit(Inst::Pos {
                            dst: pos,
                            dope,
                            dim: 0,
                            idx: i,
                            what: m,
                        });
                        put(self, pos, a.value)?;
                    }
                    Choice::Range(rc) => {
                        let r = self.range(rc, span)?;
                        let d = self.dir_reg(r.dir);
                        let z = self.int(0);
                        let up = self.cmp(CmpOp::Eq, d, z);
                        let one = self.int(1);
                        let m1 = self.int(-1);
                        let step = self.select(up, one, m1, RegTy::I64);
                        let rd = self.alloc_n(24);
                        let parts = self.fb.regs(&[r.left, r.right, d]);
                        self.emit(Inst::MakeDope { at: rd, parts });
                        let cnt = self.dope_len(rd);
                        let left = r.left;
                        let dope = self.offset(desc, 24 * u32::from(k));
                        let e = a.value;
                        self.for_n(cnt, |fx, i| {
                            let o = fx.bin(IntOp::Mul, i, step);
                            let idx = fx.bin(IntOp::Add, left, o);
                            let pos = fx.reg(RegTy::I64);
                            fx.emit(Inst::Pos {
                                dst: pos,
                                dope,
                                dim: 0,
                                idx,
                                what: m,
                            });
                            put(fx, pos, e)
                        })?;
                    }
                    _ => return Err(self.cx.ice(span, "element choice in an array aggregate")),
                }
            }
        }
        Ok(())
    }

    /// Element `pos` (flat, from the left) of an aggregate's array.
    #[allow(clippy::too_many_arguments)]
    fn agg_elem(
        &mut self,
        tid: TyId,
        est: SubtypeId,
        data: Reg,
        desc: Reg,
        pos: Reg,
        e: ExprId,
        span: Span,
    ) -> R<()> {
        let db = self.db;
        let v = self.expr(e)?;
        if let Some(leaf) = Ty::logic_leaf(self.fb.program().types(), tid) {
            let (_, total) = self.layout(tid, desc);
            let (etid, ed) = self.elem_desc(tid, desc)?;
            match v {
                V::S(r) => {
                    self.check_scalar(r, Some(db.exprs[e].subtype), est, "", span)?;
                    let bias = if leaf == LogicKind::StdULogic { 0 } else { 2 };
                    let code = self.enc(r, bias);
                    let one = self.int(1);
                    let l = self.bin(IntOp::Sub, total, one);
                    let idx = self.bin(IntOp::Sub, l, pos);
                    self.emit(Inst::LSet {
                        base: data,
                        idx,
                        v: code,
                    });
                }
                V::A(vd, vp) => {
                    self.check_desc(db.base(est), etid, vp, ed, "aggregate element", span)?;
                    let (_, m) = self.layout(etid, ed);
                    let o = self.row_off(total, m, pos, None);
                    let z = self.int(0);
                    self.emit(Inst::LCopy {
                        dst: data,
                        dst_off: o,
                        src: vd,
                        src_off: z,
                        n: m,
                    });
                }
            }
            return Ok(());
        }
        let (etid, ed) = self.elem_desc(tid, desc)?;
        let a = self.elem_addr(tid, data, desc, pos);
        match v {
            V::S(r) => {
                self.check_scalar(r, Some(db.exprs[e].subtype), est, "", span)?;
            }
            V::A(_, vp) => {
                self.check_desc(db.base(est), etid, vp, ed, "aggregate element", span)?;
            }
        }
        self.store_at(etid, a, v, span)
    }

    fn record_agg(
        &mut self,
        t: TypeId,
        tid: TyId,
        nfields: usize,
        assocs: &[AggAssoc],
        st: SubtypeId,
        span: Span,
    ) -> R<V> {
        let db = self.db;
        // The value of each field, in field order.
        let mut vals: Vec<Option<ExprId>> = vec![None; nfields];
        let mut k = 0;
        for a in assocs.iter() {
            if a.choices.is_empty() {
                if k < nfields {
                    vals[k] = Some(a.value);
                }
                k += 1;
                continue;
            }
            for c in a.choices.iter() {
                match c {
                    Choice::Element(i) => vals[*i as usize] = Some(a.value),
                    Choice::Others => {
                        for v in vals.iter_mut().filter(|v| v.is_none()) {
                            *v = Some(a.value);
                        }
                    }
                    _ => return Err(self.cx.ice(span, "array choice in a record aggregate")),
                }
            }
        }
        let mut evs = Vec::new();
        for (n, v) in vals.iter().enumerate() {
            let Some(e) = v else {
                return Err(self.cx.ice(span, "record aggregate without a field value"));
            };
            let fv = self.expr(*e)?;
            let fst = self.field_subtype(t, n as u32)?;
            if let V::S(r) = fv {
                self.check_scalar(r, Some(db.exprs[*e].subtype), fst, "", span)?;
            }
            evs.push(fv);
        }
        let Some(Ty::Record { fields }) = self.fb.program().types().get(tid).cloned() else {
            return Err(self.cx.ice(span, "record aggregate of a non-record"));
        };
        // The descriptor: the context subtype's, else the field values' (rule 1).
        let ds = self.desc_size(tid);
        let desc = match self.desc_of(st, None, span)? {
            Some(d) => d,
            None => {
                let d = self.alloc_n(ds);
                let mut off = 0;
                for (f, v) in fields.iter().zip(&evs) {
                    if f.desc.is_some() || !Ty::needs_desc(self.fb.program().types(), f.ty) {
                        continue;
                    }
                    let n = self.desc_size(f.ty);
                    let V::A(_, vd) = v else {
                        return Err(self.cx.ice(span, "scalar value of a composite field"));
                    };
                    let at = self.offset(d, off);
                    let b = self.int(i64::from(n));
                    self.emit(Inst::Copy {
                        dst: at,
                        src: *vd,
                        bytes: b,
                    });
                    off += n;
                }
                d
            }
        };
        let size = self.size_of(tid, desc);
        let data = self.alloc(size);
        for (n, (f, v)) in fields.iter().zip(evs).enumerate() {
            let (a, fd) = self.field_addr(tid, data, desc, n as u32, span)?;
            if let V::A(_, vd) = v {
                let fst = self.field_subtype(t, n as u32)?;
                self.check_desc(db.base(fst), f.ty, vd, fd, "record aggregate element", span)?;
            }
            self.store_at(f.ty, a, v, span)?;
        }
        Ok(V::A(data, desc))
    }

    // ---- constants ----

    /// A folded record or array-of-composites constant: bytes in the pool, used in place.
    pub fn konst_comp(&mut self, v: &ConstValue, st: SubtypeId, span: Span) -> R<V> {
        let db = self.db;
        let t = db.base(st);
        let tid = self.ty(t, span)?;
        let mut desc = Vec::new();
        let Some(bytes) = self.encode(t, tid, v, &mut desc) else {
            return Err(self.cx.unsupported(span, "this composite constant"));
        };
        let d = self.bytes_addr(desc);
        let a = self.bytes_addr(bytes);
        Ok(V::A(a, d))
    }

    /// Value bytes of constant `v` of type `t`; its descriptor bytes are appended to `desc`.
    fn encode(
        &mut self,
        t: TypeId,
        tid: TyId,
        v: &ConstValue,
        desc: &mut Vec<u8>,
    ) -> Option<Vec<u8>> {
        let db = self.db;
        if let Some(w) = scalar_w(self.fb.program(), tid) {
            let c = ty::cell(v)?;
            return Some(c.to_le_bytes()[..w.bytes() as usize].to_vec());
        }
        match (self.fb.program().types().get(tid).cloned()?, v) {
            (Ty::Record { fields }, ConstValue::Record(vs)) => {
                let TypeKind::Record { elems } = &db.types[t].kind else {
                    return None;
                };
                let mut fbytes = Vec::new();
                let mut own = Vec::new();
                for (i, f) in fields.iter().enumerate() {
                    let mut fd = Vec::new();
                    let b = self.encode(db.base(elems[i].subtype), f.ty, vs.get(i)?, &mut fd)?;
                    if f.desc.is_none() {
                        own.extend(fd);
                    }
                    fbytes.push(b);
                }
                let (l, _) = self.fb.program().layout(tid, &own).ok()?;
                let mut out = vec![0u8; l.size as usize];
                for (b, off) in fbytes.iter().zip(&l.offsets) {
                    let off = *off as usize;
                    out.get_mut(off..off + b.len())?.copy_from_slice(b);
                }
                desc.extend(own);
                Some(out)
            }
            (
                Ty::Array {
                    elem, elem_desc, ..
                },
                ConstValue::Array(a),
            ) => {
                let mut own = Vec::new();
                for &(l, d, r) in a.bounds.iter() {
                    for x in [l, r, dir_cell(d)] {
                        own.extend_from_slice(&x.to_le_bytes());
                    }
                }
                let (est, _) = ty::array_of(db, t)?;
                let et = db.base(est);
                if let Some(leaf) = Ty::logic_leaf(self.fb.program().types(), tid) {
                    let mut codes = Vec::new();
                    let mut ed = Vec::new();
                    self.codes(et, elem, &a.elems, leaf, &mut codes, &mut ed)?;
                    if elem_desc.is_none() && Ty::needs_desc(self.fb.program().types(), elem) {
                        own.extend(ed);
                    }
                    desc.extend(own);
                    return Some(planes(&codes));
                }
                let mut out = Vec::new();
                let mut ed = Vec::new();
                let mut push =
                    |fx: &mut Self, n: u64, c: &ConstValue, ed: &mut Vec<u8>| -> Option<()> {
                        let mut d = Vec::new();
                        let b = fx.encode(et, elem, c, &mut d)?;
                        if ed.is_empty() {
                            *ed = d;
                        }
                        let (l, _) = fx.fb.program().layout(elem, ed).ok()?;
                        let mut b = b;
                        b.resize(l.size.next_multiple_of(l.align.max(1)) as usize, 0);
                        for _ in 0..n {
                            out.extend_from_slice(&b);
                        }
                        Some(())
                    };
                match &a.elems {
                    Elems::Bytes(b) => {
                        for &x in b.iter() {
                            push(self, 1, &ConstValue::Enum(u32::from(x)), &mut ed)?;
                        }
                    }
                    Elems::Runs(runs) => {
                        for (n, c) in runs.iter() {
                            push(self, *n, c, &mut ed)?;
                        }
                    }
                }
                if elem_desc.is_none() && Ty::needs_desc(self.fb.program().types(), elem) {
                    own.extend(ed);
                }
                desc.extend(own);
                Some(out)
            }
            _ => None,
        }
    }

    /// Leaf codes of a logic array constant, left first.
    fn codes(
        &mut self,
        t: TypeId,
        tid: TyId,
        e: &Elems,
        leaf: LogicKind,
        out: &mut Vec<u8>,
        ed: &mut Vec<u8>,
    ) -> Option<()> {
        let bias = if leaf == LogicKind::StdULogic { 0 } else { 2 };
        let one =
            |fx: &mut Self, c: &ConstValue, out: &mut Vec<u8>, ed: &mut Vec<u8>| -> Option<()> {
                match c {
                    ConstValue::Array(a) => {
                        if ed.is_empty() {
                            for &(l, d, r) in a.bounds.iter() {
                                for x in [l, r, dir_cell(d)] {
                                    ed.extend_from_slice(&x.to_le_bytes());
                                }
                            }
                        }
                        let (est, _) = ty::array_of(fx.db, t)?;
                        let inner = match fx.fb.program().types().get(tid)? {
                            Ty::Array { elem, .. } => *elem,
                            _ => return None,
                        };
                        let mut sub = Vec::new();
                        fx.codes(fx.db.base(est), inner, &a.elems, leaf, out, &mut sub)
                    }
                    c => {
                        out.push(u8::try_from(ty::cell(c)?).ok()? + bias);
                        Some(())
                    }
                }
            };
        match e {
            Elems::Bytes(b) => out.extend(b.iter().map(|&x| x + bias)),
            Elems::Runs(runs) => {
                for (n, c) in runs.iter() {
                    for _ in 0..*n {
                        one(self, c, out, ed)?;
                    }
                }
            }
        }
        Some(())
    }
}

/// Bit-planes of `codes` (left first): element 0 of the planes is the rightmost.
fn planes(codes: &[u8]) -> Vec<u8> {
    let n = codes.len();
    let mut blocks = vec![[0u64; 4]; n.div_ceil(64)];
    for (k, &c) in codes.iter().rev().enumerate() {
        for (p, plane) in blocks[k / 64].iter_mut().enumerate() {
            if c >> p & 1 == 1 {
                *plane |= 1 << (k % 64);
            }
        }
    }
    blocks
        .iter()
        .flat_map(|b| b.iter().flat_map(|w| w.to_le_bytes()))
        .collect()
}
