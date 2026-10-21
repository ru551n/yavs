//! Programs, functions and their side tables (ir.md 1.1, 1.4, 1.6, 1.7).

use yavs_diag::Span;
use yavs_ffi::{CType, Signature};

use crate::{
    BlockId, CaseTableId, CheckMsgId, ConstId, CovItem, ForeignId, FuncId, IdVec, Inst, NativeId,
    Refl, ReflId, Reg, RegTy, StmtId, Ty, TyId, Width,
};

/// A whole lowered design: immutable and `Send + Sync` once built.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, Default)]
pub struct Program {
    #[serde(with = "funcs")]
    pub funcs: IdVec<FuncId, Func>,
    pub types: IdVec<TyId, Ty>,
    pub consts: IdVec<ConstId, ConstData>,
    /// Global statement table: `StmtId` is `yavs_kernel::StmtId`.
    pub stmts: IdVec<StmtId, StmtInfo>,
    pub natives: IdVec<NativeId, NativeRef>,
    pub foreign: IdVec<ForeignId, ForeignRef>,
    pub cov: IdVec<CovItem, CovInfo>,
    pub checks: IdVec<CheckMsgId, CheckMsg>,
    pub cases: IdVec<CaseTableId, CaseTable>,
    /// Subtype descriptions for `std.reflection` mirrors (`yavs.attr.reflect`).
    pub reflect: IdVec<ReflId, Refl>,
    /// Root map of the shared variables area (`AddrSpace::Shared`), for the collection of
    /// the shared heap (#303).
    pub shared_roots: Vec<(u32, RootKind)>,
    /// Cache of the optimiser (`opt.rs`); leave it to `Default`.
    #[serde(skip)]
    pub layouts: crate::opt::LayoutCache,
    /// Functions from this id on were not verified when the program was built: [`Program::func`]
    /// verifies each when it hands it out first (`None`: all were, or a stored program).
    #[serde(skip)]
    #[doc(hidden)]
    pub lazy_verify: Option<u32>,
    /// Optimised copies an earlier run of this program made ([`Program::load_optimized`]).
    #[serde(skip)]
    #[doc(hidden)]
    pub stored_opt: StoredOpt,
}

/// An optimised copy of a function, serialised (see [`Program::export_optimized`]).
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
pub struct OptCopy {
    pub func: FuncId,
    pub bytes: yavs_diag::Slice,
}

/// Serialised optimised copies by function id, used instead of running the optimiser. A copy
/// of the program starts without them.
#[derive(Default, Debug)]
pub struct StoredOpt(std::sync::Mutex<std::collections::HashMap<u32, yavs_diag::Slice>>);

impl Clone for StoredOpt {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl StoredOpt {
    fn map(&self) -> std::sync::MutexGuard<'_, std::collections::HashMap<u32, yavs_diag::Slice>> {
        self.0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

impl Program {
    /// The function as it executes: its code optimised (see `opt`), the first time asked for.
    /// `funcs` holds the code as lowering emitted it. Both are the same function: statements,
    /// frame, parameters and flags agree, instruction numbers, blocks and resume points do not.
    #[inline]
    pub fn func(&self, f: FuncId) -> Option<&Func> {
        let raw = self.funcs.get(f)?;
        match raw.opt.get() {
            Some(Some(o)) => Some(o),
            Some(None) => Some(raw.full()),
            None => Some(self.optimized(raw, f)),
        }
    }

    #[cold]
    #[inline(never)]
    fn optimized<'a>(&'a self, raw: &'a Func, f: FuncId) -> &'a Func {
        raw.opt
            .get_or_init(|| {
                if self.lazy_verify.is_some_and(|from| f.0 >= from) {
                    let mut errs = Vec::new();
                    crate::verify::verify_func(self, f, raw.full(), &mut errs);
                    // A producer bug: the kernel turns the panic into an internal error.
                    assert!(
                        errs.is_empty(),
                        "internal error: {}; this is a bug in yavs",
                        errs[0]
                    );
                }
                self.stored_copy(f)
                    .or_else(|| crate::opt::optimize_func(self, f))
            })
            .as_deref()
            .unwrap_or_else(|| raw.full())
    }

    /// The optimised copy of `f` an earlier run stored, if it is still the one this run would make.
    fn stored_copy(&self, f: FuncId) -> Option<Box<Func>> {
        if !crate::opt::copies_are_portable() {
            return None;
        }
        let bytes = self.stored_opt.map().remove(&f.0)?;
        postcard::from_bytes::<Func>(bytes.get()).ok().map(Box::new)
    }

    /// Make the optimised copies `copies` (from [`Program::export_optimized`] of a run of the same
    /// program) the ones [`Program::func`] hands out, instead of optimising again (#479).
    pub fn load_optimized(&self, copies: Vec<OptCopy>) {
        if crate::opt::copies_are_portable() {
            self.stored_opt
                .map()
                .extend(copies.into_iter().map(|c| (c.func.0, c.bytes)));
        }
    }

    /// How many copies [`Program::export_optimized`] would return, without making them.
    pub fn optimized_count(&self) -> usize {
        if !crate::opt::copies_are_portable() {
            return 0;
        }
        let made = self
            .funcs
            .values()
            .iter()
            .filter(|f| matches!(f.opt.get(), Some(Some(_))))
            .count();
        made + self.stored_opt.map().len()
    }

    /// The optimised copies made so far of the functions `want` says yes to, serialised (not the
    /// loaded ones not used yet), by function id.
    pub fn export_made(&self, mut want: impl FnMut(FuncId) -> bool) -> Vec<OptCopy> {
        if !crate::opt::copies_are_portable() {
            return Vec::new();
        }
        self.funcs
            .iter()
            .filter_map(|(id, f)| match f.opt.get() {
                Some(Some(o)) if want(id) => postcard::to_stdvec(&**o).ok().map(|bytes| OptCopy {
                    func: id,
                    bytes: bytes.into(),
                }),
                _ => None,
            })
            .collect()
    }

    /// The optimised copies made so far and the loaded ones not used yet, serialised, by function
    /// id. Empty when copies are not portable (the pass is off, a debugger keeps variables).
    pub fn export_optimized(&self) -> Vec<OptCopy> {
        if !crate::opt::copies_are_portable() {
            return Vec::new();
        }
        let mut out: Vec<OptCopy> = self
            .funcs
            .iter()
            .filter_map(|(id, f)| match f.opt.get() {
                Some(Some(o)) => postcard::to_stdvec(&**o).ok().map(|bytes| OptCopy {
                    func: id,
                    bytes: bytes.into(),
                }),
                _ => None,
            })
            .collect();
        out.extend(self.stored_opt.map().iter().map(|(&f, b)| OptCopy {
            func: FuncId(f),
            bytes: b.clone(),
        }));
        out.sort_by_key(|c| c.func.0);
        out
    }

    /// Verify the functions not verified yet (see `lazy_verify`), all of them; afterwards
    /// [`Program::func`] verifies nothing.
    pub fn verify_pending(&mut self) -> Result<(), Vec<crate::VerifyError>> {
        let Some(from) = self.lazy_verify else {
            return Ok(());
        };
        let errs = crate::verify::verify(self, from as usize);
        if errs.is_empty() {
            self.lazy_verify = None;
            Ok(())
        } else {
            Err(errs)
        }
    }

    /// The process must run serially (ir.md 3.5): it or a subprogram it reaches touches
    /// shared state.
    pub fn is_serial(&self, f: FuncId) -> bool {
        self.funcs
            .get(f)
            .is_some_and(|f| f.flags.contains(FuncFlags::TOUCHES_SHARED))
    }

    /// The process has one suspension point: one `Suspend` and no call of a `MAY_SUSPEND`
    /// procedure (a sensitivity list, or one wait statement). It orders the processes of a
    /// delta (#252, yavs-kernel DESIGN.md "Parallel evaluation").
    pub fn single_suspend(&self, f: FuncId) -> bool {
        // The lowered code: the optimiser may copy a suspension point (`opt/memo.rs`).
        self.funcs.get(f).map(Func::full).is_some_and(|f| {
            f.resume.len() == 1 && f.code.iter().any(|i| matches!(i, Inst::Suspend { .. }))
        })
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FuncKind {
    Process,
    Procedure,
    Function,
    /// A design unit's elaboration body (run by yavs-elab without a kernel).
    Elab,
    /// An elaboration-time expression (generic actual, constraint, initial value).
    Thunk,
}

/// Function attributes computed by `ProgramBuilder::finish`.
#[derive(
    serde::Serialize, serde::Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq, Hash,
)]
pub struct FuncFlags(pub u8);

impl FuncFlags {
    /// The function or a procedure it calls can execute `Suspend`.
    pub const MAY_SUSPEND: FuncFlags = FuncFlags(1);
    /// Touches shared variables, protected objects, the shared heap or disk files (serial
    /// marking).
    pub const TOUCHES_SHARED: FuncFlags = FuncFlags(2);
    /// Set by lowering: the function is pure.
    pub const PURE: FuncFlags = FuncFlags(4);
    pub const HAS_COVER: FuncFlags = FuncFlags(8);
    /// Set by lowering: an implicit process for a concurrent statement.
    pub const IMPLICIT: FuncFlags = FuncFlags(16);
    /// Set by lowering: a process with a sensitivity list (`process (all)` too): a `wait`
    /// executed in a procedure it calls is a run-time error (LRM 11.3).
    pub const SENS_LIST: FuncFlags = FuncFlags(32);

    pub fn contains(self, o: FuncFlags) -> bool {
        self.0 & o.0 == o.0
    }

    pub fn insert(&mut self, o: FuncFlags) {
        self.0 |= o.0;
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Mode {
    In,
    Out,
    Inout,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ParamClass {
    /// A value (or an address, for composites and by-reference scalars).
    Value,
    /// A signal reference (`Width::Sig`).
    Signal,
}

/// A parameter: the caller's argument register is stored into `slot` of the new frame.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
pub struct Param {
    pub name: String,
    pub slot: u32,
    pub width: Width,
    pub class: ParamClass,
    pub mode: Mode,
    pub ty: Option<TyId>,
}

/// A named object in a frame slot.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
pub struct Local {
    pub name: String,
    pub slot: u32,
    pub size: u32,
    pub ty: TyId,
}

/// What a frame slot holds for the GC (precise roots, rule 6).
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum RootKind {
    /// An 8-byte access handle.
    Handle,
    /// A value of this type at the slot (handles found through its layout).
    Composite(TyId),
    /// An 8-byte address (a parameter passed by reference, the data or descriptor of an
    /// array parameter): the heap object it points into stays.
    Addr,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct Block {
    pub start: u32,
    pub end: u32,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug)]
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
    /// A process: the block its statements start at, after the declarative part (#342).
    pub body: Option<BlockId>,
    /// Instruction count, for the LLVM size threshold.
    pub size: u32,
    pub defined: bool,
    /// Another function names this one as its parent (its frame can be reached by `Up`).
    pub nested: bool,
    /// The optimised copy, made when [`Program::func`] first asks (`None`: the code is as
    /// lowered).
    #[serde(skip)]
    pub opt: std::sync::OnceLock<Option<Box<Func>>>,
    /// The body (`code`, `loc`, `operands`, `resume`) of a function read from a stored program
    /// (see [`lazy_bodies`]): these four are empty here and [`Func::full`] decodes them.
    #[serde(skip)]
    pub(crate) lazy: Option<Box<LazyBody>>,
}

/// A function body not decoded yet.
#[derive(Debug)]
pub(crate) struct LazyBody {
    src: yavs_diag::Slice,
    full: std::sync::OnceLock<Box<Func>>,
}

impl Clone for LazyBody {
    fn clone(&self) -> Self {
        let full = std::sync::OnceLock::new();
        if let Some(f) = self.full.get() {
            let _ = full.set(f.clone());
        }
        LazyBody {
            src: self.src.clone(),
            full,
        }
    }
}

/// The part of a function that [`funcs`] stores apart from the rest.
#[derive(serde::Serialize, serde::Deserialize)]
struct Body {
    code: Vec<Inst>,
    loc: Vec<StmtId>,
    operands: Vec<Reg>,
    resume: Vec<u32>,
}

#[derive(serde::Serialize)]
struct BodyRef<'a> {
    code: &'a [Inst],
    loc: &'a [StmtId],
    operands: &'a [Reg],
    resume: &'a [u32],
}

thread_local! {
    static LAZY_BODIES: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Run `f`, which deserializes a [`Program`], so that the programs it reads keep the bodies of
/// their functions as bytes and decode one when [`Program::func`] first hands it out (a run
/// calls a fraction of the functions of a lowered design). The bytes should come from a
/// [`yavs_diag::sharing`] buffer, else every body is copied once. Without this the bodies are
/// decoded with the program.
pub fn lazy_bodies<R>(f: impl FnOnce() -> R) -> R {
    let before = LAZY_BODIES.with(|l| l.replace(true));
    let r = f();
    LAZY_BODIES.with(|l| l.set(before));
    r
}

/// How a program stores its functions: the function without its body, then the body as one
/// byte string, so that a reader can leave the body undecoded.
mod funcs {
    use serde::{Deserialize, Deserializer, Serializer};

    use super::{Body, BodyRef, Func, LAZY_BODIES, LazyBody};
    use crate::{FuncId, IdVec};

    pub fn serialize<S: Serializer>(v: &IdVec<FuncId, Func>, s: S) -> Result<S::Ok, S::Error> {
        use serde::ser::{Error, SerializeSeq};
        let mut seq = s.serialize_seq(Some(v.len()))?;
        for f in v.values() {
            let body: yavs_diag::Slice = match &f.lazy {
                Some(l) => l.src.clone(),
                None => postcard::to_stdvec(&BodyRef {
                    code: &f.code,
                    loc: &f.loc,
                    operands: &f.operands,
                    resume: &f.resume,
                })
                .map_err(S::Error::custom)?
                .into(),
            };
            seq.serialize_element(&(f.shell(), body))?;
        }
        seq.end()
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<IdVec<FuncId, Func>, D::Error> {
        use serde::de::Error;
        let lazy = LAZY_BODIES.with(std::cell::Cell::get);
        let stored = Vec::<(Func, yavs_diag::Slice)>::deserialize(d)?;
        let mut out = IdVec::new();
        for (mut f, src) in stored {
            if lazy {
                f.lazy = Some(Box::new(LazyBody {
                    src,
                    full: Default::default(),
                }));
            } else {
                let b: Body = postcard::from_bytes(src.get()).map_err(D::Error::custom)?;
                (f.code, f.loc, f.operands, f.resume) = (b.code, b.loc, b.operands, b.resume);
            }
            out.push(f);
        }
        Ok(out)
    }
}

impl Func {
    /// A copy without code, locations, operands and optimised copy.
    pub(crate) fn shell(&self) -> Func {
        Func {
            name: self.name.clone(),
            kind: self.kind,
            mangled: self.mangled.clone(),
            parent: self.parent,
            params: self.params.clone(),
            locals: self.locals.clone(),
            frame: self.frame,
            roots: self.roots.clone(),
            reg_ty: self.reg_ty.clone(),
            blocks: self.blocks.clone(),
            code: Vec::new(),
            loc: Vec::new(),
            operands: Vec::new(),
            resume: Vec::new(),
            flags: self.flags,
            body: self.body,
            size: self.size,
            defined: self.defined,
            nested: self.nested,
            opt: Default::default(),
            lazy: None,
        }
    }

    /// The function with its body in place: `self`, or for a function whose body is still
    /// bytes ([`lazy_bodies`]) a copy with the body decoded, made once. Everything that reads
    /// `code`, `loc`, `operands` or `resume` of a function out of [`Program::funcs`] asks this.
    pub fn full(&self) -> &Func {
        let Some(l) = &self.lazy else {
            return self;
        };
        l.full.get_or_init(|| {
            // The checksum of the entry matched, so a body that does not decode is a producer bug.
            let b: Body = postcard::from_bytes(l.src.get())
                .unwrap_or_else(|e| panic!("internal error: corrupt function body: {e}"));
            let mut f = self.shell();
            (f.code, f.loc, f.operands, f.resume) = (b.code, b.loc, b.operands, b.resume);
            Box::new(f)
        })
    }

    pub fn nregs(&self) -> u32 {
        self.reg_ty.len() as u32
    }

    pub fn regs(&self, r: crate::Regs) -> Option<&[Reg]> {
        self.operands
            .get(r.start as usize..r.start as usize + r.len as usize)
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
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

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
pub struct StmtInfo {
    pub span: Span,
    pub func: FuncId,
    pub kind: StmtKind,
    pub parent: Option<StmtId>,
}

/// Constant bytes as one block in a serialised program, not as a sequence of numbers (the
/// constant pool of a verification library is megabytes; the wire format is the same).
pub mod bytes {
    use serde::{Deserializer, Serializer, de};

    pub fn serialize<S: Serializer>(v: &[u8], s: S) -> Result<S::Ok, S::Error> {
        s.serialize_bytes(v)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<u8>, D::Error> {
        struct V;
        impl<'de> de::Visitor<'de> for V {
            type Value = Vec<u8>;

            fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
                f.write_str("bytes")
            }

            fn visit_bytes<E>(self, v: &[u8]) -> Result<Vec<u8>, E> {
                Ok(v.to_vec())
            }
        }
        d.deserialize_bytes(V)
    }
}

/// Compact constant data (rule 12).
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
pub enum ConstData {
    Bytes(#[serde(with = "bytes")] Vec<u8>),
    /// `n` copies of one element.
    Repeat {
        #[serde(with = "bytes")]
        elem: Vec<u8>,
        n: u64,
    },
    /// Runs of `(count, element)`.
    Runs(Vec<(u64, Vec<u8>)>),
    /// A logic array as runs of `(count, std_ulogic code)`, element 0 first; expands to
    /// bit-planes.
    Logic(Vec<(u64, u8)>),
}

impl ConstData {
    /// The bytes `off..off + out.len()` of the constant as expanded, without expanding it
    /// (`false` when out of range or for `Logic`, whose bytes are bit-planes). The way for
    /// code that reads pool bytes at compile time (the optimiser) to see a compact constant.
    pub fn read(&self, off: usize, out: &mut [u8]) -> bool {
        let end = off.saturating_add(out.len());
        match self {
            ConstData::Bytes(b) => b.get(off..end).map(|s| out.copy_from_slice(s)).is_some(),
            ConstData::Repeat { elem, n } => {
                let total = (elem.len() as u64).saturating_mul(*n);
                if elem.is_empty() || end as u64 > total {
                    return false;
                }
                for (i, o) in out.iter_mut().enumerate() {
                    *o = elem[(off + i) % elem.len()];
                }
                true
            }
            ConstData::Runs(runs) => {
                let mut start = 0u64;
                let mut at = off as u64;
                let mut i = 0;
                for (n, e) in runs {
                    let len = n.saturating_mul(e.len() as u64);
                    // Bytes still wanted from this run, if `at` falls inside it.
                    while i < out.len() && at >= start && at < start + len {
                        out[i] = e[((at - start) % e.len() as u64) as usize];
                        i += 1;
                        at += 1;
                    }
                    start += len;
                }
                i == out.len()
            }
            ConstData::Logic(_) => false,
        }
    }
}

/// A native subprogram: a `lib/NATIVE.md` symbol. Implicit-family natives
/// (`yavs.implicit.<family>.<type>`) and attribute natives (`yavs.attr.<name>`) also name the
/// type they operate on.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct NativeRef {
    pub symbol: String,
    pub ty: Option<TyId>,
}

/// How one VHDL argument of a foreign call is passed.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
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
    /// A signal parameter of a VHPI model (never a C call): `cells` registers, the signal
    /// reference, then for `out` the caller's driver slot, then the descriptor of an array.
    Signal { out: bool, cells: u8 },
}

impl ForeignArg {
    /// The registers the argument takes in a `CallForeign`.
    pub fn cells(self) -> usize {
        match self {
            ForeignArg::Signal { cells, .. } => cells.into(),
            _ => 1,
        }
    }
}

/// A VHPIDIRECT subprogram (ir.md 1.7). The function pointer is bound outside the IR (the
/// elaborator looks the symbol up); the program only describes the call.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct ForeignRef {
    pub name: String,
    pub symbol: String,
    pub args: Vec<ForeignArg>,
    pub ret: Option<CType>,
    /// `Some` for a `'FOREIGN "VHPI library model"` subprogram: the plugin's registered model
    /// is called instead of a C function (`symbol` is the model name).
    pub model: Option<VhpiModel>,
}

/// The parameter mode of a VHPI foreign subprogram's formal.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum ForeignMode {
    In,
    Out,
    InOut,
}

/// One formal of a VHPI foreign subprogram: its name, mode and type (the array type for an
/// array formal, else the scalar's base type).
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct VhpiParam {
    pub name: String,
    pub mode: ForeignMode,
    pub ty: TyId,
}

/// What a VHPI foreign call needs beyond the C-shaped [`ForeignRef`] fields.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct VhpiModel {
    pub library: String,
    pub model: String,
    pub params: Vec<VhpiParam>,
    /// The result type: a scalar base type, or a one-dimensional array of scalars
    /// (`ret_index` then says how it is indexed).
    pub ret: Option<TyId>,
    pub ret_index: Option<VhpiIndex>,
}

/// The index range of a VHPI function's array result: the left bound and direction, and the
/// length when the result subtype is constrained (else the model sets it with
/// `vhpiSizeConstraint`).
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct VhpiIndex {
    pub left: i64,
    pub downto: bool,
    pub len: Option<u64>,
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
                    ForeignArg::Signal { .. } => vec![],
                })
                .collect(),
            ret: self.ret,
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum CovKind {
    Statement,
    /// Arm `n` of a branching statement, in source order (an `if` without `else` and a
    /// conditional assignment without a final `else` get one more arm: no choice taken).
    Branch(u32),
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
pub struct CovInfo {
    pub stmt: StmtId,
    pub kind: CovKind,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Dir {
    To,
    Downto,
}

/// The static part of a check's message: what is checked (`"index of s"`) and the direction
/// the bounds are shown in.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct CheckMsg {
    pub what: String,
    pub dir: Dir,
    /// The subtype checked against, as messages name it (`INTEGER`, `PCT`); may be empty.
    pub subtype: String,
}

/// `Case` choices as sorted, non-overlapping inclusive ranges.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct CaseTable {
    pub arms: Vec<(i64, i64, BlockId)>,
    pub default: BlockId,
}

#[cfg(test)]
mod const_tests {
    use super::ConstData;

    fn read(c: &ConstData, off: usize, n: usize) -> Option<Vec<u8>> {
        let mut out = vec![0; n];
        c.read(off, &mut out).then_some(out)
    }

    #[test]
    fn compact_constants_read_in_place() {
        let rep = ConstData::Repeat {
            elem: vec![1, 2, 3],
            n: 4,
        };
        assert_eq!(read(&rep, 2, 5), Some(vec![3, 1, 2, 3, 1]));
        assert_eq!(read(&rep, 10, 2), Some(vec![2, 3]));
        assert_eq!(read(&rep, 11, 2), None);
        let runs = ConstData::Runs(vec![(2, vec![9]), (3, vec![1, 2]), (1, vec![7, 8])]);
        // 9 9 1 2 1 2 1 2 7 8
        assert_eq!(read(&runs, 0, 10), Some(vec![9, 9, 1, 2, 1, 2, 1, 2, 7, 8]));
        assert_eq!(read(&runs, 1, 4), Some(vec![9, 1, 2, 1]));
        assert_eq!(read(&runs, 7, 3), Some(vec![2, 7, 8]));
        assert_eq!(read(&runs, 8, 3), None);
        assert_eq!(read(&ConstData::Bytes(vec![5, 6]), 1, 1), Some(vec![6]));
        assert_eq!(read(&ConstData::Logic(vec![(4, 3)]), 0, 1), None);
    }
}

#[cfg(test)]
mod lazy_verify_tests {
    use yavs_diag::{FileId, Span};

    use crate::{Inst, ProgramBuilder, StmtKind};

    /// A function that was not verified when the program was built (a release build) is
    /// verified when first handed out; a stored program (`verify_pending`) is not verified again.
    #[test]
    fn broken_function_is_caught_at_first_use() {
        let mut pb = ProgramBuilder::new();
        let f = pb.declare("f", crate::FuncKind::Procedure, None);
        let mut fb = pb.define(f);
        fb.stmt(StmtKind::Null, Span::new(FileId(0), 0, 1));
        fb.emit(Inst::Ret { v: None });
        fb.finish();
        let mut prog = pb.finish().expect("well formed");
        assert!(prog.lazy_verify.is_none() == cfg!(debug_assertions));
        prog.lazy_verify = Some(0);
        assert!(prog.verify_pending().is_ok());
        assert!(prog.lazy_verify.is_none());
        // Break it behind the builder's back.
        prog.funcs.get_mut(f).expect("f").code.clear();
        assert!(prog.verify_pending().is_ok(), "nothing pending");
        prog.lazy_verify = Some(0);
        let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            prog.func(f);
        }));
        assert!(caught.is_err(), "the broken function must not run");
        assert!(prog.verify_pending().is_err());
    }
}
