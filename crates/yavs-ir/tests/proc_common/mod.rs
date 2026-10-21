//! A process whose variable slot only ever holds 0, 3 or 5, and adds one to it in a
//! range-checked sum: the number of range-checked sums left after optimisation.

use yavs_diag::{FileId, Span};
use yavs_ir::*;

fn pin(fb: &mut FuncBuilder<'_>, off: u32) {
    let a = fb.addr(AddrSpace::Frame, off);
    let b = fb.addr(AddrSpace::Frame, off + 8);
    fb.emit(Inst::Store {
        addr: b,
        src: a,
        w: Width::B8,
    });
}

fn store(fb: &mut FuncBuilder<'_>, off: u32, v: i64) {
    let a = fb.addr(AddrSpace::Frame, off);
    let k = fb.int(v);
    fb.emit(Inst::Store {
        addr: a,
        src: k,
        w: Width::B8,
    });
}

/// `wait`: the process waits on a timeout (it resumes), else for ever. The variable at slot
/// 40 is 3 from the start of the first activation, 5 after each activation.
pub fn sums_left(hi: i64, wait: bool) -> usize {
    let mut pb = ProgramBuilder::new();
    let f = pb.declare("p", FuncKind::Process, None);
    let mut fb = pb.define(f);
    fb.slot(64, 8);
    fb.stmt(StmtKind::Wait, Span::new(FileId(0), 0, 1));
    store(&mut fb, 8, 3);
    // Long enough, with a loop that does not suspend (the conditions of the pass).
    for k in 0..40 {
        let a = fb.reg(RegTy::I64);
        fb.emit(Inst::Const { dst: a, v: k });
    }
    let (head, body) = (fb.block(), fb.block());
    fb.body(head);
    fb.emit(Inst::Br { to: head });
    fb.at(head);
    let a = fb.addr(AddrSpace::Frame, 48);
    pin(&mut fb, 48);
    let c = fb.reg(RegTy::I64);
    fb.emit(Inst::Load {
        dst: c,
        addr: a,
        w: Width::B1,
    });
    fb.emit(Inst::CondBr {
        c,
        t: head,
        f: body,
    });
    fb.at(body);
    let a = fb.addr(AddrSpace::Frame, 8);
    let x = fb.reg(RegTy::I64);
    fb.emit(Inst::Load {
        dst: x,
        addr: a,
        w: Width::B8,
    });
    let one = fb.int(1);
    let (l, h) = (fb.int(0), fb.int(hi));
    let what = fb.program().check_msg("range", Dir::To);
    let d = fb.reg(RegTy::I64);
    fb.emit(Inst::BinRange {
        op: IntOp::Add,
        dst: d,
        a: x,
        b: one,
        lo: l,
        hi: h,
        what,
    });
    store(&mut fb, 0, 0);
    let s = fb.addr(AddrSpace::Frame, 0);
    fb.emit(Inst::Store {
        addr: s,
        src: d,
        w: Width::B8,
    });
    store(&mut fb, 8, 5);
    if wait {
        let t = fb.int(1);
        fb.emit(Inst::WaitFor { t });
    }
    fb.emit(Inst::Suspend { resume: 0 });
    fb.emit(Inst::Br { to: head });
    fb.finish();
    let p = pb.finish().expect("verifies");
    let code = &p.func(f).expect("p").code;
    code.iter()
        .filter(|i| matches!(i, Inst::BinRange { .. }))
        .count()
}
