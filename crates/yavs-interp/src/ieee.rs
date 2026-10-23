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

use crate::exec::{Fault, Machine, R, add_addr, cold, ice, ice_f, rt_err};
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
        c => Err(ice_f!("std_ulogic code {c}")),
    }
}

fn int(a: &[u64], i: usize) -> R<i64> {
    Ok(arg(a, i)? as i64)
}

/// The data address, bounds and direction of the array operand `i` (the address of its fat pair).
#[inline]
fn fat_dim(m: &Machine, a: &[u64], i: usize) -> R<(u64, i64, i64, bool)> {
    let fat = arg(a, i)?;
    let w = m.rd_hot(fat, 16)?;
    let (data, dope) = (le8(&w[0..8]), le8(&w[8..16]));
    let (left, right, d) = m.dim_of(dope, 0)?;
    Ok((data, left, right, d == 1))
}

#[inline(always)]
fn le8(b: &[u8]) -> u64 {
    u64::from_le_bytes(b.try_into().unwrap_or([0; 8]))
}

fn vh(m: &Machine, a: &[u64], i: usize) -> R<Vh> {
    let (data, left, right, down) = fat_dim(m, a, i)?;
    let n = dim_len(left, right, down).ok_or_else(|| Fault::Runtime("array too long".into()))?;
    Ok(Vh {
        v: m.planes(data, n)?,
        left,
        right,
        down,
    })
}

fn put(m: &mut Machine, o: &Vh) -> Ret {
    // The fat pair, the dope vector and the planes in one block of temporary memory.
    let blocks = o.v.repr().blocks();
    let size = blocks.len() as u64 * 32;
    let fat = m.temp_alloc(16 + DOPE_DIM_BYTES + size)?;
    let (dope, data) = (add_addr(fat, 16)?, add_addr(fat, 16 + DOPE_DIM_BYTES)?);
    let b = m.wr(fat, 16 + DOPE_DIM_BYTES + size)?;
    let head = [data, dope, o.left as u64, o.right as u64, u64::from(o.down)];
    for (d, w) in b
        .as_chunks_mut::<8>()
        .0
        .iter_mut()
        .zip(head.iter().chain(blocks.as_flattened()))
    {
        d.copy_from_slice(&w.to_le_bytes());
    }
    Ok(Some(fat))
}

/// An array operand of 1 to 64 elements that holds only `0 1 L H` (`bit_vector`s included):
/// numeric_std on such operands is plain integer arithmetic on one word, with no allocation.
pub(crate) struct Small {
    /// Element count.
    pub(crate) n: usize,
    /// The elements as bits, element 0 in bit 0.
    pub(crate) bits: u64,
    /// No element is `L` or `H`: operations that move elements around keep those as they are.
    pure: bool,
    left: i64,
    right: i64,
    down: bool,
}

/// The low `n` bits (`n` from 1 to 64).
fn low(n: usize) -> u64 {
    if n >= 64 { !0 } else { (1 << n) - 1 }
}

/// [`Small`] of the array operand `i`, or `None` for anything else (empty, longer, a
/// metavalue), which the general path handles.
pub(crate) fn small(m: &Machine, a: &[u64], i: usize) -> R<Option<Small>> {
    let (data, left, right, down) = fat_dim(m, a, i)?;
    let n = dim_len(left, right, down).ok_or_else(|| Fault::Runtime("array too long".into()))?;
    if !(1..=64).contains(&n) {
        return Ok(None);
    }
    let b = m.rd_hot(data, 32)?;
    let (p0, p1, p2, p3) = (
        le8(&b[0..8]),
        le8(&b[8..16]),
        le8(&b[16..24]),
        le8(&b[24..32]),
    );
    // `0` or `L`: plane 1 only (plane 2 is the weak bit); `1` or `H`: planes 0 and 1.
    let (zero, one) = (!p3 & p1 & !p0, !p3 & p1 & p0);
    let n = n as usize;
    if (zero | one) & low(n) != low(n) {
        return Ok(None);
    }
    Ok(Some(Small {
        n,
        bits: one & low(n),
        pure: p2 & low(n) == 0,
        left,
        right,
        down,
    }))
}

/// The result `bits` of `n` elements, bounds `left`, `right`, `down`, as [`put`] writes it.
fn put_small(m: &mut Machine, n: usize, bits: u64, left: i64, right: i64, down: bool) -> R<u64> {
    let fat = m.temp_alloc(16 + DOPE_DIM_BYTES + 32)?;
    let (dope, data) = (add_addr(fat, 16)?, add_addr(fat, 16 + DOPE_DIM_BYTES)?);
    let b = m.wr(fat, 16 + DOPE_DIM_BYTES + 32)?;
    let mask = low(n);
    let words = [
        data,
        dope,
        left as u64,
        right as u64,
        u64::from(down),
        bits & mask,
        mask,
        0,
        0,
    ];
    for (d, w) in b.as_chunks_mut::<8>().0.iter_mut().zip(words) {
        d.copy_from_slice(&w.to_le_bytes());
    }
    Ok(fat)
}

/// [`put_small`] with the bounds `(n-1 downto 0)` ([`Vh::down`]).
fn put_down(m: &mut Machine, n: usize, bits: u64) -> R<u64> {
    put_small(m, n, bits, n as i64 - 1, 0, true)
}

/// `bits` of `n` elements extended to `to` elements (`to >= n`): sign or zero.
fn extend_bits(bits: u64, n: usize, to: usize, signed: bool) -> u64 {
    if signed && n < to && bits >> (n - 1) & 1 == 1 {
        (bits | !low(n)) & low(to)
    } else {
        bits
    }
}

/// The value of `bits` of `n` elements as a signed integer.
fn signed_of(bits: u64, n: usize) -> i64 {
    ((bits << (64 - n)) as i64) >> (64 - n)
}

/// A report from a native, as `Inst::Report` does it. The process ends after a report at the
/// stop severity (`Machine::ieee_stop`).
pub(crate) fn report(m: &mut Machine, ctx: Cx<'_, '_, '_>, s: Severity, text: &str) -> R<()> {
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
            if !m.rt.asserts.report_elab(s) {
                return Ok(());
            }
            crate::env::print_elab(s, text);
            if let Some(r) = m.init_reports.as_mut() {
                r.push((s as u8, text.to_string()));
            }
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

/// A warning of `numeric_std`, or of `numeric_bit` (`bit`: the call has a type argument), whose
/// texts differ in the package. The arrays need no conversion: `bit` elements are stored with
/// the codes of `'0'` and `'1'` of `std_ulogic`.
fn warnb(m: &mut Machine, ctx: Cx<'_, '_, '_>, text: &str, bit: bool) -> R<()> {
    if bit {
        return warn(m, ctx, &text.replacen("NUMERIC_STD", "NUMERIC_BIT", 1));
    }
    warn(m, ctx, text)
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

/// The index of `g` in `GATES`, for `Op::Gate`.
pub(crate) fn gate_index(g: Gate) -> Option<u8> {
    GATES.iter().position(|x| *x == g).map(|i| i as u8)
}

/// A scalar gate on two `std_ulogic` codes, as the native `gate::<g>` computes it.
#[inline(always)]
pub(crate) fn scalar_gate(g: u8, x: u64, y: u64) -> R<u64> {
    static TABLES: std::sync::OnceLock<[[u8; 81]; 6]> = std::sync::OnceLock::new();
    let t = TABLES.get_or_init(|| {
        let mut t = [[0u8; 81]; 6];
        for (gi, gate) in GATES.iter().enumerate() {
            for a in 0..9u8 {
                for b in 0..9u8 {
                    let (x, y) = (StdULogic::from_code_pub(a), StdULogic::from_code_pub(b));
                    t[gi][usize::from(a) * 9 + usize::from(b)] = gate_of(*gate, x, y) as u8;
                }
            }
        }
        t
    });
    if x < 9
        && y < 9
        && let Some(row) = t.get(g as usize)
    {
        return Ok(u64::from(row[(x * 9 + y) as usize]));
    }
    Ok(gate_of(GATES[g as usize], code(x)?, code(y)?) as u64)
}

/// `not` of a `std_ulogic` code.
#[inline(always)]
pub(crate) fn scalar_not(x: u64) -> R<u64> {
    // `to_x01` of the complement: U X 0 1 Z W L H - become U X 1 0 X X 1 0 X.
    const NOT: [u8; 9] = [0, 1, 3, 2, 1, 1, 3, 2, 1];
    match NOT.get(x as usize) {
        Some(&c) => Ok(u64::from(c)),
        None => Ok(code(x)?.not() as u64),
    }
}

#[inline(always)]
fn code(c: u64) -> R<StdULogic> {
    match c {
        0..=8 => Ok(StdULogic::from_code_pub(c as u8)),
        c => Err(ice_f!("std_ulogic code {c}")),
    }
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
    if let (Some(l), Some(r)) = (small(m, a, 0)?, small(m, a, 1)?)
        && l.n == r.n
    {
        let bits = match g {
            Gate::And => l.bits & r.bits,
            Gate::Nand => !(l.bits & r.bits),
            Gate::Or => l.bits | r.bits,
            Gate::Nor => !(l.bits | r.bits),
            Gate::Xor => l.bits ^ r.bits,
            Gate::Xnor => !(l.bits ^ r.bits),
        };
        return put_small(m, l.n, bits, 1, l.n as i64, false).map(Some);
    }
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

/// `numeric_std` gate of two `unsigned` or `signed`: the std_logic_1164 gate of the same elements
/// with the bounds `(n-1 downto 0)`. Unequal lengths go back to the body.
fn num_gate<const G: u8>(m: &mut Machine, _: Cx<'_, '_, '_>, _: Option<TyId>, a: &[u64]) -> Ret {
    let g = GATES[G as usize];
    if let (Some(l), Some(r)) = (small(m, a, 0)?, small(m, a, 1)?)
        && l.n == r.n
    {
        let bits = match g {
            Gate::And => l.bits & r.bits,
            Gate::Nand => !(l.bits & r.bits),
            Gate::Or => l.bits | r.bits,
            Gate::Nor => !(l.bits | r.bits),
            Gate::Xor => l.bits ^ r.bits,
            Gate::Xnor => !(l.bits ^ r.bits),
        };
        return put_down(m, l.n, bits).map(Some);
    }
    let (l, r) = (vh(m, a, 0)?, vh(m, a, 1)?);
    if l.v.len() != r.v.len() {
        return fallback(m);
    }
    let v = match g {
        Gate::And => l.v.and(&r.v),
        Gate::Nand => l.v.nand(&r.v),
        Gate::Or => l.v.or(&r.v),
        Gate::Nor => l.v.nor(&r.v),
        Gate::Xor => l.v.xor(&r.v),
        Gate::Xnor => l.v.xnor(&r.v),
    };
    put(m, &Vh::down(v))
}

/// `numeric_std` `not` of an `unsigned` or `signed`.
fn num_not(m: &mut Machine, _: Cx<'_, '_, '_>, _: Option<TyId>, a: &[u64]) -> Ret {
    if let Some(l) = small(m, a, 0)? {
        return put_down(m, l.n, !l.bits).map(Some);
    }
    let l = vh(m, a, 0)?;
    put(m, &Vh::down(l.v.not()))
}

fn not(_: &mut Machine, _: Cx<'_, '_, '_>, _: Option<TyId>, a: &[u64]) -> Ret {
    Ok(Some(sul(a, 0)?.not() as u64))
}

fn vec_not(m: &mut Machine, _: Cx<'_, '_, '_>, _: Option<TyId>, a: &[u64]) -> Ret {
    if let Some(l) = small(m, a, 0)? {
        return put_small(m, l.n, !l.bits, 1, l.n as i64, false).map(Some);
    }
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
    Ok(Some(conv_of(K, sul(a, 0)?) as u64))
}

/// `to_x01`, `to_x01z` or `to_ux01` (`k` 0, 1, 2) of one `std_ulogic`.
fn conv_of(k: u8, s: StdULogic) -> StdULogic {
    match k {
        0 => s.to_x01(),
        1 => s.to_x01z(),
        _ => s.to_ux01(),
    }
}

/// The results of the native of `symbol` when it is a function of one or two `std_ulogic`
/// operands (`std_logic_1164` gates, `not`, `to_x01`/`to_x01z`/`to_ux01`, `??`), by operand
/// code (`a * 9 + b` for two), as the native computes them; `None` for any other native. For
/// compiled code (a table lookup instead of the call); an operand beyond the nine codes is
/// still the native's error.
pub fn code_table(symbol: &str) -> Option<Vec<u8>> {
    let key = symbol.strip_prefix("yavs.ieee.")?;
    let op = yavs_ir::ieee::parse(key.split_once('@').map_or(key, |k| k.0))?;
    let all = || (0..9).map(StdULogic::from_code_pub);
    Some(match op {
        Op::Gate(g) => all()
            .flat_map(|x| all().map(move |y| gate_of(g, x, y) as u8))
            .collect(),
        Op::Not => all().map(|x| x.not() as u8).collect(),
        Op::Conv(k, false) => all().map(|x| conv_of(k, x) as u8).collect(),
        Op::Truthy => all().map(|x| u8::from(x.is_true())).collect(),
        _ => return None,
    })
}

/// Whether the native of `symbol` is `rising_edge` (`Some(true)`) or `falling_edge` of a
/// `std_ulogic` signal (`TierHost::edge`).
pub fn edge_native(symbol: &str) -> Option<bool> {
    let key = symbol.strip_prefix("yavs.ieee.")?;
    match yavs_ir::ieee::parse(key.split_once('@').map_or(key, |k| k.0))? {
        Op::Edge(rising) => Some(rising),
        _ => None,
    }
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

/// `is_x` of a vector: any element not `0 1 L H`. A null vector has none.
/// A gate over all elements of a vector (`Op::Reduce`), on the plane words: `and` is `0` if
/// any element is `0`/`L`, else `U` if any is `U`, else `X` if any is not `1`/`H`, else `1`;
/// `or` the same with `1` and `0` swapped; `xor` `U` if any is `U`, else `X` if any is not
/// `0 1 L H`, else the parity of the ones. The bodies fold the same tables element by element.
fn reduce<const G: u8>(m: &mut Machine, _: Cx<'_, '_, '_>, _: Option<TyId>, a: &[u64]) -> Ret {
    let (data, left, right, down) = fat_dim(m, a, 0)?;
    let n = dim_len(left, right, down).ok_or_else(|| Fault::Runtime("array too long".into()))?;
    let n = usize::try_from(n).map_err(|_| Fault::Runtime("array too long".into()))?;
    let b = match n {
        0 => &[][..],
        _ => m.rd(data, n.div_ceil(64) as u64 * 32)?,
    };
    // Per kind of element: any `1`/`H`, any `0`/`L`, any `U`, any of the other five.
    let (mut one, mut zero, mut u, mut x, mut parity) = (false, false, false, false, 0);
    for (i, w) in b.as_chunks::<32>().0.iter().enumerate() {
        let valid = low(n - i * 64);
        let (p0, p1, p2, p3) = (
            le8(&w[0..8]),
            le8(&w[8..16]),
            le8(&w[16..24]),
            le8(&w[24..32]),
        );
        let o = !p3 & p1 & p0 & valid;
        let z = !p3 & p1 & !p0 & valid;
        let un = !(p0 | p1 | p2 | p3) & valid;
        one |= o != 0;
        zero |= z != 0;
        u |= un != 0;
        x |= valid & !(o | z | un) != 0;
        parity ^= o.count_ones() & 1;
    }
    let meta = if u { StdULogic::U } else { StdULogic::X };
    let r = match GATES[G as usize] {
        Gate::And | Gate::Nand if zero => StdULogic::Zero,
        Gate::Or | Gate::Nor if one => StdULogic::One,
        _ if u || x => meta,
        Gate::And | Gate::Nand => StdULogic::One,
        Gate::Or | Gate::Nor => StdULogic::Zero,
        Gate::Xor | Gate::Xnor if parity == 1 => StdULogic::One,
        Gate::Xor | Gate::Xnor => StdULogic::Zero,
    };
    let r = match GATES[G as usize] {
        Gate::Nand | Gate::Nor | Gate::Xnor => r.not(),
        _ => r,
    };
    Ok(Some(r as u64))
}

fn is_x_vec(m: &mut Machine, _: Cx<'_, '_, '_>, _: Option<TyId>, a: &[u64]) -> Ret {
    Ok(Some(u64::from(vh(m, a, 0)?.v.has_x())))
}

fn is_x_sul(_: &mut Machine, _: Cx<'_, '_, '_>, _: Option<TyId>, a: &[u64]) -> Ret {
    Ok(Some(u64::from(sul(a, 0)?.is_x())))
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
    Ok(Some(u64::from(edge_of(
        m,
        ctx,
        arg(a, 0)?,
        arg(a, 1)?,
        RISING,
    )?)))
}

/// `rising_edge` / `falling_edge` of the signal in cells `c0`, `c1` (`edge`).
pub(crate) fn edge_of(m: &Machine, ctx: Cx<'_, '_, '_>, c0: u64, c1: u64, rising: bool) -> R<bool> {
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
    c.edge(sr, rising)?
        .ok_or_else(|| ice("edge of a null signal range"))
}

// ---- math_real ----

fn real(a: &[u64], i: usize) -> R<f64> {
    Ok(f64::from_bits(arg(a, i)?))
}

/// `FLOOR`/`CEIL` return the argument unchanged from INTEGER'HIGH up (the body converts to
/// INTEGER below that).
fn large(m: &Machine) -> f64 {
    m.rt.revision.integer_bounds().high as f64
}

/// `FLOOR`/`CEIL` as the bodies compute them: from `RD = REAL (INTEGER (X))` (round half away
/// from zero), so a result of zero is always `+0.0`.
fn floor_r(x: f64, large: f64) -> f64 {
    if x.abs() >= large {
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

fn ceil_r(x: f64, large: f64) -> f64 {
    if x.abs() >= large {
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
    let lg = large(m);
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
        Math::Ceil => ret(ceil_r(real(a, 0)?, large(m))),
        Math::Floor => ret(floor_r(real(a, 0)?, large(m))),
        Math::Round => {
            let x = real(a, 0)?;
            ret(if x > 0.0 {
                floor_r(x + 0.5, lg)
            } else if x < 0.0 {
                ceil_r(x - 0.5, lg)
            } else {
                0.0
            })
        }
        Math::Trunc => {
            let x = real(a, 0)?;
            ret(if x > 0.0 {
                floor_r(x, lg)
            } else if x < 0.0 {
                ceil_r(x, lg)
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

/// `is_less` and friends: operands `(n-1 downto 0)` of one length whose elements are all `0` or
/// `1` (the bodies compare elements with `'0'` and `'1'`, so `L` and `H` are the body's).
fn syn_less<const OR_EQUAL: bool, const SIGNED: bool>(
    m: &mut Machine,
    _: Cx<'_, '_, '_>,
    _: Option<TyId>,
    a: &[u64],
) -> Ret {
    let (l, r) = (vh(m, a, 0)?, vh(m, a, 1)?);
    let plain = |v: &Vh| {
        let b = bin_down(v)?;
        (LogicVec::from_bitvec(&b) == v.v).then_some(b)
    };
    let (Some(x), Some(y)) = (plain(&l), plain(&r)) else {
        return fallback(m);
    };
    if x.len() != y.len() {
        return fallback(m);
    }
    let o = x.cmp_num(&y, SIGNED);
    Ok(Some(u64::from(if OR_EQUAL {
        o.is_le()
    } else {
        o.is_lt()
    })))
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
    // INTEGER'LOW: -2**31 in 2008, -2**63 in 2019.
    let low = m.rt.revision.integer_bounds().low;
    // `temp - 1` of INTEGER'LOW is an overflow error in the unsigned body.
    if size < 1 || (!SIGNED && arg == low) {
        return fallback(m);
    }
    let n = size as usize;
    let mut t = arg;
    let mut r = BitVec::new(n);
    for i in 0..n {
        r.set(i, t.rem_euclid(2) == 1);
        // The signed body halves INTEGER'LOW without the `- 1` of the other negative values.
        t = if t > 0 || (SIGNED && t <= low) {
            t / 2
        } else {
            (t - 1) / 2
        };
    }
    put(m, &Vh::down(LogicVec::from_bitvec(&r)))
}

/// `math_real.uniform`: the L'Ecuyer generator of the body in `i64`, every intermediate within
/// INTEGER for the seeds the two range checks admit. The out-of-range reports are the body's.
fn uniform(m: &mut Machine, ctx: Cx<'_, '_, '_>, _: Option<TyId>, a: &[u64]) -> Ret {
    let (pa, pb, px) = (arg(a, 0)?, arg(a, 1)?, arg(a, 2)?);
    let (s1, s2) = (m.load(pa, Width::B8)? as i64, m.load(pb, Width::B8)? as i64);
    for (bad, text) in [
        (s1 > 2147483562, "SEED1 > 2147483562 in UNIFORM"),
        (s2 > 2147483398, "SEED2 > 2147483398 in UNIFORM"),
    ] {
        if bad {
            report(m, ctx, Severity::Error, text)?;
            m.store(px, Width::B8, 0f64.to_bits())?;
            return Ok(None);
        }
    }
    let k = s1 / 53668;
    let mut t1 = 40014 * (s1 - k * 53668) - k * 12211;
    if t1 < 0 {
        t1 += 2147483563;
    }
    let k = s2 / 52774;
    let mut t2 = 40692 * (s2 - k * 52774) - k * 3791;
    if t2 < 0 {
        t2 += 2147483399;
    }
    let mut z = t1 - t2;
    if z < 1 {
        z += 2147483562;
    }
    m.store(pa, Width::B8, t1 as u64)?;
    m.store(pb, Width::B8, t2 as u64)?;
    m.store(px, Width::B8, (z as f64 * 4.656613e-10).to_bits())?;
    Ok(None)
}

/// `to_hstring` / `to_ostring`: groups of 4 (3) elements from the left, padded on the left with
/// `Z` when the leftmost element is `Z`, else `0`; a group of `Z` is `Z`, any other non-`0 1`
/// element (after `L`, `H` are mapped) makes it `X`.
fn to_str<const OCT: bool>(m: &mut Machine, _: Cx<'_, '_, '_>, _: Option<TyId>, a: &[u64]) -> Ret {
    let w = if OCT { 3 } else { 4 };
    let v = vh(m, a, 0)?.v;
    let n = v.len();
    if n == 0 {
        return fallback(m);
    }
    let rl = n.div_ceil(w);
    let pad = rl * w - n;
    let pad_el = if v.get(n - 1) == StdULogic::Z {
        StdULogic::Z
    } else {
        StdULogic::Zero
    };
    let out: Vec<u8> = (0..rl)
        .map(|g| {
            let mut val = 0u32;
            let (mut zs, mut xs) = (0, false);
            for k in g * w..(g + 1) * w {
                let e = if k < pad {
                    pad_el
                } else {
                    v.get(n - 1 - (k - pad))
                };
                match e.to_x01z() {
                    StdULogic::Zero => val *= 2,
                    StdULogic::One => val = val * 2 + 1,
                    StdULogic::Z => zs += 1,
                    _ => xs = true,
                }
            }
            if zs == w {
                b'Z'
            } else if zs > 0 || xs {
                b'X'
            } else {
                b"0123456789ABCDEF"[val as usize]
            }
        })
        .collect();
    Ok(Some(m.ret_string(&out)?))
}

// ---- numeric_std ----

fn zero_ext(b: &BitVec, n: usize, signed: bool) -> BitVec {
    b.extend(n, signed)
}

/// `TO_UNSIGNED (arg, size)` / `TO_SIGNED` with their warning.
fn from_int(
    m: &mut Machine,
    ctx: Cx<'_, '_, '_>,
    arg: i64,
    size: i64,
    signed: bool,
    bit: bool,
) -> R<Vh> {
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
        warnb(m, ctx, &format!("NUMERIC_STD.{t}: vector truncated"), bit)?;
    }
    Ok(Vh::down(LogicVec::from_bitvec(&v)))
}

/// The bits of `TO_UNSIGNED (arg, size)` / `TO_SIGNED` for `size` from 1 to 64, with the same
/// warning as [`from_int`].
fn int_bits(
    m: &mut Machine,
    ctx: Cx<'_, '_, '_>,
    arg: i64,
    size: usize,
    signed: bool,
    bit: bool,
) -> R<u64> {
    let fits = if signed {
        size >= 64 || (-(1i64 << (size - 1))..1i64 << (size - 1)).contains(&arg)
    } else {
        size >= 64 || (arg as u64) >> size == 0
    };
    if !fits {
        let t = if signed { "TO_SIGNED" } else { "TO_UNSIGNED" };
        warnb(m, ctx, &format!("NUMERIC_STD.{t}: vector truncated"), bit)?;
    }
    Ok(arg as u64 & low(size))
}

/// A NATURAL operand: the subtype check the body's formal makes.
fn natural(m: &Machine, i: i64) -> R<i64> {
    if i < 0 {
        return rt_err(format!(
            "value {i} outside of NATURAL range 0 to {}",
            m.rt.revision.integer_bounds().high
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

/// [`add`] on operands of at most 64 elements without a metavalue; `None` for any other.
fn add_small<const SUB: bool, const SIGNED: bool, const FORM: u8>(
    m: &mut Machine,
    ctx: Cx<'_, '_, '_>,
    bit: bool,
    a: &[u64],
) -> R<Option<u64>> {
    let (n, x, y) = match FORM {
        0 => {
            let (Some(l), Some(r)) = (small(m, a, 0)?, small(m, a, 1)?) else {
                return Ok(None);
            };
            let n = l.n.max(r.n);
            (
                n,
                extend_bits(l.bits, l.n, n, SIGNED),
                extend_bits(r.bits, r.n, n, SIGNED),
            )
        }
        1 => {
            let Some(l) = small(m, a, 0)? else {
                return Ok(None);
            };
            let i = int(a, 1)?;
            let i = if SIGNED { i } else { natural(m, i)? };
            (l.n, l.bits, int_bits(m, ctx, i, l.n, SIGNED, bit)?)
        }
        _ => {
            let Some(r) = small(m, a, 1)? else {
                return Ok(None);
            };
            let i = int(a, 0)?;
            let i = if SIGNED { i } else { natural(m, i)? };
            (r.n, int_bits(m, ctx, i, r.n, SIGNED, bit)?, r.bits)
        }
    };
    let s = if SUB {
        x.wrapping_sub(y)
    } else {
        x.wrapping_add(y)
    };
    put_down(m, n, s).map(Some)
}

fn add<const SUB: bool, const SIGNED: bool, const FORM: u8>(
    m: &mut Machine,
    ctx: Cx<'_, '_, '_>,
    t: Option<TyId>,
    a: &[u64],
) -> Ret {
    let bit = t.is_some();
    if let Some(r) = add_small::<SUB, SIGNED, FORM>(m, ctx, bit, a)? {
        return Ok(Some(r));
    }
    let out = match FORM {
        0 => add_vv(&vh(m, a, 0)?.v, &vh(m, a, 1)?.v, SUB, SIGNED),
        1 => {
            let l = vh(m, a, 0)?;
            let i = int(a, 1)?;
            let i = if SIGNED { i } else { natural(m, i)? };
            let r = from_int(m, ctx, i, l.v.len() as i64, SIGNED, bit)?;
            add_vv(&l.v, &r.v, SUB, SIGNED)
        }
        _ => {
            let r = vh(m, a, 1)?;
            let i = int(a, 0)?;
            let i = if SIGNED { i } else { natural(m, i)? };
            let l = from_int(m, ctx, i, r.v.len() as i64, SIGNED, bit)?;
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

/// Quotient (`num` long) and remainder (`den` long) of two unsigned vectors, `den` not zero.
fn divmod(num: &BitVec, den: &BitVec) -> (BitVec, BitVec) {
    let (n, d) = (num.len(), den.len());
    if let (Some(a), Some(b)) = (num.to_u64(), den.to_u64()) {
        return (BitVec::from_u64(n, a / b), BitVec::from_u64(d, a % b));
    }
    // Shift-subtract: the running remainder stays below `den`, so `d + 1` bits hold it.
    let den = den.extend(d + 1, false);
    let (mut q, mut r) = (BitVec::new(n), BitVec::new(d + 1));
    for i in (0..n).rev() {
        r = r.add(&r, false);
        r.set(0, num.get(i));
        if r.cmp_num(&den, false) != std::cmp::Ordering::Less {
            r = r.sub(&den);
            q.set(i, true);
        }
    }
    (q, r.extend(d, false))
}

fn neg(x: &BitVec) -> BitVec {
    BitVec::new(x.len()).sub(x)
}

/// `/`, `rem` and `mod` of `numeric_std` vectors (`KIND` 0, 1, 2), as the bodies compute them
/// through `DIVMOD` on magnitudes. A zero divisor is the body's.
fn divide<const KIND: u8, const SIGNED: bool>(
    m: &mut Machine,
    _: Cx<'_, '_, '_>,
    _: Option<TyId>,
    a: &[u64],
) -> Ret {
    let (l, r) = (vh(m, a, 0)?.v, vh(m, a, 1)?.v);
    if l.is_empty() || r.is_empty() {
        return put(m, &Vh::null());
    }
    let (Some(x), Some(y)) = (l.to_bitvec(), r.to_bitvec()) else {
        let n = if KIND == 0 { l.len() } else { r.len() };
        return put(m, &Vh::down(x_vec(n)));
    };
    let (xneg, yneg) = (SIGNED && x.get(x.len() - 1), SIGNED && y.get(y.len() - 1));
    let (xm, ym) = (
        if xneg { neg(&x) } else { x },
        if yneg { neg(&y) } else { y },
    );
    if ym.words().iter().all(|w| *w == 0) {
        return fallback(m);
    }
    let (q, mut rm) = divmod(&xm, &ym);
    let out = match KIND {
        0 if xneg != yneg => neg(&q),
        0 => q,
        1 => {
            if xneg {
                rm = neg(&rm);
            }
            rm
        }
        _ => {
            let nonzero = rm.words().iter().any(|w| *w != 0);
            if yneg && xneg {
                neg(&rm)
            } else if yneg && nonzero {
                rm.sub(&ym)
            } else if xneg && nonzero {
                ym.sub(&rm)
            } else {
                rm
            }
        }
    };
    put(m, &Vh::down(LogicVec::from_bitvec(&out)))
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
    t: Option<TyId>,
    a: &[u64],
) -> Ret {
    let bit = t.is_some();
    let c = CMPS[C as usize];
    let dflt = c == Cmp::Ne;
    let bad = |m: &mut Machine, ctx: Cx<'_, '_, '_>, what: &str| -> Ret {
        warnb(
            m,
            ctx,
            &format!(
                "NUMERIC_STD.\"{}\": {what} detected, returning {}",
                cmp_name(c),
                if dflt { "TRUE" } else { "FALSE" }
            ),
            bit,
        )?;
        Ok(Some(u64::from(dflt)))
    };
    // Operands of at most 64 elements without a metavalue: plain integers.
    let ord = match FORM {
        0 => match (small(m, a, 0)?, small(m, a, 1)?) {
            (Some(l), Some(r)) if SIGNED => {
                Some(signed_of(l.bits, l.n).cmp(&signed_of(r.bits, r.n)))
            }
            (Some(l), Some(r)) => Some(l.bits.cmp(&r.bits)),
            _ => None,
        },
        _ => match small(m, a, usize::from(FORM != 1))? {
            Some(v) => {
                let i = int(a, usize::from(FORM == 1))?;
                let i = if SIGNED { i } else { natural(m, i)? };
                let o = if SIGNED {
                    signed_of(v.bits, v.n).cmp(&i)
                } else {
                    v.bits.cmp(&(i as u64))
                };
                Some(if FORM == 1 { o } else { o.reverse() })
            }
            None => None,
        },
    };
    if let Some(o) = ord {
        return Ok(Some(u64::from(holds(c, o))));
    }
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
        lb.cmp_ext(&rb, SIGNED)
    } else if FORM == 1 {
        let i = int(a, 1)?;
        let i = if SIGNED { i } else { natural(m, i)? };
        vec_vs_int(&lb, SIGNED, i)
    } else {
        let i = int(a, 0)?;
        let i = if SIGNED { i } else { natural(m, i)? };
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
    t: Option<TyId>,
    a: &[u64],
) -> Ret {
    let bit = t.is_some();
    if let Some(v) = small(m, a, 0)? {
        let neg = SIGNED && v.bits >> (v.n - 1) & 1 == 1;
        if SIGNED && v.n == 1 && neg {
            // The body negates `XARG + 1`, whose `TO_SIGNED (1, 1)` truncates.
            warnb(m, ctx, "NUMERIC_STD.TO_SIGNED: vector truncated", bit)?;
        }
        let mag = if neg { !v.bits & low(v.n) } else { v.bits };
        // Beyond INTEGER'HIGH the body's checked addition fails: the general path words it.
        if mag <= m.rt.revision.integer_bounds().high as u64 {
            return Ok(Some(if neg { (-(mag as i64) - 1) as u64 } else { mag }));
        }
    }
    let l = vh(m, a, 0)?.v;
    if l.is_empty() {
        warnb(
            m,
            ctx,
            "NUMERIC_STD.TO_INTEGER: null detected, returning 0",
            bit,
        )?;
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
        warnb(m, ctx, "NUMERIC_STD.TO_SIGNED: vector truncated", bit)?;
    }
    // The body accumulates the magnitude (`-(x + 1)`, that is `not x`, for a negative signed
    // value) most significant bit first with `R := R + R`, a checked INTEGER addition.
    let neg = SIGNED && b.get(b.len() - 1);
    let mag = if neg { b.not() } else { b };
    let hi = m.rt.revision.integer_bounds().high as u64;
    let mut r: u64 = 0;
    for i in (0..mag.len()).rev() {
        if r > hi / 2 {
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
    t: Option<TyId>,
    a: &[u64],
) -> Ret {
    let bit = t.is_some();
    let (v, size) = (int(a, 0)?, int(a, 1)?);
    let v = if SIGNED { v } else { natural(m, v)? };
    let size = natural(m, size)?;
    if (1..=64).contains(&size) {
        let bits = int_bits(m, ctx, v, size as usize, SIGNED, bit)?;
        return put_down(m, size as usize, bits).map(Some);
    }
    let o = from_int(m, ctx, v, size, SIGNED, bit)?;
    put(m, &o)
}

fn resize<const SIGNED: bool>(
    m: &mut Machine,
    _: Cx<'_, '_, '_>,
    _: Option<TyId>,
    a: &[u64],
) -> Ret {
    if let Some(v) = small(m, a, 0)?.filter(|v| v.pure) {
        let new = natural(m, int(a, 1)?)? as usize;
        if (1..=64).contains(&new) {
            let bits = if SIGNED {
                // The top element is the argument's sign element; below it the low elements.
                let keep = v.n.min(new) - 1;
                let sign = v.bits >> (v.n - 1) & 1 == 1;
                let kept = if keep == 0 { 0 } else { v.bits & low(keep) };
                kept | if sign { low(new) & !low(keep) } else { 0 }
            } else {
                v.bits & low(v.n.min(new))
            };
            return put_down(m, new, bits).map(Some);
        }
    }
    let l = vh(m, a, 0)?.v;
    let new = natural(m, int(a, 1)?)? as usize;
    if new < 1 {
        return put(m, &Vh::null());
    }
    let len = l.len();
    let v = if len == 0 {
        LogicVec::new(new, StdULogic::Zero)
    } else if SIGNED {
        // The top element is the argument's sign element; below it the low elements.
        let keep = len.min(new) - 1;
        LogicVec::new(new - keep, l.get(len - 1)).concat(&l.slice(0, keep))
    } else if len >= new {
        l.slice(0, new)
    } else {
        LogicVec::new(new - len, StdULogic::Zero).concat(&l)
    };
    put(m, &Vh::down(v))
}

/// `shift_of` on at most 64 elements without a metavalue, in one word; `None` when the
/// general path is needed (and, for a rotation by an empty array, its error).
fn shift_small<const K: u8>(m: &mut Machine, v: &Small, count: usize) -> R<Option<u64>> {
    let n = v.n;
    let bits = match K {
        0 if count >= n => 0,
        0 => v.bits << count,
        1 if count >= n => 0,
        1 => v.bits >> count,
        2 => {
            // XSRA returns its argument unchanged (its own bounds) for 0 or 1 elements.
            if n <= 1 || count == 0 {
                return put_small(m, n, v.bits, v.left, v.right, v.down).map(Some);
            }
            let k = count.min(n - 1);
            ((signed_of(v.bits, n) >> k) as u64) & low(n)
        }
        3 => {
            let k = count % n;
            if k == 0 {
                v.bits
            } else {
                v.bits << k | v.bits >> (n - k)
            }
        }
        _ => {
            let k = count % n;
            if k == 0 {
                v.bits
            } else {
                v.bits >> k | v.bits << (n - k)
            }
        }
    };
    put_down(m, n, bits).map(Some)
}

fn shift<const K: u8>(m: &mut Machine, _: Cx<'_, '_, '_>, _: Option<TyId>, a: &[u64]) -> Ret {
    if let Some(v) = small(m, a, 0)?.filter(|v| v.pure) {
        let count = natural(m, int(a, 1)?)? as usize;
        return shift_small::<K>(m, &v, count);
    }
    let l = vh(m, a, 0)?;
    let count = natural(m, int(a, 1)?)? as usize;
    shift_of::<K>(m, l, count)
}

/// `shift_left` ... `rotate_right`: the null array constant for a null argument, else the
/// local function (`XSLL` ...) the body calls.
fn shift_fn<const K: u8>(m: &mut Machine, _: Cx<'_, '_, '_>, _: Option<TyId>, a: &[u64]) -> Ret {
    if let Some(v) = small(m, a, 0)?.filter(|v| v.pure) {
        let count = natural(m, int(a, 1)?)? as usize;
        return shift_small::<K>(m, &v, count);
    }
    let l = vh(m, a, 0)?;
    let count = natural(m, int(a, 1)?)? as usize;
    if l.v.is_empty() {
        return put(m, &Vh::null());
    }
    shift_of::<K>(m, l, count)
}

fn shift_of<const K: u8>(m: &mut Machine, l: Vh, count: usize) -> Ret {
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
        Op::NumGate(g) => [
            num_gate::<0> as NativeFn,
            num_gate::<1>,
            num_gate::<2>,
            num_gate::<3>,
            num_gate::<4>,
            num_gate::<5>,
        ][GATES.iter().position(|x| *x == g)?],
        Op::Reduce(g) => [
            reduce::<0> as NativeFn,
            reduce::<1>,
            reduce::<2>,
            reduce::<3>,
            reduce::<4>,
            reduce::<5>,
        ][GATES.iter().position(|x| *x == g)?],
        Op::NumNot => num_not,
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
        Op::IsX(true) => is_x_vec,
        Op::IsX(false) => is_x_sul,
        Op::Edge(true) => edge::<true>,
        Op::Edge(false) => edge::<false>,
        Op::Add {
            sub,
            signed,
            form: f,
        } => pick3!(add, sub, signed, form(f)),
        Op::Div { kind, signed } => match (kind, signed) {
            (0, false) => divide::<0, false>,
            (0, true) => divide::<0, true>,
            (1, false) => divide::<1, false>,
            (1, true) => divide::<1, true>,
            (_, false) => divide::<2, false>,
            (_, true) => divide::<2, true>,
        },
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
        Op::Syn(Syn::Less { or_equal, signed }) => match (or_equal, signed) {
            (false, false) => syn_less::<false, false>,
            (false, true) => syn_less::<false, true>,
            (true, false) => syn_less::<true, false>,
            (true, true) => syn_less::<true, true>,
        },
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
        Op::Uniform => uniform,
        Op::Str(false) => to_str::<false>,
        Op::Str(true) => to_str::<true>,
        Op::ShiftFn(Shift::Sll) => shift_fn::<0>,
        Op::ShiftFn(Shift::Srl) => shift_fn::<1>,
        Op::ShiftFn(Shift::Sra) => shift_fn::<2>,
        Op::ShiftFn(Shift::Rol) => shift_fn::<3>,
        Op::ShiftFn(Shift::Ror) => shift_fn::<4>,
        Op::Shift(Shift::Sll) => shift::<0>,
        Op::Shift(Shift::Srl) => shift::<1>,
        Op::Shift(Shift::Sra) => shift::<2>,
        Op::Shift(Shift::Rol) => shift::<3>,
        Op::Shift(Shift::Ror) => shift::<4>,
    })
}
