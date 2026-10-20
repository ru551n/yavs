//! Pure 0/1 vector on `u64` words: the fast path for `bit_vector` and for numeric_std arithmetic
//! once a `Logic` vector is known to hold only `0 1 L H`. Bits above `len` are always zero.

use core::cmp::Ordering;

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct BitVec {
    len: usize,
    w: Vec<u64>,
}

impl BitVec {
    pub fn new(len: usize) -> Self {
        Self {
            len,
            w: vec![0; len.div_ceil(64)],
        }
    }
    /// `w` must hold `len.div_ceil(64)` words; bits above `len` are cleared.
    pub fn from_words(len: usize, w: Vec<u64>) -> Self {
        assert_eq!(w.len(), len.div_ceil(64));
        let mut s = Self { len, w };
        s.norm();
        s
    }
    pub fn words(&self) -> &[u64] {
        &self.w
    }
    pub fn len(&self) -> usize {
        self.len
    }
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }
    fn norm(&mut self) {
        if !self.len.is_multiple_of(64) {
            *self.w.last_mut().unwrap() &= (1u64 << (self.len % 64)) - 1;
        }
    }
    pub fn get(&self, i: usize) -> bool {
        self.w[i >> 6] >> (i & 63) & 1 == 1
    }
    pub fn set(&mut self, i: usize, b: bool) {
        self.w[i >> 6] = self.w[i >> 6] & !(1 << (i & 63)) | (b as u64) << (i & 63);
    }
    pub fn not(&self) -> Self {
        let mut s = Self {
            len: self.len,
            w: self.w.iter().map(|x| !x).collect(),
        };
        s.norm();
        s
    }
    fn zip(&self, o: &Self, f: impl Fn(u64, u64) -> u64) -> Self {
        assert_eq!(self.len, o.len);
        Self {
            len: self.len,
            w: self.w.iter().zip(&o.w).map(|(&a, &b)| f(a, b)).collect(),
        }
    }
    pub fn and(&self, o: &Self) -> Self {
        self.zip(o, |a, b| a & b)
    }
    pub fn or(&self, o: &Self) -> Self {
        self.zip(o, |a, b| a | b)
    }
    pub fn xor(&self, o: &Self) -> Self {
        self.zip(o, |a, b| a ^ b)
    }
    /// Widen to `len` bits (`len >= self.len()`), zero- or sign-extending.
    pub fn extend(&self, len: usize, signed: bool) -> Self {
        let mut s = Self {
            len,
            w: self.w.clone(),
        };
        s.w.resize(len.div_ceil(64), 0);
        if signed && self.len > 0 && self.get(self.len - 1) {
            if let (Some(t), true) = (s.w.get_mut(self.w.len() - 1), !self.len.is_multiple_of(64)) {
                *t |= !0 << (self.len % 64);
            }
            s.w.iter_mut().skip(self.w.len()).for_each(|x| *x = !0);
        }
        s.norm();
        s
    }
    /// Wrapping `self + o + carry`; equal lengths.
    pub fn add(&self, o: &Self, carry: bool) -> Self {
        assert_eq!(self.len, o.len);
        let mut c = carry as u64;
        let w = self
            .w
            .iter()
            .zip(&o.w)
            .map(|(&a, &b)| {
                let (s1, c1) = a.overflowing_add(b);
                let (s2, c2) = s1.overflowing_add(c);
                c = (c1 | c2) as u64;
                s2
            })
            .collect();
        Self::from_words(self.len, w)
    }
    /// `self * o` modulo 2^`len` (operands taken as unsigned words, zero-extended); a signed
    /// product is the same after sign-extending both operands to `len` first.
    pub fn mul_trunc(&self, o: &Self, len: usize) -> Self {
        let n = len.div_ceil(64);
        let mut w = vec![0u64; n];
        for (i, &a) in self.w.iter().enumerate().take(n) {
            let mut carry = 0u128;
            for (j, &b) in o.w.iter().enumerate().take(n - i) {
                let t = u128::from(a) * u128::from(b) + u128::from(w[i + j]) + carry;
                w[i + j] = t as u64;
                carry = t >> 64;
            }
            let mut k = i + o.w.len().min(n - i);
            while carry > 0 && k < n {
                let t = u128::from(w[k]) + carry;
                w[k] = t as u64;
                carry = t >> 64;
                k += 1;
            }
        }
        Self::from_words(len, w)
    }
    /// Wrapping `self - o`; equal lengths.
    pub fn sub(&self, o: &Self) -> Self {
        self.add(&o.not(), true)
    }
    /// Compare as unsigned, or as two's complement when `signed`; equal lengths.
    pub fn cmp_num(&self, o: &Self, signed: bool) -> Ordering {
        assert_eq!(self.len, o.len);
        // Flipping the sign bit turns the two's complement order into the unsigned one.
        let flip = if signed && self.len > 0 {
            1u64 << ((self.len - 1) % 64)
        } else {
            0
        };
        let top = self.w.len().wrapping_sub(1);
        self.w
            .iter()
            .zip(&o.w)
            .enumerate()
            .rev()
            .map(|(i, (&a, &b))| {
                let f = if i == top { flip } else { 0 };
                (a ^ f).cmp(&(b ^ f))
            })
            .find(|c| c.is_ne())
            .unwrap_or(Ordering::Equal)
    }
    /// Word `i` of the vector widened to any length: zero or sign extension (`signed`).
    fn ext_word(&self, i: usize, signed: bool) -> u64 {
        let neg = signed && self.len > 0 && self.get(self.len - 1);
        match self.w.get(i) {
            Some(&w) if neg && i == self.w.len() - 1 && !self.len.is_multiple_of(64) => {
                w | !0 << (self.len % 64)
            }
            Some(&w) => w,
            None if neg => !0,
            None => 0,
        }
    }
    /// `self.extend(n, signed).cmp_num(&o.extend(n, signed), signed)` for `n` the longer
    /// length, without building the widened vectors.
    pub fn cmp_ext(&self, o: &Self, signed: bool) -> Ordering {
        let n = self.len.max(o.len);
        let top = n.div_ceil(64).wrapping_sub(1);
        let (mask, flip) = if n.is_multiple_of(64) {
            (!0, 1 << 63)
        } else {
            ((1u64 << (n % 64)) - 1, 1 << ((n - 1) % 64))
        };
        let flip = if signed && n > 0 { flip } else { 0 };
        (0..=top)
            .rev()
            .map(|i| {
                let m = if i == top { mask } else { !0 };
                let f = if i == top { flip } else { 0 };
                let (a, b) = (self.ext_word(i, signed) & m, o.ext_word(i, signed) & m);
                (a ^ f).cmp(&(b ^ f))
            })
            .find(|c| c.is_ne())
            .unwrap_or(Ordering::Equal)
    }
    /// Unsigned value, `None` if it does not fit.
    pub fn to_u64(&self) -> Option<u64> {
        if self.w.iter().skip(1).any(|&x| x != 0) {
            return None;
        }
        Some(self.w.first().copied().unwrap_or(0))
    }
    /// Two's complement value, `None` if it does not fit. An empty vector is 0, as in numeric_std.
    pub fn to_i64(&self) -> Option<i64> {
        if self.len == 0 {
            return Some(0);
        }
        let neg = self.get(self.len - 1);
        if self.len <= 64 {
            let v = self.w[0];
            return Some(if neg && self.len < 64 {
                (v | !0 << self.len) as i64
            } else {
                v as i64
            });
        }
        // Fits iff bits 63.. are all equal to the sign.
        let fill = if neg { !0 } else { 0 };
        let last = self.w.len() - 1;
        let top = if self.len.is_multiple_of(64) {
            !0
        } else {
            (1u64 << (self.len % 64)) - 1
        };
        let ok = (self.w[0] >> 63 == u64::from(neg))
            && self.w[1..last].iter().all(|&x| x == fill)
            && self.w[last] == fill & top;
        ok.then(|| self.w[0] as i64)
    }
    /// Low `len` bits of `v` (modulo 2^len), as `TO_UNSIGNED`.
    pub fn from_u64(len: usize, v: u64) -> Self {
        let mut s = Self::new(len);
        if let Some(x) = s.w.first_mut() {
            *x = v;
        }
        s.norm();
        s
    }
    /// Two's complement of `v` in `len` bits, as `TO_SIGNED`.
    pub fn from_i64(len: usize, v: i64) -> Self {
        let mut s = Self {
            len,
            w: vec![if v < 0 { !0 } else { 0 }; len.div_ceil(64)],
        };
        if let Some(x) = s.w.first_mut() {
            *x = v as u64;
        }
        s.norm();
        s
    }
}
