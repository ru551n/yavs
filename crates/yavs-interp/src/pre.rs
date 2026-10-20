//! Per-function facts computed once, before a function first runs (#412).
//!
//! A register defined exactly once by a `Const`, `ConstF` or `AddrOf` holds the same value
//! whenever it is read, so a new frame starts with it instead of every pass over the
//! instruction executing it again (lowering emits these at each use, in loops too). The
//! instruction stays in the code, so compiled code, pc numbering and the debug loop are
//! unchanged; the IR loop (`Machine::exec_in`) skips it through `next`. An unconditional jump
//! is skipped the same way: `next` leads to its target.
//!
//! The fast loop (`Machine::exec_fast`, no debugger and no compiled tier) does not run the IR
//! itself but `Code`, a predecoded form of it made here (#431): one small `Op` per executed
//! instruction, without the skipped definitions and threaded jumps, with branch targets
//! resolved to op indices, loads and stores through a preloaded frame or instance address
//! reading that memory directly, an offset by a constant folded into the access, a compare
//! fused with the branch on it and a `LayoutOf` with the one instruction that reads its
//! result. Everything else is `Op::Step`: the IR instruction
//! through `Machine::step`. `Code::at` maps each op back to its IR pc, which frames keep
//! (`Frame::pc`), so traces, messages, coverage, root maps and the debug loop see the IR.

use yavs_ir::{AddrSpace, CheckKind, CheckMsgId, CmpOp, Func, Inst, IntOp, Role, Width};

use crate::exec::{CONST, FRAME, INST, PKG, R, SHARED, mk_addr};

pub(crate) struct Pre {
    /// Initial registers: the preloaded values, zero elsewhere.
    init: Vec<u64>,
    /// Registers holding an address in the frame itself: (register, offset).
    frame: Vec<(u32, u64)>,
    /// For each pc (and one past the end), the first pc at or after it that is not a
    /// preloaded definition.
    pub next: Vec<u32>,
    /// The predecoded code of the fast loop.
    pub code: Code,
}

impl Pre {
    /// Append the registers of a frame at index `idx` to `stack`: the preloaded registers have
    /// their values, the others are zero.
    ///
    /// ponytail: zeroing is not needed for correctness (#430: the verifier proves every other
    /// register is written before it is read; a sparse fill must list preloaded zeros too),
    /// but the collection scans registers conservatively and stale ones kept garbage alive
    /// (+25% run time, +31% peak memory on a long testbench). Leave them stale once the scan
    /// is precise (#422).
    pub(crate) fn fill(&self, stack: &mut Vec<u64>, idx: u32) -> R<()> {
        let at = stack.len();
        stack.extend_from_slice(&self.init);
        if idx >= 1 << 29 {
            // The error of `mk_addr`.
            if let Some(&(_, off)) = self.frame.first() {
                mk_addr(FRAME, idx, off)?;
            }
        }
        // Offsets are `u32`s, so `mk_addr` cannot fail below the index bound.
        let hi = u64::from(FRAME) << 61 | u64::from(idx) << 32;
        for &(r, off) in &self.frame {
            stack[at + r as usize] = hi | off;
        }
        Ok(())
    }

    pub(crate) fn new(f: &Func) -> Pre {
        let n = f.reg_ty.len();
        let mut defs = vec![0u8; n];
        for inst in &f.code {
            inst.visit(&mut |r, role, _| {
                if role == Role::Def
                    && let Some(d) = defs.get_mut(r.0 as usize)
                {
                    *d = d.saturating_add(1);
                }
            });
        }
        let mut init = vec![0u64; n];
        let mut frame = Vec::new();
        let mut known = vec![Known::None; n];
        let mut next = vec![f.code.len() as u32; f.code.len() + 1];
        let mut skip = vec![false; f.code.len()];
        for pc in (0..f.code.len()).rev() {
            let once = |r: yavs_ir::Reg| defs.get(r.0 as usize) == Some(&1);
            let hit = match f.code[pc] {
                Inst::Const { dst, v } if once(dst) => {
                    init[dst.0 as usize] = v as u64;
                    known[dst.0 as usize] = Known::Int(v);
                    true
                }
                Inst::ConstF { dst, v } if once(dst) => {
                    init[dst.0 as usize] = v.to_bits();
                    true
                }
                Inst::AddrOf { dst, space, off } if once(dst) => {
                    let off = u64::from(off);
                    let a = match space {
                        AddrSpace::Frame => {
                            frame.push((dst.0, off));
                            known[dst.0 as usize] = Known::Frame(off as u32);
                            Some(0)
                        }
                        AddrSpace::Up(_) => None,
                        AddrSpace::Inst => {
                            known[dst.0 as usize] = Known::Inst(off as u32);
                            mk_addr(INST, 0, off).ok()
                        }
                        AddrSpace::Pkg(p) => mk_addr(PKG, p, off).ok(),
                        AddrSpace::Const(c) => mk_addr(CONST, c.0, off).ok(),
                        AddrSpace::Shared => mk_addr(SHARED, 0, off).ok(),
                    };
                    if let Some(a) = a {
                        init[dst.0 as usize] = a;
                    }
                    a.is_some()
                }
                _ => false,
            };
            next[pc] = if hit { next[pc + 1] } else { pc as u32 };
            skip[pc] = hit;
        }
        // An unconditional jump costs nothing: execution goes on at its target (through
        // other jumps and definitions the frame started with). Not to the block a machine
        // pauses at.
        let jump = |pc: usize| match f.code.get(pc) {
            Some(&Inst::Br { to }) if f.body != Some(to) => f.blocks.get(to).map(|b| b.start),
            _ => None,
        };
        let thread: Vec<Option<u32>> = (0..f.code.len())
            .map(|pc| {
                let (mut at, mut hops) = (pc, 0);
                while let Some(start) = jump(at) {
                    at = next[start as usize] as usize;
                    hops += 1;
                    if hops > 32 || at == pc {
                        return None;
                    }
                }
                (hops > 0).then_some(at as u32)
            })
            .collect();
        for n in &mut next {
            if let Some(Some(t)) = thread.get(*n as usize) {
                *n = *t;
            }
        }
        let code = Code::new(f, &skip, &known, &next);
        Pre {
            init,
            frame,
            next,
            code,
        }
    }
}

/// What the encoder knows about a register a frame starts with.
#[derive(Clone, Copy, PartialEq)]
enum Known {
    None,
    Int(i64),
    /// The address of this offset in the frame's own slots.
    Frame(u32),
    /// The address of this offset in the instance data.
    Inst(u32),
}

/// One predecoded operation (`Code`). Registers are `u32` indices into the frame's
/// registers, jump targets op indices. Widths are never `Width::Sig` (those go to `Step`).
#[derive(Clone, Copy, Debug)]
pub(crate) enum Op {
    /// The IR instruction at this pc, through `Machine::step`.
    Step(u32),
    Stmt,
    Move {
        dst: u32,
        src: u32,
    },
    Load {
        dst: u32,
        addr: u32,
        w: Width,
    },
    Store {
        addr: u32,
        src: u32,
        w: Width,
    },
    /// A load from the frame's own slots at `off`.
    LoadSlot {
        dst: u32,
        off: u32,
        w: Width,
    },
    StoreSlot {
        src: u32,
        off: u32,
        w: Width,
    },
    /// A load from the instance data at `off`.
    LoadInst {
        dst: u32,
        off: u32,
        w: Width,
    },
    /// A load at `base + off` (an `Index` by a constant whose result only this load reads).
    LoadOff {
        dst: u32,
        base: u32,
        off: u32,
        w: Width,
    },
    StoreOff {
        base: u32,
        off: u32,
        src: u32,
        w: Width,
    },
    Index {
        dst: u32,
        base: u32,
        pos: u32,
        stride: u32,
    },
    /// `Index` by a constant: `base + off`.
    IndexImm {
        dst: u32,
        base: u32,
        off: u32,
    },
    Pos {
        dst: u32,
        dope: u32,
        idx: u32,
        what: CheckMsgId,
        dim: u8,
    },
    Bin {
        op: IntOp,
        dst: u32,
        a: u32,
        b: u32,
    },
    /// `Inst::BinRange`; its message is read from the IR when it fails.
    BinRange {
        op: IntOp,
        dst: u32,
        a: u32,
        b: u32,
        lo: u32,
        hi: u32,
    },
    Cmp {
        op: CmpOp,
        dst: u32,
        a: u32,
        b: u32,
    },
    /// `Cmp` and the `CondBr` on its result.
    CmpBr {
        op: CmpOp,
        dst: u32,
        a: u32,
        b: u32,
        t: u32,
        f: u32,
    },
    Check {
        kind: CheckKind,
        v: u32,
        lo: u32,
        hi: u32,
        what: CheckMsgId,
    },
    Br {
        to: u32,
    },
    CondBr {
        c: u32,
        t: u32,
        f: u32,
    },
    /// A jump to the block a process pauses at (`Machine::pause`).
    BrBody {
        to: u32,
        block: u32,
    },
    SigVal {
        dst: u32,
        sig: u32,
        offset: u32,
        len: u32,
    },
    DopeLen {
        dst: u32,
        dope: u32,
        dim: u8,
    },
    Alloc {
        dst: u32,
        size: u32,
    },
    /// `Inst::Call`; `dst` is `NONE` for a procedure.
    Call {
        func: u32,
        start: u32,
        len: u32,
        dst: u32,
    },
    /// `Inst::CallNative`; `dst` is `NONE` without a result.
    CallNative {
        native: u32,
        start: u32,
        len: u32,
        dst: u32,
    },
    SigReadScalar {
        dst: u32,
        sig: u32,
    },
    WaitOn {
        sig: u32,
    },
    Suspend,
    /// `Inst::Ret`; `v` is `NONE` without a value.
    Ret {
        v: u32,
    },
    /// `LayoutOf` of type `ty`.
    Layout {
        dst: u32,
        ty: u32,
        desc: u32,
    },
    /// Word `k` of a layout that only this load reads (`LayoutOf`, then a load of its size or
    /// of a word at a constant offset).
    LayoutWord {
        dst: u32,
        ty: u32,
        desc: u32,
        k: u32,
    },
    /// `LayoutOf` and the `Field` that is its only reader: the address of field `n`.
    FieldOf {
        dst: u32,
        base: u32,
        ty: u32,
        desc: u32,
        n: u32,
    },
}

// Small enough for three ops to a cache line.
const _: () = assert!(size_of::<Op>() <= 24);

/// A function's predecoded code (#431).
pub(crate) struct Code {
    pub ops: Vec<Op>,
    /// The IR pc of each op (of its first instruction when it stands for several).
    pub at: Vec<u32>,
    /// For each IR pc (and one past the end), the op execution goes on at from there;
    /// `NONE` inside a fused op, which no frame ever stops at.
    pub entry: Vec<u32>,
}

pub(crate) const NONE: u32 = u32::MAX;

impl Code {
    fn new(f: &Func, skip: &[bool], known: &[Known], next: &[u32]) -> Code {
        let n = f.code.len();
        let mut block_start = vec![false; n + 1];
        for (_, b) in f.blocks.iter() {
            if let Some(s) = block_start.get_mut(b.start as usize) {
                *s = true;
            }
        }
        let mut uses = vec![0u32; f.reg_ty.len()];
        for inst in &f.code {
            inst.visit(&mut |r, role, _| {
                if role == Role::Use
                    && let Some(u) = uses.get_mut(r.0 as usize)
                {
                    *u = u.saturating_add(1);
                }
            });
        }
        for r in &f.operands {
            if let Some(u) = uses.get_mut(r.0 as usize) {
                *u = u.saturating_add(1);
            }
        }
        let int = |r: yavs_ir::Reg| match known.get(r.0 as usize) {
            Some(&Known::Int(v)) => Some(v),
            _ => None,
        };
        // The IR pc of a block's first instruction; jumps to the block a process pauses at
        // stay `Step`s (the pause check is in the step path).
        let target = |b: yavs_ir::BlockId| match f.blocks.get(b) {
            Some(blk) if f.body != Some(b) => Some(blk.start),
            _ => None,
        };
        let mut ops = Vec::with_capacity(n);
        let mut at = Vec::with_capacity(n);
        let mut start = vec![NONE; n + 1];
        let mut pc = 0;
        while pc < n {
            if skip[pc] {
                pc += 1;
                continue;
            }
            // The instruction run after `p` within the block, when only skipped definitions
            // are between: it can be fused into this op (no frame ever stops there).
            let after = |p: usize| {
                let mut nx = p + 1;
                while nx < n && skip[nx] && !block_start[nx] {
                    nx += 1;
                }
                (nx < n && !block_start[nx]).then(|| (nx, f.code[nx]))
            };
            let (nx, fusable) = match after(pc) {
                Some((p, i)) => (p, Some(i)),
                None => (pc, None),
            };
            let once = |r: yavs_ir::Reg| uses.get(r.0 as usize) == Some(&1);
            start[pc] = ops.len() as u32;
            at.push(pc as u32);
            // The last instruction this op stands for.
            let mut last = pc;
            let op = match f.code[pc] {
                Inst::Stmt => Op::Stmt,
                Inst::Move { dst, src } => Op::Move {
                    dst: dst.0,
                    src: src.0,
                },
                Inst::Load { dst, addr, w } if w != Width::Sig => {
                    match known.get(addr.0 as usize) {
                        Some(&Known::Frame(off)) => Op::LoadSlot { dst: dst.0, off, w },
                        Some(&Known::Inst(off)) => Op::LoadInst { dst: dst.0, off, w },
                        _ => Op::Load {
                            dst: dst.0,
                            addr: addr.0,
                            w,
                        },
                    }
                }
                Inst::Store { addr, src, w } if w != Width::Sig => {
                    match known.get(addr.0 as usize) {
                        Some(&Known::Frame(off)) => Op::StoreSlot { src: src.0, off, w },
                        _ => Op::Store {
                            addr: addr.0,
                            src: src.0,
                            w,
                        },
                    }
                }
                Inst::Index {
                    dst,
                    base,
                    pos,
                    stride,
                } => {
                    let off = int(pos)
                        .and_then(|p| u64::try_from(p).ok())
                        .and_then(|p| p.checked_mul(u64::from(stride)))
                        .and_then(|o| u32::try_from(o).ok());
                    match (off, fusable) {
                        (Some(off), Some(Inst::Load { dst: d, addr, w }))
                            if once(dst) && addr == dst && w != Width::Sig =>
                        {
                            last = nx;
                            Op::LoadOff {
                                dst: d.0,
                                base: base.0,
                                off,
                                w,
                            }
                        }
                        (Some(off), Some(Inst::Store { addr, src, w }))
                            if once(dst) && addr == dst && src != dst && w != Width::Sig =>
                        {
                            last = nx;
                            Op::StoreOff {
                                base: base.0,
                                off,
                                src: src.0,
                                w,
                            }
                        }
                        (Some(off), _) => Op::IndexImm {
                            dst: dst.0,
                            base: base.0,
                            off,
                        },
                        _ => Op::Index {
                            dst: dst.0,
                            base: base.0,
                            pos: pos.0,
                            stride,
                        },
                    }
                }
                Inst::Pos {
                    dst,
                    dope,
                    dim,
                    idx,
                    what,
                } => Op::Pos {
                    dst: dst.0,
                    dope: dope.0,
                    idx: idx.0,
                    what,
                    dim,
                },
                Inst::Bin { op, dst, a, b } => Op::Bin {
                    op,
                    dst: dst.0,
                    a: a.0,
                    b: b.0,
                },
                Inst::BinRange {
                    op,
                    dst,
                    a,
                    b,
                    lo,
                    hi,
                    ..
                } => Op::BinRange {
                    op,
                    dst: dst.0,
                    a: a.0,
                    b: b.0,
                    lo: lo.0,
                    hi: hi.0,
                },
                Inst::Cmp { op, dst, a, b } => match fusable {
                    Some(Inst::CondBr { c, t, f: e })
                        if c == dst && target(t).is_some() && target(e).is_some() =>
                    {
                        last = nx;
                        Op::CmpBr {
                            op,
                            dst: dst.0,
                            a: a.0,
                            b: b.0,
                            t: target(t).unwrap_or(0),
                            f: target(e).unwrap_or(0),
                        }
                    }
                    _ => Op::Cmp {
                        op,
                        dst: dst.0,
                        a: a.0,
                        b: b.0,
                    },
                },
                Inst::Check {
                    kind,
                    v,
                    lo,
                    hi,
                    what,
                } => Op::Check {
                    kind,
                    v: v.0,
                    lo: lo.0,
                    hi: hi.0,
                    what,
                },
                Inst::LayoutOf { dst, ty, desc } => {
                    let word = |d: yavs_ir::Reg, k: u32| Op::LayoutWord {
                        dst: d.0,
                        ty: ty.0,
                        desc: desc.0,
                        k,
                    };
                    let fused = match fusable {
                        _ if !once(dst) => None,
                        Some(Inst::Field {
                            dst: d,
                            base,
                            layout,
                            n,
                        }) if layout == dst => Some((
                            nx,
                            Op::FieldOf {
                                dst: d.0,
                                base: base.0,
                                ty: ty.0,
                                desc: desc.0,
                                n,
                            },
                        )),
                        Some(Inst::Load { dst: d, addr, w }) if addr == dst && w == Width::B8 => {
                            Some((nx, word(d, 0)))
                        }
                        // A word at a constant offset: `Index` of the layout, then a load.
                        Some(Inst::Index {
                            dst: i,
                            base,
                            pos,
                            stride,
                        }) if base == dst && once(i) => {
                            let off = int(pos).and_then(|p| p.checked_mul(i64::from(stride)));
                            match (off, after(nx)) {
                                (Some(o), Some((p2, Inst::Load { dst: d, addr, w })))
                                    if addr == i && w == Width::B8 && o >= 0 && o % 8 == 0 =>
                                {
                                    u32::try_from(o / 8).ok().map(|k| (p2, word(d, k)))
                                }
                                _ => None,
                            }
                        }
                        _ => None,
                    };
                    match fused {
                        Some((p, op)) => {
                            last = p;
                            op
                        }
                        None => Op::Layout {
                            dst: dst.0,
                            ty: ty.0,
                            desc: desc.0,
                        },
                    }
                }
                Inst::Br { to } => match (target(to), f.blocks.get(to)) {
                    (Some(t), _) => Op::Br { to: t },
                    (None, Some(blk)) => Op::BrBody {
                        to: blk.start,
                        block: to.0,
                    },
                    _ => Op::Step(pc as u32),
                },
                Inst::SigVal {
                    dst,
                    sig,
                    offset,
                    len,
                } => Op::SigVal {
                    dst: dst.0,
                    sig,
                    offset: offset.0,
                    len: len.0,
                },
                Inst::DopeLen { dst, dope, dim } => Op::DopeLen {
                    dst: dst.0,
                    dope: dope.0,
                    dim,
                },
                Inst::Call { func, args, dst } => Op::Call {
                    func: func.0,
                    start: args.start,
                    len: args.len,
                    dst: dst.map_or(NONE, |d| d.0),
                },
                Inst::CallNative { native, args, dst } => Op::CallNative {
                    native: native.0,
                    start: args.start,
                    len: args.len,
                    dst: dst.map_or(NONE, |d| d.0),
                },
                Inst::SigReadScalar { dst, sig } => Op::SigReadScalar {
                    dst: dst.0,
                    sig: sig.0,
                },
                Inst::WaitOn { sig } => Op::WaitOn { sig: sig.0 },
                Inst::Suspend { .. } => Op::Suspend,
                Inst::Ret { v } => Op::Ret {
                    v: v.map_or(NONE, |r| r.0),
                },
                Inst::Alloc { dst, size } => Op::Alloc {
                    dst: dst.0,
                    size: size.0,
                },
                Inst::CondBr { c, t, f: e } => match (target(t), target(e)) {
                    (Some(t), Some(e)) => Op::CondBr { c: c.0, t, f: e },
                    _ => Op::Step(pc as u32),
                },
                _ => Op::Step(pc as u32),
            };
            ops.push(op);
            pc = last + 1;
        }
        // Running off the end: the step path reports the pc beyond the code.
        start[n] = ops.len() as u32;
        ops.push(Op::Step(n as u32));
        at.push(n as u32);
        let entry: Vec<u32> = next
            .iter()
            .map(|&p| start.get(p as usize).copied().unwrap_or(NONE))
            .collect();
        // Jump targets as op indices. A target is the first executed instruction of a
        // block, never inside a fused op; should one be, a jump is left to the step path and
        // a fused compare is an internal error (`Step(NONE)` runs off the code).
        let to = |p: u32| entry.get(p as usize).copied().filter(|&k| k != NONE);
        for (k, op) in ops.iter_mut().enumerate() {
            let ir = at[k];
            *op = match *op {
                Op::Br { to: t } => to(t).map_or(Op::Step(ir), |t| Op::Br { to: t }),
                Op::BrBody { to: t, block } => {
                    to(t).map_or(Op::Step(ir), |t| Op::BrBody { to: t, block })
                }
                Op::CondBr { c, t, f: e } => match (to(t), to(e)) {
                    (Some(t), Some(e)) => Op::CondBr { c, t, f: e },
                    _ => Op::Step(ir),
                },
                Op::CmpBr {
                    op,
                    dst,
                    a,
                    b,
                    t,
                    f: e,
                } => match (to(t), to(e)) {
                    (Some(t), Some(e)) => Op::CmpBr {
                        op,
                        dst,
                        a,
                        b,
                        t,
                        f: e,
                    },
                    _ => Op::Step(NONE),
                },
                o => o,
            };
        }
        Code { ops, at, entry }
    }
}
