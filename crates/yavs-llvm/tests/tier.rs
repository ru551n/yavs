//! The LLVM tier against the interpreter (rule 5): hand-written IR run both ways.
#![cfg(feature = "llvm")]

use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use yavs_diag::{FileId, Span};
use yavs_interp::{Bindings, InstData, Runtime, Tier};
use yavs_ir::*;
use yavs_llvm::{LlvmTier, OptLevel};

/// Counts the entries into compiled code.
struct Counting(LlvmTier, AtomicUsize);

impl Tier for Counting {
    fn run(
        &self,
        func: FuncId,
        pc: u32,
        regs: &mut Vec<u64>,
        step: &mut dyn FnMut(&mut Vec<u64>, u32) -> bool,
    ) -> Option<u32> {
        let r = self.0.run(func, pc, regs, step);
        if r.is_some() {
            self.1.fetch_add(1, Ordering::Relaxed);
        }
        r
    }
}

/// A program compiled once, called in the interpreter and with the LLVM tier.
struct Both {
    prog: Arc<Program>,
    tier: Arc<Counting>,
}

impl Both {
    fn new(prog: Program) -> Both {
        let tier = yavs_llvm::compile(&prog, OptLevel::O2).unwrap();
        assert!(tier.notes.is_empty(), "{:?}", tier.notes);
        Both {
            prog: Arc::new(prog),
            tier: Arc::new(Counting(tier, AtomicUsize::new(0))),
        }
    }

    fn call(&self, f: FuncId, args: &[u64], llvm: bool) -> Result<u64, String> {
        let rt = Runtime::new(self.prog.clone(), Bindings::default()).unwrap();
        if llvm {
            assert!(rt.set_tier(self.tier.clone()));
        }
        yavs_interp::call(rt, f, Arc::new(InstData::default()), args)
            .map(|v| v.expect("a value"))
            .map_err(|e| e.to_string())
    }

    /// Both results (value or runtime error text) and the entries into compiled code.
    fn both(&self, f: FuncId, args: &[u64]) -> (Result<u64, String>, Result<u64, String>, usize) {
        self.tier.1.store(0, Ordering::Relaxed);
        let (a, b) = (self.call(f, args, false), self.call(f, args, true));
        (a, b, self.tier.1.load(Ordering::Relaxed))
    }
}

fn stmt(fb: &mut FuncBuilder<'_>) {
    fb.stmt(StmtKind::Return, Span::new(FileId(0), 0, 1));
}

/// `function f(a, b) return a op b` with an optional range check of the result.
fn binop(op: IntOp, range: Option<(i64, i64)>) -> (Program, FuncId) {
    let mut pb = ProgramBuilder::new();
    let f = pb.declare("f", FuncKind::Function, None);
    let msg = pb.check_msg("result", Dir::To);
    let mut fb = pb.define(f);
    let sa = fb.param("a", Width::B8, ParamClass::Value, Mode::In, None);
    let sb = fb.param("b", Width::B8, ParamClass::Value, Mode::In, None);
    stmt(&mut fb);
    let (pa, pbr) = (fb.addr(AddrSpace::Frame, sa), fb.addr(AddrSpace::Frame, sb));
    let (a, b) = (fb.reg(RegTy::I64), fb.reg(RegTy::I64));
    fb.emit(Inst::Load {
        dst: a,
        addr: pa,
        w: Width::B8,
    });
    fb.emit(Inst::Load {
        dst: b,
        addr: pbr,
        w: Width::B8,
    });
    let dst = fb.reg(RegTy::I64);
    match range {
        None => fb.emit(Inst::Bin { op, dst, a, b }),
        Some((l, h)) => {
            let (lo, hi) = (fb.int(l), fb.int(h));
            fb.emit(Inst::BinRange {
                op,
                dst,
                a,
                b,
                lo,
                hi,
                what: msg,
            });
        }
    }
    fb.emit(Inst::Ret { v: Some(dst) });
    fb.finish();
    (pb.finish().unwrap(), f)
}

#[test]
fn integer_ops_match_the_interpreter() {
    let vals = [
        0,
        1,
        -1,
        2,
        -2,
        7,
        -7,
        1000,
        i64::MAX,
        i64::MIN,
        i64::MAX - 1,
        i64::MIN + 1,
    ];
    let ops = [
        IntOp::Add,
        IntOp::Sub,
        IntOp::Mul,
        IntOp::Div,
        IntOp::Rem,
        IntOp::Mod,
        IntOp::Pow,
    ];
    for op in ops {
        for range in [None, Some((-100, 100))] {
            let (prog, f) = binop(op, range);
            let p = Both::new(prog);
            for &x in &vals {
                for &y in &vals {
                    let (a, b, n) = p.both(f, &[x as u64, y as u64]);
                    assert_eq!(a, b, "{op:?} {range:?} {x} {y}");
                    assert!(n > 0, "compiled code never ran");
                }
            }
        }
    }
}

#[test]
fn loop_runs_compiled() {
    // sum := 0; for i in 0 to n loop sum := sum + i; end loop; return sum;
    let mut pb = ProgramBuilder::new();
    let f = pb.declare("sum", FuncKind::Function, None);
    let mut fb = pb.define(f);
    let sn = fb.param("n", Width::B8, ParamClass::Value, Mode::In, None);
    let (head, body, done) = (fb.block(), fb.block(), fb.block());
    stmt(&mut fb);
    let pn = fb.addr(AddrSpace::Frame, sn);
    let n = fb.reg(RegTy::I64);
    fb.emit(Inst::Load {
        dst: n,
        addr: pn,
        w: Width::B8,
    });
    let (i, acc, one) = (fb.int(0), fb.int(0), fb.int(1));
    fb.emit(Inst::Br { to: head });
    fb.at(head);
    let c = fb.reg(RegTy::I64);
    fb.emit(Inst::Cmp {
        op: CmpOp::Le,
        dst: c,
        a: i,
        b: n,
    });
    fb.emit(Inst::CondBr {
        c,
        t: body,
        f: done,
    });
    fb.at(body);
    fb.emit(Inst::Bin {
        op: IntOp::Add,
        dst: acc,
        a: acc,
        b: i,
    });
    fb.emit(Inst::Bin {
        op: IntOp::Add,
        dst: i,
        a: i,
        b: one,
    });
    fb.emit(Inst::Br { to: head });
    fb.at(done);
    fb.emit(Inst::Ret { v: Some(acc) });
    fb.finish();
    let p = Both::new(pb.finish().unwrap());
    let (a, b, n) = p.both(f, &[100_000]);
    assert_eq!(a, Ok(5_000_050_000));
    assert_eq!(a, b);
    // One entry: the whole loop runs in compiled code, the interpreter only executes `Ret`.
    assert_eq!(n, 1);
}
