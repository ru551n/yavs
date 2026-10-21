//! Interval analysis of the integer registers of a function that cannot suspend (#444).
//!
//! Lowering checks every subtype conversion, every incremented loop variable and every
//! length it computes, and selects between the results of tests that depend on lengths
//! being non-negative. Most of that is decided by what the code just did: a length is not
//! negative, a counter that was checked and incremented stays in range. This pass finds an
//! interval `[lo, hi]` for every `I64` register at every instruction (a forward dataflow
//! over the blocks, joins by hull, widening after a few visits, the test of a conditional
//! branch narrowing the registers it compares on each edge) and then
//!
//! - drops a `Check` the interval proves,
//! - turns a `BinRange` whose result cannot leave its range (and cannot overflow) into a `Bin`,
//! - replaces a `Cmp` the intervals decide by a constant, and a `CondBr` on a decided
//!   condition by a `Br`.
//!
//! Soundness rests on the interpreter's checked arithmetic: an `Add`, `Sub` or `Mul` that
//! overflows is a runtime error, so the registers of the continuing path hold only results
//! within `i64`. Everything the analysis does not model is the full range.

use super::*;

type Iv = (i64, i64);
const FULL: Iv = (i64::MIN, i64::MAX);

/// Visits of a block after which a bound that still moves jumps to infinity.
const WIDEN: u32 = 2;
/// Functions of fewer instructions are left alone: the analysis costs more than it finds
/// (every subprogram is optimised the first time it runs, and most run a few times).
pub(super) const MIN_CODE: usize = 40;
/// Registers times blocks above which the analysis is skipped (its state is a vector of
/// intervals per block).
const BUDGET: usize = 1 << 15;

/// An interval from `i128` bounds: the part inside `i64` (arithmetic that leaves `i64` is a
/// runtime error); all of `i64` when nothing is left.
fn mk(lo: i128, hi: i128) -> Iv {
    let (l, h) = (lo.max(i128::from(i64::MIN)), hi.min(i128::from(i64::MAX)));
    if l > h { FULL } else { (l as i64, h as i64) }
}

fn meet(a: Iv, b: Iv) -> Iv {
    let r = (a.0.max(b.0), a.1.min(b.1));
    // An empty meet means the path cannot run; keep what is known of `a`.
    if r.0 > r.1 { a } else { r }
}

/// The interval of `x op y` and whether the result is certain to be representable (no
/// operand pair in the intervals overflows).
fn bin_iv(op: IntOp, x: Iv, y: Iv) -> (Iv, bool) {
    let (xl, xh, yl, yh) = (
        i128::from(x.0),
        i128::from(x.1),
        i128::from(y.0),
        i128::from(y.1),
    );
    let exact = |lo: i128, hi: i128| {
        let fits = lo >= i128::from(i64::MIN) && hi <= i128::from(i64::MAX);
        (mk(lo, hi), fits)
    };
    match op {
        IntOp::Add => exact(xl + yl, xh + yh),
        IntOp::Sub => exact(xl - yh, xh - yl),
        IntOp::Mul => {
            let p = [xl * yl, xl * yh, xh * yl, xh * yh];
            exact(
                p.iter().copied().min().unwrap_or(0),
                p.iter().copied().max().unwrap_or(0),
            )
        }
        // By a positive constant: truncating division is monotone.
        IntOp::Div if yl == yh && yl > 0 => (mk(xl / yl, xh / yl), true),
        // `mod` by a positive constant is in 0 .. c - 1; `rem` follows the dividend's sign.
        IntOp::Mod if yl == yh && yl > 0 => (mk(0, yl - 1), true),
        IntOp::Rem if yl == yh && yl > 0 => {
            if x.0 >= 0 {
                (mk(0, (yl - 1).min(xh)), true)
            } else {
                (mk(-(yl - 1), yl - 1), true)
            }
        }
        _ => (FULL, false),
    }
}

/// `0` or `1` when the intervals decide `x op y`.
fn cmp_iv(op: CmpOp, x: Iv, y: Iv) -> Option<bool> {
    match op {
        CmpOp::Eq => {
            if x.0 == x.1 && y.0 == y.1 && x.0 == y.0 {
                Some(true)
            } else if x.1 < y.0 || y.1 < x.0 {
                Some(false)
            } else {
                None
            }
        }
        CmpOp::Ne => cmp_iv(CmpOp::Eq, x, y).map(|b| !b),
        CmpOp::Lt => {
            if x.1 < y.0 {
                Some(true)
            } else if x.0 >= y.1 {
                Some(false)
            } else {
                None
            }
        }
        CmpOp::Le => {
            if x.1 <= y.0 {
                Some(true)
            } else if x.0 > y.1 {
                Some(false)
            } else {
                None
            }
        }
        CmpOp::Gt => cmp_iv(CmpOp::Lt, y, x),
        CmpOp::Ge => cmp_iv(CmpOp::Le, y, x),
    }
}

/// Is a truth value (non-zero) decided by the interval?
fn truth(x: Iv) -> Option<bool> {
    if x.0 == 0 && x.1 == 0 {
        Some(false)
    } else if x.0 > 0 || x.1 < 0 {
        Some(true)
    } else {
        None
    }
}

fn negate(op: CmpOp) -> CmpOp {
    match op {
        CmpOp::Eq => CmpOp::Ne,
        CmpOp::Ne => CmpOp::Eq,
        CmpOp::Lt => CmpOp::Ge,
        CmpOp::Le => CmpOp::Gt,
        CmpOp::Gt => CmpOp::Le,
        CmpOp::Ge => CmpOp::Lt,
    }
}

/// What a condition register was computed from, within the current block.
#[derive(Clone, Copy)]
enum Cond {
    Cmp(CmpOp, Reg, Reg),
    Not(Reg),
}

/// The state at a point: an interval per tracked register.
type State = Vec<Iv>;

/// The registers the analysis tracks, numbered densely.
struct Tr {
    idx: Vec<u32>,
    k: usize,
    /// Frame offsets of the tracked process variable slots (state entries `k..`).
    slots: Vec<u32>,
    /// The state after a suspension: constants, and each slot at its invariant.
    reset: State,
}

impl Tr {
    fn new(rel: &[bool]) -> Tr {
        let mut k = 0;
        let idx = rel
            .iter()
            .map(|&t| {
                if t {
                    k += 1;
                    k as u32 - 1
                } else {
                    u32::MAX
                }
            })
            .collect();
        Tr {
            idx,
            k,
            slots: Vec::new(),
            reset: Vec::new(),
        }
    }

    fn slot(&self, off: u32) -> Option<usize> {
        self.slots.binary_search(&off).ok().map(|i| self.k + i)
    }

    fn at(&self, r: Reg) -> Option<usize> {
        self.idx
            .get(r.0 as usize)
            .filter(|&&i| i != u32::MAX)
            .map(|&i| i as usize)
    }

    fn get(&self, st: &State, r: Reg) -> Iv {
        self.at(r).map_or(FULL, |i| st[i])
    }
}

impl Opt<'_> {
    /// Run the analysis and apply what it proves; whether the code changed.
    ///
    /// A process is analysed per activation: after a suspension the registers are unknown
    /// (constants apart), and so is every variable slot except for the interval all its
    /// stores, and its zero start, keep it in (found by iterating the solution until the
    /// stored intervals stay inside the assumed ones, widening to everything after a few
    /// rounds).
    pub(super) fn ranges(&mut self) -> bool {
        let nb = self.blocks.len();
        let n = self.defs.len();
        let process = self.f.kind == FuncKind::Process;
        if !(self.nosusp && self.f.body.is_none() && !self.f.flags.contains(FuncFlags::MAY_SUSPEND)
            || process)
            || n.saturating_mul(nb) > BUDGET
        {
            return false;
        }
        let slots = if process {
            self.frame_slots()
        } else {
            Vec::new()
        };
        let mut tracked = Tr::new(&self.relevant(&slots));
        if tracked.k == 0 {
            return false;
        }
        tracked.slots = slots;
        // A register defined once by a `Const` holds its value everywhere (a frame starts
        // with it); the frame's memory starts zeroed.
        let mut base: State = vec![FULL; tracked.k];
        for (r, k) in self.kc.iter().enumerate() {
            if let Some(v) = k
                && let Some(i) = tracked.at(Reg(r as u32))
            {
                base[i] = (*v, *v);
            }
        }
        base.extend(tracked.slots.iter().map(|_| (0, 0)));
        let mut inv: State = base[tracked.k..].to_vec();
        if super::external() {
            // A plugin may deposit into any variable of a process between activations.
            inv.fill(FULL);
        }
        for round in 0.. {
            tracked.reset = base.clone();
            tracked.reset[tracked.k..].copy_from_slice(&inv);
            let Some(inn) = self.solve(&base, &tracked) else {
                return false;
            };
            if tracked.slots.is_empty() {
                return self.apply(&inn, &tracked);
            }
            let mut next = inv.clone();
            self.store_hull(&inn, &tracked, &mut next);
            if next == inv || super::external() {
                return self.apply(&inn, &tracked);
            }
            if round >= 12 {
                return false;
            }
            for (o, n) in inv.iter_mut().zip(next) {
                *o = if round >= 2 { FULL } else { n };
            }
        }
        false
    }

    /// Does the process register no wait condition anywhere? Then each `Suspend` is `wait;`
    /// (a signal, a timeout or a deadline wakes a process only if it was registered).
    pub(super) fn waits_forever(&self) -> bool {
        !self.code.iter().any(|i| {
            !is_dead(i)
                && matches!(
                    i,
                    Inst::WaitOn { .. } | Inst::WaitFor { .. } | Inst::WaitAt { .. }
                )
        })
    }

    /// The hull of the intervals stored into each slot, added to `hull`.
    fn store_hull(&self, inn: &[Option<State>], tracked: &Tr, hull: &mut State) {
        for (b, st) in inn.iter().enumerate() {
            let Some(mut st) = st.clone() else {
                continue;
            };
            for pc in self.range(b) {
                let i = &self.code[pc];
                if is_dead(i) {
                    continue;
                }
                if let Inst::Store { addr, src, .. } = *i
                    && let Some(si) = self.slot_at(tracked, addr)
                {
                    let v = tracked.get(&st, src);
                    let h = &mut hull[si - tracked.k];
                    *h = (h.0.min(v.0), h.1.max(v.1));
                }
                self.step(i, &mut st, tracked);
            }
        }
    }

    /// The state entry of the frame slot a load or store at `addr` accesses, when tracked.
    fn slot_at(&self, tracked: &Tr, addr: Reg) -> Option<usize> {
        match self.ka.get(self.resolve(addr).0 as usize) {
            Some(Some((AddrSpace::Frame, off))) => tracked.slot(*off),
            _ => None,
        }
    }

    /// The frame slots of a process whose every access is a direct 8-byte integer load or
    /// store: no escaping address, no collector root, no debugger.
    fn frame_slots(&mut self) -> Vec<u32> {
        let none = Vec::new();
        if KEEP_VARS.load(std::sync::atomic::Ordering::Relaxed) || self.nested {
            return none;
        }
        let mut acc: std::collections::BTreeMap<u32, bool> = std::collections::BTreeMap::new();
        for i in &self.code {
            if is_dead(i) {
                continue;
            }
            // An address of the frame in a register defined more than once is not followed.
            if let Inst::AddrOf {
                dst,
                space: AddrSpace::Frame,
                ..
            } = *i
                && self.defs.get(dst.0 as usize) != Some(&1)
            {
                return none;
            }
            let (addr, w, r) = match *i {
                Inst::Load { dst, addr, w } => (addr, w, dst),
                Inst::Store { addr, src, w } => (addr, w, src),
                _ => continue,
            };
            let Some(Some((AddrSpace::Frame, off))) = self.ka.get(self.resolve(addr).0 as usize)
            else {
                continue;
            };
            let ok = w == Width::B8 && self.rty(r) == Some(RegTy::I64);
            let e = acc.entry(*off).or_insert(true);
            *e &= ok;
        }
        self.ensure_esc();
        let esc = self.esc.clone().unwrap_or_default();
        let roots: Vec<(i64, i64)> = self
            .f
            .roots
            .iter()
            .map(|&(s, k)| {
                let z = match k {
                    crate::RootKind::Handle | crate::RootKind::Addr => 8,
                    crate::RootKind::Composite(_) => i64::MAX / 4,
                };
                (i64::from(s), z)
            })
            .collect();
        let mut out = Vec::new();
        let mut prev_end = i64::MIN;
        let offs: Vec<(u32, bool)> = acc.into_iter().collect();
        for (k, &(off, ok)) in offs.iter().enumerate() {
            let o = i64::from(off);
            let next = offs.get(k + 1).map_or(i64::MAX, |x| i64::from(x.0));
            let overlap = o < prev_end || o + 8 > next;
            prev_end = prev_end.max(o + 8);
            if ok
                && !overlap
                && clear_of(&esc, o, 8)
                && !roots.iter().any(|&(s, z)| o < s + z && s < o + 8)
            {
                out.push(off);
            }
        }
        out
    }

    /// The solution of the dataflow: the state at the start of each block (`None` for the
    /// blocks nothing reaches). Starts at the entry and, in a process, at the code behind each
    /// suspension that ends a block.
    fn solve(&self, base: &State, tracked: &Tr) -> Option<Vec<Option<State>>> {
        let nb = self.blocks.len();
        let entry = base;
        // Blocks entered by the interpreter after a suspension that ends the block before.
        let mut roots = vec![0usize];
        let perm = self.f.kind == FuncKind::Process && self.waits_forever();
        // (A process paused at the start of its statements can be given values there.)
        if let Some(b) = self.f.body
            && super::external()
        {
            roots.push(b.0 as usize);
        }
        if self.f.kind == FuncKind::Process && !perm {
            for b in 0..nb {
                let (s, e) = self.blocks[b];
                if e > s
                    && self.is_susp(&self.code[e as usize - 1])
                    && let Some(n) = self.blocks.iter().position(|x| x.0 == e && x.1 > e)
                {
                    roots.push(n);
                }
            }
        }
        // Reverse post-order of the blocks reachable from the roots.
        let mut order = Vec::new();
        let mut seen = vec![false; nb];
        for &r in &roots {
            if seen[r] {
                continue;
            }
            let mut stack = vec![(r, 0usize)];
            seen[r] = true;
            while let Some(&(b, k)) = stack.last() {
                if let Some(s) = self.succ(b, k) {
                    if let Some(top) = stack.last_mut() {
                        top.1 += 1;
                    }
                    if !seen[s] {
                        seen[s] = true;
                        stack.push((s, 0));
                    }
                } else {
                    order.push(b);
                    stack.pop();
                }
            }
        }
        order.reverse();
        let mut pos = vec![usize::MAX; nb];
        for (i, &b) in order.iter().enumerate() {
            pos[b] = i;
        }
        let mut inn: Vec<Option<State>> = vec![None; nb];
        let mut visits = vec![0u32; nb];
        // The blocks to (re)visit, as a set ordered by position in the reverse post-order.
        let mut work: std::collections::BTreeSet<usize> = std::collections::BTreeSet::new();
        for &r in &roots {
            inn[r] = Some(if r == 0 {
                entry.clone()
            } else {
                tracked.reset.clone()
            });
            work.insert(pos[r]);
        }
        let mut guard = 0usize;
        while let Some(i) = work.pop_first() {
            guard += 1;
            if guard > 64 * nb + 64 {
                // Cannot happen (widening ends every chain); give up rather than loop.
                return None;
            }
            let b = order[i];
            let Some(mut st) = inn[b].clone() else {
                continue;
            };
            let cond = self.transfer(b, &mut st, tracked);
            // Nothing wakes the process behind a suspension when it waits on nothing.
            if perm
                && self
                    .range(b)
                    .any(|pc| matches!(self.code[pc], Inst::Suspend { .. }))
            {
                continue;
            }
            let mut k = 0;
            while let Some(s) = self.succ(b, k) {
                let refined;
                let out: &State = match cond {
                    Some(c) if self.is_cond_edge(b, k) => {
                        let mut e = st.clone();
                        let live = self.refine_edge(&mut e, c, k == 0, tracked);
                        k += 1;
                        if !live {
                            continue;
                        }
                        refined = e;
                        &refined
                    }
                    _ => {
                        k += 1;
                        &st
                    }
                };
                let changed = match &mut inn[s] {
                    slot @ None => {
                        *slot = Some(out.clone());
                        true
                    }
                    Some(old) => {
                        visits[s] += 1;
                        let widen = visits[s] > WIDEN;
                        let mut ch = false;
                        for (o, n) in old.iter_mut().zip(out.iter()) {
                            let lo = if n.0 < o.0 {
                                if widen { i64::MIN } else { n.0 }
                            } else {
                                o.0
                            };
                            let hi = if n.1 > o.1 {
                                if widen { i64::MAX } else { n.1 }
                            } else {
                                o.1
                            };
                            if (lo, hi) != *o {
                                *o = (lo, hi);
                                ch = true;
                            }
                        }
                        ch
                    }
                };
                if changed && pos[s] != usize::MAX {
                    work.insert(pos[s]);
                }
            }
        }
        Some(inn)
    }

    /// The integer registers the analysis can use: those a check, a compare, a boolean
    /// operation or a branch reads, and what they are computed from. The others stay the full
    /// range, which keeps the states small.
    fn relevant(&self, slots: &[u32]) -> Vec<bool> {
        let n = self.defs.len();
        let mut rel = vec![false; n];
        let ok = |r: Reg, rel: &mut Vec<bool>| {
            let i = r.0 as usize;
            if self.rty(r) == Some(RegTy::I64) && !rel[i] {
                rel[i] = true;
                true
            } else {
                false
            }
        };
        for i in &self.code {
            match *i {
                Inst::Check {
                    kind, v, lo, hi, ..
                } => {
                    ok(v, &mut rel);
                    if matches!(
                        kind,
                        CheckKind::Range | CheckKind::Index | CheckKind::Length
                    ) {
                        ok(lo, &mut rel);
                    }
                    if matches!(kind, CheckKind::Range | CheckKind::Index) {
                        ok(hi, &mut rel);
                    }
                }
                Inst::BinRange { a, b, lo, hi, .. } => {
                    for r in [a, b, lo, hi] {
                        ok(r, &mut rel);
                    }
                }
                Inst::Cmp { a, b, .. } | Inst::Bool { a, b, .. } => {
                    ok(a, &mut rel);
                    ok(b, &mut rel);
                }
                Inst::CondBr { c, .. } => {
                    ok(c, &mut rel);
                }
                Inst::Load { dst: r, addr, .. } | Inst::Store { src: r, addr, .. }
                    if !slots.is_empty() && self.is_slot(slots, addr) =>
                {
                    ok(r, &mut rel);
                }
                _ => {}
            }
        }
        // What a relevant register is computed from.
        loop {
            let mut grew = false;
            for i in &self.code {
                if is_dead(i) {
                    continue;
                }
                let srcs: &[Reg] = &match *i {
                    Inst::Move { dst, src } if rel[dst.0 as usize] => vec![src],
                    Inst::Select { dst, a, b, .. } if rel[dst.0 as usize] => vec![a, b],
                    Inst::Bin { dst, a, b, .. } if rel[dst.0 as usize] => vec![a, b],
                    Inst::BinRange {
                        dst, a, b, lo, hi, ..
                    } if rel[dst.0 as usize] => {
                        vec![a, b, lo, hi]
                    }
                    Inst::Un { dst, a, .. } if rel[dst.0 as usize] => vec![a],
                    _ => continue,
                };
                for &r in srcs {
                    grew |= ok(r, &mut rel);
                }
            }
            if !grew {
                break;
            }
        }
        rel
    }

    fn is_slot(&self, slots: &[u32], addr: Reg) -> bool {
        matches!(
            self.ka.get(self.resolve(addr).0 as usize),
            Some(Some((AddrSpace::Frame, off))) if slots.binary_search(off).is_ok()
        )
    }

    /// Does block `b` end in a `CondBr` (whose two edges carry different facts)?
    fn is_cond_edge(&self, b: usize, k: usize) -> bool {
        let (s, e) = self.blocks[b];
        e > s && k < 2 && matches!(self.code[e as usize - 1], Inst::CondBr { .. })
    }

    /// Narrow `st` for the edge of a branch on `cond`: the true edge when `taken`. Whether
    /// the edge can be taken at all.
    fn refine_edge(&self, st: &mut State, cond: Cond, taken: bool, tracked: &Tr) -> bool {
        match cond {
            Cond::Not(c) => {
                // `c` is a boolean: its truth is the opposite on this edge.
                let Some(c) = tracked.at(c) else {
                    return true;
                };
                let want = if taken { (0, 0) } else { (1, i64::MAX) };
                let r = (st[c].0.max(want.0), st[c].1.min(want.1));
                if r.0 > r.1 {
                    return false;
                }
                st[c] = r;
                true
            }
            Cond::Cmp(op, a, b) => {
                let op = if taken { op } else { negate(op) };
                let (Some(ai), Some(bi)) = (tracked.at(a), tracked.at(b)) else {
                    return true;
                };
                if ai == bi {
                    return true;
                }
                let (x, y) = (st[ai], st[bi]);
                let (nx, ny) = match op {
                    CmpOp::Eq => {
                        let m = (x.0.max(y.0), x.1.min(y.1));
                        (m, m)
                    }
                    CmpOp::Ne => {
                        let shrink = |v: Iv, o: Iv| {
                            if o.0 == o.1 && v.0 == o.0 && v.0 < v.1 {
                                (v.0 + 1, v.1)
                            } else if o.0 == o.1 && v.1 == o.0 && v.0 < v.1 {
                                (v.0, v.1 - 1)
                            } else {
                                v
                            }
                        };
                        (shrink(x, y), shrink(y, x))
                    }
                    CmpOp::Lt => (
                        (x.0, x.1.min(y.1.saturating_sub(1))),
                        (y.0.max(x.0.saturating_add(1)), y.1),
                    ),
                    CmpOp::Le => ((x.0, x.1.min(y.1)), (y.0.max(x.0), y.1)),
                    CmpOp::Gt => (
                        (x.0.max(y.0.saturating_add(1)), x.1),
                        (y.0, y.1.min(x.1.saturating_sub(1))),
                    ),
                    CmpOp::Ge => ((x.0.max(y.0), x.1), (y.0, y.1.min(x.1))),
                };
                if nx.0 > nx.1 || ny.0 > ny.1 {
                    return false;
                }
                st[ai] = nx;
                st[bi] = ny;
                true
            }
        }
    }

    /// Run block `b` over `st`. Returns what the block's final conditional branch tests,
    /// when that is a comparison (or the negation of a boolean) computed in the block from
    /// registers it did not change afterwards.
    fn transfer(&self, b: usize, st: &mut State, tracked: &Tr) -> Option<Cond> {
        let mut conds: Vec<(Reg, Cond)> = Vec::new();
        let mut last = None;
        for pc in self.range(b) {
            let i = &self.code[pc];
            if is_dead(i) {
                continue;
            }
            self.step(i, st, tracked);
            if self.is_susp(i) {
                conds.clear();
            }
            // Forget the conditions this instruction invalidates, then note its own.
            let mut defs = [Reg(u32::MAX); 4];
            let mut nd = 0;
            for_defs(i, |r| {
                if nd < defs.len() {
                    defs[nd] = r;
                    nd += 1;
                }
            });
            if nd > 0 {
                conds.retain(|&(d, c)| {
                    !defs[..nd].contains(&d)
                        && match c {
                            Cond::Cmp(_, x, y) => {
                                !defs[..nd].contains(&x) && !defs[..nd].contains(&y)
                            }
                            Cond::Not(x) => !defs[..nd].contains(&x),
                        }
                });
            }
            match *i {
                Inst::Cmp { op, dst, a, b } if a != dst && b != dst => {
                    conds.push((dst, Cond::Cmp(op, a, b)));
                }
                Inst::Bool {
                    op: BoolOp::Not,
                    dst,
                    a,
                    ..
                } if a != dst => conds.push((dst, Cond::Not(a))),
                Inst::CondBr { c, .. } => {
                    last = conds.iter().rev().find(|e| e.0 == c).map(|e| e.1);
                }
                _ => {}
            }
        }
        last
    }

    /// The effect of one instruction on the intervals.
    fn step(&self, i: &Inst, st: &mut State, tracked: &Tr) {
        let get = |st: &State, r: Reg| tracked.get(st, r);
        let set = |st: &mut State, r: Reg, v: Iv| {
            if let Some(i) = tracked.at(r) {
                st[i] = v;
            }
        };
        if self.is_susp(i) {
            st.clone_from(&tracked.reset);
        }
        match *i {
            Inst::Const { dst, v } => set(st, dst, (v, v)),
            Inst::Move { dst, src } => {
                let v = get(st, src);
                set(st, dst, v);
            }
            Inst::Select { dst, a, b, .. } => {
                let (x, y) = (get(st, a), get(st, b));
                set(st, dst, (x.0.min(y.0), x.1.max(y.1)));
            }
            Inst::Bin { op, dst, a, b } => {
                let v = bin_iv(op, get(st, a), get(st, b)).0;
                set(st, dst, v);
            }
            Inst::BinRange {
                op,
                dst,
                a,
                b,
                lo,
                hi,
                ..
            } => {
                let r = bin_iv(op, get(st, a), get(st, b)).0;
                let v = meet(r, (get(st, lo).0, get(st, hi).1));
                set(st, dst, v);
            }
            Inst::Un { op, dst, a } => {
                let x = get(st, a);
                let v = match op {
                    IntUnOp::Neg if x.0 > i64::MIN => (-x.1, -x.0),
                    IntUnOp::Abs if x.0 > i64::MIN => {
                        if x.0 >= 0 {
                            x
                        } else if x.1 <= 0 {
                            (-x.1, -x.0)
                        } else {
                            (0, x.1.max(-x.0))
                        }
                    }
                    _ => FULL,
                };
                set(st, dst, v);
            }
            Inst::Cmp { op, dst, a, b } => {
                let v = match cmp_iv(op, get(st, a), get(st, b)) {
                    Some(t) => (i64::from(t), i64::from(t)),
                    None => (0, 1),
                };
                set(st, dst, v);
            }
            Inst::Bool { op, dst, a, b } => {
                let (x, y) = (truth(get(st, a)), truth(get(st, b)));
                let t = match op {
                    BoolOp::Not => x.map(|x| !x),
                    BoolOp::And => match (x, y) {
                        (Some(false), _) | (_, Some(false)) => Some(false),
                        (Some(true), Some(true)) => Some(true),
                        _ => None,
                    },
                    BoolOp::Or => match (x, y) {
                        (Some(true), _) | (_, Some(true)) => Some(true),
                        (Some(false), Some(false)) => Some(false),
                        _ => None,
                    },
                    BoolOp::Xor => match (x, y) {
                        (Some(x), Some(y)) => Some(x != y),
                        _ => None,
                    },
                };
                let v = t.map_or((0, 1), |t| (i64::from(t), i64::from(t)));
                set(st, dst, v);
            }
            Inst::DopeLen { dst, .. } => set(st, dst, (0, i64::MAX)),
            Inst::Load { dst, addr, .. } if self.slot_at(tracked, addr).is_some() => {
                let si = self.slot_at(tracked, addr).unwrap_or(0);
                let v = st[si];
                set(st, dst, v);
            }
            Inst::Store { addr, src, .. } if self.slot_at(tracked, addr).is_some() => {
                let si = self.slot_at(tracked, addr).unwrap_or(0);
                st[si] = get(st, src);
            }
            Inst::Load { dst, w, .. } => {
                let v = match w {
                    Width::B1 => (0, 255),
                    Width::B4 => (i64::from(i32::MIN), i64::from(i32::MAX)),
                    _ => FULL,
                };
                set(st, dst, v);
            }
            Inst::Check {
                kind, v, lo, hi, ..
            } => {
                let x = get(st, v);
                let r = match kind {
                    CheckKind::Range | CheckKind::Index => meet(x, (get(st, lo).0, get(st, hi).1)),
                    CheckKind::Length => meet(x, get(st, lo)),
                    CheckKind::Null => {
                        if x.0 == 0 && x.1 > 0 {
                            (1, x.1)
                        } else if x.1 == 0 && x.0 < 0 {
                            (x.0, -1)
                        } else {
                            x
                        }
                    }
                };
                set(st, v, r);
            }
            _ => for_defs(i, |r| set(st, r, FULL)),
        }
    }

    /// Rewrite what the solution proves; whether anything changed.
    #[allow(clippy::needless_range_loop)]
    fn apply(&mut self, inn: &[Option<State>], tracked: &Tr) -> bool {
        let mut any = false;
        for b in 0..self.blocks.len() {
            let Some(mut st) = inn[b].clone() else {
                continue;
            };
            for pc in self.range(b) {
                let i = self.code[pc];
                if is_dead(&i) {
                    continue;
                }
                let get = |r: Reg| tracked.get(&st, r);
                let new = match i {
                    Inst::Check {
                        kind, v, lo, hi, ..
                    } => {
                        let (x, l, h) = (get(v), get(lo), get(hi));
                        let ok = match kind {
                            CheckKind::Range | CheckKind::Index => x.0 >= l.1 && x.1 <= h.0,
                            CheckKind::Length => x.0 == x.1 && l.0 == l.1 && x.0 == l.0,
                            CheckKind::Null => truth(x) == Some(true),
                        };
                        ok.then(dead)
                    }
                    Inst::BinRange {
                        op,
                        dst,
                        a,
                        b,
                        lo,
                        hi,
                        ..
                    } => {
                        let (r, fits) = bin_iv(op, get(a), get(b));
                        (fits && r.0 >= get(lo).1 && r.1 <= get(hi).0).then_some(Inst::Bin {
                            op,
                            dst,
                            a,
                            b,
                        })
                    }
                    Inst::Cmp { op, dst, a, b } => {
                        cmp_iv(op, get(a), get(b)).map(|t| Inst::Const {
                            dst,
                            v: i64::from(t),
                        })
                    }
                    Inst::CondBr { c, t, f } => truth(get(c)).map(|c| Inst::Br {
                        to: if c { t } else { f },
                    }),
                    _ => None,
                };
                self.step(&i, &mut st, tracked);
                if let Some(new) = new {
                    self.code[pc] = new;
                    any = true;
                }
            }
        }
        any |= self.thread(inn, tracked);
        if any {
            self.changed = true;
        }
        any
    }

    /// A branch to a block that is nothing but a conditional branch, where the intervals
    /// decide the condition on that edge, goes straight to the successor it would pick (the
    /// merge of `a and then b` tests a flag the predecessors just set).
    #[allow(clippy::needless_range_loop)]
    fn thread(&mut self, inn: &[Option<State>], tracked: &Tr) -> bool {
        let nb = self.blocks.len();
        let mut only: Vec<Option<(Reg, BlockId, BlockId)>> = vec![None; nb];
        for (b, slot) in only.iter_mut().enumerate() {
            let (s, e) = self.blocks[b];
            if e == s + 1
                && self.f.body != Some(BlockId(b as u32))
                && let Inst::CondBr { c, t, f } = self.code[s as usize]
                && t != f
            {
                *slot = Some((c, t, f));
            }
        }
        if only.iter().all(Option::is_none) {
            return false;
        }
        let mut any = false;
        for p in 0..nb {
            let Some(mut st) = inn[p].clone() else {
                continue;
            };
            let (s, e) = self.blocks[p];
            if e <= s {
                continue;
            }
            let last = e as usize - 1;
            let reaches = |t: BlockId| only.get(t.0 as usize).is_some_and(Option::is_some);
            let any_only = match self.code[last] {
                Inst::Br { to } => reaches(to),
                Inst::CondBr { t, f, .. } => reaches(t) || reaches(f),
                _ => false,
            };
            if !any_only {
                continue;
            }
            let cond = self.transfer(p, &mut st, tracked);
            // The target an edge would reach: where the single branch of `to` goes.
            let edge = |this: &Self, st: &State, to: BlockId, taken: Option<bool>| {
                let (c, t, f) = only.get(to.0 as usize).copied().flatten()?;
                if to.0 as usize == p {
                    return None;
                }
                let mut es = st.clone();
                if let (Some(cd), Some(k)) = (cond, taken)
                    && !this.refine_edge(&mut es, cd, k, tracked)
                {
                    return None;
                }
                truth(tracked.get(&es, c)).map(|v| if v { t } else { f })
            };
            match self.code[last] {
                Inst::Br { to } => {
                    if let Some(n) = edge(self, &st, to, None) {
                        self.code[last] = Inst::Br { to: n };
                        any = true;
                    }
                }
                Inst::CondBr { c, t, f } => {
                    let nt = edge(self, &st, t, Some(true)).unwrap_or(t);
                    let nf = edge(self, &st, f, Some(false)).unwrap_or(f);
                    if (nt, nf) != (t, f) {
                        self.code[last] = if nt == nf {
                            Inst::Br { to: nt }
                        } else {
                            Inst::CondBr { c, t: nt, f: nf }
                        };
                        any = true;
                    }
                }
                _ => {}
            }
        }
        any
    }
}
