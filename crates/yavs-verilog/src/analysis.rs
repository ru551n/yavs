//! Module analysis: ports of a module (ANSI and non-ANSI), duplicate declarations, undeclared
//! identifiers under `` `default_nettype none ``.

use std::collections::{HashMap, HashSet};

use crate::ast::*;
use crate::pre::{Nettype, Pre};
use crate::{Diag, Dir, Port, Span};

/// Source-like text of an expression, for ranges in messages and port records.
pub(crate) fn show(e: &Expr) -> String {
    match e {
        Expr::Empty => String::new(),
        Expr::Num(s) | Expr::Real(s) => s.clone(),
        Expr::Str(s) => format!("\"{s}\""),
        Expr::Ident(i) => i.name.clone(),
        Expr::Dot(b, i) => format!("{}.{}", show(b), i.name),
        Expr::Index(b, i) => format!("{}[{}]", show(b), show(i)),
        Expr::Range(b, k, x, y) => {
            let op = match k {
                RangeKind::Colon => ":",
                RangeKind::PlusColon => "+:",
                RangeKind::MinusColon => "-:",
            };
            format!("{}[{}{}{}]", show(b), show(x), op, show(y))
        }
        Expr::Concat(v) => format!("{{{}}}", v.iter().map(show).collect::<Vec<_>>().join(", ")),
        Expr::Repl(n, v) => format!(
            "{{{}{{{}}}}}",
            show(n),
            v.iter().map(show).collect::<Vec<_>>().join(", ")
        ),
        Expr::Unary(o, x) => format!("{o}{}", show(x)),
        Expr::Binary(o, a, b) => format!("({} {o} {})", show(a), show(b)),
        Expr::Cond(c, a, b) => format!("({} ? {} : {})", show(c), show(a), show(b)),
        Expr::Call(n, a) => format!(
            "{}({})",
            show(n),
            a.iter().map(show).collect::<Vec<_>>().join(", ")
        ),
        Expr::SysCall(n, a) => {
            format!("{n}({})", a.iter().map(show).collect::<Vec<_>>().join(", "))
        }
        Expr::MinTypMax(a, b, c) => format!("{}:{}:{}", show(a), show(b), show(c)),
    }
}

fn range_text(r: &Option<Range>) -> Option<(String, String)> {
    r.as_ref().map(|r| (show(&r.msb), show(&r.lsb)))
}

pub(crate) struct Ctx<'a> {
    pub pre: &'a Pre,
    pub file: usize,
    pub diags: &'a mut Vec<Diag>,
}

impl Ctx<'_> {
    fn span(&self, id: &Ident) -> Span {
        self.pre.span(id.pos, id.pos + id.name.len().max(1) as u32)
    }
}

// ----- visitors -----

/// Every expression of an expression tree that is a use of a plain name, skipping hierarchical roots.
fn visit_names(e: &Expr, f: &mut dyn FnMut(&Ident)) {
    match e {
        Expr::Ident(i) => f(i),
        Expr::Dot(b, _) => visit_root(b, f),
        Expr::Index(a, b) => {
            visit_names(a, f);
            visit_names(b, f);
        }
        Expr::Range(a, _, b, c) | Expr::Cond(a, b, c) | Expr::MinTypMax(a, b, c) => {
            visit_names(a, f);
            visit_names(b, f);
            visit_names(c, f);
        }
        Expr::Concat(v) | Expr::SysCall(_, v) => v.iter().for_each(|x| visit_names(x, f)),
        Expr::Repl(n, v) => {
            visit_names(n, f);
            v.iter().for_each(|x| visit_names(x, f));
        }
        Expr::Unary(_, x) => visit_names(x, f),
        Expr::Binary(_, a, b) => {
            visit_names(a, f);
            visit_names(b, f);
        }
        Expr::Call(n, v) => {
            visit_names(n, f);
            v.iter().for_each(|x| visit_names(x, f));
        }
        Expr::Empty | Expr::Num(_) | Expr::Real(_) | Expr::Str(_) => {}
    }
}

/// The base of `a.b`: its root name is a hierarchical scope, not a use.
fn visit_root(b: &Expr, f: &mut dyn FnMut(&Ident)) {
    match b {
        Expr::Ident(_) => {}
        Expr::Dot(bb, _) => visit_root(bb, f),
        Expr::Index(bb, i) => {
            visit_root(bb, f);
            visit_names(i, f);
        }
        other => visit_names(other, f),
    }
}

fn delay_exprs(d: &Option<Delay>, f: &mut dyn FnMut(&Expr)) {
    if let Some(d) = d {
        d.0.iter().for_each(&mut *f);
    }
}

fn range_exprs(r: &Option<Range>, f: &mut dyn FnMut(&Expr)) {
    if let Some(r) = r {
        f(&r.msb);
        f(&r.lsb);
    }
}

fn declarator_exprs(ds: &[Declarator], f: &mut dyn FnMut(&Expr)) {
    for d in ds {
        for r in &d.dims {
            f(&r.msb);
            f(&r.lsb);
        }
        if let Some(i) = &d.init {
            f(i);
        }
    }
}

fn event_exprs(c: &EventCtrl, f: &mut dyn FnMut(&Expr)) {
    if let EventCtrl::List(v) = c {
        v.iter().for_each(|e| f(&e.expr));
    }
}

/// Calls `f` on every top-level expression of a statement, nested statements included.
fn stmt_exprs(s: &Stmt, f: &mut dyn FnMut(&Expr)) {
    match s {
        Stmt::Null => {}
        Stmt::Block { decls, stmts, .. } => {
            decls.iter().for_each(|d| item_exprs(d, f));
            stmts.iter().for_each(|s| stmt_exprs(s, f));
        }
        Stmt::Assign { lhs, rhs, ctrl, .. } => {
            f(lhs);
            f(rhs);
            match ctrl {
                Some(Ctrl::Delay(d)) => d.0.iter().for_each(&mut *f),
                Some(Ctrl::Event(c)) => event_exprs(c, f),
                Some(Ctrl::Repeat(n, c)) => {
                    f(n);
                    event_exprs(c, f);
                }
                None => {}
            }
        }
        Stmt::ProcAssign { lhs, rhs, .. } => {
            f(lhs);
            if let Some(r) = rhs {
                f(r);
            }
        }
        Stmt::If { cond, then, els } => {
            f(cond);
            stmt_exprs(then, f);
            if let Some(e) = els {
                stmt_exprs(e, f);
            }
        }
        Stmt::Case { expr, items, .. } => {
            f(expr);
            for it in items {
                it.exprs.iter().for_each(&mut *f);
                stmt_exprs(&it.stmt, f);
            }
        }
        Stmt::For {
            init,
            cond,
            step,
            body,
        } => {
            f(&init.0);
            f(&init.1);
            f(cond);
            f(&step.0);
            f(&step.1);
            stmt_exprs(body, f);
        }
        Stmt::While { cond: e, body }
        | Stmt::Repeat { count: e, body }
        | Stmt::Wait { cond: e, body } => {
            f(e);
            stmt_exprs(body, f);
        }
        Stmt::Forever(b) => stmt_exprs(b, f),
        Stmt::Delay { delay, body } => {
            delay.0.iter().for_each(&mut *f);
            stmt_exprs(body, f);
        }
        Stmt::Event { ctrl, body } => {
            event_exprs(ctrl, f);
            stmt_exprs(body, f);
        }
        Stmt::Trigger(e) => f(e),
        Stmt::Disable(_) => {}
        Stmt::TaskCall { name, args } => {
            f(name);
            args.iter().for_each(&mut *f);
        }
        Stmt::SysCall { args, .. } => args.iter().for_each(f),
    }
}

fn inst_exprs(i: &Instance, f: &mut dyn FnMut(&Expr)) {
    range_exprs(&i.range, f);
    for c in &i.conns {
        match c {
            Conn::Ordered(e) => f(e),
            Conn::Named(_, Some(e)) => f(e),
            Conn::Named(_, None) => {}
        }
    }
}

fn gen_body_exprs(b: &GenBody, f: &mut dyn FnMut(&Expr)) {
    b.items.iter().for_each(|i| item_exprs(i, f));
}

/// Calls `f` on every top-level expression of an item.
fn item_exprs(it: &Item, f: &mut dyn FnMut(&Expr)) {
    match it {
        Item::Port(p) => {
            range_exprs(&p.range, f);
            declarator_exprs(&p.names, f);
        }
        Item::Net(n) => {
            range_exprs(&n.range, f);
            delay_exprs(&n.delay, f);
            declarator_exprs(&n.names, f);
        }
        Item::Var(v) => {
            range_exprs(&v.range, f);
            declarator_exprs(&v.names, f);
        }
        Item::Event(ds) => declarator_exprs(ds, f),
        Item::Genvar(_) => {}
        Item::Param(p) => {
            range_exprs(&p.range, f);
            p.assigns.iter().for_each(|(_, e)| f(e));
        }
        Item::Assign(a) => {
            delay_exprs(&a.delay, f);
            for (l, r) in &a.assigns {
                f(l);
                f(r);
            }
        }
        Item::Gate(g) => {
            delay_exprs(&g.delay, f);
            g.insts.iter().for_each(|i| inst_exprs(i, f));
        }
        Item::Inst(m) => {
            for c in &m.params {
                match c {
                    Conn::Ordered(e) | Conn::Named(_, Some(e)) => f(e),
                    Conn::Named(_, None) => {}
                }
            }
            m.insts.iter().for_each(|i| inst_exprs(i, f));
        }
        Item::Defparam(v) => v.iter().for_each(|(_, e)| f(e)),
        Item::Always(s) | Item::Initial(s) => stmt_exprs(s, f),
        Item::Task(t) => {
            t.decls.iter().for_each(|d| item_exprs(d, f));
            t.body.iter().for_each(|s| stmt_exprs(s, f));
        }
        Item::Function(t) => {
            range_exprs(&t.range, f);
            t.decls.iter().for_each(|d| item_exprs(d, f));
            t.body.iter().for_each(|s| stmt_exprs(s, f));
        }
        Item::Generate(v) => v.iter().for_each(|i| item_exprs(i, f)),
        Item::GenIf { cond, then, els } => {
            f(cond);
            gen_body_exprs(then, f);
            if let Some(e) = els {
                gen_body_exprs(e, f);
            }
        }
        Item::GenFor {
            init,
            cond,
            step,
            body,
        } => {
            f(&init.1);
            f(cond);
            f(&step.1);
            gen_body_exprs(body, f);
        }
        Item::GenCase { expr, items } => {
            f(expr);
            for (l, b) in items {
                l.iter().for_each(&mut *f);
                gen_body_exprs(b, f);
            }
        }
        Item::Specify(v) => {
            for s in v {
                match s {
                    SpecItem::Param(p) => p.assigns.iter().for_each(|(_, e)| f(e)),
                    SpecItem::Path(p) => {
                        p.cond.iter().for_each(&mut *f);
                        p.src.iter().for_each(&mut *f);
                        p.dst.iter().for_each(&mut *f);
                        p.data.iter().for_each(&mut *f);
                        p.delays.iter().for_each(&mut *f);
                    }
                    SpecItem::Check { args, .. } => {
                        for a in args {
                            f(&a.expr);
                            a.cond.iter().for_each(&mut *f);
                        }
                    }
                    SpecItem::Pulse { outputs, .. } => outputs.iter().for_each(&mut *f),
                }
            }
        }
    }
}

/// Every name declared anywhere in the module, any scope (the check is deliberately loose).
fn collect_declared(items: &[Item], out: &mut HashSet<String>) {
    for it in items {
        match it {
            Item::Port(p) => p.names.iter().for_each(|d| {
                out.insert(d.name.name.clone());
            }),
            Item::Net(n) => n.names.iter().for_each(|d| {
                out.insert(d.name.name.clone());
            }),
            Item::Var(v) => v.names.iter().for_each(|d| {
                out.insert(d.name.name.clone());
            }),
            Item::Event(ds) => ds.iter().for_each(|d| {
                out.insert(d.name.name.clone());
            }),
            Item::Genvar(v) => v.iter().for_each(|i| {
                out.insert(i.name.clone());
            }),
            Item::Param(p) => p.assigns.iter().for_each(|(i, _)| {
                out.insert(i.name.clone());
            }),
            Item::Gate(g) => g
                .insts
                .iter()
                .filter_map(|i| i.name.as_ref())
                .for_each(|i| {
                    out.insert(i.name.clone());
                }),
            Item::Inst(m) => m
                .insts
                .iter()
                .filter_map(|i| i.name.as_ref())
                .for_each(|i| {
                    out.insert(i.name.clone());
                }),
            Item::Always(s) | Item::Initial(s) => collect_stmt(s, out),
            Item::Task(t) => {
                out.insert(t.name.name.clone());
                collect_declared(&t.decls, out);
                t.body.iter().for_each(|s| collect_stmt(s, out));
            }
            Item::Function(t) => {
                out.insert(t.name.name.clone());
                collect_declared(&t.decls, out);
                t.body.iter().for_each(|s| collect_stmt(s, out));
            }
            Item::Generate(v) => collect_declared(v, out),
            Item::GenIf { then, els, .. } => {
                collect_body(then, out);
                els.iter().for_each(|b| collect_body(b, out));
            }
            Item::GenFor {
                init, step, body, ..
            } => {
                out.insert(init.0.name.clone());
                out.insert(step.0.name.clone());
                collect_body(body, out);
            }
            Item::GenCase { items, .. } => items.iter().for_each(|(_, b)| collect_body(b, out)),
            Item::Specify(v) => {
                for s in v {
                    if let SpecItem::Param(p) = s {
                        p.assigns.iter().for_each(|(i, _)| {
                            out.insert(i.name.clone());
                        });
                    }
                }
            }
            Item::Assign(_) | Item::Defparam(_) => {}
        }
    }
}

fn collect_body(b: &GenBody, out: &mut HashSet<String>) {
    if let Some(l) = &b.label {
        out.insert(l.name.clone());
    }
    collect_declared(&b.items, out);
}

fn collect_stmt(s: &Stmt, out: &mut HashSet<String>) {
    match s {
        Stmt::Block {
            label,
            decls,
            stmts,
            ..
        } => {
            if let Some(l) = label {
                out.insert(l.name.clone());
            }
            collect_declared(decls, out);
            stmts.iter().for_each(|s| collect_stmt(s, out));
        }
        Stmt::If { then, els, .. } => {
            collect_stmt(then, out);
            if let Some(e) = els {
                collect_stmt(e, out);
            }
        }
        Stmt::Case { items, .. } => items.iter().for_each(|i| collect_stmt(&i.stmt, out)),
        Stmt::For { body, .. }
        | Stmt::While { body, .. }
        | Stmt::Repeat { body, .. }
        | Stmt::Wait { body, .. }
        | Stmt::Delay { body, .. }
        | Stmt::Event { body, .. } => collect_stmt(body, out),
        Stmt::Forever(b) => collect_stmt(b, out),
        _ => {}
    }
}

/// Whether the module contains a timing check.
pub(crate) fn has_timing_check(m: &Module) -> bool {
    fn walk(items: &[Item]) -> bool {
        items.iter().any(|i| match i {
            Item::Specify(v) => v.iter().any(|s| matches!(s, SpecItem::Check { .. })),
            Item::Generate(v) => walk(v),
            Item::GenIf { then, els, .. } => {
                walk(&then.items) || els.as_ref().is_some_and(|e| walk(&e.items))
            }
            Item::GenFor { body, .. } => walk(&body.items),
            Item::GenCase { items, .. } => items.iter().any(|(_, b)| walk(&b.items)),
            _ => false,
        })
    }
    walk(&m.items)
}

// ----- the module -----

/// Check a module and return its ports.
pub(crate) fn module(cx: &mut Ctx, m: &Module) -> Vec<Port> {
    let ports = ports(cx, m);
    if m.encrypted.is_some() {
        return ports;
    }
    duplicates(cx, m);
    if m.nettype == Nettype::None {
        undeclared(cx, m);
    }
    ports
}

fn ports(cx: &mut Ctx, m: &Module) -> Vec<Port> {
    match &m.ports {
        Ports::Ansi(v) => v
            .iter()
            .flat_map(|d| {
                d.names.iter().map(|n| Port {
                    name: n.name.name.clone(),
                    dir: d.dir,
                    range: range_text(&d.range),
                })
            })
            .collect(),
        Ports::None | Ports::NonAnsi(_) => {
            // Direction declarations in the body.
            let mut decl: HashMap<&str, (&PortDecl, &Ident)> = HashMap::new();
            for it in &m.items {
                if let Item::Port(p) = it {
                    for n in &p.names {
                        if let Some((_, first)) = decl.get(n.name.name.as_str()) {
                            let d = Diag::error(
                                "E4202",
                                format!("port `{}` is declared twice", n.name.name),
                                cx.span(&n.name),
                            )
                            .label("declared again here");
                            let _ = first;
                            cx.diags.push(d);
                        } else {
                            decl.insert(n.name.name.as_str(), (p, &n.name));
                        }
                    }
                }
            }
            let list: &[NonAnsiPort] = match &m.ports {
                Ports::NonAnsi(l) => l,
                _ => &[],
            };
            let mut out = Vec::new();
            let mut listed: HashSet<String> = HashSet::new();
            for p in list {
                let mut first_ident: Option<Ident> = None;
                let key: Option<Ident> = match (&p.name, &p.expr) {
                    (Some(n), None) => Some(n.clone()),
                    (_, Some(e)) => {
                        visit_names(e, &mut |i| {
                            first_ident.get_or_insert_with(|| i.clone());
                        });
                        first_ident
                    }
                    (None, None) => None,
                };
                let Some(key) = key else { continue };
                listed.insert(key.name.clone());
                let ext = p.name.as_ref().unwrap_or(&key);
                match decl.get(key.name.as_str()) {
                    Some((d, _)) => out.push(Port {
                        name: ext.name.clone(),
                        dir: d.dir,
                        range: if p.expr.is_none() {
                            range_text(&d.range)
                        } else {
                            None
                        },
                    }),
                    None if m.encrypted.is_some() => {}
                    None => cx.diags.push(
                        Diag::error(
                            "E4202",
                            format!("port `{}` has no direction declaration", ext.name),
                            cx.span(ext),
                        )
                        .label("listed in the port list")
                        .help("declare it with `input`, `output` or `inout` in the module body"),
                    ),
                }
            }
            if m.encrypted.is_none() {
                for (name, (_, id)) in &decl {
                    if !listed.contains(*name) {
                        cx.diags.push(
                            Diag::error("E4202", format!("`{name}` is declared as a port but is not in the port list of `{}`", m.name.name), cx.span(id))
                                .label("not listed"),
                        );
                    }
                }
            }
            out
        }
    }
}

fn duplicates(cx: &mut Ctx, m: &Module) {
    let mut seen: HashMap<String, ()> = HashMap::new();
    let mut check = |cx: &mut Ctx, id: &Ident, what: &str| {
        if seen.insert(id.name.clone(), ()).is_some() {
            cx.diags.push(
                Diag::error(
                    "E4203",
                    format!(
                        "`{}` is already declared in module `{}`",
                        id.name, m.name.name
                    ),
                    cx.span(id),
                )
                .label(format!("{what} declared again here"))
                .note("a name is declared once per scope (clause 12.1.2)"),
            );
        }
    };
    for p in &m.params {
        for (i, _) in &p.assigns {
            check(cx, i, "parameter");
        }
    }
    for it in &m.items {
        match it {
            Item::Net(n) => n.names.iter().for_each(|d| check(cx, &d.name, "net")),
            Item::Var(v) => v.names.iter().for_each(|d| check(cx, &d.name, "variable")),
            Item::Event(ds) => ds.iter().for_each(|d| check(cx, &d.name, "event")),
            Item::Genvar(v) => v.iter().for_each(|i| check(cx, i, "genvar")),
            Item::Param(p) => p
                .assigns
                .iter()
                .for_each(|(i, _)| check(cx, i, "parameter")),
            Item::Task(t) => check(cx, &t.name, "task"),
            Item::Function(t) => check(cx, &t.name, "function"),
            Item::Inst(mi) => mi
                .insts
                .iter()
                .filter_map(|i| i.name.as_ref())
                .for_each(|i| check(cx, i, "instance")),
            _ => {}
        }
    }
}

fn undeclared(cx: &mut Ctx, m: &Module) {
    let mut declared: HashSet<String> = HashSet::new();
    declared.insert(m.name.name.clone());
    for p in &m.params {
        p.assigns.iter().for_each(|(i, _)| {
            declared.insert(i.name.clone());
        });
    }
    if let Ports::Ansi(v) = &m.ports {
        for d in v {
            d.names.iter().for_each(|n| {
                declared.insert(n.name.name.clone());
            });
        }
    }
    if let Ports::NonAnsi(v) = &m.ports {
        for p in v {
            if let Some(n) = &p.name {
                declared.insert(n.name.clone());
            }
        }
    }
    collect_declared(&m.items, &mut declared);
    let mut reported: HashSet<String> = HashSet::new();
    let mut found: Vec<Ident> = Vec::new();
    let mut f = |e: &Expr| {
        visit_names(e, &mut |i| {
            if !declared.contains(&i.name) && reported.insert(i.name.clone()) {
                found.push(i.clone());
            }
        })
    };
    for p in &m.params {
        range_exprs(&p.range, &mut f);
        p.assigns.iter().for_each(|(_, e)| f(e));
    }
    if let Ports::Ansi(v) = &m.ports {
        for d in v {
            range_exprs(&d.range, &mut f);
        }
    }
    for it in &m.items {
        item_exprs(it, &mut f);
    }
    let names: Vec<&str> = declared.iter().map(|s| s.as_str()).collect();
    for id in found {
        let mut d = Diag::error(
            "E4201",
            format!("undeclared identifier `{}`", id.name),
            cx.span(&id),
        )
        .label("not declared in this module")
        .note("`default_nettype none is in force, so no implicit net is created (clause 4.5)");
        d = match yavs_diag::suggest::suggest(&id.name, "name", names.iter().map(|n| (*n, *n))) {
            Some(h) => d.help(h),
            None => d.help("declare it, or use `default_nettype wire"),
        };
        cx.diags.push(d);
    }
}

/// The ports of a UDP: the output first.
pub(crate) fn udp_ports(u: &Udp) -> Vec<Port> {
    let mut v = vec![Port {
        name: u.output.name.clone(),
        dir: Dir::Output,
        range: None,
    }];
    v.extend(u.inputs.iter().map(|i| Port {
        name: i.name.clone(),
        dir: Dir::Input,
        range: None,
    }));
    v
}
