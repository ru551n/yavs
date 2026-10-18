//! Elaboration-time values: the evaluator, subtype shapes, initial values and `-g` overrides.
//!
//! ponytail: globally static expressions are folded with sem's static folder
//! (`Db::fold_with`) over generic and constant values; anything else (user function calls,
//! aggregates) reports E2006 until rsim-interp evaluates elaboration code (ir.md 3.6).

use std::collections::HashMap;

use rsim_diag::{Diagnostic, Span};
use rsim_kernel::{Elem, LogicVec, Value};
use rsim_sem::decl::{DeclKind, ObjClass, ObjRole};
use rsim_sem::hir::{AttrPrefix, Direction, ExprKind, PredefAttr, RangeC};
use rsim_sem::scope::ScopeKind as SemScope;
use rsim_sem::stat::{ArrayConst, Elems};
use rsim_sem::types::{Constraint, Resolution, TypeKind};
use rsim_sem::{Db, DeclId, ExprId, SubtypeId, TypeId};

use crate::codes;
use crate::hier::{Bounds, Hierarchy, NetGraph, ObjId, ScopeId, Shape};

pub type Val = rsim_sem::stat::ConstValue;

/// What evaluation needs besides the HIR: values per elaborated scope and signal objects.
#[derive(Default)]
pub struct Model {
    pub hier: Hierarchy,
    pub nets: NetGraph,
    /// Generic, generate parameter and constant values, per scope.
    pub(crate) vals: Vec<HashMap<DeclId, Val>>,
    /// Signal objects by (scope, declaration).
    pub(crate) sigs: HashMap<(ScopeId, DeclId), ObjId>,
    /// Package constants.
    pub(crate) pkg: HashMap<DeclId, Val>,
}

type R<T> = Result<T, Box<Diagnostic>>;

fn err(code: &'static str, span: Span, msg: impl Into<String>, label: &str) -> Box<Diagnostic> {
    Box::new(Diagnostic::error(code, msg).with_primary(span, label))
}

impl Model {
    /// The signal object `d` names, seen from `scope`.
    pub fn signal(&self, scope: ScopeId, d: DeclId) -> Option<ObjId> {
        let mut cur = Some(scope);
        while let Some(s) = cur {
            if let Some(&o) = self.sigs.get(&(s, d)) {
                return Some(o);
            }
            cur = self.hier.scope(s).parent;
        }
        None
    }

    pub(crate) fn set(&mut self, scope: ScopeId, d: DeclId, v: Val) {
        let i = scope.index();
        if self.vals.len() <= i {
            self.vals.resize_with(i + 1, HashMap::new);
        }
        self.vals[i].insert(d, v);
    }

    fn get(&self, scope: Option<ScopeId>, d: DeclId) -> Option<&Val> {
        let mut cur = scope;
        while let Some(s) = cur {
            if let Some(v) = self.vals.get(s.index()).and_then(|m| m.get(&d)) {
                return Some(v);
            }
            cur = self.hier.scope(s).parent;
        }
        self.pkg.get(&d)
    }

    /// Evaluate `e` in `scope` (`None`: package level).
    pub fn eval(&mut self, db: &Db, e: ExprId, scope: Option<ScopeId>) -> R<Val> {
        let ex = &db.exprs[e];
        if let Some(c) = ex.value {
            return Ok(db.consts[c].clone());
        }
        match &ex.kind {
            ExprKind::Const(c) => return Ok(db.consts[*c].clone()),
            ExprKind::Obj(d) => return self.value_of(db, *d, scope, ex.span),
            ExprKind::Attr { attr, prefix, args }
                if matches!(
                    attr,
                    PredefAttr::Left
                        | PredefAttr::Right
                        | PredefAttr::High
                        | PredefAttr::Low
                        | PredefAttr::Length
                        | PredefAttr::Ascending
                ) && !matches!(prefix, AttrPrefix::Subtype(st) if db.types[db.base(*st)].kind.is_scalar()) =>
            {
                let dim = match args.first() {
                    Some(a) => int(&self.eval(db, *a, scope)?, db.exprs[*a].span)? - 1,
                    None => 0,
                };
                let b = self.prefix_bounds(db, prefix, dim as u32, scope, ex.span)?;
                let (lo, hi) = b.low_high();
                let wrap = |v: i64| match db.types[db.base(ex.subtype)].kind {
                    TypeKind::Enum { .. } => Val::Enum(v as u32),
                    _ => Val::Int(v),
                };
                return Ok(match attr {
                    PredefAttr::Left => wrap(b.left),
                    PredefAttr::Right => wrap(b.right),
                    PredefAttr::Low => wrap(lo),
                    PredefAttr::High => wrap(hi),
                    PredefAttr::Length => Val::Int(b.len() as i64),
                    _ => Val::Enum(u32::from(b.dir == Direction::To)),
                });
            }
            _ => {}
        }
        let mut inner: Option<Box<Diagnostic>> = None;
        let r = db.fold_with(e, &mut |c| match self.eval(db, c, scope) {
            Ok(v) => Some(v),
            Err(d) => {
                inner.get_or_insert(d);
                None
            }
        });
        match r {
            Ok(Some(v)) => Ok(v),
            Ok(None) => Err(inner.unwrap_or_else(|| {
                Box::new(
                    Diagnostic::error(
                        codes::NOT_EVALUATED,
                        "this expression cannot be evaluated during elaboration yet",
                    )
                    .with_primary(ex.span, "needs the IR interpreter")
                    .with_note(
                        "rsim folds globally static expressions over generics and constants; \
                         function calls and composite values at elaboration time need \
                         rsim-interp (docs/design/ir.md 3.6), which is not wired in yet",
                    ),
                )
            })),
            Err(why) => Err(err(
                codes::EVAL_FAILED,
                ex.span,
                format!("evaluation fails during elaboration: {why}"),
                why,
            )),
        }
    }

    pub(crate) fn value_of(
        &mut self,
        db: &Db,
        d: DeclId,
        scope: Option<ScopeId>,
        span: Span,
    ) -> R<Val> {
        if let Some(v) = self.get(scope, d) {
            return Ok(v.clone());
        }
        let decl = &db.decls[d];
        let DeclKind::Object(o) = &decl.kind else {
            return Err(err(
                codes::NOT_EVALUATED,
                span,
                format!("`{}` has no value during elaboration", decl.name),
                "not a constant",
            ));
        };
        match (o.class, &o.role, o.default) {
            (ObjClass::Constant, ObjRole::Plain, Some(init)) => {
                let in_pkg = matches!(
                    db.scopes[decl.scope].kind,
                    SemScope::Package | SemScope::PackageBody
                );
                if in_pkg {
                    self.check_body_elaborated(db, init, decl.scope)?;
                    let v = self.eval(db, init, None)?;
                    self.pkg.insert(d, v.clone());
                    Ok(v)
                } else {
                    let v = self.eval(db, init, scope)?;
                    if let Some(s) = scope {
                        self.set(s, d, v.clone());
                    }
                    Ok(v)
                }
            }
            (ObjClass::Constant, ObjRole::Deferred, _) => Err(Box::new(
                Diagnostic::error(
                    codes::NOT_IMPLEMENTED,
                    format!("the value of deferred constant `{}`", decl.name),
                )
                .with_primary(span, "used here during elaboration")
                .with_note("deferred constants at elaboration time are planned for M3"),
            )),
            (ObjClass::Constant, ObjRole::Generic | ObjRole::GenerateParam, _) => Err(Box::new(
                Diagnostic::error(
                    codes::INTERNAL,
                    format!("internal error: generic `{}` has no value", decl.name),
                )
                .with_primary(span, "while elaborating this")
                .with_note("this is a bug in rsim"),
            )),
            _ => Err(err(
                codes::NOT_EVALUATED,
                span,
                format!(
                    "`{}` is not a constant: it has no value during elaboration",
                    decl.name
                ),
                "needs a globally static value",
            )),
        }
    }

    /// LRM 14.4.2: a package constant whose initial value calls a subprogram of the same
    /// package, whose body (in the package body) is not elaborated yet. NVC and Questa
    /// report it; fail early with both spans.
    fn check_body_elaborated(&self, db: &Db, e: ExprId, pkg: rsim_sem::ScopeId) -> R<()> {
        let mut stack = vec![e];
        while let Some(e) = stack.pop() {
            let ex = &db.exprs[e];
            match &ex.kind {
                ExprKind::Call { callee, args, .. } => {
                    let cd = &db.decls[*callee];
                    let user = cd.kind.subprogram().is_some_and(|s| {
                        !matches!(
                            s.body,
                            rsim_sem::decl::Body::Implicit(_) | rsim_sem::decl::Body::Native(_)
                        )
                    });
                    if user && cd.scope == pkg && db.scopes[pkg].kind == SemScope::Package {
                        return Err(Box::new(
                            Diagnostic::error(
                                codes::BODY_NOT_ELABORATED,
                                format!(
                                    "subprogram `{}` is called before its body is elaborated",
                                    cd.name
                                ),
                            )
                            .with_primary(ex.span, "called while its package is elaborated")
                            .with_secondary(cd.span, "declared here; its body is in the package body")
                            .with_note(
                                "the package declaration is elaborated before its body (LRM 14.4.2)",
                            )
                            .with_help("move the constant into the package body, or the function into another package"),
                        ));
                    }
                    for a in args.iter() {
                        if let rsim_sem::hir::Actual::Expr(x) = a.actual {
                            stack.push(x);
                        }
                    }
                }
                ExprKind::Index { prefix, indices } => {
                    stack.push(*prefix);
                    stack.extend(indices.iter().copied());
                }
                ExprKind::Field { prefix, .. }
                | ExprKind::Deref { prefix }
                | ExprKind::Slice { prefix, .. } => stack.push(*prefix),
                ExprKind::Qualified { operand, .. }
                | ExprKind::Conv { operand, .. }
                | ExprKind::ImplicitConv { operand } => stack.push(*operand),
                ExprKind::Aggregate { assocs, .. } => {
                    stack.extend(assocs.iter().map(|a| a.value));
                }
                ExprKind::Conditional { alts } => {
                    for (v, c) in alts.iter() {
                        stack.push(*v);
                        stack.extend(*c);
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    /// Bounds of dimension `dim` of an attribute prefix (`s'range`, `t'length`).
    fn prefix_bounds(
        &mut self,
        db: &Db,
        p: &AttrPrefix,
        dim: u32,
        scope: Option<ScopeId>,
        span: Span,
    ) -> R<Bounds> {
        let shape = match p {
            AttrPrefix::Subtype(st) => {
                if db.types[db.base(*st)].kind.is_scalar() {
                    return self.scalar_bounds(db, *st, scope, span);
                }
                self.shape(db, *st, scope, span)?
            }
            AttrPrefix::Expr(e) => self.expr_shape(db, *e, scope)?,
            AttrPrefix::Entity(_) => None,
        };
        match shape {
            Some(Shape::Array { dims, .. }) if (dim as usize) < dims.len() => {
                Ok(dims[dim as usize])
            }
            _ => Err(err(
                codes::NOT_EVALUATED,
                span,
                "the bounds of this prefix are not known during elaboration",
                "not a constrained array",
            )),
        }
    }

    /// The shape of an object expression: a signal, or a constant's value.
    fn expr_shape(&mut self, db: &Db, e: ExprId, scope: Option<ScopeId>) -> R<Option<Shape>> {
        if let (ExprKind::Obj(d), Some(s)) = (&db.exprs[e].kind, scope)
            && let Some(o) = self.signal(s, *d)
        {
            return Ok(Some(self.nets.obj(o).shape.clone()));
        }
        let span = db.exprs[e].span;
        match self.eval(db, e, scope)? {
            Val::Array(ArrayConst { bounds, .. }) => Ok(Some(Shape::Array {
                ty: db.base(db.exprs[e].subtype),
                dims: bounds
                    .iter()
                    .map(|&(left, dir, right)| Bounds { left, dir, right })
                    .collect(),
                // Only the bounds are used (attribute prefixes).
                elem: Box::new(Shape::Scalar {
                    ty: db.base(db.exprs[e].subtype),
                    elem: Elem::Bytes(1),
                    left: Val::Int(0),
                    res: None,
                }),
            })),
            _ => Err(err(
                codes::NOT_EVALUATED,
                span,
                "not an array",
                "bounds needed here",
            )),
        }
    }

    /// Evaluate a range constraint.
    pub fn range(&mut self, db: &Db, r: &RangeC, scope: Option<ScopeId>, span: Span) -> R<Bounds> {
        match r {
            RangeC::Explicit { left, dir, right } => Ok(Bounds {
                left: int(&self.eval(db, *left, scope)?, db.exprs[*left].span)?,
                dir: *dir,
                right: int(&self.eval(db, *right, scope)?, db.exprs[*right].span)?,
            }),
            RangeC::Attr {
                prefix,
                reverse,
                dim,
            } => {
                let b = self.prefix_bounds(db, prefix, dim.saturating_sub(1), scope, span)?;
                Ok(if *reverse {
                    Bounds {
                        left: b.right,
                        dir: match b.dir {
                            Direction::To => Direction::Downto,
                            Direction::Downto => Direction::To,
                        },
                        right: b.left,
                    }
                } else {
                    b
                })
            }
        }
    }

    fn scalar_bounds(
        &mut self,
        db: &Db,
        st: SubtypeId,
        scope: Option<ScopeId>,
        span: Span,
    ) -> R<Bounds> {
        match db.scalar_range(st) {
            Some(r) => self.range(db, &r, scope, span),
            None => match &db.types[db.base(st)].kind {
                TypeKind::Enum { lits } => Ok(Bounds {
                    left: 0,
                    dir: Direction::To,
                    right: lits.len() as i64 - 1,
                }),
                _ => Err(err(
                    codes::NOT_EVALUATED,
                    span,
                    "the range of this subtype is not known",
                    "here",
                )),
            },
        }
    }

    /// The fully constrained shape of subtype `st`; `None` when it is not fully constrained.
    pub fn shape(
        &mut self,
        db: &Db,
        st: SubtypeId,
        scope: Option<ScopeId>,
        span: Span,
    ) -> R<Option<Shape>> {
        let c = db.subtypes[st].constraint.clone();
        self.shape_c(db, st, &c, scope, span)
    }

    fn shape_c(
        &mut self,
        db: &Db,
        st: SubtypeId,
        c: &Constraint,
        scope: Option<ScopeId>,
        span: Span,
    ) -> R<Option<Shape>> {
        let ty = db.base(st);
        match &db.types[ty].kind {
            k if k.is_scalar() => {
                let b = self.scalar_bounds(db, st, scope, span)?;
                let left = match k {
                    TypeKind::Enum { .. } => Val::Enum(b.left as u32),
                    TypeKind::Physical { .. } => Val::Phys(b.left),
                    TypeKind::Floating | TypeKind::UniversalReal => match db.scalar_range(st) {
                        Some(RangeC::Explicit { left, .. }) => self.eval(db, left, scope)?,
                        _ => Val::Real(-f64::MAX),
                    },
                    _ => Val::Int(b.left),
                };
                let res = match &db.subtypes[st].resolution {
                    Some(Resolution::Function(f)) => Some(*f),
                    _ => None,
                };
                Ok(Some(Shape::Scalar {
                    ty,
                    elem: elem_of(db, ty),
                    left,
                    res,
                }))
            }
            TypeKind::Array { elem, .. } => {
                let (elem, (dims, ec)) = (
                    *elem,
                    match c {
                        Constraint::Array { dims, elem } => (dims.clone(), (**elem).clone()),
                        _ => (None, Constraint::None),
                    },
                );
                let Some(dims) = dims else { return Ok(None) };
                let mut ds = Vec::with_capacity(dims.len());
                for d in dims.iter() {
                    ds.push(self.range(db, d, scope, span)?);
                }
                let es = if matches!(ec, Constraint::None) {
                    self.shape(db, elem, scope, span)?
                } else {
                    self.shape_c(db, elem, &ec, scope, span)?
                };
                let Some(mut es) = es else { return Ok(None) };
                if let Some(Resolution::Element(r)) = &db.subtypes[st].resolution {
                    match &**r {
                        Resolution::Function(f) => set_res(&mut es, *f),
                        _ => {
                            return Err(err(
                                codes::NOT_IMPLEMENTED,
                                span,
                                "nested element resolution is not implemented yet",
                                "here",
                            ));
                        }
                    }
                }
                Ok(Some(Shape::Array {
                    ty,
                    dims: ds,
                    elem: Box::new(es),
                }))
            }
            TypeKind::Record { elems } => {
                let mut fields = Vec::with_capacity(elems.len());
                for (i, f) in elems.iter().enumerate() {
                    let fc = match c {
                        Constraint::Record { elems } => elems
                            .iter()
                            .find(|(k, _)| *k as usize == i)
                            .map(|(_, c)| c.clone()),
                        _ => None,
                    };
                    let s = match fc {
                        Some(fc) => self.shape_c(db, f.subtype, &fc, scope, span)?,
                        None => self.shape(db, f.subtype, scope, span)?,
                    };
                    let Some(s) = s else { return Ok(None) };
                    fields.push((f.name, s));
                }
                Ok(Some(Shape::Record { ty, fields }))
            }
            _ => Err(err(
                codes::NOT_IMPLEMENTED,
                span,
                "signals of this type are not implemented yet",
                "here",
            )),
        }
    }
}

fn set_res(s: &mut Shape, f: DeclId) {
    match s {
        Shape::Scalar { res, .. } => *res = Some(f),
        Shape::Array { elem, .. } => set_res(elem, f),
        Shape::Record { fields, .. } => fields.iter_mut().for_each(|(_, s)| set_res(s, f)),
    }
}

fn int(v: &Val, span: Span) -> R<i64> {
    v.as_int().ok_or_else(|| {
        err(
            codes::INTERNAL,
            span,
            "internal error: expected a discrete value",
            "this is a bug in rsim",
        )
    })
}

/// The kernel element of a scalar type: bit-plane logic for the `LogicKind` enums
/// (std_ulogic, bit, boolean; ir.md 1.3), else bytes as laid out in memory.
pub fn elem_of(db: &Db, t: TypeId) -> Elem {
    match &db.types[t].kind {
        TypeKind::Enum { .. } if is_logic(db, t) => Elem::Logic,
        TypeKind::Enum { lits } if lits.len() <= 256 => Elem::Bytes(1),
        TypeKind::Enum { .. } => Elem::Bytes(4),
        _ => Elem::Bytes(8),
    }
}

fn is_logic(db: &Db, t: TypeId) -> bool {
    let wk = &db.well_known;
    if Some(t) == wk.bit || Some(t) == wk.boolean {
        return true;
    }
    is_std_ulogic(db, t)
}

pub(crate) fn is_std_ulogic(db: &Db, t: TypeId) -> bool {
    let td = &db.types[t];
    let owner = db.scopes[db.decls[td.decl].scope].owner;
    td.name.key.as_str() == "std_ulogic"
        && owner.is_some_and(|o| db.decls[o].name.key().as_str() == "std_logic_1164")
}

// ---------------------------------------------------------------------------
// Values of leaves.

/// The scalar values of `v` (of shape `shape`) in row-major order, `None` meaning
/// the scalar's default (`'left`).
fn flatten(
    shape: &Shape,
    v: Option<&Val>,
    out: &mut Vec<(Val, Vec<u32>)>,
    path: &mut Vec<u32>,
) -> Result<(), String> {
    match shape {
        Shape::Scalar { left, .. } => {
            out.push((v.cloned().unwrap_or_else(|| left.clone()), path.clone()));
            Ok(())
        }
        Shape::Array { elem, .. } => {
            let n = shape.len();
            match v {
                None => {
                    for _ in 0..n {
                        flatten(elem, None, out, path)?;
                    }
                }
                Some(Val::Array(a)) => {
                    let have: u64 = a
                        .bounds
                        .iter()
                        .map(|&(l, d, r)| {
                            Bounds {
                                left: l,
                                dir: d,
                                right: r,
                            }
                            .len()
                        })
                        .product();
                    if have != n {
                        return Err(format!("value has {have} elements, the subtype {n}"));
                    }
                    match &a.elems {
                        Elems::Bytes(b) => {
                            for &x in b.iter() {
                                flatten(elem, Some(&Val::Enum(u32::from(x))), out, path)?;
                            }
                        }
                        Elems::Runs(rs) => {
                            for (k, x) in rs.iter() {
                                for _ in 0..*k {
                                    flatten(elem, Some(x), out, path)?;
                                }
                            }
                        }
                    }
                }
                Some(_) => return Err("expected an array value".into()),
            }
            Ok(())
        }
        Shape::Record { fields, .. } => {
            let vs = match v {
                None => None,
                Some(Val::Record(vs)) if vs.len() == fields.len() => Some(vs),
                Some(_) => return Err("expected a record value".into()),
            };
            for (i, (_, f)) in fields.iter().enumerate() {
                path.push(i as u32);
                flatten(f, vs.map(|vs| &vs[i]), out, path)?;
                path.pop();
            }
            Ok(())
        }
    }
}

/// Initial kernel values of each leaf of `shape`, in leaf order.
pub fn leaf_values(shape: &Shape, v: Option<&Val>) -> Result<Vec<Value>, String> {
    let mut flat = Vec::new();
    flatten(shape, v, &mut flat, &mut Vec::new())?;
    let mut out = Vec::new();
    for (path, leaf, n) in shape.leaves() {
        let Shape::Scalar { elem, .. } = leaf else {
            continue;
        };
        let xs: Vec<&Val> = flat
            .iter()
            .filter(|(_, p)| *p == path)
            .map(|(v, _)| v)
            .collect();
        debug_assert_eq!(xs.len() as u64, n);
        out.push(encode(*elem, &xs));
    }
    Ok(out)
}

/// Kernel bytes or planes of scalar values in row-major order.
pub fn encode(elem: Elem, xs: &[&Val]) -> Value {
    match elem {
        Elem::Logic => {
            let n = xs.len();
            Value::Logic(LogicVec::from_fn(n, |i| {
                let code = xs[n - 1 - i].as_int().unwrap_or(0) as u8;
                rsim_logic::StdULogic::from_code_pub(code)
            }))
        }
        Elem::Bytes(w) => {
            let mut b = Vec::with_capacity(xs.len() * w as usize);
            for x in xs {
                match (x, w) {
                    (Val::Real(r), _) => b.extend_from_slice(&r.to_le_bytes()),
                    (x, 1) => b.push(x.as_int().unwrap_or(0) as u8),
                    (x, 4) => b.extend_from_slice(&(x.as_int().unwrap_or(0) as u32).to_le_bytes()),
                    (x, _) => b.extend_from_slice(&x.as_int().unwrap_or(0).to_le_bytes()),
                }
            }
            Value::Bytes(b)
        }
    }
}

// ---------------------------------------------------------------------------
// `-g NAME=VALUE` (plan rule 4, GHDL #1117): the text is a value of the generic's type,
// parsed with the `'value` parser of rsim-std.

pub fn parse_override(
    m: &mut Model,
    db: &Db,
    g: DeclId,
    text: &str,
    scope: ScopeId,
) -> Result<Val, String> {
    use rsim_std::types as st;
    use rsim_std::value as v;
    let DeclKind::Object(o) = &db.decls[g].kind else {
        return Err("only generic constants can be overridden".into());
    };
    let sub = o.subtype;
    let ty = db.base(sub);
    let bytes = st::to_latin1(text);
    let span = db.decls[g].span;
    let bounds = |m: &mut Model| {
        m.scalar_bounds(db, sub, Some(scope), span)
            .map_err(|d| d.message.clone())
    };
    match &db.types[ty].kind {
        TypeKind::Integer | TypeKind::UniversalInteger => {
            let (lo, hi) = bounds(m)?.low_high();
            v::integer_value(&bytes, st::Bounds { low: lo, high: hi })
                .map(Val::Int)
                .map_err(|e| e.to_string())
        }
        TypeKind::Floating | TypeKind::UniversalReal => v::real_value(&bytes, st::RealBounds::REAL)
            .map(Val::Real)
            .map_err(|e| e.to_string()),
        TypeKind::Physical { units } => {
            let pt = st::PhysicalType {
                units: units
                    .iter()
                    .map(|u| st::Unit {
                        name: st::to_latin1(&db.decls[u.decl].name.to_string()),
                        value: u.scale,
                    })
                    .collect(),
            };
            let (lo, hi) = bounds(m)?.low_high();
            v::physical_value(&bytes, &pt, st::Bounds { low: lo, high: hi })
                .map(Val::Phys)
                .map_err(|e| e.to_string())
        }
        TypeKind::Enum { lits } => {
            let et = enum_type(db, lits);
            let p = v::enum_value(&bytes, &et).map_err(|e| e.to_string())?;
            let b = bounds(m)?;
            if b.pos(p as i64).is_none() {
                return Err(format!("`{text}` is outside the subtype of the generic"));
            }
            Ok(Val::Enum(p as u32))
        }
        TypeKind::Array { indexes, elem } if indexes.len() == 1 => {
            let et =
                match &db.types[db.base(*elem)].kind {
                    TypeKind::Enum { lits } => enum_type(db, lits),
                    _ => return Err(
                        "only arrays of characters or other enumeration literals can be overridden"
                            .into(),
                    ),
                };
            // A string is the raw text; an array of other enums is one character literal
            // per element (`-gV=0101`).
            let mut pos = Vec::with_capacity(bytes.len());
            for &c in &bytes {
                let p = et
                    .lits
                    .iter()
                    .position(|l| *l == st::EnumLit::Char(c))
                    .ok_or_else(|| {
                        format!("'{}' is not a literal of the element type", c as char)
                    })?;
                pos.push(p as u8);
            }
            let n = pos.len() as i64;
            let b = match m
                .shape(db, sub, Some(scope), span)
                .map_err(|d| d.message.clone())?
            {
                Some(Shape::Array { dims, .. }) => {
                    if dims[0].len() != n as u64 {
                        return Err(format!(
                            "the generic has {} elements, the value {n}",
                            dims[0].len()
                        ));
                    }
                    dims[0]
                }
                _ => {
                    // Unconstrained: the index subtype's 'left, ascending (LRM 9.3.3.3 like a string literal).
                    let ib = m
                        .scalar_bounds(db, indexes[0], Some(scope), span)
                        .map_err(|d| d.message.clone())?;
                    Bounds {
                        left: ib.left,
                        dir: ib.dir,
                        right: match ib.dir {
                            Direction::To => ib.left + n - 1,
                            Direction::Downto => ib.left - n + 1,
                        },
                    }
                }
            };
            Ok(Val::Array(ArrayConst {
                bounds: Box::new([(b.left, b.dir, b.right)]),
                elems: Elems::Bytes(pos.into()),
            }))
        }
        _ => Err(format!(
            "generics of type `{}` cannot be set from the command line",
            db.types[ty].name.text
        )),
    }
}

fn enum_type(db: &Db, lits: &[DeclId]) -> rsim_std::types::EnumType {
    use rsim_sem::decl::Designator;
    rsim_std::types::EnumType {
        lits: lits
            .iter()
            .map(|&l| match db.decls[l].name {
                Designator::Char(c) => rsim_std::types::EnumLit::Char(c as u8),
                d => rsim_std::types::EnumLit::Ident(rsim_std::types::to_latin1(&d.to_string())),
            })
            .collect(),
    }
}
