//! The VHDL view of a module: what `entity lib.module` in VHDL takes its generics and ports
//! from. Analysis records it (parameters at their default values, so ports sized by a parameter
//! have the width the defaults give) as text in the module's library record, and VHDL analysis
//! reads that text without the Verilog front end.
//!
//! One line per item: `param NAME int 8`, `param NAME real 1.5`, `param NAME str HEX`,
//! `param NAME vec 4 01xz` (most significant bit first), `port NAME in|out|inout scalar`,
//! `port NAME in|out|inout MSB LSB`, or the single line `error MESSAGE`.

use crate::Dir;

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

#[derive(Clone, Debug, PartialEq)]
pub struct IfPort {
    pub name: String,
    pub dir: Dir,
    /// `(msb, lsb)` as declared; `None` for a scalar.
    pub range: Option<(i64, i64)>,
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
                Dir::Input => "in",
                Dir::Output => "out",
                Dir::Inout => "inout",
            };
            match p.range {
                None => o.push_str(&format!("port {} {dir} scalar\n", p.name)),
                Some((m, l)) => o.push_str(&format!("port {} {dir} {m} {l}\n", p.name)),
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
                        "in" => Dir::Input,
                        "out" => Dir::Output,
                        "inout" => Dir::Inout,
                        _ => return None,
                    };
                    let range = match range {
                        ["scalar"] => None,
                        [m, l] => Some((m.parse().ok()?, l.parse().ok()?)),
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
                    dir: Dir::Input,
                    range: None,
                },
                IfPort {
                    name: "q".into(),
                    dir: Dir::Output,
                    range: Some((0, 7)),
                },
                IfPort {
                    name: "io".into(),
                    dir: Dir::Inout,
                    range: Some((7, 0)),
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
