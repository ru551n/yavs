//! Hand-written IR helpers and a kernel harness that runs a design at 1 and 4 threads.
#![allow(dead_code)]

use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use yavs_diag::{FileId, Span};
use yavs_interp::{InstData, InstSig, InterpProcess, ProcDriver, Runtime, SharedState, proc_flags};
use yavs_ir::*;
use yavs_kernel::*;

pub const NS: i64 = 1_000_000;

/// A distinct span per statement, so traces show which statement failed.
pub fn sp(n: u32) -> Span {
    Span::new(FileId(0), n, n + 1)
}

pub fn stmt(fb: &mut FuncBuilder<'_>, kind: StmtKind) -> StmtId {
    static N: AtomicU32 = AtomicU32::new(0);
    fb.stmt(kind, sp(N.fetch_add(2, Ordering::Relaxed)))
}

/// `Suspend` with no sensitivity and no timeout: `wait;`.
pub fn wait_forever(fb: &mut FuncBuilder<'_>) {
    stmt(fb, StmtKind::Wait);
    fb.emit(Inst::Suspend { resume: 0 });
    fb.emit(Inst::Ret { v: None });
}

/// `driver <= <bytes at src> after delay`.
pub fn drive_mem(fb: &mut FuncBuilder<'_>, driver: u32, delay: i64, src: Reg, bytes: i64) {
    let size = fb.int(8 + (bytes + 7) / 8 * 8);
    let wave = fb.reg(RegTy::Ptr);
    fb.emit(Inst::Alloc { dst: wave, size });
    let t = fb.int(delay);
    fb.emit(Inst::Store {
        addr: wave,
        src: t,
        w: Width::B8,
    });
    let eight = fb.int(8);
    let at = fb.reg(RegTy::Ptr);
    fb.emit(Inst::Index {
        dst: at,
        base: wave,
        pos: eight,
        stride: 1,
    });
    let n = fb.int(bytes);
    fb.emit(Inst::Copy {
        dst: at,
        src,
        bytes: n,
    });
    let one = fb.int(1);
    fb.emit(Inst::Drive {
        driver,
        target: None,
        wave,
        n: one,
        mode: DriveMode::Inertial,
        reject: None,
    });
}

/// A reference to instance signal `idx`, elements `off .. off + len`.
pub fn sig(fb: &mut FuncBuilder<'_>, idx: u32, off: i64, len: i64) -> Reg {
    let dst = fb.reg(RegTy::Sig);
    let (o, l) = (fb.int(off), fb.int(len));
    fb.emit(Inst::SigVal {
        dst,
        sig: idx,
        offset: o,
        len: l,
    });
    dst
}

pub fn read_scalar(fb: &mut FuncBuilder<'_>, idx: u32) -> Reg {
    let s = sig(fb, idx, 0, 1);
    let dst = fb.reg(RegTy::I64);
    fb.emit(Inst::SigReadScalar { dst, sig: s });
    dst
}

pub fn bin(fb: &mut FuncBuilder<'_>, op: IntOp, a: Reg, b: Reg) -> Reg {
    let dst = fb.reg(RegTy::I64);
    fb.emit(Inst::Bin { op, dst, a, b });
    dst
}

pub fn cmp(fb: &mut FuncBuilder<'_>, op: CmpOp, a: Reg, b: Reg) -> Reg {
    let dst = fb.reg(RegTy::I64);
    fb.emit(Inst::Cmp { op, dst, a, b });
    dst
}

pub fn load(fb: &mut FuncBuilder<'_>, space: AddrSpace, off: u32, w: Width) -> Reg {
    let a = fb.addr(space, off);
    let dst = fb.reg(RegTy::I64);
    fb.emit(Inst::Load { dst, addr: a, w });
    dst
}

/// An access handle at `off` of `space`.
pub fn load_handle(fb: &mut FuncBuilder<'_>, space: AddrSpace, off: u32) -> Reg {
    let a = fb.addr(space, off);
    let dst = fb.reg(RegTy::Handle);
    fb.emit(Inst::Load {
        dst,
        addr: a,
        w: Width::B8,
    });
    dst
}

pub fn store(fb: &mut FuncBuilder<'_>, space: AddrSpace, off: u32, v: Reg, w: Width) {
    let a = fb.addr(space, off);
    fb.emit(Inst::Store { addr: a, src: v, w });
}

/// `driver <= value after delay` with a one-element waveform of an `es`-byte value.
pub fn drive(fb: &mut FuncBuilder<'_>, driver: u32, delay: i64, v: Reg, w: Width) {
    let size = fb.int(16);
    let wave = fb.reg(RegTy::Ptr);
    fb.emit(Inst::Alloc { dst: wave, size });
    let t = fb.int(delay);
    fb.emit(Inst::Store {
        addr: wave,
        src: t,
        w: Width::B8,
    });
    let eight = fb.int(8);
    let at = fb.reg(RegTy::Ptr);
    fb.emit(Inst::Index {
        dst: at,
        base: wave,
        pos: eight,
        stride: 1,
    });
    fb.emit(Inst::Store {
        addr: at,
        src: v,
        w,
    });
    let n = fb.int(1);
    fb.emit(Inst::Drive {
        driver,
        target: None,
        wave,
        n,
        mode: DriveMode::Inertial,
        reject: None,
    });
}

/// `report "text"`.
pub fn report(fb: &mut FuncBuilder<'_>, text: &str) {
    let c = fb
        .program()
        .konst(ConstData::Bytes(text.as_bytes().to_vec()));
    let a = fb.addr(AddrSpace::Const(c), 0);
    let len = fb.int(text.len() as i64);
    let sev = fb.int(0);
    fb.emit(Inst::Report {
        msg: a,
        len,
        severity: sev,
    });
}

/// Data address and length of a fat string `[data, dope]`.
pub fn fat_parts(fb: &mut FuncBuilder<'_>, fat: Reg) -> (Reg, Reg) {
    let data = fb.reg(RegTy::Ptr);
    fb.emit(Inst::Load {
        dst: data,
        addr: fat,
        w: Width::B8,
    });
    let eight = fb.int(8);
    let pd = fb.reg(RegTy::Ptr);
    fb.emit(Inst::Index {
        dst: pd,
        base: fat,
        pos: eight,
        stride: 1,
    });
    let dope = fb.reg(RegTy::Ptr);
    fb.emit(Inst::Load {
        dst: dope,
        addr: pd,
        w: Width::B8,
    });
    let len = fb.reg(RegTy::I64);
    fb.emit(Inst::DopeLen {
        dst: len,
        dope,
        dim: 0,
    });
    (data, len)
}

/// `report prefix & <fat string>`.
pub fn report_with(fb: &mut FuncBuilder<'_>, prefix: &str, fat: Reg) {
    let (data, len) = fat_parts(fb, fat);
    let c = fb
        .program()
        .konst(ConstData::Bytes(prefix.as_bytes().to_vec()));
    let pa = fb.addr(AddrSpace::Const(c), 0);
    let plen = fb.int(prefix.len() as i64);
    let total = bin(fb, IntOp::Add, plen, len);
    let buf = fb.reg(RegTy::Ptr);
    fb.emit(Inst::Alloc {
        dst: buf,
        size: total,
    });
    fb.emit(Inst::Copy {
        dst: buf,
        src: pa,
        bytes: plen,
    });
    let tail = fb.reg(RegTy::Ptr);
    fb.emit(Inst::Index {
        dst: tail,
        base: buf,
        pos: plen,
        stride: 1,
    });
    fb.emit(Inst::Copy {
        dst: tail,
        src: data,
        bytes: len,
    });
    let sev = fb.int(0);
    fb.emit(Inst::Report {
        msg: buf,
        len: total,
        severity: sev,
    });
}

/// `report prefix & T'image(v)` through the native `yavs.attr.image`.
pub fn report_image(fb: &mut FuncBuilder<'_>, prefix: &str, ty: TyId, v: Reg) {
    let n = fb.program().native("yavs.attr.image", Some(ty));
    let args = fb.regs(&[v]);
    let fat = fb.reg(RegTy::Ptr);
    fb.emit(Inst::CallNative {
        native: n,
        args,
        dst: Some(fat),
    });
    report_with(fb, prefix, fat);
}

/// `WaitOn` signal `idx`, `Suspend`, then branch to `to`.
pub fn wait_on_loop(fb: &mut FuncBuilder<'_>, idx: u32, to: BlockId) {
    stmt(fb, StmtKind::Wait);
    let s = sig(fb, idx, 0, 1);
    fb.emit(Inst::WaitOn { sig: s });
    fb.emit(Inst::Suspend { resume: 0 });
    fb.emit(Inst::Br { to });
}

pub fn wait_for(fb: &mut FuncBuilder<'_>, t: i64) {
    stmt(fb, StmtKind::Wait);
    let r = fb.int(t);
    fb.emit(Inst::WaitFor { t: r });
    fb.emit(Inst::Suspend { resume: 0 });
}

pub fn integer(pb: &mut ProgramBuilder) -> TyId {
    pb.ty(Ty::Int {
        lo: i32::MIN.into(),
        hi: i32::MAX.into(),
    })
}

pub fn time_ty(pb: &mut ProgramBuilder) -> TyId {
    pb.ty(Ty::Physical {
        lo: 0,
        hi: i64::MAX,
        units: vec![
            ("fs".into(), 1),
            ("ps".into(), 1000),
            ("ns".into(), 1_000_000),
        ],
    })
}

/// A process instance to build: function, signals it drives (instance index, len).
pub struct ProcSpec {
    pub name: &'static str,
    pub func: FuncId,
    pub drives: Vec<(u32, u32)>,
}

/// Records value changes, reports and output as text lines.
#[derive(Clone, Default)]
pub struct Trace(pub Arc<Mutex<Vec<String>>>);

impl Observer for Trace {
    fn value_change(&mut self, h: &mut Hook<'_>, s: SignalId, r: Range) {
        let v = h.value(s.slice(r.offset, r.len)).unwrap();
        let name = h.signal_name(s).unwrap().to_string();
        let line = format!("{} +{} {name}={v}", h.now(), h.delta());
        self.0.lock().unwrap().push(line);
    }

    fn output(&mut self, h: &mut Hook<'_>, p: ProcId, bytes: &[u8]) {
        let text = String::from_utf8_lossy(bytes);
        let line = format!("{} +{} p{} out: {text:?}", h.now(), h.delta(), p.0);
        self.0.lock().unwrap().push(line);
    }

    fn report(&mut self, h: &mut Hook<'_>, r: &Report) {
        let line = format!(
            "{} +{} p{} {:?}: {}",
            h.now(),
            h.delta(),
            r.process.map_or(u32::MAX, |p| p.0),
            r.severity,
            r.message
        );
        self.0.lock().unwrap().push(line);
    }
}

pub struct Design {
    pub prog: Arc<Program>,
    pub signals: Vec<(&'static str, SignalType, Value)>,
    pub procs: Vec<ProcSpec>,
}

impl Design {
    pub fn new(pb: ProgramBuilder) -> Design {
        let prog = match pb.finish() {
            Ok(p) => Arc::new(p),
            Err(e) => panic!("verifier: {e:#?}"),
        };
        Design {
            prog,
            signals: Vec::new(),
            procs: Vec::new(),
        }
    }

    pub fn build(&self, threads: usize) -> (SimControl, Trace, Arc<Runtime>) {
        self.build_with(threads, |p, _| p)
    }

    pub fn build_with(
        &self,
        threads: usize,
        wrap: impl Fn(InterpProcess, &str) -> InterpProcess,
    ) -> (SimControl, Trace, Arc<Runtime>) {
        let rt = Runtime::new(self.prog.clone(), Default::default()).unwrap();
        self.build_rt(threads, rt, wrap)
    }

    pub fn build_rt(
        &self,
        threads: usize,
        rt: Arc<Runtime>,
        wrap: impl Fn(InterpProcess, &str) -> InterpProcess,
    ) -> (SimControl, Trace, Arc<Runtime>) {
        let mut b = Builder::new();
        let mut sigs = Vec::new();
        for (name, ty, init) in &self.signals {
            let id = b.signal(name, *ty, init);
            sigs.push(InstSig {
                signal: id,
                ty: *ty,
                off: 0,
            });
        }
        let inst = Arc::new(InstData {
            data: Vec::new(),
            sigs: sigs.clone(),
            cov_base: 0,
        });
        for p in &self.procs {
            let mut ids = Vec::new();
            let mut drivers = Vec::new();
            for &(s, len) in &p.drives {
                let range = Range::new(0, len);
                let id = b.driver(sigs[s as usize].signal.slice(0, len));
                ids.push(id);
                drivers.push(ProcDriver { id, sig: s, range });
            }
            // File users are serial (#82) and get the (here empty) shared state.
            let shared = self
                .prog
                .is_serial(p.func)
                .then(|| Arc::new(Mutex::new(SharedState::new(Vec::new()))));
            let ip = InterpProcess::new(rt.clone(), p.func, inst.clone(), drivers, shared).unwrap();
            let ip = wrap(ip, p.name);
            b.process(p.name, proc_flags(&self.prog, p.func), &ids, ip);
        }
        let cfg = Config {
            threads,
            ..Config::default()
        };
        let mut sim = b.build(cfg).unwrap();
        let t = Trace::default();
        sim.subscribe(Box::new(t.clone()));
        (sim, t, rt)
    }

    /// Run to `limit` at 1 and 4 threads; both must give the same trace and result.
    pub fn run(&self, limit: Time) -> (Vec<String>, Result<RunStatus, SimError>) {
        let mut out = Vec::new();
        for threads in [1, 4] {
            let (mut sim, t, _) = self.build(threads);
            let r = sim.run_until(limit);
            let lines = t.0.lock().unwrap().clone();
            out.push((lines, r));
        }
        let b = out.pop().unwrap();
        let a = out.pop().unwrap();
        assert_eq!(a.0, b.0, "traces differ between 1 and 4 threads");
        assert_eq!(a.1, b.1, "results differ between 1 and 4 threads");
        a
    }
}

pub fn has(lines: &[String], s: &str) -> bool {
    lines.iter().any(|l| l.contains(s))
}

/// A fat pair `[data, dope]` for a constant string.
pub fn fat_str(fb: &mut FuncBuilder<'_>, s: &[u8]) -> Reg {
    let c = fb.program().konst(ConstData::Bytes(s.to_vec()));
    let data = fb.addr(AddrSpace::Const(c), 0);
    fat_of(fb, data, 1, s.len() as i64, false)
}

/// A fat pair `[data, dope]` in temporary memory with one dimension `l .. r`.
pub fn fat_of(fb: &mut FuncBuilder<'_>, data: Reg, l: i64, r: i64, downto: bool) -> Reg {
    let size = fb.int(40);
    let fp = fb.reg(RegTy::Ptr);
    fb.emit(Inst::Alloc { dst: fp, size });
    let sixteen = fb.int(16);
    let dope = fb.reg(RegTy::Ptr);
    fb.emit(Inst::Index {
        dst: dope,
        base: fp,
        pos: sixteen,
        stride: 1,
    });
    let parts = [fb.int(l), fb.int(r), fb.int(downto.into())];
    let parts = fb.regs(&parts);
    fb.emit(Inst::MakeDope { at: dope, parts });
    fb.emit(Inst::Store {
        addr: fp,
        src: data,
        w: Width::B8,
    });
    let eight = fb.int(8);
    let a = fb.reg(RegTy::Ptr);
    fb.emit(Inst::Index {
        dst: a,
        base: fp,
        pos: eight,
        stride: 1,
    });
    fb.emit(Inst::Store {
        addr: a,
        src: dope,
        w: Width::B8,
    });
    fp
}

/// Call native `sym` (type `ty` for implicit families); `ret` gives a result register.
pub fn native(
    fb: &mut FuncBuilder<'_>,
    sym: &str,
    ty: Option<TyId>,
    args: &[Reg],
    ret: bool,
) -> Option<Reg> {
    let n = fb.program().native(sym, ty);
    let args = fb.regs(args);
    let dst = ret.then(|| fb.reg(RegTy::I64));
    fb.emit(Inst::CallNative {
        native: n,
        args,
        dst,
    });
    dst
}

/// Call a native returning a string (the address of a fat pair).
pub fn native_str(fb: &mut FuncBuilder<'_>, sym: &str, args: &[Reg]) -> Reg {
    let n = fb.program().native(sym, None);
    let args = fb.regs(args);
    let dst = fb.reg(RegTy::Ptr);
    fb.emit(Inst::CallNative {
        native: n,
        args,
        dst: Some(dst),
    });
    dst
}
