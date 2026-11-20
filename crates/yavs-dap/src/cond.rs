//! Expressions of the debugger (#326, #930, #935): breakpoint conditions, `examine` and the Watch
//! of a part or a computed value. A VHDL expression over the locals and signals a stop sees, parsed
//! by `yavs-syntax` and evaluated over the values the debugger shows (`Lit`).
//!
//! Predefined operators on scalars (and `=`, `/=` and ordering on arrays) are computed here; a
//! call of a function, a type conversion and an operator on a type that declares its own
//! (`numeric_std."+"`, `std_logic_1164."and"`) go to the environment (`Env::call`, #364), and so
//! do the attributes of signals (`Env::signal_attr`). The array attributes `'length`, `'left`,
//! `'right`, `'high`, `'low` and `'ascending` are read from the value.

use std::cmp::Ordering;

use yavs_std::types::{Bounds, EnumLit, EnumType, boolean_type};
use yavs_std::value::{Lit, LitType, literal};
use yavs_syntax::ast::{
    Actual, BinaryOp, Decl, Expr, ExprKind, Ident, LibraryUnit, Name, NameKind, Suffix, UnaryOp,
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
    /// What an access value or a protected object designates.
    fn deref(&self, _t: &LitType, _v: &Lit) -> Option<(LitType, Lit)> {
        None
    }
    /// The name of the type of signal `name`; `None` for a local: the IR does not keep it.
    fn type_name(&self, _name: &str) -> Option<String> {
        None
    }
    /// A name that has a value, but not one this environment knows (the check made when a
    /// condition is made, `check_types`): a local of a process, read at a stop. The environment
    /// notes the name.
    fn local(&self, _name: &str) -> bool {
        false
    }
    /// A call of the function, or the conversion to the type, `name` as written (an operator as
    /// `"+"`) with `args`. `None` when nothing of that name takes them here, so an operator keeps
    /// its predefined meaning.
    fn call(&self, _name: &str, _args: &[Arg<'_>]) -> Option<Result<Called, String>> {
        None
    }
    /// Attribute `attr` (lower case: `event`, `active`, `last_value`, `last_event`,
    /// `last_active`) of signal `name` or a part of one; `None` when `name` is no signal here.
    fn signal_attr(&self, _name: &str, _attr: &str) -> Option<Result<Called, String>> {
        None
    }
}

/// An argument of [`Env::call`]: a value with the name of its type where it is known, a literal
/// to read in the type of the parameter, or a value not known before the stop.
pub(crate) enum Arg<'a> {
    Value(&'a LitType, &'a Lit, Option<&'a str>),
    Literal(&'a str),
    Unknown,
}

/// What [`Env::call`] gives: a value and the name of its type; `None` before the stop.
pub(crate) type Called = Option<(LitType, Lit, Option<String>)>;

/// A value of type `t` to check types with.
pub(crate) fn zero(t: &LitType) -> Lit {
    match t {
        LitType::Enum(_) => Lit::Enum(0),
        LitType::Int(_) => Lit::Int(0),
        LitType::Real(_) => Lit::Real(0.0),
        LitType::Phys(..) => Lit::Phys(0),
        LitType::Array { len, elem, .. } => Lit::Array(vec![zero(elem); *len as usize]),
        LitType::Record(fs) => Lit::Record(fs.iter().map(|(_, t)| zero(t)).collect()),
        LitType::Handle { .. } => Lit::Handle(0),
    }
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
    /// An array of a bit-like type, with the name of its type if it is known.
    Bits(LitType, Lit, Option<String>),
    Text(String),
    /// A value that exists but is not known before the stop (`Env::local`): it fits any type.
    Unknown,
}

impl V {
    fn arg(&self) -> Arg<'_> {
        match self {
            V::Typed(t, x) => Arg::Value(t, x, None),
            V::Bits(t, x, n) => Arg::Value(t, x, n.as_deref()),
            V::Text(s) => Arg::Literal(s),
            V::Unknown => Arg::Unknown,
        }
    }

    fn called(c: Called) -> V {
        match c {
            None => V::Unknown,
            Some((t, x, n)) if is_bits(&t) => V::Bits(t, x, n),
            Some((t, x, _)) => V::Typed(t, x),
        }
    }

    /// A value of a type that may declare its own operators: not a number, a boolean or a bit.
    fn own_ops(&self) -> bool {
        match self {
            V::Bits(..) => true,
            V::Typed(t @ LitType::Enum(e), _) => {
                !is_boolean(t) && !(e.lits.len() == 2 && is_bit(e))
            }
            V::Typed(LitType::Record(_) | LitType::Array { .. }, _) => true,
            _ => false,
        }
    }

    /// As any other typed value.
    fn plain(self) -> V {
        match self {
            V::Bits(t, v, _) => V::Typed(t, v),
            v => v,
        }
    }
}

fn is_bits(t: &LitType) -> bool {
    matches!(t, LitType::Array { elem, .. }
        if matches!(&**elem, LitType::Enum(e) if char_pos(e, b'0').is_some() && char_pos(e, b'1').is_some()))
}

/// How a bit-like array reads as a number: `Some(signed)`; `None` when the type has no numbers
/// (`std_logic_vector`). A local's type is not known here, so an unknown name reads as unsigned
/// where both readings agree (`number`).
fn numeric(ty: &Option<String>) -> Option<Option<bool>> {
    let Some(ty) = ty else {
        return Some(None);
    };
    match ty.to_ascii_lowercase().as_str() {
        "unsigned" | "unresolved_unsigned" | "u_unsigned" => Some(Some(false)),
        "signed" | "unresolved_signed" | "u_signed" => Some(Some(true)),
        _ => None,
    }
}

/// The number a bit-like array holds, leftmost bit first; `None` with a metavalue in it. `signed` is
/// what `numeric` found.
fn number(t: &LitType, v: &Lit, signed: Option<bool>) -> Result<Option<i128>, String> {
    let (LitType::Array { elem, .. }, Lit::Array(xs)) = (t, v) else {
        return Err("this is a bug in yavs: not an array of bits".into());
    };
    let LitType::Enum(e) = &**elem else {
        return Err("this is a bug in yavs: not an array of bits".into());
    };
    let mut bits = Vec::with_capacity(xs.len());
    for x in xs {
        let Lit::Enum(p) = x else {
            return Ok(None);
        };
        let bit = match e.lits.get(*p) {
            Some(EnumLit::Char(b'0' | b'L')) => 0,
            Some(EnumLit::Char(b'1' | b'H')) => 1,
            _ => return Ok(None),
        };
        bits.push(bit);
    }
    let negative = bits.first() == Some(&1);
    if negative && signed.is_none() {
        return Err(
            "the type of this local is not known here, and its leftmost bit is 1, so it reads \
             differently as `signed` and as `unsigned`: compare it with a bit string"
                .into(),
        );
    }
    let negative = negative && signed == Some(true);
    let sign = i128::from(negative);
    let start = bits.iter().take_while(|&&b| b == sign).count();
    if bits.len() - start > 120 {
        return Err(
            "this array is too wide to read as a number here: compare it with a bit string".into(),
        );
    }
    let mut n = if negative { -1 } else { 0 };
    for &b in &bits[start..] {
        n = n * 2 + b;
    }
    Ok(Some(n))
}

fn compare(op: BinaryOp, o: Ordering) -> bool {
    use BinaryOp as B;
    match op {
        B::Eq => o == Ordering::Equal,
        B::Ne => o != Ordering::Equal,
        B::Lt => o == Ordering::Less,
        B::Le => o != Ordering::Greater,
        B::Gt => o == Ordering::Greater,
        _ => o != Ordering::Less,
    }
}

fn is_compare(op: BinaryOp) -> bool {
    use BinaryOp as B;
    matches!(op, B::Eq | B::Ne | B::Lt | B::Le | B::Gt | B::Ge)
}

/// `cnt = 3`, `cnt > n`, `a < b` on `unsigned` and `signed`, as `numeric_std` has them: a metavalue
/// makes only `/=` true. `None` when `l` and `r` are nothing of the kind.
fn numeric_op(op: BinaryOp, l: &V, r: &V) -> Option<Result<V, String>> {
    let int = |v: &V| match v {
        V::Text(s) => match literal(s.as_bytes(), &LitType::Int(Bounds::I64)) {
            Ok(Lit::Int(i)) => Some(i),
            _ => None,
        },
        V::Typed(_, Lit::Int(i)) => Some(*i),
        _ => None,
    };
    let (a, b): (Result<Option<i128>, String>, Result<Option<i128>, String>) = match (l, r) {
        (V::Bits(t1, v1, n1), V::Bits(t2, v2, n2)) => {
            let (Some(Some(s1)), Some(Some(s2))) = (numeric(n1), numeric(n2)) else {
                return None;
            };
            if s1 != s2 {
                return None;
            }
            (number(t1, v1, Some(s1)), number(t2, v2, Some(s2)))
        }
        (V::Bits(t, v, n), other) | (other, V::Bits(t, v, n)) => {
            let i = int(other)?;
            let Some(sign) = numeric(n) else {
                let name = n.as_deref().unwrap_or("this array").to_ascii_lowercase();
                return Some(Err(format!(
                    "{i} is not a value of `{name}`, which has no numbers: compare it with a bit \
                     string such as \"0101\""
                )));
            };
            let x = number(t, v, sign);
            if matches!(l, V::Bits(..)) {
                (x, Ok(Some(i128::from(i))))
            } else {
                (Ok(Some(i128::from(i))), x)
            }
        }
        _ => return None,
    };
    if !is_compare(op) {
        return Some(Err(format!(
            "`{}` on an array whose type is not known here (a field, or a variable of a \
             subprogram): convert it, as in unsigned(x) {} 1, or write to_integer(x) {} ...",
            op_text(op),
            op_text(op),
            op_text(op)
        )));
    }
    Some(match (a, b) {
        (Err(e), _) | (_, Err(e)) => Err(e),
        (Ok(Some(a)), Ok(Some(b))) => Ok(boolean(compare(op, a.cmp(&b)))),
        _ => Ok(boolean(op == BinaryOp::Ne)),
    })
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

/// An error that depends on the values of the moment, not on names and types.
fn by_value(e: &str) -> bool {
    e == "overflow in the condition"
        || e.ends_with("is outside the array")
        || e.ends_with("the access value is null")
}

/// The truth of a result: a boolean, or a `bit` or `std_ulogic` as `??` converts it.
fn as_bool(v: V) -> Result<bool, String> {
    match v {
        V::Typed(t, Lit::Enum(p)) if is_boolean(&t) => Ok(p == 1),
        V::Typed(LitType::Enum(t), Lit::Enum(p))
            if char_pos(&t, b'1').is_some() && char_pos(&t, b'0').is_some() =>
        {
            Ok(Some(p) == char_pos(&t, b'1') || Some(p) == char_pos(&t, b'H'))
        }
        V::Text(t) if is_identifier(&t) => Err(format!("no local or signal `{t}` here")),
        _ => Err("the condition is not a boolean".into()),
    }
}

fn is_identifier(s: &str) -> bool {
    s.starts_with(|c: char| c.is_ascii_alphabetic())
        && s.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_bit(e: &EnumType) -> bool {
    char_pos(e, b'0') == Some(0) && char_pos(e, b'1') == Some(1)
}

/// The bit-like types: enumerations whose literals include `'0'` and `'1'`.
fn char_pos(t: &EnumType, c: u8) -> Option<usize> {
    t.lits.iter().position(|l| *l == EnumLit::Char(c))
}

impl Cond {
    /// The names a condition reads (prefixes of fields and elements included, which do no harm).
    pub(crate) fn names(&self) -> Vec<String> {
        fn expr(c: &Cond, e: &Expr, out: &mut Vec<String>) {
            match &e.kind {
                ExprKind::Name(n) => {
                    out.push(
                        c.src
                            .get(n.span.start as usize..n.span.end as usize)
                            .unwrap_or_default()
                            .to_string(),
                    );
                    name(c, n, out);
                }
                ExprKind::Paren(x) => expr(c, x, out),
                ExprKind::Unary { operand, .. } => expr(c, operand, out),
                ExprKind::Binary { left, right, .. } => {
                    expr(c, left, out);
                    expr(c, right, out);
                }
                _ => {}
            }
        }
        // The names inside a name: arguments of a call, prefixes of attributes.
        fn name(c: &Cond, n: &Name, out: &mut Vec<String>) {
            match &n.kind {
                NameKind::Apply { prefix, args } => {
                    name(c, prefix, out);
                    for a in args {
                        if let Actual::Expr(x) = &a.actual {
                            expr(c, x, out);
                        }
                    }
                }
                NameKind::Attribute { prefix, .. } => {
                    out.push(c.text_of(prefix));
                    name(c, prefix, out);
                }
                _ => {}
            }
        }
        let mut out = Vec::new();
        expr(self, &self.expr, &mut out);
        out
    }

    fn text_of(&self, n: &Name) -> String {
        let (a, b) = (n.span.start as usize, n.span.end as usize);
        self.src.get(a..b).unwrap_or_default().to_string()
    }

    fn text(&self, e: &Expr) -> String {
        let (a, b) = (e.span.start as usize, e.span.end as usize);
        self.src.get(a..b).unwrap_or_default().to_string()
    }

    /// Reject what evaluation does not support, before the run.
    fn check(&self, e: &Expr) -> Result<(), String> {
        let no = |what: &str| Err(format!("{what} is not supported in conditions"));
        match &e.kind {
            ExprKind::Name(n) => self.check_name(n),
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
            NameKind::Simple(_) | NameKind::Character(_) | NameKind::Operator(_) => Ok(()),
            NameKind::Attribute { prefix, attr } => {
                let a = attr.text.to_string().to_ascii_lowercase();
                if !SIGNAL_ATTRS.contains(&a.as_str()) && !ARRAY_ATTRS.contains(&a.as_str()) {
                    return Err(format!(
                        "attribute `'{}` is not supported in conditions (only {})",
                        attr.text,
                        SIGNAL_ATTRS
                            .iter()
                            .chain(&ARRAY_ATTRS)
                            .map(|a| format!("'{a}"))
                            .collect::<Vec<_>>()
                            .join(", ")
                    ));
                }
                self.check_name(prefix)
            }
            NameKind::Selected {
                prefix,
                suffix: Suffix::Ident(_),
            } => self.check_name(prefix),
            NameKind::Selected {
                prefix,
                suffix: Suffix::All(_),
            } => self.check_name(prefix),
            NameKind::Apply { prefix, args } => {
                if matches!(&prefix.kind, NameKind::Attribute { .. }) {
                    self.check_name(prefix)?;
                    return Err(format!(
                        "`{}`: attributes with an argument are not supported in conditions",
                        self.text_of(n)
                    ));
                }
                self.check_name(prefix)?;
                for a in args {
                    match &a.actual {
                        Actual::Expr(x) if a.formal.is_none() => self.check(x)?,
                        Actual::Range(_) if a.formal.is_none() && args.len() == 1 => {}
                        _ => return Err("named arguments are not supported in conditions".into()),
                    }
                }
                Ok(())
            }
            _ => Err(format!(
                "`{}` is not supported in conditions (a local, a signal, a field or an element)",
                self.src
                    .get(n.span.start as usize..n.span.end as usize)
                    .unwrap_or_default()
            )),
        }
    }

    /// Evaluate to a value. The error is `None` when the expression only names what is not here.
    pub(crate) fn evaluate(&self, env: &dyn Env) -> Result<(LitType, Lit), Option<String>> {
        match self.value(&self.expr, env).map_err(Some)?.plain() {
            V::Typed(t, v) => Ok((t, v)),
            V::Text(s) => {
                let int = LitType::Int(Bounds::I64);
                let real = LitType::Real(yavs_std::types::RealBounds::REAL);
                [int, real]
                    .into_iter()
                    .find_map(|t| literal(s.as_bytes(), &t).ok().map(|v| (t, v)))
                    .ok_or(None)
            }
            V::Bits(..) => Err(Some("this is a bug in yavs: bits after `plain`".into())),
            V::Unknown => Err(None),
        }
    }

    /// What can be known when the condition is made: the evaluation over the types of the
    /// signals with a value of each, and what the environment calls `local` as a value that fits
    /// any type. The errors are those of the evaluation (an unknown name, operands of different
    /// types, a result that is not boolean); what depends on a value (a division by zero, an
    /// index, a null access) waits for the stop.
    pub(crate) fn check_types(&self, env: &dyn Env) -> Result<(), String> {
        let v = match self.value(&self.expr, env) {
            Ok(v) => v.plain(),
            Err(e) if by_value(&e) => return Ok(()),
            Err(e) => return Err(e),
        };
        match v {
            V::Unknown => Ok(()),
            v => as_bool(v).map(|_| ()),
        }
    }

    /// Evaluate to a boolean; `bit` and `std_ulogic` results convert as `??` does.
    pub(crate) fn eval(&self, env: &dyn Env) -> Result<bool, String> {
        as_bool(self.value(&self.expr, env)?.plain())
    }

    fn value(&self, e: &Expr, env: &dyn Env) -> Result<V, String> {
        match &e.kind {
            ExprKind::Paren(x) => self.value(x, env),
            ExprKind::Literal(_) => Ok(V::Text(self.text(e))),
            ExprKind::Name(n) => match self.name(n, env)? {
                // A word that names no object: a function without parameters, else a literal.
                V::Text(w) if matches!(n.kind, NameKind::Simple(_)) => match env.call(&w, &[]) {
                    Some(c) => c.map(V::called),
                    None => Ok(V::Text(w)),
                },
                v => Ok(v),
            },
            ExprKind::Unary { op, operand } => {
                let v = self.value(operand, env)?;
                let sym = match op {
                    UnaryOp::Not => Some("not"),
                    UnaryOp::Minus => Some("-"),
                    UnaryOp::Plus => Some("+"),
                    UnaryOp::Abs => Some("abs"),
                    _ => None,
                };
                if let Some(sym) = sym.filter(|_| v.own_ops())
                    && let Some(c) = env.call(&format!("\"{sym}\""), &[v.arg()])
                {
                    return c.map(V::called);
                }
                unary(*op, v.plain())
            }
            ExprKind::Binary { op, left, right } => {
                let (l, r) = (self.value(left, env)?, self.value(right, env)?);
                if (l.own_ops() || r.own_ops())
                    && let Some(c) = env.call(&format!("\"{}\"", op_text(*op)), &[l.arg(), r.arg()])
                {
                    return c.map(V::called);
                }
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
            return Ok(if is_bits(&t) {
                V::Bits(t, v, env.type_name(&text))
            } else {
                V::Typed(t, v)
            });
        }
        if env.local(&text) {
            return Ok(V::Unknown);
        }
        // Through an access value, as VHDL's `p.x` and `p(1)` go.
        let through = |v: V| match v.plain() {
            V::Typed(t @ LitType::Handle { .. }, x) => env
                .deref(&t, &x)
                .map(|(t, x)| V::Typed(t, x))
                .ok_or_else(|| format!("`{text}`: the access value is null")),
            v => Ok(v),
        };
        match &n.kind {
            NameKind::Selected {
                prefix,
                suffix: Suffix::All(_),
            } => match self.name(prefix, env)?.plain() {
                V::Unknown => Ok(V::Unknown),
                V::Text(_) => Ok(V::Text(text)),
                v @ V::Typed(LitType::Handle { .. }, _) => through(v),
                _ => Err(format!("`{text}`: not an access value")),
            },
            NameKind::Selected {
                prefix,
                suffix: Suffix::Ident(f),
            } => match through(self.name(prefix, env)?)? {
                V::Typed(LitType::Record(fs), Lit::Record(xs)) => {
                    let k = fs
                        .iter()
                        .position(|(n, _)| n.eq_ignore_ascii_case(&f.text.to_string()))
                        .ok_or_else(|| {
                            let cands = fs.iter().map(|(n, _)| n.clone()).collect::<Vec<_>>();
                            let help =
                                crate::console::similar(&f.text.to_string(), "field", &cands);
                            let help = help.map_or(String::new(), |h| format!(": {h}"));
                            format!("no field `{}` in `{text}`{help}", f.text)
                        })?;
                    let (t, x) = (fs[k].1.clone(), xs[k].clone());
                    Ok(if is_bits(&t) {
                        V::Bits(t, x, None)
                    } else {
                        V::Typed(t, x)
                    })
                }
                // A path such as `tb.u1.s` the environment did not know.
                V::Text(_) => Ok(V::Text(text)),
                V::Unknown => Ok(V::Unknown),
                _ => Err(format!("`{text}`: not a record")),
            },
            NameKind::Attribute { prefix, attr } => self.attribute(&text, prefix, attr, env),
            NameKind::Apply { prefix, args } => {
                if matches!(&args[0].actual, Actual::Range(_)) {
                    // A slice has the type of its prefix.
                    let whole = self.name(prefix, env)?;
                    let ty = match &whole {
                        V::Bits(_, _, n) => n.clone(),
                        _ => None,
                    };
                    let inner = through(whole)?;
                    if let V::Unknown = inner {
                        return Ok(V::Unknown);
                    }
                    let V::Typed(t, x) = inner else {
                        return Err(format!("`{text}`: not an array, local or signal here"));
                    };
                    let p = self
                        .src
                        .get(prefix.span.start as usize..prefix.span.end as usize);
                    let suffix = text[p.unwrap_or_default().len()..].to_string();
                    let (t, x) = crate::part_of(&t, &x, &suffix)?;
                    return Ok(match is_bits(&t) {
                        true => V::Bits(t, x, ty),
                        false => V::Typed(t, x),
                    });
                }
                let prefix_value = self.name(prefix, env)?;
                // A name that is no object here: a function or a type conversion.
                if let V::Text(f) = &prefix_value {
                    let mut vals = Vec::new();
                    for a in args {
                        let Actual::Expr(x) = &a.actual else {
                            return Err(format!("`{text}` is not supported in conditions"));
                        };
                        vals.push(self.value(x, env)?);
                    }
                    let call: Vec<Arg<'_>> = vals.iter().map(V::arg).collect();
                    if let Some(c) = env.call(f, &call) {
                        return c.map(V::called);
                    }
                    if args.len() > 1 {
                        return Err(format!("no function `{f}` here"));
                    }
                }
                let Actual::Expr(x) = &args[0].actual else {
                    return Err(format!("`{text}` is not supported in conditions"));
                };
                if let V::Text(f) = &prefix_value
                    && f.eq_ignore_ascii_case("to_integer")
                {
                    return match self.value(x, env)? {
                        V::Bits(t, v, ty) => {
                            let sign = numeric(&ty).ok_or_else(|| {
                                format!(
                                    "`{text}`: `{}` has no numbers",
                                    ty.as_deref().unwrap_or("")
                                )
                            })?;
                            // A metavalue reads as 0, with the warning `numeric_std` has at run time.
                            let i = number(&t, &v, sign)?.unwrap_or(0);
                            let i =
                                i64::try_from(i).map_err(|_| format!("`{text}` is too large"))?;
                            Ok(V::Typed(LitType::Int(Bounds::I64), Lit::Int(i)))
                        }
                        V::Unknown => Ok(V::Unknown),
                        _ => Err(format!("`{text}`: not an `unsigned` or `signed`")),
                    };
                }
                let element = through(prefix_value)?;
                if let V::Unknown = element {
                    return Ok(V::Unknown);
                }
                let V::Typed(t, Lit::Array(xs)) = element else {
                    return Err(format!("`{text}`: not an array, local or signal here"));
                };
                let LitType::Array { len, elem, .. } = &t else {
                    return Err(format!("`{text}`: not an array"));
                };
                let i = match self.value(x, env)?.plain() {
                    V::Typed(_, Lit::Int(i)) => i,
                    V::Text(s) => match literal(s.as_bytes(), &LitType::Int(Bounds::I64)) {
                        Ok(Lit::Int(i)) => i,
                        _ => return Err(format!("`{s}` is not an index")),
                    },
                    V::Unknown => return Ok(V::Unknown),
                    _ => return Err(format!("`{text}`: only integer indices")),
                };
                let k = (0..*len)
                    .find(|&k| t.index(k) == Some(i))
                    .ok_or_else(|| format!("`{text}`: index {i} is outside the array"))?;
                let (et, ex) = ((**elem).clone(), xs[k as usize].clone());
                Ok(if is_bits(&et) {
                    V::Bits(et, ex, None)
                } else {
                    V::Typed(et, ex)
                })
            }
            _ => Ok(V::Text(text)),
        }
    }
}

/// The attributes of signals, read by the environment.
const SIGNAL_ATTRS: [&str; 5] = ["event", "active", "last_value", "last_event", "last_active"];
/// The attributes of arrays, read from the value.
const ARRAY_ATTRS: [&str; 6] = ["length", "left", "right", "high", "low", "ascending"];

impl Cond {
    /// `prefix'attr`, with `text` the whole name.
    fn attribute(
        &self,
        text: &str,
        prefix: &Name,
        attr: &Ident,
        env: &dyn Env,
    ) -> Result<V, String> {
        let a = attr.text.to_string().to_ascii_lowercase();
        let p = self.text_of(prefix);
        if SIGNAL_ATTRS.contains(&a.as_str()) {
            return match env.signal_attr(&p, &a) {
                Some(r) => r.map(V::called),
                None if env.local(&p) => Ok(V::Unknown),
                None => Err(format!("`{text}`: `{p}` is not a signal here")),
            };
        }
        let LitType::Array {
            left,
            ascending,
            len,
            ..
        } = (match self.name(prefix, env)?.plain() {
            V::Unknown => return Ok(V::Unknown),
            V::Typed(LitType::Handle { .. }, _) => {
                return Err(format!("`{text}`: write `{p}.all'{a}`"));
            }
            V::Typed(t, _) => t,
            V::Text(_) => {
                return Err(format!(
                    "`{text}`: no local or signal `{p}` here (attributes of types are not \
                     supported in conditions)"
                ));
            }
            _ => return Err("this is a bug in yavs: bits after `plain`".into()),
        })
        else {
            return Err(format!("`{text}`: `{p}` is not an array"));
        };
        let ov = || "overflow in the condition".to_string();
        // ponytail: integer indices only; an enumeration index reads as its position.
        let last = i64::try_from(len)
            .map_err(|_| ov())?
            .checked_sub(1)
            .ok_or_else(ov)?;
        let right = if ascending {
            left.checked_add(last)
        } else {
            left.checked_sub(last)
        }
        .ok_or_else(ov)?;
        let (low, high) = if ascending {
            (left, right)
        } else {
            (right, left)
        };
        let int = |i: i64| V::Typed(LitType::Int(Bounds::I64), Lit::Int(i));
        Ok(match a.as_str() {
            "length" => int(i64::try_from(len).map_err(|_| ov())?),
            "left" => int(left),
            "right" => int(right),
            "high" => int(high),
            "low" => int(low),
            _ => boolean(ascending),
        })
    }
}

/// Both operands in one type: a literal is read in the type of the other.
fn unify(l: V, r: V) -> Result<(LitType, Lit, Lit), String> {
    // A word that reads as no literal is a name the environment does not know.
    let read = |t: &LitType, s: &str| {
        crate::design::typed_literal(s, t).map_err(|e| match is_identifier(s) {
            true => format!("no local or signal `{s}` here"),
            false => format!("`{s}` is not a value of the other operand's type: {e}"),
        })
    };
    match (l.plain(), r.plain()) {
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
        _ => Err("this is a bug in yavs: bits after `plain`".into()),
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
    // Before the stop: a comparison is a boolean whatever its operands hold.
    if matches!(l, V::Unknown) || matches!(r, V::Unknown) {
        return Ok(if is_compare(op) {
            boolean(false)
        } else {
            V::Unknown
        });
    }
    if let Some(v) = numeric_op(op, &l, &r) {
        return v;
    }
    let (l, r) = (l.plain(), r.plain());
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
        // The most negative value rem or mod -1 is 0, not an overflow.
        BinaryOp::Rem => (y != 0).then(|| x.wrapping_rem(y)),
        // VHDL `mod` takes the sign of the right operand.
        BinaryOp::Mod => (y != 0).then(|| x.wrapping_rem(y)).map(|m| {
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
        V::Unknown => return Ok(V::Unknown),
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
            ("m", (LitType::Int(Bounds::I64), Lit::Int(i64::MIN))),
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
        assert_eq!(ok("f(i, 2)"), Err("no function `f` here".into()));
        assert_eq!(
            ok("1 = nosuch"),
            Err("no local or signal `nosuch` here".into())
        );
        assert_eq!(ok("i"), Err("the condition is not a boolean".into()));
        assert!(ok("v(9) = '1'").unwrap_err().contains("outside"));
        assert!(ok("i = '1'").is_err());
        assert!(ok("i / 0 = 1").unwrap_err().contains("overflow"));
        assert_eq!(ok("m rem (-1) = 0 and m mod (-1) = 0"), Ok(true));
        assert_eq!(ok("m rem 3 = -2 and m mod 3 = 1"), Ok(true));
        assert!(ok("m / (-1) = 0").unwrap_err().contains("overflow"));
        assert!(ok("i rem 0 = 0").unwrap_err().contains("overflow"));
        assert!(ok("i mod 0 = 0").unwrap_err().contains("overflow"));
        // Array attributes (#364).
        assert_eq!(
            ok("v'length = 4 and v'left = 3 and v'right = 0 and v'high = 3 and v'low = 0"),
            Ok(true)
        );
        assert_eq!(ok("not v'ascending and v'length * 2 = 8"), Ok(true));
        assert!(ok("i'length = 1").unwrap_err().contains("not an array"));
        assert!(
            ok("v'image = 1")
                .unwrap_err()
                .contains("'image` is not supported")
        );
        assert!(
            ok("v'length(1) = 4")
                .unwrap_err()
                .contains("with an argument")
        );
        assert!(ok("clk'event").unwrap_err().contains("not a signal here"));
    }

    /// `M` with a local `loc`, whose value waits for the stop.
    struct Before(M);
    impl Env for Before {
        fn get(&self, n: &str) -> Option<(LitType, Lit)> {
            self.0.get(n)
        }
        fn local(&self, n: &str) -> bool {
            n == "loc"
        }
    }

    #[test]
    fn conditions_are_checked_before_the_stop() {
        let logic = LitType::Enum(EnumType {
            lits: b"UX01ZWLH-".iter().map(|&c| EnumLit::Char(c)).collect(),
        });
        let env = Before(M(HashMap::from([
            ("i", (LitType::Int(Bounds::I64), Lit::Int(0))),
            ("clk", (logic, Lit::Enum(0))),
        ])));
        let check = |s: &str| parse(s).and_then(|c| c.check_types(&env));
        assert_eq!(check("i = 6 and clk = '1'"), Ok(()));
        assert_eq!(check("loc > 3 and i = 1"), Ok(()));
        assert_eq!(check("loc"), Ok(()));
        // A value of the moment is no error before it.
        assert_eq!(check("i / i = 1"), Ok(()));
        assert_eq!(
            check("nosuch = 1"),
            Err("no local or signal `nosuch` here".into())
        );
        assert_eq!(
            check("i = nosuch"),
            Err("no local or signal `nosuch` here".into())
        );
        assert_eq!(check("i"), Err("the condition is not a boolean".into()));
        assert_eq!(
            check("loc = 1 and i"),
            Err("operands of different types".into())
        );
        assert!(check("clk = 7").is_err());
        assert!(check("i = '1'").is_err());
    }
}
