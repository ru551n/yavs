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
    /// The condition trees of `Inst::CoverCond`.
    pub cov_conds: Vec<CovCond>,
    pub checks: IdVec<CheckMsgId, CheckMsg>,
    pub cases: IdVec<CaseTableId, CaseTable>,
    /// Result tables of pure functions of one small discrete argument, by function id
    /// (`casetab.rs`).
    pub tabs: std::collections::BTreeMap<u32, crate::CaseTab>,
    /// Subtype descriptions for `std.reflection` mirrors (`yavs.attr.reflect`).
    pub reflect: IdVec<ReflId, Refl>,
    /// Root map of the shared variables area (`AddrSpace::Shared`), for the collection of
    /// the shared heap (#303).
    pub shared_roots: Vec<(u32, RootKind)>,
    /// Cache of the optimiser (`opt.rs`); leave it to `Default`.
    #[serde(skip)]
    pub layouts: crate::opt::LayoutCache,
    /// Inlinable bodies of the functions asked about (`opt/inline.rs`); leave it to `Default`.
    #[serde(skip)]
    #[doc(hidden)]
    pub fwd: crate::opt::FwdCache,
    /// Functions from this id on were not verified when the program was built: [`Program::func`]
    /// verifies each when it hands it out first (`None`: all were, or a stored program).
    #[serde(skip)]
    #[doc(hidden)]
    pub lazy_verify: Option<u32>,
    /// Optimised copies an earlier run of this program made ([`Program::load_optimized`]).
    #[serde(skip)]
    #[doc(hidden)]
    pub stored_opt: StoredOpt,
    /// Specialised copies of processes made while the program runs (`copies.rs`); a copy of the
    /// program starts without them.
    #[serde(skip)]
    pub copies: crate::Copies,
    /// Functions elaboration-time code asked for often ([`ElabRaw`]).
    #[serde(skip)]
    #[doc(hidden)]
    pub hot: HotList,
    /// The key of the stage-cache entry this program was read from or stored to (the lowering
    /// key: the same key is the same program, ids and all); `None` for a program not cached.
    /// Compiled code is found by it before it is generated (yavs-llvm `objcache`, #785).
    #[serde(skip)]
    pub identity: Option<String>,
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
        self.func_inline(f)
    }

    /// [`Program::func`], inlined: for a caller on every activation of a process.
    #[inline(always)]
    pub fn func_inline(&self, f: FuncId) -> Option<&Func> {
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
        if ELAB_RAW.with(std::cell::Cell::get) != 0
            && matches!(raw.kind, FuncKind::Function | FuncKind::Procedure)
            && !raw.flags.contains(FuncFlags::MAY_SUSPEND)
            && !raw.spare
        {
            let n = raw
                .fetches
                .0
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            if n == 0 {
                self.verify_lazily(f, raw);
            }
            // A stored optimised copy costs a decode, less than running the lowered code.
            if n > 0 || !self.stored_opt.map().contains_key(&f.0) {
                if (n + 1).is_multiple_of(HOT) {
                    self.hot
                        .0
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner)
                        .push(f);
                }
                return raw.full();
            }
        }
        let o = raw
            .opt
            .get_or_init(|| {
                // A specialised copy (`copies.rs`) is made from a function already verified.
                if let Some(c) = self.copies.get(f) {
                    return self.optimize_copy_of(c);
                }
                if raw.spare {
                    return None;
                }
                // Verified already when elaboration-time code ran it as lowered.
                if raw.fetches.0.load(std::sync::atomic::Ordering::Relaxed) == 0 {
                    self.verify_lazily(f, raw);
                }
                let o = self.stored_copy(f).or_else(|| {
                    OPTIMISED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    crate::opt::optimize_func(self, f)
                });
                if raw.kind == FuncKind::Process {
                    self.copies
                        .shared(o.as_ref().map_or(raw.full().code.len(), |o| o.code.len()));
                }
                o
            })
            .as_deref()
            .unwrap_or_else(|| raw.full());
        // Debug aid: the finished code of each function, as it first runs.
        if std::env::var_os("YAVS_DUMPIR").is_some() {
            eprintln!("{}", self.dump_func(f));
        }
        o
    }

    /// Does `f` run as lowered because elaboration-time code asked for it ([`ElabRaw`])?
    pub fn runs_raw(&self, f: FuncId) -> bool {
        self.funcs.get(f).is_some_and(|r| {
            r.opt.get().is_none() && matches!(r.kind, FuncKind::Function | FuncKind::Procedure)
        })
    }

    /// A jump back of function `f`: `None` unless it runs as lowered; `Some(true)` (and `f` listed
    /// to be optimised before the next call) when its loops have taken [`BACK_BUDGET`] of them.
    pub fn back_edge(&self, f: FuncId) -> Option<bool> {
        use std::sync::atomic::Ordering::Relaxed;
        let r = self.funcs.get(f)?;
        if r.opt.get().is_some() || !matches!(r.kind, FuncKind::Function | FuncKind::Procedure) {
            return None;
        }
        let n = r.fetches.1.load(Relaxed).wrapping_add(1);
        r.fetches.1.store(n, Relaxed);
        if !n.is_multiple_of(BACK_BUDGET) {
            return Some(false);
        }
        self.hot
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .push(f);
        Some(true)
    }

    /// Verify `raw` (function `f`) when the program was not verified as a whole (`lazy_verify`).
    fn verify_lazily(&self, f: FuncId, raw: &Func) {
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
    }

    /// The functions elaboration-time code asked for [`HOT`] times since the last call, to
    /// optimise before the next call ([`ElabRaw`]).
    pub fn take_hot(&self) -> Vec<FuncId> {
        std::mem::take(
            &mut *self
                .hot
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner),
        )
    }

    /// Is the optimised copy of `f` valid only while nothing outside writes process variables
    /// ([`crate::opt::external_variables`])? Such a copy is neither stored nor loaded.
    fn private_copy(&self, f: FuncId) -> bool {
        crate::opt::external()
            && self
                .funcs
                .get(f)
                .is_some_and(|x| x.kind == FuncKind::Process)
    }

    /// The optimised copy of `f` an earlier run stored, if it is still the one this run would make.
    fn stored_copy(&self, f: FuncId) -> Option<Box<Func>> {
        if !crate::opt::copies_are_portable() {
            return None;
        }
        if self.private_copy(f) {
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
            .filter(|f| !f.spare && matches!(f.opt.get(), Some(Some(_))))
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
                Some(Some(o)) if !f.spare && want(id) && !self.private_copy(id) => {
                    postcard::to_stdvec(&**o).ok().map(|bytes| OptCopy {
                        func: id,
                        bytes: bytes.into(),
                    })
                }
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
                Some(Some(o)) if !f.spare && !self.private_copy(id) => {
                    postcard::to_stdvec(&**o).ok().map(|bytes| OptCopy {
                        func: id,
                        bytes: bytes.into(),
                    })
                }
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
        self.funcs.get(self.base_of(f)).is_some_and(|f| {
            f.flags.contains(FuncFlags::TOUCHES_SHARED)
                || f.flags.contains(FuncFlags::TOUCHES_FILES)
        })
    }

    /// Serial because it needs the shared state (shared variables, protected objects, the shared
    /// heap), not only because it touches files: it runs with the `SharedState` of the design
    /// and allocates in its heap.
    pub fn uses_shared_state(&self, f: FuncId) -> bool {
        self.funcs
            .get(self.base_of(f))
            .is_some_and(|f| f.flags.contains(FuncFlags::TOUCHES_SHARED))
    }

    /// The process has one suspension point: one `Suspend` and no call of a `MAY_SUSPEND`
    /// procedure (a sensitivity list, or one wait statement). It orders the processes of a
    /// delta (#252, yavs-kernel DESIGN.md "Parallel evaluation").
    pub fn single_suspend(&self, f: FuncId) -> bool {
        // The lowered code: the optimiser may copy a suspension point (`opt/memo.rs`).
        self.raw(f).is_some_and(|f| {
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
    /// Touches shared variables, protected objects or the shared heap (serial marking; the
    /// process needs the shared state).
    pub const TOUCHES_SHARED: FuncFlags = FuncFlags(2);
    /// Reads or writes disk files (serial marking only: the kernel orders such processes, the
    /// machine needs no shared state).
    pub const TOUCHES_FILES: FuncFlags = FuncFlags(64);
    /// Set by lowering: the function is pure.
    pub const PURE: FuncFlags = FuncFlags(4);
    pub const HAS_COVER: FuncFlags = FuncFlags(8);
    /// Set by lowering: an implicit process for a concurrent statement.
    pub const IMPLICIT: FuncFlags = FuncFlags(16);
    /// Set by lowering: a process with a sensitivity list (`process (all)` too): a `wait`
    /// executed in a procedure it calls is a run-time error (LRM 11.3).
    pub const SENS_LIST: FuncFlags = FuncFlags(32);
    /// A call of the function leaves nothing in the caller's temporary memory: it allocates
    /// none and calls only such functions and natives with a scalar result (`opt::leaves_temps`,
    /// set by `propagate`). Absent in older stores, which reads as "may leave some".
    pub const NO_TEMPS: FuncFlags = FuncFlags(128);

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
    /// The data cell of an array parameter passed by value: the array type (the next parameter
    /// is its descriptor). `ty` stays `None` so the debugger shows no local for the cell.
    pub arr: Option<TyId>,
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

/// A call that was inlined: its statement (of the function that contained the call), the
/// function called, and the call it was itself inlined through (0: none, else 1 and an index
/// in `Func::via_nodes`).
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub struct ViaNode {
    pub site: StmtId,
    pub callee: FuncId,
    pub up: u32,
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
    /// Only in an optimised copy that inlined a call (`opt/inline.rs`): for each instruction,
    /// 0 for the function's own instructions, else 1 and the index in `via_nodes` of the
    /// innermost call it was inlined through. Empty when nothing was inlined. A trace shows an
    /// inlined function as a frame of its own: [`Func::inlined_at`], [`Func::inlined_chain`].
    pub via: Vec<u32>,
    /// The inlined calls: see [`ViaNode`].
    pub via_nodes: Vec<ViaNode>,
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
    /// An empty slot reserved for a specialised copy of a process (`Program::reserve_copies`):
    /// the last functions of a program, never stored.
    #[serde(skip)]
    pub spare: bool,
    /// A function: the base type of its result (`None` for a procedure, a process).
    pub ret_ty: Option<TyId>,
    /// The optimised copy, made when [`Program::func`] first asks (`None`: the code is as
    /// lowered).
    #[serde(skip)]
    pub opt: std::sync::OnceLock<Option<Box<Func>>>,
    /// What a call of this function can be memoised on ([`Program::memo_sig`]), worked out the
    /// first time asked.
    #[serde(skip)]
    pub(crate) memo: std::sync::OnceLock<Option<Box<crate::MemoSig>>>,
    /// The body (`code`, `loc`, `operands`, `resume`) of a function read from a stored program
    /// (see [`lazy_bodies`]): these four are empty here and [`Func::full`] decodes them.
    #[serde(skip)]
    pub(crate) lazy: Option<Box<LazyBody>>,
    /// How often elaboration-time code asked for this function before it was optimised, and how
    /// many jumps back it took as lowered ([`ElabRaw`]).
    #[serde(skip)]
    pub(crate) fetches: Fetches,
}

/// A counter that a clone of its function starts again.
#[derive(Debug, Default)]
pub(crate) struct Fetches(std::sync::atomic::AtomicU32, std::sync::atomic::AtomicU32);

impl Clone for Fetches {
    fn clone(&self) -> Self {
        Self::default()
    }
}

/// The functions elaboration-time code asked for [`HOT`] times (`Program::take_hot`).
#[derive(Debug, Default)]
pub struct HotList(std::sync::Mutex<Vec<FuncId>>);

impl Clone for HotList {
    fn clone(&self) -> Self {
        Self::default()
    }
}

/// Elaboration-time calls run a function as lowered, without the optimiser, until it has been
/// asked for this many times ([`ElabRaw`]).
const HOT: u32 = 64;

/// Jumps back a function that runs as lowered may take (over all its calls) before it is
/// optimised ([`Program::back_edge`]). The optimiser costs about as much as 2000 iterations of
/// a lowered loop save; the iterations run before the switch are repeated when the call restarts.
const BACK_BUDGET: u32 = 2048;

static OPTIMISED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

static RESTARTED: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

/// Count a call abandoned and run again with a function optimised ([`ElabRaw::restartable`]).
pub fn note_restart() {
    RESTARTED.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
}

/// How many calls restarted in this process (a work counter: `YAVS_COUNTERS`).
pub fn restarted_calls() -> u64 {
    RESTARTED.load(std::sync::atomic::Ordering::Relaxed)
}

/// How many functions the optimiser ran on in this process (a work counter: `YAVS_COUNTERS`).
pub fn optimised_functions() -> u64 {
    OPTIMISED.load(std::sync::atomic::Ordering::Relaxed)
}

thread_local! {
    /// 0: off, 1: on, 2: on and a call may not restart ([`ElabRaw::no_restart`]).
    static ELAB_RAW: std::cell::Cell<u8> = const { std::cell::Cell::new(0) };
}

/// While alive, [`Program::func`] hands out a subprogram as lowered instead of optimising it at
/// its first use (the call is a one-shot elaboration-time evaluation: most functions of a
/// library run once or twice there and the optimiser costs more than it saves). Functions asked
/// for [`HOT`] times are listed ([`Program::take_hot`]); the caller optimises them between two
/// calls, when no frame runs their lowered code (`yavs_interp::Runtime::promote_hot`).
///
/// A function whose loops take [`BACK_BUDGET`] jumps back is listed too (`Program::back_edge`), and
/// a call that has only computed may then be run again with it optimised ([`ElabRaw::restartable`]).
///
/// ponytail: the jumps back are counted at the jumps the predecoder leaves to the generic step
/// (`yavs_interp::pre`); a call that printed or touched anything finishes lowered, and an element
/// loop (`Op::Loop`) is not counted.
pub struct ElabRaw(u8);

impl ElabRaw {
    pub fn on() -> ElabRaw {
        ElabRaw(ELAB_RAW.with(|c| c.replace(c.get().max(1))))
    }

    /// Like [`ElabRaw::on`], and no call restarts while it is alive: the last attempt.
    pub fn no_restart() -> ElabRaw {
        ElabRaw(ELAB_RAW.with(|c| c.replace(2)))
    }

    /// Whether the call running now may be abandoned and run again (`Fault::Restart`).
    pub fn restartable() -> bool {
        ELAB_RAW.with(std::cell::Cell::get) == 1
    }
}

impl Drop for ElabRaw {
    fn drop(&mut self) {
        ELAB_RAW.with(|c| c.set(self.0));
    }
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
        let n = v.values().iter().take_while(|f| !f.spare).count();
        let mut seq = s.serialize_seq(Some(n))?;
        for f in &v.values()[..n] {
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
            via: Vec::new(),
            via_nodes: Vec::new(),
            resume: Vec::new(),
            flags: self.flags,
            body: self.body,
            size: self.size,
            defined: self.defined,
            nested: self.nested,
            spare: false,
            ret_ty: self.ret_ty,
            opt: Default::default(),
            memo: Default::default(),
            lazy: None,
            fetches: Default::default(),
        }
    }

    /// An empty function that copies of processes will take the place of.
    pub(crate) fn spare() -> Func {
        Func {
            name: String::new(),
            kind: FuncKind::Process,
            mangled: String::new(),
            parent: None,
            params: Vec::new(),
            locals: Vec::new(),
            frame: 0,
            roots: Vec::new(),
            reg_ty: Vec::new(),
            blocks: IdVec::new(),
            code: Vec::new(),
            loc: Vec::new(),
            operands: Vec::new(),
            via: Vec::new(),
            via_nodes: Vec::new(),
            resume: Vec::new(),
            flags: FuncFlags::default(),
            body: None,
            size: 0,
            defined: false,
            nested: false,
            spare: true,
            ret_ty: None,
            opt: Default::default(),
            memo: Default::default(),
            lazy: None,
            fetches: Default::default(),
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

    /// The call at instruction `pc` was inlined from: the statement of the call and the
    /// function called, when the instruction belongs to an inlined function.
    #[inline]
    pub fn inlined_at(&self, pc: usize) -> Option<(StmtId, FuncId)> {
        self.inlined_chain(pc).next().map(|n| (n.site, n.callee))
    }

    /// The calls instruction `pc` was inlined through, innermost first: the first node's
    /// callee is the function the instruction belongs to and its site a statement of the
    /// next node's callee (of this function for the last), and so on outward.
    pub fn inlined_chain(&self, pc: usize) -> impl Iterator<Item = ViaNode> + '_ {
        let mut at = self.via.get(pc).copied().unwrap_or(0);
        std::iter::from_fn(move || {
            let n = *self.via_nodes.get(at.checked_sub(1)? as usize)?;
            at = n.up;
            Some(n)
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
    /// Row `row` of a condition (`if`, `elsif`, `when`, `while`, `exit`, `next`): row
    /// `2 * i + v` is input `i` at value `v` (focused expression coverage).
    Condition(u32),
    /// The same for a logic expression on the right side of an assignment.
    Expression(u32),
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
pub struct CovInfo {
    pub stmt: StmtId,
    pub kind: CovKind,
    /// A condition or expression row: what it is about.
    pub row: Option<CovRow>,
}

/// The source of a condition or expression row.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
pub struct CovRow {
    /// The whole condition or expression.
    pub whole: Span,
    /// The input.
    pub input: Span,
    /// `'0'` or `'1'` when the row's input at its value decides the result alone (no `xor`
    /// between the input and the result), the result then; `'?'` when it depends on the
    /// other inputs.
    pub result: char,
}

/// A node of a condition tree in postfix order.
#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
pub enum CovNode {
    /// Input `n`.
    Input(u8),
    Not,
    And,
    Or,
    Nand,
    Nor,
    Xor,
    Xnor,
}

/// The tree of one condition: its first row's counter is `first`, the row of input `i` at
/// value `v` is `first + 2 * k + v`, `k` the number of non-constant inputs before `i`.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
pub struct CovCond {
    pub first: CovItem,
    /// The number of inputs, at most [`COV_INPUTS`].
    pub inputs: u8,
    /// Inputs that are constant after elaboration (bit `i`): evaluated, but they have no rows.
    pub consts: u32,
    pub nodes: Vec<CovNode>,
}

/// The most inputs of a condition that is measured (two bits of the state each).
pub const COV_INPUTS: usize = 31;

impl CovCond {
    /// The rows (bit `2 * k + v`, `k` as in [`CovCond`]) one evaluation covers. `state` holds two bits per input:
    /// 0 not evaluated (short-circuit), 1 false, 2 true, 3 another value (nothing is counted
    /// then). Input `i` at value `v` is covered when flipping it alone changes the result
    /// while the other evaluated inputs keep their values; an input not evaluated is masked:
    /// it may take any value.
    pub fn hits(&self, state: u64) -> u64 {
        let n = usize::from(self.inputs);
        let code = |i: usize| (state >> (2 * i)) & 3;
        if (0..n).any(|i| code(i) == 3) {
            return 0;
        }
        let r = self.reach(state, None);
        if r != 1 && r != 2 {
            return 0;
        }
        let mut hits = 0;
        for (k, i) in (0..n).filter(|&i| self.consts >> i & 1 == 0).enumerate() {
            if code(i) != 0 && self.reach(state, Some(i)) & (3 ^ r) != 0 {
                hits |= 1 << (2 * k + usize::from(code(i) == 2));
            }
        }
        hits
    }

    /// The results the tree can have: bit 0 false, bit 1 true. Input `flip` has the other
    /// value, an input not evaluated either.
    fn reach(&self, state: u64, flip: Option<usize>) -> u8 {
        let mut st = [0u8; 64];
        let mut sp = 0;
        for &n in &self.nodes {
            if let CovNode::Input(i) = n {
                let c = (state >> (2 * u32::from(i))) & 3;
                st[sp] = match c {
                    0 => 3,
                    _ if flip == Some(usize::from(i)) => 3 ^ c as u8,
                    c => c as u8,
                };
                sp += 1;
                continue;
            }
            let not = |a: u8| (a & 1) << 1 | (a >> 1) & 1;
            if n == CovNode::Not {
                st[sp - 1] = not(st[sp - 1]);
                continue;
            }
            sp -= 1;
            let (a, b) = (st[sp - 1], st[sp]);
            let and = (a | b) & 1 | a & b & 2;
            let or = a & b & 1 | (a | b) & 2;
            let xor = u8::from((a & b & 1 != 0) || (a & b & 2 != 0))
                | u8::from((a & 1 != 0 && b & 2 != 0) || (a & 2 != 0 && b & 1 != 0)) << 1;
            st[sp - 1] = match n {
                CovNode::And => and,
                CovNode::Nand => not(and),
                CovNode::Or => or,
                CovNode::Nor => not(or),
                CovNode::Xor => xor,
                _ => not(xor),
            };
        }
        st[0]
    }
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
    /// The subtype checked against, as messages name it (`INTEGER`, `PCT`); may be empty. For a
    /// `Length` check it is the lead word of the message instead (`expression length 4 does
    /// not match case choice length 3`), `value` when empty.
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
        // `finish` leaves everything pending unless it verified eagerly (#482); where it did
        // not, the flag is the builder's own, elsewhere the test forces it.
        assert!(prog.lazy_verify.is_none() == crate::build::eager_verify());
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

#[cfg(test)]
mod cov_tests {
    use super::*;

    fn cond(nodes: &[CovNode], inputs: u8) -> CovCond {
        CovCond {
            first: CovItem(0),
            inputs,
            consts: 0,
            nodes: nodes.to_vec(),
        }
    }

    /// State of the inputs `v` (`None`: not evaluated), two bits each.
    fn state(v: &[Option<bool>]) -> u64 {
        v.iter().enumerate().fold(0, |s, (i, x)| {
            s | match x {
                None => 0,
                Some(false) => 1,
                Some(true) => 2,
            } << (2 * i)
        })
    }

    #[test]
    fn and_or_rows() {
        use CovNode::*;
        // a and b
        let c = cond(&[Input(0), Input(1), And], 2);
        // Both true: each input flips the result.
        assert_eq!(c.hits(state(&[Some(true), Some(true)])), 0b1010);
        // a false, b true: a decides (row a=0), b does not.
        assert_eq!(c.hits(state(&[Some(false), Some(true)])), 0b0001);
        // a false, b not evaluated (short-circuit): b is masked, a decides.
        assert_eq!(c.hits(state(&[Some(false), None])), 0b0001);
        // Both false: neither flips alone.
        assert_eq!(c.hits(state(&[Some(false), Some(false)])), 0);
        // a true, b false: b decides (row b=0).
        assert_eq!(c.hits(state(&[Some(true), Some(false)])), 0b0100);
        // a or (b and c)
        let c = cond(&[Input(0), Input(1), Input(2), And, Or], 3);
        assert_eq!(
            c.hits(state(&[Some(false), Some(true), Some(true)])),
            1 << 3 | 1 << 5
        );
        // a true: the right side is not evaluated and masked; a decides.
        assert_eq!(c.hits(state(&[Some(true), None, None])), 1 << 1);
    }

    #[test]
    fn constant_inputs_have_no_rows() {
        use CovNode::*;
        // a and k and b, k constant true: rows are a, b only (k is evaluated, never reported).
        let mut c = cond(&[Input(0), Input(1), And, Input(2), And], 3);
        c.consts = 0b010;
        let t = Some(true);
        assert_eq!(c.hits(state(&[t, t, t])), 0b1010);
        assert_eq!(c.hits(state(&[Some(false), t, t])), 0b0001);
        assert_eq!(c.hits(state(&[t, t, Some(false)])), 0b0100);
    }

    #[test]
    fn xor_not_and_other_values() {
        use CovNode::*;
        // not (a xor b): every input decides alone, in every evaluation.
        let c = cond(&[Input(0), Input(1), Xor, Not], 2);
        assert_eq!(c.hits(state(&[Some(true), Some(false)])), 0b0110);
        assert_eq!(c.hits(state(&[Some(true), Some(true)])), 0b1010);
        // An input with another value (code 3) counts nothing.
        assert_eq!(c.hits(1 | 3 << 2), 0);
        // nand and nor invert the result, not the rows.
        let c = cond(&[Input(0), Input(1), Nand], 2);
        assert_eq!(c.hits(state(&[Some(true), Some(true)])), 0b1010);
        let c = cond(&[Input(0), Input(1), Nor], 2);
        assert_eq!(c.hits(state(&[Some(false), Some(false)])), 0b0101);
    }

    /// The rows by brute force over the unevaluated inputs, for every tree shape and state of
    /// three inputs.
    #[test]
    fn brute_force() {
        use CovNode::*;
        let ops = [And, Or, Nand, Nor, Xor, Xnor];
        let mut trees = Vec::new();
        for &o1 in &ops {
            for &o2 in &ops {
                trees.push(vec![Input(0), Input(1), o1, Input(2), o2]);
                trees.push(vec![Input(0), Input(1), Input(2), o1, o2]);
                trees.push(vec![Input(0), Input(1), o1, Not, Input(2), o2]);
            }
        }
        let eval = |nodes: &[CovNode], v: [bool; 3]| {
            let mut st: Vec<bool> = Vec::new();
            for n in nodes {
                match n {
                    Input(i) => st.push(v[usize::from(*i)]),
                    Not => {
                        let a = st.pop().unwrap();
                        st.push(!a);
                    }
                    _ => {
                        let (b, a) = (st.pop().unwrap(), st.pop().unwrap());
                        st.push(match n {
                            And => a & b,
                            Nand => !(a & b),
                            Or => a | b,
                            Nor => !(a | b),
                            Xor => a ^ b,
                            _ => !(a ^ b),
                        });
                    }
                }
            }
            st[0]
        };
        for t in &trees {
            let c = cond(t, 3);
            for s in 0..4u64.pow(3) {
                let code = |i: u32| (s >> (2 * i)) & 3;
                let want = if (0..3).any(|i| code(i) == 3) {
                    0
                } else {
                    // Free inputs: the result must not depend on them.
                    let all = |f: &dyn Fn([bool; 3]) -> bool| {
                        let mut r = Vec::new();
                        for m in 0..8u32 {
                            let v = [m & 1 != 0, m & 2 != 0, m & 4 != 0];
                            // Inputs with a value keep it.
                            if (0..3).all(|i| code(i) == 0 || v[i as usize] == (code(i) == 2)) {
                                r.push(f(v));
                            }
                        }
                        r
                    };
                    let orig = all(&|v| eval(t, v));
                    if orig.iter().any(|&x| x != orig[0]) {
                        0
                    } else {
                        let mut h = 0u64;
                        for i in (0..3u32).filter(|&i| code(i) != 0) {
                            // Flip input i, keep the other evaluated inputs.
                            let mut r = Vec::new();
                            for m in 0..8u32 {
                                let v = [m & 1 != 0, m & 2 != 0, m & 4 != 0];
                                let ok = (0..3).all(|j| {
                                    code(j) == 0
                                        || if j == i {
                                            v[j as usize] != (code(j) == 2)
                                        } else {
                                            v[j as usize] == (code(j) == 2)
                                        }
                                });
                                if ok {
                                    r.push(eval(t, v));
                                }
                            }
                            if r.iter().any(|&x| x != orig[0]) {
                                h |= 1 << (2 * i + u32::from(code(i) == 2));
                            }
                        }
                        h
                    }
                };
                assert_eq!(c.hits(s), want, "{t:?} state {s:#b}");
            }
        }
    }
}
