//! Composites resolved as a whole (LRM 4.6, #193). The kernel joins the composite's usual
//! leaves (`Builder::resolved_composite`) and calls one resolver with each source as the
//! leaves concatenated in leaf order: a logic leaf one `std_ulogic` code per kernel offset, a
//! byte leaf its bytes. The resolver here repacks those sources into the IR layout of an array
//! of the composite, calls the resolution function and unpacks the result the same way.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use yavs_interp::{CallVal, InstData, Runtime};
use yavs_ir::{ConstData, DOPE_DIM_BYTES, FuncId, LogicKind, Program, Ty, TyId};
use yavs_kernel::Elem;
use yavs_sem::hir::Direction;

use crate::hier::Shape;

/// Where one kernel element lives in the IR value, in bytes from the value's start.
#[derive(Clone, Copy, Debug)]
enum Place {
    /// A byte scalar: `mem` bytes at `off`, `w` bytes in the kernel (a narrow integer is 4
    /// bytes in the kernel and 8 as a record field, `Ty::elem_size`).
    Bytes { off: u64, w: u64, mem: u64 },
    /// A logic scalar (`std_ulogic`, `bit`, `boolean`): its position at `off`, which is its
    /// kernel code minus `bias` (2 for BIT and BOOLEAN, docs/decisions.md).
    Pos { off: u64, bias: u8 },
    /// An element of a logic array: bit `bit` of the plane blocks at `base` (four 64-bit
    /// planes per block of 64 elements, ir.md 1.3).
    Plane { base: u64, bit: u64 },
}

fn mismatch() -> String {
    "internal error: the composite's shape does not match its IR type".into()
}

fn const_bytes(prog: &Program, c: yavs_ir::ConstId) -> Result<&[u8], String> {
    match prog.consts.get(c) {
        Some(ConstData::Bytes(b)) => Ok(b),
        _ => Err(mismatch()),
    }
}

/// The IR descriptor of a value of shape `shape` and IR type `ty` (ir.md 1.3).
fn desc_of(prog: &Program, ty: TyId, shape: &Shape) -> Result<Vec<u8>, String> {
    let mut d = Vec::new();
    match (prog.types.get(ty).ok_or_else(mismatch)?, shape) {
        (t, Shape::Scalar { .. }) if t.scalar_size().is_some() => {}
        (
            Ty::Array {
                elem,
                elem_desc,
                dims: n,
            },
            Shape::Array { dims, elem: es, .. },
        ) if usize::from(*n) == dims.len() => {
            for b in dims {
                for x in [b.left, b.right, i64::from(b.dir == Direction::Downto)] {
                    d.extend(x.to_le_bytes());
                }
            }
            if elem_desc.is_none() {
                d.extend(desc_of(prog, *elem, es)?);
            }
        }
        (Ty::Record { fields }, Shape::Record { fields: fs, .. }) if fields.len() == fs.len() => {
            for (f, (_, s)) in fields.iter().zip(fs) {
                if f.desc.is_none() {
                    d.extend(desc_of(prog, f.ty, s)?);
                }
            }
        }
        _ => return Err(mismatch()),
    }
    Ok(d)
}

/// The places of the scalars of a value of shape `shape` (IR type `ty`, descriptor `desc`)
/// at byte `base`, per leaf (record field path), each in row-major order; `in_array`: the
/// value is an array element.
#[allow(clippy::too_many_arguments)]
fn places(
    prog: &Program,
    ty: TyId,
    shape: &Shape,
    desc: &[u8],
    base: u64,
    path: &mut Vec<u32>,
    out: &mut BTreeMap<Vec<u32>, Vec<Place>>,
    in_array: bool,
) -> Result<(), String> {
    let t = prog.types.get(ty).ok_or_else(mismatch)?;
    if let Some(w) = t.scalar_size() {
        // An array element is `elem_size` bytes in memory, a record field `scalar_size`.
        let mem = if in_array {
            t.elem_size().unwrap_or(w)
        } else {
            w
        };
        let p = match (shape, t) {
            (
                Shape::Scalar {
                    elem: Elem::Logic, ..
                },
                Ty::Enum { logic: Some(k), .. },
            ) => Place::Pos {
                off: base,
                bias: if *k == LogicKind::StdULogic { 0 } else { 2 },
            },
            (
                Shape::Scalar {
                    elem: Elem::Bytes(n),
                    ..
                },
                _,
            ) if Some(u64::from(*n)) == t.elem_size() => Place::Bytes {
                off: base,
                w: u64::from(*n),
                mem,
            },
            _ => return Err(mismatch()),
        };
        out.entry(path.clone()).or_default().push(p);
        return Ok(());
    }
    if Ty::logic_leaf(&prog.types, ty).is_some() {
        // One block of planes for all (nested) elements; row-major element p is bit n - 1 - p.
        let mut n = 1;
        let mut s = shape;
        while let Shape::Array { elem, .. } = s {
            n *= s.len();
            s = elem;
        }
        let v = out.entry(path.clone()).or_default();
        v.extend((0..n).map(|p| Place::Plane {
            base,
            bit: n - 1 - p,
        }));
        return Ok(());
    }
    let layout = |ty, desc| {
        Ty::layout(&prog.types, &prog.consts, ty, desc).map_err(|e| format!("internal error: {e}"))
    };
    match (t, shape) {
        (
            Ty::Array {
                elem, elem_desc, ..
            },
            Shape::Array { dims, elem: es, .. },
        ) => {
            let (l, _) = layout(ty, desc)?;
            let edesc = match elem_desc {
                Some(c) => const_bytes(prog, *c)?,
                None => desc
                    .get(dims.len() * DOPE_DIM_BYTES as usize..)
                    .unwrap_or(&[]),
            };
            for k in 0..l.count {
                places(prog, *elem, es, edesc, base + k * l.stride, path, out, true)?;
            }
            Ok(())
        }
        (Ty::Record { fields }, Shape::Record { fields: fs, .. }) if fields.len() == fs.len() => {
            let (l, _) = layout(ty, desc)?;
            let mut used = 0;
            for (i, ((f, (_, s)), off)) in fields.iter().zip(fs).zip(&l.offsets).enumerate() {
                let fd = match f.desc {
                    Some(c) => const_bytes(prog, c)?,
                    None => {
                        let sub = desc.get(used..).unwrap_or(&[]);
                        let (_, n) = layout(f.ty, sub)?;
                        used += n;
                        &sub[..n]
                    }
                };
                path.push(i as u32);
                places(prog, f.ty, s, fd, base + off, path, out, false)?;
                path.pop();
            }
            Ok(())
        }
        _ => Err(mismatch()),
    }
}

/// How one composite value maps between the kernel and IR memory.
pub struct Map {
    /// One place per kernel element, in the kernel's concatenation order.
    flat: Vec<Place>,
    /// Bytes of the IR value, and the distance between array elements of it.
    size: u64,
    count: u64,
    stride: u64,
    pub(crate) desc: Vec<u8>,
    /// `Some(n)`: the composite is a logic array of `n` elements, so an array of them is one
    /// block of planes (ir.md 1.3).
    logic: Option<u64>,
}

fn short() -> String {
    "a resolution value of the wrong size".into()
}

impl Map {
    pub(crate) fn new(prog: &Program, ty: TyId, shape: &Shape) -> Result<Map, String> {
        let desc = desc_of(prog, ty, shape)?;
        let mut per = BTreeMap::new();
        places(prog, ty, shape, &desc, 0, &mut Vec::new(), &mut per, false)?;
        let mut flat = Vec::new();
        for (path, leaf, n) in shape.leaves() {
            let mut v = per.remove(&path).ok_or_else(mismatch)?;
            if v.len() as u64 != n {
                return Err(mismatch());
            }
            // Kernel offset k of a logic leaf is row-major element n - 1 - k (hier.rs).
            if matches!(
                leaf,
                Shape::Scalar {
                    elem: Elem::Logic,
                    ..
                }
            ) {
                v.reverse();
            }
            flat.extend(v);
        }
        let (l, _) = Ty::layout(&prog.types, &prog.consts, ty, &desc)
            .map_err(|e| format!("internal error: {e}"))?;
        let logic = Ty::logic_leaf(&prog.types, ty).map(|_| l.count);
        Ok(Map {
            flat,
            size: l.size,
            count: l.count,
            stride: l.size.next_multiple_of(l.align.max(1)),
            desc,
            logic,
        })
    }

    /// One value of the composite in IR memory from its kernel leaves concatenated (a port
    /// conversion's argument).
    pub(crate) fn pack(&self, src: &[u8]) -> Result<Vec<u8>, String> {
        let mut data = vec![0u8; self.size as usize];
        self.put(src, &mut data, 0, 0).ok_or_else(short)?;
        Ok(data)
    }

    /// The kernel leaves concatenated of the IR value `data` of type `ty` with descriptor
    /// `desc` (a port conversion's result), checked to have this composite's size.
    pub(crate) fn unpack(
        &self,
        prog: &Program,
        ty: TyId,
        data: &[u8],
        desc: &[u8],
    ) -> Result<Vec<u8>, String> {
        let fits = Ty::layout(&prog.types, &prog.consts, ty, desc)
            .is_ok_and(|(l, _)| l.size == self.size && l.count == self.count);
        if !fits {
            return Err("the conversion result does not fit the port's subtype".into());
        }
        let n = self
            .flat
            .iter()
            .map(|p| match p {
                Place::Bytes { w, .. } => *w as usize,
                Place::Pos { .. } | Place::Plane { .. } => 1,
            })
            .sum();
        let mut out = vec![0u8; n];
        self.get(data, &mut out).ok_or_else(short)?;
        Ok(out)
    }

    /// Write kernel value `src` into `data`, `sb` bytes and `bs` plane bits further on.
    fn put(&self, src: &[u8], data: &mut [u8], sb: u64, bs: u64) -> Option<()> {
        let mut at = 0;
        for p in &self.flat {
            match *p {
                Place::Bytes { off, w, mem } => {
                    let (o, w, mem) = ((sb + off) as usize, w as usize, mem as usize);
                    let k = src.get(at..at + w)?;
                    let d = data.get_mut(o..o + mem)?;
                    match <[u8; 4]>::try_from(k) {
                        // A narrow integer widened: sign-extended.
                        Ok(x) if mem == 8 => {
                            d.copy_from_slice(&i64::from(i32::from_le_bytes(x)).to_le_bytes())
                        }
                        _ => d.copy_from_slice(k),
                    }
                    at += w;
                }
                Place::Pos { off, bias } => {
                    *data.get_mut((sb + off) as usize)? = src.get(at)?.checked_sub(bias)?;
                    at += 1;
                }
                Place::Plane { base, bit } => {
                    let code = *src.get(at)?;
                    let b = bit + bs;
                    for plane in 0..4 {
                        let byte = (sb + base + b / 64 * 32 + plane * 8 + b % 64 / 8) as usize;
                        let m = 1u8 << (b % 8);
                        let x = data.get_mut(byte)?;
                        *x = if code >> plane & 1 == 1 {
                            *x | m
                        } else {
                            *x & !m
                        };
                    }
                    at += 1;
                }
            }
        }
        (at == src.len()).then_some(())
    }

    /// Read the IR value `data` into kernel value `out` (the inverse of [`Map::put`]).
    fn get(&self, data: &[u8], out: &mut [u8]) -> Option<()> {
        let mut at = 0;
        for p in &self.flat {
            match *p {
                Place::Bytes { off, w, .. } => {
                    let (o, w) = (off as usize, w as usize);
                    // Little-endian: a narrow integer's kernel bytes are its first bytes.
                    out.get_mut(at..at + w)?
                        .copy_from_slice(data.get(o..o + w)?);
                    at += w;
                }
                Place::Pos { off, bias } => {
                    *out.get_mut(at)? = data.get(off as usize)?.checked_add(bias)?;
                    at += 1;
                }
                Place::Plane { base, bit } => {
                    let mut code = 0;
                    for plane in 0..4 {
                        let byte = (base + bit / 64 * 32 + plane * 8 + bit % 64 / 8) as usize;
                        code |= (data.get(byte)? >> (bit % 8) & 1) << plane;
                    }
                    *out.get_mut(at)? = code;
                    at += 1;
                }
            }
        }
        (at == out.len()).then_some(())
    }
}

/// The kernel resolver of a signal of shape `shape` resolved as a whole by `func`, whose
/// parameter has IR type `param` (an unconstrained array of the composite) and whose result
/// has IR type `ty`; `inst` is the data of the region declaring `func` (#164); the array of
/// driving values starts at `left` and runs down when `down`.
pub(crate) fn resolver(
    rt: Arc<Runtime>,
    func: FuncId,
    inst: Arc<InstData>,
    (param, ty): (TyId, TyId),
    (left, down): (i64, bool),
    shape: &Shape,
    shared: Option<Arc<Mutex<yavs_interp::SharedState>>>,
) -> Result<yavs_kernel::Resolver, String> {
    let map = Map::new(&rt.prog, ty, shape)?;
    let tail = match rt.prog.types.get(param) {
        Some(Ty::Array {
            elem_desc, dims: 1, ..
        }) => elem_desc.is_none(),
        _ => return Err(mismatch()),
    };
    Ok(Arc::new(move |vals: &[&[u8]], out: &mut [u8]| {
        let n = vals.len() as u64;
        let mut data = vec![
            0u8;
            match map.logic {
                Some(m) => (n * m).div_ceil(64) * 32,
                None => n * map.stride,
            } as usize
        ];
        for (j, v) in vals.iter().enumerate() {
            let j = j as u64;
            let (sb, bs) = match map.logic {
                // Row-major element j of the array starts at bit (n - 1 - j) * m.
                Some(m) => (0, (n - 1 - j) * m),
                None => (j * map.stride, 0),
            };
            map.put(v, &mut data, sb, bs).ok_or_else(short)?;
        }
        let mut desc: Vec<u8> = yavs_interp::res_dope(left, down, n)
            .ok_or("too many drivers for the resolution function's index subtype")?
            .to_vec();
        if tail {
            desc.extend(&map.desc);
        }
        let arg = CallVal::Composite { data, desc };
        let r = yavs_interp::call_values_shared(
            rt.clone(),
            func,
            inst.clone(),
            &[arg],
            Some(ty),
            shared.as_ref(),
        )
        .map_err(|e| e.to_string())?;
        let Some(CallVal::Composite { data, desc }) = r else {
            return Err("the resolution function returned no composite value".into());
        };
        let fits = Ty::layout(&rt.prog.types, &rt.prog.consts, ty, &desc)
            .is_ok_and(|(l, _)| l.size == map.size && l.count == map.count);
        if !fits {
            return Err("the resolved value does not fit the signal's subtype".into());
        }
        map.get(&data, out).ok_or_else(short)
    }))
}
