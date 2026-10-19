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

mod design;
mod wire;

use std::collections::HashMap;
use std::io::{BufReader, Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex, OnceLock};

use serde_json::{Value as Json, json};
use yavs_diag::{Diagnostic, MessageFormat};
use yavs_elab::Elaborated;
use yavs_interp::{DebugSession, Debugger, HitCond, LineBp, ProcState, Resume, Stop, StopReason};
use yavs_kernel::{Config, EndReason, Interrupt, RunStatus, SigRef, SimControl, Time, Value};
use yavs_sem::Db;

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
}

impl Shared {
    fn send(&self, mut m: Json) {
        let mut out = self.out.lock().unwrap_or_else(|p| p.into_inner());
        let mut seq = self.seq.lock().unwrap_or_else(|p| p.into_inner());
        *seq += 1;
        m["seq"] = json!(*seq);
        // A broken connection ends the session through the reader's end of input.
        let _ = write_message(&mut **out, &m);
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
        let (reason, text) = match &stop.reason {
            StopReason::Breakpoint { .. } => ("breakpoint", None),
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
    sh.stopped.store(true, Ordering::Relaxed);
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
    sh.respond(req, json!({"scopes": s}));
}

/// The process name of thread `tid`.
fn proc_name(sh: &Shared, tid: i64) -> Option<String> {
    let procs = sh.session().processes();
    Some(procs.get(usize::try_from(tid - 1).ok()?)?.name.clone())
}

/// Locals of frame `i` of the stopped process: name, value.
fn locals(at: &At<'_, '_>, tid: i64, i: usize) -> Vec<(String, String)> {
    let At::Process(s) = at else {
        return Vec::new();
    };
    if s.process as i64 != tid - 1 {
        return Vec::new();
    }
    s.frames
        .locals(i)
        .iter()
        .map(|l| {
            // ponytail: composite variables live outside the frame; show them once FrameView
            // can decode them through the heap.
            let v = s
                .frames
                .format(l.ty, l.bytes)
                .unwrap_or_else(|| "(...)".into());
            (l.name.to_string(), v)
        })
        .collect()
}

fn variables(sh: &Shared, at: &At<'_, '_>, req: &Json) {
    let r = req["arguments"]["variablesReference"].as_i64().unwrap_or(0);
    let (frame, kind) = (r >> 2, r & 3);
    let (tid, i) = (frame >> 16, (frame & 0xffff) as usize);
    let var = |name: &str, value: &str, ty: Option<&str>| {
        let mut v = json!({"name": name, "value": value, "variablesReference": 0});
        if let Some(t) = ty {
            v["type"] = json!(t);
        }
        v
    };
    let proc = proc_name(sh, tid).and_then(|n| sh.design.proc(&n));
    let vars: Vec<Json> = match kind {
        1 => locals(at, tid, i)
            .iter()
            .map(|(n, v)| var(n, v, None))
            .collect(),
        2 => proc
            .into_iter()
            .flat_map(|p| &p.signals)
            .map(|&k| {
                let s = &sh.design.sigs[k];
                var(&s.name, &s.show(&|r| at.read(r)), Some(&s.ty))
            })
            .collect(),
        3 => proc
            .into_iter()
            .flat_map(|p| &p.generics)
            .map(|(n, v)| var(n, v, None))
            .collect(),
        _ => return sh.fail(req, &format!("no variables {r}")),
    };
    sh.respond(req, json!({"variables": vars}));
}

/// A local of the frame, or a signal by name or path: its value.
fn evaluate(sh: &Shared, at: &At<'_, '_>, req: &Json) {
    let a = &req["arguments"];
    let expr = a["expression"].as_str().unwrap_or_default().trim();
    let (tid, i) = match a["frameId"].as_i64() {
        Some(f) => (f >> 16, (f & 0xffff) as usize),
        None => (at.process().map_or(0, |p| p as i64 + 1), 0),
    };
    let local = locals(at, tid, i)
        .into_iter()
        .find(|(n, _)| n.eq_ignore_ascii_case(expr));
    if let Some((_, v)) = local {
        return sh.respond(req, json!({"result": v, "variablesReference": 0}));
    }
    let proc = proc_name(sh, tid).and_then(|n| sh.design.proc(&n));
    match sh.design.lookup(proc, expr) {
        Some(s) => sh.respond(
            req,
            json!({"result": s.show(&|r| at.read(r)), "type": s.ty, "variablesReference": 0}),
        ),
        None => sh.fail(req, &format!("no variable or signal `{expr}` here")),
    }
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
                    "supportsConditionalBreakpoints": false,
                    "supportsHitConditionalBreakpoints": true,
                    "supportsEvaluateForHovers": true,
                    "supportsTerminateRequest": true,
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
        "setExceptionBreakpoints" => sh.respond(&req, json!({})),
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
            sh.pause_req.store(true, Ordering::Relaxed);
            sh.session().request_pause();
            if let Some(i) = sh.interrupt.get() {
                i.pause();
            }
            sh.respond(&req, json!({}));
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

/// Map each requested line to the statements starting on it (or the next line that has
/// some) and replace the file's breakpoints.
fn set_breakpoints(sh: &Shared, req: &Json) {
    let a = &req["arguments"];
    let path = a["source"]["path"].as_str().unwrap_or_default();
    let key = design::absolute(path);
    let session = sh.session();
    let mut set = sh.bps.lock().unwrap_or_else(|p| p.into_inner());
    for s in set.remove(&key).into_iter().flatten() {
        session.clear_breakpoint(s);
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
        let found = sh.design.line_stmts(&key, line);
        match (found, hit) {
            (Some((l, stmts)), Ok(hit)) => {
                for &s in stmts {
                    session.set_breakpoint(s, LineBp::new(None, hit));
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
/// the run (docs/cli-contract.md section 5).
pub fn serve(
    db: &Db,
    e: &Elaborated,
    cfg: Config,
    stop: Time,
    transport: Transport,
) -> Result<u8, Box<Diagnostic>> {
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
        Transport::Stdio => (Box::new(std::io::stdin()), Box::new(std::io::stdout())),
        Transport::Tcp(port) => {
            let l = TcpListener::bind(("127.0.0.1", port))
                .map_err(|err| io_err(&format!("cannot listen on port {port}"), err))?;
            let addr = l
                .local_addr()
                .map_err(|err| io_err("no local address", err))?;
            eprintln!("yavs: DAP listening on 127.0.0.1:{}", addr.port());
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
    });
    let session = DebugSession::new(&ir.program, Box::new(Front(sh.clone())));
    let _ = sh.session.set(session.clone());
    // Stops inside a process need one thread (ir.md 3.4).
    let cfg = Config { threads: 1, ..cfg };
    let (mut sim, rt) = match e.build_debug(cfg, &session) {
        Ok(x) => x,
        Err(err) => return Err(Box::new(e.runtime_diagnostic(&err))),
    };
    let _ = sh.interrupt.set(sim.interrupt());
    let names: Vec<String> = e.procs.iter().map(|p| e.proc_name(p)).collect();
    sim.subscribe(Box::new(Output {
        sh: sh.clone(),
        names,
    }));
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
        let queued = sh.queued.lock().unwrap_or_else(|p| p.into_inner()).take();
        let in_process = sh.paused_in_process.swap(false, Ordering::Relaxed);
        let reason = match queued {
            // The cycle stopped in has ended: that was the delta step.
            Some(Run::Delta) => "step",
            Some(run) => {
                next = Some(run);
                continue;
            }
            None if in_process => continue,
            None if sh.pause_req.swap(false, Ordering::Relaxed) => {
                session.cancel_pause();
                "pause"
            }
            None if step || limit < stop => {
                limit = stop;
                "step"
            }
            None => break r, // the stop time
        };
        let body = json!({
            "reason": reason,
            "allThreadsStopped": true,
            "description": format!("{reason} at {}, delta {}", sim.now(), sim.delta()),
        });
        match serve_stop(&sh, &At::Kernel(&sim), body) {
            Some(Next::Resume(Resume::GlobalStep)) => session.request_global_step(),
            Some(Next::Run(run)) => next = Some(run),
            Some(Next::Resume(_)) => {}
            None => break sim.run_until(stop),
        }
    };
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
    let code = if code == 0 && failed { 1 } else { code };
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
        let name = self
            .names
            .get(r.process.0 as usize)
            .map_or("?", |s| s.as_str());
        self.sh.output(
            "stdout",
            &format!(
                "** {sev}: {}fs+{}: {}\n    Process {name}\n",
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
