//! Layout (b): four bit-planes. Block `i` holds elements `64i..64i+64`; `[u64; 4]` are plane 0
//! (code bit 0) to plane 3. All operations are bit-parallel over 64 elements.

use crate::repr::{BinOp, Repr, UnOp};
use crate::scalar::StdULogic;

type B = [u64; 4];

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Planes {
    len: usize,
    b: Vec<B>,
}

// Class masks of a block (code: U0 X1 0=2 1=3 Z4 W5 L6 H7 -8).
#[inline]
fn is_u([a, b, c, d]: B) -> u64 {
    !(a | b | c | d)
}
/// `0` or `L`.
#[inline]
fn is0([a, b, _, d]: B) -> u64 {
    !d & b & !a
}
/// `1` or `H`.
#[inline]
fn is1([a, b, _, d]: B) -> u64 {
    !d & b & a
}

fn and(x: B, y: B) -> B {
    let z = is0(x) | is0(y);
    let u = is_u(x) | is_u(y);
    let one = is1(x) & is1(y);
    [!z & !u, z | one, 0, 0]
}
fn or(x: B, y: B) -> B {
    let o = is1(x) | is1(y);
    let u = is_u(x) | is_u(y);
    let zz = is0(x) & is0(y);
    [o | (!u & !zz), o | (!u & zz), 0, 0]
}
fn xor(x: B, y: B) -> B {
    let u = is_u(x) | is_u(y);
    let k = (is0(x) | is1(x)) & (is0(y) | is1(y));
    let v = is1(x) ^ is1(y);
    [!u & (!k | v), !u & k, 0, 0]
}
fn resolve(x: B, y: B) -> B {
    let cls = |[a, b, c, d]: B| {
        let nd = !d;
        let f = nd & !c & b; // forced 0/1
        let k = nd & c & !b; // Z or W
        let wk = nd & c & b; // L or H
        (
            !(a | b | c | d),       // U
            (a & !b & !c & nd) | d, // X or -
            f & !a,                 // 0
            f & a,                  // 1
            wk & !a,                // L
            wk & a,                 // H
            k & a,                  // W
        )
    };
    let (ux, bx, f0x, f1x, lx, hx, wx) = cls(x);
    let (uy, by, f0y, f1y, ly, hy, wy) = cls(y);
    let u = ux | uy;
    let xm = !u & (bx | by | (f0x & f1y) | (f1x & f0y));
    let rest = !u & !xm;
    let m0 = rest & (f0x | f0y);
    let m1 = rest & (f1x | f1y);
    let weak = rest & !m0 & !m1;
    let mw = weak & (wx | wy | (lx & hy) | (hx & ly));
    let ml = weak & !mw & (lx | ly);
    let mh = weak & !mw & (hx | hy);
    let mz = weak & !mw & !ml & !mh;
    [xm | m1 | mw | mh, m0 | m1 | ml | mh, mz | mw | ml | mh, 0]
}
fn not(x: B) -> B {
    [!is_u(x) & !is1(x), is0(x) | is1(x), 0, 0]
}
fn to_x01(x: B) -> B {
    [!is0(x), is0(x) | is1(x), 0, 0]
}
fn to_x01z([a, b, c, d]: B) -> B {
    let z = !d & c & !b & !a;
    let x = [a, b, c, d];
    [!is0(x) & !z, is0(x) | is1(x), z, 0]
}
fn to_ux01(x: B) -> B {
    [!is0(x) & !is_u(x), is0(x) | is1(x), 0, 0]
}

/// One block: elements `64i..64i+64`, planes 0 to 3 (code bits 0 to 3).
pub type Block = [u64; 4];

impl Planes {
    /// The raw blocks, element 0 in bit 0 of block 0. Bits beyond `len` are 0.
    pub fn blocks(&self) -> &[Block] {
        &self.b
    }

    /// Build from raw blocks; `blocks.len()` must be `len.div_ceil(64)`. Bits beyond `len`
    /// are cleared.
    pub fn from_blocks(len: usize, blocks: Vec<Block>) -> Self {
        assert_eq!(
            blocks.len(),
            len.div_ceil(64),
            "block count does not match length"
        );
        let mut s = Self { len, b: blocks };
        s.norm();
        s
    }

    /// IEEE 1164 resolution of two blocks, element by element.
    pub fn resolve_block(x: Block, y: Block) -> Block {
        resolve(x, y)
    }

    fn mask(&self, bi: usize) -> u64 {
        let r = self.len - bi * 64;
        if r >= 64 { !0 } else { (1u64 << r) - 1 }
    }
    /// Clear the planes beyond `len` (code 0 = `U`) so `==` is derived.
    fn norm(&mut self) {
        if let Some(l) = self.b.len().checked_sub(1) {
            let m = self.mask(l);
            self.b[l].iter_mut().for_each(|p| *p &= m);
        }
    }
    #[inline(always)]
    fn zip(&mut self, o: &Self, f: impl Fn(B, B) -> B) {
        for (x, &y) in self.b.iter_mut().zip(&o.b) {
            *x = f(*x, y);
        }
    }
    #[inline(always)]
    fn map(&mut self, f: impl Fn(B) -> B) {
        for x in &mut self.b {
            *x = f(*x);
        }
        self.norm();
    }
}

impl Repr for Planes {
    fn new(len: usize, v: StdULogic) -> Self {
        let c = v as u8;
        let blk = [0, 1, 2, 3].map(|k| if c >> k & 1 == 1 { !0 } else { 0 });
        let mut s = Self {
            len,
            b: vec![blk; len.div_ceil(64)],
        };
        s.norm();
        s
    }
    fn len(&self) -> usize {
        self.len
    }
    #[inline]
    fn get(&self, i: usize) -> StdULogic {
        let b = &self.b[i >> 6];
        let s = i & 63;
        StdULogic::from_code((0..4).fold(0, |c, k| c | ((b[k] >> s & 1) as u8) << k))
    }
    #[inline]
    fn set(&mut self, i: usize, v: StdULogic) {
        let b = &mut self.b[i >> 6];
        let s = i & 63;
        for (k, p) in b.iter_mut().enumerate() {
            *p = *p & !(1 << s) | ((v as u64 >> k) & 1) << s;
        }
    }
    fn bin(&mut self, o: &Self, op: BinOp) {
        match op {
            BinOp::And => self.zip(o, and),
            BinOp::Or => self.zip(o, or),
            BinOp::Xor => self.zip(o, xor),
            BinOp::Resolve => self.zip(o, resolve),
        }
    }
    fn un(&mut self, op: UnOp) {
        match op {
            UnOp::Not => self.map(not),
            UnOp::ToX01 => self.map(to_x01),
            UnOp::ToX01Z => self.map(to_x01z),
            UnOp::ToUx01 => self.map(to_ux01),
        }
    }
    fn to_words01(&self) -> Option<Vec<u64>> {
        let mut w = Vec::with_capacity(self.b.len());
        for (i, &x) in self.b.iter().enumerate() {
            if is0(x) | is1(x) != self.mask(i) {
                return None;
            }
            w.push(is1(x));
        }
        Some(w)
    }
    fn from_words01(len: usize, w: &[u64]) -> Self {
        let mut s = Self {
            len,
            b: w.iter().map(|&v| [v, !0, 0, 0]).collect(),
        };
        s.norm();
        s
    }
}
