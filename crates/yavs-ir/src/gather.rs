//! The leaf table of `Inst::SigGather` and `Inst::SigScatter` (#677): a constant that names the
//! record type of a record signal and lists its leaves in leaf order. Table: `u32` count, `u32`
//! record type, then 8 bytes a leaf, little endian: `sig` (instance signal), `kind`, `bias`, `sw`
//! (bytes of a scalar in the value), `ew` (bytes of a kernel element).
//!
//! The type is that of the value: a record, or an array (any nesting) of records; a record may
//! have arrays of records as fields. A leaf is a field of the innermost records, and stands for
//! that field of every instance of them, instances in row-major order (the rule of the kernel's
//! sub-element map: logic leaves are mirrored over all instances).
//!
//! Where a leaf lies in the value, how many elements one instance has and where each instance
//! lies, follows from the type and the descriptor of the signal ([`plan`]): the instructions
//! take the descriptor, so a record whose fields have their bounds from generics or constants of
//! the design works like one with static fields. The table does not say how the kernel keeps
//! the leaves, so a different storage of record signals can serve the same instructions.

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

/// A leaf placed in a value ([`plan`]).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Leaf {
    pub sig: u32,
    /// Kernel elements of one instance.
    pub len: u32,
    /// Byte offset in the value of the first instance and size of one.
    pub voff: u32,
    pub vsize: u32,
    /// Instances of the leaf in the value: those of every array of records on the way to it.
    pub insts: u32,
    /// Instances for each element of the outermost array of the value.
    pub inner: u32,
    /// The byte offset of every instance when there are several, else empty.
    pub offs: Box<[u32]>,
    pub kind: LeafKind,
    pub bias: u8,
    pub sw: u8,
    pub ew: u8,
}

impl Leaf {
    /// The byte offset in the value of instance `j`.
    #[inline(always)]
    pub fn off(&self, j: usize) -> usize {
        match self.offs.get(j) {
            Some(&o) => o as usize,
            None => self.voff as usize,
        }
    }

    /// Whether the leaf is a logic leaf, which the kernel keeps mirrored over its instances.
    #[inline(always)]
    pub fn logic(&self) -> bool {
        matches!(self.kind, LeafKind::LogicScalar | LeafKind::LogicVec)
    }

    /// The first kernel element, counted from the start of the leaf's signal, of instance `j` of
    /// the part that starts at element `first` of an outermost array of `outer` elements (a
    /// whole value: `first` 0, `outer` its element count).
    #[inline(always)]
    pub fn kstart(&self, j: usize, first: u64, outer: u64) -> u64 {
        let inner = u64::from(self.inner);
        let (g, t) = (first * inner + j as u64, outer * inner);
        (if self.logic() { t - 1 - g } else { g }) * u64::from(self.len)
    }
}

/// The leaves of a value and the size of the value.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plan {
    pub size: u64,
    /// Elements of the outermost array of the value, all dimensions and nestings: 1 for a record.
    pub count: u64,
    /// Every leaf has one instance: no array of records anywhere in the value.
    pub flat: bool,
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

/// The record type at the bottom of arrays of arrays of records `ty`; `ty` itself for a record.
fn record_of(types: &Types, ty: TyId) -> Option<TyId> {
    let mut t = ty;
    for _ in 0..64 {
        match types.get(t)? {
            Ty::Record { .. } => return Some(t),
            Ty::Array { elem, .. } => t = *elem,
            _ => return None,
        }
    }
    None
}

/// The number of leaves value type `ty` has for a table, `None` when it has a shape the
/// instructions do not take: records and arrays (any nesting) of records, whose fields are such
/// types, scalars, or arrays of scalars (logic arrays of any nesting).
pub fn shape(types: &Types, ty: TyId) -> Option<usize> {
    let Ty::Record { fields } = types.get(record_of(types, ty)?)? else {
        return None;
    };
    let mut n = 0;
    for f in fields {
        n += match types.get(f.ty)? {
            t if leaf_array(types, f.ty, t) || t.scalar_size().is_some() => 1,
            _ => shape(types, f.ty)?,
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

/// The places of the leaves of table `tab` in a value of its type with descriptor `desc`.
pub fn plan(types: &Types, consts: &Consts, tab: &[u8], desc: &[u8]) -> Result<Plan, PlanError> {
    let Some((ty, it)) = entries(tab) else {
        return bad("a leaf table that is not one");
    };
    let mut places = Vec::new();
    walk(types, consts, ty, desc, &[0], &mut places)?;
    let (lay, _): (Layout, usize) = Ty::layout(types, consts, ty, desc)?;
    let count = top_count(types, consts, ty, desc)?;
    let mut leaves = Vec::with_capacity(places.len());
    for ((t, ok), (offs, vsize, len)) in it.zip(places) {
        let (Ok(len), Ok(vs)) = (u32::try_from(len), u32::try_from(vsize)) else {
            return Err(LayoutError::TooLarge.into());
        };
        let offs: Result<Vec<u32>, _> = offs.iter().map(|&o| u32::try_from(o)).collect();
        let Ok(offs) = offs else {
            return Err(LayoutError::TooLarge.into());
        };
        // (No instance at all: a null array of records.)
        let voff = offs.first().copied().unwrap_or(0);
        let insts = offs.len() as u32;
        if !ok
            || t.sw > 8
            || t.ew > 8
            || u64::from(vs) != size_of(t.kind, len, t.sw, t.ew)
            || (count != 0 && u64::from(insts) % count != 0)
            || (count == 0 && insts != 0)
        {
            return bad("a leaf that does not fit its place");
        }
        leaves.push(Leaf {
            sig: t.sig,
            len,
            voff,
            vsize: vs,
            insts,
            inner: u64::from(insts).checked_div(count).unwrap_or(0) as u32,
            offs: if insts > 1 { offs.into() } else { Box::new([]) },
            kind: t.kind,
            bias: t.bias,
            sw: t.sw,
            ew: t.ew,
        });
    }
    Ok(Plan {
        size: lay.size,
        count,
        flat: count == 1 && leaves.iter().all(|l| l.insts == 1),
        leaves,
    })
}

/// Elements of the arrays on the way to the records of a value of type `ty`, 1 for a record.
fn top_count(types: &Types, consts: &Consts, ty: TyId, desc: &[u8]) -> Result<u64, PlanError> {
    let Some(Ty::Array { .. }) = types.get(ty) else {
        return Ok(1);
    };
    let (lay, _) = Ty::layout(types, consts, ty, desc)?;
    let (elem, ed) = array_elem(types, consts, ty, desc)?;
    lay.count
        .checked_mul(top_count(types, consts, elem, &ed)?)
        .ok_or_else(|| LayoutError::TooLarge.into())
}

/// The element type of array type `ty` and the descriptor of its elements.
fn array_elem<'a>(
    types: &Types,
    consts: &'a Consts,
    ty: TyId,
    desc: &'a [u8],
) -> Result<(TyId, std::borrow::Cow<'a, [u8]>), PlanError> {
    let Some(Ty::Array {
        elem,
        elem_desc,
        dims,
    }) = types.get(ty)
    else {
        return bad("an array type that is none");
    };
    let ed = match elem_desc {
        Some(c) => match consts.get(*c) {
            Some(ConstData::Bytes(b)) => std::borrow::Cow::Borrowed(b.as_slice()),
            _ => return bad("an element descriptor that is not plain bytes"),
        },
        None => std::borrow::Cow::Borrowed(
            desc.get(*dims as usize * crate::DOPE_DIM_BYTES as usize..)
                .unwrap_or(&[]),
        ),
    };
    Ok((*elem, ed))
}

/// The `(offset of every instance, size of one, elements of one)` of every leaf below value type
/// `ty` (descriptor `desc`), in leaf order; `bases` are the offsets of the instances of `ty`.
fn walk(
    types: &Types,
    consts: &Consts,
    ty: TyId,
    desc: &[u8],
    bases: &[u64],
    out: &mut Vec<(Vec<u64>, u64, u64)>,
) -> Result<(), PlanError> {
    let fields = match types.get(ty) {
        Some(Ty::Record { fields }) => fields,
        Some(Ty::Array { .. }) => {
            let (lay, _) = Ty::layout(types, consts, ty, desc)?;
            let (elem, ed) = array_elem(types, consts, ty, desc)?;
            let mut inner = Vec::with_capacity(bases.len().saturating_mul(lay.count as usize));
            for b in bases {
                for e in 0..lay.count {
                    inner.push(b + e * lay.stride);
                }
            }
            return walk(types, consts, elem, &ed, &inner, out);
        }
        _ => return bad("a leaf table of a type that is neither record nor array"),
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
        let at: Vec<u64> = bases.iter().map(|b| b + off).collect();
        match types.get(f.ty) {
            Some(t) if leaf_array(types, f.ty, t) || t.scalar_size().is_some() => {
                out.push((at, fl.size, fl.count))
            }
            Some(Ty::Record { .. } | Ty::Array { .. }) => walk(types, consts, f.ty, fd, &at, out)?,
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

    fn dope(dims: &[(i64, i64, i64)]) -> Vec<u8> {
        dims.iter()
            .flat_map(|&(l, r, d)| [l, r, d])
            .flat_map(i64::to_le_bytes)
            .collect()
    }

    fn tl(sig: u32, kind: LeafKind, sw: u8, ew: u8) -> TabLeaf {
        TabLeaf {
            sig,
            kind,
            bias: 0,
            sw,
            ew,
        }
    }

    /// An array of three records, and a record with an array of two of them as a field: the
    /// places of the instances and the kernel start of each (logic leaves mirrored).
    #[test]
    fn arrays_of_records() {
        use crate::{Field, LogicKind};
        let mut types = Types::new();
        let mut consts = Consts::new();
        let sl = types.push(Ty::Enum {
            lits: "UX01ZWLH-".chars().map(|c| format!("'{c}'")).collect(),
            logic: Some(LogicKind::StdULogic),
        });
        let int = types.push(Ty::Int {
            lo: i32::MIN.into(),
            hi: i32::MAX.into(),
        });
        let slv = types.push(Ty::Array {
            elem: sl,
            elem_desc: None,
            dims: 1,
        });
        let byte = consts.push(ConstData::Bytes(dope(&[(7, 0, 1)])));
        let f = |name: &str, ty, desc| Field {
            name: name.into(),
            ty,
            desc,
        };
        let rec = types.push(Ty::Record {
            fields: vec![f("a", sl, None), f("d", slv, Some(byte)), f("n", int, None)],
        });
        let arr = types.push(Ty::Array {
            elem: rec,
            elem_desc: None,
            dims: 1,
        });
        let top = types.push(Ty::Record {
            fields: vec![f("x", sl, None), f("v", arr, None)],
        });
        assert_eq!(shape(&types, arr), Some(3));
        assert_eq!(shape(&types, top), Some(4));
        let rl = [
            tl(1, LeafKind::LogicScalar, 1, 1),
            tl(2, LeafKind::LogicVec, 1, 1),
            tl(3, LeafKind::Scalar, 8, 4),
        ];
        let ConstData::Bytes(t) = encode(arr, &rl) else {
            panic!("bytes")
        };
        assert!(valid(&types, &t));
        let p = plan(&types, &consts, &t, &dope(&[(0, 2, 0)])).unwrap();
        assert_eq!((p.size, p.count), (3 * 48, 3));
        assert_eq!(&*p.leaves[0].offs, &[0, 48, 96]);
        assert_eq!(&*p.leaves[1].offs, &[8, 56, 104]);
        assert_eq!(&*p.leaves[2].offs, &[40, 88, 136]);
        // The whole array, then elements 1 and 2 of an array of 4 (first 1, outer 4): logic leaves
        // mirrored over the instances, others in order.
        let a = &p.leaves[0];
        assert_eq!((a.kstart(0, 0, 3), a.kstart(2, 0, 3)), (2, 0));
        let n = &p.leaves[2];
        assert_eq!((n.kstart(0, 0, 3), n.kstart(2, 0, 3)), (0, 2));
        assert_eq!((a.kstart(0, 1, 4), a.kstart(1, 1, 4)), (2, 1));
        assert_eq!(p.leaves[1].kstart(1, 0, 3), 8);
        // The record with the array as a field: one instance of `x`, two of the others.
        let tl2 = [
            tl(0, LeafKind::LogicScalar, 1, 1),
            tl(1, LeafKind::LogicScalar, 1, 1),
            tl(2, LeafKind::LogicVec, 1, 1),
            tl(3, LeafKind::Scalar, 8, 4),
        ];
        let ConstData::Bytes(t) = encode(top, &tl2) else {
            panic!("bytes")
        };
        let p = plan(&types, &consts, &t, &dope(&[(1, 2, 0)])).unwrap();
        assert_eq!(p.count, 1);
        assert_eq!(p.leaves[0].insts, 1);
        assert_eq!((p.leaves[1].insts, p.leaves[1].inner), (2, 2));
        assert_eq!(p.leaves[1].kstart(0, 0, 1), 1);
        // A table whose leaf has the wrong size for its place is refused.
        let mut bad_leaves = rl;
        bad_leaves[2].sw = 4;
        let ConstData::Bytes(t) = encode(arr, &bad_leaves) else {
            panic!("bytes")
        };
        assert!(plan(&types, &consts, &t, &dope(&[(0, 2, 0)])).is_err());
    }
}
