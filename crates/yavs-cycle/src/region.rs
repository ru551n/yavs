//! Which processes join the region, and the region's boundary (design sections 1.1 to 1.3,
//! amended: selection is automatic). A process joins when its body lowers ([`classify`]) and
//! the boundary rules below hold for the set; every other process stays in the event kernel,
//! listed with its reason.

use std::collections::{BTreeMap, BTreeSet};

use yavs_elab::{Case, Elaborated, LeafId, LeafRes, ScopeKind};
use yavs_sem::hir::{ExprKind, ObjRoot, Sensitivity, StmtKind};
use yavs_sem::{Db, ExprId, StmtId};

use crate::lower::{self, Body, Clock, Kind};

/// What happens to one process.
pub(crate) enum Verdict {
    Join(Body),
    Out(String),
}

/// The one membership decision for a single process (milestone 3's checker replaces its body).
pub(crate) fn classify(db: &Db, e: &Elaborated, pi: usize) -> Verdict {
    match lower::lower(db, e, pi) {
        Ok(b) => Verdict::Join(b),
        Err(why) => Verdict::Out(why),
    }
}

/// The region: members (in schedule order: clocked first, then combinational in dependency
/// order) and the boundary.
pub(crate) struct Selection {
    pub members: Vec<(usize, Body)>,
    pub out: Vec<(usize, String)>,
    pub clock: Option<Clock>,
    /// Leaves members drive, and of those the ones something outside reads (exported).
    pub driven: BTreeSet<LeafId>,
    pub outputs: BTreeSet<LeafId>,
    /// Leaves members read that no member drives (read from the kernel at each wake); of
    /// those, the ones a combinational member reads wake the region.
    pub inputs: BTreeSet<LeafId>,
    pub wakes: BTreeSet<LeafId>,
    /// Leaves a clocked member writes and a clocked member reads: written to a next-state
    /// block, committed after all clocked bodies ran (design 6.3).
    pub next: BTreeSet<LeafId>,
}

pub(crate) fn select(db: &Db, e: &Elaborated) -> Selection {
    let n = e.procs.len();
    let mut verdicts: Vec<Verdict> = (0..n).map(|pi| classify(db, e, pi)).collect();
    if !db.facts().forces.is_empty() {
        for v in &mut verdicts {
            if let Verdict::Join(_) = v {
                *v = Verdict::Out("the design uses `force` or `release`".into());
            }
        }
    }
    // Leave out what the set as a whole does not allow, until nothing changes.
    loop {
        let mut out: Vec<(usize, String)> = Vec::new();
        let joined = |v: &Verdict| matches!(v, Verdict::Join(_));
        // Drivers of each leaf, by process.
        let mut drivers: BTreeMap<LeafId, BTreeSet<usize>> = BTreeMap::new();
        for d in &e.nets.drivers {
            drivers.entry(d.leaf).or_default().insert(d.proc.index());
        }
        let consts: BTreeSet<LeafId> = e.nets.consts.iter().map(|c| c.0).collect();
        let mut clock: Option<Clock> = None;
        let mut writer: BTreeMap<LeafId, usize> = BTreeMap::new();
        for (pi, v) in verdicts.iter().enumerate() {
            let Verdict::Join(b) = v else { continue };
            let mut why = None;
            for (&l, ps) in drivers.iter().filter(|(_, ps)| ps.contains(&pi)) {
                if let Some(&q) = ps.iter().find(|&&q| q != pi) {
                    why = Some(format!(
                        "{} is also driven by {}",
                        e.leaf_name(l),
                        e.proc_label(&e.procs[q])
                    ));
                    break;
                }
                let leaf = e.nets.leaf(l);
                if consts.contains(&l)
                    || leaf.guard.is_some()
                    || matches!(leaf.resolution, Some(LeafRes::Func(_) | LeafRes::Whole(_)))
                {
                    why = Some(format!(
                        "{} has a resolution, guard or other source cycle mode does not model yet",
                        e.leaf_name(l)
                    ));
                    break;
                }
            }
            if why.is_none()
                && let Some(c) = b.clock
            {
                let leaf_driver = drivers.get(&c.leaf).into_iter().flatten();
                if leaf_driver.clone().any(|&q| joined(&verdicts[q])) {
                    why = Some(format!(
                        "its clock {} is made inside the region",
                        e.leaf_name(c.leaf)
                    ));
                } else if clock.is_some_and(|k| k != c) {
                    why = Some(
                        "it runs on another clock or edge than the region (several clocks are \
                         not supported yet)"
                            .into(),
                    );
                } else {
                    clock = Some(c);
                }
            }
            match why {
                Some(w) => out.push((pi, w)),
                None => {
                    for &l in &b.writes {
                        writer.insert(l, pi);
                    }
                }
            }
        }
        // Combinational members in dependency order; a cycle (a process reading what it
        // drives included) is a combinational loop and stays outside.
        let comb: Vec<usize> = (0..n)
            .filter(|&pi| !out.iter().any(|o| o.0 == pi))
            .filter(|&pi| matches!(&verdicts[pi], Verdict::Join(b) if b.kind == Kind::Comb))
            .collect();
        let deps = |pi: usize| -> BTreeSet<usize> {
            let Verdict::Join(b) = &verdicts[pi] else {
                return BTreeSet::new();
            };
            (b.reads.iter())
                .filter_map(|l| writer.get(l))
                .copied()
                .filter(|q| comb.contains(q))
                .collect()
        };
        let mut order: Vec<usize> = Vec::new();
        let mut left: Vec<usize> = comb.clone();
        while !left.is_empty() {
            let ready: Vec<usize> = (left.iter().copied())
                .filter(|&pi| deps(pi).iter().all(|q| order.contains(q)))
                .collect();
            if ready.is_empty() {
                break;
            }
            order.extend(&ready);
            left.retain(|pi| !ready.contains(pi));
        }
        for pi in left {
            out.push((
                pi,
                "it is part of a combinational loop (it reads, through other processes or \
                 itself, what it drives)"
                    .into(),
            ));
        }
        if out.is_empty() {
            let mut members = Vec::new();
            let mut outs = Vec::new();
            let mut taken: Vec<Option<Body>> = Vec::new();
            for (pi, v) in verdicts.into_iter().enumerate() {
                match v {
                    Verdict::Join(b) => taken.push(Some(b)),
                    Verdict::Out(w) => {
                        outs.push((pi, w));
                        taken.push(None);
                    }
                }
            }
            for (pi, t) in taken.iter_mut().enumerate() {
                if matches!(t, Some(b) if b.kind == Kind::Clocked) {
                    members.push((pi, t.take().expect("checked")));
                }
            }
            for &pi in &order {
                members.push((pi, taken[pi].take().expect("a comb member")));
            }
            return boundary(db, e, members, outs, clock);
        }
        for (pi, w) in out {
            verdicts[pi] = Verdict::Out(w);
        }
    }
}

fn boundary(
    db: &Db,
    e: &Elaborated,
    members: Vec<(usize, Body)>,
    out: Vec<(usize, String)>,
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
    let outputs = if outside_reads_all(db, e, &is_member) {
        driven.clone()
    } else {
        let seen = outside_reads(db, e, &is_member);
        driven.intersection(&seen).copied().collect()
    };
    Selection {
        members,
        out,
        clock,
        driven,
        outputs,
        inputs,
        wakes,
        next,
    }
}

/// Something may read any signal: an external name anywhere in the design.
fn outside_reads_all(db: &Db, _e: &Elaborated, _members: &BTreeSet<usize>) -> bool {
    !db.externals.is_empty()
}

/// The leaves anything that is not a member may read: the code of the other processes (by the
/// objects their names denote, whole), port edges and implicit signals the kernel updates,
/// guards, and the signals of packages. A call that may read signals it is not given (a
/// subprogram declared in a design unit rather than a package) counts as reading every signal
/// of the design.
fn outside_reads(db: &Db, e: &Elaborated, members: &BTreeSet<usize>) -> BTreeSet<LeafId> {
    let mut seen = BTreeSet::new();
    let all = || (0..e.nets.leaves.len() as u32).map(LeafId).collect();
    for (pi, p) in e.procs.iter().enumerate() {
        if members.contains(&pi) {
            continue;
        }
        let mut lsps = Vec::new();
        let mut calls_local = false;
        if let Some(pa) = &p.actual {
            db.reads(pa.expr, &mut lsps);
            calls_local |= exprs_call_local(db, &[pa.expr]);
        } else {
            match &db.stmts[p.stmt].kind {
                StmtKind::Process {
                    sensitivity, body, ..
                } => {
                    db.reads_deep(body, &mut lsps);
                    if let Sensitivity::List(l) = sensitivity {
                        lsps.extend(l.iter().cloned());
                    }
                    let mut stmts = Vec::new();
                    deep(db, body, &mut stmts);
                    for s in stmts {
                        if let StmtKind::Wait { on, .. } = &db.stmts[s].kind {
                            lsps.extend(on.iter().cloned());
                        }
                        calls_local |= stmt_calls_local(db, s);
                    }
                }
                _ => {
                    db.stmt_reads(p.stmt, &mut lsps);
                    calls_local |= stmt_calls_local(db, p.stmt);
                }
            }
        }
        if calls_local {
            return all();
        }
        for l in lsps {
            match l.root {
                ObjRoot::Decl(d) => match e.signal(p.scope, d) {
                    Some(o) => {
                        let (a, z) = e.nets.obj(o).leaves;
                        seen.extend((a.0..z.0).map(LeafId));
                    }
                    None => return all(),
                },
                ObjRoot::External(_) => return all(),
                ObjRoot::ImplicitSignal(_) => {}
            }
        }
        if let Some(g) = &p.guard {
            seen.extend(g.iter().map(|x| x.0));
        }
    }
    for pe in &e.nets.ports {
        seen.insert(pe.formal.0);
        seen.insert(pe.actual.0);
    }
    for cp in &e.nets.conv_ports {
        seen.extend(cp.formal.iter().chain(&cp.actual).map(|x| x.0));
    }
    for leaf in &e.nets.leaves {
        if let Some((_, src, _)) = leaf.implicit {
            seen.insert(src);
        }
        seen.extend(leaf.implicit_more.iter().map(|x| x.0));
    }
    for o in &e.nets.objs {
        if matches!(e.hier.scope(o.scope).kind, ScopeKind::Package { .. }) {
            seen.extend((o.leaves.0.0..o.leaves.1.0).map(LeafId));
        }
    }
    seen
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

/// A behavioural process (D2): one the testbench side of a design is made of. Milestone 1's
/// reading, from the statements alone: a process without a sensitivity list that waits for a
/// time, waits forever (`wait;`), has more than one wait, calls a procedure (which may wait),
/// or declares a file.
pub(crate) fn behavioural(db: &Db, e: &Elaborated, pi: usize) -> bool {
    let p = &e.procs[pi];
    let StmtKind::Process {
        sensitivity: Sensitivity::None,
        body,
        scope,
        ..
    } = &db.stmts[p.stmt].kind
    else {
        return false;
    };
    let files = db.scopes[*scope].order.iter().any(|&d| {
        matches!(&db.decls[d].kind, yavs_sem::decl::DeclKind::Object(o)
            if o.class == yavs_sem::decl::ObjClass::File)
    });
    let mut stmts = Vec::new();
    deep(db, body, &mut stmts);
    let mut waits = 0;
    for &s in &stmts {
        match &db.stmts[s].kind {
            StmtKind::Wait {
                on, until, timeout, ..
            } => {
                waits += 1;
                if timeout.is_some() || (on.is_empty() && until.is_none()) {
                    return true;
                }
            }
            StmtKind::ProcCall(_) => return true,
            _ => {}
        }
    }
    files || waits != 1
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
