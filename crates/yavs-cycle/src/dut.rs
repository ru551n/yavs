//! The design under test, found for the default scope of `--cycle` and `--two-state` (A64
//! addendum 4): from what the processes do, never from names.
//!
//! A process is of the testbench when it is behavioural (it waits for a time, waits several
//! times, uses files; not a one-shot `wait;`), uses protected objects or access values, or makes
//! a waveform with `after` from nothing or from its own output without a clock (a clock or reset
//! maker); every other process is RTL, a delayed assignment of logic included. A candidate is an instance
//! with RTL whose processes, its own and those below it, are at most 5% of the testbench kind.
//! From the top-level unit down: the candidates among the instances of a level, else the
//! candidates found inside those of its instances that hold RTL (a harness around design and
//! models). The scope is their union; the top-level unit itself when none is found and it holds
//! RTL of its own, its testbench-kind processes left out (A76); empty when the design has no RTL.
//! A process that assigns a `'Z'` literal (a tri-state driver, such as a pad's model) is left
//! out of the scope found too (A78): two-state, its `'Z'` would drive `0` against the bus.

use std::collections::BTreeMap;

use yavs_elab::{Elaborated, ScopeId, ScopeKind};
use yavs_sem::Db;

use crate::check::{self, Verdict};
use yavs_sem::hir::{Rhs, StmtKind, Waveform};

/// The testbench share an instance may hold and still be the design: one in twenty.
const TB_SHARE: usize = 20;

/// Whether process `pi` is of the testbench kind.
fn testbench(db: &Db, e: &Elaborated, pi: usize, callees: &mut check::Callees) -> bool {
    match check::check(db, e, pi, callees) {
        Verdict::Behavioural(w) => w != check::WAITS_FOREVER,
        Verdict::Out(r) if r.code == crate::codes::DELAY => maker(db, e, pi),
        Verdict::Out(r) => r.code == crate::codes::TESTBENCH,
        Verdict::Fit(_) => false,
    }
}

/// A delayed process that reads nothing (`rst <= '1', '0' after 100 ns;`) or, without a clock,
/// its own output (`clk <= not clk after 5 ns;`): it makes a waveform, it computes nothing.
fn maker(db: &Db, e: &Elaborated, pi: usize) -> bool {
    let p = &e.procs[pi];
    let Some((reads, _)) = crate::region::proc_reads(db, e, pi, &mut Default::default()) else {
        return false;
    };
    let own = |l: &yavs_elab::LeafId| p.lsps.iter().flatten().any(|d| d.1 == *l);
    reads.is_empty() || (p.trigger.is_none() && p.edges.is_none() && reads.iter().any(own))
}

/// Whether process `pi` assigns a `'Z'` literal in its own statements (A78).
// ponytail: own statements only; a procedure it calls that drives `'Z'` is not followed.
fn drives_z(db: &Db, e: &Elaborated, pi: usize) -> bool {
    let z = |x: yavs_sem::ExprId| {
        let ex = &db.exprs[x];
        let v = ex.value.map(|c| &db.consts[c]);
        v.is_some_and(|v| check::holds_z(db, x, v))
    };
    let wave = |rhs: &Rhs<Waveform>| {
        let one = |w: &Waveform| match w {
            Waveform::Elements(es) => es.iter().filter_map(|w| w.value).any(z),
            Waveform::Unaffected => false,
        };
        match rhs {
            Rhs::Simple(w) => one(w),
            Rhs::Conditional(c) => c.iter().any(|(w, _)| one(w)),
            Rhs::Selected { alts, .. } => alts.iter().any(|(w, _)| one(w)),
        }
    };
    fn walk(db: &Db, ss: &[yavs_sem::StmtId], f: &dyn Fn(&Rhs<Waveform>) -> bool) -> bool {
        ss.iter().any(|&s| match &db.stmts[s].kind {
            StmtKind::SignalAssign { rhs, .. } => f(rhs),
            StmtKind::If { branches, else_ } => {
                branches.iter().any(|(_, b)| walk(db, b, f))
                    || else_.as_ref().is_some_and(|b| walk(db, b, f))
            }
            StmtKind::Case { alts, .. } => alts.iter().any(|(_, b)| walk(db, b, f)),
            StmtKind::Loop { body, .. } | StmtKind::SeqBlock { body, .. } => walk(db, body, f),
            _ => false,
        })
    }
    match &db.stmts[e.procs[pi].stmt].kind {
        StmtKind::Process { body, .. } => walk(db, body, &wave),
        StmtKind::ConcSignalAssign { rhs, .. } => wave(rhs),
        _ => false,
    }
}

/// The roots of the design under test, and the processes left out of it.
pub fn detect(db: &Db, e: &Elaborated) -> yavs_elab::Detected {
    let h = &e.hier;
    let top = ScopeId(0);
    let inst = |s: ScopeId| matches!(h.scope(s).kind, ScopeKind::Instance { .. });
    // The nearest instance strictly above `s`.
    let up = |s: ScopeId| {
        let mut c = h.scope(s).parent;
        while let Some(x) = c {
            if inst(x) {
                return Some(x);
            }
            c = h.scope(x).parent;
        }
        None
    };
    // Per instance: (testbench, RTL) processes at and below it, and its own RTL.
    let mut count: BTreeMap<ScopeId, (usize, usize)> = BTreeMap::new();
    let (mut own_rtl, mut own_tb) = (0, Vec::new());
    let mut callees = check::Callees::default();
    for pi in 0..e.procs.len() {
        let tb = testbench(db, e, pi, &mut callees);
        let mut s = Some(h.instance_of(e.procs[pi].scope));
        if s == Some(top) {
            match tb {
                true => own_tb.push(pi),
                false => own_rtl += 1,
            }
        }
        while let Some(x) = s {
            let c = count.entry(x).or_default();
            if tb {
                c.0 += 1
            } else {
                c.1 += 1
            }
            s = up(x);
        }
    }
    let mut kids: BTreeMap<ScopeId, Vec<ScopeId>> = BTreeMap::new();
    for i in (0..h.scopes.len()).map(|i| ScopeId(i as u32)) {
        if inst(i)
            && let Some(p) = up(i)
        {
            kids.entry(p).or_default().push(i);
        }
    }
    fn level(
        s: ScopeId,
        kids: &BTreeMap<ScopeId, Vec<ScopeId>>,
        count: &BTreeMap<ScopeId, (usize, usize)>,
    ) -> Vec<ScopeId> {
        let with_rtl: Vec<ScopeId> = (kids.get(&s).into_iter().flatten().copied())
            .filter(|c| count.get(c).is_some_and(|n| n.1 > 0))
            .collect();
        let pure: Vec<ScopeId> = (with_rtl.iter().copied())
            .filter(|c| count.get(c).is_some_and(|&(t, x)| TB_SHARE * t <= t + x))
            .collect();
        if !pure.is_empty() {
            return pure;
        }
        with_rtl
            .into_iter()
            .flat_map(|c| level(c, kids, count))
            .collect()
    }
    let roots = level(top, &kids, &count);
    let tristate = (0..e.procs.len()).filter(|&pi| drives_z(db, e, pi));
    if roots.is_empty() && own_rtl > 0 {
        return yavs_elab::Detected {
            roots: vec![top],
            outside: own_tb.into_iter().chain(tristate).collect(),
        };
    }
    yavs_elab::Detected {
        roots,
        outside: tristate.collect(),
    }
}
