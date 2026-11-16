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
use crate::lower::{self, Body, Clock, Kind, Op, Ref};

/// Why a process stays in the event kernel.
pub(crate) enum Why {
    /// Outside the subset.
    Subset(Reason),
    /// A process of the testbench kind.
    Behavioural(String),
    /// In the subset; cycle mode does not implement something it needs yet.
    NotYet(String),
}

/// The region: members (clocked first, then combinational in dependency order) and the boundary.
pub(crate) struct Selection {
    pub members: Vec<(usize, Body)>,
    /// The steps of the settle, as indices into `members`: the combinational members and the
    /// reset branches of clocked ones, in dependency order; a member of a loop group once per
    /// pass level of what it drives.
    pub settle: Vec<usize>,
    /// The loop groups of the members, as spans of `settle`.
    pub loops: Vec<Loop>,
    pub out: Vec<(usize, Why)>,
    /// The edge classes: each clock and edge the clocked members use, in the order met.
    pub clocks: Vec<Clock>,
    /// Loop groups of the processes in the subset (before what is implemented is known).
    pub groups: Vec<dep::Group>,
    /// Processes in the subset.
    pub fits: usize,
    /// Leaves members drive, and of those the ones something outside reads (exported).
    pub driven: BTreeSet<LeafId>,
    pub outputs: BTreeSet<LeafId>,
    /// Leaves something outside may wait on (or that it is not known it does not).
    pub waited: BTreeSet<LeafId>,
    /// Leaves members read that no member drives (read from the kernel at each wake); of
    /// those, the ones a combinational member reads wake the region.
    pub inputs: BTreeSet<LeafId>,
    pub wakes: BTreeSet<LeafId>,
    /// The wakes whose change the region settles and exports at once, with the reason: a path
    /// through combinational code (members, reset branches) to an exported output (its reader
    /// outside) or to combinational code with a check. A change of any other input is read
    /// when it comes and settled at the next wake that needs it (an edge at the latest).
    pub at_once: BTreeMap<LeafId, Reach>,
    /// Leaves a clocked member writes and a clocked member reads: written to a next-state
    /// block, committed after all clocked bodies ran.
    pub next: BTreeSet<LeafId>,
}

/// Why a change of an input is settled at once: the exported output (and a process outside that
/// reads it, if one is known) or the member process (its index) with a check its combinational
/// path reaches.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Reach {
    Output(LeafId, Option<usize>),
    Check(usize),
}

/// A loop group of the settle: its steps, each member run at the pass levels of what it drives
/// (`dep::Group::runs`), its last run the one on settled values.
pub(crate) struct Loop {
    pub at: std::ops::Range<usize>,
    /// Its members, and the passes of all of them in order that the runs replace (the report).
    pub procs: usize,
    pub k: u32,
}

/// Kernel parts of leaves.
type Parts = Vec<(LeafId, yavs_kernel::Range)>;

/// What fits, why the rest does not, and the loop groups of what fits.
type Subset = (BTreeMap<usize, Fit>, Vec<(usize, Why)>, Vec<dep::Group>);

/// The subset verdict of every process: what fits, and why the others do not.
pub(crate) fn subset(db: &Db, e: &Elaborated) -> Subset {
    let t0 = std::time::Instant::now();
    let mut fits: BTreeMap<usize, Fit> = BTreeMap::new();
    let mut out: Vec<(usize, Why)> = Vec::new();
    let mut callees = check::Callees::default();
    let mut consts = check::Consts::default();
    let mut slow: Vec<(u128, usize)> = Vec::new();
    for pi in 0..e.procs.len() {
        let t = std::time::Instant::now();
        let v = check::check(db, e, pi, &mut callees, &mut consts);
        slow.push((t.elapsed().as_micros(), pi));
        match v {
            check::Verdict::Fit(f) => {
                fits.insert(pi, f);
            }
            check::Verdict::Behavioural(w) => out.push((pi, Why::Behavioural(w))),
            check::Verdict::Out(r) => out.push((pi, Why::Subset(r))),
        }
    }
    slow.sort_unstable_by(|a, b| b.cmp(a));
    for &(us, pi) in slow.iter().take(8) {
        stat(|| format!("check {us} us: {}", e.proc_label(&e.procs[pi])));
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
    for (pi, r) in shared_rule(db, e, &fits) {
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
        stat(|| format!("subset round: {} ms", t0.elapsed().as_millis()));
        let a = analyse(db, e, fits.iter().map(|(pi, f)| (*pi, f)));
        stat(|| format!("subset analyse: {} ms", t0.elapsed().as_millis()));
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
fn analyse<'f>(
    db: &Db,
    e: &Elaborated,
    fits: impl Iterator<Item = (usize, &'f Fit)>,
) -> dep::Analysis {
    let _ = db;
    let n = e.procs.len();
    let mut procs: Vec<(usize, Vec<dep::Out>)> = fits
        .filter(|(_, f)| !f.outs.is_empty() || f.form == Form::Comb)
        .map(|(pi, f)| (pi, f.outs.clone()))
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
        g.runs.retain(|r| r.0 < n);
    }
    a.groups.retain(|g| !g.procs.is_empty());
    a.order.retain(|&p| p < n);
    a
}

/// Developers (`YAVS_CYCLE_STATS`): a line of the selection's timing.
fn stat(what: impl FnOnce() -> String) {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    if *ON.get_or_init(|| std::env::var_os("YAVS_CYCLE_STATS").is_some()) {
        eprintln!("cycle stats: {}", what());
    }
}

pub(crate) fn select(db: &Db, e: &Elaborated) -> Selection {
    let t0 = std::time::Instant::now();
    let (fits, mut out, groups) = subset(db, e);
    stat(|| format!("select: subset done {} ms", t0.elapsed().as_millis()));
    let mut nfits = fits.len();
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
        // Developers bisecting a difference: `YAVS_CYCLE_EXCLUDE=a,b` keeps the processes whose
        // names hold one of those texts out.
        if let Some(x) = std::env::var("YAVS_CYCLE_EXCLUDE")
            .ok()
            .filter(|x| !x.is_empty())
            && let name = e.proc_name(&e.procs[pi])
            && x.split(',').any(|t| !t.is_empty() && name.contains(t))
        {
            out.push((pi, Why::NotYet("excluded by YAVS_CYCLE_EXCLUDE".into())));
            continue;
        }
        match lower::lower(db, e, pi) {
            // The lowering must see the shape the checker proved, clock included.
            Ok(b) if differs(e, &f, &b).is_some() => {
                out.push((pi, Why::NotYet(differs(e, &f, &b).expect("checked"))))
            }
            // The asserts and reports of the top-level unit are evaluated (A19): at a clock edge
            // in region code, else the process stays in the event kernel.
            Ok(b) if b.dropped > 0 && in_top_unit(e, pi) => {
                nfits -= 1;
                out.push((
                    pi,
                    Why::Behavioural(format!(
                        "it holds an assert or report of the top-level unit that cycle mode \
                         evaluates only in the event kernel: {}",
                        b.note_why
                            .as_deref()
                            .unwrap_or("not evaluated in region code")
                    )),
                ))
            }
            Ok(b) => {
                taken.insert(pi, (f, b));
            }
            Err(w) => out.push((pi, Why::NotYet(w))),
        }
    }
    stat(|| format!("select: lowered {} ms", t0.elapsed().as_millis()));
    // Combinational logic feeding a process outside whose result depends on how often it runs.
    for (pi, w) in run_count_cone(db, e, &out) {
        if taken.remove(&pi).is_some() {
            out.push((pi, Why::NotYet(w)));
        }
    }
    // The history processes read (A10), and the sources of implicit signals.
    let mut memo = BTreeMap::new();
    let hist: Vec<Vec<Hist>> = (0..e.procs.len())
        .map(|pi| history_reads(db, e, pi, &mut memo))
        .collect();
    let mut implicit: BTreeSet<LeafId> = BTreeSet::new();
    for l in &e.nets.leaves {
        if let Some((_, src, _)) = l.implicit {
            implicit.insert(src);
        }
        implicit.extend(l.implicit_more.iter().map(|x| x.0));
    }
    stat(|| format!("select: history {} ms", t0.elapsed().as_millis()));
    // Leave out what the set as a whole does not allow yet, until nothing changes.
    loop {
        stat(|| format!("select: round at {} ms", t0.elapsed().as_millis()));
        let mut gone: Vec<(usize, String)> = Vec::new();
        // A signal whose history a process outside reads keeps its drivers out: the region
        // keeps no history of what it drives (outside the subset).
        let hist_gone = history_rule(db, e, &taken, &hist, &implicit);
        if !hist_gone.is_empty() {
            for (pi, r) in hist_gone {
                if taken.remove(&pi).is_some() {
                    out.push((pi, Why::Subset(r)));
                    // Outside the subset after all: not counted as fitting.
                    nfits -= 1;
                }
            }
            continue;
        }
        // Leaves with a source no process owns: a constant source, or a port edge or converted
        // port the kernel updates (a member would read its own driving value from the image,
        // not the resolved one). The actual of an `in` port only feeds it: its value is its
        // drivers', and the kernel reads it from the region's export.
        let mut consts: BTreeSet<LeafId> = e.nets.consts.iter().map(|c| c.0).collect();
        for pe in &e.nets.ports {
            consts.insert(pe.formal.0);
            if pe.mode != yavs_kernel::PortMode::In {
                consts.insert(pe.actual.0);
            }
        }
        for cp in &e.nets.conv_ports {
            consts.extend(cp.formal.iter().map(|x| x.0));
            if cp.mode != yavs_kernel::PortMode::In {
                consts.extend(cp.actual.iter().map(|x| x.0));
            }
        }
        // Per leaf, one driving process outside the region.
        let mut outside: BTreeMap<LeafId, usize> = BTreeMap::new();
        for d in &e.nets.drivers {
            if !taken.contains_key(&d.proc.index()) {
                outside.entry(d.leaf).or_insert(d.proc.index());
            }
        }
        // Leaves members drive.
        let inside: BTreeSet<LeafId> = (taken.keys())
            .flat_map(|&pi| e.procs[pi].drivers.iter().map(|&d| e.nets.drivers[d].leaf))
            .collect();
        for (&pi, (f, b)) in &taken {
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
            // A checker of the top-level unit evaluated in region code (A19) reads what members
            // drive only: it sees the values the normal mode's checker would see of them (the
            // region exports two-state values), never a metavalue of the kernel read as 0.
            // What the checker counts too: the lowering reads nothing of a value it folds
            // (`is_x` is false on two-state values).
            let fit_reads = f.reads.iter().map(|(k, _, _)| LeafId(k as u32));
            if b.notes > 0
                && let Some(l) = (b.reads.iter().copied()).chain(fit_reads).find(|&l| {
                    b.clock.is_none_or(|c| c.leaf != l)
                        && (outside.contains_key(&l) || !inside.contains(&l) || consts.contains(&l))
                })
            {
                gone.push((
                    pi,
                    format!(
                        "it checks {}, which is not driven by the region alone; its asserts and \
                         reports run in the event kernel, on the values the kernel has",
                        e.leaf_name(l)
                    ),
                ));
                continue;
            }
            // The reset branch runs again in the settle, apart from the edge test.
            if b.reset.is_some() && reset_piece(&b.ops).is_none() {
                gone.push((
                    pi,
                    "the lowering gives its asynchronous reset another shape than the settle \
                     takes it in"
                        .into(),
                ));
            }
        }
        stat(|| format!("select: sources {} ms", t0.elapsed().as_millis()));
        if gone.is_empty() {
            gone = ram_rule(db, e, &taken);
        }
        stat(|| format!("select: ram {} ms", t0.elapsed().as_millis()));
        if gone.is_empty() {
            gone = cross_delta(db, e, &taken);
        }
        stat(|| format!("select: cross {} ms", t0.elapsed().as_millis()));
        if gone.is_empty() {
            // The order and the loop groups of what is left: a group runs as a loop of passes.
            let a = analyse(db, e, taken.iter().map(|(pi, (f, _))| (*pi, f)));
            let mut members = Vec::new();
            let order: Vec<usize> = a
                .order
                .iter()
                .copied()
                .filter(|pi| taken.contains_key(pi))
                .collect();
            // What the reset branch of each clocked member reads (from the checker), for the
            // wakes: it runs in the settle while the reset holds.
            let mut reset_reads: BTreeSet<LeafId> = BTreeSet::new();
            for (f, b) in taken.values() {
                if let Some((r, ..)) = b.reset {
                    reset_reads
                        .extend(out_reads(f).filter(|&l| l != r || e.nets.leaf(l).ty.len > 1));
                }
            }
            let mut rest: BTreeMap<usize, Body> =
                taken.into_iter().map(|(pi, (_, b))| (pi, b)).collect();
            let clocked: Vec<usize> = (rest.iter())
                .filter(|(_, b)| b.kind == Kind::Clocked)
                .map(|(pi, _)| *pi)
                .collect();
            for pi in clocked {
                members.push((pi, rest.remove(&pi).expect("listed")));
            }
            // The settle: combinational members and the reset branches of clocked ones, in the
            // order of the element graph.
            let mut resets: BTreeMap<usize, usize> = (members.iter().enumerate())
                .filter(|(_, m)| m.1.reset.is_some())
                .map(|(k, m)| (m.0, k))
                .collect();
            let mut settle = Vec::new();
            // Each group's members are contiguous in the order, in the order of a pass.
            let group_of: BTreeMap<usize, usize> = (a.groups.iter().enumerate())
                .flat_map(|(g, x)| x.procs.iter().map(move |&pi| (pi, g)))
                .collect();
            let mut loops: Vec<Loop> = Vec::new();
            // The step of each member placed so far.
            let mut step: BTreeMap<usize, usize> = BTreeMap::new();
            let mut place = |pi: usize, members: &mut Vec<(usize, Body)>| -> Option<usize> {
                if let Some(&k) = step.get(&pi) {
                    return Some(k);
                }
                let k = if let Some(b) = rest.remove(&pi) {
                    members.push((pi, b));
                    members.len() - 1
                } else {
                    resets.remove(&pi)?
                };
                step.insert(pi, k);
                Some(k)
            };
            let mut placed: BTreeSet<usize> = BTreeSet::new();
            for pi in order {
                match group_of.get(&pi) {
                    // The whole group at its first member (its members are contiguous).
                    Some(&g) if placed.insert(g) => {
                        let at = settle.len();
                        for &(q, _) in &a.groups[g].runs {
                            let k = place(q, &mut members).expect("a group member is a piece");
                            settle.push(k);
                        }
                        loops.push(Loop {
                            at: at..settle.len(),
                            procs: a.groups[g].procs.len(),
                            k: a.groups[g].k,
                        });
                    }
                    Some(_) => {}
                    None => settle.extend(place(pi, &mut members)),
                }
            }
            // Members the dependence order does not list (no outputs): any place.
            for m in rest {
                members.push(m);
                settle.push(members.len() - 1);
            }
            // Reset branches the order does not list (they assign nothing): any place.
            settle.extend(resets.into_values());
            out.sort_by_key(|o| o.0);
            stat(|| format!("select: order {} ms", t0.elapsed().as_millis()));
            let mut sel = boundary(db, e, members, out, &reset_reads);
            stat(|| format!("select: boundary {} ms", t0.elapsed().as_millis()));
            sel.settle = settle;
            sel.loops = loops;
            sel.groups = groups;
            sel.fits = nfits;
            return sel;
        }
        for (pi, w) in gone {
            if taken.remove(&pi).is_some() {
                out.push((pi, Why::NotYet(w)));
            }
        }
    }
}

/// The signal of a clock, with its element when the signal has several.
pub(crate) fn clock_name(e: &Elaborated, c: &Clock) -> String {
    match e.nets.leaf(c.leaf).ty.len {
        1 => e.leaf_name(c.leaf),
        _ => format!("{} (element {})", e.leaf_name(c.leaf), c.elem),
    }
}

/// The reset branch of a clocked member with an asynchronous reset, as the settle runs it: the
/// lowering's `if <reset> then A else (if <edge> then B)` (after the operations computing the
/// condition) without its edge part; `None` for another shape.
pub(crate) fn reset_piece(ops: &[Op]) -> Option<Vec<Op>> {
    let (Op::If { cond, then, else_ }, head) = ops.split_last()? else {
        return None;
    };
    let [
        Op::If {
            cond: Ref::Edge, ..
        },
    ] = &else_[..]
    else {
        return None;
    };
    let mut v = head.to_vec();
    v.push(Op::If {
        cond: *cond,
        then: then.clone(),
        else_: Vec::new(),
    });
    Some(v)
}

/// The signal leaves the outputs of a fit depend on (of a clocked one: its reset branch).
fn out_reads(f: &Fit) -> impl Iterator<Item = LeafId> + '_ {
    let mut ks: BTreeSet<u64> = BTreeSet::new();
    for o in &f.outs {
        ks.extend(o.dep.ident.map(|i| i.0));
        ks.extend(o.dep.set.iter().map(|(k, _, _)| k));
    }
    ks.into_iter()
        .filter(|k| k & VAR == 0)
        .map(|k| LeafId(k as u32))
}

/// The process every driver of `c`'s element belongs to when that process assigns without
/// delay and has one wait (a clock model making a pair): the clocks it makes have their edges in
/// one delta. Else the clock's element alone.
fn delta_key(db: &Db, e: &Elaborated, c: &Clock) -> (bool, usize, u32) {
    let mut ps = (e.nets.drivers.iter())
        .filter(|d| d.leaf == c.leaf && d.range.offset <= c.elem && c.elem < d.range.end())
        .map(|d| d.proc.index());
    if let Some(p) = ps.next()
        && ps.all(|q| q == p)
        && one_step(db, e, p)
    {
        return (true, p, 0);
    }
    (false, c.leaf.index(), c.elem)
}

/// Every activation of process `pi` starts at the first delta of a time step and assigns
/// without delay: one `wait for` (beside any `wait;`), no procedure call (it might wait), no
/// `after`. ponytail: a `wait for` lasting 0 ns would resume it in a later delta; clock models do
/// not, a run-time test of the delta would close it.
fn one_step(db: &Db, e: &Elaborated, pi: usize) -> bool {
    use yavs_sem::hir::{Delay, Rhs, Waveform};
    let p = &e.procs[pi];
    if p.actual.is_some() {
        return false;
    }
    let StmtKind::Process {
        sensitivity: Sensitivity::None,
        body,
        ..
    } = &db.stmts[p.stmt].kind
    else {
        return false;
    };
    let mut all = Vec::new();
    deep(db, body, &mut all);
    let mut waits = 0;
    for s in all {
        match &db.stmts[s].kind {
            StmtKind::Wait {
                on,
                until: None,
                timeout,
            } if on.is_empty() => waits += usize::from(timeout.is_some()),
            StmtKind::Wait { .. } => return false,
            StmtKind::ProcCall(_) => return false,
            StmtKind::SignalAssign { delay, rhs, .. } => {
                let ws: Vec<&Waveform> = match rhs {
                    Rhs::Simple(w) => vec![w],
                    Rhs::Conditional(a) => a.iter().map(|x| &x.0).collect(),
                    Rhs::Selected { alts, .. } => alts.iter().map(|x| &x.0).collect(),
                };
                let timed = ws.iter().any(|w| match w {
                    Waveform::Elements(els) => els.len() != 1 || els[0].after.is_some(),
                    Waveform::Unaffected => false,
                });
                if !matches!(delay, Delay::Default) || timed {
                    return false;
                }
            }
            _ => {}
        }
    }
    waits == 1
}

/// Members that would hand the region's settled value where the normal mode samples one that
/// has not settled yet: combinational logic (a combinational member, or the reset branch of a
/// clocked one) between a register of one clock and a register of another clock whose edges
/// may come in a later delta of the same time step (a clock gated or copied in the kernel, two
/// clock processes of their own). In the normal mode such a register samples the logic before
/// its new value arrives; the region settles it at the first edge's wake. Clocks one process
/// makes in one step (`delta_key`) or one signal's two edges never differ by a delta.
/// Element runs are taken as whole leaves (a superset).
fn cross_delta(
    db: &Db,
    e: &Elaborated,
    taken: &BTreeMap<usize, (Fit, Body)>,
) -> Vec<(usize, String)> {
    let mut keys: Vec<(bool, usize, u32)> = Vec::new();
    let mut names: Vec<Clock> = Vec::new();
    let mut bit: BTreeMap<(LeafId, u32), u64> = BTreeMap::new();
    for (_, b) in taken.values() {
        if let Some(c) = b.clock
            && !bit.contains_key(&(c.leaf, c.elem))
        {
            let k = delta_key(db, e, &c);
            let g = keys.iter().position(|x| *x == k).unwrap_or_else(|| {
                keys.push(k);
                names.push(c);
                keys.len() - 1
            });
            // ponytail: beyond 63 groups they share the top bit, which counts as differing from
            // everything (sound, coarse); a set per leaf if a design ever has that many.
            bit.insert((c.leaf, c.elem), 1 << g.min(63));
        }
    }
    if keys.len() < 2 {
        return Vec::new();
    }
    const TOP: u64 = 1 << 63;
    let differ = |t: u64, f: u64| {
        t != 0 && f != 0 && (t | f).count_ones() + u32::from((t | f) & TOP != 0) > 1
    };
    // Registers by clock group, and what registers of each group read.
    let mut reg: BTreeMap<LeafId, u64> = BTreeMap::new();
    let mut feeds: BTreeMap<LeafId, u64> = BTreeMap::new();
    // The combinational pieces: process, reads, writes.
    let mut pieces: Vec<(usize, Vec<LeafId>, Vec<LeafId>)> = Vec::new();
    for (&pi, (f, b)) in taken {
        match b.clock {
            Some(c) => {
                let g = bit[&(c.leaf, c.elem)];
                for &w in &b.writes {
                    *reg.entry(w).or_default() |= g;
                }
                for &r in &b.reads {
                    *feeds.entry(r).or_default() |= g;
                }
                if b.reset.is_some() {
                    let mut ws: Vec<LeafId> =
                        f.outs.iter().map(|o| LeafId(o.leaf as u32)).collect();
                    ws.dedup();
                    pieces.push((pi, out_reads(f).collect(), ws));
                }
            }
            None => pieces.push((
                pi,
                b.reads.iter().copied().collect(),
                b.writes.iter().copied().collect(),
            )),
        }
    }
    // What reaches each leaf from registers through the pieces, and which registers each leaf
    // reaches through them.
    let mut taint: BTreeMap<LeafId, u64> = BTreeMap::new();
    loop {
        let mut changed = false;
        for (_, rs, ws) in &pieces {
            let t = (rs.iter())
                .map(|r| reg.get(r).copied().unwrap_or(0) | taint.get(r).copied().unwrap_or(0))
                .fold(0, |a, b| a | b);
            for w in ws {
                let x = taint.entry(*w).or_default();
                if *x | t != *x {
                    *x |= t;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    loop {
        let mut changed = false;
        for (_, rs, ws) in pieces.iter().rev() {
            let f = (ws.iter())
                .map(|w| feeds.get(w).copied().unwrap_or(0))
                .fold(0, |a, b| a | b);
            for r in rs {
                let x = feeds.entry(*r).or_default();
                if *x | f != *x {
                    *x |= f;
                    changed = true;
                }
            }
        }
        if !changed {
            break;
        }
    }
    let name = |m: u64| clock_name(e, &names[m.trailing_zeros() as usize]);
    let mut gone = Vec::new();
    for (pi, rs, ws) in &pieces {
        let t = (rs.iter())
            .map(|r| reg.get(r).copied().unwrap_or(0) | taint.get(r).copied().unwrap_or(0))
            .fold(0, |a, b| a | b);
        let f = (ws.iter())
            .map(|w| feeds.get(w).copied().unwrap_or(0))
            .fold(0, |a, b| a | b);
        if differ(t, f) {
            let lo = |m: u64| m.isolate_lowest_one();
            let a = lo(t);
            let (from, to) = match (f & !a, lo(t & !a)) {
                (0, 0) => (a, a),
                (0, b) => (b, a),
                (g, _) => (a, lo(g)),
            };
            gone.push((*pi, format!(
                "it computes, from registers on {}, a value a register on {} samples, whose edges \
                 may come in a later delta of the same time step: the normal mode samples it \
                 before it settles, the region settles it at once (not modelled yet)",
                name(from),
                name(to)
            )));
        }
    }
    gone
}

/// A memory: a kernel signal of more than 64 words (4096 logic elements, or 64 integers). A
/// clocked member's writes of one are kept back to the commit (`build`), so it needs no
/// next-state copy; that takes writes of elements or slices at run-time indices only. One read
/// outside the region would be exported whole at every change.
pub(crate) fn big_signal(e: &Elaborated, k: u32) -> bool {
    let nets = &e.nets;
    let n = nets.signal_len(yavs_kernel::SignalId(k));
    match nets.leaf(nets.packs[k as usize].0).ty.elem {
        yavs_kernel::Elem::Logic => n > 4096,
        yavs_kernel::Elem::Bytes(_) => n > 64,
    }
}

/// The kernel signals `ops` write, each with how: at run-time indices (`IndexWrite`), otherwise,
/// or both.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) struct How {
    pub indexed: bool,
    pub other: bool,
}

pub(crate) fn sig_writes(ops: &[Op], out: &mut BTreeMap<u32, How>) {
    let none = How {
        indexed: false,
        other: false,
    };
    for op in ops {
        match op {
            Op::If { then, else_, .. } => {
                sig_writes(then, out);
                sig_writes(else_, out);
            }
            Op::IndexWrite {
                dst: Ref::Sig(k), ..
            } => out.entry(*k).or_insert(none).indexed = true,
            op => {
                let w = std::cell::RefCell::new(Vec::new());
                op.map(
                    &|r| r,
                    &|r| {
                        w.borrow_mut().push(r);
                        r
                    },
                    &|x| x,
                );
                for r in w.into_inner() {
                    if let Ref::Sig(k) = r {
                        out.entry(k).or_insert(none).other = true;
                    }
                }
            }
        }
    }
}

/// Clocked members writing a memory both at run-time indices and otherwise, or one read outside
/// the region: not implemented yet. A big signal no clocked member writes at a run-time index is
/// no memory (whole writes go through a next-state block, as for any signal).
fn ram_rule(db: &Db, e: &Elaborated, taken: &BTreeMap<usize, (Fit, Body)>) -> Vec<(usize, String)> {
    let mut rams: BTreeMap<u32, Vec<(usize, bool, bool)>> = BTreeMap::new();
    for (&pi, (_, b)) in taken.iter().filter(|(_, (_, b))| b.kind == Kind::Clocked) {
        let mut w = BTreeMap::new();
        sig_writes(&b.ops, &mut w);
        // A reset branch also runs in the settle, where its writes go in place.
        let mut rw = BTreeMap::new();
        if b.reset.is_some()
            && let Some(piece) = reset_piece(&b.ops)
        {
            sig_writes(&piece, &mut rw);
        }
        for (k, how) in w
            .into_iter()
            .filter(|&(k, h)| h.indexed && big_signal(e, k))
        {
            rams.entry(k)
                .or_default()
                .push((pi, !how.other, rw.contains_key(&k)));
        }
    }
    if rams.is_empty() {
        return Vec::new();
    }
    let members: BTreeSet<usize> = taken.keys().copied().collect();
    let seen = (!outside_reads_all(db, e, &members)).then(|| outside_reads(db, e, &members).0);
    let mut gone = Vec::new();
    for (k, ws) in rams {
        let (a, z) = e.nets.packs[k as usize];
        let name = e.leaf_name(a);
        // A memory of logic elements is exported by the words its writes touched (`rt::RamWrite`).
        let logic = e.nets.leaf(a).ty.elem == yavs_kernel::Elem::Logic;
        let outside = !logic
            && seen
                .as_ref()
                .is_none_or(|s| (a.0..z.0).any(|l| s.contains(&LeafId(l))));
        for (pi, ok, in_reset) in ws {
            if in_reset {
                gone.push((
                    pi,
                    format!(
                        "its asynchronous reset branch writes the memory {name}, which cycle mode \
                         does not take yet"
                    ),
                ));
            } else if !ok {
                gone.push((
                    pi,
                    format!(
                        "it writes the memory {name} both at indices chosen at run time and \
                         otherwise, which cycle mode does not take for a signal of this size yet"
                    ),
                ));
            } else if outside {
                gone.push((
                    pi,
                    format!(
                        "the memory {name} it writes is read outside the region, which cycle mode \
                         does not export yet"
                    ),
                ));
            }
        }
    }
    gone
}

fn boundary(
    db: &Db,
    e: &Elaborated,
    members: Vec<(usize, Body)>,
    out: Vec<(usize, Why)>,
    reset_reads: &BTreeSet<LeafId>,
) -> Selection {
    let mut clocks: Vec<Clock> = Vec::new();
    for c in members.iter().filter_map(|m| m.1.clock) {
        if !clocks.contains(&c) {
            clocks.push(c);
        }
    }
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
    // A reset branch runs in the settle: what it reads wakes the region like a combinational
    // member's reads (the reset itself is a wake of its own).
    comb_reads.extend(reset_reads.iter().copied());
    let inputs: BTreeSet<LeafId> = reads.difference(&driven).copied().collect();
    let wakes = comb_reads.difference(&driven).copied().collect();
    let next = clocked_writes
        .intersection(&clocked_reads)
        .copied()
        .collect();
    let (outputs, waited, readers) = if outside_reads_all(db, e, &is_member) {
        (driven.clone(), driven.clone(), BTreeMap::new())
    } else {
        let (seen, waited, readers) = outside_reads(db, e, &is_member);
        (
            driven.intersection(&seen).copied().collect(),
            waited,
            readers,
        )
    };
    let at_once = at_once(&members, reset_reads, &outputs, &readers, &wakes);
    Selection {
        members,
        settle: Vec::new(),
        loops: Vec::new(),
        out,
        clocks,
        groups: Vec::new(),
        fits: 0,
        driven,
        outputs,
        waited,
        inputs,
        wakes,
        at_once,
        next,
    }
}

/// The wakes with a combinational path (combinational members, reset branches and the resets
/// themselves; leaf by leaf, a read reaching every leaf the code writes) to an exported output or
/// to combinational code with a check: a change of such an input is visible before the next edge.
fn at_once(
    members: &[(usize, Body)],
    reset_reads: &BTreeSet<LeafId>,
    outputs: &BTreeSet<LeafId>,
    readers: &BTreeMap<LeafId, usize>,
    wakes: &BTreeSet<LeafId>,
) -> BTreeMap<LeafId, Reach> {
    let mut succ: BTreeMap<LeafId, BTreeSet<LeafId>> = BTreeMap::new();
    let mut checked: BTreeMap<LeafId, usize> = BTreeMap::new();
    for (pi, b) in members {
        let reads: Vec<LeafId> = match b.kind {
            Kind::Comb => b.reads.iter().copied().collect(),
            // A reset branch: what it reads and the reset itself (made inside, it is no wake).
            Kind::Clocked if let Some((r, ..)) = b.reset => (b.reads.intersection(reset_reads))
                .copied()
                .chain([r])
                .collect(),
            Kind::Clocked => continue,
        };
        for r in reads {
            succ.entry(r).or_default().extend(b.writes.iter().copied());
            if !b.checks.is_empty() {
                checked.entry(r).or_insert(*pi);
            }
        }
    }
    let mut out = BTreeMap::new();
    for &w in wakes {
        let (mut seen, mut todo) = (BTreeSet::from([w]), vec![w]);
        while let Some(l) = todo.pop() {
            if let Some(&pi) = checked.get(&l) {
                out.insert(w, Reach::Check(pi));
                break;
            }
            if l != w && outputs.contains(&l) {
                out.insert(w, Reach::Output(l, readers.get(&l).copied()));
                break;
            }
            for &n in succ.get(&l).into_iter().flatten() {
                if seen.insert(n) {
                    todo.push(n);
                }
            }
        }
    }
    out
}

/// Something may read any signal: an external name anywhere in the design.
fn outside_reads_all(db: &Db, _e: &Elaborated, _members: &BTreeSet<usize>) -> bool {
    !db.externals.is_empty()
}

/// The leaves anything that is not a member may read, and of those the ones something outside
/// may wait on. Read: the code of the other processes (by the objects their names denote, whole),
/// port edges and implicit signals the kernel updates, guards, and the signals of packages. A
/// call of a subprogram declared in a design unit rather than a package reads what its body (and
/// the local subprograms it calls) names; one the scan cannot follow counts as reading every
/// signal of the design. Waited on: a sensitivity list (`all`: every read), the signals of a wait
/// statement (its `until` condition when it names none), in the process or in a local subprogram
/// it calls (a wait on a formal: the call's actuals), every read of a statement that is not a
/// process, and everything above that is not process code.
fn outside_reads(
    db: &Db,
    e: &Elaborated,
    members: &BTreeSet<usize>,
) -> (BTreeSet<LeafId>, BTreeSet<LeafId>, BTreeMap<LeafId, usize>) {
    let mut seen = BTreeSet::new();
    let mut waited = BTreeSet::new();
    let mut readers = BTreeMap::new();
    let all = || -> BTreeSet<LeafId> { (0..e.nets.leaves.len() as u32).map(LeafId).collect() };
    let mut memo = BTreeMap::new();
    for (pi, p) in e.procs.iter().enumerate() {
        if members.contains(&pi) {
            continue;
        }
        let Some((reads, waits)) = proc_reads(db, e, pi, &mut memo) else {
            return (all(), all(), BTreeMap::new());
        };
        for &l in &reads {
            readers.entry(l).or_insert(pi);
        }
        seen.extend(reads);
        waited.extend(waits);
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
    (seen, waited, readers)
}

/// The signal leaves process `pi` may read (its names, whole objects, and those of the local
/// subprograms it calls), and of those the ones it waits on; `None` when the scan cannot follow
/// it (an external name, a call it cannot see into): it may read every leaf.
fn proc_reads(
    db: &Db,
    e: &Elaborated,
    pi: usize,
    memo: &mut BTreeMap<yavs_sem::DeclId, Option<CallSigs>>,
) -> Option<(BTreeSet<LeafId>, BTreeSet<LeafId>)> {
    let p = &e.procs[pi];
    let mut seen = BTreeSet::new();
    let mut waited = BTreeSet::new();
    // Names read, and names waited on.
    let mut lsps = Vec::new();
    let mut waits = Vec::new();
    let mut calls = Vec::new();
    let mut follow = true;
    if let Some(pa) = &p.actual {
        db.reads(pa.expr, &mut waits);
        follow &= exprs_calls(db, &[pa.expr], &mut calls);
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
                    follow &= stmt_calls(db, s, &mut calls);
                }
            }
            _ => {
                db.stmt_reads(p.stmt, &mut waits);
                follow &= stmt_calls(db, p.stmt, &mut calls);
            }
        }
    }
    if !follow {
        return None;
    }
    // A subprogram declared outside a package reads the signals its body names and waits on
    // those its waits name; a wait on a formal waits on the call's actuals.
    for (c, args) in &calls {
        let sum = callee_sigs(db, *c, 0, memo)?;
        lsps.extend(sum.reads);
        waits.extend(sum.waits);
        if sum.waits_formal {
            waits.extend(actual_reads(db, args));
        }
    }
    for (l, wait) in lsps
        .iter()
        .map(|l| (l, false))
        .chain(waits.iter().map(|l| (l, true)))
    {
        match l.root {
            ObjRoot::Decl(d) => {
                let (a, z) = e.nets.obj(e.signal(p.scope, d)?).leaves;
                let leaves = (a.0..z.0).map(LeafId);
                if wait {
                    waited.extend(leaves.clone());
                }
                seen.extend(leaves);
            }
            ObjRoot::External(_) => return None,
            ObjRoot::ImplicitSignal(_) => {}
        }
    }
    Some((seen, waited))
}

/// The combinational processes (members or not) in the fan-in of what a process outside the
/// subset reads whose result depends on how often it runs: a combinational process with state or
/// a latch (E4119, E4120), which keeps a value of an earlier run. (An incomplete sensitivity list,
/// E4121, keeps nothing and is not counted: its runs see the values of the deltas the listed
/// signals change in.) Back from its reads through every
/// combinational process driving them, up to registers (edge-triggered processes, whose outputs
/// come in the delta after the edge in both modes). The region settles such logic in one delta
/// where the normal mode spreads it over several, so the reader would run another number of
/// times (Microwatt's `dcache_request` gives `req_nc` the `nc` of its previous run). Each with
/// the reason; `None` reads (a name the scan cannot follow) reach every leaf.
fn run_count_cone(db: &Db, e: &Elaborated, out: &[(usize, Why)]) -> Vec<(usize, String)> {
    let sensitive: Vec<usize> = (out.iter())
        .filter_map(|(pi, w)| match w {
            Why::Subset(r) if [crate::codes::LATCH, crate::codes::STATE].contains(&r.code) => {
                Some(*pi)
            }
            _ => None,
        })
        .collect();
    if sensitive.is_empty() {
        return Vec::new();
    }
    let mut drivers: BTreeMap<LeafId, Vec<usize>> = BTreeMap::new();
    for d in &e.nets.drivers {
        drivers.entry(d.leaf).or_default().push(d.proc.index());
    }
    // The testbench's processes run by their waits, not as logic: the walk stops there.
    let behavioural: BTreeSet<usize> = (out.iter())
        .filter(|(_, w)| matches!(w, Why::Behavioural(_)))
        .map(|(pi, _)| *pi)
        .collect();
    let every = || (0..e.nets.leaves.len() as u32).map(LeafId);
    let mut memo = BTreeMap::new();
    let mut procs: BTreeSet<usize> = sensitive.iter().copied().collect();
    let mut leaves: BTreeSet<LeafId> = BTreeSet::new();
    let mut work: Vec<(LeafId, usize)> = Vec::new();
    for &q in &sensitive {
        match proc_reads(db, e, q, &mut memo) {
            Some((r, _)) => work.extend(r.into_iter().map(|l| (l, q))),
            None => work.extend(every().map(|l| (l, q))),
        }
    }
    let mut gone = Vec::new();
    while let Some((l, q)) = work.pop() {
        if !leaves.insert(l) {
            continue;
        }
        for &d in drivers.get(&l).into_iter().flatten() {
            if e.procs[d].trigger.is_some() || behavioural.contains(&d) || !procs.insert(d) {
                continue;
            }
            gone.push((
                d,
                format!(
                    "it computes, combinationally, {}, in the fan-in of {}, a process outside \
                     the region whose result depends on how often it runs (it keeps a value of an \
                     earlier run): the region would wake it at other deltas",
                    e.leaf_name(l),
                    e.proc_label(&e.procs[q])
                ),
            ));
            match proc_reads(db, e, d, &mut memo) {
                Some((r, _)) => work.extend(r.into_iter().map(|x| (x, q))),
                None => work.extend(every().map(|x| (x, q))),
            }
        }
    }
    gone
}

/// The members driving a signal whose history a process outside the region reads, each with
/// the reason (the reading process and attribute); the sources of implicit signals count too.
fn history_rule(
    db: &Db,
    e: &Elaborated,
    taken: &BTreeMap<usize, (Fit, Body)>,
    hist: &[Vec<Hist>],
    implicit: &BTreeSet<LeafId>,
) -> Vec<(usize, Reason)> {
    let mut read: BTreeMap<LeafId, (usize, yavs_diag::Span, &str)> = BTreeMap::new();
    let mut every: Option<(usize, yavs_diag::Span, &str)> = None;
    for (pi, hs) in hist.iter().enumerate() {
        if taken.contains_key(&pi) {
            continue;
        }
        for (ls, span, what) in hs {
            match ls {
                Some(ls) => {
                    for &l in ls {
                        read.entry(l).or_insert((pi, *span, what));
                    }
                }
                None => {
                    every.get_or_insert((pi, *span, what));
                }
            }
        }
    }
    let mut gone = Vec::new();
    for &pi in taken.keys() {
        for &d in &e.procs[pi].drivers {
            let l = e.nets.drivers[d].leaf;
            let text = match read.get(&l).or(every.as_ref()) {
                Some(&(q, span, what)) => {
                    let how = if what == "a call" {
                        "calls a subprogram that reads the history of".to_string()
                    } else if what == "an implicit signal" {
                        "reads an implicit signal (`'stable`, `'quiet`, `'delayed` or \
                         `'transaction`) of"
                            .to_string()
                    } else {
                        format!("reads `'{what}` of")
                    };
                    (
                        span,
                        format!(
                            "{} {how} {}, and the region keeps no history of the signals it \
                             drives: its drivers stay in the event kernel",
                            e.proc_label(&e.procs[q]),
                            e.leaf_name(l)
                        ),
                    )
                }
                None if implicit.contains(&l) => (
                    db.stmts[e.procs[pi].stmt].span,
                    format!(
                        "an implicit signal (`'stable`, `'quiet`, `'delayed` or `'transaction`) \
                         of {} is read, and the region keeps no history of the signals it drives",
                        e.leaf_name(l)
                    ),
                ),
                None => continue,
            };
            gone.push((
                pi,
                Reason {
                    code: crate::codes::HISTORY_READ,
                    span: text.0,
                    text: text.1,
                },
            ));
            break;
        }
    }
    gone
}

/// The signal history a process reads (amendment A10): leaves whose `'active`, `'last_active`,
/// `'last_event`, `'last_value` it reads, with the attribute's position and name; `None` for the
/// leaves when the prefix is a name the scan does not place (an external name), which stands for
/// every leaf. Calls: a subprogram whose body (or one it calls) reads such an attribute counts
/// as reading it of every signal it is given and every signal its body names. The implicit
/// signals (`'stable`, `'quiet`, `'delayed`, `'transaction`) are in the nets (`history`).
type Hist = (Option<Vec<LeafId>>, yavs_diag::Span, &'static str);

fn hist_attr(a: yavs_sem::attr::PredefAttr) -> Option<&'static str> {
    use yavs_sem::attr::PredefAttr::*;
    Some(match a {
        Active => "active",
        LastActive => "last_active",
        LastEvent => "last_event",
        LastValue => "last_value",
        Transaction => "transaction",
        Stable => "stable",
        Quiet => "quiet",
        Delayed => "delayed",
        _ => return None,
    })
}

/// The object a name starts from, through aliases of objects; `None` for an external name or a
/// prefix of another kind.
fn name_root(db: &Db, mut x: ExprId) -> Option<yavs_sem::DeclId> {
    use yavs_sem::decl::{Alias, DeclKind};
    for _ in 0..64 {
        match &db.exprs[x].kind {
            ExprKind::Field { prefix, .. }
            | ExprKind::Index { prefix, .. }
            | ExprKind::Slice { prefix, .. } => x = *prefix,
            ExprKind::Obj(d) => match &db.decls[*d].kind {
                DeclKind::Alias(Alias::Object { target, .. }) => x = *target,
                _ => return Some(*d),
            },
            _ => return None,
        }
    }
    None
}

/// What a subprogram's body (and what it calls) reads the history of: some formal (then every
/// signal it is given counts), signals it names that are not its formals, or something the scan
/// does not place (`opaque`: an external name, a call nested too deep).
#[derive(Clone, Default)]
struct HistSum {
    formals: bool,
    globals: Vec<yavs_sem::DeclId>,
    opaque: bool,
}

impl HistSum {
    fn any(&self) -> bool {
        self.formals || self.opaque || !self.globals.is_empty()
    }
}

fn is_formal(db: &Db, d: yavs_sem::DeclId) -> bool {
    matches!(&db.decls[d].kind, yavs_sem::decl::DeclKind::Object(o)
        if o.role == yavs_sem::decl::ObjRole::Param)
}

fn is_implicit(db: &Db, d: yavs_sem::DeclId) -> bool {
    matches!(&db.decls[d].kind, yavs_sem::decl::DeclKind::Object(o)
        if o.role == yavs_sem::decl::ObjRole::ImplicitSignal)
}

/// The names of signal objects an actual passes: into `sum` as formals (of the caller, a
/// subprogram), globals, or opaque.
fn actual_names(db: &Db, args: &[yavs_sem::hir::Actual], sum: &mut HistSum) {
    let mut stack = Vec::new();
    for a in args {
        if !push_actual(a, &mut stack) {
            sum.opaque = true;
        }
    }
    for x in stack {
        db.walk_expr(x, &mut |y| match db.exprs[y].kind {
            ExprKind::External(_) => sum.opaque = true,
            ExprKind::Obj(_) => match name_root(db, y) {
                Some(d) if is_formal(db, d) => sum.formals = true,
                Some(d) => sum.globals.push(d),
                None => sum.opaque = true,
            },
            _ => {}
        });
    }
}

fn callee_history(
    db: &Db,
    d: yavs_sem::DeclId,
    depth: u32,
    memo: &mut BTreeMap<yavs_sem::DeclId, HistSum>,
) -> HistSum {
    use yavs_sem::decl::Body as B;
    if let Some(v) = memo.get(&d) {
        return v.clone();
    }
    if depth > 16 {
        return HistSum {
            opaque: true,
            ..HistSum::default()
        };
    }
    memo.insert(d, HistSum::default());
    let Some(sp) = db.decls[d].kind.subprogram() else {
        return HistSum::default();
    };
    // The edge functions of `ieee` read `'last_value` as part of an edge test, which the region
    // gives right (it publishes every change as an event); nothing else of `std` and `ieee`
    // reads history.
    if ["ieee.", "std."]
        .iter()
        .any(|l| sp.mangled.as_str().starts_with(l))
    {
        return HistSum::default();
    }
    let B::Hir { stmts, .. } = &sp.body else {
        return HistSum::default();
    };
    let mut all = Vec::new();
    deep(db, stmts, &mut all);
    let mut sum = HistSum::default();
    let mut xs = Vec::new();
    let mut calls: Vec<(yavs_sem::DeclId, Vec<yavs_sem::hir::Actual>)> = Vec::new();
    for s in all {
        if let StmtKind::ProcCall(c) = &db.stmts[s].kind {
            calls.push((c.callee, c.args.iter().map(|a| a.actual.clone()).collect()));
        }
        db.stmt_exprs(s, &mut |x, _| xs.push(x));
    }
    for x in xs {
        db.walk_expr(x, &mut |y| match &db.exprs[y].kind {
            ExprKind::Attr {
                attr,
                prefix: yavs_sem::hir::AttrPrefix::Expr(pre),
                ..
            } if hist_attr(*attr).is_some() => match name_root(db, *pre) {
                Some(r) if is_formal(db, r) => sum.formals = true,
                Some(r) => sum.globals.push(r),
                None => sum.opaque = true,
            },
            ExprKind::Obj(r) if is_implicit(db, *r) => sum.globals.push(*r),
            ExprKind::Call { callee, args, .. } => {
                calls.push((*callee, args.iter().map(|a| a.actual.clone()).collect()))
            }
            _ => {}
        });
    }
    for (c, args) in calls {
        let inner = callee_history(db, c, depth + 1, memo);
        sum.opaque |= inner.opaque;
        sum.globals.extend(inner.globals);
        if inner.formals {
            actual_names(db, &args, &mut sum);
        }
    }
    memo.insert(d, sum.clone());
    sum
}

/// The history process `pi` reads (see [`Hist`]).
fn history_reads(
    db: &Db,
    e: &Elaborated,
    pi: usize,
    memo: &mut BTreeMap<yavs_sem::DeclId, HistSum>,
) -> Vec<Hist> {
    let p = &e.procs[pi];
    let mut xs: Vec<ExprId> = Vec::new();
    let mut out = Vec::new();
    if let Some(pa) = &p.actual {
        xs.push(pa.expr);
    } else {
        let body = match &db.stmts[p.stmt].kind {
            StmtKind::Process { body, .. } => body.to_vec(),
            _ => vec![p.stmt],
        };
        let mut all = Vec::new();
        deep(db, &body, &mut all);
        for s in all {
            if let StmtKind::ProcCall(c) | StmtKind::ConcProcCall { call: c, .. } =
                &db.stmts[s].kind
            {
                let args: Vec<_> = c.args.iter().map(|a| a.actual.clone()).collect();
                out.extend(call_history(
                    db,
                    e,
                    pi,
                    c.callee,
                    &args,
                    db.stmts[s].span,
                    memo,
                ));
            }
            db.stmt_exprs(s, &mut |x, _| xs.push(x));
        }
    }
    out.extend(expr_history(db, e, pi, &xs, memo));
    out
}

/// The leaves of a signal object seen from process `pi`; an implicit signal's are its sources.
fn sig_leaves(
    e: &Elaborated,
    pi: usize,
    d: yavs_sem::DeclId,
    implicit: bool,
) -> Option<Vec<LeafId>> {
    let o = e.signal(e.procs[pi].scope, d)?;
    let (a, z) = e.nets.obj(o).leaves;
    Some(if implicit {
        (a.0..z.0)
            .flat_map(|l| {
                let leaf = e.nets.leaf(LeafId(l));
                (leaf.implicit.map(|i| i.1).into_iter())
                    .chain(leaf.implicit_more.iter().map(|x| x.0))
            })
            .collect()
    } else {
        (a.0..z.0).map(LeafId).collect()
    })
}

/// What a call of `callee` counts as, if its body reads history: the signals it is given when
/// it reads a formal's, and the signals it names (seen from the caller; a subprogram of a
/// package names only package signals, which are never driven in the region).
fn call_history(
    db: &Db,
    e: &Elaborated,
    pi: usize,
    callee: yavs_sem::DeclId,
    args: &[yavs_sem::hir::Actual],
    span: yavs_diag::Span,
    memo: &mut BTreeMap<yavs_sem::DeclId, HistSum>,
) -> Option<Hist> {
    let sum = callee_history(db, callee, 0, memo);
    if !sum.any() {
        return None;
    }
    let mut names = HistSum {
        globals: sum.globals.clone(),
        opaque: sum.opaque,
        ..HistSum::default()
    };
    if sum.formals {
        actual_names(db, args, &mut names);
    }
    if names.opaque {
        return Some((None, span, "a call"));
    }
    let mut ls = Vec::new();
    for d in names.globals {
        // A name that is no signal (a variable, a constant) reads no history; one the
        // elaboration does not place from here stands for every leaf.
        if !matches!(&db.decls[d].kind, yavs_sem::decl::DeclKind::Object(o)
            if o.class == yavs_sem::decl::ObjClass::Signal)
        {
            continue;
        }
        match sig_leaves(e, pi, d, is_implicit(db, d)) {
            Some(v) => ls.extend(v),
            None => return Some((None, span, "a call")),
        }
    }
    Some((Some(ls), span, "a call"))
}

fn expr_history(
    db: &Db,
    e: &Elaborated,
    pi: usize,
    xs: &[ExprId],
    memo: &mut BTreeMap<yavs_sem::DeclId, HistSum>,
) -> Vec<Hist> {
    let mut out = Vec::new();
    for &x in xs {
        let mut calls = Vec::new();
        db.walk_expr(x, &mut |y| match &db.exprs[y].kind {
            ExprKind::Attr {
                attr,
                prefix: yavs_sem::hir::AttrPrefix::Expr(pre),
                ..
            } if hist_attr(*attr).is_some() => {
                let name = hist_attr(*attr).expect("matched");
                let ls = name_root(db, *pre).and_then(|d| sig_leaves(e, pi, d, false));
                out.push((ls, db.exprs[y].span, name));
            }
            // An implicit signal (`'stable`, `'quiet`, `'delayed`, `'transaction`): its sources.
            ExprKind::Obj(d) if is_implicit(db, *d) => {
                out.push((
                    sig_leaves(e, pi, *d, true),
                    db.exprs[y].span,
                    "an implicit signal",
                ));
            }
            ExprKind::Call { callee, args, .. } => calls.push((
                *callee,
                args.iter().map(|a| a.actual.clone()).collect::<Vec<_>>(),
                db.exprs[y].span,
            )),
            _ => {}
        });
        for (callee, args, span) in calls {
            out.extend(call_history(db, e, pi, callee, &args, span, memo));
        }
    }
    out
}

/// A shared variable of a design unit's region a fitting process names (A15) joins with it only
/// when no other process uses that variable: two processes writing and reading one on the same
/// edge give, in the normal mode, the result of the order they run in. Every other process is
/// scanned, with the subprograms it calls (a call the scan cannot follow counts as a use).
fn shared_rule(db: &Db, e: &Elaborated, fits: &BTreeMap<usize, Fit>) -> Vec<(usize, Reason)> {
    use yavs_sem::decl::{DeclKind, ObjClass};
    // The shared variables a process names directly, by declaring region; false when it calls
    // something the scan cannot follow.
    let names = |pi: usize, out: &mut BTreeSet<(yavs_elab::ScopeId, yavs_sem::DeclId)>| -> bool {
        let p = &e.procs[pi];
        let mut stmts = Vec::new();
        let mut exprs: Vec<ExprId> = Vec::new();
        if let Some(pa) = &p.actual {
            exprs.push(pa.expr);
        }
        deep(db, &[p.stmt], &mut stmts);
        // The process's own declarations: initial values, constraints, aliases.
        if let StmtKind::Process { scope, .. } = &db.stmts[p.stmt].kind {
            decl_exprs(db, *scope, &mut exprs);
        }
        let mut seen = BTreeSet::new();
        let mut ok = true;
        let mut later: Vec<ExprId> = Vec::new();
        loop {
            for x in std::mem::take(&mut exprs) {
                db.walk_expr(x, &mut |y| match &db.exprs[y].kind {
                    ExprKind::Obj(d) => match &db.decls[*d].kind {
                        DeclKind::Object(o) if o.class == ObjClass::SharedVariable => {
                            let mut sc = Some(p.scope);
                            while let Some(x) = sc {
                                if e.regions
                                    .get(&x)
                                    .is_some_and(|r| r.contains(&db.decls[*d].scope))
                                {
                                    out.insert((x, *d));
                                    break;
                                }
                                sc = e.hier.scope(x).parent;
                            }
                        }
                        // An alias names its target.
                        DeclKind::Alias(yavs_sem::decl::Alias::Object { target, .. }) => {
                            later.push(*target)
                        }
                        _ => {}
                    },
                    ExprKind::Call { callee, .. } => {
                        ok &= follow(db, *callee, &mut seen, &mut stmts, &mut later);
                    }
                    ExprKind::External(_) => ok = false,
                    _ => {}
                });
            }
            exprs.append(&mut later);
            if let Some(s) = stmts.pop() {
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
        ok
    };
    let mut mine: BTreeMap<usize, BTreeSet<(yavs_elab::ScopeId, yavs_sem::DeclId)>> =
        BTreeMap::new();
    for &pi in fits.keys() {
        let mut v = BTreeSet::new();
        names(pi, &mut v);
        if !v.is_empty() {
            mine.insert(pi, v);
        }
    }
    if mine.is_empty() {
        return Vec::new();
    }
    let mut users: BTreeMap<(yavs_elab::ScopeId, yavs_sem::DeclId), Vec<usize>> = BTreeMap::new();
    let mut blind: Option<usize> = None;
    for pi in 0..e.procs.len() {
        let mut v = BTreeSet::new();
        if !names(pi, &mut v) && !mine.contains_key(&pi) {
            blind.get_or_insert(pi);
        }
        for k in v {
            users.entry(k).or_default().push(pi);
        }
    }
    let span = |pi: usize| db.stmts[e.procs[pi].stmt].span;
    let mut gone = Vec::new();
    for (&pi, vars) in &mine {
        for k in vars {
            let other = (users[k].iter().copied())
                .find(|&q| q != pi)
                .or(blind.filter(|&q| q != pi));
            if let Some(q) = other {
                gone.push((
                    pi,
                    Reason {
                        code: crate::codes::TESTBENCH,
                        span: span(pi),
                        text: format!(
                            "the shared variable `{}` is also used by {}: written and read on one \
                             edge, the normal mode's result would depend on the order the two run in",
                            db.decls[k.1].name,
                            e.proc_label(&e.procs[q])
                        ),
                    },
                ));
                break;
            }
        }
    }
    gone
}

/// Add the statements of subprogram `f` to `stmts` once (a body of the design); false for one the
/// scan cannot follow (no body here). `ieee` and `std` bodies use no shared variable of the design.
fn follow(
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
fn decl_exprs(db: &Db, scope: yavs_sem::ScopeId, out: &mut Vec<ExprId>) {
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

fn deep(db: &Db, body: &[StmtId], out: &mut Vec<StmtId>) {
    for &s in body {
        out.push(s);
        deep(db, &db.stmt_children(s), out);
    }
}

/// A call of a subprogram declared outside a package, with its actuals.
type LocalCall = (yavs_sem::DeclId, Vec<yavs_sem::hir::Actual>);

/// The calls statement `s` (not its children) makes of subprograms that may read signals they
/// are not given, into `out`; false when the scan cannot follow it (see [`exprs_calls`]).
fn stmt_calls(db: &Db, s: StmtId, out: &mut Vec<LocalCall>) -> bool {
    if let StmtKind::ProcCall(c) | StmtKind::ConcProcCall { call: c, .. } = &db.stmts[s].kind
        && local_subprogram(db, c.callee)
    {
        out.push((c.callee, c.args.iter().map(|a| a.actual.clone()).collect()));
    }
    let mut xs = Vec::new();
    db.stmt_exprs(s, &mut |x, _| xs.push(x));
    exprs_calls(db, &xs, out)
}

/// The calls of subprograms declared outside a package in `xs`, into `out`; false at an actual
/// that is a subtype, a subprogram or a package, or an allocator of a subtype (whose default
/// value may call anything).
fn exprs_calls(db: &Db, xs: &[ExprId], out: &mut Vec<LocalCall>) -> bool {
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
                    out.push((*callee, args.iter().map(|a| a.actual.clone()).collect()));
                }
                stack.extend(method_of);
                for a in args.iter() {
                    if !push_actual(&a.actual, &mut stack) {
                        return false;
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
            ExprKind::New {
                init: yavs_sem::hir::NewInit::Value(v),
            } => stack.push(*v),
            // The default value of an allocated subtype may call anything.
            ExprKind::New { .. } => return false,
            ExprKind::Const(_)
            | ExprKind::Obj(_)
            | ExprKind::External(_)
            | ExprKind::AttrValue { .. }
            | ExprKind::Null
            | ExprKind::Open
            | ExprKind::Error => {}
        }
    }
    true
}

/// What a subprogram declared outside a package (and the local subprograms it calls) reads and
/// waits on by name: the LSPs of the signals its body names that are not its formals, and
/// whether a wait names a formal (then the signals of each call's actuals are waited on).
#[derive(Clone, Default)]
struct CallSigs {
    reads: Vec<yavs_sem::hir::Lsp>,
    waits: Vec<yavs_sem::hir::Lsp>,
    waits_formal: bool,
}

/// [`CallSigs`] of subprogram `d`; `None` when the scan cannot place what it reads: a body that
/// is not HIR (a predefined operation reads nothing), an actual it does not follow, calls nested too deep, a recursion through another
/// subprogram (memo `None` while one is being scanned).
fn callee_sigs(
    db: &Db,
    d: yavs_sem::DeclId,
    depth: u32,
    memo: &mut BTreeMap<yavs_sem::DeclId, Option<CallSigs>>,
) -> Option<CallSigs> {
    use yavs_sem::decl::Body as B;
    if let Some(v) = memo.get(&d) {
        return v.clone();
    }
    if depth > 16 {
        return None;
    }
    memo.insert(d, None);
    let sp = db.decls[d].kind.subprogram()?;
    let (scope, stmts) = match &sp.body {
        B::Hir { scope, stmts } => (*scope, stmts),
        // A predefined operation of a local type reads only its operands.
        B::Implicit(_) => {
            memo.insert(d, Some(CallSigs::default()));
            return Some(CallSigs::default());
        }
        _ => return None,
    };
    let formal = |l: &yavs_sem::hir::Lsp| matches!(l.root, ObjRoot::Decl(r) if is_formal(db, r));
    let mut all = Vec::new();
    deep(db, stmts, &mut all);
    let (mut reads, mut sum, mut calls) = (Vec::new(), CallSigs::default(), Vec::new());
    // What the declarations evaluate at each call (initial values, file names, constraints) and
    // the defaults of the formals.
    let mut xs: Vec<ExprId> = sp.params.iter().filter_map(|p| p.default).collect();
    for &x in &db.scopes[scope].order {
        if let yavs_sem::decl::DeclKind::Object(o) = &db.decls[x].kind {
            xs.extend(o.default);
            if let Some((a, b)) = o.file_open {
                xs.extend(a);
                xs.push(b);
            }
            constraint_exprs(&db.subtypes[o.subtype].constraint, &mut xs);
        }
    }
    for &x in &xs {
        db.reads(x, &mut reads);
    }
    if !exprs_calls(db, &xs, &mut calls) {
        return None;
    }
    for &s in &all {
        db.stmt_reads(s, &mut reads);
        if let StmtKind::Wait { on, until, .. } = &db.stmts[s].kind {
            let mut w = on.to_vec();
            if on.is_empty()
                && let Some(c) = until
            {
                db.reads(*c, &mut w);
            }
            sum.waits_formal |= w.iter().any(formal);
            sum.waits.extend(w.into_iter().filter(|l| !formal(l)));
        }
        if !stmt_calls(db, s, &mut calls) {
            return None;
        }
    }
    sum.reads = reads.into_iter().filter(|l| !formal(l)).collect();
    for (c, args) in calls {
        if c == d {
            continue;
        }
        let inner = callee_sigs(db, c, depth + 1, memo)?;
        sum.reads.extend(inner.reads);
        sum.waits.extend(inner.waits);
        if inner.waits_formal {
            let w = actual_reads(db, &args);
            sum.waits_formal |= w.iter().any(formal);
            sum.waits.extend(w.into_iter().filter(|l| !formal(l)));
        }
    }
    memo.insert(d, Some(sum.clone()));
    Some(sum)
}

/// The bound expressions of a constraint.
fn constraint_exprs(c: &yavs_sem::types::Constraint, out: &mut Vec<ExprId>) {
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

/// The signals the actuals of a call read.
fn actual_reads(db: &Db, args: &[yavs_sem::hir::Actual]) -> Vec<yavs_sem::hir::Lsp> {
    let (mut xs, mut out) = (Vec::new(), Vec::new());
    for a in args {
        push_actual(a, &mut xs);
    }
    for x in xs {
        db.reads(x, &mut out);
    }
    out
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
    in_top_scope(e, e.procs[pi].scope)
}

/// Whether a process of scope `scope` is one of the top-level unit.
pub(crate) fn in_top_scope(e: &Elaborated, scope: yavs_elab::ScopeId) -> bool {
    let mut s = e.hier.scope(scope).parent;
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
