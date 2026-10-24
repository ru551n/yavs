//! Natives of the implicitly declared operations: `yavs.implicit.<family>.<type>[#<form>]`
//! (lib/NATIVE.md section 2; families and forms are yavs-sem's `implicit::Family`/`Form`).
//! The file families (`file_open` .. `file_canseek`) are in `textio.rs`.
//!
//! Calling convention (on top of `native.rs`): `NativeRef::ty` is the type the operation was
//! declared for (`ImplicitOp::ty`); arguments are cells in formal order, scalars by value
//! (bit and boolean as positions 0/1, reals as `f64` bits), arrays as the address of a fat pair
//! `[data, dope]`; array results are a fat pair in temporary memory. Additions:
//!
//! - `cat` takes three more cells after its operands: `S'LEFT`, `S'RIGHT` and the direction
//!   (0 `to`, 1 `downto`) of the index subtype `S` of the array type (LRM 9.2.5 needs it and
//!   `Ty::Array` does not carry it).
//! - `rising_edge`/`falling_edge` take the signal as its two `Sig` cells.
//! - `deallocate` takes the address of the access object (it sets it to null).
//!
//! BIT and BOOLEAN elements of bit-plane arrays are read as positions 0/1 or as the codes of
//! '0'/'1', and written as the codes 2/3 (docs/decisions.md). Only arrays of scalar elements
//! are handled; others are a "not implemented" runtime error, as lowering does not produce them
//! (record equality is inlined, tests/corpus/implicit/record_eq).

use std::cmp::Ordering;

use yavs_ir::{DOPE_DIM_BYTES, LogicKind, Ty, TyId, Width, dim_len};
use yavs_kernel::{Ctx, Elem, LogicVec};
use yavs_logic::StdULogic;
use yavs_std::types::PhysicalType;
use yavs_std::{arith, image};

use crate::exec::{Fault, Machine, R, add_addr, ice, rt_err};
use crate::native::NativeFn;

/// The native of an implicit operation symbol, `None` for symbols this module does not own.
pub(crate) fn lookup(symbol: &str) -> Option<NativeFn> {
    let rest = symbol.strip_prefix("yavs.implicit.")?;
    let (fam, ty) = rest.split_once('.')?;
    // The form follows `#`, which no type name contains.
    let form = ty
        .split_once('#')
        .map(|(_, f)| f)
        .filter(|f| FORMS.contains(f));
    let f: NativeFn = match (fam, form.unwrap_or("")) {
        ("eq", "") => |m, _, t, a| rel(m, t, a, Rel::Eq),
        ("neq", "") => |m, _, t, a| rel(m, t, a, Rel::Ne),
        ("lt", "") => |m, _, t, a| rel(m, t, a, Rel::Lt),
        ("le", "") => |m, _, t, a| rel(m, t, a, Rel::Le),
        ("gt", "") => |m, _, t, a| rel(m, t, a, Rel::Gt),
        ("ge", "") => |m, _, t, a| rel(m, t, a, Rel::Ge),
        ("minimum", "") => |m, _, t, a| minmax(m, t, a, false),
        ("maximum", "") => |m, _, t, a| minmax(m, t, a, true),
        ("minimum", "reduce") => |m, _, t, a| reduce_minmax(m, t, a, false),
        ("maximum", "reduce") => |m, _, t, a| reduce_minmax(m, t, a, true),
        ("add", "") => |m, _, t, a| arith_op(m, t, a, Ar::Add, ""),
        ("sub", "") => |m, _, t, a| arith_op(m, t, a, Ar::Sub, ""),
        ("mul", "") => |m, _, t, a| arith_op(m, t, a, Ar::Mul, ""),
        ("mul", "pi") => |m, _, t, a| arith_op(m, t, a, Ar::Mul, "pi"),
        ("mul", "pr") => |m, _, t, a| arith_op(m, t, a, Ar::Mul, "pr"),
        ("mul", "ip") => |m, _, t, a| arith_op(m, t, a, Ar::Mul, "ip"),
        ("mul", "rp") => |m, _, t, a| arith_op(m, t, a, Ar::Mul, "rp"),
        ("mul", "ri") => |m, _, t, a| arith_op(m, t, a, Ar::Mul, "ri"),
        ("mul", "ir") => |m, _, t, a| arith_op(m, t, a, Ar::Mul, "ir"),
        ("div", "") => |m, _, t, a| arith_op(m, t, a, Ar::Div, ""),
        ("div", "pi") => |m, _, t, a| arith_op(m, t, a, Ar::Div, "pi"),
        ("div", "pr") => |m, _, t, a| arith_op(m, t, a, Ar::Div, "pr"),
        ("div", "pp") => |m, _, t, a| arith_op(m, t, a, Ar::Div, "pp"),
        ("div", "ri") => |m, _, t, a| arith_op(m, t, a, Ar::Div, "ri"),
        ("mod", "") => |m, _, t, a| arith_op(m, t, a, Ar::Mod, ""),
        ("rem", "") => |m, _, t, a| arith_op(m, t, a, Ar::Rem, ""),
        ("exp", "") => |m, _, t, a| arith_op(m, t, a, Ar::Exp, ""),
        ("neg", "") => |m, _, t, a| arith_op(m, t, a, Ar::Neg, ""),
        ("pos", "") => |m, _, t, a| arith_op(m, t, a, Ar::Pos, ""),
        ("abs", "") => |m, _, t, a| arith_op(m, t, a, Ar::Abs, ""),
        ("and", f) => logical_for(Lg::And, f)?,
        ("or", f) => logical_for(Lg::Or, f)?,
        ("nand", f) => logical_for(Lg::Nand, f)?,
        ("nor", f) => logical_for(Lg::Nor, f)?,
        ("xor", f) => logical_for(Lg::Xor, f)?,
        ("xnor", f) => logical_for(Lg::Xnor, f)?,
        ("not", "") => |m, _, t, a| logical(m, t, a, Lg::Not, Shape::Plain),
        ("sll", "") => |m, _, t, a| shift(m, t, a, Sh::Sll),
        ("srl", "") => |m, _, t, a| shift(m, t, a, Sh::Srl),
        ("sla", "") => |m, _, t, a| shift(m, t, a, Sh::Sla),
        ("sra", "") => |m, _, t, a| shift(m, t, a, Sh::Sra),
        ("rol", "") => |m, _, t, a| shift(m, t, a, Sh::Rol),
        ("ror", "") => |m, _, t, a| shift(m, t, a, Sh::Ror),
        ("cat", "") => |m, _, t, a| cat(m, t, a, false, false),
        ("cat", "as") => |m, _, t, a| cat(m, t, a, false, true),
        ("cat", "sa") => |m, _, t, a| cat(m, t, a, true, false),
        ("cat", "ss") => |m, _, t, a| cat(m, t, a, true, true),
        ("meq", "") => |m, _, t, a| matching(m, t, a, Rel::Eq),
        ("mneq", "") => |m, _, t, a| matching(m, t, a, Rel::Ne),
        ("mlt", "") => |m, _, t, a| matching(m, t, a, Rel::Lt),
        ("mle", "") => |m, _, t, a| matching(m, t, a, Rel::Le),
        ("mgt", "") => |m, _, t, a| matching(m, t, a, Rel::Gt),
        ("mge", "") => |m, _, t, a| matching(m, t, a, Rel::Ge),
        ("cond", "") => |_, _, _, a| Ok(Some(u64::from(bit(arg(a, 0)?)? == 1))),
        ("to_string", "") => to_string,
        ("to_string", "digits") => to_string_digits,
        ("to_string", "format") => to_string_format,
        ("to_string", "unit") => to_string_unit,
        ("to_hstring", "") => |m, _, t, a| radix(m, t, a, 4, image::bits_to_hstring),
        ("to_ostring", "") => |m, _, t, a| radix(m, t, a, 3, image::bits_to_ostring),
        ("rising_edge", "") => |m, c, _, a| edge(m, c, a, 1),
        ("falling_edge", "") => |m, c, _, a| edge(m, c, a, 0),
        ("deallocate", "") => |m, _, _, a| m.deallocate(arg(a, 0)?).map(|()| None),
        _ => return None,
    };
    Some(f)
}

const FORMS: &[&str] = &[
    "as", "sa", "ss", "reduce", "pi", "pr", "ip", "rp", "pp", "ri", "ir", "status", "function",
    "digits", "format", "unit",
];

fn arg(a: &[u64], i: usize) -> R<u64> {
    crate::textio::arg(a, i)
}

fn ty_of(m: &Machine, t: Option<TyId>) -> R<(TyId, &Ty)> {
    let t = t.ok_or_else(|| ice("implicit native without a type"))?;
    let ty =
        m.rt.prog
            .types
            .get(t)
            .ok_or_else(|| ice(format!("no type {t}")))?;
    Ok((t, ty))
}

// ---- scalars ----

/// The class of a scalar type, as the natives need it.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Sc {
    Int(i64, i64),
    Phys(i64, i64),
    Real,
    /// Literal count and logic kind.
    Enum(u64, Option<LogicKind>),
}

fn scalar(m: &Machine, t: TyId) -> R<Sc> {
    Ok(match *ty_of(m, Some(t))?.1 {
        Ty::Int { lo, hi } => Sc::Int(lo, hi),
        Ty::Physical { lo, hi, .. } => Sc::Phys(lo, hi),
        Ty::Real => Sc::Real,
        Ty::Enum { ref lits, logic } => Sc::Enum(lits.len() as u64, logic),
        Ty::Access { .. } => Sc::Enum(u64::MAX, None),
        _ => {
            return rt_err(format!(
                "predefined operations on arrays of composite elements ({t}) are not \
                 implemented yet"
            ));
        }
    })
}

/// A BIT or BOOLEAN value: position 0/1 or the std_ulogic code of '0'/'1'.
fn bit(v: u64) -> R<u64> {
    match v {
        0 | 2 => Ok(0),
        1 | 3 => Ok(1),
        _ => Err(ice(format!("BIT value with code {v}"))),
    }
}

fn order(sc: Sc, a: u64, b: u64) -> R<Ordering> {
    Ok(match sc {
        Sc::Int(..) | Sc::Phys(..) => (a as i64).cmp(&(b as i64)),
        Sc::Real => f64::from_bits(a)
            .partial_cmp(&f64::from_bits(b))
            .ok_or_else(|| ice("NaN in a real comparison"))?,
        Sc::Enum(_, Some(LogicKind::Bit | LogicKind::Boolean)) => bit(a)?.cmp(&bit(b)?),
        Sc::Enum(..) => a.cmp(&b),
    })
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Rel {
    Eq,
    Ne,
    Lt,
    Le,
    Gt,
    Ge,
}

impl Rel {
    fn holds(self, o: Ordering) -> bool {
        match self {
            Rel::Eq => o.is_eq(),
            Rel::Ne => o.is_ne(),
            Rel::Lt => o.is_lt(),
            Rel::Le => o.is_le(),
            Rel::Gt => o.is_gt(),
            Rel::Ge => o.is_ge(),
        }
    }
}

// ---- arrays ----

/// A decoded array of scalars: bounds per dimension and the elements, row major, leftmost
/// first (bit and boolean elements as 0/1).
#[derive(Clone, Debug, PartialEq)]
struct Arr {
    dims: Vec<(i64, i64, bool)>,
    el: Vec<u64>,
}

impl Arr {
    fn one(left: i64, right: i64, down: bool, el: Vec<u64>) -> Arr {
        Arr {
            dims: vec![(left, right, down)],
            el,
        }
    }
}

/// Element type, its class and the dimension count of the array type `t`.
fn array_ty(m: &Machine, t: TyId) -> R<(TyId, Sc, u8)> {
    match *ty_of(m, Some(t))?.1 {
        Ty::Array { elem, dims, .. } => Ok((elem, scalar(m, elem)?, dims)),
        _ => Err(ice(format!("array native on non-array type {t}"))),
    }
}

fn is_array(m: &Machine, t: TyId) -> R<bool> {
    Ok(matches!(ty_of(m, Some(t))?.1, Ty::Array { .. }))
}

fn logic_of(sc: Sc) -> Option<LogicKind> {
    match sc {
        Sc::Enum(_, l) => l,
        _ => None,
    }
}

/// Bytes of one element (`Ty::elem_size`).
fn elem_size(sc: Sc) -> u64 {
    match sc {
        Sc::Enum(n, _) if n <= 256 => 1,
        Sc::Enum(n, _) if n != u64::MAX => 4,
        Sc::Int(lo, hi) if Ty::Int { lo, hi }.elem_size() == Some(4) => 4,
        _ => 8,
    }
}

fn read_arr(m: &Machine, t: TyId, fat: u64) -> R<Arr> {
    let (_, sc, nd) = array_ty(m, t)?;
    let data = m.load(fat, Width::B8)?;
    let dope = m.load(add_addr(fat, 8)?, Width::B8)?;
    let mut dims = Vec::new();
    let mut n: u64 = 1;
    for d in 0..u64::from(nd) {
        let at = |k: u64| add_addr(dope, d * DOPE_DIM_BYTES + k * 8);
        let l = m.load(at(0)?, Width::B8)? as i64;
        let r = m.load(at(1)?, Width::B8)? as i64;
        let down = m.load(at(2)?, Width::B8)? == 1;
        let len = dim_len(l, r, down).ok_or_else(|| Fault::Runtime("array too long".into()))?;
        n = n
            .checked_mul(len)
            .ok_or_else(|| Fault::Runtime("array too long".into()))?;
        dims.push((l, r, down));
    }
    let el = match logic_of(sc) {
        Some(k) => {
            let v = m.planes(data, n)?;
            let nn = n as usize;
            (0..nn)
                .map(|i| {
                    let c = v.get(nn - 1 - i) as u64;
                    if k == LogicKind::StdULogic {
                        Ok(c)
                    } else {
                        bit(c)
                    }
                })
                .collect::<R<Vec<u64>>>()?
        }
        None => {
            let w = elem_size(sc);
            let bytes = m.rd(data, n * w)?;
            bytes.chunks(w as usize).map(crate::exec::cell).collect()
        }
    };
    Ok(Arr { dims, el })
}

/// An array result: data, dope and fat pair in temporary memory.
fn write_arr(m: &mut Machine, t: TyId, a: &Arr) -> R<u64> {
    let (_, sc, _) = array_ty(m, t)?;
    let n = a.el.len();
    let data = match logic_of(sc) {
        Some(k) => {
            let mut codes = Vec::with_capacity(n);
            for &v in a.el.iter().rev() {
                let c = match k {
                    LogicKind::StdULogic if v <= 8 => v as u8,
                    LogicKind::StdULogic => return Err(ice(format!("std_ulogic code {v}"))),
                    _ => 2 | bit(v)? as u8,
                };
                codes.push(StdULogic::from_code_pub(c));
            }
            let v = LogicVec::from_fn(n, |i| codes[i]);
            let d = m.temp_alloc(n.div_ceil(64) as u64 * 32)?;
            m.put_planes(d, &v)?;
            d
        }
        None => {
            let w = elem_size(sc) as usize;
            let bytes: Vec<u8> =
                a.el.iter()
                    .flat_map(|v| v.to_le_bytes()[..w].to_vec())
                    .collect();
            let d = m.temp_alloc(bytes.len() as u64)?;
            m.put(d, &bytes)?;
            d
        }
    };
    let dope_bytes: Vec<u8> = a
        .dims
        .iter()
        .flat_map(|&(l, r, down)| [l, r, i64::from(down)])
        .flat_map(i64::to_le_bytes)
        .collect();
    let dope = m.temp_alloc(dope_bytes.len() as u64)?;
    m.put(dope, &dope_bytes)?;
    if Ty::has_handles(&m.rt.prog.types, t) {
        m.temp_root(data, Some(t), dope_bytes.into());
    }
    let fat = m.temp_alloc(16)?;
    m.store(fat, Width::B8, data)?;
    m.store(add_addr(fat, 8)?, Width::B8, dope)?;
    Ok(fat)
}

fn lens(a: &Arr) -> Vec<u64> {
    a.dims
        .iter()
        .map(|&(l, r, d)| dim_len(l, r, d).unwrap_or(0))
        .collect()
}

/// Lexicographic order of two one-dimensional arrays (LRM 9.2.3).
fn arr_order(sc: Sc, a: &Arr, b: &Arr) -> R<Ordering> {
    for (&x, &y) in a.el.iter().zip(&b.el) {
        let o = order(sc, x, y)?;
        if o.is_ne() {
            return Ok(o);
        }
    }
    Ok(a.el.len().cmp(&b.el.len()))
}

// ---- relational, minimum, maximum ----

fn rel(m: &mut Machine, t: Option<TyId>, a: &[u64], r: Rel) -> R<Option<u64>> {
    let (t, ty) = ty_of(m, t)?;
    let (x, y) = (arg(a, 0)?, arg(a, 1)?);
    let v = match ty {
        Ty::Array { .. } => {
            let (_, sc, _) = array_ty(m, t)?;
            let (p, q) = (read_arr(m, t, x)?, read_arr(m, t, y)?);
            if matches!(r, Rel::Eq | Rel::Ne) {
                let mut eq = lens(&p) == lens(&q);
                for (&u, &w) in p.el.iter().zip(&q.el) {
                    eq = eq && order(sc, u, w)?.is_eq();
                }
                eq == (r == Rel::Eq)
            } else {
                r.holds(arr_order(sc, &p, &q)?)
            }
        }
        Ty::Record { .. } => {
            return rt_err(
                "predefined \"=\" on record types is not a native (lowering inlines it)",
            );
        }
        _ => r.holds(order(scalar(m, t)?, x, y)?),
    };
    Ok(Some(u64::from(v)))
}

fn minmax(m: &mut Machine, t: Option<TyId>, a: &[u64], max: bool) -> R<Option<u64>> {
    let (t, _) = ty_of(m, t)?;
    let (x, y) = (arg(a, 0)?, arg(a, 1)?);
    let o = if is_array(m, t)? {
        let (_, sc, _) = array_ty(m, t)?;
        arr_order(sc, &read_arr(m, t, x)?, &read_arr(m, t, y)?)?
    } else {
        order(scalar(m, t)?, x, y)?
    };
    // MINIMUM(L, R) is L when L < R, else R; MAXIMUM is L when L > R.
    let left = if max { o.is_gt() } else { o.is_lt() };
    Ok(Some(if left { x } else { y }))
}

/// `MINIMUM(L)` / `MAXIMUM(L)` of an array; a null array gives the element type's
/// `'HIGH` / `'LOW` (sim1 agrees).
fn reduce_minmax(m: &mut Machine, t: Option<TyId>, a: &[u64], max: bool) -> R<Option<u64>> {
    let (t, _) = ty_of(m, t)?;
    let (_, sc, _) = array_ty(m, t)?;
    let arr = read_arr(m, t, arg(a, 0)?)?;
    let mut best: Option<u64> = None;
    for &v in &arr.el {
        best = Some(match best {
            Some(b) if (order(sc, v, b)?.is_gt() == max) && order(sc, v, b)?.is_ne() => v,
            Some(b) => b,
            None => v,
        });
    }
    let v = match best {
        Some(v) => v,
        None => {
            let (lo, hi) = match sc {
                Sc::Int(lo, hi) | Sc::Phys(lo, hi) => (lo as u64, hi as u64),
                Sc::Real => ((-f64::MAX).to_bits(), f64::MAX.to_bits()),
                Sc::Enum(n, _) => (0, n - 1),
            };
            if max { lo } else { hi }
        }
    };
    Ok(Some(v))
}

// ---- arithmetic ----

#[derive(Clone, Copy, Debug, PartialEq)]
enum Ar {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Rem,
    Exp,
    Neg,
    Pos,
    Abs,
}

fn ierr(e: arith::ArithError) -> Fault {
    match e {
        arith::ArithError::DivisionByZero { .. } => Fault::Runtime("division by zero".into()),
        e => Fault::Runtime(e.to_string()),
    }
}

fn real(x: f64, what: &str) -> R<u64> {
    if x.is_finite() {
        Ok(x.to_bits())
    } else {
        rt_err(format!("real {what} result {x} is out of range"))
    }
}

fn arith_op(m: &mut Machine, t: Option<TyId>, a: &[u64], op: Ar, form: &str) -> R<Option<u64>> {
    let (t, _) = ty_of(m, t)?;
    let sc = scalar(m, t)?;
    let x = arg(a, 0)?;
    let y = match op {
        Ar::Neg | Ar::Pos | Ar::Abs => 0,
        _ => arg(a, 1)?,
    };
    let f = f64::from_bits;
    let v = match sc {
        Sc::Real => {
            let r = match (op, form) {
                (Ar::Add, _) => f(x) + f(y),
                (Ar::Sub, _) => f(x) - f(y),
                (Ar::Mul, "ri") => f(x) * y as i64 as f64,
                (Ar::Mul, "ir") => x as i64 as f64 * f(y),
                (Ar::Mul, _) => f(x) * f(y),
                (Ar::Div, "ri") | (Ar::Div, _) if form == "ri" || f(y) != 0.0 => {
                    let d = if form == "ri" { y as i64 as f64 } else { f(y) };
                    if d == 0.0 {
                        return rt_err("division by zero");
                    }
                    f(x) / d
                }
                (Ar::Div, _) => return rt_err("division by zero"),
                (Ar::Exp, _) => match i32::try_from(y as i64) {
                    Ok(e) => f(x).powi(e),
                    Err(_) => f(x).powf(y as i64 as f64),
                },
                (Ar::Neg, _) => -f(x),
                (Ar::Pos, _) => f(x),
                (Ar::Abs, _) => f(x).abs(),
                (Ar::Mod | Ar::Rem, _) => return Err(ice("mod/rem on a real type")),
            };
            return real(r, "arithmetic").map(Some);
        }
        Sc::Int(lo, hi) | Sc::Phys(lo, hi) => {
            let (x, y) = (x as i64, y as i64);
            let r = match (op, form) {
                (Ar::Add, _) => arith::add(x, y),
                (Ar::Sub, _) => arith::sub(x, y),
                (Ar::Mul, "pr") => arith::phys_mul_real(x, f(y as u64)),
                (Ar::Mul, "rp") => arith::phys_mul_real(y, f(x as u64)),
                (Ar::Mul, _) => arith::mul(x, y),
                (Ar::Div, "pr") => arith::phys_div_real(x, f(y as u64)),
                (Ar::Div, "pp") => return arith::div(x, y).map(|v| Some(v as u64)).map_err(ierr),
                (Ar::Div, _) => arith::div(x, y),
                (Ar::Mod, _) => arith::modulo(x, y),
                (Ar::Rem, _) => arith::rem(x, y),
                (Ar::Exp, _) => arith::pow(x, y),
                (Ar::Neg, _) => arith::neg(x),
                (Ar::Pos, _) => Ok(x),
                (Ar::Abs, _) => arith::abs(x),
            };
            let r = r.map_err(ierr)?;
            arith::check_range(r, lo, hi).map_err(ierr)?
        }
        Sc::Enum(..) => return Err(ice("arithmetic on an enumeration type")),
    };
    Ok(Some(v as u64))
}

// ---- logical operators and shifts ----

#[derive(Clone, Copy, Debug, PartialEq)]
enum Lg {
    And,
    Or,
    Nand,
    Nor,
    Xor,
    Xnor,
    Not,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Shape {
    Plain,
    ArrayScalar,
    ScalarArray,
    Reduce,
}

fn logical_for(op: Lg, form: &str) -> Option<NativeFn> {
    Some(match (op, form) {
        (Lg::And, "") => |m, _, t, a| logical(m, t, a, Lg::And, Shape::Plain),
        (Lg::And, "as") => |m, _, t, a| logical(m, t, a, Lg::And, Shape::ArrayScalar),
        (Lg::And, "sa") => |m, _, t, a| logical(m, t, a, Lg::And, Shape::ScalarArray),
        (Lg::And, "reduce") => |m, _, t, a| logical(m, t, a, Lg::And, Shape::Reduce),
        (Lg::Or, "") => |m, _, t, a| logical(m, t, a, Lg::Or, Shape::Plain),
        (Lg::Or, "as") => |m, _, t, a| logical(m, t, a, Lg::Or, Shape::ArrayScalar),
        (Lg::Or, "sa") => |m, _, t, a| logical(m, t, a, Lg::Or, Shape::ScalarArray),
        (Lg::Or, "reduce") => |m, _, t, a| logical(m, t, a, Lg::Or, Shape::Reduce),
        (Lg::Nand, "") => |m, _, t, a| logical(m, t, a, Lg::Nand, Shape::Plain),
        (Lg::Nand, "as") => |m, _, t, a| logical(m, t, a, Lg::Nand, Shape::ArrayScalar),
        (Lg::Nand, "sa") => |m, _, t, a| logical(m, t, a, Lg::Nand, Shape::ScalarArray),
        (Lg::Nand, "reduce") => |m, _, t, a| logical(m, t, a, Lg::Nand, Shape::Reduce),
        (Lg::Nor, "") => |m, _, t, a| logical(m, t, a, Lg::Nor, Shape::Plain),
        (Lg::Nor, "as") => |m, _, t, a| logical(m, t, a, Lg::Nor, Shape::ArrayScalar),
        (Lg::Nor, "sa") => |m, _, t, a| logical(m, t, a, Lg::Nor, Shape::ScalarArray),
        (Lg::Nor, "reduce") => |m, _, t, a| logical(m, t, a, Lg::Nor, Shape::Reduce),
        (Lg::Xor, "") => |m, _, t, a| logical(m, t, a, Lg::Xor, Shape::Plain),
        (Lg::Xor, "as") => |m, _, t, a| logical(m, t, a, Lg::Xor, Shape::ArrayScalar),
        (Lg::Xor, "sa") => |m, _, t, a| logical(m, t, a, Lg::Xor, Shape::ScalarArray),
        (Lg::Xor, "reduce") => |m, _, t, a| logical(m, t, a, Lg::Xor, Shape::Reduce),
        (Lg::Xnor, "") => |m, _, t, a| logical(m, t, a, Lg::Xnor, Shape::Plain),
        (Lg::Xnor, "as") => |m, _, t, a| logical(m, t, a, Lg::Xnor, Shape::ArrayScalar),
        (Lg::Xnor, "sa") => |m, _, t, a| logical(m, t, a, Lg::Xnor, Shape::ScalarArray),
        (Lg::Xnor, "reduce") => |m, _, t, a| logical(m, t, a, Lg::Xnor, Shape::Reduce),
        _ => return None,
    })
}

fn lg(op: Lg, x: u64, y: u64) -> u64 {
    match op {
        Lg::And => x & y,
        Lg::Or => x | y,
        Lg::Nand => 1 ^ (x & y),
        Lg::Nor => 1 ^ (x | y),
        Lg::Xor => x ^ y,
        Lg::Xnor => 1 ^ x ^ y,
        Lg::Not => 1 ^ x,
    }
}

fn logical(m: &mut Machine, t: Option<TyId>, a: &[u64], op: Lg, shape: Shape) -> R<Option<u64>> {
    let (t, _) = ty_of(m, t)?;
    if !is_array(m, t)? {
        let x = bit(arg(a, 0)?)?;
        let y = if op == Lg::Not { 0 } else { bit(arg(a, 1)?)? };
        return Ok(Some(lg(op, x, y)));
    }
    let r = match shape {
        Shape::Reduce => {
            // The reduction is the operator applied left to right from the unit of its base
            // operator (LRM 9.2.2): `and ""` is '1', `nand ""` is '0', `xnor ""` is '1'.
            let p = read_arr(m, t, arg(a, 0)?)?;
            let (base, unit, inv) = match op {
                Lg::And | Lg::Nand => (Lg::And, 1, op == Lg::Nand),
                Lg::Or | Lg::Nor => (Lg::Or, 0, op == Lg::Nor),
                _ => (Lg::Xor, 0, op == Lg::Xnor),
            };
            let v = p.el.iter().fold(unit, |acc, &e| lg(base, acc, e));
            return Ok(Some(if inv { 1 ^ v } else { v }));
        }
        Shape::Plain if op == Lg::Not => {
            let mut p = read_arr(m, t, arg(a, 0)?)?;
            p.el.iter_mut().for_each(|e| *e ^= 1);
            p
        }
        Shape::Plain => {
            let mut p = read_arr(m, t, arg(a, 0)?)?;
            let q = read_arr(m, t, arg(a, 1)?)?;
            if p.el.len() != q.el.len() {
                return rt_err(format!(
                    "left and right operands of \"{}\" have different lengths ({} and {})",
                    lg_name(op),
                    p.el.len(),
                    q.el.len()
                ));
            }
            for (e, &f) in p.el.iter_mut().zip(&q.el) {
                *e = lg(op, *e, f);
            }
            p
        }
        Shape::ArrayScalar | Shape::ScalarArray => {
            let (ai, si) = if shape == Shape::ArrayScalar {
                (0, 1)
            } else {
                (1, 0)
            };
            let mut p = read_arr(m, t, arg(a, ai)?)?;
            let s = bit(arg(a, si)?)?;
            for e in &mut p.el {
                *e = if shape == Shape::ArrayScalar {
                    lg(op, *e, s)
                } else {
                    lg(op, s, *e)
                };
            }
            p
        }
    };
    write_arr(m, t, &r).map(Some)
}

fn lg_name(op: Lg) -> &'static str {
    match op {
        Lg::And => "and",
        Lg::Or => "or",
        Lg::Nand => "nand",
        Lg::Nor => "nor",
        Lg::Xor => "xor",
        Lg::Xnor => "xnor",
        Lg::Not => "not",
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Sh {
    Sll,
    Srl,
    Sla,
    Sra,
    Rol,
    Ror,
}

/// LRM 9.2.4, a negative count shifts the other way (`sla -1` is `sra 1`). sim1 fills
/// `sla`/`sra` with the wrong end for negative counts; sim2 follows the LRM.
fn shift(m: &mut Machine, t: Option<TyId>, a: &[u64], op: Sh) -> R<Option<u64>> {
    let (t, _) = ty_of(m, t)?;
    let mut p = read_arr(m, t, arg(a, 0)?)?;
    let k = arg(a, 1)? as i64;
    let op = if k >= 0 {
        op
    } else {
        match op {
            Sh::Sll => Sh::Srl,
            Sh::Srl => Sh::Sll,
            Sh::Sla => Sh::Sra,
            Sh::Sra => Sh::Sla,
            Sh::Rol => Sh::Ror,
            Sh::Ror => Sh::Rol,
        }
    };
    let n = p.el.len();
    if n > 0 {
        let k = usize::try_from(k.unsigned_abs()).unwrap_or(usize::MAX);
        let e = &p.el;
        let at = |i: usize, fill: u64| e.get(i).copied().unwrap_or(fill);
        let r: Vec<u64> = (0..n)
            .map(|i| match op {
                Sh::Sll => at(i.saturating_add(k), 0),
                Sh::Sla => at(i.saturating_add(k), e[n - 1]),
                Sh::Srl => i.checked_sub(k).map_or(0, |j| e[j]),
                Sh::Sra => i.checked_sub(k).map_or(e[0], |j| e[j]),
                Sh::Rol => e[(i + k % n) % n],
                Sh::Ror => e[(i + n - k % n) % n],
            })
            .collect();
        p.el = r;
    }
    write_arr(m, t, &p).map(Some)
}

// ---- concatenation ----

/// `&` (LRM 2008 9.2.5): two null arrays give the right operand; otherwise the result runs
/// from `S'LEFT` in the direction of `S`, and both bounds must belong to `S`. sim1 deviates for
/// two null operands and for downto index subtypes; sim2 follows the LRM.
fn cat(m: &mut Machine, t: Option<TyId>, a: &[u64], ls: bool, rs: bool) -> R<Option<u64>> {
    let (t, _) = ty_of(m, t)?;
    let (x, y) = (arg(a, 0)?, arg(a, 1)?);
    let (sl, sr, sdown) = (arg(a, 2)? as i64, arg(a, 3)? as i64, arg(a, 4)? == 1);
    let part = |m: &Machine, v: u64, scalar: bool| -> R<Vec<u64>> {
        if scalar {
            Ok(vec![v])
        } else {
            Ok(read_arr(m, t, v)?.el)
        }
    };
    let mut el = part(m, x, ls)?;
    el.extend(part(m, y, rs)?);
    if el.is_empty() {
        return Ok(Some(y));
    }
    let last = i128::from(el.len() as u64) - 1;
    let right = if sdown {
        i128::from(sl) - last
    } else {
        i128::from(sl) + last
    };
    let (lo, hi) = if sdown { (sr, sl) } else { (sl, sr) };
    let range = if sdown {
        format!("{sl} downto {sr}")
    } else {
        format!("{sl} to {sr}")
    };
    if sl < lo || sl > hi || right < i128::from(lo) || right > i128::from(hi) {
        return rt_err(format!(
            "the result of \"&\" has {} elements, its right bound {right} is outside the index \
             range {range}",
            el.len()
        ));
    }
    let r = Arr::one(sl, right as i64, sdown, el);
    write_arr(m, t, &r).map(Some)
}

// ---- matching operators ----

/// `?=` `?/=` `?<` `?<=` `?>` `?>=` on BIT and STD_ULOGIC, `?=`/`?/=` on their arrays.
fn matching(m: &mut Machine, t: Option<TyId>, a: &[u64], r: Rel) -> R<Option<u64>> {
    let (t, _) = ty_of(m, t)?;
    let (x, y) = (arg(a, 0)?, arg(a, 1)?);
    if is_array(m, t)? {
        let (_, sc, _) = array_ty(m, t)?;
        let (p, q) = (read_arr(m, t, x)?, read_arr(m, t, y)?);
        if p.el.len() != q.el.len() {
            return rt_err(format!(
                "operands of \"{}\" have different lengths ({} and {})",
                if r == Rel::Eq { "?=" } else { "?/=" },
                p.el.len(),
                q.el.len()
            ));
        }
        let v = if logic_of(sc) == Some(LogicKind::StdULogic) {
            let mut acc = StdULogic::One;
            for (&u, &w) in p.el.iter().zip(&q.el) {
                acc = acc.and(sul(u)?.match_eq(sul(w)?));
            }
            let acc = if r == Rel::Ne { acc.not() } else { acc };
            acc as u64
        } else {
            let eq = p.el == q.el;
            u64::from(eq == (r == Rel::Eq))
        };
        return Ok(Some(v));
    }
    match scalar(m, t)? {
        Sc::Enum(_, Some(LogicKind::StdULogic)) => match_sul(sul(x)?, sul(y)?, r).map(Some),
        _ => Ok(Some(u64::from(r.holds(bit(x)?.cmp(&bit(y)?))))),
    }
}

fn sul(v: u64) -> R<StdULogic> {
    if v <= 8 {
        Ok(StdULogic::from_code_pub(v as u8))
    } else {
        Err(ice(format!("std_ulogic code {v}")))
    }
}

/// The STD_ULOGIC matching operators (LRM 9.2.3): `U` dominates, then `X`; `-` matches
/// anything in `?=` and is an error in the ordering operators.
fn match_sul(x: StdULogic, y: StdULogic, r: Rel) -> R<u64> {
    use StdULogic::*;
    let v = match r {
        Rel::Eq => x.match_eq(y),
        Rel::Ne => x.match_eq(y).not(),
        _ => {
            if x == DontCare || y == DontCare {
                return rt_err(format!(
                    "'-' operand of the matching ordering operator \"?{}\"",
                    match r {
                        Rel::Lt => "<",
                        Rel::Le => "<=",
                        Rel::Gt => ">",
                        _ => ">=",
                    }
                ));
            }
            let (a, b) = (x.to_ux01(), y.to_ux01());
            if a == U || b == U {
                U
            } else if a == X || b == X {
                X
            } else {
                StdULogic::from_bool(r.holds(a.cmp(&b)))
            }
        }
    };
    Ok(v as u64)
}

// ---- strings ----

fn ret(m: &mut Machine, s: &[u8]) -> R<Option<u64>> {
    m.ret_string(s).map(Some)
}

fn to_string(
    m: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    t: Option<TyId>,
    a: &[u64],
) -> R<Option<u64>> {
    crate::native::text_result(m, t, arg(a, 0)?, false).map(Some)
}

fn conv(e: yavs_std::error::ConvError) -> Fault {
    Fault::Runtime(e.to_string())
}

fn to_string_digits(
    m: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    a: &[u64],
) -> R<Option<u64>> {
    let d = usize::try_from(arg(a, 1)? as i64).map_err(|_| ice("negative DIGITS"))?;
    let s = image::real_to_string_digits(f64::from_bits(arg(a, 0)?), d).map_err(conv)?;
    ret(m, &s)
}

fn to_string_format(
    m: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    _: Option<TyId>,
    a: &[u64],
) -> R<Option<u64>> {
    let f = m.fat_bytes(arg(a, 1)?)?;
    let s = image::real_to_string_format(f64::from_bits(arg(a, 0)?), &f).map_err(conv)?;
    ret(m, &s)
}

fn to_string_unit(
    m: &mut Machine,
    _: &mut Option<&mut Ctx<'_>>,
    t: Option<TyId>,
    a: &[u64],
) -> R<Option<u64>> {
    let Ty::Physical { units, .. } = ty_of(m, t)?.1 else {
        return Err(ice("TO_STRING(VALUE, UNIT) of a non-physical type"));
    };
    let u: Vec<(&str, i64)> = units.iter().map(|(n, v)| (n.as_str(), *v)).collect();
    let pt = PhysicalType::new(&u);
    let s =
        image::physical_to_string_unit(arg(a, 0)? as i64, &pt, arg(a, 1)? as i64).map_err(conv)?;
    ret(m, &s)
}

fn radix(
    m: &mut Machine,
    t: Option<TyId>,
    a: &[u64],
    width: usize,
    f: fn(&[bool]) -> Vec<u8>,
) -> R<Option<u64>> {
    let (t, _) = ty_of(m, t)?;
    let (_, sc, nd) = array_ty(m, t)?;
    // A `bit_vector` of up to 64 elements is one word.
    if nd == 1
        && logic_of(sc) == Some(LogicKind::Bit)
        && let Some(v) = crate::ieee::small_fat(m, a, 0)
    {
        return ret(m, &image::word_to_radix(v.bits, v.n, width));
    }
    let bits: Vec<bool> = match logic_of(sc) {
        // The usual case: the bits straight from the planes.
        Some(k) if nd == 1 => {
            let (data, n) = m.fat1(arg(a, 0)?)?;
            let v = m.planes(data, n)?;
            let nn = n as usize;
            (0..nn)
                .map(|i| {
                    let c = v.get(nn - 1 - i) as u64;
                    Ok(if k == LogicKind::StdULogic {
                        c
                    } else {
                        bit(c)?
                    } == 1)
                })
                .collect::<R<_>>()?
        }
        _ => read_arr(m, t, arg(a, 0)?)?
            .el
            .iter()
            .map(|&e| e == 1)
            .collect(),
    };
    ret(m, &f(&bits))
}

// ---- signals ----

/// `RISING_EDGE(S)` (`want` 1) / `FALLING_EDGE(S)` (0) of a BIT or BOOLEAN signal.
fn edge(m: &mut Machine, ctx: &mut Option<&mut Ctx<'_>>, a: &[u64], want: u64) -> R<Option<u64>> {
    let (c0, c1) = (arg(a, 0)?, arg(a, 1)?);
    let s = m
        .inst
        .sigs
        .get((c0 >> 32) as usize)
        .copied()
        .ok_or_else(|| ice(format!("no instance signal {}", c0 >> 32)))?;
    let sr = s.signal.slice(s.off + c0 as u32, c1 as u32);
    let Some(c) = ctx.as_deref_mut() else {
        return rt_err("signal read during elaboration");
    };
    if !c.event(sr)? {
        return Ok(Some(0));
    }
    let v = match s.ty.elem {
        Elem::Logic => {
            let l = c.logic(sr)?;
            if l.is_empty() {
                return Err(ice("edge of a null signal range"));
            }
            l.get(0) as u64
        }
        Elem::Bytes(_) => u64::from(*c.value(sr)?.first().ok_or_else(|| ice("empty signal"))?),
    };
    Ok(Some(u64::from(bit(v)? == want)))
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use yavs_ir::{FuncId, LogicKind, ProgramBuilder, Ty, TyId};

    use super::*;
    use crate::{Bindings, InstData, Runtime};

    struct T {
        m: Machine,
    }

    // In the order `setup` declares them.
    const INT: TyId = TyId(0);
    const SUL: TyId = TyId(2);
    const REAL: TyId = TyId(3);
    const BV: TyId = TyId(4);
    const SV: TyId = TyId(5);
    const IV: TyId = TyId(6);

    fn setup() -> T {
        let mut pb = ProgramBuilder::new();
        let int = pb.ty(Ty::Int {
            lo: -2147483648,
            hi: 2147483647,
        });
        let bit = pb.ty(Ty::Enum {
            lits: vec!["'0'".into(), "'1'".into()],
            logic: Some(LogicKind::Bit),
        });
        let sul = pb.ty(Ty::Enum {
            lits: "UX01ZWLH-".chars().map(|c| format!("'{c}'")).collect(),
            logic: Some(LogicKind::StdULogic),
        });
        let real = pb.ty(Ty::Real);
        let arr = |pb: &mut ProgramBuilder, elem| {
            pb.ty(Ty::Array {
                elem,
                elem_desc: None,
                dims: 1,
            })
        };
        let (bv, sv, iv) = (arr(&mut pb, bit), arr(&mut pb, sul), arr(&mut pb, int));
        let rt = Runtime::new(Arc::new(pb.finish().unwrap()), Bindings::default()).unwrap();
        let m = Machine::new(rt, Arc::new(InstData::default()), Vec::new(), FuncId(0));
        assert_eq!((int, sul, real, bv, sv, iv), (INT, SUL, REAL, BV, SV, IV));
        T { m }
    }

    fn call(t: &mut T, sym: &str, ty: TyId, a: &[u64]) -> Result<u64, String> {
        let f = lookup(sym).unwrap_or_else(|| panic!("no native {sym}"));
        match f(&mut t.m, &mut None, Some(ty), a) {
            Ok(v) => Ok(v.unwrap_or(0)),
            Err(Fault::Runtime(e)) => Err(e),
            Err(e) => panic!("{sym}: {e:?}"),
        }
    }

    /// A bit or std_ulogic array from its characters, `left to/downto right`.
    fn arr(t: &mut T, ty: TyId, s: &str, left: i64, down: bool) -> u64 {
        let el: Vec<u64> = s
            .chars()
            .map(|c| "UX01ZWLH-".find(c).unwrap() as u64)
            .map(|v| if ty == BV { v - 2 } else { v })
            .collect();
        let n = el.len() as i64 - 1;
        let right = if down { left - n } else { left + n };
        write_arr(&mut t.m, ty, &Arr::one(left, right, down, el)).unwrap()
    }

    fn text(t: &mut T, ty: TyId, fat: u64) -> (String, (i64, i64, bool)) {
        let a = read_arr(&t.m, ty, fat).unwrap();
        let s =
            a.el.iter()
                .map(|&v| {
                    let v = if ty == BV { v + 2 } else { v };
                    "UX01ZWLH-".as_bytes()[v as usize] as char
                })
                .collect();
        (s, a.dims[0])
    }

    #[test]
    fn shifts_follow_the_lrm() {
        // (op, count, result) for "10110010"(7 downto 0); sim2's output, sim1 differs for
        // negative arithmetic shifts.
        let cases = [
            ("sll", 1, "01100100"),
            ("srl", 1, "01011001"),
            ("sla", 1, "01100100"),
            ("sra", 1, "11011001"),
            ("rol", 1, "01100101"),
            ("ror", 1, "01011001"),
            ("sla", -1, "11011001"),
            ("sra", -2, "11001000"),
            ("sll", -7, "00000001"),
            ("sra", 9, "11111111"),
            ("rol", 9, "01100101"),
            ("ror", -8, "10110010"),
            ("sll", i64::MIN, "00000000"),
        ];
        let mut t = setup();
        for (op, k, want) in cases {
            let v = arr(&mut t, BV, "10110010", 7, true);
            let r = call(
                &mut t,
                &format!("yavs.implicit.{op}.std.standard.bit_vector"),
                BV,
                &[v, k as u64],
            )
            .unwrap();
            assert_eq!(text(&mut t, BV, r), (want.into(), (7, 0, true)), "{op} {k}");
        }
    }

    #[test]
    fn concatenation_bounds() {
        let mut t = setup();
        let sym = "yavs.implicit.cat.std.standard.bit_vector";
        let nat = [0, 2147483647, 0];
        // (left, right, S) -> result; LRM 9.2.5 (sim2's output, sim1 differs for null & null
        // and downto index subtypes).
        let a = arr(&mut t, BV, "1010", 7, true);
        let b = arr(&mut t, BV, "11", 1, false);
        let r = call(&mut t, sym, BV, &[a, b, nat[0], nat[1], nat[2]]).unwrap();
        assert_eq!(text(&mut t, BV, r), ("101011".into(), (0, 5, false)));
        let r = call(&mut t, sym, BV, &[a, b, 10, 0, 1]).unwrap();
        assert_eq!(text(&mut t, BV, r), ("101011".into(), (10, 5, true)));
        let n1 = write_arr(&mut t.m, BV, &Arr::one(5, 6, true, vec![])).unwrap();
        let n2 = write_arr(&mut t.m, BV, &Arr::one(3, 2, false, vec![])).unwrap();
        let r = call(&mut t, sym, BV, &[n1, n2, 0, 2147483647, 0]).unwrap();
        assert_eq!(r, n2, "two null arrays give the right operand");
        let r = call(&mut t, sym, BV, &[n1, a, 0, 2147483647, 0]).unwrap();
        assert_eq!(text(&mut t, BV, r), ("1010".into(), (0, 3, false)));
        let r = call(
            &mut t,
            "yavs.implicit.cat.std.standard.bit_vector#ss",
            BV,
            &[1, 0, 10, 0, 1],
        )
        .unwrap();
        assert_eq!(text(&mut t, BV, r), ("10".into(), (10, 9, true)));
        let r = call(
            &mut t,
            "yavs.implicit.cat.std.standard.bit_vector#as",
            BV,
            &[n1, 1, 0, 2147483647, 0],
        )
        .unwrap();
        assert_eq!(text(&mut t, BV, r), ("1".into(), (0, 0, false)));
        // 12 elements do not fit in 10 downto 0.
        let big = arr(&mut t, BV, "111111111111", 11, true);
        let e = call(&mut t, sym, BV, &[big, n1, 10, 0, 1]).unwrap_err();
        assert!(e.contains("outside the index range 10 downto 0"), "{e}");
    }

    #[test]
    fn reductions_and_logical() {
        let mut t = setup();
        let e = write_arr(&mut t.m, BV, &Arr::one(1, 0, false, vec![])).unwrap();
        let v = arr(&mut t, BV, "10110010", 7, true);
        // Empty: and 1, or 0, xor 0, nand 0, nor 1, xnor 1 (sim1 agrees).
        for (op, empty, full) in [
            ("and", 1, 0),
            ("or", 0, 1),
            ("xor", 0, 0),
            ("nand", 0, 1),
            ("nor", 1, 0),
            ("xnor", 1, 1),
        ] {
            let s = format!("yavs.implicit.{op}.std.standard.bit_vector#reduce");
            assert_eq!(call(&mut t, &s, BV, &[e]), Ok(empty), "{op}");
            assert_eq!(call(&mut t, &s, BV, &[v]), Ok(full), "{op}");
        }
        let r = call(
            &mut t,
            "yavs.implicit.xor.std.standard.bit_vector#sa",
            BV,
            &[1, v],
        );
        assert_eq!(text(&mut t, BV, r.unwrap()).0, "01001101");
        let r = call(
            &mut t,
            "yavs.implicit.nand.std.standard.bit_vector#as",
            BV,
            &[v, 1],
        );
        assert_eq!(text(&mut t, BV, r.unwrap()).0, "01001101");
        let w = arr(&mut t, BV, "0110", 3, true);
        let x = arr(&mut t, BV, "0101", 0, false);
        let r = call(
            &mut t,
            "yavs.implicit.or.std.standard.bit_vector",
            BV,
            &[w, x],
        );
        assert_eq!(text(&mut t, BV, r.unwrap()), ("0111".into(), (3, 0, true)));
        let e = call(
            &mut t,
            "yavs.implicit.or.std.standard.bit_vector",
            BV,
            &[w, v],
        );
        assert!(e.unwrap_err().contains("different lengths (4 and 8)"));
        assert_eq!(
            call(
                &mut t,
                "yavs.implicit.nor.std.standard.bit",
                TyId(1),
                &[0, 0]
            ),
            Ok(1)
        );
    }

    #[test]
    fn matching_tables() {
        // Rows of `?<` from sim1, columns U X 0 1 Z W L H.
        let lt = [
            "UUUUUUUU", "UXXXXXXX", "UX01XX01", "UX00XX00", "UXXXXXXX", "UXXXXXXX", "UX01XX01",
            "UX00XX00",
        ];
        let mut t = setup();
        let sym = |op: &str| format!("yavs.implicit.{op}.ieee.std_logic_1164.std_ulogic");
        let c = |v: u64| "UX01ZWLH-".as_bytes()[v as usize] as char;
        for (l, row) in lt.iter().enumerate() {
            let got: String = (0..8)
                .map(|r| c(call(&mut t, &sym("mlt"), SUL, &[l as u64, r]).unwrap()))
                .collect();
            assert_eq!(&got, row, "row {l}");
        }
        assert!(call(&mut t, &sym("mle"), SUL, &[8, 3]).is_err());
        assert_eq!(c(call(&mut t, &sym("meq"), SUL, &[8, 1]).unwrap()), '1');
        let v = |t: &mut T, s| arr(t, SV, s, 3, true);
        let vsym = "yavs.implicit.meq.ieee.std_logic_1164.std_ulogic_vector";
        for (a, b, want) in [
            ("1-0X", "110X", 'X'),
            ("1U00", "1000", 'U'),
            ("0U00", "1U00", '0'),
            ("X100", "X000", '0'),
            ("1-01", "1101", '1'),
        ] {
            let (x, y) = (v(&mut t, a), v(&mut t, b));
            assert_eq!(
                c(call(&mut t, vsym, SV, &[x, y]).unwrap()),
                want,
                "{a} ?= {b}"
            );
        }
        let (x, y) = (v(&mut t, "10"), v(&mut t, "100"));
        assert!(call(&mut t, vsym, SV, &[x, y]).is_err());
    }

    #[test]
    fn arithmetic_and_order() {
        let mut t = setup();
        let s = |op: &str| format!("yavs.implicit.{op}.std.standard.integer");
        let i = |v: i64| v as u64;
        assert_eq!(call(&mut t, &s("mod"), INT, &[i(-7), 3]), Ok(i(2)));
        assert_eq!(call(&mut t, &s("rem"), INT, &[i(-7), 3]), Ok(i(-1)));
        assert_eq!(call(&mut t, &s("exp"), INT, &[2, 10]), Ok(1024));
        assert!(call(&mut t, &s("add"), INT, &[2147483647, 1]).is_err());
        assert!(call(&mut t, &s("exp"), INT, &[2, 31]).is_err());
        assert_eq!(
            call(&mut t, &s("div"), INT, &[1, 0]),
            Err("division by zero".into())
        );
        assert!(call(&mut t, &s("abs"), INT, &[i(i64::MIN)]).is_err());
        let r = |x: f64| x.to_bits();
        let rs = "yavs.implicit.exp.std.standard.real";
        assert_eq!(call(&mut t, rs, REAL, &[r(2.0), i(-1)]), Ok(r(0.5)));
        assert!(call(&mut t, rs, REAL, &[r(0.0), i(-1)]).is_err());
        let iv = |t: &mut T, v: &[i64]| {
            let el = v.iter().map(|&x| x as u64).collect();
            write_arr(&mut t.m, IV, &Arr::one(0, v.len() as i64 - 1, false, el)).unwrap()
        };
        let red = "yavs.implicit.minimum.std.standard.integer_vector#reduce";
        let (a, e) = (iv(&mut t, &[3, -2, 7]), iv(&mut t, &[]));
        assert_eq!(call(&mut t, red, IV, &[a]), Ok(i(-2)));
        assert_eq!(call(&mut t, red, IV, &[e]), Ok(2147483647));
        let red = "yavs.implicit.maximum.std.standard.integer_vector#reduce";
        assert_eq!(call(&mut t, red, IV, &[e]), Ok(i(-2147483648)));
        let lt = "yavs.implicit.lt.std.standard.integer_vector";
        let (p, q) = (iv(&mut t, &[1, 2]), iv(&mut t, &[1, 2, 0]));
        assert_eq!(call(&mut t, lt, IV, &[p, q]), Ok(1));
        assert_eq!(call(&mut t, lt, IV, &[q, a]), Ok(1));
        let (b1, b2) = (
            arr(&mut t, BV, "101", 2, true),
            arr(&mut t, BV, "1001", 3, true),
        );
        let min = "yavs.implicit.minimum.std.standard.bit_vector";
        assert_eq!(call(&mut t, min, BV, &[b1, b2]), Ok(b2));
        let eq = "yavs.implicit.eq.std.standard.bit_vector";
        let b3 = arr(&mut t, BV, "101", 0, false);
        assert_eq!(call(&mut t, eq, BV, &[b1, b3]), Ok(1));
    }

    #[test]
    fn strings() {
        let mut t = setup();
        let to_s = |t: &mut T, sym: &str, ty, a: &[u64]| {
            let f = call(t, sym, ty, a).unwrap();
            String::from_utf8(t.m.fat_bytes(f).unwrap()).unwrap()
        };
        let v = arr(&mut t, BV, "10110", 4, true);
        let h = "yavs.implicit.to_hstring.std.standard.bit_vector";
        assert_eq!(to_s(&mut t, h, BV, &[v]), "16");
        let o = "yavs.implicit.to_ostring.std.standard.bit_vector";
        assert_eq!(to_s(&mut t, o, BV, &[v]), "26");
        let s = "yavs.implicit.to_string.std.standard.bit_vector";
        assert_eq!(to_s(&mut t, s, BV, &[v]), "10110");
        let x = arr(&mut t, SV, "UX01-", 4, true);
        let s = "yavs.implicit.to_string.ieee.std_logic_1164.std_ulogic_vector";
        assert_eq!(to_s(&mut t, s, SV, &[x]), "UX01-");
        let iv = write_arr(
            &mut t.m,
            IV,
            &Arr::one(0, 2, false, vec![1, (-2i64) as u64, 3]),
        )
        .unwrap();
        let s = "yavs.implicit.to_string.std.standard.integer_vector";
        assert_eq!(to_s(&mut t, s, IV, &[iv]), "(1,-2,3)");
        let d = "yavs.implicit.to_string.std.standard.real#digits";
        assert_eq!(to_s(&mut t, d, REAL, &[1.5f64.to_bits(), 3]), "1.500");
    }

    #[test]
    fn unknown_symbols_are_left_to_native_rs() {
        assert!(lookup("yavs.implicit.file_open.std.textio.text").is_none());
        assert!(lookup("yavs.env.stop.void").is_none());
        assert!(lookup("yavs.implicit.eq.work.p.t").is_some());
    }

    #[test]
    fn a_type_named_like_a_form_is_not_a_form() {
        // `mul` of a user type `pi` is the plain one; `mul` of `bit#pi` is the physical form.
        assert!(lookup("yavs.implicit.mul.work.p.pi").is_some());
        assert!(lookup("yavs.implicit.cat.work.p.ss").is_some());
        assert!(lookup("yavs.implicit.cat.work.p.ss#ss").is_some());
    }
}
