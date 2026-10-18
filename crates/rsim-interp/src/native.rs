//! Natives: the dispatch table keyed by `lib/NATIVE.md` symbol, wired to rsim-std.
//!
//! A symbol that NATIVE.md does not define is an error when the [`crate::Runtime`] is built
//! (fail early). A symbol that is defined but not implemented here yet is a runtime error
//! naming it when it is called.
//!
//! Calling convention: arguments are cells (scalars by value, composites by address); string
//! results are returned as the address of a fat pair `[data, dope]` in temporary memory.
//! Implicit-family natives (`rsim.implicit.<family>.<type>`) and attribute natives
//! (`rsim.attr.<name>`, an rsim addition) get the type from `NativeRef::ty`.

use std::collections::HashSet;
use std::sync::OnceLock;

use rsim_ir::{Ty, TyId};
use rsim_kernel::Ctx;
use rsim_std::image;
use rsim_std::types::{EnumLit, PhysicalType};

use crate::exec::{Fault, Machine, R, ice, rt_err};

pub(crate) type NativeFn =
    fn(&mut Machine, &mut Option<&mut Ctx<'_>>, Option<TyId>, &[u64]) -> R<Option<u64>>;

const NATIVE_MD: &str = include_str!("../../../lib/NATIVE.md");

/// Attribute natives rsim defines beyond NATIVE.md.
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
            let implicit = line.contains("`rsim.implicit.");
            for (i, tok) in line.split('`').enumerate() {
                if i % 2 == 0 {
                    continue;
                }
                if let Some(f) = tok.strip_prefix("rsim.implicit.") {
                    if !f.contains('<') {
                        families.insert(f);
                    }
                } else if implicit && let Some(f) = tok.strip_prefix('.') {
                    families.insert(f);
                } else if tok.starts_with("rsim.") && !tok.contains('<') {
                    explicit.insert(tok);
                }
            }
        }
        Known { explicit, families }
    })
}

/// The implementation of `symbol`: `Ok(None)` when NATIVE.md defines it but rsim does not
/// implement it yet, `Err` when it is not a native symbol at all.
pub(crate) fn lookup(symbol: &str) -> Result<Option<NativeFn>, String> {
    let k = known();
    let family = symbol
        .strip_prefix("rsim.implicit.")
        .map(|r| r.split('.').next().unwrap_or(r));
    let attr = symbol.strip_prefix("rsim.attr.");
    let valid = k.explicit.contains(symbol)
        || family.is_some_and(|f| k.families.contains(f))
        || attr.is_some_and(|a| ATTRS.contains(&a));
    if !valid {
        return Err(format!(
            "`{symbol}` is not a native subprogram (lib/NATIVE.md defines the symbols)"
        ));
    }
    let f: NativeFn = match (symbol, family, attr) {
        ("rsim.standard.now.void.r_delay_length", _, _) => now,
        ("rsim.env.stop.void", _, _) => stop0,
        ("rsim.env.stop.integer", _, _) => stop1,
        ("rsim.env.finish.void", _, _) => finish0,
        ("rsim.env.finish.integer", _, _) => finish1,
        (_, Some("to_string"), _) => to_string,
        (_, _, Some("image")) => image_attr,
        _ => return Ok(None),
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
    Ok(Some(kctx(ctx)?.now().as_fs()))
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

fn lit(s: &str) -> EnumLit {
    let b = s.as_bytes();
    if b.len() == 3 && b[0] == b'\'' && b[2] == b'\'' {
        EnumLit::Char(b[1])
    } else {
        EnumLit::ident(s)
    }
}

/// `'image` (`image = true`) or `to_string` of a scalar.
fn scalar_text(m: &Machine, ty: Option<TyId>, v: u64, image: bool) -> R<Vec<u8>> {
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

fn image_attr(
    m: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    ty: Option<TyId>,
    a: &[u64],
) -> R<Option<u64>> {
    let s = scalar_text(m, ty, arg(a, 0)?, true)?;
    m.ret_string(&s).map(Some)
}

fn to_string(
    m: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    ty: Option<TyId>,
    a: &[u64],
) -> R<Option<u64>> {
    let s = scalar_text(m, ty, arg(a, 0)?, false)?;
    m.ret_string(&s).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn symbols() {
        assert!(lookup("rsim.env.stop.integer").unwrap().is_some());
        assert!(lookup("rsim.implicit.to_string.integer").unwrap().is_some());
        assert!(lookup("rsim.attr.image").unwrap().is_some());
        // Defined in NATIVE.md, not implemented yet.
        assert!(lookup("rsim.implicit.file_open.text").unwrap().is_none());
        let e = lookup("rsim.env.bogus.void").unwrap_err();
        assert!(e.contains("rsim.env.bogus.void"), "{e}");
    }
}
