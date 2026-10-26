//! `opt::frame_blocks` (`--tier=clift-auto`): a 32-byte block the interpreter fuses with the signal
//! read into it is frame bytes, as with the layout slots, and the layout slots stay off. A process
//! of its own: the setting is global.

use yavs_diag::{FileId, Span};
use yavs_ir::*;

#[test]
fn a_fused_block_is_frame_bytes_without_layout_slots() {
    if !opt::enabled() {
        return;
    }
    opt::frame_blocks();
    let fp = opt::fingerprint();
    assert!(
        fp.contains("blocks=true") && fp.contains("layouts=false"),
        "{fp}"
    );
    let mut pb = ProgramBuilder::new();
    let f = pb.declare("p", FuncKind::Process, None);
    let mut fb = pb.define(f);
    let var = fb.slot(8, 8);
    let span = Span::new(FileId(0), 0, 1);
    fb.stmt(StmtKind::Wait, span);
    let head = fb.block();
    fb.body(head);
    fb.emit(Inst::Br { to: head });
    fb.at(head);
    fb.stmt(StmtKind::Wait, span);
    let (off, len) = (fb.int(0), fb.int(16));
    let sig = fb.reg(RegTy::Sig);
    fb.emit(Inst::SigVal {
        dst: sig,
        sig: 0,
        offset: off,
        len,
    });
    let size = fb.int(32);
    let t = fb.reg(RegTy::Ptr);
    fb.emit(Inst::Alloc { dst: t, size });
    fb.emit(Inst::SigRead { dst: t, sig });
    let w = fb.reg(RegTy::I64);
    fb.emit(Inst::Load {
        dst: w,
        addr: t,
        w: Width::B8,
    });
    let v = fb.addr(AddrSpace::Frame, var);
    fb.emit(Inst::Store {
        addr: v,
        src: w,
        w: Width::B8,
    });
    fb.emit(Inst::Suspend { resume: 0 });
    fb.emit(Inst::Br { to: head });
    fb.finish();
    let p = pb.finish().expect("verifies");
    let code = &p.func(f).expect("p").code;
    assert!(
        !code.iter().any(|i| matches!(i, Inst::Alloc { .. })),
        "{code:?}"
    );
}
