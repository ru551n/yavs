//! Longest static prefixes, drivers and `process (all)` sensitivity (sem.md
//! 6.9; LRM 8.1, 10.2, 11.3, 14.7.2), and the multiple-source check for
//! unresolved signals. Rule 2: everything here is per sub-element, keyed by
//! longest static prefix, so M2 builds one driver map from it (sim1 #856).
//!
//! All functions work on resolved HIR. The data types live in [`crate::hir`].

// The entry points are called from decl.rs and unit.rs once those land.
#![allow(dead_code)]

use std::collections::HashMap;

use yavs_diag::{Diagnostic, Span};

use crate::db::Db;
use crate::decl::{Alias, DeclKind, ObjClass, ObjRole};
use crate::diag;
use crate::hir::{
    Actual, AttrPrefix, Call, Choice, ExprKind, InstTarget, Lsp, LspStep, Mode, NewInit, ObjRoot,
    PredefAttr, ProcessInfo, RangeC, Rhs, StmtKind, Target, UnitHir, Waveform,
};
use crate::ids::{DeclId, ExprId, ScopeId, StmtId, SubtypeId};
use crate::stat::Staticness;
use crate::types::{Resolution, TypeKind};

/// What a name denotes at its root, after following object aliases.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NameRoot {
    Obj(ObjRoot),
    /// The name goes through an access value (`p.all`, `p.f`): an allocated variable.
    Deref,
}

/// How an expression is used by its statement (for the `all` and purity walks).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Use {
    /// Evaluated: every signal in it is read.
    Value,
    /// An assignment target: only its index and slice expressions are read.
    Target,
    /// An actual of a procedure call, with the formal's mode.
    Actual(Mode),
}

/// Whether two longest static prefixes share a sub-element.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Overlap {
    Yes,
    No,
    /// Depends on values only elab knows (globally static indices).
    Maybe,
}

impl Db {
    // -- longest static prefixes (8.1) ----------------------------------------

    /// The longest static prefix of a name, or `None` when `e` is not an object
    /// name (a call, a literal, an aggregate, ...). Globally static indices are
    /// kept (elab evaluates them, sim1 #856); a non-static index or a
    /// dereference ends the prefix.
    pub fn lsp(&self, e: ExprId) -> Option<Lsp> {
        let (root, path, stat, _) = self.lsp_parts(e)?;
        Some(Lsp {
            root,
            path: path.into(),
            stat,
            span: self.exprs[e].span,
        })
    }

    /// `(root, path, staticness, cut)`; `cut` once a non-static step was met.
    pub(crate) fn lsp_parts(&self, e: ExprId) -> Option<(ObjRoot, Vec<LspStep>, Staticness, bool)> {
        match &self.exprs[e].kind {
            ExprKind::Obj(d) => match &self.decls[*d].kind {
                DeclKind::Alias(Alias::Object { target, subtype }) => {
                    let mut p = self.lsp_parts(*target)?;
                    if let Some(st) = *subtype
                        && !p.3
                        && matches!(self.types[self.base(st)].kind, TypeKind::Array { .. })
                    {
                        p.1.push(LspStep::Rebase(st));
                    }
                    Some(p)
                }
                DeclKind::Object(o) if o.role == ObjRole::ImplicitSignal => Some((
                    ObjRoot::ImplicitSignal(*d),
                    Vec::new(),
                    Staticness::Locally,
                    false,
                )),
                DeclKind::Object(_) | DeclKind::Error => {
                    Some((ObjRoot::Decl(*d), Vec::new(), Staticness::Locally, false))
                }
                _ => None,
            },
            ExprKind::External(x) => Some((
                ObjRoot::External(*x),
                Vec::new(),
                Staticness::Locally,
                false,
            )),
            ExprKind::Index { prefix, indices } => {
                let mut p = self.lsp_parts(*prefix)?;
                if p.3 {
                    return Some(p);
                }
                let s = indices
                    .iter()
                    .map(|&i| self.exprs[i].stat)
                    .min()
                    .unwrap_or(Staticness::Locally);
                if s == Staticness::None {
                    p.3 = true;
                    return Some(p);
                }
                // An index after a slice addresses the same array level.
                if matches!(p.1.last(), Some(LspStep::Slice(_))) {
                    p.1.pop();
                }
                p.1.push(LspStep::Index(indices.clone()));
                p.2 = p.2.min(s);
                Some(p)
            }
            ExprKind::Slice { prefix, range } => {
                let mut p = self.lsp_parts(*prefix)?;
                if p.3 {
                    return Some(p);
                }
                let s = self.range_stat(range);
                if s == Staticness::None {
                    p.3 = true;
                    return Some(p);
                }
                if matches!(p.1.last(), Some(LspStep::Slice(_))) {
                    p.1.pop();
                }
                p.1.push(LspStep::Slice(range.clone()));
                p.2 = p.2.min(s);
                Some(p)
            }
            ExprKind::Field { prefix, elem } => {
                let mut p = self.lsp_parts(*prefix)?;
                if !p.3 {
                    p.1.push(LspStep::Field(*elem));
                }
                Some(p)
            }
            ExprKind::Deref { prefix } => {
                let mut p = self.lsp_parts(*prefix)?;
                p.3 = true;
                Some(p)
            }
            _ => None,
        }
    }

    fn range_stat(&self, r: &RangeC) -> Staticness {
        match r {
            RangeC::Explicit { left, right, .. } => {
                self.exprs[*left].stat.min(self.exprs[*right].stat)
            }
            // `x'range` is static unless the prefix is an object that exists only while a
            // process or subprogram runs and whose subtype leaves the bounds open: a
            // parameter `d : bit_vector` (the longest static prefix of `s(d'range)` is
            // `s`, #518).
            // ponytail (#42): other prefixes count as static; exact only once types say
            // whether the prefix's subtype is static (S3).
            RangeC::Attr {
                prefix: AttrPrefix::Expr(p),
                ..
            } if self.bounds_known_at_run_time_only(*p) => Staticness::None,
            RangeC::Attr { .. } => Staticness::Globally,
        }
    }

    /// Whether the bounds of object `e` are only known while a process or subprogram runs: its
    /// subtype leaves them open (or takes them from an object that does) and the object is
    /// a parameter or declared in a process or subprogram, so elaboration has no value for it.
    fn bounds_known_at_run_time_only(&self, e: ExprId) -> bool {
        use crate::types::{Constrainedness, Constraint};
        let st = self.exprs[e].subtype;
        if let Constraint::FromObject { expr, .. } = self.subtypes[st].constraint {
            return self.bounds_known_at_run_time_only(expr);
        }
        if self.constrainedness(st) == Constrainedness::Full {
            return false;
        }
        let Some(ObjRoot::Decl(d)) = self.lsp(e).map(|l| l.root) else {
            return false;
        };
        let DeclKind::Object(o) = &self.decls[d].kind else {
            return false;
        };
        if o.role == ObjRole::Param {
            return true;
        }
        let mut sc = Some(self.decls[d].scope);
        while let Some(s) = sc {
            match self.scopes[s].kind {
                crate::scope::ScopeKind::Process | crate::scope::ScopeKind::Subprogram => {
                    return true;
                }
                crate::scope::ScopeKind::Loop | crate::scope::ScopeKind::SeqBlock => {
                    sc = self.scopes[s].parent;
                }
                _ => return false,
            }
        }
        false
    }

    /// The root of a name, following object aliases; `None` when `e` is not a name.
    pub fn name_root(&self, e: ExprId) -> Option<NameRoot> {
        match &self.exprs[e].kind {
            ExprKind::Obj(_) | ExprKind::External(_) => {
                self.lsp_parts(e).map(|p| NameRoot::Obj(p.0))
            }
            ExprKind::Index { prefix, .. }
            | ExprKind::Slice { prefix, .. }
            | ExprKind::Field { prefix, .. } => self.name_root(*prefix),
            ExprKind::Deref { prefix } => self.name_root(*prefix).map(|_| NameRoot::Deref),
            _ => None,
        }
    }

    /// Whether the root denotes a signal (explicit, port, parameter, implicit,
    /// `GUARD`, external signal).
    pub fn is_signal(&self, root: ObjRoot) -> bool {
        match root {
            ObjRoot::ImplicitSignal(_) => true,
            ObjRoot::Decl(d) => matches!(
                &self.decls[d].kind,
                DeclKind::Object(o) if o.class == ObjClass::Signal
            ),
            ObjRoot::External(x) => {
                self.externals[x].class == yavs_syntax::ast::ExternalClass::Signal
            }
        }
    }

    fn is_param(&self, root: ObjRoot) -> bool {
        matches!(root, ObjRoot::Decl(d) if matches!(
            &self.decls[d].kind,
            DeclKind::Object(o) if o.role == ObjRole::Param
        ))
    }

    /// The LSP of `e` when it is a signal name.
    fn signal_lsp(&self, e: ExprId) -> Option<Lsp> {
        self.lsp(e).filter(|l| self.is_signal(l.root))
    }

    // -- walks ------------------------------------------------------------------

    /// Visit `e` and every expression below it, prefix first.
    pub fn walk_expr(&self, e: ExprId, f: &mut dyn FnMut(ExprId)) {
        f(e);
        match &self.exprs[e].kind {
            ExprKind::Index { prefix, indices } => {
                self.walk_expr(*prefix, f);
                for &i in indices.iter() {
                    self.walk_expr(i, f);
                }
            }
            ExprKind::Slice { prefix, range } => {
                self.walk_expr(*prefix, f);
                self.walk_range(range, f);
            }
            ExprKind::Field { prefix, .. }
            | ExprKind::Deref { prefix }
            | ExprKind::PrivateVar { object: prefix, .. } => self.walk_expr(*prefix, f),
            ExprKind::Call {
                args, method_of, ..
            } => {
                if let Some(m) = method_of {
                    self.walk_expr(*m, f);
                }
                for a in args.iter() {
                    self.walk_actual(&a.actual, f);
                }
            }
            ExprKind::Attr { prefix, args, .. } => {
                if let AttrPrefix::Expr(p) = prefix {
                    self.walk_expr(*p, f);
                }
                for &a in args.iter() {
                    self.walk_expr(a, f);
                }
            }
            ExprKind::Aggregate { assocs, .. } => {
                for a in assocs.iter() {
                    for c in a.choices.iter() {
                        self.walk_choice(c, f);
                    }
                    self.walk_expr(a.value, f);
                }
            }
            ExprKind::Qualified { operand, .. }
            | ExprKind::Conv { operand, .. }
            | ExprKind::ImplicitConv { operand } => self.walk_expr(*operand, f),
            ExprKind::New {
                init: NewInit::Value(v),
            } => self.walk_expr(*v, f),
            ExprKind::Conditional { alts } => {
                for (v, c) in alts.iter() {
                    self.walk_expr(*v, f);
                    if let Some(c) = c {
                        self.walk_expr(*c, f);
                    }
                }
            }
            ExprKind::Const(_)
            | ExprKind::Obj(_)
            | ExprKind::External(_)
            | ExprKind::AttrValue { .. }
            | ExprKind::New { .. }
            | ExprKind::Null
            | ExprKind::Open
            | ExprKind::Error => {}
        }
    }

    fn walk_range(&self, r: &RangeC, f: &mut dyn FnMut(ExprId)) {
        match r {
            RangeC::Explicit { left, right, .. } => {
                self.walk_expr(*left, f);
                self.walk_expr(*right, f);
            }
            RangeC::Attr {
                prefix: AttrPrefix::Expr(p),
                ..
            } => self.walk_expr(*p, f),
            RangeC::Attr { .. } => {}
        }
    }

    fn walk_choice(&self, c: &Choice, f: &mut dyn FnMut(ExprId)) {
        match c {
            Choice::Expr(e) => self.walk_expr(*e, f),
            Choice::Range(r) => self.walk_range(r, f),
            Choice::Element(_) | Choice::Others => {}
        }
    }

    fn walk_actual(&self, a: &Actual, f: &mut dyn FnMut(ExprId)) {
        match a {
            Actual::Expr(e) | Actual::Converted { actual: e, .. } => self.walk_expr(*e, f),
            Actual::Individual(parts) => {
                for (_, e, _, _) in parts.iter() {
                    self.walk_expr(*e, f);
                }
            }
            Actual::Default
            | Actual::Open
            | Actual::Subtype(_)
            | Actual::Subprogram(_)
            | Actual::Package(_) => {}
        }
    }

    /// The mode of formal `i` of a subprogram (`In` when unknown).
    fn formal_mode(&self, callee: DeclId, i: u16) -> (Mode, ObjClass) {
        self.decls[callee]
            .kind
            .subprogram()
            .and_then(|s| s.params.get(usize::from(i)))
            .map_or((Mode::In, ObjClass::Constant), |p| (p.mode, p.class))
    }

    /// The top-level expressions of one statement with their use; child
    /// statements are not visited.
    pub fn stmt_exprs(&self, s: StmtId, f: &mut dyn FnMut(ExprId, Use)) {
        let target = |t: &Target, f: &mut dyn FnMut(ExprId, Use)| f(t.expr, Use::Target);
        match &self.stmts[s].kind {
            StmtKind::Wait { until, timeout, .. } => {
                // The `on` names are read only through the sensitivity.
                for e in until.iter().chain(timeout) {
                    f(*e, Use::Value);
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
                f(*cond, Use::Value);
                for e in report.iter().chain(severity) {
                    f(*e, Use::Value);
                }
            }
            StmtKind::Report { report, severity } => {
                f(*report, Use::Value);
                if let Some(e) = severity {
                    f(*e, Use::Value);
                }
            }
            StmtKind::SignalAssign {
                target: t,
                delay,
                rhs,
            }
            | StmtKind::ConcSignalAssign {
                target: t,
                delay,
                rhs,
                ..
            } => {
                target(t, f);
                if let crate::hir::Delay::Inertial { reject: Some(r) } = delay {
                    f(*r, Use::Value);
                }
                self.rhs_exprs(rhs, f, &mut |w, f| {
                    if let Waveform::Elements(els) = w {
                        for el in els.iter() {
                            for e in el.value.iter().chain(&el.after) {
                                f(*e, Use::Value);
                            }
                        }
                    }
                });
            }
            StmtKind::Force { target: t, rhs, .. } | StmtKind::VarAssign { target: t, rhs } => {
                target(t, f);
                self.rhs_exprs(rhs, f, &mut |e, f| f(*e, Use::Value));
            }
            StmtKind::Release { target: t, .. } => target(t, f),
            StmtKind::ProcCall(c) | StmtKind::ConcProcCall { call: c, .. } => self.call_exprs(c, f),
            StmtKind::If { branches, .. } => {
                for (c, _) in branches.iter() {
                    f(*c, Use::Value);
                }
            }
            StmtKind::Case { expr, alts, .. } => {
                f(*expr, Use::Value);
                for (cs, _) in alts.iter() {
                    self.choice_exprs(&cs.choices, f);
                }
            }
            StmtKind::Loop { scheme, .. } => match scheme {
                crate::hir::LoopScheme::Forever => {}
                crate::hir::LoopScheme::While(c) => f(*c, Use::Value),
                crate::hir::LoopScheme::For { range, .. } => {
                    self.walk_range_top(range, f);
                }
            },
            StmtKind::Next { when, .. } | StmtKind::Exit { when, .. } => {
                if let Some(c) = when {
                    f(*c, Use::Value);
                }
            }
            StmtKind::Return(v) => {
                if let Some(v) = v {
                    f(*v, Use::Value);
                }
            }
            StmtKind::Null | StmtKind::SeqBlock { .. } => {}
            StmtKind::Process { .. } | StmtKind::Block { .. } => {}
            StmtKind::Instance {
                generic_map,
                port_map,
                ..
            } => {
                for a in generic_map.iter().chain(port_map.iter()) {
                    self.walk_actual_top(&a.actual, Use::Value, f);
                }
            }
            StmtKind::ForGenerate { range, .. } => self.walk_range_top(range, f),
            StmtKind::IfGenerate { alts } => {
                for a in alts.iter() {
                    if let Some(c) = a.cond {
                        f(c, Use::Value);
                    }
                }
            }
            StmtKind::CaseGenerate { expr, alts } => {
                f(*expr, Use::Value);
                for (cs, _) in alts.iter() {
                    self.choice_exprs(&cs.choices, f);
                }
            }
        }
    }

    fn rhs_exprs<T>(
        &self,
        rhs: &Rhs<T>,
        f: &mut dyn FnMut(ExprId, Use),
        each: &mut ExprEach<'_, T>,
    ) {
        match rhs {
            Rhs::Simple(t) => each(t, f),
            Rhs::Conditional(alts) => {
                for (t, c) in alts.iter() {
                    each(t, f);
                    if let Some(c) = c {
                        f(*c, Use::Value);
                    }
                }
            }
            Rhs::Selected { expr, alts, .. } => {
                f(*expr, Use::Value);
                for (t, cs) in alts.iter() {
                    each(t, f);
                    self.choice_exprs(&cs.choices, f);
                }
            }
        }
    }

    fn choice_exprs(&self, cs: &[Choice], f: &mut dyn FnMut(ExprId, Use)) {
        for c in cs {
            match c {
                Choice::Expr(e) => f(*e, Use::Value),
                Choice::Range(r) => self.walk_range_top(r, f),
                Choice::Element(_) | Choice::Others => {}
            }
        }
    }

    fn walk_range_top(&self, r: &RangeC, f: &mut dyn FnMut(ExprId, Use)) {
        match r {
            RangeC::Explicit { left, right, .. } => {
                f(*left, Use::Value);
                f(*right, Use::Value);
            }
            RangeC::Attr {
                prefix: AttrPrefix::Expr(p),
                ..
            } => f(*p, Use::Target),
            RangeC::Attr { .. } => {}
        }
    }

    fn walk_actual_top(&self, a: &Actual, u: Use, f: &mut dyn FnMut(ExprId, Use)) {
        match a {
            Actual::Expr(e) | Actual::Converted { actual: e, .. } => f(*e, u),
            Actual::Individual(parts) => {
                for (_, e, _, _) in parts.iter() {
                    f(*e, u);
                }
            }
            Actual::Default
            | Actual::Open
            | Actual::Subtype(_)
            | Actual::Subprogram(_)
            | Actual::Package(_) => {}
        }
    }

    fn call_exprs(&self, c: &Call, f: &mut dyn FnMut(ExprId, Use)) {
        if let Some(m) = c.method_of {
            f(m, Use::Value);
        }
        for a in c.args.iter() {
            let (mode, _) = self.formal_mode(c.callee, a.formal);
            self.walk_actual_top(&a.actual, Use::Actual(mode), f);
        }
    }

    /// The child statement lists of a statement.
    pub fn stmt_children(&self, s: StmtId) -> Vec<StmtId> {
        let mut out = Vec::new();
        match &self.stmts[s].kind {
            StmtKind::If { branches, else_ } => {
                for (_, b) in branches.iter() {
                    out.extend_from_slice(b);
                }
                if let Some(b) = else_ {
                    out.extend_from_slice(b);
                }
            }
            StmtKind::Case { alts, .. } => {
                for (_, b) in alts.iter() {
                    out.extend_from_slice(b);
                }
            }
            StmtKind::Loop { body, .. }
            | StmtKind::SeqBlock { body, .. }
            | StmtKind::Process { body, .. }
            | StmtKind::Block { body, .. }
            | StmtKind::ForGenerate { body, .. } => out.extend_from_slice(body),
            StmtKind::IfGenerate { alts } => {
                for a in alts.iter() {
                    out.extend_from_slice(&a.body);
                }
            }
            StmtKind::CaseGenerate { alts, .. } => {
                for (_, a) in alts.iter() {
                    out.extend_from_slice(&a.body);
                }
            }
            _ => {}
        }
        out
    }

    // -- signals read (10.2, 11.3) ----------------------------------------------

    /// Add the LSP of every signal read by evaluating `e` (10.2).
    pub fn reads(&self, e: ExprId, out: &mut Vec<Lsp>) {
        match &self.exprs[e].kind {
            ExprKind::Obj(_)
            | ExprKind::External(_)
            | ExprKind::Index { .. }
            | ExprKind::Slice { .. }
            | ExprKind::Field { .. }
            | ExprKind::Deref { .. }
            | ExprKind::PrivateVar { .. } => {
                if let Some(l) = self.signal_lsp(e) {
                    push_unique(self, out, l);
                }
                self.name_reads(e, out);
            }
            ExprKind::Attr { attr, prefix, args } => {
                use PredefAttr::*;
                if let AttrPrefix::Expr(p) = prefix
                    && matches!(
                        attr,
                        Event
                            | Active
                            | LastEvent
                            | LastActive
                            | LastValue
                            | Driving
                            | DrivingValue
                            | Delayed
                            | Stable
                            | Quiet
                            | Transaction
                    )
                {
                    self.reads(*p, out);
                }
                for &a in args.iter() {
                    self.reads(a, out);
                }
            }
            ExprKind::Call {
                args, method_of, ..
            } => {
                if let Some(m) = method_of {
                    self.reads(*m, out);
                }
                for a in args.iter() {
                    self.actual_reads(&a.actual, out);
                }
            }
            ExprKind::Aggregate { assocs, .. } => {
                for a in assocs.iter() {
                    // Choices are read too (10.2): `(1 to i => '1')` is sensitive to `i`.
                    for c in a.choices.iter() {
                        match c {
                            Choice::Expr(e) => self.reads(*e, out),
                            Choice::Range(RangeC::Explicit { left, right, .. }) => {
                                self.reads(*left, out);
                                self.reads(*right, out);
                            }
                            _ => {}
                        }
                    }
                    self.reads(a.value, out);
                }
            }
            ExprKind::Qualified { operand, .. }
            | ExprKind::Conv { operand, .. }
            | ExprKind::ImplicitConv { operand } => self.reads(*operand, out),
            ExprKind::New {
                init: NewInit::Value(v),
            } => self.reads(*v, out),
            ExprKind::Conditional { alts } => {
                for (v, c) in alts.iter() {
                    self.reads(*v, out);
                    if let Some(c) = c {
                        self.reads(*c, out);
                    }
                }
            }
            ExprKind::Const(_)
            | ExprKind::AttrValue { .. }
            | ExprKind::New { .. }
            | ExprKind::Null
            | ExprKind::Open
            | ExprKind::Error => {}
        }
    }

    /// Signals read by the index and slice expressions of a name, and by a
    /// prefix that is not a name (`f(x)(i)`).
    fn name_reads(&self, e: ExprId, out: &mut Vec<Lsp>) {
        match &self.exprs[e].kind {
            ExprKind::Index { prefix, indices } => {
                for &i in indices.iter() {
                    self.reads(i, out);
                }
                self.name_reads(*prefix, out);
            }
            ExprKind::Slice { prefix, range } => {
                if let RangeC::Explicit { left, right, .. } = range {
                    self.reads(*left, out);
                    self.reads(*right, out);
                }
                self.name_reads(*prefix, out);
            }
            ExprKind::Field { prefix, .. } | ExprKind::Deref { prefix } => {
                self.name_reads(*prefix, out)
            }
            ExprKind::Obj(_) | ExprKind::External(_) => {}
            _ => self.reads(e, out),
        }
    }

    fn actual_reads(&self, a: &Actual, out: &mut Vec<Lsp>) {
        let mut v = Vec::new();
        self.walk_actual_top(a, Use::Value, &mut |e, _| v.push(e));
        for e in v {
            self.reads(e, out);
        }
    }

    /// Signals read by one statement (not its children), for `process (all)`
    /// and the equivalent processes of concurrent statements.
    pub fn stmt_reads(&self, s: StmtId, out: &mut Vec<Lsp>) {
        let mut uses = Vec::new();
        self.stmt_exprs(s, &mut |e, u| uses.push((e, u)));
        for (e, u) in uses {
            match u {
                Use::Value | Use::Actual(Mode::In | Mode::Inout) => self.reads(e, out),
                Use::Target | Use::Actual(_) => self.name_reads(e, out),
            }
        }
    }

    // -- drivers (14.7.2) ---------------------------------------------------------

    /// Drivers of a statement list: signal assignment targets and signal
    /// actuals of `out`/`inout`/`buffer` formals in procedure calls, recursing
    /// into nested statements. Targets rooted at a parameter are driven by the
    /// caller's actual, so they are skipped.
    pub fn collect_drivers(&self, body: &[StmtId], out: &mut Vec<Lsp>) {
        for &s in body {
            match &self.stmts[s].kind {
                StmtKind::SignalAssign { target, .. }
                | StmtKind::ConcSignalAssign { target, .. } => {
                    for l in target.lsps.iter() {
                        if !self.is_param(l.root) {
                            push_unique(self, out, l.clone());
                        }
                    }
                }
                StmtKind::ProcCall(c) | StmtKind::ConcProcCall { call: c, .. } => {
                    self.call_drivers(c, out)
                }
                _ => {}
            }
            self.collect_drivers(&self.stmt_children(s), out);
        }
    }

    fn call_drivers(&self, c: &Call, out: &mut Vec<Lsp>) {
        for a in c.args.iter() {
            let (mode, class) = self.formal_mode(c.callee, a.formal);
            if class != ObjClass::Signal {
                continue;
            }
            // A mode view drives the actual's `out`, `inout` and `buffer` elements only
            // (6.5.2): one driver per such element of the view's mode tree.
            let driven: Vec<Vec<LspStep>> = match mode {
                Mode::Out | Mode::Inout | Mode::Buffer => vec![Vec::new()],
                Mode::View { .. } => self.view_driven_paths(mode),
                _ => continue,
            };
            // (actual, driven paths below it): an individual association's parts each
            // cover the part of the view their formal part names.
            let mut es = Vec::new();
            match &a.actual {
                Actual::Individual(parts) if matches!(mode, Mode::View { .. }) => {
                    for (steps, e, _, _) in parts.iter() {
                        es.push((*e, self.view_driven_below(mode, steps)));
                    }
                }
                _ => self.walk_actual_top(&a.actual, Use::Target, &mut |e, _| {
                    es.push((e, driven.clone()));
                }),
            }
            for (e, driven) in es {
                if let Some(l) = self.signal_lsp(e)
                    && !self.is_param(l.root)
                {
                    for p in &driven {
                        let mut l = l.clone();
                        if !p.is_empty() {
                            l.path = l.path.iter().chain(p).cloned().collect();
                        }
                        push_unique(self, out, l);
                    }
                }
            }
        }
    }

    /// Drivers in the bodies of subprograms declared in `scope` (and in their
    /// nested regions): a process drives what procedures declared inside it
    /// assign (14.7.2).
    fn scope_subprogram_drivers(&self, scope: ScopeId, out: &mut Vec<Lsp>) {
        for &d in &self.scopes[scope].order {
            if let Some(sp) = self.decls[d].kind.subprogram()
                && let crate::decl::Body::Hir { scope, stmts } = &sp.body
            {
                let mut ls = Vec::new();
                self.collect_drivers(stmts, &mut ls);
                for l in ls {
                    push_unique(self, out, self.cut_at_subprogram_values(l));
                }
                self.scope_subprogram_drivers(*scope, out);
            }
        }
    }

    /// A driver from a subprogram body ends before the first index or slice that reads an
    /// object of a subprogram (a parameter, or a constant computed from one): its value
    /// exists only when the call runs, so the driver is the prefix before it (14.7.2, #205).
    fn cut_at_subprogram_values(&self, mut l: Lsp) -> Lsp {
        let dynamic = |e: ExprId| {
            let mut hit = false;
            self.walk_expr(e, &mut |x| {
                if let ExprKind::Obj(d) = self.exprs[x].kind
                    && self.in_subprogram(self.decls[d].scope)
                {
                    hit = true;
                }
            });
            hit
        };
        let k = l.path.iter().position(|st| match st {
            LspStep::Field(_) | LspStep::Rebase(_) => false,
            LspStep::Index(ix) => ix.iter().any(|&e| dynamic(e)),
            LspStep::Slice(RangeC::Explicit { left, right, .. }) => {
                dynamic(*left) || dynamic(*right)
            }
            LspStep::Slice(RangeC::Attr { .. }) => false,
        });
        if let Some(mut k) = k {
            // A rebase only applies to the steps after it.
            while k > 0 && matches!(l.path[k - 1], LspStep::Rebase(_)) {
                k -= 1;
            }
            l.path = l.path[..k].into();
        }
        l
    }

    /// Drivers and (for `process (all)`) the signals read of a process
    /// (sem.md 6.9). `scope` is the process's declarative region.
    pub fn process_info(&self, scope: ScopeId, body: &[StmtId], all: bool) -> ProcessInfo {
        let mut drivers = Vec::new();
        self.collect_drivers(body, &mut drivers);
        self.scope_subprogram_drivers(scope, &mut drivers);
        let mut seq_scopes = Vec::new();
        self.seq_block_scopes(body, &mut seq_scopes);
        for s in seq_scopes {
            self.scope_subprogram_drivers(s, &mut drivers);
        }
        let reads_all = all.then(|| {
            let mut r = Vec::new();
            self.reads_deep(body, &mut r);
            r
        });
        ProcessInfo { drivers, reads_all }
    }

    fn seq_block_scopes(&self, body: &[StmtId], out: &mut Vec<ScopeId>) {
        for &s in body {
            if let StmtKind::SeqBlock { scope, .. } = &self.stmts[s].kind {
                out.push(*scope);
            }
            self.seq_block_scopes(&self.stmt_children(s), out);
        }
    }

    /// Signals read by a statement list and everything nested in it.
    pub fn reads_deep(&self, body: &[StmtId], out: &mut Vec<Lsp>) {
        for &s in body {
            self.stmt_reads(s, out);
            self.reads_deep(&self.stmt_children(s), out);
        }
    }

    /// The equivalent-process information of a concurrent statement (11.4 to
    /// 11.6): its drivers, and its sensitivity in `reads_all`. `guard` is the
    /// `GUARD` signal of a guarded assignment.
    pub fn conc_info(&self, s: StmtId, guard: Option<(DeclId, Span)>) -> ProcessInfo {
        let mut drivers = Vec::new();
        self.collect_drivers(&[s], &mut drivers);
        let mut reads = Vec::new();
        self.stmt_reads(s, &mut reads);
        // The assignment tests the guard condition itself, not the GUARD signal, which
        // updates a delta later: sensitive to the condition's signals (11.6).
        let cond = guard.and_then(|(g, _)| match &self.decls[g].kind {
            DeclKind::Object(o) if o.role == ObjRole::Guard => o.default,
            _ => None,
        });
        if let Some(c) = cond {
            self.reads(c, &mut reads);
        } else if let Some((g, span)) = guard {
            push_unique(
                self,
                &mut reads,
                Lsp {
                    root: ObjRoot::Decl(g),
                    path: Box::new([]),
                    stat: Staticness::Locally,
                    span,
                },
            );
        }
        ProcessInfo {
            drivers,
            reads_all: Some(reads),
        }
    }

    // -- comparing prefixes ---------------------------------------------------------

    fn folded(&self, e: ExprId) -> Option<i64> {
        self.value(e).and_then(|v| v.as_int())
    }

    fn folded_range(&self, r: &RangeC) -> Option<(i64, i64)> {
        match r {
            RangeC::Explicit { left, dir, right } => {
                let (l, r) = (self.folded(*left)?, self.folded(*right)?);
                Some(match dir {
                    crate::types::Direction::To => (l, r),
                    crate::types::Direction::Downto => (r, l),
                })
            }
            RangeC::Attr { .. } => None,
        }
    }

    /// Whether two prefixes share a sub-element.
    pub fn overlap(&self, a: &Lsp, b: &Lsp) -> Overlap {
        if a.root != b.root {
            return Overlap::No;
        }
        let mut res = Overlap::Yes;
        for (x, y) in a.path.iter().zip(b.path.iter()) {
            let step = match (x, y) {
                (LspStep::Field(i), LspStep::Field(j)) => {
                    if i == j {
                        Overlap::Yes
                    } else {
                        Overlap::No
                    }
                }
                (LspStep::Index(i), LspStep::Index(j)) => {
                    let vi: Option<Vec<i64>> = i.iter().map(|&e| self.folded(e)).collect();
                    let vj: Option<Vec<i64>> = j.iter().map(|&e| self.folded(e)).collect();
                    match (vi, vj) {
                        (Some(vi), Some(vj)) if vi == vj => Overlap::Yes,
                        (Some(_), Some(_)) => Overlap::No,
                        _ => Overlap::Maybe,
                    }
                }
                (LspStep::Index(i), LspStep::Slice(r)) | (LspStep::Slice(r), LspStep::Index(i)) => {
                    match (
                        i.first().and_then(|&e| self.folded(e)),
                        self.folded_range(r),
                    ) {
                        (Some(v), Some((lo, hi))) if lo <= v && v <= hi => Overlap::Yes,
                        (Some(_), Some(_)) => Overlap::No,
                        _ => Overlap::Maybe,
                    }
                }
                (LspStep::Slice(r), LspStep::Slice(q)) => {
                    match (self.folded_range(r), self.folded_range(q)) {
                        (Some((a0, a1)), Some((b0, b1))) => {
                            if a0 <= a1 && b0 <= b1 && a0 <= b1 && b0 <= a1 {
                                Overlap::Yes
                            } else {
                                Overlap::No
                            }
                        }
                        _ => Overlap::Maybe,
                    }
                }
                (LspStep::Rebase(s), LspStep::Rebase(q)) if s == q => Overlap::Yes,
                // The steps that follow are in different index spaces: elab compares them.
                (LspStep::Rebase(_), _) | (_, LspStep::Rebase(_)) => return Overlap::Maybe,
                // Different shapes at one level cannot come from one type.
                _ => Overlap::Maybe,
            };
            match step {
                Overlap::No => return Overlap::No,
                Overlap::Maybe => res = Overlap::Maybe,
                Overlap::Yes => {}
            }
        }
        res
    }

    /// Structural equality with folded indices (driver deduplication; lowering finds a
    /// target's driver with it).
    pub fn same_lsp(&self, a: &Lsp, b: &Lsp) -> bool {
        a.path.len() == b.path.len()
            && self.overlap(a, b) == Overlap::Yes
            && a.path.iter().zip(b.path.iter()).all(|(x, y)| match (x, y) {
                (LspStep::Field(_), LspStep::Field(_)) | (LspStep::Index(_), LspStep::Index(_)) => {
                    true
                }
                (LspStep::Slice(r), LspStep::Slice(q)) => {
                    self.folded_range(r).is_some() && self.folded_range(r) == self.folded_range(q)
                }
                (LspStep::Rebase(s), LspStep::Rebase(q)) => s == q,
                _ => false,
            })
    }

    // -- resolution and multiple sources ----------------------------------------

    /// Whether every scalar sub-element of the subtype is resolved.
    pub fn is_resolved(&self, st: SubtypeId) -> bool {
        let mut cur = Some(st);
        while let Some(s) = cur {
            match &self.subtypes[s].resolution {
                Some(Resolution::Function(_) | Resolution::Element(_)) => return true,
                Some(Resolution::Record(_)) => break,
                None => cur = self.subtypes[s].parent,
            }
        }
        match &self.types[self.subtypes[st].base].kind {
            TypeKind::Array { elem, .. } => self.is_resolved(*elem),
            TypeKind::Record { elems } => {
                let per: HashMap<u32, bool> = self.record_resolution(st);
                elems.iter().enumerate().all(|(i, el)| {
                    per.get(&(i as u32)).copied().unwrap_or(false) || self.is_resolved(el.subtype)
                })
            }
            TypeKind::Error | TypeKind::Generic { .. } => true,
            _ => false,
        }
    }

    /// Elements given a resolution by a record resolution indication.
    fn record_resolution(&self, st: SubtypeId) -> HashMap<u32, bool> {
        let mut cur = Some(st);
        while let Some(s) = cur {
            if let Some(Resolution::Record(r)) = &self.subtypes[s].resolution {
                return r.iter().map(|(i, _)| (*i, true)).collect();
            }
            cur = self.subtypes[s].parent;
        }
        HashMap::new()
    }

    /// The subtype of the sub-element a prefix denotes, and whether it is
    /// resolved: by its own subtype or by that of an enclosing part (the
    /// elements of a `(resolved) std_ulogic_vector` signal are, 4.6). `None`
    /// when unknown.
    fn lsp_subtype(&self, l: &Lsp) -> Option<(SubtypeId, bool)> {
        let ObjRoot::Decl(d) = l.root else {
            return None;
        };
        let DeclKind::Object(o) = &self.decls[d].kind else {
            return None;
        };
        let mut st = o.subtype;
        let mut resolved = self.is_resolved(st);
        for step in l.path.iter() {
            st = match (step, &self.types[self.subtypes[st].base].kind) {
                (LspStep::Field(i), TypeKind::Record { elems }) => elems.get(*i as usize)?.subtype,
                (LspStep::Index(_), TypeKind::Array { elem, .. }) => *elem,
                (LspStep::Slice(_), TypeKind::Array { .. }) => st,
                (LspStep::Rebase(s), TypeKind::Array { .. }) => *s,
                _ => return None,
            };
            resolved = resolved || self.is_resolved(st);
        }
        Some((st, resolved))
    }

    /// The formals of an instantiated unit: `(generics, ports)`.
    pub fn inst_formals(&self, t: &InstTarget) -> Option<(&[DeclId], &[DeclId])> {
        let entity = |u: crate::ids::UnitId| match &self.units[u].hir {
            UnitHir::Entity {
                generics, ports, ..
            } => Some((&generics[..], &ports[..])),
            _ => None,
        };
        match t {
            InstTarget::Component(c) => match &self.decls[*c].kind {
                DeclKind::Component(c) => Some((&c.generics[..], &c.ports[..])),
                _ => None,
            },
            InstTarget::Entity { unit, .. } => entity(*unit),
            InstTarget::Configuration(cfg) => match &self.units[*cfg].hir {
                UnitHir::Configuration(b) => match b.target {
                    crate::hir::BlockTarget::Architecture(a) => match &self.units[a].hir {
                        UnitHir::Architecture { entity: e, .. } => entity(*e),
                        _ => None,
                    },
                    _ => None,
                },
                _ => None,
            },
        }
    }

    /// Every source of a signal in a concurrent statement region: drivers of
    /// processes and concurrent statements, and actuals of `out`, `inout` and
    /// `buffer` ports. Generate statements are not entered: whether (and how
    /// often) their bodies exist is decided by elaboration, and the LRM rule
    /// is about the elaborated design, so elab checks those sources.
    fn sources(&self, body: &[StmtId], out: &mut Vec<Source>) {
        for &s in body {
            let add = |l: &Lsp, out: &mut Vec<Source>| {
                out.push(Source {
                    lsp: l.clone(),
                    stmt: s,
                })
            };
            match &self.stmts[s].kind {
                StmtKind::Process { info, .. }
                | StmtKind::ConcSignalAssign { info, .. }
                | StmtKind::ConcProcCall { info, .. } => {
                    for l in &info.drivers {
                        add(l, out);
                    }
                }
                StmtKind::Instance {
                    target, port_map, ..
                } => {
                    let Some((_, ports)) = self.inst_formals(target) else {
                        continue;
                    };
                    for a in port_map.iter() {
                        let Some(&p) = ports.get(usize::from(a.formal)) else {
                            continue;
                        };
                        let DeclKind::Object(o) = &self.decls[p].kind else {
                            continue;
                        };
                        if !matches!(o.mode, Some(Mode::Out | Mode::Inout | Mode::Buffer)) {
                            continue;
                        }
                        let mut es = Vec::new();
                        self.walk_actual_top(&a.actual, Use::Target, &mut |e, _| es.push(e));
                        for e in es {
                            if let Some(l) = self.signal_lsp(e) {
                                add(&l, out);
                            }
                        }
                    }
                }
                StmtKind::Block { body, .. } => self.sources(body, out),
                _ => {}
            }
        }
    }

    /// LRM 14.7.2 / 6.4.2.3: an unresolved signal (or unresolved sub-element) has
    /// at most one source. Checked over the concurrent statements of one
    /// architecture or entity, with both sources in the error (`E1812`).
    /// Sources whose overlap depends on globally static values are left to elab.
    /// Returns whether it reported.
    pub(crate) fn check_multiple_sources(&mut self, body: &[StmtId]) -> bool {
        let mut srcs = Vec::new();
        self.sources(body, &mut srcs);
        let mut by_root: HashMap<DeclId, Vec<usize>> = HashMap::new();
        for (i, s) in srcs.iter().enumerate() {
            if let ObjRoot::Decl(d) = s.lsp.root {
                by_root.entry(d).or_default().push(i);
            }
        }
        let mut roots: Vec<DeclId> = by_root.keys().copied().collect();
        roots.sort();
        let mut found = Vec::new();
        for d in roots {
            let idx = &by_root[&d];
            // ponytail (#42): quadratic in the sources of one signal (measured, debug
            // build: 1000 single-bit drivers 0.2 s, 4000 drivers 2.6 s); group by
            // first static step if a design has thousands of drivers of one signal.
            'pairs: for (n, &i) in idx.iter().enumerate() {
                for &j in &idx[..n] {
                    let (a, b) = (&srcs[j], &srcs[i]);
                    if a.stmt == b.stmt {
                        continue;
                    }
                    if self.overlap(&a.lsp, &b.lsp) != Overlap::Yes {
                        continue;
                    }
                    let deeper = if a.lsp.path.len() >= b.lsp.path.len() {
                        &a.lsp
                    } else {
                        &b.lsp
                    };
                    let Some((st, false)) = self.lsp_subtype(deeper) else {
                        continue;
                    };
                    found.push((d, a.lsp.span, b.lsp.span, st));
                    break 'pairs;
                }
            }
        }
        let reported = !found.is_empty();
        for (d, first, second, st) in found {
            let ty = &self.types[self.subtypes[st].base];
            let d = Diagnostic::error(
                diag::MULTIPLE_SOURCES,
                format!(
                    "unresolved signal `{}` has multiple sources",
                    self.decls[d].name
                ),
            )
            .with_primary(second, "a second source here")
            .with_secondary(first, "first source here")
            .with_note(format!(
                "the driven part has the unresolved type `{}`, so only one process may drive it",
                ty.name.text
            ))
            .with_help("drive it from one process, or use a resolved subtype such as `std_logic`");
            self.report(d);
        }
        reported
    }
}

/// Visits the expressions of one right-hand side piece.
type ExprEach<'a, T> = dyn FnMut(&T, &mut dyn FnMut(ExprId, Use)) + 'a;

struct Source {
    lsp: Lsp,
    stmt: StmtId,
}

fn push_unique(db: &Db, out: &mut Vec<Lsp>, l: Lsp) {
    if !out.iter().any(|o| db.same_lsp(o, &l)) {
        out.push(l);
    }
}
