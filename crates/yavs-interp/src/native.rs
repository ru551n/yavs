//! Natives: the dispatch table keyed by `lib/NATIVE.md` symbol, wired to yavs-std.
//!
//! A symbol that NATIVE.md does not define is an error when the [`crate::Runtime`] is built
//! (fail early). A symbol that is defined but not implemented here yet is a runtime error
//! naming it when it is called.
//!
//! Calling convention: arguments are cells (scalars by value, composites by address); string
//! results are returned as the address of a fat pair `[data, dope]` in temporary memory.
//! Implicit-family natives (`yavs.implicit.<family>.<type>`) and attribute natives
//! (`yavs.attr.<name>`, an yavs addition) get the type from `NativeRef::ty`.

use std::collections::HashSet;
use std::sync::OnceLock;

use yavs_ir::{Ty, TyId};
use yavs_kernel::Ctx;
use yavs_std::image;
use yavs_std::types::{EnumLit, PhysicalType};

use crate::exec::{Fault, Machine, R, add_addr, ice, rt_err};
use crate::reflect::{const_bytes, layout_err, le, logic_pos, plane_get, plane_set};

pub(crate) type NativeFn =
    fn(&mut Machine, &mut Option<&mut Ctx<'_>>, Option<TyId>, &[u64]) -> R<Option<u64>>;

const NATIVE_MD: &str = include_str!("../../../lib/NATIVE.md");

/// Attribute natives yavs defines beyond NATIVE.md.
const ATTRS: &[&str] = &["image", "value"];

struct Known {
    explicit: HashSet<&'static str>,
    families: HashSet<&'static str>,
}

fn known() -> &'static Known {
    static K: OnceLock<Known> = OnceLock::new();
    K.get_or_init(|| {
        let mut explicit = HashSet::new();
        let mut families = HashSet::new();
        for line in NATIVE_MD.lines() {
            let implicit = line.contains("`yavs.implicit.");
            for (i, tok) in line.split('`').enumerate() {
                if i % 2 == 0 {
                    continue;
                }
                if let Some(f) = tok.strip_prefix("yavs.implicit.") {
                    if !f.contains('<') {
                        families.insert(f);
                    }
                } else if implicit && let Some(f) = tok.strip_prefix('.') {
                    families.insert(f);
                } else if tok.starts_with("yavs.") && !tok.contains('<') {
                    explicit.insert(tok);
                }
            }
        }
        Known { explicit, families }
    })
}

/// Whether the native `symbol` only computes from its arguments and the heap: package
/// initialisation that calls any other (files, the environment, clocks, assert state,
/// reflection) is not stored (`SharedState::tainted`, #407). A symbol not named here is not pure.
pub(crate) fn pure(symbol: &str) -> bool {
    yavs_ir::native_is_pure(symbol)
}

/// The implementation of `symbol`: `Ok(None)` when NATIVE.md defines it but yavs does not
/// implement it yet, `Err` when it is not a native symbol at all.
pub(crate) fn lookup(symbol: &str) -> Result<Option<NativeFn>, String> {
    if let Some(f) = crate::reflect::lookup(symbol) {
        return Ok(Some(f));
    }
    if symbol.starts_with("yavs.ieee.") {
        return crate::ieee::lookup(symbol)
            .map(Some)
            .ok_or_else(|| format!("`{symbol}` is not a native ieee subprogram"));
    }
    let family = symbol
        .strip_prefix("yavs.implicit.")
        .map(|r| r.split('.').next().unwrap_or(r));
    let attr = symbol.strip_prefix("yavs.attr.");
    // NATIVE.md (about 38 KB) is read only to tell "defined, not implemented" from "not a native
    // symbol": a run that finds every implementation never parses it (#426). Debug builds still
    // check that what is implemented is documented.
    let valid = || {
        let k = known();
        k.explicit.contains(symbol)
            || family.is_some_and(|f| k.families.contains(f))
            || attr.is_some_and(|a| ATTRS.contains(&a))
    };
    let found = implemented(symbol, family, attr);
    if found.is_some() {
        debug_assert!(
            valid(),
            "`{symbol}` is implemented but not in lib/NATIVE.md"
        );
        return Ok(found);
    }
    if !valid() {
        return Err(format!(
            "`{symbol}` is not a native subprogram (lib/NATIVE.md defines the symbols)"
        ));
    }
    Ok(None)
}

/// The implementation of the native `symbol` (`family`: the family of a `yavs.implicit.` symbol,
/// `attr`: the name of a `yavs.attr.` one), when yavs has one.
fn implemented(symbol: &str, family: Option<&str>, attr: Option<&str>) -> Option<NativeFn> {
    if let Some(f) = crate::implicit::lookup(symbol).or_else(|| crate::env::lookup(symbol)) {
        return Some(f);
    }
    let f: NativeFn = match (symbol, family, attr) {
        ("yavs.standard.now.void.r_delay_length", _, _) => now,
        ("yavs.env.stop.void", _, _) => stop0,
        ("yavs.env.stop.integer", _, _) => stop1,
        ("yavs.env.finish.void", _, _) => finish0,
        ("yavs.env.finish.integer", _, _) => finish1,
        (_, _, Some("value")) => value_attr,
        (_, _, Some("image")) => image_attr,
        ("yavs.env.resolution_limit.void.r_delay_length", _, _) => resolution_limit,
        ("yavs.env.getenv.string.r_string", _, _) => getenv_string,
        ("yavs.env.getenv.string.r_line", _, _) => getenv_line,
        ("yavs.env.vhdl_version.void.r_string", _, _) => vhdl_version,
        ("yavs.env.tool_type.void.r_string", _, _) => tool_type,
        ("yavs.env.tool_vendor.void.r_string", _, _) => tool_vendor,
        ("yavs.env.tool_name.void.r_string", _, _) => tool_name,
        ("yavs.env.tool_edition.void.r_string", _, _) => tool_edition,
        ("yavs.env.tool_version.void.r_string", _, _) => tool_version,
        ("yavs.env.epoch.void.r_real", _, _) => epoch,
        ("yavs.env.time_to_seconds.time.r_real", _, _) => time_to_seconds,
        ("yavs.env.seconds_to_time.real.r_time", _, _) => seconds_to_time,
        ("yavs.env.dir_open.directory_string_dir_open_status", _, _) => dir_open_proc,
        ("yavs.env.dir_open.directory_string.r_dir_open_status", _, _) => dir_open_fn,
        ("yavs.env.dir_close.directory", _, _) => dir_close,
        ("yavs.env.dir_itemexists.string.r_boolean", _, _) => dir_itemexists,
        ("yavs.env.dir_itemisdir.string.r_boolean", _, _) => dir_itemisdir,
        ("yavs.env.dir_itemisfile.string.r_boolean", _, _) => dir_itemisfile,
        ("yavs.env.dir_workingdir.string_dir_open_status", _, _) => set_workingdir_proc,
        ("yavs.env.dir_workingdir.string.r_dir_open_status", _, _) => set_workingdir_fn,
        ("yavs.env.dir_workingdir.void.r_string", _, _) => workingdir,
        ("yavs.env.dir_createdir.string_dir_create_status", _, _) => createdir_proc,
        ("yavs.env.dir_createdir.string_boolean_dir_create_status", _, _) => createdir_proc,
        ("yavs.env.dir_createdir.string_boolean.r_dir_create_status", _, _) => createdir_fn,
        ("yavs.env.dir_deletedir.string_dir_delete_status", _, _) => deletedir_proc,
        ("yavs.env.dir_deletedir.string_boolean_dir_delete_status", _, _) => deletedir_proc,
        ("yavs.env.dir_deletedir.string_boolean.r_dir_delete_status", _, _) => deletedir_fn,
        ("yavs.env.dir_deletefile.string_file_delete_status", _, _) => deletefile_proc,
        ("yavs.env.dir_deletefile.string.r_file_delete_status", _, _) => deletefile_fn,
        _ => crate::textio::lookup(symbol, family)?,
    };
    Some(f)
}

fn kctx<'a, 'c>(ctx: &'a mut Option<&mut Ctx<'c>>) -> R<&'a mut Ctx<'c>> {
    match ctx.as_deref_mut() {
        Some(c) => Ok(c),
        None => rt_err("simulation-only native called during elaboration"),
    }
}

fn arg(args: &[u64], i: usize) -> R<u64> {
    args.get(i)
        .copied()
        .ok_or_else(|| ice(format!("native argument {i} missing")))
}

fn status(args: &[u64]) -> R<i32> {
    let v = arg(args, 0)? as i64;
    i32::try_from(v).map_err(|_| Fault::Runtime(format!("exit status {v} out of range")))
}

fn now(
    _: &mut Machine,
    ctx: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    _: &[u64],
) -> R<Option<u64>> {
    // During elaboration the current time is 0 fs (LRM 14.7.5.2: time starts at 0).
    Ok(Some(ctx.as_deref().map_or(0, |c| c.now().as_fs())))
}

fn stop0(
    _: &mut Machine,
    ctx: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    _: &[u64],
) -> R<Option<u64>> {
    kctx(ctx)?.stop(0);
    Ok(None)
}

fn stop1(
    _: &mut Machine,
    ctx: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    a: &[u64],
) -> R<Option<u64>> {
    let s = status(a)?;
    kctx(ctx)?.stop(s);
    Ok(None)
}

fn finish0(
    _: &mut Machine,
    ctx: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    _: &[u64],
) -> R<Option<u64>> {
    kctx(ctx)?.finish(0);
    Ok(None)
}

fn finish1(
    _: &mut Machine,
    ctx: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    a: &[u64],
) -> R<Option<u64>> {
    let s = status(a)?;
    kctx(ctx)?.finish(s);
    Ok(None)
}

pub(crate) fn lit(s: &str) -> EnumLit {
    let b = s.as_bytes();
    if b.len() == 3 && b[0] == b'\'' && b[2] == b'\'' {
        EnumLit::Char(b[1])
    } else {
        EnumLit::ident(s)
    }
}

/// `'image` (`image = true`) or `to_string` of a scalar.
pub(crate) fn scalar_text(m: &Machine, ty: Option<TyId>, v: u64, image: bool) -> R<Vec<u8>> {
    let ty = ty.ok_or_else(|| ice("image native without a type"))?;
    let t =
        m.rt.prog
            .types
            .get(ty)
            .ok_or_else(|| ice(format!("no type {ty}")))?;
    Ok(match t {
        Ty::Int { .. } => image::integer_image(v as i64),
        Ty::Real => image::real_image(f64::from_bits(v)),
        Ty::Physical { units, .. } => {
            let u: Vec<(&str, i64)> = units.iter().map(|(n, v)| (n.as_str(), *v)).collect();
            image::physical_image(v as i64, &PhysicalType::new(&u))
        }
        Ty::Enum { lits, .. } => {
            let l = lits
                .get(v as usize)
                .ok_or_else(|| ice(format!("enumeration position {v} of {ty}")))?;
            let l = lit(l);
            match (&l, image) {
                (EnumLit::Char(c), true) => image::character_image(*c),
                (EnumLit::Char(c), false) => image::character_to_string(*c),
                (_, true) => image::enum_image(&l),
                (_, false) => image::enum_to_string(&l),
            }
        }
        _ => {
            return rt_err(format!(
                "'image of a non-scalar type {ty} is not implemented yet"
            ));
        }
    })
}

/// `T'value(s)`: the string as a fat pair, `ty` the base type. A composite
/// takes its constrained prefix descriptor, or each open index's left and direction.
fn value_attr(
    m: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    ty: Option<TyId>,
    a: &[u64],
) -> R<Option<u64>> {
    use yavs_std::types::{Bounds, EnumType, RealBounds};
    use yavs_std::value;
    let s = m.fat_bytes(arg(a, 0)?)?;
    let s = if a.len() > 2
        && !s.iter().copied().any(yavs_std::value::is_ws)
        && s.first() != Some(&b'(')
        && s.first() != Some(&b'"')
    {
        let mut quoted = Vec::with_capacity(s.len() + 2);
        quoted.push(b'"');
        for &c in &s {
            quoted.push(c);
            if c == b'"' {
                quoted.push(c);
            }
        }
        quoted.push(b'"');
        quoted
    } else {
        s
    };
    let ty = ty.ok_or_else(|| ice("'value native without a type"))?;
    let t =
        m.rt.prog
            .types
            .get(ty)
            .ok_or_else(|| ice(format!("no type {ty}")))?;
    let r = match t {
        Ty::Int { lo, hi } => value::integer_value(
            &s,
            Bounds {
                low: *lo,
                high: *hi,
            },
        )
        .map(|v| v as u64),
        Ty::Real => value::real_value(&s, RealBounds::REAL).map(f64::to_bits),
        Ty::Physical { lo, hi, units } => {
            let u: Vec<(&str, i64)> = units.iter().map(|(n, v)| (n.as_str(), *v)).collect();
            value::physical_value(
                &s,
                &PhysicalType::new(&u),
                Bounds {
                    low: *lo,
                    high: *hi,
                },
            )
            .map(|v| v as u64)
        }
        Ty::Enum { lits, .. } => {
            let e = EnumType {
                lits: lits.iter().map(|l| lit(l)).collect(),
            };
            value::enum_value(&s, &e).map(|v| v as u64)
        }
        Ty::Array { .. } | Ty::Record { .. } => {
            let n = Ty::desc_size(&m.rt.prog.types, ty).ok_or_else(|| ice("descriptor size"))?;
            let desc = if a.len() > 2 || n == 0 {
                Vec::new()
            } else {
                let dp = arg(a, 1)?;
                m.rd(dp, n)?.to_vec()
            };
            let mut at = 0;
            let lt = lit_type(m, ty, &desc, &s, &mut at, a)?;
            let v = value::literal(&s, &lt).map_err(|e| Fault::Runtime(format!("'value: {e}")))?;
            return encode_composite(m, ty, &lt, &v).map(Some);
        }
        _ => return rt_err(format!("'value of this type {ty} is not implemented yet")),
    };
    r.map(Some)
        .map_err(|e| Fault::Runtime(format!("'value: {e}")))
}

/// The parser type of `ty` under `desc`. Open dimensions take the index
/// subtype's left and direction, and their length from the literal.
fn lit_type(
    m: &Machine,
    ty: TyId,
    desc: &[u8],
    s: &[u8],
    at: &mut usize,
    a: &[u64],
) -> R<yavs_std::value::LitType> {
    use yavs_std::types::{Bounds, EnumType, RealBounds};
    use yavs_std::value::LitType;
    let t =
        m.rt.prog
            .types
            .get(ty)
            .ok_or_else(|| ice(format!("no type {ty}")))?;
    Ok(match t {
        Ty::Int { lo, hi } => LitType::Int(Bounds {
            low: *lo,
            high: *hi,
        }),
        Ty::Real => LitType::Real(RealBounds::REAL),
        Ty::Physical { lo, hi, units } => {
            let u: Vec<(&str, i64)> = units.iter().map(|(n, v)| (n.as_str(), *v)).collect();
            LitType::Phys(
                PhysicalType::new(&u),
                Bounds {
                    low: *lo,
                    high: *hi,
                },
            )
        }
        Ty::Enum { lits, .. } => LitType::Enum(EnumType {
            lits: lits.iter().map(|l| lit(l)).collect(),
        }),
        Ty::Record { fields } => {
            let mut fs = Vec::with_capacity(fields.len());
            for f in fields {
                let fd = match f.desc {
                    Some(c) => const_bytes(&m.rt.prog, c)?,
                    None => {
                        let n = Ty::desc_size(&m.rt.prog.types, f.ty)
                            .ok_or_else(|| ice("record field descriptor size"))?
                            as usize;
                        let fd = desc.get(*at..*at + n).unwrap_or(&[]).to_vec();
                        *at += n;
                        fd
                    }
                };
                fs.push((f.name.clone(), lit_type(m, f.ty, &fd, s, &mut 0, a)?));
            }
            LitType::Record(fs)
        }
        Ty::Array {
            elem,
            elem_desc,
            dims,
        } => {
            let nd = usize::from(*dims);
            let mut bounds = Vec::with_capacity(nd);
            for d in 0..nd {
                let b = desc.get(d * 24..d * 24 + 24);
                bounds.push(match b {
                    Some(b) => {
                        let l = crate::reflect::rd64(b, 0) as i64;
                        let r = crate::reflect::rd64(b, 8) as i64;
                        let down = crate::reflect::rd64(b, 16) == 1;
                        let len = yavs_ir::dim_len(l, r, down)
                            .ok_or_else(|| Fault::Runtime("array too long".into()))?;
                        (l, !down, len)
                    }
                    None => {
                        let len = literal_len(m, ty, d, s)?;
                        let (left, down) = a
                            .get(1 + d * 2)
                            .zip(a.get(2 + d * 2))
                            .map(|(&l, &d)| (l as i64, d == 1))
                            .unwrap_or((0, false));
                        (left, !down, len)
                    }
                });
            }
            let ed = match elem_desc {
                Some(c) => const_bytes(&m.rt.prog, *c)?,
                None => desc.get(nd * 24..).unwrap_or(&[]).to_vec(),
            };
            let et = lit_type(m, *elem, &ed, s, &mut 0, a)?;
            let mut lt = et;
            for &(left, ascending, len) in bounds.iter().rev() {
                lt = LitType::Array {
                    left,
                    ascending,
                    len,
                    elem: Box::new(lt),
                };
            }
            lt
        }
        _ => return rt_err(format!("'value of this type {ty} is not implemented yet")),
    })
}

/// Length of the outermost open dimension. A string gives it directly; an
/// aggregate gives it through its positional elements or its named choices.
fn literal_len(m: &Machine, ty: TyId, dim: usize, s: &[u8]) -> R<u64> {
    let start = s
        .iter()
        .position(|&c| !yavs_std::value::is_ws(c))
        .unwrap_or(s.len());
    if s.get(start) != Some(&b'(') {
        if s.get(start) == Some(&b'"') && dim == 0 {
            let mut n = 0u64;
            let mut i = start + 1;
            while let Some(&c) = s.get(i) {
                match c {
                    b'"' if s.get(i + 1) == Some(&b'"') => {
                        n += 1;
                        i += 2;
                    }
                    b'"' => return Ok(n),
                    _ => {
                        n += 1;
                        i += 1;
                    }
                }
            }
            return rt_err("'value: '\"' closing the string");
        }
        return yavs_std::value::string_len(s, start)
            .map_err(|e| Fault::Runtime(format!("'value: {e}")));
    }
    let _ = (m, ty, dim);
    let mut depth = 0usize;
    let mut n = 0u64;
    let mut i = start + 1;
    while let Some(&c) = s.get(i) {
        match c {
            b'\'' if s.get(i + 2) == Some(&b'\'') => i += 2,
            b'"' => {
                i += 1;
                while i < s.len() && !(s[i] == b'"' && s.get(i + 1) != Some(&b'"')) {
                    i += if s[i] == b'"' { 2 } else { 1 };
                }
            }
            b'(' => depth += 1,
            b')' if depth == 0 => return Ok(n),
            b')' => depth -= 1,
            b',' if depth == 0 => n += 1,
            _ => {}
        }
        i += 1;
    }
    let _ = (m, ty, dim);
    rt_err("'value: ')' closing the aggregate")
}

/// Encode a parsed composite in the IR's value layout and return its fat pair.
fn encode_composite(
    m: &mut Machine,
    ty: TyId,
    lt: &yavs_std::value::LitType,
    v: &yavs_std::value::Lit,
) -> R<u64> {
    let mut desc = Vec::new();
    let mut data = Vec::new();
    encode_value(m, ty, lt, v, &mut desc, &mut data)?;
    let d = m.temp_alloc(data.len() as u64)?;
    m.put(d, &data)?;
    let p = m.temp_alloc(desc.len() as u64)?;
    m.put(p, &desc)?;
    if Ty::has_handles(&m.rt.prog.types, ty) {
        m.temp_root(d, Some(ty), desc.clone().into());
    }
    let fat = m.temp_alloc(16)?;
    m.store(fat, Width::B8, d)?;
    m.store(add_addr(fat, 8)?, Width::B8, p)?;
    Ok(fat)
}

fn encode_value(
    m: &Machine,
    ty: TyId,
    lt: &yavs_std::value::LitType,
    v: &yavs_std::value::Lit,
    desc: &mut Vec<u8>,
    out: &mut Vec<u8>,
) -> R<()> {
    use yavs_std::value::{Lit, LitType};
    let t =
        m.rt.prog
            .types
            .get(ty)
            .ok_or_else(|| ice(format!("no type {ty}")))?;
    match (t, lt, v) {
        (Ty::Int { .. }, LitType::Int(_), Lit::Int(x))
        | (Ty::Physical { .. }, LitType::Phys(..), Lit::Phys(x)) => {
            out.extend_from_slice(&x.to_le_bytes())
        }
        (Ty::Real, LitType::Real(_), Lit::Real(x)) => {
            out.extend_from_slice(&x.to_bits().to_le_bytes())
        }
        (Ty::Enum { lits, .. }, LitType::Enum(_), Lit::Enum(p)) => {
            let w = if lits.len() <= 256 { 1 } else { 4 };
            out.extend_from_slice(&(*p as u64).to_le_bytes()[..w]);
        }
        (Ty::Record { fields }, LitType::Record(fs), Lit::Record(xs)) => {
            let mut parts = Vec::with_capacity(fields.len());
            for (f, ((_, ft), x)) in fields.iter().zip(fs.iter().zip(xs)) {
                let mut fd = match f.desc {
                    Some(c) => const_bytes(&m.rt.prog, c)?,
                    None => Vec::new(),
                };
                let mut data = Vec::new();
                encode_value(m, f.ty, ft, x, &mut fd, &mut data)?;
                parts.push((f.ty, fd, data));
            }
            let full: Vec<u8> = parts.iter().flat_map(|p| p.1.clone()).collect();
            let (lay, _) =
                Ty::layout(&m.rt.prog.types, &m.rt.prog.consts, ty, &full).map_err(layout_err)?;
            let start = out.len();
            out.resize(start + lay.size as usize, 0);
            for (i, (_, fd, data)) in parts.into_iter().enumerate() {
                if fields[i].desc.is_none() {
                    desc.extend(fd);
                }
                let at = start + lay.offsets[i] as usize;
                out[at..at + data.len()].copy_from_slice(&data);
            }
        }
        (
            Ty::Array {
                elem,
                elem_desc,
                dims,
            },
            LitType::Array {
                left,
                ascending,
                len,
                elem: et,
            },
            Lit::Array(xs),
        ) => {
            let right = if *len == 0 {
                Some(if *ascending {
                    left.wrapping_sub(1)
                } else {
                    left.wrapping_add(1)
                })
            } else if *ascending {
                left.checked_add(*len as i64 - 1)
            } else {
                left.checked_sub(*len as i64 - 1)
            }
            .ok_or_else(|| Fault::Runtime("array bounds overflow".into()))?;
            for x in [*left, right, i64::from(!*ascending)] {
                desc.extend_from_slice(&x.to_le_bytes());
            }
            let mut data = Vec::new();
            let mut element_desc = Vec::new();
            if *dims > 1 {
                let LitType::Array { elem: row, .. } = &**et else {
                    return Err(ice("multi-dimensional parser type without rows"));
                };
                for x in xs {
                    let Lit::Array(row_values) = x else {
                        return Err(ice("multi-dimensional value without rows"));
                    };
                    for x in row_values {
                        encode_value(m, *elem, row, x, &mut element_desc, &mut data)?;
                    }
                }
            } else {
                encode_array_data(m, *elem, et, xs, &mut element_desc, &mut data)?;
            }
            if elem_desc.is_none() {
                if *dims > 1 {
                    let LitType::Array {
                        left,
                        ascending,
                        len,
                        ..
                    } = &**et
                    else {
                        return Err(ice("matrix row parser type"));
                    };
                    let right = if *ascending {
                        left + *len as i64 - 1
                    } else {
                        left - *len as i64 + 1
                    };
                    for x in [*left, right, i64::from(!*ascending)] {
                        desc.extend_from_slice(&x.to_le_bytes());
                    }
                }
                desc.extend_from_slice(&element_desc);
            }
            if Ty::logic_leaf(&m.rt.prog.types, ty).is_some() {
                out.extend(data);
            } else {
                let ed = match elem_desc {
                    Some(c) => const_bytes(&m.rt.prog, *c)?,
                    None => element_desc.clone(),
                };
                let (el, _) = Ty::layout(&m.rt.prog.types, &m.rt.prog.consts, *elem, &ed)
                    .map_err(layout_err)?;
                let stride = el.size.next_multiple_of(el.align.max(1)) as usize;
                let width =
                    m.rt.prog
                        .types
                        .get(*elem)
                        .and_then(Ty::elem_size)
                        .unwrap_or(el.size) as usize;
                let base = out.len();
                let count = if *dims > 1 {
                    xs.iter()
                        .map(|x| match x {
                            Lit::Array(row) => row.len(),
                            _ => 0,
                        })
                        .sum()
                } else {
                    xs.len()
                };
                out.resize(base + stride * count, 0);
                for (k, chunk) in data.chunks(width).enumerate() {
                    let at = base + k * stride;
                    out[at..at + chunk.len()].copy_from_slice(chunk);
                }
            }
        }
        _ => return Err(ice("'value: parsed value does not match its type")),
    }
    Ok(())
}

fn encode_array_data(
    m: &Machine,
    ty: TyId,
    lt: &yavs_std::value::LitType,
    xs: &[yavs_std::value::Lit],
    desc: &mut Vec<u8>,
    out: &mut Vec<u8>,
) -> R<()> {
    use yavs_std::value::Lit;
    if let Some(kind) = Ty::logic_leaf(&m.rt.prog.types, ty) {
        let mut codes = Vec::with_capacity(xs.len());
        for x in xs {
            let Lit::Enum(p) = x else {
                return Err(ice("'value: logic element is not an enumeration"));
            };
            let code = match kind {
                yavs_ir::LogicKind::StdULogic => *p as u8,
                _ => 2 | (*p as u8 & 1),
            };
            codes.push(yavs_logic::StdULogic::from_code_pub(code));
        }
        let v = yavs_kernel::LogicVec::from_fn(xs.len(), |i| codes[xs.len() - 1 - i]);
        for w in v.repr().blocks().as_flattened() {
            out.extend_from_slice(&w.to_le_bytes());
        }
        return Ok(());
    }
    for x in xs {
        encode_value(m, ty, lt, x, desc, out)?;
    }
    Ok(())
}

fn image_attr(
    m: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    ty: Option<TyId>,
    a: &[u64],
) -> R<Option<u64>> {
    text_result(m, ty, arg(a, 0)?, true).map(Some)
}

/// [`text`] as a string result (the fat pair in temporary memory); an integer is written from
/// a stack buffer.
pub(crate) fn text_result(m: &mut Machine, ty: Option<TyId>, v: u64, image: bool) -> R<u64> {
    match ty.and_then(|t| m.rt.prog.types.get(t)) {
        Some(Ty::Int { .. }) => {
            return m.ret_string(image::integer_digits(v as i64, &mut [0; 20]));
        }
        // An enumeration literal: its text is made once per machine.
        Some(Ty::Enum { lits, .. }) if (v as usize) < lits.len() => {
            if let Some(id) = ty {
                let n = lits.len();
                let at = match m.enum_text.iter().position(|e| e.0 == id && e.1 == image) {
                    Some(at) => at,
                    None => {
                        m.enum_text.push((id, image, vec![None; n]));
                        m.enum_text.len() - 1
                    }
                };
                if m.enum_text[at].2[v as usize].is_none() {
                    let s = scalar_text(m, ty, v, image)?;
                    m.enum_text[at].2[v as usize] = Some(s.into_boxed_slice());
                }
                let mut buf = [0u8; 64];
                let t = m.enum_text[at].2[v as usize].as_deref().unwrap_or(&[]);
                if t.len() <= buf.len() {
                    let n = t.len();
                    buf[..n].copy_from_slice(t);
                    return m.ret_string(&buf[..n]);
                }
                let t = t.to_vec();
                return m.ret_string(&t);
            }
        }
        _ => {}
    }
    let s = text(m, ty, v, image)?;
    m.ret_string(&s)
}

/// `'image` (`image = true`) or `to_string` of any value: scalars by value, arrays and
/// records as the address of their fat pair.
pub(crate) fn text(m: &Machine, ty: Option<TyId>, v: u64, image: bool) -> R<Vec<u8>> {
    let t = ty.ok_or_else(|| ice("image native without a type"))?;
    match m.rt.prog.types.get(t) {
        Some(Ty::Array { .. } | Ty::Record { .. }) => {}
        _ => return scalar_text(m, ty, v, image),
    }
    let prog = &m.rt.prog;
    let data = m.load(v, Width::B8)?;
    let dp = m.load(crate::exec::add_addr(v, 8)?, Width::B8)?;
    let n = Ty::desc_size(&prog.types, t).ok_or_else(|| ice("descriptor size"))?;
    let desc = if n == 0 { &[] } else { m.rd(dp, n)? };
    let (lay, _) = Ty::layout(&prog.types, &prog.consts, t, desc).map_err(layout_err)?;
    composite_text(m, t, desc, m.rd(data, lay.size)?, image, false)
}

/// Whether arrays of the enumeration `lits` print as a string (`"01"`, `TO_STRING` its
/// characters): every literal a character literal, or CHARACTER (whose other literals are the
/// control character names).
fn string_like(lits: &[String]) -> bool {
    lits.iter().all(|l| l.starts_with('\''))
        || (lits.len() == 256 && lits[0].eq_ignore_ascii_case("nul"))
}

/// The text of one value of `ty` given its descriptor and bytes (LRM 2019 16.2.2 `'IMAGE`,
/// 5.7 `TO_STRING`): a record is `(e1,e2)`, an array `(e1,e2)` per dimension, a
/// one-dimensional row of a string-like element type whose values are all character
/// literals a string literal (`'IMAGE`) or its characters (`TO_STRING`). An array element
/// that is a character literal prints without quotes (`bare`). docs/design/lrm-checks.md.
fn composite_text(
    m: &Machine,
    ty: TyId,
    desc: &[u8],
    val: &[u8],
    image: bool,
    bare: bool,
) -> R<Vec<u8>> {
    let prog = &m.rt.prog;
    let (types, consts) = (&prog.types, &prog.consts);
    let t = types.get(ty).ok_or_else(|| ice(format!("no type {ty}")))?;
    if let Some(w) = t.scalar_size() {
        let v = le(val.get(..w as usize).unwrap_or(val));
        if let (true, Ty::Enum { lits, .. }) = (bare, t)
            && let Some(EnumLit::Char(c)) = lits.get(v as usize).map(|l| lit(l))
        {
            return Ok(vec![c]);
        }
        return scalar_text(m, Some(ty), v, image);
    }
    let mut out = Vec::new();
    match t {
        Ty::Record { fields } => {
            let (lay, _) = Ty::layout(types, consts, ty, desc).map_err(layout_err)?;
            let mut cur = 0;
            out.push(b'(');
            for (i, f) in fields.iter().enumerate() {
                let fdesc = match f.desc {
                    Some(c) => const_bytes(prog, c)?,
                    None => {
                        let n = Ty::desc_size(types, f.ty)
                            .ok_or_else(|| ice("record field type"))?
                            as usize;
                        cur += n;
                        desc.get(cur - n..cur).unwrap_or(&[]).to_vec()
                    }
                };
                let (fl, _) = Ty::layout(types, consts, f.ty, &fdesc).map_err(layout_err)?;
                let at = lay.offsets[i] as usize;
                let fv = val
                    .get(at..at + fl.size as usize)
                    .ok_or_else(|| ice("record field out of its value"))?;
                if i > 0 {
                    out.push(b',');
                }
                out.extend(composite_text(m, f.ty, &fdesc, fv, image, false)?);
            }
            out.push(b')');
        }
        Ty::Array {
            elem,
            elem_desc,
            dims,
        } => {
            let nd = usize::from(*dims);
            let (lay, _) = Ty::layout(types, consts, ty, desc).map_err(layout_err)?;
            let edesc = match elem_desc {
                Some(c) => const_bytes(prog, *c)?,
                None => desc.get(nd * 24..).unwrap_or(&[]).to_vec(),
            };
            let (el, _) = Ty::layout(types, consts, *elem, &edesc).map_err(layout_err)?;
            let mut lens = Vec::with_capacity(nd);
            for d in 0..nd {
                let r = |k: usize| crate::reflect::rd64(desc, d * 24 + k * 8);
                let len = yavs_ir::dim_len(r(0) as i64, r(1) as i64, r(2) == 1)
                    .ok_or_else(|| Fault::Runtime("array too long".into()))?;
                lens.push(len);
            }
            let et = types.get(*elem).ok_or_else(|| ice("array element type"))?;
            let scalar = et.scalar_size().is_some();
            let logic = Ty::logic_leaf(types, ty);
            // Element `k` (row major) as the bytes `composite_text` takes.
            let elem_val = |k: u64| -> R<Vec<u8>> {
                if let Some(kind) = logic {
                    if scalar {
                        let code = plane_get(val, lay.count - 1 - k);
                        return Ok(logic_pos(kind, code).to_le_bytes().to_vec());
                    }
                    let start = lay.count - (k + 1) * el.count;
                    let mut b = vec![0u8; el.size as usize];
                    for j in 0..el.count {
                        plane_set(&mut b, j, plane_get(val, start + j));
                    }
                    return Ok(b);
                }
                let at = (k * lay.stride) as usize;
                // A scalar element may be narrower than the scalar (`Ty::elem_size`).
                let n = et.elem_size().unwrap_or(el.size) as usize;
                let b = val
                    .get(at..at + n)
                    .ok_or_else(|| ice("array element out of its value"))?;
                Ok(if scalar {
                    crate::exec::cell(b).to_le_bytes().to_vec()
                } else {
                    b.to_vec()
                })
            };
            let chars = match et {
                Ty::Enum { lits, .. } if string_like(lits) => Some(lits),
                _ => None,
            };
            let row = lens.last().copied().unwrap_or(0);
            // A string-like row of scalars (the usual `to_string` and `'image` of a vector or
            // a string) is spelled straight from the element codes, with no per-element text.
            if let (1, Some(lits), true) = (nd, chars, scalar) {
                let n = et.elem_size().unwrap_or(el.size) as usize;
                let mut s = Vec::with_capacity(row as usize + 2);
                if image {
                    s.push(b'"');
                }
                let all = (0..row).all(|k| {
                    let code = match logic {
                        Some(kind) => logic_pos(kind, plane_get(val, lay.count - 1 - k)),
                        None => {
                            let at = (k * lay.stride) as usize;
                            val.get(at..at + n).map_or(u64::MAX, crate::exec::cell)
                        }
                    };
                    match lits.get(code as usize).map(|l| lit(l)) {
                        Some(EnumLit::Char(c)) => {
                            s.push(c);
                            if image && c == b'"' {
                                s.push(c);
                            }
                            true
                        }
                        _ => false,
                    }
                });
                if all {
                    if image {
                        s.push(b'"');
                    }
                    return Ok(s);
                }
            }
            let rows: u64 = lens[..nd.saturating_sub(1)].iter().product();
            let mut texts = Vec::new();
            for r in 0..rows {
                let mut cs = Vec::new();
                let mut items = Vec::new();
                for k in r * row..(r + 1) * row {
                    let b = elem_val(k)?;
                    if let Some(lits) = chars
                        && let Some(EnumLit::Char(c)) = lits.get(le(&b) as usize).map(|l| lit(l))
                    {
                        cs.push(c);
                    }
                    items.push(composite_text(m, *elem, &edesc, &b, image, true)?);
                }
                texts.push(if chars.is_some() && cs.len() as u64 == row {
                    if image {
                        let mut s = vec![b'"'];
                        for c in cs {
                            s.push(c);
                            if c == b'"' {
                                s.push(c);
                            }
                        }
                        s.push(b'"');
                        s
                    } else {
                        cs
                    }
                } else {
                    paren(items)
                });
            }
            // Group the rows by the outer dimensions, innermost first.
            for &len in lens[..nd.saturating_sub(1)].iter().rev() {
                texts = texts
                    .chunks(len.max(1) as usize)
                    .map(|c| paren(c.to_vec()))
                    .collect();
            }
            out = texts.into_iter().next().unwrap_or_else(|| b"()".to_vec());
        }
        _ => return Err(ice(format!("text of type {ty}"))),
    }
    Ok(out)
}

fn paren(items: Vec<Vec<u8>>) -> Vec<u8> {
    let mut s = vec![b'('];
    s.extend(items.join(&b','));
    s.push(b')');
    s
}

// ---- std.env ----

use crate::textio::arg as targ;
use yavs_ir::Width;
use yavs_std::env;

fn resolution_limit(
    m: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    _: &[u64],
) -> R<Option<u64>> {
    Ok(Some(env::resolution_limit(m.rt.resolution) as u64))
}

fn getenv_string(
    m: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    a: &[u64],
) -> R<Option<u64>> {
    let name = m.fat_bytes(targ(a, 0)?)?;
    let v = env::getenv(&name).unwrap_or_default();
    m.ret_string(&v).map(Some)
}

fn getenv_line(
    m: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    a: &[u64],
) -> R<Option<u64>> {
    let name = m.fat_bytes(targ(a, 0)?)?;
    match env::getenv(&name) {
        // A result is not stored in shared memory: the private heap.
        Some(v) => m.new_line(0, &v).map(Some),
        None => Ok(Some(0)),
    }
}

macro_rules! const_strings {
    ($($name:ident => $v:expr),*) => {$(
        fn $name(m: &mut Machine, _: &mut Option<&mut Ctx<'_>>, _: Option<TyId>, _: &[u64]) -> R<Option<u64>> {
            let s: &str = $v(m);
            m.ret_string(s.as_bytes()).map(Some)
        }
    )*};
}

const_strings! {
    vhdl_version => |m: &Machine| env::vhdl_version(m.rt.revision),
    tool_type => |_: &Machine| env::TOOL_TYPE,
    tool_vendor => |_: &Machine| env::TOOL_VENDOR,
    tool_name => |_: &Machine| env::TOOL_NAME,
    tool_edition => |_: &Machine| env::TOOL_EDITION,
    tool_version => |_: &Machine| env::TOOL_VERSION
}

fn epoch(
    _: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    _: &[u64],
) -> R<Option<u64>> {
    let d = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_err(|_| Fault::Runtime("system clock is before 1970".into()))?;
    Ok(Some(d.as_secs_f64().to_bits()))
}

fn time_to_seconds(
    _: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    a: &[u64],
) -> R<Option<u64>> {
    Ok(Some(env::time_to_seconds(targ(a, 0)? as i64).to_bits()))
}

fn seconds_to_time(
    _: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    a: &[u64],
) -> R<Option<u64>> {
    let t = env::seconds_to_time(f64::from_bits(targ(a, 0)?))
        .map_err(|e| Fault::Runtime(e.to_string()))?;
    Ok(Some(t as u64))
}

/// `DIR_OPEN (Dir, Path)`: DIRECTORY is `(Name: line; Items: access line_vector)`, two
/// handles, passed as a fat pair like every composite; Items is `[0 to n - 1][line handles]`.
fn dir_open(m: &mut Machine, a: &[u64]) -> R<u64> {
    let dir = crate::env::record_arg(m, targ(a, 0)?)?;
    let path = m.fat_bytes(targ(a, 1)?)?;
    let d = match env::dir_open(&path) {
        Ok(d) => d,
        Err(s) => return Ok(s as u64),
    };
    m.line_set(dir, &d.name)?;
    let items_cell = crate::exec::add_addr(dir, 8)?;
    let mut obj: Vec<u8> = [0i64, d.items.len() as i64 - 1, 0]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    for it in &d.items {
        let h = m.new_line(items_cell, it)?;
        obj.extend(h.to_le_bytes());
    }
    let h = m.alloc_line_vector(items_cell, obj)?;
    m.store(items_cell, Width::B8, h)?;
    Ok(env::DirOpenStatus::Ok as u64)
}

fn dir_open_proc(
    m: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    a: &[u64],
) -> R<Option<u64>> {
    let s = dir_open(m, a)?;
    m.store(targ(a, 2)?, Width::B1, s)?;
    Ok(None)
}

fn dir_open_fn(
    m: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    a: &[u64],
) -> R<Option<u64>> {
    dir_open(m, a).map(Some)
}

/// `DIR_CLOSE (Dir)`: deallocates the item names and the item vector.
fn dir_close(
    m: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    a: &[u64],
) -> R<Option<u64>> {
    let dir = crate::env::record_arg(m, targ(a, 0)?)?;
    let items = m.load(crate::exec::add_addr(dir, 8)?, Width::B8)?;
    if items != 0 {
        let lines = m.line_vector_handles(items)?;
        for h in lines {
            m.free_handle(h)?;
        }
        m.free_handle(items)?;
    }
    Ok(None)
}

fn path_test(m: &mut Machine, a: &[u64], f: fn(&[u8]) -> bool) -> R<Option<u64>> {
    let p = m.fat_bytes(targ(a, 0)?)?;
    Ok(Some(f(&p).into()))
}

fn dir_itemexists(
    m: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    a: &[u64],
) -> R<Option<u64>> {
    path_test(m, a, env::dir_itemexists)
}
fn dir_itemisdir(
    m: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    a: &[u64],
) -> R<Option<u64>> {
    path_test(m, a, env::dir_itemisdir)
}
fn dir_itemisfile(
    m: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    a: &[u64],
) -> R<Option<u64>> {
    path_test(m, a, env::dir_itemisfile)
}

fn workingdir(
    m: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    _: &[u64],
) -> R<Option<u64>> {
    let d = env::dir_workingdir().map_err(|e| Fault::Runtime(format!("dir_workingdir: {e}")))?;
    m.ret_string(&d).map(Some)
}

/// A path operation with a status: the procedure form stores it in the last argument, the
/// function form returns it. `flag` is the optional boolean (Parents, Recursive) after the
/// path, given when the call has it.
fn status_op(
    m: &mut Machine,
    a: &[u64],
    proc_form: bool,
    f: fn(&[u8], bool) -> u64,
) -> R<Option<u64>> {
    let p = m.fat_bytes(targ(a, 0)?)?;
    let n = a.len() - usize::from(proc_form);
    let flag = n == 2 && targ(a, 1)? != 0;
    let s = f(&p, flag);
    if proc_form {
        m.store(targ(a, a.len() - 1)?, Width::B1, s)?;
        Ok(None)
    } else {
        Ok(Some(s))
    }
}

fn set_wd(p: &[u8], _: bool) -> u64 {
    env::dir_set_workingdir(p) as u64
}
fn mkdir(p: &[u8], parents: bool) -> u64 {
    env::dir_createdir(p, parents) as u64
}
fn rmdir(p: &[u8], recursive: bool) -> u64 {
    env::dir_deletedir(p, recursive) as u64
}
fn rm(p: &[u8], _: bool) -> u64 {
    env::dir_deletefile(p) as u64
}

macro_rules! status_natives {
    ($($name:ident => ($proc:expr, $f:ident)),*) => {$(
        fn $name(m: &mut Machine, _: &mut Option<&mut Ctx<'_>>, _: Option<TyId>, a: &[u64]) -> R<Option<u64>> {
            status_op(m, a, $proc, $f)
        }
    )*};
}

status_natives! {
    set_workingdir_proc => (true, set_wd),
    set_workingdir_fn => (false, set_wd),
    createdir_proc => (true, mkdir),
    createdir_fn => (false, mkdir),
    deletedir_proc => (true, rmdir),
    deletedir_fn => (false, rmdir),
    deletefile_proc => (true, rm),
    deletefile_fn => (false, rm)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbols() {
        assert!(lookup("yavs.env.stop.integer").unwrap().is_some());
        assert!(lookup("yavs.implicit.to_string.integer").unwrap().is_some());
        assert!(lookup("yavs.attr.image").unwrap().is_some());
        assert!(lookup("yavs.implicit.file_open.text").unwrap().is_some());
        // Families sem declares (#163): all valid and implemented.
        for f in ["to_hstring", "to_ostring", "file_canseek"] {
            let s = format!("yavs.implicit.{f}.std.standard.bit_vector");
            assert!(lookup(&s).unwrap().is_some(), "{s}");
        }
        assert!(
            lookup("yavs.textio.read.line_integer_boolean")
                .unwrap()
                .is_some()
        );
        assert!(lookup("yavs.textio.read.line_boolean").unwrap().is_some());
        assert!(
            lookup("yavs.env.dir_open.directory_string.r_dir_open_status")
                .unwrap()
                .is_some()
        );
        // Defined in NATIVE.md, not implemented yet.
        assert!(
            lookup("yavs.env.pslassertfailed.void.r_boolean")
                .unwrap()
                .is_none()
        );
        let e = lookup("yavs.env.bogus.void").unwrap_err();
        assert!(e.contains("yavs.env.bogus.void"), "{e}");
    }
}
