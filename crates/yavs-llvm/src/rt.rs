//! The runtime half of the compiled tiers: the frame view (`Mem`), the callbacks compiled code
//! calls (`yavs_rt_*`), `enter`, and the per-instruction facts both code generators share.
//! No code generator here: it builds without LLVM or Cranelift.

// The callbacks are `unsafe extern "C"` functions called only by generated code, which passes the
// `env` and `mem` pointers `enter` made: one safety contract for all of them, stated on `enter`.
#![allow(clippy::missing_safety_doc)]

use std::any::Any;
use std::ffi::c_void;
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

use yavs_interp::{NativeFrame, Stepped, TIER_SUSPEND, TIER_UNWIND, TierFrame, TierHost, space};
use yavs_ir::{AddrSpace, Func, FuncId, FuncKind, Inst, Program, Reg, Width};

pub const STEP: &str = "yavs_rt_step";
pub const HOST: &str = "yavs_rt_host";
pub const SIGNAL: &str = "yavs_rt_signal";
pub const DRIVE: &str = "yavs_rt_drive";
pub const WAIT: &str = "yavs_rt_wait";
pub const LCOPY: &str = "yavs_rt_lcopy";
pub const LAYOUT: &str = "yavs_rt_layout";
pub const FAST: &str = "yavs_rt_fast";
pub const DEOPT: &str = "yavs_rt_deopt";
pub const DEOPT_SLOTS: &str = "yavs_rt_deopt_slots";
pub const DEOPT_END: &str = "yavs_rt_deopt_end";
pub const FAST_ALLOC: u32 = 0;
pub const FAST_DOPE_LEN: u32 = 1;
pub const FAST_SUBTYPE: u32 = 2;
pub const FAST_FAT_ROOT: u32 = 3;
pub const FAST_DEREF: u32 = 4;
pub const MEMEQ: &str = "yavs_rt_memeq";

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
    pub fr: TierFrame,
    pub objs: [Obj; OBJS],
    pub obj_next: u64,
    /// `objs` changed since it was loaded from the process's record (`save_objs`).
    pub dirty: bool,
    /// Some entry of `objs` is not one `forget_end` keeps.
    pub transient: bool,
    /// The temporary area's length the interpreter last gave: no hand-back while unchanged.
    pub temp_seen: u64,
    /// Nonzero while native frames unwind to be materialised (`yavs_rt_deopt`).
    pub ndeopt: u64,
    /// Per function, the address of its native-frame code (`Gen::native_call`), 0 for none.
    pub ntab: *const usize,
    /// Below this native stack address no native callee is entered (`NEST_STACK`).
    pub stack_lo: u64,
}

/// An object in `Mem::objs`: the high half of its addresses (`u64::MAX`: none), start,
/// length and whether it was found for writing.
#[derive(Clone, Copy)]
#[repr(C)]
pub struct Obj {
    pub hi: u64,
    pub ptr: *mut u8,
    pub len: u64,
    pub w: u64,
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
pub const MEM_INST: u64 = std::mem::offset_of!(Mem, fr.inst) as u64;
pub const MEM_INST_LEN: u64 = std::mem::offset_of!(Mem, fr.inst_len) as u64;
pub const MEM_NSIGS: u64 = std::mem::offset_of!(Mem, fr.nsigs) as u64;
pub const MEM_TEMP_ROOTS: u64 = std::mem::offset_of!(Mem, fr.temp_roots) as u64;
pub const MEM_SIGTAB: u64 = std::mem::offset_of!(Mem, fr.sigtab) as u64;
pub const MEM_SIGTAB_LEN: u64 = std::mem::offset_of!(Mem, fr.sigtab_len) as u64;
pub const MEM_KNOWN_SIG: u64 = std::mem::offset_of!(Mem, fr.known_sig) as u64;
pub const MEM_KNOWN_ELEM: u64 = std::mem::offset_of!(Mem, fr.known_elem) as u64;
pub const MEM_KNOWN_UP: u64 = std::mem::offset_of!(Mem, fr.known_up) as u64;
pub const MEM_FRAME_IDX: u64 = std::mem::offset_of!(Mem, fr.frame) as u64;
pub const MEM_MAX_DEPTH: u64 = std::mem::offset_of!(Mem, fr.max_depth) as u64;
pub const MEM_NDEOPT: u64 = std::mem::offset_of!(Mem, ndeopt) as u64;
pub const MEM_NTAB: u64 = std::mem::offset_of!(Mem, ntab) as u64;
pub const MEM_STACK_LO: u64 = std::mem::offset_of!(Mem, stack_lo) as u64;
/// Bytes of a `TierSig` and the offsets of its fields.
pub const SIG_ENT: i64 = std::mem::size_of::<yavs_interp::TierSig>() as i64;
pub const SIG_CODES: i64 = std::mem::offset_of!(yavs_interp::TierSig, codes) as i64;
pub const SIG_LEN: i64 = std::mem::offset_of!(yavs_interp::TierSig, len) as i64;
pub const SIG_SIGNAL: i64 = std::mem::offset_of!(yavs_interp::TierSig, signal) as i64;
pub const SIG_OFF: i64 = std::mem::offset_of!(yavs_interp::TierSig, off) as i64;

impl Mem {
    pub fn of(fr: TierFrame) -> Mem {
        Mem {
            fr,
            objs: [NO_OBJ; OBJS],
            obj_next: 0,
            dirty: false,
            transient: false,
            temp_seen: fr.temp_len,
            ndeopt: 0,
            ntab: std::ptr::null(),
            stack_lo: 0,
        }
    }

    /// Give the temporary area the length compiled code left it.
    pub fn sync(&mut self, host: &mut dyn TierHost) {
        if self.fr.temp_len != self.temp_seen {
            // SAFETY: compiled code only shrinks the length or grows it within the capacity
            // after zeroing the new bytes (`Inst::Alloc`), so they are initialised.
            unsafe { host.temp().set_len(self.fr.temp_len as usize) }
        }
    }

    pub fn load_objs(&mut self, w: &[u64; 17]) {
        const { assert!(size_of::<[Obj; OBJS]>() == size_of::<[u64; 16]>()) };
        // SAFETY: `Obj` is four words (`hi`, the address as a word, `len`, `w`); the record
        // was written by `save_objs` from such entries.
        unsafe {
            std::ptr::copy_nonoverlapping(w.as_ptr(), self.objs.as_mut_ptr().cast::<u64>(), 16);
        }
        self.obj_next = self.objs.iter().take_while(|o| o.hi != u64::MAX).count() as u64;
        self.dirty = false;
        self.transient = false;
    }

    /// Keep the objects `forget_end` left for the next activation.
    pub fn save_objs(&self, slot: &mut Option<Box<yavs_interp::TierState>>) {
        if self.obj_next == 0 {
            if let Some(st) = slot {
                st.used = false;
            }
            return;
        }
        let st = slot.get_or_insert_with(Default::default);
        if !self.dirty && st.used {
            return;
        }
        // SAFETY: as in `load_objs`.
        unsafe {
            std::ptr::copy_nonoverlapping(
                self.objs.as_ptr().cast::<u64>(),
                st.words.as_mut_ptr(),
                16,
            );
        }
        st.words[16] = self.fr.heap_epoch;
        st.used = true;
    }

    /// The end of an activation: forget the objects found, except constants and the process
    /// heap's objects, which stay valid while the heap's epoch does not change.
    #[inline]
    pub fn forget_end(&mut self) {
        if !self.transient {
            return;
        }
        let keep = |hi: u64| {
            let sp = (hi >> 29) as u8;
            sp == space::CONST || sp == space::HEAP
        };
        let objs = std::mem::replace(&mut self.objs, [NO_OBJ; OBJS]);
        self.obj_next = 0;
        for o in objs {
            if o.hi != u64::MAX && keep(o.hi) {
                self.objs[self.obj_next as usize] = o;
                self.obj_next += 1;
            }
        }
        self.dirty = true;
        self.transient = false;
    }

    /// Forget the objects found before a step, except constants (they never move or change).
    pub fn forget(&mut self) {
        if self.obj_next == 0 {
            return;
        }
        let objs = std::mem::replace(&mut self.objs, [NO_OBJ; OBJS]);
        self.obj_next = 0;
        for o in objs {
            if o.hi != u64::MAX && o.hi >> 29 == u64::from(space::CONST) {
                self.objs[self.obj_next as usize] = o;
                self.obj_next += 1;
            }
        }
        self.dirty = true;
        self.transient = false;
    }
}

/// What a running compiled function reaches through `env`.
pub struct Env<'a> {
    pub host: &'a mut dyn TierHost,
    pub lookup: Lookup<'a>,
    /// Below this native stack address no compiled callee is entered (`NEST_STACK`).
    pub stack_lo: usize,
    pub panic: Option<Box<dyn Any + Send>>,
    /// Native frames recorded on their way to `TierHost::materialise`, innermost first.
    pub pending: Vec<NativeFrame>,
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
    // SAFETY: the pointers are the ones compiled code was given.
    unsafe { stepped(envp, mem, pc, r) }
}

/// What a `TierHost::signal`-like call returns to compiled code: `GO`, or the pc after a
/// fault, `TIER_SUSPEND`, `TIER_UNWIND`.
pub unsafe fn stepped(
    envp: *mut Env<'_>,
    mem: *mut Mem,
    pc: u32,
    r: Result<Stepped, Box<dyn Any + Send>>,
) -> u32 {
    // SAFETY: as in `yavs_rt_step`.
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

/// Record a native frame of `func` at `pc` for materialisation (`docs/decisions.md` "Compiled
/// frames"): its registers go to the returned buffer, its slots to `yavs_rt_deopt_slots`.
pub unsafe extern "C" fn yavs_rt_deopt(
    env: *mut c_void,
    mem: *mut Mem,
    func: u32,
    pc: u32,
    temp_base: u64,
    nregs: u64,
    slen: u64,
) -> *mut u64 {
    // SAFETY: as in `yavs_rt_step`.
    let (env, mem) = unsafe { (&mut *(env as *mut Env<'_>), &mut *mem) };
    mem.ndeopt = 1;
    env.pending.push(NativeFrame {
        func: FuncId(func),
        pc,
        regs: vec![0; nregs as usize],
        slots: vec![0; slen as usize],
        temp_base,
    });
    let last = env.pending.len() - 1;
    env.pending[last].regs.as_mut_ptr()
}

/// The slot buffer of the frame `yavs_rt_deopt` recorded last.
pub unsafe extern "C" fn yavs_rt_deopt_slots(env: *mut c_void) -> *mut u8 {
    // SAFETY: as in `yavs_rt_step`; called right after `yavs_rt_deopt`.
    let env = unsafe { &mut *(env as *mut Env<'_>) };
    let last = env.pending.len() - 1;
    env.pending[last].slots.as_mut_ptr()
}

/// A frame-mode function's native callee unwound for materialisation: push the recorded
/// frames on the top frame, which waits at its call at `pc`. Returns `TIER_UNWIND` (the
/// interpreter goes on with the top frame), or `pc` after a fault.
pub unsafe extern "C" fn yavs_rt_deopt_end(env: *mut c_void, mem: *mut Mem, pc: u32) -> u32 {
    // SAFETY: as in `yavs_rt_step`.
    let (env, mem) = unsafe { (&mut *(env as *mut Env<'_>), &mut *mem) };
    mem.sync(&mut *env.host);
    let mut frames = std::mem::take(&mut env.pending);
    frames.reverse();
    let r = catch_unwind(AssertUnwindSafe(|| {
        env.host.materialise(&mut mem.fr, pc, &frames)
    }));
    mem.forget();
    mem.temp_seen = mem.fr.temp_len;
    mem.ndeopt = 0;
    match r {
        Ok(Stepped::Next) => TIER_UNWIND,
        Ok(_) => pc,
        Err(p) => {
            env.panic = Some(p);
            pc
        }
    }
}

/// `TierHost::drive_block` for the `Drive` at `pc`: `wave` (40 readable bytes, checked by the
/// caller) holds the time and the first block of planes of its one waveform element.
pub unsafe extern "C" fn yavs_rt_drive(
    env: *mut c_void,
    _regs: *mut u64,
    mem: *mut Mem,
    pc: u32,
    driver: u32,
    lo: u64,
    len: u64,
    wave: *const u8,
    inertial: u32,
) -> u32 {
    // SAFETY: as in `yavs_rt_step`; the caller checked 40 readable bytes at `wave`.
    let envp = env as *mut Env<'_>;
    let word = |k: usize| unsafe { std::ptr::read_unaligned(wave.add(k * 8).cast::<u64>()) };
    let (delay, block) = (word(0) as i64, [word(1), word(2), word(3), word(4)]);
    // The usual case, taken by the host without a frame view or a result.
    if unsafe { (*envp).host.drive_now(driver, [lo, len], delay, block) } {
        return GO;
    }
    unsafe { (*mem).sync(&mut *(*envp).host) };
    let r = catch_unwind(AssertUnwindSafe(|| unsafe {
        (*envp).host.drive_block(
            &mut (*mem).fr,
            pc,
            driver,
            [lo, len],
            delay,
            block,
            inertial != 0,
        )
    }));
    // SAFETY: the pointers are the ones compiled code was given.
    unsafe { stepped(envp, mem, pc, r) }
}

/// `TierHost::wait_suspend` for the `WaitOn` at `pc` and the `Suspend` after it.
pub unsafe extern "C" fn yavs_rt_wait(
    env: *mut c_void,
    _regs: *mut u64,
    mem: *mut Mem,
    pc: u32,
    lo: u64,
    len: u64,
) -> u32 {
    // SAFETY: as in `yavs_rt_step`.
    let envp = env as *mut Env<'_>;
    if unsafe { (*envp).host.wait_again(pc, [lo, len]) } {
        return TIER_SUSPEND;
    }
    unsafe { (*mem).sync(&mut *(*envp).host) };
    let r = catch_unwind(AssertUnwindSafe(|| unsafe {
        (*envp).host.wait_suspend(&mut (*mem).fr, pc, [lo, len])
    }));
    // SAFETY: the pointers are the ones compiled code was given.
    unsafe { stepped(envp, mem, pc, r) }
}

/// `Inst::LCopy` on the `ns` source and `nd` destination plane blocks compiled code found
/// (`Gen::host`): `yavs_interp::copy_bits`, the interpreter's own copy, on stack copies of both
/// (they may be one vector).
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

/// `Inst::MemEq` on the `n` bytes compiled code found at `a` and `b` (`Gen::host`): 1 when equal.
pub unsafe extern "C" fn yavs_rt_memeq(a: *const u8, b: *const u8, n: u64) -> u64 {
    // SAFETY: compiled code checked `n` readable bytes at each.
    unsafe {
        u64::from(
            std::slice::from_raw_parts(a, n as usize) == std::slice::from_raw_parts(b, n as usize),
        )
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
    pub fn find(&self, a: u64, extra: u64, n: u64, write: bool) -> Option<*mut u8> {
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
    // SAFETY: the pointers are the ones compiled code was given.
    if unsafe { yavs_rt_obj(env, mem, a, write) } == 0 {
        return std::ptr::null_mut();
    }
    // SAFETY: as above.
    let m = unsafe { &*mem };
    m.find(a, extra, n, write != 0)
        .unwrap_or(std::ptr::null_mut())
}

/// `Inst::LayoutOf` of an array whose element layout is fixed (`StaticLayout::Array`): reads
/// the `dims` dimensions of the descriptor at `desc`, builds the four layout words in temporary
/// memory and puts its address in register `dst`; 1 when done, 0 where `Ty::layout` or the
/// temporary area would fail (the interpreter then executes the instruction, with its error).
pub unsafe extern "C" fn yavs_rt_layout(
    env: *mut c_void,
    regs: *mut u64,
    mem: *mut Mem,
    desc: u64,
    dst: u32,
    dims: u32,
    stride: u64,
    align: u64,
    el_count: u64,
    logic: u32,
) -> u32 {
    // SAFETY: the pointers are the ones compiled code was given.
    let p = unsafe { yavs_rt_host(env, mem, desc, 0, u64::from(dims) * 24, 0) };
    if p.is_null() {
        return 0;
    }
    let word = |k: usize| -> i64 {
        // SAFETY: `yavs_rt_host` checked `dims * 24` readable bytes at `p`.
        unsafe { std::ptr::read_unaligned(p.add(k * 8).cast::<i64>()) }
    };
    let mut count: u64 = 1;
    for k in 0..dims as usize {
        let (l, r, dir) = (word(k * 3), word(k * 3 + 1), word(k * 3 + 2));
        if dir as u64 > 1 {
            return 0;
        }
        let (lo, hi) = if dir == 1 { (r, l) } else { (l, r) };
        let n = hi.wrapping_sub(lo).wrapping_add(1) as u64;
        let len = if hi < lo {
            0
        } else if n == 0 {
            return 0;
        } else {
            n
        };
        let Some(c) = count.checked_mul(len) else {
            return 0;
        };
        count = c;
    }
    let words = if logic != 0 {
        let Some(n) = count.checked_mul(el_count) else {
            return 0;
        };
        [n.div_ceil(64) * 32, 8, 0, n]
    } else {
        let Some(size) = stride.checked_mul(count) else {
            return 0;
        };
        [size, align, stride, count]
    };
    // SAFETY: as in `yavs_rt_step`; the bytes written lie within the temporary area's capacity.
    let m = unsafe { &mut *mem };
    let len = m.fr.temp_len;
    let off = (len + 7) & !7;
    let end = off + 32;
    if end > 0xffff_ffff || end > m.fr.temp_cap {
        return 0;
    }
    // SAFETY: `temp` has `temp_cap >= end` bytes; `regs` has the function's registers
    // (`bad_reg` checked `dst`).
    unsafe {
        std::ptr::write_bytes(m.fr.temp.add(len as usize), 0, (end - len) as usize);
        for (k, w) in words.iter().enumerate() {
            std::ptr::write_unaligned(m.fr.temp.add(off as usize + k * 8).cast::<u64>(), *w);
        }
        *regs.add(dst as usize) = u64::from(space::TEMP) << 61 | off;
    }
    m.fr.temp_len = end;
    1
}

/// The length of dimension `k` of the descriptor at `dope` as `yavs_ir::dim_len`, `None`
/// where that is not defined or the memory is the interpreter's.
pub unsafe fn dim_at(env: *mut c_void, mem: *mut Mem, dope: u64, k: u64) -> Option<u64> {
    // SAFETY: the pointers are the ones compiled code was given.
    let p = unsafe { yavs_rt_host(env, mem, dope, k.checked_mul(24)?, 24, 0) };
    if p.is_null() {
        return None;
    }
    // SAFETY: `yavs_rt_host` checked 24 readable bytes at `p`.
    let w = |i: usize| unsafe { std::ptr::read_unaligned(p.add(i * 8).cast::<i64>()) };
    yavs_ir::dim_len(w(0), w(1), w(2) == 1)
}

/// Instructions whose inline form was large and no faster than this call: `FAST_ALLOC`
/// (`dst` = `Alloc`'s result register, `a` the size), `FAST_DOPE_LEN` (`dope` `a`, dimension
/// `b`, result register `dst`), `FAST_SUBTYPE` (`dst` dimensions of `a` against `b`),
/// `FAST_FAT_ROOT` (the fat pair at `a` points into no heap). 1 when
/// done, 0 where the interpreter must execute the instruction (its error, its memory).
pub unsafe extern "C" fn yavs_rt_fast(
    env: *mut c_void,
    regs: *mut u64,
    mem: *mut Mem,
    op: u32,
    dst: u32,
    a: u64,
    b: u64,
) -> u32 {
    match op {
        FAST_ALLOC => {
            // SAFETY: as in `yavs_rt_step`.
            let m = unsafe { &mut *mem };
            let len = m.fr.temp_len;
            let off = (len + 7) & !7;
            let Some(end) = off.checked_add(a) else {
                return 0;
            };
            if end > 0xffff_ffff || end > m.fr.temp_cap {
                return 0;
            }
            // SAFETY: `temp` has `temp_cap >= end` bytes; `regs` holds register `dst`.
            unsafe {
                std::ptr::write_bytes(m.fr.temp.add(len as usize), 0, (end - len) as usize);
                *regs.add(dst as usize) = u64::from(space::TEMP) << 61 | off;
            }
            m.fr.temp_len = end;
            1
        }
        // SAFETY: the pointers are the ones compiled code was given.
        FAST_DOPE_LEN => match unsafe { dim_at(env, mem, a, b) } {
            // Lengths above i64::MAX are the interpreter's error.
            Some(n) if n <= i64::MAX as u64 => {
                // SAFETY: `regs` holds register `dst`.
                unsafe { *regs.add(dst as usize) = n };
                1
            }
            _ => 0,
        },
        FAST_FAT_ROOT => {
            // SAFETY: as in `yavs_rt_step`.
            let m = unsafe { &*mem };
            let Some(p) = m.find(a, 0, 16, false) else {
                return 0;
            };
            // 1 when neither word points into a heap (HEAP 4, SHARED_HEAP 7): nothing to record.
            let heap = |k: usize| {
                // SAFETY: `find` checked 16 readable bytes at `p`.
                let w = unsafe { std::ptr::read_unaligned(p.add(k * 8).cast::<u64>()) };
                matches!(w >> 61, 4 | 7)
            };
            u32::from(!heap(0) && !heap(1))
        }
        FAST_DEREF => {
            // SAFETY: as in `yavs_rt_step`.
            let env = unsafe { &mut *(env as *mut Env<'_>) };
            match catch_unwind(AssertUnwindSafe(|| env.host.deref(a))) {
                Ok(Some(addr)) => {
                    // SAFETY: `regs` holds register `dst`.
                    unsafe { *regs.add(dst as usize) = addr };
                    1
                }
                Ok(None) => 0,
                Err(p) => {
                    env.panic = Some(p);
                    0
                }
            }
        }
        FAST_SUBTYPE => {
            for k in 0..u64::from(dst) {
                // SAFETY: as above.
                match (unsafe { dim_at(env, mem, a, k) }, unsafe {
                    dim_at(env, mem, b, k)
                }) {
                    (Some(x), Some(y)) if x == y => {}
                    _ => return 0,
                }
            }
            1
        }
        _ => 0,
    }
}

/// Finds the object `addr` points into for `Mem::find`: 1 when found, 0 when the access is
/// the interpreter's (stepped).
pub unsafe fn yavs_rt_obj(env: *mut c_void, mem: *mut Mem, addr: u64, write: u32) -> u32 {
    // SAFETY: as in `yavs_rt_step`.
    let env = unsafe { &mut *(env as *mut Env<'_>) };
    let mem = unsafe { &mut *mem };
    match catch_unwind(AssertUnwindSafe(|| env.host.object(addr, write != 0))) {
        Ok(Some((p, len))) => {
            let hi = addr >> 32;
            // The same object found for the other direction takes the same entry.
            let at = match mem.objs.iter().position(|o| o.hi == hi) {
                Some(k) => k,
                None => {
                    mem.obj_next = mem.obj_next.wrapping_add(1);
                    (mem.obj_next as usize - 1) % OBJS
                }
            };
            mem.objs[at] = Obj {
                hi,
                ptr: p,
                len: len as u64,
                w: u64::from(write),
            };
            let sp = (hi >> 29) as u8;
            mem.transient |= sp != space::CONST && sp != space::HEAP;
            mem.dirty = true;
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
    ntab: *const usize,
    state: Option<&mut Option<Box<yavs_interp::TierState>>>,
) -> Option<u32> {
    let mut mem = Mem::of(TierFrame::default());
    if !host.view_again(&mut mem.fr) {
        host.view(&mut mem.fr);
    }
    mem.temp_seen = mem.fr.temp_len;
    // The objects found at an earlier activation are good while the heap did not change.
    if let Some(Some(st)) = state.as_deref()
        && st.used
        && st.words[16] == mem.fr.heap_epoch
    {
        mem.load_objs(&st.words);
        // The heap objects it may write count as written in this activation.
        for o in mem.objs {
            if o.hi != u64::MAX && o.w != 0 && (o.hi >> 29) as u8 == space::HEAP {
                host.wrote(o.hi << 32);
            }
        }
    }
    // A frame index beyond the address format is the interpreter's error.
    if mem.fr.nregs != nregs as u64 || mem.fr.frame >= 1 << 29 {
        return None;
    }
    let mut env = Env {
        host,
        lookup,
        stack_lo: stack_here().saturating_sub(NEST_STACK),
        panic: None,
        pending: Vec::new(),
    };
    mem.ntab = ntab;
    mem.stack_lo = env.stack_lo as u64;
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
    mem.forget_end();
    if let Some(slot) = state {
        mem.save_objs(slot);
    }
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
        Inst::LayoutOf { dst, desc, .. } => vec![dst, desc],
        Inst::Load { dst, addr, w } => [vec![addr], pair(dst, w)].concat(),
        Inst::Store { addr, src, w } => [vec![addr], pair(src, w)].concat(),
        Inst::Copy { dst, src, bytes } => vec![dst, src, bytes],
        Inst::Pos { dst, dope, idx, .. } => vec![dst, dope, idx],
        Inst::Field {
            dst, base, layout, ..
        } => vec![dst, base, layout],
        Inst::DopeLen { dst, dope, .. } => vec![dst, dope],
        Inst::Deref { dst, handle, .. } => vec![dst, handle],
        Inst::MemEq { dst, a, b, n } => vec![dst, a, b, n],
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
        Inst::Select { dst, c, a, b } => vec![dst, c, a, b],
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
            | Inst::Select { .. }
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
            | Inst::Cat { .. }
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
    /// The address of byte `.0` of the instance data.
    Inst(u64),
    /// An address in the temporary area (`Alloc`, or an index into one).
    Temp,
}

/// The registers `f` defines exactly once, by a `Const` or `ConstF` (its bits), and their values.
pub fn fixed_consts(f: &Func) -> Vec<Option<i64>> {
    let n = f.reg_ty.len();
    let mut defs = vec![0u32; n + 1];
    let mut val = vec![None; n + 1];
    for i in &f.code {
        i.visit(&mut |r, role, ty| {
            if role == yavs_ir::Role::Def {
                let w = if ty == Some(yavs_ir::RegTy::Sig) {
                    2
                } else {
                    1
                };
                for d in &mut defs[r.0 as usize..(r.0 as usize + w).min(n + 1)] {
                    *d += 1;
                }
            }
        });
        if let Inst::Const { dst, .. } | Inst::ConstF { dst, .. } = *i
            && let Some(x) = val.get_mut(dst.0 as usize)
        {
            *x = Some(match *i {
                Inst::ConstF { v, .. } => v.to_bits() as i64,
                Inst::Const { v, .. } => v,
                _ => unreachable!(),
            });
        }
    }
    (0..n)
        .map(|r| if defs[r] == 1 { val[r] } else { None })
        .collect()
}

/// Update `known` for what `i` writes.
pub fn track(known: &mut [Option<Known>], fixed: &[Option<i64>], i: &Inst) {
    let get = |r: Reg| known.get(r.0 as usize).copied().flatten();
    // Before the definitions clear what they overwrite.
    let len_known = match *i {
        Inst::SigVal { len, .. } => match get(len) {
            Some(k @ Known::Int(_)) => Some(k),
            _ => fixed.get(len.0 as usize).copied().flatten().map(Known::Int),
        },
        _ => None,
    };
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
        Inst::AddrOf {
            dst,
            space: AddrSpace::Inst,
            off,
        } => Some((dst, Known::Inst(off.into()))),
        Inst::Alloc { dst, .. } => Some((dst, Known::Temp)),
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
                (Some(Known::Inst(o)), Some(Known::Int(p))) => {
                    at(o, p).map(|a| (dst, Known::Inst(a)))
                }
                // An offset past 4 GiB leaves the block through the instruction's own exit.
                (Some(Known::Temp), _) => Some((dst, Known::Temp)),
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
    // The length word of a signal reference is the `len` it was made with.
    if let Inst::SigVal { dst, .. } = *i
        && let Some(Known::Int(l)) = len_known
        && let Some(x) = known.get_mut(dst.0 as usize + 1)
    {
        *x = Some(Known::Int(l));
    }
}

/// Below this many IR instructions a function without a loop stays interpreted (`too_short`).
pub const SHORT: usize = 64;

/// Why `f` stays interpreted although hot: shorter than `SHORT` IR instructions and without a
/// jump back, so each entry into compiled code from the interpreter (the frame view, the
/// entry switch, the way back) would cost about what interpreting its body costs: the small
/// helpers a testbench calls millions of times ran slower compiled (#432).
pub fn too_short(prog: &yavs_ir::Program, f: &yavs_ir::Func) -> Option<String> {
    (f.code.len() < SHORT && !loops(prog, f)).then(|| {
        format!(
            "{} IR instructions without a loop: entering compiled code would cost about as much",
            f.code.len()
        )
    })
}

/// The backward jumps of `f` as ranges of instructions (a jump back to a block start and the jump).
pub fn back_edges(prog: &yavs_ir::Program, f: &yavs_ir::Func) -> Vec<std::ops::Range<usize>> {
    use yavs_ir::Inst;
    let start = |b: yavs_ir::BlockId| f.blocks.get(b).map_or(0, |b| b.start as usize);
    // A process's way back through its suspension is no loop (as in `loops`).
    let mut waits = vec![false; f.code.len()];
    if f.kind == FuncKind::Process {
        for (_, b) in f.blocks.iter() {
            let r = b.start as usize..(b.end as usize).min(f.code.len());
            if f.code[r.clone()]
                .iter()
                .any(|i| matches!(i, Inst::Suspend { .. }))
            {
                waits[r].fill(true);
            }
        }
    }
    let mut out = Vec::new();
    for (pc, i) in f.code.iter().enumerate() {
        if waits[pc] {
            continue;
        }
        let targets: Vec<usize> = match i {
            Inst::Br { to } => vec![start(*to)],
            Inst::CondBr { t, f: e, .. } => vec![start(*t), start(*e)],
            Inst::Case { table, .. } => prog.cases.get(*table).map_or(Vec::new(), |t| {
                std::iter::once(start(t.default))
                    .chain(t.arms.iter().map(|a| start(a.2)))
                    .collect()
            }),
            _ => Vec::new(),
        };
        out.extend(targets.into_iter().filter(|&t| t <= pc).map(|t| t..pc + 1));
    }
    out
}

/// Whether the blocks of `f` form a cycle, not counting a process's way through its
/// suspension (each activation runs its body once).
pub fn loops(prog: &yavs_ir::Program, f: &yavs_ir::Func) -> bool {
    use yavs_ir::Inst;
    let blocks: Vec<&[Inst]> = f
        .blocks
        .iter()
        .map(|(_, b)| f.code.get(b.start as usize..b.end as usize).unwrap_or(&[]))
        .collect();
    let waits: Vec<bool> = blocks
        .iter()
        .map(|c| f.kind == FuncKind::Process && c.iter().any(|i| matches!(i, Inst::Suspend { .. })))
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
    // Blocks the optimiser left unreachable (a self-jump) are no loop of the process.
    let mut reach = vec![false; blocks.len()];
    let mut work = vec![0usize];
    while let Some(b) = work.pop() {
        if b >= blocks.len() || std::mem::replace(&mut reach[b], true) {
            continue;
        }
        for i in blocks[b] {
            match i {
                Inst::Br { to } => work.push(to.0 as usize),
                Inst::CondBr { t, f: e, .. } => work.extend([t.0 as usize, e.0 as usize]),
                Inst::Case { table, .. } => {
                    if let Some(t) = prog.cases.get(*table) {
                        work.push(t.default.0 as usize);
                        work.extend(t.arms.iter().map(|a| a.2.0 as usize));
                    }
                }
                _ => {}
            }
        }
    }
    // Depth-first search for an edge back to a block on the current path.
    let mut state = vec![0u8; blocks.len()]; // 0 new, 1 on the path, 2 done
    for root in 0..blocks.len() {
        if state[root] != 0 || !reach[root] {
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

/// A code generator for the first compiled level of `AutoTier` (`--tier=auto3`): it translates
/// one function to the frame-mode contract of this module (`RunFn`, the function's register
/// count) and keeps the code alive until it is dropped.
pub trait FirstLevel: Send {
    fn compile(&mut self, prog: &Program, id: FuncId, f: &Func) -> Result<(RunFn, usize), String>;
}
