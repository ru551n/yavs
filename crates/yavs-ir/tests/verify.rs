//! The verifier rejects malformed IR; the dump is stable text.

use yavs_diag::{FileId, Span};
use yavs_ir::*;

fn sp() -> Span {
    Span::new(FileId(0), 0, 1)
}

/// Build one function with `body` and return the verifier messages.
fn errors(kind: FuncKind, body: impl FnOnce(&mut FuncBuilder<'_>)) -> Vec<String> {
    let mut pb = ProgramBuilder::new();
    let f = pb.declare("f", kind, None);
    let mut fb = pb.define(f);
    body(&mut fb);
    fb.finish();
    match pb.finish() {
        Ok(_) => Vec::new(),
        Err(e) => e.into_iter().map(|e| e.msg).collect(),
    }
}

fn expect(kind: FuncKind, want: &str, body: impl FnOnce(&mut FuncBuilder<'_>)) {
    let e = errors(kind, body);
    assert!(
        e.iter().any(|m| m.contains(want)),
        "want {want:?}, got {e:#?}"
    );
}

#[test]
fn well_formed_passes() {
    let e = errors(FuncKind::Process, |fb| {
        fb.stmt(StmtKind::Wait, sp());
        let t = fb.int(5);
        fb.emit(Inst::WaitFor { t });
        fb.emit(Inst::Suspend { resume: 0 });
        fb.emit(Inst::Br { to: BlockId(0) });
    });
    assert!(e.is_empty(), "{e:#?}");
}

#[test]
fn malformed_ir_is_rejected() {
    expect(FuncKind::Process, "does not end with a terminator", |fb| {
        fb.stmt(StmtKind::Null, sp());
        fb.int(1);
    });
    expect(FuncKind::Process, "terminator in the middle", |fb| {
        fb.stmt(StmtKind::Null, sp());
        fb.emit(Inst::Ret { v: None });
        fb.emit(Inst::Ret { v: None });
    });
    expect(FuncKind::Process, "expected Some(F64)", |fb| {
        fb.stmt(StmtKind::Null, sp());
        let i = fb.int(1);
        let d = fb.reg(RegTy::F64);
        fb.emit(Inst::FUn {
            op: FUnOp::Neg,
            dst: d,
            a: i,
        });
        fb.emit(Inst::Ret { v: None });
    });
    expect(FuncKind::Process, "branch to missing block", |fb| {
        fb.stmt(StmtKind::Null, sp());
        fb.emit(Inst::Br { to: BlockId(9) });
    });
    expect(FuncKind::Process, "elaboration-only instruction", |fb| {
        fb.stmt(StmtKind::Null, sp());
        let args = fb.regs(&[]);
        fb.emit(Inst::Elab {
            op: ElabOp::Signal,
            args,
            dst: None,
        });
        fb.emit(Inst::Ret { v: None });
    });
    expect(FuncKind::Function, "cannot suspend", |fb| {
        fb.stmt(StmtKind::Wait, sp());
        fb.emit(Inst::Suspend { resume: 0 });
        let z = fb.int(0);
        fb.emit(Inst::Ret { v: Some(z) });
    });
    expect(FuncKind::Process, "live across a suspension point", |fb| {
        fb.stmt(StmtKind::Wait, sp());
        let t = fb.int(5);
        fb.emit(Inst::Suspend { resume: 0 });
        fb.emit(Inst::WaitFor { t });
        fb.emit(Inst::Ret { v: None });
    });
    // A register never written (a frame may start with stale registers).
    if cfg!(debug_assertions) {
        expect(FuncKind::Process, "read before it is written", |fb| {
            fb.stmt(StmtKind::Null, sp());
            let t = fb.reg(RegTy::I64);
            fb.emit(Inst::WaitFor { t });
            fb.emit(Inst::Ret { v: None });
        });
    }
    expect(
        FuncKind::Process,
        "instruction before any statement",
        |fb| {
            fb.emit(Inst::Ret { v: None });
        },
    );
    // A slice of a signal reference takes a reference, not an integer.
    expect(FuncKind::Process, "expected Some(Sig)", |fb| {
        fb.stmt(StmtKind::Null, sp());
        let i = fb.int(1);
        let dst = fb.reg(RegTy::Sig);
        fb.emit(Inst::SigSlice {
            dst,
            sig: i,
            offset: i,
            len: i,
        });
        fb.emit(Inst::Ret { v: None });
    });
    expect(FuncKind::Procedure, "return value does not match", |fb| {
        fb.stmt(StmtKind::Return, sp());
        let z = fb.int(0);
        fb.emit(Inst::Ret { v: Some(z) });
    });
}

#[test]
fn calls_are_checked() {
    let mut pb = ProgramBuilder::new();
    let w = pb.declare("waits", FuncKind::Procedure, None);
    let f = pb.declare("fun", FuncKind::Function, None);
    let mut fb = pb.define(w);
    fb.param("x", Width::B8, ParamClass::Value, Mode::In, None);
    fb.stmt(StmtKind::Wait, sp());
    fb.emit(Inst::Suspend { resume: 0 });
    fb.emit(Inst::Ret { v: None });
    fb.finish();
    let mut fb = pb.define(f);
    fb.stmt(StmtKind::Call, sp());
    let args = fb.regs(&[]);
    fb.emit(Inst::Call {
        func: w,
        args,
        dst: None,
    });
    let z = fb.int(0);
    fb.emit(Inst::Ret { v: Some(z) });
    fb.finish();
    let e: Vec<String> = pb
        .finish()
        .unwrap_err()
        .into_iter()
        .map(|e| e.msg)
        .collect();
    assert!(
        e.iter().any(|m| m.contains("takes 1 arguments, 0 given")),
        "{e:#?}"
    );
    assert!(e.iter().any(|m| m.contains("which may suspend")), "{e:#?}");
}

#[test]
fn undefined_function() {
    let mut pb = ProgramBuilder::new();
    pb.declare("ghost", FuncKind::Function, None);
    let e = pb.finish().unwrap_err();
    assert_eq!(e[0].msg, "declared but never defined");
}

#[test]
fn flags_and_dump() {
    let mut pb = ProgramBuilder::new();
    let int = pb.ty(Ty::Int { lo: 0, hi: 9 });
    let tick = pb.declare("tick", FuncKind::Procedure, None);
    let p = pb.declare("p", FuncKind::Process, None);
    let mut fb = pb.define(tick);
    fb.stmt(StmtKind::Wait, sp());
    fb.emit(Inst::Suspend { resume: 0 });
    fb.emit(Inst::Ret { v: None });
    fb.finish();
    let mut fb = pb.define(p);
    fb.local("v", int, 8, 8);
    fb.stmt(StmtKind::Call, sp());
    let args = fb.regs(&[]);
    fb.emit(Inst::Call {
        func: tick,
        args,
        dst: None,
    });
    fb.stmt(StmtKind::VarAssign, sp());
    let a = fb.addr(AddrSpace::Shared, 0);
    let one = fb.int(1);
    fb.emit(Inst::Store {
        addr: a,
        src: one,
        w: Width::B8,
    });
    fb.emit(Inst::Br { to: BlockId(0) });
    fb.finish();
    let prog = pb.finish().unwrap();
    let pf = prog.func(p).unwrap();
    assert!(pf.flags.contains(FuncFlags::MAY_SUSPEND));
    assert!(prog.is_serial(p) && prog.uses_shared_state(p));
    assert!(!prog.is_serial(tick));
    assert_eq!(pf.resume, [2]);
    assert_eq!(
        prog.dump_func(p),
        "f1 Process `p` frame 8 regs 2 flags 0x3
b0:
     0 s1    Stmt
     1 s1    Call { func: f0, args: #0, dst: None } []
     2 s2    AddrOf { dst: r0, space: Shared, off: 0 }
     3 s2    Const { dst: r1, v: 1 }
     4 s2    Store { addr: r0, src: r1, w: B8 }
     5 s2    Br { to: b0 }
"
    );
}

#[test]
fn inst_stays_small() {
    assert_eq!(std::mem::size_of::<Inst>(), 32);
}

/// Callers of disk-file natives are serial (#82, #107); line-only textio natives are not.
#[test]
fn file_natives_make_callers_serial() {
    let mut pb = ProgramBuilder::new();
    let mut procs = vec![];
    for (i, sym) in [
        "yavs.textio.writeline.text_line",
        "yavs.implicit.file_open.std.textio.text",
        "yavs.implicit.endfile.std.textio.text",
        "yavs.env.dir_deletefile.string.r_file_delete_status",
        "yavs.textio.write.line_integer_side_width",
        "yavs.implicit.eq.integer",
    ]
    .into_iter()
    .enumerate()
    {
        let p = pb.declare(&format!("p{i}"), FuncKind::Process, None);
        let native = pb.native(sym, None);
        let mut fb = pb.define(p);
        fb.stmt(StmtKind::Call, sp());
        let args = fb.regs(&[]);
        fb.emit(Inst::CallNative {
            native,
            args,
            dst: None,
        });
        fb.emit(Inst::Ret { v: None });
        fb.finish();
        procs.push(p);
    }
    let prog = pb.finish().unwrap();
    let serial: Vec<bool> = procs.iter().map(|&p| prog.is_serial(p)).collect();
    assert_eq!(serial, [true, true, true, true, false, false]);
    // Serial for the kernel's order, but they need no shared state: the machine runs them on the
    // quick path.
    assert!(procs.iter().all(|&p| !prog.uses_shared_state(p)));
}

/// An `ieee` native takes `data, left, right, dir` for each array argument (#697).
#[test]
fn ieee_array_arguments_are_four_registers() {
    let call = |n: usize| {
        errors(FuncKind::Process, move |fb| {
            fb.stmt(StmtKind::Null, sp());
            let native = fb
                .program()
                .native("yavs.ieee.numeric_std.to_integer(unsigned)integer", None);
            let regs: Vec<Reg> = (0..n).map(|i| fb.int(i as i64)).collect();
            let args = fb.regs(&regs);
            let dst = fb.reg(RegTy::I64);
            fb.emit(Inst::CallNative {
                native,
                args,
                dst: Some(dst),
            });
            fb.emit(Inst::Ret { v: None });
        })
    };
    assert!(call(4).is_empty(), "{:#?}", call(4));
    for n in [1, 2, 5] {
        let e = call(n);
        assert!(
            e.iter()
                .any(|m| m.contains("takes 4 argument registers")
                    && m.contains(&format!("{n} given"))),
            "{n}: {e:#?}"
        );
    }
}

#[test]
fn leaf_tables_are_checked() {
    use yavs_ir::gather::{LeafKind, TabLeaf, encode};
    let leaf = |sig| TabLeaf {
        sig,
        kind: LeafKind::Scalar,
        bias: 0,
        sw: 8,
        ew: 8,
    };
    // A record of two integers, a table with `n` leaves (or bytes that are no table).
    let gather = |n: Option<u32>| {
        errors(FuncKind::Process, |fb| {
            fb.stmt(StmtKind::Null, sp());
            let int = fb.program().ty(Ty::Int { lo: 0, hi: 9 });
            let field = |name: &str| Field {
                name: name.into(),
                ty: int,
                desc: None,
            };
            let rec = fb.program().ty(Ty::Record {
                fields: vec![field("a"), field("b")],
            });
            let c = match n {
                Some(n) => encode(rec, &(0..n).map(leaf).collect::<Vec<_>>()),
                None => ConstData::Bytes(vec![1, 2, 3]),
            };
            let tab = fb.program().konst(c);
            let dst = fb.reg(RegTy::Ptr);
            fb.emit(Inst::AddrOf {
                dst,
                space: AddrSpace::Frame,
                off: 0,
            });
            fb.emit(Inst::SigGather {
                dst,
                desc: dst,
                tab,
                part: None,
            });
            fb.emit(Inst::Br { to: BlockId(0) });
        })
    };
    let leafy = |e: &[String]| e.iter().any(|m| m.contains("is not a leaf table"));
    assert!(!leafy(&gather(Some(2))), "a table that fits its type");
    for bad in [Some(1), Some(3), None] {
        let e = gather(bad);
        assert!(leafy(&e), "{bad:?}: {e:#?}");
    }
}
