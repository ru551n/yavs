//! The leaf table of `Inst::SigGather` and `Inst::SigScatter` (#677): a constant that names the
//! record type of a record signal and lists its leaves in leaf order. Table: `u32` count, `u32`
//! record type, then 8 bytes a leaf, little endian: `sig` (instance signal), `kind`, `bias`, `sw`
//! (bytes of a scalar in the value), `ew` (bytes of a kernel element).
//!
//! Where a leaf lies in the record value, and how many elements it has, follows from the type
//! and the descriptor of the signal ([`plan`]): the instructions take the descriptor, so a record
//! whose fields have their bounds from generics or constants of the design works like one with
//! static fields. The table does not say how the kernel keeps the leaves, so a different storage
//! of record signals can serve the same instructions.

use crate::{ConstData, ConstId, IdVec, Layout, LayoutError, Ty, TyId};

/// Bytes of one leaf in the table, and of the header.
pub const LEAF_BYTES: usize = 8;
pub const HEADER_BYTES: usize = 8;

/// How a leaf is stored in the record value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LeafKind {
    /// One `std_ulogic`, BIT or BOOLEAN: its position (`code - bias`) in `sw` bytes.
    LogicScalar = 0,
    /// A logic array (of any nesting): plane blocks, `ceil(len / 64) * 32` bytes.
    LogicVec = 1,
    /// One scalar of a byte signal: `ew` bytes in the kernel, `sw` in the value (a narrow
    /// integer is sign extended).
    Scalar = 2,
    /// An array of scalars of a byte signal: `len * ew` bytes, as the kernel keeps them.
    Bytes = 3,
}

/// A leaf as the table lists it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TabLeaf {
    pub sig: u32,
    pub kind: LeafKind,
    pub bias: u8,
    pub sw: u8,
    pub ew: u8,
}

/// A leaf placed in a record value ([`plan`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Leaf {
    pub sig: u32,
    /// Kernel elements.
    pub len: u32,
    /// Byte offset and size in the value.
    pub voff: u32,
    pub vsize: u32,
    pub kind: LeafKind,
    pub bias: u8,
    pub sw: u8,
    pub ew: u8,
}

/// The leaves of a record value and the size of the value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub size: u64,
    pub leaves: Vec<Leaf>,
}

/// Why no plan: the layout failed, or the table does not fit the type or descriptor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PlanError {
    Layout(LayoutError),
    Bad(String),
}

impl From<LayoutError> for PlanError {
    fn from(e: LayoutError) -> PlanError {
        PlanError::Layout(e)
    }
}

fn bad<T>(m: impl Into<String>) -> Result<T, PlanError> {
    Err(PlanError::Bad(m.into()))
}

type Types = IdVec<TyId, Ty>;
type Consts = IdVec<ConstId, ConstData>;

/// The bytes a leaf with these fields takes in the value.
pub fn size_of(kind: LeafKind, len: u32, sw: u8, ew: u8) -> u64 {
    match kind {
        LeafKind::LogicScalar | LeafKind::Scalar => u64::from(sw),
        LeafKind::LogicVec => u64::from(len).div_ceil(64) * 32,
        LeafKind::Bytes => u64::from(len) * u64::from(ew),
    }
}

/// The table for the leaves of record type `ty`, as a constant.
pub fn encode(ty: TyId, leaves: &[TabLeaf]) -> ConstData {
    let mut b = Vec::with_capacity(HEADER_BYTES + LEAF_BYTES * leaves.len());
    b.extend_from_slice(&(leaves.len() as u32).to_le_bytes());
    b.extend_from_slice(&ty.0.to_le_bytes());
    for l in leaves {
        b.extend_from_slice(&l.sig.to_le_bytes());
        b.extend_from_slice(&[l.kind as u8, l.bias, l.sw, l.ew]);
    }
    ConstData::Bytes(b)
}

/// The record type and the leaves of a table; the second half of a leaf is false for a kind byte
/// that is none. No other check: see [`valid`].
pub fn entries(b: &[u8]) -> Option<(TyId, impl Iterator<Item = (TabLeaf, bool)> + '_)> {
    let n = u32::from_le_bytes(b.get(..4)?.try_into().ok()?) as usize;
    let ty = u32::from_le_bytes(b.get(4..8)?.try_into().ok()?);
    let body = b.get(HEADER_BYTES..)?;
    if body.len() != n.checked_mul(LEAF_BYTES)? {
        return None;
    }
    let leaf = |c: &[u8; LEAF_BYTES]| {
        let (kind, ok) = match c[4] {
            0 => (LeafKind::LogicScalar, true),
            1 => (LeafKind::LogicVec, true),
            2 => (LeafKind::Scalar, true),
            3 => (LeafKind::Bytes, true),
            _ => (LeafKind::Bytes, false),
        };
        let l = TabLeaf {
            sig: u32::from_le_bytes([c[0], c[1], c[2], c[3]]),
            kind,
            bias: c[5],
            sw: c[6],
            ew: c[7],
        };
        (l, ok)
    };
    Some((TyId(ty), body.as_chunks::<LEAF_BYTES>().0.iter().map(leaf)))
}

/// The number of leaves record type `ty` has for a table, `None` when it has a shape the
/// instructions do not take: fields that are records, scalars, or arrays of scalars (logic
/// arrays of any nesting) only.
pub fn shape(types: &Types, ty: TyId) -> Option<usize> {
    let Ty::Record { fields } = types.get(ty)? else {
        return None;
    };
    let mut n = 0;
    for f in fields {
        n += match types.get(f.ty)? {
            Ty::Record { .. } => shape(types, f.ty)?,
            t if leaf_array(types, f.ty, t) || t.scalar_size().is_some() => 1,
            _ => return None,
        };
    }
    Some(n)
}

/// An array of scalars, or of logic arrays (one plane vector over all elements).
fn leaf_array(types: &Types, ty: TyId, t: &Ty) -> bool {
    match t {
        Ty::Array { elem, .. } => {
            types.get(*elem).is_some_and(|e| e.scalar_size().is_some())
                || Ty::logic_leaf(types, ty).is_some()
        }
        _ => false,
    }
}

/// Whether `b` is a table for a type of `types`: well formed, of a known record shape with that
/// many leaves, each of a known kind with widths of at most 8 bytes (the verifier's check; the
/// interpreter relies on it).
pub fn valid(types: &Types, b: &[u8]) -> bool {
    let Some((ty, mut it)) = entries(b) else {
        return false;
    };
    let n = b.len().saturating_sub(HEADER_BYTES) / LEAF_BYTES;
    shape(types, ty) == Some(n) && it.all(|(l, ok)| ok && l.sw <= 8 && l.ew <= 8)
}

/// The places of the leaves of table `tab` in a value of its record type with descriptor `desc`.
pub fn plan(types: &Types, consts: &Consts, tab: &[u8], desc: &[u8]) -> Result<Plan, PlanError> {
    let Some((ty, it)) = entries(tab) else {
        return bad("a leaf table that is not one");
    };
    let mut places = Vec::new();
    walk(types, consts, ty, desc, 0, &mut places)?;
    let (lay, _): (Layout, usize) = Ty::layout(types, consts, ty, desc)?;
    let mut leaves = Vec::with_capacity(places.len());
    for ((t, ok), (voff, vsize, count)) in it.zip(places.iter().copied()) {
        let (Ok(len), Ok(voff), Ok(vs)) = (
            u32::try_from(count),
            u32::try_from(voff),
            u32::try_from(vsize),
        ) else {
            return Err(LayoutError::TooLarge.into());
        };
        if !ok || t.sw > 8 || t.ew > 8 || u64::from(vs) != size_of(t.kind, len, t.sw, t.ew) {
            return bad("a leaf that does not fit its place");
        }
        leaves.push(Leaf {
            sig: t.sig,
            len,
            voff,
            vsize: vs,
            kind: t.kind,
            bias: t.bias,
            sw: t.sw,
            ew: t.ew,
        });
    }
    if leaves.len() != places.len() {
        return bad("a leaf table that does not match its type");
    }
    Ok(Plan {
        size: lay.size,
        leaves,
    })
}

/// The `(offset, size, element count)` of every leaf of record type `ty` (descriptor `desc`,
/// value at `base`), in leaf order.
fn walk(
    types: &Types,
    consts: &Consts,
    ty: TyId,
    desc: &[u8],
    base: u64,
    out: &mut Vec<(u64, u64, u64)>,
) -> Result<(), PlanError> {
    let Some(Ty::Record { fields }) = types.get(ty) else {
        return bad("a leaf table of a type that is no record");
    };
    let (lay, _) = Ty::layout(types, consts, ty, desc)?;
    let mut used = 0usize;
    for (i, f) in fields.iter().enumerate() {
        let fd: &[u8] = match f.desc {
            Some(c) => match consts.get(c) {
                Some(ConstData::Bytes(b)) => b,
                _ => return bad("a field descriptor that is not plain bytes"),
            },
            None => desc.get(used..).unwrap_or(&[]),
        };
        let (fl, fused) = Ty::layout(types, consts, f.ty, fd)?;
        if f.desc.is_none() {
            used += fused;
        }
        let Some(&off) = lay.offsets.get(i) else {
            return bad("a record layout without a field");
        };
        let voff = base + off;
        match types.get(f.ty) {
            Some(Ty::Record { .. }) => walk(types, consts, f.ty, fd, voff, out)?,
            Some(t) if leaf_array(types, f.ty, t) || t.scalar_size().is_some() => {
                out.push((voff, fl.size, fl.count))
            }
            _ => return bad("a leaf that is neither a scalar nor an array of scalars"),
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip() {
        let ls = [
            TabLeaf {
                sig: 3,
                kind: LeafKind::LogicScalar,
                bias: 2,
                sw: 1,
                ew: 1,
            },
            TabLeaf {
                sig: 4,
                kind: LeafKind::Bytes,
                bias: 0,
                sw: 0,
                ew: 4,
            },
        ];
        let ConstData::Bytes(b) = encode(TyId(7), &ls) else {
            panic!("bytes")
        };
        let (ty, it) = entries(&b).unwrap();
        assert_eq!(ty, TyId(7));
        assert_eq!(it.map(|e| e.0).collect::<Vec<_>>(), ls);
        assert!(entries(&b[..b.len() - 1]).is_none());
        assert_eq!(size_of(LeafKind::LogicVec, 70, 0, 1), 64);
        assert_eq!(size_of(LeafKind::Bytes, 3, 0, 4), 12);
    }
}
