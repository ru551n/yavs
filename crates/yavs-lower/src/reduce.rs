//! The reduction idiom (#531): a user function that folds a `std_ulogic` vector with one gate
//! (`v := '0'; for i in d'range loop v := v or d(i); end loop; return v;`) is called as the
//! word-parallel native of the unary `std_logic_1164` operator instead of being interpreted.

use yavs_ir::ieee::{self, Gate, Op};
use yavs_sem::decl::{Body, DeclKind, Designator, ObjClass};
use yavs_sem::hir::{Actual, AttrPrefix, ExprKind, LoopScheme, Rhs, StmtKind};
use yavs_sem::stat::ConstValue;
use yavs_sem::types::{Constraint, RangeC, TypeKind};
use yavs_sem::{Db, DeclId, ExprId, SubtypeId};

/// The `std_ulogic` positions of `'0'` and `'1'`, the identities of `or`/`xor` and `and`.
const ZERO: u32 = 2;
const ONE: u32 = 3;

fn is_std_ulogic(db: &Db, st: SubtypeId) -> bool {
    let td = &db.types[db.base(st)];
    matches!(&td.kind, TypeKind::Enum { lits } if lits.len() == 9)
        && td.name.key.as_str() == "std_ulogic"
}

/// The object `e` names, if it is a plain name.
fn obj(db: &Db, e: ExprId) -> Option<DeclId> {
    match db.exprs[e].kind {
        ExprKind::Obj(d) => Some(d),
        _ => None,
    }
}

/// The gate `callee` folds its one unconstrained `std_ulogic` array parameter with, when its body
/// is exactly the idiom above with the gate's identity as the start value.
pub(crate) fn reduction(db: &Db, callee: DeclId) -> Option<Gate> {
    let sp = db.decls[callee].kind.subprogram()?;
    let Body::Hir { scope, stmts } = &sp.body else {
        return None;
    };
    let [p] = &*sp.params else { return None };
    let ts = &db.types[db.base(p.subtype)].kind;
    let TypeKind::Array { indexes, elem } = ts else {
        return None;
    };
    if indexes.len() != 1
        || !is_std_ulogic(db, *elem)
        || !matches!(
            db.subtypes[p.subtype].constraint,
            Constraint::None | Constraint::Array { dims: None, .. }
        )
        || p.class != ObjClass::Constant
        || !sp.ret.is_some_and(|r| is_std_ulogic(db, r))
    {
        return None;
    }
    // `v := <identity>;` before the loop, or `variable v : std_ulogic := <identity>;` in the
    // body's declarations.
    let (start, lp, ret) = match &**stmts {
        [init, lp, ret] => {
            let StmtKind::VarAssign {
                target,
                rhs: Rhs::Simple(start),
            } = &db.stmts[*init].kind
            else {
                return None;
            };
            (Some((obj(db, target.expr)?, *start)), lp, ret)
        }
        [lp, ret] => (None, lp, ret),
        _ => return None,
    };
    // `for i in d'range loop v := v <gate> d(i); end loop;`
    let StmtKind::Loop {
        scheme: LoopScheme::For { param, range },
        body,
        ..
    } = &db.stmts[*lp].kind
    else {
        return None;
    };
    let RangeC::Attr {
        prefix: AttrPrefix::Expr(pe),
        reverse: false,
        dim: 0 | 1,
    } = range
    else {
        return None;
    };
    // The parameter as the body names it (a spec and its body declare one each).
    let d = obj(db, *pe)?;
    let named = matches!(db.decls[d].name, Designator::Ident(i) if i.key == p.name.key);
    if db.decls[d].scope != *scope || !named {
        return None;
    }
    let [step] = &**body else { return None };
    let StmtKind::VarAssign {
        target,
        rhs: Rhs::Simple(e),
    } = &db.stmts[*step].kind
    else {
        return None;
    };
    let v = obj(db, target.expr)?;
    let DeclKind::Object(vo) = &db.decls[v].kind else {
        return None;
    };
    if db.decls[v].scope != *scope
        || vo.class != ObjClass::Variable
        || !is_std_ulogic(db, vo.subtype)
    {
        return None;
    }
    let start = match start {
        Some((sv, e)) if sv == v => e,
        None => vo.default?,
        _ => return None,
    };
    let start = match db.value(start) {
        Some(ConstValue::Enum(pos)) => *pos,
        _ => return None,
    };
    let ExprKind::Call {
        callee: g,
        args,
        method_of: None,
        ..
    } = &db.exprs[*e].kind
    else {
        return None;
    };
    let gate = match ieee::native_symbol(db.decls[*g].kind.subprogram()?.mangled.as_str())
        .as_deref()
        .and_then(ieee::symbol_key)
        .and_then(ieee::parse)
    {
        Some(Op::Gate(gate @ (Gate::And | Gate::Or | Gate::Xor))) => gate,
        _ => return None,
    };
    let [a, b] = &**args else { return None };
    let (Actual::Expr(a), Actual::Expr(b)) = (&a.actual, &b.actual) else {
        return None;
    };
    // One operand is `v`, the other `d(i)`.
    let elem_of_d = |x: ExprId| match &db.exprs[x].kind {
        ExprKind::Index { prefix, indices } => {
            matches!(&**indices, [i] if obj(db, *i) == Some(*param)) && obj(db, *prefix) == Some(d)
        }
        _ => false,
    };
    if !(obj(db, *a) == Some(v) && elem_of_d(*b) || obj(db, *b) == Some(v) && elem_of_d(*a)) {
        return None;
    }
    // `return v;`
    let StmtKind::Return(Some(r)) = &db.stmts[*ret].kind else {
        return None;
    };
    if obj(db, *r) != Some(v) {
        return None;
    }
    let identity = if gate == Gate::And { ONE } else { ZERO };
    (start == identity).then_some(gate)
}

/// The native symbol of the unary operator of `gate` on a `std_ulogic_vector`.
pub(crate) fn symbol(gate: Gate) -> String {
    let name = match gate {
        Gate::And => "and",
        Gate::Or => "or",
        _ => "xor",
    };
    format!("yavs.ieee.std_logic_1164.\"{name}\"(std_ulogic_vector)std_ulogic")
}
