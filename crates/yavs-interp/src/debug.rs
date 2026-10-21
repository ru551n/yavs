//! Debug hooks (ir.md 3.4): what a sim4 / sim3 style debugger (DAP, M8) needs.
//!
//! - **Line breakpoints** on a `StmtId`, with an optional condition (an IR `Function` whose
//!   lexical parent is the function containing the statement, so it reads the stopped frame
//!   through `AddrSpace::Up(1)`) and a hit-count condition.
//! - **Stepping** within a process (in / over / out, by frame depth) and a **global step**
//!   that stops at the next statement executed by any process.
//! - **Pause** requests, polled every few thousand instructions, so an endless loop inside a
//!   process can be interrupted.
//! - **Process table**: every attached process publishes its state (running, waiting,
//!   terminated) and what it waits on (signals, absolute timeout, the wait statement, whose
//!   span shows the condition).
//! - **Signal breakpoints** are a kernel observer ([`SignalBreak`]) that pauses
//!   `SimControl` when a signal changes or reaches a value; run-for-time, next delta and next
//!   time step are `SimControl::run_for`, `step_delta` and `step_timestep`.
//!
//! Every stop reports the time and delta. Stops inside a process need `--threads 1` (the
//! kernel is mid-delta); the CLI forces it when a session is attached.

use std::cell::RefCell;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use yavs_ir::{FuncId, Program, StmtId, Ty, TyId};
use yavs_kernel::{Ctx, Hook, Interrupt, Observer, SigRef, SignalId, Time, Value};

use crate::CallVal;
use crate::exec::Machine;

/// What to do after a stop.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Resume {
    Continue,
    /// Stop at the next statement of this process, entering calls.
    StepIn,
    /// Stop at the next statement of this process in this frame or a caller.
    StepOver,
    /// Stop at the next statement of this process in a caller.
    StepOut,
    /// Stop at the next statement executed by any process.
    GlobalStep,
    /// Not a stop after all (a breakpoint condition the debugger evaluates did not hold):
    /// run on as before it, a step under way included.
    Skip,
}

/// When a line breakpoint fires, counting the hits where its condition held.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HitCond {
    Always,
    /// Exactly the n-th hit.
    Eq(u64),
    /// The n-th hit and every later one.
    Ge(u64),
    /// Every n-th hit.
    Multiple(u64),
}

/// A line breakpoint.
pub struct LineBp {
    /// A `Function` without parameters returning a boolean, nested in the statement's
    /// function.
    pub cond: Option<FuncId>,
    pub hit: HitCond,
    hits: AtomicU64,
}

impl LineBp {
    pub fn new(cond: Option<FuncId>, hit: HitCond) -> LineBp {
        LineBp {
            cond,
            hit,
            hits: AtomicU64::new(0),
        }
    }

    pub fn hits(&self) -> u64 {
        self.hits.load(Ordering::Relaxed)
    }

    /// Count a hit (its condition held); true when the breakpoint fires.
    pub(crate) fn hit(&self) -> bool {
        let n = self.hits.fetch_add(1, Ordering::Relaxed) + 1;
        match self.hit {
            HitCond::Always => true,
            HitCond::Eq(k) => n == k,
            HitCond::Ge(k) => n >= k,
            HitCond::Multiple(k) => k != 0 && n.is_multiple_of(k),
        }
    }
}

/// Why execution stopped.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StopReason {
    Breakpoint {
        hits: u64,
    },
    Step,
    GlobalStep,
    Pause,
    /// A breakpoint condition raised an error; the message says which.
    ConditionError(String),
}

/// A stop inside a process.
pub struct Stop<'a> {
    pub reason: StopReason,
    pub stmt: StmtId,
    /// Index in the session's process table.
    pub process: usize,
    pub now: Option<Time>,
    pub delta: Option<u32>,
    pub frames: FrameView<'a>,
    /// Reads current signal values; absent during elaboration.
    pub ctx: Option<&'a Ctx<'a>>,
}

/// The debugger front end.
pub trait Debugger: Send + Sync {
    /// Blocks while the user inspects.
    fn on_stop(&self, stop: &Stop<'_>, session: &DebugSession) -> Resume;
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ProcState {
    NotStarted,
    Running,
    Waiting,
    Terminated,
}

/// What a process waits on, published at each suspension.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ProcStatus {
    pub name: String,
    pub state: ProcState,
    /// The `wait` statement (its span shows the condition and timeout clause).
    pub wait: Option<StmtId>,
    pub sensitivity: Vec<SigRef>,
    /// Absolute timeout.
    pub timeout: Option<Time>,
}

/// One debug session, shared by every process of a simulation.
pub struct DebugSession {
    front: Box<dyn Debugger>,
    bits: Vec<AtomicU64>,
    lines: Mutex<HashMap<StmtId, Arc<LineBp>>>,
    global_step: AtomicBool,
    pause: AtomicBool,
    procs: Mutex<Vec<ProcStatus>>,
}

impl DebugSession {
    pub fn new(prog: &Program, front: Box<dyn Debugger>) -> Arc<DebugSession> {
        // Idempotent: the command line sets it before elaboration; for other users of the
        // library this is the earliest point (functions already optimised keep their registers).
        yavs_ir::opt::keep_variables();
        Arc::new(DebugSession {
            front,
            bits: (0..prog.stmts.len().div_ceil(64))
                .map(|_| AtomicU64::new(0))
                .collect(),
            lines: Mutex::new(HashMap::new()),
            global_step: AtomicBool::new(false),
            pause: AtomicBool::new(false),
            procs: Mutex::new(Vec::new()),
        })
    }

    /// Set a line breakpoint; false for a statement outside the program.
    pub fn set_breakpoint(&self, s: StmtId, bp: LineBp) -> Option<Arc<LineBp>> {
        let w = self.bits.get(s.0 as usize / 64)?;
        let bp = Arc::new(bp);
        self.lock_lines().insert(s, bp.clone());
        w.fetch_or(1 << (s.0 % 64), Ordering::Relaxed);
        Some(bp)
    }

    pub fn clear_breakpoint(&self, s: StmtId) {
        if let Some(w) = self.bits.get(s.0 as usize / 64) {
            w.fetch_and(!(1 << (s.0 % 64)), Ordering::Relaxed);
        }
        self.lock_lines().remove(&s);
    }

    fn lock_lines(&self) -> std::sync::MutexGuard<'_, HashMap<StmtId, Arc<LineBp>>> {
        self.lines.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub(crate) fn line(&self, s: StmtId) -> Option<Arc<LineBp>> {
        let set = self
            .bits
            .get(s.0 as usize / 64)
            .is_some_and(|w| w.load(Ordering::Relaxed) >> (s.0 % 64) & 1 == 1);
        if set {
            self.lock_lines().get(&s).cloned()
        } else {
            None
        }
    }

    /// Stop at the next statement executed by any process.
    pub fn request_global_step(&self) {
        self.global_step.store(true, Ordering::Relaxed);
    }

    pub(crate) fn take_global_step(&self) -> bool {
        self.global_step.swap(false, Ordering::Relaxed)
    }

    /// Stop the running process at its next statement.
    pub fn request_pause(&self) {
        // Release/acquire with `take_pause`: what the requester wrote first is visible to the
        // process that takes the pause (#441).
        self.pause.store(true, Ordering::Release);
    }

    /// Withdraw a pause request no process has taken (the simulation paused between cycles
    /// first).
    pub fn cancel_pause(&self) {
        self.pause.store(false, Ordering::Relaxed);
    }

    pub(crate) fn take_pause(&self) -> bool {
        self.pause.swap(false, Ordering::Acquire)
    }

    pub(crate) fn pause_pending(&self) -> bool {
        self.pause.load(Ordering::Relaxed)
    }

    /// All attached processes and what they wait on.
    pub fn processes(&self) -> Vec<ProcStatus> {
        self.lock_procs().clone()
    }

    fn lock_procs(&self) -> std::sync::MutexGuard<'_, Vec<ProcStatus>> {
        self.procs.lock().unwrap_or_else(|p| p.into_inner())
    }

    pub(crate) fn register(&self, name: &str) -> usize {
        let mut p = self.lock_procs();
        p.push(ProcStatus {
            name: name.to_string(),
            state: ProcState::NotStarted,
            wait: None,
            sensitivity: Vec::new(),
            timeout: None,
        });
        p.len() - 1
    }

    pub(crate) fn update(&self, i: usize, f: impl FnOnce(&mut ProcStatus)) {
        if let Some(s) = self.lock_procs().get_mut(i) {
            f(s);
        }
    }

    pub(crate) fn stop(&self, stop: &Stop<'_>) -> Resume {
        self.front.on_stop(stop, self)
    }
}

/// A stopped process's call stack, outermost frame first. Locals can be read and written
/// whole, composites included (`value`, `set_value`).
pub struct FrameView<'a> {
    pub(crate) m: RefCell<&'a mut Machine>,
    pub(crate) prog: &'a Program,
}

/// A named local of a frame and its slot bytes (a composite's slot holds its addresses).
pub struct LocalView<'a> {
    pub name: &'a str,
    pub ty: TyId,
    pub bytes: Vec<u8>,
}

impl<'a> FrameView<'a> {
    pub fn depth(&self) -> usize {
        self.m.borrow().frames.len()
    }

    pub fn func(&self, i: usize) -> Option<FuncId> {
        self.m.borrow().frames.get(i).map(|f| f.func)
    }

    pub fn func_name(&self, i: usize) -> Option<&'a str> {
        self.prog.func(self.func(i)?).map(|f| f.name.as_str())
    }

    /// The current statement of frame `i`.
    pub fn stmt(&self, i: usize) -> Option<StmtId> {
        let m = self.m.borrow();
        let f = m.frames.get(i)?;
        self.prog.func(f.func)?.loc.get(f.pc as usize).copied()
    }

    /// The locals of frame `i` (parameters and variables the IR describes).
    pub fn locals(&self, i: usize) -> Vec<LocalView<'a>> {
        let m = self.m.borrow();
        let Some(fr) = m.frames.get(i) else {
            return Vec::new();
        };
        let Some(f) = self.prog.func(fr.func) else {
            return Vec::new();
        };
        f.locals
            .iter()
            .filter_map(|l| {
                let bytes = m
                    .fslots(i)?
                    .get(l.slot as usize..l.slot as usize + l.size as usize)?;
                Some(LocalView {
                    name: &l.name,
                    ty: l.ty,
                    bytes: bytes.to_vec(),
                })
            })
            .collect()
    }

    /// The IR type and slot address of local `name` of frame `i` (the first of that name).
    fn slot(&self, i: usize, name: &str) -> Result<(TyId, u64), String> {
        let func = self.func(i).ok_or_else(|| format!("no frame {i}"))?;
        let l = self
            .prog
            .func(func)
            .and_then(|f| f.locals.iter().find(|l| l.name == name))
            .ok_or_else(|| format!("no local `{name}` in frame {i}"))?;
        let a = crate::exec::mk_addr(crate::exec::FRAME, i as u32, u64::from(l.slot))
            .map_err(crate::fault_text)?;
        Ok((l.ty, a))
    }

    /// The whole value of local `name` of frame `i`: a scalar as a cell, a composite as data
    /// and descriptor bytes.
    pub fn value(&self, i: usize, name: &str) -> Result<(TyId, CallVal), String> {
        let (ty, a) = self.slot(i, name)?;
        let v = crate::read_slot(&self.m.borrow(), ty, a).map_err(crate::fault_text)?;
        Ok((ty, v))
    }

    /// Writes `v` into local `name` of frame `i`; a composite keeps its bounds.
    pub fn set_value(&self, i: usize, name: &str, v: &CallVal) -> Result<(), String> {
        let (ty, a) = self.slot(i, name)?;
        crate::write_slot(&mut self.m.borrow_mut(), ty, a, v).map_err(crate::fault_text)
    }

    /// A scalar local formatted the way VHDL `'image` shows it.
    pub fn format(&self, ty: TyId, bytes: &[u8]) -> Option<String> {
        format_scalar(self.prog, ty, bytes)
    }
}

/// `'image`-style text of a scalar value; `None` for composites.
pub fn format_scalar(prog: &Program, ty: TyId, bytes: &[u8]) -> Option<String> {
    let v = crate::exec::cell(bytes);
    let text = match prog.types.get(ty)? {
        Ty::Int { .. } => (v as i64).to_string(),
        Ty::Physical { units, .. } => format!("{} {}", v as i64, units.first()?.0),
        Ty::Real => format!("{}", f64::from_bits(v)),
        Ty::Enum { lits, .. } => lits.get(v as usize)?.clone(),
        Ty::Access { .. } => {
            if v == 0 {
                "null".into()
            } else {
                format!("access {v:#x}")
            }
        }
        _ => return None,
    };
    Some(text)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum StepMode {
    Run,
    In,
    Over(usize),
    Out(usize),
}

impl StepMode {
    pub(crate) fn stops(self, depth: usize) -> bool {
        match self {
            StepMode::Run => false,
            StepMode::In => true,
            StepMode::Over(d) => depth <= d,
            StepMode::Out(d) => depth < d,
        }
    }

    pub(crate) fn after(r: Resume, depth: usize) -> StepMode {
        match r {
            Resume::Continue | Resume::GlobalStep | Resume::Skip => StepMode::Run,
            Resume::StepIn => StepMode::In,
            Resume::StepOver => StepMode::Over(depth),
            Resume::StepOut => StepMode::Out(depth),
        }
    }
}

/// The signals a [`SignalBreak`] watches, each on any change or on one value.
pub type Watches = Arc<Mutex<Vec<(SignalId, Option<Value>)>>>;

/// `(time, delta, signal)` of each [`SignalBreak`] hit.
pub type Hits = Arc<Mutex<Vec<(Time, u32, SignalId)>>>;

/// A signal breakpoint: pauses `SimControl` (through its `Interrupt`) after a cycle in which
/// a watched signal changed, or changed to a given value. Subscribe it with
/// `SimControl::subscribe`; `run_for` then returns `RunStatus::Paused` at that time and
/// delta.
pub struct SignalBreak {
    watches: Watches,
    interrupt: Interrupt,
    hits: Hits,
}

impl SignalBreak {
    pub fn new(interrupt: Interrupt) -> SignalBreak {
        SignalBreak {
            watches: Watches::default(),
            interrupt,
            hits: Arc::default(),
        }
    }

    /// Stop on any change of `s`, or only when it becomes `value`.
    pub fn watch(self, s: SignalId, value: Option<Value>) -> SignalBreak {
        if let Ok(mut w) = self.watches.lock() {
            w.push((s, value));
        }
        self
    }

    /// The watch list, to change while the simulation runs (a debugger's signal breakpoints).
    pub fn watches(&self) -> Watches {
        self.watches.clone()
    }

    /// `(time, delta, signal)` of every hit so far.
    pub fn hits(&self) -> Hits {
        self.hits.clone()
    }
}

impl Observer for SignalBreak {
    fn value_change(&mut self, h: &mut Hook<'_>, s: SignalId, _r: yavs_kernel::Range) {
        let Ok(watches) = self.watches.lock() else {
            return;
        };
        for (w, want) in watches.iter() {
            if *w != s {
                continue;
            }
            let fire = match want {
                None => true,
                Some(v) => h.value(s).is_ok_and(|cur| &cur == v),
            };
            if fire {
                if let Ok(mut hits) = self.hits.lock() {
                    hits.push((h.now(), h.delta(), s));
                }
                self.interrupt.pause();
            }
        }
    }
}
