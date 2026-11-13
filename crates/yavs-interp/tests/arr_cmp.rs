//! `yavs_interp::arr_cmp` (`Inst::ArrCmp`) against a per-element model: logic planes with all
//! nine codes and every first-difference position around block boundaries, lengths that put the
//! two operands' planes at different offsets, null arrays, and byte elements of each width with
//! negative values (LRM 9.2.3).

use yavs_interp::arr_cmp;
use yavs_ir::Width;

/// The model: the first difference from the left decides, else the shorter is less.
fn model(a: &[i64], b: &[i64]) -> i64 {
    for (x, y) in a.iter().zip(b) {
        if x != y {
            return if x < y { -1 } else { 1 };
        }
    }
    (a.len().cmp(&b.len())) as i64
}

/// Bit planes of `codes` (element 0 from the left is the highest plane element).
fn planes(codes: &[i64]) -> Vec<u8> {
    let n = codes.len();
    let mut w = vec![[0u64; 4]; n.div_ceil(64)];
    for (i, &c) in codes.iter().enumerate() {
        let e = n - 1 - i;
        for (p, word) in w[e / 64].iter_mut().enumerate() {
            *word |= ((c as u64 >> p) & 1) << (e % 64);
        }
    }
    // Garbage above the last element, which the scan must not read as elements.
    if !n.is_multiple_of(64) {
        let last = w.len() - 1;
        for word in &mut w[last] {
            *word |= !0u64 << (n % 64);
        }
    }
    w.iter().flatten().flat_map(|x| x.to_le_bytes()).collect()
}

fn bytes(v: &[i64], w: Width) -> Vec<u8> {
    let k = w.bytes() as usize;
    v.iter()
        .flat_map(|x| x.to_le_bytes()[..k].to_vec())
        .collect()
}

struct Lcg(u64);
impl Lcg {
    fn next(&mut self, n: u64) -> u64 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 ^ self.0 >> 29) % n
    }
}

const LENS: [usize; 10] = [0, 1, 2, 63, 64, 65, 127, 128, 129, 300];

#[test]
fn logic_planes_against_the_model() {
    let mut r = Lcg(7);
    for &la in &LENS {
        for &lb in &LENS {
            let a: Vec<i64> = (0..la).map(|_| r.next(9) as i64).collect();
            let m = la.min(lb);
            // `b` agrees with `a` up to position `p`, where it takes every code.
            let mut ps: Vec<usize> = vec![0, 1, 62, 63, 64, 65, 127, 128, m / 2];
            ps.extend(m.checked_sub(1));
            for &p in ps.iter().filter(|&&p| p < lb) {
                for code in 0..9 {
                    let mut b: Vec<i64> = (0..lb).map(|_| r.next(9) as i64).collect();
                    let k = p.min(m);
                    b[..k].copy_from_slice(&a[..k]);
                    b[p] = code;
                    let got = arr_cmp(&planes(&a), la, &planes(&b), lb, None);
                    assert_eq!(
                        got,
                        model(&a, &b),
                        "logic la={la} lb={lb} p={p} code={code}"
                    );
                    let back = arr_cmp(&planes(&b), lb, &planes(&a), la, None);
                    assert_eq!(back, model(&b, &a), "logic lb={lb} la={la} p={p}");
                }
            }
            // A prefix and the equal array.
            let pre = &a[..m];
            assert_eq!(
                arr_cmp(&planes(pre), m, &planes(&a), la, None),
                model(pre, &a)
            );
            assert_eq!(arr_cmp(&planes(&a), la, &planes(&a), la, None), 0);
        }
    }
}

#[test]
fn byte_elements_against_the_model() {
    let mut r = Lcg(11);
    for (w, lo, span) in [
        (Width::B1, 0i64, 256u64),
        (Width::B4, -(1 << 31), 1 << 32),
        (Width::B8, i64::MIN, u64::MAX),
    ] {
        for &la in &LENS {
            for &lb in &LENS {
                let a: Vec<i64> = (0..la)
                    .map(|_| lo.wrapping_add(r.next(span) as i64))
                    .collect();
                let m = la.min(lb);
                let mut ps: Vec<usize> = vec![0, 1, 31, 32, 33, 255, 256, m / 2];
                ps.extend(m.checked_sub(1));
                for &p in ps.iter().filter(|&&p| p < lb) {
                    for v in [lo, lo.wrapping_add((span - 1) as i64), 0, -1, 1] {
                        if w == Width::B1 && v < 0 {
                            continue;
                        }
                        let mut b: Vec<i64> = (0..lb)
                            .map(|_| lo.wrapping_add(r.next(span) as i64))
                            .collect();
                        let k = p.min(m);
                        b[..k].copy_from_slice(&a[..k]);
                        b[p] = v;
                        let got = arr_cmp(&bytes(&a, w), la, &bytes(&b, w), lb, Some(w));
                        assert_eq!(got, model(&a, &b), "{w:?} la={la} lb={lb} p={p} v={v}");
                    }
                }
                assert_eq!(arr_cmp(&bytes(&a, w), la, &bytes(&a, w), la, Some(w)), 0);
            }
        }
    }
}

#[test]
fn null_arrays() {
    assert_eq!(arr_cmp(&[], 0, &[], 0, None), 0);
    assert_eq!(arr_cmp(&[], 0, &[], 0, Some(Width::B1)), 0);
    assert_eq!(arr_cmp(&[], 0, &planes(&[2]), 1, None), -1);
    assert_eq!(
        arr_cmp(&bytes(&[-5], Width::B4), 1, &[], 0, Some(Width::B4)),
        1
    );
}
