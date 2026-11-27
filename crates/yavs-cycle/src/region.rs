//! Which processes join the region, and the region's boundary. Selection is automatic: the
//! subset checker (`check`) decides which processes fit cycle mode at all, the set rules and the
//! element-level dependence graph (`dep`) decide which of those can run together, and the
//! lowering decides what is implemented yet. Every other process stays in the event kernel,
//! listed with its reason, and the two kinds of reason stay apart: outside the subset (a code)
//! or not implemented yet.

use std::collections::{BTreeMap, BTreeSet};

use yavs_elab::reach::{constraint_exprs, decl_exprs, deep, follow};
use yavs_elab::{Case, Elaborated, LeafId, LeafRes, ScopeKind};
use yavs_sem::hir::{ExprKind, ObjRoot, Sensitivity, StmtKind};
use yavs_sem::{Db, ExprId, StmtId};

use crate::check::{self, Fit, Form, Reason};
use crate::dep::{self, VAR};
use crate::lower::{self, Body, Clock, Kind, Op, Ref};

/// Why a process stays in the event kernel.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) enum Why {
    /// Outside the subset.
    Subset(Reason),
    /// A process of the testbench kind.
    Behavioural(String),
    /// In the subset; cycle mode does not implement something it needs yet, or a rule of the
    /// region as a whole keeps it out.
    NotYet(Reason),
    /// Outside the cycle scope (`--cycle=PATH`, or the design under test found): a process of
    /// the testbench.
    Outside,
}

/// The region: members (clocked first, then combinational in dependency order) and the boundary.
#[derive(serde::Serialize, serde::Deserialize)]
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
/// reads it, if one is known), the member process (its index) with a check its combinational
/// path reaches, or the clocked member whose reset branch it reaches (a pulse between edges loads
/// its registers).
#[derive(Clone, Copy, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) enum Reach {
    Output(LeafId, Option<usize>),
    Check(usize),
    Reset(usize),
}

/// A loop group of the settle: its steps, each member run at the pass levels of what it drives
/// (`dep::Group::runs`), its last run the one on settled values.
#[derive(serde::Serialize, serde::Deserialize)]
pub(crate) struct Loop {
    pub at: std::ops::Range<usize>,
    /// Its members, and the passes of all of them in order that the runs replace (the report).
    pub procs: usize,
    pub k: u32,
}

/// Kernel parts of leaves.
type Parts = Vec<(LeafId, yavs_kernel::Range)>;

/// What fits, why the rest does not, and the loop groups of what fits.
/// The fits, the others with their reasons, the loop groups, and every process the checker
/// fitted (shown to keep no value between runs and to name all it reads in its wait set).
type Subset = (
    BTreeMap<usize, Fit>,
    Vec<(usize, Why)>,
    Vec<dep::Group>,
    BTreeSet<usize>,
);

/// Clock elements that are copies of another: (leaf, element) to its source.
type Copies = BTreeMap<(LeafId, u32), (LeafId, u32)>;

/// Without `--two-state` (A66): the region computes with the normal mode's nine values
/// (`nine.rs`). Off: two-state values (`--two-state`).
static EXACT_META: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(true);

/// `--two-state` (true): a two-state region; without it nine-valued (the default).
pub fn set_two_state(on: bool) {
    EXACT_META.store(!on, std::sync::atomic::Ordering::Relaxed);
}

/// `--wave` (A52): the kernel signals the wave records, sorted; `None` without a wave.
static WAVE: std::sync::Mutex<Option<Vec<u32>>> = std::sync::Mutex::new(None);

/// The kernel signals `--wave` records (sorted), or `None`: the region exports the leaves it
/// drives of those, so the kernel holds their values for the recorder.
pub fn set_wave(sigs: Option<Vec<u32>>) {
    *WAVE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner) = sigs;
}

pub(crate) fn wave() -> Option<Vec<u32>> {
    WAVE.lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .clone()
}

/// The kernel signals `--wave` records that members drive and the region does not export (a
/// memory of integers written at run-time indices), sorted: the recorder leaves them out.
pub(crate) fn wave_held(e: &Elaborated, members: &[usize], outputs: &BTreeSet<LeafId>) -> Vec<u32> {
    let Some(w) = wave() else {
        return Vec::new();
    };
    let k = |l: LeafId| e.nets.loc[l.index()].signal.0;
    let out: BTreeSet<u32> = outputs.iter().map(|&l| k(l)).collect();
    let mem: BTreeSet<usize> = members.iter().copied().collect();
    let driven: BTreeSet<u32> = (e.nets.drivers.iter())
        .filter(|d| mem.contains(&d.proc.index()))
        .map(|d| k(d.leaf))
        .collect();
    (w.into_iter())
        .filter(|s| driven.contains(s) && !out.contains(s))
        .collect()
}

/// Whether the region is nine-valued (no `--two-state`): also, failed checks keep their operands
/// for the message.
pub(crate) fn exact_meta() -> bool {
    EXACT_META.load(std::sync::atomic::Ordering::Relaxed)
}

/// The start of the body of a combinational member with notes (A52): its `fresh` slot is 1 at
/// its first run or when an element of its wait set changed since its last run, the normal
/// mode's event that runs the process; the elements are kept in slots of their own for the next.
fn fresh_ops(e: &Elaborated, b: &mut Body) {
    let Some((f, armed)) = b.fresh else {
        return;
    };
    let nets = &e.nets;
    let mut ops = vec![Op::Copy {
        dst: Ref::Loc(f),
        src: Ref::Loc(armed),
        len: 1,
    }];
    let mut snaps = vec![(armed, 1)];
    for &(l, a, len) in &b.wait {
        let at = nets.loc[l.index()];
        let n = nets.signal_len(at.signal);
        // Logic elements in kernel order; a word per element of others, from the right.
        let (lo, bits, full) = match n.checked_sub(at.base + a + len) {
            Some(_) if nets.leaf(l).ty.elem == yavs_kernel::Elem::Logic => (at.base + a, len, n),
            Some(right) => (64 * right, 64 * len, 64 * n),
            // ponytail: not a part of the signal (no case known): all of it, at worst a note
            // made again on an unchanged wait set.
            None => (0, 64 * n, 64 * n),
        };
        // Nine values: each plane of a logic signal (`nine.rs`).
        let logic = nets.leaf(l).ty.elem == yavs_kernel::Elem::Logic;
        let pw = 64 * crate::codegen::words(n);
        let planes = if exact_meta() && logic { 4 } else { 1 };
        for p in 0..planes {
            let (lo, full) = if planes == 4 {
                (lo + p * pw, 4 * pw)
            } else {
                (lo, full)
            };
            let w = crate::codegen::words(bits);
            let (snap, tmp, ne) = (b.locs, b.locs + w, b.locs + 2 * w);
            b.locs += 2 * w + 1;
            snaps.push((snap, w));
            ops.extend([
                Op::Extract {
                    dst: Ref::Loc(tmp),
                    src: Ref::Sig(at.signal.0),
                    lo,
                    len: bits,
                    full,
                },
                Op::VecEq {
                    dst: Ref::Loc(ne),
                    a: Ref::Loc(tmp),
                    b: Ref::Loc(snap),
                    len: bits,
                    ne: true,
                },
                Op::Gate {
                    g: lower::Gate::Or,
                    dst: Ref::Loc(f),
                    a: Ref::Loc(f),
                    b: Ref::Loc(ne),
                    len: 1,
                },
                Op::Copy {
                    dst: Ref::Loc(snap),
                    src: Ref::Loc(tmp),
                    len: bits,
                },
            ]);
        }
    }
    ops.push(Op::Lit {
        dst: Ref::Loc(armed),
        val: Box::new([0]),
    });
    b.fresh_len = ops.len() as u32;
    ops.append(&mut b.ops);
    b.ops = ops;
    b.snaps = snaps;
}

/// The key of a process's own dependence element for its notes (`analyse`): the process index
/// above every signal leaf and variable key.
const NOTES: u64 = 1 << 40;

/// `--no-asserts` (A54): the asserts and reports of processes below the top-level unit are
/// dropped; off (the default), every member evaluates its own.
static NO_ASSERTS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

/// Drop the asserts and reports below the top-level unit (true) or evaluate them (the default).
pub fn set_no_asserts(on: bool) {
    NO_ASSERTS.store(on, std::sync::atomic::Ordering::Relaxed);
}

/// Whether `--no-asserts` is on.
pub(crate) fn no_asserts() -> bool {
    NO_ASSERTS.load(std::sync::atomic::Ordering::Relaxed)
}

/// Whether the asserts and reports of a process of scope `scope` are evaluated.
pub(crate) fn asserts_kept(e: &Elaborated, scope: yavs_elab::ScopeId) -> bool {
    !NO_ASSERTS.load(std::sync::atomic::Ordering::Relaxed) || in_top_scope(e, scope)
}

/// Whether the region is two-state (A57): `--two-state`. A metavalue literal of region code then
/// takes [`meta_fill`]'s bits.
pub(crate) fn two_state() -> bool {
    !exact_meta()
}

/// What a metavalue literal of a two-state region becomes: `--init-x`, else 0. Only the kind:
/// a `random` seed is a run-time input, never part of the code or its key (A68, `Body::rands`).
pub(crate) fn meta_fill() -> yavs_logic::start::Fill {
    match yavs_logic::start::x() {
        Some(yavs_logic::start::Fill::Random(_)) => yavs_logic::start::Fill::Random(0),
        f => f.unwrap_or(yavs_logic::start::Fill::Zero),
    }
}

/// `YAVS_CYCLE_EXACT_CLOCKS=1` (tests and gates only): a clock copied by assignment stays a
/// clock of its own, as the normal mode's delta order has it (A32).
pub(crate) fn exact_clocks() -> Result<bool, String> {
    let v = std::env::var_os("YAVS_CYCLE_EXACT_CLOCKS").unwrap_or_default();
    match &*v.to_string_lossy() {
        "" => Ok(false),
        "1" => Ok(true),
        v => Err(format!(
            "YAVS_CYCLE_EXACT_CLOCKS takes `1`, not `{v}`{}",
            yavs_diag::suggest::tail(v, "value", ["1"].into_iter())
        )),
    }
}

/// The elements of signals copied unchanged from one signal element each (A32): a concurrent
/// assignment without delay of a name, or of an aggregate of scalar names with static index
/// choices and `others`, to a name with static indices, the only driver of what it assigns; and
/// the formal of an `in` port without conversion. Anything computed (`not clk`, `clk and en`) is
/// no copy.
fn clock_copies(db: &Db, e: &Elaborated) -> Copies {
    use yavs_sem::hir::{Choice, Delay, Rhs, Waveform};
    let mut sourced: BTreeSet<LeafId> = e.nets.consts.iter().map(|c| c.0).collect();
    for pe in &e.nets.ports {
        sourced.insert(pe.formal.0);
        if pe.mode != yavs_kernel::PortMode::In {
            sourced.insert(pe.actual.0);
        }
    }
    for cp in &e.nets.conv_ports {
        sourced.extend(cp.formal.iter().chain(&cp.actual).map(|x| x.0));
    }
    let mut map = Copies::new();
    for (pi, p) in e.procs.iter().enumerate() {
        let StmtKind::ConcSignalAssign {
            postponed: false,
            guarded: false,
            target,
            delay: Delay::Default,
            rhs,
            ..
        } = &db.stmts[p.stmt].kind
        else {
            continue;
        };
        // One waveform, or the first whose condition elaboration knows to hold.
        let w = match rhs {
            Rhs::Simple(w) => Some(w),
            Rhs::Conditional(alts) => (alts.iter())
                .map(|(w, c)| match c {
                    None => Some(Some(w)),
                    Some(c) => match int_of(db, e, pi, *c)? {
                        0 => Some(None),
                        _ => Some(Some(w)),
                    },
                })
                .find(|x| !matches!(x, Some(None)))
                .flatten()
                .flatten(),
            Rhs::Selected { .. } => None,
        };
        let Some(Waveform::Elements(els)) = w else {
            continue;
        };
        let ([el], None) = (&els[..], &p.actual) else {
            continue;
        };
        let (Some(v), None) = (el.value, el.after) else {
            continue;
        };
        let Some(t) = name_elems(db, e, pi, target.expr, &[]) else {
            continue;
        };
        let pairs: Vec<((LeafId, u32), (LeafId, u32))> = match &db.exprs[v].kind {
            ExprKind::Aggregate { assocs, .. } => {
                let mut named: Vec<((LeafId, u32), (LeafId, u32))> = Vec::new();
                let mut others = None;
                let mut ok = true;
                for a in assocs.iter() {
                    let src = match name_elems(db, e, pi, a.value, &[]).as_deref() {
                        Some([x]) if !a.slice_piece => *x,
                        _ => {
                            ok = false;
                            break;
                        }
                    };
                    for c in a.choices.iter() {
                        match c {
                            Choice::Expr(x) => match index_elems(db, e, pi, target.expr, *x) {
                                Some(ts) => named.extend(ts.into_iter().map(|t| (t, src))),
                                None => ok = false,
                            },
                            Choice::Others => others = Some(src),
                            _ => ok = false,
                        }
                    }
                    ok &= !a.choices.is_empty();
                }
                if !ok {
                    continue;
                }
                let mut all: Vec<_> = named.clone();
                if let Some(src) = others {
                    all.extend(
                        (t.iter())
                            .filter(|x| !named.iter().any(|n| n.0 == **x))
                            .map(|&x| (x, src)),
                    );
                }
                all
            }
            _ => match name_elems(db, e, pi, v, &[]) {
                Some(s) if s.len() == t.len() => t.iter().copied().zip(s).collect(),
                _ => continue,
            },
        };
        // Every element of the target exactly once, driven by this process alone.
        if pairs.len() != t.len()
            || pairs.iter().any(|(x, _)| {
                sourced.contains(&x.0)
                    || (e.nets.drivers.iter()).any(|d| {
                        d.leaf == x.0
                            && d.range.offset <= x.1
                            && x.1 < d.range.end()
                            && d.proc.index() != pi
                    })
            })
        {
            continue;
        }
        map.extend(pairs);
    }
    for pe in &e.nets.ports {
        let (fl, fr) = pe.formal;
        let (al, ar) = pe.actual;
        if pe.mode == yavs_kernel::PortMode::In
            && pe.to_formal.is_none()
            && pe.to_actual.is_none()
            && fr.len == ar.len
        {
            for j in 0..fr.len {
                map.insert((fl, fr.offset + j), (al, ar.offset + j));
            }
        }
    }
    map
}

/// The signal elements name `x` of process `pi` names, with selectors `more` after it: a signal
/// with indices elaboration knows, in a layout of its own (not a port collapsed into another's).
fn name_elems(
    db: &Db,
    e: &Elaborated,
    pi: usize,
    x: ExprId,
    more: &[yavs_elab::Sel],
) -> Option<Vec<(LeafId, u32)>> {
    let mut sels: Vec<yavs_elab::Sel> = Vec::new();
    let mut at = x;
    loop {
        match &db.exprs[at].kind {
            ExprKind::Obj(d) => {
                let o = e.signal(e.procs[pi].scope, *d)?;
                let obj = e.nets.obj(o);
                if !obj.parts.is_empty()
                    || (obj.leaves.0 != obj.leaves.1 && e.nets.leaf(obj.leaves.0).obj != o)
                {
                    // A port collapsed into its actual: whole, its leaves or parts of them.
                    if !sels.is_empty() || !more.is_empty() {
                        return None;
                    }
                    return Some(
                        (obj.leaves.0.0..obj.leaves.1.0)
                            .enumerate()
                            .flat_map(|(k, l)| {
                                let l = LeafId(l);
                                let r = obj
                                    .part(k)
                                    .unwrap_or(yavs_kernel::Range::new(0, e.nets.leaf(l).ty.len));
                                (r.offset..r.end()).map(move |j| (l, j))
                            })
                            .collect(),
                    );
                }
                sels.reverse();
                sels.extend_from_slice(more);
                let (ps, _) = e.nets.select(o, &sels).ok()?;
                return Some(
                    (ps.iter())
                        .flat_map(|(l, r)| (r.offset..r.end()).map(move |j| (*l, j)))
                        .collect(),
                );
            }
            ExprKind::Index { prefix, indices } => {
                let v = (indices.iter())
                    .map(|&i| int_of(db, e, pi, i))
                    .collect::<Option<Vec<i64>>>()?;
                sels.push(yavs_elab::Sel::Index(v));
                at = *prefix;
            }
            _ => return None,
        }
    }
}

/// The elements of `target(c)`, `c` a choice elaboration knows.
fn index_elems(
    db: &Db,
    e: &Elaborated,
    pi: usize,
    target: ExprId,
    c: ExprId,
) -> Option<Vec<(LeafId, u32)>> {
    name_elems(
        db,
        e,
        pi,
        target,
        &[yavs_elab::Sel::Index(vec![int_of(db, e, pi, c)?])],
    )
}

fn int_of(db: &Db, e: &Elaborated, pi: usize, x: ExprId) -> Option<i64> {
    check::const_value(
        db,
        e,
        &e.procs[pi],
        Default::default(),
        &mut Default::default(),
        x,
    )?
    .as_int()
}

/// The start value of element `i` of leaf `l` as the analysis may use it: none (unknown) where
/// `--init-unset=random` drew it, so the cached analysis holds for every seed (A68).
fn start_char(e: &Elaborated, l: LeafId, i: u32) -> Option<char> {
    if e.drawn.contains(&l) {
        return None;
    }
    (e.nets.leaf(l).init.as_logic()).map(|v| v.get(i as usize).to_char())
}

/// The copy chain of clock `c` (`c` first): each a copy of the next, ending where a copy's start
/// value would give it an edge at time 0 that its source does not have (`'event and c = '1'`
/// sees one from any other start, `rising_edge` only from `'0'` or `'L'`), and before a source
/// a member drives (clocks come from outside the region).
fn copy_chain(
    e: &Elaborated,
    copies: &Copies,
    made: &BTreeSet<LeafId>,
    c: Clock,
) -> Vec<(LeafId, u32)> {
    let init = |(l, i): (LeafId, u32)| start_char(e, l, i);
    let (on, off) = if c.rising {
        (['1', 'H'], ['0', 'L'])
    } else {
        (['0', 'L'], ['1', 'H'])
    };
    let mut chain = vec![(c.leaf, c.elem)];
    while chain.len() < 64 {
        let at = chain[chain.len() - 1];
        let Some(&src) = copies.get(&at) else { break };
        let spurious = match (init(at), init(src)) {
            (Some(a), Some(b)) if c.legacy => a != b && b == on[0],
            (Some(a), Some(b)) => off.contains(&a) && on.contains(&b),
            _ => true,
        };
        if spurious || made.contains(&src.0) || chain.contains(&src) {
            break;
        }
        chain.push(src);
    }
    chain
}

/// The clocks of `taken` as their sources (A32): the last element of each copy chain whose
/// kernel signal keeps the event history an edge test needs; where none above the clock does,
/// copies of one source at the same distance from it (they change in the same delta) share the
/// first of them.
fn same_clocks(
    e: &Elaborated,
    copies: &Copies,
    taken: &mut BTreeMap<usize, (Fit, Body)>,
) -> BTreeMap<usize, (Clock, Vec<LeafId>)> {
    type El = (LeafId, u32);
    // Copies of one source at one distance, with one start value.
    type Sib = (El, usize, Option<char>);
    let made: BTreeSet<LeafId> = (taken.keys())
        .flat_map(|&pi| e.procs[pi].drivers.iter().map(|&d| e.nets.drivers[d].leaf))
        .collect();
    let history = |x: &El| e.bare.get(x.0.index()) != Some(&true);
    let init = |x: El| start_char(e, x.0, x.1);
    let mut to: BTreeMap<usize, (Result<El, Sib>, Vec<El>)> = BTreeMap::new();
    for (&pi, (_, b)) in taken.iter() {
        let Some(c) = b.clock else { continue };
        let chain = copy_chain(e, copies, &made, c);
        if chain.len() < 2 {
            continue;
        }
        let t = match (1..chain.len()).rev().find(|&k| history(&chain[k])) {
            Some(k) => Ok(chain[k]),
            None => Err((chain[chain.len() - 1], chain.len(), init(chain[0]))),
        };
        to.insert(pi, (t, chain));
    }
    let mut first: BTreeMap<Sib, El> = BTreeMap::new();
    for (pi, (t, _)) in &to {
        if let (Err(k), Some(c)) = (t, taken[pi].1.clock) {
            let x = first.entry(*k).or_insert((c.leaf, c.elem));
            *x = (*x).min((c.leaf, c.elem));
        }
    }
    let mut moved = BTreeMap::new();
    for (pi, (t, chain)) in to {
        let b = &mut taken.get_mut(&pi).expect("listed").1;
        if let Some(c) = b.clock {
            // The copies between the clock and where it moved: their edges come later. A
            // sibling's edges come in the same delta.
            let later = match t {
                Ok(to) => chain
                    .iter()
                    .take_while(|x| **x != to)
                    .map(|x| x.0)
                    .collect(),
                Err(_) => Vec::new(),
            };
            let (leaf, elem) = t.unwrap_or_else(|k| first[&k]);
            b.clock = Some(Clock { leaf, elem, ..c });
            moved.insert(pi, (c, later));
        }
    }
    moved
}

/// Members moved onto a copied clock's source (`same_clocks`) whose copy a process outside
/// waits on (a register of the kernel on the copy), or whose signals a member on a copy of that
/// source that did not move reads (one sent back, or one whose copy keeps its own clock): that
/// register samples in the copy's delta, after the moved member changed, so the member goes back
/// to its own clock.
fn copy_rule(
    db: &Db,
    e: &Elaborated,
    copies: &Copies,
    taken: &mut BTreeMap<usize, (Fit, Body)>,
    moved: &mut BTreeMap<usize, (Clock, Vec<LeafId>)>,
) -> bool {
    moved.retain(|pi, _| taken.contains_key(pi));
    if moved.is_empty() {
        return false;
    }
    let mut memo = BTreeMap::new();
    let mut waited: Option<BTreeSet<LeafId>> = Some(BTreeSet::new());
    for q in (0..e.procs.len()).filter(|q| !taken.contains_key(q)) {
        match (proc_reads(db, e, q, &mut memo), waited.as_mut()) {
            (Some((_, w)), Some(all)) => all.extend(w),
            _ => waited = None,
        }
    }
    let mut back: BTreeSet<usize> = (moved.iter())
        .filter(|(_, (_, ls))| {
            ls.iter()
                .any(|l| waited.as_ref().is_none_or(|w| w.contains(l)))
        })
        .map(|(&pi, _)| pi)
        .collect();
    // A member on a later copy of a moved member's source that reads what the moved member
    // drives (found by the fuzzer under the two-state contract, `clkcopy.vhd` `smp`).
    let made: BTreeSet<LeafId> = (taken.keys())
        .flat_map(|&pi| e.procs[pi].drivers.iter().map(|&d| e.nets.drivers[d].leaf))
        .collect();
    let drives = |pi: usize| -> BTreeSet<LeafId> {
        (e.procs[pi].drivers.iter())
            .map(|&d| e.nets.drivers[d].leaf)
            .collect()
    };
    for (&r, (f, b)) in taken.iter() {
        // One sent back samples on its own clock.
        let c = match moved.get(&r) {
            Some((own, _)) if back.contains(&r) => *own,
            Some(_) => continue,
            None => match b.clock {
                Some(c) => c,
                None => continue,
            },
        };
        let chain = copy_chain(e, copies, &made, c);
        let reads: BTreeSet<LeafId> = (b.reads.iter().copied())
            .chain(f.reads.iter().map(|(k, _, _)| LeafId(k as u32)))
            .collect();
        for &m in moved.keys() {
            let to = taken[&m].1.clock.map(|x| (x.leaf, x.elem));
            if m != r
                && !back.contains(&m)
                && chain[1..].iter().any(|x| Some(*x) == to)
                && drives(m).iter().any(|l| reads.contains(l))
            {
                back.insert(m);
            }
        }
    }
    for pi in &back {
        let (c, _) = moved.remove(pi).expect("listed");
        taken.get_mut(pi).expect("member").1.clock = Some(c);
    }
    !back.is_empty()
}

/// The subset verdict of every process: what fits, and why the others do not.
pub(crate) fn subset(db: &Db, e: &Elaborated) -> Subset {
    let t0 = std::time::Instant::now();
    let mut fits: BTreeMap<usize, Fit> = BTreeMap::new();
    let mut out: Vec<(usize, Why)> = Vec::new();
    let mut checked = BTreeSet::new();
    let mut callees = check::Callees::default();
    let mut slow: Vec<(u128, usize)> = Vec::new();
    for pi in 0..e.procs.len() {
        let t = std::time::Instant::now();
        let v = check::check(db, e, pi, &mut callees);
        slow.push((t.elapsed().as_micros(), pi));
        // Outside the cycle scope a process that fits never joins; the others keep their
        // reason, which what follows paths through logic reads.
        match v {
            check::Verdict::Fit(_) if outside_scope(e, pi) => {
                checked.insert(pi);
                out.push((pi, Why::Outside));
            }
            check::Verdict::Fit(f) => {
                fits.insert(pi, f);
                checked.insert(pi);
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
                let (text, also) = if let Some(&q) = other.get(&d) {
                    let who = e.proc_label(&e.procs[q]);
                    (
                        format!(
                            "a signal with two drivers: {} is driven by {who} too",
                            e.leaf_name(l)
                        ),
                        Some((span(q), format!("{who} also drives {}", e.leaf_name(l)))),
                    )
                } else if leaf.guard.is_some() {
                    (format!("a guarded signal: {}", e.leaf_name(l)), None)
                } else if matches!(leaf.resolution, Some(LeafRes::Func(_) | LeafRes::Whole(_))) {
                    let t = format!(
                        "a signal with a resolution function of its own: {}",
                        e.leaf_name(l)
                    );
                    (t, None)
                } else {
                    continue;
                };
                let mut r = Reason::new(crate::codes::SHARED, span(pi), text);
                r.also = also;
                return Some((pi, r));
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
                    let (who, other, text) = if m.form == Form::Comb {
                        (
                            q,
                            *pi,
                            format!(
                                "a clock made inside the design: this logic makes {}, the {what} of {}",
                                e.leaf_name(l),
                                e.proc_label(&e.procs[*pi])
                            ),
                        )
                    } else {
                        (
                            *pi,
                            q,
                            format!(
                                "a clock made inside the design: the {what} {} comes from the register {}",
                                e.leaf_name(l),
                                e.proc_label(&e.procs[q])
                            ),
                        )
                    };
                    if !made.iter().any(|m| m.0 == who) {
                        let also = if other == q {
                            format!("{} makes {}", e.proc_label(&e.procs[q]), e.leaf_name(l))
                        } else {
                            format!(
                                "{} is clocked by {}",
                                e.proc_label(&e.procs[other]),
                                e.leaf_name(l)
                            )
                        };
                        made.push((
                            who,
                            Reason::new(crate::codes::CLOCK_MADE, span(who), text)
                                .also(span(other), also),
                        ));
                    }
                }
            }
        }
        // A loop element by element in the combinational logic.
        stat(|| format!("subset round: {} ms", t0.elapsed().as_millis()));
        let a = analyse(db, e, fits.iter().map(|(pi, f)| (*pi, f)), &BTreeSet::new());
        stat(|| format!("subset analyse: {} ms", t0.elapsed().as_millis()));
        for (ps, path) in &a.cycles {
            for &q in ps {
                if !made.iter().any(|m| m.0 == q) {
                    made.push((
                        q,
                        Reason::new(
                            crate::codes::LOOP,
                            span(q),
                            format!("a combinational loop: {path}"),
                        ),
                    ));
                }
            }
        }
        if made.is_empty() {
            return (fits, out, a.groups, checked);
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
            "an internal mismatch in yavs (please report it): the two passes of yavs read the \
             process's shape, clock or reset differently"
                .into(),
        );
    }
    let (l, x) = (b.rparts.iter()).find_map(|&(l, lo, n)| {
        f.reads
            .first_missing(u64::from(l.0), lo, lo + n)
            .map(|x| (l, x))
    })?;
    // Reached until 2026-11-18 by a call with a mode view, whose elements the checker did not count.
    Some(format!(
        "an internal mismatch in yavs (please report it): a read of element {x} of {} that the \
         first pass of yavs missed",
        e.leaf_name(l)
    ))
}

/// The dependence analysis of the fitting processes. A combinational member of `noted` runs its
/// notes on settled values: it also drives an element of its own ([`NOTES`]) that depends on all
/// it reads, so it runs after what makes any of that.
fn analyse<'f>(
    db: &Db,
    e: &Elaborated,
    fits: impl Iterator<Item = (usize, &'f Fit)>,
    noted: &BTreeSet<usize>,
) -> dep::Analysis {
    let _ = db;
    let n = e.procs.len();
    let mut procs: Vec<(usize, Vec<dep::Out>)> = fits
        .filter(|(_, f)| !f.outs.is_empty() || f.form == Form::Comb)
        .map(|(pi, f)| {
            let mut outs = f.outs.clone();
            if noted.contains(&pi) {
                let mut reads = dep::ESet::default();
                (f.reads.iter())
                    .filter(|r| r.0 & VAR == 0)
                    .for_each(|(k, a, z)| reads.add(k, a, z));
                outs.push(dep::Out {
                    leaf: NOTES | pi as u64,
                    lo: 0,
                    hi: 1,
                    dep: dep::Dep::set(std::rc::Rc::new(reads)),
                });
            }
            (pi, outs)
        })
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
        if k & NOTES != 0 {
            return "the asserts and reports of a process".into();
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
    let (fits, mut out, groups, checked) = subset(db, e);
    let forced = forced(db, e);
    let mut copies = if exact_clocks().unwrap_or(true) {
        Copies::new()
    } else {
        clock_copies(db, e)
    };
    // A forced element is no copy of its source (A32).
    copies.retain(|k, _| !forced.has(k.0));
    stat(|| format!("select: subset done {} ms", t0.elapsed().as_millis()));
    let mut nfits = fits.len();
    // What each driven leaf of a combinational process depends on, for the run-count cone.
    let comb_deps: BTreeMap<usize, Vec<crate::dep::Out>> = (fits.iter())
        .filter(|(_, f)| f.form == check::Form::Comb)
        .map(|(&pi, f)| (pi, f.outs.clone()))
        .collect();
    let mut taken: BTreeMap<usize, (Fit, Body)> = BTreeMap::new();
    let pspan = |pi: usize| db.stmts[e.procs[pi].stmt].span;
    for (pi, f) in fits {
        if let Some(r) = forced.reason(db, e, pi, pspan(pi)) {
            out.push((pi, Why::NotYet(r)));
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
            out.push((
                pi,
                Why::NotYet(Reason::new(
                    crate::codes::LEFT_OUT,
                    pspan(pi),
                    "a process excluded by the developer variable YAVS_CYCLE_EXCLUDE",
                )),
            ));
            continue;
        }
        match lower::lower(db, e, pi) {
            // The lowering must see the shape the checker proved, clock included.
            Ok(b) if let Some(w) = differs(e, &f, &b) => out.push((
                pi,
                Why::NotYet(Reason::new(crate::codes::LEFT_OUT, pspan(pi), w)),
            )),
            // Asserts and reports are evaluated (A19, A52): at a clock edge or after the settle
            // in region code, else the process stays in the event kernel.
            Ok(b) if b.note_why.is_some() => {
                let w = b.note_why.as_deref().unwrap_or_default();
                if in_top_unit(e, pi) {
                    nfits -= 1;
                    out.push((
                        pi,
                        Why::Behavioural(format!(
                            "it holds an assert or report of the top-level unit that cycle mode \
                             evaluates only in the event kernel: {w}"
                        )),
                    ));
                } else {
                    let at = b.drops.first().map_or(pspan(pi), |d| d.0);
                    out.push((
                        pi,
                        Why::NotYet(Reason::new(
                            crate::codes::ASSERT,
                            at,
                            format!("an assert or report of this form: {w}"),
                        )),
                    ));
                }
            }
            Ok(mut b) => {
                // A combinational member with notes runs them when what it waits on changed
                // (`Body::fresh`): the region must read all of it.
                let wait: Vec<(LeafId, u32, u32)> = (f.wait.iter())
                    .filter(|_| b.fresh.is_some())
                    .map(|(k, a, z)| (LeafId(k as u32), a, z - a))
                    .collect();
                if let Some(&(l, ..)) = wait.iter().find(|w| !b.reads.contains(&w.0)) {
                    out.push((
                        pi,
                        Why::NotYet(Reason::new(
                            crate::codes::ASSERT,
                            pspan(pi),
                            format!(
                                "an assert or report in a process whose sensitivity list names \
                                 more than it reads: it does not read {}",
                                e.leaf_name(l)
                            ),
                        )),
                    ));
                    continue;
                }
                b.wait = wait;
                fresh_ops(e, &mut b);
                taken.insert(pi, (f, b));
            }
            Err(w) => out.push((pi, Why::NotYet(lower::reason(w, pspan(pi))))),
        }
    }
    stat(|| format!("select: lowered {} ms", t0.elapsed().as_millis()));
    // A copied clock is its source (A32): one edge class, in the source's delta. The copy
    // itself stays exempt where a rule exempts a member's own clock.
    let own_clock: BTreeMap<usize, LeafId> = (taken.iter())
        .filter_map(|(&pi, (_, b))| Some((pi, b.clock?.leaf)))
        .collect();
    let mut moved = same_clocks(e, &copies, &mut taken);
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
    // Combinational logic feeding a process outside whose result depends on how often it runs.
    let seeds = delta_seeds(db, e, &out, &checked);
    // Developers (`YAVS_CYCLE_SEEDS=FILE`): the signals those processes drive, one leaf a line,
    // for a trace of both modes (`YAVS_TRACE`, tools/difftest/seeddiff.py).
    if let Some(f) = std::env::var_os("YAVS_CYCLE_SEEDS") {
        let mut names = BTreeSet::new();
        for &(pi, _) in &seeds {
            for &d in &e.procs[pi].drivers {
                names.insert(e.leaf_name(e.nets.drivers[d].leaf));
            }
        }
        let text: String = names.into_iter().map(|n| n + "\n").collect();
        if let Err(x) = std::fs::write(&f, text) {
            eprintln!("YAVS_CYCLE_SEEDS: {}: {x}", f.to_string_lossy());
        }
    }
    // Developers (`YAVS_CYCLE_RUNCOUNT=off`): no cone, to show what the rule protects (wrong
    // results) and what it costs.
    let off = std::env::var_os("YAVS_CYCLE_RUNCOUNT").is_some_and(|v| v == "off");
    // Leaves whose transactions someone may see (`'active`, `'transaction`, ...): a kernel
    // process of the cone must run in the normal mode's deltas, so its walk takes all it reads.
    let seen_tx: Option<BTreeSet<LeafId>> = (hist.iter().flatten())
        .map(|h| h.0.clone())
        .collect::<Option<Vec<Vec<LeafId>>>>()
        .map(|v| {
            v.into_iter()
                .flatten()
                .chain(implicit.iter().copied())
                .collect()
        });
    let cone = if off { &[][..] } else { &seeds[..] };
    for (pi, r) in run_count_cone(db, e, &out, cone, &comb_deps, seen_tx.as_ref()) {
        if taken.remove(&pi).is_some() {
            out.push((pi, Why::NotYet(r)));
        }
    }
    stat(|| format!("select: history {} ms", t0.elapsed().as_millis()));
    // Leave out what the set as a whole does not allow yet, until nothing changes.
    loop {
        stat(|| format!("select: round at {} ms", t0.elapsed().as_millis()));
        let mut gone: Vec<(usize, Reason)> = Vec::new();
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
                    Reason::new(
                        crate::codes::PORT_SOURCE,
                        pspan(pi),
                        format!(
                            "a signal with a port or constant source: the process drives {}, which also gets a \
                             value from a port connection or a constant",
                            e.leaf_name(l)
                        ),
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
                    Reason::new(
                        crate::codes::SPLIT,
                        pspan(pi),
                        format!(
                            "a signal driven partly outside the region: other elements of {} come from {}",
                            e.leaf_name(l),
                            e.proc_label(&e.procs[q])
                        ),
                    )
                    .also(
                        pspan(q),
                        format!("{} drives other elements of {}", e.proc_label(&e.procs[q]), e.leaf_name(l)),
                    ),
                ));
                continue;
            }
            // A checker of the top-level unit evaluated in region code (A19, whole-design scope)
            // reads what members drive only: it sees the values the normal mode's checker would
            // see of them, never a metavalue of the kernel read as 0. What the checker counts too:
            // the lowering reads nothing of a value it folds (`is_x` is false on two-state values).
            let fit_reads = f.reads.iter().map(|(k, _, _)| LeafId(k as u32));
            let top = in_top_unit(e, pi);
            if b.notes > 0
                && top
                && let Some(l) = (b.reads.iter().copied()).chain(fit_reads).find(|&l| {
                    b.clock.is_none_or(|c| c.leaf != l)
                        && own_clock.get(&pi).is_none_or(|&c| c != l)
                        && (outside.contains_key(&l) || !inside.contains(&l) || consts.contains(&l))
                })
            {
                let (code, what) = (
                    crate::codes::CHECK_OUTSIDE,
                    "an assert reading a signal the region does not drive alone",
                );
                gone.push((
                    pi,
                    Reason::new(code, pspan(pi), format!("{what}: {}", e.leaf_name(l))),
                ));
                continue;
            }
            // The reset branch runs again in the settle, apart from the edge test.
            // The lowering gives every member with a reset that shape (`Lx::process`).
            if b.reset.is_some() && reset_piece(&b.ops).is_none() {
                gone.push((
                    pi,
                    Reason::new(
                        crate::codes::INTERNAL,
                        pspan(pi),
                        lower::bug("an asynchronous reset branch of another shape"),
                    ),
                ));
            }
        }
        stat(|| format!("select: sources {} ms", t0.elapsed().as_millis()));
        if gone.is_empty() && copy_rule(db, e, &copies, &mut taken, &mut moved) {
            continue;
        }
        if gone.is_empty() {
            gone = ram_rule(db, e, &taken);
        }
        stat(|| format!("select: ram {} ms", t0.elapsed().as_millis()));
        if gone.is_empty() {
            gone = pulse_rule(db, e, &taken);
        }
        if gone.is_empty() {
            gone = cross_delta(db, e, &taken, &out, &forced);
        }
        stat(|| format!("select: cross {} ms", t0.elapsed().as_millis()));
        if gone.is_empty() {
            // The order and the loop groups of what is left: a group runs as a loop of passes.
            let noted: BTreeSet<usize> = (taken.iter())
                .filter(|(_, (_, b))| b.fresh.is_some())
                .map(|(pi, _)| *pi)
                .collect();
            let a = analyse(db, e, taken.iter().map(|(pi, (f, _))| (*pi, f)), &noted);
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
        for (pi, r) in gone {
            if taken.remove(&pi).is_some() {
                out.push((pi, Why::NotYet(r)));
            }
        }
    }
}

/// Logic leaves that nothing drives (no driver, no port or other source) whose start value holds
/// a metavalue: the normal mode reads that metavalue for the whole run.
pub(crate) fn undriven_metas(e: &Elaborated) -> BTreeSet<LeafId> {
    let mut sourced: BTreeSet<LeafId> = e.nets.drivers.iter().map(|d| d.leaf).collect();
    sourced.extend(e.nets.consts.iter().map(|c| c.0));
    for pe in &e.nets.ports {
        sourced.insert(pe.formal.0);
        if pe.mode != yavs_kernel::PortMode::In {
            sourced.insert(pe.actual.0);
        }
    }
    for cp in &e.nets.conv_ports {
        sourced.extend(cp.formal.iter().map(|x| x.0));
        if cp.mode != yavs_kernel::PortMode::In {
            sourced.extend(cp.actual.iter().map(|x| x.0));
        }
    }
    (e.nets.leaves.iter().enumerate())
        .filter_map(|(k, l)| {
            let id = LeafId(k as u32);
            let v = l.init.as_logic()?;
            (l.implicit.is_none()
                && !sourced.contains(&id)
                && (0..v.len()).any(|i| v.get(i).is_x()))
            .then_some(id)
        })
        .collect()
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

/// The delta class of a clock with its edges in the first delta of their time step (every
/// assignment to it made with a delay above zero) and of one with its edges in the second (made
/// by clock models that wait for a time above zero and assign without delay).
const DELTA0: usize = usize::MAX;
const DELTA1: usize = usize::MAX - 1;

/// When the edges of `c`'s element come within a time step: in a delta class (`DELTA0`,
/// `DELTA1`) when every driver's process makes them there, else the clock's element alone.
fn delta_key(db: &Db, e: &Elaborated, c: &Clock, forced: &Forced) -> (bool, usize, u32) {
    // A force gives a clock edges in deltas its drivers do not tell.
    if forced.has(c.leaf) {
        return (false, c.leaf.index(), c.elem);
    }
    let ps: BTreeSet<usize> = (e.nets.drivers.iter())
        .filter(|d| d.leaf == c.leaf && d.range.offset <= c.elem && c.elem < d.range.end())
        .map(|d| d.proc.index())
        .collect();
    if !ps.is_empty() && ps.iter().all(|&p| delayed_only(db, e, p)) {
        return (true, DELTA0, 0);
    }
    if let [p] = ps.iter().copied().collect::<Vec<_>>()[..]
        && let Some(timeout) = one_step(db, e, p)
    {
        // A wait of 0 ns would resume it in a later delta: then only its own clocks share it.
        let k = if positive_time(db, e, p, timeout) {
            DELTA1
        } else {
            p
        };
        return (true, k, 0);
    }
    (false, c.leaf.index(), c.elem)
}

/// The delta of its time step a clock of key `k` (`delta_key`) has its edges in: at the earliest,
/// and exactly when known. A clock process whose timeout elaboration does not know resumes in the
/// first delta (above zero) or later (zero), so its edges come in the second or later.
fn first_delta(k: (bool, usize, u32)) -> (u8, Option<u8>) {
    match k {
        (true, DELTA0, _) => (0, Some(0)),
        (true, DELTA1, _) => (1, Some(1)),
        (true, _, _) => (1, None),
        _ => (0, None),
    }
}

/// The time of `x` when elaboration knows it is above zero.
fn positive_time(db: &Db, e: &Elaborated, pi: usize, x: ExprId) -> bool {
    check::const_value(
        db,
        e,
        &e.procs[pi],
        Default::default(),
        &mut Default::default(),
        x,
    )
    .is_some_and(|v| matches!(v, yavs_sem::stat::ConstValue::Phys(t) if t > 0))
}

/// Every signal assignment of process `pi` (not a port actual's) is made with delays above zero
/// that elaboration knows, so the signals it drives change only in the first delta of a time
/// step whenever the process runs; no procedure call, `force` or `release` (they might assign).
fn delayed_only(db: &Db, e: &Elaborated, pi: usize) -> bool {
    use yavs_sem::hir::{Rhs, Waveform};
    let p = &e.procs[pi];
    if p.actual.is_some() {
        return false;
    }
    // Transport or inertial, every transaction lies in a later time step.
    let delayed = |rhs: &Rhs<Waveform>| {
        waveforms(rhs).iter().all(|w| match w {
            Waveform::Elements(els) => els.iter().all(|el| {
                el.value.is_some() && el.after.is_some_and(|a| positive_time(db, e, pi, a))
            }),
            Waveform::Unaffected => true,
        })
    };
    let mut all = Vec::new();
    match &db.stmts[p.stmt].kind {
        StmtKind::ConcSignalAssign {
            guarded: false,
            rhs,
            ..
        } => return delayed(rhs),
        StmtKind::Process { body, .. } => deep(db, body, &mut all),
        _ => return false,
    }
    let mut any = false;
    for s in all {
        match &db.stmts[s].kind {
            StmtKind::SignalAssign { rhs, .. } => {
                if !delayed(rhs) {
                    return false;
                }
                any = true;
            }
            StmtKind::ProcCall(_) | StmtKind::Force { .. } | StmtKind::Release { .. } => {
                return false;
            }
            _ => {}
        }
    }
    any
}

/// The waveforms of an assignment's right-hand side.
fn waveforms(rhs: &yavs_sem::hir::Rhs<yavs_sem::hir::Waveform>) -> Vec<&yavs_sem::hir::Waveform> {
    use yavs_sem::hir::Rhs;
    match rhs {
        Rhs::Simple(w) => vec![w],
        Rhs::Conditional(a) => a.iter().map(|x| &x.0).collect(),
        Rhs::Selected { alts, .. } => alts.iter().map(|x| &x.0).collect(),
    }
}

/// Every activation of process `pi` starts at the first delta of a time step and assigns
/// without delay: one `wait for` (beside any `wait;`), no procedure call (it might wait), no
/// `after`; its timeout.
fn one_step(db: &Db, e: &Elaborated, pi: usize) -> Option<ExprId> {
    use yavs_sem::hir::{Delay, Waveform};
    let p = &e.procs[pi];
    if p.actual.is_some() {
        return None;
    }
    let StmtKind::Process {
        sensitivity: Sensitivity::None,
        body,
        ..
    } = &db.stmts[p.stmt].kind
    else {
        return None;
    };
    let mut all = Vec::new();
    deep(db, body, &mut all);
    let mut waits = Vec::new();
    for s in all {
        match &db.stmts[s].kind {
            StmtKind::Wait {
                on,
                until: None,
                timeout,
            } if on.is_empty() => waits.extend(*timeout),
            StmtKind::Wait { .. } => return None,
            StmtKind::ProcCall(_) => return None,
            StmtKind::SignalAssign { delay, rhs, .. } => {
                let timed = waveforms(rhs).iter().any(|w| match w {
                    Waveform::Elements(els) => els.len() != 1 || els[0].after.is_some(),
                    Waveform::Unaffected => false,
                });
                if !matches!(delay, Delay::Default) || timed {
                    return None;
                }
            }
            _ => {}
        }
    }
    match waits[..] {
        [t] => Some(t),
        _ => None,
    }
}

/// Members that would hand the region's settled value where the normal mode samples one that
/// has not settled yet: combinational logic (a combinational member, or the reset branch of a
/// clocked one) between a register of one clock and a register of another clock whose edges
/// may come in a later delta of the same time step (a clock gated or copied in the kernel, two
/// clock processes of their own). In the normal mode such a register samples the logic before
/// its new value arrives; the region settles it at the first edge's wake. Clocks one process
/// makes in one step (`delta_key`) or one signal's two edges never differ by a delta.
/// Element runs are taken as whole leaves (a superset).
/// A register whose asynchronous reset another register makes can change twice in a time step in
/// the normal mode: at the edge, then at the reset a delta later (a pulse of one delta). The region
/// settles both at once, so where that register's output is the clock or the asynchronous reset of
/// a process (in the region or outside), that process would miss the pulse: the register stays out.
fn pulse_rule(
    db: &Db,
    e: &Elaborated,
    taken: &BTreeMap<usize, (Fit, Body)>,
) -> Vec<(usize, Reason)> {
    let sig = |l: LeafId| e.nets.loc[l.index()].signal;
    // Each clock and asynchronous reset, with one process it clocks or resets.
    let mut control: BTreeMap<yavs_kernel::SignalId, usize> = BTreeMap::new();
    for (q, p) in e.procs.iter().enumerate() {
        if let Some(t) = &p.trigger {
            control.entry(sig(t.leaf)).or_insert(q);
            if let Some(r) = t.reset {
                control.entry(sig(r.0)).or_insert(q);
            }
        }
    }
    let registers: BTreeSet<yavs_kernel::SignalId> = (e.nets.drivers.iter())
        .filter(|d| e.procs[d.proc.index()].trigger.is_some())
        .map(|d| sig(d.leaf))
        .collect();
    let mut gone = Vec::new();
    for (&pi, (_, b)) in taken {
        let Some((r, ..)) = b.reset else {
            continue;
        };
        if !registers.contains(&sig(r)) {
            continue;
        }
        if let Some((l, q)) = (e.procs[pi].drivers.iter())
            .map(|&d| e.nets.drivers[d].leaf)
            .find_map(|l| control.get(&sig(l)).map(|&q| (l, q)))
        {
            let span = |p: usize| db.stmts[e.procs[p].stmt].span;
            gone.push((
                pi,
                Reason::new(
                    crate::codes::PULSE,
                    span(pi),
                    format!(
                        "a register reset by another register that drives a clock or reset: its \
                         asynchronous reset {} comes from a register, and it drives {}, the clock or \
                         asynchronous reset of {}",
                        e.leaf_name(r),
                        e.leaf_name(l),
                        e.proc_label(&e.procs[q])
                    ),
                )
                .also(span(q), format!("{} is clocked or reset by {}", e.proc_label(&e.procs[q]), e.leaf_name(l))),
            ));
        }
    }
    gone
}

/// Logic in the region between registers of clocks whose edges may come in different deltas of
/// a time step. Paths go through every process but the testbench's (behavioural: design 2.1),
/// members or not, and through port edges: a process left out for any reason (a metavalue
/// literal, history read) does not cut a path, and a register outside is a source and a sink of
/// its clock's class. Only members leave.
fn cross_delta(
    db: &Db,
    e: &Elaborated,
    taken: &BTreeMap<usize, (Fit, Body)>,
    out: &[(usize, Why)],
    forced: &Forced,
) -> Vec<(usize, Reason)> {
    let behavioural: BTreeSet<usize> = (out.iter())
        .filter(|(_, w)| matches!(w, Why::Behavioural(_)))
        .map(|(pi, _)| *pi)
        .collect();
    let outside = |pi: &usize| !taken.contains_key(pi) && !behavioural.contains(pi);
    let trig = |pi: usize| {
        (e.procs[pi].trigger.as_ref()).map(|t| Clock {
            leaf: t.leaf,
            elem: t.elem,
            rising: t.rising,
            legacy: t.legacy,
        })
    };
    // An edge-only process without a trigger (a sensitivity list with edge tests) acts at the
    // events of what its list names: placed when that is one element.
    let mut memo = BTreeMap::new();
    let mut place = |pi: usize| {
        trig(pi).or_else(|| {
            let (_, w) = proc_reads(db, e, pi, &mut memo).filter(|_| edge_only(db, e, pi))?;
            let [l] = w.into_iter().collect::<Vec<_>>()[..] else {
                return None;
            };
            (e.nets.leaf(l).ty.len == 1).then_some(Clock {
                leaf: l,
                elem: 0,
                rising: true,
                legacy: false,
            })
        })
    };
    let placed: BTreeMap<usize, Clock> = (0..e.procs.len())
        .filter(outside)
        .filter_map(|pi| Some((pi, place(pi)?)))
        .collect();
    // A process of the testbench kind with a trigger (a clocked logger) samples at its edge: a
    // sink, though no source.
    let tb: BTreeMap<usize, Clock> = (behavioural.iter().copied())
        .filter_map(|pi| Some((pi, trig(pi)?)))
        .collect();
    let clocks = (taken.values().filter_map(|(_, b)| b.clock))
        .chain(placed.values().copied())
        .chain(tb.values().copied());
    let mut keys: Vec<(bool, usize, u32)> = Vec::new();
    let mut names: Vec<Clock> = Vec::new();
    let mut bit: BTreeMap<(LeafId, u32), u64> = BTreeMap::new();
    for c in clocks {
        bit.entry((c.leaf, c.elem)).or_insert_with(|| {
            let k = delta_key(db, e, &c, forced);
            let g = keys.iter().position(|x| *x == k).unwrap_or_else(|| {
                keys.push(k);
                names.push(c);
                keys.len() - 1
            });
            // ponytail: beyond 63 groups they share the top bit, which counts as differing from
            // everything (sound, coarse); a set per leaf if a design ever has that many.
            1 << g.min(63)
        });
    }
    const TOP: u64 = 1 << 63;
    // A register may sample what registers of a group make whose edges never come before its
    // own delta (`first_delta`): they change after it has. Beyond 63 groups, the shared top bit
    // differs from everything.
    let first = |m: u64| first_delta(keys[m.trailing_zeros() as usize]);
    let bits = |m: u64| (0..64).map(|i| 1u64 << i).filter(move |b| m & b != 0);
    let differ = |t: u64, f: u64| {
        if (t | f) & TOP != 0 && t != 0 && f != 0 {
            return Some((t, f));
        }
        bits(t)
            .flat_map(|a| bits(f).map(move |b| (a, b)))
            .find(|&(a, b)| a != b && !first(b).1.is_some_and(|d| first(a).0 >= d))
    };
    // Registers by clock group, and what registers of each group read.
    let mut reg: BTreeMap<LeafId, u64> = BTreeMap::new();
    let mut feeds: BTreeMap<LeafId, u64> = BTreeMap::new();
    // Each register with its group and what it reads directly: the second place of an error.
    let mut samplers: Vec<(usize, u64, BTreeSet<LeafId>)> = Vec::new();
    // The combinational pieces: member or not, reads, writes.
    let mut pieces: Vec<(Option<usize>, Vec<LeafId>, Vec<LeafId>)> = Vec::new();
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
                samplers.push((pi, g, b.reads.iter().copied().collect()));
                if b.reset.is_some() {
                    let mut ws: Vec<LeafId> =
                        f.outs.iter().map(|o| LeafId(o.leaf as u32)).collect();
                    ws.dedup();
                    pieces.push((Some(pi), out_reads(f).collect(), ws));
                }
            }
            None => pieces.push((
                Some(pi),
                b.reads.iter().copied().collect(),
                b.writes.iter().copied().collect(),
            )),
        }
    }
    // Processes outside: a register by its clock (through its list too when it has an
    // asynchronous reset, as `run_count_cone` walks it), any other process as logic. Reads the
    // scan cannot follow, and a register whose clock is not known, count as every group.
    let mut anywhere = 0u64;
    let mut unknown = false;
    for pi in (0..e.procs.len()).filter(outside) {
        let mut ws: Vec<LeafId> = (e.procs[pi].drivers.iter())
            .map(|&d| e.nets.drivers[d].leaf)
            .collect();
        ws.dedup();
        let reads = proc_reads(db, e, pi, &mut memo);
        let g = match placed.get(&pi) {
            Some(c) => Some(bit[&(c.leaf, c.elem)]),
            None if edge_only(db, e, pi) => {
                unknown = true;
                Some(TOP)
            }
            None => None,
        };
        match (g, reads) {
            (Some(g), Some((r, w))) => {
                for &l in &ws {
                    *reg.entry(l).or_default() |= g;
                }
                // A monitor without a trigger that drives nothing (a reporter on `process (done)`)
                // sees settled values earlier by contract.
                let monitor = ws.is_empty() && trig(pi).is_none();
                if !monitor {
                    samplers.push((pi, g, r.iter().copied().collect()));
                }
                for l in r.into_iter().filter(|_| !monitor) {
                    *feeds.entry(l).or_default() |= g;
                }
                if e.procs[pi]
                    .trigger
                    .as_ref()
                    .is_some_and(|t| t.reset.is_some())
                {
                    pieces.push((None, w.into_iter().collect(), ws));
                }
            }
            (Some(g), None) => {
                for &l in &ws {
                    *reg.entry(l).or_default() |= g;
                }
                if !ws.is_empty() || trig(pi).is_some() {
                    anywhere |= g;
                }
            }
            (None, Some((r, _))) => pieces.push((None, r.into_iter().collect(), ws)),
            (None, None) => {
                for &l in &ws {
                    *reg.entry(l).or_default() |= TOP;
                }
                if !ws.is_empty() {
                    anywhere |= TOP;
                    unknown = true;
                }
            }
        }
    }
    for (&pi, c) in &tb {
        let g = bit[&(c.leaf, c.elem)];
        match proc_reads(db, e, pi, &mut memo) {
            Some((r, _)) => r
                .into_iter()
                .for_each(|l| *feeds.entry(l).or_default() |= g),
            None => anywhere |= g,
        }
    }
    // Port edges the kernel updates in the delta of their source.
    let port = |mode: yavs_kernel::PortMode, f: Vec<LeafId>, a: Vec<LeafId>| match mode {
        yavs_kernel::PortMode::In => vec![(None, a, f)],
        yavs_kernel::PortMode::Inout => vec![(None, a.clone(), f.clone()), (None, f, a)],
        _ => vec![(None, f, a)],
    };
    for pe in &e.nets.ports {
        pieces.extend(port(pe.mode, vec![pe.formal.0], vec![pe.actual.0]));
    }
    for cp in &e.nets.conv_ports {
        let side = |v: &[(LeafId, yavs_kernel::Range)]| v.iter().map(|x| x.0).collect();
        pieces.extend(port(cp.mode, side(&cp.formal), side(&cp.actual)));
    }
    // Logic a clock is made from: two levels of it in the region give the edge in fewer deltas.
    let mut gone = clock_logic(db, e, taken, &pieces);
    if keys.len() < 2 && !unknown {
        return gone;
    }
    stat(|| {
        let ks: Vec<String> = (keys.iter().zip(&names))
            .map(|(k, c)| format!("{} {k:?}", clock_name(e, c)))
            .collect();
        format!(
            "delta classes: {} pieces ({} ports), unknown {unknown}, read by an unknown scan {}, groups: {}",
            pieces.len(),
            e.nets.ports.len() + e.nets.conv_ports.len(),
            anywhere != 0,
            ks.join(", ")
        )
    });
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
    let name = |m: u64| match names.get(m.trailing_zeros() as usize) {
        Some(c) => clock_name(e, c),
        None => "a clock whose delta cycle yavs does not know".to_string(),
    };
    for (pi, rs, ws) in &pieces {
        let Some(pi) = pi else { continue };
        let t = (rs.iter())
            .map(|r| reg.get(r).copied().unwrap_or(0) | taint.get(r).copied().unwrap_or(0))
            .fold(0, |a, b| a | b);
        let f = (ws.iter())
            .map(|w| feeds.get(w).copied().unwrap_or(0))
            .fold(anywhere, |a, b| a | b);
        if gone.iter().any(|(q, _)| q == pi) {
            continue;
        }
        if let Some((from, to)) = differ(t, f) {
            // A register of the later clock reading what the piece writes, when one does directly.
            let sampler = (samplers.iter())
                .find(|(_, g, r)| g & to != 0 && ws.iter().any(|w| r.contains(w)))
                .map(|&(q, _, _)| {
                    let at = db.stmts[e.procs[q].stmt].span;
                    (
                        at,
                        format!("{} samples it on {}", e.proc_label(&e.procs[q]), name(to)),
                    )
                });
            let mut r = Reason::new(
                crate::codes::CROSS_DELTA,
                db.stmts[e.procs[*pi].stmt].span,
                format!(
                    "logic between clocks whose edges come in different delta cycles: registers \
                         on {} feed it and a register on {} samples it, with its edge in a later \
                         delta cycle",
                    name(from),
                    name(to)
                ),
            );
            r.also = sampler;
            gone.push((*pi, r));
        }
    }
    gone
}

/// The members in the combinational fan-in of a clock (of any process) that read what another
/// member drives: the region exports their value in the delta after its inputs change, where the
/// normal mode takes a delta per level, so the edge would come early. One level reading only
/// inputs changes in the same delta in both modes.
fn clock_logic(
    db: &Db,
    e: &Elaborated,
    taken: &BTreeMap<usize, (Fit, Body)>,
    pieces: &[(Option<usize>, Vec<LeafId>, Vec<LeafId>)],
) -> Vec<(usize, Reason)> {
    let mut clocks: BTreeSet<LeafId> = (e.procs.iter())
        .filter_map(|p| p.trigger.as_ref().map(|t| t.leaf))
        .collect();
    clocks.extend(taken.values().filter_map(|(_, b)| b.clock.map(|c| c.leaf)));
    let made: BTreeSet<LeafId> = (taken.keys())
        .flat_map(|&pi| e.procs[pi].drivers.iter().map(|&d| e.nets.drivers[d].leaf))
        .collect();
    let mut by_write: BTreeMap<LeafId, Vec<usize>> = BTreeMap::new();
    for (i, (_, _, ws)) in pieces.iter().enumerate() {
        for &w in ws {
            by_write.entry(w).or_default().push(i);
        }
    }
    let (mut seen, mut done) = (BTreeSet::new(), BTreeSet::new());
    let mut work: Vec<(LeafId, LeafId)> = clocks.iter().map(|&c| (c, c)).collect();
    let mut gone = Vec::new();
    while let Some((l, c)) = work.pop() {
        if !seen.insert(l) {
            continue;
        }
        for &i in by_write.get(&l).into_iter().flatten() {
            if !done.insert(i) {
                continue;
            }
            let (pi, rs, _) = &pieces[i];
            if let Some(pi) = pi
                && let Some(&m) = rs.iter().find(|r| made.contains(r))
            {
                gone.push((
                    *pi,
                    Reason::new(
                        crate::codes::CLOCK_LOGIC,
                        db.stmts[e.procs[*pi].stmt].span,
                        format!(
                            "logic in a clock path: it computes part of the clock {} from {}, which \
                             the region drives",
                            e.leaf_name(c),
                            e.leaf_name(m)
                        ),
                    ),
                ));
                // The process the clock clocks.
                let user = (e
                    .procs
                    .iter()
                    .position(|p| p.trigger.as_ref().is_some_and(|t| t.leaf == c)))
                .or_else(|| {
                    (taken.iter())
                        .find(|(_, (_, b))| b.clock.is_some_and(|k| k.leaf == c))
                        .map(|(q, _)| *q)
                });
                if let (Some(q), Some(last)) = (user, gone.last_mut()) {
                    let at = db.stmts[e.procs[q].stmt].span;
                    last.1.also = Some((
                        at,
                        format!(
                            "{} is clocked by {}",
                            e.proc_label(&e.procs[q]),
                            e.leaf_name(c)
                        ),
                    ));
                }
            }
            work.extend(rs.iter().map(|&r| (r, c)));
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
fn ram_rule(db: &Db, e: &Elaborated, taken: &BTreeMap<usize, (Fit, Body)>) -> Vec<(usize, Reason)> {
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
    let seen = (!outside_reads_all(db)).then(|| outside_reads(db, e, &members).0);
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
            let text = if in_reset {
                format!("a memory written in an asynchronous reset branch: {name}")
            } else if !ok {
                format!("a memory written both at run-time indices and at fixed ones: {name}")
            } else if outside {
                format!("a memory read outside the region: {name}")
            } else {
                continue;
            };
            let span = db.stmts[e.procs[pi].stmt].span;
            gone.push((pi, Reason::new(crate::codes::MEMORY, span, text)));
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
    let (mut outputs, waited, readers) = if outside_reads_all(db) {
        (driven.clone(), driven.clone(), BTreeMap::new())
    } else {
        let (seen, waited, readers) = outside_reads(db, e, &is_member);
        (
            driven.intersection(&seen).copied().collect(),
            waited,
            readers,
        )
    };
    // `--wave`: the leaves it records are exported too (not memories of integers, which the export
    // cannot take), before `at_once` so an input change reaching them settles in its time step.
    if let Some(w) = wave() {
        let k = |l: &LeafId| e.nets.loc[l.index()].signal.0;
        let mut rams = BTreeMap::new();
        for (_, b) in members.iter().filter(|m| m.1.kind == Kind::Clocked) {
            sig_writes(&b.ops, &mut rams);
        }
        let held = |s: u32| {
            rams.get(&s).is_some_and(|h: &How| h.indexed)
                && big_signal(e, s)
                && e.nets.leaf(e.nets.packs[s as usize].0).ty.elem != yavs_kernel::Elem::Logic
        };
        let rec = |l: &&LeafId| w.binary_search(&k(l)).is_ok() && !held(k(l));
        outputs.extend(driven.iter().filter(rec).copied());
    }
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
/// themselves; leaf by leaf, a read reaching every leaf the code writes) to an exported output, to
/// combinational code with a check, or to a reset branch: a change of such an input is visible
/// before the next edge, or loads registers between edges.
fn at_once(
    members: &[(usize, Body)],
    reset_reads: &BTreeSet<LeafId>,
    outputs: &BTreeSet<LeafId>,
    readers: &BTreeMap<LeafId, usize>,
    wakes: &BTreeSet<LeafId>,
) -> BTreeMap<LeafId, Reach> {
    let mut succ: BTreeMap<LeafId, BTreeSet<LeafId>> = BTreeMap::new();
    let mut checked: BTreeMap<LeafId, usize> = BTreeMap::new();
    let mut resets: BTreeMap<LeafId, usize> = BTreeMap::new();
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
            if b.kind == Kind::Clocked {
                resets.entry(r).or_insert(*pi);
            }
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
        if !out.contains_key(&w)
            && let Some(&pi) = seen.iter().find_map(|l| resets.get(l))
        {
            out.insert(w, Reach::Reset(pi));
        }
    }
    out
}

/// Something may read any signal: an external signal name in a subprogram of a package, which
/// the scan of outside reads does not follow. Elaboration binds the other external names to
/// aliases the scan resolves like any name; one it cannot resolve makes `proc_reads` give `None`.
fn outside_reads_all(db: &Db) -> bool {
    use yavs_sem::decl::{Alias, Body as B, DeclKind};
    use yavs_sem::external::ExtOwner;
    use yavs_syntax::ast::ExternalClass;
    if db.externals.is_empty() {
        return false;
    }
    let signal = |x: yavs_sem::ids::ExternalId| db.externals[x].class == ExternalClass::Signal;
    let alias = |d| db.external_alias(d).is_some_and(signal);
    // Subprogram `m` or one nested in it names an external signal.
    fn names(
        db: &Db,
        m: yavs_sem::DeclId,
        hit: &dyn Fn(ExprId) -> bool,
        alias: &dyn Fn(yavs_sem::DeclId) -> bool,
    ) -> bool {
        let Some(sp) = db.decls[m].kind.subprogram() else {
            return false;
        };
        let B::Hir { scope, stmts } = &sp.body else {
            return false;
        };
        if db.external_uses(ExtOwner::Sub(m)).iter().any(|&d| alias(d))
            || sp.params.iter().filter_map(|p| p.default).any(hit)
        {
            return true;
        }
        for &x in &db.scopes[*scope].order {
            match &db.decls[x].kind {
                DeclKind::Object(o) => {
                    let mut xs: Vec<ExprId> = o.default.into_iter().collect();
                    constraint_exprs(&db.subtypes[o.subtype].constraint, &mut xs);
                    if xs.into_iter().any(hit) {
                        return true;
                    }
                }
                DeclKind::Alias(Alias::Object { target, .. }) if alias(x) || hit(*target) => {
                    return true;
                }
                _ if db
                    .decl_subprograms(x)
                    .into_iter()
                    .any(|n| names(db, n, hit, alias)) =>
                {
                    return true;
                }
                _ => {}
            }
        }
        let mut all = Vec::new();
        deep(db, stmts, &mut all);
        all.iter().any(|&s| {
            let mut xs = Vec::new();
            db.stmt_exprs(s, &mut |x, _| xs.push(x));
            // Also the names of waits and drivers that are no expression.
            let mut lsps = Vec::new();
            db.stmt_reads(s, &mut lsps);
            xs.into_iter().any(hit)
                || lsps.iter().any(|l| match l.root {
                    ObjRoot::External(x) => db.externals[x].class == ExternalClass::Signal,
                    ObjRoot::Decl(d) => alias(d),
                    ObjRoot::ImplicitSignal(_) => false,
                })
        })
    }
    let hit = |e: ExprId| {
        let mut found = false;
        db.walk_expr(e, &mut |x| {
            found |= match db.exprs[x].kind {
                ExprKind::External(x) => signal(x),
                ExprKind::Obj(d) => alias(d),
                _ => false,
            };
        });
        found
    };
    (db.units.iter()).any(|(u, unit)| {
        db.libs[unit.lib].name.key.as_str() != "std"
            && (db.package_regions(u).unwrap_or_default().into_iter())
                .flat_map(|r| db.decls_with_instances(r))
                .flat_map(|d| db.decl_subprograms(d))
                .any(|m| names(db, m, &hit, &alias))
    })
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

/// Developers (`YAVS_CYCLE_DOWNSTREAM`): the processes outside the region that read an exported
/// leaf, directly or through other processes outside, port edges and implicit signals. Only they
/// can see a value the region made two-state (tools/difftest README "contract classes").
pub fn downstream(db: &Db, e: &Elaborated, sel: &Selection) -> Vec<usize> {
    let mut hot: BTreeSet<LeafId> = sel.outputs.clone();
    let mut memo = BTreeMap::new();
    let mut todo: Vec<(usize, Option<BTreeSet<LeafId>>)> = (sel.out.iter())
        .map(|&(pi, _)| (pi, proc_reads(db, e, pi, &mut memo).map(|r| r.0)))
        .collect();
    let mut links: Vec<(LeafId, LeafId)> = (e.nets.ports.iter())
        .map(|p| (p.actual.0, p.formal.0))
        .collect();
    for cp in &e.nets.conv_ports {
        for a in &cp.actual {
            links.extend(cp.formal.iter().map(|f| (a.0, f.0)));
        }
    }
    for (i, leaf) in e.nets.leaves.iter().enumerate() {
        if let Some((_, src, _)) = leaf.implicit {
            links.push((src, LeafId(i as u32)));
        }
    }
    let mut found = Vec::new();
    loop {
        // Both ways across a link: an over-approximation, so the rule only ever explains less.
        for &(a, b) in &links {
            if hot.contains(&a) || hot.contains(&b) {
                hot.insert(a);
                hot.insert(b);
            }
        }
        let n = found.len();
        todo.retain(|(pi, reads)| {
            let hit = reads
                .as_ref()
                .is_none_or(|r| r.iter().any(|l| hot.contains(l)));
            if hit {
                found.push(*pi);
            }
            !hit
        });
        if found.len() == n {
            break;
        }
        for &pi in &found[n..] {
            hot.extend(e.procs[pi].drivers.iter().map(|&d| e.nets.drivers[d].leaf));
        }
    }
    found.sort_unstable();
    found
}

/// The signal leaves process `pi` may read (its names, whole objects, and those of the local
/// subprograms it calls), and of those the ones it waits on; `None` when the scan cannot follow
/// it (an external name without the alias elaboration bound it to, a call it cannot see into):
/// it may read every leaf.
pub(crate) fn proc_reads(
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
    let ext_alias = |x: yavs_sem::ids::ExternalId| {
        let mut cands: Vec<yavs_sem::DeclId> = db
            .external_uses(yavs_sem::external::ExtOwner::Stmt(p.stmt))
            .to_vec();
        if let StmtKind::Process { scope, .. } = &db.stmts[p.stmt].kind {
            cands.extend(&db.scopes[*scope].order);
        }
        // The declarative regions of the process's region and of those around it.
        let mut sc = Some(p.scope);
        while let Some(s) = sc {
            for &r in e.regions.get(&s).into_iter().flatten() {
                cands.extend(&db.scopes[r].order);
            }
            sc = e.hier.scope(s).parent;
        }
        let span = db.externals[x].span;
        cands.into_iter().find(|&d| {
            db.external_alias(d)
                .is_some_and(|y| db.externals[y].span == span)
        })
    };
    for (l, wait) in lsps
        .iter()
        .map(|l| (l, false))
        .chain(waits.iter().map(|l| (l, true)))
    {
        // A root that is an external name (a wait on one, a declared alias of one): the alias
        // elaboration bound it to, for the same name in the source (sem may give the root an
        // id of its own).
        let root = match l.root {
            ObjRoot::External(x) => ObjRoot::Decl(ext_alias(x)?),
            r => r,
        };
        match root {
            // An implicit signal (`'transaction`, `'stable`, ...) is read with the signals it
            // follows: its value comes from their events or transactions.
            ObjRoot::Decl(d) | ObjRoot::ImplicitSignal(d) => {
                let (a, z) = e.nets.obj(e.signal(p.scope, d)?).leaves;
                let mut leaves: Vec<LeafId> = (a.0..z.0).map(LeafId).collect();
                for k in a.0..z.0 {
                    let leaf = e.nets.leaf(LeafId(k));
                    leaves.extend(leaf.implicit.map(|x| x.1));
                    leaves.extend(leaf.implicit_more.iter().map(|x| x.0));
                }
                if wait {
                    waited.extend(leaves.iter().copied());
                }
                seen.extend(leaves);
            }
            ObjRoot::External(_) => {}
        }
    }
    Some((seen, waited))
}

/// The processes outside whose result may depend on the deltas their inputs change in (see
/// `run_count_cone`), each with why.
fn delta_seeds(
    db: &Db,
    e: &Elaborated,
    out: &[(usize, Why)],
    checked: &BTreeSet<usize>,
) -> Vec<(usize, &'static str)> {
    (out.iter())
        .filter_map(|(pi, w)| {
            let how = match w {
                // A loop of elements (a latch written as feedback, `l <= d when en else l`)
                // keeps a value too, even when its processes fit one by one.
                Why::Subset(r) if r.code == crate::codes::LOOP => {
                    "is in a combinational loop, which keeps a value from its previous run"
                }
                // ponytail: an edge-only process without a trigger (a sensitivity list with
                // edge tests) is taken as early; find its clock if the fuzzer shows one.
                _ if checked.contains(pi) || edge_only(db, e, *pi) => return None,
                _ if plain_assignment(db, e, *pi) => return None,
                Why::Behavioural(_) => return None,
                Why::Subset(r) if [crate::codes::LATCH, crate::codes::STATE].contains(&r.code) => {
                    "keeps a value from its previous run (a latch, or a variable read before it is assigned)"
                }
                // A testbench process sees the region's settled values earlier by contract
                // ("Delta cycles inside the region are not visible" of the user guide): one of
                // the top-level unit, and below it one that drives nothing (a monitor).
                // Only the top-level unit: a model outside the cycle scope can count its runs.
                _ if in_top_scope(e, e.procs[*pi].scope) || e.procs[*pi].drivers.is_empty() => {
                    return None;
                }
                Why::Subset(r) if r.code == crate::codes::SENSITIVITY => {
                    "has an incomplete sensitivity list"
                }
                _ => "is outside the region for another reason (see its own error), which yavs treats as keeping a value from its previous run",
            };
            Some((*pi, how))
        })
        .collect()
}

/// The combinational processes (members or not) in the fan-in of what a process outside the
/// region reads whose result may depend on the deltas its inputs change in: every process
/// without an edge trigger that is not of the testbench kind and that the checker did not fit.
/// Such a process may keep a value of an earlier run (state, a latch: E4119, E4120), read a
/// signal its sensitivity list does not name (E4121: it sees that signal as of the delta a
/// listed one changed in), or be rejected for another reason before the checker could show it
/// does neither. Back from its reads through every combinational process driving them, up to
/// registers (`edge_only`, whose outputs come in the delta after the edge in both modes). A register
/// with an asynchronous reset stays out too (the normal mode changes it twice in a time step when a
/// reset made by another register follows the edge), the walk going on through what it waits on. The region settles such logic in one delta where the normal mode spreads it over
/// several, so the reader would see other values or run another number of times (Microwatt's
/// `dcache_request` gives `req_nc` the `nc` of its previous run). Each with the reason; `None`
/// reads (a name the scan cannot follow) reach every leaf.
fn run_count_cone(
    db: &Db,
    e: &Elaborated,
    out: &[(usize, Why)],
    sensitive: &[(usize, &str)],
    comb_deps: &BTreeMap<usize, Vec<crate::dep::Out>>,
    seen_tx: Option<&BTreeSet<LeafId>>,
) -> Vec<(usize, Reason)> {
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
    let mut procs: BTreeSet<usize> = sensitive.iter().map(|s| s.0).collect();
    let mut leaves: BTreeSet<LeafId> = BTreeSet::new();
    let mut work: Vec<(LeafId, usize)> = Vec::new();
    for (i, &(q, _)) in sensitive.iter().enumerate() {
        match proc_reads(db, e, q, &mut memo) {
            Some((r, _)) => work.extend(r.into_iter().map(|l| (l, i))),
            None => work.extend(every().map(|l| (l, i))),
        }
    }
    let mut gone = Vec::new();
    while let Some((l, q)) = work.pop() {
        if !leaves.insert(l) {
            continue;
        }
        for &d in drivers.get(&l).into_iter().flatten() {
            if edge_only(db, e, d) || behavioural.contains(&d) {
                continue;
            }
            // A combinational process the checker fitted changes `l` only when what `l`
            // depends on changes: the walk goes on through those leaves alone.
            let narrow = (seen_tx.is_some_and(|t| !t.contains(&l)))
                .then(|| comb_deps.get(&d).and_then(|o| leaf_deps(e, d, o, l)))
                .flatten();
            let narrowed = narrow.is_some();
            if let Some(dep) = narrow {
                work.extend(dep.into_iter().map(|x| (x, q)));
            }
            if !procs.insert(d) {
                continue;
            }
            let reader = e.proc_label(&e.procs[sensitive[q].0]);
            gone.push((
                d,
                Reason::new(
                    crate::codes::DELTA_FANIN,
                    db.stmts[e.procs[d].stmt].span,
                    format!(
                        "{} feeding a process that depends on delta cycles: it drives {}, which \
                         {reader} reads through logic",
                        if e.procs[d].trigger.is_some() {
                            "a register with an asynchronous reset"
                        } else {
                            "combinational logic"
                        },
                        e.leaf_name(l),
                    ),
                )
                .also(
                    db.stmts[e.procs[sensitive[q].0].stmt].span,
                    format!("{reader}, outside the region, {}", sensitive[q].1),
                ),
            ));
            // A register's data comes at the edge in both modes: only what its list waits on
            // (the reset; its reset branch reads no more where the checker fitted it) is logic.
            let reg = e.procs[d].trigger.is_some();
            if narrowed {
                continue;
            }
            match proc_reads(db, e, d, &mut memo) {
                Some((_, w)) if reg => work.extend(w.into_iter().map(|x| (x, q))),
                Some((r, _)) => work.extend(r.into_iter().map(|x| (x, q))),
                None => work.extend(every().map(|x| (x, q))),
            }
        }
    }
    gone
}

/// The signal leaves the runs of leaf `l` in `outs` of process `pi` depend on; `None` when the
/// runs do not cover every element `pi` drives of `l` or one depends on a variable.
fn leaf_deps(
    e: &Elaborated,
    pi: usize,
    outs: &[crate::dep::Out],
    l: LeafId,
) -> Option<BTreeSet<LeafId>> {
    let mut have = crate::dep::ESet::default();
    for o in outs.iter().filter(|o| o.leaf == l.0 as u64) {
        have.add(o.leaf, o.lo, o.hi);
    }
    for &d in &e.procs[pi].drivers {
        let d = &e.nets.drivers[d];
        if d.leaf == l
            && have
                .first_missing(l.0 as u64, d.range.offset, d.range.end())
                .is_some()
        {
            return None;
        }
    }
    let mut r = BTreeSet::new();
    for o in outs.iter().filter(|o| o.leaf == l.0 as u64) {
        for k in (o.dep.set.iter().map(|x| x.0)).chain(o.dep.ident.map(|x| x.0)) {
            if k & crate::dep::VAR != 0 {
                return None;
            }
            r.insert(LeafId(k as u32));
        }
    }
    Some(r)
}

/// Whether process `pi` acts only at clock edges, as a register does: an edge trigger without an
/// asynchronous reset, or a process with a sensitivity list whose every statement outside an
/// edge test (`if <edge> [and ...]`) is an `if`, an assert or a report (a RAM on two clocks, a
/// memory checking its size). Its outputs change in the delta after an edge in both modes.
fn edge_only(db: &Db, e: &Elaborated, pi: usize) -> bool {
    fn edge(db: &Db, c: ExprId) -> bool {
        if db.edge_shape(c).is_some() {
            return true;
        }
        let ExprKind::Call { callee, args, .. } = &db.exprs[c].kind else {
            return false;
        };
        let and = matches!(
            db.decls[*callee].kind.subprogram().map(|s| &s.body),
            Some(yavs_sem::decl::Body::Implicit(op)) if op.family == yavs_sem::implicit::Family::And
        );
        and && args.iter().any(|a| match a.actual {
            yavs_sem::hir::Actual::Expr(x) => edge(db, x),
            _ => false,
        })
    }
    fn outside(db: &Db, body: &[StmtId]) -> bool {
        body.iter().all(|&s| match &db.stmts[s].kind {
            StmtKind::If { branches, else_ } => {
                branches.iter().all(|(c, b)| edge(db, *c) || outside(db, b))
                    && else_.as_deref().is_none_or(|b| outside(db, b))
            }
            // Messages only: a check of a generic (`if SIZE > MAX then assert false`).
            StmtKind::Null | StmtKind::Assert { .. } | StmtKind::Report { .. } => true,
            _ => false,
        })
    }
    let p = &e.procs[pi];
    // An asynchronous reset branch is combinational in what it reads (the checker's fit shows
    // it reads only what its list names; that is not known here).
    p.trigger.as_ref().is_some_and(|t| t.reset.is_none())
        || p.actual.is_none()
            && matches!(&db.stmts[p.stmt].kind, StmtKind::Process {
                sensitivity: Sensitivity::List(_),
                body,
                ..
            } if outside(db, body))
}

/// Whether process `pi` is a concurrent signal assignment that keeps no value between runs and
/// waits on all it reads: not guarded, no `unaffected`, a final `else`, only pure functions
/// called, no attribute of a signal's history or events, every signal it reads (in the bodies
/// of the functions too) in its wait set, no delay. Such a statement is a function of the values
/// of its last run, whatever made the checker reject it (a `'Z'`). A delay keeps a value: an
/// inertial one rejects a pending transaction on a one-delta glitch, a transport one with delays
/// that differ by alternative keeps the glitch's transaction (`inertial.vhd`).
fn plain_assignment(db: &Db, e: &Elaborated, pi: usize) -> bool {
    use yavs_sem::attr::PredefAttr;
    use yavs_sem::decl::SubprogramKind;
    use yavs_sem::hir::{Delay, Rhs, Waveform};
    let p = &e.procs[pi];
    let StmtKind::ConcSignalAssign {
        guarded: false,
        delay: Delay::Default,
        rhs,
        ..
    } = &db.stmts[p.stmt].kind
    else {
        return false;
    };
    let delayed =
        |w: &Waveform| matches!(w, Waveform::Elements(x) if x.iter().any(|x| x.after.is_some()));
    if waveforms(rhs).into_iter().any(delayed) {
        return false;
    }
    if p.actual.is_some() {
        return false;
    }
    let complete = match rhs {
        Rhs::Simple(w) => !matches!(w, Waveform::Unaffected),
        Rhs::Conditional(alts) => {
            alts.last().is_some_and(|a| a.1.is_none())
                && alts.iter().all(|a| !matches!(a.0, Waveform::Unaffected))
        }
        Rhs::Selected { alts, .. } => alts.iter().all(|a| !matches!(a.0, Waveform::Unaffected)),
    };
    let mut xs = Vec::new();
    db.stmt_exprs(p.stmt, &mut |x, _| xs.push(x));
    let mut plain = complete;
    for x in xs {
        db.walk_expr(x, &mut |y| match &db.exprs[y].kind {
            ExprKind::Call { callee, .. } => {
                plain &= matches!(
                    db.decls[*callee].kind.subprogram().map(|s| &s.kind),
                    Some(SubprogramKind::Function { pure: true })
                )
            }
            ExprKind::Attr { attr, .. } => {
                use PredefAttr::*;
                plain &=
                    hist_attr(*attr).is_none() && !matches!(attr, Event | Driving | DrivingValue)
            }
            ExprKind::Obj(d) => plain &= !is_implicit(db, *d),
            ExprKind::External(_) => plain = false,
            _ => {}
        });
    }
    let drives: BTreeSet<LeafId> = (p.drivers.iter())
        .map(|&d| e.nets.drivers[d].leaf)
        .collect();
    // It reads none of what it drives: `l <= d when en = '1' else l` is a latch.
    plain
        && proc_reads(db, e, pi, &mut BTreeMap::new())
            .is_some_and(|(seen, waited)| seen.is_subset(&waited) && seen.is_disjoint(&drives))
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
                        "a call of a subprogram reading the history".to_string()
                    } else if what == "an implicit signal" {
                        "an implicit signal (`'stable`, `'quiet`, `'delayed` or `'transaction`)"
                            .to_string()
                    } else {
                        format!("`'{what}`")
                    };
                    (
                        span,
                        format!(
                            "history of a signal read outside the region: {how} of {}, read by {}",
                            e.leaf_name(l),
                            e.proc_label(&e.procs[q])
                        ),
                    )
                }
                None if implicit.contains(&l) => (
                    db.stmts[e.procs[pi].stmt].span,
                    format!(
                        "history of a signal read outside the region: an implicit signal \
                         (`'stable`, `'quiet`, `'delayed` or `'transaction`) of {}",
                        e.leaf_name(l)
                    ),
                ),
                None => continue,
            };
            let mut r = Reason::new(crate::codes::HISTORY_READ, text.0, text.1);
            if r.span != db.stmts[e.procs[pi].stmt].span {
                r = r.also(
                    db.stmts[e.procs[pi].stmt].span,
                    format!("{} drives {}", e.proc_label(&e.procs[pi]), e.leaf_name(l)),
                );
            }
            gone.push((pi, r));
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
                    Reason::new(
                        crate::codes::TESTBENCH,
                        span(pi),
                        format!(
                            "a shared variable used by two processes: `{}` is also used by {}",
                            db.decls[k.1].name,
                            e.proc_label(&e.procs[q])
                        ),
                    )
                    .also(
                        span(q),
                        format!(
                            "{} uses `{}` too",
                            e.proc_label(&e.procs[q]),
                            db.decls[k.1].name
                        ),
                    ),
                ));
                break;
            }
        }
    }
    gone
}

/// What the forces and releases the design can run keep out (E4128).
enum Forced {
    /// A force whose targets are not known: every process.
    All(StmtId),
    /// The leaves forced, each with one force of it: their drivers.
    Leaves(BTreeMap<LeafId, StmtId>),
}

impl Forced {
    /// The leaf may be forced.
    fn has(&self, l: LeafId) -> bool {
        match self {
            Forced::All(_) => true,
            Forced::Leaves(m) => m.contains_key(&l),
        }
    }

    fn reason(&self, db: &Db, e: &Elaborated, pi: usize, span: yavs_diag::Span) -> Option<Reason> {
        let (text, s) = match self {
            Forced::All(s) => ("`force` or `release` in the design".to_string(), *s),
            Forced::Leaves(m) => {
                let (l, s) = (e.procs[pi].drivers.iter())
                    .find_map(|&d| m.get_key_value(&e.nets.drivers[d].leaf))?;
                (
                    format!(
                        "a signal forced elsewhere in the design: {}",
                        e.leaf_name(*l)
                    ),
                    *s,
                )
            }
        };
        let r = Reason::new(crate::codes::FORCED, span, text);
        Some(r.also(db.stmts[s].span, "the design forces a signal here"))
    }
}

/// The forces and releases the elaborated design can run (in a process, or in a subprogram a
/// process calls) and what they keep out. A force whose target is a signal parameter or a name
/// not placed in the design keeps every process out; a call the scan cannot follow makes every
/// force count. Members read a forced signal from the kernel like any input.
fn forced(db: &Db, e: &Elaborated) -> Forced {
    use yavs_sem::decl::{DeclKind, ObjRole};
    let facts = db.facts();
    let mut leaves = BTreeMap::new();
    if facts.forces.is_empty() {
        return Forced::Leaves(leaves);
    }
    let (reached, ok) = yavs_elab::reach::forces(db, &e.procs);
    stat(|| {
        format!(
            "forces: {} in the design, {} reached, every call followed: {ok}",
            facts.forces.len(),
            reached.len()
        )
    });
    // The signal objects of each declaration and external name, in every instance.
    let mut objs: BTreeMap<yavs_sem::DeclId, Vec<yavs_elab::ObjId>> = BTreeMap::new();
    let mut ext: BTreeMap<yavs_sem::ids::ExternalId, Vec<yavs_elab::ObjId>> = BTreeMap::new();
    for (&(_, d), &o) in &e.sigs {
        objs.entry(d).or_default().push(o);
        if let Some(x) = db.external_alias(d) {
            ext.entry(x).or_default().push(o);
        }
    }
    for &s in &reached {
        let (StmtKind::Force { target, .. } | StmtKind::Release { target, .. }) = &db.stmts[s].kind
        else {
            return Forced::All(s);
        };
        if target.lsps.is_empty() {
            return Forced::All(s);
        }
        for l in &target.lsps {
            let os = match l.root {
                ObjRoot::Decl(d)
                    if !matches!(&db.decls[d].kind,
                        DeclKind::Object(o) if o.role == ObjRole::Param) =>
                {
                    objs.get(&d)
                }
                ObjRoot::External(x) => ext.get(&x),
                _ => None,
            };
            let Some(os) = os else {
                return Forced::All(s);
            };
            for &o in os {
                let (a, z) = e.nets.obj(o).leaves;
                for k in a.0..z.0 {
                    leaves.entry(LeafId(k)).or_insert(s);
                }
            }
        }
    }
    Forced::Leaves(leaves)
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
                match range {
                    yavs_sem::types::RangeC::Explicit { left, right, .. } => {
                        stack.extend([*left, *right])
                    }
                    yavs_sem::types::RangeC::Record(e) => stack.push(*e),
                    yavs_sem::types::RangeC::Attr { .. } => {}
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
pub(crate) struct CallSigs {
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

/// A subprogram declared outside a library package (in an architecture, block, process, another
/// subprogram of one or a package declared in one): its body may read the signals of its region
/// directly.
fn local_subprogram(db: &Db, d: yavs_sem::DeclId) -> bool {
    use yavs_sem::scope::ScopeKind as S;
    let mut s = Some(db.decls[d].scope);
    while let Some(x) = s {
        match db.scopes[x].kind {
            // A package declared in a design unit's region (not a library unit) is local: its
            // subprograms may read that region's signals.
            S::Architecture | S::Entity | S::Process | S::Block | S::Generate => return true,
            _ => s = db.scopes[x].parent,
        }
    }
    false
}

/// The process is outside the cycle scope (A64): of the testbench, exempt under
/// `--cycle-strict`, never a member.
pub(crate) fn outside_scope(e: &Elaborated, pi: usize) -> bool {
    !e.cycle_in(e.procs[pi].scope)
}

/// The process is directly in the top-level design unit (its architecture, or a block or
/// generate of it) and the cycle scope is the whole design (A76): the testbench, under the
/// top-level rules (A19) and exempt under `--cycle-strict`.
pub(crate) fn in_top_unit(e: &Elaborated, pi: usize) -> bool {
    e.scopes.cycle_whole && in_top_scope(e, e.procs[pi].scope)
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
