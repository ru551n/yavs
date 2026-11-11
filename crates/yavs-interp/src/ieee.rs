//! Native `ieee` subprograms (`yavs_ir::ieee`): the hot operators of `std_logic_1164` and
//! `numeric_std`, run on bit-plane vectors instead of their VHDL bodies.
//!
//! Each function is a port of the body it replaces: same result bounds, same handling of
//! metavalues, same warning texts and severities. `lib/ieee/*/` is the reference; the
//! differential test (`tests/ieee_native.rs`) runs both on the same operands. Calling
//! convention: array arguments are four cells (`data, left, right, dir`), scalars one (#697).

use crate::ieee_wide::{self as wide, Bounds};
use yavs_ir::ieee::{ARRAY_REGS, Cmp, Form, Gate, Math, Op, Opd, Shift, Syn, SynKind};
use yavs_ir::{DOPE_DIM_BYTES, TyId, Width, dim_len};
use yavs_kernel::{Ctx, LogicVec, Severity};
use yavs_logic::{BitVec, StdULogic};

use crate::exec::{Fault, Machine, R, add_addr, cold, ice, ice_f, rt_err, split};
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

/// Cell of the second array operand: an array argument is [`ARRAY_REGS`] cells, `data, left,
/// right, dir` (`yavs_ir::ieee`); this is where the next argument after one array starts.
const A1: usize = ARRAY_REGS;

/// The data address, bounds and direction of the array operand at cell `i`.
#[inline(always)]
fn dim_at(a: &[u64], i: usize) -> R<(u64, i64, i64, bool)> {
    match a.get(i..i + ARRAY_REGS) {
        Some(&[data, left, right, d]) => Ok((data, left as i64, right as i64, d == 1)),
        _ => Err(ice(format!("native array argument at {i} missing"))),
    }
}

#[inline(always)]
fn le8(b: &[u8]) -> u64 {
    u64::from_le_bytes(b.try_into().unwrap_or([0; 8]))
}

fn vh(m: &Machine, a: &[u64], i: usize) -> R<Vh> {
    let (data, left, right, down) = dim_at(a, i)?;
    let n = dim_len(left, right, down).ok_or_else(|| Fault::Runtime("array too long".into()))?;
    Ok(Vh {
        v: m.planes(data, n)?,
        left,
        right,
        down,
    })
}

fn put(m: &mut Machine, o: &Vh) -> Ret {
    put_blocks(m, o.v.repr().blocks(), o.left, o.right, o.down)
}

/// The result with these plane blocks and bounds: the fat pair, the dope vector and the planes in
/// one block of temporary memory.
fn put_blocks(m: &mut Machine, blocks: &[[u64; 4]], left: i64, right: i64, down: bool) -> Ret {
    let size = blocks.len() as u64 * 32;
    let fat = m.temp_alloc(16 + DOPE_DIM_BYTES + size)?;
    let (dope, data) = (add_addr(fat, 16)?, add_addr(fat, 16 + DOPE_DIM_BYTES)?);
    let b = m.wr(fat, 16 + DOPE_DIM_BYTES + size)?;
    let head = [data, dope, left as u64, right as u64, u64::from(down)];
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

/// The low `n` bits for `n` from 1 to 64, without a branch.
#[inline(always)]
fn low1(n: usize) -> u64 {
    u64::MAX >> (64 - n)
}

/// The low `n` bits (`n` from 0 to 64 and above).
pub(crate) fn low(n: usize) -> u64 {
    if n >= 64 { !0 } else { (1 << n) - 1 }
}

/// The four words (one block of planes) at `addr`: the common spaces inline, anything else (a
/// frame of a compiled native frame, package data) through [`words_slow`]; `None` for an
/// unreadable address (the general path then reads again and words the error).
#[inline(always)]
fn block4(m: &Machine, addr: u64) -> Option<[u64; 4]> {
    let (sp, idx, off) = split(addr);
    let Some(b) = m
        .mem_temp_first(sp, idx)
        .and_then(|b| b.get(off as usize..off as usize + 32))
    else {
        return words_slow(m, addr);
    };
    let (w, _) = b.as_chunks::<8>();
    Some([
        u64::from_le_bytes(w[0]),
        u64::from_le_bytes(w[1]),
        u64::from_le_bytes(w[2]),
        u64::from_le_bytes(w[3]),
    ])
}

/// `N` words at `addr` (`block4` for one block of planes).
#[inline(always)]
fn words<const N: usize>(m: &Machine, addr: u64) -> Option<[u64; N]> {
    let (sp, idx, off) = split(addr);
    let Some(b) = m
        .mem_temp_first(sp, idx)
        .and_then(|b| b.get(off as usize..off as usize + N * 8))
    else {
        return words_slow(m, addr);
    };
    Some(std::array::from_fn(|k| le8(&b[k * 8..k * 8 + 8])))
}

#[cold]
#[inline(never)]
fn words_slow<const N: usize>(m: &Machine, addr: u64) -> Option<[u64; N]> {
    let b = m.rd(addr, N as u64 * 8).ok()?;
    Some(std::array::from_fn(|k| le8(&b[k * 8..k * 8 + 8])))
}

/// [`Small`] of the array operand at cell `i`, or `None` for anything else (empty, longer, a
/// metavalue, an unreadable operand), which the general path handles.
#[inline(always)]
pub(crate) fn small(m: &Machine, a: &[u64], i: usize) -> Option<Small> {
    let &[data, left, right, d] = a.get(i..i + ARRAY_REGS)? else {
        return None;
    };
    small_of(m, data, left as i64, right as i64, d == 1)
}

/// [`small`] of the array operand `i` of a native that takes arrays as fat pairs.
pub(crate) fn small_fat(m: &Machine, a: &[u64], i: usize) -> Option<Small> {
    let [data, dope] = words::<2>(m, *a.get(i)?)?;
    let [left, right, d] = words::<3>(m, dope)?;
    small_of(m, data, left as i64, right as i64, d == 1)
}

#[inline(always)]
fn small_of(m: &Machine, data: u64, left: i64, right: i64, down: bool) -> Option<Small> {
    let (lo, hi) = if down { (right, left) } else { (left, right) };
    let n = usize::try_from(hi.checked_sub(lo)?).ok()?.checked_add(1)?;
    if n > 64 {
        return None;
    }
    let [p0, p1, p2, p3] = block4(m, data)?;
    // `0` or `L`: plane 1 only (plane 2 is the weak bit); `1` or `H`: planes 0 and 1.
    let mask = low1(n);
    if !p3 & p1 & mask != mask {
        return None;
    }
    Some(Small {
        n,
        bits: p0 & mask,
        pure: p2 & mask == 0,
        left,
        right,
        down,
    })
}

/// The result `bits` of `n` elements, bounds `left`, `right`, `down`, as [`put`] writes it.
#[inline(always)]
pub(crate) fn put_small(
    m: &mut Machine,
    n: usize,
    bits: u64,
    left: i64,
    right: i64,
    down: bool,
) -> R<u64> {
    let mask = low1(n);
    // The fat pair, the dope vector and one block of planes; `fat` is the address of the block.
    m.temp_words(|fat| {
        [
            fat + 16 + DOPE_DIM_BYTES,
            fat + 16,
            left as u64,
            right as u64,
            u64::from(down),
            bits & mask,
            mask,
            0,
            0,
        ]
    })
}

/// [`put_small`] with the bounds `(n-1 downto 0)` ([`Vh::down`]).
#[inline(always)]
pub(crate) fn put_down(m: &mut Machine, n: usize, bits: u64) -> R<u64> {
    put_small(m, n, bits, n as i64 - 1, 0, true)
}

/// `bits` of `n` elements extended to `to` elements (`to >= n`): sign or zero.
fn extend_bits(bits: u64, n: usize, to: usize, signed: bool) -> u64 {
    if signed && n < to && bits >> (n - 1) & 1 == 1 {
        (bits | !low1(n)) & low1(to)
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
    if let Some(p) = m.pm.as_mut() {
        p.reports += 1;
    }
    if s == Severity::Warning && crate::ieee_warnings_off() {
        return Ok(());
    }
    match ctx.as_deref_mut() {
        Some(c) => {
            let Some(text) = m.rt.asserts.report(c, s, text.to_string()) else {
                return Ok(());
            };
            m.rt.asserts
                .note_site(|| m.trace().first().copied(), s, &text);
            let shown = m.has_debugger().then(|| text.clone());
            c.report(s, text);
            let stops = s >= c.stop_severity();
            if let Some(text) = shown {
                m.assert_stop(s, &text, ctx);
            }
            if stops {
                m.ieee_stop = true;
            }
        }
        None => {
            if m.init_report_seen(s, text) || !m.rt.asserts.report_elab(s) {
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
    if let Some(r) = fast_vec_gate::<G>(m, a) {
        return Ok(Some(r));
    }
    general(m);
    let (l, r) = (vh(m, a, 0)?, vh(m, a, A1)?);
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
    if let Some(r) = fast_num_gate::<G>(m, a) {
        return Ok(Some(r));
    }
    general(m);
    let (l, r) = (vh(m, a, 0)?, vh(m, a, A1)?);
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
    if let Some(r) = fast_num_not(m, a) {
        return Ok(Some(r));
    }
    general(m);
    let l = vh(m, a, 0)?;
    put(m, &Vh::down(l.v.not()))
}

/// Unary `-` of a `signed`: null is `NAS`, a metavalue gives all `X` (the body's `to_01 (arg, 'X')`),
/// neither warns; the most negative value wraps onto itself.
fn neg_signed(m: &mut Machine, _: Cx<'_, '_, '_>, _: Option<TyId>, a: &[u64]) -> Ret {
    if let Some(r) = fast_neg(m, a) {
        return Ok(Some(r));
    }
    general(m);
    let l = vh(m, a, 0)?;
    if l.v.is_empty() {
        return put(m, &Vh::null());
    }
    let v = match l.v.to_bitvec() {
        Some(b) => LogicVec::from_bitvec(&neg(&b)),
        None => x_vec(l.v.len()),
    };
    put(m, &Vh::down(v))
}

/// `abs` of a `signed`: as unary `-` for a negative operand, the operand through `to_01` otherwise;
/// null is `NAS`, a metavalue gives all `X`, neither warns.
fn abs_signed(m: &mut Machine, _: Cx<'_, '_, '_>, _: Option<TyId>, a: &[u64]) -> Ret {
    if let Some(r) = fast_abs(m, a) {
        return Ok(Some(r));
    }
    general(m);
    let l = vh(m, a, 0)?;
    if l.v.is_empty() {
        return put(m, &Vh::null());
    }
    let v = match l.v.to_bitvec() {
        Some(b) if b.get(b.len() - 1) => LogicVec::from_bitvec(&neg(&b)),
        Some(b) => LogicVec::from_bitvec(&b),
        None => x_vec(l.v.len()),
    };
    put(m, &Vh::down(v))
}

fn not(_: &mut Machine, _: Cx<'_, '_, '_>, _: Option<TyId>, a: &[u64]) -> Ret {
    Ok(Some(sul(a, 0)?.not() as u64))
}

fn vec_not(m: &mut Machine, _: Cx<'_, '_, '_>, _: Option<TyId>, a: &[u64]) -> Ret {
    if let Some(r) = fast_vec_not(m, a) {
        return Ok(Some(r));
    }
    general(m);
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
        if let Some(r) = conv_small::<K>(m, a) {
            return Ok(Some(r));
        }
        general(m);
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
        Op::IsX(false) => all().map(|x| u8::from(x.is_x())).collect(),
        Op::MatchEq { ne, vec: false } => all()
            .flat_map(|x| {
                all().map(move |y| {
                    let c = x.match_eq(y);
                    (if ne { c.not() } else { c }) as u8
                })
            })
            .collect(),
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
    if let Some(r) = fast_to_01(m, a) {
        return Ok(Some(r));
    }
    general(m);
    let l = vh(m, a, 0)?;
    let x = sul(a, A1)?;
    put(m, &Vh::down(to_01(&l.v, x)))
}

/// `is_x` of a vector: any element not `0 1 L H`. A null vector has none.
/// A gate over all elements of a vector (`Op::Reduce`), on the plane words: `and` is `0` if
/// any element is `0`/`L`, else `U` if any is `U`, else `X` if any is not `1`/`H`, else `1`;
/// `or` the same with `1` and `0` swapped; `xor` `U` if any is `U`, else `X` if any is not
/// `0 1 L H`, else the parity of the ones. The bodies fold the same tables element by element.
fn reduce<const G: u8>(m: &mut Machine, _: Cx<'_, '_, '_>, _: Option<TyId>, a: &[u64]) -> Ret {
    let (data, left, right, down) = dim_at(a, 0)?;
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
    let (data, left, right, down) = dim_at(a, 0)?;
    let n = dim_len(left, right, down).ok_or_else(|| Fault::Runtime("array too long".into()))?;
    // The planes are tested where they are: no vector is made for a test of the elements (the
    // codes with plane 1 set and plane 3 clear are the `0 1 L H`, as `yavs_logic::wide::has_x`).
    let bytes = m.rd(data, n.div_ceil(64) * 32)?;
    let mut any = 0u64;
    for (i, c) in bytes.as_chunks::<32>().0.iter().enumerate() {
        any |= (!le8(&c[8..16]) | le8(&c[24..32])) & yavs_logic::wide::nmask(n as usize - i * 64);
    }
    Ok(Some(u64::from(any != 0)))
}

/// `to_bitvector (s, xmap)`: `'0'` for `0`/`L`, `'1'` for `1`/`H` and `xmap` for any other element,
/// as the result `(n - 1 downto 0)`. A `bit` element has the plane code of `'0'` or `'1'`.
fn to_bitvector(m: &mut Machine, _: Cx<'_, '_, '_>, _: Option<TyId>, a: &[u64]) -> Ret {
    if let Some(r) = to_bitvector_small(m, a) {
        return Ok(Some(r));
    }
    let (data, left, right, down) = dim_at(a, 0)?;
    let n = dim_len(left, right, down).ok_or_else(|| Fault::Runtime("array too long".into()))?;
    let n = usize::try_from(n).map_err(|_| Fault::Runtime("array too long".into()))?;
    let xmap = if arg(a, A1)? != 0 { !0u64 } else { 0 };
    let mut out = Vec::with_capacity(n.div_ceil(64));
    if n > 0 {
        let b = m.rd(data, n.div_ceil(64) as u64 * 32)?;
        for (i, w) in b.as_chunks::<32>().0.iter().enumerate() {
            let (p0, p1, p3) = (le8(&w[0..8]), le8(&w[8..16]), le8(&w[24..32]));
            // `0 L` and `1 H` are the codes with plane 1 set and plane 3 clear; plane 0 tells which.
            let known = p1 & !p3;
            let valid = low(n - i * 64);
            out.push([(p0 & known | !known & xmap) & valid, valid, 0, 0]);
        }
    }
    put_blocks(m, &out, n as i64 - 1, 0, true)
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
    general(m);
    Ok(Some(0))
}

/// The native runs its general path: its one-word and wide paths declined, or it gives the
/// call back to the body. Counted per native under `YAVS_COUNTERS` (`ieee_general.<key>`); the
/// callers take the flag with `ieee_fallback`.
#[inline]
fn general(m: &mut Machine) {
    if crate::census::ieee_on() {
        m.ieee_general = true;
    }
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
    let (l, r) = (vh(m, a, 0)?, vh(m, a, A1)?);
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
    let (l, r) = (vh(m, a, 0)?, vh(m, a, A1)?);
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
    let (l, r) = (vh(m, a, 0)?, vh(m, a, A1)?);
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
    if let Some(v) = fast_syn_conv_vec::<FROM_SIGNED>(m, a) {
        return Ok(Some(v));
    }
    general(m);
    let l = vh(m, a, 0)?;
    let size = int(a, A1)?;
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
    if let Some(v) = fast_syn_conv_int::<SIGNED>(m, a) {
        return Ok(Some(v));
    }
    general(m);
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

/// One step of the L'Ecuyer generator of `math_real.uniform`, in `i64` (every intermediate within
/// INTEGER for the seeds the two range checks admit): the new seeds and the bits of the real.
#[inline(always)]
pub(crate) fn uniform_step(s1: i64, s2: i64) -> (i64, i64, u64) {
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
    (t1, t2, (z as f64 * 4.656613e-10).to_bits())
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
    let (t1, t2, x) = uniform_step(s1, s2);
    m.store(pa, Width::B8, t1 as u64)?;
    m.store(pb, Width::B8, t2 as u64)?;
    m.store(px, Width::B8, x)?;
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
            let (Some(l), Some(r)) = (small(m, a, 0), small(m, a, A1)) else {
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
            let Some(l) = small(m, a, 0) else {
                return Ok(None);
            };
            let i = int(a, A1)?;
            let i = if SIGNED { i } else { natural(m, i)? };
            (l.n, l.bits, int_bits(m, ctx, i, l.n, SIGNED, bit)?)
        }
        _ => {
            let Some(r) = small(m, a, 1) else {
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
    if let Some(r) = fast_add::<SUB, SIGNED, FORM>(m, a) {
        return Ok(Some(r));
    }
    if let Some(r) = add_small::<SUB, SIGNED, FORM>(m, ctx, bit, a)? {
        return Ok(Some(r));
    }
    general(m);
    let out = match FORM {
        0 => add_vv(&vh(m, a, 0)?.v, &vh(m, a, A1)?.v, SUB, SIGNED),
        1 => {
            let l = vh(m, a, 0)?;
            let i = int(a, A1)?;
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

/// Element `i` of `RESIZE (v, size)`: zeros, or the sign element of a `signed` operand, above `v`.
fn resized(v: &LogicVec, i: usize, signed: bool) -> StdULogic {
    match v.len() {
        n if i < n => v.get(i),
        n if signed => v.get(n - 1),
        _ => StdULogic::Zero,
    }
}

/// The matching relational operators: the one-word path or wide compare, else a port of the
/// bodies (every report included). An integer is `to_unsigned`/`to_signed` at the vector's length.
fn qcompare<const C: u8, const SIGNED: bool, const FORM: u8>(
    m: &mut Machine,
    ctx: Cx<'_, '_, '_>,
    _: Option<TyId>,
    a: &[u64],
) -> Ret {
    if let Some(r) = fast_qcompare::<C, SIGNED, FORM>(m, a) {
        return Ok(Some(r));
    }
    general(m);
    let c = CMPS[C as usize];
    let name = format!("?{}", cmp_name(c));
    let (l, r) = match FORM {
        0 => (vh(m, a, 0)?.v, vh(m, a, A1)?.v),
        1 => {
            let l = vh(m, a, 0)?;
            let i = int(a, A1)?;
            let i = if SIGNED { i } else { natural(m, i)? };
            let r = from_int(m, ctx, i, l.v.len() as i64, SIGNED, false)?;
            (l.v, r.v)
        }
        _ => {
            let r = vh(m, a, 1)?;
            let i = int(a, 0)?;
            let i = if SIGNED { i } else { natural(m, i)? };
            let l = from_int(m, ctx, i, r.v.len() as i64, SIGNED, false)?;
            (l.v, r.v)
        }
    };
    let x = StdULogic::X as u64;
    if l.is_empty() || r.is_empty() {
        warn(
            m,
            ctx,
            &format!("NUMERIC_STD.\"{name}\": null detected, returning X"),
        )?;
        return Ok(Some(x));
    }
    if C >= 4 {
        // The body's fold: a `U` ends it, an `X` stays, else `and` (`or` for `?/=`) of 0 and 1.
        let ne = C == 5;
        let size = l.len().max(r.len());
        let mut result = if ne { StdULogic::Zero } else { StdULogic::One };
        for i in 0..size {
            let e = resized(&l, i, SIGNED).match_eq(resized(&r, i, SIGNED));
            let e = if ne { e.not() } else { e };
            if e == StdULogic::U {
                return Ok(Some(StdULogic::U as u64));
            } else if e == StdULogic::X || result == StdULogic::X {
                result = StdULogic::X;
            } else if ne {
                result = result.or(e);
            } else {
                result = result.and(e);
            }
        }
        return Ok(Some(result as u64));
    }
    let dash = |v: &LogicVec| (0..v.len()).any(|i| v.get(i) == StdULogic::DontCare);
    if dash(&l) || dash(&r) {
        report(
            m,
            ctx,
            Severity::Error,
            &format!("NUMERIC_STD.\"{name}\": '-' found in compare string"),
        )?;
        return Ok(Some(x));
    }
    let (Some(lb), Some(rb)) = (l.to_bitvec(), r.to_bitvec()) else {
        return Ok(Some(x));
    };
    Ok(Some(2 + u64::from(holds(c, lb.cmp_ext(&rb, SIGNED)))))
}

/// `maximum` / `minimum` (the body's `to_01 (resize (x), 'X')` of both operands, so a metavalue
/// gives all `X`, a null array the null constant, neither with a report): the one-word or wide
/// path, else this port; an integer is `to_unsigned`/`to_signed` at the vector's length.
fn minmax<const MAX: bool, const SIGNED: bool, const FORM: u8>(
    m: &mut Machine,
    ctx: Cx<'_, '_, '_>,
    _: Option<TyId>,
    a: &[u64],
) -> Ret {
    if let Some(r) = fast_minmax::<MAX, SIGNED, FORM>(m, a) {
        return Ok(Some(r));
    }
    general(m);
    let (l, r) = match FORM {
        0 => (vh(m, a, 0)?.v, vh(m, a, A1)?.v),
        1 => {
            let l = vh(m, a, 0)?;
            let i = int(a, A1)?;
            let i = if SIGNED { i } else { natural(m, i)? };
            (from_int(m, ctx, i, l.v.len() as i64, SIGNED, false)?.v, l.v)
        }
        _ => {
            let r = vh(m, a, 1)?;
            let i = int(a, 0)?;
            let i = if SIGNED { i } else { natural(m, i)? };
            (from_int(m, ctx, i, r.v.len() as i64, SIGNED, false)?.v, r.v)
        }
    };
    // (Forms 1 and 2 put the converted integer first; the pick below is symmetric.)
    if l.is_empty() || r.is_empty() {
        return put(m, &Vh::null());
    }
    let size = l.len().max(r.len());
    let (Some(x), Some(y)) = (l.to_bitvec(), r.to_bitvec()) else {
        return put(m, &Vh::down(x_vec(size)));
    };
    let (x, y) = (zero_ext(&x, size, SIGNED), zero_ext(&y, size, SIGNED));
    let less = x.cmp_ext(&y, SIGNED).is_lt();
    let v = LogicVec::from_bitvec(if less == MAX { &y } else { &x });
    put(m, &Vh::down(v))
}

/// `std_match` of two vectors: the one-word or wide path, else a port of the body: a null array
/// and unequal lengths warn and give `false`.
fn std_match(m: &mut Machine, ctx: Cx<'_, '_, '_>, _: Option<TyId>, a: &[u64]) -> Ret {
    if let Some(r) = fast_match(m, a) {
        return Ok(Some(r));
    }
    general(m);
    let (l, r) = (vh(m, a, 0)?.v, vh(m, a, A1)?.v);
    if l.is_empty() || r.is_empty() {
        warn(
            m,
            ctx,
            "NUMERIC_STD.STD_MATCH: null detected, returning FALSE",
        )?;
        return Ok(Some(0));
    }
    if l.len() != r.len() {
        warn(
            m,
            ctx,
            "NUMERIC_STD.STD_MATCH: L'LENGTH /= R'LENGTH, returning FALSE",
        )?;
        return Ok(Some(0));
    }
    // MATCH_TABLE: `-` matches anything, else both must be `0 1 L H` with the same value.
    let known = |e: StdULogic| {
        matches!(
            e,
            StdULogic::Zero | StdULogic::One | StdULogic::L | StdULogic::H
        )
    };
    let one = |e: StdULogic| matches!(e, StdULogic::One | StdULogic::H);
    let ok = (0..l.len()).all(|i| {
        let (x, y) = (l.get(i), r.get(i));
        x == StdULogic::DontCare
            || y == StdULogic::DontCare
            || known(x) && known(y) && one(x) == one(y)
    });
    Ok(Some(u64::from(ok)))
}

/// The predefined `?=` (`NE` false) and `?/=` of two `std_ulogic` or two `std_ulogic_vector`
/// (`VEC`, LRM 9.2.3): the AND of the element matches; unequal lengths are an error.
fn match_eq<const NE: bool, const VEC: bool>(
    m: &mut Machine,
    _: Cx<'_, '_, '_>,
    _: Option<TyId>,
    a: &[u64],
) -> Ret {
    if let Some(r) = fast_match_eq::<NE, VEC>(m, a) {
        return Ok(Some(r));
    }
    general(m);
    let c = if VEC {
        let (l, r) = (vh(m, a, 0)?.v, vh(m, a, A1)?.v);
        if l.len() != r.len() {
            return crate::implicit::match_lengths(NE, l.len(), r.len());
        }
        l.match_eq(&r)
    } else {
        sul(a, 0)?.match_eq(sul(a, 1)?)
    };
    Ok(Some((if NE { c.not() } else { c }) as u64))
}

fn mul<const SIGNED: bool, const FORM: u8>(
    m: &mut Machine,
    ctx: Cx<'_, '_, '_>,
    t: Option<TyId>,
    a: &[u64],
) -> Ret {
    if let Some(r) = fast_mul::<SIGNED, FORM>(m, a) {
        return Ok(Some(r));
    }
    general(m);
    // An integer operand is `TO_UNSIGNED`/`TO_SIGNED` of the vector's length, as the body does.
    let (l, r) = match FORM {
        0 => (vh(m, a, 0)?.v, vh(m, a, A1)?.v),
        1 => {
            let l = vh(m, a, 0)?.v;
            let i = int(a, A1)?;
            let i = if SIGNED { i } else { natural(m, i)? };
            let r = from_int(m, ctx, i, l.len() as i64, SIGNED, t.is_some())?.v;
            (l, r)
        }
        _ => {
            let r = vh(m, a, 1)?.v;
            let i = int(a, 0)?;
            let i = if SIGNED { i } else { natural(m, i)? };
            let l = from_int(m, ctx, i, r.len() as i64, SIGNED, t.is_some())?.v;
            (l, r)
        }
    };
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

/// `UNSIGNED_NUM_BITS` / `SIGNED_NUM_BITS` of the bodies: the elements an integer needs.
fn num_bits(i: i64, signed: bool) -> usize {
    let n = if signed && i < 0 { !i as u64 } else { i as u64 };
    let len = 64 - n.leading_zeros() as usize;
    if signed { len + 1 } else { len.max(1) }
}

/// The `DIVMOD` of the vector bodies (`KIND` 0, 1, 2): magnitudes divided, signs applied as the
/// bodies do. `None` for a zero divisor, which is the body's error.
fn div_vh<const KIND: u8, const SIGNED: bool>(l: &LogicVec, r: &LogicVec) -> Option<Vh> {
    if l.is_empty() || r.is_empty() {
        return Some(Vh::null());
    }
    let (Some(x), Some(y)) = (l.to_bitvec(), r.to_bitvec()) else {
        let n = if KIND == 0 { l.len() } else { r.len() };
        return Some(Vh::down(x_vec(n)));
    };
    let (xneg, yneg) = (SIGNED && x.get(x.len() - 1), SIGNED && y.get(y.len() - 1));
    // The `mod` body tests the raw dividend's sign (`L(L'left) = '1'`), where `H` is not `1`.
    let lone = SIGNED && l.get(l.len() - 1) == StdULogic::One;
    let (xm, ym) = (
        if xneg { neg(&x) } else { x },
        if yneg { neg(&y) } else { y },
    );
    if ym.words().iter().all(|w| *w == 0) {
        return None;
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
            if yneg && lone {
                neg(&rm)
            } else if yneg && nonzero {
                rm.sub(&ym)
            } else if lone && nonzero {
                ym.sub(&rm)
            } else {
                rm
            }
        }
    };
    Some(Vh::down(LogicVec::from_bitvec(&out)))
}

/// `RESIZE (res, n)` of a result computed at a larger length, after the body's check that no
/// element is lost (`"/"`, `rem`, `mod` of an integer wider than the vector): the warning when
/// the elements above `n` are not zeros (copies of element `n - 1` for `signed`).
fn narrow<const KIND: u8, const SIGNED: bool>(
    m: &mut Machine,
    ctx: Cx<'_, '_, '_>,
    res: Vh,
    n: usize,
    bit: bool,
) -> R<Vh> {
    let v = res.v;
    let big = v.len();
    // A metavalue result is all `X`: no warning, and `X` at the new length.
    let Some(b) = v.to_bitvec() else {
        return Ok(Vh::down(x_vec(n)));
    };
    let top = SIGNED && b.get(n - 1);
    if (n..big).any(|j| b.get(j) != top) {
        let (op, what) =
            [("/", "Quotient"), ("rem", "Remainder"), ("mod", "Modulus")][KIND as usize];
        warnb(
            m,
            ctx,
            &format!("NUMERIC_STD.\"{op}\": {what} Truncated"),
            bit,
        )?;
    }
    // Signed: the sign element of the larger vector on top of the low elements.
    Ok(Vh::down(if SIGNED {
        LogicVec::new(1, v.get(big - 1)).concat(&v.slice(0, n - 1))
    } else {
        v.slice(0, n)
    }))
}

/// `/`, `rem` and `mod` of `numeric_std` vectors (`KIND` 0, 1, 2), as the bodies compute them
/// through `DIVMOD` on magnitudes. A zero divisor is the body's, and so is the length error of
/// `natural rem unsigned` with an integer wider than the vector.
fn divide<const KIND: u8, const SIGNED: bool, const FORM: u8>(
    m: &mut Machine,
    ctx: Cx<'_, '_, '_>,
    t: Option<TyId>,
    a: &[u64],
) -> Ret {
    if let Some(r) = fast_div::<KIND, SIGNED, FORM>(m, a) {
        return Ok(Some(r));
    }
    general(m);
    let bit = t.is_some();
    // An integer is `TO_UNSIGNED`/`TO_SIGNED` at the vector's length, or at the larger length it
    // needs, which the bodies then resize back (`narrow`).
    let mut narrow_to = None;
    let (l, r) = match FORM {
        0 => (vh(m, a, 0)?.v, vh(m, a, A1)?.v),
        _ => {
            let (v, i) = if FORM == 1 {
                (vh(m, a, 0)?.v, int(a, A1)?)
            } else {
                (vh(m, a, 1)?.v, int(a, 0)?)
            };
            let i = if SIGNED { i } else { natural(m, i)? };
            let n = v.len();
            if n == 0 {
                // NAU/NAS; `natural rem unsigned` has no such check and fails on the length.
                return if FORM == 2 && KIND == 1 && !SIGNED {
                    fallback(m)
                } else {
                    put(m, &Vh::null())
                };
            }
            let nb = num_bits(i, SIGNED);
            let w = if nb <= n {
                from_int(m, ctx, i, n as i64, SIGNED, bit)?.v
            } else {
                match (FORM, KIND) {
                    // The body returns zeros whatever the vector holds.
                    (1, 0) => return put(m, &Vh::down(LogicVec::new(n, StdULogic::Zero))),
                    (2, 1) if !SIGNED => return fallback(m),
                    _ => {}
                }
                narrow_to = (FORM == 1 || KIND == 0).then_some(n);
                from_int(m, ctx, i, nb as i64, SIGNED, bit)?.v
            };
            if FORM == 1 { (v, w) } else { (w, v) }
        }
    };
    let Some(res) = div_vh::<KIND, SIGNED>(&l, &r) else {
        return fallback(m);
    };
    let res = match narrow_to {
        Some(n) => narrow::<KIND, SIGNED>(m, ctx, res, n, bit)?,
        None => res,
    };
    put(m, &res)
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

pub(crate) fn holds(c: Cmp, o: std::cmp::Ordering) -> bool {
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
    if let Some(r) = fast_compare::<C, SIGNED, FORM>(m, a) {
        return Ok(Some(r));
    }
    general(m);
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
    let (l, r) = match FORM {
        0 => (vh(m, a, 0)?.v, Some(vh(m, a, A1)?.v)),
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
        let i = int(a, A1)?;
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
    if let Some(r) = fast_to_integer::<SIGNED>(m, a) {
        return Ok(Some(r));
    }
    if let Some(v) = small(m, a, 0) {
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
    general(m);
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
    if let Some(r) = fast_to_unsigned::<SIGNED>(m, a) {
        return Ok(Some(r));
    }
    general(m);
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
    if let Some(r) = fast_resize::<SIGNED>(m, a) {
        return Ok(Some(r));
    }
    general(m);
    let l = vh(m, a, 0)?.v;
    let new = natural(m, int(a, A1)?)? as usize;
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
    if let Some(r) = fast_shift::<K>(m, a) {
        return Ok(Some(r));
    }
    general(m);
    let l = vh(m, a, 0)?;
    let count = natural(m, int(a, A1)?)? as usize;
    shift_of::<K>(m, l, count)
}

/// `shift_left` ... `rotate_right`: the null array constant for a null argument, else the
/// local function (`XSLL` ...) the body calls.
fn shift_fn<const K: u8>(m: &mut Machine, _: Cx<'_, '_, '_>, _: Option<TyId>, a: &[u64]) -> Ret {
    if let Some(r) = fast_shift::<K>(m, a) {
        return Ok(Some(r));
    }
    general(m);
    let l = vh(m, a, 0)?;
    let count = natural(m, int(a, A1)?)? as usize;
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

// ---- the one-word paths without a report ----

/// The one-word path of a native (operands of 1 to 64 elements without a metavalue, no report,
/// no error), or the word-wise one for longer vectors (`ieee_wide`): the result, or `None` when
/// the native must run (its general path words every warning and error). `Op::CallIeee` tries
/// it first, without the bookkeeping of a native call, and so does each native.
pub(crate) type FastFn = fn(&mut Machine, &[u64]) -> Option<u64>;

/// The bits of `TO_UNSIGNED (arg, size)` / `TO_SIGNED` for `size` from 1 to 64 when `arg` fits.
#[inline(always)]
pub(crate) fn fit_bits(arg: i64, size: usize, signed: bool) -> Option<u64> {
    let fits = if signed {
        size >= 64 || (-(1i64 << (size - 1))..1i64 << (size - 1)).contains(&arg)
    } else {
        size >= 64 || (arg as u64) >> size == 0
    };
    fits.then(|| arg as u64 & low(size))
}

/// An integer operand: a NATURAL one (`unsigned`) must not be negative.
#[inline(always)]
fn nat_or_int(i: i64, signed: bool) -> Option<i64> {
    (signed || i >= 0).then_some(i)
}

#[inline(always)]
fn cell(a: &[u64], i: usize) -> Option<i64> {
    a.get(i).map(|&c| c as i64)
}

fn fast_add<const SUB: bool, const SIGNED: bool, const FORM: u8>(
    m: &mut Machine,
    a: &[u64],
) -> Option<u64> {
    let (n, x, y) = match FORM {
        0 => {
            let (Some(l), Some(r)) = (small(m, a, 0), small(m, a, A1)) else {
                return wide::add(m, a, SUB, SIGNED, 0);
            };
            let n = l.n.max(r.n);
            (
                n,
                extend_bits(l.bits, l.n, n, SIGNED),
                extend_bits(r.bits, r.n, n, SIGNED),
            )
        }
        1 => {
            let Some(l) = small(m, a, 0) else {
                return wide::add(m, a, SUB, SIGNED, 1);
            };
            let i = nat_or_int(cell(a, A1)?, SIGNED)?;
            (l.n, l.bits, fit_bits(i, l.n, SIGNED)?)
        }
        _ => {
            let Some(r) = small(m, a, 1) else {
                return wide::add(m, a, SUB, SIGNED, 2);
            };
            let i = nat_or_int(cell(a, 0)?, SIGNED)?;
            (r.n, fit_bits(i, r.n, SIGNED)?, r.bits)
        }
    };
    let s = if SUB {
        x.wrapping_sub(y)
    } else {
        x.wrapping_add(y)
    };
    put_down(m, n, s).ok()
}

/// `*` on two operands of at most 64 elements together, without a metavalue: the product of
/// `l + r` elements is the low `l + r` bits of the 64-bit product of the operands extended to
/// that length. An integer operand is converted at the vector's length and must fit it (else the
/// body's warning comes from the general path). A product over 64 elements is the word-wise
/// path's; anything else (a metavalue, a null array, over `ieee_wide::MAX_BITS`) the general path.
fn fast_mul<const SIGNED: bool, const FORM: u8>(m: &mut Machine, a: &[u64]) -> Option<u64> {
    let ((ln, lb), (rn, rb)) = match FORM {
        0 => {
            let (Some(l), Some(r)) = (small(m, a, 0), small(m, a, A1)) else {
                return wide::mul(m, a, SIGNED, 0);
            };
            ((l.n, l.bits), (r.n, r.bits))
        }
        1 => {
            let Some(l) = small(m, a, 0) else {
                return wide::mul(m, a, SIGNED, 1);
            };
            let i = nat_or_int(cell(a, A1)?, SIGNED)?;
            ((l.n, l.bits), (l.n, fit_bits(i, l.n, SIGNED)?))
        }
        _ => {
            let Some(r) = small(m, a, 1) else {
                return wide::mul(m, a, SIGNED, 2);
            };
            let i = nat_or_int(cell(a, 0)?, SIGNED)?;
            ((r.n, fit_bits(i, r.n, SIGNED)?), (r.n, r.bits))
        }
    };
    let n = ln + rn;
    if n > 64 {
        return wide::mul(m, a, SIGNED, FORM);
    }
    let x = extend_bits(lb, ln, n, SIGNED);
    let y = extend_bits(rb, rn, n, SIGNED);
    put_down(m, n, x.wrapping_mul(y)).ok()
}

/// `/`, `rem` and `mod` of a vector of `n` elements (`v` its bits) and an integer `i` that does
/// not fit them: the bodies divide at the larger length and resize back. The result bits, or
/// `None` where the body warns, fails (`natural rem unsigned`, a zero divisor) or is `narrow`'s.
fn div_wide_int<const KIND: u8, const SIGNED: bool, const FORM: u8>(
    n: usize,
    v: u64,
    i: i64,
) -> Option<u64> {
    let x: i128 = if SIGNED {
        signed_of(v, n).into()
    } else {
        v.into()
    };
    let i = i128::from(i);
    // Floored division's remainder has the divisor's sign.
    let modulo = |a: i128, b: i128| {
        let r = a % b;
        if r != 0 && (r < 0) != (b < 0) {
            r + b
        } else {
            r
        }
    };
    let out = if FORM == 1 {
        match KIND {
            // Zeros whatever the vector holds; the vector is its own remainder when unsigned.
            0 => 0,
            _ if !SIGNED => x,
            1 => x % i,
            _ => modulo(x, i),
        }
    } else {
        if x == 0 {
            return None;
        }
        match (KIND, SIGNED) {
            (1, false) => return None,
            (0, _) => i / x,
            (1, true) | (2, false) => i % x,
            _ => modulo(i, x),
        }
    };
    // Warns (and resizes by the larger vector's sign) where the result needs more elements.
    let fits = if SIGNED {
        (-(1i128 << (n - 1))..1i128 << (n - 1)).contains(&out)
    } else {
        (0..1i128 << n).contains(&out)
    };
    fits.then_some(out as u64 & low(n))
}

/// `/`, `rem` and `mod` of operands without a metavalue or a weak bit (`mod` reads the raw sign
/// element): `u64` magnitudes, the signs applied as the bodies do, the word-wise path above 64
/// elements. A zero divisor and an integer that does not fit the vector take the general path.
fn fast_div<const KIND: u8, const SIGNED: bool, const FORM: u8>(
    m: &mut Machine,
    a: &[u64],
) -> Option<u64> {
    let pure = |v: Small| v.pure.then_some(v);
    let ((n, x), (k, y)) = match FORM {
        0 => {
            let (Some(l), Some(r)) = (small(m, a, 0), small(m, a, A1)) else {
                return wide::divide(m, a, KIND, SIGNED, 0);
            };
            let (l, r) = (pure(l)?, pure(r)?);
            ((l.n, l.bits), (r.n, r.bits))
        }
        1 => {
            let Some(l) = small(m, a, 0) else {
                return wide::divide(m, a, KIND, SIGNED, 1);
            };
            let l = pure(l)?;
            let i = nat_or_int(cell(a, A1)?, SIGNED)?;
            let Some(y) = fit_bits(i, l.n, SIGNED) else {
                let out = div_wide_int::<KIND, SIGNED, FORM>(l.n, l.bits, i)?;
                return put_down(m, l.n, out).ok();
            };
            ((l.n, l.bits), (l.n, y))
        }
        _ => {
            let Some(r) = small(m, a, 1) else {
                return wide::divide(m, a, KIND, SIGNED, 2);
            };
            let r = pure(r)?;
            let i = nat_or_int(cell(a, 0)?, SIGNED)?;
            let Some(x) = fit_bits(i, r.n, SIGNED) else {
                let out = div_wide_int::<KIND, SIGNED, FORM>(r.n, r.bits, i)?;
                return put_down(m, r.n, out).ok();
            };
            ((r.n, x), (r.n, r.bits))
        }
    };
    // Magnitude and sign; the most negative value is its own magnitude as an unsigned one.
    let mag = |v: u64, n: usize| {
        let neg = SIGNED && v >> (n - 1) & 1 == 1;
        (if neg { v.wrapping_neg() & low(n) } else { v }, neg)
    };
    let ((xm, xneg), (ym, yneg)) = (mag(x, n), mag(y, k));
    if ym == 0 {
        return None;
    }
    let (q, r) = (xm / ym, xm % ym);
    let (len, out) = match KIND {
        0 => (n, if xneg != yneg { q.wrapping_neg() } else { q }),
        1 => (k, if xneg { r.wrapping_neg() } else { r }),
        _ => (
            k,
            if yneg && xneg {
                r.wrapping_neg()
            } else if yneg && r != 0 {
                r.wrapping_sub(ym)
            } else if xneg && r != 0 {
                ym.wrapping_sub(r)
            } else {
                r
            },
        ),
    };
    put_down(m, len, out & low(len)).ok()
}

/// A Synopsys operator (`K`: 0 `+`, 1 `-`, 2 `*`, 3 and up `CMPS`; `LK`, `RK`: 0 unsigned, 1
/// signed, 2 integer, 3 `std_ulogic`) on 1 to 64 elements of `0 1 L H` with the body's lengths
/// and integer truncation (decisions.md); `None`: the general path. `KEEP`: the operand's own
/// bounds.
fn fast_oper<const K: u8, const LK: u8, const RK: u8, const KEEP: bool>(
    m: &mut Machine,
    a: &[u64],
) -> Option<u64> {
    let cmp = |ord: std::cmp::Ordering| Some(u64::from(holds(CMPS[K as usize - 3], ord)));
    if LK < 2 && RK < 2 {
        let (l, r) = (small(m, a, 0)?, small(m, a, A1)?);
        let p = l.n + usize::from(LK == 0 && RK == 1);
        let q = r.n + usize::from(LK == 1 && RK == 0);
        let n = if K == 2 { p + q } else { p.max(q) };
        if n > 64 {
            return None;
        }
        let x = extend_bits(l.bits, l.n, n, LK == 1);
        let y = extend_bits(r.bits, r.n, n, RK == 1);
        return match K {
            0 => put_down(m, n, x.wrapping_add(y)).ok(),
            1 => put_down(m, n, x.wrapping_sub(y)).ok(),
            2 => put_down(m, n, x.wrapping_mul(y)).ok(),
            _ if LK == 1 || RK == 1 => cmp(signed_of(x, n).cmp(&signed_of(y, n))),
            _ => cmp(x.cmp(&y)),
        };
    }
    let vec_first = RK >= 2;
    let (v, i) = if vec_first {
        (small(m, a, 0)?, cell(a, A1)?)
    } else {
        (small(m, a, 1)?, cell(a, 0)?)
    };
    let i = if LK == 3 || RK == 3 { bit_of(i)? } else { i };
    let iv = i as u64;
    match K {
        0 | 1 => {
            let y = iv & low(v.n);
            let (x, y) = if vec_first { (v.bits, y) } else { (y, v.bits) };
            let r = if K == 0 {
                x.wrapping_add(y)
            } else {
                x.wrapping_sub(y)
            };
            if KEEP {
                put_small(m, v.n, r, v.left, v.right, v.down).ok()
            } else {
                put_down(m, v.n, r).ok()
            }
        }
        2 => None,
        _ => {
            // A signed vector against the integer's low `n` bits; an unsigned one against its
            // low `n + 1` bits as a signed value.
            let o = if (if vec_first { LK } else { RK }) == 1 {
                signed_of(v.bits, v.n).cmp(&signed_of(iv & low(v.n), v.n))
            } else {
                if v.n >= 64 {
                    return None;
                }
                (v.bits as i64).cmp(&signed_of(iv & low(v.n + 1), v.n + 1))
            };
            cmp(if vec_first { o } else { o.reverse() })
        }
    }
}

/// `conv_unsigned`, `conv_signed`, `conv_std_logic_vector`, `ext`, `sxt` of 1 to 64 elements of
/// `0 1 L H` to 1 to 64: the low `min (n, size)` elements, zero or the top element above them.
fn fast_syn_conv_vec<const FROM_SIGNED: bool>(m: &mut Machine, a: &[u64]) -> Option<u64> {
    let l = small(m, a, 0)?;
    let size = usize::try_from(cell(a, A1)?)
        .ok()
        .filter(|s| (1..=64).contains(s))?;
    let bits = if l.n >= size {
        l.bits & low(size)
    } else {
        extend_bits(l.bits, l.n, size, FROM_SIGNED)
    };
    put_down(m, size, bits).ok()
}

/// The conversions of an integer to 1 to 64 elements: its low two's complement bits. The unsigned
/// body fails for `INTEGER'LOW` (either revision), which the body words.
fn fast_syn_conv_int<const SIGNED: bool>(m: &mut Machine, a: &[u64]) -> Option<u64> {
    let (arg, size) = (cell(a, 0)?, usize::try_from(cell(a, 1)?).ok()?);
    if !(1..=64).contains(&size) || (!SIGNED && (arg == i64::MIN || arg == i64::from(i32::MIN))) {
        return None;
    }
    put_down(m, size, arg as u64 & low(size)).ok()
}

/// `conv_integer` of 1 to 31 (unsigned) or 32 (signed) elements of `0 1 L H`; a longer vector is
/// the body's failure, a metavalue its warning.
fn fast_syn_to_int<const SIGNED: bool>(m: &mut Machine, a: &[u64]) -> Option<u64> {
    let l = small(m, a, 0)?;
    if l.n > 31 + usize::from(SIGNED) {
        return None;
    }
    Some(if SIGNED {
        signed_of(l.bits, l.n) as u64
    } else {
        l.bits
    })
}

fn syn_to_int<const SIGNED: bool>(
    m: &mut Machine,
    _: Cx<'_, '_, '_>,
    _: Option<TyId>,
    a: &[u64],
) -> Ret {
    match fast_syn_to_int::<SIGNED>(m, a) {
        Some(v) => Ok(Some(v)),
        None => fallback(m),
    }
}

/// `shl` / `shr`: the elements (no `L` or `H`) move, zeros or the sign come in, by a count of at
/// most 31 elements (a longer one may name 2 ** 31, the body's error). Result `(n-1 downto 0)`.
fn fast_syn_shift<const RIGHT: bool, const SIGNED: bool>(
    m: &mut Machine,
    a: &[u64],
) -> Option<u64> {
    let (v, c) = (small(m, a, 0)?, small(m, a, A1)?);
    if !v.pure || c.n > 31 {
        return None;
    }
    let n = v.n;
    let big = c.bits >= n as u64;
    let bits = match (RIGHT, SIGNED) {
        (false, _) => {
            if big {
                0
            } else {
                v.bits << c.bits
            }
        }
        (true, false) => {
            if big {
                0
            } else {
                v.bits >> c.bits
            }
        }
        (true, true) => (signed_of(v.bits, n) >> c.bits.min(n as u64 - 1)) as u64,
    };
    put_down(m, n, bits & low(n)).ok()
}

/// [`fast_syn_shift`] on any argument, metavalues and weak elements included (they move as they
/// are): `Ok(None)` for a null argument and for a count that is empty, over 31 elements or not binary.
fn syn_shift_wide<const RIGHT: bool, const SIGNED: bool>(m: &mut Machine, a: &[u64]) -> Ret {
    let (v, c) = (vh(m, a, 0)?, vh(m, a, A1)?);
    let n = v.v.len();
    let count = c.v.to_bitvec().filter(|b| (1..=31).contains(&b.len()));
    let (Some(cnt), true) = (count.and_then(|b| b.to_u64()), n > 0) else {
        return Ok(None);
    };
    let c = usize::try_from(cnt).unwrap_or(usize::MAX);
    let out = match (RIGHT, SIGNED) {
        (false, _) => v.v.shl(c),
        (true, false) => v.v.shr(c),
        (true, true) => v.v.sra(c),
    };
    put(m, &Vh::down(out))
}

fn syn_shift<const RIGHT: bool, const SIGNED: bool>(
    m: &mut Machine,
    _: Cx<'_, '_, '_>,
    _: Option<TyId>,
    a: &[u64],
) -> Ret {
    if let Some(v) = fast_syn_shift::<RIGHT, SIGNED>(m, a) {
        return Ok(Some(v));
    }
    match syn_shift_wide::<RIGHT, SIGNED>(m, a)? {
        Some(v) => Ok(Some(v)),
        None => fallback(m),
    }
}

/// [`fast_oper`] on vectors of any length, on `BitVec`s. `Ok(None)`: the body's (a null or a
/// metavalue, which it words with its warnings and its index error).
fn oper_wide<const K: u8, const LK: u8, const RK: u8, const KEEP: bool>(
    m: &mut Machine,
    a: &[u64],
) -> Ret {
    let bin = |v: &Vh| v.v.to_bitvec().filter(|b| !b.is_empty());
    let down = |b: &BitVec| Vh::down(LogicVec::from_bitvec(b));
    if LK < 2 && RK < 2 {
        let (l, r) = (vh(m, a, 0)?, vh(m, a, A1)?);
        let (Some(x), Some(y)) = (bin(&l), bin(&r)) else {
            return Ok(None);
        };
        let p = x.len() + usize::from(LK == 0 && RK == 1);
        let q = y.len() + usize::from(LK == 1 && RK == 0);
        let n = if K == 2 { p + q } else { p.max(q) };
        let (x, y) = (x.extend(n, LK == 1), y.extend(n, RK == 1));
        return match K {
            0 => put(m, &down(&x.add(&y, false))),
            1 => put(m, &down(&x.sub(&y))),
            2 => put(m, &down(&x.mul_trunc(&y, n))),
            _ => {
                let o = x.cmp_num(&y, LK == 1 || RK == 1);
                Ok(Some(u64::from(holds(CMPS[K as usize - 3], o))))
            }
        };
    }
    let vec_first = RK >= 2;
    let (v, i) = if vec_first {
        (vh(m, a, 0)?, int(a, A1)?)
    } else {
        (vh(m, a, 1)?, int(a, 0)?)
    };
    let (Some(x), Some(i)) = (
        bin(&v),
        if LK == 3 || RK == 3 {
            bit_of(i)
        } else {
            Some(i)
        },
    ) else {
        return Ok(None);
    };
    let n = x.len();
    match K {
        0 | 1 => {
            let y = BitVec::from_i64(n, i);
            let (p, q) = if vec_first { (x, y) } else { (y, x) };
            let r = if K == 0 { p.add(&q, false) } else { p.sub(&q) };
            let mut out = down(&r);
            if KEEP {
                (out.left, out.right, out.down) = (v.left, v.right, v.down);
            }
            put(m, &out)
        }
        2 => Ok(None),
        _ => {
            // A signed vector against the integer's low `n` bits, an unsigned one (one more
            // element) against its low `n + 1` bits, both as signed values.
            let o = if (if vec_first { LK } else { RK }) == 1 {
                x.cmp_num(&BitVec::from_i64(n, i), true)
            } else {
                x.extend(n + 1, false)
                    .cmp_num(&BitVec::from_i64(n + 1, i), true)
            };
            let o = if vec_first { o } else { o.reverse() };
            Ok(Some(u64::from(holds(CMPS[K as usize - 3], o))))
        }
    }
}

/// The native of [`fast_oper`]: its result, or the body when a metavalue or a null asks for it.
fn oper<const K: u8, const LK: u8, const RK: u8, const KEEP: bool>(
    m: &mut Machine,
    _: Cx<'_, '_, '_>,
    _: Option<TyId>,
    a: &[u64],
) -> Ret {
    if let Some(v) = fast_oper::<K, LK, RK, KEEP>(m, a) {
        return Ok(Some(v));
    }
    match oper_wide::<K, LK, RK, KEEP>(m, a)? {
        Some(v) => Ok(Some(v)),
        None => fallback(m),
    }
}

/// Unary `+`, `-` and `abs` of the Synopsys packages (`Syn::Unary`) on 1 to 64 elements of
/// `0 1 L H`; an operand the body returns as it is (`+`, a non-negative `abs`) only without `L` or
/// `H`, which it keeps. `None`: the body.
fn fast_syn_unary<const NEG: bool, const ABS: bool, const KEEP: bool>(
    m: &mut Machine,
    a: &[u64],
) -> Option<u64> {
    let v = small(m, a, 0)?;
    if !NEG || (ABS && v.bits >> (v.n - 1) & 1 == 0) {
        return v
            .pure
            .then(|| put_small(m, v.n, v.bits, v.left, v.right, v.down).ok())?;
    }
    let r = v.bits.wrapping_neg();
    if KEEP {
        put_small(m, v.n, r, v.left, v.right, v.down).ok()
    } else {
        put_down(m, v.n, r).ok()
    }
}

fn syn_unary<const NEG: bool, const ABS: bool, const KEEP: bool>(
    m: &mut Machine,
    _: Cx<'_, '_, '_>,
    _: Option<TyId>,
    a: &[u64],
) -> Ret {
    if let Some(v) = fast_syn_unary::<NEG, ABS, KEEP>(m, a) {
        return Ok(Some(v));
    }
    general(m);
    // Any length: the operand as it is (`+`, `abs` with a left element `0` or `L`, whatever the
    // others are), else `0 - L` of `0 1 L H`; a null or a metavalue is the body's.
    let v = vh(m, a, 0)?;
    let n = v.v.len();
    if !NEG || (ABS && n > 0 && matches!(v.v.get(n - 1), StdULogic::Zero | StdULogic::L)) {
        return put(m, &v);
    }
    let Some(x) = v.v.to_bitvec().filter(|b| !b.is_empty()) else {
        return fallback(m);
    };
    let mut out = Vh::down(LogicVec::from_bitvec(&BitVec::from_i64(n, 0).sub(&x)));
    if KEEP {
        (out.left, out.right, out.down) = (v.left, v.right, v.down);
    }
    put(m, &out)
}

/// The instance of `$f` for `Syn::Unary` (`+` is the same function whatever `keep` says).
macro_rules! unary_pick {
    ($f:ident, $t:ty, $neg:expr, $abs:expr, $keep:expr) => {
        match ($neg, $abs, $keep) {
            (false, _, _) => $f::<false, false, false> as $t,
            (true, false, false) => $f::<true, false, false> as $t,
            (true, false, true) => $f::<true, false, true> as $t,
            (true, true, false) => $f::<true, true, false> as $t,
            (true, true, true) => $f::<true, true, true> as $t,
        }
    };
}

fn oper_code(k: SynKind) -> u8 {
    match k {
        SynKind::Add => 0,
        SynKind::Sub => 1,
        SynKind::Mul => 2,
        SynKind::Cmp(c) => 3 + CMPS.iter().position(|x| *x == c).unwrap_or(0) as u8,
    }
}

fn opd_code(o: Opd) -> u8 {
    match o {
        Opd::U => 0,
        Opd::S => 1,
        Opd::I => 2,
        Opd::B => 3,
    }
}

/// A `std_ulogic` operand of a Synopsys operator: `0 L` as 0, `1 H` as 1, `None` for a metavalue
/// (the body's warning and all `X`).
fn bit_of(code: i64) -> Option<i64> {
    matches!(code, 2 | 3 | 6 | 7).then_some(code & 1)
}

/// The forms of one operator kind: every pair of operands but two integers.
macro_rules! oper_forms {
    ($f:ident, $t:ty, $k:literal, $l:expr, $r:expr, $keep:expr) => {
        match ($l, $r, $keep) {
            (0, 0, _) => $f::<$k, 0, 0, false> as $t,
            (1, 1, _) => $f::<$k, 1, 1, false> as $t,
            (0, 1, _) => $f::<$k, 0, 1, false> as $t,
            (1, 0, _) => $f::<$k, 1, 0, false> as $t,
            (0, 2, false) => $f::<$k, 0, 2, false> as $t,
            (0, 2, true) => $f::<$k, 0, 2, true> as $t,
            (1, 2, false) => $f::<$k, 1, 2, false> as $t,
            (1, 2, true) => $f::<$k, 1, 2, true> as $t,
            (2, 0, false) => $f::<$k, 2, 0, false> as $t,
            (2, 0, true) => $f::<$k, 2, 0, true> as $t,
            (2, 1, false) => $f::<$k, 2, 1, false> as $t,
            (2, 1, true) => $f::<$k, 2, 1, true> as $t,
            (0, 3, false) => $f::<$k, 0, 3, false> as $t,
            (0, 3, true) => $f::<$k, 0, 3, true> as $t,
            (1, 3, false) => $f::<$k, 1, 3, false> as $t,
            (1, 3, true) => $f::<$k, 1, 3, true> as $t,
            (3, 0, false) => $f::<$k, 3, 0, false> as $t,
            (3, 0, true) => $f::<$k, 3, 0, true> as $t,
            (3, 1, false) => $f::<$k, 3, 1, false> as $t,
            (3, 1, true) => $f::<$k, 3, 1, true> as $t,
            _ => return None,
        }
    };
}

macro_rules! oper_pick {
    ($f:ident, $t:ty, $k:expr, $l:expr, $r:expr, $keep:expr) => {
        match $k {
            0 => oper_forms!($f, $t, 0, $l, $r, $keep),
            1 => oper_forms!($f, $t, 1, $l, $r, $keep),
            2 => oper_forms!($f, $t, 2, $l, $r, $keep),
            3 => oper_forms!($f, $t, 3, $l, $r, $keep),
            4 => oper_forms!($f, $t, 4, $l, $r, $keep),
            5 => oper_forms!($f, $t, 5, $l, $r, $keep),
            6 => oper_forms!($f, $t, 6, $l, $r, $keep),
            7 => oper_forms!($f, $t, 7, $l, $r, $keep),
            8 => oper_forms!($f, $t, 8, $l, $r, $keep),
            _ => return None,
        }
    };
}

/// `to_01` of 1 to 64 elements that are all `0 1 L H`: the bits as `(n-1 downto 0)`, whatever the
/// operand's bounds. A metavalue (the result is `xmap`), a null or a longer vector takes the general path.
fn fast_to_01(m: &mut Machine, a: &[u64]) -> Option<u64> {
    let l = small(m, a, 0)?;
    put_down(m, l.n, l.bits).ok()
}

fn fast_compare<const C: u8, const SIGNED: bool, const FORM: u8>(
    m: &mut Machine,
    a: &[u64],
) -> Option<u64> {
    let ord = match FORM {
        0 => {
            let (Some(l), Some(r)) = (small(m, a, 0), small(m, a, A1)) else {
                return wide::compare(m, a, CMPS[C as usize], SIGNED, 0);
            };
            if SIGNED {
                signed_of(l.bits, l.n).cmp(&signed_of(r.bits, r.n))
            } else {
                l.bits.cmp(&r.bits)
            }
        }
        _ => {
            let Some(v) = small(m, a, usize::from(FORM != 1)) else {
                return wide::compare(m, a, CMPS[C as usize], SIGNED, FORM);
            };
            let i = nat_or_int(cell(a, if FORM == 1 { A1 } else { 0 })?, SIGNED)?;
            let o = if SIGNED {
                signed_of(v.bits, v.n).cmp(&i)
            } else {
                v.bits.cmp(&(i as u64))
            };
            if FORM == 1 { o } else { o.reverse() }
        }
    };
    Some(u64::from(holds(CMPS[C as usize], ord)))
}

/// `?=` ... `?>=`: for operands of `0 1 L H` the relation of [`fast_compare`] as `'0'` or `'1'`
/// (the codes 2 and 3), with the integer fitting the vector's length (else the body's
/// `to_unsigned` warns). `?=` and `?/=` take any element value ([`fast_qeq`]), the ordering
/// relations give `X` for a metavalue other than `-`; a `-` there, a null array and anything else
/// go to the general path.
fn fast_qcompare<const C: u8, const SIGNED: bool, const FORM: u8>(
    m: &mut Machine,
    a: &[u64],
) -> Option<u64> {
    if C >= 4 {
        return fast_qeq::<SIGNED, FORM>(m, a, C == 5);
    }
    if FORM != 0
        && let Some(v) = small(m, a, usize::from(FORM != 1))
    {
        let i = nat_or_int(cell(a, if FORM == 1 { A1 } else { 0 })?, SIGNED)?;
        fit_bits(i, v.n, SIGNED)?;
    }
    fast_compare::<C, SIGNED, FORM>(m, a)
        .map(|h| h + 2)
        .or_else(|| wide::order_meta(m, a, SIGNED, FORM))
}

/// The `std_ulogic` code of the body's `?=` fold (`wide::qeq_code`) for two operands of up to 64
/// elements as planes, the shorter extended with zeros or, for `signed`, its sign element.
fn qeq_word(p: [u64; 4], n: usize, q: [u64; 4], k: usize, signed: bool) -> u64 {
    let size = n.max(k);
    let valid = low(size);
    let ext = |x: [u64; 4], c: usize| {
        let sign = signed.then(|| x.map(|w| 0u64.wrapping_sub(w >> (c - 1) & 1)));
        wide::ext_block(x, low(c), valid & !low(c), sign)
    };
    wide::qeq_code(wide::qeq_block(ext(p, n), ext(q, k), valid))
}

/// `?=` (`ne` false) and `?/=` of up to 64 elements of any value, or the wide form.
fn fast_qeq<const SIGNED: bool, const FORM: u8>(
    m: &mut Machine,
    a: &[u64],
    ne: bool,
) -> Option<u64> {
    let code = match FORM {
        0 => match (block_of(m, a), block_of(m, a.get(A1..)?)) {
            (Some((n, p)), Some((k, q))) => qeq_word(p, n, q, k, SIGNED),
            _ => wide::qeq(m, a, SIGNED, 0)?,
        },
        _ => {
            let at = usize::from(FORM != 1);
            let i = cell(a, if FORM == 1 { A1 } else { 0 })?;
            match block_of(m, a.get(at..)?) {
                Some((n, p)) => {
                    let b = fit_bits(nat_or_int(i, SIGNED)?, n, SIGNED)?;
                    qeq_word(p, n, [b, low(n), 0, 0], n, SIGNED)
                }
                None => wide::qeq(m, a, SIGNED, FORM)?,
            }
        }
    };
    // `?/=` is the element-wise negation of the fold: `U` and `X` stay.
    Some(if ne && code >= 2 { 5 - code } else { code })
}

/// `maximum` / `minimum` of up to 1024 elements of `0 1 L H`: the operand with the larger (smaller)
/// value, both extended to the longer length; the result is `(n-1 downto 0)`. Integer rules of
/// [`fast_qcompare`]; anything else goes to the body.
fn fast_minmax<const MAX: bool, const SIGNED: bool, const FORM: u8>(
    m: &mut Machine,
    a: &[u64],
) -> Option<u64> {
    let (n, x, y) = match FORM {
        0 => {
            let (Some(l), Some(r)) = (small(m, a, 0), small(m, a, A1)) else {
                return wide::minmax(m, a, MAX, SIGNED, 0);
            };
            let n = l.n.max(r.n);
            (
                n,
                extend_bits(l.bits, l.n, n, SIGNED),
                extend_bits(r.bits, r.n, n, SIGNED),
            )
        }
        1 => {
            let Some(l) = small(m, a, 0) else {
                return wide::minmax(m, a, MAX, SIGNED, 1);
            };
            let i = nat_or_int(cell(a, A1)?, SIGNED)?;
            (l.n, l.bits, fit_bits(i, l.n, SIGNED)?)
        }
        _ => {
            let Some(r) = small(m, a, 1) else {
                return wide::minmax(m, a, MAX, SIGNED, 2);
            };
            let i = nat_or_int(cell(a, 0)?, SIGNED)?;
            (r.n, fit_bits(i, r.n, SIGNED)?, r.bits)
        }
    };
    let less = if SIGNED {
        signed_of(x, n) < signed_of(y, n)
    } else {
        x < y
    };
    put_down(m, n, if less == MAX { y } else { x }).ok()
}

/// `std_match` of two vectors of one length, up to 1024 elements: an element matches when either
/// side is `-`, or both are `0 1 L H` with the same value (plane 3 is `-`, plane 1 without plane 3
/// the known elements, plane 0 their value). Unequal lengths and null arrays go to the body.
fn fast_match(m: &mut Machine, a: &[u64]) -> Option<u64> {
    let (Some((n, p)), Some((k, q))) = (block_of(m, a), block_of(m, a.get(A1..)?)) else {
        return wide::std_match(m, a);
    };
    if n != k {
        return None;
    }
    let (kp, kq) = (p[1] & !p[3], q[1] & !q[3]);
    let ok = p[3] | q[3] | kp & kq & !(p[0] ^ q[0]);
    Some(u64::from(!ok & low(n) == 0))
}

/// [`match_eq`] of any element values: up to 64 elements in one word, up to `wide::MAX_BITS` in
/// `wide::match_eq`; unequal lengths go to the native's error.
fn fast_match_eq<const NE: bool, const VEC: bool>(m: &mut Machine, a: &[u64]) -> Option<u64> {
    let c = if !VEC {
        let (x, y) = (cell(a, 0)?, cell(a, 1)?);
        let sc = |v: i64| {
            (0..=8)
                .contains(&v)
                .then(|| StdULogic::from_code_pub(v as u8))
        };
        sc(x)?.match_eq(sc(y)?) as u64
    } else {
        match (block_of(m, a), block_of(m, a.get(A1..)?)) {
            (Some((n, p)), Some((k, q))) if n == k => wide::and_code(wide::qeq_block(p, q, low(n))),
            _ => wide::match_eq(m, a)?,
        }
    };
    // `?/=` is the negation: `U` and `X` stay.
    Some(if NE && c >= 2 { 5 - c } else { c })
}

fn fast_to_integer<const SIGNED: bool>(m: &mut Machine, a: &[u64]) -> Option<u64> {
    let high = m.rt.revision.integer_bounds().high as u64;
    let Some(v) = small(m, a, 0) else {
        return wide::to_integer(m, a, SIGNED, high);
    };
    let neg = SIGNED && v.bits >> (v.n - 1) & 1 == 1;
    // A one-element `signed` 1 warns, and beyond INTEGER'HIGH the body's checked addition fails.
    if SIGNED && v.n == 1 && neg {
        return None;
    }
    let mag = if neg { !v.bits & low(v.n) } else { v.bits };
    (mag <= high).then(|| if neg { (-(mag as i64) - 1) as u64 } else { mag })
}

fn fast_to_unsigned<const SIGNED: bool>(m: &mut Machine, a: &[u64]) -> Option<u64> {
    let (arg, size) = (cell(a, 0)?, usize::try_from(cell(a, 1)?).ok()?);
    if (1..=64).contains(&size) {
        let bits = fit_bits(nat_or_int(arg, SIGNED)?, size, SIGNED)?;
        return put_down(m, size, bits).ok();
    }
    wide::from_int(m, arg, size, SIGNED)
}

#[inline(always)]
pub(crate) fn gate_bits(g: Gate, x: u64, y: u64) -> u64 {
    match g {
        Gate::And => x & y,
        Gate::Nand => !(x & y),
        Gate::Or => x | y,
        Gate::Nor => !(x | y),
        Gate::Xor => x ^ y,
        Gate::Xnor => !(x ^ y),
    }
}

fn fast_vec_gate<const G: u8>(m: &mut Machine, a: &[u64]) -> Option<u64> {
    let g = GATES[G as usize];
    let (Some(l), Some(r)) = (small(m, a, 0), small(m, a, A1)) else {
        return wide::gate(m, a, Some(g), Bounds::Up1);
    };
    if l.n != r.n {
        return None;
    }
    put_small(m, l.n, gate_bits(g, l.bits, r.bits), 1, l.n as i64, false).ok()
}

fn fast_num_gate<const G: u8>(m: &mut Machine, a: &[u64]) -> Option<u64> {
    let g = GATES[G as usize];
    let (Some(l), Some(r)) = (small(m, a, 0), small(m, a, A1)) else {
        return wide::gate(m, a, Some(g), Bounds::Down);
    };
    if l.n != r.n {
        return None;
    }
    put_down(m, l.n, gate_bits(g, l.bits, r.bits)).ok()
}

fn fast_num_not(m: &mut Machine, a: &[u64]) -> Option<u64> {
    let Some(l) = small(m, a, 0) else {
        return wide::gate(m, a, None, Bounds::Down);
    };
    put_down(m, l.n, !l.bits).ok()
}

fn fast_neg(m: &mut Machine, a: &[u64]) -> Option<u64> {
    let Some(l) = small(m, a, 0) else {
        return wide::neg(m, a);
    };
    put_down(m, l.n, l.bits.wrapping_neg()).ok()
}

fn fast_abs(m: &mut Machine, a: &[u64]) -> Option<u64> {
    let Some(l) = small(m, a, 0) else {
        return wide::abs(m, a);
    };
    let neg = l.bits >> (l.n - 1) & 1 == 1;
    put_down(m, l.n, if neg { l.bits.wrapping_neg() } else { l.bits }).ok()
}

fn fast_vec_not(m: &mut Machine, a: &[u64]) -> Option<u64> {
    let Some(l) = small(m, a, 0) else {
        return wide::gate(m, a, None, Bounds::Up1);
    };
    put_small(m, l.n, !l.bits, 1, l.n as i64, false).ok()
}

fn fast_resize<const SIGNED: bool>(m: &mut Machine, a: &[u64]) -> Option<u64> {
    let new = usize::try_from(cell(a, A1)?).ok()?;
    if (1..=64).contains(&new)
        && let Some(v) = small(m, a, 0).filter(|v| v.pure)
    {
        let bits = if SIGNED {
            let keep = v.n.min(new) - 1;
            let sign = v.bits >> (v.n - 1) & 1 == 1;
            let kept = if keep == 0 { 0 } else { v.bits & low(keep) };
            kept | if sign { low(new) & !low(keep) } else { 0 }
        } else {
            v.bits & low(v.n.min(new))
        };
        return put_down(m, new, bits).ok();
    }
    wide::resize(m, a, SIGNED)
}

fn fast_shift<const K: u8>(m: &mut Machine, a: &[u64]) -> Option<u64> {
    let count = usize::try_from(cell(a, A1)?).ok()?;
    match small(m, a, 0) {
        Some(v) if v.pure => shift_small::<K>(m, &v, count).ok().flatten(),
        Some(_) => None,
        None if K != 2 => wide::shift(m, a, K),
        None => None,
    }
}

fn fast_is_x_sul(_: &mut Machine, a: &[u64]) -> Option<u64> {
    Some(u64::from(sul(a, 0).ok()?.is_x()))
}

fn fast_truthy(_: &mut Machine, a: &[u64]) -> Option<u64> {
    Some(u64::from(sul(a, 0).ok()?.is_true()))
}

fn fast_conv_sul<const K: u8>(_: &mut Machine, a: &[u64]) -> Option<u64> {
    Some(conv_of(K, sul(a, 0).ok()?) as u64)
}

/// `is_x` of a vector reads its planes in place and makes nothing: its general path is its fast one.
fn fast_is_x_vec(m: &mut Machine, a: &[u64]) -> Option<u64> {
    is_x_vec(m, &mut None, None, a).ok().flatten()
}

fn fast_reduce<const G: u8>(m: &mut Machine, a: &[u64]) -> Option<u64> {
    reduce::<G>(m, &mut None, None, a).ok().flatten()
}

/// The one block of planes of the array operand at cell 0 when it has 1 to 64 elements, with the
/// element count.
#[inline(always)]
fn block_of(m: &Machine, a: &[u64]) -> Option<(usize, [u64; 4])> {
    let &[data, left, right, d] = a.get(..ARRAY_REGS)? else {
        return None;
    };
    let (lo, hi) = if d == 1 {
        (right as i64, left as i64)
    } else {
        (left as i64, right as i64)
    };
    let n = usize::try_from(hi.checked_sub(lo)?).ok()?.checked_add(1)?;
    (n <= 64).then_some(())?;
    Some((n, block4(m, data)?))
}

/// A result of `n` elements (1 to 64) in one block of planes with bounds `left`, `right`, `down`.
#[inline(always)]
fn put_block(m: &mut Machine, blk: [u64; 4], left: i64, right: i64, down: bool) -> Option<u64> {
    m.temp_words(|fat| {
        [
            fat + 16 + DOPE_DIM_BYTES,
            fat + 16,
            left as u64,
            right as u64,
            u64::from(down),
            blk[0],
            blk[1],
            blk[2],
            blk[3],
        ]
    })
    .ok()
}

/// `to_x01`, `to_x01z`, `to_ux01` (`K` 0, 1, 2) of a vector of up to 64 elements, as the planes
/// of `yavs_logic` compute them; `None` for a longer one.
fn conv_small<const K: u8>(m: &mut Machine, a: &[u64]) -> Option<u64> {
    let (n, [p0, p1, p2, p3]) = block_of(m, a)?;
    let mask = low1(n);
    let (is0, is1) = (!p3 & p1 & !p0, !p3 & p1 & p0);
    let known = is0 | is1;
    let blk = match K {
        0 => [!is0, known, 0, 0],
        1 => {
            let z = !p3 & p2 & !p1 & !p0;
            [!is0 & !z, known, z, 0]
        }
        _ => [!is0 & (p0 | p1 | p2 | p3), known, 0, 0],
    };
    put_block(m, blk.map(|p| p & mask), 1, n as i64, false)
}

// One word only: a longer vector goes to the native, whose general path is counted.
fn fast_conv_vec<const K: u8>(m: &mut Machine, a: &[u64]) -> Option<u64> {
    conv_small::<K>(m, a)
}

/// `to_bitvector (s, xmap)` of up to 64 elements in one word; `None` for a longer one.
fn to_bitvector_small(m: &mut Machine, a: &[u64]) -> Option<u64> {
    let (n, [p0, p1, _, p3]) = block_of(m, a)?;
    let xmap = if *a.get(A1)? != 0 { !0u64 } else { 0 };
    let known = p1 & !p3;
    put_down(m, n, p0 & known | !known & xmap).ok()
}

fn fast_to_bitvector(m: &mut Machine, a: &[u64]) -> Option<u64> {
    to_bitvector_small(m, a).or_else(|| to_bitvector(m, &mut None, None, a).ok().flatten())
}

/// The instance of `$f::<KIND, SIGNED, FORM>` for a runtime `kind` (0 to 2), `signed` and `form`.
macro_rules! divs {
    ($f:ident, $t:ty, $k:expr, $s:expr, $fm:expr) => {
        match ($k, $s, $fm) {
            (0, false, 0) => $f::<0, false, 0> as $t,
            (0, false, 1) => $f::<0, false, 1> as $t,
            (0, false, _) => $f::<0, false, 2> as $t,
            (0, true, 0) => $f::<0, true, 0> as $t,
            (0, true, 1) => $f::<0, true, 1> as $t,
            (0, true, _) => $f::<0, true, 2> as $t,
            (1, false, 0) => $f::<1, false, 0> as $t,
            (1, false, 1) => $f::<1, false, 1> as $t,
            (1, false, _) => $f::<1, false, 2> as $t,
            (1, true, 0) => $f::<1, true, 0> as $t,
            (1, true, 1) => $f::<1, true, 1> as $t,
            (1, true, _) => $f::<1, true, 2> as $t,
            (_, false, 0) => $f::<2, false, 0> as $t,
            (_, false, 1) => $f::<2, false, 1> as $t,
            (_, false, _) => $f::<2, false, 2> as $t,
            (_, true, 0) => $f::<2, true, 0> as $t,
            (_, true, 1) => $f::<2, true, 1> as $t,
            (_, true, _) => $f::<2, true, 2> as $t,
        }
    };
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
        Op::Neg => neg_signed,
        Op::Abs => abs_signed,
        Op::Not => not,
        Op::VecNot => vec_not,
        Op::Conv(0, false) => conv::<0, false>,
        Op::Conv(1, false) => conv::<1, false>,
        Op::Conv(_, false) => conv::<2, false>,
        Op::Conv(0, true) => conv::<0, true>,
        Op::Conv(1, true) => conv::<1, true>,
        Op::Conv(_, true) => conv::<2, true>,
        Op::To01 => to_01_vec,
        Op::ToBitVector => to_bitvector,
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
        Op::Div {
            kind,
            signed,
            form: f,
        } => divs!(divide, NativeFn, kind, signed, form(f)),
        Op::Mul { signed, form: f } => match (signed, form(f)) {
            (false, 0) => mul::<false, 0>,
            (false, 1) => mul::<false, 1>,
            (false, _) => mul::<false, 2>,
            (true, 0) => mul::<true, 0>,
            (true, 1) => mul::<true, 1>,
            (true, _) => mul::<true, 2>,
        },
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
        Op::QCompare {
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
                            ($c, false, 0) => qcompare::<$c, false, 0> as NativeFn,
                            ($c, false, 1) => qcompare::<$c, false, 1> as NativeFn,
                            ($c, false, _) => qcompare::<$c, false, 2> as NativeFn,
                            ($c, true, 0) => qcompare::<$c, true, 0> as NativeFn,
                            ($c, true, 1) => qcompare::<$c, true, 1> as NativeFn,
                            ($c, true, _) => qcompare::<$c, true, 2> as NativeFn,
                        )*
                        _ => return None,
                    }
                };
            }
            cs!(0, 1, 2, 3, 4, 5)
        }
        Op::MinMax {
            max,
            signed,
            form: f,
        } => pick3!(minmax, max, signed, form(f)),
        Op::Match => std_match,
        Op::MatchEq { ne, vec } => match (ne, vec) {
            (false, false) => match_eq::<false, false>,
            (false, true) => match_eq::<false, true>,
            (true, false) => match_eq::<true, false>,
            (true, true) => match_eq::<true, true>,
        },
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
        Op::Syn(Syn::Oper { kind, l, r, keep }) => {
            oper_pick!(
                oper,
                NativeFn,
                oper_code(kind),
                opd_code(l),
                opd_code(r),
                keep
            )
        }
        Op::Syn(Syn::Shift { right, signed }) => match (right, signed) {
            (false, false) => syn_shift::<false, false>,
            (false, true) => syn_shift::<false, true>,
            (true, false) => syn_shift::<true, false>,
            (true, true) => syn_shift::<true, true>,
        },
        Op::Syn(Syn::ToInt { signed: false }) => syn_to_int::<false>,
        Op::Syn(Syn::ToInt { signed: true }) => syn_to_int::<true>,
        Op::Syn(Syn::Unary { neg, abs, keep }) => unary_pick!(syn_unary, NativeFn, neg, abs, keep),
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

macro_rules! pick3_fast {
    ($f:ident, $a:expr, $b:expr, $c:expr) => {
        match ($a, $b, $c) {
            (false, false, 0) => $f::<false, false, 0> as FastFn,
            (false, false, 1) => $f::<false, false, 1> as FastFn,
            (false, false, _) => $f::<false, false, 2> as FastFn,
            (false, true, 0) => $f::<false, true, 0> as FastFn,
            (false, true, 1) => $f::<false, true, 1> as FastFn,
            (false, true, _) => $f::<false, true, 2> as FastFn,
            (true, false, 0) => $f::<true, false, 0> as FastFn,
            (true, false, 1) => $f::<true, false, 1> as FastFn,
            (true, false, _) => $f::<true, false, 2> as FastFn,
            (true, true, 0) => $f::<true, true, 0> as FastFn,
            (true, true, 1) => $f::<true, true, 1> as FastFn,
            (true, true, _) => $f::<true, true, 2> as FastFn,
        }
    };
}

/// The one-word path of the native of an `yavs.ieee.<key>` symbol, when it has one.
pub(crate) fn lookup_fast(symbol: &str) -> Option<FastFn> {
    let key = symbol.strip_prefix("yavs.ieee.")?;
    let op = yavs_ir::ieee::parse(key.split_once('@').map_or(key, |k| k.0))?;
    Some(match op {
        Op::VecGate(g) => [
            fast_vec_gate::<0> as FastFn,
            fast_vec_gate::<1>,
            fast_vec_gate::<2>,
            fast_vec_gate::<3>,
            fast_vec_gate::<4>,
            fast_vec_gate::<5>,
        ][GATES.iter().position(|x| *x == g)?],
        Op::NumGate(g) => [
            fast_num_gate::<0> as FastFn,
            fast_num_gate::<1>,
            fast_num_gate::<2>,
            fast_num_gate::<3>,
            fast_num_gate::<4>,
            fast_num_gate::<5>,
        ][GATES.iter().position(|x| *x == g)?],
        Op::NumNot => fast_num_not,
        Op::Neg => fast_neg,
        Op::Abs => fast_abs,
        Op::VecNot => fast_vec_not,
        Op::Reduce(g) => [
            fast_reduce::<0> as FastFn,
            fast_reduce::<1>,
            fast_reduce::<2>,
            fast_reduce::<3>,
            fast_reduce::<4>,
            fast_reduce::<5>,
        ][GATES.iter().position(|x| *x == g)?],
        Op::IsX(true) => fast_is_x_vec,
        Op::IsX(false) => fast_is_x_sul,
        Op::Truthy => fast_truthy,
        Op::Conv(0, false) => fast_conv_sul::<0>,
        Op::Conv(1, false) => fast_conv_sul::<1>,
        Op::Conv(_, false) => fast_conv_sul::<2>,
        Op::Conv(0, true) => fast_conv_vec::<0>,
        Op::Conv(1, true) => fast_conv_vec::<1>,
        Op::Conv(_, true) => fast_conv_vec::<2>,
        Op::ToBitVector => fast_to_bitvector,
        Op::Add {
            sub,
            signed,
            form: f,
        } => pick3_fast!(fast_add, sub, signed, form(f)),
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
                            ($c, false, 0) => fast_compare::<$c, false, 0> as FastFn,
                            ($c, false, 1) => fast_compare::<$c, false, 1> as FastFn,
                            ($c, false, _) => fast_compare::<$c, false, 2> as FastFn,
                            ($c, true, 0) => fast_compare::<$c, true, 0> as FastFn,
                            ($c, true, 1) => fast_compare::<$c, true, 1> as FastFn,
                            ($c, true, _) => fast_compare::<$c, true, 2> as FastFn,
                        )*
                        _ => return None,
                    }
                };
            }
            cs!(0, 1, 2, 3, 4, 5)
        }
        Op::QCompare {
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
                            ($c, false, 0) => fast_qcompare::<$c, false, 0> as FastFn,
                            ($c, false, 1) => fast_qcompare::<$c, false, 1> as FastFn,
                            ($c, false, _) => fast_qcompare::<$c, false, 2> as FastFn,
                            ($c, true, 0) => fast_qcompare::<$c, true, 0> as FastFn,
                            ($c, true, 1) => fast_qcompare::<$c, true, 1> as FastFn,
                            ($c, true, _) => fast_qcompare::<$c, true, 2> as FastFn,
                        )*
                        _ => return None,
                    }
                };
            }
            cs!(0, 1, 2, 3, 4, 5)
        }
        Op::MinMax {
            max,
            signed,
            form: f,
        } => pick3_fast!(fast_minmax, max, signed, form(f)),
        Op::Match => fast_match,
        Op::MatchEq { ne, vec } => match (ne, vec) {
            (false, false) => fast_match_eq::<false, false>,
            (false, true) => fast_match_eq::<false, true>,
            (true, false) => fast_match_eq::<true, false>,
            (true, true) => fast_match_eq::<true, true>,
        },
        Op::To01 => fast_to_01,
        Op::Div {
            kind,
            signed,
            form: f,
        } => divs!(fast_div, FastFn, kind, signed, form(f)),
        Op::Syn(Syn::Shift { right, signed }) => match (right, signed) {
            (false, false) => fast_syn_shift::<false, false>,
            (false, true) => fast_syn_shift::<false, true>,
            (true, false) => fast_syn_shift::<true, false>,
            (true, true) => fast_syn_shift::<true, true>,
        },
        Op::Syn(Syn::ToInt { signed: false }) => fast_syn_to_int::<false>,
        Op::Syn(Syn::ToInt { signed: true }) => fast_syn_to_int::<true>,
        Op::Syn(Syn::Unary { neg, abs, keep }) => {
            unary_pick!(fast_syn_unary, FastFn, neg, abs, keep)
        }
        Op::Syn(Syn::Conv {
            from_signed: Some(false),
            ..
        }) => fast_syn_conv_vec::<false>,
        Op::Syn(Syn::Conv {
            from_signed: Some(true),
            ..
        }) => fast_syn_conv_vec::<true>,
        Op::Syn(Syn::Conv {
            to_signed,
            from_signed: None,
        }) => {
            if to_signed {
                fast_syn_conv_int::<true>
            } else {
                fast_syn_conv_int::<false>
            }
        }
        Op::Syn(Syn::Oper { kind, l, r, keep }) => {
            oper_pick!(
                fast_oper,
                FastFn,
                oper_code(kind),
                opd_code(l),
                opd_code(r),
                keep
            )
        }
        Op::Mul { signed, form: f } => match (signed, form(f)) {
            (false, 0) => fast_mul::<false, 0>,
            (false, 1) => fast_mul::<false, 1>,
            (false, _) => fast_mul::<false, 2>,
            (true, 0) => fast_mul::<true, 0>,
            (true, 1) => fast_mul::<true, 1>,
            (true, _) => fast_mul::<true, 2>,
        },
        Op::ToInteger { signed: false } => fast_to_integer::<false>,
        Op::ToInteger { signed: true } => fast_to_integer::<true>,
        Op::ToUnsigned => fast_to_unsigned::<false>,
        Op::ToSigned => fast_to_unsigned::<true>,
        Op::Resize { signed: false } => fast_resize::<false>,
        Op::Resize { signed: true } => fast_resize::<true>,
        Op::ShiftFn(Shift::Sll) | Op::Shift(Shift::Sll) => fast_shift::<0>,
        Op::ShiftFn(Shift::Srl) | Op::Shift(Shift::Srl) => fast_shift::<1>,
        Op::ShiftFn(Shift::Sra) | Op::Shift(Shift::Sra) => fast_shift::<2>,
        Op::ShiftFn(Shift::Rol) | Op::Shift(Shift::Rol) => fast_shift::<3>,
        Op::ShiftFn(Shift::Ror) | Op::Shift(Shift::Ror) => fast_shift::<4>,
        _ => return None,
    })
}

#[cfg(test)]
#[path = "../tests/common/ieee_paths.rs"]
mod ieee_paths;

#[cfg(test)]
mod path_tests {
    use super::ieee_paths as paths;

    /// Every op `parse` returns is implemented, and has a one-word path exactly when its row
    /// says so: dropping an arm of `lookup_fast`, or a new op, fails here.
    #[test]
    fn one_word_paths_are_the_decided_ones() {
        let ops = paths::every_op();
        paths::assert_every_row_reached(&ops);
        for (op, key) in &ops {
            let sym = format!("yavs.ieee.{key}");
            assert!(super::lookup(&sym).is_some(), "{key}: no native");
            let row = paths::row(*op);
            assert_eq!(
                super::lookup_fast(&sym).is_some(),
                row.fast,
                "{key} ({op:?}): `lookup_fast` disagrees with row `{}` of tests/common/ieee_paths.rs",
                row.name
            );
        }
    }
}
