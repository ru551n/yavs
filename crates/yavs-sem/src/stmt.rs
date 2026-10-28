//! Sequential (LRM 10, slice S2) and concurrent (LRM 11, slice S5) statements:
//! AST to HIR (sem.md 7.2) with the placement and target legality rules
//! (sem.md 8.1, E13xx/E18xx). Longest static prefixes, drivers and `all`
//! sensitivity come from [`crate::lsp`].
//!
//! Entry points for the other modules:
//! - [`Db::analyze_subprogram_stmts`]: a subprogram body (decl.rs calls it
//!   once the parameters and body declarations are in `scope`).
//! - [`Db::analyze_conc_stmts`]: the statement part of an architecture or
//!   entity (unit.rs); runs the multiple-source check over the whole part.
//!
//! Expression typing, declarations, choices and binding belong to other
//! modules; this file reaches them only through the hooks at the bottom
//! (CONTRIBUTING.md, "Requests").

// `analyze_conc_stmts` is called from unit.rs once architectures land.
#![allow(dead_code)]

use rustc_hash::FxHashSet;
use yavs_diag::{Diagnostic, Span};
use yavs_syntax::Symbol;
use yavs_syntax::ast::{self, ForceMode, Ident};

use crate::LangRev;
use crate::db::{Db, Reported};
use crate::decl::{Decl, DeclKind, Designator, ObjClass, ObjRole, Object, SubprogramKind};
use crate::diag;
use crate::expr::Expected;
use crate::hir::{
    Arg, Call, ChoiceSet, Delay, ExprKind, GenAlt, InstTarget, LoopScheme, Lsp, Mode, ObjRoot, Rhs,
    Sensitivity, Stmt, StmtKind, Target, WaveElem, Waveform,
};
use crate::ids::{DeclId, ExprId, ScopeId, StmtId, SubtypeId, TypeId};
use crate::lsp::NameRoot;
use crate::scope::ScopeKind;
use crate::types::{BoundsFrom, RangeC, Resolution};

/// What contains the sequential statements being analyzed.
#[derive(Clone, Copy, Debug)]
pub(crate) enum Owner {
    /// `sensitivity`: the span of the process's sensitivity list, if any.
    Process { sensitivity: Option<Span> },
    Subprogram {
        decl: DeclId,
        kind: SubprogramKind,
        /// Declared (transitively) inside a process: may drive any visible signal.
        in_process: bool,
        /// Why `wait` is illegal here (10.2): a function, or a parent that is
        /// a function or a process with a sensitivity list.
        wait_ban: Option<WaitBan>,
    },
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum WaitBan {
    /// Inside a function (`own`: the function itself, not a parent).
    Function { own: bool },
    /// Inside a procedure declared in a process with a sensitivity list.
    Sensitized(Span),
}

/// The statement context: regions and enclosing loops.
struct StmtCx {
    /// Innermost region (loop and block scopes nest here).
    scope: ScopeId,
    /// The process or subprogram region: statement labels are declared here.
    labels: ScopeId,
    owner: Owner,
    /// Enclosing loops, innermost last: `(label key, loop statement)`.
    loops: Vec<(Option<Symbol>, StmtId)>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TargetKind {
    Signal,
    Variable,
    Force,
    Release,
}

impl TargetKind {
    fn wants_signal(self) -> bool {
        self != TargetKind::Variable
    }
}

/// Collects `Err`s so analysis continues after an error and reports the
/// independent ones too (fail clearly), but the result is still `Err`.
#[derive(Default)]
struct Errs(bool);

impl Errs {
    fn keep<T>(&mut self, r: Result<T, Reported>) -> Option<T> {
        r.map_err(|_| self.0 = true).ok()
    }
    fn finish<T>(self, v: T) -> Result<T, Reported> {
        if self.0 { Err(Reported) } else { Ok(v) }
    }
}

impl Db {
    // -- entry points -----------------------------------------------------------

    /// Analyze the statements of the body of subprogram `decl`, whose
    /// parameters and declarations are already in `scope`. Checks `wait`,
    /// `return`, drivers outside processes (E1810) and purity (E1830).
    pub(crate) fn analyze_subprogram_stmts(
        &mut self,
        decl: DeclId,
        scope: ScopeId,
        stmts: &[ast::SeqStmt],
    ) -> Result<Box<[StmtId]>, Reported> {
        let Some(sp) = self.decls[decl].kind.subprogram() else {
            return Err(self.ice(self.decls[decl].span, "subprogram body of a non-subprogram"));
        };
        let kind = sp.kind;
        let (in_process, parent_ban) = match self.stmt_owners.last() {
            Some(Owner::Process { sensitivity }) => (true, sensitivity.map(WaitBan::Sensitized)),
            Some(Owner::Subprogram {
                in_process,
                wait_ban,
                ..
            }) => (
                *in_process,
                wait_ban.map(|b| match b {
                    WaitBan::Function { .. } => WaitBan::Function { own: false },
                    b => b,
                }),
            ),
            None => (false, None),
        };
        let wait_ban = match kind {
            SubprogramKind::Function { .. } => Some(WaitBan::Function { own: true }),
            SubprogramKind::Procedure => parent_ban,
        };
        let owner = Owner::Subprogram {
            decl,
            kind,
            in_process,
            wait_ban,
        };
        let mut cx = StmtCx {
            scope,
            labels: scope,
            owner,
            loops: Vec::new(),
        };
        self.stmt_owners.push(owner);
        let body = self.seq_list(&mut cx, stmts);
        self.stmt_owners.pop();
        let unresolved = crate::attr::unresolved_labels(self, scope);
        let body = body?;
        unresolved?;
        if let SubprogramKind::Function { pure: true } = kind {
            self.check_purity(decl, scope, &body);
        }
        Ok(body)
    }

    /// Analyze the concurrent statement part of an architecture (or, with
    /// `entity`, of an entity: passive statements only), declaring labels in
    /// `scope`, then check that no unresolved signal has two sources.
    pub(crate) fn analyze_conc_stmts(
        &mut self,
        scope: ScopeId,
        stmts: &[ast::ConcStmt],
        entity: bool,
    ) -> Result<Box<[StmtId]>, Reported> {
        let ids = self.conc_stmts(scope, stmts, entity)?;
        if self.check_multiple_sources(&ids) {
            return Err(Reported);
        }
        Ok(ids)
    }

    /// The statements of a subprogram body whose parameter region is `scope`
    /// (its `owner` is the subprogram). Hook called by decl.rs; errors are
    /// reported, the result is then empty.
    pub(crate) fn seq_stmts(&mut self, scope: ScopeId, stmts: &[ast::SeqStmt]) -> Box<[StmtId]> {
        let Some(decl) = self.scopes[scope].owner else {
            let span = stmts.first().map_or_else(Span::default, |s| s.span);
            self.ice(span, "subprogram body region without its subprogram");
            return Box::default();
        };
        self.analyze_subprogram_stmts(decl, scope, stmts)
            .unwrap_or_default()
    }

    // -- helpers ----------------------------------------------------------------

    fn new_stmt(&mut self, label: Option<Ident>, span: Span) -> StmtId {
        self.stmts.push(Stmt {
            id: self.stmts.next_id(),
            label,
            span,
            kind: StmtKind::Null,
        })
    }

    /// Declare a non-overloadable name (label, loop or generate parameter,
    /// `GUARD`): any other declaration of the same key in the region is a
    /// homograph (E1101).
    fn declare_plain(&mut self, scope: ScopeId, d: Decl) -> Result<DeclId, Reported> {
        let id = self.decls.push(d);
        let before = self.error_count();
        self.declare(scope, id);
        if self.error_count() > before {
            Err(Reported)
        } else {
            Ok(id)
        }
    }

    fn declare_label(
        &mut self,
        scope: ScopeId,
        label: Ident,
        stmt: StmtId,
    ) -> Result<DeclId, Reported> {
        let d = self.declare_plain(
            scope,
            Decl {
                name: Designator::Ident(label),
                kind: DeclKind::Label(stmt),
                scope,
                span: label.span,
                implicit: true,
                since: LangRev::Vhdl1993,
            },
        )?;
        crate::attr::label_declared(self, scope, d)?;
        Ok(d)
    }

    /// A constant parameter (loop or generate).
    fn declare_param(
        &mut self,
        scope: ScopeId,
        name: Ident,
        role: ObjRole,
        subtype: SubtypeId,
    ) -> Result<DeclId, Reported> {
        self.declare_plain(
            scope,
            Decl {
                name: Designator::Ident(name),
                kind: DeclKind::Object(Object {
                    class: ObjClass::Constant,
                    role,
                    mode: None,
                    subtype,
                    default: None,
                    bounds: BoundsFrom::Subtype,
                    signal_kind: None,
                    file_open: None,
                }),
                scope,
                span: name.span,
                implicit: false,
                since: LangRev::Vhdl1993,
            },
        )
    }

    /// A region owned by the statement's label (for expanded names `l.x`).
    fn labelled_scope(
        &mut self,
        kind: ScopeKind,
        parent: ScopeId,
        label_scope: ScopeId,
        label: Option<Ident>,
        stmt: StmtId,
    ) -> Result<ScopeId, Reported> {
        let owner = match label {
            Some(l) => Some(self.declare_label(label_scope, l, stmt)?),
            None => None,
        };
        Ok(self.add_scope(kind, Some(parent), owner))
    }

    fn well_known_type(
        &mut self,
        t: Option<TypeId>,
        span: Span,
        what: &str,
    ) -> Result<TypeId, Reported> {
        match t {
            Some(t) => Ok(t),
            None => Err(self.ice(span, format!("`{what}` used before std.standard is loaded"))),
        }
    }

    fn time_type(&mut self, span: Span) -> Result<TypeId, Reported> {
        self.well_known_type(self.well_known.time, span, "time")
    }

    fn expr_type(&self, e: ExprId) -> TypeId {
        self.base(self.exprs[e].subtype)
    }

    fn decl_name(&self, d: DeclId) -> String {
        self.decls[d].name.to_string()
    }

    // -- sequential statements (LRM 10) -----------------------------------------

    fn seq_list(
        &mut self,
        cx: &mut StmtCx,
        stmts: &[ast::SeqStmt],
    ) -> Result<Box<[StmtId]>, Reported> {
        let mut errs = Errs::default();
        let mut out = Vec::with_capacity(stmts.len());
        for s in stmts {
            if let Some(id) = errs.keep(self.seq_stmt(cx, s)) {
                out.push(id);
            }
        }
        errs.finish(out.into())
    }

    fn seq_stmt(&mut self, cx: &mut StmtCx, s: &ast::SeqStmt) -> Result<StmtId, Reported> {
        let id = self.new_stmt(s.label, s.span);
        // Loops and blocks own a region named by the label; others declare it plainly.
        if let Some(l) = s.label
            && !matches!(
                s.kind,
                ast::SeqStmtKind::Loop { .. } | ast::SeqStmtKind::Block { .. }
            )
        {
            self.declare_label(cx.labels, l, id)?;
        }
        let kind = self.seq_kind(cx, s, id)?;
        self.stmts[id].kind = kind;
        Ok(id)
    }

    fn seq_kind(
        &mut self,
        cx: &mut StmtCx,
        s: &ast::SeqStmt,
        id: StmtId,
    ) -> Result<StmtKind, Reported> {
        use ast::SeqStmtKind as K;
        let scope = cx.scope;
        Ok(match &s.kind {
            K::Wait { on, until, timeout } => {
                self.check_wait_allowed(cx, s.span)?;
                let mut lsps = Vec::new();
                let mut errs = Errs::default();
                for n in on.iter().flatten() {
                    if let Some(l) = errs.keep(self.sensitivity_name(scope, n, "a `wait on` list"))
                    {
                        lsps.push(l);
                    }
                }
                let until = match until {
                    Some(c) => errs.keep(self.st_expr(scope, c, Expected::Condition)),
                    None => None,
                };
                let timeout = match timeout {
                    Some(t) => {
                        let time = self.time_type(t.span)?;
                        errs.keep(self.st_expr(scope, t, Expected::Type(time)))
                    }
                    None => None,
                };
                errs.finish(())?;
                // 10.2: without `on`, the sensitivity is the signals of the condition.
                if on.is_none()
                    && let Some(c) = until
                {
                    self.reads(c, &mut lsps);
                }
                StmtKind::Wait {
                    on: lsps.into(),
                    until,
                    timeout,
                }
            }
            K::Assert(a) => {
                let (cond, report, severity) = self.assertion(scope, a)?;
                StmtKind::Assert {
                    cond,
                    report,
                    severity,
                }
            }
            K::Report { report, severity } => {
                let string = self.well_known_type(self.well_known.string, report.span, "string")?;
                let report = self.st_expr(scope, report, Expected::Type(string))?;
                let severity = self.severity(scope, severity.as_ref())?;
                StmtKind::Report { report, severity }
            }
            K::SignalAssign(sa) => self.seq_signal_assign(cx, sa, s.span)?,
            K::VariableAssign { target, rhs } => match unaffected_alts(rhs) {
                Some(alts) => self.split_alts(cx, &alts, s.span, &mut |db, cx, v| {
                    let (target, rhs) = db.assign_parts(
                        cx.scope,
                        target,
                        &ast::Rhs::Simple(v),
                        TargetKind::Variable,
                        &mut |db, e, ty| db.value_expr(cx.scope, e, ty),
                    )?;
                    db.check_protected_assign(target.expr)?;
                    Ok(StmtKind::VarAssign { target, rhs })
                })?,
                None => {
                    let (target, rhs) = self.assign_parts(
                        scope,
                        target,
                        rhs,
                        TargetKind::Variable,
                        &mut |db, e, ty| db.value_expr(scope, e, ty),
                    )?;
                    self.check_protected_assign(target.expr)?;
                    StmtKind::VarAssign { target, rhs }
                }
            },
            K::ProcedureCall(n) => {
                let call = self.st_proc_call(scope, n)?;
                self.check_call_outside_process(cx, &call)?;
                StmtKind::ProcCall(call)
            }
            K::If {
                branches, else_, ..
            } => {
                let mut errs = Errs::default();
                let mut bs = Vec::with_capacity(branches.len());
                for (c, body) in branches {
                    let c = errs.keep(self.st_expr(scope, c, Expected::Condition));
                    let body = errs.keep(self.seq_list(cx, body));
                    if let (Some(c), Some(body)) = (c, body) {
                        bs.push((c, body));
                    }
                }
                let else_ = match else_ {
                    Some(b) => errs.keep(self.seq_list(cx, b)),
                    None => None,
                };
                errs.finish(())?;
                StmtKind::If {
                    branches: bs.into(),
                    else_,
                }
            }
            K::Case {
                matching,
                expr,
                alts,
                ..
            } => {
                let subject = self.st_expr(scope, expr, Expected::Any)?;
                let choices: Vec<&[ast::Choice]> = alts.iter().map(|(c, _)| &c[..]).collect();
                let sets = self.st_case_choices(scope, subject, *matching, &choices, s.span);
                let mut errs = Errs::default();
                let mut bodies = Vec::with_capacity(alts.len());
                for (_, body) in alts {
                    bodies.push(errs.keep(self.seq_list(cx, body)));
                }
                let sets = sets?;
                errs.finish(())?;
                StmtKind::Case {
                    expr: subject,
                    matching: *matching,
                    alts: sets.into_iter().zip(bodies.into_iter().flatten()).collect(),
                }
            }
            K::Loop { scheme, stmts, .. } => {
                let lscope = self.labelled_scope(ScopeKind::Loop, scope, cx.labels, s.label, id)?;
                let scheme = match scheme {
                    None => LoopScheme::Forever,
                    Some(ast::LoopScheme::While(c)) => {
                        LoopScheme::While(self.st_expr(scope, c, Expected::Condition)?)
                    }
                    Some(ast::LoopScheme::For(name, r)) => {
                        let (range, st) = self.st_discrete_range(scope, r)?;
                        let param = self.declare_param(lscope, *name, ObjRole::LoopParam, st)?;
                        LoopScheme::For { param, range }
                    }
                };
                cx.loops.push((s.label.map(|l| l.key), id));
                let outer = std::mem::replace(&mut cx.scope, lscope);
                let body = self.seq_list(cx, stmts);
                cx.scope = outer;
                cx.loops.pop();
                StmtKind::Loop {
                    scheme,
                    scope: lscope,
                    body: body?,
                }
            }
            K::Next { label, when } | K::Exit { label, when } => {
                let next = matches!(s.kind, K::Next { .. });
                let what = if next { "next" } else { "exit" };
                let loop_ = self.loop_target(cx, *label, s.span, what)?;
                let when = match when {
                    Some(c) => Some(self.st_expr(scope, c, Expected::Condition)?),
                    None => None,
                };
                if next {
                    StmtKind::Next { loop_, when }
                } else {
                    StmtKind::Exit { loop_, when }
                }
            }
            // 2019 conditional return (10.14): when no condition holds, execution goes on.
            K::Return(Some(
                e @ ast::Expr {
                    kind: ast::ExprKind::Conditional(_) | ast::ExprKind::Unaffected,
                    ..
                },
            )) => {
                let alts: Vec<_> = match &e.kind {
                    ast::ExprKind::Conditional(alts) => {
                        alts.iter().map(|(v, c)| (v, c.as_ref())).collect()
                    }
                    _ => vec![(e, None)],
                };
                self.split_alts(cx, &alts, s.span, &mut |db, cx, v| {
                    Ok(StmtKind::Return(db.return_value(cx, Some(v), s.span)?))
                })?
            }
            K::Return(v) => StmtKind::Return(self.return_value(cx, v.as_ref(), s.span)?),
            // 2019 `return when c;` (10.14): a procedure return under a condition.
            K::ReturnWhen(c) => {
                let v = self.return_value(cx, None, s.span);
                let c = self.st_expr(cx.scope, c, Expected::Condition);
                let (v, c) = (v?, c?);
                let ret = self.new_stmt(None, s.span);
                self.stmts[ret].kind = StmtKind::Return(v);
                StmtKind::If {
                    branches: Box::from([(c, Box::from([ret]))]),
                    else_: None,
                }
            }
            K::Null => StmtKind::Null,
            K::Block { decls, stmts, .. } => {
                let bscope =
                    self.labelled_scope(ScopeKind::SeqBlock, scope, cx.labels, s.label, id)?;
                let mut errs = Errs::default();
                errs.keep(self.st_decls(bscope, decls, s.span));
                let outer = std::mem::replace(&mut cx.scope, bscope);
                let body = errs.keep(self.seq_list(cx, stmts));
                cx.scope = outer;
                let body = errs.finish(body)?;
                StmtKind::SeqBlock {
                    scope: bscope,
                    body: body.unwrap_or_default(),
                }
            }
        })
    }

    /// 10.2: no `wait` in a function, in a process with a sensitivity list,
    /// or in a procedure whose parent is either.
    fn check_wait_allowed(&mut self, cx: &StmtCx, span: Span) -> Result<(), Reported> {
        let (msg, sec) = match cx.owner {
            Owner::Process {
                sensitivity: Some(sp),
            } => (
                "`wait` is not allowed in a process with a sensitivity list",
                Some((sp, "the sensitivity list")),
            ),
            Owner::Subprogram {
                wait_ban: Some(b), ..
            } => match b {
                WaitBan::Function { own: true } => ("`wait` is not allowed in a function", None),
                WaitBan::Function { own: false } => (
                    "`wait` is not allowed in a procedure declared inside a function",
                    None,
                ),
                WaitBan::Sensitized(sp) => (
                    "`wait` is not allowed in a procedure declared in a process with a sensitivity list",
                    Some((sp, "the sensitivity list")),
                ),
            },
            _ => return Ok(()),
        };
        let mut d = Diagnostic::error(diag::BAD_WAIT, msg).with_primary(span, "");
        if let Some((sp, m)) = sec {
            d = d.with_secondary(sp, m);
        }
        Err(self.error(d))
    }

    /// 10.2, 11.3: a procedure called by a process with a sensitivity list, directly or through
    /// other procedures, shall not contain a `wait`. Decided for procedures whose body is in the
    /// unit being analysed; others (another unit, a protected method, an instance) are caught
    /// when the `wait` executes (#597).
    /// The same for a function (`by` names it, `sens` is its designator), #823.
    pub(crate) fn check_calls_wait(
        &mut self,
        body: &[StmtId],
        sens: Span,
        by: Option<&str>,
    ) -> Result<(), Reported> {
        let mut bad = Vec::new();
        self.calls_wait(body, &mut bad);
        let failed = !bad.is_empty();
        for (call, wait) in bad {
            let name = self.decls[call.0].name;
            let (who, what) = match by {
                Some(f) => (format!("function `{f}`"), "the function"),
                None => (
                    "a process with a sensitivity list".to_string(),
                    "the sensitivity list",
                ),
            };
            let d = Diagnostic::error(
                diag::BAD_WAIT,
                format!("procedure `{name}` contains a `wait` and is called by {who}"),
            )
            .with_primary(call.1, "called here")
            .with_secondary(wait, "the `wait`")
            .with_secondary(sens, what);
            self.error(d);
        }
        if failed { Err(Reported) } else { Ok(()) }
    }

    /// The calls in `body` (nested statements included) of procedures that wait, as
    /// `((callee, call span), span of a wait)`.
    fn calls_wait(&self, body: &[StmtId], out: &mut Vec<((DeclId, Span), Span)>) {
        for &s in body {
            if let StmtKind::ProcCall(c) = &self.stmts[s].kind
                && let Some(w) = self.waits_in(self.entity(c.callee), &mut FxHashSet::default())
            {
                out.push(((c.callee, self.stmts[s].span), w));
            }
            self.calls_wait(&self.stmt_children(s), out);
        }
    }

    /// A `wait` in procedure `d` or in a procedure it calls, if its body is in the unit being
    /// analysed.
    fn waits_in(&self, d: DeclId, seen: &mut FxHashSet<DeclId>) -> Option<Span> {
        self.waits_below(d, seen, true)
    }

    /// A `wait` in procedure `d` or in a procedure it calls, in any analysed body (10.2): for
    /// the lowering of a call in a function, when every body of the design is known (#823).
    pub fn procedure_waits(&self, d: DeclId) -> Option<Span> {
        self.waits_below(self.entity(d), &mut FxHashSet::default(), false)
    }

    fn waits_below(&self, d: DeclId, seen: &mut FxHashSet<DeclId>, here: bool) -> Option<Span> {
        let crate::decl::Body::Hir { scope, stmts } = &self.decls[d].kind.subprogram()?.body else {
            return None;
        };
        if here {
            let unit = self.cur()?.unit;
            let mut s = Some(*scope);
            while let Some(x) = s {
                if x == self.units[unit].scope {
                    break;
                }
                s = self.scopes[x].parent;
            }
            s?;
        }
        if !seen.insert(d) {
            return None;
        }
        let mut stack = stmts.to_vec();
        while let Some(s) = stack.pop() {
            match &self.stmts[s].kind {
                StmtKind::Wait { .. } => return Some(self.stmts[s].span),
                StmtKind::ProcCall(c) => {
                    if let Some(w) = self.waits_below(self.entity(c.callee), seen, here) {
                        return Some(w);
                    }
                }
                _ => {}
            }
            stack.extend(self.stmt_children(s));
        }
        None
    }

    fn loop_target(
        &mut self,
        cx: &StmtCx,
        label: Option<Ident>,
        span: Span,
        what: &str,
    ) -> Result<StmtId, Reported> {
        match label {
            None => match cx.loops.last() {
                Some(&(_, l)) => Ok(l),
                None => Err(self.error(
                    Diagnostic::error(
                        diag::NEXT_OUTSIDE_LOOP,
                        format!("`{what}` is not inside a loop"),
                    )
                    .with_primary(span, ""),
                )),
            },
            Some(l) => match cx.loops.iter().rev().find(|(k, _)| *k == Some(l.key)) {
                Some(&(_, s)) => Ok(s),
                None => Err(self.error(
                    Diagnostic::error(
                        diag::NEXT_OUTSIDE_LOOP,
                        format!("`{}` is not a label of an enclosing loop", l.text),
                    )
                    .with_primary(l.span, "")
                    .with_note(format!(
                        "`{what}` can only leave or continue a loop it is inside"
                    )),
                )),
            },
        }
    }

    fn return_value(
        &mut self,
        cx: &StmtCx,
        v: Option<&ast::Expr>,
        span: Span,
    ) -> Result<Option<ExprId>, Reported> {
        let bad = |db: &mut Db, msg: &str| {
            Err(db.error(Diagnostic::error(diag::BAD_RETURN, msg).with_primary(span, "")))
        };
        match (cx.owner, v) {
            (Owner::Process { .. }, _) => bad(self, "`return` is only allowed in a subprogram"),
            (
                Owner::Subprogram {
                    kind: SubprogramKind::Procedure,
                    ..
                },
                None,
            ) => Ok(None),
            (
                Owner::Subprogram {
                    kind: SubprogramKind::Procedure,
                    ..
                },
                Some(_),
            ) => bad(self, "a procedure cannot return a value"),
            (Owner::Subprogram { .. }, None) => bad(self, "a function return needs a value"),
            (Owner::Subprogram { decl, .. }, Some(e)) => {
                let Some(ret) = self.decls[decl].kind.subprogram().and_then(|s| s.ret) else {
                    return Err(self.ice(span, "function without a return subtype"));
                };
                // A named return value (2019, 4.2.1): the value takes the call's subtype.
                let ret = self.ret_names.get(&cx.labels).map_or(ret, |&(_, st)| st);
                let id = self.expr_in_subtype(cx.scope, e, ret);
                if self.is_poison_expr(id) {
                    return Err(Reported);
                }
                self.check_linkage_reads(id, false)?;
                Ok(Some(id))
            }
        }
    }

    fn assertion(
        &mut self,
        scope: ScopeId,
        a: &ast::Assertion,
    ) -> Result<(ExprId, Option<ExprId>, Option<ExprId>), Reported> {
        let mut errs = Errs::default();
        let cond = errs.keep(self.st_expr(scope, &a.cond, Expected::Condition));
        let report = match &a.report {
            Some(r) => {
                let string = self.well_known_type(self.well_known.string, r.span, "string")?;
                errs.keep(self.st_expr(scope, r, Expected::Type(string)))
            }
            None => None,
        };
        let severity = errs
            .keep(self.severity(scope, a.severity.as_ref()))
            .flatten();
        let cond = errs.finish(cond)?;
        match cond {
            Some(c) => Ok((c, report, severity)),
            None => Err(Reported),
        }
    }

    fn severity(
        &mut self,
        scope: ScopeId,
        e: Option<&ast::Expr>,
    ) -> Result<Option<ExprId>, Reported> {
        let Some(e) = e else { return Ok(None) };
        let sl = self.well_known_type(self.well_known.severity_level, e.span, "severity_level")?;
        Ok(Some(self.st_expr(scope, e, Expected::Type(sl))?))
    }

    fn seq_signal_assign(
        &mut self,
        cx: &StmtCx,
        sa: &ast::SignalAssign,
        span: Span,
    ) -> Result<StmtKind, Reported> {
        let scope = cx.scope;
        let kind = match &sa.kind {
            ast::SignalAssignKind::Waveform { delay, rhs } => {
                let (target, delay, rhs) =
                    self.signal_assign(scope, &sa.target, delay.as_ref(), rhs)?;
                self.check_driver_outside_process(cx, &target.lsps)?;
                StmtKind::SignalAssign { target, delay, rhs }
            }
            ast::SignalAssignKind::Force { mode, rhs } if let Some(alts) = unaffected_alts(rhs) => {
                self.split_alts(cx, &alts, span, &mut |db, cx, v| {
                    let (target, rhs) = db.assign_parts(
                        cx.scope,
                        &sa.target,
                        &ast::Rhs::Simple(v),
                        TargetKind::Force,
                        &mut |db, e, ty| db.value_expr(cx.scope, e, ty),
                    )?;
                    db.check_force_mode(&target, *mode)?;
                    db.check_force_member(&target)?;
                    Ok(StmtKind::Force {
                        target,
                        mode: *mode,
                        rhs,
                    })
                })?
            }
            ast::SignalAssignKind::Force { mode, rhs } => {
                let (target, rhs) = self.assign_parts(
                    scope,
                    &sa.target,
                    rhs,
                    TargetKind::Force,
                    &mut |db, e, ty| db.value_expr(scope, e, ty),
                )?;
                self.check_force_mode(&target, *mode)?;
                self.check_force_member(&target)?;
                StmtKind::Force {
                    target,
                    mode: *mode,
                    rhs,
                }
            }
            ast::SignalAssignKind::Release { mode } => {
                let e = self.st_target(scope, &sa.target, Expected::Any)?;
                let lsps = self.check_target(e, TargetKind::Release)?;
                let target = Target { expr: e, lsps };
                self.check_force_mode(&target, *mode)?;
                self.check_force_member(&target)?;
                StmtKind::Release {
                    target,
                    mode: *mode,
                }
            }
        };
        Ok(kind)
    }

    /// 10.5.2.1: `force out` / `release out` is not allowed on an `in` port.
    fn check_force_mode(&mut self, t: &Target, mode: Option<ForceMode>) -> Result<(), Reported> {
        if mode != Some(ForceMode::Out) {
            return Ok(());
        }
        for l in t.lsps.iter() {
            if let ObjRoot::Decl(d) = l.root
                && let DeclKind::Object(o) = &self.decls[d].kind
                && o.mode == Some(Mode::In)
            {
                let name = self.decl_name(d);
                return Err(self.error(
                    Diagnostic::error(
                        diag::CLASS_MISUSE,
                        format!("force mode `out` is not allowed on `{name}` of mode `in`"),
                    )
                    .with_primary(l.span, "")
                    .with_help("use `force in` to force the effective value"),
                ));
            }
        }
        Ok(())
    }

    /// 10.5.2.1: the target of a force or release is not a member of a signal of a composite
    /// subtype resolved as a whole (E1362); the whole composite may be forced.
    fn check_force_member(&mut self, t: &Target) -> Result<(), Reported> {
        let mut e = t.expr;
        loop {
            let p = match &self.exprs[e].kind {
                ExprKind::Field { prefix, .. }
                | ExprKind::Index { prefix, .. }
                | ExprKind::Slice { prefix, .. } => *prefix,
                _ => return Ok(()),
            };
            let mut st = Some(self.exprs[p].subtype);
            while let Some(s) = st {
                match &self.subtypes[s].resolution {
                    Some(Resolution::Function(_)) => {
                        return Err(self.error(
                            Diagnostic::error(
                                diag::FORCE_RESOLVED_MEMBER,
                                "the target of a force or release is a member of a composite signal resolved as a whole",
                            )
                            .with_primary(self.exprs[t.expr].span, "")
                            .with_secondary(self.exprs[p].span, "resolved as a whole")
                            .with_help("force or release the whole composite signal"),
                        ));
                    }
                    Some(_) => break,
                    None => st = self.subtypes[s].parent,
                }
            }
            e = p;
        }
    }

    /// 4.? / 11.3: a subprogram not inside a process may only drive its
    /// signal parameters (E1810).
    fn check_driver_outside_process(&mut self, cx: &StmtCx, lsps: &[Lsp]) -> Result<(), Reported> {
        let Owner::Subprogram {
            in_process: false, ..
        } = cx.owner
        else {
            return Ok(());
        };
        for l in lsps {
            if let ObjRoot::External(x) = l.root {
                // LRM 10.5.2.1 names no exception for external names (#255).
                return Err(self.error(
                    Diagnostic::error(
                        diag::DRIVER_OUTSIDE_PROCESS,
                        "an external signal is driven from a subprogram declared outside a \
                         process",
                    )
                    .with_primary(l.span, "")
                    .with_secondary(self.externals[x].span, "external name")
                    .with_note(
                        "a subprogram that is not declared inside a process may drive only its signal parameters",
                    )
                    .with_help(
                        "pass the signal as a `signal ... : out` parameter, or use `<= force` and `<= release`",
                    ),
                ));
            }
            let ObjRoot::Decl(d) = l.root else { continue };
            let is_param =
                matches!(&self.decls[d].kind, DeclKind::Object(o) if o.role == ObjRole::Param);
            if !is_param {
                let name = self.decl_name(d);
                return Err(self.error(
                    Diagnostic::error(
                        diag::DRIVER_OUTSIDE_PROCESS,
                        format!("signal `{name}` is driven from a subprogram and is not a parameter"),
                    )
                    .with_primary(l.span, "")
                    .with_secondary(self.decls[d].span, "declared outside the subprogram here")
                    .with_note(
                        "a subprogram that is not declared inside a process may drive only its signal parameters",
                    )
                    .with_help("pass the signal as a `signal ... : out` parameter"),
                ));
            }
        }
        Ok(())
    }

    fn check_call_outside_process(&mut self, cx: &StmtCx, call: &Call) -> Result<(), Reported> {
        let mut drv = Vec::new();
        self.collect_drivers_of_call(call, &mut drv);
        self.check_driver_outside_process(cx, &drv)
    }

    fn collect_drivers_of_call(&self, call: &Call, out: &mut Vec<Lsp>) {
        // Drivers of a call are its signal actuals of `out`/`inout`/`buffer`
        // formals; parameter-rooted ones are kept here for the E1810 check.
        let Some(sp) = self.decls[call.callee].kind.subprogram() else {
            return;
        };
        for a in call.args.iter() {
            let Some(p) = sp.params.get(usize::from(a.formal)) else {
                continue;
            };
            if p.class != ObjClass::Signal
                || !matches!(p.mode, Mode::Out | Mode::Inout | Mode::Buffer)
            {
                continue;
            }
            let es: Vec<ExprId> = match &a.actual {
                crate::hir::Actual::Expr(e) | crate::hir::Actual::Converted { actual: e, .. } => {
                    vec![*e]
                }
                crate::hir::Actual::Individual(parts) => parts.iter().map(|p| p.1).collect(),
                _ => continue,
            };
            for e in es {
                if let Some(l) = self.lsp(e)
                    && self.is_signal(l.root)
                {
                    out.push(l);
                }
            }
        }
    }

    /// Resolve a waveform or force assignment: the target and its right-hand
    /// side. An aggregate target takes its type from the right-hand side
    /// (10.5.2.1), every other target gives the type to it.
    fn assign_parts<A, H>(
        &mut self,
        scope: ScopeId,
        target: &ast::Expr,
        rhs: &ast::Rhs<A>,
        kind: TargetKind,
        each: &mut Each<'_, A, H>,
    ) -> Result<(Target, Rhs<H>), Reported>
    where
        H: HasType,
    {
        if let ast::ExprKind::Aggregate(_) = &target.kind {
            let rhs = self.rhs(scope, rhs, None, each)?;
            let Some(ty) = rhs_type(self, &rhs) else {
                return Err(self.error(
                    Diagnostic::error(
                        diag::AGG_NO_TYPE,
                        "the type of this aggregate target is not determined",
                    )
                    .with_primary(target.span, "")
                    .with_help("qualify the right-hand side (`t'(...)`) so it has one type"),
                ));
            };
            let e = self.st_target(scope, target, Expected::Type(ty))?;
            let lsps = self.check_target(e, kind)?;
            self.check_agg_distinct(e)?;
            Ok((Target { expr: e, lsps }, rhs))
        } else {
            let e = self.st_target(scope, target, Expected::Any)?;
            let lsps = self.check_target(e, kind)?;
            // The target's subtype, so an `others` aggregate takes its bounds (9.3.3.3).
            let mut st = self.exprs[e].subtype;
            // `--relaxed` (R5): an unconstrained target (a port, a formal) gives an `others`
            // aggregate the bounds it has at run time.
            if self.constrainedness(st) != crate::types::Constrainedness::Full && self.relaxed() {
                st = self.runtime_subtype(e, target.span);
                self.relaxed_target = Some(st);
            }
            let rhs = self.rhs(scope, rhs, Some(st), each);
            self.relaxed_target = None;
            let rhs = rhs?;
            Ok((Target { expr: e, lsps }, rhs))
        }
    }

    fn signal_assign(
        &mut self,
        scope: ScopeId,
        target: &ast::Expr,
        delay: Option<&ast::DelayMechanism>,
        rhs: &ast::Rhs<ast::Waveform>,
    ) -> Result<(Target, Delay, Rhs<Waveform>), Reported> {
        let mut nulls = Vec::new();
        let (target, rhs) =
            self.assign_parts(scope, target, rhs, TargetKind::Signal, &mut |db, w, ty| {
                db.waveform(scope, w, ty, &mut nulls)
            })?;
        self.check_null_waveform(&target, &nulls)?;
        let delay = match delay {
            None => Delay::Default,
            Some(ast::DelayMechanism::Transport) => Delay::Transport,
            Some(ast::DelayMechanism::Inertial { reject }) => Delay::Inertial {
                reject: match reject {
                    Some(r) => {
                        let time = self.time_type(r.span)?;
                        Some(self.st_expr(scope, r, Expected::Type(time))?)
                    }
                    None => None,
                },
            },
        };
        Ok((target, delay, rhs))
    }

    /// 11.6: the target of a concurrent signal assignment is all guarded signals or none,
    /// and a guarded one needs a guarded assignment (#812; sim1 and sim2 agree).
    fn check_guarded_target(&mut self, t: &Target, guarded: bool) -> Result<(), Reported> {
        let kinds: Vec<bool> = t
            .lsps
            .iter()
            .filter_map(|l| match l.root {
                ObjRoot::Decl(d) => Some(
                    matches!(&self.decls[d].kind, DeclKind::Object(o) if o.signal_kind.is_some()),
                ),
                _ => None,
            })
            .collect();
        let msg = if kinds.contains(&true) && kinds.contains(&false) {
            "the target mixes guarded and unguarded signals"
        } else if kinds.contains(&true) && !guarded {
            "a guarded signal is the target of an unguarded concurrent signal assignment"
        } else {
            return Ok(());
        };
        Err(self.error(
            Diagnostic::error(diag::CLASS_MISUSE, msg)
                .with_primary(self.exprs[t.expr].span, "")
                .with_note("LRM 11.6: the target is either guarded (all its signals are guarded, and the statement is `guarded`) or unguarded"),
        ))
    }

    /// 10.5.2.2: a `null` waveform element needs a guarded signal target.
    fn check_null_waveform(&mut self, t: &Target, nulls: &[Span]) -> Result<(), Reported> {
        let Some(&span) = nulls.first() else {
            return Ok(());
        };
        let guarded = t.lsps.iter().all(|l| match l.root {
            ObjRoot::Decl(d) => matches!(
                &self.decls[d].kind,
                DeclKind::Object(o) if o.signal_kind.is_some()
            ),
            _ => false,
        });
        if guarded {
            return Ok(());
        }
        Err(self.error(
            Diagnostic::error(
                diag::CLASS_MISUSE,
                "a `null` waveform element needs a guarded signal target",
            )
            .with_primary(span, "")
            .with_note("only signals declared `bus` or `register` can be disconnected"),
        ))
    }

    fn waveform(
        &mut self,
        scope: ScopeId,
        w: &ast::Waveform,
        ty: Option<SubtypeId>,
        nulls: &mut Vec<Span>,
    ) -> Result<Waveform, Reported> {
        let els = match w {
            ast::Waveform::Unaffected(_) => return Ok(Waveform::Unaffected),
            ast::Waveform::Elements(els) => els,
        };
        let mut ty = ty;
        let mut out = Vec::with_capacity(els.len());
        let mut errs = Errs::default();
        for el in els {
            let value = if let ast::ExprKind::Literal(ast::Literal {
                kind: ast::LiteralKind::Null,
                ..
            }) = el.value.kind
                && ty.is_none_or(|t| !self.is_access(self.base(t)))
            {
                nulls.push(el.value.span);
                None
            } else {
                let v = errs.keep(self.value_expr(scope, &el.value, ty));
                if let Some(v) = v {
                    ty.get_or_insert(self.types[self.expr_type(v)].first_subtype);
                }
                v
            };
            let after = match &el.after {
                Some(a) => {
                    let time = self.time_type(a.span)?;
                    errs.keep(self.st_expr(scope, a, Expected::Type(time)))
                }
                None => None,
            };
            out.push(WaveElem { value, after });
        }
        errs.finish(Waveform::Elements(out.into()))
    }

    fn is_access(&self, t: TypeId) -> bool {
        matches!(self.types[t].kind, crate::types::TypeKind::Access { .. })
    }

    /// A right-hand side value for a target of subtype `st`, if known.
    fn value_expr(
        &mut self,
        scope: ScopeId,
        e: &ast::Expr,
        st: Option<SubtypeId>,
    ) -> Result<ExprId, Reported> {
        let Some(st) = st else {
            // Only the value of an aggregate target has no target subtype: it is composite.
            return self.st_expr(scope, e, Expected::Composite);
        };
        let id = self.expr_in_subtype(scope, e, st);
        if self.is_poison_expr(id) {
            return Err(Reported);
        }
        self.check_linkage_reads(id, false)?;
        Ok(id)
    }

    /// A 2019 `conditional_or_unaffected_expression` in a statement (10.5.2.1, 10.6.2.1,
    /// 10.14): an `if` with the statement `one` builds for each value, none for `unaffected`.
    fn split_alts(
        &mut self,
        cx: &StmtCx,
        alts: &[(&ast::Expr, Option<&ast::Expr>)],
        span: Span,
        one: &mut dyn FnMut(&mut Db, &StmtCx, &ast::Expr) -> Result<StmtKind, Reported>,
    ) -> Result<StmtKind, Reported> {
        let mut errs = Errs::default();
        let (mut branches, mut else_) = (Vec::new(), None);
        for &(v, c) in alts {
            let body = if matches!(v.kind, ast::ExprKind::Unaffected) {
                Some(Box::default())
            } else {
                errs.keep(one(self, cx, v)).map(|k| {
                    let id = self.new_stmt(None, span);
                    self.stmts[id].kind = k;
                    Box::from([id])
                })
            };
            match c {
                Some(c) => {
                    let c = errs.keep(self.st_expr(cx.scope, c, Expected::Condition));
                    if let (Some(c), Some(b)) = (c, body) {
                        branches.push((c, b));
                    }
                }
                None => else_ = body,
            }
        }
        errs.finish(())?;
        if branches.is_empty() {
            // A lone `unaffected`.
            return Ok(StmtKind::Null);
        }
        Ok(StmtKind::If {
            branches: branches.into(),
            else_,
        })
    }

    fn rhs<A, H>(
        &mut self,
        scope: ScopeId,
        rhs: &ast::Rhs<A>,
        ty: Option<SubtypeId>,
        each: &mut Each<'_, A, H>,
    ) -> Result<Rhs<H>, Reported>
    where
        H: HasType,
    {
        let mut ty = ty;
        let mut errs = Errs::default();
        let mut one = |db: &mut Db, a: &A, ty: &mut Option<SubtypeId>, errs: &mut Errs| {
            let h = errs.keep(each(db, a, *ty));
            if let Some(h) = &h
                && ty.is_none()
            {
                *ty = h.ty(db).map(|t| db.types[t].first_subtype);
            }
            h
        };
        Ok(match rhs {
            ast::Rhs::Simple(a) => {
                let h = one(self, a, &mut ty, &mut errs);
                errs.finish(())?;
                Rhs::Simple(h.expect("no error means a value"))
            }
            ast::Rhs::Conditional(alts) => {
                let mut out = Vec::with_capacity(alts.len());
                for (a, c) in alts {
                    let h = one(self, a, &mut ty, &mut errs);
                    let c = match c {
                        Some(c) => errs
                            .keep(self.st_expr(scope, c, Expected::Condition))
                            .map(Some),
                        None => Some(None),
                    };
                    if let (Some(h), Some(c)) = (h, c) {
                        out.push((h, c));
                    }
                }
                errs.finish(())?;
                Rhs::Conditional(out.into())
            }
            ast::Rhs::Selected {
                expr,
                matching,
                alts,
            } => {
                let subject = self.st_expr(scope, expr, Expected::Any)?;
                let choices: Vec<&[ast::Choice]> = alts.iter().map(|(_, c)| &c[..]).collect();
                let sets =
                    errs.keep(self.st_case_choices(scope, subject, *matching, &choices, expr.span));
                let mut hs = Vec::with_capacity(alts.len());
                for (a, _) in alts {
                    hs.push(one(self, a, &mut ty, &mut errs));
                }
                errs.finish(())?;
                let sets = sets.expect("no error means choices");
                Rhs::Selected {
                    expr: subject,
                    matching: *matching,
                    alts: hs.into_iter().flatten().zip(sets).collect(),
                }
            }
        })
    }

    /// An aggregate target identifies each variable or signal once (LRM 10.5.2.1, 10.6.2.1;
    /// E1335): no association has several choices (null ranges aside), and no two elements
    /// are the same static name. Elements that only overlap (`(v(0 to 1), v(1 to 2))`,
    /// `(v(1), v)`) are accepted: two of three reference simulators accept them.
    fn check_agg_distinct(&mut self, e: ExprId) -> Result<(), Reported> {
        use crate::expr::aggregate::range_value;
        use crate::hir::{Choice, LspStep};
        let mut leaves: Vec<(ExprId, Span)> = Vec::new();
        let mut stack = vec![e];
        while let Some(x) = stack.pop() {
            let ExprKind::Aggregate { assocs, .. } = &self.exprs[x].kind else {
                continue;
            };
            for a in assocs.iter() {
                let many = !a.slice_piece
                    && a.choices
                        .iter()
                        .map(|c| match c {
                            Choice::Expr(_) => 1,
                            Choice::Range(r) => range_value(self, r).map_or(1, |r| r.len()),
                            _ => 1,
                        })
                        .sum::<u128>()
                        > 1;
                if many {
                    return Err(self.error(
                        Diagnostic::error(
                            diag::AGG_TARGET_TWICE,
                            "this aggregate target element is identified more than once",
                        )
                        .with_primary(self.exprs[a.value].span, "")
                        .with_note("one association with several choices names its target once per choice (LRM 10.6.2.1)"),
                    ));
                }
                if matches!(self.exprs[a.value].kind, ExprKind::Aggregate { .. }) {
                    stack.push(a.value);
                } else {
                    // 10.5.2.1, 10.6.2.1: each element is a locally static name (#812).
                    if let Some((_, _, s, cut)) = self.lsp_parts(a.value)
                        && (cut || s < crate::stat::Staticness::Locally)
                    {
                        return Err(self.error(
                            Diagnostic::error(
                                diag::NOT_STATIC,
                                "an element of an aggregate target must be a locally static name",
                            )
                            .with_primary(self.exprs[a.value].span, "not locally static")
                            .with_note("LRM 10.5.2.1, 10.6.2.1: every index and slice of an aggregate target element is locally static"),
                        ));
                    }
                    leaves.push((a.value, self.exprs[a.value].span));
                }
            }
        }
        // `Some(key)` for a fully static name without alias rebasing.
        let same = |db: &Db, x: ExprId, y: ExprId| -> bool {
            let (Some((rx, px, _, cx)), Some((ry, py, _, cy))) = (db.lsp_parts(x), db.lsp_parts(y))
            else {
                return false;
            };
            if cx || cy || rx != ry || px.len() != py.len() {
                return false;
            }
            px.iter().zip(&py).all(|(a, b)| match (a, b) {
                (LspStep::Field(m), LspStep::Field(n)) => m == n,
                (LspStep::Index(m), LspStep::Index(n)) => {
                    m.len() == n.len()
                        && m.iter().zip(n.iter()).all(|(i, j)| {
                            match (
                                db.value(*i).and_then(|v| v.as_int()),
                                db.value(*j).and_then(|v| v.as_int()),
                            ) {
                                (Some(i), Some(j)) => i == j,
                                _ => false,
                            }
                        })
                }
                (LspStep::Slice(m), LspStep::Slice(n)) => {
                    match (range_value(db, m), range_value(db, n)) {
                        (Some(m), Some(n)) => {
                            !m.is_null() && m.low() == n.low() && m.high() == n.high()
                        }
                        _ => false,
                    }
                }
                _ => false,
            })
        };
        for (i, &(x, _)) in leaves.iter().enumerate() {
            for &(y, span) in &leaves[i + 1..] {
                if same(self, x, y) {
                    return Err(self.error(
                        Diagnostic::error(
                            diag::AGG_TARGET_TWICE,
                            "this aggregate target element is identified more than once",
                        )
                        .with_primary(span, "named again here")
                        .with_secondary(self.exprs[x].span, "first named here"),
                    ));
                }
            }
        }
        Ok(())
    }

    /// Target legality (10.5, 10.6; E1360) and, for signals, the target's
    /// longest static prefixes (one per element of an aggregate target).
    fn check_target(&mut self, e: ExprId, kind: TargetKind) -> Result<Box<[Lsp]>, Reported> {
        if let ExprKind::Aggregate { assocs, .. } = &self.exprs[e].kind {
            let elems: Vec<ExprId> = assocs.iter().map(|a| a.value).collect();
            let mut out = Vec::new();
            let mut errs = Errs::default();
            for v in elems {
                if let Some(l) = errs.keep(self.check_target(v, kind)) {
                    out.extend(l.into_vec());
                }
            }
            return errs.finish(out.into());
        }
        let span = self.exprs[e].span;
        let fail = |db: &mut Db, msg: String| {
            Err(db.error(Diagnostic::error(diag::CLASS_MISUSE, msg).with_primary(span, "")))
        };
        let wants_signal = kind.wants_signal();
        let root = match self.name_root(e) {
            Some(r) => r,
            None if matches!(self.exprs[e].kind, ExprKind::Error) => return Err(Reported),
            None => {
                return fail(
                    self,
                    "the target of an assignment must be the name of an object".into(),
                );
            }
        };
        let what = match kind {
            TargetKind::Force => Some("`force`"),
            TargetKind::Release => Some("`release`"),
            _ => None,
        };
        let not_signal = |db: &mut Db, name: String| match what {
            Some(w) => fail(db, format!("{w} target `{name}` must be a signal")),
            None => fail(db, format!("`{name}` is a variable; use `:=` to assign it")),
        };
        match root {
            NameRoot::Deref if wants_signal => {
                return not_signal(self, "the designated object".into());
            }
            NameRoot::Deref => return Ok(Box::new([])),
            NameRoot::Obj(ObjRoot::External(x)) => {
                use crate::external::TargetUse;
                let u = match kind {
                    TargetKind::Signal => TargetUse::SignalAssign,
                    TargetKind::Variable => TargetUse::VarAssign,
                    TargetKind::Force | TargetKind::Release => TargetUse::Force,
                };
                self.check_external_target(x, u, span)?;
            }
            NameRoot::Obj(ObjRoot::ImplicitSignal(d)) => {
                let name = self.decl_name(d);
                return fail(
                    self,
                    format!("cannot assign to the implicit signal `{name}`"),
                );
            }
            NameRoot::Obj(ObjRoot::Decl(d)) => {
                let name = self.decl_name(d);
                let o = match &self.decls[d].kind {
                    DeclKind::Object(o) => o,
                    DeclKind::Error => return Err(Reported),
                    _ => return fail(self, format!("`{name}` is not an object")),
                };
                let (class, role, mode) = (o.class, o.role.clone(), o.mode);
                let msg = match class {
                    ObjClass::Constant => Some(match role {
                        ObjRole::LoopParam => format!("cannot assign to loop parameter `{name}`"),
                        ObjRole::GenerateParam => {
                            format!("cannot assign to generate parameter `{name}`")
                        }
                        ObjRole::Generic => format!("cannot assign to generic `{name}`"),
                        ObjRole::Param => {
                            format!("cannot assign to parameter `{name}` of mode `in`")
                        }
                        _ => format!("cannot assign to constant `{name}`"),
                    }),
                    ObjClass::File => Some(format!("cannot assign to file `{name}`")),
                    ObjClass::Signal if !wants_signal => {
                        Some(format!("`{name}` is a signal; use `<=` to assign it"))
                    }
                    ObjClass::Variable | ObjClass::SharedVariable if wants_signal => {
                        return not_signal(self, name);
                    }
                    _ if role == ObjRole::Guard => {
                        Some(format!("cannot assign to the implicit signal `{name}`"))
                    }
                    _ => match mode {
                        Some(m @ (Mode::In | Mode::Linkage))
                            if kind != TargetKind::Force && kind != TargetKind::Release
                                || m == Mode::Linkage =>
                        {
                            let m = if m == Mode::In { "in" } else { "linkage" };
                            let what = if role == ObjRole::Port {
                                "port"
                            } else {
                                "parameter"
                            };
                            Some(format!("cannot assign to {what} `{name}` of mode `{m}`"))
                        }
                        _ => None,
                    },
                };
                if let Some(m) = msg {
                    return fail(self, m);
                }
            }
        }
        if !wants_signal {
            return Ok(Box::new([]));
        }
        let Some(l) = self.lsp(e) else {
            return Err(self.ice(span, "signal target without a longest static prefix"));
        };
        if let ObjRoot::Decl(d) = l.root {
            // Ports with a mode view: no `in` element below the target (E1903).
            self.check_view_drive(d, &self.view_name_path(e), span)?;
        }
        Ok(Box::new([l]))
    }

    /// A name in a sensitivity list or `wait on` clause: a static name of a
    /// readable signal (10.2, 11.3).
    fn sensitivity_name(
        &mut self,
        scope: ScopeId,
        n: &ast::Name,
        list: &str,
    ) -> Result<Lsp, Reported> {
        let e = self.st_name(scope, n)?;
        self.check_sensitivity(e, list)
    }

    pub(crate) fn check_sensitivity(&mut self, e: ExprId, list: &str) -> Result<Lsp, Reported> {
        let span = self.exprs[e].span;
        let parts = self.lsp_parts(e);
        let Some((root, _, _, cut)) = parts else {
            if matches!(self.exprs[e].kind, ExprKind::Error) {
                return Err(Reported);
            }
            return Err(self.error(
                Diagnostic::error(
                    diag::CLASS_MISUSE,
                    format!("a name in {list} must be a signal"),
                )
                .with_primary(span, ""),
            ));
        };
        if let ObjRoot::Decl(d) = root
            && matches!(self.decls[d].kind, DeclKind::Error)
        {
            return Err(Reported);
        }
        let name = match root {
            ObjRoot::Decl(d) | ObjRoot::ImplicitSignal(d) => self.decl_name(d),
            ObjRoot::External(x) => self.externals[x]
                .path
                .last()
                .map_or_else(String::new, |(i, _)| i.text.to_string()),
        };
        if !self.is_signal(root) || self.name_root(e) == Some(NameRoot::Deref) {
            return Err(self.error(
                Diagnostic::error(
                    diag::CLASS_MISUSE,
                    format!("`{name}` in {list} must be a signal"),
                )
                .with_primary(span, ""),
            ));
        }
        if cut {
            return Err(self.error(
                Diagnostic::error(
                    diag::SENSITIVITY_NOT_STATIC,
                    format!("the name in {list} is not a static name"),
                )
                .with_primary(span, "an index or slice here is not static")
                .with_help(format!("name the whole signal `{name}`")),
            ));
        }
        if let ObjRoot::Decl(d) = root
            && let DeclKind::Object(o) = &self.decls[d].kind
            && o.mode == Some(Mode::Linkage)
        {
            return Err(self.error(
                Diagnostic::error(
                    diag::CLASS_MISUSE,
                    format!("cannot read port `{name}` of mode `linkage`"),
                )
                .with_primary(span, ""),
            ));
        }
        Ok(self.lsp(e).expect("a name with a root has a prefix"))
    }

    /// 4.3 (E1830): a pure function references no signal, variable or file
    /// declared outside it and calls no impure function. `--relaxed` (R2)
    /// makes it warning W1830.
    fn check_purity(&mut self, f: DeclId, fscope: ScopeId, body: &[StmtId]) {
        let mut found: Vec<(Span, String)> = Vec::new();
        let fname = self.decl_name(f);
        let vital = self.in_vital_library(fscope);
        let mut stack: Vec<StmtId> = body.to_vec();
        while let Some(s) = stack.pop() {
            let mut roots = Vec::new();
            self.stmt_exprs(s, &mut |e, _| roots.push(e));
            for r in roots {
                self.walk_expr(r, &mut |e| match &self.exprs[e].kind {
                    ExprKind::Obj(d) => {
                        if let DeclKind::Object(o) = &self.decls[*d].kind
                            && o.role != ObjRole::ImplicitSignal
                            && !self.scope_within(self.decls[*d].scope, fscope)
                        {
                            let class = match o.class {
                                ObjClass::Signal => "signal",
                                ObjClass::Variable | ObjClass::SharedVariable => "variable",
                                ObjClass::File => "file",
                                ObjClass::Constant => return,
                            };
                            found.push((
                                self.exprs[e].span,
                                format!(
                                    "pure function `{fname}` reads {class} `{}` declared outside it",
                                    self.decls[*d].name
                                ),
                            ));
                        }
                    }
                    ExprKind::Call { callee, .. } => {
                        if let Some(sp) = self.decls[*callee].kind.subprogram()
                            && sp.kind == (SubprogramKind::Function { pure: false })
                            // Vendored VITAL packages call `NOW` in pure functions; that
                            // is the only exemption (decisions.md).
                            && !(vital
                                && self.decls[*callee].name.to_string().eq_ignore_ascii_case("now")
                                && self.in_std_library(self.decls[*callee].scope))
                        {
                            found.push((
                                self.exprs[e].span,
                                format!(
                                    "pure function `{fname}` calls impure function `{}`",
                                    self.decls[*callee].name
                                ),
                            ));
                        }
                    }
                    _ => {}
                });
            }
            // ponytail (#42): procedure calls are not followed into the procedure's
            // body; a pure function calling a procedure with side effects passes.
            // The driver sets of procedure calls are exact (`lsp.rs` `sources`).
            stack.extend(self.stmt_children(s));
        }
        found.sort_by_key(|(s, _)| s.start);
        if !found.is_empty()
            && let DeclKind::Subprogram(sp) = &mut self.decls[f].kind
        {
            sp.impure_reads = true;
        }
        for (span, msg) in found {
            let d = if self.relaxed() {
                Diagnostic::warning(diag::W_PURE_IMPURE, msg)
            } else {
                Diagnostic::error(diag::PURE_SIDE_EFFECT, msg)
                    .with_help("declare the function `impure`")
            };
            self.report(d.with_primary(span, ""));
        }
    }

    /// Whether `s` is `ancestor` or nested in it.
    fn scope_within(&self, s: ScopeId, ancestor: ScopeId) -> bool {
        let mut cur = Some(s);
        while let Some(c) = cur {
            if c == ancestor {
                return true;
            }
            cur = self.scopes[c].parent;
        }
        false
    }

    // -- concurrent statements (LRM 11) -----------------------------------------

    fn conc_stmts(
        &mut self,
        scope: ScopeId,
        stmts: &[ast::ConcStmt],
        passive: bool,
    ) -> Result<Box<[StmtId]>, Reported> {
        let mut errs = Errs::default();
        let mut out = Vec::with_capacity(stmts.len());
        for s in stmts {
            if let Some(id) = errs.keep(self.conc_stmt(scope, s, passive)) {
                out.push(id);
            }
        }
        errs.keep(crate::attr::unresolved_labels(self, scope));
        errs.finish(out.into())
    }

    fn conc_stmt(
        &mut self,
        scope: ScopeId,
        s: &ast::ConcStmt,
        passive: bool,
    ) -> Result<StmtId, Reported> {
        use ast::ConcStmtKind as K;
        let id = self.new_stmt(s.label, s.span);
        let owns_region = matches!(
            s.kind,
            K::Process(_)
                | K::Block(_)
                | K::ForGenerate { .. }
                | K::IfGenerate { .. }
                | K::CaseGenerate { .. }
        );
        if let Some(l) = s.label
            && !owns_region
        {
            self.declare_label(scope, l, id)?;
        }
        let postponed = s.postponed;
        let kind = match &s.kind {
            K::Process(p) => self.process(scope, s, p, id, postponed, passive)?,
            K::Block(b) => self.block(scope, s, b, id)?,
            K::Instance(i) => {
                let target = self.st_inst_target(scope, &i.unit)?;
                self.instance_kind(
                    scope,
                    s.span,
                    target,
                    i.generic_map.as_deref().unwrap_or(&[]),
                    i.port_map.as_deref().unwrap_or(&[]),
                )?
            }
            // `l : c;` is parsed as a procedure call; it instantiates when `c`
            // is not a subprogram (11.7).
            K::ProcedureCall(n) if s.label.is_some() && self.names_component(scope, n) => {
                let target = self.component_target(scope, n)?;
                self.instance_kind(scope, s.span, target, &[], &[])?
            }
            K::ProcedureCall(n) => {
                let call = self.st_proc_call(scope, n)?;
                self.stmts[id].kind = StmtKind::ConcProcCall {
                    postponed,
                    call,
                    info: Default::default(),
                };
                let info = self.conc_info(id, None);
                self.check_passive(passive, &info)?;
                let StmtKind::ConcProcCall { info: i, .. } = &mut self.stmts[id].kind else {
                    unreachable!("set just above")
                };
                *i = info;
                return Ok(id);
            }
            K::Assert(a) => {
                let (cond, report, severity) = self.assertion(scope, a)?;
                self.stmts[id].kind = StmtKind::ConcAssert {
                    postponed,
                    cond,
                    report,
                    severity,
                    info: Default::default(),
                };
                let info = self.conc_info(id, None);
                if let StmtKind::ConcAssert { info: i, .. } = &mut self.stmts[id].kind {
                    *i = info;
                }
                return Ok(id);
            }
            K::SignalAssign(sa) => {
                let ast::SignalAssignKind::Waveform { delay, rhs } = &sa.kind else {
                    return Err(self.error(
                        Diagnostic::error(
                            diag::CLASS_MISUSE,
                            "`force` and `release` are sequential statements",
                        )
                        .with_primary(s.span, "")
                        .with_help("put it in a process"),
                    ));
                };
                let guard = if sa.guarded {
                    Some((self.st_lookup_guard(scope, s.span)?, s.span))
                } else {
                    None
                };
                let (target, delay, rhs) =
                    self.signal_assign(scope, &sa.target, delay.as_ref(), rhs)?;
                self.check_guarded_target(&target, sa.guarded)?;
                self.stmts[id].kind = StmtKind::ConcSignalAssign {
                    postponed,
                    guarded: sa.guarded,
                    guard: guard.map(|g| g.0),
                    target,
                    delay,
                    rhs,
                    info: Default::default(),
                };
                let info = self.conc_info(id, guard);
                self.check_passive(passive, &info)?;
                if let StmtKind::ConcSignalAssign { info: i, .. } = &mut self.stmts[id].kind {
                    *i = info;
                }
                return Ok(id);
            }
            K::ForGenerate { param, range, body } => {
                let gscope = self.labelled_scope(ScopeKind::Generate, scope, scope, s.label, id)?;
                let (range, st) = self.st_discrete_range(scope, range)?;
                let param = self.declare_param(gscope, *param, ObjRole::GenerateParam, st)?;
                let body = self.generate_body(gscope, body)?;
                StmtKind::ForGenerate {
                    param,
                    range,
                    scope: gscope,
                    body,
                }
            }
            K::IfGenerate { branches, else_ } => {
                let owner = match s.label {
                    Some(l) => Some(self.declare_label(scope, l, id)?),
                    None => None,
                };
                let mut errs = Errs::default();
                let mut alts = Vec::with_capacity(branches.len() + 1);
                for (label, cond, body) in branches {
                    let cond = errs.keep(self.st_expr(scope, cond, Expected::Condition));
                    let alt = errs.keep(self.gen_alt(scope, owner, *label, cond, body));
                    alts.extend(alt);
                }
                if let Some((label, body)) = else_ {
                    alts.extend(errs.keep(self.gen_alt(scope, owner, *label, None, body)));
                }
                errs.finish(())?;
                StmtKind::IfGenerate { alts: alts.into() }
            }
            K::CaseGenerate { expr, alts } => {
                let owner = match s.label {
                    Some(l) => Some(self.declare_label(scope, l, id)?),
                    None => None,
                };
                let subject = self.st_expr(scope, expr, Expected::Any)?;
                let choices: Vec<&[ast::Choice]> = alts.iter().map(|(_, c, _)| &c[..]).collect();
                let mut errs = Errs::default();
                let sets = errs.keep(self.st_case_choices(scope, subject, false, &choices, s.span));
                let mut gs = Vec::with_capacity(alts.len());
                for (label, _, body) in alts {
                    gs.push(errs.keep(self.gen_alt(scope, owner, *label, None, body)));
                }
                errs.finish(())?;
                StmtKind::CaseGenerate {
                    expr: subject,
                    alts: sets
                        .expect("no error means choices")
                        .into_iter()
                        .zip(gs.into_iter().flatten())
                        .collect(),
                }
            }
        };
        self.stmts[id].kind = kind;
        Ok(id)
    }

    fn check_passive(
        &mut self,
        passive: bool,
        info: &crate::hir::ProcessInfo,
    ) -> Result<(), Reported> {
        let Some(l) = info.drivers.first().filter(|_| passive) else {
            return Ok(());
        };
        Err(self.error(
            Diagnostic::error(
                diag::NOT_PASSIVE,
                "a statement in an entity statement part cannot drive a signal",
            )
            .with_primary(l.span, "driven here")
            .with_note("entity statements must be passive (LRM 11.1)")
            .with_help("move it to an architecture"),
        ))
    }

    fn process(
        &mut self,
        scope: ScopeId,
        s: &ast::ConcStmt,
        p: &ast::ProcessStmt,
        id: StmtId,
        postponed: bool,
        passive: bool,
    ) -> Result<StmtKind, Reported> {
        let pscope = self.labelled_scope(ScopeKind::Process, scope, scope, s.label, id)?;
        let mut errs = Errs::default();
        let (sensitivity, sens_span) = match &p.sensitivity {
            None => (Sensitivity::None, None),
            Some(ast::Sensitivity::All(sp)) => (Sensitivity::All, Some(*sp)),
            Some(ast::Sensitivity::List(names)) => {
                let mut lsps = Vec::with_capacity(names.len());
                for n in names {
                    lsps.extend(errs.keep(self.sensitivity_name(scope, n, "a sensitivity list")));
                }
                let span = names.iter().map(|n| n.span).reduce(Span::to);
                (Sensitivity::List(lsps.into()), span)
            }
        };
        let owner = Owner::Process {
            sensitivity: sens_span,
        };
        self.stmt_owners.push(owner);
        errs.keep(self.st_decls(pscope, &p.decls, s.span));
        let mut cx = StmtCx {
            scope: pscope,
            labels: pscope,
            owner,
            loops: Vec::new(),
        };
        let body = errs.keep(self.seq_list(&mut cx, &p.stmts));
        self.stmt_owners.pop();
        errs.keep(crate::attr::unresolved_labels(self, pscope));
        errs.finish(())?;
        let body = body.expect("no error means a body");
        if let Some(sp) = sens_span {
            self.check_calls_wait(&body, sp, None)?;
        }
        let all = matches!(sensitivity, Sensitivity::All);
        let info = self.process_info(pscope, &body, all);
        self.check_passive(passive, &info)?;
        Ok(StmtKind::Process {
            postponed,
            sensitivity,
            info,
            scope: pscope,
            body,
        })
    }

    fn block(
        &mut self,
        scope: ScopeId,
        s: &ast::ConcStmt,
        b: &ast::BlockStmt,
        id: StmtId,
    ) -> Result<StmtKind, Reported> {
        let bscope = self.labelled_scope(ScopeKind::Block, scope, scope, s.label, id)?;
        let mut errs = Errs::default();
        let guard = match &b.guard {
            Some(g) => {
                let cond = errs.keep(self.st_expr(bscope, g, Expected::Condition));
                match cond {
                    Some(c) => errs.keep(self.declare_guard(bscope, g.span, c)),
                    None => None,
                }
            }
            None => None,
        };
        self.hidden_types.clear();
        let generics = match &b.generics {
            Some(l) => errs.keep(self.st_interfaces(bscope, l, ObjRole::Generic, s.span)),
            None => Some(Box::default()),
        };
        // Anonymous interface types (#283) come after the declared generics.
        let mut hidden = self.take_hidden_types();
        let generic_map = match (&generics, &b.generic_map) {
            (Some(g), Some(m)) => {
                let g = g.to_vec();
                errs.keep(self.st_assoc(scope, &g, m, s.span))
            }
            _ => Some(Box::default()),
        };
        let ports = match &b.ports {
            Some(l) => errs.keep(self.st_interfaces(bscope, l, ObjRole::Port, s.span)),
            None => Some(Box::default()),
        };
        hidden.extend(self.take_hidden_types());
        let port_map = match (&ports, &b.port_map) {
            (Some(p), Some(m)) => {
                let p = p.to_vec();
                errs.keep(self.st_assoc(scope, &p, m, s.span))
            }
            _ => Some(Box::default()),
        };
        let generics = generics.map(|g| {
            let mut v = g.into_vec();
            v.extend(&hidden);
            Box::<[DeclId]>::from(v)
        });
        let generic_map = match (&generics, generic_map, &ports, &port_map) {
            (Some(g), Some(m), Some(p), Some(pm)) if !hidden.is_empty() => {
                errs.keep(self.with_hidden_args(g, p, m, pm, s.span))
            }
            (_, m, _, _) => m,
        };
        errs.keep(self.st_decls(bscope, &b.decls, s.span));
        let body = errs.keep(self.conc_stmts(bscope, &b.stmts, false));
        errs.finish(())?;
        // Generic types, subprograms and packages: cloned with the actuals at the end of
        // the architecture (`Db::instantiate_blocks`, #222).
        if let Some(g) = &generics
            && g.iter().any(|&d| self.is_non_constant_generic(d))
            && let Some(cx) = self.cur()
        {
            self.generic.blocks.push((cx.unit, id));
        }
        Ok(StmtKind::Block {
            scope: bscope,
            guard,
            generics: generics.unwrap_or_default(),
            generic_map: generic_map.unwrap_or_default(),
            ports: ports.unwrap_or_default(),
            port_map: port_map.unwrap_or_default(),
            body: body.unwrap_or_default(),
        })
    }

    /// The implicit `GUARD` signal of a guarded block (11.2); its default is the guard
    /// condition, which the block's guard process assigns to it.
    fn declare_guard(
        &mut self,
        scope: ScopeId,
        span: Span,
        cond: ExprId,
    ) -> Result<DeclId, Reported> {
        let boolean = self.well_known_type(self.well_known.boolean, span, "boolean")?;
        let subtype = self.types[boolean].first_subtype;
        let name = Ident {
            text: Symbol::intern("GUARD"),
            key: Symbol::intern("guard"),
            span,
        };
        self.declare_plain(
            scope,
            Decl {
                name: Designator::Ident(name),
                kind: DeclKind::Object(Object {
                    class: ObjClass::Signal,
                    role: ObjRole::Guard,
                    mode: None,
                    subtype,
                    default: Some(cond),
                    bounds: BoundsFrom::Subtype,
                    signal_kind: None,
                    file_open: None,
                }),
                scope,
                span,
                implicit: true,
                since: LangRev::Vhdl1993,
            },
        )
    }

    fn generate_body(
        &mut self,
        gscope: ScopeId,
        body: &ast::GenerateBody,
    ) -> Result<Box<[StmtId]>, Reported> {
        let mut errs = Errs::default();
        errs.keep(self.st_decls(gscope, &body.decls, body.span));
        let stmts = errs.keep(self.conc_stmts(gscope, &body.stmts, false));
        errs.finish(stmts.unwrap_or_default())
    }

    fn gen_alt(
        &mut self,
        scope: ScopeId,
        owner: Option<DeclId>,
        label: Option<Ident>,
        cond: Option<ExprId>,
        body: &ast::GenerateBody,
    ) -> Result<GenAlt, Reported> {
        let gscope = self.add_scope(ScopeKind::Generate, Some(scope), owner);
        let body = self.generate_body(gscope, body)?;
        Ok(GenAlt {
            label,
            cond,
            scope: gscope,
            body,
        })
    }

    /// An instance statement once its target is known: maps checked against
    /// the target's generics and ports.
    fn instance_kind(
        &mut self,
        scope: ScopeId,
        span: Span,
        target: InstTarget,
        gmap: &[ast::AssocElem],
        pmap: &[ast::AssocElem],
    ) -> Result<StmtKind, Reported> {
        let Some((gens, ports)) = self
            .inst_formals(&target)
            .map(|(g, p)| (g.to_vec(), p.to_vec()))
        else {
            return Err(self.ice(span, "instantiated unit has no generics and ports"));
        };
        let mut errs = Errs::default();
        // The generic map records the actuals of generic types in `inst_types`, so generic
        // subprograms and ports of those types take them (6.5.6.2).
        self.inst_types.clear();
        let generic_map = errs.keep(self.st_assoc(scope, &gens, gmap, span));
        let port_map = errs.keep(self.st_assoc(scope, &ports, pmap, span));
        self.inst_types.clear();
        // Entities with anonymous interface types: their hidden types take the actuals' (#283).
        let generic_map = match (generic_map, &port_map) {
            (Some(g), Some(p)) => errs.keep(self.with_hidden_args(&gens, &ports, g, p, span)),
            _ => None,
        };
        errs.finish(())?;
        Ok(StmtKind::Instance {
            target,
            scope,
            generic_map: generic_map.unwrap_or_default(),
            port_map: port_map.unwrap_or_default(),
        })
    }

    // -- hooks: modules owned by others (CONTRIBUTING.md, "Requests") ----------
    //
    // Each hook names the function stmt.rs needs. Until the owner provides
    // it, the hook reports `E9999` (never a silent default); then the body
    // becomes a one-line call.

    /// expr (lead): resolve an expression against what the context expects (3.1).
    fn st_expr(
        &mut self,
        scope: ScopeId,
        e: &ast::Expr,
        exp: Expected,
    ) -> Result<ExprId, Reported> {
        let id = self.expr(scope, e, exp);
        if self.is_poison_expr(id) {
            return Err(Reported);
        }
        self.check_linkage_reads(id, false)?;
        Ok(id)
    }

    /// expr (names): an assignment target; its root object is written, not read.
    fn st_target(
        &mut self,
        scope: ScopeId,
        e: &ast::Expr,
        exp: Expected,
    ) -> Result<ExprId, Reported> {
        let id = self.expr(scope, e, exp);
        if self.is_poison_expr(id) {
            return Err(Reported);
        }
        self.check_linkage_reads(id, true)?;
        self.not_read(id);
        Ok(id)
    }

    /// expr (lead): a name as an object or value without expected type
    /// (sensitivity lists and `wait on`).
    fn st_name(&mut self, scope: ScopeId, n: &ast::Name) -> Result<ExprId, Reported> {
        let saved = std::mem::take(&mut self.memo);
        let cs = self.name_cands(scope, n);
        let r = match cs {
            crate::expr::CandSet::Types(is) if is.len() == 1 => {
                let e = self.build_name(scope, n, is[0], n.span);
                if self.is_poison_expr(e) {
                    Err(Reported)
                } else {
                    Ok(e)
                }
            }
            _ => Err(self.error(
                Diagnostic::error(diag::CLASS_MISUSE, "this name does not denote a signal")
                    .with_primary(n.span, "a signal name is required here"),
            )),
        };
        self.memo = saved;
        r
    }

    /// expr (lead): a procedure call statement: overload resolution with a
    /// procedure result, complete argument list in formal order.
    fn st_proc_call(&mut self, scope: ScopeId, n: &ast::Name) -> Result<Call, Reported> {
        self.proc_call(scope, n)
    }

    /// types (lead): a discrete range and the subtype a loop or generate
    /// parameter gets from it (10.10, 11.8).
    fn st_discrete_range(
        &mut self,
        scope: ScopeId,
        r: &ast::DiscreteRange,
    ) -> Result<(RangeC, SubtypeId), Reported> {
        let span = drange_span(r);
        let (range, ty) = self.discrete_range(scope, r, None)?;
        // The parameter's subtype: its type constrained by the range (10.10, 11.8).
        let st = self.subtypes.push(crate::types::SubtypeDef {
            base: ty,
            name: None,
            parent: Some(self.types[ty].first_subtype),
            constraint: crate::types::Constraint::Range(range.clone()),
            resolution: None,
            span,
        });
        Ok((range, st))
    }

    /// aggregate: resolve and check the choices of every alternative of a
    /// `case`, `case?`, selected assignment or case generate against the
    /// subject (coverage and duplicates, E1820), one `ChoiceSet` per
    /// alternative in order.
    fn st_case_choices(
        &mut self,
        scope: ScopeId,
        subject: ExprId,
        matching: bool,
        alts: &[&[ast::Choice]],
        span: Span,
    ) -> Result<Vec<ChoiceSet>, Reported> {
        use crate::expr::aggregate::{
            check_case, check_case_others, check_case_subject, resolve_choices,
        };
        let st = self.exprs[subject].subtype;
        let ty = self.base(st);
        check_case_subject(self, st, self.exprs[subject].span)?;
        check_case_others(self, alts)?;
        // An aggregate choice takes the case expression's subtype (9.3.3.3).
        let mut r = ChoiceResolver(scope, st);
        let mut errs = Errs::default();
        let mut resolved = Vec::with_capacity(alts.len());
        for a in alts {
            if let Some(c) = errs.keep(resolve_choices(self, &mut r, a, ty)) {
                resolved.push(c.into_boxed_slice());
            }
        }
        errs.finish(())?;
        check_case(self, st, matching, resolved, span)
    }

    /// decl (lead): a declarative part analyzed into `scope`.
    /// `span`: the statement that owns the region (diagnostics of the stub only).
    fn st_decls(
        &mut self,
        scope: ScopeId,
        decls: &[ast::Decl],
        span: Span,
    ) -> Result<(), Reported> {
        let _ = span;
        let before = self.error_count();
        self.decls_in(scope, decls);
        if self.error_count() > before {
            Err(Reported)
        } else {
            Ok(())
        }
    }

    /// unit: an interface list (block generics and ports).
    fn st_interfaces(
        &mut self,
        scope: ScopeId,
        list: &[ast::InterfaceDecl],
        role: ObjRole,
        _span: Span,
    ) -> Result<Box<[DeclId]>, Reported> {
        self.interface_list(scope, list, role)
    }

    /// unit: an association list against `formals` (generic and port
    /// maps): complete, in formal order, missing actuals reported (E1311)
    /// at `span`. Actuals are resolved in `scope`.
    fn st_assoc(
        &mut self,
        scope: ScopeId,
        formals: &[DeclId],
        assocs: &[ast::AssocElem],
        span: Span,
    ) -> Result<Box<[Arg]>, Reported> {
        self.assoc_list(scope, formals, assocs, span)
    }

    /// unit: the component, entity (with architecture) or configuration an
    /// instance names.
    fn st_inst_target(
        &mut self,
        scope: ScopeId,
        u: &ast::InstantiatedUnit,
    ) -> Result<InstTarget, Reported> {
        self.inst_target(scope, u)
    }

    /// scope (lead): the `GUARD` signal visible at a guarded assignment (11.6).
    fn st_lookup_guard(&mut self, scope: ScopeId, span: Span) -> Result<DeclId, Reported> {
        let key = Symbol::intern("guard");
        match self.lookup(scope, key) {
            crate::scope::Visible::One(d) => Ok(d),
            _ => Err(self.error(
                Diagnostic::error(
                    diag::NOT_DECLARED,
                    "a guarded assignment needs a visible `GUARD` signal",
                )
                .with_primary(span, "")
                .with_help("put it in a block with a guard condition: `b: block (cond) is ...`"),
            )),
        }
    }
}

/// Case choices resolved in the statement's region (aggregate.rs's resolver interface).
struct ChoiceResolver(ScopeId, SubtypeId);

impl crate::expr::aggregate::AggResolver for ChoiceResolver {
    fn cands(&mut self, db: &mut Db, e: &ast::Expr) -> crate::expr::CandSet {
        db.cands(self.0, e)
    }
    fn resolve(&mut self, db: &mut Db, e: &ast::Expr, exp: Expected) -> Result<ExprId, Reported> {
        db.context_subtype = Some(self.1);
        let id = db.static_expr(self.0, e, exp);
        if db.is_poison_expr(id) {
            Err(Reported)
        } else {
            Ok(id)
        }
    }
    fn discrete_range(
        &mut self,
        db: &mut Db,
        r: &ast::DiscreteRange,
        index_ty: TypeId,
    ) -> Result<RangeC, Reported> {
        db.discrete_range(self.0, r, Some(index_ty)).map(|(r, _)| r)
    }
    fn type_mark(&mut self, db: &mut Db, e: &ast::Expr) -> Option<SubtypeId> {
        match &e.kind {
            ast::ExprKind::Name(n) => db.quiet_type_mark(self.0, n),
            _ => None,
        }
    }
}

/// Resolves one right-hand side piece, given the target subtype if known.
/// The alternatives of a variable or force assignment value with an `unaffected` in it.
fn unaffected_alts(rhs: &ast::Rhs<ast::Expr>) -> Option<Vec<(&ast::Expr, Option<&ast::Expr>)>> {
    let alts: Vec<_> = match rhs {
        ast::Rhs::Simple(v) => vec![(v, None)],
        ast::Rhs::Conditional(alts) => alts.iter().map(|(v, c)| (v, c.as_ref())).collect(),
        ast::Rhs::Selected { .. } => return None,
    };
    alts.iter()
        .any(|(v, _)| matches!(v.kind, ast::ExprKind::Unaffected))
        .then_some(alts)
}

type Each<'a, A, H> = dyn FnMut(&mut Db, &A, Option<SubtypeId>) -> Result<H, Reported> + 'a;

/// The value type of a resolved right-hand side piece.
trait HasType {
    fn ty(&self, db: &Db) -> Option<TypeId>;
}

impl HasType for ExprId {
    fn ty(&self, db: &Db) -> Option<TypeId> {
        Some(db.expr_type(*self))
    }
}

impl HasType for Waveform {
    fn ty(&self, db: &Db) -> Option<TypeId> {
        match self {
            Waveform::Elements(els) => els.iter().find_map(|e| e.value).map(|v| db.expr_type(v)),
            Waveform::Unaffected => None,
        }
    }
}

fn rhs_type<H: HasType>(db: &Db, rhs: &Rhs<H>) -> Option<TypeId> {
    match rhs {
        Rhs::Simple(h) => h.ty(db),
        Rhs::Conditional(alts) => alts.iter().find_map(|(h, _)| h.ty(db)),
        Rhs::Selected { alts, .. } => alts.iter().find_map(|(h, _)| h.ty(db)),
    }
}

#[cfg(test)]
mod tests;

fn drange_span(r: &ast::DiscreteRange) -> Span {
    match r {
        ast::DiscreteRange::Subtype(s) => s.span,
        ast::DiscreteRange::Range(ast::Range::Simple { left, right, .. }) => {
            left.span.to(right.span)
        }
        ast::DiscreteRange::Range(ast::Range::Name(n)) => n.span,
    }
}
