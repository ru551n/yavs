//! Word-parallel kernels over runs of plane blocks (`[u64; 4]`, 64 elements each): bit-range
//! loads and stores, copies, comparisons and the many-source IEEE 1164 resolution. Everything
//! works on a block slice and an element offset, so a signal holding several runs can use it
//! without building a `Planes`. Whole-block (offset multiple of 64) runs take straight loops
//! the compiler vectorises; other offsets shift each plane word.
//!
//! These are the reference-equal fast paths: `tests/wide.rs` checks each against a per-element
//! scalar model for random planes and odd lengths.

use crate::planes::Block;
use std::ops::{BitAnd, BitOr, BitXor, Not};

/// The low `n` bits (`n <= 64`).
#[inline(always)]
pub fn nmask(n: usize) -> u64 {
    if n >= 64 { !0 } else { (1u64 << n) - 1 }
}

/// Every element `Z` (code 4: plane 2 only).
pub const Z_BLOCK: Block = [0, 0, !0, 0];

/// Elements `off .. off + n` (`n <= 64`) as one block, element `off` in bit 0; bits `>= n` are 0.
#[inline(always)]
pub fn get_block(b: &[Block], off: usize, n: usize) -> Block {
    let (w, s) = (off / 64, off % 64);
    let m = nmask(n);
    if s == 0 {
        return b[w].map(|p| p & m);
    }
    let hi = b.get(w + 1);
    std::array::from_fn(|k| {
        let mut v = b[w][k] >> s;
        if let Some(h) = hi {
            v |= h[k] << (64 - s);
        }
        v & m
    })
}

/// Write the elements of `v` selected by `sel` (low `n` bits) at `off`.
#[inline(always)]
pub fn put_block(b: &mut [Block], off: usize, n: usize, v: Block, sel: u64) {
    let m = sel & nmask(n);
    let (w, s) = (off / 64, off % 64);
    if s == 0 {
        for k in 0..4 {
            b[w][k] = (b[w][k] & !m) | (v[k] & m);
        }
        return;
    }
    for k in 0..4 {
        b[w][k] = (b[w][k] & !(m << s)) | ((v[k] & m) << s);
        if s + n > 64 {
            let m2 = m >> (64 - s);
            b[w + 1][k] = (b[w + 1][k] & !m2) | ((v[k] >> (64 - s)) & m2);
        }
    }
}

/// Chunks of at most 64 elements covering `0..n`: `(start, len)`.
#[inline(always)]
pub fn chunks(n: usize) -> impl Iterator<Item = (usize, usize)> {
    (0..n).step_by(64).map(move |c| (c, (n - c).min(64)))
}

/// `out` = the full blocks of the run starting at element `off` of `src` (`out.len()` blocks of
/// 64 elements). Bits past the end of `src` read as 0.
#[inline(always)]
fn load_blocks(out: &mut [Block], src: &[Block], off: usize) {
    let (w, s) = (off / 64, off % 64);
    let m = out.len();
    if s == 0 {
        out.copy_from_slice(&src[w..w + m]);
        return;
    }
    // Blocks with a successor in `src` are plain shift pairs, which vectorise.
    let safe = m.min(src.len().saturating_sub(w + 1));
    for (o, p) in out.iter_mut().zip(src[w..].windows(2)).take(safe) {
        *o = std::array::from_fn(|k| p[0][k] >> s | p[1][k] << (64 - s));
    }
    for (j, o) in out.iter_mut().enumerate().skip(safe) {
        *o = get_block(src, off + 64 * j, 64);
    }
}

/// The elements before the next block boundary of the destination (at most `n`).
#[inline(always)]
fn head(doff: usize, n: usize) -> usize {
    ((64 - doff % 64) % 64).min(n)
}

/// Copy `n` elements of `src` from `soff` to `dst` at `doff` (distinct slices).
pub fn copy_range(dst: &mut [Block], doff: usize, src: &[Block], soff: usize, n: usize) {
    // A partial first block, then whole destination blocks, then a partial last block.
    let h = head(doff, n);
    if h != 0 {
        put_block(dst, doff, h, get_block(src, soff, h), !0);
    }
    let (doff, soff, n) = (doff + h, soff + h, n - h);
    let full = n / 64;
    let d0 = doff / 64;
    load_blocks(&mut dst[d0..d0 + full], src, soff);
    let c = full * 64;
    if n != c {
        put_block(dst, doff + c, n - c, get_block(src, soff + c, n - c), !0);
    }
}

/// Whether elements `aoff .. aoff + n` of `a` equal `boff ..` of `b`.
pub fn eq_range(a: &[Block], aoff: usize, b: &[Block], boff: usize, n: usize) -> bool {
    const G: usize = 8;
    let full = n / 64;
    if aoff.is_multiple_of(64) && boff.is_multiple_of(64) {
        let (x, y) = (aoff / 64, boff / 64);
        if a[x..x + full] != b[y..y + full] {
            return false;
        }
    } else {
        // No early exit inside a group: branch-free beats a mispredicted exit on wide runs.
        let (mut la, mut lb) = ([[0u64; 4]; G], [[0u64; 4]; G]);
        let mut j = 0;
        while j + G <= full {
            load_blocks(&mut la, a, aoff + 64 * j);
            load_blocks(&mut lb, b, boff + 64 * j);
            let d = (0..G).fold(0u64, |d, g| {
                (0..4).fold(d, |d, k| d | (la[g][k] ^ lb[g][k]))
            });
            if d != 0 {
                return false;
            }
            j += G;
        }
        for c in (j * 64..full * 64).step_by(64) {
            if get_block(a, aoff + c, 64) != get_block(b, boff + c, 64) {
                return false;
            }
        }
    }
    let c = full * 64;
    n == c || get_block(a, aoff + c, n - c) == get_block(b, boff + c, n - c)
}

/// A word of 64 elements, or four of them side by side (`L4`): the plane arithmetic is written
/// once over this trait, so one block or four blocks per step (lane-wise, which compilers
/// vectorise) share the same code.
pub(crate) trait Lane:
    Copy + BitAnd<Output = Self> + BitOr<Output = Self> + BitXor<Output = Self> + Not<Output = Self>
{
    const ZERO: Self;
}
impl Lane for u64 {
    const ZERO: u64 = 0;
}

/// The same plane word of four blocks.
#[derive(Clone, Copy)]
pub(crate) struct L4([u64; 4]);
macro_rules! lanewise {
    ($($tr:ident $f:ident $op:tt),*) => {$(
        impl $tr for L4 {
            type Output = L4;
            #[inline(always)]
            fn $f(self, o: L4) -> L4 {
                L4(std::array::from_fn(|i| self.0[i] $op o.0[i]))
            }
        }
    )*};
}
lanewise!(BitAnd bitand &, BitOr bitor |, BitXor bitxor ^);
impl Not for L4 {
    type Output = L4;
    #[inline(always)]
    fn not(self) -> L4 {
        L4(self.0.map(|x| !x))
    }
}
impl Lane for L4 {
    const ZERO: L4 = L4([0; 4]);
}

/// Four blocks as planes of four lanes.
#[inline(always)]
pub(crate) fn gather(p: &[Block]) -> [L4; 4] {
    std::array::from_fn(|k| L4(std::array::from_fn(|l| p[l][k])))
}

/// Inverse of `gather`.
#[inline(always)]
pub(crate) fn scatter(x: [L4; 4], out: &mut [Block]) {
    for (l, o) in out.iter_mut().enumerate().take(4) {
        *o = std::array::from_fn(|k| x[k].0[l]);
    }
}

/// Accumulated classes of the sources of a block under IEEE 1164 resolution. Resolution is
/// associative and commutative, so the sources are folded into class masks and resolved once.
#[derive(Clone, Copy)]
pub(crate) struct Classes<T> {
    u: T,
    x: T,
    f0: T,
    f1: T,
    wl: T,
    wh: T,
    ww: T,
}

impl<T: Lane> Classes<T> {
    #[inline(always)]
    pub(crate) fn new() -> Self {
        let z = T::ZERO;
        Self {
            u: z,
            x: z,
            f0: z,
            f1: z,
            wl: z,
            wh: z,
            ww: z,
        }
    }
    #[inline(always)]
    pub(crate) fn add(&mut self, [a, b, c, d]: [T; 4]) {
        let nd = !d;
        let f = nd & !c & b; // forced 0/1
        let k = nd & c & !b; // Z or W
        let wk = nd & c & b; // L or H
        self.u = self.u | !(a | b | c | d);
        self.x = self.x | (a & !b & !c & nd) | d;
        self.f0 = self.f0 | (f & !a);
        self.f1 = self.f1 | (f & a);
        self.wl = self.wl | (wk & !a);
        self.wh = self.wh | (wk & a);
        self.ww = self.ww | (k & a);
    }
    #[inline(always)]
    pub(crate) fn finish(self) -> [T; 4] {
        let Self {
            u,
            x,
            f0,
            f1,
            wl,
            wh,
            ww,
        } = self;
        let xm = !u & (x | (f0 & f1));
        let rest = !u & !xm;
        let (m0, m1) = (rest & f0, rest & f1);
        let weak = rest & !m0 & !m1;
        let mw = weak & (ww | (wl & wh));
        let ml = weak & !mw & wl;
        let mh = weak & !mw & wh;
        let mz = weak & !mw & !ml & !mh;
        [
            xm | m1 | mw | mh,
            m0 | m1 | ml | mh,
            mz | mw | ml | mh,
            T::ZERO,
        ]
    }
}

/// Resolve `nsrc` sources into `n` elements of `dst` at `doff`; `src(i)` is source `i` as a
/// block slice and an element offset (a closure, so the caller builds no list of sources).
/// No source gives `Z`; one source is copied unchanged (the identity, not the resolution of one
/// driver, which would map `-` to `X`).
pub fn resolve_into<'a>(
    dst: &mut [Block],
    doff: usize,
    nsrc: usize,
    src: impl Fn(usize) -> (&'a [Block], usize),
    n: usize,
) {
    match nsrc {
        0 => {
            for (c, k) in chunks(n) {
                put_block(dst, doff + c, k, Z_BLOCK, !0);
            }
        }
        1 => {
            let (s, so) = src(0);
            copy_range(dst, doff, s, so, n)
        }
        _ => {
            // The partial first block, then whole blocks four at a time, planes apart and one
            // lane per block, so the class arithmetic is lane-wise (compilers vectorise it; the
            // plane transpose costs more than a wider vector gains, so there is no per-CPU
            // variant), then the rest block by block.
            let h = if n >= 320 { head(doff, n) } else { 0 };
            let mut done = 0;
            if h != 0 {
                let mut acc = Classes::<u64>::new();
                for i in 0..nsrc {
                    let (s, so) = src(i);
                    acc.add(get_block(s, so, h));
                }
                put_block(dst, doff, h, acc.finish(), !0);
                done = h;
            }
            let mut g = [[0u64; 4]; 4];
            while (doff + done).is_multiple_of(64) && done + 256 <= n {
                let mut acc = Classes::<L4>::new();
                for i in 0..nsrc {
                    let (s, so) = src(i);
                    load_blocks(&mut g, s, so + done);
                    acc.add(gather(&g));
                }
                let d = (doff + done) / 64;
                scatter(acc.finish(), &mut dst[d..d + 4]);
                done += 256;
            }
            for (c, k) in chunks(n - done).map(|(c0, k)| (done + c0, k)) {
                let mut acc = Classes::<u64>::new();
                for i in 0..nsrc {
                    let (s, so) = src(i);
                    acc.add(get_block(s, so + c, k));
                }
                put_block(dst, doff + c, k, acc.finish(), !0);
            }
        }
    }
}

/// Whether any of the `n` elements is not `0 1 L H` (`is_x`): those are exactly the codes
/// with plane 1 set and plane 3 clear.
pub fn has_x(b: &[Block], n: usize) -> bool {
    let mut any = 0u64;
    for (i, [_, p1, _, p3]) in b.iter().enumerate().take(n.div_ceil(64)) {
        any |= (!p1 | p3) & nmask(n - i * 64);
    }
    any != 0
}

/// The vector `?=` of two equal-length runs as a `std_ulogic` code (`0`: U, `1`: X, `2`: '0',
/// `3`: '1'): the AND fold of the element matches, so a definite mismatch beats `U`, which beats
/// `X`. A `-` element matches anything.
pub fn match_eq(a: &[Block], aoff: usize, b: &[Block], boff: usize, n: usize) -> u8 {
    let (mut zero, mut un, mut xx) = (0u64, 0u64, 0u64);
    for (c, k) in chunks(n) {
        let (x, y) = (get_block(a, aoff + c, k), get_block(b, boff + c, k));
        let m = nmask(k);
        let live = !(x[3] | y[3]) & m; // not '-'
        let (ux, uy) = (!(x[0] | x[1] | x[2] | x[3]), !(y[0] | y[1] | y[2] | y[3]));
        let (dx, dy) = (x[1] & !x[3], y[1] & !y[3]); // 0 1 L H
        un |= live & (ux | uy);
        let known = live & !ux & !uy;
        xx |= known & !(dx & dy);
        zero |= known & dx & dy & (x[0] ^ y[0]);
    }
    if zero != 0 {
        2
    } else if un != 0 {
        0
    } else if xx != 0 {
        1
    } else {
        3
    }
}
