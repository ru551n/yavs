//! Sinking of computations into the branch that needs them (round 16 of #520).
//!
//! After a call is inlined, the code that built its arguments sits in front of the branch the
//! callee starts with. A checking procedure (`assert`-like: `check(cond, "got " & image(x))`)
//! evaluates its message for every call, though only a failing check reads it. The LRM has the
//! actual evaluated before the call, so the evaluation can be skipped only where nothing of it
//! is observable: this pass moves an instruction into the branch only if running it can neither
//! report, nor write memory anything else reads, nor fail except for a result too long to
//! hold (a string of 2^31 elements or more cannot be built in the memory a run has; such a
//! result stops the run in the resource limits whichever way it is reached).
//!
//! What moves (`eff`): copies and computations of registers, loads, descriptor lengths and
//! index arithmetic over memory the run built itself (temporary blocks, the strings `'image`
//! returns) or the constant pool, `Alloc`, `MakeDope`, `LayoutOf` and `Cat` into such blocks,
//! the checked additions and the length check of `&` when all their operands are lengths, and
//! `'image` of integers and enumerations. What stays: everything that can report or fail
//! (`to_integer` of a vector that may hold `U`, a division, a range check of a counter), every
//! call, every access to memory the run did not build, every instruction with a use of its
//! result outside the moved set.
//!
//! The moved set is the greatest set closed under the rules below, found by dropping members
//! until none breaks one; the code before the branch is one straight run (the block and the
//! single-predecessor blocks after it), the moved instructions go to the start of the branch
//! target that has a single predecessor, in their order, and write fresh registers (the code
//! before the branch reuses registers, so the originals cannot be kept):
//!
//! - every reader of a moved result is moved, or is in the target block (which reads the fresh
//!   register instead) and the result is read nowhere after it;
//! - no operand of a moved instruction is written by an instruction that stays and comes after
//!   it (it would read the later value);
//! - a moved read of a block does not move past a write that may reach that block, and no
//!   moved allocation or write moves past a call or a statement end.
//!
//! Only functions that inlined a call are looked at (`optimize_func_with`), and only
//! where at least `MIN_SUNK` instructions move.

// Loops index several parallel per-position vectors by position.
#![allow(clippy::needless_range_loop)]

use super::*;

mod calls;

/// Fewest moved instructions that make the change worth its fresh registers.
const MIN_SUNK: usize = 8;
/// Fewest moved instructions in front of the replay of a call: the call is rarely made.
const MIN_REPLAY: usize = 3;
/// Longest straight run looked at.
const MAX_SEQ: usize = 800;
/// Registers times blocks above which the liveness bit sets are not made.
const BUDGET: usize = 1 << 21;

type Bits = Vec<u64>;

fn bit(b: &Bits, r: Reg) -> bool {
    b.get(r.0 as usize / 64)
        .is_some_and(|w| w >> (r.0 % 64) & 1 == 1)
}

fn set(b: &mut Bits, r: Reg) {
    if let Some(w) = b.get_mut(r.0 as usize / 64) {
        *w |= 1 << (r.0 % 64);
    }
}

/// Where the memory a register addresses comes from.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum Org {
    Unknown,
    /// The constant pool or the instance data (read only).
    Ro,
    /// An object behind an access value.
    Heap,
    /// The slots of the function's own frame.
    Frame,
    /// A block the run made in the straight run: position of its creator.
    Temp(u32),
    /// The string `'image` returned (and the pointers inside it): position of the call.
    Img(u32),
}

/// What an instruction does to memory, as far as sinking cares.
#[derive(Clone, Default)]
struct Eff {
    /// Blocks it reads.
    reads: Vec<Org>,
    /// It makes a block (the result register addresses it).
    creates: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Class {
    /// Can neither move nor be skipped.
    No,
    /// Moves when the rules allow.
    Movable,
    /// Harmless and free (constants, addresses): stays where it is.
    Free,
}

struct Seq {
    insts: Vec<Inst>,
    loc: Vec<StmtId>,
    via: Vec<u32>,
}

struct Sink<'a> {
    prog: &'a Program,
    code: Vec<Vec<Inst>>,
    loc: Vec<Vec<StmtId>>,
    via: Vec<Vec<u32>>,
    operands: Vec<Reg>,
    reg_ty: Vec<RegTy>,
    preds: Vec<Vec<u32>>,
    /// Registers defined once by the address of read-only memory.
    ro: Vec<bool>,
    /// Registers defined once by an address in the function's own frame.
    frame: Vec<bool>,
    /// Registers defined once by a constant.
    konst: Vec<Option<i64>>,
    /// Registers defined once by an address: the space and offset.
    addr: Vec<Option<(AddrSpace, u32)>>,
    live_in: Vec<Bits>,
    /// Blocks `calls.rs` made to replay a call: their branch is worth a look for what a second
    /// call's arguments left in front of it.
    replay: Vec<usize>,
}

impl Sink<'_> {
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

    fn list(&self, i: &Inst) -> &[Reg] {
        i.list()
            .and_then(|l| {
                self.operands
                    .get(l.start as usize..l.start as usize + l.len as usize)
            })
            .unwrap_or(&[])
    }

    /// The registers `i` reads (visit order, then its operand list).
    fn reads(&self, i: &Inst) -> Vec<Reg> {
        let mut v = Vec::new();
        i.visit(&mut |r, role, _| {
            if role == Role::Use {
                v.push(r);
            }
        });
        v.extend_from_slice(self.list(i));
        v
    }

    fn writes(i: &Inst) -> Vec<Reg> {
        let mut v = Vec::new();
        i.visit(&mut |r, role, _| {
            if role == Role::Def {
                v.push(r);
            }
        });
        v
    }

    /// Backward liveness over the blocks.
    fn liveness(&mut self) {
        let nb = self.code.len();
        let w = self.reg_ty.len().div_ceil(64).max(1);
        let mut uses: Vec<Bits> = vec![vec![0; w]; nb];
        let mut defs: Vec<Bits> = vec![vec![0; w]; nb];
        for b in 0..nb {
            for i in &self.code[b] {
                for r in self.reads(i) {
                    if !bit(&defs[b], r) {
                        set(&mut uses[b], r);
                    }
                }
                for r in Self::writes(i) {
                    set(&mut defs[b], r);
                }
            }
        }
        let mut live: Vec<Bits> = uses.clone();
        let mut again = true;
        while again {
            again = false;
            for b in (0..nb).rev() {
                let mut out = vec![0u64; w];
                for s in self.succs(b) {
                    for (o, i) in out.iter_mut().zip(&live[s as usize]) {
                        *o |= *i;
                    }
                }
                let inn: Bits = (0..w)
                    .map(|x| uses[b][x] | (out[x] & !defs[b][x]))
                    .collect();
                if inn != live[b] {
                    live[b] = inn;
                    again = true;
                }
            }
        }
        self.live_in = live;
    }

    fn live_out(&self, b: usize) -> Bits {
        let w = self.reg_ty.len().div_ceil(64).max(1);
        let mut out = vec![0u64; w];
        for s in self.succs(b) {
            for (o, i) in out.iter_mut().zip(&self.live_in[s as usize]) {
                *o |= *i;
            }
        }
        out
    }

    /// Is the native `'image` of an integer or enumeration?
    fn is_image(&self, native: crate::NativeId, nargs: usize) -> bool {
        let Some(n) = self.prog.natives.get(native) else {
            return false;
        };
        n.symbol == "yavs.attr.image"
            && nargs == 1
            && matches!(
                n.ty.and_then(|t| self.prog.types.get(t)),
                Some(Ty::Int { .. } | Ty::Enum { .. })
            )
    }

    /// A native that reads its arguments and returns a new block (the `numeric_std` and
    /// `std_logic_1164` functions), touching no memory else: not a barrier for what moves.
    fn light_native(&self, native: crate::NativeId) -> bool {
        let Some(n) = self.prog.natives.get(native) else {
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
}

/// The straight run and what is known along it.
struct Run {
    xlen: usize,
    seq: Seq,
    /// Per position: (register read, position of its definition in the run).
    uses: Vec<Vec<(Reg, Option<usize>)>>,
    defs: Vec<Vec<Reg>>,
    users: Vec<Vec<usize>>,
    class: Vec<Class>,
    eff: Vec<Eff>,
    /// The memory an instruction writes, as the straight run sees it (`None`: none).
    wrote: Vec<Option<Org>>,
    /// Statements end, calls, suspensions: nothing allocated moves past one.
    barrier: Vec<bool>,
    /// Can the instruction suspend the process (no register lives across a suspension).
    suspends: Vec<bool>,
    last_def: HashMap<Reg, usize>,
}

impl Sink<'_> {
    /// Analyse the instructions `seq` (a straight run ending in a `CondBr`).
    fn analyse(&self, seq: Seq, xlen: usize) -> Run {
        let n = seq.insts.len();
        let mut r = Run {
            xlen,
            uses: vec![Vec::new(); n],
            defs: vec![Vec::new(); n],
            users: vec![Vec::new(); n],
            class: vec![Class::No; n],
            eff: vec![Eff::default(); n],
            wrote: vec![None; n],
            barrier: vec![false; n],
            suspends: vec![false; n],
            last_def: HashMap::new(),
            seq,
        };
        let mut org: Vec<Org> = vec![Org::Unknown; n];
        let mut len: Vec<bool> = vec![false; n];
        // Positions of `LayoutOf` (its words are sizes and strides of lengths).
        let mut layout: Vec<bool> = vec![false; n];
        let small = |v: i64| (-(1i64 << 40)..=(1i64 << 40)).contains(&v);
        for p in 0..n {
            let i = r.seq.insts[p];
            let reads = self.reads(&i);
            let rds: Vec<Option<usize>> =
                reads.iter().map(|o| r.last_def.get(o).copied()).collect();
            for (o, d) in reads.iter().zip(&rds) {
                r.uses[p].push((*o, *d));
                if let Some(d) = d {
                    r.users[*d].push(p);
                }
            }
            let orgof = |k: usize| -> Org {
                match rds[k] {
                    Some(d) => org[d],
                    None if self.ro[reads[k].0 as usize] => Org::Ro,
                    None if self.frame[reads[k].0 as usize] => Org::Frame,
                    None => Org::Unknown,
                }
            };
            let lenof = |k: usize| -> bool {
                match rds[k] {
                    Some(d) => len[d],
                    None => self.konst[reads[k].0 as usize].is_some_and(small),
                }
            };
            let (mut my_org, mut my_len) = (Org::Unknown, false);
            let mut class = Class::No;
            let mut eff = Eff::default();
            let mut wrote: Option<Org> = None;
            let mut barrier = false;
            match i {
                Inst::Const { v, .. } => {
                    class = Class::Free;
                    my_len = small(v);
                }
                Inst::ConstF { .. } => class = Class::Free,
                Inst::AddrOf { space, .. } => {
                    class = Class::Free;
                    my_org = match space {
                        AddrSpace::Const(_) | AddrSpace::Inst => Org::Ro,
                        AddrSpace::Frame => Org::Frame,
                        _ => Org::Unknown,
                    };
                }
                Inst::Move { .. } => {
                    class = Class::Movable;
                    (my_org, my_len) = (orgof(0), lenof(0));
                }
                Inst::Select { .. } => {
                    class = Class::Movable;
                    if orgof(1) == orgof(2) {
                        my_org = orgof(1);
                    }
                    my_len = lenof(1) && lenof(2);
                }
                Inst::Cmp { .. } | Inst::FCmp { .. } | Inst::Bool { .. } => {
                    class = Class::Movable;
                    my_len = true;
                }
                // (Checked arithmetic fails only on a result outside `i64`: not for lengths.)
                Inst::Bin {
                    op: IntOp::Add | IntOp::Sub | IntOp::Mul,
                    ..
                } if lenof(0) && lenof(1) => {
                    class = Class::Movable;
                    my_len = true;
                }
                Inst::Index { .. } => {
                    class = Class::Movable;
                    my_org = orgof(0);
                }
                // The length of a string must fit the index type: a string of 2^31 elements
                // does not exist in a run.
                Inst::Check {
                    kind: CheckKind::Index,
                    ..
                } if lenof(0) => class = Class::Movable,
                Inst::DopeLen { .. } => {
                    class = Class::Movable;
                    eff.reads.push(orgof(0));
                    my_len = true;
                }
                Inst::Load { w, .. } if w != Width::Sig => {
                    class = Class::Movable;
                    let o = orgof(0);
                    eff.reads.push(o);
                    // The pointers inside a string `'image` returned point into it, and a word
                    // of a layout is a size or a stride: a length.
                    match o {
                        Org::Img(_) => my_org = o,
                        Org::Temp(k) => my_len = layout[k as usize],
                        _ => {}
                    }
                }
                Inst::Alloc { .. } => {
                    my_org = Org::Temp(p as u32);
                    if lenof(0) {
                        class = Class::Movable;
                        eff.creates = true;
                    }
                }
                Inst::LayoutOf { cache: None, .. } => {
                    class = Class::Movable;
                    eff.reads.push(orgof(0));
                    eff.creates = true;
                    my_org = Org::Temp(p as u32);
                    layout[p] = true;
                }
                // Signal values do not change while a process runs.
                Inst::SigVal { .. } | Inst::SigReadScalar { .. } => class = Class::Movable,
                Inst::SigRead { .. } => {
                    class = Class::Movable;
                    wrote = Some(orgof(0));
                }
                Inst::MakeDope { .. }
                | Inst::Store {
                    w: Width::B1 | Width::B4 | Width::B8,
                    ..
                } => {
                    class = Class::Movable;
                    wrote = Some(orgof(0));
                }
                Inst::Cat { parts, scalars, .. } => {
                    class = Class::Movable;
                    // Slots: dst, n, m, then the list (address and count per piece).
                    wrote = Some(orgof(0));
                    let base = reads.len() - parts.len as usize;
                    for k in 0..parts.len as usize / 2 {
                        if scalars >> k & 1 == 0 {
                            eff.reads.push(orgof(base + 2 * k));
                        }
                    }
                }
                Inst::CallNative { native, args, .. } => {
                    if self.is_image(native, args.len as usize) {
                        class = Class::Movable;
                        eff.creates = true;
                        my_org = Org::Img(p as u32);
                    } else if self.light_native(native) {
                        // A new block, nothing else written.
                        my_org = Org::Temp(p as u32);
                    } else {
                        barrier = true;
                    }
                }
                Inst::Stmt
                | Inst::TempRoot { .. }
                | Inst::Suspend { .. }
                | Inst::Call { .. }
                | Inst::CallProt { .. }
                | Inst::CallForeign { .. }
                | Inst::CallIndirect { .. }
                | Inst::New { .. }
                | Inst::NewFill { .. }
                | Inst::NewFrame { .. }
                | Inst::Free { .. }
                | Inst::Report { .. }
                | Inst::Elab { .. } => barrier = true,
                Inst::Deref { .. } => my_org = Org::Heap,
                // Writes into the block the first operand addresses, and no other.
                Inst::Copy { .. }
                | Inst::Fill { .. }
                | Inst::CopyConst { .. }
                | Inst::LCopy { .. }
                | Inst::LSet { .. }
                | Inst::LFill { .. }
                | Inst::LogicOp { .. } => wrote = Some(orgof(0)),
                _ => {}
            }
            // What is not named writes memory nobody can place: it may reach any block.
            if class != Class::Movable
                && !barrier
                && wrote.is_none()
                && !writes_nothing(&i)
                && !matches!(i, Inst::CallNative { .. })
            {
                wrote = Some(Org::Unknown);
            }
            r.class[p] = class;
            r.eff[p] = eff;
            r.wrote[p] = wrote;
            r.barrier[p] = barrier;
            r.suspends[p] = match i {
                Inst::Suspend { .. } | Inst::CallIndirect { .. } | Inst::Elab { .. } => true,
                Inst::Call { func, .. } | Inst::CallProt { func, .. } => self
                    .prog
                    .funcs
                    .get(func)
                    .is_none_or(|c| c.flags.contains(FuncFlags::MAY_SUSPEND)),
                _ => false,
            };
            org[p] = my_org;
            len[p] = my_len;
            let ds = Self::writes(&i);
            for d in &ds {
                r.last_def.insert(*d, p);
            }
            r.defs[p] = ds;
        }
        r
    }
}

/// Instructions that write no memory other than a new block: values, reads, waits, drivers.
fn writes_nothing(i: &Inst) -> bool {
    matches!(
        i,
        Inst::Const { .. }
            | Inst::ConstF { .. }
            | Inst::Move { .. }
            | Inst::AddrOf { .. }
            | Inst::Select { .. }
            | Inst::Alloc { .. }
            | Inst::LayoutOf { .. }
            | Inst::Load { .. }
            | Inst::Pos { .. }
            | Inst::SlicePos { .. }
            | Inst::Index { .. }
            | Inst::Field { .. }
            | Inst::Bin { .. }
            | Inst::Un { .. }
            | Inst::FBin { .. }
            | Inst::FUn { .. }
            | Inst::Phys { .. }
            | Inst::IToF { .. }
            | Inst::FToI { .. }
            | Inst::FToIRange { .. }
            | Inst::Cmp { .. }
            | Inst::FCmp { .. }
            | Inst::Bool { .. }
            | Inst::LEq { .. }
            | Inst::MemEq { .. }
            | Inst::LGet { .. }
            | Inst::Check { .. }
            | Inst::FCheck { .. }
            | Inst::BinRange { .. }
            | Inst::DopeLen { .. }
            | Inst::SubtypeCheck { .. }
            | Inst::Deref { .. }
            | Inst::SigVal { .. }
            | Inst::SigParts { .. }
            | Inst::SigSlice { .. }
            | Inst::SigReadScalar { .. }
            | Inst::Edge { .. }
            | Inst::Drive { .. }
            | Inst::DriveSlot { .. }
            | Inst::WaitOn { .. }
            | Inst::WaitEdge { .. }
            | Inst::WaitFor { .. }
            | Inst::WaitAt { .. }
            | Inst::Now { .. }
            | Inst::Delta { .. }
            | Inst::TimedOut { .. }
            | Inst::Cover { .. }
            | Inst::Br { .. }
            | Inst::CondBr { .. }
            | Inst::Case { .. }
            | Inst::Ret { .. }
            | Inst::Unreachable { .. }
    )
}

/// Can moved memory effect `a` (a block it reads) be reached by a write to `w`?
fn alias(w: Org, r: Org) -> bool {
    match (w, r) {
        (Org::Unknown, _) | (_, Org::Unknown) => true,
        (Org::Heap, _) | (_, Org::Heap) | (Org::Ro, _) | (_, Org::Ro) => false,
        (Org::Frame, Org::Frame) => true,
        (a, b) => a == b,
    }
}

impl Sink<'_> {
    fn try_block(&mut self, x: usize) -> bool {
        // The straight run: `x`, then the blocks only the one before reaches, to a `CondBr`.
        let mut blocks = vec![x];
        let mut cur = x;
        let (t, e) = loop {
            match self.code[cur].last() {
                Some(&Inst::CondBr { t, f, .. }) => break (t.0 as usize, f.0 as usize),
                Some(&Inst::Br { to }) => {
                    let to = to.0 as usize;
                    if blocks.len() >= 6 || self.preds[to].len() != 1 || blocks.contains(&to) {
                        return false;
                    }
                    blocks.push(to);
                    cur = to;
                }
                _ => return false,
            }
        };
        let xlen = self.code[x].len() - 1;
        let replay = self.replay.contains(&t) || self.replay.contains(&e);
        if xlen < if replay { MIN_REPLAY } else { MIN_SUNK } {
            return false;
        }
        let mut seq = Seq {
            insts: Vec::new(),
            loc: Vec::new(),
            via: Vec::new(),
        };
        for (k, &b) in blocks.iter().enumerate() {
            let n = self.code[b].len();
            let take = if k + 1 == blocks.len() { n } else { n - 1 };
            seq.insts.extend_from_slice(&self.code[b][..take]);
            seq.loc.extend_from_slice(&self.loc[b][..take]);
            if self.via[b].is_empty() {
                seq.via.extend(std::iter::repeat_n(0, take));
            } else {
                seq.via.extend_from_slice(&self.via[b][..take]);
            }
        }
        if seq.insts.len() > MAX_SEQ {
            return false;
        }
        // (Only the straight runs that start a string, or end in the replay of a call, have
        // their liveness looked at.)
        if !replay
            && !seq.insts[..xlen]
                .iter()
                .any(|i| matches!(i, Inst::Cat { logic: false, .. }))
        {
            return false;
        }
        let run = self.analyse(seq, xlen);
        if self.live_in.is_empty() {
            self.liveness();
        }
        for (target, other) in [(t, e), (e, t)] {
            if target == x || blocks.contains(&target) || self.preds[target] != [cur as u32] {
                continue;
            }
            if let Some(lazy) = self.plan(&run, target, other, replay) {
                self.apply(&run, &lazy, x, target);
                return true;
            }
        }
        false
    }

    /// The set of positions of the run that can move to the start of `target` (`other` is the
    /// branch not taken), or `None` when it is too small.
    fn plan(&self, r: &Run, target: usize, other: usize, replay: bool) -> Option<Vec<bool>> {
        let n = r.seq.insts.len();
        let in_u = &self.live_in[other];
        let mut out_t = self.live_out(target);
        // What the target writes is not the moved value any more at its end.
        for i in &self.code[target] {
            for d in Self::writes(i) {
                if let Some(w) = out_t.get_mut(d.0 as usize / 64) {
                    *w &= !(1 << (d.0 % 64));
                }
            }
        }
        let mut lazy: Vec<bool> = (0..n)
            .map(|p| p < r.xlen && r.class[p] == Class::Movable)
            .collect();
        loop {
            let mut changed = false;
            for p in 0..r.xlen {
                if !lazy[p] {
                    continue;
                }
                let mut ok = true;
                // Readers of a result move too; a result that reaches the branch is read in the
                // target only.
                if r.users[p].iter().any(|&q| q >= r.xlen || !lazy[q]) {
                    ok = false;
                }
                if ok {
                    for d in &r.defs[p] {
                        if r.last_def.get(d) == Some(&p) && (bit(in_u, *d) || bit(&out_t, *d)) {
                            ok = false;
                        }
                    }
                }
                // An operand an instruction that stays overwrites later.
                if ok {
                    'ops: for &(o, d) in &r.uses[p] {
                        if d.is_some_and(|d| lazy[d]) {
                            continue;
                        }
                        for q in p + 1..n {
                            if (q >= r.xlen || !lazy[q]) && r.defs[q].contains(&o) {
                                ok = false;
                                break 'ops;
                            }
                        }
                    }
                }
                // No register lives across a suspension.
                if ok && (p + 1..n).any(|q| (q >= r.xlen || !lazy[q]) && r.suspends[q]) {
                    ok = false;
                }
                // Memory: reads must not meet a later write, allocations and writes no barrier.
                if ok {
                    let creates_or_writes = r.eff[p].creates || r.wrote[p].is_some();
                    for q in p + 1..n {
                        if q < r.xlen && lazy[q] {
                            continue;
                        }
                        if creates_or_writes && r.barrier[q] {
                            ok = false;
                            break;
                        }
                        if r.barrier[q]
                            && r.eff[p]
                                .reads
                                .iter()
                                .any(|o| !matches!(o, Org::Ro | Org::Heap))
                        {
                            ok = false;
                            break;
                        }
                        if let Some(w) = r.wrote[q]
                            && r.eff[p].reads.iter().any(|&o| alias(w, o))
                        {
                            ok = false;
                            break;
                        }
                    }
                    // Reads only of blocks the run made (or read-only memory).
                    for &o in &r.eff[p].reads {
                        if !matches!(o, Org::Ro | Org::Frame | Org::Img(_) | Org::Temp(_)) {
                            ok = false;
                        }
                    }
                    if let Some(w) = r.wrote[p] {
                        // Only into a block a moved instruction made.
                        let own = matches!(w, Org::Temp(k) if lazy.get(k as usize).copied().unwrap_or(false));
                        if !own {
                            ok = false;
                        }
                    }
                }
                if !ok {
                    lazy[p] = false;
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        let count = lazy.iter().filter(|&&l| l).count();
        (count >= if replay { MIN_REPLAY } else { MIN_SUNK }).then_some(lazy)
    }

    fn apply(&mut self, r: &Run, lazy: &[bool], x: usize, target: usize) {
        let n = r.seq.insts.len();
        // Fresh registers for the moved definitions, reused once their last reader has run.
        let mut fresh: Vec<Vec<Reg>> = vec![Vec::new(); n];
        let mut last_use: Vec<usize> = vec![0; n];
        let live_t = self.live_in[target].clone();
        for p in 0..r.xlen {
            if !lazy[p] {
                continue;
            }
            last_use[p] = r.users[p].iter().copied().max().unwrap_or(p);
            // A result the target reads stays live to the end.
            if r.defs[p]
                .iter()
                .any(|d| r.last_def.get(d) == Some(&p) && bit(&live_t, *d))
            {
                last_use[p] = usize::MAX;
            }
        }
        let mut pool: HashMap<RegTy, Vec<Reg>> = HashMap::new();
        let mut held: Vec<(usize, Reg, RegTy)> = Vec::new();
        for p in 0..r.xlen {
            if !lazy[p] {
                continue;
            }
            held.retain(|&(lu, reg, ty)| {
                if lu < p {
                    pool.entry(ty).or_default().push(reg);
                    false
                } else {
                    true
                }
            });
            for d in &r.defs[p] {
                let ty = self.reg_ty[d.0 as usize];
                let reg = match pool.get_mut(&ty).and_then(Vec::pop) {
                    Some(reg) => reg,
                    None => {
                        self.reg_ty.push(ty);
                        let reg = Reg(self.reg_ty.len() as u32 - 1);
                        // A signal reference takes the next register as well.
                        if ty == RegTy::Sig {
                            self.reg_ty.push(RegTy::I64);
                        }
                        reg
                    }
                };
                fresh[p].push(reg);
                held.push((last_use[p], reg, ty));
            }
        }
        // The moved instructions with fresh definitions and the readers renamed.
        let mut moved: Vec<(Inst, StmtId, u32)> = Vec::new();
        for p in 0..r.xlen {
            if !lazy[p] {
                continue;
            }
            let mut inst = r.seq.insts[p];
            let mut k = 0;
            let mut dk = 0;
            let uses = &r.uses[p];
            inst.visit_mut(&mut |reg, role, _| match role {
                Role::Use => {
                    if let Some(d) = uses[k].1.filter(|&d| lazy[d]) {
                        *reg = r.defs[d]
                            .iter()
                            .position(|x| *x == uses[k].0)
                            .map_or(*reg, |j| fresh[d][j]);
                    }
                    k += 1;
                }
                Role::Def => {
                    *reg = fresh[p][dk];
                    dk += 1;
                }
            });
            if let Some(l) = inst.list() {
                let old = self.list(&inst).to_vec();
                let mut changed = false;
                let mut new = old.clone();
                for (j, o) in new.iter_mut().enumerate() {
                    if let Some(d) = uses[k + j].1.filter(|&d| lazy[d])
                        && let Some(jj) = r.defs[d].iter().position(|x| *x == *o)
                    {
                        *o = fresh[d][jj];
                        changed = true;
                    }
                }
                if changed {
                    let start = self.operands.len() as u32;
                    self.operands.extend_from_slice(&new);
                    set_list(&mut inst, crate::Regs { start, len: l.len });
                }
            }
            moved.push((inst, r.seq.loc[p], r.seq.via[p]));
        }
        // The results the target reads: the register they had, the fresh one now.
        let mut into_target: HashMap<Reg, Reg> = HashMap::new();
        for p in 0..r.xlen {
            if !lazy[p] {
                continue;
            }
            for (j, d) in r.defs[p].iter().enumerate() {
                if r.last_def.get(d) == Some(&p) && bit(&live_t, *d) {
                    into_target.insert(*d, fresh[p][j]);
                }
            }
        }
        // The block `x` without them.
        let mut keep_code = Vec::new();
        let mut keep_loc = Vec::new();
        let mut keep_via = Vec::new();
        let has_via = !self.via[x].is_empty();
        for p in 0..r.xlen {
            if !lazy[p] {
                keep_code.push(self.code[x][p]);
                keep_loc.push(self.loc[x][p]);
                if has_via {
                    keep_via.push(self.via[x][p]);
                }
            }
        }
        let term = self.code[x].len() - 1;
        keep_code.push(self.code[x][term]);
        keep_loc.push(self.loc[x][term]);
        if has_via {
            keep_via.push(self.via[x][term]);
        }
        self.code[x] = keep_code;
        self.loc[x] = keep_loc;
        self.via[x] = keep_via;
        // The target: the moved code first, then its own with the renamed reads.
        let tcode = std::mem::take(&mut self.code[target]);
        let mut tloc = std::mem::take(&mut self.loc[target]);
        let mut tvia = std::mem::take(&mut self.via[target]);
        let t_has_via = !tvia.is_empty();
        let mut out_code: Vec<Inst> = moved.iter().map(|m| m.0).collect();
        let mut out_loc: Vec<StmtId> = moved.iter().map(|m| m.1).collect();
        let mut out_via: Vec<u32> = Vec::new();
        let need_via = t_has_via || moved.iter().any(|m| m.2 != 0) || has_via;
        if need_via {
            out_via.extend(moved.iter().map(|m| m.2));
            if !t_has_via {
                tvia = vec![0; tcode.len()];
            }
        }
        let mut defined: std::collections::HashSet<Reg> = std::collections::HashSet::new();
        for mut i in tcode {
            let mut c = i;
            c.visit_mut(&mut |reg, role, _| {
                if role == Role::Use
                    && !defined.contains(reg)
                    && let Some(f) = into_target.get(reg)
                {
                    *reg = *f;
                }
            });
            if let Some(l) = c.list() {
                let old = self.list(&c).to_vec();
                let new: Vec<Reg> = old
                    .iter()
                    .map(|o| {
                        if defined.contains(o) {
                            *o
                        } else {
                            into_target.get(o).copied().unwrap_or(*o)
                        }
                    })
                    .collect();
                if new != old {
                    let start = self.operands.len() as u32;
                    self.operands.extend_from_slice(&new);
                    set_list(&mut c, crate::Regs { start, len: l.len });
                }
            }
            for d in Self::writes(&i) {
                defined.insert(d);
            }
            i = c;
            out_code.push(i);
        }
        out_loc.append(&mut tloc);
        if need_via {
            out_via.append(&mut tvia);
        }
        self.code[target] = out_code;
        self.loc[target] = out_loc;
        self.via[target] = out_via;
    }
}

impl Sink<'_> {
    fn find_preds(&mut self) {
        let nb = self.code.len();
        self.preds = vec![Vec::new(); nb];
        for b in 0..nb {
            for t in self.succs(b) {
                let t = t as usize;
                if t < nb && !self.preds[t].contains(&(b as u32)) {
                    self.preds[t].push(b as u32);
                }
            }
        }
    }

    /// Sink what can be sunk, run by run, until nothing moves (at most 8 runs).
    fn sink_all(&mut self) -> bool {
        let mut changed = false;
        for _ in 0..8 {
            self.live_in.clear();
            let mut any = false;
            for x in 0..self.code.len() {
                if self.code[x].len() > MIN_REPLAY && self.try_block(x) {
                    any = true;
                    break;
                }
            }
            if !any {
                break;
            }
            changed = true;
        }
        changed
    }
}

fn set_list(i: &mut Inst, l: crate::Regs) {
    if let Some(slot) = i.list_mut() {
        *slot = l;
    }
}

/// Sink what the branches after the code of `f` leave unread; whether anything moved.
pub(super) fn run(prog: &Program, f: &mut crate::Func) -> bool {
    if keep_variables_on()
        || f.flags.contains(FuncFlags::HAS_COVER)
        || !matches!(
            f.kind,
            FuncKind::Function | FuncKind::Procedure | FuncKind::Process
        )
    {
        return false;
    }
    let nb = f.blocks.len();
    // Only a string built piece by piece is worth it.
    if nb == 0
        || f.reg_ty.len().saturating_mul(nb) > BUDGET
        || !f
            .code
            .iter()
            .any(|i| matches!(i, Inst::Cat { logic: false, .. }))
    {
        return false;
    }
    let mut code: Vec<Vec<Inst>> = Vec::with_capacity(nb);
    let mut loc: Vec<Vec<StmtId>> = Vec::with_capacity(nb);
    let mut via: Vec<Vec<u32>> = Vec::with_capacity(nb);
    for b in f.blocks.values() {
        let r = b.start as usize..b.end as usize;
        code.push(f.code[r.clone()].to_vec());
        loc.push(f.loc[r.clone()].to_vec());
        via.push(if f.via.is_empty() {
            Vec::new()
        } else {
            f.via[r].to_vec()
        });
    }
    let mut defs = vec![0u8; f.reg_ty.len()];
    let mut ro = vec![false; f.reg_ty.len()];
    let mut frame = vec![false; f.reg_ty.len()];
    let mut konst = vec![None; f.reg_ty.len()];
    let mut addr = vec![None; f.reg_ty.len()];
    for i in &f.code {
        i.visit(&mut |r, role, _| {
            if role == Role::Def {
                defs[r.0 as usize] = defs[r.0 as usize].saturating_add(1);
            }
        });
        if let Inst::AddrOf { dst, space, off } = *i {
            addr[dst.0 as usize] = Some((space, off));
        }
        match *i {
            Inst::AddrOf {
                dst,
                space: AddrSpace::Inst | AddrSpace::Const(_),
                ..
            } => ro[dst.0 as usize] = true,
            Inst::AddrOf {
                dst,
                space: AddrSpace::Frame,
                ..
            } => frame[dst.0 as usize] = true,
            Inst::Const { dst, v } => konst[dst.0 as usize] = Some(v),
            _ => {}
        }
    }
    for r in 0..defs.len() {
        if defs[r] != 1 {
            ro[r] = false;
            frame[r] = false;
            konst[r] = None;
            addr[r] = None;
        }
    }
    let mut s = Sink {
        prog,
        code,
        loc,
        via,
        operands: f.operands.clone(),
        reg_ty: f.reg_ty.clone(),
        preds: vec![Vec::new(); nb],
        ro,
        frame,
        konst,
        addr,
        live_in: Vec::new(),
        replay: Vec::new(),
    };
    s.find_preds();
    let mut changed = s.sink_all();
    // A call repeated on the same value is made once (`calls.rs`); what its second argument
    // built then sinks in front of the replay.
    if s.share_calls() {
        s.find_preds();
        s.sink_all();
        changed = true;
    }
    if !changed {
        return false;
    }
    let nb = s.code.len();
    let (mut code, mut locs, mut vias) = (Vec::new(), Vec::new(), Vec::new());
    let has_via = !f.via.is_empty() || s.via.iter().any(|v| !v.is_empty());
    for b in 0..nb {
        let start = code.len() as u32;
        code.extend_from_slice(&s.code[b]);
        locs.extend_from_slice(&s.loc[b]);
        if has_via {
            if s.via[b].is_empty() {
                vias.extend(std::iter::repeat_n(0, s.code[b].len()));
            } else {
                vias.extend_from_slice(&s.via[b]);
            }
        }
        let end = code.len() as u32;
        match f.blocks.get_mut(BlockId(b as u32)) {
            Some(blk) => {
                blk.start = start;
                blk.end = end;
            }
            None => {
                f.blocks.push(crate::Block { start, end });
            }
        }
    }
    f.code = code;
    f.loc = locs;
    f.via = vias;
    f.operands = s.operands;
    f.reg_ty = s.reg_ty;
    true
}
