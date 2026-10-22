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

use crate::exec::{CONST, FRAME, INST, PKG, R, SHARED, mk_addr};

pub(crate) struct Pre {
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

    pub(crate) fn new(f: &Func, prog: &Program) -> Pre {
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
        let mut code = Code::new(f, prog, &skip, &known, &next);
        let params: Box<[(u32, Width)]> = f.params.iter().map(|p| (p.slot, p.width)).collect();
        let pro = prologue(&mut code, &params);
        let lean = lean_params(f, prog, &code, &pro);
        // The registers holding the address of a parameter's slot are only read by the
        // prologue the call skips: they need no address.
        if lean {
            frame.retain(|&(_, off)| !f.params.iter().any(|p| p.slot as u64 == off));
        }
        let all_params: u32 = f.params.iter().map(|p| p.width.bytes()).sum();
        let fslots = if lean && f.frame == all_params && f.roots.is_empty() {
            0
        } else {
            f.frame
        };
        Pre {
            init,
            slots: f.frame,
            lean,
            fslots,
            params,
            pro,
            nested: f.parent.is_some(),
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

/// Bit of `PosIndexLoadCmpBr::dim`: the op heads a search loop (see there).
pub(crate) const SCAN: u8 = 0x80;

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
        rising: bool,
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
    /// The `Stmt` of a wait statement whose sensitivity is the same at every activation
    /// (`static_waits`): when the kernel can keep the last wait (`Ctx::wait_same`), the
    /// op goes on at the `Suspend` (op `to`) without naming the signals again.
    WaitStatic {
        to: u32,
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
    /// `AllocK`, a `Cat` into it and the `LCopy` of all of it (from and to element 0)
    /// into `dst`: the concatenation of logic elements is written there directly
    /// when it fits one block of planes (the temporary is dead).
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
    /// `Inst::Const` or `ConstF` (`v` the bits), defined more than once in the function.
    Const {
        dst: u32,
        v: i64,
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
    /// `Inst::CallNative`; `dst` is `NONE` without a result.
    CallNative {
        native: u32,
        start: u32,
        len: u32,
        dst: u32,
    },
    /// `Inst::CallNative` of a native `ieee` subprogram with at most four arguments, none a signal:
    /// the call without the bookkeeping other natives need (they may reach the heap, files and
    /// the like; these work on temporary memory only).
    CallIeee {
        native: u32,
        start: u32,
        len: u32,
        dst: u32,
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
    /// `SigVal` of one element and `rising_edge` / `falling_edge` of it.
    EdgeSig {
        dst: u32,
        sig: u32,
        offset: u32,
        rising: bool,
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
    /// `entry[0]`, so a call finds the first op without reading `entry`.
    pub first: u32,
    /// For each `Aff` op (sorted by op index): the IR pcs of the `Bin` run it stands for, from
    /// its first instruction to the one defining the op's result.
    pub aff: Vec<(u32, u32, u32)>,
    /// The `Case` tables of the `Op::Case`s, with op indices as targets.
    pub cases: Vec<CaseOps>,
    /// The op a call goes on at when it has written `Pre::pro` itself: after the ops that
    /// only load parameters (`NONE`: there are none).
    pub skip: u32,
}

/// Is every parameter handed over in the register the prologue loads it into, and read from
/// its slot by nothing else? Then a call writes the registers and leaves the slots alone.
/// Plain functions and procedures whose parameters are whole 8-byte values passed in only.
fn lean_params(f: &Func, prog: &Program, code: &Code, pro: &[u32]) -> bool {
    if f.nested
        || f.parent.is_some()
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
                || p.mode != yavs_ir::Mode::In
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
    /// `rising_edge` / `falling_edge` of the signal reference in register `sig`.
    Edge {
        rising: bool,
        sig: yavs_ir::Reg,
    },
}

/// Whether the native is an `ieee` one that `Op::CallIeee` runs: at most four arguments, none a
/// signal, not the procedure `uniform` (which writes through its arguments).
fn light_ieee(prog: &Program, f: &Func, native: yavs_ir::NativeId, args: yavs_ir::Regs) -> bool {
    let Some(sym) = prog.natives.get(native).map(|n| &n.symbol) else {
        return false;
    };
    let Some(key) = sym.strip_prefix("yavs.ieee.") else {
        return false;
    };
    let Some(regs) = f.regs(args) else {
        return false;
    };
    regs.len() <= 4
        && regs
            .iter()
            .all(|r| f.reg_ty.get(r.0 as usize) != Some(&yavs_ir::RegTy::Sig))
        && !matches!(
            yavs_ir::ieee::parse(key.split_once('@').map_or(key, |k| k.0)),
            None | Some(yavs_ir::ieee::Op::Uniform)
        )
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
        (Ie::Edge(rising), [r]) if f.reg_ty.get(r.0 as usize) == Some(&yavs_ir::RegTy::Sig) => {
            Some(NativeOp::Edge { rising, sig: *r })
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
fn live_after(f: &Func, pc: usize, r: yavs_ir::Reg) -> bool {
    let mut seen = vec![false; f.code.len() + 1];
    let mut stack = vec![pc + 1];
    while let Some(mut p) = stack.pop() {
        loop {
            let Some(i) = f.code.get(p) else {
                return true;
            };
            let (mut read, mut written) = (false, false);
            reads(f, i, |x| read |= x == r);
            i.visit(&mut |x, role, _| written |= role == Role::Def && x == r);
            if read {
                return true;
            }
            if written {
                break;
            }
            let to = |b: yavs_ir::BlockId| f.blocks.get(b).map(|b| b.start as usize);
            match *i {
                Inst::Br { to: t } => {
                    let Some(s) = to(t) else { return true };
                    if !std::mem::replace(&mut seen[s], true) {
                        stack.push(s);
                    }
                    break;
                }
                Inst::CondBr { t, f: e, .. } => {
                    for b in [t, e] {
                        let Some(s) = to(b) else { return true };
                        if !std::mem::replace(&mut seen[s], true) {
                            stack.push(s);
                        }
                    }
                    break;
                }
                Inst::Ret { .. } | Inst::Unreachable { .. } => break,
                Inst::Case { .. } => return true,
                _ => p += 1,
            }
        }
    }
    false
}

/// The waveform buffer made at IR pc `from` (register `wave`) and written through `base` is
/// read by the `Drive` at `to` and by nothing else: all in one block, `wave` read exactly by
/// its three instructions (`Store`, `Index` and the `Drive`), `base` once, neither read later.
fn waveform_is_private(
    f: &Func,
    block_start: &[bool],
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
    nw == 3
        && nb == 1
        && !live_after(f, to, yavs_ir::Reg(wave))
        && !live_after(f, to, yavs_ir::Reg(base))
}

/// Pairs and triples of adjacent ops that are one op (they never cross a block start: the
/// first op's pc stays an entry, the others' pcs are not).
#[allow(clippy::too_many_arguments)]
fn peephole(
    f: &Func,
    ops: &mut Vec<Op>,
    at: &mut Vec<u32>,
    start: &mut [u32],
    aff: &mut Vec<(u32, u32, u32)>,
    block_start: &[bool],
    uses: &[u32],
    known: &[Known],
) {
    let (old, old_at) = (std::mem::take(ops), std::mem::take(at));
    // The waveform starts `WaveBuf` made so far: wave register, the register 8 bytes in and
    // the IR pc of the `Alloc`.
    // The `Copy` positions where a `CopyDrive` goes, and the op.
    let mut dropped: Vec<(usize, Op)> = Vec::new();
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
                let pair = (i + 3..old.len()).take_while(|&j| free(j)).find_map(|j| {
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
                    let (from, to) = (*old_at.get(i)? as usize, *old_at.get(j + 1)? as usize);
                    let value = match old[j] {
                        Op::Copy { dst, src, bytes } if dst == e => Some((src, bytes, true)),
                        Op::LSet { base, idx, v } if base == e && int(idx) == Some(0) => {
                            Some((v, 0, false))
                        }
                        _ => None,
                    }?;
                    let clean = wave == w
                        && int(n) == Some(1)
                        && free(j + 1)
                        && waveform_is_private(f, block_start, from, to, w, e)
                        && !(from + 3..to).any(|p| {
                            let mut def = false;
                            f.code[p]
                                .visit(&mut |r, role, _| def |= role == Role::Def && r.0 == time);
                            def
                        });
                    let r = |x: u32| u16::try_from(x).ok();
                    match (r(driver), r(target), r(time), r(size), r(w), r(e), r(n)) {
                        (
                            Some(driver),
                            Some(target),
                            Some(time),
                            Some(size),
                            Some(w16),
                            Some(e16),
                            Some(n),
                        ) if clean => {
                            let (a, b, copy) = value;
                            if copy {
                                Some((
                                    j,
                                    Op::CopyDrive {
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
                                ))
                            } else {
                                Some((
                                    j,
                                    Op::LSetDrive {
                                        v: a,
                                        driver,
                                        target,
                                        time,
                                        size,
                                        w: w16,
                                        e: e16,
                                        inertial,
                                    },
                                ))
                            }
                        }
                        _ => None,
                    }
                });
                match pair {
                    Some((j, op)) => {
                        dropped.push((j, op));
                        remap[i] = ops.len() as u32;
                        i += 3;
                        continue;
                    }
                    None => (Some(Op::WaveBuf { w, size, time, e }), 3),
                }
            }
            _ if let Some(p) = dropped.iter().position(|d| d.0 == i) => {
                let (_, op) = dropped.swap_remove(p);
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
    let Some(k) = ops.iter().position(|o| matches!(o, Op::Suspend)) else {
        return;
    };
    let calls = |i: &Inst| match *i {
        Inst::Call { func, .. } | Inst::CallProt { func, .. } => prog
            .func(func)
            .is_none_or(|c| c.flags.contains(FuncFlags::MAY_SUSPEND)),
        Inst::CallIndirect { .. } => true,
        _ => false,
    };
    let one = f.resume.len() == 1
        && f.code
            .iter()
            .filter(|i| matches!(i, Inst::Suspend { .. }))
            .count()
            == 1
        && (f.flags.contains(FuncFlags::SENS_LIST) || !f.code.iter().any(calls));
    let Some(&s) = at.get(k).filter(|_| one) else {
        return;
    };
    let s = s as usize;
    let Some(st) = (0..s).rev().find(|&p| {
        !matches!(
            f.code[p],
            Inst::Const { .. }
                | Inst::ConstF { .. }
                | Inst::AddrOf { .. }
                | Inst::DopeLen { .. }
                | Inst::Pos { .. }
                | Inst::Index { .. }
                | Inst::Field { .. }
                | Inst::Bin { .. }
                | Inst::BinRange { .. }
                | Inst::Load { .. }
                | Inst::SigVal { .. }
                | Inst::SigSlice { .. }
                | Inst::WaitOn { .. }
        )
    }) else {
        return;
    };
    let waits = f.code[st + 1..s]
        .iter()
        .filter(|i| matches!(i, Inst::WaitOn { .. }))
        .count();
    if !matches!(f.code[st], Inst::Stmt)
        || waits < 2
        || (st + 1..=s).any(|p| f.blocks.iter().any(|(_, b)| b.start as usize == p))
    {
        return;
    }
    if let Some(&k0) = entry.get(st)
        && let Some(o @ Op::Stmt) = ops.get_mut(k0 as usize)
    {
        *o = Op::WaitStatic { to: k as u32 };
    }
}

fn superops(ops: &mut [Op], at: &[u32], f: &Func, known: &[Known], uses: &[u32]) {
    let int = |r: u32| match known.get(r as usize) {
        Some(&Known::Int(v)) => Some(v),
        _ => None,
    };
    let once = |r: u32| uses.get(r as usize) == Some(&1);
    let r16 = |r: u32| u16::try_from(r).ok();
    for k in 0..ops.len() {
        let fused = match (ops[k], ops.get(k + 1), ops.get(k + 2)) {
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
                    rising,
                },
                Some(&Op::CondBr { c, t, f }),
                _,
            ) if c == dst => Some(Op::EdgeBr {
                dst,
                rising,
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
                Some(&Op::LCopy {
                    dst,
                    doff,
                    src,
                    soff,
                    n,
                }),
            ) if let Some(size) = int(sz).and_then(|n| u32::try_from(n).ok())
                && src == tmp
                && doff == soff
                && int(doff) == Some(0)
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
                Op::Alloc { dst, size },
                Some(&Op::LCopy {
                    dst: d2,
                    doff,
                    src,
                    soff,
                    n,
                }),
                _,
            ) if int(size) == Some(32) && d2 == dst && src != dst => Some(Op::AllocLCopy {
                dst,
                doff,
                src,
                soff,
                n,
            }),
            (Op::Alloc { dst, size }, Some(&Op::SigReadVec { dst: d2, sig }), _)
                if int(size) == Some(32) && d2 == dst =>
            {
                Some(Op::AllocReadVec { dst, sig })
            }
            (Op::Alloc { dst, size }, Some(&Op::Cat), _)
                if int(size) == Some(32)
                    && let Some(&cp) = at.get(k + 1)
                    && let Some(&Inst::Cat {
                        dst: cd,
                        logic: true,
                        ..
                    }) = f.code.get(cp as usize)
                    && cd.0 == dst =>
            {
                Some(Op::AllocCat { dst })
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
    // A `Stmt` before the edge test or the wait of a clocked process is part of that op, and a
    // jump to such a wait is a copy of it (the end of every activation).
    for k in 0..ops.len() {
        let fused = match (ops[k], ops.get(k + 1)) {
            (
                Op::Stmt,
                Some(&Op::EdgeBr {
                    dst,
                    rising,
                    stmt: false,
                    sig,
                    offset,
                    t,
                    f,
                }),
            ) => Some(Op::EdgeBr {
                dst,
                rising,
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
            _ => None,
        };
        if let Some(op) = fused {
            ops[k] = op;
        }
    }
    for k in 0..ops.len() {
        if let Op::Br { to } = ops[k]
            && let Some(&w @ Op::WaitSuspend { .. }) = ops.get(to as usize)
        {
            ops[k] = w;
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
    fn new(f: &Func, prog: &Program, skip: &[bool], known: &[Known], next: &[u32]) -> Code {
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
                        Some(Inst::CallNative {
                            native,
                            args,
                            dst: d,
                        }) if one => match (native_op(prog, f, native, args), d) {
                            (Some(NativeOp::Edge { rising, sig: r }), Some(d)) if r == dst => {
                                Some(Op::EdgeSig {
                                    dst: d.0,
                                    sig,
                                    offset: offset.0,
                                    rising,
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
                        (Some(NativeOp::Gate { g, a, b }), Some(d)) => Op::Gate {
                            g,
                            dst: d.0,
                            a: a.0,
                            b: b.0,
                        },
                        (Some(NativeOp::Not { a }), Some(d)) => Op::NotL { dst: d.0, a: a.0 },
                        _ if light_ieee(prog, f, native, args) => Op::CallIeee {
                            native: native.0,
                            start: args.start,
                            len: args.len,
                            dst: dst.map_or(NONE, |d| d.0),
                        },
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
                Inst::Const { dst, v } => Op::Const { dst: dst.0, v },
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
        peephole(
            f,
            &mut ops,
            &mut at,
            &mut start,
            &mut aff,
            &block_start,
            &uses,
            known,
        );
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
        superops(&mut ops, &at, f, known, &uses);
        static_waits(prog, f, &mut ops, &at, &entry);
        let first = entry.first().copied().unwrap_or(NONE);
        Code {
            ops,
            at,
            entry,
            first,
            aff,
            cases,
            skip: NONE,
        }
    }
}
