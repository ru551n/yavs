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

mod cond;
mod design;
mod waves;
mod wire;

use std::collections::HashMap;
use std::io::{BufReader, Read, Write};
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
use yavs_std::value::{Lit, LitType, literal, literal_image};

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
}

/// How a stop ends.
#[derive(Clone, Copy)]
enum Next {
    Resume(Resume),
    Run(Run),
}

/// What the run loop hands the requests it serves while stopped.
enum Msg {
    Request(Json),
    Start,
    Eof,
}

/// State shared by the reader thread, the simulation thread and the front end.
struct Shared {
    out: Mutex<Box<dyn Write + Send>>,
    seq: Mutex<i64>,
    design: Design,
    session: OnceLock<Arc<DebugSession>>,
    interrupt: OnceLock<Interrupt>,
    rx: Mutex<Receiver<Msg>>,
    /// The simulation thread serves requests.
    stopped: AtomicBool,
    /// Disconnect or end of input: let the simulation finish without stops.
    terminating: AtomicBool,
    /// A client pause the simulation has not stopped for yet.
    pause_req: AtomicBool,
    /// A process stopped for the pause, so the kernel's pause after its cycle is not a stop.
    paused_in_process: AtomicBool,
    stop_on_entry: AtomicBool,
    /// A run requested at a stop inside a process, taken once its cycle has ended.
    queued: Mutex<Option<Run>>,
    /// The `disconnect` request to answer last.
    disconnect: Mutex<Option<Json>>,
    /// Breakpoint statements per absolute file path, so a request replaces one file's set.
    bps: Mutex<HashMap<String, Vec<yavs_ir::StmtId>>>,
    /// Signal breakpoints: those of `setDataBreakpoints` and of `yavs/setSignalBreakpoints`.
    sig_bps: Mutex<[Vec<SigBp>; 2]>,
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
    stream: Arc<waves::Stream>,
    db: Arc<Db>,
    elab: Arc<Elaborated>,
    built: OnceLock<Built>,
    sync: OnceLock<Sync>,
}

/// Work at the end of the run, before `exited` (the caller's `--wave` recorder finishing its
/// file); a diagnostic fails the run.
pub type Finish = Box<dyn FnOnce(&SimControl) -> Option<Diagnostic>>;

/// Work at every stop, before `stopped` is sent (the caller's `--wave` file flushed, so an
/// editor's wave viewer can read every change up to the stop).
pub type Sync = Box<dyn Fn() + Send + std::marker::Sync>;

/// Work after the run, with the final kernel statistics and interpreter runtime.
pub type Report = Box<dyn FnOnce(yavs_kernel::Stats, Option<&yavs_interp::Runtime>)>;

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
    /// Index into `Design::regions`.
    Region(usize),
}

impl Node {
    fn child(&self, k: usize) -> Node {
        let mut n = self.clone();
        if let Node::Local { path, .. } | Node::Signal { path, .. } = &mut n {
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

impl Shared {
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

    fn lock_wave(&self) -> MutexGuard<'_, Option<waves::Sub>> {
        self.wave.lock().unwrap_or_else(|p| p.into_inner())
    }

    fn event(&self, event: &str, body: Json) {
        self.send(json!({"type": "event", "event": event, "body": body}));
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

    fn session(&self) -> &Arc<DebugSession> {
        self.session.get().expect("set before the reader starts")
    }

    fn output(&self, category: &str, text: &str) {
        self.event("output", json!({"category": category, "output": text}));
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

    /// The stopped process, if the stop is inside one.
    fn process(&self) -> Option<usize> {
        match self {
            At::Process(s) => Some(s.process),
            At::Kernel(_) => None,
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
                            Err(m) => Err(format!("breakpoint condition: {m}")),
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
            StopReason::GlobalStep if sh.stop_on_entry.swap(false, Ordering::Relaxed) => {
                ("entry", None)
            }
            StopReason::GlobalStep => ("step", None),
            StopReason::Pause => {
                sh.pause_req.store(false, Ordering::Relaxed);
                sh.paused_in_process.store(true, Ordering::Relaxed);
                ("pause", None)
            }
            StopReason::ConditionError(m) => ("exception", Some(m.clone())),
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
            None => Resume::Continue,
        }
    }
}

/// Report the stop, then answer requests until one resumes; `None` ends the session.
fn serve_stop(sh: &Shared, at: &At<'_, '_>, stopped: Json) -> Option<Next> {
    sh.nodes.lock().unwrap_or_else(|p| p.into_inner()).clear();
    sh.stopped.store(true, Ordering::Relaxed);
    // The waves of the completed time steps, then the stop.
    sh.stream.push();
    if let Some(sync) = sh.sync.get() {
        sync();
    }
    sh.event("stopped", stopped);
    let rx = sh.rx.lock().unwrap_or_else(|p| p.into_inner());
    loop {
        let req = match rx.recv() {
            Ok(Msg::Request(r)) => r,
            Ok(Msg::Start) => continue,
            Ok(Msg::Eof) | Err(_) => {
                sh.stopped.store(false, Ordering::Relaxed);
                return None;
            }
        };
        let resume = match req["command"].as_str().unwrap_or_default() {
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
            "evaluate" => {
                evaluate(sh, at, &req);
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
            // Between cycles every step is a global step: the next statement of any process.
            let n = match (at, n) {
                (At::Kernel(_), Next::Resume(r)) if r != Resume::Continue => {
                    Next::Resume(Resume::GlobalStep)
                }
                (_, n) => n,
            };
            sh.stopped.store(false, Ordering::Relaxed);
            match n {
                Next::Resume(Resume::Continue) => {
                    sh.respond(&req, json!({"allThreadsContinued": true}))
                }
                Next::Resume(_) => sh.respond(&req, json!({})),
                // A client knows only its own requests resume: tell it.
                Next::Run(_) => {
                    sh.respond(&req, json!({}));
                    let tid = at.process().map_or(1, |p| p + 1);
                    sh.event(
                        "continued",
                        json!({"threadId": tid, "allThreadsContinued": true}),
                    );
                }
            }
            return Some(n);
        }
    }
}

/// The absolute time a `yavs/runFor` (relative) or `yavs/runUntil` (absolute) runs to.
fn run_to(at: &At<'_, '_>, req: &Json) -> Result<Time, String> {
    let text = req["arguments"]["time"]
        .as_str()
        .ok_or("expected a `time` argument such as \"10 ns\"")?;
    let t = yavs_std::value::time_literal(text)
        .map_err(|e| e.to_string())
        .and_then(|fs| Time::from_vhdl(fs).map_err(|e| e.to_string()))
        .map_err(|e| format!("`{text}` is not a time: {e}"))?;
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
    *sh.disconnect.lock().unwrap_or_else(|p| p.into_inner()) = Some(req);
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
                    p.name.clone()
                } else {
                    f.func_name(i).unwrap_or("?").to_string()
                };
                frames.push(frame(sh, tid << 16 | i as i64, &name, stmt));
            }
        }
        // A process not executing shows where it waits.
        _ if p.state == ProcState::Waiting => {
            frames.push(frame(sh, tid << 16, &p.name, p.wait));
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
    s.push(json!({"name": "Hierarchy", "variablesReference": id << 2, "expensive": true}));
    sh.respond(req, json!({"scopes": s}));
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
    let c = yavs_elab::ir_decode(&sh.prog, ty, &v)?;
    let (t, v) = design::ir_lit(&sh.prog, ty, &c)?;
    Some((t, v, c))
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
        Node::Region(_) => None,
    }
}

fn node_path(n: &Node) -> &[usize] {
    match n {
        Node::Local { path, .. } | Node::Signal { path, .. } => path,
        Node::Region(_) => &[],
    }
}

/// A variable showing value `v` of type `t`; a composite gets a reference to `node`, its
/// children (an array's paged through `indexedVariables`).
fn value_var(sh: &Shared, name: &str, t: &LitType, v: &Lit, node: impl FnOnce() -> Node) -> Json {
    let mut j = json!({"name": name, "value": literal_image(v, t), "variablesReference": 0});
    match (t, v) {
        (LitType::Array { .. }, Lit::Array(xs)) => {
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
        Some(v) => value_var(sh, &s.name, &s.lit, &v, || Node::Signal {
            sig: k,
            path: Vec::new(),
        }),
        None => json!({"name": s.name, "value": "?", "variablesReference": 0}),
    };
    j["type"] = json!(s.ty);
    // VS Code's context key for the extension's force, deposit and release.
    j["__vscodeVariableMenuContext"] = json!("signal");
    j
}

/// The locals of frame `i` of thread `tid`, as variables.
fn local_vars(sh: &Shared, at: &At<'_, '_>, tid: i64, i: usize) -> Vec<Json> {
    let Some(s) = stop_of(at, tid) else {
        return Vec::new();
    };
    s.frames
        .locals(i)
        .iter()
        .map(|l| match local(sh, s, i, l.name) {
            Some((t, v, _)) => value_var(sh, l.name, &t, &v, || Node::Local {
                tid,
                frame: i,
                name: l.name.to_string(),
                path: Vec::new(),
            }),
            // Access values.
            None => {
                let v = s
                    .frames
                    .format(l.ty, &l.bytes)
                    .unwrap_or_else(|| "?".into());
                json!({"name": l.name, "value": v, "variablesReference": 0})
            }
        })
        .collect()
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
        let name = p.rsplit(':').next().unwrap_or(p);
        out.push(json!({
            "name": name,
            "value": format!("process, {}", state.unwrap_or("?")),
            "variablesReference": 0,
            "evaluateName": p,
        }));
    }
    out.extend(
        sh.design.regions[r]
            .sigs
            .iter()
            .map(|&k| signal_var(sh, at, k)),
    );
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
            Some(n) => match node_root(sh, at, n).and_then(|(t, v)| descend(t, v, node_path(n))) {
                Some((LitType::Record(fs), Lit::Record(xs))) => fs
                    .iter()
                    .zip(&xs)
                    .enumerate()
                    .map(|(k, ((f, t), x))| value_var(sh, f, t, x, || n.child(k)))
                    .collect(),
                Some((t, Lit::Array(xs))) => {
                    let LitType::Array { elem, .. } = &t else {
                        return sh.fail(req, "this is a bug in yavs: an array of no array type");
                    };
                    xs.iter()
                        .enumerate()
                        .skip(start)
                        .take(count)
                        .map(|(k, x)| {
                            let i = t.index(k as u64).unwrap_or(k as i64);
                            value_var(sh, &i.to_string(), elem, x, || n.child(k))
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
    /// The local `name` names (frame and exact name), or the signal.
    fn find(&self, name: &str) -> Option<Result<(usize, String), usize>> {
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
            self.sh.design.find(None, &format!("{r}{name}"))
        };
        in_region()
            .filter(|_| proc.is_none())
            .or_else(|| self.sh.design.find(proc, name))
            .map(Err)
    }
}

impl cond::Env for Names<'_, '_, '_> {
    fn get(&self, name: &str) -> Option<(LitType, Lit)> {
        match self.find(name)? {
            Ok((i, n)) => {
                let (t, v, _) = local(self.sh, stop_of(self.at, self.tid)?, i, &n)?;
                Some((t, v))
            }
            Err(k) => {
                let s = &self.sh.design.sigs[k];
                Some((s.lit.clone(), s.value(&|r| self.at.read(r))?))
            }
        }
    }
}

/// A local of the frame, or a signal by name or path: its value (a composite expands).
fn evaluate(sh: &Shared, at: &At<'_, '_>, req: &Json) {
    let a = &req["arguments"];
    let expr = a["expression"].as_str().unwrap_or_default().trim();
    let (tid, i) = match a["frameId"].as_i64() {
        Some(f) => (f >> 16, (f & 0xffff) as usize),
        None => (at.process().map_or(0, |p| p as i64 + 1), 0),
    };
    let names = Names {
        sh,
        at,
        tid,
        frame: Some(i),
        region: None,
    };
    let found = names.find(expr);
    let mut j = match found {
        Some(Ok((frame, name))) => {
            let Some(s) = stop_of(at, tid) else {
                return sh.fail(req, "no locals here");
            };
            match local(sh, s, frame, &name) {
                Some((t, v, _)) => value_var(sh, &name, &t, &v, || Node::Local {
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
        Some(Err(k)) => signal_var(sh, at, k),
        None => return sh.fail(req, &format!("no variable or signal `{expr}` here")),
    };
    // A variable becomes a result.
    if let Some(o) = j.as_object_mut() {
        o.remove("name");
        o.remove("__vscodeVariableMenuContext");
        if let Some(v) = o.remove("value") {
            o.insert("result".into(), v);
        }
    }
    sh.respond(req, j)
}

/// `yavs/force`, `yavs/deposit` and `yavs/release` of a whole signal (`signal`: a name seen
/// from `frameId` or the stopped process, or a path; `value`: a literal of its type; `mode`:
/// `in`, the default, or `out`).
fn signal_op(sh: &Shared, at: &At<'_, '_>, req: &Json) {
    let a = &req["arguments"];
    let cmd = req["command"].as_str().unwrap_or_default();
    let r = (|| -> Result<Json, String> {
        let name = a["signal"].as_str().ok_or("expected a `signal` argument")?;
        let tid = match a["frameId"].as_i64() {
            Some(f) => f >> 16,
            None => at.process().map_or(0, |p| p as i64 + 1),
        };
        let (k, suffix) = split_part(sh, thread_proc(sh, tid), name)
            .ok_or_else(|| format!("no signal `{name}` here"))?;
        let s = &sh.design.sigs[k];
        let (paths, ty) = select(&s.lit, suffix)?;
        let target = format!("{}{}", s.path, suffix.to_ascii_lowercase());
        let mode = match a["mode"].as_str() {
            None | Some("in") => ForceMode::Effective,
            Some("out") => ForceMode::Driving,
            Some(m) => return Err(format!("force mode `{m}`: expected `in` or `out`")),
        };
        if cmd == "yavs/release" {
            put(sh, at, k, &paths, None, OpKind::Release(mode))?;
            return Ok(json!({"signal": target}));
        }
        let text = a["value"].as_str().ok_or("expected a `value` argument")?;
        let v = literal(text.as_bytes(), &ty).map_err(|e| e.to_string())?;
        let kind = if cmd == "yavs/force" {
            OpKind::Force(mode)
        } else {
            OpKind::Deposit
        };
        let mut whole = s
            .value(&|r| at.read(r))
            .ok_or("the signal is not readable here")?;
        set_part(&mut whole, &paths, &v)?;
        put(sh, at, k, &paths, Some(&whole), kind)?;
        Ok(json!({"signal": target, "value": literal_image(&v, &ty)}))
    })();
    match r {
        Ok(b) => sh.respond(req, b),
        Err(m) => sh.fail(req, &m),
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
                .ok_or_else(|| format!("no field `{f}`"))?;
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
    let leaves = s
        .part(paths, whole)
        .ok_or_else(|| format!("this is a bug in yavs: a value of `{}` not encoded", s.path))?;
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
            Some(n) => child_pos(sh, at, &n, name).map(|k| n.child(k)),
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
    *pv = literal(text.as_bytes(), pt).map_err(|e| e.to_string())?;
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
        Node::Region(_) => return Err("a region has no value".into()),
    }
    Ok(shown)
}

/// Requests the reader thread answers itself; the rest go to the simulation thread.
fn handle(sh: &Shared, tx: &Sender<Msg>, req: Json) {
    let cmd = req["command"].as_str().unwrap_or_default().to_string();
    match cmd.as_str() {
        "initialize" => {
            sh.respond(
                &req,
                json!({
                    "supportsConfigurationDoneRequest": true,
                    "supportsConditionalBreakpoints": true,
                    "supportsHitConditionalBreakpoints": true,
                    "supportsEvaluateForHovers": true,
                    "supportsTerminateRequest": true,
                    "supportsDataBreakpoints": true,
                    "supportsSetVariable": true,
                }),
            );
            sh.event("initialized", json!({}));
        }
        "launch" | "attach" => {
            let entry = req["arguments"]["stopOnEntry"].as_bool().unwrap_or(false);
            sh.stop_on_entry.store(entry, Ordering::Relaxed);
            sh.respond(&req, json!({}));
        }
        "setBreakpoints" => set_breakpoints(sh, &req),
        "dataBreakpointInfo" => data_breakpoint_info(sh, &req),
        "setDataBreakpoints" => set_signal_breakpoints(sh, &req, 0),
        "yavs/setSignalBreakpoints" => set_signal_breakpoints(sh, &req, 1),
        "setExceptionBreakpoints" => sh.respond(&req, json!({})),
        "yavs/hierarchy" => sh.respond(&req, hierarchy(sh)),
        "yavs/waveSubscribe" => wave_subscribe(sh, &req),
        "yavs/waveUnsubscribe" => {
            sh.lock_wave().take();
            sh.stream.push();
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
                .map(|(i, p)| json!({"id": i + 1, "name": p.name}))
                .collect();
            sh.respond(&req, json!({"threads": threads}));
        }
        "pause" => {
            // The response first: the stop it causes may otherwise reach the client before it
            // (#441), and a client waiting for the stop after the response would wait forever.
            sh.respond(&req, json!({}));
            sh.pause_req.store(true, Ordering::Relaxed);
            sh.session().request_pause();
            if let Some(i) = sh.interrupt.get() {
                i.pause();
            }
        }
        "disconnect" | "terminate" => {
            sh.terminating.store(true, Ordering::Relaxed);
            if let Some(i) = sh.interrupt.get() {
                i.stop();
            }
            let _ = tx.send(Msg::Request(req));
        }
        _ if sh.stopped.load(Ordering::Relaxed) => {
            let _ = tx.send(Msg::Request(req));
        }
        _ => sh.fail(&req, "the simulation is running"),
    }
}

/// `yavs/hierarchy`: the elaborated regions, nested, with their processes and signals.
fn hierarchy(sh: &Shared) -> Json {
    fn region(sh: &Shared, r: usize) -> Json {
        let reg = &sh.design.regions[r];
        json!({
            "name": reg.name,
            "path": reg.path,
            "kind": reg.kind,
            "regions": reg.children.iter().map(|&c| region(sh, c)).collect::<Vec<_>>(),
            "processes": reg.procs,
            "signals": reg.sigs.iter().map(|&k| {
                let s = &sh.design.sigs[k];
                json!({"name": s.name, "path": s.path, "type": s.ty})
            }).collect::<Vec<_>>(),
        })
    }
    json!({"regions": sh.design.roots.iter().map(|&r| region(sh, r)).collect::<Vec<_>>()})
}

/// Map each requested line to the statements starting on it (or the next line that has
/// some) and replace the file's breakpoints.
fn set_breakpoints(sh: &Shared, req: &Json) {
    let a = &req["arguments"];
    let path = a["source"]["path"].as_str().unwrap_or_default();
    let key = design::absolute(path);
    let session = sh.session();
    let mut set = sh.bps.lock().unwrap_or_else(|p| p.into_inner());
    let mut conds = sh.conds.lock().unwrap_or_else(|p| p.into_inner());
    for s in set.remove(&key).into_iter().flatten() {
        session.clear_breakpoint(s);
        conds.remove(&s);
    }
    let mut ids = Vec::new();
    let mut out = Vec::new();
    let empty = Vec::new();
    for bp in a["breakpoints"].as_array().unwrap_or(&empty) {
        let line = bp["line"].as_u64().unwrap_or(0) as u32;
        let hit = match bp["hitCondition"].as_str().map(parse_hit) {
            None => Ok(HitCond::Always),
            Some(h) => h,
        };
        // A condition is evaluated here; the interpreter then stops on every hit.
        let hit = hit.and_then(|hit| match bp["condition"].as_str() {
            None => Ok((hit, None)),
            Some(c) => cond::parse(c).map(|cond| {
                let c = Arc::new(LineCond {
                    cond,
                    hit,
                    hits: std::sync::atomic::AtomicU64::new(0),
                });
                (HitCond::Always, Some(c))
            }),
        });
        let found = sh.design.line_stmts(&key, line);
        match (found, hit) {
            (Some((l, stmts)), Ok((hit, c))) => {
                for &s in stmts {
                    session.set_breakpoint(s, LineBp::new(None, hit));
                    if let Some(c) = &c {
                        conds.insert(s, c.clone());
                    }
                    ids.push(s);
                }
                out.push(json!({"verified": true, "line": l, "source": source(&key)}));
            }
            (None, _) => out.push(json!({
                "verified": false,
                "line": line,
                "message": "no statement on or after this line",
            })),
            (_, Err(m)) => out.push(json!({"verified": false, "line": line, "message": m})),
        }
    }
    set.insert(key, ids);
    sh.respond(req, json!({"breakpoints": out}));
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
            .filter_map(|g| g.as_str().map(String::from))
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
    sh.stream.push();
    let r = yavs_wave::record_to(&sh.db, &sh.elab, sel, built, |_| {
        Ok(waves::sink(&sh.stream, every))
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
            json!({"dataId": path, "description": path, "accessTypes": ["write"], "canPersist": true})
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
    let tid = a["frameId"].as_i64().map_or(0, |f| f >> 16);
    let p = thread_proc(sh, tid);
    let mut bps = Vec::new();
    let mut out = Vec::new();
    let empty = Vec::new();
    for bp in a["breakpoints"].as_array().unwrap_or(&empty) {
        let name = bp[if set == 0 { "dataId" } else { "signal" }]
            .as_str()
            .unwrap_or_default();
        let r = (|| {
            let k = sh
                .design
                .find(p, name)
                .ok_or_else(|| format!("no signal `{name}` here"))?;
            let value = match bp["value"].as_str() {
                None => None,
                Some(v) => Some(sh.design.sigs[k].parse(v)?),
            };
            let cond = match bp["condition"].as_str() {
                None => None,
                Some(c) => Some(cond::parse(c)?),
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
                let d = &sh.design.sigs[b.sig].path;
                bps.push(b);
                json!({"verified": true, "description": d})
            }
            Err(m) => json!({"verified": false, "message": m}),
        });
    }
    let mut all = sh.sig_bps.lock().unwrap_or_else(|p| p.into_inner());
    all[set] = bps;
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
    sh.respond(req, json!({"breakpoints": out}));
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
                fires(b.hit, b.hits)
                    .then(|| ("data breakpoint", format!("{} = {}", s.path, s.show(&read))))
            }
            Ok(false) => None,
            Err(m) => Some((
                "exception",
                format!("{}: breakpoint condition: {m}", s.path),
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

/// Run the elaborated design under a DAP client until it ends; returns the exit status of
/// the run (docs/cli-contract.md section 5). `attach` subscribes the caller's observers
/// (`--wave`, `--vhpi`) to the built simulation and returns what to do at its end. `report`
/// receives the final statistics before the exit events.
pub fn serve(
    db: Arc<Db>,
    e: Arc<Elaborated>,
    cfg: Config,
    stop: Time,
    transport: Transport,
    attach: impl FnOnce(
        &mut SimControl,
        Option<&Built>,
    ) -> Result<Option<(Finish, Sync)>, Box<Diagnostic>>,
    report: Report,
) -> Result<u8, Box<Diagnostic>> {
    let (db_arc, e_arc) = (db, e);
    let (db, e) = (&*db_arc, &*e_arc);
    let Some(ir) = &e.ir else {
        return Err(Box::new(
            Diagnostic::error("E9999", "internal compiler error: this is a bug in yavs")
                .with_note("the design was not lowered before the DAP server started"),
        ));
    };
    let io_err = |what: &str, err: std::io::Error| {
        Box::new(Diagnostic::error(IO_ERROR, format!("DAP: {what}")).with_note(err.to_string()))
    };
    // The socket, to close it gracefully at the end.
    let mut closer = None;
    let (reader, writer): (Box<dyn Read + Send>, Box<dyn Write + Send>) = match transport {
        Transport::Stdio => {
            let out: Box<dyn Write + Send> = match STDOUT.get().map(std::fs::File::try_clone) {
                Some(f) => {
                    Box::new(std::io::BufWriter::new(f.map_err(|err| {
                        io_err("cannot duplicate standard output", err)
                    })?))
                }
                None => Box::new(std::io::stdout()),
            };
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
            let (s, _) = l.accept().map_err(|err| io_err("accept failed", err))?;
            let r = s.try_clone().map_err(|err| io_err("accept failed", err))?;
            closer = s.try_clone().ok();
            (Box::new(r), Box::new(s))
        }
    };
    let (tx, rx) = channel();
    let sh = Arc::new(Shared {
        out: Mutex::new(writer),
        seq: Mutex::new(0),
        design: Design::new(db, e, &ir.program),
        session: OnceLock::new(),
        interrupt: OnceLock::new(),
        rx: Mutex::new(rx),
        stopped: AtomicBool::new(false),
        terminating: AtomicBool::new(false),
        pause_req: AtomicBool::new(false),
        paused_in_process: AtomicBool::new(false),
        stop_on_entry: AtomicBool::new(false),
        queued: Mutex::new(None),
        disconnect: Mutex::new(None),
        bps: Mutex::new(HashMap::new()),
        sig_bps: Mutex::default(),
        watches: OnceLock::new(),
        hits: OnceLock::new(),
        ops: Mutex::new(Vec::new()),
        prog: ir.program.clone(),
        nodes: Mutex::new(Vec::new()),
        conds: Mutex::new(HashMap::new()),
        wave: Arc::default(),
        stream: Arc::default(),
        db: db_arc.clone(),
        elab: e_arc.clone(),
        built: OnceLock::new(),
        sync: OnceLock::new(),
    });
    let session = DebugSession::new(&ir.program, Box::new(Front(sh.clone())));
    let _ = sh.session.set(session.clone());
    // Stops inside a process need one thread (ir.md 3.4).
    let cfg = Config { threads: 1, ..cfg };
    let (mut sim, built) = match e.build_debug(cfg, &session) {
        Ok(x) => x,
        Err(err) => return Err(Box::new(e.runtime_diagnostic(&err))),
    };
    let _ = sh.interrupt.set(sim.interrupt());
    let names: Vec<String> = e.procs.iter().map(|p| e.proc_name(p)).collect();
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
    let finish = match attach(&mut sim, built.as_ref())? {
        Some((finish, sync)) => {
            let _ = sh.sync.set(sync);
            Some(finish)
        }
        None => None,
    };
    let rt = built.as_ref().map(|b| b.rt.clone());
    if let Some(b) = built {
        let _ = sh.built.set(b);
    }
    {
        let sh = sh.clone();
        std::thread::spawn(move || {
            waves::sender(sh.stream.clone(), || {
                let mut out = sh.out.lock().unwrap_or_else(|p| p.into_inner());
                sh.drain(&mut out);
            })
        });
    }
    {
        let sh = sh.clone();
        std::thread::spawn(move || {
            let mut r = BufReader::new(reader);
            while let Ok(Some(m)) = read_message(&mut r) {
                if m["type"] == "request" {
                    handle(&sh, &tx, m);
                }
            }
            sh.terminating.store(true, Ordering::Relaxed);
            if let Some(i) = sh.interrupt.get() {
                i.stop();
            }
            let _ = tx.send(Msg::Eof);
        });
    }
    // Wait for the configuration.
    {
        let rx = sh.rx.lock().unwrap_or_else(|p| p.into_inner());
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
            }
        }
    }
    if sh.stop_on_entry.load(Ordering::Relaxed) {
        session.request_global_step();
    }
    // The time the run goes to: `stop`, or earlier while a `yavs/runFor` is under way. A stop
    // on the way does not cancel it: continuing resumes it.
    let mut limit = stop;
    let mut next = None;
    let r = loop {
        apply_ops(&sh, &mut sim);
        let (r, step) = match next.take() {
            None => (sim.run_until(limit), false),
            Some(Run::Until(t)) => {
                limit = t.min(stop);
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
            None => break sim.run_until(stop),
        }
    };
    report(sim.stats(), rt.as_deref());
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
            let d = e.runtime_diagnostic(&err);
            sh.output(
                "stderr",
                &yavs_diag::render(&db.sources, &d, MessageFormat::Human, false),
            );
            if err.exit_status() == 101 { 3 } else { 1 }
        }
    };
    let failed = rt.is_some_and(|rt| {
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
    for d in live.into_iter().chain(finish.and_then(|f| f(&sim))) {
        sh.output(
            "stderr",
            &yavs_diag::render(&db.sources, &d, MessageFormat::Human, false),
        );
        code = code.max(1);
    }
    sh.event("exited", json!({"exitCode": code}));
    sh.event("terminated", json!({}));
    // Answer the disconnect, or wait for it.
    let mut pending = sh
        .disconnect
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .take();
    let rx = sh.rx.lock().unwrap_or_else(|p| p.into_inner());
    let mut eof = false;
    while pending.is_none() {
        match rx.recv() {
            Ok(Msg::Request(r))
                if matches!(r["command"].as_str(), Some("disconnect" | "terminate")) =>
            {
                pending = Some(r)
            }
            Ok(Msg::Request(r)) => sh.fail(&r, "the simulation has ended"),
            Ok(Msg::Start) => {}
            Ok(Msg::Eof) | Err(_) => {
                eof = true;
                break;
            }
        }
    }
    if let Some(d) = pending {
        sh.respond(&d, json!({}));
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
    Ok(code)
}

/// Reports and textio `OUTPUT` text as DAP output events, in the format `yavs run` prints.
struct Output {
    sh: Arc<Shared>,
    names: Vec<String>,
}

impl yavs_kernel::Observer for Output {
    fn report(&mut self, hook: &mut yavs_kernel::Hook<'_>, r: &yavs_kernel::Report) {
        let sev = match r.severity {
            yavs_kernel::Severity::Note => "Note",
            yavs_kernel::Severity::Warning => "Warning",
            yavs_kernel::Severity::Error => "Error",
            yavs_kernel::Severity::Failure => "Failure",
        };
        let at = match (r.process, &r.context) {
            (Some(p), _) => format!(
                "Process {}",
                self.names.get(p.0 as usize).map_or("?", |s| s.as_str())
            ),
            (None, c) => format!("Function {}", c.as_deref().unwrap_or("?")),
        };
        self.sh.output(
            "stdout",
            &format!(
                "** {sev}: {}fs+{}: {}\n    {at}\n",
                hook.now().as_fs(),
                hook.delta(),
                r.message
            ),
        );
    }

    fn output(&mut self, _: &mut yavs_kernel::Hook<'_>, _: yavs_kernel::ProcId, bytes: &[u8]) {
        self.sh.output("stdout", &String::from_utf8_lossy(bytes));
    }
}
