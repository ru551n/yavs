//! The 4-state operators of `value.rs` against a small reference model written from the tables of
//! IEEE 1364-2005 clause 5: values as strings of `0 1 x z` (index 0 the least significant bit),
//! numbers as big integers, every rule spelled out bit by bit. Widths 1 to 130.

use num_bigint::{BigInt, BigUint, Sign};
use proptest::prelude::*;
use yavs_verilog::value::{Bit, V4};

type M = Vec<u8>;

fn to_v4(m: &M) -> V4 {
    let mut v = V4::zeros(m.len() as u32);
    for (i, c) in m.iter().enumerate() {
        v.set(
            i as u32,
            match c {
                b'0' => Bit::Zero,
                b'1' => Bit::One,
                b'x' => Bit::X,
                _ => Bit::Z,
            },
        );
    }
    v
}

fn of_v4(v: &V4) -> M {
    (0..v.width()).map(|i| v.get(i).char() as u8).collect()
}

fn known(m: &M) -> bool {
    m.iter().all(|&c| c == b'0' || c == b'1')
}

fn xs(w: usize) -> M {
    vec![b'x'; w]
}

fn unsigned(m: &M) -> BigUint {
    let mut n = BigUint::from(0u8);
    for &c in m.iter().rev() {
        n = n * 2u8 + u8::from(c == b'1');
    }
    n
}

fn int(m: &M, signed: bool) -> BigInt {
    let u = BigInt::from(unsigned(m));
    if signed && m[m.len() - 1] == b'1' {
        u - (BigInt::from(1) << m.len())
    } else {
        u
    }
}

/// `n` modulo 2^w as `w` bits.
fn bits(n: &BigInt, w: usize) -> M {
    let m = BigInt::from(1) << w;
    let mut r = ((n % &m) + &m) % &m;
    let mut out = Vec::with_capacity(w);
    for _ in 0..w {
        out.push(if (&r & BigInt::from(1)) == BigInt::from(1) {
            b'1'
        } else {
            b'0'
        });
        r >>= 1;
    }
    out
}

fn one_bit(c: u8) -> M {
    vec![c]
}

fn b(v: bool) -> u8 {
    if v { b'1' } else { b'0' }
}

/// Table 5-11 style truth tables; z is read as x.
fn and1(p: u8, q: u8) -> u8 {
    match (p, q) {
        (b'0', _) | (_, b'0') => b'0',
        (b'1', b'1') => b'1',
        _ => b'x',
    }
}

fn or1(p: u8, q: u8) -> u8 {
    match (p, q) {
        (b'1', _) | (_, b'1') => b'1',
        (b'0', b'0') => b'0',
        _ => b'x',
    }
}

fn xor1(p: u8, q: u8) -> u8 {
    match (p, q) {
        (b'0' | b'1', b'0' | b'1') => b(p != q),
        _ => b'x',
    }
}

fn not1(p: u8) -> u8 {
    match p {
        b'0' => b'1',
        b'1' => b'0',
        _ => b'x',
    }
}

fn bitwise(l: &M, r: &M, f: fn(u8, u8) -> u8) -> M {
    l.iter().zip(r).map(|(&p, &q)| f(p, q)).collect()
}

fn truth(m: &M) -> u8 {
    if m.contains(&b'1') {
        b'1'
    } else if known(m) {
        b'0'
    } else {
        b'x'
    }
}

fn arith(l: &M, r: &M, f: impl Fn(BigInt, BigInt) -> Option<BigInt>, signed: bool) -> M {
    if !known(l) || !known(r) {
        return xs(l.len());
    }
    match f(int(l, signed), int(r, signed)) {
        Some(n) => bits(&n, l.len()),
        None => xs(l.len()),
    }
}

/// Table 5-6 of the power operator.
fn pow(l: &M, r: &M, signed: bool, r_signed: bool) -> M {
    let w = l.len();
    if !known(l) || !known(r) {
        return xs(w);
    }
    let (base, e) = (int(l, signed), int(r, r_signed));
    let zero = BigInt::from(0);
    if e < zero {
        let n = if base == zero {
            return xs(w);
        } else if base == BigInt::from(1) {
            BigInt::from(1)
        } else if base == BigInt::from(-1) {
            if (&e % 2) == zero {
                BigInt::from(1)
            } else {
                BigInt::from(-1)
            }
        } else {
            zero
        };
        return bits(&n, w);
    }
    let m = BigUint::from(1u8) << w;
    let ub = match (base % BigInt::from(m.clone()) + BigInt::from(m.clone())).to_biguint() {
        Some(u) => u % &m,
        None => unreachable!(),
    };
    let p = ub.modpow(&e.to_biguint().unwrap(), &m);
    bits(&BigInt::from_biguint(Sign::Plus, p), w)
}

fn shift(l: &M, r: &M, left: bool, fill: u8) -> M {
    let w = l.len();
    if !known(r) {
        return xs(w);
    }
    let k = unsigned(r);
    (0..w)
        .map(|i| {
            let src = if left {
                // Bit i comes from i - k.
                if BigUint::from(i) >= k {
                    Some(i - usize::try_from(&k).unwrap())
                } else {
                    return b'0';
                }
            } else {
                let s = k.clone() + i;
                (s < BigUint::from(w)).then(|| usize::try_from(&s).unwrap())
            };
            src.map_or(fill, |s| l[s])
        })
        .collect()
}

fn cmp(l: &M, r: &M, signed: bool, f: fn(&BigInt, &BigInt) -> bool) -> M {
    if !known(l) || !known(r) {
        return one_bit(b'x');
    }
    one_bit(b(f(&int(l, signed), &int(r, signed))))
}

fn eq(l: &M, r: &M) -> M {
    let differ = l
        .iter()
        .zip(r)
        .any(|(&p, &q)| (p == b'0' || p == b'1') && (q == b'0' || q == b'1') && p != q);
    if differ {
        one_bit(b'0')
    } else if known(l) && known(r) {
        one_bit(b'1')
    } else {
        one_bit(b'x')
    }
}

fn ext(m: &M, w: usize, signed: bool) -> M {
    let mut r: M = m.iter().take(w).copied().collect();
    let fill = if signed { m[m.len() - 1] } else { b'0' };
    r.resize(w, fill);
    r
}

fn merge(l: &M, r: &M) -> M {
    l.iter()
        .zip(r)
        .map(|(&p, &q)| {
            if p == q && (p == b'0' || p == b'1') {
                p
            } else {
                b'x'
            }
        })
        .collect()
}

/// A value of width `w`: random bits, sometimes with x and z, sometimes a corner value.
fn value(w: usize) -> impl Strategy<Value = M> {
    let corner = prop::sample::select(vec![0u8, 1, 2, 3, 4]).prop_map(move |k| match k {
        0 => vec![b'0'; w],
        1 => vec![b'1'; w],
        2 => {
            let mut m = vec![b'0'; w];
            m[w - 1] = b'1';
            m
        }
        3 => {
            let mut m = vec![b'0'; w];
            m[0] = b'1';
            m
        }
        _ => {
            let mut m = vec![b'1'; w];
            m[w - 1] = b'0';
            m
        }
    });
    let known = prop::collection::vec(prop::sample::select(vec![b'0', b'1']), w);
    let mixed = prop::collection::vec(
        prop::sample::select(vec![b'0', b'1', b'0', b'1', b'0', b'1', b'x', b'z']),
        w,
    );
    prop_oneof![3 => known, 1 => mixed, 1 => corner]
}

fn pair() -> impl Strategy<Value = (M, M, bool)> {
    (1usize..=130).prop_flat_map(|w| (value(w), value(w), any::<bool>()))
}

/// A left operand and a self-determined right operand of another width.
fn uneven() -> impl Strategy<Value = (M, M, bool, bool)> {
    (1usize..=130, 1usize..=70)
        .prop_flat_map(|(w, v)| (value(w), value(v), any::<bool>(), any::<bool>()))
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(600))]

    #[test]
    fn bitwise_operators((l, r, _) in pair()) {
        let (vl, vr) = (to_v4(&l), to_v4(&r));
        prop_assert_eq!(of_v4(&vl.and(&vr)), bitwise(&l, &r, and1));
        prop_assert_eq!(of_v4(&vl.or(&vr)), bitwise(&l, &r, or1));
        prop_assert_eq!(of_v4(&vl.xor(&vr)), bitwise(&l, &r, xor1));
        prop_assert_eq!(of_v4(&vl.xnor(&vr)), bitwise(&l, &r, |p, q| not1(xor1(p, q))));
        prop_assert_eq!(of_v4(&vl.not()), l.iter().map(|&p| not1(p)).collect::<M>());
    }

    #[test]
    fn arithmetic((l, r, s) in pair()) {
        let (vl, vr) = (to_v4(&l), to_v4(&r));
        prop_assert_eq!(of_v4(&vl.add(&vr)), arith(&l, &r, |a, b| Some(a + b), s));
        prop_assert_eq!(of_v4(&vl.sub(&vr)), arith(&l, &r, |a, b| Some(a - b), s));
        prop_assert_eq!(of_v4(&vl.mul(&vr)), arith(&l, &r, |a, b| Some(a * b), s));
        prop_assert_eq!(of_v4(&vl.neg()), arith(&l, &l, |a, _| Some(-a), s));
        // Truncation towards zero; the remainder takes the sign of the dividend.
        prop_assert_eq!(
            of_v4(&vl.div(&vr, s)),
            arith(&l, &r, |a, b| (b != BigInt::from(0)).then(|| a / b), s)
        );
        prop_assert_eq!(
            of_v4(&vl.rem(&vr, s)),
            arith(&l, &r, |a, b| (b != BigInt::from(0)).then(|| a % b), s)
        );
    }

    #[test]
    fn power((l, r, s, rs) in uneven()) {
        prop_assert_eq!(of_v4(&to_v4(&l).pow(&to_v4(&r), s, rs)), pow(&l, &r, s, rs));
    }

    #[test]
    fn shifts((l, r, s, _) in uneven()) {
        let (vl, vr) = (to_v4(&l), to_v4(&r));
        prop_assert_eq!(of_v4(&vl.shl(&vr)), shift(&l, &r, true, b'0'));
        prop_assert_eq!(of_v4(&vl.shr(&vr, false)), shift(&l, &r, false, b'0'));
        let fill = if s { l[l.len() - 1] } else { b'0' };
        prop_assert_eq!(of_v4(&vl.shr(&vr, s)), shift(&l, &r, false, fill));
    }

    #[test]
    fn relations((l, r, s) in pair()) {
        let (vl, vr) = (to_v4(&l), to_v4(&r));
        prop_assert_eq!(of_v4(&vl.lt(&vr, s)), cmp(&l, &r, s, |a, b| a < b));
        prop_assert_eq!(of_v4(&vl.le(&vr, s)), cmp(&l, &r, s, |a, b| a <= b));
        prop_assert_eq!(of_v4(&vl.gt(&vr, s)), cmp(&l, &r, s, |a, b| a > b));
        prop_assert_eq!(of_v4(&vl.ge(&vr, s)), cmp(&l, &r, s, |a, b| a >= b));
        prop_assert_eq!(of_v4(&vl.eq(&vr)), eq(&l, &r));
        prop_assert_eq!(of_v4(&vl.ne(&vr)), eq(&l, &r).iter().map(|&p| not1(p)).collect::<M>());
        prop_assert_eq!(of_v4(&vl.case_eq(&vr)), one_bit(b(l == r)));
        prop_assert_eq!(of_v4(&vl.case_eq(&vl)), one_bit(b'1'));
    }

    #[test]
    fn logical_reduction_and_merge((l, r, _) in pair()) {
        let (vl, vr) = (to_v4(&l), to_v4(&r));
        prop_assert_eq!(of_v4(&vl.land(&vr)), one_bit(and1(truth(&l), truth(&r))));
        prop_assert_eq!(of_v4(&vl.lor(&vr)), one_bit(or1(truth(&l), truth(&r))));
        prop_assert_eq!(of_v4(&vl.lnot()), one_bit(not1(truth(&l))));
        let red = |f: fn(u8, u8) -> u8| one_bit(l[1..].iter().fold(l[0], |a, &c| f(a, c)));
        // Reductions fold the bit table over the vector; a single z still reads as x.
        let fix = |m: M| m.into_iter().map(|c| if c == b'z' { b'x' } else { c }).collect::<M>();
        prop_assert_eq!(of_v4(&vl.red_and()), fix(red(and1)));
        prop_assert_eq!(of_v4(&vl.red_or()), fix(red(or1)));
        prop_assert_eq!(of_v4(&vl.red_xor()), fix(red(xor1)));
        prop_assert_eq!(of_v4(&vl.merge(&vr)), merge(&l, &r));
    }

    #[test]
    fn extension_and_selection((l, _, s) in pair(), w in 1u32..=140, lo in -3i64..140, n in 1u32..20) {
        let vl = to_v4(&l);
        prop_assert_eq!(of_v4(&vl.ext(w, s)), ext(&l, w as usize, s));
        let want: M = (0..n as i64)
            .map(|i| {
                let k = lo + i;
                if (0..l.len() as i64).contains(&k) { l[k as usize] } else { b'x' }
            })
            .collect();
        prop_assert_eq!(of_v4(&vl.slice(lo, n)), want);
        prop_assert_eq!(V4::from_logic(&vl.to_logic()), vl);
    }
}

#[test]
fn integers_round_trip() {
    for w in [1u32, 7, 32, 63, 64, 65, 130] {
        for v in [0i64, 1, -1, 5, -6, i64::MIN, i64::MAX] {
            let x = V4::from_i64(w, v);
            let m = of_v4(&x);
            assert_eq!(m, bits(&BigInt::from(v), w as usize), "w={w} v={v}");
            if w >= 64 {
                assert_eq!(x.to_i64(true), Some(v));
            }
        }
    }
}
