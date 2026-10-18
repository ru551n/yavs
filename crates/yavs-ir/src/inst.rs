//! Instructions (ir.md 1.5).
//!
//! Operands are registers or small immediates; addresses come from registers. Variable-length
//! operand lists ([`Regs`]) index `Func::operands`, so `Inst` stays `Copy` and small.

use std::fmt;

use crate::{
    BlockId, CaseTableId, CheckMsgId, ConstId, CovItem, ForeignId, FuncId, NativeId, TyId,
};

/// Bytes of one entry of [`Inst::SigParts`]: reference, driver slot, first element.
pub const SIG_PART_BYTES: u32 = 32;

/// A register: an untyped 64-bit cell. A `Sig` value takes two consecutive registers.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Reg(pub u32);

impl fmt::Debug for Reg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "r{}", self.0)
    }
}

/// A list of registers in `Func::operands[start .. start + len]`.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Default)]
pub struct Regs {
    pub start: u32,
    pub len: u32,
}

impl fmt::Debug for Regs {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.len)
    }
}

/// What a register holds, for the verifier and the LLVM tier.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RegTy {
    /// Integers, physical values, enums, booleans, handles.
    I64,
    F64,
    /// An address (`Addr`).
    Ptr,
    /// A signal reference in two registers: `{base, offset}` and `len` (one range of one
    /// instance signal), or a reference of several parts (`Inst::SigParts`).
    Sig,
}

/// Memory access width. Narrow loads zero-extend.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Width {
    B1,
    B4,
    B8,
    /// A signal reference: 16 bytes, two registers.
    Sig,
}

impl Width {
    pub fn bytes(self) -> u32 {
        match self {
            Width::B1 => 1,
            Width::B4 => 4,
            Width::B8 => 8,
            Width::Sig => 16,
        }
    }
}

/// The space of an `AddrOf`. `Up(depth)` walks `depth` static links and yields a frame
/// address; `Shared` is the shared-variable area (serial processes only).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AddrSpace {
    Frame,
    Up(u8),
    Inst,
    Pkg(u32),
    Const(ConstId),
    Shared,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IntOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Mod,
    Pow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IntUnOp {
    Neg,
    Abs,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FOp {
    Add,
    Sub,
    Mul,
    Div,
    /// `real ** integer` (`b` is an integer register).
    PowI,
    /// `real ** real` (2008 `math_real` style).
    Pow,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FUnOp {
    Neg,
    Abs,
}

/// Physical times real (`a` physical, `b` real).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PhysOp {
    MulReal,
    DivReal,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BoolOp {
    And,
    Or,
    Xor,
    Not,
}

/// Element-wise logic-array operations (bit-planes, yavs-logic).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LOp {
    And,
    Or,
    Nand,
    Nor,
    Xor,
    Xnor,
    Not,
    ToX01,
    ToX01Z,
    ToUx01,
    Resolve,
}

/// Numeric operations on logic arrays (numeric_std style, equal lengths).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LNumOp {
    Add,
    Sub,
    /// Compares write a boolean register; an operand with a metavalue compares FALSE
    /// (numeric_std semantics).
    Cmp(CmpOp),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CheckKind {
    /// `lo <= v <= hi`.
    Range,
    /// Like `Range`, reported as an index error.
    Index,
    /// `v == lo` (array lengths).
    Length,
    /// `v != 0` (null access).
    Null,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AttrKind {
    Event,
    Active,
    LastEvent,
    LastActive,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DriveMode {
    Transport,
    /// Inertial; the rejection limit is the instruction's `reject` register when present.
    Inertial,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ForceKind {
    /// `force in`: the effective value.
    In,
    /// `force out`: the driving value.
    Out,
}

/// Elaboration-only operations (yavs-elab defines their operands). The verifier rejects
/// them outside `FuncKind::Elab`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ElabOp {
    Signal,
    Driver,
    Process,
    Instance,
    Port,
    External,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Inst {
    // move / const
    Const {
        dst: Reg,
        v: i64,
    },
    ConstF {
        dst: Reg,
        v: f64,
    },
    Move {
        dst: Reg,
        src: Reg,
    },
    AddrOf {
        dst: Reg,
        space: AddrSpace,
        off: u32,
    },
    CopyConst {
        dst: Reg,
        c: ConstId,
    },
    /// `size` bytes of zeroed temporary memory, live until the end of the statement.
    Alloc {
        dst: Reg,
        size: Reg,
    },
    /// `size` bytes of zeroed memory appended to the data of the package being initialised,
    /// live as long as the package (package initialisation thunks only): constants whose
    /// size is known only at run time.
    PkgAlloc {
        dst: Reg,
        size: Reg,
    },
    // memory
    Load {
        dst: Reg,
        addr: Reg,
        w: Width,
    },
    Store {
        addr: Reg,
        src: Reg,
        w: Width,
    },
    Copy {
        dst: Reg,
        src: Reg,
        bytes: Reg,
    },
    /// `n` copies of the `size`-byte element at `elem`.
    Fill {
        dst: Reg,
        elem: Reg,
        n: Reg,
        size: u32,
    },
    /// Checked position (0-based, from the left) of index `idx` in dimension `dim` of `dope`.
    Pos {
        dst: Reg,
        dope: Reg,
        dim: u8,
        idx: Reg,
        what: CheckMsgId,
    },
    /// `base + pos * stride`.
    Index {
        dst: Reg,
        base: Reg,
        pos: Reg,
        stride: u32,
    },
    /// Field `n` of a record whose layout (`[size, align, off_0, off_1, ...]` as u64) is at
    /// `layout`.
    Field {
        dst: Reg,
        base: Reg,
        layout: Reg,
        n: u32,
    },
    // integer and physical, checked (rule 7)
    Bin {
        op: IntOp,
        dst: Reg,
        a: Reg,
        b: Reg,
    },
    Un {
        op: IntUnOp,
        dst: Reg,
        a: Reg,
    },
    // real
    FBin {
        op: FOp,
        dst: Reg,
        a: Reg,
        b: Reg,
    },
    FUn {
        op: FUnOp,
        dst: Reg,
        a: Reg,
    },
    Phys {
        op: PhysOp,
        dst: Reg,
        a: Reg,
        b: Reg,
    },
    IToF {
        dst: Reg,
        src: Reg,
    },
    /// Round to nearest (LRM 9.3.6), checked.
    FToI {
        dst: Reg,
        src: Reg,
    },
    /// `FToI` checked against the integer subtype `lo .. hi`; the message shows the real
    /// (`value 30000000000 outside of INTEGER range ...`).
    FToIRange {
        dst: Reg,
        src: Reg,
        lo: Reg,
        hi: Reg,
        what: CheckMsgId,
    },
    // compare / logic
    Cmp {
        op: CmpOp,
        dst: Reg,
        a: Reg,
        b: Reg,
    },
    FCmp {
        op: CmpOp,
        dst: Reg,
        a: Reg,
        b: Reg,
    },
    Bool {
        op: BoolOp,
        dst: Reg,
        a: Reg,
        b: Reg,
    },
    LogicOp {
        op: LOp,
        dst: Reg,
        a: Reg,
        b: Reg,
        n: Reg,
    },
    LNum {
        op: LNumOp,
        dst: Reg,
        a: Reg,
        b: Reg,
        n: Reg,
        signed: bool,
    },
    /// Element-wise equality of two logic arrays of `n` elements.
    LEq {
        dst: Reg,
        a: Reg,
        b: Reg,
        n: Reg,
    },
    LGet {
        dst: Reg,
        base: Reg,
        idx: Reg,
    },
    LSet {
        base: Reg,
        idx: Reg,
        v: Reg,
    },
    LCopy {
        dst: Reg,
        dst_off: Reg,
        src: Reg,
        src_off: Reg,
        n: Reg,
    },
    // checks
    Check {
        kind: CheckKind,
        v: Reg,
        lo: Reg,
        hi: Reg,
        what: CheckMsgId,
    },
    /// `lo <= v <= hi` on reals (real subtype constraints).
    FCheck {
        v: Reg,
        lo: Reg,
        hi: Reg,
        what: CheckMsgId,
    },
    /// Integer arithmetic whose result must lie in `lo .. hi` (the base type of the
    /// operands): `result of 2147483647 + 1 cannot be represented as INTEGER`.
    BinRange {
        op: IntOp,
        dst: Reg,
        a: Reg,
        b: Reg,
        lo: Reg,
        hi: Reg,
        what: CheckMsgId,
    },
    // subtypes
    /// Write a descriptor at `at`: `parts` are `left, right, dir` per dimension.
    MakeDope {
        at: Reg,
        parts: Regs,
    },
    DopeLen {
        dst: Reg,
        dope: Reg,
        dim: u8,
    },
    /// Layout of `ty` constrained by the descriptor at `desc`, written to temporary memory as
    /// `[size, align, offsets...]` (records) or `[size, align, stride, count]` (arrays).
    LayoutOf {
        dst: Reg,
        ty: TyId,
        desc: Reg,
    },
    /// Per-dimension length check of the value descriptor `dope` against the target `want`.
    SubtypeCheck {
        dope: Reg,
        want: Reg,
        dims: u8,
        what: CheckMsgId,
    },
    // control
    Br {
        to: BlockId,
    },
    CondBr {
        c: Reg,
        t: BlockId,
        f: BlockId,
    },
    Case {
        v: Reg,
        table: CaseTableId,
    },
    Ret {
        v: Option<Reg>,
    },
    /// Reaching it is a runtime error with this message (a function without `return`).
    Unreachable {
        what: CheckMsgId,
    },
    // calls
    Call {
        func: FuncId,
        args: Regs,
        dst: Option<Reg>,
    },
    CallNative {
        native: NativeId,
        args: Regs,
        dst: Option<Reg>,
    },
    CallForeign {
        foreign: ForeignId,
        args: Regs,
        dst: Option<Reg>,
    },
    CallIndirect {
        func: Reg,
        args: Regs,
        dst: Option<Reg>,
    },
    /// A protected method; `obj` is passed as the first parameter (`self`).
    CallProt {
        func: FuncId,
        obj: Reg,
        args: Regs,
        dst: Option<Reg>,
    },
    // heap
    /// Allocate an object of `ty` initialised from `size` bytes at `init`.
    New {
        dst: Reg,
        ty: TyId,
        init: Reg,
        size: Reg,
        shared: bool,
    },
    /// `deallocate`: frees the object the access variable at `ptr` designates and sets the
    /// variable to null; null is a no-op.
    Free {
        ptr: Reg,
    },
    /// The address of the object a handle designates; null and stale handles are errors.
    Deref {
        dst: Reg,
        handle: Reg,
        what: CheckMsgId,
    },
    // signals
    /// A signal reference to instance signal `sig`, elements `offset .. offset + len`.
    SigVal {
        dst: Reg,
        sig: u32,
        offset: Reg,
        len: Reg,
    },
    /// A signal reference over several parts, possibly in different signals, in order of
    /// the referenced object's kernel elements (an individually associated signal parameter,
    /// #71). `parts` holds `n` entries of [`SIG_PART_BYTES`]: a signal reference (16 bytes),
    /// the caller's driver slot for it (`-1`: none) and the part's first element in the whole
    /// (8 bytes each). The entries tile `0 .. total` in any order; a part may itself be a
    /// reference of several parts.
    SigParts {
        dst: Reg,
        parts: Reg,
        n: Reg,
    },
    /// Elements `offset .. offset + len` of signal reference `sig`, of either form.
    SigSlice {
        dst: Reg,
        sig: Reg,
        offset: Reg,
        len: Reg,
    },
    SigRead {
        dst: Reg,
        sig: Reg,
    },
    SigReadScalar {
        dst: Reg,
        sig: Reg,
    },
    SigLastValue {
        dst: Reg,
        sig: Reg,
    },
    SigAttr {
        dst: Reg,
        kind: AttrKind,
        sig: Reg,
    },
    /// `'driving` of driver slot `driver`.
    Driving {
        dst: Reg,
        driver: u32,
    },
    /// `'driving_value` of driver slot `driver`, written to memory at `dst`.
    DrivingValue {
        dst: Reg,
        driver: u32,
    },
    /// `n` `(time, value)` pairs at `wave`, each `8 + value size rounded up to 8` bytes.
    Drive {
        driver: u32,
        target: Option<Reg>,
        wave: Reg,
        n: Reg,
        mode: DriveMode,
        reject: Option<Reg>,
    },
    /// [`Inst::Drive`] through the driver slot in register `slot`: an `out`/`inout` signal
    /// parameter drives its actual through the caller's slot, passed as a cell.
    DriveSlot {
        slot: Reg,
        target: Reg,
        wave: Reg,
        n: Reg,
        mode: DriveMode,
        reject: Option<Reg>,
    },
    Disconnect {
        driver: u32,
        target: Option<Reg>,
        after: Option<Reg>,
        mode: DriveMode,
    },
    Force {
        sig: Reg,
        kind: ForceKind,
        value: Reg,
    },
    Release {
        sig: Reg,
        kind: ForceKind,
    },
    // wait
    WaitOn {
        sig: Reg,
    },
    WaitFor {
        t: Reg,
    },
    WaitAt {
        t: Reg,
    },
    /// End of `run`; execution continues at the next instruction. `resume` indexes
    /// `Func::resume` (filled in by the builder).
    Suspend {
        resume: u32,
    },
    TimedOut {
        dst: Reg,
    },
    Now {
        dst: Reg,
    },
    Delta {
        dst: Reg,
    },
    // effects
    Report {
        msg: Reg,
        len: Reg,
        severity: Reg,
    },
    Output {
        addr: Reg,
        len: Reg,
    },
    Cover {
        item: CovItem,
    },
    /// Statement boundary: the debugger hook, and the end of the previous statement's
    /// temporaries.
    Stmt,
    Elab {
        op: ElabOp,
        args: Regs,
        dst: Option<Reg>,
    },
}

/// How an instruction uses a register.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Role {
    Def,
    Use,
}

impl Inst {
    pub fn is_terminator(&self) -> bool {
        matches!(
            self,
            Inst::Br { .. }
                | Inst::CondBr { .. }
                | Inst::Case { .. }
                | Inst::Ret { .. }
                | Inst::Unreachable { .. }
        )
    }

    /// The operand list, if any.
    pub fn list(&self) -> Option<Regs> {
        match *self {
            Inst::MakeDope { parts, .. } => Some(parts),
            Inst::Call { args, .. }
            | Inst::CallNative { args, .. }
            | Inst::CallForeign { args, .. }
            | Inst::CallIndirect { args, .. }
            | Inst::CallProt { args, .. }
            | Inst::Elab { args, .. } => Some(args),
            _ => None,
        }
    }

    /// Every fixed register operand with its role and expected type (`None`: any cell that
    /// is not a signal). List operands are not included (see [`Inst::list`]).
    pub fn visit(&self, f: &mut dyn FnMut(Reg, Role, Option<RegTy>)) {
        use RegTy::*;
        use Role::*;
        let i = Some(I64);
        let p = Some(Ptr);
        let s = Some(Sig);
        let fl = Some(F64);
        let mut d = |r: Reg, t: Option<RegTy>| f(r, Def, t);
        match *self {
            Inst::Const { dst, .. } => d(dst, None),
            Inst::ConstF { dst, .. } => d(dst, fl),
            Inst::Move { dst, src } => {
                d(dst, None);
                f(src, Use, None);
            }
            Inst::AddrOf { dst, .. } => d(dst, p),
            Inst::CopyConst { dst, .. } => f(dst, Use, p),
            Inst::Alloc { dst, size } | Inst::PkgAlloc { dst, size } => {
                d(dst, p);
                f(size, Use, i);
            }
            Inst::Load { dst, addr, w } => {
                d(dst, (w == Width::Sig).then_some(Sig));
                f(addr, Use, p);
            }
            Inst::Store { addr, src, w } => {
                f(addr, Use, p);
                f(src, Use, (w == Width::Sig).then_some(Sig));
            }
            Inst::Copy { dst, src, bytes } => {
                f(dst, Use, p);
                f(src, Use, p);
                f(bytes, Use, i);
            }
            Inst::Fill { dst, elem, n, .. } => {
                f(dst, Use, p);
                f(elem, Use, p);
                f(n, Use, i);
            }
            Inst::Pos { dst, dope, idx, .. } => {
                d(dst, i);
                f(dope, Use, p);
                f(idx, Use, i);
            }
            Inst::Index { dst, base, pos, .. } => {
                d(dst, p);
                f(base, Use, p);
                f(pos, Use, i);
            }
            Inst::Field {
                dst, base, layout, ..
            } => {
                d(dst, p);
                f(base, Use, p);
                f(layout, Use, p);
            }
            Inst::Bin { dst, a, b, .. } | Inst::Cmp { dst, a, b, .. } => {
                d(dst, i);
                f(a, Use, i);
                f(b, Use, i);
            }
            Inst::Bool { op, dst, a, b } => {
                d(dst, i);
                f(a, Use, i);
                if op != BoolOp::Not {
                    f(b, Use, i);
                }
            }
            Inst::Un { dst, a, .. } => {
                d(dst, i);
                f(a, Use, i);
            }
            Inst::FBin { op, dst, a, b } => {
                d(dst, fl);
                f(a, Use, fl);
                f(b, Use, if op == FOp::PowI { i } else { fl });
            }
            Inst::FUn { dst, a, .. } => {
                d(dst, fl);
                f(a, Use, fl);
            }
            Inst::Phys { dst, a, b, .. } => {
                d(dst, i);
                f(a, Use, i);
                f(b, Use, fl);
            }
            Inst::IToF { dst, src } => {
                d(dst, fl);
                f(src, Use, i);
            }
            Inst::FToI { dst, src } => {
                d(dst, i);
                f(src, Use, fl);
            }
            Inst::FToIRange {
                dst, src, lo, hi, ..
            } => {
                d(dst, i);
                f(src, Use, fl);
                f(lo, Use, i);
                f(hi, Use, i);
            }
            Inst::FCheck { v, lo, hi, .. } => {
                f(v, Use, fl);
                f(lo, Use, fl);
                f(hi, Use, fl);
            }
            Inst::BinRange {
                dst, a, b, lo, hi, ..
            } => {
                d(dst, i);
                f(a, Use, i);
                f(b, Use, i);
                f(lo, Use, i);
                f(hi, Use, i);
            }
            Inst::FCmp { dst, a, b, .. } => {
                d(dst, i);
                f(a, Use, fl);
                f(b, Use, fl);
            }
            Inst::LogicOp { op, dst, a, b, n } => {
                f(dst, Use, p);
                f(a, Use, p);
                if !matches!(op, LOp::Not | LOp::ToX01 | LOp::ToX01Z | LOp::ToUx01) {
                    f(b, Use, p);
                }
                f(n, Use, i);
            }
            Inst::LNum {
                op, dst, a, b, n, ..
            } => {
                if let LNumOp::Cmp(_) = op {
                    d(dst, i);
                } else {
                    f(dst, Use, p);
                }
                f(a, Use, p);
                f(b, Use, p);
                f(n, Use, i);
            }
            Inst::LEq { dst, a, b, n } => {
                d(dst, i);
                f(a, Use, p);
                f(b, Use, p);
                f(n, Use, i);
            }
            Inst::LGet { dst, base, idx } => {
                d(dst, i);
                f(base, Use, p);
                f(idx, Use, i);
            }
            Inst::LSet { base, idx, v } => {
                f(base, Use, p);
                f(idx, Use, i);
                f(v, Use, i);
            }
            Inst::LCopy {
                dst,
                dst_off,
                src,
                src_off,
                n,
            } => {
                f(dst, Use, p);
                f(dst_off, Use, i);
                f(src, Use, p);
                f(src_off, Use, i);
                f(n, Use, i);
            }
            Inst::Check {
                v, lo, hi, kind, ..
            } => {
                f(v, Use, i);
                if kind != CheckKind::Null {
                    f(lo, Use, i);
                }
                if matches!(kind, CheckKind::Range | CheckKind::Index) {
                    f(hi, Use, i);
                }
            }
            Inst::MakeDope { at, .. } => f(at, Use, p),
            Inst::DopeLen { dst, dope, .. } => {
                d(dst, i);
                f(dope, Use, p);
            }
            Inst::LayoutOf { dst, desc, .. } => {
                d(dst, p);
                f(desc, Use, p);
            }
            Inst::SubtypeCheck { dope, want, .. } => {
                f(dope, Use, p);
                f(want, Use, p);
            }
            Inst::Br { .. } | Inst::Unreachable { .. } | Inst::Stmt | Inst::Cover { .. } => {}
            Inst::CondBr { c, .. } => f(c, Use, i),
            Inst::Case { v, .. } => f(v, Use, i),
            Inst::Ret { v } => {
                if let Some(v) = v {
                    f(v, Use, None);
                }
            }
            Inst::Call { dst, .. }
            | Inst::CallNative { dst, .. }
            | Inst::CallForeign { dst, .. }
            | Inst::Elab { dst, .. } => {
                if let Some(r) = dst {
                    d(r, None);
                }
            }
            Inst::CallIndirect { func, dst, .. } => {
                f(func, Use, i);
                if let Some(r) = dst {
                    f(r, Def, None);
                }
            }
            Inst::CallProt { obj, dst, .. } => {
                f(obj, Use, i);
                if let Some(r) = dst {
                    f(r, Def, None);
                }
            }
            Inst::New {
                dst, init, size, ..
            } => {
                d(dst, i);
                f(init, Use, p);
                f(size, Use, i);
            }
            Inst::Free { ptr } => f(ptr, Use, p),
            Inst::Deref { dst, handle, .. } => {
                d(dst, p);
                f(handle, Use, i);
            }
            Inst::SigVal {
                dst, offset, len, ..
            } => {
                d(dst, s);
                f(offset, Use, i);
                f(len, Use, i);
            }
            Inst::SigParts { dst, parts, n } => {
                d(dst, s);
                f(parts, Use, p);
                f(n, Use, i);
            }
            Inst::SigSlice {
                dst,
                sig,
                offset,
                len,
            } => {
                d(dst, s);
                f(sig, Use, s);
                f(offset, Use, i);
                f(len, Use, i);
            }
            Inst::SigRead { dst, sig } | Inst::SigLastValue { dst, sig } => {
                f(dst, Use, p);
                f(sig, Use, s);
            }
            Inst::SigReadScalar { dst, sig } | Inst::SigAttr { dst, sig, .. } => {
                d(dst, i);
                f(sig, Use, s);
            }
            Inst::Driving { dst, .. } => d(dst, i),
            Inst::DrivingValue { dst, .. } => f(dst, Use, p),
            Inst::Drive {
                target,
                wave,
                n,
                reject,
                ..
            } => {
                if let Some(t) = target {
                    f(t, Use, s);
                }
                f(wave, Use, p);
                f(n, Use, i);
                if let Some(r) = reject {
                    f(r, Use, i);
                }
            }
            Inst::DriveSlot {
                slot,
                target,
                wave,
                n,
                reject,
                ..
            } => {
                f(slot, Use, i);
                f(target, Use, s);
                f(wave, Use, p);
                f(n, Use, i);
                if let Some(r) = reject {
                    f(r, Use, i);
                }
            }
            Inst::Disconnect { target, after, .. } => {
                if let Some(t) = target {
                    f(t, Use, s);
                }
                if let Some(a) = after {
                    f(a, Use, i);
                }
            }
            Inst::Force { sig, value, .. } => {
                f(sig, Use, s);
                f(value, Use, p);
            }
            Inst::Release { sig, .. } | Inst::WaitOn { sig } => f(sig, Use, s),
            Inst::WaitFor { t } | Inst::WaitAt { t } => f(t, Use, i),
            Inst::Suspend { .. } => {}
            Inst::TimedOut { dst } | Inst::Now { dst } | Inst::Delta { dst } => d(dst, i),
            Inst::Report { msg, len, severity } => {
                f(msg, Use, p);
                f(len, Use, i);
                f(severity, Use, i);
            }
            Inst::Output { addr, len } => {
                f(addr, Use, p);
                f(len, Use, i);
            }
        }
    }

    /// Branch targets.
    pub fn targets(&self) -> Vec<BlockId> {
        match *self {
            Inst::Br { to } => vec![to],
            Inst::CondBr { t, f, .. } => vec![t, f],
            _ => Vec::new(),
        }
    }
}
