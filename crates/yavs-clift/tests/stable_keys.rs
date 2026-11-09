//! Compiled code stays found: an unchanged program makes the same bodies, whatever the policy
//! compiles with a function in this run (#1035). One test: the store is one per process.
#![cfg(feature = "clift")]

use std::path::Path;

use yavs_clift::First;
use yavs_diag::{FileId, Span};
use yavs_ir::*;
use yavs_rt::native::Part;
use yavs_rt::{Generator, OptLevel};

/// `inc(a)`, a chain of additions ending in `step`, too long for the optimiser to inline, and
/// `caller(a) = inc(a)`.
fn program(step: i64) -> (Program, FuncId, FuncId) {
    let mut pb = ProgramBuilder::new();
    let inc = pb.declare("inc", FuncKind::Function, None);
    let caller = pb.declare("caller", FuncKind::Function, None);
    let arg = |fb: &mut FuncBuilder<'_>| {
        let s = fb.param("a", Width::B8, ParamClass::Value, Mode::In, None);
        fb.stmt(StmtKind::Return, Span::new(FileId(0), 0, 1));
        let p = fb.addr(AddrSpace::Frame, s);
        let dst = fb.reg(RegTy::I64);
        fb.emit(Inst::Load {
            dst,
            addr: p,
            w: Width::B8,
        });
        dst
    };
    {
        let mut fb = pb.define(inc);
        let a = arg(&mut fb);
        let mut acc = fb.int(step);
        for _ in 0..80 {
            let dst = fb.reg(RegTy::I64);
            fb.emit(Inst::Bin {
                op: IntOp::Add,
                dst,
                a,
                b: acc,
            });
            acc = dst;
        }
        fb.emit(Inst::Ret { v: Some(acc) });
        fb.finish();
    }
    {
        let mut fb = pb.define(caller);
        let a = arg(&mut fb);
        let args = fb.regs(&[a]);
        let dst = fb.reg(RegTy::I64);
        fb.emit(Inst::Call {
            func: inc,
            args,
            dst: Some(dst),
        });
        fb.emit(Inst::Ret { v: Some(dst) });
        fb.finish();
    }
    (pb.finish().expect("verified"), caller, inc)
}

/// A run's compile of `caller` with `tree`.
fn compile(prog: &Program, caller: FuncId, tree: &[FuncId]) {
    let mut g = First::new().expect("a code generator");
    g.tree(tree);
    let f = prog.func(caller).expect("the function");
    g.compile(prog, caller, f, OptLevel::O2, Part::Native)
        .expect("compiled");
}

fn compiled(dir: &Path) -> usize {
    std::fs::read_dir(dir.join("yavs-cache").join("obj")).map_or(0, |d| {
        d.filter(|e| {
            e.as_ref()
                .is_ok_and(|e| e.file_name().to_string_lossy().starts_with("clif-"))
        })
        .count()
    })
}

#[test]
fn a_body_does_not_follow_what_compiles_with_it() {
    let tmp = std::env::temp_dir().join(format!("yavs_stable_keys_{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("scratch");
    yavs_clift::set_cache(Some(&tmp));

    let (prog, caller, inc) = program(1);
    // First run: the callee compiles with the caller, so the caller calls it directly.
    compile(&prog, caller, &[caller, inc]);
    let n = compiled(&tmp);
    assert!(n >= 2, "caller and callee stored: {n}");
    let (hits, stored) = yavs_clift::cache_stats();

    // Later runs: the samples may leave the callee out. Nothing new, all from the store.
    for tree in [&[caller][..], &[caller, inc][..]] {
        compile(&prog, caller, tree);
        assert_eq!(compiled(&tmp), n, "tree {tree:?} made another body");
    }
    let (hits2, stored2) = yavs_clift::cache_stats();
    assert_eq!(stored2, stored, "nothing compiled again");
    assert!(hits2 > hits, "linked from the store");

    // Another design misses.
    let (other, caller2, inc2) = program(2);
    compile(&other, caller2, &[caller2, inc2]);
    assert!(compiled(&tmp) > n, "another callee is another entry");

    yavs_clift::set_cache(None);
    let _ = std::fs::remove_dir_all(&tmp);
}
