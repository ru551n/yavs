//! Composite elaboration-time values (#77): array and record values as `Val`, and their
//! marshalling to and from interpreter memory for elaboration-time calls (`call_values`).
//!
//! ponytail: slices, concatenations and aggregates expand runs into one `Val` per element;
//! fine for elaboration-time values (generics, constants, initial values); keep runs if a
//! design folds huge aggregates here.

use std::collections::HashMap;
use std::rc::Rc;

use yavs_interp::CallVal;
use yavs_ir::{DOPE_DIM_BYTES, LogicKind, Program, Ty, TyId};
use yavs_sem::stat::{ArrayConst, Elems};
use yavs_sem::types::Direction;

use crate::eval::Val;
use crate::hier::Bounds;

pub(crate) fn bounds_of(a: &ArrayConst) -> Vec<Bounds> {
    a.bounds
        .iter()
        .map(|&(left, dir, right)| Bounds { left, dir, right })
        .collect()
}

/// The elements of `a` in row-major order.
pub(crate) fn elems(a: &ArrayConst) -> Vec<Val> {
    match &a.elems {
        Elems::Bytes(b) => b.iter().map(|&x| Val::Enum(u32::from(x))).collect(),
        Elems::Runs(rs) => rs
            .iter()
            .flat_map(|(n, x)| std::iter::repeat_n(x.clone(), *n as usize))
            .collect(),
    }
}

/// LRM 9.2.3: values of an array type are equal when their lengths and elements are, whatever
/// their bounds and directions; other values by structure.
pub(crate) fn same(a: &Val, b: &Val) -> bool {
    match (a, b) {
        (Val::Array(x), Val::Array(y)) => {
            let (bx, by) = (bounds_of(x), bounds_of(y));
            bx.len() == by.len() && bx.iter().zip(&by).all(|(p, q)| p.len() == q.len()) && {
                let (ex, ey) = (elems(x), elems(y));
                ex.len() == ey.len() && ex.iter().zip(&ey).all(|(p, q)| same(p, q))
            }
        }
        (Val::Record(x), Val::Record(y)) => {
            x.len() == y.len() && x.iter().zip(y.iter()).all(|(p, q)| same(p, q))
        }
        _ => a == b,
    }
}

/// Element `k` (row major) of `a`, without expanding runs.
pub(crate) fn elem_at(a: &ArrayConst, mut k: u64) -> Option<Val> {
    match &a.elems {
        Elems::Bytes(b) => b.get(k as usize).map(|&x| Val::Enum(u32::from(x))),
        Elems::Runs(rs) => {
            for (n, x) in rs.iter() {
                if k < *n {
                    return Some(x.clone());
                }
                k -= n;
            }
            None
        }
    }
}

/// An array value: bytes for 1-D arrays of small enumeration positions, else runs.
pub(crate) fn make(bounds: &[Bounds], xs: Vec<Val>) -> Val {
    let bounds: Box<[_]> = bounds.iter().map(|b| (b.left, b.dir, b.right)).collect();
    let small = |x: &Val| matches!(x, Val::Enum(p) if *p < 256);
    let elems = if bounds.len() == 1 && !xs.is_empty() && xs.iter().all(small) {
        Elems::Bytes(xs.iter().map(|x| x.as_int().unwrap_or(0) as u8).collect())
    } else {
        let mut runs: Vec<(u64, Val)> = Vec::new();
        for x in xs {
            match runs.last_mut() {
                Some((n, y)) if *y == x => *n += 1,
                _ => runs.push((1, x)),
            }
        }
        Elems::Runs(runs.into())
    };
    Val::Array(ArrayConst { bounds, elems })
}

/// Bounds of `n` elements starting at `left` in direction `dir`.
pub(crate) fn span(left: i64, dir: Direction, n: u64) -> Bounds {
    let n = n as i64;
    Bounds {
        left,
        dir,
        right: match dir {
            Direction::To => left + n - 1,
            Direction::Downto => left - n + 1,
        },
    }
}

fn scalar_cell(v: &Val) -> Option<u64> {
    match v {
        Val::Int(x) | Val::Phys(x) => Some(*x as u64),
        Val::Enum(p) => Some(u64::from(*p)),
        Val::Real(r) => Some(r.to_bits()),
        _ => None,
    }
}

fn scalar_val(t: &Ty, c: u64) -> Option<Val> {
    Some(match t {
        Ty::Enum { .. } => Val::Enum(c as u32),
        Ty::Int { .. } => Val::Int(c as i64),
        Ty::Physical { .. } => Val::Phys(c as i64),
        Ty::Real => Val::Real(f64::from_bits(c)),
        _ => return None,
    })
}

/// The cell of `n` bytes at `at`; a 4-byte cell (a narrow integer array element,
/// `Ty::elem_size`) is sign-extended.
fn read(b: &[u8], at: usize, n: usize) -> Option<u64> {
    if n == 4 {
        let w: [u8; 4] = b.get(at..at + 4)?.try_into().ok()?;
        return Some(i64::from(i32::from_le_bytes(w)) as u64);
    }
    let mut w = [0u8; 8];
    w[..n].copy_from_slice(b.get(at..at + n)?);
    Some(u64::from_le_bytes(w))
}

/// `v` of IR type `ty` as a call argument: a scalar as a cell, a composite as data and
/// descriptor bytes in the IR layout. `None` for a value that does not fit the type.
pub fn encode(prog: &Program, ty: TyId, v: &Val) -> Option<CallVal> {
    if prog.types.get(ty)?.scalar_size().is_some() {
        return scalar_cell(v).map(CallVal::Cell);
    }
    let (data, desc) = put(prog, ty, v)?;
    Some(CallVal::Composite { data, desc })
}

/// The descriptor bytes of a static constraint in the pool.
fn pool(prog: &Program, c: yavs_ir::ConstId) -> Option<&[u8]> {
    match prog.consts.get(c)? {
        yavs_ir::ConstData::Bytes(b) => Some(b),
        _ => None,
    }
}

pub(crate) fn dope(bounds: &[(i64, Direction, i64)]) -> Vec<u8> {
    bounds
        .iter()
        .flat_map(|&(l, d, r)| [l, r, i64::from(d == Direction::Downto)])
        .flat_map(i64::to_le_bytes)
        .collect()
}

/// Value `v` of IR type `ty`: its data and descriptor (ir.md 1.3). Composite elements and
/// fields are laid out as `Ty::layout` places them; the element descriptor of an array whose
/// type gives none is the first element's (all elements of an array share one subtype).
fn put(prog: &Program, ty: TyId, v: &Val) -> Option<(Vec<u8>, Vec<u8>)> {
    let t = prog.types.get(ty)?;
    if let Some(n) = t.scalar_size() {
        let c = scalar_cell(v)?;
        return Some((c.to_le_bytes()[..n as usize].to_vec(), Vec::new()));
    }
    match (t, v) {
        (Ty::Array { .. }, Val::Array(_)) if Ty::logic_leaf(&prog.types, ty).is_some() => {
            let k = Ty::logic_leaf(&prog.types, ty)?;
            let bias = if k == LogicKind::StdULogic { 0 } else { 2 };
            let mut codes = Vec::new();
            let desc = logic_codes(prog, ty, v, &mut codes)?;
            let n = codes.len();
            let mut blocks = vec![[0u64; 4]; n.div_ceil(64)];
            for (p, &c) in codes.iter().enumerate() {
                let i = n - 1 - p; // element 0 is the rightmost
                for (plane, w) in blocks[i / 64].iter_mut().enumerate() {
                    *w |= ((c + bias) >> plane & 1) << (i % 64);
                }
            }
            let data = blocks
                .iter()
                .flat_map(|b| b.iter().flat_map(|w| w.to_le_bytes()))
                .collect();
            Some((data, desc))
        }
        (
            Ty::Array {
                elem,
                elem_desc,
                dims,
            },
            Val::Array(a),
        ) => {
            if a.bounds.len() != *dims as usize {
                return None;
            }
            let xs = elems(a);
            let mut parts = Vec::with_capacity(xs.len());
            // A scalar element may be narrower than the scalar (`Ty::elem_size`).
            let es = prog.types.get(*elem)?.elem_size();
            for x in &xs {
                let (mut d, ed) = put(prog, *elem, x)?;
                if let Some(n) = es {
                    d.truncate(n as usize);
                }
                parts.push((d, ed));
            }
            let mut desc = dope(&a.bounds);
            if elem_desc.is_none() && Ty::needs_desc(&prog.types, *elem) {
                // ponytail: a null array of unconstrained elements has no element to take
                // the element bounds from; the subtype would have to give them.
                desc.extend_from_slice(&parts.first()?.1);
            }
            let (l, _) = Ty::layout(&prog.types, &prog.consts, ty, &desc).ok()?;
            let mut data = vec![0u8; l.size as usize];
            for (k, (d, _)) in parts.iter().enumerate() {
                let at = k * l.stride as usize;
                data.get_mut(at..at + d.len())?.copy_from_slice(d);
            }
            Some((data, desc))
        }
        (Ty::Record { fields }, Val::Record(vs)) if vs.len() == fields.len() => {
            let mut parts = Vec::with_capacity(vs.len());
            let mut desc = Vec::new();
            for (f, x) in fields.iter().zip(vs.iter()) {
                let (d, fd) = put(prog, f.ty, x)?;
                if f.desc.is_none() {
                    desc.extend_from_slice(&fd);
                }
                parts.push(d);
            }
            let (l, _) = Ty::layout(&prog.types, &prog.consts, ty, &desc).ok()?;
            let mut data = vec![0u8; l.size as usize];
            for (d, &off) in parts.iter().zip(&l.offsets) {
                let off = off as usize;
                data.get_mut(off..off + d.len())?.copy_from_slice(d);
            }
            Some((data, desc))
        }
        _ => None,
    }
}

/// The scalar leaves of logic array value `v` (nested arrays row major) into `out`, and the
/// descriptor of `v`.
fn logic_codes(prog: &Program, ty: TyId, v: &Val, out: &mut Vec<u64>) -> Option<Vec<u8>> {
    let t = prog.types.get(ty)?;
    if t.scalar_size().is_some() {
        out.push(scalar_cell(v)?);
        return Some(Vec::new());
    }
    let (
        Ty::Array {
            elem,
            elem_desc,
            dims,
        },
        Val::Array(a),
    ) = (t, v)
    else {
        return None;
    };
    if a.bounds.len() != *dims as usize {
        return None;
    }
    let mut desc = dope(&a.bounds);
    let mut first = None;
    for x in elems(a) {
        let d = logic_codes(prog, *elem, &x, out)?;
        first.get_or_insert(d);
    }
    if elem_desc.is_none() && Ty::needs_desc(&prog.types, *elem) {
        desc.extend_from_slice(&first?);
    }
    Some(desc)
}

/// The value of IR type `ty` in `data` with descriptor `desc`, and the descriptor bytes it
/// used (the inverse of [`put`]).
fn get(prog: &Program, ty: TyId, data: &[u8], desc: &[u8]) -> Option<(Val, usize)> {
    let t = prog.types.get(ty)?;
    if let Some(n) = t.scalar_size() {
        return Some((scalar_val(t, read(data, 0, n as usize)?)?, 0));
    }
    let (l, used) = Ty::layout(&prog.types, &prog.consts, ty, desc).ok()?;
    match t {
        Ty::Array { .. } if Ty::logic_leaf(&prog.types, ty).is_some() => {
            let k = Ty::logic_leaf(&prog.types, ty)?;
            let bias = if k == LogicKind::StdULogic { 0 } else { 2 };
            let n = l.count as usize;
            // A vector of scalars (the common case): the bytes straight from the planes.
            if let Ty::Array { dims: 1, elem, .. } = t
                && prog.types.get(*elem).and_then(Ty::scalar_size).is_some()
                && n > 0
            {
                let (bounds, _) = dims_of(t, desc)?;
                let mut bytes = vec![0u8; n];
                for (w, chunk) in bytes.rchunks_mut(64).enumerate() {
                    let mut planes = [0u64; 4];
                    for (plane, word) in planes.iter_mut().enumerate() {
                        *word = read(data, w * 32 + plane * 8, 8)?;
                    }
                    for (j, b) in chunk.iter_mut().rev().enumerate() {
                        let code =
                            (0..4).fold(0u64, |c, plane| c | (planes[plane] >> j & 1) << plane);
                        *b = u8::try_from(code.checked_sub(u64::from(bias))?).ok()?;
                    }
                }
                let bounds: Box<[_]> = bounds.iter().map(|b| (b.left, b.dir, b.right)).collect();
                let elems = Elems::Bytes(bytes.into());
                return Some((Val::Array(ArrayConst { bounds, elems }), used));
            }
            let mut codes = Vec::with_capacity(n);
            for p in 0..n {
                let i = n - 1 - p;
                let mut code = 0;
                for plane in 0..4 {
                    let word = read(data, (i / 64) * 32 + plane * 8, 8)?;
                    code |= (word >> (i % 64) & 1) << plane;
                }
                codes.push(u32::try_from(code).ok()?.checked_sub(bias)?);
            }
            Some((logic_val(prog, ty, desc, &codes)?, used))
        }
        Ty::Array {
            elem, elem_desc, ..
        } => {
            let (bounds, at) = dims_of(t, desc)?;
            let ed = match elem_desc {
                Some(c) => pool(prog, *c)?,
                None => desc.get(at..)?,
            };
            let mut xs = Vec::with_capacity(l.count as usize);
            let et = prog.types.get(*elem)?;
            for k in 0..l.count as usize {
                let at = k * l.stride as usize;
                xs.push(match et.elem_size() {
                    Some(n) => scalar_val(et, read(data, at, n as usize)?)?,
                    None => get(prog, *elem, data.get(at..)?, ed)?.0,
                });
            }
            Some((make(&bounds, xs), used))
        }
        Ty::Record { fields } => {
            let mut vs = Vec::with_capacity(fields.len());
            let mut at = 0;
            for (f, &off) in fields.iter().zip(&l.offsets) {
                let fd = match f.desc {
                    Some(c) => pool(prog, c)?,
                    None => desc.get(at..)?,
                };
                let (x, n) = get(prog, f.ty, data.get(off as usize..)?, fd)?;
                if f.desc.is_none() {
                    at += n;
                }
                vs.push(x);
            }
            Some((Val::Record(vs.into()), used))
        }
        _ => None,
    }
}

/// The bounds in the descriptor of array type `t`, and the offset after them.
fn dims_of(t: &Ty, desc: &[u8]) -> Option<(Vec<Bounds>, usize)> {
    let Ty::Array { dims, .. } = t else {
        return None;
    };
    let mut bounds = Vec::new();
    for d in 0..*dims as usize {
        let at = d * DOPE_DIM_BYTES as usize;
        let (l, r, dir) = (
            read(desc, at, 8)?,
            read(desc, at + 8, 8)?,
            read(desc, at + 16, 8)?,
        );
        bounds.push(Bounds {
            left: l as i64,
            dir: if dir == 0 {
                Direction::To
            } else {
                Direction::Downto
            },
            right: r as i64,
        });
    }
    Some((bounds, *dims as usize * DOPE_DIM_BYTES as usize))
}

/// The value of logic array type `ty` with descriptor `desc` from its leaf codes.
fn logic_val(prog: &Program, ty: TyId, desc: &[u8], codes: &[u32]) -> Option<Val> {
    let t = prog.types.get(ty)?;
    if t.scalar_size().is_some() {
        return Some(Val::Enum(*codes.first()?));
    }
    let Ty::Array {
        elem, elem_desc, ..
    } = t
    else {
        return None;
    };
    let (bounds, at) = dims_of(t, desc)?;
    // A vector of scalars: its codes are the bytes (`make` would find them small).
    if bounds.len() == 1
        && !codes.is_empty()
        && prog.types.get(*elem).and_then(Ty::scalar_size).is_some()
        && codes.len() as u64 == bounds[0].len()
        && codes.iter().all(|&c| c < 256)
    {
        let bounds = bounds.iter().map(|b| (b.left, b.dir, b.right)).collect();
        let elems = Elems::Bytes(codes.iter().map(|&c| c as u8).collect());
        return Some(Val::Array(ArrayConst { bounds, elems }));
    }
    let ed = match elem_desc {
        Some(c) => pool(prog, *c)?,
        None => desc.get(at..)?,
    };
    let w = Ty::layout(&prog.types, &prog.consts, *elem, ed)
        .ok()?
        .0
        .count as usize;
    let n = bounds.iter().map(|b| b.len()).product::<u64>() as usize;
    let mut xs = Vec::with_capacity(n);
    for k in 0..n {
        xs.push(logic_val(prog, *elem, ed, codes.get(k * w..(k + 1) * w)?)?);
    }
    Some(make(&bounds, xs))
}

/// The value of a call result of IR type `ty` (the inverse of [`encode`]).
pub fn decode(prog: &Program, ty: TyId, r: &CallVal) -> Option<Val> {
    match r {
        CallVal::Cell(c) => scalar_val(prog.types.get(ty)?, *c),
        CallVal::Composite { data, desc } => get(prog, ty, data, desc).map(|(v, _)| v),
    }
}

/// How a scalar cell reads: an enumeration position, an integer or physical value, a real.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CellKind {
    Enum,
    Int,
    Real,
}

/// Where one leaf of a value lives in its IR data (the layout [`decode`] reads): a scalar
/// cell, or a run of elements of a logic array's bit-planes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DataLeaf {
    /// `n` bytes at byte `at`.
    Cell { at: u32, n: u8, kind: CellKind },
    /// DataLeaf elements `from .. from + len` (row major) of the logic array of `total` leaf
    /// elements whose planes start at byte `at`; `bias` is the code of position 0.
    Planes {
        at: u32,
        total: u32,
        from: u32,
        len: u32,
        bias: u8,
    },
}

impl DataLeaf {
    /// The bytes of the data the leaf is read from, `lo .. hi`.
    pub fn span(&self) -> (usize, usize) {
        match *self {
            DataLeaf::Cell { at, n, .. } => (at as usize, at as usize + n as usize),
            DataLeaf::Planes {
                at,
                total,
                from,
                len,
                ..
            } => {
                // Element `p` is plane bit `total - 1 - p`; 64 bits are 32 bytes.
                let (lo, hi) = ((total - from - len) as usize, (total - 1 - from) as usize);
                (at as usize + lo / 64 * 32, at as usize + (hi / 64 + 1) * 32)
            }
        }
    }

    /// The number of bytes [`DataLeaf::bytes`] appends.
    pub fn width(&self, logic: bool) -> usize {
        match *self {
            DataLeaf::Cell { kind, .. } => match kind {
                CellKind::Enum if logic => 1,
                CellKind::Enum => 4,
                CellKind::Int | CellKind::Real => 8,
            },
            DataLeaf::Planes { len, .. } if logic => len as usize,
            DataLeaf::Planes { len, .. } => 4 * len as usize,
        }
    }

    /// The leaf's value of `data` as the waveform layouts have it, appended to `out`: with
    /// `logic`, one byte per enumeration position (the codes of a vector), else an
    /// enumeration as a `u32`, integers and physicals as `i64`, reals as `f64`, little
    /// endian. `None` when `data` does not hold the leaf (a bug of the caller's plan).
    pub fn bytes(&self, data: &[u8], logic: bool, out: &mut Vec<u8>) -> Option<()> {
        match *self {
            DataLeaf::Cell { at, n, kind } => {
                let c = read(data, at as usize, n as usize)?;
                match kind {
                    CellKind::Enum if logic => out.push(c as u8),
                    CellKind::Enum => out.extend_from_slice(&(c as u32).to_le_bytes()),
                    CellKind::Int | CellKind::Real => out.extend_from_slice(&c.to_le_bytes()),
                }
            }
            DataLeaf::Planes {
                at,
                total,
                from,
                len,
                bias,
            } => {
                for p in from..from + len {
                    let i = (total - 1 - p) as usize;
                    let base = at as usize + i / 64 * 32;
                    let mut code = 0u64;
                    for plane in 0..4 {
                        code |= (read(data, base + plane * 8, 8)? >> (i % 64) & 1) << plane;
                    }
                    let v = u32::try_from(code.checked_sub(u64::from(bias))?).ok()?;
                    if logic {
                        out.push(u8::try_from(v).ok()?);
                    } else {
                        out.extend_from_slice(&v.to_le_bytes());
                    }
                }
            }
        }
        Some(())
    }
}

/// Where the leaf at each of `paths` lives in the data of a value of IR type `ty` with
/// descriptor `desc`, without decoding the value (the same layout as [`decode`]). A path is
/// the field index at a record and the row-major element index at an array; it ends at a
/// scalar, or at a logic array (the whole array is then one run of its planes). `paths`
/// are in order of first elements, so a group is one call. `None` when a path does not fit
/// the type.
pub fn leaf_locs(prog: &Program, ty: TyId, desc: &[u8], paths: &[&[u32]]) -> Option<Vec<DataLeaf>> {
    let mut l = Locator {
        prog,
        memo: HashMap::new(),
        out: vec![None; paths.len()],
    };
    let ents: Vec<(usize, &[u32])> = paths.iter().copied().enumerate().collect();
    l.go(ty, desc, 0, &ents, false)?;
    l.out.into_iter().collect()
}

type Ents<'a> = [(usize, &'a [u32])];
/// Entries with the same first path element, with it removed.
type Group<'a> = (u32, Vec<(usize, &'a [u32])>);

struct Locator<'p> {
    prog: &'p Program,
    /// Layouts by type and descriptor slice (the elements of an array share theirs).
    memo: HashMap<(TyId, usize, usize), Rc<(yavs_ir::Layout, usize)>>,
    out: Vec<Option<DataLeaf>>,
}

impl Locator<'_> {
    fn layout(&mut self, ty: TyId, desc: &[u8]) -> Option<Rc<(yavs_ir::Layout, usize)>> {
        let key = (ty, desc.as_ptr() as usize, desc.len());
        if let Some(l) = self.memo.get(&key) {
            return Some(l.clone());
        }
        let l = Rc::new(Ty::layout(&self.prog.types, &self.prog.consts, ty, desc).ok()?);
        self.memo.insert(key, l.clone());
        Some(l)
    }

    /// Consecutive entries with the same first path element, with it removed.
    fn groups<'a>(ents: &Ents<'a>) -> Option<Vec<Group<'a>>> {
        let mut gs: Vec<Group<'a>> = Vec::new();
        for &(i, p) in ents {
            let (&k, rest) = p.split_first()?;
            match gs.last_mut() {
                Some((g, v)) if *g == k => v.push((i, rest)),
                _ => gs.push((k, vec![(i, rest)])),
            }
        }
        Some(gs)
    }

    /// `elem`: the value is an array element, so a narrow integer takes its element size.
    fn go(&mut self, ty: TyId, desc: &[u8], at: usize, ents: &Ents<'_>, elem: bool) -> Option<()> {
        let prog = self.prog;
        let t = prog.types.get(ty)?;
        if let Some(n) = t
            .scalar_size()
            .map(|n| if elem { t.elem_size().unwrap_or(n) } else { n })
        {
            let kind = match t {
                Ty::Enum { .. } => CellKind::Enum,
                Ty::Int { .. } | Ty::Physical { .. } => CellKind::Int,
                Ty::Real => CellKind::Real,
                _ => return None,
            };
            for &(i, p) in ents {
                if !p.is_empty() {
                    return None;
                }
                self.out[i] = Some(DataLeaf::Cell {
                    at: u32::try_from(at).ok()?,
                    n: n as u8,
                    kind,
                });
            }
            return Some(());
        }
        let lay = self.layout(ty, desc)?;
        let l = &lay.0;
        match t {
            Ty::Array { .. } if Ty::logic_leaf(&prog.types, ty).is_some() => {
                self.planes(ty, desc, at, l.count, ents)
            }
            Ty::Array {
                elem, elem_desc, ..
            } => {
                let (_, dat) = dims_of(t, desc)?;
                let ed = match elem_desc {
                    Some(c) => pool(prog, *c)?,
                    None => desc.get(dat..)?,
                };
                for (k, g) in Self::groups(ents)? {
                    if u64::from(k) >= l.count {
                        return None;
                    }
                    self.go(*elem, ed, at + k as usize * l.stride as usize, &g, true)?;
                }
                Some(())
            }
            Ty::Record { fields } => {
                let mut used = 0;
                let mut fds = Vec::with_capacity(fields.len());
                for f in fields {
                    let fd = match f.desc {
                        Some(c) => pool(prog, c)?,
                        None => desc.get(used..)?,
                    };
                    if f.desc.is_none() {
                        used += self.layout(f.ty, fd)?.1;
                    }
                    fds.push(fd);
                }
                for (k, g) in Self::groups(ents)? {
                    let k = k as usize;
                    let f = fields.get(k)?;
                    self.go(f.ty, fds[k], at + *l.offsets.get(k)? as usize, &g, false)?;
                }
                Some(())
            }
            _ => None,
        }
    }

    /// The runs of a logic array of `total` leaf elements: each path index selects one
    /// element of the nesting level it is at.
    fn planes(
        &mut self,
        ty: TyId,
        desc: &[u8],
        at: usize,
        total: u64,
        ents: &Ents<'_>,
    ) -> Option<()> {
        let prog = self.prog;
        let bias = if Ty::logic_leaf(&prog.types, ty)? == LogicKind::StdULogic {
            0
        } else {
            2
        };
        // Element counts per nesting level, outermost first.
        let (mut counts, mut t, mut d) = (Vec::new(), ty, desc);
        while let Some(
            tt @ Ty::Array {
                elem, elem_desc, ..
            },
        ) = prog.types.get(t)
        {
            let (bounds, dat) = dims_of(tt, d)?;
            counts.push(bounds.iter().map(|b| b.len()).product::<u64>());
            d = match elem_desc {
                Some(c) => pool(prog, *c)?,
                None => d.get(dat..)?,
            };
            t = *elem;
        }
        // width[k]: leaf elements in one element of level k - 1 (`width[levels]` is 1).
        let mut width = vec![1u64; counts.len() + 1];
        for k in (0..counts.len()).rev() {
            width[k] = width[k + 1].checked_mul(counts[k])?;
        }
        if width[0] != total {
            return None;
        }
        for &(i, p) in ents {
            if p.len() > counts.len() {
                return None;
            }
            let mut from = 0u64;
            for (k, &x) in p.iter().enumerate() {
                if u64::from(x) >= counts[k] {
                    return None;
                }
                from += u64::from(x) * width[k + 1];
            }
            self.out[i] = Some(DataLeaf::Planes {
                at: u32::try_from(at).ok()?,
                total: u32::try_from(total).ok()?,
                from: u32::try_from(from).ok()?,
                len: u32::try_from(width[p.len()]).ok()?,
                bias,
            });
        }
        Some(())
    }
}

// ---------------------------------------------------------------------------
// Composite expressions.

use yavs_diag::Diagnostic;
use yavs_sem::decl::Body;
use yavs_sem::hir::{Actual, AggAssoc, AggBounds, Choice, ExprKind};
use yavs_sem::implicit::Family;
use yavs_sem::types::{Constraint, TypeKind};
use yavs_sem::{Db, ExprId};

use crate::codes;
use crate::eval::{Model, R, err, int};
use crate::hier::{ScopeId, Shape};

fn not_yet(db: &Db, e: ExprId, what: &str) -> Box<Diagnostic> {
    Box::new(
        Diagnostic::error(
            codes::NOT_EVALUATED,
            format!("this expression cannot be evaluated during elaboration yet: {what}"),
        )
        .with_primary(db.exprs[e].span, "evaluated during elaboration")
        .with_note("elaboration evaluates aggregates, indexing, slices, fields, conversions, predefined operations and calls of functions with constant `in` parameters"),
    )
}

fn fails(db: &Db, e: ExprId, why: String) -> Box<Diagnostic> {
    err(
        codes::EVAL_FAILED,
        db.exprs[e].span,
        format!("evaluation fails during elaboration: {why}"),
        "here",
    )
}

fn is_composite(db: &Db, e: ExprId) -> bool {
    matches!(
        db.types[db.base(db.exprs[e].subtype)].kind,
        TypeKind::Array { .. } | TypeKind::Record { .. }
    )
}

impl Model {
    /// Composite expressions `Db::fold_with` does not fold; `None` when `e` is none of them.
    pub(crate) fn composite(
        &mut self,
        db: &Db,
        e: ExprId,
        scope: Option<ScopeId>,
    ) -> R<Option<Val>> {
        let ex = &db.exprs[e];
        Ok(Some(match &ex.kind {
            ExprKind::Aggregate { assocs, .. } => self.aggregate(db, e, assocs, scope)?,
            ExprKind::Index { prefix, indices } if is_composite(db, *prefix) => {
                let Val::Array(a) = self.eval(db, *prefix, scope)? else {
                    return Err(not_yet(db, e, "the prefix is not an array value"));
                };
                let bs = bounds_of(&a);
                let mut k = 0u64;
                for (b, &i) in bs.iter().zip(indices.iter()) {
                    let x = int(&self.eval(db, i, scope)?, db.exprs[i].span)?;
                    let p = b.pos(x).ok_or_else(|| {
                        fails(
                            db,
                            i,
                            format!("index {x} is outside {} {:?} {}", b.left, b.dir, b.right),
                        )
                    })?;
                    k = k * b.len() + p;
                }
                elem_at(&a, k).ok_or_else(|| fails(db, e, "index outside the value".into()))?
            }
            ExprKind::Slice { prefix, range } => {
                let Val::Array(a) = self.eval(db, *prefix, scope)? else {
                    return Err(not_yet(db, e, "the prefix is not an array value"));
                };
                let r = self.range(db, range, scope, ex.span)?;
                let b = bounds_of(&a)[0];
                if r.is_empty() {
                    return Ok(Some(make(&[r], Vec::new())));
                }
                let (Some(p), Some(q)) = (b.pos(r.left), b.pos(r.right)) else {
                    return Err(fails(
                        db,
                        e,
                        "slice outside the bounds of its prefix".into(),
                    ));
                };
                if r.dir != b.dir {
                    return Err(fails(
                        db,
                        e,
                        "slice direction differs from its prefix".into(),
                    ));
                }
                let xs = elems(&a)[p as usize..=q as usize].to_vec();
                make(&[r], xs)
            }
            ExprKind::Field { prefix, elem } => match self.eval(db, *prefix, scope)? {
                Val::Record(vs) => vs
                    .get(*elem as usize)
                    .cloned()
                    .ok_or_else(|| fails(db, e, "no such record element".into()))?,
                _ => return Err(not_yet(db, e, "the prefix is not a record value")),
            },
            ExprKind::Conv { to, operand } | ExprKind::Qualified { to, operand }
                if is_composite(db, e) =>
            {
                let v = self.eval(db, *operand, scope)?;
                let Val::Array(a) = &v else {
                    return Ok(Some(v));
                };
                // A constrained target gives the bounds (LRM 9.3.6), lengths must match.
                match self.shape(db, *to, scope, ex.span)? {
                    Some(Shape::Array { dims, .. }) => {
                        let have = bounds_of(a);
                        if have.len() != dims.len()
                            || have.iter().zip(&dims).any(|(x, y)| x.len() != y.len())
                        {
                            return Err(fails(db, e, "length mismatch in a conversion".into()));
                        }
                        make(&dims, elems(a))
                    }
                    _ => v,
                }
            }
            ExprKind::Call { callee, args, .. } => {
                let Some(Body::Implicit(op)) = db.decls[*callee].kind.subprogram().map(|s| &s.body)
                else {
                    return Ok(None);
                };
                let xs: Vec<ExprId> = args
                    .iter()
                    .filter_map(|a| match a.actual {
                        Actual::Expr(x) => Some(x),
                        _ => None,
                    })
                    .collect();
                match (op.family, xs.as_slice()) {
                    (Family::Cat, [l, r]) => self.concat(db, e, *l, *r, scope)?,
                    (Family::Eq | Family::Neq, [l, r]) if is_composite(db, *l) => {
                        let (a, b) = (self.eval(db, *l, scope)?, self.eval(db, *r, scope)?);
                        let same = match (&a, &b) {
                            // Equal lengths and elements; bounds do not matter (LRM 9.2.3).
                            (Val::Array(x), Val::Array(y)) => elems(x) == elems(y),
                            _ => a == b,
                        };
                        Val::Enum(u32::from(same == (op.family == Family::Eq)))
                    }
                    (
                        f @ (Family::And
                        | Family::Or
                        | Family::Nand
                        | Family::Nor
                        | Family::Xor
                        | Family::Xnor
                        | Family::Not),
                        xs,
                    ) if is_composite(db, e) && xs.iter().all(|&x| is_composite(db, x)) => {
                        self.logical(db, e, f, xs, scope)?
                    }
                    (
                        Family::Deallocate
                        | Family::FileOpen
                        | Family::FileClose
                        | Family::Read
                        | Family::Write
                        | Family::Endfile
                        | Family::Flush
                        | Family::FileState
                        | Family::FileMode
                        | Family::FilePosition
                        | Family::FileSize
                        | Family::FileSeek
                        | Family::FileTruncate
                        | Family::FileRewind
                        | Family::FileCanseek
                        | Family::RisingEdge
                        | Family::FallingEdge,
                        _,
                    ) => return Ok(None),
                    _ if op.family == Family::ToString
                        || is_composite(db, e)
                        || xs.iter().any(|&x| is_composite(db, x)) =>
                    {
                        self.implicit_call(db, e, *callee, op, args, scope)?
                    }
                    _ => return Ok(None),
                }
            }
            _ => return Ok(None),
        }))
    }

    /// A predefined operation elaboration does not evaluate structurally (shifts, array
    /// ordering, matching operators, `TO_STRING`, ...): its yavs-interp native runs on the
    /// marshalled operands, as in the simulation (#220).
    fn implicit_call(
        &mut self,
        db: &Db,
        e: ExprId,
        callee: yavs_sem::DeclId,
        op: &yavs_sem::implicit::ImplicitOp,
        args: &[yavs_sem::hir::Arg],
        scope: Option<ScopeId>,
    ) -> R<Val> {
        let Some(sp) = db.decls[callee].kind.subprogram() else {
            return Err(not_yet(db, e, "not a subprogram"));
        };
        let Some(ret) = sp.ret else {
            return Err(not_yet(db, e, "a predefined procedure"));
        };
        let mut vals = Vec::with_capacity(sp.params.len());
        for i in 0..sp.params.len() {
            match args
                .iter()
                .find(|a| a.formal as usize == i)
                .map(|a| &a.actual)
            {
                Some(Actual::Expr(x)) => vals.push(self.eval(db, *x, scope)?),
                _ => {
                    return Err(not_yet(
                        db,
                        e,
                        "a predefined operation without all operands",
                    ));
                }
            }
        }
        let params: Vec<_> = sp.params.iter().map(|p| db.base(p.subtype)).collect();
        self.native_call(db, e, vals, &params, db.base(ret), |l| {
            l.implicit_native(op)
        })
    }

    /// `T'image(x)` and `T'value(s)` during elaboration: the attribute natives run on the
    /// marshalled operand, as in the simulation. `None` for any other expression.
    pub(crate) fn attr_text(
        &mut self,
        db: &Db,
        e: ExprId,
        scope: Option<ScopeId>,
    ) -> R<Option<Val>> {
        use yavs_sem::hir::{AttrPrefix, PredefAttr};
        let ExprKind::Attr { attr, prefix, args } = &db.exprs[e].kind else {
            return Ok(None);
        };
        let (name, x, from, to) = match (attr, prefix, args.first()) {
            (PredefAttr::Image, AttrPrefix::Subtype(st), Some(&x)) => (
                "yavs.attr.image",
                x,
                db.base(*st),
                db.base(db.exprs[e].subtype),
            ),
            (PredefAttr::Image, AttrPrefix::Expr(x), None) => (
                "yavs.attr.image",
                *x,
                db.base(db.exprs[*x].subtype),
                db.base(db.exprs[e].subtype),
            ),
            (PredefAttr::Value, AttrPrefix::Subtype(st), Some(&x)) => (
                "yavs.attr.value",
                x,
                db.base(db.exprs[x].subtype),
                db.base(*st),
            ),
            _ => return Ok(None),
        };
        let v = self.eval(db, x, scope)?;
        let v = if name == "yavs.attr.value"
            && matches!(
                db.types[to].kind,
                TypeKind::Array { .. } | TypeKind::Record { .. }
            )
            && let Val::Array(a) = &v
            && let Some((left, dir, right)) = a.bounds.first()
        {
            let mut chars = Vec::new();
            for c in elems(a) {
                let Some(c) = c.as_int().and_then(|c| u8::try_from(c).ok()) else {
                    return Err(not_yet(db, e, "the text of a value attribute"));
                };
                chars.push(c);
            }
            let mut quoted = Vec::with_capacity(chars.len() + 2);
            quoted.push(b'"');
            for c in chars {
                quoted.push(c);
                if c == b'"' {
                    quoted.push(c);
                }
            }
            quoted.push(b'"');
            Val::Array(ArrayConst {
                bounds: Box::new([(*left, *dir, *right)]),
                elems: Elems::Bytes(quoted.into()),
            })
        } else {
            v
        };
        let t = if name == "yavs.attr.image" { from } else { to };
        let r = self.native_call(db, e, vec![v], &[from], to, |l| l.attr_native(name, t))?;
        // 'value: the result must belong to the prefix subtype (LRM 16.2.1).
        if let (PredefAttr::Value, AttrPrefix::Subtype(st)) = (attr, prefix)
            && let Some(x) = r.as_int()
            && db.types[db.base(*st)].kind.is_scalar()
            && !matches!(db.types[db.base(*st)].kind, TypeKind::Floating)
        {
            let (lo, hi) = self
                .scalar_bounds(db, *st, scope, db.exprs[e].span)?
                .low_high();
            if x < lo || x > hi {
                return Err(fails(
                    db,
                    e,
                    format!("'value result {x} is outside the subtype"),
                ));
            }
        }
        Ok(Some(r))
    }

    /// Run `native` on `vals` (of the types `params`) and decode its result of type `ret`.
    fn native_call(
        &self,
        db: &Db,
        e: ExprId,
        vals: Vec<Val>,
        params: &[yavs_sem::TypeId],
        ret: yavs_sem::TypeId,
        native: impl FnOnce(&mut yavs_lower::Lowerer) -> Result<yavs_ir::NativeId, Vec<Diagnostic>>,
    ) -> R<Val> {
        let lowered = (|| {
            let mut l = yavs_lower::Lowerer::new(db, yavs_lower::LowerOptions::default());
            let native = native(&mut l)?;
            let mut tys = Vec::with_capacity(params.len());
            for &p in params {
                tys.push(l.ty(p)?);
            }
            let ret = l.ty(ret)?;
            Ok::<_, Vec<Diagnostic>>((native, tys, ret, l.finish()?.program))
        })();
        let (native, params, ret, prog) = lowered.map_err(|ds| {
            let why = ds.into_iter().next().map(|d| d.message).unwrap_or_default();
            fails(
                db,
                e,
                format!("internal error: lowering the operation: {why}"),
            )
        })?;
        let prog = std::sync::Arc::new(prog);
        let mut cvs = Vec::with_capacity(vals.len());
        for (v, &t) in vals.iter().zip(&params) {
            cvs.push(encode(&prog, t, v).ok_or_else(|| not_yet(db, e, "an operand of this type"))?);
        }
        let composite = prog
            .types
            .get(ret)
            .is_some_and(|t| t.scalar_size().is_none());
        let rt = yavs_interp::Runtime::new(prog.clone(), Default::default())
            .map_err(|x| fails(db, e, format!("internal error: {x}")))?;
        let r = yavs_interp::call_native(rt, native, &cvs, composite.then_some(ret))
            .map_err(|x| {
                fails(
                    db,
                    e,
                    match &x.kind {
                        yavs_kernel::ErrorKind::Runtime(m) => m.clone(),
                        k => k.to_string(),
                    },
                )
            })?
            .ok_or_else(|| fails(db, e, "the operation returned no value".into()))?;
        decode(&prog, ret, &r).ok_or_else(|| not_yet(db, e, "a result of this type"))
    }

    /// Element-wise logical operators on BIT and BOOLEAN arrays (LRM 9.2.2): the result has
    /// the left operand's bounds; operands of different lengths fail.
    fn logical(
        &mut self,
        db: &Db,
        e: ExprId,
        f: Family,
        xs: &[ExprId],
        scope: Option<ScopeId>,
    ) -> R<Val> {
        let mut ops = Vec::with_capacity(xs.len());
        for &x in xs {
            let Val::Array(a) = self.eval(db, x, scope)? else {
                return Err(not_yet(db, e, "a logical operator on a non-array value"));
            };
            ops.push(a);
        }
        let b = bounds_of(&ops[0])[0];
        let vs: Vec<Vec<Val>> = ops.iter().map(elems).collect();
        if vs.iter().any(|v| v.len() != vs[0].len()) {
            return Err(fails(db, e, "operands of different lengths".into()));
        }
        let bit = |v: &Val| match v {
            Val::Enum(p @ (0 | 1)) => Ok(*p == 1),
            _ => Err(not_yet(db, e, "a logical operator on this element type")),
        };
        let mut out = Vec::with_capacity(vs[0].len());
        for k in 0..vs[0].len() {
            let a = bit(&vs[0][k])?;
            let r = match (f, vs.get(1)) {
                (Family::Not, None) => !a,
                (_, Some(w)) => {
                    let c = bit(&w[k])?;
                    match f {
                        Family::And => a && c,
                        Family::Or => a || c,
                        Family::Nand => !(a && c),
                        Family::Nor => !(a || c),
                        Family::Xor => a != c,
                        _ => a == c,
                    }
                }
                _ => return Err(not_yet(db, e, "this logical operator")),
            };
            out.push(Val::Enum(u32::from(r)));
        }
        Ok(make(&[b], out))
    }

    /// The `'left` and direction of index subtype `dim` of the array type of `e`.
    fn index_start(
        &mut self,
        db: &Db,
        e: ExprId,
        dim: usize,
        scope: Option<ScopeId>,
    ) -> R<(i64, Direction)> {
        let TypeKind::Array { indexes, .. } = &db.types[db.base(db.exprs[e].subtype)].kind else {
            return Err(not_yet(db, e, "not an array"));
        };
        let Some(&ix) = indexes.get(dim) else {
            return Err(not_yet(db, e, "an aggregate deeper than its type"));
        };
        let b = self.scalar_bounds(db, ix, scope, db.exprs[e].span)?;
        Ok((b.left, b.dir))
    }

    /// `l & r` (LRM 9.2.5) on one-dimensional arrays and their elements.
    fn concat(
        &mut self,
        db: &Db,
        e: ExprId,
        l: ExprId,
        r: ExprId,
        scope: Option<ScopeId>,
    ) -> R<Val> {
        let rt = db.base(db.exprs[e].subtype);
        let piece = |m: &mut Model, x: ExprId| -> R<(Option<Bounds>, Vec<Val>)> {
            let v = m.eval(db, x, scope)?;
            Ok(match v {
                Val::Array(a) if db.base(db.exprs[x].subtype) == rt => {
                    (Some(bounds_of(&a)[0]), elems(&a))
                }
                v => (None, vec![v]),
            })
        };
        let (lb, mut xs) = piece(self, l)?;
        let (rb, ys) = piece(self, r)?;
        let n = (xs.len() + ys.len()) as u64;
        let b = match (lb, rb) {
            (Some(lb), Some(rb)) if lb.is_empty() => rb,
            (Some(lb), _) if !lb.is_empty() => span(lb.left, lb.dir, n),
            _ => {
                let (left, dir) = self.index_start(db, e, 0, scope)?;
                span(left, dir, n)
            }
        };
        xs.extend(ys);
        Ok(make(&[b], xs))
    }

    fn aggregate(
        &mut self,
        db: &Db,
        e: ExprId,
        assocs: &[AggAssoc],
        scope: Option<ScopeId>,
    ) -> R<Val> {
        let ex = &db.exprs[e];
        match &db.types[db.base(ex.subtype)].kind {
            TypeKind::Record { elems: fs } => {
                let mut vs: Vec<Option<Val>> = vec![None; fs.len()];
                let mut next = 0;
                for a in assocs {
                    let v = self.eval(db, a.value, scope)?;
                    if a.choices.is_empty() {
                        *vs.get_mut(next)
                            .ok_or_else(|| fails(db, e, "too many elements".into()))? = Some(v);
                        next += 1;
                        continue;
                    }
                    for c in a.choices.iter() {
                        match c {
                            Choice::Element(i) => vs[*i as usize] = Some(v.clone()),
                            Choice::Others => vs
                                .iter_mut()
                                .filter(|x| x.is_none())
                                .for_each(|x| *x = Some(v.clone())),
                            _ => return Err(not_yet(db, e, "this record aggregate choice")),
                        }
                    }
                }
                let vs: Option<Vec<Val>> = vs.into_iter().collect();
                Ok(Val::Record(
                    vs.ok_or_else(|| fails(db, e, "a record element has no value".into()))?
                        .into(),
                ))
            }
            TypeKind::Array { .. } => {
                let (bs, xs) = self.array_agg(db, e, 0, scope)?;
                Ok(make(&bs, xs))
            }
            _ => Err(not_yet(db, e, "an aggregate of this type")),
        }
    }

    /// The (sub-)aggregate `e` for dimension `dim` of an array aggregate (LRM 9.3.3.3): the
    /// bounds of this and the deeper dimensions and the elements in row-major order. A
    /// sub-aggregate's subtype is the outermost aggregate's (yavs-sem); its dimension is its
    /// nesting depth, and the last one may be a string literal.
    fn array_agg(
        &mut self,
        db: &Db,
        e: ExprId,
        dim: usize,
        scope: Option<ScopeId>,
    ) -> R<(Vec<Bounds>, Vec<Val>)> {
        let ex = &db.exprs[e];
        let TypeKind::Array { indexes, .. } = &db.types[db.base(ex.subtype)].kind else {
            return Err(not_yet(db, e, "not an array"));
        };
        let last = dim + 1 == indexes.len();
        let (left, dir) = self.index_start(db, e, dim, scope)?;
        let ExprKind::Aggregate { assocs, bounds } = &ex.kind else {
            // A string literal (or another folded value) as the last dimension's row.
            let Val::Array(a) = self.eval(db, e, scope)? else {
                return Err(not_yet(db, e, "this row of a multi-dimensional aggregate"));
            };
            let xs = elems(&a);
            return Ok((vec![span(left, dir, xs.len() as u64)], xs));
        };
        // Items: one element each for the last dimension (slice pieces spliced in), one row
        // of the deeper dimensions each otherwise.
        let mut deeper: Option<Vec<Bounds>> = None;
        let mut pos: Vec<Vec<Val>> = Vec::new();
        let mut named: Vec<(&Choice, Vec<Val>)> = Vec::new();
        for a in assocs.iter() {
            let item = if last {
                match self.eval(db, a.value, scope)? {
                    Val::Array(x) if a.slice_piece => {
                        if !a.choices.is_empty() {
                            return Err(not_yet(db, e, "a named slice in an aggregate"));
                        }
                        pos.extend(elems(&x).into_iter().map(|v| vec![v]));
                        continue;
                    }
                    v => vec![v],
                }
            } else {
                let (bs, row) = self.array_agg(db, a.value, dim + 1, scope)?;
                match &deeper {
                    Some(d) if d.iter().zip(&bs).any(|(x, y)| x.len() != y.len()) => {
                        return Err(fails(
                            db,
                            a.value,
                            "the sub-aggregates of a multi-dimensional aggregate differ in length"
                                .into(),
                        ));
                    }
                    Some(_) => {}
                    None => deeper = Some(bs),
                }
                row
            };
            if a.choices.is_empty() {
                pos.push(item);
            } else {
                for c in a.choices.iter() {
                    named.push((c, item.clone()));
                }
            }
        }
        let b = match bounds {
            AggBounds::Context => match self.shape(db, ex.subtype, scope, ex.span)? {
                Some(Shape::Array { dims, .. }) if dim < dims.len() => dims[dim],
                _ => return Err(not_yet(db, e, "an aggregate without known bounds")),
            },
            AggBounds::Positional { .. } | AggBounds::Concat => span(left, dir, pos.len() as u64),
            AggBounds::Named { low, high } => {
                let lo = int(&self.eval(db, *low, scope)?, ex.span)?;
                let hi = int(&self.eval(db, *high, scope)?, ex.span)?;
                // Sem's result subtype has the direction of a constrained context (LRM
                // 9.3.3.3), not the index subtype's: `(0 => g0, 1 => g1)` for `(1 downto 0)`.
                let dir = match &db.subtypes[ex.subtype].constraint {
                    Constraint::Array { dims: Some(ds), .. } if dim < ds.len() => {
                        self.range(db, &ds[dim], scope, ex.span)?.dir
                    }
                    _ => dir,
                };
                match dir {
                    Direction::To => Bounds {
                        left: lo,
                        dir,
                        right: hi,
                    },
                    Direction::Downto => Bounds {
                        left: hi,
                        dir,
                        right: lo,
                    },
                }
            }
            AggBounds::Record => {
                return Err(not_yet(db, e, "a record aggregate of an array"));
            }
        };
        let n = b.len() as usize;
        let mut xs: Vec<Option<Vec<Val>>> = vec![None; n];
        if pos.len() > n {
            return Err(fails(
                db,
                e,
                format!("{} elements for {n} positions", pos.len()),
            ));
        }
        for (k, v) in pos.into_iter().enumerate() {
            xs[k] = Some(v);
        }
        for (c, v) in named {
            match c {
                Choice::Expr(x) => {
                    let i = int(&self.eval(db, *x, scope)?, db.exprs[*x].span)?;
                    let p = b.pos(i).ok_or_else(|| {
                        fails(
                            db,
                            *x,
                            format!("choice {i} is outside the aggregate's bounds"),
                        )
                    })?;
                    xs[p as usize] = Some(v);
                }
                Choice::Range(r) => {
                    let r = self.range(db, r, scope, ex.span)?;
                    let (lo, hi) = r.low_high();
                    for i in lo..=hi {
                        let p = b.pos(i).ok_or_else(|| {
                            fails(
                                db,
                                e,
                                format!("choice {i} is outside the aggregate's bounds"),
                            )
                        })?;
                        xs[p as usize] = Some(v.clone());
                    }
                }
                Choice::Others => xs
                    .iter_mut()
                    .filter(|x| x.is_none())
                    .for_each(|x| *x = Some(v.clone())),
                Choice::Element(_) => {
                    return Err(not_yet(db, e, "an element choice in an array aggregate"));
                }
            }
        }
        let xs: Option<Vec<Vec<Val>>> = xs.into_iter().collect();
        let xs = xs.ok_or_else(|| fails(db, e, "an aggregate element has no value".into()))?;
        let mut bs = vec![b];
        if !last {
            match deeper {
                Some(d) => bs.extend(d),
                // ponytail: a null outer dimension has no row to take the deeper bounds
                // from; a constrained context would give them.
                None => return Err(not_yet(db, e, "a null multi-dimensional aggregate")),
            }
        }
        Ok((bs, xs.into_iter().flatten().collect()))
    }
}
