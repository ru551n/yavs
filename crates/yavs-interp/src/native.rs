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

use crate::exec::{Fault, Machine, R, ice, rt_err};

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

/// The implementation of `symbol`: `Ok(None)` when NATIVE.md defines it but yavs does not
/// implement it yet, `Err` when it is not a native symbol at all.
pub(crate) fn lookup(symbol: &str) -> Result<Option<NativeFn>, String> {
    if let Some(f) = crate::reflect::lookup(symbol) {
        return Ok(Some(f));
    }
    let k = known();
    let family = symbol
        .strip_prefix("yavs.implicit.")
        .map(|r| r.split('.').next().unwrap_or(r));
    let attr = symbol.strip_prefix("yavs.attr.");
    let valid = k.explicit.contains(symbol)
        || family.is_some_and(|f| k.families.contains(f))
        || attr.is_some_and(|a| ATTRS.contains(&a));
    if !valid {
        return Err(format!(
            "`{symbol}` is not a native subprogram (lib/NATIVE.md defines the symbols)"
        ));
    }
    if let Some(f) = crate::implicit::lookup(symbol).or_else(|| crate::env::lookup(symbol)) {
        return Ok(Some(f));
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
        _ => match crate::textio::lookup(symbol, family) {
            Some(f) => f,
            None => return Ok(None),
        },
    };
    Ok(Some(f))
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

/// `T'value(s)` of a scalar type: the string as a fat pair, `ty` the base type.
fn value_attr(
    m: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    ty: Option<TyId>,
    a: &[u64],
) -> R<Option<u64>> {
    use yavs_std::types::{Bounds, EnumType, RealBounds};
    use yavs_std::value;
    let s = m.fat_bytes(arg(a, 0)?)?;
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
        _ => return rt_err(format!("'value of a non-scalar type {ty}")),
    };
    r.map(Some)
        .map_err(|e| Fault::Runtime(format!("'value: {e}")))
}

fn image_attr(
    m: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    ty: Option<TyId>,
    a: &[u64],
) -> R<Option<u64>> {
    let s = scalar_text(m, ty, arg(a, 0)?, true)?;
    m.ret_string(&s).map(Some)
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
