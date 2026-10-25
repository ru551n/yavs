//! Names, expressions, ranges, subtype indications and association lists.

use super::{PResult, Parser, Recover};
use crate::ast::*;
use crate::directive::unquote;
use crate::token::{Kw, Tok};
use crate::{Symbol, codes};
use yavs_diag::{Diagnostic, Span};

fn logical_op(t: Tok) -> Option<BinaryOp> {
    Some(match t {
        Tok::Kw(Kw::And) => BinaryOp::And,
        Tok::Kw(Kw::Or) => BinaryOp::Or,
        Tok::Kw(Kw::Nand) => BinaryOp::Nand,
        Tok::Kw(Kw::Nor) => BinaryOp::Nor,
        Tok::Kw(Kw::Xor) => BinaryOp::Xor,
        Tok::Kw(Kw::Xnor) => BinaryOp::Xnor,
        _ => return None,
    })
}

fn relational_op(t: Tok) -> Option<BinaryOp> {
    Some(match t {
        Tok::Eq => BinaryOp::Eq,
        Tok::Ne => BinaryOp::Ne,
        Tok::Lt => BinaryOp::Lt,
        Tok::Le => BinaryOp::Le,
        Tok::Gt => BinaryOp::Gt,
        Tok::Ge => BinaryOp::Ge,
        Tok::MatchEq => BinaryOp::MatchEq,
        Tok::MatchNe => BinaryOp::MatchNe,
        Tok::MatchLt => BinaryOp::MatchLt,
        Tok::MatchLe => BinaryOp::MatchLe,
        Tok::MatchGt => BinaryOp::MatchGt,
        Tok::MatchGe => BinaryOp::MatchGe,
        _ => return None,
    })
}

fn shift_op(t: Tok) -> Option<BinaryOp> {
    Some(match t {
        Tok::Kw(Kw::Sll) => BinaryOp::Sll,
        Tok::Kw(Kw::Srl) => BinaryOp::Srl,
        Tok::Kw(Kw::Sla) => BinaryOp::Sla,
        Tok::Kw(Kw::Sra) => BinaryOp::Sra,
        Tok::Kw(Kw::Rol) => BinaryOp::Rol,
        Tok::Kw(Kw::Ror) => BinaryOp::Ror,
        _ => return None,
    })
}

fn adding_op(t: Tok) -> Option<BinaryOp> {
    Some(match t {
        Tok::Plus => BinaryOp::Add,
        Tok::Minus => BinaryOp::Sub,
        Tok::Amp => BinaryOp::Concat,
        _ => return None,
    })
}

fn multiplying_op(t: Tok) -> Option<BinaryOp> {
    Some(match t {
        Tok::Star => BinaryOp::Mul,
        Tok::Slash => BinaryOp::Div,
        Tok::Kw(Kw::Mod) => BinaryOp::Mod,
        Tok::Kw(Kw::Rem) => BinaryOp::Rem,
        _ => return None,
    })
}

fn binary(op: BinaryOp, l: Expr, r: Expr) -> Expr {
    let span = l.span.to(r.span);
    Expr {
        kind: ExprKind::Binary {
            op,
            left: Box::new(l),
            right: Box::new(r),
        },
        span,
    }
}

/// Turn an expression that must be a name into one, or report.
pub(super) fn expr_into_name(p: &mut Parser, e: Expr, what: &str) -> PResult<Name> {
    match e.kind {
        ExprKind::Name(n) => Ok(n),
        ExprKind::Literal(Literal {
            kind: LiteralKind::Char(c),
            span,
        }) => Ok(Name {
            kind: NameKind::Character(c),
            span,
        }),
        ExprKind::Literal(Literal {
            kind: LiteralKind::String(s),
            span,
        }) => Ok(Name {
            kind: NameKind::Operator(s),
            span,
        }),
        _ => {
            p.error(e.span, format!("expected {what}"), "not a name");
            Err(Recover)
        }
    }
}

impl Parser<'_> {
    /// No `nested` here: every recursion back into `expr` (a parenthesis, an aggregate, a call's
    /// arguments, an operand of `not`) goes through `primary`, which counts it, so one level of
    /// parentheses is one level of depth, not two (#604).
    pub(super) fn expr(&mut self) -> PResult<Expr> {
        if self.at(Tok::Cond) {
            let start = self.bump_span();
            let operand = self.primary()?;
            let span = start.to(operand.span);
            return Ok(Expr {
                kind: ExprKind::Unary {
                    op: UnaryOp::Condition,
                    operand: Box::new(operand),
                },
                span,
            });
        }
        self.logical()
    }

    /// A VHDL-2019 `conditional_expression`: `e { when c else e }`. Used for
    /// initial values, interface defaults and association actuals.
    pub(super) fn cond_expr(&mut self) -> PResult<Expr> {
        let first = self.expr()?;
        self.cond_rest(first, false, Self::expr)
    }

    /// The value of a `return`: in VHDL-2019 a
    /// `conditional_or_unaffected_expression` (values may be `unaffected`, a
    /// final `when c` needs no `else`).
    pub(super) fn return_value(&mut self) -> PResult<Expr> {
        let first = self.var_value()?;
        self.cond_rest(first, true, Self::var_value)
    }

    /// The `when c else e` branches after `first`, if any. `trailing` allows a
    /// last `when c` without `else`.
    fn cond_rest(
        &mut self,
        first: Expr,
        trailing: bool,
        value: fn(&mut Self) -> PResult<Expr>,
    ) -> PResult<Expr> {
        if !self.at_kw(Kw::When) {
            return Ok(first);
        }
        let when = self.span();
        self.require_2019(when, "a conditional expression");
        let start = first.span;
        let mut branches = Vec::new();
        let mut v = first;
        while self.eat_kw(Kw::When) {
            let c = self.expr()?;
            branches.push((v, Some(c)));
            if !self.eat_kw(Kw::Else) {
                if !trailing {
                    return Err(self.expected("`else`"));
                }
                return Ok(Expr {
                    kind: ExprKind::Conditional(branches),
                    span: self.span_from(start),
                });
            }
            v = value(self)?;
        }
        branches.push((v, None));
        Ok(Expr {
            kind: ExprKind::Conditional(branches),
            span: self.span_from(start),
        })
    }

    /// An assignment or `return` value: an expression, or `unaffected` in
    /// VHDL-2019.
    pub(super) fn var_value(&mut self) -> PResult<Expr> {
        if self.at_kw(Kw::Unaffected) {
            let span = self.bump_span();
            self.require_2019(span, "`unaffected` as an assignment or return value");
            return Ok(Expr {
                kind: ExprKind::Unaffected,
                span,
            });
        }
        self.expr()
    }

    fn logical(&mut self) -> PResult<Expr> {
        let mut l = self.relation()?;
        let mut first: Option<BinaryOp> = None;
        while let Some(op) = logical_op(self.peek()) {
            let op_span = self.span();
            if let Some(f) = first
                && (f != op || matches!(op, BinaryOp::Nand | BinaryOp::Nor))
            {
                self.report(
                    Diagnostic::error(codes::SYNTAX, "mixed logical operators need parentheses")
                        .with_primary(op_span, "")
                        .with_help("add parentheses to show the order of evaluation"),
                );
            }
            first = Some(op);
            self.bump();
            let r = self.relation()?;
            l = binary(op, l, r);
        }
        Ok(l)
    }

    fn relation(&mut self) -> PResult<Expr> {
        let l = self.shift()?;
        if let Some(op) = relational_op(self.peek()) {
            self.bump();
            let r = self.shift()?;
            return Ok(binary(op, l, r));
        }
        Ok(l)
    }

    fn shift(&mut self) -> PResult<Expr> {
        let l = self.simple_expr()?;
        if let Some(op) = shift_op(self.peek()) {
            self.bump();
            let r = self.simple_expr()?;
            return Ok(binary(op, l, r));
        }
        Ok(l)
    }

    fn simple_expr(&mut self) -> PResult<Expr> {
        let sign = match self.peek() {
            Tok::Plus => Some(UnaryOp::Plus),
            Tok::Minus => Some(UnaryOp::Minus),
            _ => None,
        };
        let mut l = if let Some(op) = sign {
            let start = self.bump_span();
            let t = self.term()?;
            let span = start.to(t.span);
            Expr {
                kind: ExprKind::Unary {
                    op,
                    operand: Box::new(t),
                },
                span,
            }
        } else {
            self.term()?
        };
        while let Some(op) = adding_op(self.peek()) {
            self.bump();
            let r = self.term()?;
            l = binary(op, l, r);
        }
        Ok(l)
    }

    fn term(&mut self) -> PResult<Expr> {
        let mut l = self.factor()?;
        while let Some(op) = multiplying_op(self.peek()) {
            self.bump();
            let r = self.factor()?;
            l = binary(op, l, r);
        }
        Ok(l)
    }

    fn factor(&mut self) -> PResult<Expr> {
        let op = match self.peek() {
            Tok::Kw(Kw::Abs) => Some(UnaryOp::Abs),
            Tok::Kw(Kw::Not) => Some(UnaryOp::Not),
            Tok::Kw(Kw::And) => Some(UnaryOp::And),
            Tok::Kw(Kw::Or) => Some(UnaryOp::Or),
            Tok::Kw(Kw::Nand) => Some(UnaryOp::Nand),
            Tok::Kw(Kw::Nor) => Some(UnaryOp::Nor),
            Tok::Kw(Kw::Xor) => Some(UnaryOp::Xor),
            Tok::Kw(Kw::Xnor) => Some(UnaryOp::Xnor),
            _ => None,
        };
        if let Some(op) = op {
            let start = self.bump_span();
            let operand = self.primary()?;
            let span = start.to(operand.span);
            return Ok(Expr {
                kind: ExprKind::Unary {
                    op,
                    operand: Box::new(operand),
                },
                span,
            });
        }
        let l = self.primary()?;
        if self.eat(Tok::Pow) {
            let r = self.primary()?;
            return Ok(binary(BinaryOp::Pow, l, r));
        }
        Ok(l)
    }

    fn literal(&mut self, kind: LiteralKind, span: Span) -> Expr {
        Expr {
            kind: ExprKind::Literal(Literal { kind, span }),
            span,
        }
    }

    pub(super) fn primary(&mut self) -> PResult<Expr> {
        self.nested(|p| p.primary_inner())
    }

    fn primary_inner(&mut self) -> PResult<Expr> {
        let start = self.span();
        match self.peek() {
            Tok::Abstract => {
                let t = self.bump();
                let value = Symbol::intern(&self.text(t));
                if self.at_ident() {
                    let u = self.bump();
                    let unit = self.make_ident(u);
                    let span = start.to(unit.span);
                    return Ok(self.literal(LiteralKind::Physical { value, unit }, span));
                }
                Ok(self.literal(LiteralKind::Abstract(value), start))
            }
            Tok::Char if self.nth(1) != Tok::LBracket => {
                self.bump();
                let c = self.char_value(start)?;
                Ok(self.literal(LiteralKind::Char(c), start))
            }
            Tok::BitStr => {
                let t = self.bump();
                let s = Symbol::intern(&self.text(t));
                Ok(self.literal(LiteralKind::BitString(s), start))
            }
            Tok::Str if !matches!(self.nth(1), Tok::LParen | Tok::Dot) => {
                let t = self.bump();
                let s = Symbol::intern(&unquote(&self.text(t)));
                Ok(self.literal(LiteralKind::String(s), start))
            }
            Tok::Kw(Kw::Null) => {
                self.bump();
                Ok(self.literal(LiteralKind::Null, start))
            }
            Tok::Kw(Kw::New) => {
                self.bump();
                let tm = self.type_mark()?;
                let alloc = if self.at(Tok::Tick) && self.nth(1) == Tok::LParen {
                    Allocator::Qualified(self.qualified(tm)?)
                } else {
                    let resolution_or_mark = tm;
                    Allocator::Subtype(self.subtype_rest(None, resolution_or_mark)?)
                };
                Ok(Expr {
                    kind: ExprKind::New(Box::new(alloc)),
                    span: self.span_from(start),
                })
            }
            Tok::LParen => self.paren_or_aggregate(),
            Tok::Ident | Tok::ExtIdent | Tok::Str | Tok::Char | Tok::LtLt => {
                let name = self.name_opts(true)?;
                if self.at(Tok::Tick) && self.nth(1) == Tok::LParen {
                    return self.qualified(name);
                }
                let span = name.span;
                Ok(Expr {
                    kind: ExprKind::Name(name),
                    span,
                })
            }
            _ => Err(self.expected("expression")),
        }
    }

    /// `type_mark ' ( ... )`, positioned at the tick.
    fn qualified(&mut self, type_mark: Name) -> PResult<Expr> {
        self.expect(Tok::Tick)?;
        let operand = self.paren_inner(false)?;
        let span = type_mark.span.to(operand.span);
        Ok(Expr {
            kind: ExprKind::Qualified {
                type_mark: Box::new(type_mark),
                operand: Box::new(operand),
            },
            span,
        })
    }

    /// `( expr )`, a 2019 `( conditional_expression )`, or an aggregate.
    pub(super) fn paren_or_aggregate(&mut self) -> PResult<Expr> {
        self.paren_inner(true)
    }

    /// As `paren_or_aggregate`; `cond` is false for the operand of a
    /// qualified expression, which the 2019 grammar does not let be a
    /// conditional expression (VI#385).
    fn paren_inner(&mut self, cond: bool) -> PResult<Expr> {
        let start = self.expect(Tok::LParen)?;
        let mut elems = Vec::new();
        loop {
            elems.push(self.element_assoc()?);
            if elems.len() == 1 && elems[0].choices.is_empty() && self.at_kw(Kw::When) {
                let when = self.span();
                if !cond {
                    self.report(
                        Diagnostic::error(
                            codes::SYNTAX,
                            "a conditional expression cannot be the operand of a qualified \
                             expression",
                        )
                        .with_primary(when, "`when` is not allowed here")
                        .with_help("add parentheses: `T'((a when c else b))`"),
                    );
                    return Err(Recover);
                }
                let first = elems.pop().map(|e| e.value).ok_or(Recover)?;
                let e = self.cond_rest(first, false, Self::expr)?;
                self.expect(Tok::RParen)?;
                return Ok(Expr {
                    kind: ExprKind::Paren(Box::new(e)),
                    span: self.span_from(start),
                });
            }
            if !self.eat(Tok::Comma) {
                break;
            }
        }
        self.expect(Tok::RParen)?;
        let span = self.span_from(start);
        if elems.len() == 1 && elems[0].choices.is_empty() {
            let e = elems.pop().map(|e| e.value).ok_or(Recover)?;
            return Ok(Expr {
                kind: ExprKind::Paren(Box::new(e)),
                span,
            });
        }
        Ok(Expr {
            kind: ExprKind::Aggregate(elems),
            span,
        })
    }

    fn element_assoc(&mut self) -> PResult<ElementAssoc> {
        let start = self.span();
        let first = self.choice()?;
        if !matches!(self.peek(), Tok::Bar | Tok::Arrow) {
            return match first {
                Choice::Expr(value) => Ok(ElementAssoc {
                    choices: Vec::new(),
                    span: value.span,
                    value,
                }),
                // A range without `=>`: a positional slice of an aggregate is
                // not VHDL, but report it as a missing `=>`.
                _ => Err(self.expected("`=>`")),
            };
        }
        let mut choices = vec![first];
        while self.eat(Tok::Bar) {
            choices.push(self.choice()?);
        }
        self.expect(Tok::Arrow)?;
        let value = self.expr()?;
        Ok(ElementAssoc {
            choices,
            value,
            span: self.span_from(start),
        })
    }

    /// One choice: `others`, a range, a subtype, or an expression.
    pub(super) fn choice(&mut self) -> PResult<Choice> {
        if self.at_kw(Kw::Others) {
            return Ok(Choice::Others(self.bump_span()));
        }
        let e = self.expr()?;
        Ok(match self.range_after(e)? {
            RangeOrExpr::Expr(e) => Choice::Expr(e),
            RangeOrExpr::Range(r) => Choice::Range(DiscreteRange::Range(r)),
            RangeOrExpr::Subtype(s) => Choice::Range(DiscreteRange::Subtype(s)),
        })
    }

    pub(super) fn choices(&mut self) -> PResult<Vec<Choice>> {
        let mut v = vec![self.choice()?];
        while self.eat(Tok::Bar) {
            v.push(self.choice()?);
        }
        Ok(v)
    }

    /// Having parsed `e`, look for `to`/`downto` or `range` after it.
    fn range_after(&mut self, e: Expr) -> PResult<RangeOrExpr> {
        let dir = match self.peek() {
            Tok::Kw(Kw::To) => Direction::To,
            Tok::Kw(Kw::Downto) => Direction::Downto,
            Tok::Kw(Kw::Range) if self.nth(1) != Tok::Box => {
                let type_mark = expr_into_name(self, e, "a type mark before `range`")?;
                let span = type_mark.span;
                self.bump();
                let r = self.range()?;
                return Ok(RangeOrExpr::Subtype(SubtypeIndication {
                    resolution: None,
                    type_mark,
                    constraint: Some(Constraint::Range(Box::new(r))),
                    span: self.span_from(span),
                }));
            }
            _ => return Ok(RangeOrExpr::Expr(e)),
        };
        self.bump();
        let right = self.expr()?;
        Ok(RangeOrExpr::Range(Range::Simple {
            left: e,
            dir,
            right,
        }))
    }

    /// A range: `l to r`, `l downto r` or a range attribute name.
    pub(super) fn range(&mut self) -> PResult<Range> {
        let e = self.expr()?;
        match self.range_after(e)? {
            RangeOrExpr::Range(r) => Ok(r),
            RangeOrExpr::Expr(Expr {
                kind: ExprKind::Name(n),
                ..
            }) => Ok(Range::Name(n)),
            RangeOrExpr::Expr(_) => Err(self.expected("`to` or `downto`")),
            RangeOrExpr::Subtype(s) => {
                self.error(
                    s.span,
                    "expected a range",
                    "a subtype indication is not a range",
                );
                Err(Recover)
            }
        }
    }

    pub(super) fn discrete_range(&mut self) -> PResult<DiscreteRange> {
        if self.at(Tok::LParen) {
            // `(resolved) t range ...` is legal but unheard of; parse ranges.
            return Ok(DiscreteRange::Range(self.range()?));
        }
        let e = self.expr()?;
        match self.range_after(e)? {
            RangeOrExpr::Range(r) => Ok(DiscreteRange::Range(r)),
            RangeOrExpr::Subtype(s) => Ok(DiscreteRange::Subtype(s)),
            RangeOrExpr::Expr(Expr {
                kind: ExprKind::Name(n),
                ..
            }) => Ok(DiscreteRange::Range(Range::Name(n))),
            RangeOrExpr::Expr(_) => Err(self.expected("`to` or `downto`")),
        }
    }

    // Names.

    /// A full name, with selected, indexed/call, attribute and signature suffixes.
    /// Stops before `'(` (a qualified expression).
    pub(super) fn name(&mut self) -> PResult<Name> {
        self.name_opts(false)
    }

    /// A name; in an expression (`in_expr`) a VHDL-2019 `generic map` suffix
    /// is part of it, elsewhere it belongs to an instantiation.
    fn name_opts(&mut self, in_expr: bool) -> PResult<Name> {
        let n = self.name_prefix()?;
        self.name_suffixes(n, in_expr)
    }

    /// The suffixes after `n`; `in_expr` as for `name_opts`.
    pub(super) fn name_suffixes(&mut self, mut n: Name, in_expr: bool) -> PResult<Name> {
        loop {
            match self.peek() {
                Tok::Kw(Kw::Generic) if in_expr && self.nth(1) == Tok::Kw(Kw::Map) => {
                    let start = self.span();
                    self.require_2019(start, "a generic map in a subprogram call");
                    let args = self.map_aspect(Kw::Generic)?;
                    let span = self.span_from(n.span);
                    n = Name {
                        kind: NameKind::GenericMap {
                            prefix: Box::new(n),
                            args,
                        },
                        span,
                    };
                }
                Tok::Dot => n = self.selected(n)?,
                Tok::LParen => {
                    let args = self.assoc_list()?;
                    let span = self.span_from(n.span);
                    n = Name {
                        kind: NameKind::Apply {
                            prefix: Box::new(n),
                            args,
                        },
                        span,
                    };
                }
                Tok::Tick if self.nth(1) != Tok::LParen => n = self.attribute(n)?,
                Tok::LBracket if !self.psl_no_sig => n = self.signature_suffix(n)?,
                _ => return Ok(n),
            }
        }
    }

    /// A type mark: a name with selected and attribute suffixes but no parentheses,
    /// except the dimension of a 2019 `A'INDEX(n)` (16.2.3), which a constraint
    /// could not follow anyway (the index subtype is scalar).
    pub(super) fn type_mark(&mut self) -> PResult<Name> {
        let mut n = self.name_prefix()?;
        loop {
            match self.peek() {
                Tok::Dot => n = self.selected(n)?,
                Tok::Tick if self.nth(1) != Tok::LParen => n = self.attribute(n)?,
                // `arr(0)'subtype`: the parentheses index or slice the object name,
                // they are not an index constraint (which never has a `'` after it).
                Tok::LParen if self.toks[self.after_matching_paren(self.pos)].kind == Tok::Tick => {
                    let args = self.assoc_list()?;
                    let span = self.span_from(n.span);
                    n = Name {
                        kind: NameKind::Apply {
                            prefix: Box::new(n),
                            args,
                        },
                        span,
                    };
                }
                Tok::LParen if matches!(&n.kind, NameKind::Attribute { attr, .. } if attr.key.as_str() == "index") =>
                {
                    let args = self.assoc_list()?;
                    let span = self.span_from(n.span);
                    n = Name {
                        kind: NameKind::Apply {
                            prefix: Box::new(n),
                            args,
                        },
                        span,
                    };
                }
                _ => return Ok(n),
            }
        }
    }

    fn name_prefix(&mut self) -> PResult<Name> {
        let span = self.span();
        let kind = match self.peek() {
            Tok::Ident | Tok::ExtIdent => {
                let t = self.bump();
                NameKind::Simple(self.make_ident(t))
            }
            Tok::Str => {
                let t = self.bump();
                NameKind::Operator(Symbol::intern(&unquote(&self.text(t))))
            }
            Tok::Char => {
                self.bump();
                NameKind::Character(self.char_value(span)?)
            }
            Tok::LtLt => NameKind::External(Box::new(self.external_name()?)),
            _ => return Err(self.expected("name")),
        };
        Ok(Name {
            kind,
            span: self.span_from(span),
        })
    }

    fn selected(&mut self, prefix: Name) -> PResult<Name> {
        self.expect(Tok::Dot)?;
        let span = self.span();
        let suffix = match self.peek() {
            Tok::Ident | Tok::ExtIdent => {
                let t = self.bump();
                Suffix::Ident(self.make_ident(t))
            }
            Tok::Char => {
                self.bump();
                let value = self.char_value(span)?;
                Suffix::Char(CharLit { value, span })
            }
            Tok::Str => {
                let t = self.bump();
                Suffix::Operator(Symbol::intern(&unquote(&self.text(t))), span)
            }
            Tok::Kw(Kw::All) => {
                self.bump();
                Suffix::All(span)
            }
            _ => return Err(self.expected("identifier, character, operator or `all` after `.`")),
        };
        Ok(Name {
            span: prefix.span.to(span),
            kind: NameKind::Selected {
                prefix: Box::new(prefix),
                suffix,
            },
        })
    }

    fn attribute(&mut self, prefix: Name) -> PResult<Name> {
        self.expect(Tok::Tick)?;
        let attr = match self.peek() {
            Tok::Ident | Tok::ExtIdent => {
                let t = self.bump();
                self.make_ident(t)
            }
            // Reserved words that are also attribute names.
            Tok::Kw(Kw::Range | Kw::Subtype | Kw::Record) => {
                let t = self.bump();
                self.make_ident(t)
            }
            _ => return Err(self.expected("attribute name")),
        };
        Ok(Name {
            span: prefix.span.to(attr.span),
            kind: NameKind::Attribute {
                prefix: Box::new(prefix),
                attr,
            },
        })
    }

    fn signature_suffix(&mut self, prefix: Name) -> PResult<Name> {
        let signature = self.signature()?;
        Ok(Name {
            span: prefix.span.to(signature.span),
            kind: NameKind::Signature {
                prefix: Box::new(prefix),
                signature,
            },
        })
    }

    /// `[t1, t2 return t3]`
    pub(super) fn signature(&mut self) -> PResult<Signature> {
        let start = self.expect(Tok::LBracket)?;
        let mut params = Vec::new();
        if !matches!(self.peek(), Tok::Kw(Kw::Return) | Tok::RBracket) {
            params.push(self.type_mark()?);
            while self.eat(Tok::Comma) {
                params.push(self.type_mark()?);
            }
        }
        let ret = if self.eat_kw(Kw::Return) {
            Some(Box::new(self.type_mark()?))
        } else {
            None
        };
        self.expect(Tok::RBracket)?;
        Ok(Signature {
            params,
            ret,
            span: self.span_from(start),
        })
    }

    fn external_name(&mut self) -> PResult<ExternalName> {
        let open = self.expect(Tok::LtLt)?;
        let class = match self.peek() {
            Tok::Kw(Kw::Constant) => ExternalClass::Constant,
            Tok::Kw(Kw::Signal) => ExternalClass::Signal,
            Tok::Kw(Kw::Variable) => ExternalClass::Variable,
            _ => return Err(self.expected("`constant`, `signal` or `variable`")),
        };
        self.bump();
        let start = self.span();
        let kind = if self.eat(Tok::At) {
            PathKind::Package
        } else if self.eat(Tok::Dot) {
            PathKind::Absolute
        } else {
            let mut ups = 0;
            while self.eat(Tok::Caret) {
                ups += 1;
                self.expect(Tok::Dot)?;
            }
            PathKind::Relative(ups)
        };
        let mut elements = Vec::new();
        loop {
            let name = self.ident()?;
            let index = if self.eat(Tok::LParen) {
                let e = self.expr()?;
                self.expect(Tok::RParen)?;
                Some(e)
            } else {
                None
            };
            elements.push(PathElement { name, index });
            if !self.eat(Tok::Dot) {
                break;
            }
        }
        let path = ExternalPath {
            kind,
            elements,
            span: self.span_from(start),
        };
        self.expect(Tok::Colon)?;
        let subtype = self.subtype_indication()?;
        if !self.eat(Tok::GtGt) {
            let found = self.peek().describe();
            let span = self.span();
            self.report(
                Diagnostic::error(codes::SYNTAX, format!("expected `>>`, found {found}"))
                    .with_primary(span, "expected `>>`")
                    .with_secondary(open, "external name starts here"),
            );
            return Err(Recover);
        }
        Ok(ExternalName {
            class,
            path,
            subtype,
        })
    }

    // Association lists.

    /// `( [formal =>] actual, ... )`
    pub(super) fn assoc_list(&mut self) -> PResult<Vec<AssocElem>> {
        let mark = self.sc.assoc.len();
        match self.assoc_items() {
            Ok(()) => Ok(self.sc.assoc.drain(mark..).collect()),
            Err(e) => {
                self.sc.assoc.truncate(mark);
                Err(e)
            }
        }
    }

    fn assoc_items(&mut self) -> PResult<()> {
        self.expect(Tok::LParen)?;
        loop {
            let e = self.assoc_elem()?;
            self.sc.assoc.push(e);
            if !self.eat(Tok::Comma) {
                break;
            }
        }
        self.expect(Tok::RParen).map(|_| ())
    }

    fn assoc_elem(&mut self) -> PResult<AssocElem> {
        let start = self.span();
        let first = self.actual()?;
        if !self.eat(Tok::Arrow) {
            return Ok(AssocElem {
                formal: None,
                actual: first,
                span: self.span_from(start),
            });
        }
        let formal = match first {
            Actual::Expr(e) => expr_into_name(self, e, "a formal name before `=>`")?,
            _ => {
                let span = self.span_from(start);
                self.error(span, "expected a formal name before `=>`", "not a name");
                return Err(Recover);
            }
        };
        let actual = self.actual()?;
        Ok(AssocElem {
            formal: Some(formal),
            actual,
            span: self.span_from(start),
        })
    }

    fn actual(&mut self) -> PResult<Actual> {
        if self.at_kw(Kw::Open) {
            return Ok(Actual::Open(self.bump_span()));
        }
        if self.eat_kw(Kw::Inertial) {
            return Ok(Actual::Inertial(self.cond_expr()?));
        }
        let e = self.cond_expr()?;
        // `resolved std_logic` as a generic type actual.
        if let ExprKind::Name(_) = e.kind
            && self.at_ident()
        {
            let res = expr_into_name(self, e, "a name")?;
            let tm = self.type_mark()?;
            let st = self.subtype_rest(Some(Resolution::Function(res)), tm)?;
            return Ok(Actual::Subtype(Box::new(st)));
        }
        Ok(match self.range_after(e)? {
            RangeOrExpr::Expr(e) => Actual::Expr(e),
            RangeOrExpr::Range(r) => Actual::Range(Box::new(r)),
            RangeOrExpr::Subtype(s) => Actual::Subtype(Box::new(s)),
        })
    }

    // Subtype indications.

    pub(super) fn subtype_indication(&mut self) -> PResult<SubtypeIndication> {
        self.nested(|p| {
            let resolution = if p.at(Tok::LParen) {
                Some(p.paren_resolution()?)
            } else {
                None
            };
            let first = p.type_mark()?;
            if resolution.is_none() && p.at_ident() {
                let tm = p.type_mark()?;
                return p.subtype_rest(Some(Resolution::Function(first)), tm);
            }
            p.subtype_rest(resolution, first)
        })
    }

    /// The constraint part of a subtype indication whose type mark is parsed.
    fn subtype_rest(
        &mut self,
        resolution: Option<Resolution>,
        type_mark: Name,
    ) -> PResult<SubtypeIndication> {
        let start = type_mark.span;
        let constraint = self.opt_constraint()?;
        Ok(SubtypeIndication {
            resolution,
            type_mark,
            constraint,
            span: self.span_from(start),
        })
    }

    fn paren_resolution(&mut self) -> PResult<Resolution> {
        self.nested(|p| {
            p.expect(Tok::LParen)?;
            let r = if p.at(Tok::LParen) {
                Resolution::Element(Box::new(p.paren_resolution()?))
            } else if p.at_ident() && matches!(p.nth(1), Tok::Ident | Tok::ExtIdent | Tok::LParen) {
                let mut v = Vec::new();
                loop {
                    let id = p.ident()?;
                    let r = if p.at(Tok::LParen) {
                        p.paren_resolution()?
                    } else {
                        Resolution::Function(p.type_mark()?)
                    };
                    v.push((id, r));
                    if !p.eat(Tok::Comma) {
                        break;
                    }
                }
                Resolution::Record(v)
            } else {
                Resolution::Element(Box::new(Resolution::Function(p.type_mark()?)))
            };
            p.expect(Tok::RParen)?;
            Ok(r)
        })
    }

    pub(super) fn opt_constraint(&mut self) -> PResult<Option<Constraint>> {
        match self.peek() {
            Tok::Kw(Kw::Range) => {
                self.bump();
                Ok(Some(Constraint::Range(Box::new(self.range()?))))
            }
            Tok::LParen => Ok(Some(self.paren_constraint()?)),
            _ => Ok(None),
        }
    }

    /// `(` positioned: is this a record element constraint `(a(...)(...), ...)`?
    /// Not when a name suffix follows the parentheses, as in
    /// `(g(0)(1).f'range)` (#59).
    fn at_record_constraint(&self) -> bool {
        if !matches!(self.nth(1), Tok::Ident | Tok::ExtIdent) || self.nth(2) != Tok::LParen {
            return false;
        }
        let mut after = self.after_matching_paren(self.pos + 2);
        while self.toks[after].kind == Tok::LParen {
            after = self.after_matching_paren(after);
        }
        matches!(self.toks[after].kind, Tok::Comma | Tok::RParen)
    }

    fn paren_constraint(&mut self) -> PResult<Constraint> {
        self.nested(|p| {
            let start = p.span();
            if p.at_record_constraint() {
                p.bump();
                let mut v = Vec::new();
                loop {
                    let id = p.ident()?;
                    let c = p.paren_constraint()?;
                    v.push((id, c));
                    if !p.eat(Tok::Comma) {
                        break;
                    }
                }
                p.expect(Tok::RParen)?;
                return Ok(Constraint::Record(v, p.span_from(start)));
            }
            p.expect(Tok::LParen)?;
            let indexes = if p.at_kw(Kw::Open) && p.nth(1) == Tok::RParen {
                p.bump();
                None
            } else {
                let mut v = vec![p.discrete_range()?];
                while p.eat(Tok::Comma) {
                    v.push(p.discrete_range()?);
                }
                Some(v)
            };
            p.expect(Tok::RParen)?;
            let element = if p.at(Tok::LParen) {
                Some(Box::new(p.paren_constraint()?))
            } else {
                None
            };
            Ok(Constraint::Array {
                indexes,
                element,
                span: p.span_from(start),
            })
        })
    }
}

enum RangeOrExpr {
    Expr(Expr),
    Range(Range),
    Subtype(SubtypeIndication),
}
