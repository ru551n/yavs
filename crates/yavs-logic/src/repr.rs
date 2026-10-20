//! The storage layout contract. Everything else on `Logic<R>` is built on these primitives.
//! Element 0 is the rightmost (least significant) element.

use crate::scalar::StdULogic::{self, *};
use core::fmt::Debug;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum BinOp {
    And,
    Or,
    Xor,
    /// Two-driver resolution.
    Resolve,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum UnOp {
    Not,
    ToX01,
    ToX01Z,
    ToUx01,
}

#[allow(clippy::len_without_is_empty)]
pub trait Repr: Clone + PartialEq + Eq + Debug {
    fn new(len: usize, v: StdULogic) -> Self;
    fn len(&self) -> usize;
    fn get(&self, i: usize) -> StdULogic;
    fn set(&mut self, i: usize, v: StdULogic);
    /// `self = self op o`; lengths are equal.
    fn bin(&mut self, o: &Self, op: BinOp);
    fn un(&mut self, op: UnOp);
    /// Elements `lo .. lo + n`.
    fn slice(&self, lo: usize, n: usize) -> Self {
        assert!(lo + n <= self.len());
        let mut r = Self::new(n, U);
        (0..n).for_each(|i| r.set(i, self.get(lo + i)));
        r
    }
    /// `self & low` in VHDL terms: `self` becomes the high part.
    fn concat(&self, low: &Self) -> Self {
        let l = low.len();
        let mut r = Self::new(l + self.len(), U);
        (0..l).for_each(|i| r.set(i, low.get(i)));
        (0..self.len()).for_each(|i| r.set(l + i, self.get(i)));
        r
    }
    /// Any element not `0 1 L H`.
    fn has_x(&self) -> bool {
        (0..self.len()).any(|i| self.get(i).is_x())
    }
    /// Vector `?=` (equal lengths): the AND fold of the element matches.
    fn match_eq(&self, o: &Self) -> StdULogic {
        (0..self.len()).fold(One, |r, i| r.and(self.get(i).match_eq(o.get(i))))
    }
    /// Resolve N >= 1 drivers of equal length (one driver is passed through unchanged).
    fn resolve_all(drivers: &[&Self]) -> Self {
        let (first, rest) = drivers.split_first().expect("at least one driver");
        let mut r = (*first).clone();
        rest.iter().for_each(|d| r.bin(d, BinOp::Resolve));
        r
    }
    /// Bit words (element 0 = bit 0 of word 0) if every element is `0 1 L H`, else `None`.
    fn to_words01(&self) -> Option<Vec<u64>>;
    fn from_words01(len: usize, w: &[u64]) -> Self;
}
