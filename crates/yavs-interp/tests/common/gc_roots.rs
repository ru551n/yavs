//! Precise roots across collections inside an activation (#422), shared by the interpreter
//! tests and the LLVM tier tests (`crates/yavs-llvm/tests/tier.rs` includes this file).
//!
//! A serial process allocates objects whose handles live only in a register of its frame,
//! only in a temporary (a scalar and an array of handles), in a frame slot while it is the
//! caller, in a record in its frame and in an array object, one reached only through an
//! address in a `Ptr` register, and one whose handle sits only in an integer slot no root map
//! names. It then calls `churn`, which keeps one object in its
//! own register and allocates `CHURN` dropped objects, so the shared heap is collected many
//! times inside the activation. The process writes what it reads back into the shared
//! variables area, then dereferences the unrooted handle, which must have been collected.

use std::sync::{Arc, Mutex};

use yavs_diag::{FileId, Span};
use yavs_interp::{Bindings, InstData, InterpProcess, Limits, Runtime, SharedState, Tier};
use yavs_ir::*;
use yavs_kernel::{Builder, Config, SimError, Time};

pub const CHURN: i64 = 2000;
/// The values the process reads back, in shared-variable order.
pub const WANT: [u64; 9] = [9, 1, 2, 3, 4, 5, 6, 8, 10];
const OUT: u32 = 8;

fn st(fb: &mut FuncBuilder<'_>, kind: StmtKind) {
    fb.stmt(kind, Span::new(FileId(0), 0, 1));
}

fn alloc(fb: &mut FuncBuilder<'_>, n: i64) -> Reg {
    let size = fb.int(n);
    let dst = fb.reg(RegTy::Ptr);
    fb.emit(Inst::Alloc { dst, size });
    dst
}

fn load(fb: &mut FuncBuilder<'_>, t: RegTy, addr: Reg) -> Reg {
    let dst = fb.reg(t);
    fb.emit(Inst::Load {
        dst,
        addr,
        w: Width::B8,
    });
    dst
}

fn store(fb: &mut FuncBuilder<'_>, addr: Reg, src: Reg) {
    fb.emit(Inst::Store {
        addr,
        src,
        w: Width::B8,
    });
}

fn at(fb: &mut FuncBuilder<'_>, base: Reg, off: i64) -> Reg {
    let pos = fb.int(off);
    let dst = fb.reg(RegTy::Ptr);
    fb.emit(Inst::Index {
        dst,
        base,
        pos,
        stride: 1,
    });
    dst
}

fn deref(fb: &mut FuncBuilder<'_>, handle: Reg, what: CheckMsgId) -> Reg {
    let dst = fb.reg(RegTy::Ptr);
    fb.emit(Inst::Deref { dst, handle, what });
    dst
}

/// `new integer'(v)`: a handle register.
fn new_int(fb: &mut FuncBuilder<'_>, int: TyId, v: i64) -> Reg {
    let init = alloc(fb, 8);
    let vr = fb.int(v);
    store(fb, init, vr);
    let size = fb.int(8);
    let h = fb.reg(RegTy::Handle);
    fb.emit(Inst::New {
        dst: h,
        ty: int,
        init,
        size,
        shared: false,
    });
    h
}

/// `h.all` of an integer object.
fn read(fb: &mut FuncBuilder<'_>, h: Reg, what: CheckMsgId) -> Reg {
    let p = deref(fb, h, what);
    load(fb, RegTy::I64, p)
}

/// The program and its process.
pub fn program() -> (Program, FuncId) {
    let mut pb = ProgramBuilder::new();
    let int = pb.ty(Ty::Int {
        lo: i64::MIN,
        hi: i64::MAX,
    });
    let acc = pb.ty(Ty::Access { target: int });
    let rec = pb.ty(Ty::Record {
        fields: vec![
            Field {
                name: "v".into(),
                ty: int,
                desc: None,
            },
            Field {
                name: "next".into(),
                ty: acc,
                desc: None,
            },
        ],
    });
    let arr = pb.ty(Ty::Array {
        elem: acc,
        elem_desc: None,
        dims: 1,
    });
    let what = pb.check_msg("p.all", Dir::To);
    let churn = pb.declare("churn", FuncKind::Function, None);
    let p = pb.declare("p", FuncKind::Process, None);
    // function churn return integer: k := new integer'(9) (then only in a register of the
    // top frame), CHURN dropped objects, return k.all.
    {
        let mut fb = pb.define(churn);
        let slot = fb.local("k", int, 8, 8);
        st(&mut fb, StmtKind::Return);
        // Through an unrooted slot, so compiled code loads the handle into a register itself.
        let k = new_int(&mut fb, int, 9);
        let a = fb.addr(AddrSpace::Frame, slot);
        store(&mut fb, a, k);
        let zero = fb.int(0);
        let k = load(&mut fb, RegTy::Handle, a);
        store(&mut fb, a, zero);
        let i = fb.int(0);
        let (head, body, done) = (fb.block(), fb.block(), fb.block());
        fb.emit(Inst::Br { to: head });
        fb.at(head);
        let n = fb.int(CHURN);
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
        new_int(&mut fb, int, 0);
        let one = fb.int(1);
        fb.emit(Inst::Bin {
            op: IntOp::Add,
            dst: i,
            a: i,
            b: one,
        });
        fb.emit(Inst::Br { to: head });
        fb.at(done);
        let v = read(&mut fb, k, what);
        fb.emit(Inst::Ret { v: Some(v) });
        fb.finish();
    }
    {
        let mut fb = pb.define(p);
        let keep = fb.local("keep", acc, 8, 8);
        fb.root(keep, RootKind::Handle);
        let rs = fb.local("r", rec, 16, 8);
        fb.root(rs, RootKind::Composite(rec));
        let arrh = fb.local("a", acc, 8, 8);
        fb.root(arrh, RootKind::Handle);
        let raw = fb.local("raw", int, 8, 8);
        let out = |fb: &mut FuncBuilder<'_>, k: u32, v: Reg| {
            let a = fb.addr(AddrSpace::Shared, OUT * k);
            store(fb, a, v);
        };
        st(&mut fb, StmtKind::VarAssign);
        // One statement: its temporaries live until the reads below.
        let in_reg = new_int(&mut fb, int, 1);
        let t = alloc(&mut fb, 8);
        fb.emit(Inst::TempRoot {
            addr: t,
            desc: None,
            ty: Some(acc),
        });
        let h = new_int(&mut fb, int, 2);
        store(&mut fb, t, h);
        // `(new integer'(3), new integer'(4))` in a temporary: descriptor `0 to 1`.
        let (l, r, d) = (fb.int(0), fb.int(1), fb.int(0));
        let dope = alloc(&mut fb, 24);
        let parts = fb.regs(&[l, r, d]);
        fb.emit(Inst::MakeDope { at: dope, parts });
        let ta = alloc(&mut fb, 16);
        fb.emit(Inst::TempRoot {
            addr: ta,
            desc: Some(dope),
            ty: Some(arr),
        });
        for (k, v) in [(0, 3), (8, 4)] {
            let h = new_int(&mut fb, int, v);
            let a = at(&mut fb, ta, k);
            store(&mut fb, a, h);
        }
        let h = new_int(&mut fb, int, 5);
        let a = fb.addr(AddrSpace::Frame, keep);
        store(&mut fb, a, h);
        let h = new_int(&mut fb, int, 6);
        let a = fb.addr(AddrSpace::Frame, rs + 8);
        store(&mut fb, a, h);
        // An array object `(new integer'(8), null)`: `[descriptor][elements]`.
        let init = alloc(&mut fb, 40);
        let n24 = fb.int(24);
        fb.emit(Inst::Copy {
            dst: init,
            src: dope,
            bytes: n24,
        });
        // Its initial value is a temporary until `new` copies it, as lowering records it.
        let e = at(&mut fb, init, 24);
        fb.emit(Inst::TempRoot {
            addr: e,
            desc: Some(init),
            ty: Some(arr),
        });
        let h = new_int(&mut fb, int, 8);
        store(&mut fb, e, h);
        let n40 = fb.int(40);
        let ha = fb.reg(RegTy::Handle);
        fb.emit(Inst::New {
            dst: ha,
            ty: arr,
            init,
            size: n40,
            shared: false,
        });
        let a = fb.addr(AddrSpace::Frame, arrh);
        store(&mut fb, a, ha);
        let h = new_int(&mut fb, int, 7);
        let a = fb.addr(AddrSpace::Frame, raw);
        store(&mut fb, a, h);
        // `new integer'(10)` reached only through the address `p.all` in a `Ptr` register.
        let h = new_int(&mut fb, int, 10);
        let by_addr = deref(&mut fb, h, what);
        let args = fb.regs(&[]);
        let v = fb.reg(RegTy::I64);
        fb.emit(Inst::Call {
            func: churn,
            args,
            dst: Some(v),
        });
        out(&mut fb, 0, v);
        let v = read(&mut fb, in_reg, what);
        out(&mut fb, 1, v);
        let h = load(&mut fb, RegTy::Handle, t);
        let v = read(&mut fb, h, what);
        out(&mut fb, 2, v);
        for (k, off) in [(3, 0), (4, 8)] {
            let a = at(&mut fb, ta, off);
            let h = load(&mut fb, RegTy::Handle, a);
            let v = read(&mut fb, h, what);
            out(&mut fb, k, v);
        }
        for (k, off) in [(5, keep), (6, rs + 8)] {
            let a = fb.addr(AddrSpace::Frame, off);
            let h = load(&mut fb, RegTy::Handle, a);
            let v = read(&mut fb, h, what);
            out(&mut fb, k, v);
        }
        let a = fb.addr(AddrSpace::Frame, arrh);
        let h = load(&mut fb, RegTy::Handle, a);
        let p = deref(&mut fb, h, what);
        let e = at(&mut fb, p, 24);
        let h = load(&mut fb, RegTy::Handle, e);
        let v = read(&mut fb, h, what);
        out(&mut fb, 7, v);
        let v = load(&mut fb, RegTy::I64, by_addr);
        out(&mut fb, 8, v);
        // The handle no root map names was collected: a stale access value.
        st(&mut fb, StmtKind::VarAssign);
        let a = fb.addr(AddrSpace::Frame, raw);
        let h = load(&mut fb, RegTy::Handle, a);
        read(&mut fb, h, what);
        st(&mut fb, StmtKind::Wait);
        fb.emit(Inst::Suspend { resume: 0 });
        fb.emit(Inst::Ret { v: None });
        fb.finish();
    }
    (pb.finish().unwrap(), p)
}

/// Run the process to its end at `threads` threads, with `tier` when given: the run's
/// result, the values it read back and the shared heap's slot and live object counts.
pub fn run(
    prog: Arc<Program>,
    p: FuncId,
    tier: Option<Arc<dyn Tier>>,
    threads: usize,
) -> (Result<(), SimError>, Vec<u64>, usize, usize) {
    let b = Bindings {
        limits: Limits {
            gc_threshold: 256,
            ..Limits::default()
        },
        ..Bindings::default()
    };
    let rt = Runtime::new(prog.clone(), b).unwrap();
    if let Some(t) = tier {
        assert!(rt.set_tier(t));
    }
    let shared = Arc::new(Mutex::new(SharedState::new(vec![0; 72])));
    let ip = InterpProcess::new(
        rt.clone(),
        p,
        Arc::new(InstData::default()),
        vec![],
        Some(shared.clone()),
    )
    .unwrap();
    let mut b = Builder::new();
    b.process("p", yavs_interp::proc_flags(&prog, p), &[], ip);
    let mut sim = b
        .build(Config {
            threads,
            ..Config::default()
        })
        .unwrap();
    sim.subscribe(Box::new(yavs_interp::SharedGc {
        rt,
        shared: shared.clone(),
    }));
    let r = sim.run_until(Time::ns(1)).map(drop);
    let s = shared.lock().unwrap();
    let vals = s
        .vars
        .chunks(8)
        .map(|c| u64::from_le_bytes(c.try_into().unwrap()))
        .collect();
    (r, vals, s.heap_slots(), s.heap_live())
}

/// The `CHURN` dropped objects were collected. Slots are recycled for new objects, so the
/// heap stays small; the `gc-stress` build never reuses a slot, so there the live objects
/// are counted instead (every collection runs and frees them, the slots stay).
pub fn assert_churn_collected(slots: usize, live: usize) {
    if cfg!(feature = "gc-stress") {
        assert!(live < 200, "{live} live objects of {slots} slots");
    } else {
        assert!(slots < 200, "{slots} slots");
    }
}
