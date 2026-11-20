//! The rules about one architecture as a design: drivers, reads, clocks, resets and state machines,
//! and the design facts report. Sem says what a statement drives and reads (`ProcessInfo`,
//! `Db::stmt_reads`, `Db::edge_shape`), so nothing here reads names from text. A process is
//! judged only in a shape that says what it is; anything else is left alone.

pub(crate) mod fsm;

use crate::Config;
use crate::rules::{Raw, Related};
use serde_json::{Value, json};
use std::collections::{BTreeSet, HashMap, HashSet};
use yavs_diag::Span;
use yavs_sem::attr::PredefAttr;
use yavs_sem::decl::{Body, DeclKind, ObjClass, ObjRole, SubprogramKind};
use yavs_sem::hir::{
    Actual, AttrPrefix, Choice, ConstValue, ExprKind, LoopScheme, Lsp, Mode, ObjRoot, Rhs,
    Sensitivity, StmtKind, UnitHir, Waveform,
};
use yavs_sem::loader::UnitKey;
use yavs_sem::lsp::{Overlap, Use};
use yavs_sem::types::{RangeC, TypeKind};
use yavs_sem::{Db, DeclId, ExprId, ScopeId, StmtId, UnitId};

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

/// Whether `e` asks about a change in time (`a'event`, `a'stable`): no combinational logic does.
fn tests_time(db: &Db, e: ExprId) -> bool {
    let mut found = false;
    db.walk_expr(e, &mut |x| {
        found |= matches!(
            &db.exprs[x].kind,
            ExprKind::Attr {
                attr: PredefAttr::Event
                    | PredefAttr::Active
                    | PredefAttr::LastEvent
                    | PredefAttr::LastActive
                    | PredefAttr::Stable
                    | PredefAttr::Quiet
                    | PredefAttr::Delayed
                    | PredefAttr::Transaction,
                ..
            }
        );
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
    let exprs = exprs_of(db, body);
    if !exprs.iter().any(|(e, _)| edge_in(db, *e).is_some()) {
        return if exprs.iter().any(|(e, _)| tests_time(db, *e)) {
            Shape::Other
        } else {
            Shape::Combinational
        };
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

/// Whether a condition reads no signal or variable and calls nothing impure: analysis may not have
/// folded it (a generic), but it cannot change while the design runs.
fn before_the_run(db: &Db, c: ExprId) -> bool {
    let mut fixed = true;
    db.walk_expr(c, &mut |x| match &db.exprs[x].kind {
        ExprKind::Obj(d) => {
            fixed &=
                matches!(&db.decls[*d].kind, DeclKind::Object(o) if o.class == ObjClass::Constant);
        }
        ExprKind::Call { callee, .. } => {
            fixed &= matches!(
                db.decls[*callee].kind.subprogram().map(|s| &s.kind),
                Some(SubprogramKind::Function { pure: true })
            );
        }
        ExprKind::External(_) | ExprKind::Deref { .. } => fixed = false,
        _ => {}
    });
    fixed
}

/// The prefixes of attributes under `x` itself, in a slice's or a choice's range too (`v'range`):
/// names there give a property of the object, not its value.
fn attribute_prefixes(db: &Db, x: ExprId, out: &mut HashSet<ExprId>) {
    let mut range = |r: &RangeC| {
        if let RangeC::Attr {
            prefix: AttrPrefix::Expr(p),
            ..
        } = r
        {
            out.insert(*p);
        }
    };
    match &db.exprs[x].kind {
        ExprKind::Slice { range: r, .. } => range(r),
        ExprKind::Aggregate { assocs, .. } => {
            for c in assocs.iter().flat_map(|a| a.choices.iter()) {
                if let Choice::Range(r) = c {
                    range(r);
                }
            }
        }
        ExprKind::Attr {
            prefix: AttrPrefix::Expr(p),
            ..
        } => {
            out.insert(*p);
        }
        _ => {}
    }
}

/// The variables with no initial value that an expression reads as a value (`v'length` does not).
fn variables_read(db: &Db, e: ExprId, out: &mut Vec<(DeclId, Span)>) {
    let mut attr_prefix = HashSet::new();
    db.walk_expr(e, &mut |x| {
        attribute_prefixes(db, x, &mut attr_prefix);
        if let ExprKind::Obj(d) = &db.exprs[x].kind
            && !attr_prefix.contains(&x)
            && let DeclKind::Object(o) = &db.decls[*d].kind
            && o.class == ObjClass::Variable
            && o.role == ObjRole::Plain
            && o.default.is_none()
            && !matches!(
                db.types[db.base(o.subtype)].kind,
                TypeKind::Access { .. } | TypeKind::Protected { .. } | TypeKind::File { .. }
            )
        {
            out.push((*d, db.exprs[x].span));
        }
    });
}

/// `lint_600` of variables: walks `list` with what every path so far assigned, and notes the first
/// read of a variable some path has not assigned. Part of a variable counts as all of it: filling
/// `v(0)` and reading it in the next iteration is ordinary code, and proving it needs more.
fn kept_variables(
    db: &Db,
    list: &[StmtId],
    assigned: &mut HashSet<DeclId>,
    out: &mut HashMap<DeclId, Span>,
) {
    let meet = |paths: Vec<&[StmtId]>, assigned: &mut HashSet<DeclId>, out: &mut _| {
        let mut all: Option<HashSet<DeclId>> = None;
        for p in paths {
            let mut one = assigned.clone();
            kept_variables(db, p, &mut one, out);
            all = Some(match all {
                Some(a) => a.intersection(&one).copied().collect(),
                None => one,
            });
        }
        all
    };
    for &s in list {
        let mut exprs = Vec::new();
        db.stmt_exprs(s, &mut |e, u| {
            if u == Use::Value {
                exprs.push(e);
            }
        });
        let mut reads = Vec::new();
        exprs
            .iter()
            .for_each(|e| variables_read(db, *e, &mut reads));
        for (d, span) in reads {
            if !assigned.contains(&d) {
                let at = out.entry(d).or_insert(span);
                if span.start < at.start {
                    *at = span;
                }
            }
        }
        match &db.stmts[s].kind {
            StmtKind::VarAssign { target, .. } => match db.lsp(target.expr) {
                Some(l) => assigned.extend(decl(l.root)),
                None => db.walk_expr(target.expr, &mut |x| {
                    if let ExprKind::Obj(d) = db.exprs[x].kind {
                        assigned.insert(d);
                    }
                }),
            },
            StmtKind::If { branches, else_ } => {
                // Each condition is read before its branch, and the reads above took them all.
                let mut paths: Vec<&[StmtId]> = branches.iter().map(|(_, b)| &b[..]).collect();
                // A condition no signal decides leaves no path that skips the branch.
                let whole = else_.is_some() || branches.iter().all(|(c, _)| before_the_run(db, *c));
                paths.extend(else_.iter().map(|e| &e[..]));
                if let Some(all) = meet(paths, assigned, out)
                    && whole
                {
                    *assigned = all;
                }
            }
            StmtKind::Case { alts, .. } => {
                if let Some(all) = meet(alts.iter().map(|(_, b)| &b[..]).collect(), assigned, out) {
                    *assigned = all;
                }
            }
            // A `for` loop runs and is unrolled; a `while` loop may not run at all.
            StmtKind::Loop { scheme, body, .. } => {
                let mut one = assigned.clone();
                kept_variables(db, body, &mut one, &mut HashMap::new());
                // An iteration may read what an earlier one assigned, whichever branch that took.
                let mut inside = assigned.clone();
                let mut stmts = Vec::new();
                deep(db, body, &mut stmts);
                for &x in &stmts {
                    if let StmtKind::VarAssign { target, .. } = &db.stmts[x].kind {
                        inside.extend(db.lsp(target.expr).and_then(|l| decl(l.root)));
                    }
                }
                kept_variables(db, body, &mut inside, out);
                if matches!(scheme, LoopScheme::For { .. }) {
                    *assigned = one;
                }
            }
            StmtKind::SeqBlock { body, .. } => kept_variables(db, body, assigned, out),
            _ => {}
        }
    }
}

/// `lint_720`: for each signal, the signals assigned from it.
type Edges = HashMap<DeclId, BTreeSet<DeclId>>;

/// `lint_720`: what a variable holds, from the assignment to all of it and from those to its parts.
type Vars = HashMap<DeclId, [BTreeSet<DeclId>; 2]>;

/// `lint_720` in one combinational process: a target depends on what its statement reads, on the
/// conditions around it, and on variables through what they hold at that point. `exit` and `next`
/// add no condition to the statements after them, so a loop only through one is not found. Parts
/// assigned after the whole (`v := r; v.x := a;`) may leave nothing of it, so it is then forgotten.
struct Flow<'a> {
    db: &'a Db,
    vars: Vars,
    control: Vec<DeclId>,
    edges: &'a mut Edges,
    places: &'a mut HashMap<DeclId, Span>,
}

impl Flow<'_> {
    /// A loop runs again, and a variable keeps its value to the next activation of the process, so
    /// run until nothing more is learnt.
    fn repeat(&mut self, body: &[StmtId]) {
        loop {
            let before = self.vars.clone();
            self.list(body);
            merge(&mut self.vars, &before);
            if self.vars == before {
                return;
            }
        }
    }

    /// The whole objects `e` reads: signals, and what the variables it names hold. An attribute,
    /// or a part of a variable, gives nothing, as for a part of a signal.
    fn expr(&self, e: ExprId, out: &mut BTreeSet<DeclId>) {
        let db = self.db;
        let mut reads = Vec::new();
        db.reads(e, &mut reads);
        out.extend(
            reads
                .iter()
                .filter(|l| l.path.is_empty())
                .filter_map(|l| decl(l.root)),
        );
        let (mut names, mut prefixes) = (Vec::new(), HashSet::new());
        db.walk_expr(e, &mut |x| {
            attribute_prefixes(db, x, &mut prefixes);
            match &db.exprs[x].kind {
                ExprKind::Obj(_) => names.push(x),
                ExprKind::Index { prefix, .. }
                | ExprKind::Slice { prefix, .. }
                | ExprKind::Field { prefix, .. } => {
                    prefixes.insert(*prefix);
                }
                _ => {}
            }
        });
        for x in names {
            if !prefixes.contains(&x)
                && let Some(l) = db.lsp(x)
                && l.path.is_empty()
                && let Some(held) = decl(l.root).and_then(|d| self.vars.get(&d))
            {
                out.extend(held.iter().flatten());
            }
        }
    }

    fn stmt(&self, s: StmtId) -> BTreeSet<DeclId> {
        let mut out: BTreeSet<DeclId> = self.control.iter().copied().collect();
        self.db.stmt_exprs(s, &mut |e, u| {
            if u == Use::Value {
                self.expr(e, &mut out);
            }
        });
        out
    }

    fn under(&mut self, conds: BTreeSet<DeclId>, body: &[StmtId], repeat: bool) {
        let mark = self.control.len();
        self.control.extend(conds);
        if repeat {
            self.repeat(body);
        } else {
            self.list(body);
        }
        self.control.truncate(mark);
    }

    fn list(&mut self, body: &[StmtId]) {
        let db = self.db;
        for &s in body {
            match &db.stmts[s].kind {
                StmtKind::SignalAssign { target, rhs, .. } if !delayed(rhs) => {
                    let reads = self.stmt(s);
                    for l in target.lsps.iter().filter(|l| l.path.is_empty()) {
                        let Some(t) = decl(l.root) else { continue };
                        self.places.entry(t).or_insert(db.stmts[s].span);
                        for &r in &reads {
                            self.edges.entry(r).or_default().insert(t);
                        }
                    }
                }
                StmtKind::VarAssign { target, .. } => {
                    let reads = self.stmt(s);
                    // An aggregate target names several: each keeps what its other parts hold.
                    let mut named = Vec::new();
                    match db.lsp(target.expr) {
                        Some(l) => named.push((decl(l.root), l.path.is_empty())),
                        None => db.walk_expr(target.expr, &mut |x| {
                            if let ExprKind::Obj(_) = db.exprs[x].kind {
                                named.push((db.lsp(x).and_then(|l| decl(l.root)), false));
                            }
                        }),
                    }
                    for (v, whole) in named {
                        let Some(v) = v else { continue };
                        let [all, parts] = self.vars.entry(v).or_default();
                        all.clear();
                        if whole {
                            parts.clear();
                            all.extend(&reads);
                        } else {
                            parts.extend(&reads);
                        }
                    }
                }
                StmtKind::If { branches, else_ } => {
                    let start = self.vars.clone();
                    let mark = self.control.len();
                    let mut joined = HashMap::new();
                    for (c, then) in branches.iter() {
                        let mut conds = BTreeSet::new();
                        self.expr(*c, &mut conds);
                        // Each later branch is also decided by the conditions before it.
                        self.control.extend(conds);
                        self.vars = start.clone();
                        self.list(then);
                        merge(&mut joined, &self.vars);
                    }
                    self.vars = start;
                    if let Some(e) = else_ {
                        self.list(e);
                    }
                    merge(&mut self.vars, &joined);
                    self.control.truncate(mark);
                }
                StmtKind::Case { alts, .. } => {
                    let conds = self.stmt(s);
                    let start = self.vars.clone();
                    let mut joined = HashMap::new();
                    for (_, then) in alts.iter() {
                        self.vars = start.clone();
                        self.under(conds.clone(), then, false);
                        merge(&mut joined, &self.vars);
                    }
                    self.vars = joined;
                }
                StmtKind::Loop { body, .. } => self.under(self.stmt(s), body, true),
                StmtKind::SeqBlock { body, .. } => self.list(body),
                _ => {}
            }
        }
    }
}

fn merge(into: &mut Vars, from: &Vars) {
    for (v, [all, parts]) in from {
        let held = into.entry(*v).or_default();
        held[0].extend(all);
        held[1].extend(parts);
    }
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

fn name(db: &Db, d: DeclId) -> String {
    db.decls[d].name.to_string()
}

fn line(db: &Db, span: Span) -> usize {
    db.sources.get(span.file).line_col(span.start).0
}

fn span_text(db: &Db, span: Span) -> String {
    let file = db.sources.get(span.file);
    file.try_text()
        .ok()
        .and_then(|t| t.get(file.text_offset(span.start)..file.text_offset(span.end)))
        .unwrap_or_default()
        .to_string()
}

fn show(db: &Db, l: &Lsp) -> String {
    let text = span_text(db, l.span);
    if text.is_empty() {
        decl(l.root).map_or_else(String::new, |d| name(db, d))
    } else {
        text
    }
}

struct Arch<'a> {
    db: &'a Db,
    stmts: &'a [StmtId],
    conc: Vec<Conc<'a>>,
    scopes: Vec<ScopeId>,
    out: &'a mut Vec<Raw>,
    related: &'a mut Related,
}

/// The rules of one architecture. Returns its design facts.
pub(crate) fn architecture(
    db: &Db,
    u: UnitId,
    cfg: &Config,
    out: &mut Vec<Raw>,
    related: &mut Related,
) -> Value {
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
        related,
    };
    a.drivers();
    a.undriven();
    a.loops();
    a.sensitivity();
    a.latches();
    a.unused();
    a.over_wide();
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
                found.push((
                    first.1,
                    format!(
                        "Signal `{}` is assigned by {} concurrent statements",
                        name(db, d),
                        lines.len(),
                    ),
                    lines.iter().map(|l| l.1).collect::<Vec<_>>(),
                ));
            }
        }
        for (span, m, others) in found {
            self.related.insert((span, m.clone()), others);
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
    /// See `Flow` for what a statement of a process depends on.
    fn loops(&mut self) {
        let db = self.db;
        let mut edges: Edges = HashMap::new();
        let mut places: HashMap<DeclId, Span> = HashMap::new();
        for c in &self.conc {
            if let Some(body) = c.process {
                if matches!(shape(db, body), Shape::Combinational) {
                    Flow {
                        db,
                        vars: HashMap::new(),
                        control: Vec::new(),
                        edges: &mut edges,
                        places: &mut places,
                    }
                    .repeat(body);
                }
                continue;
            }
            // `q <= d when rising_edge (clk)` is a register written as an assignment.
            if !matches!(db.stmts[c.stmt].kind, StmtKind::ConcSignalAssign { .. })
                || c.delayed
                || exprs_of(db, &[c.stmt])
                    .iter()
                    .any(|(e, _)| edge_in(db, *e).is_some() || tests_time(db, *e))
            {
                continue;
            }
            for t in c.drivers.iter().filter(|l| l.path.is_empty()) {
                let Some(t) = decl(t.root) else { continue };
                places.entry(t).or_insert(c.span);
                for r in c.reads.iter().filter(|l| l.path.is_empty()) {
                    if let Some(r) = decl(r.root) {
                        edges.entry(r).or_default().insert(t);
                    }
                }
            }
        }
        // Depth first, one cycle per back edge found; 0 unseen, 1 on the path, 2 done.
        fn visit(
            n: DeclId,
            edges: &Edges,
            state: &mut HashMap<DeclId, u8>,
            path: &mut Vec<DeclId>,
            cycles: &mut Vec<Vec<DeclId>>,
        ) {
            state.insert(n, 1);
            path.push(n);
            for &m in edges.get(&n).into_iter().flatten() {
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
            let m = format!(
                "Combinational loop: {} depends on itself with no register in the way",
                names.join(" -> ")
            );
            let on_it = cycle
                .iter()
                .filter_map(|d| places.get(d))
                .copied()
                .collect();
            self.related.insert((span, m.clone()), on_it);
            self.push("lint_720", span, m);
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
                    // A generic decides whether the branch exists: no path of the design skips it.
                    StmtKind::If {
                        branches,
                        else_: None,
                    } if branches.iter().all(|(c, _)| before_the_run(db, *c)) => {
                        meet(branches.iter().map(|(_, b)| &b[..]).collect(), out);
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
            let mut read_first = HashMap::new();
            kept_variables(db, body, &mut HashSet::new(), &mut read_first);
            for (d, span) in read_first {
                found.push((
                    span,
                    format!(
                        "Variable `{}` is read on a path that has not assigned it: it keeps its value from the last run of this combinational process, which infers a latch",
                        name(db, d)
                    ),
                ));
            }
        }
        for (span, m) in found {
            self.push("lint_600", span, m);
        }
    }

    /// `lint_790`: a signal used only inside one block or generate body below its declaration. A
    /// signal with an attribute specification stays: moving it means moving that too.
    fn over_wide(&mut self) {
        let db = self.db;
        let mut uses = Uses::default();
        uses.regions.push(Region {
            parent: None,
            name: String::new(),
            per_iteration: false,
        });
        uses.scope(db, self.scopes[0], 0, true);
        uses.region(db, self.stmts, 0);
        let decorated: HashSet<DeclId> = db.attr_index.iter().map(|((d, _), _)| *d).collect();
        let mut found = Vec::new();
        for (d, at) in &uses.declared {
            let Some(&used) = uses.used.get(d) else {
                continue;
            };
            if used != *at && !decorated.contains(d) {
                let region = &uses.regions[used];
                let per = if region.per_iteration {
                    ": declared there, each iteration has one of its own instead of all of them sharing it"
                } else {
                    ""
                };
                found.push((
                    db.decls[*d].name_span(),
                    format!(
                        "Signal `{}` is only used inside {}{per}",
                        name(db, *d),
                        region.name
                    ),
                ));
            }
        }
        for (span, m) in found {
            self.push("lint_790", span, m);
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

fn range_exprs(r: &RangeC) -> Vec<ExprId> {
    match r {
        RangeC::Explicit { left, right, .. } => vec![*left, *right],
        RangeC::Attr {
            prefix: yavs_sem::hir::AttrPrefix::Expr(e),
            ..
        }
        | RangeC::Record(e) => vec![*e],
        RangeC::Attr { .. } => Vec::new(),
    }
}

/// A region a signal can be declared in, for `lint_790`: the architecture, a block, a generate body.
struct Region {
    parent: Option<usize>,
    /// As a reader names it: `generate `g``.
    name: String,
    per_iteration: bool,
}

/// Where each signal of an architecture is declared and used, by the regions around the uses.
#[derive(Default)]
struct Uses {
    regions: Vec<Region>,
    declared: Vec<(DeclId, usize)>,
    /// The innermost region that holds every use.
    used: HashMap<DeclId, usize>,
}

impl Uses {
    fn chain(&self, mut r: usize) -> Vec<usize> {
        let mut out = vec![r];
        while let Some(p) = self.regions[r].parent {
            out.push(p);
            r = p;
        }
        out
    }

    fn mark(&mut self, d: DeclId, at: usize) {
        let joined = match self.used.get(&d) {
            None => at,
            Some(&was) => {
                let theirs = self.chain(was);
                self.chain(at)
                    .into_iter()
                    .find(|r| theirs.contains(r))
                    .expect("every chain ends in the architecture")
            }
        };
        self.used.insert(d, joined);
    }

    fn expr(&mut self, db: &Db, e: ExprId, at: usize) {
        let mut roots = Vec::new();
        db.walk_expr(e, &mut |x| {
            if let Some(yavs_sem::lsp::NameRoot::Obj(r)) = db.name_root(x) {
                roots.extend(decl(r));
            }
        });
        for d in roots {
            self.mark(d, at);
        }
    }

    /// The declarations of `scope` use what their values and ranges name, at `at`. `signals`: the
    /// region's own signals are candidates; a process or subprogram declares none.
    fn scope(&mut self, db: &Db, scope: ScopeId, at: usize, signals: bool) {
        for &d in &db.scopes[scope].order {
            match &db.decls[d].kind {
                DeclKind::Object(o) => {
                    if signals && o.class == ObjClass::Signal && o.role == ObjRole::Plain {
                        self.declared.push((d, at));
                    }
                    if let Some(e) = o.default {
                        self.expr(db, e, at);
                    }
                    for r in db.subtype_ranges(o.subtype) {
                        for e in range_exprs(r) {
                            self.expr(db, e, at);
                        }
                    }
                }
                DeclKind::Type(_) | DeclKind::Subtype(_) => {
                    for r in db.declared_ranges(d) {
                        for e in range_exprs(r) {
                            self.expr(db, e, at);
                        }
                    }
                }
                DeclKind::Alias(yavs_sem::decl::Alias::Object { target, .. }) => {
                    self.expr(db, *target, at);
                }
                DeclKind::Subprogram(sp) => {
                    if let Body::Hir { scope, stmts } = &sp.body {
                        self.scope(db, *scope, at, false);
                        self.statements(db, stmts, at);
                    }
                }
                _ => {}
            }
        }
    }

    /// Sequential statements, or a process and what is below it: all at `at`.
    fn statements(&mut self, db: &Db, list: &[StmtId], at: usize) {
        let mut all = Vec::new();
        deep(db, list, &mut all);
        for s in all {
            let mut exprs = Vec::new();
            db.stmt_exprs(s, &mut |e, _| exprs.push(e));
            for e in exprs {
                self.expr(db, e, at);
            }
            let mut reads = Vec::new();
            db.stmt_reads(s, &mut reads);
            for d in reads.iter().filter_map(|l| decl(l.root)) {
                self.mark(d, at);
            }
            match &db.stmts[s].kind {
                StmtKind::Process {
                    sensitivity, scope, ..
                } => {
                    self.scope(db, *scope, at, false);
                    if let Sensitivity::List(l) = sensitivity {
                        for d in l.iter().filter_map(|l| decl(l.root)) {
                            self.mark(d, at);
                        }
                    }
                }
                StmtKind::SeqBlock { scope, .. } => self.scope(db, *scope, at, false),
                _ => {}
            }
        }
    }

    fn open(&mut self, name: String, per_iteration: bool, parent: usize) -> usize {
        self.regions.push(Region {
            parent: Some(parent),
            name,
            per_iteration,
        });
        self.regions.len() - 1
    }

    /// The statements of region `at`: a block's guard and maps, and a generate's header, are read
    /// in the region around it.
    fn region(&mut self, db: &Db, list: &[StmtId], at: usize) {
        for &s in list {
            let stmt = &db.stmts[s];
            let label = stmt
                .label
                .map_or(String::new(), |l| format!(" `{}`", l.text));
            let mut exprs = Vec::new();
            db.stmt_exprs(s, &mut |e, _| exprs.push(e));
            match &stmt.kind {
                StmtKind::Block {
                    scope,
                    guard,
                    generic_map,
                    port_map,
                    body,
                    ..
                } => {
                    if let Some(g) = guard
                        && let DeclKind::Object(o) = &db.decls[*g].kind
                    {
                        exprs.extend(o.default);
                    }
                    for a in generic_map.iter().chain(port_map.iter()) {
                        match &a.actual {
                            Actual::Expr(e) | Actual::Converted { actual: e, .. } => exprs.push(*e),
                            Actual::Individual(parts) => exprs.extend(parts.iter().map(|p| p.1)),
                            _ => {}
                        }
                    }
                    for e in exprs {
                        self.expr(db, e, at);
                    }
                    let r = self.open(format!("block{label}"), false, at);
                    self.scope(db, *scope, r, true);
                    self.region(db, body, r);
                }
                StmtKind::ForGenerate { scope, body, .. } => {
                    for e in exprs {
                        self.expr(db, e, at);
                    }
                    let r = self.open(format!("generate{label}"), true, at);
                    self.scope(db, *scope, r, true);
                    self.region(db, body, r);
                }
                StmtKind::IfGenerate { alts } => {
                    for e in exprs {
                        self.expr(db, e, at);
                    }
                    for (i, alt) in alts.iter().enumerate() {
                        let name = if alt.cond.is_none() {
                            format!("the else branch of generate{label}")
                        } else if i > 0 {
                            format!("an elsif branch of generate{label}")
                        } else {
                            format!("generate{label}")
                        };
                        let r = self.open(name, false, at);
                        self.scope(db, alt.scope, r, true);
                        self.region(db, &alt.body, r);
                    }
                }
                StmtKind::CaseGenerate { alts, .. } => {
                    for e in exprs {
                        self.expr(db, e, at);
                    }
                    for (choices, alt) in alts {
                        let shown: Vec<String> = choices
                            .choices
                            .iter()
                            .map(|c| match c {
                                Choice::Expr(e) => span_text(db, db.exprs[*e].span),
                                Choice::Range(RangeC::Explicit { left, right, .. }) => format!(
                                    "{} .. {}",
                                    span_text(db, db.exprs[*left].span),
                                    span_text(db, db.exprs[*right].span)
                                ),
                                Choice::Others => "others".to_string(),
                                _ => "...".to_string(),
                            })
                            .collect();
                        let name =
                            format!("the `when {}` branch of generate{label}", shown.join(" | "));
                        let r = self.open(name, false, at);
                        self.scope(db, alt.scope, r, true);
                        self.region(db, &alt.body, r);
                    }
                }
                _ => self.statements(db, &[s], at),
            }
        }
    }
}

/// One entry of `suffixes` or `prefixes`: a pattern, or with `re:` a regular expression, tried on
/// the lower case name. Anchored at the end for a suffix, at the start for a prefix.
struct Affix {
    text: String,
    regex: Option<regex::Regex>,
}

impl Affix {
    fn new(text: &str) -> Affix {
        let regex = text.strip_prefix("re:").map(|r| {
            regex::Regex::new(&format!("(?i){r}")).expect("checked when the configuration was read")
        });
        Affix {
            text: text.to_string(),
            regex,
        }
    }

    fn fits(&self, name: &str, front: bool) -> bool {
        // ponytail: `*` in an affix is the whole-name glob, so `_p*` matches `x_p1` and
        // `x_p1_y` alike; anchor it per affix if that ever matters.
        if let Some(r) = &self.regex {
            return r.find_iter(name).any(|m| {
                if front {
                    m.start() == 0
                } else {
                    m.end() == name.len()
                }
            });
        }
        let a = self.text.to_ascii_lowercase();
        let p = if front {
            format!("{a}**")
        } else {
            format!("**{a}")
        };
        yavs_diag::files::glob(p.as_bytes(), name.as_bytes())
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
        let affixes = |rule: &str| -> Vec<Affix> {
            cfg.option_list(rule)
                .iter()
                .map(|a| Affix::new(a))
                .collect()
        };
        let naming = [
            ("lint_602", "suffix", affixes("lint_602"), false),
            ("lint_603", "prefix", affixes("lint_603"), true),
        ];
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
                for (rule, opt, affixes, front) in &naming {
                    // A naming convention is the project's: none listed, nothing to check.
                    if affixes.is_empty() || affixes.iter().any(|a| a.fits(&n, *front)) {
                        continue;
                    }
                    let listed: Vec<&str> = affixes.iter().map(|a| a.text.as_str()).collect();
                    found.push((
                        *rule,
                        l.span,
                        format!(
                            "Registered signal `{}` does not have the {opt} {}",
                            name(db, d),
                            listed.join(" or ")
                        ),
                    ));
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
