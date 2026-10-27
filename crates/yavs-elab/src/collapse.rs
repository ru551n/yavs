//! Port collapsing (`docs/design/performance.md` R4, plan rule 3): the formal of an `in`,
//! `out` or `buffer` port whose actual is whole kernel leaves of the same type, or (a formal
//! of one leaf) part of one leaf (`d => v(i)`), with no conversion, becomes an alias of them.
//! The formal object keeps its name, scope and shape (waves, `'path_name`) and takes the
//! actual's leaf range, with `SigObj::part` for a part of a leaf; its own leaves, their port
//! edges and the copy per delta are gone. Run after `bind_late`, when every force target is
//! known; leaves are renumbered in their old order, so elaboration order stays deterministic.
//!
//! The effective value of an `in` formal is the effective value of its actual (LRM 14.7.3.3),
//! so its value, events and activity are the actual's either way. Not collapsed: forced or
//! released formals (a force of an `in` port acts on the port only, LRM 10.5.2.2), formals
//! with drivers, a guard, a user resolution function or a composite resolved as a whole.
//!
//! An `out` or `buffer` formal that is the only source of its actual's elements (#528): the
//! effective value of both is their driving value (LRM 14.7.3.3), and the actual's driving
//! value is the formal's (one source, unresolved or resolved by `resolved`, the identity on
//! one value), so the formal's drivers can drive the actual directly. The actual takes the
//! formal's initial value: the drivers start from it, and it is the actual's value after
//! initialisation anyway (LRM 14.7.5.2). Not collapsed: an actual with another source, a
//! guard, a conversion, an effective value from above (the formal of an `in` or `inout` port),
//! in a composite resolved as a whole, or forced or released; a formal resolved where the
//! actual is not; `inout` ports.

use std::collections::HashSet;

use yavs_sem::Db;
use yavs_sem::decl::{DeclKind, ObjRole};
use yavs_sem::hir::{ObjRoot, StmtKind};

use yavs_kernel::{PortMode, Range, Value};

use crate::ProcInst;
use crate::hier::{LeafId, LeafRes, NetGraph, ObjId, ObjKind};

/// Collapse the ports of `nets` that qualify; returns how many formals were collapsed.
pub(crate) fn ports(
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
    // Per leaf: its source ranges (drivers, ports other than `in`, conversions), and whether
    // its effective value comes from above (the formal of an `in` or `inout` port).
    let mut sources: Vec<Vec<Range>> = vec![Vec::new(); n];
    let mut fed = vec![false; n];
    for e in &nets.ports {
        if e.mode != PortMode::In {
            sources[e.actual.0.index()].push(e.actual.1);
        }
        if matches!(e.mode, PortMode::In | PortMode::Inout) {
            fed[e.formal.0.index()] = true;
        }
    }
    for p in &nets.conv_ports {
        p.formal.iter().for_each(|f| excluded[f.0.index()] = true);
        p.formal.iter().for_each(|f| fed[f.0.index()] = true);
        p.actual.iter().for_each(|a| sources[a.0.index()].push(a.1));
    }
    let mut driven = vec![false; n];
    for d in &nets.drivers {
        driven[d.leaf.index()] = true;
        sources[d.leaf.index()].push(d.range);
    }
    for w in &nets.wholes {
        w.instances
            .iter()
            .flatten()
            .for_each(|p| excluded[p.0.index()] = true);
    }
    // The actual of an `out` formal: no other source of these elements, its effective value
    // its driving value, no guard, not forced, not in a composite resolved as a whole.
    let sole = |e: &crate::hier::PortEdge| {
        let (a, r) = (e.actual.0.index(), e.actual.1);
        let al = &nets.leaves[a];
        let overlaps = sources[a].iter().filter(|s| s.overlaps(r)).count();
        overlaps == 1 && !fed[a] && !excluded[a] && al.guard.is_none() && !forced.contains(&al.obj)
    };
    // The leaf and kernel offset each collapsed formal leaf aliases (before following chains):
    // element `k` of the formal is element `offset + k` of the target (logic leaves too:
    // `NetGraph::select` mirrors both the same way).
    let mut alias: Vec<Option<(LeafId, u32)>> = vec![None; n];
    let mut count = 0;
    // The collapsed `out` and `buffer` formal leaves (their actuals take their initial values).
    let mut outs = Vec::new();
    for (o, obj) in nets.objs.iter().enumerate() {
        let mode = match obj.kind {
            ObjKind::Port(m) | ObjKind::ComponentPort(m) => m,
            _ => continue,
        };
        let out = matches!(mode, PortMode::Out | PortMode::Buffer);
        let (a, b) = (obj.leaves.0.index(), obj.leaves.1.index());
        if !(out || mode == PortMode::In) || a == b || forced.contains(&ObjId(o as u32)) {
            continue;
        }
        let mut targets = Vec::with_capacity(b - a);
        for l in a..b {
            let leaf = &nets.leaves[l];
            let ok = !excluded[l]
                && edge[l] != usize::MAX
                && leaf.guard.is_none()
                && leaf.implicit.is_none()
                && (out || !driven[l])
                && matches!(leaf.resolution, None | Some(LeafRes::StdLogic));
            let e = ok.then(|| &nets.ports[edge[l]]);
            let Some(e) = e.filter(|e| {
                let al = &nets.leaves[e.actual.0.index()];
                e.mode == mode
                    && e.to_formal.is_none()
                    && e.to_actual.is_none()
                    && e.formal.1.offset == 0
                    && e.formal.1.len == leaf.ty.len
                    && e.actual.1.len == leaf.ty.len
                    && al.ty.elem == leaf.ty.elem
                    // Several leaves keep one leaf range: whole leaves of one object.
                    && (b - a == 1 || (e.actual.1.offset == 0 && al.ty == leaf.ty))
                    // An `out` formal's drivers drive the actual: it resolves them the same
                    // way (an unresolved formal has at most one).
                    && (!out
                        || sole(e)
                            && match (&leaf.resolution, &al.resolution) {
                                (None, None | Some(LeafRes::StdLogic)) => true,
                                (Some(LeafRes::StdLogic), r) => {
                                    matches!(r, Some(LeafRes::StdLogic))
                                }
                                _ => false,
                            })
            }) else {
                break;
            };
            targets.push(e.actual);
        }
        let one_range = targets.len() == b - a
            && targets.iter().enumerate().all(|(k, t)| {
                t.0.0 == targets[0].0.0 + k as u32
                    && nets.leaves[t.0.index()].obj == nets.leaves[targets[0].0.index()].obj
            });
        if one_range {
            for (l, t) in (a..b).zip(targets) {
                alias[l] = Some((t.0, t.1.offset));
            }
            if out {
                outs.extend(a..b);
            }
            count += 1;
        }
    }
    if count == 0 {
        return 0;
    }
    // An actual takes its `out` formal's initial value, the deepest formal's of a chain
    // first.
    let depth = |mut l: usize| {
        let mut d = 0;
        while let Some((t, _)) = alias[l] {
            (l, d) = (t.index(), d + 1);
        }
        d
    };
    outs.sort_by_key(|&l| std::cmp::Reverse(depth(l)));
    for l in outs {
        let (t, off) = alias[l].expect("collapsed");
        let src = nets.leaves[l].init.clone();
        let len = nets.leaves[l].ty.len;
        put_init(&mut nets.leaves[t.index()].init, off, &src, len);
    }
    // Follow chains (a component port collapsed into a signal, the entity port into it).
    let find = |mut l: LeafId| {
        let mut off = 0;
        while let Some((t, o)) = alias[l.index()] {
            (l, off) = (t, off + o);
        }
        (l, off)
    };
    // New ids: the kept leaves in their old order.
    let mut pos = Vec::with_capacity(n + 1);
    let mut k = 0u32;
    for a in &alias {
        pos.push(k);
        k += u32::from(a.is_none());
    }
    pos.push(k);
    let m = |l: LeafId| {
        let (t, off) = find(l);
        (LeafId(pos[t.index()]), off)
    };
    let mr = |p: &mut (LeafId, Range)| {
        let (l, off) = m(p.0);
        *p = (l, Range::new(p.1.offset + off, p.1.len));
    };
    for obj in &mut nets.objs {
        let (a, b) = obj.leaves;
        if a == b {
            obj.leaves = (LeafId(pos[a.index()]), LeafId(pos[a.index()]));
            continue;
        }
        let (first, off) = m(a);
        obj.leaves = (first, LeafId(first.0 + (b.0 - a.0)));
        let own = nets.leaves[a.index()].ty.len;
        if alias[a.index()].is_some() && (off != 0 || own != nets.leaves[find(a).0.index()].ty.len)
        {
            obj.part = Some(Range::new(off, own));
        }
    }
    let mut i = 0;
    nets.leaves.retain(|_| {
        i += 1;
        alias[i - 1].is_none()
    });
    for leaf in &mut nets.leaves {
        if let Some((_, src, r)) = &mut leaf.implicit {
            let mut p = (*src, *r);
            mr(&mut p);
            (*src, *r) = p;
        }
        leaf.implicit_more.iter_mut().for_each(mr);
    }
    nets.ports.retain(|e| alias[e.formal.0.index()].is_none());
    for e in &mut nets.ports {
        mr(&mut e.formal);
        mr(&mut e.actual);
    }
    for p in &mut nets.conv_ports {
        p.formal.iter_mut().chain(&mut p.actual).for_each(mr);
    }
    for d in &mut nets.drivers {
        let mut p = (d.leaf, d.range);
        mr(&mut p);
        (d.leaf, d.range) = p;
    }
    for w in &mut nets.wholes {
        w.instances.iter_mut().flatten().for_each(mr);
    }
    for p in procs {
        for (_, l, r) in p.lsps.iter_mut().flatten() {
            let mut q = (*l, *r);
            mr(&mut q);
            (*l, *r) = q;
        }
        if let Some(c) = &mut p.copy {
            c.iter_mut().for_each(mr);
        }
        let remap = |x: &mut crate::ProcSrc| {
            if let crate::ProcSrc::Sig(l, r) = x {
                let mut q = (*l, *r);
                mr(&mut q);
                (*l, *r) = q;
            }
        };
        if let Some(g) = &mut p.gate {
            remap(&mut g.a);
            remap(&mut g.b);
        }
        if let Some(x) = &mut p.mux {
            match &mut x.cond {
                crate::ProcCond::Mask(c, _) => mr(c),
                crate::ProcCond::Eq { a, b, .. } => {
                    remap(a);
                    remap(b);
                }
            }
            x.srcs.iter_mut().flatten().for_each(remap);
        }
        if let Some(g) = &mut p.prog {
            for n in &mut g.nodes {
                if let crate::ProcOp::Mask(l, r, _) | crate::ProcOp::Bytes(l, r, ..) = &mut n.op {
                    let mut q = (*l, *r);
                    mr(&mut q);
                    (*l, *r) = q;
                }
            }
            g.opnds_mut().for_each(|o| {
                if let crate::ProcOpnd::Src(x) = o {
                    remap(x);
                }
            });
        }
        if let Some(v) = &mut p.guard {
            v.iter_mut().for_each(mr);
        }
        for r in p.idle.iter_mut().flatten() {
            let mut q = (r.leaf, r.range);
            mr(&mut q);
            (r.leaf, r.range) = q;
            for l in r.guard.iter_mut().flatten().flatten() {
                let (n, off) = m(l.leaf);
                (l.leaf, l.elem) = (n, l.elem + off);
            }
        }
        if let Some(t) = &mut p.toggle {
            let (l, off) = m(t.leaf);
            (t.leaf, t.elem) = (l, t.elem + off);
        }
        if let Some(t) = &mut p.trigger {
            let (l, off) = m(t.leaf);
            (t.leaf, t.elem) = (l, t.elem + off);
            if let Some((l, e, _)) = &mut t.reset {
                let (n, off) = m(*l);
                (*l, *e) = (n, *e + off);
            }
        }
    }
    count
}

/// The elements `r` of `v`, a leaf of `len` elements.
pub(crate) fn slice_init(v: &Value, len: u32, r: Range) -> Value {
    match v {
        Value::Bytes(b) if len > 0 => {
            let w = b.len() / len as usize;
            Value::Bytes(b[r.offset as usize * w..(r.offset + r.len) as usize * w].to_vec())
        }
        Value::Logic(l) => Value::Logic(yavs_kernel::LogicVec::from_fn(r.len as usize, |i| {
            l.get(r.offset as usize + i)
        })),
        v => v.clone(),
    }
}

/// The `len` elements of `src` into `dst` from element `off`.
pub(crate) fn put_init(dst: &mut Value, off: u32, src: &Value, len: u32) {
    match (dst, src) {
        (Value::Bytes(d), Value::Bytes(s)) if len > 0 => {
            let o = off as usize * (s.len() / len as usize);
            d[o..o + s.len()].copy_from_slice(s);
        }
        (Value::Logic(d), Value::Logic(s)) => {
            for i in 0..len as usize {
                d.set(off as usize + i, s.get(i));
            }
        }
        _ => {}
    }
}

/// The signal objects a VHDL force or release names, directly, through an alias or through
/// an external name; `None` when one names a signal parameter (any port may be its actual).
pub(crate) fn forced(
    db: &Db,
    sigs: &std::collections::HashMap<(crate::ScopeId, yavs_sem::DeclId), ObjId>,
) -> Option<HashSet<ObjId>> {
    let mut decls = HashSet::new();
    let mut exts = HashSet::new();
    for &id in &db.facts().forces {
        let t = match &db.stmts[id].kind {
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
