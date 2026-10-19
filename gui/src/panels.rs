//! The debugger panels of slice G2: console, source with breakpoint gutter, call stack, locals,
//! watch, breakpoints. They read `Dbg` and return `Action`s; `Dbg::act` turns those into requests.

use eframe::egui::{self, Color32, RichText, Sense, text::LayoutJob};

use crate::app::{Dbg, Run, VarEdit};
use crate::state::{Kind, Var, VarTree, highlight, word_at};

pub enum Action {
    ToggleBp(String, u32),
    EditBp(String, u32),
    SetBp(String, u32, String, String),
    EnableBp(String, u32, bool),
    Open(String),
    SelectFrame(i64, usize),
    /// A variables reference to fetch; whether it is an array (paged).
    Expand(i64, bool),
    More(i64, u64),
    Hover(String),
    AddWatch(String),
    RemoveWatch(usize),
    SetVar(i64, String, String),
}

pub fn console(d: &Dbg, ui: &mut egui::Ui) {
    egui::ScrollArea::vertical()
        .auto_shrink(false)
        .stick_to_bottom(true)
        .show(ui, |ui| {
            for l in &d.console {
                let t = RichText::new(&l.text).monospace();
                ui.label(if l.error {
                    t.color(Color32::LIGHT_RED)
                } else {
                    t
                });
            }
        });
}

fn file_name(path: &str) -> &str {
    path.rsplit(['/', '\\']).next().unwrap_or(path)
}

/// The gutter text of a line: current-line arrow, else breakpoint (filled, hollow when
/// disabled, `?` when the server refused it), then the line number.
pub fn gutter(current: bool, bp: Option<(bool, Option<bool>)>, line: u32) -> String {
    let m = match (current, bp) {
        (true, _) => "▶",
        (_, Some((_, Some(false)))) => "?",
        (_, Some((true, _))) => "●",
        (_, Some((false, _))) => "○",
        _ => " ",
    };
    format!("{m} {line:>4}")
}

fn colour(k: Kind, ui: &egui::Ui) -> Color32 {
    match k {
        Kind::Plain => ui.visuals().text_color(),
        Kind::Keyword => Color32::from_rgb(70, 130, 200),
        Kind::Comment => Color32::from_rgb(100, 150, 90),
        Kind::Text => Color32::from_rgb(200, 120, 60),
    }
}

pub fn source(d: &mut Dbg, ui: &mut egui::Ui, acts: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        for p in d.open.clone() {
            let on = d.shown.as_deref() == Some(&p);
            if ui.selectable_label(on, file_name(&p)).clicked() {
                d.shown = Some(p);
            }
        }
        ui.separator();
        let r = ui.add(
            egui::TextEdit::singleline(&mut d.open_text)
                .hint_text("path of a source file")
                .desired_width(180.0),
        );
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (ui.button("Open").clicked() || enter) && !d.open_text.trim().is_empty() {
            acts.push(Action::Open(d.open_text.trim().to_string()));
            d.open_text.clear();
        }
    });
    let Some(path) = d.shown.clone() else {
        ui.weak("No source file open: it opens at a stop, or enter a path above.");
        return;
    };
    let lines = match d.sources.get(&path) {
        Some(Ok(l)) => l,
        Some(Err(e)) => {
            ui.colored_label(Color32::LIGHT_RED, e);
            return;
        }
        None => return,
    };
    let stopped = matches!(d.run, Run::Stopped { .. });
    let current = if stopped {
        d.stack.current_line(&path)
    } else {
        None
    };
    let font = egui::TextStyle::Monospace.resolve(ui.style());
    let row_h = ui.fonts_mut(|f| f.row_height(&font)) + ui.spacing().item_spacing.y;
    let char_w = ui
        .painter()
        .layout_no_wrap("0".into(), font.clone(), Color32::WHITE)
        .size()
        .x;
    let mut area = egui::ScrollArea::vertical().auto_shrink(false);
    if let Some(l) = d.scroll_to.take() {
        area = area.vertical_scroll_offset((l.saturating_sub(6)) as f32 * row_h);
    }
    area.show_rows(ui, row_h, lines.len(), |ui, range| {
        for i in range {
            let n = i as u32 + 1;
            let bp = d.bps.get(&path, n).map(|b| (b.enabled, b.verified));
            let is_current = current == Some(n);
            let row = ui.horizontal(|ui| {
                let text = RichText::new(gutter(is_current, bp, n))
                    .font(font.clone())
                    .color(if bp.is_some() {
                        Color32::from_rgb(220, 60, 60)
                    } else {
                        ui.visuals().weak_text_color()
                    });
                let g = ui.add(egui::Label::new(text).sense(Sense::click()));
                if g.clicked() {
                    acts.push(Action::ToggleBp(path.clone(), n));
                }
                g.context_menu(|ui| {
                    if ui.button("Edit breakpoint...").clicked() {
                        acts.push(Action::EditBp(path.clone(), n));
                        ui.close();
                    }
                    if let Some((on, _)) = bp
                        && ui.button(if on { "Disable" } else { "Enable" }).clicked()
                    {
                        acts.push(Action::EnableBp(path.clone(), n, !on));
                        ui.close();
                    }
                });
                if let Some(b) = d.bps.get(&path, n)
                    && !b.message.is_empty()
                {
                    g.on_hover_text(&b.message);
                }
                let mut job = LayoutJob::default();
                for (k, s) in highlight(&lines[i]) {
                    job.append(
                        s,
                        0.0,
                        egui::TextFormat::simple(font.clone(), colour(k, ui)),
                    );
                }
                if lines[i].is_empty() {
                    job.append(
                        " ",
                        0.0,
                        egui::TextFormat::simple(font.clone(), Color32::GRAY),
                    );
                }
                let code = ui.add(egui::Label::new(job).sense(Sense::hover()));
                if stopped
                    && let Some(pos) = code.hover_pos()
                    && let col = ((pos.x - code.rect.left()) / char_w) as usize
                    && let Some(w) = word_at(&lines[i], col)
                {
                    match d.hover.get(w) {
                        None => acts.push(Action::Hover(w.to_string())),
                        Some(v) => {
                            code.on_hover_text(format!("{w} = {v}"));
                        }
                    }
                }
            });
            if is_current {
                ui.painter().rect_filled(
                    row.response.rect,
                    0.0,
                    Color32::from_rgba_unmultiplied(255, 210, 0, 45),
                );
            }
        }
    });
}

pub fn call_stack(d: &mut Dbg, ui: &mut egui::Ui, acts: &mut Vec<Action>) {
    if d.stack.threads.is_empty() {
        ui.weak("Nothing is stopped.");
        return;
    }
    ui.add_enabled_ui(d.run != Run::Running, |ui| {
        let stopped = match d.run {
            Run::Stopped { thread } => thread,
            _ => 0,
        };
        for t in &d.stack.threads {
            let open = t.id == stopped;
            egui::CollapsingHeader::new(&t.name)
                .id_salt(t.id)
                .default_open(open)
                .show(ui, |ui| match &t.frames {
                    None => {
                        ui.weak("...");
                    }
                    Some(f) if f.is_empty() => {
                        ui.weak("not running");
                    }
                    Some(frames) => {
                        for (i, f) in frames.iter().enumerate() {
                            let file = f.path.as_deref().map_or("?", file_name);
                            let text = format!("{}  {}:{}", f.name, file, f.line);
                            let on = d.stack.selected == Some((t.id, i));
                            if ui.selectable_label(on, text).clicked() {
                                acts.push(Action::SelectFrame(t.id, i));
                            }
                        }
                    }
                });
        }
    });
}

pub fn locals(d: &mut Dbg, ui: &mut egui::Ui, acts: &mut Vec<Action>) {
    if d.vars.scopes.is_empty() {
        ui.weak("Nothing is stopped.");
        return;
    }
    let running = d.run == Run::Running;
    ui.add_enabled_ui(!running, |ui| {
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                for (name, r) in d.vars.scopes.clone() {
                    egui::CollapsingHeader::new(&name)
                        .id_salt(("scope", r))
                        .default_open(name == "Locals")
                        .show(ui, |ui| var_rows(&d.vars, &mut d.var_edit, r, ui, acts));
                }
            });
    });
}

fn var_rows(
    tree: &VarTree,
    edit: &mut Option<VarEdit>,
    parent: i64,
    ui: &mut egui::Ui,
    acts: &mut Vec<Action>,
) {
    let Some(vars) = tree.children.get(&parent) else {
        ui.weak("...");
        return;
    };
    for v in vars {
        if v.reference == 0 {
            leaf(v, parent, edit, ui, acts);
            continue;
        }
        let paged = v.indexed > 0;
        egui::CollapsingHeader::new(format!("{} = {}", v.name, v.value))
            .id_salt(("var", v.reference))
            .show(ui, |ui| {
                if !tree.children.contains_key(&v.reference) {
                    if !tree.requested.contains(&v.reference) {
                        acts.push(Action::Expand(v.reference, paged));
                    }
                    ui.weak("...");
                    return;
                }
                var_rows(tree, edit, v.reference, ui, acts);
                if let Some((r, start)) = tree.more(v)
                    && ui
                        .button(format!("more ({} of {})", start, v.indexed))
                        .clicked()
                {
                    acts.push(Action::More(r, start));
                }
            });
    }
}

fn leaf(
    v: &Var,
    parent: i64,
    edit: &mut Option<VarEdit>,
    ui: &mut egui::Ui,
    acts: &mut Vec<Action>,
) {
    if let Some(e) = edit
        && e.parent == parent
        && e.name == v.name
    {
        let r = ui.text_edit_singleline(&mut e.text);
        r.request_focus();
        if ui.input(|i| i.key_pressed(egui::Key::Escape)) {
            *edit = None;
        } else if ui.input(|i| i.key_pressed(egui::Key::Enter)) {
            acts.push(Action::SetVar(parent, v.name.clone(), e.text.clone()));
            *edit = None;
        }
        return;
    }
    let r = ui.add(egui::Label::new(format!("{} = {}", v.name, v.value)).sense(Sense::click()));
    let r = if v.ty.is_empty() {
        r
    } else {
        r.on_hover_text(&v.ty)
    };
    if r.double_clicked() {
        *edit = Some(VarEdit {
            parent,
            name: v.name.clone(),
            text: v.value.clone(),
        });
    }
}

pub fn watch(d: &mut Dbg, ui: &mut egui::Ui, acts: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        let r = ui.add(
            egui::TextEdit::singleline(&mut d.watch_text)
                .hint_text("expression")
                .desired_width(160.0),
        );
        let enter = r.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
        if (ui.button("Add watch").clicked() || enter) && !d.watch_text.trim().is_empty() {
            acts.push(Action::AddWatch(d.watch_text.trim().to_string()));
            d.watch_text.clear();
        }
    });
    ui.add_enabled_ui(d.run != Run::Running, |ui| {
        for (i, (expr, value)) in d.watch.iter().enumerate() {
            ui.horizontal(|ui| {
                if ui.small_button("x").on_hover_text("Remove").clicked() {
                    acts.push(Action::RemoveWatch(i));
                }
                ui.label(format!("{expr} = {value}"));
            });
        }
    });
}

pub fn breakpoints(d: &mut Dbg, ui: &mut egui::Ui, acts: &mut Vec<Action>) {
    if d.bps.files.is_empty() {
        ui.weak("No breakpoints: click the gutter of a source line.");
        return;
    }
    let files = d.bps.files.clone();
    for (p, bps) in &files {
        for b in bps {
            ui.horizontal(|ui| {
                let mut on = b.enabled;
                if ui
                    .checkbox(&mut on, format!("{}:{}", file_name(p), b.line))
                    .changed()
                {
                    acts.push(Action::EnableBp(p.clone(), b.line, on));
                }
                if !b.condition.is_empty() {
                    ui.label(format!("if {}", b.condition));
                }
                if !b.hit_condition.is_empty() {
                    ui.label(format!("hit {}", b.hit_condition));
                }
                if b.verified == Some(false) {
                    ui.colored_label(Color32::LIGHT_RED, &b.message);
                }
                if ui.small_button("Edit").clicked() {
                    acts.push(Action::EditBp(p.clone(), b.line));
                }
                if ui.small_button("Remove").clicked() {
                    acts.push(Action::ToggleBp(p.clone(), b.line));
                }
            });
        }
    }
}

/// The condition and hit count dialog of a breakpoint.
pub fn bp_dialog(d: &mut Dbg, ctx: &egui::Context, acts: &mut Vec<Action>) {
    let Some(e) = &mut d.bp_edit else { return };
    let mut done = None;
    egui::Window::new("Edit breakpoint")
        .collapsible(false)
        .show(ctx, |ui| {
            ui.label(format!("{}:{}", file_name(&e.path), e.line));
            ui.horizontal(|ui| {
                ui.label("Condition");
                ui.text_edit_singleline(&mut e.condition);
            });
            ui.horizontal(|ui| {
                ui.label("Hit count");
                ui.text_edit_singleline(&mut e.hit);
            });
            ui.horizontal(|ui| {
                if ui.button("OK").clicked() {
                    done = Some(true);
                }
                if ui.button("Cancel").clicked() {
                    done = Some(false);
                }
            });
        });
    match done {
        Some(true) => {
            acts.push(Action::SetBp(
                e.path.clone(),
                e.line,
                e.condition.clone(),
                e.hit.clone(),
            ));
            d.bp_edit = None;
        }
        Some(false) => d.bp_edit = None,
        None => {}
    }
}
