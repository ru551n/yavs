//! Signals whose event history nothing asks for (#519). The kernel keeps each element's last
//! event of a signal of more than 64 elements (`yavs_kernel::Builder::lean_events` says when
//! it need not); only `'last_event` and `'last_value` of something other than whole signals
//! ask for that, and the analysis facts (`yavs_sem::Facts`) say where they occur.

use std::collections::HashSet;

use yavs_sem::Db;

use crate::hier::{NetGraph, Shape};

/// Per leaf: whether it is wide (more than 64 elements) and no `'last_event` or `'last_value`
/// in the design can ask for the history of a part of it. All false when one is applied to
/// something the elaborator cannot trace to signal objects (a signal parameter, an external
/// name): any signal may be its actual.
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
            || (wholes.contains(&o.decl) && (o.part.is_some() || !one_part_per_leaf(&o.shape)))
        {
            lean[o.leaves.0.index()..o.leaves.1.index()].fill(false);
        }
    }
    lean
}

/// A scalar, an array of scalars, or a record of those: a whole object is one range of each leaf.
fn one_part_per_leaf(s: &Shape) -> bool {
    match s {
        Shape::Scalar { .. } => true,
        Shape::Array { elem, .. } => matches!(**elem, Shape::Scalar { .. }),
        Shape::Record { fields, .. } => fields.iter().all(|(_, f)| one_part_per_leaf(f)),
    }
}
