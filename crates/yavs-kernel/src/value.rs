//! Signal value storage.
//!
//! A signal holds either plain bytes (integers, enums, reals, record fields: `elem_size` bytes
//! per element) or IEEE 1164 logic in yavs-logic's bit-plane layout (four planes of 64
//! elements per block). Element-addressed operations work on 64-element chunks with shifts and
//! masks, so a sub-range copy, compare or resolution never decodes single elements.

use std::fmt;

use smallvec::{SmallVec, smallvec};
use yavs_logic::{Block, Logic, Planes};

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

/// Every element `Z` (code 4: plane 2 only).
const Z_BLOCK: Block = [0, 0, !0, 0];

/// Storage for a run of elements of one signal.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Buf {
    Bytes { es: usize, data: SmallVec<[u8; 16]> },
    Planes { len: usize, b: SmallVec<[Block; 1]> },
}

/// The low `n` bits (`n <= 64`).
fn nmask(n: usize) -> u64 {
    if n >= 64 { !0 } else { (1u64 << n) - 1 }
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

/// Set or clear bits `off .. off + n` of a bitset.
pub(crate) fn fill_bits(words: &mut [u64], off: usize, n: usize, on: bool) {
    for i in off..off + n {
        if on {
            words[i / 64] |= 1 << (i % 64);
        } else {
            words[i / 64] &= !(1 << (i % 64));
        }
    }
}

fn get_block(b: &[Block], off: usize, n: usize) -> Block {
    let (w, s) = (off / 64, off % 64);
    let m = nmask(n);
    std::array::from_fn(|k| {
        let mut v = b[w][k] >> s;
        if s != 0 && w + 1 < b.len() {
            v |= b[w + 1][k] << (64 - s);
        }
        v & m
    })
}

/// Write the elements of `v` selected by `sel` (low `n` bits) at `off`.
fn put_block(b: &mut [Block], off: usize, n: usize, v: Block, sel: u64) {
    let m = sel & nmask(n);
    let (w, s) = (off / 64, off % 64);
    for k in 0..4 {
        b[w][k] = (b[w][k] & !(m << s)) | ((v[k] & m) << s);
        if s != 0 && s + n > 64 {
            let m2 = m >> (64 - s);
            b[w + 1][k] = (b[w + 1][k] & !m2) | ((v[k] >> (64 - s)) & m2);
        }
    }
}

/// Chunks of at most 64 elements covering `0..n`: `(start, len)`.
fn chunks(n: usize) -> impl Iterator<Item = (usize, usize)> {
    (0..n).step_by(64).map(move |c| (c, (n - c).min(64)))
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

    /// Element `i` as one byte: its first byte, or its logic code.
    pub fn code(&self, i: usize) -> u8 {
        match self {
            Buf::Bytes { es, data } => data[i * es],
            Buf::Planes { b, .. } => {
                let (blk, s) = (&b[i / 64], i % 64);
                (0..4).fold(0u8, |c, k| c | ((blk[k] >> s & 1) as u8) << k)
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
                for (c, k) in chunks(n) {
                    out[c / 64] = get_block(b, off + c, k);
                }
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
                d.len() == n * es && data.get(off * es..(off + n) * es) == Some(d)
            }
            (Buf::Planes { len, b }, ValueRef::Logic(l)) => {
                l.len() == n
                    && off + n <= *len
                    && chunks(n).all(|(c, k)| {
                        get_block(b, off + c, k) == get_block(l.repr().blocks(), c, k)
                    })
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
            (Buf::Planes { b, .. }, Buf::Planes { b: ob, .. }) => {
                let (x, y) = (get_block(b, off, k), get_block(ob, ooff, k));
                (0..4).fold(0, |a, p| a | (x[p] ^ y[p]))
            }
            _ => unreachable!("storage kinds are checked when values enter the kernel"),
        }
    }

    /// IEEE 1164 resolution of `sources` (each with its element offset) into `off..off + n`.
    /// No sources gives `Z`, as the `resolved` function does for an empty array.
    pub fn resolve_std_logic(&mut self, off: usize, n: usize, sources: &[(&Buf, usize)]) {
        let Buf::Planes { b, .. } = self else {
            unreachable!("std_logic resolution on byte storage")
        };
        for (c, k) in chunks(n) {
            let acc = sources
                .iter()
                .map(|(s, soff)| match s {
                    Buf::Planes { b: sb, .. } => get_block(sb, soff + c, k),
                    Buf::Bytes { .. } => unreachable!("mixed storage kinds"),
                })
                .reduce(Planes::resolve_block)
                .unwrap_or(Z_BLOCK);
            put_block(b, off + c, k, acc, !0);
        }
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
        r.resolve_std_logic(0, 4, &[(&a, 0), (&b, 0)]);
        assert_eq!(r.to_value(), Value::Logic(lv("X10Z")));
        r.resolve_std_logic(0, 4, &[]);
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
