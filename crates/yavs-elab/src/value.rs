//! Composite elaboration-time values (#77): array and record values as `Val`, and their
//! marshalling to and from interpreter memory for elaboration-time calls (`call_values`).
//!
//! ponytail: slices, concatenations and aggregates expand runs into one `Val` per element;
//! fine for elaboration-time values (generics, constants, initial values); keep runs if a
//! design folds huge aggregates here.

use yavs_interp::CallVal;
use yavs_ir::{DOPE_DIM_BYTES, LogicKind, Program, Ty, TyId};
use yavs_sem::stat::{ArrayConst, Elems};
use yavs_sem::types::Direction;

use crate::eval::Val;
use crate::hier::Bounds;

pub(crate) fn bounds_of(a: &ArrayConst) -> Vec<Bounds> {
    a.bounds
        .iter()
        .map(|&(left, dir, right)| Bounds { left, dir, right })
        .collect()
}

/// The elements of `a` in row-major order.
pub(crate) fn elems(a: &ArrayConst) -> Vec<Val> {
    match &a.elems {
        Elems::Bytes(b) => b.iter().map(|&x| Val::Enum(u32::from(x))).collect(),
        Elems::Runs(rs) => rs
            .iter()
            .flat_map(|(n, x)| std::iter::repeat_n(x.clone(), *n as usize))
            .collect(),
    }
}

/// Element `k` (row major) of `a`, without expanding runs.
pub(crate) fn elem_at(a: &ArrayConst, mut k: u64) -> Option<Val> {
    match &a.elems {
        Elems::Bytes(b) => b.get(k as usize).map(|&x| Val::Enum(u32::from(x))),
        Elems::Runs(rs) => {
            for (n, x) in rs.iter() {
                if k < *n {
                    return Some(x.clone());
                }
                k -= n;
            }
            None
        }
    }
}

/// An array value: bytes for 1-D arrays of small enumeration positions, else runs.
pub(crate) fn make(bounds: &[Bounds], xs: Vec<Val>) -> Val {
    let bounds: Box<[_]> = bounds.iter().map(|b| (b.left, b.dir, b.right)).collect();
    let small = |x: &Val| matches!(x, Val::Enum(p) if *p < 256);
    let elems = if bounds.len() == 1 && !xs.is_empty() && xs.iter().all(small) {
        Elems::Bytes(xs.iter().map(|x| x.as_int().unwrap_or(0) as u8).collect())
    } else {
        let mut runs: Vec<(u64, Val)> = Vec::new();
        for x in xs {
            match runs.last_mut() {
                Some((n, y)) if *y == x => *n += 1,
                _ => runs.push((1, x)),
            }
        }
        Elems::Runs(runs.into())
    };
    Val::Array(ArrayConst { bounds, elems })
}

/// Bounds of `n` elements starting at `left` in direction `dir`.
pub(crate) fn span(left: i64, dir: Direction, n: u64) -> Bounds {
    let n = n as i64;
    Bounds {
        left,
        dir,
        right: match dir {
            Direction::To => left + n - 1,
            Direction::Downto => left - n + 1,
        },
    }
}

fn scalar_cell(v: &Val) -> Option<u64> {
    match v {
        Val::Int(x) | Val::Phys(x) => Some(*x as u64),
        Val::Enum(p) => Some(u64::from(*p)),
        Val::Real(r) => Some(r.to_bits()),
        _ => None,
    }
}

fn scalar_val(t: &Ty, c: u64) -> Option<Val> {
    Some(match t {
        Ty::Enum { .. } => Val::Enum(c as u32),
        Ty::Int { .. } => Val::Int(c as i64),
        Ty::Physical { .. } => Val::Phys(c as i64),
        Ty::Real => Val::Real(f64::from_bits(c)),
        _ => return None,
    })
}

fn read(b: &[u8], at: usize, n: usize) -> Option<u64> {
    let mut w = [0u8; 8];
    w[..n].copy_from_slice(b.get(at..at + n)?);
    Some(u64::from_le_bytes(w))
}

/// `v` of IR type `ty` as a call argument: scalars as a cell, arrays of scalars and records
/// of scalars as data and descriptor bytes. `None` for other values.
pub(crate) fn encode(prog: &Program, ty: TyId, v: &Val) -> Option<CallVal> {
    let t = prog.types.get(ty)?;
    if t.scalar_size().is_some() {
        return scalar_cell(v).map(CallVal::Cell);
    }
    match (t, v) {
        (Ty::Array { .. }, Val::Array(_)) => {
            let (data, desc) = yavs_lower::encode_array(prog, ty, v)?;
            Some(CallVal::Composite { data, desc })
        }
        (Ty::Record { .. }, Val::Record(_)) => {
            let (l, _) = Ty::layout(&prog.types, &prog.consts, ty, &[]).ok()?;
            let mut data = vec![0u8; l.size as usize];
            put_record(prog, ty, v, &mut data)?;
            Some(CallVal::Composite {
                data,
                desc: Vec::new(),
            })
        }
        _ => None,
    }
}

/// Write record value `v` of IR type `ty` (fields scalars or records) into `data`.
fn put_record(prog: &Program, ty: TyId, v: &Val, data: &mut [u8]) -> Option<()> {
    let (Ty::Record { fields }, Val::Record(vs)) = (prog.types.get(ty)?, v) else {
        return None;
    };
    if vs.len() != fields.len() {
        return None;
    }
    let (l, _) = Ty::layout(&prog.types, &prog.consts, ty, &[]).ok()?;
    for ((f, x), off) in fields.iter().zip(vs.iter()).zip(&l.offsets) {
        let off = *off as usize;
        match prog.types.get(f.ty)?.scalar_size() {
            Some(n) => {
                let c = scalar_cell(x)?;
                data.get_mut(off..off + n as usize)?
                    .copy_from_slice(&c.to_le_bytes()[..n as usize]);
            }
            None => put_record(prog, f.ty, x, data.get_mut(off..)?)?,
        }
    }
    Some(())
}

/// The record value of IR type `ty` (fields scalars or records) in `data`.
fn get_record(prog: &Program, ty: TyId, data: &[u8]) -> Option<Val> {
    let Ty::Record { fields } = prog.types.get(ty)? else {
        return None;
    };
    let (l, _) = Ty::layout(&prog.types, &prog.consts, ty, &[]).ok()?;
    let mut vs = Vec::with_capacity(fields.len());
    for (f, off) in fields.iter().zip(&l.offsets) {
        let ft = prog.types.get(f.ty)?;
        let off = *off as usize;
        vs.push(match ft.scalar_size() {
            Some(n) => scalar_val(ft, read(data, off, n as usize)?)?,
            None => get_record(prog, f.ty, data.get(off..)?)?,
        });
    }
    Some(Val::Record(vs.into()))
}

/// The value of a call result of IR type `ty` (the inverse of [`encode`]).
pub(crate) fn decode(prog: &Program, ty: TyId, r: &CallVal) -> Option<Val> {
    let t = prog.types.get(ty)?;
    let (data, desc) = match r {
        CallVal::Cell(c) => return scalar_val(t, *c),
        CallVal::Composite { data, desc } => (data, desc),
    };
    match t {
        Ty::Array { elem, dims, .. } => {
            let et = prog.types.get(*elem)?;
            let w = et.scalar_size()? as usize;
            let mut bounds = Vec::new();
            for d in 0..*dims as usize {
                let at = d * DOPE_DIM_BYTES as usize;
                let (l, r, dir) = (
                    read(desc, at, 8)?,
                    read(desc, at + 8, 8)?,
                    read(desc, at + 16, 8)?,
                );
                let dir = if dir == 0 {
                    Direction::To
                } else {
                    Direction::Downto
                };
                bounds.push(Bounds {
                    left: l as i64,
                    dir,
                    right: r as i64,
                });
            }
            let n = bounds.iter().map(|b| b.len()).product::<u64>() as usize;
            let mut xs = Vec::with_capacity(n);
            match Ty::logic_leaf(&prog.types, ty) {
                Some(k) => {
                    let bias = if k == LogicKind::StdULogic { 0 } else { 2 };
                    for p in 0..n {
                        let i = n - 1 - p; // element 0 is the rightmost
                        let mut code = 0;
                        for plane in 0..4 {
                            let word = read(data, (i / 64) * 32 + plane * 8, 8)?;
                            code |= (word >> (i % 64) & 1) << plane;
                        }
                        xs.push(Val::Enum((code as u32).checked_sub(bias)?));
                    }
                }
                None => {
                    for p in 0..n {
                        xs.push(scalar_val(et, read(data, p * w, w)?)?);
                    }
                }
            }
            Some(make(&bounds, xs))
        }
        Ty::Record { .. } if desc.is_empty() => get_record(prog, ty, data),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Composite expressions.

use yavs_diag::Diagnostic;
use yavs_sem::decl::Body;
use yavs_sem::hir::{Actual, AggAssoc, AggBounds, Choice, ExprKind};
use yavs_sem::implicit::Family;
use yavs_sem::types::TypeKind;
use yavs_sem::{Db, ExprId};

use crate::codes;
use crate::eval::{Model, R, err, int};
use crate::hier::{ScopeId, Shape};

fn not_yet(db: &Db, e: ExprId, what: &str) -> Box<Diagnostic> {
    Box::new(
        Diagnostic::error(
            codes::NOT_EVALUATED,
            format!("this expression cannot be evaluated during elaboration yet: {what}"),
        )
        .with_primary(db.exprs[e].span, "evaluated during elaboration")
        .with_note("composite values are evaluated for one-dimensional aggregates, records, indexing, slices, concatenation and calls with arrays or records of scalars (#77)"),
    )
}

fn fails(db: &Db, e: ExprId, why: String) -> Box<Diagnostic> {
    err(
        codes::EVAL_FAILED,
        db.exprs[e].span,
        format!("evaluation fails during elaboration: {why}"),
        "here",
    )
}

fn is_composite(db: &Db, e: ExprId) -> bool {
    matches!(
        db.types[db.base(db.exprs[e].subtype)].kind,
        TypeKind::Array { .. } | TypeKind::Record { .. }
    )
}

impl Model {
    /// Composite expressions `Db::fold_with` does not fold; `None` when `e` is none of them.
    pub(crate) fn composite(
        &mut self,
        db: &Db,
        e: ExprId,
        scope: Option<ScopeId>,
    ) -> R<Option<Val>> {
        let ex = &db.exprs[e];
        Ok(Some(match &ex.kind {
            ExprKind::Aggregate { assocs, bounds } => {
                self.aggregate(db, e, assocs, bounds, scope)?
            }
            ExprKind::Index { prefix, indices } if is_composite(db, *prefix) => {
                let Val::Array(a) = self.eval(db, *prefix, scope)? else {
                    return Err(not_yet(db, e, "the prefix is not an array value"));
                };
                let bs = bounds_of(&a);
                let mut k = 0u64;
                for (b, &i) in bs.iter().zip(indices.iter()) {
                    let x = int(&self.eval(db, i, scope)?, db.exprs[i].span)?;
                    let p = b.pos(x).ok_or_else(|| {
                        fails(
                            db,
                            i,
                            format!("index {x} is outside {} {:?} {}", b.left, b.dir, b.right),
                        )
                    })?;
                    k = k * b.len() + p;
                }
                elem_at(&a, k).ok_or_else(|| fails(db, e, "index outside the value".into()))?
            }
            ExprKind::Slice { prefix, range } => {
                let Val::Array(a) = self.eval(db, *prefix, scope)? else {
                    return Err(not_yet(db, e, "the prefix is not an array value"));
                };
                let r = self.range(db, range, scope, ex.span)?;
                let b = bounds_of(&a)[0];
                if r.is_empty() {
                    return Ok(Some(make(&[r], Vec::new())));
                }
                let (Some(p), Some(q)) = (b.pos(r.left), b.pos(r.right)) else {
                    return Err(fails(
                        db,
                        e,
                        "slice outside the bounds of its prefix".into(),
                    ));
                };
                if r.dir != b.dir {
                    return Err(fails(
                        db,
                        e,
                        "slice direction differs from its prefix".into(),
                    ));
                }
                let xs = elems(&a)[p as usize..=q as usize].to_vec();
                make(&[r], xs)
            }
            ExprKind::Field { prefix, elem } => match self.eval(db, *prefix, scope)? {
                Val::Record(vs) => vs
                    .get(*elem as usize)
                    .cloned()
                    .ok_or_else(|| fails(db, e, "no such record element".into()))?,
                _ => return Err(not_yet(db, e, "the prefix is not a record value")),
            },
            ExprKind::Conv { to, operand } | ExprKind::Qualified { to, operand }
                if is_composite(db, e) =>
            {
                let v = self.eval(db, *operand, scope)?;
                let Val::Array(a) = &v else {
                    return Ok(Some(v));
                };
                // A constrained target gives the bounds (LRM 9.3.6), lengths must match.
                match self.shape(db, *to, scope, ex.span)? {
                    Some(Shape::Array { dims, .. }) => {
                        let have = bounds_of(a);
                        if have.len() != dims.len()
                            || have.iter().zip(&dims).any(|(x, y)| x.len() != y.len())
                        {
                            return Err(fails(db, e, "length mismatch in a conversion".into()));
                        }
                        make(&dims, elems(a))
                    }
                    _ => v,
                }
            }
            ExprKind::Call { callee, args, .. } => {
                let Some(Body::Implicit(op)) = db.decls[*callee].kind.subprogram().map(|s| &s.body)
                else {
                    return Ok(None);
                };
                let xs: Vec<ExprId> = args
                    .iter()
                    .filter_map(|a| match a.actual {
                        Actual::Expr(x) => Some(x),
                        _ => None,
                    })
                    .collect();
                match (op.family, xs.as_slice()) {
                    (Family::Cat, [l, r]) => self.concat(db, e, *l, *r, scope)?,
                    (Family::Eq | Family::Neq, [l, r]) if is_composite(db, *l) => {
                        let (a, b) = (self.eval(db, *l, scope)?, self.eval(db, *r, scope)?);
                        let same = match (&a, &b) {
                            // Equal lengths and elements; bounds do not matter (LRM 9.2.3).
                            (Val::Array(x), Val::Array(y)) => elems(x) == elems(y),
                            _ => a == b,
                        };
                        Val::Enum(u32::from(same == (op.family == Family::Eq)))
                    }
                    (
                        f @ (Family::And
                        | Family::Or
                        | Family::Nand
                        | Family::Nor
                        | Family::Xor
                        | Family::Xnor
                        | Family::Not),
                        xs,
                    ) if is_composite(db, e) && xs.iter().all(|&x| is_composite(db, x)) => {
                        self.logical(db, e, f, xs, scope)?
                    }
                    (Family::ToString, [x]) if !is_composite(db, *x) => {
                        self.text_of(db, e, *x, scope)?
                    }
                    _ if is_composite(db, e) || xs.iter().any(|&x| is_composite(db, x)) => {
                        return Err(not_yet(db, e, "a predefined operator on composite values"));
                    }
                    _ => return Ok(None),
                }
            }
            _ => return Ok(None),
        }))
    }

    /// `TO_STRING` of an integer or enumeration value (LRM 5.7): its image, a character
    /// literal without the apostrophes, as a STRING `1 to n`.
    fn text_of(&mut self, db: &Db, e: ExprId, x: ExprId, scope: Option<ScopeId>) -> R<Val> {
        use yavs_sem::types::TypeKind;
        let v = self.eval(db, x, scope)?;
        let text = match (&db.types[db.base(db.exprs[x].subtype)].kind, &v) {
            (TypeKind::Integer | TypeKind::UniversalInteger, Val::Int(i)) => {
                yavs_std::image::integer_image(*i)
            }
            (TypeKind::Enum { lits }, Val::Enum(p)) => {
                let Some(&l) = lits.get(*p as usize) else {
                    return Err(fails(db, e, "enumeration position out of range".into()));
                };
                let name = db.decls[l].name.to_string();
                match name.as_bytes() {
                    [b'\'', c, b'\''] => vec![*c],
                    n => yavs_std::image::identifier_image(n),
                }
            }
            _ => return Err(not_yet(db, e, "TO_STRING of this type")),
        };
        let n = text.len() as u64;
        Ok(make(
            &[span(1, Direction::To, n)],
            text.into_iter().map(|b| Val::Enum(u32::from(b))).collect(),
        ))
    }

    /// Element-wise logical operators on BIT and BOOLEAN arrays (LRM 9.2.2): the result has
    /// the left operand's bounds; operands of different lengths fail.
    fn logical(
        &mut self,
        db: &Db,
        e: ExprId,
        f: Family,
        xs: &[ExprId],
        scope: Option<ScopeId>,
    ) -> R<Val> {
        let mut ops = Vec::with_capacity(xs.len());
        for &x in xs {
            let Val::Array(a) = self.eval(db, x, scope)? else {
                return Err(not_yet(db, e, "a logical operator on a non-array value"));
            };
            ops.push(a);
        }
        let b = bounds_of(&ops[0])[0];
        let vs: Vec<Vec<Val>> = ops.iter().map(elems).collect();
        if vs.iter().any(|v| v.len() != vs[0].len()) {
            return Err(fails(db, e, "operands of different lengths".into()));
        }
        let bit = |v: &Val| match v {
            Val::Enum(p @ (0 | 1)) => Ok(*p == 1),
            _ => Err(not_yet(db, e, "a logical operator on this element type")),
        };
        let mut out = Vec::with_capacity(vs[0].len());
        for k in 0..vs[0].len() {
            let a = bit(&vs[0][k])?;
            let r = match (f, vs.get(1)) {
                (Family::Not, None) => !a,
                (_, Some(w)) => {
                    let c = bit(&w[k])?;
                    match f {
                        Family::And => a && c,
                        Family::Or => a || c,
                        Family::Nand => !(a && c),
                        Family::Nor => !(a || c),
                        Family::Xor => a != c,
                        _ => a == c,
                    }
                }
                _ => return Err(not_yet(db, e, "this logical operator")),
            };
            out.push(Val::Enum(u32::from(r)));
        }
        Ok(make(&[b], out))
    }

    /// The index subtype's `'left` and direction of 1-D array type of `e`.
    fn index_start(&mut self, db: &Db, e: ExprId, scope: Option<ScopeId>) -> R<(i64, Direction)> {
        let TypeKind::Array { indexes, .. } = &db.types[db.base(db.exprs[e].subtype)].kind else {
            return Err(not_yet(db, e, "not an array"));
        };
        let b = self.scalar_bounds(db, indexes[0], scope, db.exprs[e].span)?;
        Ok((b.left, b.dir))
    }

    /// `l & r` (LRM 9.2.5) on one-dimensional arrays and their elements.
    fn concat(
        &mut self,
        db: &Db,
        e: ExprId,
        l: ExprId,
        r: ExprId,
        scope: Option<ScopeId>,
    ) -> R<Val> {
        let rt = db.base(db.exprs[e].subtype);
        let piece = |m: &mut Model, x: ExprId| -> R<(Option<Bounds>, Vec<Val>)> {
            let v = m.eval(db, x, scope)?;
            Ok(match v {
                Val::Array(a) if db.base(db.exprs[x].subtype) == rt => {
                    (Some(bounds_of(&a)[0]), elems(&a))
                }
                v => (None, vec![v]),
            })
        };
        let (lb, mut xs) = piece(self, l)?;
        let (rb, ys) = piece(self, r)?;
        let n = (xs.len() + ys.len()) as u64;
        let b = match (lb, rb) {
            (Some(lb), Some(rb)) if lb.is_empty() => rb,
            (Some(lb), _) if !lb.is_empty() => span(lb.left, lb.dir, n),
            _ => {
                let (left, dir) = self.index_start(db, e, scope)?;
                span(left, dir, n)
            }
        };
        xs.extend(ys);
        Ok(make(&[b], xs))
    }

    fn aggregate(
        &mut self,
        db: &Db,
        e: ExprId,
        assocs: &[AggAssoc],
        bounds: &AggBounds,
        scope: Option<ScopeId>,
    ) -> R<Val> {
        let ex = &db.exprs[e];
        match &db.types[db.base(ex.subtype)].kind {
            TypeKind::Record { elems: fs } => {
                let mut vs: Vec<Option<Val>> = vec![None; fs.len()];
                let mut next = 0;
                for a in assocs {
                    let v = self.eval(db, a.value, scope)?;
                    if a.choices.is_empty() {
                        *vs.get_mut(next)
                            .ok_or_else(|| fails(db, e, "too many elements".into()))? = Some(v);
                        next += 1;
                        continue;
                    }
                    for c in a.choices.iter() {
                        match c {
                            Choice::Element(i) => vs[*i as usize] = Some(v.clone()),
                            Choice::Others => vs
                                .iter_mut()
                                .filter(|x| x.is_none())
                                .for_each(|x| *x = Some(v.clone())),
                            _ => return Err(not_yet(db, e, "this record aggregate choice")),
                        }
                    }
                }
                let vs: Option<Vec<Val>> = vs.into_iter().collect();
                Ok(Val::Record(
                    vs.ok_or_else(|| fails(db, e, "a record element has no value".into()))?
                        .into(),
                ))
            }
            TypeKind::Array { indexes, .. } if indexes.len() == 1 => {
                // Positional values (slice pieces spliced in) and named ones.
                let mut pos: Vec<Val> = Vec::new();
                let mut named: Vec<(&Choice, Val)> = Vec::new();
                for a in assocs {
                    let v = self.eval(db, a.value, scope)?;
                    if a.choices.is_empty() {
                        match v {
                            Val::Array(x) if a.slice_piece => pos.extend(elems(&x)),
                            v => pos.push(v),
                        }
                    } else {
                        if a.slice_piece {
                            return Err(not_yet(db, e, "a named slice in an aggregate"));
                        }
                        for c in a.choices.iter() {
                            named.push((c, v.clone()));
                        }
                    }
                }
                let (left, dir) = self.index_start(db, e, scope)?;
                let b = match bounds {
                    AggBounds::Context => match self.shape(db, ex.subtype, scope, ex.span)? {
                        Some(Shape::Array { dims, .. }) => dims[0],
                        _ => return Err(not_yet(db, e, "an aggregate without known bounds")),
                    },
                    AggBounds::Positional { .. } | AggBounds::Concat => {
                        span(left, dir, pos.len() as u64)
                    }
                    AggBounds::Named { low, high } => {
                        let lo = int(&self.eval(db, *low, scope)?, ex.span)?;
                        let hi = int(&self.eval(db, *high, scope)?, ex.span)?;
                        match dir {
                            Direction::To => Bounds {
                                left: lo,
                                dir,
                                right: hi,
                            },
                            Direction::Downto => Bounds {
                                left: hi,
                                dir,
                                right: lo,
                            },
                        }
                    }
                    AggBounds::Record => {
                        return Err(not_yet(db, e, "a record aggregate of an array"));
                    }
                };
                let n = b.len() as usize;
                let mut xs: Vec<Option<Val>> = vec![None; n];
                if pos.len() > n {
                    return Err(fails(
                        db,
                        e,
                        format!("{} elements for {n} positions", pos.len()),
                    ));
                }
                for (k, v) in pos.into_iter().enumerate() {
                    xs[k] = Some(v);
                }
                for (c, v) in named {
                    match c {
                        Choice::Expr(x) => {
                            let i = int(&self.eval(db, *x, scope)?, db.exprs[*x].span)?;
                            let p = b.pos(i).ok_or_else(|| {
                                fails(
                                    db,
                                    *x,
                                    format!("choice {i} is outside the aggregate's bounds"),
                                )
                            })?;
                            xs[p as usize] = Some(v);
                        }
                        Choice::Range(r) => {
                            let r = self.range(db, r, scope, ex.span)?;
                            let (lo, hi) = r.low_high();
                            for i in lo..=hi {
                                let p = b.pos(i).ok_or_else(|| {
                                    fails(
                                        db,
                                        e,
                                        format!("choice {i} is outside the aggregate's bounds"),
                                    )
                                })?;
                                xs[p as usize] = Some(v.clone());
                            }
                        }
                        Choice::Others => xs
                            .iter_mut()
                            .filter(|x| x.is_none())
                            .for_each(|x| *x = Some(v.clone())),
                        Choice::Element(_) => {
                            return Err(not_yet(db, e, "an element choice in an array aggregate"));
                        }
                    }
                }
                let xs: Option<Vec<Val>> = xs.into_iter().collect();
                let xs =
                    xs.ok_or_else(|| fails(db, e, "an aggregate element has no value".into()))?;
                Ok(make(&[b], xs))
            }
            _ => Err(not_yet(db, e, "a multi-dimensional aggregate")),
        }
    }
}
