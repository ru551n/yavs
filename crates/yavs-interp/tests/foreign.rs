//! VHPIDIRECT marshalling of arrays, records and access values, with copy-back.

mod common;

use std::sync::Arc;

use common::*;
use yavs_ffi::{CType, ForeignFn, testing};
use yavs_interp::{Bindings, InstData, Runtime};
use yavs_ir::*;
use yavs_kernel::{ErrorKind, LogicVec, SimError};
use yavs_logic::StdULogic;

/// Start `fun() return integer` calling foreign `name`; returns the builder pieces.
fn setup(
    pb: &mut ProgramBuilder,
    name: &str,
    args: Vec<ForeignArg>,
    ret: Option<CType>,
) -> (ForeignId, FuncId) {
    let x = pb.foreign(ForeignRef {
        name: name.into(),
        symbol: name.into(),
        args,
        ret,
        model: None,
    });
    (x, pb.declare("fun", FuncKind::Function, None))
}

fn finish(pb: ProgramBuilder, fun: FuncId, x: ForeignId, f: ForeignFn) -> Result<i64, SimError> {
    let prog = Arc::new(pb.finish().unwrap());
    let b = Bindings {
        foreign: vec![(x, f)],
        ..Bindings::default()
    };
    let rt = Runtime::new(prog, b)?;
    yavs_interp::call(rt, fun, Arc::new(InstData::default()), &[]).map(|v| v.unwrap() as i64)
}

fn alloc(fb: &mut FuncBuilder<'_>, n: i64) -> Reg {
    let size = fb.int(n);
    let p = fb.reg(RegTy::Ptr);
    fb.emit(Inst::Alloc { dst: p, size });
    p
}

fn at(fb: &mut FuncBuilder<'_>, base: Reg, off: i64) -> Reg {
    let o = fb.int(off);
    let a = fb.reg(RegTy::Ptr);
    fb.emit(Inst::Index {
        dst: a,
        base,
        pos: o,
        stride: 1,
    });
    a
}

fn ld(fb: &mut FuncBuilder<'_>, a: Reg, w: Width) -> Reg {
    let v = fb.reg(RegTy::I64);
    fb.emit(Inst::Load { dst: v, addr: a, w });
    v
}

fn st(fb: &mut FuncBuilder<'_>, a: Reg, v: Reg, w: Width) {
    fb.emit(Inst::Store { addr: a, src: v, w });
}

/// A fat pair `[data, dope]` with one dimension `l .. r`.
fn fat(fb: &mut FuncBuilder<'_>, data: Reg, l: i64, r: i64, downto: bool) -> Reg {
    let fp = alloc(fb, 40);
    let dope = at(fb, fp, 16);
    let parts = [fb.int(l), fb.int(r), fb.int(downto.into())];
    let parts = fb.regs(&parts);
    fb.emit(Inst::MakeDope { at: dope, parts });
    st(fb, fp, data, Width::B8);
    let a = at(fb, fp, 8);
    st(fb, a, dope, Width::B8);
    fp
}

fn foreign(fb: &mut FuncBuilder<'_>, x: ForeignId, args: &[Reg]) -> Reg {
    let args = fb.regs(args);
    let r = fb.reg(RegTy::I64);
    fb.emit(Inst::CallForeign {
        foreign: x,
        args,
        dst: Some(r),
    });
    r
}

/// `integer_vector` (unconstrained) passed as pointer plus length, doubled in place by C.
#[test]
fn unconstrained_integer_array_with_copy_back() {
    let mut pb = ProgramBuilder::new();
    let int = integer(&mut pb);
    let arr = pb.ty(Ty::Array {
        elem: int,
        elem_desc: None,
        dims: 1,
    });
    let arg = ForeignArg::Array {
        ty: arr,
        lengths: 1,
        copy_back: true,
    };
    let (x, fun) = setup(&mut pb, "sum_double", vec![arg], Some(CType::I64));
    let mut fb = pb.define(fun);
    stmt(&mut fb, StmtKind::Return);
    // `integer` elements are 4 bytes (`Ty::elem_size`).
    let data = alloc(&mut fb, 12);
    for k in 0..3 {
        let a = at(&mut fb, data, 4 * k);
        let v = fb.int(k + 1);
        st(&mut fb, a, v, Width::B4);
    }
    let fp = fat(&mut fb, data, 1, 3, false);
    let s = foreign(&mut fb, x, &[fp]);
    let a = at(&mut fb, data, 8);
    let last = ld(&mut fb, a, Width::B4);
    let k = fb.int(1000);
    let s = bin(&mut fb, IntOp::Mul, s, k);
    let r = bin(&mut fb, IntOp::Add, s, last);
    fb.emit(Inst::Ret { v: Some(r) });
    fb.finish();
    let f = ForeignFn::from_fn(testing::sum_double as extern "C" fn(_, i64) -> i64);
    assert_eq!(finish(pb, fun, x, f), Ok(6006));
}

/// `std_logic_vector(7 downto 0)` (constrained): one code byte per element, left to right.
#[test]
fn logic_array_codes_left_to_right() {
    let code = |s: &str| -> Vec<u8> {
        s.chars()
            .map(|c| StdULogic::from_char(c).unwrap() as u8)
            .collect()
    };
    // Planes hold element 0 at the right end of the VHDL value.
    let planes = |s: &str| {
        let c = code(s);
        let v = LogicVec::from_fn(c.len(), |i| StdULogic::from_code_pub(c[c.len() - 1 - i]));
        let mut runs: Vec<(u64, u8)> = Vec::new();
        for e in v.iter() {
            runs.push((1, e as u8));
        }
        ConstData::Logic(runs)
    };
    let mut pb = ProgramBuilder::new();
    let sl = pb.ty(Ty::Enum {
        lits: "UX01ZWLH-".chars().map(|c| format!("'{c}'")).collect(),
        logic: Some(LogicKind::StdULogic),
    });
    let slv = pb.ty(Ty::Array {
        elem: sl,
        elem_desc: None,
        dims: 1,
    });
    let input = pb.konst(planes("01ZX1100"));
    let want = pb.konst(planes("10XX0011"));
    let arg = ForeignArg::Array {
        ty: slv,
        lengths: 0,
        copy_back: true,
    };
    let (x, fun) = setup(&mut pb, "invert8", vec![arg], None);
    let mut fb = pb.define(fun);
    stmt(&mut fb, StmtKind::Return);
    let data = alloc(&mut fb, 32);
    fb.emit(Inst::CopyConst {
        dst: data,
        c: input,
    });
    let fp = fat(&mut fb, data, 7, 0, true);
    foreign(&mut fb, x, &[fp]);
    let w = fb.addr(AddrSpace::Const(want), 0);
    let n = fb.int(8);
    let eq = fb.reg(RegTy::I64);
    fb.emit(Inst::LEq {
        dst: eq,
        a: data,
        b: w,
        n,
    });
    fb.emit(Inst::Ret { v: Some(eq) });
    fb.finish();
    let f = ForeignFn::from_fn(testing::invert8 as extern "C" fn(_));
    assert_eq!(finish(pb, fun, x, f), Ok(1));
}

fn record_ty(pb: &mut ProgramBuilder, fields: &[(&str, TyId)]) -> TyId {
    pb.ty(Ty::Record {
        fields: fields
            .iter()
            .map(|&(n, ty)| Field {
                name: n.into(),
                ty,
                desc: None,
            })
            .collect(),
    })
}

fn boolean(pb: &mut ProgramBuilder) -> TyId {
    pb.ty(Ty::Enum {
        lits: vec!["false".into(), "true".into()],
        logic: Some(LogicKind::Boolean),
    })
}

/// A record `(a: integer; b: real; c: boolean)` as a naturally aligned C struct, both ways.
#[test]
fn record_with_copy_back() {
    let mut pb = ProgramBuilder::new();
    let int = integer(&mut pb);
    let real = pb.ty(Ty::Real);
    let b = boolean(&mut pb);
    let rec = record_ty(&mut pb, &[("a", int), ("b", real), ("c", b)]);
    let arg = ForeignArg::Record {
        ty: rec,
        copy_back: true,
    };
    let (x, fun) = setup(&mut pb, "rec_bump", vec![arg], None);
    let mut fb = pb.define(fun);
    stmt(&mut fb, StmtKind::Return);
    // VHDL layout: a at 0, b at 8, c at 16.
    let r = alloc(&mut fb, 24);
    let four = fb.int(4);
    st(&mut fb, r, four, Width::B8);
    let bb = at(&mut fb, r, 8);
    let half = fb.reg(RegTy::F64);
    fb.emit(Inst::ConstF { dst: half, v: 2.5 });
    st(&mut fb, bb, half, Width::B8);
    foreign(&mut fb, x, &[r]);
    let a = ld(&mut fb, r, Width::B8);
    let bv = fb.reg(RegTy::F64);
    fb.emit(Inst::Load {
        dst: bv,
        addr: bb,
        w: Width::B8,
    });
    let bi = fb.reg(RegTy::I64);
    fb.emit(Inst::FToI { dst: bi, src: bv });
    let ca = at(&mut fb, r, 16);
    let c = ld(&mut fb, ca, Width::B1);
    let h = fb.int(100);
    let t = fb.int(10);
    let a = bin(&mut fb, IntOp::Mul, a, h);
    let bi = bin(&mut fb, IntOp::Mul, bi, t);
    let s = bin(&mut fb, IntOp::Add, a, bi);
    let s = bin(&mut fb, IntOp::Add, s, c);
    fb.emit(Inst::Ret { v: Some(s) });
    fb.finish();
    let f = ForeignFn::from_fn(testing::rec_bump as extern "C" fn(_));
    assert_eq!(finish(pb, fun, x, f), Ok(551));
}

/// An access value is a pointer to the designated object (NULL for null).
#[test]
fn access_values() {
    let mut pb = ProgramBuilder::new();
    let int = integer(&mut pb);
    let what = pb.check_msg("p.all", Dir::To);
    let (x, fun) = setup(&mut pb, "poke", vec![ForeignArg::Access], Some(CType::I32));
    let mut fb = pb.define(fun);
    stmt(&mut fb, StmtKind::Return);
    let init = alloc(&mut fb, 8);
    let eight = fb.int(8);
    let h = fb.reg(RegTy::Handle);
    fb.emit(Inst::New {
        dst: h,
        ty: int,
        init,
        size: eight,
        shared: false,
    });
    let r0 = foreign(&mut fb, x, &[h]);
    let null = fb.int(0);
    let r1 = foreign(&mut fb, x, &[null]);
    let obj = fb.reg(RegTy::Ptr);
    fb.emit(Inst::Deref {
        dst: obj,
        handle: h,
        what,
    });
    let v = ld(&mut fb, obj, Width::B8);
    let k = fb.int(10);
    let v = bin(&mut fb, IntOp::Mul, v, k);
    let v = bin(&mut fb, IntOp::Add, v, r1);
    let v = bin(&mut fb, IntOp::Add, v, r0);
    fb.emit(Inst::Ret { v: Some(v) });
    fb.finish();
    let f = ForeignFn::from_fn(testing::poke as extern "C" fn(_) -> i32);
    assert_eq!(finish(pb, fun, x, f), Ok(991));
}

/// C writing a value outside the VHDL type into an `out` record field is a runtime error.
#[test]
fn invalid_copy_back_is_an_error() {
    let mut pb = ProgramBuilder::new();
    let b = boolean(&mut pb);
    let rec = record_ty(&mut pb, &[("c", b)]);
    let arg = ForeignArg::Record {
        ty: rec,
        copy_back: true,
    };
    let (x, fun) = setup(&mut pb, "bad_bool", vec![arg], None);
    let mut fb = pb.define(fun);
    stmt(&mut fb, StmtKind::Return);
    let r = alloc(&mut fb, 8);
    foreign(&mut fb, x, &[r]);
    let z = fb.int(0);
    fb.emit(Inst::Ret { v: Some(z) });
    fb.finish();
    let f = ForeignFn::from_fn(testing::bad_bool as extern "C" fn(_));
    let e = finish(pb, fun, x, f).unwrap_err();
    assert_eq!(
        e.kind,
        ErrorKind::Runtime(
            "foreign `bad_bool` wrote an invalid value to argument 1: 7 is not a position of \
             the type"
                .into()
        )
    );
}
