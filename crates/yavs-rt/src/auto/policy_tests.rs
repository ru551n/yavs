//! The tier-up policy of `--tier=auto` (`Tune::first`): what becomes hot and when, what a compile costs and repays,
//! copies of processes, and the publication of native-frame code. The worker runs on programs
//! built here and on `Look`s made up here; the generator is a stub, since none of these decisions
//! compiles anything. Tests of the LLVM configuration of the policy were removed with that tier.

use super::{Part, models};
use yavs_diag::{FileId, Span};
use yavs_ir::{Inst, ProgramBuilder, StmtKind};

/// A generator that compiles nothing: the policy only asks it what it is.
pub(super) struct Stub;

impl crate::Generator for Stub {
    fn compile(
        &mut self,
        _: &yavs_ir::Program,
        _: yavs_ir::FuncId,
        _: &yavs_ir::Func,
        _: Part,
    ) -> Result<crate::native::Compiled, String> {
        Err("stub".into())
    }
}

/// A function of `n` frees whose last block loops on itself: a compute loop for
/// `compute`, whatever its size.
fn padded_loop(n: usize) -> (yavs_ir::Program, yavs_ir::FuncId) {
    let mut pb = ProgramBuilder::new();
    let f = pb.declare("f", yavs_ir::FuncKind::Procedure, None);
    let mut fb = pb.define(f);
    let spin = fb.block();
    fb.stmt(StmtKind::Return, Span::new(FileId(0), 0, 1));
    let ptr = fb.reg(yavs_ir::RegTy::Ptr);
    fb.emit(Inst::AddrOf {
        dst: ptr,
        space: yavs_ir::AddrSpace::Frame,
        off: 0,
    });
    for _ in 0..n {
        fb.emit(Inst::Free { ptr });
    }
    fb.emit(Inst::Br { to: spin });
    fb.at(spin);
    fb.emit(Inst::Br { to: spin });
    fb.finish();
    (pb.finish().unwrap(), f)
}

/// `padded_loop` with the padding inside the loop: a large function that is all loop.
fn padding_in_loop(n: usize) -> (yavs_ir::Program, yavs_ir::FuncId) {
    let mut pb = ProgramBuilder::new();
    let f = pb.declare("f", yavs_ir::FuncKind::Procedure, None);
    let mut fb = pb.define(f);
    let body = fb.block();
    fb.stmt(StmtKind::Return, Span::new(FileId(0), 0, 1));
    let ptr = fb.reg(yavs_ir::RegTy::Ptr);
    fb.emit(Inst::AddrOf {
        dst: ptr,
        space: yavs_ir::AddrSpace::Frame,
        off: 0,
    });
    fb.emit(Inst::Br { to: body });
    fb.at(body);
    for _ in 0..n {
        fb.emit(Inst::Free { ptr });
    }
    fb.emit(Inst::Br { to: body });
    fb.finish();
    (pb.finish().unwrap(), f)
}

/// A process whose activation (after its `Suspend`) runs `ops` inline additions, an address
/// and `outs` instructions the compiled code hands to the interpreter (`Free`, analysed only).
fn process(ops: usize, outs: usize) -> (yavs_ir::Program, yavs_ir::FuncId) {
    let mut pb = ProgramBuilder::new();
    let f = pb.declare("p", yavs_ir::FuncKind::Process, None);
    let mut fb = pb.define(f);
    let body = fb.block();
    fb.stmt(StmtKind::Wait, Span::new(FileId(0), 0, 1));
    fb.emit(Inst::Br { to: body });
    fb.at(body);
    fb.emit(Inst::Suspend { resume: 0 });
    let x = fb.int(1);
    for _ in 0..ops {
        fb.emit(Inst::Bin {
            op: yavs_ir::IntOp::Add,
            dst: x,
            a: x,
            b: x,
        });
    }
    let ptr = fb.reg(yavs_ir::RegTy::Ptr);
    fb.emit(Inst::AddrOf {
        dst: ptr,
        space: yavs_ir::AddrSpace::Frame,
        off: 0,
    });
    for _ in 0..outs {
        fb.emit(Inst::Free { ptr });
    }
    fb.emit(Inst::Br { to: body });
    fb.finish();
    (pb.finish().unwrap(), f)
}

/// A worker over the program of `padded_loop(n)`.
fn worker(n: usize) -> (super::Worker, yavs_ir::FuncId) {
    worker_on(padded_loop(n))
}

/// A worker over `prog`.
fn worker_on((prog, f): (yavs_ir::Program, yavs_ir::FuncId)) -> (super::Worker, yavs_ir::FuncId) {
    let rt = yavs_interp::Runtime::new(std::sync::Arc::new(prog), Default::default()).unwrap();
    let table: std::sync::Arc<[super::Entry]> = (0..rt.prog.funcs.len())
        .map(|_| super::Entry::default())
        .collect();
    let tab = || -> std::sync::Arc<[std::sync::atomic::AtomicUsize]> {
        (0..rt.prog.funcs.len())
            .map(|_| std::sync::atomic::AtomicUsize::new(0))
            .collect()
    };
    let w = super::Worker::new(
        rt.prog.clone(),
        table,
        tab(),
        tab(),
        rt.native_flags(),
        super::Heat::DEFAULT,
        usize::MAX,
        Box::new(Stub),
        Box::new(|_, _| {}),
    );
    (w, f)
}

/// A worker over `prog` with the default heat.
fn worker_of(prog: yavs_ir::Program) -> super::Worker {
    let rt = yavs_interp::Runtime::new(std::sync::Arc::new(prog), Default::default()).unwrap();
    let tab = || -> std::sync::Arc<[std::sync::atomic::AtomicUsize]> {
        (0..rt.prog.funcs.len())
            .map(|_| std::sync::atomic::AtomicUsize::new(0))
            .collect()
    };
    super::Worker::new(
        rt.prog.clone(),
        (0..rt.prog.funcs.len())
            .map(|_| super::Entry::default())
            .collect(),
        tab(),
        tab(),
        rt.native_flags(),
        super::Heat::DEFAULT,
        usize::MAX,
        Box::new(Stub),
        Box::new(|_, _| {}),
    )
}

/// A process with a loop through its suspension and two reserved copies (not a clocked
/// process: the copies are judged by their own seen time).
fn process_with_copies() -> (yavs_ir::Program, yavs_ir::FuncId, [yavs_ir::FuncId; 2]) {
    use yavs_ir::{AddrSpace, CmpOp, FuncKind, RegTy, Width};
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
    let mut p = pb.finish().unwrap();
    p.reserve_copies(2);
    let copy = |g: i64| {
        let mut d = g.to_le_bytes().to_vec();
        d.extend([0; 8]);
        p.copy_of(f, &d, &[]).expect("a copy")
    };
    let cs = [copy(3), copy(4)];
    (p, f, cs)
}

/// Two procedures of the shape of `padded_loop`, neither calling the other.
fn two_loops(n: usize) -> (yavs_ir::Program, yavs_ir::FuncId, yavs_ir::FuncId) {
    let mut pb = ProgramBuilder::new();
    let ids = ["f", "g"].map(|name| pb.declare(name, yavs_ir::FuncKind::Procedure, None));
    for id in ids {
        let mut fb = pb.define(id);
        let spin = fb.block();
        fb.stmt(StmtKind::Return, Span::new(FileId(0), 0, 1));
        let ptr = fb.reg(yavs_ir::RegTy::Ptr);
        fb.emit(Inst::AddrOf {
            dst: ptr,
            space: yavs_ir::AddrSpace::Frame,
            off: 0,
        });
        for _ in 0..n {
            fb.emit(Inst::Free { ptr });
        }
        fb.emit(Inst::Br { to: spin });
        fb.at(spin);
        fb.emit(Inst::Br { to: spin });
        fb.finish();
    }
    (pb.finish().unwrap(), ids[0], ids[1])
}

/// What becomes hot, and when, is a function of the looks of the sampler and the program, and
/// of nothing else (#670): not of how fast the looks are worked through, how they are batched,
/// or what was compiled meanwhile. The compile cost is a fixed price of the function.
#[test]
fn what_is_hot_depends_on_the_looks_only() {
    let look = |ms: u64| super::Look {
        running: vec![(yavs_ir::FuncId(0), false, ms * 1000)],
        cpu: ms * 1000,
    };
    let hot_after = |pause: bool| {
        let (mut w, f) = worker(100);
        let price = w.cost(f);
        assert_eq!(w.kind(f), super::Kind::Compute);
        let n = w.prog.func(f).unwrap().code.len();
        assert_eq!(price, (super::FIRST_US_PER_INSTR * n as f64) as u64);
        let mut pending = super::Pending::default();
        for n in 1.. {
            if pause && n % 7 == 0 {
                std::thread::sleep(std::time::Duration::from_millis(3));
            }
            w.see(&look(1), &mut pending);
            if !pending.cands.is_empty() {
                assert_eq!(pending.cands, [f]);
                assert_eq!(w.cost(f), price, "{n}");
                return (n, w.total, w.need(f));
            }
            assert!(n < 100_000);
        }
        unreachable!()
    };
    let (a, b) = (hot_after(false), hot_after(true));
    assert_eq!(a, b);
    // A compute loop is compiled once seen for what its price and the payback of its kind say.
    assert!((a.2..a.2 + 1000).contains(&a.1), "{a:?}");
    assert_eq!(a.0 * 1000, a.1);
}

/// The kinds of function are judged apart and priced alike (#673): a first-level generator costs
/// the same per IR instruction for every kind, so a kind's payback alone says how long a function
/// is seen before it is compiled: a better speedup means an earlier compile, a function of the
/// size of a large process waits longest, and one that is mostly loops less long.
#[test]
fn kinds_are_judged_apart_and_priced_alike() {
    use super::Kind;
    let (w, f) = worker(1_600);
    assert_eq!(w.kind(f), Kind::Big);
    let (w2, f2) = worker(100);
    assert_eq!(w2.kind(f2), Kind::Compute);
    let (w3, f3) = worker_on(padding_in_loop(1_600));
    assert_eq!(w3.kind(f3), Kind::BigLoop);
    let per_ir = |w: &super::Worker, f| {
        let n = w.prog.func(f).unwrap().code.len() as f64;
        assert_eq!(w.cost(f), (super::FIRST_US_PER_INSTR * n) as u64);
        w.need(f) as f64 / w.cost(f) as f64
    };
    let (big, compute, big_loop) = (per_ir(&w, f), per_ir(&w2, f2), per_ir(&w3, f3));
    let m = |k: Kind| models()[k as usize];
    // The better the speedup, the fewer times its price a function must be seen.
    assert!(
        (compute - m(Kind::Compute).payback()).abs() < 0.01,
        "{compute}"
    );
    assert!((big - m(Kind::Big).payback()).abs() < 0.01, "{big}");
    assert!(compute < m(Kind::Frame).payback() && m(Kind::Frame).payback() < big);
    // A large function whose code is mostly loops is compiled earlier than a straight-line one
    // of the same size (#679); the share comes from the loops, not from the size.
    assert_eq!(super::loop_share(&w.prog, w.prog.func(f).unwrap()), 0);
    assert!(super::loop_share(&w3.prog, w3.prog.func(f3).unwrap()) > 90);
    assert!(big_loop < big);
    // A function whose code is in the cache is priced at what linking it costs, whatever its kind
    // (#728, #785).
    let n2 = w2.prog.func(f2).unwrap().code.len();
    assert_eq!(w2.price_of(f2, true), super::FIRST_HIT.price(n2));
    assert!(w2.price_of(f2, true) * 10 < w2.price_of(f2, false));
}

/// A warm run compiles a function whose object is in the cache once its cache price repays,
/// without the floor every other function waits for; `--tier-heat` keeps the floor (#728).
#[test]
fn a_cached_function_waits_for_its_cache_price_only() {
    let (mut w, f) = worker(200);
    let floor = w.heat.threshold(w.prog.func(f).unwrap());
    assert_eq!(w.floor(f), floor, "not in the cache");
    w.in_cache[f.0 as usize] = std::cell::OnceCell::from(true);
    assert_eq!(w.floor(f), 0);
    assert!(w.threshold(f) < floor, "{}", w.threshold(f));
    assert_eq!(w.threshold(f), w.need(f));
    w.heat.budget = 0;
    assert_eq!(w.floor(f), floor, "--tier-heat");
}

/// What the profile of an earlier run calls hot is a candidate at its first look (#754), whatever
/// the payback rule says yet, once; the longest seen last time is compiled first; a function a
/// trial took back or that is compiled already is not.
#[test]
fn the_profile_makes_a_function_a_candidate_at_its_first_look() {
    let (mut w, f) = worker(100);
    let look = |us: u64| super::Look {
        running: vec![(f, false, us)],
        cpu: us,
    };
    let mut pending = super::Pending::default();
    w.see(&look(10), &mut pending);
    assert!(
        pending.early.is_empty() && pending.cands.is_empty(),
        "no profile"
    );
    w.asked_for[f.0 as usize] = false;
    w.early.insert(f, 500_000);
    w.see(&look(10), &mut pending);
    assert_eq!(pending.early, [(500_000, f)]);
    assert!(pending.cands.is_empty());
    w.see(&look(10), &mut pending);
    assert_eq!(pending.early.len(), 1, "asked once");
    let ids = w.next_early(&mut pending).expect("a tree");
    assert_eq!(ids[0].0, f);
    assert!(w.next_early(&mut pending).is_none());
    // Taken back by a trial: not asked for.
    let (mut w, f) = worker(100);
    w.early.insert(f, 500_000);
    w.lost[f.0 as usize] = true;
    let mut none = super::Pending::default();
    w.see(&look(10), &mut none);
    assert!(none.early.is_empty() && none.cands.is_empty());
    // Compiled in the meantime: skipped when its turn comes.
    let (mut w, f) = worker(100);
    pending.early = vec![(1, f)];
    w.done[f.0 as usize][0] = true;
    assert!(w.next_early(&mut pending).is_none());
}

/// The snapshot of a run names the processes seen for a good part of the time whose kind is
/// expected to gain from compiled code, and what the profile needs besides (#754).
#[test]
fn the_snapshot_names_the_processes_that_are_hot_and_would_gain() {
    use super::Kind;
    let (mut w, f) = worker(100);
    let look = |us: u64| super::Look {
        running: vec![(f, false, us)],
        cpu: us,
    };
    let mut pending = super::Pending::default();
    // 19 ms: not yet; 21 ms: hot.
    w.see(&look(19_000), &mut pending);
    assert!(w.snapshot().seen.is_empty());
    w.see(&look(2_000), &mut pending);
    // (A function, not a process: no early compile of it.)
    assert!(w.snapshot().seen.is_empty());
    assert_eq!(w.hotish, [f.0]);
    // The same numbers for a process of a kind that gains and one that does not.
    let (prog, p) = process(150, 0);
    let rt = yavs_interp::Runtime::new(std::sync::Arc::new(prog), Default::default()).unwrap();
    let tab = || -> std::sync::Arc<[std::sync::atomic::AtomicUsize]> {
        (0..rt.prog.funcs.len())
            .map(|_| std::sync::atomic::AtomicUsize::new(0))
            .collect()
    };
    let table: std::sync::Arc<[super::Entry]> = (0..rt.prog.funcs.len())
        .map(|_| super::Entry::default())
        .collect();
    let mut w = super::Worker::new(
        rt.prog.clone(),
        table,
        tab(),
        tab(),
        rt.native_flags(),
        super::Heat::DEFAULT,
        usize::MAX,
        Box::new(Stub),
        Box::new(|_, _| {}),
    );
    let look = |us: u64| super::Look {
        running: vec![(p, false, us)],
        cpu: us,
    };
    w.see(&look(100_000), &mut pending);
    for (kind, named) in [
        (Kind::ComputeFrame, true),
        (Kind::Rtl, true),
        (Kind::RtlFew, false),
        (Kind::Big, false),
    ] {
        w.kinds[p.0 as usize] = Default::default();
        w.kinds[p.0 as usize].set(kind).unwrap();
        let snap = w.snapshot();
        // (A clocked process seen for under an eighth of the run is not named either.)
        let share = if kind == Kind::Rtl && snap.seen.is_empty() {
            "cold"
        } else {
            ""
        };
        assert_eq!(snap.seen == [(p.0, 100_000)], named, "{kind:?} {share}");
        assert_eq!(snap.total, 100_000);
    }
    w.lost_bases.push(p.0);
    w.early_done.push(p);
    let snap = w.snapshot();
    assert_eq!((snap.lost, snap.early), (vec![p.0], vec![p.0]));
}

/// The CPU time of a look is shared by the functions running in it, and the time seen running
/// compiled code is kept apart.
#[test]
fn a_look_shares_its_time_between_the_running_functions() {
    let (mut w, f) = worker(100);
    let mut pending = super::Pending::default();
    let look = super::shared_look(1000, &[(0, f, false), (1, f, true)]);
    w.see(&look, &mut pending);
    assert_eq!(w.time[f.0 as usize], [500, 500]);
    assert_eq!(w.total, 1000);
}

/// The class rules of the threshold analyse what a function can call; one below the base
/// threshold gets none of that (#544).
#[test]
fn a_function_below_the_base_threshold_is_not_analysed() {
    let (mut w, f) = worker(100);
    let at = super::Heat::DEFAULT.threshold(w.prog.func(f).unwrap());
    let look = |us: u64| super::Look {
        running: vec![(f, false, us)],
        cpu: us,
    };
    let mut pending = super::Pending::default();
    w.see(&look(at - 1), &mut pending);
    assert!(w.compute[f.0 as usize].get().is_none());
    assert!(pending.cands.is_empty());
    w.see(&look(1), &mut pending);
    assert!(w.compute[f.0 as usize].get().is_some());
}

/// The copies of a process that is no clocked process are modules of their own: a copy is a
/// candidate once the process is hot and it has repaid its own price, not when the process's
/// other copies made the process hot (#757). The cache list names each copy apart.
#[test]
fn a_copy_repays_its_own_compile() {
    let (prog, f, [c1, c2]) = process_with_copies();
    let mut w = worker_of(prog);
    assert_eq!(w.prog.base_of(c1), f);
    assert!(!w.is_rtl(c1));
    let look = |id: yavs_ir::FuncId, us: u64| super::Look {
        running: vec![(id, false, us)],
        cpu: us,
    };
    let mut pending = super::Pending::default();
    let need = w.need(c2);
    w.see(&look(c1, 10 * need), &mut pending);
    assert_eq!(pending.cands, [c1], "repaid by its own time");
    w.see(&look(c2, need - 1), &mut pending);
    assert_eq!(
        pending.cands,
        [c1],
        "the process is hot, but this copy has not repaid yet"
    );
    w.see(&look(c2, 1), &mut pending);
    assert_eq!(pending.cands, [c1, c2]);
    let k = |id| {
        let f = w.prog.func(id).unwrap();
        w.list_key(id, f)
    };
    assert_ne!(k(c1), k(c2));
    assert_eq!(k(f).0, "p");
}

/// A large process whose copy has loops (`Kind::BigLoop`) is named by the snapshot although
/// its shared code is `Big` (maintainer, 2026-10-27, #679); a next run compiles the copy early
/// only when the time seen reaches the copy's own threshold.
#[test]
fn the_profile_takes_a_process_with_a_big_loop_copy() {
    use super::Kind;
    let (prog, f, [c1, c2]) = process_with_copies();
    let mut w = worker_of(prog);
    let look = |id: yavs_ir::FuncId, us: u64| super::Look {
        running: vec![(id, false, us)],
        cpu: us,
    };
    let mut pending = super::Pending::default();
    w.see(&look(c1, 100_000), &mut pending);
    let set = |w: &mut super::Worker, id: yavs_ir::FuncId, k: Kind| {
        w.kinds[id.0 as usize] = Default::default();
        w.kinds[id.0 as usize].set(k).unwrap();
    };
    set(&mut w, f, Kind::Big);
    set(&mut w, c1, Kind::Big);
    assert!(w.snapshot().seen.is_empty(), "Big alone gains nothing");
    set(&mut w, c1, Kind::BigLoop);
    assert_eq!(w.snapshot().seen, [(f.0, 100_000)]);
    // The next run: the copy is compiled early only once the time seen pays for it.
    set(&mut w, c2, Kind::BigLoop);
    let need = w.threshold(c2);
    let mut pending = super::Pending::default();
    w.early.insert(f, need - 1);
    w.see(&look(c2, 1), &mut pending);
    assert!(pending.early.is_empty());
    w.early.insert(f, need);
    w.see(&look(c2, 1), &mut pending);
    assert_eq!(pending.early.len(), 1);
    assert_eq!(pending.early[0].1, c2);
}

/// A compile is paid by what its whole tree saves: the price of a callee it brings counts against
/// the seen time of the root and of the callee (#757).
#[test]
fn a_compile_waits_until_its_tree_repays() {
    use super::Part;
    let (prog, f, g) = two_loops(100);
    let mut w = worker_of(prog);
    let tree = [(f, Part::Native), (g, Part::Native)];
    // (`see` keeps a function's time and its group's alike.)
    let seen = |w: &mut super::Worker, id: yavs_ir::FuncId, us: u64| {
        w.time[id.0 as usize][0] = us;
        w.group[id.0 as usize] = us;
    };
    let (nf, ng) = (w.need(f), w.need(g));
    seen(&mut w, f, nf);
    let (saves, costs) = w.repays(f, &tree[..1]);
    assert!(saves >= costs, "alone at its threshold: {saves} {costs}");
    let (saves, costs) = w.repays(f, &tree);
    assert!(saves < costs, "with a callee: {saves} {costs}");
    // A program that ran before shares the cost among the runs the cache serves.
    w.earlier.base = Some(1);
    let (_, shared) = w.repays(f, &tree);
    assert_eq!(shared, costs / super::profile::EARLY_RUNS as f64);
    w.earlier.base = None;
    seen(&mut w, g, ng);
    let (saves, costs) = w.repays(f, &tree);
    assert!(saves >= costs, "the callee repaid its own: {saves} {costs}");
}

/// A candidate whose compile does not repay yet waits without holding up the next pick: it is
/// asked again at the next look (`pick_at` stays).
#[test]
fn a_candidate_that_does_not_repay_yet_is_asked_again() {
    let (mut w, f) = worker(100);
    let k = f.0 as usize;
    w.total = 100 * w.need(f);
    (w.time[k][0], w.group[k]) = (w.need(f) / 2, w.need(f) / 2);
    let mut cands = vec![f];
    assert!(w.pick(&mut cands).is_none());
    assert_eq!((cands.as_slice(), w.pick_at), ([f].as_slice(), 0));
    (w.time[k][0], w.group[k]) = (w.need(f), w.need(f));
    assert!(w.pick(&mut cands).is_some());
    assert!(cands.is_empty());
}

/// Native-frame code is published once what it reaches through compiled callees is compiled;
/// a callee never seen running does not hold it back, one seen running does, one left out
/// of a compile does not (#797).
#[test]
fn native_code_waits_only_for_callees_that_ran() {
    let mut pb = ProgramBuilder::new();
    let ids: Vec<_> = ["f", "g", "h"]
        .into_iter()
        .map(|n| pb.declare(n, yavs_ir::FuncKind::Procedure, None))
        .collect();
    for k in 0..3 {
        let mut fb = pb.define(ids[k]);
        fb.stmt(StmtKind::Return, Span::new(FileId(0), 0, 1));
        // Something of its own, so the optimiser keeps the call.
        let keep = fb.slot(8, 8);
        let at = fb.addr(yavs_ir::AddrSpace::Frame, keep);
        fb.emit(Inst::Store {
            addr: at,
            src: at,
            w: yavs_ir::Width::B8,
        });
        if k < 2 {
            let args = fb.regs(&[]);
            fb.emit(Inst::Call {
                func: ids[k + 1],
                args,
                dst: None,
            });
        }
        fb.emit(Inst::Ret { v: None });
        fb.finish();
    }
    let mut w = worker_of(pb.finish().unwrap());
    let (f, g, h) = (ids[0], ids[1], ids[2]);
    assert!(w.nset(f).contains(&h));
    w.nat[f.0 as usize].0 = 1;
    assert!(w.missing(f).is_none(), "g never ran");
    w.time[g.0 as usize][0] = 1;
    assert!(w.missing(f).is_some(), "g ran and is not compiled");
    w.nat[g.0 as usize].0 = 1;
    assert!(w.missing(f).is_none(), "h never ran");
    w.time[h.0 as usize][0] = 1;
    assert!(w.missing(f).is_some(), "h ran and is not compiled");
    w.left[h.0 as usize].set(true);
    assert!(w.missing(f).is_none(), "h is left out");
}
