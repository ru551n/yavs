//! Runtime type descriptions and value layout (ir.md 1.3).
//!
//! One description per VHDL type (rule 1). A composite subtype is a runtime value, its
//! **descriptor**: bytes in memory, built by instructions or taken from the constant pool when
//! static. There is no second, static-only path: [`Ty::layout`] computes the same layout from
//! descriptor bytes whether they came from the pool or were built at run time.
//!
//! Descriptor format:
//! - array: per dimension `[left: i64, right: i64, dir: i64]` (dir 0 = `to`, 1 = `downto`),
//!   followed by the element descriptor when the element type needs one and the array type
//!   gives none (`elem_desc`);
//! - record: the descriptors of the fields without a static constraint, in field order;
//! - scalars, access, file, protected: empty.
//!
//! Value layout: integers, physical, reals, access handles and protected handles are 8 bytes;
//! enums are 1 byte (up to 256 literals) or 4; files are 4. An array element of an integer type
//! whose range fits in 32 bits is 4 bytes ([`Ty::elem_size`], docs/decisions.md "Narrow integer
//! array elements"); 4-byte cells are sign-extended on load. Logic arrays (whose scalar leaf is
//! a [`LogicKind`] enum) are yavs-logic bit-planes: `ceil(n / 64) * 32` bytes for `n` leaf
//! elements, nested logic arrays being one plane vector over all elements. Other arrays are
//! row major with naturally aligned elements. Record fields are naturally aligned; a
//! logic-array field is its own plane vector. Sizes are `u64` (rule 6).

use std::fmt;

use crate::{ConstData, ConstId, IdVec, ReflId, TyId};

/// Bytes of one dimension in an array descriptor.
pub const DOPE_DIM_BYTES: u64 = 24;

#[derive(serde::Serialize, serde::Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum LogicKind {
    StdULogic,
    Bit,
    Boolean,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
pub struct Field {
    pub name: String,
    pub ty: TyId,
    /// Static constraint of the field (a descriptor in the pool), `None` when the field's
    /// constraint comes from the record's descriptor or the type needs none.
    pub desc: Option<ConstId>,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
pub enum Ty {
    /// Integer type: `i64` in registers and as a scalar in memory; 4 bytes as an array
    /// element when the range fits in 32 bits ([`Ty::elem_size`]).
    Int {
        lo: i64,
        hi: i64,
    },
    /// Physical type: `i64` in primary units; `units` lists `(name, primary units)`, primary
    /// first, for `'image`.
    Physical {
        lo: i64,
        hi: i64,
        units: Vec<(String, i64)>,
    },
    /// Enumeration type with its literals in original spelling (`'image` needs them);
    /// character literals are written with their quotes (`'a'`).
    Enum {
        lits: Vec<String>,
        logic: Option<LogicKind>,
    },
    Real,
    Array {
        elem: TyId,
        /// Static element constraint, when the element is constrained by the type.
        elem_desc: Option<ConstId>,
        dims: u8,
    },
    Record {
        fields: Vec<Field>,
    },
    Access {
        target: TyId,
    },
    File {
        elem: TyId,
    },
    /// A protected type; `state` is the record type of its private variables.
    Protected {
        state: TyId,
    },
}

/// A subtype as `std.reflection` sees it (LRM 2019 16.11): what a mirror needs beyond the
/// value's [`Ty`]. Constraints of composites come from the descriptor at run time; scalar
/// ranges are static. `name` is the subtype's simple name, upper case (as sim1 reports it).
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
pub struct Refl {
    pub name: String,
    pub kind: ReflKind,
}

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
pub enum ReflKind {
    Enum {
        ty: TyId,
        left: i64,
        right: i64,
        downto: bool,
    },
    Int {
        left: i64,
        right: i64,
        downto: bool,
    },
    Float {
        left: f64,
        right: f64,
        downto: bool,
    },
    /// `time`: values are reported in units of `std.env.resolution_limit` (docs/decisions.md).
    Physical {
        ty: TyId,
        left: i64,
        right: i64,
        downto: bool,
        time: bool,
    },
    Array {
        ty: TyId,
        index: Vec<ReflId>,
        elem: ReflId,
    },
    /// Element names upper case, in declaration order.
    Record {
        ty: TyId,
        elems: Vec<(String, ReflId)>,
    },
    Access {
        designated: ReflId,
    },
    File {
        designated: ReflId,
    },
    Protected,
}

/// The layout of a fully constrained value.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct Layout {
    pub size: u64,
    pub align: u64,
    /// Arrays: bytes per element (0 for logic arrays, which are addressed by element index).
    pub stride: u64,
    /// Arrays: element count (all dimensions, logic leaves for logic arrays).
    pub count: u64,
    /// Records: byte offset of each field.
    pub offsets: Vec<u64>,
}

/// Why a layout cannot be computed: a malformed descriptor or type table (an ICE), or a size
/// beyond 64 bits (a runtime error).
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq, Eq)]
pub enum LayoutError {
    Malformed(String),
    TooLarge,
}

impl fmt::Display for LayoutError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LayoutError::Malformed(m) => write!(f, "malformed type descriptor: {m}"),
            LayoutError::TooLarge => f.write_str("object size exceeds 64 bits"),
        }
    }
}

/// Element count of one dimension: `left to right` or `left downto right`, 0 for a null
/// range, `None` when the count does not fit in 64 bits.
pub fn dim_len(left: i64, right: i64, downto: bool) -> Option<u64> {
    let (lo, hi) = if downto { (right, left) } else { (left, right) };
    if hi < lo {
        Some(0)
    } else {
        u64::try_from(i128::from(hi) - i128::from(lo) + 1).ok()
    }
}

fn bad(m: impl Into<String>) -> LayoutError {
    LayoutError::Malformed(m.into())
}

type Types = IdVec<TyId, Ty>;
type Consts = IdVec<ConstId, ConstData>;

impl Ty {
    /// Size of a scalar (or handle) value; `None` for arrays and records.
    pub fn scalar_size(&self) -> Option<u64> {
        Some(match self {
            Ty::Int { .. } | Ty::Physical { .. } | Ty::Real => 8,
            Ty::Access { .. } | Ty::Protected { .. } => 8,
            Ty::Enum { lits, .. } => {
                if lits.len() <= 256 {
                    1
                } else {
                    4
                }
            }
            Ty::File { .. } => 4,
            Ty::Array { .. } | Ty::Record { .. } => return None,
        })
    }

    /// Size of one array element of this scalar type: [`Ty::scalar_size`], except an integer
    /// type whose range fits in 32 bits takes 4 bytes (sign-extended on load); `None` for
    /// arrays and records.
    pub fn elem_size(&self) -> Option<u64> {
        match self {
            Ty::Int { lo, hi } if *lo >= i64::from(i32::MIN) && *hi <= i64::from(i32::MAX) => {
                Some(4)
            }
            t => t.scalar_size(),
        }
    }

    /// Layout of `ty` constrained by `desc`, plus the number of descriptor bytes consumed.
    pub fn layout(
        types: &Types,
        consts: &Consts,
        ty: TyId,
        desc: &[u8],
    ) -> Result<(Layout, usize), LayoutError> {
        layout(types, consts, ty, desc, 0)
    }

    /// The logic kind of the scalar leaf when `ty` is a (possibly nested) logic array.
    pub fn logic_leaf(types: &Types, ty: TyId) -> Option<LogicKind> {
        let mut t = ty;
        for _ in 0..64 {
            match types.get(t)? {
                Ty::Array { elem, .. } => t = *elem,
                Ty::Enum { logic, .. } => return *logic,
                _ => return None,
            }
        }
        None
    }

    /// Bytes of a descriptor of `ty` (static per type); `None` for a missing or cyclic type.
    pub fn desc_size(types: &Types, ty: TyId) -> Option<u64> {
        fn go(types: &Types, ty: TyId, depth: u32) -> Option<u64> {
            if depth > 64 {
                return None;
            }
            Some(match types.get(ty)? {
                Ty::Array {
                    elem,
                    elem_desc,
                    dims,
                } => {
                    let e = if elem_desc.is_none() {
                        go(types, *elem, depth + 1)?
                    } else {
                        0
                    };
                    u64::from(*dims) * DOPE_DIM_BYTES + e
                }
                Ty::Record { fields } => {
                    let mut n = 0;
                    for f in fields.iter().filter(|f| f.desc.is_none()) {
                        n += go(types, f.ty, depth + 1)?;
                    }
                    n
                }
                _ => 0,
            })
        }
        go(types, ty, 0)
    }

    /// Whether values of `ty` can contain access handles (the GC follows them); a missing
    /// or too deeply nested type counts as one.
    pub fn has_handles(types: &Types, ty: TyId) -> bool {
        fn go(types: &Types, ty: TyId, depth: u32) -> bool {
            if depth > 64 {
                return true;
            }
            match types.get(ty) {
                Some(Ty::Access { .. } | Ty::Protected { .. }) => true,
                Some(Ty::Array { elem, .. }) => go(types, *elem, depth + 1),
                Some(Ty::Record { fields }) => fields.iter().any(|f| go(types, f.ty, depth + 1)),
                Some(_) => false,
                None => true,
            }
        }
        go(types, ty, 0)
    }

    /// Whether values of `ty` need a runtime descriptor (arrays, records with such fields).
    pub fn needs_desc(types: &Types, ty: TyId) -> bool {
        match types.get(ty) {
            Some(Ty::Array { .. }) => true,
            Some(Ty::Record { fields }) => fields
                .iter()
                .any(|f| f.desc.is_none() && Ty::needs_desc(types, f.ty)),
            _ => false,
        }
    }
}

fn read_i64(desc: &[u8], at: usize) -> Result<i64, LayoutError> {
    let b = desc
        .get(at..at + 8)
        .ok_or_else(|| bad("descriptor too short"))?;
    let mut a = [0; 8];
    a.copy_from_slice(b);
    Ok(i64::from_le_bytes(a))
}

fn const_bytes(consts: &Consts, c: ConstId) -> Result<&[u8], LayoutError> {
    match consts.get(c) {
        Some(ConstData::Bytes(b)) => Ok(b),
        Some(_) => Err(bad(format!("descriptor constant {c} is not plain bytes"))),
        None => Err(bad(format!("no constant {c}"))),
    }
}

fn layout(
    types: &Types,
    consts: &Consts,
    ty: TyId,
    desc: &[u8],
    depth: u32,
) -> Result<(Layout, usize), LayoutError> {
    if depth > 64 {
        return Err(bad("type nesting deeper than 64"));
    }
    let t = types.get(ty).ok_or_else(|| bad(format!("no type {ty}")))?;
    if let Some(size) = t.scalar_size() {
        let l = Layout {
            size,
            align: size,
            stride: 0,
            count: 1,
            offsets: Vec::new(),
        };
        return Ok((l, 0));
    }
    match t {
        Ty::Array {
            elem,
            elem_desc,
            dims,
        } => {
            let mut count: u64 = 1;
            for d in 0..*dims as usize {
                let at = d * DOPE_DIM_BYTES as usize;
                let (l, r, dir) = (
                    read_i64(desc, at)?,
                    read_i64(desc, at + 8)?,
                    read_i64(desc, at + 16)?,
                );
                if dir != 0 && dir != 1 {
                    return Err(bad(format!("direction {dir} in dimension {d}")));
                }
                count = count
                    .checked_mul(dim_len(l, r, dir == 1).ok_or(LayoutError::TooLarge)?)
                    .ok_or(LayoutError::TooLarge)?;
            }
            let mut used = *dims as usize * DOPE_DIM_BYTES as usize;
            let (el, eused) = match elem_desc {
                Some(c) => layout(types, consts, *elem, const_bytes(consts, *c)?, depth + 1)?,
                None => layout(
                    types,
                    consts,
                    *elem,
                    desc.get(used..).unwrap_or(&[]),
                    depth + 1,
                )?,
            };
            if elem_desc.is_none() {
                used += eused;
            }
            let el = match types.get(*elem).and_then(Ty::elem_size) {
                Some(n) => Layout {
                    size: n,
                    align: n,
                    ..el
                },
                None => el,
            };
            if Ty::logic_leaf(types, ty).is_some() {
                let n = count.checked_mul(el.count).ok_or(LayoutError::TooLarge)?;
                let size = n
                    .div_ceil(64)
                    .checked_mul(32)
                    .ok_or(LayoutError::TooLarge)?;
                let l = Layout {
                    size,
                    align: 8,
                    stride: 0,
                    count: n,
                    offsets: Vec::new(),
                };
                return Ok((l, used));
            }
            let stride = el.size.next_multiple_of(el.align.max(1));
            let size = stride.checked_mul(count).ok_or(LayoutError::TooLarge)?;
            let l = Layout {
                size,
                align: el.align,
                stride,
                count,
                offsets: Vec::new(),
            };
            Ok((l, used))
        }
        Ty::Record { fields } => {
            let (mut off, mut align, mut used) = (0u64, 1u64, 0usize);
            let mut offsets = Vec::with_capacity(fields.len());
            for f in fields {
                let (fl, fused) = match f.desc {
                    Some(c) => layout(types, consts, f.ty, const_bytes(consts, c)?, depth + 1)?,
                    None => layout(
                        types,
                        consts,
                        f.ty,
                        desc.get(used..).unwrap_or(&[]),
                        depth + 1,
                    )?,
                };
                if f.desc.is_none() {
                    used += fused;
                }
                off = off.next_multiple_of(fl.align.max(1));
                offsets.push(off);
                off = off.checked_add(fl.size).ok_or(LayoutError::TooLarge)?;
                align = align.max(fl.align);
            }
            let l = Layout {
                size: off.next_multiple_of(align),
                align,
                stride: 0,
                count: 1,
                offsets,
            };
            Ok((l, used))
        }
        _ => Err(bad("unreachable type class")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dope(dims: &[(i64, i64, i64)]) -> Vec<u8> {
        dims.iter()
            .flat_map(|&(l, r, d)| [l, r, d])
            .flat_map(i64::to_le_bytes)
            .collect()
    }

    #[test]
    fn layouts() {
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
        let ints = types.push(Ty::Array {
            elem: int,
            elem_desc: None,
            dims: 1,
        });
        let byte = consts.push(ConstData::Bytes(dope(&[(7, 0, 1)])));
        let mem = types.push(Ty::Array {
            elem: slv,
            elem_desc: Some(byte),
            dims: 1,
        });
        let rec = types.push(Ty::Record {
            fields: vec![
                Field {
                    name: "flag".into(),
                    ty: sl,
                    desc: None,
                },
                Field {
                    name: "data".into(),
                    ty: slv,
                    desc: None,
                },
                Field {
                    name: "n".into(),
                    ty: int,
                    desc: None,
                },
            ],
        });
        let (l, used) = Ty::layout(&types, &consts, slv, &dope(&[(7, 0, 1)])).unwrap();
        assert_eq!((l.size, l.count, used), (32, 8, 24));
        let (l, _) = Ty::layout(&types, &consts, slv, &dope(&[(0, 64, 0)])).unwrap();
        assert_eq!(l.size, 64);
        let (l, _) = Ty::layout(&types, &consts, slv, &dope(&[(0, -1, 0)])).unwrap();
        assert_eq!((l.size, l.count), (0, 0));
        let (l, _) = Ty::layout(&types, &consts, ints, &dope(&[(1, 10, 0)])).unwrap();
        assert_eq!((l.size, l.stride), (40, 4));
        // 1024 rows of 8 bits: one plane vector over 8192 elements.
        let (l, _) = Ty::layout(&types, &consts, mem, &dope(&[(0, 1023, 0)])).unwrap();
        assert_eq!((l.size, l.count), (8192 / 64 * 32, 8192));
        let (l, used) = Ty::layout(&types, &consts, rec, &dope(&[(15, 0, 1)])).unwrap();
        assert_eq!((l.offsets.clone(), l.size, used), (vec![0, 8, 40], 48, 24));
        assert!(Ty::needs_desc(&types, rec));
        assert!(matches!(
            Ty::layout(&types, &consts, slv, &[0; 8]),
            Err(LayoutError::Malformed(_))
        ));
        let huge = dope(&[(i64::MIN, i64::MAX, 0)]);
        assert_eq!(
            Ty::layout(&types, &consts, ints, &huge),
            Err(LayoutError::TooLarge)
        );
    }
}
