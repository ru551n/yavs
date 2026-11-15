//! Which processes join the region, and the region's boundary. Selection is automatic: a
//! process joins when its body lowers ([`classify`]) and the boundary rules below hold for the
//! set; every other process stays in the event kernel, listed with its reason.

use std::collections::{BTreeMap, BTreeSet};

use yavs_elab::{Case, Elaborated, LeafId, LeafRes, ScopeKind};
use yavs_sem::hir::{ExprKind, ObjRoot, Sensitivity, StmtKind};
use yavs_sem::{Db, ExprId, StmtId};

use crate::lower::{self, Body, Clock, Kind};

/// What happens to one process.
pub(crate) enum Verdict {
    Join(Box<Body>),
    Out(String),
}

/// The one membership decision for a single process (milestone 3's checker replaces its body).
pub(crate) fn classify(db: &Db, e: &Elaborated, pi: usize) -> Verdict {
    match lower::lower(db, e, pi) {
        Ok(b) => Verdict::Join(Box::new(b)),
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
    // Drivers of each leaf: process and element range, by first element.
    let mut drivers: BTreeMap<LeafId, Vec<(usize, u32, u32)>> = BTreeMap::new();
    for d in &e.nets.drivers {
        (drivers.entry(d.leaf).or_default()).push((d.proc.index(), d.range.offset, d.range.len));
    }
    for ds in drivers.values_mut() {
        ds.sort_unstable_by_key(|d| (d.1, d.0));
    }
    // Per (process, leaf): another process driving some of the same elements.
    let mut clash: BTreeMap<(usize, LeafId), usize> = BTreeMap::new();
    for (&l, ds) in &drivers {
        let mut far: Option<(u32, usize)> = None;
        for &(q, lo, n) in ds {
            if let Some((end, p)) = far
                && lo < end
                && p != q
            {
                clash.entry((q, l)).or_insert(p);
                clash.entry((p, l)).or_insert(q);
            }
            if far.is_none_or(|f| lo + n > f.0) {
                far = Some((lo + n, q));
            }
        }
    }
    // Leave out what the set as a whole does not allow, until nothing changes.
    loop {
        let mut out: Vec<(usize, String)> = Vec::new();
        let joined = |v: &Verdict| matches!(v, Verdict::Join(_));
        // Per leaf, a process outside the region that drives part of it.
        let outside: BTreeMap<LeafId, usize> = (drivers.iter())
            .filter_map(|(&l, ds)| {
                let q = ds.iter().find(|d| !joined(&verdicts[d.0]))?;
                Some((l, q.0))
            })
            .collect();
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
        let mut clock: Option<Clock> = None;
        // Per leaf, the element ranges members drive, by member.
        let mut writer: BTreeMap<LeafId, Vec<(u32, u32, usize)>> = BTreeMap::new();
        for (pi, v) in verdicts.iter().enumerate() {
            let Verdict::Join(b) = v else { continue };
            let mut why = None;
            for &di in &e.procs[pi].drivers {
                let l = e.nets.drivers[di].leaf;
                // Another driver of the same elements, or of other elements of the leaf from
                // outside the region (whose elements the region would not read).
                let other = (clash.get(&(pi, l)).copied())
                    .or_else(|| outside.get(&l).copied().filter(|&q| q != pi));
                if let Some(q) = other {
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
                        "{} has a resolution, guard, port or other source cycle mode does not \
                         model yet",
                        e.leaf_name(l)
                    ));
                    break;
                }
            }
            if why.is_none()
                && let Some(c) = b.clock
            {
                let leaf_driver = drivers.get(&c.leaf).into_iter().flatten();
                if leaf_driver.clone().any(|&(q, ..)| joined(&verdicts[q])) {
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
                    for &di in &e.procs[pi].drivers {
                        let d = &e.nets.drivers[di];
                        (writer.entry(d.leaf).or_default()).push((d.range.offset, d.range.len, pi));
                    }
                }
            }
        }
        for w in writer.values_mut() {
            w.sort_unstable();
        }
        // Combinational members in dependency order, by elements; a cycle (a process reading
        // what it drives included) is a combinational loop and stays outside.
        let is_out: BTreeSet<usize> = out.iter().map(|o| o.0).collect();
        let comb: BTreeSet<usize> = (0..n)
            .filter(|pi| !is_out.contains(pi))
            .filter(|&pi| matches!(&verdicts[pi], Verdict::Join(b) if b.kind == Kind::Comb))
            .collect();
        // The combinational members each one reads from, and the reverse.
        let mut deps: BTreeMap<usize, BTreeSet<usize>> = BTreeMap::new();
        let mut users: BTreeMap<usize, Vec<usize>> = BTreeMap::new();
        for &pi in &comb {
            let Verdict::Join(b) = &verdicts[pi] else {
                continue;
            };
            let mut ds = BTreeSet::new();
            for &(l, lo, len) in &b.rparts {
                let Some(ws) = writer.get(&l) else { continue };
                // Ranges of one leaf do not overlap (checked above): the first that can.
                let k = ws.partition_point(|w| w.0 + w.1 <= lo);
                for &(_, _, q) in ws[k..].iter().take_while(|w| w.0 < lo + len) {
                    if comb.contains(&q) {
                        ds.insert(q);
                    }
                }
            }
            for &q in &ds {
                users.entry(q).or_default().push(pi);
            }
            deps.insert(pi, ds);
        }
        let mut need: BTreeMap<usize, usize> = deps.iter().map(|(&p, d)| (p, d.len())).collect();
        let mut ready: BTreeSet<usize> = (need.iter())
            .filter(|(_, n)| **n == 0)
            .map(|(&p, _)| p)
            .collect();
        let mut order: Vec<usize> = Vec::new();
        while let Some(pi) = ready.pop_first() {
            order.push(pi);
            for &u in users.get(&pi).into_iter().flatten() {
                let k = need.get_mut(&u).expect("a comb member");
                *k -= 1;
                if *k == 0 {
                    ready.insert(u);
                }
            }
        }
        let placed: BTreeSet<usize> = order.iter().copied().collect();
        for pi in comb.into_iter().filter(|p| !placed.contains(p)) {
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
                    Verdict::Join(b) => taken.push(Some(*b)),
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

/// A behavioural process: one the testbench side of a design is made of. Milestone 1's
/// reading, from the statements alone: a process that declares a file, or one without a
/// sensitivity list that waits for a time, waits forever (`wait;`), has more than one wait or
/// calls a procedure (which may wait).
pub(crate) fn behavioural(db: &Db, e: &Elaborated, pi: usize) -> bool {
    let p = &e.procs[pi];
    let StmtKind::Process {
        sensitivity,
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
    if files {
        return true;
    }
    if !matches!(sensitivity, Sensitivity::None) {
        return false;
    }
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
    waits != 1
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
