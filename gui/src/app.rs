//! The window: toolbar, dock layout, status bar, and the DAP conversation behind the debugger
//! panels (slices G1 and G2 of docs/design/gui.md). The state lives in `state.rs`, the panels in
//! `panels.rs`; this file turns DAP messages into state and user actions into requests.

use std::collections::HashMap;

use eframe::egui::{self, WidgetText};
use egui_dock::{DockArea, DockState, NodeIndex, TabViewer};
use serde_json::{Value, json};

use crate::dap::{Incoming, Session, Target, Wake};
use crate::panels::{self, Action};
use crate::state::{Breakpoints, PAGE, Stack, VarTree, parse_time};

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Run {
    Starting,
    Running,
    Stopped { thread: i64 },
    Ended,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Tab {
    Hierarchy,
    Objects,
    Source,
    Waves,
    CallStack,
    Locals,
    Watch,
    Processes,
    Console,
    Breakpoints,
}

impl Tab {
    fn title(self) -> &'static str {
        match self {
            Tab::Hierarchy => "Hierarchy",
            Tab::Objects => "Objects",
            Tab::Source => "Source",
            Tab::Waves => "Waves",
            Tab::CallStack => "Call stack",
            Tab::Locals => "Locals",
            Tab::Watch => "Watch",
            Tab::Processes => "Processes",
            Tab::Console => "Console",
            Tab::Breakpoints => "Breakpoints",
        }
    }

    /// The slice of docs/design/gui.md section 10 that fills a placeholder tab.
    fn slice(self) -> &'static str {
        match self {
            Tab::Hierarchy | Tab::Objects | Tab::Processes => "G3",
            _ => "G4",
        }
    }
}

pub struct Line {
    pub text: String,
    pub error: bool,
}

/// What a request was for; the response is handled by it. The number is the stop it belongs to:
/// answers for an earlier stop are dropped.
enum Purpose {
    Threads(u64),
    Stack(u64, i64),
    Scopes(u64),
    Vars(u64, i64, u64),
    Breakpoints(String),
    Hover(u64, String),
    Watch(u64, usize),
    SetVariable(u64),
}

/// The open "edit breakpoint" dialog.
pub struct BpEdit {
    pub path: String,
    pub line: u32,
    pub condition: String,
    pub hit: String,
}

/// A local being edited in the Locals panel.
pub struct VarEdit {
    pub parent: i64,
    pub name: String,
    pub text: String,
}

/// Everything but the dock layout, so the panels can borrow it while the dock is drawn.
pub struct Dbg {
    session: Option<Session>,
    attach: bool,
    pub run: Run,
    pub status: String,
    /// Time and delta of the last stop.
    pub time: Option<(String, u64)>,
    pub console: Vec<Line>,
    pending: HashMap<i64, Purpose>,
    stop: u64,
    pub bps: Breakpoints,
    pub stack: Stack,
    pub vars: VarTree,
    /// Source files read from disk (lines, or why not), and the open tabs.
    pub sources: HashMap<String, Result<Vec<String>, String>>,
    pub open: Vec<String>,
    pub shown: Option<String>,
    pub open_text: String,
    /// The line to scroll to on the next frame.
    pub scroll_to: Option<u32>,
    pub hover: HashMap<String, String>,
    pub watch: Vec<(String, String)>,
    pub watch_text: String,
    pub bp_edit: Option<BpEdit>,
    pub var_edit: Option<VarEdit>,
    pub run_for: String,
}

pub struct App {
    d: Dbg,
    dock: DockState<Tab>,
}

impl App {
    pub fn new(target: &Target, wake: Wake) -> App {
        let mut d = Dbg {
            session: None,
            attach: matches!(target, Target::Attach(_)),
            run: Run::Starting,
            status: "starting".into(),
            time: None,
            console: Vec::new(),
            pending: HashMap::new(),
            stop: 0,
            bps: Breakpoints::default(),
            stack: Stack::default(),
            vars: VarTree::default(),
            sources: HashMap::new(),
            open: Vec::new(),
            shown: None,
            open_text: String::new(),
            scroll_to: None,
            hover: HashMap::new(),
            watch: Vec::new(),
            watch_text: String::new(),
            bp_edit: None,
            var_edit: None,
            run_for: "100 ns".into(),
        };
        match Session::start(target, wake) {
            Ok(s) => {
                d.session = Some(s);
                d.send(
                    "initialize",
                    json!({"adapterID": "yavs", "clientID": "yavs-gui"}),
                );
            }
            Err(e) => d.fail(e),
        }
        App {
            d,
            dock: default_layout(),
        }
    }

    pub fn show(&mut self, ui: &mut egui::Ui) {
        self.d.poll();
        egui::Panel::top("toolbar").show(ui, |ui| self.d.toolbar(ui));
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(&self.d.status);
                if let Some((t, delta)) = &self.d.time {
                    ui.separator();
                    ui.label(format!("time {t}  delta {delta}"));
                }
                if let Run::Stopped { thread } = self.d.run
                    && let Some(n) = self.d.stack.name(thread)
                {
                    ui.separator();
                    ui.label(format!("process {n}"));
                }
            });
        });
        let mut actions = Vec::new();
        DockArea::new(&mut self.dock)
            .show_close_buttons(false)
            .show_leaf_close_all_buttons(false)
            .show_inside(
                ui,
                &mut Viewer {
                    d: &mut self.d,
                    actions: &mut actions,
                },
            );
        panels::bp_dialog(&mut self.d, ui.ctx(), &mut actions);
        for a in actions {
            self.d.act(a);
        }
    }

    /// Bring a tab to the front (the tests cannot click egui_dock's tab buttons).
    #[cfg(test)]
    pub fn focus(&mut self, tab: Tab) {
        if let Some(loc) = self.dock.find_tab(&tab) {
            let _ = self.dock.set_active_tab(loc);
        }
    }

    #[cfg(test)]
    pub fn dbg(&mut self) -> &mut Dbg {
        &mut self.d
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
    }
}

impl Dbg {
    fn fail(&mut self, e: String) {
        self.console.push(Line {
            text: e.clone(),
            error: true,
        });
        self.status = e;
        self.run = Run::Ended;
    }

    fn send(&mut self, command: &str, args: Value) -> Option<i64> {
        let s = self.session.as_mut()?;
        match s.request(command, args) {
            Ok(seq) => Some(seq),
            Err(e) => {
                self.fail(format!("cannot send `{command}`: {e}"));
                None
            }
        }
    }

    fn ask(&mut self, command: &str, args: Value, why: Purpose) {
        if let Some(seq) = self.send(command, args) {
            self.pending.insert(seq, why);
        }
    }

    fn poll(&mut self) {
        while let Some(m) = self.session.as_ref().and_then(|s| s.rx.try_recv().ok()) {
            match m {
                Incoming::Message(m) => self.handle(&m),
                Incoming::Stderr(text) => self.console.push(Line { text, error: true }),
                Incoming::Closed(why) => {
                    if self.run != Run::Ended {
                        self.fail(why);
                    }
                }
            }
        }
    }

    fn handle(&mut self, m: &Value) {
        let body = &m["body"];
        match m["type"].as_str() {
            Some("response") => {
                let command = m["command"].as_str().unwrap_or("");
                let why = m["request_seq"]
                    .as_i64()
                    .and_then(|s| self.pending.remove(&s));
                if m["success"] != true {
                    let msg = m["message"].as_str().unwrap_or("failed");
                    return self.failed(command, msg, why);
                }
                if let Some(why) = why {
                    return self.answered(why, body);
                }
                match command {
                    "initialize" => {
                        let start = if self.attach { "attach" } else { "launch" };
                        self.send(start, json!({}));
                    }
                    "configurationDone" if self.run == Run::Starting => {
                        self.run = Run::Running;
                        self.status = "running".into();
                    }
                    _ => {}
                }
            }
            Some("event") => match m["event"].as_str() {
                Some("initialized") => {
                    // The breakpoints go in before the run starts.
                    for path in self.bps.files.keys().cloned().collect::<Vec<_>>() {
                        self.set_breakpoints(&path);
                    }
                    self.send("configurationDone", json!({}));
                }
                Some("output") => {
                    let text = body["output"].as_str().unwrap_or("");
                    self.console.push(Line {
                        text: text.trim_end_matches('\n').to_string(),
                        error: body["category"] == "stderr",
                    });
                }
                Some("stopped") => {
                    let thread = body["threadId"].as_i64().unwrap_or(1);
                    self.run = Run::Stopped { thread };
                    let what = body["description"].as_str().or(body["reason"].as_str());
                    self.status = format!("stopped: {}", what.unwrap_or("?"));
                    self.time = body["description"].as_str().and_then(parse_time);
                    self.on_stop();
                }
                Some("continued") => {
                    self.run = Run::Running;
                    self.status = "running".into();
                }
                Some("exited") => {
                    self.status = format!("exited with code {}", body["exitCode"]);
                }
                Some("terminated") => {
                    self.run = Run::Ended;
                    self.console.push(Line {
                        text: format!("session ended: {}", self.status),
                        error: false,
                    });
                }
                _ => {}
            },
            _ => {}
        }
    }

    /// A new stop: forget the old one and ask for the processes.
    fn on_stop(&mut self) {
        self.stop += 1;
        self.stack = Stack::default();
        self.vars.clear();
        self.hover.clear();
        self.var_edit = None;
        let g = self.stop;
        self.ask("threads", json!({}), Purpose::Threads(g));
    }

    fn current(&self, n: u64) -> bool {
        n == self.stop
    }

    fn failed(&mut self, command: &str, msg: &str, why: Option<Purpose>) {
        match why {
            Some(Purpose::Hover(n, expr)) if self.current(n) => {
                self.hover.insert(expr, format!("error: {msg}"));
            }
            Some(Purpose::Watch(n, i)) if self.current(n) => {
                if let Some(w) = self.watch.get_mut(i) {
                    w.1 = format!("error: {msg}");
                }
            }
            Some(Purpose::Vars(n, r, _)) => {
                self.vars.requested.remove(&r);
                if self.current(n) {
                    self.console.push(err(format!("{command}: {msg}")));
                }
            }
            Some(Purpose::Threads(n) | Purpose::Stack(n, _) | Purpose::Scopes(n))
                if !self.current(n) => {}
            Some(Purpose::Hover(..) | Purpose::Watch(..)) => {}
            _ => {
                self.console.push(err(format!("{command}: {msg}")));
            }
        }
    }

    fn answered(&mut self, why: Purpose, body: &Value) {
        match why {
            Purpose::Threads(g) if self.current(g) => {
                self.stack.set_threads(body);
                let stopped = match self.run {
                    Run::Stopped { thread } => Some(thread),
                    _ => None,
                };
                let mut ids: Vec<i64> = self.stack.threads.iter().map(|t| t.id).collect();
                // The stopped process first: its frames select the source line.
                ids.sort_by_key(|&i| Some(i) != stopped);
                for t in ids {
                    self.ask("stackTrace", json!({"threadId": t}), Purpose::Stack(g, t));
                }
            }
            Purpose::Stack(g, t) if self.current(g) => {
                self.stack.set_frames(t, body);
                let stopped = matches!(self.run, Run::Stopped { thread } if thread == t);
                let has = self
                    .stack
                    .threads
                    .iter()
                    .find(|x| x.id == t)
                    .and_then(|x| x.frames.as_ref())
                    .is_some_and(|f| !f.is_empty());
                if stopped && has && self.stack.selected.is_none() {
                    self.select_frame(t, 0);
                }
            }
            Purpose::Scopes(g) if self.current(g) => {
                for r in self.vars.set_scopes(body) {
                    self.fetch(r, 0, false);
                }
            }
            Purpose::Vars(g, r, start) if self.current(g) => self.vars.set_children(r, start, body),
            Purpose::Breakpoints(path) => self.bps.apply(&path, body),
            Purpose::Hover(g, expr) if self.current(g) => {
                let v = body["result"].as_str().unwrap_or("").to_string();
                self.hover.insert(expr, v);
            }
            Purpose::Watch(g, i) if self.current(g) => {
                if let Some(w) = self.watch.get_mut(i) {
                    w.1 = body["result"].as_str().unwrap_or("").to_string();
                }
            }
            Purpose::SetVariable(g) if self.current(g) => self.refresh_vars(),
            _ => {}
        }
    }

    fn fetch(&mut self, reference: i64, start: u64, paged: bool) {
        if start == 0 && !self.vars.requested.insert(reference) {
            return;
        }
        let mut a = json!({"variablesReference": reference});
        if paged {
            self.vars.paged.insert(reference);
            a["start"] = json!(start);
            a["count"] = json!(PAGE);
        }
        let g = self.stop;
        self.ask("variables", a, Purpose::Vars(g, reference, start));
    }

    /// Fetch the first page of everything shown; ponytail: an expanded array goes back to its
    /// first page, a per-reference page memory if that gets in the way.
    fn refresh_vars(&mut self) {
        let refs: Vec<i64> = self.vars.children.keys().copied().collect();
        for r in refs {
            self.vars.requested.remove(&r);
            let paged = self.vars.paged.contains(&r);
            self.fetch(r, 0, paged);
        }
    }

    pub fn select_frame(&mut self, thread: i64, index: usize) {
        self.stack.selected = Some((thread, index));
        self.vars.clear();
        self.hover.clear();
        self.var_edit = None;
        let Some(f) = self.stack.frame().cloned() else {
            return;
        };
        let g = self.stop;
        self.ask("scopes", json!({"frameId": f.id}), Purpose::Scopes(g));
        for i in 0..self.watch.len() {
            self.eval_watch(i);
        }
        if let Some(p) = f.path {
            self.open_file(&p);
            self.scroll_to = Some(f.line);
        }
    }

    pub fn open_file(&mut self, path: &str) {
        if !self.sources.contains_key(path) {
            let text = std::fs::read_to_string(path)
                .map(|t| t.lines().map(|l| l.replace('\t', "    ")).collect())
                .map_err(|e| format!("cannot read `{path}`: {e}"));
            self.sources.insert(path.to_string(), text);
        }
        if !self.open.iter().any(|p| p == path) {
            self.open.push(path.to_string());
        }
        self.shown = Some(path.to_string());
    }

    fn frame_id(&self) -> Option<i64> {
        self.stack.frame().map(|f| f.id)
    }

    fn eval_watch(&mut self, i: usize) {
        let (Some(frame), Some((expr, _))) = (self.frame_id(), self.watch.get(i).cloned()) else {
            return;
        };
        let g = self.stop;
        self.ask(
            "evaluate",
            json!({"expression": expr, "frameId": frame, "context": "watch"}),
            Purpose::Watch(g, i),
        );
    }

    fn set_breakpoints(&mut self, path: &str) {
        let a = self.bps.request(path);
        self.ask("setBreakpoints", a, Purpose::Breakpoints(path.to_string()));
    }

    pub fn act(&mut self, a: Action) {
        match a {
            Action::ToggleBp(path, line) => {
                self.bps.toggle(&path, line);
                self.set_breakpoints(&path);
            }
            Action::EditBp(path, line) => {
                let b = self.bps.get(&path, line);
                self.bp_edit = Some(BpEdit {
                    condition: b.map(|b| b.condition.clone()).unwrap_or_default(),
                    hit: b.map(|b| b.hit_condition.clone()).unwrap_or_default(),
                    path,
                    line,
                });
            }
            Action::SetBp(path, line, cond, hit) => {
                self.bps.set(&path, line, &cond, &hit);
                self.set_breakpoints(&path);
            }
            Action::EnableBp(path, line, on) => {
                self.bps.set_enabled(&path, line, on);
                self.set_breakpoints(&path);
            }
            Action::Open(path) => self.open_file(&path),
            Action::SelectFrame(t, i) => self.select_frame(t, i),
            Action::Expand(r, paged) => self.fetch(r, 0, paged),
            Action::More(r, start) => self.fetch(r, start, true),
            Action::Hover(expr) => {
                let Some(frame) = self.frame_id() else { return };
                self.hover.insert(expr.clone(), "...".into());
                let g = self.stop;
                self.ask(
                    "evaluate",
                    json!({"expression": expr, "frameId": frame, "context": "hover"}),
                    Purpose::Hover(g, expr),
                );
            }
            Action::AddWatch(expr) => {
                self.watch.push((expr, String::new()));
                self.eval_watch(self.watch.len() - 1);
            }
            Action::RemoveWatch(i) => {
                self.watch.remove(i);
                for i in 0..self.watch.len() {
                    self.eval_watch(i);
                }
            }
            Action::SetVar(parent, name, value) => {
                let g = self.stop;
                self.ask(
                    "setVariable",
                    json!({"variablesReference": parent, "name": name, "value": value}),
                    Purpose::SetVariable(g),
                );
            }
        }
    }

    fn toolbar(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let stopped = match self.run {
                Run::Stopped { thread } => Some(thread),
                _ => None,
            };
            let go = |ui: &mut egui::Ui, label: &str, enabled: bool| {
                ui.add_enabled(enabled, egui::Button::new(label)).clicked()
            };
            let t = stopped.unwrap_or(1);
            let a = json!({"threadId": t});
            if go(ui, "Run", stopped.is_some()) {
                self.send("continue", a.clone());
            }
            if go(ui, "Pause", self.run == Run::Running) {
                self.send("pause", json!({"threadId": 1}));
            }
            if go(ui, "Stop", self.run != Run::Ended) {
                self.send("disconnect", json!({}));
            }
            ui.separator();
            for (label, cmd) in [
                ("Over", "next"),
                ("In", "stepIn"),
                ("Out", "stepOut"),
                ("Delta", "yavs/stepDelta"),
                ("Timestep", "yavs/stepTimestep"),
            ] {
                if go(ui, label, stopped.is_some()) {
                    self.send(cmd, a.clone());
                }
            }
            ui.separator();
            ui.add(egui::TextEdit::singleline(&mut self.run_for).desired_width(70.0));
            if go(ui, "Run for", stopped.is_some()) {
                let time = self.run_for.trim().to_string();
                self.send("yavs/runFor", json!({"threadId": t, "time": time}));
            }
        });
    }
}

fn err(text: String) -> Line {
    Line { text, error: true }
}

struct Viewer<'a> {
    d: &'a mut Dbg,
    actions: &'a mut Vec<Action>,
}

impl TabViewer for Viewer<'_> {
    type Tab = Tab;

    fn id(&mut self, tab: &mut Tab) -> egui::Id {
        egui::Id::new(tab.title())
    }

    fn title(&mut self, tab: &mut Tab) -> WidgetText {
        tab.title().into()
    }

    fn ui(&mut self, ui: &mut egui::Ui, tab: &mut Tab) {
        let (d, a) = (&mut *self.d, &mut *self.actions);
        match tab {
            Tab::Console => panels::console(d, ui),
            Tab::Source => panels::source(d, ui, a),
            Tab::CallStack => panels::call_stack(d, ui, a),
            Tab::Locals => panels::locals(d, ui, a),
            Tab::Watch => panels::watch(d, ui, a),
            Tab::Breakpoints => panels::breakpoints(d, ui, a),
            _ => {
                ui.weak(format!(
                    "{}: not implemented yet ({})",
                    tab.title(),
                    tab.slice()
                ));
            }
        }
    }

    fn is_closeable(&self, _tab: &Tab) -> bool {
        false
    }
}

/// The default arrangement of docs/design/gui.md section 6.
fn default_layout() -> DockState<Tab> {
    let mut dock = DockState::new(vec![Tab::Source]);
    let tree = dock.main_surface_mut();
    let [center, _] = tree.split_below(
        NodeIndex::root(),
        0.75,
        vec![Tab::Console, Tab::Breakpoints],
    );
    let [center, left] = tree.split_left(center, 0.2, vec![Tab::Hierarchy]);
    tree.split_below(left, 0.5, vec![Tab::Objects]);
    let [center, right] = tree.split_right(center, 0.75, vec![Tab::CallStack]);
    let [_, locals] = tree.split_below(right, 0.3, vec![Tab::Locals, Tab::Watch]);
    tree.split_below(locals, 0.55, vec![Tab::Processes]);
    tree.split_below(center, 0.55, vec![Tab::Waves]);
    dock
}
