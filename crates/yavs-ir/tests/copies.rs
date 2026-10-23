//! Specialised copies of a process (`copies.rs`): content addressed, in reserved slots, and the
//! optimiser folds the reads of the instance data into them.

use yavs_diag::{FileId, Span};
use yavs_ir::*;

/// A process that reads generic `G` (8 bytes at instance offset 0), and on each activation stores
/// 1 or 2 in a frame slot depending on `G == 3`.
fn process() -> (Program, FuncId) {
    let mut pb = ProgramBuilder::new();
    let f = pb.declare("p", FuncKind::Process, None);
    let mut fb = pb.define(f);
    fb.slot(16, 8);
    fb.stmt(StmtKind::Wait, Span::new(FileId(0), 0, 1));
    let (head, body, yes, no, end) = (fb.block(), fb.block(), fb.block(), fb.block(), fb.block());
    fb.body(head);
    fb.emit(Inst::Br { to: head });
    fb.at(head);
    fb.emit(Inst::Br { to: body });
    fb.at(body);
    let a = fb.addr(AddrSpace::Inst, 0);
    let g = fb.reg(RegTy::I64);
    fb.emit(Inst::Load {
        dst: g,
        addr: a,
        w: Width::B8,
    });
    let three = fb.int(3);
    let c = fb.reg(RegTy::I64);
    fb.emit(Inst::Cmp {
        op: CmpOp::Eq,
        dst: c,
        a: g,
        b: three,
    });
    fb.emit(Inst::CondBr { c, t: yes, f: no });
    for (blk, v) in [(yes, 1), (no, 2)] {
        fb.at(blk);
        let s = fb.addr(AddrSpace::Frame, 0);
        let k = fb.int(v);
        fb.emit(Inst::Store {
            addr: s,
            src: k,
            w: Width::B8,
        });
        fb.emit(Inst::Br { to: end });
    }
    fb.at(end);
    fb.emit(Inst::Suspend { resume: 0 });
    fb.emit(Inst::Br { to: head });
    fb.finish();
    (pb.finish().expect("verifies"), f)
}

fn data(g: i64, other: u8) -> Vec<u8> {
    let mut d = g.to_le_bytes().to_vec();
    d.extend([other; 8]);
    d
}

fn branches(p: &Program, f: FuncId) -> usize {
    p.func(f)
        .expect("f")
        .code
        .iter()
        .filter(|i| matches!(i, Inst::CondBr { .. }))
        .count()
}

#[test]
fn a_copy_folds_the_instance_data_it_is_for() {
    if !yavs_ir::opt::specialise_on() {
        return;
    }
    let (mut p, f) = process();
    p.reserve_copies(2);
    assert!(p.specialisable(f));
    let c3 = p.copy_of(f, &data(3, 0), &[]).expect("a copy");
    assert_eq!(p.base_of(c3), f);
    assert_eq!(p.base_of(f), f);
    // Equal data share a copy (and the bytes of a hole do not count); other data do not.
    assert_eq!(p.copy_of(f, &data(3, 0), &[]), Ok(c3));
    assert_eq!(p.copy_of(f, &data(3, 7), &[(8, 8)]), Ok(c3));
    let c4 = p.copy_of(f, &data(4, 0), &[]).expect("a second copy");
    assert_ne!(c3, c4);
    assert_eq!(p.copy_of(f, &data(5, 0), &[]), Err(NoCopy::Full));
    // The shared code tests `G == 3`; each copy knows the answer.
    assert_eq!(branches(&p, f), 1);
    assert_eq!(branches(&p, c3), 0);
    assert_eq!(branches(&p, c4), 0);
    // A hole is read at run time: the test stays.
    let (mut q, f) = process();
    q.reserve_copies(1);
    let h = q.copy_of(f, &data(3, 0), &[(0, 8)]).expect("a copy");
    assert_eq!(branches(&q, h), 1);
    let st = p.copy_stats();
    assert_eq!((st.copies, st.reserved, st.full), (2, 2, 1));
}

#[test]
fn nothing_is_stored_for_the_slots() {
    if !yavs_ir::opt::specialise_on() {
        return;
    }
    let (mut p, f) = process();
    let before = postcard::to_stdvec(&p).expect("stored").len();
    p.reserve_copies(4);
    let _ = p.copy_of(f, &data(3, 0), &[]);
    assert_eq!(postcard::to_stdvec(&p).expect("stored").len(), before);
    assert!(p.export_optimized().iter().all(|c| c.func == f));
}
