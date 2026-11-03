//! `stmt_trims`: which statement markers compiled code must run.

use yavs_diag::{FileId, Span};
use yavs_ir::*;

fn sp() -> Span {
    Span::new(FileId(0), 0, 1)
}

/// A temporary of `size` bytes (a constant), as a statement makes one.
fn alloc(fb: &mut FuncBuilder<'_>, size: i64) {
    let n = fb.int(size);
    let a = fb.reg(RegTy::Ptr);
    fb.emit(Inst::Alloc { dst: a, size: n });
}

/// The decisions for the markers of `f`, in code order, of the code as lowered.
fn trims(kind: FuncKind, body: impl FnOnce(&mut FuncBuilder<'_>)) -> Vec<StmtTrim> {
    let mut pb = ProgramBuilder::new();
    let f = pb.declare("f", kind, None);
    let mut fb = pb.define(f);
    // (The verifier wants a statement first and a terminator last; the first marker is not returned.)
    fb.stmt(StmtKind::Process, sp());
    body(&mut fb);
    fb.finish();
    let p = pb.finish().expect("verifies");
    let raw = p.funcs.get(f).expect("f");
    let all = stmt_trims(&p, raw);
    raw.code
        .iter()
        .enumerate()
        .filter(|(_, i)| matches!(i, Inst::Stmt))
        .map(|(pc, _)| all[pc])
        .skip(1)
        .collect()
}

const FREE: StmtTrim = StmtTrim::Trim { roots_free: true };

#[test]
fn a_process_carries_small_temporaries_to_its_suspension() {
    let t = trims(FuncKind::Process, |fb| {
        let top = fb.block();
        fb.emit(Inst::Br { to: top });
        fb.at(top);
        for _ in 0..3 {
            fb.stmt(StmtKind::SigAssign, sp());
            alloc(fb, 40);
        }
        fb.stmt(StmtKind::Wait, sp());
        fb.emit(Inst::Suspend { resume: 0 });
        fb.emit(Inst::Br { to: top });
    });
    assert_eq!(t, vec![StmtTrim::Skip; 4]);
}

#[test]
fn a_loop_without_a_suspension_trims_every_round() {
    let t = trims(FuncKind::Process, |fb| {
        let (head, body) = (fb.block(), fb.block());
        fb.emit(Inst::Br { to: head });
        fb.at(head);
        fb.stmt(StmtKind::SigAssign, sp());
        alloc(fb, 40);
        let c = fb.int(1);
        fb.emit(Inst::CondBr {
            c,
            t: body,
            f: head,
        });
        fb.at(body);
        fb.emit(Inst::Br { to: head });
    });
    assert_eq!(t, vec![FREE]);
}

#[test]
fn a_loop_through_a_suspension_is_not_a_loop() {
    let t = trims(FuncKind::Process, |fb| {
        let top = fb.block();
        fb.emit(Inst::Br { to: top });
        fb.at(top);
        fb.stmt(StmtKind::SigAssign, sp());
        alloc(fb, 40);
        fb.emit(Inst::Suspend { resume: 0 });
        fb.emit(Inst::Br { to: top });
    });
    assert_eq!(t, vec![StmtTrim::Skip]);
}

#[test]
fn a_root_or_an_unbounded_temporary_forces_the_trim() {
    // A recorded root: the test of the roots stays.
    let t = trims(FuncKind::Process, |fb| {
        fb.stmt(StmtKind::VarAssign, sp());
        let n = fb.int(8);
        let a = fb.reg(RegTy::Ptr);
        fb.emit(Inst::Alloc { dst: a, size: n });
        fb.emit(Inst::TempRoot {
            addr: a,
            desc: None,
            ty: None,
        });
        fb.stmt(StmtKind::VarAssign, sp());
        fb.emit(Inst::Ret { v: None });
    });
    assert_eq!(t[1], StmtTrim::Trim { roots_free: false });
    // A temporary above the small limit: trim, but no root can be recorded.
    let t = trims(FuncKind::Process, |fb| {
        fb.stmt(StmtKind::VarAssign, sp());
        alloc(fb, 1 << 20);
        fb.stmt(StmtKind::VarAssign, sp());
        fb.emit(Inst::Ret { v: None });
    });
    assert_eq!(t[1], FREE);
    // Many small ones add up to the limit.
    let t = trims(FuncKind::Process, |fb| {
        for _ in 0..4 {
            fb.stmt(StmtKind::VarAssign, sp());
            alloc(fb, 100);
        }
        fb.emit(Inst::Ret { v: None });
    });
    assert!(t.contains(&FREE), "{t:?}");
    assert_eq!(t[0], StmtTrim::Skip);
}

#[test]
fn a_subprogram_trims_unless_nothing_was_made() {
    let t = trims(FuncKind::Procedure, |fb| {
        fb.stmt(StmtKind::VarAssign, sp());
        fb.stmt(StmtKind::VarAssign, sp());
        alloc(fb, 8);
        fb.stmt(StmtKind::VarAssign, sp());
        fb.emit(Inst::Ret { v: None });
    });
    let keep = StmtTrim::Trim { roots_free: false };
    assert_eq!(t, vec![StmtTrim::Skip, StmtTrim::Skip, keep]);
}
