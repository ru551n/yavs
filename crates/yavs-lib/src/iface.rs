//! The VHDL view of a Verilog module: what `entity lib.module` in VHDL takes its generics and ports
//! from. Analysis records it (parameters at their default values, so ports sized by a parameter
//! have the width the defaults give) as text in the module's library record, and VHDL analysis
//! reads that text without the Verilog front end.
//!
//! One line per item: `param NAME int 8`, `param NAME real 1.5`, `param NAME str HEX`,
//! `param NAME vec 4 01xz` (most significant bit first), `port NAME in|out|inout scalar`,
//! `port NAME in|out|inout MSB LSB` (with ` ~ EXPR ~ EXPR` after it when the bounds are
//! expressions of integer parameters, see [`IfExpr`]), or the single line `error MESSAGE`.

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
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IfDir {
    In,
    Out,
    Inout,
}

/// A bound of a port range as an expression of integer parameters, in the subset VHDL has too.
#[derive(Clone, Debug, PartialEq)]
pub enum IfExpr {
    Num(i64),
    Param(String),
    Neg(Box<IfExpr>),
    /// `+ - * / % **` (`%` is VHDL `rem`).
    Bin(char, Box<IfExpr>, Box<IfExpr>),
}

impl IfExpr {
    /// Whether a parameter occurs in it.
    pub fn has_param(&self) -> bool {
        match self {
            IfExpr::Num(_) => false,
            IfExpr::Param(_) => true,
            IfExpr::Neg(a) => a.has_param(),
            IfExpr::Bin(_, a, b) => a.has_param() || b.has_param(),
        }
    }

    fn encode(&self, o: &mut String) {
        match self {
            IfExpr::Num(n) => o.push_str(&n.to_string()),
            IfExpr::Param(p) => o.push_str(p),
            IfExpr::Neg(a) => {
                o.push_str("(neg ");
                a.encode(o);
                o.push(')');
            }
            IfExpr::Bin(op, a, b) => {
                o.push('(');
                // `**` is written `^`.
                o.push(if *op == 'p' { '^' } else { *op });
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
        if let Some(a) = inner.strip_prefix("neg ") {
            let (a, rest) = IfExpr::decode(a)?;
            return Some((IfExpr::Neg(Box::new(a)), rest.strip_prefix(')')?));
        }
        let op = inner.chars().next()?;
        let op = match op {
            '+' | '-' | '*' | '/' | '%' => op,
            '^' => 'p',
            _ => return None,
        };
        let (a, rest) = IfExpr::decode(inner.get(2..)?)?;
        let (b, rest) = IfExpr::decode(rest.strip_prefix(' ')?)?;
        Some((
            IfExpr::Bin(op, Box::new(a), Box::new(b)),
            rest.strip_prefix(')')?,
        ))
    }
}

/// The range of a vector port.
#[derive(Clone, Debug, PartialEq)]
pub struct IfRange {
    /// The bounds with the parameters at their defaults.
    pub msb: i64,
    pub lsb: i64,
    /// The bounds as expressions, when they can be.
    pub exprs: Option<(IfExpr, IfExpr)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct IfPort {
    pub name: String,
    pub dir: IfDir,
    /// `None` for a scalar.
    pub range: Option<IfRange>,
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
            o.push_str(&format!("param {} {v}\n", p.name));
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
                    if let Some((m, l)) = &r.exprs {
                        o.push_str(" ~ ");
                        m.encode(&mut o);
                        o.push_str(" ~ ");
                        l.encode(&mut o);
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
            let (line, exprs) = match line.split_once(" ~ ") {
                Some((l, e)) => (l, Some(e)),
                None => (line, None),
            };
            let w: Vec<&str> = line.split(' ').collect();
            match w.as_slice() {
                ["param", name, "int", v] => out.params.push(IfParam {
                    name: name.to_string(),
                    value: IfValue::Int(v.parse().ok()?),
                }),
                ["param", name, "real", v] => out.params.push(IfParam {
                    name: name.to_string(),
                    value: IfValue::Real(v.parse().ok()?),
                }),
                ["param", name, "str", hex] => {
                    let bytes = (0..hex.len() / 2)
                        .map(|i| u8::from_str_radix(hex.get(2 * i..2 * i + 2)?, 16).ok())
                        .collect::<Option<Vec<u8>>>()?;
                    out.params.push(IfParam {
                        name: name.to_string(),
                        value: IfValue::Str(bytes),
                    });
                }
                ["param", name, "vec", _, bits] => out.params.push(IfParam {
                    name: name.to_string(),
                    value: IfValue::Vec(bits.to_string()),
                }),
                ["port", name, dir, range @ ..] => {
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
                            exprs: match exprs {
                                None => None,
                                Some(e) => {
                                    let (m, rest) = IfExpr::decode(e)?;
                                    let (l, rest) = IfExpr::decode(rest.strip_prefix(" ~ ")?)?;
                                    rest.is_empty().then_some((m, l))
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
                _ => return None,
            }
        }
        Some(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_round_trips() {
        let i = Interface {
            params: vec![
                IfParam {
                    name: "W".into(),
                    value: IfValue::Int(-8),
                },
                IfParam {
                    name: "R".into(),
                    value: IfValue::Real(1.5),
                },
                IfParam {
                    name: "S".into(),
                    value: IfValue::Str(b"TRUE".to_vec()),
                },
                IfParam {
                    name: "V".into(),
                    value: IfValue::Vec("01xz".into()),
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
                        exprs: None,
                    }),
                },
                IfPort {
                    name: "io".into(),
                    dir: IfDir::Inout,
                    range: Some(IfRange {
                        msb: 7,
                        lsb: 0,
                        exprs: Some((
                            IfExpr::Bin(
                                '-',
                                Box::new(IfExpr::Bin(
                                    'p',
                                    Box::new(IfExpr::Param("W".into())),
                                    Box::new(IfExpr::Num(2)),
                                )),
                                Box::new(IfExpr::Neg(Box::new(IfExpr::Num(1)))),
                            ),
                            IfExpr::Num(0),
                        )),
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
}
