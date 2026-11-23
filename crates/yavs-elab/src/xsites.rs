//! The metavalue literals `--init-x` gives a value (`Elaborated::x_sites`): the value of a signal
//! or variable assignment in a process or concurrent assignment of an architecture below the
//! top-level unit that is a `std_ulogic` literal `'U'`, `'X'` or `'W'`, or `'-'` in a default
//! branch (the `others` of a `case` or a selected assignment, the last `else` of an `if` or a
//! conditional assignment), alone, qualified, or as the one `others` choice of an aggregate,
//! nested too (`(others => (others => 'X'))`). Nothing else: literals in conditions,
//! comparisons, calls and subprograms, string literals, initial values and the top-level unit
//! stay as written, so a `'-'` mask given to `std_match` keeps its meaning.

use std::collections::HashSet;

use yavs_sem::hir::{Choice, ChoiceSet, ExprKind, Rhs, StmtKind, Waveform};
use yavs_sem::stat::ConstValue;
use yavs_sem::{Db, ExprId, StmtId};

/// The sites of the statements `stmts` of an architecture, into `out`, and the statements that
/// hold one into `stmts_out`.
pub(crate) fn collect(
    db: &Db,
    stmts: &[StmtId],
    out: &mut HashSet<ExprId>,
    stmts_out: &mut HashSet<StmtId>,
) {
    for &s in stmts {
        let before = out.len();
        match &db.stmts[s].kind {
            StmtKind::Process { body, .. } => seq(db, body, false, out),
            StmtKind::ConcSignalAssign { rhs, .. } => waves(db, rhs, false, out),
            StmtKind::Block { body, .. } | StmtKind::ForGenerate { body, .. } => {
                collect(db, body, out, stmts_out)
            }
            StmtKind::IfGenerate { alts } => alts
                .iter()
                .for_each(|a| collect(db, &a.body, out, stmts_out)),
            StmtKind::CaseGenerate { alts, .. } => alts
                .iter()
                .for_each(|(_, a)| collect(db, &a.body, out, stmts_out)),
            _ => {}
        }
        if out.len() != before {
            stmts_out.insert(s);
        }
    }
}

fn others(c: &ChoiceSet) -> bool {
    c.choices.iter().any(|c| matches!(c, Choice::Others))
}

/// Sequential statements; `dflt`: their innermost branch is a default one (a `then` branch or a
/// `case` choice inside an `else` is not).
fn seq(db: &Db, stmts: &[StmtId], dflt: bool, out: &mut HashSet<ExprId>) {
    for &s in stmts {
        match &db.stmts[s].kind {
            StmtKind::SignalAssign { rhs, .. } => waves(db, rhs, dflt, out),
            StmtKind::VarAssign { rhs, .. } => rhs_of(rhs, dflt, &mut |e, d| value(db, *e, d, out)),
            StmtKind::If { branches, else_ } => {
                branches.iter().for_each(|(_, b)| seq(db, b, false, out));
                if let Some(b) = else_ {
                    seq(db, b, true, out);
                }
            }
            StmtKind::Case { alts, .. } => {
                for (c, b) in alts.iter() {
                    seq(db, b, others(c), out);
                }
            }
            StmtKind::Loop { body, .. } | StmtKind::SeqBlock { body, .. } => {
                seq(db, body, dflt, out)
            }
            _ => {}
        }
    }
}

/// Each value of `rhs` with whether it is in a default branch.
fn rhs_of<T>(rhs: &Rhs<T>, dflt: bool, f: &mut dyn FnMut(&T, bool)) {
    match rhs {
        Rhs::Simple(v) => f(v, dflt),
        Rhs::Conditional(alts) => {
            for (v, c) in alts.iter() {
                f(v, c.is_none());
            }
        }
        Rhs::Selected { alts, .. } => {
            for (v, c) in alts.iter() {
                f(v, others(c));
            }
        }
    }
}

fn waves(db: &Db, rhs: &Rhs<Waveform>, dflt: bool, out: &mut HashSet<ExprId>) {
    rhs_of(rhs, dflt, &mut |w, d| {
        if let Waveform::Elements(es) = w {
            for v in es.iter().filter_map(|e| e.value) {
                value(db, v, d, out);
            }
        }
    });
}

/// Mark `e` (or the operand of a qualified expression) when it is a site.
fn value(db: &Db, e: ExprId, dflt: bool, out: &mut HashSet<ExprId>) {
    match &db.exprs[e].kind {
        ExprKind::Qualified { operand, .. } => value(db, *operand, dflt, out),
        _ if is_site(db, e, dflt) => {
            out.insert(e);
        }
        _ => {}
    }
}

/// A metavalue literal of `std_ulogic`, or an aggregate of one `others` choice of one.
fn is_site(db: &Db, e: ExprId, dflt: bool) -> bool {
    let x = &db.exprs[e];
    match &x.kind {
        ExprKind::Const(c) => {
            db.is_std_ulogic(db.base(x.subtype))
                && match db.consts[*c] {
                    ConstValue::Enum(p) => matches!(p, 0 | 1 | 5) || (p == 8 && dflt),
                    _ => false,
                }
        }
        ExprKind::Aggregate { assocs, .. } => {
            matches!(&assocs[..], [a] if matches!(&a.choices[..], [Choice::Others])
                && !a.slice_piece
                && is_site(db, a.value, dflt))
        }
        ExprKind::Qualified { operand, .. } => is_site(db, *operand, dflt),
        _ => false,
    }
}
