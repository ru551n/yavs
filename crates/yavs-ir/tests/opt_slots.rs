//! Constant-size temporaries of a process live in its frame (`opt/slots.rs`, #668): the `Alloc`
//! becomes bytes of the frame, zeroed only where the code that follows does not write them whole,
//! and a temporary a later block uses stays an `Alloc`.

use yavs_diag::{FileId, Span};
use yavs_ir::*;

/// Add two words of the block at `t` and store the sum in the process's variable.
fn sum_words(fb: &mut FuncBuilder<'_>, t: Reg, var: u32) {
    let (w1, w2) = (fb.reg(RegTy::I64), fb.reg(RegTy::I64));
    fb.emit(Inst::Load {
        dst: w1,
        addr: t,
        w: Width::B8,
    });
    let eight = fb.int(8);
    let u = fb.reg(RegTy::Ptr);
    fb.emit(Inst::Index {
        dst: u,
        base: t,
        pos: eight,
        stride: 1,
    });
    fb.emit(Inst::Load {
        dst: w2,
        addr: u,
        w: Width::B8,
    });
    let sum = fb.reg(RegTy::I64);
    fb.emit(Inst::Bin {
        op: IntOp::Add,
        dst: sum,
        a: w1,
        b: w2,
    });
    let v = fb.addr(AddrSpace::Frame, var);
    fb.emit(Inst::Store {
        addr: v,
        src: sum,
        w: Width::B8,
    });
}

/// A store through a pointer the optimiser knows nothing about: the words stored before it are
/// not forwarded to the reads after it.
fn barrier(fb: &mut FuncBuilder<'_>) {
    let slot = fb.slot(8, 8);
    let qa = fb.addr(AddrSpace::Frame, slot);
    let q = fb.reg(RegTy::Ptr);
    fb.emit(Inst::Load {
        dst: q,
        addr: qa,
        w: Width::B8,
    });
    let z = fb.int(0);
    fb.emit(Inst::Store {
        addr: q,
        src: z,
        w: Width::B8,
    });
}

fn store_word(fb: &mut FuncBuilder<'_>, t: Reg, off: i64, v: i64) {
    let a = fb.reg(RegTy::Ptr);
    let k = fb.int(off);
    fb.emit(Inst::Index {
        dst: a,
        base: t,
        pos: k,
        stride: 1,
    });
    let x = fb.int(v);
    fb.emit(Inst::Store {
        addr: a,
        src: x,
        w: Width::B8,
    });
}

/// A process whose first statement makes a temporary of 16 bytes (`t`, its address); `shape`
/// writes and reads it (the variable's slot is its third argument).
/// Returns the instructions and the frame size.
fn process(shape: impl FnOnce(&mut FuncBuilder<'_>, Reg, u32)) -> (Vec<Inst>, u32) {
    process_of(16, false, |fb, t, var, _| shape(fb, t, var))
}

/// As `process`, the temporary being `bytes` long and, with `sig`, a signal reference made
/// before the `Alloc` (the last argument of `shape`).
fn process_of(
    bytes: i64,
    sig: bool,
    shape: impl FnOnce(&mut FuncBuilder<'_>, Reg, u32, Option<Reg>),
) -> (Vec<Inst>, u32) {
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
    let sr = sig.then(|| {
        let (off, len) = (fb.int(0), fb.int(16));
        let r = fb.reg(RegTy::Sig);
        fb.emit(Inst::SigVal {
            dst: r,
            sig: 0,
            offset: off,
            len,
        });
        r
    });
    let size = fb.int(bytes);
    let t = fb.reg(RegTy::Ptr);
    fb.emit(Inst::Alloc { dst: t, size });
    shape(&mut fb, t, var, sr);
    fb.emit(Inst::Suspend { resume: 0 });
    fb.emit(Inst::Br { to: head });
    fb.finish();
    let p = pb.finish().expect("verifies");
    let func = p.func(f).expect("p");
    (func.code.to_vec(), func.frame)
}

fn count(code: &[Inst], hit: impl Fn(&Inst) -> bool) -> usize {
    code.iter().filter(|i| hit(i)).count()
}

/// Both words stored, the block never read before: no `Alloc`, no `Zero`, bytes of the frame.
#[test]
fn a_temporary_written_whole_is_frame_bytes_without_zeroing() {
    if !yavs_ir::opt::enabled() {
        return;
    }
    let (code, frame) = process(|fb, t, var| {
        store_word(fb, t, 0, 3);
        store_word(fb, t, 8, 4);
        barrier(fb);
        sum_words(fb, t, var);
    });
    assert_eq!(
        count(&code, |i| matches!(i, Inst::Alloc { .. })),
        0,
        "{code:?}"
    );
    assert_eq!(
        count(&code, |i| matches!(i, Inst::Zero { .. })),
        0,
        "{code:?}"
    );
    assert!(frame >= 8 + 16, "frame {frame}");
}

/// A word the code reads without writing it: the block is zeroed first.
#[test]
fn a_temporary_read_before_it_is_written_is_zeroed() {
    if !yavs_ir::opt::enabled() {
        return;
    }
    let (code, _) = process(|fb, t, var| {
        store_word(fb, t, 0, 3);
        sum_words(fb, t, var);
    });
    assert_eq!(
        count(&code, |i| matches!(i, Inst::Alloc { .. })),
        0,
        "{code:?}"
    );
    assert_eq!(
        count(&code, |i| matches!(i, Inst::Zero { bytes: 16, .. })),
        1,
        "{code:?}"
    );
}

/// The pointer is used in the next block: it stays a temporary of the statement.
#[test]
fn a_temporary_a_later_block_uses_stays_an_alloc() {
    if !yavs_ir::opt::enabled() {
        return;
    }
    let (code, _) = process(|fb, t, var| {
        let next = fb.block();
        store_word(fb, t, 0, 3);
        store_word(fb, t, 8, 4);
        barrier(fb);
        fb.emit(Inst::Br { to: next });
        fb.at(next);
        sum_words(fb, t, var);
    });
    assert_eq!(
        count(&code, |i| matches!(i, Inst::Alloc { .. })),
        1,
        "{code:?}"
    );
    assert_eq!(
        count(&code, |i| matches!(i, Inst::Zero { .. })),
        0,
        "{code:?}"
    );
}

/// A second temporary `d` whose address `shape` gets; both made by the first statement of the loop.
fn two_temporaries(shape: impl FnOnce(&mut FuncBuilder<'_>, Reg, Reg, u32)) -> Vec<Inst> {
    let (code, _) = process(|fb, p, var| {
        let size = fb.int(16);
        let d = fb.reg(RegTy::Ptr);
        fb.emit(Inst::Alloc { dst: d, size });
        shape(fb, p, d, var);
    });
    code
}

/// Store `d`'s address into `p` and load it back into a new register, with a store through an
/// unknown pointer between (so the load is not forwarded from the store).
fn store_and_load_back(fb: &mut FuncBuilder<'_>, p: Reg, d: Reg) -> Reg {
    fb.emit(Inst::Store {
        addr: p,
        src: d,
        w: Width::B8,
    });
    let q = fb.reg(RegTy::Ptr);
    let slot = fb.slot(8, 8);
    let qa = fb.addr(AddrSpace::Frame, slot);
    fb.emit(Inst::Load {
        dst: q,
        addr: qa,
        w: Width::B8,
    });
    let z = fb.int(0);
    fb.emit(Inst::Store {
        addr: q,
        src: z,
        w: Width::B8,
    });
    let x = fb.reg(RegTy::Ptr);
    fb.emit(Inst::Load {
        dst: x,
        addr: p,
        w: Width::B8,
    });
    x
}

/// `p` holds the address of `d`; the pointer loaded back out of `p` is read in the next block, so
/// `d` must stay an `Alloc` (S1 of the #668 review).
#[test]
fn a_temporary_whose_address_is_loaded_back_and_read_later_stays_an_alloc() {
    if !yavs_ir::opt::enabled() {
        return;
    }
    let code = two_temporaries(|fb, p, d, var| {
        store_word(fb, d, 0, 3);
        store_word(fb, d, 8, 4);
        let x = store_and_load_back(fb, p, d);
        let next = fb.block();
        fb.emit(Inst::Br { to: next });
        fb.at(next);
        sum_words(fb, x, var);
    });
    assert!(
        count(&code, |i| matches!(i, Inst::Alloc { .. })) >= 1,
        "{code:?}"
    );
}

/// The same in one block: a temporary made after `d`'s last direct use must not take `d`'s bytes
/// while the pointer loaded out of `p` still reads them.
#[test]
fn a_temporary_whose_address_is_loaded_back_keeps_its_bytes() {
    if !yavs_ir::opt::enabled() {
        return;
    }
    let code = two_temporaries(|fb, p, d, var| {
        store_word(fb, d, 0, 3);
        store_word(fb, d, 8, 4);
        let x = store_and_load_back(fb, p, d);
        let size = fb.int(16);
        let e = fb.reg(RegTy::Ptr);
        fb.emit(Inst::Alloc { dst: e, size });
        store_word(fb, e, 0, 5);
        store_word(fb, e, 8, 6);
        sum_words(fb, e, var);
        sum_words(fb, x, var);
    });
    assert!(
        count(&code, |i| matches!(i, Inst::Alloc { .. })) >= 1,
        "{code:?}"
    );
}

/// A block of 32 bytes the interpreter fuses with the signal read into it stays an `Alloc` (its
/// frame form is out of its dispatch loop, #756); the same block written by stores is frame bytes.
#[test]
fn a_block_the_interpreter_fuses_with_a_signal_read_stays_an_alloc() {
    if !yavs_ir::opt::enabled() {
        return;
    }
    let (code, _) = process_of(32, true, |fb, t, var, sig| {
        let sig = sig.expect("made");
        fb.emit(Inst::SigRead { dst: t, sig });
        sum_words(fb, t, var);
    });
    assert_eq!(
        count(&code, |i| matches!(i, Inst::Alloc { .. })),
        1,
        "{code:?}"
    );
    let (code, _) = process_of(32, false, |fb, t, var, _| {
        store_word(fb, t, 0, 3);
        store_word(fb, t, 8, 4);
        sum_words(fb, t, var);
    });
    assert_eq!(
        count(&code, |i| matches!(i, Inst::Alloc { .. })),
        0,
        "{code:?}"
    );
}
