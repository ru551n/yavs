//! Loop-invariant code motion in a subprogram that cannot suspend (#526) and in the loops of a
//! process that hold no suspension (#541).
//!
//! Index and descriptor arithmetic of nested array loops (`r(i, j) := m(i)(j)`) is mostly the
//! same on every pass of the inner loop: the position of `i`, the element descriptor, the
//! lengths and their products. Such an instruction moves to a preheader, a block run once
//! before the loop is entered.
//!
//! Almost every instruction can fail (a position out of range, checked arithmetic), and a
//! failure must be the one the unmoved code gives, with its message, after the same effects.
//! So an instruction that can fail moves only from the run of instructions at the start of
//! the loop header that has no effect (the header runs whenever the loop is entered, and on
//! its first pass nothing before it was visible), and the preheader repeats that run as the
//! first pass would execute it: the moved instructions in their places, between copies of the
//! instructions that stay in the loop and can fail (with fresh results). Whatever fails first
//! on the first pass fails first in the preheader, the copies cost one run per entry. An
//! instruction that cannot fail and reads no memory (a compare, a select, a constant) moves
//! from anywhere in the loop, and so does a load from read-only memory (instance data, the
//! constant pool) at a constant address. In a loop that writes only the elements of logic
//! arrays (a fill or copy loop), loads, dereferences of handles and descriptor reads are
//! invariant when their operands are, and move from the start of the header like the rest
//! (#768).
//!
//! Loops are the natural loops of the back edges a depth-first walk finds, with a single entry
//! (the header); inner loops go first, so what moved into an inner preheader can leave the
//! enclosing loop too. Linear in the code, times the loop depth.

// Loops index several parallel per-block vectors by position.
#![allow(clippy::needless_range_loop)]

use super::*;

/// Can it neither fail nor read memory: safe to run where the loop did not run it.
fn speculable(i: &Inst) -> bool {
    match *i {
        Inst::AddrOf { space, .. } => !matches!(space, AddrSpace::Up(_)),
        Inst::Const { .. }
        | Inst::ConstF { .. }
        | Inst::Move { .. }
        | Inst::Select { .. }
        | Inst::Cmp { .. }
        | Inst::FCmp { .. }
        | Inst::Bool { .. }
        | Inst::IToF { .. } => true,
        _ => false,
    }
}

/// Does it read a descriptor (which a write in the loop could change)?
fn reads_dope(i: &Inst) -> bool {
    matches!(
        i,
        Inst::Pos { .. } | Inst::SlicePos { .. } | Inst::DopeLen { .. }
    )
}

/// Does it read memory a write in the loop could change: a descriptor, a word or the object a
/// handle names? (Writes into the elements of a logic array reach none of them, #768.)
fn reads_mem(i: &Inst) -> bool {
    reads_dope(i)
        || matches!(i, Inst::Deref { .. })
        || matches!(*i, Inst::Load { w, .. } if w != Width::Sig)
}

/// May it move out of the start of the header: a computation whose result depends only on
/// its operands (and, for a descriptor read, on a descriptor nothing in the loop writes).
fn movable(i: &Inst) -> bool {
    speculable(i)
        || matches!(
            i,
            // (A frame up the static links is the same for the whole run of a loop; the
            // address of one is not run where the loop did not, as `speculable` asks.)
            Inst::AddrOf {
                space: AddrSpace::Up(_),
                ..
            } | Inst::Bin { .. }
                | Inst::BinRange { .. }
                | Inst::Un { .. }
                | Inst::Index { .. }
                | Inst::Check { .. }
        )
        || reads_mem(i)
}

/// Can the preheader run it again with the values of the first pass: no effect, at most one
/// result, no operand list.
fn replayable(i: &Inst) -> bool {
    movable(i)
        || matches!(*i, Inst::Load { w, .. } if w != Width::Sig)
        || matches!(
            i,
            Inst::LGet { .. } | Inst::Field { .. } | Inst::Deref { .. }
        )
}

/// Can it fail other than by a broken invariant (a copy that cannot is dropped unless read)?
/// (The length of a descriptor that is there cannot: it moves past the checks before it, #771.)
fn may_fail(i: &Inst) -> bool {
    !speculable(i)
        && !matches!(
            i,
            Inst::Index { .. }
                | Inst::DopeLen { .. }
                | Inst::Load { .. }
                | Inst::LGet { .. }
                | Inst::Field { .. }
                | Inst::AddrOf { .. }
        )
}

/// Does it write memory a descriptor could be in? Writes into the elements of a logic array
/// never reach a descriptor, nor does a new temporary block, a statement marker or an `ieee`
/// function (which reads its operands and makes a block of its own, #771).
fn writes(prog: &Program, i: &Inst) -> bool {
    !(replayable(i)
        || matches!(
            i,
            Inst::Br { .. }
                | Inst::CondBr { .. }
                | Inst::Case { .. }
                | Inst::Ret { .. }
                | Inst::Unreachable { .. }
                | Inst::LSet { .. }
                | Inst::LFill { .. }
                | Inst::LCopy { .. }
                | Inst::Alloc { .. }
                | Inst::Cover { .. }
        )
        // (A debugger may write a variable at a marker.)
        || matches!(i, Inst::Stmt) && !keep_variables_on()
        || matches!(*i, Inst::CallNative { native, .. } if light_native(prog, native)))
}

/// A native `ieee` function: it reads its arguments and returns a new block, touching no other
/// memory.
fn light_native(prog: &Program, native: crate::NativeId) -> bool {
    let Some(n) = prog.natives.get(native) else {
        return false;
    };
    let Some(key) = n.symbol.strip_prefix("yavs.ieee.") else {
        return false;
    };
    !matches!(
        crate::ieee::parse(key.split_once('@').map_or(key, |k| k.0)),
        None | Some(crate::ieee::Op::Uniform)
    )
}

fn def_of(i: &Inst) -> Option<Reg> {
    let mut d = None;
    i.visit(&mut |r, role, _| {
        if role == Role::Def {
            d = Some(r);
        }
    });
    d
}

struct Licm<'a> {
    prog: &'a Program,
    code: Vec<Vec<Inst>>,
    loc: Vec<Vec<StmtId>>,
    preds: Vec<Vec<u32>>,
    reg_ty: Vec<RegTy>,
    defs: Vec<u8>,
    /// Per register: the tag of the loop being looked at when that defines it.
    ldef: Vec<u32>,
    /// Per block: the tag of the last walk that reached it.
    member: Vec<u32>,
    /// The last tag handed out (blocks and registers of the loop being looked at carry it).
    tags: u32,
    /// Per register: defined once, by the address of read-only memory (the constant pool, the
    /// instance data) at a constant offset.
    ro: Vec<bool>,
}

impl Licm<'_> {
    /// A load from read-only memory at a constant address: it cannot fail and its value is the
    /// same wherever it runs.
    fn ro_load(&self, i: &Inst) -> bool {
        matches!(*i, Inst::Load { addr, w, .. }
            if w != Width::Sig && self.ro.get(addr.0 as usize).copied().unwrap_or(false))
    }

    /// Does the loop of these blocks suspend (a register may not live across a suspension)?
    fn suspends(&self, body: &[u32]) -> bool {
        body.iter().any(|&b| {
            self.code[b as usize].iter().any(|i| match *i {
                Inst::Suspend { .. } => true,
                Inst::Call { func, .. } => self
                    .prog
                    .funcs
                    .get(func)
                    .is_none_or(|c| c.flags.contains(FuncFlags::MAY_SUSPEND)),
                _ => false,
            })
        })
    }

    fn succs(&self, b: usize) -> Vec<u32> {
        match self.code[b].last() {
            Some(&Inst::Case { table, .. }) => match self.prog.cases.get(table) {
                Some(t) => std::iter::once(t.default)
                    .chain(t.arms.iter().map(|a| a.2))
                    .map(|x| x.0)
                    .collect(),
                None => Vec::new(),
            },
            Some(i) => i.targets().iter().map(|x| x.0).collect(),
            None => Vec::new(),
        }
    }

    /// The back edges (latch, header) of a depth-first walk from the entry.
    fn back_edges(&self) -> Vec<(u32, u32)> {
        let nb = self.code.len();
        // 0 unseen, 1 on the stack, 2 done.
        let mut state = vec![0u8; nb];
        let mut out = Vec::new();
        let mut stack: Vec<(u32, Vec<u32>, usize)> = vec![(0, self.succs(0), 0)];
        state[0] = 1;
        while let Some((b, ss, k)) = stack.last_mut() {
            if let Some(&s) = ss.get(*k) {
                *k += 1;
                let b = *b;
                match state.get(s as usize) {
                    Some(0) => {
                        state[s as usize] = 1;
                        let next = self.succs(s as usize);
                        stack.push((s, next, 0));
                    }
                    Some(1) => out.push((b, s)),
                    _ => {}
                }
            } else {
                state[*b as usize] = 2;
                stack.pop();
            }
        }
        out
    }

    /// The blocks that reach one of `latches` without passing `h`, and `h`, marked with a new
    /// tag (returned).
    fn walk(&mut self, h: u32, latches: &[u32]) -> (u32, Vec<u32>) {
        self.tags += 1;
        let tag = self.tags;
        let mut body = vec![h];
        self.member[h as usize] = tag;
        let mut work: Vec<u32> = Vec::new();
        for &l in latches {
            if self.member[l as usize] != tag {
                self.member[l as usize] = tag;
                body.push(l);
                work.push(l);
            }
        }
        while let Some(b) = work.pop() {
            for k in 0..self.preds[b as usize].len() {
                let p = self.preds[b as usize][k];
                if self.member[p as usize] != tag {
                    self.member[p as usize] = tag;
                    body.push(p);
                    work.push(p);
                }
            }
        }
        (tag, body)
    }

    /// Is `h` the only way into the loop (and not the function's entry)?
    fn single_entry(&self, h: u32, body: &[u32], tag: u32) -> bool {
        body.iter().all(|&b| {
            b != 0
                && (b == h
                    || self.preds[b as usize]
                        .iter()
                        .all(|&p| self.member[p as usize] == tag))
        })
    }

    /// Can the branch at the end of `o` be pointed elsewhere?
    fn plain_branch(&self, o: u32) -> bool {
        matches!(
            self.code[o as usize].last(),
            Some(Inst::Br { .. } | Inst::CondBr { .. })
        )
    }

    /// A new block of these instructions; its predecessors are `preds`, whose branches to `h`
    /// now go to it.
    fn new_block(&mut self, code: Vec<Inst>, loc: Vec<StmtId>, h: u32, preds: Vec<u32>) -> u32 {
        let p = self.code.len() as u32;
        self.code.push(code);
        self.loc.push(loc);
        self.member.push(0);
        for &o in &preds {
            if let Some(t) = self.code[o as usize].last_mut() {
                match t {
                    Inst::Br { to } if to.0 == h => *to = BlockId(p),
                    Inst::CondBr { t, f, .. } => {
                        if t.0 == h {
                            *t = BlockId(p);
                        }
                        if f.0 == h {
                            *f = BlockId(p);
                        }
                    }
                    _ => {}
                }
            }
        }
        self.preds[h as usize].retain(|q| !preds.contains(q));
        self.preds[h as usize].push(p);
        self.preds.push(preds);
        p
    }

    /// Loops that share a header (an outer loop whose body starts with the inner loop) get a
    /// header each: the entry and the outer back edges go through a new block that jumps to
    /// the shared one. Returns whether a block was made.
    fn split_shared_headers(&mut self) -> bool {
        let mut loops: FxMap<u32, Vec<u32>> = FxMap::default();
        for (latch, h) in self.back_edges() {
            loops.entry(h).or_default().push(latch);
        }
        let mut any = false;
        for (h, latches) in loops {
            if latches.len() < 2 {
                continue;
            }
            // The innermost: the latch with the smallest loop, which holds no other latch. The
            // loops of the others need not hold it (the inner loop starts the outer one's body:
            // the outer back edge reaches the header around it), so this is a guess; a wrong
            // one costs a jump and moves less.
            let mut best: Option<(usize, usize)> = None;
            let mut tie = false;
            for (k, &l) in latches.iter().enumerate() {
                let (tag, body) = self.walk(h, &[l]);
                if latches
                    .iter()
                    .any(|&x| x != l && self.member[x as usize] == tag)
                {
                    continue;
                }
                match best {
                    Some((_, n)) if n < body.len() => {}
                    Some((_, n)) if n == body.len() => tie = true,
                    _ => {
                        best = Some((k, body.len()));
                        tie = false;
                    }
                }
            }
            let inner = best.filter(|_| !tie).map(|b| b.0);
            let Some(k) = inner else {
                continue;
            };
            let others: Vec<u32> = self.preds[h as usize]
                .iter()
                .copied()
                .filter(|&p| p != latches[k])
                .collect();
            if others.iter().any(|&o| !self.plain_branch(o)) {
                continue;
            }
            let s = self.loc[h as usize].first().copied().unwrap_or(StmtId(0));
            self.new_block(vec![Inst::Br { to: BlockId(h) }], vec![s], h, others);
            any = true;
        }
        any
    }

    /// Does `i` define no register, or one register of one cell (a signal reference takes two)?
    fn one_cell(&self, i: &Inst) -> bool {
        def_of(i).is_none_or(|d| self.reg_ty[d.0 as usize] != RegTy::Sig)
    }

    fn uses_invariant(&self, i: &Inst, tag: u32, operands: &[Reg]) -> bool {
        let mut ok = true;
        i.visit(&mut |r, role, _| {
            if role == Role::Use && self.ldef[r.0 as usize] == tag {
                ok = false;
            }
        });
        ok && operands.iter().all(|r| self.ldef[r.0 as usize] != tag)
    }

    fn fresh(&mut self, like: Reg) -> Reg {
        let r = Reg(self.reg_ty.len() as u32);
        self.reg_ty.push(self.reg_ty[like.0 as usize]);
        self.defs.push(1);
        self.ldef.push(0);
        r
    }

    /// Hoist what the loop of `h` allows; returns whether anything moved.
    fn hoist(&mut self, f: &crate::Func, h: u32, latches: &[u32]) -> bool {
        let (tag, body) = self.walk(h, latches);
        if !self.single_entry(h, &body, tag) || self.suspends(&body) {
            return false;
        }
        let outside: Vec<u32> = self.preds[h as usize]
            .iter()
            .copied()
            .filter(|&p| self.member[p as usize] != tag)
            .collect();
        if outside.is_empty() || outside.iter().any(|&o| !self.plain_branch(o)) {
            return false;
        }
        let mut clean = true;
        for &b in &body {
            for i in &self.code[b as usize] {
                clean &= !writes(self.prog, i);
                i.visit(&mut |r, role, _| {
                    if role == Role::Def {
                        self.ldef[r.0 as usize] = tag;
                    }
                });
            }
        }
        let ops = |i: &Inst| -> &[Reg] { i.list().and_then(|l| f.regs(l)).unwrap_or(&[]) };
        // The start of the header: moved instructions and the ones that stay. The first pass is
        // repeated up to the last moved instruction that can fail, which pays only where fewer
        // copies are made than instructions leave the loop: past the best such point (`cap`)
        // those stay.
        let hc = self.code[h as usize].clone();
        let n = hc.len().saturating_sub(1);
        let mut moved = vec![false; hc.len()];
        let mut prefix = 0;
        let mut last = None;
        let mut cap = usize::MAX;
        for _ in 0..2 {
            let (mut gain, mut best, mut fail_moved) = (0i64, (0i64, None), None);
            prefix = 0;
            while prefix < n && replayable(&hc[prefix]) && self.one_cell(&hc[prefix]) {
                let i = &hc[prefix];
                let d = def_of(i);
                let fails = may_fail(i);
                if (movable(i) || self.ro_load(i))
                    && (!fails || prefix < cap)
                    && (clean || !reads_mem(i) || self.ro_load(i))
                    && d.is_none_or(|d| self.defs[d.0 as usize] == 1)
                    && self.uses_invariant(i, tag, ops(i))
                {
                    moved[prefix] = true;
                    gain += 1;
                    if fails {
                        fail_moved = Some(prefix);
                        if gain > best.0 {
                            best = (gain, Some(prefix));
                        }
                    }
                    if let Some(d) = d {
                        self.ldef[d.0 as usize] = 0;
                    }
                } else if fails {
                    gain -= 1;
                }
                prefix += 1;
            }
            last = best.1;
            if last == fail_moved {
                break;
            }
            // Again, with the ones that can fail past the best point left in the loop.
            cap = last.map_or(0, |k| k + 1);
            for k in 0..prefix {
                if moved[k] {
                    moved[k] = false;
                    if let Some(d) = def_of(&hc[k]) {
                        self.ldef[d.0 as usize] = tag;
                    }
                }
            }
        }
        // The first pass up to the last moved instruction that can fail, copies with fresh
        // results, then the other moved ones.
        let mut pre: Vec<(Inst, StmtId, bool)> = Vec::new();
        {
            let mut map: FxMap<Reg, Reg> = FxMap::default();
            let hl = self.loc[h as usize].clone();
            for k in 0..prefix {
                if moved[k] {
                    pre.push((hc[k], hl[k], false));
                    continue;
                }
                if last.is_none_or(|l| k > l) {
                    continue;
                }
                let mut c = hc[k];
                let mut def = None;
                c.visit_mut(&mut |r, role, _| match role {
                    Role::Use => {
                        if let Some(&x) = map.get(r) {
                            *r = x;
                        }
                    }
                    Role::Def => def = Some(*r),
                });
                if let Some(d) = def {
                    let x = self.fresh(d);
                    c.visit_mut(&mut |r, role, _| {
                        if role == Role::Def {
                            *r = x;
                        }
                    });
                    map.insert(d, x);
                }
                pre.push((c, hl[k], true));
            }
            // Drop the copies that cannot fail and that no kept copy reads.
            let mut need: FxMap<Reg, ()> = FxMap::default();
            for k in (0..pre.len()).rev() {
                let (i, _, copy) = pre[k];
                let keep =
                    !copy || may_fail(&i) || def_of(&i).is_some_and(|d| need.contains_key(&d));
                if keep {
                    i.visit(&mut |r, role, _| {
                        if role == Role::Use {
                            need.insert(r, ());
                        }
                    });
                } else {
                    pre[k].2 = false;
                    pre[k].0 = dead();
                }
            }
            pre.retain(|p| !is_dead(&p.0));
        }
        // Instructions that can neither fail nor read memory move from anywhere in the loop.
        let mut spec: Vec<(u32, usize)> = Vec::new();
        for &b in &body {
            let from = if b == h { prefix } else { 0 };
            let len = self.code[b as usize].len();
            for k in from..len {
                let i = &self.code[b as usize][k];
                if (speculable(i) || self.ro_load(i))
                    && self.one_cell(i)
                    && def_of(i).is_some_and(|d| self.defs[d.0 as usize] == 1)
                    && self.uses_invariant(i, tag, &[])
                {
                    if let Some(d) = def_of(i) {
                        self.ldef[d.0 as usize] = 0;
                    }
                    spec.push((b, k));
                }
            }
        }
        if pre.is_empty() && spec.is_empty() {
            return false;
        }
        for &(b, k) in &spec {
            pre.push((self.code[b as usize][k], self.loc[b as usize][k], false));
            self.code[b as usize][k] = dead();
        }
        for k in 0..hc.len() {
            if moved[k] {
                self.code[h as usize][k] = dead();
            }
        }
        // The preheader: the one outside predecessor when it only jumps here, else a new block.
        if let [o] = outside[..]
            && matches!(self.code[o as usize].last(), Some(Inst::Br { .. }))
        {
            let at = self.code[o as usize].len() - 1;
            self.code[o as usize].splice(at..at, pre.iter().map(|p| p.0));
            self.loc[o as usize].splice(at..at, pre.iter().map(|p| p.1));
            return true;
        }
        let s = self.loc[h as usize].first().copied().unwrap_or(StmtId(0));
        let (mut c, mut l): (Vec<Inst>, Vec<StmtId>) =
            pre.into_iter().map(|(i, s, _)| (i, s)).unzip();
        c.push(Inst::Br { to: BlockId(h) });
        l.push(s);
        self.new_block(c, l, h, outside);
        true
    }
}

/// Move loop-invariant code of `f` into preheaders; returns whether the code changed.
pub(super) fn run(prog: &Program, f: &mut crate::Func) -> bool {
    if !f.via.is_empty() || keep_variables_on() || !cfg::has_back_edge(f) {
        return false;
    }
    let nb = f.blocks.len();
    let (code, loc): (Vec<Vec<Inst>>, Vec<Vec<StmtId>>) = f
        .blocks
        .values()
        .iter()
        .map(|b| {
            let r = b.start as usize..b.end as usize;
            (f.code[r.clone()].to_vec(), f.loc[r].to_vec())
        })
        .unzip();
    let mut defs = vec![0u8; f.reg_ty.len()];
    let mut ro = vec![false; f.reg_ty.len()];
    for i in &f.code {
        i.visit(&mut |r, role, _| {
            if role == Role::Def {
                defs[r.0 as usize] = defs[r.0 as usize].saturating_add(1);
            }
        });
        if let Inst::AddrOf {
            dst,
            space: AddrSpace::Inst | AddrSpace::Const(_),
            ..
        } = *i
        {
            ro[dst.0 as usize] = true;
        }
    }
    for (r, d) in ro.iter_mut().zip(&defs) {
        *r &= *d == 1;
    }
    let mut l = Licm {
        prog,
        code,
        loc,
        preds: vec![Vec::new(); nb],
        reg_ty: f.reg_ty.clone(),
        ldef: vec![0; f.reg_ty.len()],
        defs,
        member: vec![0; nb],
        tags: 0,
        ro,
    };
    for b in 0..nb {
        for s in l.succs(b) {
            if (s as usize) < nb && !l.preds[s as usize].contains(&(b as u32)) {
                l.preds[s as usize].push(b as u32);
            }
        }
    }
    // ponytail: a few rounds cover loops nested that deep around one header.
    for _ in 0..4 {
        if !l.split_shared_headers() {
            break;
        }
    }
    let mut loops: FxMap<u32, Vec<u32>> = FxMap::default();
    for (latch, h) in l.back_edges() {
        loops.entry(h).or_default().push(latch);
    }
    // Inner loops first: a loop's body is larger than that of every loop inside it.
    let mut order: Vec<(usize, u32, Vec<u32>)> = Vec::new();
    for (h, latches) in loops {
        let n = l.walk(h, &latches).1.len();
        order.push((n, h, latches));
    }
    order.sort_unstable_by_key(|o| (o.0, o.1));
    let mut changed = false;
    for (_, h, latches) in &order {
        changed |= l.hoist(f, *h, latches);
    }
    if !changed {
        return false;
    }
    let (mut code, mut locs) = (
        Vec::with_capacity(f.code.len()),
        Vec::with_capacity(f.code.len()),
    );
    let mut blocks = Vec::with_capacity(l.code.len());
    for (c, s) in l.code.iter().zip(&l.loc) {
        let start = code.len() as u32;
        for (i, s) in c.iter().zip(s) {
            if !is_dead(i) {
                code.push(*i);
                locs.push(*s);
            }
        }
        blocks.push(crate::Block {
            start,
            end: code.len() as u32,
        });
    }
    for (i, b) in blocks.into_iter().enumerate() {
        match f.blocks.get_mut(BlockId(i as u32)) {
            Some(x) => *x = b,
            None => {
                f.blocks.push(b);
            }
        }
    }
    f.code = code;
    f.loc = locs;
    f.reg_ty = l.reg_ty;
    true
}
