//! The operators of [`V4`](super::V4) on values of 1 to 64 bits held as two words, the planes
//! `(a, b)` (bits above the width 0): no allocation. Each gives exactly what the `V4` operator
//! gives (`crates/yavs-verilog/tests/ops.rs` holds them against it, x and z bits included);
//! `None` where an operand falls outside what the word form computes, and the caller then takes
//! the `V4` operator.

/// A value: the `a` plane and the `b` plane.
pub type P = (u64, u64);

/// The bits of a value of `w` bits (1 to 64).
#[inline(always)]
pub fn mask(w: u32) -> u64 {
    if w >= 64 { !0 } else { (1u64 << w) - 1 }
}

#[inline(always)]
pub fn xs(w: u32) -> P {
    (mask(w), mask(w))
}

#[inline(always)]
fn bit(v: bool) -> P {
    (v as u64, 0)
}

const X: P = (1, 1);

/// The value as signed.
#[inline(always)]
fn sext(v: u64, w: u32) -> i64 {
    ((v << (64 - w)) as i64) >> (64 - w)
}

#[inline(always)]
pub fn not(w: u32, x: P) -> P {
    ((!x.0 | x.1) & mask(w), x.1)
}

#[inline(always)]
pub fn and(x: P, y: P) -> P {
    let zero = (!x.0 & !x.1) | (!y.0 & !y.1);
    // Above the width both planes are 0 on both sides: zero there.
    let unk = (x.1 | y.1) & !zero;
    (!zero, unk)
}

#[inline(always)]
pub fn or(x: P, y: P) -> P {
    let one = (x.0 & !x.1) | (y.0 & !y.1);
    let unk = (x.1 | y.1) & !one;
    (one | unk, unk)
}

#[inline(always)]
pub fn xor(x: P, y: P) -> P {
    let unk = x.1 | y.1;
    ((x.0 ^ y.0) | unk, unk)
}

#[inline(always)]
pub fn merge(x: P, y: P) -> P {
    let diff = (x.0 ^ y.0) | x.1 | y.1;
    ((x.0 & !diff) | diff, diff)
}

/// Bits `wa` and up of `w`, from the sign bit of `a` when `signed`.
#[inline(always)]
pub fn ext(w: u32, wa: u32, signed: bool, a: P) -> P {
    if w <= wa {
        return (a.0 & mask(w), a.1 & mask(w));
    }
    let hi = mask(w) & !mask(wa);
    let (fa, fb) = match signed {
        true => (a.0 >> (wa - 1) & 1, a.1 >> (wa - 1) & 1),
        false => (0, 0),
    };
    (
        a.0 | if fa != 0 { hi } else { 0 },
        a.1 | if fb != 0 { hi } else { 0 },
    )
}

/// `{hi, lo}` of at most 64 bits.
#[inline(always)]
pub fn cat(wlo: u32, hi: P, lo: P) -> P {
    (hi.0 << wlo | lo.0, hi.1 << wlo | lo.1)
}

/// `n` bits of `x` (`w` bits) from bit `at`; bits outside are x.
#[inline(always)]
pub fn sel(w: u32, n: u32, x: P, at: i64) -> P {
    let lo = at.max(0);
    let hi = at.saturating_add(n.into()).min(w.into());
    if lo >= hi {
        return xs(n);
    }
    let len = (hi - lo) as u32;
    let off = (lo - at) as u32;
    let keep = mask(len) << off;
    let (a, b) = ((x.0 >> lo) & mask(len), (x.1 >> lo) & mask(len));
    let out = mask(n) & !keep;
    (out | a << off, out | b << off)
}

/// The truth (5.1.9): 1, 0, or 2 for x.
#[inline(always)]
pub fn truth(x: P) -> u64 {
    if x.0 & !x.1 != 0 {
        1
    } else if x.1 == 0 {
        0
    } else {
        2
    }
}

#[inline(always)]
fn tbit(t: u64) -> P {
    match t {
        0 => (0, 0),
        1 => (1, 0),
        _ => X,
    }
}

/// A unary operator (`VUnOp` order: not, neg, lnot, &, ~&, |, ~|, ^, ~^, `$clog2`): `op` as
/// its index, so this module needs no IR type.
#[inline]
pub fn un(op: u8, w: u32, x: P) -> Option<P> {
    let m = mask(w);
    Some(match op {
        0 => not(w, x),
        1 => match x.1 {
            0 => (x.0.wrapping_neg() & m, 0),
            _ => xs(w),
        },
        2 => not(1, tbit(truth(x))),
        3 | 4 => {
            let r = if (!x.0 & !x.1 & m) != 0 {
                (0, 0)
            } else if x.1 == 0 {
                (1, 0)
            } else {
                X
            };
            if op == 4 { not(1, r) } else { r }
        }
        5 | 6 => {
            let r = tbit(truth(x));
            if op == 6 { not(1, r) } else { r }
        }
        7 | 8 => {
            let r = match x.1 {
                0 => bit(x.0.count_ones() % 2 == 1),
                _ => X,
            };
            if op == 8 { not(1, r) } else { r }
        }
        9 => match x.1 {
            0 if x.0 <= 1 => (0, 0),
            0 => ((64 - (x.0 - 1).leading_zeros()).into(), 0),
            _ => xs(32),
        },
        _ => return None,
    })
}

/// Binary operators (`VBinOp` order: + - * / % ** & | ^ ~^ << >> >>> < <= > >= == != === !==
/// && ||, `**` as 5 with its exponent's sign in `es`), `x` of `w` bits, `y` of `wy`.
#[inline]
#[allow(clippy::too_many_arguments)]
pub fn bin(op: u8, signed: bool, es: bool, w: u32, wy: u32, x: P, y: P) -> Option<P> {
    let m = mask(w);
    let known = x.1 | y.1 == 0;
    Some(match op {
        0..=5 if !known => xs(w),
        0 => (x.0.wrapping_add(y.0) & m, 0),
        1 => (x.0.wrapping_sub(y.0) & m, 0),
        2 => (x.0.wrapping_mul(y.0) & m, 0),
        3 | 4 if y.0 == 0 => xs(w),
        3 | 4 => {
            let v = match (signed, op) {
                (false, 3) => x.0 / y.0,
                (false, _) => x.0 % y.0,
                (true, 3) => sext(x.0, w).wrapping_div(sext(y.0, w)) as u64,
                (true, _) => sext(x.0, w).wrapping_rem(sext(y.0, w)) as u64,
            };
            (v & m, 0)
        }
        5 => {
            if es && y.0 >> (wy - 1) & 1 == 1 {
                // A negative exponent (Table 5-6).
                if x.0 == 0 {
                    xs(w)
                } else if x.0 == 1 {
                    (1, 0)
                } else if signed && x.0 == m {
                    if y.0 & 1 == 1 { (m, 0) } else { (1, 0) }
                } else {
                    (0, 0)
                }
            } else {
                let (mut r, mut b) = (1u64, x.0);
                for i in 0..wy {
                    if y.0 >> i & 1 == 1 {
                        r = r.wrapping_mul(b);
                    }
                    b = b.wrapping_mul(b);
                }
                (r & m, 0)
            }
        }
        6 => and(x, y),
        7 => or(x, y),
        8 => xor(x, y),
        9 => not(w, xor(x, y)),
        10..=12 => {
            if y.1 != 0 {
                return Some(xs(w));
            }
            let k = y.0;
            let fill = match op {
                12 if signed => (x.0 >> (w - 1) & 1, x.1 >> (w - 1) & 1),
                _ => (0, 0),
            };
            if k >= u64::from(w) {
                return Some(match op {
                    10 => (0, 0),
                    _ => (fill.0.wrapping_neg() & m, fill.1.wrapping_neg() & m),
                });
            }
            match op {
                10 => ((x.0 << k) & m, (x.1 << k) & m),
                _ => {
                    let top = m & !(m >> k);
                    let f = |v: u64, s: u64| (v >> k) | if s != 0 { top } else { 0 };
                    (f(x.0, fill.0), f(x.1, fill.1))
                }
            }
        }
        13..=16 if !known => X,
        13..=16 => {
            let o = match signed {
                true => sext(x.0, w).cmp(&sext(y.0, w)),
                false => x.0.cmp(&y.0),
            };
            bit(match op {
                13 => o.is_lt(),
                14 => o.is_le(),
                15 => o.is_gt(),
                _ => o.is_ge(),
            })
        }
        17 | 18 => {
            let k = !x.1 & !y.1 & m;
            let r = if (x.0 ^ y.0) & k != 0 {
                (0, 0)
            } else if !known {
                X
            } else {
                (1, 0)
            };
            if op == 18 { not(1, r) } else { r }
        }
        19 => bit(x == y),
        20 => bit(x != y),
        21 => and(tbit(truth(x)), tbit(truth(y))),
        22 => or(tbit(truth(x)), tbit(truth(y))),
        _ => return None,
    })
}

/// A `case` item match (9.5.1): `===`, or z bits (`wild`) and x bits too (`x`) on either side
/// match anything.
#[inline(always)]
pub fn case_match(wild: bool, x_wild: bool, a: P, b: P) -> bool {
    if !wild {
        return a == b;
    }
    let z = (!a.0 & a.1) | (!b.0 & b.1);
    let x = if x_wild { (a.0 & a.1) | (b.0 & b.1) } else { 0 };
    ((a.0 ^ b.0) | (a.1 ^ b.1)) & !(z | x) == 0
}

/// The value as a count (`V4::to_i64`, 0 when unknown or negative).
#[inline(always)]
pub fn count(w: u32, signed: bool, x: P) -> i64 {
    if x.1 != 0 {
        return 0;
    }
    match signed {
        true => sext(x.0, w).max(0),
        false => i64::try_from(x.0).unwrap_or(0),
    }
}

/// The value as a real (`V4::to_f64`) when it is below 2^53 in magnitude, where every way of
/// summing the bits is exact; `None` otherwise.
#[inline(always)]
pub fn to_f64(w: u32, signed: bool, x: P) -> Option<f64> {
    let neg = signed && x.1 == 0 && x.0 >> (w - 1) & 1 == 1;
    let v = match neg {
        true => x.0.wrapping_neg() & mask(w),
        false => x.0 & !x.1,
    };
    (v < 1 << 53).then(|| if neg { -(v as f64) } else { v as f64 })
}
