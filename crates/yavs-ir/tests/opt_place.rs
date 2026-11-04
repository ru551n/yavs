//! Logic rows read in place (`opt/place.rs`): the copy of a row into a temporary goes when
//! nothing can write the source before the read, and stays when something can. The whole
//! designs are in `crates/yavs/tests/place.rs`.

use yavs_diag::{FileId, Span};
use yavs_ir::*;

/// A process that copies elements 16 to 48 of an array in its frame into a temporary and reads
/// element 3 of it; `between` runs after the copy, before the read, and gets the source's slot.
/// Returns the instructions.
fn process(between: impl FnOnce(&mut FuncBuilder<'_>, u32)) -> Vec<Inst> {
    let mut pb = ProgramBuilder::new();
    let f = pb.declare("p", FuncKind::Process, None);
    let mut fb = pb.define(f);
    let var = fb.slot(8, 8);
    let rows = fb.slot(256, 8);
    let span = Span::new(FileId(0), 0, 1);
    fb.stmt(StmtKind::Wait, span);
    let head = fb.block();
    fb.body(head);
    fb.emit(Inst::Br { to: head });
    fb.at(head);
    fb.stmt(StmtKind::Wait, span);
    let src = fb.addr(AddrSpace::Frame, rows);
    let (size, zero, so, n, i) = (fb.int(64), fb.int(0), fb.int(16), fb.int(32), fb.int(3));
    let t = fb.reg(RegTy::Ptr);
    fb.emit(Inst::Alloc { dst: t, size });
    fb.emit(Inst::LCopy {
        dst: t,
        dst_off: zero,
        src,
        src_off: so,
        n,
    });
    between(&mut fb, rows);
    let x = fb.reg(RegTy::I64);
    fb.emit(Inst::LGet {
        dst: x,
        base: t,
        idx: i,
    });
    let v = fb.addr(AddrSpace::Frame, var);
    fb.emit(Inst::Store {
        addr: v,
        src: x,
        w: Width::B8,
    });
    fb.emit(Inst::Suspend { resume: 0 });
    fb.emit(Inst::Br { to: head });
    fb.finish();
    let p = pb.finish().expect("verifies");
    p.func(f).expect("p").code.to_vec()
}

fn copies(code: &[Inst]) -> usize {
    code.iter()
        .filter(|i| matches!(i, Inst::LCopy { .. }))
        .count()
}

#[test]
fn an_element_of_a_copied_row_is_read_from_the_source() {
    if !opt::enabled() {
        return;
    }
    let code = process(|_, _| {});
    assert_eq!(copies(&code), 0, "{code:?}");
    assert!(
        code.iter().any(|i| matches!(i, Inst::LGet { .. })),
        "{code:?}"
    );
}

#[test]
fn a_write_between_the_copy_and_the_read_keeps_the_copy() {
    if !opt::enabled() {
        return;
    }
    let code = process(|fb, rows| {
        let (base, idx, v) = (fb.addr(AddrSpace::Frame, rows), fb.int(19), fb.int(3));
        fb.emit(Inst::LSet { base, idx, v });
    });
    assert_eq!(copies(&code), 1, "{code:?}");
}
