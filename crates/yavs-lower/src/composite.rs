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
    Ty, TyId, Width,
};
use yavs_sem::decl::DeclKind;
use yavs_sem::hir::{AggAssoc, AggBounds, Choice, ExprKind};
use yavs_sem::stat::{ConstValue, Elems};
use yavs_sem::types::{Constraint, Direction, RangeC, TypeKind};
use yavs_sem::{Db, DeclId, ExprId, SubtypeId, TypeId};

use crate::expr::{Dr, Place, V};
use crate::func::Fx;
use crate::{Cx, Failed, R, ty};

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
    // (Also `t'range` of a scalar subtype with folded bounds: an index named by a type mark.)
    let r = yavs_sem::expr::aggregate::range_value(db, rc)?;
    Some([r.left, r.right, dir_cell(r.dir)])
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

/// The width of an array element of IR type `tid` ([`ty::elem_width`]), `None` for composites.
pub(crate) fn elem_w(pb: &ProgramBuilder, tid: TyId) -> Option<Width> {
    pb.types()
        .get(tid)
        .filter(|t| t.scalar_size().is_some())
        .map(ty::elem_width)
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

    pub(crate) fn const_addr(&mut self, c: yavs_ir::ConstId) -> Reg {
        let dst = self.reg(RegTy::Ptr);
        self.emit(Inst::AddrOf {
            dst,
            space: AddrSpace::Const(c),
            off: 0,
        });
        dst
    }

    pub(crate) fn bytes_addr(&mut self, b: Vec<u8>) -> Reg {
        let c = self.fb.program().konst(ConstData::Bytes(b));
        self.const_addr(c)
    }

    // ---- descriptors ----

    /// The descriptor an array parameter of subtype `st` sees for an actual with descriptor
    /// `dope`: the subtype's own when it is constrained, else its dimensions from the actual
    /// and its constrained elements from the subtype (`ow(open)(7 downto 0)`). Every
    /// constrained level's length is checked against the actual's (LRM 5.3.2.2, 6.5.6.2).
    pub(crate) fn param_dope(
        &mut self,
        st: SubtypeId,
        dope: Reg,
        name: &str,
        span: Span,
    ) -> R<Reg> {
        let db = self.db;
        let partial = matches!(
            &db.subtypes[st].constraint,
            Constraint::Array { dims: None, elem } if !matches!(**elem, Constraint::None)
        );
        let want = match self.subtype_dope(st, span)? {
            Some(w) => w,
            None if partial => match self.desc_of(st, Some(dope), span)? {
                Some(w) => w,
                None => return Err(self.cx.ice(span, "partial formal without a descriptor")),
            },
            None => return Ok(dope),
        };
        let what = self.msg(name, yavs_ir::Dir::To);
        let mut t = db.base(st);
        let mut c = db.subtypes[st].constraint.clone();
        let mut tid = self.ty(t, span)?;
        let mut off = 0;
        while let Some(Ty::Array {
            elem,
            elem_desc,
            dims,
        }) = self.fb.program().types().get(tid).cloned()
        {
            if !matches!(c, Constraint::Array { dims: None, .. } | Constraint::None) {
                let (d, w) = (self.offset(dope, off), self.offset(want, off));
                self.emit(Inst::SubtypeCheck {
                    dope: d,
                    want: w,
                    dims,
                    what,
                });
            }
            if elem_desc.is_some() || !Ty::needs_desc(self.fb.program().types(), elem) {
                break;
            }
            off += 24 * u32::from(dims);
            tid = elem;
            let (est, ec) = elem_constraint(db, t, &c);
            (t, c) = (db.base(est), ec);
        }
        Ok(want)
    }

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
                        let indexes = crate::expr::index_subtypes(db, t);
                        for (k, rc) in ds.iter().enumerate() {
                            let r = self.range(rc, span)?;
                            // Element and field constraints too (#637).
                            self.check_index_range(indexes.as_deref(), k, rc, r, span)?;
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
            cache: None,
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
                let rt = self.rt_of(t);
                Ok(V::S(self.load(addr, w, rt)))
            }
            None => Ok(V::A(addr, desc)),
        }
    }

    /// [`Fx::value_at`] for an array element: a scalar has the element width.
    pub fn elem_value_at(&mut self, st: SubtypeId, addr: Reg, desc: Reg, span: Span) -> R<V> {
        let tid = self.ty(self.db.base(st), span)?;
        match elem_w(self.fb.program(), tid) {
            Some(w) => {
                let (_, rt, _) = self.scalar(st, span)?;
                Ok(V::S(self.load(addr, w, rt)))
            }
            None => self.value_at(st, addr, desc, span),
        }
    }

    /// [`Fx::store_at`] for an array element: a scalar has the element width.
    pub fn store_elem(&mut self, tid: TyId, addr: Reg, v: V, span: Span) -> R<()> {
        match (elem_w(self.fb.program(), tid), v) {
            (Some(w), V::S(r)) => {
                self.store(addr, r, w);
                Ok(())
            }
            _ => self.store_at(tid, addr, v, span),
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
        // Not a clone of the record type: this runs for every field access.
        let Some(Ty::Record { fields }) = self.fb.program().types().get(tid) else {
            return Err(self.cx.ice(span, "field of a non-record"));
        };
        Ok(match fields[n as usize].desc {
            Some(c) => self.const_addr(c),
            None => {
                let before: Vec<TyId> = fields[..n as usize]
                    .iter()
                    .filter(|g| g.desc.is_none())
                    .map(|g| g.ty)
                    .collect();
                let off = before.into_iter().map(|t| self.desc_size(t)).sum();
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
            let total = self.layout_count(tid, dope);
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
        self.elem_value_at(est, a, ed, span)
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
            let total = self.layout_count(tid, dope);
            let m = self.layout_count(etid, ed);
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
            let total = self.layout_count(tid, dope);
            let m = self.layout_count(etid, ed);
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
            let total = self.layout_count(tid, dope);
            let m = self.layout_count(etid, ed);
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
                    let n = self.layout_count(tid, a.1);
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
                let count = self.layout_count(tid, a.1);
                let ew = elem_w(self.fb.program(), etid);
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

    /// [`Fx::default_into`] for an array element: a narrow integer element (`Ty::elem_size`)
    /// is written with its 4 bytes.
    fn default_elem(
        &mut self,
        est: SubtypeId,
        c: &Constraint,
        addr: Reg,
        desc: Reg,
        span: Span,
    ) -> R<()> {
        let tid = self.ty(self.db.base(est), span)?;
        let ew = elem_w(self.fb.program(), tid);
        if let Some(w) = ew.filter(|&w| Some(w) != scalar_w(self.fb.program(), tid)) {
            let v = self.scalar_left(est, span)?;
            self.store(addr, v, w);
            return Ok(());
        }
        self.default_into(est, c, addr, desc, span)
    }

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
                TypeKind::Protected { .. } if self.prot_defaults => {
                    self.prot_new(t, false, span)?
                }
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
                    let count = self.layout_count(tid, desc);
                    let mut s = est;
                    while let Some((e, _)) = ty::array_of(db, db.base(s)) {
                        s = e;
                    }
                    let v = self.scalar_left(s, span)?;
                    let bias = if leaf == LogicKind::StdULogic { 0 } else { 2 };
                    let code = self.enc(v, bias);
                    let zero = self.int(0);
                    self.emit(Inst::LFill {
                        base: addr,
                        idx: zero,
                        n: count,
                        v: code,
                    });
                    return Ok(());
                }
                let count = self.layout_count(tid, desc);
                let _ = etid;
                if !(self.prot_defaults && holds_protected(db, db.base(est))) {
                    // Every element has the same default: write the first, then double.
                    let (some, first, done) = (self.block(), self.block(), self.block());
                    let zero = self.int(0);
                    let any = self.cmp(CmpOp::Gt, count, zero);
                    self.branch(any, first, done);
                    self.at(first);
                    let a0 = self.elem_addr(tid, addr, desc, zero);
                    self.default_elem(est, &ec, a0, ed, span)?;
                    let l = self.lay(tid, desc);
                    let sa = self.offset(l, 16);
                    let stride = self.load(sa, Width::B8, RegTy::I64);
                    let filled = self.reg(RegTy::I64);
                    let one = self.int(1);
                    self.emit(Inst::Move {
                        dst: filled,
                        src: one,
                    });
                    let head = self.block();
                    self.jump(head);
                    self.at(head);
                    let more = self.cmp(CmpOp::Lt, filled, count);
                    self.branch(more, some, done);
                    self.at(some);
                    // n = min(filled, count - filled) elements, from the start to `filled`.
                    let rest = self.bin(IntOp::Sub, count, filled);
                    let fits = self.cmp(CmpOp::Le, filled, rest);
                    let n = self.select(fits, filled, rest, RegTy::I64);
                    let off = self.bin(IntOp::Mul, filled, stride);
                    let to = self.reg(RegTy::Ptr);
                    self.emit(Inst::Index {
                        dst: to,
                        base: addr,
                        pos: off,
                        stride: 1,
                    });
                    let bytes = self.bin(IntOp::Mul, n, stride);
                    self.emit(Inst::Copy {
                        dst: to,
                        src: addr,
                        bytes,
                    });
                    let next = self.bin(IntOp::Add, filled, n);
                    self.emit(Inst::Move {
                        dst: filled,
                        src: next,
                    });
                    self.jump(head);
                    self.at(done);
                    return Ok(());
                }
                self.for_n(count, |fx, i| {
                    let a = fx.elem_addr(tid, addr, desc, i);
                    fx.default_elem(est, &ec, a, ed, span)
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
        let h = self.reg(RegTy::Handle);
        // The object is built in place by the allocation, not in a temporary copied from.
        let direct = match v {
            Some(V::A(d, vd)) if !self.logic(tid) && scalar_w(self.fb.program(), tid).is_none() => {
                Some((d, self.size_of(tid, vd), 0))
            }
            None if self.zero_default(st, tid, span)? => Some((self.bytes_addr(vec![0]), size, 1)),
            _ => None,
        };
        if let Some((data, n, fill)) = direct {
            self.emit(Inst::NewFill {
                dst: h,
                ty: tid,
                desc,
                dsize: ds,
                data,
                size: n,
                fill,
                shared,
            });
            return self.new_object_end(h, ds);
        }
        let dsr = self.int(i64::from(ds));
        let total = self.bin(IntOp::Add, size, dsr);
        let buf = self.alloc(total);
        self.emit(Inst::Copy {
            dst: buf,
            src: desc,
            bytes: dsr,
        });
        let vat = self.offset(buf, ds);
        self.temp_root(vat, tid, Some(buf));
        match v {
            Some(V::A(d, _)) if self.logic(tid) && scalar_w(self.fb.program(), tid).is_none() => {
                let count = self.layout_count(tid, desc);
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
        self.emit(Inst::New {
            dst: h,
            ty: tid,
            init: buf,
            size: total,
            shared,
        });
        self.new_object_end(h, ds)
    }

    /// The handle, data address and descriptor address of the heap object just allocated.
    pub(crate) fn new_object_end(&mut self, h: Reg, ds: u32) -> R<(Reg, Reg, Reg)> {
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

    /// The default value of an array of subtype `st` (IR type `tid`) is all zero bytes: scalar
    /// elements whose left bound has code 0 and whose planes need no bias.
    pub(crate) fn zero_default(&mut self, st: SubtypeId, tid: TyId, span: Span) -> R<bool> {
        let db = self.db;
        let Some((elem, _)) = ty::array_of(db, db.base(st)) else {
            return Ok(false);
        };
        let pt = db.base(elem);
        if !matches!(self.fb.program().types().get(tid), Some(Ty::Array { .. }))
            || matches!(db.types[pt].kind, TypeKind::Protected { .. })
        {
            return Ok(false);
        }
        let left0 = db
            .static_bounds(elem)
            .and_then(|(l, _, _)| ty::cell(&l))
            .is_some_and(|c| c == 0);
        let etid = self.ty(pt, span)?;
        Ok(left0 && self.code_bias(etid) == 0 && scalar_w(self.fb.program(), etid).is_some())
    }

    /// The slot of a variable or constant of a record type or an array of composites:
    /// `[data, desc, handle]` at `slot` over a heap object (rule 6), in the shared heap with
    /// `shared`.
    pub fn comp_init(&mut self, d: DeclId, slot: Reg, shared: bool) -> R<()> {
        let db = self.db;
        let decl = &db.decls[d];
        let DeclKind::Object(o) = &decl.kind else {
            return Err(self.cx.ice(decl.span, "not an object"));
        };
        let span = decl.span;
        let t = db.base(o.subtype);
        let tid = self.ty(t, span)?;
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
        let (h, data, p) = self.new_object(o.subtype, tid, desc, init, shared, span)?;
        self.store(slot, data, Width::B8);
        let ds = self.offset(slot, 8);
        self.store(ds, p, Width::B8);
        let hs = self.offset(slot, 16);
        self.store(hs, h, Width::B8);
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
            None => self.agg_desc(t, tid, st, assocs, bounds, dims, span)?,
        };
        if matches!(bounds, AggBounds::Context) {
            let len = self.dope_len(desc);
            self.agg_count_check(assocs, len);
        }
        let size = self.size_of(tid, desc);
        let data = self.alloc_val(size, tid, Some(desc));
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
        st: SubtypeId,
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
                // A context whose index ranges are given and whose elements are open
                // (`strings(0 to 2)`): the ranges come from the context subtype.
                let Constraint::Array { dims: Some(ds), .. } = &db.subtypes[st].constraint else {
                    return Err(self
                        .cx
                        .unsupported(span, "`others` in a row of an unconstrained aggregate"));
                };
                let r = self.range(&ds[k], span)?;
                let d = self.dir_reg(r.dir);
                (r.left, r.right, d)
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
        // Equal scalar elements of a one-dimensional aggregate are written by `Fill`.
        let fill = if last && base.is_none() {
            self.fill_width(tid)
        } else {
            None
        };
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
            if let Some(w) = fill.filter(|_| self.fill_value(e)) {
                let (go, done) = (self.block(), self.block());
                let zero = self.int(0);
                let any = self.cmp(CmpOp::Gt, len, zero);
                self.branch(any, go, done);
                self.at(go);
                let v = self.fill_element(est, e, span)?;
                self.fill_scalars(data, zero, len, v, w);
                self.jump(done);
                self.at(done);
                continue;
            }
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
                        let done = self.block();
                        if let Some(w) = fill.filter(|_| self.fill_value(e)) {
                            let right = r.right;
                            self.fill_run_or_fall(
                                data,
                                dope,
                                left,
                                right,
                                cnt,
                                w,
                                m,
                                done,
                                |fx| fx.fill_element(est, e, span),
                            )?;
                        }
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
                        self.jump(done);
                        self.at(done);
                    }
                    _ => return Err(self.cx.ice(span, "element choice in an array aggregate")),
                }
            }
        }
        Ok(())
    }

    /// Width of the elements of array type `tid` when they are plain scalars (not bit-planes).
    fn fill_width(&mut self, tid: TyId) -> Option<Width> {
        let p = self.fb.program();
        if Ty::logic_leaf(p.types(), tid).is_some() {
            return None;
        }
        match p.types().get(tid) {
            Some(Ty::Array { elem, .. }) => elem_w(p, *elem),
            _ => None,
        }
    }

    /// Does `e` give the same scalar every time, without side effects (so a fill may
    /// evaluate it once)?
    fn fill_value(&self, e: ExprId) -> bool {
        match &self.db.exprs[e].kind {
            ExprKind::Const(_) | ExprKind::Null | ExprKind::Obj(_) => true,
            ExprKind::ImplicitConv { operand } | ExprKind::Qualified { operand, .. } => {
                self.fill_value(*operand)
            }
            _ => false,
        }
    }

    /// The scalar element value of `e`, checked against the element subtype `est`.
    fn fill_element(&mut self, est: SubtypeId, e: ExprId, span: Span) -> R<Reg> {
        let V::S(r) = self.expr(e)? else {
            return Err(self.cx.ice(span, "aggregate element of the wrong shape"));
        };
        self.check_scalar(r, Some(self.db.exprs[e].subtype), est, "", span)?;
        Ok(r)
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
            let total = self.layout_count(tid, desc);
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
                    let m = self.layout_count(etid, ed);
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
        self.store_elem(etid, a, v, span)
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
        self.record_value(t, tid, evs, st, span)
    }

    /// A record of type `t` from the value of each field, in field order (the context
    /// subtype `st` gives the descriptor, else the field values do).
    pub(crate) fn record_value(
        &mut self,
        t: TypeId,
        tid: TyId,
        evs: Vec<V>,
        st: SubtypeId,
        span: Span,
    ) -> R<V> {
        let db = self.db;
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
        let data = self.alloc_val(size, tid, Some(desc));
        for (n, (f, v)) in fields.iter().zip(evs).enumerate() {
            let (a, fd) = self.field_addr(tid, data, desc, n as u32, span)?;
            if let V::A(_, vd) = v {
                let fst = self.field_subtype(t, n as u32)?;
                let what = match &db.types[t].kind {
                    TypeKind::Record { elems } => {
                        format!("field {}", elems[n].name.text.as_str().to_ascii_uppercase())
                    }
                    _ => "record aggregate element".to_string(),
                };
                self.check_desc(db.base(fst), f.ty, vd, fd, &what, span)?;
            }
            self.store_at(f.ty, a, v, span)?;
        }
        Ok(V::A(data, desc))
    }

    /// Conversion between record types with the same element names (2019 9.3.6, LCS-2016-075):
    /// each element is converted like a conversion to its type.
    pub(crate) fn conv_record(&mut self, to: SubtypeId, operand: ExprId, span: Span) -> R<V> {
        let from = self.db.exprs[operand].subtype;
        let v = self.expr(operand)?;
        self.conv_record_val(v, from, to, span)
    }

    fn conv_record_val(&mut self, v: V, from: SubtypeId, to: SubtypeId, span: Span) -> R<V> {
        let db = self.db;
        let (ft, tt) = (db.base(from), db.base(to));
        let (fid, tid) = (self.ty(ft, span)?, self.ty(tt, span)?);
        let (sdata, sdesc) = self.arr(v, span)?;
        let (TypeKind::Record { elems: fe }, TypeKind::Record { elems: te }) =
            (&db.types[ft].kind, &db.types[tt].kind)
        else {
            return Err(self.cx.ice(span, "record conversion of a non-record"));
        };
        let mut evs = Vec::with_capacity(te.len());
        for t in te.iter() {
            let Some(m) = fe.iter().position(|f| f.name.key == t.name.key) else {
                return Err(self
                    .cx
                    .ice(span, "record conversion without a matching element"));
            };
            let (a, d) = self.field_addr(fid, sdata, sdesc, m as u32, span)?;
            let fv = self.value_at(fe[m].subtype, a, d, span)?;
            evs.push(self.conv_value(fv, fe[m].subtype, t.subtype, span)?);
        }
        self.record_value(tt, tid, evs, to, span)
    }

    /// Value `v` of subtype `from` converted to subtype `to` (an element of a converted record).
    fn conv_value(&mut self, v: V, from: SubtypeId, to: SubtypeId, span: Span) -> R<V> {
        let db = self.db;
        let (ft, tt) = (db.base(from), db.base(to));
        if ft == tt {
            if let V::S(r) = v {
                self.check_scalar(r, Some(from), to, "", span)?;
            }
            return Ok(v);
        }
        match (&db.types[ft].kind, &db.types[tt].kind) {
            (TypeKind::Record { .. }, TypeKind::Record { .. }) => {
                self.conv_record_val(v, from, to, span)
            }
            _ => match v {
                V::S(r) => Ok(V::S(self.conv_scalar(r, from, to, span)?)),
                V::A(..) => Err(self.cx.unsupported(
                    span,
                    "a record conversion with an array element of another type",
                )),
            },
        }
    }

    // ---- constants ----

    /// A folded record or array-of-composites constant: bytes in the pool, used in place.
    pub fn konst_comp(&mut self, v: &ConstValue, st: SubtypeId, span: Span) -> R<V> {
        let db = self.db;
        let t = db.base(st);
        let tid = self.ty(t, span)?;
        let mut desc = Vec::new();
        let Some(runs) = self.encode(t, tid, v, &mut desc) else {
            return Err(self.cx.unsupported(span, "this composite constant"));
        };
        let d = self.bytes_addr(desc);
        let c = self.fb.program().konst(compact_runs(runs));
        let a = self.const_addr(c);
        Ok(V::A(a, d))
    }

    /// Value bytes of constant `v` of type `t` as `(count, bytes)` runs, so a periodic
    /// constant is never written out (#446); its descriptor bytes are appended to `desc`.
    fn encode(&mut self, t: TypeId, tid: TyId, v: &ConstValue, desc: &mut Vec<u8>) -> Option<Runs> {
        let db = self.db;
        if let Some(w) = scalar_w(self.fb.program(), tid) {
            let c = ty::cell(v)?;
            return Some(vec![(1, c.to_le_bytes()[..w.bytes() as usize].to_vec())]);
        }
        match (self.fb.program().types().get(tid).cloned()?, v) {
            (Ty::Record { fields }, ConstValue::Record(vs)) => {
                let TypeKind::Record { elems } = &db.types[t].kind else {
                    return None;
                };
                let mut fruns = Vec::new();
                let mut own = Vec::new();
                for (i, f) in fields.iter().enumerate() {
                    let mut fd = Vec::new();
                    let b = self.encode(db.base(elems[i].subtype), f.ty, vs.get(i)?, &mut fd)?;
                    if f.desc.is_none() {
                        own.extend(fd);
                    }
                    fruns.push(b);
                }
                let (l, _) = self.fb.program().layout(tid, &own).ok()?;
                // Fields at their offsets, zero padding between and after.
                let mut out = Runs::new();
                let mut at = 0u64;
                for (b, off) in fruns.into_iter().zip(&l.offsets) {
                    let off = *off;
                    if off < at {
                        return None;
                    }
                    push_run(&mut out, 1, vec![0; (off - at) as usize]);
                    at = off + runs_len(&b);
                    for (n, e) in b {
                        push_run(&mut out, n, e);
                    }
                }
                if at > l.size {
                    return None;
                }
                push_run(&mut out, 1, vec![0; (l.size - at) as usize]);
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
                    return Some(plane_runs(&codes));
                }
                let mut out = Runs::new();
                let mut ed = Vec::new();
                let mut push =
                    |fx: &mut Self, n: u64, c: &ConstValue, ed: &mut Vec<u8>| -> Option<()> {
                        let mut d = Vec::new();
                        let mut ep = fx.encode(et, elem, c, &mut d)?;
                        if ed.is_empty() {
                            *ed = d;
                        }
                        let (l, _) = fx.fb.program().layout(elem, ed).ok()?;
                        // A scalar element has the element size (`Ty::elem_size`).
                        let es = fx.fb.program().types().get(elem).and_then(Ty::elem_size);
                        let want = es.unwrap_or(l.size.next_multiple_of(l.align.max(1)));
                        let have = runs_len(&ep);
                        if want >= have {
                            push_run(&mut ep, 1, vec![0; (want - have) as usize]);
                        } else {
                            let mut b = flat(&ep);
                            b.truncate(want as usize);
                            ep = vec![(1, b)];
                        }
                        // An element of several large runs is repeated as its runs, not
                        // written out (a record holding a periodic array, #446).
                        if ep.len() > 1 && want > 256 && n.saturating_mul(ep.len() as u64) <= 4096 {
                            for _ in 0..n {
                                for (m, e) in &ep {
                                    push_run(&mut out, *m, e.clone());
                                }
                            }
                        } else {
                            push_run(&mut out, n, flat(&ep));
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
                        let (est, _) = ty::array_of(fx.db, t)?;
                        let (inner, elem_desc) = match fx.fb.program().types().get(tid)? {
                            Ty::Array {
                                elem, elem_desc, ..
                            } => (*elem, *elem_desc),
                            _ => return None,
                        };
                        let mut sub = Vec::new();
                        fx.codes(fx.db.base(est), inner, &a.elems, leaf, out, &mut sub)?;
                        // The element descriptor is this level's bounds, then the descriptor
                        // of the elements' own unconstrained elements (#329).
                        if ed.is_empty() {
                            for &(l, d, r) in a.bounds.iter() {
                                for x in [l, r, dir_cell(d)] {
                                    ed.extend_from_slice(&x.to_le_bytes());
                                }
                            }
                            if elem_desc.is_none() && Ty::needs_desc(fx.fb.program().types(), inner)
                            {
                                ed.extend(sub);
                            }
                        }
                        Some(())
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
                    if !matches!(c, ConstValue::Array(_)) {
                        // A run of one scalar: no per-element work.
                        let code = u8::try_from(ty::cell(c)?).ok()? + bias;
                        out.resize(out.len() + usize::try_from(*n).ok()?, code);
                        continue;
                    }
                    for _ in 0..*n {
                        one(self, c, out, ed)?;
                    }
                }
            }
        }
        Some(())
    }
}

/// Constant bytes as `(count, bytes)` runs.
type Runs = Vec<(u64, Vec<u8>)>;

/// Append a run, joining it to an equal last one; empty runs are dropped.
fn push_run(out: &mut Runs, n: u64, b: Vec<u8>) {
    if n == 0 || b.is_empty() {
        return;
    }
    match out.last_mut() {
        Some((m, last)) if *last == b => *m += n,
        _ => out.push((n, b)),
    }
}

fn runs_len(r: &Runs) -> u64 {
    r.iter().map(|(n, b)| n * b.len() as u64).sum()
}

fn flat(r: &Runs) -> Vec<u8> {
    let mut out = Vec::with_capacity(runs_len(r) as usize);
    for (n, b) in r {
        for _ in 0..*n {
            out.extend_from_slice(b);
        }
    }
    out
}

/// Pool data for `runs`: the runs themselves when they are at least four times smaller than
/// the bytes, else the bytes (#446).
fn compact_runs(mut runs: Runs) -> ConstData {
    let kept: u64 = runs.iter().map(|(_, b)| b.len() as u64 + 32).sum();
    if runs.is_empty() || runs_len(&runs) < 4 * kept {
        ConstData::Bytes(flat(&runs))
    } else if runs.len() == 1 {
        let (n, elem) = runs.remove(0);
        ConstData::Repeat { elem, n }
    } else {
        ConstData::Runs(runs)
    }
}

/// [`planes`] as runs: a constant of one value is one run of its 64-element word.
fn plane_runs(codes: &[u8]) -> Runs {
    let mut out = Runs::new();
    let whole = codes.len() / 64 * 64;
    if whole > 0 && codes.iter().all(|&x| x == codes[0]) {
        push_run(&mut out, (whole / 64) as u64, planes(&codes[..64]));
        push_run(&mut out, 1, planes(&codes[whole..]));
    } else {
        push_run(&mut out, 1, planes(codes));
    }
    out
}

/// Bit-planes of `codes` (left first): element 0 of the planes is the rightmost.
fn planes(codes: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(codes.len().div_ceil(64) * 32);
    let mut push = |w: [u64; 4]| {
        w.iter()
            .for_each(|x| out.extend_from_slice(&x.to_le_bytes()))
    };
    // A constant of one value (`(others => '0')`) is most of the large ones: whole words.
    if let Some(&c) = codes.first().filter(|&&c| codes.iter().all(|&x| x == c)) {
        let word = |p: u32, bits: usize| {
            if c >> p & 1 == 1 {
                u64::MAX >> (64 - bits)
            } else {
                0
            }
        };
        for _ in 0..codes.len() / 64 {
            push([0, 1, 2, 3].map(|p| word(p, 64)));
        }
        if !codes.len().is_multiple_of(64) {
            push([0, 1, 2, 3].map(|p| word(p, codes.len() % 64)));
        }
        return out;
    }
    for chunk in codes.rchunks(64) {
        let mut w = [0u64; 4];
        for (k, &c) in chunk.iter().rev().enumerate() {
            let c = u64::from(c);
            for (p, plane) in w.iter_mut().enumerate() {
                *plane |= (c >> p & 1) << k;
            }
        }
        push(w);
    }
    out
}

/// Does a value of type `t` hold a protected object (arrays and records are searched)?
fn holds_protected(db: &Db, t: TypeId) -> bool {
    match &db.types[t].kind {
        TypeKind::Protected { .. } => true,
        TypeKind::Record { elems } => elems
            .iter()
            .any(|e| holds_protected(db, db.base(e.subtype))),
        _ => ty::array_of(db, t).is_some_and(|(e, _)| holds_protected(db, db.base(e))),
    }
}

#[cfg(test)]
mod tests {
    use super::planes;

    /// The definition: element `k` from the right is bit `k % 64` of block `k / 64`.
    fn naive(codes: &[u8]) -> Vec<u8> {
        let mut blocks = vec![[0u64; 4]; codes.len().div_ceil(64)];
        for (k, &c) in codes.iter().rev().enumerate() {
            for (p, plane) in blocks[k / 64].iter_mut().enumerate() {
                *plane |= u64::from(c >> p & 1) << (k % 64);
            }
        }
        blocks
            .iter()
            .flat_map(|b| b.iter().flat_map(|w| w.to_le_bytes()))
            .collect()
    }

    #[test]
    fn planes_match_the_definition() {
        for n in [0, 1, 63, 64, 65, 200, 1024] {
            for c in [0u8, 3, 9, 15] {
                let same = vec![c; n];
                assert_eq!(planes(&same), naive(&same), "{n} x {c}");
            }
            let mixed: Vec<u8> = (0..n).map(|i| (i * 7 % 10) as u8).collect();
            assert_eq!(planes(&mixed), naive(&mixed), "{n} mixed");
        }
    }
}
