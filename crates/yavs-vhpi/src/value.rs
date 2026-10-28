//! Values for `vhpi_get_value` (#270): an object's value as typed scalar elements ([`Read`]),
//! from the kernel or from elaboration, encoded in the format a plugin asks for; and for
//! `vhpi_put_value` (#271) the reverse, a plugin's value decoded into a kernel value.
//!
//! Enumeration values are positions at the boundary: std_ulogic positions are the `vhpiU` ..
//! `vhpiDontCare` codes, BIT and BOOLEAN (kernel codes 2 and 3) become 0 and 1 (`vhpibit0`,
//! `vhpiFalse`). TIME values count in the resolution limit, like `vhpi_get_time`; other
//! physical values in base units. Safe Rust only: `abi.rs` copies the result into the
//! plugin's `vhpiValueT`.

// VHPI names are matched as written in the C standard.
#![allow(non_upper_case_globals)]

use std::sync::Arc;

use yavs_kernel::{LogicVec, Value};
use yavs_sem::stat::{ArrayConst, ConstValue, Elems};

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
    /// `narrow`: the kernel holds it in 4 bytes, not 8 (an integer type within 32 bits,
    /// `Ty::elem_size`).
    Int {
        narrow: bool,
    },
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

impl Scalar {
    /// Stored as logic planes in the kernel: std_ulogic, BIT and BOOLEAN.
    pub(crate) fn on_planes(&self) -> bool {
        matches!(
            self,
            Scalar::Enum { logic: true, .. } | Scalar::Enum { bit01: true, .. }
        )
    }
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
            (Scalar::Int { narrow: true }, Value::Bytes(b)) => Cells::Int(
                b.as_chunks::<4>()
                    .0
                    .iter()
                    .map(|c| i64::from(i32::from_le_bytes(*c)))
                    .collect(),
            ),
            (Scalar::Int { .. } | Scalar::Phys { .. }, Value::Bytes(b)) => Cells::Int(
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
            ConstValue::Array(a) => (true, elements(a)),
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

    /// This value as an elaborated value; an array takes the bounds of `like`, the value it
    /// replaces (#309).
    pub(crate) fn to_const(&self, like: &ConstValue) -> Option<ConstValue> {
        let one = |i: usize| -> Option<ConstValue> {
            Some(match (&self.ty, &self.cells) {
                (_, Cells::Real(v)) => ConstValue::Real(*v.get(i)?),
                (Scalar::Enum { .. }, Cells::Int(v)) => {
                    ConstValue::Enum(u32::try_from(*v.get(i)?).ok()?)
                }
                (Scalar::Phys { .. }, Cells::Int(v)) => ConstValue::Phys(*v.get(i)?),
                (_, Cells::Int(v)) => ConstValue::Int(*v.get(i)?),
            })
        };
        if !self.array {
            return one(0);
        }
        let ConstValue::Array(a) = like else {
            return None;
        };
        let n = match &self.cells {
            Cells::Int(v) => v.len(),
            Cells::Real(v) => v.len(),
        };
        let elems = (0..n)
            .map(|i| one(i).map(|x| (1, x)))
            .collect::<Option<_>>()?;
        Some(ConstValue::Array(ArrayConst {
            bounds: a.bounds.clone(),
            elems: Elems::Runs(elems),
        }))
    }

    /// The format `vhpiObjTypeVal` stands for.
    pub(crate) fn natural(&self) -> vhpiFormatT {
        natural_format(&self.ty, self.array)
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
        let is_int = matches!(self.ty, Scalar::Int { .. });
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
            vhpiLogicVal => Encoded::Scalar(vhpiValueU {
                enumv: scalar(enum_lits.is_some())? as u32,
            }),
            // Both reference simulators also read an integer with vhpiEnumVal (#318).
            vhpiEnumVal => {
                let v = scalar(is_int || enum_lits.is_some())?;
                Encoded::Scalar(vhpiValueU {
                    enumv: u32::try_from(v)
                        .map_err(|_| Error::Mismatch(format!("{v} does not fit format {name}")))?,
                })
            }
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
            Scalar::Int { .. } => "an integer",
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

/// The value a plugin passes to `vhpi_put_value`, read out of its `vhpiValueT` by `abi.rs`.
pub(crate) enum Given<'a> {
    /// A scalar format: the union member it names.
    Scalar(Member),
    /// A string format: the text up to the NUL.
    Text(&'a [u8]),
    /// A vector format: `numElems` elements of the format's C type, in native byte order.
    Vector(&'a [u8]),
}

/// The bytes of one element of vector format `f`; `None` for other formats.
pub(crate) fn vector_elem_size(f: vhpiFormatT) -> Option<usize> {
    Some(match f {
        vhpiEnumVecVal | vhpiLogicVecVal | vhpiIntVecVal => 4,
        vhpiSmallEnumVecVal => 1,
        vhpiLongIntVecVal | vhpiRealVecVal | vhpiTimeVecVal | vhpiPhysVecVal => 8,
        _ => return None,
    })
}

pub(crate) fn is_text(f: vhpiFormatT) -> bool {
    matches!(
        f,
        vhpiBinStrVal | vhpiOctStrVal | vhpiHexStrVal | vhpiDecStrVal | vhpiStrVal
    )
}

impl Read {
    /// `vhpi_put_value`: the value `given` in `format` (not `vhpiObjTypeVal`) as elements of
    /// `ty`: `n` of them for an array, one scalar for `None`. `res_fs` is the resolution limit.
    pub(crate) fn decode(
        ty: Scalar,
        n: Option<usize>,
        format: vhpiFormatT,
        given: Given<'_>,
        res_fs: u64,
    ) -> Result<Read, Error> {
        let name = format_name(format);
        let what = match (&ty, n) {
            (Scalar::Enum { logic: true, .. }, _) => "a logic",
            (Scalar::Enum { .. }, _) => "an enumeration",
            (Scalar::Int { .. }, _) => "an integer",
            (Scalar::Real, _) => "a real",
            (Scalar::Phys { .. }, _) => "a physical",
        };
        let shape = if n.is_some() { "array" } else { "object" };
        let mismatch =
            || Error::Mismatch(format!("format {name} does not apply to {what} {shape}"));
        let bad = |m: String| Error::Mismatch(format!("format {name}: {m}"));
        let lits = match &ty {
            Scalar::Enum { lits, .. } => Some(lits.clone()),
            _ => None,
        };
        let is_time = matches!(ty, Scalar::Phys { time: true });
        let is_phys = matches!(ty, Scalar::Phys { .. });
        let fs = |ticks: i64| {
            ticks
                .checked_mul(res_fs as i64)
                .ok_or_else(|| bad(format!("time {ticks} overflows")))
        };
        // The position of the literal shown as character `c`.
        let by_char = |c: u8| {
            let lits = lits.as_ref()?;
            let find = |c: u8| lits.iter().position(|l| l.ch == Some(c));
            find(c).or_else(|| find(c.to_ascii_uppercase()))
        };
        let cells: Cells = match (format, given) {
            (_, Given::Scalar(v)) if n.is_none() => match (v, &ty) {
                (Member::Real(r), Scalar::Real) => Cells::Real(vec![r]),
                (Member::Int(i), Scalar::Int { .. } | Scalar::Enum { .. }) => Cells::Int(vec![i]),
                (Member::Char(c), Scalar::Enum { .. }) => Cells::Int(vec![
                    by_char(c).ok_or_else(|| bad(format!("no literal '{}'", c as char)))? as i64,
                ]),
                (Member::Time(t), _) if is_time => Cells::Int(vec![fs(t)?]),
                (Member::Phys(p), _) if is_phys && !is_time => Cells::Int(vec![p]),
                _ => return Err(mismatch()),
            },
            // An integer into a logic array: its two's complement, the rightmost element the
            // least significant bit (#319).
            (vhpiIntVal | vhpiLongIntVal, Given::Scalar(Member::Int(i)))
                if matches!(ty, Scalar::Enum { logic: true, .. }) && n.is_some() =>
            {
                let bits = n.expect("guarded");
                if bits < 64 && !(-(1i64 << bits.saturating_sub(1))..1i64 << bits).contains(&i) {
                    return Err(bad(format!("{i} does not fit {bits} bits")));
                }
                Cells::Int(
                    (0..bits)
                        .rev()
                        .map(|k| {
                            let c = if (i >> k.min(63)) & 1 == 1 {
                                b'1'
                            } else {
                                b'0'
                            };
                            by_char(c)
                                .map(|p| p as i64)
                                .ok_or_else(|| bad(format!("no literal '{}'", c as char)))
                        })
                        .collect::<Result<_, _>>()?,
                )
            }
            (_, Given::Vector(b)) if n.is_some() => {
                let size = vector_elem_size(format).ok_or_else(mismatch)?;
                let words = b.chunks_exact(size);
                match (format, &ty) {
                    (vhpiRealVecVal, Scalar::Real) => Cells::Real(
                        words
                            .map(|w| f64::from_ne_bytes(w.try_into().expect("8 bytes")))
                            .collect(),
                    ),
                    (vhpiEnumVecVal | vhpiLogicVecVal, Scalar::Enum { .. }) => Cells::Int(
                        words
                            .map(|w| i64::from(u32::from_ne_bytes(w.try_into().expect("4 bytes"))))
                            .collect(),
                    ),
                    (vhpiSmallEnumVecVal, Scalar::Enum { .. }) => {
                        Cells::Int(words.map(|w| i64::from(w[0])).collect())
                    }
                    (vhpiIntVecVal, Scalar::Int { .. }) => Cells::Int(
                        words
                            .map(|w| i64::from(i32::from_ne_bytes(w.try_into().expect("4 bytes"))))
                            .collect(),
                    ),
                    (vhpiLongIntVecVal, Scalar::Int { .. }) => Cells::Int(
                        words
                            .map(|w| i64::from_ne_bytes(w.try_into().expect("8 bytes")))
                            .collect(),
                    ),
                    (vhpiTimeVecVal | vhpiPhysVecVal, _)
                        if is_phys && is_time == (format == vhpiTimeVecVal) =>
                    {
                        let mut out = Vec::new();
                        for w in words {
                            let high = i32::from_ne_bytes(w[..4].try_into().expect("4 bytes"));
                            let low = u32::from_ne_bytes(w[4..].try_into().expect("4 bytes"));
                            let v = (i64::from(high) << 32) | i64::from(low);
                            out.push(if is_time { fs(v)? } else { v });
                        }
                        Cells::Int(out)
                    }
                    _ => return Err(mismatch()),
                }
            }
            // Text for an array fills it from the left: longer text is cut, shorter text is
            // padded with NUL where the element type has it (two of three references, #345).
            (vhpiStrVal, Given::Text(t)) => match n {
                Some(n) => {
                    let mut v = t
                        .iter()
                        .take(n)
                        .map(|&c| {
                            by_char(c)
                                .map(|p| p as i64)
                                .ok_or_else(|| bad(format!("no literal '{}'", c as char)))
                        })
                        .collect::<Result<Vec<_>, _>>()?;
                    if let Some(nul) = by_char(0) {
                        v.resize(n, nul as i64);
                    }
                    Cells::Int(v)
                }
                None => {
                    let lits = lits.as_ref().ok_or_else(mismatch)?;
                    let text = String::from_utf8_lossy(t);
                    let p = lits
                        .iter()
                        .position(|l| match l.ch {
                            Some(c) => t == [c],
                            None => vhpi_name(&l.name) == vhpi_name(&text),
                        })
                        .ok_or_else(|| bad(format!("no literal {text}")))?;
                    Cells::Int(vec![p as i64])
                }
            },
            (vhpiBinStrVal | vhpiOctStrVal | vhpiHexStrVal | vhpiDecStrVal, Given::Text(t)) => {
                let text = std::str::from_utf8(t).map_err(|_| bad("not ASCII text".into()))?;
                let radix = match format {
                    vhpiBinStrVal => 2,
                    vhpiOctStrVal => 8,
                    vhpiHexStrVal => 16,
                    _ => 10,
                };
                match (n, &ty) {
                    // A logic array: binary text is one literal per element; other radixes a
                    // number of that many bits.
                    (n, Scalar::Enum { logic: true, .. }) => {
                        let bits = n.unwrap_or(1);
                        let chars: Vec<u8> = if radix == 2 {
                            t.to_vec()
                        } else {
                            to_bits(text, radix, bits).ok_or_else(|| {
                                bad(format!("{text} is not a number of {bits} bits"))
                            })?
                        };
                        Cells::Int(
                            chars
                                .iter()
                                .map(|&c| {
                                    by_char(c)
                                        .map(|p| p as i64)
                                        .ok_or_else(|| bad(format!("no literal '{}'", c as char)))
                                })
                                .collect::<Result<_, _>>()?,
                        )
                    }
                    (None, Scalar::Int { .. } | Scalar::Enum { .. }) => {
                        let (neg, digits) = match text.strip_prefix('-') {
                            Some(d) => (true, d),
                            None => (false, text),
                        };
                        let v = i64::from_str_radix(digits, radix)
                            .map_err(|_| bad(format!("{text} is not a number")))?;
                        Cells::Int(vec![if neg { -v } else { v }])
                    }
                    _ => return Err(mismatch()),
                }
            }
            _ => return Err(mismatch()),
        };
        let len = match &cells {
            Cells::Int(v) => v.len(),
            Cells::Real(v) => v.len(),
        };
        match n {
            Some(n) if len != n => {
                return Err(bad(format!("{len} elements for an array of {n}")));
            }
            None if len != 1 => return Err(bad(format!("{len} elements for a scalar"))),
            _ => {}
        }
        if let (Some(lits), Cells::Int(v)) = (&lits, &cells)
            && let Some(p) = v.iter().find(|&&p| p < 0 || p as usize >= lits.len())
        {
            return Err(bad(format!("{p} is not a literal position")));
        }
        Ok(Read {
            ty,
            array: n.is_some(),
            cells,
        })
    }

    /// The kernel value of these elements, laid out as `from_kernel` reads it.
    pub(crate) fn to_kernel(&self) -> Value {
        match (&self.ty, &self.cells) {
            (Scalar::Enum { logic, bit01, .. }, Cells::Int(v)) if *logic || *bit01 => {
                // Leftmost first, as a literal: the kernel stores the rightmost at offset 0.
                // BIT and BOOLEAN are the codes of '0' and '1'.
                let chars: String = v
                    .iter()
                    .map(|&p| {
                        let code = p as usize + if *bit01 { 2 } else { 0 };
                        b"UX01ZWLH-"[code] as char
                    })
                    .collect();
                Value::Logic(chars.parse::<LogicVec>().expect("logic literals"))
            }
            (Scalar::Enum { lits, .. }, Cells::Int(v)) if lits.len() <= 256 => {
                Value::Bytes(v.iter().map(|&p| p as u8).collect())
            }
            (Scalar::Enum { .. }, Cells::Int(v)) => {
                Value::Bytes(v.iter().flat_map(|&p| (p as u32).to_le_bytes()).collect())
            }
            (Scalar::Int { narrow: true }, Cells::Int(v)) => {
                Value::Bytes(v.iter().flat_map(|&p| (p as i32).to_le_bytes()).collect())
            }
            (_, Cells::Int(v)) => Value::Bytes(v.iter().flat_map(|p| p.to_le_bytes()).collect()),
            (_, Cells::Real(v)) => Value::Bytes(v.iter().flat_map(|r| r.to_le_bytes()).collect()),
        }
    }
}

/// The union member a scalar format names.
pub(crate) enum Member {
    Int(i64),
    Real(f64),
    Char(u8),
    /// Ticks of the resolution limit.
    Time(i64),
    Phys(i64),
}

/// The `n` binary digits, leftmost first, of the unsigned number `text` in `radix`; `None`
/// when it is not one or needs more bits.
fn to_bits(text: &str, radix: u32, n: usize) -> Option<Vec<u8>> {
    let mut bits = vec![0u8; n]; // least significant first
    for c in text.chars() {
        let d = c.to_digit(radix)?;
        // bits = bits * radix + d
        let mut carry = d;
        for b in &mut bits {
            let x = u32::from(*b) * radix + carry;
            *b = (x % 2) as u8;
            carry = x / 2;
        }
        if carry != 0 {
            return None;
        }
    }
    Some(bits.iter().rev().map(|b| b'0' + b).collect())
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

/// The format `vhpiObjTypeVal` stands for, for a scalar or an `array` of `ty`.
/// The elements of an array value, row major.
pub(crate) fn elements(a: &ArrayConst) -> Vec<ConstValue> {
    match &a.elems {
        Elems::Bytes(b) => b.iter().map(|&x| ConstValue::Enum(x.into())).collect(),
        Elems::Runs(r) => r
            .iter()
            .flat_map(|(n, v)| std::iter::repeat_n(v.clone(), *n as usize))
            .collect(),
    }
}

pub(crate) fn natural_format(ty: &Scalar, array: bool) -> vhpiFormatT {
    let (s, a) = match ty {
        Scalar::Enum { logic: true, .. } => (vhpiLogicVal, vhpiLogicVecVal),
        Scalar::Enum {
            character: true, ..
        } => (vhpiCharVal, vhpiStrVal),
        Scalar::Enum { lits, .. } if lits.len() <= 256 => (vhpiSmallEnumVal, vhpiSmallEnumVecVal),
        Scalar::Enum { .. } => (vhpiEnumVal, vhpiEnumVecVal),
        // ponytail: 32-bit integer formats; report vhpiLongIntVal for types wider than
        // 32 bits when 64-bit integer types need it.
        Scalar::Int { .. } => (vhpiIntVal, vhpiIntVecVal),
        Scalar::Real => (vhpiRealVal, vhpiRealVecVal),
        Scalar::Phys { time: true } => (vhpiTimeVal, vhpiTimeVecVal),
        Scalar::Phys { time: false } => (vhpiPhysVal, vhpiPhysVecVal),
    };
    if array { a } else { s }
}

/// `vhpiIntVal` for 6.
pub(crate) fn format_name(f: vhpiFormatT) -> String {
    VHPIFORMATT_VALUES
        .iter()
        .find(|&&(_, v)| v == i64::from(f))
        .map_or_else(|| format!("{f}"), |(n, _)| (*n).to_owned())
}

/// std_ulogic: the type of the logic formats when no object gives one (`vhpi_format_value`).
pub(crate) fn std_logic() -> Scalar {
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

/// STD.CHARACTER as the character formats see it: every position is its byte.
fn character() -> Scalar {
    let lits: Vec<Lit> = (0..=255u8)
        .map(|c| Lit {
            name: format!("'{}'", c as char),
            ch: Some(c),
        })
        .collect();
    Scalar::Enum {
        lits: lits.into(),
        logic: false,
        character: true,
        bit01: false,
    }
}

/// `vhpi_format_value`: the type a value given in `format` stands for without an object, and
/// its element count (`None`: a scalar). Strings of bits, octal and hex digits are logic
/// vectors, decimal strings and the enumeration formats integers, characters and strings
/// CHARACTER values; `None` for a format with no such reading.
pub(crate) fn format_type(
    format: vhpiFormatT,
    given: &Given<'_>,
) -> Option<(Scalar, Option<usize>)> {
    let len = match given {
        Given::Text(t) => t.len(),
        Given::Vector(v) => v.len() / vector_elem_size(format)?,
        Given::Scalar(_) => 1,
    };
    let int = Scalar::Int { narrow: false };
    Some(match format {
        vhpiLogicVal => (std_logic(), None),
        vhpiLogicVecVal | vhpiBinStrVal => (std_logic(), Some(len)),
        vhpiOctStrVal => (std_logic(), Some(3 * len)),
        vhpiHexStrVal => (std_logic(), Some(4 * len)),
        vhpiIntVal | vhpiLongIntVal | vhpiEnumVal | vhpiSmallEnumVal | vhpiDecStrVal => (int, None),
        vhpiIntVecVal | vhpiLongIntVecVal | vhpiEnumVecVal | vhpiSmallEnumVecVal => {
            (int, Some(len))
        }
        vhpiRealVal => (Scalar::Real, None),
        vhpiRealVecVal => (Scalar::Real, Some(len)),
        vhpiTimeVal => (Scalar::Phys { time: true }, None),
        vhpiTimeVecVal => (Scalar::Phys { time: true }, Some(len)),
        vhpiPhysVal => (Scalar::Phys { time: false }, None),
        vhpiPhysVecVal => (Scalar::Phys { time: false }, Some(len)),
        vhpiCharVal => (character(), None),
        vhpiStrVal => (character(), Some(len)),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use yavs_kernel::LogicVec;

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

    /// Puts: what a plugin passes decodes into the kernel value a read gives back.
    #[test]
    fn decode_round_trips_through_the_kernel() {
        let put = |ty: Scalar, n: Option<usize>, f, g: Given<'_>| {
            Read::decode(ty, n, f, g, 1_000_000).map(|r| r.to_kernel())
        };
        // An integer into a logic vector: two's complement, rightmost bit least significant.
        let three = put(
            std_logic(),
            Some(4),
            vhpiIntVal,
            Given::Scalar(Member::Int(3)),
        );
        assert_eq!(three, Ok(Value::Logic("0011".parse().unwrap())));
        let neg = put(
            std_logic(),
            Some(4),
            vhpiIntVal,
            Given::Scalar(Member::Int(-2)),
        );
        assert_eq!(neg, Ok(Value::Logic("1110".parse().unwrap())));
        assert!(
            put(
                std_logic(),
                Some(4),
                vhpiIntVal,
                Given::Scalar(Member::Int(16))
            )
            .is_err()
        );
        assert!(
            put(
                std_logic(),
                Some(4),
                vhpiIntVal,
                Given::Scalar(Member::Int(-9))
            )
            .is_err()
        );
        let want = Value::Logic("01XZ".parse().unwrap());
        let bin = put(std_logic(), Some(4), vhpiBinStrVal, Given::Text(b"01xZ"));
        assert_eq!(bin, Ok(want));
        let codes: Vec<u8> = [2u32, 3, 1, 4]
            .iter()
            .flat_map(|c| c.to_ne_bytes())
            .collect();
        let vec = put(std_logic(), Some(4), vhpiLogicVecVal, Given::Vector(&codes));
        assert_eq!(vec, Ok(Value::Logic("01XZ".parse().unwrap())));
        let hex = put(std_logic(), Some(6), vhpiHexStrVal, Given::Text(b"2A"));
        assert_eq!(hex, Ok(Value::Logic("101010".parse().unwrap())));
        assert!(put(std_logic(), Some(4), vhpiHexStrVal, Given::Text(b"1F")).is_err());
        assert!(put(std_logic(), Some(4), vhpiBinStrVal, Given::Text(b"010")).is_err());
        let one = put(bit(), None, vhpiLogicVal, Given::Scalar(Member::Int(1)));
        assert_eq!(one, Ok(Value::Logic("1".parse().unwrap())));
        assert!(put(bit(), None, vhpiLogicVal, Given::Scalar(Member::Int(2))).is_err());
        let int = put(
            Scalar::Int { narrow: false },
            None,
            vhpiDecStrVal,
            Given::Text(b"-9"),
        );
        assert_eq!(int, Ok(Value::Bytes((-9i64).to_le_bytes().to_vec())));
        // TIME counts in the resolution limit (here ns).
        let t = put(
            Scalar::Phys { time: true },
            None,
            vhpiTimeVal,
            Given::Scalar(Member::Time(3)),
        );
        assert_eq!(t, Ok(Value::Bytes(3_000_000i64.to_le_bytes().to_vec())));
        let r = put(
            Scalar::Int { narrow: false },
            None,
            vhpiRealVal,
            Given::Scalar(Member::Real(1.0)),
        );
        assert!(matches!(r, Err(Error::Mismatch(_))));
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
            ty: Scalar::Int { narrow: false },
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
        // vhpiEnumVal reads a non-negative integer (#318).
        assert!(matches!(i.encode(vhpiEnumVal, 1), Err(Error::Mismatch(_))));
        let Encoded::Scalar(u) = Read {
            cells: Cells::Int(vec![17]),
            ..i.clone()
        }
        .encode(vhpiEnumVal, 1)
        .unwrap() else {
            panic!()
        };
        assert_eq!(unsafe_free_int(u), 17); // enumv shares the int field's bytes
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
        let iv = Read::from_const(Scalar::Int { narrow: false }, &arr).unwrap();
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
