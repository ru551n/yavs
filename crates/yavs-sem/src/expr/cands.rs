//! Bottom-up candidate sets (sem.md 3.1).
//!
//! `cands(e)` is the set of possible result types of AST expression `e`,
//! memoized per AST node address during one complete context, so each node is
//! visited once. An operator keeps only the overloads whose every operand can
//! match its formal, which keeps `a & b & c & ...` linear.

use yavs_syntax::ast::{self, BinaryOp, UnaryOp};

use super::{CandSet, ClassMask, Expected, Interp, Via};
use crate::db::Db;
use crate::decl::{DeclKind, operator_key};
use crate::ids::{DeclId, Id, ScopeId, TypeId};
use crate::scope::Visible;
use crate::types::TypeKind;

pub fn unary_symbol(op: UnaryOp) -> &'static str {
    match op {
        UnaryOp::Plus => "+",
        UnaryOp::Minus => "-",
        UnaryOp::Not => "not",
        UnaryOp::Abs => "abs",
        UnaryOp::Condition => "??",
        UnaryOp::And => "and",
        UnaryOp::Or => "or",
        UnaryOp::Nand => "nand",
        UnaryOp::Nor => "nor",
        UnaryOp::Xor => "xor",
        UnaryOp::Xnor => "xnor",
    }
}

pub fn binary_symbol(op: BinaryOp) -> &'static str {
    use BinaryOp::*;
    match op {
        And => "and",
        Or => "or",
        Nand => "nand",
        Nor => "nor",
        Xor => "xor",
        Xnor => "xnor",
        Eq => "=",
        Ne => "/=",
        Lt => "<",
        Le => "<=",
        Gt => ">",
        Ge => ">=",
        MatchEq => "?=",
        MatchNe => "?/=",
        MatchLt => "?<",
        MatchLe => "?<=",
        MatchGt => "?>",
        MatchGe => "?>=",
        Sll => "sll",
        Srl => "srl",
        Sla => "sla",
        Sra => "sra",
        Rol => "rol",
        Ror => "ror",
        Add => "+",
        Sub => "-",
        Concat => "&",
        Mul => "*",
        Div => "/",
        Mod => "mod",
        Rem => "rem",
        Pow => "**",
    }
}

const FIRST_NONE: u32 = 1;
const FIRST_WILD: u32 = 2;
const FIRST_TYPE: u32 = 3;

impl Db {
    pub(crate) fn cands(&mut self, scope: ScopeId, e: &ast::Expr) -> CandSet {
        let key = e as *const ast::Expr as usize;
        if let Some(c) = self.memo.get(&key) {
            return c.clone();
        }
        let c = self.cands_uncached(scope, e);
        self.memo.insert(key, c.clone());
        c
    }

    /// Poison: compatible with everything, so no follow-up diagnostics.
    pub(crate) fn poison_cands(&self) -> CandSet {
        CandSet::Types(smallvec::smallvec![Interp::new(self.poison.ty, Via::Other)])
    }

    fn cands_uncached(&mut self, scope: ScopeId, e: &ast::Expr) -> CandSet {
        use ast::ExprKind as K;
        match &e.kind {
            K::Literal(l) => self.literal_cands(scope, l),
            K::Paren(inner) => self.cands(scope, inner),
            K::Name(n) => self.name_cands(scope, n),
            K::Aggregate(_) => CandSet::Class(ClassMask::ANY_COMPOSITE),
            K::New(a) => {
                let mark = match &**a {
                    ast::Allocator::Subtype(si) => Some(&si.type_mark),
                    ast::Allocator::Qualified(q) => match &q.kind {
                        K::Qualified { type_mark, .. } => Some(&**type_mark),
                        _ => None,
                    },
                };
                match mark.and_then(|m| self.quiet_type_mark(scope, m)) {
                    Some(st) => CandSet::Alloc(self.base(st)),
                    None => CandSet::Class(ClassMask::ANY_ACCESS),
                }
            }
            K::Qualified { type_mark, .. } => match self.type_mark(scope, type_mark) {
                Ok(st) => {
                    CandSet::Types(smallvec::smallvec![Interp::new(self.base(st), Via::Other)])
                }
                Err(_) => self.poison_cands(),
            },
            K::Unary { op, operand } => {
                let ops = [operand.as_ref()];
                self.operator_cands(scope, unary_symbol(*op), &ops)
            }
            K::Binary { op, left, right } => {
                let ops = [left.as_ref(), right.as_ref()];
                let mut c = self.operator_cands(scope, binary_symbol(*op), &ops);
                // 9.3.6: the division of two values of one physical type is convertible.
                if *op == BinaryOp::Div
                    && let CandSet::Types(is) = &mut c
                {
                    for i in is.iter_mut() {
                        if Some(i.ty) == self.well_known.universal_integer {
                            i.convertible = true;
                        }
                    }
                }
                c
            }
            // 2019: the types every alternative can have.
            K::Conditional(alts) => {
                let mut acc: Option<CandSet> = None;
                for (v, _) in alts {
                    let c = self.cands(scope, v);
                    acc = Some(match acc {
                        None => c,
                        Some(a) => self.meet(a, c),
                    });
                }
                acc.unwrap_or_else(|| self.poison_cands())
            }
            K::Unaffected | K::Error => self.poison_cands(),
        }
    }

    /// The interpretations two candidate sets share (a conditional expression's alternatives).
    fn meet(&self, a: CandSet, b: CandSet) -> CandSet {
        if self.is_poison_set(&a) {
            return b;
        }
        if self.is_poison_set(&b) {
            return a;
        }
        match (a, b) {
            (CandSet::Class(x), CandSet::Class(y)) => CandSet::Class(ClassMask(x.0 & y.0)),
            (CandSet::Alloc(x), CandSet::Alloc(y)) if x == y => CandSet::Alloc(x),
            (CandSet::Alloc(d), CandSet::Class(m)) | (CandSet::Class(m), CandSet::Alloc(d))
                if m.contains(ClassMask::ANY_ACCESS) =>
            {
                CandSet::Alloc(d)
            }
            (CandSet::Alloc(_), CandSet::Alloc(_) | CandSet::Class(_))
            | (CandSet::Class(_), CandSet::Alloc(_)) => CandSet::Class(ClassMask(0)),
            (CandSet::Types(is), CandSet::Alloc(d)) | (CandSet::Alloc(d), CandSet::Types(is)) => {
                CandSet::Types(
                    is.into_iter()
                        .filter(|i| self.alloc_admits(d, i.ty))
                        .collect(),
                )
            }
            (CandSet::Types(is), CandSet::Class(m)) | (CandSet::Class(m), CandSet::Types(is)) => {
                CandSet::Types(
                    is.into_iter()
                        .filter(|i| self.class_admits(m, i.ty))
                        .collect(),
                )
            }
            (CandSet::Types(xs), CandSet::Types(ys)) => {
                let mut out = super::Interps::new();
                for x in &xs {
                    for y in &ys {
                        // A convertible universal side takes the other side's type.
                        let i = if x.ty == y.ty {
                            Interp {
                                convertible: x.convertible && y.convertible,
                                convs: x.convs.max(y.convs),
                                ..Interp::new(x.ty, Via::Other)
                            }
                        } else if self.cost(x, super::Expected::Type(y.ty)).is_some() {
                            Interp::new(y.ty, Via::Other)
                        } else if self.cost(y, super::Expected::Type(x.ty)).is_some() {
                            Interp::new(x.ty, Via::Other)
                        } else {
                            continue;
                        };
                        if !out.iter().any(|o| o.ty == i.ty) {
                            out.push(i);
                        }
                    }
                }
                CandSet::Types(out)
            }
        }
    }

    fn literal_cands(&mut self, scope: ScopeId, l: &ast::Literal) -> CandSet {
        use ast::LiteralKind as L;
        let ui = self.well_known.universal_integer;
        let ur = self.well_known.universal_real;
        match &l.kind {
            L::Abstract(t) => {
                let ty = if crate::expr::literal::abstract_literal(t.as_str())
                    .is_ok_and(|a| matches!(a, crate::expr::literal::Abs::Real(_)))
                {
                    ur
                } else {
                    ui
                };
                match ty {
                    Some(ty) => CandSet::Types(smallvec::smallvec![Interp {
                        convertible: true,
                        ..Interp::new(ty, Via::Literal)
                    }]),
                    None => self.poison_cands(),
                }
            }
            L::Char(c) => self.overload_cands(scope, crate::decl::char_key(*c)),
            L::String(_) | L::BitString(_) => CandSet::Class(ClassMask::STRING_LIKE),
            L::Null => CandSet::Class(ClassMask::ANY_ACCESS),
            L::Physical { unit, .. } => match self.lookup(scope, unit.key) {
                Visible::One(d) => match self.decls[self.entity(d)].kind {
                    DeclKind::PhysUnit { ty, .. } => {
                        CandSet::Types(smallvec::smallvec![Interp::new(ty, Via::Literal)])
                    }
                    _ => self.not_a_unit(*unit),
                },
                Visible::None => {
                    self.undeclared(scope, *unit);
                    self.poison_cands()
                }
                _ => self.not_a_unit(*unit),
            },
        }
    }

    /// A physical literal's unit that names no unit: reported, never silent (poison).
    fn not_a_unit(&mut self, unit: yavs_syntax::ast::Ident) -> CandSet {
        self.error(
            yavs_diag::Diagnostic::error(
                crate::diag::BAD_LITERAL,
                format!("`{}` is not a physical unit", unit.text),
            )
            .with_primary(unit.span, "not a unit"),
        );
        self.poison_cands()
    }

    /// Candidates of a name that denotes only overloadables (character literals).
    fn overload_cands(&mut self, scope: ScopeId, key: yavs_syntax::Symbol) -> CandSet {
        match self.lookup(scope, key) {
            Visible::Overloads(ds) => {
                CandSet::Types(ds.iter().filter_map(|&d| self.value_interp(d, 0)).collect())
            }
            _ => self.poison_cands(),
        }
    }

    /// The interpretation of overloadable `d` called with `nargs` positional
    /// arguments, ignoring argument types (enum literals take none).
    pub(crate) fn value_interp(&self, d: DeclId, nargs: usize) -> Option<Interp> {
        let e = self.entity(d);
        match &self.decls[e].kind {
            DeclKind::EnumLit { ty, .. } if nargs == 0 => Some(Interp::new(*ty, Via::EnumLit(e))),
            DeclKind::Subprogram(s) | DeclKind::GenericSubprogram(s) => {
                let ret = s.ret?;
                let required = s.params.iter().filter(|p| p.default.is_none()).count();
                (nargs >= required && nargs <= s.params.len())
                    .then(|| Interp::new(self.base(ret), Via::Call(d)))
            }
            _ => None,
        }
    }

    /// The overloads of operator `sym` whose operands can all match.
    fn operator_cands(&mut self, scope: ScopeId, sym: &str, operands: &[&ast::Expr]) -> CandSet {
        let ocs: smallvec::SmallVec<[CandSet; 2]> =
            operands.iter().map(|o| self.cands(scope, o)).collect();
        if ocs.iter().any(|c| self.is_poison_set(c)) {
            return self.poison_cands();
        }
        let Visible::Overloads(ds) = self.lookup(scope, operator_key(sym)) else {
            return CandSet::Types(Default::default());
        };
        let mut out = super::Interps::new();
        // An operator set holds hundreds of declarations (every visible type has its own
        // `=`), and an operand of a known type fits those of that type and the poison or
        // error type: the first parameter's type decides before the declaration is opened.
        // Not for short sets, a universal literal operand that converts to any numeric type,
        // a literal class, or anonymous generic parameter types.
        let by_first = ds.len() >= 12
            && self.generic.implicit.is_empty()
            && match &ocs[0] {
                CandSet::Types(is) => !is.iter().any(|i| i.convertible),
                CandSet::Class(_) | CandSet::Alloc(_) => false,
            };
        for &d in ds.iter() {
            if by_first {
                let CandSet::Types(is) = &ocs[0] else {
                    unreachable!("by_first needs a type set")
                };
                match self.first_param_key(d) {
                    FIRST_NONE => continue,
                    FIRST_WILD => {}
                    k if is.iter().any(|i| FIRST_TYPE + i.ty.index() as u32 == k) => {}
                    _ => continue,
                }
            }
            if let Some(i) = self.operator_interp(d, &ocs) {
                out.push(i);
            }
        }
        #[cfg(debug_assertions)]
        if by_first {
            let all: super::Interps = ds
                .iter()
                .filter_map(|&d| self.operator_interp(d, &ocs))
                .collect();
            assert_eq!(
                out, all,
                "first parameter filter misses a candidate of `{sym}`"
            );
        }
        CandSet::Types(out)
    }

    /// What decides whether overload `d` can take an operand of type `t` as its first
    /// operand: [`FIRST_NONE`] (not a subprogram with a result and a parameter),
    /// [`FIRST_WILD`] (the parameter has the poison or an error type: fits every operand) or
    /// `FIRST_TYPE + ` the index of its base type. Kept per declaration once it is a
    /// subprogram (its profile never changes after).
    fn first_param_key(&mut self, d: DeclId) -> u32 {
        if let Some(&k) = self.first_param.get(d.index())
            && k != 0
        {
            return k;
        }
        let e = self.entity(d);
        let (DeclKind::Subprogram(s) | DeclKind::GenericSubprogram(s)) = &self.decls[e].kind else {
            // Not (yet) a subprogram: a placeholder may still become one.
            return FIRST_NONE;
        };
        let k = match (s.ret, s.params.first()) {
            (Some(_), Some(p)) => {
                let f = self.base(p.subtype);
                if f == self.poison.ty || matches!(self.types[f].kind, TypeKind::Error) {
                    FIRST_WILD
                } else {
                    FIRST_TYPE + f.index() as u32
                }
            }
            _ => FIRST_NONE,
        };
        if self.first_param.len() <= d.index() {
            self.first_param
                .resize(self.decls.len().max(d.index() + 1), 0);
        }
        self.first_param[d.index()] = k;
        k
    }

    /// The interpretation of operator overload `d` for operands with candidates `ocs`.
    fn operator_interp(&self, d: DeclId, ocs: &[CandSet]) -> Option<Interp> {
        let e = self.entity(d);
        let s = self.decls[e].kind.subprogram()?;
        let ret = s.ret?;
        if s.params.len() != ocs.len() {
            return None;
        }
        let mut ret = self.base(ret);
        let mut convs = 0;
        // An anonymous parameter type takes any type of its class; a result of such a
        // type is the type its operand binds (#308).
        let mut bound: Vec<(TypeId, TypeId)> = Vec::new();
        for (c, p) in ocs.iter().zip(&s.params) {
            let f = self.base(p.subtype);
            let cost = if self.anonymous_param(e, f) {
                if !bound.iter().any(|b| b.0 == f)
                    && let Some(t) = self.anonymous_binding(c, f)
                {
                    bound.push((f, t));
                }
                self.anonymous_cost(c, f)
            } else {
                self.best_cost(c, Expected::Type(f))
            };
            convs += cost?;
        }
        if self.anonymous_param(e, ret) {
            ret = bound.iter().find(|b| b.0 == ret)?.1;
        }
        Some(Interp {
            convs,
            ..Interp::new(ret, Via::Call(d))
        })
    }

    pub(crate) fn is_poison_set(&self, c: &CandSet) -> bool {
        matches!(c, CandSet::Types(v) if v.iter().any(|i| i.ty == self.poison.ty))
    }

    /// Cost of using interpretation `i` where `exp` is wanted, or `None` if it cannot.
    pub(crate) fn cost(&self, i: &Interp, exp: Expected) -> Option<u32> {
        let poison = self.poison.ty;
        match exp {
            Expected::Any => Some(i.convs),
            Expected::Condition => self
                .well_known
                .boolean
                .filter(|&b| b == i.ty || i.ty == poison)
                .map(|_| i.convs),
            Expected::Discrete => self.types[i.ty].kind.is_discrete().then_some(i.convs),
            Expected::Composite => (i.ty == poison
                || matches!(
                    self.types[i.ty].kind,
                    TypeKind::Array { .. } | TypeKind::Record { .. } | TypeKind::Error
                ))
            .then_some(i.convs),
            Expected::Type(t) => {
                if i.ty == t || i.ty == poison || t == poison {
                    return Some(i.convs);
                }
                if matches!(self.types[t].kind, TypeKind::Error) {
                    return Some(i.convs);
                }
                let ui = self.well_known.universal_integer;
                let ur = self.well_known.universal_real;
                let target = &self.types[t].kind;
                // A generic type of the integer or floating class is such a type (2019 6.5.3).
                use crate::generic::GenericClass as G;
                if i.convertible
                    && ((Some(i.ty) == ui
                        && matches!(
                            target,
                            TypeKind::Integer | TypeKind::Generic { class: G::Integer }
                        ))
                        || (Some(i.ty) == ur
                            && matches!(
                                target,
                                TypeKind::Floating | TypeKind::Generic { class: G::Floating }
                            )))
                {
                    return Some(i.convs + 1);
                }
                None
            }
        }
    }

    /// The cheapest way a candidate set can satisfy `exp`.
    pub(crate) fn best_cost(&self, c: &CandSet, exp: Expected) -> Option<u32> {
        match c {
            CandSet::Types(is) => is.iter().filter_map(|i| self.cost(i, exp)).min(),
            CandSet::Class(m) => match exp {
                Expected::Type(t) => self.class_admits(*m, t).then_some(0),
                _ => None,
            },
            CandSet::Alloc(d) => match exp {
                Expected::Type(t) => self.alloc_admits(*d, t).then_some(0),
                _ => None,
            },
        }
    }

    /// The cheapest way a candidate set can be an actual of anonymous parameter type `t`
    /// (any type of its class; a universal value converts to INTEGER or REAL; the class is
    /// checked in full by the call, #283). Dearer than an exact match of another overload.
    pub(crate) fn anonymous_cost(&self, c: &CandSet, t: TypeId) -> Option<u32> {
        let CandSet::Types(is) = c else {
            return None;
        };
        is.iter()
            .filter_map(|i| {
                let ui = Some(i.ty) == self.well_known.universal_integer;
                let ur = Some(i.ty) == self.well_known.universal_real;
                let own = if ui {
                    self.well_known.integer
                } else if ur {
                    self.well_known.real
                } else {
                    Some(i.ty)
                };
                let ok = i.ty == self.poison.ty || own.is_some_and(|o| self.class_fits(t, o));
                ok.then_some(i.convs + 1 + u32::from(ui || ur))
            })
            .min()
    }

    /// The type an actual with candidates `c` gives anonymous parameter type `t`: its only
    /// interpretation (a universal value as INTEGER or REAL), if it fits the class.
    pub(crate) fn anonymous_binding(&self, c: &CandSet, t: TypeId) -> Option<TypeId> {
        let CandSet::Types(is) = c else {
            return None;
        };
        let mut own: Vec<TypeId> = is
            .iter()
            .filter_map(|i| {
                if Some(i.ty) == self.well_known.universal_integer {
                    self.well_known.integer
                } else if Some(i.ty) == self.well_known.universal_real {
                    self.well_known.real
                } else {
                    Some(i.ty)
                }
            })
            .filter(|&o| self.class_fits(t, o))
            .collect();
        own.sort();
        own.dedup();
        (own.len() == 1).then(|| own[0])
    }

    /// Whether an allocator of designated base type `d` admits type `t`: an access type
    /// designating `d` (9.3.7), or the error type.
    pub(crate) fn alloc_admits(&self, d: TypeId, t: TypeId) -> bool {
        matches!(self.types[t].kind, TypeKind::Error)
            || self.designated(t).is_some_and(|x| self.base(x) == d)
    }

    /// Whether literal class `m` (aggregate, string, null, ...) admits type `t`.
    pub(crate) fn class_admits(&self, m: ClassMask, t: TypeId) -> bool {
        match &self.types[t].kind {
            TypeKind::Error => true,
            TypeKind::Access { .. } => m.contains(ClassMask::ANY_ACCESS),
            TypeKind::Record { .. } => m.contains(ClassMask::ANY_COMPOSITE),
            TypeKind::Array { indexes, elem } => {
                m.contains(ClassMask::ANY_COMPOSITE)
                    || (m.contains(ClassMask::STRING_LIKE)
                        && indexes.len() == 1
                        && match &self.types[self.base(*elem)].kind {
                            TypeKind::Enum { lits } => lits.iter().any(|&l| {
                                matches!(self.decls[l].name, crate::decl::Designator::Char(_))
                            }),
                            _ => false,
                        })
            }
            _ => false,
        }
    }

    /// Discrete types a range bound may have (for `l to r` without a type mark).
    pub(crate) fn discrete_cands(&mut self, scope: ScopeId, e: &ast::Expr) -> Vec<TypeId> {
        let saved = (
            std::mem::take(&mut self.memo),
            std::mem::take(&mut self.name_memo),
        );
        let c = self.cands(scope, e);
        let out = match c {
            CandSet::Types(is) => is
                .iter()
                .filter(|i| i.ty == self.poison.ty || self.types[i.ty].kind.is_discrete())
                .map(|i| i.ty)
                .collect(),
            CandSet::Class(_) | CandSet::Alloc(_) => Vec::new(),
        };
        (self.memo, self.name_memo) = saved;
        out
    }
}
