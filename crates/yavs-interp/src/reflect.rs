//! `std.reflection` (LRM 2019 16.11): mirror objects and the 123 mirror methods.
//!
//! `x'reflect` and `T'reflect` call `yavs.attr.reflect (refl, is_value, v, ranges)` with the
//! [`yavs_ir::Refl`] entry of the subtype: `v` is the scalar cell, the address of a composite's
//! fat pair `[data, descriptor]`, or for a subtype mirror the address of the array subtype's
//! descriptor (0 when unconstrained or not an array); `ranges` is 0 or the address of
//! `[count][refl, left, right, dir]...`, the run-time ranges of the reachable scalar subtypes
//! whose range is not static (their entries hold the type's range).
//!
//! A mirror is a private-heap object (`Obj::Mirror`, generational handle) holding a copy of
//! what it reflects: `[refl: u32][flags: u32][desc_len: u64][descriptor][value][ranges]` (see
//! `encode`). The value
//! of a scalar is its 8-byte cell; a composite keeps its IR layout, so descriptor and value
//! are the `[descriptor][elements]` form of a heap array and the GC scans them like one. Every
//! mirror access type designates such an object: `value_mirror.to_integer` returns the same
//! handle as an `integer_value_mirror`, so the class conversions allocate nothing. Methods
//! receive the handle as their first argument (`self`).

use yavs_ir::{ConstData, ConstId, LogicKind, Program, Refl, ReflId, ReflKind, Ty, TyId, Width};
use yavs_kernel::Ctx;
use yavs_std::image;
use yavs_std::types::{EnumLit, PhysicalType};

use crate::exec::{Fault, Machine, R, add_addr, ice, rt_err};
use crate::heap::{HeapError, Obj};
use crate::native::NativeFn;

const HDR: usize = 16;

/// `std.reflection.TYPE_CLASS` positions.
const CLASS_NAMES: [&str; 9] = [
    "enumeration",
    "integer",
    "floating",
    "physical",
    "record",
    "array",
    "access",
    "file",
    "protected",
];

fn class(k: &ReflKind) -> u64 {
    match k {
        ReflKind::Enum { .. } => 0,
        ReflKind::Int { .. } => 1,
        ReflKind::Float { .. } => 2,
        ReflKind::Physical { .. } => 3,
        ReflKind::Record { .. } => 4,
        ReflKind::Array { .. } => 5,
        ReflKind::Access { .. } => 6,
        ReflKind::File { .. } => 7,
        ReflKind::Protected => 8,
    }
}

struct Mirror {
    refl: ReflId,
    value: bool,
    desc: Vec<u8>,
    val: Vec<u8>,
    /// Run-time ranges of non-static scalar subtypes, `[refl, left, right, dir]` per entry,
    /// passed on to every mirror made from this one.
    ranges: Vec<u8>,
}

/// One `ranges` entry.
const RANGE: usize = 32;

pub(crate) fn rd64(b: &[u8], at: usize) -> u64 {
    let mut a = [0u8; 8];
    if let Some(s) = b.get(at..at + 8) {
        a.copy_from_slice(s);
    }
    u64::from_le_bytes(a)
}

/// A cell of up to 8 bytes (4-byte cells sign-extended: [`crate::exec::cell`]).
pub(crate) fn le(b: &[u8]) -> u64 {
    crate::exec::cell(b)
}

fn refl(prog: &Program, id: ReflId) -> R<&Refl> {
    prog.reflect
        .get(id)
        .ok_or_else(|| ice(format!("no reflection entry {id}")))
}

pub(crate) fn layout_err(e: yavs_ir::LayoutError) -> Fault {
    match e {
        yavs_ir::LayoutError::TooLarge => Fault::Runtime(e.to_string()),
        yavs_ir::LayoutError::Malformed(_) => ice(e.to_string()),
    }
}

pub(crate) fn const_bytes(prog: &Program, c: ConstId) -> R<Vec<u8>> {
    match prog.consts.get(c) {
        Some(ConstData::Bytes(b)) => Ok(b.clone()),
        _ => Err(ice(format!(
            "element descriptor {c} is not a byte constant"
        ))),
    }
}

fn alloc(
    m: &mut Machine,
    id: ReflId,
    value: bool,
    desc: &[u8],
    val: &[u8],
    ranges: &[u8],
) -> R<u64> {
    let d = encode(id, value, desc, val, ranges)?;
    m.alloc_heap(false)?
        .alloc(Obj::Mirror, d)
        .map_err(|_| Fault::Runtime("heap object limit (16M objects per heap) exceeded".into()))
}

/// A mirror object's bytes: `[refl: u32][flags: u32][desc_len: u64][descriptor][value]
/// [ranges]`; flags bit 0 is set for a value mirror, the rest count the `ranges` entries.
fn encode(id: ReflId, value: bool, desc: &[u8], val: &[u8], ranges: &[u8]) -> R<Vec<u8>> {
    let mut d = Vec::with_capacity(HDR + desc.len() + val.len() + ranges.len());
    d.extend(id.0.to_le_bytes());
    let flags = u32::try_from(ranges.len() / RANGE * 2)
        .map_err(|_| ice("too many run-time ranges in a mirror"))?;
    d.extend((flags | u32::from(value)).to_le_bytes());
    d.extend((desc.len() as u64).to_le_bytes());
    d.extend_from_slice(desc);
    d.extend_from_slice(val);
    d.extend_from_slice(ranges);
    Ok(d)
}

fn parse(d: &[u8]) -> Option<Mirror> {
    let id = u32::try_from(le(d.get(0..4)?)).ok()?;
    let flags = le(d.get(4..8)?);
    let n = usize::try_from(rd64(d, 8)).ok()?;
    let r = usize::try_from(flags >> 1).ok()?.checked_mul(RANGE)?;
    let end = d.len().checked_sub(r)?;
    let desc = d.get(HDR..HDR + n)?.to_vec();
    let val = d.get(HDR + n..end)?.to_vec();
    Some(Mirror {
        refl: ReflId(id),
        value: flags & 1 != 0,
        desc,
        val,
        ranges: d[end..].to_vec(),
    })
}

fn get(m: &mut Machine, h: u64) -> R<Mirror> {
    if h == 0 {
        return rt_err("null access value dereferenced: a mirror method was called through null");
    }
    let (hp, _) = m.heap_of(h)?;
    let i = match hp.resolve(h) {
        Ok(i) => i,
        Err(HeapError::Stale) => {
            return rt_err("access value designates a mirror that was deallocated");
        }
        Err(e) => return Err(ice(format!("mirror handle: {e:?}"))),
    };
    if hp.kind(i) != Some(Obj::Mirror) {
        return Err(ice(
            "mirror method called on an object that is not a mirror",
        ));
    }
    hp.data(i)
        .and_then(parse)
        .ok_or_else(|| ice("malformed mirror object"))
}

/// Handles inside a mirror object, for the GC (`heap::Heap::collect`).
pub(crate) fn mirror_handles(prog: &Program, data: &[u8], out: &mut Vec<u64>) -> bool {
    let Some(mi) = parse(data) else {
        return false;
    };
    if !mi.value {
        return true;
    }
    match prog.reflect.get(mi.refl).map(|r| &r.kind) {
        Some(ReflKind::Access { .. } | ReflKind::Protected) => {
            out.push(rd64(&mi.val, 0));
            true
        }
        Some(ReflKind::Array { ty, .. }) => {
            crate::heap::object_handles(prog, *ty, &data[HDR..data.len() - mi.ranges.len()], out)
        }
        Some(ReflKind::Record { ty, .. }) => crate::heap::value_handles(prog, *ty, &mi.val, out),
        Some(_) => true,
        None => false,
    }
}

// ---- values and elements ----

/// Bytes of a scalar of this subtype in memory (record fields, array elements).
fn scalar_width(prog: &Program, k: &ReflKind) -> R<Option<usize>> {
    Ok(Some(match k {
        ReflKind::Enum { ty, .. } => match prog.types.get(*ty) {
            Some(Ty::Enum { lits, .. }) if lits.len() <= 256 => 1,
            Some(Ty::Enum { .. }) => 4,
            _ => return Err(ice(format!("enumeration mirror of non-enumeration {ty}"))),
        },
        ReflKind::File { .. } => 4,
        ReflKind::Array { .. } | ReflKind::Record { .. } => return Ok(None),
        _ => 8,
    }))
}

pub(crate) fn plane_get(v: &[u8], i: u64) -> u64 {
    let b = (i / 64) as usize * 32;
    let bit = i % 64;
    (0..4).map(|p| ((rd64(v, b + p * 8) >> bit) & 1) << p).sum()
}

pub(crate) fn plane_set(v: &mut [u8], i: u64, code: u64) {
    let b = (i / 64) as usize * 32;
    let bit = i % 64;
    for p in 0..4 {
        let at = b + p * 8;
        let w = (rd64(v, at) & !(1 << bit)) | (((code >> p) & 1) << bit);
        if let Some(s) = v.get_mut(at..at + 8) {
            s.copy_from_slice(&w.to_le_bytes());
        }
    }
}

/// Position of a logic element from its plane code (BIT and BOOLEAN store the codes of '0'
/// and '1'; positions 0 and 1 are accepted too, as `bits_at` does).
pub(crate) fn logic_pos(k: LogicKind, code: u64) -> u64 {
    match k {
        LogicKind::StdULogic => code,
        _ if code >= 2 => code - 2,
        _ => code,
    }
}

/// `(left, right, downto)` per dimension of an array descriptor.
fn dims(desc: &[u8], n: usize) -> R<Vec<(i64, i64, bool)>> {
    if desc.len() < n * 24 {
        return rt_err("the array subtype is unconstrained: its mirror has no index bounds");
    }
    Ok((0..n)
        .map(|d| {
            (
                rd64(desc, d * 24) as i64,
                rd64(desc, d * 24 + 8) as i64,
                rd64(desc, d * 24 + 16) == 1,
            )
        })
        .collect())
}

fn dim_count(l: i64, r: i64, downto: bool) -> R<u64> {
    yavs_ir::dim_len(l, r, downto).ok_or_else(|| Fault::Runtime("array too long".into()))
}

/// Element `k` (row major, 0 = leftmost) of an array value: its descriptor and value.
fn array_elem(
    prog: &Program,
    ty: TyId,
    elem: &Refl,
    desc: &[u8],
    val: &[u8],
    k: u64,
) -> R<(Vec<u8>, Vec<u8>)> {
    let (types, consts) = (&prog.types, &prog.consts);
    let Some(Ty::Array {
        elem: et,
        elem_desc,
        dims: nd,
    }) = types.get(ty)
    else {
        return Err(ice(format!("array mirror of non-array {ty}")));
    };
    let (lay, _) = Ty::layout(types, consts, ty, desc).map_err(layout_err)?;
    let edesc = match elem_desc {
        Some(c) => const_bytes(prog, *c)?,
        None => desc.get(usize::from(*nd) * 24..).unwrap_or(&[]).to_vec(),
    };
    let scalar = scalar_width(prog, &elem.kind)?;
    if let Some(kind) = Ty::logic_leaf(types, ty) {
        if scalar.is_some() {
            let code = plane_get(val, lay.count - 1 - k);
            return Ok((Vec::new(), logic_pos(kind, code).to_le_bytes().to_vec()));
        }
        let (el, _) = Ty::layout(types, consts, *et, &edesc).map_err(layout_err)?;
        let start = lay.count - (k + 1) * el.count;
        let mut out = vec![0u8; el.size as usize];
        for j in 0..el.count {
            plane_set(&mut out, j, plane_get(val, start + j));
        }
        return Ok((edesc, out));
    }
    let at = (k * lay.stride) as usize;
    // A scalar element may be narrower than the scalar (`Ty::elem_size`).
    if let Some(w) = scalar.map(|w| {
        types
            .get(*et)
            .and_then(Ty::elem_size)
            .map_or(w, |n| n as usize)
    }) {
        let b = val
            .get(at..at + w)
            .ok_or_else(|| ice("array element out of its value"))?;
        return Ok((Vec::new(), le(b).to_le_bytes().to_vec()));
    }
    let (el, _) = Ty::layout(types, consts, *et, &edesc).map_err(layout_err)?;
    let b = val
        .get(at..at + el.size as usize)
        .ok_or_else(|| ice("array element out of its value"))?;
    Ok((edesc, b.to_vec()))
}

/// Element `i` of a record value: its descriptor and value.
fn record_elem(
    prog: &Program,
    ty: TyId,
    elem: &Refl,
    desc: &[u8],
    val: &[u8],
    i: usize,
) -> R<(Vec<u8>, Vec<u8>)> {
    let (types, consts) = (&prog.types, &prog.consts);
    let Some(Ty::Record { fields }) = types.get(ty) else {
        return Err(ice(format!("record mirror of non-record {ty}")));
    };
    let (lay, _) = Ty::layout(types, consts, ty, desc).map_err(layout_err)?;
    let mut cur = 0;
    let mut fdesc = Vec::new();
    for (j, f) in fields.iter().enumerate().take(i + 1) {
        let d = match f.desc {
            Some(c) => const_bytes(prog, c)?,
            None => {
                let n =
                    Ty::desc_size(types, f.ty).ok_or_else(|| ice("record field type"))? as usize;
                let d = desc.get(cur..cur + n).unwrap_or(&[]).to_vec();
                cur += n;
                d
            }
        };
        if j == i {
            fdesc = d;
        }
    }
    let f = fields.get(i).ok_or_else(|| ice("record element index"))?;
    let at = *lay
        .offsets
        .get(i)
        .ok_or_else(|| ice("record field offset"))? as usize;
    if let Some(w) = scalar_width(prog, &elem.kind)? {
        let b = val
            .get(at..at + w)
            .ok_or_else(|| ice("record field out of its value"))?;
        return Ok((Vec::new(), le(b).to_le_bytes().to_vec()));
    }
    let (fl, _) = Ty::layout(types, consts, f.ty, &fdesc).map_err(layout_err)?;
    let b = val
        .get(at..at + fl.size as usize)
        .ok_or_else(|| ice("record field out of its value"))?;
    Ok((fdesc, b.to_vec()))
}

// ---- the attribute ----

fn reflect_attr(m: &mut Machine, a: &[u64]) -> R<u64> {
    let id = ReflId(arg(a, 0)? as u32);
    let value = arg(a, 1)? != 0;
    let v = arg(a, 2)?;
    let ranges = match arg(a, 3)? {
        0 => Vec::new(),
        p => {
            let n = m.load(p, Width::B8)?;
            let bytes = n
                .checked_mul(RANGE as u64)
                .ok_or_else(|| ice("run-time range count"))?;
            m.rd(add_addr(p, 8)?, bytes)?.to_vec()
        }
    };
    let rt = m.rt.clone();
    let prog = &rt.prog;
    let r = refl(prog, id)?;
    let (types, consts) = (&prog.types, &prog.consts);
    let (desc, val) = match (&r.kind, value) {
        (ReflKind::Array { ty, .. } | ReflKind::Record { ty, .. }, true) => {
            let data = m.load(v, Width::B8)?;
            let dp = m.load(add_addr(v, 8)?, Width::B8)?;
            let n = Ty::desc_size(types, *ty).ok_or_else(|| ice("descriptor size"))?;
            let desc = if n == 0 {
                Vec::new()
            } else {
                m.rd(dp, n)?.to_vec()
            };
            let (lay, _) = Ty::layout(types, consts, *ty, &desc).map_err(layout_err)?;
            (desc, m.rd(data, lay.size)?.to_vec())
        }
        (ReflKind::Array { ty, .. }, false) if v != 0 => {
            let n = Ty::desc_size(types, *ty).ok_or_else(|| ice("array descriptor size"))?;
            (m.rd(v, n)?.to_vec(), Vec::new())
        }
        (_, true) => (Vec::new(), v.to_le_bytes().to_vec()),
        (_, false) => (Vec::new(), Vec::new()),
    };
    alloc(m, id, value, &desc, &val, &ranges)
}

// ---- method helpers ----

fn arg(a: &[u64], i: usize) -> R<u64> {
    a.get(i)
        .copied()
        .ok_or_else(|| ice(format!("mirror method argument {i} missing")))
}

/// `self` with its reflection entry.
fn this(m: &mut Machine, a: &[u64]) -> R<(Mirror, Refl)> {
    let mi = get(m, arg(a, 0)?)?;
    let r = refl(&m.rt.prog, mi.refl)?.clone();
    Ok((mi, r))
}

fn cell(mi: &Mirror) -> u64 {
    rd64(&mi.val, 0)
}

fn same(_: &mut Machine, a: &[u64]) -> R<u64> {
    arg(a, 0)
}

fn subtype_of(m: &mut Machine, a: &[u64]) -> R<u64> {
    let mi = get(m, arg(a, 0)?)?;
    alloc(m, mi.refl, false, &mi.desc, &[], &mi.ranges)
}

fn simple_name(m: &mut Machine, a: &[u64]) -> R<u64> {
    let (_, r) = this(m, a)?;
    m.ret_string(r.name.as_bytes())
}

fn class_of(m: &mut Machine, a: &[u64]) -> R<u64> {
    let (_, r) = this(m, a)?;
    Ok(class(&r.kind))
}

/// `value_mirror.to_<class>` and `subtype_mirror.to_<class>`: `self` when the class matches.
fn to_class(m: &mut Machine, a: &[u64], want: u64) -> R<u64> {
    let (mi, r) = this(m, a)?;
    let c = class(&r.kind);
    if c != want {
        let what = if mi.value { "value" } else { "subtype" };
        return rt_err(format!(
            "to_{} called on a {what} mirror of class {} ({})",
            CLASS_NAMES[want as usize], CLASS_NAMES[c as usize], r.name
        ));
    }
    arg(a, 0)
}

fn bool_cell(b: bool) -> u64 {
    u64::from(b)
}

fn lit_of(s: &str) -> EnumLit {
    let b = s.as_bytes();
    if b.len() == 3 && b[0] == b'\'' && b[2] == b'\'' {
        EnumLit::Char(b[1])
    } else {
        EnumLit::ident(s)
    }
}

fn units(prog: &Program, ty: TyId) -> R<&[(String, i64)]> {
    match prog.types.get(ty) {
        Some(Ty::Physical { units, .. }) => Ok(units),
        _ => Err(ice(format!("physical mirror of non-physical {ty}"))),
    }
}

fn lits(prog: &Program, ty: TyId) -> R<&[String]> {
    match prog.types.get(ty) {
        Some(Ty::Enum { lits, .. }) => Ok(lits),
        _ => Err(ice(format!("enumeration mirror of non-enumeration {ty}"))),
    }
}

fn image_of(prog: &Program, r: &Refl, v: u64) -> R<Vec<u8>> {
    Ok(match &r.kind {
        ReflKind::Int { .. } => image::integer_image(v as i64),
        ReflKind::Float { .. } => image::real_image(f64::from_bits(v)),
        ReflKind::Physical { ty, .. } => {
            let u: Vec<(&str, i64)> = units(prog, *ty)?
                .iter()
                .map(|(n, s)| (n.as_str(), *s))
                .collect();
            // sim1's unit case (docs/decisions.md): `5000000 FS`.
            image::physical_image(v as i64, &PhysicalType::new(&u)).to_ascii_uppercase()
        }
        ReflKind::Enum { ty, .. } => {
            let l = lits(prog, *ty)?
                .get(v as usize)
                .ok_or_else(|| ice(format!("enumeration position {v}")))?;
            image::enum_image(&lit_of(l))
        }
        _ => return Err(ice("image of a composite mirror")),
    })
}

fn image(m: &mut Machine, a: &[u64]) -> R<u64> {
    let (mi, r) = this(m, a)?;
    let s = image_of(&m.rt.prog, &r, cell(&mi))?;
    m.ret_string(&s)
}

fn value(m: &mut Machine, a: &[u64]) -> R<u64> {
    let (mi, _) = this(m, a)?;
    Ok(cell(&mi))
}

/// `(left, right, downto)` of the scalar subtype of mirror `mi`, as cells: its run-time range
/// when it has one, else its entry's.
fn scalar_range(mi: &Mirror, r: &Refl) -> R<(u64, u64, bool)> {
    let (entries, _) = mi.ranges.as_chunks::<RANGE>();
    if let Some(e) = entries.iter().find(|e| rd64(*e, 0) == u64::from(mi.refl.0)) {
        return Ok((rd64(e, 8), rd64(e, 16), rd64(e, 24) == 1));
    }
    Ok(match r.kind {
        ReflKind::Enum {
            left,
            right,
            downto,
            ..
        }
        | ReflKind::Int {
            left,
            right,
            downto,
        }
        | ReflKind::Physical {
            left,
            right,
            downto,
            ..
        } => (left as u64, right as u64, downto),
        ReflKind::Float {
            left,
            right,
            downto,
        } => (left.to_bits(), right.to_bits(), downto),
        _ => return Err(ice("range of a non-scalar mirror")),
    })
}

#[derive(Clone, Copy)]
enum Bound {
    Left,
    Right,
    Low,
    High,
}

fn bound_cell(mi: &Mirror, r: &Refl, b: Bound) -> R<u64> {
    let (l, rr, downto) = scalar_range(mi, r)?;
    Ok(match (b, downto) {
        (Bound::Left, _) => l,
        (Bound::Right, _) => rr,
        (Bound::Low, false) | (Bound::High, true) => l,
        (Bound::Low, true) | (Bound::High, false) => rr,
    })
}

/// `left`, `right`, `low`, `high` of a scalar subtype mirror: a value mirror.
fn bound(m: &mut Machine, a: &[u64], b: Bound) -> R<u64> {
    let (mi, r) = this(m, a)?;
    let v = bound_cell(&mi, &r, b)?;
    alloc(m, mi.refl, true, &[], &v.to_le_bytes(), &mi.ranges)
}

fn ascending(m: &mut Machine, a: &[u64]) -> R<u64> {
    let (mi, r) = this(m, a)?;
    Ok(bool_cell(!scalar_range(&mi, &r)?.2))
}

/// Number of values of a discrete (or physical) subtype.
fn length(m: &mut Machine, a: &[u64]) -> R<u64> {
    let (mi, r) = this(m, a)?;
    let lo = bound_cell(&mi, &r, Bound::Low)? as i64;
    let hi = bound_cell(&mi, &r, Bound::High)? as i64;
    if hi < lo {
        return Ok(0);
    }
    hi.checked_sub(lo)
        .and_then(|d| d.checked_add(1))
        .map(|n| n as u64)
        .ok_or_else(|| Fault::Runtime(format!("length of {} exceeds INDEX'high", r.name)))
}

// ---- enumeration ----

fn enum_literal_idx(m: &mut Machine, a: &[u64]) -> R<u64> {
    let (mi, r) = this(m, a)?;
    let idx = arg(a, 1)? as i64;
    let lo = bound_cell(&mi, &r, Bound::Low)? as i64;
    let hi = bound_cell(&mi, &r, Bound::High)? as i64;
    let pos = lo.checked_add(idx).filter(|&p| idx >= 0 && p <= hi);
    let Some(pos) = pos else {
        return rt_err(format!(
            "enumeration_literal({idx}): {} has {} literals",
            r.name,
            (hi - lo + 1).max(0)
        ));
    };
    alloc(
        m,
        mi.refl,
        true,
        &[],
        &(pos as u64).to_le_bytes(),
        &mi.ranges,
    )
}

fn enum_literal_name(m: &mut Machine, a: &[u64]) -> R<u64> {
    let (mi, r) = this(m, a)?;
    let name = m.fat_bytes(arg(a, 1)?)?;
    let ReflKind::Enum { ty, .. } = r.kind else {
        return Err(ice("enumeration method on another class"));
    };
    let lo = bound_cell(&mi, &r, Bound::Low)? as usize;
    let hi = bound_cell(&mi, &r, Bound::High)? as i64;
    let rt = m.rt.clone();
    let ls = lits(&rt.prog, ty)?;
    let pos = (lo..ls.len())
        .take_while(|&p| p as i64 <= hi)
        .find(|&p| lit_matches(&ls[p], &name));
    match pos {
        Some(p) => alloc(m, mi.refl, true, &[], &(p as u64).to_le_bytes(), &mi.ranges),
        None => rt_err(format!(
            "enumeration_literal(\"{}\"): no such literal in {}",
            String::from_utf8_lossy(&name),
            r.name
        )),
    }
}

fn lit_matches(lit: &str, name: &[u8]) -> bool {
    if lit.starts_with('\'') || lit.starts_with('\\') {
        lit.as_bytes() == name
    } else {
        lit.as_bytes().eq_ignore_ascii_case(name)
    }
}

// ---- physical ----

/// The unit the value of a TIME mirror is expressed in: the resolution limit (decisions.md).
fn phys_unit(m: &Machine, r: &Refl) -> R<(usize, i64)> {
    let ReflKind::Physical { ty, time, .. } = r.kind else {
        return Err(ice("physical method on another class"));
    };
    if !time {
        return Ok((1, 1));
    }
    let res = yavs_std::env::resolution_limit(m.rt.resolution);
    time_unit(units(&m.rt.prog, ty)?, res)
}

/// The 1-based index and scale of the TIME unit equal to the resolution limit `res` (fs).
fn time_unit(units: &[(String, i64)], res: i64) -> R<(usize, i64)> {
    let i = units
        .iter()
        .position(|u| u.1 == res)
        .ok_or_else(|| ice(format!("no TIME unit of {res} fs")))?;
    Ok((i + 1, res))
}

fn phys_value(m: &mut Machine, a: &[u64]) -> R<u64> {
    let (mi, r) = this(m, a)?;
    let (_, scale) = phys_unit(m, &r)?;
    Ok(((cell(&mi) as i64) / scale) as u64)
}

fn phys_unit_index(m: &mut Machine, a: &[u64]) -> R<u64> {
    let (_, r) = this(m, a)?;
    Ok(phys_unit(m, &r)?.0 as u64)
}

fn phys_units(m: &Machine, r: &Refl) -> R<Vec<(String, i64)>> {
    let ReflKind::Physical { ty, .. } = r.kind else {
        return Err(ice("physical method on another class"));
    };
    Ok(units(&m.rt.prog, ty)?.to_vec())
}

fn unit_at(m: &mut Machine, a: &[u64]) -> R<(String, i64)> {
    let (_, r) = this(m, a)?;
    let us = phys_units(m, &r)?;
    let i = arg(a, 1)? as i64;
    usize::try_from(i - 1)
        .ok()
        .and_then(|i| us.get(i).cloned())
        .ok_or_else(|| {
            Fault::Runtime(format!(
                "unit index {i} outside of 1 to {} for {}",
                us.len(),
                r.name
            ))
        })
}

fn unit_named(m: &mut Machine, a: &[u64]) -> R<(usize, i64)> {
    let (_, r) = this(m, a)?;
    let us = phys_units(m, &r)?;
    let name = m.fat_bytes(arg(a, 1)?)?;
    us.iter()
        .position(|u| u.0.as_bytes().eq_ignore_ascii_case(&name))
        .map(|i| (i + 1, us[i].1))
        .ok_or_else(|| {
            Fault::Runtime(format!(
                "\"{}\" is not a unit of {}",
                String::from_utf8_lossy(&name),
                r.name
            ))
        })
}

fn units_length(m: &mut Machine, a: &[u64]) -> R<u64> {
    let (_, r) = this(m, a)?;
    Ok(phys_units(m, &r)?.len() as u64)
}

fn unit_name(m: &mut Machine, a: &[u64]) -> R<u64> {
    let (n, _) = unit_at(m, a)?;
    m.ret_string(n.to_ascii_uppercase().as_bytes())
}

fn unit_index(m: &mut Machine, a: &[u64]) -> R<u64> {
    Ok(unit_named(m, a)?.0 as u64)
}

fn scale_idx(m: &mut Machine, a: &[u64]) -> R<u64> {
    Ok(unit_at(m, a)?.1 as u64)
}

fn scale_name(m: &mut Machine, a: &[u64]) -> R<u64> {
    Ok(unit_named(m, a)?.1 as u64)
}

// ---- records ----

fn record_parts(r: &Refl) -> R<(TyId, &[(String, ReflId)])> {
    match &r.kind {
        ReflKind::Record { ty, elems } => Ok((*ty, elems)),
        _ => Err(ice("record method on another class")),
    }
}

fn elem_index(m: &Machine, r: &Refl, a: &[u64], by_name: bool) -> R<usize> {
    let (_, elems) = record_parts(r)?;
    if by_name {
        let name = m.fat_bytes(arg(a, 1)?)?;
        return elems
            .iter()
            .position(|e| e.0.as_bytes().eq_ignore_ascii_case(&name))
            .ok_or_else(|| {
                Fault::Runtime(format!(
                    "\"{}\" is not an element of {}",
                    String::from_utf8_lossy(&name),
                    r.name
                ))
            });
    }
    let i = arg(a, 1)? as i64;
    usize::try_from(i)
        .ok()
        .filter(|&i| i < elems.len())
        .ok_or_else(|| {
            Fault::Runtime(format!(
                "element index {i} outside of 0 to {} for {}",
                elems.len() as i64 - 1,
                r.name
            ))
        })
}

fn record_get(m: &mut Machine, a: &[u64], by_name: bool) -> R<u64> {
    let (mi, r) = this(m, a)?;
    let i = elem_index(m, &r, a, by_name)?;
    let (ty, elems) = record_parts(&r)?;
    let eid = elems[i].1;
    let rt = m.rt.clone();
    let er = refl(&rt.prog, eid)?;
    let (d, v) = record_elem(&rt.prog, ty, er, &mi.desc, &mi.val, i)?;
    alloc(m, eid, true, &d, &v, &mi.ranges)
}

fn record_length(m: &mut Machine, a: &[u64]) -> R<u64> {
    let (_, r) = this(m, a)?;
    Ok(record_parts(&r)?.1.len() as u64)
}

fn element_name(m: &mut Machine, a: &[u64]) -> R<u64> {
    let (_, r) = this(m, a)?;
    let i = elem_index(m, &r, a, false)?;
    let n = record_parts(&r)?.1[i].0.clone();
    m.ret_string(n.as_bytes())
}

fn element_index(m: &mut Machine, a: &[u64]) -> R<u64> {
    let (_, r) = this(m, a)?;
    Ok(elem_index(m, &r, a, true)? as u64)
}

/// The element's subtype mirror; its constraint is the element's part of the descriptor.
fn record_elem_subtype(m: &mut Machine, a: &[u64], by_name: bool) -> R<u64> {
    let (mi, r) = this(m, a)?;
    let i = elem_index(m, &r, a, by_name)?;
    let (ty, elems) = record_parts(&r)?;
    let eid = elems[i].1;
    let rt = m.rt.clone();
    let prog = &rt.prog;
    let Some(Ty::Record { fields }) = prog.types.get(ty) else {
        return Err(ice("record mirror of a non-record type"));
    };
    let mut cur = 0;
    let mut d = Vec::new();
    for (j, f) in fields.iter().enumerate().take(i + 1) {
        let fd = match f.desc {
            Some(c) => const_bytes(prog, c)?,
            None => {
                let n = Ty::desc_size(&prog.types, f.ty).ok_or_else(|| ice("record field type"))?
                    as usize;
                let x = mi.desc.get(cur..cur + n).unwrap_or(&[]).to_vec();
                cur += n;
                x
            }
        };
        if j == i {
            d = fd;
        }
    }
    alloc(m, eid, false, &d, &[], &mi.ranges)
}

// ---- arrays ----

fn array_parts(r: &Refl) -> R<(TyId, &[ReflId], ReflId)> {
    match &r.kind {
        ReflKind::Array { ty, index, elem } => Ok((*ty, index, *elem)),
        _ => Err(ice("array method on another class")),
    }
}

fn array_get(m: &mut Machine, a: &[u64], idx: &[i64]) -> R<u64> {
    let (mi, r) = this(m, a)?;
    let (ty, index, eid) = array_parts(&r)?;
    let ds = dims(&mi.desc, index.len())?;
    if idx.len() != ds.len() {
        return rt_err(format!(
            "get with {} indices on a {}-dimensional array mirror of {}",
            idx.len(),
            ds.len(),
            r.name
        ));
    }
    let mut k: u64 = 0;
    for (&i, &(l, rr, downto)) in idx.iter().zip(&ds) {
        let n = dim_count(l, rr, downto)?;
        let p = if downto {
            i128::from(l) - i128::from(i)
        } else {
            i128::from(i) - i128::from(l)
        };
        if p < 0 || p >= i128::from(n) {
            let d = if downto { "downto" } else { "to" };
            return rt_err(format!("index {i} outside of {l} {d} {rr} for {}", r.name));
        }
        k = k * n + p as u64;
    }
    let rt = m.rt.clone();
    let er = refl(&rt.prog, eid)?;
    let (d, v) = array_elem(&rt.prog, ty, er, &mi.desc, &mi.val, k)?;
    alloc(m, eid, true, &d, &v, &mi.ranges)
}

fn get1(m: &mut Machine, a: &[u64]) -> R<u64> {
    array_get(m, a, &[arg(a, 1)? as i64])
}

fn get2(m: &mut Machine, a: &[u64]) -> R<u64> {
    array_get(m, a, &[arg(a, 1)? as i64, arg(a, 2)? as i64])
}

fn get3(m: &mut Machine, a: &[u64]) -> R<u64> {
    array_get(
        m,
        a,
        &[arg(a, 1)? as i64, arg(a, 2)? as i64, arg(a, 3)? as i64],
    )
}

fn getv(m: &mut Machine, a: &[u64]) -> R<u64> {
    let (data, n) = m.fat1(arg(a, 1)?)?;
    let b = m.rd(data, n * 8)?.to_vec();
    let idx: Vec<i64> = (0..n as usize).map(|i| rd64(&b, i * 8) as i64).collect();
    array_get(m, a, &idx)
}

fn dimensions(m: &mut Machine, a: &[u64]) -> R<u64> {
    let (_, r) = this(m, a)?;
    Ok(array_parts(&r)?.1.len() as u64)
}

fn dim_arg(r: &Refl, a: &[u64]) -> R<usize> {
    let (_, index, _) = array_parts(r)?;
    let d = arg(a, 1)? as i64;
    usize::try_from(d - 1)
        .ok()
        .filter(|&d| d < index.len())
        .ok_or_else(|| {
            Fault::Runtime(format!(
                "dimension {d} outside of 1 to {} for {}",
                index.len(),
                r.name
            ))
        })
}

fn index_subtype(m: &mut Machine, a: &[u64]) -> R<u64> {
    let (mi, r) = this(m, a)?;
    let d = dim_arg(&r, a)?;
    let id = array_parts(&r)?.1[d];
    alloc(m, id, false, &[], &[], &mi.ranges)
}

fn element_subtype(m: &mut Machine, a: &[u64]) -> R<u64> {
    let (mi, r) = this(m, a)?;
    let (ty, index, eid) = array_parts(&r)?;
    let d = match m.rt.prog.types.get(ty) {
        Some(Ty::Array {
            elem_desc: Some(c), ..
        }) => const_bytes(&m.rt.prog, *c)?,
        _ => mi.desc.get(index.len() * 24..).unwrap_or(&[]).to_vec(),
    };
    alloc(m, eid, false, &d, &[], &mi.ranges)
}

fn array_dim(m: &mut Machine, a: &[u64]) -> R<(i64, i64, bool, String)> {
    let (mi, r) = this(m, a)?;
    let d = dim_arg(&r, a)?;
    let ds = dims(&mi.desc, array_parts(&r)?.1.len())?;
    let (l, rr, downto) = ds[d];
    Ok((l, rr, downto, r.name))
}

fn arr_left(m: &mut Machine, a: &[u64]) -> R<u64> {
    Ok(array_dim(m, a)?.0 as u64)
}

fn arr_right(m: &mut Machine, a: &[u64]) -> R<u64> {
    Ok(array_dim(m, a)?.1 as u64)
}

fn arr_low(m: &mut Machine, a: &[u64]) -> R<u64> {
    let (l, r, downto, _) = array_dim(m, a)?;
    Ok(if downto { r } else { l } as u64)
}

fn arr_high(m: &mut Machine, a: &[u64]) -> R<u64> {
    let (l, r, downto, _) = array_dim(m, a)?;
    Ok(if downto { l } else { r } as u64)
}

fn arr_length(m: &mut Machine, a: &[u64]) -> R<u64> {
    let (l, r, downto, _) = array_dim(m, a)?;
    dim_count(l, r, downto)
}

fn arr_ascending(m: &mut Machine, a: &[u64]) -> R<u64> {
    Ok(bool_cell(!array_dim(m, a)?.2))
}

// ---- access, file, protected ----

fn designated(r: &Refl) -> R<ReflId> {
    match r.kind {
        ReflKind::Access { designated } | ReflKind::File { designated } => Ok(designated),
        _ => Err(ice("designated_subtype on another class")),
    }
}

fn designated_subtype(m: &mut Machine, a: &[u64]) -> R<u64> {
    let (mi, r) = this(m, a)?;
    let d = designated(&r)?;
    alloc(m, d, false, &[], &[], &mi.ranges)
}

fn is_null(m: &mut Machine, a: &[u64]) -> R<u64> {
    let (mi, _) = this(m, a)?;
    Ok(bool_cell(cell(&mi) == 0))
}

/// The value mirror of the object an access value designates.
fn access_get(m: &mut Machine, a: &[u64]) -> R<u64> {
    let (mi, r) = this(m, a)?;
    let did = designated(&r)?;
    let h = cell(&mi);
    if h == 0 {
        return rt_err(format!(
            "null access value dereferenced: get of a mirror of {}",
            r.name
        ));
    }
    let (hp, _) = m.heap_of(h)?;
    let i = hp.resolve(h).map_err(|_| {
        Fault::Runtime("access value designates an object that was deallocated".into())
    })?;
    let data = hp
        .data(i)
        .ok_or_else(|| ice("heap object vanished"))?
        .to_vec();
    let rt = m.rt.clone();
    let prog = &rt.prog;
    let dr = refl(prog, did)?;
    let (desc, val) = match &dr.kind {
        ReflKind::Array { ty, .. } => {
            let n = Ty::desc_size(&prog.types, *ty).ok_or_else(|| ice("array descriptor size"))?
                as usize;
            let desc = data
                .get(..n)
                .ok_or_else(|| ice("array object shorter than its descriptor"))?;
            (desc.to_vec(), data[n..].to_vec())
        }
        ReflKind::Record { .. } => (Vec::new(), data),
        k => {
            let w = scalar_width(prog, k)?.unwrap_or(8);
            let b = data
                .get(..w)
                .ok_or_else(|| ice("scalar heap object too short"))?;
            (Vec::new(), le(b).to_le_bytes().to_vec())
        }
    };
    alloc(m, did, true, &desc, &val, &mi.ranges)
}

fn file_logical_name(m: &mut Machine, a: &[u64]) -> R<u64> {
    let (mi, _) = this(m, a)?;
    let name = crate::textio::file_info(m, cell(&mi))?.0;
    m.ret_string(&name)
}

fn file_open_kind(m: &mut Machine, a: &[u64]) -> R<u64> {
    let (mi, _) = this(m, a)?;
    Ok(crate::textio::file_info(m, cell(&mi))?.1)
}

macro_rules! natives {
    ($($sym:literal => $f:expr,)*) => {
        /// The implementation of a `std.reflection` method or of `yavs.attr.reflect`.
        pub(crate) fn lookup(symbol: &str) -> Option<NativeFn> {
            Some(match symbol {
                "yavs.attr.reflect" => {
                    fn w(m: &mut Machine, _: &mut Option<&mut Ctx<'_>>, _: Option<TyId>, a: &[u64]) -> R<Option<u64>> {
                        reflect_attr(m, a).map(Some)
                    }
                    w
                }
                $(concat!("yavs.reflection.", $sym) => {
                    fn w(m: &mut Machine, _: &mut Option<&mut Ctx<'_>>, _: Option<TyId>, a: &[u64]) -> R<Option<u64>> {
                        let f: fn(&mut Machine, &[u64]) -> R<u64> = $f;
                        f(m, a).map(Some)
                    }
                    w
                })*
                _ => return None,
            })
        }

        #[cfg(test)]
        const SYMBOLS: &[&str] = &[$(concat!("yavs.reflection.", $sym)),*];
    };
}

natives! {
    "enumeration_value_mirror_pt.get_subtype_mirror.void.r_enumeration_subtype_mirror" => subtype_of,
    "enumeration_value_mirror_pt.to_value_mirror.void.r_value_mirror" => same,
    "enumeration_value_mirror_pt.pos.void.r_integer" => value,
    "enumeration_value_mirror_pt.image.void.r_string" => image,
    "enumeration_subtype_mirror_pt.to_subtype_mirror.void.r_subtype_mirror" => same,
    "enumeration_subtype_mirror_pt.enumeration_literal.natural_index.r_enumeration_value_mirror" => enum_literal_idx,
    "enumeration_subtype_mirror_pt.enumeration_literal.string.r_enumeration_value_mirror" => enum_literal_name,
    "enumeration_subtype_mirror_pt.simple_name.void.r_string" => simple_name,
    "enumeration_subtype_mirror_pt.left.void.r_enumeration_value_mirror" => |m, a| bound(m, a, Bound::Left),
    "enumeration_subtype_mirror_pt.right.void.r_enumeration_value_mirror" => |m, a| bound(m, a, Bound::Right),
    "enumeration_subtype_mirror_pt.low.void.r_enumeration_value_mirror" => |m, a| bound(m, a, Bound::Low),
    "enumeration_subtype_mirror_pt.high.void.r_enumeration_value_mirror" => |m, a| bound(m, a, Bound::High),
    "enumeration_subtype_mirror_pt.length.void.r_positive_index" => length,
    "enumeration_subtype_mirror_pt.ascending.void.r_boolean" => ascending,
    "integer_value_mirror_pt.get_subtype_mirror.void.r_integer_subtype_mirror" => subtype_of,
    "integer_value_mirror_pt.to_value_mirror.void.r_value_mirror" => same,
    "integer_value_mirror_pt.value.void.r_integer" => value,
    "integer_value_mirror_pt.image.void.r_string" => image,
    "integer_subtype_mirror_pt.to_subtype_mirror.void.r_subtype_mirror" => same,
    "integer_subtype_mirror_pt.simple_name.void.r_string" => simple_name,
    "integer_subtype_mirror_pt.left.void.r_integer_value_mirror" => |m, a| bound(m, a, Bound::Left),
    "integer_subtype_mirror_pt.right.void.r_integer_value_mirror" => |m, a| bound(m, a, Bound::Right),
    "integer_subtype_mirror_pt.low.void.r_integer_value_mirror" => |m, a| bound(m, a, Bound::Low),
    "integer_subtype_mirror_pt.high.void.r_integer_value_mirror" => |m, a| bound(m, a, Bound::High),
    "integer_subtype_mirror_pt.length.void.r_index" => length,
    "integer_subtype_mirror_pt.ascending.void.r_boolean" => ascending,
    "floating_value_mirror_pt.get_subtype_mirror.void.r_floating_subtype_mirror" => subtype_of,
    "floating_value_mirror_pt.to_value_mirror.void.r_value_mirror" => same,
    "floating_value_mirror_pt.value.void.r_real" => value,
    "floating_value_mirror_pt.image.void.r_string" => image,
    "floating_subtype_mirror_pt.to_subtype_mirror.void.r_subtype_mirror" => same,
    "floating_subtype_mirror_pt.simple_name.void.r_string" => simple_name,
    "floating_subtype_mirror_pt.left.void.r_floating_value_mirror" => |m, a| bound(m, a, Bound::Left),
    "floating_subtype_mirror_pt.right.void.r_floating_value_mirror" => |m, a| bound(m, a, Bound::Right),
    "floating_subtype_mirror_pt.low.void.r_floating_value_mirror" => |m, a| bound(m, a, Bound::Low),
    "floating_subtype_mirror_pt.high.void.r_floating_value_mirror" => |m, a| bound(m, a, Bound::High),
    "floating_subtype_mirror_pt.ascending.void.r_boolean" => ascending,
    "physical_value_mirror_pt.get_subtype_mirror.void.r_physical_subtype_mirror" => subtype_of,
    "physical_value_mirror_pt.to_value_mirror.void.r_value_mirror" => same,
    "physical_value_mirror_pt.unit_index.void.r_index" => phys_unit_index,
    "physical_value_mirror_pt.value.void.r_integer" => phys_value,
    "physical_value_mirror_pt.image.void.r_string" => image,
    "physical_subtype_mirror_pt.to_subtype_mirror.void.r_subtype_mirror" => same,
    "physical_subtype_mirror_pt.units_length.void.r_index" => units_length,
    "physical_subtype_mirror_pt.unit_name.index.r_string" => unit_name,
    "physical_subtype_mirror_pt.unit_index.string.r_index" => unit_index,
    "physical_subtype_mirror_pt.scale.index.r_natural" => scale_idx,
    "physical_subtype_mirror_pt.scale.string.r_natural" => scale_name,
    "physical_subtype_mirror_pt.simple_name.void.r_string" => simple_name,
    "physical_subtype_mirror_pt.left.void.r_physical_value_mirror" => |m, a| bound(m, a, Bound::Left),
    "physical_subtype_mirror_pt.right.void.r_physical_value_mirror" => |m, a| bound(m, a, Bound::Right),
    "physical_subtype_mirror_pt.low.void.r_physical_value_mirror" => |m, a| bound(m, a, Bound::Low),
    "physical_subtype_mirror_pt.high.void.r_physical_value_mirror" => |m, a| bound(m, a, Bound::High),
    "physical_subtype_mirror_pt.length.void.r_index" => length,
    "physical_subtype_mirror_pt.ascending.void.r_boolean" => ascending,
    "record_value_mirror_pt.get_subtype_mirror.void.r_record_subtype_mirror" => subtype_of,
    "record_value_mirror_pt.to_value_mirror.void.r_value_mirror" => same,
    "record_value_mirror_pt.get.index.r_value_mirror" => |m, a| record_get(m, a, false),
    "record_value_mirror_pt.get.string.r_value_mirror" => |m, a| record_get(m, a, true),
    "record_subtype_mirror_pt.to_subtype_mirror.void.r_subtype_mirror" => same,
    "record_subtype_mirror_pt.length.void.r_index" => record_length,
    "record_subtype_mirror_pt.element_name.index.r_string" => element_name,
    "record_subtype_mirror_pt.element_index.string.r_index" => element_index,
    "record_subtype_mirror_pt.element_subtype.index.r_subtype_mirror" => |m, a| record_elem_subtype(m, a, false),
    "record_subtype_mirror_pt.element_subtype.string.r_subtype_mirror" => |m, a| record_elem_subtype(m, a, true),
    "record_subtype_mirror_pt.simple_name.void.r_string" => simple_name,
    "array_value_mirror_pt.get_subtype_mirror.void.r_array_subtype_mirror" => subtype_of,
    "array_value_mirror_pt.to_value_mirror.void.r_value_mirror" => same,
    "array_value_mirror_pt.get.index.r_value_mirror" => get1,
    "array_value_mirror_pt.get.index_index.r_value_mirror" => get2,
    "array_value_mirror_pt.get.index_index_index.r_value_mirror" => get3,
    "array_value_mirror_pt.get.index_vector.r_value_mirror" => getv,
    "array_subtype_mirror_pt.to_subtype_mirror.void.r_subtype_mirror" => same,
    "array_subtype_mirror_pt.dimensions.void.r_dimension" => dimensions,
    "array_subtype_mirror_pt.index_subtype.dimension.r_subtype_mirror" => index_subtype,
    "array_subtype_mirror_pt.element_subtype.void.r_subtype_mirror" => element_subtype,
    "array_subtype_mirror_pt.simple_name.void.r_string" => simple_name,
    "array_subtype_mirror_pt.left.dimension.r_index" => arr_left,
    "array_subtype_mirror_pt.right.dimension.r_index" => arr_right,
    "array_subtype_mirror_pt.low.dimension.r_index" => arr_low,
    "array_subtype_mirror_pt.high.dimension.r_index" => arr_high,
    "array_subtype_mirror_pt.length.dimension.r_index" => arr_length,
    "array_subtype_mirror_pt.ascending.dimension.r_boolean" => arr_ascending,
    "access_value_mirror_pt.get_subtype_mirror.void.r_access_subtype_mirror" => subtype_of,
    "access_value_mirror_pt.to_value_mirror.void.r_value_mirror" => same,
    "access_value_mirror_pt.get.void.r_value_mirror" => access_get,
    "access_value_mirror_pt.is_null.void.r_boolean" => is_null,
    "access_subtype_mirror_pt.to_subtype_mirror.void.r_subtype_mirror" => same,
    "access_subtype_mirror_pt.simple_name.void.r_string" => simple_name,
    "access_subtype_mirror_pt.designated_subtype.void.r_subtype_mirror" => designated_subtype,
    "file_value_mirror_pt.get_subtype_mirror.void.r_file_subtype_mirror" => subtype_of,
    "file_value_mirror_pt.to_value_mirror.void.r_value_mirror" => same,
    "file_value_mirror_pt.get_file_logical_name.void.r_string" => file_logical_name,
    "file_value_mirror_pt.get_file_open_kind.void.r_file_open_kind" => file_open_kind,
    "file_subtype_mirror_pt.to_subtype_mirror.void.r_subtype_mirror" => same,
    "file_subtype_mirror_pt.simple_name.void.r_string" => simple_name,
    "file_subtype_mirror_pt.designated_subtype.void.r_subtype_mirror" => designated_subtype,
    "protected_value_mirror_pt.get_subtype_mirror.void.r_protected_subtype_mirror" => subtype_of,
    "protected_value_mirror_pt.to_value_mirror.void.r_value_mirror" => same,
    "protected_subtype_mirror_pt.to_subtype_mirror.void.r_subtype_mirror" => same,
    "protected_subtype_mirror_pt.simple_name.void.r_string" => simple_name,
    "subtype_mirror_pt.get_type_class.void.r_type_class" => class_of,
    "subtype_mirror_pt.to_enumeration.void.r_enumeration_subtype_mirror" => |m, a| to_class(m, a, 0),
    "subtype_mirror_pt.to_integer.void.r_integer_subtype_mirror" => |m, a| to_class(m, a, 1),
    "subtype_mirror_pt.to_floating.void.r_floating_subtype_mirror" => |m, a| to_class(m, a, 2),
    "subtype_mirror_pt.to_physical.void.r_physical_subtype_mirror" => |m, a| to_class(m, a, 3),
    "subtype_mirror_pt.to_record.void.r_record_subtype_mirror" => |m, a| to_class(m, a, 4),
    "subtype_mirror_pt.to_array.void.r_array_subtype_mirror" => |m, a| to_class(m, a, 5),
    "subtype_mirror_pt.to_access.void.r_access_subtype_mirror" => |m, a| to_class(m, a, 6),
    "subtype_mirror_pt.to_file.void.r_file_subtype_mirror" => |m, a| to_class(m, a, 7),
    "subtype_mirror_pt.to_protected.void.r_protected_subtype_mirror" => |m, a| to_class(m, a, 8),
    "subtype_mirror_pt.simple_name.void.r_string" => simple_name,
    "value_mirror_pt.get_value_class.void.r_value_class" => class_of,
    "value_mirror_pt.get_subtype_mirror.void.r_subtype_mirror" => subtype_of,
    "value_mirror_pt.to_enumeration.void.r_enumeration_value_mirror" => |m, a| to_class(m, a, 0),
    "value_mirror_pt.to_integer.void.r_integer_value_mirror" => |m, a| to_class(m, a, 1),
    "value_mirror_pt.to_floating.void.r_floating_value_mirror" => |m, a| to_class(m, a, 2),
    "value_mirror_pt.to_physical.void.r_physical_value_mirror" => |m, a| to_class(m, a, 3),
    "value_mirror_pt.to_record.void.r_record_value_mirror" => |m, a| to_class(m, a, 4),
    "value_mirror_pt.to_array.void.r_array_value_mirror" => |m, a| to_class(m, a, 5),
    "value_mirror_pt.to_access.void.r_access_value_mirror" => |m, a| to_class(m, a, 6),
    "value_mirror_pt.to_file.void.r_file_value_mirror" => |m, a| to_class(m, a, 7),
    "value_mirror_pt.to_protected.void.r_protected_value_mirror" => |m, a| to_class(m, a, 8),
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every reflection row of NATIVE.md is implemented, and nothing else.
    #[test]
    fn all_native_md_rows() {
        let md = include_str!("../../../lib/NATIVE.md");
        let mut rows: Vec<&str> = md
            .split('`')
            .filter(|t| t.starts_with("yavs.reflection."))
            .collect();
        rows.sort_unstable();
        rows.dedup();
        assert_eq!(rows.len(), 123);
        let mut ours = SYMBOLS.to_vec();
        ours.sort_unstable();
        assert_eq!(ours, rows);
        for s in SYMBOLS {
            assert!(lookup(s).is_some(), "{s}");
        }
        assert!(lookup("yavs.attr.reflect").is_some());
    }

    fn dope(dims: &[(i64, i64, i64)]) -> Vec<u8> {
        dims.iter()
            .flat_map(|&(l, r, d)| [l, r, d])
            .flat_map(i64::to_le_bytes)
            .collect()
    }

    fn entry(name: &str, kind: ReflKind) -> Refl {
        Refl {
            name: name.into(),
            kind,
        }
    }

    /// Element extraction from records with a logic-array field and from a nested logic array
    /// (one plane vector over all leaves, element 0 rightmost).
    #[test]
    fn record_and_nested_logic_elements() {
        use yavs_ir::{Field, IdVec};
        let mut types = IdVec::new();
        let int = types.push(Ty::Int {
            lo: i64::MIN,
            hi: i64::MAX,
        });
        let sl = types.push(Ty::Enum {
            lits: [
                "'U'", "'X'", "'0'", "'1'", "'Z'", "'W'", "'L'", "'H'", "'-'",
            ]
            .map(String::from)
            .to_vec(),
            logic: Some(LogicKind::StdULogic),
        });
        let slv = types.push(Ty::Array {
            elem: sl,
            elem_desc: None,
            dims: 1,
        });
        let mut consts = IdVec::new();
        let c3 = consts.push(ConstData::Bytes(dope(&[(1, 3, 0)])));
        let rec = types.push(Ty::Record {
            fields: vec![
                Field {
                    name: "a".into(),
                    ty: int,
                    desc: None,
                },
                Field {
                    name: "b".into(),
                    ty: slv,
                    desc: Some(c3),
                },
            ],
        });
        let mem = types.push(Ty::Array {
            elem: slv,
            elem_desc: Some(c3),
            dims: 1,
        });
        let prog = Program {
            types,
            consts,
            ..Program::default()
        };
        let sl_r = entry(
            "STD_ULOGIC",
            ReflKind::Enum {
                ty: sl,
                left: 0,
                right: 8,
                downto: false,
            },
        );
        let slv_r = entry(
            "STD_ULOGIC_VECTOR",
            ReflKind::Array {
                ty: slv,
                index: vec![],
                elem: ReflId(0),
            },
        );
        let int_r = entry(
            "INTEGER",
            ReflKind::Int {
                left: 0,
                right: 9,
                downto: false,
            },
        );
        // rec = (a => 42, b => "01Z"): b(1) is the leftmost, plane index 2.
        let (lay, _) = Ty::layout(&prog.types, &prog.consts, rec, &[]).unwrap();
        let mut val = vec![0u8; lay.size as usize];
        val[..8].copy_from_slice(&42i64.to_le_bytes());
        let off = lay.offsets[1] as usize;
        for (i, code) in [2u64, 3, 4].into_iter().enumerate() {
            plane_set(&mut val[off..], 2 - i as u64, code);
        }
        let (_, a) = record_elem(&prog, rec, &int_r, &[], &val, 0).unwrap();
        assert_eq!(rd64(&a, 0), 42);
        let (bd, b) = record_elem(&prog, rec, &slv_r, &[], &val, 1).unwrap();
        assert_eq!(bd, dope(&[(1, 3, 0)]));
        let (_, e) = array_elem(&prog, slv, &sl_r, &bd, &b, 0).unwrap();
        assert_eq!(rd64(&e, 0), 2);
        let (_, e) = array_elem(&prog, slv, &sl_r, &bd, &b, 2).unwrap();
        assert_eq!(rd64(&e, 0), 4);
        // mem(0 to 1) of slv(1 to 3): ("01Z", "HL-") as 6 leaves, leftmost leaf at plane 5.
        let md = dope(&[(0, 1, 0)]);
        let (ml, _) = Ty::layout(&prog.types, &prog.consts, mem, &md).unwrap();
        let mut mv = vec![0u8; ml.size as usize];
        for (p, code) in [2u64, 3, 4, 7, 6, 8].into_iter().enumerate() {
            plane_set(&mut mv, 5 - p as u64, code);
        }
        let (rd, row) = array_elem(&prog, mem, &slv_r, &md, &mv, 1).unwrap();
        assert_eq!(rd, dope(&[(1, 3, 0)]));
        let codes: Vec<u64> = (0..3)
            .map(|k| rd64(&array_elem(&prog, slv, &sl_r, &rd, &row, k).unwrap().1, 0))
            .collect();
        assert_eq!(codes, [7, 6, 8]);
    }

    /// TIME mirror values are in resolution-limit units (decisions.md): 5 ns at a
    /// picosecond resolution is 5000 of unit 2 (PS).
    #[test]
    fn time_in_resolution_units() {
        let us: Vec<(String, i64)> = [("FS", 1), ("PS", 1000), ("NS", 1_000_000)]
            .map(|(n, s)| (n.to_string(), s))
            .to_vec();
        assert_eq!(time_unit(&us, 1).unwrap(), (1, 1));
        let (idx, scale) = time_unit(&us, 1000).unwrap();
        assert_eq!((idx, 5_000_000 / scale), (2, 5000));
        assert!(time_unit(&us, 10).is_err());
    }

    /// Run-time ranges survive the object encoding and win over the entry's static range.
    #[test]
    fn mirror_ranges_round_trip() {
        let ranges: Vec<u8> = [9i64, 0, 0, 0, 7, 4, -2, 1]
            .into_iter()
            .flat_map(i64::to_le_bytes)
            .collect();
        let d = encode(ReflId(7), true, &[1, 2], &5u64.to_le_bytes(), &ranges).unwrap();
        let mi = parse(&d).unwrap();
        assert!(mi.value);
        assert_eq!(
            (mi.refl, mi.desc.as_slice(), cell(&mi)),
            (ReflId(7), &[1u8, 2][..], 5)
        );
        let r = entry(
            "RNG",
            ReflKind::Int {
                left: 0,
                right: 3,
                downto: false,
            },
        );
        assert_eq!(scalar_range(&mi, &r).unwrap(), (4, (-2i64) as u64, true));
    }

    #[test]
    fn planes_round_trip() {
        let mut v = vec![0u8; 64];
        for i in 0..70 {
            plane_set(&mut v, i, i % 9);
        }
        for i in 0..70 {
            assert_eq!(plane_get(&v, i), i % 9);
        }
    }
}
