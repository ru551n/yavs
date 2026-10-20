//! The yavs IR interpreter (docs/design/ir.md section 3).
//!
//! - [`Runtime`]: one per simulation; the verified [`Program`] plus everything bound to it
//!   outside the IR (foreign function pointers, package instance data, coverage counters,
//!   limits). Natives are resolved here by their `lib/NATIVE.md` symbol.
//! - [`InterpProcess`]: one per process instance; implements [`yavs_kernel::Process`]. It
//!   owns its frame stack (frames are heap objects, so a `wait` inside a procedure needs
//!   nothing special), its private heap and its temporaries.
//! - [`call`]: runs a subprogram, elaboration body or thunk without a kernel (elaboration-time
//!   evaluation, ir.md 3.6).
//!
//! Every runtime error is a kernel `SimError` whose `trace` is the VHDL call stack as
//! `StmtId`s, innermost first. Broken invariants are `ErrorKind::Internal`.

mod binfile;
mod debug;
mod env;
mod exec;
mod foreign;
mod pre;
pub use foreign::{ForeignModel, ModelArg, ModelCall, ModelSignal, ModelTxn, ScalarKind};
mod heap;
mod ieee;
mod implicit;
mod native;
mod reflect;
mod roots;
mod textio;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub use env::{Sources, elaboration_reports};
use yavs_ffi::ForeignFn;
use yavs_ir::{ForeignId, FuncId, FuncKind, Program};
use yavs_kernel::{
    Ctx, DriverId, ErrorKind, ProcFlags, Process, Range, SignalId, SignalType, SimError,
};
use yavs_std::arith::TimeResolution;
use yavs_std::types::Revision;

pub use debug::{
    DebugSession, Debugger, FrameView, HitCond, Hits, LineBp, LocalView, ProcState, ProcStatus,
    Resume, SignalBreak, Stop, StopReason, Watches, format_scalar,
};
pub use heap::Heap;
pub use textio::{FILE_INPUT, FILE_OUTPUT};

use exec::{Fault, Machine, Outcome};

/// Per-instance data of a design unit, frozen before simulation (ir.md 2).
#[derive(Clone, Debug, Default)]
pub struct InstData {
    /// Generics and constants, addressed by `AddrSpace::Inst`.
    pub data: Vec<u8>,
    /// The instance's kernel signal leaves, indexed by `SigVal::sig`.
    pub sigs: Vec<InstSig>,
    /// This instance's first counter in the runtime's coverage array.
    pub cov_base: u32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InstSig {
    pub signal: SignalId,
    pub ty: SignalType,
    /// Kernel offset of the object in `signal`: 0, or where an `in` port collapsed into part
    /// of its actual's leaf starts (yavs-elab `SigObj::part`). Every element offset adds it.
    pub off: u32,
}

/// A driver slot of a process: the kernel driver and the signal elements it drives.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ProcDriver {
    pub id: DriverId,
    /// Index into `InstData::sigs`.
    pub sig: u32,
    pub range: Range,
}

/// Shared variables and the shared heap, used only by serial processes (ir.md 3.5).
#[derive(Clone, Default)]
pub struct SharedState {
    /// Addressed by `AddrSpace::Shared`; `Program::shared_roots` is its root map.
    pub vars: Vec<u8>,
    /// Boxed: a process that never allocates in it carries only the pointer.
    pub(crate) heap: Option<Box<Heap>>,
    /// The shared-heap roots of every serial process that ran, as of its last suspension
    /// (`None`: one could not be located).
    pub(crate) procs: Vec<Option<ProcRoots>>,
    /// The simulation has started ([`SharedGc`] saw the start): until then, code run for
    /// elaboration holds handles that no root map names, so nothing is collected inside an
    /// activation.
    pub(crate) started: bool,
}

/// A serial process's shared-heap roots: the handles its frames' root maps and recorded
/// temporaries hold, and the slots the addresses there point into (#422).
#[derive(Clone, Default)]
pub(crate) struct ProcRoots {
    pub handles: Vec<u64>,
    pub pinned: Vec<u32>,
}

impl SharedState {
    pub fn new(vars: Vec<u8>) -> SharedState {
        SharedState {
            vars,
            heap: Some(Box::new(Heap::new(heap::SHARED))),
            procs: Vec::new(),
            started: false,
        }
    }

    /// Shared-heap slots that some serial process's frames address, sorted; `None` when a
    /// process's roots could not be located.
    pub(crate) fn pinned(&self) -> Option<Vec<u32>> {
        let mut v = Vec::new();
        for p in &self.procs {
            v.extend(&p.as_ref()?.pinned);
        }
        v.sort_unstable();
        v.dedup();
        Some(v)
    }

    /// Collect the shared heap. Call only at a serial safepoint, when no process runs: roots
    /// are the shared variables area (`Program::shared_roots`), the sealed objects and what
    /// each serial process published at its last suspension. False (nothing freed) when a
    /// root's handles cannot be located.
    pub fn collect(&mut self, prog: &Program) -> bool {
        let mut roots = heap::Refs::default();
        if !heap::slot_roots(prog, &prog.shared_roots, &self.vars, &mut roots) {
            return false;
        }
        for p in &self.procs {
            match p {
                Some(p) => roots.handles.extend(&p.handles),
                None => return false,
            }
        }
        let Some(pinned) = self.pinned() else {
            return false;
        };
        self.heap
            .as_mut()
            .is_none_or(|h| h.collect(prog, &roots.handles, &pinned))
    }

    /// Objects alive in the shared heap (for tests and statistics).
    pub fn heap_live(&self) -> usize {
        self.heap.as_ref().map_or(0, |h| h.live())
    }

    /// Slots of the shared heap, deallocated or not (for tests and statistics).
    pub fn heap_slots(&self) -> usize {
        self.heap.as_ref().map_or(0, |h| h.slots())
    }
}

/// Collects the shared heap at serial safepoints: seals it when the simulation starts
/// ([`Heap::seal`]) and collects it at the start of a cycle, before any process runs, once
/// more was allocated than [`Limits::gc_threshold`] and than the last collection kept
/// (`Heap::gc_due`). Subscribe it to every simulation with a [`SharedState`].
pub struct SharedGc {
    pub rt: Arc<Runtime>,
    pub shared: Arc<Mutex<SharedState>>,
}

impl yavs_kernel::Observer for SharedGc {
    fn phase(&mut self, _hook: &mut yavs_kernel::Hook<'_>, phase: yavs_kernel::Phase) {
        // A poisoned lock is reported by the next process that takes it.
        let Ok(mut s) = self.shared.lock() else {
            return;
        };
        match phase {
            yavs_kernel::Phase::StartOfSim => {
                s.started = true;
                if let Some(h) = s.heap.as_mut() {
                    h.seal();
                }
            }
            yavs_kernel::Phase::NextCycle => {
                let t = self.rt.limits.gc_threshold;
                if s.heap.as_ref().is_some_and(|h| h.gc_due(t)) {
                    s.collect(&self.rt.prog);
                }
            }
            _ => {}
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Limits {
    /// Deepest call stack before a "call depth" runtime error.
    pub max_depth: u32,
    /// Bytes allocated in a process heap before it is collected at the next suspension.
    pub gc_threshold: u64,
}

impl Default for Limits {
    fn default() -> Limits {
        Limits {
            max_depth: 10_000,
            gc_threshold: 1 << 20,
        }
    }
}

/// What the elaborator binds to a program.
pub struct Bindings {
    /// A function for every `Program::foreign` entry (looked up by the elaborator).
    pub foreign: Vec<(ForeignId, ForeignFn)>,
    /// A plugin-registered model for every `'FOREIGN "VHPI library model"` entry that has one.
    pub models: Vec<(ForeignId, Arc<dyn ForeignModel>)>,
    /// Where the bindings looked for symbols (reported when an unbound one is called).
    pub searched: String,
    /// Package instance data, addressed by `AddrSpace::Pkg(i)`.
    pub packages: Vec<Arc<InstData>>,
    /// Coverage counters (instances times items); `None` means `Program::cov.len()`.
    pub cov_counters: Option<usize>,
    pub limits: Limits,
    /// The analysed revision: INTEGER bounds for textio reads, `vhdl_version`.
    pub revision: Revision,
    /// The time resolution, for `resolution_limit`.
    pub resolution: TimeResolution,
    /// The design's source files, for `std.env` call paths ([`Sources::of`]).
    pub sources: Sources,
}

impl Default for Bindings {
    fn default() -> Bindings {
        Bindings {
            foreign: Vec::new(),
            models: Vec::new(),
            searched: String::new(),
            packages: Vec::new(),
            cov_counters: None,
            limits: Limits::default(),
            revision: Revision::Vhdl2008,
            resolution: TimeResolution::Fs,
            sources: Sources::default(),
        }
    }
}

/// A compiled tier for some functions of a program (the LLVM tier in `yavs-llvm`).
///
/// Compiled code runs a function on the registers of its interpreter frame and leaves
/// everything it does not lower itself to the interpreter: it calls `step` to execute one
/// instruction in place (the instructions that always continue with the next one) and
/// returns at the others (calls, returns, waits, reports, slow and error paths), which the
/// interpreter then executes. The interpreter stays the semantic reference (rule 5).
pub trait Tier: Send + Sync {
    /// Run `func` from instruction `pc` until an instruction the interpreter must execute
    /// and return its index; `None` when `func` is not compiled.
    fn run(
        &self,
        func: FuncId,
        pc: u32,
        fr: &mut TierFrame,
        host: &mut dyn TierHost,
    ) -> Option<u32>;

    /// Whether `func` has compiled code now: the interpreter calls `run` only then.
    fn ready(&self, _func: FuncId) -> bool {
        true
    }

    /// `func` reached its threshold (`Runtime::set_heat`): compile it. Called from
    /// simulation threads, so it must not block.
    fn hot(&self, _func: FuncId) {}
}

/// The interpreter's side of [`Tier::run`].
pub trait TierHost {
    /// Execute the instruction at `pc` on `fr`; `false` on a fault, after which `run`
    /// returns `pc` at once.
    fn step(&mut self, fr: &mut TierFrame, pc: u32) -> bool;

    /// The memory object `addr` points into, as its start and length, writable when
    /// `write`: `None` for the memory in the [`TierFrame`] and wherever the interpreter's
    /// access would fail (compiled code then steps the instruction). Valid until the next
    /// `step`.
    fn object(&mut self, addr: u64, write: bool) -> Option<(*mut u8, usize)>;
}

/// What compiled code reads and writes itself (ir.md 5): the top frame's registers and
/// slots and the temporary area, taken out of the machine for [`Tier::run`] and put back
/// around every `step`. Every other address goes through `step`.
///
/// The registers and slots of all frames live in two stacks (#438): `regs` is the register
/// stack, the frame's registers start at `regs_at`; `slots` is the stack the frame's slots
/// are in (the process's own variables for frame 0), they start at `slots_at` and are
/// `slots_len` bytes. Use [`TierFrame::regs_mut`] and [`TierFrame::slots_mut`].
pub struct TierFrame {
    pub regs: Vec<u64>,
    pub regs_at: usize,
    /// The slots of the frame, reached by addresses `[space::FRAME, frame, offset]`.
    pub slots: Vec<u8>,
    pub slots_at: usize,
    pub slots_len: usize,
    pub frame: u32,
    /// The temporary area, `[space::TEMP, 0, offset]`; `Inst::Stmt` truncates it to
    /// `temp_base`.
    pub temp: Vec<u8>,
    pub temp_base: usize,
    /// Temporaries recorded for the GC (`Inst::TempRoot`): while there are any, compiled
    /// code steps `Inst::Stmt`, whose interpreter form drops those of the ending statement.
    pub temp_roots: usize,
    /// The instance data (read only), `[space::INST, 0, offset]`. It outlives the run.
    pub inst: *const [u8],
}

impl TierFrame {
    /// The registers of the frame.
    pub fn regs_mut(&mut self) -> &mut [u64] {
        &mut self.regs[self.regs_at..]
    }

    /// The slots of the frame.
    pub fn slots_mut(&mut self) -> &mut [u8] {
        &mut self.slots[self.slots_at..self.slots_at + self.slots_len]
    }
}

/// Address spaces: the top 3 bits of an address `[space: 3 | index: 29 | offset: 32]`.
pub mod space {
    pub use crate::exec::{CONST, FRAME, INST, PKG, SHARED, TEMP};
}

/// A program bound for execution, shared by all processes of a simulation.
pub struct Runtime {
    pub prog: Arc<Program>,
    pub(crate) natives: Vec<Option<native::NativeFn>>,
    pub(crate) foreign: Vec<Option<ForeignFn>>,
    pub(crate) models: Vec<Option<Arc<dyn ForeignModel>>>,
    /// Where unbound foreign symbols were looked for, for the error at their call.
    pub(crate) searched: String,
    pub(crate) pkgs: Vec<Arc<InstData>>,
    pub(crate) cov: Vec<AtomicU64>,
    /// Compact constants expanded on first use by address.
    pub(crate) expanded: Vec<std::sync::OnceLock<Vec<u8>>>,
    /// Per-function facts, computed on the first call (`pre.rs`).
    pre: Vec<std::sync::OnceLock<pre::Pre>>,
    pub(crate) limits: Limits,
    pub(crate) revision: Revision,
    pub(crate) resolution: TimeResolution,
    pub(crate) sources: Sources,
    /// VHDL assert counts and enables (`env.rs`).
    pub(crate) asserts: env::Asserts,
    /// Open file objects; a file cell holds index + 1 (0: never opened).
    pub(crate) files: Mutex<Vec<textio::FileSlot>>,
    /// Compiled code, set once before the simulation starts (`set_tier`).
    pub(crate) tier: std::sync::OnceLock<Arc<dyn Tier>>,
    /// Calls run again for their side effects (`set_replay`): their reports without a
    /// kernel were printed and counted the first time.
    pub(crate) replay: std::sync::atomic::AtomicBool,
    /// Per function: the tier-up count and its threshold (`set_heat`).
    heat: std::sync::OnceLock<(Box<[Count]>, i64)>,
    /// Live registers at the safepoints of each function a collection met (`roots`).
    live: Vec<std::sync::OnceLock<roots::LiveRefs>>,
}

/// A function's tier-up count and its threshold (`Runtime::set_heat`).
pub(crate) type Count = (std::sync::atomic::AtomicU32, u32);

fn usage(m: String) -> SimError {
    SimError::new(ErrorKind::Usage(m))
}

impl Runtime {
    /// Fails early on a native symbol that `lib/NATIVE.md` does not define or a foreign
    /// function whose C signature differs from the IR's. An unbound foreign subprogram is
    /// an error only when it is called.
    pub fn new(prog: Arc<Program>, b: Bindings) -> Result<Arc<Runtime>, SimError> {
        let mut natives = Vec::with_capacity(prog.natives.len());
        for (_, n) in prog.natives.iter() {
            natives.push(native::lookup(&n.symbol).map_err(usage)?);
        }
        let mut foreign: Vec<Option<ForeignFn>> = vec![None; prog.foreign.len()];
        for (id, f) in b.foreign {
            let Some(x) = prog.foreign.get(id) else {
                return Err(usage(format!(
                    "binding for missing foreign subprogram {id}"
                )));
            };
            if f.signature() != &x.signature() {
                return Err(usage(format!(
                    "foreign subprogram `{}` ({}): bound function has C signature {:?}, \
                     the declaration needs {:?}",
                    x.name,
                    x.symbol,
                    f.signature(),
                    x.signature()
                )));
            }
            foreign[id.0 as usize] = Some(f);
        }
        let mut models: Vec<Option<Arc<dyn ForeignModel>>> = vec![None; prog.foreign.len()];
        for (id, h) in b.models {
            match prog.foreign.get(id) {
                Some(x) if x.model.is_some() => models[id.0 as usize] = Some(h),
                _ => {
                    return Err(usage(format!(
                        "model binding for non-VHPI foreign subprogram {id}"
                    )));
                }
            }
        }
        let n = b.cov_counters.unwrap_or(prog.cov.len());
        Ok(Arc::new(Runtime {
            natives,
            foreign,
            models,
            searched: b.searched,
            pkgs: b.packages,
            cov: (0..n).map(|_| AtomicU64::new(0)).collect(),
            expanded: (0..prog.consts.len()).map(|_| Default::default()).collect(),
            pre: (0..prog.funcs.len()).map(|_| Default::default()).collect(),
            limits: b.limits,
            revision: b.revision,
            resolution: b.resolution,
            sources: b.sources,
            asserts: Default::default(),
            files: Mutex::new(textio::FileSlot::preopened()),
            tier: Default::default(),
            replay: Default::default(),
            heat: Default::default(),
            live: (0..prog.funcs.len()).map(|_| Default::default()).collect(),
            prog,
        }))
    }

    /// Run `prog`, a program that extends this one (every id of this one names the same item
    /// there, `Lowerer::snapshot`), the files of its new statements read from `map`.
    /// Everything else stays: package data, open files, assert counts, coverage counters.
    /// Elaboration adds the functions it calls to one program this way instead of
    /// initialising the packages again.
    pub fn extend(
        &mut self,
        prog: Arc<Program>,
        map: &yavs_diag::SourceMap,
    ) -> Result<(), SimError> {
        let first = self.prog.stmts.len();
        self.extend_from(prog, first, map)
    }

    /// Take the program out of the runtime to grow it in place, without a copy: the runtime
    /// holds an empty one until [`Runtime::extend_from`] gets the grown program (the caller
    /// keeps the old statement count). `None` while anything else holds the program.
    pub fn take_program(&mut self) -> Option<Program> {
        Arc::get_mut(&mut self.prog).map(std::mem::take)
    }

    /// [`Runtime::extend`] when the statements before `first` are those of the old program
    /// (it was taken with [`Runtime::take_program`]).
    pub fn extend_from(
        &mut self,
        prog: Arc<Program>,
        first: usize,
        map: &yavs_diag::SourceMap,
    ) -> Result<(), SimError> {
        for (_, n) in prog.natives.iter().skip(self.natives.len()) {
            self.natives.push(native::lookup(&n.symbol).map_err(usage)?);
        }
        self.foreign.resize(prog.foreign.len(), None);
        self.models.resize(prog.foreign.len(), None);
        self.expanded
            .resize_with(prog.consts.len(), Default::default);
        self.pre.resize_with(prog.funcs.len(), Default::default);
        self.live.resize_with(prog.funcs.len(), Default::default);
        self.sources.add_from(&prog, first, map);
        self.prog = prog;
        Ok(())
    }

    /// The facts of function `id` (`f`), computed once.
    pub(crate) fn pre(&self, id: yavs_ir::FuncId, f: &yavs_ir::Func) -> Option<&pre::Pre> {
        Some(
            self.pre
                .get(id.0 as usize)?
                .get_or_init(|| pre::Pre::new(f)),
        )
    }

    /// The `Handle` and `Ptr` registers live at each safepoint of `id`, computed when first
    /// asked for (`roots::live_refs`).
    pub(crate) fn live_refs(&self, id: FuncId) -> Option<&roots::LiveRefs> {
        let f = self.prog.func(id)?;
        Some(
            self.live
                .get(id.0 as usize)?
                .get_or_init(|| roots::live_refs(&self.prog, f, heap::STRESS)),
        )
    }

    /// The data of the bound packages, in `Bindings::packages` order.
    pub fn package_data(&self) -> &[Arc<InstData>] {
        &self.pkgs
    }

    /// Whether code run so far opened a file or closed a standard one: such state cannot be
    /// carried into another runtime ([`Runtime::inherit`]).
    pub fn files_touched(&self) -> bool {
        let files = self.files.lock().unwrap_or_else(|e| e.into_inner());
        files.len() != 2 || !files.iter().all(textio::FileSlot::is_open)
    }

    /// Close the files the design left open, writing out what they buffer, as dropping the
    /// runtime does: for a process that ends without dropping it.
    pub fn close_files(&self) {
        self.files.lock().unwrap_or_else(|e| e.into_inner()).clear();
    }

    /// Start from `from`'s VHDL assert state (enables and the like, as code run by `from`
    /// left it): a simulation going on from package state that elaboration-time calls made.
    pub fn inherit(&self, from: &Runtime) {
        self.asserts.inherit(&from.asserts);
    }

    /// While `on`, calls run again what they ran before (elaboration-time calls replayed on
    /// fresh package state, #390): a report without a kernel is neither printed nor counted
    /// again.
    pub fn set_replay(&self, on: bool) {
        self.replay.store(on, Ordering::Relaxed);
    }

    /// The VHDL assert count of severity `s` (`GetVhdlAssertCount(s)`) as it stands now:
    /// the CLI's exit status follows the error and failure counts at the end of the run.
    pub fn assert_count(&self, s: yavs_kernel::Severity) -> u64 {
        self.asserts.counts()[s as usize]
    }

    /// Run compiled code of `tier` where it has some, from now on (`yavs run --tier`).
    /// Elaboration-time code that already ran stayed interpreted; a debugger keeps a
    /// process interpreted. Returns `false` if a tier was set before.
    pub fn set_tier(&self, tier: Arc<dyn Tier>) -> bool {
        self.tier.set(tier).is_ok()
    }

    /// Estimate per function what compiled code would save and call `Tier::hot` once a
    /// function reaches `threshold(function)` (`yavs run --tier=auto`): the predecoded loop
    /// counts the ops it runs and takes `cool` off at every op compiled code would leave to
    /// the interpreter (calls, returns, natives, signal reads, waits, generic steps), as leaving
    /// and entering compiled code costs more than it saves there. Returns `false` if counting
    /// was set up before.
    pub fn set_heat(&self, threshold: impl Fn(&yavs_ir::Func) -> u32, cool: u32) -> bool {
        let counts = self
            .prog
            .funcs
            .iter()
            .map(|(_, f)| (Default::default(), threshold(f)))
            .collect();
        self.heat.set((counts, cool.into())).is_ok()
    }

    /// The tier-up count of `func` and its threshold, while counting is on (`set_heat`).
    #[inline]
    pub(crate) fn count(&self, func: FuncId) -> Option<&Count> {
        self.heat.get()?.0.get(func.0 as usize)
    }

    /// What an op compiled code would leave to the interpreter takes off a count (`set_heat`).
    #[inline]
    pub(crate) fn cool(&self) -> i64 {
        self.heat.get().map_or(0, |h| h.1)
    }

    /// How much `func` (its `count`) still lacks to its threshold: at most 0 once reached.
    #[inline]
    pub(crate) fn left((c, hot): &Count) -> i64 {
        i64::from(*hot) - i64::from(c.load(std::sync::atomic::Ordering::Relaxed))
    }

    /// Add `d` (ops interpreted, less `cool` per op compiled code would leave) to `func`'s count,
    /// within 0 and the threshold, and call `Tier::hot` when it reaches it; returns what is
    /// left (`left`). The counters are racy on purpose (a lost update only delays tier-up;
    /// `Tier::hot` may see a function twice).
    pub(crate) fn heat(&self, func: FuncId, (c, hot): &Count, d: i64) -> i64 {
        use std::sync::atomic::Ordering::Relaxed;
        let was = c.load(Relaxed);
        if was >= *hot {
            return 0;
        }
        let now = (i64::from(was) + d).clamp(0, i64::from(*hot)) as u32;
        c.store(now, Relaxed);
        if now >= *hot
            && let Some(t) = self.tier.get()
        {
            t.hot(func);
        }
        i64::from(*hot) - i64::from(now)
    }

    /// Coverage counter values.
    pub fn coverage(&self) -> Vec<u64> {
        self.cov.iter().map(|c| c.load(Ordering::Relaxed)).collect()
    }
}

/// Kernel flags of a process function: serial when it touches shared state (ir.md 3.5),
/// several waits unless it has one suspension point (#252).
pub fn proc_flags(prog: &Program, func: FuncId) -> ProcFlags {
    ProcFlags {
        postponed: false,
        serial: prog.is_serial(func),
        several_waits: !prog.single_suspend(func),
        concurrent_call: prog
            .func(func)
            .is_some_and(|f| f.flags.contains(yavs_ir::FuncFlags::IMPLICIT))
            && !prog.single_suspend(func),
    }
}

/// A process instance running IR in the interpreter.
pub struct InterpProcess {
    m: Machine,
    started: bool,
    done: bool,
    shared: Option<Arc<Mutex<SharedState>>>,
    /// This process's entry in `SharedState::procs`.
    roots_slot: Option<usize>,
    /// Bumped whenever the machine runs or a variable is written: unchanged means every
    /// variable of the process has its last value (the waveform recorder skips it).
    execs: u64,
}

impl InterpProcess {
    /// `func` must be a `FuncKind::Process` of `rt.prog`. A process that touches shared state
    /// (`proc_flags(..).serial`) needs `shared`, and must be built serial in the kernel.
    pub fn new(
        rt: Arc<Runtime>,
        func: FuncId,
        inst: Arc<InstData>,
        drivers: Vec<ProcDriver>,
        shared: Option<Arc<Mutex<SharedState>>>,
    ) -> Result<InterpProcess, SimError> {
        let Some(f) = rt.prog.func(func) else {
            return Err(usage(format!("no function {func}")));
        };
        if f.kind != FuncKind::Process {
            return Err(usage(format!("{func} `{}` is not a process", f.name)));
        }
        if rt.prog.is_serial(func) && shared.is_none() {
            return Err(usage(format!(
                "process `{}` touches shared state: give it the SharedState and build it \
                 serial",
                f.name
            )));
        }
        for d in &drivers {
            if inst.sigs.get(d.sig as usize).is_none() {
                return Err(usage(format!(
                    "process `{}`: driver on missing instance signal {}",
                    f.name, d.sig
                )));
            }
        }
        Ok(InterpProcess {
            m: Machine::new(rt, inst, drivers, func),
            started: false,
            done: false,
            shared,
            roots_slot: None,
            execs: 0,
        })
    }

    /// Attach a debug session (ir.md 3.4); `name` appears in the session's process table.
    pub fn with_debug(mut self, session: Arc<DebugSession>, name: &str) -> InterpProcess {
        let i = session.register(name);
        self.m.attach(session, i);
        self
    }

    /// Objects alive in the private heap (for tests and statistics).
    pub fn heap_live(&self) -> usize {
        self.m.heap.live()
    }

    /// The program the process runs (for decoding [`InterpProcess::var`]).
    pub fn program(&self) -> &Program {
        &self.m.rt.prog
    }

    /// Variable `name` of the process's declarative part, read while the process is
    /// suspended (VHPI, #309): its IR type and value (a composite's data and descriptor, as
    /// [`call_values`] returns them). The declarative part of a process that has not run
    /// yet runs first ([`InterpProcess::prestart`]). An error for a process that has ended
    /// or a name the process does not declare.
    pub fn var(&mut self, name: &str) -> Result<(yavs_ir::TyId, CallVal), String> {
        let (ty, a) = self.var_slot(name)?;
        let v = self.with_sh(|m| read_slot(m, ty, a))?;
        Ok((ty, v))
    }

    /// Whether variable `name` holds exactly `v` (as [`InterpProcess::var`] would return it),
    /// compared in place without copying the value (the waveform recorder's per-step check).
    /// `stamp` is the [`Stamp`] of the data when `v` was read; a composite whose data is an
    /// object of a heap and has the same stamp is unchanged without comparing its bytes.
    /// On return `stamp` is the data's current one.
    pub fn var_is(
        &mut self,
        name: &str,
        v: &CallVal,
        stamp: &mut Option<Stamp>,
    ) -> Result<bool, String> {
        let (ty, a) = self.var_slot(name)?;
        self.with_sh(|m| slot_is(m, ty, a, v, stamp))
    }

    /// Writes `v` into variable `name` (see [`InterpProcess::var`]); a composite keeps its
    /// bounds, so `v`'s data must have the size of the current value's.
    pub fn set_var(&mut self, name: &str, v: &CallVal) -> Result<(), String> {
        self.execs += 1;
        let (ty, a) = self.var_slot(name)?;
        self.with_sh(|m| write_slot(m, ty, a, v))
    }

    /// Runs `f` on the machine, holding the shared state if the process has it: a serial
    /// process keeps its composite variables in the shared heap.
    fn with_sh<T>(&mut self, f: impl FnOnce(&mut Machine) -> exec::R<T>) -> Result<T, String> {
        let Some(shared) = self.shared.clone() else {
            return f(&mut self.m).map_err(fault_text);
        };
        let mut g = shared
            .lock()
            .map_err(|_| "shared state lock poisoned".to_string())?;
        self.m.sh = Some(std::mem::take(&mut *g));
        let r = f(&mut self.m);
        if let Some(st) = self.m.sh.take() {
            *g = st;
        }
        r.map_err(fault_text)
    }

    /// Whether the process has its variables: it has started (or [`InterpProcess::prestart`]
    /// ran) and not ended.
    /// A counter that changes whenever the process has run or had a variable written.
    pub fn execs(&self) -> u64 {
        self.execs
    }

    pub fn has_vars(&self) -> bool {
        let m = &self.m;
        m.frames.first().is_some_and(|f| f.func == m.entry)
    }

    /// The IR type and slot address of variable `name` in the process's frame.
    fn var_slot(&mut self, name: &str) -> Result<(yavs_ir::TyId, u64), String> {
        self.prestart()?;
        let m = &self.m;
        if !self.has_vars() {
            return Err("the process has ended".into());
        }
        // ponytail: by name; the declarative part is lowered before the statements, so the
        // first local of a name is the variable (a loop parameter may reuse the name later).
        // Key locals by declaration if two ever share a name in one frame.
        let l =
            m.rt.prog
                .func(m.entry)
                .and_then(|f| f.locals.iter().find(|l| l.name == name))
                .ok_or_else(|| format!("the process has no variable `{name}`"))?;
        let a = exec::mk_addr(exec::FRAME, 0, u64::from(l.slot)).map_err(fault_text)?;
        Ok((l.ty, a))
    }
}

/// The value of the non-protected shared variable of IR type `ty` whose slot is at `slot`
/// (the address its `Lowered::var_slots` thunk returns), read between cycles (VHPI, #309).
pub fn shared_var(
    rt: &Arc<Runtime>,
    shared: &Mutex<SharedState>,
    slot: u64,
    ty: yavs_ir::TyId,
) -> Result<CallVal, String> {
    with_shared(rt, shared, |m| read_slot(m, ty, slot))
}

/// Whether the shared variable holds exactly `v`, compared in place (see
/// [`InterpProcess::var_is`]).
pub fn shared_var_is(
    rt: &Arc<Runtime>,
    shared: &Mutex<SharedState>,
    slot: u64,
    ty: yavs_ir::TyId,
    v: &CallVal,
    stamp: &mut Option<Stamp>,
) -> Result<bool, String> {
    with_shared(rt, shared, |m| slot_is(m, ty, slot, v, stamp))
}

/// Writes `v` into a shared variable (see [`shared_var`]); a composite keeps its bounds.
pub fn set_shared_var(
    rt: &Arc<Runtime>,
    shared: &Mutex<SharedState>,
    slot: u64,
    ty: yavs_ir::TyId,
    v: &CallVal,
) -> Result<(), String> {
    with_shared(rt, shared, |m| write_slot(m, ty, slot, v))
}

/// Runs `f` on a machine holding the shared state (as [`call_shared`] does).
fn with_shared<T>(
    rt: &Arc<Runtime>,
    shared: &Mutex<SharedState>,
    f: impl FnOnce(&mut Machine) -> exec::R<T>,
) -> Result<T, String> {
    let mut g = shared
        .lock()
        .map_err(|_| "shared state lock poisoned".to_string())?;
    let mut m = Machine::new(rt.clone(), Arc::default(), Vec::new(), yavs_ir::FuncId(0));
    m.sh = Some(std::mem::take(&mut *g));
    let r = f(&mut m);
    if let Some(st) = m.sh.take() {
        *g = st;
    }
    r.map_err(fault_text)
}

/// The value of a variable of IR type `ty` whose slot is at `a`: a scalar, or a composite's
/// `[data, desc, ..]` (yavs-lower `VarShape`).
pub(crate) fn read_slot(m: &Machine, ty: yavs_ir::TyId, a: u64) -> exec::R<CallVal> {
    if let Some(n) = m.rt.prog.types.get(ty).and_then(yavs_ir::Ty::scalar_size) {
        return Ok(CallVal::Cell(m.load(a, width(n))?));
    }
    let ((data, n), desc) = composite(m, ty, a)?;
    Ok(CallVal::Composite {
        data: m.rd(data, n)?.to_vec(),
        desc,
    })
}

/// Whether [`read_slot`] would return `v`, without building it.
fn slot_is(
    m: &Machine,
    ty: yavs_ir::TyId,
    a: u64,
    v: &CallVal,
    stamp: &mut Option<Stamp>,
) -> exec::R<bool> {
    if let Some(n) = m.rt.prog.types.get(ty).and_then(yavs_ir::Ty::scalar_size) {
        return Ok(matches!(v, CallVal::Cell(c) if *c == m.load(a, width(n))?));
    }
    let CallVal::Composite { data: d, desc: dd } = v else {
        return Ok(false);
    };
    let prog = &m.rt.prog;
    let data = m.load(a, yavs_ir::Width::B8)?;
    let dn = yavs_ir::Ty::desc_size(&prog.types, ty)
        .ok_or_else(|| exec::Fault::Internal("variable of a cyclic type".into()))?;
    let desc = if dn == 0 {
        &[][..]
    } else {
        m.rd(m.load(exec::add_addr(a, 8)?, yavs_ir::Width::B8)?, dn)?
    };
    let now = m.stamp(data);
    let before = std::mem::replace(stamp, now);
    if desc != dd.as_slice() {
        return Ok(false);
    }
    // Same descriptor, same layout: an unwritten heap object needs no further look.
    if now.is_some() && now == before {
        return Ok(true);
    }
    let (l, _) = yavs_ir::Ty::layout(&prog.types, &prog.consts, ty, desc)
        .map_err(|e| exec::Fault::Internal(format!("variable layout: {e:?}")))?;
    Ok(l.size == d.len() as u64 && m.rd(data, l.size)? == d.as_slice())
}

/// Writes `v` into the variable slot at `a` (see [`read_slot`]).
pub(crate) fn write_slot(m: &mut Machine, ty: yavs_ir::TyId, a: u64, v: &CallVal) -> exec::R<()> {
    let scalar = m.rt.prog.types.get(ty).and_then(yavs_ir::Ty::scalar_size);
    match (v, scalar) {
        (CallVal::Cell(c), Some(n)) => m.store(a, width(n), *c),
        (CallVal::Composite { data, .. }, None) => {
            let ((at, n), _) = composite(m, ty, a)?;
            if n != data.len() as u64 {
                return Err(exec::Fault::Internal(format!(
                    "{} bytes for a variable of {n}",
                    data.len()
                )));
            }
            m.put(at, data)
        }
        _ => Err(exec::Fault::Internal(
            "value and variable shapes differ".into(),
        )),
    }
}

/// The data (address, size) and descriptor bytes of a composite variable whose slot is
/// `[data, desc, ..]` at `a`.
fn composite(m: &Machine, ty: yavs_ir::TyId, a: u64) -> exec::R<((u64, u64), Vec<u8>)> {
    let prog = &m.rt.prog;
    let data = m.load(a, yavs_ir::Width::B8)?;
    let n = yavs_ir::Ty::desc_size(&prog.types, ty)
        .ok_or_else(|| exec::Fault::Internal("variable of a cyclic type".into()))?;
    let desc = if n == 0 {
        Vec::new()
    } else {
        let d = m.load(exec::add_addr(a, 8)?, yavs_ir::Width::B8)?;
        m.rd(d, n)?.to_vec()
    };
    let (l, _) = yavs_ir::Ty::layout(&prog.types, &prog.consts, ty, &desc)
        .map_err(|e| exec::Fault::Internal(format!("variable layout: {e:?}")))?;
    Ok(((data, l.size), desc))
}

fn width(n: u64) -> yavs_ir::Width {
    match n {
        1 => yavs_ir::Width::B1,
        4 => yavs_ir::Width::B4,
        _ => yavs_ir::Width::B8,
    }
}

pub(crate) fn fault_text(e: exec::Fault) -> String {
    match e {
        exec::Fault::Runtime(m) | exec::Fault::Internal(m) => m,
        exec::Fault::Unbound(b) => b.0,
        exec::Fault::Kernel(e) => format!("{e:?}"),
    }
}

impl Process for InterpProcess {
    fn serial(&self) -> bool {
        self.m.rt.prog.is_serial(self.m.entry)
    }

    fn several_waits(&self) -> bool {
        !self.m.rt.prog.single_suspend(self.m.entry)
    }

    fn concurrent_call(&self) -> bool {
        let p = &self.m.rt.prog;
        p.func(self.m.entry)
            .is_some_and(|f| f.flags.contains(yavs_ir::FuncFlags::IMPLICIT))
            && !p.single_suspend(self.m.entry)
    }

    fn as_any(&mut self) -> Option<&mut dyn std::any::Any> {
        Some(self)
    }

    fn run(&mut self, ctx: &mut Ctx<'_>) -> Result<(), SimError> {
        if self.done {
            let e = Fault::Internal("a terminated process was resumed".into());
            return Err(self.m.error(e, Some(ctx)));
        }
        if !self.started {
            self.started = true;
            if let Err(e) = self.m.enter(self.m.entry, &[]) {
                return Err(self.m.error(e, Some(ctx)));
            }
        }
        if let Err(e) = self.m.borrow_scratch() {
            return Err(self.m.error(e, Some(ctx)));
        }
        let r = self.exec(Some(&mut *ctx));
        match r {
            Ok(Outcome::Suspended) => {
                self.m.after_suspend();
                self.m.release_scratch();
                Ok(())
            }
            Ok(Outcome::Returned(_)) => {
                // The frame is gone: `var` fails from now on, and the waveform keeps the
                // variable values of the last suspension.
                self.done = true;
                self.m.terminated();
                Ok(())
            }
            Err(e) => Err(self.m.error(e, Some(ctx))),
        }
    }
}

impl InterpProcess {
    /// Runs the machine with the shared state locked, until it suspends or returns.
    fn exec(&mut self, ctx: Option<&mut Ctx<'_>>) -> Result<Outcome, Fault> {
        self.execs += 1;
        let shared = self.shared.clone();
        let mut guard = match &shared {
            Some(s) => Some(
                s.lock()
                    .map_err(|_| Fault::Internal("shared state lock poisoned".into()))?,
            ),
            None => None,
        };
        if let Some(g) = guard.as_mut() {
            self.m.sh = Some(std::mem::take(&mut **g));
        }
        let r = self.m.run(ctx, 0);
        if r.is_ok() {
            self.m.publish_shared(&mut self.roots_slot);
        }
        if let (Some(g), Some(st)) = (guard.as_mut(), self.m.sh.take()) {
            **g = st;
        }
        r
    }

    /// Runs the declarative part of a process that has not started, so its variables hold
    /// their initial values (VHPI at the start of simulation, #342); the first run then
    /// continues with the statements. Runs outside the kernel: a declarative part that
    /// needs the running simulation (a signal value, say) is an error, and the process is
    /// left unstarted.
    pub fn prestart(&mut self) -> Result<(), String> {
        if self.started {
            return Ok(());
        }
        let Some(body) = self.m.rt.prog.func(self.m.entry).and_then(|f| f.body) else {
            return Err("this process has no declarative part to run".into());
        };
        self.m.pause = Some(body);
        let r = self
            .m
            .enter(self.m.entry, &[])
            .and_then(|()| self.exec(None));
        self.m.pause = None;
        match r {
            Ok(Outcome::Suspended) => {
                self.started = true;
                Ok(())
            }
            r => {
                self.m.truncate_frames(0);
                Err(match r {
                    Err(e) => format!("its declarative part cannot run yet: {}", fault_text(e)),
                    _ => "this is a bug in yavs: a process ended in its declarative part".into(),
                })
            }
        }
    }
}

/// Run `func` (a function, procedure that does not wait, elaboration body or thunk) without a
/// kernel. Signal operations are "during elaboration" errors.
pub fn call(
    rt: Arc<Runtime>,
    func: FuncId,
    inst: Arc<InstData>,
    args: &[u64],
) -> Result<Option<u64>, SimError> {
    call_shared(rt, func, inst, args, None)
}

/// [`call`] with access to the shared variables and the shared heap, for elaboration code
/// that creates shared protected objects (`New { shared: true }`) or initialises shared
/// variables.
pub fn call_shared(
    rt: Arc<Runtime>,
    func: FuncId,
    inst: Arc<InstData>,
    args: &[u64],
    shared: Option<&Arc<Mutex<SharedState>>>,
) -> Result<Option<u64>, SimError> {
    let mut m = Machine::new(rt, inst, Vec::new(), func);
    let mut guard = match shared {
        Some(s) => Some(s.lock().map_err(|_| {
            SimError::new(ErrorKind::Internal("shared state lock poisoned".into()))
        })?),
        None => None,
    };
    if let Some(g) = guard.as_mut() {
        m.sh = Some(std::mem::take(&mut **g));
    }
    let r = m.enter(func, args).and_then(|()| m.run(None, 0));
    if let (Some(g), Some(st)) = (guard.as_mut(), m.sh.take()) {
        **g = st;
    }
    match r {
        Ok(Outcome::Returned(v)) => Ok(v),
        Ok(Outcome::Suspended) => Err(m.error(
            Fault::Runtime("wait statement executed during elaboration".into()),
            None,
        )),
        Err(e) => Err(m.error(e, None)),
    }
}

/// Initialise package instance `pkg` (`AddrSpace::Pkg(pkg)`, `size` bytes, zeroed): run its
/// initialisation function `func` (a thunk without parameters, yavs-lower's
/// `PackageIr::init`) with that package's data writable, and return the data. Packages it
/// reads must already be in the runtime's [`Bindings::packages`]; shared variables and the
/// shared heap (protected objects) are reachable through `shared`.
pub fn init_package(
    rt: Arc<Runtime>,
    func: FuncId,
    pkg: u32,
    size: usize,
    shared: Option<&Arc<Mutex<SharedState>>>,
) -> Result<InstData, SimError> {
    let mut m = Machine::new(rt, Arc::new(InstData::default()), Vec::new(), func);
    m.pkg_init = Some((pkg, vec![0; size]));
    let mut guard = match shared {
        Some(s) => Some(s.lock().map_err(|_| {
            SimError::new(ErrorKind::Internal("shared state lock poisoned".into()))
        })?),
        None => None,
    };
    if let Some(g) = guard.as_mut() {
        m.sh = Some(std::mem::take(&mut **g));
    }
    let r = m.enter(func, &[]).and_then(|()| m.run(None, 0));
    if let (Some(g), Some(st)) = (guard.as_mut(), m.sh.take()) {
        **g = st;
    }
    match r {
        Ok(Outcome::Returned(_)) => Ok(InstData {
            data: m.pkg_init.take().map(|p| p.1).unwrap_or_default(),
            ..Default::default()
        }),
        Ok(Outcome::Suspended) => Err(m.error(
            Fault::Runtime("wait statement executed during elaboration".into()),
            None,
        )),
        Err(e) => Err(m.error(e, None)),
    }
}

/// Identity and write count of the heap object a composite variable's data lives in (see
/// [`InterpProcess::var_is`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Stamp {
    addr: u64,
    generation: u32,
    writes: u32,
}

/// An argument or result of [`call_values`]: a register cell, or a composite (array or
/// record) as the bytes of its data (IR layout, bit-planes for logic arrays) and of its
/// descriptor (ir.md 1.3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CallVal {
    Cell(u64),
    Composite { data: Vec<u8>, desc: Vec<u8> },
}

/// [`call`] with composite arguments and results, for elaboration-time evaluation (#77). A
/// composite argument is copied into temporaries and passed as its two cells (data and
/// descriptor addresses). `ret` is the IR type of a composite result (returned as the
/// address of a fat pair `[data, desc]`), `None` for a scalar or no result.
pub fn call_values(
    rt: Arc<Runtime>,
    func: FuncId,
    inst: Arc<InstData>,
    args: &[CallVal],
    ret: Option<yavs_ir::TyId>,
) -> Result<Option<CallVal>, SimError> {
    call_values_shared(rt, func, inst, args, ret, None)
}

/// [`call_values`] with access to the shared state (see [`call_shared`]), for the thunks of
/// composite constants whose initialisers call protected methods.
pub fn call_values_shared(
    rt: Arc<Runtime>,
    func: FuncId,
    inst: Arc<InstData>,
    args: &[CallVal],
    ret: Option<yavs_ir::TyId>,
    shared: Option<&Arc<Mutex<SharedState>>>,
) -> Result<Option<CallVal>, SimError> {
    let prog = rt.prog.clone();
    let mut m = Machine::new(rt, inst, Vec::new(), func);
    let mut guard = match shared {
        Some(s) => Some(s.lock().map_err(|_| {
            SimError::new(ErrorKind::Internal("shared state lock poisoned".into()))
        })?),
        None => None,
    };
    if let Some(g) = guard.as_mut() {
        m.sh = Some(std::mem::take(&mut **g));
    }
    let r = (|| {
        let mut cells = Vec::with_capacity(args.len() * 2);
        for a in args {
            match a {
                CallVal::Cell(c) => cells.push(*c),
                CallVal::Composite { data, desc } => {
                    for b in [data, desc] {
                        let at = m.temp_alloc(b.len() as u64)?;
                        m.put(at, b)?;
                        cells.push(at);
                    }
                }
            }
        }
        m.enter(func, &cells)?;
        let v = match m.run(None, 0)? {
            Outcome::Returned(v) => v,
            Outcome::Suspended => {
                return Err(Fault::Runtime(
                    "wait statement executed during elaboration".into(),
                ));
            }
        };
        result(&mut m, &prog, ret, v)
    })();
    if let (Some(g), Some(st)) = (guard.as_mut(), m.sh.take()) {
        **g = st;
    }
    r.map_err(|e| m.error(e, None))
}

/// The result `v` of a call: a cell, or for a composite of IR type `ret` the fat pair `v`
/// points to, read back.
fn result(
    m: &mut Machine,
    prog: &Program,
    ret: Option<yavs_ir::TyId>,
    v: Option<u64>,
) -> exec::R<Option<CallVal>> {
    let (Some(ty), Some(fat)) = (ret, v) else {
        return Ok(v.map(CallVal::Cell));
    };
    let data = m.load(fat, yavs_ir::Width::B8)?;
    let d = m.load(exec::add_addr(fat, 8)?, yavs_ir::Width::B8)?;
    let n = yavs_ir::Ty::desc_size(&prog.types, ty)
        .ok_or_else(|| exec::ice("result type without a descriptor size"))?;
    let desc = m.rd(d, n)?.to_vec();
    let (l, _) = yavs_ir::Ty::layout(&prog.types, &prog.consts, ty, &desc)
        .map_err(|e| exec::ice(format!("result layout: {e:?}")))?;
    let data = m.rd(data, l.size)?.to_vec();
    Ok(Some(CallVal::Composite { data, desc }))
}

/// Run native `native` of `rt`'s program (an implicit operation, say) on `args` without a
/// kernel, for elaboration-time evaluation (#220): the natives' convention, composites as the
/// address of a fat pair `[data, desc]`. `ret` as for [`call_values`].
pub fn call_native(
    rt: Arc<Runtime>,
    native: yavs_ir::NativeId,
    args: &[CallVal],
    ret: Option<yavs_ir::TyId>,
) -> Result<Option<CallVal>, SimError> {
    let prog = rt.prog.clone();
    let mut m = Machine::new(rt, Arc::default(), Vec::new(), FuncId(0));
    let r = (|| {
        let n = prog
            .natives
            .get(native)
            .ok_or_else(|| exec::ice(format!("no native {native}")))?;
        let Some(fun) = m.rt.natives.get(native.0 as usize).copied().flatten() else {
            return Err(Fault::Runtime(format!(
                "native subprogram `{}` is not implemented in yavs yet",
                n.symbol
            )));
        };
        let mut cells = Vec::with_capacity(args.len());
        for a in args {
            match a {
                CallVal::Cell(c) => cells.push(*c),
                CallVal::Composite { data, desc } => {
                    let fat = m.temp_alloc(16)?;
                    for (k, b) in [data, desc].into_iter().enumerate() {
                        let at = m.temp_alloc(b.len() as u64)?;
                        m.put(at, b)?;
                        m.put(fat + 8 * k as u64, &at.to_le_bytes())?;
                    }
                    cells.push(fat);
                }
            }
        }
        let v = fun(&mut m, &mut None, n.ty, &cells)?;
        result(&mut m, &prog, ret, v)
    })();
    r.map_err(|e| m.error(e, None))
}

/// The dope of the array of `n` driving values passed to a resolution function: from the
/// left bound of the parameter's index subtype, in its direction (`down`), as most
/// reference simulators do (lrm-checks L101). `None` when the bounds overflow.
pub fn res_dope(left: i64, down: bool, n: u64) -> Option<[u8; 24]> {
    let last = i64::try_from(n).ok()?.checked_sub(1)?;
    let right = if down {
        left.checked_sub(last)?
    } else {
        left.checked_add(last)?
    };
    let mut d = [0u8; 24];
    for (k, x) in [left, right, i64::from(down)].into_iter().enumerate() {
        d[k * 8..k * 8 + 8].copy_from_slice(&x.to_le_bytes());
    }
    Some(d)
}

/// Run the resolution function `func` (one parameter: an unconstrained array of the element
/// type) over the driving values `vals` of one element and write the resolved scalar into
/// `out`. A byte element is passed in the kernel's byte representation, which is its memory
/// layout. A logic element (`logic`: the kind of the element type) is one kernel code per
/// value, passed as a logic array of bit-planes (ir.md 1.3) and returned as a position, which
/// is the code minus 2 for BIT and BOOLEAN. The array passed is [`res_dope`]`(left, down, n)`;
/// a byte element takes `elem` bytes in it (`Ty::elem_size` of the parameter's element: a
/// narrow integer is 4 bytes there even when the signal holds it in 8).
#[allow(clippy::too_many_arguments)]
pub fn resolve(
    rt: Arc<Runtime>,
    func: FuncId,
    inst: Arc<InstData>,
    vals: &[&[u8]],
    out: &mut [u8],
    logic: Option<yavs_ir::LogicKind>,
    (left, down): (i64, bool),
    elem: u64,
) -> Result<(), SimError> {
    let mut m = Machine::new(rt, inst, Vec::new(), func);
    let stride = out.len() as u64;
    let bias = match logic {
        Some(yavs_ir::LogicKind::StdULogic) | None => 0,
        Some(_) => 2,
    };
    let r = (|| {
        if vals.iter().any(|v| v.len() as u64 != stride) || (logic.is_some() && stride != 1) {
            return Err(exec::ice("driving value of the wrong size for resolution"));
        }
        let n = vals.len() as u64;
        let data = if logic.is_some() {
            // Row-major element j is plane bit n - 1 - j: four 64-bit planes per 64 elements.
            let mut planes = vec![0u8; (n.div_ceil(64) * 32) as usize];
            for (j, v) in vals.iter().enumerate() {
                let b = n - 1 - j as u64;
                for plane in 0..4 {
                    if v[0] >> plane & 1 == 1 {
                        planes[(b / 64 * 32 + plane * 8 + b % 64 / 8) as usize] |= 1 << (b % 8);
                    }
                }
            }
            let at = m.temp_alloc(planes.len() as u64)?;
            m.put(at, &planes)?;
            at
        } else {
            let es = elem.min(8);
            let at = m.temp_alloc(es * n)?;
            for (k, v) in vals.iter().enumerate() {
                m.put(
                    at + k as u64 * es,
                    &exec::cell(v).to_le_bytes()[..es as usize],
                )?;
            }
            at
        };
        let dope = m.temp_alloc(24)?;
        let d = res_dope(left, down, n).ok_or_else(|| {
            Fault::Runtime("too many drivers for the resolution function's index subtype".into())
        })?;
        m.put(dope, &d)?;
        m.enter(func, &[data, dope])?;
        m.run(None, 0)
    })();
    match r {
        Ok(Outcome::Returned(Some(v))) if logic.is_some() => {
            match u8::try_from(v).ok().and_then(|c| c.checked_add(bias)) {
                Some(c) if c < 9 => {
                    out[0] = c;
                    Ok(())
                }
                _ => Err(m.error(
                    exec::ice("logic resolution function returned an invalid position"),
                    None,
                )),
            }
        }
        Ok(Outcome::Returned(Some(v))) => {
            let n = out.len().min(8);
            out[..n].copy_from_slice(&v.to_le_bytes()[..n]);
            Ok(())
        }
        Ok(_) => Err(m.error(
            Fault::Runtime("resolution function returned no value".into()),
            None,
        )),
        Err(e) => Err(m.error(e, None)),
    }
}
