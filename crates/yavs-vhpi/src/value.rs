//! Values for `vhpi_get_value` (#270): an object's value as typed scalar elements ([`Read`]),
//! from the kernel or from elaboration, encoded in the format a plugin asks for.
//!
//! Enumeration values are positions at the boundary: std_ulogic positions are the `vhpiU` ..
//! `vhpiDontCare` codes, BIT and BOOLEAN (kernel codes 2 and 3) become 0 and 1 (`vhpibit0`,
//! `vhpiFalse`). TIME values count in the resolution limit, like `vhpi_get_time`; other
//! physical values in base units. Safe Rust only: `abi.rs` copies the result into the
//! plugin's `vhpiValueT`.

// VHPI names are matched as written in the C standard.
#![allow(non_upper_case_globals)]

use std::sync::Arc;

use yavs_kernel::Value;
use yavs_sem::stat::{ConstValue, Elems};

use crate::design::vhpi_name;
use crate::sys::*;

/// One enumeration literal.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Lit {
    /// Original spelling (`vhpiCaseNameP`): `Idle`, `'0'`.
    pub(crate) name: String,
    /// The character a string shows for it: a character literal's, every CHARACTER position.
    pub(crate) ch: Option<u8>,
}

/// The type of a value's scalar elements.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Scalar {
    Enum {
        lits: Arc<[Lit]>,
        /// std_ulogic (and its subtypes) or BIT: natural format `vhpiLogicVal`.
        logic: bool,
        /// STD.CHARACTER: natural format `vhpiCharVal`, arrays `vhpiStrVal`.
        character: bool,
        /// BIT or BOOLEAN: kernel logic codes 2 and 3 for positions 0 and 1.
        bit01: bool,
    },
    Int,
    Real,
    Phys {
        time: bool,
    },
}

/// Element values: enumeration positions, integers and physical values (base units), or
/// reals.
#[derive(Clone, Debug, PartialEq)]
pub(crate) enum Cells {
    Int(Vec<i64>),
    Real(Vec<f64>),
}

/// A scalar, or an array of scalars with its elements in row-major order, leftmost first.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Read {
    pub(crate) ty: Scalar,
    pub(crate) array: bool,
    pub(crate) cells: Cells,
}

/// An encoded value.
pub(crate) enum Encoded {
    /// Goes into `value.value`.
    Scalar(vhpiValueU),
    /// Goes into the plugin's buffer at `value.value.ptr`; `n` is `numElems`.
    Buffer { bytes: Vec<u8>, n: usize },
}

/// Why a value cannot be encoded.
#[derive(Debug, PartialEq)]
pub(crate) enum Error {
    /// A format yavs does not produce (W0904).
    Unsupported(String),
    /// A format that does not apply to this value (W0903).
    Mismatch(String),
}

impl Read {
    fn len(&self) -> usize {
        match &self.cells {
            Cells::Int(v) => v.len(),
            Cells::Real(v) => v.len(),
        }
    }

    fn ints(&self) -> &[i64] {
        match &self.cells {
            Cells::Int(v) => v,
            Cells::Real(_) => &[],
        }
    }

    /// Decodes a kernel value of `n` elements of type `ty`.
    pub(crate) fn from_kernel(ty: Scalar, array: bool, v: &Value) -> Option<Read> {
        let cells = match (&ty, v) {
            // Kernel offset 0 is the rightmost element.
            (Scalar::Enum { bit01, .. }, Value::Logic(l)) => {
                let base = if *bit01 { 2 } else { 0 };
                Cells::Int(
                    (0..l.len())
                        .rev()
                        .map(|i| i64::from((l.get(i) as u8).wrapping_sub(base)))
                        .collect(),
                )
            }
            (Scalar::Enum { lits, .. }, Value::Bytes(b)) if lits.len() <= 256 => {
                Cells::Int(b.iter().map(|&x| i64::from(x)).collect())
            }
            (Scalar::Enum { .. }, Value::Bytes(b)) => Cells::Int(
                b.as_chunks::<4>()
                    .0
                    .iter()
                    .map(|c| i64::from(u32::from_le_bytes(*c)))
                    .collect(),
            ),
            (Scalar::Real, Value::Bytes(b)) => Cells::Real(
                b.as_chunks::<8>()
                    .0
                    .iter()
                    .map(|c| f64::from_le_bytes(*c))
                    .collect(),
            ),
            (Scalar::Int | Scalar::Phys { .. }, Value::Bytes(b)) => Cells::Int(
                b.as_chunks::<8>()
                    .0
                    .iter()
                    .map(|c| i64::from_le_bytes(*c))
                    .collect(),
            ),
            _ => return None,
        };
        Some(Read { ty, array, cells })
    }

    /// An elaboration-time value: a scalar, or an array of scalars.
    pub(crate) fn from_const(ty: Scalar, v: &ConstValue) -> Option<Read> {
        let (array, vals): (bool, Vec<ConstValue>) = match v {
            ConstValue::Array(a) => (
                true,
                match &a.elems {
                    Elems::Bytes(b) => b.iter().map(|&x| ConstValue::Enum(x.into())).collect(),
                    Elems::Runs(r) => r
                        .iter()
                        .flat_map(|(n, v)| std::iter::repeat_n(v.clone(), *n as usize))
                        .collect(),
                },
            ),
            v => (false, vec![v.clone()]),
        };
        let cells = match ty {
            Scalar::Real => Cells::Real(
                vals.iter()
                    .map(|v| match v {
                        ConstValue::Real(r) => Some(*r),
                        _ => None,
                    })
                    .collect::<Option<_>>()?,
            ),
            _ => Cells::Int(vals.iter().map(ConstValue::as_int).collect::<Option<_>>()?),
        };
        Some(Read { ty, array, cells })
    }

    /// The format `vhpiObjTypeVal` stands for.
    pub(crate) fn natural(&self) -> vhpiFormatT {
        let (s, a) = match &self.ty {
            Scalar::Enum { logic: true, .. } => (vhpiLogicVal, vhpiLogicVecVal),
            Scalar::Enum {
                character: true, ..
            } => (vhpiCharVal, vhpiStrVal),
            Scalar::Enum { lits, .. } if lits.len() <= 256 => {
                (vhpiSmallEnumVal, vhpiSmallEnumVecVal)
            }
            Scalar::Enum { .. } => (vhpiEnumVal, vhpiEnumVecVal),
            // ponytail: 32-bit integer formats; report vhpiLongIntVal for types wider than
            // 32 bits when 64-bit integer types need it.
            Scalar::Int => (vhpiIntVal, vhpiIntVecVal),
            Scalar::Real => (vhpiRealVal, vhpiRealVecVal),
            Scalar::Phys { time: true } => (vhpiTimeVal, vhpiTimeVecVal),
            Scalar::Phys { time: false } => (vhpiPhysVal, vhpiPhysVecVal),
        };
        if self.array { a } else { s }
    }

    /// The value in `format` (not `vhpiObjTypeVal`); `res_fs` is the resolution limit.
    pub(crate) fn encode(&self, format: vhpiFormatT, res_fs: u64) -> Result<Encoded, Error> {
        let name = format_name(format);
        let mismatch = || {
            Error::Mismatch(format!(
                "format {name} does not apply to {}",
                self.describe()
            ))
        };
        let enum_lits = match &self.ty {
            Scalar::Enum { lits, .. } => Some(lits.len()),
            _ => None,
        };
        let scalar = |ok: bool| {
            if ok && !self.array {
                Ok(self.ints()[0])
            } else {
                Err(mismatch())
            }
        };
        let vector = |ok: bool| {
            if ok && self.array {
                Ok(self.ints())
            } else {
                Err(mismatch())
            }
        };
        let int32 = |v: i64| {
            i32::try_from(v).map_err(|_| Error::Mismatch(format!("{v} does not fit format {name}")))
        };
        let is_int = matches!(self.ty, Scalar::Int);
        let is_phys = matches!(self.ty, Scalar::Phys { .. });
        let is_time = matches!(self.ty, Scalar::Phys { time: true });
        let ticks = |fs: i64| fs / res_fs as i64;
        let buf = |bytes: Vec<u8>| Encoded::Buffer {
            bytes,
            n: self.len(),
        };
        Ok(match format {
            vhpiBinStrVal => self.radix(1).ok_or_else(mismatch)?,
            vhpiOctStrVal => self.radix(3).ok_or_else(mismatch)?,
            vhpiHexStrVal => self.radix(4).ok_or_else(mismatch)?,
            vhpiDecStrVal => self.decimal()?.ok_or_else(mismatch)?,
            vhpiStrVal => self.string().ok_or_else(mismatch)?,
            vhpiCharVal => {
                let ch = match &self.ty {
                    Scalar::Enum { lits, .. } if !self.array => lits[self.ints()[0] as usize].ch,
                    _ => None,
                };
                Encoded::Scalar(vhpiValueU {
                    ch: ch.ok_or_else(mismatch)?,
                })
            }
            vhpiEnumVal | vhpiLogicVal => Encoded::Scalar(vhpiValueU {
                enumv: scalar(enum_lits.is_some())? as u32,
            }),
            vhpiSmallEnumVal => Encoded::Scalar(vhpiValueU {
                smallenumv: scalar(enum_lits.is_some_and(|n| n <= 256))? as u8,
            }),
            vhpiIntVal => Encoded::Scalar(vhpiValueU {
                intg: int32(scalar(is_int || enum_lits.is_some())?)?,
            }),
            vhpiLongIntVal => Encoded::Scalar(vhpiValueU {
                longintg: scalar(is_int || enum_lits.is_some())?,
            }),
            vhpiRealVal => match (&self.cells, self.array) {
                (Cells::Real(v), false) => Encoded::Scalar(vhpiValueU { real: v[0] }),
                _ => return Err(mismatch()),
            },
            vhpiTimeVal => Encoded::Scalar(vhpiValueU {
                time: split_time(ticks(scalar(is_time)?)),
            }),
            vhpiPhysVal => Encoded::Scalar(vhpiValueU {
                phys: split_phys(scalar(is_phys)?),
            }),
            vhpiEnumVecVal | vhpiLogicVecVal => buf(vector(enum_lits.is_some())?
                .iter()
                .flat_map(|&p| (p as u32).to_ne_bytes())
                .collect()),
            vhpiSmallEnumVecVal => buf(vector(enum_lits.is_some_and(|n| n <= 256))?
                .iter()
                .map(|&p| p as u8)
                .collect()),
            vhpiIntVecVal => {
                let mut out = Vec::new();
                for &v in vector(is_int)? {
                    out.extend(int32(v)?.to_ne_bytes());
                }
                buf(out)
            }
            vhpiLongIntVecVal => buf(vector(is_int)?
                .iter()
                .flat_map(|v| v.to_ne_bytes())
                .collect()),
            vhpiRealVecVal => match (&self.cells, self.array) {
                (Cells::Real(v), true) => buf(v.iter().flat_map(|r| r.to_ne_bytes()).collect()),
                _ => return Err(mismatch()),
            },
            vhpiTimeVecVal => buf(vector(is_time)?
                .iter()
                .flat_map(|&v| {
                    let t = split_time(ticks(v));
                    [t.high.to_ne_bytes(), t.low.to_ne_bytes()].concat()
                })
                .collect()),
            vhpiPhysVecVal => buf(vector(is_phys)?
                .iter()
                .flat_map(|&v| {
                    let p = split_phys(v);
                    [p.high.to_ne_bytes(), p.low.to_ne_bytes()].concat()
                })
                .collect()),
            _ => {
                return Err(Error::Unsupported(format!("vhpi_get_value: format {name}")));
            }
        })
    }

    fn describe(&self) -> String {
        let t = match &self.ty {
            Scalar::Enum { logic: true, .. } => "a logic",
            Scalar::Enum {
                character: true, ..
            } => "a character",
            Scalar::Enum { .. } => "an enumeration",
            Scalar::Int => "an integer",
            Scalar::Real => "a real",
            Scalar::Phys { time: true } => "a time",
            Scalar::Phys { time: false } => "a physical",
        };
        if self.array {
            format!("{t} array")
        } else {
            format!("{t} value")
        }
    }

    /// The characters of a logic value, leftmost first.
    fn logic_chars(&self) -> Option<Vec<u8>> {
        let Scalar::Enum {
            lits, logic: true, ..
        } = &self.ty
        else {
            return None;
        };
        self.ints().iter().map(|&p| lits[p as usize].ch).collect()
    }

    /// Binary (`bits` 1), octal (3) or hexadecimal (4) text. Logic values: one character per
    /// element in binary; a digit for each group of bits counted from the right, the group's
    /// character when all its elements are one metavalue, `X` for a mixed group. Other
    /// discrete and physical scalars: the number, signed.
    fn radix(&self, bits: usize) -> Option<Encoded> {
        if let Some(chars) = self.logic_chars() {
            let text = if bits == 1 {
                chars
            } else {
                let pad = (bits - chars.len() % bits) % bits;
                let padded: Vec<u8> = std::iter::repeat_n(b'0', pad).chain(chars).collect();
                padded
                    .chunks(bits)
                    .map(|g| {
                        if g.iter().all(|c| matches!(c, b'0' | b'1')) {
                            let d = g.iter().fold(0, |a, &c| a * 2 + u32::from(c - b'0'));
                            char::from_digit(d, 16).expect("digit").to_ascii_uppercase() as u8
                        } else if g.iter().all(|&c| c == g[0]) {
                            g[0]
                        } else {
                            b'X'
                        }
                    })
                    .collect()
            };
            return Some(cstring(text));
        }
        if self.array || matches!(self.ty, Scalar::Real) {
            return None;
        }
        let v = self.ints()[0];
        let digits = match bits {
            1 => format!("{:b}", v.unsigned_abs()),
            3 => format!("{:o}", v.unsigned_abs()),
            _ => format!("{:X}", v.unsigned_abs()),
        };
        let sign = if v < 0 { "-" } else { "" };
        Some(cstring(format!("{sign}{digits}").into_bytes()))
    }

    /// Decimal text: a logic value as an unsigned number (an error with metavalues), other
    /// discrete and physical scalars as the number.
    fn decimal(&self) -> Result<Option<Encoded>, Error> {
        if let Some(chars) = self.logic_chars() {
            let mut digits = vec![0u8]; // least significant first
            for c in chars {
                let bit = match c {
                    b'0' => 0,
                    b'1' => 1,
                    _ => {
                        return Err(Error::Mismatch(format!(
                            "format vhpiDecStrVal: the value has the metavalue '{}'",
                            c as char
                        )));
                    }
                };
                let mut carry = bit;
                for d in &mut digits {
                    let x = *d * 2 + carry;
                    *d = x % 10;
                    carry = x / 10;
                }
                if carry > 0 {
                    digits.push(carry);
                }
            }
            let text = digits.iter().rev().map(|d| b'0' + d).collect();
            return Ok(Some(cstring(text)));
        }
        if self.array || matches!(self.ty, Scalar::Real) {
            return Ok(None);
        }
        Ok(Some(cstring(self.ints()[0].to_string().into_bytes())))
    }

    /// `vhpiStrVal`: an array of character-literal elements as its characters, an enumeration
    /// value as its literal.
    fn string(&self) -> Option<Encoded> {
        let Scalar::Enum { lits, .. } = &self.ty else {
            return None;
        };
        let lit = |p: i64| &lits[p as usize];
        let text = if self.array {
            self.ints()
                .iter()
                .map(|&p| lit(p).ch)
                .collect::<Option<Vec<u8>>>()?
        } else {
            let l = lit(self.ints()[0]);
            match l.ch {
                Some(c) => vec![c],
                None => vhpi_name(&l.name).into_bytes(),
            }
        };
        Some(cstring(text))
    }
}

/// A NUL-terminated buffer; `numElems` is its length without the NUL.
fn cstring(mut text: Vec<u8>) -> Encoded {
    let n = text.len();
    text.push(0);
    Encoded::Buffer { bytes: text, n }
}

fn split_time(v: i64) -> vhpiTimeT {
    vhpiTimeT {
        high: (v >> 32) as i32,
        low: v as u32,
    }
}

fn split_phys(v: i64) -> vhpiPhysT {
    vhpiPhysT {
        high: (v >> 32) as i32,
        low: v as u32,
    }
}

/// `vhpiIntVal` for 6.
pub(crate) fn format_name(f: vhpiFormatT) -> String {
    VHPIFORMATT_VALUES
        .iter()
        .find(|&&(_, v)| v == i64::from(f))
        .map_or_else(|| format!("{f}"), |(n, _)| (*n).to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use yavs_kernel::LogicVec;

    fn std_logic() -> Scalar {
        let lits: Vec<Lit> = "UX01ZWLH-"
            .bytes()
            .map(|c| Lit {
                name: format!("'{}'", c as char),
                ch: Some(c),
            })
            .collect();
        Scalar::Enum {
            lits: lits.into(),
            logic: true,
            character: false,
            bit01: false,
        }
    }

    fn bit() -> Scalar {
        Scalar::Enum {
            lits: vec![
                Lit {
                    name: "'0'".into(),
                    ch: Some(b'0'),
                },
                Lit {
                    name: "'1'".into(),
                    ch: Some(b'1'),
                },
            ]
            .into(),
            logic: true,
            character: false,
            bit01: true,
        }
    }

    fn text(e: Encoded) -> (String, usize) {
        match e {
            Encoded::Buffer { bytes, n } => {
                assert_eq!(bytes.last(), Some(&0));
                (String::from_utf8(bytes[..n].to_vec()).unwrap(), n)
            }
            Encoded::Scalar(_) => panic!("scalar"),
        }
    }

    fn slv(s: &str) -> Read {
        let v: LogicVec = s.parse().unwrap();
        Read::from_kernel(std_logic(), true, &Value::Logic(v)).unwrap()
    }

    #[test]
    fn logic_vectors_in_every_string_format() {
        let r = slv("10XZ01UH");
        assert_eq!(r.natural(), vhpiLogicVecVal);
        assert_eq!(r.ints(), [3, 2, 1, 4, 2, 3, 0, 7]);
        assert_eq!(text(r.encode(vhpiBinStrVal, 1).unwrap()).0, "10XZ01UH");
        assert_eq!(text(r.encode(vhpiStrVal, 1).unwrap()).0, "10XZ01UH");
        assert_eq!(text(r.encode(vhpiHexStrVal, 1).unwrap()).0, "XX");
        let r = slv("1011ZZZZ0000");
        assert_eq!(text(r.encode(vhpiHexStrVal, 1).unwrap()), ("BZ0".into(), 3));
        assert_eq!(text(r.encode(vhpiOctStrVal, 1).unwrap()).0, "5XX0");
        assert_eq!(text(slv("11010").encode(vhpiHexStrVal, 1).unwrap()).0, "1A");
        let r = slv("1".repeat(130).as_str());
        let (dec, _) = text(r.encode(vhpiDecStrVal, 1).unwrap());
        assert_eq!(dec, "1361129467683753853853498429727072845823");
        assert!(matches!(
            slv("1X").encode(vhpiDecStrVal, 1),
            Err(Error::Mismatch(_))
        ));
        assert!(matches!(
            slv("10").encode(vhpiIntVal, 1),
            Err(Error::Mismatch(_))
        ));
        let Encoded::Buffer { bytes, n: 2 } = slv("1U").encode(vhpiLogicVecVal, 1).unwrap() else {
            panic!()
        };
        assert_eq!(bytes, [3u32.to_ne_bytes(), 0u32.to_ne_bytes()].concat());
    }

    #[test]
    fn bit_and_boolean_are_positions() {
        let v: LogicVec = "10".parse().unwrap(); // kernel codes '1' '0' = 3 2
        let r = Read::from_kernel(bit(), true, &Value::Logic(v)).unwrap();
        assert_eq!(r.ints(), [1, 0]);
        assert_eq!(text(r.encode(vhpiBinStrVal, 1).unwrap()).0, "10");
        assert_eq!(text(r.encode(vhpiDecStrVal, 1).unwrap()).0, "2");
    }

    #[test]
    fn scalars() {
        let i = Read {
            ty: Scalar::Int,
            array: false,
            cells: Cells::Int(vec![-42]),
        };
        assert_eq!(i.natural(), vhpiIntVal);
        let Encoded::Scalar(u) = i.encode(vhpiIntVal, 1).unwrap() else {
            panic!()
        };
        assert_eq!(unsafe_free_int(u), -42);
        assert_eq!(text(i.encode(vhpiDecStrVal, 1).unwrap()).0, "-42");
        assert_eq!(text(i.encode(vhpiHexStrVal, 1).unwrap()).0, "-2A");
        assert!(matches!(i.encode(vhpiRealVal, 1), Err(Error::Mismatch(_))));
        assert!(matches!(
            i.encode(vhpiPtrVal, 1),
            Err(Error::Unsupported(_))
        ));
        let big = Read {
            cells: Cells::Int(vec![1 << 40]),
            ..i
        };
        assert!(matches!(big.encode(vhpiIntVal, 1), Err(Error::Mismatch(_))));
        let t = Read {
            ty: Scalar::Phys { time: true },
            array: false,
            cells: Cells::Int(vec![3_000_000]),
        };
        assert_eq!(t.natural(), vhpiTimeVal);
        let Encoded::Scalar(u) = t.encode(vhpiTimeVal, 1000).unwrap() else {
            panic!()
        };
        // SAFETY (test): the time field was written.
        let tm = unsafe { u.time };
        assert_eq!((tm.high, tm.low), (0, 3000));
    }

    fn unsafe_free_int(u: vhpiValueU) -> i32 {
        // SAFETY (test): the int field was written.
        unsafe { u.intg }
    }

    #[test]
    fn enums_strings_and_constants() {
        let lits: Arc<[Lit]> = ["IDLE", "RUN"]
            .iter()
            .map(|n| Lit {
                name: (*n).into(),
                ch: None,
            })
            .collect();
        let st = Scalar::Enum {
            lits,
            logic: false,
            character: false,
            bit01: false,
        };
        let r = Read::from_const(st.clone(), &ConstValue::Enum(1)).unwrap();
        assert_eq!(r.natural(), vhpiSmallEnumVal);
        assert_eq!(text(r.encode(vhpiStrVal, 1).unwrap()).0, "RUN");
        let chars: Arc<[Lit]> = (0..=255u8)
            .map(|c| Lit {
                name: format!("'{}'", c as char),
                ch: Some(c),
            })
            .collect();
        let character = Scalar::Enum {
            lits: chars,
            logic: false,
            character: true,
            bit01: false,
        };
        let s = Read::from_kernel(character, true, &Value::Bytes(b"hello".to_vec())).unwrap();
        assert_eq!(s.natural(), vhpiStrVal);
        assert_eq!(text(s.encode(vhpiStrVal, 1).unwrap()), ("hello".into(), 5));
        assert!(matches!(
            s.encode(vhpiBinStrVal, 1),
            Err(Error::Mismatch(_))
        ));
        let arr = ConstValue::Array(yavs_sem::stat::ArrayConst {
            bounds: Box::new([(0, yavs_sem::hir::Direction::To, 2)]),
            elems: Elems::Runs(Box::new([
                (2, ConstValue::Int(10)),
                (1, ConstValue::Int(-20)),
            ])),
        });
        let iv = Read::from_const(Scalar::Int, &arr).unwrap();
        assert_eq!(iv.natural(), vhpiIntVecVal);
        let Encoded::Buffer { bytes, n: 3 } = iv.encode(vhpiIntVecVal, 1).unwrap() else {
            panic!()
        };
        assert_eq!(
            bytes,
            [10i32, 10, -20]
                .iter()
                .flat_map(|v| v.to_ne_bytes())
                .collect::<Vec<_>>()
        );
    }
}
