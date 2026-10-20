//! The block-slice kernels of `yavs_logic::wide` against per-element scalar models, for random
//! planes over all nine values, odd lengths and every bit offset.

use proptest::prelude::*;
use yavs_logic::StdULogic::{self, *};
use yavs_logic::wide::{self, Z_BLOCK};
use yavs_logic::{ALL, BitVec, Block, Logic, Planes, Repr};

/// Elements, element 0 first.
type V = Vec<StdULogic>;

fn blocks(v: &V) -> Vec<Block> {
    let mut b = vec![[0u64; 4]; v.len().div_ceil(64)];
    for (i, &x) in v.iter().enumerate() {
        for (k, plane) in b[i / 64].iter_mut().enumerate() {
            *plane |= ((x as u64 >> k) & 1) << (i % 64);
        }
    }
    b
}
fn unblock(b: &[Block], n: usize) -> V {
    (0..n)
        .map(|i| {
            StdULogic::from_code_pub(
                (0..4).fold(0, |c, k| c | ((b[i / 64][k] >> (i % 64) & 1) as u8) << k),
            )
        })
        .collect()
}
fn vec_of(max: usize) -> impl Strategy<Value = V> {
    prop::collection::vec(0usize..9, 0..max).prop_map(|v| v.into_iter().map(|i| ALL[i]).collect())
}
/// `v` repeated to `n` elements (`X` for an empty `v`).
fn cycle(v: &V, n: usize) -> V {
    (0..n)
        .map(|i| if v.is_empty() { X } else { v[i % v.len()] })
        .collect()
}
fn rl(v: &V) -> Logic<Planes> {
    Logic::from_fn(v.len(), |i| v[i])
}
fn res_ref(vs: &[&[StdULogic]]) -> V {
    (0..vs[0].len())
        .map(|i| match vs {
            [one] => one[i],
            _ => vs.iter().fold(Z, |r, v| r.resolve(v[i])),
        })
        .collect()
}

fn mk_len(v: &[bool], n: usize) -> BitVec {
    let mut x = BitVec::new(n);
    v.iter().take(n).enumerate().for_each(|(i, &t)| x.set(i, t));
    x
}

proptest! {
    #![proptest_config(ProptestConfig::with_cases(600))]

    #[test]
    fn resolve_into_matches_scalar(
        srcs in prop::collection::vec(vec_of(300), 0..6),
        doff in prop_oneof![Just(0usize), Just(64), 0usize..130],
        pad in prop_oneof![Just(0usize), 0usize..130],
        n in prop_oneof![0usize..250, 250usize..800],
    ) {
        // Every source and the destination are windows into longer runs.
        let srcs: Vec<V> = srcs.into_iter().map(|v| cycle(&v, n + pad + 70)).collect();
        let sb: Vec<Vec<Block>> = srcs.iter().map(blocks).collect();
        let offs: Vec<usize> = (0..srcs.len()).map(|i| (i * 37 + pad) % 70).collect();
        let args: Vec<(&[Block], usize)> =
            sb.iter().zip(&offs).map(|(b, &o)| (b.as_slice(), o)).collect();
        let mut dst = vec![[0x5555u64, 0, 0x3333, 0]; (doff + n).div_ceil(64) + 1];
        let before = unblock(&dst, dst.len() * 64);
        wide::resolve_into(&mut dst, doff, args.len(), |i| args[i], n);
        let after = unblock(&dst, dst.len() * 64);
        let win: Vec<&[StdULogic]> =
            srcs.iter().zip(&offs).map(|(v, &o)| &v[o..o + n]).collect();
        let want = if win.is_empty() { vec![Z; n] } else { res_ref(&win) };
        prop_assert_eq!(&after[doff..doff + n], &want[..]);
        prop_assert_eq!(&after[..doff], &before[..doff]);
        prop_assert_eq!(&after[doff + n..], &before[doff + n..]);
    }

    #[test]
    fn copy_eq_range(a in vec_of(300), b in vec_of(300), doff in prop_oneof![Just(0usize), 0usize..70], soff in prop_oneof![Just(0usize), 0usize..70], n in prop_oneof![0usize..200, 500usize..900]) {
        let (a, b) = (cycle(&a, doff + n + 1), cycle(&b, soff + n + 1));
        let (mut ab, bb) = (blocks(&a), blocks(&b));
        let same = a[doff..doff + n] == b[soff..soff + n];
        prop_assert_eq!(wide::eq_range(&ab, doff, &bb, soff, n), same);
        wide::copy_range(&mut ab, doff, &bb, soff, n);
        let mut want = a.clone();
        want[doff..doff + n].copy_from_slice(&b[soff..soff + n]);
        prop_assert_eq!(unblock(&ab, a.len()), want);
        prop_assert!(wide::eq_range(&ab, doff, &bb, soff, n));
    }

    #[test]
    fn has_x_and_match(a in vec_of(300), b in vec_of(300), same in any::<bool>()) {
        let n = a.len().min(b.len());
        let (mut a, mut b) = (a, b);
        a.truncate(n);
        b.truncate(n);
        if same {
            b = a.clone();
        }
        prop_assert_eq!(rl(&a).has_x(), a.iter().any(|x| x.is_x()));
        let want = a.iter().zip(&b).fold(One, |r, (&x, &y)| r.and(x.match_eq(y)));
        prop_assert_eq!(rl(&a).match_eq(&rl(&b)), want);
        prop_assert_eq!(rl(&a).match_ne(&rl(&b)), want.not());
    }

    #[test]
    fn slice_concat_shift(a in vec_of(300), b in vec_of(300), lo in 0usize..100, k in 0usize..350) {
        let ra = rl(&a);
        let n = a.len();
        let lo = lo.min(n);
        let m = (n - lo).min(k);
        prop_assert_eq!(ra.slice(lo, m).iter().collect::<V>(), a[lo..lo + m].to_vec());
        let cat: V = b.iter().chain(&a).copied().collect();
        prop_assert_eq!(ra.concat(&rl(&b)).iter().collect::<V>(), cat);
        if n > 0 {
            let sh = |f: &dyn Fn(usize) -> StdULogic| -> V { (0..n).map(f).collect() };
            prop_assert_eq!(ra.shl(k).iter().collect::<V>(), sh(&|i| if i >= k { a[i - k] } else { Zero }));
            prop_assert_eq!(ra.shr(k).iter().collect::<V>(), sh(&|i| if i + k < n { a[i + k] } else { Zero }));
            prop_assert_eq!(ra.sra(k).iter().collect::<V>(), sh(&|i| if i + k < n { a[i + k] } else { a[n - 1] }));
            prop_assert_eq!(ra.rol(k).iter().collect::<V>(), sh(&|i| a[(i + n - k % n) % n]));
            prop_assert_eq!(ra.ror(k).iter().collect::<V>(), sh(&|i| a[(i + k) % n]));
        }
    }

    #[test]
    fn bitvec_cmp_and_i64(
        a in prop::collection::vec(any::<bool>(), 1..200),
        b in prop::collection::vec(any::<bool>(), 1..200),
        signed in any::<bool>(),
    ) {
        let n = a.len().min(b.len());
        let mk = |v: &Vec<bool>| {
            let mut x = BitVec::new(n);
            v.iter().take(n).enumerate().for_each(|(i, &t)| x.set(i, t));
            x
        };
        let (x, y) = (mk(&a), mk(&b));
        let key = |v: &Vec<bool>| {
            let mut k: Vec<bool> = v.iter().take(n).copied().collect();
            if signed {
                k[n - 1] = !k[n - 1];
            }
            k.reverse();
            k
        };
        prop_assert_eq!(x.cmp_num(&y, signed), key(&a).cmp(&key(&b)));
        // Different lengths: widened first.
        let (p, q) = (mk_len(&a, a.len()), mk_len(&b, b.len()));
        let w = a.len().max(b.len());
        prop_assert_eq!(
            p.cmp_ext(&q, signed),
            p.extend(w, signed).cmp_num(&q.extend(w, signed), signed)
        );
        // to_i64 fits iff bits 63.. all equal the sign; the value is the low 64 bits sign-extended.
        let neg = a[n - 1];
        let fit = (63..n).all(|i| a[i] == neg);
        prop_assert_eq!(x.to_i64().is_some(), fit);
        if fit {
            let mut v: i128 = 0;
            for i in (0..n.min(64)).rev() {
                v = v << 1 | i128::from(a[i]);
            }
            if n < 64 && neg {
                v -= 1 << n;
            }
            if n >= 64 {
                v = i128::from(v as u64 as i64);
            }
            prop_assert_eq!(x.to_i64().unwrap() as i128, v);
        }
    }
}

#[test]
fn single_source_is_identity_and_pairs_agree() {
    let all: V = ALL.to_vec();
    let b = blocks(&all);
    let mut d = vec![[0; 4]];
    wide::resolve_into(&mut d, 0, 1, |_| (&b, 0), 9);
    assert_eq!(unblock(&d, 9), all);
    for x in ALL {
        for y in ALL {
            let r = Planes::resolve_block(blocks(&vec![x])[0], blocks(&vec![y])[0]);
            assert_eq!(unblock(&[r], 1)[0], x.resolve(y));
        }
    }
    assert_eq!(Planes::resolve_block(Z_BLOCK, Z_BLOCK)[2] & 1, 1);
    assert_eq!(Planes::new(0, U).len(), 0);
}
