//! Statements and expressions.

use crate::Diag;
use crate::ast::*;
use crate::lex::{K, Tok};
use crate::parse::{Parser, R};

fn binop(s: &str) -> Option<(&'static str, u8)> {
    Some(match s {
        "||" => ("||", 1),
        "&&" => ("&&", 2),
        "|" => ("|", 3),
        "^" => ("^", 4),
        "~^" => ("~^", 4),
        "^~" => ("^~", 4),
        "&" => ("&", 5),
        "==" => ("==", 6),
        "!=" => ("!=", 6),
        "===" => ("===", 6),
        "!==" => ("!==", 6),
        "<" => ("<", 7),
        "<=" => ("<=", 7),
        ">" => (">", 7),
        ">=" => (">=", 7),
        "<<" => ("<<", 8),
        ">>" => (">>", 8),
        "<<<" => ("<<<", 8),
        ">>>" => (">>>", 8),
        "+" => ("+", 9),
        "-" => ("-", 9),
        "*" => ("*", 10),
        "/" => ("/", 10),
        "%" => ("%", 10),
        "**" => ("**", 11),
        _ => return None,
    })
}

const UNARY: &[&str] = &["+", "-", "!", "~", "&", "|", "^", "~&", "~|", "~^", "^~"];

impl Parser<'_> {
    // ----- expressions -----

    pub(crate) fn expr(&mut self) -> R<Expr> {
        self.enter()?;
        let r = self.cond_expr();
        self.leave();
        r
    }

    fn cond_expr(&mut self) -> R<Expr> {
        let c = self.binary(1)?;
        if self.eat_p("?") {
            self.attrs()?;
            let a = self.expr()?;
            self.expect_p(":")?;
            let b = self.expr()?;
            return Ok(Expr::Cond(Box::new(c), Box::new(a), Box::new(b)));
        }
        Ok(c)
    }

    /// `expr` or `min:typ:max`.
    pub(crate) fn mintypmax(&mut self) -> R<Expr> {
        let a = self.expr()?;
        if self.p(":") {
            self.bump();
            let b = self.expr()?;
            self.expect_p(":")?;
            let c = self.expr()?;
            return Ok(Expr::MinTypMax(Box::new(a), Box::new(b), Box::new(c)));
        }
        Ok(a)
    }

    fn binary(&mut self, min: u8) -> R<Expr> {
        let mut l = self.unary()?;
        loop {
            let t = self.peek();
            if t.k != K::P {
                return Ok(l);
            }
            // `a++b` and `a--b` are `a + (+b)` and `a - (-b)` in 1364; `i++` is SystemVerilog.
            if matches!(t.s, "++" | "--") {
                let next = self.peek_at(1);
                let operand = matches!(next.k, K::Ident | K::Num | K::Real | K::Sys | K::Str)
                    || (next.k == K::P && (matches!(next.s, "(" | "{") || UNARY.contains(&next.s)));
                if !operand {
                    self.unexpected("an expression");
                    return Err(());
                }
                if 9 < min {
                    return Ok(l);
                }
                self.bump();
                let sign = &t.s[..1];
                let sign: &'static str = if sign == "+" { "+" } else { "-" };
                let r = self.binary(10)?;
                l = Expr::Binary(sign, Box::new(l), Box::new(Expr::Unary(sign, Box::new(r))));
                continue;
            }
            let Some((op, prec)) = binop(t.s) else {
                return Ok(l);
            };
            if prec < min {
                return Ok(l);
            }
            self.bump();
            self.attrs()?;
            let r = self.binary(prec + 1)?;
            l = Expr::Binary(op, Box::new(l), Box::new(r));
        }
    }

    fn unary(&mut self) -> R<Expr> {
        self.attrs()?;
        let t = self.peek();
        if t.k == K::P && UNARY.contains(&t.s) {
            self.bump();
            self.attrs()?;
            self.enter()?;
            let e = self.unary();
            self.leave();
            let op = UNARY.iter().find(|u| **u == t.s).unwrap();
            return Ok(Expr::Unary(op, Box::new(e?)));
        }
        self.primary()
    }

    fn primary(&mut self) -> R<Expr> {
        let t = self.peek();
        match t.k {
            K::Num => {
                self.bump();
                Ok(Expr::Num(self.text(t)))
            }
            K::Real => {
                self.bump();
                Ok(Expr::Real(self.text(t)))
            }
            K::Str => {
                self.bump();
                let s = self.text(t);
                Ok(Expr::Str(
                    s.get(1..s.len().saturating_sub(1).max(1))
                        .unwrap_or("")
                        .to_string(),
                ))
            }
            K::Sys => {
                self.bump();
                let name = self.text(t);
                let mut args = Vec::new();
                if self.eat_p("(") {
                    args = self.args()?;
                    self.expect_p(")")?;
                }
                Ok(Expr::SysCall(name, args))
            }
            K::Ident => {
                let id = self.ident()?;
                self.postfix(Expr::Ident(id), true)
            }
            K::P if t.s == "(" => {
                self.bump();
                let e = self.mintypmax()?;
                self.expect_p(")")?;
                Ok(e)
            }
            K::P if t.s == "{" => {
                self.bump();
                let at = self.peek();
                let first = self.expr()?;
                if self.eat_p("{") {
                    if matches!(&first, Expr::Num(n) if has_xz(n)) {
                        self.push(Diag::error(
                            "E4100",
                            "a replication constant is not x or z (clause 5.1.14)",
                            self.span(at),
                        ));
                        return Err(());
                    }
                    let mut v = Vec::new();
                    loop {
                        let at = self.peek();
                        v.push(self.expr()?);
                        self.sized(&v[v.len() - 1], at)?;
                        if !self.eat_p(",") {
                            break;
                        }
                    }
                    self.expect_p("}")?;
                    self.expect_p("}")?;
                    return Ok(Expr::Repl(Box::new(first), v));
                }
                self.sized(&first, at)?;
                let mut v = vec![first];
                while self.eat_p(",") {
                    let at = self.peek();
                    v.push(self.expr()?);
                    self.sized(&v[v.len() - 1], at)?;
                }
                self.expect_p("}")?;
                Ok(Expr::Concat(v))
            }
            _ => {
                self.unexpected("an expression");
                Err(())
            }
        }
    }

    /// Clause 5.1.14: an unsized constant has no place in a concatenation.
    fn sized(&mut self, e: &Expr, at: Tok) -> R<()> {
        let n = match e {
            Expr::Num(n) => n,
            Expr::Unary("-" | "+", x) => match &**x {
                Expr::Num(n) => n,
                _ => return Ok(()),
            },
            _ => return Ok(()),
        };
        if n.trim_start().starts_with('\'') || !n.contains('\'') {
            self.push(Diag::error(
                "E4100",
                "an unsized constant has no place in a concatenation (clause 5.1.14)",
                self.span(at),
            ));
            return Err(());
        }
        Ok(())
    }

    /// Call arguments up to (not including) `)`: an empty slot is `Expr::Empty`.
    pub(crate) fn args(&mut self) -> R<Vec<Expr>> {
        let mut v = Vec::new();
        if self.p(")") {
            return Ok(v);
        }
        loop {
            if self.p(",") || self.p(")") {
                v.push(Expr::Empty);
            } else {
                v.push(self.expr()?);
            }
            if !self.eat_p(",") {
                break;
            }
            if self.p(")") {
                v.push(Expr::Empty);
                break;
            }
        }
        Ok(v)
    }

    /// Hierarchical names, selects and (for expressions) function calls after a name.
    pub(crate) fn postfix(&mut self, mut e: Expr, call: bool) -> R<Expr> {
        loop {
            if self.p(".") {
                self.bump();
                e = Expr::Dot(Box::new(e), self.ident()?);
            } else if self.p("[") {
                e = self.selects(e)?;
            } else if call
                && (self.p("(") || (self.p("(*") && self.p_at(self.attrs_len(), "(")))
                && matches!(e, Expr::Ident(_) | Expr::Dot(..))
            {
                // Clause 3.8.1: an attribute instance may stand between a function name and its arguments.
                self.attrs()?;
                self.bump();
                let a = self.args()?;
                self.expect_p(")")?;
                e = Expr::Call(Box::new(e), a);
            } else {
                return Ok(e);
            }
        }
    }

    /// One or more `[...]` after an expression.
    pub(crate) fn selects(&mut self, mut e: Expr) -> R<Expr> {
        while self.p("[") {
            self.bump();
            let a = self.expr()?;
            let t = self.peek();
            e = if t.k == K::P && matches!(t.s, ":" | "+:" | "-:") {
                self.bump();
                let kind = match t.s {
                    ":" => RangeKind::Colon,
                    "+:" => RangeKind::PlusColon,
                    _ => RangeKind::MinusColon,
                };
                let b = self.expr()?;
                Expr::Range(Box::new(e), kind, Box::new(a), Box::new(b))
            } else {
                Expr::Index(Box::new(e), Box::new(a))
            };
            self.expect_p("]")?;
        }
        Ok(e)
    }

    /// An assignment target or a task name: a name with selects, or a concatenation.
    pub(crate) fn lvalue(&mut self) -> R<Expr> {
        if self.p("{") {
            let at = self.peek();
            let e = self.primary()?;
            if has_repl(&e) {
                self.push(Diag::error(
                    "E4100",
                    "a replication is not a target of an assignment (clause 5.1.14)",
                    self.span(at),
                ));
                return Err(());
            }
            return Ok(e);
        }
        let id = self.ident()?;
        self.postfix(Expr::Ident(id), false)
    }

    // ----- statements -----

    pub(crate) fn stmt(&mut self) -> R<Stmt> {
        self.enter()?;
        let r = self.stmt_inner();
        self.leave();
        r
    }

    /// Clause 10.4.4: what a function body does not contain.
    fn not_in_func(&mut self, at: Tok, what: &str) -> R<()> {
        if self.in_func {
            self.push(Diag::error(
                "E4100",
                format!("a function has no {what} (clause 10.4.4)"),
                self.span(at),
            ));
            return Err(());
        }
        Ok(())
    }

    fn stmt_inner(&mut self) -> R<Stmt> {
        self.attrs()?;
        let t = self.peek();
        match (t.k, t.s) {
            (K::Kw, "wait") | (K::P, "#" | "@") => self.not_in_func(t, "timing control")?,
            (K::P, "->") => self.not_in_func(t, "event trigger")?,
            (K::Kw, "assign" | "force" | "deassign" | "release") => {
                self.not_in_func(t, "procedural continuous assignment")?
            }
            _ => {}
        }
        match (t.k, t.s) {
            (K::P, ";") => {
                self.bump();
                Ok(Stmt::Null)
            }
            (K::Kw, "begin" | "fork") => self.block(),
            (K::Kw, "if") => {
                self.bump();
                self.expect_p("(")?;
                let cond = self.expr()?;
                self.expect_p(")")?;
                let then = Box::new(self.stmt()?);
                let els = if self.eat_kw("else") {
                    Some(Box::new(self.stmt()?))
                } else {
                    None
                };
                Ok(Stmt::If { cond, then, els })
            }
            (K::Kw, "case" | "casex" | "casez") => {
                self.bump();
                self.expect_p("(")?;
                let expr = self.expr()?;
                self.expect_p(")")?;
                let mut items = Vec::new();
                let mut defaults = 0;
                while !self.kw("endcase") {
                    if self.peek().k == K::Eof {
                        return self.fail("`endcase`");
                    }
                    self.attrs()?;
                    let mut exprs = Vec::new();
                    if self.kw("default") {
                        defaults += 1;
                        self.second_default(defaults)?;
                        self.bump();
                        self.eat_p(":");
                    } else {
                        loop {
                            exprs.push(self.expr()?);
                            if !self.eat_p(",") {
                                break;
                            }
                        }
                        self.expect_p(":")?;
                    }
                    items.push(CaseItem {
                        exprs,
                        stmt: self.stmt()?,
                    });
                }
                if items.is_empty() {
                    return self.fail("a case item");
                }
                self.bump();
                Ok(Stmt::Case {
                    kind: t.s,
                    expr,
                    items,
                })
            }
            (K::Kw, "for") => {
                self.bump();
                self.expect_p("(")?;
                let l1 = self.lvalue()?;
                self.expect_p("=")?;
                let r1 = self.expr()?;
                self.expect_p(";")?;
                let cond = self.expr()?;
                self.expect_p(";")?;
                let l2 = self.lvalue()?;
                self.expect_p("=")?;
                let r2 = self.expr()?;
                self.expect_p(")")?;
                let body = Box::new(self.stmt()?);
                Ok(Stmt::For {
                    init: (l1, r1),
                    cond,
                    step: (l2, r2),
                    body,
                })
            }
            (K::Kw, "while") => {
                self.bump();
                self.expect_p("(")?;
                let cond = self.expr()?;
                self.expect_p(")")?;
                Ok(Stmt::While {
                    cond,
                    body: Box::new(self.stmt()?),
                })
            }
            (K::Kw, "repeat") => {
                self.bump();
                self.expect_p("(")?;
                let count = self.expr()?;
                self.expect_p(")")?;
                Ok(Stmt::Repeat {
                    count,
                    body: Box::new(self.stmt()?),
                })
            }
            (K::Kw, "forever") => {
                self.bump();
                Ok(Stmt::Forever(Box::new(self.stmt()?)))
            }
            (K::Kw, "wait") => {
                self.bump();
                self.expect_p("(")?;
                let cond = self.expr()?;
                self.expect_p(")")?;
                Ok(Stmt::Wait {
                    cond,
                    body: Box::new(self.stmt()?),
                })
            }
            (K::P, "#") => {
                let delay = self.delay()?.expect("`#` was checked");
                Ok(Stmt::Delay {
                    delay,
                    body: Box::new(self.stmt()?),
                })
            }
            (K::P, "@") => {
                let ctrl = self.event_ctrl()?;
                Ok(Stmt::Event {
                    ctrl,
                    body: Box::new(self.stmt()?),
                })
            }
            (K::P, "->") => {
                self.bump();
                let e = self.lvalue()?;
                self.expect_p(";")?;
                Ok(Stmt::Trigger(e))
            }
            (K::Kw, "disable") => {
                self.bump();
                let e = self.lvalue()?;
                self.expect_p(";")?;
                Ok(Stmt::Disable(e))
            }
            (K::Kw, "assign" | "force") => {
                self.bump();
                let lhs = self.lvalue()?;
                if t.s == "assign" && !plain_target(&lhs) {
                    return self.proc_target(t);
                }
                self.expect_p("=")?;
                let rhs = Some(self.expr()?);
                self.expect_p(";")?;
                Ok(Stmt::ProcAssign {
                    kind: t.s,
                    lhs,
                    rhs,
                })
            }
            (K::Kw, "deassign" | "release") => {
                self.bump();
                let lhs = self.lvalue()?;
                if t.s == "deassign" && !plain_target(&lhs) {
                    return self.proc_target(t);
                }
                self.expect_p(";")?;
                Ok(Stmt::ProcAssign {
                    kind: t.s,
                    lhs,
                    rhs: None,
                })
            }
            (K::Sys, _) => {
                self.bump();
                let name = self.text(t);
                let mut args = Vec::new();
                if self.eat_p("(") {
                    args = self.args()?;
                    self.expect_p(")")?;
                }
                self.expect_p(";")?;
                Ok(Stmt::SysCall { name, args })
            }
            (K::Ident, _) | (K::P, "{") => {
                if self.sv_word(t)
                    && !self.p_at(1, "=")
                    && !self.p_at(1, "<=")
                    && !self.p_at(1, "[")
                {
                    self.unexpected("a statement");
                    return Err(());
                }
                let lhs = self.lvalue()?;
                let n = self.peek();
                if n.k == K::P && (n.s == "=" || n.s == "<=") {
                    self.bump();
                    if n.s == "<=" {
                        self.not_in_func(n, "nonblocking assignment")?;
                    }
                    let ctrl_at = self.peek();
                    let ctrl = self.intra_ctrl()?;
                    if ctrl.is_some() {
                        self.not_in_func(ctrl_at, "timing control")?;
                    }
                    let rhs = self.expr()?;
                    self.expect_p(";")?;
                    return Ok(Stmt::Assign {
                        lhs,
                        rhs,
                        nonblocking: n.s == "<=",
                        ctrl,
                    });
                }
                if matches!(lhs, Expr::Ident(_) | Expr::Dot(..)) {
                    let mut args = Vec::new();
                    let open = self.peek();
                    if self.eat_p("(") {
                        args = self.args()?;
                        self.expect_p(")")?;
                        // Clause 10.2.2: no empty list and no null argument in a task enable.
                        if args.is_empty() || args.iter().any(|a| matches!(a, Expr::Empty)) {
                            self.push(Diag::error(
                                "E4100",
                                "a task enable gives an expression for every argument and has no parentheses without arguments (clause 10.2.2)",
                                self.span(open),
                            ));
                            return Err(());
                        }
                    }
                    if self.p(";") {
                        self.not_in_func(t, "task enable")?;
                        self.bump();
                        return Ok(Stmt::TaskCall { name: lhs, args });
                    }
                }
                self.fail("`=`, `<=` or `;`")
            }
            _ => {
                self.unexpected("a statement");
                Err(())
            }
        }
    }

    fn proc_target<T>(&mut self, at: Tok) -> R<T> {
        self.push(Diag::error(
            "E4100",
            format!(
                "the target of `{}` is a variable or a concatenation of variables, not a bit-select, part-select or array element (clause 9.3)",
                at.s
            ),
            self.span(at),
        ));
        Err(())
    }

    fn intra_ctrl(&mut self) -> R<Option<Ctrl>> {
        if self.p("#") {
            return Ok(self.delay()?.map(Ctrl::Delay));
        }
        if self.p("@") {
            return Ok(Some(Ctrl::Event(self.event_ctrl()?)));
        }
        if self.kw("repeat") {
            self.bump();
            self.expect_p("(")?;
            let n = self.expr()?;
            self.expect_p(")")?;
            let ev = self.event_ctrl()?;
            return Ok(Some(Ctrl::Repeat(n, ev)));
        }
        Ok(None)
    }

    fn event_ctrl(&mut self) -> R<EventCtrl> {
        self.expect_p("@")?;
        if self.p("*") {
            self.bump();
            return Ok(EventCtrl::Star);
        }
        if self.p("(*") && self.p_at(1, ")") {
            self.bump();
            self.bump();
            return Ok(EventCtrl::Star);
        }
        if self.eat_p("(") {
            if self.p("*") && self.p_at(1, ")") {
                self.bump();
                self.bump();
                return Ok(EventCtrl::Star);
            }
            let mut v = Vec::new();
            loop {
                let edge = if self.kw("posedge") || self.kw("negedge") {
                    Some(self.bump().s)
                } else {
                    None
                };
                v.push(EventExpr {
                    edge,
                    expr: self.expr()?,
                });
                if !(self.eat_kw("or") || self.eat_p(",")) {
                    break;
                }
            }
            self.expect_p(")")?;
            return Ok(EventCtrl::List(v));
        }
        let id = self.ident()?;
        let e = self.postfix(Expr::Ident(id), false)?;
        Ok(EventCtrl::List(vec![EventExpr {
            edge: None,
            expr: e,
        }]))
    }

    fn block(&mut self) -> R<Stmt> {
        let open = self.bump();
        let par = open.s == "fork";
        let mut label = None;
        if self.eat_p(":") {
            label = Some(self.ident()?);
        }
        let mut decls = Vec::new();
        let decl_at = self.peek_at(self.attrs_len());
        while self.block_decl(&mut decls, false)? {}
        // Clauses 9.8.1 and 9.8.2: only a named block has declarations.
        if label.is_none() && !decls.is_empty() {
            self.push(
                Diag::error(
                    "E4100",
                    format!(
                        "a `{}` block with declarations has a name (clause 9.8.1)",
                        open.s
                    ),
                    self.span(decl_at),
                )
                .help(format!("write `{} : name`", open.s)),
            );
            return Err(());
        }
        let mut stmts = Vec::new();
        loop {
            let t = self.peek();
            if t.k == K::Kw && ((par && t.s == "join") || (!par && t.s == "end")) {
                self.bump();
                break;
            }
            if t.k == K::Eof {
                self.push(
                    Diag::error(
                        "E4100",
                        format!("`{}` block is not closed", open.s),
                        self.span(open),
                    )
                    .label("opened here"),
                );
                return Err(());
            }
            if self.sv_word(t) && matches!(self.text(t).as_str(), "join_any" | "join_none") {
                self.unexpected("`join`");
                return Err(());
            }
            stmts.push(self.stmt()?);
        }
        if self.p(":") {
            let t = self.peek();
            self.push(
                Diag::error(
                    "E4003",
                    "block end label (`: name`) is SystemVerilog",
                    self.span(t),
                )
                .help("IEEE 1364-2005 has no label after `end` and `join`; remove it"),
            );
            return Err(());
        }
        Ok(Stmt::Block {
            par,
            label,
            decls,
            stmts,
        })
    }
}

/// A constant has an x, z or ? digit (after its base).
fn has_xz(n: &str) -> bool {
    n.split_once('\'')
        .is_some_and(|(_, d)| d.chars().any(|c| matches!(c, 'x' | 'X' | 'z' | 'Z' | '?')))
}

fn has_repl(e: &Expr) -> bool {
    match e {
        Expr::Repl(..) => true,
        Expr::Concat(v) => v.iter().any(has_repl),
        _ => false,
    }
}

/// A name, or a concatenation of names.
fn plain_target(e: &Expr) -> bool {
    match e {
        Expr::Ident(_) | Expr::Dot(..) => true,
        Expr::Concat(v) => v.iter().all(plain_target),
        _ => false,
    }
}
