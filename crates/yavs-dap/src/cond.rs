//! Breakpoint conditions (#326): a VHDL boolean expression over the locals and signals a stop
//! sees, parsed by `yavs-syntax` and evaluated over the values the debugger shows (`Lit`).
//!
//! ponytail: predefined operators on scalars (and `=`, `/=` and ordering on arrays) only, no
//! function calls, attributes or overloaded operators (`numeric_std."+"`); lowering the
//! condition to an IR function nested in the statement's (ir.md 3.4) would allow them, once
//! a running program can take new functions.

use std::cmp::Ordering;

use yavs_std::types::{Bounds, EnumLit, EnumType, boolean_type};
use yavs_std::value::{Lit, LitType, literal};
use yavs_syntax::ast::{
    Actual, BinaryOp, Decl, Expr, ExprKind, LibraryUnit, LiteralKind, Name, NameKind, Suffix,
    UnaryOp,
};

/// A parsed condition.
pub(crate) struct Cond {
    /// The text the expression was parsed from; spans index it.
    src: String,
    expr: Expr,
}

/// What a condition reads: a local or signal by name or path, with its type.
pub(crate) trait Env {
    fn get(&self, name: &str) -> Option<(LitType, Lit)>;
}

const PREFIX: &str = "package p is constant c : boolean := ";

/// Parse `text`; the error is the parser's first message.
pub(crate) fn parse(text: &str) -> Result<Cond, String> {
    let src = format!("{PREFIX}{text}\n; end package;");
    let p = yavs_syntax::parse_str(yavs_diag::FileId(0), &src, &yavs_syntax::Options::default());
    let unit = p.design.and_then(|d| d.units.into_iter().next());
    let expr = match unit.map(|u| u.unit) {
        Some(LibraryUnit::Package(pk)) => pk.decls.into_iter().find_map(|d| match d {
            Decl::Object(o) => o.default,
            _ => None,
        }),
        _ => None,
    };
    let Some(expr) = expr else {
        let first = p.diags.first().map(|d| d.message.clone());
        return Err(format!(
            "condition `{text}` does not parse: {}",
            // The `;` is the wrapper's.
            first
                .unwrap_or_else(|| "expected an expression".into())
                .replace("found `;`", "found the end")
        ));
    };
    let c = Cond { src, expr };
    c.check(&c.expr)?;
    Ok(c)
}

/// An operator as written.
fn op_text(op: BinaryOp) -> &'static str {
    use BinaryOp as B;
    match op {
        B::And => "and",
        B::Or => "or",
        B::Nand => "nand",
        B::Nor => "nor",
        B::Xor => "xor",
        B::Xnor => "xnor",
        B::Eq => "=",
        B::Ne => "/=",
        B::Lt => "<",
        B::Le => "<=",
        B::Gt => ">",
        B::Ge => ">=",
        B::MatchEq => "?=",
        B::MatchNe => "?/=",
        B::MatchLt => "?<",
        B::MatchLe => "?<=",
        B::MatchGt => "?>",
        B::MatchGe => "?>=",
        B::Sll => "sll",
        B::Srl => "srl",
        B::Sla => "sla",
        B::Sra => "sra",
        B::Rol => "rol",
        B::Ror => "ror",
        B::Add => "+",
        B::Sub => "-",
        B::Concat => "&",
        B::Mul => "*",
        B::Div => "/",
        B::Mod => "mod",
        B::Rem => "rem",
        B::Pow => "**",
    }
}

/// A value during evaluation: typed, or a literal (or unknown name) still to be read in the
/// type of the other operand.
enum V {
    Typed(LitType, Lit),
    Text(String),
}

fn boolean(b: bool) -> V {
    V::Typed(LitType::Enum(boolean_type()), Lit::Enum(b as usize))
}

fn is_boolean(t: &LitType) -> bool {
    let LitType::Enum(e) = t else {
        return false;
    };
    let named = |l: &EnumLit, n: &str| matches!(l, EnumLit::Ident(b) if b.eq_ignore_ascii_case(n.as_bytes()));
    matches!(&e.lits[..], [f, t] if named(f, "false") && named(t, "true"))
}

/// The bit-like types: enumerations whose literals include `'0'` and `'1'`.
fn char_pos(t: &EnumType, c: u8) -> Option<usize> {
    t.lits.iter().position(|l| *l == EnumLit::Char(c))
}

impl Cond {
    fn text(&self, e: &Expr) -> String {
        let (a, b) = (e.span.start as usize, e.span.end as usize);
        self.src.get(a..b).unwrap_or_default().to_string()
    }

    /// Reject what evaluation does not support, before the run.
    fn check(&self, e: &Expr) -> Result<(), String> {
        let no = |what: &str| Err(format!("{what} is not supported in conditions"));
        match &e.kind {
            ExprKind::Name(n) => self.check_name(n),
            ExprKind::Literal(l) if matches!(l.kind, LiteralKind::Null) => no("`null`"),
            ExprKind::Literal(_) => Ok(()),
            ExprKind::Paren(x) => self.check(x),
            ExprKind::Unary { op, operand } => match op {
                UnaryOp::Plus | UnaryOp::Minus | UnaryOp::Not | UnaryOp::Abs => self.check(operand),
                UnaryOp::Condition => self.check(operand),
                _ => no("a reduction operator"),
            },
            ExprKind::Binary { op, left, right } => match op {
                BinaryOp::Concat
                | BinaryOp::Pow
                | BinaryOp::Sll
                | BinaryOp::Srl
                | BinaryOp::Sla
                | BinaryOp::Sra
                | BinaryOp::Rol
                | BinaryOp::Ror
                | BinaryOp::MatchEq
                | BinaryOp::MatchNe
                | BinaryOp::MatchLt
                | BinaryOp::MatchLe
                | BinaryOp::MatchGt
                | BinaryOp::MatchGe => no(&format!("operator `{}`", op_text(*op))),
                _ => {
                    self.check(left)?;
                    self.check(right)
                }
            },
            _ => no(&format!("`{}`", self.text(e))),
        }
    }

    fn check_name(&self, n: &Name) -> Result<(), String> {
        match &n.kind {
            NameKind::Simple(_) | NameKind::Character(_) => Ok(()),
            NameKind::Selected {
                prefix,
                suffix: Suffix::Ident(_),
            } => self.check_name(prefix),
            NameKind::Apply { prefix, args } if args.len() == 1 => match &args[0].actual {
                Actual::Expr(x) if args[0].formal.is_none() => {
                    self.check_name(prefix)?;
                    self.check(x)
                }
                _ => Err("slices and named arguments are not supported in conditions".into()),
            },
            _ => Err(format!(
                "`{}` is not supported in conditions (a local, a signal, a field or an element)",
                self.src
                    .get(n.span.start as usize..n.span.end as usize)
                    .unwrap_or_default()
            )),
        }
    }

    /// Evaluate to a boolean; `bit` and `std_ulogic` results convert as `??` does.
    pub(crate) fn eval(&self, env: &dyn Env) -> Result<bool, String> {
        match self.value(&self.expr, env)? {
            V::Typed(t, Lit::Enum(p)) if is_boolean(&t) => Ok(p == 1),
            V::Typed(LitType::Enum(t), Lit::Enum(p))
                if char_pos(&t, b'1').is_some() && char_pos(&t, b'0').is_some() =>
            {
                Ok(Some(p) == char_pos(&t, b'1') || Some(p) == char_pos(&t, b'H'))
            }
            V::Text(t) => Err(format!("no local or signal `{t}` here")),
            _ => Err("the condition is not a boolean".into()),
        }
    }

    fn value(&self, e: &Expr, env: &dyn Env) -> Result<V, String> {
        match &e.kind {
            ExprKind::Paren(x) => self.value(x, env),
            ExprKind::Literal(_) => Ok(V::Text(self.text(e))),
            ExprKind::Name(n) => self.name(n, env),
            ExprKind::Unary { op, operand } => {
                let v = self.value(operand, env)?;
                unary(*op, v)
            }
            ExprKind::Binary { op, left, right } => {
                let (l, r) = (self.value(left, env)?, self.value(right, env)?);
                binary(*op, l, r)
            }
            _ => Err(format!("`{}` is not supported in conditions", self.text(e))),
        }
    }

    /// A local or signal, a field or element of one, or (unknown) its text for the other
    /// operand's type: an enumeration literal.
    fn name(&self, n: &Name, env: &dyn Env) -> Result<V, String> {
        let text = self
            .src
            .get(n.span.start as usize..n.span.end as usize)
            .unwrap_or_default()
            .to_string();
        if let Some((t, v)) = env.get(&text) {
            return Ok(V::Typed(t, v));
        }
        match &n.kind {
            NameKind::Selected {
                prefix,
                suffix: Suffix::Ident(f),
            } => match self.name(prefix, env)? {
                V::Typed(LitType::Record(fs), Lit::Record(xs)) => {
                    let k = fs
                        .iter()
                        .position(|(n, _)| n.eq_ignore_ascii_case(&f.text.to_string()))
                        .ok_or_else(|| format!("no field `{}` in `{text}`", f.text))?;
                    Ok(V::Typed(fs[k].1.clone(), xs[k].clone()))
                }
                // A path such as `tb.u1.s` the environment did not know.
                V::Text(_) => Ok(V::Text(text)),
                _ => Err(format!("`{text}`: not a record")),
            },
            NameKind::Apply { prefix, args } => {
                let Actual::Expr(x) = &args[0].actual else {
                    return Err(format!("`{text}` is not supported in conditions"));
                };
                let V::Typed(t, Lit::Array(xs)) = self.name(prefix, env)? else {
                    return Err(format!("`{text}`: not an array, local or signal here"));
                };
                let LitType::Array { len, elem, .. } = &t else {
                    return Err(format!("`{text}`: not an array"));
                };
                let i = match self.value(x, env)? {
                    V::Typed(_, Lit::Int(i)) => i,
                    V::Text(s) => match literal(s.as_bytes(), &LitType::Int(Bounds::I64)) {
                        Ok(Lit::Int(i)) => i,
                        _ => return Err(format!("`{s}` is not an index")),
                    },
                    _ => return Err(format!("`{text}`: only integer indices")),
                };
                let k = (0..*len)
                    .find(|&k| t.index(k) == Some(i))
                    .ok_or_else(|| format!("`{text}`: index {i} is outside the array"))?;
                Ok(V::Typed((**elem).clone(), xs[k as usize].clone()))
            }
            _ => Ok(V::Text(text)),
        }
    }
}

/// Both operands in one type: a literal is read in the type of the other.
fn unify(l: V, r: V) -> Result<(LitType, Lit, Lit), String> {
    let read = |t: &LitType, s: &str| literal(s.as_bytes(), t).map_err(|e| e.to_string());
    match (l, r) {
        (V::Typed(t, a), V::Typed(_, b)) => {
            if std::mem::discriminant(&a) != std::mem::discriminant(&b) {
                return Err("operands of different types".into());
            }
            Ok((t, a, b))
        }
        (V::Typed(t, a), V::Text(s)) => {
            let b = read(&t, &s)?;
            Ok((t, a, b))
        }
        (V::Text(s), V::Typed(t, b)) => {
            let a = read(&t, &s)?;
            Ok((t, a, b))
        }
        (V::Text(a), V::Text(b)) => {
            // Two literals: integers, else reals.
            for t in [
                LitType::Int(Bounds::I64),
                LitType::Real(yavs_std::types::RealBounds::REAL),
            ] {
                if let (Ok(x), Ok(y)) = (read(&t, &a), read(&t, &b)) {
                    return Ok((t, x, y));
                }
            }
            let num = |s: &str| read(&LitType::Real(yavs_std::types::RealBounds::REAL), s).is_ok();
            let name = if num(&a) { b } else { a };
            Err(format!("no local or signal `{name}` here"))
        }
    }
}

fn order(a: &Lit, b: &Lit) -> Result<Ordering, String> {
    Ok(match (a, b) {
        (Lit::Int(x), Lit::Int(y)) | (Lit::Phys(x), Lit::Phys(y)) => x.cmp(y),
        (Lit::Enum(x), Lit::Enum(y)) => x.cmp(y),
        (Lit::Real(x), Lit::Real(y)) => x.partial_cmp(y).ok_or("a real NaN")?,
        (Lit::Array(xs), Lit::Array(ys)) => {
            for (x, y) in xs.iter().zip(ys) {
                match order(x, y)? {
                    Ordering::Equal => {}
                    o => return Ok(o),
                }
            }
            xs.len().cmp(&ys.len())
        }
        _ => return Err("only scalars and arrays are ordered".into()),
    })
}

fn logic(op: BinaryOp, t: &LitType, a: &Lit, b: &Lit) -> Result<V, String> {
    let LitType::Enum(e) = t else {
        return Err(format!("`{}` needs boolean or bit operands", op_text(op)));
    };
    let (Lit::Enum(x), Lit::Enum(y)) = (a, b) else {
        return Err(format!("`{}` needs boolean or bit operands", op_text(op)));
    };
    // `true`/`'1'` is position 1 for boolean and bit alike.
    let bit = char_pos(e, b'0') == Some(0) && char_pos(e, b'1') == Some(1) && e.lits.len() == 2;
    if !is_boolean(t) && !bit {
        return Err(format!(
            "`{}` on this type is not supported in conditions",
            op_text(op)
        ));
    }
    let (x, y) = (*x == 1, *y == 1);
    let r = match op {
        BinaryOp::And => x && y,
        BinaryOp::Or => x || y,
        BinaryOp::Nand => !(x && y),
        BinaryOp::Nor => !(x || y),
        BinaryOp::Xor => x != y,
        _ => x == y,
    };
    Ok(V::Typed(t.clone(), Lit::Enum(r as usize)))
}

fn binary(op: BinaryOp, l: V, r: V) -> Result<V, String> {
    use BinaryOp as B;
    // Physical times integer, physical divided by physical: the operands differ in type.
    if matches!(op, B::Mul | B::Div)
        && let (V::Typed(LitType::Phys(..), _), _) | (_, V::Typed(LitType::Phys(..), _)) = (&l, &r)
    {
        return phys_scale(op, l, r);
    }
    let (t, a, b) = unify(l, r)?;
    let ov = || "overflow in the condition".to_string();
    Ok(match op {
        B::Eq => boolean(a == b),
        B::Ne => boolean(a != b),
        B::Lt => boolean(order(&a, &b)? == Ordering::Less),
        B::Le => boolean(order(&a, &b)? != Ordering::Greater),
        B::Gt => boolean(order(&a, &b)? == Ordering::Greater),
        B::Ge => boolean(order(&a, &b)? != Ordering::Less),
        B::And | B::Or | B::Nand | B::Nor | B::Xor | B::Xnor => logic(op, &t, &a, &b)?,
        _ => {
            let v = match (a, b) {
                (Lit::Int(x), Lit::Int(y)) => Lit::Int(int_op(op, x, y).ok_or_else(ov)?),
                (Lit::Phys(x), Lit::Phys(y)) if matches!(op, B::Add | B::Sub) => {
                    Lit::Phys(int_op(op, x, y).ok_or_else(ov)?)
                }
                (Lit::Real(x), Lit::Real(y)) => Lit::Real(match op {
                    B::Add => x + y,
                    B::Sub => x - y,
                    B::Mul => x * y,
                    B::Div => x / y,
                    _ => return Err(format!("`{}` on reals", op_text(op))),
                }),
                _ => return Err(format!("`{}` on these operands", op_text(op))),
            };
            V::Typed(t, v)
        }
    })
}

fn int_op(op: BinaryOp, x: i64, y: i64) -> Option<i64> {
    match op {
        BinaryOp::Add => x.checked_add(y),
        BinaryOp::Sub => x.checked_sub(y),
        BinaryOp::Mul => x.checked_mul(y),
        BinaryOp::Div => x.checked_div(y),
        BinaryOp::Rem => x.checked_rem(y),
        // VHDL `mod` takes the sign of the right operand.
        BinaryOp::Mod => x.checked_rem(y).map(|m| {
            if m != 0 && (m < 0) != (y < 0) {
                m + y
            } else {
                m
            }
        }),
        _ => None,
    }
}

/// `p * i`, `i * p`, `p / i`, `p / p`.
fn phys_scale(op: BinaryOp, l: V, r: V) -> Result<V, String> {
    let int = |v: V| -> Result<i64, String> {
        match v {
            V::Typed(_, Lit::Int(i)) => Ok(i),
            V::Text(s) => match literal(s.as_bytes(), &LitType::Int(Bounds::I64)) {
                Ok(Lit::Int(i)) => Ok(i),
                _ => Err(format!("`{s}` is not an integer")),
            },
            _ => Err("a physical value scales only by an integer".into()),
        }
    };
    let ov = || "overflow in the condition".to_string();
    // A physical literal divisor is read in the dividend's type.
    let r = match (&l, r) {
        (V::Typed(t @ LitType::Phys(..), _), V::Text(s)) if op == BinaryOp::Div => {
            match literal(s.as_bytes(), t) {
                Ok(y) => V::Typed(t.clone(), y),
                Err(_) => V::Text(s),
            }
        }
        (_, r) => r,
    };
    match (op, l, r) {
        (BinaryOp::Div, V::Typed(LitType::Phys(..), Lit::Phys(x)), V::Typed(_, Lit::Phys(y))) => {
            Ok(V::Typed(
                LitType::Int(Bounds::I64),
                Lit::Int(x.checked_div(y).ok_or_else(ov)?),
            ))
        }
        (_, V::Typed(t @ LitType::Phys(..), Lit::Phys(x)), r) => {
            let i = int(r)?;
            let v = if op == BinaryOp::Mul {
                x.checked_mul(i)
            } else {
                x.checked_div(i)
            };
            Ok(V::Typed(t, Lit::Phys(v.ok_or_else(ov)?)))
        }
        (BinaryOp::Mul, l, V::Typed(t @ LitType::Phys(..), Lit::Phys(x))) => {
            let i = int(l)?;
            Ok(V::Typed(t, Lit::Phys(x.checked_mul(i).ok_or_else(ov)?)))
        }
        _ => Err("this physical arithmetic is not supported in conditions".into()),
    }
}

fn unary(op: UnaryOp, v: V) -> Result<V, String> {
    let v = match v {
        // A lone literal: an integer, else a real.
        V::Text(s) => {
            let mut out = None;
            for t in [
                LitType::Int(Bounds::I64),
                LitType::Real(yavs_std::types::RealBounds::REAL),
            ] {
                if let Ok(x) = literal(s.as_bytes(), &t) {
                    out = Some((t, x));
                    break;
                }
            }
            let (t, x) = out.ok_or_else(|| format!("no local or signal `{s}` here"))?;
            V::Typed(t, x)
        }
        v => v,
    };
    let V::Typed(t, x) = v else {
        return Err("internal: untyped operand".into());
    };
    let ov = || "overflow in the condition".to_string();
    Ok(match (op, x) {
        (UnaryOp::Plus, x) => V::Typed(t, x),
        (UnaryOp::Minus, Lit::Int(i)) => V::Typed(t, Lit::Int(i.checked_neg().ok_or_else(ov)?)),
        (UnaryOp::Minus, Lit::Phys(i)) => V::Typed(t, Lit::Phys(i.checked_neg().ok_or_else(ov)?)),
        (UnaryOp::Minus, Lit::Real(r)) => V::Typed(t, Lit::Real(-r)),
        (UnaryOp::Abs, Lit::Int(i)) => V::Typed(t, Lit::Int(i.checked_abs().ok_or_else(ov)?)),
        (UnaryOp::Abs, Lit::Phys(i)) => V::Typed(t, Lit::Phys(i.checked_abs().ok_or_else(ov)?)),
        (UnaryOp::Abs, Lit::Real(r)) => V::Typed(t, Lit::Real(r.abs())),
        (UnaryOp::Not, x @ Lit::Enum(_)) => logic(BinaryOp::Xor, &t, &x, &Lit::Enum(1))?,
        (UnaryOp::Condition, Lit::Enum(p)) => match &t {
            LitType::Enum(e) => {
                boolean(Some(p) == char_pos(e, b'1') || Some(p) == char_pos(e, b'H'))
            }
            _ => return Err("`??` needs a bit or std_ulogic operand".into()),
        },
        _ => {
            let o = match op {
                UnaryOp::Plus => "+",
                UnaryOp::Minus => "-",
                UnaryOp::Abs => "abs",
                UnaryOp::Not => "not",
                _ => "??",
            };
            return Err(format!("`{o}` is not defined for this operand"));
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;

    struct M(HashMap<&'static str, (LitType, Lit)>);
    impl Env for M {
        fn get(&self, n: &str) -> Option<(LitType, Lit)> {
            self.0.get(n.to_ascii_lowercase().as_str()).cloned()
        }
    }

    #[test]
    fn conditions() {
        let logic = LitType::Enum(EnumType {
            lits: b"UX01ZWLH-".iter().map(|&c| EnumLit::Char(c)).collect(),
        });
        let vec = LitType::Array {
            left: 3,
            ascending: false,
            len: 4,
            elem: Box::new(logic.clone()),
        };
        let time = LitType::Phys(yavs_std::types::time_type(), Bounds::I64);
        let rec = LitType::Record(vec![("a".into(), LitType::Int(Bounds::I64))]);
        let env = M(HashMap::from([
            ("i", (LitType::Int(Bounds::I64), Lit::Int(6))),
            ("clk", (logic.clone(), Lit::Enum(3))),
            ("v", (vec.clone(), literal(b"\"10ZX\"", &vec).unwrap())),
            ("t", (time.clone(), Lit::Phys(5_000_000))),
            ("r", (rec, Lit::Record(vec![Lit::Int(2)]))),
            ("tb.u1.s", (LitType::Int(Bounds::I64), Lit::Int(1))),
        ]));
        let ok = |s: &str| parse(s).and_then(|c| c.eval(&env));
        assert_eq!(ok("i = 6"), Ok(true));
        assert_eq!(ok("i > 2 * 3 or i mod 4 = 2"), Ok(true));
        assert_eq!(ok("I /= 6 and clk = '1'"), Ok(false));
        assert_eq!(ok("clk"), Ok(true));
        assert_eq!(ok("not (clk = '0')"), Ok(true));
        assert_eq!(ok("v = \"10ZX\" and v(3) = '1' and v(1) = 'Z'"), Ok(true));
        assert_eq!(ok("v < x\"F\""), Ok(true));
        assert_eq!(
            ok("t >= 5 ns and t < 6 ns and t * 2 = 10 ns and t / 1 ns = 5"),
            Ok(true)
        );
        assert_eq!(ok("r.a + 1 = 3"), Ok(true));
        assert_eq!(ok("tb.u1.s = 1"), Ok(true));
        assert_eq!(ok("-i < abs (-7)"), Ok(true));
        assert!(ok("i =").unwrap_err().contains("does not parse"));
        assert!(ok("i ** 2 = 36").unwrap_err().contains("not supported"));
        assert!(ok("f(i, 2)").unwrap_err().contains("not supported"));
        assert_eq!(
            ok("1 = nosuch"),
            Err("no local or signal `nosuch` here".into())
        );
        assert_eq!(ok("i"), Err("the condition is not a boolean".into()));
        assert!(ok("v(9) = '1'").unwrap_err().contains("outside"));
        assert!(ok("i = '1'").is_err());
        assert!(ok("i / 0 = 1").unwrap_err().contains("overflow"));
    }
}
