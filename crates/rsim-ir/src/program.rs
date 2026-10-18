//! Programs, functions and their side tables (ir.md 1.1, 1.4, 1.6, 1.7).

use rsim_diag::Span;
use rsim_ffi::{CType, Signature};

use crate::{
    BlockId, CaseTableId, CheckMsgId, ConstId, CovItem, ForeignId, FuncId, IdVec, Inst, NativeId,
    Reg, RegTy, StmtId, Ty, TyId, Width,
};

/// A whole lowered design: immutable and `Send + Sync` once built.
#[derive(Debug, Default)]
pub struct Program {
    pub funcs: IdVec<FuncId, Func>,
    pub types: IdVec<TyId, Ty>,
    pub consts: IdVec<ConstId, ConstData>,
    /// Global statement table: `StmtId` is `rsim_kernel::StmtId`.
    pub stmts: IdVec<StmtId, StmtInfo>,
    pub natives: IdVec<NativeId, NativeRef>,
    pub foreign: IdVec<ForeignId, ForeignRef>,
    pub cov: IdVec<CovItem, CovInfo>,
    pub checks: IdVec<CheckMsgId, CheckMsg>,
    pub cases: IdVec<CaseTableId, CaseTable>,
}

impl Program {
    pub fn func(&self, f: FuncId) -> Option<&Func> {
        self.funcs.get(f)
    }

    /// The process must run serially (ir.md 3.5): it or a subprogram it reaches touches
    /// shared state.
    pub fn is_serial(&self, f: FuncId) -> bool {
        self.funcs
            .get(f)
            .is_some_and(|f| f.flags.contains(FuncFlags::TOUCHES_SHARED))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FuncKind {
    Process,
    Procedure,
    Function,
    /// A design unit's elaboration body (run by rsim-elab without a kernel).
    Elab,
    /// An elaboration-time expression (generic actual, constraint, initial value).
    Thunk,
}

/// Function attributes computed by `ProgramBuilder::finish`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct FuncFlags(pub u8);

impl FuncFlags {
    /// The function or a procedure it calls can execute `Suspend`.
    pub const MAY_SUSPEND: FuncFlags = FuncFlags(1);
    /// Touches shared variables, protected objects or the shared heap (serial marking).
    pub const TOUCHES_SHARED: FuncFlags = FuncFlags(2);
    /// Set by lowering: the function is pure.
    pub const PURE: FuncFlags = FuncFlags(4);
    pub const HAS_COVER: FuncFlags = FuncFlags(8);
    /// Set by lowering: an implicit process for a concurrent statement.
    pub const IMPLICIT: FuncFlags = FuncFlags(16);

    pub fn contains(self, o: FuncFlags) -> bool {
        self.0 & o.0 == o.0
    }

    pub fn insert(&mut self, o: FuncFlags) {
        self.0 |= o.0;
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    In,
    Out,
    Inout,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ParamClass {
    /// A value (or an address, for composites and by-reference scalars).
    Value,
    /// A signal reference (`Width::Sig`).
    Signal,
}

/// A parameter: the caller's argument register is stored into `slot` of the new frame.
#[derive(Clone, Debug, PartialEq)]
pub struct Param {
    pub name: String,
    pub slot: u32,
    pub width: Width,
    pub class: ParamClass,
    pub mode: Mode,
    pub ty: Option<TyId>,
}

/// A named object in a frame slot.
#[derive(Clone, Debug, PartialEq)]
pub struct Local {
    pub name: String,
    pub slot: u32,
    pub size: u32,
    pub ty: TyId,
}

/// What a frame slot holds for the GC (precise roots, rule 6).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RootKind {
    /// An 8-byte access handle.
    Handle,
    /// A value of this type at the slot (handles found through its layout).
    Composite(TyId),
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Block {
    pub start: u32,
    pub end: u32,
}

#[derive(Clone, Debug)]
pub struct Func {
    /// Original spelling (rule 8).
    pub name: String,
    pub kind: FuncKind,
    /// Unique per generic-instance identity (rule 4).
    pub mangled: String,
    /// Lexically enclosing subprogram, for `AddrSpace::Up` and static links.
    pub parent: Option<FuncId>,
    pub params: Vec<Param>,
    /// Named objects in the frame (parameters and variables), for debuggers.
    pub locals: Vec<Local>,
    /// Frame size in bytes.
    pub frame: u32,
    pub roots: Vec<(u32, RootKind)>,
    pub reg_ty: Vec<RegTy>,
    pub blocks: IdVec<BlockId, Block>,
    pub code: Vec<Inst>,
    /// Statement of each instruction, parallel to `code`.
    pub loc: Vec<StmtId>,
    /// Operand lists (`Regs`).
    pub operands: Vec<Reg>,
    /// Resume points: the instruction after each `Suspend` and each call of a `MAY_SUSPEND`
    /// procedure, in code order. `Suspend::resume` indexes this.
    pub resume: Vec<u32>,
    pub flags: FuncFlags,
    /// Instruction count, for the LLVM size threshold.
    pub size: u32,
    pub defined: bool,
}

impl Func {
    pub fn nregs(&self) -> u32 {
        self.reg_ty.len() as u32
    }

    pub fn regs(&self, r: crate::Regs) -> Option<&[Reg]> {
        self.operands
            .get(r.start as usize..r.start as usize + r.len as usize)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum StmtKind {
    Process,
    Subprogram,
    VarAssign,
    SigAssign,
    Wait,
    If,
    Case,
    Loop,
    Exit,
    Next,
    Call,
    Return,
    Report,
    Assert,
    Null,
    Decl,
    Implicit,
}

#[derive(Clone, Debug, PartialEq)]
pub struct StmtInfo {
    pub span: Span,
    pub func: FuncId,
    pub kind: StmtKind,
    pub parent: Option<StmtId>,
}

/// Compact constant data (rule 12).
#[derive(Clone, Debug, PartialEq)]
pub enum ConstData {
    Bytes(Vec<u8>),
    /// `n` copies of one element.
    Repeat {
        elem: Vec<u8>,
        n: u64,
    },
    /// Runs of `(count, element)`.
    Runs(Vec<(u64, Vec<u8>)>),
    /// A logic array as runs of `(count, std_ulogic code)`, element 0 first; expands to
    /// bit-planes.
    Logic(Vec<(u64, u8)>),
}

/// A native subprogram: a `lib/NATIVE.md` symbol. Implicit-family natives
/// (`rsim.implicit.<family>.<type>`) and attribute natives (`rsim.attr.<name>`) also name the
/// type they operate on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NativeRef {
    pub symbol: String,
    pub ty: Option<TyId>,
}

/// How one VHDL argument of a foreign call is passed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForeignArg {
    /// A scalar by value; the register is converted to the C type (range-checked).
    Value(CType),
    /// A scalar `out`/`inout`: the register holds the address of the VHDL value (`Width`),
    /// C gets a pointer to a C-typed temp that is written back after the call.
    ScalarRef(CType, Width),
    /// An array of scalars of type `ty`: the register holds the address of a fat pair
    /// `[data, dope]` (constrained arrays too). C gets a pointer to a C-layout copy (one C
    /// element per VHDL element, left to right; logic elements as one code byte), followed
    /// by `lengths` `int64_t` lengths (one per dimension for an unconstrained formal, else 0).
    /// `copy_back` (`out`/`inout`) writes the C array back after the call.
    Array {
        ty: TyId,
        lengths: u8,
        copy_back: bool,
    },
    /// A record of scalars of type `ty` (static layout): the register holds its address, C
    /// gets a pointer to a naturally aligned C struct.
    Record { ty: TyId, copy_back: bool },
    /// An access value: C gets a pointer to the designated object's data (after the
    /// descriptor for an array object) in VHDL layout, or NULL.
    Access,
}

/// A VHPIDIRECT subprogram (ir.md 1.7). The function pointer is bound outside the IR (the
/// elaborator looks the symbol up); the program only describes the call.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ForeignRef {
    pub name: String,
    pub symbol: String,
    pub args: Vec<ForeignArg>,
    pub ret: Option<CType>,
}

impl ForeignRef {
    pub fn signature(&self) -> Signature {
        Signature {
            params: self
                .args
                .iter()
                .flat_map(|a| match *a {
                    ForeignArg::Value(c) => vec![c],
                    ForeignArg::Array { lengths, .. } => {
                        let mut v = vec![CType::Ptr];
                        v.extend(std::iter::repeat_n(CType::I64, lengths.into()));
                        v
                    }
                    ForeignArg::ScalarRef(..) | ForeignArg::Record { .. } | ForeignArg::Access => {
                        vec![CType::Ptr]
                    }
                })
                .collect(),
            ret: self.ret,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CovKind {
    Statement,
    Branch,
}

#[derive(Clone, Debug, PartialEq)]
pub struct CovInfo {
    pub stmt: StmtId,
    pub kind: CovKind,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Dir {
    To,
    Downto,
}

/// The static part of a check's message: what is checked (`"index of s"`) and the direction
/// the bounds are shown in.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CheckMsg {
    pub what: String,
    pub dir: Dir,
    /// The subtype checked against, as messages name it (`INTEGER`, `PCT`); may be empty.
    pub subtype: String,
}

/// `Case` choices as sorted, non-overlapping inclusive ranges.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CaseTable {
    pub arms: Vec<(i64, i64, BlockId)>,
    pub default: BlockId,
}
