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
mod heap;
mod implicit;
mod native;
mod reflect;
mod textio;

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

pub use env::Sources;
use yavs_ffi::ForeignFn;
use yavs_ir::{ForeignId, FuncId, FuncKind, Program};
use yavs_kernel::{
    Ctx, DriverId, ErrorKind, ProcFlags, Process, Range, SignalId, SignalType, SimError,
};
use yavs_std::arith::TimeResolution;
use yavs_std::types::Revision;

pub use debug::{
    DebugSession, Debugger, FrameView, HitCond, LineBp, LocalView, ProcState, ProcStatus, Resume,
    SignalBreak, Stop, StopReason, format_scalar,
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
#[derive(Default)]
pub struct SharedState {
    /// Addressed by `AddrSpace::Shared`.
    pub vars: Vec<u8>,
    pub(crate) heap: Option<Heap>,
}

impl SharedState {
    pub fn new(vars: Vec<u8>) -> SharedState {
        SharedState {
            vars,
            heap: Some(Heap::new(heap::SHARED)),
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
            packages: Vec::new(),
            cov_counters: None,
            limits: Limits::default(),
            revision: Revision::Vhdl2008,
            resolution: TimeResolution::Fs,
            sources: Sources::default(),
        }
    }
}

/// A program bound for execution, shared by all processes of a simulation.
pub struct Runtime {
    pub prog: Arc<Program>,
    pub(crate) natives: Vec<Option<native::NativeFn>>,
    pub(crate) foreign: Vec<Option<ForeignFn>>,
    pub(crate) pkgs: Vec<Arc<InstData>>,
    pub(crate) cov: Vec<AtomicU64>,
    /// Compact constants expanded on first use by address.
    pub(crate) expanded: Vec<std::sync::OnceLock<Vec<u8>>>,
    pub(crate) limits: Limits,
    pub(crate) revision: Revision,
    pub(crate) resolution: TimeResolution,
    pub(crate) sources: Sources,
    /// VHDL assert counts and enables (`env.rs`).
    pub(crate) asserts: env::Asserts,
    /// Open file objects; a file cell holds index + 1 (0: never opened).
    pub(crate) files: Mutex<Vec<textio::FileSlot>>,
}

fn usage(m: String) -> SimError {
    SimError::new(ErrorKind::Usage(m))
}

impl Runtime {
    /// Fails early on a native symbol that `lib/NATIVE.md` does not define, an unbound
    /// foreign subprogram or a foreign function whose C signature differs from the IR's.
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
        if let Some((id, x)) = prog
            .foreign
            .iter()
            .find(|(id, _)| foreign[id.0 as usize].is_none())
        {
            return Err(usage(format!(
                "foreign subprogram `{}` ({id}): symbol `{}` is not bound",
                x.name, x.symbol
            )));
        }
        let n = b.cov_counters.unwrap_or(prog.cov.len());
        Ok(Arc::new(Runtime {
            natives,
            foreign,
            pkgs: b.packages,
            cov: (0..n).map(|_| AtomicU64::new(0)).collect(),
            expanded: (0..prog.consts.len()).map(|_| Default::default()).collect(),
            limits: b.limits,
            revision: b.revision,
            resolution: b.resolution,
            sources: b.sources,
            asserts: Default::default(),
            files: Mutex::new(textio::FileSlot::preopened()),
            prog,
        }))
    }

    /// The VHDL assert count of severity `s` (`GetVhdlAssertCount(s)`) as it stands now:
    /// the CLI's exit status follows the error and failure counts at the end of the run.
    pub fn assert_count(&self, s: yavs_kernel::Severity) -> u64 {
        self.asserts.counts()[s as usize]
    }

    /// Coverage counter values.
    pub fn coverage(&self) -> Vec<u64> {
        self.cov.iter().map(|c| c.load(Ordering::Relaxed)).collect()
    }
}

/// Kernel flags of a process function: serial when it touches shared state (ir.md 3.5).
pub fn proc_flags(prog: &Program, func: FuncId) -> ProcFlags {
    ProcFlags {
        postponed: false,
        serial: prog.is_serial(func),
    }
}

/// A process instance running IR in the interpreter.
pub struct InterpProcess {
    m: Machine,
    started: bool,
    done: bool,
    shared: Option<Arc<Mutex<SharedState>>>,
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
}

impl Process for InterpProcess {
    fn serial(&self) -> bool {
        self.m.rt.prog.is_serial(self.m.entry)
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
        let shared = self.shared.clone();
        let mut guard = match &shared {
            Some(s) => Some(s.lock().map_err(|_| {
                ctx.error(ErrorKind::Internal("shared state lock poisoned".into()))
            })?),
            None => None,
        };
        if let Some(g) = guard.as_mut() {
            self.m.sh = Some(std::mem::take(&mut **g));
        }
        let r = self.m.run(Some(&mut *ctx), 0);
        if matches!(r, Ok(Outcome::Suspended)) {
            self.m.recycle_shared();
        }
        if let (Some(g), Some(st)) = (guard.as_mut(), self.m.sh.take()) {
            **g = st;
        }
        match r {
            Ok(Outcome::Suspended) => {
                self.m.after_suspend();
                Ok(())
            }
            Ok(Outcome::Returned(_)) => {
                self.done = true;
                self.m.terminated();
                Ok(())
            }
            Err(e) => Err(self.m.error(e, Some(ctx))),
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
    })();
    if let (Some(g), Some(st)) = (guard.as_mut(), m.sh.take()) {
        **g = st;
    }
    r.map_err(|e| m.error(e, None))
}

/// Run the resolution function `func` (one parameter: an unconstrained array of the element
/// type) over the driving values `vals` (each in the kernel's byte representation, which is
/// the element's memory layout) and write the resolved scalar into `out`. The array passed
/// is indexed `0 to n - 1`.
pub fn resolve(
    rt: Arc<Runtime>,
    func: FuncId,
    inst: Arc<InstData>,
    vals: &[&[u8]],
    out: &mut [u8],
) -> Result<(), SimError> {
    let mut m = Machine::new(rt, inst, Vec::new(), func);
    let stride = out.len() as u64;
    let r = (|| {
        let data = m.temp_alloc(stride * vals.len() as u64)?;
        for (k, v) in vals.iter().enumerate() {
            if v.len() as u64 != stride {
                return Err(exec::ice("driving value of the wrong size for resolution"));
            }
            m.put(data + k as u64 * stride, v)?;
        }
        let dope = m.temp_alloc(24)?;
        let d: Vec<u8> = [0i64, vals.len() as i64 - 1, 0]
            .iter()
            .flat_map(|x| x.to_le_bytes())
            .collect();
        m.put(dope, &d)?;
        m.enter(func, &[data, dope])?;
        m.run(None, 0)
    })();
    match r {
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
