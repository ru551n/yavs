//! Expressions of an elaborated module: literals, names, and the sizing and sign rules of IEEE
//! 1364-2005 5.4 and 5.5 decided here, so the run-time tree (`exec::E`) is fixed-width: every
//! operand arrives at the width its operator needs, with explicit extensions.
//!
//! The algorithm is the clause's: the self-determined type of each expression bottom up
//! (`ty`, Table 5-22), then the context width and the expression's own sign pushed down to the
//! context-determined operands (`build`); a self-determined operand is built at its own type and
//! extended as a leaf.

use std::collections::HashMap;

use crate::ast::{Expr, Ident, RangeKind};
use crate::exec::{BOp, E, RBOp, UOp};
use crate::value::{Bit, V4};
use crate::{Diag, Span};

pub(crate) type R<T> = Result<T, Box<Diag>>;

/// The type of an expression: a vector of `w` bits, signed or not, or a real.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Ty {
    pub w: u32,
    pub signed: bool,
    pub real: bool,
}

impl Ty {
    pub(crate) fn vec(w: u32, signed: bool) -> Ty {
        Ty {
            w,
            signed,
            real: false,
        }
    }

    pub(crate) const REAL: Ty = Ty {
        w: 64,
        signed: true,
        real: true,
    };

    const INTEGER: Ty = Ty {
        w: 32,
        signed: true,
        real: false,
    };
}

/// A constant: a parameter's value, or a folded literal.
#[derive(Clone, Debug)]
pub(crate) enum Const {
    V(V4, bool),
    R(f64),
}

impl Const {
    pub(crate) fn ty(&self) -> Ty {
        match self {
            Const::V(v, s) => Ty::vec(v.width(), *s),
            Const::R(_) => Ty::REAL,
        }
    }
}

/// A net or variable of the instance: domain net `net`, declared `[msb:lsb]`.
#[derive(Clone, Debug)]
pub(crate) struct Var {
    pub name: String,
    pub net: u32,
    pub msb: i64,
    pub lsb: i64,
    pub signed: bool,
    /// A `reg`, `integer` or `time` variable (procedural targets), not a net.
    pub reg: bool,
    pub pos: u32,
}

impl Var {
    pub(crate) fn w(&self) -> u32 {
        (self.msb - self.lsb).unsigned_abs() as u32 + 1
    }
}

#[derive(Clone, Debug)]
pub(crate) enum Sym {
    Param(Const),
    Var(usize),
}

/// What the expressions of one instance see.
pub(crate) struct Sx<'a> {
    pub syms: &'a HashMap<String, Sym>,
    pub vars: &'a [Var],
    /// The span diagnostics point at when an expression has no name to point at.
    pub at: Span,
    pub span: &'a dyn Fn(u32, u32) -> Span,
    /// Names are constants only (parameters, ranges, replication counts).
    pub konst: bool,
    /// The module's time unit, for `$time`.
    pub unit_fs: u64,
}

pub(crate) fn unsupported(what: &str, at: Span) -> Box<Diag> {
    Box::new(
        Diag::error(
            "E4501",
            format!("{what} is not implemented in Verilog elaboration"),
            at,
        )
        .label("reached here"),
    )
}

/// The first identifier of an expression, for a span.
fn first_ident(e: &Expr) -> Option<&Ident> {
    match e {
        Expr::Ident(i) => Some(i),
        Expr::Dot(b, i) => first_ident(b).or(Some(i)),
        Expr::Index(a, b) => first_ident(a).or_else(|| first_ident(b)),
        Expr::Range(a, _, b, c) => first_ident(a)
            .or_else(|| first_ident(b))
            .or_else(|| first_ident(c)),
        Expr::Concat(v) => v.iter().find_map(first_ident),
        Expr::Repl(n, v) => first_ident(n).or_else(|| v.iter().find_map(first_ident)),
        Expr::Unary(_, a) => first_ident(a),
        Expr::Binary(_, a, b) => first_ident(a).or_else(|| first_ident(b)),
        Expr::Cond(a, b, c) => first_ident(a)
            .or_else(|| first_ident(b))
            .or_else(|| first_ident(c)),
        Expr::Call(f, v) => first_ident(f).or_else(|| v.iter().find_map(first_ident)),
        Expr::SysCall(_, v) => v.iter().find_map(first_ident),
        Expr::MinTypMax(_, b, _) => first_ident(b),
        _ => None,
    }
}

impl Sx<'_> {
    pub(crate) fn span_of(&self, e: &Expr) -> Span {
        match first_ident(e) {
            Some(i) => (self.span)(i.pos, i.pos + i.name.len() as u32),
            None => self.at,
        }
    }

    fn err(&self, code: &'static str, msg: String, e: &Expr) -> Box<Diag> {
        Box::new(Diag::error(code, msg, self.span_of(e)))
    }

    fn lookup(&self, i: &Ident) -> R<&Sym> {
        self.syms.get(&i.name).ok_or_else(|| {
            Box::new(
                Diag::error(
                    "E4504",
                    format!("`{}` is not declared", i.name),
                    (self.span)(i.pos, i.pos + i.name.len() as u32),
                )
                .note("IEEE 1364-2005 12.7: a name is declared in the module before it is used in an expression"),
            )
        })
    }

    fn var(&self, i: &Ident) -> R<Option<&Var>> {
        match self.lookup(i)? {
            Sym::Var(_) if self.konst => Err(Box::new(
                Diag::error(
                    "E4505",
                    format!("`{}` is not a constant", i.name),
                    (self.span)(i.pos, i.pos + i.name.len() as u32),
                )
                .note(
                    "IEEE 1364-2005 5.2: a constant expression names parameters and literals only",
                ),
            )),
            Sym::Var(k) => Ok(Some(&self.vars[*k])),
            Sym::Param(_) => Ok(None),
        }
    }

    /// The self-determined type (Table 5-22).
    pub(crate) fn ty(&self, e: &Expr) -> R<Ty> {
        Ok(match e {
            Expr::Num(s) => number(s).map_err(|m| self.err("E4506", m, e))?.ty(),
            Expr::Real(_) => Ty::REAL,
            Expr::Str(s) => Ty::vec(8 * string_bytes(s).len().max(1) as u32, false),
            Expr::Ident(i) => match self.lookup(i)? {
                Sym::Param(c) => c.ty(),
                Sym::Var(k) => {
                    let v = &self.vars[*k];
                    Ty::vec(v.w(), v.signed)
                }
            },
            Expr::Index(..) => Ty::vec(1, false),
            Expr::Range(_, k, a, b) => Ty::vec(self.part_width(*k, a, b, e)?, false),
            Expr::Concat(v) => {
                let mut w = 0u32;
                for p in v {
                    w += self.concat_part(p)?.w;
                }
                Ty::vec(w, false)
            }
            Expr::Repl(n, v) => {
                let k = self.count(n)?;
                let mut w = 0u32;
                for p in v {
                    w += self.concat_part(p)?.w;
                }
                Ty::vec(w * k, false)
            }
            Expr::Unary(op, a) => match *op {
                "+" | "-" | "~" => self.ty(a)?,
                "!" => Ty::vec(1, false),
                _ => Ty::vec(1, false),
            },
            Expr::Binary(op, a, b) => {
                let (ta, tb) = (self.ty(a)?, self.ty(b)?);
                match *op {
                    "+" | "-" | "*" | "/" | "%" | "&" | "|" | "^" | "^~" | "~^" => {
                        if ta.real || tb.real {
                            if matches!(*op, "&" | "|" | "^" | "^~" | "~^" | "%") {
                                return Err(self.err(
                                    "E4506",
                                    format!("`{op}` takes no real operand (IEEE 1364-2005 4.8.1)"),
                                    e,
                                ));
                            }
                            Ty::REAL
                        } else {
                            Ty::vec(ta.w.max(tb.w), ta.signed && tb.signed)
                        }
                    }
                    "**" if ta.real || tb.real => Ty::REAL,
                    "<<" | ">>" | "<<<" | ">>>" | "**" => ta,
                    _ => Ty::vec(1, false),
                }
            }
            Expr::Cond(_, a, b) => {
                let (ta, tb) = (self.ty(a)?, self.ty(b)?);
                if ta.real || tb.real {
                    Ty::REAL
                } else {
                    Ty::vec(ta.w.max(tb.w), ta.signed && tb.signed)
                }
            }
            Expr::MinTypMax(_, t, _) => self.ty(t)?,
            Expr::SysCall(n, args) => match (n.as_str(), args.as_slice()) {
                ("$signed", [a]) => Ty::vec(self.ty(a)?.w, true),
                ("$unsigned", [a]) => Ty::vec(self.ty(a)?.w, false),
                ("$clog2", [_]) => Ty::INTEGER,
                ("$time", []) => Ty::vec(64, false),
                ("$stime", []) => Ty::vec(32, false),
                ("$realtime", []) => Ty::REAL,
                _ => {
                    return Err(unsupported(
                        &format!("the system function `{n}`"),
                        self.span_of(e),
                    ));
                }
            },
            Expr::Dot(..) => return Err(unsupported("a hierarchical name", self.span_of(e))),
            Expr::Call(..) => return Err(unsupported("a function call", self.span_of(e))),
            Expr::Empty => return Err(self.err("E4506", "an empty expression".into(), e)),
        })
    }

    fn concat_part(&self, p: &Expr) -> R<Ty> {
        if let Expr::Num(s) = p
            && number(s).is_ok_and(|n| n.unsized_)
        {
            return Err(self.err(
                "E4506",
                format!("the unsized constant `{s}` in a concatenation (IEEE 1364-2005 5.1.14)"),
                p,
            ));
        }
        let t = self.ty(p)?;
        if t.real {
            return Err(self.err(
                "E4506",
                "a real operand in a concatenation (IEEE 1364-2005 4.8.1)".into(),
                p,
            ));
        }
        Ok(t)
    }

    /// A replication count: a constant, at least 1.
    fn count(&self, n: &Expr) -> R<u32> {
        let v = self.konst_int(n)?;
        if !(1..=1 << 24).contains(&v) {
            return Err(self.err(
                "E4506",
                format!("a replication count of {v}: it must be a positive constant (IEEE 1364-2005 5.1.14)"),
                n,
            ));
        }
        Ok(v as u32)
    }

    /// A constant integer expression (ranges, counts, part-select widths).
    pub(crate) fn konst_int(&self, e: &Expr) -> R<i64> {
        let sx = Sx {
            konst: true,
            ..*self
        };
        let t = sx.ty(e)?;
        let node = sx.build(e, t)?;
        match crate::exec::eval_const(&node) {
            Const::V(v, s) => v.to_i64(s).ok_or_else(|| {
                self.err(
                    "E4506",
                    format!(
                        "the constant `{}` has an x or z bit or does not fit 64 bits",
                        v
                    ),
                    e,
                )
            }),
            Const::R(r) => Ok(real_to_i64(r)),
        }
    }

    /// A constant value of its own type.
    pub(crate) fn konst(&self, e: &Expr) -> R<Const> {
        let sx = Sx {
            konst: true,
            ..*self
        };
        let t = sx.ty(e)?;
        let node = sx.build(e, t)?;
        Ok(crate::exec::eval_const(&node))
    }

    fn part_width(&self, k: RangeKind, a: &Expr, b: &Expr, e: &Expr) -> R<u32> {
        match k {
            RangeKind::Colon => {
                Ok((self.konst_int(a)? - self.konst_int(b)?).unsigned_abs() as u32 + 1)
            }
            _ => {
                let w = self.konst_int(b)?;
                if w < 1 {
                    return Err(self.err(
                        "E4506",
                        format!("the width {w} of an indexed part-select must be positive (IEEE 1364-2005 5.2.1)"),
                        e,
                    ));
                }
                Ok(w as u32)
            }
        }
    }

    /// `e` at its own type.
    pub(crate) fn self_det(&self, e: &Expr) -> R<(E, Ty)> {
        let t = self.ty(e)?;
        Ok((self.build(e, t)?, t))
    }

    /// A condition: any width, tested by its truth (`V4::truth`); a real tests against 0.
    pub(crate) fn cond(&self, e: &Expr) -> R<E> {
        let (n, t) = self.self_det(e)?;
        Ok(if t.real {
            E::RCmp(BOp::Ne, Box::new(n), Box::new(E::R(0.0)))
        } else {
            n
        })
    }

    /// `e` as an operand of `ctx`: a vector of `ctx.w` bits whose extension follows `ctx.signed`,
    /// or a real.
    pub(crate) fn build(&self, e: &Expr, ctx: Ty) -> R<E> {
        let t = self.ty(e)?;
        if ctx.real {
            return self.build_real(e, t);
        }
        if t.real {
            return Ok(E::ToVec(Box::new(self.build_real(e, t)?), ctx.w));
        }
        let ext = |n: E, w: u32| -> E {
            if w == ctx.w {
                n
            } else {
                E::Ext(Box::new(n), ctx.w, ctx.signed)
            }
        };
        Ok(match e {
            Expr::Num(_) | Expr::Str(_) => {
                let c = self.literal(e)?;
                match c {
                    Const::V(v, _) => E::K(v.ext(ctx.w, ctx.signed)),
                    Const::R(_) => unreachable!("a real literal has a real type"),
                }
            }
            Expr::Ident(i) => match self.var(i)? {
                Some(v) => ext(E::Var(v.net), v.w()),
                None => match self.lookup(i)? {
                    Sym::Param(Const::V(v, _)) => E::K(v.ext(ctx.w, ctx.signed)),
                    _ => unreachable!("a real parameter has a real type"),
                },
            },
            Expr::Index(base, i) => ext(self.select(e, base, i, None)?, 1),
            Expr::Range(base, k, a, b) => {
                let n = self.part_width(*k, a, b, e)?;
                ext(self.select(e, base, a, Some((*k, b, n)))?, n)
            }
            Expr::Concat(v) => {
                let parts = v
                    .iter()
                    .map(|p| self.self_det(p).map(|x| x.0))
                    .collect::<R<Vec<_>>>()?;
                ext(E::Cat(parts), t.w)
            }
            Expr::Repl(n, v) => {
                let k = self.count(n)?;
                let mut parts = Vec::new();
                for _ in 0..k {
                    for p in v {
                        parts.push(self.self_det(p)?.0);
                    }
                }
                ext(E::Cat(parts), t.w)
            }
            Expr::Unary(op, a) => match *op {
                "+" => self.build(a, ctx)?,
                "-" => E::Un(UOp::Neg, Box::new(self.build(a, ctx)?)),
                "~" => E::Un(UOp::Not, Box::new(self.build(a, ctx)?)),
                _ => {
                    let u = match *op {
                        "!" => UOp::LNot,
                        "&" => UOp::RedAnd,
                        "~&" => UOp::RedNand,
                        "|" => UOp::RedOr,
                        "~|" => UOp::RedNor,
                        "^" => UOp::RedXor,
                        _ => UOp::RedXnor,
                    };
                    if *op == "!" && self.ty(a)?.real {
                        let r = self.build_real(a, Ty::REAL)?;
                        return Ok(ext(E::RCmp(BOp::Eq, Box::new(r), Box::new(E::R(0.0))), 1));
                    }
                    if self.ty(a)?.real {
                        return Err(self.err(
                            "E4506",
                            format!("`{op}` takes no real operand (IEEE 1364-2005 4.8.1)"),
                            e,
                        ));
                    }
                    ext(E::Un(u, Box::new(self.self_det(a)?.0)), 1)
                }
            },
            Expr::Binary(op, a, b) => self.binary(op, a, b, ctx, e)?,
            Expr::Cond(c, a, b) => E::Cond(
                Box::new(self.cond(c)?),
                Box::new(self.build(a, ctx)?),
                Box::new(self.build(b, ctx)?),
            ),
            Expr::MinTypMax(_, t, _) => self.build(t, ctx)?,
            Expr::SysCall(n, args) => match (n.as_str(), args.as_slice()) {
                ("$signed" | "$unsigned", [a]) => ext(self.self_det(a)?.0, t.w),
                ("$clog2", [a]) => {
                    let (x, _) = self.self_det(a)?;
                    ext(E::Un(UOp::Clog2, Box::new(x)), 32)
                }
                ("$time", []) => ext(E::Time(self.unit_fs, 64), 64),
                ("$stime", []) => ext(E::Time(self.unit_fs, 32), 32),
                _ => {
                    return Err(unsupported(
                        &format!("the system function `{n}`"),
                        self.span_of(e),
                    ));
                }
            },
            _ => {
                self.ty(e)?;
                return Err(unsupported("this expression", self.span_of(e)));
            }
        })
    }

    fn binary(&self, op: &str, a: &Expr, b: &Expr, ctx: Ty, e: &Expr) -> R<E> {
        let bx = Box::new;
        let arith = |o: BOp| -> R<E> {
            Ok(E::Bin(
                o,
                bx(self.build(a, ctx)?),
                bx(self.build(b, ctx)?),
                ctx.signed,
            ))
        };
        let rel = |o: BOp| -> R<E> {
            let (ta, tb) = (self.ty(a)?, self.ty(b)?);
            let n = if ta.real || tb.real {
                E::RCmp(o, bx(self.build_real(a, ta)?), bx(self.build_real(b, tb)?))
            } else {
                let inner = Ty::vec(ta.w.max(tb.w), ta.signed && tb.signed);
                E::Bin(
                    o,
                    bx(self.build(a, inner)?),
                    bx(self.build(b, inner)?),
                    inner.signed,
                )
            };
            Ok(if ctx.w == 1 {
                n
            } else {
                E::Ext(bx(n), ctx.w, ctx.signed)
            })
        };
        let logical = |o: BOp| -> R<E> {
            let n = E::Bin(o, bx(self.cond(a)?), bx(self.cond(b)?), false);
            Ok(if ctx.w == 1 {
                n
            } else {
                E::Ext(bx(n), ctx.w, ctx.signed)
            })
        };
        match op {
            "+" => arith(BOp::Add),
            "-" => arith(BOp::Sub),
            "*" => arith(BOp::Mul),
            "/" => arith(BOp::Div),
            "%" => arith(BOp::Rem),
            "&" => arith(BOp::And),
            "|" => arith(BOp::Or),
            "^" => arith(BOp::Xor),
            "^~" | "~^" => arith(BOp::Xnor),
            "<" => rel(BOp::Lt),
            "<=" => rel(BOp::Le),
            ">" => rel(BOp::Gt),
            ">=" => rel(BOp::Ge),
            "==" => rel(BOp::Eq),
            "!=" => rel(BOp::Ne),
            "===" | "!==" => {
                let (ta, tb) = (self.ty(a)?, self.ty(b)?);
                if ta.real || tb.real {
                    return Err(self.err(
                        "E4506",
                        format!("`{op}` takes no real operand (IEEE 1364-2005 4.8.1)"),
                        e,
                    ));
                }
                rel(if op == "===" {
                    BOp::CaseEq
                } else {
                    BOp::CaseNe
                })
            }
            "&&" => logical(BOp::LAnd),
            "||" => logical(BOp::LOr),
            "<<" | "<<<" | ">>" | ">>>" | "**" => {
                let (amount, at) = self.self_det(b)?;
                if at.real {
                    return Err(self.err(
                        "E4506",
                        format!("`{op}` takes no real right operand here"),
                        e,
                    ));
                }
                let o = match op {
                    "<<" | "<<<" => BOp::Shl,
                    ">>" => BOp::Shr,
                    ">>>" => BOp::Ashr,
                    _ => BOp::Pow(at.signed),
                };
                Ok(E::Bin(o, bx(self.build(a, ctx)?), bx(amount), ctx.signed))
            }
            _ => Err(unsupported(
                &format!("the operator `{op}`"),
                self.span_of(e),
            )),
        }
    }

    fn build_real(&self, e: &Expr, t: Ty) -> R<E> {
        let bx = Box::new;
        if !t.real {
            let (n, t) = self.self_det(e)?;
            return Ok(E::ToReal(bx(n), t.signed));
        }
        Ok(match e {
            Expr::Real(s) => E::R(real_literal(s).map_err(|m| self.err("E4506", m, e))?),
            Expr::Ident(i) => match self.lookup(i)? {
                Sym::Param(Const::R(r)) => E::R(*r),
                _ => return Err(unsupported("a real variable", self.span_of(e))),
            },
            Expr::Unary("-", a) => E::RUn(bx(self.build_real(a, Ty::REAL)?)),
            Expr::Unary("+", a) => self.build_real(a, Ty::REAL)?,
            Expr::Binary(op, a, b) => {
                let o = match *op {
                    "+" => RBOp::Add,
                    "-" => RBOp::Sub,
                    "*" => RBOp::Mul,
                    "/" => RBOp::Div,
                    "**" => RBOp::Pow,
                    _ => {
                        return Err(unsupported(
                            &format!("the real operator `{op}`"),
                            self.span_of(e),
                        ));
                    }
                };
                let (ta, tb) = (self.ty(a)?, self.ty(b)?);
                E::RBin(o, bx(self.build_real(a, ta)?), bx(self.build_real(b, tb)?))
            }
            Expr::Cond(c, a, b) => {
                let (ta, tb) = (self.ty(a)?, self.ty(b)?);
                E::RCond(
                    bx(self.cond(c)?),
                    bx(self.build_real(a, ta)?),
                    bx(self.build_real(b, tb)?),
                )
            }
            Expr::MinTypMax(_, x, _) => self.build_real(x, t)?,
            Expr::SysCall(n, _) if n == "$realtime" => E::RTime(self.unit_fs),
            _ => return Err(unsupported("this real expression", self.span_of(e))),
        })
    }

    /// A literal's value at its own type.
    fn literal(&self, e: &Expr) -> R<Const> {
        match e {
            Expr::Num(s) => {
                let n = number(s).map_err(|m| self.err("E4506", m, e))?;
                Ok(Const::V(n.value, n.signed))
            }
            Expr::Str(s) => Ok(Const::V(string_value(&string_bytes(s)), false)),
            _ => unreachable!(),
        }
    }

    /// `base[i]` (`part` `None`) or a part-select `base[a:b]`, `[a+:n]`, `[a-:n]` of `n` bits.
    fn select(
        &self,
        e: &Expr,
        base: &Expr,
        i: &Expr,
        part: Option<(RangeKind, &Expr, u32)>,
    ) -> R<E> {
        let Expr::Ident(name) = base else {
            return Err(unsupported(
                "a select of something other than a name",
                self.span_of(e),
            ));
        };
        let (net, msb, lsb) = match self.var(name)? {
            Some(v) => (Some(v.net), v.msb, v.lsb),
            None => match self.lookup(name)? {
                // A parameter without a range is numbered from 0 (12.2).
                Sym::Param(Const::V(v, _)) => (None, v.width() as i64 - 1, 0),
                _ => {
                    return Err(self.err(
                        "E4506",
                        format!("`{}` is a real, it has no bits", name.name),
                        e,
                    ));
                }
            },
        };
        let desc = msb >= lsb;
        let n = part.map_or(1, |p| p.2);
        // The element offset of the select's lowest element for index value `k`:
        // `desc`: k - adj, else adj - k (Sel in exec.rs).
        let (idx, adj) = match part {
            None => (self.self_det(i)?, lsb),
            Some((RangeKind::Colon, b, _)) => {
                let (hi, lo) = (self.konst_int(i)?, self.konst_int(b)?);
                if (hi >= lo) != desc && hi != lo {
                    return Err(self.err(
                        "E4506",
                        format!(
                            "the part-select [{hi}:{lo}] runs the other way than `{}` [{msb}:{lsb}] (IEEE 1364-2005 5.2.1)",
                            name.name
                        ),
                        e,
                    ));
                }
                // The lowest element is that of the right index either way.
                ((E::K(V4::from_i64(64, lo)), Ty::vec(64, true)), lsb)
            }
            Some((RangeKind::PlusColon, _, n)) => {
                // Indices i .. i + n - 1: the lowest element is i's when descending,
                // (i + n - 1)'s when ascending.
                let adj = if desc { lsb } else { lsb - (n as i64 - 1) };
                (self.self_det(i)?, adj)
            }
            Some((RangeKind::MinusColon, _, n)) => {
                let adj = if desc { lsb + n as i64 - 1 } else { lsb };
                (self.self_det(i)?, adj)
            }
        };
        if idx.1.real {
            return Err(self.err("E4506", "a real index (IEEE 1364-2005 4.8.1)".into(), e));
        }
        let src = match net {
            Some(net) => E::Var(net),
            None => match self.lookup(name)? {
                Sym::Param(Const::V(v, _)) => E::K(v.clone()),
                _ => unreachable!(),
            },
        };
        Ok(E::Sel {
            src: Box::new(src),
            idx: Box::new(idx.0),
            idx_signed: idx.1.signed,
            desc,
            adj,
            n,
        })
    }
}

/// A number literal (3.5.1).
pub(crate) struct Number {
    pub value: V4,
    pub signed: bool,
    pub unsized_: bool,
}

impl Number {
    fn ty(&self) -> Ty {
        Ty::vec(self.value.width(), self.signed)
    }
}

/// Parses `12`, `8'hff`, `'b1x`, `4'sd3`, `8 'h ff` (white space and `_` allowed).
pub(crate) fn number(s: &str) -> Result<Number, String> {
    let t: String = s
        .chars()
        .filter(|c| !c.is_whitespace() && *c != '_')
        .collect();
    let Some(q) = t.find('\'') else {
        // A plain decimal: a signed integer of at least 32 bits.
        let v = decimal(&t, 32)?;
        let w = 32.max(v.1 + 1);
        return Ok(Number {
            value: v.0.ext(w, false),
            signed: true,
            unsized_: true,
        });
    };
    let size = match &t[..q] {
        "" => None,
        d => Some(
            d.parse::<u32>()
                .ok()
                .filter(|&n| n > 0)
                .ok_or_else(|| format!("`{s}` has a bad size"))?,
        ),
    };
    let mut rest = &t[q + 1..];
    let signed = rest.starts_with(['s', 'S']);
    if signed {
        rest = &rest[1..];
    }
    let base = rest
        .chars()
        .next()
        .ok_or("a based literal without a base")?;
    let digits = &rest[1..];
    if digits.is_empty() {
        return Err(format!("`{s}` has no digits"));
    }
    let k = match base.to_ascii_lowercase() {
        'b' => 1,
        'o' => 3,
        'h' => 4,
        'd' => 0,
        _ => return Err(format!("`{s}` has a bad base")),
    };
    let (raw, need) = if k == 0 {
        match digits.to_ascii_lowercase().as_str() {
            "x" => (V4::xs(1), 1),
            "z" | "?" => (V4::zs(1), 1),
            _ => decimal(digits, 1)?,
        }
    } else {
        let n = digits.len() as u32 * k;
        let mut v = V4::zeros(n);
        for (j, c) in digits.chars().rev().enumerate() {
            let bits: Option<u32> = c.to_digit(1 << k);
            for b in 0..k {
                let bit = match c.to_ascii_lowercase() {
                    'x' => Bit::X,
                    'z' | '?' => Bit::Z,
                    _ => match bits {
                        Some(d) if d >> b & 1 == 1 => Bit::One,
                        Some(_) => Bit::Zero,
                        None => return Err(format!("`{c}` is not a digit of base {base}")),
                    },
                };
                v.set(j as u32 * k + b, bit);
            }
        }
        (v, n)
    };
    let w = size.unwrap_or(32.max(need));
    // The leftmost digit's x or z fills the rest; otherwise zeros (3.5.1).
    let fill_xz = matches!(raw.msb(), Bit::X | Bit::Z);
    Ok(Number {
        value: raw.ext(w, fill_xz),
        signed,
        unsized_: size.is_none(),
    })
}

/// A decimal digit string as a known vector and the bits it needs.
fn decimal(d: &str, min: u32) -> Result<(V4, u32), String> {
    let mut limbs: Vec<u64> = vec![0];
    for c in d.chars() {
        let dv = c
            .to_digit(10)
            .ok_or_else(|| format!("`{c}` is not a decimal digit"))? as u128;
        let mut carry = dv;
        for l in limbs.iter_mut() {
            let t = *l as u128 * 10 + carry;
            *l = t as u64;
            carry = t >> 64;
        }
        if carry != 0 {
            limbs.push(carry as u64);
        }
    }
    let bits = limbs
        .iter()
        .enumerate()
        .rev()
        .find(|(_, l)| **l != 0)
        .map_or(0, |(i, l)| i as u32 * 64 + 64 - l.leading_zeros());
    let w = bits.max(min).max(1);
    Ok((V4::from_words(w, &limbs), bits))
}

fn real_literal(s: &str) -> Result<f64, String> {
    let t: String = s.chars().filter(|c| *c != '_').collect();
    t.parse()
        .map_err(|_| format!("`{s}` is not a real literal"))
}

/// The bytes of a string literal, escapes processed (3.6.3).
pub(crate) fn string_bytes(s: &str) -> Vec<u8> {
    let b = s.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 1 < b.len() {
            i += 1;
            match b[i] {
                b'n' => out.push(b'\n'),
                b't' => out.push(b'\t'),
                b'\\' => out.push(b'\\'),
                b'"' => out.push(b'"'),
                c @ b'0'..=b'7' => {
                    let mut v = (c - b'0') as u32;
                    let mut n = 1;
                    while n < 3 && i + 1 < b.len() && (b'0'..=b'7').contains(&b[i + 1]) {
                        i += 1;
                        v = v * 8 + (b[i] - b'0') as u32;
                        n += 1;
                    }
                    out.push(v as u8);
                }
                c => out.push(c),
            }
        } else {
            out.push(b[i]);
        }
        i += 1;
    }
    out
}

/// A string as a vector of 8 bits per character, the last character in the low byte (3.6).
pub(crate) fn string_value(bytes: &[u8]) -> V4 {
    let n = bytes.len().max(1) as u32;
    let mut v = V4::zeros(8 * n);
    for (j, &c) in bytes.iter().rev().enumerate() {
        v.put(8 * j as u32, &V4::from_u64(8, c as u64));
    }
    v
}

pub(crate) use crate::value::real_to_i64;
