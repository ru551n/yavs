//! The LLVM tier against the interpreter (rule 5): hand-written IR run both ways.
#![cfg(feature = "clift")]

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use yavs_clift::{ClifTier, First};
use yavs_diag::{FileId, Span};
use yavs_interp::{Bindings, InstData, Runtime, Tier, TierHost};
use yavs_ir::*;
use yavs_llvm::{AutoTier, Gens, Heat, Note, OptLevel};

/// Counts the entries into compiled code.
struct Counting(ClifTier, AtomicUsize);

impl Tier for Counting {
    fn run(&self, func: FuncId, pc: u32, host: &mut dyn TierHost) -> Option<u32> {
        let r = self.0.run(func, pc, host);
        if r.is_some() {
            self.1.fetch_add(1, Ordering::Relaxed);
        }
        r
    }
}

/// A program compiled once, called in the interpreter and with the LLVM tier.
struct Both {
    prog: Arc<Program>,
    tier: Arc<Counting>,
    inst: Arc<InstData>,
}

impl Both {
    fn new(prog: Program) -> Both {
        let tier = yavs_clift::compile(&prog, usize::MAX).unwrap();
        assert!(tier.notes.is_empty(), "{:?}", tier.notes);
        Both {
            prog: Arc::new(prog),
            tier: Arc::new(Counting(tier, AtomicUsize::new(0))),
            inst: Arc::new(InstData {
                data: (1..=16).collect(),
                ..InstData::default()
            }),
        }
    }

    fn call(&self, f: FuncId, args: &[u64], llvm: bool) -> Result<u64, String> {
        let rt = Runtime::new(self.prog.clone(), Bindings::default()).unwrap();
        if llvm {
            assert!(rt.set_tier(self.tier.clone()));
        }
        yavs_interp::call(rt, f, self.inst.clone(), args)
            .map(|v| v.expect("a value"))
            .map_err(|e| e.to_string())
    }

    /// Both results (value or runtime error text) and the entries into compiled code.
    fn both(&self, f: FuncId, args: &[u64]) -> (Result<u64, String>, Result<u64, String>, usize) {
        self.tier.1.store(0, Ordering::Relaxed);
        let (a, b) = (self.call(f, args, false), self.call(f, args, true));
        (a, b, self.tier.1.load(Ordering::Relaxed))
    }
}

fn stmt(fb: &mut FuncBuilder<'_>) {
    fb.stmt(StmtKind::Return, Span::new(FileId(0), 0, 1));
}

/// `function f(a, b) return a op b` with an optional range check of the result.
fn binop(op: IntOp, range: Option<(i64, i64)>) -> (Program, FuncId) {
    let mut pb = ProgramBuilder::new();
    let f = pb.declare("f", FuncKind::Function, None);
    let msg = pb.check_msg("result", Dir::To);
    let mut fb = pb.define(f);
    let sa = fb.param("a", Width::B8, ParamClass::Value, Mode::In, None);
    let sb = fb.param("b", Width::B8, ParamClass::Value, Mode::In, None);
    stmt(&mut fb);
    let (pa, pbr) = (fb.addr(AddrSpace::Frame, sa), fb.addr(AddrSpace::Frame, sb));
    let (a, b) = (fb.reg(RegTy::I64), fb.reg(RegTy::I64));
    fb.emit(Inst::Load {
        dst: a,
        addr: pa,
        w: Width::B8,
    });
    fb.emit(Inst::Load {
        dst: b,
        addr: pbr,
        w: Width::B8,
    });
    let dst = fb.reg(RegTy::I64);
    match range {
        None => fb.emit(Inst::Bin { op, dst, a, b }),
        Some((l, h)) => {
            let (lo, hi) = (fb.int(l), fb.int(h));
            fb.emit(Inst::BinRange {
                op,
                dst,
                a,
                b,
                lo,
                hi,
                what: msg,
            });
        }
    }
    fb.emit(Inst::Ret { v: Some(dst) });
    fb.finish();
    (pb.finish().unwrap(), f)
}

#[test]
fn integer_ops_match_the_interpreter() {
    let vals = [
        0,
        1,
        -1,
        2,
        -2,
        7,
        -7,
        1000,
        i64::MAX,
        i64::MIN,
        i64::MAX - 1,
        i64::MIN + 1,
    ];
    let ops = [
        IntOp::Add,
        IntOp::Sub,
        IntOp::Mul,
        IntOp::Div,
        IntOp::Rem,
        IntOp::Mod,
        IntOp::Pow,
    ];
    for op in ops {
        for range in [None, Some((-100, 100))] {
            let (prog, f) = binop(op, range);
            let p = Both::new(prog);
            for &x in &vals {
                for &y in &vals {
                    let (a, b, n) = p.both(f, &[x as u64, y as u64]);
                    assert_eq!(a, b, "{op:?} {range:?} {x} {y}");
                    assert!(n > 0, "compiled code never ran");
                }
            }
        }
    }
}

#[test]
fn loop_runs_compiled() {
    let (prog, f) = sum();
    let p = Both::new(prog);
    let (a, b, n) = p.both(f, &[100_000]);
    assert_eq!(a, Ok(5_000_050_000));
    assert_eq!(a, b);
    // The whole loop runs in compiled code and the interpreter only executes `Ret`: one entry
    // from the function's start, or two when its first instruction is a definition the frame
    // starts with (skipped, so the interpreter runs the preamble and enters at the loop).
    assert!((1..=2).contains(&n), "{n} entries");
}

/// A threshold the loops of these tests pass early: seen in one sample of the interpreter.
const HOT: Heat = Heat {
    base: 1,
    per_instr: 0,
    period: 100,
    budget: 0,
};

/// The tier-up policy with Cranelift as its only generator.
fn start(rt: &Runtime, max_size: usize, note: Note) -> Arc<AutoTier> {
    let gens = Gens {
        main: None,
        first: Some(Box::new(First::new().unwrap())),
    };
    AutoTier::start_with(rt, OptLevel::O2, max_size, HOT, note, gens).unwrap()
}

/// `--tier=auto`: the loop becomes hot while it runs in the interpreter, is compiled on the
/// background thread and swapped in; the results never change (rule 5).
#[test]
fn auto_tier_compiles_hot_functions() {
    let (prog, f) = sum();
    let rt = Runtime::new(Arc::new(prog), Bindings::default()).unwrap();
    let notes = Arc::new(Mutex::new(Vec::new()));
    let seen = notes.clone();
    let note: Note = Box::new(move |f, _, why| seen.lock().unwrap().push((f, why)));
    let tier = start(&rt, usize::MAX, note);
    let call = || yavs_interp::call(rt.clone(), f, Arc::new(InstData::default()), &[100_000]);
    // The loop runs until the sampler has seen it and its code is compiled.
    let t0 = Instant::now();
    while tier.compiled() == 0 {
        assert!(t0.elapsed() < Duration::from_secs(60), "never compiled");
        assert_eq!(call().unwrap(), Some(5_000_050_000));
    }
    assert_eq!(call().unwrap(), Some(5_000_050_000));
    tier.stop(true);
    // One note per part compiled (native-frame code, frame-mode code).
    let notes = notes.lock().unwrap();
    assert!(!notes.is_empty());
    assert!(notes.iter().all(|n| n.0 == f && n.1.is_ok()), "{notes:?}");
}

/// A hot function over `--tier-max-size` stays interpreted, with a note.
#[test]
fn auto_tier_skips_functions_over_the_size_cap() {
    let (prog, f) = sum();
    let rt = Runtime::new(Arc::new(prog), Bindings::default()).unwrap();
    let notes = Arc::new(Mutex::new(Vec::new()));
    let seen = notes.clone();
    let note: Note = Box::new(move |f, _, why| seen.lock().unwrap().push((f, why)));
    let tier = start(&rt, 3, note);
    let t0 = Instant::now();
    while notes.lock().unwrap().is_empty() {
        assert!(t0.elapsed() < Duration::from_secs(60), "no note");
        let r = yavs_interp::call(rt.clone(), f, Arc::new(InstData::default()), &[100_000]);
        assert_eq!(r.unwrap(), Some(5_000_050_000));
    }
    tier.stop(true);
    assert_eq!(tier.compiled(), 0);
    let notes = notes.lock().unwrap();
    assert_eq!(notes.len(), 1);
    let why = notes[0].1.as_ref().unwrap_err();
    assert!(
        why.contains("over the size cap of 3 (--tier-max-size)"),
        "{why}"
    );
}

/// sum := 0; for i in 0 to n loop sum := sum + i; end loop; return sum;
fn sum() -> (Program, FuncId) {
    let mut pb = ProgramBuilder::new();
    let f = pb.declare("sum", FuncKind::Function, None);
    let mut fb = pb.define(f);
    let sn = fb.param("n", Width::B8, ParamClass::Value, Mode::In, None);
    let (head, body, done) = (fb.block(), fb.block(), fb.block());
    stmt(&mut fb);
    let pn = fb.addr(AddrSpace::Frame, sn);
    let n = fb.reg(RegTy::I64);
    fb.emit(Inst::Load {
        dst: n,
        addr: pn,
        w: Width::B8,
    });
    let (i, acc, one) = (fb.int(0), fb.int(0), fb.int(1));
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

/// `function f(a, b) return <body>` where `body` builds the result from the two argument
/// registers, with a 32-byte frame area `local` and a 16-byte constant `c0` (1 to 16).
fn two(body: impl FnOnce(&mut FuncBuilder<'_>, Reg, Reg, Reg) -> Reg) -> (Program, FuncId) {
    let mut pb = ProgramBuilder::new();
    pb.konst(ConstData::Bytes((1..=16).collect()));
    let f = pb.declare("f", FuncKind::Function, None);
    let mut fb = pb.define(f);
    let sa = fb.param("a", Width::B8, ParamClass::Value, Mode::In, None);
    let sb = fb.param("b", Width::B8, ParamClass::Value, Mode::In, None);
    let local = fb.slot(32, 8);
    stmt(&mut fb);
    let (pa, pbr) = (fb.addr(AddrSpace::Frame, sa), fb.addr(AddrSpace::Frame, sb));
    let (a, b) = (fb.reg(RegTy::I64), fb.reg(RegTy::I64));
    for (dst, addr) in [(a, pa), (b, pbr)] {
        fb.emit(Inst::Load {
            dst,
            addr,
            w: Width::B8,
        });
    }
    let l = fb.addr(AddrSpace::Frame, local);
    let v = body(&mut fb, a, b, l);
    fb.emit(Inst::Ret { v: Some(v) });
    fb.finish();
    (pb.finish().unwrap(), f)
}

fn index(fb: &mut FuncBuilder<'_>, base: Reg, pos: Reg) -> Reg {
    let dst = fb.reg(RegTy::Ptr);
    fb.emit(Inst::Index {
        dst,
        base,
        pos,
        stride: 1,
    });
    dst
}

fn alloc(fb: &mut FuncBuilder<'_>, size: Reg) -> Reg {
    let dst = fb.reg(RegTy::Ptr);
    fb.emit(Inst::Alloc { dst, size });
    dst
}

fn load(fb: &mut FuncBuilder<'_>, addr: Reg, w: Width) -> Reg {
    let dst = fb.reg(if w == Width::Sig {
        RegTy::Sig
    } else {
        RegTy::I64
    });
    fb.emit(Inst::Load { dst, addr, w });
    dst
}

/// A descriptor `left, right, dir` in temporary memory.
fn dope(fb: &mut FuncBuilder<'_>, l: Reg, r: Reg, dir: Reg) -> Reg {
    let n = fb.int(24);
    let at = alloc(fb, n);
    let parts = fb.regs(&[l, r, dir]);
    fb.emit(Inst::MakeDope { at, parts });
    at
}

const EDGE: [i64; 14] = [
    0,
    1,
    2,
    7,
    8,
    15,
    16,
    24,
    31,
    32,
    -1,
    i64::MAX,
    i64::MIN,
    1 << 32,
];

/// Every pair of edge values gives the same result or error text both ways, compiled code
/// runs, and none of `native` is left to the interpreter.
fn check(prog: Program, f: FuncId, native: &[&str], args: &[i64]) {
    let p = Both::new(prog);
    for k in native {
        assert!(
            !p.tier.0.interpreted.iter().any(|(n, _)| n == k),
            "{k} interpreted: {:?}",
            p.tier.0.interpreted
        );
    }
    for &x in args {
        for &y in args {
            let (a, b, n) = p.both(f, &[x as u64, y as u64]);
            assert_eq!(a, b, "{native:?} {x} {y}");
            assert!(n > 0, "compiled code never ran");
        }
    }
}

#[test]
fn loads_and_stores_match_the_interpreter() {
    for w in [Width::B1, Width::B4, Width::B8] {
        // local[b] := a (width w); return local[b]
        let (prog, f) = two(|fb, a, b, l| {
            let at = index(fb, l, b);
            fb.emit(Inst::Store {
                addr: at,
                src: a,
                w,
            });
            load(fb, at, w)
        });
        check(prog, f, &["Load", "Store", "AddrOf"], &EDGE);
    }
    // A signal reference pair (a, b) loaded from local[0] and stored at local[b]; returns
    // the second half read back.
    let (prog, f) = two(|fb, a, b, l| {
        let parts = fb.regs(&[a, b]);
        fb.emit(Inst::MakeDope { at: l, parts });
        let s = load(fb, l, Width::Sig);
        let at = index(fb, l, b);
        fb.emit(Inst::Store {
            addr: at,
            src: s,
            w: Width::Sig,
        });
        let eight = fb.int(8);
        let hi = index(fb, at, eight);
        load(fb, hi, Width::B8)
    });
    check(prog, f, &["Load", "Store"], &EDGE);
    // Reads of the instance data and a constant (through the object cache) at a + b.
    for space in [AddrSpace::Inst, AddrSpace::Const(ConstId(0))] {
        for w in [Width::B1, Width::B4, Width::B8] {
            let (prog, f) = two(|fb, a, b, _| {
                let base = fb.addr(space, 0);
                let at = index(fb, base, a);
                let at = index(fb, at, b);
                load(fb, at, w)
            });
            check(prog, f, &["Load", "AddrOf"], &EDGE);
        }
    }
    // A write to read-only memory is the interpreter's error.
    let (prog, f) = two(|fb, a, b, _| {
        let base = fb.addr(AddrSpace::Const(ConstId(0)), 0);
        let at = index(fb, base, b);
        fb.emit(Inst::Store {
            addr: at,
            src: a,
            w: Width::B8,
        });
        a
    });
    check(prog, f, &["Store"], &[0, 1, 9]);
}

#[test]
fn temporaries_and_copies_match_the_interpreter() {
    // Two allocations: the second one's address, and its contents after a store of b.
    for addr in [false, true] {
        let (prog, f) = two(|fb, a, b, _| {
            let _ = alloc(fb, a);
            let eight = fb.int(8);
            let t = alloc(fb, eight);
            fb.emit(Inst::Store {
                addr: t,
                src: b,
                w: Width::B4,
            });
            if addr { t } else { load(fb, t, Width::B8) }
        });
        check(prog, f, &["Alloc", "Stmt"], &EDGE);
    }
    // A loop of statements each allocating a bytes: the area is reset at every `Stmt`.
    let mut pb = ProgramBuilder::new();
    let f = pb.declare("f", FuncKind::Function, None);
    let mut fb = pb.define(f);
    let sa = fb.param("a", Width::B8, ParamClass::Value, Mode::In, None);
    let _ = fb.param("b", Width::B8, ParamClass::Value, Mode::In, None);
    let (head, done) = (fb.block(), fb.block());
    stmt(&mut fb);
    let pa = fb.addr(AddrSpace::Frame, sa);
    let a = load(&mut fb, pa, Width::B8);
    let (i, one, n) = (fb.int(0), fb.int(1), fb.int(1000));
    fb.emit(Inst::Br { to: head });
    fb.at(head);
    stmt(&mut fb);
    let t = alloc(&mut fb, a);
    fb.emit(Inst::Bin {
        op: IntOp::Add,
        dst: i,
        a: i,
        b: one,
    });
    let c = fb.reg(RegTy::I64);
    fb.emit(Inst::Cmp {
        op: CmpOp::Lt,
        dst: c,
        a: i,
        b: n,
    });
    fb.emit(Inst::CondBr {
        c,
        t: head,
        f: done,
    });
    fb.at(done);
    fb.emit(Inst::Ret { v: Some(t) });
    fb.finish();
    check(
        pb.finish().unwrap(),
        f,
        &["Alloc", "Stmt"],
        &[0, 1, 13, 1 << 20, 1 << 32, -1],
    );
    // local[0..32] := 1, 2, ...; copy b bytes from local[a] to local[0]; return local[0].
    let (prog, f) = two(|fb, a, b, l| {
        let ws: Vec<Reg> = (0..4).map(|k| fb.int(0x0807_0605_0403_0201 + k)).collect();
        let parts = fb.regs(&ws);
        fb.emit(Inst::MakeDope { at: l, parts });
        let src = index(fb, l, a);
        fb.emit(Inst::Copy {
            dst: l,
            src,
            bytes: b,
        });
        load(fb, l, Width::B8)
    });
    check(prog, f, &["Copy", "MakeDope"], &EDGE);
    // A constant copied into temporary memory.
    let (prog, f) = two(|fb, a, b, _| {
        let t = alloc(fb, a);
        let c = fb.addr(AddrSpace::Const(ConstId(0)), 0);
        fb.emit(Inst::Copy {
            dst: t,
            src: c,
            bytes: b,
        });
        load(fb, t, Width::B4)
    });
    check(prog, f, &["Copy"], &EDGE);
}

#[test]
fn descriptors_match_the_interpreter() {
    for dir in [0, 1, 2] {
        // 'length of (a, b, dir), and the position of 5 in it.
        let (prog, f) = two(|fb, a, b, _| {
            let d = fb.int(dir);
            let at = dope(fb, a, b, d);
            let n = fb.reg(RegTy::I64);
            fb.emit(Inst::DopeLen {
                dst: n,
                dope: at,
                dim: 0,
            });
            n
        });
        check(prog, f, &["DopeLen", "MakeDope"], &EDGE);
        for idx in [0, 5, -3] {
            let (prog, f) = two(|fb, a, b, _| {
                let d = fb.int(dir);
                let at = dope(fb, a, b, d);
                let i = fb.int(idx);
                let what = fb.program().check_msg("v", Dir::To);
                let dst = fb.reg(RegTy::I64);
                fb.emit(Inst::Pos {
                    dst,
                    dope: at,
                    dim: 0,
                    idx: i,
                    what,
                });
                dst
            });
            check(prog, f, &["Pos"], &EDGE);
        }
    }
    // (a to b) against (b downto 3) and (0 to 4), two dimensions.
    let (prog, f) = two(|fb, a, b, _| {
        let (z, one, three, four) = (fb.int(0), fb.int(1), fb.int(3), fb.int(4));
        let n = fb.int(48);
        let x = alloc(fb, n);
        let parts = fb.regs(&[a, b, z, b, three, one]);
        fb.emit(Inst::MakeDope { at: x, parts });
        let y = alloc(fb, n);
        let parts = fb.regs(&[z, four, z, a, b, z]);
        fb.emit(Inst::MakeDope { at: y, parts });
        let what = fb.program().check_msg("v", Dir::To);
        fb.emit(Inst::SubtypeCheck {
            dope: x,
            want: y,
            dims: 2,
            what,
        });
        a
    });
    check(prog, f, &["SubtypeCheck"], &EDGE);
    // Field 1 of a layout [size, align, 0, a] at local[b].
    let (prog, f) = two(|fb, a, b, l| {
        let (z, eight) = (fb.int(0), fb.int(8));
        let n = fb.int(32);
        let lay = alloc(fb, n);
        let parts = fb.regs(&[eight, eight, z, a]);
        fb.emit(Inst::MakeDope { at: lay, parts });
        let base = index(fb, l, b);
        let dst = fb.reg(RegTy::Ptr);
        fb.emit(Inst::Field {
            dst,
            base,
            layout: lay,
            n: 1,
        });
        dst
    });
    check(prog, f, &["Field"], &EDGE);
}

#[test]
fn logic_elements_match_the_interpreter() {
    // local is two bit-plane blocks of 4 x 64 bits: element b := code a, then read back
    // element b and element b xor 1 (untouched neighbours stay).
    for other in [false, true] {
        let (prog, f) = two(|fb, a, b, l| {
            fb.emit(Inst::LSet {
                base: l,
                idx: b,
                v: a,
            });
            let i = if other {
                let one = fb.int(1);
                let i = fb.reg(RegTy::I64);
                fb.emit(Inst::Bin {
                    op: IntOp::Add,
                    dst: i,
                    a: b,
                    b: one,
                });
                i
            } else {
                b
            };
            let dst = fb.reg(RegTy::I64);
            fb.emit(Inst::LGet {
                dst,
                base: l,
                idx: i,
            });
            dst
        });
        check(
            prog,
            f,
            &["LGet", "LSet"],
            &[0, 1, 3, 8, 9, 63, 64, 127, 128, -1, 1 << 40],
        );
    }
    // Elements of a constant (read only) and of temporary memory.
    let (prog, f) = two(|fb, a, b, _| {
        let c = fb.addr(AddrSpace::Const(ConstId(0)), 0);
        let x = fb.reg(RegTy::I64);
        fb.emit(Inst::LGet {
            dst: x,
            base: c,
            idx: a,
        });
        let n = fb.int(64);
        let t = alloc(fb, n);
        fb.emit(Inst::LSet {
            base: t,
            idx: b,
            v: x,
        });
        let dst = fb.reg(RegTy::I64);
        fb.emit(Inst::LGet {
            dst,
            base: t,
            idx: b,
        });
        dst
    });
    check(prog, f, &["LGet", "LSet"], &[0, 1, 5, 63, 64, 127, 128, -1]);
}

/// `LCopy` of `n` plane elements from offset `b` to offset `a` within ten blocks of temporary
/// memory (overlapping) or from a constant into them, at every edge of a block and beyond
/// the eight blocks compiled code copies itself: the same bytes (a weighted sum of all 32-bit
/// words) or the same error text.
#[test]
fn logic_copies_match_the_interpreter() {
    const LEN: usize = 320;
    let pattern: Vec<u8> = (0..LEN)
        .map(|i| (i as u32).wrapping_mul(2_654_435_761).to_le_bytes()[3])
        .collect();
    for n in [0, 1, 7, 63, 64, 65, 128, 190, 300, 449, 500] {
        for from_const in [false, true] {
            let mut pb = ProgramBuilder::new();
            let k = pb.konst(ConstData::Bytes(pattern.clone()));
            let f = pb.declare("f", FuncKind::Function, None);
            let mut fb = pb.define(f);
            let sa = fb.param("a", Width::B8, ParamClass::Value, Mode::In, None);
            let sb = fb.param("b", Width::B8, ParamClass::Value, Mode::In, None);
            stmt(&mut fb);
            let (pa, pbr) = (fb.addr(AddrSpace::Frame, sa), fb.addr(AddrSpace::Frame, sb));
            let (a, b) = (load(&mut fb, pa, Width::B8), load(&mut fb, pbr, Width::B8));
            let c = fb.addr(AddrSpace::Const(k), 0);
            let size = fb.int(LEN as i64);
            let t = alloc(&mut fb, size);
            fb.emit(Inst::Copy {
                dst: t,
                src: c,
                bytes: size,
            });
            let n = fb.int(n);
            fb.emit(Inst::LCopy {
                dst: t,
                dst_off: a,
                src: if from_const { c } else { t },
                src_off: b,
                n,
            });
            let mut acc = fb.int(0);
            for w in 0..LEN / 4 {
                let at = fb.int(w as i64 * 4);
                let p = index(&mut fb, t, at);
                let v = load(&mut fb, p, Width::B4);
                let m = fb.int(w as i64 + 1);
                let (x, sum) = (fb.reg(RegTy::I64), fb.reg(RegTy::I64));
                fb.emit(Inst::Bin {
                    op: IntOp::Mul,
                    dst: x,
                    a: v,
                    b: m,
                });
                fb.emit(Inst::Bin {
                    op: IntOp::Add,
                    dst: sum,
                    a: acc,
                    b: x,
                });
                acc = sum;
            }
            fb.emit(Inst::Ret { v: Some(acc) });
            fb.finish();
            check(
                pb.finish().unwrap(),
                f,
                &["LCopy"],
                &[0, 1, 5, 63, 64, 65, 127, 128, 200, 639, -1, 1 << 40],
            );
        }
    }
}

/// The scalar `std_ulogic` natives compiled code looks up in a table give what the natives
/// give for all nine codes of each operand, and the native's error beyond them.
#[test]
fn std_ulogic_natives_match_the_interpreter() {
    let gates = ["and", "nand", "or", "nor", "xor", "xnor"].map(|g| {
        (
            format!("yavs.ieee.std_logic_1164.\"{g}\"(std_ulogic,std_ulogic)std_ulogic"),
            2,
        )
    });
    let ones = ["not", "to_x01", "to_x01z", "to_ux01"].map(|n| {
        (
            format!("yavs.ieee.std_logic_1164.\"{n}\"(std_ulogic)std_ulogic"),
            1,
        )
    });
    let truthy = (
        "yavs.ieee.std_logic_1164.\"??\"(std_ulogic)boolean".to_string(),
        1,
    );
    for (sym, n) in gates.into_iter().chain(ones).chain([truthy]) {
        assert!(yavs_interp::code_table(&sym).is_some(), "{sym}");
        let mut pb = ProgramBuilder::new();
        let native = pb.native(&sym, None);
        let f = pb.declare("f", FuncKind::Function, None);
        let mut fb = pb.define(f);
        let sa = fb.param("a", Width::B8, ParamClass::Value, Mode::In, None);
        let sb = fb.param("b", Width::B8, ParamClass::Value, Mode::In, None);
        stmt(&mut fb);
        let (pa, pbr) = (fb.addr(AddrSpace::Frame, sa), fb.addr(AddrSpace::Frame, sb));
        let (a, b) = (load(&mut fb, pa, Width::B8), load(&mut fb, pbr, Width::B8));
        let args = fb.regs(&[a, b][..n]);
        let dst = fb.reg(RegTy::I64);
        fb.emit(Inst::CallNative {
            native,
            args,
            dst: Some(dst),
        });
        fb.emit(Inst::Ret { v: Some(dst) });
        fb.finish();
        check(
            pb.finish().unwrap(),
            f,
            &["CallNative"],
            &[0, 1, 2, 3, 4, 5, 6, 7, 8, 9, -1],
        );
    }
}

#[path = "../../yavs-interp/tests/common/gc_roots.rs"]
mod gc_roots;

/// Compiled code reaches `new` through `TierHost::step` with the registers, frame slots and
/// temporaries in the machine, so a collection there sees the precise roots the interpreter
/// sees (#422): the same objects survive and the unrooted one is collected.
#[test]
fn precise_roots_under_compiled_code() {
    let (prog, p) = gc_roots::program();
    let both = Both::new(prog);
    let tier: Arc<dyn Tier> = both.tier.clone();
    let (r, vals, slots, live) = gc_roots::run(both.prog.clone(), p, Some(tier), 1);
    assert!(
        both.tier.1.load(Ordering::Relaxed) > 0,
        "compiled code never ran"
    );
    assert_eq!(vals, gc_roots::WANT, "{r:?}");
    let e = r.unwrap_err();
    assert!(e.to_string().contains("deallocated"), "{e}");
    gc_roots::assert_churn_collected(slots, live);
}

/// `inc(a) = a + 1` (range checked up to 1_000_000), `run(n)` calling it `n` times in a loop,
/// and `depth(n) = if n = 0 then 0 else depth(n - 1) + 1`.
fn calls() -> (Program, FuncId, FuncId) {
    let mut pb = ProgramBuilder::new();
    let msg = pb.check_msg("result", Dir::To);
    let inc = pb.declare("inc", FuncKind::Function, None);
    let run = pb.declare("run", FuncKind::Function, None);
    let depth = pb.declare("depth", FuncKind::Function, None);
    let arg = |fb: &mut FuncBuilder<'_>| {
        let s = fb.param("a", Width::B8, ParamClass::Value, Mode::In, None);
        stmt(fb);
        let p = fb.addr(AddrSpace::Frame, s);
        load(fb, p, Width::B8)
    };
    {
        let mut fb = pb.define(inc);
        let a = arg(&mut fb);
        let (one, lo, hi) = (fb.int(1), fb.int(0), fb.int(1_000_000));
        let dst = fb.reg(RegTy::I64);
        fb.emit(Inst::BinRange {
            op: IntOp::Add,
            dst,
            a,
            b: one,
            lo,
            hi,
            what: msg,
        });
        fb.emit(Inst::Ret { v: Some(dst) });
        fb.finish();
    }
    {
        let mut fb = pb.define(run);
        let n = arg(&mut fb);
        let (head, body, done) = (fb.block(), fb.block(), fb.block());
        let (i, acc, one) = (fb.int(0), fb.int(0), fb.int(1));
        fb.emit(Inst::Br { to: head });
        fb.at(head);
        let c = fb.reg(RegTy::I64);
        fb.emit(Inst::Cmp {
            op: CmpOp::Lt,
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
        stmt(&mut fb);
        let args = fb.regs(&[acc]);
        fb.emit(Inst::Call {
            func: inc,
            args,
            dst: Some(acc),
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
    }
    {
        let mut fb = pb.define(depth);
        let n = arg(&mut fb);
        let (base, rec) = (fb.block(), fb.block());
        let (zero, one) = (fb.int(0), fb.int(1));
        let c = fb.reg(RegTy::I64);
        fb.emit(Inst::Cmp {
            op: CmpOp::Eq,
            dst: c,
            a: n,
            b: zero,
        });
        fb.emit(Inst::CondBr { c, t: base, f: rec });
        fb.at(base);
        fb.emit(Inst::Ret { v: Some(zero) });
        fb.at(rec);
        let m = fb.reg(RegTy::I64);
        fb.emit(Inst::Bin {
            op: IntOp::Sub,
            dst: m,
            a: n,
            b: one,
        });
        let args = fb.regs(&[m]);
        let d = fb.reg(RegTy::I64);
        fb.emit(Inst::Call {
            func: depth,
            args,
            dst: Some(d),
        });
        fb.emit(Inst::Bin {
            op: IntOp::Add,
            dst: d,
            a: d,
            b: one,
        });
        fb.emit(Inst::Ret { v: Some(d) });
        fb.finish();
    }
    (pb.finish().unwrap(), run, depth)
}

/// `boxed(a)` returns the address of a temporary holding `a + 1`; `sum(n)` adds the values of
/// `n` such calls, one statement each (the caller's temporaries end before every call).
fn temp_calls() -> (Program, FuncId) {
    let mut pb = ProgramBuilder::new();
    let boxed = pb.declare("boxed", FuncKind::Function, None);
    let run = pb.declare("sum", FuncKind::Function, None);
    {
        let mut fb = pb.define(boxed);
        let s = fb.param("a", Width::B8, ParamClass::Value, Mode::In, None);
        stmt(&mut fb);
        let p = fb.addr(AddrSpace::Frame, s);
        let a = load(&mut fb, p, Width::B8);
        let one = fb.int(1);
        let v = fb.reg(RegTy::I64);
        fb.emit(Inst::Bin {
            op: IntOp::Add,
            dst: v,
            a,
            b: one,
        });
        let n = fb.int(8);
        let t = alloc(&mut fb, n);
        fb.emit(Inst::Store {
            addr: t,
            src: v,
            w: Width::B8,
        });
        fb.emit(Inst::Ret { v: Some(t) });
        fb.finish();
    }
    {
        let mut fb = pb.define(run);
        let s = fb.param("n", Width::B8, ParamClass::Value, Mode::In, None);
        stmt(&mut fb);
        let p = fb.addr(AddrSpace::Frame, s);
        let n = load(&mut fb, p, Width::B8);
        let (head, body, done) = (fb.block(), fb.block(), fb.block());
        let (i, acc, one) = (fb.int(0), fb.int(0), fb.int(1));
        fb.emit(Inst::Br { to: head });
        fb.at(head);
        let c = fb.reg(RegTy::I64);
        fb.emit(Inst::Cmp {
            op: CmpOp::Lt,
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
        stmt(&mut fb);
        let args = fb.regs(&[i]);
        let t = fb.reg(RegTy::Ptr);
        fb.emit(Inst::Call {
            func: boxed,
            args,
            dst: Some(t),
        });
        let v = load(&mut fb, t, Width::B8);
        fb.emit(Inst::Bin {
            op: IntOp::Add,
            dst: acc,
            a: acc,
            b: v,
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
    }
    (pb.finish().unwrap(), run)
}

/// A compiled callee returns a temporary of its own to a compiled caller whose statements
/// end their temporaries in compiled code: the interpreter sees the length compiled code left.
#[test]
fn temporaries_across_compiled_calls() {
    let (prog, run) = temp_calls();
    let p = Both::new(prog);
    let (a, b, _) = p.both(run, &[1000]);
    assert_eq!(a, Ok(500_500));
    assert_eq!(a, b);
}

/// Compiled code calls compiled code without returning to the interpreter (#336); an error in
/// the callee, a recursion deeper than the native stack allowance and the results are the
/// interpreter's.
#[test]
fn calls_between_compiled_functions() {
    let (prog, run, depth) = calls();
    let p = Both::new(prog);
    let (a, b, n) = p.both(run, &[100_000]);
    assert_eq!(a, Ok(100_000));
    assert_eq!(a, b);
    // The loop and the calls of `inc` run compiled: a few entries (the first calls may move
    // the register stack, which leaves the call to the interpreter), not one per call.
    assert!(n <= 3, "{n} entries");
    // `inc` fails in its 1_000_001st call, inside the compiled caller.
    let (a, b, _) = p.both(run, &[2_000_000]);
    assert!(a.as_ref().is_err_and(|e| e.contains("result")), "{a:?}");
    assert_eq!(a, b);
    let (a, b, n) = p.both(depth, &[5000]);
    assert_eq!(a, Ok(5000));
    assert_eq!(a, b);
    assert!(n > 1, "{n} entries: no recursion left to the interpreter");
}

/// `up(l, r, dir, k, x, start, e)` and `down(..)`: a descriptor `l, r, dir` over a 16-byte array of
/// zeros but for a 7 at element `k`, and the search loop the interpreter runs as one op (`Pos`,
/// `Index`, `Load`, compare with `x`, compare of the index with `e`, step by 1 or -1) from `start`:
/// `i * 1000 + element` where the element matches, `-i` at `e`.
fn search_loop(step: IntOp) -> (Program, FuncId) {
    let mut pb = ProgramBuilder::new();
    let what = pb.check_msg("index", Dir::To);
    let f = pb.declare("search", FuncKind::Function, None);
    let mut fb = pb.define(f);
    let names = ["l", "r", "dir", "k", "x", "start", "e"];
    let slots: Vec<u32> = names
        .iter()
        .map(|n| fb.param(n, Width::B8, ParamClass::Value, Mode::In, None))
        .collect();
    stmt(&mut fb);
    let v: Vec<Reg> = slots
        .iter()
        .map(|&s| {
            let p = fb.addr(AddrSpace::Frame, s);
            load(&mut fb, p, Width::B8)
        })
        .collect();
    let (l, r, dir, k, x, start, e) = (v[0], v[1], v[2], v[3], v[4], v[5], v[6]);
    let d = dope(&mut fb, l, r, dir);
    let sixteen = fb.int(16);
    let arr = alloc(&mut fb, sixteen);
    let seven = fb.int(7);
    let at = index(&mut fb, arr, k);
    fb.emit(Inst::Store {
        addr: at,
        src: seven,
        w: Width::B1,
    });
    let (head, latch, stepb, found, end) =
        (fb.block(), fb.block(), fb.block(), fb.block(), fb.block());
    let (i, one) = (fb.reg(RegTy::I64), fb.int(1));
    fb.emit(Inst::Move { dst: i, src: start });
    fb.emit(Inst::Br { to: head });
    fb.at(head);
    let (p, a, ch, c) = (
        fb.reg(RegTy::I64),
        fb.reg(RegTy::Ptr),
        fb.reg(RegTy::I64),
        fb.reg(RegTy::I64),
    );
    fb.emit(Inst::Pos {
        dst: p,
        dope: d,
        dim: 0,
        idx: i,
        what,
    });
    fb.emit(Inst::Index {
        dst: a,
        base: arr,
        pos: p,
        stride: 1,
    });
    fb.emit(Inst::Load {
        dst: ch,
        addr: a,
        w: Width::B1,
    });
    fb.emit(Inst::Cmp {
        op: CmpOp::Eq,
        dst: c,
        a: ch,
        b: x,
    });
    fb.emit(Inst::CondBr {
        c,
        t: found,
        f: latch,
    });
    fb.at(latch);
    let c2 = fb.reg(RegTy::I64);
    fb.emit(Inst::Cmp {
        op: CmpOp::Eq,
        dst: c2,
        a: i,
        b: e,
    });
    fb.emit(Inst::CondBr {
        c: c2,
        t: end,
        f: stepb,
    });
    fb.at(stepb);
    fb.emit(Inst::Bin {
        op: step,
        dst: i,
        a: i,
        b: one,
    });
    fb.emit(Inst::Br { to: head });
    fb.at(found);
    let (k1000, m, r1) = (fb.int(1000), fb.reg(RegTy::I64), fb.reg(RegTy::I64));
    fb.emit(Inst::Bin {
        op: IntOp::Mul,
        dst: m,
        a: i,
        b: k1000,
    });
    fb.emit(Inst::Bin {
        op: IntOp::Add,
        dst: r1,
        a: m,
        b: ch,
    });
    fb.emit(Inst::Ret { v: Some(r1) });
    fb.at(end);
    let (zero, r2) = (fb.int(0), fb.reg(RegTy::I64));
    fb.emit(Inst::Bin {
        op: IntOp::Sub,
        dst: r2,
        a: zero,
        b: i,
    });
    fb.emit(Inst::Ret { v: Some(r2) });
    fb.finish();
    // `outer` calls it, so it runs on a native frame.
    let outer = pb.declare("outer", FuncKind::Function, None);
    let mut fb = pb.define(outer);
    let slots: Vec<u32> = names
        .iter()
        .map(|n| fb.param(n, Width::B8, ParamClass::Value, Mode::In, None))
        .collect();
    stmt(&mut fb);
    let v: Vec<Reg> = slots
        .iter()
        .map(|&s| {
            let p = fb.addr(AddrSpace::Frame, s);
            load(&mut fb, p, Width::B8)
        })
        .collect();
    // Room in the temporary area, which native frames do not grow.
    let room = fb.int(4096);
    let spare = alloc(&mut fb, room);
    let zero = fb.int(0);
    fb.emit(Inst::Store {
        addr: spare,
        src: zero,
        w: Width::B1,
    });
    stmt(&mut fb);
    let args = fb.regs(&v);
    let r = fb.reg(RegTy::I64);
    fb.emit(Inst::Call {
        func: f,
        args,
        dst: Some(r),
    });
    fb.emit(Inst::Ret { v: Some(r) });
    fb.finish();
    (pb.finish().unwrap(), outer)
}

/// A search loop in a native frame runs through the scan helper and gives the interpreter's
/// result (the Cranelift lowering, as the LLVM one): found, ended, an index outside the descriptor, bytes outside the array, either
/// direction, either step.
#[test]
fn search_loops_match_the_interpreter() {
    let scans = || {
        yavs_llvm::callback_counts()
            .and_then(|c| c.into_iter().find(|c| c.0 == "scan").map(|c| c.1))
            .unwrap_or(0)
    };
    let before = scans();
    for (step, dirs) in [(IntOp::Add, [0u64, 1]), (IntOp::Sub, [1, 0])] {
        let (prog, f) = search_loop(step);
        let p = Both::new(prog);
        // The loop runs along the direction of the descriptor for the step that goes with it.
        let dir = dirs[0];
        for (l, r) in [
            (0i64, 15i64),
            (1, 16),
            (3, 12),
            (0, 20),
            (15, 0),
            (16, 1),
            (20, 0),
        ] {
            for k in [0u64, 5, 15, 16] {
                for x in [0u64, 7, 9] {
                    for (start, e) in [(l, r), (l, l), (r, l), (l + 1, r), (r + 2, l), (l - 1, r)] {
                        let args = [l as u64, r as u64, dir, k, x, start as u64, e as u64];
                        let (a, b, _) = p.both(f, &args);
                        assert_eq!(a, b, "{step:?} {args:?}");
                    }
                }
            }
        }
    }
    if cfg!(debug_assertions) {
        assert!(scans() > before, "no loop went through the helper");
    }
}
