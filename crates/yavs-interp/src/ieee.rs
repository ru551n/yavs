//! Native `ieee` subprograms (`yavs_ir::ieee`): the hot operators of `std_logic_1164` and
//! `numeric_std`, run on bit-plane vectors instead of their VHDL bodies.
//!
//! Each function is a port of the body it replaces: same result bounds, same handling of
//! metavalues, same warning texts and severities. `lib/ieee/*/` is the reference; the
//! differential test (`tests/ieee_native.rs`) runs both on the same operands. Calling
//! convention: `native.rs` (arrays as the address of a fat pair, scalars as cells).

use yavs_ir::ieee::{Cmp, Form, Gate, Math, Op, Shift, Syn};
use yavs_ir::{DOPE_DIM_BYTES, TyId, Width, dim_len};
use yavs_kernel::{Ctx, LogicVec, Severity};
use yavs_logic::{BitVec, StdULogic};

use crate::exec::{Fault, Machine, R, add_addr, ice, rt_err};
use crate::native::NativeFn;

type Ret = R<Option<u64>>;
type Cx<'a, 'b, 'c> = &'a mut Option<&'b mut Ctx<'c>>;

/// An array operand or result: elements and the bounds of its one dimension.
struct Vh {
    v: LogicVec,
    left: i64,
    right: i64,
    down: bool,
}

impl Vh {
    /// `(n-1 downto 0)`.
    fn down(v: LogicVec) -> Self {
        let n = v.len() as i64;
        Vh {
            v,
            left: n - 1,
            right: 0,
            down: true,
        }
    }
    /// `(1 to n)`, the range of the std_logic_1164 operators.
    fn up1(v: LogicVec) -> Self {
        let n = v.len() as i64;
        Vh {
            v,
            left: 1,
            right: n,
            down: false,
        }
    }
    /// The null array constants NAU and NAS: `(0 downto 1)`.
    fn null() -> Self {
        Vh {
            v: LogicVec::new(0, StdULogic::U),
            left: 0,
            right: 1,
            down: true,
        }
    }
}

fn arg(a: &[u64], i: usize) -> R<u64> {
    a.get(i)
        .copied()
        .ok_or_else(|| ice(format!("native argument {i} missing")))
}

fn sul(a: &[u64], i: usize) -> R<StdULogic> {
    match arg(a, i)? {
        c @ 0..=8 => Ok(StdULogic::from_code_pub(c as u8)),
        c => Err(ice(format!("std_ulogic code {c}"))),
    }
}

fn int(a: &[u64], i: usize) -> R<i64> {
    Ok(arg(a, i)? as i64)
}

fn vh(m: &Machine, a: &[u64], i: usize) -> R<Vh> {
    let fat = arg(a, i)?;
    let data = m.load(fat, Width::B8)?;
    let dope = m.load(add_addr(fat, 8)?, Width::B8)?;
    let left = m.load(dope, Width::B8)? as i64;
    let right = m.load(add_addr(dope, 8)?, Width::B8)? as i64;
    let down = m.load(add_addr(dope, 16)?, Width::B8)? == 1;
    let n = dim_len(left, right, down).ok_or_else(|| Fault::Runtime("array too long".into()))?;
    Ok(Vh {
        v: m.planes(data, n)?,
        left,
        right,
        down,
    })
}

fn put(m: &mut Machine, o: &Vh) -> Ret {
    let data = m.temp_alloc(o.v.len().div_ceil(64) as u64 * 32)?;
    m.put_planes(data, &o.v)?;
    let bytes: Vec<u8> = [o.left, o.right, i64::from(o.down)]
        .iter()
        .flat_map(|v| v.to_le_bytes())
        .collect();
    debug_assert_eq!(bytes.len() as u64, DOPE_DIM_BYTES);
    let dope = m.temp_alloc(DOPE_DIM_BYTES)?;
    m.put(dope, &bytes)?;
    let fat = m.temp_alloc(16)?;
    m.store(fat, Width::B8, data)?;
    m.store(add_addr(fat, 8)?, Width::B8, dope)?;
    Ok(Some(fat))
}

/// A report from a native, as `Inst::Report` does it. The process ends after a report at the
/// stop severity (`Machine::ieee_stop`).
fn report(m: &mut Machine, ctx: Cx<'_, '_, '_>, s: Severity, text: &str) -> R<()> {
    match ctx.as_deref_mut() {
        Some(c) => {
            if !m.rt.asserts.report(c, s) {
                return Ok(());
            }
            c.report(s, text);
            if s >= c.stop_severity() {
                m.ieee_stop = true;
            }
        }
        None => {
            use std::io::Write;
            if !m.rt.asserts.report_elab(s) {
                return Ok(());
            }
            let name = ["Note", "Warning", "Error", "Failure"][s as usize];
            let mut o = std::io::stderr().lock();
            let _ = writeln!(o, "** {name}: (init): {text}");
            if s >= Severity::Failure {
                return rt_err(format!("report of severity failure: {text}"));
            }
        }
    }
    Ok(())
}

fn warn(m: &mut Machine, ctx: Cx<'_, '_, '_>, text: &str) -> R<()> {
    report(m, ctx, Severity::Warning, text)
}

fn x_vec(n: usize) -> LogicVec {
    LogicVec::new(n, StdULogic::X)
}

// ---- std_logic_1164 ----

fn gate_of(g: Gate, x: StdULogic, y: StdULogic) -> StdULogic {
    match g {
        Gate::And => x.and(y),
        Gate::Nand => x.nand(y),
        Gate::Or => x.or(y),
        Gate::Nor => x.nor(y),
        Gate::Xor => x.xor(y),
        Gate::Xnor => x.xnor(y),
    }
}

fn gate_name(g: Gate) -> &'static str {
    match g {
        Gate::And => "and",
        Gate::Nand => "nand",
        Gate::Or => "or",
        Gate::Nor => "nor",
        Gate::Xor => "xor",
        Gate::Xnor => "xnor",
    }
}

fn gate<const G: u8>(_: &mut Machine, _: Cx<'_, '_, '_>, _: Option<TyId>, a: &[u64]) -> Ret {
    Ok(Some(
        gate_of(GATES[G as usize], sul(a, 0)?, sul(a, 1)?) as u64
    ))
}

const GATES: [Gate; 6] = [
    Gate::And,
    Gate::Nand,
    Gate::Or,
    Gate::Nor,
    Gate::Xor,
    Gate::Xnor,
];

fn vec_gate<const G: u8>(m: &mut Machine, ctx: Cx<'_, '_, '_>, _: Option<TyId>, a: &[u64]) -> Ret {
    let g = GATES[G as usize];
    let (l, r) = (vh(m, a, 0)?, vh(m, a, 1)?);
    if l.v.len() != r.v.len() {
        let n = gate_name(g);
        report(
            m,
            ctx,
            Severity::Failure,
            &format!(
                "STD_LOGIC_1164.\"{n}\": arguments of overloaded '{n}' operator are not of the same length"
            ),
        )?;
        return put(m, &Vh::up1(LogicVec::new(l.v.len(), StdULogic::U)));
    }
    let v = match g {
        Gate::And => l.v.and(&r.v),
        Gate::Nand => l.v.nand(&r.v),
        Gate::Or => l.v.or(&r.v),
        Gate::Nor => l.v.nor(&r.v),
        Gate::Xor => l.v.xor(&r.v),
        Gate::Xnor => l.v.xnor(&r.v),
    };
    put(m, &Vh::up1(v))
}

fn not(_: &mut Machine, _: Cx<'_, '_, '_>, _: Option<TyId>, a: &[u64]) -> Ret {
    Ok(Some(sul(a, 0)?.not() as u64))
}

fn vec_not(m: &mut Machine, _: Cx<'_, '_, '_>, _: Option<TyId>, a: &[u64]) -> Ret {
    let l = vh(m, a, 0)?;
    put(m, &Vh::up1(l.v.not()))
}

fn conv<const K: u8, const VEC: bool>(
    m: &mut Machine,
    _: Cx<'_, '_, '_>,
    _: Option<TyId>,
    a: &[u64],
) -> Ret {
    if VEC {
        let l = vh(m, a, 0)?;
        let v = match K {
            0 => l.v.to_x01(),
            1 => l.v.to_x01z(),
            _ => l.v.to_ux01(),
        };
        return put(m, &Vh::up1(v));
    }
    let s = sul(a, 0)?;
    Ok(Some(match K {
        0 => s.to_x01(),
        1 => s.to_x01z(),
        _ => s.to_ux01(),
    } as u64))
}

/// `TO_01` of a vector: every element `0 1 L H` gives the 0/1 vector, any other the whole
/// result `xmap`.
fn to_01(v: &LogicVec, xmap: StdULogic) -> LogicVec {
    match v.to_bitvec() {
        Some(b) => LogicVec::from_bitvec(&b),
        None => LogicVec::new(v.len(), xmap),
    }
}

fn to_01_vec(m: &mut Machine, _: Cx<'_, '_, '_>, _: Option<TyId>, a: &[u64]) -> Ret {
    let l = vh(m, a, 0)?;
    let x = sul(a, 1)?;
    put(m, &Vh::down(to_01(&l.v, x)))
}

fn truthy(_: &mut Machine, _: Cx<'_, '_, '_>, _: Option<TyId>, a: &[u64]) -> Ret {
    Ok(Some(u64::from(sul(a, 0)?.is_true())))
}

/// `rising_edge (s)` / `falling_edge (s)` of a `std_ulogic` signal: `s'event` and
/// `To_X01 (s)` is the new level and `To_X01 (s'last_value)` the old one. The signal is two
/// cells (instance signal and slice), as for the implicit edge functions.
fn edge<const RISING: bool>(
    m: &mut Machine,
    ctx: Cx<'_, '_, '_>,
    _: Option<TyId>,
    a: &[u64],
) -> Ret {
    let (c0, c1) = (arg(a, 0)?, arg(a, 1)?);
    let s = m
        .inst
        .sigs
        .get((c0 >> 32) as usize)
        .copied()
        .ok_or_else(|| ice(format!("no instance signal {}", c0 >> 32)))?;
    let sr = s.signal.slice(c0 as u32, c1 as u32);
    let Some(c) = ctx.as_deref_mut() else {
        return rt_err("signal read during elaboration");
    };
    if !c.event(sr)? {
        return Ok(Some(0));
    }
    let (new, old) = (c.logic(sr)?, c.last_value(sr)?);
    let yavs_kernel::Value::Logic(old) = old else {
        return Err(ice("std_ulogic signal without logic storage"));
    };
    if new.is_empty() || old.is_empty() {
        return Err(ice("edge of a null signal range"));
    }
    let (want_new, want_old) = if RISING {
        (StdULogic::One, StdULogic::Zero)
    } else {
        (StdULogic::Zero, StdULogic::One)
    };
    Ok(Some(u64::from(
        new.get(0).to_x01() == want_new && old.get(0).to_x01() == want_old,
    )))
}

// ---- math_real ----

fn real(a: &[u64], i: usize) -> R<f64> {
    Ok(f64::from_bits(arg(a, i)?))
}

/// `FLOOR`/`CEIL` return the argument unchanged from INTEGER'HIGH up (the body converts to
/// INTEGER below that).
const LARGE: f64 = 2147483647.0;

/// `FLOOR`/`CEIL` as the bodies compute them: from `RD = REAL (INTEGER (X))` (round half away
/// from zero), so a result of zero is always `+0.0`.
fn floor_r(x: f64) -> f64 {
    if x.abs() >= LARGE {
        return x;
    }
    let rd = x.round();
    if rd == x {
        x
    } else if x > 0.0 {
        if rd <= x { rd } else { rd - 1.0 }
    } else if rd >= x {
        rd - 1.0
    } else {
        rd
    }
}

fn ceil_r(x: f64) -> f64 {
    if x.abs() >= LARGE {
        return x;
    }
    let rd = x.round();
    if rd == x {
        x
    } else if x > 0.0 {
        if rd >= x { rd } else { rd + 1.0 }
    } else if rd <= x {
        rd + 1.0
    } else {
        rd
    }
}

const MATH: [Math; 15] = [
    Math::Sqrt,
    Math::Exp,
    Math::Log,
    Math::Log2,
    Math::Log10,
    Math::LogBase,
    Math::Sin,
    Math::Cos,
    Math::PowRR,
    Math::PowIR,
    Math::Ceil,
    Math::Floor,
    Math::Round,
    Math::Trunc,
    Math::Sign,
];

/// An out-of-domain argument: the body's error report and its error value.
fn domain(m: &mut Machine, ctx: Cx<'_, '_, '_>, text: &str, value: f64) -> Ret {
    report(m, ctx, Severity::Error, text)?;
    Ok(Some(value.to_bits()))
}

/// The `math_real` functions on `f64` library functions. Their bodies are series and Newton
/// iterations: the results can differ from the body's in the last digits, the error cases
/// (report text, severity and returned value) do not.
fn math<const K: u8>(m: &mut Machine, ctx: Cx<'_, '_, '_>, _: Option<TyId>, a: &[u64]) -> Ret {
    let ret = |v: f64| Ok(Some(v.to_bits()));
    let low = f64::MIN; // REAL'LOW
    match MATH[K as usize] {
        Math::Sqrt => {
            let x = real(a, 0)?;
            if x < 0.0 {
                return domain(m, ctx, "X < 0.0 in SQRT(X)", 0.0);
            }
            ret(x.sqrt())
        }
        Math::Exp => {
            let x = real(a, 0)?;
            if x.abs() > f64::MAX.ln() {
                if x < 0.0 {
                    return ret(0.0);
                }
                report(m, ctx, Severity::Note, "X > LOG(REAL'HIGH) in EXP(X)")?;
                return ret(f64::MAX);
            }
            ret(x.exp())
        }
        Math::Log => {
            let x = real(a, 0)?;
            if x <= 0.0 {
                return domain(m, ctx, "X <= 0.0 in LOG(X)", low);
            }
            ret(x.ln())
        }
        Math::Log2 => {
            let x = real(a, 0)?;
            if x <= 0.0 {
                return domain(m, ctx, "X <= 0.0 in LOG2(X)", low);
            }
            ret(x.log2())
        }
        Math::Log10 => {
            let x = real(a, 0)?;
            if x <= 0.0 {
                return domain(m, ctx, "X <= 0.0 in LOG10(X)", low);
            }
            ret(x.log10())
        }
        Math::LogBase => {
            let (x, b) = (real(a, 0)?, real(a, 1)?);
            if x <= 0.0 {
                return domain(m, ctx, "X <= 0.0 in LOG(X, BASE)", low);
            }
            if b <= 0.0 || b == 1.0 {
                return domain(m, ctx, "BASE <= 0.0 or BASE = 1.0 in LOG(X, BASE)", low);
            }
            ret(if x == 1.0 {
                0.0
            } else if x == b {
                1.0
            } else {
                x.ln() / b.ln()
            })
        }
        Math::Sin => ret(real(a, 0)?.sin()),
        Math::Cos => ret(real(a, 0)?.cos()),
        k @ (Math::PowRR | Math::PowIR) => {
            let (x, y) = if k == Math::PowRR {
                (real(a, 0)?, real(a, 1)?)
            } else {
                (int(a, 0)? as f64, real(a, 1)?)
            };
            if x < 0.0 && y != 0.0 {
                let t = if k == Math::PowRR {
                    "X < 0.0 and Y /= 0.0 in X**Y"
                } else {
                    "X < 0 and Y /= 0.0 in X**Y"
                };
                return domain(m, ctx, t, 0.0);
            }
            if x == 0.0 && y <= 0.0 {
                let t = if k == Math::PowRR {
                    "X = 0.0 and Y <= 0.0 in X**Y"
                } else {
                    "X = 0 and Y <= 0.0 in X**Y"
                };
                return domain(m, ctx, t, 0.0);
            }
            ret(if x == 0.0 {
                0.0
            } else if x == 1.0 || y == 0.0 {
                1.0
            } else if y == 1.0 {
                x
            } else {
                x.powf(y)
            })
        }
        Math::Ceil => ret(ceil_r(real(a, 0)?)),
        Math::Floor => ret(floor_r(real(a, 0)?)),
        Math::Round => {
            let x = real(a, 0)?;
            ret(if x > 0.0 {
                floor_r(x + 0.5)
            } else if x < 0.0 {
                ceil_r(x - 0.5)
            } else {
                0.0
            })
        }
        Math::Trunc => {
            let x = real(a, 0)?;
            ret(if x > 0.0 {
                floor_r(x)
            } else if x < 0.0 {
                ceil_r(x)
            } else {
                0.0
            })
        }
        Math::Sign => {
            let x = real(a, 0)?;
            ret(if x > 0.0 {
                1.0
            } else if x < 0.0 {
                -1.0
            } else {
                0.0
            })
        }
    }
}

// ---- std_logic_arith (Synopsys) ----

/// Give the call back to the VHDL body: the metavalue and range cases of the Synopsys helpers.
fn fallback(m: &mut Machine) -> Ret {
    m.ieee_fallback = true;
    Ok(Some(0))
}

/// An operand the helpers treat as binary: `(n-1 downto 0)` with every element `0 1 L H`.
fn bin_down(v: &Vh) -> Option<BitVec> {
    if v.v.is_empty() || !v.down || v.right != 0 || v.left != v.v.len() as i64 - 1 {
        return None;
    }
    v.v.to_bitvec()
}

fn syn_add<const SUB: bool, const SIGNED: bool>(
    m: &mut Machine,
    _: Cx<'_, '_, '_>,
    _: Option<TyId>,
    a: &[u64],
) -> Ret {
    let (l, r) = (vh(m, a, 0)?, vh(m, a, 1)?);
    let (Some(x), Some(y)) = (bin_down(&l), bin_down(&r)) else {
        return fallback(m);
    };
    if x.len() != y.len() {
        return fallback(m);
    }
    let s = if SUB { x.sub(&y) } else { x.add(&y, false) };
    put(m, &Vh::down(LogicVec::from_bitvec(&s)))
}

fn syn_mult<const SIGNED: bool>(
    m: &mut Machine,
    _: Cx<'_, '_, '_>,
    _: Option<TyId>,
    a: &[u64],
) -> Ret {
    let (l, r) = (vh(m, a, 0)?, vh(m, a, 1)?);
    let (Some(x), Some(y)) = (bin_down(&l), bin_down(&r)) else {
        return fallback(m);
    };
    let n = x.len() + y.len();
    let p = x.extend(n, SIGNED).mul_trunc(&y.extend(n, SIGNED), n);
    put(m, &Vh::down(LogicVec::from_bitvec(&p)))
}

fn make_binary(m: &mut Machine, _: Cx<'_, '_, '_>, _: Option<TyId>, a: &[u64]) -> Ret {
    let l = vh(m, a, 0)?;
    let Some(b) = l.v.to_bitvec() else {
        return fallback(m);
    };
    put(
        m,
        &Vh {
            v: LogicVec::from_bitvec(&b),
            ..l
        },
    )
}

/// `conv_unsigned`/`conv_signed` of a vector: the low `min (len, size)` elements, above them the
/// zero (unsigned argument) or the argument's top element (signed argument).
fn conv_vec<const FROM_SIGNED: bool>(
    m: &mut Machine,
    _: Cx<'_, '_, '_>,
    _: Option<TyId>,
    a: &[u64],
) -> Ret {
    let l = vh(m, a, 0)?;
    let size = int(a, 1)?;
    let Some(b) = l.v.to_bitvec() else {
        return fallback(m);
    };
    if size < 1 || b.is_empty() {
        return fallback(m);
    }
    let n = size as usize;
    let keep = b.len().min(n);
    let fill = FROM_SIGNED && b.get(b.len() - 1);
    let mut r = BitVec::new(n);
    (0..n).for_each(|i| r.set(i, if i < keep { b.get(i) } else { fill }));
    put(m, &Vh::down(LogicVec::from_bitvec(&r)))
}

/// `conv_unsigned`/`conv_signed` of an integer: the bits of the two's complement value
/// (the body's `mod 2` and `/ 2` loop), `SIZE` of them.
fn conv_int<const SIGNED: bool>(
    m: &mut Machine,
    _: Cx<'_, '_, '_>,
    _: Option<TyId>,
    a: &[u64],
) -> Ret {
    let (arg, size) = (int(a, 0)?, int(a, 1)?);
    // `temp - 1` of INTEGER'LOW is an overflow error in the unsigned body.
    if size < 1 || (!SIGNED && arg == -(1i64 << 31)) {
        return fallback(m);
    }
    let n = size as usize;
    let mut t = arg;
    let mut r = BitVec::new(n);
    for i in 0..n {
        r.set(i, t.rem_euclid(2) == 1);
        // The signed body halves INTEGER'LOW without the `- 1` of the other negative values.
        t = if t > 0 || (SIGNED && t <= -(1i64 << 31)) {
            t / 2
        } else {
            (t - 1) / 2
        };
    }
    put(m, &Vh::down(LogicVec::from_bitvec(&r)))
}

// ---- numeric_std ----

fn zero_ext(b: &BitVec, n: usize, signed: bool) -> BitVec {
    b.extend(n, signed)
}

/// `TO_UNSIGNED (arg, size)` / `TO_SIGNED` with their warning.
fn from_int(m: &mut Machine, ctx: Cx<'_, '_, '_>, arg: i64, size: i64, signed: bool) -> R<Vh> {
    if size < 1 {
        return Ok(Vh::null());
    }
    let n = size as usize;
    let (v, fits) = if signed {
        let fits = n >= 64 || {
            let lim = 1i64 << (n - 1);
            (-lim..lim).contains(&arg)
        };
        (BitVec::from_i64(n, arg), fits)
    } else {
        (
            BitVec::from_u64(n, arg as u64),
            n >= 64 || (arg as u64) >> n == 0,
        )
    };
    if !fits {
        let t = if signed { "TO_SIGNED" } else { "TO_UNSIGNED" };
        warn(m, ctx, &format!("NUMERIC_STD.{t}: vector truncated"))?;
    }
    Ok(Vh::down(LogicVec::from_bitvec(&v)))
}

/// A NATURAL operand: the subtype check the body's formal makes.
fn natural(i: i64) -> R<i64> {
    if i < 0 {
        return rt_err(format!(
            "value {i} outside of NATURAL range 0 to 2147483647"
        ));
    }
    Ok(i)
}

fn add_vv(l: &LogicVec, r: &LogicVec, sub: bool, signed: bool) -> Vh {
    if l.is_empty() || r.is_empty() {
        return Vh::null();
    }
    let size = l.len().max(r.len());
    let (Some(a), Some(b)) = (l.to_bitvec(), r.to_bitvec()) else {
        return Vh::down(x_vec(size));
    };
    let (a, b) = (zero_ext(&a, size, signed), zero_ext(&b, size, signed));
    let s = if sub { a.sub(&b) } else { a.add(&b, false) };
    Vh::down(LogicVec::from_bitvec(&s))
}

fn add<const SUB: bool, const SIGNED: bool, const FORM: u8>(
    m: &mut Machine,
    ctx: Cx<'_, '_, '_>,
    _: Option<TyId>,
    a: &[u64],
) -> Ret {
    let out = match FORM {
        0 => add_vv(&vh(m, a, 0)?.v, &vh(m, a, 1)?.v, SUB, SIGNED),
        1 => {
            let l = vh(m, a, 0)?;
            let i = int(a, 1)?;
            let i = if SIGNED { i } else { natural(i)? };
            let r = from_int(m, ctx, i, l.v.len() as i64, SIGNED)?;
            add_vv(&l.v, &r.v, SUB, SIGNED)
        }
        _ => {
            let r = vh(m, a, 1)?;
            let i = int(a, 0)?;
            let i = if SIGNED { i } else { natural(i)? };
            let l = from_int(m, ctx, i, r.v.len() as i64, SIGNED)?;
            add_vv(&l.v, &r.v, SUB, SIGNED)
        }
    };
    put(m, &out)
}

fn mul<const SIGNED: bool>(m: &mut Machine, _: Cx<'_, '_, '_>, _: Option<TyId>, a: &[u64]) -> Ret {
    let (l, r) = (vh(m, a, 0)?.v, vh(m, a, 1)?.v);
    if l.is_empty() || r.is_empty() {
        return put(m, &Vh::null());
    }
    let n = l.len() + r.len();
    let (Some(x), Some(y)) = (l.to_bitvec(), r.to_bitvec()) else {
        return put(m, &Vh::down(x_vec(n)));
    };
    let p = zero_ext(&x, n, SIGNED).mul_trunc(&zero_ext(&y, n, SIGNED), n);
    put(m, &Vh::down(LogicVec::from_bitvec(&p)))
}

fn cmp_name(c: Cmp) -> &'static str {
    match c {
        Cmp::Gt => ">",
        Cmp::Lt => "<",
        Cmp::Le => "<=",
        Cmp::Ge => ">=",
        Cmp::Eq => "=",
        Cmp::Ne => "/=",
    }
}

const CMPS: [Cmp; 6] = [Cmp::Gt, Cmp::Lt, Cmp::Le, Cmp::Ge, Cmp::Eq, Cmp::Ne];

fn holds(c: Cmp, o: std::cmp::Ordering) -> bool {
    use std::cmp::Ordering::*;
    match c {
        Cmp::Gt => o == Greater,
        Cmp::Lt => o == Less,
        Cmp::Le => o != Greater,
        Cmp::Ge => o != Less,
        Cmp::Eq => o == Equal,
        Cmp::Ne => o != Equal,
    }
}

fn compare<const C: u8, const SIGNED: bool, const FORM: u8>(
    m: &mut Machine,
    ctx: Cx<'_, '_, '_>,
    _: Option<TyId>,
    a: &[u64],
) -> Ret {
    let c = CMPS[C as usize];
    let dflt = c == Cmp::Ne;
    let bad = |m: &mut Machine, ctx: Cx<'_, '_, '_>, what: &str| -> Ret {
        warn(
            m,
            ctx,
            &format!(
                "NUMERIC_STD.\"{}\": {what} detected, returning {}",
                cmp_name(c),
                if dflt { "TRUE" } else { "FALSE" }
            ),
        )?;
        Ok(Some(u64::from(dflt)))
    };
    let (l, r) = match FORM {
        0 => (vh(m, a, 0)?.v, Some(vh(m, a, 1)?.v)),
        1 => (vh(m, a, 0)?.v, None),
        _ => (vh(m, a, 1)?.v, None),
    };
    if l.is_empty() || r.as_ref().is_some_and(LogicVec::is_empty) {
        return bad(m, ctx, "null argument");
    }
    let lb = l.to_bitvec();
    let rb = r.as_ref().map(LogicVec::to_bitvec);
    let (Some(lb), true) = (lb, rb.as_ref().is_none_or(Option::is_some)) else {
        return bad(m, ctx, "metavalue");
    };
    let ord = if let Some(Some(rb)) = rb {
        let n = lb.len().max(rb.len());
        zero_ext(&lb, n, SIGNED).cmp_num(&zero_ext(&rb, n, SIGNED), SIGNED)
    } else if FORM == 1 {
        let i = int(a, 1)?;
        let i = if SIGNED { i } else { natural(i)? };
        vec_vs_int(&lb, SIGNED, i)
    } else {
        let i = int(a, 0)?;
        let i = if SIGNED { i } else { natural(i)? };
        vec_vs_int(&lb, SIGNED, i).reverse()
    };
    Ok(Some(u64::from(holds(c, ord))))
}

/// The order of a 0/1 vector's value and an integer, whatever the vector's length.
fn vec_vs_int(b: &BitVec, signed: bool, i: i64) -> std::cmp::Ordering {
    use std::cmp::Ordering::*;
    if signed {
        match b.to_i64() {
            Some(v) => v.cmp(&i),
            None if b.get(b.len() - 1) => Less,
            None => Greater,
        }
    } else {
        match b.to_u64() {
            Some(v) => v.cmp(&(i as u64)),
            None => Greater,
        }
    }
}

fn to_integer<const SIGNED: bool>(
    m: &mut Machine,
    ctx: Cx<'_, '_, '_>,
    _: Option<TyId>,
    a: &[u64],
) -> Ret {
    let l = vh(m, a, 0)?.v;
    if l.is_empty() {
        warn(m, ctx, "NUMERIC_STD.TO_INTEGER: null detected, returning 0")?;
        return Ok(Some(0));
    }
    let Some(b) = l.to_bitvec() else {
        warn(
            m,
            ctx,
            "NUMERIC_STD.TO_INTEGER: metavalue detected, returning 0",
        )?;
        return Ok(Some(0));
    };
    if SIGNED && l.len() == 1 && b.get(0) {
        // The body negates `XARG + 1`, whose `TO_SIGNED (1, 1)` truncates.
        warn(m, ctx, "NUMERIC_STD.TO_SIGNED: vector truncated")?;
    }
    // The body accumulates the magnitude (`-(x + 1)`, that is `not x`, for a negative signed
    // value) most significant bit first with `R := R + R`, a checked INTEGER addition.
    let neg = SIGNED && b.get(b.len() - 1);
    let mag = if neg { b.not() } else { b };
    let mut r: u64 = 0;
    for i in (0..mag.len()).rev() {
        if r >= 1 << 30 {
            return rt_err(format!(
                "result of {r} + {r} cannot be represented as INTEGER"
            ));
        }
        r = 2 * r + u64::from(mag.get(i));
    }
    Ok(Some(if neg { (-(r as i64) - 1) as u64 } else { r }))
}

fn to_unsigned<const SIGNED: bool>(
    m: &mut Machine,
    ctx: Cx<'_, '_, '_>,
    _: Option<TyId>,
    a: &[u64],
) -> Ret {
    let (v, size) = (int(a, 0)?, int(a, 1)?);
    let v = if SIGNED { v } else { natural(v)? };
    let size = natural(size)?;
    let o = from_int(m, ctx, v, size, SIGNED)?;
    put(m, &o)
}

fn resize<const SIGNED: bool>(
    m: &mut Machine,
    _: Cx<'_, '_, '_>,
    _: Option<TyId>,
    a: &[u64],
) -> Ret {
    let l = vh(m, a, 0)?.v;
    let new = natural(int(a, 1)?)? as usize;
    if new < 1 {
        return put(m, &Vh::null());
    }
    let len = l.len();
    let v = if len == 0 {
        LogicVec::new(new, StdULogic::Zero)
    } else if SIGNED {
        // The top element is the argument's sign element; below it the low elements.
        let keep = len.min(new) - 1;
        let sign = l.get(len - 1);
        LogicVec::from_fn(new, |i| if i < keep { l.get(i) } else { sign })
    } else {
        LogicVec::from_fn(new, |i| if i < len { l.get(i) } else { StdULogic::Zero })
    };
    put(m, &Vh::down(v))
}

fn shift<const K: u8>(m: &mut Machine, _: Cx<'_, '_, '_>, _: Option<TyId>, a: &[u64]) -> Ret {
    let l = vh(m, a, 0)?;
    let count = natural(int(a, 1)?)? as usize;
    let n = l.v.len();
    let v = match K {
        0 => l.v.shl(count),
        1 => l.v.shr(count),
        2 => {
            // XSRA returns its argument unchanged (its own bounds) for 0 or 1 elements.
            if n <= 1 || count == 0 {
                return put(m, &l);
            }
            l.v.sra(count.min(n - 1))
        }
        _ if n == 0 => return rt_err("division by zero"),
        3 => l.v.rol(count),
        _ => l.v.ror(count),
    };
    put(m, &Vh::down(v))
}

macro_rules! pick3 {
    ($f:ident, $a:expr, $b:expr, $c:expr) => {
        match ($a, $b, $c) {
            (false, false, 0) => $f::<false, false, 0> as NativeFn,
            (false, false, 1) => $f::<false, false, 1> as NativeFn,
            (false, false, _) => $f::<false, false, 2> as NativeFn,
            (false, true, 0) => $f::<false, true, 0> as NativeFn,
            (false, true, 1) => $f::<false, true, 1> as NativeFn,
            (false, true, _) => $f::<false, true, 2> as NativeFn,
            (true, false, 0) => $f::<true, false, 0> as NativeFn,
            (true, false, 1) => $f::<true, false, 1> as NativeFn,
            (true, false, _) => $f::<true, false, 2> as NativeFn,
            (true, true, 0) => $f::<true, true, 0> as NativeFn,
            (true, true, 1) => $f::<true, true, 1> as NativeFn,
            (true, true, _) => $f::<true, true, 2> as NativeFn,
        }
    };
}

fn form(f: Form) -> u8 {
    match f {
        Form::Vv => 0,
        Form::Vi => 1,
        Form::Iv => 2,
    }
}

/// The native of an `yavs.ieee.<key>` symbol.
pub(crate) fn lookup(symbol: &str) -> Option<NativeFn> {
    // A native that may give the call back carries the body's function id after `@`.
    let key = symbol.strip_prefix("yavs.ieee.")?;
    let op = yavs_ir::ieee::parse(key.split_once('@').map_or(key, |k| k.0))?;
    Some(match op {
        Op::Gate(g) => [
            gate::<0> as NativeFn,
            gate::<1>,
            gate::<2>,
            gate::<3>,
            gate::<4>,
            gate::<5>,
        ][GATES.iter().position(|x| *x == g)?],
        Op::VecGate(g) => [
            vec_gate::<0> as NativeFn,
            vec_gate::<1>,
            vec_gate::<2>,
            vec_gate::<3>,
            vec_gate::<4>,
            vec_gate::<5>,
        ][GATES.iter().position(|x| *x == g)?],
        Op::Not => not,
        Op::VecNot => vec_not,
        Op::Conv(0, false) => conv::<0, false>,
        Op::Conv(1, false) => conv::<1, false>,
        Op::Conv(_, false) => conv::<2, false>,
        Op::Conv(0, true) => conv::<0, true>,
        Op::Conv(1, true) => conv::<1, true>,
        Op::Conv(_, true) => conv::<2, true>,
        Op::To01 => to_01_vec,
        Op::Truthy => truthy,
        Op::Edge(true) => edge::<true>,
        Op::Edge(false) => edge::<false>,
        Op::Add {
            sub,
            signed,
            form: f,
        } => pick3!(add, sub, signed, form(f)),
        Op::Mul { signed: false } => mul::<false>,
        Op::Mul { signed: true } => mul::<true>,
        Op::Compare {
            cmp,
            signed,
            form: f,
        } => {
            let c = CMPS.iter().position(|x| *x == cmp)?;
            let fs = form(f);
            macro_rules! cs {
                ($($c:literal),*) => {
                    match (c, signed, fs) {
                        $(
                            ($c, false, 0) => compare::<$c, false, 0> as NativeFn,
                            ($c, false, 1) => compare::<$c, false, 1> as NativeFn,
                            ($c, false, _) => compare::<$c, false, 2> as NativeFn,
                            ($c, true, 0) => compare::<$c, true, 0> as NativeFn,
                            ($c, true, 1) => compare::<$c, true, 1> as NativeFn,
                            ($c, true, _) => compare::<$c, true, 2> as NativeFn,
                        )*
                        _ => return None,
                    }
                };
            }
            cs!(0, 1, 2, 3, 4, 5)
        }
        Op::ToInteger { signed: false } => to_integer::<false>,
        Op::ToInteger { signed: true } => to_integer::<true>,
        Op::ToUnsigned => to_unsigned::<false>,
        Op::ToSigned => to_unsigned::<true>,
        Op::Resize { signed: false } => resize::<false>,
        Op::Resize { signed: true } => resize::<true>,
        Op::Syn(Syn::Add { sub, signed }) => match (sub, signed) {
            (false, false) => syn_add::<false, false>,
            (false, true) => syn_add::<false, true>,
            (true, false) => syn_add::<true, false>,
            (true, true) => syn_add::<true, true>,
        },
        Op::Syn(Syn::Mult { signed: false }) => syn_mult::<false>,
        Op::Syn(Syn::Mult { signed: true }) => syn_mult::<true>,
        Op::Syn(Syn::MakeBinary) => make_binary,
        Op::Syn(Syn::Conv {
            from_signed: Some(false),
            ..
        }) => conv_vec::<false>,
        Op::Syn(Syn::Conv {
            from_signed: Some(true),
            ..
        }) => conv_vec::<true>,
        Op::Syn(Syn::Conv {
            to_signed,
            from_signed: None,
        }) => {
            if to_signed {
                conv_int::<true>
            } else {
                conv_int::<false>
            }
        }
        Op::Math(k) => {
            macro_rules! ms {
                ($($i:literal),*) => {
                    match MATH.iter().position(|x| *x == k)? {
                        $($i => math::<$i> as NativeFn,)*
                        _ => return None,
                    }
                };
            }
            ms!(0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14)
        }
        Op::Shift(Shift::Sll) => shift::<0>,
        Op::Shift(Shift::Srl) => shift::<1>,
        Op::Shift(Shift::Sra) => shift::<2>,
        Op::Shift(Shift::Rol) => shift::<3>,
        Op::Shift(Shift::Ror) => shift::<4>,
    })
}
