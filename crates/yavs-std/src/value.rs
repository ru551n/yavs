//! `T'VALUE`: parse the string representation of a scalar (LRM 16.2.2).
//!
//! The string is one literal of the type, optionally with leading and
//! trailing whitespace, using the full lexical rules (LRM 15.5): underscores
//! between digits, based literals (`16#FF#`, `2#1.1#E2`, with `:` as the
//! replacement for `#`), exponents, physical literals with any unit of the
//! type, and enumeration literals matched case-insensitively except for
//! extended identifiers and character literals. Out-of-range values are
//! errors; nothing is clamped.

use crate::error::{ConvError, ErrorKind, What, latin1};
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

    /// The value as the nearest f64, correctly rounded (ties to even) in every base.
    pub fn to_f64(&self) -> f64 {
        let e = self.exp.saturating_sub(self.frac_len as i64);
        if self.base == 10 {
            let mut t: String = self.digits.iter().map(|d| char::from(b'0' + d)).collect();
            t.push_str(&format!("e{e}"));
            t.parse().unwrap_or(f64::NAN)
        } else {
            based_to_f64(self.base, &self.digits, e)
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

/// `digits` (in `base`) times `base^e`, correctly rounded to f64 (ties to even), through exact
/// big-integer arithmetic: `num / den` scaled by `2^s` to 54 bits (53 plus a guard bit), the
/// remainder as the sticky bit, then one rounding step. Subnormals round at 2^-1074.
fn based_to_f64(base: u32, digits: &[u8], e: i64) -> f64 {
    let digits = &digits[digits.iter().take_while(|&&d| d == 0).count()..];
    if digits.is_empty() {
        return 0.0;
    }
    // base^(mag - 1) <= value < base^mag: decide overflow and underflow before building
    // big numbers, which also bounds `e` below.
    let mag = (digits.len() as i64).saturating_add(e);
    let lb = f64::from(base).log2();
    if (mag - 1) as f64 * lb > 1025.0 {
        return f64::INFINITY;
    }
    if mag as f64 * lb < -1080.0 {
        return 0.0;
    }
    let mut num = vec![0u32];
    for &d in digits {
        mul_add(&mut num, base, u32::from(d));
    }
    let mut den = vec![1u32];
    for _ in 0..e.unsigned_abs() {
        mul_add(if e > 0 { &mut num } else { &mut den }, base, 0);
    }
    // num / den lies in [2^(k-1), 2^(k+1)) for k = bitlen(num) - bitlen(den).
    let k = bit_len(&num) as i64 - bit_len(&den) as i64;
    let mut s = 53 - k;
    if cmp_shifted(&num, s, &den, 53) == std::cmp::Ordering::Less {
        s += 1;
    }
    s = s.min(1075);
    let (mut n, d) = if s >= 0 {
        (shl(&num, s as usize), den)
    } else {
        (num, shl(&den, s.unsigned_abs() as usize))
    };
    let mut q = 0u64;
    for i in (0..=54).rev() {
        let t = shl(&d, i);
        if cmp(&n, &t) != std::cmp::Ordering::Less {
            sub(&mut n, &t);
            q |= 1 << i;
        }
    }
    let sticky = n.iter().any(|&w| w != 0);
    let mut m = q >> 1;
    if q & 1 == 1 && (sticky || m & 1 == 1) {
        m += 1;
    }
    ldexp(m, 1 - s)
}

/// `m * 2^e` for `m <= 2^53`; exact unless it overflows (to infinity).
fn ldexp(m: u64, mut e: i64) -> f64 {
    let pow2 = |k: i64| f64::from_bits(((k + 1023) as u64) << 52);
    let mut x = m as f64;
    while e > 1000 {
        x *= pow2(1000);
        e -= 1000;
    }
    while e < -1000 {
        x *= pow2(-1000);
        e += 1000;
    }
    x * pow2(e)
}

// Little-endian base-2^32 naturals, just enough for `based_to_f64`.

fn mul_add(a: &mut Vec<u32>, m: u32, add: u32) {
    let mut carry = u64::from(add);
    for w in a.iter_mut() {
        let v = u64::from(*w) * u64::from(m) + carry;
        *w = v as u32;
        carry = v >> 32;
    }
    if carry != 0 {
        a.push(carry as u32);
    }
}

fn bit_len(a: &[u32]) -> usize {
    a.iter()
        .rposition(|&w| w != 0)
        .map_or(0, |i| i * 32 + 32 - a[i].leading_zeros() as usize)
}

fn shl(a: &[u32], n: usize) -> Vec<u32> {
    let (words, bits) = (n / 32, n % 32);
    let mut out = vec![0u32; words];
    let mut carry = 0u32;
    for &w in a {
        out.push(if bits == 0 { w } else { (w << bits) | carry });
        carry = if bits == 0 { 0 } else { w >> (32 - bits) };
    }
    out.push(carry);
    out
}

fn cmp(a: &[u32], b: &[u32]) -> std::cmp::Ordering {
    let len = a.len().max(b.len());
    let at = |x: &[u32], i: usize| x.get(i).copied().unwrap_or(0);
    (0..len)
        .rev()
        .map(|i| at(a, i).cmp(&at(b, i)))
        .find(|o| o.is_ne())
        .unwrap_or(std::cmp::Ordering::Equal)
}

/// `num * 2^s` against `den * 2^t` (`t >= 0`).
fn cmp_shifted(num: &[u32], s: i64, den: &[u32], t: usize) -> std::cmp::Ordering {
    if s >= 0 {
        cmp(&shl(num, s as usize), &shl(den, t))
    } else {
        cmp(num, &shl(den, t + s.unsigned_abs() as usize))
    }
}

/// `a -= b` for `a >= b`.
fn sub(a: &mut [u32], b: &[u32]) {
    let mut borrow = 0i64;
    for (i, w) in a.iter_mut().enumerate() {
        let v = i64::from(*w) - i64::from(b.get(i).copied().unwrap_or(0)) - borrow;
        *w = v.rem_euclid(1 << 32) as u32;
        borrow = i64::from(v < 0);
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

/// `real'value`. An integer literal is accepted as a real, as sim1 and sim2 do.
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

/// A TIME value as typed on a command line or in a debugger, with or without a space
/// before the unit (`10ms`, `1.5 ns`), in femtoseconds.
pub fn time_literal(t: &str) -> R<i64> {
    let t = t.trim();
    let split = t
        .bytes()
        .position(|c| c.is_ascii_alphabetic())
        .unwrap_or(t.len());
    let text = format!("{} {}", t[..split].trim(), &t[split..]);
    physical_value(text.as_bytes(), &crate::types::time_type(), Bounds::I64)
}

/// `boolean'value`.
pub fn boolean_value(s: &[u8]) -> R<bool> {
    enum_value(s, &crate::types::boolean_type()).map(|p| p == 1)
}

/// `character'value`; returns the character.
pub fn character_value(s: &[u8]) -> R<u8> {
    enum_value(s, &crate::types::character_type()).map(|p| p as u8)
}

// ---------------------------------------------------------------------------
// Values of any type as typed in a debugger (deposit, force, value breakpoints).

/// The type of a value [`literal`] reads and [`literal_image`] writes.
#[derive(Debug, Clone, PartialEq)]
pub enum LitType {
    Enum(EnumType),
    Int(Bounds),
    Real(RealBounds),
    Phys(PhysicalType, Bounds),
    /// One dimension: the left index (a position for enumeration indices), its direction and
    /// the length; more dimensions nest.
    Array {
        left: i64,
        ascending: bool,
        len: u64,
        elem: Box<LitType>,
    },
    Record(Vec<(String, LitType)>),
}

/// A value of a [`LitType`]: enumeration positions, physical values in primary units.
#[derive(Debug, Clone, PartialEq)]
pub enum Lit {
    Enum(usize),
    Int(i64),
    Real(f64),
    Phys(i64),
    Array(Vec<Lit>),
    Record(Vec<Lit>),
}

impl LitType {
    /// The index of element `k` of an array, as written in an aggregate choice.
    pub fn index(&self, k: u64) -> Option<i64> {
        match self {
            LitType::Array {
                left, ascending, ..
            } => {
                let k = i64::try_from(k).ok()?;
                if *ascending {
                    left.checked_add(k)
                } else {
                    left.checked_sub(k)
                }
            }
            _ => None,
        }
    }

    fn what(&self) -> What {
        match self {
            LitType::Enum(_) => What::Enumeration,
            LitType::Int(_) => What::Integer,
            LitType::Real(_) => What::Real,
            LitType::Phys(..) => What::Physical,
            LitType::Array { .. } | LitType::Record(_) => What::Aggregate,
        }
    }
}

/// A value of type `ty` as a VHDL literal: a scalar as `T'VALUE` reads it (any unit of a
/// physical type), a string or bit string literal (`"01ZX"`, `x"F0"`, `8ux"F"`) for an array
/// of characters, or an aggregate with positional and named elements, index ranges and
/// `others` (`(a => 1, others => '0')`, `(7 downto 4 => '1', others => '0')`).
pub fn literal(s: &[u8], ty: &LitType) -> R<Lit> {
    let b = start(s, ty.what())?;
    let (v, j) = lit_at(s, b, ty)?;
    finish(s, j, ty.what())?;
    Ok(v)
}

/// `v` of type `ty` as text: scalars as [`literal`] reads them (physical values in the
/// primary unit), arrays of characters as strings, other composites as aggregates; arrays
/// longer than 256 elements end in `...`.
pub fn literal_image(v: &Lit, ty: &LitType) -> String {
    const MAX: usize = 256;
    match (v, ty) {
        // Identifiers as the type spells them.
        (Lit::Enum(p), LitType::Enum(t)) => match t.lits.get(*p) {
            Some(EnumLit::Ident(n)) => latin1(n),
            Some(EnumLit::Char(c)) => format!("'{}'", *c as char),
            None => p.to_string(),
        },
        (Lit::Int(x), _) => x.to_string(),
        (Lit::Real(x), _) => x.to_string(),
        (Lit::Phys(x), LitType::Phys(t, _)) => {
            format!("{x} {}", latin1(&t.primary().name))
        }
        (Lit::Array(xs), LitType::Array { elem, .. }) => {
            if let LitType::Enum(t) = &**elem {
                let chars: Option<String> = xs
                    .iter()
                    .map(|x| match x {
                        Lit::Enum(p) => match t.lits.get(*p) {
                            Some(EnumLit::Char(c)) => Some(*c as char),
                            _ => None,
                        },
                        _ => None,
                    })
                    .collect();
                let has_chars = t.lits.iter().any(|l| matches!(l, EnumLit::Char(_)));
                if let Some(s) = chars.filter(|_| has_chars && xs.len() <= MAX) {
                    return format!("\"{}\"", s.replace('"', "\"\""));
                }
            }
            let items: Vec<String> = xs
                .iter()
                .take(MAX)
                .map(|x| literal_image(x, elem))
                .collect();
            let more = if xs.len() > MAX { ", ..." } else { "" };
            format!("({}{more})", items.join(", "))
        }
        (Lit::Record(xs), LitType::Record(fs)) => {
            let items: Vec<String> = xs
                .iter()
                .zip(fs)
                .map(|(x, (n, t))| format!("{n} => {}", literal_image(x, t)))
                .collect();
            format!("({})", items.join(", "))
        }
        _ => "?".to_string(),
    }
}

fn agg_err(s: &[u8], at: std::ops::Range<usize>, expected: &'static str) -> ConvError {
    ConvError::new(What::Aggregate, ErrorKind::Syntax { expected }, s, at)
}

/// The end of the element starting at `i`: the `,` or `)` closing it at depth 0, and the
/// `=>` in it, if any.
fn element_end(s: &[u8], mut i: usize) -> (usize, Option<usize>) {
    let mut depth = 0usize;
    let mut arrow = None;
    while let Some(&c) = s.get(i) {
        match c {
            b'\'' if s.get(i + 2) == Some(&b'\'') => i += 2,
            b'"' => {
                i += 1;
                while i < s.len() && !(s[i] == b'"' && s.get(i + 1) != Some(&b'"')) {
                    i += if s[i] == b'"' { 2 } else { 1 };
                }
            }
            b'(' => depth += 1,
            b')' if depth == 0 => break,
            b')' => depth -= 1,
            b',' if depth == 0 => break,
            b'=' if depth == 0 && arrow.is_none() && s.get(i + 1) == Some(&b'>') => arrow = Some(i),
            _ => {}
        }
        i += 1;
    }
    (i, arrow)
}

/// The value of type `ty` at `i` (not blank) and where it ends.
fn lit_at(s: &[u8], i: usize, ty: &LitType) -> R<(Lit, usize)> {
    match ty {
        LitType::Array { len, elem, .. } => match s.get(i) {
            Some(b'(') => aggregate(s, i, ty),
            Some(_) => {
                let (chars, j) = string_at(s, i)?;
                if chars.len() as u64 != *len {
                    let what = What::String;
                    let found = chars.len() as u64;
                    return Err(ConvError::new(
                        what,
                        ErrorKind::Length {
                            expected: *len,
                            found,
                        },
                        s,
                        i..j,
                    ));
                }
                let LitType::Enum(t) = &**elem else {
                    return Err(agg_err(s, i..j, "an aggregate"));
                };
                let xs = chars
                    .iter()
                    .map(|&c| {
                        t.lits
                            .iter()
                            .position(|l| *l == EnumLit::Char(c))
                            .map(Lit::Enum)
                            .ok_or_else(|| {
                                ConvError::new(
                                    What::String,
                                    ErrorKind::UnknownLiteral,
                                    &[b'\'', c, b'\''],
                                    0..3,
                                )
                            })
                    })
                    .collect::<R<Vec<_>>>()?;
                Ok((Lit::Array(xs), j))
            }
            None => Err(agg_err(s, i..i, "an aggregate or a string")),
        },
        LitType::Record(_) if s.get(i) == Some(&b'(') => aggregate(s, i, ty),
        LitType::Record(_) => Err(agg_err(s, i..i + 1, "a record aggregate such as (a => 1)")),
        _ => {
            let (end, _) = element_end(s, i);
            let end = end - s[i..end].iter().rev().take_while(|&&c| is_ws(c)).count();
            let t = &s[i..end];
            let v = match ty {
                LitType::Enum(e) => Lit::Enum(enum_value(t, e)?),
                LitType::Int(b) => Lit::Int(integer_value(t, *b)?),
                LitType::Real(b) => Lit::Real(real_value(t, *b)?),
                LitType::Phys(p, b) => Lit::Phys(physical_value(t, p, *b)?),
                LitType::Array { .. } | LitType::Record(_) => unreachable!("matched above"),
            };
            Ok((v, end))
        }
    }
}

/// The number of elements in the string or bit string literal at `i`, without
/// converting its characters to an enumeration type.
pub fn string_len(s: &[u8], i: usize) -> R<u64> {
    string_at(s, i).map(|(v, _)| v.len() as u64)
}

/// A string literal or a bit string literal (LRM 15.7, 15.8) at `i`: its characters and its
/// end.
fn string_at(s: &[u8], i: usize) -> R<(Vec<u8>, usize)> {
    let what = What::BitVector;
    let quoted = |at: usize| -> R<(Vec<u8>, usize)> {
        let mut out = Vec::new();
        let mut j = at + 1;
        loop {
            match s.get(j) {
                Some(b'"') if s.get(j + 1) == Some(&b'"') => {
                    out.push(b'"');
                    j += 2;
                }
                Some(b'"') => return Ok((out, j + 1)),
                Some(&c) => {
                    out.push(c);
                    j += 1;
                }
                None => return Err(syntax(What::String, s, j, "'\"' closing the string")),
            }
        }
    };
    if s.get(i) == Some(&b'"') {
        return quoted(i);
    }
    // [length] [U|S] B|O|X|D "..."
    let mut j = i;
    while s.get(j).is_some_and(u8::is_ascii_digit) {
        j += 1;
    }
    let width = if j > i {
        Some(integer_value(&s[i..j], Bounds::I64)? as usize)
    } else {
        None
    };
    let signed = match s.get(j).map(|c| c.to_ascii_lowercase()) {
        Some(b'u') if s.get(j + 1).is_some_and(|c| b"bBoOxX".contains(c)) => {
            j += 1;
            Some(false)
        }
        Some(b's') if s.get(j + 1).is_some_and(|c| b"bBoOxX".contains(c)) => {
            j += 1;
            Some(true)
        }
        _ => None,
    };
    let base = match s.get(j).map(|c| c.to_ascii_lowercase()) {
        Some(b'b') => 1,
        Some(b'o') => 3,
        Some(b'x') => 4,
        Some(b'd') if signed.is_none() => 0,
        _ => {
            return Err(syntax(
                what,
                s,
                i,
                "a string, a bit string such as x\"F0\" or an aggregate",
            ));
        }
    };
    if s.get(j + 1) != Some(&b'"') {
        return Err(syntax(what, s, j + 1, "'\"' after the base specifier"));
    }
    let (text, end) = quoted(j + 1)?;
    let digits: Vec<u8> = text.into_iter().filter(|&c| c != b'_').collect();
    let mut bits = Vec::new();
    if base == 0 {
        let mut v: u128 = 0;
        for &c in &digits {
            let d = (c as char)
                .to_digit(10)
                .ok_or_else(|| syntax(what, s, j + 2, "decimal digits"))?;
            v = v
                .checked_mul(10)
                .and_then(|v| v.checked_add(u128::from(d)))
                .ok_or_else(|| ConvError::new(what, ErrorKind::Overflow, s, i..end))?;
        }
        let n = (128 - v.leading_zeros()).max(1);
        bits.extend(
            (0..n)
                .rev()
                .map(|b| if v >> b & 1 == 1 { b'1' } else { b'0' }),
        );
    } else {
        for &c in &digits {
            match (c as char).to_digit(16).filter(|&d| d < 1 << base) {
                Some(d) => bits.extend(
                    (0..base)
                        .rev()
                        .map(|b| if d >> b & 1 == 1 { b'1' } else { b'0' }),
                ),
                None if c.is_ascii_hexdigit() => {
                    return Err(syntax(what, s, j + 2, "digits of the base"));
                }
                // Other characters stand for themselves, once per bit (LRM 15.8).
                None => bits.extend(std::iter::repeat_n(c, base as usize)),
            }
        }
    }
    if let Some(w) = width {
        let fill = if signed == Some(true) {
            bits.first().copied().unwrap_or(b'0')
        } else {
            b'0'
        };
        if bits.len() < w {
            let mut out = vec![fill; w - bits.len()];
            out.extend(bits);
            bits = out;
        } else {
            let cut = bits.len() - w;
            if bits[..cut].iter().any(|&c| c != fill) || (signed == Some(true) && w == 0) {
                return Err(ConvError::new(what, ErrorKind::DoesNotFit, s, i..end));
            }
            bits.drain(..cut);
        }
    }
    Ok((bits, end))
}

/// An aggregate of array or record type `ty` at `i` (`(`).
fn aggregate(s: &[u8], i: usize, ty: &LitType) -> R<(Lit, usize)> {
    let (n, names): (u64, Vec<&str>) = match ty {
        LitType::Array { len, .. } => (*len, Vec::new()),
        LitType::Record(fs) => (
            fs.len() as u64,
            fs.iter().map(|(n, _)| n.as_str()).collect(),
        ),
        _ => unreachable!("aggregate of a scalar"),
    };
    let elem_ty = |k: usize| match ty {
        LitType::Array { elem, .. } => &**elem,
        LitType::Record(fs) => &fs[k].1,
        _ => unreachable!("aggregate of a scalar"),
    };
    let name_of = |k: usize| match ty {
        LitType::Record(fs) => format!("field {}", fs[k].0),
        _ => format!("index {}", ty.index(k as u64).unwrap_or(0)),
    };
    let mut slots: Vec<Option<Lit>> = vec![None; n as usize];
    let mut pos = 0usize;
    let mut named = false;
    let mut others = None;
    let mut j = skip_ws(s, i + 1);
    loop {
        let (end, arrow) = element_end(s, j);
        if others.is_some() {
            return Err(agg_err(s, j..end, "`others` as the last element"));
        }
        let set = |k: usize, at: usize, slots: &mut Vec<Option<Lit>>| -> R<usize> {
            let (v, e) = lit_at(s, skip_ws(s, at), elem_ty(k))?;
            if slots[k].replace(v).is_some() {
                return Err(ConvError::new(
                    What::Aggregate,
                    ErrorKind::Element {
                        what: format!("two values for {}", name_of(k)),
                    },
                    s,
                    j..end,
                ));
            }
            Ok(e)
        };
        let e = match arrow {
            None if named => {
                return Err(agg_err(s, j..end, "named elements after positional ones"));
            }
            None => {
                if pos as u64 >= n {
                    let found = 1 + s[j..].iter().filter(|&&c| c == b',').count() as u64;
                    return Err(ConvError::new(
                        What::Aggregate,
                        ErrorKind::Length {
                            expected: n,
                            found: pos as u64 + found,
                        },
                        s,
                        i..s.len(),
                    ));
                }
                pos += 1;
                set(pos - 1, j, &mut slots)?
            }
            Some(a) => {
                named = true;
                let mut e = a + 2;
                for c in s[j..a].split(|&c| c == b'|') {
                    let c = latin1(c);
                    let c = c.trim();
                    if c.eq_ignore_ascii_case("others") {
                        others = Some(a + 2);
                        e = element_end(s, a + 2).0;
                        continue;
                    }
                    for k in choice(s, j..a, c, ty, &names)? {
                        e = set(k, a + 2, &mut slots)?;
                    }
                }
                e
            }
        };
        let e = skip_ws(s, e);
        match s.get(e) {
            Some(b',') => j = skip_ws(s, e + 1),
            Some(b')') => {
                for (k, slot) in slots.iter_mut().enumerate() {
                    if slot.is_some() {
                        continue;
                    }
                    match others {
                        Some(at) => {
                            *slot = Some(lit_at(s, skip_ws(s, at), elem_ty(k))?.0);
                        }
                        None if !named => {
                            return Err(ConvError::new(
                                What::Aggregate,
                                ErrorKind::Length {
                                    expected: n,
                                    found: pos as u64,
                                },
                                s,
                                i..e + 1,
                            ));
                        }
                        None => {
                            return Err(ConvError::new(
                                What::Aggregate,
                                ErrorKind::Element {
                                    what: format!("no value for {}", name_of(k)),
                                },
                                s,
                                i..e + 1,
                            ));
                        }
                    }
                }
                let xs = slots.into_iter().flatten().collect();
                let v = match ty {
                    LitType::Record(_) => Lit::Record(xs),
                    _ => Lit::Array(xs),
                };
                return Ok((v, e + 1));
            }
            _ => return Err(agg_err(s, e..e + 1, "',' or ')'")),
        }
    }
}

/// The element positions a choice `c` (in `s[at]`) names: a field, an index or an index
/// range (`3 downto 0`).
fn choice(
    s: &[u8],
    at: std::ops::Range<usize>,
    c: &str,
    ty: &LitType,
    names: &[&str],
) -> R<Vec<usize>> {
    if let LitType::Record(_) = ty {
        return names
            .iter()
            .position(|n| n.eq_ignore_ascii_case(c))
            .map(|k| vec![k])
            .ok_or_else(|| agg_err(s, at, "a field name or others"));
    }
    let LitType::Array { len, .. } = ty else {
        unreachable!("aggregate of a scalar")
    };
    let int = |t: &str| integer_value(t.as_bytes(), Bounds::I64);
    let lower = c.to_ascii_lowercase();
    let (a, b) = if let Some((a, b)) = lower.split_once(" downto ") {
        (int(a)?, int(b)?)
    } else if let Some((a, b)) = lower.split_once(" to ") {
        (int(a)?, int(b)?)
    } else {
        let x = int(c)?;
        (x, x)
    };
    let pos = |x: i64| {
        (0..*len)
            .find(|&k| ty.index(k) == Some(x))
            .map(|k| k as usize)
            .ok_or_else(|| agg_err(s, at.clone(), "an index of the array"))
    };
    let (p, q) = (pos(a)?, pos(b)?);
    Ok((p.min(q)..=p.max(q)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::time_type;

    /// Based reals are correctly rounded (#105): against exact f64 division and products,
    /// ties to even, subnormals, overflow and the f64 limits.
    #[test]
    fn based_reals_round_correctly() {
        let v = |s: &str| real_value(s.as_bytes(), RealBounds::REAL).unwrap();
        assert_eq!(v("3#0.1#"), 1.0 / 3.0);
        assert_eq!(v("7#0.1#E-3"), 1.0 / 2401.0);
        assert_eq!(v("13#0.C#"), 12.0 / 13.0);
        assert_eq!(v("5#0.0001#"), 1.0 / 625.0);
        // 2^53 + 1 is a tie: to even (down); 2^53 + 3 rounds up.
        assert_eq!(v("16#20000000000001.0#"), 9007199254740992.0);
        assert_eq!(v("16#20000000000003.0#"), 9007199254740996.0);
        // Sticky bits past the guard bit decide a near-tie.
        assert_eq!(v("16#20000000000001.00000000001#"), 9007199254740994.0);
        assert_eq!(v("2#1.0#E-1074"), f64::from_bits(1));
        assert_eq!(v("2#1.0#E-1075"), 0.0);
        assert_eq!(v("2#1.1#E-1075"), f64::from_bits(1));
        assert_eq!(v("2#1.1#E-1074"), f64::from_bits(2));
        let ones = "1".repeat(52);
        assert_eq!(v(&format!("2#1.{ones}#E1023")), f64::MAX);
        // MAX plus just under half an ulp still rounds to MAX; half an ulp is a tie to infinity.
        assert_eq!(v(&format!("2#1.{ones}0111#E1023")), f64::MAX);
        assert!(real_value(format!("2#1.{ones}1#E1023").as_bytes(), RealBounds::REAL).is_err());
        assert_eq!(v("3#1.0#E-700"), 3f64.powi(-700));
        assert_eq!(v("16#0.000#"), 0.0);
        assert!(real_value(b"16#1.0#E300", RealBounds::REAL).is_err());
        assert_eq!(v("16#1.0#E-300"), 0.0);
        // Exact quotients and products of f64-exact integers in every base.
        let mut seed = 0x9e3779b97f4a7c15u64;
        for _ in 0..2000 {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            let base = 2 + (seed % 15) as u32;
            let m = (seed >> 20) % (1 << 40);
            let n = (seed >> 8) % 8;
            let p = u64::from(base).pow(n as u32);
            let text = format!("{base}#{}.0#", to_base(m, base));
            let lit = lex_abstract(text.as_bytes(), 0, What::Real).unwrap().0;
            let mut neg = lit.clone();
            neg.exp = -(n as i64);
            assert_eq!(neg.to_f64(), m as f64 / p as f64, "{text}E-{n}");
            if m.checked_mul(p).is_some_and(|x| x < (1 << 53)) {
                let mut pos = lit;
                pos.exp = n as i64;
                assert_eq!(pos.to_f64(), (m * p) as f64, "{text}E{n}");
            }
        }
    }

    fn to_base(mut m: u64, base: u32) -> String {
        let mut s = Vec::new();
        loop {
            s.push(char::from_digit((m % u64::from(base)) as u32, base).unwrap());
            m /= u64::from(base);
            if m == 0 {
                break;
            }
        }
        s.iter().rev().collect()
    }

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

    /// Debugger values (#323): scalars, string and bit string literals, aggregates, and
    /// their images.
    #[test]
    fn literals() {
        let logic = LitType::Enum(EnumType {
            lits: b"UX01ZWLH-".iter().map(|&c| EnumLit::Char(c)).collect(),
        });
        let slv = |len, left| LitType::Array {
            left,
            ascending: false,
            len,
            elem: Box::new(logic.clone()),
        };
        let v8 = slv(8, 7);
        let p = |s: &str, t: &LitType| literal(s.as_bytes(), t);
        let img = |s: &str, t: &LitType| literal_image(&p(s, t).unwrap(), t);
        assert_eq!(img("x\"A5\"", &v8), "\"10100101\"");
        assert_eq!(img("8ux\"F\"", &v8), "\"00001111\"");
        assert_eq!(img("8sx\"F\"", &v8), "\"11111111\"");
        assert_eq!(img("8d\"200\"", &v8), "\"11001000\"");
        assert_eq!(img("8x\"0F\"", &v8), "\"00001111\"");
        assert_eq!(img("X\"Z_1\"", &v8), "\"ZZZZ0001\"");
        assert_eq!(img(" \"0101UXZ-\" ", &v8), "\"0101UXZ-\"");
        assert_eq!(img("(others => '1')", &v8), "\"11111111\"");
        assert_eq!(
            img("(7 downto 4 => '1', others => '0')", &v8),
            "\"11110000\""
        );
        assert_eq!(img("(0 | 2 => '1', others => 'Z')", &v8), "\"ZZZZZ1Z1\"");
        assert_eq!(img("('1', '0', others => 'U')", &v8), "\"10UUUUUU\"");
        let err = |s: &str, t: &LitType| p(s, t).unwrap_err().to_string();
        assert_eq!(
            err("x\"A\"", &v8),
            "string value x\"A\" has 4 elements, expected 8"
        );
        assert!(p("8x\"1FF\"", &v8).is_err());
        assert!(p("\"0101010Q\"", &v8).is_err());
        assert!(p("(others => '1', 0 => '0')", &v8).is_err());
        assert_eq!(
            err("(7 => '1')", &v8),
            "aggregate \"(7 => '1')\": no value for index 6"
        );
        assert!(p("(8 => '1', others => '0')", &v8).is_err());
        assert!(p("('1', '0')", &v8).is_err());

        let time = LitType::Phys(time_type(), Bounds::I64);
        assert_eq!(p("1.5 ns", &time), Ok(Lit::Phys(1_500_000)));
        assert_eq!(img("2 us", &time), "2000000000 fs");
        let rec = LitType::Record(vec![
            ("a".into(), LitType::Int(Bounds::I32)),
            ("b".into(), slv(2, 1)),
            ("t".into(), time),
        ]);
        assert_eq!(
            img("(t => 5 ns, A => -3, b => \"1Z\")", &rec),
            "(a => -3, b => \"1Z\", t => 5000000 fs)"
        );
        assert_eq!(
            img("(1, ('0', '1'), others => 1 ps)", &rec),
            "(a => 1, b => \"01\", t => 1000 fs)"
        );
        assert!(p("(a => 1, b => \"11\")", &rec).is_err());
        assert!(p("(a => 1, a => 2, others => 0 fs)", &rec).is_err());
        assert!(p("(a => 4294967296, b => \"11\", t => 0 fs)", &rec).is_err());

        let chars = LitType::Array {
            left: 1,
            ascending: true,
            len: 3,
            elem: Box::new(LitType::Enum(crate::types::character_type())),
        };
        assert_eq!(img("\"a\"\"b\"", &chars), "\"a\"\"b\"");
        assert_eq!(img("(1 => 'x', 2 to 3 => ',')", &chars), "\"x,,\"");
        let grid = LitType::Array {
            left: 0,
            ascending: true,
            len: 2,
            elem: Box::new(LitType::Array {
                left: 0,
                ascending: true,
                len: 2,
                elem: Box::new(LitType::Real(RealBounds::REAL)),
            }),
        };
        assert_eq!(
            img("((1.5, 2.0), (others => 0.0))", &grid),
            "((1.5, 2), (0, 0))"
        );
        let b = LitType::Enum(crate::types::boolean_type());
        assert_eq!(img(" true ", &b), "TRUE");
        assert!(p("maybe", &b).is_err());
    }
}
