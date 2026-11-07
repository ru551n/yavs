//! The VHDL view of a Verilog module: what `entity lib.module` in VHDL takes its generics and ports
//! from. Analysis records it (parameters at their default values, so ports sized by a parameter
//! have the width the defaults give) as text in the module's library record, and VHDL analysis
//! reads that text without the Verilog front end.
//!
//! One line per item: `param NAME int 8`, `param NAME real 1.5`, `param NAME str HEX`,
//! `param NAME vec 4 01xz` (most significant bit first; an integer parameter whose default
//! names other parameters ends with ` = EXPR`), `port NAME in|out|inout scalar`,
//! `port NAME in|out|inout MSB LSB` (followed by ` ~ EXPR ~ EXPR` when the bounds are
//! expressions of integer parameters, or ` ~ ! WHY` when they depend on parameters in a way
//! that is not expressible; see [`IfExpr`]), or the single line `error MESSAGE`.

/// The default value of a parameter, which also gives the type of its VHDL generic.
#[derive(Clone, Debug, PartialEq)]
pub enum IfValue {
    /// A 32-bit signed value: `integer`.
    Int(i64),
    Real(f64),
    /// A string: `string`.
    Str(Vec<u8>),
    /// A vector (also of one bit): `std_logic_vector`, bits `0 1 x z`, most significant first.
    Vec(String),
}

#[derive(Clone, Debug, PartialEq)]
pub struct IfParam {
    pub name: String,
    pub value: IfValue,
    /// An integer parameter whose default is an expression of other parameters: what it is
    /// while the instance leaves it out.
    pub derived: Option<IfExpr>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IfDir {
    In,
    Out,
    Inout,
}

/// The binary operators of [`IfExpr`], as Verilog writes them.
pub const IF_OPS: [&str; 16] = [
    "+", "-", "*", "/", "%", "**", "<<", ">>", "<", "<=", ">", ">=", "==", "!=", "&&", "||",
];

/// An integer constant expression of parameters, in the subset the VHDL view can follow.
#[derive(Clone, Debug, PartialEq)]
pub enum IfExpr {
    Num(i64),
    Param(String),
    Neg(Box<IfExpr>),
    /// `!`
    Not(Box<IfExpr>),
    /// `$clog2`
    Clog2(Box<IfExpr>),
    /// One of [`IF_OPS`].
    Bin(&'static str, Box<IfExpr>, Box<IfExpr>),
    /// `c ? a : b`
    Cond(Box<IfExpr>, Box<IfExpr>, Box<IfExpr>),
}

impl IfExpr {
    /// Whether a parameter occurs in it.
    pub fn has_param(&self) -> bool {
        match self {
            IfExpr::Num(_) => false,
            IfExpr::Param(_) => true,
            IfExpr::Neg(a) | IfExpr::Not(a) | IfExpr::Clog2(a) => a.has_param(),
            IfExpr::Bin(_, a, b) => a.has_param() || b.has_param(),
            IfExpr::Cond(c, a, b) => c.has_param() || a.has_param() || b.has_param(),
        }
    }

    /// The expression as Verilog writes it, for messages.
    pub fn verilog(&self) -> String {
        match self {
            IfExpr::Num(n) => n.to_string(),
            IfExpr::Param(p) => p.clone(),
            IfExpr::Neg(a) => format!("-{}", a.verilog()),
            IfExpr::Not(a) => format!("!{}", a.verilog()),
            IfExpr::Clog2(a) => format!("$clog2({})", a.verilog()),
            IfExpr::Bin(op, a, b) => format!("({} {op} {})", a.verilog(), b.verilog()),
            IfExpr::Cond(c, a, b) => {
                format!("({} ? {} : {})", c.verilog(), a.verilog(), b.verilog())
            }
        }
    }

    fn encode(&self, o: &mut String) {
        let un = |o: &mut String, tag: &str, a: &IfExpr| {
            o.push_str(&format!("({tag} "));
            a.encode(o);
            o.push(')');
        };
        match self {
            IfExpr::Num(n) => o.push_str(&n.to_string()),
            IfExpr::Param(p) => o.push_str(p),
            IfExpr::Neg(a) => un(o, "neg", a),
            IfExpr::Not(a) => un(o, "not", a),
            IfExpr::Clog2(a) => un(o, "clog2", a),
            IfExpr::Bin(op, a, b) => {
                o.push_str(&format!("({op} "));
                a.encode(o);
                o.push(' ');
                b.encode(o);
                o.push(')');
            }
            IfExpr::Cond(c, a, b) => {
                o.push_str("(? ");
                c.encode(o);
                o.push(' ');
                a.encode(o);
                o.push(' ');
                b.encode(o);
                o.push(')');
            }
        }
    }

    /// The expression at the start of `t` and the rest of `t`.
    fn decode(t: &str) -> Option<(IfExpr, &str)> {
        let Some(inner) = t.strip_prefix('(') else {
            let end = t.find([' ', ')']).unwrap_or(t.len());
            let (w, rest) = t.split_at(end);
            return match w.parse() {
                Ok(n) => Some((IfExpr::Num(n), rest)),
                Err(_) if !w.is_empty() => Some((IfExpr::Param(w.to_string()), rest)),
                Err(_) => None,
            };
        };
        let (tag, args) = inner.split_once(' ')?;
        let one = |mk: fn(Box<IfExpr>) -> IfExpr| -> Option<(IfExpr, &str)> {
            let (a, rest) = IfExpr::decode(args)?;
            Some((mk(Box::new(a)), rest.strip_prefix(')')?))
        };
        match tag {
            "neg" => return one(IfExpr::Neg),
            "not" => return one(IfExpr::Not),
            "clog2" => return one(IfExpr::Clog2),
            "?" => {
                let (c, rest) = IfExpr::decode(args)?;
                let (a, rest) = IfExpr::decode(rest.strip_prefix(' ')?)?;
                let (b, rest) = IfExpr::decode(rest.strip_prefix(' ')?)?;
                return Some((
                    IfExpr::Cond(Box::new(c), Box::new(a), Box::new(b)),
                    rest.strip_prefix(')')?,
                ));
            }
            _ => {}
        }
        let op = IF_OPS.iter().find(|o| **o == tag)?;
        let (a, rest) = IfExpr::decode(args)?;
        let (b, rest) = IfExpr::decode(rest.strip_prefix(' ')?)?;
        Some((
            IfExpr::Bin(op, Box::new(a), Box::new(b)),
            rest.strip_prefix(')')?,
        ))
    }
}

/// How the bounds of a vector port follow the parameters.
#[derive(Clone, Debug, PartialEq)]
pub enum IfBounds {
    /// They do not (or the instance cannot change them): the numbers.
    Fixed,
    /// They are these expressions of the integer parameters.
    Exprs(IfExpr, IfExpr),
    /// They depend on parameters in a way the view cannot follow; why.
    Opaque(String),
}

/// The range of a vector port.
#[derive(Clone, Debug, PartialEq)]
pub struct IfRange {
    /// The bounds with the parameters at their defaults.
    pub msb: i64,
    pub lsb: i64,
    pub bounds: IfBounds,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IfPort {
    pub name: String,
    pub dir: IfDir,
    /// `None` for a scalar.
    pub range: Option<IfRange>,
}

impl IfPort {
    /// The declaration as Verilog writes it, for messages.
    pub fn verilog(&self) -> String {
        let dir = match self.dir {
            IfDir::In => "input",
            IfDir::Out => "output",
            IfDir::Inout => "inout",
        };
        match &self.range {
            None => format!("{dir} {}", self.name),
            Some(r) => match &r.bounds {
                IfBounds::Exprs(m, l) => {
                    format!("{dir} [{}:{}] {}", m.verilog(), l.verilog(), self.name)
                }
                _ => format!("{dir} [{}:{}] {}", r.msb, r.lsb, self.name),
            },
        }
    }
}

impl IfParam {
    /// The declaration as Verilog writes it, for messages.
    pub fn verilog(&self) -> String {
        let v = match &self.value {
            IfValue::Int(i) => i.to_string(),
            IfValue::Real(r) => format!("{r:?}"),
            IfValue::Str(s) => format!("\"{}\"", String::from_utf8_lossy(s)),
            IfValue::Vec(b) => format!("{}'b{b}", b.len()),
        };
        match &self.derived {
            Some(e) => format!("parameter {} = {}", self.name, e.verilog()),
            None => format!("parameter {} = {v}", self.name),
        }
    }
}

/// A module's parameters and ports in declaration order, or why it has no VHDL view.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Interface {
    pub params: Vec<IfParam>,
    pub ports: Vec<IfPort>,
    pub error: Option<String>,
}

impl Interface {
    pub fn failed(why: impl Into<String>) -> Interface {
        Interface {
            error: Some(why.into()),
            ..Interface::default()
        }
    }

    pub fn encode(&self) -> String {
        if let Some(e) = &self.error {
            return format!("error {}\n", e.replace('\n', " "));
        }
        let mut o = String::new();
        for p in &self.params {
            let v = match &p.value {
                IfValue::Int(i) => format!("int {i}"),
                IfValue::Real(r) => format!("real {r:?}"),
                IfValue::Str(s) => {
                    format!(
                        "str {}",
                        s.iter().map(|b| format!("{b:02x}")).collect::<String>()
                    )
                }
                IfValue::Vec(b) => format!("vec {} {b}", b.len()),
            };
            o.push_str(&format!("param {} {v}", p.name));
            if let Some(e) = &p.derived {
                o.push_str(" = ");
                e.encode(&mut o);
            }
            o.push('\n');
        }
        for p in &self.ports {
            let dir = match p.dir {
                IfDir::In => "in",
                IfDir::Out => "out",
                IfDir::Inout => "inout",
            };
            match &p.range {
                None => o.push_str(&format!("port {} {dir} scalar\n", p.name)),
                Some(r) => {
                    o.push_str(&format!("port {} {dir} {} {}", p.name, r.msb, r.lsb));
                    match &r.bounds {
                        IfBounds::Fixed => {}
                        IfBounds::Exprs(m, l) => {
                            o.push_str(" ~ ");
                            m.encode(&mut o);
                            o.push_str(" ~ ");
                            l.encode(&mut o);
                        }
                        IfBounds::Opaque(why) => {
                            o.push_str(&format!(" ~ ! {}", why.replace('\n', " ")));
                        }
                    }
                    o.push('\n');
                }
            }
        }
        o
    }

    /// The interface `encode` wrote; `None` when the text is not one.
    pub fn decode(text: &str) -> Option<Interface> {
        if let Some(e) = text.strip_prefix("error ") {
            return Some(Interface::failed(e.trim_end()));
        }
        let mut out = Interface::default();
        for line in text.lines() {
            if let Some(rest) = line.strip_prefix("param ") {
                let (head, derived) = match rest.split_once(" = ") {
                    Some((h, e)) => {
                        let (e, tail) = IfExpr::decode(e)?;
                        tail.is_empty().then_some(())?;
                        (h, Some(e))
                    }
                    None => (rest, None),
                };
                let w: Vec<&str> = head.split(' ').collect();
                let value = match w.as_slice() {
                    [_, "int", v] => IfValue::Int(v.parse().ok()?),
                    [_, "real", v] => IfValue::Real(v.parse().ok()?),
                    [_, "str", hex] => IfValue::Str(
                        (0..hex.len() / 2)
                            .map(|i| u8::from_str_radix(hex.get(2 * i..2 * i + 2)?, 16).ok())
                            .collect::<Option<Vec<u8>>>()?,
                    ),
                    [_, "vec", _, bits] => IfValue::Vec(bits.to_string()),
                    _ => return None,
                };
                out.params.push(IfParam {
                    name: w[0].to_string(),
                    value,
                    derived,
                });
                continue;
            }
            let (line, bounds) = match line.split_once(" ~ ") {
                Some((l, e)) => (l, Some(e)),
                None => (line, None),
            };
            let w: Vec<&str> = line.split(' ').collect();
            let ["port", name, dir, range @ ..] = w.as_slice() else {
                return None;
            };
            let dir = match *dir {
                "in" => IfDir::In,
                "out" => IfDir::Out,
                "inout" => IfDir::Inout,
                _ => return None,
            };
            let range = match range {
                ["scalar"] => None,
                [m, l] => Some(IfRange {
                    msb: m.parse().ok()?,
                    lsb: l.parse().ok()?,
                    bounds: match bounds {
                        None => IfBounds::Fixed,
                        Some(why) if why.starts_with("! ") => {
                            IfBounds::Opaque(why[2..].to_string())
                        }
                        Some(e) => {
                            let (m, rest) = IfExpr::decode(e)?;
                            let (l, rest) = IfExpr::decode(rest.strip_prefix(" ~ ")?)?;
                            rest.is_empty().then_some(())?;
                            IfBounds::Exprs(m, l)
                        }
                    },
                }),
                _ => return None,
            };
            out.ports.push(IfPort {
                name: name.to_string(),
                dir,
                range,
            });
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn b(e: IfExpr) -> Box<IfExpr> {
        Box::new(e)
    }

    #[test]
    fn text_round_trips() {
        let w = || IfExpr::Param("W".into());
        let i = Interface {
            params: vec![
                IfParam {
                    name: "W".into(),
                    value: IfValue::Int(-8),
                    derived: None,
                },
                IfParam {
                    name: "D".into(),
                    value: IfValue::Int(16),
                    derived: Some(IfExpr::Bin("*", b(w()), b(IfExpr::Num(2)))),
                },
                IfParam {
                    name: "R".into(),
                    value: IfValue::Real(1.5),
                    derived: None,
                },
                IfParam {
                    name: "S".into(),
                    value: IfValue::Str(b"TRUE".to_vec()),
                    derived: None,
                },
                IfParam {
                    name: "V".into(),
                    value: IfValue::Vec("01xz".into()),
                    derived: None,
                },
            ],
            ports: vec![
                IfPort {
                    name: "a".into(),
                    dir: IfDir::In,
                    range: None,
                },
                IfPort {
                    name: "q".into(),
                    dir: IfDir::Out,
                    range: Some(IfRange {
                        msb: 0,
                        lsb: 7,
                        bounds: IfBounds::Fixed,
                    }),
                },
                IfPort {
                    name: "io".into(),
                    dir: IfDir::Inout,
                    range: Some(IfRange {
                        msb: 7,
                        lsb: 0,
                        bounds: IfBounds::Exprs(
                            IfExpr::Cond(
                                b(IfExpr::Bin("<", b(w()), b(IfExpr::Num(4)))),
                                b(IfExpr::Clog2(b(w()))),
                                b(IfExpr::Neg(b(IfExpr::Not(b(IfExpr::Num(1)))))),
                            ),
                            IfExpr::Bin("**", b(w()), b(IfExpr::Num(2))),
                        ),
                    }),
                },
                IfPort {
                    name: "z".into(),
                    dir: IfDir::Out,
                    range: Some(IfRange {
                        msb: 3,
                        lsb: 0,
                        bounds: IfBounds::Opaque("the operator `&`".into()),
                    }),
                },
            ],
            error: None,
        };
        assert_eq!(Interface::decode(&i.encode()), Some(i));
        let e = Interface::failed("not representable");
        assert_eq!(Interface::decode(&e.encode()), Some(e));
        assert_eq!(Interface::decode("port a sideways scalar\n"), None);
    }

    #[test]
    fn declarations_read_as_verilog() {
        let p = IfPort {
            name: "q".into(),
            dir: IfDir::Out,
            range: Some(IfRange {
                msb: 7,
                lsb: 0,
                bounds: IfBounds::Exprs(
                    IfExpr::Bin("-", b(IfExpr::Param("W".into())), b(IfExpr::Num(1))),
                    IfExpr::Num(0),
                ),
            }),
        };
        assert_eq!(p.verilog(), "output [(W - 1):0] q");
    }
}
