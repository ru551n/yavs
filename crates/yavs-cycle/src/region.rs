//! Which processes join the region, and the region's boundary. Selection is automatic: the
//! subset checker (`check`) decides which processes fit cycle mode at all, the set rules and the
//! element-level dependence graph (`dep`) decide which of those can run together, and the
//! lowering decides what is implemented yet. Every other process stays in the event kernel,
//! listed with its reason, and the two kinds of reason stay apart: outside the subset (a code)
//! or not implemented yet.

use std::collections::{BTreeMap, BTreeSet};

use yavs_elab::{Case, Elaborated, LeafId, LeafRes, ScopeKind};
use yavs_sem::hir::{ExprKind, ObjRoot, Sensitivity, StmtKind};
use yavs_sem::{Db, ExprId, StmtId};

use crate::check::{self, Fit, Form, Reason};
use crate::dep::{self, VAR};
use crate::lower::{self, Body, Clock, Kind};

/// Why a process stays in the event kernel.
pub(crate) enum Why {
    /// Outside the subset.
    Subset(Reason),
    /// A process of the testbench kind.
    Behavioural(String),
    /// In the subset; cycle mode does not implement something it needs yet.
    NotYet(String),
}

/// The region: members (in schedule order: clocked first, then combinational in dependency
/// order) and the boundary.
pub(crate) struct Selection {
    pub members: Vec<(usize, Body)>,
    pub out: Vec<(usize, Why)>,
    pub clock: Option<Clock>,
    /// Loop groups of the processes in the subset (before what is implemented is known).
    pub groups: Vec<dep::Group>,
    /// Processes in the subset, and asserts and reports they hold.
    pub fits: usize,
    pub dropped: u32,
    /// Leaves members drive, and of those the ones something outside reads (exported).
    pub driven: BTreeSet<LeafId>,
    pub outputs: BTreeSet<LeafId>,
    /// Leaves something outside may wait on (or that it is not known it does not).
    pub waited: BTreeSet<LeafId>,
    /// Leaves members read that no member drives (read from the kernel at each wake); of
    /// those, the ones a combinational member reads wake the region.
    pub inputs: BTreeSet<LeafId>,
    pub wakes: BTreeSet<LeafId>,
    /// Leaves a clocked member writes and a clocked member reads: written to a next-state
    /// block, committed after all clocked bodies ran.
    pub next: BTreeSet<LeafId>,
}

/// Kernel parts of leaves.
type Parts = Vec<(LeafId, yavs_kernel::Range)>;

/// What fits, why the rest does not, and the loop groups of what fits.
type Subset = (BTreeMap<usize, Fit>, Vec<(usize, Why)>, Vec<dep::Group>);

/// The subset verdict of every process: what fits, and why the others do not.
pub(crate) fn subset(db: &Db, e: &Elaborated) -> Subset {
    let mut fits: BTreeMap<usize, Fit> = BTreeMap::new();
    let mut out: Vec<(usize, Why)> = Vec::new();
    for pi in 0..e.procs.len() {
        match check::check(db, e, pi) {
            check::Verdict::Fit(f) => {
                fits.insert(pi, f);
            }
            check::Verdict::Behavioural(w) => out.push((pi, Why::Behavioural(w))),
            check::Verdict::Out(r) => out.push((pi, Why::Subset(r))),
        }
    }
    // Every element a fitting process drives must have it as its only source.
    let mut drivers: BTreeMap<LeafId, Vec<(usize, u32, u32)>> = BTreeMap::new();
    for d in &e.nets.drivers {
        drivers
            .entry(d.leaf)
            .or_default()
            .push((d.proc.index(), d.range.offset, d.range.end()));
    }
    let other = overlapping(e);
    let span = |pi: usize| db.stmts[e.procs[pi].stmt].span;
    let shared: Vec<(usize, Reason)> = (fits.keys().copied())
        .filter_map(|pi| {
            for &d in &e.procs[pi].drivers {
                let l = e.nets.drivers[d].leaf;
                let leaf = e.nets.leaf(l);
                let text = if let Some(&q) = other.get(&d) {
                    format!(
                        "{} is also driven by {}",
                        e.leaf_name(l),
                        e.proc_label(&e.procs[q])
                    )
                } else if leaf.guard.is_some() {
                    format!("{} is a guarded signal", e.leaf_name(l))
                } else if matches!(leaf.resolution, Some(LeafRes::Func(_) | LeafRes::Whole(_))) {
                    format!("{} has a resolution function of its own", e.leaf_name(l))
                } else {
                    continue;
                };
                return Some((
                    pi,
                    Reason {
                        code: crate::codes::SHARED,
                        span: span(pi),
                        text,
                    },
                ));
            }
            None
        })
        .collect();
    for (pi, r) in shared {
        fits.remove(&pi);
        out.push((pi, Why::Subset(r)));
    }
    loop {
        // Clocks come from outside the region. Made by combinational logic (a gate, a copy),
        // the maker stays out and the kernel computes the clock; made by a register (a
        // divider), the process using it stays out. An asynchronous reset made inside is fine:
        // its branch is combinational logic while it holds (its edges are in the graph).
        let mut made: Vec<(usize, Reason)> = Vec::new();
        for (pi, f) in &fits {
            if let Some((l, what)) = f.clock.map(|c| (c.0, "clock")) {
                for &(q, _, _) in drivers.get(&l).into_iter().flatten() {
                    let Some(m) = fits.get(&q) else { continue };
                    let (who, text) = if m.form == Form::Comb {
                        (
                            q,
                            format!(
                                "it drives {}, the {what} of {}: clocks come from outside the region",
                                e.leaf_name(l),
                                e.proc_label(&e.procs[*pi])
                            ),
                        )
                    } else {
                        (
                            *pi,
                            format!(
                                "its {what} {} is made by a register of the region ({}); clocks come \
                             from outside the region",
                                e.leaf_name(l),
                                e.proc_label(&e.procs[q])
                            ),
                        )
                    };
                    if !made.iter().any(|m| m.0 == who) {
                        made.push((
                            who,
                            Reason {
                                code: crate::codes::CLOCK_MADE,
                                span: span(who),
                                text,
                            },
                        ));
                    }
                }
            }
        }
        // A loop element by element in the combinational logic.
        let a = analyse(db, e, &fits);
        for (ps, path) in &a.cycles {
            for &q in ps {
                if !made.iter().any(|m| m.0 == q) {
                    made.push((
                        q,
                        Reason {
                            code: crate::codes::LOOP,
                            span: span(q),
                            text: format!("it is in a combinational loop: {path}"),
                        },
                    ));
                }
            }
        }
        if made.is_empty() {
            return (fits, out, a.groups);
        }
        for (pi, r) in made {
            fits.remove(&pi);
            out.push((pi, Why::Subset(r)));
        }
    }
}

/// For every driver (by its index in `e.nets.drivers`) with an element another process also
/// drives: one such process. Per leaf in O(n log n): the drivers sorted by first element; one
/// starting at or before driver `i` overlaps it when it ends after `i` starts (the two farthest
/// ends of distinct processes so far give one of another process), one starting after it when it
/// starts before `i` ends (the next driver of another process in the order).
fn overlapping(e: &Elaborated) -> std::collections::HashMap<usize, usize> {
    let mut by_leaf: BTreeMap<LeafId, Vec<(u32, u32, usize, usize)>> = BTreeMap::new();
    for (i, d) in e.nets.drivers.iter().enumerate() {
        if d.range.len > 0 {
            (by_leaf.entry(d.leaf).or_default()).push((
                d.range.offset,
                d.range.end(),
                d.proc.index(),
                i,
            ));
        }
    }
    let mut other = std::collections::HashMap::new();
    for ds in by_leaf.values_mut() {
        overlaps(ds, &mut other);
    }
    other
}

/// The drivers `ds` (first, end, process, id) of one leaf: into `other`, for the id of each with
/// an element another process drives, one such process.
fn overlaps(
    ds: &mut [(u32, u32, usize, usize)],
    other: &mut std::collections::HashMap<usize, usize>,
) {
    if ds.len() < 2 {
        return;
    }
    ds.sort_unstable();
    let n = ds.len();
    // The first later index whose process differs.
    let mut next = vec![n; n];
    for k in (0..n - 1).rev() {
        next[k] = if ds[k + 1].2 != ds[k].2 {
            k + 1
        } else {
            next[k + 1]
        };
    }
    // The farthest end so far, and the farthest of another process: (end, process).
    let mut best: [Option<(u32, usize)>; 2] = [None, None];
    for (i, &(a, z, p, d)) in ds.iter().enumerate() {
        let before = best.iter().flatten().find(|b| b.1 != p && b.0 > a);
        let after = match ds.get(i + 1) {
            Some(&(b, _, q, _)) if b < z && q != p => Some(q),
            Some(&(b, ..)) if b < z => (ds.get(next[i + 1])).filter(|o| o.0 < z).map(|o| o.2),
            _ => None,
        };
        if let Some(q) = before.map(|b| b.1).or(after) {
            other.insert(d, q);
        }
        // Keep the two farthest ends of distinct processes.
        match best {
            [Some(b), _] if b.1 == p => best[0] = Some((b.0.max(z), p)),
            [_, Some(c)] if c.1 == p => best[1] = Some((c.0.max(z), p)),
            _ => {
                if best[1].is_none_or(|c| c.0 < z) {
                    best[1] = Some((z, p));
                }
            }
        }
        if best[1].map(|c| c.0) > best[0].map(|b| b.0) {
            best.swap(0, 1);
        }
    }
}

/// The lowering sees the shape, clock and reset the checker proved, and reads nothing the
/// checker did not count (the order of the members rests on its reads): `None`, else why not.
fn differs(e: &Elaborated, f: &Fit, b: &Body) -> Option<String> {
    let shape = match (f.form, b.kind, f.clock, b.clock) {
        (Form::Comb, Kind::Comb, None, None) => true,
        (Form::Clocked, Kind::Clocked, Some(c), Some(d)) => {
            (c.0, c.1, c.2) == (d.leaf, d.elem, d.rising)
        }
        _ => false,
    };
    let reset = match b.reset {
        Some((l, _, _)) => f.reset == [l],
        None => f.reset.is_empty(),
    };
    if !shape || !reset {
        return Some(
            "the lowering takes it for another shape, clock or reset than the subset checker"
                .into(),
        );
    }
    let (l, x) = (b.rparts.iter()).find_map(|&(l, lo, n)| {
        f.reads
            .first_missing(u64::from(l.0), lo, lo + n)
            .map(|x| (l, x))
    })?;
    Some(format!(
        "the lowering reads element {x} of {}, which the subset checker did not count",
        e.leaf_name(l)
    ))
}

/// The dependence analysis of the fitting processes.
fn analyse(db: &Db, e: &Elaborated, fits: &BTreeMap<usize, Fit>) -> dep::Analysis {
    let _ = db;
    let n = e.procs.len();
    let mut procs: Vec<(usize, Vec<dep::Out>)> = (fits.iter())
        .filter(|(_, f)| !f.outs.is_empty() || f.form == Form::Comb)
        .map(|(pi, f)| (*pi, f.outs.clone()))
        .collect();
    // Port edges the kernel updates are copies in the same delta: an `in` formal depends on its
    // actual, an `out` actual on its formal (a conversion: on all of the other side). They are
    // pseudo processes after the real ones, so a loop through a port is seen.
    let side = |ps: &Parts| -> std::rc::Rc<dep::ESet> {
        let mut s = dep::ESet::default();
        for (l, r) in ps {
            s.add(u64::from(l.0), r.offset, r.end());
        }
        std::rc::Rc::new(s)
    };
    // Each edge: what it drives, what from, whether through a conversion.
    let mut edges: Vec<(Parts, Parts, bool)> = Vec::new();
    for pe in &e.nets.ports {
        let conv = pe.to_formal.is_some() || pe.to_actual.is_some();
        match pe.mode {
            yavs_kernel::PortMode::In => edges.push((vec![pe.formal], vec![pe.actual], conv)),
            yavs_kernel::PortMode::Out | yavs_kernel::PortMode::Buffer => {
                edges.push((vec![pe.actual], vec![pe.formal], conv))
            }
            // ponytail: an `inout` edge goes both ways, a loop of its own; such a port's
            // drivers are kernel sources, so its processes are not implemented yet anyway.
            yavs_kernel::PortMode::Inout => {}
        }
    }
    for cp in &e.nets.conv_ports {
        match cp.mode {
            yavs_kernel::PortMode::In => edges.push((cp.formal.clone(), cp.actual.clone(), true)),
            yavs_kernel::PortMode::Out | yavs_kernel::PortMode::Buffer => {
                edges.push((cp.actual.clone(), cp.formal.clone(), true))
            }
            yavs_kernel::PortMode::Inout => {}
        }
    }
    for (i, (dst, src, conv)) in edges.into_iter().enumerate() {
        let outs = if conv || dst.len() != 1 || src.len() != 1 || dst[0].1.len != src[0].1.len {
            let s = side(&src);
            (dst.iter())
                .map(|(l, r)| dep::Out {
                    leaf: u64::from(l.0),
                    lo: r.offset,
                    hi: r.end(),
                    dep: dep::Dep::set(s.clone()),
                })
                .collect()
        } else {
            vec![dep::Out {
                leaf: u64::from(dst[0].0.0),
                lo: dst[0].1.offset,
                hi: dst[0].1.end(),
                dep: dep::Dep {
                    ident: Some((u64::from(src[0].0.0), src[0].1.offset)),
                    set: std::rc::Rc::new(dep::ESet::default()),
                },
            }]
        };
        procs.push((n + i, outs));
    }
    let atom = |k: u64, a: u32, z: u32| -> String {
        if k & VAR != 0 {
            return "a variable".into();
        }
        let l = LeafId(k as u32);
        let n = e.leaf_name(l);
        if e.nets.leaf(l).ty.len == z - a {
            n
        } else if z == a + 1 {
            format!("{n} (element {a})")
        } else {
            format!("{n} (elements {a} to {})", z - 1)
        }
    };
    let pname = |pi: usize| {
        if pi < n {
            e.proc_label(&e.procs[pi])
        } else {
            "a port".into()
        }
    };
    let mut a = dep::analyse(&procs, &atom, &pname);
    // Only real processes leave the region or form groups.
    for c in &mut a.cycles {
        c.0.retain(|&p| p < n);
    }
    for g in &mut a.groups {
        g.procs.retain(|&p| p < n);
    }
    a.groups.retain(|g| !g.procs.is_empty());
    a.order.retain(|&p| p < n);
    a
}

pub(crate) fn select(db: &Db, e: &Elaborated) -> Selection {
    let (fits, mut out, groups) = subset(db, e);
    let mut drivers: BTreeMap<LeafId, Vec<(usize, u32, u32)>> = BTreeMap::new();
    for d in &e.nets.drivers {
        drivers
            .entry(d.leaf)
            .or_default()
            .push((d.proc.index(), 0, 0));
    }
    let nfits = fits.len();
    let dropped = fits.values().map(|f| f.dropped).sum();
    let forced = !db.facts().forces.is_empty();
    let mut taken: BTreeMap<usize, (Fit, Body)> = BTreeMap::new();
    for (pi, f) in fits {
        if forced {
            out.push((
                pi,
                Why::NotYet(
                    "the design uses `force` or `release`, which cycle mode \
                does not combine with a region yet"
                        .into(),
                ),
            ));
            continue;
        }
        match lower::lower(db, e, pi) {
            // The lowering must see the shape the checker proved, clock included.
            Ok(b) if differs(e, &f, &b).is_some() => {
                out.push((pi, Why::NotYet(differs(e, &f, &b).expect("checked"))))
            }
            Ok(b) => {
                taken.insert(pi, (f, b));
            }
            Err(w) => out.push((pi, Why::NotYet(w))),
        }
    }
    // Leave out what the set as a whole does not allow yet, until nothing changes.
    loop {
        let mut gone: Vec<(usize, String)> = Vec::new();
        // Leaves with a source no process owns: a constant source, or a port edge or converted
        // port the kernel updates (a member would read its own driving value from the image,
        // not the resolved one).
        let mut consts: BTreeSet<LeafId> = e.nets.consts.iter().map(|c| c.0).collect();
        for pe in &e.nets.ports {
            consts.extend([pe.formal.0, pe.actual.0]);
        }
        for cp in &e.nets.conv_ports {
            consts.extend(cp.formal.iter().chain(&cp.actual).map(|x| x.0));
        }
        // Per leaf, one driving process outside the region.
        let mut outside: BTreeMap<LeafId, usize> = BTreeMap::new();
        for d in &e.nets.drivers {
            if !taken.contains_key(&d.proc.index()) {
                outside.entry(d.leaf).or_insert(d.proc.index());
            }
        }
        let mut clock: Option<Clock> = None;
        for (&pi, (_, b)) in &taken {
            let p = &e.procs[pi];
            let src = (p.drivers.iter())
                .map(|&d| e.nets.drivers[d].leaf)
                .find(|l| consts.contains(l));
            if let Some(l) = src {
                gone.push((
                    pi,
                    format!(
                        "{} has a port or other source cycle mode does not model \
                    yet",
                        e.leaf_name(l)
                    ),
                ));
                continue;
            }
            // Other elements of a driven leaf driven outside the region: the region would not
            // read them. Members driving different elements of one leaf are fine.
            let split = (p.drivers.iter())
                .map(|&d| e.nets.drivers[d].leaf)
                .find_map(|l| outside.get(&l).map(|&q| (l, q)));
            if let Some((l, q)) = split {
                gone.push((
                    pi,
                    format!(
                        "other elements of {} are driven by {}, outside the region; cycle mode \
                         does not split a signal between the region and the kernel yet",
                        e.leaf_name(l),
                        e.proc_label(&e.procs[q])
                    ),
                ));
                continue;
            }
            // A reset made inside the region: the lowering applies a reset branch only at the
            // region's wakes, before it settles, so it would act one wake late.
            if let Some((r, ..)) = b.reset
                && let Some(q) = (drivers.get(&r).into_iter().flatten())
                    .map(|o| o.0)
                    .find(|q| taken.contains_key(q))
            {
                gone.push((
                    pi,
                    format!(
                        "its asynchronous reset is made inside the region, by {}; cycle mode \
                         applies a reset only when an event of it from outside wakes the region \
                         (not implemented yet)",
                        e.proc_label(&e.procs[q])
                    ),
                ));
                continue;
            }
            if let Some(c) = b.clock {
                if clock.is_some_and(|k| k != c) {
                    gone.push((
                        pi,
                        "it runs on another clock or edge than the region (several \
                        clocks are not supported yet)"
                            .into(),
                    ));
                } else {
                    clock = Some(c);
                }
            }
        }
        // Loop groups need several passes or a split, which the schedule does not do yet.
        let leaving: BTreeSet<usize> = gone.iter().map(|g| g.0).collect();
        let fits: BTreeMap<usize, Fit> = (taken.iter())
            .filter(|(pi, _)| !leaving.contains(pi))
            .map(|(pi, (f, _))| (*pi, f.clone()))
            .collect();
        let a = analyse(db, e, &fits);
        for g in &a.groups {
            for &pi in &g.procs {
                gone.push((pi, format!(
                    "it is in a loop group of {} process{} (reading, through {} or itself, what \
                     it drives) that needs {} passes, or a split into {} pieces; cycle mode \
                     runs one pass and does not split processes yet",
                    g.procs.len(),
                    if g.procs.len() == 1 { "" } else { "es" },
                    if g.procs.len() == 1 { "nothing else" } else { "the others" },
                    g.k,
                    g.slices
                )));
            }
        }
        if gone.is_empty() {
            let mut members = Vec::new();
            let order: Vec<usize> = a
                .order
                .iter()
                .copied()
                .filter(|pi| taken.contains_key(pi))
                .collect();
            let mut rest: BTreeMap<usize, Body> =
                taken.into_iter().map(|(pi, (_, b))| (pi, b)).collect();
            let clocked: Vec<usize> = (rest.iter())
                .filter(|(_, b)| b.kind == Kind::Clocked)
                .map(|(pi, _)| *pi)
                .collect();
            for pi in clocked {
                members.push((pi, rest.remove(&pi).expect("listed")));
            }
            for pi in order {
                if let Some(b) = rest.remove(&pi) {
                    members.push((pi, b));
                }
            }
            // Members the dependence order does not list (no outputs): any place.
            members.extend(rest);
            out.sort_by_key(|o| o.0);
            let mut sel = boundary(db, e, members, out, clock);
            sel.groups = groups;
            sel.fits = nfits;
            sel.dropped = dropped;
            return sel;
        }
        for (pi, w) in gone {
            if taken.remove(&pi).is_some() {
                out.push((pi, Why::NotYet(w)));
            }
        }
    }
}

fn boundary(
    db: &Db,
    e: &Elaborated,
    members: Vec<(usize, Body)>,
    out: Vec<(usize, Why)>,
    clock: Option<Clock>,
) -> Selection {
    let is_member: BTreeSet<usize> = members.iter().map(|m| m.0).collect();
    let mut driven = BTreeSet::new();
    for d in &e.nets.drivers {
        if is_member.contains(&d.proc.index()) {
            driven.insert(d.leaf);
        }
    }
    let mut reads = BTreeSet::new();
    let mut comb_reads = BTreeSet::new();
    let mut clocked_reads = BTreeSet::new();
    let mut clocked_writes = BTreeSet::new();
    for (_, b) in &members {
        reads.extend(b.reads.iter().copied());
        match b.kind {
            Kind::Comb => comb_reads.extend(b.reads.iter().copied()),
            Kind::Clocked => {
                clocked_reads.extend(b.reads.iter().copied());
                clocked_writes.extend(b.writes.iter().copied());
            }
        }
    }
    let inputs: BTreeSet<LeafId> = reads.difference(&driven).copied().collect();
    let wakes = comb_reads.difference(&driven).copied().collect();
    let next = clocked_writes
        .intersection(&clocked_reads)
        .copied()
        .collect();
    let (outputs, waited) = if outside_reads_all(db, e, &is_member) {
        (driven.clone(), driven.clone())
    } else {
        let (seen, waited) = outside_reads(db, e, &is_member);
        (driven.intersection(&seen).copied().collect(), waited)
    };
    Selection {
        members,
        out,
        clock,
        groups: Vec::new(),
        fits: 0,
        dropped: 0,
        driven,
        outputs,
        waited,
        inputs,
        wakes,
        next,
    }
}

/// Something may read any signal: an external name anywhere in the design.
fn outside_reads_all(db: &Db, _e: &Elaborated, _members: &BTreeSet<usize>) -> bool {
    !db.externals.is_empty()
}

/// The leaves anything that is not a member may read, and of those the ones something outside
/// may wait on. Read: the code of the other processes (by the objects their names denote, whole),
/// port edges and implicit signals the kernel updates, guards, and the signals of packages. A
/// call that may read signals it is not given (a subprogram declared in a design unit rather than
/// a package) counts as reading every signal of the design. Waited on: a sensitivity list (`all`:
/// every read), the signals of a wait statement (its `until` condition when it names none), every
/// read of a statement that is not a process, and everything above that is not process code.
fn outside_reads(
    db: &Db,
    e: &Elaborated,
    members: &BTreeSet<usize>,
) -> (BTreeSet<LeafId>, BTreeSet<LeafId>) {
    let mut seen = BTreeSet::new();
    let mut waited = BTreeSet::new();
    let all = || -> BTreeSet<LeafId> { (0..e.nets.leaves.len() as u32).map(LeafId).collect() };
    for (pi, p) in e.procs.iter().enumerate() {
        if members.contains(&pi) {
            continue;
        }
        // Names read, and names waited on.
        let mut lsps = Vec::new();
        let mut waits = Vec::new();
        let mut calls_local = false;
        if let Some(pa) = &p.actual {
            db.reads(pa.expr, &mut waits);
            calls_local |= exprs_call_local(db, &[pa.expr]);
        } else {
            match &db.stmts[p.stmt].kind {
                StmtKind::Process {
                    sensitivity, body, ..
                } => {
                    db.reads_deep(body, &mut lsps);
                    match sensitivity {
                        Sensitivity::List(l) => waits.extend(l.iter().cloned()),
                        Sensitivity::All => waits.extend(lsps.iter().cloned()),
                        Sensitivity::None => {}
                    }
                    let mut stmts = Vec::new();
                    deep(db, body, &mut stmts);
                    for s in stmts {
                        if let StmtKind::Wait { on, until, .. } = &db.stmts[s].kind {
                            waits.extend(on.iter().cloned());
                            if on.is_empty()
                                && let Some(c) = until
                            {
                                db.reads(*c, &mut waits);
                            }
                        }
                        calls_local |= stmt_calls_local(db, s);
                    }
                }
                _ => {
                    db.stmt_reads(p.stmt, &mut waits);
                    calls_local |= stmt_calls_local(db, p.stmt);
                }
            }
        }
        if calls_local {
            return (all(), all());
        }
        for (l, wait) in lsps
            .iter()
            .map(|l| (l, false))
            .chain(waits.iter().map(|l| (l, true)))
        {
            match l.root {
                ObjRoot::Decl(d) => match e.signal(p.scope, d) {
                    Some(o) => {
                        let (a, z) = e.nets.obj(o).leaves;
                        let leaves = (a.0..z.0).map(LeafId);
                        if wait {
                            waited.extend(leaves.clone());
                        }
                        seen.extend(leaves);
                    }
                    None => return (all(), all()),
                },
                ObjRoot::External(_) => return (all(), all()),
                ObjRoot::ImplicitSignal(_) => {}
            }
        }
        if let Some(g) = &p.guard {
            seen.extend(g.iter().map(|x| x.0));
            waited.extend(g.iter().map(|x| x.0));
        }
    }
    let mut other = BTreeSet::new();
    for pe in &e.nets.ports {
        other.insert(pe.formal.0);
        other.insert(pe.actual.0);
    }
    for cp in &e.nets.conv_ports {
        other.extend(cp.formal.iter().chain(&cp.actual).map(|x| x.0));
    }
    for leaf in &e.nets.leaves {
        if let Some((_, src, _)) = leaf.implicit {
            other.insert(src);
        }
        other.extend(leaf.implicit_more.iter().map(|x| x.0));
    }
    for o in &e.nets.objs {
        if matches!(e.hier.scope(o.scope).kind, ScopeKind::Package { .. }) {
            other.extend((o.leaves.0.0..o.leaves.1.0).map(LeafId));
        }
    }
    seen.extend(other.iter().copied());
    waited.extend(other);
    (seen, waited)
}

fn deep(db: &Db, body: &[StmtId], out: &mut Vec<StmtId>) {
    for &s in body {
        out.push(s);
        deep(db, &db.stmt_children(s), out);
    }
}

/// Statement `s` (not its children) calls a subprogram that may read signals it is not given.
fn stmt_calls_local(db: &Db, s: StmtId) -> bool {
    if let StmtKind::ProcCall(c) | StmtKind::ConcProcCall { call: c, .. } = &db.stmts[s].kind
        && local_subprogram(db, c.callee)
    {
        return true;
    }
    let mut xs = Vec::new();
    db.stmt_exprs(s, &mut |x, _| xs.push(x));
    exprs_call_local(db, &xs)
}

fn exprs_call_local(db: &Db, xs: &[ExprId]) -> bool {
    let mut stack: Vec<ExprId> = xs.to_vec();
    while let Some(x) = stack.pop() {
        match &db.exprs[x].kind {
            ExprKind::Call {
                callee,
                args,
                method_of,
                ..
            } => {
                if local_subprogram(db, *callee) {
                    return true;
                }
                stack.extend(method_of);
                for a in args.iter() {
                    if !push_actual(&a.actual, &mut stack) {
                        return true;
                    }
                }
            }
            ExprKind::Index { prefix, indices } => {
                stack.push(*prefix);
                stack.extend(indices.iter());
            }
            ExprKind::Slice { prefix, range } => {
                stack.push(*prefix);
                if let yavs_sem::types::RangeC::Explicit { left, right, .. } = range {
                    stack.extend([*left, *right]);
                }
            }
            ExprKind::Field { prefix, .. } | ExprKind::Deref { prefix } => stack.push(*prefix),
            ExprKind::PrivateVar { object, .. } => stack.push(*object),
            ExprKind::Attr { prefix, args, .. } => {
                if let yavs_sem::hir::AttrPrefix::Expr(p) = prefix {
                    stack.push(*p);
                }
                stack.extend(args.iter());
            }
            ExprKind::Aggregate { assocs, .. } => {
                for a in assocs.iter() {
                    stack.push(a.value);
                    for c in a.choices.iter() {
                        match c {
                            yavs_sem::hir::Choice::Expr(x) => stack.push(*x),
                            yavs_sem::hir::Choice::Range(yavs_sem::types::RangeC::Explicit {
                                left,
                                right,
                                ..
                            }) => stack.extend([*left, *right]),
                            _ => {}
                        }
                    }
                }
            }
            ExprKind::Qualified { operand, .. }
            | ExprKind::Conv { operand, .. }
            | ExprKind::ImplicitConv { operand } => stack.push(*operand),
            ExprKind::Conditional { alts } => {
                for (v, c) in alts.iter() {
                    stack.push(*v);
                    stack.extend(c);
                }
            }
            // An allocator's initial value may call anything: counted as a call.
            ExprKind::New { .. } => return true,
            ExprKind::Const(_)
            | ExprKind::Obj(_)
            | ExprKind::External(_)
            | ExprKind::AttrValue { .. }
            | ExprKind::Null
            | ExprKind::Open
            | ExprKind::Error => {}
        }
    }
    false
}

/// The expressions of an actual; false for one this scan does not follow.
fn push_actual(a: &yavs_sem::hir::Actual, stack: &mut Vec<ExprId>) -> bool {
    use yavs_sem::hir::Actual;
    match a {
        Actual::Expr(x) => stack.push(*x),
        Actual::Converted { actual, .. } => stack.push(*actual),
        Actual::Individual(parts) => stack.extend(parts.iter().map(|p| p.1)),
        Actual::Default | Actual::Open => {}
        Actual::Subtype(_) | Actual::Subprogram(_) | Actual::Package(_) => return false,
    }
    true
}

/// A subprogram declared outside a package (in an architecture, block, process or another
/// subprogram of one): its body may read the signals of its region directly.
fn local_subprogram(db: &Db, d: yavs_sem::DeclId) -> bool {
    use yavs_sem::scope::ScopeKind as S;
    let mut s = Some(db.decls[d].scope);
    while let Some(x) = s {
        match db.scopes[x].kind {
            S::Package | S::PackageBody => return false,
            S::Architecture | S::Entity | S::Process | S::Block | S::Generate => return true,
            _ => s = db.scopes[x].parent,
        }
    }
    false
}

/// The process is directly in the top-level design unit (its architecture, or a block or
/// generate of it): the testbench, exempt under `--cycle-strict`.
pub(crate) fn in_top_unit(e: &Elaborated, pi: usize) -> bool {
    let mut s = e.hier.scope(e.procs[pi].scope).parent;
    while let Some(x) = s {
        let sc = e.hier.scope(x);
        if let ScopeKind::Instance { .. } = sc.kind {
            return sc.parent.is_none();
        }
        s = sc.parent;
    }
    false
}

/// What texts call process `pi`, with its file and line.
pub(crate) fn where_(db: &Db, e: &Elaborated, pi: usize) -> String {
    let p = &e.procs[pi];
    let name = e.hier.proc_label(p, Case::Lrm);
    if p.what.is_some() {
        return name;
    }
    let span = db.stmts[p.stmt].span;
    let f = db.sources.get(span.file);
    let file = std::path::Path::new(&f.path)
        .file_name()
        .map_or(f.path.clone(), |n| n.to_string_lossy().into_owned());
    format!("{name} ({file}:{})", f.line_col(span.start).0)
}

#[cfg(test)]
mod tests {
    /// `overlaps` against the pairwise test on many small random sets of drivers.
    #[test]
    fn overlaps_finds_what_the_pairwise_test_finds() {
        let mut seed = 0x9e37_79b9_7f4a_7c15u64;
        let mut rnd = |m: u64| {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed % m
        };
        for _ in 0..20_000 {
            let n = 1 + rnd(7) as usize;
            let ds: Vec<(u32, u32, usize, usize)> = (0..n)
                .map(|i| {
                    let a = rnd(12) as u32;
                    (a, a + 1 + rnd(5) as u32, rnd(4) as usize, i)
                })
                .collect();
            let mut got = std::collections::HashMap::new();
            super::overlaps(&mut ds.clone(), &mut got);
            for &(a, z, p, d) in &ds {
                let any = ds.iter().any(|&(b, y, q, _)| q != p && b < z && a < y);
                assert_eq!(got.contains_key(&d), any, "{ds:?} driver {d}");
                if let Some(&q) = got.get(&d) {
                    let (b, y, ..) = *ds
                        .iter()
                        .find(|o| o.2 == q && o.0 < z && a < o.1)
                        .expect("overlaps");
                    assert!(q != p && b < z && a < y);
                }
            }
        }
    }
}
