//! Signals whose event history nothing asks for (#519). The kernel keeps each element's last
//! event of a signal (`yavs_kernel::Builder::lean_events` says when it need not); only
//! `'last_event` and `'last_value` of something other than whole signals ask for that, and the
//! analysis facts (`yavs_sem::Facts`) say where they occur.

use std::collections::{HashMap, HashSet};

use yavs_sem::{Db, DeclId};

use crate::hier::{NetGraph, ObjId, ObjKind, ScopeId, Shape};

/// Per leaf: whether it is wide (more than 64 elements) and no `'last_event` or `'last_value`
/// in the design can ask for the history of a part of it. All false when one is applied to
/// something the elaborator cannot trace to signal objects (a signal parameter, an external
/// name): any signal may be its actual. A narrow leaf is lean only through `bare`
/// (`docs/decisions.md` "Narrow signals nothing asks the history of").
pub(crate) fn leaves(db: &Db, nets: &NetGraph) -> Vec<bool> {
    let f = db.facts();
    if f.history_all {
        return vec![false; nets.leaves.len()];
    }
    let mut lean: Vec<bool> = nets.leaves.iter().map(|l| l.ty.len > 64).collect();
    let parts: HashSet<_> = f.history_parts.iter().copied().collect();
    let wholes: HashSet<_> = f.history_wholes.iter().copied().collect();
    for o in &nets.objs {
        // A whole object is whole leaves, unless a port was collapsed into part of its actual,
        // and the interpreter asks one leaf part by part when the object is an array of
        // composites (a part per element).
        if parts.contains(&o.decl)
            || (wholes.contains(&o.decl) && (!o.parts.is_empty() || !one_part_per_leaf(&o.shape)))
        {
            lean[o.leaves.0.index()..o.leaves.1.index()].fill(false);
        }
    }
    lean
}

/// Per leaf, of any width (#1111): whether nothing in the design can ask any event history or
/// `'last_value` of it. `history` is what the lowered code says (`Func::sig_history_uses`, as
/// declarations; `None`: one could not be named), so a reader the lowerer adds is seen by
/// construction; the kernel takes its own readers back (`SimControl::new`). Nothing in a design
/// with a force qualifies, nor an implicit signal. The kernel then keeps no event records and no
/// `'last_value` for such a bus (`yavs_kernel::Builder::no_history`); a record pack is bare when all
/// its leaves are (#1117, `build.rs`). Such a leaf is lean too, whatever `history_all` says.
pub(crate) fn bare(
    db: &Db,
    nets: &NetGraph,
    sigs: &HashMap<(ScopeId, DeclId), ObjId>,
    history: Option<&[DeclId]>,
) -> Vec<bool> {
    let Some(history) = history.filter(|_| db.facts().forces.is_empty()) else {
        return vec![false; nets.leaves.len()];
    };
    let mut bare = vec![true; nets.leaves.len()];
    let asked: HashSet<_> = history.iter().copied().collect();
    // By the objects the lowered code's names denote in a region: an external alias is the object
    // it names, not a declaration of its own.
    for ((_, d), &o) in sigs {
        if asked.contains(d) {
            let (a, z) = nets.obj(o).leaves;
            bare[a.index()..z.index()].fill(false);
        }
    }
    for o in &nets.objs {
        if o.kind == ObjKind::Implicit {
            bare[o.leaves.0.index()..o.leaves.1.index()].fill(false);
        }
    }
    bare
}

/// A scalar, an array of scalars, or a record of those: a whole object is one range of each leaf.
fn one_part_per_leaf(s: &Shape) -> bool {
    match s {
        Shape::Scalar { .. } => true,
        Shape::Array { elem, .. } => matches!(**elem, Shape::Scalar { .. }),
        Shape::Record { fields, .. } => fields.iter().all(|(_, f)| one_part_per_leaf(f)),
    }
}
