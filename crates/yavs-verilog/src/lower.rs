//! Process code (`exec::Op`, the trees `sem.rs` built) lowered to `yavs-ir` functions (#776):
//! one process function per distinct body, shared by every process whose body is the same up to
//! the nets it names (instances of one module with one parameter set). A function names nets by
//! the process's own numbers; [`ProcInfo::nets`] maps them to the domain's.
//!
//! A value of `w` bits is a run of `yavs_ir::vregs(w)` registers. No register lives across a
//! suspension (the interpreter does not keep them): what a process keeps past a wait (the value
//! of `a = #d b`, a `repeat` count) is in its frame, and the count of jumps back of one
//! activation starts again after each wait.

use std::collections::HashMap;
use std::sync::Arc;

use yavs_ir::{
    AddrSpace, BlockId, CmpOp, FOp, FUnOp, FuncId, FuncKind, Inst as I, IntOp, ProgramBuilder, Reg,
    RegTy, StmtKind, VBinOp, VCaseKind, VEdge, VUnOp, Width, vregs,
};

use crate::exec::{Arg, BOp, CaseKind, Code, E, Lhs, Op, RBOp, Start, UOp};
use crate::runtime::Edge;

/// The widest vector a process computes with.
pub(crate) const MAX_WIDTH: u32 = 1 << 16;

/// Why a process cannot be lowered.
pub(crate) enum LowerError {
    /// A vector wider than [`MAX_WIDTH`].
    Wide(u32),
    /// A broken invariant of the process code.
    Bug(String),
}

/// What a system task call of a function does (`Inst::VTask`).
#[derive(Clone, Debug)]
pub(crate) enum Task {
    /// `$display` (the flag: a line end) or `$write`, its arguments in order.
    Display(Vec<ArgSpec>, bool),
    Finish,
}

/// An argument of a `$display` task: what its registers hold.
#[derive(Clone, Debug)]
pub(crate) enum ArgSpec {
    Fmt(Vec<u8>),
    /// `w` bits, signed when the flag.
    Val(u32, bool),
    Real,
}

/// A process of the design: its function and what it names.
#[derive(Clone)]
pub(crate) struct ProcInfo {
    pub name: String,
    pub func: FuncId,
    /// The domain's net of each of the function's nets.
    pub nets: Vec<u32>,
    pub inst: Arc<crate::exec::Inst>,
}

/// The lowered processes of a design.
#[derive(Default)]
pub(crate) struct Lowerer {
    pub pb: ProgramBuilder,
    /// Functions by body (the process code with its own net numbers, and the time units).
    funcs: HashMap<String, FuncId>,
    pub tasks: Vec<Task>,
    pub procs: Vec<ProcInfo>,
}

impl Lowerer {
    /// Lowers `code` (nets of `widths`), or finds the function of a process with the same body.
    pub(crate) fn add(&mut self, code: &Code, widths: &[u32]) -> Result<(), LowerError> {
        let mut nets = Vec::new();
        let mut map = HashMap::new();
        let mut ops = code.ops.clone();
        for op in &mut ops {
            op_nets(op, &mut |n| {
                *n = *map.entry(*n).or_insert_with(|| {
                    nets.push(*n);
                    nets.len() as u32 - 1
                })
            });
        }
        for op in &mut ops {
            op_exprs(op, &mut |e| {
                fold(e);
            });
        }
        let local: Vec<u32> = nets.iter().map(|&n| widths[n as usize]).collect();
        if let Some(&w) = local.iter().find(|&&w| w > MAX_WIDTH) {
            return Err(LowerError::Wide(w));
        }
        let key = format!(
            "{} {} {local:?} {ops:?}",
            code.inst.unit_fs, code.inst.precision_fs
        );
        let func = match self.funcs.get(&key) {
            Some(&f) => f,
            None => {
                let f = self.pb.declare(&code.name, FuncKind::Process, None);
                lower_func(self, f, &ops, &local, &code.inst).map_err(LowerError::Bug)?;
                self.funcs.insert(key, f);
                f
            }
        };
        self.procs.push(ProcInfo {
            name: code.name.clone(),
            func,
            nets,
            inst: code.inst.clone(),
        });
        Ok(())
    }

    /// The number of distinct functions (tests).
    pub(crate) fn functions(&self) -> usize {
        self.funcs.len()
    }
}

/// The nets an op names, writable.
fn op_nets(op: &mut Op, f: &mut dyn FnMut(&mut u32)) {
    match op {
        Op::Set(l, e) | Op::Nba(l, e, None) => {
            lhs_nets(l, f);
            e_nets(e, f);
        }
        Op::Nba(l, e, Some(d)) => {
            lhs_nets(l, f);
            e_nets(e, f);
            e_nets(d, f);
        }
        Op::Hold(_, e) | Op::JmpIfNot(e, _) | Op::RepeatInit(_, e, _) | Op::Delay(e, _) => {
            e_nets(e, f)
        }
        Op::SetHeld(l, _) => lhs_nets(l, f),
        Op::Case { sel, items, .. } => {
            e_nets(sel, f);
            items.iter_mut().for_each(|(e, _)| e_nets(e, f));
        }
        Op::Wait(items) => items.iter_mut().for_each(|(n, _)| f(n)),
        Op::Display(args, _) => args.iter_mut().for_each(|a| match a {
            Arg::Val(e, _) | Arg::Real(e) => e_nets(e, f),
            Arg::Fmt(_) => {}
        }),
        Op::Jmp(_) | Op::RepeatTest(..) | Op::Finish | Op::Done => {}
    }
}

fn lhs_nets(l: &mut Lhs, f: &mut dyn FnMut(&mut u32)) {
    match l {
        Lhs::Part { net, start, .. } => {
            f(net);
            if let Start::Dyn { idx, .. } = start {
                e_nets(idx, f);
            }
        }
        Lhs::Cat(v) => v.iter_mut().for_each(|p| lhs_nets(p, f)),
    }
}

fn e_nets(e: &mut E, f: &mut dyn FnMut(&mut u32)) {
    match e {
        E::Var(n) => f(n),
        E::Sel { src, idx, .. } => {
            e_nets(src, f);
            e_nets(idx, f);
        }
        E::Un(_, a) | E::Ext(a, ..) | E::ToReal(a, _) | E::ToVec(a, _) | E::RUn(a) => e_nets(a, f),
        E::Bin(_, a, b, _) | E::RBin(_, a, b) | E::RCmp(_, a, b) => {
            e_nets(a, f);
            e_nets(b, f);
        }
        E::Cond(a, b, c) | E::RCond(a, b, c) => {
            e_nets(a, f);
            e_nets(b, f);
            e_nets(c, f);
        }
        E::Cat(v) => v.iter_mut().for_each(|p| e_nets(p, f)),
        E::K(_) | E::R(_) | E::Time(..) | E::RTime(_) => {}
    }
}

/// The trees of an op, writable.
fn op_exprs(op: &mut Op, f: &mut dyn FnMut(&mut E)) {
    match op {
        Op::Set(l, e) | Op::Nba(l, e, None) => {
            lhs_exprs(l, f);
            f(e);
        }
        Op::Nba(l, e, Some(d)) => {
            lhs_exprs(l, f);
            f(e);
            f(d);
        }
        Op::Hold(_, e) | Op::JmpIfNot(e, _) | Op::RepeatInit(_, e, _) | Op::Delay(e, _) => f(e),
        Op::SetHeld(l, _) => lhs_exprs(l, f),
        Op::Case { sel, items, .. } => {
            f(sel);
            items.iter_mut().for_each(|(e, _)| f(e));
        }
        Op::Display(args, _) => args.iter_mut().for_each(|a| match a {
            Arg::Val(e, _) | Arg::Real(e) => f(e),
            Arg::Fmt(_) => {}
        }),
        Op::Wait(_) | Op::Jmp(_) | Op::RepeatTest(..) | Op::Finish | Op::Done => {}
    }
}

fn lhs_exprs(l: &mut Lhs, f: &mut dyn FnMut(&mut E)) {
    match l {
        Lhs::Part {
            start: Start::Dyn { idx, .. },
            ..
        } => f(idx),
        Lhs::Part { .. } => {}
        Lhs::Cat(v) => v.iter_mut().for_each(|p| lhs_exprs(p, f)),
    }
}

/// Folds the subtrees of `e` that read no net and no time into constants (a parameter's
/// arithmetic, a constant concatenation): what it gives at run time, `exec::eval_const`'s.
/// Whether `e` is constant.
fn fold(e: &mut E) -> bool {
    let k = match e {
        E::K(_) | E::R(_) => return true,
        E::Var(_) | E::Time(..) | E::RTime(_) => return false,
        E::Sel { src, idx, .. } => fold(src) & fold(idx),
        E::Un(_, a) | E::Ext(a, ..) | E::ToReal(a, _) | E::ToVec(a, _) | E::RUn(a) => fold(a),
        E::Bin(_, a, b, _) | E::RBin(_, a, b) | E::RCmp(_, a, b) => fold(a) & fold(b),
        E::Cond(a, b, c) | E::RCond(a, b, c) => fold(a) & fold(b) & fold(c),
        E::Cat(v) => v.iter_mut().fold(true, |k, p| fold(p) & k),
    };
    if k {
        *e = match crate::exec::eval_const(e) {
            crate::sem::Const::V(v, _) => E::K(v),
            crate::sem::Const::R(r) => E::R(r),
        };
    }
    k
}

/// `10^k` femtoseconds as `k` (time units and precisions are powers of ten, 19.8).
fn exp10(fs: u64) -> Result<u8, String> {
    let k = fs.max(1).ilog10();
    match 10u64.checked_pow(k) == Some(fs) {
        true => Ok(k as u8),
        false => Err(format!("the time unit {fs} fs is not a power of ten")),
    }
}

/// Lowers the process function `f`.
fn lower_func(
    l: &mut Lowerer,
    f: FuncId,
    ops: &[Op],
    widths: &[u32],
    inst: &crate::exec::Inst,
) -> Result<(), String> {
    let unit = exp10(inst.unit_fs)?;
    let prec = exp10(inst.precision_fs)?;
    // The tasks are the lowerer's, the builder borrows its program: lowered apart, then
    // appended.
    let mut pb = std::mem::take(&mut l.pb);
    let mut tasks = std::mem::take(&mut l.tasks);
    let r = {
        let mut fb = pb.define(f);
        // One statement for the whole process: Verilog has no spans in the program.
        fb.locate(StmtKind::Process, yavs_diag::Span::default());
        let cnt = fb.reg(RegTy::I64);
        let blocks: Vec<BlockId> = (0..=ops.len()).map(|_| fb.block()).collect();
        let mut t = Lower {
            fb,
            widths,
            blocks,
            cnt,
            slots: HashMap::new(),
            unit,
            prec,
            tasks: &mut tasks,
        };
        t.fb.emit(I::Const { dst: cnt, v: 0 });
        t.br(0);
        let r = (0..ops.len()).try_for_each(|pc| t.op(pc, &ops[pc]));
        // Past the last op: a process that ran off its code ends.
        t.fb.at(t.blocks[ops.len()]);
        t.fb.emit(I::Ret { v: None });
        t.fb.flag(yavs_ir::FuncFlags::MAY_SUSPEND);
        t.fb.finish();
        r
    };
    l.pb = pb;
    l.tasks = tasks;
    r
}

struct Lower<'a, 'b> {
    fb: yavs_ir::FuncBuilder<'a>,
    widths: &'a [u32],
    blocks: Vec<BlockId>,
    cnt: Reg,
    slots: HashMap<u32, (u32, u32)>,
    unit: u8,
    prec: u8,
    tasks: &'b mut Vec<Task>,
}

/// A value: its first register and its width.
type V = (Reg, u32);

impl Lower<'_, '_> {
    fn br(&mut self, pc: usize) {
        self.fb.emit(I::Br {
            to: self.blocks[pc],
        });
    }

    /// A fresh run of registers for `w` bits.
    fn run(&mut self, w: u32) -> Reg {
        let r = self.fb.reg(RegTy::I64);
        for k in 1..vregs(w) {
            let x = self.fb.reg(RegTy::I64);
            debug_assert_eq!(x.0, r.0 + k);
        }
        r
    }

    fn int(&mut self, v: i64) -> Reg {
        self.fb.int(v)
    }

    /// The truth of `c` (`Inst::VTruth`) compared with `want` (1 or 0).
    fn truth_is(&mut self, t: Reg, want: i64) -> Reg {
        let k = self.int(want);
        let dst = self.fb.reg(RegTy::I64);
        self.fb.emit(I::Cmp {
            op: CmpOp::Eq,
            dst,
            a: t,
            b: k,
        });
        dst
    }

    fn truth(&mut self, e: &E) -> Result<Reg, String> {
        let (a, w) = self.expr(e)?;
        let dst = self.fb.reg(RegTy::I64);
        self.fb.emit(I::VTruth { w, dst, a });
        Ok(dst)
    }

    /// The three ways of a condition: blocks for true, false and unknown, `fb` in none of them.
    fn three_way(&mut self, t: Reg) -> [BlockId; 3] {
        let [one, zero, x, no] = [(); 4].map(|_| self.fb.block());
        let is1 = self.truth_is(t, 1);
        self.fb.emit(I::CondBr {
            c: is1,
            t: one,
            f: no,
        });
        self.fb.at(no);
        let is0 = self.truth_is(t, 0);
        self.fb.emit(I::CondBr {
            c: is0,
            t: zero,
            f: x,
        });
        [one, zero, x]
    }

    fn moves(&mut self, dst: Reg, src: Reg, n: u32) {
        for k in 0..n {
            self.fb.emit(I::Move {
                dst: Reg(dst.0 + k),
                src: Reg(src.0 + k),
            });
        }
    }

    fn konst(&mut self, v: &crate::value::V4) -> V {
        let w = v.width();
        let dst = self.run(w);
        let (a, b) = v.planes();
        for (k, &x) in a.iter().chain(b).enumerate() {
            self.fb.emit(I::Const {
                dst: Reg(dst.0 + k as u32),
                v: x as i64,
            });
        }
        (dst, w)
    }

    /// A vector expression into a fresh run.
    fn expr(&mut self, e: &E) -> Result<V, String> {
        Ok(match e {
            E::K(v) => self.konst(v),
            E::Var(n) => {
                let w = self.widths[*n as usize];
                let dst = self.run(w);
                self.fb.emit(I::VRead { net: *n, w, dst });
                (dst, w)
            }
            E::Sel {
                src,
                idx,
                idx_signed,
                desc,
                adj,
                n,
            } => {
                let (s, w) = self.expr(src)?;
                let at = self.index(idx, *idx_signed, *desc, *adj)?;
                let dst = self.run(*n);
                self.fb.emit(I::VSel {
                    w,
                    n: *n,
                    dst,
                    src: s,
                    at,
                });
                (dst, *n)
            }
            E::Un(op, a) => {
                let (a, w) = self.expr(a)?;
                let op = match op {
                    UOp::Not => VUnOp::Not,
                    UOp::Neg => VUnOp::Neg,
                    UOp::LNot => VUnOp::LNot,
                    UOp::RedAnd => VUnOp::RedAnd,
                    UOp::RedNand => VUnOp::RedNand,
                    UOp::RedOr => VUnOp::RedOr,
                    UOp::RedNor => VUnOp::RedNor,
                    UOp::RedXor => VUnOp::RedXor,
                    UOp::RedXnor => VUnOp::RedXnor,
                    UOp::Clog2 => VUnOp::Clog2,
                };
                let rw = op.width(w);
                let dst = self.run(rw);
                self.fb.emit(I::VUn { op, w, dst, a });
                (dst, rw)
            }
            E::Bin(op, a, b, signed) => {
                let (a, w) = self.expr(a)?;
                let (b, wb) = self.expr(b)?;
                let op = bin_op(*op);
                let rw = if op.is_bit() { 1 } else { w };
                let dst = self.run(rw);
                self.fb.emit(I::VBin {
                    op,
                    signed: *signed,
                    w,
                    wb,
                    dst,
                    a,
                    b,
                });
                (dst, rw)
            }
            E::Ext(a, w, signed) => {
                let (a, wa) = self.expr(a)?;
                let dst = self.run(*w);
                self.fb.emit(I::VExt {
                    signed: *signed,
                    w: *w,
                    wa,
                    dst,
                    a,
                });
                (dst, *w)
            }
            E::Cond(c, a, b) => {
                let t = self.truth(c)?;
                let [one, zero, x] = self.three_way(t);
                let join = self.fb.block();
                // Both arms have the width of the expression (sem.rs).
                self.fb.at(one);
                let (ra, w) = self.expr(a)?;
                let dst = self.run(w);
                self.moves(dst, ra, vregs(w));
                self.fb.emit(I::Br { to: join });
                self.fb.at(zero);
                let (rb, _) = self.expr(b)?;
                self.moves(dst, rb, vregs(w));
                self.fb.emit(I::Br { to: join });
                self.fb.at(x);
                let (ra, _) = self.expr(a)?;
                let (rb, _) = self.expr(b)?;
                self.fb.emit(I::VMerge {
                    w,
                    dst,
                    a: ra,
                    b: rb,
                });
                self.fb.emit(I::Br { to: join });
                self.fb.at(join);
                (dst, w)
            }
            E::Cat(parts) => {
                let mut it = parts.iter();
                let first = it.next().ok_or("an empty concatenation")?;
                let mut acc = self.expr(first)?;
                for p in it {
                    let (lo, wlo) = self.expr(p)?;
                    let w = acc.1 + wlo;
                    let dst = self.run(w);
                    self.fb.emit(I::VCat {
                        whi: acc.1,
                        wlo,
                        dst,
                        hi: acc.0,
                        lo,
                    });
                    acc = (dst, w);
                }
                acc
            }
            E::Time(unit, w) => {
                let dst = self.run(*w);
                self.fb.emit(I::VTime {
                    unit: exp10(*unit)?,
                    w: *w,
                    dst,
                });
                (dst, *w)
            }
            E::ToVec(r, w) => {
                let a = self.real(r)?;
                let dst = self.run(*w);
                self.fb.emit(I::VFromReal { w: *w, dst, a });
                (dst, *w)
            }
            E::RCmp(op, a, b) => {
                let (a, b) = (self.real(a)?, self.real(b)?);
                let c = self.fb.reg(RegTy::I64);
                let (op, not) = match op {
                    BOp::Lt => (CmpOp::Lt, false),
                    BOp::Le => (CmpOp::Le, false),
                    BOp::Gt => (CmpOp::Gt, false),
                    BOp::Ge => (CmpOp::Ge, false),
                    BOp::Eq => (CmpOp::Eq, false),
                    // `!=` is true for a NaN, which compares false with everything.
                    _ => (CmpOp::Eq, true),
                };
                self.fb.emit(I::FCmp { op, dst: c, a, b });
                let dst = self.run(1);
                match not {
                    true => self.fb.emit(I::Bool {
                        op: yavs_ir::BoolOp::Not,
                        dst,
                        a: c,
                        b: c,
                    }),
                    false => self.fb.emit(I::Move { dst, src: c }),
                }
                self.fb.emit(I::Const {
                    dst: Reg(dst.0 + 1),
                    v: 0,
                });
                (dst, 1)
            }
            E::R(_) | E::ToReal(..) | E::RTime(_) | E::RUn(_) | E::RBin(..) | E::RCond(..) => {
                return Err("a real expression where a vector is wanted".into());
            }
        })
    }

    /// A real expression into an `F64` register.
    fn real(&mut self, e: &E) -> Result<Reg, String> {
        let dst = self.fb.reg(RegTy::F64);
        match e {
            E::R(r) => self.fb.emit(I::ConstF { dst, v: *r }),
            E::ToReal(a, signed) => {
                let (a, w) = self.expr(a)?;
                self.fb.emit(I::VToReal {
                    signed: *signed,
                    w,
                    dst,
                    a,
                });
            }
            E::RTime(unit) => self.fb.emit(I::VRTime {
                unit: exp10(*unit)?,
                dst,
            }),
            E::RUn(a) => {
                let a = self.real(a)?;
                self.fb.emit(I::FUn {
                    op: FUnOp::Neg,
                    dst,
                    a,
                });
            }
            E::RBin(op, a, b) => {
                let (a, b) = (self.real(a)?, self.real(b)?);
                let op = match op {
                    RBOp::Add => FOp::Add,
                    RBOp::Sub => FOp::Sub,
                    RBOp::Mul => FOp::Mul,
                    RBOp::Div => FOp::Div,
                    RBOp::Pow => FOp::Pow,
                };
                self.fb.emit(I::VRBin { op, dst, a, b });
            }
            E::RCond(c, a, b) => {
                let t = self.truth(c)?;
                let [one, zero, x] = self.three_way(t);
                let join = self.fb.block();
                self.fb.at(one);
                let ra = self.real(a)?;
                self.fb.emit(I::Move { dst, src: ra });
                self.fb.emit(I::Br { to: join });
                self.fb.at(zero);
                let rb = self.real(b)?;
                self.fb.emit(I::Move { dst, src: rb });
                self.fb.emit(I::Br { to: join });
                // An unknown condition gives 0.0 when the arms differ (Table 5-21 has no real
                // row).
                self.fb.at(x);
                let (ra, rb) = (self.real(a)?, self.real(b)?);
                let eq = self.fb.reg(RegTy::I64);
                self.fb.emit(I::FCmp {
                    op: CmpOp::Eq,
                    dst: eq,
                    a: ra,
                    b: rb,
                });
                let z = self.fb.reg(RegTy::F64);
                self.fb.emit(I::ConstF { dst: z, v: 0.0 });
                self.fb.emit(I::Select {
                    dst,
                    c: eq,
                    a: ra,
                    b: z,
                });
                self.fb.emit(I::Br { to: join });
                self.fb.at(join);
            }
            _ => return Err("a vector expression where a real is wanted".into()),
        }
        Ok(dst)
    }

    /// The first element a select from the index value of `idx` starts at.
    fn index(&mut self, idx: &E, signed: bool, desc: bool, adj: i64) -> Result<Reg, String> {
        let (i, w) = self.expr(idx)?;
        let dst = self.fb.reg(RegTy::I64);
        self.fb.emit(I::VIndex {
            signed,
            desc,
            w,
            adj,
            dst,
            idx: i,
        });
        Ok(dst)
    }

    /// The writes of value `v` to `lhs`: every target is found before the first write (9.2).
    fn targets(
        &mut self,
        lhs: &Lhs,
        v: V,
        lo: u32,
        out: &mut Vec<(u32, Reg, Reg, u32)>,
    ) -> Result<(), String> {
        match lhs {
            Lhs::Part { net, start, n, .. } => {
                let at = match start {
                    Start::Fixed(k) => self.int(*k as i64),
                    Start::Dyn {
                        idx,
                        signed,
                        desc,
                        adj,
                    } => self.index(idx, *signed, *desc, *adj)?,
                };
                let src = if lo == 0 && *n == v.1 {
                    v.0
                } else {
                    let lo = self.int(lo as i64);
                    let dst = self.run(*n);
                    self.fb.emit(I::VSel {
                        w: v.1,
                        n: *n,
                        dst,
                        src: v.0,
                        at: lo,
                    });
                    dst
                };
                out.push((*net, at, src, *n));
            }
            Lhs::Cat(parts) => {
                let mut lo = lo + lhs.width();
                for p in parts {
                    lo -= p.width();
                    self.targets(p, v, lo, out)?;
                }
            }
        }
        Ok(())
    }

    fn write(&mut self, lhs: &Lhs, v: V, delay: Option<Reg>) -> Result<(), String> {
        let mut t = Vec::new();
        self.targets(lhs, v, 0, &mut t)?;
        for (net, at, src, n) in t {
            self.fb.emit(match delay {
                None => I::VWrite { net, n, src, at },
                Some(delay) => I::VNba {
                    net,
                    n,
                    src,
                    at,
                    delay,
                },
            });
        }
        Ok(())
    }

    /// A delay in femtoseconds.
    fn delay(&mut self, e: &E, real: bool) -> Result<Reg, String> {
        let dst = self.fb.reg(RegTy::I64);
        if real {
            let a = self.real(e)?;
            self.fb.emit(I::VDelayR {
                unit: self.unit,
                prec: self.prec,
                dst,
                a,
            });
        } else {
            let (a, w) = self.expr(e)?;
            self.fb.emit(I::VDelay {
                unit: self.unit,
                w,
                dst,
                a,
            });
        }
        Ok(dst)
    }

    /// The frame slot of key `k`, of `w` bits.
    fn slot(&mut self, k: u32, w: u32) -> u32 {
        let fb = &mut self.fb;
        self.slots
            .entry(k)
            .or_insert_with(|| (fb.slot(8 * vregs(w), 8), w))
            .0
    }

    fn store(&mut self, off: u32, src: Reg, n: u32) {
        for k in 0..n {
            let addr = self.fb.addr(AddrSpace::Frame, off + 8 * k);
            self.fb.emit(I::Store {
                addr,
                src: Reg(src.0 + k),
                w: Width::B8,
            });
        }
    }

    fn load(&mut self, off: u32, dst: Reg, n: u32) {
        for k in 0..n {
            let addr = self.fb.addr(AddrSpace::Frame, off + 8 * k);
            self.fb.emit(I::Load {
                dst: Reg(dst.0 + k),
                addr,
                w: Width::B8,
            });
        }
    }

    /// After a suspension: a new activation counts its jumps back from 0.
    fn suspend(&mut self, pc: usize) {
        self.fb.emit(I::Suspend { resume: 0 });
        self.fb.emit(I::Const {
            dst: self.cnt,
            v: 0,
        });
        self.br(pc + 1);
    }

    fn op(&mut self, pc: usize, op: &Op) -> Result<(), String> {
        self.fb.at(self.blocks[pc]);
        match op {
            Op::Set(lhs, e) => {
                let v = self.expr(e)?;
                self.write(lhs, v, None)?;
                self.br(pc + 1);
            }
            Op::Nba(lhs, e, d) => {
                let v = self.expr(e)?;
                let d = match d {
                    Some(d) => self.delay(d, is_real(d))?,
                    None => self.int(0),
                };
                self.write(lhs, v, Some(d))?;
                self.br(pc + 1);
            }
            Op::Hold(k, e) => {
                let (r, w) = self.expr(e)?;
                let off = self.slot(*k, w);
                self.store(off, r, vregs(w));
                self.br(pc + 1);
            }
            Op::SetHeld(lhs, k) => {
                let (off, w) = *self.slots.get(k).ok_or("a held value never kept")?;
                let r = self.run(w);
                self.load(off, r, vregs(w));
                self.write(lhs, (r, w), None)?;
                self.br(pc + 1);
            }
            Op::Jmp(t) => {
                if (*t as usize) < pc {
                    self.fb.emit(I::VBack { cnt: self.cnt });
                }
                self.br(*t as usize);
            }
            Op::JmpIfNot(c, t) => {
                let tr = self.truth(c)?;
                let is1 = self.truth_is(tr, 1);
                self.fb.emit(I::CondBr {
                    c: is1,
                    t: self.blocks[pc + 1],
                    f: self.blocks[*t as usize],
                });
            }
            Op::Case {
                sel,
                kind,
                items,
                default,
            } => {
                let (s, w) = self.expr(sel)?;
                let kind = match kind {
                    CaseKind::Case => VCaseKind::Case,
                    CaseKind::Casez => VCaseKind::Casez,
                    CaseKind::Casex => VCaseKind::Casex,
                };
                for (e, t) in items {
                    let (b, _) = self.expr(e)?;
                    let m = self.fb.reg(RegTy::I64);
                    self.fb.emit(I::VCase {
                        kind,
                        w,
                        dst: m,
                        a: s,
                        b,
                    });
                    let next = self.fb.block();
                    self.fb.emit(I::CondBr {
                        c: m,
                        t: self.blocks[*t as usize],
                        f: next,
                    });
                    self.fb.at(next);
                }
                self.br(*default as usize);
            }
            Op::RepeatInit(k, e, signed) => {
                let (a, w) = self.expr(e)?;
                let n = self.fb.reg(RegTy::I64);
                self.fb.emit(I::VCount {
                    signed: *signed,
                    w,
                    dst: n,
                    a,
                });
                let off = self.slot(*k, 64);
                let addr = self.fb.addr(AddrSpace::Frame, off);
                self.fb.emit(I::Store {
                    addr,
                    src: n,
                    w: Width::B8,
                });
                self.br(pc + 1);
            }
            Op::RepeatTest(k, end) => {
                let off = self.slot(*k, 64);
                let addr = self.fb.addr(AddrSpace::Frame, off);
                let n = self.fb.reg(RegTy::I64);
                self.fb.emit(I::Load {
                    dst: n,
                    addr,
                    w: Width::B8,
                });
                let zero = self.int(0);
                let done = self.fb.reg(RegTy::I64);
                self.fb.emit(I::Cmp {
                    op: CmpOp::Le,
                    dst: done,
                    a: n,
                    b: zero,
                });
                let more = self.fb.block();
                self.fb.emit(I::CondBr {
                    c: done,
                    t: self.blocks[*end as usize],
                    f: more,
                });
                self.fb.at(more);
                let one = self.int(1);
                let m = self.fb.reg(RegTy::I64);
                self.fb.emit(I::Bin {
                    op: IntOp::Sub,
                    dst: m,
                    a: n,
                    b: one,
                });
                self.fb.emit(I::Store {
                    addr,
                    src: m,
                    w: Width::B8,
                });
                self.br(pc + 1);
            }
            Op::Wait(items) => {
                for &(net, e) in items {
                    let edge = match e {
                        Edge::Any => VEdge::Any,
                        Edge::Pos => VEdge::Pos,
                        Edge::Neg => VEdge::Neg,
                    };
                    self.fb.emit(I::VWaitOn { net, edge });
                }
                self.suspend(pc);
            }
            Op::Delay(e, real) => {
                let fs = self.delay(e, *real)?;
                self.fb.emit(I::VWaitDelay { fs });
                self.suspend(pc);
            }
            Op::Display(args, newline) => {
                let mut regs = Vec::new();
                let mut spec = Vec::new();
                for a in args {
                    match a {
                        Arg::Fmt(f) => spec.push(ArgSpec::Fmt(f.clone())),
                        Arg::Val(e, s) => {
                            let (r, w) = self.expr(e)?;
                            regs.extend((0..vregs(w)).map(|k| Reg(r.0 + k)));
                            spec.push(ArgSpec::Val(w, *s));
                        }
                        Arg::Real(e) => {
                            regs.push(self.real(e)?);
                            spec.push(ArgSpec::Real);
                        }
                    }
                }
                let args = self.fb.regs(&regs);
                self.tasks.push(Task::Display(spec, *newline));
                let id = self.tasks.len() as u32 - 1;
                self.fb.emit(I::VTask { id, args });
                self.br(pc + 1);
            }
            Op::Finish => {
                self.tasks.push(Task::Finish);
                let id = self.tasks.len() as u32 - 1;
                let args = self.fb.regs(&[]);
                self.fb.emit(I::VTask { id, args });
                self.fb.emit(I::Ret { v: None });
            }
            Op::Done => self.fb.emit(I::Ret { v: None }),
        }
        Ok(())
    }
}

fn is_real(e: &E) -> bool {
    matches!(
        e,
        E::R(_) | E::ToReal(..) | E::RTime(_) | E::RUn(_) | E::RBin(..) | E::RCond(..)
    )
}

fn bin_op(op: BOp) -> VBinOp {
    match op {
        BOp::Add => VBinOp::Add,
        BOp::Sub => VBinOp::Sub,
        BOp::Mul => VBinOp::Mul,
        BOp::Div => VBinOp::Div,
        BOp::Rem => VBinOp::Rem,
        BOp::Pow(s) => VBinOp::Pow(s),
        BOp::And => VBinOp::And,
        BOp::Or => VBinOp::Or,
        BOp::Xor => VBinOp::Xor,
        BOp::Xnor => VBinOp::Xnor,
        BOp::Shl => VBinOp::Shl,
        BOp::Shr => VBinOp::Shr,
        BOp::Ashr => VBinOp::Ashr,
        BOp::Lt => VBinOp::Lt,
        BOp::Le => VBinOp::Le,
        BOp::Gt => VBinOp::Gt,
        BOp::Ge => VBinOp::Ge,
        BOp::Eq => VBinOp::Eq,
        BOp::Ne => VBinOp::Ne,
        BOp::CaseEq => VBinOp::CaseEq,
        BOp::CaseNe => VBinOp::CaseNe,
        BOp::LAnd => VBinOp::LAnd,
        BOp::LOr => VBinOp::LOr,
    }
}

#[cfg(test)]
mod tests {
    //! Random expression trees of every operator and width: the lowered code on the interpreter
    //! against `exec::eval`, the tree's own evaluation.

    use std::sync::{Arc, Mutex};

    use proptest::prelude::*;
    use yavs_kernel::{Builder, Config, Hook, Observer, ProcFlags, ProcId, Time};

    use crate::elab::Design;
    use crate::exec::{Arg, BOp, Code, E, Env, Inst, Lhs, Op, Start, UOp, eval};
    use crate::value::V4;

    struct Rng(u64);

    impl Rng {
        fn next(&mut self) -> u64 {
            self.0 ^= self.0 << 13;
            self.0 ^= self.0 >> 7;
            self.0 ^= self.0 << 17;
            self.0
        }
        fn below(&mut self, n: u64) -> u64 {
            self.next() % n
        }
        fn width(&mut self) -> u32 {
            [1, 1, 2, 3, 4, 7, 8, 16, 31, 32, 33, 63, 64, 65, 100, 130][self.below(16) as usize]
        }
        /// A value with some x and z bits now and then.
        fn value(&mut self, w: u32) -> V4 {
            let n = w.div_ceil(64) as usize;
            let a: Vec<u64> = (0..n).map(|_| self.next()).collect();
            let b: Vec<u64> = (0..n)
                .map(|_| match self.below(3) {
                    0 => self.next() & self.next() & self.next(),
                    _ => 0,
                })
                .collect();
            V4::from_planes(w, &a, &b)
        }
    }

    /// The nets of a test: initial values, found by width.
    struct Gen {
        r: Rng,
        nets: Vec<V4>,
    }

    impl Gen {
        fn net(&mut self, w: u32) -> E {
            if let Some(i) = self.nets.iter().position(|v| v.width() == w)
                && self.r.below(2) == 0
            {
                return E::Var(i as u32);
            }
            let v = self.r.value(w);
            self.nets.push(v);
            E::Var(self.nets.len() as u32 - 1)
        }

        /// A tree of width `w`.
        fn expr(&mut self, w: u32, depth: u32) -> E {
            let b = |e: E| Box::new(e);
            if depth == 0 {
                return match self.r.below(2) {
                    0 => E::K(self.r.value(w)),
                    _ => self.net(w),
                };
            }
            let d = depth - 1;
            let s = self.r.below(2) == 0;
            match self.r.below(11) {
                0 => E::K(self.r.value(w)),
                1 => self.net(w),
                2 => {
                    let op = [UOp::Not, UOp::Neg][self.r.below(2) as usize];
                    E::Un(op, b(self.expr(w, d)))
                }
                3 | 4 => {
                    let ops = [
                        BOp::Add,
                        BOp::Sub,
                        BOp::Mul,
                        BOp::Div,
                        BOp::Rem,
                        BOp::And,
                        BOp::Or,
                        BOp::Xor,
                        BOp::Xnor,
                    ];
                    let op = ops[self.r.below(ops.len() as u64) as usize];
                    E::Bin(op, b(self.expr(w, d)), b(self.expr(w, d)), s)
                }
                5 => {
                    let ops = [BOp::Shl, BOp::Shr, BOp::Ashr, BOp::Pow(s)];
                    let op = ops[self.r.below(4) as usize];
                    let wb = [1, 3, 5, 8, 32][self.r.below(5) as usize];
                    E::Bin(op, b(self.expr(w, d)), b(self.expr(wb, d)), s)
                }
                6 if w == 1 => {
                    let k = self.r.width();
                    match self.r.below(3) {
                        0 => {
                            let ops = [
                                BOp::Lt,
                                BOp::Le,
                                BOp::Gt,
                                BOp::Ge,
                                BOp::Eq,
                                BOp::Ne,
                                BOp::CaseEq,
                                BOp::CaseNe,
                                BOp::LAnd,
                                BOp::LOr,
                            ];
                            let op = ops[self.r.below(ops.len() as u64) as usize];
                            E::Bin(op, b(self.expr(k, d)), b(self.expr(k, d)), s)
                        }
                        1 => {
                            let ops = [
                                UOp::LNot,
                                UOp::RedAnd,
                                UOp::RedNand,
                                UOp::RedOr,
                                UOp::RedNor,
                                UOp::RedXor,
                                UOp::RedXnor,
                            ];
                            let op = ops[self.r.below(ops.len() as u64) as usize];
                            E::Un(op, b(self.expr(k, d)))
                        }
                        _ => E::RCmp(
                            [BOp::Lt, BOp::Eq, BOp::Ne][self.r.below(3) as usize],
                            b(E::ToReal(b(self.expr(k, d)), s)),
                            b(E::ToReal(b(self.expr(k, d)), s)),
                        ),
                    }
                }
                6 if w == 32 => E::Un(
                    UOp::Clog2,
                    b({
                        let k = self.r.width();
                        self.expr(k, d)
                    }),
                ),
                7 => E::Ext(
                    b({
                        let k = self.r.width();
                        self.expr(k, d)
                    }),
                    w,
                    s,
                ),
                8 => E::Cond(
                    b({
                        let k = self.r.width();
                        self.expr(k, d)
                    }),
                    b(self.expr(w, d)),
                    b(self.expr(w, d)),
                ),
                9 if w >= 2 => {
                    let lo = 1 + self.r.below(w as u64 - 1) as u32;
                    E::Cat(vec![self.expr(w - lo, d), self.expr(lo, d)])
                }
                10 => {
                    let k = self.r.width();
                    let wi = [2, 4, 8, 32][self.r.below(4) as usize];
                    E::Sel {
                        src: b(self.expr(k, d)),
                        idx: b(self.expr(wi, d)),
                        idx_signed: s,
                        desc: self.r.below(2) == 0,
                        adj: self.r.below(8) as i64 - 2,
                        n: w,
                    }
                }
                _ => E::ToVec(
                    b(E::RBin(
                        crate::exec::RBOp::Mul,
                        b(E::ToReal(
                            b({
                                let k = self.r.width();
                                self.expr(k, d)
                            }),
                            s,
                        )),
                        b(E::R(0.75)),
                    )),
                    w,
                ),
            }
        }
    }

    struct Nets<'a>(&'a [V4]);

    impl Env for Nets<'_> {
        fn net(&self, n: u32) -> V4 {
            self.0[n as usize].clone()
        }
        fn now_fs(&self) -> u64 {
            0
        }
    }

    #[derive(Clone, Default)]
    struct Out(Arc<Mutex<Vec<u8>>>);

    impl Observer for Out {
        fn output(&mut self, _h: &mut Hook<'_>, _p: ProcId, bytes: &[u8]) {
            self.0.lock().unwrap().extend_from_slice(bytes);
        }
    }

    /// Each tree is shown with `%b` by an `initial` block, and assigned to a net of its own and
    /// shown again (the write path).
    fn run(trees: &[E], nets: &[V4]) -> String {
        let mut d = Design::default();
        let inst = Arc::new(Inst {
            path: "top".into(),
            unit_fs: 1_000_000,
            precision_fs: 1_000,
        });
        d.nets_mut().extend(nets.iter().map(V4::to_logic));
        let mut ops = Vec::new();
        for e in trees {
            let w = eval(e, &Nets(nets)).width();
            let net = d.nets_mut().len() as u32;
            d.nets_mut().push(V4::xs(w).to_logic());
            let fmt = Arg::Fmt(b"%b ".to_vec());
            ops.push(Op::Display(vec![fmt, Arg::Val(e.clone(), false)], false));
            let lhs = Lhs::Part {
                net,
                start: Start::Fixed(0),
                n: w,
            };
            ops.push(Op::Set(lhs, e.clone()));
            let fmt = Arg::Fmt(b"%b\n".to_vec());
            ops.push(Op::Display(vec![fmt, Arg::Val(E::Var(net), false)], false));
        }
        ops.push(Op::Done);
        let code = Code {
            name: "top.initial1".into(),
            ops,
            inst,
        };
        d.add_code(&code).unwrap();
        let mut b = Builder::new();
        let dom = d.domain(&|_| unreachable!("no ports"), &[]);
        b.process("top.verilog", ProcFlags::default(), &[], dom);
        let mut sim = b.build(Config::default()).unwrap();
        let out = Out::default();
        sim.subscribe(Box::new(out.clone()));
        sim.run_until(Time::ns(1)).unwrap();
        String::from_utf8_lossy(&out.0.lock().unwrap()).into_owned()
    }

    fn bin(v: &V4) -> String {
        (0..v.width()).rev().map(|i| v.get(i).char()).collect()
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(64))]
        #[test]
        fn lowered_expressions_match_the_trees(seed in 1u64..u64::MAX) {
            let mut g = Gen { r: Rng(seed), nets: Vec::new() };
            let trees: Vec<E> = (0..8).map(|_| {
                let w = g.r.width();
                g.expr(w, 4)
            }).collect();
            let want: String = trees
                .iter()
                .map(|e| {
                    let v = bin(&eval(e, &Nets(&g.nets)));
                    format!("{v} {v}\n")
                })
                .collect();
            prop_assert_eq!(run(&trees, &g.nets), want);
        }
    }

    /// Processes with the same body share their function, whatever nets they name.
    #[test]
    fn instances_share_functions() {
        let mut d = Design::default();
        let inst = Arc::new(Inst {
            path: "top".into(),
            unit_fs: 1_000_000,
            precision_fs: 1_000,
        });
        for k in 0..10u32 {
            d.nets_mut().push(V4::xs(8).to_logic());
            d.nets_mut().push(V4::xs(8).to_logic());
            let e = E::Bin(
                BOp::Add,
                Box::new(E::Var(2 * k)),
                Box::new(E::K(V4::from_u64(8, 1))),
                false,
            );
            let lhs = Lhs::Part {
                net: 2 * k + 1,
                start: Start::Fixed(0),
                n: 8,
            };
            let code = Code {
                name: format!("top.u{k}.assign1"),
                ops: vec![
                    Op::Set(lhs, e),
                    Op::Wait(vec![(2 * k, crate::runtime::Edge::Any)]),
                    Op::Jmp(0),
                ],
                inst: inst.clone(),
            };
            d.add_code(&code).unwrap();
        }
        assert_eq!(d.functions(), 1);
        d.prepare().unwrap();
    }
}
