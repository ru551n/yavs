//! READ and WRITE on files of non-text types (LRM 5.5.2): `file of T` with `T` a scalar type,
//! an array of scalars, a record or an array of composites. The on-disk format is documented in
//! `docs/decisions.md` "Binary files": a scalar is little-endian in the fewest whole bytes its
//! range needs (as the reference simulators do), an array of scalars is a `u32` element count and
//! then its elements, any other composite is its descriptor and memory image.

use yavs_ir::{Ty, TyId, Width};
use yavs_kernel::Ctx;
use yavs_logic::StdULogic;

use crate::exec::{Fault, Machine, R, add_addr, ice, rt_err};
use crate::textio::{arg, text};

/// A scalar type as stored on disk and in memory.
struct Sc {
    /// Bytes on disk.
    disk: usize,
    signed: bool,
    /// Width of the value in memory.
    mem: Width,
    /// Valid range of the value as an i64 (enumeration positions, integer and physical ranges).
    lo: i64,
    hi: i64,
}

impl Sc {
    fn of(t: &Ty) -> Option<Sc> {
        Some(match *t {
            Ty::Int { lo, hi } | Ty::Physical { lo, hi, .. } => {
                let (disk, signed) = if lo >= 0 {
                    let d = if hi <= 0xff {
                        1
                    } else if hi <= 0xffff {
                        2
                    } else if hi <= 0xffff_ffff {
                        4
                    } else {
                        8
                    };
                    (d, false)
                } else if lo >= -0x80 && hi <= 0x7f {
                    (1, true)
                } else if lo >= -0x8000 && hi <= 0x7fff {
                    (2, true)
                } else if lo >= -0x8000_0000 && hi <= 0x7fff_ffff {
                    (4, true)
                } else {
                    (8, true)
                };
                Sc {
                    disk,
                    signed,
                    mem: Width::B8,
                    lo,
                    hi,
                }
            }
            Ty::Real => Sc {
                disk: 8,
                signed: false,
                mem: Width::B8,
                lo: i64::MIN,
                hi: i64::MAX,
            },
            Ty::Enum { ref lits, .. } => {
                let big = lits.len() > 256;
                Sc {
                    disk: if big { 4 } else { 1 },
                    signed: false,
                    mem: if big { Width::B4 } else { Width::B1 },
                    lo: 0,
                    hi: lits.len() as i64 - 1,
                }
            }
            _ => return None,
        })
    }

    fn encode(&self, v: u64, out: &mut Vec<u8>) {
        out.extend_from_slice(&v.to_le_bytes()[..self.disk]);
    }

    fn decode(&self, b: &[u8]) -> R<u64> {
        let mut x = [0u8; 8];
        x[..self.disk].copy_from_slice(b);
        let mut v = u64::from_le_bytes(x);
        if self.signed && self.disk < 8 {
            let s = 64 - 8 * self.disk as u32;
            v = (((v << s) as i64) >> s) as u64;
        }
        if !(self.lo..=self.hi).contains(&(v as i64)) {
            return rt_err(format!(
                "value {} read from a file is outside {} to {}",
                v as i64, self.lo, self.hi
            ));
        }
        Ok(v)
    }
}

/// What a file of type `file_ty` holds.
enum Elem {
    Scalar(Sc),
    /// An array of scalars: its dimensions, the element and whether the
    /// elements are a logic leaf (`Some(is_bit)`).
    Array(u8, Sc, Option<bool>),
    /// A record or an array of composites, as a memory image: the type and the dimensions of
    /// an array (0 for a record).
    Image(TyId, u8),
}

impl Machine {
    fn file_elem(&self, file_ty: Option<TyId>) -> R<Elem> {
        let ft = file_ty.ok_or_else(|| ice("file read/write native without a type"))?;
        let Ty::File { elem } = self.ty(ft)? else {
            return Err(ice(format!("file read/write on non-file type {ft}")));
        };
        let t = self.ty(elem)?;
        if let Some(s) = Sc::of(&t) {
            return Ok(Elem::Scalar(s));
        }
        if let Ty::Array { elem: e, dims, .. } = t
            && let Some(mut s) = Sc::of(&self.ty(e)?)
        {
            // Elements may be narrower in memory than scalars (`Ty::elem_size`); the disk
            // format is the same.
            if self.ty(e)?.elem_size() == Some(4) {
                s.mem = Width::B4;
            }
            let logic = Ty::logic_leaf(&self.rt.prog.types, elem)
                .map(|k| k != yavs_ir::LogicKind::StdULogic);
            return Ok(Elem::Array(dims, s, logic));
        }
        let dims = match t {
            Ty::Array { dims, .. } => dims,
            _ => 0,
        };
        Ok(Elem::Image(elem, dims))
    }

    /// The descriptor bytes and the size of the data of a value of type `ty` whose fat pair
    /// is at `fat`, and the address of its data.
    fn image_of(&self, ty: TyId, fat: u64) -> R<(Vec<u8>, u64, u64)> {
        let data = self.load(fat, Width::B8)?;
        let desc = self.load(add_addr(fat, 8)?, Width::B8)?;
        let dsz = Ty::desc_size(&self.rt.prog.types, ty)
            .ok_or_else(|| ice(format!("type {ty} has no descriptor size")))?;
        let d = self.rd(desc, dsz)?.to_vec();
        let (l, _) = Ty::layout(&self.rt.prog.types, &self.rt.prog.consts, ty, &d)
            .map_err(|e| Fault::Runtime(e.to_string()))?;
        Ok((d, l.size, data))
    }
}

pub(crate) fn write(
    m: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    file_ty: Option<TyId>,
    a: &[u64],
) -> R<Option<u64>> {
    let mut out = Vec::new();
    match m.file_elem(file_ty)? {
        Elem::Scalar(s) => s.encode(arg(a, 1)?, &mut out),
        Elem::Image(ty, _) => write_image(m, ty, arg(a, 1)?, &mut out)?,
        Elem::Array(dims, s, logic) => {
            let (data, lens) = m.fat(arg(a, 1)?, dims)?;
            let n = lens.iter().try_fold(1u64, |x, &l| x.checked_mul(l));
            let n = n
                .filter(|&n| n <= u64::from(u32::MAX))
                .ok_or_else(|| Fault::Runtime("array too long to write to a file".into()))?;
            out.extend_from_slice(&(n as u32).to_le_bytes());
            if let Some(bit) = logic {
                let v = m.planes(data, n)?;
                for i in 0..n as usize {
                    let code = v.get(n as usize - 1 - i) as u64;
                    let code = if bit {
                        code.checked_sub(2)
                            .filter(|&c| c <= 1)
                            .ok_or_else(|| ice("bit array element that is not '0' or '1'"))?
                    } else {
                        code
                    };
                    s.encode(code, &mut out);
                }
            } else {
                let es = s.mem.bytes() as u64;
                for i in 0..n {
                    s.encode(m.load(add_addr(data, i * es)?, s.mem)?, &mut out);
                }
            }
        }
    }
    text(m, arg(a, 0)?, |f| f.write_bytes(&out))?;
    Ok(None)
}

/// The image of a record or an array of composites: `u32` descriptor size, descriptor, `u32`
/// data size, data.
fn write_image(m: &mut Machine, ty: TyId, fat: u64, out: &mut Vec<u8>) -> R<()> {
    let (desc, size, data) = m.image_of(ty, fat)?;
    let (dn, sn) = (u32::try_from(desc.len()), u32::try_from(size));
    let (Ok(dn), Ok(sn)) = (dn, sn) else {
        return rt_err("value too large to write to a file");
    };
    out.extend_from_slice(&dn.to_le_bytes());
    out.extend_from_slice(&desc);
    out.extend_from_slice(&sn.to_le_bytes());
    out.extend_from_slice(m.rd(data, size)?);
    Ok(())
}

/// Reads an image into the object at `fat`: its bounds must be the stored ones (`LENGTH`, when
/// given, is the stored length of the first dimension).
fn read_image(
    m: &mut Machine,
    cell: u64,
    ty: TyId,
    dims: u8,
    fat: u64,
    len_out: Option<u64>,
) -> R<()> {
    let (want, size, data) = m.image_of(ty, fat)?;
    let n = |m: &mut Machine| -> R<usize> {
        let h = text(m, cell, |f| f.read_bytes(4))?;
        Ok(u32::from_le_bytes([h[0], h[1], h[2], h[3]]) as usize)
    };
    let dn = n(m)?;
    let got = text(m, cell, |f| f.read_bytes(dn))?;
    let sn = n(m)?;
    let bytes = text(m, cell, |f| f.read_bytes(sn))?;
    if got != want || sn as u64 != size {
        return rt_err("a value read from a file has other bounds than the object it is read into");
    }
    m.wr(data, size)?.copy_from_slice(&bytes);
    if let Some(l) = len_out {
        let first = if dims == 0 {
            1
        } else {
            let (_, lens) = m.fat(fat, dims)?;
            lens[0]
        };
        m.store(l, Width::B8, first)?;
    }
    Ok(())
}

pub(crate) fn read(
    m: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    file_ty: Option<TyId>,
    a: &[u64],
) -> R<Option<u64>> {
    let cell = arg(a, 0)?;
    match m.file_elem(file_ty)? {
        Elem::Scalar(s) => {
            let b = text(m, cell, |f| f.read_bytes(s.disk))?;
            let v = s.decode(&b)?;
            m.store(arg(a, 1)?, s.mem, v)?;
        }
        Elem::Image(ty, dims) => read_image(m, cell, ty, dims, arg(a, 1)?, a.get(2).copied())?,
        Elem::Array(dims, s, logic) => {
            let (data, lens) = m.fat(arg(a, 1)?, dims)?;
            let cap = lens.iter().try_fold(1u64, |x, &l| x.checked_mul(l));
            let cap = cap.ok_or_else(|| Fault::Runtime("array too long".into()))?;
            let h = text(m, cell, |f| f.read_bytes(4))?;
            let n = u64::from(u32::from_le_bytes([h[0], h[1], h[2], h[3]]));
            // The unconstrained form has a LENGTH out parameter and takes what fits; a
            // constrained array must match the stored length.
            let len_out = a.get(2).copied();
            if len_out.is_none() && n != cap {
                return rt_err(format!(
                    "array of {n} elements read from a file into an object of {cap} elements"
                ));
            }
            let k = n.min(cap);
            let es = s.disk as u64;
            let b = text(m, cell, |f| f.read_bytes((k * es) as usize))?;
            text(m, cell, |f| f.skip_bytes((n - k) * es))?;
            let vals = b
                .chunks(s.disk)
                .map(|c| s.decode(c))
                .collect::<R<Vec<u64>>>()?;
            if let Some(bit) = logic {
                let mut v = m.planes(data, cap)?;
                for (i, &c) in vals.iter().enumerate() {
                    let code = if bit { c + 2 } else { c };
                    if code > 8 || (bit && c > 1) {
                        return rt_err(format!("invalid logic value {c} read from a file"));
                    }
                    v.set(cap as usize - 1 - i, StdULogic::from_code_pub(code as u8));
                }
                m.put_planes(data, &v)?;
            } else {
                let ms = s.mem.bytes() as u64;
                for (i, &c) in vals.iter().enumerate() {
                    m.store(add_addr(data, i as u64 * ms)?, s.mem, c)?;
                }
            }
            if let Some(l) = len_out {
                m.store(l, Width::B8, k)?;
            }
        }
    }
    Ok(None)
}
