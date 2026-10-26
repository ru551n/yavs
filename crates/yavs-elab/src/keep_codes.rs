//! Signals that need no byte codes (#734). The kernel mirrors every element of a logic signal in
//! one byte (`yavs_kernel::Builder::no_codes` says when it need not), which makes a read of one
//! element one load but costs a refresh on every change. Only reads of a single element use them;
//! the analysis facts (`yavs_sem::Facts::elem_reads`) say which signal objects have such reads.
//! The kernel adds the width condition of #519: above 64 elements only with narrow sources.

use std::collections::HashSet;

use yavs_sem::Db;

use crate::hier::{NetGraph, ObjKind, Shape};

/// Per leaf: whether it keeps no codes. A leaf keeps them when it is one element (a scalar), when
/// an object that holds it has a read of one element, and when that object has records in it
/// (a record read element by element, `SigGather`). A scalar port read whole counts as an
/// element read of its actual when it is collapsed into part of it (`Shape` leaves shared), and
/// a port-map actual is not a read itself. All false when an element is read through something
/// the elaborator cannot trace to signal objects (an external name, a callee it cannot see into).
pub(crate) fn leaves(db: &Db, nets: &NetGraph) -> Vec<bool> {
    let f = db.facts();
    if f.elem_reads_all {
        return vec![false; nets.leaves.len()];
    }
    let reads: HashSet<_> = f.elem_reads.iter().copied().collect();
    let mut keep: Vec<bool> = nets.leaves.iter().map(|l| l.ty.len <= 1).collect();
    let mut seen = vec![false; nets.leaves.len()];
    for o in &nets.objs {
        let r = o.leaves.0.index()..o.leaves.1.index();
        seen[r.clone()].fill(true);
        if o.kind == ObjKind::Implicit || reads.contains(&o.decl) || !elements_only(&o.shape) {
            keep[r].fill(true);
        }
    }
    keep.iter().zip(seen).map(|(&k, s)| s && !k).collect()
}

/// A scalar or an array of those (of any depth): no record, so no leaf is read element by element
/// for a whole value.
fn elements_only(s: &Shape) -> bool {
    match s {
        Shape::Scalar { .. } => true,
        Shape::Array { elem, .. } => elements_only(elem),
        Shape::Record { .. } => false,
    }
}
