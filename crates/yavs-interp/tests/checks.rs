//! sim1-compatible check messages, real checks, 'value, to_hstring and thunks with a value.

mod common;

use std::sync::Arc;

use common::*;
use yavs_interp::{Bindings, InstData, Runtime};
use yavs_ir::*;
use yavs_kernel::*;

/// Run a parameterless `kind` built by `body` (which returns its result register).
fn eval(
    kind: FuncKind,
    body: impl FnOnce(&mut FuncBuilder<'_>) -> Reg,
) -> Result<Option<u64>, String> {
    let mut pb = ProgramBuilder::new();
    let f = pb.declare("f", kind, None);
    let mut fb = pb.define(f);
    stmt(&mut fb, StmtKind::Return);
    let r = body(&mut fb);
    fb.emit(Inst::Ret { v: Some(r) });
    fb.finish();
    let prog = Arc::new(pb.finish().map_err(|e| format!("{e:?}"))?);
    let rt = Runtime::new(prog, Bindings::default()).map_err(|e| e.to_string())?;
    yavs_interp::call(rt, f, Arc::new(InstData::default()), &[]).map_err(|e| match &e.kind {
        ErrorKind::Runtime(m) => m.clone(),
        _ => e.to_string(),
    })
}

fn fconst(fb: &mut FuncBuilder<'_>, v: f64) -> Reg {
    let r = fb.reg(RegTy::F64);
    fb.emit(Inst::ConstF { dst: r, v });
    r
}

#[test]
fn messages() {
    let integer = |fb: &mut FuncBuilder<'_>| (fb.int(i32::MIN.into()), fb.int(i32::MAX.into()));
    let e = eval(FuncKind::Function, |fb| {
        let w = fb.program().check_msg_for("", Dir::To, "INTEGER");
        let (a, b) = (fb.int(i32::MAX.into()), fb.int(1));
        let (lo, hi) = integer(fb);
        let dst = fb.reg(RegTy::I64);
        fb.emit(Inst::BinRange {
            op: IntOp::Add,
            dst,
            a,
            b,
            lo,
            hi,
            what: w,
        });
        dst
    });
    assert_eq!(
        e,
        Err("result of 2147483647 + 1 cannot be represented as INTEGER".into())
    );
    let e = eval(FuncKind::Function, |fb| {
        let (a, b) = (fb.int(5), fb.int(0));
        bin(fb, IntOp::Div, a, b)
    });
    assert_eq!(e, Err("division by zero".into()));
    let e = eval(FuncKind::Function, |fb| {
        let w = fb.program().check_msg_for("variable P", Dir::To, "PCT");
        let (v, lo, hi) = (fb.int(101), fb.int(0), fb.int(100));
        fb.emit(Inst::Check {
            kind: CheckKind::Range,
            v,
            lo,
            hi,
            what: w,
        });
        v
    });
    assert_eq!(
        e,
        Err("value 101 outside of PCT range 0 to 100 for variable P".into())
    );
    let e = eval(FuncKind::Function, |fb| {
        let w = fb.program().check_msg_for("", Dir::To, "INTEGER");
        let x = fconst(fb, 3e10);
        let (lo, hi) = integer(fb);
        let dst = fb.reg(RegTy::I64);
        fb.emit(Inst::FToIRange {
            dst,
            src: x,
            lo,
            hi,
            what: w,
        });
        dst
    });
    assert_eq!(
        e,
        Err("value 30000000000 outside of INTEGER range -2147483648 to 2147483647".into())
    );
    let e = eval(FuncKind::Function, |fb| {
        let w = fb.program().check_msg_for("", Dir::To, "INTEGER");
        let x = fconst(fb, 1e30);
        let (lo, hi) = integer(fb);
        let dst = fb.reg(RegTy::I64);
        fb.emit(Inst::FToIRange {
            dst,
            src: x,
            lo,
            hi,
            what: w,
        });
        dst
    });
    assert_eq!(
        e,
        Err("value 1e+30 outside of INTEGER range -2147483648 to 2147483647".into())
    );
    let e = eval(FuncKind::Function, |fb| {
        let w = fb.program().check_msg_for("signal R", Dir::To, "UNIT");
        let (v, lo, hi) = (fconst(fb, 1.5), fconst(fb, 0.0), fconst(fb, 1.0));
        fb.emit(Inst::FCheck { v, lo, hi, what: w });
        fb.int(0)
    });
    assert_eq!(
        e,
        Err("value 1.5 outside of UNIT range 0 to 1 for signal R".into())
    );
    // In range: the value passes through.
    let ok = eval(FuncKind::Function, |fb| {
        let w = fb.program().check_msg_for("", Dir::To, "INTEGER");
        let x = fconst(fb, 2.5);
        let (lo, hi) = integer(fb);
        let dst = fb.reg(RegTy::I64);
        fb.emit(Inst::FToIRange {
            dst,
            src: x,
            lo,
            hi,
            what: w,
        });
        dst
    });
    assert_eq!(ok, Ok(Some(3)));
}

#[test]
fn value_attribute_and_hstring() {
    let r = eval(FuncKind::Thunk, |fb| {
        let int = fb.program().ty(Ty::Int { lo: 0, hi: 100 });
        let s = fat_str(fb, b"  42 ");
        native(fb, "yavs.attr.value", Some(int), &[s], true).unwrap()
    });
    assert_eq!(r, Ok(Some(42)));
    let r = eval(FuncKind::Function, |fb| {
        let int = fb.program().ty(Ty::Int { lo: 0, hi: 100 });
        let s = fat_str(fb, b"420");
        native(fb, "yavs.attr.value", Some(int), &[s], true).unwrap()
    });
    assert!(
        r.as_ref().is_err_and(|m| m.starts_with("'value: ")),
        "{r:?}"
    );
    let r = eval(FuncKind::Function, |fb| {
        let e = fb.program().ty(Ty::Enum {
            lits: vec!["idle".into(), "Busy".into()],
            logic: None,
        });
        let s = fat_str(fb, b"BUSY");
        native(fb, "yavs.attr.value", Some(e), &[s], true).unwrap()
    });
    assert_eq!(r, Ok(Some(1)));
    // to_hstring of bit_vector "1100" is "C"; the result's first byte is returned.
    let r = eval(FuncKind::Function, |fb| {
        let bit = fb.program().ty(Ty::Enum {
            lits: vec!["'0'".into(), "'1'".into()],
            logic: Some(LogicKind::Bit),
        });
        let bv = fb.program().ty(Ty::Array {
            elem: bit,
            elem_desc: None,
            dims: 1,
        });
        let c = fb.program().konst(ConstData::Logic(vec![(2, 2), (2, 3)]));
        let data = fb.addr(AddrSpace::Const(c), 0);
        let v = fat_of(fb, data, 0, 3, false);
        let n = fb
            .program()
            .native("yavs.implicit.to_hstring.bit_vector", Some(bv));
        let args = fb.regs(&[v]);
        let fat = fb.reg(RegTy::Ptr);
        fb.emit(Inst::CallNative {
            native: n,
            args,
            dst: Some(fat),
        });
        let (d, _) = fat_parts(fb, fat);
        let ch = fb.reg(RegTy::I64);
        fb.emit(Inst::Load {
            dst: ch,
            addr: d,
            w: Width::B1,
        });
        ch
    });
    assert_eq!(r, Ok(Some(u64::from(b'C'))));
}

/// A failure report ends the simulation and the process runs no further.
#[test]
fn failure_report_halts_the_process() {
    let mut pb = ProgramBuilder::new();
    let p = pb.declare("p", FuncKind::Process, None);
    let mut fb = pb.define(p);
    stmt(&mut fb, StmtKind::Report);
    let c = fb.program().konst(ConstData::Bytes(b"boom".to_vec()));
    let (a, n, sev) = (fb.addr(AddrSpace::Const(c), 0), fb.int(4), fb.int(3));
    fb.emit(Inst::Report {
        msg: a,
        len: n,
        severity: sev,
    });
    stmt(&mut fb, StmtKind::Report);
    report(&mut fb, "never");
    wait_forever(&mut fb);
    fb.finish();
    let mut d = Design::new(pb);
    d.procs = vec![ProcSpec {
        name: "p",
        func: p,
        drives: vec![],
    }];
    let (lines, r) = d.run(Time::ns(1));
    assert_eq!(
        r,
        Ok(RunStatus::Ended(EndReason::Severity(Severity::Failure)))
    );
    assert_eq!(lines, ["0 fs +0 p0 Failure: boom"]);
}
