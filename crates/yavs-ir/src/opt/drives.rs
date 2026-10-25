//! Repeated zero-delay drives of one driver in an activation (#667).
//!
//! A combinational process that gives every output a default and overrides it in an `if` drives
//! the output twice (`x <= '0'; if c then x <= d; end if;`). The second drive deletes the
//! transaction of the first (LRM 10.5.2.2: a new transaction at the same time deletes the ones
//! at or after its time), so the first drive, with the waveform it builds, is wasted on the
//! paths that drive again. This pass delays the first drive down the control flow, as partial
//! dead code elimination: it is dropped where a drive of the same elements with a waveform of
//! one element at delay zero follows, and a copy is made at the last point of the other paths
//! the drive can be delayed to.
//!
//! What may be delayed (`Dr::payload`): a drive of one waveform element at time zero (the first
//! word of its block is a stored constant 0), no rejection limit, onto a target that is a
//! `SigVal` of a constant length and an offset that is a constant or a register defined once.
//! What moves with it is what builds the waveform: the block it allocates, the stores and copies
//! into it, and the temporaries and constants only they read, none of which can fail. Registers
//! defined once that these read (the target, an offset, a count) keep their value and stay;
//! everything the moved instructions define is read by no other instruction. Signal values do
//! not change inside an activation, so a copy reads what the original read.
//!
//! Two drives are the same when their targets are: an instance signal, an offset (the same
//! constant or the same register) and a length. The driver slot is no part of it, because two
//! slots of a process that name the same elements are one kernel driver.
//!
//! What it may be delayed past: everything except an instruction that can see or change the
//! transactions of the driver (a drive onto elements that may overlap the target, `DriveSlot`,
//! `SigScatter`, `Disconnect`, `Force`, `Release`, `Driving`, `DrivingValue`, calls of procedures, protected
//! methods and foreign code), a suspension (the copy goes before the `Stmt` of the wait, which
//! `static_wait` wants first) and a join that some path enters without the drive pending (the
//! copy goes to the end of the pending predecessors, an edge of a `CondBr` is split). The blocks
//! the drive is delayed in are found like a topological order: a block joins when all its
//! predecessors do and pass the drive on, so cycles never do and a copy runs at most once per
//! activation. Case tables cannot grow, so the arms of a `Case` all take the drive or the copy
//! goes before the `Case`. At most `MAX_COPIES` copies are made, and the drive must be dropped on
//! at least one path.

// Loops index several parallel per-block vectors by position.
#![allow(clippy::needless_range_loop)]

use super::*;
use crate::{ConstData, ConstId, Regs, SCATTER_PART, SCATTER_REJECT};

use std::rc::Rc;

/// Most copies of a drive (placed copies plus split edges); more would grow the code for a
/// path or two.
const MAX_COPIES: usize = 6;
/// Most drives looked at in one process.
const MAX_TRIES: usize = 2000;

/// (instance signal, offset kind (0 a constant, 1 a register defined once), offset or register,
/// length): the static identity of the target of a drive. The driver slot is no part of it: two
/// slots of one process that name the same elements are one kernel driver.
type Key = (u32, u8, i64, i64);

/// The leaf signals of a table (sorted) and whether its value type is a record.
type TabInfo = (Rc<[u32]>, bool);

/// The operands of a scatter: the delay, a rejection limit, the part (first, outer).
type ScOps = (Reg, bool, Option<(Reg, Reg)>);

/// An offset or count of a part: a constant (0) or a register defined once (1).
type Pk = (u8, i64);

/// What a drive-like instruction writes: elements of one signal ([`Key`]), or the leaf signals
/// of a record driven by one `SigScatter` (sorted), all of them (`None`) or the part of the
/// outermost array `(first, outer)` the registers name (#677).
#[derive(Clone, PartialEq, Eq, Hash)]
enum Fp {
    D(Key),
    S(Rc<[u32]>, Option<(Pk, Pk)>),
}

impl Fp {
    /// Does a zero-delay write of `self` certainly write every element `d` writes? A scatter of
    /// the whole record writes every element of its leaf signals; a drive writes no leaf of a
    /// scatter entirely (the length of a signal is not known here).
    fn covers(&self, d: &Fp) -> bool {
        match (self, d) {
            (Fp::D(a), Fp::D(b)) => a == b,
            (Fp::S(s, None), Fp::D(k)) => s.binary_search(&k.0).is_ok(),
            (Fp::S(s, p), Fp::S(t, q)) => {
                (p.is_none() || p == q) && t.iter().all(|x| s.binary_search(x).is_ok())
            }
            _ => false,
        }
    }

    /// Do the two certainly write no element in common?
    fn apart(&self, o: &Fp) -> bool {
        match (self, o) {
            (Fp::D(a), Fp::D(b)) => a.0 != b.0 || disjoint(*a, *b),
            (Fp::S(s, _), Fp::D(k)) | (Fp::D(k), Fp::S(s, _)) => s.binary_search(&k.0).is_err(),
            (Fp::S(s, _), Fp::S(t, _)) => !s.iter().any(|x| t.binary_search(x).is_ok()),
        }
    }
}

/// Something in a block that matters to a delayed drive (`Dr::events`).
enum Ev {
    Drive {
        idx: u32,
        fp: Option<Fp>,
        /// A drive of one element at delay zero, so one that deletes an earlier one.
        zero: bool,
    },
    Bar {
        idx: u32,
        wait: bool,
    },
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Scan {
    /// A drive of the same slot and range follows before anything else matters.
    Kill,
    /// Something needs the drive done before it: the copy goes before this instruction.
    Barrier(usize),
    /// The block ends (by a branch) with the drive still delayed.
    Through,
    /// A shape the pass does not handle.
    Give,
}

struct Dr<'a> {
    prog: &'a Program,
    code: Vec<Vec<Inst>>,
    loc: Vec<Vec<StmtId>>,
    reg_ty: Vec<RegTy>,
    /// Definitions per register.
    defs: Vec<u32>,
    /// What the (last) definition of each register is: a constant, a `SigVal`, or an address.
    rconst: Vec<Option<i64>>,
    rsig: Vec<Option<(u32, Reg, Reg)>>,
    operands: Vec<Reg>,
    /// Occurrences of each register in the code (operand lists included).
    occ: Vec<u32>,
    occ_on: bool,
    /// How many drives each key has, and how many scatters each signal is a leaf of.
    keys: FxMap<Key, u32>,
    scat: FxMap<u32, u32>,
    /// Constant memory: registers defined by `AddrOf` of the constant pool.
    caddr: Vec<bool>,
    /// The leaf signals of a leaf table (sorted), by constant.
    tabs: std::cell::RefCell<FxMap<u32, Option<TabInfo>>>,
    preds: Vec<Vec<u32>>,
    /// `events` per block, dropped when the block changes.
    evs: std::cell::RefCell<Vec<Option<Rc<Vec<Ev>>>>>,
}

/// Are the targets in one signal certainly disjoint (constant offsets only)?
fn disjoint(a: Key, b: Key) -> bool {
    a.1 == 0 && b.1 == 0 && (a.2 + a.3 <= b.2 || b.2 + b.3 <= a.2)
}

impl Dr<'_> {
    fn succs(&self, b: usize) -> Vec<usize> {
        match self.code[b].last() {
            Some(&Inst::Case { table, .. }) => match self.prog.cases.get(table) {
                Some(t) => std::iter::once(t.default)
                    .chain(t.arms.iter().map(|a| a.2))
                    .map(|x| x.0 as usize)
                    .collect(),
                None => Vec::new(),
            },
            Some(i) => i.targets().iter().map(|x| x.0 as usize).collect(),
            None => Vec::new(),
        }
    }

    fn distinct_succs(&self, b: usize) -> Vec<usize> {
        let mut s = self.succs(b);
        s.sort_unstable();
        s.dedup();
        s
    }

    fn konst(&self, r: Reg) -> Option<i64> {
        if self.defs[r.0 as usize] == 1 {
            self.rconst[r.0 as usize]
        } else {
            None
        }
    }

    /// (signal, offset kind, offset, length) of a register defined once by a `SigVal` of a
    /// constant length and an offset that is a constant or a register defined once.
    fn sigv(&self, r: Reg) -> Option<(u32, u8, i64, i64)> {
        if self.defs[r.0 as usize] != 1 {
            return None;
        }
        let (s, o, l) = self.rsig[r.0 as usize]?;
        let l = self.konst(l)?;
        match self.konst(o) {
            Some(c) => Some((s, 0, c, l)),
            None if self.defs[o.0 as usize] == 1 => Some((s, 1, i64::from(o.0), l)),
            None => None,
        }
    }

    /// Count the instruction in the tables (`add`) or take it out.
    fn note(&mut self, i: &Inst, add: bool) {
        let step = |n: &mut u32| {
            if add {
                *n += 1;
            } else {
                *n -= 1;
            }
        };
        let defs = &mut self.defs;
        for_defs_fast(i, |r| step(&mut defs[r.0 as usize]));
        if self.occ_on {
            self.count(i, add);
        }
        if !add {
            return;
        }
        match *i {
            Inst::Const { dst, v } => self.rconst[dst.0 as usize] = Some(v),
            Inst::SigVal {
                dst,
                sig,
                offset,
                len,
            } => self.rsig[dst.0 as usize] = Some((sig, offset, len)),
            Inst::AddrOf {
                dst,
                space: AddrSpace::Const(_),
                ..
            } => self.caddr[dst.0 as usize] = true,
            _ => {}
        }
    }

    /// Occurrences of the registers of `i` (`add`, else taken out).
    fn count(&mut self, i: &Inst, add: bool) {
        let occ = &mut self.occ;
        let step = |n: &mut u32| {
            if add {
                *n += 1;
            } else {
                *n -= 1;
            }
        };
        i.visit(&mut |r, _, _| step(&mut occ[r.0 as usize]));
        if let Some(l) = i.list() {
            for r in self
                .operands
                .get(l.start as usize..(l.start + l.len) as usize)
                .unwrap_or(&[])
            {
                step(&mut occ[r.0 as usize]);
            }
        }
    }

    /// Count the occurrences of every register, the first time somebody asks (most processes
    /// never need them).
    fn ensure_occ(&mut self) {
        if self.occ_on {
            return;
        }
        self.occ = vec![0; self.reg_ty.len()];
        let code = std::mem::take(&mut self.code);
        for b in &code {
            for i in b {
                self.count(i, true);
            }
        }
        self.code = code;
        self.occ_on = true;
    }

    /// The tables, from the code as it is now.
    fn init(&mut self) {
        let n = self.reg_ty.len();
        self.defs = vec![0; n];
        self.rconst = vec![None; n];
        self.rsig = vec![None; n];
        self.caddr = vec![false; n];
        let code = std::mem::take(&mut self.code);
        for b in &code {
            for i in b {
                self.note(i, true);
            }
        }
        self.keys.clear();
        self.scat.clear();
        for b in &code {
            for i in b {
                self.tally(i, true);
            }
        }
        self.code = code;
        self.compute_preds();
    }

    /// Predecessors, from the blocks the entry reaches (dead blocks left by earlier passes
    /// point at joins and must not keep them from being passed).
    fn compute_preds(&mut self) {
        let nb = self.code.len();
        self.preds = vec![Vec::new(); nb];
        let mut seen = vec![false; nb];
        let mut st = vec![0usize];
        while let Some(b) = st.pop() {
            if b >= nb || seen[b] {
                continue;
            }
            seen[b] = true;
            for s in self.distinct_succs(b) {
                if s < nb {
                    self.preds[s].push(b as u32);
                    st.push(s);
                }
            }
        }
    }

    /// Key of a drive instruction.
    fn key(&self, i: &Inst) -> Option<Key> {
        let Inst::Drive {
            target: Some(t), ..
        } = *i
        else {
            return None;
        };
        self.sigv(t)
    }

    /// Count the drive or scatter `i` in `keys` and `scat` (`add`) or take it out.
    fn tally(&mut self, i: &Inst, add: bool) {
        let step = |n: &mut u32| {
            if add {
                *n += 1;
            } else {
                *n = n.saturating_sub(1);
            }
        };
        match self.fp(i) {
            Some(Fp::D(k)) => step(self.keys.entry(k).or_insert(0)),
            Some(Fp::S(s, _)) => s
                .iter()
                .for_each(|x| step(self.scat.entry(*x).or_insert(0))),
            None => {}
        }
    }

    /// A register that is a constant or defined once.
    fn pk(&self, r: Reg) -> Option<Pk> {
        match self.konst(r) {
            Some(c) => Some((0, c)),
            None if self.defs[r.0 as usize] == 1 => Some((1, i64::from(r.0))),
            None => None,
        }
    }

    /// The operands of a scatter: the delay, whether it has a rejection limit, and the part.
    fn scatter_ops(&self, ops: Regs, has: u8) -> Option<ScOps> {
        let o = self
            .operands
            .get(ops.start as usize..(ops.start + ops.len) as usize)?;
        let (&after, mut rest) = o.split_first()?;
        let reject = has & SCATTER_REJECT != 0;
        if reject {
            rest = rest.get(1..)?;
        }
        let part = match (has & SCATTER_PART != 0, rest) {
            (false, []) => None,
            (true, &[a, b]) => Some((a, b)),
            _ => return None,
        };
        Some((after, reject, part))
    }

    /// The signals of the leaves of table `tab`, sorted, and whether its value type is a record
    /// (one instance of it; an array of records has a count that only the descriptor knows).
    fn table_sigs(&self, tab: ConstId) -> Option<TabInfo> {
        if let Some(r) = self.tabs.borrow().get(&tab.0) {
            return r.clone();
        }
        let r = match self.prog.consts.get(tab) {
            Some(ConstData::Bytes(b)) => crate::gather::entries(b).map(|(ty, it)| {
                let mut v: Vec<u32> = it.map(|(l, _)| l.sig).collect();
                v.sort_unstable();
                v.dedup();
                let rec = matches!(self.prog.types.get(ty), Some(Ty::Record { .. }));
                (Rc::from(v), rec)
            }),
            _ => None,
        };
        self.tabs.borrow_mut().insert(tab.0, r.clone());
        r
    }

    /// What a drive or scatter writes, when that is static.
    fn fp(&self, i: &Inst) -> Option<Fp> {
        match *i {
            Inst::Drive { .. } => self.key(i).map(Fp::D),
            Inst::SigScatter { tab, ops, has, .. } => {
                let (_, _, part) = self.scatter_ops(ops, has)?;
                let (sigs, rec) = self.table_sigs(tab)?;
                // A part is a count of elements of the outermost array: one when the value is a
                // record, else known at run time only.
                let part = match part {
                    Some((a, b)) if rec => Some((self.pk(a)?, self.pk(b)?)),
                    Some(_) => return None,
                    None => None,
                };
                Some(Fp::S(sigs, part))
            }
            _ => None,
        }
    }

    /// Does the scatter `i` write at delay zero, with no rejection limit?
    fn zero_scatter(&self, i: &Inst) -> bool {
        let Inst::SigScatter { ops, has, .. } = *i else {
            return false;
        };
        matches!(self.scatter_ops(ops, has), Some((a, false, _)) if self.konst(a) == Some(0))
    }

    /// Is `r` the address of constant memory?
    fn const_addr(&self, r: Reg) -> bool {
        self.defs[r.0 as usize] == 1 && self.caddr[r.0 as usize]
    }

    /// What `code[b][idx]` writes if it is a drive or scatter that a later one can delete and
    /// that may move (the instruction's own form; `payload` checks the rest).
    fn candidate(&self, b: usize, idx: usize) -> Option<Fp> {
        match self.code[b][idx] {
            Inst::Drive { .. } => self.zero_drive(b, idx).map(Fp::D),
            Inst::SigScatter { src, desc, .. } => {
                let i = &self.code[b][idx];
                (self.zero_scatter(i) && self.const_addr(src) && self.const_addr(desc))
                    .then(|| self.fp(i))?
            }
            _ => None,
        }
    }

    /// Could a drive or scatter of the same signals as `fp` follow? (A cheap test before the
    /// search.)
    fn related(&self, fp: &Fp) -> bool {
        let n = |x: &u32| self.scat.get(x).copied().unwrap_or(0);
        match fp {
            Fp::D(k) => self.keys.get(k).copied().unwrap_or(0) >= 2 || n(&k.0) >= 1,
            Fp::S(s, _) => s.iter().any(|x| n(x) >= 2),
        }
    }

    /// Is `code[b][idx]` a drive of one element at delay zero with no rejection limit; its key.
    fn zero_drive(&self, b: usize, idx: usize) -> Option<Key> {
        let c = &self.code[b];
        let Inst::Drive {
            wave,
            n,
            reject: None,
            ..
        } = c[idx]
        else {
            return None;
        };
        if self.konst(n) != Some(1) {
            return None;
        }
        let key = self.key(&c[idx])?;
        // The nearest store to the first word of the waveform block is the constant 0, and
        // nothing between it and the drive writes the block but through an `Index`.
        let mut j = idx;
        while j > 0 {
            j -= 1;
            match c[j] {
                Inst::Store {
                    addr,
                    src,
                    w: Width::B8,
                } if addr == wave => {
                    if self.konst(src) != Some(0) {
                        return None;
                    }
                    for x in &c[j + 1..idx] {
                        let mut bad = false;
                        x.visit(&mut |r, _, _| bad |= r == wave);
                        let index_past_time = matches!(*x, Inst::Index { base, pos, .. }
                            if base == wave && self.konst(pos).is_some_and(|p| p >= 8));
                        if bad && !index_past_time {
                            return None;
                        }
                    }
                    return Some(key);
                }
                _ => {
                    let mut def = false;
                    c[j].visit(&mut |r, role, _| def |= r == wave && role == Role::Def);
                    if def {
                        return None;
                    }
                }
            }
        }
        None
    }

    /// What in block `b` matters to a delayed drive, in order (built once per version of the
    /// block): drives with their key, and the other instructions that need the drive done
    /// before them (a wait also asks for the copy to go before its `Stmt`).
    fn events(&self, b: usize) -> Rc<Vec<Ev>> {
        if let Some(Some(e)) = self.evs.borrow().get(b) {
            return e.clone();
        }
        let mut v = Vec::new();
        for (idx, i) in self.code[b].iter().enumerate() {
            let idx = idx as u32;
            match *i {
                Inst::Drive { .. } => v.push(Ev::Drive {
                    idx,
                    fp: self.fp(i),
                    zero: self.zero_drive(b, idx as usize).is_some(),
                }),
                Inst::SigScatter { .. } => v.push(Ev::Drive {
                    idx,
                    fp: self.fp(i),
                    zero: self.zero_scatter(i),
                }),
                Inst::DriveSlot { .. }
                | Inst::Disconnect { .. }
                | Inst::Force { .. }
                | Inst::Release { .. }
                | Inst::Driving { .. }
                | Inst::DrivingValue { .. }
                | Inst::CallProt { .. }
                | Inst::CallIndirect { .. }
                | Inst::CallForeign { .. }
                | Inst::Elab { .. }
                | Inst::Ret { .. }
                | Inst::Unreachable { .. } => v.push(Ev::Bar { idx, wait: false }),
                Inst::Call { func, .. } => {
                    if !matches!(
                        self.prog.funcs.get(func).map(|f| f.kind),
                        Some(FuncKind::Function)
                    ) {
                        v.push(Ev::Bar { idx, wait: false });
                    }
                }
                Inst::WaitOn { .. }
                | Inst::WaitEdge { .. }
                | Inst::WaitFor { .. }
                | Inst::WaitAt { .. }
                | Inst::Suspend { .. } => v.push(Ev::Bar { idx, wait: true }),
                _ => {}
            }
        }
        let v = Rc::new(v);
        let mut evs = self.evs.borrow_mut();
        if evs.len() <= b {
            evs.resize(b + 1, None);
        }
        evs[b] = Some(v.clone());
        v
    }

    fn scan(&self, b: usize, from: usize, fp: &Fp) -> Scan {
        let c = &self.code[b];
        for ev in self.events(b).iter() {
            match ev {
                Ev::Drive { idx, fp: k, zero } => {
                    let idx = *idx;
                    if (idx as usize) < from {
                        continue;
                    }
                    if *zero && k.as_ref().is_some_and(|k| k.covers(fp)) {
                        return Scan::Kill;
                    }
                    let free = k.as_ref().is_some_and(|k| k.apart(fp));
                    if !free {
                        return Scan::Barrier(idx as usize);
                    }
                }
                &Ev::Bar { idx, wait } => {
                    let idx = idx as usize;
                    if idx < from {
                        continue;
                    }
                    if !wait {
                        return Scan::Barrier(idx);
                    }
                    return match (from..=idx).rev().find(|&j| c[j] == Inst::Stmt) {
                        Some(j) => Scan::Barrier(j),
                        None => Scan::Give,
                    };
                }
            }
        }
        match c.last() {
            Some(Inst::Br { .. } | Inst::CondBr { .. } | Inst::Case { .. }) => Scan::Through,
            _ => Scan::Give,
        }
    }

    /// May the instruction be part of a delayed drive: it cannot fail and reads only registers
    /// that keep their value, signals (which do not change inside an activation) and memory the
    /// drive builds itself.
    fn movable(&self, i: &Inst) -> bool {
        let k = |r: Reg| self.konst(r).is_some();
        match *i {
            Inst::Const { .. }
            | Inst::ConstF { .. }
            | Inst::Move { .. }
            | Inst::SigReadScalar { .. }
            | Inst::SigRead { .. }
            | Inst::Store { .. }
            | Inst::LSet { .. }
            | Inst::LCopy { .. }
            | Inst::SigScatter { .. }
            | Inst::Drive { .. } => true,
            Inst::AddrOf { space, .. } => !matches!(space, AddrSpace::Up(_)),
            Inst::Alloc { size, .. } => k(size),
            Inst::Index { pos, .. } => k(pos),
            Inst::Copy { bytes, .. } => k(bytes),
            _ => false,
        }
    }

    /// The instructions of the drive at `code[b][k]` that build and do it, in order (`None`:
    /// not movable). They are what its waveform is made of: the blocks it allocates and
    /// writes, the constants and temporary values only they read. Registers defined once and
    /// read by them (the target, offsets, constants) keep their value and stay where they are.
    fn payload(&mut self, b: usize, k: usize) -> Option<Vec<usize>> {
        self.ensure_occ();
        let c = &self.code[b];
        if !matches!(c[k], Inst::Drive { .. } | Inst::SigScatter { .. }) {
            return None;
        }
        let s0 = (0..k).rev().find(|&j| c[j] == Inst::Stmt)? + 1;
        let defines = |i: &Inst, r: Reg| {
            let mut d = false;
            i.visit(&mut |x, role, _| d |= x == r && role == Role::Def);
            d
        };
        let mentions = |i: &Inst, r: Reg| {
            let mut d = false;
            i.visit(&mut |x, _, _| d |= x == r);
            d
        };
        let mut inm = vec![false; k - s0 + 1];
        inm[k - s0] = true;
        // Registers that address a block of the drive, with the instruction that made them.
        let mut roots: Vec<(usize, Reg)> = Vec::new();
        let mut work: Vec<usize> = vec![k];
        loop {
            while let Some(j) = work.pop() {
                let i = c[j];
                if !self.movable(&i) {
                    return None;
                }
                let mut reads: Vec<Reg> = Vec::new();
                i.visit(&mut |r, role, _| {
                    if role == Role::Use {
                        reads.push(r);
                    }
                });
                // A scatter reads constant memory only (its source and descriptor), and its
                // operands keep their value (they are not renamed in a copy).
                if let Inst::SigScatter { src, desc, ops, .. } = i {
                    if !self.const_addr(src) || !self.const_addr(desc) {
                        return None;
                    }
                    for &r in self
                        .operands
                        .get(ops.start as usize..(ops.start + ops.len) as usize)?
                    {
                        if self.defs[r.0 as usize] != 1 {
                            return None;
                        }
                        reads.push(r);
                    }
                }
                for r in reads {
                    // The nearest definition in the statement, if any.
                    let Some(d) = (s0..j).rev().find(|&x| defines(&c[x], r)) else {
                        // From outside: it must keep its value.
                        if self.defs[r.0 as usize] > 1 {
                            return None;
                        }
                        continue;
                    };
                    let block = matches!(c[d], Inst::Alloc { .. } | Inst::Index { .. });
                    if self.defs[r.0 as usize] == 1 && !block && !inm[d - s0] {
                        // A value computed here that nothing changes: it stays.
                        if !matches!(
                            c[d],
                            Inst::Const { .. }
                                | Inst::ConstF { .. }
                                | Inst::AddrOf { .. }
                                | Inst::SigVal { .. }
                                | Inst::Pos { .. }
                                | Inst::Bin { .. }
                        ) {
                            return None;
                        }
                        continue;
                    }
                    if !inm[d - s0] {
                        inm[d - s0] = true;
                        work.push(d);
                    }
                    if block && !roots.iter().any(|x| x.0 == d) {
                        roots.push((d, r));
                    }
                }
            }
            // Writes into a block of the drive belong to it: everything after the block's
            // creation that names it (or a pointer into it) up to the drive.
            let mut grew = false;
            let mut x = 0;
            while x < roots.len() {
                let (d, reg) = roots[x];
                x += 1;
                let mut live: Vec<Reg> = vec![reg];
                for j2 in d + 1..k {
                    let i2 = c[j2];
                    let redef = live.iter().any(|&r| defines(&i2, r));
                    if !redef && live.iter().any(|&r| mentions(&i2, r)) && !inm[j2 - s0] {
                        if matches!(i2, Inst::Drive { .. }) {
                            // Another drive of the same block: not this one's to move.
                            return None;
                        }
                        inm[j2 - s0] = true;
                        work.push(j2);
                        grew = true;
                    }
                    if let Inst::Index { dst, base, .. } = i2
                        && live.contains(&base)
                    {
                        live.push(dst);
                        if !roots.iter().any(|y| y.0 == j2) {
                            roots.push((j2, dst));
                        }
                    } else if redef {
                        live.retain(|&r| !defines(&i2, r));
                        if live.is_empty() {
                            break;
                        }
                    }
                }
            }
            if !grew {
                break;
            }
        }
        let m: Vec<usize> = (0..=k - s0).filter(|&x| inm[x]).map(|x| x + s0).collect();
        // Closed: what the instructions define is read only by them.
        for &d in &m {
            let mut dsts: Vec<Reg> = Vec::new();
            c[d].visit(&mut |r, role, _| {
                if role == Role::Def {
                    dsts.push(r);
                }
            });
            for r in dsts {
                if self.defs[r.0 as usize] == 1 {
                    let mut inside = 0;
                    for &x in &m {
                        c[x].visit(&mut |y, _, _| inside += u32::from(y == r));
                    }
                    if self.occ[r.0 as usize] != inside {
                        return None;
                    }
                } else {
                    let mut killed = false;
                    for (j2, i2) in c.iter().enumerate().skip(d + 1) {
                        let inside = m.binary_search(&j2).is_ok();
                        if !inside && mentions(i2, r) && !defines(i2, r) {
                            return None;
                        }
                        if defines(i2, r) {
                            killed = true;
                            break;
                        }
                    }
                    if !killed {
                        return None;
                    }
                }
            }
        }
        Some(m)
    }

    /// The statement `seg` with fresh registers for what it defines.
    fn copy_of(&mut self, seg: &[Inst]) -> Vec<Inst> {
        let mut map: Vec<(u32, u32)> = Vec::new();
        let mut out = Vec::with_capacity(seg.len());
        for i in seg {
            let mut i = *i;
            // Definitions first: a register is defined before it is read inside the statement.
            i.visit_mut(&mut |r, role, _| {
                if role == Role::Def && !map.iter().any(|m| m.0 == r.0) {
                    let ty = self.reg_ty[r.0 as usize];
                    map.push((r.0, self.reg_ty.len() as u32));
                    self.reg_ty.push(ty);
                    if ty == RegTy::Sig {
                        self.reg_ty.push(RegTy::I64);
                    }
                    let n = self.reg_ty.len();
                    self.defs.resize(n, 0);
                    if self.occ_on {
                        self.occ.resize(n, 0);
                    }
                    self.rconst.resize(n, None);
                    self.rsig.resize(n, None);
                    self.caddr.resize(n, false);
                }
            });
            i.visit_mut(&mut |r, _, _| {
                if let Some(m) = map.iter().find(|m| m.0 == r.0) {
                    r.0 = m.1;
                }
            });
            // A copy has an operand list of its own.
            if let Some(l) = i.list_mut() {
                let o = self
                    .operands
                    .get(l.start as usize..(l.start + l.len) as usize)
                    .unwrap_or(&[])
                    .to_vec();
                l.start = self.operands.len() as u32;
                self.operands.extend(o);
            }
            self.note(&i, true);
            self.tally(&i, true);
            out.push(i);
        }
        out
    }

    /// Try to delay the drive at `code[b0][k]`; whether the code changed.
    fn try_one(&mut self, b0: usize, k: usize) -> bool {
        let Some(key) = self.candidate(b0, k) else {
            return false;
        };
        if !self.related(&key) {
            return false;
        }
        let nb = self.code.len();
        let first = self.scan(b0, k + 1, &key);
        if first == Scan::Give {
            return false;
        }
        // The blocks the drive is still delayed in: a block joins them when every predecessor
        // does (and passes the drive on), so a join entered by a path that does not is outside
        // and so is every cycle. `ready` holds each block with what its scan found.
        let mut ready: FxMap<usize, Scan> = FxMap::default();
        let mut left: FxMap<usize, u32> = FxMap::default();
        let mut work: Vec<(usize, Scan)> = vec![(b0, first)];
        let mut kills = 0;
        let mut place: Vec<(usize, usize)> = Vec::new();
        let mut split: Vec<(usize, usize)> = Vec::new();
        while let Some((q, sc)) = work.pop() {
            let mut sc = sc;
            let succs = self.distinct_succs(q);
            if sc == Scan::Through && matches!(self.code[q].last(), Some(Inst::Case { .. })) {
                // A `Case` cannot send the drive to some arms only: all its arms must have it
                // from here alone.
                let alone = succs.iter().all(|&x| {
                    x != b0 && left.get(&x).copied().unwrap_or(self.preds[x].len() as u32) == 1
                });
                if !alone {
                    sc = Scan::Barrier(self.code[q].len() - 1);
                }
            }
            ready.insert(q, sc);
            match sc {
                Scan::Kill => kills += 1,
                Scan::Barrier(i) => {
                    place.push((q, i));
                    if place.len() > MAX_COPIES {
                        return false;
                    }
                }
                Scan::Give => return false,
                Scan::Through => {
                    for &x in &succs {
                        if x == b0 {
                            return false;
                        }
                        let n = left.entry(x).or_insert(self.preds[x].len() as u32);
                        *n -= 1;
                        if *n == 0 {
                            let sx = self.scan(x, 0, &key);
                            work.push((x, sx));
                        }
                    }
                }
            }
        }
        // Where the drive stops: the edges from the blocks that pass it on to the others.
        for (&q, &sc) in &ready {
            if sc != Scan::Through {
                continue;
            }
            let all = self.distinct_succs(q);
            let unwanted: Vec<usize> = all
                .iter()
                .copied()
                .filter(|x| !ready.contains_key(x))
                .collect();
            if unwanted.len() == all.len() {
                place.push((q, self.code[q].len() - 1));
            } else if !unwanted.is_empty() {
                split.extend(unwanted.iter().map(|&x| (q, x)));
            }
        }
        if kills == 0 {
            return false;
        }
        if place.len() + split.len() > MAX_COPIES {
            return false;
        }
        let Some(m) = self.payload(b0, k) else {
            return false;
        };
        // Apply.
        let seg: Vec<Inst> = m.iter().map(|&j| self.code[b0][j]).collect();
        let segloc: Vec<StmtId> = m.iter().map(|&j| self.loc[b0][j]).collect();
        for i in &seg {
            self.tally(i, false);
            self.note(i, false);
        }
        let mut ins: Vec<Vec<(usize, Vec<Inst>)>> = vec![Vec::new(); nb];
        for &(q, i) in &place {
            let c = self.copy_of(&seg);
            ins[q].push((i, c));
        }
        let mut new_blocks: Vec<(Vec<Inst>, Vec<StmtId>)> = Vec::new();
        for &(q, to) in &split {
            let mut c = self.copy_of(&seg);
            let id = nb + new_blocks.len();
            c.push(Inst::Br {
                to: BlockId(to as u32),
            });
            let mut l = segloc.clone();
            l.push(segloc[segloc.len() - 1]);
            new_blocks.push((c, l));
            if let Some(Inst::CondBr { t, f, .. }) = self.code[q].last_mut() {
                if t.0 as usize == to {
                    *t = BlockId(id as u32);
                }
                if f.0 as usize == to {
                    *f = BlockId(id as u32);
                }
            }
        }
        for q in 0..nb {
            let mut v = std::mem::take(&mut ins[q]);
            let removed = q == b0;
            if v.is_empty() && !removed {
                continue;
            }
            v.sort_by_key(|x| x.0);
            let old = std::mem::take(&mut self.code[q]);
            let oldl = std::mem::take(&mut self.loc[q]);
            let mut nc = Vec::with_capacity(old.len());
            let mut nl = Vec::with_capacity(old.len());
            let mut vi = v.into_iter().peekable();
            for (idx, (i, l)) in old.into_iter().zip(oldl).enumerate() {
                while vi.peek().is_some_and(|x| x.0 == idx) {
                    if let Some((_, c)) = vi.next() {
                        nl.extend_from_slice(&segloc);
                        nc.extend(c);
                    }
                }
                if removed && m.binary_search(&idx).is_ok() {
                    continue;
                }
                nc.push(i);
                nl.push(l);
            }
            self.code[q] = nc;
            self.loc[q] = nl;
            if let Some(e) = self.evs.get_mut().get_mut(q) {
                *e = None;
            }
        }
        for (c, l) in new_blocks {
            self.code.push(c);
            self.loc.push(l);
        }
        self.compute_preds();
        true
    }
}

/// Delay the repeated drives of a process; whether anything changed.
pub(super) fn run(prog: &Program, f: &mut crate::Func) -> bool {
    if keep_variables_on()
        || f.kind != FuncKind::Process
        || f.flags.contains(FuncFlags::HAS_COVER)
        || !f.via.is_empty()
    {
        return false;
    }
    // Two drives of one slot onto the same static range are needed.
    let n = f.reg_ty.len();
    let mut konst: Vec<Option<i64>> = vec![None; n];
    let mut sigv: Vec<Option<(u32, u32, u32)>> = vec![None; n];
    for i in &f.code {
        match *i {
            Inst::Const { dst, v } => konst[dst.0 as usize] = Some(v),
            Inst::SigVal {
                dst,
                sig,
                offset,
                len,
            } => sigv[dst.0 as usize] = Some((sig, offset.0, len.0)),
            _ => {}
        }
    }
    let mut seen: std::collections::HashSet<(u32, u8, i64, i64), BuildHasherDefault<Fx>> =
        std::collections::HashSet::default();
    let mut twice = false;
    // Signals driven by a drive with a static target.
    let mut driven: std::collections::HashSet<u32, BuildHasherDefault<Fx>> =
        std::collections::HashSet::default();
    for i in &f.code {
        if let Inst::Drive {
            target: Some(t), ..
        } = *i
            && let Some((sig, o, l)) = sigv[t.0 as usize]
            && let Some(l) = konst[l as usize]
        {
            let off = match konst[o as usize] {
                Some(c) => (0, c),
                None => (1, i64::from(o)),
            };
            twice |= !seen.insert((sig, off.0, off.1, l));
            driven.insert(sig);
        }
    }
    // A scatter and a write of one of its leaves, or two scatters of one: the search tells.
    let mut sc: std::collections::HashSet<u32, BuildHasherDefault<Fx>> =
        std::collections::HashSet::default();
    for i in &f.code {
        if let Inst::SigScatter { tab, .. } = *i
            && let Some(ConstData::Bytes(b)) = prog.consts.get(tab)
            && let Some((_, it)) = crate::gather::entries(b)
        {
            let mut mine = Vec::new();
            for (l, _) in it {
                twice |= driven.contains(&l.sig) || sc.contains(&l.sig);
                mine.push(l.sig);
            }
            sc.extend(mine);
        }
    }
    if !twice {
        return false;
    }
    let nb = f.blocks.len();
    let mut code: Vec<Vec<Inst>> = Vec::with_capacity(nb);
    let mut loc: Vec<Vec<StmtId>> = Vec::with_capacity(nb);
    for b in f.blocks.values() {
        let r = b.start as usize..b.end as usize;
        code.push(f.code[r.clone()].to_vec());
        loc.push(f.loc[r].to_vec());
    }
    let mut d = Dr {
        prog,
        code,
        loc,
        reg_ty: f.reg_ty.clone(),
        defs: Vec::new(),
        rconst: Vec::new(),
        rsig: Vec::new(),
        operands: f.operands.clone(),
        occ: Vec::new(),
        occ_on: false,
        keys: FxMap::default(),
        scat: FxMap::default(),
        caddr: Vec::new(),
        tabs: std::cell::RefCell::new(FxMap::default()),
        preds: Vec::new(),
        evs: std::cell::RefCell::new(Vec::new()),
    };
    d.init();
    let mut tried: std::collections::HashSet<(StmtId, u32), BuildHasherDefault<Fx>> =
        std::collections::HashSet::default();
    let mut changed = false;
    let mut ntry = 0;
    let (mut b, mut k) = (0, 0);
    while b < d.code.len() && ntry < MAX_TRIES {
        if k >= d.code[b].len() {
            b += 1;
            k = 0;
            continue;
        }
        if let Inst::Drive { driver, .. } | Inst::SigScatter { driver, .. } = d.code[b][k]
            && tried.insert((d.loc[b][k], driver))
        {
            ntry += 1;
            if d.try_one(b, k) {
                changed = true;
                // The statement is gone: look at the block again from its start.
                k = 0;
                continue;
            }
        }
        k += 1;
    }
    if !changed {
        return false;
    }
    let (mut code, mut locs) = (Vec::new(), Vec::new());
    for (i, (c, l)) in d.code.iter().zip(&d.loc).enumerate() {
        let start = code.len() as u32;
        code.extend_from_slice(c);
        locs.extend_from_slice(l);
        let b = crate::Block {
            start,
            end: code.len() as u32,
        };
        match f.blocks.get_mut(BlockId(i as u32)) {
            Some(x) => *x = b,
            None => {
                f.blocks.push(b);
            }
        }
    }
    f.code = code;
    f.loc = locs;
    f.reg_ty = d.reg_ty;
    f.operands = d.operands;
    true
}
