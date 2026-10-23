//! Sequential and concurrent statements.

use super::expr::expr_into_name;
use super::{PResult, Parser, Recover};
use crate::ast::*;
use crate::token::{Kw, Tok};
use yavs_diag::Span;

/// Tokens that start PSL directives in a statement part.
fn is_psl_start(t: Tok) -> bool {
    matches!(
        t,
        Tok::Kw(
            Kw::Assume
                | Kw::AssumeGuarantee
                | Kw::Cover
                | Kw::Restrict
                | Kw::RestrictGuarantee
                | Kw::Fairness
                | Kw::Property
                | Kw::Sequence
                | Kw::Default
                | Kw::Strong
        )
    )
}

impl Parser<'_> {
    // Sequential statements.

    /// Statements up to `end`, `else`, `elsif`, `when` or end of file.
    pub(super) fn seq_stmts(&mut self) -> Vec<SeqStmt> {
        let mark = self.sc.stmts.len();
        while !matches!(
            self.peek(),
            Tok::Kw(Kw::End | Kw::Else | Kw::Elsif | Kw::When) | Tok::Eof
        ) {
            let before = self.pos;
            match self.seq_stmt() {
                Ok(s) => self.sc.stmts.push(s),
                Err(Recover) => {
                    self.sync();
                    if self.pos == before {
                        self.bump();
                    }
                }
            }
        }
        self.sc.stmts.drain(mark..).collect()
    }

    fn seq_stmt(&mut self) -> PResult<SeqStmt> {
        self.nested(|p| {
            let start = p.span();
            let label = p.label();
            let kind = p.seq_stmt_kind(label.as_ref())?;
            Ok(SeqStmt {
                label,
                kind,
                span: p.span_from(start),
            })
        })
    }

    fn seq_stmt_kind(&mut self, label: Option<&Ident>) -> PResult<SeqStmtKind> {
        Ok(match self.peek() {
            Tok::Kw(Kw::Wait) => {
                self.bump();
                let on = if self.eat_kw(Kw::On) {
                    let mut v = vec![self.name()?];
                    while self.eat(Tok::Comma) {
                        v.push(self.name()?);
                    }
                    Some(v)
                } else {
                    None
                };
                let until = if self.eat_kw(Kw::Until) {
                    Some(self.expr()?)
                } else {
                    None
                };
                let timeout = if self.eat_kw(Kw::For) {
                    Some(self.expr()?)
                } else {
                    None
                };
                self.semi()?;
                SeqStmtKind::Wait { on, until, timeout }
            }
            Tok::Kw(Kw::Assert) => {
                let a = self.assertion()?;
                self.semi()?;
                SeqStmtKind::Assert(a)
            }
            Tok::Kw(Kw::Report) => {
                self.bump();
                let report = self.expr()?;
                let severity = if self.eat_kw(Kw::Severity) {
                    Some(self.expr()?)
                } else {
                    None
                };
                self.semi()?;
                SeqStmtKind::Report { report, severity }
            }
            Tok::Kw(Kw::If) => self.if_stmt(label)?,
            Tok::Kw(Kw::Case) => {
                let open = self.bump_span();
                let matching = self.eat(Tok::Question);
                let expr = self.header_expr(&[Tok::Kw(Kw::Is)]);
                self.expect_kw(Kw::Is)?;
                let mut alts = Vec::new();
                while self.eat_kw(Kw::When) {
                    let choices = self.choices()?;
                    self.expect(Tok::Arrow)?;
                    alts.push((choices, self.seq_stmts()));
                }
                let end_label = self.end_case(open, matching, label)?;
                SeqStmtKind::Case {
                    matching,
                    expr: expr?,
                    alts,
                    end_label,
                }
            }
            Tok::Kw(Kw::While | Kw::For | Kw::Loop) => {
                let open = self.span();
                let scheme = if self.eat_kw(Kw::While) {
                    Some(LoopScheme::While(self.header_expr(&[Tok::Kw(Kw::Loop)])?))
                } else if self.eat_kw(Kw::For) {
                    let id = self.ident()?;
                    self.expect_kw(Kw::In)?;
                    let r = self.discrete_range();
                    if r.is_err() {
                        self.sync_to(&[Tok::Kw(Kw::Loop)]);
                    }
                    Some(LoopScheme::For(id, r?))
                } else {
                    None
                };
                self.expect_kw(Kw::Loop)?;
                let stmts = self.seq_stmts();
                let end_label = self.end(open, "loop", &[Kw::Loop], false, label)?;
                SeqStmtKind::Loop {
                    scheme,
                    stmts,
                    end_label,
                }
            }
            Tok::Kw(k @ (Kw::Next | Kw::Exit)) => {
                self.bump();
                let label = if self.at_ident() {
                    Some(self.ident()?)
                } else {
                    None
                };
                let when = if self.eat_kw(Kw::When) {
                    Some(self.expr()?)
                } else {
                    None
                };
                self.semi()?;
                if k == Kw::Next {
                    SeqStmtKind::Next { label, when }
                } else {
                    SeqStmtKind::Exit { label, when }
                }
            }
            Tok::Kw(Kw::Return) => {
                self.bump();
                if self.at_kw(Kw::When) {
                    let when = self.span();
                    self.bump();
                    self.require_2019(when, "a conditional return");
                    let c = self.expr()?;
                    self.semi()?;
                    SeqStmtKind::ReturnWhen(c)
                } else {
                    let e = if self.at(Tok::Semi) {
                        None
                    } else {
                        Some(self.return_value()?)
                    };
                    self.semi()?;
                    SeqStmtKind::Return(e)
                }
            }
            Tok::Kw(Kw::Null) => {
                self.bump();
                self.semi()?;
                SeqStmtKind::Null
            }
            Tok::Kw(Kw::Block) => {
                let open = self.bump_span();
                self.require_2019(open, "a sequential block statement");
                self.eat_kw(Kw::Is);
                let decls = self.decls(false);
                self.expect_kw(Kw::Begin)?;
                let stmts = self.seq_stmts();
                let end_label = self.end(open, "block", &[Kw::Block], false, label)?;
                SeqStmtKind::Block {
                    decls,
                    stmts,
                    end_label,
                }
            }
            Tok::Kw(Kw::With) => {
                let at = self.span();
                self.require_2008(at, "a selected assignment in a process");
                let (target, expr, matching) = self.with_header()?;
                if self.eat(Tok::Assign) {
                    let alts = self.selected_alts(|p| p.expr())?;
                    SeqStmtKind::VariableAssign {
                        target,
                        rhs: Rhs::Selected {
                            expr,
                            matching,
                            alts,
                        },
                    }
                } else {
                    self.expect(Tok::Le)?;
                    SeqStmtKind::SignalAssign(
                        self.selected_signal_rhs(target, expr, matching, false)?,
                    )
                }
            }
            _ => {
                let target = self.target()?;
                match self.peek() {
                    Tok::Le => {
                        self.bump();
                        SeqStmtKind::SignalAssign(self.signal_assign_rhs(target, false)?)
                    }
                    Tok::Assign => {
                        self.bump();
                        let rhs = self.conditional(|p| p.var_value())?;
                        if !matches!(rhs, Rhs::Simple(_)) {
                            let at = self.span();
                            self.require_2008(at, "a conditional variable assignment");
                        }
                        self.semi()?;
                        SeqStmtKind::VariableAssign { target, rhs }
                    }
                    _ => {
                        let mut name = expr_into_name(self, target, "a procedure name")?;
                        if self.at_kw(Kw::Generic) && self.nth(1) == Tok::Kw(Kw::Map) {
                            // 2019 `p generic map (..) (..);` (LCS-2016-049).
                            name = self.name_suffixes(name, true)?;
                        }
                        self.semi()?;
                        SeqStmtKind::ProcedureCall(name)
                    }
                }
            }
        })
    }

    /// An expression in a statement header; on error skip to one of `stops`
    /// so the body still parses.
    fn header_expr(&mut self, stops: &[Tok]) -> PResult<Expr> {
        let r = self.expr();
        if r.is_err() {
            self.sync_to(stops);
        }
        r
    }

    fn end_case(
        &mut self,
        open: Span,
        matching: bool,
        label: Option<&Ident>,
    ) -> PResult<Option<Ident>> {
        if !matching {
            return self.end(open, "case statement", &[Kw::Case], false, label);
        }
        // `end case ?`: `?` is not a keyword, so check by hand.
        if self.at_kw(Kw::End) && self.nth(1) == Tok::Kw(Kw::Case) && self.nth(2) == Tok::Question {
            self.pos += 3;
            let end_label = if self.at_ident() {
                Some(self.ident()?)
            } else {
                None
            };
            self.check_end_label(end_label.as_ref(), label);
            self.semi()?;
            return Ok(end_label);
        }
        let r = self.end(open, "case statement", &[Kw::Case], false, label)?;
        Ok(r)
    }

    fn if_stmt(&mut self, label: Option<&Ident>) -> PResult<SeqStmtKind> {
        let open = self.expect_kw(Kw::If)?;
        let mut branches = Vec::new();
        let mut else_ = None;
        let mut cond = self.header_expr(&[Tok::Kw(Kw::Then)]);
        loop {
            self.expect_kw(Kw::Then)?;
            let stmts = self.seq_stmts();
            branches.push((cond?, stmts));
            if self.eat_kw(Kw::Elsif) {
                cond = self.header_expr(&[Tok::Kw(Kw::Then)]);
            } else {
                if self.eat_kw(Kw::Else) {
                    else_ = Some(self.seq_stmts());
                }
                break;
            }
        }
        let end_label = self.end(open, "if statement", &[Kw::If], false, label)?;
        Ok(SeqStmtKind::If {
            branches,
            else_,
            end_label,
        })
    }

    fn assertion(&mut self) -> PResult<Assertion> {
        self.expect_kw(Kw::Assert)?;
        let cond = self.expr()?;
        let report = if self.eat_kw(Kw::Report) {
            Some(self.expr()?)
        } else {
            None
        };
        let severity = if self.eat_kw(Kw::Severity) {
            Some(self.expr()?)
        } else {
            None
        };
        Ok(Assertion {
            cond,
            report,
            severity,
        })
    }

    /// An assignment target: a name or an aggregate.
    fn target(&mut self) -> PResult<Expr> {
        if self.at(Tok::LParen) {
            return self.paren_or_aggregate();
        }
        let n = self.name()?;
        let span = n.span;
        Ok(Expr {
            kind: ExprKind::Name(n),
            span,
        })
    }

    /// `with e select [?] target`
    fn with_header(&mut self) -> PResult<(Expr, Expr, bool)> {
        self.expect_kw(Kw::With)?;
        let expr = self.expr()?;
        self.expect_kw(Kw::Select)?;
        let matching = self.eat(Tok::Question);
        let target = self.target()?;
        Ok((target, expr, matching))
    }

    /// `v when c1, v when c2 ;`
    fn selected_alts<T>(
        &mut self,
        mut value: impl FnMut(&mut Self) -> PResult<T>,
    ) -> PResult<Vec<(T, Vec<Choice>)>> {
        let mut alts = Vec::new();
        loop {
            let v = value(self)?;
            self.expect_kw(Kw::When)?;
            alts.push((v, self.choices()?));
            if !self.eat(Tok::Comma) {
                break;
            }
        }
        self.semi()?;
        Ok(alts)
    }

    /// `v [when c else v when c ...]`, without the final `;`.
    fn conditional<T>(
        &mut self,
        mut value: impl FnMut(&mut Self) -> PResult<T>,
    ) -> PResult<Rhs<T>> {
        let first = value(self)?;
        if !self.at_kw(Kw::When) {
            return Ok(Rhs::Simple(first));
        }
        let mut branches = Vec::new();
        let mut v = first;
        loop {
            if self.eat_kw(Kw::When) {
                let c = self.expr()?;
                branches.push((v, Some(c)));
                if !self.eat_kw(Kw::Else) {
                    break;
                }
                v = value(self)?;
            } else {
                branches.push((v, None));
                break;
            }
        }
        Ok(Rhs::Conditional(branches))
    }

    fn force_mode(&mut self) -> Option<ForceMode> {
        if self.eat_kw(Kw::In) {
            Some(ForceMode::In)
        } else if self.eat_kw(Kw::Out) {
            Some(ForceMode::Out)
        } else {
            None
        }
    }

    fn delay(&mut self) -> PResult<Option<DelayMechanism>> {
        if self.eat_kw(Kw::Transport) {
            return Ok(Some(DelayMechanism::Transport));
        }
        let reject = if self.eat_kw(Kw::Reject) {
            Some(self.expr()?)
        } else {
            None
        };
        if reject.is_some() {
            self.expect_kw(Kw::Inertial)?;
            return Ok(Some(DelayMechanism::Inertial { reject }));
        }
        if self.eat_kw(Kw::Inertial) {
            return Ok(Some(DelayMechanism::Inertial { reject: None }));
        }
        Ok(None)
    }

    fn waveform(&mut self) -> PResult<Waveform> {
        if self.at_kw(Kw::Unaffected) {
            return Ok(Waveform::Unaffected(self.bump_span()));
        }
        let mut v = Vec::new();
        loop {
            let value = self.expr()?;
            let after = if self.eat_kw(Kw::After) {
                Some(self.expr()?)
            } else {
                None
            };
            v.push(WaveformElement { value, after });
            if !self.eat(Tok::Comma) {
                break;
            }
        }
        Ok(Waveform::Elements(v))
    }

    /// After `target <=`: waveform, force or release, up to and including `;`.
    fn signal_assign_rhs(&mut self, target: Expr, concurrent: bool) -> PResult<SignalAssign> {
        let guarded = concurrent && self.eat_kw(Kw::Guarded);
        let kind = if !concurrent && self.eat_kw(Kw::Force) {
            let mode = self.force_mode();
            let rhs = self.conditional(|p| p.var_value())?;
            SignalAssignKind::Force { mode, rhs }
        } else if !concurrent && self.eat_kw(Kw::Release) {
            SignalAssignKind::Release {
                mode: self.force_mode(),
            }
        } else {
            let delay = self.delay()?;
            let rhs = self.conditional(|p| p.waveform())?;
            if !concurrent && matches!(rhs, Rhs::Conditional(_)) {
                let at = self.span();
                self.require_2008(at, "a conditional assignment in a process");
            }
            SignalAssignKind::Waveform { delay, rhs }
        };
        self.semi()?;
        Ok(SignalAssign {
            target,
            guarded,
            kind,
        })
    }

    /// After `with e select target <=`.
    fn selected_signal_rhs(
        &mut self,
        target: Expr,
        expr: Expr,
        matching: bool,
        concurrent: bool,
    ) -> PResult<SignalAssign> {
        let guarded = concurrent && self.eat_kw(Kw::Guarded);
        let kind = if !concurrent && self.eat_kw(Kw::Force) {
            let mode = self.force_mode();
            let alts = self.selected_alts(|p| p.expr())?;
            SignalAssignKind::Force {
                mode,
                rhs: Rhs::Selected {
                    expr,
                    matching,
                    alts,
                },
            }
        } else {
            let delay = self.delay()?;
            let alts = self.selected_alts(|p| p.waveform())?;
            SignalAssignKind::Waveform {
                delay,
                rhs: Rhs::Selected {
                    expr,
                    matching,
                    alts,
                },
            }
        };
        Ok(SignalAssign {
            target,
            guarded,
            kind,
        })
    }

    // Concurrent statements.

    /// Statements up to `end`, `elsif`, `else`, `when` or end of file.
    pub(super) fn conc_stmts(&mut self) -> Vec<ConcStmt> {
        let mark = self.sc.conc.len();
        while !matches!(
            self.peek(),
            Tok::Kw(Kw::End | Kw::Elsif | Kw::Else | Kw::When) | Tok::Eof
        ) {
            let before = self.pos;
            match self.conc_stmt() {
                Ok(Some(s)) => self.sc.conc.push(s),
                Ok(None) => {}
                Err(Recover) => {
                    self.sync();
                    if self.pos == before {
                        self.bump();
                    }
                }
            }
        }
        self.sc.conc.drain(mark..).collect()
    }

    /// `Ok(None)` for skipped PSL.
    fn conc_stmt(&mut self) -> PResult<Option<ConcStmt>> {
        self.nested(|p| {
            let start = p.span();
            let label = p.label();
            let postponed = p.eat_kw(Kw::Postponed);
            let Some(kind) = p.conc_stmt_kind(label.as_ref(), postponed)? else {
                return Ok(None);
            };
            Ok(Some(ConcStmt {
                label,
                postponed,
                kind,
                span: p.span_from(start),
            }))
        })
    }

    fn need_label(&mut self, label: Option<&Ident>, what: &str) {
        if label.is_none() {
            let span = self.span();
            self.error(
                span,
                format!("{what} must have a label"),
                "add `label :` before this",
            );
        }
    }

    fn conc_stmt_kind(
        &mut self,
        label: Option<&Ident>,
        postponed: bool,
    ) -> PResult<Option<ConcStmtKind>> {
        Ok(Some(match self.peek() {
            Tok::Kw(Kw::Process) => ConcStmtKind::Process(self.process(label, postponed)?),
            Tok::Kw(Kw::Block) => {
                self.need_label(label, "a block statement");
                ConcStmtKind::Block(self.block(label)?)
            }
            Tok::Kw(Kw::Assert) if self.at_psl_directive() => match self.psl_directive() {
                Some(k) => k,
                None => {
                    self.psl_skip();
                    return Ok(None);
                }
            },
            Tok::Kw(Kw::Assert) => {
                let a = self.assertion()?;
                self.semi()?;
                ConcStmtKind::Assert(a)
            }
            Tok::Kw(Kw::With) => {
                let (target, expr, matching) = self.with_header()?;
                self.expect(Tok::Le)?;
                ConcStmtKind::SignalAssign(self.selected_signal_rhs(target, expr, matching, true)?)
            }
            Tok::Kw(Kw::For) => {
                self.need_label(label, "a generate statement");
                let open = self.bump_span();
                let param = self.ident()?;
                self.expect_kw(Kw::In)?;
                let range = self.discrete_range();
                if range.is_err() {
                    self.sync_to(&[Tok::Kw(Kw::Generate)]);
                }
                self.expect_kw(Kw::Generate)?;
                let body = self.generate_body()?;
                self.end(open, "generate statement", &[Kw::Generate], false, label)?;
                ConcStmtKind::ForGenerate {
                    param,
                    range: range?,
                    body,
                }
            }
            Tok::Kw(Kw::If) => {
                self.need_label(label, "a generate statement");
                let open = self.bump_span();
                let mut branches = Vec::new();
                let mut else_ = None;
                loop {
                    let alt = self.label();
                    let cond = self.header_expr(&[Tok::Kw(Kw::Generate)]);
                    self.expect_kw(Kw::Generate)?;
                    let body = self.generate_body()?;
                    branches.push((alt, cond?, body));
                    if self.at_kw(Kw::Elsif) || self.at_kw(Kw::Else) {
                        let at = self.span();
                        self.require_2008(at, "`elsif` or `else` in an if generate statement");
                    }
                    if self.eat_kw(Kw::Elsif) {
                        continue;
                    }
                    if self.eat_kw(Kw::Else) {
                        let alt = self.label();
                        self.expect_kw(Kw::Generate)?;
                        else_ = Some((alt, self.generate_body()?));
                    }
                    break;
                }
                self.end(open, "generate statement", &[Kw::Generate], false, label)?;
                ConcStmtKind::IfGenerate { branches, else_ }
            }
            Tok::Kw(Kw::Case) => {
                self.need_label(label, "a generate statement");
                let open = self.bump_span();
                self.require_2008(open, "a case generate statement");
                let expr = self.header_expr(&[Tok::Kw(Kw::Generate)]);
                self.expect_kw(Kw::Generate)?;
                let mut alts = Vec::new();
                while self.eat_kw(Kw::When) {
                    let alt = self.label();
                    let choices = self.choices()?;
                    self.expect(Tok::Arrow)?;
                    alts.push((alt, choices, self.generate_body()?));
                }
                self.end(open, "generate statement", &[Kw::Generate], false, label)?;
                ConcStmtKind::CaseGenerate { expr: expr?, alts }
            }
            Tok::Kw(Kw::Entity) => {
                self.bump();
                let name = self.type_mark()?;
                let arch = if self.eat(Tok::LParen) {
                    let a = self.ident()?;
                    self.expect(Tok::RParen)?;
                    Some(a)
                } else {
                    None
                };
                ConcStmtKind::Instance(self.instance_rest(InstantiatedUnit::Entity { name, arch })?)
            }
            Tok::Kw(Kw::Configuration) => {
                self.bump();
                let name = self.type_mark()?;
                ConcStmtKind::Instance(self.instance_rest(InstantiatedUnit::Configuration(name))?)
            }
            Tok::Kw(Kw::Component) => {
                self.bump();
                let name = self.type_mark()?;
                ConcStmtKind::Instance(self.instance_rest(InstantiatedUnit::Component(name))?)
            }
            Tok::Kw(Kw::Assume | Kw::Cover) => match self.psl_directive() {
                Some(k) => k,
                None => {
                    self.psl_skip();
                    return Ok(None);
                }
            },
            Tok::Kw(Kw::Default) if self.psl_default_clock() => return Ok(None),
            t if is_psl_start(t) => {
                if !self.psl_decl() {
                    if self.at_kw(Kw::Property) || self.at_kw(Kw::Sequence) {
                        self.psl_skipped_name();
                    }
                    self.psl_skip();
                }
                return Ok(None);
            }
            _ => {
                let target = self.target()?;
                match self.peek() {
                    Tok::Kw(Kw::Generic) if self.nth(1) == Tok::Kw(Kw::Map) => {
                        let name = expr_into_name(self, target, "a component name")?;
                        let start = self.span();
                        let generic_map = self.map_aspect(Kw::Generic)?;
                        if self.at(Tok::LParen) {
                            // 2019 concurrent `p generic map (..) (..);` (LCS-2016-049);
                            // without parameters it stays a component instantiation.
                            self.require_2019(start, "a generic map in a subprogram call");
                            let span = self.span_from(name.span);
                            let name = Name {
                                kind: NameKind::GenericMap {
                                    prefix: Box::new(name),
                                    args: generic_map,
                                },
                                span,
                            };
                            let name = self.name_suffixes(name, true)?;
                            self.semi()?;
                            ConcStmtKind::ProcedureCall(name)
                        } else {
                            let port_map = self.opt_map(Kw::Port)?;
                            self.semi()?;
                            ConcStmtKind::Instance(InstanceStmt {
                                unit: InstantiatedUnit::Component(name),
                                generic_map: Some(generic_map),
                                port_map,
                            })
                        }
                    }
                    Tok::Kw(Kw::Generic | Kw::Port) => {
                        let name = expr_into_name(self, target, "a component name")?;
                        ConcStmtKind::Instance(
                            self.instance_rest(InstantiatedUnit::Component(name))?,
                        )
                    }
                    Tok::Le => {
                        self.bump();
                        ConcStmtKind::SignalAssign(self.signal_assign_rhs(target, true)?)
                    }
                    _ => {
                        let name = expr_into_name(self, target, "a procedure or component name")?;
                        if !self.at(Tok::Semi) {
                            return Err(self.expected("`<=`, `port map` or `;`"));
                        }
                        self.bump();
                        ConcStmtKind::ProcedureCall(name)
                    }
                }
            }
        }))
    }

    fn instance_rest(&mut self, unit: InstantiatedUnit) -> PResult<InstanceStmt> {
        let generic_map = self.opt_map(Kw::Generic)?;
        let port_map = self.opt_map(Kw::Port)?;
        self.semi()?;
        Ok(InstanceStmt {
            unit,
            generic_map,
            port_map,
        })
    }

    fn process(&mut self, label: Option<&Ident>, postponed: bool) -> PResult<ProcessStmt> {
        let open = self.expect_kw(Kw::Process)?;
        let sensitivity = if self.eat(Tok::LParen) {
            let s = if self.at_kw(Kw::All) {
                let at = self.span();
                self.require_2008(at, "`process (all)`");
                Sensitivity::All(self.bump_span())
            } else {
                let mut v = vec![self.name()?];
                while self.eat(Tok::Comma) {
                    v.push(self.name()?);
                }
                Sensitivity::List(v)
            };
            self.expect(Tok::RParen)?;
            Some(s)
        } else {
            None
        };
        self.eat_kw(Kw::Is);
        let decls = self.decls(false);
        self.expect_kw(Kw::Begin)?;
        let stmts = self.seq_stmts();
        let kws: &[Kw] = if postponed && self.nth(1) == Tok::Kw(Kw::Postponed) {
            &[Kw::Postponed, Kw::Process]
        } else {
            &[Kw::Process]
        };
        let end_label = self.end(open, "process", kws, false, label)?;
        Ok(ProcessStmt {
            sensitivity,
            decls,
            stmts,
            end_label,
        })
    }

    /// A block statement; a `default clock` in it holds only inside.
    fn block(&mut self, label: Option<&Ident>) -> PResult<BlockStmt> {
        let clock = self.psl_clock;
        let b = self.block_inner(label);
        self.psl_clock = clock;
        b
    }

    fn block_inner(&mut self, label: Option<&Ident>) -> PResult<BlockStmt> {
        let open = self.expect_kw(Kw::Block)?;
        let guard = if self.eat(Tok::LParen) {
            let e = self.expr()?;
            self.expect(Tok::RParen)?;
            Some(e)
        } else {
            None
        };
        self.eat_kw(Kw::Is);
        let generics = self.opt_interface_clause(Kw::Generic)?;
        let generic_map = self.opt_map(Kw::Generic)?;
        if generic_map.is_some() {
            self.semi()?;
        }
        let ports = self.opt_interface_clause(Kw::Port)?;
        let port_map = self.opt_map(Kw::Port)?;
        if port_map.is_some() {
            self.semi()?;
        }
        let decls = self.decls(false);
        self.expect_kw(Kw::Begin)?;
        let stmts = self.conc_stmts();
        let end_label = self.end(open, "block", &[Kw::Block], false, label)?;
        Ok(BlockStmt {
            guard,
            generics,
            generic_map,
            ports,
            port_map,
            decls,
            stmts,
            end_label,
        })
    }

    /// A generate body, including an inner `end [alt_label];`.
    fn generate_body(&mut self) -> PResult<GenerateBody> {
        let clock = self.psl_clock;
        let body = self.generate_body_inner();
        self.psl_clock = clock;
        body
    }

    fn generate_body_inner(&mut self) -> PResult<GenerateBody> {
        let start = self.span();
        let decls = self.decls(true);
        let has_begin = self.eat_kw(Kw::Begin);
        if !decls.is_empty() && !has_begin {
            return Err(self.expected("`begin`"));
        }
        let stmts = self.conc_stmts();
        let mut end_label = None;
        if self.at_kw(Kw::End) && self.nth(1) != Tok::Kw(Kw::Generate) {
            self.bump();
            if self.at_ident() {
                end_label = Some(self.ident()?);
            }
            self.semi()?;
        }
        Ok(GenerateBody {
            decls,
            stmts,
            end_label,
            span: self.span_from(start),
        })
    }
}
