//! C `printf` formatting of one real value: the engine behind
//! `to_string(real, format)`, `write(line, real, format)` and the `%g`, `%e`
//! and `%f` forms the other real conversions use.
//!
//! Supported: literal text, `%%`, and exactly one conversion
//! `%[flags][width][.precision]conv` with flags `- + space # 0` and
//! conversions `f F e E g G a A`, with C99 semantics. Digits are exact
//! (correctly rounded, ties to even on the exact binary value), as glibc.

use crate::error::{ConvError, ErrorKind, What};

/// Largest width or precision accepted; larger ones are an error rather
/// than a huge allocation.
pub const MAX_WIDTH: usize = 65_535;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Spec {
    pub left: bool,
    pub plus: bool,
    pub space: bool,
    pub alt: bool,
    pub zero: bool,
    pub width: usize,
    pub precision: Option<usize>,
    /// One of `f F e E g G a A`.
    pub conv: u8,
}

impl Spec {
    pub fn new(conv: u8, precision: Option<usize>) -> Spec {
        Spec {
            conv,
            precision,
            ..Spec::default()
        }
    }
}

/// Format `x` with a C format string such as `"%8.3f"` or `"x=%g s"`.
pub fn format_real(fmt: &[u8], x: f64) -> Result<Vec<u8>, ConvError> {
    let err = |kind, span| Err(ConvError::new(What::Format, kind, fmt, span));
    let mut out = Vec::new();
    let mut seen = false;
    let mut i = 0;
    while i < fmt.len() {
        let c = fmt[i];
        if c != b'%' {
            out.push(c);
            i += 1;
            continue;
        }
        let start = i;
        i += 1;
        if fmt.get(i) == Some(&b'%') {
            out.push(b'%');
            i += 1;
            continue;
        }
        let mut s = Spec::default();
        while let Some(&f) = fmt.get(i) {
            match f {
                b'-' => s.left = true,
                b'+' => s.plus = true,
                b' ' => s.space = true,
                b'#' => s.alt = true,
                b'0' => s.zero = true,
                _ => break,
            }
            i += 1;
        }
        let num = |i: &mut usize| -> Option<usize> {
            let b = *i;
            let mut v: usize = 0;
            while let Some(d) = fmt.get(*i).filter(|d| d.is_ascii_digit()) {
                v = v.saturating_mul(10).saturating_add((d - b'0') as usize);
                *i += 1;
            }
            (*i > b).then_some(v)
        };
        let wstart = i;
        s.width = num(&mut i).unwrap_or(0);
        if s.width > MAX_WIDTH {
            return err(ErrorKind::Overflow, wstart..i);
        }
        if fmt.get(i) == Some(&b'.') {
            i += 1;
            let pstart = i;
            let p = num(&mut i).unwrap_or(0);
            if p > MAX_WIDTH {
                return err(ErrorKind::Overflow, pstart..i);
            }
            s.precision = Some(p);
        }
        match fmt.get(i) {
            Some(&conv) if b"fFeEgGaA".contains(&conv) => {
                if seen {
                    return err(
                        ErrorKind::Syntax {
                            expected: "exactly one conversion for the one real value",
                        },
                        start..i + 1,
                    );
                }
                seen = true;
                s.conv = conv;
                i += 1;
                out.extend_from_slice(&format_spec(&s, x));
            }
            _ => {
                return err(
                    ErrorKind::Syntax {
                        expected: "a real conversion: one of f, F, e, E, g, G, a, A",
                    },
                    start..(i + 1).min(fmt.len()),
                );
            }
        }
    }
    if !seen {
        return err(
            ErrorKind::Syntax {
                expected: "a conversion such as %f, %e or %g",
            },
            0..fmt.len(),
        );
    }
    Ok(out)
}

/// Format `x` with one parsed conversion.
pub fn format_spec(s: &Spec, x: f64) -> Vec<u8> {
    let upper = s.conv.is_ascii_uppercase();
    let sign = if x.is_sign_negative() {
        "-"
    } else if s.plus {
        "+"
    } else if s.space {
        " "
    } else {
        ""
    };
    let a = x.abs();
    let (prefix, body) = if !x.is_finite() {
        let t = if x.is_nan() { "nan" } else { "inf" };
        (
            "",
            if upper {
                t.to_uppercase()
            } else {
                t.to_string()
            },
        )
    } else {
        match s.conv.to_ascii_lowercase() {
            b'f' => ("", fixed(a, s.precision.unwrap_or(6), s.alt)),
            b'e' => ("", expo(a, s.precision.unwrap_or(6), s.alt, upper)),
            b'g' => ("", general(a, s.precision.unwrap_or(6), s.alt, upper)),
            _ => (
                if upper { "0X" } else { "0x" },
                hexfloat(a, s.precision, s.alt, upper),
            ),
        }
    };
    let len = sign.len() + prefix.len() + body.len();
    let pad = s.width.saturating_sub(len);
    let mut out = String::with_capacity(len + pad);
    if s.left {
        out.push_str(sign);
        out.push_str(prefix);
        out.push_str(&body);
        out.extend(std::iter::repeat_n(' ', pad));
    } else if s.zero && x.is_finite() {
        out.push_str(sign);
        out.push_str(prefix);
        out.extend(std::iter::repeat_n('0', pad));
        out.push_str(&body);
    } else {
        out.extend(std::iter::repeat_n(' ', pad));
        out.push_str(sign);
        out.push_str(prefix);
        out.push_str(&body);
    }
    out.into_bytes()
}

/// `%.{p}f` of a non-negative finite value, without sign.
fn fixed(a: f64, p: usize, alt: bool) -> String {
    let mut s = format!("{a:.p$}");
    if alt && p == 0 {
        s.push('.');
    }
    s
}

/// Mantissa digits and decimal exponent of `a` rounded to `p` digits after
/// the point: `("1.500", 10)`.
fn expo_parts(a: f64, p: usize) -> (String, i32) {
    let s = format!("{a:.p$e}");
    let (m, e) = s.split_once('e').expect("Rust {:e} always has an exponent");
    (
        m.to_string(),
        e.parse().expect("Rust {:e} exponent is an integer"),
    )
}

fn exp_suffix(e: i32, upper: bool) -> String {
    format!(
        "{}{}{:02}",
        if upper { 'E' } else { 'e' },
        if e < 0 { '-' } else { '+' },
        e.unsigned_abs()
    )
}

fn expo(a: f64, p: usize, alt: bool, upper: bool) -> String {
    let (mut m, e) = expo_parts(a, p);
    if alt && p == 0 {
        m.push('.');
    }
    m + &exp_suffix(e, upper)
}

fn general(a: f64, p: usize, alt: bool, upper: bool) -> String {
    let p = p.max(1);
    let (_, x) = expo_parts(a, p - 1);
    let mut s = if (x as i64) < p as i64 && x >= -4 {
        fixed(a, (p as i64 - 1 - x as i64) as usize, alt)
    } else {
        expo(a, p - 1, alt, upper)
    };
    if !alt {
        let (num, suffix) = match s.find(['e', 'E']) {
            Some(i) => (s[..i].to_string(), s[i..].to_string()),
            None => (s.clone(), String::new()),
        };
        let num = if num.contains('.') {
            num.trim_end_matches('0').trim_end_matches('.').to_string()
        } else {
            num
        };
        s = num + &suffix;
    }
    s
}

/// `%a` body after the `0x` prefix: `1.8p+0`.
fn hexfloat(a: f64, precision: Option<usize>, alt: bool, upper: bool) -> String {
    let bits = a.to_bits();
    let biased = ((bits >> 52) & 0x7ff) as i32;
    let mut frac = bits & ((1u64 << 52) - 1);
    let (mut lead, exp) = if biased == 0 {
        (0u64, if frac == 0 { 0 } else { -1022 })
    } else {
        (1u64, biased - 1023)
    };
    let mut nibbles = 13usize;
    match precision {
        Some(p) if p < 13 => {
            let shift = (13 - p) * 4;
            let rest = frac & ((1u64 << shift) - 1);
            frac >>= shift;
            let half = 1u64 << (shift - 1);
            let odd = if p == 0 { lead & 1 == 1 } else { frac & 1 == 1 };
            if rest > half || (rest == half && odd) {
                frac += 1;
                if frac >> (p * 4) != 0 {
                    lead += 1;
                    frac &= (1u64 << (p * 4)) - 1;
                }
            }
            nibbles = p;
        }
        Some(_) => {}
        None => {
            while nibbles > 0 && frac & 0xf == 0 {
                frac >>= 4;
                nibbles -= 1;
            }
        }
    }
    let mut digits = if nibbles == 0 {
        String::new()
    } else {
        format!("{frac:0nibbles$x}")
    };
    if let Some(p) = precision {
        digits.extend(std::iter::repeat_n('0', p.saturating_sub(digits.len())));
    }
    let mut s = lead.to_string();
    if !digits.is_empty() || alt {
        s.push('.');
        s.push_str(&digits);
    }
    s.push_str(&format!(
        "p{}{}",
        if exp < 0 { '-' } else { '+' },
        exp.unsigned_abs()
    ));
    if upper { s.to_uppercase() } else { s }
}

/// `%.{p}g`, the form sim1 uses for `real'image` (p = 15) and
/// `to_string(real, 0)` (p = 17).
pub fn g(x: f64, p: usize) -> Vec<u8> {
    format_spec(&Spec::new(b'g', Some(p)), x)
}

/// `%.{p}e`.
pub fn e(x: f64, p: usize) -> Vec<u8> {
    format_spec(&Spec::new(b'e', Some(p)), x)
}

/// `%.{p}f`.
pub fn f(x: f64, p: usize) -> Vec<u8> {
    format_spec(&Spec::new(b'f', Some(p)), x)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fr(fmt: &str, x: f64) -> String {
        String::from_utf8(format_real(fmt.as_bytes(), x).unwrap()).unwrap()
    }

    #[test]
    fn c_semantics() {
        // Expected values are glibc printf output.
        assert_eq!(fr("%g", 100000.0), "100000");
        assert_eq!(fr("%g", 1000000.0), "1e+06");
        assert_eq!(fr("%g", 0.0001), "0.0001");
        assert_eq!(fr("%g", 0.00001), "1e-05");
        assert_eq!(fr("%#g", 1.0), "1.00000");
        assert_eq!(fr("%.0g", 0.0), "0");
        assert_eq!(fr("%#.0f", 1.0), "1.");
        assert_eq!(fr("%+f", 1.5), "+1.500000");
        assert_eq!(fr("% f", 1.5), " 1.500000");
        assert_eq!(fr("%-8.2f|", 1.5), "1.50    |");
        assert_eq!(fr("%08.2f", -1.5), "-0001.50");
        assert_eq!(fr("%e", 0.0), "0.000000e+00");
        assert_eq!(fr("%.2E", 12345.0), "1.23E+04");
        assert_eq!(fr("%e", 1e100), "1.000000e+100");
        assert_eq!(fr("%a", 1.5), "0x1.8p+0");
        assert_eq!(fr("%a", 1.0), "0x1p+0");
        assert_eq!(fr("%a", 0.0), "0x0p+0");
        assert_eq!(fr("%.0a", 1.5), "0x2p+0");
        assert_eq!(fr("%.0a", 2.5), "0x1p+1");
        assert_eq!(fr("%#.0g", 0.0), "0.");
        assert_eq!(fr("%.1a", 1.0), "0x1.0p+0");
        assert_eq!(fr("%A", -0.1), "-0X1.999999999999AP-4");
        assert_eq!(fr("%a", 5e-324), "0x0.0000000000001p-1022");
        assert_eq!(fr("%%%.1f", 2.25), "%2.2");
        assert_eq!(fr("%5.1g", 0.0), "    0");
        assert_eq!(fr("%f", f64::INFINITY), "inf");
        assert_eq!(fr("%05f", f64::NEG_INFINITY), " -inf");
        assert_eq!(fr("%.3g", 0.0001234), "0.000123");
        assert_eq!(fr("%.3g", 999.5), "1e+03");
    }

    #[test]
    fn format_errors() {
        for f in ["", "abc", "%d", "%s", "%f%f", "%5", "%.3", "%lf", "%"] {
            assert!(format_real(f.as_bytes(), 1.5).is_err(), "{f}");
        }
        assert!(format_real(b"%99999999f", 1.0).is_err());
    }
}
