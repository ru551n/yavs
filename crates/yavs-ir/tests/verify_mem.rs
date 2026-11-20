//! The undefined-use check needs memory roughly linear in the function, not blocks times
//! registers (#1168). Its own test binary: it installs a counting allocator.

use yavs_diag::{FileId, Span};
use yavs_ir::*;

/// Peak resident set of this process in KiB (`None` off Linux).
fn hwm_kib() -> Option<usize> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    let l = s.lines().find(|l| l.starts_with("VmHWM:"))?;
    l.split_whitespace().nth(1)?.parse().ok()
}

/// A chain of `n` blocks, each defining one register that the next block reads.
#[test]
fn long_chain_verifies_in_linear_memory() {
    const N: u32 = 20_000;
    let mut pb = ProgramBuilder::new();
    let f = pb.declare("f", FuncKind::Function, None);
    let mut fb = pb.define(f);
    fb.stmt(StmtKind::Return, Span::new(FileId(0), 0, 1));
    let mut prev = fb.int(1);
    for _ in 0..N {
        let next = fb.block();
        let d = fb.reg(RegTy::I64);
        fb.emit(Inst::Bin {
            op: IntOp::Add,
            dst: d,
            a: prev,
            b: prev,
        });
        fb.emit(Inst::CondBr {
            c: d,
            t: next,
            f: next,
        });
        fb.at(next);
        prev = d;
    }
    fb.emit(Inst::Ret { v: Some(prev) });
    fb.finish();
    let Some(base) = hwm_kib() else { return };
    pb.finish().expect("valid chain");
    let grew = hwm_kib().unwrap() - base;
    // The dense sets took N * N / 8 = 50 MB; the rest of the work is a few MB.
    assert!(grew < 16 << 10, "verifier raised the peak by {grew} KiB");
}
