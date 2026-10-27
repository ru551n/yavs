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
//! result, and further common sequences (#443, `Op` below). Everything else is `Op::Step`: the IR instruction
//! through `Machine::step`. `Code::at` maps each op back to its IR pc, which frames keep
//! (`Frame::pc`), so traces, messages, coverage, root maps and the debug loop see the IR.

use yavs_ir::{
    AddrSpace, BoolOp, CheckKind, CheckMsgId, CmpOp, Func, FuncFlags, Inst, IntOp, Program, Role,
    Width,
};

use std::collections::HashMap;

use crate::exec::{CONST, FRAME, INST, PKG, R, SHARED, mk_addr};

pub(crate) struct Pre {
    /// Made from the function as lowered, under elaboration-time evaluation (`Runtime::forget_raw`).
    pub raw: bool,
    /// Initial registers: the preloaded values, zero elsewhere.
    init: Vec<u64>,
    /// The function's frame size in bytes (`Func::frame`), its parameters' slots and widths and
    /// whether it is nested: what a call needs, without touching the function itself.
    pub slots: u32,
    /// A call from the plain loop needs not write the parameters to their slots: the prologue it
    /// skips is the only reader (`lean_params`).
    pub lean: bool,
    /// The bytes of slots such a call allocates: none when the parameters are all the frame has.
    pub fslots: u32,
    pub params: Box<[(u32, Width)]>,
    /// For each parameter, the register the function's first ops load it into (`NONE`: none).
    /// A call writes these registers itself and goes on at `Code::skip`.
    pub pro: Box<[u32]>,
    pub nested: bool,
    /// The memo of pure calls is off for this function in every process (`pmemo::State::look`
    /// found too few repeats in several processes).
    pub memo_off: std::sync::atomic::AtomicBool,
    /// Processes that gave the memo of this function up.
    pub memo_votes: std::sync::atomic::AtomicU32,
    /// Processes that called this function through the memo.
    pub memo_users: std::sync::atomic::AtomicU32,
    /// Some flag of `NativeFlags::func` is set for this function (compiled, asked, called by the
    /// tier or counted; never cleared). The watching loop looks at this byte of the callee's
    /// `Pre`, which the call has loaded, instead of the flag table (`Runtime::sync_native`).
    pub native: std::sync::atomic::AtomicBool,
    /// Registers defined once by a constant or an address (not of an up-level frame): the
    /// frame starts with their values, the interpreter never executes their definitions.
    pub preloaded: Box<[bool]>,
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
    /// The collection reads only registers live at a safepoint (#422), so zeros are not
    /// needed for it; appending the table costs what appending stale words would.
    pub(crate) fn fill(&self, stack: &mut Vec<u64>, idx: u32) -> R<()> {
        if idx < 1 << 29 {
            self.fill_below(stack, idx);
            return Ok(());
        }
        // The error of `mk_addr`.
        if let Some(&(_, off)) = self.frame.first() {
            mk_addr(FRAME, idx, off)?;
        }
        self.fill_below(stack, idx);
        Ok(())
    }

    /// [`Pre::fill`] for a frame index below 2^29, where no address can fail.
    #[inline(always)]
    pub(crate) fn fill_below(&self, stack: &mut Vec<u64>, idx: u32) {
        let at = stack.len();
        stack.extend_from_slice(&self.init);
        let hi = u64::from(FRAME) << 61 | u64::from(idx) << 32;
        for &(r, off) in &self.frame {
            stack[at + r as usize] = hi | off;
        }
    }

    /// `budget`: `f` runs as lowered under elaboration-time evaluation, and its jumps back are
    /// counted ([`count_back_edges`]).
    pub(crate) fn new(f: &Func, prog: &Program, budget: bool) -> Pre {
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
        let mut preloaded = vec![false; n];
        for inst in &f.code {
            if matches!(inst, Inst::Const { .. } | Inst::ConstF { .. })
                || matches!(inst, Inst::AddrOf { space, .. } if !matches!(space, AddrSpace::Up(_)))
            {
                inst.visit(&mut |r, role, _| {
                    if role == Role::Def && defs.get(r.0 as usize) == Some(&1) {
                        preloaded[r.0 as usize] = true;
                    }
                });
            }
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
                        AddrSpace::Pkg(p) => {
                            let a = mk_addr(PKG, p, off).ok();
                            if a.is_some() {
                                known[dst.0 as usize] = Known::Pkg(p, off as u32);
                            }
                            a
                        }
                        AddrSpace::Const(c) => mk_addr(CONST, c.0, off).ok(),
                        AddrSpace::Shared => mk_addr(SHARED, 0, off).ok(),
                    };
                    if let Some(a) = a {
                        init[dst.0 as usize] = a;
                    }
                    a.is_some()
                }
                // A move of a register onto itself does nothing.
                Inst::Move { dst, src } if dst == src => true,
                // A jump forward to where execution goes on anyway (the blocks between are
                // dead or empty after the optimiser) is no operation.
                Inst::Br { to } if f.body != Some(to) => f.blocks.get(to).is_some_and(|b| {
                    b.start as usize > pc && next[b.start as usize] == next[pc + 1]
                }),
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
        let mut code = Code::new(f, prog, &skip, &known, &next, budget);
        let params: Box<[(u32, Width)]> = f.params.iter().map(|p| (p.slot, p.width)).collect();
        let pro = prologue(&mut code, &params);
        let lean = lean_params(f, prog, &code, &pro);
        // The registers holding the address of a parameter's slot are only read by the
        // prologue the call skips: they need no address.
        if lean {
            frame.retain(|&(_, off)| !f.params.iter().any(|p| p.slot as u64 == off));
        }
        // A lean frame needs slots only for what the code addresses besides the parameters
        // (which the skipped prologue alone read): none when it addresses no other.
        let fslots = if lean
            && (f.roots.is_empty() || !may_collect(f))
            && !f.code.iter().any(|i| {
                matches!(*i, Inst::AddrOf { space: AddrSpace::Frame, off, .. }
                    if !f.params.iter().any(|p| p.slot == off))
            }) {
            0
        } else {
            f.frame
        };
        Pre {
            raw: budget,
            init,
            slots: f.frame,
            lean,
            fslots,
            params,
            pro,
            nested: f.parent.is_some(),
            memo_off: Default::default(),
            memo_votes: Default::default(),
            memo_users: Default::default(),
            native: Default::default(),
            preloaded: preloaded.into(),
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
    /// The address of this offset in the data of this package.
    Pkg(u32, u32),
}

/// Bit of `PosIndexLoadCmpBr::dim`: the op heads a search loop (see there).
pub(crate) const SCAN: u8 = 0x80;

/// `Inst::AddrOf` of a fixed object: the address as a constant.
fn const_addr(dst: yavs_ir::Reg, space: u8, idx: u32, off: u32, pc: usize) -> Op {
    match mk_addr(space, idx, u64::from(off)) {
        Ok(v) => Op::Const {
            dst: dst.0,
            v: v as i64,
        },
        Err(_) => Op::Step(pc as u32),
    }
}

/// One predecoded operation (`Code`). Registers are `u32` indices into the frame's
/// registers, jump targets op indices. Widths are never `Width::Sig` (those go to `Step`).
#[derive(Clone, Copy, Debug)]
pub(crate) enum Op {
    /// The IR instruction at this pc, through `Machine::step`.
    Step(u32),
    /// `Inst::Case`: the arms of `Code::cases[tab]` as op indices.
    Case {
        v: u32,
        tab: u32,
    },
    Stmt,
    /// `Inst::Cover`: count item `item` of the instance (`Machine::op_cover`).
    Cover {
        item: u32,
    },
    /// `Inst::CoverIn`.
    CoverIn {
        dst: u32,
        acc: u32,
        src: u32,
        shift: u8,
        ulogic: bool,
    },
    /// `Inst::CoverCond`.
    CoverCond {
        cond: u32,
        state: u32,
    },
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
    /// `LoadSlot` of eight bytes.
    LoadSlot8 {
        dst: u32,
        off: u32,
    },
    /// `Stmt`, then `LoadSlot8`.
    StmtLoadSlot8 {
        dst: u32,
        off: u32,
    },
    /// Two `LoadSlot8`s.
    LoadSlot8x2 {
        d1: u32,
        o1: u32,
        d2: u32,
        o2: u32,
    },
    /// `LoadSlot8` and the compare-and-branch on it against `other` (the loaded register is
    /// the left operand when `side` is 0, else the right one); the compare's own result
    /// register is read by nothing else and not written.
    LoadSlot8CmpBr {
        ld: u32,
        off: u32,
        other: u32,
        t: u32,
        f: u32,
        m: u8,
        side: u8,
    },
    /// `StoreSlot`, then `Stmt`.
    StoreSlotStmt {
        src: u32,
        off: u32,
        w: Width,
    },
    /// `StoreSlot`, then the jump.
    StoreSlotBr {
        src: u32,
        off: u32,
        to: u32,
        w: Width,
    },
    /// `StoreSlot`, then `IndexImm`.
    StoreSlotIndexImm {
        src: u32,
        off: u32,
        dst: u32,
        base: u32,
        add: u32,
        w: Width,
    },
    /// `Move`, then the jump.
    MoveBr {
        dst: u32,
        src: u32,
        to: u32,
    },
    /// `Pos` and the `Index` by it (its message is read from the IR when it fails).
    PosIndex {
        dst: u32,
        dope: u32,
        idx: u32,
        base: u32,
        stride: u32,
        dim: u8,
    },
    PosIndexLoad {
        dst: u32,
        dope: u32,
        idx: u32,
        base: u32,
        stride: u32,
        w: Width,
        dim: u8,
    },
    PosIndexStore {
        src: u32,
        dope: u32,
        idx: u32,
        base: u32,
        stride: u32,
        w: Width,
        dim: u8,
    },
    /// `Index` and the load or store at it.
    IndexLoad {
        dst: u32,
        base: u32,
        pos: u32,
        stride: u32,
        w: Width,
    },
    IndexStore {
        src: u32,
        base: u32,
        pos: u32,
        stride: u32,
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
    /// A load from the data of package `pkg` at `off` (a package variable, for one the handle of a
    /// protected object).
    LoadPkg {
        dst: u32,
        pkg: u32,
        off: u32,
        w: Width,
    },
    /// Two adjacent words at `base + off` and `base + off + 8` (the data pointer and the descriptor of
    /// a fat pair; a `Load` or `LoadOff` and the `LoadOff` after it), both read before either is written.
    LoadPair {
        d1: u32,
        d2: u32,
        base: u32,
        off: u32,
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
    /// `Inst::SlicePos` at this pc.
    SlicePos(u32),
    Bin {
        op: IntOp,
        dst: u32,
        a: u32,
        b: u32,
    },
    /// A run of `Bin` (add, subtract, multiply by a constant), `Pos` and `DopeLen` as linear
    /// forms (`Lin`): `c0 + ca * a`, `Aff2` of two registers, `AffP` of the position of `idx`
    /// in the first dimension of `dope`, `AffL` of its length, `AffPL` of both, `AffPR` of the
    /// position and a register. Atoms within 2^30 give the instructions' result by
    /// construction; anything else (an index out of range, a long range, big values) executes
    /// the run as the IR has it (`Code::aff`).
    Aff1 {
        dst: u32,
        a: u32,
        ca: i16,
        c0: i32,
    },
    Aff2 {
        dst: u32,
        a: u32,
        b: u32,
        ca: i16,
        cb: i16,
        c0: i32,
    },
    AffP {
        dst: u32,
        dope: u32,
        idx: u32,
        cp: i16,
        c0: i32,
    },
    AffL {
        dst: u32,
        dope: u32,
        cl: i16,
        c0: i32,
    },
    AffPL {
        dst: u32,
        dope: u32,
        idx: u32,
        cp: i16,
        cl: i16,
        c0: i32,
    },
    AffPR {
        dst: u32,
        dope: u32,
        idx: u32,
        a: u32,
        cp: i16,
        ca: i16,
        c0: i16,
    },
    /// `Alloc`, the `Store` of a time to its start and the `Index` 8 bytes into it: the start
    /// of a waveform (`time` the register holding the delay).
    WaveBuf {
        w: u32,
        size: u32,
        time: u32,
        e: u32,
    },
    /// A `WaveBuf` that is not run (its `Alloc`, `Store` and `Index` are dropped), the `Copy` of the
    /// value into it and the one-element `Drive` of it, which nothing else reads, without delay
    /// limit: the value goes from `src` to the kernel as it is. `w`, `size`, `time` and `n` are
    /// the registers of the waveform, for the cases the fast path does not take.
    CopyDrive {
        driver: u16,
        target: u16,
        src: u16,
        bytes: u16,
        time: u16,
        size: u16,
        w: u16,
        n: u16,
        inertial: bool,
    },
    /// A `WaveBuf` that is not run, the `LSet` of element 0 of that waveform and the one-element
    /// `Drive` of it, which nothing else reads, without delay limit: a scalar logic assignment
    /// straight to the kernel. `size`, `time`, `w` and `e` are the registers of the waveform, for
    /// the cases the fast path does not take.
    LSetDrive {
        v: u32,
        driver: u16,
        target: u16,
        time: u16,
        size: u16,
        w: u16,
        e: u16,
        inertial: bool,
    },
    /// A `WaveBuf` that is not run, the `LCopy` of `cnt` elements from element `soff` of the
    /// logic array at `src` into element 0 of that waveform and the one-element `Drive` of it,
    /// which nothing else reads, without delay limit (`a <= b(7 downto 3)`): at zero delay and as
    /// long as the target, within one block, the planes go to the kernel as they are. `size`,
    /// `time`, `w` and `e` are the registers of the waveform for the cases the fast path does
    /// not take.
    LCopyDrive {
        driver: u16,
        target: u16,
        src: u16,
        soff: u16,
        cnt: u16,
        time: u16,
        size: u16,
        w: u16,
        e: u16,
        n: u16,
        inertial: bool,
    },
    /// `AffPL` / `AffPR` (the second with the register's coefficient 1) and the `SigRead1` of
    /// the element it computes (nothing else reads the offset).
    ReadPL {
        dst: u32,
        dope: u32,
        idx: u32,
        sig: u16,
        cp: i16,
        cl: i16,
        c0: i16,
    },
    ReadPR {
        dst: u32,
        dope: u32,
        idx: u32,
        a: u32,
        sig: u16,
        cp: i16,
        c0: i16,
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
    /// `BinRange` whose bounds are constants within 32 bits.
    BinRangeK {
        op: IntOp,
        dst: u32,
        a: u32,
        b: u32,
        lo: i32,
        hi: i32,
    },
    Cmp {
        m: u8,
        dst: u32,
        a: u32,
        b: u32,
    },
    Bool {
        op: BoolOp,
        dst: u32,
        a: u32,
        b: u32,
    },
    /// `Inst::Select`.
    Select {
        dst: u32,
        c: u32,
        a: u32,
        b: u32,
    },
    /// `Inst::Cat` (read from the IR at the op's pc).
    Cat,
    /// `Inst::LEq` (equal logic vectors of `n` elements), out of line.
    LEq {
        dst: u32,
        a: u32,
        b: u32,
        n: u32,
    },
    /// `Inst::LCopy`.
    LCopy {
        dst: u32,
        doff: u32,
        src: u32,
        soff: u32,
        n: u32,
    },
    /// `Inst::SigRead` of one signal reference (a reference of several parts steps).
    SigReadVec {
        dst: u32,
        sig: u32,
    },
    /// `Cmp` and the `CondBr` on its result.
    CmpBr {
        m: u8,
        dst: u32,
        a: u32,
        b: u32,
        t: u32,
        f: u32,
    },
    /// A loop latch: the compare-and-branch (taken: `t`; not taken: the next op), then
    /// `dst = src + delta` (the `Bin` after it, checked) and the jump to `head`. The three ops
    /// stay in place after it, for any other way in.
    CmpBrStep {
        m: u8,
        delta: i16,
        a: u16,
        b: u16,
        dst: u16,
        src: u16,
        t: u32,
        head: u32,
    },
    /// `PosIndexLoad` (of one byte or word at `dst`) and the `CmpBr` of `dst` (the left
    /// operand; `m` is swapped when it was the right one) against `b`, which nothing else
    /// reads the result of. Bit 7 of `dim` ([`SCAN`]) marks the head of a search loop: this op
    /// goes on (`f`) with a `CmpBrStep` of the same index that goes back to it.
    PosIndexLoadCmpBr {
        m: u8,
        w: Width,
        dim: u8,
        dst: u16,
        dope: u16,
        idx: u16,
        base: u16,
        b: u16,
        stride: u16,
        t: u32,
        f: u32,
    },
    /// `Cmp` (its result read by nothing else) and the `Select` on it.
    CmpSelect {
        m: u8,
        dst: u32,
        a: u32,
        b: u32,
        x: u32,
        y: u32,
    },
    /// `Deref`, and the `LoadOff` through its result (which is also written).
    DerefLoadOff {
        w: Width,
        d1: u32,
        handle: u32,
        what: CheckMsgId,
        d2: u32,
        off: u32,
    },
    /// `Deref`, and the `IndexImm` of its result (which is also written): the address `off` bytes
    /// into the object (the elements of an array behind an access value).
    DerefImm {
        d1: u32,
        handle: u32,
        d2: u32,
        off: u32,
    },
    /// `LoadOff` of an access value (also written), its `Deref` and the `IndexImm` of that.
    LoadDerefImm {
        base: u32,
        h: u32,
        d1: u32,
        d2: u32,
        o1: u16,
        o2: u16,
    },
    /// `MoveBr` to a `CondBr` on the moved register: the move, then the branch on it.
    MoveCondBr {
        dst: u32,
        src: u32,
        t: u32,
        f: u32,
    },
    /// `EdgeSig` and the `CondBr` on its result (which is also written). With `stmt`, the
    /// `Stmt` before it too (the op stands in its place and goes on as the next one).
    EdgeBr {
        dst: u32,
        kind: yavs_ir::EdgeKind,
        stmt: bool,
        sig: u32,
        offset: u32,
        t: u32,
        f: u32,
    },
    /// `WaitSig` and the `Suspend` after it, as op `at`: the op itself, or the op a `Br` to it
    /// was replaced by a copy of (the copy runs as op `at`). With `stmt`, op `at` is the `Stmt`
    /// before them (and the op after it the `WaitSig`).
    WaitSuspend {
        sig: u32,
        offset: u32,
        at: u32,
        stmt: bool,
    },
    /// `SigVal`, `WaitEdge`, `WaitSig` and `Suspend` of a `wait until` on a clock edge of one
    /// element: the edge of the element the wait names, then the wait, then the suspension.
    /// Placed like `WaitSuspend` (`at`, `stmt`); `body` is the block id a `BrBody` copy of it
    /// stands for (`NONE` otherwise), whose pause check runs first. The `Suspend` is 3 ops on.
    WaitEdgeSuspend {
        sig: u32,
        offset: u32,
        kind: u8,
        at: u32,
        stmt: bool,
        body: u32,
    },
    /// The `Stmt` of a wait statement whose sensitivity is the same at every activation
    /// (`static_waits`): when the kernel can keep the last wait (`Ctx::wait_same`), the
    /// op goes on at the `Suspend` (op `to`) without naming the signals again.
    WaitStatic {
        to: u32,
    },
    /// The `Stmt` of the first of a run of scalar drives from frame slots (`drive_runs`): it ends
    /// the statement, and when each drive of run `run` would change nothing it goes on after the
    /// run (`Machine::drive_run`), else at the first drive that would.
    DriveRun {
        run: u32,
        /// It stands for the first drive's `Stmt` (else, a statement without one, for its
        /// `SigVal`: `yavs_ir` `opt/slots.rs` drops markers).
        stmt: bool,
    },
    /// `StmtLoadSlot8` and the `CondBr` on the register it loaded (which is also written).
    FlagBr {
        dst: u32,
        off: u32,
        t: u32,
        f: u32,
    },
    /// `LoadSlot8` (its register is also written) and the `SigVal` that indexes with it.
    SigValSlot {
        d: u32,
        off: u32,
        dst: u32,
        sig: u32,
        len: u32,
    },
    /// `Alloc`, a `Cat` into it and the `LCopy` of it into `dst` (offsets read from that
    /// instruction): the concatenation of logic elements is written there directly when
    /// all of it is copied and it fits one block of planes at element 0, or is wider and no
    /// piece shares a byte with `dst` (the temporary is dead). `size` is the byte count, or a register that
    /// holds it with `SIZE_REG` set.
    CatLCopy {
        tmp: u32,
        size: u32,
        dst: u32,
        n: u32,
    },
    /// `Inst::TempRoot` of a fat pair.
    FatRoot {
        addr: u32,
    },
    /// `Inst::NewFrame` at this IR pc.
    NewFrame(u32),
    MemEq {
        dst: u32,
        a: u32,
        b: u32,
        n: u32,
    },
    /// `AllocK` of 32 bytes and the `LCopy` into it of at most 64 elements from one block (else
    /// the `AllocK` alone).
    AllocLCopy {
        dst: u32,
        doff: u32,
        src: u32,
        soff: u32,
        n: u32,
    },
    /// `AllocK` of 32 bytes and the `SigReadVec` into it, in one step when the signal is a single
    /// range of at most 64 logic elements (else the `AllocK` alone).
    AllocReadVec {
        dst: u32,
        sig: u32,
    },
    /// `AllocK` of 32 bytes and the logic `Cat` into it, in one step when the pieces fit one
    /// block (else the `AllocK` alone).
    AllocCat {
        dst: u32,
        n: u32,
        m: u32,
        /// The start and length of the operand list of the `Cat` (pieces of two registers).
        parts: u32,
        len: u8,
        scalars: u16,
        bias: u8,
    },
    /// `Alloc` of a size known at decode time.
    AllocK {
        dst: u32,
        n: u32,
    },
    /// `AllocK` of 16 bytes and the two words `a` and `b` stored in it (a fat pair).
    NewPair {
        dst: u32,
        a: u32,
        b: u32,
    },
    /// `AllocK` of `len` words and the `MakeDope` of the registers at `start` into it.
    NewDope {
        at: u32,
        start: u32,
        len: u32,
    },
    Check {
        kind: CheckKind,
        v: u32,
        lo: u32,
        hi: u32,
        what: CheckMsgId,
    },
    /// A `Check` of kind `Range` or `Index` whose bounds are constants of 32 bits.
    CheckK {
        kind: CheckKind,
        v: u32,
        lo: i32,
        hi: i32,
        what: CheckMsgId,
    },
    Br {
        to: u32,
    },
    /// The entry `Br` to the head `to` of an element loop (`Code::loops[id]`, `elem_loops`): the
    /// iterations but the last may run at once (`Machine::op_loop`), then the jump.
    Loop {
        id: u32,
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
    /// `Inst::Const` or `ConstF` (`v` the bits), defined more than once in the function.
    Const {
        dst: u32,
        v: i64,
    },
    /// `Inst::FCmp`.
    FCmp {
        op: yavs_ir::CmpOp,
        dst: u32,
        a: u32,
        b: u32,
    },
    /// `Inst::AddrOf` of the frame, defined more than once in the function.
    AddrFrame {
        dst: u32,
        off: u32,
    },
    /// `Inst::AddrOf` of a frame `depth` static links up, defined more than once.
    AddrUp {
        dst: u32,
        depth: u8,
        off: u32,
    },
    /// `AddrUp` and the `Load` through its result, which nothing else reads: a variable of an
    /// enclosing subprogram (interpreter only).
    LoadUp {
        dst: u32,
        depth: u8,
        w: Width,
        off: u32,
    },
    /// `AddrUp` and the `Store` through its result, which nothing else reads (interpreter only).
    StoreUp {
        src: u32,
        depth: u8,
        w: Width,
        off: u32,
    },
    Deref {
        dst: u32,
        handle: u32,
        what: CheckMsgId,
    },
    SubtypeCheck {
        dope: u32,
        want: u32,
        what: CheckMsgId,
        dims: u8,
    },
    Copy {
        dst: u32,
        src: u32,
        bytes: u32,
    },
    CopyConst {
        dst: u32,
        c: u32,
    },
    MakeDope {
        at: u32,
        start: u32,
        len: u32,
    },
    Field {
        dst: u32,
        base: u32,
        layout: u32,
        n: u32,
    },
    LGet {
        dst: u32,
        base: u32,
        idx: u32,
    },
    LSet {
        base: u32,
        idx: u32,
        v: u32,
    },
    /// `Inst::Call`; `dst` is `NONE` for a procedure.
    Call {
        func: u32,
        start: u32,
        len: u32,
        dst: u32,
    },
    /// `Inst::Call` of a function the memo of pure calls may answer (`pmemo.rs`).
    CallM {
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
    /// `Inst::CallNative` of `'image` of an integer or enumeration value in register `a` (type
    /// `ty`): the string made at once, without the argument cells and checks of other natives
    /// (it reads nothing but its argument and writes only temporary memory).
    Image {
        dst: u32,
        a: u32,
        ty: u32,
    },
    /// `Image` (result `x`) and the three ops after it that read its string: the data address
    /// into `p`, the descriptor into `q`, the length into `l` (`Load`, `LoadOff` by 8,
    /// `DopeLen`); steps over them.
    ImageStr {
        a: u32,
        ty: u32,
        x: u16,
        p: u16,
        q: u16,
        l: u16,
    },
    /// `Inst::CallNative` of a native `ieee` subprogram with at most [`IEEE_CELLS`] cells, none a signal:
    /// the call without the bookkeeping other natives need (they may reach the heap, files and
    /// the like; these work on temporary memory only).
    CallIeee {
        native: u16,
        /// Register of the result, `NONE16` for none.
        dst: u16,
        /// Argument cells in `regs`.
        len: u8,
        /// The native takes a type (`numeric_bit`): `NativeRef::ty` is `Some`.
        bit: bool,
        /// The argument registers: no operand list is read at run time.
        regs: [u16; IEEE_CELLS],
    },
    /// `Inst::CallProt`: a call with the protected object `obj` as the first argument.
    CallProt {
        func: u32,
        obj: u32,
        start: u32,
        len: u32,
        dst: u32,
    },
    SigReadScalar {
        dst: u32,
        sig: u32,
    },
    /// `SigVal` of one element of instance signal `sig` at `offset`, and the `SigReadScalar`
    /// of it (nothing else reads the reference).
    SigRead1 {
        dst: u32,
        sig: u32,
        offset: u32,
    },
    /// `LoadSlot8` of `off` and the `SigRead1` of the element it indexes.
    SigRead1Slot {
        dst: u32,
        sig: u32,
        off: u32,
    },
    /// `SigVal` of one element and the clock-edge test of it (`Inst::Edge`, or the
    /// `rising_edge` / `falling_edge` native).
    EdgeSig {
        dst: u32,
        sig: u32,
        offset: u32,
        kind: yavs_ir::EdgeKind,
    },
    /// `SigVal` of one element and the `WaitOn` of it.
    WaitSig {
        sig: u32,
        offset: u32,
    },
    /// A scalar `std_ulogic` gate (`ieee::GATES` index `g`) as the native computes it.
    Gate {
        g: u8,
        dst: u32,
        a: u32,
        b: u32,
    },
    /// `math_real.uniform` of three variables in memory, in place; a seed out of range or an
    /// address that does not load runs the native call itself (interpreter only).
    Uniform {
        s1: u32,
        s2: u32,
        x: u32,
        native: u32,
        start: u32,
    },
    /// Scalar `not` of a `std_ulogic`.
    NotL {
        dst: u32,
        a: u32,
    },
    /// `Inst::Drive` onto a signal reference in register `target`.
    Drive {
        driver: u32,
        target: u32,
        wave: u32,
        n: u32,
        /// The register of the rejection limit, `NONE` without one.
        reject: u32,
        inertial: bool,
    },
    WaitOn {
        sig: u32,
    },
    /// `Inst::WaitEdge`; `kind` is `EdgeKind::code`.
    WaitEdge {
        sig: u32,
        kind: u8,
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
    // (Ops of the temporaries in the frame go last: new variants in the middle renumber the
    // others and move the dispatch loop's code, #508.)
    /// `Inst::Zero` of `n` bytes at `off` of the frame's slots, the address being in `w` (a
    /// temporary of a process in its frame, `yavs_ir` `opt/slots.rs`).
    ZeroSlot {
        w: u32,
        off: u32,
        n: u32,
    },
    /// `AllocReadVec`, `AllocCat`, `AllocLCopy` and `CatLCopy` of a temporary in the frame (its
    /// `ZeroSlot` in place of the `AllocK`): the register holds the block's address and keeps it.
    ZeroReadVec {
        dst: u32,
        sig: u32,
        /// The block's offset in the frame's slots.
        off: u32,
    },
    ZeroCat {
        dst: u32,
    },
    ZeroLCopy {
        dst: u32,
        doff: u32,
        src: u32,
        soff: u32,
        n: u32,
    },
    ZeroCatLCopy {
        tmp: u32,
        size: u32,
        dst: u32,
        n: u32,
    },
    /// `FlagBr` without the `Stmt` (a statement whose marker the optimiser dropped).
    SlotBr {
        dst: u32,
        off: u32,
        t: u32,
        f: u32,
    },
    /// `Alloc` of 32 bytes, `LCopy` of at most 64 elements from one block into it and the `is_x`
    /// of that vector (`CallIeee`), the temporary read by nothing else: the planes are tested where
    /// they are (else the `Alloc` alone). #770.
    IsXSlice {
        tmp: u32,
        src: u32,
        soff: u32,
        n: u32,
        res: u32,
    },
    /// `BinRangeK` of `k * i`, `BinRangeK` of `+ c` (both of 32 bits) and the `SlicePos` of the slice
    /// between them in an ascending descriptor (`v (4 * i to 4 * i + 3)`): the three registers
    /// are written together when both ends are inside the bounds, else the first op alone. #771.
    SliceAff {
        i: u32,
        dope: u32,
        a: u32,
        b: u32,
        p: u32,
        k: i8,
        c: i8,
    },
    /// `PosIndexLoad` of an element and the `Index` that takes it as the position in another block
    /// (the character and its row in a result table, #722); registers of 16 bits.
    PosIndexLoadIndex {
        dst: u16,
        dope: u16,
        idx: u16,
        base: u16,
        dst2: u16,
        base2: u16,
        stride: u16,
        stride2: u16,
        w: Width,
        dim: u8,
    },
    /// [`Op::StmtIsXSlice`] and the `CondBr` on its result: the branch goes with it when the
    /// fast path ran (registers of 16 bits).
    StmtIsXBr {
        tmp: u16,
        src: u16,
        soff: u16,
        n: u16,
        res: u16,
        t: u32,
        f: u32,
    },
    /// `Aff2` (`c0 + ca * a + cb * b`) and the `LCopy` that takes its result as the offset it
    /// copies to (the plane position of a slice, `v (a to b) := x`): the copy follows the sum
    /// when it is in range, else the sum alone is replayed. #771.
    AffLCopy {
        ad: u16,
        a: u16,
        b: u16,
        ld: u16,
        src: u16,
        soff: u16,
        n: u16,
        ca: i8,
        cb: i8,
        c0: i16,
    },
    /// `Stmt` and then [`Op::IsXSlice`]: the statement of an `is_x` of a slice makes no temporary
    /// on the fast path, so the marker goes with it.
    StmtIsXSlice {
        tmp: u32,
        src: u32,
        soff: u32,
        n: u32,
        res: u32,
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
    /// `entry[0]`, so a call finds the first op without reading `entry`.
    pub first: u32,
    /// For each `Aff` op (sorted by op index): the IR pcs of the `Bin` run it stands for, from
    /// its first instruction to the one defining the op's result.
    pub aff: Vec<(u32, u32, u32)>,
    /// Ranges of ops (first, last; sorted) where compiled code may not take over the frame:
    /// between a waveform buffer that is not run and the op that drives it, where the buffer's
    /// registers are undefined.
    pub no_tier: Vec<(u32, u32)>,
    /// The `Case` tables of the `Op::Case`s, with op indices as targets.
    pub cases: Vec<CaseOps>,
    /// The element loops that `Op::Loop` enters.
    pub loops: Vec<ElemLoop>,
    /// `YAVS_DUMPIR` is set: `Machine::op_loop` says what each entry did.
    pub trace: bool,
    /// The op a call goes on at when it has written `Pre::pro` itself: after the ops that
    /// only load parameters (`NONE`: there are none).
    pub skip: u32,
    /// The runs of drives of the `Op::DriveRun`s.
    pub runs: Vec<DriveRun>,
}

/// A run of scalar drives, each `[Stmt?, SigVal, LoadSlot, Aff1?, LSetDrive]`, in consecutive ops.
pub(crate) struct DriveRun {
    /// The op of each drive's `Stmt` (its `SigVal` when it has none).
    pub starts: Vec<u32>,
    /// The op after the run.
    pub end: u32,
    pub drives: Vec<RunDrive>,
}

/// The operands of one drive of a `DriveRun`.
pub(crate) struct RunDrive {
    pub sv: u32,
    pub sig: u32,
    pub offset: u32,
    pub len: u32,
    pub ld: u32,
    pub off: u32,
    pub w: Width,
    /// The `Aff1` between the load and the drive: its register, factor and constant.
    pub aff: Option<(u32, i16, i32)>,
    pub driver: u16,
    pub time: u16,
}

/// The drive at the start of `ops` (and how many ops it is), if it is of the shape `DriveRun` takes.
fn run_drive(ops: &[Op]) -> Option<(RunDrive, usize)> {
    let stmt = matches!(ops.first(), Some(Op::Stmt));
    let [
        Op::SigVal {
            dst: sv,
            sig,
            offset,
            len,
        },
        Op::LoadSlot { dst: ld, off, w },
        rest @ ..,
    ] = &ops[usize::from(stmt)..]
    else {
        return None;
    };
    let (aff, v, last) = match rest {
        [Op::Aff1 { dst, a, ca, c0 }, last, ..] if a == ld => (Some((*dst, *ca, *c0)), *dst, last),
        [last, ..] => (None, *ld, last),
        [] => return None,
    };
    let Op::LSetDrive {
        v: lv,
        driver,
        target,
        time,
        ..
    } = *last
    else {
        return None;
    };
    (lv == v && u32::from(target) == *sv).then_some((
        RunDrive {
            sv: *sv,
            sig: *sig,
            offset: *offset,
            len: *len,
            ld: *ld,
            off: *off,
            w: *w,
            aff,
            driver,
            time,
        },
        3 + usize::from(stmt) + usize::from(aff.is_some()),
    ))
}

/// Runs of at least two drives like [`run_drive`] in a row: the first `Stmt` becomes an
/// `Op::DriveRun`. A process that assigns its outputs one by one at the end of every
/// activation mostly assigns what they hold, and the run is then one op.
fn drive_runs(ops: &mut [Op]) -> Vec<DriveRun> {
    let mut runs = Vec::new();
    let mut k = 0;
    while k < ops.len() {
        let (mut starts, mut drives) = (Vec::new(), Vec::new());
        let mut j = k;
        while let Some((d, n)) = run_drive(&ops[j..]) {
            starts.push(j as u32);
            drives.push(d);
            j += n;
        }
        if drives.len() >= 2 {
            ops[k] = Op::DriveRun {
                run: runs.len() as u32,
                stmt: matches!(ops[k], Op::Stmt),
            };
            runs.push(DriveRun {
                starts,
                end: j as u32,
                drives,
            });
            k = j;
        } else {
            k += 1;
        }
    }
    runs
}

/// Whether a collection can run while a frame of `f` is on the stack: it does at `New`, `Free`,
/// a native call and a suspension, and in anything a call reaches. A frame of a function that
/// cannot be in one of those needs no slot the collector reads as a root.
fn may_collect(f: &Func) -> bool {
    f.code.iter().any(|i| {
        matches!(
            i,
            Inst::Call { .. }
                | Inst::CallProt { .. }
                | Inst::CallNative { .. }
                | Inst::New { .. }
                | Inst::NewFill { .. }
                | Inst::Free { .. }
                | Inst::WaitOn { .. }
                | Inst::WaitEdge { .. }
                | Inst::WaitFor { .. }
                | Inst::WaitAt { .. }
                | Inst::Suspend { .. }
        )
    })
}

/// Is every parameter handed over in the register the prologue loads it into, and read from
/// its slot by nothing else? Then a call writes the registers and leaves the slots alone.
/// Plain functions and procedures (nested ones too, they get a static link) whose parameters are
/// whole 8-byte values or addresses (`out` and `inout` scalars are by reference).
fn lean_params(f: &Func, prog: &Program, code: &Code, pro: &[u32]) -> bool {
    if f.nested
        || f.body.is_some()
        || !matches!(
            f.kind,
            yavs_ir::FuncKind::Function | yavs_ir::FuncKind::Procedure
        )
        || code.skip == NONE
        || pro.len() != f.params.len()
        || pro.contains(&NONE)
        || f.params.iter().any(|p| {
            p.width != Width::B8
                || p.class != yavs_ir::ParamClass::Value
                // The address of an `out` or `inout` parameter is a collector root in its
                // slot, which a lean call leaves empty: only where no collection can run.
                || p.mode != yavs_ir::Mode::In && may_collect(f)
        })
    {
        return false;
    }
    let in_param = |off: u32| f.params.iter().any(|p| p.slot == off);
    // The registers holding a parameter slot's address, each defined once.
    let mut addr: Vec<u32> = Vec::new();
    for i in &f.code {
        if let Inst::AddrOf {
            dst,
            space: AddrSpace::Frame,
            off,
        } = *i
            && in_param(off)
        {
            addr.push(dst.0);
        }
    }
    let Some(&end) = code.at.get(code.skip as usize) else {
        return false;
    };
    // The prologue is entered by the call only: no jump goes back to it.
    let targets = f.code.iter().flat_map(|i| match *i {
        Inst::Case { table, .. } => prog.cases.get(table).map_or_else(Vec::new, |t| {
            std::iter::once(t.default)
                .chain(t.arms.iter().map(|a| a.2))
                .collect()
        }),
        ref i => i.targets(),
    });
    for t in targets {
        if f.blocks.get(t).is_none_or(|b| b.start < end) {
            return false;
        }
    }
    // Nothing at or after the first op the call runs reads one of them.
    for i in f.code.iter().skip(end as usize) {
        let mut used = false;
        i.visit(&mut |r, role, _| used |= role == Role::Use && addr.contains(&r.0));
        if let Some(l) = i.list() {
            used |= f
                .regs(l)
                .is_none_or(|rs| rs.iter().any(|r| addr.contains(&r.0)));
        }
        if used {
            return false;
        }
    }
    true
}

/// The leading ops that load a parameter from its slot into a register, each a whole 8-byte
/// parameter, and the statement markers after them: for each parameter the register (`NONE`),
/// and `code.skip` set past the ops.
fn prologue(code: &mut Code, params: &[(u32, Width)]) -> Box<[u32]> {
    let mut pro = vec![NONE; params.len()];
    let param = |off: u32| params.iter().position(|&p| p == (off, Width::B8));
    let mut k = code.first as usize;
    let mut any = false;
    while let Some(&op) = code.ops.get(k) {
        let loads: &[(u32, u32)] = match op {
            Op::LoadSlot8 { dst, off }
            | Op::LoadSlot {
                dst,
                off,
                w: Width::B8,
            } => &[(dst, off)],
            Op::LoadSlot8x2 { d1, o1, d2, o2 } => &[(d1, o1), (d2, o2)],
            _ => break,
        };
        if !loads.iter().all(|&(_, off)| param(off).is_some()) {
            break;
        }
        for &(dst, off) in loads {
            if let Some(p) = param(off) {
                pro[p] = dst;
            }
        }
        any = true;
        k += 1;
    }
    // A statement marker at the start frees nothing: the frame has no temporaries yet.
    while matches!(code.ops.get(k), Some(Op::Stmt)) {
        any = true;
        k += 1;
    }
    code.skip = if any { k as u32 } else { NONE };
    if pro.iter().any(|&r| r != NONE) {
        pro.into()
    } else {
        Box::default()
    }
}

/// A `Case` table with its targets as op indices.
pub(crate) struct CaseOps {
    /// Sorted, non-overlapping inclusive ranges and their targets.
    pub arms: Vec<(i64, i64, u32)>,
    pub default: u32,
}

pub(crate) const NONE: u32 = u32::MAX;

/// Set in the `size` of `Op::CatLCopy` when the rest names the register that holds the byte count.
pub(crate) const SIZE_REG: u32 = 1 << 31;

/// A native the loop runs as an op of its own.
enum NativeOp {
    Gate {
        g: u8,
        a: yavs_ir::Reg,
        b: yavs_ir::Reg,
    },
    Not {
        a: yavs_ir::Reg,
    },
    /// The clock-edge test `kind` of the signal reference in register `sig`: `Inst::Edge`, or
    /// the `rising_edge` / `falling_edge` native.
    Edge {
        kind: yavs_ir::EdgeKind,
        sig: yavs_ir::Reg,
    },
    /// `math_real.uniform(seed1, seed2, x)`: the addresses of the three variables.
    Uniform {
        s1: yavs_ir::Reg,
        s2: yavs_ir::Reg,
        x: yavs_ir::Reg,
    },
}

/// The argument and type of a call of `'image` of an integer or enumeration value (`Op::Image`).
fn scalar_image(
    prog: &Program,
    f: &Func,
    native: yavs_ir::NativeId,
    args: yavs_ir::Regs,
) -> Option<(yavs_ir::Reg, yavs_ir::TyId)> {
    let n = prog.natives.get(native)?;
    let ty = n.ty?;
    let &[a] = f.regs(args)? else {
        return None;
    };
    (n.symbol == "yavs.attr.image"
        && f.reg_ty.get(a.0 as usize) == Some(&yavs_ir::RegTy::I64)
        && matches!(
            prog.types.get(ty),
            Some(yavs_ir::Ty::Int { .. } | yavs_ir::Ty::Enum { .. })
        ))
    .then_some((a, ty))
}

/// The most argument cells of an `Op::CallIeee`: two arrays of `yavs_ir::ieee::ARRAY_REGS`.
pub(crate) const IEEE_CELLS: usize = 8;

/// No result register in an [`Op::CallIeee`].
pub(crate) const NONE16: u16 = u16::MAX;

/// Whether the native is an `ieee` one that `Op::CallIeee` runs: at most [`IEEE_CELLS`] argument
/// cells, none a signal, not the procedure `uniform` (which writes through its arguments). Such a
/// native allocates only temporary memory and never collects ([`crate::TierHost::native_light`]).
pub fn light_ieee(
    prog: &Program,
    f: &Func,
    native: yavs_ir::NativeId,
    args: yavs_ir::Regs,
) -> bool {
    let Some(sym) = prog.natives.get(native).map(|n| &n.symbol) else {
        return false;
    };
    let Some(key) = sym.strip_prefix("yavs.ieee.") else {
        return false;
    };
    let Some(regs) = f.regs(args) else {
        return false;
    };
    regs.len() <= IEEE_CELLS
        && regs
            .iter()
            .all(|r| f.reg_ty.get(r.0 as usize) != Some(&yavs_ir::RegTy::Sig))
        && !matches!(
            yavs_ir::ieee::parse(key.split_once('@').map_or(key, |k| k.0)),
            None | Some(yavs_ir::ieee::Op::Uniform)
        )
}

/// The [`Op::CallIeee`] of a light `ieee` native whose numbers all fit its fields, else `None`
/// (the call stays an `Op::CallNative`).
fn call_ieee(
    prog: &Program,
    f: &Func,
    native: yavs_ir::NativeId,
    args: yavs_ir::Regs,
    dst: Option<yavs_ir::Reg>,
) -> Option<Op> {
    if !light_ieee(prog, f, native, args) {
        return None;
    }
    // The unused cells repeat the first register, so a read of all of them stays in the frame.
    let list = f.regs(args)?;
    let first = u16::try_from(list.first()?.0)
        .ok()
        .filter(|&r| r != NONE16)?;
    let mut regs = [first; IEEE_CELLS];
    for (c, r) in regs.iter_mut().zip(list) {
        *c = u16::try_from(r.0).ok().filter(|&r| r != NONE16)?;
    }
    Some(Op::CallIeee {
        native: u16::try_from(native.0).ok()?,
        dst: match dst {
            Some(d) => u16::try_from(d.0).ok().filter(|&r| r != NONE16)?,
            None => NONE16,
        },
        len: u8::try_from(args.len).ok()?,
        bit: prog.natives.get(native)?.ty.is_some(),
        regs,
    })
}

/// The scalar `std_logic_1164` natives, by their symbol (`ieee::lookup` has the bodies).
fn native_op(
    prog: &Program,
    f: &Func,
    native: yavs_ir::NativeId,
    args: yavs_ir::Regs,
) -> Option<NativeOp> {
    use yavs_ir::ieee::Op as Ie;
    let sym = &prog.natives.get(native)?.symbol;
    let key = sym.strip_prefix("yavs.ieee.")?;
    let regs = f.regs(args)?;
    let scalar = |r: &yavs_ir::Reg| f.reg_ty.get(r.0 as usize) != Some(&yavs_ir::RegTy::Sig);
    match (yavs_ir::ieee::parse(key)?, regs) {
        (Ie::Gate(g), [a, b]) if scalar(a) && scalar(b) => Some(NativeOp::Gate {
            g: crate::ieee::gate_index(g)?,
            a: *a,
            b: *b,
        }),
        (Ie::Not, [a]) if scalar(a) => Some(NativeOp::Not { a: *a }),
        (Ie::Uniform, [s1, s2, x]) if scalar(s1) && scalar(s2) && scalar(x) => {
            Some(NativeOp::Uniform {
                s1: *s1,
                s2: *s2,
                x: *x,
            })
        }
        (Ie::Edge(rising), [r]) if f.reg_ty.get(r.0 as usize) == Some(&yavs_ir::RegTy::Sig) => {
            let kind = if rising {
                yavs_ir::EdgeKind::Rising
            } else {
                yavs_ir::EdgeKind::Falling
            };
            Some(NativeOp::Edge { kind, sig: *r })
        }
        _ => None,
    }
}

/// A comparison as a mask over the three outcomes of `Ord::cmp`: bit `ord + 1` is set when
/// the comparison holds for that outcome (`cmp_holds`), so no dispatch on the operator.
pub(crate) fn cmp_mask(op: CmpOp) -> u8 {
    match op {
        CmpOp::Lt => 0b001,
        CmpOp::Eq => 0b010,
        CmpOp::Le => 0b011,
        CmpOp::Gt => 0b100,
        CmpOp::Ne => 0b101,
        CmpOp::Ge => 0b110,
    }
}

/// Whether the comparison of `cmp_mask` `m` holds for `a` and `b`.
#[inline(always)]
pub(crate) fn cmp_holds(m: u8, a: i64, b: i64) -> bool {
    m >> (a.cmp(&b) as i8 + 1) & 1 != 0
}

/// What a linear form is made of: a register, the position of an index in a descriptor
/// (`Inst::Pos`) or the length of a descriptor's first dimension (`Inst::DopeLen`).
#[derive(Clone, Copy, PartialEq)]
enum Atom {
    Reg(u32),
    Pos { dope: u32, idx: u32 },
    Len { dope: u32 },
}

/// A linear form `c0 + sum coef * atom` of at most two atoms, with bounds on the size of any
/// value the instructions it replaces compute from atoms within 2^30: `am` times 2^30 plus
/// `cm`.
#[derive(Clone)]
struct Lin {
    c0: i128,
    t: [(Atom, i128); 2],
    n: usize,
    am: i128,
    cm: i128,
}

impl Lin {
    fn konst(v: i64) -> Lin {
        Lin {
            c0: v.into(),
            t: [(Atom::Reg(0), 0); 2],
            n: 0,
            am: 0,
            cm: i128::from(v).abs(),
        }
    }

    fn atom(a: Atom) -> Lin {
        Lin {
            c0: 0,
            t: [(a, 1), (Atom::Reg(0), 0)],
            n: 1,
            am: 1,
            cm: 0,
        }
    }

    fn scale(&self, k: i128) -> Lin {
        let mut l = self.clone();
        l.c0 *= k;
        for t in &mut l.t {
            t.1 *= k;
        }
        l.am *= k.abs();
        l.cm *= k.abs();
        l
    }

    /// `self + sign * o`.
    fn add(&self, o: &Lin, sign: i128) -> Option<Lin> {
        let mut l = self.clone();
        l.c0 += sign * o.c0;
        for &(a, c) in &o.t[..o.n] {
            match l.t[..l.n].iter_mut().find(|t| t.0 == a) {
                Some(t) => t.1 += sign * c,
                None if l.n < 2 => {
                    l.t[l.n] = (a, sign * c);
                    l.n += 1;
                }
                None => return None,
            }
        }
        l.am += o.am;
        l.cm += o.cm;
        Some(l)
    }

    /// Whether the form has an op (`lin_op`) and cannot overflow `i64` for atoms within 2^30.
    fn fits(&self) -> bool {
        let kinds = match self.t[..self.n] {
            [] | [_] => true,
            [(Atom::Reg(_), _), (Atom::Reg(_), _)] => true,
            [(Atom::Pos { dope: a, .. }, _), (Atom::Len { dope: b }, _)]
            | [(Atom::Len { dope: b }, _), (Atom::Pos { dope: a, .. }, _)] => a == b,
            // (`AffPR` keeps its constant in 16 bits.)
            [(Atom::Pos { .. }, _), (Atom::Reg(_), _)]
            | [(Atom::Reg(_), _), (Atom::Pos { .. }, _)] => self.c0.abs() <= i128::from(i16::MAX),
            _ => false,
        };
        kinds
            && self.c0.abs() <= i128::from(i32::MAX)
            && self.t[..self.n]
                .iter()
                .all(|t| t.1.abs() <= i128::from(i16::MAX))
            && self.am <= 1 << 31
            && self.cm <= 1 << 40
    }

    /// The op computing the form into `dst`.
    fn op(&self, dst: u32) -> Op {
        let co = |i: usize| self.t[i].1 as i16;
        let c0 = self.c0 as i32;
        match self.t[..self.n] {
            [] => Op::Const {
                dst,
                v: self.c0 as i64,
            },
            [(Atom::Reg(a), _)] => Op::Aff1 {
                dst,
                a,
                ca: co(0),
                c0,
            },
            [(Atom::Reg(a), _), (Atom::Reg(b), _)] => Op::Aff2 {
                dst,
                a,
                b,
                ca: co(0),
                cb: co(1),
                c0,
            },
            [(Atom::Pos { dope, idx }, _)] => Op::AffP {
                dst,
                dope,
                idx,
                cp: co(0),
                c0,
            },
            [(Atom::Len { dope }, _)] => Op::AffL {
                dst,
                dope,
                cl: co(0),
                c0,
            },
            [(Atom::Pos { dope, idx }, _), (Atom::Len { .. }, _)] => Op::AffPL {
                dst,
                dope,
                idx,
                cp: co(0),
                cl: co(1),
                c0,
            },
            [(Atom::Len { .. }, _), (Atom::Pos { dope, idx }, _)] => Op::AffPL {
                dst,
                dope,
                idx,
                cp: co(1),
                cl: co(0),
                c0,
            },
            [(Atom::Pos { dope, idx }, _), (Atom::Reg(a), _)] => Op::AffPR {
                dst,
                dope,
                idx,
                a,
                cp: co(0),
                ca: co(1),
                c0: self.c0 as i16,
            },
            [(Atom::Reg(a), _), (Atom::Pos { dope, idx }, _)] => Op::AffPR {
                dst,
                dope,
                idx,
                a,
                cp: co(1),
                ca: co(0),
                c0: self.c0 as i16,
            },
            // `fits` admits no other pair.
            _ => Op::Step(NONE),
        }
    }
}

/// The ops of a run (`lin_run`): all but the last, the last, and the pc of the run's last
/// instruction.
struct LinRun {
    head: Vec<(Op, u32)>,
    tail: (Op, u32),
    end: usize,
}

struct LinCx<'a> {
    f: &'a Func,
    skip: &'a [bool],
    block_start: &'a [bool],
    known: &'a [Known],
    defs: &'a [u8],
    uses: &'a [u32],
}

/// The run of add, subtract and multiply-by-constant `Bin`s, `Pos`es and `DopeLen`s of the
/// first dimension starting at `pc` (within a block, only preloaded definitions between them,
/// each result defined once) as one op per result something else reads (`Lin::op`), in
/// program order, with the IR pc of the instruction that defines it; and the pc of the run's
/// last instruction. `None` when the first instruction does not fit.
fn lin_run(cx: &LinCx, pc: usize) -> Option<LinRun> {
    let n = cx.f.code.len();
    // (register, its form, use count by the run, pc of its definition).
    let mut forms: Vec<(u32, Lin, u32, usize)> = Vec::new();
    let (mut p, mut last) = (pc, pc);
    while p < n {
        if p != pc && cx.block_start[p] {
            break;
        }
        if cx.skip[p] {
            p += 1;
            continue;
        }
        let once = |r: yavs_ir::Reg| cx.defs.get(r.0 as usize) == Some(&1);
        let virt = |r: yavs_ir::Reg| forms.iter().any(|x| x.0 == r.0);
        let form = |r: yavs_ir::Reg| match forms.iter().find(|x| x.0 == r.0) {
            Some(x) => x.1.clone(),
            None => match cx.known.get(r.0 as usize) {
                Some(&Known::Int(v)) => Lin::konst(v),
                _ => Lin::atom(Atom::Reg(r.0)),
            },
        };
        let (dst, lin, srcs) = match cx.f.code[p] {
            Inst::Bin {
                op: op @ (IntOp::Add | IntOp::Sub | IntOp::Mul),
                dst,
                a,
                b,
            } if once(dst) => {
                let (x, y) = (form(a), form(b));
                let lin = match op {
                    IntOp::Add => x.add(&y, 1),
                    IntOp::Sub => x.add(&y, -1),
                    _ if y.n == 0 => Some(x.scale(y.c0)),
                    _ if x.n == 0 => Some(y.scale(x.c0)),
                    _ => None,
                };
                // Constants folded here must not overflow where the instructions would.
                let folded = x.n == 0 && y.n == 0;
                (
                    dst,
                    lin.filter(|l| !folded || i64::try_from(l.c0).is_ok()),
                    [Some(a), Some(b)],
                )
            }
            Inst::Pos {
                dst,
                dope,
                dim: 0,
                idx,
                ..
            } if once(dst) && !virt(dope) && !virt(idx) => (
                dst,
                Some(Lin::atom(Atom::Pos {
                    dope: dope.0,
                    idx: idx.0,
                })),
                [None, None],
            ),
            Inst::DopeLen { dst, dope, dim: 0 } if once(dst) && !virt(dope) => (
                dst,
                Some(Lin::atom(Atom::Len { dope: dope.0 })),
                [None, None],
            ),
            _ => break,
        };
        let Some(lin) = lin.filter(Lin::fits) else {
            break;
        };
        for r in srcs.into_iter().flatten() {
            if let Some(x) = forms.iter_mut().find(|x| x.0 == r.0) {
                x.2 += 1;
            }
        }
        forms.push((dst.0, lin, 0, p));
        last = p;
        p += 1;
    }
    // A run of `Pos`es and `DopeLen`s alone gains nothing over their own ops (and fusions).
    let has_bin = (pc..=last).any(|q| matches!(cx.f.code[q], Inst::Bin { .. }));
    if forms.is_empty() || !has_bin {
        return None;
    }
    let mut emit = Vec::new();
    for (dst, l, inrun, at) in &forms {
        if *inrun != 0 && cx.uses.get(*dst as usize) == Some(inrun) {
            continue;
        }
        emit.push((l.op(*dst), *at as u32));
    }
    let tail = emit.pop()?;
    Some(LinRun {
        head: emit,
        tail,
        end: last,
    })
}

/// The registers `i` reads, list operands included.
fn reads(f: &Func, i: &Inst, mut each: impl FnMut(yavs_ir::Reg)) {
    i.visit(&mut |r, role, _| {
        if role == Role::Use {
            each(r);
        }
    });
    if let Some(rs) = i.list().and_then(|l| f.regs(l)) {
        rs.iter().for_each(|&r| each(r));
    }
}

/// Whether register `r` may be read after the instruction at `pc` and before it is written,
/// on some path (an instruction it cannot follow counts as a read).
pub(crate) fn live_after(f: &Func, pc: usize, r: yavs_ir::Reg) -> bool {
    live_within(f, pc, r, usize::MAX).unwrap_or(true)
}

/// [`live_after`] when the scan decides within `budget` instructions, else `None`.
fn live_within(f: &Func, pc: usize, r: yavs_ir::Reg, mut budget: usize) -> Option<bool> {
    // Made at the first branch: most scans end in the block.
    let mut seen: Vec<bool> = Vec::new();
    let mut stack = vec![pc + 1];
    while let Some(mut p) = stack.pop() {
        loop {
            let Some(i) = f.code.get(p) else {
                return Some(true);
            };
            budget = budget.checked_sub(1)?;
            let (mut read, mut written) = (false, false);
            reads(f, i, |x| read |= x == r);
            i.visit(&mut |x, role, _| written |= role == Role::Def && x == r);
            if read {
                return Some(true);
            }
            if written {
                break;
            }
            let to = |b: yavs_ir::BlockId| f.blocks.get(b).map(|b| b.start as usize);
            let mut push = |s: usize| {
                seen.resize(f.code.len() + 1, false);
                if !std::mem::replace(&mut seen[s], true) {
                    stack.push(s);
                }
            };
            match *i {
                Inst::Br { to: t } => {
                    let Some(s) = to(t) else { return Some(true) };
                    push(s);
                    break;
                }
                Inst::CondBr { t, f: e, .. } => {
                    for b in [t, e] {
                        let Some(s) = to(b) else { return Some(true) };
                        push(s);
                    }
                    break;
                }
                Inst::Ret { .. } | Inst::Unreachable { .. } => break,
                Inst::Case { .. } => return Some(true),
                _ => p += 1,
            }
        }
    }
    Some(false)
}

/// How often each register is read in `f`.
fn read_counts(f: &Func) -> Vec<u32> {
    let mut t = vec![0u32; f.reg_ty.len()];
    for i in &f.code {
        reads(f, i, |r| {
            if let Some(c) = t.get_mut(r.0 as usize) {
                *c += 1;
            }
        });
    }
    t
}

/// The waveform buffer made at IR pc `from` (register `wave`) and written through `base` is
/// read by the `Drive` at `to` and by nothing else: all in one block, `wave` read exactly by
/// its three instructions (`Store`, `Index` and the `Drive`), `base` once, neither read later.
fn waveform_is_private(
    f: &Func,
    block_start: &[bool],
    total: &std::cell::OnceCell<Vec<u32>>,
    from: usize,
    to: usize,
    wave: u32,
    base: u32,
) -> bool {
    if from >= to || (from + 1..=to).any(|p| block_start.get(p).copied().unwrap_or(true)) {
        return false;
    }
    let (mut nw, mut nb) = (0, 0);
    for i in f.code.iter().take(to + 1).skip(from) {
        reads(f, i, |r| {
            nw += u32::from(r.0 == wave);
            nb += u32::from(r.0 == base);
        });
    }
    // A scan that is not over after 256 instructions is replaced by a count: a register read only
    // in the range is dead after it (nothing enters the range but at `from`, which defines `wave`,
    // and `base` follows), so the rest of a very large function is not scanned for each waveform.
    let dead = |r: u32, n: u32| {
        let reg = yavs_ir::Reg(r);
        match live_within(f, to, reg, 256) {
            Some(live) => !live,
            None => {
                total.get_or_init(|| read_counts(f)).get(r as usize) == Some(&n)
                    || !live_after(f, to, reg)
            }
        }
    };
    nw == 3 && nb == 1 && dead(wave, nw) && dead(base, nb)
}

/// The waveform in the frame bytes `off..off + bytes` (`yavs_ir` `opt/slots.rs`), zeroed at IR
/// pc `from`, is read by the `Drive` at `to` and by nothing else: all in one block, the reads of
/// every register that points into those bytes (not only the waveform's two: the frame starts
/// with each address derived by a constant) number `want` there (the `Zero`, the `Store` of the
/// time if any, the write of the value and the `Drive`), and none of them is read after it.
fn frame_wave_is_private(
    f: &Func,
    known: &[Known],
    block_start: &[bool],
    total: &std::cell::OnceCell<Vec<u32>>,
    (from, to): (usize, usize),
    (off, bytes): (u32, u32),
    want: u32,
) -> bool {
    if from >= to || (from + 1..=to).any(|p| block_start.get(p).copied().unwrap_or(true)) {
        return false;
    }
    let inside = |r: yavs_ir::Reg| matches!(known.get(r.0 as usize), Some(&Known::Frame(o)) if o >= off && o - off < bytes);
    let mut regs: Vec<(u32, u32)> = Vec::new();
    for i in f.code.iter().take(to + 1).skip(from) {
        reads(f, i, |r| {
            if inside(r) {
                match regs.iter_mut().find(|x| x.0 == r.0) {
                    Some(x) => x.1 += 1,
                    None => regs.push((r.0, 1)),
                }
            }
        });
    }
    // (Those registers are defined once, before `from`: read only in the range, they are dead.)
    regs.iter().map(|x| x.1).sum::<u32>() == want
        && regs.iter().all(|&(r, n)| {
            total.get_or_init(|| read_counts(f)).get(r as usize) == Some(&n)
                || live_within(f, to, yavs_ir::Reg(r), 256) == Some(false)
        })
}

/// Bit of the `size` field of `CopyDrive`, `LSetDrive` and `LCopyDrive`: the waveform is a
/// temporary in the frame, at the address its `w` register holds from the start, and the rest of
/// the field is its size in bytes (else `size` is the register holding the size of an `Alloc`).
pub(crate) const FRAME_WAVE: u16 = 0x8000;

/// Pairs and triples of adjacent ops that are one op (they never cross a block start: the
/// first op's pc stays an entry, the others' pcs are not).
#[allow(clippy::too_many_arguments)]
fn peephole(
    f: &Func,
    ops: &mut Vec<Op>,
    at: &mut Vec<u32>,
    start: &mut [u32],
    aff: &mut Vec<(u32, u32, u32)>,
    no_tier: &mut Vec<(u32, u32)>,
    block_start: &[bool],
    uses: &[u32],
    known: &[Known],
) {
    let (old, old_at) = (std::mem::take(ops), std::mem::take(at));
    // The waveform starts `WaveBuf` made so far: wave register, the register 8 bytes in and
    // the IR pc of the `Alloc`.
    // The `Copy` positions where a `CopyDrive` goes, and the op.
    let mut dropped: Vec<(usize, Op, u32)> = Vec::new();
    // What the dropped waveform was filled with.
    enum Val {
        /// `Copy` of a block: its address and byte count.
        Copy(u32, u32),
        /// `LSet` of element 0: the code.
        LSet(u32),
        /// `LCopy` into element 0: the source, its first element, the count.
        LCopy(u32, u32, u32),
    }
    let int = |r: u32| match known.get(r as usize) {
        Some(&Known::Int(v)) => Some(v),
        _ => None,
    };
    let mut remap = vec![NONE; old.len()];
    let mut new_aff = Vec::with_capacity(aff.len());
    // The run an `Aff` op stands for.
    let run = |k: usize| {
        aff.binary_search_by_key(&(k as u32), |e| e.0)
            .ok()
            .and_then(|i| aff.get(i).map(|e| (e.1, e.2)))
    };
    let free = |k: usize| old_at.get(k).is_some_and(|&p| !block_start[p as usize]);
    // A register the frame starts with that holds 0 (the time of a waveform in the frame whose
    // zero store the optimiser dropped). The replay of compiled code takes the `LSet`'s index
    // instead (`Host::drive_bit_replay`): keep the two in step.
    let zero_reg = known
        .iter()
        .position(|k| matches!(k, Known::Int(0)))
        .and_then(|r| u16::try_from(r).ok());
    // The reads of each register in the function, found when a waveform needs them.
    let total = std::cell::OnceCell::new();
    let mut i = 0;
    while i < old.len() {
        let here = ops.len() as u32;
        let (fused, len) = match (old[i], old.get(i + 1), old.get(i + 2)) {
            (
                Op::Alloc { dst: w, size },
                Some(&Op::Store {
                    addr,
                    src: time,
                    w: Width::B8,
                }),
                Some(&Op::IndexImm {
                    dst: e,
                    base,
                    off: 8,
                }),
            ) if addr == w && base == w && free(i + 1) && free(i + 2) => {
                // The `Copy` into the waveform and the `Drive` of it, later in the block, with
                // nothing else using it: both become one op there and these three are not run.
                // (Compiled code may take over the frame after a call that is not run inline, and
                // would find the waveform's registers undefined: the ops from here to the fused
                // one are no entry for it, `Code::no_tier`.)
                // Only the first `Drive` of the waveform can be the one: a later one finds the
                // waveform read four times, so the search ends at the first (#683).
                let seen = std::cell::Cell::new(false);
                let go = |j: usize| free(j) && !seen.get();
                let pair = (i + 3..old.len()).take_while(|&j| go(j)).find_map(|j| {
                    let Some(&Op::Drive {
                        driver,
                        target,
                        wave,
                        n,
                        reject: NONE,
                        inertial,
                    }) = old.get(j + 1)
                    else {
                        return None;
                    };
                    seen.set(wave == w);
                    let (from, to) = (*old_at.get(i)? as usize, *old_at.get(j + 1)? as usize);
                    let value = match old[j] {
                        Op::Copy { dst, src, bytes } if dst == e => Some(Val::Copy(src, bytes)),
                        Op::LSet { base, idx, v } if base == e && int(idx) == Some(0) => {
                            Some(Val::LSet(v))
                        }
                        Op::LCopy {
                            dst,
                            doff,
                            src,
                            soff,
                            n: cnt,
                        } if dst == e && int(doff) == Some(0) => Some(Val::LCopy(src, soff, cnt)),
                        _ => None,
                    }?;
                    let clean = wave == w
                        && int(n) == Some(1)
                        && free(j + 1)
                        && waveform_is_private(f, block_start, &total, from, to, w, e)
                        && !(from + 3..to).any(|p| {
                            let mut def = false;
                            f.code[p].visit(&mut |r, role, _| {
                                def |= role == Role::Def && (r.0 == time || r.0 == size)
                            });
                            def
                        });
                    let r = |x: u32| u16::try_from(x).ok();
                    let rs = |x: u32| r(x).filter(|&x| x < FRAME_WAVE);
                    match (r(driver), r(target), r(time), rs(size), r(w), r(e), r(n)) {
                        (
                            Some(driver),
                            Some(target),
                            Some(time),
                            Some(size),
                            Some(w16),
                            Some(e16),
                            Some(n),
                        ) if clean => Some((
                            j,
                            match value {
                                Val::Copy(a, b) => Op::CopyDrive {
                                    driver,
                                    target,
                                    src: r(a)?,
                                    bytes: r(b)?,
                                    time,
                                    size,
                                    w: w16,
                                    n,
                                    inertial,
                                },
                                Val::LSet(v) => Op::LSetDrive {
                                    v,
                                    driver,
                                    target,
                                    time,
                                    size,
                                    w: w16,
                                    e: e16,
                                    inertial,
                                },
                                Val::LCopy(a, b, c) => Op::LCopyDrive {
                                    driver,
                                    target,
                                    src: r(a)?,
                                    soff: r(b)?,
                                    cnt: r(c)?,
                                    time,
                                    size,
                                    w: w16,
                                    e: e16,
                                    n,
                                    inertial,
                                },
                            },
                        )),
                        _ => None,
                    }
                });
                match pair {
                    Some((j, op)) => {
                        dropped.push((j, op, ops.len() as u32));
                        remap[i] = ops.len() as u32;
                        i += 3;
                        continue;
                    }
                    None => (Some(Op::WaveBuf { w, size, time, e }), 3),
                }
            }
            (Op::ZeroSlot { w, off, n: bytes }, next, _)
                if let Ok(b16) = u16::try_from(bytes)
                    && b16 < FRAME_WAVE
                    && let Some(w16) = u16::try_from(w).ok()
                    && let Some(&from) = old_at.get(i) =>
            {
                // A waveform in the frame (`yavs_ir` `opt/slots.rs`): its zeroing, the store of
                // the time unless the time is 0, the value written 8 bytes in (a register that
                // holds that address from the start) and the one-element `Drive`. Run as above;
                // the registers keep the frame's addresses, so only `Code::no_tier` matters.
                let (time, skip) = match next {
                    Some(&Op::StoreSlot {
                        src,
                        off: o,
                        w: Width::B8,
                    }) if o == off
                        && free(i + 1)
                        && let Some(&sp) = old_at.get(i + 1) =>
                    {
                        (Some((src, sp)), 2)
                    }
                    _ => (None, 1),
                };
                let at_e = |b: u32| matches!(known.get(b as usize), Some(&Known::Frame(o)) if Some(o) == off.checked_add(8));
                let seen = std::cell::Cell::new(false);
                let go = |j: usize| free(j) && !seen.get();
                let pair = (i + skip..old.len()).take_while(|&j| go(j)).find_map(|j| {
                    let Some(&Op::Drive {
                        driver,
                        target,
                        wave,
                        n,
                        reject: NONE,
                        inertial,
                    }) = old.get(j + 1)
                    else {
                        return None;
                    };
                    seen.set(wave == w);
                    let to = *old_at.get(j + 1)? as usize;
                    let (value, e) = match old[j] {
                        Op::Copy { dst, src, bytes } if at_e(dst) => (Val::Copy(src, bytes), dst),
                        Op::LSet { base, idx, v } if at_e(base) && int(idx) == Some(0) => {
                            (Val::LSet(v), base)
                        }
                        Op::LCopy {
                            dst,
                            doff,
                            src,
                            soff,
                            n: cnt,
                        } if at_e(dst) && int(doff) == Some(0) => (Val::LCopy(src, soff, cnt), dst),
                        _ => return None,
                    };
                    let from = from as usize;
                    // A store of the time further on stays (the replay writes that word again).
                    let time = time.or_else(|| {
                        (i + 1..j).find_map(|k| match old[k] {
                            Op::StoreSlot {
                                src,
                                off: o,
                                w: Width::B8,
                            } if o == off => Some((src, *old_at.get(k)?)),
                            _ => None,
                        })
                    });
                    let clean = wave == w
                        && int(n) == Some(1)
                        && free(j + 1)
                        && frame_wave_is_private(
                            f,
                            known,
                            block_start,
                            &total,
                            (from, to),
                            (off, bytes),
                            3 + u32::from(time.is_some()),
                        )
                        && time.is_none_or(|(t, at)| {
                            !(at as usize + 1..to).any(|p| {
                                let mut def = false;
                                f.code[p]
                                    .visit(&mut |r, role, _| def |= role == Role::Def && r.0 == t);
                                def
                            })
                        });
                    if !clean {
                        return None;
                    }
                    let r = |x: u32| u16::try_from(x).ok();
                    // No store: the time is 0, which a register the frame starts with holds.
                    let time = match time {
                        Some((t, _)) => r(t)?,
                        None => zero_reg?,
                    };
                    let (driver, target, e, n) = (r(driver)?, r(target)?, r(e)?, r(n)?);
                    let size = FRAME_WAVE | b16;
                    Some((
                        j,
                        match value {
                            Val::Copy(a, b) => Op::CopyDrive {
                                driver,
                                target,
                                src: r(a)?,
                                bytes: r(b)?,
                                time,
                                size,
                                w: w16,
                                n,
                                inertial,
                            },
                            Val::LSet(v) => Op::LSetDrive {
                                v,
                                driver,
                                target,
                                time,
                                size,
                                w: w16,
                                e,
                                inertial,
                            },
                            Val::LCopy(a, b, c) => Op::LCopyDrive {
                                driver,
                                target,
                                src: r(a)?,
                                soff: r(b)?,
                                cnt: r(c)?,
                                time,
                                size,
                                w: w16,
                                e,
                                n,
                                inertial,
                            },
                        },
                    ))
                });
                match pair {
                    Some((j, op)) => {
                        dropped.push((j, op, ops.len() as u32));
                        remap[i] = ops.len() as u32;
                        i += skip;
                        continue;
                    }
                    None => (None, 1),
                }
            }
            _ if let Some(p) = dropped.iter().position(|d| d.0 == i) => {
                let (_, op, first) = dropped.swap_remove(p);
                no_tier.push((first, here));
                (Some(op), 2)
            }
            (
                Op::AffPL {
                    dst: o,
                    dope,
                    idx,
                    cp,
                    cl,
                    c0,
                },
                Some(&Op::SigRead1 { dst, sig, offset }),
                _,
            ) if offset == o
                && uses.get(o as usize) == Some(&1)
                && free(i + 1)
                && let (Ok(sig), Ok(c0)) = (u16::try_from(sig), i16::try_from(c0)) =>
            {
                (
                    Some(Op::ReadPL {
                        dst,
                        dope,
                        idx,
                        sig,
                        cp,
                        cl,
                        c0,
                    }),
                    2,
                )
            }
            (
                Op::AffPR {
                    dst: o,
                    dope,
                    idx,
                    a,
                    cp,
                    ca: 1,
                    c0,
                },
                Some(&Op::SigRead1 { dst, sig, offset }),
                _,
            ) if offset == o
                && uses.get(o as usize) == Some(&1)
                && free(i + 1)
                && let Ok(sig) = u16::try_from(sig) =>
            {
                (
                    Some(Op::ReadPR {
                        dst,
                        dope,
                        idx,
                        a,
                        sig,
                        cp,
                        c0,
                    }),
                    2,
                )
            }
            _ => (None, 1),
        };
        match fused {
            Some(op) => {
                remap[i] = here;
                if let Some((from, root)) = run(i) {
                    new_aff.push((here, from, root));
                }
                ops.push(op);
                at.push(old_at[i]);
            }
            None => {
                remap[i] = here;
                if let Some((from, root)) = run(i) {
                    new_aff.push((here, from, root));
                }
                ops.push(old[i]);
                at.push(old_at[i]);
            }
        }
        i += len;
    }
    for s in start.iter_mut() {
        if *s != NONE {
            *s = remap[*s as usize];
        }
    }
    *aff = new_aff;
}

/// Superinstructions over resolved ops: an op that starts a known sequence is replaced by one
/// that does all of it; the ops after it stay as they are (a jump or a frame resumed there
/// runs them one by one), so no index changes.
/// The wait statement of a process that has no other (`Program::single_suspend`, and a
/// sensitivity list or no call of anything that may suspend), from the `Stmt` that starts it
/// to its `Suspend` in one block, when it only names signals: constants, addresses, loads,
/// lengths, positions, indices, fields and arithmetic, signal references and at least two
/// `WaitOn`s (no timeout). Its `Stmt` op becomes `WaitStatic` (#530). The signals of a wait are
/// static names (LRM 10.2, 11.3 and the equivalent processes of concurrent statements), so
/// every activation names the same ones; with one suspension point, the last wait the kernel
/// holds is this one unless the process just started. Skipping the instructions changes
/// nothing else: they write registers only, and no register is live across a suspension.
fn static_waits(prog: &Program, f: &Func, ops: &mut [Op], at: &[u32], entry: &[u32]) {
    for (st, s) in static_wait(prog, f, 2) {
        let Some(k) =
            (0..ops.len()).find(|&k| matches!(ops[k], Op::Suspend) && at[k] as usize == s)
        else {
            continue;
        };
        if let Some(&k0) = entry.get(st)
            && let Some(o @ Op::Stmt) = ops.get_mut(k0 as usize)
        {
            *o = Op::WaitStatic { to: k as u32 };
        }
    }
}

/// The waits of `f` (at least `min` `WaitOn`s each) that name the same signals at every
/// activation (`static_waits`): the IR pc of the `Stmt` that starts each and of its `Suspend`.
/// Compiled code takes the same short cut (with `min` 1: the call it saves is more than the
/// interpreter's fused wait). A process with a sensitivity list may have several suspension
/// points (the lowering makes the first wait of a list that names array elements or slices a copy
/// that also keeps the references for the later ones, and the activations after the first run the
/// later copy): they all wait for the same list, so every one that qualifies short-cuts.
pub fn static_wait(prog: &Program, f: &Func, min: usize) -> Vec<(usize, usize)> {
    static_wait_edge(prog, f, min, false)
        .into_iter()
        .map(|(st, s, _)| (st, s))
        .collect()
}

/// [`static_wait`], with `edge` also the waits of a `wait until <edge>` (a `WaitEdge` among the
/// instructions, third field `true`): such a wait names the same signals too, and its `WaitEdge`
/// changes nothing where the process has an edge trigger (`Ctx::wait_edge`), which compiled code
/// checks at run time (`TierFrame::known`, #772).
pub fn static_wait_edge(
    prog: &Program,
    f: &Func,
    min: usize,
    edge: bool,
) -> Vec<(usize, usize, bool)> {
    let calls = |i: &Inst| match *i {
        Inst::Call { func, .. } | Inst::CallProt { func, .. } => prog
            .func(func)
            .is_none_or(|c| c.flags.contains(FuncFlags::MAY_SUSPEND)),
        Inst::CallIndirect { .. } => true,
        _ => false,
    };
    let suspends: Vec<usize> = (0..f.code.len())
        .filter(|&p| matches!(f.code[p], Inst::Suspend { .. }))
        .collect();
    let sens = f.flags.contains(FuncFlags::SENS_LIST);
    // Lazily: asking whether a callee may suspend decodes its body.
    let quiet = || !f.code.iter().any(calls);
    let one = suspends.len() == 1 && f.resume.len() == 1 && (sens || quiet());
    // The concurrent statement's process waits on its one list too (no call suspends).
    let same = sens || f.flags.contains(FuncFlags::IMPLICIT) && quiet();
    if !(one || same && !suspends.is_empty()) {
        return Vec::new();
    }
    suspends
        .into_iter()
        .filter_map(|s| {
            let st = (0..s).rev().find(|&p| {
                !matches!(
                    f.code[p],
                    Inst::Const { .. }
                        | Inst::ConstF { .. }
                        | Inst::AddrOf { .. }
                        | Inst::DopeLen { .. }
                        | Inst::Pos { .. }
                        | Inst::SlicePos { .. }
                        | Inst::Index { .. }
                        | Inst::Field { .. }
                        | Inst::Bin { .. }
                        | Inst::BinRange { .. }
                        | Inst::Un { .. }
                        | Inst::Cmp { .. }
                        | Inst::Bool { .. }
                        | Inst::Select { .. }
                        | Inst::Load { .. }
                        | Inst::Alloc { .. }
                        | Inst::Zero { .. }
                        | Inst::MakeDope { .. }
                        | Inst::LayoutOf { .. }
                        | Inst::SigVal { .. }
                        | Inst::SigSlice { .. }
                        | Inst::WaitOn { .. }
                ) && !(edge && matches!(f.code[p], Inst::WaitEdge { .. }))
            })?;
            let waits = f.code[st + 1..s]
                .iter()
                .filter(|i| matches!(i, Inst::WaitOn { .. }))
                .count();
            let has_edge = f.code[st + 1..s]
                .iter()
                .any(|i| matches!(i, Inst::WaitEdge { .. }));
            let starts = |p: usize| f.blocks.iter().any(|(_, b)| b.start as usize == p);
            // A `wait until` is the head of its own loop: `Stmt; Br` to the block of the wait,
            // which the condition jumps back to (that way runs the wait in full).
            let (stmt, from) = match f.code[st] {
                Inst::Br { to }
                    if has_edge
                        && st > 0
                        && matches!(f.code[st - 1], Inst::Stmt)
                        && f.blocks.get(to).is_some_and(|b| b.start as usize == st + 1) =>
                {
                    (st - 1, st + 2)
                }
                _ => (st, st + 1),
            };
            (matches!(f.code[stmt], Inst::Stmt) && waits >= min && !(from..=s).any(starts))
                .then_some((stmt, s, has_edge))
        })
        .collect()
}

/// A scalar logic assignment the predecoder runs as one op (`Op::LSetDrive`): the `Alloc`,
/// `Store` and `Index` that make the waveform (for a waveform in the frame, `yavs_ir`
/// `opt/slots.rs`: its `Zero` and the `Store` of the time unless it is 0), the `LSet` of its
/// element and the one-element `Drive` of it, which nothing else reads. A tier that runs the
/// same pattern as one step leaves the head and the `LSet` alone and does the assignment at the
/// `Drive`; it must use exactly these sites, because the interpreter, which continues where
/// compiled code leaves, does not write the waveform (nor, for an `Alloc`, its registers) either.
#[derive(Clone, Copy, Debug)]
pub struct ScalarDrive {
    /// The IR pcs of the head, in order (`u32::MAX` where there is none).
    pub head: [u32; 3],
    /// The IR pcs of the `LSet` and of the `Drive`.
    pub lset: u32,
    pub drive: u32,
    pub driver: u32,
    /// The signal reference (two registers) the `Drive` names.
    pub target: yavs_ir::Reg,
    /// The register holding the delay in femtoseconds (the waveform element's time).
    pub time: yavs_ir::Reg,
    /// The register holding the logic code.
    pub value: yavs_ir::Reg,
    pub inertial: bool,
}

/// The scalar logic assignments of `f` that the interpreter's predecoder fuses (`ScalarDrive`).
pub fn scalar_drives(prog: &Program, f: &Func) -> Vec<ScalarDrive> {
    let pre = Pre::new(f, prog, false);
    let mut out = Vec::new();
    for (k, op) in pre.code.ops.iter().enumerate() {
        let Op::LSetDrive {
            v,
            driver,
            target,
            time,
            size,
            w,
            e,
            inertial,
        } = *op
        else {
            continue;
        };
        let lset = pre.code.at[k] as usize;
        let find =
            |from: usize, hit: &dyn Fn(&Inst) -> bool| (0..from).rev().find(|&p| hit(&f.code[p]));
        let head = if size & FRAME_WAVE != 0 {
            let Some(zero) = find(
                lset,
                &|i| matches!(i, Inst::Zero { addr, .. } if addr.0 == u32::from(w)),
            ) else {
                continue;
            };
            let store = (zero + 1..lset)
                .find(|&p| matches!(f.code[p], Inst::Store { addr, .. } if addr.0 == u32::from(w)))
                .map_or(u32::MAX, |p| p as u32);
            [zero as u32, store, u32::MAX]
        } else if let (Some(alloc), Some(store), Some(index)) = (
            find(
                lset,
                &|i| matches!(i, Inst::Alloc { dst, .. } if dst.0 == u32::from(w)),
            ),
            find(
                lset,
                &|i| matches!(i, Inst::Store { addr, .. } if addr.0 == u32::from(w)),
            ),
            find(
                lset,
                &|i| matches!(i, Inst::Index { dst, .. } if dst.0 == u32::from(e)),
            ),
        ) {
            [alloc as u32, store as u32, index as u32]
        } else {
            continue;
        };
        let Some(drive) =
            (lset + 1..f.code.len()).find(|&p| matches!(f.code[p], Inst::Drive { .. }))
        else {
            continue;
        };
        out.push(ScalarDrive {
            head,
            lset: lset as u32,
            drive: drive as u32,
            driver: u32::from(driver),
            target: yavs_ir::Reg(u32::from(target)),
            time: yavs_ir::Reg(u32::from(time)),
            value: yavs_ir::Reg(v),
            inertial,
        });
    }
    out
}

/// Where an operand of an element loop comes from: a register the loop does not write, the
/// loop variable, or the result of an earlier step of the same iteration.
#[derive(Clone, Copy, Debug)]
pub(crate) enum V {
    Inv(u32),
    Ind,
    Slot(u8),
}

/// The step of the loop variable: a constant of the latch or the register of the `Bin` after it.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Delta {
    K(i64),
    R(u32),
}

/// The range a `BinRange` result must lie in: constants, or registers the loop does not write.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Bound {
    None,
    K(i32, i32),
    R(u32, u32),
}

/// One instruction of an element loop's body. Every value is affine in the iteration number, so
/// `Machine::loop_plan` evaluates each as a pair (value at 0, change per iteration).
#[derive(Clone, Copy, Debug)]
pub(crate) enum LStep {
    Mov(V),
    K(i64),
    /// `Bin`, `BinRange`, `BinRangeK` of an add, subtract or multiply (then one operand does not
    /// change with the iteration).
    Bin(IntOp, V, V, Bound),
    /// `c0 + ca * a`, `c0 + ca * a + cb * b` with atoms within 2^30 (`Aff1`, `Aff2`).
    Lin1(V, i64, i64),
    Lin2(V, V, i64, i64, i64),
    /// `Inst::Pos`: the position of an index in dimension `dim` of the descriptor in register `dope`.
    Pos {
        dope: V,
        dim: u8,
        idx: V,
    },
    /// `pos_len`'s position: first dimension, a length that fits 63 bits.
    PosL {
        dope: V,
        idx: V,
    },
    /// `DopeLen` of dimension `dim`.
    Len {
        dope: V,
        dim: u8,
    },
    /// `Load` / `LoadOff`: `w` bytes at `base + off`; the loop must not write them.
    Ld(V, u32, Width),
    /// `LoadSlot`: `w` bytes at `off` of the frame's slots.
    LdSlot(u32, Width),
    /// `IndexImm`: the address `off` bytes on.
    Addr(V, u32),
    /// `Cmp` of two numbers the loop does not change (`cmp_mask`).
    Cmp(u8, V, V),
    /// `Select` (and `CmpSelect`, with a `Cmp` before it): `c ? a : b`, `c` not changing.
    Sel(V, V, V),
    /// `LGet`: the code of element `idx` of the logic vector at `base`.
    Get {
        base: V,
        idx: V,
    },
    /// `LSet`: `v` is the slot of the `Get` (a copy) or a register holding a code (a fill).
    Set {
        base: V,
        idx: V,
        v: V,
    },
}

/// A loop over the elements of logic vectors (#520): a body of affine index arithmetic with
/// one `LGet` and the `LSet` of it, or one `LSet` of a register the loop does not write, and a
/// latch that counts the loop variable by 1 or -1 against a bound it does not change. It is
/// entered through the `Op::Loop` that every branch to its head from outside leads to
/// (`elem_loops`); there `Machine::op_loop` runs the iterations that go round again at once,
/// when every position, index and element of them is in range. The generic ops run the rest
/// from the head, the iteration that leaves the loop included, so registers, the exit and every
/// failure are theirs.
pub(crate) struct ElemLoop {
    pub ind: u32,
    pub bound: u32,
    pub delta: Delta,
    /// The latch tests the loop variable after the step (the step is the op before the compare).
    pub after: bool,
    /// The outcomes of `Ord::cmp` of the loop variable against the bound after which the loop
    /// goes round again, as a `cmp_mask`.
    pub cmask: u8,
    pub steps: Vec<LStep>,
}

/// Longest body of an element loop, in steps.
pub(crate) const LOOP_STEPS: usize = 12;

/// Most reads of memory besides the vectors in an element loop.
pub(crate) const LOOP_LOADS: usize = 4;

/// A `cmp_mask` with its operands exchanged.
fn swap_mask(m: u8) -> u8 {
    m & 0b010 | (m & 1) << 2 | m >> 2 & 1
}

/// The branch targets of an op.
fn targets_mut(op: &mut Op) -> [Option<&mut u32>; 2] {
    match op {
        Op::Br { to }
        | Op::MoveBr { to, .. }
        | Op::StoreSlotBr { to, .. }
        | Op::BrBody { to, .. } => [Some(to), None],
        Op::CondBr { t, f, .. }
        | Op::CmpBr { t, f, .. }
        | Op::MoveCondBr { t, f, .. }
        | Op::LoadSlot8CmpBr { t, f, .. }
        | Op::FlagBr { t, f, .. }
        | Op::SlotBr { t, f, .. }
        | Op::EdgeBr { t, f, .. } => [Some(t), Some(f)],
        _ => [None, None],
    }
}

/// Every branch target of an op that jumps back, for [`count_back_edges`].
fn jumps_mut(op: &mut Op) -> [Option<&mut u32>; 2] {
    match op {
        Op::CmpBrStep { t, head, .. } => [Some(t), Some(head)],
        Op::PosIndexLoadCmpBr { t, f, .. } => [Some(t), Some(f)],
        op => targets_mut(op),
    }
}

/// (pc of the `Br`, pc of the head block) of every jump back of `f`: the retreating `Br`s of a
/// depth-first walk of its blocks (a jump to a join the code lays out earlier is none). Every loop
/// that goes round by a `Br` has one.
pub fn retreating_brs(prog: &Program, f: &Func) -> Vec<(u32, u32)> {
    let blocks = f.blocks.values();
    let succ = |b: usize| -> Vec<usize> {
        let bl = &blocks[b];
        if bl.end <= bl.start {
            return Vec::new();
        }
        match f.code[bl.end as usize - 1] {
            Inst::Case { table, .. } => prog.cases.get(table).map_or_else(Vec::new, |t| {
                t.arms
                    .iter()
                    .map(|a| a.2.0 as usize)
                    .chain(std::iter::once(t.default.0 as usize))
                    .collect()
            }),
            ref i => i.targets().iter().map(|t| t.0 as usize).collect(),
        }
    };
    let mut back: Vec<(u32, u32)> = Vec::new();
    let mut state = vec![0u8; blocks.len()];
    let mut stack: Vec<(usize, Vec<usize>, usize)> = Vec::new();
    if !blocks.is_empty() {
        state[0] = 1;
        stack.push((0, succ(0), 0));
    }
    while let Some((b, ss, i)) = stack.last_mut() {
        let Some(&s) = ss.get(*i) else {
            state[*b] = 2;
            stack.pop();
            continue;
        };
        *i += 1;
        let from = *b;
        match state.get(s) {
            Some(0) => {
                state[s] = 1;
                stack.push((s, succ(s), 0));
            }
            Some(1) => {
                let last = blocks[from].end - 1;
                if matches!(f.code[last as usize], Inst::Br { .. }) {
                    back.push((last, blocks[s].start));
                }
            }
            _ => {}
        }
    }
    back
}

/// A function that runs as lowered under elaboration-time evaluation counts its jumps back
/// (`Program::back_edge`, #695): the jump back to the head of a loop goes to a new `Op::Step` of
/// that `Br` of the IR, placed after the end of the code, where `Machine::step_out` counts it.
/// Loops are fused into ops that jump back themselves (`CmpBrStep`), so the counting is a
/// trampoline, and the loop ops of every other function stay as they are. The jumps back are the
/// retreating `Br`s of a depth-first walk of the blocks (a jump to a join the code lays out
/// earlier is none). An element loop (`Op::Loop`) is not counted: its iterations run at once.
fn count_back_edges(prog: &Program, f: &Func, entry: &[u32], ops: &mut Vec<Op>, at: &mut Vec<u32>) {
    let back = retreating_brs(prog, f);
    // The ops in code order come first; the ones the passes appended (element loops) follow.
    let main = at
        .windows(2)
        .position(|w| w[1] < w[0])
        .map_or(at.len(), |p| p + 1);
    let mut tramp: HashMap<u32, u32> = HashMap::new();
    for (br, head) in back {
        let Some(&h) = entry.get(head as usize).filter(|&&h| h != NONE) else {
            continue;
        };
        let k = at[..main].partition_point(|&p| p <= br).saturating_sub(1);
        let mut op = ops[k];
        let mut changed = false;
        for t in jumps_mut(&mut op).into_iter().flatten() {
            if *t == h {
                *t = *tramp.entry(h).or_insert_with(|| {
                    ops.push(Op::Step(br));
                    at.push(br);
                    ops.len() as u32 - 1
                });
                changed = true;
            }
        }
        if changed {
            ops[k] = op;
        }
    }
}

/// The element loops of `ops`: every branch to the head of one from outside it is sent to a new
/// `Op::Loop` (placed after the end of the code) that goes on at the head.
fn elem_loops(ops: &mut Vec<Op>, at: &mut Vec<u32>) -> Vec<ElemLoop> {
    let mut loops: Vec<ElemLoop> = Vec::new();
    for e in 0..ops.len() {
        // (head, last op of the loop, variable, bound, step, test after the step, going round
        // again on these outcomes of the compare) and the end of the body.
        let mut found = Vec::new();
        match ops[e] {
            // The compare fused with the step: the compare, then `i += delta` and the jump back.
            Op::CmpBrStep {
                m,
                delta: d @ (-1 | 1),
                a,
                b,
                dst,
                src,
                head,
                ..
            } if dst == src && (a == dst) != (b == dst) => {
                let m = if a == dst { m } else { swap_mask(m) };
                found.push((
                    head as usize,
                    e + 2,
                    u32::from(dst),
                    u32::from(if a == dst { b } else { a }),
                    Delta::K(i64::from(d)),
                    false,
                    !m & 0b111,
                    e,
                ));
            }
            Op::CmpBr { m, a, b, t, f, .. } => {
                // The step of the variable: the op before the compare (the compare goes back
                // to the head), or the two ops after it (the compare goes on to them).
                let step = |i: usize| match ops.get(i) {
                    Some(&Op::Bin {
                        op: IntOp::Add,
                        dst,
                        a: x,
                        b: s,
                    }) if x == dst && s != dst => Some((dst, s)),
                    _ => None,
                };
                let is_var = |v: u32| (a == v) != (b == v);
                let other = |ind: u32| if a == ind { b } else { a };
                // The outcomes of the compare of the variable against the bound.
                let outcomes = |ind: u32, go_on: bool| {
                    let m = if a == ind { m } else { swap_mask(m) };
                    if go_on { m } else { !m & 0b111 }
                };
                if let Some((ind, s)) = e.checked_sub(1).and_then(step)
                    && is_var(ind)
                {
                    // Step, then compare: either edge back to the head goes round again.
                    for (head, go_on) in [(t as usize, true), (f as usize, false)] {
                        if head + 1 < e {
                            found.push((
                                head,
                                e,
                                ind,
                                other(ind),
                                Delta::R(s),
                                true,
                                outcomes(ind, go_on),
                                e - 1,
                            ));
                        }
                    }
                }
                if let (Some((ind, s)), Some(&Op::Br { to })) = (step(e + 1), ops.get(e + 2))
                    && is_var(ind)
                    && (to as usize) < e
                    && (f as usize == e + 1) != (t as usize == e + 1)
                {
                    // Compare, then step and the jump back: the edge to the step goes round again.
                    found.push((
                        to as usize,
                        e + 2,
                        ind,
                        other(ind),
                        Delta::R(s),
                        false,
                        outcomes(ind, t as usize == e + 1),
                        e,
                    ));
                }
            }
            _ => {}
        }
        for (head, last, ind, bound, delta, after, cmask, end) in found {
            if head >= end || last >= ops.len() {
                continue;
            }
            let steps = match loop_body(&ops[head..end], ind, bound, delta) {
                Ok(steps) => steps,
                Err(i) => {
                    if std::env::var_os("YAVS_DUMPIR").is_some() {
                        eprintln!(
                            "loop of ops {head} to {end} is not an element loop: op {}",
                            head + i
                        );
                    }
                    continue;
                }
            };
            let id = loops.len() as u32;
            let to = ops.len() as u32;
            let mut entered = false;
            for (p, op) in ops.iter_mut().enumerate() {
                if p >= head && p <= last {
                    continue;
                }
                for t in targets_mut(op).into_iter().flatten() {
                    if *t as usize == head {
                        *t = to;
                        entered = true;
                    }
                }
            }
            if entered {
                let pc = at[head];
                ops.push(Op::Loop {
                    id,
                    to: head as u32,
                });
                at.push(pc);
                loops.push(ElemLoop {
                    ind,
                    bound,
                    delta,
                    after,
                    cmask,
                    steps,
                });
                break;
            }
        }
    }
    loops
}

/// The steps of a loop body, or `None` when it holds anything else, reads a register before the
/// iteration writes it (a value carried to the next iteration), writes the loop variable, the
/// bound or the step, or has a shape the plan cannot do.
fn loop_body(body: &[Op], ind: u32, bound: u32, delta: Delta) -> Result<Vec<LStep>, usize> {
    // The registers the body writes.
    let mut written: Vec<u32> = Vec::new();
    for op in body {
        match *op {
            Op::Pos { dst, .. }
            | Op::Aff1 { dst, .. }
            | Op::Aff2 { dst, .. }
            | Op::AffP { dst, .. }
            | Op::AffL { dst, .. }
            | Op::AffPL { dst, .. }
            | Op::AffPR { dst, .. }
            | Op::Bin { dst, .. }
            | Op::BinRange { dst, .. }
            | Op::BinRangeK { dst, .. }
            | Op::Move { dst, .. }
            | Op::Const { dst, .. }
            | Op::DopeLen { dst, .. }
            | Op::Load { dst, .. }
            | Op::LoadOff { dst, .. }
            | Op::LoadSlot { dst, .. }
            | Op::LoadSlot8 { dst, .. }
            | Op::IndexImm { dst, .. }
            | Op::Cmp { dst, .. }
            | Op::Select { dst, .. }
            | Op::CmpSelect { dst, .. }
            | Op::LGet { dst, .. } => written.push(dst),
            Op::LoadSlot8x2 { d1, d2, .. } => {
                written.push(d1);
                written.push(d2);
            }
            Op::LSet { .. } => {}
            _ => return Err(0),
        }
    }
    let keep = [
        Some(ind),
        Some(bound),
        match delta {
            Delta::R(r) => Some(r),
            Delta::K(_) => None,
        },
    ];
    if written.iter().any(|w| keep.contains(&Some(*w))) {
        return Err(0);
    }
    struct Cx {
        steps: Vec<LStep>,
        /// Whether each slot changes with the iteration, and whether it is a loaded code.
        dep: Vec<bool>,
        data: Vec<bool>,
        defined: Vec<(u32, u8)>,
        ind: u32,
        written: Vec<u32>,
    }
    impl Cx {
        fn rd(&self, r: u32) -> Option<V> {
            if r == self.ind {
                Some(V::Ind)
            } else if let Some(&(_, s)) = self.defined.iter().rev().find(|d| d.0 == r) {
                Some(V::Slot(s))
            } else if self.written.contains(&r) {
                // Written later in the body: the value of the previous iteration.
                None
            } else {
                Some(V::Inv(r))
            }
        }
        fn dep(&self, v: V) -> bool {
            match v {
                V::Inv(_) => false,
                V::Ind => true,
                V::Slot(s) => self.dep[s as usize],
            }
        }
        fn data(&self, v: V) -> bool {
            matches!(v, V::Slot(s) if self.data[s as usize])
        }
        /// A step whose result is a number (`dep`: it changes with the iteration).
        fn push(&mut self, st: LStep, dep: bool, dst: Option<u32>) -> Option<V> {
            if self.steps.len() >= LOOP_STEPS {
                return None;
            }
            let s = self.steps.len() as u8;
            self.steps.push(st);
            self.dep.push(dep);
            self.data.push(false);
            if let Some(d) = dst {
                self.defined.push((d, s));
            }
            Some(V::Slot(s))
        }
        /// `Bin`: an add, subtract or multiply (by something that does not change).
        fn bin(&mut self, op: IntOp, dst: u32, a: u32, b: u32, bound: Bound) -> Option<()> {
            let (x, y) = (self.num(self.rd(a)?)?, self.num(self.rd(b)?)?);
            let dep = self.dep(x) || self.dep(y);
            if !matches!(op, IntOp::Add | IntOp::Sub | IntOp::Mul)
                || op == IntOp::Mul && self.dep(x) && self.dep(y)
            {
                return None;
            }
            self.push(LStep::Bin(op, x, y, bound), dep, Some(dst))?;
            Some(())
        }
        /// An atom of a linear form: a number.
        fn num(&self, v: V) -> Option<V> {
            (!self.data(v)).then_some(v)
        }
        /// A register the loop does not write.
        fn inv_reg(&self, r: u32) -> Option<u32> {
            matches!(self.rd(r)?, V::Inv(_)).then_some(r)
        }
        /// A register whose value does not change with the iteration (not a loaded code).
        fn inv(&self, r: u32) -> Option<V> {
            let v = self.rd(r)?;
            (!self.dep(v) && !self.data(v)).then_some(v)
        }
    }
    let mut cx = Cx {
        steps: Vec::new(),
        dep: Vec::new(),
        data: Vec::new(),
        defined: Vec::new(),
        ind,
        written,
    };
    let (mut gets, mut sets) = (0, 0);
    let mut skip = false;
    let loads = body
        .iter()
        .map(|o| match o {
            Op::Load { .. } | Op::LoadOff { .. } | Op::LoadSlot { .. } | Op::LoadSlot8 { .. } => 1,
            Op::LoadSlot8x2 { .. } => 2,
            _ => 0,
        })
        .sum::<usize>();
    if loads > LOOP_LOADS {
        return Err(0);
    }
    for (i, op) in body.iter().enumerate() {
        // The `Select` a `CmpSelect` stands for is not run after it.
        if std::mem::take(&mut skip) {
            continue;
        }
        let ok = (|| -> Option<()> {
            match *op {
                Op::Move { dst, src } => {
                    let v = cx.rd(src)?;
                    let dep = cx.dep(v);
                    let data = cx.data(v);
                    cx.push(LStep::Mov(v), dep, Some(dst))?;
                    if let Some(d) = cx.data.last_mut() {
                        *d = data;
                    }
                }
                Op::Const { dst, v } => {
                    cx.push(LStep::K(v), false, Some(dst))?;
                }
                Op::Bin { op, dst, a, b } => cx.bin(op, dst, a, b, Bound::None)?,
                Op::BinRangeK {
                    op,
                    dst,
                    a,
                    b,
                    lo,
                    hi,
                } => cx.bin(op, dst, a, b, Bound::K(lo, hi))?,
                Op::BinRange {
                    op,
                    dst,
                    a,
                    b,
                    lo,
                    hi,
                } => {
                    let bound = Bound::R(cx.inv_reg(lo)?, cx.inv_reg(hi)?);
                    cx.bin(op, dst, a, b, bound)?
                }
                Op::Aff1 { dst, a, ca, c0 } => {
                    let x = cx.num(cx.rd(a)?)?;
                    let dep = cx.dep(x);
                    cx.push(LStep::Lin1(x, ca.into(), c0.into()), dep, Some(dst))?;
                }
                Op::Aff2 {
                    dst,
                    a,
                    b,
                    ca,
                    cb,
                    c0,
                } => {
                    let (x, y) = (cx.num(cx.rd(a)?)?, cx.num(cx.rd(b)?)?);
                    let dep = cx.dep(x) || cx.dep(y);
                    cx.push(
                        LStep::Lin2(x, y, ca.into(), cb.into(), c0.into()),
                        dep,
                        Some(dst),
                    )?;
                }
                Op::Pos {
                    dst,
                    dope,
                    idx,
                    dim,
                    ..
                } => {
                    let dope = cx.inv(dope)?;
                    let x = cx.num(cx.rd(idx)?)?;
                    let dep = cx.dep(x);
                    cx.push(LStep::Pos { dope, dim, idx: x }, dep, Some(dst))?;
                }
                Op::DopeLen { dst, dope, dim } => {
                    let dope = cx.inv(dope)?;
                    cx.push(LStep::Len { dope, dim }, false, Some(dst))?;
                }
                Op::AffP {
                    dst,
                    dope,
                    idx,
                    cp,
                    c0,
                } => {
                    let dope = cx.inv(dope)?;
                    let x = cx.num(cx.rd(idx)?)?;
                    let dep = cx.dep(x);
                    let p = cx.push(LStep::PosL { dope, idx: x }, dep, None)?;
                    cx.push(LStep::Lin1(p, cp.into(), c0.into()), dep, Some(dst))?;
                }
                Op::AffL { dst, dope, cl, c0 } => {
                    let dope = cx.inv(dope)?;
                    let l = cx.push(LStep::Len { dope, dim: 0 }, false, None)?;
                    cx.push(LStep::Lin1(l, cl.into(), c0.into()), false, Some(dst))?;
                }
                Op::AffPL {
                    dst,
                    dope,
                    idx,
                    cp,
                    cl,
                    c0,
                } => {
                    let dope = cx.inv(dope)?;
                    let x = cx.num(cx.rd(idx)?)?;
                    let dep = cx.dep(x);
                    let p = cx.push(LStep::PosL { dope, idx: x }, dep, None)?;
                    let l = cx.push(LStep::Len { dope, dim: 0 }, false, None)?;
                    cx.push(
                        LStep::Lin2(p, l, cp.into(), cl.into(), c0.into()),
                        dep,
                        Some(dst),
                    )?;
                }
                Op::AffPR {
                    dst,
                    dope,
                    idx,
                    a,
                    cp,
                    ca,
                    c0,
                } => {
                    let dope = cx.inv(dope)?;
                    let x = cx.num(cx.rd(idx)?)?;
                    let y = cx.num(cx.rd(a)?)?;
                    let dep = cx.dep(x) || cx.dep(y);
                    let p = cx.push(LStep::PosL { dope, idx: x }, cx.dep(x), None)?;
                    cx.push(
                        LStep::Lin2(p, y, cp.into(), ca.into(), c0.into()),
                        dep,
                        Some(dst),
                    )?;
                }
                Op::Load { dst, addr, w } => {
                    let a = cx.inv(addr)?;
                    cx.push(LStep::Ld(a, 0, w), false, Some(dst))?;
                }
                Op::LoadOff { dst, base, off, w } => {
                    let a = cx.inv(base)?;
                    cx.push(LStep::Ld(a, off, w), false, Some(dst))?;
                }
                Op::LoadSlot { dst, off, w } => {
                    cx.push(LStep::LdSlot(off, w), false, Some(dst))?;
                }
                Op::LoadSlot8 { dst, off } => {
                    cx.push(LStep::LdSlot(off, Width::B8), false, Some(dst))?;
                }
                Op::LoadSlot8x2 { d1, o1, d2, o2 } => {
                    cx.push(LStep::LdSlot(o1, Width::B8), false, Some(d1))?;
                    cx.push(LStep::LdSlot(o2, Width::B8), false, Some(d2))?;
                }
                Op::IndexImm { dst, base, off } => {
                    let a = cx.inv(base)?;
                    cx.push(LStep::Addr(a, off), false, Some(dst))?;
                }
                Op::Cmp { m, dst, a, b } => {
                    let (x, y) = (cx.num(cx.rd(a)?)?, cx.num(cx.rd(b)?)?);
                    if cx.dep(x) || cx.dep(y) {
                        return None;
                    }
                    cx.push(LStep::Cmp(m, x, y), false, Some(dst))?;
                }
                Op::Select { dst, c, a, b } => {
                    let (c, x, y) = (cx.num(cx.rd(c)?)?, cx.num(cx.rd(a)?)?, cx.num(cx.rd(b)?)?);
                    if cx.dep(c) {
                        return None;
                    }
                    let dep = cx.dep(x) || cx.dep(y);
                    cx.push(LStep::Sel(c, x, y), dep, Some(dst))?;
                }
                Op::CmpSelect { m, dst, a, b, x, y } => {
                    let (p, q) = (cx.num(cx.rd(a)?)?, cx.num(cx.rd(b)?)?);
                    let (x, y) = (cx.num(cx.rd(x)?)?, cx.num(cx.rd(y)?)?);
                    if cx.dep(p) || cx.dep(q) {
                        return None;
                    }
                    let c = cx.push(LStep::Cmp(m, p, q), false, None)?;
                    let dep = cx.dep(x) || cx.dep(y);
                    cx.push(LStep::Sel(c, x, y), dep, Some(dst))?;
                    skip = true;
                }
                Op::LGet { dst, base, idx } => {
                    gets += 1;
                    let base = cx.inv(base)?;
                    let x = cx.num(cx.rd(idx)?)?;
                    let s = cx.push(LStep::Get { base, idx: x }, true, Some(dst))?;
                    if let V::Slot(s) = s {
                        cx.data[s as usize] = true;
                    }
                }
                Op::LSet { base, idx, v } => {
                    sets += 1;
                    let base = cx.inv(base)?;
                    let x = cx.num(cx.rd(idx)?)?;
                    let v = cx.rd(v)?;
                    // A copy of the loaded code, or a fill with a code that does not change.
                    if !(cx.data(v) || matches!(v, V::Inv(_))) {
                        return None;
                    }
                    if cx.steps.len() >= LOOP_STEPS {
                        return None;
                    }
                    cx.steps.push(LStep::Set { base, idx: x, v });
                    cx.dep.push(false);
                    cx.data.push(false);
                }
                _ => return None,
            }
            Some(())
        })();
        if ok.is_none() {
            return Err(i);
        }
    }
    // A copy (one `LGet`, one `LSet` of it) or a fill (one `LSet`).
    let copy = cx
        .steps
        .iter()
        .any(|s| matches!(s, LStep::Set { v: V::Slot(_), .. }));
    if sets == 1 && gets == usize::from(copy) {
        Ok(cx.steps)
    } else {
        Err(body.len())
    }
}

/// Is the native the `is_x` of a vector of `std_ulogic`?
/// `Load`/`LoadOff` and a `LoadOff` of the next word from the same base: `LoadPair`. After the
/// element loops are planned (their analysis knows the single loads only); the second op stays in
/// place for a jump or a stop inside the pair.
fn word_pairs(ops: &mut [Op]) {
    for k in 0..ops.len().saturating_sub(1) {
        let pair = match (ops[k], ops[k + 1]) {
            (
                Op::Load {
                    dst: d1,
                    addr,
                    w: Width::B8,
                },
                Op::LoadOff {
                    dst: d2,
                    base,
                    off: 8,
                    w: Width::B8,
                },
            ) if base == addr && d1 != addr => Some((d1, d2, addr, 0)),
            (
                Op::LoadOff {
                    dst: d1,
                    base: b1,
                    off: o1,
                    w: Width::B8,
                },
                Op::LoadOff {
                    dst: d2,
                    base: b2,
                    off: o2,
                    w: Width::B8,
                },
            ) if b1 == b2 && d1 != b1 && o1.checked_add(8) == Some(o2) => Some((d1, d2, b1, o1)),
            _ => None,
        };
        if let Some((d1, d2, base, off)) = pair {
            ops[k] = Op::LoadPair { d1, d2, base, off };
        }
    }
}

fn is_x_vec(prog: &Program, native: u32) -> bool {
    prog.natives
        .get(yavs_ir::NativeId(native))
        .and_then(|n| yavs_ir::ieee::symbol_key(&n.symbol))
        .and_then(|k| yavs_ir::ieee::parse(k.split_once('@').map_or(k, |k| k.0)))
        .is_some_and(|op| matches!(op, yavs_ir::ieee::Op::IsX(true)))
}

fn superops(prog: &Program, ops: &mut [Op], at: &[u32], f: &Func, known: &[Known], uses: &[u32]) {
    let int = |r: u32| match known.get(r as usize) {
        Some(&Known::Int(v)) => Some(v),
        _ => None,
    };
    let once = |r: u32| uses.get(r as usize) == Some(&1);
    let r16 = |r: u32| u16::try_from(r).ok();
    // A new block of known size: `Alloc`, or the zeroing of a temporary in the frame (its
    // register keeps the address). The register, the bytes, whether in the frame.
    let block = |op: Op| match op {
        Op::Alloc { dst, size } => int(size)
            .and_then(|n| u32::try_from(n).ok())
            .map(|n| (dst, n, false)),
        Op::ZeroSlot { w, n, .. } => Some((w, n, true)),
        _ => None,
    };
    // Does register `r` point into the `n` frame bytes at `off`? (A fused frame op skips the
    // zeroing, so an operand inside the block would read what the `Zero` clears.)
    let in_slot = |r: u32, off: u32, n: u32| matches!(known.get(r as usize), Some(&Known::Frame(o)) if o >= off && o - off < n);
    // `Alloc` of a vector block at `k`, the `LCopy` into it of a slice and the `is_x` of the copy
    // (nothing else reads it): one op (#770).
    let is_x_slice = |ops: &[Op], k: usize| -> Option<Op> {
        match (ops.get(k), ops.get(k + 1), ops.get(k + 2)) {
            (
                Some(&first),
                Some(&Op::LCopy {
                    dst: d2,
                    doff,
                    src,
                    soff,
                    n,
                }),
                Some(&Op::CallIeee {
                    native,
                    len: 4,
                    dst: res,
                    regs: [a0, ..],
                    ..
                }),
            ) if let Some((tmp, 32, false)) = block(first)
                && d2 == tmp
                && src != tmp
                && res != NONE16
                && int(doff) == Some(0)
                && is_x_vec(prog, u32::from(native))
                && u32::from(a0) == tmp
                && let Some(&cp) = at.get(k + 2)
                && !live_after(f, cp as usize, yavs_ir::Reg(tmp)) =>
            {
                Some(Op::IsXSlice {
                    tmp,
                    src,
                    soff,
                    n,
                    res: u32::from(res),
                })
            }
            _ => None,
        }
    };
    for k in 0..ops.len() {
        let fused = match (ops[k], ops.get(k + 1), ops.get(k + 2)) {
            (
                Op::PosIndexLoad {
                    dst,
                    dope,
                    idx,
                    base,
                    stride,
                    w,
                    dim,
                },
                Some(&Op::Index {
                    dst: dst2,
                    base: base2,
                    pos,
                    stride: stride2,
                }),
                _,
            ) if pos == dst
                && let (Some(dst), Some(dope), Some(idx), Some(base), Some(dst2), Some(base2)) = (
                    r16(dst),
                    r16(dope),
                    r16(idx),
                    r16(base),
                    r16(dst2),
                    r16(base2),
                )
                && let (Ok(stride), Ok(stride2)) =
                    (u16::try_from(stride), u16::try_from(stride2)) =>
            {
                Some(Op::PosIndexLoadIndex {
                    dst,
                    dope,
                    idx,
                    base,
                    dst2,
                    base2,
                    stride,
                    stride2,
                    w,
                    dim,
                })
            }
            (
                Op::Aff2 {
                    dst: ad,
                    a,
                    b,
                    ca,
                    cb,
                    c0,
                },
                Some(&Op::LCopy {
                    dst: ld,
                    doff,
                    src,
                    soff,
                    n,
                }),
                _,
            ) if doff == ad
                && let (Ok(ca), Ok(cb), Ok(c0)) =
                    (i8::try_from(ca), i8::try_from(cb), i16::try_from(c0))
                && let (Some(ad), Some(a), Some(b), Some(ld), Some(src), Some(soff), Some(n)) = (
                    r16(ad),
                    r16(a),
                    r16(b),
                    r16(ld),
                    r16(src),
                    r16(soff),
                    r16(n),
                ) =>
            {
                Some(Op::AffLCopy {
                    ad,
                    a,
                    b,
                    ld,
                    src,
                    soff,
                    n,
                    ca,
                    cb,
                    c0,
                })
            }
            (
                Op::BinRangeK {
                    op: IntOp::Mul,
                    dst: a,
                    a: kr,
                    b: i,
                    lo: i32::MIN,
                    hi: i32::MAX,
                },
                Some(&Op::BinRangeK {
                    op: IntOp::Add,
                    dst: b,
                    a: a2,
                    b: cr,
                    lo: i32::MIN,
                    hi: i32::MAX,
                }),
                Some(&Op::SlicePos(pc)),
            ) if a2 == a
                && a != b
                && let Some(kv) = int(kr)
                    .and_then(|v| i8::try_from(v).ok())
                    .filter(|&v| v > 0)
                && let Some(c) = int(cr)
                    .and_then(|v| i8::try_from(v).ok())
                    .filter(|&v| v >= 0)
                && let Some(&Inst::SlicePos {
                    dst: p,
                    dope,
                    left,
                    right,
                    dir,
                    ..
                }) = f.code.get(pc as usize)
                && left.0 == a
                && right.0 == b
                && int(dir.0) == Some(0)
                && dope.0 != a
                && dope.0 != b =>
            {
                Some(Op::SliceAff {
                    i,
                    dope: dope.0,
                    a,
                    b,
                    p: p.0,
                    k: kv,
                    c,
                })
            }
            (Op::AddrUp { dst: a, depth, off }, Some(&Op::Load { dst, addr, w }), _)
                if addr == a
                    && let Some(&lp) = at.get(k + 1)
                    && (dst == a || !live_after(f, lp as usize, yavs_ir::Reg(a))) =>
            {
                Some(Op::LoadUp { dst, depth, w, off })
            }
            (Op::AddrUp { dst: a, depth, off }, Some(&Op::Store { addr, src, w }), _)
                if addr == a
                    && src != a
                    && let Some(&sp) = at.get(k + 1)
                    && !live_after(f, sp as usize, yavs_ir::Reg(a)) =>
            {
                Some(Op::StoreUp { src, depth, w, off })
            }
            (
                Op::Alloc { dst, size },
                Some(&Op::Store {
                    addr,
                    src: a,
                    w: Width::B8,
                }),
                Some(&Op::StoreOff {
                    base,
                    off: 8,
                    src: b,
                    w: Width::B8,
                }),
            ) if int(size) == Some(16) && addr == dst && base == dst && a != dst && b != dst => {
                Some(Op::NewPair { dst, a, b })
            }
            (Op::Alloc { dst, size }, Some(&Op::MakeDope { at, start, len }), _)
                if at == dst && len > 0 && int(size) == i64::from(len).checked_mul(8) =>
            {
                Some(Op::NewDope { at, start, len })
            }
            (Op::Cmp { m, dst: cd, a, b }, Some(&Op::Select { dst, c, a: x, b: y }), _)
                if c == cd && once(cd) =>
            {
                Some(Op::CmpSelect { m, dst, a, b, x, y })
            }
            (
                Op::LoadOff {
                    dst: h,
                    base,
                    off: o1,
                    w: Width::B8,
                },
                Some(&Op::Deref {
                    dst: d1, handle, ..
                }),
                Some(&Op::IndexImm {
                    dst: d2,
                    base: b2,
                    off: o2,
                }),
            ) if handle == h
                && b2 == d1
                && let (Ok(o1), Ok(o2)) = (u16::try_from(o1), u16::try_from(o2)) =>
            {
                Some(Op::LoadDerefImm {
                    base,
                    h,
                    d1,
                    d2,
                    o1,
                    o2,
                })
            }
            (
                Op::Deref {
                    dst: d1, handle, ..
                },
                Some(&Op::IndexImm { dst: d2, base, off }),
                _,
            ) if base == d1 => Some(Op::DerefImm {
                d1,
                handle,
                d2,
                off,
            }),
            (
                Op::Deref {
                    dst: d1,
                    handle,
                    what,
                },
                Some(&Op::LoadOff {
                    dst: d2,
                    base,
                    off,
                    w,
                }),
                _,
            ) if base == d1 => Some(Op::DerefLoadOff {
                w,
                d1,
                handle,
                what,
                d2,
                off,
            }),
            (Op::MoveBr { dst, src, to }, _, _)
                if let Some(&Op::CondBr { c, t, f }) = ops.get(to as usize)
                    && c == dst =>
            {
                Some(Op::MoveCondBr { dst, src, t, f })
            }
            (
                Op::EdgeSig {
                    dst,
                    sig,
                    offset,
                    kind,
                },
                Some(&Op::CondBr { c, t, f }),
                _,
            ) if c == dst => Some(Op::EdgeBr {
                dst,
                kind,
                stmt: false,
                sig,
                offset,
                t,
                f,
            }),
            (Op::WaitSig { sig, offset }, Some(&Op::Suspend), _) => Some(Op::WaitSuspend {
                sig,
                offset,
                at: k as u32,
                stmt: false,
            }),
            (Op::StmtLoadSlot8 { dst, off }, Some(&Op::CondBr { c, t, f }), _) if c == dst => {
                Some(Op::FlagBr { dst, off, t, f })
            }
            (Op::LoadSlot8 { dst, off }, Some(&Op::CondBr { c, t, f }), _) if c == dst => {
                Some(Op::SlotBr { dst, off, t, f })
            }
            (
                Op::LoadSlot8 { dst: d, off },
                Some(&Op::SigVal {
                    dst,
                    sig,
                    offset,
                    len,
                }),
                _,
            ) if offset == d && len != d && dst != d => Some(Op::SigValSlot {
                d,
                off,
                dst,
                sig,
                len,
            }),
            (
                Op::Alloc { dst: tmp, size: sz },
                Some(&Op::Cat),
                Some(&Op::LCopy { dst, src, n, .. }),
            ) if let Some(size) = match int(sz) {
                Some(n) => u32::try_from(n).ok().filter(|n| n & SIZE_REG == 0),
                None => Some(sz | SIZE_REG),
            } && src == tmp
                && dst != tmp
                && let (Some(&cp), Some(&lp)) = (at.get(k + 1), at.get(k + 2))
                && let Some(&Inst::Cat {
                    dst: cd,
                    logic: true,
                    ..
                }) = f.code.get(cp as usize)
                && cd.0 == tmp
                && !live_after(f, lp as usize, yavs_ir::Reg(tmp)) =>
            {
                Some(Op::CatLCopy { tmp, size, dst, n })
            }
            (
                Op::ZeroSlot {
                    w: tmp,
                    off,
                    n: size,
                },
                Some(&Op::Cat),
                Some(&Op::LCopy {
                    dst,
                    doff,
                    src,
                    soff,
                    n,
                }),
            ) if src == tmp
                && doff == soff
                && int(doff) == Some(0)
                && !in_slot(dst, off, size)
                && let (Some(&cp), Some(&lp)) = (at.get(k + 1), at.get(k + 2))
                && let Some(&Inst::Cat {
                    dst: cd,
                    logic: true,
                    ..
                }) = f.code.get(cp as usize)
                && cd.0 == tmp
                && !live_after(f, lp as usize, yavs_ir::Reg(tmp)) =>
            {
                Some(Op::ZeroCatLCopy { tmp, size, dst, n })
            }
            (Op::Stmt, Some(&Op::Alloc { .. }), _)
                if let Some(Op::IsXSlice {
                    tmp,
                    src,
                    soff,
                    n,
                    res,
                }) = is_x_slice(ops, k + 1) =>
            {
                // (A branch on the result follows after the fused `Alloc`, `LCopy`, `CallIeee`.)
                match (
                    ops.get(k + 4),
                    r16(tmp),
                    r16(src),
                    r16(soff),
                    r16(n),
                    r16(res),
                ) {
                    (
                        Some(&Op::CondBr { c, t, f: e }),
                        Some(tmp),
                        Some(src),
                        Some(soff),
                        Some(n),
                        Some(r),
                    ) if c == res => Some(Op::StmtIsXBr {
                        tmp,
                        src,
                        soff,
                        n,
                        res: r,
                        t,
                        f: e,
                    }),
                    _ => Some(Op::StmtIsXSlice {
                        tmp,
                        src,
                        soff,
                        n,
                        res,
                    }),
                }
            }
            _ if let Some(op) = is_x_slice(ops, k) => Some(op),
            (
                first,
                Some(&Op::LCopy {
                    dst: d2,
                    doff,
                    src,
                    soff,
                    n,
                }),
                _,
            ) if let Some((dst, 32, frame)) = block(first)
                && d2 == dst
                && src != dst
                && !matches!(first, Op::ZeroSlot { off, .. } if in_slot(src, off, 32)) =>
            {
                Some(if frame {
                    Op::ZeroLCopy {
                        dst,
                        doff,
                        src,
                        soff,
                        n,
                    }
                } else {
                    Op::AllocLCopy {
                        dst,
                        doff,
                        src,
                        soff,
                        n,
                    }
                })
            }
            (first, Some(&Op::SigReadVec { dst: d2, sig }), _)
                if let Some((dst, 32, frame)) = block(first)
                    && d2 == dst =>
            {
                Some(match first {
                    Op::ZeroSlot { off, .. } if frame => Op::ZeroReadVec { dst, sig, off },
                    _ => Op::AllocReadVec { dst, sig },
                })
            }
            (first, Some(&Op::Cat), _)
                if let Some((dst, 32, frame)) = block(first)
                    && let Some(&cp) = at.get(k + 1)
                    && let Some(&Inst::Cat {
                        dst: cd,
                        logic: true,
                        n,
                        m,
                        parts,
                        scalars,
                        bias,
                        ..
                    }) = f.code.get(cp as usize)
                    && cd.0 == dst
                    && (frame || u8::try_from(parts.len).is_ok()) =>
            {
                Some(if frame {
                    Op::ZeroCat { dst }
                } else {
                    Op::AllocCat {
                        dst,
                        n: n.0,
                        m: m.0,
                        parts: parts.start,
                        len: parts.len as u8,
                        scalars,
                        bias,
                    }
                })
            }
            (Op::Alloc { dst, size }, _, _)
                if let Some(n) = int(size).and_then(|n| u32::try_from(n).ok()) =>
            {
                Some(Op::AllocK { dst, n })
            }
            (
                Op::CmpBr {
                    m,
                    dst: cd,
                    a,
                    b,
                    t,
                    f: e,
                },
                Some(&Op::Bin {
                    op: bop @ (IntOp::Add | IntOp::Sub),
                    dst,
                    a: src,
                    b: step,
                }),
                Some(&Op::Br { to: head }),
            ) if e as usize == k + 1
                && once(cd)
                && let Some(v) = int(step)
                && let Some(delta) = i16::try_from(if bop == IntOp::Add { v } else { -v }).ok()
                && let (Some(a), Some(b), Some(dst), Some(src)) =
                    (r16(a), r16(b), r16(dst), r16(src)) =>
            {
                Some(Op::CmpBrStep {
                    m,
                    delta,
                    a,
                    b,
                    dst,
                    src,
                    t,
                    head,
                })
            }
            (
                Op::PosIndexLoad {
                    dst,
                    dope,
                    idx,
                    base,
                    stride,
                    w,
                    dim,
                },
                Some(&Op::CmpBr {
                    m,
                    dst: cd,
                    a,
                    b,
                    t,
                    f: e,
                }),
                _,
            ) if once(cd) && (a == dst || b == dst) => {
                let (m, other) = if a == dst {
                    (m, b)
                } else {
                    // Swap the operands: lt and gt change places.
                    ((m & 0b010) | (m & 1) << 2 | (m >> 2 & 1), a)
                };
                match (
                    r16(dst),
                    r16(dope),
                    r16(idx),
                    r16(base),
                    r16(other),
                    u16::try_from(stride).ok(),
                ) {
                    (Some(dst), Some(dope), Some(idx), Some(base), Some(b), Some(stride)) => {
                        Some(Op::PosIndexLoadCmpBr {
                            m,
                            w,
                            dim,
                            dst,
                            dope,
                            idx,
                            base,
                            b,
                            stride,
                            t,
                            f: e,
                        })
                    }
                    _ => None,
                }
            }
            _ => None,
        };
        if let Some(op) = fused {
            ops[k] = op;
        }
    }
    // `'image` and the reads of its string result.
    for k in 0..ops.len() {
        if let (
            Op::Image { dst: x, a, ty },
            Some(&Op::Load {
                dst: p,
                addr,
                w: Width::B8,
            }),
            Some(&Op::LoadOff {
                dst: q,
                base,
                off: 8,
                w: Width::B8,
            }),
            Some(&Op::DopeLen {
                dst: l,
                dope,
                dim: 0,
            }),
        ) = (ops[k], ops.get(k + 1), ops.get(k + 2), ops.get(k + 3))
            && addr == x
            && base == x
            && p != x
            && dope == q
            && let (Some(x), Some(p), Some(q), Some(l)) = (r16(x), r16(p), r16(q), r16(l))
        {
            ops[k] = Op::ImageStr { a, ty, x, p, q, l };
        }
    }
    // `wait until <edge>`: the edge test of the element the wait names, the wait and the suspend (#767).
    for k in 0..ops.len().saturating_sub(3) {
        if let (
            Op::SigVal {
                dst,
                sig,
                offset: eo,
                len,
            },
            Op::WaitEdge { sig: er, kind },
            Op::WaitSuspend {
                sig: ws,
                offset: wo,
                stmt: false,
                ..
            },
            Op::Suspend,
        ) = (ops[k], ops[k + 1], ops[k + 2], ops[k + 3])
            && er == dst
            && once(dst)
            && sig == ws
            && int(len) == Some(1)
            && (eo == wo || int(eo).is_some() && int(eo) == int(wo))
        {
            ops[k] = Op::WaitEdgeSuspend {
                sig,
                offset: wo,
                kind,
                at: k as u32,
                stmt: false,
                body: NONE,
            };
        }
    }
    // A `Stmt` before the edge test or the wait of a clocked process is part of that op, and a
    // jump to such a wait is a copy of it (the end of every activation).
    for k in 0..ops.len() {
        let fused = match (ops[k], ops.get(k + 1)) {
            (
                Op::Stmt,
                Some(&Op::EdgeBr {
                    dst,
                    kind,
                    stmt: false,
                    sig,
                    offset,
                    t,
                    f,
                }),
            ) => Some(Op::EdgeBr {
                dst,
                kind,
                stmt: true,
                sig,
                offset,
                t,
                f,
            }),
            (
                Op::Stmt,
                Some(&Op::WaitSuspend {
                    sig,
                    offset,
                    stmt: false,
                    ..
                }),
            ) => Some(Op::WaitSuspend {
                sig,
                offset,
                at: k as u32,
                stmt: true,
            }),
            (
                Op::Stmt,
                Some(&Op::WaitEdgeSuspend {
                    sig,
                    offset,
                    kind,
                    stmt: false,
                    body,
                    ..
                }),
            ) => Some(Op::WaitEdgeSuspend {
                sig,
                offset,
                kind,
                at: k as u32,
                stmt: true,
                body,
            }),
            _ => None,
        };
        if let Some(op) = fused {
            ops[k] = op;
        }
    }
    for k in 0..ops.len() {
        match ops[k] {
            Op::Br { to } => {
                if let Some(&w @ (Op::WaitSuspend { .. } | Op::WaitEdgeSuspend { .. })) =
                    ops.get(to as usize)
                {
                    ops[k] = w;
                }
            }
            // The jump to the block a process pauses at keeps its check in the copy.
            Op::BrBody { to, block } => {
                if let Some(&Op::WaitEdgeSuspend {
                    sig,
                    offset,
                    kind,
                    at,
                    stmt,
                    ..
                }) = ops.get(to as usize)
                {
                    ops[k] = Op::WaitEdgeSuspend {
                        sig,
                        offset,
                        kind,
                        at,
                        stmt,
                        body: block,
                    };
                }
            }
            _ => {}
        }
    }
    // The search loop: a load and compare of a position, then a step of the index that jumps
    // back, with nothing else in the loop.
    for k in 0..ops.len() {
        if let Op::PosIndexLoadCmpBr {
            m,
            w,
            dim,
            dst,
            dope,
            idx,
            base,
            b,
            stride,
            t,
            f,
        } = ops[k]
            && let Some(&Op::CmpBrStep {
                a: sa,
                b: sb,
                dst: sd,
                src,
                head,
                ..
            }) = ops.get(f as usize)
            && head as usize == k
            && sd == idx
            && src == idx
            && dim & SCAN == 0
            && [dope, idx, base, b, sa, sb].iter().all(|&r| r != dst)
            && b != idx
            && (sa == idx) != (sb == idx)
        {
            ops[k] = Op::PosIndexLoadCmpBr {
                m,
                w,
                dim: dim | SCAN,
                dst,
                dope,
                idx,
                base,
                b,
                stride,
                t,
                f,
            };
        }
    }
}

impl Code {
    fn new(
        f: &Func,
        prog: &Program,
        skip: &[bool],
        known: &[Known],
        next: &[u32],
        budget: bool,
    ) -> Code {
        let n = f.code.len();
        // A block start stops a fusion only where something can jump to it: a block that
        // only a skipped jump (to where execution goes on anyway) or nothing reaches is run
        // through, and what the optimiser left split by it fuses as one block (#534).
        let mut refd = vec![false; f.blocks.len()];
        let mut mark = |b: yavs_ir::BlockId| {
            if let Some(r) = refd.get_mut(b.0 as usize) {
                *r = true;
            }
        };
        let mut all = false;
        mark(yavs_ir::BlockId(0));
        f.body.map(&mut mark);
        for (pc, i) in f.code.iter().enumerate() {
            match *i {
                _ if skip[pc] => {}
                Inst::Br { to } => mark(to),
                Inst::CondBr { t, f, .. } => {
                    mark(t);
                    mark(f);
                }
                Inst::Case { table, .. } => match prog.cases.get(table) {
                    Some(t) => {
                        mark(t.default);
                        t.arms.iter().for_each(|a| mark(a.2));
                    }
                    None => all = true,
                },
                _ => {}
            }
        }
        let mut block_start = vec![false; n + 1];
        for (b, blk) in f.blocks.iter() {
            if let Some(s) = block_start.get_mut(blk.start as usize) {
                *s |= all || refd[b.0 as usize];
            }
        }
        for &pc in &f.resume {
            if let Some(s) = block_start.get_mut(pc as usize) {
                *s = true;
            }
        }
        // (Lists through the instructions that name them: the optimiser leaves the lists of
        // removed instructions in `operands`, #752.)
        let uses = read_counts(f);
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
        let mut defs = vec![0u8; f.reg_ty.len()];
        for inst in &f.code {
            inst.visit(&mut |r, role, _| {
                if role == Role::Def
                    && let Some(d) = defs.get_mut(r.0 as usize)
                {
                    *d = d.saturating_add(1);
                }
            });
        }
        let mut aff = Vec::new();
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
                Inst::Stmt => {
                    // Statements in a row end one set of temporaries once.
                    let mut p = pc;
                    while let Some((n2, Inst::Stmt)) = after(p) {
                        p = n2;
                    }
                    last = p;
                    match after(p) {
                        Some((
                            n2,
                            Inst::Load {
                                dst,
                                addr,
                                w: Width::B8,
                            },
                        )) if let Some(&Known::Frame(off)) = known.get(addr.0 as usize) => {
                            last = n2;
                            Op::StmtLoadSlot8 { dst: dst.0, off }
                        }
                        _ => Op::Stmt,
                    }
                }
                Inst::Bin {
                    op: IntOp::Add | IntOp::Sub | IntOp::Mul,
                    ..
                }
                | Inst::Pos { dim: 0, .. }
                | Inst::DopeLen { dim: 0, .. }
                    if let Some(LinRun { head, tail, end }) = lin_run(
                        &LinCx {
                            f,
                            skip,
                            block_start: &block_start,
                            known,
                            defs: &defs,
                            uses: &uses,
                        },
                        pc,
                    ) =>
                {
                    last = end;
                    // The ops of the run share its first pc (compiled code entering at one
                    // starts at the run).
                    for (o, root) in head {
                        aff.push((ops.len() as u32, pc as u32, root));
                        ops.push(o);
                        at.push(pc as u32);
                    }
                    aff.push((ops.len() as u32, pc as u32, tail.1));
                    tail.0
                }
                Inst::Cover { item } => Op::Cover { item: item.0 },
                Inst::CoverIn {
                    dst,
                    acc,
                    src,
                    shift,
                    ulogic,
                } => Op::CoverIn {
                    dst: dst.0,
                    acc: acc.0,
                    src: src.0,
                    shift,
                    ulogic,
                },
                Inst::CoverCond { cond, state } => Op::CoverCond {
                    cond,
                    state: state.0,
                },
                Inst::Move { dst, src } => match fusable {
                    Some(Inst::Br { to }) if target(to).is_some() => {
                        last = nx;
                        Op::MoveBr {
                            dst: dst.0,
                            src: src.0,
                            to: target(to).unwrap_or(0),
                        }
                    }
                    _ => Op::Move {
                        dst: dst.0,
                        src: src.0,
                    },
                },
                // A signal reference in the frame (a memoised `SigVal`): its two words.
                Inst::Load {
                    dst,
                    addr,
                    w: Width::Sig,
                } if let Some(&Known::Frame(off)) = known.get(addr.0 as usize) => Op::LoadSlot8x2 {
                    d1: dst.0,
                    o1: off,
                    d2: dst.0 + 1,
                    o2: off + 8,
                },
                Inst::Load { dst, addr, w } if w != Width::Sig => {
                    match known.get(addr.0 as usize) {
                        Some(&Known::Frame(off)) if w == Width::B8 => {
                            // A load and the read of the signal element it indexes (the loaded
                            // register is read by nothing else).
                            let slot_read = match (fusable, fusable.and_then(|_| after(nx))) {
                                (
                                    Some(Inst::SigVal {
                                        dst: sv,
                                        sig,
                                        offset,
                                        len,
                                    }),
                                    Some((n2, Inst::SigReadScalar { dst: d, sig: r })),
                                ) if offset == dst
                                    && r == sv
                                    && int(len) == Some(1)
                                    && once(sv)
                                    && (d == dst || !live_after(f, n2, dst)) =>
                                {
                                    Some((n2, Op::SigRead1Slot { dst: d.0, sig, off }))
                                }
                                _ => None,
                            };
                            if let Some((n2, op)) = slot_read {
                                last = n2;
                                op
                            } else {
                                // Two loads, or a load and the branch on its compare.
                                let cmp_br = match fusable.and_then(|_| after(nx)) {
                                    Some((n2, Inst::CondBr { c, t, f: e }))
                                        if let Some(Inst::Cmp { op, dst: cd, a, b }) = fusable
                                            && c == cd
                                            && once(cd)
                                            && (a == dst || b == dst)
                                            && target(t).is_some()
                                            && target(e).is_some() =>
                                    {
                                        let (side, other) = if a == dst { (0, b) } else { (1, a) };
                                        Some((
                                            n2,
                                            Op::LoadSlot8CmpBr {
                                                ld: dst.0,
                                                off,
                                                other: other.0,
                                                t: target(t).unwrap_or(0),
                                                f: target(e).unwrap_or(0),
                                                m: cmp_mask(op),
                                                side,
                                            },
                                        ))
                                    }
                                    _ => None,
                                };
                                match (cmp_br, fusable) {
                                    (Some((n2, op)), _) => {
                                        last = n2;
                                        op
                                    }
                                    (
                                        None,
                                        Some(Inst::Load {
                                            dst: d2,
                                            addr: a2,
                                            w: Width::B8,
                                        }),
                                    ) if let Some(&Known::Frame(o2)) = known.get(a2.0 as usize) => {
                                        last = nx;
                                        Op::LoadSlot8x2 {
                                            d1: dst.0,
                                            o1: off,
                                            d2: d2.0,
                                            o2,
                                        }
                                    }
                                    _ => Op::LoadSlot8 { dst: dst.0, off },
                                }
                            }
                        }
                        Some(&Known::Frame(off)) => Op::LoadSlot { dst: dst.0, off, w },
                        Some(&Known::Inst(off)) => Op::LoadInst { dst: dst.0, off, w },
                        Some(&Known::Pkg(pkg, off)) => Op::LoadPkg {
                            dst: dst.0,
                            pkg,
                            off,
                            w,
                        },
                        _ => Op::Load {
                            dst: dst.0,
                            addr: addr.0,
                            w,
                        },
                    }
                }
                Inst::Store { addr, src, w } if w != Width::Sig => {
                    match known.get(addr.0 as usize) {
                        Some(&Known::Frame(off)) if matches!(fusable, Some(Inst::Stmt)) => {
                            last = nx;
                            Op::StoreSlotStmt { src: src.0, off, w }
                        }
                        Some(&Known::Frame(off)) if matches!(fusable, Some(Inst::Br { to }) if target(to).is_some()) =>
                        {
                            last = nx;
                            let to = match fusable {
                                Some(Inst::Br { to }) => target(to).unwrap_or(0),
                                _ => 0,
                            };
                            Op::StoreSlotBr {
                                src: src.0,
                                off,
                                to,
                                w,
                            }
                        }
                        Some(&Known::Frame(off)) => {
                            // An `Index` by a constant after it, unless a load or store at that
                            // address makes it one op of its own.
                            let imm = match fusable {
                                Some(Inst::Index {
                                    dst,
                                    base,
                                    pos,
                                    stride,
                                }) => int(pos)
                                    .and_then(|p| u64::try_from(p).ok())
                                    .and_then(|p| p.checked_mul(u64::from(stride)))
                                    .and_then(|o| u32::try_from(o).ok())
                                    .filter(|_| {
                                        !matches!(
                                            after(nx),
                                            Some((_, Inst::Load { addr, .. } | Inst::Store { addr, .. }))
                                                if addr == dst
                                        )
                                    })
                                    .map(|o| (dst, base, o)),
                                _ => None,
                            };
                            match imm {
                                Some((d, b, o)) => {
                                    last = nx;
                                    Op::StoreSlotIndexImm {
                                        src: src.0,
                                        off,
                                        w,
                                        dst: d.0,
                                        base: b.0,
                                        add: o,
                                    }
                                }
                                None => Op::StoreSlot { src: src.0, off, w },
                            }
                        }
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
                        (None, Some(Inst::Load { dst: d, addr, w }))
                            if once(dst) && addr == dst && w != Width::Sig =>
                        {
                            last = nx;
                            Op::IndexLoad {
                                dst: d.0,
                                base: base.0,
                                pos: pos.0,
                                stride,
                                w,
                            }
                        }
                        (None, Some(Inst::Store { addr, src, w }))
                            if once(dst) && addr == dst && src != dst && w != Width::Sig =>
                        {
                            last = nx;
                            Op::IndexStore {
                                src: src.0,
                                base: base.0,
                                pos: pos.0,
                                stride,
                                w,
                            }
                        }
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
                } => {
                    // The `Index` by this position (nothing else reads it), then maybe the
                    // load or store at that address (nothing else reads it either).
                    let fused = match fusable {
                        Some(Inst::Index {
                            dst: a,
                            base,
                            pos,
                            stride,
                        }) if pos == dst && once(dst) && int(pos).is_none() => match after(nx) {
                            Some((n2, Inst::Load { dst: d, addr, w }))
                                if addr == a && once(a) && w != Width::Sig =>
                            {
                                Some((
                                    n2,
                                    Op::PosIndexLoad {
                                        dst: d.0,
                                        dope: dope.0,
                                        idx: idx.0,
                                        base: base.0,
                                        stride,
                                        w,
                                        dim,
                                    },
                                ))
                            }
                            Some((n2, Inst::Store { addr, src, w }))
                                if addr == a && once(a) && src != a && w != Width::Sig =>
                            {
                                Some((
                                    n2,
                                    Op::PosIndexStore {
                                        src: src.0,
                                        dope: dope.0,
                                        idx: idx.0,
                                        base: base.0,
                                        stride,
                                        w,
                                        dim,
                                    },
                                ))
                            }
                            _ => Some((
                                nx,
                                Op::PosIndex {
                                    dst: a.0,
                                    dope: dope.0,
                                    idx: idx.0,
                                    base: base.0,
                                    stride,
                                    dim,
                                },
                            )),
                        },
                        _ => None,
                    };
                    match fused {
                        Some((p, op)) => {
                            last = p;
                            op
                        }
                        None => Op::Pos {
                            dst: dst.0,
                            dope: dope.0,
                            idx: idx.0,
                            what,
                            dim,
                        },
                    }
                }
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
                } => match (int(lo).map(i32::try_from), int(hi).map(i32::try_from)) {
                    (Some(Ok(l)), Some(Ok(h))) => Op::BinRangeK {
                        op,
                        dst: dst.0,
                        a: a.0,
                        b: b.0,
                        lo: l,
                        hi: h,
                    },
                    _ => Op::BinRange {
                        op,
                        dst: dst.0,
                        a: a.0,
                        b: b.0,
                        lo: lo.0,
                        hi: hi.0,
                    },
                },
                Inst::Cmp { op, dst, a, b } => match fusable {
                    // `Cmp`, `Not` of it, `CondBr` on that: the compare with the opposite mask.
                    Some(Inst::Bool {
                        op: BoolOp::Not,
                        dst: nd,
                        a: na,
                        ..
                    }) if na == dst
                        && once(dst)
                        && let Some((n2, Inst::CondBr { c, t, f: e })) = after(nx)
                        && c == nd
                        && target(t).is_some()
                        && target(e).is_some() =>
                    {
                        last = n2;
                        Op::CmpBr {
                            m: !cmp_mask(op) & 0b111,
                            dst: nd.0,
                            a: a.0,
                            b: b.0,
                            t: target(t).unwrap_or(0),
                            f: target(e).unwrap_or(0),
                        }
                    }
                    Some(Inst::CondBr { c, t, f: e })
                        if c == dst && target(t).is_some() && target(e).is_some() =>
                    {
                        last = nx;
                        Op::CmpBr {
                            m: cmp_mask(op),
                            dst: dst.0,
                            a: a.0,
                            b: b.0,
                            t: target(t).unwrap_or(0),
                            f: target(e).unwrap_or(0),
                        }
                    }
                    _ => Op::Cmp {
                        m: cmp_mask(op),
                        dst: dst.0,
                        a: a.0,
                        b: b.0,
                    },
                },
                Inst::Bool { op, dst, a, b } => Op::Bool {
                    op,
                    dst: dst.0,
                    a: a.0,
                    b: b.0,
                },
                Inst::LCopy {
                    dst,
                    dst_off,
                    src,
                    src_off,
                    n,
                } => Op::LCopy {
                    dst: dst.0,
                    doff: dst_off.0,
                    src: src.0,
                    soff: src_off.0,
                    n: n.0,
                },
                Inst::LEq { dst, a, b, n } => Op::LEq {
                    dst: dst.0,
                    a: a.0,
                    b: b.0,
                    n: n.0,
                },
                Inst::Cat { .. } => Op::Cat,
                Inst::Select { dst, c, a, b } => Op::Select {
                    dst: dst.0,
                    c: c.0,
                    a: a.0,
                    b: b.0,
                },
                Inst::SigRead { dst, sig } => Op::SigReadVec {
                    dst: dst.0,
                    sig: sig.0,
                },
                Inst::Check {
                    kind,
                    v,
                    lo,
                    hi,
                    what,
                } => match (kind, int(lo).map(i32::try_from), int(hi).map(i32::try_from)) {
                    (CheckKind::Range | CheckKind::Index, Some(Ok(lo)), Some(Ok(hi))) => {
                        Op::CheckK {
                            kind,
                            v: v.0,
                            lo,
                            hi,
                            what,
                        }
                    }
                    _ => Op::Check {
                        kind,
                        v: v.0,
                        lo: lo.0,
                        hi: hi.0,
                        what,
                    },
                },
                Inst::LayoutOf { dst, ty, desc, .. } => {
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
                } => {
                    // One element that only the next instruction reads.
                    let one = int(len) == Some(1) && once(dst);
                    let fused = match fusable {
                        Some(Inst::SigReadScalar { dst: d, sig: r }) if one && r == dst => {
                            Some(Op::SigRead1 {
                                dst: d.0,
                                sig,
                                offset: offset.0,
                            })
                        }
                        Some(Inst::WaitOn { sig: r }) if one && r == dst => Some(Op::WaitSig {
                            sig,
                            offset: offset.0,
                        }),
                        Some(Inst::Edge {
                            dst: d,
                            kind,
                            sig: r,
                        }) if one && r == dst => Some(Op::EdgeSig {
                            dst: d.0,
                            sig,
                            offset: offset.0,
                            kind,
                        }),
                        Some(Inst::CallNative {
                            native,
                            args,
                            dst: d,
                        }) if one => match (native_op(prog, f, native, args), d) {
                            (Some(NativeOp::Edge { kind, sig: r }), Some(d)) if r == dst => {
                                Some(Op::EdgeSig {
                                    dst: d.0,
                                    sig,
                                    offset: offset.0,
                                    kind,
                                })
                            }
                            _ => None,
                        },
                        _ => None,
                    };
                    match fused {
                        Some(op) => {
                            last = nx;
                            op
                        }
                        None => Op::SigVal {
                            dst: dst.0,
                            sig,
                            offset: offset.0,
                            len: len.0,
                        },
                    }
                }
                Inst::SlicePos { .. } => Op::SlicePos(pc as u32),
                Inst::DopeLen { dst, dope, dim } => Op::DopeLen {
                    dst: dst.0,
                    dope: dope.0,
                    dim,
                },
                Inst::Call { func, args, dst } => {
                    let (func, start, len, dst) =
                        (func.0, args.start, args.len, dst.map_or(NONE, |d| d.0));
                    if prog.memo_sig(yavs_ir::FuncId(func)).is_some() {
                        Op::CallM {
                            func,
                            start,
                            len,
                            dst,
                        }
                    } else {
                        Op::Call {
                            func,
                            start,
                            len,
                            dst,
                        }
                    }
                }
                Inst::CallProt {
                    func,
                    obj,
                    args,
                    dst,
                } => Op::CallProt {
                    func: func.0,
                    obj: obj.0,
                    start: args.start,
                    len: args.len,
                    dst: dst.map_or(NONE, |d| d.0),
                },
                Inst::CallNative { native, args, dst } => {
                    match (native_op(prog, f, native, args), dst) {
                        (_, Some(d)) if let Some((a, ty)) = scalar_image(prog, f, native, args) => {
                            Op::Image {
                                dst: d.0,
                                a: a.0,
                                ty: ty.0,
                            }
                        }
                        (Some(NativeOp::Gate { g, a, b }), Some(d)) => Op::Gate {
                            g,
                            dst: d.0,
                            a: a.0,
                            b: b.0,
                        },
                        (Some(NativeOp::Not { a }), Some(d)) => Op::NotL { dst: d.0, a: a.0 },
                        (Some(NativeOp::Uniform { s1, s2, x }), None) => Op::Uniform {
                            s1: s1.0,
                            s2: s2.0,
                            x: x.0,
                            native: native.0,
                            start: args.start,
                        },
                        _ if let Some(op) = call_ieee(prog, f, native, args, dst) => op,
                        _ => Op::CallNative {
                            native: native.0,
                            start: args.start,
                            len: args.len,
                            dst: dst.map_or(NONE, |d| d.0),
                        },
                    }
                }
                Inst::SigReadScalar { dst, sig } => Op::SigReadScalar {
                    dst: dst.0,
                    sig: sig.0,
                },
                Inst::WaitOn { sig } => Op::WaitOn { sig: sig.0 },
                Inst::WaitEdge { sig, kind } => Op::WaitEdge {
                    sig: sig.0,
                    kind: kind.code(),
                },
                Inst::Drive {
                    driver,
                    target: Some(target),
                    wave,
                    n,
                    mode,
                    reject,
                } => Op::Drive {
                    driver,
                    target: target.0,
                    wave: wave.0,
                    n: n.0,
                    reject: reject.map_or(NONE, |r| r.0),
                    inertial: mode == yavs_ir::DriveMode::Inertial,
                },
                Inst::TempRoot { addr, ty: None, .. } => Op::FatRoot { addr: addr.0 },
                Inst::NewFrame { .. } => Op::NewFrame(pc as u32),
                Inst::MemEq { dst, a, b, n } => Op::MemEq {
                    dst: dst.0,
                    a: a.0,
                    b: b.0,
                    n: n.0,
                },
                Inst::Suspend { .. } => Op::Suspend,
                Inst::Ret { v } => Op::Ret {
                    v: v.map_or(NONE, |r| r.0),
                },
                Inst::Alloc { dst, size } => Op::Alloc {
                    dst: dst.0,
                    size: size.0,
                },
                Inst::Zero { addr, bytes } => match known.get(addr.0 as usize) {
                    Some(&Known::Frame(off)) => Op::ZeroSlot {
                        w: addr.0,
                        off,
                        n: bytes,
                    },
                    _ => Op::Step(pc as u32),
                },
                Inst::Const { dst, v } => Op::Const { dst: dst.0, v },
                Inst::FCmp { op, dst, a, b } => Op::FCmp {
                    op,
                    dst: dst.0,
                    a: a.0,
                    b: b.0,
                },
                // The other spaces are numbers (an address the machine cannot make stays a
                // step, which reports it).
                Inst::AddrOf { dst, space, off } => match space {
                    AddrSpace::Frame => Op::AddrFrame { dst: dst.0, off },
                    AddrSpace::Up(depth) => Op::AddrUp {
                        dst: dst.0,
                        depth,
                        off,
                    },
                    AddrSpace::Inst => const_addr(dst, INST, 0, off, pc),
                    AddrSpace::Pkg(p) => const_addr(dst, PKG, p, off, pc),
                    AddrSpace::Const(c) => const_addr(dst, CONST, c.0, off, pc),
                    AddrSpace::Shared => const_addr(dst, SHARED, 0, off, pc),
                },
                Inst::ConstF { dst, v } => Op::Const {
                    dst: dst.0,
                    v: v.to_bits() as i64,
                },
                Inst::SubtypeCheck {
                    dope,
                    want,
                    dims,
                    what,
                } => Op::SubtypeCheck {
                    dope: dope.0,
                    want: want.0,
                    what,
                    dims,
                },
                Inst::Copy { dst, src, bytes } => Op::Copy {
                    dst: dst.0,
                    src: src.0,
                    bytes: bytes.0,
                },
                Inst::CopyConst { dst, c } => Op::CopyConst { dst: dst.0, c: c.0 },
                Inst::MakeDope { at, parts } => Op::MakeDope {
                    at: at.0,
                    start: parts.start,
                    len: parts.len,
                },
                Inst::Deref { dst, handle, what } => Op::Deref {
                    dst: dst.0,
                    handle: handle.0,
                    what,
                },
                Inst::Field {
                    dst,
                    base,
                    layout,
                    n,
                } => Op::Field {
                    dst: dst.0,
                    base: base.0,
                    layout: layout.0,
                    n,
                },
                Inst::LGet { dst, base, idx } => Op::LGet {
                    dst: dst.0,
                    base: base.0,
                    idx: idx.0,
                },
                Inst::LSet { base, idx, v } => Op::LSet {
                    base: base.0,
                    idx: idx.0,
                    v: v.0,
                },
                Inst::CondBr { c, t, f: e } => match (target(t), target(e)) {
                    (Some(t), Some(e)) => Op::CondBr { c: c.0, t, f: e },
                    _ => Op::Step(pc as u32),
                },
                Inst::Case { v, table } => Op::Case {
                    v: v.0,
                    tab: table.0,
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
        let mut no_tier = Vec::new();
        peephole(
            f,
            &mut ops,
            &mut at,
            &mut start,
            &mut aff,
            &mut no_tier,
            &block_start,
            &uses,
            known,
        );
        // Disjoint, in order: a search by either end finds the range of an op.
        no_tier.sort_unstable();
        let mut merged: Vec<(u32, u32)> = Vec::new();
        for r in no_tier {
            match merged.last_mut() {
                Some(m) if r.0 <= m.1 => m.1 = m.1.max(r.1),
                _ => merged.push(r),
            }
        }
        let no_tier = merged;
        let entry: Vec<u32> = next
            .iter()
            .map(|&p| start.get(p as usize).copied().unwrap_or(NONE))
            .collect();
        // Jump targets as op indices. A target is the first executed instruction of a
        // block, never inside a fused op; should one be, a jump is left to the step path and
        // a fused compare is an internal error (`Step(NONE)` runs off the code).
        let to = |p: u32| entry.get(p as usize).copied().filter(|&k| k != NONE);
        let mut cases = Vec::new();
        for (k, op) in ops.iter_mut().enumerate() {
            let ir = at[k];
            *op = match *op {
                Op::Case { v, tab } => {
                    let go = |b| target(b).and_then(to);
                    let t = prog.cases.get(yavs_ir::CaseTableId(tab));
                    let arms: Option<Vec<_>> = t.and_then(|t| {
                        t.arms
                            .iter()
                            .map(|&(lo, hi, b)| Some((lo, hi, go(b)?)))
                            .collect()
                    });
                    match (arms, t.and_then(|t| go(t.default))) {
                        (Some(arms), Some(default)) => {
                            cases.push(CaseOps { arms, default });
                            Op::Case {
                                v,
                                tab: cases.len() as u32 - 1,
                            }
                        }
                        _ => Op::Step(ir),
                    }
                }
                Op::Br { to: t } => to(t).map_or(Op::Step(ir), |t| Op::Br { to: t }),
                Op::StoreSlotBr { src, off, to: t, w } => {
                    to(t).map_or(Op::Step(NONE), |t| Op::StoreSlotBr { src, off, to: t, w })
                }
                Op::MoveBr { dst, src, to: t } => {
                    to(t).map_or(Op::Step(NONE), |t| Op::MoveBr { dst, src, to: t })
                }
                Op::LoadSlot8CmpBr {
                    ld,
                    off,
                    other,
                    t,
                    f: e,
                    m,
                    side,
                } => match (to(t), to(e)) {
                    (Some(t), Some(e)) => Op::LoadSlot8CmpBr {
                        ld,
                        off,
                        other,
                        t,
                        f: e,
                        m,
                        side,
                    },
                    _ => Op::Step(NONE),
                },
                Op::BrBody { to: t, block } => {
                    to(t).map_or(Op::Step(ir), |t| Op::BrBody { to: t, block })
                }
                Op::CondBr { c, t, f: e } => match (to(t), to(e)) {
                    (Some(t), Some(e)) => Op::CondBr { c, t, f: e },
                    _ => Op::Step(ir),
                },
                Op::CmpBr {
                    m,
                    dst,
                    a,
                    b,
                    t,
                    f: e,
                } => match (to(t), to(e)) {
                    (Some(t), Some(e)) => Op::CmpBr {
                        m,
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
        superops(prog, &mut ops, &at, f, known, &uses);
        static_waits(prog, f, &mut ops, &at, &entry);
        let runs = drive_runs(&mut ops);
        let loops = elem_loops(&mut ops, &mut at);
        word_pairs(&mut ops);
        // A process that never suspends polls for a stop request at every jump back: the jump
        // is stepped (`Machine::step_out`), so no other function pays for the check.
        if f.kind == yavs_ir::FuncKind::Process && !f.flags.contains(FuncFlags::MAY_SUSPEND) {
            for (k, op) in ops.iter_mut().enumerate() {
                if let Op::Br { to } | Op::BrBody { to, .. } = *op
                    && (to as usize) <= k
                {
                    *op = Op::Step(at[k]);
                }
            }
        }
        if budget {
            count_back_edges(prog, f, &entry, &mut ops, &mut at);
        }
        let first = entry.first().copied().unwrap_or(NONE);
        // Debug aid next to the IR dump of `YAVS_DUMPIR`.
        if std::env::var_os("YAVS_DUMPIR").is_some() {
            eprintln!("ops of `{}`", f.name);
            for (i, o) in ops.iter().enumerate() {
                eprintln!("{i:6} pc{} {o:?}", at[i]);
            }
        }
        Code {
            ops,
            at,
            entry,
            first,
            aff,
            no_tier,
            cases,
            loops,
            trace: std::env::var_os("YAVS_DUMPIR").is_some(),
            skip: NONE,
            runs,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yavs_diag::{FileId, Span};
    use yavs_ir::{CmpOp, FuncKind, Mode, ParamClass, ProgramBuilder, RegTy, StmtKind};

    /// `depth(n) = if n = 0 then 0 else depth(n - 1) + 1` (one 8-byte `in` parameter, so
    /// `lean`); its first instruction takes the address of the parameter's slot.
    fn depth() -> (Program, yavs_ir::FuncId) {
        depth_with(false, false)
    }

    /// `depth` with a slot nothing addresses (`spare`), or one the body takes the address of.
    fn depth_with(spare: bool, used: bool) -> (Program, yavs_ir::FuncId) {
        let mut pb = ProgramBuilder::new();
        let depth = pb.declare("depth", FuncKind::Function, None);
        let mut fb = pb.define(depth);
        let s = fb.param("n", Width::B8, ParamClass::Value, Mode::In, None);
        fb.stmt(StmtKind::Return, Span::new(FileId(0), 0, 1));
        let extra = (spare || used).then(|| fb.slot(16, 8));
        let extra_at = extra.filter(|_| used).map(|e| fb.addr(AddrSpace::Frame, e));
        let at = fb.addr(AddrSpace::Frame, s);
        let n = fb.reg(RegTy::I64);
        fb.emit(Inst::Load {
            dst: n,
            addr: at,
            w: Width::B8,
        });
        if let Some(at) = extra_at {
            let x = fb.reg(RegTy::I64);
            fb.emit(Inst::Store {
                addr: at,
                src: n,
                w: Width::B8,
            });
            fb.emit(Inst::Load {
                dst: x,
                addr: at,
                w: Width::B8,
            });
            fb.emit(Inst::Bin {
                op: yavs_ir::IntOp::Add,
                dst: n,
                a: n,
                b: x,
            });
        }
        let (base, rec) = (fb.block(), fb.block());
        let (zero, one, c) = (fb.int(0), fb.int(1), fb.reg(RegTy::I64));
        // (Computed before the test: the parameter load then stays an op of its own, which the
        // compare and branch would otherwise take over now that nothing here needs a marker.)
        let m = fb.reg(RegTy::I64);
        fb.emit(Inst::Bin {
            op: yavs_ir::IntOp::Sub,
            dst: m,
            a: n,
            b: one,
        });
        fb.emit(Inst::Cmp {
            op: CmpOp::Eq,
            dst: c,
            a: n,
            b: zero,
        });
        fb.emit(Inst::CondBr { c, t: base, f: rec });
        fb.at(base);
        fb.emit(Inst::Ret { v: Some(zero) });
        fb.at(rec);
        let args = fb.regs(&[m]);
        let d = fb.reg(RegTy::I64);
        fb.emit(Inst::Call {
            func: depth,
            args,
            dst: Some(d),
        });
        fb.emit(Inst::Bin {
            op: yavs_ir::IntOp::Add,
            dst: d,
            a: d,
            b: one,
        });
        fb.emit(Inst::Ret { v: Some(d) });
        fb.finish();
        (pb.finish().unwrap(), depth)
    }

    /// A lean call allocates the slots the code addresses besides the parameters: none for a
    /// frame the code never addresses (#730).
    #[test]
    fn lean_frame_has_slots_only_when_addressed() {
        for (spare, used) in [(false, false), (true, false), (false, true)] {
            let (prog, id) = depth_with(spare, used);
            let f = prog.func(id).unwrap();
            let pre = Pre::new(f, &prog, false);
            assert!(pre.lean);
            let addressed = f.code.iter().any(
                |i| matches!(*i, Inst::AddrOf { space: AddrSpace::Frame, off, .. } if off != 0),
            );
            assert_eq!(
                pre.fslots,
                if addressed { f.frame } else { 0 },
                "{spare} {used}"
            );
            if spare && !used {
                assert!(
                    f.frame > 8 && pre.fslots == 0,
                    "an unused slot is not allocated"
                );
            }
        }
    }

    /// A function with an `inout` scalar parameter (the address of the variable, a collector root
    /// in the slot), optionally calling itself, optionally nested in `outer`.
    fn by_ref(calls: bool, nested: bool) -> (Program, yavs_ir::FuncId) {
        let mut pb = ProgramBuilder::new();
        let outer = pb.declare("outer", FuncKind::Procedure, None);
        let inc = pb.declare("inc", FuncKind::Procedure, nested.then_some(outer));
        let mut fb = pb.define(outer);
        fb.stmt(StmtKind::Return, Span::new(FileId(0), 0, 1));
        fb.emit(Inst::Ret { v: None });
        fb.finish();
        let mut fb = pb.define(inc);
        let s = fb.param("v", Width::B8, ParamClass::Value, Mode::Inout, None);
        fb.root(s, yavs_ir::RootKind::Addr);
        fb.stmt(StmtKind::Return, Span::new(FileId(0), 0, 1));
        let at = fb.addr(AddrSpace::Frame, s);
        let p = fb.reg(RegTy::Ptr);
        fb.emit(Inst::Load {
            dst: p,
            addr: at,
            w: Width::B8,
        });
        let (x, one) = (fb.reg(RegTy::I64), fb.int(1));
        fb.emit(Inst::Load {
            dst: x,
            addr: p,
            w: Width::B8,
        });
        fb.emit(Inst::Bin {
            op: yavs_ir::IntOp::Add,
            dst: x,
            a: x,
            b: one,
        });
        fb.emit(Inst::Store {
            addr: p,
            src: x,
            w: Width::B8,
        });
        if calls {
            let args = fb.regs(&[p]);
            fb.emit(Inst::Call {
                func: inc,
                args,
                dst: None,
            });
        }
        fb.emit(Inst::Ret { v: None });
        fb.finish();
        (pb.finish().unwrap(), inc)
    }

    /// An `out` or `inout` parameter and a static link do not keep a call from being lean; the
    /// parameter's slot is a collector root, so a callee in which a collection can run keeps its
    /// calls the full way (#730).
    #[test]
    fn lean_covers_by_reference_parameters_and_nested_functions() {
        for (calls, nested) in [(false, false), (false, true), (true, false)] {
            let (prog, id) = by_ref(calls, nested);
            let f = prog.func(id).unwrap();
            let pre = Pre::new(f, &prog, false);
            assert_eq!(pre.lean, !calls, "calls {calls} nested {nested}");
            assert_eq!(pre.nested, nested);
            if pre.lean {
                assert_eq!(pre.fslots, 0, "no collection runs in the callee: no root");
            }
        }
    }

    /// The contract tiers rely on (#566): the register holding a lean parameter's slot address
    /// is zero in every frame `fill` makes, yet `next` skips its `AddrOf`, so a tier entering
    /// at `next` must not read it (the other preloaded registers do hold their values).
    #[test]
    fn lean_parameter_address_register_is_unset() {
        let (prog, id) = depth();
        let f = prog.func(id).unwrap();
        let pre = Pre::new(f, &prog, false);
        assert!(pre.lean);
        // `finish` numbers the registers afresh: find the register in the code.
        let (def, at) = match f.code[0] {
            Inst::AddrOf {
                dst,
                space: AddrSpace::Frame,
                off: 0,
            } => (0, dst),
            ref i => panic!("{i:?}"),
        };
        assert!(pre.next[def] as usize > def, "the AddrOf is skipped");
        for idx in [0, 1, 7] {
            let mut stack = vec![99];
            pre.fill(&mut stack, idx).unwrap();
            assert_eq!(stack[1 + at.0 as usize], 0, "frame {idx}");
        }
    }
}
