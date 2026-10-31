//! `run --surfer[=ADDR]`: shows the `--wave` file in the Surfer waveform viewer while the run
//! writes it, through Surfer's Waveform Control Protocol (WCP, #876; `docs/cli-contract.md` 5).
//! JSON messages ended by a NUL byte over TCP; the client greets first.

use std::collections::HashMap;
use std::io::{self, BufRead, BufReader, ErrorKind, Write};
use std::net::{TcpListener, TcpStream, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde_json::{Value as Json, json};
use yavs_diag::Diagnostic;
use yavs_wave::{HierItem, Hierarchy, ScopeKind};

/// How long Surfer may take to start, connect, answer or load a file.
const WAIT: Duration = Duration::from_secs(30);

/// At most this many signals are added on their own (`wave add` adds what it is told).
const MAX_SHOWN: usize = 200;

/// What the run tells the viewer thread.
pub enum Msg {
    /// A debugger stop: the file was just synced.
    Stop(yavs_dap::ViewStop),
    /// A console's `wave add`: the items, and where the answer goes.
    Add(Vec<String>, Sender<Result<usize, String>>),
    /// A console's `wave cursor`, in fs.
    Cursor(u64),
    /// The run starts over and the file with it: its size with no values yet.
    Restart(Option<u64>),
}

/// A connected Surfer, driven by its own thread so the simulation never waits for it.
pub struct Surfer {
    tx: Mutex<Option<Sender<Msg>>>,
    thread: Mutex<Option<JoinHandle<io::Result<()>>>>,
    /// Where Surfer's "go to declaration" goes (`yavs_dap::Viewer::on_goto`).
    goto: Arc<OnceLock<yavs_dap::GotoFn>>,
    file: PathBuf,
}

impl Surfer {
    /// Connects to the Surfer at `addr`, or starts `surfer` from PATH when `None`, loads `file`
    /// and adds `names`; then reloads every `every` while the file grows. An error is E0130.
    pub fn start(
        addr: Option<&str>,
        file: &Path,
        names: Vec<String>,
        every: Option<Duration>,
    ) -> Result<Arc<Self>, Box<Diagnostic>> {
        let fail = |m: &str, note: String, help: &str| {
            Box::new(
                Diagnostic::error("E0130", m.to_string())
                    .with_note(note)
                    .with_help(help.to_string()),
            )
        };
        let listen_help = "start Surfer 0.6.0 or later, run its command `wcp_server_start` (or set \
                           `[wcp] autostart = true` in its config) and pass its address, by default \
                           `--surfer=127.0.0.1:54321`";
        let stream = match addr {
            Some(a) => connect(a).map_err(|e| {
                fail(
                    &format!("cannot connect to Surfer at `{a}`"),
                    e.to_string(),
                    listen_help,
                )
            })?,
            None => spawn().map_err(|e| {
                fail(
                    "cannot start Surfer",
                    e.to_string(),
                    "install Surfer 0.6.0 or later so that `surfer` is on PATH, or start it \
                     yourself and pass `--surfer=HOST:PORT`",
                )
            })?,
        };
        let file = std::path::absolute(file).unwrap_or_else(|_| file.to_path_buf());
        let goto: Arc<OnceLock<yavs_dap::GotoFn>> = Arc::default();
        let mut w = Wcp::open(stream, goto.clone())
            .and_then(|mut w| {
                w.command("load", json!({"source": file}))?;
                w.wait_loaded(1)?;
                Ok(w)
            })
            .map_err(|e| {
                fail(
                    "Surfer did not take the waveform file",
                    e.to_string(),
                    "use Surfer 0.6.0 or later; its own log says more",
                )
            })?;
        let (tx, rx) = channel();
        // Before the run: the thread comes to it when the run may have written.
        let empty = std::fs::metadata(&file).map(|m| m.len()).ok();
        let path = file.clone();
        let thread = std::thread::spawn(move || drive(&mut w, &rx, &path, every, names, empty));
        Ok(Arc::new(Self {
            tx: Mutex::new(Some(tx)),
            thread: Mutex::new(Some(thread)),
            goto,
            file,
        }))
    }

    fn send(&self, m: Msg) {
        if let Some(tx) = &*self.tx.lock().unwrap_or_else(|p| p.into_inner()) {
            // The thread has ended: `end` says why.
            let _ = tx.send(m);
        }
    }

    fn size(&self) -> Option<u64> {
        std::fs::metadata(&self.file).map(|m| m.len()).ok()
    }

    /// The run starts over: the file is new.
    pub fn restart(&self) {
        self.send(Msg::Restart(self.size()));
    }

    /// After the file is finished: reload it and show the whole run. A failure at any time of
    /// the run is warning W0131; the run's result never depends on the viewer.
    pub fn end(&self) -> Option<Diagnostic> {
        self.tx.lock().unwrap_or_else(|p| p.into_inner()).take();
        let t = self
            .thread
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .take()?;
        let err = match t.join() {
            Ok(Ok(())) => return None,
            Ok(Err(e)) => e.to_string(),
            Err(_) => "the viewer thread panicked".to_string(),
        };
        Some(
            Diagnostic::warning("W0131", "lost the connection to Surfer")
                .with_note(err)
                .with_note(
                    "the simulation was not affected; Surfer shows the run up to the failure",
                ),
        )
    }
}

impl yavs_dap::Viewer for Surfer {
    fn stop(&self, s: yavs_dap::ViewStop) {
        self.send(Msg::Stop(s));
    }

    fn add(&self, items: Vec<String>) -> Result<usize, String> {
        let (tx, rx) = channel();
        self.send(Msg::Add(items, tx));
        rx.recv_timeout(WAIT + WAIT)
            .map_err(|_| "Surfer does not answer".to_string())?
    }

    fn cursor(&self, fs: u64) {
        self.send(Msg::Cursor(fs));
    }

    fn on_goto(&self, f: yavs_dap::GotoFn) {
        let _ = self.goto.set(f);
    }
}

/// The signals to show on their own: those in the top scope, with the records and arrays
/// directly in it (scopes in the file) one level down, or every recorded one when `all` (the
/// `--wave-include` globs chose them); at most [`MAX_SHOWN`] and how many were left out. Names as
/// Surfer reads them from the file.
pub fn names(h: &Hierarchy, all: bool) -> (Vec<String>, usize) {
    let (mut path, mut out) = (Vec::new(), Vec::new());
    // Below the top scope, whether each open scope is a composite of the object.
    let mut composite = Vec::new();
    for it in h.items() {
        match it {
            HierItem::Scope { name, kind, .. } => {
                path.push(name);
                composite.push(matches!(kind, ScopeKind::Record | ScopeKind::Array));
            }
            HierItem::Up => {
                path.pop();
                composite.pop();
            }
            HierItem::Signal { name, .. }
                if all
                    || path.len() == 1
                    || (path.len() == 2 && composite.last() == Some(&true)) =>
            {
                // Vector bounds in the name (`data[7:0]`) are not part of the viewer's name.
                let name = match name.rfind('[') {
                    Some(i) if name.ends_with(']') => &name[..i],
                    _ => name,
                };
                out.push(format!("{}.{name}", path.join(".")));
            }
            HierItem::Signal { .. } => {}
        }
    }
    let over = out.len().saturating_sub(MAX_SHOWN);
    out.truncate(MAX_SHOWN);
    (out, over)
}

fn connect(addr: &str) -> io::Result<TcpStream> {
    let mut last = io::Error::new(ErrorKind::InvalidInput, "the address names no host");
    for a in addr.to_socket_addrs()? {
        match TcpStream::connect_timeout(&a, Duration::from_secs(5)) {
            Ok(s) => return Ok(s),
            Err(e) => last = e,
        }
    }
    Err(last)
}

/// Starts `surfer --wcp-initiate PORT`, which connects back to a port yavs listens on.
fn spawn() -> io::Result<TcpStream> {
    let l = TcpListener::bind(("127.0.0.1", 0))?;
    let port = l.local_addr()?.port();
    let mut child = Command::new("surfer")
        .args(["--wcp-initiate", &port.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| match e.kind() {
            ErrorKind::NotFound => io::Error::new(e.kind(), "`surfer` is not on PATH"),
            _ => io::Error::new(e.kind(), format!("`surfer`: {e}")),
        })?;
    l.set_nonblocking(true)?;
    let t0 = Instant::now();
    loop {
        match l.accept() {
            Ok((s, _)) => {
                s.set_nonblocking(false)?;
                return Ok(s);
            }
            Err(e) if e.kind() == ErrorKind::WouldBlock => {}
            Err(e) => return Err(e),
        }
        if let Some(st) = child.try_wait()? {
            return Err(io::Error::other(format!(
                "`surfer` ended ({st}) before it connected"
            )));
        }
        if t0.elapsed() > WAIT {
            return Err(io::Error::new(
                ErrorKind::TimedOut,
                "`surfer` did not connect within 30 s",
            ));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// One WCP connection: messages read by a thread of their own, `waveforms_loaded` counted, and
/// the displayed items by name.
struct Wcp {
    w: TcpStream,
    rx: Receiver<Json>,
    loaded: u64,
    shown: HashMap<String, Json>,
    /// The items coloured for the last stop.
    coloured: Vec<Json>,
    goto: Arc<OnceLock<yavs_dap::GotoFn>>,
}

/// The colour of the stopped process's signals, and of the others after it.
const STOPPED_COLOUR: &str = "Yellow";
// ponytail: WCP cannot give an item its default colour back; this is the default theme's.
const PLAIN_COLOUR: &str = "Green";

impl Wcp {
    fn open(w: TcpStream, goto: Arc<OnceLock<yavs_dap::GotoFn>>) -> io::Result<Self> {
        w.set_write_timeout(Some(WAIT))?;
        let r = w.try_clone()?;
        let (tx, rx) = channel();
        std::thread::spawn(move || {
            let mut r = BufReader::new(r);
            loop {
                let mut b = Vec::new();
                match r.read_until(0, &mut b) {
                    Ok(_) if b.pop() == Some(0) => {}
                    _ => break,
                }
                let Ok(m) = serde_json::from_slice(&b) else {
                    break;
                };
                if tx.send(m).is_err() {
                    break;
                }
            }
        });
        let mut s = Self {
            w,
            rx,
            loaded: 0,
            shown: HashMap::new(),
            coloured: Vec::new(),
            goto,
        };
        s.send(json!({
            "type": "greeting",
            "version": "0",
            "commands": ["waveforms_loaded", "goto_declaration"],
        }))?;
        let g = s.next()?;
        if g["type"] != "greeting" {
            return Err(io::Error::other(format!(
                "Surfer answered the greeting with {g}"
            )));
        }
        Ok(s)
    }

    fn send(&mut self, m: Json) -> io::Result<()> {
        let mut b = m.to_string().into_bytes();
        b.push(0);
        self.w.write_all(&b)
    }

    fn next(&mut self) -> io::Result<Json> {
        let m = self.rx.recv_timeout(WAIT).map_err(|e| match e {
            RecvTimeoutError::Timeout => {
                io::Error::new(ErrorKind::TimedOut, "Surfer did not answer within 30 s")
            }
            RecvTimeoutError::Disconnected => {
                io::Error::new(ErrorKind::ConnectionAborted, "Surfer closed the connection")
            }
        })?;
        self.on_event(&m);
        Ok(m)
    }

    fn on_event(&mut self, m: &Json) {
        if m["type"] != "event" {
            return;
        }
        if m["event"] == "waveforms_loaded" {
            self.loaded += 1;
        }
        if m["event"] == "goto_declaration"
            && let (Some(f), Some(v)) = (self.goto.get(), m["variable"].as_str())
        {
            f(v);
        }
    }

    /// Add items to the view, remembering their ids; how many were added.
    fn add(&mut self, items: &[String]) -> io::Result<usize> {
        if items.is_empty() {
            return Ok(0);
        }
        let r = self.command("add_items", json!({"items": items}))?;
        let ids = r["ids"].as_array().cloned().unwrap_or_default();
        // A scope adds several ids for one item: only one-to-one names are known by name.
        if ids.len() == items.len() {
            let names = items.iter().map(|n| n.to_ascii_lowercase());
            self.shown.extend(names.zip(ids.iter().cloned()));
        }
        Ok(ids.len())
    }

    /// The stopped process's signals in colour, the last stop's back.
    fn colour(&mut self, signals: &[String]) -> io::Result<()> {
        for id in std::mem::take(&mut self.coloured) {
            self.command("set_item_color", json!({"id": id, "color": PLAIN_COLOUR}))?;
        }
        for name in signals {
            if let Some(id) = self.shown.get(name).cloned() {
                self.command("set_item_color", json!({"id": id, "color": STOPPED_COLOUR}))?;
                self.coloured.push(id);
            }
        }
        Ok(())
    }

    /// Sends a command and waits for its response.
    fn command(&mut self, cmd: &str, mut m: Json) -> io::Result<Json> {
        m["type"] = json!("command");
        m["command"] = json!(cmd);
        self.send(m)?;
        loop {
            let r = self.next()?;
            match r["type"].as_str() {
                Some("response") => return Ok(r),
                Some("error") => {
                    return Err(io::Error::other(format!(
                        "Surfer refused `{cmd}`: {}",
                        r["message"].as_str().unwrap_or_default()
                    )));
                }
                _ => {}
            }
        }
    }

    /// Waits until `n` loads (the first and the reloads) are complete.
    fn wait_loaded(&mut self, n: u64) -> io::Result<()> {
        while self.loaded < n {
            self.next()?;
        }
        Ok(())
    }

    /// Counts the events that arrived, without waiting.
    fn poll(&mut self) {
        while let Ok(m) = self.rx.try_recv() {
            self.on_event(&m);
        }
    }
}

/// What the viewer thread keeps.
struct View<'a> {
    file: &'a PathBuf,
    /// The file's size when it held no values yet: Surfer panics on reading the signals of a file
    /// with an empty time table, so nothing reloads or adds to the view before it has grown.
    empty: Option<u64>,
    seen: Option<u64>,
    /// `reload`s and the `load`, for `Wcp::wait_loaded`.
    asked: u64,
    /// What to add once the file has values.
    waiting: Vec<String>,
}

impl View<'_> {
    fn len(&self) -> Option<u64> {
        std::fs::metadata(self.file).map(|m| m.len()).ok()
    }

    fn has_values(&self) -> bool {
        self.len() != self.empty
    }

    /// Reload, after the one under way (two at once make Surfer read the body twice).
    fn reload(&mut self, w: &mut Wcp) -> io::Result<()> {
        w.wait_loaded(self.asked)?;
        self.seen = self.len();
        self.asked += 1;
        w.command("reload", json!({}))?;
        self.show_waiting(w)
    }

    fn show_waiting(&mut self, w: &mut Wcp) -> io::Result<()> {
        if self.waiting.is_empty() || !self.has_values() {
            return Ok(());
        }
        w.wait_loaded(self.asked)?;
        w.add(&std::mem::take(&mut self.waiting))?;
        Ok(())
    }
}

/// The viewer thread: a reload when the file grew (at most one under way), the cursor, markers
/// and colours at debugger stops (those queued count as one), and when the run is over (the sender
/// dropped) the whole run in view.
fn drive(
    w: &mut Wcp,
    rx: &Receiver<Msg>,
    file: &PathBuf,
    every: Option<Duration>,
    names: Vec<String>,
    empty: Option<u64>,
) -> io::Result<()> {
    let mut v = View {
        file,
        empty,
        seen: empty,
        asked: 1,
        waiting: names,
    };
    let mut marks = 0;
    let mut next = None;
    loop {
        // Events (go to declaration) wait no longer than this for a message.
        let wait = every.unwrap_or(Duration::from_secs(1));
        let m = match next.take() {
            Some(m) => Ok(m),
            None => rx.recv_timeout(wait),
        };
        w.poll();
        match m {
            Err(RecvTimeoutError::Timeout) => {
                // ponytail: a reload may meet a block being written; the next one shows it.
                if every.is_some() && v.len() != v.seen && v.has_values() && w.loaded >= v.asked {
                    v.reload(w)?;
                }
            }
            Ok(Msg::Stop(s)) => {
                let (mut last, mut times) = (s, Vec::new());
                loop {
                    if let (true, Some(t)) = (last.breakpoint, last.now_fs) {
                        times.push(t);
                    }
                    match rx.try_recv() {
                        Ok(Msg::Stop(s)) => last = s,
                        Ok(other) => {
                            next = Some(other);
                            break;
                        }
                        Err(_) => break,
                    }
                }
                if v.has_values() {
                    v.reload(w)?;
                    w.wait_loaded(v.asked)?;
                    for t in times {
                        marks += 1;
                        let name = format!("bp{marks}");
                        let marker = json!({"time": t, "name": name, "move_focus": false});
                        w.command("add_markers", json!({"markers": [marker]}))?;
                    }
                    w.colour(&last.signals)?;
                }
                if let Some(t) = last.now_fs {
                    w.command("set_cursor", json!({"timestamp": t}))?;
                    w.command("set_viewport_to", json!({"timestamp": t}))?;
                }
            }
            Ok(Msg::Add(items, answer)) => {
                let r = if v.has_values() {
                    w.wait_loaded(v.asked)?;
                    w.add(&items).map_err(|e| e.to_string())
                } else {
                    // No values yet: they are shown when there are.
                    let n = items.len();
                    v.waiting.extend(items);
                    Ok(n)
                };
                let _ = answer.send(r);
            }
            Ok(Msg::Cursor(t)) => {
                w.command("set_cursor", json!({"timestamp": t}))?;
                w.command("set_viewport_to", json!({"timestamp": t}))?;
            }
            Ok(Msg::Restart(empty)) => {
                // The new file is empty: what is shown stays, and reloads when it has values.
                v.empty = empty;
                v.seen = empty;
            }
            Err(RecvTimeoutError::Disconnected) => {
                if v.has_values() {
                    v.reload(w)?;
                    w.wait_loaded(v.asked)?;
                }
                w.command("zoom_to_fit", json!({"viewport_idx": 0}))?;
                return Ok(());
            }
        }
    }
}
