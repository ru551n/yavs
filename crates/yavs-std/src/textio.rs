//! std.textio READ and WRITE as operations on a line buffer.
//!
//! A LINE is Latin-1 bytes. READ consumes from the front of the line (a
//! slice it advances); WRITE appends to a `Vec<u8>`. The access-type plumbing (null lines, deallocation)
//! belongs to the runtime.
//!
//! Every READ returns `Result`: the GOOD form of the procedure maps `Err` to
//! `good = false`, the form without GOOD reports the error. On failure the
//! line is left unchanged, so a caller can retry with another type.

use crate::error::{ConvError, ErrorKind, What};
use crate::image;
use crate::printf;
use crate::types::{Bounds, PhysicalType, RealBounds, lower};
use crate::value::{Abstract, is_ws};

/// textio SIDE.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Right,
    Left,
}

/// `justify(value, justified, field)`: pad with spaces to `field` columns.
pub fn justify(value: &[u8], side: Side, field: usize) -> Vec<u8> {
    let pad = field.saturating_sub(value.len());
    let mut out = Vec::with_capacity(value.len() + pad);
    if side == Side::Right {
        out.resize(pad, b' ');
    }
    out.extend_from_slice(value);
    if side == Side::Left {
        out.resize(out.len() + pad, b' ');
    }
    out
}

fn put(line: &mut Vec<u8>, value: &[u8], side: Side, field: usize) {
    let pad = field.saturating_sub(value.len());
    if side == Side::Right {
        line.resize(line.len() + pad, b' ');
    }
    line.extend_from_slice(value);
    if side == Side::Left {
        line.resize(line.len() + pad, b' ');
    }
}

pub fn write_bit(line: &mut Vec<u8>, v: bool, side: Side, field: usize) {
    put(line, if v { b"1" } else { b"0" }, side, field);
}

pub fn write_bit_vector(line: &mut Vec<u8>, bits: &[bool], side: Side, field: usize) {
    put(line, &image::bits_to_string(bits), side, field);
}

pub fn write_boolean(line: &mut Vec<u8>, v: bool, side: Side, field: usize) {
    put(line, if v { b"true" } else { b"false" }, side, field);
}

pub fn write_character(line: &mut Vec<u8>, c: u8, side: Side, field: usize) {
    put(line, &[c], side, field);
}

pub fn write_integer(line: &mut Vec<u8>, v: i64, side: Side, field: usize) {
    put(line, image::integer_digits(v, &mut [0; 20]), side, field);
}

/// `write(l, real, justified, field, digits)`: `digits = 0` writes `%e`
/// (exponent form), otherwise fixed point with `digits` decimals.
pub fn write_real(
    line: &mut Vec<u8>,
    x: f64,
    side: Side,
    field: usize,
    digits: usize,
) -> Result<(), ConvError> {
    let s = if digits == 0 {
        printf::e(x, 6)
    } else {
        image::real_to_string_digits(x, digits)?
    };
    put(line, &s, side, field);
    Ok(())
}

/// `write(l, real, format)`.
pub fn write_real_format(line: &mut Vec<u8>, x: f64, format: &[u8]) -> Result<(), ConvError> {
    line.extend(printf::format_real(format, x)?);
    Ok(())
}

pub fn write_string(line: &mut Vec<u8>, s: &[u8], side: Side, field: usize) {
    put(line, s, side, field);
}

/// `write(l, time, justified, field, unit)`; `unit` must be a unit value.
pub fn write_physical(
    line: &mut Vec<u8>,
    v: i64,
    ty: &PhysicalType,
    side: Side,
    field: usize,
    unit: i64,
) -> Result<(), ConvError> {
    put(line, &image::physical_write_unit(v, ty, unit)?, side, field);
    Ok(())
}

pub fn hwrite(line: &mut Vec<u8>, bits: &[bool], side: Side, field: usize) {
    put(line, &image::bits_to_hstring(bits), side, field);
}

pub fn owrite(line: &mut Vec<u8>, bits: &[bool], side: Side, field: usize) {
    put(line, &image::bits_to_ostring(bits), side, field);
}

// ------------------------------------------------------------------ reads

type R<T> = Result<T, ConvError>;

fn skip(l: &[u8]) -> usize {
    l.iter().take_while(|&&c| is_ws(c)).count()
}

fn err(what: What, l: &[u8], at: usize, expected: &'static str) -> ConvError {
    let kind = if at >= l.len() {
        ErrorKind::Empty
    } else {
        ErrorKind::Syntax { expected }
    };
    ConvError::new(what, kind, l, at..at + 1)
}

/// Consume `n` bytes from the front of the line.
fn consume(line: &mut &[u8], n: usize) {
    *line = &line[n..];
}

fn digits_end(l: &[u8], i: usize) -> usize {
    i + l[i..].iter().take_while(|c| c.is_ascii_digit()).count()
}

/// The READ number grammar: `[+|-] digits [. digits] [E [+|-] digits]`
/// without underscores or bases. With `allow_real = false` only the
/// integer part is read.
fn read_number(l: &[u8], what: What, allow_real: bool) -> R<(bool, Abstract, usize)> {
    let mut i = skip(l);
    let neg = match l.get(i) {
        Some(b'-') => {
            i += 1;
            true
        }
        Some(b'+') => {
            i += 1;
            false
        }
        _ => false,
    };
    let j = digits_end(l, i);
    if j == i {
        return Err(err(what, l, i, "a digit"));
    }
    let mut lit = Abstract {
        base: 10,
        digits: l[i..j].iter().map(|c| c - b'0').collect(),
        frac_len: 0,
        exp: 0,
        real: false,
    };
    let mut i = j;
    if allow_real {
        if l.get(i) == Some(&b'.') {
            let j = digits_end(l, i + 1);
            if j == i + 1 {
                return Err(err(what, l, i + 1, "a digit after '.'"));
            }
            lit.digits.extend(l[i + 1..j].iter().map(|c| c - b'0'));
            lit.frac_len = j - i - 1;
            lit.real = true;
            i = j;
        }
        if matches!(l.get(i), Some(b'e' | b'E')) {
            let (eneg, k) = match l.get(i + 1) {
                Some(b'-') => (true, i + 2),
                Some(b'+') => (false, i + 2),
                _ => (false, i + 1),
            };
            let j = digits_end(l, k);
            if j == k {
                return Err(err(what, l, k, "exponent digits"));
            }
            let e: i64 = std::str::from_utf8(&l[k..j])
                .ok()
                .and_then(|s| s.parse().ok())
                .ok_or_else(|| ConvError::new(what, ErrorKind::Overflow, l, i..j))?;
            lit.exp = if eneg { -e } else { e };
            i = j;
        }
    }
    Ok((neg, lit, i))
}

/// `read(l, integer)`.
pub fn read_integer(line: &mut &[u8], bounds: Bounds) -> R<i64> {
    let (neg, lit, end) = read_number(line, What::Integer, false)?;
    let span = skip(line)..end;
    let v = lit
        .to_i128()
        .map(|v| if neg { -v } else { v })
        .ok_or_else(|| ConvError::new(What::Integer, ErrorKind::Overflow, line, span.clone()))?;
    if v < bounds.low as i128 || v > bounds.high as i128 {
        return Err(ConvError::new(
            What::Integer,
            ErrorKind::OutOfRange {
                low: bounds.low.to_string(),
                high: bounds.high.to_string(),
            },
            line,
            span,
        ));
    }
    consume(line, end);
    Ok(v as i64)
}

/// `read(l, real)`. An integer such as `1` or `1e5` is accepted.
pub fn read_real(line: &mut &[u8], bounds: RealBounds) -> R<f64> {
    let (neg, lit, end) = read_number(line, What::Real, true)?;
    let span = skip(line)..end;
    let v = lit.to_f64();
    if !v.is_finite() {
        return Err(ConvError::new(What::Real, ErrorKind::Overflow, line, span));
    }
    let v = if neg { -v } else { v };
    if v < bounds.low || v > bounds.high {
        return Err(ConvError::new(
            What::Real,
            ErrorKind::OutOfRange {
                low: String::from_utf8_lossy(&image::real_image(bounds.low)).into_owned(),
                high: String::from_utf8_lossy(&image::real_image(bounds.high)).into_owned(),
            },
            line,
            span,
        ));
    }
    consume(line, end);
    Ok(v)
}

/// `read(l, time)` (any physical type): a number, optional whitespace and
/// a unit. The unit is the longest unit name that prefixes the remaining
/// text, case-insensitively, so `"5 nsx"` reads 5 ns and leaves `x`.
pub fn read_physical(line: &mut &[u8], ty: &PhysicalType, bounds: Bounds) -> R<i64> {
    let what = What::Physical;
    let (neg, lit, end) = read_number(line, what, true)?;
    let span_start = skip(line);
    let u0 = end + skip(&line[end..]);
    let rest = &line[u0..];
    let unit = ty
        .units
        .iter()
        .filter(|u| {
            u.name.first() != Some(&b'\\')
                && rest.len() >= u.name.len()
                && u.name.iter().zip(rest).all(|(&a, &b)| lower(a) == lower(b))
        })
        .max_by_key(|u| u.name.len())
        .ok_or_else(|| {
            let e = u0
                + rest
                    .iter()
                    .take_while(|c| c.is_ascii_alphanumeric() || **c == b'_')
                    .count();
            if rest.is_empty() {
                err(What::Unit, line, u0, "a unit name")
            } else {
                ConvError::new(What::Unit, ErrorKind::UnknownUnit, line, u0..e.max(u0 + 1))
            }
        })?;
    let stop = u0 + unit.name.len();
    let span = span_start..stop;
    let mag = lit
        .scaled(unit.value)
        .ok_or_else(|| ConvError::new(what, ErrorKind::Overflow, line, span.clone()))?;
    let v = if neg != (unit.value < 0) { -mag } else { mag };
    if v < bounds.low as i128 || v > bounds.high as i128 {
        let img = |v| String::from_utf8_lossy(&image::physical_image(v, ty)).into_owned();
        return Err(ConvError::new(
            what,
            ErrorKind::OutOfRange {
                low: img(bounds.low),
                high: img(bounds.high),
            },
            line,
            span,
        ));
    }
    consume(line, stop);
    Ok(v as i64)
}

/// `read(l, boolean)`: `true` or `false` in any case.
pub fn read_boolean(line: &mut &[u8]) -> R<bool> {
    let i = skip(line);
    let rest = &line[i..];
    for (word, v) in [(&b"true"[..], true), (&b"false"[..], false)] {
        if rest.len() >= word.len() && rest.iter().zip(word).all(|(&a, &b)| lower(a) == b) {
            consume(line, i + word.len());
            return Ok(v);
        }
    }
    Err(err(What::Enumeration, line, i, "TRUE or FALSE"))
}

/// `read(l, bit)`.
pub fn read_bit(line: &mut &[u8]) -> R<bool> {
    let i = skip(line);
    match line.get(i) {
        Some(&c @ (b'0' | b'1')) => {
            consume(line, i + 1);
            Ok(c == b'1')
        }
        _ => Err(err(What::BitVector, line, i, "'0' or '1'")),
    }
}

/// `read(l, character)`: the next character, whitespace included.
pub fn read_character(line: &mut &[u8]) -> R<u8> {
    match line.first() {
        Some(&c) => {
            consume(line, 1);
            Ok(c)
        }
        None => Err(ConvError::new(
            What::String,
            ErrorKind::TooShort {
                expected: 1,
                found: 0,
            },
            line,
            0..0,
        )),
    }
}

/// `read(l, string)` for a string of length `n`: exactly the next `n`
/// characters, whitespace included.
pub fn read_string(line: &mut &[u8], n: usize) -> R<Vec<u8>> {
    if line.len() < n {
        return Err(ConvError::new(
            What::String,
            ErrorKind::TooShort {
                expected: n,
                found: line.len(),
            },
            line,
            0..line.len(),
        ));
    }
    let v = line[..n].to_vec();
    consume(line, n);
    Ok(v)
}

/// `sread(l, value, strlen)`: skip whitespace, then read up to `n`
/// non-whitespace characters. Never fails; the result may be shorter.
pub fn sread(line: &mut &[u8], n: usize) -> Vec<u8> {
    if n == 0 {
        return Vec::new();
    }
    let i = skip(line);
    let len = line[i..].iter().take(n).take_while(|&&c| !is_ws(c)).count();
    let v = line[i..i + len].to_vec();
    consume(line, i + len);
    v
}

/// `read(l, bit_vector)` (and BREAD) of length `n`: `n` binary digits, white space allowed
/// before each (as a READ of each BIT would; LRM 16.4 is silent, L131), without underscores.
pub fn read_bit_vector(line: &mut &[u8], n: usize) -> R<Vec<bool>> {
    let mut v = Vec::with_capacity(n);
    let mut i = 0;
    for k in 0..n {
        i += line[i..].iter().take_while(|&&c| is_ws(c)).count();
        match line.get(i) {
            Some(&c @ (b'0' | b'1')) => v.push(c == b'1'),
            Some(_) => return Err(err(What::BitVector, line, i, "'0' or '1'")),
            None => {
                return Err(ConvError::new(
                    What::BitVector,
                    ErrorKind::TooShort {
                        expected: n,
                        found: k,
                    },
                    line,
                    0..i,
                ));
            }
        }
        i += 1;
    }
    consume(line, i);
    Ok(v)
}

fn read_radix(line: &mut &[u8], n: usize, bits_per: usize) -> R<Vec<bool>> {
    if n == 0 {
        return Ok(Vec::new());
    }
    let what = What::BitVector;
    let ndig = n.div_ceil(bits_per);
    let radix = 1u32 << bits_per;
    let i = skip(line);
    let mut j = i;
    let mut bits = Vec::with_capacity(ndig * bits_per);
    for k in 0..ndig {
        if k > 0 && line.get(j) == Some(&b'_') {
            j += 1;
        }
        let d = line
            .get(j)
            .and_then(|&c| (c as char).to_digit(radix))
            .ok_or_else(|| match line.get(j) {
                None => ConvError::new(
                    what,
                    ErrorKind::TooShort {
                        expected: ndig,
                        found: k,
                    },
                    line,
                    i..j,
                ),
                Some(_) => err(
                    what,
                    line,
                    j,
                    if bits_per == 4 {
                        "a hexadecimal digit"
                    } else {
                        "an octal digit"
                    },
                ),
            })?;
        j += 1;
        bits.extend((0..bits_per).rev().map(|b| d >> b & 1 == 1));
    }
    let extra = bits.len() - n;
    if bits[..extra].iter().any(|&b| b) {
        return Err(ConvError::new(what, ErrorKind::DoesNotFit, line, i..j));
    }
    consume(line, j);
    Ok(bits.split_off(extra))
}

/// `hread(l, bit_vector)` of length `n`: `ceil(n/4)` hex digits, single
/// underscores allowed between digits; the unused high bits must be zero.
pub fn hread(line: &mut &[u8], n: usize) -> R<Vec<bool>> {
    read_radix(line, n, 4)
}

/// `oread(l, bit_vector)` of length `n`.
pub fn oread(line: &mut &[u8], n: usize) -> R<Vec<bool>> {
    read_radix(line, n, 3)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn failure_leaves_line() {
        let mut l: &[u8] = b"  1.5e";
        assert!(read_real(&mut l, RealBounds::REAL).is_err());
        assert_eq!(l, b"  1.5e");
        let mut l: &[u8] = b"A__5";
        assert!(hread(&mut l, 8).is_err());
        assert_eq!(l, b"A__5");
        let mut l: &[u8] = b"3F";
        let e = hread(&mut l, 5).unwrap_err();
        assert_eq!(e.kind, ErrorKind::DoesNotFit);
    }
}
