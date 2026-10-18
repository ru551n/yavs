//! Bottom-up candidate sets (sem.md 3.1).
//!
//! `cands(e)` is the set of possible result types of AST expression `e`,
//! memoized per AST node address during one complete context, so each node is
//! visited once. An operator keeps only the overloads whose every operand can
//! match its formal, which keeps `a & b & c & ...` linear.

use rsim_syntax::ast::{self, BinaryOp, UnaryOp};

use super::{CandSet, ClassMask, Expected, Interp, Via};
use crate::db::Db;
use crate::decl::{DeclKind, operator_key};
use crate::ids::{DeclId, ScopeId, TypeId};
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
        CandSet::Types(vec![Interp::new(self.poison.ty, Via::Other)])
    }

    fn cands_uncached(&mut self, scope: ScopeId, e: &ast::Expr) -> CandSet {
        use ast::ExprKind as K;
        match &e.kind {
            K::Literal(l) => self.literal_cands(scope, l),
            K::Paren(inner) => self.cands(scope, inner),
            K::Name(n) => self.name_cands(scope, n),
            K::Aggregate(_) => CandSet::Class(ClassMask::ANY_COMPOSITE),
            K::New(_) => CandSet::Class(ClassMask::ANY_ACCESS),
            K::Qualified { type_mark, .. } => match self.type_mark(scope, type_mark) {
                Ok(st) => CandSet::Types(vec![Interp::new(self.base(st), Via::Other)]),
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
            K::Conditional(_) | K::Unaffected | K::Error => self.poison_cands(),
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
                    Some(ty) => CandSet::Types(vec![Interp {
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
                        CandSet::Types(vec![Interp::new(ty, Via::Literal)])
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
    fn not_a_unit(&mut self, unit: rsim_syntax::ast::Ident) -> CandSet {
        self.error(
            rsim_diag::Diagnostic::error(
                crate::diag::BAD_LITERAL,
                format!("`{}` is not a physical unit", unit.text),
            )
            .with_primary(unit.span, "not a unit"),
        );
        self.poison_cands()
    }

    /// Candidates of a name that denotes only overloadables (character literals).
    fn overload_cands(&mut self, scope: ScopeId, key: rsim_syntax::Symbol) -> CandSet {
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
        let ocs: Vec<CandSet> = operands.iter().map(|o| self.cands(scope, o)).collect();
        if ocs.iter().any(|c| self.is_poison_set(c)) {
            return self.poison_cands();
        }
        let Visible::Overloads(ds) = self.lookup(scope, operator_key(sym)) else {
            return CandSet::Types(Vec::new());
        };
        let mut out = Vec::new();
        for d in ds {
            let e = self.entity(d);
            let Some(s) = self.decls[e].kind.subprogram() else {
                continue;
            };
            let Some(ret) = s.ret else { continue };
            if s.params.len() != operands.len() {
                continue;
            }
            let formals: Vec<TypeId> = s.params.iter().map(|p| self.base(p.subtype)).collect();
            let mut convs = 0;
            let mut ok = true;
            for (c, &f) in ocs.iter().zip(&formals) {
                match self.best_cost(c, Expected::Type(f)) {
                    Some(k) => convs += k,
                    None => {
                        ok = false;
                        break;
                    }
                }
            }
            if ok {
                out.push(Interp {
                    convs,
                    ..Interp::new(self.base(ret), Via::Call(d))
                });
            }
        }
        CandSet::Types(out)
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
                if i.convertible
                    && ((Some(i.ty) == ui && matches!(target, TypeKind::Integer))
                        || (Some(i.ty) == ur && matches!(target, TypeKind::Floating)))
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
        }
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
            CandSet::Class(_) => Vec::new(),
        };
        (self.memo, self.name_memo) = saved;
        out
    }
}
