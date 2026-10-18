//! Name interpretation: simple and expanded names, `Apply` (calls, conversions,
//! attribute functions), predefined attributes (sem.md 1.5, 3.3).
//!
//! Both passes go through [`Db::classify`], which resolves the non-overloaded
//! prefix chain of a name without reporting; the top-down pass reports.

use rsim_diag::{Diagnostic, Span};
use rsim_syntax::Symbol;
use rsim_syntax::ast::{self, Ident};

use super::{CandSet, Expected, Interp, Via};
use crate::attr::PredefAttr;
use crate::db::{Db, Reported};
use crate::decl::{Alias, DeclKind, char_key, operator_key};
use crate::diag;
use crate::hir::{Actual, Arg, AttrPrefix, Expr, ExprKind};
use crate::ids::{DeclId, ExprId, ScopeId, SubtypeId, TypeId};
use crate::scope::Visible;
use crate::stat::{ConstValue, Staticness};
use crate::types::{Direction, TypeKind};
use crate::unit::{Region, suffix_key, suffix_span};

/// What a name without arguments denotes.
pub(crate) enum NameRes {
    One(DeclId),
    Overloads(Vec<DeclId>),
    /// Nothing visible: report `E1100` (or the conflict) at `ident`.
    Missing(Ident),
    Conflict(Ident, crate::scope::Conflict),
    /// A construct of a later slice.
    Unsupported(Span, &'static str, &'static str),
    /// Already reported.
    Error,
}

fn name_ident(n: &ast::Name) -> Option<(Symbol, Ident)> {
    match &n.kind {
        ast::NameKind::Simple(i) => Some((i.key, *i)),
        ast::NameKind::Operator(op) => Some((
            operator_key(op.as_str()),
            Ident {
                text: Symbol::intern(&format!("\"{op}\"")),
                key: operator_key(op.as_str()),
                span: n.span,
            },
        )),
        ast::NameKind::Character(c) => Some((
            char_key(*c),
            Ident {
                text: Symbol::intern(&format!("'{c}'")),
                key: char_key(*c),
                span: n.span,
            },
        )),
        _ => None,
    }
}

impl Db {
    /// Resolve a simple or expanded name (no arguments, no attribute).
    pub(crate) fn classify(&mut self, scope: ScopeId, n: &ast::Name) -> NameRes {
        if let Some((key, ident)) = name_ident(n) {
            return match self.lookup(scope, key) {
                Visible::One(d) => NameRes::One(d),
                Visible::Overloads(ds) => NameRes::Overloads(ds),
                Visible::None => NameRes::Missing(ident),
                Visible::Hidden(c) => NameRes::Conflict(ident, c),
            };
        }
        match &n.kind {
            ast::NameKind::Selected { prefix, suffix } => {
                let Some(key) = suffix_key(suffix) else {
                    return NameRes::Unsupported(n.span, "`.all`", "S6");
                };
                match self.classify(scope, prefix) {
                    NameRes::One(d) => {
                        let region = match self.decls[self.entity(d)].kind {
                            DeclKind::Library(l) => Region::Library(l),
                            DeclKind::Unit(u)
                                if self.units[u].kind == crate::loader::UnitKind::Package =>
                            {
                                Region::Scope(self.units[u].scope)
                            }
                            DeclKind::Object(_) | DeclKind::Alias(Alias::Object { .. }) => {
                                return NameRes::Unsupported(
                                    n.span,
                                    "selected names of objects (record elements, protected methods)",
                                    "S3",
                                );
                            }
                            DeclKind::Error => return NameRes::Error,
                            _ => {
                                return NameRes::Unsupported(
                                    n.span,
                                    "expanded names into this kind of region",
                                    "S5",
                                );
                            }
                        };
                        match region {
                            Region::Library(l) => match self.load_dep(
                                l,
                                crate::loader::UnitKey::Primary(key),
                                Some(n.span),
                            ) {
                                Ok(u) => {
                                    self.note_dep(u);
                                    NameRes::One(self.units[u].decl)
                                }
                                Err(_) => NameRes::Error,
                            },
                            Region::Scope(s) => match self.scopes[s].names.get(&key) {
                                Some(ds) if ds.len() == 1 && !self.is_overloadable(ds[0]) => {
                                    NameRes::One(ds[0])
                                }
                                Some(ds) if !ds.is_empty() => NameRes::Overloads(ds.clone()),
                                _ => {
                                    self.not_in_region(s, suffix_span(suffix, n.span), key);
                                    NameRes::Error
                                }
                            },
                        }
                    }
                    NameRes::Overloads(_) => {
                        NameRes::Unsupported(n.span, "selected names of function results", "S3")
                    }
                    other => other,
                }
            }
            ast::NameKind::External(_) => NameRes::Unsupported(n.span, "external names", "S6"),
            _ => NameRes::Unsupported(n.span, "this form of name", "S3"),
        }
    }

    /// Report why a name has no usable interpretation.
    pub(crate) fn report_name(&mut self, scope: ScopeId, r: NameRes) -> Reported {
        match r {
            NameRes::Missing(i) => self.undeclared(scope, i),
            NameRes::Conflict(i, c) => self.conflict(i, &c),
            NameRes::Unsupported(span, what, slice) => self.unsupported(span, what, slice),
            NameRes::Error => Reported,
            NameRes::One(_) | NameRes::Overloads(_) => Reported,
        }
    }

    /// The subtype a type mark name denotes, without reporting.
    pub(crate) fn quiet_type_mark(&mut self, scope: ScopeId, n: &ast::Name) -> Option<SubtypeId> {
        match self.classify(scope, n) {
            NameRes::One(d) => match self.decls[self.entity(d)].kind {
                DeclKind::Type(t) | DeclKind::GenericType(t) => Some(self.types[t].first_subtype),
                DeclKind::Subtype(s) => Some(s),
                _ => None,
            },
            _ => None,
        }
    }

    /// Bottom-up candidates of a name in an expression.
    pub(crate) fn name_cands(&mut self, scope: ScopeId, n: &ast::Name) -> CandSet {
        match &n.kind {
            ast::NameKind::Attribute { prefix, attr } => self.attr_cands(scope, prefix, *attr, 0),
            ast::NameKind::Apply { prefix, args } => self.apply_cands(scope, prefix, args),
            _ => match self.classify(scope, n) {
                NameRes::One(d) => match self.object_type(d) {
                    Some(t) => CandSet::Types(vec![Interp::new(t, Via::Object)]),
                    None => match self.decls[self.entity(d)].kind {
                        DeclKind::PhysUnit { ty, .. } => {
                            CandSet::Types(vec![Interp::new(ty, Via::Literal)])
                        }
                        DeclKind::Error => self.poison_cands(),
                        _ => CandSet::Types(Vec::new()),
                    },
                },
                NameRes::Overloads(ds) => {
                    CandSet::Types(ds.iter().filter_map(|&d| self.value_interp(d, 0)).collect())
                }
                NameRes::Error => self.poison_cands(),
                _ => CandSet::Types(Vec::new()),
            },
        }
    }

    /// The type of an object or object alias.
    pub(crate) fn object_type(&self, d: DeclId) -> Option<TypeId> {
        match &self.decls[d].kind {
            DeclKind::Object(o) => Some(self.base(o.subtype)),
            DeclKind::Alias(Alias::Object { target, subtype }) => {
                Some(self.base(subtype.unwrap_or(self.exprs[*target].subtype)))
            }
            _ => None,
        }
    }

    /// Result type of predefined attribute `attr` of a type mark prefix.
    fn attr_cands(
        &mut self,
        scope: ScopeId,
        prefix: &ast::Name,
        attr: Ident,
        nargs: usize,
    ) -> CandSet {
        let Some(st) = self.quiet_type_mark(scope, prefix) else {
            return CandSet::Types(Vec::new());
        };
        let t = self.base(st);
        let Some(a) = PredefAttr::from_key(attr.key) else {
            return CandSet::Types(Vec::new());
        };
        let w = self.well_known.clone();
        use PredefAttr::*;
        let (ty, convertible) = match (a, nargs) {
            (Left | Right | High | Low, 0) => (Some(t), false),
            (Ascending, 0) => (w.boolean, false),
            (Image, 1) => (w.string, false),
            (Value | Val | Succ | Pred | Leftof | Rightof, 1) => (Some(t), false),
            (Pos, 1) => (w.universal_integer, true),
            _ => (None, false),
        };
        match ty {
            Some(ty) => CandSet::Types(vec![Interp {
                convertible,
                ..Interp::new(ty, Via::Attr)
            }]),
            None => CandSet::Types(Vec::new()),
        }
    }

    fn apply_cands(
        &mut self,
        scope: ScopeId,
        prefix: &ast::Name,
        args: &[ast::AssocElem],
    ) -> CandSet {
        if let ast::NameKind::Attribute { prefix: p, attr } = &prefix.kind {
            return self.attr_cands(scope, p, *attr, args.len());
        }
        if let Some(st) = self.quiet_type_mark(scope, prefix) {
            // Type conversion (9.3.6).
            return CandSet::Types(vec![Interp::new(self.base(st), Via::Other)]);
        }
        match self.classify(scope, prefix) {
            NameRes::Overloads(ds) => {
                let mut out = Vec::new();
                for d in ds {
                    if let Some(i) = self.call_interp(scope, d, args) {
                        out.push(i);
                    }
                }
                CandSet::Types(out)
            }
            NameRes::Error => self.poison_cands(),
            _ => CandSet::Types(Vec::new()),
        }
    }

    /// The interpretation of function `d` applied to `args`, if the association fits.
    fn call_interp(
        &mut self,
        scope: ScopeId,
        d: DeclId,
        args: &[ast::AssocElem],
    ) -> Option<Interp> {
        let e = self.entity(d);
        let s = self.decls[e].kind.subprogram()?;
        let ret = self.base(s.ret?);
        let formals: Vec<(Symbol, TypeId, bool)> = s
            .params
            .iter()
            .map(|p| (p.name.key, self.base(p.subtype), p.default.is_some()))
            .collect();
        let map = assoc_map(&formals.iter().map(|f| f.0).collect::<Vec<_>>(), args)?;
        let mut convs = 0;
        for (i, (_, ft, has_default)) in formals.iter().enumerate() {
            match map[i] {
                Some(a) => {
                    let ast::Actual::Expr(x) = &args[a].actual else {
                        return None;
                    };
                    let c = self.cands(scope, x);
                    convs += self.best_cost(&c, Expected::Type(*ft))?;
                }
                None if *has_default => {}
                None => return None,
            }
        }
        Some(Interp {
            convs,
            ..Interp::new(ret, Via::Call(d))
        })
    }

    // -- top-down ---------------------------------------------------------------

    /// Build the HIR of name `n` for the chosen interpretation.
    pub(crate) fn build_name(
        &mut self,
        scope: ScopeId,
        n: &ast::Name,
        i: Interp,
        span: Span,
    ) -> ExprId {
        match &n.kind {
            ast::NameKind::Attribute { prefix, attr } => {
                self.build_attr(scope, prefix, *attr, &[], i, span)
            }
            ast::NameKind::Apply { prefix, args } => {
                if let ast::NameKind::Attribute { prefix: p, attr } = &prefix.kind {
                    return self.build_attr(scope, p, *attr, args, i, span);
                }
                if let Some(st) = self.quiet_type_mark(scope, prefix) {
                    return self.build_conversion(scope, st, args, span);
                }
                match i.via {
                    Via::Call(d) => self.build_call(scope, d, args, span, None),
                    _ => self.poison_expr(),
                }
            }
            _ => match i.via {
                Via::Call(d) => self.build_call(scope, d, &[], span, None),
                Via::EnumLit(d) => {
                    let DeclKind::EnumLit { ty, pos } = self.decls[d].kind else {
                        return self.poison_expr();
                    };
                    let st = self.types[ty].first_subtype;
                    self.lit_expr(ConstValue::Enum(pos), st, span)
                }
                Via::Literal => match self.classify(scope, n) {
                    NameRes::One(d) => match self.decls[self.entity(d)].kind {
                        DeclKind::PhysUnit { ty, index } => {
                            let scale = match &self.types[ty].kind {
                                TypeKind::Physical { units } => units[index as usize].scale,
                                _ => return self.poison_expr(),
                            };
                            let st = self.types[ty].first_subtype;
                            self.lit_expr(ConstValue::Phys(scale), st, span)
                        }
                        _ => self.poison_expr(),
                    },
                    _ => self.poison_expr(),
                },
                Via::Object => match self.classify(scope, n) {
                    NameRes::One(d) => self.object_expr(d, span),
                    _ => self.poison_expr(),
                },
                _ => self.poison_expr(),
            },
        }
    }

    pub(crate) fn lit_expr(&mut self, v: ConstValue, st: SubtypeId, span: Span) -> ExprId {
        self.const_expr(v, st, span)
    }

    pub(crate) fn poison_expr(&self) -> ExprId {
        self.poison.expr
    }

    /// A reference to an object; constants carry their folded value (9.4.1).
    pub(crate) fn object_ref(&mut self, d: DeclId, span: Span) -> ExprId {
        self.object_expr(d, span)
    }

    fn object_expr(&mut self, d: DeclId, span: Span) -> ExprId {
        let (subtype, stat, value) = match &self.decls[d].kind {
            DeclKind::Object(o) => {
                let (stat, value) = match (o.class, &o.role, o.default) {
                    (crate::decl::ObjClass::Constant, crate::decl::ObjRole::Plain, Some(init)) => {
                        let e = &self.exprs[init];
                        (
                            e.stat,
                            if e.stat == Staticness::Locally {
                                e.value
                            } else {
                                None
                            },
                        )
                    }
                    (crate::decl::ObjClass::Constant, _, _) => (Staticness::Globally, None),
                    _ => (Staticness::None, None),
                };
                (o.subtype, stat, value)
            }
            DeclKind::Alias(Alias::Object { target, subtype }) => {
                let e = &self.exprs[*target];
                (subtype.unwrap_or(e.subtype), e.stat, e.value)
            }
            _ => return self.poison_expr(),
        };
        self.add_expr(Expr {
            kind: ExprKind::Obj(d),
            subtype,
            span,
            stat,
            value,
        })
    }

    /// A function call with named or positional association; defaults are `Actual::Default`.
    pub(crate) fn build_call(
        &mut self,
        scope: ScopeId,
        d: DeclId,
        args: &[ast::AssocElem],
        span: Span,
        method_of: Option<ExprId>,
    ) -> ExprId {
        let e = self.entity(d);
        let Some(s) = self.decls[e].kind.subprogram() else {
            return self.poison_expr();
        };
        let formals: Vec<(Symbol, TypeId)> = s
            .params
            .iter()
            .map(|p| (p.name.key, self.base(p.subtype)))
            .collect();
        let ret = s.ret.unwrap_or(self.poison.subtype);
        let implicit = matches!(s.body, crate::decl::Body::Implicit(_));
        let Some(map) = assoc_map(&formals.iter().map(|f| f.0).collect::<Vec<_>>(), args) else {
            return self.poison_expr();
        };
        let mut hargs = Vec::with_capacity(formals.len());
        let mut stat = if implicit {
            Staticness::Locally
        } else {
            Staticness::None
        };
        for (k, (_, ft)) in formals.iter().enumerate() {
            let actual = match map[k] {
                Some(a) => match &args[a].actual {
                    ast::Actual::Expr(x) => {
                        let id = self.resolve(scope, x, Expected::Type(*ft));
                        stat = stat.min(self.exprs[id].stat);
                        Actual::Expr(id)
                    }
                    ast::Actual::Open(_) => Actual::Open,
                    other => {
                        let sp = actual_span(other).unwrap_or(span);
                        self.unsupported(sp, "this kind of actual", "S2");
                        return self.poison_expr();
                    }
                },
                None => Actual::Default,
            };
            hargs.push(Arg {
                formal: k as u16,
                actual,
            });
        }
        let id = self.add_expr(Expr {
            kind: ExprKind::Call {
                callee: e,
                args: hargs.into(),
                method_of,
                implicit_cond: false,
            },
            subtype: ret,
            span,
            stat,
            value: None,
        });
        if implicit && stat == Staticness::Locally {
            self.fold_into(id);
        }
        id
    }

    fn build_conversion(
        &mut self,
        scope: ScopeId,
        st: SubtypeId,
        args: &[ast::AssocElem],
        span: Span,
    ) -> ExprId {
        let [
            ast::AssocElem {
                formal: None,
                actual: ast::Actual::Expr(x),
                ..
            },
        ] = args
        else {
            return Expr::poison_of(self, span, "a type conversion takes one operand");
        };
        let operand = self.resolve(scope, x, Expected::Any);
        let stat = self.exprs[operand].stat;
        let id = self.add_expr(Expr {
            kind: ExprKind::Conv { to: st, operand },
            subtype: st,
            span,
            stat,
            value: None,
        });
        if stat == Staticness::Locally {
            self.fold_into(id);
        }
        id
    }

    /// Scalar attributes of a type mark: `T'high`, `T'pos(x)`, `T'image(x)`, ...
    fn build_attr(
        &mut self,
        scope: ScopeId,
        prefix: &ast::Name,
        attr: Ident,
        args: &[ast::AssocElem],
        i: Interp,
        span: Span,
    ) -> ExprId {
        let Some(st) = self.quiet_type_mark(scope, prefix) else {
            return self.poison_expr();
        };
        let Some(a) = PredefAttr::from_key(attr.key) else {
            return self.poison_expr();
        };
        let t = self.base(st);
        let mut hargs = Vec::new();
        let mut stat = self.subtype_staticness(st);
        for arg in args {
            let ast::Actual::Expr(x) = &arg.actual else {
                return self.poison_expr();
            };
            let want = match a {
                PredefAttr::Value => {
                    Expected::Type(self.well_known.string.unwrap_or(self.poison.ty))
                }
                PredefAttr::Val => {
                    Expected::Type(self.well_known.universal_integer.unwrap_or(self.poison.ty))
                }
                _ => Expected::Type(t),
            };
            let id = if a == PredefAttr::Val {
                self.resolve_integer(scope, x)
            } else {
                self.resolve(scope, x, want)
            };
            stat = stat.min(self.exprs[id].stat);
            hargs.push(id);
        }
        let subtype = self.types[i.ty].first_subtype;
        let id = self.add_expr(Expr {
            kind: ExprKind::Attr {
                attr: a,
                prefix: AttrPrefix::Subtype(st),
                args: hargs.into(),
            },
            subtype,
            span,
            stat,
            value: None,
        });
        if stat == Staticness::Locally {
            self.fold_into(id);
        }
        id
    }

    /// `T'val(x)` takes any integer type.
    fn resolve_integer(&mut self, scope: ScopeId, x: &ast::Expr) -> ExprId {
        self.resolve(scope, x, Expected::Any)
    }

    /// A subtype is locally static when its range bounds are (9.4.2, scalar case).
    pub(crate) fn subtype_staticness(&self, st: SubtypeId) -> Staticness {
        match self.scalar_range(st) {
            Some(crate::types::RangeC::Explicit { left, right, .. }) => {
                self.exprs[left].stat.min(self.exprs[right].stat)
            }
            Some(crate::types::RangeC::Attr { .. }) => Staticness::Globally,
            None => Staticness::Locally,
        }
    }

    /// `high` bound helper for attribute folding.
    pub(crate) fn bound(&self, st: SubtypeId, which: PredefAttr) -> Option<ConstValue> {
        let (l, dir, r) = self.static_bounds(st)?;
        Some(match (which, dir) {
            (PredefAttr::Left, _) => l,
            (PredefAttr::Right, _) => r,
            (PredefAttr::High, Direction::To) | (PredefAttr::Low, Direction::Downto) => r,
            (PredefAttr::Low, Direction::To) | (PredefAttr::High, Direction::Downto) => l,
            _ => return None,
        })
    }
}

impl Expr {
    fn poison_of(db: &mut Db, span: Span, msg: &str) -> ExprId {
        db.report(
            Diagnostic::error(diag::BAD_CONVERSION, msg.to_string()).with_primary(span, "here"),
        );
        db.poison.expr
    }
}

fn actual_span(a: &ast::Actual) -> Option<Span> {
    match a {
        ast::Actual::Expr(e) | ast::Actual::Inertial(e) => Some(e.span),
        ast::Actual::Open(s) => Some(*s),
        ast::Actual::Subtype(si) => Some(si.span),
        ast::Actual::Range(_) => None,
    }
}

/// Map formals (by key) to association indexes: positional first, then named.
/// `None` if the association does not fit (too many, unknown or repeated name).
// ponytail: named association by simple formal name only; sub-element and conversion
// formals (6.5.7.1) come with S5.
pub(crate) fn assoc_map(formals: &[Symbol], args: &[ast::AssocElem]) -> Option<Vec<Option<usize>>> {
    let mut map = vec![None; formals.len()];
    let mut named = false;
    for (k, a) in args.iter().enumerate() {
        match &a.formal {
            None => {
                if named || k >= formals.len() {
                    return None;
                }
                map[k] = Some(k);
            }
            Some(f) => {
                named = true;
                let ast::NameKind::Simple(i) = &f.kind else {
                    return None;
                };
                let pos = formals.iter().position(|&x| x == i.key)?;
                if map[pos].is_some() {
                    return None;
                }
                map[pos] = Some(k);
            }
        }
    }
    Some(map)
}
