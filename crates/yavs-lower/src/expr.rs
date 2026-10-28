//! Expressions (DESIGN.md "Expressions"): values, ranges and descriptors, calls, implicit
//! operations, attributes, names and conversions.

use yavs_diag::Span;
use yavs_ir::{
    AttrKind, BoolOp, CmpOp, ConstData, Dir, FOp, FUnOp, IntOp, IntUnOp, LOp, PhysOp, Reg, RegTy,
    Ty, TyId, Width,
};
use yavs_ir::{CheckKind, Inst};
use yavs_sem::decl::{Alias, Body, DeclKind, Param, Subprogram};
use yavs_sem::hir::UnitHir;
use yavs_sem::hir::{Actual, AggAssoc, AggBounds, Arg, AttrPrefix, Choice, ExprKind, PredefAttr};
use yavs_sem::implicit::{Family, Form, ImplicitOp};
use yavs_sem::stat::{ConstValue, Elems};
use yavs_sem::types::{Constraint, Direction, RangeC, TypeKind};
use yavs_sem::{Db, DeclId, ExprId, SubtypeId, TypeId};

use crate::func::{Fx, Shape, func_of, shape};
use crate::{Failed, Obj, R, Space, ty};

/// How the array arguments of a call are passed.
#[derive(Clone, Copy)]
pub(crate) enum Conv {
    /// A subprogram body: data and descriptor.
    Body,
    /// A native: the address of a fat pair `[data, descriptor]`.
    Fat,
    /// A native `ieee` subprogram: `data, left, right, dir` (`yavs_ir::ieee::ARRAY_REGS`).
    Flat,
}

/// The most elements of an aggregate choice range written out one by one (more: a loop).
const AGG_UNROLL: i64 = 4;

/// A value: a scalar cell, or an array as `(data address, descriptor address)`.
#[derive(Clone, Copy, Debug)]
pub(crate) enum V {
    S(Reg),
    A(Reg, Reg),
}

/// Direction of a range: known when the range is written, else a register (0 `to`, 1 `downto`).
#[derive(Clone, Copy, Debug)]
pub(crate) enum Dr {
    Static(Direction),
    Dyn(Reg),
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct Rng {
    pub left: Reg,
    pub right: Reg,
    pub dir: Dr,
}

/// [`Fx::leaf_facts`]: logic flag, plane code bias, scalar width and element (kernel) width.
pub(crate) type LeafFacts = (bool, i64, Width, Width);

/// An assignment target.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Place {
    Scalar {
        addr: Reg,
        w: Width,
        st: SubtypeId,
    },
    /// Element `idx` (0 = rightmost) of a logic array.
    Logic {
        data: Reg,
        idx: Reg,
        st: SubtypeId,
    },
    /// An array; `off` is the element offset (0 = rightmost) of a logic slice in `data`.
    Array {
        data: Reg,
        dope: Reg,
        off: Option<Reg>,
        tid: TyId,
        dims: u8,
    },
}

/// One selection below a composite signal.
#[derive(Clone, Copy)]
pub(crate) enum Step<'d> {
    Field(u32),
    Index(&'d [ExprId]),
    Slice(&'d RangeC),
    /// The steps that follow index in the index space of this alias subtype (LRM 6.6.2).
    Rebase(SubtypeId),
}

/// Where a chain of selections leads (`Fx::walk_steps`): the part's type, descriptor and
/// field path, and the `count` consecutive instances from instance `offset` it covers, each
/// of type `inst` (the element type after a slice) or the part's own.
pub(crate) struct Walk {
    t: TypeId,
    desc: Reg,
    path: Vec<u32>,
    offset: Reg,
    count: Reg,
    inst: Option<(TypeId, Reg)>,
    /// The instances of the arrays an index or slice passed, `offset`'s units (all of them).
    outer: Reg,
    /// An index or slice was passed.
    selected: bool,
}

/// The part of a composite signal that a name selects (`Fx::comp_select`).
pub(crate) struct SigPart {
    pub t: TypeId,
    pub tid: TyId,
    pub desc: Reg,
    /// Its field path from the signal.
    pub path: Vec<u32>,
    /// Each leaf below it, a reference to its selected elements and their count.
    pub leaves: Vec<(crate::RecLeaf, Reg, Reg)>,
    /// For `SigGather`/`SigScatter`: the first element and the number of elements of the
    /// outermost array of the part, and of the signal it is a part of (`None`: the whole signal).
    pub part: Option<yavs_ir::Part>,
}

fn dir_cell(d: Direction) -> i64 {
    match d {
        Direction::To => 0,
        Direction::Downto => 1,
    }
}

fn ir_dir(d: Dr) -> Dir {
    match d {
        Dr::Static(Direction::Downto) => Dir::Downto,
        _ => Dir::To,
    }
}

impl<'a> Fx<'a, '_> {
    pub fn arr(&mut self, v: V, span: Span) -> R<(Reg, Reg)> {
        match v {
            V::A(d, p) => Ok((d, p)),
            V::S(_) => Err(self.cx.ice(span, "scalar where an array was expected")),
        }
    }

    pub fn scal(&mut self, v: V, span: Span) -> R<Reg> {
        match v {
            V::S(r) => Ok(r),
            V::A(..) => Err(self.cx.ice(span, "array where a scalar was expected")),
        }
    }

    pub fn scalar_expr(&mut self, e: ExprId) -> R<Reg> {
        let v = self.expr(e)?;
        self.scal(v, self.db.exprs[e].span)
    }

    /// What a BIT or BOOLEAN position adds to become its plane code (the std_ulogic codes of
    /// '0' and '1', docs/decisions.md "Interpreter: BIT/BOOLEAN codes"); 0 for STD_ULOGIC.
    pub fn code_bias(&mut self, tid: TyId) -> i64 {
        match Ty::logic_leaf(self.fb.program().types(), tid) {
            Some(yavs_ir::LogicKind::Bit | yavs_ir::LogicKind::Boolean) => 2,
            _ => 0,
        }
    }

    /// Position to plane code.
    pub fn enc(&mut self, v: Reg, bias: i64) -> Reg {
        if bias == 0 {
            return v;
        }
        let b = self.int(bias);
        self.bin(IntOp::Add, v, b)
    }

    /// Plane code to position.
    pub fn dec(&mut self, v: Reg, bias: i64) -> Reg {
        if bias == 0 {
            return v;
        }
        let b = self.int(bias);
        self.bin(IntOp::Sub, v, b)
    }

    pub(crate) fn is_logic(&mut self, tid: TyId) -> bool {
        Ty::logic_leaf(self.fb.program().types(), tid).is_some()
    }

    /// The register type of a scalar of base type `t`: access and protected values are
    /// handles, GC roots while live (rule 6).
    pub(crate) fn rt_of(&self, t: TypeId) -> RegTy {
        if ty::is_real(self.db, t) {
            RegTy::F64
        } else if matches!(
            self.db.types[t].kind,
            TypeKind::Access { .. } | TypeKind::Protected { .. }
        ) {
            RegTy::Handle
        } else {
            RegTy::I64
        }
    }

    pub fn expr(&mut self, id: ExprId) -> R<V> {
        let v = self.expr_value(id)?;
        if !self.cov_inputs.is_empty() {
            self.cov_input(id, v);
        }
        Ok(v)
    }

    fn expr_value(&mut self, id: ExprId) -> R<V> {
        let db = self.db;
        let e = &db.exprs[id];
        let span = e.span;
        if let Some(c) = e.value {
            let v = self.konst_checked(&db.consts[c], e.subtype, span)?;
            // A folded constant keeps the bounds of its subtype, element bounds too, not those
            // of its initialiser's literals (LRM 6.4.2.2): `'element'left` of it, or of a
            // parameter it is passed to, reads them.
            if let (ExprKind::Obj(_), V::A(data, _)) = (&e.kind, v)
                && let Some(want) = self.subtype_dope(e.subtype, span)?
            {
                return Ok(V::A(data, want));
            }
            return Ok(v);
        }
        match &e.kind {
            ExprKind::Const(c) => self.konst_checked(&db.consts[*c], e.subtype, span),
            ExprKind::Obj(d) => self.read_obj(*d, span),
            ExprKind::Call {
                callee,
                args,
                method_of,
                ..
            } => {
                if method_of.is_none()
                    && let Some(r) = self.edge_test(id)?
                {
                    return Ok(V::S(r));
                }
                match self.call(*callee, args, *method_of, Some(e.subtype), span)? {
                    Some(v) => Ok(v),
                    None => Err(self.cx.ice(span, "procedure call in an expression")),
                }
            }
            ExprKind::Index { prefix, indices } => match self.rec_part(id, span)? {
                Some(v) => Ok(v),
                None => match self.sig_elem(id, *prefix, span)? {
                    Some(v) => Ok(v),
                    None => match self.sig_part(id, span)? {
                        Some(v) => Ok(v),
                        None => self.index(*prefix, indices, span),
                    },
                },
            },
            ExprKind::Slice { prefix, range } => match self.sig_part(id, span)? {
                Some(v) => Ok(v),
                None => match self.sig_slice(id, *prefix, span)? {
                    Some(v) => Ok(v),
                    None => self.slice(*prefix, range, span),
                },
            },
            ExprKind::Attr { attr, prefix, args } => {
                self.attr(*attr, *prefix, args, e.subtype, span)
            }
            ExprKind::Qualified { to, operand } => self.qualified(*to, *operand, span),
            ExprKind::Conv { to, operand } => self.conv(*to, *operand, span),
            ExprKind::ImplicitConv { operand } => {
                let from = db.exprs[*operand].subtype;
                let v = self.expr(*operand)?;
                let V::S(r) = v else { return Ok(v) };
                let to = e.subtype;
                if ty::is_real(db, db.base(to)) && !ty::is_real(db, db.base(from)) {
                    let dst = self.reg(RegTy::F64);
                    self.emit(Inst::IToF { dst, src: r });
                    return Ok(V::S(dst));
                }
                self.check_scalar(r, Some(from), to, "", span)?;
                Ok(V::S(r))
            }
            ExprKind::Conditional { alts } => {
                let alts = alts.clone();
                self.conditional(&alts, e.subtype, span)
            }
            ExprKind::Aggregate { assocs, bounds } => {
                self.aggregate(assocs, bounds, e.subtype, span)
            }
            ExprKind::Field { prefix, elem } => match self.rec_part(id, span)? {
                Some(v) => Ok(v),
                None => self.field(*prefix, *elem, span),
            },
            ExprKind::Deref { prefix } => self.deref(*prefix, span),
            ExprKind::PrivateVar { object, var } => self.private_var(*object, *var, span),
            ExprKind::New { init } => self.allocator(init, e.subtype, span),
            ExprKind::Null => Ok(V::S(self.int(0))),
            // Every external name elab binds is used through an alias; one left is in a package
            // declaration and names the hierarchy (elab rejects those) or something not
            // supported yet.
            ExprKind::External(_) => Err(self
                .cx
                .unsupported(span, "this use of an external name (#255)")),
            // The value of a user-defined attribute is its specification's expression.
            ExprKind::AttrValue { spec } => {
                let v = db.attr_values[*spec].value;
                let val = self.expr(v)?;
                // The value takes the attribute's subtype (implicit subtype conversion): a
                // string literal given to `string (3 downto 1)` has those bounds (#638).
                if let V::A(data, vd) = val
                    && let Some(want) = self.desc_of(e.subtype, Some(vd), span)?
                {
                    let t = db.base(e.subtype);
                    let tid = self.ty(t, span)?;
                    self.check_desc(t, tid, vd, want, "attribute value", span)?;
                    return Ok(V::A(data, want));
                }
                Ok(val)
            }
            ExprKind::Open | ExprKind::Error => {
                Err(self.cx.ice(span, "open or poisoned expression"))
            }
        }
    }

    // ---- constants ----

    /// `konst`, and an integer literal typed by a subtype whose range it lies outside of (a
    /// universal literal converted to `INTEGER`, LRM 9.3.6) fails when evaluated.
    fn konst_checked(&mut self, v: &ConstValue, st: SubtypeId, span: Span) -> R<V> {
        let r = self.konst(v, st, span)?;
        if let (ConstValue::Int(i), V::S(reg), Some((l, d, h))) = (v, r, self.db.static_bounds(st))
            && let (Some(l), Some(h)) = (l.as_int(), h.as_int())
        {
            let (lo, hi) = if d == Direction::To { (l, h) } else { (h, l) };
            if lo <= hi && (*i < lo || *i > hi) {
                self.check_scalar(reg, None, st, "", span)?;
            }
        }
        Ok(r)
    }

    pub fn konst(&mut self, v: &ConstValue, st: SubtypeId, span: Span) -> R<V> {
        if let Some(c) = ty::cell(v) {
            let dst = self.reg(self.rt_of(self.db.base(st)));
            match v {
                ConstValue::Real(f) => self.emit(Inst::ConstF { dst, v: *f }),
                _ => self.emit(Inst::Const { dst, v: c as i64 }),
            }
            return Ok(V::S(dst));
        }
        let ConstValue::Array(a) = v else {
            return match v {
                ConstValue::Null => Ok(V::S(self.int(0))),
                _ => self.konst_comp(v, st, span),
            };
        };
        let t = self.db.base(st);
        if self.is_general(t, span)? {
            return self.konst_comp(v, st, span);
        }
        let tid = self.ty(t, span)?;
        let Some((elem, _)) = ty::array_of(self.db, t) else {
            return Err(self.cx.ice(span, "array constant of a non-array type"));
        };
        let etid = self.ty(self.db.base(elem), span)?;
        let logic = self.is_logic(tid);
        let ew = self
            .fb
            .program()
            .types()
            .get(etid)
            .map_or(8, |t| t.elem_size().unwrap_or(8));
        let mut dope = Vec::new();
        for &(l, d, r) in a.bounds.iter() {
            for x in [l, r, dir_cell(d)] {
                dope.extend_from_slice(&x.to_le_bytes());
            }
        }
        let dc = self.fb.program().konst(ConstData::Bytes(dope));
        let dope = self.reg(RegTy::Ptr);
        self.emit(Inst::AddrOf {
            dst: dope,
            space: yavs_ir::AddrSpace::Const(dc),
            off: 0,
        });
        let enc = |c: &ConstValue| -> Option<Vec<u8>> {
            let x = ty::cell(c)?;
            Some(x.to_le_bytes()[..ew as usize].to_vec())
        };
        let data = match (&a.elems, logic) {
            (Elems::Bytes(b), false) if ew == 1 => {
                let c = self.fb.program().konst(ConstData::Bytes(b.to_vec()));
                let dst = self.reg(RegTy::Ptr);
                self.emit(Inst::AddrOf {
                    dst,
                    space: yavs_ir::AddrSpace::Const(c),
                    off: 0,
                });
                return Ok(V::A(dst, dope));
            }
            (Elems::Bytes(b), true) => {
                let bias = self.code_bias(tid) as u8;
                ConstData::Logic(b.iter().rev().map(|&x| (1, x + bias)).collect())
            }
            (Elems::Runs(runs), true) => {
                let mut out = Vec::new();
                for (n, c) in runs.iter().rev() {
                    let Some(x) = ty::cell(c) else {
                        return Err(self.cx.unsupported(span, "nested array constants"));
                    };
                    out.push((*n, x as u8 + self.code_bias(tid) as u8));
                }
                ConstData::Logic(out)
            }
            (Elems::Runs(runs), false) => {
                let mut out = Vec::new();
                for (n, c) in runs.iter() {
                    let Some(b) = enc(c) else {
                        return Err(self.cx.unsupported(span, "nested array constants"));
                    };
                    out.push((*n, b));
                }
                ConstData::Runs(out)
            }
            (Elems::Bytes(b), false) => ConstData::Bytes(
                b.iter()
                    .flat_map(|&x| u64::from(x).to_le_bytes()[..ew as usize].to_vec())
                    .collect(),
            ),
        };
        // Read in place, like the string constants above: the runtime expands a compact
        // constant once; no consumer writes into the value of a literal.
        let c = self.fb.program().konst(data);
        let dst = self.reg(RegTy::Ptr);
        self.emit(Inst::AddrOf {
            dst,
            space: yavs_ir::AddrSpace::Const(c),
            off: 0,
        });
        Ok(V::A(dst, dope))
    }

    // ---- objects ----

    /// The value of object `d` as one cell: a scalar, or the address of a composite's fat
    /// pair `[data, desc]` (a thunk's result).
    pub(crate) fn obj_cell(&mut self, d: DeclId, span: Span) -> R<Reg> {
        match self.read_obj(d, span)? {
            V::S(r) => Ok(r),
            V::A(data, desc) => {
                let fat = self.alloc_n(16);
                self.store(fat, data, Width::B8);
                let dp = self.offset(fat, 8);
                self.store(dp, desc, Width::B8);
                self.fat_root(fat);
                Ok(fat)
            }
        }
    }

    fn read_obj(&mut self, d: DeclId, span: Span) -> R<V> {
        let db = self.db;
        let decl = &db.decls[d];
        // A deferred constant of a package instance has a slot of its own that nothing fills:
        // its value is the full declaration's (LRM 4.8).
        if let DeclKind::Object(o) = &decl.kind
            && o.role == yavs_sem::decl::ObjRole::Deferred
            && o.class == yavs_sem::decl::ObjClass::Constant
            && let Some(c) = self.deferred_completion(d)
        {
            return self.read_obj(c, span);
        }
        let obj = self.cx.objs.get(&d).copied();
        if self.cx.capture_free
            && obj.is_some()
            && let Some(i) = self.cx.free_objs.iter().position(|o| o.decl == d)
        {
            self.cx.free_readers.push((i, self.fb.id()));
        }
        match obj {
            Some(Obj::RecSignal(_)) => {
                // ponytail: any name below a composite signal reads all of it, then selects;
                // read only the selected part (`comp_select`) if large memories need it.
                if let Some((data, desc)) = self.sigcopy_of(d, span)? {
                    return Ok(V::A(data, desc));
                }
                if let Some(tab) = self.rec_table(d, span)? {
                    let p = self.comp_head(d, &[], span)?;
                    let size = self.size_of(p.tid, p.desc);
                    let val = self.alloc(size);
                    self.emit(Inst::SigGather {
                        dst: val,
                        desc: p.desc,
                        tab,
                        part: None,
                    });
                    return Ok(V::A(val, p.desc));
                }
                let p = self.comp_select(d, &[], span)?;
                self.read_comp(&p, span)
            }
            Some(Obj::Scalar { space, off, w }) => {
                let rt = self.obj_rt(d);
                let a = self.addr(space, off);
                Ok(V::S(self.load(a, w, rt)))
            }
            Some(Obj::ScalarRef { space, off, w }) => {
                let rt = self.obj_rt(d);
                let a = self.addr(space, off);
                let p = self.load(a, Width::B8, RegTy::Ptr);
                Ok(V::S(self.load(p, w, rt)))
            }
            Some(Obj::Array { space, off }) => {
                let a = self.addr(space, off);
                let data = self.load(a, Width::B8, RegTy::Ptr);
                let dope = self.array_dope(d, a);
                Ok(V::A(data, dope))
            }
            Some(Obj::InstArray { space, off }) => {
                let (data, dope) = self.inst_array(Some(d), space, off);
                Ok(V::A(data, dope))
            }
            Some(Obj::Fixed { space, off, size }) => {
                let data = self.addr(space, off);
                let dope = self.addr(space, off + size);
                Ok(V::A(data, dope))
            }
            Some(Obj::Signal { .. } | Obj::SignalRef { .. })
                if self.sig_dope(d, span)?.is_some() =>
            {
                let dope = self.sig_dope(d, span)?.ok_or(Failed)?;
                let s = self.sig_of(d, span)?;
                let tid = self.decl_ty(d, span)?;
                let size = self.size_of(tid, dope);
                let buf = self.alloc(size);
                self.emit(Inst::SigRead { dst: buf, sig: s });
                Ok(V::A(buf, dope))
            }
            Some(Obj::Signal { .. } | Obj::SignalRef { .. }) => {
                let s = self.sig_of(d, span)?;
                let raw = self.reg(RegTy::I64);
                self.emit(Inst::SigReadScalar { dst: raw, sig: s });
                let bias = match ty::obj_subtype(db, d) {
                    Some(st) => {
                        let tid = self.ty(db.base(st), span)?;
                        self.code_bias(tid)
                    }
                    None => 0,
                };
                let dst = self.dec(raw, bias);
                if self.obj_rt(d) == RegTy::F64 {
                    let f = self.reg(RegTy::F64);
                    self.emit(Inst::Move { dst: f, src: dst });
                    return Ok(V::S(f));
                }
                Ok(V::S(dst))
            }
            None => match &decl.kind {
                // An object alias without storage of its own (LRM 6.6.2): the aliased name,
                // seen with the alias subtype's bounds.
                DeclKind::Alias(Alias::Object { target, subtype }) => match self.expr(*target)? {
                    V::A(data, dope) => {
                        let what = decl.name.to_string();
                        Ok(V::A(data, self.dope_as(dope, *subtype, &what, span)?))
                    }
                    v => Ok(v),
                },
                DeclKind::EnumLit { pos, .. } => {
                    let dst = self.int(i64::from(*pos));
                    Ok(V::S(dst))
                }
                // A deferred constant (LRM 4.8): the package body's full declaration.
                DeclKind::Object(o)
                    if o.class == yavs_sem::decl::ObjClass::Constant
                        && o.default.is_none()
                        && self.deferred_completion(d).is_some() =>
                {
                    let c = self.deferred_completion(d).ok_or(Failed)?;
                    self.read_obj(c, span)
                }
                // A constant or generic of an enclosing region, read by a function called
                // during elaboration: a slot of instance data the elaborator fills.
                DeclKind::Object(o)
                    if self.cx.capture_free
                        && o.class == yavs_sem::decl::ObjClass::Constant
                        && (o.role == yavs_sem::decl::ObjRole::Generic
                            || o.default.is_none_or(|e| db.exprs[e].value.is_none()))
                        && matches!(
                            db.scopes[decl.scope].kind,
                            yavs_sem::scope::ScopeKind::Entity
                                | yavs_sem::scope::ScopeKind::Architecture
                                | yavs_sem::scope::ScopeKind::Block
                                | yavs_sem::scope::ScopeKind::Generate
                        ) =>
                {
                    self.capture_free(d, o.subtype, span)?;
                    self.read_obj(d, span)
                }
                // A shared variable of an enclosing region: a slot holding the handle of its
                // object (protected) or the address of its slot (`--relaxed`), created and
                // filled in by the elaborator.
                DeclKind::Object(o)
                    if self.cx.capture_free
                        && self.cx.capture_vars
                        && o.class == yavs_sem::decl::ObjClass::SharedVariable
                        && matches!(
                            db.scopes[decl.scope].kind,
                            yavs_sem::scope::ScopeKind::Entity
                                | yavs_sem::scope::ScopeKind::Architecture
                                | yavs_sem::scope::ScopeKind::Block
                                | yavs_sem::scope::ScopeKind::Generate
                        ) =>
                {
                    let off = self.cx.inst_alloc(8);
                    let (obj, t) = if matches!(
                        db.types[db.base(o.subtype)].kind,
                        yavs_sem::types::TypeKind::Protected { .. }
                    ) {
                        let t = self.ty(db.base(o.subtype), span)?;
                        let obj = Obj::Scalar {
                            space: Space::Inst,
                            off,
                            w: Width::B8,
                        };
                        (obj, t)
                    } else {
                        let (shape, t) = self.shared_shape(d, db.base(o.subtype), span)?;
                        (shape.obj(Space::Region(off), 0), t)
                    };
                    self.cx.objs.insert(d, obj);
                    self.cx.free_objs.push(crate::FreeObj {
                        decl: d,
                        ty: t,
                        off,
                        var: true,
                    });
                    self.read_obj(d, span)
                }
                // A package constant whose initialiser is not folded, read by a function lowered
                // alone for an elaboration-time call: evaluating the initialiser at every read
                // rebuilt a whole table per call. The call runs against the package state instead.
                DeclKind::Object(o)
                    if self.cx.capture_free
                        && !self.cx.capture_vars
                        && o.class == yavs_sem::decl::ObjClass::Constant
                        && o.default.is_some_and(|e| db.exprs[e].value.is_none())
                        && matches!(
                            db.scopes[decl.scope].kind,
                            yavs_sem::scope::ScopeKind::Package
                                | yavs_sem::scope::ScopeKind::PackageBody
                        ) =>
                {
                    Err(self.cx.unsupported(
                        span,
                        &format!("package constant `{}` without a folded value", decl.name),
                    ))
                }
                // A constant without run-time data (locally static) is evaluated from its
                // folded initialiser.
                DeclKind::Object(o)
                    if o.class == yavs_sem::decl::ObjClass::Constant && o.default.is_some() =>
                {
                    let e = o.default.ok_or(Failed)?;
                    let v = match self.expr(e)? {
                        // The constant's own bounds, not the initialiser's (LRM 6.4.2.2).
                        V::A(data, dope) => V::A(
                            data,
                            self.dope_as(dope, Some(o.subtype), &decl.name.to_string(), span)?,
                        ),
                        v => v,
                    };
                    if let V::S(r) = v {
                        self.check_scalar(
                            r,
                            Some(db.exprs[e].subtype),
                            o.subtype,
                            &decl.name.to_string(),
                            span,
                        )?;
                    }
                    Ok(v)
                }
                _ => Err(self.cx.unsupported(
                    span,
                    &format!("object `{}` declared outside the lowered units", decl.name),
                )),
            },
        }
    }

    /// Give constant `d` of subtype `st` a slot of instance data (`Cx::capture_free`).
    fn capture_free(&mut self, d: DeclId, st: SubtypeId, span: Span) -> R<()> {
        let t = self.ty(self.db.base(st), span)?;
        let w = match self.fb.program().types().get(t) {
            Some(x) if x.scalar_size().is_some() => Some(ty::width(x)),
            _ => None,
        };
        let off = self.cx.inst_alloc(if w.is_some() { 8 } else { 16 });
        let obj = match w {
            Some(w) => Obj::Scalar {
                space: Space::Inst,
                off,
                w,
            },
            None => Obj::InstArray {
                space: Space::Inst,
                off,
            },
        };
        self.cx.objs.insert(d, obj);
        self.cx.free_objs.push(crate::FreeObj {
            decl: d,
            ty: t,
            off,
            var: false,
        });
        Ok(())
    }

    /// The full declaration in the package body of deferred constant `d`.
    pub fn deferred_completion(&self, d: DeclId) -> Option<DeclId> {
        deferred_completion(self.db, d)
    }

    fn obj_rt(&self, d: DeclId) -> RegTy {
        match ty::obj_subtype(self.db, d) {
            Some(st) => self.rt_of(self.db.base(st)),
            None => RegTy::I64,
        }
    }

    /// Elements and descriptor of an array in instance data (`Obj::InstArray`).
    pub fn inst_array(&mut self, d: Option<DeclId>, space: Space, off: u32) -> (Reg, Reg) {
        let base = self.addr(space, 0);
        let at = |fx: &mut Self, k: u32| {
            let cell = fx.addr(space, off + k);
            let o = fx.load(cell, Width::B8, RegTy::I64);
            let dst = fx.reg(RegTy::Ptr);
            fx.emit(Inst::Index {
                dst,
                base,
                pos: o,
                stride: 1,
            });
            dst
        };
        let data = at(self, 0);
        // (A static subtype has its descriptor in the pool, `Cx::const_dope`.)
        match d.and_then(|d| self.cx.const_dope.get(&d).copied()) {
            Some(c) => (data, self.const_addr(c)),
            None => (data, at(self, 8)),
        }
    }

    /// A reference to all of the scalar signal or signal parameter `d`.
    pub fn sig_of(&mut self, d: DeclId, span: Span) -> R<Reg> {
        match self.cx.objs.get(&d).copied() {
            Some(Obj::Signal { sig }) => match self.sig_dope(d, span)? {
                Some(dope) => {
                    // Every scalar: an array of arrays is one leaf of all of them, row major.
                    let t = self.db.base(ty::obj_subtype(self.db, d).ok_or(Failed)?);
                    let count = self.leaf_count(t, dope, &[], span)?;
                    let offset = self.int(0);
                    Ok(self.sigval_at(sig, offset, count))
                }
                None => Ok(self.sigval(sig)),
            },
            Some(Obj::SignalRef { space, off, .. }) => {
                let a = self.addr(space, off);
                Ok(self.load(a, Width::Sig, RegTy::Sig))
            }
            None if self.alias_target(d).is_some() => {
                let (t, _) = self.alias_target(d).ok_or(Failed)?;
                self.sig_expr(t)
            }
            _ => Err(self.cx.unsupported(span, "this signal (ports, M2 slice 3)")),
        }
    }

    /// The aliased name and alias subtype of object alias `d` when lowering gave it no
    /// storage of its own (an alias of a signal, or one declared outside the lowered units):
    /// uses of the alias lower the aliased name (LRM 6.6.2).
    pub fn alias_target(&self, d: DeclId) -> Option<(ExprId, Option<SubtypeId>)> {
        if self.cx.objs.contains_key(&d) {
            return None;
        }
        match &self.db.decls[d].kind {
            DeclKind::Alias(Alias::Object { target, subtype }) => Some((*target, *subtype)),
            _ => None,
        }
    }

    /// The descriptor under which a constant or alias of subtype `st` sees an array with
    /// descriptor `dope`: that of `st` when it is constrained (checked to match in length,
    /// LRM 5.3.2.2, 6.6.2), else `dope`.
    pub fn dope_as(&mut self, dope: Reg, st: Option<SubtypeId>, what: &str, span: Span) -> R<Reg> {
        let Some(st) = st else {
            return Ok(dope);
        };
        // A record has no bounds to check or replace.
        let Some((_, dims)) = ty::array_of(self.db, self.db.base(st)) else {
            return Ok(dope);
        };
        let Some(want) = self.subtype_dope(st, span)? else {
            return Ok(dope);
        };
        let what = self.msg(what, yavs_ir::Dir::To);
        self.emit(Inst::SubtypeCheck {
            dope,
            want,
            dims,
            what,
        });
        Ok(want)
    }

    /// The driver slot through which this process drives signal `d` (an actual of an `out`
    /// signal parameter): its first `ProcessInfo::drivers` entry rooted at `d`, or the slot a
    /// signal parameter was given.
    pub fn driver_slot_of(&mut self, d: DeclId, span: Span) -> R<Reg> {
        if let Some(Obj::SignalRef {
            space,
            off,
            out: true,
        }) = self.cx.objs.get(&d).copied()
        {
            let a = self.addr(space, off + 16);
            return Ok(self.load(a, Width::B8, RegTy::I64));
        }
        // The first entry with the same root: callers with the actual's LSP use `driver_of`.
        match self
            .drivers
            .iter()
            .position(|l| l.root == yavs_sem::hir::ObjRoot::Decl(d))
        {
            Some(k) => {
                let slot = self.driver_base.get(k).copied().unwrap_or(k as u32);
                Ok(self.int(i64::from(slot)))
            }
            None => Err(self
                .cx
                .ice(span, "out signal actual without a driver of the process")),
        }
    }

    /// The driver slot for actual `e` of an `out` signal parameter, whose signal is `d`:
    /// the driven signal, through any alias (LRM 6.6.2).
    pub(crate) fn out_driver_slot(&mut self, d: DeclId, e: ExprId) -> R<Reg> {
        // A part of a composite signal: the driver slot of its first leaf.
        if let Some((cd, steps)) = self.comp_steps(e) {
            let span = self.db.exprs[e].span;
            let part = self.comp_select(cd, &steps, span)?;
            return self.leaf_slot(cd, e, &part, span);
        }
        let lsp = self.db.lsp(e);
        let d = match lsp.as_ref().map(|l| l.root) {
            Some(yavs_sem::hir::ObjRoot::Decl(r)) => r,
            _ => d,
        };
        // A signal of the process (not a signal parameter passed on): the driver of the
        // actual's longest static prefix, which sem collected for this call (LRM 14.7.2, #231).
        if let (Some(l), false) = (
            lsp,
            matches!(self.cx.objs.get(&d), Some(Obj::SignalRef { out: true, .. })),
        ) {
            let slot = self.driver_of(&l)?;
            return Ok(self.int(i64::from(slot)));
        }
        self.driver_slot_of(d, self.db.exprs[e].span)
    }

    /// Write `out`/`inout` temporaries back to their actuals after a procedure call. The
    /// actual's place is computed again: no register lives across a call that may suspend.
    pub fn copy_out(&mut self, span: Span) -> R<()> {
        for (obj, f, parts) in std::mem::take(&mut self.writeback_parts) {
            self.through_formal(f, obj, &parts, false, span)?;
        }
        for (off, e, st) in std::mem::take(&mut self.writeback) {
            let Some(e) = e else { continue };
            if self.is_comp(self.db.base(st)) {
                // A logic slice actual: the temporary's elements go back into it.
                let t = self.addr(Space::Frame(self.depth), off);
                let tmp = self.load(t, Width::B8, RegTy::Ptr);
                let p = self.place(e)?;
                let Place::Array { dope, .. } = p else {
                    return Err(self.cx.ice(span, "array write-back to a scalar"));
                };
                let what = format!("parameter of {}", self.name);
                self.store_place(p, V::A(tmp, dope), st, &what, span)?;
                continue;
            }
            let (_, rt, w) = self.scalar(st, span)?;
            let t = self.addr(Space::Frame(self.depth), off);
            let v = self.load(t, w, rt);
            let p = self.place(e)?;
            let what = format!("parameter of {}", self.name);
            self.store_place(p, V::S(v), st, &what, span)?;
        }
        Ok(())
    }

    /// The driver slot through which this process drives the first leaf of `part`, the part
    /// of composite signal `d` that actual `e` of an `out` signal parameter names: the slot of
    /// the actual's longest static prefix plus the leaf's position below it, or the slot a
    /// signal parameter was given plus the leaf's index.
    pub(crate) fn leaf_slot(&mut self, d: DeclId, e: ExprId, part: &SigPart, span: Span) -> R<Reg> {
        let Some((first, _, _)) = part.leaves.first() else {
            return Err(self.cx.ice(span, "composite signal part without leaves"));
        };
        if let Some(Obj::RecSignal(Some(p))) = self.cx.objs.get(&d).copied() {
            if !p.out {
                return Err(self
                    .cx
                    .ice(span, "`in` signal parameter as an `out` actual"));
            }
            let a = self.addr(p.space, p.slot_off());
            let base = self.load(a, Width::B8, RegTy::I64);
            let k = self.int(i64::from(first.sig));
            return Ok(self.bin(IntOp::Add, base, k));
        }
        let Some(lsp) = self.db.lsp(e) else {
            return Err(self
                .cx
                .ice(span, "signal actual without a longest static prefix"));
        };
        let base = self.driver_of(&lsp)?;
        let covered = self.leaves_under(d, &crate::func::lsp_fields(&lsp));
        let Some(k) = covered.iter().position(|l| l.path == first.path) else {
            return Err(self.cx.ice(span, "signal actual outside its driver"));
        };
        Ok(self.int(i64::from(base + k as u32)))
    }

    /// The driver slot of leaf `leaf` of an actual of a view formal, `rel` being the leaf's
    /// field path below the formal (part) the actual is associated with: the driver sem
    /// collected for it (`lsp` is the actual's), or for a signal parameter passed on its
    /// slot cell plus the leaf's index.
    pub(crate) fn view_leaf_slot(
        &mut self,
        d: DeclId,
        lsp: &Option<yavs_sem::hir::Lsp>,
        leaf: &crate::RecLeaf,
        rel: &[u32],
        span: Span,
    ) -> R<Reg> {
        Ok(match lsp {
            None => {
                let Some(Obj::RecSignal(Some(rp))) = self.cx.objs.get(&d).copied() else {
                    return Err(self.cx.ice(span, "signal parameter without its cells"));
                };
                let a = self.addr(rp.space, rp.slot_off());
                let base = self.load(a, Width::B8, RegTy::I64);
                let k = self.int(i64::from(leaf.sig));
                self.bin(IntOp::Add, base, k)
            }
            // The driver sem collected for this leaf of the actual, and the leaf's
            // position below it.
            Some(l) => {
                let mut l = l.clone();
                l.path = l
                    .path
                    .iter()
                    .cloned()
                    .chain(rel.iter().map(|&f| yavs_sem::hir::LspStep::Field(f)))
                    .collect();
                let Some(k) = self.driver_index(&l) else {
                    return Err(self
                        .cx
                        .ice(span, "view element without a driver of the process"));
                };
                let base = self.driver_base.get(k).copied().unwrap_or(k as u32);
                let under = crate::func::lsp_fields(&self.drivers[k]);
                let covered = self.leaves_under(d, &under);
                let Some(j) = covered.iter().position(|c| c.path == leaf.path) else {
                    return Err(self.cx.ice(span, "view element outside its driver"));
                };
                self.int(i64::from(base + j as u32))
            }
        })
    }

    /// The leaf references of actual `e` (part `part` of signal `d`) for a signal formal
    /// `p` with a mode view, then the unused slot cell: a leaf the view drives (`out`,
    /// `inout`, `buffer`) is a reference of one part carrying its own driver slot, since
    /// the process drives only those leaves and their slots need not be consecutive.
    fn view_leaves(
        &mut self,
        p: &Param,
        d: DeclId,
        e: ExprId,
        part: &SigPart,
        span: Span,
        cells: &mut Vec<Reg>,
    ) -> R<()> {
        let db = self.db;
        let tree = db.port_mode_tree(p.mode);
        let lsp = match self.cx.objs.get(&d) {
            Some(Obj::RecSignal(Some(_))) => None,
            _ => Some(self.db.lsp(e).ok_or_else(|| {
                self.cx
                    .ice(span, "signal actual without a longest static prefix")
            })?),
        };
        for (leaf, r, _) in &part.leaves {
            let rel = &leaf.path[part.path.len().min(leaf.path.len())..];
            let mode = tree.leaf_mode(rel);
            if !matches!(
                mode,
                Some(
                    yavs_sem::hir::Mode::Out
                        | yavs_sem::hir::Mode::Inout
                        | yavs_sem::hir::Mode::Buffer
                )
            ) {
                cells.push(*r);
                continue;
            }
            let slot = self.view_leaf_slot(d, &lsp, leaf, rel, span)?;
            let table = self.alloc_n(yavs_ir::SIG_PART_BYTES);
            self.store(table, *r, Width::Sig);
            let s = self.offset(table, 16);
            self.store(s, slot, Width::B8);
            let f = self.offset(table, 24);
            let z = self.int(0);
            self.store(f, z, Width::B8);
            let dst = self.reg(RegTy::Sig);
            let one = self.int(1);
            self.emit(Inst::SigParts {
                dst,
                parts: table,
                n: one,
            });
            cells.push(dst);
        }
        // Every driven leaf carries its slot.
        let none = self.int(-1);
        cells.push(none);
        Ok(())
    }

    /// The signal an expression names.
    pub fn sig_expr(&mut self, e: ExprId) -> R<Reg> {
        Ok(self.sig_name(e)?.1)
    }

    /// The descriptor of an array signal named by `e` (`None` for a scalar).
    fn dope_of_signal(&mut self, e: ExprId) -> R<Option<Reg>> {
        Ok(self.sig_name(e)?.2)
    }

    /// The IR type of object `d`.
    pub fn decl_ty(&mut self, d: DeclId, span: Span) -> R<TyId> {
        match ty::obj_subtype(self.db, d) {
            Some(st) => self.ty(self.db.base(st), span),
            None => Err(self.cx.ice(span, "not an object")),
        }
    }

    /// The descriptor of an array signal (from its subtype, evaluated in this instance);
    /// `None` for a scalar signal.
    /// The pool constant holding the descriptor of object `d` of subtype `st` when every bound
    /// is locally static (the same in every instance), made once.
    fn static_dope(&mut self, d: DeclId, st: SubtypeId, span: Span) -> Option<yavs_ir::ConstId> {
        if let Some(c) = self.cx.const_dope.get(&d).copied() {
            return Some(c);
        }
        let db = self.db;
        let t = db.base(st);
        let tid = self.ty(t, span).ok()?;
        let b = crate::composite::static_desc(
            db,
            self.fb.program(),
            t,
            &db.subtypes[st].constraint,
            tid,
        )?;
        let n = Ty::desc_size(self.fb.program().types(), tid)?;
        if b.is_empty() || n != b.len() as u64 {
            return None;
        }
        let k = self.fb.program().konst(yavs_ir::ConstData::Bytes(b));
        self.cx.const_dope.insert(d, k);
        Some(k)
    }

    pub fn sig_dope(&mut self, d: DeclId, span: Span) -> R<Option<Reg>> {
        if let Some((t, st)) = self.alias_target(d) {
            let Some(dope) = self.dope_of_signal(t)? else {
                return Ok(None);
            };
            let what = self.db.decls[d].name.to_string();
            return self.dope_as(dope, st, &what, span).map(Some);
        }
        if self.db.external_signal_alias(d).is_some() {
            // An alias of an external signal: its descriptor slot holds the target's bounds.
            if !self.cx.sig_dopes.contains_key(&d) {
                return Ok(None);
            }
            let (space, off) = *self.cx.sig_dopes.get(&d).ok_or(Failed)?;
            return Ok(Some(self.addr(space, off)));
        }
        let DeclKind::Object(o) = &self.db.decls[d].kind else {
            return Err(self.cx.ice(span, "signal is not an object"));
        };
        if !ty::is_array(self.db, self.db.base(o.subtype)) {
            return Ok(None);
        }
        if let Some(&(space, off)) = self.cx.sig_dopes.get(&d) {
            // (A static subtype has its descriptor in the pool, not in the instance's data.)
            if let Some(c) = self.static_dope(d, o.subtype, span) {
                return Ok(Some(self.const_addr(c)));
            }
            return Ok(Some(self.addr(space, off)));
        }
        if let Some(Obj::SignalRef { space, off, out }) = self.cx.objs.get(&d).copied() {
            // An array signal parameter: the actual's descriptor follows the reference (and
            // the driver slot of an `out` parameter).
            let a = self.addr(space, off + if out { 24 } else { 16 });
            return Ok(Some(self.load(a, Width::B8, RegTy::Ptr)));
        }
        match self.subtype_dope(o.subtype, span)? {
            Some(d) => Ok(Some(d)),
            None => Err(self.cx.unsupported(span, "unconstrained array signals")),
        }
    }

    /// The signal (or signal element or slice) an expression names: its root declaration,
    /// the reference and, for an array, the descriptor of the named part. Kernel offsets
    /// follow docs/decisions.md "Elaboration" (row major, mirrored for logic leaves).
    pub fn sig_name(&mut self, e: ExprId) -> R<(DeclId, Reg, Option<Reg>)> {
        let db = self.db;
        let x = &db.exprs[e];
        let span = x.span;
        if let Some((d, steps)) = self.comp_steps(e) {
            let p = self.comp_select(d, &steps, span)?;
            let [(leaf, r, _)] = p.leaves.as_slice() else {
                return Err(self
                    .cx
                    .unsupported(span, "record parts of composite signals here"));
            };
            if leaf.path != p.path {
                return Err(self
                    .cx
                    .unsupported(span, "record parts of composite signals here"));
            }
            let dope = ty::is_array(db, p.t).then_some(p.desc);
            return Ok((d, *r, dope));
        }
        if let Some((d, steps)) = self.nested_steps(e) {
            let (r, dope) = self.sig_steps(d, &steps, span)?;
            return Ok((d, r, dope));
        }
        let ExprKind::Obj(d) = &x.kind else {
            return Err(self
                .cx
                .unsupported(span, "signal names with record or nested selections"));
        };
        let dope = match self.cx.objs.get(d) {
            Some(Obj::Signal { .. } | Obj::SignalRef { .. }) => self.sig_dope(*d, span)?,
            None if self.alias_target(*d).is_some() => self.sig_dope(*d, span)?,
            _ => None,
        };
        Ok((*d, self.sig_of(*d, span)?, dope))
    }

    /// The root object and the indices and slices of a name below a signal without records:
    /// `s(i)`, `s(i)(j)`, `s(i, j)`, `s(i)(7 downto 4)`.
    fn nested_steps(&self, e: ExprId) -> Option<(DeclId, Vec<Step<'a>>)> {
        let db = self.db;
        let mut steps = Vec::new();
        let mut e = e;
        let d = loop {
            match &db.exprs[e].kind {
                ExprKind::Index { prefix, indices } => {
                    steps.push(Step::Index(indices));
                    e = *prefix;
                }
                ExprKind::Slice { prefix, range } => {
                    steps.push(Step::Slice(range));
                    e = *prefix;
                }
                ExprKind::Obj(d) => break *d,
                _ => return None,
            }
        };
        if steps.is_empty() {
            return None;
        }
        steps.reverse();
        Some((d, steps))
    }

    /// The elements of signal `d` (without records) that `steps` select, and their descriptor
    /// for an array: one contiguous kernel range by the rule of `NetGraph::select`, row major
    /// over all scalars, mirrored for logic signals.
    pub fn sig_steps(
        &mut self,
        d: DeclId,
        steps: &[Step<'_>],
        span: Span,
    ) -> R<(Reg, Option<Reg>)> {
        let db = self.db;
        let sig = match self.cx.objs.get(&d).copied() {
            Some(Obj::Signal { sig }) => Ok(sig),
            Some(Obj::SignalRef { .. }) => Err(self.sig_of(d, span)?),
            None if self.alias_target(d).is_some() => Err(self.sig_of(d, span)?),
            _ => return Err(self.cx.ice(span, "selection of a non-signal")),
        };
        let Some(root) = self.sig_dope(d, span)? else {
            return Err(self.cx.ice(span, "indexed scalar signal"));
        };
        let root_t = db.base(ty::obj_subtype(db, d).ok_or(Failed)?);
        let what = db.decls[d].name.to_string();
        let w = self.walk_steps(root_t, root, steps, &what, span)?;
        let (it, idesc) = w.inst.unwrap_or((w.t, w.desc));
        let total = self.leaf_count(root_t, root, &[], span)?;
        let k = self.leaf_count(it, idesc, &[], span)?;
        let a = self.bin(IntOp::Mul, w.offset, k);
        let len = self.bin(IntOp::Mul, w.count, k);
        let tid = self.ty(root_t, span)?;
        let a = if self.is_logic(tid) {
            let r = self.bin(IntOp::Sub, total, a);
            self.bin(IntOp::Sub, r, len)
        } else {
            a
        };
        let dope = ty::is_array(db, w.t).then_some(w.desc);
        Ok((self.sub_sig(sig, a, len), dope))
    }

    /// The steps from a composite signal (`Obj::RecSignal`) down to the part `e` names.
    pub fn comp_steps(&self, e: ExprId) -> Option<(DeclId, Vec<Step<'a>>)> {
        let db = self.db;
        let mut steps = Vec::new();
        let mut e = e;
        loop {
            match &db.exprs[e].kind {
                ExprKind::Field { prefix, elem } => {
                    steps.push(Step::Field(*elem));
                    e = *prefix;
                }
                ExprKind::Index { prefix, indices } => {
                    steps.push(Step::Index(indices));
                    e = *prefix;
                }
                ExprKind::Slice { prefix, range } => {
                    steps.push(Step::Slice(range));
                    e = *prefix;
                }
                ExprKind::Obj(d) if matches!(self.cx.objs.get(d), Some(Obj::RecSignal(_))) => {
                    steps.reverse();
                    return Some((*d, steps));
                }
                ExprKind::Obj(d) if self.alias_target(*d).is_some() => {
                    e = self.alias_target(*d)?.0;
                }
                _ => return None,
            }
        }
    }

    /// The leaves of composite signal `d` under field path `path`, in leaf order.
    pub fn leaves_under(&self, d: DeclId, path: &[u32]) -> Vec<crate::RecLeaf> {
        self.cx
            .rec_leaves
            .get(&d)
            .map(|ls| {
                ls.iter()
                    .filter(|l| l.path.starts_with(path))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Elements of all dimensions of one level of an array of type `tid`.
    fn elems(&mut self, tid: TyId, desc: Reg) -> Reg {
        let dims = match self.fb.program().types().get(tid) {
            Some(Ty::Array { dims, .. }) => *dims,
            _ => 1,
        };
        let mut n = self.int(1);
        for dim in 0..dims {
            let m = self.reg(RegTy::I64);
            self.emit(Inst::DopeLen {
                dst: m,
                dope: desc,
                dim,
            });
            n = self.bin(IntOp::Mul, n, m);
        }
        n
    }

    /// Elements of the leaf at field path `rel` in one value of type `t` (descriptor `desc`).
    pub(crate) fn leaf_count(&mut self, t: TypeId, desc: Reg, rel: &[u32], span: Span) -> R<Reg> {
        let db = self.db;
        let (mut t, mut desc, mut rel) = (t, desc, rel);
        let mut n = self.int(1);
        loop {
            let tid = self.ty(t, span)?;
            match &db.types[t].kind {
                TypeKind::Record { .. } => {
                    let [f, rest @ ..] = rel else {
                        return Err(self.cx.ice(span, "leaf path ends at a record"));
                    };
                    desc = self.field_desc(tid, desc, *f, span)?;
                    t = db.base(self.field_subtype(t, *f)?);
                    rel = rest;
                }
                TypeKind::Array { elem, .. } => {
                    let m = self.elems(tid, desc);
                    n = self.bin(IntOp::Mul, n, m);
                    desc = self.elem_desc(tid, desc)?.1;
                    t = db.base(*elem);
                }
                _ => return Ok(n),
            }
        }
    }

    /// The descriptor of composite signal `d` (the one the elaborator wrote, with the bounds
    /// of the initial value or the actual, or the actual's of a signal parameter) and the walk
    /// of `steps` from it. Reads no signal value, so it also works during elaboration.
    fn comp_walk(
        &mut self,
        d: DeclId,
        steps: &[Step<'_>],
        span: Span,
    ) -> R<(Option<crate::RecParam>, Reg, Walk)> {
        let db = self.db;
        let st = ty::obj_subtype(db, d).ok_or(Failed)?;
        let param = match self.cx.objs.get(&d).copied() {
            Some(Obj::RecSignal(p)) => p,
            _ => {
                return Err(self
                    .cx
                    .ice(span, "composite selection of a non-composite signal"));
            }
        };
        let root = match (self.cx.sig_dopes.get(&d).copied(), param) {
            (_, Some(p)) if p.desc => {
                let a = self.addr(p.space, p.desc_off());
                self.load(a, Width::B8, RegTy::Ptr)
            }
            (Some((space, off)), _) => match self.static_dope(d, st, span) {
                Some(c) => self.const_addr(c),
                None => self.addr(space, off),
            },
            (None, _) => match self.desc_of(st, None, span)? {
                Some(r) => r,
                None => return Err(self.cx.ice(span, "composite signal without a descriptor")),
            },
        };
        let what = db.decls[d].name.to_string();
        let w = self.walk_steps(db.base(st), root, steps, &what, span)?;
        Ok((param, root, w))
    }

    /// The type and descriptor of record signal `d` as a whole, without its leaves (a read or
    /// drive through a leaf table, `rec_table`).
    pub(crate) fn comp_head(&mut self, d: DeclId, steps: &[Step<'_>], span: Span) -> R<SigPart> {
        let (_, _, w) = self.comp_walk(d, steps, span)?;
        let tid = self.ty(w.t, span)?;
        let part = self.part_regs(&w, tid);
        Ok(SigPart {
            t: w.t,
            tid,
            desc: w.desc,
            path: w.path,
            leaves: Vec::new(),
            part,
        })
    }

    /// The first element and the number of elements of the outermost array of a walk's part and
    /// of the signal it is a part of, in the units of the elements of the part's value: `None`
    /// for a part no index or slice selected (the whole signal or a field of it).
    fn part_regs(&mut self, w: &Walk, tid: TyId) -> Option<yavs_ir::Part> {
        if !w.selected {
            return None;
        }
        let (first, outer) = if w.inst.is_none() && ty::is_array(self.db, w.t) {
            // The whole value of an array field: its elements are the units.
            let u = self.elems(tid, w.desc);
            (
                self.bin(IntOp::Mul, w.offset, u),
                self.bin(IntOp::Mul, w.outer, u),
            )
        } else {
            (w.offset, w.outer)
        };
        Some(yavs_ir::Part { first, outer })
    }

    /// The part of composite signal `d` that `steps` select, with a reference to the selected
    /// elements of each leaf below it. Offsets follow the one sub-element map (rule 2, the
    /// rule of `yavs_elab::NetGraph::select`): a leaf holds its field of every element in row
    /// major order, mirrored for logic leaves.
    pub fn comp_select(&mut self, d: DeclId, steps: &[Step<'_>], span: Span) -> R<SigPart> {
        let db = self.db;
        let root_t = db.base(ty::obj_subtype(db, d).ok_or(Failed)?);
        let (param, root, w) = self.comp_walk(d, steps, span)?;
        let tid = self.ty(w.t, span)?;
        let part = self.part_regs(&w, tid);
        let (t, desc, path, offset, count) = (w.t, w.desc, w.path, w.offset, w.count);
        let (it, idesc) = w.inst.unwrap_or((t, desc));
        let mut leaves = Vec::new();
        for l in self.leaves_under(d, &path) {
            let total = self.leaf_count(root_t, root, &l.path, span)?;
            let k = self.leaf_count(it, idesc, &l.path[path.len()..], span)?;
            let a = self.bin(IntOp::Mul, offset, k);
            let len = self.bin(IntOp::Mul, count, k);
            let ltid = self.ty(db.base(l.st), span)?;
            let a = if self.is_logic(ltid) {
                let r = self.bin(IntOp::Sub, total, a);
                self.bin(IntOp::Sub, r, len)
            } else {
                a
            };
            let r = match param {
                // The leaf's reference covers the actual's elements, in kernel order.
                Some(p) => {
                    let at = self.addr(p.space, p.off + 16 * l.sig);
                    let base = self.load(at, Width::Sig, RegTy::Sig);
                    self.sub_sig(Err(base), a, len)
                }
                None => self.sigval_at(l.sig, a, len),
            };
            leaves.push((l, r, len));
        }
        Ok(SigPart {
            t,
            tid,
            desc,
            path,
            leaves,
            part,
        })
    }

    /// Follow `steps` from a value of type `root_t` with descriptor `root` (`Walk`).
    fn walk_steps(
        &mut self,
        root_t: TypeId,
        root: Reg,
        steps: &[Step<'_>],
        what: &str,
        span: Span,
    ) -> R<Walk> {
        let db = self.db;
        let (mut t, mut desc) = (root_t, root);
        let mut path = Vec::new();
        let (mut offset, mut count) = (self.int(0), self.int(1));
        let mut outer = self.int(1);
        let mut selected = false;
        let mut inst: Option<(TypeId, Reg)> = None;
        for s in steps {
            let tid = self.ty(t, span)?;
            match *s {
                Step::Field(_) if inst.is_some() => {
                    return Err(self.cx.ice(span, "record element of a slice"));
                }
                Step::Field(n) => {
                    desc = self.field_desc(tid, desc, n, span)?;
                    t = db.base(self.field_subtype(t, n)?);
                    path.push(n);
                }
                Step::Rebase(st) => desc = self.dope_as(desc, Some(st), what, span)?,
                Step::Index(ix) => {
                    let lin = self.flat_pos(desc, ix, what)?;
                    if inst.take().is_some() {
                        // An element of a slice: counted from the slice's first instance.
                        offset = self.bin(IntOp::Add, offset, lin);
                        count = self.int(1);
                    } else {
                        let n = self.elems(tid, desc);
                        let o = self.bin(IntOp::Mul, offset, n);
                        offset = self.bin(IntOp::Add, o, lin);
                        outer = self.bin(IntOp::Mul, outer, n);
                    }
                    selected = true;
                    desc = self.elem_desc(tid, desc)?.1;
                    t = db.base(ty::array_of(db, t).ok_or(Failed)?.0);
                }
                Step::Slice(r) => {
                    let (p, len, nd) = self.slice_of(desc, r, what, span)?;
                    if inst.is_some() {
                        // A slice of a slice: the same instances, fewer of them.
                        offset = self.bin(IntOp::Add, offset, p);
                    } else {
                        let n = self.elems(tid, desc);
                        let o = self.bin(IntOp::Mul, offset, n);
                        offset = self.bin(IntOp::Add, o, p);
                        outer = self.bin(IntOp::Mul, outer, n);
                        let ed = self.elem_desc(tid, desc)?.1;
                        inst = Some((db.base(ty::array_of(db, t).ok_or(Failed)?.0), ed));
                    }
                    selected = true;
                    count = len;
                    desc = self.slice_desc(tid, desc, nd);
                }
            }
        }
        Ok(Walk {
            t,
            desc,
            path,
            offset,
            count,
            inst,
            outer,
            selected,
        })
    }

    /// Logic flag, plane code bias and the widths of the elements of leaf `l`: as a scalar
    /// (a record field or a scalar leaf in memory) and as an array element, which is also
    /// the kernel's width (`Ty::elem_size`: narrow integers are 4 bytes there).
    pub(crate) fn leaf_facts(&mut self, l: &crate::RecLeaf, span: Span) -> R<LeafFacts> {
        let mut t = self.db.base(l.st);
        while let Some((e, _)) = ty::array_of(self.db, t) {
            t = self.db.base(e);
        }
        let tid = self.ty(t, span)?;
        let p = self.fb.program();
        let (Some(sw), Some(ew)) = (
            crate::composite::scalar_w(p, tid),
            crate::composite::elem_w(p, tid),
        ) else {
            return Err(self
                .cx
                .ice(span, "leaf of a composite signal is not scalar"));
        };
        let logic = self.is_logic(tid);
        Ok((logic, self.code_bias(tid), sw, ew))
    }

    /// Copy the leaf at field path `rel` of the value of type `t` at `(addr, desc)` between
    /// the value and the leaf buffer `buf` (`leaf_copy`), run by run in row-major order: a
    /// scalar, or an array of scalars stored as one block; `k` is the leaf position of the
    /// run's first element, a register this advances.
    #[allow(clippy::too_many_arguments)]
    fn leaf_walk(
        &mut self,
        t: TypeId,
        addr: Reg,
        desc: Reg,
        rel: &[u32],
        k: Reg,
        buf: (Reg, Reg),
        facts: LeafFacts,
        into_value: bool,
        span: Span,
    ) -> R<()> {
        let db = self.db;
        let tid = self.ty(t, span)?;
        let m = match &db.types[t].kind {
            TypeKind::Record { .. } => {
                let [n, rest @ ..] = rel else {
                    return Err(self.cx.ice(span, "leaf path ends at a record"));
                };
                let (a, d) = self.field_addr(tid, addr, desc, *n, span)?;
                let ft = db.base(self.field_subtype(t, *n)?);
                return self.leaf_walk(ft, a, d, rest, k, buf, facts, into_value, span);
            }
            TypeKind::Array { elem, .. } => {
                let et = db.base(*elem);
                let etid = self.ty(et, span)?;
                let scalar = crate::composite::scalar_w(self.fb.program(), etid).is_some();
                if !scalar && !self.is_logic(tid) {
                    let n = self.elems(tid, desc);
                    let ed = self.elem_desc(tid, desc)?.1;
                    return self.for_n(n, |fx, i| {
                        let a = fx.elem_addr(tid, addr, desc, i);
                        fx.leaf_walk(et, a, ed, rel, k, buf, facts, into_value, span)
                    });
                }
                let m = self.layout_count(tid, desc);
                self.leaf_copy(buf, facts, addr, Some(m), k, into_value);
                m
            }
            _ => {
                let one = self.int(1);
                self.leaf_copy(buf, facts, addr, None, k, into_value);
                one
            }
        };
        let next = self.bin(IntOp::Add, k, m);
        self.emit(Inst::Move { dst: k, src: next });
        Ok(())
    }

    /// Copy one run of a leaf between a value at `addr` and the leaf buffer `buf` of `len`
    /// elements (`into_value`: from the buffer into the value).
    #[allow(clippy::too_many_arguments)]
    fn leaf_copy(
        &mut self,
        (buf, len): (Reg, Reg),
        (logic, bias, sw, w): LeafFacts,
        addr: Reg,
        block: Option<Reg>,
        k: Reg,
        into_value: bool,
    ) {
        match (logic, block) {
            (true, None) => {
                let one = self.int(1);
                let last = self.bin(IntOp::Sub, len, one);
                let idx = self.bin(IntOp::Sub, last, k);
                if into_value {
                    let c = self.reg(RegTy::I64);
                    self.emit(Inst::LGet {
                        dst: c,
                        base: buf,
                        idx,
                    });
                    let v = self.dec(c, bias);
                    self.store(addr, v, w);
                } else {
                    let x = self.load(addr, w, RegTy::I64);
                    let v = self.enc(x, bias);
                    self.emit(Inst::LSet { base: buf, idx, v });
                }
            }
            (true, Some(m)) => {
                let r = self.bin(IntOp::Sub, len, k);
                let off = self.bin(IntOp::Sub, r, m);
                let z = self.int(0);
                let (dst, dst_off, src, src_off) = if into_value {
                    (addr, z, buf, off)
                } else {
                    (buf, off, addr, z)
                };
                self.emit(Inst::LCopy {
                    dst,
                    dst_off,
                    src,
                    src_off,
                    n: m,
                });
            }
            (false, None) if sw != w => {
                // A narrow integer scalar: 8 bytes in the value, 4 in the kernel.
                let at = self.reg(RegTy::Ptr);
                self.emit(Inst::Index {
                    dst: at,
                    base: buf,
                    pos: k,
                    stride: w.bytes(),
                });
                if into_value {
                    let x = self.load(at, w, RegTy::I64);
                    self.store(addr, x, sw);
                } else {
                    let x = self.load(addr, sw, RegTy::I64);
                    self.store(at, x, w);
                }
            }
            (false, m) => {
                let at = self.reg(RegTy::Ptr);
                self.emit(Inst::Index {
                    dst: at,
                    base: buf,
                    pos: k,
                    stride: w.bytes(),
                });
                let ws = self.int(i64::from(w.bytes()));
                let bytes = match m {
                    Some(m) => self.bin(IntOp::Mul, m, ws),
                    None => ws,
                };
                let (dst, src) = if into_value { (addr, at) } else { (at, addr) };
                self.emit(Inst::Copy { dst, src, bytes });
            }
        }
    }

    /// `'event`, `'active` (any scalar), `'last_event`, `'last_active` (the most recent of
    /// its scalars) and `'last_value` (each scalar's) of part `p` of a composite signal.
    fn comp_attr(&mut self, attr: PredefAttr, p: &SigPart, span: Span) -> R<V> {
        use PredefAttr::*;
        let (kind, any) = match attr {
            Event => (AttrKind::Event, true),
            Active => (AttrKind::Active, true),
            LastEvent => (AttrKind::LastEvent, false),
            LastActive => (AttrKind::LastActive, false),
            _ => return self.read_leaves(p, true, span),
        };
        let mut acc: Option<Reg> = None;
        for &(_, sig, _) in &p.leaves {
            let dst = self.reg(RegTy::I64);
            self.emit(Inst::SigAttr { dst, kind, sig });
            acc = Some(match acc {
                None => dst,
                Some(a) if any => {
                    let r = self.reg(RegTy::I64);
                    self.emit(Inst::Bool {
                        op: BoolOp::Or,
                        dst: r,
                        a,
                        b: dst,
                    });
                    r
                }
                Some(a) => {
                    let lt = self.cmp(CmpOp::Lt, dst, a);
                    self.select(lt, dst, a, RegTy::I64)
                }
            });
        }
        acc.map(V::S)
            .ok_or_else(|| self.cx.ice(span, "composite signal part without leaves"))
    }

    /// Read part `p` of a composite signal into a new value, leaf by leaf.
    pub fn read_comp(&mut self, p: &SigPart, span: Span) -> R<V> {
        self.read_leaves(p, false, span)
    }

    /// Read the current values, or with `last` the `'last_value`s, of part `p` of a composite
    /// signal into a new value, leaf by leaf.
    fn read_leaves(&mut self, p: &SigPart, last: bool, span: Span) -> R<V> {
        let size = self.size_of(p.tid, p.desc);
        let val = self.alloc(size);
        self.read_leaves_into(p, last, val, span)?;
        Ok(V::A(val, p.desc))
    }

    /// The whole value of record signal `d` for a process with a sensitivity list, from the copy
    /// the process keeps ([`SigCopies`]), made at the first read. `None` outside such processes
    /// and for a signal the process reads as a whole once.
    fn sigcopy_of(&mut self, d: DeclId, span: Span) -> R<Option<(Reg, Reg)>> {
        let Some(sc) = self.sigcopy.as_mut() else {
            return Ok(None);
        };
        if let Some(&(_, off)) = sc.copies.iter().find(|c| c.0 == d) {
            return Ok(Some(self.copy_slot(off)));
        }
        if !sc.wanted.contains(&d) {
            return Ok(None);
        }
        let (decl_end, top, (pstmt, pspan)) = (sc.decl_end, sc.top, sc.stmt);
        let (here, live, cur) = (self.fb.current(), self.live, self.cur);
        let old = self.fb.set_stmt(Some(pstmt));
        // The object once, in the declarative part (before the jump that ends it), in a slot
        // like a variable's: it lives as long as the process (temporaries end with the statement).
        let code = self.fb.take(decl_end);
        let Some(((jump, js), keep)) = code.split_last() else {
            return Err(self.cx.ice(span, "empty end of a declarative part"));
        };
        self.fb.splice(decl_end, 0, keep.to_vec());
        let head = self.fb.block();
        self.fb.at(decl_end);
        self.fb.emit(Inst::Br { to: head });
        self.fb.at(head);
        self.live = true;
        let table = self.rec_table(d, span)?;
        let p = self.comp_head(d, &[], span)?;
        let ds = self.desc_size(p.tid);
        let size = self.size_of(p.tid, p.desc);
        let h = self.reg(RegTy::Handle);
        let zero = self.bytes_addr(vec![0]);
        self.emit(Inst::NewFill {
            dst: h,
            ty: p.tid,
            desc: p.desc,
            dsize: ds,
            data: zero,
            size,
            fill: 1,
            shared: false,
        });
        let (h, data, desc) = self.new_object_end(h, ds)?;
        let off = self.fb.slot(24, 8);
        self.fb.root(off + 16, yavs_ir::RootKind::Handle);
        let slot = self.addr(Space::Frame(self.depth), off);
        self.store(slot, data, Width::B8);
        let at = self.offset(slot, 8);
        self.store(at, desc, Width::B8);
        let at = self.offset(slot, 16);
        self.store(at, h, Width::B8);
        self.fb.set_stmt(*js);
        self.fb.emit(*jump);
        // Each activation fills it again, before the first statement of the body; the
        // statement marker frees the leaf buffers.
        self.fb.set_stmt(Some(pstmt));
        let first = self.fb.take(top);
        let rest = self.fb.block();
        self.fb.splice(rest, 0, first);
        let head = self.fb.block();
        self.fb.at(top);
        self.fb.emit(Inst::Br { to: head });
        self.fb.at(head);
        self.live = true;
        self.fb.stmt(yavs_ir::StmtKind::Implicit, pspan);
        let (data, cdesc) = self.copy_slot(off);
        match table {
            Some(tab) => self.emit(Inst::SigGather {
                dst: data,
                desc: cdesc,
                tab,
                part: None,
            }),
            None => {
                let p = self.comp_select(d, &[], span)?;
                self.read_leaves_into(&p, false, data, span)?;
            }
        }
        self.fb.emit(Inst::Br { to: rest });
        self.fb.set_stmt(old);
        self.fb.at(if here == top { rest } else { here });
        self.live = live;
        self.cur = cur;
        if let Some(sc) = self.sigcopy.as_mut() {
            sc.copies.push((d, off));
        }
        Ok(Some(self.copy_slot(off)))
    }

    /// The data and descriptor addresses of a process's copy of a signal in frame slot `off`.
    fn copy_slot(&mut self, off: u32) -> (Reg, Reg) {
        let slot = self.addr(Space::Frame(self.depth), off);
        let data = self.load(slot, Width::B8, RegTy::Ptr);
        let at = self.offset(slot, 8);
        let desc = self.load(at, Width::B8, RegTy::Ptr);
        (data, desc)
    }

    /// [`Fx::read_leaves`] into the value at `val`.
    fn read_leaves_into(&mut self, p: &SigPart, last: bool, val: Reg, span: Span) -> R<()> {
        // The addresses of the records on the path to the leaf last made, to share with the next
        // leaf: (field, address and descriptor of what the field selects, its type).
        let mut walked: Vec<(u32, Reg, Reg, TypeId)> = Vec::new();
        for (l, sref, len) in &p.leaves {
            let facts = self.leaf_facts(l, span)?;
            let rel = &l.path[p.path.len()..];
            // A `std_ulogic` field below records only: its code, stored where it lies.
            if !last
                && facts.0
                && ty::array_of(self.db, self.db.base(l.st)).is_none()
                && !rel.is_empty()
            {
                let (mut at, mut keep) = ((val, p.desc), 0);
                let mut t = p.t;
                let mut ok = true;
                for (i, &n) in rel.iter().enumerate() {
                    if !matches!(self.db.types[t].kind, TypeKind::Record { .. }) {
                        ok = false;
                        break;
                    }
                    if keep == i && walked.get(i).is_some_and(|w| w.0 == n) {
                        let w = walked[i];
                        (at, t) = ((w.1, w.2), w.3);
                        keep = i + 1;
                        continue;
                    }
                    let tid = self.ty(t, span)?;
                    let a = self.field_addr(tid, at.0, at.1, n, span)?;
                    t = self.db.base(self.field_subtype(t, n)?);
                    walked.truncate(i);
                    walked.push((n, a.0, a.1, t));
                    at = a;
                    keep = i + 1;
                }
                if ok {
                    let raw = self.reg(RegTy::I64);
                    self.emit(Inst::SigReadScalar {
                        dst: raw,
                        sig: *sref,
                    });
                    let v = self.dec(raw, facts.1);
                    self.store(at.0, v, facts.3);
                    continue;
                }
            }
            // A logic vector that is a field of nested records: read it where it lies.
            if facts.0 && self.plain_vector_field(p.t, rel, span)? {
                let (mut a, mut d, mut t) = (val, p.desc, p.t);
                for &n in rel {
                    let tid = self.ty(t, span)?;
                    (a, d) = self.field_addr(tid, a, d, n, span)?;
                    t = self.db.base(self.field_subtype(t, n)?);
                }
                self.emit(if last {
                    Inst::SigLastValue { dst: a, sig: *sref }
                } else {
                    Inst::SigRead { dst: a, sig: *sref }
                });
                continue;
            }
            let size = self.leaf_size(l, *len, span)?;
            let buf = self.alloc(size);
            self.emit(if last {
                Inst::SigLastValue {
                    dst: buf,
                    sig: *sref,
                }
            } else {
                Inst::SigRead {
                    dst: buf,
                    sig: *sref,
                }
            });
            let k = self.int(0);
            self.leaf_walk(p.t, val, p.desc, rel, k, (buf, *len), facts, true, span)?;
        }
        Ok(())
    }

    /// Does the field path `rel` from record type `t` lead through records only to an array
    /// of logic scalars, whose planes then lie in one piece in the record's value?
    fn plain_vector_field(&mut self, t: TypeId, rel: &[u32], span: Span) -> R<bool> {
        let mut t = t;
        for &n in rel {
            if !matches!(self.db.types[t].kind, TypeKind::Record { .. }) {
                return Ok(false);
            }
            t = self.db.base(self.field_subtype(t, n)?);
        }
        let tid = self.ty(t, span)?;
        Ok(!rel.is_empty() && ty::is_array(self.db, t) && self.is_logic(tid))
    }

    /// Write the elements of leaf `l` (relative field path `rel`) of value `(data, desc)` of
    /// part type `t` into `buf`, a leaf buffer of `len` elements.
    #[allow(clippy::too_many_arguments)]
    pub fn leaf_of_value(
        &mut self,
        l: &crate::RecLeaf,
        rel: &[u32],
        t: TypeId,
        (data, desc): (Reg, Reg),
        (buf, len): (Reg, Reg),
        span: Span,
    ) -> R<()> {
        let facts = self.leaf_facts(l, span)?;
        let k = self.int(0);
        self.leaf_walk(t, data, desc, rel, k, (buf, len), facts, false, span)
    }

    /// Bytes of a buffer of `len` elements of leaf `l`: planes for logic, else `len * w`.
    pub fn leaf_size(&mut self, l: &crate::RecLeaf, len: Reg, span: Span) -> R<Reg> {
        let (logic, _, _, w) = self.leaf_facts(l, span)?;
        Ok(if logic {
            let k = self.int(63);
            let a = self.bin(IntOp::Add, len, k);
            let b = self.int(64);
            let blocks = self.bin(IntOp::Div, a, b);
            let bs = self.int(32);
            self.bin(IntOp::Mul, blocks, bs)
        } else {
            let ws = self.int(i64::from(w.bytes()));
            self.bin(IntOp::Mul, len, ws)
        })
    }

    /// Elements `offset .. offset + len` (kernel order) of instance signal `Ok(sig)`, or of the
    /// signal reference `Err(r)` (a signal parameter: the offset is relative to its elements).
    pub fn sub_sig(&mut self, sig: Result<u32, Reg>, offset: Reg, len: Reg) -> Reg {
        let r = match sig {
            Ok(sig) => return self.sigval_at(sig, offset, len),
            Err(r) => r,
        };
        // A reference of several parts is sliced across them (`Inst::SigSlice`).
        let dst = self.reg(RegTy::Sig);
        self.emit(Inst::SigSlice {
            dst,
            sig: r,
            offset,
            len,
        });
        dst
    }

    /// Elements `offset .. offset + len` (kernel order) of instance signal `sig`.
    pub fn sigval_at(&mut self, sig: u32, offset: Reg, len: Reg) -> Reg {
        let dst = self.reg(RegTy::Sig);
        self.emit(Inst::SigVal {
            dst,
            sig,
            offset,
            len,
        });
        dst
    }

    /// A reference to all of scalar instance signal `sig`.
    pub fn sigval(&mut self, sig: u32) -> Reg {
        let offset = self.int(0);
        let len = self.int(1);
        let dst = self.reg(RegTy::Sig);
        self.emit(Inst::SigVal {
            dst,
            sig,
            offset,
            len,
        });
        dst
    }

    /// The descriptor of array variable `d` whose slot is at `slot`: its constant when the
    /// bounds are static, else the pointer the slot holds.
    fn array_dope(&mut self, d: DeclId, slot: Reg) -> Reg {
        if let Some(c) = self.cx.const_dope.get(&d).copied() {
            return self.const_addr(c);
        }
        let dp = self.offset(slot, 8);
        self.load(dp, Width::B8, RegTy::Ptr)
    }

    /// The descriptor of an array expression, without reading its elements when it names an
    /// object.
    pub(crate) fn dope_of(&mut self, e: ExprId) -> R<Reg> {
        // A part of a composite signal (`p.data`, `b.resp.user`): its bounds come from the
        // signal's descriptor, so a constant initialiser needs no signal value (#304).
        if let Some((d, steps)) = self.comp_steps(e) {
            return Ok(self.comp_walk(d, &steps, self.db.exprs[e].span)?.2.desc);
        }
        if let ExprKind::Obj(d) = self.db.exprs[e].kind {
            match self.cx.objs.get(&d).copied() {
                Some(Obj::Array { space, off }) => {
                    let a = self.addr(space, off);
                    return Ok(self.array_dope(d, a));
                }
                Some(Obj::InstArray { space, off }) => {
                    return Ok(self.inst_array(Some(d), space, off).1);
                }
                Some(Obj::Fixed { space, off, size }) => return Ok(self.addr(space, off + size)),
                Some(Obj::Signal { .. }) => {
                    let span = self.db.exprs[e].span;
                    if let Some(d) = self.sig_dope(d, span)? {
                        return Ok(d);
                    }
                }
                _ => {}
            }
        }
        let v = self.expr(e)?;
        Ok(self.arr(v, self.db.exprs[e].span)?.1)
    }

    // ---- ranges and descriptors ----

    pub fn range(&mut self, rc: &RangeC, span: Span) -> R<Rng> {
        match rc {
            // A range of a declared type or subtype: the bounds its declaration evaluated. An
            // elaboration-time call has no instance data of the architecture; it evaluates them.
            // ponytail: there per use, once per call would need them as captured objects.
            RangeC::Explicit { left, dir, .. }
                if self.cx.decl_ranges.get(left).is_some_and(|&(sp, _)| {
                    !(self.cx.capture_free && matches!(sp, Space::Inst))
                }) =>
            {
                let (space, off) = self.cx.decl_ranges[left];
                let t = self.rt_of(self.db.base(self.db.exprs[*left].subtype));
                let a = self.addr(space, off);
                let l = self.load(a, Width::B8, t);
                let b = self.offset(a, 8);
                let r = self.load(b, Width::B8, t);
                Ok(Rng {
                    left: l,
                    right: r,
                    dir: Dr::Static(*dir),
                })
            }
            RangeC::Explicit { left, dir, right } => {
                let left = self.scalar_expr(*left)?;
                let right = self.scalar_expr(*right)?;
                Ok(Rng {
                    left,
                    right,
                    dir: Dr::Static(*dir),
                })
            }
            RangeC::Attr {
                prefix,
                reverse,
                dim,
            } => {
                // `dim` is 1-based in the HIR.
                let r = match prefix {
                    AttrPrefix::Subtype(st) if !ty::is_array(self.db, self.db.base(*st)) => {
                        self.scalar_bounds(*st, span)?
                    }
                    AttrPrefix::Subtype(st) => {
                        let Some(dope) = self.subtype_dope(*st, span)? else {
                            return Err(self.cx.ice(span, "range of an unconstrained array type"));
                        };
                        self.dope_range(dope, dim.saturating_sub(1) as u8, RegTy::I64)
                    }
                    AttrPrefix::Expr(e) => {
                        let dope = self.dope_of(*e)?;
                        // A scalar named return value's range (`Db::scalar_range`): the
                        // bounds of a floating point type are its values' bits.
                        let t = self.rt_of(self.db.base(self.db.exprs[*e].subtype));
                        self.dope_range(dope, dim.saturating_sub(1) as u8, t)
                    }
                    AttrPrefix::Entity(_) => {
                        return Err(self.cx.ice(span, "range of a named entity"));
                    }
                };
                if !*reverse {
                    return Ok(r);
                }
                let dir = match r.dir {
                    Dr::Static(Direction::To) => Dr::Static(Direction::Downto),
                    Dr::Static(Direction::Downto) => Dr::Static(Direction::To),
                    Dr::Dyn(d) => {
                        let one = self.int(1);
                        let dst = self.reg(RegTy::I64);
                        self.emit(Inst::Bin {
                            op: IntOp::Sub,
                            dst,
                            a: one,
                            b: d,
                        });
                        Dr::Dyn(dst)
                    }
                };
                Ok(Rng {
                    left: r.right,
                    right: r.left,
                    dir,
                })
            }
        }
    }

    /// Bounds `dim` of descriptor `dope`, as registers of type `t`.
    fn dope_range(&mut self, dope: Reg, dim: u8, t: RegTy) -> Rng {
        let base = self.offset(dope, 24 * u32::from(dim));
        let left = self.load(base, Width::B8, t);
        let r = self.offset(base, 8);
        let right = self.load(r, Width::B8, t);
        let d = self.offset(base, 16);
        let dir = self.load(d, Width::B8, RegTy::I64);
        Rng {
            left,
            right,
            dir: Dr::Dyn(dir),
        }
    }

    pub fn dir_reg(&mut self, d: Dr) -> Reg {
        match d {
            Dr::Static(d) => self.int(dir_cell(d)),
            Dr::Dyn(r) => r,
        }
    }

    /// `(low, high)` of a range.
    pub fn low_high(&mut self, r: Rng) -> (Reg, Reg) {
        match r.dir {
            Dr::Static(Direction::To) => (r.left, r.right),
            Dr::Static(Direction::Downto) => (r.right, r.left),
            Dr::Dyn(d) => {
                let z = self.int(0);
                let up = self.cmp(CmpOp::Eq, d, z);
                let lo = self.select(up, r.left, r.right, RegTy::I64);
                let hi = self.select(up, r.right, r.left, RegTy::I64);
                (lo, hi)
            }
        }
    }

    /// The bounds of a scalar subtype.
    pub fn scalar_bounds(&mut self, st: SubtypeId, span: Span) -> R<Rng> {
        let db = self.db;
        if let Some(rc) = db.scalar_range(st) {
            return self.range(&rc, span);
        }
        match &db.types[db.base(st)].kind {
            TypeKind::Enum { lits } => {
                let left = self.int(0);
                let right = self.int(lits.len() as i64 - 1);
                Ok(Rng {
                    left,
                    right,
                    dir: Dr::Static(Direction::To),
                })
            }
            TypeKind::UniversalInteger => {
                let left = self.int(i64::MIN);
                let right = self.int(i64::MAX);
                Ok(Rng {
                    left,
                    right,
                    dir: Dr::Static(Direction::To),
                })
            }
            _ => Err(self.cx.ice(span, "scalar subtype without a range")),
        }
    }

    pub fn scalar_left(&mut self, st: SubtypeId, span: Span) -> R<Reg> {
        if ty::is_real(self.db, self.db.base(st))
            && let Some((l, _, _)) = self.db.static_bounds(st)
            && let ConstValue::Real(f) = l
        {
            let dst = self.reg(RegTy::F64);
            self.emit(Inst::ConstF { dst, v: f });
            return Ok(dst);
        }
        Ok(self.scalar_bounds(st, span)?.left)
    }

    /// The descriptor of an array subtype with index constraints; `None` when it has none.
    pub fn subtype_dope(&mut self, st: SubtypeId, span: Span) -> R<Option<Reg>> {
        self.subtype_dope_in(st, span, None)
    }

    /// [`Fx::subtype_dope`]; a descriptor of run-time bounds is written at `into` (24 bytes per
    /// dimension) instead of a temporary when given, and the result is then `into`.
    pub fn subtype_dope_in(
        &mut self,
        st: SubtypeId,
        span: Span,
        into: Option<Reg>,
    ) -> R<Option<Reg>> {
        let db = self.db;
        // `x'subtype` of an unconstrained object, a named return value's subtype: the
        // object's own descriptor.
        if let Constraint::FromObject {
            expr,
            elem_depth: 0,
        } = db.subtypes[st].constraint
        {
            return self.dope_of(expr).map(Some);
        }
        // `x'element...` of an object whose element bounds are open: the part of the object's
        // descriptor after the dimensions of the `elem_depth` outer levels (rule 1).
        if let Constraint::FromObject { expr, elem_depth } = db.subtypes[st].constraint
            && elem_depth > 0
            && !matches!(
                db.subtypes[st].parent.map(|p| &db.subtypes[p].constraint),
                Some(Constraint::Array { dims: Some(_), .. })
            )
        {
            let mut tid = self.ty(db.base(db.exprs[expr].subtype), span)?;
            let mut off = 0;
            for _ in 0..elem_depth {
                let Some(Ty::Array { elem, dims, .. }) =
                    self.fb.program().types().get(tid).cloned()
                else {
                    return Err(self
                        .cx
                        .ice(span, "'element of a non-array descriptor level"));
                };
                off += 24 * u32::from(dims);
                tid = elem;
            }
            let dope = self.dope_of(expr)?;
            return Ok(Some(self.offset(dope, off)));
        }
        if self.is_general(db.base(st), span)? {
            return self.desc_of(st, None, span);
        }
        let Constraint::Array {
            dims: Some(dims), ..
        } = &db.subtypes[st].constraint
        else {
            return Ok(None);
        };
        // Static bounds: a pool constant (ir.md 1.3), the same descriptor format.
        let mut stat = Vec::new();
        for rc in dims.iter() {
            if let RangeC::Explicit { left, dir, right } = rc
                && let (Some(l), Some(r)) = (
                    db.value(*left).and_then(ConstValue::as_int),
                    db.value(*right).and_then(ConstValue::as_int),
                )
            {
                for x in [l, r, dir_cell(*dir)] {
                    stat.extend_from_slice(&x.to_le_bytes());
                }
            } else {
                stat.clear();
                break;
            }
        }
        if !stat.is_empty() {
            // A static index range outside the index subtype fails when it is elaborated.
            self.check_index_ranges(st, dims, span)?;
            let c = self.fb.program().konst(ConstData::Bytes(stat));
            let dst = self.reg(RegTy::Ptr);
            self.emit(Inst::AddrOf {
                dst,
                space: yavs_ir::AddrSpace::Const(c),
                off: 0,
            });
            return Ok(Some(dst));
        }
        let dims = dims.clone();
        let mut parts = Vec::new();
        let indexes = index_subtypes(db, db.base(st));
        for (i, rc) in dims.iter().enumerate() {
            let r = self.range(rc, span)?;
            self.check_index_range(indexes.as_deref(), i, rc, r, span)?;
            let d = self.dir_reg(r.dir);
            parts.extend([r.left, r.right, d]);
        }
        let at = match into {
            Some(at) => at,
            None => self.alloc_n(24 * dims.len() as u32),
        };
        let parts = self.fb.regs(&parts);
        self.emit(Inst::MakeDope { at, parts });
        Ok(Some(at))
    }

    // ---- aggregates ----

    /// A one-dimensional array aggregate of scalar elements (LRM 9.3.3.3).
    fn aggregate(
        &mut self,
        assocs: &[AggAssoc],
        bounds: &AggBounds,
        st: SubtypeId,
        span: Span,
    ) -> R<V> {
        let db = self.db;
        let t = db.base(st);
        let TypeKind::Array { indexes, elem } = &db.types[t].kind else {
            return self.aggregate_comp(assocs, bounds, st, span);
        };
        if indexes.len() != 1 || self.is_general(t, span)? {
            return self.aggregate_comp(assocs, bounds, st, span);
        }
        if assocs.iter().any(|a| a.slice_piece) {
            return self.aggregate_pieces(assocs, bounds, st, span);
        }
        let (index, elem) = (indexes[0], *elem);
        let dope = match bounds {
            AggBounds::Context => match self.subtype_dope(st, span)? {
                Some(d) => {
                    let len = self.dope_len(d);
                    self.agg_count_check(assocs, len);
                    d
                }
                None => return Err(self.cx.ice(span, "aggregate context without bounds")),
            },
            AggBounds::Positional { count } => {
                let s = self.scalar_bounds(index, span)?;
                let n = self.scalar_expr(*count)?;
                let one = self.int(1);
                let last = self.bin(IntOp::Sub, n, one);
                let right = match s.dir {
                    Dr::Static(Direction::To) => self.bin(IntOp::Add, s.left, last),
                    Dr::Static(Direction::Downto) => self.bin(IntOp::Sub, s.left, last),
                    Dr::Dyn(_) => {
                        return Err(self.cx.ice(span, "index subtype with a runtime direction"));
                    }
                };
                let dir = self.dir_reg(s.dir);
                self.one_dope(s.left, right, dir)
            }
            // Sem's result subtype: the direction of a constrained context (LRM 9.3.3.3).
            AggBounds::Named { .. } if let Some(d) = self.subtype_dope(st, span)? => d,
            AggBounds::Named { low, high } => {
                let s = self.scalar_bounds(index, span)?;
                let lo = self.scalar_expr(*low)?;
                let hi = self.scalar_expr(*high)?;
                match s.dir {
                    Dr::Static(Direction::To) => {
                        let d = self.int(0);
                        self.one_dope(lo, hi, d)
                    }
                    Dr::Static(Direction::Downto) => {
                        let d = self.int(1);
                        self.one_dope(hi, lo, d)
                    }
                    Dr::Dyn(_) => {
                        return Err(self.cx.ice(span, "index subtype with a runtime direction"));
                    }
                }
            }
            _ => return Err(self.cx.unsupported(span, "this aggregate form")),
        };
        let (tid, logic, w) = self.elem_facts(t, span)?;
        let bias = self.code_bias(tid);
        let (size, count) = self.layout(tid, dope);
        let data = self.alloc_val(size, tid, Some(dope));
        let set = |fx: &mut Self, pos: Reg, v: Reg| {
            if logic {
                let v = fx.enc(v, bias);
                let one = fx.int(1);
                let last = fx.bin(IntOp::Sub, count, one);
                let idx = fx.bin(IntOp::Sub, last, pos);
                fx.emit(Inst::LSet { base: data, idx, v });
            } else {
                let a = fx.reg(RegTy::Ptr);
                fx.emit(Inst::Index {
                    dst: a,
                    base: data,
                    pos,
                    stride: w.bytes(),
                });
                fx.store(a, v, w);
            }
        };
        let elem_value = |fx: &mut Self, e: ExprId| -> R<Reg> {
            let v = fx.scalar_expr(e)?;
            fx.check_scalar(v, Some(fx.db.exprs[e].subtype), elem, "", span)?;
            Ok(v)
        };
        let m = self.msg("aggregate", Dir::To);
        // `others` first: the named and positional elements then overwrite their positions.
        for a in assocs
            .iter()
            .filter(|a| a.choices.iter().any(|c| matches!(c, Choice::Others)))
        {
            let v = elem_value(self, a.value)?;
            if logic {
                let code = self.enc(v, bias);
                let zero = self.int(0);
                self.emit(Inst::LFill {
                    base: data,
                    idx: zero,
                    n: count,
                    v: code,
                });
            } else {
                let zero = self.int(0);
                self.fill_scalars(data, zero, count, v, w);
            }
        }
        let mut k = 0i64;
        for a in assocs.iter() {
            if a.choices.iter().any(|c| matches!(c, Choice::Others)) {
                continue;
            }
            let v = elem_value(self, a.value)?;
            if a.choices.is_empty() {
                let pos = self.int(k);
                k += 1;
                set(self, pos, v);
                continue;
            }
            for c in a.choices.iter() {
                match c {
                    Choice::Expr(c) => {
                        let i = self.scalar_expr(*c)?;
                        let pos = self.reg(RegTy::I64);
                        self.emit(Inst::Pos {
                            dst: pos,
                            dope,
                            dim: 0,
                            idx: i,
                            what: m,
                        });
                        set(self, pos, v);
                    }
                    Choice::Range(rc) => {
                        // A few elements (`(0 to 0 => cin)`): straight code, not a loop.
                        if let Some(r) = yavs_sem::expr::aggregate::range_value(db, rc)
                            && let Some(n) = match r.dir {
                                Direction::To => r.right.checked_sub(r.left),
                                Direction::Downto => r.left.checked_sub(r.right),
                            }
                            .and_then(|d| d.checked_add(1))
                            && n <= AGG_UNROLL
                        {
                            for i in 0..n.max(0) {
                                let idx = self.int(match r.dir {
                                    Direction::To => r.left + i,
                                    Direction::Downto => r.left - i,
                                });
                                let pos = self.reg(RegTy::I64);
                                self.emit(Inst::Pos {
                                    dst: pos,
                                    dope,
                                    dim: 0,
                                    idx,
                                    what: m,
                                });
                                set(self, pos, v);
                            }
                            continue;
                        }
                        let r = self.range(rc, span)?;
                        let d = self.dir_reg(r.dir);
                        let rd = self.one_dope(r.left, r.right, d);
                        let n = self.dope_len(rd);
                        let z = self.int(0);
                        let up = self.cmp(CmpOp::Eq, d, z);
                        let one = self.int(1);
                        let m1 = self.int(-1);
                        let step = self.select(up, one, m1, RegTy::I64);
                        let left = r.left;
                        let done = self.block();
                        if logic {
                            let slow = self.block();
                            self.jump(slow);
                            self.at(slow);
                        } else {
                            let (right, d) = (r.right, done);
                            self.fill_run_or_fall(data, dope, left, right, n, w, m, d, |_| Ok(v))?;
                        }
                        self.for_n(n, |fx, i| {
                            let off = fx.bin(IntOp::Mul, i, step);
                            let idx = fx.bin(IntOp::Add, left, off);
                            let pos = fx.reg(RegTy::I64);
                            fx.emit(Inst::Pos {
                                dst: pos,
                                dope,
                                dim: 0,
                                idx,
                                what: m,
                            });
                            set(fx, pos, v);
                            Ok(())
                        })?;
                        self.jump(done);
                        self.at(done);
                    }
                    _ => return Err(self.cx.ice(span, "element choice in an array aggregate")),
                }
            }
        }
        Ok(V::A(data, dope))
    }

    /// The positional elements of an aggregate level against the length `len` of the context
    /// subtype (LRM 9.3.3.3): without `others` the counts must be equal, with it the
    /// positional elements must fit. A mismatch is a run-time length error, not a write
    /// outside the aggregate's object.
    pub(crate) fn agg_count_check(&mut self, assocs: &[AggAssoc], len: Reg) {
        let n = assocs.iter().filter(|a| a.choices.is_empty()).count() as i64;
        if n == 0 {
            return;
        }
        let others = assocs
            .iter()
            .any(|a| a.choices.iter().any(|c| matches!(c, Choice::Others)));
        let k = self.int(n);
        let v = if others {
            let over = self.cmp(CmpOp::Gt, k, len);
            self.select(over, k, len, RegTy::I64)
        } else {
            k
        };
        let what = self.msg("aggregate", Dir::To);
        self.emit(Inst::Check {
            kind: CheckKind::Length,
            v,
            lo: len,
            hi: len,
            what,
        });
    }

    /// `n` copies of scalar `v` (width `w`) at element `start` of the array at `data`.
    pub(crate) fn fill_scalars(&mut self, data: Reg, start: Reg, n: Reg, v: Reg, w: Width) {
        let cell = self.alloc_n(8);
        self.store(cell, v, w);
        let at = self.reg(RegTy::Ptr);
        self.emit(Inst::Index {
            dst: at,
            base: data,
            pos: start,
            stride: w.bytes(),
        });
        self.emit(Inst::Fill {
            dst: at,
            elem: cell,
            n,
            size: w.bytes(),
        });
    }

    /// A run of `n` elements from index `left` to `right` that lies inside the array at
    /// `data` (descriptor `dope`) is written as one fill of the scalar `value` gives (width
    /// `w`), then control goes to `done`. An empty run or one partly outside falls through
    /// (the current block afterwards) to the caller's loop and its index checks.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn fill_run_or_fall(
        &mut self,
        data: Reg,
        dope: Reg,
        left: Reg,
        right: Reg,
        n: Reg,
        w: Width,
        what: yavs_ir::CheckMsgId,
        done: yavs_ir::BlockId,
        value: impl FnOnce(&mut Self) -> R<Reg>,
    ) -> R<()> {
        let (chk, inside, slow) = (self.block(), self.block(), self.block());
        let z = self.int(0);
        let any = self.cmp(CmpOp::Gt, n, z);
        self.branch(any, chk, slow);
        self.at(chk);
        let (l0, r0, d0) = self.dope_parts(dope);
        let up = self.cmp(CmpOp::Eq, d0, z);
        let lo = self.select(up, l0, r0, RegTy::I64);
        let hi = self.select(up, r0, l0, RegTy::I64);
        let mut ok = self.cmp(CmpOp::Le, lo, left);
        for (a, b) in [(left, hi), (lo, right), (right, hi)] {
            let c = self.cmp(CmpOp::Le, a, b);
            ok = self.bool_and(ok, c);
        }
        self.branch(ok, inside, slow);
        self.at(inside);
        let v = value(self)?;
        let [pl, pr] = [left, right].map(|idx| {
            let pos = self.reg(RegTy::I64);
            self.emit(Inst::Pos {
                dst: pos,
                dope,
                dim: 0,
                idx,
                what,
            });
            pos
        });
        let first = self.cmp(CmpOp::Le, pl, pr);
        let start = self.select(first, pl, pr, RegTy::I64);
        self.fill_scalars(data, start, n, v, w);
        self.jump(done);
        self.at(slow);
        Ok(())
    }

    /// Left, right and direction cells of the descriptor part at `dope`.
    fn dope_parts(&mut self, dope: Reg) -> (Reg, Reg, Reg) {
        let l = self.load(dope, Width::B8, RegTy::I64);
        let ra = self.offset(dope, 8);
        let r = self.load(ra, Width::B8, RegTy::I64);
        let da = self.offset(dope, 16);
        let d = self.load(da, Width::B8, RegTy::I64);
        (l, r, d)
    }

    fn bool_and(&mut self, a: Reg, b: Reg) -> Reg {
        let dst = self.reg(RegTy::I64);
        self.emit(Inst::Bool {
            op: yavs_ir::BoolOp::And,
            dst,
            a,
            b,
        });
        dst
    }

    /// `body(i)` for `i` in `0 .. n`.
    pub fn for_n(&mut self, n: Reg, mut body: impl FnMut(&mut Self, Reg) -> R<()>) -> R<()> {
        let i = self.reg(RegTy::I64);
        self.emit(Inst::Const { dst: i, v: 0 });
        let (head, b, exit) = (self.block(), self.block(), self.block());
        self.jump(head);
        self.at(head);
        let more = self.cmp(CmpOp::Lt, i, n);
        self.branch(more, b, exit);
        self.at(b);
        body(self, i)?;
        let one = self.int(1);
        let next = self.bin(IntOp::Add, i, one);
        self.emit(Inst::Move { dst: i, src: next });
        self.jump(head);
        self.at(exit);
        Ok(())
    }

    pub(crate) fn one_dope(&mut self, left: Reg, right: Reg, dir: Reg) -> Reg {
        let at = self.alloc_n(24);
        let parts = self.fb.regs(&[left, right, dir]);
        self.emit(Inst::MakeDope { at, parts });
        at
    }

    // ---- small helpers ----

    pub fn cmp(&mut self, op: CmpOp, a: Reg, b: Reg) -> Reg {
        let dst = self.reg(RegTy::I64);
        self.emit(Inst::Cmp { op, dst, a, b });
        dst
    }

    pub fn bin(&mut self, op: IntOp, a: Reg, b: Reg) -> Reg {
        let dst = self.reg(RegTy::I64);
        self.emit(Inst::Bin { op, dst, a, b });
        dst
    }

    pub fn not(&mut self, a: Reg) -> Reg {
        let dst = self.reg(RegTy::I64);
        self.emit(Inst::Bool {
            op: BoolOp::Not,
            dst,
            a,
            b: a,
        });
        dst
    }

    /// `c ? a : b`; both are already computed.
    pub fn select(&mut self, c: Reg, a: Reg, b: Reg, t: RegTy) -> Reg {
        let dst = self.reg(t);
        self.emit(Inst::Select { dst, c, a, b });
        dst
    }

    /// Range check of `v` (from subtype `from`, if known) against scalar subtype `to`.
    pub fn check_scalar(
        &mut self,
        v: Reg,
        from: Option<SubtypeId>,
        to: SubtypeId,
        what: &str,
        span: Span,
    ) -> R<()> {
        let db = self.db;
        let t = db.base(to);
        if from == Some(to) {
            return Ok(());
        }
        if !matches!(
            db.types[t].kind,
            TypeKind::Enum { .. }
                | TypeKind::Integer
                | TypeKind::Physical { .. }
                | TypeKind::Floating
        ) {
            return Ok(());
        }
        // A value already known to lie within `to` needs no check.
        if let (Some(f), Some((tl, td, tr))) = (from, db.static_bounds(to))
            && let Some((fl, fd, fr)) = db.static_bounds(f)
            && let (Some(fl), Some(fr), Some(tl), Some(tr)) =
                (fl.as_int(), fr.as_int(), tl.as_int(), tr.as_int())
        {
            let (flo, fhi) = if fd == Direction::To {
                (fl, fr)
            } else {
                (fr, fl)
            };
            let (tlo, thi) = if td == Direction::To {
                (tl, tr)
            } else {
                (tr, tl)
            };
            if flo > fhi || (tlo <= flo && fhi <= thi) {
                return Ok(());
            }
        }
        let r = self.scalar_bounds(to, span)?;
        self.check_in(v, r, to, what);
        Ok(())
    }

    /// Check scalar `v` against range `r` of subtype `to` (named in the message).
    pub(crate) fn check_in(&mut self, v: Reg, r: Rng, to: SubtypeId, what: &str) {
        let db = self.db;
        let t = db.base(to);
        let dir = ir_dir(r.dir);
        let real = ty::is_real(db, t);
        let (lo, hi) = if real {
            self.low_high_f(r)
        } else {
            self.low_high(r)
        };
        let st = ty::subtype_name(db, to);
        let what = self.fb.program().check_msg_for(what, dir, &st);
        if real {
            self.emit(Inst::FCheck { v, lo, hi, what });
        } else {
            self.emit(Inst::Check {
                kind: CheckKind::Range,
                v,
                lo,
                hi,
                what,
            });
        }
    }

    /// Both bounds of range `r` belong to scalar subtype `to`, unless `r` is null (LRM 5.2.1,
    /// 5.3.2.2, 9.3.6); a violation is a run-time error.
    pub(crate) fn check_range_in(
        &mut self,
        r: Rng,
        to: SubtypeId,
        what: &str,
        span: Span,
    ) -> R<()> {
        let pr = self.scalar_bounds(to, span)?;
        let (lo, hi) = self.low_high(r);
        let nonnull = self.cmp(CmpOp::Le, lo, hi);
        let (t, j) = (self.block(), self.block());
        self.branch(nonnull, t, j);
        self.at(t);
        self.check_in(lo, pr, to, what);
        self.check_in(hi, pr, to, what);
        self.jump(j);
        self.at(j);
        Ok(())
    }

    /// LRM 5.3.2.2: index range `r` (of `rc`, dimension `k`) lies inside its index subtype
    /// among `indexes`, whatever form the range has (`string(x'range)`, #637); nothing is
    /// emitted for a range known to be inside.
    pub(crate) fn check_index_range(
        &mut self,
        indexes: Option<&[SubtypeId]>,
        k: usize,
        rc: &RangeC,
        r: Rng,
        span: Span,
    ) -> R<()> {
        match indexes.and_then(|v| v.get(k)) {
            Some(&ix) if !ty::is_real(self.db, self.db.base(ix)) && !self.range_within(rc, ix) => {
                self.check_range_in(r, ix, "index constraint", span)
            }
            _ => Ok(()),
        }
    }

    /// The static index ranges `dims` of `st` that are not inside their index subtypes fail
    /// when the subtype is elaborated (LRM 5.3.2.2); nothing is emitted for the others.
    fn check_index_ranges(&mut self, st: SubtypeId, dims: &[RangeC], span: Span) -> R<()> {
        let db = self.db;
        let Some(indexes) = index_subtypes(db, db.base(st)) else {
            return Ok(());
        };
        for (rc, &ix) in dims.iter().zip(indexes.iter()) {
            if matches!(rc, RangeC::Explicit { .. })
                && !ty::is_real(db, db.base(ix))
                && !self.range_within(rc, ix)
            {
                let r = self.range(rc, span)?;
                self.check_range_in(r, ix, "index constraint", span)?;
            }
        }
        Ok(())
    }

    /// Whether the explicit range `rc` is null or lies inside `to`, both known at compile time.
    fn range_within(&self, rc: &RangeC, to: SubtypeId) -> bool {
        let db = self.db;
        let (RangeC::Explicit { left, dir, right }, Some((tl, td, tr))) =
            (rc, db.static_bounds(to))
        else {
            return false;
        };
        let (Some(l), Some(r), Some(tl), Some(tr)) = (
            db.value(*left).and_then(ConstValue::as_int),
            db.value(*right).and_then(ConstValue::as_int),
            tl.as_int(),
            tr.as_int(),
        ) else {
            return false;
        };
        let lh = |l, d, r| if d == Direction::To { (l, r) } else { (r, l) };
        let ((lo, hi), (tlo, thi)) = (lh(l, *dir, r), lh(tl, td, tr));
        lo > hi || (tlo <= lo && hi <= thi)
    }

    /// The range constraints of subtype `st` and of the subtypes it is built on must lie inside
    /// their type mark's range (LRM 5.2.1, 6.4.2.3). Evaluated where an object is declared;
    /// nothing is emitted for ranges known to be inside. The index constraints of arrays are
    /// checked where their descriptor is made (`subtype_dope`).
    pub fn check_subtype_compat(&mut self, st: SubtypeId, span: Span) -> R<()> {
        let db = self.db;
        let mut cur = Some(st);
        while let Some(s) = cur {
            let sub = &db.subtypes[s];
            match &sub.constraint {
                Constraint::Range(rc @ RangeC::Explicit { .. })
                    if let Some(p) = sub.parent
                        && !ty::is_real(db, db.base(s))
                        && db.types[db.base(s)].kind.is_scalar()
                        && !self.range_within(rc, p) =>
                {
                    let r = self.range(rc, span)?;
                    self.check_range_in(r, p, "range constraint", span)?;
                }
                _ => {}
            }
            cur = sub.parent;
        }
        Ok(())
    }

    /// `(low, high)` of a real range (registers are F64).
    fn low_high_f(&mut self, r: Rng) -> (Reg, Reg) {
        match r.dir {
            Dr::Static(Direction::To) => (r.left, r.right),
            Dr::Static(Direction::Downto) => (r.right, r.left),
            Dr::Dyn(d) => {
                let z = self.int(0);
                let up = self.cmp(CmpOp::Eq, d, z);
                let lo = self.select(up, r.left, r.right, RegTy::F64);
                let hi = self.select(up, r.right, r.left, RegTy::F64);
                (lo, hi)
            }
        }
    }

    /// Integer or physical arithmetic checked against the base range of the result type
    /// (rule 7, U-1); universal results are only checked for i64 overflow.
    fn bin_checked(
        &mut self,
        op: IntOp,
        a: Reg,
        b: Reg,
        res: Option<SubtypeId>,
        span: Span,
    ) -> R<Reg> {
        let db = self.db;
        let Some(st) = res.filter(|&st| {
            matches!(
                db.types[db.base(st)].kind,
                TypeKind::Integer | TypeKind::Physical { .. }
            )
        }) else {
            return Ok(self.bin(op, a, b));
        };
        let t = db.base(st);
        let bs = db.types[t].base_subtype;
        let r = self.scalar_bounds(bs, span)?;
        let (lo, hi) = self.low_high(r);
        let name = db.types[t].name.text.as_str().to_ascii_uppercase();
        let what = self.fb.program().check_msg_for("", Dir::To, &name);
        let dst = self.reg(RegTy::I64);
        self.emit(Inst::BinRange {
            op,
            dst,
            a,
            b,
            lo,
            hi,
            what,
        });
        Ok(dst)
    }

    // ---- calls ----

    fn actual(&mut self, a: &Arg, p: &Param, span: Span) -> R<ExprId> {
        match &a.actual {
            Actual::Expr(e) => Ok(*e),
            // `open` for a parameter with a default means the default (LRM 6.5.6.2).
            Actual::Default | Actual::Open if p.default.is_some() => p
                .default
                .ok_or_else(|| self.cx.ice(span, "parameter default")),
            Actual::Default => Err(self.cx.ice(span, "default actual without a default")),
            Actual::Open => Err(self
                .cx
                .unsupported(span, "open actuals of parameters without a default")),
            _ => Err(self
                .cx
                .unsupported(span, "individual and converted associations")),
        }
    }

    /// A call; `None` for a procedure. `res` is a function call's result subtype.
    pub fn call(
        &mut self,
        callee: DeclId,
        args: &[Arg],
        method: Option<ExprId>,
        res: Option<SubtypeId>,
        span: Span,
    ) -> R<Option<V>> {
        let db = self.db;
        let decl = &db.decls[callee];
        let Some(sp) = decl.kind.subprogram() else {
            return Err(self.cx.ice(span, "call of a non-subprogram"));
        };
        if let (Some(o), Body::Native(sym)) = (method, &sp.body) {
            return self.native_method(sp, sym.as_str(), o, args, span);
        }
        if method.is_some() || crate::protected::method_type(db, callee).is_some() {
            return self.method_call(callee, args, method, res, span);
        }
        match &sp.body {
            Body::Implicit(op) => self.implicit(op, sp, args, span),
            Body::Native(sym) => {
                // READ of a string takes its target as `data, left, right, dir`: no fat pair
                // per call (#771).
                let conv = if sym.as_str().starts_with("yavs.textio.read.line_string") {
                    Conv::Flat
                } else {
                    Conv::Fat
                };
                let cells = self.cells(sp, args, span, conv)?;
                let r = self.native(sym.as_str(), None, &cells, sp.ret, span)?;
                self.copy_out(span)?;
                Ok(r)
            }
            // A hot `ieee` subprogram runs natively unless `--no-native-ieee` (yavs-ir `ieee`).
            Body::Hir { .. }
                if !self.cx.opts.no_native_ieee
                    && yavs_ir::ieee::native_symbol(sp.mangled.as_str()).is_some() =>
            {
                let mut sym = yavs_ir::ieee::native_symbol(sp.mangled.as_str())
                    .ok_or_else(|| self.cx.ice(span, "ieee native symbol"))?;
                // A native that may give the call back names the body's function.
                if yavs_ir::ieee::symbol_key(&sym)
                    .and_then(yavs_ir::ieee::parse)
                    .is_some_and(yavs_ir::ieee::has_fallback)
                {
                    let f = func_of(db, self.fb.program(), self.cx, callee, span)?;
                    sym = format!("{sym}@{}", f.0);
                }
                // A `numeric_bit` native gets a type argument: it tells the bit arrays from
                // the `std_ulogic` ones (yavs-interp `ieee`).
                let ty = match (sym.starts_with("yavs.ieee.numeric_bit."), sp.params.first()) {
                    (true, Some(p)) => Some(self.ty(db.base(p.subtype), span)?),
                    _ => None,
                };
                let cells = self.cells(sp, args, span, Conv::Flat)?;
                let r = self.native(&sym, ty, &cells, sp.ret, span)?;
                self.copy_out(span)?;
                Ok(r)
            }
            // A user function that folds a logic vector with one gate runs as the native of the
            // unary operator (#531).
            Body::Hir { .. }
                if !self.cx.opts.no_native_ieee
                    && !self.cx.opts.any_coverage()
                    && crate::reduce::reduction(db, callee).is_some() =>
            {
                let gate = crate::reduce::reduction(db, callee)
                    .ok_or_else(|| self.cx.ice(span, "reduction idiom"))?;
                let cells = self.cells(sp, args, span, Conv::Flat)?;
                let r = self.native(&crate::reduce::symbol(gate), None, &cells, sp.ret, span)?;
                self.copy_out(span)?;
                Ok(r)
            }
            Body::Hir { scope, .. } => {
                let f = func_of(db, self.fb.program(), self.cx, callee, span)?;
                let mut cells = self.cells(sp, args, span, Conv::Body)?;
                self.named_return_cells(callee, *scope, res, span, &mut cells)?;
                let args = self.fb.regs(&cells);
                let Some(ret) = sp.ret else {
                    self.emit(Inst::Call {
                        func: f,
                        args,
                        dst: None,
                    });
                    self.copy_out(span)?;
                    return Ok(None);
                };
                let rt = if self.is_comp(db.base(ret)) {
                    RegTy::Ptr
                } else {
                    self.rt_of(db.base(ret))
                };
                let dst = self.reg(rt);
                self.emit(Inst::Call {
                    func: f,
                    args,
                    dst: Some(dst),
                });
                if rt == RegTy::Ptr {
                    return Ok(Some(self.fat_value(dst)));
                }
                Ok(Some(V::S(dst)))
            }
            Body::Pending => Err(self.cx.unsupported(
                span,
                &format!("call of `{}`, whose body is not analyzed", decl.name),
            )),
            Body::Foreign(attr) => self.foreign_call(callee, sp, *attr, args, span),
            _ => Err(self
                .cx
                .unsupported(span, "generic and instantiated subprograms (M3)")),
        }
    }

    /// A named return value (2019, 4.2.1): the result subtype `res`'s descriptor follows the
    /// arguments of a call of `callee` (parameter region `scope`), as an array parameter (the
    /// data cell is unused).
    pub(crate) fn named_return_cells(
        &mut self,
        callee: DeclId,
        scope: yavs_sem::ScopeId,
        res: Option<SubtypeId>,
        span: Span,
        cells: &mut Vec<Reg>,
    ) -> R<()> {
        let db = self.db;
        if !db.ret_names.contains_key(&scope) {
            return Ok(());
        }
        let decl = &db.decls[callee];
        let st = res
            .or(decl.kind.subprogram().and_then(|s| s.ret))
            .unwrap_or(db.poison.subtype);
        let dope = if db.types[db.base(st)].kind.is_scalar() {
            // A scalar: its range as a one-dimensional descriptor (`Db::scalar_range`).
            let r = self.scalar_bounds(st, span)?;
            let dope = self.alloc_n(24);
            let dir = self.dir_reg(r.dir);
            for (i, v) in [r.left, r.right, dir].into_iter().enumerate() {
                let a = self.offset(dope, 8 * i as u32);
                self.store(a, v, Width::B8);
            }
            dope
        } else {
            let Some(dope) = self.subtype_dope(st, span)? else {
                return Err(self.cx.unsupported(
                    span,
                    &format!(
                        "a call of `{}` (a named return value) whose context gives no \
                         constrained result subtype",
                        decl.name
                    ),
                ));
            };
            dope
        };
        cells.extend([dope, dope]);
        Ok(())
    }

    /// `[data, dope]` at `fat`.
    pub(crate) fn fat_value(&mut self, fat: Reg) -> V {
        let data = self.load(fat, Width::B8, RegTy::Ptr);
        let d = self.offset(fat, 8);
        let dope = self.load(d, Width::B8, RegTy::Ptr);
        V::A(data, dope)
    }

    pub(crate) fn make_fat(&mut self, data: Reg, dope: Reg) -> Reg {
        let fat = self.alloc_n(16);
        self.store(fat, data, Width::B8);
        let d = self.offset(fat, 8);
        self.store(d, dope, Width::B8);
        self.fat_root(fat);
        fat
    }

    /// An array argument as the cells of `conv`.
    pub(crate) fn array_cells(&mut self, conv: Conv, data: Reg, dope: Reg, cells: &mut Vec<Reg>) {
        match conv {
            Conv::Body => cells.extend([data, dope]),
            Conv::Fat => cells.push(self.make_fat(data, dope)),
            Conv::Flat => {
                let (l, r, d) = self.dope_bounds(dope);
                cells.extend([data, l, r, d]);
            }
        }
    }

    /// Left, right and direction of the one-dimensional descriptor `dope`: the registers it was
    /// made from in this block, else loaded from it.
    fn dope_bounds(&mut self, dope: Reg) -> (Reg, Reg, Reg) {
        let here = self.fb.current();
        match (self.made_dopes.iter().rev()).find(|(b, at, _)| *b == here && *at == dope) {
            Some(&(_, _, bounds)) => bounds,
            None => self.dope_parts(dope),
        }
    }

    /// Argument cells in formal order (DESIGN.md "Subprograms"); natives take arrays as the
    /// address of a fat pair, or as values for `ieee` natives ([`Conv`]).
    pub(crate) fn cells(
        &mut self,
        sp: &Subprogram,
        args: &[Arg],
        span: Span,
        conv: Conv,
    ) -> R<Vec<Reg>> {
        let db = self.db;
        let mut cells = Vec::new();
        for a in args {
            let Some(p) = sp.params.get(a.formal as usize) else {
                return Err(self.cx.ice(span, "argument for a missing formal"));
            };
            if let Actual::Individual(parts) = &a.actual {
                self.individual_cells(p, parts, conv, span, &mut cells)?;
                continue;
            }
            if let Actual::Converted {
                actual,
                copy_in,
                copy_out: Some(out),
                ..
            } = &a.actual
            {
                self.converted_cells(p, *actual, *copy_in, *out, conv, span, &mut cells)?;
                continue;
            }
            let e = self.actual(a, p, span)?;
            let es = db.exprs[e].span;
            match shape(db, self.cx, p, span)? {
                Shape::Val => {
                    let v = self.scalar_expr(e)?;
                    self.check_scalar(
                        v,
                        Some(db.exprs[e].subtype),
                        p.subtype,
                        &format!("parameter {}", p.name.text.as_str().to_ascii_uppercase()),
                        es,
                    )?;
                    cells.push(v);
                }
                Shape::Ref => match self.place(e)? {
                    Place::Scalar { addr, w, .. } if w != self.scalar(p.subtype, es)?.2 => {
                        // A narrow array element (4 bytes): copy-in/copy-out through a frame
                        // temporary of the formal's width, as for a logic element below.
                        let (_, rt, fw) = self.scalar(p.subtype, es)?;
                        let off = self.fb.slot(8, 8);
                        let t = self.addr(Space::Frame(self.depth), off);
                        let v = self.load(addr, w, rt);
                        self.store(t, v, fw);
                        cells.push(t);
                        self.writeback.push((off, Some(e), p.subtype));
                    }
                    Place::Scalar { addr, .. } => cells.push(addr),
                    Place::Logic { data, idx, st } => {
                        // Copy-in/copy-out through a frame temporary (LRM 6.5.6.2 allows it
                        // for scalars); the actual is written back after the call.
                        let off = self.fb.slot(8, 8);
                        let t = self.addr(Space::Frame(self.depth), off);
                        let code = self.reg(RegTy::I64);
                        self.emit(Inst::LGet {
                            dst: code,
                            base: data,
                            idx,
                        });
                        let tid = self.ty(db.base(st), es)?;
                        let bias = self.code_bias(tid);
                        let v = self.dec(code, bias);
                        let (_, _, w) = self.scalar(p.subtype, es)?;
                        self.store(t, v, w);
                        cells.push(t);
                        self.writeback.push((off, Some(e), p.subtype));
                    }
                    _ => return Err(self.cx.ice(es, "array place for a scalar parameter")),
                },
                Shape::Sig => {
                    let (_, sref, dope) = self.sig_name(e)?;
                    cells.push(sref);
                    let dope = self.sig_formal_dope(p, dope, es)?;
                    cells.extend(dope);
                }
                Shape::SigOut => {
                    let (d, sref, dope) = self.sig_name(e)?;
                    let slot = self.out_driver_slot(d, e)?;
                    cells.extend([sref, slot]);
                    let dope = self.sig_formal_dope(p, dope, es)?;
                    cells.extend(dope);
                }
                Shape::RecSig(out) => {
                    let Some((d, steps)) = self.comp_steps(e) else {
                        return Err(self.cx.ice(es, "record signal actual is not a signal name"));
                    };
                    let part = self.comp_select(d, &steps, es)?;
                    if part.leaves.len() != ty::leaves(db, p.subtype).len() {
                        return Err(self.cx.ice(es, "record signal actual of another type"));
                    }
                    if let yavs_sem::hir::Mode::View { .. } = p.mode {
                        self.view_leaves(p, d, e, &part, es, &mut cells)?;
                    } else {
                        cells.extend(part.leaves.iter().map(|(_, r, _)| *r));
                        if out {
                            let slot = self.leaf_slot(d, e, &part, es)?;
                            cells.push(slot);
                        }
                    }
                    let t = self.ty(db.base(p.subtype), es)?;
                    if Ty::needs_desc(self.fb.program().types(), t) {
                        let mut desc = part.desc;
                        // A constrained array formal takes its own bounds (LRM 4.2.2.3).
                        if ty::array_of(db, db.base(p.subtype)).is_some() {
                            desc =
                                self.param_dope(p.subtype, desc, &p.name.text.to_string(), es)?;
                        }
                        cells.push(desc);
                    }
                }
                Shape::Arr => {
                    let (data, mut dope) = match p.mode {
                        yavs_sem::hir::Mode::In => {
                            let v = self.expr(e)?;
                            self.arr(v, es)?
                        }
                        _ => match self.place(e)? {
                            Place::Array {
                                data,
                                dope,
                                off: None,
                                ..
                            } => (data, dope),
                            Place::Array {
                                data,
                                dope,
                                off: Some(o),
                                tid,
                                ..
                            } => {
                                // A logic slice is not addressable: copy in to a temporary,
                                // write it back after the call (copy-out).
                                let (size, count) = self.layout(tid, dope);
                                let tmp = self.alloc(size);
                                let z = self.int(0);
                                self.emit(Inst::LCopy {
                                    dst: tmp,
                                    dst_off: z,
                                    src: data,
                                    src_off: o,
                                    n: count,
                                });
                                let slot = self.fb.slot(8, 8);
                                let a = self.addr(Space::Frame(self.depth), slot);
                                self.store(a, tmp, Width::B8);
                                self.writeback.push((slot, Some(e), p.subtype));
                                (tmp, dope)
                            }
                            _ => {
                                return Err(self.cx.ice(es, "scalar place for an array parameter"));
                            }
                        },
                    };
                    if ty::array_of(db, db.base(p.subtype)).is_some() {
                        dope = self.param_dope(p.subtype, dope, &p.name.text.to_string(), es)?;
                    }
                    self.array_cells(conv, data, dope, &mut cells);
                }
            }
        }
        Ok(cells)
    }

    /// The descriptor of an array signal actual as the formal `p` sees it.
    fn sig_formal_dope(&mut self, p: &Param, dope: Option<Reg>, span: Span) -> R<Option<Reg>> {
        let Some(d) = dope else { return Ok(None) };
        if ty::array_of(self.db, self.db.base(p.subtype)).is_some() {
            return self
                .param_dope(p.subtype, d, &p.name.text.to_string(), span)
                .map(Some);
        }
        Ok(Some(d))
    }

    pub(crate) fn native(
        &mut self,
        sym: &str,
        ty: Option<TyId>,
        cells: &[Reg],
        ret: Option<SubtypeId>,
        span: Span,
    ) -> R<Option<V>> {
        let key = (sym.to_string(), ty);
        let id = match self.cx.natives.get(&key) {
            Some(&id) => id,
            None => {
                let id = self.fb.program().native(sym, ty);
                self.cx.natives.insert(key, id);
                id
            }
        };
        let args = self.fb.regs(cells);
        let Some(ret) = ret else {
            self.emit(Inst::CallNative {
                native: id,
                args,
                dst: None,
            });
            return Ok(None);
        };
        let db = self.db;
        let arr = self.is_comp(db.base(ret));
        let dst = self.reg(if arr {
            RegTy::Ptr
        } else {
            self.rt_of(db.base(ret))
        });
        self.emit(Inst::CallNative {
            native: id,
            args,
            dst: Some(dst),
        });
        let _ = span;
        Ok(Some(if arr { self.fat_value(dst) } else { V::S(dst) }))
    }

    // ---- predefined operations ----

    fn implicit(
        &mut self,
        op: &ImplicitOp,
        sp: &Subprogram,
        args: &[Arg],
        span: Span,
    ) -> R<Option<V>> {
        let db = self.db;
        let mut ex = Vec::new();
        for a in args {
            let Some(p) = sp.params.get(a.formal as usize) else {
                return Err(self.cx.ice(span, "argument for a missing formal"));
            };
            ex.push(self.actual(a, p, span)?);
        }
        let t = op.ty;
        let scalar = matches!(
            db.types[t].kind,
            TypeKind::Enum { .. }
                | TypeKind::Integer
                | TypeKind::Floating
                | TypeKind::Physical { .. }
                | TypeKind::UniversalInteger
                | TypeKind::UniversalReal
        );
        let real = ty::is_real(db, t);
        let res = sp.ret;
        use Family::*;
        match op.family {
            Deallocate => return self.deallocate(ex[0], span).map(|()| None),
            FileOpen | FileClose | Read | Write | Endfile | Flush | FileState | FileMode
            | FilePosition | FileSize | FileSeek | FileTruncate | FileRewind | FileCanseek => {
                return self.file_op(op, sp, args, span);
            }
            Eq | Neq if matches!(db.types[t].kind, TypeKind::Access { .. }) => {
                let a = self.scalar_expr(ex[0])?;
                let b = self.scalar_expr(ex[1])?;
                return Ok(Some(V::S(self.cmp(cmp_op(op.family), a, b))));
            }
            Eq | Neq
                if self.is_comp(t)
                    && (self.is_general(t, span)?
                        || ty::array_of(db, t).is_some_and(|x| x.1 > 1)) =>
            {
                let a = self.expr(ex[0])?;
                let b = self.expr(ex[1])?;
                let r = self.comp_eq(t, a, b, span)?;
                return Ok(Some(V::S(if op.family == Neq { self.not(r) } else { r })));
            }
            _ => {}
        }
        let v = match op.family {
            And | Or | Nand | Nor if scalar && ex.len() == 2 => {
                let a = self.scalar_expr(ex[0])?;
                let dst = self.reg(RegTy::I64);
                let (eval, short, j) = (self.block(), self.block(), self.block());
                let is_and = matches!(op.family, And | Nand);
                if is_and {
                    self.branch(a, eval, short);
                } else {
                    self.branch(a, short, eval);
                }
                self.at(eval);
                let b = self.scalar_expr(ex[1])?;
                self.emit(Inst::Move { dst, src: b });
                self.jump(j);
                self.at(short);
                self.emit(Inst::Const {
                    dst,
                    v: i64::from(!is_and),
                });
                self.jump(j);
                self.at(j);
                if matches!(op.family, Nand | Nor) {
                    self.not(dst)
                } else {
                    dst
                }
            }
            Xor | Xnor if scalar => {
                let a = self.scalar_expr(ex[0])?;
                let b = self.scalar_expr(ex[1])?;
                let dst = self.reg(RegTy::I64);
                self.emit(Inst::Bool {
                    op: BoolOp::Xor,
                    dst,
                    a,
                    b,
                });
                if op.family == Xnor {
                    self.not(dst)
                } else {
                    dst
                }
            }
            Not if scalar => {
                let a = self.scalar_expr(ex[0])?;
                self.not(a)
            }
            Cond | MEq | MNeq if scalar && db.well_known.bit == Some(t) => {
                let a = self.scalar_expr(ex[0])?;
                let b = match ex.get(1) {
                    Some(&e) => self.scalar_expr(e)?,
                    None => self.int(1),
                };
                let c = if op.family == MNeq {
                    CmpOp::Ne
                } else {
                    CmpOp::Eq
                };
                self.cmp(c, a, b)
            }
            Eq | Neq | Lt | Le | Gt | Ge if scalar => {
                let c = cmp_op(op.family);
                let a = self.scalar_expr(ex[0])?;
                let b = self.scalar_expr(ex[1])?;
                let dst = self.reg(RegTy::I64);
                if real {
                    self.emit(Inst::FCmp { op: c, dst, a, b });
                } else {
                    self.emit(Inst::Cmp { op: c, dst, a, b });
                }
                dst
            }
            Minimum | Maximum if scalar && ex.len() == 2 => {
                let a = self.scalar_expr(ex[0])?;
                let b = self.scalar_expr(ex[1])?;
                let c = self.reg(RegTy::I64);
                let o = if op.family == Minimum {
                    CmpOp::Lt
                } else {
                    CmpOp::Gt
                };
                if real {
                    self.emit(Inst::FCmp {
                        op: o,
                        dst: c,
                        a,
                        b,
                    });
                } else {
                    self.emit(Inst::Cmp {
                        op: o,
                        dst: c,
                        a,
                        b,
                    });
                }
                let rt = if real { RegTy::F64 } else { RegTy::I64 };
                self.select(c, a, b, rt)
            }
            Add | Sub | Mul | Div | Rem | Mod | Exp | Neg | Pos | Abs => {
                return self.arith(op, &ex, res, span).map(|r| Some(V::S(r)));
            }
            Eq | Neq | Lt | Le | Gt | Ge if ty::is_array(db, t) => {
                let a = self.expr(ex[0])?;
                let b = self.expr(ex[1])?;
                self.array_cmp(cmp_op(op.family), a, b, t, span)?
            }
            And | Or | Nand | Nor | Xor | Xnor | Not
                if ty::is_array(db, t) && op.form == Form::Plain =>
            {
                return self.logic_op(op, &ex, t, span).map(Some);
            }
            Cat => return self.cat(op, &ex, span).map(Some),
            RisingEdge | FallingEdge => {
                let s = self.sig_expr(ex[0])?;
                let ev = self.reg(RegTy::I64);
                self.emit(Inst::SigAttr {
                    dst: ev,
                    kind: AttrKind::Event,
                    sig: s,
                });
                let v = self.reg(RegTy::I64);
                self.emit(Inst::SigReadScalar { dst: v, sig: s });
                // BIT and BOOLEAN signals hold the codes of '0' and '1'.
                let want = self.int(2 + i64::from(op.family == RisingEdge));
                let c = self.cmp(CmpOp::Eq, v, want);
                let dst = self.reg(RegTy::I64);
                self.emit(Inst::Bool {
                    op: BoolOp::And,
                    dst,
                    a: ev,
                    b: c,
                });
                dst
            }
            _ => {
                let mut cells = Vec::new();
                for &e in &ex {
                    match self.expr(e)? {
                        V::S(r) => cells.push(r),
                        V::A(d, p) => cells.push(self.make_fat(d, p)),
                    }
                }
                let tid = self.ty(t, span)?;
                let sym = yavs_sem::mangle::implicit_native(db, op);
                return self.native(&sym, Some(tid), &cells, res, span);
            }
        };
        Ok(Some(V::S(v)))
    }

    fn arith(
        &mut self,
        op: &ImplicitOp,
        ex: &[ExprId],
        res: Option<SubtypeId>,
        span: Span,
    ) -> R<Reg> {
        use Family::*;
        let db = self.db;
        let real = ty::is_real(db, op.ty);
        let a = self.scalar_expr(ex[0])?;
        let b = match ex.get(1) {
            Some(&e) => Some(self.scalar_expr(e)?),
            None => None,
        };
        let r = match (op.family, b) {
            (Pos, _) => return Ok(a),
            (Neg | Abs, _) if real => {
                let dst = self.reg(RegTy::F64);
                let o = if op.family == Neg {
                    FUnOp::Neg
                } else {
                    FUnOp::Abs
                };
                self.emit(Inst::FUn { op: o, dst, a });
                return Ok(dst);
            }
            (Neg | Abs, _) => {
                let dst = self.reg(RegTy::I64);
                let o = if op.family == Neg {
                    IntUnOp::Neg
                } else {
                    IntUnOp::Abs
                };
                self.emit(Inst::Un { op: o, dst, a });
                dst
            }
            (f, Some(b)) => match op.form {
                Form::PhysReal | Form::RealPhys => {
                    let (p, x) = if op.form == Form::PhysReal {
                        (a, b)
                    } else {
                        (b, a)
                    };
                    let dst = self.reg(RegTy::I64);
                    let o = if f == Div {
                        PhysOp::DivReal
                    } else {
                        PhysOp::MulReal
                    };
                    self.emit(Inst::Phys {
                        op: o,
                        dst,
                        a: p,
                        b: x,
                    });
                    dst
                }
                Form::RealInt | Form::IntReal => {
                    let (x, i) = if op.form == Form::RealInt {
                        (a, b)
                    } else {
                        (b, a)
                    };
                    let fi = self.reg(RegTy::F64);
                    self.emit(Inst::IToF { dst: fi, src: i });
                    let (l, r) = if op.form == Form::RealInt {
                        (x, fi)
                    } else {
                        (fi, x)
                    };
                    let dst = self.reg(RegTy::F64);
                    let o = if f == Div { FOp::Div } else { FOp::Mul };
                    self.emit(Inst::FBin {
                        op: o,
                        dst,
                        a: l,
                        b: r,
                    });
                    return Ok(dst);
                }
                _ if real => {
                    let o = match f {
                        Add => FOp::Add,
                        Sub => FOp::Sub,
                        Mul => FOp::Mul,
                        Div => FOp::Div,
                        Exp => FOp::PowI,
                        _ => return Err(self.cx.ice(span, "real rem/mod")),
                    };
                    let dst = self.reg(RegTy::F64);
                    self.emit(Inst::FBin { op: o, dst, a, b });
                    return Ok(dst);
                }
                _ => {
                    let o = match f {
                        Add => IntOp::Add,
                        Sub => IntOp::Sub,
                        Mul => IntOp::Mul,
                        Div => IntOp::Div,
                        Rem => IntOp::Rem,
                        Mod => IntOp::Mod,
                        Exp => IntOp::Pow,
                        _ => return Err(self.cx.ice(span, "arithmetic family")),
                    };
                    return self.bin_checked(o, a, b, res, span);
                }
            },
            _ => return Err(self.cx.ice(span, "binary operation with one operand")),
        };
        if let Some(st) = res {
            // `-x` and `abs x` overflow only at the most negative value of the base range.
            self.check_scalar(
                r,
                None,
                self.db.types[self.db.base(st)].base_subtype,
                "",
                span,
            )?;
        }
        Ok(r)
    }

    /// Element `i` (0-based from the left) of a 1-D array with `n` elements.
    pub(crate) fn elem_at(&mut self, data: Reg, i: Reg, n: Reg, logic: bool, w: Width) -> Reg {
        if logic {
            let one = self.int(1);
            let last = self.bin(IntOp::Sub, n, one);
            let idx = self.bin(IntOp::Sub, last, i);
            let dst = self.reg(RegTy::I64);
            self.emit(Inst::LGet {
                dst,
                base: data,
                idx,
            });
            dst
        } else {
            let a = self.reg(RegTy::Ptr);
            self.emit(Inst::Index {
                dst: a,
                base: data,
                pos: i,
                stride: w.bytes(),
            });
            self.load(a, w, RegTy::I64)
        }
    }

    /// Element type facts of an array type: (IR type, logic, element width).
    pub(crate) fn elem_facts(&mut self, t: TypeId, span: Span) -> R<(TyId, bool, Width)> {
        let tid = self.ty(t, span)?;
        let logic = self.is_logic(tid);
        let Some((elem, _)) = ty::array_of(self.db, t) else {
            return Err(self.cx.ice(span, "not an array type"));
        };
        let (_, _, w) = self.elem_scalar(elem, span)?;
        Ok((tid, logic, w))
    }

    /// Lexicographic comparison of 1-D arrays of discrete elements (LRM 9.2.3).
    pub(crate) fn array_cmp(&mut self, op: CmpOp, a: V, b: V, t: TypeId, span: Span) -> R<Reg> {
        let (ad, ap) = self.arr(a, span)?;
        let (bd, bp) = self.arr(b, span)?;
        let (_, logic, w) = self.elem_facts(t, span)?;
        let la = self.dope_len(ap);
        let lb = self.dope_len(bp);
        let r = self.reg(RegTy::I64);
        if op == CmpOp::Eq && logic {
            // Equal when the lengths are and the planes are, word by word.
            let same = self.cmp(CmpOp::Eq, la, lb);
            let (go, no, j) = (self.block(), self.block(), self.block());
            self.branch(same, go, no);
            self.at(go);
            self.emit(Inst::LEq {
                dst: r,
                a: ad,
                b: bd,
                n: la,
            });
            self.jump(j);
            self.at(no);
            self.emit(Inst::Const { dst: r, v: 0 });
            self.jump(j);
            self.at(j);
            return Ok(r);
        }
        if !logic && matches!(op, CmpOp::Eq | CmpOp::Ne) {
            // Equal when the lengths are and the bytes are (elements here are integers and
            // enumerations, which compare as their bytes do).
            let same = self.cmp(CmpOp::Eq, la, lb);
            let (go, no, j) = (self.block(), self.block(), self.block());
            self.branch(same, go, no);
            self.at(go);
            let stride = self.int(i64::from(w.bytes()));
            let n = self.bin(IntOp::Mul, la, stride);
            self.emit(Inst::MemEq {
                dst: r,
                a: ad,
                b: bd,
                n,
            });
            self.jump(j);
            self.at(no);
            self.emit(Inst::Const { dst: r, v: 0 });
            self.jump(j);
            self.at(j);
            return Ok(if op == CmpOp::Ne { self.not(r) } else { r });
        }
        let i = self.reg(RegTy::I64);
        self.emit(Inst::Const { dst: i, v: 0 });
        let lt = self.cmp(CmpOp::Lt, la, lb);
        let m = self.select(lt, la, lb, RegTy::I64);
        let (head, body, inc, diff, tail, j) = (
            self.block(),
            self.block(),
            self.block(),
            self.block(),
            self.block(),
            self.block(),
        );
        self.jump(head);
        self.at(head);
        let more = self.cmp(CmpOp::Lt, i, m);
        self.branch(more, body, tail);
        self.at(body);
        let ea = self.elem_at(ad, i, la, logic, w);
        let eb = self.elem_at(bd, i, lb, logic, w);
        let same = self.cmp(CmpOp::Eq, ea, eb);
        self.branch(same, inc, diff);
        self.at(inc);
        let one = self.int(1);
        let n = self.bin(IntOp::Add, i, one);
        self.emit(Inst::Move { dst: i, src: n });
        self.jump(head);
        self.at(diff);
        let strict = match op {
            CmpOp::Le => CmpOp::Lt,
            CmpOp::Ge => CmpOp::Gt,
            o => o,
        };
        let d = self.cmp(strict, ea, eb);
        self.emit(Inst::Move { dst: r, src: d });
        self.jump(j);
        self.at(tail);
        let e = self.cmp(op, la, lb);
        self.emit(Inst::Move { dst: r, src: e });
        self.jump(j);
        self.at(j);
        Ok(r)
    }

    pub fn dope_len(&mut self, dope: Reg) -> Reg {
        let dst = self.reg(RegTy::I64);
        self.emit(Inst::DopeLen { dst, dope, dim: 0 });
        dst
    }

    fn logic_op(&mut self, op: &ImplicitOp, ex: &[ExprId], t: TypeId, span: Span) -> R<V> {
        let (tid, logic, _) = self.elem_facts(t, span)?;
        if !logic {
            return Err(self
                .cx
                .unsupported(span, "logical operators on arrays of non-logic types"));
        }
        let a = self.expr(ex[0])?;
        let (ad, ap) = self.arr(a, span)?;
        let n = self.dope_len(ap);
        let b = match ex.get(1) {
            Some(&e) => {
                let v = self.expr(e)?;
                let (bd, bp) = self.arr(v, span)?;
                let what = self.msg("right operand", Dir::To);
                self.emit(Inst::SubtypeCheck {
                    dope: bp,
                    want: ap,
                    dims: 1,
                    what,
                });
                bd
            }
            None => ad,
        };
        let size = self.size_of(tid, ap);
        let dst = self.alloc(size);
        let o = match op.family {
            Family::And => LOp::And,
            Family::Or => LOp::Or,
            Family::Nand => LOp::Nand,
            Family::Nor => LOp::Nor,
            Family::Xor => LOp::Xor,
            Family::Xnor => LOp::Xnor,
            _ => LOp::Not,
        };
        self.emit(Inst::LogicOp {
            op: o,
            dst,
            a: ad,
            b,
            n,
        });
        Ok(V::A(dst, ap))
    }

    /// The operands of `&` expression `ex` with a flag for each telling whether it is an
    /// element: those of a chain `a & b & c` (the left operand an array `&` of the same type)
    /// in order.
    fn cat_items(&self, op: &ImplicitOp, ex: &[ExprId]) -> Vec<(ExprId, bool)> {
        let (ls, rs) = cat_scalars(op.form);
        let mut right = vec![(ex[1], rs)];
        let mut cur = (ex[0], ls);
        while !cur.1
            && let Some((l, r, inner)) = self.cat_args(cur.0, op)
        {
            let (ils, irs) = cat_scalars(inner.form);
            right.push((r, irs));
            cur = (l, ils);
        }
        right.push(cur);
        right.reverse();
        right
    }

    /// The operands and operation of expression `e` when it is a `&` of the type of `op`.
    fn cat_args(&self, e: ExprId, op: &ImplicitOp) -> Option<(ExprId, ExprId, ImplicitOp)> {
        let db = self.db;
        let ExprKind::Call {
            callee,
            args,
            method_of: None,
            ..
        } = &db.exprs[e].kind
        else {
            return None;
        };
        let Body::Implicit(inner) = &db.decls[*callee].kind.subprogram()?.body else {
            return None;
        };
        if inner.family != Family::Cat || inner.ty != op.ty {
            return None;
        }
        match &args[..] {
            [
                Arg {
                    formal: 0,
                    actual: Actual::Expr(l),
                    ..
                },
                Arg {
                    formal: 1,
                    actual: Actual::Expr(r),
                    ..
                },
            ] => Some((*l, *r, *inner)),
            _ => None,
        }
    }

    /// `&` of 1-D arrays and elements (LRM 2008 9.2.5): bounds from the index subtype.
    /// Composite elements (arrays of arrays, of records) are copied as rows; an element
    /// subtype the type leaves open takes its constraint from the left operand, and every
    /// operand is checked against it.
    fn cat(&mut self, op: &ImplicitOp, ex: &[ExprId], span: Span) -> R<V> {
        let db = self.db;
        let t = op.ty;
        let TypeKind::Array { indexes, elem } = &db.types[t].kind else {
            return Err(self.cx.ice(span, "concatenation of a non-array type"));
        };
        let (index, elem) = (indexes[0], *elem);
        let tid = self.ty(t, span)?;
        let etid = self.ty(db.base(elem), span)?;
        let scalar = crate::composite::elem_w(self.fb.program(), etid);
        let logic = self.is_logic(tid);
        // `a & b & c` of scalar elements is one concatenation of three operands, not two with
        // an intermediate array (the result's bounds and the null-operand rule are the same).
        let items = if scalar.is_some() {
            self.cat_items(op, ex)
        } else {
            let (ls, rs) = cat_scalars(op.form);
            vec![(ex[0], ls), (ex[1], rs)]
        };
        let one = self.int(1);
        // Per operand: (data or scalar, element count of an array, descriptor of an array
        // operand or of a composite element).
        let mut ops = Vec::new();
        for &(e, sc) in &items {
            let v = self.expr(e)?;
            ops.push(match (sc, scalar) {
                (true, Some(_)) => (self.scal(v, span)?, None, None),
                (true, None) => {
                    let (d, p) = self.arr(v, span)?;
                    (d, Some(one), Some((p, true)))
                }
                (false, _) => {
                    let (d, p) = self.arr(v, span)?;
                    (d, Some(self.dope_len(p)), Some((p, false)))
                }
            });
        }
        let mut n = ops[0].1.unwrap_or(one);
        for o in &ops[1..] {
            n = self.bin(IntOp::Add, n, o.1.unwrap_or(one));
        }
        let s = self.scalar_bounds(index, span)?;
        let last = self.bin(IntOp::Sub, n, one);
        let right = match s.dir {
            Dr::Static(Direction::To) => self.bin(IntOp::Add, s.left, last),
            Dr::Static(Direction::Downto) => self.bin(IntOp::Sub, s.left, last),
            Dr::Dyn(_) => return Err(self.cx.ice(span, "index subtype with a runtime direction")),
        };
        let dir = self.dir_reg(s.dir);
        let mut dope = self.one_dope(s.left, right, dir);
        let z = self.int(0);
        let ok = self.cmp(CmpOp::Ne, n, z);
        let (lo, hi) = self.low_high(s);
        let what = self.msg(
            &format!("the result of `&` ({})", ty::subtype_name(db, index)),
            ir_dir(s.dir),
        );
        let chk = self.select(ok, right, s.left, RegTy::I64);
        self.emit(Inst::Check {
            kind: CheckKind::Index,
            v: chk,
            lo,
            hi,
            what,
        });
        let (m, w) = match scalar {
            Some(w) => (self.int(if logic { 1 } else { i64::from(w.bytes()) }), w),
            None => {
                // An open element subtype: the element part of the left operand.
                let ds = self.desc_size(tid);
                if ds > 24 {
                    let Some((p, elem_op)) = ops[0].2 else {
                        return Err(self.cx.ice(span, "composite operand without a descriptor"));
                    };
                    let src = if elem_op {
                        p
                    } else {
                        self.elem_desc(tid, p)?.1
                    };
                    let d = self.alloc_n(ds);
                    let n24 = self.int(24);
                    self.emit(Inst::Copy {
                        dst: d,
                        src: dope,
                        bytes: n24,
                    });
                    let at = self.offset(d, 24);
                    let rest = self.int(i64::from(ds - 24));
                    self.emit(Inst::Copy {
                        dst: at,
                        src,
                        bytes: rest,
                    });
                    dope = d;
                }
                let (_, ed) = self.elem_desc(tid, dope)?;
                for &(_, _, p) in &ops {
                    let what = "concatenation operand";
                    match p {
                        Some((p, true)) => {
                            self.check_desc(db.base(elem), etid, p, ed, what, span)?
                        }
                        Some((p, false)) if ds > 24 => {
                            let (_, pe) = self.elem_desc(tid, p)?;
                            self.check_desc(db.base(elem), etid, pe, ed, what, span)?;
                        }
                        _ => {}
                    }
                }
                // Units per element: plane elements of a logic array, else bytes.
                if logic {
                    (self.layout_count(etid, ed), Width::B8)
                } else {
                    let l = self.lay(tid, dope);
                    let sa = self.offset(l, 16);
                    (self.load(sa, Width::B8, RegTy::I64), Width::B8)
                }
            }
        };
        let size = self.size_of(tid, dope);
        let data = self.alloc_val(size, tid, Some(dope));
        let bias = self.code_bias(tid);
        let pieces: Vec<_> = ops.iter().map(|o| (o.0, o.1)).collect();
        self.put_pieces(data, n, logic, (bias, w), m, &pieces);
        // Null operands only: the result is the right operand (LRM 9.2.5 a), the last of a
        // chain.
        if let Some(&(last, _, Some((rd, false)))) = ops.last()
            && ops.iter().all(|o| matches!(o.2, Some((_, false))))
        {
            let data = self.select(ok, data, last, RegTy::Ptr);
            let dope = self.select(ok, dope, rd, RegTy::Ptr);
            return Ok(V::A(data, dope));
        }
        Ok(V::A(data, dope))
    }

    /// Store `pieces` (a scalar value, or array data and its element count) one after another
    /// from the left into the 1-D array `data` of `n` elements, each `m` plane elements
    /// (`logic`) or `m` bytes; scalars are encoded with `bias` or stored with width `w`.
    #[allow(clippy::too_many_arguments)]
    fn put_pieces(
        &mut self,
        data: Reg,
        n: Reg,
        logic: bool,
        bw: (i64, Width),
        m: Reg,
        pieces: &[(Reg, Option<Reg>)],
    ) {
        let one = self.int(1);
        // One instruction for up to 16 pieces; more are stored piece by piece.
        if pieces.len() <= 16
            && let Ok(bias) = u8::try_from(bw.0)
        {
            let mut list = Vec::with_capacity(2 * pieces.len());
            let mut scalars = 0u16;
            for (j, &(r, len)) in pieces.iter().enumerate() {
                list.push(r);
                list.push(len.unwrap_or(one));
                if len.is_none() {
                    scalars |= 1 << j;
                }
            }
            let parts = self.fb.regs(&list);
            self.emit(Inst::Cat {
                dst: data,
                n,
                m,
                parts,
                scalars,
                bias,
                logic,
                w: bw.1,
            });
            return;
        }
        let mut k = self.int(0);
        for &piece in pieces {
            self.put_at(data, n, logic, bw, m, k, piece);
            k = self.bin(IntOp::Add, k, piece.1.unwrap_or(one));
        }
    }

    /// Store one piece of `put_pieces` at element position `k` (from the left).
    #[allow(clippy::too_many_arguments)]
    fn put_at(
        &mut self,
        data: Reg,
        n: Reg,
        logic: bool,
        (bias, w): (i64, Width),
        m: Reg,
        k: Reg,
        (r, len): (Reg, Option<Reg>),
    ) {
        let one = self.int(1);
        let z = self.int(0);
        let at = self.bin(IntOp::Mul, k, m);
        let units = len.map(|len| self.bin(IntOp::Mul, len, m));
        if logic {
            // Planes hold the leftmost element last.
            let total = self.bin(IntOp::Mul, n, m);
            let x = self.bin(IntOp::Sub, total, at);
            let off = self.bin(IntOp::Sub, x, units.unwrap_or(one));
            match units {
                None => {
                    let v = self.enc(r, bias);
                    self.emit(Inst::LSet {
                        base: data,
                        idx: off,
                        v,
                    })
                }
                Some(units) => self.emit(Inst::LCopy {
                    dst: data,
                    dst_off: off,
                    src: r,
                    src_off: z,
                    n: units,
                }),
            }
        } else {
            let a = self.reg(RegTy::Ptr);
            self.emit(Inst::Index {
                dst: a,
                base: data,
                pos: at,
                stride: 1,
            });
            match units {
                None => self.store(a, r, w),
                Some(bytes) => self.emit(Inst::Copy {
                    dst: a,
                    src: r,
                    bytes,
                }),
            }
        }
    }

    /// A 1-D aggregate with 2008 array-valued pieces (LRM 9.3.3.3). Positional pieces
    /// (`(a, '1', b)`, `(rows, row)`) are concatenated, bounds as for `&` unless the context
    /// gives them (checked); a piece under a range choice (`(7 downto 4 => a, others => '0')`)
    /// fills the elements of the range in the aggregate's order. Each value is evaluated
    /// once. An element subtype the type leaves open takes its constraint from the context or
    /// the first value, and every value is checked against it.
    pub(crate) fn aggregate_pieces(
        &mut self,
        assocs: &[AggAssoc],
        bounds: &AggBounds,
        st: SubtypeId,
        span: Span,
    ) -> R<V> {
        let db = self.db;
        let t = db.base(st);
        let TypeKind::Array { indexes, elem } = &db.types[t].kind else {
            return Err(self.cx.ice(span, "aggregate pieces of a non-array"));
        };
        // Sem allows pieces in one-dimensional aggregates only (LRM 9.3.3.3).
        if indexes.len() != 1 {
            return Err(self.cx.ice(
                span,
                "array-valued elements in a multi-dimensional aggregate",
            ));
        }
        let (index, elem) = (indexes[0], *elem);
        let tid = self.ty(t, span)?;
        let etid = self.ty(db.base(elem), span)?;
        let scalar = crate::composite::elem_w(self.fb.program(), etid);
        let logic = self.is_logic(tid);
        let is_others = |a: &AggAssoc| a.choices.iter().any(|c| matches!(c, Choice::Others));
        let mut vals = Vec::new();
        // Elements of the positional associations.
        let mut n = self.int(0);
        for a in assocs {
            if a.slice_piece && is_others(a) {
                return Err(self.cx.ice(
                    span,
                    "`others` with an array value in an aggregate (sem rejects it)",
                ));
            }
            let v = self.expr(a.value)?;
            let len = if a.slice_piece {
                let (_, p) = self.arr(v, span)?;
                self.dope_len(p)
            } else {
                self.int(1)
            };
            if a.choices.is_empty() {
                n = self.bin(IntOp::Add, n, len);
            }
            vals.push((a, v, len));
        }
        let (mut dope, full) = if assocs.iter().all(|a| a.choices.is_empty()) {
            let s = self.scalar_bounds(index, span)?;
            let one = self.int(1);
            let last = self.bin(IntOp::Sub, n, one);
            let right = match s.dir {
                Dr::Static(Direction::To) => self.bin(IntOp::Add, s.left, last),
                Dr::Static(Direction::Downto) => self.bin(IntOp::Sub, s.left, last),
                Dr::Dyn(_) => {
                    return Err(self.cx.ice(span, "index subtype with a runtime direction"));
                }
            };
            let dir = self.dir_reg(s.dir);
            let dope = self.one_dope(s.left, right, dir);
            let z = self.int(0);
            let ok = self.cmp(CmpOp::Ne, n, z);
            let (lo, hi) = self.low_high(s);
            let what = self.msg(
                &format!("the aggregate ({})", ty::subtype_name(db, index)),
                ir_dir(s.dir),
            );
            let chk = self.select(ok, right, s.left, RegTy::I64);
            self.emit(Inst::Check {
                kind: CheckKind::Index,
                v: chk,
                lo,
                hi,
                what,
            });
            match bounds {
                AggBounds::Context => match self.subtype_dope(st, span)? {
                    Some(want) => {
                        let what = self.msg("aggregate", Dir::To);
                        self.emit(Inst::SubtypeCheck {
                            dope,
                            want,
                            dims: 1,
                            what,
                        });
                        (want, true)
                    }
                    None => (dope, false),
                },
                _ => (dope, false),
            }
        } else {
            match bounds {
                AggBounds::Context => match self.subtype_dope(st, span)? {
                    Some(d) => (d, true),
                    None => return Err(self.cx.ice(span, "aggregate context without bounds")),
                },
                // Sem's result subtype: the direction of a constrained context.
                AggBounds::Named { .. } if let Some(d) = self.subtype_dope(st, span)? => (d, true),
                AggBounds::Named { low, high } => {
                    let s = self.scalar_bounds(index, span)?;
                    let lo = self.scalar_expr(*low)?;
                    let hi = self.scalar_expr(*high)?;
                    match s.dir {
                        Dr::Static(Direction::To) => {
                            let d = self.int(0);
                            (self.one_dope(lo, hi, d), false)
                        }
                        Dr::Static(Direction::Downto) => {
                            let d = self.int(1);
                            (self.one_dope(hi, lo, d), false)
                        }
                        Dr::Dyn(_) => {
                            return Err(self
                                .cx
                                .ice(span, "index subtype with a runtime direction"));
                        }
                    }
                }
                _ => return Err(self.cx.ice(span, "named aggregate without named bounds")),
            }
        };
        // An element subtype the type leaves open: the element part of the descriptor comes
        // from the first value unless the context gave it.
        let ds = self.desc_size(tid);
        if ds > 24 && !full {
            let Some((a, v, _)) = vals.first() else {
                return Err(self.cx.ice(span, "aggregate without associations"));
            };
            let (_, vd) = self.arr(*v, span)?;
            let src = if a.slice_piece {
                self.elem_desc(tid, vd)?.1
            } else {
                vd
            };
            let d = self.alloc_n(ds);
            let n24 = self.int(24);
            self.emit(Inst::Copy {
                dst: d,
                src: dope,
                bytes: n24,
            });
            let at = self.offset(d, 24);
            let rest = self.int(i64::from(ds - 24));
            self.emit(Inst::Copy {
                dst: at,
                src,
                bytes: rest,
            });
            dope = d;
        }
        let (_, ed) = self.elem_desc(tid, dope)?;
        // Units per element: plane elements of a logic array, else bytes.
        let (m, w) = match scalar {
            Some(w) => (self.int(if logic { 1 } else { i64::from(w.bytes()) }), w),
            None if logic => (self.layout_count(etid, ed), Width::B8),
            None => {
                let l = self.lay(tid, dope);
                let sa = self.offset(l, 16);
                (self.load(sa, Width::B8, RegTy::I64), Width::B8)
            }
        };
        let mut pieces = Vec::new();
        for (a, v, len) in vals {
            pieces.push(match v {
                V::S(r) => {
                    self.check_scalar(r, Some(db.exprs[a.value].subtype), elem, "", span)?;
                    (a, (r, None), None)
                }
                V::A(d, p) if a.slice_piece => {
                    if ds > 24 {
                        let (_, pe) = self.elem_desc(tid, p)?;
                        self.check_desc(db.base(elem), etid, pe, ed, "aggregate element", span)?;
                    }
                    (a, (d, Some(len)), Some(p))
                }
                V::A(d, p) => {
                    let what = "aggregate element";
                    self.check_desc(db.base(elem), etid, p, ed, what, span)?;
                    (a, (d, Some(len)), None)
                }
            });
        }
        let size = self.size_of(tid, dope);
        let data = self.alloc_val(size, tid, Some(dope));
        let count = self.dope_len(dope);
        let bw = (self.code_bias(etid), w);
        if assocs.iter().all(|a| a.choices.is_empty()) {
            let pieces: Vec<_> = pieces.into_iter().map(|p| p.1).collect();
            self.put_pieces(data, n, logic, bw, m, &pieces);
            return Ok(V::A(data, dope));
        }
        let what = self.msg("aggregate", Dir::To);
        let pos_of = |fx: &mut Self, idx: Reg| {
            let pos = fx.reg(RegTy::I64);
            fx.emit(Inst::Pos {
                dst: pos,
                dope,
                dim: 0,
                idx,
                what,
            });
            pos
        };
        // `others` first: the other associations then overwrite their positions.
        for (_, piece, _) in pieces.iter().filter(|p| is_others(p.0)) {
            let piece = *piece;
            self.for_n(count, |fx, i| {
                fx.put_at(data, count, logic, bw, m, i, piece);
                Ok(())
            })?;
        }
        let mut k = self.int(0);
        let one = self.int(1);
        for (a, piece, pd) in pieces.into_iter().filter(|p| !is_others(p.0)) {
            if a.choices.is_empty() {
                self.put_at(data, count, logic, bw, m, k, piece);
                k = self.bin(IntOp::Add, k, piece.1.unwrap_or(one));
                continue;
            }
            for c in a.choices.iter() {
                match (c, pd) {
                    (Choice::Expr(c), None) => {
                        let i = self.scalar_expr(*c)?;
                        let pos = pos_of(self, i);
                        self.put_at(data, count, logic, bw, m, pos, piece);
                    }
                    // The piece fills the range's elements in the aggregate's order.
                    (Choice::Range(rc), Some(pd)) => {
                        let r = self.range(rc, span)?;
                        let d = self.dir_reg(r.dir);
                        let rd = self.one_dope(r.left, r.right, d);
                        self.emit(Inst::SubtypeCheck {
                            dope: pd,
                            want: rd,
                            dims: 1,
                            what,
                        });
                        let rn = self.dope_len(rd);
                        let z = self.int(0);
                        let some = self.cmp(CmpOp::Gt, rn, z);
                        let (body, join) = (self.block(), self.block());
                        self.branch(some, body, join);
                        self.at(body);
                        let (lo, hi) = self.low_high(r);
                        let (pl, ph) = (pos_of(self, lo), pos_of(self, hi));
                        let first = self.cmp(CmpOp::Lt, pl, ph);
                        let p = self.select(first, pl, ph, RegTy::I64);
                        self.put_at(data, count, logic, bw, m, p, piece);
                        self.jump(join);
                        self.at(join);
                    }
                    (Choice::Range(rc), None) => {
                        let r = self.range(rc, span)?;
                        let d = self.dir_reg(r.dir);
                        let rd = self.one_dope(r.left, r.right, d);
                        let rn = self.dope_len(rd);
                        let z = self.int(0);
                        let up = self.cmp(CmpOp::Eq, d, z);
                        let m1 = self.int(-1);
                        let step = self.select(up, one, m1, RegTy::I64);
                        let left = r.left;
                        self.for_n(rn, |fx, i| {
                            let off = fx.bin(IntOp::Mul, i, step);
                            let idx = fx.bin(IntOp::Add, left, off);
                            let pos = pos_of(fx, idx);
                            fx.put_at(data, count, logic, bw, m, pos, piece);
                            Ok(())
                        })?;
                    }
                    _ => return Err(self.cx.ice(span, "element choice in an array aggregate")),
                }
            }
        }
        Ok(V::A(data, dope))
    }

    // ---- attributes ----

    /// `T'IMAGE(X)` and the 2019 object form `X'IMAGE` (LRM 2019 16.2.2): scalars by value,
    /// arrays and records as a fat pair (the native formats them recursively).
    fn image_attr(&mut self, prefix: AttrPrefix, args: &[ExprId], span: Span) -> R<V> {
        let db = self.db;
        let (st, x) = match (prefix, args.first()) {
            (AttrPrefix::Subtype(st), Some(&x)) => (st, x),
            (AttrPrefix::Expr(e), None) => (db.exprs[e].subtype, e),
            _ => return Err(self.cx.ice(span, "'image without a value")),
        };
        let t = db.base(st);
        let v = if ty::is_array(db, t) || matches!(db.types[t].kind, TypeKind::Record { .. }) {
            let v = self.expr(x)?;
            let (data, desc) = self.arr(v, span)?;
            self.make_fat(data, desc)
        } else {
            self.scalar_expr(x)?
        };
        let tid = self.ty(t, span)?;
        let string = self.string_subtype(span)?;
        self.native("yavs.attr.image", Some(tid), &[v], Some(string), span)?
            .ok_or(Failed)
    }

    /// `T'VALUE(S)`: the native also takes the prefix descriptor, whose bounds it
    /// preserves for a constrained prefix and whose open dimensions it fills from `S`.
    fn value_attr(
        &mut self,
        prefix: AttrPrefix,
        args: &[ExprId],
        res: SubtypeId,
        span: Span,
    ) -> R<V> {
        let AttrPrefix::Subtype(st) = prefix else {
            return Err(self.cx.ice(span, "'value without a type prefix"));
        };
        let t = self.db.base(st);
        let v = self.expr(args[0])?;
        let (d, p) = self.arr(v, span)?;
        let fat = self.make_fat(d, p);
        let tid = self.ty(t, span)?;
        let desc = self.desc_of(st, None, span)?;
        let mut cells = vec![fat];
        if let Some(desc) = desc {
            cells.push(desc);
        } else {
            let TypeKind::Array { indexes, .. } = &self.db.types[t].kind else {
                return Err(self.cx.ice(span, "open record without a descriptor"));
            };
            for &ix in indexes {
                let r = self.scalar_bounds(ix, span)?;
                let d = self.dir_reg(r.dir);
                cells.extend([r.left, d]);
            }
        }
        let r = self
            .native("yavs.attr.value", Some(tid), &cells, Some(res), span)?
            .ok_or(Failed)?;
        if let V::S(r) = r {
            self.check_scalar(r, None, st, "", span)?;
        }
        Ok(r)
    }

    fn attr(
        &mut self,
        attr: PredefAttr,
        prefix: AttrPrefix,
        args: &[ExprId],
        res: SubtypeId,
        span: Span,
    ) -> R<V> {
        use PredefAttr::*;
        if attr == Reflect {
            return self.reflect_attr(prefix, span);
        }
        if attr == Image {
            return self.image_attr(prefix, args, span);
        }
        if attr == Value {
            return self.value_attr(prefix, args, res, span);
        }
        let db = self.db;
        let dim = |fx: &Fx<'_, '_>| -> u8 {
            args.first()
                .and_then(|&a| fx.db.value(a))
                .and_then(ConstValue::as_int)
                .map_or(0, |d| (d - 1).clamp(0, 255) as u8)
        };
        match prefix {
            AttrPrefix::Subtype(st) if !ty::is_array(db, db.base(st)) => {
                let t = db.base(st);
                match attr {
                    Left | Right | High | Low | Ascending => {
                        let r = self.scalar_bounds(st, span)?;
                        self.range_attr(attr, r, None, span)
                    }
                    // 2019 `T'LENGTH` of a discrete subtype: its number of values (LCS-2016-018).
                    Length => {
                        let r = self.scalar_bounds(st, span)?;
                        let (lo, hi) = self.low_high(r);
                        let d = self.bin(IntOp::Sub, hi, lo);
                        let one = self.int(1);
                        let n = self.bin(IntOp::Add, d, one);
                        let z = self.int(0);
                        let neg = self.cmp(CmpOp::Lt, n, z);
                        Ok(V::S(self.select(neg, z, n, RegTy::I64)))
                    }
                    Pos => Ok(V::S(self.scalar_expr(args[0])?)),
                    Val => {
                        let v = self.scalar_expr(args[0])?;
                        self.check_scalar(v, None, st, "", span)?;
                        Ok(V::S(v))
                    }
                    Succ | Pred | Leftof | Rightof => {
                        let v = self.scalar_expr(args[0])?;
                        // The step: +1 or -1 (`'leftof`/`'rightof` follow the direction).
                        let step = match attr {
                            Succ => self.int(1),
                            Pred => self.int(-1),
                            _ => {
                                let r = self.scalar_bounds(st, span)?;
                                let toward = if attr == Rightof { 1 } else { -1 };
                                match r.dir {
                                    Dr::Static(Direction::To) => self.int(toward),
                                    Dr::Static(Direction::Downto) => self.int(-toward),
                                    Dr::Dyn(d) => {
                                        let z = self.int(0);
                                        let up = self.cmp(CmpOp::Eq, d, z);
                                        let a = self.int(toward);
                                        let b = self.int(-toward);
                                        self.select(up, a, b, RegTy::I64)
                                    }
                                }
                            }
                        };
                        let r = self.bin(IntOp::Add, v, step);
                        let check = if matches!(attr, Succ | Pred) {
                            db.types[t].base_subtype
                        } else {
                            st
                        };
                        self.check_scalar(
                            r,
                            None,
                            check,
                            &format!("{}'{}", ty::subtype_name(db, st), attr.name()),
                            span,
                        )?;
                        Ok(V::S(r))
                    }
                    _ => Err(self
                        .cx
                        .unsupported(span, &format!("attribute '{}", attr.name()))),
                }
            }
            AttrPrefix::Subtype(st) => {
                let Some(dope) = self.subtype_dope(st, span)? else {
                    // Only the range attributes read the bounds of the type mark; the others
                    // (2019 'value of a composite type) are not lowered.
                    if !matches!(attr, Left | Right | Low | High | Ascending | Length) {
                        return Err(self
                            .cx
                            .unsupported(span, &format!("attribute '{}", attr.name())));
                    }
                    return Err(self
                        .cx
                        .ice(span, "array attribute of an unconstrained type"));
                };
                let k = dim(self);
                let r = self.dope_range(dope, k, RegTy::I64);
                self.range_attr(attr, r, Some((dope, k)), span)
            }
            AttrPrefix::Expr(e) if matches!(attr, Driving | DrivingValue) => {
                // The process's own driver of the whole signal (LRM 16.2.4).
                let slot = match db.exprs[e].kind {
                    ExprKind::Obj(d) => self.drivers.iter().position(|l| {
                        l.root == yavs_sem::hir::ObjRoot::Decl(d) && l.path.is_empty()
                    }),
                    _ => None,
                };
                let Some(slot) = slot else {
                    // A signal this process does not drive at all (LRM 16.2.4: an error when
                    // evaluated, so at run time, as the reference simulators do). Signal
                    // parameters and partly driven composites stay unlowered.
                    if let ExprKind::Obj(d) = db.exprs[e].kind {
                        let root = yavs_sem::hir::ObjRoot::Decl(d);
                        if !self.drivers.iter().any(|l| l.root == root)
                            && !matches!(self.cx.objs.get(&d), Some(Obj::SignalRef { .. }))
                        {
                            let what = self.msg(
                                &format!(
                                    "'{} of signal {} in a process that does not contain a driver for it (LRM 16.2.4)",
                                    if attr == Driving { "driving" } else { "driving_value" },
                                    db.decls[d].name
                                ),
                                Dir::To,
                            );
                            self.term(Inst::Unreachable { what });
                            let dead = self.block();
                            self.at(dead);
                            return Ok(match (attr == Driving, self.dope_of_signal(e)?) {
                                (true, _) => V::S(self.int(0)),
                                (_, Some(dope)) => {
                                    let tid = self.ty(db.base(db.exprs[e].subtype), span)?;
                                    let size = self.size_of(tid, dope);
                                    V::A(self.alloc(size), dope)
                                }
                                _ => V::S(self.int(0)),
                            });
                        }
                    }
                    return Err(self
                        .cx
                        .unsupported(span, "'driving of signal parameters and sub-elements"));
                };
                let slot = self.driver_base.get(slot).copied().unwrap_or(slot as u32);
                if attr == Driving {
                    let dst = self.reg(RegTy::I64);
                    self.emit(Inst::Driving { dst, driver: slot });
                    return Ok(V::S(dst));
                }
                let st = db.exprs[e].subtype;
                let tid = self.ty(db.base(st), span)?;
                if let Some(dope) = self.dope_of_signal(e)? {
                    let size = self.size_of(tid, dope);
                    let buf = self.alloc(size);
                    self.emit(Inst::DrivingValue {
                        dst: buf,
                        driver: slot,
                    });
                    return Ok(V::A(buf, dope));
                }
                let (_, rt, w) = self.sig_scalar(st, span)?;
                let logic = self.is_logic(tid);
                let buf = self.alloc_n(if logic { 32 } else { 8 });
                self.emit(Inst::DrivingValue {
                    dst: buf,
                    driver: slot,
                });
                if logic {
                    let z = self.int(0);
                    let c = self.reg(RegTy::I64);
                    self.emit(Inst::LGet {
                        dst: c,
                        base: buf,
                        idx: z,
                    });
                    let bias = self.code_bias(tid);
                    return Ok(V::S(self.dec(c, bias)));
                }
                Ok(V::S(self.load(buf, w, rt)))
            }
            AttrPrefix::Expr(e)
                if matches!(attr, Event | Active | LastEvent | LastActive | LastValue) =>
            {
                // A part of a composite signal over several leaves: the attribute of the
                // composite from those of its scalars (LRM 16.2.4).
                if let Some((d, steps)) = self.comp_steps(e) {
                    let p = self.comp_select(d, &steps, span)?;
                    if !matches!(p.leaves.as_slice(), [(l, _, _)] if l.path == p.path) {
                        return self.comp_attr(attr, &p, span);
                    }
                }
                // A signal, signal element or slice, or signal parameter (LRM 16.2.4).
                let (_, s, dope) = self.sig_name(e)?;
                let st = db.exprs[e].subtype;
                let kind = match attr {
                    Event => AttrKind::Event,
                    Active => AttrKind::Active,
                    LastEvent => AttrKind::LastEvent,
                    LastActive => AttrKind::LastActive,
                    _ => {
                        let tid = self.ty(db.base(st), span)?;
                        if let Some(dope) = dope {
                            let size = self.size_of(tid, dope);
                            let buf = self.alloc(size);
                            self.emit(Inst::SigLastValue { dst: buf, sig: s });
                            return Ok(V::A(buf, dope));
                        }
                        let (_, rt, w) = self.sig_scalar(st, span)?;
                        let logic = self.is_logic(tid);
                        // A logic leaf arrives as one block of planes.
                        let buf = self.alloc_n(if logic { 32 } else { 8 });
                        self.emit(Inst::SigLastValue { dst: buf, sig: s });
                        if logic {
                            let z = self.int(0);
                            let dst = self.reg(RegTy::I64);
                            self.emit(Inst::LGet {
                                dst,
                                base: buf,
                                idx: z,
                            });
                            let bias = self.code_bias(tid);
                            return Ok(V::S(self.dec(dst, bias)));
                        }
                        return Ok(V::S(self.load(buf, w, rt)));
                    }
                };
                let dst = self.reg(RegTy::I64);
                self.emit(Inst::SigAttr { dst, kind, sig: s });
                Ok(V::S(dst))
            }
            AttrPrefix::Expr(e) if ty::is_array(db, db.base(db.exprs[e].subtype)) => {
                let dope = self.dope_of(e)?;
                let k = dim(self);
                let r = self.dope_range(dope, k, RegTy::I64);
                self.range_attr(attr, r, Some((dope, k)), span)
            }
            AttrPrefix::Expr(_) => Err(self.cx.unsupported(
                span,
                &format!(
                    "attribute '{} of an object (implicit signals, M2 slice 4)",
                    attr.name()
                ),
            )),
            AttrPrefix::Entity(d) if attr == SimpleName => {
                let name = db.decls[d].name.to_string().to_ascii_lowercase();
                let n = name.len() as i64;
                let st = self.string_subtype(span)?;
                let v = ConstValue::Array(yavs_sem::stat::ArrayConst {
                    bounds: Box::new([(1, Direction::To, n)]),
                    elems: Elems::Bytes(name.into_bytes().into_boxed_slice()),
                });
                self.konst(&v, st, span)
            }
            AttrPrefix::Entity(d) if matches!(attr, PathName | InstanceName) => {
                // An item of a library-unit package is named the same in every instance, and the
                // subprogram that reads it is shared by them: a constant, not an instance slot.
                if let Some(name) = crate::pathname::library_package_item_name(db, d) {
                    let st = self.string_subtype(span)?;
                    let v = ConstValue::Array(yavs_sem::stat::ArrayConst {
                        bounds: Box::new([(1, Direction::To, name.len() as i64)]),
                        elems: Elems::Bytes(name.into_bytes().into_boxed_slice()),
                    });
                    return self.konst(&v, st, span);
                }
                let instance = attr == InstanceName;
                let off = match self
                    .cx
                    .names
                    .iter()
                    .find(|n| n.decl == d && n.instance == instance)
                {
                    Some(n) => n.off,
                    None => {
                        let off = self.cx.inst_alloc(16);
                        self.cx.context_funcs.push(self.fb.id());
                        self.cx.names.push(crate::InstName {
                            decl: d,
                            instance,
                            off,
                        });
                        off
                    }
                };
                let (data, dope) = self.inst_array(None, Space::Inst, off);
                Ok(V::A(data, dope))
            }
            AttrPrefix::Entity(_) => Err(self.cx.unsupported(
                span,
                &format!("attribute '{} of a named entity", attr.name()),
            )),
        }
    }

    fn string_subtype(&mut self, span: Span) -> R<SubtypeId> {
        match self.db.well_known.string {
            Some(t) => Ok(self.db.types[t].first_subtype),
            None => Err(self.cx.ice(span, "STRING is not known")),
        }
    }

    fn range_attr(
        &mut self,
        attr: PredefAttr,
        r: Rng,
        dope: Option<(Reg, u8)>,
        span: Span,
    ) -> R<V> {
        use PredefAttr::*;
        Ok(V::S(match attr {
            Left => r.left,
            Right => r.right,
            Low => self.low_high(r).0,
            High => self.low_high(r).1,
            Ascending => {
                let d = self.dir_reg(r.dir);
                let z = self.int(0);
                self.cmp(CmpOp::Eq, d, z)
            }
            Length => match dope {
                Some((dope, dim)) => {
                    let dst = self.reg(RegTy::I64);
                    self.emit(Inst::DopeLen { dst, dope, dim });
                    dst
                }
                None => return Err(self.cx.ice(span, "'length of a scalar")),
            },
            _ => {
                return Err(self
                    .cx
                    .unsupported(span, &format!("attribute '{}", attr.name())));
            }
        }))
    }

    // ---- names ----

    /// Position (0-based from the left, all dimensions flattened) of `indices` in `dope`.
    pub(crate) fn flat_pos(&mut self, dope: Reg, indices: &[ExprId], what: &str) -> R<Reg> {
        let mut acc: Option<Reg> = None;
        for (k, &ix) in indices.iter().enumerate() {
            let i = self.scalar_expr(ix)?;
            let what = self.msg(what, Dir::To);
            let p = self.reg(RegTy::I64);
            self.emit(Inst::Pos {
                dst: p,
                dope,
                dim: k as u8,
                idx: i,
                what,
            });
            acc = Some(match acc {
                None => p,
                Some(a) => {
                    let len = self.reg(RegTy::I64);
                    self.emit(Inst::DopeLen {
                        dst: len,
                        dope,
                        dim: k as u8,
                    });
                    let m = self.bin(IntOp::Mul, a, len);
                    self.bin(IntOp::Add, m, p)
                }
            });
        }
        acc.ok_or(Failed)
    }

    pub(crate) fn prefix_name(&self, e: ExprId) -> String {
        match self.db.exprs[e].kind {
            ExprKind::Obj(d) => self.db.decls[d].name.to_string(),
            _ => "array".into(),
        }
    }

    /// `r.f` (or `r.f.g`) of a record signal `r`: read the selected part alone, as a scalar
    /// signal is read, not the whole record and then select the field (every activation of a
    /// process that names one field of a wide control record copied all of it).
    fn rec_part(&mut self, e: ExprId, span: Span) -> R<Option<V>> {
        let db = self.db;
        let Some((d, steps)) = self.comp_steps(e) else {
            return Ok(None);
        };
        let fi = |s: &[Step<'_>]| {
            s.iter()
                .all(|s| matches!(s, Step::Field(_) | Step::Index(_)))
        };
        let last_slice = match steps.split_last() {
            Some((Step::Slice(_), rest)) => fi(rest),
            _ => false,
        };
        if !fi(&steps) && !last_slice {
            return Ok(None);
        }
        if let Some(v) = self.gather_part(e, d, &steps, span)? {
            return Ok(Some(v));
        }
        // ponytail: slices of an array of records without a leaf table still read all of it.
        if last_slice {
            return Ok(None);
        }
        let p = self.comp_select(d, &steps, span)?;
        let t = db.base(db.exprs[e].subtype);
        let one_leaf = matches!(p.leaves.as_slice(), [(l, _, _)] if l.path == p.path);
        if !one_leaf || (ty::is_array(db, t) && self.is_general(t, span)?) {
            return self.read_comp(&p, span).map(Some);
        }
        let s = p.leaves[0].1;
        if ty::is_array(db, t) {
            let size = self.size_of(p.tid, p.desc);
            let buf = self.alloc(size);
            self.emit(Inst::SigRead { dst: buf, sig: s });
            return Ok(Some(V::A(buf, p.desc)));
        }
        let raw = self.reg(RegTy::I64);
        self.emit(Inst::SigReadScalar { dst: raw, sig: s });
        let bias = self.code_bias(p.tid);
        let dst = self.dec(raw, bias);
        if self.rt_of(t) == RegTy::F64 {
            let f = self.reg(RegTy::F64);
            self.emit(Inst::Move { dst: f, src: dst });
            return Ok(Some(V::S(f)));
        }
        Ok(Some(V::S(dst)))
    }

    /// A part of record signal `d` that is a record or an array of records with more than a
    /// scalar leaf below it, read by one `SigGather` (`None`: not such a part, or no leaf table).
    fn gather_part(
        &mut self,
        e: ExprId,
        d: DeclId,
        steps: &[Step<'_>],
        span: Span,
    ) -> R<Option<V>> {
        let db = self.db;
        let tid = self.ty(db.base(db.exprs[e].subtype), span)?;
        let path: Vec<u32> = steps
            .iter()
            .filter_map(|s| match *s {
                Step::Field(n) => Some(n),
                _ => None,
            })
            .collect();
        let leaves = self.leaves_under(d, &path);
        if matches!(leaves.as_slice(), [l] if l.path == path) || !self.part_value_ok(steps, tid) {
            return Ok(None);
        }
        let Some(tab) = self.table_for(d, &path, tid, span)? else {
            return Ok(None);
        };
        let h = self.comp_head(d, steps, span)?;
        let size = self.size_of(h.tid, h.desc);
        let val = self.alloc(size);
        self.emit(Inst::SigGather {
            dst: val,
            desc: h.desc,
            tab,
            part: h.part,
        });
        Ok(Some(V::A(val, h.desc)))
    }

    /// `s(i)` of an array signal `s` of this unit with scalar elements: read that element
    /// alone (as a scalar signal is read), not the whole signal and then index it. Index
    /// checks and their messages are those of `index`.
    fn sig_elem(&mut self, e: ExprId, prefix: ExprId, span: Span) -> R<Option<V>> {
        let db = self.db;
        let (d, nested) = match db.exprs[prefix].kind {
            ExprKind::Obj(d) => (d, false),
            // `s(i)(j)` of an array of arrays: the element of the element, alone.
            _ => match self.nested_steps(prefix) {
                Some((d, _)) => (d, true),
                None => return Ok(None),
            },
        };
        let et = db.base(db.exprs[e].subtype);
        if !matches!(self.cx.objs.get(&d), Some(Obj::Signal { .. }))
            || !db.types[et].kind.is_scalar()
            || (!nested && self.is_general(db.base(db.exprs[prefix].subtype), span)?)
        {
            return Ok(None);
        }
        let (_, s, _) = self.sig_name(e)?;
        let raw = self.reg(RegTy::I64);
        self.emit(Inst::SigReadScalar { dst: raw, sig: s });
        let tid = self.ty(et, span)?;
        let bias = self.code_bias(tid);
        let dst = self.dec(raw, bias);
        if self.rt_of(et) == RegTy::F64 {
            let f = self.reg(RegTy::F64);
            self.emit(Inst::Move { dst: f, src: dst });
            return Ok(Some(V::S(f)));
        }
        Ok(Some(V::S(dst)))
    }

    /// `s(i)` or `s(i)(a to b)` of an array-of-arrays signal `s` (no records): read the part
    /// the name selects, not the whole signal and then select (#436).
    fn sig_part(&mut self, e: ExprId, span: Span) -> R<Option<V>> {
        let db = self.db;
        let Some((d, _)) = self.nested_steps(e) else {
            return Ok(None);
        };
        let t = db.base(db.exprs[e].subtype);
        let root_t = db.base(ty::obj_subtype(db, d).ok_or(Failed)?);
        if !matches!(self.cx.objs.get(&d), Some(Obj::Signal { .. }))
            || !ty::is_array(db, t)
            || self.is_general(t, span)?
            || !self.is_general(root_t, span)?
        {
            return Ok(None);
        }
        let (_, s, dope) = self.sig_name(e)?;
        let Some(dope) = dope else {
            return Ok(None);
        };
        let tid = self.ty(t, span)?;
        let size = self.size_of(tid, dope);
        let buf = self.alloc(size);
        self.emit(Inst::SigRead { dst: buf, sig: s });
        Ok(Some(V::A(buf, dope)))
    }

    /// `s(h downto l)` of a signal `s` of a plain array type: read the slice, not the whole
    /// signal and then copy the slice out of it.
    fn sig_slice(&mut self, e: ExprId, prefix: ExprId, span: Span) -> R<Option<V>> {
        let db = self.db;
        let ExprKind::Obj(d) = &db.exprs[prefix].kind else {
            return Ok(None);
        };
        let t = db.base(db.exprs[e].subtype);
        if !matches!(
            self.cx.objs.get(d),
            Some(Obj::Signal { .. } | Obj::SignalRef { .. })
        ) || !ty::is_array(db, t)
            || self.is_general(t, span)?
        {
            return Ok(None);
        }
        let (_, s, dope) = self.sig_name(e)?;
        let Some(dope) = dope else {
            return Ok(None);
        };
        let tid = self.ty(t, span)?;
        let size = self.size_of(tid, dope);
        let buf = self.alloc(size);
        self.emit(Inst::SigRead { dst: buf, sig: s });
        Ok(Some(V::A(buf, dope)))
    }

    /// Whether `e` is an index or slice of an array variable, parameter or constant, possibly
    /// nested: a name [`Fx::place`] selects without copying.
    fn is_view_name(&self, e: ExprId) -> bool {
        let mut e = e;
        let mut steps = 0;
        loop {
            match &self.db.exprs[e].kind {
                ExprKind::Index { prefix, .. } | ExprKind::Slice { prefix, .. } => {
                    e = *prefix;
                    steps += 1;
                }
                ExprKind::Obj(d) => {
                    return steps > 0 && matches!(self.cx.objs.get(d), Some(Obj::Array { .. }));
                }
                _ => return false,
            }
        }
    }

    /// Plane index of element `pos` (from the left) of a logic array with `dope` seen through
    /// the plane offset `off` (a slice or row of a larger array).
    pub(crate) fn logic_elem_idx(
        &mut self,
        tid: TyId,
        dope: Reg,
        pos: Reg,
        off: Option<Reg>,
    ) -> Reg {
        let count = self.layout_count(tid, dope);
        let one = self.int(1);
        let last = self.bin(IntOp::Sub, count, one);
        let idx = self.bin(IntOp::Sub, last, pos);
        match off {
            Some(o) => self.bin(IntOp::Add, idx, o),
            None => idx,
        }
    }

    fn index(&mut self, prefix: ExprId, indices: &[ExprId], span: Span) -> R<V> {
        let db = self.db;
        let t = db.base(db.exprs[prefix].subtype);
        if self.is_general(t, span)? {
            return self.index_comp(prefix, indices, span);
        }
        let (_, logic, w) = self.elem_facts(t, span)?;
        let what = self.prefix_name(prefix);
        // `a(i)(j)` of a logic row: the element in place, not through a copy of row `a(i)`.
        if logic && self.is_view_name(prefix) {
            let tid = self.ty(t, span)?;
            if let Place::Array {
                data, dope, off, ..
            } = self.place(prefix)?
            {
                let pos = self.flat_pos(dope, indices, &what)?;
                let idx = self.logic_elem_idx(tid, dope, pos, off);
                let dst = self.reg(RegTy::I64);
                self.emit(Inst::LGet {
                    dst,
                    base: data,
                    idx,
                });
                let bias = self.code_bias(tid);
                return Ok(V::S(self.dec(dst, bias)));
            }
            return Err(self
                .cx
                .ice(span, "view of a variable array is not an array"));
        }
        let v = self.expr(prefix)?;
        let (data, dope) = self.arr(v, span)?;
        let pos = self.flat_pos(dope, indices, &what)?;
        let tid = self.ty(t, span)?;
        if logic {
            let count = self.layout_count(tid, dope);
            let code = self.elem_at(data, pos, count, true, w);
            let bias = self.code_bias(tid);
            return Ok(V::S(self.dec(code, bias)));
        }
        let rt = self.rt_of(db.base(ty::array_of(db, t).ok_or(Failed)?.0));
        let a = self.reg(RegTy::Ptr);
        self.emit(Inst::Index {
            dst: a,
            base: data,
            pos,
            stride: w.bytes(),
        });
        Ok(V::S(self.load(a, w, rt)))
    }

    /// Start position, length and descriptor of slice `range` of an array with `dope`.
    pub(crate) fn slice_of(
        &mut self,
        dope: Reg,
        range: &RangeC,
        what: &str,
        span: Span,
    ) -> R<(Reg, Reg, Reg)> {
        let r = self.range(range, span)?;
        let dir = self.dir_reg(r.dir);
        let nd = self.one_dope(r.left, r.right, dir);
        let n = self.dope_len(nd);
        let p = self.reg(RegTy::I64);
        let what_id = self.msg(what, Dir::To);
        let dir_what = self.msg(&format!("direction of the slice of {what}"), Dir::To);
        self.emit(Inst::SlicePos {
            dst: p,
            dope,
            left: r.left,
            right: r.right,
            dir,
            what: what_id,
            dir_what,
        });
        Ok((p, n, nd))
    }

    fn slice(&mut self, prefix: ExprId, range: &RangeC, span: Span) -> R<V> {
        let db = self.db;
        let t = db.base(db.exprs[prefix].subtype);
        if self.is_general(t, span)? {
            return self.slice_comp(prefix, range, span);
        }
        let (tid, logic, w) = self.elem_facts(t, span)?;
        let what = self.prefix_name(prefix);
        let v = self.expr(prefix)?;
        let (data, dope) = self.arr(v, span)?;
        let (p, n, nd) = self.slice_of(dope, range, &what, span)?;
        if logic {
            let count = self.dope_len(dope);
            let a = self.bin(IntOp::Sub, count, p);
            let src_off = self.bin(IntOp::Sub, a, n);
            let size = self.size_of(tid, nd);
            let dst = self.alloc(size);
            let z = self.int(0);
            self.emit(Inst::LCopy {
                dst,
                dst_off: z,
                src: data,
                src_off,
                n,
            });
            return Ok(V::A(dst, nd));
        }
        let a = self.reg(RegTy::Ptr);
        self.emit(Inst::Index {
            dst: a,
            base: data,
            pos: p,
            stride: w.bytes(),
        });
        Ok(V::A(a, nd))
    }

    /// The target of a variable assignment or an `out` argument.
    pub fn place(&mut self, e: ExprId) -> R<Place> {
        let db = self.db;
        let x = &db.exprs[e];
        let span = x.span;
        match &x.kind {
            ExprKind::Obj(d) => {
                let Some(st) = ty::obj_subtype(db, *d) else {
                    return Err(self.cx.ice(span, "assignment to a non-object"));
                };
                match self.cx.objs.get(d).copied() {
                    Some(Obj::Scalar { space, off, w }) => {
                        let addr = self.addr(space, off);
                        Ok(Place::Scalar { addr, w, st })
                    }
                    Some(Obj::ScalarRef { space, off, w }) => {
                        let a = self.addr(space, off);
                        let addr = self.load(a, Width::B8, RegTy::Ptr);
                        Ok(Place::Scalar { addr, w, st })
                    }
                    Some(Obj::Array { space, off }) => {
                        let a = self.addr(space, off);
                        let data = self.load(a, Width::B8, RegTy::Ptr);
                        let dope = self.array_dope(*d, a);
                        let t = db.base(st);
                        let tid = self.ty(t, span)?;
                        let dims = ty::array_of(db, t).map_or(0, |x| x.1);
                        Ok(Place::Array {
                            data,
                            dope,
                            off: None,
                            tid,
                            dims,
                        })
                    }
                    // An alias without storage: the aliased name, an array seen with the
                    // alias subtype's bounds.
                    None if self.alias_target(*d).is_some() => {
                        let (t, ast) = self.alias_target(*d).ok_or(Failed)?;
                        match self.place(t)? {
                            Place::Array {
                                data,
                                dope,
                                off,
                                tid,
                                dims,
                            } => {
                                let what = db.decls[*d].name.to_string();
                                let dope = self.dope_as(dope, ast, &what, span)?;
                                Ok(Place::Array {
                                    data,
                                    dope,
                                    off,
                                    tid,
                                    dims,
                                })
                            }
                            p => Ok(p),
                        }
                    }
                    None if matches!(&db.decls[*d].kind, DeclKind::Object(o) if o.class == yavs_sem::decl::ObjClass::File) =>
                    {
                        let addr = self.file_cell(*d, span)?;
                        Ok(Place::Scalar {
                            addr,
                            w: Width::B4,
                            st,
                        })
                    }
                    _ => Err(self.cx.unsupported(span, "this assignment target")),
                }
            }
            ExprKind::Index { prefix, indices } => {
                let t = db.base(db.exprs[*prefix].subtype);
                if self.is_general(t, span)? {
                    return self.index_place_comp(*prefix, indices, span);
                }
                let (elem, _) = ty::array_of(db, t).ok_or(Failed)?;
                let what = self.prefix_name(*prefix);
                let Place::Array {
                    data,
                    dope,
                    off,
                    tid,
                    ..
                } = self.place(*prefix)?
                else {
                    return Err(self.cx.ice(span, "indexed non-array"));
                };
                let pos = self.flat_pos(dope, indices, &what)?;
                if self.is_logic(tid) {
                    let idx = self.logic_elem_idx(tid, dope, pos, off);
                    return Ok(Place::Logic {
                        data,
                        idx,
                        st: elem,
                    });
                }
                let (_, _, w) = self.elem_scalar(elem, span)?;
                let addr = self.reg(RegTy::Ptr);
                self.emit(Inst::Index {
                    dst: addr,
                    base: data,
                    pos,
                    stride: w.bytes(),
                });
                Ok(Place::Scalar { addr, w, st: elem })
            }
            ExprKind::Slice { prefix, range } => {
                if self.is_general(db.base(db.exprs[*prefix].subtype), span)? {
                    return self.slice_place_comp(*prefix, range, span);
                }
                let what = self.prefix_name(*prefix);
                let Place::Array {
                    data,
                    dope,
                    off,
                    tid,
                    dims,
                } = self.place(*prefix)?
                else {
                    return Err(self.cx.ice(span, "sliced non-array"));
                };
                let (p, n, nd) = self.slice_of(dope, range, &what, span)?;
                if self.is_logic(tid) {
                    let count = self.dope_len(dope);
                    let a = self.bin(IntOp::Sub, count, p);
                    let mut o = self.bin(IntOp::Sub, a, n);
                    if let Some(b) = off {
                        o = self.bin(IntOp::Add, o, b);
                    }
                    return Ok(Place::Array {
                        data,
                        dope: nd,
                        off: Some(o),
                        tid,
                        dims,
                    });
                }
                let t = db.base(db.exprs[*prefix].subtype);
                let (_, _, w) = self.elem_facts(t, span)?;
                let a = self.reg(RegTy::Ptr);
                self.emit(Inst::Index {
                    dst: a,
                    base: data,
                    pos: p,
                    stride: w.bytes(),
                });
                Ok(Place::Array {
                    data: a,
                    dope: nd,
                    off: None,
                    tid,
                    dims,
                })
            }
            ExprKind::Field { prefix, elem } => self.field_place(*prefix, *elem, span),
            ExprKind::Deref { prefix } => self.deref_place(*prefix, span),
            _ => Err(self.cx.unsupported(span, "aggregate targets")),
        }
    }

    /// Store `v` (of subtype `from`) into `p`, with the implicit subtype conversion checks.
    pub fn store_place(
        &mut self,
        p: Place,
        v: V,
        from: SubtypeId,
        what: &str,
        span: Span,
    ) -> R<()> {
        match p {
            Place::Scalar { addr, w, st } => {
                let r = self.scal(v, span)?;
                self.check_scalar(r, Some(from), st, what, span)?;
                self.store(addr, r, w);
            }
            Place::Logic { data, idx, st } => {
                let r = self.scal(v, span)?;
                self.check_scalar(r, Some(from), st, what, span)?;
                let tid = self.ty(self.db.base(st), span)?;
                let bias = self.code_bias(tid);
                let r = self.enc(r, bias);
                self.emit(Inst::LSet {
                    base: data,
                    idx,
                    v: r,
                });
            }
            Place::Array {
                data,
                dope,
                off,
                tid,
                dims,
            } => {
                let (vd, vp) = self.arr(v, span)?;
                let m = self.msg(what, Dir::To);
                self.emit(Inst::SubtypeCheck {
                    dope: vp,
                    want: dope,
                    dims,
                    what: m,
                });
                let (size, count) = self.layout(tid, dope);
                if self.is_logic(tid) {
                    let z = self.int(0);
                    let dst_off = off.unwrap_or(z);
                    self.emit(Inst::LCopy {
                        dst: data,
                        dst_off,
                        src: vd,
                        src_off: z,
                        n: count,
                    });
                } else {
                    self.emit(Inst::Copy {
                        dst: data,
                        src: vd,
                        bytes: size,
                    });
                }
            }
        }
        Ok(())
    }

    // ---- conversions ----

    fn qualified(&mut self, to: SubtypeId, operand: ExprId, span: Span) -> R<V> {
        let db = self.db;
        let v = self.expr(operand)?;
        match v {
            V::S(r) => {
                self.check_scalar(
                    r,
                    Some(db.exprs[operand].subtype),
                    to,
                    &ty::subtype_name(db, to),
                    span,
                )?;
                Ok(v)
            }
            V::A(d, p) => {
                let Some(want) = self.subtype_dope(to, span)? else {
                    return Ok(v);
                };
                let dims = ty::array_of(db, db.base(to)).map_or(0, |x| x.1);
                let what = self.msg(&ty::subtype_name(db, to), Dir::To);
                self.emit(Inst::SubtypeCheck {
                    dope: p,
                    want,
                    dims,
                    what,
                });
                Ok(V::A(d, want))
            }
        }
    }

    fn conv(&mut self, to: SubtypeId, operand: ExprId, span: Span) -> R<V> {
        let db = self.db;
        let from = db.exprs[operand].subtype;
        let (ft, tt) = (db.base(from), db.base(to));
        if ty::is_array(db, tt) {
            let (fe, _) = ty::array_of(db, ft).ok_or(Failed)?;
            let (te, _) = ty::array_of(db, tt).ok_or(Failed)?;
            if db.base(fe) != db.base(te) {
                return self.conv_elements(to, operand, fe, te, span);
            }
            let v = self.qualified(to, operand, span)?;
            // An unconstrained target takes the operand's bounds, which must belong to its
            // index subtypes (LRM 9.3.6).
            if let V::A(_, p) = v
                && !self.is_general(tt, span)?
                && !self.is_general(ft, span)?
                && !ty::index_constrained(db, to)
                && let (Some(fi), Some(ti)) = (index_subtypes(db, ft), index_subtypes(db, tt))
            {
                for (dim, (&f, &t)) in fi.iter().zip(ti.iter()).enumerate() {
                    let inside = match (db.static_bounds(f), db.static_bounds(t)) {
                        (Some((fl, fd, fr)), Some((tl, td, tr))) => {
                            match (fl.as_int(), fr.as_int(), tl.as_int(), tr.as_int()) {
                                (Some(fl), Some(fr), Some(tl), Some(tr)) => {
                                    let lh =
                                        |l, d, r| if d == Direction::To { (l, r) } else { (r, l) };
                                    let ((flo, fhi), (tlo, thi)) = (lh(fl, fd, fr), lh(tl, td, tr));
                                    flo > fhi || (tlo <= flo && fhi <= thi)
                                }
                                _ => false,
                            }
                        }
                        _ => false,
                    };
                    if f != t && !inside {
                        let r = self.dope_range(p, dim as u8, RegTy::I64);
                        self.check_range_in(r, t, "index constraint", span)?;
                    }
                }
            }
            return Ok(v);
        }
        if matches!(db.types[tt].kind, TypeKind::Record { .. }) {
            return self.conv_record(to, operand, span);
        }
        let v = self.scalar_expr(operand)?;
        Ok(V::S(self.conv_scalar(v, from, to, span)?))
    }

    /// Scalar `v` of subtype `from` converted to subtype `to` (LRM 9.3.6).
    pub(crate) fn conv_scalar(
        &mut self,
        v: Reg,
        from: SubtypeId,
        to: SubtypeId,
        span: Span,
    ) -> R<Reg> {
        let db = self.db;
        let (ft, tt) = (db.base(from), db.base(to));
        let name = ty::subtype_name(db, to);
        let r = match (ty::is_real(db, ft), ty::is_real(db, tt)) {
            (true, false) => {
                let r = self.scalar_bounds(to, span)?;
                let dir = ir_dir(r.dir);
                let (lo, hi) = self.low_high(r);
                let what = self.fb.program().check_msg_for("", dir, &name);
                let dst = self.reg(RegTy::I64);
                self.emit(Inst::FToIRange {
                    dst,
                    src: v,
                    lo,
                    hi,
                    what,
                });
                dst
            }
            (false, true) => {
                let dst = self.reg(RegTy::F64);
                self.emit(Inst::IToF { dst, src: v });
                dst
            }
            (true, true) => v,
            (false, false) => {
                self.check_scalar(v, Some(from), to, "", span)?;
                v
            }
        };
        Ok(r)
    }

    fn conditional(
        &mut self,
        alts: &[(ExprId, Option<ExprId>)],
        st: SubtypeId,
        _span: Span,
    ) -> R<V> {
        let db = self.db;
        let comp = self.is_comp(db.base(st));
        let dst = self.reg(if comp {
            RegTy::Ptr
        } else {
            self.rt_of(db.base(st))
        });
        let dst2 = self.reg(RegTy::Ptr);
        let j = self.block();
        for &(v, c) in alts {
            let next = self.block();
            if let Some(c) = c {
                let c = self.scalar_expr(c)?;
                let t = self.block();
                self.branch(c, t, next);
                self.at(t);
            }
            match self.expr(v)? {
                V::S(r) => self.emit(Inst::Move { dst, src: r }),
                V::A(d, p) => {
                    self.emit(Inst::Move { dst, src: d });
                    self.emit(Inst::Move { dst: dst2, src: p });
                }
            }
            self.jump(j);
            self.at(next);
            if c.is_none() {
                break;
            }
        }
        if self.live {
            let what = self.msg(
                "no condition of the conditional expression is true",
                Dir::To,
            );
            self.term(Inst::Unreachable { what });
        }
        self.at(j);
        Ok(if comp { V::A(dst, dst2) } else { V::S(dst) })
    }

    /// Conversion between closely related array types whose element types differ (LRM
    /// 9.3.6): element by element, each converted like a scalar.
    fn conv_elements(
        &mut self,
        to: SubtypeId,
        operand: ExprId,
        fe: SubtypeId,
        te: SubtypeId,
        span: Span,
    ) -> R<V> {
        let db = self.db;
        let (ft, tt) = (db.base(db.exprs[operand].subtype), db.base(to));
        let v = self.expr(operand)?;
        let (sd, sp) = self.arr(v, span)?;
        let (stid, slogic, sw) = self.elem_facts(ft, span)?;
        let (ttid, tlogic, tw) = self.elem_facts(tt, span)?;
        if ty::array_of(db, tt).map_or(1, |x| x.1) != 1 {
            return Err(self
                .cx
                .unsupported(span, "multi-dimensional array conversions"));
        }
        let dope = match self.subtype_dope(to, span)? {
            Some(want) => {
                let what = self.msg(&ty::subtype_name(db, to), Dir::To);
                self.emit(Inst::SubtypeCheck {
                    dope: sp,
                    want,
                    dims: 1,
                    what,
                });
                want
            }
            None => sp,
        };
        let (size, count) = self.layout(ttid, dope);
        let data = self.alloc_val(size, ttid, Some(dope));
        let (sb, tb) = (self.code_bias(stid), self.code_bias(ttid));
        let (sreal, treal) = (ty::is_real(db, db.base(fe)), ty::is_real(db, db.base(te)));
        let name = ty::subtype_name(db, te);
        self.for_n(count, |fx, i| {
            let e = if slogic {
                let c = fx.elem_at(sd, i, count, true, sw);
                fx.dec(c, sb)
            } else {
                let a = fx.reg(RegTy::Ptr);
                fx.emit(Inst::Index {
                    dst: a,
                    base: sd,
                    pos: i,
                    stride: sw.bytes(),
                });
                fx.load(a, sw, if sreal { RegTy::F64 } else { RegTy::I64 })
            };
            let x = match (sreal, treal) {
                (true, false) => {
                    let r = fx.scalar_bounds(te, span)?;
                    let dir = ir_dir(r.dir);
                    let (lo, hi) = fx.low_high(r);
                    let what = fx.fb.program().check_msg_for("", dir, &name);
                    let dst = fx.reg(RegTy::I64);
                    fx.emit(Inst::FToIRange {
                        dst,
                        src: e,
                        lo,
                        hi,
                        what,
                    });
                    dst
                }
                (false, true) => {
                    let dst = fx.reg(RegTy::F64);
                    fx.emit(Inst::IToF { dst, src: e });
                    dst
                }
                _ => {
                    fx.check_scalar(e, Some(fe), te, "", span)?;
                    e
                }
            };
            if tlogic {
                let one = fx.int(1);
                let last = fx.bin(IntOp::Sub, count, one);
                let idx = fx.bin(IntOp::Sub, last, i);
                let c = fx.enc(x, tb);
                fx.emit(Inst::LSet {
                    base: data,
                    idx,
                    v: c,
                });
            } else {
                let a = fx.reg(RegTy::Ptr);
                fx.emit(Inst::Index {
                    dst: a,
                    base: data,
                    pos: i,
                    stride: tw.bytes(),
                });
                fx.store(a, x, tw);
            }
            Ok(())
        })?;
        Ok(V::A(data, dope))
    }
}

/// Which operands of a `&` form are elements: (left, right).
fn cat_scalars(form: Form) -> (bool, bool) {
    match form {
        Form::ArrayScalar => (false, true),
        Form::ScalarArray => (true, false),
        Form::ScalarScalar => (true, true),
        _ => (false, false),
    }
}

fn cmp_op(f: Family) -> CmpOp {
    match f {
        Family::Eq => CmpOp::Eq,
        Family::Neq => CmpOp::Ne,
        Family::Lt => CmpOp::Lt,
        Family::Le => CmpOp::Le,
        Family::Gt => CmpOp::Gt,
        _ => CmpOp::Ge,
    }
}

/// The full declaration in the package body of deferred constant `d`.
pub fn deferred_completion(db: &Db, d: DeclId) -> Option<DeclId> {
    let decl = &db.decls[d];
    let key = decl.name.key();
    let full = |scope, d| {
        db.scopes[scope]
            .names
            .get(&key)?
            .iter()
            .copied()
            .find(|&c| {
                c != d
                    && matches!(&db.decls[c].kind, DeclKind::Object(o)
                if o.class == yavs_sem::decl::ObjClass::Constant && o.default.is_some())
            })
    };
    // An instance of a generic package holds a copy of the body's declarations in its body
    // region (cloned when the body is first needed).
    if let Some(body) = db
        .insts
        .iter()
        .find_map(|(_, i)| (i.scope == decl.scope).then(|| i.body.get().copied()))
        .flatten()
        && let Some(c) = full(body, d)
    {
        return Some(c);
    }
    db.units.iter().find_map(|(_, u)| {
        let UnitHir::PackageBody { package } = u.hir else {
            return None;
        };
        if db.units[package].scope != decl.scope {
            return None;
        }
        db.scopes[u.scope]
            .names
            .get(&key)?
            .iter()
            .copied()
            .find(|&c| {
                c != d
                    && matches!(&db.decls[c].kind, DeclKind::Object(o)
                    if o.class == yavs_sem::decl::ObjClass::Constant && o.default.is_some())
            })
    })
}

/// The index subtypes of array type `t`.
pub(crate) fn index_subtypes(db: &Db, t: yavs_sem::TypeId) -> Option<Vec<SubtypeId>> {
    match &db.types[t].kind {
        TypeKind::Array { indexes, .. } => Some(indexes.to_vec()),
        _ => None,
    }
}
