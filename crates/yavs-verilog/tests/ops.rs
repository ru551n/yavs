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

// The word forms (`value::w64`, 1 to 64 bits) against the operators above: the interpreter and
// the compiled tiers take them for narrow values, so they must give what `V4` gives, also for a
// value that just falls off the two-state path (one x or z bit).

use yavs_verilog::value::w64;

/// A narrow value: known, known with one x or z bit, or as `value`.
fn narrow(w: usize) -> impl Strategy<Value = M> {
    let one_off = (
        prop::collection::vec(prop::sample::select(vec![b'0', b'1']), w),
        0..w,
        prop::sample::select(vec![b'x', b'z']),
    )
        .prop_map(|(mut m, i, c)| {
            m[i] = c;
            m
        });
    prop_oneof![2 => value(w), 1 => one_off]
}

fn p(v: &V4) -> w64::P {
    let (a, b) = v.planes();
    (a[0], b[0])
}

fn of_p(w: u32, x: w64::P) -> V4 {
    V4::from_planes(w, &[x.0], &[x.1])
}

/// `VBinOp` in its order, as `w64::bin` numbers it.
fn v4_bin(op: u8, s: bool, es: bool, a: &V4, b: &V4) -> V4 {
    match op {
        0 => a.add(b),
        1 => a.sub(b),
        2 => a.mul(b),
        3 => a.div(b, s),
        4 => a.rem(b, s),
        5 => a.pow(b, s, es),
        6 => a.and(b),
        7 => a.or(b),
        8 => a.xor(b),
        9 => a.xnor(b),
        10 => a.shl(b),
        11 => a.shr(b, false),
        12 => a.shr(b, s),
        13 => a.lt(b, s),
        14 => a.le(b, s),
        15 => a.gt(b, s),
        16 => a.ge(b, s),
        17 => a.eq(b),
        18 => a.ne(b),
        19 => a.case_eq(b),
        20 => a.case_eq(b).not(),
        21 => a.land(b),
        _ => a.lor(b),
    }
}

fn v4_un(op: u8, a: &V4) -> V4 {
    match op {
        0 => a.not(),
        1 => a.neg(),
        2 => a.lnot(),
        3 => a.red_and(),
        4 => a.red_and().not(),
        5 => a.red_or(),
        6 => a.red_or().not(),
        7 => a.red_xor(),
        8 => a.red_xor().not(),
        _ => a.clog2(),
    }
}

fn narrow_pair() -> impl Strategy<Value = (M, M, M, bool, bool)> {
    (1usize..=64, 1usize..=64).prop_flat_map(|(w, v)| {
        (
            narrow(w),
            narrow(w),
            narrow(v),
            any::<bool>(),
            any::<bool>(),
        )
    })
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(1500))]

    #[test]
    fn word_forms_match_the_vectors((l, r, s, signed, es) in narrow_pair(), lo in -70i64..70, n in 1u32..=64, to in 1u32..=64) {
        let (vl, vr, vs) = (to_v4(&l), to_v4(&r), to_v4(&s));
        let (w, ws) = (vl.width(), vs.width());
        for op in 0..23u8 {
            // Shifts and `**` take the self-determined operand.
            let (y, wy, vy) = if (10..=12).contains(&op) || op == 5 { (p(&vs), ws, &vs) } else { (p(&vr), w, &vr) };
            let want = v4_bin(op, signed, es, &vl, vy);
            let got = w64::bin(op, signed, es, w, wy, p(&vl), y).map(|x| of_p(want.width(), x));
            prop_assert_eq!(got, Some(want), "op {}", op);
        }
        for op in 0..10u8 {
            let want = v4_un(op, &vl);
            prop_assert_eq!(w64::un(op, w, p(&vl)).map(|x| of_p(want.width(), x)), Some(want), "un {}", op);
        }
        prop_assert_eq!(of_p(w, w64::merge(p(&vl), p(&vr))), vl.merge(&vr));
        prop_assert_eq!(of_p(to, w64::ext(to, w, signed, p(&vl))), vl.ext(to, signed));
        prop_assert_eq!(of_p(n, w64::sel(w, n, p(&vl), lo)), vl.slice(lo, n));
        prop_assert_eq!(of_p(n, w64::sel(w, n, p(&vl), i64::MIN)), V4::xs(n));
        if w + ws <= 64 {
            prop_assert_eq!(of_p(w + ws, w64::cat(ws, p(&vl), p(&vs))), V4::concat(&vl, &vs));
        }
        let t = match vl.truth() { Bit::One => 1, Bit::Zero => 0, _ => 2 };
        prop_assert_eq!(w64::truth(p(&vl)), t);
        for (wild, x) in [(false, false), (true, false), (true, true)] {
            prop_assert_eq!(w64::case_match(wild, x, p(&vl), p(&vr)), vl.case_match(&vr, wild, x));
        }
        prop_assert_eq!(w64::count(w, signed, p(&vl)), vl.to_i64(signed).unwrap_or(0).max(0));
        if let Some(f) = w64::to_f64(w, signed, p(&vl)) {
            prop_assert_eq!(f, vl.to_f64(signed));
        }
        // Through a kernel vector and back, word by word.
        let mut buf = [0u64; 2];
        yavs_verilog::value::logic_planes(&vl.to_logic(), &mut buf);
        prop_assert_eq!(of_p(w, (buf[0], buf[1])), vl.clone());
    }
}

/// Kernel vectors with the codes VHDL adds (U, W, L, H, -) read as `TO_X01Z`, at block edges.
#[test]
fn logic_planes_read_every_code() {
    use yavs_logic::{LogicVec, StdULogic};
    for len in [1usize, 63, 64, 65, 130] {
        let l = LogicVec::from_fn(len, |i| yavs_logic::ALL[i % 9]);
        let n = len.div_ceil(64);
        let mut buf = vec![0u64; 2 * n];
        yavs_verilog::value::logic_planes(&l, &mut buf);
        let v = V4::from_planes(len as u32, &buf[..n], &buf[n..]);
        for i in 0..len {
            let want = yavs_verilog::value::of_ulogic(l.get(i));
            assert_eq!(v.get(i as u32), want, "len {len} element {i}");
        }
        let back = yavs_verilog::value::logic_of_planes(len, &buf[..n], &buf[n..]);
        for i in 0..len {
            let c = back.get(i);
            assert!(matches!(
                c,
                StdULogic::Zero | StdULogic::One | StdULogic::X | StdULogic::Z
            ));
            assert_eq!(yavs_verilog::value::of_ulogic(c), v.get(i as u32));
        }
    }
}
