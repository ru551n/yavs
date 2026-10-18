//! Elaboration-time values: the evaluator, subtype shapes, initial values and `-g` overrides.
//!
//! Globally static expressions are folded with sem's static folder (`Db::fold_with`) over
//! generic and constant values; aggregates, indexing, slices, fields, concatenation,
//! composite equality and logical operators on BIT and BOOLEAN arrays are evaluated in
//! `crate::value`; calls of user functions run in yavs-interp after lowering the function
//! alone (ir.md 3.6), with arrays of scalars and records of scalars marshalled through `yavs_interp::call_values` (#77).
//! ponytail: multi-dimensional aggregates, arrays of composites passed to functions, shifts
//! and other predefined operators on arrays are still E2106.

use std::collections::HashMap;

use yavs_diag::{Diagnostic, Span};
use yavs_kernel::{Elem, LogicVec, Value};
use yavs_sem::decl::{DeclKind, ObjClass, ObjRole};
use yavs_sem::hir::{AttrPrefix, Direction, ExprKind, PredefAttr, RangeC};
use yavs_sem::scope::ScopeKind as SemScope;
use yavs_sem::stat::{ArrayConst, Elems};
use yavs_sem::types::{Constraint, Resolution, TypeKind};
use yavs_sem::{Db, DeclId, ExprId, SubtypeId, TypeId};

use crate::codes;
use crate::hier::{Bounds, Hierarchy, NetGraph, ObjId, ScopeId, Shape};

pub type Val = yavs_sem::stat::ConstValue;

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
    /// User functions lowered for elaboration-time calls (ir.md 3.6): `None` when lowering
    /// failed (the reason is reported at the call).
    pub(crate) funcs: HashMap<DeclId, Result<Lowered1, Box<Diagnostic>>>,
    /// Package scopes by package unit, library scopes by library.
    pub(crate) pkg_scopes: HashMap<yavs_sem::UnitId, ScopeId>,
    pub(crate) lib_scopes: HashMap<yavs_sem::LibId, ScopeId>,
    /// Conversions of port associations, indexed by `PortEdge::to_formal`/`to_actual`.
    pub convs: Vec<ConvFn>,
}

/// How a scalar crosses the kernel: its element and its class.
#[derive(Clone, Copy, Debug)]
pub struct Enc {
    pub elem: Elem,
    /// BIT/BOOLEAN stored as std_ulogic codes 2/3.
    pub bit01: bool,
    pub class: Class,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Class {
    Int,
    Real,
    Enum,
}

/// A conversion of a port association (LRM 6.5.7.1): a function of one scalar, run in
/// yavs-interp, or a type conversion between scalar types.
#[derive(Clone)]
pub struct ConvFn {
    pub(crate) func: Option<(std::sync::Arc<yavs_interp::Runtime>, yavs_ir::FuncId)>,
    pub from: Enc,
    pub to: Enc,
}

impl ConvFn {
    /// Convert a one-element kernel value.
    pub fn apply(&self, v: &Value) -> Result<Value, String> {
        let c = decode(&self.from, v)?;
        let r = match &self.func {
            Some((rt, f)) => yavs_interp::call(rt.clone(), *f, Default::default(), &[c])
                .map_err(|e| e.to_string())?
                .ok_or("the conversion function returned no value")?,
            None => match (self.from.class, self.to.class) {
                (Class::Int, Class::Real) => (c as i64 as f64).to_bits(),
                (Class::Real, Class::Int) => {
                    let r = f64::from_bits(c).round();
                    if !(-9.2e18..9.2e18).contains(&r) {
                        return Err(format!("{r} is outside the integer range"));
                    }
                    r as i64 as u64
                }
                _ => c,
            },
        };
        Ok(encode_cell(&self.to, r))
    }
}

fn decode(e: &Enc, v: &Value) -> Result<u64, String> {
    match (e.elem, v) {
        (Elem::Logic, Value::Logic(l)) if l.len() == 1 => {
            let code = l.get(0) as u64;
            Ok(if e.bit01 {
                code.saturating_sub(2)
            } else {
                code
            })
        }
        (Elem::Bytes(n), Value::Bytes(b)) if b.len() == n as usize => {
            let mut w = [0u8; 8];
            w[..b.len()].copy_from_slice(b);
            Ok(u64::from_le_bytes(w))
        }
        _ => Err("conversion of a composite value is not implemented yet".into()),
    }
}

fn encode_cell(e: &Enc, c: u64) -> Value {
    match e.elem {
        Elem::Logic => {
            let code = if e.bit01 { c as u8 + 2 } else { c as u8 };
            Value::Logic(LogicVec::from_fn(1, |_| {
                yavs_logic::StdULogic::from_code_pub(code)
            }))
        }
        Elem::Bytes(n) => Value::Bytes(c.to_le_bytes()[..n as usize].to_vec()),
    }
}

/// The kernel encoding of scalar type `t`.
pub(crate) fn enc_of(db: &Db, t: TypeId) -> Option<Enc> {
    let class = match &db.types[t].kind {
        TypeKind::Enum { .. } => Class::Enum,
        TypeKind::Floating | TypeKind::UniversalReal => Class::Real,
        k if k.is_scalar() => Class::Int,
        _ => return None,
    };
    let elem = elem_of(db, t);
    Some(Enc {
        elem,
        bit01: elem == Elem::Logic && !is_std_ulogic(db, t),
        class,
    })
}

/// A conversion `f` (a function or a type) from scalar type `from` to `to`.
pub(crate) fn conv_fn(db: &Db, f: DeclId, from: TypeId, to: TypeId, at: Span) -> R<ConvFn> {
    let (Some(fe), Some(te)) = (enc_of(db, from), enc_of(db, to)) else {
        return Err(err(
            codes::NOT_IMPLEMENTED,
            at,
            "conversions of composite ports are not implemented yet",
            "conversion",
        ));
    };
    let func = match &db.decls[f].kind {
        DeclKind::Subprogram(_) => {
            let l = lower_one(db, f, at)?;
            Some((l.rt, l.func))
        }
        _ => None,
    };
    Ok(ConvFn {
        func,
        from: fe,
        to: te,
    })
}

/// A function lowered on its own into a program, for `yavs_interp::call`.
pub(crate) struct Lowered1 {
    rt: std::sync::Arc<yavs_interp::Runtime>,
    func: yavs_ir::FuncId,
    /// IR types of the parameters and of the result (for marshalling values).
    params: Vec<yavs_ir::TyId>,
    ret: Option<yavs_ir::TyId>,
}

pub(crate) type R<T> = Result<T, Box<Diagnostic>>;

pub(crate) fn err(
    code: &'static str,
    span: Span,
    msg: impl Into<String>,
    label: &str,
) -> Box<Diagnostic> {
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
        if let ExprKind::Call {
            callee,
            args,
            method_of: None,
            ..
        } = &ex.kind
            && let Some(sp) = db.decls[*callee].kind.subprogram()
            && matches!(sp.body, yavs_sem::decl::Body::Hir { .. })
        {
            return self.call(db, e, *callee, args, scope);
        }
        if let Some(v) = self.composite(db, e, scope)? {
            return Ok(v);
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
                        "yavs folds globally static expressions over generics and constants and \
                         runs scalar user function calls in yavs-interp; composite values \
                         (aggregates, arrays passed to or returned by functions) are not \
                         evaluated during elaboration yet",
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

    /// A call of a user function during elaboration: scalar arguments and result, run in
    /// yavs-interp after lowering the function (and what it calls) with yavs-lower.
    fn call(
        &mut self,
        db: &Db,
        e: ExprId,
        callee: DeclId,
        args: &[yavs_sem::hir::Arg],
        scope: Option<ScopeId>,
    ) -> R<Val> {
        let ex = &db.exprs[e];
        let Some(sp) = db.decls[callee].kind.subprogram() else {
            return Err(err(
                codes::INTERNAL,
                ex.span,
                "internal error: not a subprogram",
                "here",
            ));
        };
        let not_scalar = |what: &str| {
            Box::new(
                Diagnostic::error(
                    codes::NOT_EVALUATED,
                    format!("this call cannot be evaluated during elaboration yet: {what}"),
                )
                .with_primary(ex.span, "called during elaboration")
                .with_note(
                    "elaboration-time calls take and return scalars, arrays of scalars and \
                     records of scalars",
                ),
            )
        };
        let mut vals = Vec::with_capacity(sp.params.len());
        for (i, p) in sp.params.iter().enumerate() {
            if p.mode != yavs_sem::hir::Mode::In || p.class != ObjClass::Constant {
                return Err(not_scalar("a parameter that is not an `in` constant"));
            }
            let a = args.iter().find(|a| a.formal as usize == i);
            vals.push(match a.map(|a| &a.actual) {
                Some(yavs_sem::hir::Actual::Expr(x)) => self.eval(db, *x, scope)?,
                Some(yavs_sem::hir::Actual::Default) | None => match p.default {
                    Some(d) => self.eval(db, d, scope)?,
                    None => return Err(not_scalar("a missing argument")),
                },
                Some(_) => return Err(not_scalar("an individual or converted argument")),
            });
        }
        if sp.ret.is_none() {
            return Err(not_scalar("not a function"));
        }
        let l = match self
            .funcs
            .entry(callee)
            .or_insert_with(|| lower_one(db, callee, ex.span))
        {
            Ok(l) => l,
            Err(d) => {
                let mut d = d.clone();
                if let Some(p) = &mut d.primary {
                    p.span = ex.span;
                }
                return Err(d);
            }
        };
        let prog = l.rt.prog.clone();
        let mut cvs = Vec::with_capacity(vals.len());
        for (v, &t) in vals.iter().zip(&l.params) {
            cvs.push(
                crate::value::encode(&prog, t, v)
                    .ok_or_else(|| not_scalar("an argument value of this type"))?,
            );
        }
        let ret = l.ret.ok_or_else(|| not_scalar("not a function"))?;
        let composite = prog
            .types
            .get(ret)
            .is_some_and(|t| t.scalar_size().is_none());
        let r = yavs_interp::call_values(
            l.rt.clone(),
            l.func,
            std::sync::Arc::new(yavs_interp::InstData::default()),
            &cvs,
            composite.then_some(ret),
        );
        match r {
            Ok(Some(v)) => crate::value::decode(&prog, ret, &v)
                .ok_or_else(|| not_scalar("a result of this type")),
            Ok(None) => Err(not_scalar("no result")),
            Err(e) => {
                let msg = match &e.kind {
                    yavs_kernel::ErrorKind::Runtime(m) => m.clone(),
                    k => k.to_string(),
                };
                Err(err(
                    codes::EVAL_FAILED,
                    ex.span,
                    format!("evaluation fails during elaboration: {msg}"),
                    "in this call",
                ))
            }
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
                .with_note("this is a bug in yavs"),
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
    fn check_body_elaborated(&self, db: &Db, e: ExprId, pkg: yavs_sem::ScopeId) -> R<()> {
        let mut stack = vec![e];
        while let Some(e) = stack.pop() {
            let ex = &db.exprs[e];
            match &ex.kind {
                ExprKind::Call { callee, args, .. } => {
                    let cd = &db.decls[*callee];
                    let user = cd.kind.subprogram().is_some_and(|s| {
                        !matches!(
                            s.body,
                            yavs_sem::decl::Body::Implicit(_) | yavs_sem::decl::Body::Native(_)
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
                        if let yavs_sem::hir::Actual::Expr(x) = a.actual {
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

    pub(crate) fn scalar_bounds(
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
                let left = match k {
                    TypeKind::Floating | TypeKind::UniversalReal => match db.scalar_range(st) {
                        Some(RangeC::Explicit { left, .. }) => self.eval(db, left, scope)?,
                        _ => Val::Real(-f64::MAX),
                    },
                    _ => {
                        let b = self.scalar_bounds(db, st, scope, span)?;
                        match k {
                            TypeKind::Enum { .. } => Val::Enum(b.left as u32),
                            TypeKind::Physical { .. } => Val::Phys(b.left),
                            _ => Val::Int(b.left),
                        }
                    }
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
                if let Some(Resolution::Element(r)) = &db.subtypes[st].resolution
                    && !apply_res(&mut es, r)
                {
                    return Err(err(
                        codes::INTERNAL,
                        span,
                        "internal error: resolution indication does not fit the subtype",
                        "this is a bug in yavs",
                    ));
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
                let mut sh = Shape::Record { ty, fields };
                if let Some(r @ Resolution::Record(_)) = &db.subtypes[st].resolution
                    && !apply_res(&mut sh, r)
                {
                    return Err(err(
                        codes::INTERNAL,
                        span,
                        "internal error: resolution indication does not fit the subtype",
                        "this is a bug in yavs",
                    ));
                }
                Ok(Some(sh))
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

/// Lower `f` (and what it calls) into its own program, for elaboration-time calls.
fn lower_one(db: &Db, f: DeclId, at: Span) -> Result<Lowered1, Box<Diagnostic>> {
    let mut l = yavs_lower::Lowerer::new(db, yavs_lower::LowerOptions::default());
    let sp = db.decls[f].kind.subprogram();
    let lowered = l.subprogram(f).and_then(|func| {
        let mut params = Vec::new();
        for p in sp.map(|s| &s.params[..]).unwrap_or(&[]) {
            params.push(l.ty(db.base(p.subtype))?);
        }
        let ret = match sp.and_then(|s| s.ret) {
            Some(r) => Some(l.ty(db.base(r))?),
            None => None,
        };
        l.finish().map(|lw| (func, params, ret, lw))
    });
    let (func, params, ret, lw) = lowered.map_err(|ds| {
        let first = ds.into_iter().next().map(|d| d.message).unwrap_or_default();
        Box::new(
            Diagnostic::error(
                codes::NOT_EVALUATED,
                format!(
                    "`{}` cannot be called during elaboration yet",
                    db.decls[f].name
                ),
            )
            .with_primary(at, "called during elaboration")
            .with_secondary(db.decls[f].span, "declared here")
            .with_note(format!("lowering it failed: {first}")),
        )
    })?;
    let rt = yavs_interp::Runtime::new(std::sync::Arc::new(lw.program), Default::default())
        .map_err(|e| err(codes::INTERNAL, at, format!("internal error: {e}"), "here"))?;
    Ok(Lowered1 {
        rt,
        func,
        params,
        ret,
    })
}

/// Apply an element resolution indication (LRM 6.3) to the element shape `s`:
/// `(resolved) t` resolves every scalar, `((resolved)) t` the elements of nested arrays,
/// `(a f, b g) r` per record element. `false` when it does not fit.
fn apply_res(s: &mut Shape, r: &Resolution) -> bool {
    match (r, s) {
        (Resolution::Function(f), s) => {
            set_res(s, *f);
            true
        }
        (Resolution::Element(inner), Shape::Array { elem, .. }) => apply_res(elem, inner),
        (Resolution::Record(fs), Shape::Record { fields, .. }) => fs.iter().all(|(i, r)| {
            fields
                .get_mut(*i as usize)
                .is_some_and(|(_, f)| apply_res(f, r))
        }),
        _ => false,
    }
}

fn set_res(s: &mut Shape, f: DeclId) {
    match s {
        Shape::Scalar { res, .. } => *res = Some(f),
        Shape::Array { elem, .. } => set_res(elem, f),
        Shape::Record { fields, .. } => fields.iter_mut().for_each(|(_, s)| set_res(s, f)),
    }
}

pub(crate) fn int(v: &Val, span: Span) -> R<i64> {
    v.as_int().ok_or_else(|| {
        err(
            codes::INTERNAL,
            span,
            "internal error: expected a discrete value",
            "this is a bug in yavs",
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
pub fn leaf_values(db: &Db, shape: &Shape, v: Option<&Val>) -> Result<Vec<Value>, String> {
    let mut flat = Vec::new();
    flatten(shape, v, &mut flat, &mut Vec::new())?;
    let mut out = Vec::new();
    for (path, leaf, n) in shape.leaves() {
        let Shape::Scalar { elem, ty, .. } = leaf else {
            continue;
        };
        let xs: Vec<&Val> = flat
            .iter()
            .filter(|(_, p)| *p == path)
            .map(|(v, _)| v)
            .collect();
        debug_assert_eq!(xs.len() as u64, n);
        out.push(encode(*elem, !is_std_ulogic(db, *ty), &xs));
    }
    Ok(out)
}

/// Kernel bytes or planes of scalar values in row-major order. `bit01`: a BIT or BOOLEAN
/// leaf, whose positions 0/1 are stored as the std_ulogic codes of '0'/'1' (2/3,
/// docs/decisions.md).
pub fn encode(elem: Elem, bit01: bool, xs: &[&Val]) -> Value {
    match elem {
        Elem::Logic => {
            let n = xs.len();
            let off = if bit01 { 2 } else { 0 };
            Value::Logic(LogicVec::from_fn(n, |i| {
                let code = xs[n - 1 - i].as_int().unwrap_or(0) as u8 + off;
                yavs_logic::StdULogic::from_code_pub(code)
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
// parsed with the `'value` parser of yavs-std.

pub fn parse_override(
    m: &mut Model,
    db: &Db,
    g: DeclId,
    text: &str,
    scope: ScopeId,
) -> Result<Val, String> {
    use yavs_std::types as st;
    use yavs_std::value as v;
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

fn enum_type(db: &Db, lits: &[DeclId]) -> yavs_std::types::EnumType {
    use yavs_sem::decl::Designator;
    yavs_std::types::EnumType {
        lits: lits
            .iter()
            .map(|&l| match db.decls[l].name {
                Designator::Char(c) => yavs_std::types::EnumLit::Char(c as u8),
                d => yavs_std::types::EnumLit::Ident(yavs_std::types::to_latin1(&d.to_string())),
            })
            .collect(),
    }
}
