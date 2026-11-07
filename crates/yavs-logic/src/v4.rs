//! Verilog 4-state vectors and the operators of IEEE 1364-2005 clause 5 on them. Operands arrive
//! at the width the Verilog front end decided (`yavs-verilog` `sem.rs`, 5.4 and 5.5): the binary operators here take two
//! vectors of one width, except shifts and `**`, whose right operand is self-determined.
//!
//! Each bit is a pair of plane bits `(a, b)`: 0 is `(0, 0)`, 1 is `(1, 0)`, z is `(0, 1)`, x is
//! `(1, 1)`. Bits above the width are 0 in both planes.

use crate::{LogicVec, StdULogic};

/// One bit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bit {
    Zero,
    One,
    X,
    Z,
}

impl Bit {
    fn planes(self) -> (u64, u64) {
        match self {
            Bit::Zero => (0, 0),
            Bit::One => (1, 0),
            Bit::Z => (0, 1),
            Bit::X => (1, 1),
        }
    }

    pub fn char(self) -> char {
        match self {
            Bit::Zero => '0',
            Bit::One => '1',
            Bit::X => 'x',
            Bit::Z => 'z',
        }
    }
}

/// A 4-state vector of at least one bit. Bit 0 is the least significant.
#[derive(Clone, PartialEq, Eq, Debug)]
pub struct V4 {
    w: u32,
    a: Vec<u64>,
    b: Vec<u64>,
}

fn words(w: u32) -> usize {
    (w as usize).div_ceil(64).max(1)
}

impl V4 {
    fn fill(w: u32, a: u64, b: u64) -> V4 {
        let mut v = V4 {
            w,
            a: vec![a; words(w)],
            b: vec![b; words(w)],
        };
        v.mask();
        v
    }

    pub fn zeros(w: u32) -> V4 {
        V4::fill(w, 0, 0)
    }

    pub fn ones(w: u32) -> V4 {
        V4::fill(w, !0, 0)
    }

    pub fn xs(w: u32) -> V4 {
        V4::fill(w, !0, !0)
    }

    pub fn zs(w: u32) -> V4 {
        V4::fill(w, 0, !0)
    }

    pub fn from_u64(w: u32, v: u64) -> V4 {
        let mut r = V4::zeros(w);
        r.a[0] = v;
        r.mask();
        r
    }

    /// `v` sign-extended to `w` bits (two's complement).
    pub fn from_i64(w: u32, v: i64) -> V4 {
        let mut r = V4::fill(w, if v < 0 { !0 } else { 0 }, 0);
        r.a[0] = v as u64;
        r.mask();
        r
    }

    /// Known bits from little-endian words.
    pub fn from_words(w: u32, ws: &[u64]) -> V4 {
        let mut r = V4::zeros(w);
        for (d, s) in r.a.iter_mut().zip(ws) {
            *d = *s;
        }
        r.mask();
        r
    }

    /// From planes: `a` and `b` as described in the module docs.
    pub fn from_planes(w: u32, a: &[u64], b: &[u64]) -> V4 {
        let mut r = V4::zeros(w);
        for i in 0..r.a.len() {
            r.a[i] = a.get(i).copied().unwrap_or(0);
            r.b[i] = b.get(i).copied().unwrap_or(0);
        }
        r.mask();
        r
    }

    pub fn planes(&self) -> (&[u64], &[u64]) {
        (&self.a, &self.b)
    }

    fn mask(&mut self) {
        let r = self.w % 64;
        let n = words(self.w);
        self.a.truncate(n);
        self.b.truncate(n);
        if r != 0 {
            let m = (1u64 << r) - 1;
            self.a[n - 1] &= m;
            self.b[n - 1] &= m;
        }
    }

    pub fn width(&self) -> u32 {
        self.w
    }

    pub fn get(&self, i: u32) -> Bit {
        let (w, s) = ((i / 64) as usize, i % 64);
        match (self.a[w] >> s & 1, self.b[w] >> s & 1) {
            (0, 0) => Bit::Zero,
            (1, 0) => Bit::One,
            (0, _) => Bit::Z,
            _ => Bit::X,
        }
    }

    pub fn set(&mut self, i: u32, v: Bit) {
        let (w, s) = ((i / 64) as usize, i % 64);
        let (a, b) = v.planes();
        self.a[w] = self.a[w] & !(1 << s) | a << s;
        self.b[w] = self.b[w] & !(1 << s) | b << s;
    }

    /// No bit is x or z.
    pub fn is_known(&self) -> bool {
        self.b.iter().all(|&w| w == 0)
    }

    pub fn is_zero(&self) -> bool {
        self.is_known() && self.a.iter().all(|&w| w == 0)
    }

    /// The low 64 bits as unsigned, when every bit is known and the rest are 0.
    pub fn to_u64(&self) -> Option<u64> {
        (self.is_known() && self.a[1..].iter().all(|&w| w == 0)).then_some(self.a[0])
    }

    /// The value as a signed (`signed`) or unsigned integer, when known and in range of `i64`.
    pub fn to_i64(&self, signed: bool) -> Option<i64> {
        if !self.is_known() {
            return None;
        }
        if signed && self.msb() == Bit::One {
            let m = self.neg();
            let u = m.to_u64()?;
            return if u == 1 << 63 {
                Some(i64::MIN)
            } else {
                i64::try_from(u).ok().map(|u| -u)
            };
        }
        self.to_u64().and_then(|u| i64::try_from(u).ok())
    }

    pub fn msb(&self) -> Bit {
        self.get(self.w - 1)
    }

    /// Truncated or extended to `w` bits; extension copies the sign bit (x and z too) when
    /// `signed`, else fills with 0 (IEEE 1364-2005 5.5).
    pub fn ext(&self, w: u32, signed: bool) -> V4 {
        if w <= self.w {
            let mut r = V4 {
                w,
                a: self.a.clone(),
                b: self.b.clone(),
            };
            r.mask();
            return r;
        }
        let fill = if signed { self.msb() } else { Bit::Zero };
        let mut r = match fill {
            Bit::Zero => V4::zeros(w),
            Bit::One => V4::ones(w),
            Bit::X => V4::xs(w),
            Bit::Z => V4::zs(w),
        };
        r.put(0, self);
        r
    }

    /// Bits `lo .. lo + n`; bits outside `0 .. width` are x (5.2.1).
    pub fn slice(&self, lo: i64, n: u32) -> V4 {
        let mut r = V4::zeros(n);
        for i in 0..n {
            let k = lo + i as i64;
            let v = if (0..self.w as i64).contains(&k) {
                self.get(k as u32)
            } else {
                Bit::X
            };
            r.set(i, v);
        }
        r
    }

    /// Writes `v` at bits `lo ..`, dropping what falls outside the width.
    pub fn put(&mut self, lo: u32, v: &V4) {
        // ponytail: bit by bit; word copies if part writes show in a profile.
        for i in 0..v.w {
            if lo + i < self.w {
                self.set(lo + i, v.get(i));
            }
        }
    }

    /// `{hi, lo}`.
    pub fn concat(hi: &V4, lo: &V4) -> V4 {
        let mut r = V4::zeros(hi.w + lo.w);
        r.put(0, lo);
        r.put(lo.w, hi);
        r
    }

    // Bitwise operators (5.1.10). z reads as x.

    pub fn not(&self) -> V4 {
        let mut r = self.clone();
        for i in 0..r.a.len() {
            // 0 -> 1, 1 -> 0, x and z -> x.
            r.a[i] = !self.a[i] | self.b[i];
            r.b[i] = self.b[i];
        }
        r.mask();
        r
    }

    pub fn and(&self, o: &V4) -> V4 {
        self.zip(o, |(a1, b1), (a2, b2)| {
            // A known 0 on either side wins; else x if either is unknown.
            let z1 = !a1 & !b1;
            let z2 = !a2 & !b2;
            let zero = z1 | z2;
            let unk = (b1 | b2) & !zero;
            (!zero, unk)
        })
    }

    pub fn or(&self, o: &V4) -> V4 {
        self.zip(o, |(a1, b1), (a2, b2)| {
            let one = (a1 & !b1) | (a2 & !b2);
            let unk = (b1 | b2) & !one;
            (one | unk, unk)
        })
    }

    pub fn xor(&self, o: &V4) -> V4 {
        self.zip(o, |(a1, b1), (a2, b2)| {
            let unk = b1 | b2;
            ((a1 ^ a2) | unk, unk)
        })
    }

    pub fn xnor(&self, o: &V4) -> V4 {
        self.xor(o).not()
    }

    fn zip(&self, o: &V4, f: impl Fn((u64, u64), (u64, u64)) -> (u64, u64)) -> V4 {
        debug_assert_eq!(self.w, o.w);
        let mut r = V4::zeros(self.w);
        for i in 0..r.a.len() {
            let (a, b) = f((self.a[i], self.b[i]), (o.a[i], o.b[i]));
            r.a[i] = a;
            r.b[i] = b;
        }
        r.mask();
        r
    }

    // Arithmetic (5.1.5): any x or z bit makes the whole result x.

    pub fn add(&self, o: &V4) -> V4 {
        if !self.is_known() || !o.is_known() {
            return V4::xs(self.w);
        }
        let mut r = V4::zeros(self.w);
        let mut c = 0u64;
        for i in 0..r.a.len() {
            let (s, c1) = self.a[i].overflowing_add(o.a[i]);
            let (s, c2) = s.overflowing_add(c);
            r.a[i] = s;
            c = (c1 | c2) as u64;
        }
        r.mask();
        r
    }

    pub fn sub(&self, o: &V4) -> V4 {
        if !self.is_known() || !o.is_known() {
            return V4::xs(self.w);
        }
        self.add(&o.not().add(&V4::from_u64(self.w, 1)))
    }

    /// Unary minus: `0 - self`.
    pub fn neg(&self) -> V4 {
        V4::zeros(self.w).sub(self)
    }

    pub fn mul(&self, o: &V4) -> V4 {
        if !self.is_known() || !o.is_known() {
            return V4::xs(self.w);
        }
        let n = self.a.len();
        let mut acc = vec![0u64; n];
        for i in 0..n {
            let mut carry = 0u128;
            for j in 0..n - i {
                let t = self.a[i] as u128 * o.a[j] as u128 + acc[i + j] as u128 + carry;
                acc[i + j] = t as u64;
                carry = t >> 64;
            }
        }
        V4::from_words(self.w, &acc)
    }

    fn is_neg(&self, signed: bool) -> bool {
        signed && self.msb() == Bit::One
    }

    fn abs(&self, signed: bool) -> V4 {
        if self.is_neg(signed) {
            self.neg()
        } else {
            self.clone()
        }
    }

    /// Unsigned quotient and remainder of known operands, `d` not zero.
    fn divmod_u(&self, d: &V4) -> (V4, V4) {
        let w = self.w;
        let (mut q, mut r) = (V4::zeros(w), V4::zeros(w));
        for i in (0..w).rev() {
            // r = r << 1 | bit i; `over` is the bit shifted out (r is then at least 2^w > d).
            let over = r.get(w - 1) == Bit::One;
            let mut c = self.get(i) == Bit::One;
            for k in 0..r.a.len() {
                let top = r.a[k] >> 63;
                r.a[k] = r.a[k] << 1 | c as u64;
                c = top == 1;
            }
            r.mask();
            if over || !r.lt_u(d) {
                r = r.sub(d);
                q.set(i, Bit::One);
            }
        }
        (q, r)
    }

    fn lt_u(&self, o: &V4) -> bool {
        for i in (0..self.a.len()).rev() {
            if self.a[i] != o.a[i] {
                return self.a[i] < o.a[i];
            }
        }
        false
    }

    /// `/`: truncates towards zero; division by zero is x.
    pub fn div(&self, o: &V4, signed: bool) -> V4 {
        if !self.is_known() || !o.is_known() || o.is_zero() {
            return V4::xs(self.w);
        }
        let (q, _) = self.abs(signed).divmod_u(&o.abs(signed));
        if self.is_neg(signed) != o.is_neg(signed) {
            q.neg()
        } else {
            q
        }
    }

    /// `%`: the sign of the first operand; modulus zero is x.
    pub fn rem(&self, o: &V4, signed: bool) -> V4 {
        if !self.is_known() || !o.is_known() || o.is_zero() {
            return V4::xs(self.w);
        }
        let (_, r) = self.abs(signed).divmod_u(&o.abs(signed));
        if self.is_neg(signed) { r.neg() } else { r }
    }

    /// `self ** e` with the result in the width of `self` (5.1.5, Table 5-6); `e` is
    /// self-determined, negative only when `e_signed`.
    pub fn pow(&self, e: &V4, signed: bool, e_signed: bool) -> V4 {
        let w = self.w;
        if !self.is_known() || !e.is_known() {
            return V4::xs(w);
        }
        if e.is_neg(e_signed) {
            let one = V4::from_u64(w, 1);
            let minus_one = V4::ones(w);
            return if self.is_zero() {
                V4::xs(w)
            } else if *self == one {
                one
            } else if signed && *self == minus_one {
                // -1 ** odd is -1, -1 ** even is 1.
                if e.get(0) == Bit::One { minus_one } else { one }
            } else {
                V4::zeros(w)
            };
        }
        // Square and multiply over the bits of `e`, modulo 2^w.
        let (mut r, mut base) = (V4::from_u64(w, 1), self.clone());
        for i in 0..e.w {
            if e.get(i) == Bit::One {
                r = r.mul(&base);
            }
            base = base.mul(&base);
        }
        r
    }

    // Shifts (5.1.12): an x or z bit in the amount makes the result x.

    fn amount(n: &V4) -> Option<u64> {
        if !n.is_known() {
            return None;
        }
        // A known amount beyond 64 bits shifts everything out, as `u64::MAX` does.
        Some(n.to_u64().unwrap_or(u64::MAX))
    }

    pub fn shl(&self, n: &V4) -> V4 {
        let Some(k) = V4::amount(n) else {
            return V4::xs(self.w);
        };
        self.shift(-(k.min(self.w as u64) as i64), Bit::Zero)
    }

    /// `>>`, or `>>>` with `arith` (only for a signed left operand: 5.1.12).
    pub fn shr(&self, n: &V4, arith: bool) -> V4 {
        let Some(k) = V4::amount(n) else {
            return V4::xs(self.w);
        };
        let fill = if arith { self.msb() } else { Bit::Zero };
        self.shift(k.min(self.w as u64) as i64, fill)
    }

    /// Bit `i` of the result is bit `i + k` of `self`, `fill` beyond the width.
    fn shift(&self, k: i64, fill: Bit) -> V4 {
        let mut r = V4::zeros(self.w);
        for i in 0..self.w {
            let s = i as i64 + k;
            let v = if (0..self.w as i64).contains(&s) {
                self.get(s as u32)
            } else if s < 0 {
                Bit::Zero
            } else {
                fill
            };
            r.set(i, v);
        }
        r
    }

    // Relational and equality operators (5.1.7, 5.1.8): one bit.

    pub fn bit(b: Bit) -> V4 {
        let mut r = V4::zeros(1);
        r.set(0, b);
        r
    }

    fn from_bool(v: bool) -> V4 {
        V4::from_u64(1, v as u64)
    }

    fn cmp(&self, o: &V4, signed: bool) -> Option<std::cmp::Ordering> {
        if !self.is_known() || !o.is_known() {
            return None;
        }
        let (n1, n2) = (self.is_neg(signed), o.is_neg(signed));
        if n1 != n2 {
            return Some(if n1 {
                std::cmp::Ordering::Less
            } else {
                std::cmp::Ordering::Greater
            });
        }
        // Same sign: two's complement order equals unsigned order.
        for i in (0..self.a.len()).rev() {
            if self.a[i] != o.a[i] {
                return Some(self.a[i].cmp(&o.a[i]));
            }
        }
        Some(std::cmp::Ordering::Equal)
    }

    pub fn lt(&self, o: &V4, signed: bool) -> V4 {
        self.cmp(o, signed)
            .map_or(V4::bit(Bit::X), |c| V4::from_bool(c.is_lt()))
    }

    pub fn le(&self, o: &V4, signed: bool) -> V4 {
        self.cmp(o, signed)
            .map_or(V4::bit(Bit::X), |c| V4::from_bool(c.is_le()))
    }

    pub fn gt(&self, o: &V4, signed: bool) -> V4 {
        o.lt(self, signed)
    }

    pub fn ge(&self, o: &V4, signed: bool) -> V4 {
        o.le(self, signed)
    }

    /// `==`: 0 when two known bits differ, else x when a bit is x or z, else 1.
    pub fn eq(&self, o: &V4) -> V4 {
        let mut unk = false;
        for i in 0..self.a.len() {
            let known = !self.b[i] & !o.b[i];
            if (self.a[i] ^ o.a[i]) & known != 0 {
                return V4::from_bool(false);
            }
            unk |= (self.b[i] | o.b[i]) != 0;
        }
        if unk {
            V4::bit(Bit::X)
        } else {
            V4::from_bool(true)
        }
    }

    pub fn ne(&self, o: &V4) -> V4 {
        self.eq(o).not()
    }

    /// `===`: bit for bit, x and z included.
    pub fn case_eq(&self, o: &V4) -> V4 {
        V4::from_bool(self == o)
    }

    // Logical operators (5.1.9) and truth values.

    /// The truth of a value: 1 when a bit is 1, 0 when all bits are 0, else x.
    pub fn truth(&self) -> Bit {
        let one = (0..self.a.len()).any(|i| self.a[i] & !self.b[i] != 0);
        if one {
            Bit::One
        } else if self.is_known() {
            Bit::Zero
        } else {
            Bit::X
        }
    }

    pub fn lnot(&self) -> V4 {
        V4::bit(self.truth()).not()
    }

    pub fn land(&self, o: &V4) -> V4 {
        V4::bit(self.truth()).and(&V4::bit(o.truth()))
    }

    pub fn lor(&self, o: &V4) -> V4 {
        V4::bit(self.truth()).or(&V4::bit(o.truth()))
    }

    // Reductions (5.1.11).

    pub fn red_and(&self) -> V4 {
        let zero = (0..self.w).any(|i| self.get(i) == Bit::Zero);
        if zero {
            V4::from_bool(false)
        } else if self.is_known() {
            V4::from_bool(true)
        } else {
            V4::bit(Bit::X)
        }
    }

    pub fn red_or(&self) -> V4 {
        V4::bit(self.truth())
    }

    pub fn red_xor(&self) -> V4 {
        if !self.is_known() {
            return V4::bit(Bit::X);
        }
        let ones: u32 = self.a.iter().map(|w| w.count_ones()).sum();
        V4::from_bool(ones % 2 == 1)
    }

    /// `?:` with an x or z condition (5.1.13, Table 5-21): equal known bits stay, the rest are x.
    pub fn merge(&self, o: &V4) -> V4 {
        self.zip(o, |(a1, b1), (a2, b2)| {
            let same = !(a1 ^ a2) & !b1 & !b2;
            let a = (a1 & same) | !same;
            (a, !same)
        })
    }

    /// `$clog2`: the ceiling of log2 of the value as unsigned, 0 for 0 and 1, 32 bits (17.11.1).
    pub fn clog2(&self) -> V4 {
        if !self.is_known() {
            return V4::xs(32);
        }
        let top = (0..self.w).rev().find(|&i| self.get(i) == Bit::One);
        let r = match top {
            None => 0,
            Some(t) => {
                let pow2 = (0..t).all(|i| self.get(i) == Bit::Zero);
                if pow2 { t } else { t + 1 }
            }
        };
        V4::from_u64(32, r as u64)
    }

    /// A `case` item match (9.5.1): `===`, or bits that are z (`casez`) or x and z (`casex`,
    /// `x`) on either side match anything.
    pub fn case_match(&self, o: &V4, wild: bool, x: bool) -> bool {
        if !wild {
            return self == o;
        }
        (0..self.w).all(|i| {
            let (a, b) = (self.get(i), o.get(i));
            let care = |c: Bit| c == Bit::Z || x && c == Bit::X;
            care(a) || care(b) || a == b
        })
    }

    /// As a real; an x or z bit reads as 0 (4.8.2).
    pub fn to_f64(&self, signed: bool) -> f64 {
        let neg = signed && self.msb() == Bit::One && self.is_known();
        let m = if neg { self.neg() } else { self.clone() };
        let mut r = 0.0f64;
        for i in (0..m.width()).rev() {
            r = r * 2.0 + if m.get(i) == Bit::One { 1.0 } else { 0.0 };
        }
        if neg { -r } else { r }
    }

    /// A real rounded to a two's complement integer of `w` bits (4.8.2).
    pub fn from_f64(w: u32, r: f64) -> V4 {
        V4::from_i64(64, real_to_i64(r)).ext(w, true)
    }

    // Values at the boundary: kernel logic vectors, element 0 the least significant bit.

    /// From a VHDL value read as `TO_X01Z`: L and H are 0 and 1, U, W and `-` are x.
    pub fn from_logic(l: &LogicVec) -> V4 {
        let mut r = V4::zeros(l.len().max(1) as u32);
        for i in 0..l.len() {
            r.set(i as u32, of_ulogic(l.get(i)));
        }
        r
    }

    pub fn to_logic(&self) -> LogicVec {
        LogicVec::from_fn(self.w as usize, |i| match self.get(i as u32) {
            Bit::Zero => StdULogic::Zero,
            Bit::One => StdULogic::One,
            Bit::X => StdULogic::X,
            Bit::Z => StdULogic::Z,
        })
    }
}

pub fn of_ulogic(c: StdULogic) -> Bit {
    match c.to_x01z() {
        StdULogic::Zero => Bit::Zero,
        StdULogic::One => Bit::One,
        StdULogic::Z => Bit::Z,
        _ => Bit::X,
    }
}

impl std::fmt::Display for V4 {
    /// Binary, most significant bit first.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for i in (0..self.w).rev() {
            write!(f, "{}", self.get(i).char())?;
        }
        Ok(())
    }
}

/// A real rounded to the nearest integer, ties away from zero (4.8.2); out of range saturates.
pub fn real_to_i64(r: f64) -> i64 {
    r.round() as i64
}
