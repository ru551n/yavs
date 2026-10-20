//! The runtime half of the compiled tiers: the frame view (`Mem`), the callbacks compiled code
//! calls (`yavs_rt_*`), `enter`, and the per-instruction facts both code generators share.
//! No code generator here: it builds without LLVM or Cranelift.

use std::any::Any;
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

use yavs_interp::{Stepped, TIER_SUSPEND, TIER_UNWIND, TierFrame, TierHost, space};
use yavs_ir::{AddrSpace, Func, FuncId, Inst, Program, Reg, Width};

pub const STEP: &str = "yavs_rt_step";
pub const HOST: &str = "yavs_rt_host";
pub const SIGNAL: &str = "yavs_rt_signal";
pub const LCOPY: &str = "yavs_rt_lcopy";

/// The most plane blocks an inline `LCopy` reads or writes (`yavs_rt_lcopy`'s stack copy).
pub const LCOPY_BLOCKS: u64 = 8;

pub type RunFn = unsafe extern "C" fn(*mut c_void, *mut u64, *mut Mem, u32) -> u32;

/// The compiled code of a function and its register count, if it has some.
pub type Lookup<'a> = &'a dyn Fn(FuncId) -> Option<(RunFn, usize)>;

/// `yavs_rt_step` returns it when compiled code goes on with the next instruction; anything
/// else is what the compiled function returns at once (a pc, `TIER_UNWIND`, `TIER_SUSPEND`).
pub const GO: u32 = u32::MAX;

/// Native stack compiled code may use for calls between compiled functions (`yavs_rt_step`):
/// deeper calls are left to the interpreter, whose frames are not on the native stack.
/// Simulation threads have 2 MiB.
pub const NEST_STACK: usize = 256 << 10;

/// Set in the pc compiled code passes `yavs_rt_step` when the instruction cannot free or
/// resize a memory object (`keeps_objects`): the objects `Mem` found stay valid.
pub const KEEP: u32 = 1 << 31;

/// What compiled code reaches through its `mem` argument (field offsets: `MEM_*`): the
/// frame view the interpreter keeps current ([`TierFrame`]; compiled code keeps the
/// temporary area's length in it and hands it back before every step and at the end), and
/// the last objects `yavs_rt_obj` found, valid until a step (`obj_next` is the entry it
/// replaces next).
#[repr(C)]
pub struct Mem {
    fr: TierFrame,
    objs: [Obj; OBJS],
    obj_next: u64,
    /// The temporary area's length the interpreter last gave: no hand-back while unchanged.
    temp_seen: u64,
}

/// An object in `Mem::objs`: the high half of its addresses (`u64::MAX`: none), start,
/// length and whether it was found for writing.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Obj {
    hi: u64,
    ptr: *mut u8,
    len: u64,
    w: u64,
}

pub const OBJS: usize = 4;

pub const NO_OBJ: Obj = Obj {
    hi: u64::MAX,
    ptr: std::ptr::null_mut(),
    len: 0,
    w: 0,
};

pub const MEM_SLOTS: u64 = std::mem::offset_of!(Mem, fr.slots) as u64;
pub const MEM_SLOTS_LEN: u64 = std::mem::offset_of!(Mem, fr.slots_len) as u64;
pub const MEM_FRAME: u64 = std::mem::offset_of!(Mem, fr.frame_hi) as u64;
pub const MEM_TEMP: u64 = std::mem::offset_of!(Mem, fr.temp) as u64;
pub const MEM_TEMP_LEN: u64 = std::mem::offset_of!(Mem, fr.temp_len) as u64;
pub const MEM_TEMP_CAP: u64 = std::mem::offset_of!(Mem, fr.temp_cap) as u64;
pub const MEM_TEMP_BASE: u64 = std::mem::offset_of!(Mem, fr.temp_base) as u64;
pub const MEM_UP: u64 = std::mem::offset_of!(Mem, fr.up) as u64;
pub const MEM_NSIGS: u64 = std::mem::offset_of!(Mem, fr.nsigs) as u64;
pub const MEM_TEMP_ROOTS: u64 = std::mem::offset_of!(Mem, fr.temp_roots) as u64;
pub const MEM_INST: u64 = std::mem::offset_of!(Mem, fr.inst) as u64;
pub const MEM_INST_LEN: u64 = std::mem::offset_of!(Mem, fr.inst_len) as u64;

impl Mem {
    fn of(fr: TierFrame) -> Mem {
        Mem {
            fr,
            objs: [NO_OBJ; OBJS],
            obj_next: 0,
            temp_seen: fr.temp_len,
        }
    }

    /// Give the temporary area the length compiled code left it.
    fn sync(&mut self, host: &mut dyn TierHost) {
        if self.fr.temp_len != self.temp_seen {
            // SAFETY: compiled code only shrinks the length or grows it within the capacity
            // after zeroing the new bytes (`Inst::Alloc`), so they are initialised.
            unsafe { host.temp().set_len(self.fr.temp_len as usize) }
        }
    }

    /// Forget the objects found before a step, except constants (they never move or change).
    fn forget(&mut self) {
        let objs = std::mem::replace(&mut self.objs, [NO_OBJ; OBJS]);
        self.obj_next = 0;
        for o in objs {
            if o.hi != u64::MAX && o.hi >> 29 == u64::from(space::CONST) {
                self.objs[self.obj_next as usize] = o;
                self.obj_next += 1;
            }
        }
    }
}

/// What a running compiled function reaches through `env`.
pub struct Env<'a> {
    host: &'a mut dyn TierHost,
    lookup: Lookup<'a>,
    /// Below this native stack address no compiled callee is entered (`NEST_STACK`).
    stack_lo: usize,
    panic: Option<Box<dyn Any + Send>>,
}

/// The address of a local of the caller: where the native stack is.
#[inline(always)]
pub fn stack_here() -> usize {
    let x = 0u8;
    std::hint::black_box(&x) as *const u8 as usize
}

/// Executes one instruction in the interpreter (`TierHost::step`) and returns `GO`, or what
/// the compiled code returns: the pc after a fault, `TIER_SUSPEND`, `TIER_UNWIND`. A frame the
/// instruction pushed runs here when its function has compiled code, and the call continues
/// as `GO` once that returns (`TierHost::ret`); otherwise the frame is the interpreter's
/// (`TIER_UNWIND`).
///
/// # Safety
/// `env` and `mem` are the live `Env` and `Mem` of compiled code that is blocked in this call.
pub unsafe extern "C" fn yavs_rt_step(
    env: *mut c_void,
    _regs: *mut u64,
    mem: *mut Mem,
    pc: u32,
) -> u32 {
    // SAFETY: `env` and `mem` are what `enter` (or this function, for a callee) passed to the
    // compiled function, which is blocked in this call; `regs` points into the register
    // stack, which the compiled code does not touch until we return (it passes it only so
    // LLVM reloads registers after the call). `env` is reborrowed briefly: a compiled callee
    // run from here reaches it again through the raw pointer.
    // `env` and `mem` are reborrowed briefly: a compiled callee run from here reaches them
    // again through the raw pointers.
    let envp = env as *mut Env<'_>;
    let (keep, pc) = (pc & KEEP != 0, pc & !KEEP);
    unsafe { (*mem).sync(&mut *(*envp).host) };
    // No unwinding through JIT frames: a panic is carried over the call and resumed after.
    let r = match catch_unwind(AssertUnwindSafe(|| unsafe { step_in(envp, mem, pc) })) {
        Ok(r) => r,
        Err(p) => {
            unsafe { (*envp).panic = Some(p) };
            pc
        }
    };
    let mem = unsafe { &mut *mem };
    mem.temp_seen = mem.fr.temp_len;
    if !keep || r != GO {
        mem.forget();
    }
    r
}

/// Executes the signal instruction at `pc` in the interpreter (`TierHost::signal`; `edge` 1 or 2
/// for a `rising_edge` / `falling_edge` native) and returns `GO`, or the pc after a fault. The
/// objects `Mem` found stay valid: these instructions free nothing.
///
/// # Safety
/// As `yavs_rt_step`.
pub unsafe extern "C" fn yavs_rt_signal(
    env: *mut c_void,
    _regs: *mut u64,
    mem: *mut Mem,
    pc: u32,
    edge: u32,
) -> u32 {
    // SAFETY: as in `yavs_rt_step`.
    let envp = env as *mut Env<'_>;
    unsafe { (*mem).sync(&mut *(*envp).host) };
    let r = catch_unwind(AssertUnwindSafe(|| unsafe {
        (*envp).host.signal(&mut (*mem).fr, pc, edge)
    }));
    let mem = unsafe { &mut *mem };
    mem.temp_seen = mem.fr.temp_len;
    match r {
        Ok(Stepped::Next) => GO,
        Ok(Stepped::Fault) => pc,
        Ok(Stepped::Suspend) => TIER_SUSPEND,
        // Only where `signal` stepped another instruction (never for those `Gen` sends).
        Ok(Stepped::Call(_)) => {
            mem.forget();
            TIER_UNWIND
        }
        Err(p) => {
            unsafe { (*envp).panic = Some(p) };
            pc
        }
    }
}

/// `Inst::LCopy` on the `ns` source and `nd` destination plane blocks compiled code found
/// (`Gen::host`): `yavs_interp::copy_bits`, the interpreter's own copy, on stack copies of both
/// (they may be one vector).
///
/// # Safety
/// `dst` and `src` point to `nd` writable and `ns` readable 32-byte plane blocks.
pub unsafe extern "C" fn yavs_rt_lcopy(
    dst: *mut u8,
    nd: u64,
    d0: u64,
    src: *const u8,
    ns: u64,
    s0: u64,
    n: u64,
) {
    let blocks = |b: &[u8], out: &mut [[u64; 4]]| {
        for (o, c) in out.iter_mut().zip(b.as_chunks::<32>().0) {
            *o = std::array::from_fn(|k| u64::from_le_bytes(std::array::from_fn(|j| c[k * 8 + j])));
        }
    };
    let (ns, nd) = (ns.min(LCOPY_BLOCKS) as usize, nd.min(LCOPY_BLOCKS) as usize);
    let mut sw = [[0u64; 4]; LCOPY_BLOCKS as usize];
    let mut dw = [[0u64; 4]; LCOPY_BLOCKS as usize];
    // SAFETY: compiled code checked `ns` readable blocks at `src` and `nd` writable blocks at
    // `dst` (`Gen::host`, both at most `LCOPY_BLOCKS`); each slice is dead before the next
    // one exists.
    unsafe {
        blocks(std::slice::from_raw_parts(src, ns * 32), &mut sw[..ns]);
        blocks(std::slice::from_raw_parts(dst, nd * 32), &mut dw[..nd]);
    }
    yavs_interp::copy_bits(&sw[..ns], &mut dw[..nd], s0, d0, n);
    // SAFETY: as above.
    let d = unsafe { std::slice::from_raw_parts_mut(dst, nd * 32) };
    for (c, w) in d
        .as_chunks_mut::<8>()
        .0
        .iter_mut()
        .zip(dw[..nd].as_flattened())
    {
        *c = w.to_le_bytes();
    }
}

/// The body of `yavs_rt_step`.
///
/// # Safety
/// `env` and `mem` are the live `Env` and `Mem` of the compiled code calling `yavs_rt_step`.
pub unsafe fn step_in(env: *mut Env<'_>, mem: *mut Mem, pc: u32) -> u32 {
    // SAFETY: see `yavs_rt_step`.
    unsafe {
        let stack = (*mem).fr.stack;
        let stepped = (*env).host.step(&mut (*mem).fr, pc);
        // The interpreter's view: the temporary area has the length it shows.
        (*mem).temp_seen = (*mem).fr.temp_len;
        let callee = match stepped {
            Stepped::Next => return GO,
            Stepped::Fault => return pc,
            Stepped::Suspend => return TIER_SUSPEND,
            Stepped::Call(callee) => callee,
        };
        // The callee runs here when it has compiled code, the register stack did not move
        // (the callers' compiled code holds pointers into it) and native stack is left.
        let Some((run, nregs)) = ((*env).lookup)(callee) else {
            return TIER_UNWIND;
        };
        let fr = &(*mem).fr;
        if fr.stack != stack
            || fr.nregs != nregs as u64
            || fr.frame >= 1 << 29
            || stack_here() < (*env).stack_lo
        {
            return TIER_UNWIND;
        }
        // The callee runs on the same `Mem`: the interpreter keeps its view current. The push
        // may have moved the slot stack under the objects found before.
        (*mem).forget();
        let next = run(env as *mut c_void, fr.regs, mem, 0);
        (*mem).sync(&mut *(*env).host);
        if (*env).panic.is_some() || next == TIER_UNWIND || next == TIER_SUSPEND {
            return next;
        }
        let ok = (*env).host.ret(&mut (*mem).fr, next);
        (*mem).temp_seen = (*mem).fr.temp_len;
        if ok { GO } else { TIER_UNWIND }
    }
}

impl Mem {
    /// The host pointer to `n` bytes at address `a` plus `extra` where they lie in the frame's
    /// slots, the temporary area, (reading) the instance data or a cached object.
    fn find(&self, a: u64, extra: u64, n: u64, write: bool) -> Option<*mut u8> {
        let hi = a >> 32;
        // `extra` is below 2^63: no wrap.
        let off = (a & 0xffff_ffff) + extra;
        let fr = &self.fr;
        let (base, len) = if hi == fr.frame_hi {
            (fr.slots, fr.slots_len)
        } else if hi == u64::from(space::TEMP) << 29 {
            (fr.temp, fr.temp_len)
        } else if !write && hi == u64::from(space::INST) << 29 {
            (fr.inst.cast_mut(), fr.inst_len)
        } else {
            let o = self
                .objs
                .iter()
                .find(|o| o.hi == hi && (!write || o.w != 0))?;
            (o.ptr, o.len)
        };
        (n <= len && off <= len - n).then(|| base.wrapping_add(off as usize))
    }
}

/// The host pointer for an access of compiled code (`Gen::host`), or null when the access is
/// the interpreter's (stepped): what `rd`/`wr` would reach without an error.
///
/// # Safety
/// As `yavs_rt_step`.
pub unsafe extern "C" fn yavs_rt_host(
    env: *mut c_void,
    mem: *mut Mem,
    a: u64,
    extra: u64,
    n: u64,
    write: u32,
) -> *mut u8 {
    // SAFETY: as in `yavs_rt_step`.
    let m = unsafe { &*mem };
    if let Some(p) = m.find(a, extra, n, write != 0) {
        return p;
    }
    if yavs_rt_obj(env, mem, a, write) == 0 {
        return std::ptr::null_mut();
    }
    // SAFETY: as above.
    let m = unsafe { &*mem };
    m.find(a, extra, n, write != 0)
        .unwrap_or(std::ptr::null_mut())
}

/// Finds the object `addr` points into for `Mem::find`: 1 when found, 0 when the access is
/// the interpreter's (stepped).
fn yavs_rt_obj(env: *mut c_void, mem: *mut Mem, addr: u64, write: u32) -> u32 {
    // SAFETY: as in `yavs_rt_step`.
    let env = unsafe { &mut *(env as *mut Env<'_>) };
    let mem = unsafe { &mut *mem };
    match catch_unwind(AssertUnwindSafe(|| env.host.object(addr, write != 0))) {
        Ok(Some((p, len))) => {
            mem.objs[mem.obj_next as usize % OBJS] = Obj {
                hi: addr >> 32,
                ptr: p,
                len: len as u64,
                w: u64::from(write),
            };
            mem.obj_next = mem.obj_next.wrapping_add(1);
            1
        }
        Ok(None) => 0,
        Err(p) => {
            env.panic = Some(p);
            0
        }
    }
}

/// Run compiled `f` (generated for `nregs` registers) from `pc` (`Tier::run`); `lookup`
/// finds the compiled code of the functions it calls.
pub fn enter(
    f: RunFn,
    nregs: usize,
    pc: u32,
    host: &mut dyn TierHost,
    lookup: Lookup<'_>,
) -> Option<u32> {
    let mut mem = Mem::of(TierFrame::default());
    host.view(&mut mem.fr);
    mem.temp_seen = mem.fr.temp_len;
    // A frame index beyond the address format is the interpreter's error.
    if mem.fr.nregs != nregs as u64 || mem.fr.frame >= 1 << 29 {
        return None;
    }
    let mut env = Env {
        host,
        lookup,
        stack_lo: stack_here().saturating_sub(NEST_STACK),
        panic: None,
    };
    // SAFETY: `f` was generated for this function with `nregs` registers and only
    // indexes `regs` below that (checked at compile time); it accesses memory only
    // within the bounds in `mem`; `env` and `mem` outlive the call.
    let next = unsafe {
        f(
            &mut env as *mut Env<'_> as *mut c_void,
            mem.fr.regs,
            &mut mem,
            pc,
        )
    };
    mem.sync(env.host);
    if let Some(p) = env.panic {
        resume_unwind(p);
    }
    Some(next)
}

/// A register operand outside the function's register file, if any.
pub fn bad_reg(f: &Func) -> Option<Reg> {
    let n = f.reg_ty.len() as u32;
    f.code
        .iter()
        .flat_map(|i| inline_regs(f, i))
        .find(|r| r.0 >= n)
}

/// The registers an inline instruction touches (the compiled code indexes `regs` with them);
/// `Reg(u32::MAX)` for an operand list outside the function.
pub fn inline_regs(f: &Func, i: &Inst) -> Vec<Reg> {
    let pair = |r: Reg, w: Width| {
        if w == Width::Sig {
            vec![r, Reg(r.0.saturating_add(1))]
        } else {
            vec![r]
        }
    };
    match *i {
        Inst::AddrOf { dst, .. } => vec![dst],
        Inst::Alloc { dst, size } => vec![dst, size],
        Inst::Load { dst, addr, w } => [vec![addr], pair(dst, w)].concat(),
        Inst::Store { addr, src, w } => [vec![addr], pair(src, w)].concat(),
        Inst::Copy { dst, src, bytes } => vec![dst, src, bytes],
        Inst::Pos { dst, dope, idx, .. } => vec![dst, dope, idx],
        Inst::Field {
            dst, base, layout, ..
        } => vec![dst, base, layout],
        Inst::DopeLen { dst, dope, .. } => vec![dst, dope],
        Inst::LGet { dst, base, idx } => vec![dst, base, idx],
        Inst::LCopy {
            dst,
            dst_off,
            src,
            src_off,
            n,
        } => vec![dst, dst_off, src, src_off, n],
        Inst::LSet { base, idx, v } => vec![base, idx, v],
        Inst::SigVal {
            dst, offset, len, ..
        } => vec![dst, Reg(dst.0.saturating_add(1)), offset, len],
        Inst::SubtypeCheck { dope, want, .. } => vec![dope, want],
        Inst::MakeDope { at, parts } => match f.regs(parts) {
            Some(rs) => [&[at], rs].concat(),
            None => vec![Reg(u32::MAX)],
        },
        Inst::Const { dst, .. } | Inst::ConstF { dst, .. } => vec![dst],
        Inst::Move { dst, src } | Inst::IToF { dst, src } => vec![dst, src],
        Inst::Bin { dst, a, b, .. }
        | Inst::FBin { dst, a, b, .. }
        | Inst::Cmp { dst, a, b, .. }
        | Inst::FCmp { dst, a, b, .. }
        | Inst::Bool { dst, a, b, .. } => vec![dst, a, b],
        Inst::Un { dst, a, .. } | Inst::FUn { dst, a, .. } => vec![dst, a],
        Inst::Check { v, lo, hi, .. } | Inst::FCheck { v, lo, hi, .. } => vec![v, lo, hi],
        Inst::BinRange {
            dst, a, b, lo, hi, ..
        } => vec![dst, a, b, lo, hi],
        Inst::Index { dst, base, pos, .. } => vec![dst, base, pos],
        Inst::CondBr { c, .. } => vec![c],
        Inst::Case { v, .. } => vec![v],
        _ => vec![],
    }
}

/// How compiled code runs a native call it does not step.
pub enum NativeKind {
    /// A function of `std_ulogic` codes (`yavs_interp::code_table`): the results by code, the
    /// operand registers and the result register.
    Table(Vec<u8>, Vec<Reg>, Reg),
    /// `rising_edge` (true) or `falling_edge` through `TierHost::signal`.
    Edge(bool),
}

pub fn native_kind(
    prog: &Program,
    f: &Func,
    native: yavs_ir::NativeId,
    args: yavs_ir::Regs,
    dst: Option<Reg>,
) -> Option<NativeKind> {
    let sym = &prog.natives.get(native)?.symbol;
    let (rs, d) = (f.regs(args)?, dst?);
    let nregs = f.reg_ty.len() as u32;
    if let Some(t) = yavs_interp::code_table(sym) {
        // One operand register per `std_ulogic` (no signal operand), all in the frame.
        let fits = t.len() == 9usize.pow(rs.len() as u32)
            && rs.iter().chain([&d]).all(|r| r.0 < nregs)
            && rs
                .iter()
                .all(|r| f.reg_ty.get(r.0 as usize) != Some(&yavs_ir::RegTy::Sig));
        return fits.then(|| NativeKind::Table(t, rs.to_vec(), d));
    }
    yavs_interp::edge_native(sym).map(NativeKind::Edge)
}

/// The address `AddrOf` yields in `space`, without the frame index for `Frame`; `None` where
/// the interpreter computes it (static links, an index beyond the address format).
pub fn addr_const(space: AddrSpace, off: u32) -> Option<u64> {
    let (sp, idx) = match space {
        AddrSpace::Frame => (space::FRAME, 0),
        AddrSpace::Up(_) => return None,
        AddrSpace::Inst => (space::INST, 0),
        AddrSpace::Pkg(p) => (space::PKG, p),
        AddrSpace::Const(c) => (space::CONST, c.0),
        AddrSpace::Shared => (space::SHARED, 0),
    };
    (idx < 1 << 29).then(|| u64::from(sp) << 61 | u64::from(idx) << 32 | u64::from(off))
}

/// Instructions the interpreter executes without freeing or resizing a memory object or
/// pushing a frame: the objects compiled code found stay valid across their step.
pub fn keeps_objects(i: &Inst) -> bool {
    matches!(
        i,
        Inst::Const { .. }
            | Inst::ConstF { .. }
            | Inst::Move { .. }
            | Inst::AddrOf { .. }
            | Inst::CopyConst { .. }
            | Inst::Alloc { .. }
            | Inst::Load { .. }
            | Inst::Store { .. }
            | Inst::Copy { .. }
            | Inst::Fill { .. }
            | Inst::Pos { .. }
            | Inst::Index { .. }
            | Inst::Field { .. }
            | Inst::Bin { .. }
            | Inst::Un { .. }
            | Inst::FBin { .. }
            | Inst::FUn { .. }
            | Inst::Phys { .. }
            | Inst::IToF { .. }
            | Inst::FToI { .. }
            | Inst::FToIRange { .. }
            | Inst::Cmp { .. }
            | Inst::FCmp { .. }
            | Inst::Bool { .. }
            | Inst::LogicOp { .. }
            | Inst::LNum { .. }
            | Inst::LEq { .. }
            | Inst::MemEq { .. }
            | Inst::LGet { .. }
            | Inst::LSet { .. }
            | Inst::LCopy { .. }
            | Inst::Check { .. }
            | Inst::FCheck { .. }
            | Inst::BinRange { .. }
            | Inst::MakeDope { .. }
            | Inst::DopeLen { .. }
            | Inst::LayoutOf { .. }
            | Inst::SubtypeCheck { .. }
            | Inst::Deref { .. }
            | Inst::SigVal { .. }
            | Inst::SigParts { .. }
            | Inst::SigSlice { .. }
            | Inst::SigRead { .. }
            | Inst::SigReadScalar { .. }
            | Inst::SigLastValue { .. }
            | Inst::SigAttr { .. }
            | Inst::Driving { .. }
            | Inst::DrivingValue { .. }
            | Inst::Drive { .. }
            | Inst::DriveSlot { .. }
            | Inst::WaitOn { .. }
            | Inst::WaitFor { .. }
            | Inst::WaitAt { .. }
            | Inst::TimedOut { .. }
            | Inst::Now { .. }
            | Inst::Delta { .. }
            | Inst::Cover { .. }
            | Inst::Stmt
    )
}

/// Instructions after which the interpreter may enter the compiled code: a call it finished
/// (`TIER_UNWIND`), a suspension, the return of a native that handed its call to a VHDL body
/// (`TierHost::step`).
pub fn resumes(i: &Inst) -> bool {
    matches!(
        i,
        Inst::Call { .. }
            | Inst::CallIndirect { .. }
            | Inst::CallProt { .. }
            | Inst::CallNative { .. }
            | Inst::Report { .. }
            | Inst::Suspend { .. }
    )
}

/// A `LayoutOf` result the generator can build: fixed words, or an array whose element
/// layout is fixed, so only the descriptor's lengths are read at run time.
pub enum StaticLayout {
    Words(Vec<u64>),
    Array {
        dims: u8,
        elem: yavs_ir::Layout,
        logic: bool,
    },
}

/// The `LayoutOf` of `ty` as far as it does not depend on the descriptor, as `Ty::layout`
/// computes it; `None` where the interpreter computes it.
pub fn static_layout(prog: &Program, ty: yavs_ir::TyId) -> Option<StaticLayout> {
    use yavs_ir::Ty;
    let (types, consts) = (&prog.types, &prog.consts);
    let words = |l: yavs_ir::Layout| {
        let mut w = vec![l.size, l.align];
        if l.offsets.is_empty() {
            w.extend([l.stride, l.count]);
        } else {
            w.extend(&l.offsets);
        }
        StaticLayout::Words(w)
    };
    if Ty::desc_size(types, ty)? == 0 {
        return Ty::layout(types, consts, ty, &[])
            .ok()
            .map(|(l, _)| words(l));
    }
    let Some(Ty::Array {
        elem,
        elem_desc,
        dims,
    }) = types.get(ty)
    else {
        return None;
    };
    let el = match elem_desc {
        Some(c) => match consts.get(*c)? {
            yavs_ir::ConstData::Bytes(b) => Ty::layout(types, consts, *elem, b).ok()?.0,
            _ => return None,
        },
        None if Ty::desc_size(types, *elem)? == 0 => Ty::layout(types, consts, *elem, &[]).ok()?.0,
        None => return None,
    };
    // A scalar element has its element size (`Ty::elem_size`), as in `Ty::layout`.
    let el = match types.get(*elem).and_then(Ty::elem_size) {
        Some(n) => yavs_ir::Layout {
            size: n,
            align: n,
            ..el
        },
        None => el,
    };
    Some(StaticLayout::Array {
        dims: *dims,
        elem: el,
        logic: Ty::logic_leaf(types, ty).is_some(),
    })
}

/// What the generator knows about a register's value within a basic block.
#[derive(Clone, Copy)]
pub enum Known {
    Int(i64),
    /// The address of byte `.0` of the frame's slots.
    Frame(u64),
    /// The address of byte `.1` of constant `.0`.
    Const(yavs_ir::ConstId, u64),
}

/// Update `known` for what `i` writes.
pub fn track(known: &mut [Option<Known>], i: &Inst) {
    let get = |r: Reg| known.get(r.0 as usize).copied().flatten();
    let new = match *i {
        Inst::Const { dst, v } => Some((dst, Known::Int(v))),
        Inst::AddrOf {
            dst,
            space: AddrSpace::Frame,
            off,
        } => Some((dst, Known::Frame(off.into()))),
        Inst::AddrOf {
            dst,
            space: AddrSpace::Const(c),
            off,
        } => Some((dst, Known::Const(c, off.into()))),
        Inst::Move { dst, src } => get(src).map(|k| (dst, k)),
        // As the interpreter computes it when it is no error.
        Inst::Index {
            dst,
            base,
            pos,
            stride,
        } => {
            let at = |o: u64, p: i64| {
                (p as u64)
                    .checked_mul(stride.into())
                    .and_then(|d| o.checked_add(d))
                    .filter(|&a| a <= u64::from(u32::MAX))
            };
            match (get(base), get(pos)) {
                (Some(Known::Frame(o)), Some(Known::Int(p))) => {
                    at(o, p).map(|a| (dst, Known::Frame(a)))
                }
                (Some(Known::Const(c, o)), Some(Known::Int(p))) => {
                    at(o, p).map(|a| (dst, Known::Const(c, a)))
                }
                _ => None,
            }
        }
        _ => None,
    };
    i.visit(&mut |r, role, _| {
        if role == yavs_ir::Role::Def {
            for k in [r.0, r.0.saturating_add(1)] {
                if let Some(x) = known.get_mut(k as usize) {
                    *x = None;
                }
            }
        }
    });
    if let Some((r, k)) = new
        && let Some(x) = known.get_mut(r.0 as usize)
    {
        *x = Some(k);
    }
}

/// Below this many IR instructions a function without a loop stays interpreted (`too_short`).
pub const SHORT: usize = 64;

/// Why `f` stays interpreted although hot: shorter than `SHORT` IR instructions and without a
/// loop (`loops`), so each entry into compiled code from the interpreter (the frame view, the
/// entry switch, the way back) would cost about what interpreting its body costs: the small
/// helpers a testbench calls millions of times ran slower compiled (#432), and so did the
/// one-statement processes of the kernel design `fan`.
pub fn too_short(prog: &Program, f: &Func) -> Option<String> {
    (!loops(prog, f) && f.code.len() < SHORT).then(|| {
        format!(
            "{} IR instructions without a loop: entering compiled code would cost about as much",
            f.code.len()
        )
    })
}

/// Whether the blocks of `f` form a cycle, not counting a process's way through its
/// suspension (each activation runs its body once).
pub fn loops(prog: &Program, f: &Func) -> bool {
    let blocks: Vec<&[Inst]> = f
        .blocks
        .iter()
        .map(|(_, b)| f.code.get(b.start as usize..b.end as usize).unwrap_or(&[]))
        .collect();
    let waits: Vec<bool> = blocks
        .iter()
        .map(|c| {
            f.kind == yavs_ir::FuncKind::Process
                && c.iter().any(|i| matches!(i, Inst::Suspend { .. }))
        })
        .collect();
    let succ = |b: usize| -> Vec<usize> {
        let mut to = Vec::new();
        for i in blocks[b] {
            match i {
                Inst::Br { to: t } => to.push(t.0 as usize),
                Inst::CondBr { t, f: e, .. } => to.extend([t.0 as usize, e.0 as usize]),
                Inst::Case { table, .. } => {
                    if let Some(t) = prog.cases.get(*table) {
                        to.push(t.default.0 as usize);
                        to.extend(t.arms.iter().map(|a| a.2.0 as usize));
                    }
                }
                _ => {}
            }
        }
        to.retain(|&t| t < blocks.len() && !waits[b] && !waits[t]);
        to
    };
    // Depth-first search for an edge back to a block on the current path.
    let mut state = vec![0u8; blocks.len()]; // 0 new, 1 on the path, 2 done
    for root in 0..blocks.len() {
        if state[root] != 0 {
            continue;
        }
        let mut stack = vec![(root, succ(root), 0)];
        state[root] = 1;
        while let Some((b, next, k)) = stack.last_mut() {
            match next.get(*k).copied() {
                Some(t) => {
                    *k += 1;
                    match state[t] {
                        1 => return true,
                        0 => {
                            state[t] = 1;
                            stack.push((t, succ(t), 0));
                        }
                        _ => {}
                    }
                }
                None => {
                    state[*b] = 2;
                    stack.pop();
                }
            }
        }
    }
    false
}
