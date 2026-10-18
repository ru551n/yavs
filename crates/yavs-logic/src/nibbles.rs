//! Layout (a): one 4-bit code per element, two per byte (even element in the low nibble).

use crate::repr::{BinOp, Repr, UnOp};
use crate::scalar::{self, StdULogic, Tab};

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Nibbles {
    len: usize,
    b: Vec<u8>,
}

/// code -> 0/1, or 255 for non-`01LH`.
const BIT: [u8; 16] = [255, 255, 0, 1, 255, 255, 0, 1, 255, 0, 0, 0, 0, 0, 0, 0];

impl Nibbles {
    /// Keep the unused high nibble of an odd-length vector zero, so `==` is derived.
    fn norm(&mut self) {
        if self.len & 1 == 1
            && let Some(l) = self.b.last_mut()
        {
            *l &= 0x0F;
        }
    }
}

fn tab(op: BinOp) -> &'static Tab {
    match op {
        BinOp::And => &scalar::AND,
        BinOp::Or => &scalar::OR,
        BinOp::Xor => &scalar::XOR,
        BinOp::Resolve => &scalar::RESOLVE,
    }
}

impl Repr for Nibbles {
    fn new(len: usize, v: StdULogic) -> Self {
        let c = v as u8;
        let mut s = Self {
            len,
            b: vec![c | c << 4; len.div_ceil(2)],
        };
        s.norm();
        s
    }
    fn len(&self) -> usize {
        self.len
    }
    #[inline]
    fn get(&self, i: usize) -> StdULogic {
        StdULogic::from_code((self.b[i >> 1] >> ((i & 1) * 4)) & 15)
    }
    #[inline]
    fn set(&mut self, i: usize, v: StdULogic) {
        let sh = (i & 1) * 4;
        let b = &mut self.b[i >> 1];
        *b = *b & !(15 << sh) | (v as u8) << sh;
    }
    fn bin(&mut self, o: &Self, op: BinOp) {
        let t = tab(op);
        for (x, &y) in self.b.iter_mut().zip(&o.b) {
            *x = t[(*x & 15) as usize][(y & 15) as usize]
                | t[(*x >> 4) as usize][(y >> 4) as usize] << 4;
        }
    }
    fn un(&mut self, op: UnOp) {
        let t = match op {
            UnOp::Not => &scalar::NOT,
            UnOp::ToX01 => &scalar::TO_X01,
            UnOp::ToX01Z => &scalar::TO_X01Z,
            UnOp::ToUx01 => &scalar::TO_UX01,
        };
        for x in &mut self.b {
            *x = t[(*x & 15) as usize] | t[(*x >> 4) as usize] << 4;
        }
        self.norm();
    }
    fn to_words01(&self) -> Option<Vec<u64>> {
        let mut w = vec![0u64; self.len.div_ceil(64)];
        for i in 0..self.len {
            let v = BIT[((self.b[i >> 1] >> ((i & 1) * 4)) & 15) as usize];
            if v == 255 {
                return None;
            }
            w[i >> 6] |= (v as u64) << (i & 63);
        }
        Some(w)
    }
    fn from_words01(len: usize, w: &[u64]) -> Self {
        let mut s = Self::new(len, StdULogic::Zero);
        for i in 0..len {
            if w[i >> 6] >> (i & 63) & 1 == 1 {
                s.set(i, StdULogic::One);
            }
        }
        s
    }
}
