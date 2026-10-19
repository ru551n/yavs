//! The window: toolbar, dock layout, console, status bar (slice G1 of docs/design/gui.md).

use eframe::egui::{self, Color32, RichText, WidgetText};
use egui_dock::{DockArea, DockState, NodeIndex, TabViewer};
use serde_json::{Value, json};

use crate::dap::{Incoming, Session, Target, Wake};

#[derive(Clone, Copy, PartialEq, Debug)]
enum Run {
    Starting,
    Running,
    Stopped { thread: i64 },
    Ended,
}

#[derive(Clone, Copy, PartialEq)]
enum Tab {
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
            Tab::Source | Tab::CallStack | Tab::Locals | Tab::Watch | Tab::Breakpoints => "G2",
            Tab::Hierarchy | Tab::Objects | Tab::Processes => "G3",
            Tab::Waves => "G4",
            Tab::Console => "G1",
        }
    }
}

struct Line {
    text: String,
    error: bool,
}

pub struct App {
    session: Option<Session>,
    attach: bool,
    run: Run,
    status: String,
    console: Vec<Line>,
    dock: DockState<Tab>,
}

impl App {
    pub fn new(target: &Target, wake: Wake) -> App {
        let mut app = App {
            session: None,
            attach: matches!(target, Target::Attach(_)),
            run: Run::Starting,
            status: "starting".into(),
            console: Vec::new(),
            dock: default_layout(),
        };
        match Session::start(target, wake) {
            Ok(s) => {
                app.session = Some(s);
                app.send(
                    "initialize",
                    json!({"adapterID": "yavs", "clientID": "yavs-gui"}),
                );
            }
            Err(e) => app.fail(e),
        }
        app
    }

    fn fail(&mut self, e: String) {
        self.console.push(Line {
            text: e.clone(),
            error: true,
        });
        self.status = e;
        self.run = Run::Ended;
    }

    fn send(&mut self, command: &str, args: Value) {
        if let Some(s) = &mut self.session
            && let Err(e) = s.request(command, args)
        {
            self.fail(format!("cannot send `{command}`: {e}"));
        }
    }

    /// Handle everything the reader threads have delivered.
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
                if m["success"] != true {
                    let msg = m["message"].as_str().unwrap_or("failed");
                    self.console.push(Line {
                        text: format!("{command}: {msg}"),
                        error: true,
                    });
                    return;
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
                Some("initialized") => self.send("configurationDone", json!({})),
                Some("output") => {
                    let text = body["output"].as_str().unwrap_or("");
                    self.console.push(Line {
                        text: text.trim_end_matches('\n').to_string(),
                        error: body["category"] == "stderr",
                    });
                }
                Some("stopped") => {
                    self.run = Run::Stopped {
                        thread: body["threadId"].as_i64().unwrap_or(1),
                    };
                    let what = body["description"].as_str().or(body["reason"].as_str());
                    self.status = format!("stopped: {}", what.unwrap_or("?"));
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

    pub fn show(&mut self, ui: &mut egui::Ui) {
        self.poll();
        egui::Panel::top("toolbar").show(ui, |ui| {
            ui.horizontal(|ui| {
                let stopped = match self.run {
                    Run::Stopped { thread } => Some(thread),
                    _ => None,
                };
                if ui
                    .add_enabled(stopped.is_some(), egui::Button::new("Run"))
                    .clicked()
                    && let Some(thread) = stopped
                {
                    self.send("continue", json!({"threadId": thread}));
                }
                if ui
                    .add_enabled(self.run == Run::Running, egui::Button::new("Pause"))
                    .clicked()
                {
                    self.send("pause", json!({"threadId": 1}));
                }
                if ui
                    .add_enabled(self.run != Run::Ended, egui::Button::new("Stop"))
                    .clicked()
                {
                    self.send("disconnect", json!({}));
                }
            });
        });
        egui::Panel::bottom("status").show(ui, |ui| {
            ui.label(&self.status);
        });
        DockArea::new(&mut self.dock)
            .show_close_buttons(false)
            .show_leaf_close_all_buttons(false)
            .show_inside(
                ui,
                &mut Viewer {
                    console: &self.console,
                },
            );
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.show(ui);
    }
}

struct Viewer<'a> {
    console: &'a [Line],
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
        if *tab != Tab::Console {
            ui.weak(format!(
                "{}: not implemented yet ({})",
                tab.title(),
                tab.slice()
            ));
            return;
        }
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for l in self.console {
                    let t = RichText::new(&l.text).monospace();
                    ui.label(if l.error {
                        t.color(Color32::LIGHT_RED)
                    } else {
                        t
                    });
                }
            });
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
