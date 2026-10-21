//! `'IMAGE` and `TO_STRING` for scalar and bit-string values.
//!
//! Formats follow sim1 byte for byte where the LRM leaves the format to the
//! implementation (reals, physical values with a unit), so that report text
//! matches the differential reference; they follow the LRM where sim1
//! deviates from it (extended identifiers keep their spelling). See
//! docs/decisions.md.

use crate::error::{ConvError, ErrorKind, What};
use crate::printf;
use crate::types::{EnumLit, PhysicalType, character_name, lower};

/// `integer'image`, `to_string(integer)`.
pub fn integer_image(v: i64) -> Vec<u8> {
    integer_digits(v, &mut [0; 20]).to_vec()
}

/// The decimal text of `v` in `buf` (20 bytes hold any `i64`), for callers that copy it on.
pub fn integer_digits(v: i64, buf: &mut [u8; 20]) -> &[u8] {
    let mut at = buf.len();
    let mut n = v.unsigned_abs();
    loop {
        at -= 1;
        buf[at] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            break;
        }
    }
    if v < 0 {
        at -= 1;
        buf[at] = b'-';
    }
    &buf[at..]
}

/// `real'image` and `to_string(real)`: C `%.15g` (implementation-defined
/// by the LRM; this is sim1's form).
pub fn real_image(x: f64) -> Vec<u8> {
    printf::g(x, 15)
}

/// `to_string(real, digits)`: `digits = 0` gives `%.17g` (round-trips),
/// otherwise fixed point with that many digits after the point.
pub fn real_to_string_digits(x: f64, digits: usize) -> Result<Vec<u8>, ConvError> {
    if digits > printf::MAX_WIDTH {
        return Err(ConvError::new(
            What::Real,
            ErrorKind::Overflow,
            digits.to_string().as_bytes(),
            0..usize::MAX,
        ));
    }
    Ok(if digits == 0 {
        printf::g(x, 17)
    } else {
        printf::f(x, digits)
    })
}

/// `to_string(real, format)` with a C format such as `"%8.3f"`.
pub fn real_to_string_format(x: f64, format: &[u8]) -> Result<Vec<u8>, ConvError> {
    printf::format_real(format, x)
}

/// Image of an identifier: basic identifiers in lower case, extended
/// identifiers exactly as declared (LRM 16.2.2).
pub fn identifier_image(name: &[u8]) -> Vec<u8> {
    if name.first() == Some(&b'\\') {
        name.to_vec()
    } else {
        name.iter().map(|&c| lower(c)).collect()
    }
}

/// `T'image` of an enumeration literal: `foo`, `\Bar\`, `'x'`.
pub fn enum_image(lit: &EnumLit) -> Vec<u8> {
    match lit {
        EnumLit::Ident(n) => identifier_image(n),
        EnumLit::Char(c) => vec![b'\'', *c, b'\''],
    }
}

/// `to_string` of an enumeration value: like the image, but a character
/// literal is the bare character (LRM 5.7).
pub fn enum_to_string(lit: &EnumLit) -> Vec<u8> {
    match lit {
        EnumLit::Char(c) => vec![*c],
        _ => enum_image(lit),
    }
}

/// `character'image`.
pub fn character_image(c: u8) -> Vec<u8> {
    match character_name(c) {
        Some(n) => n.to_ascii_lowercase().into_bytes(),
        None => vec![b'\'', c, b'\''],
    }
}

/// `to_string(character)`.
pub fn character_to_string(c: u8) -> Vec<u8> {
    match character_name(c) {
        Some(n) => n.to_ascii_lowercase().into_bytes(),
        None => vec![c],
    }
}

pub fn boolean_image(b: bool) -> Vec<u8> {
    if b {
        b"true".to_vec()
    } else {
        b"false".to_vec()
    }
}

/// `T'image` of a physical value: in the primary unit, `5000000 fs`.
pub fn physical_image(v: i64, ty: &PhysicalType) -> Vec<u8> {
    let mut s = integer_image(v);
    s.push(b' ');
    s.extend(identifier_image(&ty.primary().name));
    s
}

fn unit_of(ty: &PhysicalType, unit: i64) -> Result<&crate::types::Unit, ConvError> {
    ty.unit_with_value(unit).ok_or_else(|| {
        ConvError::new(
            What::Unit,
            ErrorKind::UnknownUnit,
            &physical_image(unit, ty),
            0..usize::MAX,
        )
    })
}

/// `to_string(value, unit)` of a physical value (2008: TIME). The unit must
/// be the value of one of the type's units. An exact multiple prints as an
/// integer, anything else as `%.15g` of the quotient (sim1's form).
pub fn physical_to_string_unit(v: i64, ty: &PhysicalType, unit: i64) -> Result<Vec<u8>, ConvError> {
    let u = unit_of(ty, unit)?;
    let mut s = if v % u.value == 0 {
        integer_image(v / u.value)
    } else {
        printf::g(v as f64 / u.value as f64, 15)
    };
    s.push(b' ');
    s.extend(identifier_image(&u.name));
    Ok(s)
}

/// The textio `write(line, time, ..., unit)` form: an exact decimal
/// multiple of `unit`, with at most `min(floor(log10(unit)), 17)` digits
/// after the point, truncated; an inexact fraction keeps all its digits.
pub fn physical_write_unit(v: i64, ty: &PhysicalType, unit: i64) -> Result<Vec<u8>, ConvError> {
    let u = unit_of(ty, unit)?;
    let d = u.value.unsigned_abs() as u128;
    let mag = v.unsigned_abs() as u128;
    let mut s = String::new();
    if v < 0 {
        s.push('-');
    }
    s.push_str(&(mag / d).to_string());
    let mut rem = mag % d;
    if rem != 0 {
        let limit = (u.value.unsigned_abs().ilog10() as usize).min(17);
        s.push('.');
        for _ in 0..limit {
            if rem == 0 {
                break;
            }
            rem *= 10;
            s.push(char::from(b'0' + (rem / d) as u8));
            rem %= d;
        }
    }
    let mut s = s.into_bytes();
    s.push(b' ');
    s.extend(identifier_image(&u.name));
    Ok(s)
}

/// `to_string(bit_vector)` and `to_bstring`. Bits are given left to right.
pub fn bits_to_string(bits: &[bool]) -> Vec<u8> {
    bits.iter().map(|&b| if b { b'1' } else { b'0' }).collect()
}

fn bits_to_radix(bits: &[bool], width: usize) -> Vec<u8> {
    const DIGITS: &[u8; 16] = b"0123456789ABCDEF";
    // The first digit is zero-extended on the left by `pad` bits.
    let pad = (width - bits.len() % width) % width;
    let mut out = Vec::with_capacity((bits.len() + pad) / width);
    let mut acc = 0usize;
    for (i, b) in std::iter::repeat_n(false, pad)
        .chain(bits.iter().copied())
        .enumerate()
    {
        acc = acc * 2 + usize::from(b);
        if (i + 1) % width == 0 {
            out.push(DIGITS[acc]);
            acc = 0;
        }
    }
    out
}

/// The same for a vector of `n` (1 to 64) elements held as the bits of a word, element 0 in
/// bit 0: `to_hstring` for `width` 4, `to_ostring` for 3.
pub fn word_to_radix(bits: u64, n: usize, width: usize) -> Vec<u8> {
    const DIGITS: &[u8; 16] = b"0123456789ABCDEF";
    (0..n.div_ceil(width))
        .rev()
        .map(|d| DIGITS[(bits >> (width * d) & ((1 << width) - 1)) as usize])
        .collect()
}

/// `to_hstring(bit_vector)`: zero-extended on the left to whole digits.
pub fn bits_to_hstring(bits: &[bool]) -> Vec<u8> {
    bits_to_radix(bits, 4)
}

/// `to_ostring(bit_vector)`.
pub fn bits_to_ostring(bits: &[bool]) -> Vec<u8> {
    bits_to_radix(bits, 3)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::time_type;

    #[test]
    fn integer_digits_match_to_string() {
        for v in [
            0,
            1,
            -1,
            9,
            10,
            -10,
            12345,
            i64::MAX,
            i64::MIN,
            i64::MIN + 1,
        ] {
            assert_eq!(integer_image(v), v.to_string().into_bytes(), "{v}");
        }
    }

    #[test]
    fn word_radix_matches_bits() {
        for n in 1..=64usize {
            let v = 0x9E37_79B9_7F4A_7C15u64 >> (64 - n);
            let bits: Vec<bool> = (0..n).rev().map(|i| v >> i & 1 == 1).collect();
            assert_eq!(word_to_radix(v, n, 4), bits_to_hstring(&bits), "{n}");
            assert_eq!(word_to_radix(v, n, 3), bits_to_ostring(&bits), "{n}");
        }
    }

    #[test]
    fn write_unit() {
        let t = time_type();
        let w = |v, u| String::from_utf8(physical_write_unit(v, &t, u).unwrap()).unwrap();
        assert_eq!(w(1_500_000, 1_000_000), "1.5 ns");
        assert_eq!(w(1, 3_600_000_000_000_000_000), "0.00000000000000000 hr");
        assert_eq!(w(i64::MIN, 1_000_000), "-9223372036854.775808 ns");
        assert!(physical_write_unit(1, &t, 2_000_000).is_err());
    }
}
