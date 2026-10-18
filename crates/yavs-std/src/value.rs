//! `T'VALUE`: parse the string representation of a scalar (LRM 16.2.2).
//!
//! The string is one literal of the type, optionally with leading and
//! trailing whitespace, using the full lexical rules (LRM 15.5): underscores
//! between digits, based literals (`16#FF#`, `2#1.1#E2`, with `:` as the
//! replacement for `#`), exponents, physical literals with any unit of the
//! type, and enumeration literals matched case-insensitively except for
//! extended identifiers and character literals. Out-of-range values are
//! errors; nothing is clamped.

use crate::error::{ConvError, ErrorKind, What};
use crate::types::{Bounds, EnumLit, EnumType, PhysicalType, RealBounds, is_letter, lower};

/// Whitespace accepted around a value: space, non-breaking space and the
/// format effectors HT, LF, VT, FF, CR.
pub fn is_ws(c: u8) -> bool {
    matches!(c, b' ' | 0xA0 | b'\t' | b'\n' | 0x0B | 0x0C | b'\r')
}

/// An abstract literal split into its parts, before conversion.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Abstract {
    pub base: u32,
    /// All digit values, integer part then fraction part.
    pub digits: Vec<u8>,
    /// How many of `digits` are after the point.
    pub frac_len: usize,
    pub exp: i64,
    /// Has a point, so it is a real literal.
    pub real: bool,
}

type R<T> = Result<T, ConvError>;

fn syntax(what: What, s: &[u8], at: usize, expected: &'static str) -> ConvError {
    ConvError::new(what, ErrorKind::Syntax { expected }, s, at..at + 1)
}

/// Lex `digit { [_] digit }` in `base` starting at `i`. Returns the digit
/// values and the end position.
fn lex_digits(s: &[u8], mut i: usize, base: u32, what: What) -> R<(Vec<u8>, usize)> {
    let digit = |c: u8| (c as char).to_digit(16).filter(|&d| d < base);
    let mut out = Vec::new();
    match s.get(i).and_then(|&c| digit(c)) {
        Some(d) => out.push(d as u8),
        None => {
            return Err(syntax(
                what,
                s,
                i,
                if base == 10 {
                    "a digit"
                } else {
                    "a digit of the base"
                },
            ));
        }
    }
    i += 1;
    loop {
        match s.get(i) {
            Some(b'_') => match s.get(i + 1).and_then(|&c| digit(c)) {
                Some(d) => {
                    out.push(d as u8);
                    i += 2;
                }
                None => return Err(syntax(what, s, i + 1, "a digit after '_'")),
            },
            Some(&c) if digit(c).is_some() => {
                out.push(digit(c).unwrap_or(0) as u8);
                i += 1;
            }
            Some(&c) if c.is_ascii_hexdigit() && base > 10 || c.is_ascii_digit() => {
                return Err(syntax(what, s, i, "a digit of the base"));
            }
            _ => return Ok((out, i)),
        }
    }
}

/// Lex an abstract literal (LRM 15.5) at `i`.
pub fn lex_abstract(s: &[u8], i: usize, what: What) -> R<(Abstract, usize)> {
    let (int, mut j) = lex_digits(s, i, 10, what)?;
    let mut lit = Abstract {
        base: 10,
        digits: int,
        frac_len: 0,
        exp: 0,
        real: false,
    };
    if let Some(&mark @ (b'#' | b':')) = s.get(j) {
        let base = lit
            .digits
            .iter()
            .fold(0u64, |a, &d| a.saturating_mul(10).saturating_add(d as u64));
        if !(2..=16).contains(&base) {
            return Err(ConvError::new(
                what,
                ErrorKind::Syntax {
                    expected: "a base from 2 to 16",
                },
                s,
                i..j,
            ));
        }
        lit.base = base as u32;
        let (d, k) = lex_digits(s, j + 1, lit.base, what)?;
        lit.digits = d;
        j = k;
        if s.get(j) == Some(&b'.') {
            let (f, k) = lex_digits(s, j + 1, lit.base, what)?;
            lit.frac_len = f.len();
            lit.digits.extend(f);
            lit.real = true;
            j = k;
        }
        if s.get(j) != Some(&mark) {
            return Err(syntax(
                what,
                s,
                j,
                if mark == b'#' {
                    "'#' closing the based literal"
                } else {
                    "':' closing the based literal"
                },
            ));
        }
        j += 1;
    } else if s.get(j) == Some(&b'.') && s.get(j + 1).is_some_and(u8::is_ascii_digit) {
        let (f, k) = lex_digits(s, j + 1, 10, what)?;
        lit.frac_len = f.len();
        lit.digits.extend(f);
        lit.real = true;
        j = k;
    }
    // Exponent: only when E is followed by a digit or a signed digit, so a
    // unit such as "em" directly after the number still lexes as a unit.
    if matches!(s.get(j), Some(b'e' | b'E')) {
        let (neg, k) = match s.get(j + 1) {
            Some(b'+') => (false, j + 2),
            Some(b'-') => (true, j + 2),
            _ => (false, j + 1),
        };
        if s.get(k).is_some_and(u8::is_ascii_digit) {
            let (e, k) = lex_digits(s, k, 10, what)?;
            let mut v: i64 = 0;
            for d in e {
                v = v
                    .checked_mul(10)
                    .and_then(|v| v.checked_add(d as i64))
                    .ok_or_else(|| ConvError::new(what, ErrorKind::Overflow, s, j..k))?;
            }
            lit.exp = if neg { -v } else { v };
            j = k;
        } else if k > j + 1 {
            return Err(syntax(what, s, k, "exponent digits"));
        }
    }
    Ok((lit, j))
}

impl Abstract {
    /// The value of an integer literal, exactly.
    pub fn to_i128(&self) -> Option<i128> {
        if self.real || self.exp < 0 {
            return None;
        }
        let mut v: i128 = 0;
        for &d in &self.digits {
            v = v.checked_mul(self.base as i128)?.checked_add(d as i128)?;
        }
        for _ in 0..self.exp {
            if v == 0 {
                break;
            }
            v = v.checked_mul(self.base as i128)?;
        }
        Some(v)
    }

    /// The value as the nearest f64 (exact rounding for base 10; for other
    /// bases exact while the mantissa fits 53 bits).
    pub fn to_f64(&self) -> f64 {
        let e = self.exp.saturating_sub(self.frac_len as i64);
        if self.base == 10 {
            let mut t: String = self.digits.iter().map(|d| char::from(b'0' + d)).collect();
            t.push_str(&format!("e{e}"));
            t.parse().unwrap_or(f64::NAN)
        } else {
            // ponytail: f64 accumulation, inexact past 2^53 or for bases that
            // are not powers of two; a big-rational path if it ever matters.
            let m = self
                .digits
                .iter()
                .fold(0f64, |a, &d| a * self.base as f64 + d as f64);
            let e = i32::try_from(e).unwrap_or(if e < 0 { i32::MIN } else { i32::MAX });
            m * (self.base as f64).powi(e)
        }
    }

    /// The literal times `scale`, rounded to the nearest integer (ties away
    /// from zero), as an exact computation; `None` on overflow.
    pub fn scaled(&self, scale: i64) -> Option<i128> {
        let base = self.base as u128;
        let mut num: u128 = 0;
        for &d in &self.digits {
            num = num.checked_mul(base)?.checked_add(d as u128)?;
        }
        num = num.checked_mul(scale.unsigned_abs() as u128)?;
        let e = self.exp.checked_sub(self.frac_len as i64)?;
        let mut den: u128 = 1;
        for _ in 0..e.unsigned_abs() {
            if e > 0 {
                if num == 0 {
                    break;
                }
                num = num.checked_mul(base)?;
            } else {
                match den.checked_mul(base) {
                    Some(d) => den = d,
                    // Denominator larger than any numerator: the value is 0.
                    None => return Some(0),
                }
            }
        }
        let q = num / den;
        let r = num % den;
        let q = if r >= den - r { q + 1 } else { q };
        i128::try_from(q).ok()
    }
}

fn skip_ws(s: &[u8], mut i: usize) -> usize {
    while s.get(i).copied().is_some_and(is_ws) {
        i += 1;
    }
    i
}

/// Trim and check emptiness; returns the first non-blank index.
fn start(s: &[u8], what: What) -> R<usize> {
    let i = skip_ws(s, 0);
    if i == s.len() {
        return Err(ConvError::new(what, ErrorKind::Empty, s, 0..s.len()));
    }
    Ok(i)
}

fn finish(s: &[u8], i: usize, what: What) -> R<()> {
    let j = skip_ws(s, i);
    if j < s.len() {
        let end = s.len() - s.iter().rev().take_while(|&&c| is_ws(c)).count();
        return Err(ConvError::new(
            what,
            ErrorKind::TrailingText,
            s,
            j..end.max(j + 1),
        ));
    }
    Ok(())
}

fn sign(s: &[u8], i: usize) -> (bool, usize) {
    match s.get(i) {
        Some(b'-') => (true, i + 1),
        Some(b'+') => (false, i + 1),
        _ => (false, i),
    }
}

fn out_of_range(
    what: What,
    s: &[u8],
    span: std::ops::Range<usize>,
    low: String,
    high: String,
) -> ConvError {
    ConvError::new(what, ErrorKind::OutOfRange { low, high }, s, span)
}

/// `integer'value` (or any integer type with `bounds`).
pub fn integer_value(s: &[u8], bounds: Bounds) -> R<i64> {
    let what = What::Integer;
    let b = start(s, what)?;
    let (neg, i) = sign(s, b);
    let (lit, j) = lex_abstract(s, i, what)?;
    if lit.real || lit.exp < 0 {
        return Err(ConvError::new(
            what,
            ErrorKind::Syntax {
                expected: "an integer literal (no point, no negative exponent)",
            },
            s,
            i..j,
        ));
    }
    finish(s, j, what)?;
    let v = lit
        .to_i128()
        .map(|v| if neg { -v } else { v })
        .ok_or_else(|| ConvError::new(what, ErrorKind::Overflow, s, b..j))?;
    if v < bounds.low as i128 || v > bounds.high as i128 {
        return Err(out_of_range(
            what,
            s,
            b..j,
            bounds.low.to_string(),
            bounds.high.to_string(),
        ));
    }
    Ok(v as i64)
}

/// `real'value`. An integer literal is accepted as a real, as NVC and GHDL do.
pub fn real_value(s: &[u8], bounds: RealBounds) -> R<f64> {
    let what = What::Real;
    let b = start(s, what)?;
    let (neg, i) = sign(s, b);
    let (lit, j) = lex_abstract(s, i, what)?;
    finish(s, j, what)?;
    let v = lit.to_f64();
    if !v.is_finite() {
        return Err(ConvError::new(what, ErrorKind::Overflow, s, b..j));
    }
    let v = if neg { -v } else { v };
    if v < bounds.low || v > bounds.high {
        return Err(out_of_range(
            what,
            s,
            b..j,
            String::from_utf8_lossy(&crate::image::real_image(bounds.low)).into_owned(),
            String::from_utf8_lossy(&crate::image::real_image(bounds.high)).into_owned(),
        ));
    }
    Ok(v)
}

/// A lexed identifier or character literal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Token {
    Basic,
    Extended,
    Char(u8),
}

/// Lex an identifier or character literal at `i`.
pub fn lex_name(s: &[u8], i: usize, what: What) -> R<(Token, usize)> {
    match s.get(i) {
        Some(&c) if is_letter(c) => {
            let mut j = i + 1;
            loop {
                match s.get(j) {
                    Some(&c) if is_letter(c) || c.is_ascii_digit() => j += 1,
                    Some(b'_')
                        if s.get(j + 1)
                            .is_some_and(|&c| is_letter(c) || c.is_ascii_digit()) =>
                    {
                        j += 2
                    }
                    Some(b'_') => {
                        return Err(syntax(what, s, j + 1, "a letter or digit after '_'"));
                    }
                    _ => return Ok((Token::Basic, j)),
                }
            }
        }
        Some(b'\\') => {
            let mut j = i + 1;
            loop {
                match s.get(j) {
                    Some(b'\\') if s.get(j + 1) == Some(&b'\\') => j += 2,
                    Some(b'\\') => return Ok((Token::Extended, j + 1)),
                    Some(&c) if is_graphic(c) => j += 1,
                    _ => return Err(syntax(what, s, j, "'\\' closing the extended identifier")),
                }
            }
        }
        Some(b'\'') => match (s.get(i + 1), s.get(i + 2)) {
            (Some(&c), Some(b'\'')) if is_graphic(c) => Ok((Token::Char(c), i + 3)),
            _ => Err(syntax(what, s, i, "a character literal such as 'a'")),
        },
        _ => Err(syntax(what, s, i, "an identifier or character literal")),
    }
}

/// Graphic characters (LRM 15.2): everything except the control
/// characters 0..31, 127 and 128..159.
pub fn is_graphic(c: u8) -> bool {
    !(c < 32 || (127..160).contains(&c))
}

/// Does the lexed name `s[a..b]` of kind `tok` denote the literal `lit`?
pub fn name_matches(s: &[u8], tok: Token, lit: &EnumLit) -> bool {
    match (tok, lit) {
        (Token::Char(c), EnumLit::Char(l)) => c == *l,
        (Token::Basic, EnumLit::Ident(n)) => {
            n.first() != Some(&b'\\')
                && n.len() == s.len()
                && n.iter().zip(s).all(|(&a, &b)| lower(a) == lower(b))
        }
        (Token::Extended, EnumLit::Ident(n)) => n.as_slice() == s,
        _ => false,
    }
}

/// `T'value` for an enumeration type; returns the position of the literal.
pub fn enum_value(s: &[u8], ty: &EnumType) -> R<usize> {
    let what = What::Enumeration;
    let b = start(s, what)?;
    let (tok, j) = lex_name(s, b, what)?;
    finish(s, j, what)?;
    ty.lits
        .iter()
        .position(|l| name_matches(&s[b..j], tok, l))
        .ok_or_else(|| ConvError::new(what, ErrorKind::UnknownLiteral, s, b..j))
}

/// `T'value` for a physical type, in primary units: `"1.5 ns"`, `"ns"`,
/// `"16#A# ps"`. A decimal value is rounded to the primary unit.
pub fn physical_value(s: &[u8], ty: &PhysicalType, bounds: Bounds) -> R<i64> {
    let what = What::Physical;
    let b = start(s, what)?;
    let (neg, i) = sign(s, b);
    let (lit, j) = if s.get(i).is_some_and(u8::is_ascii_digit) {
        let (l, j) = lex_abstract(s, i, what)?;
        (Some(l), skip_ws(s, j))
    } else {
        (None, i)
    };
    let (tok, k) = lex_name(s, j, What::Unit)?;
    if matches!(tok, Token::Char(_)) {
        return Err(syntax(what, s, j, "a unit name"));
    }
    finish(s, k, what)?;
    let unit = ty
        .units
        .iter()
        .find(|u| name_matches(&s[j..k], tok, &EnumLit::Ident(u.name.clone())))
        .ok_or_else(|| ConvError::new(What::Unit, ErrorKind::UnknownUnit, s, j..k))?;
    let mag = match &lit {
        Some(l) => l.scaled(unit.value),
        None => Some(unit.value.unsigned_abs() as i128),
    }
    .ok_or_else(|| ConvError::new(what, ErrorKind::Overflow, s, b..k))?;
    let v = if neg != (unit.value < 0) { -mag } else { mag };
    if v < bounds.low as i128 || v > bounds.high as i128 {
        let img = |v| String::from_utf8_lossy(&crate::image::physical_image(v, ty)).into_owned();
        return Err(out_of_range(
            what,
            s,
            b..k,
            img(bounds.low),
            img(bounds.high),
        ));
    }
    Ok(v as i64)
}

/// `boolean'value`.
pub fn boolean_value(s: &[u8]) -> R<bool> {
    enum_value(s, &crate::types::boolean_type()).map(|p| p == 1)
}

/// `character'value`; returns the character.
pub fn character_value(s: &[u8]) -> R<u8> {
    enum_value(s, &crate::types::character_type()).map(|p| p as u8)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::time_type;

    #[test]
    fn integers() {
        let v = |s: &str| integer_value(s.as_bytes(), Bounds::I64);
        assert_eq!(v(" 16#FF# "), Ok(255));
        assert_eq!(v("2#1010#E3"), Ok(80));
        assert_eq!(v("16:ff:"), Ok(255));
        assert_eq!(v("1_000"), Ok(1000));
        assert_eq!(v("1E3"), Ok(1000));
        assert_eq!(v("-9223372036854775808"), Ok(i64::MIN));
        assert!(v("9223372036854775808").is_err());
        for bad in [
            "", "1__0", "_1", "1_", "1e-3", "16#FF", "16#G#", "17#1#", "1.0", "- 5", "5 x", "16#F:",
        ] {
            assert!(v(bad).is_err(), "{bad}");
        }
        let e = integer_value(b"2147483648", Bounds::I32).unwrap_err();
        assert!(matches!(e.kind, ErrorKind::OutOfRange { .. }));
        assert_eq!(
            e.to_string(),
            "integer value \"2147483648\" is outside the range -2147483648 to 2147483647"
        );
    }

    #[test]
    fn physicals() {
        let t = time_type();
        let v = |s: &str| physical_value(s.as_bytes(), &t, Bounds::I64);
        assert_eq!(v("1.5 ns"), Ok(1_500_000));
        assert_eq!(v("ns"), Ok(1_000_000));
        assert_eq!(v("5ns"), Ok(5_000_000));
        assert_eq!(v("2.5 fs"), Ok(3));
        assert_eq!(v("-2.5 fs"), Ok(-3));
        assert_eq!(v("1.0e-30 hr"), Ok(0));
        assert_eq!(v("16#A# NS"), Ok(10_000_000));
        assert!(v("10000000 hr").is_err());
        assert!(v("5 xs").is_err());
        assert!(v("5").is_err());
    }
}
