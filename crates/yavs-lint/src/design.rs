//! The rules about one architecture as a design: drivers, reads, clocks, resets and state machines,
//! and the design facts report. Sem says what a statement drives and reads (`ProcessInfo`,
//! `Db::stmt_reads`, `Db::edge_shape`), so nothing here reads names from text. A process is
//! judged only in a shape that says what it is; anything else is left alone.

use crate::Config;
use crate::rules::Raw;
use serde_json::{Value, json};
use std::collections::{HashMap, HashSet};
use yavs_diag::Span;
use yavs_sem::decl::{Body, DeclKind, ObjClass, ObjRole};
use yavs_sem::hir::{
    Actual, ConstValue, Lsp, Mode, ObjRoot, Rhs, Sensitivity, StmtKind, UnitHir, Waveform,
};
use yavs_sem::loader::UnitKey;
use yavs_sem::lsp::{Overlap, Use};
use yavs_sem::types::TypeKind;
use yavs_sem::{Db, DeclId, ExprId, ScopeId, StmtId, TypeId, UnitId};

/// A concurrent statement as its equivalent process.
struct Conc<'a> {
    stmt: StmtId,
    span: Span,
    drivers: Vec<Lsp>,
    reads: Vec<Lsp>,
    /// The if and case generate alternatives it is in. Two statements under different ones may
    /// never exist together: `if g generate` and `if not g generate` say so, and a generic decides.
    branch: Vec<(StmtId, usize)>,
    process: Option<&'a [StmtId]>,
    /// An assignment with an `after`: time passes, no combinational path.
    delayed: bool,
}

/// What a process is, when its shape says so.
enum Shape<'a> {
    Combinational,
    Clocked {
        clock: Lsp,
        rising: bool,
        reset: Option<Lsp>,
        reset_body: &'a [StmtId],
        body: &'a [StmtId],
    },
    /// Suspends on `wait`, may do so in a call, or tests an edge in another form.
    Other,
}

fn decl(root: ObjRoot) -> Option<DeclId> {
    match root {
        ObjRoot::Decl(d) | ObjRoot::ImplicitSignal(d) => Some(d),
        ObjRoot::External(_) => None,
    }
}

fn deep(db: &Db, list: &[StmtId], out: &mut Vec<StmtId>) {
    for &s in list {
        out.push(s);
        deep(db, &db.stmt_children(s), out);
    }
}

/// Every expression of the statements below `list`, with what its statement does with it.
fn exprs_of(db: &Db, list: &[StmtId]) -> Vec<(ExprId, Use)> {
    let mut all = Vec::new();
    deep(db, list, &mut all);
    let mut out = Vec::new();
    for s in all {
        db.stmt_exprs(s, &mut |e, u| out.push((e, u)));
    }
    out
}

fn edge_in(db: &Db, e: ExprId) -> Option<yavs_sem::edge::EdgeShape> {
    let mut found = None;
    db.walk_expr(e, &mut |x| {
        found = found.or(db.edge_shape(x));
    });
    found
}

fn shape<'a>(db: &'a Db, body: &'a [StmtId]) -> Shape<'a> {
    let mut all = Vec::new();
    deep(db, body, &mut all);
    if all.iter().any(|&s| {
        matches!(
            db.stmts[s].kind,
            StmtKind::Wait { .. } | StmtKind::ProcCall(_)
        )
    }) {
        return Shape::Other;
    }
    if !exprs_of(db, body)
        .iter()
        .any(|(e, _)| edge_in(db, *e).is_some())
    {
        return Shape::Combinational;
    }
    let [only] = body else {
        return Shape::Other;
    };
    let StmtKind::If { branches, else_ } = &db.stmts[*only].kind else {
        return Shape::Other;
    };
    let clocked = |at: usize, reset, reset_body| {
        let (c, b) = branches.get(at)?;
        let edge = edge_in(db, *c)?;
        Some(Shape::Clocked {
            clock: db.lsp(edge.sig)?,
            rising: edge.rising,
            reset,
            reset_body,
            body: b,
        })
    };
    if let Some(s) = clocked(0, None, &[][..]) {
        return s;
    }
    // `if rst = '1' then ... elsif rising_edge (clk) then`: the only place a reset can be told from
    // a clock enable.
    if branches.len() == 2
        && else_.is_none()
        && let Some((sig, _)) = db.level_of(branches[0].0)
        && let Some(reset) = db.lsp(sig)
        && let Some(s) = clocked(1, Some(reset), &branches[0].1[..])
    {
        return s;
    }
    Shape::Other
}

/// Whether an assignment names an `after`: time passes, no combinational path.
fn delayed(rhs: &Rhs<Waveform>) -> bool {
    let one =
        |w: &Waveform| matches!(w, Waveform::Elements(e) if e.iter().any(|x| x.after.is_some()));
    match rhs {
        Rhs::Simple(w) => one(w),
        Rhs::Conditional(a) => a.iter().any(|(w, _)| one(w)),
        Rhs::Selected { alts, .. } => alts.iter().any(|(w, _)| one(w)),
    }
}

fn delayed_assign(db: &Db, s: StmtId) -> bool {
    matches!(&db.stmts[s].kind, StmtKind::SignalAssign { rhs, .. } if delayed(rhs))
}

fn flatten<'a>(
    db: &'a Db,
    list: &'a [StmtId],
    branch: &[(StmtId, usize)],
    out: &mut Vec<Conc<'a>>,
    scopes: &mut Vec<ScopeId>,
) {
    for &s in list {
        let stmt = &db.stmts[s];
        let mut conc = Conc {
            stmt: s,
            span: stmt.span,
            drivers: Vec::new(),
            reads: Vec::new(),
            branch: branch.to_vec(),
            process: None,
            delayed: false,
        };
        match &stmt.kind {
            StmtKind::Process {
                info, body, scope, ..
            } => {
                conc.drivers = info.drivers.clone();
                db.reads_deep(body, &mut conc.reads);
                conc.process = Some(body);
                scopes.push(*scope);
            }
            StmtKind::ConcSignalAssign { info, rhs, .. } => {
                conc.drivers = info.drivers.clone();
                db.stmt_reads(s, &mut conc.reads);
                let delayed = |w: &Waveform| matches!(w, Waveform::Elements(e) if e.iter().any(|x| x.after.is_some()));
                conc.delayed = match rhs {
                    Rhs::Simple(w) => delayed(w),
                    Rhs::Conditional(a) => a.iter().any(|(w, _)| delayed(w)),
                    Rhs::Selected { alts, .. } => alts.iter().any(|(w, _)| delayed(w)),
                };
            }
            StmtKind::ConcProcCall { info, .. } | StmtKind::ConcAssert { info, .. } => {
                conc.drivers = info.drivers.clone();
                db.stmt_reads(s, &mut conc.reads);
            }
            StmtKind::Instance {
                target, port_map, ..
            } => {
                let formals = db.inst_formals(target).map(|f| f.1);
                for a in port_map.iter() {
                    let mode = formals
                        .and_then(|f| f.get(a.formal as usize))
                        .and_then(|&d| match &db.decls[d].kind {
                            DeclKind::Object(o) => o.mode,
                            _ => None,
                        });
                    let mut actuals = Vec::new();
                    match &a.actual {
                        Actual::Expr(e) | Actual::Converted { actual: e, .. } => actuals.push(*e),
                        Actual::Individual(parts) => actuals.extend(parts.iter().map(|p| p.1)),
                        _ => {}
                    }
                    let converted = matches!(a.actual, Actual::Converted { .. });
                    for e in actuals {
                        db.reads(e, &mut conc.reads);
                        if let Some(l) = db.lsp(e) {
                            // Only a mode that is known drives only as it says.
                            if converted || !matches!(mode, Some(Mode::In)) {
                                conc.drivers.push(l);
                            }
                        }
                    }
                }
            }
            StmtKind::Block { scope, body, .. } => {
                scopes.push(*scope);
                flatten(db, body, branch, out, scopes);
                continue;
            }
            StmtKind::ForGenerate { scope, body, .. } => {
                scopes.push(*scope);
                flatten(db, body, branch, out, scopes);
                continue;
            }
            StmtKind::IfGenerate { alts } => {
                for (i, alt) in alts.iter().enumerate() {
                    if alt
                        .cond
                        .is_some_and(|c| matches!(db.value(c), Some(ConstValue::Enum(0))))
                    {
                        continue;
                    }
                    let mut b = branch.to_vec();
                    b.push((s, i));
                    scopes.push(alt.scope);
                    flatten(db, &alt.body, &b, out, scopes);
                }
                continue;
            }
            StmtKind::CaseGenerate { alts, .. } => {
                for (i, (_, alt)) in alts.iter().enumerate() {
                    let mut b = branch.to_vec();
                    b.push((s, i));
                    scopes.push(alt.scope);
                    flatten(db, &alt.body, &b, out, scopes);
                }
                continue;
            }
            _ => continue,
        }
        out.push(conc);
    }
}

fn line(db: &Db, span: Span) -> usize {
    db.sources.get(span.file).line_col(span.start).0
}

fn name(db: &Db, d: DeclId) -> String {
    db.decls[d].name.to_string()
}

fn show(db: &Db, l: &Lsp) -> String {
    let file = db.sources.get(l.span.file);
    file.try_text()
        .ok()
        .and_then(|t| t.get(file.text_offset(l.span.start)..file.text_offset(l.span.end)))
        .map_or_else(
            || decl(l.root).map_or_else(String::new, |d| name(db, d)),
            str::to_string,
        )
}

struct Arch<'a> {
    db: &'a Db,
    stmts: &'a [StmtId],
    conc: Vec<Conc<'a>>,
    scopes: Vec<ScopeId>,
    out: &'a mut Vec<Raw>,
}

/// The rules of one architecture. Returns its design facts.
pub(crate) fn architecture(db: &Db, u: UnitId, cfg: &Config, out: &mut Vec<Raw>) -> Value {
    let unit = &db.units[u];
    let UnitHir::Architecture { stmts, .. } = &unit.hir else {
        return Value::Null;
    };
    let (mut conc, mut scopes) = (Vec::new(), vec![unit.scope]);
    flatten(db, stmts, &[], &mut conc, &mut scopes);
    let mut a = Arch {
        db,
        stmts,
        conc,
        scopes,
        out,
    };
    a.drivers();
    a.undriven();
    a.loops();
    a.sensitivity();
    a.latches();
    a.unused();
    let domains = a.domains(cfg);
    let machines = a.machines();
    json!({
        "unit": match unit.key {
            UnitKey::Secondary(e, a) => format!("{e}({a})"),
            UnitKey::Primary(p) => p.to_string(),
        },
        "file": db.sources.get(unit.file).path,
        "state_machines": machines,
        "clock_domains": domains.0,
        "reset_domains": domains.1,
        "crossings": domains.2,
    })
}

impl Arch<'_> {
    fn push(&mut self, rule: &'static str, span: Span, message: String) {
        self.out.push((rule, span, message));
    }

    /// `lint_601`: one signal driven from more than one statement. A part that overlaps only for
    /// some values of a generic does not count.
    fn drivers(&mut self) {
        let db = self.db;
        let mut by_root: HashMap<DeclId, Vec<(usize, &Lsp)>> = HashMap::new();
        for (i, c) in self.conc.iter().enumerate() {
            for l in &c.drivers {
                if let Some(d) = decl(l.root) {
                    by_root.entry(d).or_default().push((i, l));
                }
            }
        }
        let mut found = Vec::new();
        for (d, all) in by_root {
            let mut lines: Vec<(usize, Span)> = Vec::new();
            for (n, (i, a)) in all.iter().enumerate() {
                for (j, b) in &all[n + 1..] {
                    if i != j
                        && matches!(db.overlap(a, b), Overlap::Yes)
                        && self.conc[*i].branch == self.conc[*j].branch
                    {
                        lines.push((*i, self.conc[*i].span));
                        lines.push((*j, self.conc[*j].span));
                    }
                }
            }
            lines.sort_by_key(|x| x.0);
            lines.dedup_by_key(|x| x.0);
            if let Some(first) = lines.first() {
                let at: Vec<String> = lines.iter().map(|l| line(db, l.1).to_string()).collect();
                found.push((
                    first.1,
                    format!(
                        "Signal `{}` is assigned by {} concurrent statements (lines {})",
                        name(db, d),
                        lines.len(),
                        at.join(", ")
                    ),
                ));
            }
        }
        for (span, m) in found {
            self.push("lint_601", span, m);
        }
    }

    /// The signals subprograms of the region may read or drive by name: not known per statement.
    fn named_in_subprograms(&self) -> HashSet<DeclId> {
        let db = self.db;
        let mut out = HashSet::new();
        let mut todo = self.scopes.clone();
        let mut seen = HashSet::new();
        while let Some(s) = todo.pop() {
            if !seen.insert(s) {
                continue;
            }
            for &d in &db.scopes[s].order {
                if let DeclKind::Subprogram(sp) = &db.decls[d].kind
                    && let Body::Hir { scope, stmts } = &sp.body
                {
                    todo.push(*scope);
                    for (e, _) in exprs_of(db, stmts) {
                        db.walk_expr(e, &mut |x| {
                            if let Some(yavs_sem::lsp::NameRoot::Obj(r)) = db.name_root(x)
                                && let Some(d) = decl(r)
                            {
                                out.insert(d);
                            }
                        });
                    }
                }
            }
        }
        out
    }

    /// `lint_730`: a signal read that nothing drives. An initial value says it is tied on purpose.
    fn undriven(&mut self) {
        let db = self.db;
        let mut driven = self.named_in_subprograms();
        let mut read = HashSet::new();
        for c in &self.conc {
            driven.extend(c.drivers.iter().filter_map(|l| decl(l.root)));
            read.extend(c.reads.iter().filter_map(|l| decl(l.root)));
            if let Some(body) = c.process {
                // `force` is no driver, and still gives the signal a value.
                for (e, u) in exprs_of(db, body) {
                    if u == Use::Target
                        && let Some(yavs_sem::lsp::NameRoot::Obj(r)) = db.name_root(e)
                    {
                        driven.extend(decl(r));
                    }
                }
            }
        }
        for s in self.scopes.clone() {
            for &d in &db.scopes[s].order {
                if let DeclKind::Object(o) = &db.decls[d].kind
                    && o.class == ObjClass::Signal
                    && o.role == ObjRole::Plain
                    && o.default.is_none()
                    && read.contains(&d)
                    && !driven.contains(&d)
                {
                    self.push(
                        "lint_730",
                        db.decls[d].name_span(),
                        format!("Signal `{}` is read but nothing drives it", name(db, d)),
                    );
                }
            }
        }
    }

    /// `lint_720`: a signal that depends on itself through combinational logic. Parts of objects,
    /// instances and assignments with an `after` give no edge, so a reported cycle is a real one.
    fn loops(&mut self) {
        let db = self.db;
        let mut edges: HashMap<DeclId, Vec<DeclId>> = HashMap::new();
        let mut places: HashMap<DeclId, Span> = HashMap::new();
        for c in &self.conc {
            let combinational = match c.process {
                Some(body) => {
                    let mut all = Vec::new();
                    deep(db, body, &mut all);
                    matches!(shape(db, body), Shape::Combinational)
                        && !all.iter().any(|&s| delayed_assign(db, s))
                }
                // `q <= d when rising_edge (clk)` is a register written as an assignment.
                None => {
                    matches!(db.stmts[c.stmt].kind, StmtKind::ConcSignalAssign { .. })
                        && !c.delayed
                        && !exprs_of(db, &[c.stmt])
                            .iter()
                            .any(|(e, _)| edge_in(db, *e).is_some())
                }
            };
            if !combinational {
                continue;
            }
            for t in c.drivers.iter().filter(|l| l.path.is_empty()) {
                let Some(t) = decl(t.root) else { continue };
                places.entry(t).or_insert(c.span);
                for r in c.reads.iter().filter(|l| l.path.is_empty()) {
                    if let Some(r) = decl(r.root) {
                        edges.entry(r).or_default().push(t);
                    }
                }
            }
        }
        // Depth first, one cycle per back edge found; 0 unseen, 1 on the path, 2 done.
        fn visit(
            n: DeclId,
            edges: &HashMap<DeclId, Vec<DeclId>>,
            state: &mut HashMap<DeclId, u8>,
            path: &mut Vec<DeclId>,
            cycles: &mut Vec<Vec<DeclId>>,
        ) {
            state.insert(n, 1);
            path.push(n);
            for &m in edges.get(&n).map_or(&[][..], |v| &v[..]) {
                match state.get(&m).copied().unwrap_or(0) {
                    0 => visit(m, edges, state, path, cycles),
                    1 => {
                        let from = path.iter().position(|&x| x == m).expect("on the path");
                        cycles.push(path[from..].to_vec());
                    }
                    _ => {}
                }
            }
            path.pop();
            state.insert(n, 2);
        }
        let mut starts: Vec<DeclId> = edges.keys().copied().collect();
        starts.sort_by_key(|&d| db.decls[d].span.start);
        let (mut state, mut cycles) = (HashMap::new(), Vec::new());
        for n in starts {
            if !state.contains_key(&n) {
                visit(n, &edges, &mut state, &mut Vec::new(), &mut cycles);
            }
        }
        for cycle in cycles {
            let mut names: Vec<String> = cycle.iter().map(|&d| name(db, d)).collect();
            names.push(names[0].clone());
            let span = cycle
                .iter()
                .find_map(|d| places.get(d))
                .copied()
                .unwrap_or(db.decls[cycle[0]].span);
            self.push(
                "lint_720",
                span,
                format!(
                    "Combinational loop: {} depends on itself with no register in the way",
                    names.join(" -> ")
                ),
            );
        }
    }

    /// `lint_001` and `lint_002`: a sensitivity list against what the process reads. A call may
    /// read signals by name, so a process that makes one is judged only for what is missing.
    fn sensitivity(&mut self) {
        let db = self.db;
        let mut found = Vec::new();
        for c in &self.conc {
            let Some(body) = c.process else { continue };
            let StmtKind::Process {
                sensitivity: Sensitivity::List(list),
                ..
            } = &db.stmts[c.stmt].kind
            else {
                continue;
            };
            let covers = |l: &Lsp, x: &Lsp| !matches!(db.overlap(l, x), Overlap::No);
            if matches!(shape(db, body), Shape::Combinational) {
                let mut named = HashSet::new();
                for r in &c.reads {
                    if !list.iter().any(|l| covers(l, r))
                        && decl(r.root).is_some_and(|d| named.insert(d))
                    {
                        found.push((
                            "lint_001",
                            r.span,
                            format!(
                                "Signal `{}` is read by this combinational process and is not in its sensitivity list",
                                show(db, r)
                            ),
                        ));
                    }
                }
            }
            let mut calls = false;
            for (e, _) in exprs_of(db, body) {
                db.walk_expr(e, &mut |x| {
                    if let yavs_sem::hir::ExprKind::Call { callee, .. } = &db.exprs[x].kind {
                        calls |= matches!(&db.decls[*callee].kind, DeclKind::Subprogram(s) if !matches!(s.body, Body::Implicit(_)) && !db.decls[*callee].implicit);
                    }
                });
            }
            let mut all = Vec::new();
            deep(db, body, &mut all);
            calls |= all
                .iter()
                .any(|&s| matches!(db.stmts[s].kind, StmtKind::ProcCall(_)));
            if !calls {
                for l in list.iter() {
                    if !c.reads.iter().any(|r| covers(l, r)) {
                        found.push((
                            "lint_002",
                            l.span,
                            format!(
                                "Signal `{}` is in the sensitivity list and this process does not read it",
                                show(db, l)
                            ),
                        ));
                    }
                }
            }
        }
        for (rule, span, m) in found {
            self.push(rule, span, m);
        }
    }

    /// `lint_600`: a signal a combinational process does not assign on every path. A signal
    /// assigned in part or in a loop is not judged.
    fn latches(&mut self) {
        let db = self.db;
        /// The whole signals every path through `list` assigns.
        fn every_path(db: &Db, list: &[StmtId], out: &mut HashSet<DeclId>) {
            for &s in list {
                let meet = |paths: Vec<&[StmtId]>, out: &mut HashSet<DeclId>| {
                    let mut all: Option<HashSet<DeclId>> = None;
                    for p in paths {
                        let mut one = HashSet::new();
                        every_path(db, p, &mut one);
                        all = Some(match all {
                            Some(a) => a.intersection(&one).copied().collect(),
                            None => one,
                        });
                    }
                    out.extend(all.unwrap_or_default());
                };
                match &db.stmts[s].kind {
                    StmtKind::SignalAssign { target, .. } => {
                        out.extend(target.lsps.iter().filter_map(|l| decl(l.root)));
                    }
                    StmtKind::If {
                        branches,
                        else_: Some(e),
                    } => {
                        let mut paths: Vec<&[StmtId]> =
                            branches.iter().map(|(_, b)| &b[..]).collect();
                        paths.push(e);
                        meet(paths, out);
                    }
                    StmtKind::Case { alts, .. } => {
                        meet(alts.iter().map(|(_, b)| &b[..]).collect(), out);
                    }
                    StmtKind::SeqBlock { body, .. } => every_path(db, body, out),
                    _ => {}
                }
            }
        }
        let mut found = Vec::new();
        for c in &self.conc {
            let Some(body) = c.process else { continue };
            if !matches!(shape(db, body), Shape::Combinational) {
                continue;
            }
            let mut assigned = HashSet::new();
            every_path(db, body, &mut assigned);
            let mut all = Vec::new();
            deep(db, body, &mut all);
            let mut in_loop = HashSet::new();
            for &s in &all {
                if let StmtKind::Loop { body, .. } = &db.stmts[s].kind {
                    let mut d = Vec::new();
                    db.collect_drivers(body, &mut d);
                    in_loop.extend(d.iter().filter_map(|l| decl(l.root)));
                }
            }
            let parts: HashSet<DeclId> = c
                .drivers
                .iter()
                .filter(|l| !l.path.is_empty())
                .filter_map(|l| decl(l.root))
                .collect();
            for l in &c.drivers {
                if let Some(d) = decl(l.root)
                    && !assigned.contains(&d)
                    && !in_loop.contains(&d)
                    && !parts.contains(&d)
                {
                    found.push((
                        l.span,
                        format!(
                            "Signal `{}` is not assigned on every path of this combinational process, which infers a latch",
                            name(db, d)
                        ),
                    ));
                }
            }
        }
        for (span, m) in found {
            self.push("lint_600", span, m);
        }
    }

    /// `lint_004`: a signal, constant or variable of this architecture nothing names. A
    /// declaration of a package can be used anywhere, and types and subprograms are not judged.
    fn unused(&mut self) {
        let db = self.db;
        let mut used = HashSet::new();
        let mut note = |e: ExprId| {
            db.walk_expr(e, &mut |x| {
                if let yavs_sem::hir::ExprKind::Obj(d) = db.exprs[x].kind {
                    used.insert(d);
                }
                if let Some(yavs_sem::lsp::NameRoot::Obj(r)) = db.name_root(x) {
                    used.extend(decl(r));
                }
            });
        };
        // Everything in the region tree of the unit is looked at once.
        let mut decls = Vec::new();
        let mut todo = self.scopes.clone();
        let mut seen = HashSet::new();
        while let Some(s) = todo.pop() {
            if !seen.insert(s) {
                continue;
            }
            for &d in &db.scopes[s].order {
                decls.push(d);
                match &db.decls[d].kind {
                    DeclKind::Object(o) => {
                        o.default.into_iter().for_each(&mut note);
                        for r in db.subtype_ranges(o.subtype) {
                            range_exprs(r).into_iter().for_each(&mut note);
                        }
                    }
                    DeclKind::Subprogram(sp) => {
                        if let Body::Hir { scope, stmts } = &sp.body {
                            todo.push(*scope);
                            for (e, _) in exprs_of(db, stmts) {
                                note(e);
                            }
                        }
                    }
                    DeclKind::Alias(yavs_sem::decl::Alias::Object { target, .. }) => note(*target),
                    DeclKind::Type(_) | DeclKind::Subtype(_) => {
                        for r in db.declared_ranges(d) {
                            range_exprs(r).into_iter().for_each(&mut note);
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut lsp_roots = Vec::new();
        for c in &self.conc {
            lsp_roots.extend(
                c.drivers
                    .iter()
                    .chain(&c.reads)
                    .filter_map(|l| decl(l.root)),
            );
            if let StmtKind::Process {
                sensitivity: Sensitivity::List(list),
                ..
            } = &db.stmts[c.stmt].kind
            {
                lsp_roots.extend(list.iter().filter_map(|l| decl(l.root)));
            }
        }
        let mut all = Vec::new();
        deep(db, self.stmts, &mut all);
        for s in all {
            db.stmt_exprs(s, &mut |e, _| note(e));
        }
        used.extend(lsp_roots);
        let mut found = Vec::new();
        for d in decls {
            if let DeclKind::Object(o) = &db.decls[d].kind
                && o.role == ObjRole::Plain
                && matches!(
                    o.class,
                    ObjClass::Signal | ObjClass::Constant | ObjClass::Variable
                )
                && !used.contains(&d)
                && !db.decls[d].implicit
            {
                found.push((
                    db.decls[d].name_span(),
                    format!("`{}` is declared and never used", name(db, d)),
                ));
            }
        }
        for (span, m) in found {
            self.push("lint_004", span, m);
        }
    }
}

fn range_exprs(r: &yavs_sem::types::RangeC) -> Vec<ExprId> {
    match r {
        yavs_sem::types::RangeC::Explicit { left, right, .. } => vec![*left, *right],
        yavs_sem::types::RangeC::Attr {
            prefix: yavs_sem::hir::AttrPrefix::Expr(e),
            ..
        } => vec![*e],
        yavs_sem::types::RangeC::Attr { .. } => Vec::new(),
    }
}

/// The value of an assignment to a state signal, as a literal when it is one.
fn values_of(db: &Db, s: StmtId) -> Option<(ExprId, Vec<ExprId>)> {
    fn wave(w: &Waveform, out: &mut Vec<ExprId>) {
        if let Waveform::Elements(e) = w {
            out.extend(e.iter().filter_map(|x| x.value));
        }
    }
    let (target, mut out) = (
        match &db.stmts[s].kind {
            StmtKind::SignalAssign { target, .. } | StmtKind::ConcSignalAssign { target, .. } => {
                target.expr
            }
            _ => return None,
        },
        Vec::new(),
    );
    if let StmtKind::SignalAssign { rhs, .. } | StmtKind::ConcSignalAssign { rhs, .. } =
        &db.stmts[s].kind
    {
        match rhs {
            Rhs::Simple(w) => wave(w, &mut out),
            Rhs::Conditional(a) => a.iter().for_each(|(w, _)| wave(w, &mut out)),
            Rhs::Selected { alts, .. } => alts.iter().for_each(|(w, _)| wave(w, &mut out)),
        }
    }
    Some((target, out))
}

impl Arch<'_> {
    /// `lint_702`, `lint_703`, `lint_700`, `lint_701`, `lint_602` and `lint_603`, and the clock and
    /// reset domains and crossings of the design facts.
    fn domains(&mut self, cfg: &Config) -> (Value, Value, Value) {
        let db = self.db;
        type Regs = HashMap<DeclId, Vec<usize>>;
        // clock/reset index per register; each clock or reset is one entry of `clocks`/`resets`.
        let (mut clocks, mut resets): (Vec<Lsp>, Vec<Lsp>) = (Vec::new(), Vec::new());
        let (mut on_clock, mut on_reset): (Regs, Regs) = Default::default();
        let mut edges: Vec<(bool, bool)> = Vec::new();
        let mut found: Vec<(&'static str, Span, String)> = Vec::new();
        let mut processes = Vec::new();
        for c in &self.conc {
            let Some(body) = c.process else { continue };
            let Shape::Clocked {
                clock,
                rising,
                reset,
                reset_body,
                body,
            } = shape(db, body)
            else {
                continue;
            };
            let index = |list: &mut Vec<Lsp>, l: &Lsp| {
                list.iter()
                    .position(|x| db.same_lsp(x, l))
                    .unwrap_or_else(|| {
                        list.push(l.clone());
                        list.len() - 1
                    })
            };
            let ci = index(&mut clocks, &clock);
            if edges.len() <= ci {
                edges.resize(ci + 1, (false, false));
            }
            let had = edges[ci];
            edges[ci] = if rising { (true, had.1) } else { (had.0, true) };
            if had != edges[ci] && edges[ci] == (true, true) {
                found.push((
                    "lint_703",
                    c.span,
                    format!(
                        "Clock `{}` is used on both edges in this architecture",
                        show(db, &clock)
                    ),
                ));
            }
            let ri = reset.as_ref().map(|r| index(&mut resets, r));
            for l in &c.drivers {
                let Some(d) = decl(l.root) else { continue };
                let v = on_clock.entry(d).or_default();
                if !v.contains(&ci) {
                    v.push(ci);
                }
                if let Some(ri) = ri {
                    let v = on_reset.entry(d).or_default();
                    if !v.contains(&ri) {
                        v.push(ri);
                    }
                }
                let DeclKind::Object(o) = &db.decls[d].kind else {
                    continue;
                };
                if o.role != ObjRole::Plain {
                    continue;
                }
                let n = name(db, d).to_ascii_lowercase();
                // ponytail: `*` in an affix is the whole-name glob, so `_p*` matches `x_p1` and
                // `x_p1_y` alike; anchor it per affix if that ever matters.
                for (rule, opt, fallback, front) in [
                    ("lint_602", "suffixes", &["_q", "_r", "_reg"][..], false),
                    ("lint_603", "prefixes", &["r_"][..], true),
                ] {
                    let listed = cfg.option_list(rule);
                    let affixes: Vec<String> = if listed.is_empty() {
                        fallback.iter().map(|s| s.to_string()).collect()
                    } else {
                        listed.iter().map(|s| s.to_ascii_lowercase()).collect()
                    };
                    let fits = affixes.iter().any(|a| {
                        let p = if front {
                            format!("{a}**")
                        } else {
                            format!("**{a}")
                        };
                        crate::config::glob(p.as_bytes(), n.as_bytes())
                    });
                    if !fits {
                        found.push((
                            rule,
                            l.span,
                            format!(
                                "Registered signal `{}` does not have the {} {}",
                                name(db, d),
                                &opt[..opt.len() - 2],
                                affixes.join(" or ")
                            ),
                        ));
                    }
                }
            }
            // `lint_702`
            if let Some(r) = &reset {
                let (mut clocked, mut cleared) = (Vec::new(), Vec::new());
                db.collect_drivers(body, &mut clocked);
                db.collect_drivers(reset_body, &mut cleared);
                let mut named = HashSet::new();
                for l in &clocked {
                    if let Some(d) = decl(l.root)
                        && !cleared.iter().any(|x| decl(x.root) == Some(d))
                        && named.insert(d)
                    {
                        found.push((
                            "lint_702",
                            l.span,
                            format!(
                                "Signal `{}` is assigned by this process but not by its `{}` branch",
                                name(db, d),
                                show(db, r)
                            ),
                        ));
                    }
                }
            }
            processes.push((ci, ri, body));
        }
        // Crossings: a read in logic of another domain. A capture into a flop and nothing else is
        // a synchroniser's first stage, so it does not count.
        let mut crossings = Vec::new();
        for (ci, ri, body) in processes {
            let mut all = Vec::new();
            deep(db, body, &mut all);
            for s in all {
                let mut reads = Vec::new();
                let capture = matches!(&db.stmts[s].kind,
                    StmtKind::SignalAssign { rhs: Rhs::Simple(Waveform::Elements(e)), .. }
                        if e.len() == 1 && e[0].value.is_some_and(|v| db.lsp(v).is_some()));
                if capture {
                    continue;
                }
                db.stmt_reads(s, &mut reads);
                for r in reads {
                    let Some(d) = decl(r.root) else { continue };
                    if clocks.iter().any(|c| decl(c.root) == Some(d))
                        || resets.iter().any(|c| decl(c.root) == Some(d))
                    {
                        continue;
                    }
                    if clocks.len() >= 2
                        && let Some([from]) = on_clock.get(&d).map(|v| &v[..])
                        && *from != ci
                    {
                        let (f, t) = (show(db, &clocks[*from]), show(db, &clocks[ci]));
                        crossings.push(json!({"kind": "clock", "signal": name(db, d), "from": f, "to": t, "line": line(db, r.span)}));
                        found.push((
                            "lint_700",
                            r.span,
                            format!(
                                "Signal `{}` is registered on `{f}` and used in logic on `{t}`: an unsynchronised clock domain crossing",
                                name(db, d)
                            ),
                        ));
                    }
                    if resets.len() >= 2
                        && let Some(own) = ri
                        && let Some([from]) = on_reset.get(&d).map(|v| &v[..])
                        && *from != own
                    {
                        let (f, t) = (show(db, &resets[*from]), show(db, &resets[own]));
                        crossings.push(json!({"kind": "reset", "signal": name(db, d), "from": f, "to": t, "line": line(db, r.span)}));
                        found.push((
                            "lint_701",
                            r.span,
                            format!(
                                "Signal `{}` is reset by `{f}` and used in logic reset by `{t}`",
                                name(db, d)
                            ),
                        ));
                    }
                }
            }
        }
        for (rule, span, m) in found {
            self.push(rule, span, m);
        }
        let domain = |key: &str, list: &[Lsp], regs: &Regs| {
            let mut out = Vec::new();
            for (i, l) in list.iter().enumerate() {
                let mut names: Vec<String> = regs
                    .iter()
                    .filter(|(_, v)| v.contains(&i))
                    .map(|(&d, _)| name(db, d))
                    .collect();
                names.sort();
                out.push(json!({key: show(db, l), "registers": names}));
            }
            Value::Array(out)
        };
        (
            domain("clock", &clocks, &on_clock),
            domain("reset", &resets, &on_reset),
            Value::Array(crossings),
        )
    }

    /// `lint_710` and `lint_711`: state machines of an enumeration this architecture declares,
    /// when every assignment to its signals is a plain value, and their design facts.
    fn machines(&mut self) -> Value {
        let db = self.db;
        let mut types: Vec<(TypeId, &[DeclId], Vec<DeclId>)> = Vec::new();
        for &s in &self.scopes {
            for &d in &db.scopes[s].order {
                match &db.decls[d].kind {
                    DeclKind::Type(t) => {
                        if let TypeKind::Enum { lits } = &db.types[*t].kind {
                            types.push((*t, &lits[..], Vec::new()));
                        }
                    }
                    DeclKind::Object(o)
                        if o.class == ObjClass::Signal && o.role == ObjRole::Plain =>
                    {
                        let base = db.subtypes[o.subtype].base;
                        if let Some(x) = types.iter_mut().find(|x| x.0 == base) {
                            x.2.push(d);
                        }
                    }
                    _ => {}
                }
            }
        }
        let mut all = Vec::new();
        for c in &self.conc {
            all.push(c.stmt);
            if let Some(b) = c.process {
                deep(db, b, &mut all);
            }
        }
        let named = self.named_in_subprograms();
        let mut facts = Vec::new();
        let mut found = Vec::new();
        for (ty, lits, signals) in types {
            if signals.is_empty() || signals.iter().any(|d| named.contains(d)) {
                continue;
            }
            let of_type = |e: ExprId| match db.exprs[e].kind {
                yavs_sem::hir::ExprKind::Obj(d) => signals.contains(&d),
                _ => false,
            };
            let pos = |e: ExprId| match db.value(e) {
                Some(ConstValue::Enum(p)) => Some(*p as usize),
                _ => None,
            };
            // Entered by an initial value, or by default its first literal.
            let mut entered: HashSet<usize> = HashSet::new();
            for d in &signals {
                if let DeclKind::Object(o) = &db.decls[*d].kind {
                    match o.default.map(pos) {
                        Some(Some(p)) => {
                            entered.insert(p);
                        }
                        None => {
                            entered.insert(0);
                        }
                        Some(None) => {}
                    }
                }
            }
            let mut readable = true;
            for &s in &all {
                if let Some((t, values)) = values_of(db, s)
                    && of_type(t)
                {
                    for v in values {
                        match pos(v) {
                            Some(p) => {
                                entered.insert(p);
                            }
                            None => readable = false,
                        }
                    }
                }
            }
            // A driver that is not an assignment (a port map, a parameter) says nothing readable.
            for c in &self.conc {
                if matches!(db.stmts[c.stmt].kind, StmtKind::Instance { .. })
                    && c.drivers
                        .iter()
                        .any(|l| decl(l.root).is_some_and(|d| signals.contains(&d)))
                {
                    readable = false;
                }
            }
            for &s in &all {
                if matches!(
                    db.stmts[s].kind,
                    StmtKind::ProcCall(_) | StmtKind::ConcProcCall { .. }
                ) {
                    let mut d = Vec::new();
                    db.collect_drivers(&[s], &mut d);
                    if d.iter()
                        .any(|l| decl(l.root).is_some_and(|d| signals.contains(&d)))
                    {
                        readable = false;
                    }
                }
            }
            if !readable {
                continue;
            }
            // The cases on a signal of the type, in the statements of their process.
            let mut transitions: Vec<(usize, usize)> = Vec::new();
            let mut cased = false;
            for c in &self.conc {
                let Some(body) = c.process else { continue };
                self.cases(body, false, &of_type, &pos, &mut |alts| {
                    cased = true;
                    let assigns_any = alts.iter().any(|a| !a.1.is_empty());
                    if !assigns_any {
                        return;
                    }
                    for (state, to, span) in alts {
                        for t in to {
                            transitions.push((*state, *t));
                        }
                        if !to.iter().any(|t| t != state) {
                            found.push((
                                "lint_711",
                                *span,
                                format!(
                                    "State `{}` of `{}` has no exit: its alternative never assigns another state",
                                    name(db, lits[*state]),
                                    name(db, db.types[ty].decl)
                                ),
                            ));
                        }
                    }
                });
            }
            if !cased {
                continue;
            }
            for (i, &l) in lits.iter().enumerate() {
                if !entered.contains(&i) {
                    found.push((
                        "lint_710",
                        db.decls[l].name_span(),
                        format!(
                            "State `{}` of `{}` is never entered: nothing assigns it",
                            name(db, l),
                            name(db, db.types[ty].decl)
                        ),
                    ));
                }
            }
            transitions.sort_unstable();
            transitions.dedup();
            facts.push(json!({
                "type": name(db, db.types[ty].decl),
                "signals": signals.iter().map(|&d| name(db, d)).collect::<Vec<_>>(),
                "states": lits.iter().map(|&d| name(db, d)).collect::<Vec<_>>(),
                "transitions": transitions.iter().map(|(a, b)| json!({"from": name(db, lits[*a]), "to": name(db, lits[*b])})).collect::<Vec<_>>(),
                "reset_state": signals.first().and_then(|d| match &db.decls[*d].kind {
                    DeclKind::Object(o) => o.default.and_then(pos).map(|p| name(db, lits[p])),
                    _ => None,
                }),
            }));
        }
        for (rule, span, m) in found {
            self.push(rule, span, m);
        }
        Value::Array(facts)
    }

    /// Calls `f` with the alternatives of each `case` on a signal of the type below `list`: the
    /// state, the literals assigned under it and where. A case after a default assignment, or with
    /// a value that does not fold, is left out: its alternatives do not say where the machine goes.
    #[allow(clippy::type_complexity)]
    fn cases(
        &self,
        list: &[StmtId],
        mut defaulted: bool,
        of_type: &dyn Fn(ExprId) -> bool,
        pos: &dyn Fn(ExprId) -> Option<usize>,
        f: &mut dyn FnMut(&[(usize, Vec<usize>, Span)]),
    ) {
        let db = self.db;
        let assigns = |s: StmtId| {
            let mut v = Vec::new();
            deep(db, std::slice::from_ref(&s), &mut v);
            v.into_iter()
                .filter_map(|x| values_of(db, x))
                .filter(|(t, _)| of_type(*t))
                .flat_map(|(_, v)| v)
                .collect::<Vec<_>>()
        };
        for &s in list {
            match &db.stmts[s].kind {
                StmtKind::Case { expr, alts, .. } if of_type(*expr) => {
                    if !defaulted {
                        let mut rows = Vec::new();
                        let mut certain = true;
                        for (set, body) in alts {
                            let to: Vec<ExprId> = body.iter().flat_map(|&b| assigns(b)).collect();
                            let to: Option<Vec<usize>> = to.iter().map(|&e| pos(e)).collect();
                            let Some(to) = to else {
                                certain = false;
                                continue;
                            };
                            let single = match set.intervals.as_deref() {
                                Some([(a, b)]) if a == b => Some(*a as usize),
                                _ => None,
                            };
                            if let Some(state) = single {
                                let span =
                                    body.first().map_or(db.stmts[s].span, |&b| db.stmts[b].span);
                                rows.push((state, to, span));
                            }
                        }
                        if certain {
                            f(&rows);
                        }
                    }
                    defaulted |= !assigns(s).is_empty();
                }
                StmtKind::If { branches, else_ } => {
                    for (_, b) in branches {
                        self.cases(b, defaulted, of_type, pos, f);
                    }
                    if let Some(e) = else_ {
                        self.cases(e, defaulted, of_type, pos, f);
                    }
                }
                StmtKind::Loop { body, .. } | StmtKind::SeqBlock { body, .. } => {
                    self.cases(body, defaulted, of_type, pos, f);
                }
                StmtKind::Case { alts, .. } => {
                    for (_, b) in alts {
                        self.cases(b, defaulted, of_type, pos, f);
                    }
                }
                _ => defaulted |= !assigns(s).is_empty(),
            }
        }
    }
}

/// `lint_750`: a component that does not match the entity of its name in the library, when that
/// entity is analyzed in this run.
pub(crate) fn component(db: &Db, d: DeclId, lib: yavs_sem::LibId, out: &mut Vec<Raw>) {
    let DeclKind::Component(c) = &db.decls[d].kind else {
        return;
    };
    let key = db.decls[d].name.key();
    let Some(yavs_sem::loader::UnitSlot::Done(u)) = db.libs[lib].units.get(&UnitKey::Primary(key))
    else {
        return;
    };
    let UnitHir::Entity { ports, .. } = &db.units[*u].hir else {
        return;
    };
    let port = |p: &DeclId| {
        let mode = match &db.decls[*p].kind {
            DeclKind::Object(o) => o.mode,
            _ => None,
        };
        (db.decls[*p].name.key(), name(db, *p), mode)
    };
    let (ours, theirs): (Vec<_>, Vec<_>) = (
        c.ports.iter().map(port).collect(),
        ports.iter().map(port).collect(),
    );
    let mut why = Vec::new();
    let missing: Vec<&str> = theirs
        .iter()
        .filter(|t| !ours.iter().any(|o| o.0 == t.0))
        .map(|t| t.1.as_str())
        .collect();
    if !missing.is_empty() {
        why.push(format!("it does not declare {}", missing.join(", ")));
    }
    let extra: Vec<&str> = ours
        .iter()
        .filter(|o| !theirs.iter().any(|t| t.0 == o.0))
        .map(|o| o.1.as_str())
        .collect();
    if !extra.is_empty() {
        why.push(format!(
            "it declares {}, which the entity does not have",
            extra.join(", ")
        ));
    }
    let moded: Vec<&str> = ours
        .iter()
        .filter(|o| theirs.iter().any(|t| t.0 == o.0 && t.2 != o.2))
        .map(|o| o.1.as_str())
        .collect();
    if !moded.is_empty() {
        why.push(format!("it gives a different mode to {}", moded.join(", ")));
    }
    if why.is_empty() && !ours.iter().map(|x| x.0).eq(theirs.iter().map(|x| x.0)) {
        why.push("it declares the ports in a different order".to_string());
    }
    if !why.is_empty() {
        out.push((
            "lint_750",
            db.decls[d].name_span(),
            format!(
                "Component `{}` does not match entity `{}`: {}",
                name(db, d),
                name(db, d),
                why.join("; ")
            ),
        ));
    }
}
