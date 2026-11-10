//! `YAVS_CLIFT_BLOCKCOV`: a block the generated code entered is marked, one it did not is not.
#![cfg(feature = "clift")]
#![allow(unsafe_code)]

use std::sync::{Arc, Once};
use std::time::{Duration, Instant};

use yavs_clift::{First, blockcov_snapshot};
use yavs_diag::{FileId, Span};
use yavs_interp::{Bindings, InstData, Runtime};
use yavs_ir::*;
use yavs_rt::{AutoTier, Heat};

fn on() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| {
        // SAFETY: first thing in every test of this binary, before any thread reads the environment.
        unsafe { std::env::set_var("YAVS_CLIFT_BLOCKCOV", env!("CARGO_TARGET_TMPDIR")) };
        // The JIT's own memory, as on hosts without the huge arena: a dropped module frees its
        // tables, so the dump must not read them there.
        unsafe { std::env::set_var("YAVS_CLIFT_HUGE", "0") };
    });
}

/// `name(n)`: for i in 0 to n loop if i < 0 then acc := acc - 1; end if; acc := acc + i; end loop.
/// The `then` branch never runs.
fn guarded(name: &str) -> (Program, FuncId) {
    let mut pb = ProgramBuilder::new();
    let f = pb.declare(name, FuncKind::Function, None);
    let mut fb = pb.define(f);
    let sn = fb.param("n", Width::B8, ParamClass::Value, Mode::In, None);
    let (head, body, neg, cont, done) =
        (fb.block(), fb.block(), fb.block(), fb.block(), fb.block());
    fb.stmt(StmtKind::Return, Span::new(FileId(0), 0, 1));
    let pn = fb.addr(AddrSpace::Frame, sn);
    let n = fb.reg(RegTy::I64);
    fb.emit(Inst::Load {
        dst: n,
        addr: pn,
        w: Width::B8,
    });
    let (i, acc, one, zero) = (fb.int(0), fb.int(0), fb.int(1), fb.int(0));
    fb.emit(Inst::Br { to: head });
    fb.at(head);
    let c = fb.reg(RegTy::I64);
    fb.emit(Inst::Cmp {
        op: CmpOp::Le,
        dst: c,
        a: i,
        b: n,
    });
    fb.emit(Inst::CondBr {
        c,
        t: body,
        f: done,
    });
    fb.at(body);
    let c2 = fb.reg(RegTy::I64);
    fb.emit(Inst::Cmp {
        op: CmpOp::Lt,
        dst: c2,
        a: i,
        b: zero,
    });
    fb.emit(Inst::CondBr {
        c: c2,
        t: neg,
        f: cont,
    });
    fb.at(neg);
    fb.emit(Inst::Bin {
        op: IntOp::Sub,
        dst: acc,
        a: acc,
        b: one,
    });
    fb.emit(Inst::Br { to: cont });
    fb.at(cont);
    fb.emit(Inst::Bin {
        op: IntOp::Add,
        dst: acc,
        a: acc,
        b: i,
    });
    fb.emit(Inst::Bin {
        op: IntOp::Add,
        dst: i,
        a: i,
        b: one,
    });
    fb.emit(Inst::Br { to: head });
    fb.at(done);
    fb.emit(Inst::Ret { v: Some(acc) });
    fb.finish();
    (pb.finish().unwrap(), f)
}

/// The blocks of the bodies of `name`: some block ran, and the block of `neg` (IR block 3) never.
fn check(name: &str) {
    let bodies: Vec<_> = blockcov_snapshot()
        .into_iter()
        .filter(|b| b.name.ends_with(&format!("_{name}")))
        .collect();
    assert!(!bodies.is_empty(), "no body of {name}");
    let ran = |ir: u32| {
        bodies
            .iter()
            .flat_map(|b| &b.blocks)
            .any(|((_, id), r)| *id == Some(ir) && *r)
    };
    let listed = |ir: u32| {
        bodies
            .iter()
            .flat_map(|b| &b.blocks)
            .any(|((_, id), _)| *id == Some(ir))
    };
    assert!(listed(3), "the never-taken block is listed");
    assert!(ran(2), "the loop body ran");
    assert!(!ran(3), "the never-taken block did not");
    assert!(ran(4), "the join ran");
}

#[test]
fn eager_tier_marks_the_blocks_that_ran() {
    on();
    let (prog, f) = guarded("cov_eager");
    let tier = yavs_clift::compile(&prog, usize::MAX).unwrap();
    assert!(tier.notes.is_empty(), "{:?}", tier.notes);
    let rt = Runtime::new(Arc::new(prog), Bindings::default()).unwrap();
    assert!(rt.set_tier(Arc::new(tier)));
    let r = yavs_interp::call(rt, f, Arc::new(InstData::default()), &[1000]).unwrap();
    assert_eq!(r, Some(500_500));
    check("cov_eager");
}

#[test]
fn auto_tier_marks_the_blocks_that_ran() {
    on();
    let (prog, f) = guarded("cov_auto");
    let rt = Runtime::new(Arc::new(prog), Bindings::default()).unwrap();
    let hot = Heat {
        base: 1,
        per_instr: 0,
        period: 100,
        budget: 0,
    };
    let note: yavs_rt::Note = Box::new(|_, _| {});
    let tier =
        AutoTier::start_with(&rt, usize::MAX, hot, note, Box::new(First::new().unwrap())).unwrap();
    let t0 = Instant::now();
    while tier.compiled() == 0 {
        assert!(t0.elapsed() < Duration::from_secs(60), "never compiled");
        let r = yavs_interp::call(rt.clone(), f, Arc::new(InstData::default()), &[100_000]);
        assert_eq!(r.unwrap(), Some(5_000_050_000));
    }
    let r = yavs_interp::call(rt.clone(), f, Arc::new(InstData::default()), &[100_000]);
    assert_eq!(r.unwrap(), Some(5_000_050_000));
    tier.stop(true);
    check("cov_auto");
}

#[test]
fn the_dump_lists_every_block() {
    on();
    let (prog, _) = guarded("cov_dump");
    yavs_clift::compile(&prog, usize::MAX).unwrap();
    yavs_clift::blockcov_write();
    let file = std::path::Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join(format!("cov-{}.tsv", std::process::id()));
    let text = std::fs::read_to_string(file).unwrap();
    let rows: Vec<_> = text.lines().filter(|l| l.contains("_cov_dump\t")).collect();
    assert!(rows.len() >= 5, "{rows:?}");
    // Body, pc, IR block, ran, and the body's key for `YAVS_CLIFT_COLD`.
    assert!(
        rows.iter().all(|r| {
            let f: Vec<&str> = r.split('\t').collect();
            f.len() == 5 && f[3] == "0" && f[4] == &f[0][..2]
        }),
        "nothing ran: {rows:?}"
    );
}
