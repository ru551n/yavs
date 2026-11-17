//! What the elaborated design can run: the statements of its processes and of the subprograms
//! they call, walked from the processes (cycle mode's rules on shared variables and forces, and
//! the forces that keep ports from collapsing, `collapse::forced`).

use std::collections::BTreeSet;

use yavs_sem::hir::{ExprKind, StmtKind};
use yavs_sem::{Db, ExprId, StmtId};

use crate::ProcInst;

/// The force and release statements the processes `procs` can run, through the subprograms they
/// call, and whether every call was followed; when one was not, every analysed force.
pub fn forces(db: &Db, procs: &[ProcInst]) -> (BTreeSet<StmtId>, bool) {
    let all: BTreeSet<StmtId> = db.facts().forces.iter().copied().collect();
    if all.is_empty() {
        return (all, true);
    }
    let (mut seen, mut stmts, mut exprs, mut ok) = (BTreeSet::new(), Vec::new(), Vec::new(), true);
    for p in procs {
        exprs.extend(p.actual.as_ref().map(|a| a.expr));
        deep(db, &[p.stmt], &mut stmts);
        if let StmtKind::Process { scope, .. } = &db.stmts[p.stmt].kind {
            decl_exprs(db, *scope, &mut exprs);
        }
    }
    let mut reached = BTreeSet::new();
    loop {
        for x in std::mem::take(&mut exprs) {
            let mut later = Vec::new();
            db.walk_expr(x, &mut |y| {
                if let ExprKind::Call { callee, .. } = &db.exprs[y].kind {
                    ok &= follow(db, *callee, &mut seen, &mut stmts, &mut later);
                }
            });
            exprs.append(&mut later);
        }
        if let Some(s) = stmts.pop() {
            if all.contains(&s) {
                reached.insert(s);
            }
            db.stmt_exprs(s, &mut |x, _| exprs.push(x));
            if let StmtKind::ProcCall(c) | StmtKind::ConcProcCall { call: c, .. } =
                &db.stmts[s].kind
            {
                ok &= follow(db, c.callee, &mut seen, &mut stmts, &mut exprs);
            }
        } else if exprs.is_empty() {
            break;
        }
    }
    (if ok { reached } else { all }, ok)
}

/// Add the statements of subprogram `f` to `stmts` once (a body of the design); false for one the
/// scan cannot follow (no body here). `ieee` and `std` bodies use no shared variable of the design.
pub fn follow(
    db: &Db,
    f: yavs_sem::DeclId,
    seen: &mut BTreeSet<yavs_sem::DeclId>,
    stmts: &mut Vec<StmtId>,
    exprs: &mut Vec<ExprId>,
) -> bool {
    let Some(sp) = db.decls[f].kind.subprogram() else {
        return true;
    };
    if ["ieee.", "std."]
        .iter()
        .any(|p| sp.mangled.as_str().starts_with(p))
    {
        return true;
    }
    match &sp.body {
        yavs_sem::decl::Body::Hir { scope, stmts: body } => {
            if seen.insert(f) {
                deep(db, body, stmts);
                exprs.extend(sp.params.iter().filter_map(|p| p.default));
                decl_exprs(db, *scope, exprs);
            }
            true
        }
        yavs_sem::decl::Body::Implicit(_) => true,
        _ => false,
    }
}

/// What the declarations of region `scope` evaluate: initial values, file names, constraints,
/// the targets of object aliases.
pub fn decl_exprs(db: &Db, scope: yavs_sem::ScopeId, out: &mut Vec<ExprId>) {
    for &x in &db.scopes[scope].order {
        match &db.decls[x].kind {
            yavs_sem::decl::DeclKind::Object(o) => {
                out.extend(o.default);
                if let Some((a, b)) = o.file_open {
                    out.extend(a);
                    out.push(b);
                }
                constraint_exprs(&db.subtypes[o.subtype].constraint, out);
            }
            yavs_sem::decl::DeclKind::Alias(yavs_sem::decl::Alias::Object { target, .. }) => {
                out.push(*target)
            }
            _ => {}
        }
    }
}

pub fn deep(db: &Db, body: &[StmtId], out: &mut Vec<StmtId>) {
    for &s in body {
        out.push(s);
        deep(db, &db.stmt_children(s), out);
    }
}

/// The bound expressions of a constraint.
pub fn constraint_exprs(c: &yavs_sem::types::Constraint, out: &mut Vec<ExprId>) {
    use yavs_sem::types::{Constraint as C, RangeC};
    let mut range = |r: &RangeC| {
        if let RangeC::Explicit { left, right, .. } = r {
            out.extend([*left, *right]);
        }
    };
    match c {
        C::Range(r) => range(r),
        C::Array { dims, elem } => {
            dims.iter().flatten().for_each(&mut range);
            constraint_exprs(elem, out);
        }
        C::Record { elems } => elems.iter().for_each(|(_, c)| constraint_exprs(c, out)),
        _ => {}
    }
}
