//! Signal value storage.
//!
//! A signal holds either plain bytes (integers, enums, reals, record fields: `elem_size` bytes
//! per element) or IEEE 1164 logic in yavs-logic's bit-plane layout (four planes of 64
//! elements per block). Element-addressed operations work on 64-element chunks with shifts and
//! masks, so a sub-range copy, compare or resolution never decodes single elements.

use std::fmt;

use smallvec::{SmallVec, smallvec};
use yavs_logic::wide::{chunks, get_block, nmask, put_block};
use yavs_logic::{Block, Logic, Planes, wide};

/// The kernel's logic vector: always bit-planes, whatever yavs-logic's default layout is.
pub type LogicVec = Logic<Planes>;

/// Element representation of a signal.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Elem {
    /// `n` opaque bytes per element.
    Bytes(u32),
    /// `std_ulogic`, stored as bit-planes.
    Logic,
}

/// The typed descriptor of a signal: element representation and element count.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SignalType {
    pub elem: Elem,
    pub len: u32,
}

impl SignalType {
    pub const fn bytes(elem_size: u32, len: u32) -> SignalType {
        SignalType {
            elem: Elem::Bytes(elem_size),
            len,
        }
    }

    pub const fn logic(len: u32) -> SignalType {
        SignalType {
            elem: Elem::Logic,
            len,
        }
    }
}

/// An owned value of some elements of a signal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Value {
    Bytes(Vec<u8>),
    Logic(LogicVec),
}

impl Value {
    pub fn as_bytes(&self) -> Option<&[u8]> {
        match self {
            Value::Bytes(b) => Some(b),
            Value::Logic(_) => None,
        }
    }

    pub fn as_logic(&self) -> Option<&LogicVec> {
        match self {
            Value::Logic(l) => Some(l),
            Value::Bytes(_) => None,
        }
    }

    pub fn as_ref(&self) -> ValueRef<'_> {
        match self {
            Value::Bytes(b) => ValueRef::Bytes(b),
            Value::Logic(l) => ValueRef::Logic(l),
        }
    }
}

impl fmt::Display for Value {
    /// Bytes as a list, logic as a VHDL string literal (high element first).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Value::Bytes(b) => write!(f, "{b:?}"),
            Value::Logic(l) => write!(f, "\"{l}\""),
        }
    }
}

/// A borrowed value passed into the kernel.
#[derive(Clone, Copy, Debug)]
pub enum ValueRef<'a> {
    Bytes(&'a [u8]),
    Logic(&'a LogicVec),
}

impl<'a> From<&'a [u8]> for ValueRef<'a> {
    fn from(b: &'a [u8]) -> ValueRef<'a> {
        ValueRef::Bytes(b)
    }
}

impl<'a, const N: usize> From<&'a [u8; N]> for ValueRef<'a> {
    fn from(b: &'a [u8; N]) -> ValueRef<'a> {
        ValueRef::Bytes(b)
    }
}

impl<'a> From<&'a Vec<u8>> for ValueRef<'a> {
    fn from(b: &'a Vec<u8>) -> ValueRef<'a> {
        ValueRef::Bytes(b)
    }
}

impl<'a> From<&'a LogicVec> for ValueRef<'a> {
    fn from(l: &'a LogicVec) -> ValueRef<'a> {
        ValueRef::Logic(l)
    }
}

impl<'a> From<&'a Value> for ValueRef<'a> {
    fn from(v: &'a Value) -> ValueRef<'a> {
        v.as_ref()
    }
}

/// Storage for a run of elements of one signal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Buf {
    Bytes { es: usize, data: SmallVec<[u8; 16]> },
    Planes { len: usize, b: SmallVec<[Block; 1]> },
}

/// `a == b` for slices of one length; the sizes of integers, enums and reals as word compares
/// (the generic slice compare is a library call).
#[inline(always)]
pub(crate) fn bytes_eq(a: &[u8], b: &[u8]) -> bool {
    if let (Ok(x), Ok(y)) = (<[u8; 4]>::try_from(a), <[u8; 4]>::try_from(b)) {
        return x == y;
    }
    if let (Ok(x), Ok(y)) = (<[u8; 8]>::try_from(a), <[u8; 8]>::try_from(b)) {
        return x == y;
    }
    if let (Ok(x), Ok(y)) = (<[u8; 1]>::try_from(a), <[u8; 1]>::try_from(b)) {
        return x == y;
    }
    a == b
}

/// `dst.copy_from_slice(src)` for the element sizes of byte signals, without the library call
/// of the generic copy (the lengths are equal).
#[inline(always)]
pub(crate) fn put_bytes(dst: &mut [u8], src: &[u8]) {
    if let (Ok(d), Ok(s)) = (
        <&mut [u8; 4]>::try_from(&mut *dst),
        <[u8; 4]>::try_from(src),
    ) {
        *d = s;
    } else if let (Ok(d), Ok(s)) = (
        <&mut [u8; 1]>::try_from(&mut *dst),
        <[u8; 1]>::try_from(src),
    ) {
        *d = s;
    } else if let (Ok(d), Ok(s)) = (
        <&mut [u8; 8]>::try_from(&mut *dst),
        <[u8; 8]>::try_from(src),
    ) {
        *d = s;
    } else {
        dst.copy_from_slice(src);
    }
}

/// Bits `off .. off + n` (`n <= 64`) of a bitset.
pub(crate) fn bits(words: &[u64], off: usize, n: usize) -> u64 {
    let (w, s) = (off / 64, off % 64);
    let mut v = words.get(w).map_or(0, |x| x >> s);
    if s != 0 {
        v |= words.get(w + 1).map_or(0, |x| x << (64 - s));
    }
    v & nmask(n)
}

/// Set or clear bits `off .. off + n` of a bitset, a word at a time.
pub(crate) fn fill_bits(words: &mut [u64], off: usize, n: usize, on: bool) {
    let (mut i, end) = (off, off + n);
    while i < end {
        let (s, k) = (i % 64, (64 - i % 64).min(end - i));
        let m = nmask(k) << s;
        if on {
            words[i / 64] |= m;
        } else {
            words[i / 64] &= !m;
        }
        i += k;
    }
}

/// Write the low `n` bits (at most 64) of `v` to bits `off .. off + n` of a bitset.
pub(crate) fn put_bits(words: &mut [u64], off: usize, n: usize, v: u64) {
    let (w, s) = (off / 64, off % 64);
    let v = v & nmask(n);
    let m = nmask(n);
    words[w] = words[w] & !(m << s) | v << s;
    if s != 0 && s + n > 64 {
        words[w + 1] = words[w + 1] & !(m >> (64 - s)) | v >> (64 - s);
    }
}

/// Per code (low four bits), each plane's bit spread over a whole word.
const SPLAT: [Block; 16] = {
    let mut t = [[0u64; 4]; 16];
    let mut c = 0;
    while c < 16 {
        let mut p = 0;
        while p < 4 {
            t[c][p] = if c >> p & 1 != 0 { !0 } else { 0 };
            p += 1;
        }
        c += 1;
    }
    t
};

/// Set the code of element `i` of plane blocks `b` (one masked blend of the four planes).
#[inline(always)]
pub(crate) fn put_plane_code(b: &mut [Block], i: usize, c: u8) {
    let (blk, m) = (&mut b[i >> 6], 1u64 << (i & 63));
    let v = &SPLAT[usize::from(c & 15)];
    for (w, x) in blk.iter_mut().zip(v) {
        *w = *w & !m | x & m;
    }
}

/// The `std_ulogic` code of element `i` of plane blocks.
#[inline(always)]
pub(crate) fn plane_code(b: &[Block], i: usize) -> u8 {
    let (blk, s) = (&b[i >> 6], i & 63);
    ((blk[0] >> s & 1) | (blk[1] >> s & 1) << 1 | (blk[2] >> s & 1) << 2 | (blk[3] >> s & 1) << 3)
        as u8
}

/// Whether element `i` of plane blocks holds `code`: the four planes against the code's
/// splat words, one test of bit `i`, no code assembled (fewer instructions than
/// `plane_code(b, i) == code`).
#[inline(always)]
pub(crate) fn plane_is(b: &[Block], i: usize, code: u8) -> bool {
    let (blk, v) = (&b[i >> 6], &SPLAT[usize::from(code & 15)]);
    ((blk[0] ^ v[0]) | (blk[1] ^ v[1]) | (blk[2] ^ v[2]) | (blk[3] ^ v[3])) >> (i & 63) & 1 == 0
}

impl Buf {
    /// `n` elements, all bytes zero or all logic `U`.
    pub fn zeroed(elem: Elem, n: usize) -> Buf {
        match elem {
            Elem::Bytes(es) => Buf::Bytes {
                es: es as usize,
                data: smallvec![0; n * es as usize],
            },
            Elem::Logic => Buf::Planes {
                len: n,
                b: smallvec![[0; 4]; n.div_ceil(64)],
            },
        }
    }

    /// `n` elements from `v`, checking its kind and size.
    pub fn from_ref(elem: Elem, n: usize, v: ValueRef<'_>) -> Result<Buf, String> {
        match (elem, v) {
            (Elem::Bytes(es), ValueRef::Bytes(d)) if d.len() == n * es as usize => Ok(Buf::Bytes {
                es: es as usize,
                data: SmallVec::from_slice(d),
            }),
            (Elem::Logic, ValueRef::Logic(l)) if l.len() == n => Ok(Buf::Planes {
                len: n,
                b: SmallVec::from_slice(l.repr().blocks()),
            }),
            (Elem::Bytes(es), ValueRef::Bytes(d)) => Err(format!(
                "value has {} bytes, expected {n} elements of {es} bytes",
                d.len()
            )),
            (Elem::Logic, ValueRef::Logic(l)) => Err(format!(
                "logic value has {} elements, expected {n}",
                l.len()
            )),
            (Elem::Bytes(_), ValueRef::Logic(_)) => Err("logic value for a byte signal".into()),
            (Elem::Logic, ValueRef::Bytes(_)) => Err("byte value for a logic signal".into()),
        }
    }

    /// `from_ref` into this buffer's storage when it already has that kind and size, so a
    /// recycled transaction value costs no allocation.
    pub fn refill(&mut self, elem: Elem, n: usize, v: ValueRef<'_>) -> Result<(), String> {
        match (&mut *self, elem, v) {
            (Buf::Bytes { es, data }, Elem::Bytes(e), ValueRef::Bytes(d))
                if *es == e as usize && d.len() == n * *es && data.len() == d.len() =>
            {
                data.copy_from_slice(d)
            }
            (Buf::Planes { len, b }, Elem::Logic, ValueRef::Logic(l))
                if l.len() == n && *len == n =>
            {
                b.copy_from_slice(l.repr().blocks())
            }
            _ => *self = Buf::from_ref(elem, n, v)?,
        }
        Ok(())
    }

    /// One logic element with `std_ulogic` code `c` (below 9).
    #[inline]
    pub fn one_code(c: u8) -> Buf {
        let p = |k: u8| u64::from(c >> k & 1);
        Buf::Planes {
            len: 1,
            b: smallvec![[p(0), p(1), p(2), p(3)]],
        }
    }

    /// Element `i` as one byte: its first byte, or its logic code.
    #[inline]
    pub fn code(&self, i: usize) -> u8 {
        match self {
            Buf::Bytes { es, data } => data[i * es],
            Buf::Planes { b, .. } => plane_code(b, i),
        }
    }

    /// `code` of a logic buffer, for callers that know the storage (one dispatch less).
    #[inline(always)]
    pub fn planes_code(&self, i: usize) -> Option<u8> {
        match self {
            Buf::Planes { b, .. } => Some(plane_code(b, i)),
            Buf::Bytes { .. } => None,
        }
    }

    /// The planes of element 0 of a one-element logic buffer, as bits 0 of four words.
    #[inline]
    pub fn first_planes(&self) -> Option<[u64; 4]> {
        match self {
            Buf::Planes { b, .. } => b.first().map(|x| x.map(|p| p & 1)),
            Buf::Bytes { .. } => None,
        }
    }

    /// Write `k` (at most the rest of the 64-element block) logic elements from `first`, given
    /// as plane words with element `first` in bit 0.
    #[inline]
    pub fn put_planes(&mut self, first: usize, k: usize, planes: [u64; 4]) {
        if let Buf::Planes { b, .. } = self {
            let (blk, s) = (&mut b[first / 64], first % 64);
            let m = nmask(k) << s;
            for (p, x) in blk.iter_mut().zip(planes) {
                *p = *p & !m | x << s;
            }
        }
    }

    /// Set the code of element `i` (a one-element `set_codes`, for logic storage).
    #[inline]
    pub fn put_code(&mut self, i: usize, c: u8) {
        match self {
            Buf::Bytes { es, data } => data[i * *es] = c,
            Buf::Planes { b, .. } => put_plane_code(b, i, c),
        }
    }

    /// The plane blocks of logic storage.
    #[inline(always)]
    pub fn planes_mut(&mut self) -> Option<&mut [Block]> {
        match self {
            Buf::Planes { b, .. } => Some(b),
            Buf::Bytes { .. } => None,
        }
    }

    /// `put_code`, returning the code the element had.
    #[inline]
    pub fn swap_code(&mut self, i: usize, c: u8) -> u8 {
        match self {
            Buf::Bytes { es, data } => std::mem::replace(&mut data[i * *es], c),
            Buf::Planes { b, .. } => {
                let (blk, s) = (&mut b[i >> 6], i & 63);
                let keep = !(1u64 << s);
                let c = u64::from(c);
                let old = (blk[0] >> s & 1)
                    | (blk[1] >> s & 1) << 1
                    | (blk[2] >> s & 1) << 2
                    | (blk[3] >> s & 1) << 3;
                blk[0] = blk[0] & keep | (c & 1) << s;
                blk[1] = blk[1] & keep | (c >> 1 & 1) << s;
                blk[2] = blk[2] & keep | (c >> 2 & 1) << s;
                blk[3] = blk[3] & keep | (c >> 3 & 1) << s;
                old as u8
            }
        }
    }

    /// The elements are on the heap (not stored inline).
    pub fn spilled(&self) -> bool {
        match self {
            Buf::Bytes { data, .. } => data.spilled(),
            Buf::Planes { b, .. } => b.spilled(),
        }
    }

    pub fn len(&self) -> usize {
        match self {
            Buf::Bytes { es, data } => data.len() / es,
            Buf::Planes { len, .. } => *len,
        }
    }

    /// Elements `off .. off + n` as bytes, for byte storage.
    #[inline]
    pub fn bytes(&self, off: usize, n: usize) -> Option<&[u8]> {
        match self {
            Buf::Bytes { es, data } => Some(&data[off * es..(off + n) * es]),
            Buf::Planes { .. } => None,
        }
    }

    pub fn extract(&self, off: usize, n: usize) -> Buf {
        match self {
            Buf::Bytes { es, data } => Buf::Bytes {
                es: *es,
                data: SmallVec::from_slice(&data[off * es..(off + n) * es]),
            },
            Buf::Planes { b, .. } => {
                let mut out: SmallVec<[Block; 1]> = smallvec![[0; 4]; n.div_ceil(64)];
                wide::copy_range(&mut out, 0, b, off, n);
                Buf::Planes { len: n, b: out }
            }
        }
    }

    /// Elements `off .. off + n` into `out` without allocating: the bytes, or for logic the
    /// plane blocks of `extract(off, n)` as little-endian words. False if `out` has another
    /// size.
    pub fn write_to(&self, off: usize, n: usize, out: &mut [u8]) -> bool {
        match self {
            Buf::Bytes { es, data } => {
                let src = &data[off * es..(off + n) * es];
                if out.len() != src.len() {
                    return false;
                }
                out.copy_from_slice(src);
            }
            // One word of at most 64 elements (a register read): no chunk loop.
            Buf::Planes { b, .. } if (1..=64).contains(&n) && out.len() == 32 => {
                let w = get_block(b, off, n);
                for (dst, x) in out.as_chunks_mut::<8>().0.iter_mut().zip(w) {
                    *dst = x.to_le_bytes();
                }
            }
            Buf::Planes { b, .. } => {
                let (blocks, rest) = out.as_chunks_mut::<32>();
                if !rest.is_empty() || blocks.len() != n.div_ceil(64) {
                    return false;
                }
                for ((c, k), dst) in chunks(n).zip(blocks) {
                    for (p, w) in get_block(b, off + c, k).iter().enumerate() {
                        dst[p * 8..p * 8 + 8].copy_from_slice(&w.to_le_bytes());
                    }
                }
            }
        }
        true
    }

    pub fn to_value(&self) -> Value {
        match self {
            Buf::Bytes { data, .. } => Value::Bytes(data.to_vec()),
            Buf::Planes { len, b } => {
                Value::Logic(Logic::from_repr(Planes::from_blocks(*len, b.to_vec())))
            }
        }
    }

    /// Copy `n` elements of `src` from `soff` to `off`.
    pub fn copy(&mut self, off: usize, src: &Buf, soff: usize, n: usize) {
        // One logic element (a process per element of a vector): no block gather.
        if n == 1
            && let (Buf::Planes { b, .. }, Buf::Planes { b: sb, .. }) = (&mut *self, src)
        {
            let (w, s) = (off / 64, off % 64);
            let (sw, ss) = (soff / 64, soff % 64);
            for k in 0..4 {
                b[w][k] = b[w][k] & !(1 << s) | (sb[sw][k] >> ss & 1) << s;
            }
            return;
        }
        self.select(off, src, soff, n, &[!0; 1], None);
    }

    /// Copy the elements of `src` (from `soff`) selected by bitset `mask` (bit `moff + i` for
    /// element `i`; all when `moff` is `None`) to `off`.
    pub fn select(
        &mut self,
        off: usize,
        src: &Buf,
        soff: usize,
        n: usize,
        mask: &[u64],
        moff: Option<usize>,
    ) {
        let sel = |c: usize, k: usize| moff.map_or(nmask(k), |m| bits(mask, m + c, k));
        match (self, src) {
            (Buf::Bytes { es, data }, Buf::Bytes { data: sd, .. }) => {
                let es = *es;
                if moff.is_none() {
                    data[off * es..(off + n) * es].copy_from_slice(&sd[soff * es..(soff + n) * es]);
                    return;
                }
                for (c, k) in chunks(n) {
                    let mut m = sel(c, k);
                    while m != 0 {
                        let i = c + m.trailing_zeros() as usize;
                        m &= m - 1;
                        data[(off + i) * es..(off + i + 1) * es]
                            .copy_from_slice(&sd[(soff + i) * es..(soff + i + 1) * es]);
                    }
                }
            }
            (Buf::Planes { b, .. }, Buf::Planes { b: sb, .. }) => {
                if moff.is_none() {
                    return wide::copy_range(b, off, sb, soff, n);
                }
                for (c, k) in chunks(n) {
                    put_block(b, off + c, k, get_block(sb, soff + c, k), sel(c, k));
                }
            }
            _ => unreachable!("storage kinds are checked when values enter the kernel"),
        }
    }

    /// Whether elements `off .. off + n` equal `v` (false for a value of another kind or size).
    pub fn same_as(&self, off: usize, n: usize, v: ValueRef<'_>) -> bool {
        match (self, v) {
            (Buf::Bytes { es, data }, ValueRef::Bytes(d)) => {
                d.len() == n * es
                    && data
                        .get(off * es..(off + n) * es)
                        .is_some_and(|a| bytes_eq(a, d))
            }
            (Buf::Planes { len, b }, ValueRef::Logic(l)) => {
                l.len() == n && off + n <= *len && wide::eq_range(b, off, l.repr().blocks(), 0, n)
            }
            _ => false,
        }
    }

    /// Bitset of the elements `0..n` where `self[off + i] != other[ooff + i]`.
    pub fn diff(&self, off: usize, other: &Buf, ooff: usize, n: usize) -> Vec<u64> {
        chunks(n)
            .map(|(c, k)| self.diff_chunk(off + c, other, ooff + c, k))
            .collect()
    }

    /// `diff` for at most 64 elements, as one word.
    pub fn diff_chunk(&self, off: usize, other: &Buf, ooff: usize, k: usize) -> u64 {
        match (self, other) {
            (Buf::Bytes { es, data }, Buf::Bytes { data: od, .. }) => {
                let es = *es;
                let (a, b) = (
                    &data[off * es..(off + k) * es],
                    &od[ooff * es..(ooff + k) * es],
                );
                if a == b {
                    return 0;
                }
                a.chunks_exact(es)
                    .zip(b.chunks_exact(es))
                    .enumerate()
                    .fold(0, |m, (i, (x, y))| if x != y { m | 1 << i } else { m })
            }
            (Buf::Planes { b, .. }, Buf::Planes { b: ob, .. }) if k == 1 => {
                let (w, s, ow, os) = (off / 64, off % 64, ooff / 64, ooff % 64);
                (0..4).fold(0, |a, p| a | ((b[w][p] >> s ^ ob[ow][p] >> os) & 1))
            }
            (Buf::Planes { b, .. }, Buf::Planes { b: ob, .. }) => {
                let (x, y) = (get_block(b, off, k), get_block(ob, ooff, k));
                (0..4).fold(0, |a, p| a | (x[p] ^ y[p]))
            }
            _ => unreachable!("storage kinds are checked when values enter the kernel"),
        }
    }

    /// IEEE 1164 resolution of `nsrc` sources (`src(i)`: the buffer and its element offset)
    /// into `off..off + n`.
    /// No sources gives `Z`, as the `resolved` function does for an empty array.
    pub fn resolve_std_logic_by<'a>(
        &mut self,
        off: usize,
        n: usize,
        nsrc: usize,
        src: impl Fn(usize) -> (&'a Buf, usize),
    ) {
        let Buf::Planes { b, .. } = self else {
            unreachable!("std_logic resolution on byte storage")
        };
        let planes = |i: usize| match src(i) {
            (Buf::Planes { b: sb, .. }, soff) => (sb.as_slice(), soff),
            (Buf::Bytes { .. }, _) => unreachable!("mixed storage kinds"),
        };
        wide::resolve_into(b, off, nsrc, planes, n);
    }

    /// Elements as one byte each: the bytes themselves, or logic codes (a transpose, used only
    /// for user resolution functions over logic).
    pub fn codes(&self, off: usize, n: usize) -> Vec<u8> {
        match self {
            Buf::Bytes { .. } => self.bytes(off, n).map(<[u8]>::to_vec).unwrap_or_default(),
            Buf::Planes { b, .. } => (0..n)
                .map(|i| {
                    let blk = get_block(b, off + i, 1);
                    (0..4).fold(0u8, |c, k| c | ((blk[k] & 1) as u8) << k)
                })
                .collect(),
        }
    }

    /// Inverse of `codes` for logic storage.
    pub fn set_codes(&mut self, off: usize, codes: &[u8]) {
        match self {
            Buf::Bytes { es, data } => {
                data[off * *es..off * *es + codes.len()].copy_from_slice(codes)
            }
            Buf::Planes { b, .. } => {
                for (i, &c) in codes.iter().enumerate() {
                    let blk = std::array::from_fn(|k| u64::from(c >> k & 1));
                    put_block(b, off + i, 1, blk, 1);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use yavs_logic::StdULogic;

    fn lv(s: &str) -> LogicVec {
        s.parse().unwrap()
    }

    #[test]
    fn unaligned_planes_copy_and_diff() {
        let src: LogicVec = LogicVec::from_fn(200, |i| {
            if i % 3 == 0 {
                StdULogic::One
            } else {
                StdULogic::Z
            }
        });
        let src = Buf::from_ref(Elem::Logic, 200, (&src).into()).unwrap();
        let mut dst = Buf::zeroed(Elem::Logic, 300);
        dst.copy(37, &src, 5, 150);
        let Value::Logic(out) = dst.extract(37, 150).to_value() else {
            panic!()
        };
        let Value::Logic(want) = src.extract(5, 150).to_value() else {
            panic!()
        };
        assert_eq!(out, want);
        assert_eq!(dst.diff(37, &src, 5, 150), vec![0, 0, 0]);
        let d = dst.diff(36, &src, 5, 150);
        assert_ne!(d, vec![0, 0, 0]);
        // Elements outside the copied range stay U.
        assert_eq!(dst.codes(36, 1), [0]);
        assert_eq!(dst.codes(187, 1), [0]);
    }

    #[test]
    fn masked_select_and_resolution() {
        let a = Buf::from_ref(Elem::Logic, 4, (&lv("01ZZ")).into()).unwrap();
        let b = Buf::from_ref(Elem::Logic, 4, (&lv("1Z0Z")).into()).unwrap();
        let mut r = Buf::zeroed(Elem::Logic, 4);
        r.resolve_std_logic_by(0, 4, 2, |i| [(&a, 0), (&b, 0)][i]);
        assert_eq!(r.to_value(), Value::Logic(lv("X10Z")));
        r.resolve_std_logic_by(0, 4, 0, |_| (&a, 0));
        assert_eq!(r.to_value(), Value::Logic(lv("ZZZZ")));
        let mut m = Buf::zeroed(Elem::Logic, 4);
        m.select(0, &a, 0, 4, &[0b0101], Some(0));
        assert_eq!(m.to_value(), Value::Logic(lv("U1UZ")));
        let codes = m.codes(0, 4);
        assert_eq!(codes, [StdULogic::Z as u8, 0, StdULogic::One as u8, 0]);
        let mut back = Buf::zeroed(Elem::Logic, 4);
        back.set_codes(0, &codes);
        assert_eq!(back, m);
    }

    #[test]
    fn write_to_matches_extract() {
        let src = LogicVec::from_fn(150, |i| StdULogic::from_code_pub((i % 9) as u8));
        let b = Buf::from_ref(Elem::Logic, 150, (&src).into()).unwrap();
        let mut out = [0u8; 64];
        assert!(b.write_to(70, 70, &mut out));
        let Buf::Planes { b: want, .. } = b.extract(70, 70) else {
            panic!()
        };
        let words: Vec<u64> = want.iter().flatten().copied().collect();
        let got: Vec<u64> = out
            .chunks(8)
            .map(|c| u64::from_le_bytes(c.try_into().unwrap()))
            .collect();
        assert_eq!(got, words);
        assert!(!b.write_to(70, 70, &mut [0u8; 32]));
        let bytes = Buf::from_ref(Elem::Bytes(2), 3, (&[1u8, 2, 3, 4, 5, 6]).into()).unwrap();
        let mut out = [0u8; 4];
        assert!(bytes.write_to(1, 2, &mut out));
        assert_eq!(out, [3, 4, 5, 6]);
    }

    #[test]
    fn refill_reuses_or_replaces() {
        let mut b = Buf::from_ref(Elem::Logic, 2, (&lv("01")).into()).unwrap();
        b.refill(Elem::Logic, 2, (&lv("ZX")).into()).unwrap();
        assert_eq!(b.to_value(), Value::Logic(lv("ZX")));
        b.refill(Elem::Logic, 3, (&lv("1HL")).into()).unwrap();
        assert_eq!(b.to_value(), Value::Logic(lv("1HL")));
        assert!(b.refill(Elem::Logic, 2, (&lv("1HL")).into()).is_err());
        let mut b = Buf::from_ref(Elem::Bytes(2), 1, (&[1u8, 2]).into()).unwrap();
        b.refill(Elem::Bytes(1), 2, (&[7u8, 8]).into()).unwrap();
        assert_eq!(b.to_value(), Value::Bytes(vec![7, 8]));
        assert_eq!(b.len(), 2);
    }

    #[test]
    fn kind_and_size_checks() {
        assert!(Buf::from_ref(Elem::Bytes(2), 2, (&[1u8, 2, 3]).into()).is_err());
        assert!(Buf::from_ref(Elem::Logic, 2, (&[1u8, 2]).into()).is_err());
        assert!(Buf::from_ref(Elem::Bytes(1), 0, (&[0u8; 0]).into()).is_ok());
    }
}
