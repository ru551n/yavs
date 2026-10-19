//! Native replacements for hot `ieee` subprograms (`docs/decisions.md` "Native ieee operators").
//!
//! The vendored `ieee` packages stay ordinary VHDL: a call of a listed subprogram is lowered
//! to a native call instead of a call of its body when the subprogram's mangled name (library,
//! package, designator, parameter and result base types) is exactly one of [`SUPPORTED`].
//! Nothing is keyed by the designator alone, so a user subprogram of the same name never
//! matches. `yavs-interp` implements every key; its `ieee` tests check both lists agree.

/// The packages whose subprograms are listed.
const PACKAGES: [&str; 5] = [
    "ieee.math_real.",
    "ieee.std_logic_1164.",
    "ieee.numeric_std.",
    "ieee.numeric_bit.",
    "std.standard.",
];

/// A two-input gate of `std_logic_1164`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Gate {
    And,
    Nand,
    Or,
    Nor,
    Xor,
    Xnor,
}

/// A relational operator of `numeric_std`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Cmp {
    Gt,
    Lt,
    Le,
    Ge,
    Eq,
    Ne,
}

/// Operand shapes of the `numeric_std` arithmetic and relational operators.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Form {
    /// vector, vector
    Vv,
    /// vector, integer
    Vi,
    /// integer, vector
    Iv,
}

/// The five local shift and rotate functions of `numeric_std` (`XSLL` ...).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Shift {
    Sll,
    Srl,
    Sra,
    Rol,
    Ror,
}

/// The `math_real` functions that run on the platform's `f64` functions.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Math {
    Sqrt,
    Exp,
    Log,
    Log2,
    Log10,
    LogBase,
    Sin,
    Cos,
    PowRR,
    PowIR,
    Ceil,
    Floor,
    Round,
    Trunc,
    Sign,
}

/// A natively implemented `ieee` subprogram.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Op {
    /// `std_logic_1164` gate on two `std_ulogic`.
    Gate(Gate),
    /// `std_logic_1164` gate on two vectors, or a vector and a scalar (`Form::Vv` only: the
    /// scalar forms are separate keys).
    VecGate(Gate),
    Not,
    VecNot,
    /// `to_x01`, `to_x01z`, `to_ux01` (0, 1, 2) on a scalar or a vector.
    Conv(u8, bool),
    /// `to_01 (std_ulogic_vector, std_ulogic)`.
    To01,
    /// `"??" (std_ulogic)`.
    Truthy,
    /// `rising_edge` (true) or `falling_edge` of a `std_ulogic` signal.
    Edge(bool),
    Add {
        sub: bool,
        signed: bool,
        form: Form,
    },
    Mul {
        signed: bool,
    },
    Compare {
        cmp: Cmp,
        signed: bool,
        form: Form,
    },
    ToInteger {
        signed: bool,
    },
    ToUnsigned,
    ToSigned,
    Resize {
        signed: bool,
    },
    Shift(Shift),
    Math(Math),
}

/// Reduced key to operation. Every operation listed here is implemented by `yavs-interp`.
pub fn parse(key: &str) -> Option<Op> {
    let (pkg, rest) = key.split_once('.')?;
    let open = rest.find('(')?;
    let close = rest.rfind(')')?;
    let name = rest[..open].trim_matches('"');
    let params: Vec<&str> = rest[open + 1..close].split(',').collect();
    let ret = &rest[close + 1..];
    let gate = |n: &str| match n {
        "and" => Some(Gate::And),
        "nand" => Some(Gate::Nand),
        "or" => Some(Gate::Or),
        "nor" => Some(Gate::Nor),
        "xor" => Some(Gate::Xor),
        "xnor" => Some(Gate::Xnor),
        _ => None,
    };
    match (pkg, name, params.as_slice(), ret) {
        ("std_logic_1164", n, ["std_ulogic", "std_ulogic"], "std_ulogic") => gate(n).map(Op::Gate),
        ("std_logic_1164", n, ["std_ulogic_vector", "std_ulogic_vector"], "std_ulogic_vector") => {
            gate(n).map(Op::VecGate)
        }
        ("std_logic_1164", "not", ["std_ulogic"], "std_ulogic") => Some(Op::Not),
        ("std_logic_1164", "not", ["std_ulogic_vector"], "std_ulogic_vector") => Some(Op::VecNot),
        ("std_logic_1164", n @ ("to_x01" | "to_x01z" | "to_ux01"), [t], r)
            if t == &r && (r == "std_ulogic" || r == "std_ulogic_vector") =>
        {
            let k = ["to_x01", "to_x01z", "to_ux01"]
                .iter()
                .position(|x| *x == n)? as u8;
            Some(Op::Conv(k, r == "std_ulogic_vector"))
        }
        ("std_logic_1164", "to_01", ["std_ulogic_vector", "std_ulogic"], "std_ulogic_vector") => {
            Some(Op::To01)
        }
        ("std_logic_1164", "??", ["std_ulogic"], "boolean") => Some(Op::Truthy),
        ("std_logic_1164", n @ ("rising_edge" | "falling_edge"), ["std_ulogic"], "boolean") => {
            Some(Op::Edge(n == "rising_edge"))
        }
        ("numeric_std", n @ ("+" | "-"), ps, r) => {
            let (signed, form, t) = num_form(ps)?;
            (r == t).then_some(Op::Add {
                sub: n == "-",
                signed,
                form,
            })
        }
        ("numeric_std", "*", [a, b], r)
            if a == b && a == &r && (r == "unsigned" || r == "signed") =>
        {
            Some(Op::Mul {
                signed: r == "signed",
            })
        }
        ("numeric_std", n @ (">" | "<" | "<=" | ">=" | "=" | "/="), ps, "boolean") => {
            let (signed, form, _) = num_form(ps)?;
            let cmp = match n {
                ">" => Cmp::Gt,
                "<" => Cmp::Lt,
                "<=" => Cmp::Le,
                ">=" => Cmp::Ge,
                "=" => Cmp::Eq,
                _ => Cmp::Ne,
            };
            Some(Op::Compare { cmp, signed, form })
        }
        ("numeric_std", "to_integer", [t @ ("unsigned" | "signed")], "integer") => {
            Some(Op::ToInteger {
                signed: *t == "signed",
            })
        }
        ("numeric_std", "to_unsigned", ["integer", "integer"], "unsigned") => Some(Op::ToUnsigned),
        ("numeric_std", "to_signed", ["integer", "integer"], "signed") => Some(Op::ToSigned),
        ("numeric_std", "resize", [t @ ("unsigned" | "signed"), "integer"], r) if t == &r => {
            Some(Op::Resize {
                signed: r == "signed",
            })
        }
        (
            "numeric_std",
            n @ ("xsll" | "xsrl" | "xsra" | "xrol" | "xror"),
            ["std_ulogic_vector", "integer"],
            "std_ulogic_vector",
        ) => Some(Op::Shift(match n {
            "xsll" => Shift::Sll,
            "xsrl" => Shift::Srl,
            "xsra" => Shift::Sra,
            "xrol" => Shift::Rol,
            _ => Shift::Ror,
        })),
        ("math_real", n, ["real"], "real") => Some(Op::Math(match n {
            "sqrt" => Math::Sqrt,
            "exp" => Math::Exp,
            "log" => Math::Log,
            "log2" => Math::Log2,
            "log10" => Math::Log10,
            "sin" => Math::Sin,
            "cos" => Math::Cos,
            "ceil" => Math::Ceil,
            "floor" => Math::Floor,
            "round" => Math::Round,
            "trunc" => Math::Trunc,
            "sign" => Math::Sign,
            _ => return None,
        })),
        ("math_real", "log", ["real", "real"], "real") => Some(Op::Math(Math::LogBase)),
        ("math_real", "**", ["real", "real"], "real") => Some(Op::Math(Math::PowRR)),
        ("math_real", "**", ["integer", "real"], "real") => Some(Op::Math(Math::PowIR)),
        _ => None,
    }
}

/// `(vector, vector)`, `(vector, integer)` or `(integer, vector)` of one signedness.
fn num_form(ps: &[&str]) -> Option<(bool, Form, &'static str)> {
    let v = |t: &str| match t {
        "unsigned" => Some((false, "unsigned")),
        "signed" => Some((true, "signed")),
        _ => None,
    };
    match ps {
        [a, b] if a == b => v(a).map(|(s, t)| (s, Form::Vv, t)),
        [a, "integer"] => v(a).map(|(s, t)| (s, Form::Vi, t)),
        ["integer", b] => v(b).map(|(s, t)| (s, Form::Iv, t)),
        _ => None,
    }
}

/// The reduced key of a mangled `ieee` subprogram name, `None` when it is not one of an `ieee`
/// package or mentions a type outside the packages above.
fn key(mangled: &str) -> Option<String> {
    let rest = mangled.strip_prefix("ieee.")?;
    let (pkg, rest) = rest.split_once('.')?;
    let open = rest.find('(')?;
    let close = rest.rfind(')')?;
    let mut out = format!("{pkg}.{}(", &rest[..open]);
    for (i, t) in rest[open + 1..close].split(',').enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&short(t)?);
    }
    out.push(')');
    out.push_str(&short(&rest[close + 1..])?);
    Some(out)
}

fn short(ty: &str) -> Option<String> {
    let p = PACKAGES.iter().find(|p| ty.starts_with(**p))?;
    let name = &ty[p.len()..];
    Some(name.strip_prefix("unresolved_").unwrap_or(name).to_string())
}

/// The native symbol (`yavs.ieee.<key>`) that replaces the body of the subprogram with this
/// mangled name, if there is one.
pub fn native_symbol(mangled: &str) -> Option<String> {
    let k = key(mangled)?;
    parse(&k).map(|_| format!("yavs.ieee.{k}"))
}

/// The key of a symbol made by [`native_symbol`].
pub fn symbol_key(symbol: &str) -> Option<&str> {
    symbol.strip_prefix("yavs.ieee.")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keys() {
        assert_eq!(
            key("ieee.numeric_std.\"+\"(ieee.numeric_std.unresolved_unsigned,ieee.numeric_std.unresolved_unsigned)ieee.numeric_std.unresolved_unsigned").as_deref(),
            Some("numeric_std.\"+\"(unsigned,unsigned)unsigned")
        );
        assert_eq!(key("work.numeric_std.\"+\"(a)b"), None);
        assert_eq!(
            key("ieee.numeric_std.resize(ieee.numeric_std.unresolved_signed,std.standard.integer)ieee.numeric_std.unresolved_signed").as_deref(),
            Some("numeric_std.resize(signed,integer)signed")
        );
        // A type of a user package keeps the subprogram VHDL.
        assert_eq!(key("ieee.numeric_std.f(work.p.t)work.p.t"), None);
    }
}
