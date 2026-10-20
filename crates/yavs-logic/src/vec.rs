//! `Logic<R>`: a packed `std_ulogic_vector` over any layout `R`. Element 0 is the rightmost
//! (least significant) element; strings and `concat` read left to right, high element first.

use crate::bitvec::BitVec;
use crate::repr::{BinOp, Repr, UnOp};
use crate::scalar::StdULogic::{self, *};
use core::cmp::Ordering;
use core::fmt;
use core::str::FromStr;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct Logic<R>(R);

impl<R: Repr> Logic<R> {
    /// Wrap a layout value.
    pub fn from_repr(r: R) -> Self {
        Self(r)
    }
    /// The layout value.
    pub fn repr(&self) -> &R {
        &self.0
    }
    pub fn new(len: usize, v: StdULogic) -> Self {
        Self(R::new(len, v))
    }
    /// Element `i` (0 = rightmost) is `f(i)`.
    pub fn from_fn(len: usize, f: impl Fn(usize) -> StdULogic) -> Self {
        let mut r = R::new(len, U);
        (0..len).for_each(|i| r.set(i, f(i)));
        Self(r)
    }
    pub fn len(&self) -> usize {
        self.0.len()
    }
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
    pub fn get(&self, i: usize) -> StdULogic {
        self.0.get(i)
    }
    pub fn set(&mut self, i: usize, v: StdULogic) {
        self.0.set(i, v)
    }
    pub fn iter(&self) -> impl Iterator<Item = StdULogic> + '_ {
        (0..self.len()).map(|i| self.0.get(i))
    }

    /// Elements `lo..lo + n`.
    pub fn slice(&self, lo: usize, n: usize) -> Self {
        Self(self.0.slice(lo, n))
    }
    /// `self & low` in VHDL terms: `self` becomes the high part.
    pub fn concat(&self, low: &Self) -> Self {
        Self(self.0.concat(&low.0))
    }

    fn binop(&self, o: &Self, op: BinOp) -> Self {
        assert_eq!(self.len(), o.len(), "length mismatch");
        let mut r = self.0.clone();
        r.bin(&o.0, op);
        Self(r)
    }
    fn unop(&self, op: UnOp) -> Self {
        let mut r = self.0.clone();
        r.un(op);
        Self(r)
    }
    pub fn and(&self, o: &Self) -> Self {
        self.binop(o, BinOp::And)
    }
    pub fn or(&self, o: &Self) -> Self {
        self.binop(o, BinOp::Or)
    }
    pub fn xor(&self, o: &Self) -> Self {
        self.binop(o, BinOp::Xor)
    }
    pub fn nand(&self, o: &Self) -> Self {
        self.and(o).not()
    }
    pub fn nor(&self, o: &Self) -> Self {
        self.or(o).not()
    }
    pub fn xnor(&self, o: &Self) -> Self {
        self.xor(o).not()
    }
    #[allow(clippy::should_implement_trait)]
    pub fn not(&self) -> Self {
        self.unop(UnOp::Not)
    }
    pub fn to_x01(&self) -> Self {
        self.unop(UnOp::ToX01)
    }
    pub fn to_x01z(&self) -> Self {
        self.unop(UnOp::ToX01Z)
    }
    pub fn to_ux01(&self) -> Self {
        self.unop(UnOp::ToUx01)
    }
    /// Resolve N >= 1 drivers of equal length (one driver is passed through unchanged).
    pub fn resolve(drivers: &[&Self]) -> Self {
        let first = drivers.first().expect("at least one driver");
        drivers
            .iter()
            .for_each(|d| assert_eq!(d.len(), first.len(), "length mismatch"));
        Self(match drivers {
            [a] => a.0.clone(),
            [a, b] => R::resolve_all(&[&a.0, &b.0]),
            _ => R::resolve_all(&drivers.iter().map(|d| &d.0).collect::<Vec<_>>()),
        })
    }
    /// `is_x` on a vector: any element not `0 1 L H`.
    pub fn has_x(&self) -> bool {
        self.0.has_x()
    }
    /// Vector `?=`: `1` if all elements match, `0` on a definite mismatch, `X`/`U` as in the LRM.
    pub fn match_eq(&self, o: &Self) -> StdULogic {
        assert_eq!(self.len(), o.len(), "length mismatch");
        // LRM 9.2.3: the AND of the element matches, so a later '0' wins over U and X.
        self.0.match_eq(&o.0)
    }
    pub fn match_ne(&self, o: &Self) -> StdULogic {
        self.match_eq(o).not()
    }

    /// Shift towards the high end, filling with `0` (`shift_left`, `sll`).
    pub fn shl(&self, n: usize) -> Self {
        self.shifted(n, Zero, true)
    }
    /// Shift towards the low end, filling with `0` (`shift_right` on unsigned, `srl`).
    pub fn shr(&self, n: usize) -> Self {
        self.shifted(n, Zero, false)
    }
    /// Shift towards the low end replicating the leftmost element (`shift_right` on signed).
    pub fn sra(&self, n: usize) -> Self {
        self.shifted(n, self.get(self.len() - 1), false)
    }
    /// `n` fill elements enter at the high (`up`) or low end; the rest move.
    fn shifted(&self, n: usize, fill: StdULogic, up: bool) -> Self {
        let l = self.len();
        let k = n.min(l);
        let fill = Self::new(k, fill);
        if up {
            self.slice(0, l - k).concat(&fill)
        } else {
            fill.concat(&self.slice(k, l - k))
        }
    }
    pub fn rol(&self, n: usize) -> Self {
        let l = self.len();
        let n = n % l;
        self.slice(0, l - n).concat(&self.slice(l - n, n))
    }
    pub fn ror(&self, n: usize) -> Self {
        let l = self.len();
        let n = n % l;
        self.slice(0, n).concat(&self.slice(n, l - n))
    }

    /// The vector as a pure 0/1 vector (`L`/`H` map to `0`/`1`), or `None` if it has a metavalue.
    pub fn to_bitvec(&self) -> Option<BitVec> {
        self.0
            .to_words01()
            .map(|w| BitVec::from_words(self.len(), w))
    }
    pub fn from_bitvec(b: &BitVec) -> Self {
        Self(R::from_words01(b.len(), b.words()))
    }

    /// numeric_std `+`/`-` on unsigned (or signed): result length is the longer operand; any
    /// metavalue makes every result element `X`; an empty operand gives an empty result.
    pub fn add_num(&self, o: &Self, signed: bool, sub: bool) -> Self {
        let size = self.len().max(o.len());
        if self.is_empty() || o.is_empty() {
            return Self::new(0, U);
        }
        match (self.to_bitvec(), o.to_bitvec()) {
            (Some(a), Some(b)) => {
                let (a, b) = (a.extend(size, signed), b.extend(size, signed));
                Self::from_bitvec(&if sub { a.sub(&b) } else { a.add(&b, false) })
            }
            _ => Self::new(size, X),
        }
    }
    /// numeric_std relational operators: `None` for an empty operand or a metavalue (every
    /// relational operator but `/=` is then false, `/=` true); else the numeric order.
    pub fn cmp_num(&self, o: &Self, signed: bool) -> Option<Ordering> {
        if self.is_empty() || o.is_empty() {
            return None;
        }
        let (a, b) = (self.to_bitvec()?, o.to_bitvec()?);
        Some(a.cmp_ext(&b, signed))
    }
    pub fn from_u64(len: usize, v: u64) -> Self {
        Self::from_bitvec(&BitVec::from_u64(len, v))
    }
    pub fn from_i64(len: usize, v: i64) -> Self {
        Self::from_bitvec(&BitVec::from_i64(len, v))
    }
    /// `to_integer` on unsigned; `None` on a metavalue or overflow (numeric_std yields 0).
    pub fn to_u64(&self) -> Option<u64> {
        self.to_bitvec()?.to_u64()
    }
    pub fn to_i64(&self) -> Option<i64> {
        self.to_bitvec()?.to_i64()
    }
}

impl<R: Repr> fmt::Display for Logic<R> {
    /// Left to right: highest element first, characters `UX01ZWLH-`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s: String = (0..self.len())
            .rev()
            .map(|i| self.get(i).to_char())
            .collect();
        f.write_str(&s)
    }
}

impl<R: Repr> FromStr for Logic<R> {
    type Err = char;
    /// Returns the first character that is not a `std_ulogic` literal.
    fn from_str(s: &str) -> Result<Self, char> {
        let v = s
            .chars()
            .map(|c| StdULogic::from_char(c).ok_or(c))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self::from_fn(v.len(), |i| v[v.len() - 1 - i]))
    }
}
