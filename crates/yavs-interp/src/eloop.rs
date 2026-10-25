//! Element loops in the IR (#757): a `for` loop whose body is index arithmetic, one `LGet` and the
//! `LSet` of it (a copy), or one `LSet` of a code that does not change (a fill). The interpreter
//! runs such a loop as one op (`pre::elem_loops`, #520); compiled code on native frames asks
//! [`run`] at the head of the loop for all its iterations but the last, which the generic code
//! then does (so a failing check, the exit and every register are the generic code's).
//!
//! [`elem_loops`] finds the loops in a function and flattens the body to a program of steps
//! (four words each: operation and three operands). Each value of a step is affine in the
//! iteration number, a pair (value at iteration 0, change per iteration); a value is a step's
//! result (`>= 0`), the loop variable ([`IND`]) or an input register (`-2 - k`, [`ElemLoop::inputs`]).

use yavs_ir::{BlockId, Func, Inst, IntOp, Reg, Width};

pub const IND: i64 = -1;
pub const MOV: i64 = 0;
pub const KONST: i64 = 1;
pub const ADD: i64 = 2;
pub const SUB: i64 = 3;
pub const MUL: i64 = 4;
pub const POS: i64 = 5;
pub const LEN: i64 = 6;
pub const LD: i64 = 7;
pub const ADDR: i64 = 8;
pub const GET: i64 = 9;
pub const SET: i64 = 10;

/// The most steps and input registers of a loop.
pub const MAX_STEPS: usize = 24;
pub const MAX_INPUTS: usize = 16;
/// The most reads of memory a loop does besides the vectors (descriptors, loaded words).
pub const MAX_READS: usize = 16;

/// A loop over the elements of logic vectors found in a function.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ElemLoop {
    /// The pc of the head block's first instruction: where compiled code asks for the iterations.
    pub head: u32,
    /// The loop variable, the bound it is compared with (equal: the last iteration) and the
    /// register it steps by (1 or -1 when the loop runs).
    pub ind: Reg,
    pub bound: Reg,
    pub step: Reg,
    /// The registers the program reads, which the loop does not write: their values, in this
    /// order, are the `inputs` of [`run`].
    pub inputs: Vec<Reg>,
    /// The steps, four words each.
    pub prog: Vec<i64>,
}

/// The element loops of `f`.
pub fn elem_loops(f: &Func) -> Vec<ElemLoop> {
    let n = f.reg_ty.len();
    let mut defs = vec![0u32; n + 1];
    let mut konst: Vec<Option<i64>> = vec![None; n + 1];
    for i in &f.code {
        for r in crate::inst_regs(f, i).1 {
            if let Some(d) = defs.get_mut(r.0 as usize) {
                *d += 1;
            }
        }
        if let Inst::Const { dst, v } = *i
            && let Some(k) = konst.get_mut(dst.0 as usize)
        {
            *k = Some(v);
        }
    }
    // The ways into each block.
    let mut preds = vec![0u32; f.code.len() + 1];
    for i in &f.code {
        let mut to = |b: BlockId| {
            if let Some(p) = f
                .blocks
                .get(b)
                .and_then(|k| preds.get_mut(k.start as usize))
            {
                *p += 1;
            }
        };
        match *i {
            Inst::Br { to: t } => to(t),
            Inst::CondBr { t, f: e, .. } => {
                to(t);
                to(e);
            }
            _ => {}
        }
    }
    let has_case = f.code.iter().any(|i| matches!(i, Inst::Case { .. }));
    let block = |b: BlockId| {
        f.blocks
            .get(b)
            .and_then(|b| f.code.get(b.start as usize..b.end as usize))
    };
    let mut out = Vec::new();
    for (tb, _) in f.blocks.iter() {
        // The tail: `Cmp Eq ind, bound`, `CondBr` to the exit or the step.
        let Some(code) = block(tb) else { continue };
        let Some(
            &[
                Inst::Cmp {
                    op: yavs_ir::CmpOp::Eq,
                    dst: c,
                    a: ca,
                    b: cb,
                },
                Inst::CondBr {
                    c: cc,
                    t: exit,
                    f: step_blk,
                },
            ],
        ) = code.last_chunk::<2>()
        else {
            continue;
        };
        let Some(
            &[
                Inst::Bin {
                    op: IntOp::Add,
                    dst: sd,
                    a: src,
                    b: step,
                },
                Inst::Br { to: head },
            ],
        ) = block(step_blk)
            .and_then(|b| <&[Inst; 2]>::try_from(b).ok())
            .map(|b| b.as_slice())
        else {
            continue;
        };
        if cc != c || sd != src || exit == step_blk {
            continue;
        }
        let (ind, bound) = if ca == sd {
            (ca, cb)
        } else if cb == sd {
            (cb, ca)
        } else {
            continue;
        };
        if ind == bound || step == ind || step == bound {
            continue;
        }
        // The body: the blocks from the head to the tail, each but the last ending in a `Br`
        // to the next, which nothing else reaches.
        let mut chain = vec![head];
        while *chain.last().unwrap_or(&head) != tb {
            let last = *chain.last().unwrap_or(&head);
            let Some(&Inst::Br { to }) = block(last).and_then(|b| b.last()) else {
                break;
            };
            if chain.len() >= 4 || chain.contains(&to) {
                break;
            }
            chain.push(to);
        }
        if chain.last() != Some(&tb) {
            continue;
        }
        let head_pc = match f.blocks.get(head) {
            Some(b) => b.start,
            None => continue,
        };
        let single = chain.iter().skip(1).all(|b| {
            f.blocks
                .get(*b)
                .and_then(|k| preds.get(k.start as usize))
                .is_some_and(|&p| p == 1 && !has_case)
        });
        if !single {
            continue;
        }
        // The straight-line instructions of the body: the blocks' without their `Br` and the tail's
        // `Cmp` and `CondBr`.
        let mut body: Vec<Inst> = Vec::new();
        for (k, b) in chain.iter().enumerate() {
            let Some(code) = block(*b) else { continue };
            let end = if k + 1 == chain.len() {
                code.len().saturating_sub(2)
            } else {
                code.len().saturating_sub(1)
            };
            body.extend_from_slice(&code[..end]);
        }
        let Some((inputs, prog)) = compile(f, &body, [ind, bound, step, c], &konst, &defs) else {
            continue;
        };
        out.push(ElemLoop {
            head: head_pc,
            ind,
            bound,
            step,
            inputs,
            prog,
        });
    }
    out
}

/// The program of a loop body, or `None` when the body is anything else, reads a register
/// before the iteration writes it (a value carried to the next one), or writes the loop
/// variable, the bound or the step.
fn compile(
    f: &Func,
    body: &[Inst],
    [ind, bound, step, cmp_dst]: [Reg; 4],
    konst: &[Option<i64>],
    defs: &[u32],
) -> Option<(Vec<Reg>, Vec<i64>)> {
    let mut written: Vec<Reg> = vec![cmp_dst];
    for i in body {
        written.extend(crate::inst_regs(f, i).1);
    }
    if written.contains(&ind) || written.contains(&bound) || written.contains(&step) {
        return None;
    }
    struct Cx {
        prog: Vec<i64>,
        steps: usize,
        /// Whether each step's result changes with the iteration, and whether it is a loaded code.
        dep: Vec<bool>,
        data: Vec<bool>,
        defined: Vec<(Reg, i64)>,
        inputs: Vec<Reg>,
        ind: Reg,
        written: Vec<Reg>,
    }
    impl Cx {
        fn rd(&mut self, r: Reg) -> Option<i64> {
            if r == self.ind {
                return Some(IND);
            }
            if let Some(&(_, s)) = self.defined.iter().rev().find(|d| d.0 == r) {
                return Some(s);
            }
            if self.written.contains(&r) {
                // Written later in the body: the value of the previous iteration.
                return None;
            }
            let k = match self.inputs.iter().position(|x| *x == r) {
                Some(k) => k,
                None => {
                    if self.inputs.len() >= MAX_INPUTS {
                        return None;
                    }
                    self.inputs.push(r);
                    self.inputs.len() - 1
                }
            };
            Some(-2 - k as i64)
        }
        fn dep(&self, v: i64) -> bool {
            match v {
                IND => true,
                s if s >= 0 => self.dep[s as usize],
                _ => false,
            }
        }
        fn data(&self, v: i64) -> bool {
            v >= 0 && self.data[v as usize]
        }
        fn push(&mut self, op: i64, a: [i64; 3], dep: bool, dst: Option<Reg>) -> Option<i64> {
            if self.steps >= MAX_STEPS {
                return None;
            }
            let s = self.steps as i64;
            self.steps += 1;
            self.prog.extend([op, a[0], a[1], a[2]]);
            self.dep.push(dep);
            self.data.push(false);
            if let Some(d) = dst {
                self.defined.push((d, s));
            }
            Some(s)
        }
        /// A register the loop does not write and whose value is not a loaded code.
        fn inv(&mut self, r: Reg) -> Option<i64> {
            let v = self.rd(r)?;
            (!self.dep(v) && !self.data(v)).then_some(v)
        }
        fn num(&mut self, r: Reg) -> Option<i64> {
            let v = self.rd(r)?;
            (!self.data(v)).then_some(v)
        }
    }
    let mut cx = Cx {
        prog: Vec::new(),
        steps: 0,
        dep: Vec::new(),
        data: Vec::new(),
        defined: Vec::new(),
        inputs: Vec::new(),
        ind,
        written,
    };
    let (mut gets, mut sets) = (0, 0);
    for i in body {
        match *i {
            Inst::Move { dst, src } => {
                let v = cx.rd(src)?;
                let (dep, data) = (cx.dep(v), cx.data(v));
                cx.push(MOV, [v, 0, 0], dep, Some(dst))?;
                if let Some(d) = cx.data.last_mut() {
                    *d = data;
                }
            }
            Inst::Const { dst, v } => {
                cx.push(KONST, [v, 0, 0], false, Some(dst))?;
            }
            Inst::Bin { op, dst, a, b } => {
                let o = match op {
                    IntOp::Add => ADD,
                    IntOp::Sub => SUB,
                    IntOp::Mul => MUL,
                    _ => return None,
                };
                let (x, y) = (cx.num(a)?, cx.num(b)?);
                let dep = cx.dep(x) || cx.dep(y);
                if o == MUL && cx.dep(x) && cx.dep(y) {
                    return None;
                }
                cx.push(o, [x, y, 0], dep, Some(dst))?;
            }
            Inst::Pos {
                dst,
                dope,
                dim,
                idx,
                ..
            } => {
                let d = cx.inv(dope)?;
                let x = cx.num(idx)?;
                let dep = cx.dep(x);
                cx.push(POS, [d, i64::from(dim), x], dep, Some(dst))?;
            }
            Inst::DopeLen { dst, dope, dim } => {
                let d = cx.inv(dope)?;
                cx.push(LEN, [d, i64::from(dim), 0], false, Some(dst))?;
            }
            Inst::Load { dst, addr, w } => {
                let a = cx.inv(addr)?;
                if !matches!(w, Width::B1 | Width::B4 | Width::B8) {
                    return None;
                }
                cx.push(LD, [a, 0, i64::from(w.bytes())], false, Some(dst))?;
            }
            Inst::Index {
                dst,
                base,
                pos,
                stride,
            } => {
                let a = cx.inv(base)?;
                let k = konst
                    .get(pos.0 as usize)
                    .copied()
                    .flatten()
                    .filter(|_| defs.get(pos.0 as usize) == Some(&1))?;
                let off = k.checked_mul(i64::from(stride))?;
                let off = i64::from(u32::try_from(off).ok()?);
                cx.push(ADDR, [a, off, 0], false, Some(dst))?;
            }
            Inst::LGet { dst, base, idx } => {
                gets += 1;
                let b = cx.inv(base)?;
                let x = cx.num(idx)?;
                let s = cx.push(GET, [b, x, 0], true, Some(dst))?;
                cx.data[s as usize] = true;
            }
            Inst::LSet { base, idx, v } => {
                sets += 1;
                let b = cx.inv(base)?;
                let x = cx.num(idx)?;
                let v = cx.rd(v)?;
                // A copy of the loaded code, or a fill with a code that does not change.
                if !(cx.data(v) || (v <= -2 && !cx.dep(v))) {
                    return None;
                }
                cx.push(SET, [b, x, v], false, None)?;
            }
            _ => return None,
        }
    }
    let copy = (0..cx.steps).any(|s| cx.prog[s * 4] == SET && cx.prog[s * 4 + 3] >= 0);
    (sets == 1 && gets == usize::from(copy)).then_some((cx.inputs, cx.prog))
}
