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
#[derive(
    serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash,
)]
pub struct Reg(pub u32);

impl fmt::Debug for Reg {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "r{}", self.0)
    }
}

/// `SigScatter::has`: the operand list has the rejection limit.
pub const SCATTER_REJECT: u8 = 1;
/// `SigScatter::has`: the operand list ends with a [`Part`].
pub const SCATTER_PART: u8 = 2;

/// The part of a signal of arrays of records that `SigGather`/`SigScatter` take: the first
/// element and the number of elements of the outermost array of the whole signal (integers).
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Part {
    pub first: Reg,
    pub outer: Reg,
}

/// A list of registers in `Func::operands[start .. start + len]`.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, PartialEq, Eq, Hash, Default)]
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
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RegTy {
    /// Integers, physical values, enums, booleans.
    I64,
    F64,
    /// An address (`Addr`).
    Ptr,
    /// A signal reference in two registers: `{base, offset}` and `len` (one range of one
    /// instance signal), or a reference of several parts (`Inst::SigParts`).
    Sig,
    /// An access handle (ir.md 1.6): an `I64` the GC treats as a root while it is live.
    Handle,
}

/// Memory access width. `B1` loads zero-extend, `B4` loads sign-extend (a 4-byte cell holds a
/// narrow integer array element or a non-negative enum position or file handle).
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AddrSpace {
    Frame,
    Up(u8),
    Inst,
    Pkg(u32),
    Const(ConstId),
    Shared,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IntOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    Mod,
    Pow,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum IntUnOp {
    Neg,
    Abs,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
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

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FUnOp {
    Neg,
    Abs,
}

/// Physical times real (`a` physical, `b` real).
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum PhysOp {
    MulReal,
    DivReal,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BoolOp {
    And,
    Or,
    Xor,
    Not,
}

/// Element-wise logic-array operations (bit-planes, yavs-logic).
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
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
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LNumOp {
    Add,
    Sub,
    /// Compares write a boolean register; an operand with a metavalue compares FALSE
    /// (numeric_std semantics).
    Cmp(CmpOp),
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
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

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum AttrKind {
    Event,
    Active,
    LastEvent,
    LastActive,
}

/// The clock edge an [`Inst::Edge`] tests (#632). Signal values are `std_ulogic` codes; BIT and
/// BOOLEAN signals hold the codes of '0' and '1'.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum EdgeKind {
    /// `falling_edge`: an event, `To_X01` of the value '0' and of `'last_value` '1'.
    Falling = 0,
    /// `rising_edge`: an event in this cycle, `To_X01` of the value '1' and of `'last_value` '0'.
    Rising = 1,
    /// `x'event and x = '0'`.
    To0 = 2,
    /// `x'event and x = '1'`: an event and the value exactly '1' (whatever it was before).
    To1 = 3,
}

impl EdgeKind {
    /// The kernel's code of the test (`yavs_kernel::Ctx::edge_by`): bit 0 rising, bit 1 an
    /// event ending on the level.
    pub fn code(self) -> u8 {
        self as u8
    }

    /// Whether the edge ends on '1'.
    pub fn rising(self) -> bool {
        matches!(self, EdgeKind::Rising | EdgeKind::To1)
    }

    /// Whether it is an event ending on the level ([`EdgeKind::To1`], [`EdgeKind::To0`]), not the
    /// IEEE function.
    pub fn to_level(self) -> bool {
        matches!(self, EdgeKind::To1 | EdgeKind::To0)
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DriveMode {
    Transport,
    /// Inertial; the rejection limit is the instruction's `reject` register when present.
    Inertial,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ForceKind {
    /// `force in`: the effective value.
    In,
    /// `force out`: the driving value.
    Out,
    /// No mode written and the target an external name: `out` for the formal of a port of
    /// mode out, inout or buffer, else `in`; the kernel decides.
    Default,
}

/// Elaboration-only operations (yavs-elab defines their operands). The verifier rejects
/// them outside `FuncKind::Elab`.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ElabOp {
    Signal,
    Driver,
    Process,
    Instance,
    Port,
    External,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq)]
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
    /// `dst` is `a` when `c` is not zero, else `b` (any class but a signal reference).
    Select {
        dst: Reg,
        c: Reg,
        a: Reg,
        b: Reg,
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
    /// The temporary memory at `addr` holds, until the end of the statement, a value of `ty`
    /// (constrained by the descriptor at `desc`) whose handles are GC roots, or with `ty`
    /// `None` a fat pair `[data, desc]` whose addresses keep the heap objects they point
    /// into (ir.md 1.6).
    TempRoot {
        addr: Reg,
        desc: Option<Reg>,
        ty: Option<TyId>,
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
    /// `bytes` zero bytes at `addr` (a multiple of 8): the zeroing an `Alloc` does, for memory that
    /// is not allocated here (the optimiser's temporaries in a process frame, `opt/slots.rs`).
    Zero {
        addr: Reg,
        bytes: u32,
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
    /// The start position of the slice `left` to or downto `right` (`dir` 0 or 1) in dimension 0
    /// of the descriptor `dope`: 0 for a null slice, which names no element and checks nothing;
    /// otherwise the checked position (as [`Inst::Pos`], message `what`) of `left`, with `right`
    /// checked the same way and `dir` checked against the array's direction (message `dir_what`).
    SlicePos {
        dst: Reg,
        dope: Reg,
        left: Reg,
        right: Reg,
        dir: Reg,
        what: CheckMsgId,
        dir_what: CheckMsgId,
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
    /// `dst` is 1 when the `n` bytes at `a` and at `b` are equal, else 0: equality of two
    /// arrays of integer or enumeration elements of the same length.
    MemEq {
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
    /// `LSet` of `v` at plane elements `idx .. idx + n`.
    LFill {
        base: Reg,
        idx: Reg,
        n: Reg,
        v: Reg,
    },
    LCopy {
        dst: Reg,
        dst_off: Reg,
        src: Reg,
        src_off: Reg,
        n: Reg,
    },
    /// Concatenation: store the pieces of `parts`, leftmost first, into the one-dimensional
    /// array of `n` elements at `dst`. A piece is two registers in the list: the address of an
    /// array and its element count, or (bit `k` of `scalars` set for piece `k`) one element in
    /// the register itself, the count ignored. Each element is `m` units: plane elements of
    /// a logic array (`logic`; a scalar piece is the std_ulogic code position plus `bias`)
    /// or bytes (a scalar piece is stored with width `w`). The stores are those of `LCopy`
    /// and `LSet` (logic) or `Copy` and `Store` at the piece's byte offset, in order.
    Cat {
        dst: Reg,
        n: Reg,
        m: Reg,
        parts: Regs,
        scalars: u16,
        bias: u8,
        logic: bool,
        w: Width,
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
    /// `[size, align, offsets...]` (records) or `[size, align, stride, count]` (arrays). With
    /// `cache`, a process's frame bytes at that offset keep the last layout of `ty` (the
    /// optimiser's `layout_cache`): a word that is 0 until a layout is kept, then 1, a copy of
    /// the descriptor bytes and the layout words. A descriptor equal to the copy makes the result
    /// the address of the kept words in the frame, which no statement ends; the interpreter may
    /// leave the bytes alone, a tier that keeps them checks the descriptor each time.
    LayoutOf {
        dst: Reg,
        ty: TyId,
        desc: Reg,
        cache: Option<u32>,
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
    /// Allocate an object of `ty` made of the `dsize` bytes at `desc` followed by `size` bytes:
    /// a copy of the bytes at `data`, or, with `fill` > 0, the `fill` bytes at `data` repeated
    /// (`size` is a multiple of `fill`). A `New` without the temporary the value would be
    /// built and copied through.
    NewFill {
        dst: Reg,
        ty: TyId,
        desc: Reg,
        dsize: u32,
        data: Reg,
        size: Reg,
        fill: u32,
        shared: bool,
    },
    /// A `NewFill` whose object lives in the frame's temporary memory instead of a heap: the
    /// bytes are allocated there and the frame's temporary base is raised over them, so the
    /// frame's own `Stmt` markers keep them and the caller's next `Stmt` frees them (as it
    /// frees what the frame returned in temporaries). `dst` is the address of the object (what
    /// `Deref` of the heap handle would give). Made only by the optimiser, for the storage of
    /// a local composite variable of a subprogram that cannot suspend, of a type without
    /// handles.
    NewFrame {
        dst: Reg,
        desc: Reg,
        dsize: u32,
        data: Reg,
        size: Reg,
        fill: u32,
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
    /// Reads every leaf of a record signal, or of an array of records, into the value at `dst`:
    /// the leaves are listed in table `tab` (`gather`), one entry per leaf with the instance
    /// signal and how it is stored; where it lies in the value follows from the value's type and
    /// the descriptor of the part of the signal at `desc`. `part` is the part of a signal of
    /// arrays of records that the value is (none: the whole signal). The result is what
    /// `SigRead`/`SigReadScalar` of each leaf stored into the value would give; how the kernel
    /// keeps the leaves does not show. One instruction for a whole record (#677).
    SigGather {
        dst: Reg,
        desc: Reg,
        tab: ConstId,
        part: Option<Part>,
    },
    /// Drives every leaf of a record signal, or of an array of records, from the value at `src`
    /// (laid out by the descriptor of the part of the signal at `desc`, which the caller has
    /// checked the value against) with one waveform element at delay `after`: leaf `k` of table
    /// `tab` through driver `driver + k`, in leaf order, as `Drive` of each leaf's waveform
    /// would (#677). `ops` lists the delay `after`, then the rejection limit if `has` has
    /// [`SCATTER_REJECT`], then the first element and the element count of the signal's outermost
    /// array (a [`Part`]: the part of a signal of arrays of records the value is) if `has` has
    /// [`SCATTER_PART`]; the instruction stays 32 bytes.
    SigScatter {
        driver: u32,
        src: Reg,
        desc: Reg,
        tab: ConstId,
        ops: Regs,
        mode: DriveMode,
        has: u8,
    },
    /// The clock-edge test `kind` (0 or 1) of the one element `sig` refers to, a `std_ulogic`,
    /// BIT or BOOLEAN signal: the one instruction every clock-edge idiom lowers to
    /// (`rising_edge`, `x'event and x = '1'`, the level of a `wait until`, ...).
    Edge {
        dst: Reg,
        kind: EdgeKind,
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
    /// Before the `WaitOn`s of a `wait until` whose condition is true only in a cycle where the
    /// edge `kind` of the one element `sig` refers to holds (an `and` conjunct of the condition
    /// is that edge): the wait resumes the process only in such a cycle. The kernel then leaves
    /// the process suspended for events of the other signals of the wait in cycles without the
    /// edge, which would only have evaluated the condition false and waited again.
    WaitEdge {
        sig: Reg,
        kind: EdgeKind,
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
    /// Condition coverage, one input: `dst = acc | code(src) << shift`, where the code is
    /// 1 for a false input, 2 for a true one and 3 for anything else (`ulogic`: a `std_ulogic`
    /// position, `0`/`L` false, `1`/`H` true; else a `bit`/`boolean` position). An input
    /// never evaluated leaves its two bits 0. With `shift` 0 (the first input of a condition,
    /// which always runs) `acc` is not read: the state starts there.
    CoverIn {
        dst: Reg,
        acc: Reg,
        src: Reg,
        shift: u8,
        ulogic: bool,
    },
    /// Condition coverage, end of an evaluation: count the rows of condition `cond`
    /// (`Program::cov_conds`) that the inputs collected in `state` cover.
    CoverCond {
        cond: u32,
        state: Reg,
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
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
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
            Inst::MakeDope { parts, .. } | Inst::Cat { parts, .. } => Some(parts),
            Inst::SigScatter { ops, .. } => Some(ops),
            Inst::Call { args, .. }
            | Inst::CallNative { args, .. }
            | Inst::CallForeign { args, .. }
            | Inst::CallIndirect { args, .. }
            | Inst::CallProt { args, .. }
            | Inst::Elab { args, .. } => Some(args),
            _ => None,
        }
    }

    /// The operand list, writable (the optimiser moves it).
    pub fn list_mut(&mut self) -> Option<&mut Regs> {
        match self {
            Inst::MakeDope { parts, .. } | Inst::Cat { parts, .. } => Some(parts),
            Inst::SigScatter { ops, .. } => Some(ops),
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
    /// is not a signal; a `Handle` register is also an `I64`). List operands are not included (see [`Inst::list`]).
    pub fn visit<F: FnMut(Reg, Role, Option<RegTy>) + ?Sized>(&self, f: &mut F) {
        let mut c = *self;
        c.visit_mut(&mut |r, role, t| f(*r, role, t));
    }

    /// [`Inst::visit`] with the operands writable (register renaming by the optimiser).
    pub fn visit_mut<F: FnMut(&mut Reg, Role, Option<RegTy>) + ?Sized>(&mut self, f: &mut F) {
        use RegTy::*;
        use Role::*;
        let i = Some(I64);
        let p = Some(Ptr);
        let s = Some(Sig);
        let fl = Some(F64);
        let h = Some(Handle);
        let mut d = |r: &mut Reg, t: Option<RegTy>| f(r, Def, t);
        match self {
            Inst::Const { dst, .. } => d(dst, None),
            Inst::ConstF { dst, .. } => d(dst, fl),
            Inst::Move { dst, src } => {
                d(dst, None);
                f(src, Use, None);
            }
            Inst::AddrOf { dst, .. } => d(dst, p),
            Inst::Select { dst, c, a, b } => {
                d(dst, None);
                f(c, Use, i);
                f(a, Use, None);
                f(b, Use, None);
            }
            Inst::CopyConst { dst, .. } => f(dst, Use, p),
            Inst::Alloc { dst, size } | Inst::PkgAlloc { dst, size } => {
                d(dst, p);
                f(size, Use, i);
            }
            Inst::TempRoot { addr, desc, .. } => {
                f(addr, Use, p);
                if let Some(r) = desc {
                    f(r, Use, p);
                }
            }
            Inst::Load { dst, addr, w } => {
                d(dst, (*w == Width::Sig).then_some(Sig));
                f(addr, Use, p);
            }
            Inst::Store { addr, src, w } => {
                f(addr, Use, p);
                f(src, Use, (*w == Width::Sig).then_some(Sig));
            }
            Inst::Copy { dst, src, bytes } => {
                f(dst, Use, p);
                f(src, Use, p);
                f(bytes, Use, i);
            }
            Inst::Zero { addr, .. } => f(addr, Use, p),
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
            Inst::SlicePos {
                dst,
                dope,
                left,
                right,
                dir,
                ..
            } => {
                d(dst, i);
                f(dope, Use, p);
                f(left, Use, i);
                f(right, Use, i);
                f(dir, Use, i);
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
                if *op != BoolOp::Not {
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
                f(b, Use, if *op == FOp::PowI { i } else { fl });
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
            Inst::MemEq { dst, a, b, n } => {
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
            Inst::LFill { base, idx, n, v } => {
                f(base, Use, p);
                f(idx, Use, i);
                f(n, Use, i);
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
            Inst::Cat { dst, n, m, .. } => {
                f(dst, Use, p);
                f(n, Use, i);
                f(m, Use, i);
            }
            Inst::Check {
                v, lo, hi, kind, ..
            } => {
                f(v, Use, i);
                if *kind != CheckKind::Null {
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
            Inst::CoverIn { dst, acc, src, .. } => {
                d(dst, i);
                f(acc, Use, i);
                f(src, Use, i);
            }
            Inst::CoverCond { state, .. } => f(state, Use, i),
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
                f(obj, Use, h);
                if let Some(r) = dst {
                    f(r, Def, None);
                }
            }
            Inst::New {
                dst, init, size, ..
            } => {
                d(dst, h);
                f(init, Use, p);
                f(size, Use, i);
            }
            Inst::NewFill {
                dst,
                desc,
                data,
                size,
                ..
            } => {
                d(dst, h);
                f(desc, Use, p);
                f(data, Use, p);
                f(size, Use, i);
            }
            Inst::NewFrame {
                dst,
                desc,
                data,
                size,
                ..
            } => {
                d(dst, p);
                f(desc, Use, p);
                f(data, Use, p);
                f(size, Use, i);
            }
            Inst::Free { ptr } => f(ptr, Use, p),
            Inst::Deref { dst, handle, .. } => {
                d(dst, p);
                f(handle, Use, h);
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
            Inst::SigReadScalar { dst, sig }
            | Inst::SigAttr { dst, sig, .. }
            | Inst::Edge { dst, sig, .. } => {
                d(dst, i);
                f(sig, Use, s);
            }
            Inst::SigGather {
                dst, desc, part, ..
            } => {
                f(dst, Use, p);
                f(desc, Use, p);
                if let Some(q) = part {
                    f(&mut q.first, Use, i);
                    f(&mut q.outer, Use, i);
                }
            }
            Inst::SigScatter { src, desc, .. } => {
                f(src, Use, p);
                f(desc, Use, p);
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
            Inst::Release { sig, .. } | Inst::WaitOn { sig } | Inst::WaitEdge { sig, .. } => {
                f(sig, Use, s)
            }
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
