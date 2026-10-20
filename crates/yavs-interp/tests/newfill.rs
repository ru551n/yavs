//! `NewFill`: an object built from a descriptor and either a copy or a repeated element.

mod common;

use std::sync::Arc;

use common::*;
use yavs_interp::{InstData, Runtime};
use yavs_ir::*;

fn word(fb: &mut FuncBuilder<'_>, p: Reg, off: i64) -> Reg {
    let o = fb.int(off);
    let a = fb.reg(RegTy::Ptr);
    fb.emit(Inst::Index {
        dst: a,
        base: p,
        pos: o,
        stride: 1,
    });
    let v = fb.reg(RegTy::I64);
    fb.emit(Inst::Load {
        dst: v,
        addr: a,
        w: Width::B8,
    });
    v
}

fn buf(fb: &mut FuncBuilder<'_>, words: &[i64]) -> Reg {
    let size = fb.int(8 * words.len() as i64);
    let p = fb.reg(RegTy::Ptr);
    fb.emit(Inst::Alloc { dst: p, size });
    for (i, w) in words.iter().enumerate() {
        let o = fb.int(8 * i as i64);
        let a = fb.reg(RegTy::Ptr);
        fb.emit(Inst::Index {
            dst: a,
            base: p,
            pos: o,
            stride: 1,
        });
        let v = fb.int(*w);
        fb.emit(Inst::Store {
            addr: a,
            src: v,
            w: Width::B8,
        });
    }
    p
}

/// The object `[7][5][5][5][5]` (repeated element), `[7]` + a copy of `[1][2]`, and a zero fill.
#[test]
fn fill_copy_and_zero() {
    let mut pb = ProgramBuilder::new();
    let int = integer(&mut pb);
    let what = pb.check_msg("o", Dir::To);
    let fun = pb.declare("fun", FuncKind::Function, None);
    {
        let mut fb = pb.define(fun);
        stmt(&mut fb, StmtKind::VarAssign);
        let desc = buf(&mut fb, &[7]);
        let five = buf(&mut fb, &[5]);
        let pair = buf(&mut fb, &[1, 2]);
        let zero = buf(&mut fb, &[0]);
        let mk = |fb: &mut FuncBuilder<'_>, data: Reg, size: i64, fill: u32| {
            let size = fb.int(size);
            let h = fb.reg(RegTy::Handle);
            fb.emit(Inst::NewFill {
                dst: h,
                ty: int,
                desc,
                dsize: 8,
                data,
                size,
                fill,
                shared: false,
            });
            let p = fb.reg(RegTy::Ptr);
            fb.emit(Inst::Deref {
                dst: p,
                handle: h,
                what,
            });
            p
        };
        let a = mk(&mut fb, five, 32, 8);
        let b = mk(&mut fb, pair, 16, 0);
        let c = mk(&mut fb, zero, 24, 8);
        let mut sum = word(&mut fb, a, 0);
        for (p, off) in [(a, 8), (a, 32), (b, 0), (b, 8), (b, 16), (c, 0), (c, 24)] {
            let v = word(&mut fb, p, off);
            sum = bin(&mut fb, IntOp::Add, sum, v);
        }
        fb.emit(Inst::Ret { v: Some(sum) });
        fb.finish();
    }
    let prog = Arc::new(pb.finish().unwrap());
    let rt = Runtime::new(prog, Default::default()).unwrap();
    let v = yavs_interp::call(rt, fun, Arc::new(InstData::default()), &[]).unwrap();
    // 7 + 5 + 5 (a) + 7 + 1 + 2 (b) + 7 + 0 (c) = 34
    assert_eq!(v, Some(34));
}
