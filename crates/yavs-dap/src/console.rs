//! The console: the commands of `yavs run --interactive` and `--do`, and of an `evaluate` with
//! context `repl` (VS Code's Debug Console). One parser, on the run control of the server.
//!
//! A line becomes a `Batch`, served at a stop one command at a time (`work`). A command that
//! resumes the simulation leaves the stop, and the batch goes on at the next stop. A test
//! runner's command (`Hook`) waits for its `done`, and while it waits the runner's `exec` lines
//! are batches of their own. Protocol: `docs/cli-contract.md` "Console hook".

use std::collections::{BTreeMap, VecDeque};
use std::io::{BufRead, BufReader, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicI64, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Mutex, MutexGuard};
use std::time::Duration;

use serde_json::{Value as Json, json};
use yavs_interp::Resume;
use yavs_kernel::Time;
use yavs_std::value::{Lit, LitType, literal_image};

use crate::{
    At, CONSOLE_BREAKS, Link, Msg, Next, RUN_UNTIL, Run, Shared, SigBp, apply_lines, apply_signals,
    cond, eval_var, rewatch, select, signal_change, split_part, thread_proc,
};

/// The environment variable naming a test runner's hook: `<port>:<secret>`.
pub const HOOK_VAR: &str = "YAVS_CONSOLE_HOOK";
/// Code: the hook cannot be used.
pub(crate) const HOOK_ERROR: &str = "E0703";
const HOOK_TIMEOUT: Duration = Duration::from_secs(30);

pub(crate) const ENDED: &str = "the simulation has ended: `restart` runs it again, `quit` leaves";

const COMMANDS: &[(&str, &str)] = &[
    (
        "run",
        "run [TIME | until CONDITION]: to the end or a breakpoint, for TIME, or until a change of a signal in the condition makes it true",
    ),
    (
        "step",
        "step [delta | timestep]: to the next statement, entering calls; or to the end of the delta or time step",
    ),
    ("next", "next: to the next statement, over calls"),
    (
        "stop",
        "stop: pause a running simulation (Ctrl-C on a terminal)",
    ),
    (
        "examine",
        "examine NAME: the value of a local or a signal, or a field, element or slice of one (also `print`)",
    ),
    ("print", ""),
    (
        "force",
        "force NAME VALUE [-out]: hold a signal at a value, from the next cycle",
    ),
    (
        "deposit",
        "deposit NAME VALUE: give a signal a value once, in the next cycle",
    ),
    ("release", "release NAME [-out]: end a force"),
    ("where", "where: time, delta, process and source line"),
    (
        "break",
        "break [FILE:]LINE [if CONDITION] | SIGNAL [= VALUE] [if CONDITION] | list | delete N | delete all",
    ),
    (
        "restart",
        "restart: back to time 0 with the same design and breakpoints (also `reset`)",
    ),
    ("reset", ""),
    (
        "source",
        "source FILE: run the commands of a file; the first failure ends it",
    ),
    ("help", "help: this list"),
    (
        "quit",
        "quit [STATUS]: leave; the exit status is STATUS, else that of the ended run, else 0",
    ),
];

/// What the client asked for at breakpoints, kept over a restart and resolved again.
#[derive(Default)]
pub(crate) struct Kept {
    lines: BTreeMap<String, Vec<Json>>,
    signals: [Vec<Json>; CONSOLE_BREAKS + 1],
    next_id: u64,
}

impl Kept {
    fn numbered(&mut self, mut list: Vec<Json>) -> Vec<Json> {
        for b in &mut list {
            self.next_id += 1;
            b["yavsId"] = json!(self.next_id);
        }
        list
    }

    pub(crate) fn set_lines(&mut self, key: &str, list: Vec<Json>) {
        let list = self.numbered(list);
        self.lines.insert(key.to_string(), list);
    }

    pub(crate) fn lines(&self, key: &str) -> Vec<Json> {
        self.lines.get(key).cloned().unwrap_or_default()
    }

    pub(crate) fn set_signals(&mut self, set: usize, list: Vec<Json>) {
        self.signals[set] = self.numbered(list);
    }

    pub(crate) fn signals(&self, set: usize) -> Vec<Json> {
        self.signals[set].clone()
    }
}

/// Resolve what is kept against a new simulation; what no longer resolves is said.
pub(crate) fn keep_breakpoints(sh: &Shared) {
    let (files, sets): (Vec<String>, Vec<usize>) = {
        let k = kept(sh);
        let sets = (0..k.signals.len()).filter(|&s| !k.signals[s].is_empty());
        (k.lines.keys().cloned().collect(), sets.collect())
    };
    let lost = files
        .iter()
        .flat_map(|f| apply_lines(sh, f))
        .chain(sets.into_iter().flat_map(|s| apply_signals(sh, s)));
    for b in lost.filter(|b| b["verified"] != true) {
        let m = b["message"].as_str().unwrap_or_default();
        sh.output("stderr", &format!("a kept breakpoint is lost: {m}\n"));
    }
}

fn kept<'a>(sh: &'a Shared) -> MutexGuard<'a, Kept> {
    sh.link.kept.lock().unwrap_or_else(|p| p.into_inner())
}

enum Origin {
    /// An `evaluate` request, until it is answered.
    Client(Option<Json>),
    /// A runner's `exec` or `reload`, answered when the batch ends.
    Hook(i64),
}

struct Batch {
    origin: Origin,
    lines: VecDeque<String>,
    text: String,
    error: Option<String>,
    /// The id of the runner's command under way.
    awaiting: Option<i64>,
}

#[derive(Default)]
pub(crate) struct State {
    batches: Vec<Batch>,
    /// `quit`'s status.
    exit: Option<u8>,
    failed: bool,
    told: bool,
}

fn state<'a>(link: &'a Link) -> MutexGuard<'a, State> {
    link.con.lock().unwrap_or_else(|p| p.into_inner())
}

/// What a command leaves for the batch.
enum Exec {
    Text(String),
    Next(Next),
    Script(Vec<String>),
    Call(i64),
}

/// The commands of a line (`;` separates, `#` at the start of a command and `--` end the line;
/// not inside strings and character literals).
fn split(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let b = line.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        match c {
            b'"' => quoted = !quoted,
            b'\'' if !quoted && b.get(i + 2) == Some(&b'\'') => {
                cur.push_str(&line[i..i + 3]);
                i += 3;
                continue;
            }
            b';' if !quoted => {
                out.push(std::mem::take(&mut cur));
                i += 1;
                continue;
            }
            b'#' if !quoted && cur.trim().is_empty() => break,
            b'-' if !quoted && b.get(i + 1) == Some(&b'-') => break,
            _ => {}
        }
        // Whole characters: everything the matches above look at is ASCII.
        let n = line[i..].chars().next().map_or(1, char::len_utf8);
        cur.push_str(&line[i..i + n]);
        i += n;
    }
    out.push(cur);
    out.iter()
        .map(|c| c.trim().to_string())
        .filter(|c| !c.is_empty())
        .collect()
}

/// Whether a request is a console line: anything in a console session, in another one what
/// starts with a command (a plain name stays an evaluation).
pub(crate) fn is_command(sh: &Shared, req: &Json) -> bool {
    if req["arguments"]["context"] != "repl" {
        return false;
    }
    if sh.link.console.load(Ordering::Relaxed) {
        return true;
    }
    let text = req["arguments"]["expression"].as_str().unwrap_or_default();
    let line = split(text);
    let word = line
        .first()
        .and_then(|c| c.split_whitespace().next())
        .unwrap_or_default();
    COMMANDS.iter().any(|(n, _)| *n == word)
        || sh
            .link
            .hook
            .get()
            .is_some_and(|h| h.commands.iter().any(|(n, _)| n == word))
}

/// Take an `evaluate` as a console line.
pub(crate) fn start(sh: &Shared, req: Json) {
    let text = req["arguments"]["expression"].as_str().unwrap_or_default();
    let lines = split(text).into();
    push(&sh.link, Origin::Client(Some(req)), lines);
}

fn push(link: &Link, origin: Origin, lines: VecDeque<String>) {
    let mut st = state(link);
    st.told = false;
    st.batches.push(Batch {
        origin,
        lines,
        text: String::new(),
        error: None,
        awaiting: None,
    });
}

impl Batch {
    fn say(&mut self, link: &Link, text: &str) {
        if text.is_empty() {
            return;
        }
        if let Origin::Client(None) = self.origin {
            link.output("console", &format!("{text}\n"));
            return;
        }
        if !self.text.is_empty() {
            self.text.push('\n');
        }
        self.text.push_str(text);
    }

    /// Answer the request now: what comes later arrives as output.
    fn flush(&mut self, link: &Link) {
        if let Origin::Client(req) = &mut self.origin
            && let Some(req) = req.take()
        {
            let text = std::mem::take(&mut self.text);
            link.respond(&req, json!({"result": text, "variablesReference": 0}));
        }
    }

    fn finish(mut self, link: &Link, st: &mut State) {
        match self.origin {
            Origin::Client(Some(ref req)) => match &self.error {
                Some(m) => {
                    // What the commands before it said is not lost.
                    if !self.text.is_empty() {
                        link.output("console", &format!("{}\n", self.text));
                    }
                    link.fail(req, m);
                }
                None => link.respond(req, json!({"result": self.text, "variablesReference": 0})),
            },
            Origin::Client(None) => {
                if let Some(m) = &self.error {
                    link.output("stderr", &format!("error: {m}\n"));
                }
            }
            Origin::Hook(id) => {
                let text = std::mem::take(&mut self.text);
                if let Some(h) = link.hook.get() {
                    h.send(&json!({
                        "result": id,
                        "ok": self.error.is_none(),
                        "text": text,
                        "error": self.error,
                    }));
                }
                return;
            }
        }
        st.failed |= self.error.is_some();
    }
}

/// Serve the batches at this stop until one resumes the simulation or all wait; then, on a
/// console, tell the client it may send the next line.
pub(crate) fn work(sh: &Shared, at: &At<'_, '_>) -> Option<Next> {
    let link = &*sh.link;
    loop {
        let mut st = state(link);
        let Some(b) = st.batches.last_mut() else {
            if !st.told && link.console.load(Ordering::Relaxed) {
                st.told = true;
                let failed = std::mem::take(&mut st.failed);
                drop(st);
                link.event("yavs/prompt", json!({"failed": failed}));
            }
            return None;
        };
        if b.awaiting.is_some() {
            return None;
        }
        let Some(line) = b.lines.pop_front() else {
            let b = st.batches.pop().expect("the last batch is there");
            b.finish(link, &mut st);
            continue;
        };
        drop(st);
        let r = exec(sh, at, &line);
        let mut st = state(link);
        let b = st.batches.last_mut().expect("a command keeps its batch");
        match r {
            Ok(Exec::Text(t)) => b.say(link, &t),
            Ok(Exec::Script(mut lines)) => {
                lines.extend(std::mem::take(&mut b.lines));
                b.lines = lines.into();
            }
            Ok(Exec::Call(id)) => {
                b.flush(link);
                b.awaiting = Some(id);
            }
            Ok(Exec::Next(n)) => {
                b.flush(link);
                st.told = false;
                return Some(n);
            }
            Err(m) => {
                b.error = Some(m);
                b.lines.clear();
            }
        }
    }
}

/// Whether a runner's command is under way (the simulation then stays at its end for it).
pub(crate) fn awaiting(sh: &Shared) -> bool {
    state(&sh.link).batches.iter().any(|b| b.awaiting.is_some())
}

/// The status `quit` asked for.
pub(crate) fn exit_status(sh: &Shared) -> Option<u8> {
    state(&sh.link).exit
}

/// Say what stopped the run, on a console; the signal conditions of `run until` are over.
pub(crate) fn on_stop(sh: &Shared, at: &At<'_, '_>, stopped: &Json) {
    let until = {
        let mut all = sh.sig_bps.lock().unwrap_or_else(|p| p.into_inner());
        !std::mem::take(&mut all[RUN_UNTIL]).is_empty()
    };
    if until {
        rewatch(sh);
    }
    if !sh.link.console.load(Ordering::Relaxed) || stopped["reason"] == "entry" {
        return;
    }
    let mut text = stopped["description"]
        .as_str()
        .unwrap_or("stopped")
        .to_string();
    if let At::Process(s) = at
        && let Some(l) = sh.design.loc(s.stmt)
    {
        text += &format!(", {}:{}", l.path, l.line);
    }
    if let Some(t) = stopped["text"].as_str() {
        text += &format!(": {t}");
    }
    sh.output("important", &format!("{text}\n"));
}

/// The first word of `line` and the rest.
fn word(line: &str) -> (&str, &str) {
    let line = line.trim();
    match line.find(char::is_whitespace) {
        Some(i) => (&line[..i], line[i..].trim_start()),
        None => (line, ""),
    }
}

/// A name that can hold spaces inside parentheses (`s(3 downto 0)`), and the rest.
fn name_word(text: &str) -> (&str, &str) {
    let text = text.trim();
    let mut depth = 0;
    for (i, c) in text.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            c if c.is_whitespace() && depth == 0 => return (&text[..i], text[i..].trim_start()),
            _ => {}
        }
    }
    (text, "")
}

/// `name` as the signal functions read it: between cycles no process sees it, so a simple name
/// is taken from the top level.
fn qualify(sh: &Shared, at: &At<'_, '_>, name: &str) -> String {
    if at.process().is_some() || split_part(sh, None, name).is_some() {
        return name.to_string();
    }
    let Some(&top) = sh.design.roots.first() else {
        return name.to_string();
    };
    let full = format!("{}{}", sh.design.regions[top].path, name);
    match split_part(sh, None, &full) {
        Some(_) => full,
        None => name.to_string(),
    }
}

fn thread_of(at: &At<'_, '_>) -> i64 {
    at.process().map_or(0, |p| p as i64 + 1)
}

fn examine(sh: &Shared, at: &At<'_, '_>, name: &str) -> Result<String, String> {
    if name.is_empty() {
        return Err("`examine` needs a name".into());
    }
    let name = qualify(sh, at, name);
    let tid = thread_of(at);
    let frame = match at {
        At::Process(s) => s.frames.depth().saturating_sub(1),
        At::Kernel(_) => 0,
    };
    let whole = eval_var(sh, at, tid, frame, &name);
    if let Ok(j) = &whole {
        return Ok(j["result"].as_str().unwrap_or("?").to_string());
    }
    let Some((k, suffix)) = split_part(sh, thread_proc(sh, tid), &name) else {
        return whole.map(|_| String::new());
    };
    let s = &sh.design.sigs[k];
    let (paths, ty) = select(&s.lit, suffix)?;
    let v = s
        .value(&|r| at.read(r))
        .ok_or("the signal is not readable here")?;
    let at_path = |p: &[usize]| {
        let mut cur = &v;
        for &i in p {
            cur = match cur {
                Lit::Record(xs) | Lit::Array(xs) => &xs[i],
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
    Ok(literal_image(&part, &ty))
}

fn change(sh: &Shared, at: &At<'_, '_>, cmd: &str, rest: &str) -> Result<String, String> {
    let (name, mut rest) = name_word(rest);
    if name.is_empty() {
        return Err(format!("`{cmd}` needs a signal"));
    }
    let mode = if let Some(r) = rest.strip_suffix("-out") {
        rest = r.trim_end();
        "out"
    } else {
        "in"
    };
    if cmd != "release" && rest.is_empty() {
        return Err(format!("`{cmd}` needs a value"));
    }
    let name = qualify(sh, at, name);
    let args = json!({"signal": name, "value": rest, "mode": mode});
    let r = signal_change(sh, at, &format!("yavs/{cmd}"), &args)?;
    Ok(match r["value"].as_str() {
        Some(v) => format!("{} = {v}", r["signal"].as_str().unwrap_or_default()),
        None => r["signal"].as_str().unwrap_or_default().to_string(),
    })
}

fn where_text(sh: &Shared, at: &At<'_, '_>) -> String {
    match at {
        At::Process(s) => {
            let mut t = match (s.now, s.delta) {
                (Some(n), Some(d)) => format!("time {n}, delta {d}\n"),
                _ => "before time 0\n".to_string(),
            };
            let procs = sh.session().processes();
            t += procs.get(s.process).map_or("?", |p| &p.name);
            if let Some(l) = sh.design.loc(s.stmt) {
                t += &format!(", {}:{}", l.path, l.line);
            }
            t
        }
        At::Kernel(sim) => format!("time {}, delta {}\nbetween cycles", sim.now(), sim.delta()),
    }
}

/// A condition's text after ` if `, and what comes before.
fn split_if(text: &str) -> (&str, Option<&str>) {
    match text.split_once(" if ") {
        Some((a, c)) => (a.trim(), Some(c.trim())),
        None => (text.trim(), None),
    }
}

fn add_field(b: &mut Json, name: &str, v: Option<&str>) {
    if let Some(v) = v {
        b[name] = json!(v);
    }
}

fn break_cmd(sh: &Shared, at: &At<'_, '_>, rest: &str) -> Result<String, String> {
    let (first, tail) = word(rest);
    match first {
        "" => return Err("`break` needs a place: see `help`".into()),
        "list" => return Ok(break_list(sh)),
        "delete" => return break_delete(sh, tail),
        _ => {}
    }
    let (head, cond_text) = split_if(rest);
    let line = match head.rsplit_once(':') {
        Some((f, l)) if !f.is_empty() && l.parse::<u32>().is_ok() => {
            Some((Some(f), l.parse::<u32>().unwrap_or(0)))
        }
        _ => head.parse::<u32>().ok().map(|l| (None, l)),
    };
    if let Some((file, line)) = line {
        return break_line(sh, at, file, line, cond_text);
    }
    let (signal, value) = match head.split_once('=') {
        Some((s, v)) => (s.trim(), Some(v.trim())),
        None => (head, None),
    };
    let mut b = json!({"signal": qualify(sh, at, signal)});
    add_field(&mut b, "value", value);
    add_field(&mut b, "condition", cond_text);
    let id = {
        let mut k = kept(sh);
        k.next_id += 1;
        b["yavsId"] = json!(k.next_id);
        k.signals[CONSOLE_BREAKS].push(b);
        k.next_id
    };
    let out = apply_signals(sh, CONSOLE_BREAKS);
    let r = out.last().cloned().unwrap_or(Json::Null);
    if r["verified"] != true {
        kept(sh).signals[CONSOLE_BREAKS].pop();
        apply_signals(sh, CONSOLE_BREAKS);
        return Err(r["message"].as_str().unwrap_or_default().to_string());
    }
    Ok(format!(
        "breakpoint {id}: {}",
        r["description"].as_str().unwrap_or_default()
    ))
}

fn break_line(
    sh: &Shared,
    at: &At<'_, '_>,
    file: Option<&str>,
    line: u32,
    cond_text: Option<&str>,
) -> Result<String, String> {
    let key = match file {
        Some(f) => sh.design.source_file(f)?,
        None => match at {
            At::Process(s) => sh
                .design
                .loc(s.stmt)
                .map(|l| l.path.clone())
                .ok_or("this stop has no source file: write FILE:LINE")?,
            At::Kernel(_) => return Err("no source file here: write FILE:LINE".into()),
        },
    };
    let mut b = json!({"line": line});
    add_field(&mut b, "condition", cond_text);
    let id = {
        let mut k = kept(sh);
        k.next_id += 1;
        b["yavsId"] = json!(k.next_id);
        k.lines.entry(key.clone()).or_default().push(b);
        k.next_id
    };
    let out = apply_lines(sh, &key);
    let r = out.last().cloned().unwrap_or(Json::Null);
    if r["verified"] != true {
        if let Some(l) = kept(sh).lines.get_mut(&key) {
            l.pop();
        }
        apply_lines(sh, &key);
        return Err(r["message"].as_str().unwrap_or_default().to_string());
    }
    Ok(format!("breakpoint {id}: {key}:{}", r["line"]))
}

fn break_list(sh: &Shared) -> String {
    let k = kept(sh);
    let mut out = Vec::new();
    for (f, list) in &k.lines {
        for b in list {
            let mut t = format!("{} {f}:{}", b["yavsId"], b["line"]);
            if let Some(c) = b["condition"].as_str() {
                t += &format!(" if {c}");
            }
            out.push(t);
        }
    }
    for (set, list) in k.signals.iter().enumerate() {
        for b in list {
            let name = &b[if set == 0 { "dataId" } else { "signal" }];
            let mut t = format!("{} {}", b["yavsId"], name.as_str().unwrap_or_default());
            if let Some(v) = b["value"].as_str() {
                t += &format!(" = {v}");
            }
            if let Some(c) = b["condition"].as_str() {
                t += &format!(" if {c}");
            }
            out.push(t);
        }
    }
    if out.is_empty() {
        return "no breakpoints".into();
    }
    out.join("\n")
}

fn break_delete(sh: &Shared, what: &str) -> Result<String, String> {
    let all = what == "all";
    let id = what.parse::<u64>().ok();
    if !all && id.is_none() {
        return Err(format!("`break delete {what}`: expected a number or `all`"));
    }
    let gone = |b: &Json| all || b["yavsId"].as_u64() == id;
    let (files, sets, n) = {
        let mut k = kept(sh);
        let mut n = 0;
        let mut files = Vec::new();
        for (f, list) in &mut k.lines {
            let before = list.len();
            list.retain(|b| !gone(b));
            if list.len() != before {
                n += before - list.len();
                files.push(f.clone());
            }
        }
        let mut sets = Vec::new();
        for (set, list) in k.signals.iter_mut().enumerate() {
            let before = list.len();
            list.retain(|b| !gone(b));
            if list.len() != before {
                n += before - list.len();
                sets.push(set);
            }
        }
        (files, sets, n)
    };
    if n == 0 && !all {
        return Err(format!("no breakpoint {what}: see `break list`"));
    }
    for f in &files {
        apply_lines(sh, f);
    }
    for s in sets {
        apply_signals(sh, s);
    }
    Ok(format!("{n} deleted"))
}

/// `run until`: stop at the first change of a signal the condition names after which it holds.
fn run_until(sh: &Shared, at: &At<'_, '_>, text: &str) -> Result<Exec, String> {
    let first = cond::parse(text)?;
    let proc = thread_proc(sh, thread_of(at));
    let sigs: Vec<usize> = first
        .names()
        .iter()
        .map(|n| qualify(sh, at, n))
        .filter_map(|n| split_part(sh, proc, &n).map(|(k, _)| k))
        .collect();
    if sigs.is_empty() {
        return Err(format!("`{text}` names no signal to watch"));
    }
    let tid = thread_of(at);
    let mut bps = Vec::new();
    for sig in sigs {
        bps.push(SigBp {
            sig,
            value: None,
            cond: Some(cond::parse(text)?),
            hit: yavs_interp::HitCond::Always,
            hits: 0,
            tid,
        });
    }
    sh.sig_bps.lock().unwrap_or_else(|p| p.into_inner())[RUN_UNTIL] = bps;
    rewatch(sh);
    Ok(Exec::Next(Next::Run(Run::ToEnd)))
}

fn run(sh: &Shared, at: &At<'_, '_>, rest: &str) -> Result<Exec, String> {
    if sh.ended.load(Ordering::Relaxed) {
        // As a simulator's `run` past the end: nothing to do, and no error.
        return Ok(Exec::Text(ENDED.to_string()));
    }
    if rest.is_empty() {
        return Ok(Exec::Next(Next::Run(Run::ToEnd)));
    }
    if let ("until", cond_text) = word(rest) {
        return run_until(sh, at, cond_text);
    }
    let t = yavs_std::value::time_literal(rest)
        .map_err(|e| e.to_string())
        .and_then(|fs| Time::from_vhdl(fs).map_err(|e| e.to_string()))
        .map_err(|e| format!("`{rest}` is not a time: {e}"))?;
    let now = at.now().ok_or("no simulation time at this stop")?;
    let to = now.checked_add(t).map_err(|e| e.to_string())?;
    Ok(Exec::Next(Next::Run(Run::Until(to))))
}

fn step(sh: &Shared, n: Next) -> Result<Exec, String> {
    if sh.ended.load(Ordering::Relaxed) {
        return Err(ENDED.into());
    }
    Ok(Exec::Next(n))
}

fn restart(sh: &Shared) -> Result<Exec, String> {
    if let Some(why) = &sh.link.no_restart {
        return Err(why.clone());
    }
    *sh.restart.lock().unwrap_or_else(|p| p.into_inner()) = Some(None);
    Ok(Exec::Next(Next::Restart))
}

fn help(sh: &Shared) -> String {
    let core = COMMANDS.iter().map(|(_, h)| *h).filter(|h| !h.is_empty());
    let mut lines: Vec<String> = core.map(String::from).collect();
    if let Some(h) = sh.link.hook.get() {
        let shown = h.commands.iter().filter(|(_, help)| !help.is_empty());
        lines.extend(shown.map(|(n, help)| format!("{n}: {help}")));
    }
    lines.join("\n")
}

fn exec(sh: &Shared, at: &At<'_, '_>, line: &str) -> Result<Exec, String> {
    let (cmd, rest) = word(line);
    match cmd {
        "run" => run(sh, at, rest),
        "step" => match rest {
            "" => step(sh, Next::Resume(Resume::StepIn)),
            "delta" => step(sh, Next::Run(Run::Delta)),
            "timestep" => step(sh, Next::Run(Run::Timestep)),
            _ => Err(format!("`step {rest}`: expected `delta` or `timestep`")),
        },
        "next" => step(sh, Next::Resume(Resume::StepOver)),
        "stop" => Ok(Exec::Text("the simulation is stopped".into())),
        "examine" | "print" => examine(sh, at, rest).map(Exec::Text),
        "force" | "deposit" | "release" => change(sh, at, cmd, rest).map(Exec::Text),
        "where" => Ok(Exec::Text(where_text(sh, at))),
        "break" => break_cmd(sh, at, rest).map(Exec::Text),
        "restart" | "reset" => restart(sh),
        "source" => {
            let text =
                std::fs::read_to_string(rest).map_err(|e| format!("cannot read `{rest}`: {e}"))?;
            Ok(Exec::Script(text.lines().flat_map(split).collect()))
        }
        "help" => Ok(Exec::Text(help(sh))),
        "quit" => quit(sh, rest),
        _ => {
            if let Some(h) = sh.link.hook.get()
                && h.commands.iter().any(|(n, _)| n == cmd)
            {
                return h.call(cmd, rest).map(Exec::Call);
            }
            examine(sh, at, line)
                .map(Exec::Text)
                .map_err(|_| format!("unknown command or signal `{cmd}`: see `help`"))
        }
    }
}

fn quit(sh: &Shared, rest: &str) -> Result<Exec, String> {
    let status = match rest {
        "" => (!sh.ended.load(Ordering::Relaxed)).then_some(0),
        s => Some(
            s.parse::<u8>()
                .map_err(|_| format!("`quit {s}`: expected a status 0..255"))?,
        ),
    };
    state(&sh.link).exit = status;
    sh.terminating.store(true, Ordering::Relaxed);
    sh.discard.store(true, Ordering::Relaxed);
    if let Some(i) = sh.interrupt.get() {
        i.stop();
    }
    Ok(Exec::Next(Next::Quit))
}

/// A test runner's connection: it serves the commands it registered.
pub(crate) struct Hook {
    w: Mutex<TcpStream>,
    pub(crate) commands: Vec<(String, String)>,
    next: AtomicI64,
    gone: AtomicBool,
}

impl Hook {
    /// Connect to the hook in `value` and read what it registers; the error is a note for E0703.
    pub(crate) fn connect(value: &str) -> Result<(Hook, BufReader<TcpStream>), String> {
        // The secret is never printed.
        let (port, secret) = value
            .split_once(':')
            .and_then(|(p, s)| Some((p.parse::<u16>().ok().filter(|&p| p != 0)?, s)))
            .filter(|(_, s)| !s.is_empty())
            .ok_or_else(|| format!("`{HOOK_VAR}` is not `<port>:<secret>`"))?;
        let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
        let fail = |e: std::io::Error| format!("the hook on {addr}: {e}");
        let s = TcpStream::connect_timeout(&addr, HOOK_TIMEOUT).map_err(fail)?;
        let _ = s.set_nodelay(true);
        let mut w = s.try_clone().map_err(fail)?;
        writeln!(w, "{}", json!({"hello": 1, "secret": secret})).map_err(fail)?;
        s.set_read_timeout(Some(HOOK_TIMEOUT)).map_err(fail)?;
        let mut r = BufReader::new(s);
        let mut line = String::new();
        r.read_line(&mut line).map_err(fail)?;
        r.get_ref().set_read_timeout(None).map_err(fail)?;
        let m: Json = serde_json::from_str(&line)
            .map_err(|_| format!("the hook on {addr} answered no command list"))?;
        let list = m["commands"]
            .as_array()
            .ok_or_else(|| format!("the hook on {addr} answered no command list"))?;
        let mut commands = Vec::new();
        for c in list {
            let name = c["name"].as_str().unwrap_or_default().to_string();
            if name.is_empty() || name.contains(char::is_whitespace) {
                return Err(format!("the hook registers a bad command name `{name}`"));
            }
            if COMMANDS.iter().any(|(n, _)| *n == name) {
                return Err(format!("the hook registers `{name}`, a yavs command"));
            }
            commands.push((name, c["help"].as_str().unwrap_or_default().to_string()));
        }
        let hook = Hook {
            w: Mutex::new(w),
            commands,
            next: AtomicI64::new(1),
            gone: AtomicBool::new(false),
        };
        Ok((hook, r))
    }

    fn send(&self, m: &Json) {
        let mut w = self.w.lock().unwrap_or_else(|p| p.into_inner());
        // A closed connection is seen by the reader thread (`hook_gone`).
        let _ = writeln!(w, "{m}");
    }

    fn call(&self, name: &str, args: &str) -> Result<i64, String> {
        if self.gone.load(Ordering::Relaxed) {
            return Err(format!("`{name}`: the test runner closed its connection"));
        }
        let id = self.next.fetch_add(1, Ordering::Relaxed);
        self.send(&json!({"call": id, "name": name, "args": args}));
        Ok(id)
    }
}

/// The reader thread of the hook: `print` at once (it can come at any time), the rest at a stop.
pub(crate) fn hook_reader(mut r: BufReader<TcpStream>, link: &Link, tx: &Sender<Msg>) {
    let mut line = String::new();
    loop {
        line.clear();
        match r.read_line(&mut line) {
            Ok(n) if n > 0 => {}
            _ => break,
        }
        let Ok(m) = serde_json::from_str::<Json>(&line) else {
            continue;
        };
        match m["print"].as_str() {
            Some(t) => link.output("console", t),
            None => {
                let _ = tx.send(Msg::Hook(m));
            }
        }
    }
    let _ = tx.send(Msg::HookGone);
}

/// A runner's message at a stop.
pub(crate) fn on_hook(sh: &Shared, at: &At<'_, '_>, m: Json) -> Option<Next> {
    let link = &*sh.link;
    let id = m["exec"]
        .as_i64()
        .or_else(|| m["reload"].as_i64())
        .or_else(|| m["done"].as_i64());
    let id = id?;
    if m.get("exec").is_some() {
        let lines = split(m["line"].as_str().unwrap_or_default()).into();
        push(link, Origin::Hook(id), lines);
        return None;
    }
    if m.get("reload").is_some() {
        return reload(sh, at, id, &m);
    }
    let mut st = state(link);
    let b = st
        .batches
        .iter_mut()
        .rev()
        .find(|b| b.awaiting == Some(id))?;
    b.awaiting = None;
    b.say(link, m["result"].as_str().unwrap_or_default());
    if let Some(e) = m["error"].as_str() {
        b.error = Some(e.to_string());
        b.lines.clear();
    }
    None
}

/// `reload`: elaborate first, so a failure leaves everything as it is.
fn reload(sh: &Shared, _at: &At<'_, '_>, id: i64, m: &Json) -> Option<Next> {
    let link = &*sh.link;
    let refuse = |why: &str| {
        if let Some(h) = link.hook.get() {
            h.send(&json!({"result": id, "ok": false, "text": "", "error": why}));
        }
    };
    if let Some(why) = &link.no_restart {
        refuse(why);
        return None;
    }
    let args: Vec<String> = m["args"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| a.as_str().map(String::from))
        .collect();
    let mut guard = link.reload.lock().unwrap_or_else(|p| p.into_inner());
    let Some(f) = guard.as_mut() else {
        refuse("this session cannot reload");
        return None;
    };
    // On its own thread: this one is inside a stopped process.
    let r = std::thread::scope(|s| {
        let t = std::thread::Builder::new()
            .stack_size(1 << 30)
            .spawn_scoped(s, || f(&args));
        match t {
            Ok(t) => t
                .join()
                .unwrap_or_else(|_| Err("this is a bug in yavs: the reload panicked".into())),
            Err(e) => Err(e.to_string()),
        }
    });
    drop(guard);
    match r {
        Err(why) => {
            refuse(&why);
            None
        }
        Ok(new) => {
            *sh.restart.lock().unwrap_or_else(|p| p.into_inner()) = Some(Some(new));
            push(link, Origin::Hook(id), VecDeque::new());
            state(link).told = false;
            Some(Next::Restart)
        }
    }
}

/// The runner's connection closed: what waits for it fails.
pub(crate) fn hook_gone(sh: &Shared) {
    if let Some(h) = sh.link.hook.get() {
        h.gone.store(true, Ordering::Relaxed);
    }
    for b in &mut state(&sh.link).batches {
        if b.awaiting.take().is_some() {
            b.error = Some("the test runner closed its connection".into());
            b.lines.clear();
        }
    }
}
