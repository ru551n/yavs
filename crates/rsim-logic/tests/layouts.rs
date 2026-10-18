//! Both layouts against a straightforward `Vec<StdULogic>` reference (element 0 = rightmost),
//! exhaustively over value pairs and triples, and by property tests.

use core::cmp::Ordering;
use proptest::prelude::*;
use rsim_logic::StdULogic::{self, *};
use rsim_logic::{ALL, Logic, Nibbles, Planes, Repr};

type V = Vec<StdULogic>;

fn zipv(a: &V, b: &V, f: fn(StdULogic, StdULogic) -> StdULogic) -> V {
    a.iter().zip(b).map(|(&x, &y)| f(x, y)).collect()
}
fn mapv(a: &V, f: fn(StdULogic) -> StdULogic) -> V {
    a.iter().map(|&x| f(x)).collect()
}
fn ext(a: &V, n: usize, signed: bool) -> V {
    let fill = if signed { *a.last().unwrap() } else { Zero };
    a.iter()
        .copied()
        .chain(std::iter::repeat(fill))
        .take(n)
        .collect()
}

/// numeric_std "+"/"-": ripple-carry on scalar ops after TO_01(.., 'X').
fn ref_add(a: &V, b: &V, signed: bool, sub: bool) -> V {
    let n = a.len().max(b.len());
    if a.is_empty() || b.is_empty() {
        return vec![];
    }
    let (a, b) = (ext(a, n, signed), ext(b, n, signed));
    if a.iter().chain(&b).any(|x| x.is_x()) {
        return vec![X; n];
    }
    let mut c = StdULogic::from_bool(sub);
    a.iter()
        .zip(&b)
        .map(|(&x, &y)| {
            let y = if sub { y.not() } else { y };
            let r = c.xor(x).xor(y);
            c = c.and(x).or(c.and(y)).or(x.and(y));
            r.to_x01()
        })
        .collect()
}
fn ref_cmp(a: &V, b: &V, signed: bool) -> Option<Ordering> {
    let n = a.len().max(b.len());
    if a.is_empty() || b.is_empty() || a.iter().chain(b).any(|x| x.is_x()) {
        return None;
    }
    let bits = |v: &V| {
        let mut e: Vec<bool> = ext(v, n, signed).iter().map(|x| x.to_bit(false)).collect();
        if signed {
            e[n - 1] = !e[n - 1];
        }
        e.reverse();
        e
    };
    Some(bits(a).cmp(&bits(b)))
}
fn ref_match(a: &V, b: &V) -> StdULogic {
    let mut r = One;
    for (&x, &y) in a.iter().zip(b) {
        match x.match_eq(y) {
            U => return U,
            X => r = X,
            m => r = r.and(m),
        }
    }
    r
}
fn ref_val(a: &V, signed: bool) -> Option<i128> {
    if a.iter().any(|x| x.is_x()) {
        return None;
    }
    if a.is_empty() {
        return Some(0); // numeric_std: null range converts to 0
    }
    let n = a.len();
    let mut v: i128 = 0;
    for (i, x) in a.iter().enumerate() {
        if x.to_bit(false) {
            if i >= 120 {
                return None;
            }
            v += 1 << i;
        }
    }
    if signed && a[n - 1].to_bit(false) && n < 120 {
        v -= 1 << n;
    }
    Some(v)
}

macro_rules! suite {
    ($m:ident, $R:ty) => {
        mod $m {
            use super::*;
            type L = Logic<$R>;

            fn l(v: &V) -> L {
                L::from_fn(v.len(), |i| v[i])
            }
            fn r(x: &L) -> V {
                x.iter().collect()
            }
            fn ck(got: L, want: &V, what: &str) {
                assert_eq!(r(&got), *want, "{what}");
                assert_eq!(got, l(want), "{what} (canonical form)");
            }
            fn unary(a: &V) {
                let x = l(a);
                ck(x.not(), &mapv(a, StdULogic::not), "not");
                ck(x.to_x01(), &mapv(a, StdULogic::to_x01), "x01");
                ck(x.to_x01z(), &mapv(a, StdULogic::to_x01z), "x01z");
                ck(x.to_ux01(), &mapv(a, StdULogic::to_ux01), "ux01");
            }
            fn binary(a: &V, b: &V) {
                let (x, y) = (l(a), l(b));
                ck(x.and(&y), &zipv(a, b, StdULogic::and), "and");
                ck(x.or(&y), &zipv(a, b, StdULogic::or), "or");
                ck(x.xor(&y), &zipv(a, b, StdULogic::xor), "xor");
                ck(x.nand(&y), &zipv(a, b, StdULogic::nand), "nand");
                ck(x.nor(&y), &zipv(a, b, StdULogic::nor), "nor");
                ck(x.xnor(&y), &zipv(a, b, StdULogic::xnor), "xnor");
                ck(L::resolve(&[&x, &y]), &zipv(a, b, StdULogic::resolve), "resolve");
                assert_eq!(x.match_eq(&y), ref_match(a, b));
            }

            #[test]
            fn exhaustive_pairs_at_every_position() {
                for s in 0..81 {
                    let a: V = (0..130).map(|i| ALL[((i + s) % 81) / 9]).collect();
                    let b: V = (0..130).map(|i| ALL[((i + s) % 81) % 9]).collect();
                    unary(&a);
                    binary(&a, &b);
                }
            }

            #[test]
            fn exhaustive_triples_resolve() {
                for s in 0..729 {
                    let d = |k: usize| -> V { (0..70).map(|i| ALL[((i + s) % 729) / 9usize.pow(k as u32) % 9]).collect() };
                    let (a, b, c) = (d(2), d(1), d(0));
                    let want = zipv(&zipv(&a, &b, StdULogic::resolve), &c, StdULogic::resolve);
                    ck(L::resolve(&[&l(&a), &l(&b), &l(&c)]), &want, "resolve3");
                }
            }

            #[test]
            fn exhaustive_numeric_small() {
                // All 9^2 element pairs and 9^3 triples as 1-3 element vectors.
                let vs: Vec<V> = (1..=3usize)
                    .flat_map(|n| (0..9usize.pow(n as u32)).map(move |k| (0..n).map(|i| ALL[k / 9usize.pow(i as u32) % 9]).collect::<V>()))
                    .collect();
                for a in vs.iter().filter(|v| v.len() <= 2) {
                    for b in &vs {
                        numeric(a, b);
                    }
                }
            }

            fn numeric(a: &V, b: &V) {
                let (x, y) = (l(a), l(b));
                for signed in [false, true] {
                    ck(x.add_num(&y, signed, false), &ref_add(a, b, signed, false), "add");
                    ck(x.add_num(&y, signed, true), &ref_add(a, b, signed, true), "sub");
                    assert_eq!(x.cmp_num(&y, signed), ref_cmp(a, b, signed), "cmp {a:?} {b:?} {signed}");
                }
            }

            #[test]
            fn scalar_roundtrips() {
                for (n, v) in [(1, 1u64), (8, 200), (64, u64::MAX), (70, u64::MAX), (3, 9)] {
                    let x = L::from_u64(n, v);
                    let m = if n >= 64 { v } else { v & ((1 << n) - 1) };
                    assert_eq!(x.to_u64(), Some(m));
                    assert_eq!(x.to_string().len(), n);
                }
                for (n, v) in [(8, -1i64), (8, -128), (64, i64::MIN), (100, -5), (100, 5), (4, 7)] {
                    assert_eq!(L::from_i64(n, v).to_i64(), Some(v), "{n} {v}");
                }
                assert_eq!(L::from_i64(8, -2).to_string(), "11111110");
                assert_eq!(L::from_i64(4, 9).to_i64(), Some(-7));
                assert_eq!(L::from_u64(70, u64::MAX).to_i64(), Some(i64::MAX).filter(|_| false));
                assert_eq!("01XZ".parse::<L>().unwrap().to_u64(), None);
                assert_eq!("01LH".parse::<L>().unwrap().to_u64(), Some(0b0101));
                assert_eq!("0a".parse::<L>().err(), Some('a'));
                let s = "UX01ZWLH-";
                assert_eq!(s.parse::<L>().unwrap().to_string(), s);
                assert_eq!(s.parse::<L>().unwrap().get(0), DontCare);
            }

            #[test]
            fn edge_cases() {
                let e = L::new(0, U);
                assert!(e.is_empty() && e.not().is_empty() && e.add_num(&e, false, false).is_empty());
                assert_eq!(e.to_string(), "");
                assert_eq!(e.match_eq(&e), One);
                // A single driver passes through, even '-'.
                let d = L::new(70, DontCare);
                assert_eq!(L::resolve(&[&d]), d);
                assert_eq!(L::resolve(&[&d, &d]), L::new(70, X));
            }

            fn vec_strat() -> impl Strategy<Value = V> {
                prop_oneof![
                    prop::collection::vec(0usize..9, 0..200).prop_map(|v| v.into_iter().map(|i| ALL[i]).collect::<V>()),
                    prop::collection::vec(prop::sample::select(vec![Zero, One, L_, H]), 0..200),
                ]
            }
            const L_: StdULogic = StdULogic::L;
            fn pair() -> impl Strategy<Value = (V, V)> {
                (0usize..200).prop_flat_map(|n| {
                    let s = || prop::collection::vec(prop::sample::select(ALL.to_vec()), n);
                    let t = || prop::collection::vec(prop::sample::select(vec![Zero, One, L_, H]), n);
                    (prop_oneof![s(), t()], prop_oneof![s(), t()])
                })
            }

            proptest! {
                #![proptest_config(ProptestConfig::with_cases(300))]

                #[test]
                fn prop_unary(a in vec_strat()) { unary(&a); }

                #[test]
                fn prop_binary((a, b) in pair()) { binary(&a, &b); }

                #[test]
                fn prop_resolve_n(ds in prop::collection::vec(prop::collection::vec(prop::sample::select(ALL.to_vec()), 150), 1..6)) {
                    let xs: Vec<L> = ds.iter().map(l).collect();
                    let refs: Vec<&L> = xs.iter().collect();
                    let want = ds[1..].iter().fold(ds[0].clone(), |acc, d| zipv(&acc, d, StdULogic::resolve));
                    ck(L::resolve(&refs), &want, "resolve_n");
                }

                #[test]
                fn prop_numeric(a in vec_strat(), b in vec_strat()) { numeric(&a, &b); }

                #[test]
                fn prop_values(a in vec_strat(), v in any::<i64>(), n in 1usize..100) {
                    let x = l(&a);
                    prop_assert_eq!(x.to_u64(), ref_val(&a, false).and_then(|v| u64::try_from(v).ok()));
                    prop_assert_eq!(x.to_i64(), ref_val(&a, true).and_then(|v| i64::try_from(v).ok()));
                    let s = L::from_i64(n, v);
                    prop_assert_eq!(ref_val(&r(&s), true), Some(if n >= 64 { v as i128 } else { ((v as i128) << (128 - n)) >> (128 - n) }));
                    let u = L::from_u64(n, v as u64);
                    prop_assert_eq!(u.to_string().parse::<L>().unwrap(), u);
                }

                #[test]
                fn prop_structure(a in vec_strat(), b in vec_strat(), k in 0usize..300) {
                    let x = l(&a);
                    prop_assert_eq!(x.to_string().chars().rev().map(|c| StdULogic::from_char(c).unwrap()).collect::<V>(), a.clone());
                    prop_assert_eq!(x.to_string().parse::<L>().unwrap(), x.clone());
                    let cat: V = a.iter().chain(&b).copied().collect();
                    ck(l(&b).concat(&x), &cat, "concat");
                    if !a.is_empty() {
                        let lo = k % a.len();
                        let n = (k / 3) % (a.len() - lo + 1);
                        ck(x.slice(lo, n), &a[lo..lo + n].to_vec(), "slice");
                        let n = a.len();
                        let z = |i| if i >= k { a[i - k] } else { Zero };
                        ck(x.shl(k), &(0..n).map(z).collect(), "shl");
                        ck(x.shr(k), &(0..n).map(|i| if i + k < n { a[i + k] } else { Zero }).collect(), "shr");
                        ck(x.sra(k), &(0..n).map(|i| if i + k < n { a[i + k] } else { a[n - 1] }).collect(), "sra");
                        let mut rot = a.clone();
                        rot.rotate_right(k % n);
                        ck(x.rol(k), &rot, "rol");
                        let mut rot = a.clone();
                        rot.rotate_left(k % n);
                        ck(x.ror(k), &rot, "ror");
                    }
                }

                #[test]
                fn prop_bitvec(a in vec_strat()) {
                    let x = l(&a);
                    let b = x.to_bitvec();
                    prop_assert_eq!(b.is_some(), !a.iter().any(|v| v.is_x()));
                    if let Some(b) = b {
                        prop_assert_eq!(L::from_bitvec(&b), x.to_x01());
                        prop_assert_eq!(b.not().to_u64().is_some() || a.len() > 64, true);
                    }
                }
            }
        }
    };
}

suite!(nibbles, Nibbles);
suite!(planes, Planes);

#[test]
fn layouts_agree_on_canonical_form() {
    let n: Logic<Nibbles> = "UX01ZWLH-01".parse().unwrap();
    let p: Logic<Planes> = "UX01ZWLH-01".parse().unwrap();
    assert_eq!(n.to_string(), p.to_string());
    assert_eq!(<Nibbles as Repr>::new(3, U).len(), 3);
}

#[test]
fn scalar_helpers() {
    assert!(StdULogic::rising_edge(L, H) && StdULogic::rising_edge(Zero, One));
    assert!(!StdULogic::rising_edge(X, One) && !StdULogic::rising_edge(One, One));
    assert!(StdULogic::falling_edge(H, L) && !StdULogic::falling_edge(One, X));
    assert!(U.is_x() && Z.is_x() && DontCare.is_x() && !H.is_x() && !L.is_x());
    assert!(H.is_true() && One.is_true() && !L.is_true());
    assert!(H.to_bit(false) && !Z.to_bit(false) && Z.to_bit(true));
    assert_eq!(
        (Z.to_x01z(), Z.to_x01(), U.to_ux01(), H.to_x01()),
        (Z, X, U, One)
    );
}
