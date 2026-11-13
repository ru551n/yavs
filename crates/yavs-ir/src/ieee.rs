//! Native replacements for hot `ieee` subprograms (`docs/decisions.md` "Native ieee operators").
//!
//! The vendored `ieee` packages stay ordinary VHDL: a call of a listed subprogram is lowered
//! to a native call instead of a call of its body when the subprogram's mangled name (library,
//! package, designator, parameter and result base types) is one that [`parse`] accepts.
//! Nothing is keyed by the designator alone, so a user subprogram of the same name never
//! matches. `yavs-interp` implements every [`Op`] that `parse` returns.

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

/// What an operator of the Synopsys packages computes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SynKind {
    Add,
    Sub,
    Mul,
    Cmp(Cmp),
}

/// An operand of such an operator: `unsigned` (or `std_logic_unsigned`'s vector), `signed` (or
/// `std_logic_signed`'s), an integer, or a `std_ulogic` (`+` and `-` only: `0 L` as 0, `1 H` as
/// 1 added to the vector's length, a metavalue the body's).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Opd {
    U,
    S,
    I,
    B,
}

impl Opd {
    /// A vector operand, not an integer or a `std_ulogic`.
    pub fn is_vec(self) -> bool {
        matches!(self, Opd::U | Opd::S)
    }
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
    /// `shl` (`right` false) and `shr` of an `unsigned` or `signed` by an `unsigned` count, also
    /// on `std_logic_vector` in `std_logic_unsigned` and `std_logic_signed`: the elements move
    /// unchanged, zeros come in (the sign element for a signed `shr`).
    Shift {
        right: bool,
        signed: bool,
    },
    /// `conv_integer` of an `unsigned` (or `std_logic_unsigned`'s vector) or a `signed`.
    ToInt {
        signed: bool,
    },
    /// An operator of `std_logic_arith`, `std_logic_unsigned` or `std_logic_signed` on two
    /// operands (at most one an integer or a `std_ulogic`): the whole body in one call. `keep`:
    /// the result has the vector operand's bounds (the `std_logic_vector` result with an integer
    /// or a `std_ulogic`), else `(n-1 downto 0)`.
    Oper {
        kind: SynKind,
        l: Opd,
        r: Opd,
        keep: bool,
    },
    /// Unary `-` and `abs` of a `signed` or `std_logic_signed`'s vector, and `+` (`neg` false:
    /// the operand as it is) of `std_logic_unsigned`'s and `std_logic_signed`'s: `-` is `0 - L`
    /// as `(n-1 downto 0)`, `abs` the operand as it is when its left element is `0` or `L`;
    /// `keep`: every result has the operand's bounds (`std_logic_unsigned`/`std_logic_signed`).
    Unary {
        neg: bool,
        abs: bool,
        keep: bool,
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
    /// `std_logic_1164` gate of a vector and a scalar (`left`: the scalar is the left operand):
    /// the gate of every element with the scalar, `(1 to n)`.
    VecScalarGate {
        g: Gate,
        left: bool,
    },
    /// `to_stdulogicvector` and `to_stdlogicvector` of a `bit_vector`: the same codes,
    /// `(n - 1 downto 0)`.
    ToSulv,
    Not,
    VecNot,
    /// A gate over all elements of one vector: the unary `std_logic_1164` and `numeric_std`
    /// operators and the `std_logic_misc` `*_reduce` functions.
    Reduce(Gate),
    /// Unary `-` of a `numeric_std` `signed`.
    Neg,
    /// `abs` of a `numeric_std` `signed`.
    Abs,
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
        form: Form,
    },
    /// `/` (0), `rem` (1) or `mod` (2) of two vectors, or of a vector and an integer (the
    /// one-word path takes the integer only when it fits the vector's length); a zero divisor
    /// and every integer form it does not take are handed back to the body.
    Div {
        kind: u8,
        signed: bool,
        form: Form,
    },
    Compare {
        cmp: Cmp,
        signed: bool,
        form: Form,
    },
    /// The matching relational operators `?=` `?/=` `?<` `?<=` `?>` `?>=` of `numeric_std`
    /// (a `std_ulogic`), with the reports of the bodies (null, `-` in the ordering ones, an
    /// integer that does not fit) made by the native.
    QCompare {
        cmp: Cmp,
        signed: bool,
        form: Form,
    },
    /// `maximum` / `minimum` of two vectors or a vector and an integer: a metavalue gives all
    /// `X`, a null array the null constant, an integer that does not fit its warning, all in the
    /// native.
    MinMax {
        max: bool,
        signed: bool,
        form: Form,
    },
    /// `std_match` of two vectors; a null array and unequal lengths warn in the native.
    Match,
    /// The predefined `?=` (`ne` false) and `?/=` of two `std_ulogic` or two `std_ulogic_vector`
    /// (`vec`; LRM 9.2.3, from [`implicit_native`]): the AND of the element matches, so a
    /// mismatch beats `U`, which beats `X`; unequal lengths are the native's error.
    MatchEq {
        ne: bool,
        vec: bool,
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
    // (A native symbol may carry `@n` after the key.)
    let key = key.split_once('@').map_or(key, |k| k.0);
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
        // (Before the gates, whose arm takes every name of this shape.)
        ("std_logic_1164", n @ ("?=" | "?/="), [a, b], "std_ulogic")
            if a == b && matches!(*a, "std_ulogic" | "std_ulogic_vector") =>
        {
            Some(Op::MatchEq {
                ne: n == "?/=",
                vec: *a == "std_ulogic_vector",
            })
        }
        ("std_logic_1164", n, ["std_ulogic", "std_ulogic"], "std_ulogic") => gate(n).map(Op::Gate),
        ("std_logic_1164", n, ["std_ulogic_vector", "std_ulogic_vector"], "std_ulogic_vector") => {
            gate(n).map(Op::VecGate)
        }
        ("std_logic_1164", n, ["std_ulogic_vector", "std_ulogic"], "std_ulogic_vector")
            if gate(n).is_some() =>
        {
            gate(n).map(|g| Op::VecScalarGate { g, left: false })
        }
        ("std_logic_1164", n, ["std_ulogic", "std_ulogic_vector"], "std_ulogic_vector")
            if gate(n).is_some() =>
        {
            gate(n).map(|g| Op::VecScalarGate { g, left: true })
        }
        (
            "std_logic_1164",
            "to_stdulogicvector" | "to_stdlogicvector",
            ["bit_vector"],
            "std_ulogic_vector",
        ) => Some(Op::ToSulv),
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
        // numeric_bit's bodies compute the same without the metavalue cases (BIT has none).
        ("numeric_std", "-", ["signed"], "signed") => Some(Op::Neg),
        ("numeric_std", "abs", ["signed"], "signed") => Some(Op::Abs),
        ("numeric_std", n @ ("+" | "-"), ps, r) => {
            let (signed, form, t) = num_form(ps)?;
            (r == t).then_some(Op::Add {
                sub: n == "-",
                signed,
                form,
            })
        }
        ("numeric_std", "*", ps, r) => {
            let (signed, form, t) = num_form(ps)?;
            (r == t).then_some(Op::Mul { signed, form })
        }
        ("numeric_std", n @ ("/" | "rem" | "mod"), ps, r) => {
            let (signed, form, t) = num_form(ps)?;
            (r == t).then_some(Op::Div {
                kind: ["/", "rem", "mod"].iter().position(|x| *x == n)? as u8,
                signed,
                form,
            })
        }
        ("numeric_std", n @ ("?>" | "?<" | "?<=" | "?>=" | "?=" | "?/="), ps, "std_ulogic")
            if !bit =>
        {
            let (signed, form, _) = num_form(ps)?;
            let cmp = match n {
                "?>" => Cmp::Gt,
                "?<" => Cmp::Lt,
                "?<=" => Cmp::Le,
                "?>=" => Cmp::Ge,
                "?=" => Cmp::Eq,
                _ => Cmp::Ne,
            };
            Some(Op::QCompare { cmp, signed, form })
        }
        // numeric_bit: two vectors only (with an integer, a truncation warning names the package).
        ("numeric_std", n @ ("maximum" | "minimum"), ps, r) => {
            let (signed, form, t) = num_form(ps)?;
            if bit && !matches!(form, Form::Vv) {
                return None;
            }
            (r == t).then_some(Op::MinMax {
                max: n == "maximum",
                signed,
                form,
            })
        }
        ("numeric_std", "std_match", [a, b], "boolean")
            if !bit && a == b && ["unsigned", "signed", "std_ulogic_vector"].contains(a) =>
        {
            Some(Op::Match)
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
        (
            "std_logic_arith",
            n @ ("ext" | "sxt"),
            ["std_ulogic_vector", "integer"],
            "std_ulogic_vector",
        ) => {
            // Zero or sign extension of the vector, as `conv_unsigned` of an unsigned or a signed.
            Some(Op::Syn(Syn::Conv {
                to_signed: false,
                from_signed: Some(n == "sxt"),
            }))
        }
        ("std_logic_arith", n @ ("shl" | "shr"), [a, "unsigned"], r)
            if *a == r && (r == "unsigned" || r == "signed") =>
        {
            Some(Op::Syn(Syn::Shift {
                right: n == "shr",
                signed: r == "signed",
            }))
        }
        (
            "std_logic_unsigned" | "std_logic_signed",
            n @ ("shl" | "shr"),
            ["std_ulogic_vector", "std_ulogic_vector"],
            "std_ulogic_vector",
        ) => Some(Op::Syn(Syn::Shift {
            right: n == "shr",
            signed: pkg == "std_logic_signed",
        })),
        ("std_logic_arith", "conv_integer", [t @ ("unsigned" | "signed")], "integer") => {
            Some(Op::Syn(Syn::ToInt {
                signed: *t == "signed",
            }))
        }
        (
            "std_logic_unsigned" | "std_logic_signed",
            "conv_integer",
            ["std_ulogic_vector"],
            "integer",
        ) => Some(Op::Syn(Syn::ToInt {
            signed: pkg == "std_logic_signed",
        })),
        (
            "std_logic_arith" | "std_logic_unsigned" | "std_logic_signed",
            n @ ("+" | "-" | "*" | "<" | "<=" | ">" | ">=" | "=" | "/="),
            [a, b],
            r,
        ) => syn_oper(pkg, n, a, b, r),
        // Not `+` of `std_logic_arith`: its body is `return L`, which costs less than a native
        // beyond 64 elements.
        ("std_logic_arith", n @ ("-" | "abs"), ["signed"], r)
            if r == "signed" || r == "std_ulogic_vector" =>
        {
            Some(Op::Syn(Syn::Unary {
                neg: true,
                abs: n == "abs",
                keep: false,
            }))
        }
        (
            p @ ("std_logic_unsigned" | "std_logic_signed"),
            n @ ("+" | "-" | "abs"),
            ["std_ulogic_vector"],
            "std_ulogic_vector",
        ) if n == "+" || p == "std_logic_signed" => Some(Op::Syn(Syn::Unary {
            neg: n != "+",
            abs: n == "abs",
            keep: true,
        })),
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

/// The operator `n` of a Synopsys package on operand types `a` and `b` and result type `r`.
fn syn_oper(pkg: &str, n: &str, a: &str, b: &str, r: &str) -> Option<Op> {
    let arith = pkg == "std_logic_arith";
    let opd = |t: &str| match (t, pkg) {
        ("integer", _) => Some(Opd::I),
        ("unsigned", "std_logic_arith") | ("std_ulogic_vector", "std_logic_unsigned") => {
            Some(Opd::U)
        }
        ("signed", "std_logic_arith") | ("std_ulogic_vector", "std_logic_signed") => Some(Opd::S),
        ("std_ulogic", _) => Some(Opd::B),
        _ => None,
    };
    let (l, rt) = (opd(a)?, opd(b)?);
    let bit = l == Opd::B || rt == Opd::B;
    if (!l.is_vec() && !rt.is_vec()) || (bit && !(n == "+" || n == "-")) {
        return None;
    }
    let kind = match n {
        "+" => SynKind::Add,
        "-" => SynKind::Sub,
        "*" => SynKind::Mul,
        _ => SynKind::Cmp(match n {
            "<" => Cmp::Lt,
            "<=" => Cmp::Le,
            ">" => Cmp::Gt,
            ">=" => Cmp::Ge,
            "=" => Cmp::Eq,
            _ => Cmp::Ne,
        }),
    };
    let int_form = !l.is_vec() || !rt.is_vec();
    if matches!(kind, SynKind::Cmp(_)) {
        return (r == "boolean").then_some(Op::Syn(Syn::Oper {
            kind,
            l,
            r: rt,
            keep: false,
        }));
    }
    // The result type of `std_logic_arith`: signed when an operand is, else unsigned; the
    // other packages return `std_logic_vector`.
    let want = if l == Opd::S || rt == Opd::S {
        "signed"
    } else {
        "unsigned"
    };
    let ok = r == "std_ulogic_vector" || (arith && r == want);
    (ok && !(kind == SynKind::Mul && int_form)).then_some(Op::Syn(Syn::Oper {
        kind,
        l,
        r: rt,
        keep: !arith && int_form,
    }))
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

/// The families of `yavs.implicit.` symbols [`implicit_native`] maps, with their operator.
pub const IMPLICIT_NATIVES: [(&str, &str); 2] = [("meq", "?="), ("mneq", "?/=")];

/// The native (`yavs.ieee.<key>`) of the predefined operation with this `yavs.implicit.` symbol,
/// if there is one: `?=` and `?/=` of `std_ulogic` and `std_ulogic_vector` ([`Op::MatchEq`]).
/// `std_logic_1164` declares no subprogram with these keys, so no body is replaced. Only keys
/// [`parse`] takes, so each is an [`Op`].
pub fn implicit_native(symbol: &str) -> Option<String> {
    let (fam, ty) = symbol.strip_prefix("yavs.implicit.")?.split_once('.')?;
    let (_, op) = IMPLICIT_NATIVES.iter().find(|(f, _)| *f == fam)?;
    let t = ty.strip_prefix("ieee.std_logic_1164.")?;
    let k = format!("std_logic_1164.\"{op}\"({t},{t})std_ulogic");
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
    fn predefined_matching() {
        let s = implicit_native("yavs.implicit.mneq.ieee.std_logic_1164.std_ulogic_vector");
        assert_eq!(
            s.as_deref().and_then(symbol_key).and_then(parse),
            Some(Op::MatchEq {
                ne: true,
                vec: true
            })
        );
        let s = implicit_native("yavs.implicit.meq.ieee.std_logic_1164.std_ulogic");
        assert_eq!(
            s.as_deref().and_then(symbol_key).and_then(parse),
            Some(Op::MatchEq {
                ne: false,
                vec: false
            })
        );
        // Ordering forms and other array types keep the implicit native.
        assert_eq!(
            implicit_native("yavs.implicit.mlt.ieee.std_logic_1164.std_ulogic"),
            None
        );
        assert_eq!(implicit_native("yavs.implicit.meq.work.p.my_vec"), None);
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
            (
                "std_logic_1164.\"?=\"(std_ulogic_vector,std_ulogic_vector)std_ulogic",
                8,
            ),
            ("std_logic_1164.\"?/=\"(std_ulogic,std_ulogic)std_ulogic", 2),
            ("math_real.uniform(integer,integer,real)", 3),
        ] {
            assert_eq!(arg_regs(k), n, "{k}");
        }
    }
}
