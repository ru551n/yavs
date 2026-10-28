//! Specialised copies of a process (`copies.rs`): content addressed, in reserved slots, and the
//! optimiser folds the reads of the instance data into them.

use yavs_diag::{FileId, Span};
use yavs_ir::*;

/// An output: an effect that keeps an arm of a branch apart from the other (an empty arm is a
/// jump, #791).
fn effect(fb: &mut FuncBuilder<'_>) {
    let (a, n) = (fb.addr(AddrSpace::Inst, 0), fb.int(0));
    fb.emit(Inst::Output { addr: a, len: n });
}

/// A process that reads generic `G` (8 bytes at instance offset 0), and on each activation stores
/// 1 or 2 in a frame slot depending on `G == 3`.
fn process() -> (Program, FuncId) {
    process_with(false)
}

/// `declarations`: the entry block holds a declarative part (a report, which stays).
fn process_with(declarations: bool) -> (Program, FuncId) {
    let mut pb = ProgramBuilder::new();
    let f = pb.declare("p", FuncKind::Process, None);
    let mut fb = pb.define(f);
    fb.slot(16, 8);
    fb.stmt(StmtKind::Wait, Span::new(FileId(0), 0, 1));
    let (head, body, yes, no, end) = (fb.block(), fb.block(), fb.block(), fb.block(), fb.block());
    fb.body(head);
    if declarations {
        let (m, n, v) = (fb.addr(AddrSpace::Frame, 0), fb.int(0), fb.int(1));
        fb.emit(Inst::Report {
            msg: m,
            len: n,
            severity: v,
        });
    }
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
        if v == 1 {
            effect(&mut fb);
        }
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

/// The tag that names a copy (compiled code is named and keyed by it, #661) depends on the
/// process and the data alone, not on the order in which the copies were asked for.
#[test]
fn a_copy_tag_does_not_depend_on_the_order_of_the_copies() {
    if !yavs_ir::opt::specialise_on() {
        return;
    }
    let (mut p, f) = process();
    p.reserve_copies(3);
    let (a3, a4) = (
        p.copy_of(f, &data(3, 0), &[]).expect("copy"),
        p.copy_of(f, &data(4, 0), &[]).expect("copy"),
    );
    let (mut q, g) = process();
    q.reserve_copies(3);
    let (b4, b3) = (
        q.copy_of(g, &data(4, 0), &[]).expect("copy"),
        q.copy_of(g, &data(3, 7), &[(8, 8)]).expect("copy"),
    );
    assert_ne!((a3, a4), (b3, b4), "the ids follow the order");
    assert_eq!(p.copy_tag(a3), q.copy_tag(b3));
    assert_eq!(p.copy_tag(a4), q.copy_tag(b4));
    assert_ne!(p.copy_tag(a3), p.copy_tag(a4));
    assert!(p.copy_tag(a3).is_some());
    assert_eq!(p.copy_tag(f), None);
}

fn reports_of(p: &Program, f: FuncId) -> usize {
    p.func(f)
        .expect("f")
        .code
        .iter()
        .filter(|i| matches!(i, Inst::Report { .. }))
        .count()
}

/// A copy takes over between two activations of the shared code, so the declarative part (the
/// blocks before the statements) is not in it.
#[test]
fn a_copy_has_no_declarative_part() {
    if !yavs_ir::opt::specialise_on() {
        return;
    }
    let (mut p, f) = process_with(true);
    p.reserve_copies(1);
    assert_eq!(
        reports_of(&p, f),
        1,
        "the shared code has the declarative part"
    );
    let c = p.copy_of(f, &data(3, 0), &[]).expect("a copy");
    assert_eq!(reports_of(&p, c), 0, "the copy has not");
    assert_eq!(branches(&p, c), 0, "and folds the statements as before");
}

/// A process whose declarative part fills a frame word with `G + 1` (G at instance offset 0) and
/// makes a record variable the way lowering does (`[data, desc, handle]` at slot 0, the heap
/// object's descriptor copied from the instance data at 8: three elements). The body tests the
/// word against 6 and the descriptor's length against 3. With `rewrites` it also writes the word
/// (after reading it), so that test cannot fold.
fn decl_process(rewrites: bool) -> (Program, FuncId) {
    let mut pb = ProgramBuilder::new();
    let int = pb.ty(Ty::Int {
        lo: i64::MIN,
        hi: i64::MAX,
    });
    let arr = pb.ty(Ty::Array {
        elem: int,
        elem_desc: None,
        dims: 1,
    });
    let rec = pb.ty(Ty::Record {
        fields: vec![Field {
            name: "a".into(),
            ty: arr,
            desc: None,
        }],
    });
    let zero = pb.konst(ConstData::Bytes(vec![0]));
    let what = pb.check_msg("object", Dir::To);
    let f = pb.declare("p", FuncKind::Process, None);
    let mut fb = pb.define(f);
    fb.slot(32, 8);
    fb.stmt(StmtKind::Wait, Span::new(FileId(0), 0, 1));
    let (head, body) = (fb.block(), fb.block());
    fb.body(head);
    // The declarative part.
    let (desc, z) = (
        fb.addr(AddrSpace::Inst, 8),
        fb.addr(AddrSpace::Const(zero), 0),
    );
    let (h, p, data) = (
        fb.reg(RegTy::Handle),
        fb.reg(RegTy::Ptr),
        fb.reg(RegTy::Ptr),
    );
    let size = fb.int(24);
    fb.emit(Inst::NewFill {
        dst: h,
        ty: rec,
        desc,
        dsize: 24,
        data: z,
        size,
        fill: 1,
        shared: false,
    });
    fb.emit(Inst::Deref {
        dst: p,
        handle: h,
        what,
    });
    let ds = fb.int(24);
    fb.emit(Inst::Index {
        dst: data,
        base: p,
        pos: ds,
        stride: 1,
    });
    for (off, v) in [(0, data), (8, p), (16, h)] {
        let a = fb.addr(AddrSpace::Frame, off);
        fb.emit(Inst::Store {
            addr: a,
            src: v,
            w: Width::B8,
        });
    }
    let (ga, g, one, s) = (
        fb.addr(AddrSpace::Inst, 0),
        fb.reg(RegTy::I64),
        fb.int(1),
        fb.reg(RegTy::I64),
    );
    fb.emit(Inst::Load {
        dst: g,
        addr: ga,
        w: Width::B8,
    });
    fb.emit(Inst::Bin {
        op: IntOp::Add,
        dst: s,
        a: g,
        b: one,
    });
    let a24 = fb.addr(AddrSpace::Frame, 24);
    fb.emit(Inst::Store {
        addr: a24,
        src: s,
        w: Width::B8,
    });
    fb.emit(Inst::Br { to: head });
    fb.at(head);
    fb.emit(Inst::Br { to: body });
    // The body.
    fb.at(body);
    let (a8, d, n) = (
        fb.addr(AddrSpace::Frame, 8),
        fb.reg(RegTy::Ptr),
        fb.reg(RegTy::I64),
    );
    fb.emit(Inst::Load {
        dst: d,
        addr: a8,
        w: Width::B8,
    });
    fb.emit(Inst::DopeLen {
        dst: n,
        dope: d,
        dim: 0,
    });
    let (three, c1, t1, e1) = (fb.int(3), fb.reg(RegTy::I64), fb.block(), fb.block());
    fb.emit(Inst::Cmp {
        op: CmpOp::Eq,
        dst: c1,
        a: n,
        b: three,
    });
    fb.emit(Inst::CondBr {
        c: c1,
        t: t1,
        f: e1,
    });
    fb.at(t1);
    effect(&mut fb);
    fb.emit(Inst::Br { to: e1 });
    fb.at(e1);
    let (w, six, c2, t2, e2) = (
        fb.reg(RegTy::I64),
        fb.int(6),
        fb.reg(RegTy::I64),
        fb.block(),
        fb.block(),
    );
    let a24 = fb.addr(AddrSpace::Frame, 24);
    fb.emit(Inst::Load {
        dst: w,
        addr: a24,
        w: Width::B8,
    });
    fb.emit(Inst::Cmp {
        op: CmpOp::Eq,
        dst: c2,
        a: w,
        b: six,
    });
    fb.emit(Inst::CondBr {
        c: c2,
        t: t2,
        f: e2,
    });
    fb.at(t2);
    effect(&mut fb);
    fb.emit(Inst::Br { to: e2 });
    fb.at(e2);
    if rewrites {
        let (a, k) = (fb.addr(AddrSpace::Frame, 24), fb.int(9));
        fb.emit(Inst::Store {
            addr: a,
            src: k,
            w: Width::B8,
        });
    }
    // (A process that registers no wait never resumes: its slots are dead.)
    let t = fb.int(1);
    fb.emit(Inst::WaitFor { t });
    fb.emit(Inst::Suspend { resume: 0 });
    fb.emit(Inst::Br { to: head });
    fb.finish();
    (pb.finish().expect("verifies"), f)
}

/// Instance data of `decl_process`: G = 5 and a descriptor of 3 elements (`0 to 2`).
fn decl_data() -> Vec<u8> {
    let mut d = 5i64.to_le_bytes().to_vec();
    for w in [0i64, 2, 0] {
        d.extend(w.to_le_bytes());
    }
    d
}

/// The declarative part runs once per instance and the body only reads what it made: the copy
/// knows the word and the length of the heap object's descriptor (#678).
#[test]
fn a_copy_knows_what_the_declarative_part_made() {
    if !yavs_ir::opt::specialise_on() {
        return;
    }
    let (mut p, f) = decl_process(false);
    p.reserve_copies(1);
    let c = p.copy_of(f, &decl_data(), &[]).expect("a copy");
    assert_eq!(branches(&p, f), 2);
    assert_eq!(branches(&p, c), 0);
    assert!(
        !p.func(c)
            .expect("c")
            .code
            .iter()
            .any(|i| matches!(i, Inst::DopeLen { .. })),
        "the length of the descriptor is a constant"
    );
}

/// What the body writes is not known: it keeps the test of the word, the descriptor still folds.
#[test]
fn a_word_the_body_writes_is_not_known() {
    if !yavs_ir::opt::specialise_on() {
        return;
    }
    let (mut p, f) = decl_process(true);
    p.reserve_copies(1);
    let c = p.copy_of(f, &decl_data(), &[]).expect("a copy");
    assert_eq!(branches(&p, c), 1);
}

/// A process whose declarative part branches on a package constant (`Pkg(0)`, word 0) and stores
/// 7 or 9 in a frame word; the body tests the word against 7.
fn pkg_process() -> (Program, FuncId) {
    let mut pb = ProgramBuilder::new();
    let f = pb.declare("p", FuncKind::Process, None);
    let mut fb = pb.define(f);
    fb.slot(16, 8);
    fb.stmt(StmtKind::Wait, Span::new(FileId(0), 0, 1));
    // (The blocks of the declarative part come before the body's.)
    let (yes, no, join) = (fb.block(), fb.block(), fb.block());
    let (head, body) = (fb.block(), fb.block());
    fb.body(head);
    let (pa, w, three, c) = (
        fb.addr(AddrSpace::Pkg(0), 0),
        fb.reg(RegTy::I64),
        fb.int(3),
        fb.reg(RegTy::I64),
    );
    fb.emit(Inst::Load {
        dst: w,
        addr: pa,
        w: Width::B8,
    });
    fb.emit(Inst::Cmp {
        op: CmpOp::Eq,
        dst: c,
        a: w,
        b: three,
    });
    fb.emit(Inst::CondBr { c, t: yes, f: no });
    for (blk, v) in [(yes, 7), (no, 9)] {
        fb.at(blk);
        let (s, k) = (fb.addr(AddrSpace::Frame, 0), fb.int(v));
        fb.emit(Inst::Store {
            addr: s,
            src: k,
            w: Width::B8,
        });
        fb.emit(Inst::Br { to: join });
    }
    fb.at(join);
    fb.emit(Inst::Br { to: head });
    fb.at(head);
    fb.emit(Inst::Br { to: body });
    fb.at(body);
    // (An instance read keeps the process specialisable.)
    let (ia, g, a0, d, seven, c1, t1, e1) = (
        fb.addr(AddrSpace::Inst, 0),
        fb.reg(RegTy::I64),
        fb.addr(AddrSpace::Frame, 0),
        fb.reg(RegTy::I64),
        fb.int(7),
        fb.reg(RegTy::I64),
        fb.block(),
        fb.block(),
    );
    fb.emit(Inst::Load {
        dst: g,
        addr: ia,
        w: Width::B8,
    });
    fb.emit(Inst::Load {
        dst: d,
        addr: a0,
        w: Width::B8,
    });
    fb.emit(Inst::Cmp {
        op: CmpOp::Eq,
        dst: c1,
        a: d,
        b: seven,
    });
    fb.emit(Inst::CondBr {
        c: c1,
        t: t1,
        f: e1,
    });
    fb.at(t1);
    effect(&mut fb);
    fb.emit(Inst::Br { to: e1 });
    fb.at(e1);
    let t = fb.int(1);
    fb.emit(Inst::WaitFor { t });
    fb.emit(Inst::Suspend { resume: 0 });
    fb.emit(Inst::Br { to: head });
    fb.finish();
    (pb.finish().expect("verifies"), f)
}

/// The data of the packages of a run: package 0 holds one word.
struct Packages(i64);

impl PkgSource for Packages {
    fn package(&self, p: u32) -> Option<PkgData> {
        (p == 0).then(|| PkgData {
            bytes: std::sync::Arc::new(self.0.to_le_bytes()),
            digest: self.0 as u64,
        })
    }
}

/// A package constant read by the declarative part is a fact of the copy (#708), and the data
/// of the package are part of the copy's key: another value makes another copy.
#[test]
fn a_copy_knows_what_a_package_constant_made() {
    if !yavs_ir::opt::specialise_on() {
        return;
    }
    let (mut p, f) = pkg_process();
    p.reserve_copies(4);
    assert_eq!(&*p.pkg_deps(f), &[0]);
    // Without the package data the branch of the declarative part is unknown: no facts.
    let none = p.copy_of(f, &[0; 8], &[]).expect("a copy");
    assert_eq!(branches(&p, none), 1);
    let (c3, c4) = (
        p.copy_of_pkgs(f, &[0; 8], &[], &Packages(3))
            .expect("a copy"),
        p.copy_of_pkgs(f, &[0; 8], &[], &Packages(4))
            .expect("a copy"),
    );
    assert!(![none, c3].contains(&c4) && none != c3, "keys differ");
    assert_eq!(p.copy_of_pkgs(f, &[0; 8], &[], &Packages(3)), Ok(c3));
    assert_eq!((branches(&p, c3), branches(&p, c4)), (0, 0));
    assert_ne!(p.copy_tag(c3), p.copy_tag(c4));
}
