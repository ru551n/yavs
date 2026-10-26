//! Native replacements for hot `ieee` subprograms (`docs/decisions.md` "Native ieee operators").
//!
//! The vendored `ieee` packages stay ordinary VHDL: a call of a listed subprogram is lowered
//! to a native call instead of a call of its body when the subprogram's mangled name (library,
//! package, designator, parameter and result base types) is exactly one of [`SUPPORTED`].
//! Nothing is keyed by the designator alone, so a user subprogram of the same name never
//! matches. `yavs-interp` implements every key; its `ieee` tests check both lists agree.

/// The packages whose subprograms are listed.
const PACKAGES: [&str; 7] = [
    "ieee.std_logic_arith.",
    "ieee.std_logic_misc.",
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

/// The helpers of the vendored Synopsys `std_logic_arith`, which hold the cost of its operators.
/// Each handles operands of `0 1 L H` only and hands the call back to the VHDL body for
/// anything else (a metavalue, an unusual range): the body then makes the results and warnings.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Syn {
    /// `plus`, `minus` (signed) and `unsigned_plus`, `unsigned_minus`: operands of one length.
    Add {
        sub: bool,
        signed: bool,
    },
    /// `mult`.
    Mult {
        signed: bool,
    },
    MakeBinary,
    /// `is_less`, `is_less_or_equal` (signed) and `unsigned_is_less`, `unsigned_is_less_or_equal`:
    /// `(n-1 downto 0)` operands of one length with `0`/`1` elements only.
    Less {
        or_equal: bool,
        signed: bool,
    },
    /// `conv_unsigned` / `conv_signed` of a vector (`from_signed`) or an integer (`None`).
    Conv {
        to_signed: bool,
        from_signed: Option<bool>,
    },
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
    /// A gate over all elements of one vector: the unary `std_logic_1164` and `numeric_std`
    /// operators and the `std_logic_misc` `*_reduce` functions.
    Reduce(Gate),
    /// `numeric_std` gate on two `unsigned` or two `signed`; unequal lengths are handed back to
    /// the body (its failure report).
    NumGate(Gate),
    /// `numeric_std` `not` of an `unsigned` or a `signed`.
    NumNot,
    /// `to_x01`, `to_x01z`, `to_ux01` (0, 1, 2) on a scalar or a vector.
    Conv(u8, bool),
    /// `to_01 (std_ulogic_vector, std_ulogic)`.
    To01,
    /// `"??" (std_ulogic)`.
    Truthy,
    /// `is_x` of a `std_ulogic_vector` (true) or a `std_ulogic`.
    IsX(bool),
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
    /// `/` (0), `rem` (1) or `mod` (2) of two vectors; a zero divisor is handed back to the
    /// body (its error report and what follows).
    Div {
        kind: u8,
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
    /// `shift_left`, `shift_right` (`Srl` for unsigned, `Sra` for signed), `rotate_left`,
    /// `rotate_right` of an `unsigned` or `signed`.
    ShiftFn(Shift),
    Math(Math),
    Syn(Syn),
    /// `math_real.uniform (seed1, seed2, x)`: the only native procedure.
    Uniform,
    /// `to_bitvector (std_ulogic_vector, bit)`: `0 L` to `'0'`, `1 H` to `'1'`, the rest to the
    /// second argument.
    ToBitVector,
    /// `to_hstring` (false) or `to_ostring` (true) of a `std_ulogic_vector`; a null vector is
    /// handed back to the body.
    Str(bool),
}

/// Reduced key to operation. Every operation listed here is implemented by `yavs-interp`.
pub fn parse(key: &str) -> Option<Op> {
    let (pkg, rest) = key.split_once('.')?;
    let open = rest.find('(')?;
    let close = rest.rfind(')')?;
    let name = rest[..open].trim_matches('"');
    // `numeric_bit` has the shapes of `numeric_std` on `bit_vector`; the native gets a
    // type argument to tell them apart (the interpreter converts the bit arrays).
    let bit = pkg == "numeric_bit";
    let pkg = if bit { "numeric_std" } else { pkg };
    // In 1993 `std_logic_vector` is a type of its own, not a subtype of `std_ulogic_vector`;
    // both are arrays of `std_ulogic` values, so one native serves both.
    let unify = |t| {
        if (bit && t == "bit_vector") || t == "std_logic_vector" {
            "std_ulogic_vector"
        } else {
            t
        }
    };
    let params: Vec<&str> = rest[open + 1..close].split(',').map(unify).collect();
    let ret = unify(&rest[close + 1..]);
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
        ("std_logic_1164", n, ["std_ulogic_vector"], "std_ulogic") => gate(n).map(Op::Reduce),
        ("std_logic_misc", n, ["std_ulogic_vector"], "std_ulogic") => {
            gate(n.strip_suffix("_reduce")?).map(Op::Reduce)
        }
        ("numeric_std", n, ["unsigned" | "signed"], "std_ulogic") if !bit => {
            gate(n).map(Op::Reduce)
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
        ("std_logic_1164", "to_bitvector", ["std_ulogic_vector", "bit"], "bit_vector") => {
            Some(Op::ToBitVector)
        }
        ("std_logic_1164", "??", ["std_ulogic"], "boolean") => Some(Op::Truthy),
        ("std_logic_1164", "is_x", [t @ ("std_ulogic" | "std_ulogic_vector")], "boolean") => {
            Some(Op::IsX(*t == "std_ulogic_vector"))
        }
        ("std_logic_1164", n @ ("rising_edge" | "falling_edge"), ["std_ulogic"], "boolean") => {
            Some(Op::Edge(n == "rising_edge"))
        }
        ("numeric_std", n, [a, b], r)
            if !bit
                && a == b
                && a == &r
                && (r == "unsigned" || r == "signed")
                && gate(n).is_some() =>
        {
            gate(n).map(Op::NumGate)
        }
        ("numeric_std", "not", [a], r) if !bit && a == &r && (r == "unsigned" || r == "signed") => {
            Some(Op::NumNot)
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
        ("numeric_std", n @ ("/" | "rem" | "mod"), [a, b], r)
            if a == b && a == &r && (r == "unsigned" || r == "signed") =>
        {
            Some(Op::Div {
                kind: ["/", "rem", "mod"].iter().position(|x| *x == n)? as u8,
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
        (
            "numeric_std",
            n @ ("shift_left" | "shift_right" | "rotate_left" | "rotate_right"),
            [t @ ("unsigned" | "signed"), "integer"],
            r,
        ) if t == &r => Some(Op::ShiftFn(match (n, r == "signed") {
            ("shift_left", _) => Shift::Sll,
            ("shift_right", false) => Shift::Srl,
            ("shift_right", true) => Shift::Sra,
            ("rotate_left", _) => Shift::Rol,
            _ => Shift::Ror,
        })),
        (
            "std_logic_arith",
            n @ ("plus" | "minus" | "unsigned_plus" | "unsigned_minus"),
            [a, b],
            r,
        ) if a == b && a == &r => {
            let signed = match (n.starts_with("unsigned"), r) {
                (false, "signed") => true,
                (true, "unsigned") => false,
                _ => return None,
            };
            Some(Op::Syn(Syn::Add {
                sub: n.ends_with("minus"),
                signed,
            }))
        }
        ("std_logic_arith", "mult", [a, b], r)
            if a == b && a == &r && (r == "unsigned" || r == "signed") =>
        {
            Some(Op::Syn(Syn::Mult {
                signed: r == "signed",
            }))
        }
        ("std_logic_arith", "make_binary", [a], r)
            if ["unsigned", "signed", "std_ulogic_vector"].contains(a)
                && ["unsigned", "signed", "std_ulogic_vector"].contains(&r) =>
        {
            // Same elements and bounds whatever the result type.
            Some(Op::Syn(Syn::MakeBinary))
        }
        (
            "std_logic_arith",
            n @ ("is_less" | "is_less_or_equal" | "unsigned_is_less" | "unsigned_is_less_or_equal"),
            [a, b],
            "boolean",
        ) if a == b
            && *a
                == if n.starts_with("unsigned") {
                    "unsigned"
                } else {
                    "signed"
                } =>
        {
            Some(Op::Syn(Syn::Less {
                or_equal: n.ends_with("equal"),
                signed: !n.starts_with("unsigned"),
            }))
        }
        ("std_logic_arith", n @ ("conv_unsigned" | "conv_signed"), [a, "integer"], r)
            if r == &n[5..] && ["unsigned", "signed", "integer"].contains(a) =>
        {
            Some(Op::Syn(Syn::Conv {
                to_signed: r == "signed",
                from_signed: match *a {
                    "unsigned" => Some(false),
                    "signed" => Some(true),
                    _ => None,
                },
            }))
        }
        ("std_logic_arith", "conv_std_logic_vector", [a, "integer"], "std_ulogic_vector")
            if ["unsigned", "signed", "integer"].contains(a) =>
        {
            // The same elements as `conv_signed` of an integer or `conv_unsigned`/`conv_signed`.
            Some(Op::Syn(Syn::Conv {
                to_signed: true,
                from_signed: match *a {
                    "unsigned" => Some(false),
                    "signed" => Some(true),
                    _ => None,
                },
            }))
        }
        ("math_real", "uniform", ["integer", "integer", "real"], "") => Some(Op::Uniform),
        ("std_logic_1164", n @ ("to_hstring" | "to_ostring"), ["std_ulogic_vector"], "string") => {
            Some(Op::Str(n == "to_ostring"))
        }
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
    // A procedure has no result.
    let ret = &rest[close + 1..];
    if !ret.is_empty() {
        out.push_str(&short(ret)?);
    }
    Some(out)
}

fn short(ty: &str) -> Option<String> {
    let p = PACKAGES.iter().find(|p| ty.starts_with(**p))?;
    let name = &ty[p.len()..];
    Some(name.strip_prefix("unresolved_").unwrap_or(name).to_string())
}

/// The parameter types of the subprogram with this reduced key (a `@<function id>` suffix of a
/// symbol's key is ignored).
fn param_types(key: &str) -> impl Iterator<Item = &str> {
    let key = key.split_once('@').map_or(key, |k| k.0);
    key.split_once('(')
        .and_then(|(_, r)| r.split_once(')'))
        .map_or("", |(p, _)| p)
        .split(',')
        .filter(|t| !t.is_empty())
}

/// Whether the parameter `i` of the subprogram with this reduced key is an array (passed to a
/// native as `data, left, right, dir`, to the body as data and descriptor).
pub fn array_param(key: &str, i: usize) -> bool {
    param_types(key).nth(i).is_some_and(|t| {
        [
            "unsigned",
            "signed",
            "std_ulogic_vector",
            "std_logic_vector",
            "bit_vector",
        ]
        .contains(&t)
    })
}

/// The registers of an array argument of a native `ieee` call, as values: the address of the
/// first plane block, `left`, `right` and the direction (`0` to, `1` downto; the descriptor's
/// cells). No fat pair or descriptor is built for the call (#697).
pub const ARRAY_REGS: usize = 4;

/// Argument registers of a `CallNative` of the native with this reduced key: one per scalar
/// parameter and [`ARRAY_REGS`] per array parameter (the verifier checks it).
pub fn arg_regs(key: &str) -> usize {
    (0..param_types(key).count())
        .map(|i| if array_param(key, i) { ARRAY_REGS } else { 1 })
        .sum()
}

/// Whether the native of `op` may hand the call back to the VHDL body.
pub fn has_fallback(op: Op) -> bool {
    matches!(
        op,
        Op::Syn(_) | Op::Str(_) | Op::Div { .. } | Op::NumGate(_)
    )
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

    #[test]
    fn argument_registers() {
        // Four registers per array, one per scalar; the 1993 `std_logic_vector` is an array too.
        for (k, n) in [
            ("numeric_std.\"+\"(unsigned,unsigned)unsigned", 8),
            ("numeric_std.\"+\"(unsigned,integer)unsigned", 5),
            ("numeric_std.\"+\"(integer,signed)signed", 5),
            ("numeric_std.to_unsigned(integer,integer)unsigned", 2),
            (
                "std_logic_1164.\"and\"(std_logic_vector,std_logic_vector)std_logic_vector@7",
                8,
            ),
            (
                "std_logic_1164.to_01(std_ulogic_vector,std_ulogic)std_ulogic_vector",
                5,
            ),
            ("std_logic_1164.rising_edge(std_ulogic)boolean", 1),
            ("math_real.uniform(integer,integer,real)", 3),
        ] {
            assert_eq!(arg_regs(k), n, "{k}");
        }
    }
}
