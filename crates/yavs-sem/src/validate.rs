//! HIR invariants (sem.md 7.3), run in debug builds and in every corpus test.
//!
//! [`check`] walks the arenas and returns every violation; [`validate`] reports them as internal
//! errors (`E9999`, "this is a bug in yavs") and is what the loader calls after a successful
//! analysis in debug builds. A violation means sem built a wrong HIR, never that the design is
//! wrong, so nothing here is a user diagnostic.
//!
//! Invariants (7.3):
//! 1. no `ExprKind::Error`, `DeclKind::Error`, `TypeKind::Error` and no pending subprogram body
//!    (outside std natives and packages whose body is not analyzed yet);
//! 2. every `Call` argument list is complete and in formal order;
//! 3. every expression's type equals the type its parent expects (except under `Conv`,
//!    `ImplicitConv`, `Qualified`);
//! 4. a universal-typed expression whose parent expects a specific type is wrapped in
//!    `ImplicitConv`;
//! 5. every signal target has an `Lsp`.
//!
//! Beyond the list: association maps are in formal order, `Next`/`Exit` name a loop, `Index`,
//! `Field`, `Deref` and `Slice` have prefixes of the right kind, and scalar range constraints have
//! bounds of the subtype's type.
//!
//! Only new entries are checked on each call (the arenas only grow); pending bodies are rechecked
//! every time because a later package body fills them.

use std::collections::HashSet;

use rustc_hash::FxHashMap;

use yavs_diag::Span;

use crate::db::{Db, Reported};
use crate::decl::{Body, DeclKind};
use crate::hir::{
    Actual, AggAssoc, Arg, Call, Choice, Delay, Expr, ExprKind, Rhs, StmtKind, Target, UnitHir,
    Waveform,
};
use crate::ids::{DeclId, ExprId, Id, ScopeId, StmtId, SubtypeId, TypeId};
use crate::scope::ScopeKind;
use crate::types::{Constraint, RangeC, TypeKind};

/// How much of each arena earlier calls already checked.
#[derive(Clone, Copy, Debug, Default, serde::Serialize, serde::Deserialize)]
pub struct Mark {
    exprs: usize,
    stmts: usize,
    decls: usize,
    types: usize,
    subtypes: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Violation {
    pub span: Span,
    pub what: String,
}

/// Check everything added since the last call and report each violation as an internal error.
/// Returns the number of violations. Does nothing when errors were already reported: the HIR of
/// a failed analysis may legitimately contain poison.
pub fn validate(db: &mut Db) -> usize {
    if db.error_count() > 0 {
        return 0;
    }
    let (found, mark) = check_from(db, db.validated);
    db.validated = mark;
    let n = found.len();
    for v in found {
        let _: Reported = db.ice(v.span, format!("HIR invariant violated: {}", v.what));
    }
    n
}

/// All violations in the whole database.
pub fn check(db: &Db) -> Vec<Violation> {
    check_from(db, Mark::default()).0
}

fn check_from(db: &Db, from: Mark) -> (Vec<Violation>, Mark) {
    let mut cx = Cx {
        db,
        out: Vec::new(),
        rows: FxHashMap::default(),
    };
    for i in from.exprs..db.exprs.len() {
        cx.note_rows(ExprId::from_index(i));
    }
    for i in from.types..db.types.len() {
        cx.type_(TypeId::from_index(i));
    }
    for i in from.subtypes..db.subtypes.len() {
        cx.subtype(SubtypeId::from_index(i));
    }
    for i in from.decls..db.decls.len() {
        cx.decl_new(DeclId::from_index(i));
    }
    cx.pending_bodies();
    for i in from.exprs..db.exprs.len() {
        cx.expr(ExprId::from_index(i));
    }
    for i in from.stmts..db.stmts.len() {
        cx.stmt(StmtId::from_index(i));
    }
    let mark = Mark {
        exprs: db.exprs.len(),
        stmts: db.stmts.len(),
        decls: db.decls.len(),
        types: db.types.len(),
        subtypes: db.subtypes.len(),
    };
    (cx.out, mark)
}

struct Cx<'a> {
    db: &'a Db,
    out: Vec<Violation>,
    /// Nested rows of multi-dimensional aggregates -> the aggregate holding them (the HIR gives a
    /// row the outermost aggregate's subtype; its dimension is its depth, aggregate.rs).
    rows: FxHashMap<ExprId, ExprId>,
}

impl<'a> Cx<'a> {
    fn bad(&mut self, span: Span, what: impl Into<String>) {
        self.out.push(Violation {
            span,
            what: what.into(),
        });
    }

    // -- type helpers ---------------------------------------------------------

    /// Follow completed incomplete types.
    fn norm(&self, mut t: TypeId) -> TypeId {
        for _ in 0..8 {
            match self.db.types[t].kind {
                TypeKind::Incomplete {
                    completion: Some(c),
                } => t = c,
                _ => break,
            }
        }
        t
    }

    fn base_of(&self, e: ExprId) -> TypeId {
        self.norm(self.db.subtypes[self.db.exprs[e].subtype].base)
    }

    fn kind(&self, t: TypeId) -> &'a TypeKind {
        let db: &'a Db = self.db;
        &db.types[t].kind
    }

    fn is_error(&self, t: TypeId) -> bool {
        matches!(self.kind(t), TypeKind::Error)
    }

    fn is_universal(&self, t: TypeId) -> bool {
        matches!(
            self.kind(t),
            TypeKind::UniversalInteger | TypeKind::UniversalReal
        )
    }

    fn name(&self, t: TypeId) -> &str {
        self.db.types[t].name.text.as_str()
    }

    /// Invariants 3 and 4 for one parent-child edge.
    fn expect(&mut self, child: ExprId, want: TypeId, ctx: &str) {
        let got = self.base_of(child);
        let want = self.norm(want);
        if got == want || self.is_error(got) || self.is_error(want) {
            return;
        }
        let span = self.db.exprs[child].span;
        if self.is_universal(got) && !self.is_universal(want) {
            let (g, w) = (self.name(got).to_string(), self.name(want).to_string());
            self.bad(
                span,
                format!("{ctx}: universal expression of type `{g}` where `{w}` is expected is not wrapped in ImplicitConv"),
            );
        } else {
            let (g, w) = (self.name(got).to_string(), self.name(want).to_string());
            self.bad(
                span,
                format!("{ctx}: expression has type `{g}` but its parent expects `{w}`"),
            );
        }
    }

    fn expect_wk(&mut self, child: ExprId, want: Option<TypeId>, ctx: &str) {
        if let Some(w) = want {
            self.expect(child, w, ctx);
        }
    }

    // -- types, subtypes, declarations ---------------------------------------

    fn type_(&mut self, t: TypeId) {
        if t == self.db.poison.ty {
            return;
        }
        let def = &self.db.types[t];
        let span = self.db.decls[def.decl].span;
        match &def.kind {
            TypeKind::Error => self.bad(span, format!("TypeKind::Error for `{}`", def.name.text)),
            TypeKind::Incomplete { completion: None } => {
                self.bad(
                    span,
                    format!("incomplete type `{}` was never completed", def.name.text),
                );
            }
            _ => {}
        }
    }

    /// Scalar range constraints: bounds have the subtype's type, or are universal (type
    /// declarations `range 0 to 15`).
    fn subtype(&mut self, s: SubtypeId) {
        if s == self.db.poison.subtype {
            return;
        }
        let def = &self.db.subtypes[s];
        let Constraint::Range(RangeC::Explicit { left, right, .. }) = &def.constraint else {
            return;
        };
        let base = self.norm(def.base);
        for (e, side) in [(*left, "left"), (*right, "right")] {
            let got = self.base_of(e);
            if got != base && !self.is_universal(got) && !self.is_error(got) && !self.is_error(base)
            {
                let span = self.db.exprs[e].span;
                let (g, b) = (self.name(got).to_string(), self.name(base).to_string());
                self.bad(
                    span,
                    format!("{side} bound of a range constraint has type `{g}`, not `{b}`"),
                );
            }
        }
    }

    fn decl_new(&mut self, d: DeclId) {
        if d == self.db.poison.decl {
            return;
        }
        let decl = &self.db.decls[d];
        match &decl.kind {
            DeclKind::Error => self.bad(decl.span, format!("DeclKind::Error for `{}`", decl.name)),
            DeclKind::Object(o) => {
                if let Some(def) = o.default {
                    let t = self.norm(self.db.subtypes[o.subtype].base);
                    self.expect(def, t, "object initial value");
                }
            }
            DeclKind::Subprogram(sp) | DeclKind::GenericSubprogram(sp) => {
                for p in sp.params.iter() {
                    if let Some(def) = p.default {
                        let t = self.norm(self.db.subtypes[p.subtype].base);
                        self.expect(def, t, "parameter default");
                    }
                }
            }
            _ => {}
        }
    }

    /// Subprograms declared in a region that needs the body there (not a package, protected
    /// type or instance) or whose package body is analyzed must not have `Body::Pending`.
    fn pending_bodies(&mut self) {
        let with_body: HashSet<ScopeId> = self
            .db
            .units
            .iter()
            .filter_map(|(_, u)| match u.hir {
                UnitHir::PackageBody { package } => Some(self.db.units[package].scope),
                _ => None,
            })
            .chain(self.db.decls.iter().filter_map(|(_, d)| match d.kind {
                DeclKind::Package {
                    scope,
                    body: Some(_),
                    ..
                } => Some(scope),
                _ => None,
            }))
            .collect();
        for (_, decl) in self.db.decls.iter() {
            let DeclKind::Subprogram(sp) = &decl.kind else {
                continue;
            };
            if !matches!(sp.body, Body::Pending) {
                continue;
            }
            let needs = match self.db.scopes[decl.scope].kind {
                ScopeKind::Package => with_body.contains(&decl.scope),
                // An entity's subprograms get their bodies from an architecture, if any.
                ScopeKind::Protected
                | ScopeKind::ProtectedBody
                | ScopeKind::Instance
                | ScopeKind::Entity => false,
                _ => true,
            };
            if needs {
                self.bad(
                    decl.span,
                    format!("subprogram `{}` has no body (Body::Pending)", decl.name),
                );
            }
        }
    }

    // -- expressions ------------------------------------------------------------

    fn call_args(&mut self, callee: DeclId, args: &[Arg], span: Span) {
        let decl = &self.db.decls[callee];
        let Some(sp) = decl.kind.subprogram() else {
            if !matches!(decl.kind, DeclKind::Error) {
                self.bad(span, format!("callee `{}` is not a subprogram", decl.name));
            }
            return;
        };
        if args.len() != sp.params.len() {
            self.bad(
                span,
                format!(
                    "call of `{}` has {} arguments for {} formals",
                    decl.name,
                    args.len(),
                    sp.params.len()
                ),
            );
            return;
        }
        for (i, (a, p)) in args.iter().zip(sp.params.iter()).enumerate() {
            if usize::from(a.formal) != i {
                self.bad(
                    span,
                    format!(
                        "argument {i} of `{}` is for formal {} (not in formal order)",
                        decl.name, a.formal
                    ),
                );
                continue;
            }
            if let Actual::Expr(e) = a.actual {
                let t = self.norm(self.db.subtypes[p.subtype].base);
                self.expect(
                    e,
                    t,
                    &format!("argument `{}` of `{}`", p.name.text, decl.name),
                );
            }
        }
    }

    /// An association map: formals strictly increasing and in range.
    fn assoc_map(&mut self, args: &[Arg], formals: Option<usize>, span: Span, what: &str) {
        let mut prev: Option<u16> = None;
        for a in args {
            if prev.is_some_and(|p| a.formal <= p) {
                self.bad(
                    span,
                    format!("{what} is not in strictly increasing formal order"),
                );
                return;
            }
            if formals.is_some_and(|n| usize::from(a.formal) >= n) {
                self.bad(
                    span,
                    format!(
                        "{what} names formal {} of only {}",
                        a.formal,
                        formals.unwrap_or(0)
                    ),
                );
                return;
            }
            prev = Some(a.formal);
        }
    }

    fn expr(&mut self, id: ExprId) {
        if id == self.db.poison.expr {
            return;
        }
        let e: &Expr = &self.db.exprs[id];
        let span = e.span;
        match &e.kind {
            ExprKind::Error => self.bad(span, "ExprKind::Error"),
            ExprKind::Const(c) => {
                if e.value.is_some_and(|v| v != *c) {
                    self.bad(
                        span,
                        "Const expression whose folded value differs from its constant",
                    );
                }
            }
            ExprKind::Call { callee, args, .. } => self.call_args(*callee, args, span),
            ExprKind::Index { prefix, indices } => {
                let pt = self.base_of(*prefix);
                match self.kind(pt) {
                    TypeKind::Array { indexes, .. } => {
                        if indexes.len() != indices.len() {
                            self.bad(
                                span,
                                format!(
                                    "{} indices for a {}-dimensional array",
                                    indices.len(),
                                    indexes.len()
                                ),
                            );
                        } else {
                            for (&ix, &st) in indices.iter().zip(indexes.iter()) {
                                let t = self.norm(self.db.subtypes[st].base);
                                self.expect(ix, t, "array index");
                            }
                        }
                    }
                    _ if self.is_error(pt) => {}
                    _ => self.bad(span, "Index prefix is not an array"),
                }
            }
            ExprKind::Slice { prefix, range } => {
                let pt = self.base_of(*prefix);
                match self.kind(pt) {
                    TypeKind::Array { indexes, .. } => {
                        if let (RangeC::Explicit { left, right, .. }, Some(&st)) =
                            (range, indexes.first())
                        {
                            let t = self.norm(self.db.subtypes[st].base);
                            self.expect(*left, t, "slice bound");
                            self.expect(*right, t, "slice bound");
                        }
                    }
                    _ if self.is_error(pt) => {}
                    _ => self.bad(span, "Slice prefix is not an array"),
                }
            }
            ExprKind::Field { prefix, elem } => {
                let pt = self.base_of(*prefix);
                match self.kind(pt) {
                    TypeKind::Record { elems } => match elems.get(*elem as usize) {
                        Some(el) => {
                            let want = self.norm(self.db.subtypes[el.subtype].base);
                            let got = self.norm(self.db.subtypes[e.subtype].base);
                            if want != got && !self.is_error(got) && !self.is_error(want) {
                                self.bad(
                                    span,
                                    format!(
                                        "record element `{}` selected with type `{}`",
                                        el.name.text,
                                        self.name(got)
                                    ),
                                );
                            }
                        }
                        None => self.bad(span, "Field index is outside the record"),
                    },
                    _ if self.is_error(pt) => {}
                    _ => self.bad(span, "Field prefix is not a record"),
                }
            }
            ExprKind::Deref { prefix } => {
                let pt = self.base_of(*prefix);
                // A generic access type too (2019 6.5.3).
                match self.db.designated(pt) {
                    Some(designated) => {
                        let want = self.norm(self.db.subtypes[designated].base);
                        let got = self.norm(self.db.subtypes[e.subtype].base);
                        if want != got && !self.is_error(got) && !self.is_error(want) {
                            self.bad(span, "Deref result is not the designated type");
                        }
                    }
                    _ if self.is_error(pt) => {}
                    _ => self.bad(span, "Deref prefix is not an access value"),
                }
            }
            ExprKind::Aggregate { assocs, .. } => self.aggregate(id, assocs),
            ExprKind::ImplicitConv { operand } => {
                let from = self.base_of(*operand);
                let to = self.norm(self.db.subtypes[e.subtype].base);
                if !self.is_universal(from) && !self.is_error(from) {
                    self.bad(
                        span,
                        format!(
                            "ImplicitConv of a non-universal operand of type `{}`",
                            self.name(from)
                        ),
                    );
                }
                if self.is_universal(to) {
                    self.bad(span, "ImplicitConv to a universal type");
                }
            }
            // Qualified and Conv change the type on purpose; the others have no typed children.
            _ => {}
        }
    }

    /// Records the rows of a multi-dimensional aggregate (see `rows`).
    fn note_rows(&mut self, id: ExprId) {
        let ExprKind::Aggregate { assocs, .. } = &self.db.exprs[id].kind else {
            return;
        };
        let at = self.base_of(id);
        if matches!(self.kind(at), TypeKind::Array { indexes, .. } if indexes.len() > 1) {
            for a in assocs.iter() {
                // Rows carry the outermost aggregate's type; an element value does not.
                if matches!(self.db.exprs[a.value].kind, ExprKind::Aggregate { .. })
                    && self.base_of(a.value) == at
                {
                    self.rows.insert(a.value, id);
                }
            }
        }
    }

    fn aggregate(&mut self, id: ExprId, assocs: &[AggAssoc]) {
        let span = self.db.exprs[id].span;
        let at = self.base_of(id);
        match self.kind(at) {
            TypeKind::Array { indexes, elem } => {
                let elem = self.norm(self.db.subtypes[*elem].base);
                let one_dim = indexes.len() == 1;
                let (mut dim, mut up) = (0, id);
                while let Some(&p) = self.rows.get(&up) {
                    (dim, up) = (dim + 1, p);
                }
                let index = indexes
                    .get(dim)
                    .map(|&s| self.norm(self.db.subtypes[s].base));
                for a in assocs {
                    // Multi-dimensional aggregates nest: a row's value is a lower-dimensional
                    // array, except in the last dimension.
                    if one_dim {
                        let want = if a.slice_piece { at } else { elem };
                        self.expect(a.value, want, "array aggregate element");
                    } else if dim + 1 == indexes.len() {
                        self.expect(a.value, elem, "array aggregate element");
                    }
                    for c in a.choices.iter() {
                        match (c, index) {
                            (Choice::Expr(e), Some(t)) => self.expect(*e, t, "aggregate choice"),
                            (Choice::Range(RangeC::Explicit { left, right, .. }), Some(t)) => {
                                self.expect(*left, t, "aggregate choice");
                                self.expect(*right, t, "aggregate choice");
                            }
                            _ => {}
                        }
                    }
                }
            }
            TypeKind::Record { elems } => {
                for a in assocs {
                    for c in a.choices.iter() {
                        if let Choice::Element(i) = c {
                            match elems.get(*i as usize) {
                                Some(el) => {
                                    let t = self.norm(self.db.subtypes[el.subtype].base);
                                    self.expect(a.value, t, "record aggregate element");
                                }
                                None => {
                                    self.bad(span, "aggregate names an element outside the record")
                                }
                            }
                        }
                    }
                }
            }
            _ if self.is_error(at) => {}
            _ => self.bad(span, "aggregate of a type that is neither array nor record"),
        }
    }

    // -- statements -------------------------------------------------------------

    fn cond(&mut self, e: ExprId, what: &str) {
        self.expect_wk(e, self.db.well_known.boolean, what);
    }

    fn time(&mut self, e: ExprId, what: &str) {
        self.expect_wk(e, self.db.well_known.time, what);
    }

    fn target(&mut self, t: &Target, signal: bool, span: Span) {
        if signal && t.lsps.is_empty() {
            self.bad(span, "signal target without a longest static prefix (Lsp)");
        }
    }

    fn waveform(&mut self, w: &Waveform, target: ExprId) {
        if let Waveform::Elements(els) = w {
            let tt = self.base_of(target);
            for el in els.iter() {
                if let Some(v) = el.value {
                    self.expect(v, tt, "waveform element");
                }
                if let Some(a) = el.after {
                    self.time(a, "`after` delay");
                }
            }
        }
    }

    fn rhs_wave(&mut self, rhs: &Rhs<Waveform>, target: ExprId) {
        match rhs {
            Rhs::Simple(w) => self.waveform(w, target),
            Rhs::Conditional(alts) => {
                for (w, c) in alts.iter() {
                    self.waveform(w, target);
                    if let Some(c) = c {
                        self.cond(*c, "condition");
                    }
                }
            }
            Rhs::Selected { alts, .. } => {
                for (w, _) in alts.iter() {
                    self.waveform(w, target);
                }
            }
        }
    }

    fn rhs_expr(&mut self, rhs: &Rhs<ExprId>, target: ExprId, what: &str) {
        let tt = self.base_of(target);
        match rhs {
            Rhs::Simple(e) => self.expect(*e, tt, what),
            Rhs::Conditional(alts) => {
                for (e, c) in alts.iter() {
                    self.expect(*e, tt, what);
                    if let Some(c) = c {
                        self.cond(*c, "condition");
                    }
                }
            }
            Rhs::Selected { alts, .. } => {
                for (e, _) in alts.iter() {
                    self.expect(*e, tt, what);
                }
            }
        }
    }

    fn delay(&mut self, d: &Delay) {
        if let Delay::Inertial { reject: Some(r) } = d {
            self.time(*r, "reject limit");
        }
    }

    fn call(&mut self, c: &Call) {
        self.call_args(c.callee, &c.args, c.span);
    }

    fn report_parts(&mut self, report: Option<ExprId>, severity: Option<ExprId>) {
        if let Some(r) = report {
            self.expect_wk(r, self.db.well_known.string, "report message");
        }
        if let Some(s) = severity {
            self.expect_wk(s, self.db.well_known.severity_level, "severity");
        }
    }

    fn loop_target(&mut self, loop_: StmtId, span: Span) {
        if !matches!(self.db.stmts[loop_].kind, StmtKind::Loop { .. }) {
            self.bad(span, "next/exit does not name a loop statement");
        }
    }

    fn stmt(&mut self, id: StmtId) {
        let s = &self.db.stmts[id];
        let span = s.span;
        match &s.kind {
            StmtKind::Wait { until, timeout, .. } => {
                if let Some(u) = until {
                    self.cond(*u, "`wait until` condition");
                }
                if let Some(t) = timeout {
                    self.time(*t, "`wait for` timeout");
                }
            }
            StmtKind::Assert {
                cond,
                report,
                severity,
            }
            | StmtKind::ConcAssert {
                cond,
                report,
                severity,
                ..
            } => {
                self.cond(*cond, "assertion condition");
                self.report_parts(*report, *severity);
            }
            StmtKind::Report { report, severity } => {
                self.report_parts(Some(*report), *severity);
            }
            StmtKind::SignalAssign {
                target, delay, rhs, ..
            }
            | StmtKind::ConcSignalAssign {
                target, delay, rhs, ..
            } => {
                self.target(target, true, span);
                self.delay(delay);
                self.rhs_wave(rhs, target.expr);
            }
            StmtKind::Force { target, rhs, .. } => {
                self.target(target, true, span);
                self.rhs_expr(rhs, target.expr, "forced value");
            }
            StmtKind::Release { target, .. } => self.target(target, true, span),
            StmtKind::VarAssign { target, rhs } => {
                self.rhs_expr(rhs, target.expr, "assigned value");
            }
            StmtKind::ProcCall(c) | StmtKind::ConcProcCall { call: c, .. } => self.call(c),
            StmtKind::If { branches, .. } => {
                for (c, _) in branches.iter() {
                    self.cond(*c, "`if` condition");
                }
            }
            StmtKind::Loop {
                scheme: crate::hir::LoopScheme::While(c),
                ..
            } => self.cond(*c, "`while` condition"),
            StmtKind::Next { loop_, when } | StmtKind::Exit { loop_, when } => {
                self.loop_target(*loop_, span);
                if let Some(w) = when {
                    self.cond(*w, "`when` condition");
                }
            }
            StmtKind::Block {
                generics,
                generic_map,
                ports,
                port_map,
                ..
            } => {
                self.assoc_map(generic_map, Some(generics.len()), span, "block generic map");
                self.assoc_map(port_map, Some(ports.len()), span, "block port map");
            }
            StmtKind::Instance {
                target,
                generic_map,
                port_map,
                ..
            } => {
                let (g, p) = match target {
                    crate::hir::InstTarget::Component(c) => match &self.db.decls[*c].kind {
                        DeclKind::Component(comp) => {
                            (Some(comp.generics.len()), Some(comp.ports.len()))
                        }
                        _ => (None, None),
                    },
                    crate::hir::InstTarget::Entity { unit, .. } => {
                        match &self.db.units[*unit].hir {
                            UnitHir::Entity {
                                generics, ports, ..
                            } => (Some(generics.len()), Some(ports.len())),
                            _ => (None, None),
                        }
                    }
                    crate::hir::InstTarget::Configuration(_) => (None, None),
                };
                self.assoc_map(generic_map, g, span, "instance generic map");
                self.assoc_map(port_map, p, span, "instance port map");
            }
            StmtKind::IfGenerate { alts } => {
                for a in alts.iter() {
                    if let Some(c) = a.cond {
                        self.cond(c, "`if generate` condition");
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::decl::{Decl, Designator, ObjClass, Param, Subprogram, SubprogramKind};
    use crate::expr::literal::tests::{add_type, char_enum, ident, new_db};
    use crate::hir::{Mode, Stmt};
    use crate::stat::Staticness;
    use yavs_syntax::Symbol;

    struct T {
        db: Db,
        int: TypeId,
        boolean: TypeId,
        uint: TypeId,
    }

    fn t() -> T {
        let mut db = new_db();
        let int = add_type(&mut db, "integer", TypeKind::Integer);
        let boolean = char_enum(&mut db, "boolean", "ft");
        let uint = add_type(&mut db, "universal_integer", TypeKind::UniversalInteger);
        db.well_known.boolean = Some(boolean);
        T {
            db,
            int,
            boolean,
            uint,
        }
    }

    fn ex(db: &mut Db, ty: TypeId, kind: ExprKind) -> ExprId {
        let subtype = db.types[ty].first_subtype;
        db.add_expr(Expr {
            kind,
            subtype,
            span: Span::default(),
            stat: Staticness::None,
            value: None,
        })
    }

    fn func(db: &mut Db, params: &[TypeId], ret: TypeId) -> DeclId {
        let root = db.root;
        let params = params
            .iter()
            .enumerate()
            .map(|(i, &ty)| Param {
                name: ident(&format!("p{i}")),
                class: ObjClass::Constant,
                mode: Mode::In,
                subtype: db.types[ty].first_subtype,
                default: None,
                decl: None,
                mark: None,
            })
            .collect();
        let sym = Symbol::intern("f");
        db.decls.push(Decl {
            name: Designator::Ident(ident("f")),
            kind: DeclKind::Subprogram(Subprogram {
                kind: SubprogramKind::Function { pure: true },
                params,
                ret: Some(db.types[ret].first_subtype),
                generics: None,
                scope: None,
                body: Body::Native(sym),
                mangled: sym,
                ret_mark: None,
                impure_reads: false,
            }),
            scope: root,
            span: Span::default(),
            implicit: false,
            since: crate::LangRev::Vhdl1993,
        })
    }

    fn call(db: &mut Db, f: DeclId, ret: TypeId, args: Vec<(u16, ExprId)>) -> ExprId {
        let args = args
            .into_iter()
            .map(|(formal, e)| Arg {
                formal,
                actual: Actual::Expr(e),
                inertial: false,
            })
            .collect();
        ex(
            db,
            ret,
            ExprKind::Call {
                callee: f,
                args,
                method_of: None,
                implicit_cond: false,
            },
        )
    }

    fn stmt(db: &mut Db, kind: StmtKind) -> StmtId {
        let id = db.stmts.next_id();
        db.stmts.push(Stmt {
            id,
            label: None,
            span: Span::default(),
            kind,
        })
    }

    fn msgs(db: &Db) -> Vec<String> {
        check(db).into_iter().map(|v| v.what).collect()
    }

    #[test]
    fn empty_and_well_formed_databases_are_clean() {
        let mut t = t();
        assert_eq!(msgs(&t.db), Vec::<String>::new());
        let f = func(&mut t.db, &[t.int, t.int], t.int);
        let a = ex(&mut t.db, t.int, ExprKind::Null);
        let b = ex(&mut t.db, t.int, ExprKind::Null);
        let c = call(&mut t.db, f, t.int, vec![(0, a), (1, b)]);
        let _ = c;
        assert_eq!(msgs(&t.db), Vec::<String>::new());
        assert_eq!(validate(&mut t.db), 0);
    }

    #[test]
    fn poison_entities_are_not_violations() {
        let t = t();
        assert!(matches!(t.db.exprs[t.db.poison.expr].kind, ExprKind::Error));
        assert!(msgs(&t.db).is_empty());
    }

    #[test]
    fn error_nodes() {
        let mut t = t();
        ex(&mut t.db, t.int, ExprKind::Error);
        add_type(&mut t.db, "bad", TypeKind::Error);
        let root = t.db.root;
        t.db.decls.push(Decl {
            name: Designator::Ident(ident("bad")),
            kind: DeclKind::Error,
            scope: root,
            span: Span::default(),
            implicit: false,
            since: crate::LangRev::Vhdl1993,
        });
        let m = msgs(&t.db);
        assert!(m.iter().any(|m| m.contains("ExprKind::Error")), "{m:?}");
        assert!(m.iter().any(|m| m.contains("TypeKind::Error")), "{m:?}");
        assert!(m.iter().any(|m| m.contains("DeclKind::Error")), "{m:?}");
    }

    #[test]
    fn call_arguments() {
        let mut t = t();
        let f = func(&mut t.db, &[t.int, t.int], t.int);
        let a = ex(&mut t.db, t.int, ExprKind::Null);
        // Incomplete.
        call(&mut t.db, f, t.int, vec![(0, a)]);
        assert!(
            msgs(&t.db)
                .iter()
                .any(|m| m.contains("1 arguments for 2 formals"))
        );
        // Out of order.
        let mut t2 = self::t();
        let f = func(&mut t2.db, &[t2.int, t2.int], t2.int);
        let a = ex(&mut t2.db, t2.int, ExprKind::Null);
        call(&mut t2.db, f, t2.int, vec![(1, a), (0, a)]);
        assert!(
            msgs(&t2.db)
                .iter()
                .any(|m| m.contains("not in formal order"))
        );
        // Wrong type.
        let mut t3 = self::t();
        let f = func(&mut t3.db, &[t3.int], t3.int);
        let b = ex(&mut t3.db, t3.boolean, ExprKind::Null);
        call(&mut t3.db, f, t3.int, vec![(0, b)]);
        let m = msgs(&t3.db);
        assert!(
            m.iter()
                .any(|m| m.contains("has type `boolean` but its parent expects `integer`")),
            "{m:?}"
        );
        // Not a subprogram.
        let mut t4 = self::t();
        let d = t4.db.types[t4.int].decl;
        let e = ex(
            &mut t4.db,
            t4.int,
            ExprKind::Call {
                callee: d,
                args: Box::new([]),
                method_of: None,
                implicit_cond: false,
            },
        );
        let _ = e;
        assert!(msgs(&t4.db).iter().any(|m| m.contains("not a subprogram")));
    }

    #[test]
    fn universal_needs_implicit_conv() {
        let mut t = t();
        let f = func(&mut t.db, &[t.int], t.int);
        let u = ex(&mut t.db, t.uint, ExprKind::Null);
        call(&mut t.db, f, t.int, vec![(0, u)]);
        let m = msgs(&t.db);
        assert!(
            m.iter().any(|m| m.contains("not wrapped in ImplicitConv")),
            "{m:?}"
        );
        // Wrapped: fine.
        let mut t = self::t();
        let f = func(&mut t.db, &[t.int], t.int);
        let u = ex(&mut t.db, t.uint, ExprKind::Null);
        let w = ex(&mut t.db, t.int, ExprKind::ImplicitConv { operand: u });
        call(&mut t.db, f, t.int, vec![(0, w)]);
        assert_eq!(msgs(&t.db), Vec::<String>::new());
        // ImplicitConv of a specific type is wrong.
        let i = ex(&mut t.db, t.int, ExprKind::Null);
        ex(&mut t.db, t.int, ExprKind::ImplicitConv { operand: i });
        assert!(
            msgs(&t.db)
                .iter()
                .any(|m| m.contains("non-universal operand"))
        );
    }

    #[test]
    fn signal_targets_need_an_lsp() {
        let mut t = t();
        let tgt = ex(&mut t.db, t.int, ExprKind::Null);
        let val = ex(&mut t.db, t.int, ExprKind::Null);
        let w = Waveform::Elements(Box::new([crate::hir::WaveElem {
            value: Some(val),
            after: None,
        }]));
        stmt(
            &mut t.db,
            StmtKind::SignalAssign {
                target: Target {
                    expr: tgt,
                    lsps: Box::new([]),
                },
                delay: Delay::Default,
                rhs: Rhs::Simple(w),
            },
        );
        assert!(
            msgs(&t.db)
                .iter()
                .any(|m| m.contains("without a longest static prefix"))
        );
        // A variable assignment does not need one.
        let mut t = self::t();
        let tgt = ex(&mut t.db, t.int, ExprKind::Null);
        let val = ex(&mut t.db, t.int, ExprKind::Null);
        stmt(
            &mut t.db,
            StmtKind::VarAssign {
                target: Target {
                    expr: tgt,
                    lsps: Box::new([]),
                },
                rhs: Rhs::Simple(val),
            },
        );
        assert_eq!(msgs(&t.db), Vec::<String>::new());
        // Wrong value type.
        let bad = ex(&mut t.db, t.boolean, ExprKind::Null);
        stmt(
            &mut t.db,
            StmtKind::VarAssign {
                target: Target {
                    expr: tgt,
                    lsps: Box::new([]),
                },
                rhs: Rhs::Simple(bad),
            },
        );
        assert!(msgs(&t.db).iter().any(|m| m.contains("assigned value")));
    }

    #[test]
    fn conditions_and_loops() {
        let mut t = t();
        let good = ex(&mut t.db, t.boolean, ExprKind::Null);
        let bad = ex(&mut t.db, t.int, ExprKind::Null);
        stmt(
            &mut t.db,
            StmtKind::If {
                branches: Box::new([(good, Box::new([]))]),
                else_: None,
            },
        );
        assert_eq!(msgs(&t.db), Vec::<String>::new());
        stmt(
            &mut t.db,
            StmtKind::If {
                branches: Box::new([(bad, Box::new([]))]),
                else_: None,
            },
        );
        let nul = stmt(&mut t.db, StmtKind::Null);
        stmt(
            &mut t.db,
            StmtKind::Exit {
                loop_: nul,
                when: None,
            },
        );
        let m = msgs(&t.db);
        assert!(m.iter().any(|m| m.contains("`if` condition")), "{m:?}");
        assert!(
            m.iter().any(|m| m.contains("does not name a loop")),
            "{m:?}"
        );
        let (body, root) = (Box::new([]), t.db.root);
        let lp = stmt(
            &mut t.db,
            StmtKind::Loop {
                scheme: crate::hir::LoopScheme::Forever,
                scope: root,
                body,
            },
        );
        let n = msgs(&t.db).len();
        stmt(
            &mut t.db,
            StmtKind::Next {
                loop_: lp,
                when: None,
            },
        );
        assert_eq!(msgs(&t.db).len(), n);
    }

    #[test]
    fn pending_bodies() {
        use crate::scope::ScopeKind;
        let mut t = t();
        let pkg = t.db.add_scope(ScopeKind::Package, None, None);
        let arch = t.db.add_scope(ScopeKind::Architecture, None, None);
        for scope in [pkg, arch] {
            let f = func(&mut t.db, &[], t.int);
            t.db.decls[f].scope = scope;
            if let DeclKind::Subprogram(sp) = &mut t.db.decls[f].kind {
                sp.body = Body::Pending;
            }
        }
        // Only the architecture one: a package body may still come.
        let m = msgs(&t.db);
        assert_eq!(m.len(), 1, "{m:?}");
        assert!(m[0].contains("Body::Pending"));
    }

    #[test]
    fn index_field_deref_shapes() {
        let mut t = t();
        let int_arr = crate::expr::literal::tests::array_of(
            &mut t.db,
            "ints",
            t.int,
            0,
            yavs_syntax::ast::Direction::To,
            9,
        );
        let arr = ex(&mut t.db, int_arr, ExprKind::Null);
        let ok = ex(&mut t.db, t.int, ExprKind::Null);
        ex(
            &mut t.db,
            t.int,
            ExprKind::Index {
                prefix: arr,
                indices: Box::new([ok]),
            },
        );
        // The index type of `ints` is its own anonymous `idx` integer, so an `integer` index mismatches.
        let m = msgs(&t.db);
        assert!(m.iter().any(|m| m.contains("array index")), "{m:?}");
        let scalar = ex(&mut t.db, t.int, ExprKind::Null);
        ex(
            &mut t.db,
            t.int,
            ExprKind::Index {
                prefix: scalar,
                indices: Box::new([ok]),
            },
        );
        ex(
            &mut t.db,
            t.int,
            ExprKind::Field {
                prefix: scalar,
                elem: 0,
            },
        );
        ex(&mut t.db, t.int, ExprKind::Deref { prefix: scalar });
        let m = msgs(&t.db);
        for want in ["Index prefix", "Field prefix", "Deref prefix"] {
            assert!(m.iter().any(|m| m.contains(want)), "{want}: {m:?}");
        }
    }

    #[test]
    fn validate_reports_ices_once_and_is_silent_after_errors() {
        let mut t = t();
        ex(&mut t.db, t.int, ExprKind::Error);
        assert_eq!(validate(&mut t.db), 1);
        assert_eq!(t.db.error_count(), 1);
        assert_eq!(t.db.diagnostics()[0].code, crate::diag::INTERNAL);
        // Incremental: the same expression is not reported again.
        assert_eq!(validate(&mut t.db), 0);
        // With errors already present nothing is checked.
        let mut t = self::t();
        t.db.ice(Span::default(), "earlier");
        ex(&mut t.db, t.int, ExprKind::Error);
        assert_eq!(validate(&mut t.db), 0);
    }
}
