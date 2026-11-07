//! The run-time form of elaborated Verilog: fixed-width expression trees (`E`, built by `sem.rs`),
//! assignment targets (`Lhs`), the linear code of a process (`Op`) and its interpreter (`Run`),
//! which a domain process body drives from one wait to the next.

use std::sync::Arc;

use yavs_kernel::{SimError, Time};

use crate::runtime::{Edge, Exec, NetId, Wait};
use crate::sem::{Const, real_to_i64};
use crate::value::{Bit, V4};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UOp {
    Not,
    Neg,
    LNot,
    RedAnd,
    RedNand,
    RedOr,
    RedNor,
    RedXor,
    RedXnor,
    Clog2,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum BOp {
    Add,
    Sub,
    Mul,
    Div,
    Rem,
    /// `**`; the flag: the exponent is signed.
    Pow(bool),
    And,
    Or,
    Xor,
    Xnor,
    Shl,
    Shr,
    Ashr,
    Lt,
    Le,
    Gt,
    Ge,
    Eq,
    Ne,
    CaseEq,
    CaseNe,
    LAnd,
    LOr,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RBOp {
    Add,
    Sub,
    Mul,
    Div,
    Pow,
}

/// An expression whose operands are at the width their operator takes. Vector nodes give a
/// `V4`, real nodes (`R`, `ToReal`, `RUn`, `RBin`, `RCond`) an `f64`.
#[derive(Clone, Debug)]
pub(crate) enum E {
    K(V4),
    R(f64),
    /// A whole net or variable.
    Var(u32),
    /// `n` elements of `src` from the element of index value `idx`: `idx - adj`
    /// when `desc`, else `adj - idx`. Elements outside `src` and an unknown index read as x.
    Sel {
        src: Box<E>,
        idx: Box<E>,
        idx_signed: bool,
        desc: bool,
        adj: i64,
        n: u32,
    },
    Un(UOp, Box<E>),
    /// The flag: the operation is signed.
    Bin(BOp, Box<E>, Box<E>, bool),
    /// Truncated or extended to the width, sign-extended with the flag.
    Ext(Box<E>, u32, bool),
    Cond(Box<E>, Box<E>, Box<E>),
    /// Most significant part first.
    Cat(Vec<E>),
    /// `$time` (64 bits) or `$stime` (32): the time in the module's unit (femtoseconds given).
    Time(u64, u32),
    ToReal(Box<E>, bool),
    /// `$realtime`: the time in the module's unit (femtoseconds given), not rounded.
    RTime(u64),
    /// A real as a two's complement integer of the width, rounded (4.8.2).
    ToVec(Box<E>, u32),
    RUn(Box<E>),
    RBin(RBOp, Box<E>, Box<E>),
    RCmp(BOp, Box<E>, Box<E>),
    RCond(Box<E>, Box<E>, Box<E>),
}

/// What an expression reads.
pub(crate) trait Env {
    fn net(&self, n: u32) -> V4;
    fn now_fs(&self) -> u64;
}

struct NoNets;

impl Env for NoNets {
    fn net(&self, _: u32) -> V4 {
        unreachable!("a constant expression reads no net (sem.rs refuses them)")
    }
    fn now_fs(&self) -> u64 {
        0
    }
}

pub(crate) fn eval_const(e: &E) -> Const {
    if is_real(e) {
        Const::R(real(e, &NoNets))
    } else {
        // A constant's sign is its expression's; sem.rs passes it with the type.
        Const::V(eval(e, &NoNets), false)
    }
}

fn is_real(e: &E) -> bool {
    matches!(
        e,
        E::R(_) | E::ToReal(..) | E::RTime(_) | E::RUn(_) | E::RBin(..) | E::RCond(..)
    )
}

fn sel_start(idx: &V4, signed: bool, desc: bool, adj: i64) -> Option<i64> {
    let k = idx.to_i64(signed)?;
    Some(if desc { k - adj } else { adj - k })
}

pub(crate) fn eval(e: &E, env: &dyn Env) -> V4 {
    match e {
        E::K(v) => v.clone(),
        E::Var(n) => env.net(*n),
        E::Sel {
            src,
            idx,
            idx_signed,
            desc,
            adj,
            n,
            ..
        } => {
            let v = eval(src, env);
            match sel_start(&eval(idx, env), *idx_signed, *desc, *adj) {
                Some(lo) => v.slice(lo, *n),
                None => V4::xs(*n),
            }
        }
        E::Un(op, a) => {
            let a = eval(a, env);
            match op {
                UOp::Not => a.not(),
                UOp::Neg => a.neg(),
                UOp::LNot => a.lnot(),
                UOp::RedAnd => a.red_and(),
                UOp::RedNand => a.red_and().not(),
                UOp::RedOr => a.red_or(),
                UOp::RedNor => a.red_or().not(),
                UOp::RedXor => a.red_xor(),
                UOp::RedXnor => a.red_xor().not(),
                UOp::Clog2 => clog2(&a),
            }
        }
        E::Bin(op, a, b, s) => {
            let (a, b) = (eval(a, env), eval(b, env));
            let s = *s;
            match op {
                BOp::Add => a.add(&b),
                BOp::Sub => a.sub(&b),
                BOp::Mul => a.mul(&b),
                BOp::Div => a.div(&b, s),
                BOp::Rem => a.rem(&b, s),
                BOp::Pow(es) => a.pow(&b, s, *es),
                BOp::And => a.and(&b),
                BOp::Or => a.or(&b),
                BOp::Xor => a.xor(&b),
                BOp::Xnor => a.xnor(&b),
                BOp::Shl => a.shl(&b),
                BOp::Shr => a.shr(&b, false),
                BOp::Ashr => a.shr(&b, s),
                BOp::Lt => a.lt(&b, s),
                BOp::Le => a.le(&b, s),
                BOp::Gt => a.gt(&b, s),
                BOp::Ge => a.ge(&b, s),
                BOp::Eq => a.eq(&b),
                BOp::Ne => a.ne(&b),
                BOp::CaseEq => a.case_eq(&b),
                BOp::CaseNe => a.case_eq(&b).not(),
                BOp::LAnd => a.land(&b),
                BOp::LOr => a.lor(&b),
            }
        }
        E::Ext(a, w, s) => eval(a, env).ext(*w, *s),
        E::Cond(c, a, b) => match eval(c, env).truth() {
            Bit::One => eval(a, env),
            Bit::Zero => eval(b, env),
            _ => eval(a, env).merge(&eval(b, env)),
        },
        E::Cat(parts) => {
            let mut it = parts.iter().map(|p| eval(p, env));
            let first = it.next().expect("a concatenation has a part");
            it.fold(first, |hi, lo| V4::concat(&hi, &lo))
        }
        E::Time(unit, w) => {
            let t = env.now_fs();
            // Rounded to the module's unit (17.7.1).
            V4::from_u64(64, (t + unit / 2) / unit).ext(*w, false)
        }
        E::ToVec(r, w) => {
            let r = real(r, env);
            V4::from_i64(64, real_to_i64(r)).ext(*w, true)
        }
        E::RCmp(op, a, b) => {
            let (a, b) = (real(a, env), real(b, env));
            let r = match op {
                BOp::Lt => a < b,
                BOp::Le => a <= b,
                BOp::Gt => a > b,
                BOp::Ge => a >= b,
                BOp::Eq => a == b,
                _ => a != b,
            };
            V4::from_u64(1, r as u64)
        }
        E::R(_) | E::ToReal(..) | E::RTime(_) | E::RUn(_) | E::RBin(..) | E::RCond(..) => {
            unreachable!("a real node where a vector is wanted (sem.rs)")
        }
    }
}

pub(crate) fn real(e: &E, env: &dyn Env) -> f64 {
    match e {
        E::R(r) => *r,
        E::ToReal(a, s) => to_f64(&eval(a, env), *s),
        E::RTime(unit) => env.now_fs() as f64 / *unit as f64,
        E::RUn(a) => -real(a, env),
        E::RBin(op, a, b) => {
            let (a, b) = (real(a, env), real(b, env));
            match op {
                RBOp::Add => a + b,
                RBOp::Sub => a - b,
                RBOp::Mul => a * b,
                RBOp::Div => a / b,
                RBOp::Pow => a.powf(b),
            }
        }
        E::RCond(c, a, b) => match eval(c, env).truth() {
            Bit::Zero => real(b, env),
            // An unknown condition gives 0.0 when the arms differ (Table 5-21 has no real row).
            Bit::One => real(a, env),
            _ => {
                let (a, b) = (real(a, env), real(b, env));
                if a == b { a } else { 0.0 }
            }
        },
        _ => unreachable!("a vector node where a real is wanted (sem.rs)"),
    }
}

/// A vector as a real; an x or z bit reads as 0 (4.8.2).
pub(crate) fn to_f64(v: &V4, signed: bool) -> f64 {
    let neg = signed && v.msb() == Bit::One && v.is_known();
    let m = if neg { v.neg() } else { v.clone() };
    let mut r = 0.0f64;
    for i in (0..m.width()).rev() {
        r = r * 2.0 + if m.get(i) == Bit::One { 1.0 } else { 0.0 };
    }
    if neg { -r } else { r }
}

/// `$clog2`: the ceiling of log2 of the operand as unsigned, 0 for 0 and 1 (17.11.1).
fn clog2(v: &V4) -> V4 {
    if !v.is_known() {
        return V4::xs(32);
    }
    let top = (0..v.width()).rev().find(|&i| v.get(i) == Bit::One);
    let r = match top {
        None => 0,
        Some(t) => {
            let pow2 = (0..t).all(|i| v.get(i) == Bit::Zero);
            if pow2 { t } else { t + 1 }
        }
    };
    V4::from_u64(32, r as u64)
}

/// Where an assignment writes.
#[derive(Clone, Debug)]
pub(crate) enum Lhs {
    /// `n` elements of net `net` (`w` elements) from a fixed element, or from the index
    /// value of `idx` as in `E::Sel`.
    Part {
        net: u32,
        w: u32,
        start: Start,
        n: u32,
    },
    /// Most significant part first.
    Cat(Vec<Lhs>),
}

#[derive(Clone, Debug)]
pub(crate) enum Start {
    Fixed(u32),
    Dyn {
        idx: E,
        signed: bool,
        desc: bool,
        adj: i64,
    },
}

impl Lhs {
    pub(crate) fn width(&self) -> u32 {
        match self {
            Lhs::Part { n, .. } => *n,
            Lhs::Cat(v) => v.iter().map(Lhs::width).sum(),
        }
    }

    /// The writes of a value of this target's width: (net, first element, bits `lo .. lo + n` of
    /// the value). Elements outside the net, and all of a select with an unknown index, are not
    /// written (9.2 and 5.2.1).
    fn targets(&self, env: &dyn Env, lo: u32, out: &mut Vec<(u32, u32, u32, u32)>) {
        match self {
            Lhs::Part { net, w, start, n } => {
                let at = match start {
                    Start::Fixed(k) => *k as i64,
                    Start::Dyn {
                        idx,
                        signed,
                        desc,
                        adj,
                    } => match sel_start(&eval(idx, env), *signed, *desc, *adj) {
                        Some(k) => k,
                        None => return,
                    },
                };
                let (a, b) = (at.max(0), (at + *n as i64).min(*w as i64));
                if a < b {
                    out.push((*net, a as u32, lo + (a - at) as u32, (b - a) as u32));
                }
            }
            Lhs::Cat(parts) => {
                let mut lo = lo + self.width();
                for p in parts {
                    lo -= p.width();
                    p.targets(env, lo, out);
                }
            }
        }
    }

    /// The nets an `@*` or a continuous assignment reads to find the target (the indices).
    pub(crate) fn reads(&self, out: &mut Vec<u32>) {
        match self {
            Lhs::Part {
                start: Start::Dyn { idx, .. },
                ..
            } => reads(idx, out),
            Lhs::Part { .. } => {}
            Lhs::Cat(v) => v.iter().for_each(|p| p.reads(out)),
        }
    }

    pub(crate) fn nets(&self, out: &mut Vec<(u32, Option<(u32, u32)>)>) {
        match self {
            Lhs::Part {
                net,
                start: Start::Fixed(k),
                n,
                ..
            } => out.push((*net, Some((*k, *n)))),
            Lhs::Part { net, .. } => out.push((*net, None)),
            Lhs::Cat(v) => v.iter().for_each(|p| p.nets(out)),
        }
    }
}

/// The nets an expression reads.
pub(crate) fn reads(e: &E, out: &mut Vec<u32>) {
    match e {
        E::Var(n) => {
            if !out.contains(n) {
                out.push(*n)
            }
        }
        E::Sel { src, idx, .. } => {
            reads(src, out);
            reads(idx, out);
        }
        E::Un(_, a) | E::Ext(a, ..) | E::ToReal(a, _) | E::ToVec(a, _) | E::RUn(a) => reads(a, out),
        E::Bin(_, a, b, _) | E::RBin(_, a, b) | E::RCmp(_, a, b) => {
            reads(a, out);
            reads(b, out);
        }
        E::Cond(a, b, c) | E::RCond(a, b, c) => {
            reads(a, out);
            reads(b, out);
            reads(c, out);
        }
        E::Cat(v) => v.iter().for_each(|p| reads(p, out)),
        E::K(_) | E::R(_) | E::Time(..) | E::RTime(_) => {}
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CaseKind {
    Case,
    Casez,
    Casex,
}

/// An argument of `$display` and `$write`.
#[derive(Clone, Debug)]
pub(crate) enum Arg {
    /// A string literal: a format.
    Fmt(Vec<u8>),
    /// A value; the flag: signed.
    Val(E, bool),
    Real(E),
}

/// One instruction of a process.
#[derive(Clone, Debug)]
pub(crate) enum Op {
    /// Blocking assignment.
    Set(Lhs, E),
    /// Non-blocking assignment, with an intra-assignment delay.
    Nba(Lhs, E, Option<E>),
    /// Keep a value for later (an intra-assignment delay of a blocking assignment).
    Hold(u32, E),
    SetHeld(Lhs, u32),
    Jmp(u32),
    JmpIfNot(E, u32),
    Case {
        sel: E,
        kind: CaseKind,
        items: Vec<(E, u32)>,
        default: u32,
    },
    /// `repeat`: the count into a slot, then a test that jumps out when it is used up.
    RepeatInit(u32, E, bool),
    RepeatTest(u32, u32),
    Wait(Vec<(u32, Edge)>),
    /// `#d`; the flag: a real delay.
    Delay(E, bool),
    Display(Vec<Arg>, bool),
    Finish,
    Done,
}

/// What every process of an instance shares.
#[derive(Debug)]
pub(crate) struct Inst {
    /// `%m`.
    pub path: String,
    pub unit_fs: u64,
    pub precision_fs: u64,
}

#[derive(Debug)]
pub(crate) struct Code {
    pub name: String,
    pub ops: Vec<Op>,
    pub inst: Arc<Inst>,
}

/// Backward jumps one activation may take before the run stops with an error.
const LOOP_LIMIT: u64 = 1 << 26;

/// A process at its next instruction.
pub(crate) struct Run {
    code: Arc<Code>,
    pc: usize,
    slots: Vec<Option<V4>>,
    /// `%t` prints in these femtoseconds (the smallest precision of the design, 17.3.2).
    tf_fs: u64,
}

struct X<'e, 'a, 'c>(&'e Exec<'a, 'c>);

impl Env for X<'_, '_, '_> {
    fn net(&self, n: u32) -> V4 {
        V4::from_logic(self.0.get(NetId(n)))
    }
    fn now_fs(&self) -> u64 {
        self.0.now().as_fs()
    }
}

impl Run {
    pub(crate) fn new(code: Arc<Code>, tf_fs: u64) -> Run {
        Run {
            code,
            pc: 0,
            slots: Vec::new(),
            tf_fs,
        }
    }

    fn write(
        &self,
        x: &mut Exec<'_, '_>,
        lhs: &Lhs,
        v: &V4,
        nba: Option<Time>,
    ) -> Result<(), SimError> {
        let mut t = Vec::new();
        lhs.targets(&X(x), 0, &mut t);
        for (net, at, lo, n) in t {
            let part = v.slice(lo as i64, n).to_logic();
            match nba {
                None => x.set_at(NetId(net), at, &part)?,
                Some(d) => x.nba(NetId(net), at, part, d)?,
            }
        }
        Ok(())
    }

    fn delay(&self, x: &Exec<'_, '_>, e: &E, is_real: bool) -> Result<Time, SimError> {
        let inst = &self.code.inst;
        let fs = if is_real {
            let r = real(e, &X(x));
            if r.is_nan() || r < 0.0 {
                return Err(x.error(format!(
                    "the delay {r} in `{}` is negative (IEEE 1364-2005 9.7.1)",
                    self.code.name
                )));
            }
            // Rounded to the precision of the module (19.8).
            let p = inst.precision_fs as f64;
            ((r * inst.unit_fs as f64 / p).round() * p) as u128
        } else {
            let v = eval(e, &X(x));
            // An x or z delay is zero (9.7.1).
            match v.is_known() {
                false => 0,
                true => match v.to_u64() {
                    Some(u) => u as u128 * inst.unit_fs as u128,
                    None => u128::MAX,
                },
            }
        };
        u64::try_from(fs).ok().and_then(Time::fs).ok_or_else(|| {
            x.error(format!(
                "the delay in `{}` is beyond the largest time",
                self.code.name
            ))
        })
    }

    /// Runs from where the process suspended to its next wait.
    pub(crate) fn step(&mut self, x: &mut Exec<'_, '_>) -> Result<Wait, SimError> {
        let code = self.code.clone();
        let mut back = 0u64;
        loop {
            let pc = self.pc;
            self.pc += 1;
            match &code.ops[pc] {
                Op::Set(lhs, e) => {
                    let v = eval(e, &X(x));
                    self.write(x, lhs, &v, None)?;
                }
                Op::Nba(lhs, e, d) => {
                    let v = eval(e, &X(x));
                    let d = match d {
                        Some(d) => self.delay(x, d, is_real(d))?,
                        None => Time::ZERO,
                    };
                    self.write(x, lhs, &v, Some(d))?;
                }
                Op::Hold(k, e) => {
                    let v = eval(e, &X(x));
                    let k = *k as usize;
                    if self.slots.len() <= k {
                        self.slots.resize(k + 1, None);
                    }
                    self.slots[k] = Some(v);
                }
                Op::SetHeld(lhs, k) => {
                    let v = self.slots[*k as usize].take().expect("a held value");
                    self.write(x, lhs, &v, None)?;
                }
                Op::Jmp(t) => {
                    if (*t as usize) < pc {
                        back += 1;
                        if back > LOOP_LIMIT {
                            return Err(x.error(format!(
                                "the Verilog process `{}` looped {LOOP_LIMIT} times without waiting",
                                code.name
                            )));
                        }
                    }
                    self.pc = *t as usize;
                }
                Op::JmpIfNot(c, t) => {
                    if eval(c, &X(x)).truth() != Bit::One {
                        self.pc = *t as usize;
                    }
                }
                Op::Case {
                    sel,
                    kind,
                    items,
                    default,
                } => {
                    let s = eval(sel, &X(x));
                    self.pc = *default as usize;
                    for (e, t) in items {
                        if case_match(&s, &eval(e, &X(x)), *kind) {
                            self.pc = *t as usize;
                            break;
                        }
                    }
                }
                Op::RepeatInit(k, e, signed) => {
                    let v = eval(e, &X(x));
                    // An unknown count runs no iteration (9.6).
                    let n = v.to_i64(*signed).unwrap_or(0).max(0);
                    let k = *k as usize;
                    if self.slots.len() <= k {
                        self.slots.resize(k + 1, None);
                    }
                    self.slots[k] = Some(V4::from_i64(64, n));
                }
                Op::RepeatTest(k, end) => {
                    let s = self.slots[*k as usize].as_mut().expect("a repeat count");
                    let n = s.to_i64(true).expect("a known count");
                    if n <= 0 {
                        self.pc = *end as usize;
                    } else {
                        *s = V4::from_i64(64, n - 1);
                    }
                }
                Op::Wait(items) => {
                    return Ok(Wait::On(
                        items.iter().map(|&(n, e)| (NetId(n), e)).collect(),
                    ));
                }
                Op::Delay(e, r) => {
                    let d = self.delay(x, e, *r)?;
                    return Ok(if d == Time::ZERO {
                        Wait::Zero
                    } else {
                        Wait::Delay(d)
                    });
                }
                Op::Display(args, newline) => {
                    let mut out = format(args, &X(x), &code.inst, self.tf_fs)
                        .map_err(|m| x.error(format!("{m} in `{}`", code.name)))?;
                    if *newline {
                        out.push(b'\n');
                    }
                    x.output(&out);
                }
                Op::Finish => {
                    x.finish();
                    return Ok(Wait::Done);
                }
                Op::Done => {
                    self.pc = pc;
                    return Ok(Wait::Done);
                }
            }
        }
    }
}

/// `case` compares with `===`; `casez` ignores bits that are z (or `?`) on either side,
/// `casex` bits that are x or z (9.5.1).
fn case_match(s: &V4, e: &V4, kind: CaseKind) -> bool {
    if kind == CaseKind::Case {
        return s == e;
    }
    (0..s.width()).all(|i| {
        let (a, b) = (s.get(i), e.get(i));
        let care = |c: Bit| match kind {
            CaseKind::Casez => c == Bit::Z,
            _ => matches!(c, Bit::X | Bit::Z),
        };
        care(a) || care(b) || a == b
    })
}

/// Decimal digits of a known vector.
fn decimal(v: &V4, signed: bool) -> String {
    let neg = signed && v.msb() == Bit::One;
    let m = if neg { v.neg() } else { v.clone() };
    let (a, _) = m.planes();
    let mut limbs = a.to_vec();
    let mut digits = Vec::new();
    loop {
        let mut rem = 0u128;
        for l in limbs.iter_mut().rev() {
            let cur = (rem << 64) | *l as u128;
            *l = (cur / 10) as u64;
            rem = cur % 10;
        }
        digits.push(b'0' + rem as u8);
        if limbs.iter().all(|&l| l == 0) {
            break;
        }
    }
    if neg {
        digits.push(b'-');
    }
    digits.reverse();
    String::from_utf8(digits).expect("digits")
}

/// `%d`: x when every bit is x, X when some are; z and Z likewise (17.1.1.4).
fn dec(v: &V4, signed: bool, pad: Option<usize>) -> String {
    let w = v.width();
    let xs = (0..w).filter(|&i| v.get(i) == Bit::X).count() as u32;
    let zs = (0..w).filter(|&i| v.get(i) == Bit::Z).count() as u32;
    let s = if xs == w {
        "x".into()
    } else if zs == w {
        "z".into()
    } else if xs > 0 {
        "X".into()
    } else if zs > 0 {
        "Z".into()
    } else {
        decimal(v, signed)
    };
    // The natural width: the digits of the largest value of the width (and a sign).
    let natural = || {
        if signed {
            decimal(&V4::zeros(w).not().shr(&V4::from_u64(8, 1), false), false).len() + 1
        } else {
            decimal(&V4::ones(w), false).len()
        }
    };
    let width = pad.unwrap_or_else(natural);
    format!("{s:>width$}")
}

/// `%b`, `%o`, `%h`: one digit per group of `k` bits; a group of all x is x, with some x X,
/// z likewise (17.1.1.4).
fn radix(v: &V4, k: u32, strip: bool) -> String {
    let w = v.width();
    let mut out = Vec::new();
    let mut g = 0;
    while g < w {
        let bits: Vec<Bit> = (g..(g + k).min(w)).map(|i| v.get(i)).collect();
        let n = bits.len();
        let xs = bits.iter().filter(|&&b| b == Bit::X).count();
        let zs = bits.iter().filter(|&&b| b == Bit::Z).count();
        let c = if xs == n {
            'x'
        } else if zs == n {
            'z'
        } else if xs > 0 {
            'X'
        } else if zs > 0 {
            'Z'
        } else {
            let d = bits
                .iter()
                .enumerate()
                .fold(0, |a, (i, &b)| a | ((b == Bit::One) as u32) << i);
            std::char::from_digit(d, 16).expect("a digit")
        };
        out.push(c);
        g += k;
    }
    out.reverse();
    let s: String = out.into_iter().collect();
    if strip {
        let t = s.trim_start_matches('0');
        if t.is_empty() { "0".into() } else { t.into() }
    } else {
        s
    }
}

/// The text of `$display` and `$write` arguments (17.1).
fn format(args: &[Arg], env: &dyn Env, inst: &Inst, tf_fs: u64) -> Result<Vec<u8>, String> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match &args[i] {
            Arg::Val(e, s) => {
                out.extend(dec(&eval(e, env), *s, None).bytes());
                i += 1;
            }
            Arg::Real(e) => {
                out.extend(format!("{}", real(e, env)).bytes());
                i += 1;
            }
            Arg::Fmt(f) => {
                i += 1;
                let mut j = 0;
                while j < f.len() {
                    if f[j] != b'%' {
                        out.push(f[j]);
                        j += 1;
                        continue;
                    }
                    j += 1;
                    let ds = j;
                    while j < f.len() && f[j].is_ascii_digit() {
                        j += 1;
                    }
                    let pad: Option<usize> = std::str::from_utf8(&f[ds..j])
                        .ok()
                        .and_then(|d| d.parse().ok());
                    let Some(&c) = f.get(j) else {
                        return Err("a format ends with `%`".into());
                    };
                    j += 1;
                    let c = c.to_ascii_lowercase();
                    if c == b'%' {
                        out.push(b'%');
                        continue;
                    }
                    if c == b'm' {
                        out.extend(inst.path.bytes());
                        continue;
                    }
                    let Some(a) = args.get(i) else {
                        return Err(format!("the format `%{}` has no argument", c as char));
                    };
                    i += 1;
                    let pad0 = pad == Some(0);
                    let fit = |s: String| match pad {
                        Some(n) if n > 0 => format!("{s:>n$}"),
                        _ => s,
                    };
                    let text = match (c, a) {
                        (b'e' | b'f' | b'g', Arg::Real(e)) => {
                            let r = real(e, env);
                            fit(match c {
                                b'f' => format!("{r:.6}"),
                                b'e' => c_exp(r),
                                _ => format!("{r}"),
                            })
                        }
                        (_, Arg::Real(e)) => fit(format!("{}", real(e, env))),
                        (_, Arg::Fmt(s)) if c == b's' => fit(String::from_utf8_lossy(s).into()),
                        (_, Arg::Fmt(s)) => {
                            let v = crate::sem::string_value(s);
                            fit(dec(&v, false, if pad0 { Some(0) } else { None }))
                        }
                        (_, Arg::Val(e, s)) => {
                            let v = eval(e, env);
                            match c {
                                b'd' => dec(&v, *s, pad.or(if pad0 { Some(0) } else { None })),
                                b'b' => fit(radix(&v, 1, pad0)),
                                b'o' => fit(radix(&v, 3, pad0)),
                                b'h' | b'x' => fit(radix(&v, 4, pad0)),
                                b'c' => {
                                    (v.slice(0, 8).to_u64().unwrap_or(0) as u8 as char).to_string()
                                }
                                b's' => {
                                    let mut s = String::new();
                                    let n = v.width().div_ceil(8);
                                    for k in (0..n).rev() {
                                        let byte =
                                            v.slice(8 * k as i64, 8).to_u64().unwrap_or(0) as u8;
                                        // ponytail: leading NUL bytes are left out; 17.1.1.7
                                        // detail of how they show is not checked.
                                        if byte != 0 || !s.is_empty() {
                                            s.push(byte as char);
                                        }
                                    }
                                    fit(s)
                                }
                                b't' => {
                                    // In the units of the smallest precision (17.3.2).
                                    let t = match v.to_u64() {
                                        Some(u) => {
                                            let [lo, hi] = mul_div(u, inst.unit_fs, tf_fs);
                                            decimal(&V4::from_words(128, &[lo, hi]), false)
                                        }
                                        None => dec(&v, false, Some(0)),
                                    };
                                    let width = pad.unwrap_or(20);
                                    format!("{t:>width$}")
                                }
                                b'v' | b'l' | b'u' | b'z' => {
                                    return Err(format!(
                                        "the format `%{}` is not implemented",
                                        c as char
                                    ));
                                }
                                _ => {
                                    return Err(format!(
                                        "`%{}` is not a format of IEEE 1364-2005 17.1.1.3",
                                        c as char
                                    ));
                                }
                            }
                        }
                    };
                    out.extend(text.bytes());
                }
            }
        }
    }
    Ok(out)
}

/// `u * a / b` as two words.
fn mul_div(u: u64, a: u64, b: u64) -> [u64; 2] {
    let t = u as u128 * a as u128 / b as u128;
    [t as u64, (t >> 64) as u64]
}

/// `%e` as C prints it: one digit, six decimals, a signed exponent of two digits.
fn c_exp(r: f64) -> String {
    let s = format!("{r:.6e}");
    match s.split_once('e') {
        Some((m, e)) => {
            let n: i32 = e.parse().unwrap_or(0);
            format!("{m}e{}{:02}", if n < 0 { '-' } else { '+' }, n.abs())
        }
        None => s,
    }
}
