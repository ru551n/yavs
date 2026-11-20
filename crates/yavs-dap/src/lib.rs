//! A Debug Adapter Protocol server over one simulation (plan M8, ir.md 3.4).
//!
//! Threads: a reader thread parses requests. Requests that do not need a stopped simulation
//! (`initialize`, `launch`, `setBreakpoints`, `threads`, `pause`, ...) are answered there;
//! the rest go over a channel to the simulation thread, which serves them while it is
//! stopped: inside a process (the interpreter's `Debugger::on_stop`) or between cycles (a
//! pause). Processes run on one thread, so a stop is a defined point of the simulation.
//!
//! DAP threads are processes (`threadId` = index in the session's process table + 1).
//! Frame ids are `threadId << 16 | frame` (outermost frame 0); variable references are
//! `frameId << 2 | scope` with scopes locals (1), signals (2) and generics (3).

mod calls;
mod cond;
mod console;
mod design;
mod layout;
mod tools;
mod waves;
mod wire;

use std::collections::HashMap;
use std::io::{BufReader, IsTerminal, Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use serde_json::{Value as Json, json};
use yavs_diag::{Diagnostic, MessageFormat};
use yavs_elab::{Built, Elaborated};
use yavs_interp::{
    DebugSession, Debugger, HitCond, Hits, LineBp, ProcState, Resume, SignalBreak, Stop,
    StopReason, Watches,
};
use yavs_kernel::{
    Config, EndReason, ForceMode, Interrupt, RunStatus, SigRef, SignalId, SimControl, Time, Value,
};
use yavs_sem::Db;
use yavs_std::value::{Lit, LitType, literal_image};

pub use console::HOOK_VAR;
use design::Design;
pub use wire::{read_message, write_message};

/// Code: the DAP connection cannot be opened or broke.
pub const IO_ERROR: &str = "E0701";

/// Where the client talks to the server.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Transport {
    /// Standard input and output (stdout carries only DAP messages).
    Stdio,
    /// One client on `127.0.0.1:PORT`; port 0 picks a free one.
    Tcp(u16),
}

static STDOUT: OnceLock<std::fs::File> = OnceLock::new();

/// The descriptor `Transport::Stdio` writes to, when the process diverted its own standard
/// output elsewhere (a plugin's prints must not reach the protocol, #389).
pub fn set_stdout(f: std::fs::File) {
    let _ = STDOUT.set(f);
}

/// Parse a `--dap` value: `stdio` or a port.
pub fn parse_transport(s: &str) -> Result<Transport, String> {
    if s == "stdio" {
        return Ok(Transport::Stdio);
    }
    s.parse()
        .map(Transport::Tcp)
        .map_err(|_| format!("expected a port number or `stdio`, found `{s}`"))
}

/// A run the kernel does next (`SimControl`), requested at a stop.
#[derive(Clone, Copy)]
enum Run {
    /// `run_until` this time (`yavs/runFor`, `yavs/runUntil`).
    Until(Time),
    /// `step_delta`; from a stop inside a process, the end of the current cycle.
    Delta,
    /// `step_timestep`.
    Timestep,
    /// `run_until` the stop time, whatever a stop on the way was running to (console `run`).
    ToEnd,
}

/// How a stop ends.
#[derive(Clone, Copy)]
enum Next {
    Resume(Resume),
    Run(Run),
    /// Back to time 0; `Shared::restart` says with which design.
    Restart,
    /// The console's `quit`.
    Quit,
}

/// What the run loop hands the requests it serves while stopped.
enum Msg {
    Request(Json),
    /// A message of the test runner's hook (`console::Hook`).
    Hook(Json),
    /// The test runner closed its hook.
    HookGone,
    Start,
    Eof,
}

/// What outlives one simulation of a session (`restart`, `reload`): the connection to the
/// client, what it asked for, and the console.
struct Link {
    out: Mutex<Box<dyn Write + Send>>,
    seq: Mutex<i64>,
    rx: Mutex<Receiver<Msg>>,
    stream: Arc<waves::Stream>,
    /// The `disconnect` request to answer last.
    disconnect: Mutex<Option<Json>>,
    /// A terminal or a script drives the session: stop at time 0 before anything has run, stay
    /// at the end of the run, say so at each stop.
    /// The width of a console's terminal (0: an editor, which wraps itself).
    columns: std::sync::atomic::AtomicUsize,
    /// The client shows ANSI styling.
    ansi: AtomicBool,
    console: AtomicBool,
    stop_on_entry: AtomicBool,
    /// The next run comes from a client's `restart`: it runs on, without the stop at time 0.
    client_restart: AtomicBool,
    /// End of input from the client.
    eof: AtomicBool,
    kept: Mutex<console::Kept>,
    con: Mutex<console::State>,
    /// The simulation of the moment.
    current: Mutex<Option<Arc<Shared>>>,
    viewer: OnceLock<Arc<dyn Viewer>>,
    hook: OnceLock<console::Hook>,
    /// Why `restart` is refused, if it is.
    no_restart: Option<String>,
    /// Told when a stop starts (`true`) and ends: VHPI's interactive callbacks.
    interactive: Option<fn(bool)>,
    /// A client that attached over TCP: the run cannot be started again from its side, so its
    /// restart button is `restart` here (#963).
    attached: bool,
    source_map: Vec<(String, String)>,
    reload: Mutex<Option<ReloadFn>>,
    /// Where this testbench's wave layout is kept.
    layout: Option<std::path::PathBuf>,
    hang_report: Option<usize>,
    assert_summary: bool,
    /// `--activity[=FILE]`.
    activity: Option<Option<String>>,
}

/// State shared by the reader thread, the simulation thread and the front end, for one simulation.
struct Shared {
    link: Arc<Link>,
    design: Design,
    session: OnceLock<Arc<DebugSession>>,
    interrupt: OnceLock<Interrupt>,
    /// The simulation thread serves requests.
    stopped: AtomicBool,
    /// Disconnect or end of input: let the simulation finish without stops.
    terminating: AtomicBool,
    /// A client pause the simulation has not stopped for yet.
    pause_req: AtomicBool,
    /// A process stopped for the pause, so the kernel's pause after its cycle is not a stop.
    paused_in_process: AtomicBool,
    /// This simulation is being left: what it still prints is dropped.
    discard: AtomicBool,
    /// The simulation has ended and the session stays (nothing runs any more).
    ended: AtomicBool,
    /// A restart under way, with the new design of a `reload`.
    restart: Mutex<Option<Option<Loaded>>>,
    /// A run requested at a stop inside a process, taken once its cycle has ended.
    queued: Mutex<Option<Run>>,
    /// Breakpoint statements per absolute file path, so a request replaces one file's set.
    bps: Mutex<HashMap<String, Vec<yavs_ir::StmtId>>>,
    /// Signal breakpoints: those of `setDataBreakpoints`, of `yavs/setSignalBreakpoints`, of the
    /// console's `break` and of its `run until` (`SIG_SETS`).
    sig_bps: Mutex<[Vec<SigBp>; SIG_SETS]>,
    /// What the kernel's `SignalBreak` watches (every leaf of `sig_bps`) and its hits.
    watches: OnceLock<Watches>,
    hits: OnceLock<Hits>,
    /// Forces, deposits and releases requested at a stop, for the run loop to hand to the
    /// kernel before it runs on.
    ops: Mutex<Vec<Op>>,
    prog: Arc<yavs_ir::Program>,
    /// What the variables references from `CHILD` on stand for; cleared at every stop.
    nodes: Mutex<Vec<Node>>,
    /// Line breakpoints with a condition, per statement.
    conds: Mutex<HashMap<yavs_ir::StmtId, Arc<LineCond>>>,
    /// Live waves (`waves`): the subscription's recorder and the batches it streams; the
    /// design and the shared state a recorder is built from.
    wave: Arc<Mutex<Option<waves::Sub>>>,
    db: Arc<Db>,
    elab: Arc<Elaborated>,
    built: OnceLock<Built>,
    sync: OnceLock<Sync>,
    /// The caller's `--wave` file.
    wave_file: OnceLock<String>,
    /// The port associations of the design (`tools`).
    ports: OnceLock<Vec<yavs_kernel::PortLink>>,
    /// The name of each kernel signal.
    signal_names: OnceLock<Vec<String>>,
    activity: OnceLock<Arc<yavs_kernel::profile::Activity>>,
    /// When each signal (by path) was last deposited.
    deposits: Mutex<HashMap<String, Time>>,
    /// The functions conditions have called (`calls`).
    calls: calls::Lowered,
}

/// Work at the end of the run, before `exited` (the caller's `--wave` recorder finishing its
/// file); a diagnostic fails the run.
pub type Finish = Box<dyn FnOnce(&SimControl) -> Option<Diagnostic>>;

/// Work at every stop, before `stopped` is sent (the caller's `--wave` file flushed, so an
/// editor's wave viewer can read every change up to the stop).
pub type Sync = Box<dyn Fn(Option<(u64, &dyn yavs_wave::Signals)>) + Send + std::marker::Sync>;

/// Work after the run, with the final kernel statistics and interpreter runtime.
pub type Report = Box<dyn FnOnce(yavs_kernel::Stats, Option<&yavs_interp::Runtime>)>;

/// What a waveform viewer (`--surfer`) is told of a stop, before `stopped` is sent.
pub struct ViewStop {
    pub now_fs: Option<u64>,
    pub breakpoint: bool,
    /// The signals of the stopped process, as `top.u1.s`, lower case.
    pub signals: Vec<String>,
}

/// Where a viewer's "go to declaration" goes, with the signal as `top.u1.s`.
pub type GotoFn = Box<dyn Fn(&str) + Send + std::marker::Sync>;

/// Where a viewer says what happened to it (it stalled, it answers again, it is gone), when it
/// happens.
pub type NoticeFn = Box<dyn Fn(&str) + Send + std::marker::Sync>;

/// Why a viewer's `add` did not add.
pub enum AddError {
    /// The waveform file does not hold the name.
    NotRecorded(String),
    /// The viewer did not answer in time, or is gone.
    Viewer(String),
}

impl AddError {
    fn text(self) -> String {
        match self {
            Self::NotRecorded(m) | Self::Viewer(m) => m,
        }
    }
}

/// A waveform viewer following the session; it never holds the simulation up, and a console's
/// `wave add` and `wave save` wait for it for a few seconds at most.
pub trait Viewer: Send + std::marker::Sync {
    fn stop(&self, s: ViewStop);
    /// Add signals or scopes (`top.u1.s`) to the view; how many were added.
    fn add(&self, items: Vec<String>) -> Result<usize, AddError>;
    fn cursor(&self, fs: u64);
    fn on_goto(&self, f: GotoFn);
    fn on_notice(&self, f: NoticeFn);
    /// What the view shows now, in its order (`top.u1.s`).
    fn items(&self) -> Result<Vec<String>, String>;
}

pub use console::{Command, Form, Needs, command_table, commands as console_commands, sentence};
pub use layout::{load as load_layout, save as save_layout};

/// What `attach` adds to one simulation: what to do at its end and at each stop.
pub struct Attached {
    pub finish: Option<Finish>,
    pub sync: Option<Sync>,
    pub viewer: Option<Arc<dyn Viewer>>,
    /// The `--wave` file of `sync`.
    pub wave_file: Option<String>,
    /// The counts of this simulation (`SimControl` has the observer).
    pub activity: Option<Arc<yavs_kernel::profile::Activity>>,
}

/// A design to run: what `reload` hands the session.
pub struct Loaded {
    pub db: Arc<Db>,
    pub e: Arc<Elaborated>,
    pub cfg: Config,
    pub stop: Time,
}

/// Elaborate again from the libraries, with these extra run flags; the error says why not, and
/// the session stays on the design it has.
pub type ReloadFn = Box<dyn FnMut(&[String]) -> Result<Loaded, String> + Send>;

/// The sets of `Shared::sig_bps`.
const SIG_SETS: usize = 4;
const CONSOLE_BREAKS: usize = 2;
const RUN_UNTIL: usize = 3;

/// The first variables reference of the per-stop table (`Shared::nodes`); the stateless
/// references of scopes stay below it.
const CHILD: i64 = 1 << 32;

/// A composite variable's children, or a hierarchy region (valid for one stop).
#[derive(Clone)]
enum Node {
    /// Local `name` of frame `frame` of thread `tid`, and the field and element positions to
    /// the composite shown.
    Local {
        tid: i64,
        frame: usize,
        name: String,
        path: Vec<usize>,
    },
    Signal {
        sig: usize,
        path: Vec<usize>,
    },
    /// A shared variable (`Design::shared`), and the path to the composite shown.
    Shared {
        var: usize,
        path: Vec<usize>,
    },
    /// The Shared Variables scope of thread `tid`.
    SharedScope(i64),
    /// Index into `Design::regions`.
    Region(usize),
    /// A value read at this stop: what an access value designates, a variable of a protected
    /// object, the result of an expression. Not written from the Variables view.
    Value(LitType, Lit),
}

impl Node {
    /// Child `k`, which holds `x` of type `t`.
    fn child(&self, k: usize, t: &LitType, x: &Lit) -> Node {
        if let Node::Value(..) = self {
            return Node::Value(t.clone(), x.clone());
        }
        self.below(k)
    }

    /// The node `k` further along the path of a local or a signal.
    fn below(&self, k: usize) -> Node {
        let mut n = self.clone();
        if let Node::Local { path, .. } | Node::Signal { path, .. } | Node::Shared { path, .. } =
            &mut n
        {
            path.push(k);
        }
        n
    }
}

/// A line breakpoint's condition (evaluated here, `cond`) and hit condition, counted over
/// the hits where the condition held.
struct LineCond {
    cond: cond::Cond,
    hit: HitCond,
    hits: std::sync::atomic::AtomicU64,
}

/// Whether the `n`-th hit fires under `h`.
fn fires(h: HitCond, n: u64) -> bool {
    match h {
        HitCond::Always => true,
        HitCond::Eq(k) => n == k,
        HitCond::Ge(k) => n >= k,
        HitCond::Multiple(k) => k != 0 && n.is_multiple_of(k),
    }
}

/// A signal breakpoint: a change of the signal, or a change to a value.
struct SigBp {
    /// Index into `Design::sigs`.
    sig: usize,
    value: Option<Lit>,
    cond: Option<cond::Cond>,
    hit: HitCond,
    /// Hits where the change, the value and the condition held.
    hits: u64,
    /// The thread whose names the condition uses (0: paths only).
    tid: i64,
}

/// What a force, deposit or release does to a signal's kernel leaves.
enum OpKind {
    Deposit,
    Force(ForceMode),
    Release(ForceMode),
}

/// A force, deposit or release of a signal or a part of one: its kernel elements and their new
/// values.
struct Op {
    kind: OpKind,
    leaves: Vec<(SigRef, Option<Value>)>,
}

impl Link {
    /// Send `m` after the queued wave batches, so events keep the order they were made in.
    fn send(&self, m: Json) {
        let mut out = self.out.lock().unwrap_or_else(|p| p.into_inner());
        self.drain(&mut out);
        self.write(&mut out, m);
    }

    /// Write the queued wave batches (`out` locked before the queue).
    fn drain(&self, out: &mut Box<dyn Write + Send>) {
        let q: Vec<_> = self
            .stream
            .queue
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .drain(..)
            .collect();
        for b in q {
            self.write(out, b.event());
        }
    }

    fn write(&self, out: &mut Box<dyn Write + Send>, mut m: Json) {
        let mut seq = self.seq.lock().unwrap_or_else(|p| p.into_inner());
        *seq += 1;
        m["seq"] = json!(*seq);
        // A broken connection ends the session through the reader's end of input.
        let _ = write_message(&mut **out, &m);
    }

    fn event(&self, event: &str, body: Json) {
        self.send(json!({"type": "event", "event": event, "body": body}));
    }

    fn output(&self, category: &str, text: &str) {
        self.event("output", json!({"category": category, "output": text}));
    }

    fn respond(&self, req: &Json, body: Json) {
        self.send(json!({
            "type": "response",
            "request_seq": req["seq"],
            "command": req["command"],
            "success": true,
            "body": body,
        }));
    }

    fn fail(&self, req: &Json, message: &str) {
        self.send(json!({
            "type": "response",
            "request_seq": req["seq"],
            "command": req["command"],
            "success": false,
            "message": message,
        }));
    }
}

impl Shared {
    fn send(&self, m: Json) {
        self.link.send(m);
    }

    fn lock_wave(&self) -> MutexGuard<'_, Option<waves::Sub>> {
        self.wave.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn event(&self, event: &str, body: Json) {
        self.send(json!({"type": "event", "event": event, "body": body}));
    }

    fn respond(&self, req: &Json, body: Json) {
        self.link.respond(req, body);
    }

    fn fail(&self, req: &Json, message: &str) {
        self.link.fail(req, message);
    }

    fn wave_path(&self) -> Option<&str> {
        self.wave_file.get().map(String::as_str)
    }

    /// Everything up to this stop is in the file.
    fn sync_wave(&self, at: &At<'_, '_>) {
        if let Some(s) = self.sync.get() {
            s(at.signals());
        }
    }

    fn signal_names(&self) -> Vec<String> {
        self.signal_names.get().cloned().unwrap_or_default()
    }

    fn deposited(&self, path: &str) -> Option<Time> {
        let d = self.deposits.lock().unwrap_or_else(|p| p.into_inner());
        d.get(path).copied()
    }

    fn session(&self) -> &Arc<DebugSession> {
        self.session.get().expect("set before the reader starts")
    }

    fn output(&self, category: &str, text: &str) {
        self.link.output(category, text);
    }
}

/// Where a stop happened.
enum At<'a, 'b> {
    Process(&'a Stop<'b>),
    /// Between cycles.
    Kernel(&'a SimControl),
}

impl At<'_, '_> {
    fn read(&self, r: SigRef) -> Option<Value> {
        match self {
            At::Process(s) => {
                let c = s.ctx?;
                c.logic(r)
                    .map(Value::Logic)
                    .or_else(|_| c.value(r).map(|b| Value::Bytes(b.to_vec())))
                    .ok()
            }
            At::Kernel(sim) => sim.value(r).ok(),
        }
    }

    fn now(&self) -> Option<Time> {
        match self {
            At::Process(s) => s.now,
            At::Kernel(sim) => Some(sim.now()),
        }
    }

    /// The time and what can be read at this stop; none during elaboration.
    fn signals(&self) -> Option<(u64, &dyn yavs_wave::Signals)> {
        match self {
            At::Process(s) => {
                s.ctx?;
                Some((s.now?.as_fs(), self))
            }
            At::Kernel(sim) => Some((sim.now().as_fs(), self)),
        }
    }

    /// The signal attributes of `r` at this stop.
    fn attrs(&self, r: SigRef) -> Result<yavs_kernel::SigAttrs, String> {
        let e = |e: yavs_kernel::SimError| e.to_string();
        match self {
            At::Process(s) => {
                let c = s.ctx.ok_or("no signal can be read here")?;
                Ok(yavs_kernel::SigAttrs {
                    event: c.event(r).map_err(e)?,
                    active: c.active(r).map_err(e)?,
                    last_event: c.last_event(r).map_err(e)?,
                    last_active: c.last_active(r).map_err(e)?,
                    last_value: c.last_value(r).map_err(e)?,
                })
            }
            At::Kernel(sim) => sim.attributes(r).map_err(e),
        }
    }

    /// The stopped process, if the stop is inside one.
    fn process(&self) -> Option<usize> {
        match self {
            At::Process(s) => Some(s.process),
            At::Kernel(_) => None,
        }
    }
}

impl yavs_wave::Signals for At<'_, '_> {
    fn view(&self, r: SigRef) -> Result<yavs_kernel::SigView<'_>, yavs_kernel::SimError> {
        match self {
            At::Process(s) => s.ctx.expect("`signals` checked the context").view(r),
            At::Kernel(sim) => sim.view(r),
        }
    }

    fn inside_process(&self) -> bool {
        matches!(self, At::Process(_))
    }

    fn stopped_var(
        &self,
        p: yavs_kernel::ProcId,
        name: &str,
    ) -> Option<(yavs_ir::TyId, yavs_interp::CallVal, &yavs_ir::Program)> {
        let At::Process(s) = self else {
            return None;
        };
        if s.ctx?.process() != p {
            return None;
        }
        let (ty, v) = s.frames.value(0, name).ok()?;
        Some((ty, v, s.frames.program()))
    }

    fn stopped_shared(
        &self,
        ty: yavs_ir::TyId,
        slot: u64,
    ) -> Option<(yavs_interp::CallVal, &yavs_ir::Program)> {
        let At::Process(s) = self else {
            return None;
        };
        let v = s.frames.shared_value(ty, slot).ok()?;
        Some((v, s.frames.program()))
    }

    fn with_process(
        &self,
        p: yavs_kernel::ProcId,
        f: &mut dyn FnMut(&mut dyn yavs_kernel::Process),
    ) -> bool {
        match self {
            At::Process(_) => false,
            At::Kernel(sim) => sim.with_process(p, f),
        }
    }
}

/// Serve the session's process stops.
struct Front(Arc<Shared>);

impl Debugger for Front {
    fn on_stop(&self, stop: &Stop<'_>, _session: &DebugSession) -> Resume {
        let sh = &self.0;
        if sh.terminating.load(Ordering::Relaxed) {
            return Resume::Continue;
        }
        let line = match &stop.reason {
            StopReason::Breakpoint { .. } => {
                let c = sh
                    .conds
                    .lock()
                    .unwrap_or_else(|p| p.into_inner())
                    .get(&stop.stmt)
                    .cloned();
                match c {
                    None => Ok(()),
                    Some(c) => {
                        let env = Names {
                            sh,
                            at: &At::Process(stop),
                            tid: stop.process as i64 + 1,
                            frame: stop.frames.depth().checked_sub(1),
                            region: None,
                        };
                        match c.cond.eval(&env) {
                            Ok(true)
                                if fires(c.hit, c.hits.fetch_add(1, Ordering::Relaxed) + 1) =>
                            {
                                Ok(())
                            }
                            // Not a stop: a step under way goes on.
                            Ok(_) => return Resume::Skip,
                            Err(m) => {
                                let m = console::explain(
                                    sh,
                                    &At::Process(stop),
                                    stop.process as i64 + 1,
                                    stop.frames.depth().saturating_sub(1),
                                    m,
                                );
                                Err(format!("breakpoint condition: {m}"))
                            }
                        }
                    }
                }
            }
            _ => Ok(()),
        };
        let (reason, text) = match &stop.reason {
            StopReason::Breakpoint { .. } => match line {
                Ok(()) => ("breakpoint", None),
                Err(m) => ("exception", Some(m)),
            },
            StopReason::Step => ("step", None),
            StopReason::GlobalStep if sh.link.stop_on_entry.swap(false, Ordering::Relaxed) => {
                ("entry", None)
            }
            StopReason::GlobalStep => ("step", None),
            StopReason::Pause => {
                sh.pause_req.store(false, Ordering::Relaxed);
                sh.paused_in_process.store(true, Ordering::Relaxed);
                ("pause", None)
            }
            StopReason::ConditionError(m) => ("exception", Some(m.clone())),
            StopReason::Assert { message, .. } => ("exception", Some(message.clone())),
        };
        let mut body = json!({
            "reason": reason,
            "threadId": stop.process + 1,
            "allThreadsStopped": true,
        });
        if let (Some(t), Some(d)) = (stop.now, stop.delta) {
            body["description"] = json!(format!("{reason} at {t}, delta {d}"));
        }
        if let Some(t) = text {
            body["text"] = json!(t);
        }
        if let StopReason::Assert { severity, .. } = &stop.reason
            && let (Some(t), Some(d)) = (stop.now, stop.delta)
        {
            let word = console::SEVERITIES[*severity as usize];
            body["description"] = json!(format!("assertion of severity {word} at {t}, delta {d}"));
        }
        match serve_stop(sh, &At::Process(stop), body) {
            Some(Next::Resume(r)) => r,
            // The kernel runs it once this cycle has ended.
            Some(Next::Run(run)) => {
                *sh.queued.lock().unwrap_or_else(|p| p.into_inner()) = Some(run);
                if let Some(i) = sh.interrupt.get() {
                    i.pause();
                }
                Resume::Continue
            }
            Some(Next::Restart | Next::Quit) | None => Resume::Continue,
        }
    }
}

/// Report the stop, then answer requests until one resumes; `None` ends the session.
fn serve_stop(sh: &Shared, at: &At<'_, '_>, stopped: Json) -> Option<Next> {
    // A plugin hears that the tool is interactive for as long as the stop lasts (#832).
    if let Some(f) = sh.link.interactive {
        f(true);
    }
    let next = stop_served(sh, at, stopped);
    if let Some(f) = sh.link.interactive {
        f(false);
    }
    next
}

fn stop_served(sh: &Shared, at: &At<'_, '_>, stopped: Json) -> Option<Next> {
    sh.nodes.lock().unwrap_or_else(|p| p.into_inner()).clear();
    sh.stopped.store(true, Ordering::Relaxed);
    // The waves of the completed time steps, then the stop.
    sh.link.stream.push();
    sh.sync_wave(at);
    console::tell_viewer(sh, at, &stopped);
    console::on_stop(sh, at, &stopped);
    sh.event("stopped", stopped);
    let rx = sh.link.rx.lock().unwrap_or_else(|p| p.into_inner());
    loop {
        if let Some(n) = console::work(sh, at) {
            return leave(sh, at, n, None);
        }
        let req = match rx.recv() {
            Ok(Msg::Request(r)) => r,
            Ok(Msg::Hook(m)) => {
                if let Some(n) = console::on_hook(sh, at, m) {
                    return leave(sh, at, n, None);
                }
                continue;
            }
            Ok(Msg::HookGone) => {
                console::hook_gone(sh);
                continue;
            }
            Ok(Msg::Start) => continue,
            Ok(Msg::Eof) | Err(_) => {
                sh.stopped.store(false, Ordering::Relaxed);
                return None;
            }
        };
        let ended = sh.ended.load(Ordering::Relaxed);
        let resume = match req["command"].as_str().unwrap_or_default() {
            "continue" | "next" | "stepIn" | "stepOut" | "yavs/runFor" | "yavs/runUntil"
            | "yavs/stepDelta" | "yavs/stepTimestep"
                if ended =>
            {
                sh.fail(&req, console::ENDED);
                None
            }
            "continue" => Some(Next::Resume(Resume::Continue)),
            "next" => Some(Next::Resume(Resume::StepOver)),
            "stepIn" => Some(Next::Resume(Resume::StepIn)),
            "stepOut" => Some(Next::Resume(Resume::StepOut)),
            "yavs/runFor" | "yavs/runUntil" => match run_to(at, &req) {
                Ok(t) => Some(Next::Run(Run::Until(t))),
                Err(m) => {
                    sh.fail(&req, &m);
                    None
                }
            },
            "yavs/stepDelta" => Some(Next::Run(Run::Delta)),
            "yavs/stepTimestep" => Some(Next::Run(Run::Timestep)),
            "restart" => restart_request(sh, &req).then_some(Next::Restart),
            "disconnect" | "terminate" => {
                sh.stopped.store(false, Ordering::Relaxed);
                stash_disconnect(sh, req);
                return None;
            }
            "stackTrace" => {
                stack_trace(sh, at, &req);
                None
            }
            "scopes" => {
                scopes(sh, at, &req);
                None
            }
            "variables" => {
                variables(sh, at, &req);
                None
            }
            "completions" => {
                console::completions(sh, at, &req);
                None
            }
            "yavs/find" => {
                let a = &req["arguments"];
                let list = tools::find_list(
                    sh,
                    a["kind"].as_str().unwrap_or("signals"),
                    a["pattern"].as_str().unwrap_or("*"),
                );
                sh.respond(&req, json!({"items": list}));
                None
            }
            "evaluate" => {
                if console::is_command(sh, &req) {
                    console::start(sh, req.clone());
                } else {
                    evaluate(sh, at, &req);
                }
                None
            }
            "yavs/force" | "yavs/deposit" | "yavs/release" => {
                signal_op(sh, at, &req);
                None
            }
            "setVariable" => {
                set_variable(sh, at, &req);
                None
            }

            c => {
                sh.fail(&req, &format!("`{c}` is not supported"));
                None
            }
        };
        if let Some(n) = resume {
            return leave(sh, at, n, Some(&req));
        }
    }
}

/// End a stop with `n`, answering the request that resumed, if a request did.
fn leave(sh: &Shared, at: &At<'_, '_>, n: Next, req: Option<&Json>) -> Option<Next> {
    // Between cycles every step is a global step: the next statement of any process.
    let n = match (at, n) {
        (At::Kernel(_), Next::Resume(r)) if r != Resume::Continue => {
            Next::Resume(Resume::GlobalStep)
        }
        (_, n) => n,
    };
    sh.stopped.store(false, Ordering::Relaxed);
    let tid = at.process().map_or(1, |p| p + 1);
    let continued = || {
        sh.event(
            "continued",
            json!({"threadId": tid, "allThreadsContinued": true}),
        )
    };
    match (n, req) {
        (Next::Resume(Resume::Continue), Some(req)) => {
            sh.respond(req, json!({"allThreadsContinued": true}))
        }
        (Next::Resume(_), Some(req)) => sh.respond(req, json!({})),
        // A client knows only its own requests resume: tell it.
        (_, Some(req)) => {
            sh.respond(req, json!({}));
            continued();
        }
        (_, None) => continued(),
    }
    if let Next::Quit = n {
        return None;
    }
    if let Next::Restart = n {
        sh.terminating.store(true, Ordering::Relaxed);
        sh.discard.store(true, Ordering::Relaxed);
        if let Some(i) = sh.interrupt.get() {
            i.stop();
        }
    }
    Some(n)
}

/// `restart` of a client: whether it restarts, with what `stopOnEntry` asks for now and otherwise
/// no stop at time 0, as after a launch (the console's `restart` stops there). A refusal is a notice
/// and a successful answer: VS Code takes a failed `restart` for the end of the session, and an
/// extension's requests to it no longer return.
fn restart_request(sh: &Shared, req: &Json) -> bool {
    if let Err(why) = console::restart(sh) {
        sh.output("important", &format!("{why}\n"));
        sh.respond(req, json!({}));
        return false;
    }
    let entry = req["arguments"]["arguments"]["stopOnEntry"] == true;
    sh.link.stop_on_entry.store(entry, Ordering::Relaxed);
    sh.link.client_restart.store(true, Ordering::Relaxed);
    true
}

/// The absolute time a `yavs/runFor` (relative) or `yavs/runUntil` (absolute) runs to.
fn run_to(at: &At<'_, '_>, req: &Json) -> Result<Time, String> {
    let text = req["arguments"]["time"]
        .as_str()
        .ok_or("expected a `time` argument such as \"10 ns\"")?;
    let t = console::parse_time(text)?;
    let now = at.now().ok_or("no simulation time at this stop")?;
    if req["command"] == "yavs/runFor" {
        return now.checked_add(t).map_err(|e| e.to_string());
    }
    if t < now {
        return Err(format!("{t} is before the current time {now}"));
    }
    Ok(t)
}

fn stash_disconnect(sh: &Shared, req: Json) {
    sh.terminating.store(true, Ordering::Relaxed);
    if let Some(i) = sh.interrupt.get() {
        i.stop();
    }
    *sh.link.disconnect.lock().unwrap_or_else(|p| p.into_inner()) = Some(req);
}

/// The source of a location, as DAP wants it.
fn source(path: &str) -> Json {
    let name = std::path::Path::new(path)
        .file_name()
        .map_or(path.to_string(), |n| n.to_string_lossy().into_owned());
    json!({"name": name, "path": path})
}

fn frame(sh: &Shared, id: i64, name: &str, stmt: Option<yavs_ir::StmtId>) -> Json {
    let mut f = json!({"id": id, "name": name, "line": 0, "column": 0});
    if let Some(l) = stmt.and_then(|s| sh.design.loc(s)) {
        f["source"] = source(&l.path);
        f["line"] = json!(l.line);
        f["column"] = json!(l.col);
    }
    f
}

fn stack_trace(sh: &Shared, at: &At<'_, '_>, req: &Json) {
    let tid = req["arguments"]["threadId"].as_i64().unwrap_or(0);
    let procs = sh.session().processes();
    let Some(p) = usize::try_from(tid - 1).ok().and_then(|i| procs.get(i)) else {
        return sh.fail(req, &format!("no thread {tid}"));
    };
    let mut frames = Vec::new();
    match at {
        At::Process(s) if s.process as i64 == tid - 1 => {
            let f = &s.frames;
            for i in (0..f.depth()).rev() {
                let stmt = if i + 1 == f.depth() {
                    Some(s.stmt)
                } else {
                    f.stmt(i)
                };
                let name = if i == 0 {
                    thread_label(sh, &p.name).to_string()
                } else {
                    f.func_name(i).unwrap_or("?").to_string()
                };
                frames.push(frame(sh, tid << 16 | i as i64, &name, stmt));
            }
        }
        // A process not executing shows where it waits.
        _ if p.state == ProcState::Waiting => {
            frames.push(frame(sh, tid << 16, thread_label(sh, &p.name), p.wait));
        }
        _ => {}
    }
    let n = frames.len();
    sh.respond(req, json!({"stackFrames": frames, "totalFrames": n}));
}

fn scopes(sh: &Shared, at: &At<'_, '_>, req: &Json) {
    let id = req["arguments"]["frameId"].as_i64().unwrap_or(0);
    let mut s = Vec::new();
    let live = at.process().is_some_and(|p| p as i64 == (id >> 16) - 1);
    if live {
        s.push(json!({"name": "Locals", "variablesReference": id << 2 | 1, "expensive": false}));
    }
    s.push(json!({"name": "Signals", "variablesReference": id << 2 | 2, "expensive": false}));
    s.push(json!({"name": "Generics", "variablesReference": id << 2 | 3, "expensive": false}));
    let tid = id >> 16;
    if thread_proc(sh, tid).is_some_and(|p| !p.shared.is_empty()) {
        let r = node_ref(sh, Node::SharedScope(tid));
        s.push(json!({"name": "Shared Variables", "variablesReference": r, "expensive": true}));
    }
    s.push(json!({"name": "Hierarchy", "variablesReference": id << 2, "expensive": true}));
    sh.respond(req, json!({"scopes": s}));
}

/// What the Threads view and the Call Stack call the process named `name`.
pub(crate) fn thread_label<'a>(sh: &'a Shared, name: &'a str) -> &'a str {
    sh.design.proc(name).map_or(name, |p| &p.label)
}

/// The process name of thread `tid`.
fn proc_name(sh: &Shared, tid: i64) -> Option<String> {
    let procs = sh.session().processes();
    Some(procs.get(usize::try_from(tid - 1).ok()?)?.name.clone())
}

/// The stop inside thread `tid`'s process, if it is that one.
fn stop_of<'a, 'b>(at: &'a At<'a, 'b>, tid: i64) -> Option<&'a Stop<'b>> {
    match at {
        At::Process(s) if s.process as i64 == tid - 1 => Some(s),
        _ => None,
    }
}

/// Local `name` of frame `i`: its type and value, and the value as the interpreter holds it.
fn local(
    sh: &Shared,
    s: &Stop<'_>,
    i: usize,
    name: &str,
) -> Option<(LitType, Lit, yavs_sem::stat::ConstValue)> {
    let (ty, v) = s.frames.value(i, name).ok()?;
    let c = yavs_elab::ir_decode_shown(&sh.prog, ty, &v)?;
    let (t, v) = design::ir_lit(&sh.prog, ty, &c)?;
    Some((t, v, c))
}

/// What an access value or a protected object designates; `None` for `null`, or where nothing
/// can read it. A stopped process reads its own heap and, if it holds the shared state, the shared
/// heap; else the shared heap is free to lock.
fn deref(sh: &Shared, at: &At<'_, '_>, t: &LitType, v: &Lit) -> Option<(LitType, Lit)> {
    let (LitType::Handle { target, protected }, Lit::Handle(h)) = (t, v) else {
        return None;
    };
    if *h == 0 {
        return None;
    }
    let ty = yavs_ir::TyId(*target);
    let shared = || {
        let b = sh.built.get()?;
        yavs_interp::shared_object(&b.rt, &b.shared, ty, *h).ok()
    };
    let cv = match at {
        At::Process(s) if s.frames.holds_shared() => s.frames.deref(ty, *h).ok(),
        At::Process(s) => s.frames.deref(ty, *h).ok().or_else(shared),
        At::Kernel(_) => shared(),
    }?;
    let c = yavs_elab::ir_decode_shown(&sh.prog, ty, &cv)?;
    let (t, v) = design::ir_lit(&sh.prog, ty, &c)?;
    // A protected state lists the body's variables only, not the object's own bookkeeping.
    match (protected, t, v) {
        (true, LitType::Record(mut fs), Lit::Record(mut xs)) => {
            let vars = (sh.prog.types.iter()).find_map(|(_, t)| match t {
                yavs_ir::Ty::Protected { state, vars } if *state == ty => Some(*vars as usize),
                _ => None,
            });
            fs.truncate(vars.unwrap_or(usize::MAX));
            xs.truncate(vars.unwrap_or(usize::MAX));
            Some((LitType::Record(fs), Lit::Record(xs)))
        }
        (_, t, v) => Some((t, v)),
    }
}

/// The type and value of a shared variable at this stop; `None`: it has no object (nothing of the
/// design uses its region).
fn shared_value(sh: &Shared, at: &At<'_, '_>, var: usize) -> Option<(LitType, Lit)> {
    let v = &sh.design.shared[var];
    let b = sh.built.get()?;
    let key = (v.region, v.decl);
    let cv = match (b.objects.get(&key), b.vars.get(&key), at) {
        (Some(&h), ..) => yavs_interp::CallVal::Cell(h),
        (_, Some(&(slot, _)), At::Process(s)) if s.frames.holds_shared() => {
            s.frames.shared_var(slot, v.ty).ok()?
        }
        (_, Some(&(slot, _)), _) => yavs_interp::shared_var(&b.rt, &b.shared, slot, v.ty).ok()?,
        _ => return None,
    };
    let c = yavs_elab::ir_decode_shown(&sh.prog, v.ty, &cv)?;
    design::ir_lit(&sh.prog, v.ty, &c)
}

/// A shared variable as a variable.
fn shared_var(sh: &Shared, at: &At<'_, '_>, var: usize) -> Json {
    let v = &sh.design.shared[var];
    let mut j = match shared_value(sh, at, var) {
        Some((t, x)) => value_var(sh, at, &v.name, &t, &x, &|| Node::Shared {
            var,
            path: Vec::new(),
        }),
        None => {
            json!({"name": v.name, "value": "no object: nothing uses its region", "variablesReference": 0})
        }
    };
    if let Some(t) = design::declared_type(&sh.db, v.decl) {
        j["type"] = json!(t);
    }
    j["evaluateName"] = json!(console::dotted(&v.path));
    j
}

/// The variables of the protected object whose method frame `i` is: the fields of its state.
fn state_vars(sh: &Shared, s: &Stop<'_>, at: &At<'_, '_>, i: usize) -> Vec<(String, LitType, Lit)> {
    let Some((t, v, _)) = local(sh, s, i, "self") else {
        return Vec::new();
    };
    match (
        matches!(
            t,
            LitType::Handle {
                protected: true,
                ..
            }
        ),
        deref(sh, at, &t, &v),
    ) {
        (true, Some((LitType::Record(fs), Lit::Record(xs)))) => fs
            .into_iter()
            .zip(xs)
            .map(|((n, t), x)| (n, t, x))
            .collect(),
        _ => Vec::new(),
    }
}

/// Write `v` (of the type `local` gives) into local `name` of frame `i`.
fn write_local(sh: &Shared, s: &Stop<'_>, i: usize, name: &str, v: &Lit) -> Result<(), String> {
    let (ty, _) = s.frames.value(i, name)?;
    let (_, _, like) =
        local(sh, s, i, name).ok_or_else(|| format!("`{name}` cannot be written here"))?;
    let bug = || format!("this is a bug in yavs: `{name}` not encoded");
    let c = design::to_const(v, &like).ok_or_else(bug)?;
    let cv = yavs_elab::ir_encode(&sh.prog, ty, &c).ok_or_else(bug)?;
    s.frames.set_value(i, name, &cv)
}

/// A reference to `n` for this stop.
fn node_ref(sh: &Shared, n: Node) -> i64 {
    let mut nodes = sh.nodes.lock().unwrap_or_else(|p| p.into_inner());
    nodes.push(n);
    CHILD + nodes.len() as i64 - 1
}

/// The element at `path` of a value.
fn descend(mut t: LitType, mut v: Lit, path: &[usize]) -> Option<(LitType, Lit)> {
    for &k in path {
        (t, v) = match (t, v) {
            (LitType::Record(mut fs), Lit::Record(mut xs)) if k < xs.len() => {
                (fs.swap_remove(k).1, xs.swap_remove(k))
            }
            (LitType::Array { elem, .. }, Lit::Array(mut xs)) if k < xs.len() => {
                (*elem, xs.swap_remove(k))
            }
            _ => return None,
        };
    }
    Some((t, v))
}

/// The whole value a node's local or signal holds, and its path into it.
fn node_root(sh: &Shared, at: &At<'_, '_>, n: &Node) -> Option<(LitType, Lit)> {
    match n {
        Node::Local {
            tid, frame, name, ..
        } => {
            let (t, v, _) = local(sh, stop_of(at, *tid)?, *frame, name)?;
            Some((t, v))
        }
        Node::Signal { sig, .. } => {
            let s = &sh.design.sigs[*sig];
            Some((s.lit.clone(), s.value(&|r| at.read(r))?))
        }
        Node::Shared { var, .. } => shared_value(sh, at, *var),
        Node::Region(_) | Node::SharedScope(_) => None,
        Node::Value(t, v) => Some((t.clone(), v.clone())),
    }
}

/// How a client writes what a node shows (`evaluateName` of its children).
fn node_text(sh: &Shared, at: &At<'_, '_>, n: &Node) -> Option<String> {
    let mut text = match n {
        Node::Local { name, .. } => name.clone(),
        Node::Signal { sig, .. } => console::dotted(&sh.design.sigs[*sig].path),
        Node::Shared { var, .. } => console::dotted(&sh.design.shared[*var].path),
        Node::Region(_) | Node::SharedScope(_) | Node::Value(..) => return None,
    };
    let (mut t, _) = node_root(sh, at, n)?;
    for &k in node_path(n) {
        t = match t {
            LitType::Record(mut fs) if k < fs.len() => {
                let (f, ft) = fs.swap_remove(k);
                text += &format!(".{f}");
                ft
            }
            LitType::Array { .. } => {
                text += &format!("({})", t.index(k as u64)?);
                let LitType::Array { elem, .. } = t else {
                    return None;
                };
                *elem
            }
            _ => return None,
        };
    }
    Some(text)
}

fn node_path(n: &Node) -> &[usize] {
    match n {
        Node::Local { path, .. } | Node::Signal { path, .. } | Node::Shared { path, .. } => path,
        Node::Region(_) | Node::SharedScope(_) | Node::Value(..) => &[],
    }
}

/// A variable showing value `v` of type `t`; a composite gets a reference to `node`, its
/// children (an array's paged through `indexedVariables`). An access value shows what it
/// designates, and expands as that does.
fn value_var(
    sh: &Shared,
    at: &At<'_, '_>,
    name: &str,
    t: &LitType,
    v: &Lit,
    node: &dyn Fn() -> Node,
) -> Json {
    if let Some((dt, dv)) = deref(sh, at, t, v) {
        let mut j = value_var(sh, at, name, &dt, &dv, &|| {
            Node::Value(dt.clone(), dv.clone())
        });
        if matches!(
            t,
            LitType::Handle {
                protected: true,
                ..
            }
        ) {
            j["value"] = json!("protected");
        }
        return j;
    }
    let image = literal_image(v, t);
    // A vector or a string is its one line: `v(3)` reads one element. Only what that line cuts
    // short keeps its children.
    let whole = image.ends_with('"');
    let mut j = json!({"name": name, "value": image, "variablesReference": 0});
    match (t, v) {
        (LitType::Array { .. }, Lit::Array(xs)) if !whole => {
            j["variablesReference"] = json!(node_ref(sh, node()));
            j["indexedVariables"] = json!(xs.len());
        }
        (LitType::Record(fs), _) => {
            j["variablesReference"] = json!(node_ref(sh, node()));
            j["namedVariables"] = json!(fs.len());
        }
        _ => {}
    }
    j
}

/// A signal as a variable.
fn signal_var(sh: &Shared, at: &At<'_, '_>, k: usize) -> Json {
    let s = &sh.design.sigs[k];
    let mut j = match s.value(&|r| at.read(r)) {
        Some(v) => value_var(sh, at, &s.name, &s.lit, &v, &|| Node::Signal {
            sig: k,
            path: Vec::new(),
        }),
        None => json!({"name": s.name, "value": "?", "variablesReference": 0}),
    };
    j["type"] = json!(s.ty);
    j["evaluateName"] = json!(console::dotted(&s.path));
    // VS Code's context key for the extension's force, deposit and release.
    j["__vscodeVariableMenuContext"] = json!("signal");
    j
}

/// The locals of frame `i` of thread `tid`, as variables.
fn local_vars(sh: &Shared, at: &At<'_, '_>, tid: i64, i: usize) -> Vec<Json> {
    let Some(s) = stop_of(at, tid) else {
        return Vec::new();
    };
    let locals = s.frames.locals(i);
    let mut vars: Vec<Json> = locals
        .iter()
        // The object of a protected method: its variables come below.
        .filter(|l| l.name != "self" || state_vars(sh, s, at, i).is_empty())
        .map(|l| match local(sh, s, i, l.name) {
            Some((t, v, _)) => {
                let mut j = value_var(sh, at, l.name, &t, &v, &|| Node::Local {
                    tid,
                    frame: i,
                    name: l.name.to_string(),
                    path: Vec::new(),
                });
                j["evaluateName"] = json!(l.name);
                j
            }
            // Files.
            None => {
                let v = s
                    .frames
                    .format(l.ty, &l.bytes)
                    .unwrap_or_else(|| "?".into());
                json!({"name": l.name, "value": v, "variablesReference": 0})
            }
        })
        .collect();
    vars.extend(
        state_vars(sh, s, at, i)
            .into_iter()
            .map(|(n, t, x)| value_var(sh, at, &n, &t, &x, &|| Node::Value(t.clone(), x.clone()))),
    );
    vars
}

/// The children of a region: regions, processes (with their state) and signals.
fn region_vars(sh: &Shared, at: &At<'_, '_>, regions: &[usize], r: Option<usize>) -> Vec<Json> {
    let mut out: Vec<Json> = regions
        .iter()
        .map(|&c| {
            let reg = &sh.design.regions[c];
            json!({
                "name": reg.name,
                "value": reg.kind,
                "variablesReference": node_ref(sh, Node::Region(c)),
                "presentationHint": {"kind": "class"},
            })
        })
        .collect();
    let Some(r) = r else {
        return out;
    };
    let procs = sh.session().processes();
    for p in &sh.design.regions[r].procs {
        let state = procs.iter().find(|s| s.name == *p).map(|s| match s.state {
            ProcState::NotStarted => "not started",
            ProcState::Running => "running",
            ProcState::Waiting => "waiting",
            ProcState::Terminated => "terminated",
        });
        // The label without its region, which the tree shows above it.
        let shown = thread_label(sh, p);
        let region = console::dotted(p.rsplit_once(':').map_or(p.as_str(), |x| x.0));
        let name = shown.strip_prefix(&format!("{region}.")).unwrap_or(shown);
        out.push(json!({
            "name": name,
            "value": format!("process, {}", state.unwrap_or("?")),
            "variablesReference": 0,
            "evaluateName": console::dotted(p),
        }));
    }
    out.extend(
        sh.design.regions[r]
            .sigs
            .iter()
            .map(|&k| signal_var(sh, at, k)),
    );
    let shared = sh.design.regions[r].shared.iter();
    out.extend(shared.map(|&k| shared_var(sh, at, k)));
    out
}

fn variables(sh: &Shared, at: &At<'_, '_>, req: &Json) {
    let a = &req["arguments"];
    let r = a["variablesReference"].as_i64().unwrap_or(0);
    let start = a["start"].as_u64().unwrap_or(0) as usize;
    let count = a["count"].as_u64().map_or(usize::MAX, |c| c as usize);
    if r >= CHILD {
        let node = sh
            .nodes
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get((r - CHILD) as usize)
            .cloned();
        let vars = match &node {
            None => return sh.fail(req, &format!("no variables {r}")),
            Some(Node::Region(g)) => region_vars(sh, at, &sh.design.regions[*g].children, Some(*g)),
            Some(Node::SharedScope(tid)) => thread_proc(sh, *tid)
                .into_iter()
                .flat_map(|p| &p.shared)
                .map(|&k| shared_var(sh, at, k))
                .collect(),
            Some(n) => match node_root(sh, at, n).and_then(|(t, v)| descend(t, v, node_path(n))) {
                Some((LitType::Record(fs), Lit::Record(xs))) => {
                    let base = node_text(sh, at, n);
                    fs.iter()
                        .zip(&xs)
                        .enumerate()
                        .map(|(k, ((f, t), x))| {
                            let mut j = value_var(sh, at, f, t, x, &|| n.child(k, t, x));
                            if let Some(b) = &base {
                                j["evaluateName"] = json!(format!("{b}.{f}"));
                            }
                            j
                        })
                        .collect()
                }
                Some((t, Lit::Array(xs))) => {
                    let LitType::Array { elem, .. } = &t else {
                        return sh.fail(req, "this is a bug in yavs: an array of no array type");
                    };
                    let base = node_text(sh, at, n);
                    xs.iter()
                        .enumerate()
                        .skip(start)
                        .take(count)
                        .map(|(k, x)| {
                            let i = t.index(k as u64).unwrap_or(k as i64);
                            let mut j =
                                value_var(sh, at, &i.to_string(), elem, x, &|| n.child(k, elem, x));
                            if let Some(b) = &base {
                                j["evaluateName"] = json!(format!("{b}({i})"));
                            }
                            j
                        })
                        .collect()
                }
                _ => Vec::new(),
            },
        };
        return sh.respond(req, json!({"variables": vars}));
    }
    let (frame, kind) = (r >> 2, r & 3);
    let (tid, i) = (frame >> 16, (frame & 0xffff) as usize);
    let proc = proc_name(sh, tid).and_then(|n| sh.design.proc(&n));
    let vars: Vec<Json> = match kind {
        0 => region_vars(sh, at, &sh.design.roots, None),
        1 => local_vars(sh, at, tid, i),
        2 => proc
            .into_iter()
            .flat_map(|p| &p.signals)
            .map(|&k| signal_var(sh, at, k))
            .collect(),
        3 => proc
            .into_iter()
            .flat_map(|p| &p.generics)
            .map(|(n, v)| json!({"name": n, "value": v, "variablesReference": 0}))
            .collect(),
        _ => return sh.fail(req, &format!("no variables {r}")),
    };
    sh.respond(req, json!({"variables": vars}));
}

/// The names a condition or an evaluation sees at a stop: the locals of frame `frame` of
/// thread `tid` and of its process frame, then signals by name seen from its process or by
/// path.
struct Names<'a, 'b, 'c> {
    sh: &'a Shared,
    at: &'a At<'b, 'c>,
    tid: i64,
    frame: Option<usize>,
    /// Without a process: the region (path prefix `:tb:u1:`) simple names are seen from.
    region: Option<&'a str>,
}

impl Names<'_, '_, '_> {
    /// The local `name` names (frame and exact name), or the signal or the shared variable.
    fn find(&self, name: &str) -> Option<Result<(usize, String), design::Named>> {
        if let Some(s) = stop_of(self.at, self.tid) {
            for i in self.frame.into_iter().chain([0]) {
                let l = s.frames.locals(i);
                if let Some(l) = l.iter().find(|l| l.name.eq_ignore_ascii_case(name)) {
                    return Some(Ok((i, l.name.to_string())));
                }
            }
        }
        let proc = proc_name(self.sh, self.tid).and_then(|n| self.sh.design.proc(&n));
        let in_region = || {
            let r = self.region?;
            self.sh.design.named(None, &format!("{r}{name}"))
        };
        in_region()
            .filter(|_| proc.is_none())
            .or_else(|| self.sh.design.named(proc, name))
            .map(Err)
    }
}

impl Names<'_, '_, '_> {
    /// A variable of the protected object of the frame.
    fn state_var(&self, name: &str) -> Option<(LitType, Lit)> {
        let s = stop_of(self.at, self.tid)?;
        state_vars(self.sh, s, self.at, self.frame?)
            .into_iter()
            .find(|(n, ..)| n.eq_ignore_ascii_case(name))
            .map(|(_, t, x)| (t, x))
    }
}

impl cond::Env for Names<'_, '_, '_> {
    fn get(&self, name: &str) -> Option<(LitType, Lit)> {
        match self.find(name) {
            Some(Ok((i, n))) => {
                let (t, v, _) = local(self.sh, stop_of(self.at, self.tid)?, i, &n)?;
                Some((t, v))
            }
            found => {
                if let Some(v) = self.state_var(name) {
                    return Some(v);
                }
                if let Some(Err(named)) = found {
                    return match named {
                        design::Named::Signal(k) => {
                            let s = &self.sh.design.sigs[k];
                            Some((s.lit.clone(), s.value(&|r| self.at.read(r))?))
                        }
                        design::Named::Shared(k) => shared_value(self.sh, self.at, k),
                    };
                }
                // A generic of the process's instance, or of the top level between cycles.
                let proc = proc_name(self.sh, self.tid);
                let all = self.sh.design.generics_at(proc.as_deref());
                let g = all.iter().find(|g| g.0.eq_ignore_ascii_case(name));
                g.map(|(_, t, x)| (t.clone(), x.clone()))
            }
        }
    }

    fn deref(&self, t: &LitType, v: &Lit) -> Option<(LitType, Lit)> {
        deref(self.sh, self.at, t, v)
    }

    fn type_name(&self, name: &str) -> Option<String> {
        match self.find(name)? {
            Err(design::Named::Signal(k)) => Some(self.sh.design.sigs[k].ty.clone()),
            Err(design::Named::Shared(k)) => {
                design::declared_type(&self.sh.db, self.sh.design.shared[k].decl)
            }
            // A variable of the process (frame 0), declared in its region.
            Ok((0, n)) => local_type(self.sh, &site(self.sh, self.tid), &n),
            Ok(_) => None,
        }
    }

    fn call(&self, name: &str, args: &[cond::Arg<'_>]) -> Option<Result<cond::Called, String>> {
        calls::call(self.sh, &site(self.sh, self.tid), name, args, Some(self.at))
    }

    fn signal_attr(&self, name: &str, attr: &str) -> Option<Result<cond::Called, String>> {
        // The longest prefix that names a signal; a local of that name hides it.
        for i in part_ends(name) {
            match self.find(&name[..i]) {
                Some(Ok(_)) => return None,
                Some(Err(design::Named::Signal(k))) => {
                    return Some(signal_attr(self.sh, self.at, k, &name[i..], attr));
                }
                _ => {}
            }
        }
        None
    }
}

/// The ends of the prefixes of `name` that may name an object (before a `.` or `(`), longest
/// first.
fn part_ends(name: &str) -> Vec<usize> {
    let mut ends: Vec<usize> = (name.char_indices())
        .filter(|&(_, c)| c == '.' || c == '(')
        .map(|(i, _)| i)
        .chain([name.len()])
        .collect();
    ends.reverse();
    ends
}

/// Attribute `attr` of signal `k`, or of its part `suffix`: `'event` and `'active` of any element,
/// the latest `'last_event` and `'last_active` of them, `'last_value` element by element.
fn signal_attr(
    sh: &Shared,
    at: &At<'_, '_>,
    k: usize,
    suffix: &str,
    attr: &str,
) -> Result<cond::Called, String> {
    let s = &sh.design.sigs[k];
    let time = |t: Time| {
        let fs = i64::try_from(t.as_fs()).unwrap_or(i64::MAX);
        let ty = LitType::Phys(yavs_std::types::time_type(), yavs_std::types::Bounds::I64);
        Some((ty, Lit::Phys(fs), None))
    };
    let boolean = |b: bool| {
        let ty = LitType::Enum(yavs_std::types::boolean_type());
        Some((ty, Lit::Enum(usize::from(b)), None))
    };
    if attr == "last_value" {
        let read = |r: SigRef| at.attrs(r).ok().map(|a| a.last_value);
        let v = s
            .value(&read)
            .ok_or("the last value of this signal cannot be read here")?;
        if suffix.is_empty() {
            return Ok(Some((s.lit.clone(), v, Some(s.ty.clone()))));
        }
        let (t, x) = part_of(&s.lit, &v, suffix)?;
        return Ok(Some((t, x, None)));
    }
    let refs: Vec<SigRef> = if suffix.is_empty() {
        let leaves = s.leaf_parts().map(|(signal, r)| SigRef {
            signal,
            range: Some(r),
        });
        leaves.collect()
    } else {
        let (paths, _) = select(&s.lit, suffix)?;
        let part = s
            .part(&paths, None)
            .ok_or("this part of the signal cannot be read here")?;
        part.into_iter().map(|(r, _)| r).collect()
    };
    let all = refs
        .into_iter()
        .map(|r| at.attrs(r))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(match attr {
        "event" => boolean(all.iter().any(|a| a.event)),
        "active" => boolean(all.iter().any(|a| a.active)),
        "last_event" => time(all.iter().map(|a| a.last_event).min().unwrap_or(Time::MAX)),
        _ => time(all.iter().map(|a| a.last_active).min().unwrap_or(Time::MAX)),
    })
}

/// Where a call seen from thread `tid` resolves its names: the process's region, or between
/// cycles the top level's.
fn site(sh: &Shared, tid: i64) -> calls::Site {
    let e = &sh.elab;
    site_of(
        sh,
        proc_name(sh, tid).and_then(|n| e.procs.iter().position(|p| e.proc_name(p) == n)),
    )
}

/// [`site`] of process instance `k` of the elaboration.
fn site_of(sh: &Shared, k: Option<usize>) -> calls::Site {
    let e = &sh.elab;
    let (inst, own) = match k {
        Some(k) => {
            let p = &e.procs[k];
            let own = match &sh.db.stmts[p.stmt].kind {
                yavs_sem::hir::StmtKind::Process { scope, .. } => Some(*scope),
                _ => None,
            };
            (
                e.hier.scopes.get(p.scope.index()).and_then(|s| s.parent),
                own,
            )
        }
        None => (Some(yavs_elab::ScopeId(0)), None),
    };
    let region = || {
        inst.and_then(|i| e.regions.get(&i))
            .and_then(|r| r.last().copied())
    };
    calls::Site {
        scope: own.or_else(region),
        inst,
    }
}

/// The name of the base type of variable `name` declared in the process region of `site`.
fn local_type(sh: &Shared, site: &calls::Site, name: &str) -> Option<String> {
    let key = yavs_syntax::Symbol::intern(&name.to_ascii_lowercase());
    let d = sh.db.scopes[site.scope?].names.get(&key)?.first()?;
    let yavs_sem::decl::DeclKind::Object(o) = &sh.db.decls[*d].kind else {
        return None;
    };
    Some(sh.db.types[sh.db.base(o.subtype)].name.text.to_string())
}

/// What a condition will see when it is read: the locals and the signals of the process (or
/// processes) it runs in. `None` is not known: a subprogram's caller decides.
struct Seen {
    locals: Option<Vec<String>>,
    sigs: Option<Vec<usize>>,
    /// The generics of the instances, with their values.
    generics: Vec<(String, LitType, Lit)>,
    /// Where calls resolve their names.
    site: calls::Site,
}

/// What a condition can know when it is made: the types of the signals, by a name that means one
/// signal (or several of one type) of the design or by path, and the names of the locals it may
/// read.
struct Typed<'a> {
    sh: &'a Shared,
    design: &'a Design,
    seen: Seen,
    /// The names met that wait for the stop to be checked.
    unchecked: std::cell::RefCell<Vec<String>>,
}

impl Typed<'_> {
    /// The signals a simple name can mean here.
    fn named(&self, name: &str) -> Vec<&design::Sig> {
        let all = self.design.sigs.iter().enumerate();
        let seen = |k: &usize| self.seen.sigs.as_ref().is_none_or(|s| s.contains(k));
        let named = |(_, s): &(usize, &design::Sig)| s.name.eq_ignore_ascii_case(name);
        all.filter(|(k, _)| seen(k))
            .filter(named)
            .map(|(_, s)| s)
            .collect()
    }

    /// The signal a name means, if it is one or all of one type.
    fn signal(&self, name: &str) -> Option<&design::Sig> {
        let named = self.named(name);
        match named.first() {
            Some(s) if named.iter().all(|o| o.lit == s.lit) => Some(*s),
            Some(_) => None,
            None => self.design.find(None, name).map(|k| &self.design.sigs[k]),
        }
    }
}

impl cond::Env for Typed<'_> {
    fn get(&self, name: &str) -> Option<(LitType, Lit)> {
        let Some(s) = self.signal(name) else {
            let g = (self.seen.generics.iter()).find(|g| g.0.eq_ignore_ascii_case(name));
            return g.map(|(_, t, x)| (t.clone(), x.clone()));
        };
        let t = s.lit.clone();
        let v = cond::zero(&t);
        Some((t, v))
    }

    fn type_name(&self, name: &str) -> Option<String> {
        Some(self.signal(name)?.ty.clone())
    }

    fn call(&self, name: &str, args: &[cond::Arg<'_>]) -> Option<Result<cond::Called, String>> {
        calls::call(self.sh, &self.seen.site, name, args, None)
    }

    fn signal_attr(&self, name: &str, attr: &str) -> Option<Result<cond::Called, String>> {
        let k = part_ends(name)
            .into_iter()
            .find(|&i| self.signal(&name[..i]).is_some())?;
        let sig = self.signal(&name[..k])?;
        let v = match attr {
            "event" | "active" => Some((
                LitType::Enum(yavs_std::types::boolean_type()),
                Lit::Enum(0),
                None,
            )),
            "last_value" if k == name.len() => {
                Some((sig.lit.clone(), cond::zero(&sig.lit), Some(sig.ty.clone())))
            }
            "last_value" => None,
            _ => Some((
                LitType::Phys(yavs_std::types::time_type(), yavs_std::types::Bounds::I64),
                Lit::Phys(0),
                None,
            )),
        };
        Some(Ok(v))
    }

    fn local(&self, name: &str) -> bool {
        let simple = !name.contains(['.', '(', ':']);
        let named = |l: &String| l.eq_ignore_ascii_case(name);
        // A name of signals of different types is read at the stop.
        let ambiguous = self.signal(name).is_none() && !self.named(name).is_empty();
        let known = match &self.seen.locals {
            Some(l) => l.iter().any(named),
            None => simple,
        };
        // A shared variable is read at the stop.
        let shared = self
            .design
            .shared
            .iter()
            .any(|v| v.name.eq_ignore_ascii_case(name))
            || matches!(
                self.design.named(None, name),
                Some(design::Named::Shared(_))
            );
        let known = known || ambiguous || shared;
        if known {
            let mut u = self.unchecked.borrow_mut();
            if !u.iter().any(named) {
                u.push(name.to_string());
            }
        }
        known
    }
}

/// What thread `tid`'s process sees (nothing for 0: between cycles, names are paths).
fn thread_seen(sh: &Shared, tid: i64) -> Seen {
    let Some(name) = proc_name(sh, tid) else {
        return Seen {
            locals: Some(Vec::new()),
            sigs: None,
            generics: sh.design.generics_at(None).to_vec(),
            site: site_of(sh, None),
        };
    };
    let t = sh.elab.proc_table();
    let k = (sh.elab.procs.iter()).position(|p| sh.elab.proc_name(p) == name);
    let func = k.and_then(|k| t.func(yavs_kernel::ProcId(k as u32)));
    Seen {
        locals: Some(
            func.and_then(|f| sh.prog.func(f))
                .map_or_else(Vec::new, locals_of),
        ),
        sigs: sh.design.proc(&name).map(|p| p.signals.clone()),
        generics: sh.design.generics_at(Some(&name)).to_vec(),
        site: site_of(sh, k),
    }
}

/// What a condition on a signal sees when it is read: a change is read between cycles, where no
/// process's variables are.
fn kernel_seen(sh: &Shared, tid: i64) -> Seen {
    Seen {
        locals: Some(Vec::new()),
        ..thread_seen(sh, tid)
    }
}

fn locals_of(f: &yavs_ir::Func) -> Vec<String> {
    let own = |n: &&String| !n.starts_with('_') && n.as_str() != "self";
    (f.locals.iter().map(|l| &l.name))
        .filter(own)
        .cloned()
        .collect()
}

/// What a condition on these statements sees: that of their processes. Unknown when a statement
/// is in a subprogram: the process that calls it has its own.
fn line_seen(sh: &Shared, stmts: &[yavs_ir::StmtId]) -> Seen {
    let t = sh.elab.proc_table();
    let procs: Vec<_> = (0..sh.elab.procs.len())
        .map(|k| (k, t.func(yavs_kernel::ProcId(k as u32))))
        .collect();
    let mut seen = Seen {
        locals: Some(Vec::new()),
        sigs: Some(Vec::new()),
        generics: Vec::new(),
        site: calls::Site::default(),
    };
    for &s in stmts {
        let Some(f) = sh.prog.stmts.get(s).map(|st| st.func) else {
            continue;
        };
        let own: Vec<_> = procs.iter().filter(|(_, g)| *g == Some(f)).collect();
        if own.is_empty() {
            return Seen {
                locals: None,
                sigs: None,
                generics: Vec::new(),
                site: calls::Site::default(),
            };
        }
        seen.locals.iter_mut().for_each(|l| {
            l.extend(sh.prog.func(f).map(locals_of).unwrap_or_default());
        });
        for (k, _) in own {
            if seen.site.scope.is_none() {
                seen.site = site_of(sh, Some(*k));
            }
            let name = sh.elab.proc_name(&sh.elab.procs[*k]);
            let sigs = sh.design.proc(&name).map(|p| p.signals.clone());
            seen.sigs
                .iter_mut()
                .for_each(|v| v.extend(sigs.iter().flatten()));
            if let Some(p) = sh.design.proc(&name) {
                seen.generics.extend(p.generic_lits.iter().cloned());
            }
        }
    }
    seen
}

/// Parse a condition and check what can be known now (`Cond::check_types`), with the help the
/// evaluation gives. The names left for the stop come back with it.
fn make_cond(sh: &Shared, text: &str, seen: Seen) -> Result<(cond::Cond, Vec<String>), String> {
    let c = cond::parse(text)?;
    let env = Typed {
        sh,
        design: &sh.design,
        seen,
        unchecked: Default::default(),
    };
    c.check_types(&env).map_err(|e| {
        let word = e
            .strip_prefix("no local or signal `")
            .and_then(|r| r.split('`').next());
        let mut names: Vec<String> = sh.design.sigs.iter().map(|s| s.name.clone()).collect();
        names.extend(env.seen.locals.iter().flatten().cloned());
        names.extend(env.seen.generics.iter().map(|g| g.0.clone()));
        let help = word.and_then(|w| console::similar(w, "signal or variable", &names));
        let help = help.map_or(String::new(), |h| format!(": {h}"));
        format!("condition `{text}`: {e}{help}")
    })?;
    Ok((c, env.unchecked.into_inner()))
}

/// A local of the frame, or a signal by name or path: its value (a composite expands).
fn evaluate(sh: &Shared, at: &At<'_, '_>, req: &Json) {
    let a = &req["arguments"];
    let expr = a["expression"].as_str().unwrap_or_default().trim();
    let (tid, i) = match a["frameId"].as_i64() {
        Some(f) => (f >> 16, (f & 0xffff) as usize),
        None => (at.process().map_or(0, |p| p as i64 + 1), 0),
    };
    match eval_var(sh, at, tid, i, expr) {
        Ok(j) => sh.respond(req, j),
        Err(m) => sh.fail(req, &m),
    }
}

/// The value of `expr` seen from frame `i` of thread `tid`, as an `evaluate` response body.
fn eval_var(sh: &Shared, at: &At<'_, '_>, tid: i64, i: usize, expr: &str) -> Result<Json, String> {
    let top = (tid == 0)
        .then(|| sh.design.roots.first())
        .flatten()
        .map(|&r| sh.design.regions[r].path.as_str());
    let names = Names {
        sh,
        at,
        tid,
        frame: Some(i),
        region: top,
    };
    let found = names.find(expr);
    let mut j = match found {
        Some(Ok((frame, name))) => {
            let s = stop_of(at, tid).ok_or("no locals here")?;
            match local(sh, s, frame, &name) {
                Some((t, v, _)) => value_var(sh, at, &name, &t, &v, &|| Node::Local {
                    tid,
                    frame,
                    name: name.clone(),
                    path: Vec::new(),
                }),
                None => {
                    let l = s.frames.locals(frame);
                    let l = l.iter().find(|l| l.name == name);
                    let v = l.and_then(|l| s.frames.format(l.ty, &l.bytes));
                    json!({"value": v.unwrap_or_else(|| "?".into()), "variablesReference": 0})
                }
            }
        }
        Some(Err(design::Named::Signal(k))) if names.state_var(expr).is_none() => {
            signal_var(sh, at, k)
        }
        Some(Err(design::Named::Shared(k))) if names.state_var(expr).is_none() => {
            shared_var(sh, at, k)
        }
        // A variable by the path `find` lists: a process has no frame to expand it from.
        _ if let Some(shown) = tools::examine_path(sh, at, &console::dotted(expr)) => {
            json!({"value": shown?, "variablesReference": 0})
        }
        // A part of a local or a signal, or any expression: what a condition computes.
        _ => {
            let unknown = || {
                let help = console::name_help(sh, at, tid, i, expr)
                    .or_else(|| console::did_you_mean(sh, expr));
                let help = help.map_or(String::new(), |h| format!(": {h}"));
                format!("no variable or signal `{expr}` here{help}")
            };
            // A text that parses but uses what conditions do not support says so.
            let (t, v) = cond::parse(expr)
                .map_err(|e| match e.contains("does not parse") {
                    true => unknown(),
                    false => e,
                })?
                .evaluate(&names)
                .map_err(|e| e.map_or_else(unknown, |e| console::explain(sh, at, tid, i, e)))?;
            value_var(sh, at, expr, &t, &v, &|| Node::Value(t.clone(), v.clone()))
        }
    };
    // A variable becomes a result.
    if let Some(o) = j.as_object_mut() {
        o.remove("name");
        o.remove("__vscodeVariableMenuContext");
        if let Some(v) = o.remove("value") {
            o.insert("result".into(), v);
        }
    }
    Ok(j)
}

/// `yavs/force`, `yavs/deposit` and `yavs/release` of a whole signal (`signal`: a name seen
/// from `frameId` or the stopped process, or a path; `value`: a literal of its type; `mode`:
/// `in`, the default, or `out`).
fn signal_op(sh: &Shared, at: &At<'_, '_>, req: &Json) {
    let cmd = req["command"].as_str().unwrap_or_default();
    match signal_change(sh, at, cmd, &req["arguments"]) {
        Ok(b) => sh.respond(req, b),
        Err(m) => sh.fail(req, &m),
    }
}

/// What `signal_op` does, for the request's arguments.
fn signal_change(sh: &Shared, at: &At<'_, '_>, cmd: &str, a: &Json) -> Result<Json, String> {
    {
        let name = a["signal"].as_str().ok_or("expected a `signal` argument")?;
        let tid = match a["frameId"].as_i64() {
            Some(f) => f >> 16,
            None => at.process().map_or(0, |p| p as i64 + 1),
        };
        let (k, suffix) = split_part(sh, thread_proc(sh, tid), name)
            .ok_or_else(|| console::no_signal(sh, tid, name))?;
        let s = &sh.design.sigs[k];
        let (paths, ty) = select(&s.lit, suffix)?;
        let target = format!(
            "{}{}",
            console::dotted(&s.path),
            suffix.to_ascii_lowercase()
        );
        let mode = match a["mode"].as_str() {
            None | Some("in") => ForceMode::Effective,
            Some("out") => ForceMode::Driving,
            Some(m) => return Err(format!("force mode `{m}`: expected `in` or `out`")),
        };
        if cmd == "yavs/release" {
            put(sh, at, k, &paths, None, OpKind::Release(mode))?;
            // No mode asked for: whichever force it is, so nobody needs to remember which.
            if a["mode"].is_null() {
                put(sh, at, k, &paths, None, OpKind::Release(ForceMode::Driving))?;
            }
            return Ok(json!({"signal": target}));
        }
        let text = a["value"].as_str().ok_or("expected a `value` argument")?;
        let v = design::typed_literal(text, &ty).map_err(|hint| {
            let of = if suffix.is_empty() {
                "type"
            } else {
                "a part of type"
            };
            format!(
                "`{text}` is not a value of `{target}`, {of} {}: {hint}",
                s.declared
            )
        })?;
        let kind = if cmd == "yavs/force" {
            OpKind::Force(mode)
        } else {
            OpKind::Deposit
        };
        let mut whole = s
            .value(&|r| at.read(r))
            .ok_or("the signal is not readable here")?;
        set_part(&mut whole, &paths, &v)?;
        if cmd == "yavs/deposit"
            && let Some(t) = at.now()
        {
            let mut d = sh.deposits.lock().unwrap_or_else(|p| p.into_inner());
            d.insert(s.path.clone(), t);
        }
        put(sh, at, k, &paths, Some(&whole), kind)?;
        Ok(json!({"signal": target, "value": literal_image(&v, &ty)}))
    }
}

/// `name` as a signal and the part of it after its name (`.a`, `(3)`, `(7 downto 4)`): the
/// longest prefix that names a signal.
fn split_part<'n>(
    sh: &Shared,
    p: Option<&design::Proc>,
    name: &'n str,
) -> Option<(usize, &'n str)> {
    let name = name.trim();
    let ends = name
        .char_indices()
        .filter(|&(_, c)| c == '.' || c == '(')
        .map(|(i, _)| i)
        .chain([name.len()]);
    let ends: Vec<usize> = ends.collect();
    ends.into_iter()
        .rev()
        .find_map(|i| Some((sh.design.find(p, &name[..i])?, &name[i..])))
}

/// The positions of the sub-elements `suffix` selects in a value of type `t` (one path per
/// element of a slice) and the type of the part.
fn select(t: &LitType, suffix: &str) -> Result<(Vec<Vec<usize>>, LitType), String> {
    let bad = || format!("`{suffix}` is not a field, element or slice of this signal");
    let mut path = Vec::new();
    let mut t = t.clone();
    let mut rest = suffix.trim();
    while !rest.is_empty() {
        if let Some(r) = rest.strip_prefix('.') {
            let end = r.find(['.', '(']).unwrap_or(r.len());
            let LitType::Record(fs) = t else {
                return Err(bad());
            };
            let f = r[..end].trim();
            let k = fs
                .iter()
                .position(|(n, _)| n.eq_ignore_ascii_case(f))
                .ok_or_else(|| {
                    let cands = fs.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>();
                    let help = console::similar(f, "field", &cands);
                    format!(
                        "no field `{f}`{}",
                        help.map_or(String::new(), |h| format!(": {h}"))
                    )
                })?;
            path.push(k);
            t = fs[k].1.clone();
            rest = r[end..].trim_start();
            continue;
        }
        let r = rest.strip_prefix('(').ok_or_else(bad)?;
        let end = r.find(')').ok_or_else(bad)?;
        let (inner, after) = (r[..end].trim(), r[end + 1..].trim_start());
        let LitType::Array {
            elem,
            len,
            ascending,
            ..
        } = &t
        else {
            return Err(bad());
        };
        let pos = |x: &str| -> Result<u64, String> {
            let i: i64 = x
                .trim()
                .parse()
                .map_err(|_| format!("index `{}`: expected an integer", x.trim()))?;
            (0..*len)
                .find(|&k| t.index(k) == Some(i))
                .ok_or_else(|| format!("index {i} is out of range"))
        };
        let lower = inner.to_ascii_lowercase();
        let slice = lower
            .split_once(" downto ")
            .map(|(a, b)| (a, b, false))
            .or_else(|| lower.split_once(" to ").map(|(a, b)| (a, b, true)));
        if let Some((l, r, asc)) = slice {
            if !after.is_empty() {
                return Err(format!("`{after}`: a slice must be last"));
            }
            if asc != *ascending {
                return Err(format!(
                    "`{inner}`: the direction differs from the signal's"
                ));
            }
            let (a, b) = (pos(l)?, pos(r)?);
            if b < a {
                return Err(format!("`{inner}` is a null slice"));
            }
            let paths = (a..=b)
                .map(|k| path.iter().copied().chain([k as usize]).collect())
                .collect();
            let ty = LitType::Array {
                left: l.trim().parse().map_err(|_| bad())?,
                ascending: asc,
                len: b - a + 1,
                elem: elem.clone(),
            };
            return Ok((paths, ty));
        }
        path.push(pos(inner)? as usize);
        t = (**elem).clone();
        rest = after;
    }
    Ok((vec![path], t))
}

/// The part `suffix` selects of value `v` of type `t`.
fn part_of(t: &LitType, v: &Lit, suffix: &str) -> Result<(LitType, Lit), String> {
    let (paths, ty) = select(t, suffix)?;
    let at_path = |p: &[usize]| {
        let mut cur = v;
        for &i in p {
            cur = match cur {
                Lit::Record(xs) | Lit::Array(xs) => xs.get(i)?,
                _ => return None,
            };
        }
        Some(cur.clone())
    };
    let part = if let LitType::Array { .. } = &ty
        && paths.len() != 1
    {
        Lit::Array(paths.iter().filter_map(|p| at_path(p)).collect())
    } else {
        at_path(&paths[0]).ok_or("the part is not readable here")?
    };
    Ok((ty, part))
}

/// Write `v` (one value, or an array of one per path) into `whole` at `paths`.
fn set_part(whole: &mut Lit, paths: &[Vec<usize>], v: &Lit) -> Result<(), String> {
    let parts: Vec<&Lit> = match (paths.len(), v) {
        (1, _) => vec![v],
        (_, Lit::Array(xs)) if xs.len() == paths.len() => xs.iter().collect(),
        _ => return Err("this is a bug in yavs: a slice value of another length".into()),
    };
    for (p, x) in paths.iter().zip(parts) {
        let mut cur = &mut *whole;
        for &k in p {
            cur = match cur {
                Lit::Record(xs) | Lit::Array(xs) => xs
                    .get_mut(k)
                    .ok_or("this is a bug in yavs: a part out of the value")?,
                _ => return Err("this is a bug in yavs: a path into a scalar".into()),
            };
        }
        *cur = x.clone();
    }
    Ok(())
}

/// Queue a force, deposit (with `whole`, the signal's new value) or release of the parts of
/// signal `k` at `paths`.
fn put(
    sh: &Shared,
    at: &At<'_, '_>,
    k: usize,
    paths: &[Vec<usize>],
    whole: Option<&Lit>,
    kind: OpKind,
) -> Result<(), String> {
    let s = &sh.design.sigs[k];
    let leaves = s.part(paths, whole).ok_or_else(|| {
        format!(
            "this is a bug in yavs: a value of `{}` not encoded",
            console::dotted(&s.path)
        )
    })?;
    queue(sh, at, Op { kind, leaves });
    Ok(())
}

/// Hand `op` to the run loop. From a stop inside a process, pause the kernel after the cycle
/// (not a stop), so the change is made in the next one.
fn queue(sh: &Shared, at: &At<'_, '_>, op: Op) {
    sh.ops.lock().unwrap_or_else(|p| p.into_inner()).push(op);
    if at.process().is_some() {
        sh.paused_in_process.store(true, Ordering::Relaxed);
        if let Some(i) = sh.interrupt.get() {
            i.pause();
        }
    }
}

/// Make the queued forces, deposits and releases (`SimControl`, applied in the next cycle).
fn apply_ops(sh: &Shared, sim: &mut SimControl) {
    let ops = std::mem::take(&mut *sh.ops.lock().unwrap_or_else(|p| p.into_inner()));
    for op in ops {
        for (l, v) in &op.leaves {
            let r = match (&op.kind, v) {
                (OpKind::Deposit, Some(v)) => sim.deposit(*l, v),
                (OpKind::Force(m), Some(v)) => sim.force(*l, *m, v),
                (OpKind::Release(m), _) => sim.release(*l, *m),
                _ => continue,
            };
            // The values were encoded for these leaves: a refusal is a bug.
            if let Err(e) = r {
                sh.output(
                    "stderr",
                    &format!("this is a bug in yavs: the kernel refused a change: {e}\n"),
                );
            }
        }
    }
}

/// `setVariable`: a local (or a field or element of one) is written, a signal deposited (in
/// the next cycle; a field or element deposits the whole signal with that part changed).
fn set_variable(sh: &Shared, at: &At<'_, '_>, req: &Json) {
    let a = &req["arguments"];
    let r = a["variablesReference"].as_i64().unwrap_or(0);
    let name = a["name"].as_str().unwrap_or_default();
    let text = a["value"].as_str().unwrap_or_default();
    let (frame, kind) = (r >> 2, r & 3);
    let (tid, i) = (frame >> 16, (frame & 0xffff) as usize);
    let target = if r >= CHILD {
        let node = sh
            .nodes
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .get((r - CHILD) as usize)
            .cloned();
        match node {
            Some(Node::Region(g)) => sh.design.regions[g]
                .sigs
                .iter()
                .copied()
                .find(|&k| sh.design.sigs[k].name == name)
                .map(|sig| Node::Signal {
                    sig,
                    path: Vec::new(),
                })
                .ok_or_else(|| format!("no signal `{name}` here")),
            Some(Node::SharedScope(_)) => Err(
                "a shared variable is shown, not written: only a local or a signal can be set"
                    .into(),
            ),
            Some(n) => child_pos(sh, at, &n, name).map(|k| n.below(k)),
            None => Err(format!("no variables {r}")),
        }
    } else {
        match kind {
            1 => Ok(Node::Local {
                tid,
                frame: i,
                name: name.to_string(),
                path: Vec::new(),
            }),
            2 => thread_proc(sh, tid)
                .and_then(|p| {
                    p.signals
                        .iter()
                        .copied()
                        .find(|&k| sh.design.sigs[k].name == name)
                })
                .map(|sig| Node::Signal {
                    sig,
                    path: Vec::new(),
                })
                .ok_or_else(|| format!("no signal `{name}` here")),
            3 => Err("generics are constants".into()),
            _ => Err(format!("no variables {r}")),
        }
    };
    match target.and_then(|n| write_node(sh, at, &n, text)) {
        Ok(v) => sh.respond(req, json!({"value": v, "variablesReference": 0})),
        Err(m) => sh.fail(req, &m),
    }
}

/// The position of child `name` (a field, or an element by its index) of a node's value.
fn child_pos(sh: &Shared, at: &At<'_, '_>, n: &Node, name: &str) -> Result<usize, String> {
    let (t, _) = node_root(sh, at, n)
        .and_then(|(t, v)| descend(t, v, node_path(n)))
        .ok_or("the variable is not readable here")?;
    match &t {
        LitType::Record(fs) => fs.iter().position(|(f, _)| f == name),
        LitType::Array { len, .. } => name
            .parse::<i64>()
            .ok()
            .and_then(|i| (0..*len).find(|&k| t.index(k) == Some(i)))
            .map(|k| k as usize),
        _ => None,
    }
    .ok_or_else(|| format!("no element `{name}` here"))
}

/// Write `text` at a node's path: a local at once, a signal as a deposit of its whole
/// value; the new part as text.
fn write_node(sh: &Shared, at: &At<'_, '_>, n: &Node, text: &str) -> Result<String, String> {
    let (t, mut root) = node_root(sh, at, n).ok_or_else(|| match n {
        Node::Local { name, .. } => format!("no local `{name}` here"),
        _ => "the signal is not readable here".to_string(),
    })?;
    let path = node_path(n);
    let mut pt = &t;
    let mut pv = &mut root;
    for &k in path {
        (pt, pv) = match (pt, pv) {
            (LitType::Record(fs), Lit::Record(xs)) => (&fs[k].1, &mut xs[k]),
            (LitType::Array { elem, .. }, Lit::Array(xs)) => (&**elem, &mut xs[k]),
            _ => return Err("this is a bug in yavs: a path into a scalar".into()),
        };
    }
    let what = node_text(sh, at, n).unwrap_or_else(|| "this variable".to_string());
    *pv = design::typed_literal(text, pt)
        .map_err(|hint| format!("`{text}` is not a value of `{what}`: {hint}"))?;
    let shown = literal_image(pv, pt);
    match n {
        Node::Local {
            tid, frame, name, ..
        } => {
            let s = stop_of(at, *tid).ok_or_else(|| format!("no local `{name}` here"))?;
            write_local(sh, s, *frame, name, &root)?;
        }
        Node::Signal { sig, path } => {
            put(
                sh,
                at,
                *sig,
                std::slice::from_ref(path),
                Some(&root),
                OpKind::Deposit,
            )?;
        }
        Node::Region(_) | Node::SharedScope(_) => return Err("a region has no value".into()),
        Node::Value(..) | Node::Shared { .. } => {
            return Err(
                "this value is shown, not written: only a local or a signal can be set".into(),
            );
        }
    }
    Ok(shown)
}

/// Requests the reader thread answers itself; the rest go to the simulation thread.
fn handle(sh: &Shared, tx: &Sender<Msg>, req: Json) {
    let cmd = req["command"].as_str().unwrap_or_default().to_string();
    match cmd.as_str() {
        "initialize" => {
            let mut caps = json!({
                    "supportsConfigurationDoneRequest": true,
                    "supportsConditionalBreakpoints": true,
                    "supportsHitConditionalBreakpoints": true,
                    "supportsEvaluateForHovers": true,
                    "supportsTerminateRequest": true,
                    "supportsDataBreakpoints": true,
                    "supportsSetVariable": true,
                    "supportsCompletionsRequest": true,
                    "exceptionBreakpointFilters": [
                        {"filter": "failure", "label": "Assertion: failure", "default": false},
                        {"filter": "error", "label": "Assertion: error", "default": false},
                        {"filter": "warning", "label": "Assertion: warning", "default": false},
                        {"filter": "note", "label": "Report: note", "default": false},
                    ],
            });
            if sh.link.attached {
                caps["supportsRestartRequest"] = json!(true);
            }
            // For the extension's "Add to Wave".
            if sh.link.viewer.get().is_some() {
                caps["yavsViewer"] = json!(true);
            }
            let ansi = req["arguments"]["supportsANSIStyling"] == true;
            sh.link.ansi.store(ansi, Ordering::Relaxed);
            sh.respond(&req, caps);
            sh.event("initialized", json!({}));
        }
        "launch" | "attach" => {
            let entry = req["arguments"]["stopOnEntry"].as_bool().unwrap_or(false);
            sh.link.stop_on_entry.store(entry, Ordering::Relaxed);
            if req["arguments"]["yavsConsole"] == true {
                sh.link.console.store(true, Ordering::Relaxed);
            }
            let columns = req["arguments"]["yavsColumns"].as_u64().unwrap_or(0);
            sh.link.columns.store(columns as usize, Ordering::Relaxed);
            sh.respond(&req, json!({}));
        }
        "setBreakpoints" => set_breakpoints(sh, &req),
        "dataBreakpointInfo" => data_breakpoint_info(sh, &req),
        "setDataBreakpoints" => set_signal_breakpoints(sh, &req, 0),
        "yavs/setSignalBreakpoints" => set_signal_breakpoints(sh, &req, 1),
        "setExceptionBreakpoints" => {
            let filters = req["arguments"]["filters"].as_array().cloned();
            console::set_filters(sh, &filters.unwrap_or_default());
            sh.respond(&req, json!({}));
        }
        "yavs/hierarchy" => sh.respond(&req, hierarchy(sh)),
        "yavs/waveSubscribe" => wave_subscribe(sh, &req),
        "yavs/waveUnsubscribe" => {
            sh.lock_wave().take();
            sh.link.stream.push();
            sh.respond(&req, json!({}));
        }
        "configurationDone" => {
            sh.respond(&req, json!({}));
            let _ = tx.send(Msg::Start);
        }
        "threads" => {
            let threads: Vec<Json> = sh
                .session()
                .processes()
                .iter()
                .enumerate()
                .map(|(i, p)| json!({"id": i + 1, "name": thread_label(sh, &p.name)}))
                .collect();
            sh.respond(&req, json!({"threads": threads}));
        }
        "pause" => {
            // The response first: the stop it causes may otherwise reach the client before it
            // (#441), and a client waiting for the stop after the response would wait forever.
            sh.respond(&req, json!({}));
            pause(sh);
        }
        // The console's `stop`: the one command a running simulation takes.
        "evaluate"
            if !sh.stopped.load(Ordering::Relaxed)
                && req["arguments"]["context"] == "repl"
                && req["arguments"]["expression"]
                    .as_str()
                    .is_some_and(|e| e.trim() == "stop") =>
        {
            sh.respond(&req, json!({"result": "", "variablesReference": 0}));
            pause(sh);
        }
        "disconnect" | "terminate" => {
            sh.terminating.store(true, Ordering::Relaxed);
            if let Some(i) = sh.interrupt.get() {
                i.stop();
            }
            let _ = tx.send(Msg::Request(req));
        }
        // While it runs: as a stop's `restart` leaves the stop (`leave`).
        "restart" if !sh.stopped.load(Ordering::Relaxed) => {
            if restart_request(sh, &req) {
                sh.respond(&req, json!({}));
                sh.terminating.store(true, Ordering::Relaxed);
                sh.discard.store(true, Ordering::Relaxed);
                if let Some(i) = sh.interrupt.get() {
                    i.stop();
                }
            }
        }
        _ if sh.stopped.load(Ordering::Relaxed) => {
            let _ = tx.send(Msg::Request(req));
        }
        _ => sh.fail(&req, "the simulation is running"),
    }
}

fn pause(sh: &Shared) {
    sh.pause_req.store(true, Ordering::Relaxed);
    sh.session().request_pause();
    if let Some(i) = sh.interrupt.get() {
        i.pause();
    }
}

/// `yavs/hierarchy`: the elaborated regions, nested, with their processes and signals.
fn hierarchy(sh: &Shared) -> Json {
    fn region(sh: &Shared, r: usize) -> Json {
        let reg = &sh.design.regions[r];
        json!({
            "name": reg.name,
            "path": console::dotted(&reg.path),
            "kind": reg.kind,
            "regions": reg.children.iter().map(|&c| region(sh, c)).collect::<Vec<_>>(),
            "processes": reg.procs.iter().map(|p| console::dotted(p)).collect::<Vec<_>>(),
            "signals": reg.sigs.iter().map(|&k| {
                let s = &sh.design.sigs[k];
                json!({"name": s.name, "path": console::dotted(&s.path), "type": s.ty})
            }).collect::<Vec<_>>(),
        })
    }
    json!({"regions": sh.design.roots.iter().map(|&r| region(sh, r)).collect::<Vec<_>>()})
}

/// `setBreakpoints`: replace a file's breakpoints.
fn set_breakpoints(sh: &Shared, req: &Json) {
    let a = &req["arguments"];
    let path = design::absolute(a["source"]["path"].as_str().unwrap_or_default());
    // One file is one entry however the client spells it, so that a later list replaces this one.
    let key = design::file_key(&path);
    let list = a["breakpoints"].as_array().cloned().unwrap_or_default();
    sh.link
        .kept
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .set_lines(&key, list);
    let mut out = apply_lines(sh, &key);
    // The answer names the file as the client did.
    for b in out.iter_mut().filter(|b| b["verified"] == true) {
        b["source"] = source(&path);
    }
    sh.respond(req, json!({"breakpoints": for_client(out)}));
}

/// What a client is told of its breakpoints: `unchecked` (the console's reply) as the message
/// of a verified breakpoint.
fn for_client(mut list: Vec<Json>) -> Vec<Json> {
    for b in &mut list {
        let note = console::unchecked_note(b);
        if let Some(o) = b.as_object_mut() {
            o.remove("unchecked");
            if !note.is_empty() {
                o.insert("message".into(), json!(note.trim()));
            }
        }
    }
    list
}

/// Map each kept line breakpoint of file `key` to the statements starting on its line (or the
/// next line that has some) and replace the file's breakpoints; what a client is told of each.
fn apply_lines(sh: &Shared, key: &str) -> Vec<Json> {
    let list = sh
        .link
        .kept
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .lines(key);
    let session = sh.session();
    let mut set = sh.bps.lock().unwrap_or_else(|p| p.into_inner());
    let mut conds = sh.conds.lock().unwrap_or_else(|p| p.into_inner());
    for s in set.remove(key).into_iter().flatten() {
        session.clear_breakpoint(s);
        conds.remove(&s);
    }
    let mut ids = Vec::new();
    let mut out = Vec::new();
    for bp in &list {
        let line = bp["line"].as_u64().unwrap_or(0) as u32;
        let hit = match bp["hitCondition"].as_str().map(parse_hit) {
            None => Ok(HitCond::Always),
            Some(h) => h,
        };
        let found = sh.design.line_stmts(key, line);
        // A condition is evaluated here; the interpreter then stops on every hit.
        let mut unchecked = Vec::new();
        let hit = hit.and_then(|hit| match bp["condition"].as_str() {
            None => Ok((hit, None)),
            Some(c) => {
                let seen = match found {
                    Some((_, stmts)) => line_seen(sh, stmts),
                    None => thread_seen(sh, 0),
                };
                let (cond, left) = make_cond(sh, c, seen)?;
                unchecked = left;
                let c = Arc::new(LineCond {
                    cond,
                    hit,
                    hits: std::sync::atomic::AtomicU64::new(0),
                });
                Ok((HitCond::Always, Some(c)))
            }
        });
        match (found, hit) {
            (Some((l, stmts)), Ok((hit, c))) => {
                for &s in stmts {
                    session.set_breakpoint(s, LineBp::new(None, hit));
                    if let Some(c) = &c {
                        conds.insert(s, c.clone());
                    }
                    ids.push(s);
                }
                let mut b = json!({"verified": true, "line": l, "source": source(key)});
                if !unchecked.is_empty() {
                    b["unchecked"] = json!(unchecked);
                }
                out.push(b);
            }
            (None, _) => out.push(json!({
                "verified": false,
                "line": line,
                "message": "no statement on or after this line",
            })),
            (_, Err(m)) => out.push(json!({"verified": false, "line": line, "message": m})),
        }
    }
    set.insert(key.to_string(), ids);
    out
}

/// `yavs/waveSubscribe`: replace the live wave subscription by a recorder of the signals (and,
/// with `variables`, the variables) `include` and `exclude` select, as `--wave` records them;
/// answers with the hierarchy. A subscription made while the simulation runs records from its
/// next callback on.
fn wave_subscribe(sh: &Shared, req: &Json) {
    let a = &req["arguments"];
    let globs = |k: &str| -> Vec<String> {
        a[k].as_array()
            .into_iter()
            .flatten()
            .filter_map(|g| g.as_str().map(console::dotted))
            .collect()
    };
    let mut sel = yavs_wave::Selection {
        include: globs("include"),
        exclude: globs("exclude"),
        ..Default::default()
    };
    if let Some(n) = a["maxArrayElems"].as_u64() {
        sel.max_array_elems = n as usize;
    }
    let every = std::time::Duration::from_millis(a["flushMs"].as_u64().unwrap_or(100));
    let built = sh.built.get().filter(|_| a["variables"] == true);
    // The previous subscription's batches go first.
    let mut cur = sh.lock_wave();
    cur.take();
    sh.link.stream.push();
    let r = yavs_wave::record_to(&sh.db, &sh.elab, sel, built, |_| {
        Ok(waves::sink(&sh.link.stream, every))
    });
    match r {
        Ok((rec, h)) => {
            *cur = Some(waves::Sub::new(rec));
            drop(cur);
            sh.respond(req, waves::hierarchy(&h));
        }
        Err(err) => {
            drop(cur);
            sh.fail(req, &err.to_string());
        }
    }
}

/// The process of thread `tid`, as the design shows it.
fn thread_proc(sh: &Shared, tid: i64) -> Option<&design::Proc> {
    proc_name(sh, tid).and_then(|n| sh.design.proc(&n))
}

/// Whether variable `name` of a variables reference (or, without one, the expression `name`
/// seen from `frameId`) is a signal a data breakpoint can watch.
fn data_breakpoint_info(sh: &Shared, req: &Json) {
    let a = &req["arguments"];
    let name = a["name"].as_str().unwrap_or_default();
    let found = match a["variablesReference"].as_i64() {
        Some(r) if r & 3 != 2 => Err("only signals can be watched"),
        Some(r) => Ok(sh.design.find(thread_proc(sh, r >> 18), name)),
        None => {
            let tid = a["frameId"].as_i64().map_or(0, |f| f >> 16);
            Ok(sh.design.find(thread_proc(sh, tid), name))
        }
    };
    let body = match found {
        Ok(Some(k)) => {
            let path = &sh.design.sigs[k].path;
            let shown = console::dotted(path);
            json!({"dataId": path, "description": shown, "accessTypes": ["write"], "canPersist": true})
        }
        Ok(None) => json!({"dataId": null, "description": format!("no signal `{name}` here")}),
        Err(m) => json!({"dataId": null, "description": m}),
    };
    sh.respond(req, body);
}

/// Replace the signal breakpoints of `setDataBreakpoints` (`set` 0: `dataId` is a path) or
/// of `yavs/setSignalBreakpoints` (`set` 1: `signal`, a name seen from `frameId` or a path,
/// and an optional `value`), then the watch list.
fn set_signal_breakpoints(sh: &Shared, req: &Json, set: usize) {
    let a = &req["arguments"];
    let mut list = a["breakpoints"].as_array().cloned().unwrap_or_default();
    for bp in &mut list {
        bp["frameId"] = a["frameId"].clone();
    }
    sh.link
        .kept
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .set_signals(set, list);
    let out = apply_signals(sh, set);
    sh.respond(req, json!({"breakpoints": for_client(out)}));
}

/// Resolve the kept signal breakpoints of `set`; what a client is told of each.
fn apply_signals(sh: &Shared, set: usize) -> Vec<Json> {
    let list = sh
        .link
        .kept
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .signals(set);
    let mut bps = Vec::new();
    let mut out = Vec::new();
    for bp in &list {
        let mut unchecked = Vec::new();
        let tid = bp["frameId"].as_i64().map_or(0, |f| f >> 16);
        let p = thread_proc(sh, tid);
        let name = bp[if set == 0 { "dataId" } else { "signal" }]
            .as_str()
            .unwrap_or_default();
        let r = (|| {
            let k = sh
                .design
                .find(p, name)
                .ok_or_else(|| console::no_signal(sh, tid, name))?;
            let value = match bp["value"].as_str() {
                None => None,
                Some(v) => Some(sh.design.sigs[k].parse(v)?),
            };
            let cond = match bp["condition"].as_str() {
                None => None,
                Some(c) => {
                    let (c, left) = make_cond(sh, c, kernel_seen(sh, tid))?;
                    unchecked = left;
                    Some(c)
                }
            };
            let hit = match bp["hitCondition"].as_str() {
                None => HitCond::Always,
                Some(h) => parse_hit(h)?,
            };
            Ok::<_, String>(SigBp {
                sig: k,
                value,
                cond,
                hit,
                hits: 0,
                tid,
            })
        })();
        out.push(match r {
            Ok(b) => {
                let d = console::dotted(&sh.design.sigs[b.sig].path);
                bps.push(b);
                let mut j = json!({"verified": true, "description": d});
                if !unchecked.is_empty() {
                    j["unchecked"] = json!(unchecked);
                }
                j
            }
            Err(m) => json!({"verified": false, "message": m}),
        });
    }
    sh.sig_bps.lock().unwrap_or_else(|p| p.into_inner())[set] = bps;
    rewatch(sh);
    out
}

/// The kernel's watch list: every leaf of every signal breakpoint.
fn rewatch(sh: &Shared) {
    let all = sh.sig_bps.lock().unwrap_or_else(|p| p.into_inner());
    let w: Vec<(SignalId, Option<Value>)> = all
        .iter()
        .flatten()
        .flat_map(|b| sh.design.sigs[b.sig].leaves())
        .map(|l| (l, None))
        .collect();
    *sh.watches
        .get()
        .expect("set before the reader starts")
        .lock()
        .unwrap_or_else(|p| p.into_inner()) = w;
}

/// After a pause: whether a watched signal changed in the last cycle, and the first signal
/// breakpoint that fires now (its change, value, condition and hit count held): the stop's
/// reason and text.
fn signal_hit(sh: &Shared, sim: &SimControl) -> (bool, Option<(&'static str, String)>) {
    let hits = std::mem::take(
        &mut *sh
            .hits
            .get()
            .expect("set before the run")
            .lock()
            .unwrap_or_else(|p| p.into_inner()),
    );
    if hits.is_empty() {
        return (false, None);
    }
    let at = At::Kernel(sim);
    let mut all = sh.sig_bps.lock().unwrap_or_else(|p| p.into_inner());
    let mut stop = None;
    for b in all.iter_mut().flatten() {
        let s = &sh.design.sigs[b.sig];
        if !hits.iter().any(|h| s.leaves().any(|l| l == h.2)) {
            continue;
        }
        let read = |r| sim.value(r).ok();
        if b.value
            .as_ref()
            .is_some_and(|v| s.value(&read).as_ref() != Some(v))
        {
            continue;
        }
        let held = match &b.cond {
            None => Ok(true),
            // Names as seen from the process of the request, else from the signal's region.
            Some(c) => c.eval(&Names {
                sh,
                at: &at,
                tid: b.tid,
                frame: None,
                region: s.path.rfind(':').map(|i| &s.path[..=i]),
            }),
        };
        let fire = match held {
            Ok(true) => {
                b.hits += 1;
                fires(b.hit, b.hits).then(|| {
                    let name = console::dotted(&s.path);
                    ("data breakpoint", format!("{name} = {}", s.show(&read)))
                })
            }
            Ok(false) => None,
            Err(m) => Some((
                "exception",
                format!("{}: breakpoint condition: {m}", console::dotted(&s.path)),
            )),
        };
        if stop.is_none() {
            stop = fire;
        }
    }
    (true, stop)
}

/// `N`, `= N`, `>= N`, `% N`: the hit count condition.
fn parse_hit(s: &str) -> Result<HitCond, String> {
    let t = s.trim();
    let (mk, rest): (fn(u64) -> HitCond, &str) = if let Some(r) = t.strip_prefix(">=") {
        (HitCond::Ge, r)
    } else if let Some(r) = t.strip_prefix('%') {
        (HitCond::Multiple, r)
    } else if let Some(r) = t.strip_prefix('=') {
        (HitCond::Eq, r)
    } else {
        (HitCond::Eq, t)
    };
    rest.trim()
        .parse()
        .map(mk)
        .map_err(|_| format!("hit condition `{s}`: expected N, = N, >= N or % N"))
}

/// What the caller adds to a session besides the design.
#[derive(Default)]
pub struct Options {
    /// Why `restart` cannot work in this run (state outside the simulation).
    pub no_restart: Option<String>,
    /// Called when a stop starts (`true`) and when it ends.
    pub interactive: Option<fn(bool)>,
    pub reload: Option<ReloadFn>,
    /// The test runner's hook, `<port>:<secret>` (`console::HOOK_VAR`).
    pub hook: Option<String>,
    /// Source paths as compiled, and as the debugger shows and takes them (`--source-map`).
    pub source_map: Vec<(String, String)>,
    /// Where this testbench's wave layout is kept (`wave save`, `wave load`).
    pub layout: Option<std::path::PathBuf>,
    /// `--hang-report[=MAX]`: at the end of the run, where each process waits.
    pub hang_report: Option<usize>,
    /// `--assert-summary`.
    pub assert_summary: bool,
    /// `--activity[=FILE]`.
    pub activity: Option<Option<String>>,
}

/// The callback that subscribes the caller's observers (`--wave`, `--vhpi`) to each built
/// simulation and returns what to do at its end.
pub trait Attach:
    FnMut(
    &mut SimControl,
    Option<&Built>,
    &Arc<Db>,
    &Arc<Elaborated>,
) -> Result<Attached, Box<Diagnostic>>
{
}
impl<F> Attach for F where
    F: FnMut(
        &mut SimControl,
        Option<&Built>,
        &Arc<Db>,
        &Arc<Elaborated>,
    ) -> Result<Attached, Box<Diagnostic>>
{
}

/// Run the elaborated design under a DAP client until it ends; returns the exit status of
/// the run (docs/cli-contract.md section 5). `attach` subscribes the caller's observers
/// (`--wave`, `--vhpi`) to the built simulation and returns what to do at its end. `report`
/// receives the final statistics before the exit events.
pub fn serve(
    loaded: Loaded,
    transport: Transport,
    options: Options,
    attach: impl Attach,
    report: Report,
) -> Result<u8, Box<Diagnostic>> {
    let io_err = |what: &str, err: std::io::Error| {
        Box::new(Diagnostic::error(IO_ERROR, format!("DAP: {what}")).with_note(err.to_string()))
    };
    // The socket, to close it gracefully at the end.
    let mut closer = None;
    let (reader, writer): (Box<dyn Read + Send>, Box<dyn Write + Send>) = match transport {
        Transport::Stdio => {
            let out = stdio_out().map_err(|err| io_err("cannot duplicate standard output", err))?;
            (Box::new(std::io::stdin()), out)
        }
        Transport::Tcp(port) => {
            let l = TcpListener::bind(("127.0.0.1", port))
                .map_err(|err| io_err(&format!("cannot listen on port {port}"), err))?;
            let addr = l
                .local_addr()
                .map_err(|err| io_err("no local address", err))?;
            eprintln!("yavs: DAP listening on 127.0.0.1:{}", addr.port());
            eprintln!("yavs: waiting for a debugger on port {}", addr.port());
            // A client that closes before it says anything (an editor probing the port) is no
            // session: wait for the next.
            let s = loop {
                let (s, _) = l.accept().map_err(|err| io_err("accept failed", err))?;
                if s.peek(&mut [0u8; 1]).is_ok_and(|n| n > 0) {
                    break s;
                }
            };
            let r = s.try_clone().map_err(|err| io_err("accept failed", err))?;
            closer = s.try_clone().ok();
            (Box::new(r), Box::new(s))
        }
    };
    session(
        loaded, reader, writer, closer, options, attach, report, true,
    )
}

/// How long a client gets to end a session that could not start; then the process exits with `code`.
const REFUSED_WAIT: std::time::Duration = std::time::Duration::from_secs(30);

/// A session that cannot start, with a client that knows nothing of it yet: `initialize` is
/// answered, `launch` or `attach` fail with `text`, which is what an editor shows, and everything
/// else until the client ends it (#934). The diagnostic on standard error is the caller's.
fn refuse_session(reader: Box<dyn Read + Send>, mut send: impl FnMut(Json), text: &str, code: u8) {
    std::thread::spawn(move || {
        std::thread::sleep(REFUSED_WAIT);
        std::process::exit(i32::from(code));
    });
    let answer = |req: &Json, m: Json| {
        let mut r =
            json!({"type": "response", "request_seq": req["seq"], "command": req["command"]});
        if let (Some(r), Some(m)) = (r.as_object_mut(), m.as_object()) {
            r.extend(m.clone());
        }
        r
    };
    let event = |event: &str, body: Json| json!({"type": "event", "event": event, "body": body});
    let mut r = BufReader::new(reader);
    while let Ok(Some(req)) = read_message(&mut r) {
        if req["type"] != "request" {
            continue;
        }
        match req["command"].as_str().unwrap_or_default() {
            "initialize" => send(answer(&req, json!({"success": true, "body": {}}))),
            "disconnect" | "terminate" => {
                send(answer(&req, json!({"success": true, "body": {}})));
                return;
            }
            c => {
                let launch = matches!(c, "launch" | "attach");
                if launch {
                    send(event(
                        "output",
                        json!({"category": "stderr", "output": text}),
                    ));
                }
                send(answer(&req, json!({"success": false, "message": text})));
            }
        }
    }
}

/// The session on standard input and output.
fn stdio_out() -> std::io::Result<Box<dyn Write + Send>> {
    Ok(match STDOUT.get().map(std::fs::File::try_clone) {
        Some(f) => Box::new(std::io::BufWriter::new(f?)),
        None => Box::new(std::io::stdout()),
    })
}

/// `--dap=stdio` failed before there is a session (no such top level, an elaboration error): tell
/// the client that started this process, which sees nothing of standard error. `text` is the
/// diagnostic as a user reads it. Nothing happens on a terminal: nobody speaks the protocol there.
pub fn refuse(text: &str, code: u8) {
    if std::io::stdin().is_terminal() {
        return;
    }
    let Ok(mut out) = stdio_out() else {
        return;
    };
    let mut seq = 0;
    let send = |mut m: Json| {
        seq += 1;
        m["seq"] = json!(seq);
        // A closed connection ends the loop through the reader's end of input.
        let _ = write_message(&mut *out, &m);
    };
    refuse_session(Box::new(std::io::stdin()), send, text, code);
}

/// How one simulation of a session ended.
enum Outcome {
    Exit(u8),
    /// Run again, at time 0.
    Again(Loaded),
}

/// [`serve`] over a connection the caller made: the console speaks DAP to the server over a pipe.
/// A failure before the session started is only returned: the caller is the client.
pub fn serve_on(
    loaded: Loaded,
    reader: Box<dyn Read + Send>,
    writer: Box<dyn Write + Send>,
    closer: Option<std::net::TcpStream>,
    options: Options,
    attach: impl Attach,
    report: Report,
) -> Result<u8, Box<Diagnostic>> {
    session(
        loaded, reader, writer, closer, options, attach, report, false,
    )
}

#[allow(clippy::too_many_arguments)] // `serve` and `serve_on` are its two callers
fn session(
    loaded: Loaded,
    reader: Box<dyn Read + Send>,
    writer: Box<dyn Write + Send>,
    closer: Option<std::net::TcpStream>,
    options: Options,
    mut attach: impl Attach,
    report: Report,
    tell_client: bool,
) -> Result<u8, Box<Diagnostic>> {
    let (tx, rx) = channel();
    let link = Arc::new(Link {
        out: Mutex::new(writer),
        seq: Mutex::new(0),
        rx: Mutex::new(rx),
        stream: Arc::default(),
        disconnect: Mutex::new(None),
        console: AtomicBool::new(false),
        columns: std::sync::atomic::AtomicUsize::new(0),
        ansi: AtomicBool::new(false),
        stop_on_entry: AtomicBool::new(false),
        client_restart: AtomicBool::new(false),
        eof: AtomicBool::new(false),
        kept: Mutex::default(),
        con: Mutex::default(),
        current: Mutex::new(None),
        viewer: OnceLock::new(),
        hook: OnceLock::new(),
        no_restart: options.no_restart,
        interactive: options.interactive,
        attached: closer.is_some(),
        source_map: options.source_map,
        layout: options.layout,
        hang_report: options.hang_report,
        assert_summary: options.assert_summary,
        activity: options.activity,
        reload: Mutex::new(options.reload),
    });
    if let Some(value) = &options.hook {
        let (hook, lines) = match console::Hook::connect(value) {
            Ok(h) => h,
            Err(why) => {
                let d = Diagnostic::error(
                    console::HOOK_ERROR,
                    format!("the console hook in `{}` cannot be used", console::HOOK_VAR),
                )
                .with_note(why)
                .with_help(format!(
                    "a test runner sets `{}` for a session while it serves the hook; unset it to \
                     run without one",
                    console::HOOK_VAR
                ));
                if tell_client {
                    let text =
                        yavs_diag::render(&loaded.db.sources, &d, MessageFormat::Human, false);
                    refuse_session(reader, |m| link.send(m), &text, 2);
                }
                return Err(Box::new(d));
            }
        };
        let _ = link.hook.set(hook);
        let (link, tx) = (link.clone(), tx.clone());
        std::thread::spawn(move || console::hook_reader(lines, &link, &tx));
    }
    {
        let link = link.clone();
        std::thread::spawn(move || {
            waves::sender(link.stream.clone(), || {
                let mut out = link.out.lock().unwrap_or_else(|p| p.into_inner());
                link.drain(&mut out);
            })
        });
    }
    let mut start = Some((reader, tx));
    let mut report = Some(report);
    let mut loaded = loaded;
    let code = loop {
        let first = start.is_some();
        let db = loaded.db.clone();
        // The reader thread starts once the first simulation can take requests.
        let begin = |sh: &Arc<Shared>, start: &mut Option<(Box<dyn Read + Send>, Sender<Msg>)>| {
            *sh.link.current.lock().unwrap_or_else(|p| p.into_inner()) = Some(sh.clone());
            let Some((reader, tx)) = start.take() else {
                return;
            };
            let link = sh.link.clone();
            std::thread::spawn(move || {
                let live = || {
                    link.current
                        .lock()
                        .unwrap_or_else(|p| p.into_inner())
                        .clone()
                        .expect("set before the reader starts")
                };
                let mut r = BufReader::new(reader);
                while let Ok(Some(m)) = read_message(&mut r) {
                    if m["type"] == "request" {
                        handle(&live(), &tx, m);
                    }
                }
                link.eof.store(true, Ordering::Relaxed);
                let sh = live();
                sh.terminating.store(true, Ordering::Relaxed);
                if let Some(i) = sh.interrupt.get() {
                    i.stop();
                }
                let _ = tx.send(Msg::Eof);
            });
        };
        match one_run(
            &link,
            loaded,
            first,
            |sh| begin(sh, &mut start),
            &mut attach,
            &mut report,
        ) {
            Ok(Outcome::Exit(code)) => break Ok(code),
            Ok(Outcome::Again(next)) => loaded = next,
            Err(d) => {
                let text = yavs_diag::render(&db.sources, &d, MessageFormat::Human, false);
                match start.take() {
                    // Nothing read the connection yet.
                    Some((reader, _)) => {
                        if tell_client {
                            refuse_session(reader, |m| link.send(m), &text, 2);
                            if let Some(s) = &closer {
                                let _ = s.shutdown(std::net::Shutdown::Write);
                            }
                        }
                        return Err(d);
                    }
                    // A `restart` that failed: the session ends here, in the open.
                    None => {
                        link.output("stderr", &text);
                        link.event("exited", json!({"exitCode": 2}));
                        link.event("terminated", json!({}));
                        break Err(d);
                    }
                }
            }
        }
    };
    // Answer the disconnect, or wait for it.
    let mut pending = link
        .disconnect
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .take();
    let rx = link.rx.lock().unwrap_or_else(|p| p.into_inner());
    let mut eof = false;
    while pending.is_none() {
        match rx.recv() {
            Ok(Msg::Request(r))
                if matches!(r["command"].as_str(), Some("disconnect" | "terminate")) =>
            {
                pending = Some(r)
            }
            Ok(Msg::Request(r)) => link.fail(&r, "the simulation has ended"),
            Ok(Msg::Eof) | Err(_) => {
                eof = true;
                break;
            }
            Ok(_) => {}
        }
    }
    if let Some(d) = pending {
        link.respond(&d, json!({}));
    }
    // Exiting with the socket open resets the connection on some systems, and the client can
    // lose the last messages: send FIN, then give the client a moment to close its side.
    if let Some(s) = closer {
        let _ = s.shutdown(std::net::Shutdown::Write);
        while !eof {
            match rx.recv_timeout(std::time::Duration::from_secs(2)) {
                Ok(Msg::Eof) | Err(_) => eof = true,
                Ok(_) => {}
            }
        }
    }
    code
}

/// Build one simulation of a session and serve it until it ends or restarts.
fn one_run(
    link: &Arc<Link>,
    loaded: Loaded,
    first: bool,
    begin: impl FnOnce(&Arc<Shared>),
    attach: &mut impl Attach,
    report: &mut Option<Report>,
) -> Result<Outcome, Box<Diagnostic>> {
    let Loaded {
        db: db_arc,
        e: e_arc,
        cfg,
        stop,
    } = loaded;
    let again = |db_arc: &Arc<Db>, e_arc: &Arc<Elaborated>, cfg: &Config| Loaded {
        db: db_arc.clone(),
        e: e_arc.clone(),
        cfg: cfg.clone(),
        stop,
    };
    let (db, e) = (&*db_arc, &*e_arc);
    // A design with nothing to run has no lowered program; its session has an empty one (#995).
    let program =
        e.ir.as_ref()
            .map_or_else(Default::default, |ir| ir.program.clone());
    let sh = Arc::new(Shared {
        link: link.clone(),
        design: Design::new(db, e, &program, &link.source_map),
        session: OnceLock::new(),
        interrupt: OnceLock::new(),
        stopped: AtomicBool::new(false),
        terminating: AtomicBool::new(link.eof.load(Ordering::Relaxed)),
        pause_req: AtomicBool::new(false),
        paused_in_process: AtomicBool::new(false),
        discard: AtomicBool::new(false),
        ended: AtomicBool::new(false),
        restart: Mutex::new(None),
        queued: Mutex::new(None),
        bps: Mutex::new(HashMap::new()),
        sig_bps: Mutex::default(),
        watches: OnceLock::new(),
        hits: OnceLock::new(),
        ops: Mutex::new(Vec::new()),
        prog: program.clone(),
        nodes: Mutex::new(Vec::new()),
        conds: Mutex::new(HashMap::new()),
        wave: Arc::default(),
        db: db_arc.clone(),
        elab: e_arc.clone(),
        built: OnceLock::new(),
        sync: OnceLock::new(),
        wave_file: OnceLock::new(),
        ports: OnceLock::new(),
        signal_names: OnceLock::new(),
        activity: OnceLock::new(),
        deposits: Mutex::new(HashMap::new()),
        calls: Default::default(),
    });
    let session = DebugSession::new(&program, Box::new(Front(sh.clone())));
    let _ = sh.session.set(session.clone());
    // Stops inside a process need one thread (ir.md 3.4).
    let cfg_run = Config {
        threads: 1,
        ..cfg.clone()
    };
    let (mut sim, built) = match e.build_debug(cfg_run, &session) {
        Ok(x) => x,
        Err(err) => return Err(Box::new(e.runtime_diagnostic(&err))),
    };
    let _ = sh.interrupt.set(sim.interrupt());
    let names = (e.procs.iter()).map(|p| e.proc_label(p)).collect();
    sim.subscribe(Box::new(Output {
        sh: sh.clone(),
        names,
    }));
    let brk = SignalBreak::new(sim.interrupt());
    let _ = sh.watches.set(brk.watches());
    let _ = sh.hits.set(brk.hits());
    sim.subscribe(Box::new(brk));
    sim.subscribe(Box::new(waves::Tap {
        sub: sh.wave.clone(),
        last: 0,
    }));
    let attached = attach(&mut sim, built.as_ref(), &db_arc, &e_arc)?;
    let mut finish = attached.finish;
    if let Some(sync) = attached.sync {
        let _ = sh.sync.set(sync);
    }
    if let Some(f) = attached.wave_file {
        let _ = sh.wave_file.set(f);
    }
    if let Some(a) = attached.activity {
        let _ = sh.activity.set(a);
    }
    let _ = sh.ports.set(sim.port_links());
    let names = (0..sim.signal_count()).map(|s| {
        let s = sim.signal_name(yavs_kernel::SignalId(s as u32));
        s.unwrap_or("?").to_string()
    });
    let _ = sh.signal_names.set(names.collect());
    if let Some(v) = attached.viewer {
        console::set_viewer(link, v);
    }
    let rt = built.as_ref().map(|b| b.rt.clone());
    if let Some(rt) = &rt {
        rt.keep_assert_sites();
    }
    if let Some(b) = built {
        let _ = sh.built.set(b);
    }
    console::keep_breakpoints(&sh);
    begin(&sh);
    // Wait for the configuration.
    if first {
        let rx = link.rx.lock().unwrap_or_else(|p| p.into_inner());
        loop {
            match rx.recv() {
                Ok(Msg::Start) => break,
                Ok(Msg::Request(r)) => {
                    if matches!(r["command"].as_str(), Some("disconnect" | "terminate")) {
                        stash_disconnect(&sh, r);
                        break;
                    }
                    sh.fail(&r, "the simulation has not started");
                }
                Ok(Msg::Eof) | Err(_) => break,
                Ok(_) => {}
            }
        }
    }
    // A console stops at time 0, before any process has run, and so does every restart.
    let by_client = link.client_restart.swap(false, Ordering::Relaxed);
    let mut at_entry = !by_client
        && !sh.terminating.load(Ordering::Relaxed)
        && (!first || link.console.load(Ordering::Relaxed));
    if !at_entry && link.stop_on_entry.load(Ordering::Relaxed) {
        session.request_global_step();
    }
    // The time the run goes to: `stop`, or earlier while a `yavs/runFor` is under way. A stop
    // on the way does not cancel it: continuing resumes it.
    let mut limit = stop;
    let mut next = None;
    let requested = Ok(RunStatus::Ended(EndReason::Requested));
    let r = 'run: loop {
        if std::mem::take(&mut at_entry) {
            let body = json!({
                "reason": "entry",
                "allThreadsStopped": true,
                "description": format!("entry at {}, delta {}", sim.now(), sim.delta()),
            });
            match serve_stop(&sh, &At::Kernel(&sim), body) {
                Some(Next::Resume(Resume::GlobalStep)) => session.request_global_step(),
                Some(Next::Run(run)) => next = Some(run),
                Some(Next::Resume(_)) => {}
                Some(Next::Restart | Next::Quit) | None => break 'run requested,
            }
        }
        apply_ops(&sh, &mut sim);
        let (r, step) = match next.take() {
            None => (sim.run_until(limit), false),
            Some(Run::Until(t)) => {
                limit = t.min(stop);
                (sim.run_until(limit), false)
            }
            Some(Run::ToEnd) => {
                limit = stop;
                (sim.run_until(limit), false)
            }
            Some(Run::Delta) => (sim.step_delta(), true),
            Some(Run::Timestep) => (sim.step_timestep(), true),
        };
        if r != Ok(RunStatus::Paused) || sh.terminating.load(Ordering::Relaxed) {
            break r;
        }
        let (watched, hit) = signal_hit(&sh, &sim);
        let in_process = sh.paused_in_process.swap(false, Ordering::Relaxed);
        // A delta step queued at a stop in a process ends with the cycle that just ended; the
        // other runs start now (or after a signal breakpoint stop, when it is continued).
        let step = match sh.queued.lock().unwrap_or_else(|p| p.into_inner()).take() {
            Some(Run::Delta) => true,
            Some(run) => {
                next = Some(run);
                step
            }
            None => step,
        };
        let reason = if let Some((r, _)) = &hit {
            r
        } else if next.is_some() {
            continue;
        } else if step {
            "step"
        } else if in_process {
            continue;
        } else if sh.pause_req.swap(false, Ordering::Relaxed) {
            session.cancel_pause();
            "pause"
        } else if watched {
            continue; // no signal breakpoint holds
        } else if limit < stop {
            limit = stop;
            "step"
        } else {
            break r; // the stop time
        };
        let mut body = json!({
            "reason": reason,
            "allThreadsStopped": true,
            "description": format!("{reason} at {}, delta {}", sim.now(), sim.delta()),
        });
        if let Some((_, t)) = hit {
            body["text"] = json!(t);
        }
        match serve_stop(&sh, &At::Kernel(&sim), body) {
            Some(Next::Resume(Resume::GlobalStep)) => session.request_global_step(),
            Some(Next::Run(run)) => next = Some(run),
            Some(Next::Resume(_)) => {}
            Some(Next::Restart | Next::Quit) => break requested,
            None => break sim.run_until(stop),
        }
    };
    if let Some(max) = link.hang_report {
        let why = match &r {
            Ok(RunStatus::Paused) => Some(yavs_elab::StuckWhy::StopTime(stop)),
            Ok(RunStatus::Ended(EndReason::Idle)) => Some(yavs_elab::StuckWhy::Idle),
            _ => None,
        };
        if let Some(why) = why {
            let report = sim.stuck_report();
            let stuck = report
                .procs
                .iter()
                .any(|p| p.state == yavs_kernel::ProcWait::Never);
            if why != yavs_elab::StuckWhy::Idle || stuck {
                let d = e.stuck_diagnostic(&report, why, &e.proc_table(), max);
                sh.output(
                    "stderr",
                    &yavs_diag::render(&db.sources, &d, MessageFormat::Human, false),
                );
            }
        }
    }
    // Leave this simulation: its files are finished, so that the next one can write them.
    let restart = |sh: &Shared| sh.restart.lock().unwrap_or_else(|p| p.into_inner()).take();
    if let Some(new) = restart(&sh) {
        sh.lock_wave().take();
        if let Some(f) = finish.take() {
            let _ = f(&sim);
        }
        if let Some(rt) = &rt {
            rt.close_files();
        }
        return Ok(Outcome::Again(
            new.unwrap_or_else(|| again(&db_arc, &e_arc, &cfg)),
        ));
    }
    let code = match r {
        Ok(RunStatus::Ended(EndReason::Stop(c) | EndReason::Finish(c))) => {
            let what = match r {
                Ok(RunStatus::Ended(EndReason::Stop(_))) => "STOP",
                _ => "FINISH",
            };
            sh.output(
                "console",
                &format!(
                    "** Note: {}fs+{}: {what} called with status {c}\n",
                    sim.now().as_fs(),
                    sim.delta()
                ),
            );
            if (0..=125).contains(&c) { c as u8 } else { 1 }
        }
        Ok(RunStatus::Ended(EndReason::Severity(_) | EndReason::Requested)) => 1,
        Ok(_) => 0,
        Err(err) => {
            let d = e.runtime_diagnostic_in(&err, &e.proc_table());
            sh.output(
                "stderr",
                &yavs_diag::render(&db.sources, &d, MessageFormat::Human, false),
            );
            if err.exit_status() == 101 { 3 } else { 1 }
        }
    };
    if link.assert_summary {
        sh.output("console", &format!("{}\n", tools::asserts(&sh)));
    }
    if let Some(file) = &link.activity {
        sh.output("console", &format!("{}\n", tools::activity(&sh)));
        if let Some(Err(m)) = file.as_ref().map(|f| tools::activity_file(&sh, f)) {
            sh.output(
                "stderr",
                &format!("cannot write the activity report: {m}\n"),
            );
        }
    }
    let failed = rt.as_ref().is_some_and(|rt| {
        rt.assert_count(yavs_kernel::Severity::Error)
            + rt.assert_count(yavs_kernel::Severity::Failure)
            > 0
    });
    let mut code = if code == 0 && failed { 1 } else { code };
    // The live waves' last batch is queued here and sent before `exited`.
    let live = sh.lock_wave().take().map(|s| s.rec);
    let live = live.and_then(|rec| {
        rec.finish(&sim).err().map(|err| {
            Diagnostic::error("E0702", "the live waves failed").with_note(err.to_string())
        })
    });
    for d in live.into_iter().chain(finish.take().and_then(|f| f(&sim))) {
        sh.output(
            "stderr",
            &yavs_diag::render(&db.sources, &d, MessageFormat::Human, false),
        );
        code = code.max(1);
    }
    // A console, or a test runner's command waiting for this run, stays at the end.
    let hold = !sh.terminating.load(Ordering::Relaxed)
        && (link.console.load(Ordering::Relaxed) || console::awaiting(&sh));
    if hold {
        sh.ended.store(true, Ordering::Relaxed);
        let body = json!({
            "reason": "pause",
            "allThreadsStopped": true,
            "description": format!("the simulation ended at {}, delta {}", sim.now(), sim.delta()),
        });
        if let Some(Next::Restart) = serve_stop(&sh, &At::Kernel(&sim), body) {
            if let Some(rt) = &rt {
                rt.close_files();
            }
            let new = restart(&sh).flatten();
            return Ok(Outcome::Again(
                new.unwrap_or_else(|| again(&db_arc, &e_arc, &cfg)),
            ));
        }
    }
    if let Some(report) = report.take() {
        report(sim.stats(), rt.as_deref());
    }
    let code = console::exit_status(&sh).unwrap_or(code);
    // An editor shows nothing else of it; a terminal has the process's own status.
    if !link.console.load(Ordering::Relaxed) {
        sh.output(
            "console",
            &format!("the session ended with exit status {code}\n"),
        );
    }
    sh.event("exited", json!({"exitCode": code}));
    sh.event("terminated", json!({}));
    Ok(Outcome::Exit(code))
}

/// Reports and textio `OUTPUT` text as DAP output events, in the format `yavs run` prints.
struct Output {
    sh: Arc<Shared>,
    names: Vec<String>,
}

/// Send a report as an `output` event.
fn send_report(
    sh: &Shared,
    now: Time,
    delta: u32,
    severity: yavs_kernel::Severity,
    message: &str,
    at: &str,
) {
    if sh.discard.load(Ordering::Relaxed) {
        return;
    }
    let sev = match severity {
        yavs_kernel::Severity::Note => "Note",
        yavs_kernel::Severity::Warning => "Warning",
        yavs_kernel::Severity::Error => "Error",
        yavs_kernel::Severity::Failure => "Failure",
    };
    let text = format!("** {sev}: {}fs+{delta}: {message}\n    {at}\n", now.as_fs());
    sh.output("stdout", &text);
}

fn send_text(sh: &Shared, bytes: &[u8]) {
    if !sh.discard.load(Ordering::Relaxed) {
        sh.output("stdout", &String::from_utf8_lossy(bytes));
    }
}

impl yavs_kernel::Observer for Output {
    fn report(&mut self, hook: &mut yavs_kernel::Hook<'_>, r: &yavs_kernel::Report) {
        let at = match (r.process, &r.context) {
            (Some(p), _) => format!(
                "Process {}",
                self.names.get(p.0 as usize).map_or("?", |s| s.as_str())
            ),
            (None, c) => format!("Function {}", c.as_deref().unwrap_or("?")),
        };
        send_report(
            &self.sh,
            hook.now(),
            hook.delta(),
            r.severity,
            &r.message,
            &at,
        );
    }

    fn output(&mut self, _: &mut yavs_kernel::Hook<'_>, _: yavs_kernel::ProcId, bytes: &[u8]) {
        send_text(&self.sh, bytes);
    }

    // A report of a process that does not suspend reaches the client at once (#939).
    fn live_sink(&self) -> Option<Arc<dyn yavs_kernel::LiveSink>> {
        Some(Arc::new(LiveOutput(self.sh.clone())))
    }
}

/// [`Output`] for what a process says while it runs.
struct LiveOutput(Arc<Shared>);

impl yavs_kernel::LiveSink for LiveOutput {
    fn report(
        &self,
        now: Time,
        delta: u32,
        process: &str,
        severity: yavs_kernel::Severity,
        message: &str,
    ) {
        let at = format!("Process {process}");
        send_report(&self.0, now, delta, severity, message, &at);
    }

    fn output(&self, bytes: &[u8]) {
        send_text(&self.0, bytes);
    }
}
