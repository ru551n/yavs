//! In-port collapsing (`docs/design/performance.md` R4, plan rule 3): the formal of an `in`
//! port whose actual is whole kernel leaves of the same type, with no conversion, becomes an
//! alias of those leaves. The formal object keeps its name, scope and shape (waves, VHPI,
//! `'path_name`) and takes the actual's leaf range; its own leaves, their port edges and the
//! copy per delta are gone. Run after `bind_late`, when every force target is known; leaves
//! are renumbered in their old order, so elaboration order stays deterministic.
//!
//! The effective value of an `in` formal is the effective value of its actual (LRM 14.7.3.3),
//! so its value, events and activity are the actual's either way. Not collapsed: forced or
//! released formals (a force of an `in` port acts on the port only, LRM 10.5.2.2), formals
//! with drivers, a guard, a user resolution function or a composite resolved as a whole, and
//! actuals that are parts of a leaf (`d => v(i)`).

use std::collections::HashSet;

use yavs_sem::Db;
use yavs_sem::decl::{DeclKind, ObjRole};
use yavs_sem::hir::{ObjRoot, StmtKind};

use crate::ProcInst;
use crate::hier::{LeafId, LeafRes, NetGraph, ObjId, ObjKind};

/// Collapse the `in` ports of `nets` that qualify; returns how many formals were collapsed.
pub(crate) fn in_ports(
    db: &Db,
    nets: &mut NetGraph,
    sigs: &std::collections::HashMap<(crate::ScopeId, yavs_sem::DeclId), ObjId>,
    procs: &mut [ProcInst],
) -> usize {
    let Some(forced) = forced(db, sigs) else {
        return 0;
    };
    let n = nets.leaves.len();
    // Per leaf: the port edge it is the formal of (`usize::MAX` when several or none).
    let mut edge = vec![usize::MAX; n];
    let mut excluded = vec![false; n];
    for (i, e) in nets.ports.iter().enumerate() {
        let f = e.formal.0.index();
        match edge[f] {
            usize::MAX if !excluded[f] => edge[f] = i,
            _ => (edge[f], excluded[f]) = (usize::MAX, true),
        }
    }
    for p in &nets.conv_ports {
        p.formal.iter().for_each(|f| excluded[f.0.index()] = true);
    }
    for d in &nets.drivers {
        excluded[d.leaf.index()] = true;
    }
    for w in &nets.wholes {
        w.instances
            .iter()
            .flatten()
            .for_each(|p| excluded[p.0.index()] = true);
    }
    // The leaf each collapsed formal leaf aliases (before following chains).
    let mut alias: Vec<Option<LeafId>> = vec![None; n];
    let mut count = 0;
    for (o, obj) in nets.objs.iter().enumerate() {
        let in_port = matches!(
            obj.kind,
            ObjKind::Port(yavs_kernel::PortMode::In)
                | ObjKind::ComponentPort(yavs_kernel::PortMode::In)
        );
        let (a, b) = (obj.leaves.0.index(), obj.leaves.1.index());
        if !in_port || a == b || forced.contains(&ObjId(o as u32)) {
            continue;
        }
        let mut targets = Vec::with_capacity(b - a);
        for l in a..b {
            let leaf = &nets.leaves[l];
            let ok = !excluded[l]
                && edge[l] != usize::MAX
                && leaf.guard.is_none()
                && leaf.implicit.is_none()
                && matches!(leaf.resolution, None | Some(LeafRes::StdLogic));
            let e = ok.then(|| &nets.ports[edge[l]]);
            let Some(e) = e.filter(|e| {
                let al = &nets.leaves[e.actual.0.index()];
                e.mode == yavs_kernel::PortMode::In
                    && e.to_formal.is_none()
                    && e.to_actual.is_none()
                    && e.formal.1.offset == 0
                    && e.formal.1.len == leaf.ty.len
                    && e.actual.1.offset == 0
                    && e.actual.1.len == al.ty.len
                    && al.ty == leaf.ty
            }) else {
                break;
            };
            targets.push(e.actual.0);
        }
        // Whole leaves of one consecutive range, so the object keeps a leaf range.
        let consecutive = targets.len() == b - a
            && targets
                .iter()
                .enumerate()
                .all(|(k, t)| t.0 == targets[0].0 + k as u32);
        if consecutive {
            for (l, t) in (a..b).zip(targets) {
                alias[l] = Some(t);
            }
            count += 1;
        }
    }
    if count == 0 {
        return 0;
    }
    // Follow chains (a component port collapsed into a signal, the entity port into it).
    let find = |mut l: LeafId| {
        while let Some(t) = alias[l.index()] {
            l = t;
        }
        l
    };
    // New ids: the kept leaves in their old order.
    let mut pos = Vec::with_capacity(n + 1);
    let mut k = 0u32;
    for a in &alias {
        pos.push(k);
        k += u32::from(a.is_none());
    }
    pos.push(k);
    let m = |l: LeafId| LeafId(pos[find(l).index()]);
    for obj in &mut nets.objs {
        let (a, b) = obj.leaves;
        let first = if a == b { LeafId(pos[a.index()]) } else { m(a) };
        obj.leaves = (first, LeafId(first.0 + (b.0 - a.0)));
    }
    let mut i = 0;
    nets.leaves.retain(|_| {
        i += 1;
        alias[i - 1].is_none()
    });
    for leaf in &mut nets.leaves {
        if let Some((_, src, _)) = &mut leaf.implicit {
            *src = m(*src);
        }
        for (l, _) in &mut leaf.implicit_more {
            *l = m(*l);
        }
    }
    nets.ports.retain(|e| alias[e.formal.0.index()].is_none());
    for e in &mut nets.ports {
        (e.formal.0, e.actual.0) = (m(e.formal.0), m(e.actual.0));
    }
    for p in &mut nets.conv_ports {
        for (l, _) in p.formal.iter_mut().chain(&mut p.actual) {
            *l = m(*l);
        }
    }
    for d in &mut nets.drivers {
        d.leaf = m(d.leaf);
    }
    for w in &mut nets.wholes {
        for (l, _) in w.instances.iter_mut().flatten() {
            *l = m(*l);
        }
    }
    for p in procs {
        for (_, l, _) in p.lsps.iter_mut().flatten() {
            *l = m(*l);
        }
        if let Some((_, l, _)) = &mut p.trigger {
            *l = m(*l);
        }
    }
    count
}

/// The signal objects a VHDL force or release names, directly, through an alias or through
/// an external name; `None` when one names a signal parameter (any port may be its actual).
fn forced(
    db: &Db,
    sigs: &std::collections::HashMap<(crate::ScopeId, yavs_sem::DeclId), ObjId>,
) -> Option<HashSet<ObjId>> {
    let mut decls = HashSet::new();
    let mut exts = HashSet::new();
    for (_, st) in db.stmts.iter() {
        let t = match &st.kind {
            StmtKind::Force { target, .. } | StmtKind::Release { target, .. } => target,
            _ => continue,
        };
        for l in &t.lsps {
            match l.root {
                ObjRoot::Decl(d) => {
                    if matches!(&db.decls[d].kind, DeclKind::Object(o) if o.role == ObjRole::Param)
                    {
                        return None;
                    }
                    decls.insert(d);
                }
                ObjRoot::External(x) => {
                    exts.insert(x);
                }
                ObjRoot::ImplicitSignal(_) => {}
            }
        }
    }
    let mut out = HashSet::new();
    if decls.is_empty() && exts.is_empty() {
        return Some(out);
    }
    for (&(_, d), &o) in sigs {
        if decls.contains(&d) || db.external_alias(d).is_some_and(|x| exts.contains(&x)) {
            out.insert(o);
        }
    }
    Some(out)
}
