//! Word-wise paths of the `numeric_std` and `std_logic_1164` natives for vectors of up to
//! [`MAX_BITS`] elements without a metavalue, where [`crate::ieee::small`] (one word) does not
//! apply. Operands are read from their planes into a fixed buffer, the operation runs on
//! words (carries across them) and the result block is written straight into temporary
//! memory: no `LogicVec`, no allocation. Each function returns `None` for anything it does not
//! take (a metavalue, a longer vector, a rotation, an error or a warning), and the native's
//! general path then words it. Part of the natives' one-word-or-wide fast path (`ieee::FastFn`).

use yavs_ir::DOPE_DIM_BYTES;
use yavs_ir::ieee::{ARRAY_REGS, Cmp, Gate};

use crate::exec::Machine;
use crate::ieee::{gate_bits, holds, low, put_down, put_small};

/// The words of an operand or a result buffer.
const WORDS: usize = 16;
/// The longest vector this module takes.
pub(crate) const MAX_BITS: usize = WORDS * 64;
/// A second operand starts at this cell (`ieee::A1`).
const A1: usize = ARRAY_REGS;

type W = [u64; WORDS];

/// An array operand: its elements as bits (element 0 in bit 0 of word 0; bits at and above `n` are 0).
pub(crate) struct Wide {
    pub(crate) n: usize,
    pub(crate) w: W,
}

impl Wide {
    fn bit(&self, i: usize) -> bool {
        self.w[i / 64] >> (i % 64) & 1 == 1
    }
}

/// The array operand at cell `i`, `1` to [`MAX_BITS`] elements, every one `0 1 L H`; with `pure`
/// none an `L` or `H` (operations that keep the elements). `None` for any other operand.
pub(crate) fn wide_of(m: &Machine, a: &[u64], i: usize, pure: bool) -> Option<Wide> {
    let &[data, left, right, d] = a.get(i..i + ARRAY_REGS)? else {
        return None;
    };
    let (left, right) = (left as i64, right as i64);
    let (lo, hi) = if d == 1 { (right, left) } else { (left, right) };
    let n = usize::try_from(hi.checked_sub(lo)?).ok()?.checked_add(1)?;
    if n > MAX_BITS {
        return None;
    }
    let nb = n.div_ceil(64);
    let b = m.rd(data, nb as u64 * 32).ok()?;
    let mut w = [0u64; WORDS];
    for (k, (c, o)) in b.as_chunks::<32>().0.iter().zip(&mut w).enumerate() {
        let valid = low(n - k * 64);
        let (p0, p1, p2, p3) = (le(&c[0..8]), le(&c[8..16]), le(&c[16..24]), le(&c[24..32]));
        // `0 L` and `1 H` are the codes with plane 1 set and plane 3 clear; plane 2 is the weak bit.
        if !p3 & p1 & valid != valid || (pure && p2 & valid != 0) {
            return None;
        }
        *o = p0 & valid;
    }
    Some(Wide { n, w })
}

#[inline(always)]
fn le(b: &[u8]) -> u64 {
    u64::from_le_bytes(b.try_into().unwrap_or([0; 8]))
}

/// How the result's bounds are written.
#[derive(Clone, Copy)]
pub(crate) enum Bounds {
    /// `(n-1 downto 0)`: `numeric_std`.
    Down,
    /// `(1 to n)`: `std_logic_1164`.
    Up1,
}

/// The result `w` of `n` elements: a one-word block for `n <= 64`, else the fat pair, the dope
/// vector and the blocks of planes written straight into temporary memory.
pub(crate) fn emit(m: &mut Machine, w: &W, n: usize, bounds: Bounds) -> Option<u64> {
    let (left, right, down) = match bounds {
        Bounds::Down => (n as i64 - 1, 0, true),
        Bounds::Up1 => (1, n as i64, false),
    };
    if n <= 64 {
        return match bounds {
            Bounds::Down => put_down(m, n, w[0]),
            Bounds::Up1 => put_small(m, n, w[0], left, right, down),
        }
        .ok();
    }
    let size = n.div_ceil(64) as u64 * 32;
    let fat = m.temp_alloc(16 + DOPE_DIM_BYTES + size).ok()?;
    let b = m.wr(fat, 16 + DOPE_DIM_BYTES + size).ok()?;
    let (head, rest) = b.split_at_mut(16 + DOPE_DIM_BYTES as usize);
    let (dope, data) = (fat + 16, fat + 16 + DOPE_DIM_BYTES);
    for (d, v) in head.as_chunks_mut::<8>().0.iter_mut().zip([
        data,
        dope,
        left as u64,
        right as u64,
        u64::from(down),
    ]) {
        d.copy_from_slice(&v.to_le_bytes());
    }
    for (k, (c, x)) in rest.as_chunks_mut::<32>().0.iter_mut().zip(w).enumerate() {
        let mask = low(n - k * 64);
        for (d, v) in c
            .as_chunks_mut::<8>()
            .0
            .iter_mut()
            .zip([x & mask, mask, 0, 0])
        {
            d.copy_from_slice(&v.to_le_bytes());
        }
    }
    Some(fat)
}

/// Bits `lo..hi` of `w` set.
fn set_range(w: &mut W, lo: usize, hi: usize) {
    for (i, x) in w.iter_mut().enumerate().take(hi.div_ceil(64)).skip(lo / 64) {
        let (a, b) = (lo.max(i * 64) - i * 64, hi.min(i * 64 + 64) - i * 64);
        *x |= low(b) & !low(a);
    }
}

/// Bits at and above `n` cleared.
fn trim(w: &mut W, n: usize) {
    for (i, x) in w.iter_mut().enumerate() {
        *x &= if i * 64 >= n { 0 } else { low(n - i * 64) };
    }
}

/// The value of `x` of `n` elements as `to` elements, zero or sign extended.
fn extend(x: &Wide, to: usize, signed: bool) -> W {
    let mut w = x.w;
    if signed && x.bit(x.n - 1) {
        set_range(&mut w, x.n, to);
    }
    w
}

/// `x + y` or `x - y` over `n` elements, carries across the words.
fn add_words(x: &W, y: &W, n: usize, sub: bool) -> W {
    let mut out = [0u64; WORDS];
    let mut carry = u64::from(sub);
    for i in 0..n.div_ceil(64) {
        let b = if sub { !y[i] } else { y[i] };
        let (s, c1) = x[i].overflowing_add(b);
        let (s, c2) = s.overflowing_add(carry);
        out[i] = s;
        carry = u64::from(c1 | c2);
    }
    trim(&mut out, n);
    out
}

/// The integer `i` as `n` elements: its two's complement bits extended with its sign for a
/// `signed` operation, with zeros for an `unsigned` one (the operand is a NATURAL).
fn int_words(i: i64, n: usize, signed: bool) -> W {
    let mut w = [0u64; WORDS];
    w[0] = i as u64;
    if signed && i < 0 {
        set_range(&mut w, 64, n);
    }
    w
}

/// `+` and `-` of `numeric_std` (`form` 0: two vectors, 1: vector and integer, 2: integer and
/// vector), where at least one vector is longer than 64 elements or the result is.
pub(crate) fn add(m: &mut Machine, a: &[u64], sub: bool, signed: bool, form: u8) -> Option<u64> {
    let (n, x, y) = match form {
        0 => {
            let (l, r) = (wide_of(m, a, 0, false)?, wide_of(m, a, A1, false)?);
            let n = l.n.max(r.n);
            (n, extend(&l, n, signed), extend(&r, n, signed))
        }
        1 => {
            let l = wide_of(m, a, 0, false).filter(|l| l.n > 64)?;
            let i = int_operand(*a.get(A1)?, signed)?;
            (l.n, l.w, int_words(i, l.n, signed))
        }
        _ => {
            let r = wide_of(m, a, 1, false).filter(|r| r.n > 64)?;
            let i = int_operand(*a.first()?, signed)?;
            (r.n, int_words(i, r.n, signed), r.w)
        }
    };
    let s = add_words(&x, &y, n, sub);
    emit(m, &s, n, Bounds::Down)
}

/// An integer operand: a NATURAL one (`unsigned`) must not be negative.
fn int_operand(i: u64, signed: bool) -> Option<i64> {
    let i = i as i64;
    (signed || i >= 0).then_some(i)
}

/// The order of the numeric values of two `n`-element word strings.
fn cmp_words(x: &W, y: &W, n: usize, signed: bool) -> std::cmp::Ordering {
    let nb = n.div_ceil(64);
    for i in (0..nb).rev() {
        let (p, q) = (x[i], y[i]);
        if p != q {
            // The sign is in the top element: in the top word it is a signed comparison.
            return if signed && i == nb - 1 {
                // Sign-extended to the top word's end, so the word compares as `i64`.
                (p as i64).cmp(&(q as i64))
            } else {
                p.cmp(&q)
            };
        }
    }
    std::cmp::Ordering::Equal
}

/// The comparison `cmp` of `numeric_std` (forms as in [`add`]); `None` unless at least one vector
/// is longer than 64 elements.
pub(crate) fn compare(m: &Machine, a: &[u64], cmp: Cmp, signed: bool, form: u8) -> Option<u64> {
    let (n, x, y) = match form {
        0 => {
            let (l, r) = (wide_of(m, a, 0, false)?, wide_of(m, a, A1, false)?);
            let n = l.n.max(r.n);
            (n, extend(&l, n, signed), extend(&r, n, signed))
        }
        1 => {
            let l = wide_of(m, a, 0, false).filter(|l| l.n > 64)?;
            let i = int_operand(*a.get(A1)?, signed)?;
            (l.n, extend(&l, l.n, signed), int_words(i, l.n, signed))
        }
        _ => {
            let r = wide_of(m, a, 1, false).filter(|r| r.n > 64)?;
            let i = int_operand(*a.first()?, signed)?;
            (r.n, int_words(i, r.n, signed), extend(&r, r.n, signed))
        }
    };
    // A signed word string must be sign-extended to the end of its top word to compare as `i64`.
    let (x, y) = if signed {
        (fill_top(x, n), fill_top(y, n))
    } else {
        (x, y)
    };
    Some(u64::from(holds(cmp, cmp_words(&x, &y, n, signed))))
}

/// `w` of `n` elements with the sign element copied up to the end of its top word.
fn fill_top(mut w: W, n: usize) -> W {
    if w[(n - 1) / 64] >> ((n - 1) % 64) & 1 == 1 {
        set_range(&mut w, n, n.div_ceil(64) * 64);
    }
    w
}

/// `to_integer` of a vector longer than 64 elements whose value is an INTEGER within the
/// largest (`high`); anything else is the general path's.
pub(crate) fn to_integer(m: &Machine, a: &[u64], signed: bool, high: u64) -> Option<u64> {
    let x = wide_of(m, a, 0, false).filter(|x| x.n > 64)?;
    let nb = x.n.div_ceil(64);
    let neg = signed && x.bit(x.n - 1);
    let w = if signed { fill_top(x.w, x.n) } else { x.w };
    // The upper words are the sign (or zero), and the low word has the same sign bit.
    let ext = if neg { !0 } else { 0 };
    if w[1..nb].iter().any(|&u| u != ext) || (signed && (w[0] >> 63 == 1) != neg) {
        return None;
    }
    let mag = if neg { !w[0] } else { w[0] };
    (mag <= high).then(|| if neg { (-(mag as i64) - 1) as u64 } else { mag })
}

/// `to_unsigned` / `to_signed` of an integer to `size` elements, `size` from 65 to [`MAX_BITS`].
pub(crate) fn from_int(m: &mut Machine, arg: i64, size: usize, signed: bool) -> Option<u64> {
    if !(65..=MAX_BITS).contains(&size) {
        return None;
    }
    let w = int_words(int_operand(arg as u64, signed)?, size, signed);
    emit(m, &w, size, Bounds::Down)
}

/// `resize` of a vector to `new` elements where the argument or the result is longer than 64.
pub(crate) fn resize(m: &mut Machine, a: &[u64], signed: bool) -> Option<u64> {
    let x = wide_of(m, a, 0, true)?;
    let new = usize::try_from(*a.get(A1)? as i64).ok()?;
    if !(1..=MAX_BITS).contains(&new) || (x.n <= 64 && new <= 64) {
        return None;
    }
    let keep = x.n.min(new) - 1;
    let mut w = x.w;
    // The top element of the result is the argument's sign element (signed), the low
    // `keep` elements the argument's lowest.
    if signed {
        trim(&mut w, keep);
        if x.bit(x.n - 1) {
            set_range(&mut w, keep, new);
        }
    } else {
        trim(&mut w, new);
    }
    emit(m, &w, new, Bounds::Down)
}

/// `sll`/`srl` (`K` 0, 1) of a vector longer than 64 elements.
pub(crate) fn shift(m: &mut Machine, a: &[u64], k: u8) -> Option<u64> {
    let x = wide_of(m, a, 0, true).filter(|x| x.n > 64)?;
    let count = usize::try_from(*a.get(A1)? as i64).ok()?;
    let n = x.n;
    let mut out = [0u64; WORDS];
    if count < n {
        let (ws, bs) = (count / 64, count % 64);
        let at = |i: usize| x.w.get(i).copied().unwrap_or(0);
        for (i, o) in out.iter_mut().enumerate().take(n.div_ceil(64)) {
            *o = if k == 0 {
                // Left: word `i - ws`, with the top bits of the word below it.
                let lo = i.checked_sub(ws);
                let v = lo.map_or(0, |j| at(j) << bs);
                let c = if bs > 0 {
                    lo.and_then(|j| j.checked_sub(1))
                        .map_or(0, |j| at(j) >> (64 - bs))
                } else {
                    0
                };
                v | c
            } else {
                let v = at(i + ws) >> bs;
                let c = if bs > 0 {
                    at(i + ws + 1) << (64 - bs)
                } else {
                    0
                };
                v | c
            };
        }
        trim(&mut out, n);
    }
    emit(m, &out, n, Bounds::Down)
}

/// The gate `g` of two vectors of one length (`bounds` as the native writes them), or of one
/// vector for `not` (`y` is `None`), where an operand is longer than 64 elements.
pub(crate) fn gate(m: &mut Machine, a: &[u64], g: Option<Gate>, bounds: Bounds) -> Option<u64> {
    let x = wide_of(m, a, 0, false)?;
    let mut out = [0u64; WORDS];
    match g {
        Some(g) => {
            let y = wide_of(m, a, A1, false)?;
            if x.n != y.n || x.n <= 64 {
                return None;
            }
            // (Words past `n` are trimmed below.)
            for ((o, &p), &q) in out.iter_mut().zip(&x.w).zip(&y.w) {
                *o = gate_bits(g, p, q);
            }
        }
        None => {
            if x.n <= 64 {
                return None;
            }
            for (o, &p) in out.iter_mut().zip(&x.w) {
                *o = !p;
            }
        }
    }
    trim(&mut out, x.n);
    emit(m, &out, x.n, bounds)
}
