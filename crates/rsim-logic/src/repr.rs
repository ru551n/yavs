//! The storage layout contract. Everything else on `Logic<R>` is built on these primitives.
//! Element 0 is the rightmost (least significant) element.

use crate::scalar::StdULogic;
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
    /// Bit words (element 0 = bit 0 of word 0) if every element is `0 1 L H`, else `None`.
    fn to_words01(&self) -> Option<Vec<u64>>;
    fn from_words01(len: usize, w: &[u64]) -> Self;
}
