//! Overflow-checked integer and physical arithmetic (plan rule 7).
//!
//! Integers and physical values (TIME) are `i64` in every tier. Each
//! operation returns an [`ArithError`] instead of wrapping, and a separate
//! [`check_range`] applies the bounds of the target subtype (for example the
//! 32-bit INTEGER of VHDL-2008).

use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Rem,
    Neg,
    Abs,
    Pow,
}

impl Op {
    fn symbol(self) -> &'static str {
        match self {
            Op::Add => "+",
            Op::Sub => "-",
            Op::Mul => "*",
            Op::Div => "/",
            Op::Mod => "mod",
            Op::Rem => "rem",
            Op::Neg => "-",
            Op::Abs => "abs",
            Op::Pow => "**",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ArithError {
    /// The exact result does not fit in 64 bits.
    Overflow {
        op: Op,
        lhs: i64,
        rhs: Option<i64>,
    },
    /// A real operand or result cannot be converted to a 64-bit value.
    RealOverflow {
        op: Op,
        value: f64,
    },
    DivisionByZero {
        op: Op,
        lhs: i64,
    },
    /// `**` with a negative exponent on an integer base.
    NegativeExponent {
        base: i64,
        exp: i64,
    },
    /// A value outside the bounds of its subtype.
    OutOfRange {
        value: i64,
        low: i64,
        high: i64,
    },
}

impl fmt::Display for ArithError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match *self {
            ArithError::Overflow {
                op,
                lhs,
                rhs: Some(r),
            } => write!(
                f,
                "result of {lhs} {} {r} cannot be represented in 64 bits",
                op.symbol()
            ),
            ArithError::Overflow { op, lhs, rhs: None } => write!(
                f,
                "result of {} {lhs} cannot be represented in 64 bits",
                op.symbol()
            ),
            ArithError::RealOverflow { op, value } => write!(
                f,
                "result of {} ({value}) cannot be represented in 64 bits",
                op.symbol()
            ),
            ArithError::DivisionByZero { op, lhs } => {
                write!(f, "division by zero in {lhs} {} 0", op.symbol())
            }
            ArithError::NegativeExponent { base, exp } => write!(
                f,
                "negative exponent {exp} in {base} ** {exp}: an integer base needs a natural exponent"
            ),
            ArithError::OutOfRange { value, low, high } => {
                write!(f, "value {value} is outside the range {low} to {high}")
            }
        }
    }
}

impl std::error::Error for ArithError {}

pub type ArithResult = Result<i64, ArithError>;

fn ovf(op: Op, lhs: i64, rhs: i64) -> ArithError {
    ArithError::Overflow {
        op,
        lhs,
        rhs: Some(rhs),
    }
}

pub fn add(a: i64, b: i64) -> ArithResult {
    a.checked_add(b).ok_or(ovf(Op::Add, a, b))
}

pub fn sub(a: i64, b: i64) -> ArithResult {
    a.checked_sub(b).ok_or(ovf(Op::Sub, a, b))
}

pub fn mul(a: i64, b: i64) -> ArithResult {
    a.checked_mul(b).ok_or(ovf(Op::Mul, a, b))
}

/// VHDL `/`: truncates toward zero.
pub fn div(a: i64, b: i64) -> ArithResult {
    if b == 0 {
        return Err(ArithError::DivisionByZero {
            op: Op::Div,
            lhs: a,
        });
    }
    a.checked_div(b).ok_or(ovf(Op::Div, a, b))
}

/// VHDL `rem`: the result has the sign of the left operand.
pub fn rem(a: i64, b: i64) -> ArithResult {
    if b == 0 {
        return Err(ArithError::DivisionByZero {
            op: Op::Rem,
            lhs: a,
        });
    }
    // i64::MIN rem -1 is 0 mathematically; checked_rem reports overflow.
    Ok(a.checked_rem(b).unwrap_or(0))
}

/// VHDL `mod`: the result has the sign of the right operand.
pub fn modulo(a: i64, b: i64) -> ArithResult {
    if b == 0 {
        return Err(ArithError::DivisionByZero {
            op: Op::Mod,
            lhs: a,
        });
    }
    let r = a.checked_rem(b).unwrap_or(0);
    Ok(if r != 0 && ((r < 0) != (b < 0)) {
        r + b
    } else {
        r
    })
}

pub fn neg(a: i64) -> ArithResult {
    a.checked_neg().ok_or(ArithError::Overflow {
        op: Op::Neg,
        lhs: a,
        rhs: None,
    })
}

pub fn abs(a: i64) -> ArithResult {
    a.checked_abs().ok_or(ArithError::Overflow {
        op: Op::Abs,
        lhs: a,
        rhs: None,
    })
}

/// Integer `**`; the exponent must be natural.
pub fn pow(base: i64, exp: i64) -> ArithResult {
    if exp < 0 {
        return Err(ArithError::NegativeExponent { base, exp });
    }
    match base {
        0 => return Ok(if exp == 0 { 1 } else { 0 }),
        1 => return Ok(1),
        -1 => return Ok(if exp % 2 == 0 { 1 } else { -1 }),
        _ => {}
    }
    // |base| >= 2, so any exponent above 63 overflows.
    let e = u32::try_from(exp).map_err(|_| ovf(Op::Pow, base, exp))?;
    base.checked_pow(e).ok_or(ovf(Op::Pow, base, exp))
}

/// Check a value against the bounds of its subtype (`low <= v <= high`).
pub fn check_range(value: i64, low: i64, high: i64) -> ArithResult {
    if value < low || value > high {
        Err(ArithError::OutOfRange { value, low, high })
    } else {
        Ok(value)
    }
}

/// Convert a real to an integer the VHDL way (round to nearest, ties away
/// from zero; LRM 9.3.6), failing when it does not fit in 64 bits.
pub fn real_to_int(op: Op, x: f64) -> ArithResult {
    let r = x.round();
    // 2^63 is exactly representable; anything at or above it does not fit.
    if !r.is_finite() || r >= 9_223_372_036_854_775_808.0 || r < -9_223_372_036_854_775_808.0 {
        return Err(ArithError::RealOverflow { op, value: x });
    }
    Ok(r as i64)
}

/// Physical times real (`5 ns * 1.5`), rounded to the primary unit.
pub fn phys_mul_real(a: i64, r: f64) -> ArithResult {
    real_to_int(Op::Mul, a as f64 * r)
}

/// Physical divided by real (`5 ns / 1.5`), rounded to the primary unit.
pub fn phys_div_real(a: i64, r: f64) -> ArithResult {
    if r == 0.0 {
        return Err(ArithError::DivisionByZero {
            op: Op::Div,
            lhs: a,
        });
    }
    real_to_int(Op::Div, a as f64 / r)
}

/// The resolution limit of TIME: the unit every TIME value is a multiple of.
/// It is a run-time setting, never baked into code (plan rule 7). Values are
/// always stored in femtoseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum TimeResolution {
    Fs,
    Ps,
    Ns,
    Us,
    Ms,
    Sec,
}

impl TimeResolution {
    /// The resolution in femtoseconds.
    pub fn fs(self) -> i64 {
        match self {
            TimeResolution::Fs => 1,
            TimeResolution::Ps => 1_000,
            TimeResolution::Ns => 1_000_000,
            TimeResolution::Us => 1_000_000_000,
            TimeResolution::Ms => 1_000_000_000_000,
            TimeResolution::Sec => 1_000_000_000_000_000,
        }
    }

    /// Parse a resolution such as `"ps"` or `"1 ns"` (command-line form).
    pub fn parse(s: &str) -> Option<TimeResolution> {
        let s = s.trim();
        let s = s.strip_prefix('1').map(str::trim_start).unwrap_or(s);
        Some(match s.to_ascii_lowercase().as_str() {
            "fs" => TimeResolution::Fs,
            "ps" => TimeResolution::Ps,
            "ns" => TimeResolution::Ns,
            "us" => TimeResolution::Us,
            "ms" => TimeResolution::Ms,
            "sec" | "s" => TimeResolution::Sec,
            _ => return None,
        })
    }

    /// `std.env.resolution_limit`, in femtoseconds.
    pub fn resolution_limit(self) -> i64 {
        self.fs()
    }

    /// Whether a TIME value (in fs) is representable at this resolution.
    pub fn is_representable(self, fs: i64) -> bool {
        fs % self.fs() == 0
    }

    /// LRM 5.2.4.2: a time value whose magnitude is below the resolution
    /// limit is truncated to zero; elaboration applies this to literals.
    /// Values above the limit that are not multiples of it are not legal
    /// at this resolution and are rejected rather than rounded.
    pub fn quantize(self, fs: i64) -> Result<i64, ArithError> {
        let r = self.fs();
        if fs.unsigned_abs() < r as u64 {
            Ok(0)
        } else if fs % r == 0 {
            Ok(fs)
        } else {
            Err(ArithError::OutOfRange {
                value: fs,
                low: -r,
                high: r,
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn checked_ops() {
        assert!(add(i64::MAX, 1).is_err());
        assert!(sub(i64::MIN, 1).is_err());
        assert_eq!(sub(-i64::MAX, 1), Ok(i64::MIN));
        assert!(mul(i64::MAX, 2).is_err());
        assert!(div(i64::MIN, -1).is_err());
        assert!(div(1, 0).is_err());
        assert!(neg(i64::MIN).is_err());
        assert!(abs(i64::MIN).is_err());
        assert_eq!(rem(i64::MIN, -1), Ok(0));
        assert_eq!(div(-7, 2), Ok(-3));
        assert_eq!(rem(-7, 2), Ok(-1));
        assert_eq!(modulo(-7, 2), Ok(1));
        assert_eq!(modulo(7, -2), Ok(-1));
        assert_eq!(modulo(-6, 3), Ok(0));
        assert_eq!(pow(2, 62), Ok(1 << 62));
        assert!(pow(2, 63).is_err());
        assert_eq!(pow(-2, 63), Ok(i64::MIN));
        assert!(pow(3, 1 << 40).is_err());
        assert_eq!(pow(-1, 1 << 40), Ok(1));
        assert!(pow(2, -1).is_err());
        assert_eq!(pow(0, 0), Ok(1));
        assert!(check_range(1 << 31, i32::MIN as i64, i32::MAX as i64).is_err());
        assert_eq!(phys_mul_real(5_000_000, 1.5), Ok(7_500_000));
        assert_eq!(real_to_int(Op::Mul, -2.5), Ok(-3));
        assert!(real_to_int(Op::Mul, 9.3e18).is_err());
        assert_eq!(
            real_to_int(Op::Mul, -9_223_372_036_854_775_808.0),
            Ok(i64::MIN)
        );
    }

    #[test]
    fn resolution() {
        let ps = TimeResolution::parse("1 ps").unwrap();
        assert_eq!(ps.resolution_limit(), 1000);
        assert_eq!(ps.quantize(999), Ok(0));
        assert_eq!(ps.quantize(-999), Ok(0));
        assert_eq!(ps.quantize(5000), Ok(5000));
        assert!(ps.quantize(1500).is_err());
        assert_eq!(TimeResolution::parse("xs"), None);
    }
}
