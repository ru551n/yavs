//! `yavs-ir` to LLVM: the compiled tier (ir.md 5, `docs/decisions.md` "LLVM tier").
//!
//! One LLVM function per IR function, `fn(env, regs, mem, pc) -> u32`, working on the
//! registers of the interpreter frame and, through [`Mem`], on the frame's slots, the
//! temporary area and the instance data (the interpreter's [`TierFrame`] view). It enters at
//! `pc` (a switch over the block starts and the instructions after a call or a suspension),
//! runs register arithmetic, compares, checks, branches, loads, stores, copies, temporary
//! allocation, descriptor and signal-reference arithmetic itself, and executes every other
//! instruction in place through the `yavs_rt_step` callback (the interpreter's own code):
//! calls, natives, reports, signal operations and the slow paths. A call whose callee has
//! compiled code runs that directly on the frame the interpreter pushed, and goes on when it
//! returns (#336). It returns at a return or a suspension, or when an inline instruction
//! fails (re-executed by the interpreter, so messages are identical): compiled code never
//! reports an error itself (rule 5).

use std::any::Any;
use std::ffi::{CString, c_void};
use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};
use std::sync::Mutex;

use llvm_sys::analysis::{LLVMVerifierFailureAction, LLVMVerifyFunction};
use llvm_sys::core::*;
use llvm_sys::prelude::*;
use llvm_sys::{LLVMIntPredicate as IP, LLVMRealPredicate as RP};
use yavs_interp::{
    NativeCall, NativeFrame, Stepped, TIER_SUSPEND, TIER_UNWIND, Tier, TierFrame, TierHost, space,
};
use yavs_ir::{
    AddrSpace, BoolOp, CheckKind, CmpOp, FOp, FUnOp, Func, FuncId, FuncKind, Inst, IntOp, IntUnOp,
    Program, Reg, Width,
};

use crate::{Error, Jit, Module, OptLevel};

const STEP: &str = "yavs_rt_step";
const HOST: &str = "yavs_rt_host";
const SIGNAL: &str = "yavs_rt_signal";
const DRIVE: &str = "yavs_rt_drive";
const WAIT: &str = "yavs_rt_wait";
const LCOPY: &str = "yavs_rt_lcopy";
const LAYOUT: &str = "yavs_rt_layout";
const FAST: &str = "yavs_rt_fast";
const DEOPT: &str = "yavs_rt_deopt";
const DEOPT_SLOTS: &str = "yavs_rt_deopt_slots";
const DEOPT_END: &str = "yavs_rt_deopt_end";
const FAST_ALLOC: u32 = 0;
const FAST_DOPE_LEN: u32 = 1;
const FAST_SUBTYPE: u32 = 2;
const FAST_FAT_ROOT: u32 = 3;
const FAST_DEREF: u32 = 4;
const MEMEQ: &str = "yavs_rt_memeq";
const NATIVE: &str = "yavs_rt_native";
const TROOT: &str = "yavs_rt_troot";
const ENDSTMT: &str = "yavs_rt_endstmt";
const XSTEP: &str = "yavs_rt_xstep";

/// The most plane blocks an inline `LCopy` reads or writes (`yavs_rt_lcopy`'s stack copy).
const LCOPY_BLOCKS: u64 = 8;

pub(crate) type RunFn = unsafe extern "C" fn(*mut c_void, *mut u64, *mut Mem, u32) -> u32;

/// The compiled code of a function and its register count, if it has some.
pub(crate) type Lookup<'a> = &'a dyn Fn(FuncId) -> Option<(RunFn, usize)>;

/// `yavs_rt_step` returns it when compiled code goes on with the next instruction; anything
/// else is what the compiled function returns at once (a pc, `TIER_UNWIND`, `TIER_SUSPEND`).
const GO: u32 = u32::MAX;

/// Native stack compiled code may use for calls between compiled functions (`yavs_rt_step`):
/// deeper calls are left to the interpreter, whose frames are not on the native stack.
/// Simulation threads have 2 MiB.
const NEST_STACK: usize = 256 << 10;

/// Set in the pc compiled code passes `yavs_rt_step` when the instruction cannot free or
/// resize a memory object (`keeps_objects`): the objects `Mem` found stay valid.
const KEEP: u32 = 1 << 31;

/// What compiled code reaches through its `mem` argument (field offsets: `MEM_*`): the
/// frame view the interpreter keeps current ([`TierFrame`]; compiled code keeps the
/// temporary area's length in it and hands it back before every step and at the end), and
/// the last objects `yavs_rt_obj` found, valid until a step (`obj_next` is the entry it
/// replaces next).
#[repr(C)]
pub(crate) struct Mem {
    fr: TierFrame,
    objs: [Obj; OBJS],
    obj_next: u64,
    /// `objs` changed since it was loaded from the process's record (`save_objs`).
    dirty: bool,
    /// Some entry of `objs` is not one `forget_end` keeps.
    transient: bool,
    /// The temporary area's length the interpreter last gave: no hand-back while unchanged.
    temp_seen: u64,
    /// Nonzero while native frames unwind to be materialised (`yavs_rt_deopt`).
    ndeopt: u64,
    /// Per function, the address of its native-frame code (`Gen::native_call`), 0 for none.
    ntab: *const usize,
    /// Below this native stack address no native callee is entered (`NEST_STACK`).
    stack_lo: u64,
    /// Per frame index, the slots of the native frame living there (`NSlot`): a native callee
    /// reaches the frames of its native callers through it.
    nslots: *mut NSlot,
}

/// The slots of a native frame, registered at its entry in `Mem::nslots`.
#[derive(Clone, Copy)]
#[repr(C)]
struct NSlot {
    ptr: *mut u8,
    len: u64,
}

/// The slots of the native frames in the table of `Mem::nslots`, for the machine to reach.
struct TabSlots(*mut NSlot);

// SAFETY: the table is the thread's own (`NSLOTS`) and the value lives only while compiled code
// of that thread calls into the interpreter.
unsafe impl Send for TabSlots {}

impl yavs_interp::NativeSlots for TabSlots {
    fn slots(&self, idx: usize) -> Option<&[u8]> {
        // SAFETY: the machine asks for frame indices of live native frames, whose entries
        // `Gen::func` wrote at their entry; each is a distinct `alloca` of its frame.
        unsafe {
            let e = *self.0.add(idx);
            Some(std::slice::from_raw_parts(e.ptr, e.len as usize))
        }
    }

    fn slots_mut(&mut self, idx: usize) -> Option<&mut [u8]> {
        // SAFETY: as `slots`.
        unsafe {
            let e = *self.0.add(idx);
            Some(std::slice::from_raw_parts_mut(e.ptr, e.len as usize))
        }
    }
}

thread_local! {
    /// The table behind `Mem::nslots` of this simulation thread, grown to the call depth limit.
    static NSLOTS: std::cell::RefCell<Vec<NSlot>> = const { std::cell::RefCell::new(Vec::new()) };
    /// `NSLOTS`' address and length, read without a borrow at every entry (`nslots`).
    static NSLOTS_AT: std::cell::Cell<(*mut NSlot, usize)> =
        const { std::cell::Cell::new((std::ptr::null_mut(), 0)) };
}

/// This thread's `NSLOTS` table with at least `depth` entries.
#[inline(always)]
fn nslots(depth: usize) -> *mut NSlot {
    match NSLOTS_AT.get() {
        (p, n) if n >= depth => p,
        _ => nslots_grow(depth),
    }
}

#[inline(never)]
fn nslots_grow(depth: usize) -> *mut NSlot {
    let at = NSLOTS.with(|t| {
        let mut t = t.borrow_mut();
        if t.len() < depth {
            t.resize(
                depth,
                NSlot {
                    ptr: std::ptr::null_mut(),
                    len: 0,
                },
            );
        }
        (t.as_mut_ptr(), t.len())
    });
    NSLOTS_AT.set(at);
    at.0
}

/// An object in `Mem::objs`: the high half of its addresses (`u64::MAX`: none), start,
/// length and whether it was found for writing.
#[derive(Clone, Copy)]
#[repr(C)]
struct Obj {
    hi: u64,
    ptr: *mut u8,
    len: u64,
    w: u64,
}

const OBJS: usize = 4;

const NO_OBJ: Obj = Obj {
    hi: u64::MAX,
    ptr: std::ptr::null_mut(),
    len: 0,
    w: 0,
};

const MEM_SLOTS: u64 = std::mem::offset_of!(Mem, fr.slots) as u64;
const MEM_SLOTS_LEN: u64 = std::mem::offset_of!(Mem, fr.slots_len) as u64;
const MEM_FRAME: u64 = std::mem::offset_of!(Mem, fr.frame_hi) as u64;
const MEM_TEMP: u64 = std::mem::offset_of!(Mem, fr.temp) as u64;
const MEM_TEMP_LEN: u64 = std::mem::offset_of!(Mem, fr.temp_len) as u64;
const MEM_TEMP_CAP: u64 = std::mem::offset_of!(Mem, fr.temp_cap) as u64;
const MEM_TEMP_BASE: u64 = std::mem::offset_of!(Mem, fr.temp_base) as u64;
const MEM_UP: u64 = std::mem::offset_of!(Mem, fr.up) as u64;
const MEM_INST: u64 = std::mem::offset_of!(Mem, fr.inst) as u64;
const MEM_INST_LEN: u64 = std::mem::offset_of!(Mem, fr.inst_len) as u64;
const MEM_NSIGS: u64 = std::mem::offset_of!(Mem, fr.nsigs) as u64;
const MEM_TEMP_ROOTS: u64 = std::mem::offset_of!(Mem, fr.temp_roots) as u64;
const MEM_SIGTAB: u64 = std::mem::offset_of!(Mem, fr.sigtab) as u64;
const MEM_SIGTAB_LEN: u64 = std::mem::offset_of!(Mem, fr.sigtab_len) as u64;
const MEM_KNOWN_SIG: u64 = std::mem::offset_of!(Mem, fr.known_sig) as u64;
const MEM_KNOWN_ELEM: u64 = std::mem::offset_of!(Mem, fr.known_elem) as u64;
const MEM_KNOWN_UP: u64 = std::mem::offset_of!(Mem, fr.known_up) as u64;
const MEM_FRAME_IDX: u64 = std::mem::offset_of!(Mem, fr.frame) as u64;
const MEM_MAX_DEPTH: u64 = std::mem::offset_of!(Mem, fr.max_depth) as u64;
const MEM_NDEOPT: u64 = std::mem::offset_of!(Mem, ndeopt) as u64;
const MEM_NTAB: u64 = std::mem::offset_of!(Mem, ntab) as u64;
const MEM_STACK_LO: u64 = std::mem::offset_of!(Mem, stack_lo) as u64;
const MEM_NSLOTS: u64 = std::mem::offset_of!(Mem, nslots) as u64;
const MEM_FLAGS: u64 = std::mem::offset_of!(Mem, fr.flags) as u64;
/// Bytes of a `TierSig` and the offsets of its fields.
const SIG_ENT: i64 = std::mem::size_of::<yavs_interp::TierSig>() as i64;
const SIG_CODES: i64 = std::mem::offset_of!(yavs_interp::TierSig, codes) as i64;
const SIG_LEN: i64 = std::mem::offset_of!(yavs_interp::TierSig, len) as i64;
const SIG_SIGNAL: i64 = std::mem::offset_of!(yavs_interp::TierSig, signal) as i64;
const SIG_OFF: i64 = std::mem::offset_of!(yavs_interp::TierSig, off) as i64;

impl Mem {
    fn of(fr: TierFrame) -> Mem {
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
            nslots: std::ptr::null_mut(),
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

    fn load_objs(&mut self, w: &[u64; 17]) {
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
    fn save_objs(&self, slot: &mut Option<Box<yavs_interp::TierState>>) {
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
    fn forget_end(&mut self) {
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
    fn forget(&mut self) {
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
struct Env<'a> {
    host: &'a mut dyn TierHost,
    lookup: Lookup<'a>,
    /// Below this native stack address no compiled callee is entered (`NEST_STACK`).
    stack_lo: usize,
    panic: Option<Box<dyn Any + Send>>,
    /// Native frames recorded on their way to `TierHost::materialise`, innermost first.
    pending: Vec<NativeFrame>,
    /// The slots of the native frames for calls into the interpreter (`TabSlots`), made when
    /// first needed in the activation.
    windows: Option<Box<dyn yavs_interp::NativeSlots>>,
}

/// The address of a local of the caller: where the native stack is.
#[inline(always)]
fn stack_here() -> usize {
    let x = 0u8;
    std::hint::black_box(&x) as *const u8 as usize
}

/// Executes one instruction in the interpreter (`TierHost::step`) and returns `GO`, or what
/// the compiled code returns: the pc after a fault, `TIER_SUSPEND`, `TIER_UNWIND`. A frame the
/// instruction pushed runs here when its function has compiled code, and the call continues
/// as `GO` once that returns (`TierHost::ret`); otherwise the frame is the interpreter's
/// (`TIER_UNWIND`).
extern "C" fn yavs_rt_step(env: *mut c_void, _regs: *mut u64, mem: *mut Mem, pc: u32) -> u32 {
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
extern "C" fn yavs_rt_signal(
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
    stepped(envp, mem, pc, r)
}

/// What a `TierHost::signal`-like call returns to compiled code: `GO`, or the pc after a
/// fault, `TIER_SUSPEND`, `TIER_UNWIND`.
fn stepped(
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
extern "C" fn yavs_rt_deopt(
    env: *mut c_void,
    mem: *mut Mem,
    func: u32,
    pc: u32,
    temp_base: u64,
    nregs: u64,
    slen: u64,
    link: u64,
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
        link,
    });
    let last = env.pending.len() - 1;
    env.pending[last].regs.as_mut_ptr()
}

/// The slot buffer of the frame `yavs_rt_deopt` recorded last.
extern "C" fn yavs_rt_deopt_slots(env: *mut c_void) -> *mut u8 {
    // SAFETY: as in `yavs_rt_step`; called right after `yavs_rt_deopt`.
    let env = unsafe { &mut *(env as *mut Env<'_>) };
    let last = env.pending.len() - 1;
    env.pending[last].slots.as_mut_ptr()
}

/// A frame-mode function's native callee unwound for materialisation: push the recorded
/// frames on the top frame, which waits at its call at `pc`. Returns `TIER_UNWIND` (the
/// interpreter goes on with the top frame), or `pc` after a fault.
extern "C" fn yavs_rt_deopt_end(env: *mut c_void, mem: *mut Mem, pc: u32) -> u32 {
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
        // A native one of the frames ran ended the process.
        Ok(Stepped::Suspend) => TIER_SUSPEND,
        Ok(_) => pc,
        Err(p) => {
            env.panic = Some(p);
            pc
        }
    }
}

/// `TierHost::drive_block` for the `Drive` at `pc`: `wave` (40 readable bytes, checked by the
/// caller) holds the time and the first block of planes of its one waveform element.
extern "C" fn yavs_rt_drive(
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
    stepped(envp, mem, pc, r)
}

/// `TierHost::wait_suspend` for the `WaitOn` at `pc` and the `Suspend` after it.
extern "C" fn yavs_rt_wait(
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
    stepped(envp, mem, pc, r)
}

/// `TierHost::native_direct` for a `CallNative` of a native frame: `cells` (`n`) are the
/// argument cells, `depth` the frame index of the calling native frame. Returns 0 where the
/// interpreter must execute the instruction, 1 when the native ran, 2 when it ran and the
/// process ends (compiled code leaves at the next instruction); bit 8 says the native returned
/// a value, written to `out`.
extern "C" fn yavs_rt_native(
    env: *mut c_void,
    mem: *mut Mem,
    native: u32,
    cells: *const u64,
    n: u32,
    depth: u64,
    out: *mut u64,
) -> u32 {
    // SAFETY: as in `yavs_rt_step`; `cells` holds `n` words of the calling native frame (an
    // `alloca` of the compiled code), `out` one word.
    let envp = env as *mut Env<'_>;
    unsafe { (*mem).sync(&mut *(*envp).host) };
    let r = catch_unwind(AssertUnwindSafe(|| unsafe {
        let cells = std::slice::from_raw_parts(cells, n as usize);
        let env = &mut *envp;
        let w = env
            .windows
            .get_or_insert_with(|| Box::new(TabSlots((*mem).nslots)));
        let _ = w;
        env.host
            .native_direct(&mut (*mem).fr, native, cells, depth, &mut env.windows)
    }));
    // SAFETY: as above.
    let m = unsafe { &mut *mem };
    m.temp_seen = m.fr.temp_len;
    m.forget();
    let put = |v: Option<u64>| {
        if let Some(v) = v {
            // SAFETY: `out` is one word of the caller.
            unsafe { *out = v };
        }
        u32::from(v.is_some()) << 8
    };
    match r {
        Ok(NativeCall::Done(v)) => 1 | put(v),
        Ok(NativeCall::Stop(v)) => 2 | put(v),
        Ok(NativeCall::Slow) => 0,
        Err(p) => {
            // SAFETY: as above.
            unsafe { (*envp).panic = Some(p) };
            0
        }
    }
}

/// `TierHost::temp_root` for a `TempRoot` of a native frame: `ty` is `u32::MAX` for a fat pair,
/// else the type with its `n` descriptor bytes at `desc`. 1 when recorded, 0 where the
/// interpreter must execute the instruction.
extern "C" fn yavs_rt_troot(
    env: *mut c_void,
    mem: *mut Mem,
    addr: u64,
    ty: u32,
    desc: *const u8,
    n: u64,
) -> u32 {
    // SAFETY: as in `yavs_rt_step`; compiled code checked `n` readable bytes at `desc`.
    let envp = env as *mut Env<'_>;
    unsafe { (*mem).sync(&mut *(*envp).host) };
    let r = catch_unwind(AssertUnwindSafe(|| unsafe {
        let d: &[u8] = if n == 0 {
            &[]
        } else {
            std::slice::from_raw_parts(desc, n as usize)
        };
        let ty = (ty != u32::MAX).then_some(ty);
        (*envp).host.temp_root(&mut (*mem).fr, addr, ty, d)
    }));
    // SAFETY: as above.
    let m = unsafe { &mut *mem };
    m.temp_seen = m.fr.temp_len;
    match r {
        Ok(ok) => u32::from(ok),
        Err(p) => {
            // SAFETY: as above.
            unsafe { (*envp).panic = Some(p) };
            0
        }
    }
}

/// `TierHost::end_stmt`: the end of a statement of a native frame whose temporaries start at
/// `base`, while temporaries are recorded for the collector.
extern "C" fn yavs_rt_endstmt(env: *mut c_void, mem: *mut Mem, base: u64) {
    // SAFETY: as in `yavs_rt_step`.
    let envp = env as *mut Env<'_>;
    unsafe { (*mem).sync(&mut *(*envp).host) };
    let r = catch_unwind(AssertUnwindSafe(|| unsafe {
        (*envp).host.end_stmt(&mut (*mem).fr, base);
    }));
    // SAFETY: as above.
    let m = unsafe { &mut *mem };
    m.temp_seen = m.fr.temp_len;
    if let Err(p) = r {
        // SAFETY: as above.
        unsafe { (*envp).panic = Some(p) };
    }
}

/// `TierHost::native_step`: instruction `pc` of `func` for a native frame at frame index `depth`
/// whose register values `ins` (registers `use_idx`) compiled code passes; the registers the
/// instruction writes (`def_idx`) come back in `outs`. 0 where the interpreter must execute the
/// instruction, 1 when it ran, 2 when it ran and the process ends.
extern "C" fn yavs_rt_xstep(
    env: *mut c_void,
    mem: *mut Mem,
    func: u32,
    pc: u32,
    depth: u64,
    use_idx: *const u32,
    ins: *const u64,
    nin: u32,
    def_idx: *const u32,
    outs: *mut u64,
    nout: u32,
    temp_base: u64,
) -> u32 {
    // SAFETY: as in `yavs_rt_step`; `ins` holds `nin` words, `outs` `nout`, the index arrays as
    // many `u32`, all of the calling native frame (allocas and constants of compiled code).
    let envp = env as *mut Env<'_>;
    unsafe { (*mem).sync(&mut *(*envp).host) };
    let r = catch_unwind(AssertUnwindSafe(|| unsafe {
        let ins = std::slice::from_raw_parts(ins, nin as usize);
        let outs = std::slice::from_raw_parts_mut(outs, nout as usize);
        let uses = std::slice::from_raw_parts(use_idx, nin as usize);
        let defs = std::slice::from_raw_parts(def_idx, nout as usize);
        let env = &mut *envp;
        env.windows
            .get_or_insert_with(|| Box::new(TabSlots((*mem).nslots)));
        env.host.native_step(
            &mut (*mem).fr,
            func,
            pc,
            depth,
            (uses, defs),
            ins,
            outs,
            &mut env.windows,
            temp_base,
        )
    }));
    // SAFETY: as above.
    let m = unsafe { &mut *mem };
    m.temp_seen = m.fr.temp_len;
    m.forget();
    match r {
        Ok(NativeCall::Done(_)) => 1,
        Ok(NativeCall::Stop(_)) => 2,
        Ok(NativeCall::Slow) => 0,
        Err(p) => {
            // SAFETY: as above.
            unsafe { (*envp).panic = Some(p) };
            0
        }
    }
}

/// `Inst::LCopy` on the `ns` source and `nd` destination plane blocks compiled code found
/// (`Gen::host`): `yavs_interp::copy_bits`, the interpreter's own copy, on stack copies of both
/// (they may be one vector).
extern "C" fn yavs_rt_lcopy(
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
extern "C" fn yavs_rt_memeq(a: *const u8, b: *const u8, n: u64) -> u64 {
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
unsafe fn step_in(env: *mut Env<'_>, mem: *mut Mem, pc: u32) -> u32 {
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
extern "C" fn yavs_rt_host(
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

/// `Inst::LayoutOf` of an array whose element layout is fixed (`StaticLayout::Array`): reads
/// the `dims` dimensions of the descriptor at `desc`, builds the four layout words in temporary
/// memory and puts its address in register `dst`; 1 when done, 0 where `Ty::layout` or the
/// temporary area would fail (the interpreter then executes the instruction, with its error).
extern "C" fn yavs_rt_layout(
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
    let p = yavs_rt_host(env, mem, desc, 0, u64::from(dims) * 24, 0);
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
fn dim_at(env: *mut c_void, mem: *mut Mem, dope: u64, k: u64) -> Option<u64> {
    let p = yavs_rt_host(env, mem, dope, k.checked_mul(24)?, 24, 0);
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
extern "C" fn yavs_rt_fast(
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
        FAST_DOPE_LEN => match dim_at(env, mem, a, b) {
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
                match (dim_at(env, mem, a, k), dim_at(env, mem, b, k)) {
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
fn yavs_rt_obj(env: *mut c_void, mem: *mut Mem, addr: u64, write: u32) -> u32 {
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

/// Compiled code for the functions of one program.
pub struct LlvmTier {
    /// Keeps the code alive; never used after construction.
    _jit: Mutex<Jit>,
    funcs: Vec<Option<(RunFn, usize)>>,
    /// Per function, its native-frame code's address or 0 (`Mem::ntab`).
    natives: Vec<usize>,
    /// Functions that stay interpreted, with the reason.
    pub notes: Vec<(FuncId, String)>,
    /// Instructions of the compiled functions the interpreter executes (stepped or at an
    /// exit), per instruction kind, most frequent first; and the number compiled inline.
    pub interpreted: Vec<(String, usize)>,
    pub inline: usize,
}

impl LlvmTier {
    /// The functions compiled code calls on native frames (`docs/decisions.md` "Compiled
    /// frames").
    pub fn native_frames(&self) -> usize {
        self.natives.iter().filter(|&&a| a != 0).count()
    }
}

impl Tier for LlvmTier {
    fn run(&self, func: FuncId, pc: u32, host: &mut dyn TierHost) -> Option<u32> {
        let &(f, nregs) = self.funcs.get(func.0 as usize)?.as_ref()?;
        let lookup = |id: FuncId| self.funcs.get(id.0 as usize).copied().flatten();
        enter(f, nregs, pc, host, &lookup, self.natives.as_ptr(), None)
    }

    fn run_cached(
        &self,
        func: FuncId,
        pc: u32,
        host: &mut dyn TierHost,
        state: &mut Option<Box<yavs_interp::TierState>>,
    ) -> Option<u32> {
        let &(f, nregs) = self.funcs.get(func.0 as usize)?.as_ref()?;
        let lookup = |id: FuncId| self.funcs.get(id.0 as usize).copied().flatten();
        enter(
            f,
            nregs,
            pc,
            host,
            &lookup,
            self.natives.as_ptr(),
            Some(state),
        )
    }

    fn ready(&self, func: FuncId) -> bool {
        self.funcs.get(func.0 as usize).is_some_and(Option::is_some)
    }
}

/// Run compiled `f` (generated for `nregs` registers) from `pc` (`Tier::run`); `lookup`
/// finds the compiled code of the functions it calls.
pub(crate) fn enter(
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
        windows: None,
    };
    mem.ntab = ntab;
    mem.stack_lo = env.stack_lo as u64;
    // The frames of native calls are at most `max_depth` deep: their slots are registered here.
    let depth = mem.fr.max_depth.min(1 << 20) as usize;
    mem.nslots = nslots(depth);
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

/// A JIT with the runtime callbacks compiled code calls.
pub(crate) fn new_jit(opt: OptLevel) -> Result<Jit, Error> {
    Jit::new(
        opt,
        &[
            (STEP, yavs_rt_step as *const () as usize),
            (HOST, yavs_rt_host as *const () as usize),
            (SIGNAL, yavs_rt_signal as *const () as usize),
            (DRIVE, yavs_rt_drive as *const () as usize),
            (WAIT, yavs_rt_wait as *const () as usize),
            (LCOPY, yavs_rt_lcopy as *const () as usize),
            (LAYOUT, yavs_rt_layout as *const () as usize),
            (FAST, yavs_rt_fast as *const () as usize),
            (DEOPT, yavs_rt_deopt as *const () as usize),
            (DEOPT_SLOTS, yavs_rt_deopt_slots as *const () as usize),
            (DEOPT_END, yavs_rt_deopt_end as *const () as usize),
            (MEMEQ, yavs_rt_memeq as *const () as usize),
            (NATIVE, yavs_rt_native as *const () as usize),
            (TROOT, yavs_rt_troot as *const () as usize),
            (ENDSTMT, yavs_rt_endstmt as *const () as usize),
            (XSTEP, yavs_rt_xstep as *const () as usize),
        ],
    )
}

/// Why `f` stays interpreted before any code is generated: over `max_size` IR instructions
/// (codegen time grows faster than linearly; #126).
pub(crate) fn too_big(f: &Func, max_size: usize) -> Option<String> {
    (f.code.len() > max_size).then(|| {
        format!(
            "{} IR instructions, over the size cap of {max_size} (--tier-max-size)",
            f.code.len()
        )
    })
}

/// Which code of a function `compile_one` makes.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Part {
    /// Frame-mode code, and native-frame code where the function can have it.
    Both,
    /// Native-frame code only (a callee compiled for its callers).
    Native,
    /// Frame-mode code only (the native code exists already).
    Frame,
}

/// A `--tier=llvm` module of more LLVM instructions than this is compiled at `OptLevel::Fast`.
const EAGER_FAST_MIN: usize = 150_000;

/// Predecessors of one exit block of a frame-mode function (`Gen::exit_for`).
const EXIT_PHI: usize = 64;

/// Functions of at least this many IR instructions get `OptLevel::Fast` instead of `O2` in
/// `level_for`: optimised code generation of such a function takes more than linear time (the
/// register allocator's live-variable analysis over thousands of blocks), 100 to 500 us per LLVM
/// instruction where small functions take 30; the unoptimised code generator takes 5 to 10 us
/// (#432, #501).
const FAST_MIN: usize = 300;

/// The level `f` is compiled at when `level` is asked for (see `FAST_MIN`); compute loops
/// keep theirs.
pub(crate) fn level_for(prog: &Program, f: &Func, level: OptLevel) -> OptLevel {
    if level == OptLevel::O2 && f.code.len() >= FAST_MIN && !crate::auto::compute(prog, f) {
        OptLevel::Fast
    } else {
        level
    }
}

/// Compile `f` alone into `jit`: its entry (none for `Part::Native`) and register count, and the
/// address of its native-frame code (0 for none), or why it stays interpreted.
pub(crate) fn compile_one(
    jit: &mut Jit,
    prog: &Program,
    id: FuncId,
    f: &Func,
    opt: OptLevel,
    part: Part,
) -> Result<(Option<RunFn>, usize, usize), String> {
    if let Some(r) = bad_reg(f) {
        return Err(format!("register r{} beyond the frame", r.0));
    }
    // Native callees are reached through `Mem::ntab`, so only the reachable set's shape
    // matters here, not what is compiled yet.
    let nc = native_set(prog, [id]);
    let name = format!("f{}", id.0);
    let nname = format!("f{}_n", id.0);
    let llvm = |e: Error| format!("LLVM: {e} (a bug in yavs)");
    let mut native = nc.contains(&id) && part != Part::Frame;
    if part == Part::Native && !native {
        return Err("no native-frame code for the function".into());
    }
    let times = std::env::var_os("YAVS_LLVM_TIMES").is_some();
    let t0 = std::time::Instant::now();
    let m = loop {
        let m = Module::new(&name);
        let g = Gen::new(&m);
        // SAFETY: generation of well-formed IR in our own module; a frame-mode function that
        // fails the verifier is deleted again, a native one makes us start again without it.
        if native && unsafe { g.func(prog, f, id, &nname, true, &nc) }.is_err() {
            if part == Part::Native {
                return Err("the generated LLVM function does not verify (a bug in yavs)".into());
            }
            native = false;
            continue;
        }
        if part != Part::Native {
            unsafe { g.func(prog, f, id, &name, false, &nc) }?;
        }
        drop(g);
        break m;
    };
    let t1 = std::time::Instant::now();
    let ninst = if times { m.instruction_count() } else { 0 };
    let full = crate::auto::compute(prog, f);
    let (mut t2, mut t3, mut nopt) = (t1, t1, 0);
    // Verified and optimised only when it is not linked from the compilation cache (#505).
    crate::objcache::add(jit, m, opt, full, |m| {
        m.verify()?;
        t2 = std::time::Instant::now();
        m.optimize_as(opt, full)?;
        t3 = std::time::Instant::now();
        nopt = if times { m.instruction_count() } else { 0 };
        Ok(())
    })
    .map_err(llvm)?;
    let run = if part == Part::Native {
        None
    } else {
        let addr = jit.lookup(&name).map_err(llvm)?;
        // SAFETY: `name` was generated by `Gen::func` with the `RunFn` signature.
        Some(unsafe { std::mem::transmute::<usize, RunFn>(addr) })
    };
    let naddr = if native {
        jit.lookup(&nname).map_err(llvm)?
    } else {
        0
    };
    if times {
        let us = |a: std::time::Instant, b: std::time::Instant| (b - a).as_micros();
        eprintln!(
            "times f{} {opt:?} ir={} llvm={ninst} opt={nopt} gen={} verify={} pass={} codegen={} total={} us native={native}",
            id.0,
            f.code.len(),
            us(t0, t1),
            us(t1, t2),
            us(t2, t3),
            us(t3, std::time::Instant::now()),
            t0.elapsed().as_micros(),
        );
    }
    Ok((run, f.reg_ty.len(), naddr))
}

/// Compile every function of `prog` that the tier can take; the others (among them those over
/// `max_size` IR instructions) stay interpreted and are listed in [`LlvmTier::notes`].
pub fn compile(prog: &Program, opt: OptLevel, max_size: usize) -> Result<LlvmTier, Error> {
    crate::lazy::ensure()?;
    let mut notes = Vec::new();
    let mut todo = Vec::new();
    for (id, f) in prog
        .funcs
        .iter()
        .filter_map(|(id, _)| prog.func(id).map(|f| (id, f)))
    {
        // Elaboration code runs before the simulation, in the interpreter.
        if f.kind == FuncKind::Elab {
            continue;
        }
        if let Some(why) = too_big(f, max_size) {
            notes.push((id, why));
            continue;
        }
        if let Some(r) = bad_reg(f) {
            notes.push((id, format!("register r{} beyond the frame", r.0)));
            continue;
        }
        if let Some(dir) = std::env::var_os("YAVS_LLVM_DUMP") {
            // Debug aid next to the module dumps of `Module::optimize`.
            let _ = std::fs::write(
                std::path::Path::new(&dir).join(format!("f{}.ir", id.0)),
                prog.dump_func(id),
            );
        }
        todo.push((id, f));
    }
    let ids: std::collections::HashSet<FuncId> = todo.iter().map(|t| t.0).collect();
    let mut nc = native_set(prog, ids.iter().copied());
    nc.retain(|id| ids.contains(id));
    let gen_all = |nc: &std::collections::HashSet<FuncId>| {
        let m = Module::new("yavs");
        let mut g = Gen::new(&m);
        g.direct = true;
        let mut names = Vec::new();
        let mut notes = Vec::new();
        for &(id, f) in &todo {
            if nc.contains(&id) {
                // SAFETY: generation of well-formed IR in our own module.
                unsafe { g.func(prog, f, id, &format!("f{}_n", id.0), true, nc) }.ok()?;
            }
        }
        for &(id, f) in &todo {
            let name = format!("f{}", id.0);
            // SAFETY: generation of well-formed IR in our own module; a function that
            // fails the verifier is deleted again.
            match unsafe { g.func(prog, f, id, &name, false, nc) } {
                Ok(()) => names.push((id, name, f.reg_ty.len())),
                Err(why) => notes.push((id, why)),
            }
        }
        drop(g);
        Some((m, names, notes))
    };
    // A native function that does not verify: everything again without native frames.
    let (m, names, more) = match gen_all(&nc) {
        Some(r) => r,
        None => {
            nc.clear();
            gen_all(&nc).ok_or_else(|| Error("no module without native frames".into()))?
        }
    };
    notes.extend(more);
    // Optimised code generation of a module this big takes minutes (#501).
    let opt = if opt == OptLevel::O2 && m.instruction_count() > EAGER_FAST_MIN {
        OptLevel::Fast
    } else {
        opt
    };
    let mut jit = new_jit(opt)?;
    crate::objcache::add(&mut jit, m, opt, false, |m| {
        m.verify()?;
        m.optimize_as(opt, false)
    })?;
    let mut funcs = vec![None; prog.funcs.len()];
    for (id, name, nregs) in names {
        let addr = jit.lookup(&name)?;
        // SAFETY: `name` was generated by `Gen::func` with the `RunFn` signature.
        let f: RunFn = unsafe { std::mem::transmute::<usize, RunFn>(addr) };
        funcs[id.0 as usize] = Some((f, nregs));
    }
    let mut natives = vec![0; prog.funcs.len()];
    for &id in &nc {
        natives[id.0 as usize] = jit.lookup(&format!("f{}_n", id.0))?;
    }
    let mut kinds = std::collections::HashMap::new();
    let mut inline = 0;
    for (id, f) in prog
        .funcs
        .iter()
        .filter_map(|(id, _)| prog.func(id).map(|f| (id, f)))
    {
        if funcs[id.0 as usize].is_none() {
            continue;
        }
        for i in &f.code {
            if is_inline(prog, f, i) {
                inline += 1;
            } else {
                let d = format!("{i:?}");
                let k = d.split([' ', '{', '(']).next().unwrap_or("").to_string();
                *kinds.entry(k).or_insert(0) += 1;
            }
        }
    }
    let mut interpreted: Vec<(String, usize)> = kinds.into_iter().collect();
    interpreted.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    Ok(LlvmTier {
        _jit: Mutex::new(jit),
        funcs,
        natives,
        notes,
        interpreted,
        inline,
    })
}

/// A register operand outside the function's register file, if any.
fn bad_reg(f: &Func) -> Option<Reg> {
    let n = f.reg_ty.len() as u32;
    f.code
        .iter()
        .flat_map(|i| inline_regs(f, i))
        .find(|r| r.0 >= n)
}

/// The registers an inline instruction touches (the compiled code indexes `regs` with them);
/// `Reg(u32::MAX)` for an operand list outside the function.
fn inline_regs(f: &Func, i: &Inst) -> Vec<Reg> {
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
enum NativeKind {
    /// A function of `std_ulogic` codes (`yavs_interp::code_table`): the results by code, the
    /// operand registers and the result register.
    Table(Vec<u8>, Vec<Reg>, Reg),
    /// `rising_edge` (true) or `falling_edge` through `TierHost::signal`.
    Edge(bool),
}

fn native_kind(
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

/// Instructions the compiled code executes itself on its fast path (keep in step with
/// `Gen::func`).
pub(crate) fn is_inline(prog: &Program, f: &Func, i: &Inst) -> bool {
    match i {
        Inst::CallNative { native, args, dst } => matches!(
            native_kind(prog, f, *native, *args, *dst),
            Some(NativeKind::Table(..))
        ),
        Inst::LayoutOf { ty, .. } => static_layout(prog, *ty).is_some(),
        Inst::Bin { op, .. } | Inst::BinRange { op, .. } => *op != IntOp::Pow,
        Inst::FBin { op, .. } => !matches!(op, FOp::Pow | FOp::PowI),
        Inst::AddrOf { space, .. } => {
            addr_const(*space, 0).is_some() || matches!(space, AddrSpace::Up(1..=2))
        }
        _ => !inline_regs(f, i).is_empty() || matches!(i, Inst::Br { .. } | Inst::Stmt),
    }
}

/// `ModRef` of an LLVM `memory(...)` attribute: reads, writes, both.
const REF: u64 = 1;
const MODS: u64 = 2;
const RW: u64 = 3;

/// The value of a `memory(...)` attribute (LLVM 21 `MemoryEffects`: two bits per location,
/// argument memory, inaccessible memory, errno, other memory).
fn mem_effects(arg: u64, inaccessible: u64, other: u64) -> u64 {
    arg | inaccessible << 2 | other << 6
}

/// The most registers, slot bytes and IR instructions of a native-callable function: its
/// registers and slots are `alloca`s, and a materialisation stores every one of them.
const NC_REGS: usize = 512;
const NC_SLOTS: u32 = 4096;
const NC_CODE: usize = 2000;

/// Instructions only a native frame lowers without a step (`Gen::func`): their frame is not the
/// interpreter's, so a frame-mode function steps them. `NewFrame`, `Fill`, a `Cat` of
/// non-logic elements, a typed `TempRoot`, a `CallNative` of a native `yavs_interp::direct_native`
/// allows, and `CallProt` (its callee is checked by `native_set`).
fn nat_inline(prog: &Program, f: &Func, i: &Inst) -> bool {
    let n = f.reg_ty.len() as u32;
    let fits = |rs: &[Reg]| rs.iter().all(|r| r.0 < n);
    match *i {
        Inst::NewFrame {
            dst,
            desc,
            data,
            size,
            ..
        } => fits(&[dst, desc, data, size]),
        Inst::Fill { dst, elem, n, .. } => fits(&[dst, elem, n]),
        Inst::Cat {
            dst,
            n: cnt,
            m,
            parts,
            logic: false,
            w,
            ..
        } => {
            w != Width::Sig
                && fits(&[dst, cnt, m])
                && f.regs(parts)
                    .is_some_and(|rs| rs.len() % 2 == 0 && rs.len() <= CAT_PARTS && fits(rs))
        }
        Inst::TempRoot { addr, desc, ty } => {
            fits(&[addr])
                && desc.is_none_or(|d| fits(&[d]))
                && ty.is_none_or(|t| yavs_ir::Ty::desc_size(&prog.types, t).is_some())
        }
        Inst::CallNative { native, args, dst } => {
            let Some(sym) = prog.natives.get(native).map(|n| &n.symbol) else {
                return false;
            };
            f.regs(args)
                .is_some_and(|rs| fits(rs) && dst.is_none_or(|d| fits(&[d])))
                && (yavs_interp::direct_native(sym) || yavs_interp::cold_native(sym))
        }
        Inst::CallProt { obj, .. } => fits(&[obj]),
        // Cold: leaving to the interpreter is what they do.
        Inst::Unreachable { .. } => true,
        _ => xstep_ok(f, i),
    }
}

/// Instructions a native frame has the interpreter execute on a frame of its own
/// (`TierHost::native_step`): heap objects, vector fills and compares, layouts, reports.
fn xstep_ok(f: &Func, i: &Inst) -> bool {
    let n = f.reg_ty.len() as u32;
    matches!(
        i,
        Inst::New { .. }
            | Inst::NewFill { .. }
            | Inst::Free { .. }
            | Inst::LayoutOf { .. }
            | Inst::LFill { .. }
            | Inst::LEq { .. }
            | Inst::Report { .. }
            | Inst::FToI { .. }
            | Inst::FToIRange { .. }
            | Inst::Phys { .. }
            | Inst::LNum { .. }
            | Inst::LogicOp { .. }
            | Inst::Cat { logic: true, .. }
            | Inst::Bin { op: IntOp::Pow, .. }
            | Inst::BinRange { op: IntOp::Pow, .. }
            | Inst::FBin {
                op: FOp::Pow | FOp::PowI,
                ..
            }
    ) && {
        let (uses, defs) = yavs_interp::inst_regs(f, i);
        uses.iter().chain(&defs).all(|r| r.0 < n)
    }
}

/// The most pieces (operand pairs) of a `Cat` a native frame lowers.
const CAT_PARTS: usize = 64;

/// Whether `f` itself may run on a native frame (`docs/decisions.md` "Compiled frames"): a
/// function or procedure without a static link, small, whose instructions are all lowered
/// without a step on their fast path; its calls are checked by `native_set`.
fn native_local(prog: &Program, f: &Func) -> bool {
    matches!(f.kind, FuncKind::Function | FuncKind::Procedure)
        && f.reg_ty.len() <= NC_REGS
        && f.frame <= NC_SLOTS
        && f.code.len() <= NC_CODE
        && bad_reg(f).is_none()
        && f.code.iter().all(|i| match i {
            Inst::Ret { .. } | Inst::Call { .. } => true,
            // The frame of the lexically enclosing subprogram, one or two levels up.
            Inst::AddrOf {
                space: AddrSpace::Up(d),
                ..
            } => f.parent.is_some() && (1..=2).contains(d),
            _ => is_inline(prog, f, i) || nat_inline(prog, f, i),
        })
}

/// The function a call instruction calls, its argument registers, and whether the first
/// parameter is a protected object (`CallProt`).
fn call_of(i: &Inst) -> Option<(FuncId, yavs_ir::Regs, bool)> {
    match *i {
        Inst::Call { func, args, .. } => Some((func, args, false)),
        Inst::CallProt { func, args, .. } => Some((func, args, true)),
        _ => None,
    }
}

/// Whether the call of `callee` with `args` from `f` (function `caller`) passes one argument per
/// parameter, in registers of `f`, and can give a nested callee its static link; for a protected
/// call (`prot`) the object is the first parameter.
fn call_fits(f: &Func, caller: FuncId, callee: &Func, args: yavs_ir::Regs, prot: bool) -> bool {
    let n = f.reg_ty.len() as u32;
    let skip = usize::from(prot);
    // A nested callee gets its static link from the caller: the caller itself or its own
    // enclosing subprogram.
    if callee
        .parent
        .is_some_and(|p| p != caller && f.parent != Some(p))
    {
        return false;
    }
    if prot && callee.params.first().is_none_or(|p| p.width == Width::Sig) {
        return false;
    }
    f.regs(args).is_some_and(|rs| {
        rs.len() + skip == callee.params.len()
            && rs
                .iter()
                .zip(callee.params.iter().skip(skip))
                .all(|(r, p)| {
                    r.0.checked_add(u32::from(p.width == Width::Sig))
                        .is_some_and(|x| x < n)
                })
    })
}

/// The functions reachable through calls from `roots` that can run on native frames: each
/// `native_local`, and calling only such functions (the greatest such set).
pub(crate) fn native_set(
    prog: &Program,
    roots: impl IntoIterator<Item = FuncId>,
) -> std::collections::HashSet<FuncId> {
    let mut set = std::collections::HashSet::new();
    let mut seen = std::collections::HashSet::new();
    let mut todo: Vec<FuncId> = roots.into_iter().collect();
    while let Some(id) = todo.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Some(f) = prog.func(id).filter(|f| native_local(prog, f)) else {
            continue;
        };
        set.insert(id);
        todo.extend(f.code.iter().filter_map(call_of).map(|c| c.0));
    }
    loop {
        let out: Vec<FuncId> = set
            .iter()
            .copied()
            .filter(|&id| {
                prog.func(id).is_none_or(|f| {
                    f.code.iter().filter_map(call_of).any(|(func, args, prot)| {
                        !set.contains(&func)
                            || prog
                                .func(func)
                                .is_none_or(|g| !call_fits(f, id, g, args, prot))
                    })
                })
            })
            .collect();
        if out.is_empty() {
            return set;
        }
        for id in out {
            set.remove(&id);
        }
    }
}

/// The address `AddrOf` yields in `space`, without the frame index for `Frame`; `None` where
/// the interpreter computes it (static links, an index beyond the address format).
fn addr_const(space: AddrSpace, off: u32) -> Option<u64> {
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
fn keeps_objects(i: &Inst) -> bool {
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

/// Instructions whose step neither collects garbage nor runs other code: it cannot look at a
/// frame variable no address of which was made (`regs::Scalars`), so they are written back
/// only if it fails.
fn quiet(i: &Inst) -> bool {
    // A call is stepped here, so it runs other code.
    !crate::regs::loud(i, &|_| false)
}

/// Instructions after which the interpreter may enter the compiled code: a call it finished
/// (`TIER_UNWIND`), a suspension, the return of a native that handed its call to a VHDL body
/// (`TierHost::step`).
fn resumes(i: &Inst) -> bool {
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
enum StaticLayout {
    Words(Vec<u64>),
    Array {
        dims: u8,
        elem: yavs_ir::Layout,
        logic: bool,
    },
}

/// The `LayoutOf` of `ty` as far as it does not depend on the descriptor, as `Ty::layout`
/// computes it; `None` where the interpreter computes it.
fn static_layout(prog: &Program, ty: yavs_ir::TyId) -> Option<StaticLayout> {
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
enum Known {
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
fn fixed_consts(f: &Func) -> Vec<Option<i64>> {
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

/// A preloaded register: its value, or the address space and offset of its address.
type Preload = (Reg, Option<(AddrSpace, u32)>, i64);

/// What the interpreter preloads into a frame's registers (`Pre`): the registers `f` defines
/// exactly once, by a `Const`, a `ConstF` or an `AddrOf` (not of a static link), with the value
/// or the address space and offset. A native frame stores them at entry, since the definition
/// may not have run on the way to a use.
fn preloads(f: &Func) -> Vec<Preload> {
    let n = f.reg_ty.len();
    let mut defs = vec![0u32; n + 1];
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
    }
    f.code
        .iter()
        .filter_map(|i| match *i {
            Inst::Const { dst, v } => Some((dst, None, v)),
            Inst::ConstF { dst, v } => Some((dst, None, v.to_bits() as i64)),
            Inst::AddrOf { dst, space, off } if !matches!(space, AddrSpace::Up(_)) => {
                Some((dst, Some((space, off)), 0))
            }
            _ => None,
        })
        .filter(|(r, ..)| defs.get(r.0 as usize) == Some(&1))
        .collect()
}

/// Update `known` for what `i` writes.
fn track(known: &mut [Option<Known>], fixed: &[Option<i64>], i: &Inst) {
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

struct Gen {
    ctx: LLVMContextRef,
    module: LLVMModuleRef,
    b: LLVMBuilderRef,
    i8: LLVMTypeRef,
    i32: LLVMTypeRef,
    i64: LLVMTypeRef,
    f64: LLVMTypeRef,
    ptr: LLVMTypeRef,
    fn_ty: LLVMTypeRef,
    step: LLVMValueRef,
    /// `yavs_rt_host(env, mem, addr, extra, n, write) -> ptr`, null for the slow path.
    host_ty: LLVMTypeRef,
    host: LLVMValueRef,
    /// `yavs_rt_signal(env, regs, mem, pc, edge) -> u32`, as `step`.
    signal_ty: LLVMTypeRef,
    signal: LLVMValueRef,
    /// `yavs_rt_drive(env, regs, mem, pc, driver, lo, len, wave, inertial) -> u32`.
    drive_ty: LLVMTypeRef,
    drive: LLVMValueRef,
    /// `yavs_rt_wait(env, regs, mem, pc, lo, len) -> u32`.
    wait_ty: LLVMTypeRef,
    wait: LLVMValueRef,
    /// `yavs_rt_lcopy(dst, nd, d0, src, ns, s0, n)`.
    lcopy_ty: LLVMTypeRef,
    lcopy: LLVMValueRef,
    /// `yavs_rt_layout(env, regs, mem, desc, dst, dims, stride, align, el_count, logic) -> u32`.
    layout_ty: LLVMTypeRef,
    layout: LLVMValueRef,
    /// `yavs_rt_fast(env, regs, mem, op, dst, a, b) -> u32`.
    fast_ty: LLVMTypeRef,
    fast: LLVMValueRef,
    /// `yavs_rt_deopt(env, mem, func, pc, temp_base, nregs, slen, link) -> ptr`.
    deopt_ty: LLVMTypeRef,
    deopt: LLVMValueRef,
    /// `yavs_rt_deopt_slots(env) -> ptr`.
    deopt_slots_ty: LLVMTypeRef,
    deopt_slots: LLVMValueRef,
    /// `yavs_rt_deopt_end(env, mem, pc) -> u32`.
    deopt_end_ty: LLVMTypeRef,
    deopt_end: LLVMValueRef,
    /// `yavs_rt_memeq(a, b, n) -> i64`.
    memeq_ty: LLVMTypeRef,
    memeq: LLVMValueRef,
    /// `yavs_rt_native(env, mem, native, cells, n, depth, out) -> u32`.
    native_ty: LLVMTypeRef,
    native: LLVMValueRef,
    /// `yavs_rt_troot(env, mem, addr, ty, desc, n) -> u32`.
    troot_ty: LLVMTypeRef,
    troot: LLVMValueRef,
    /// `yavs_rt_endstmt(env, mem, base)`.
    endstmt_ty: LLVMTypeRef,
    endstmt: LLVMValueRef,
    /// `yavs_rt_xstep(env, mem, func, pc, depth, use_idx, ins, nin, def_idx, outs, nout, temp_base) -> u32`.
    xstep_ty: LLVMTypeRef,
    xstep: LLVMValueRef,
    /// `llvm.frameaddress.p0(i32) -> ptr`.
    frame_addr: (LLVMTypeRef, LLVMValueRef),
    /// Native callees are called by symbol (one module, `compile`), else through `Mem::ntab`.
    pub(crate) direct: bool,
    /// Which memory a pointer value points into (`AA_*`), for alias scope metadata.
    kinds: std::cell::RefCell<std::collections::HashMap<usize, usize>>,
    /// `alias.scope` and `noalias` list nodes per kind, and their metadata kind ids.
    aa: std::cell::RefCell<Option<AaNodes>>,
    sadd: (LLVMTypeRef, LLVMValueRef),
    ssub: (LLVMTypeRef, LLVMValueRef),
    smul: (LLVMTypeRef, LLVMValueRef),
    umul: (LLVMTypeRef, LLVMValueRef),
    uadd: (LLVMTypeRef, LLVMValueRef),
}

/// Memory kinds that never overlap: the frame's slots, the temporary area, the instance data,
/// constants, the signals' code bytes. A pointer of a known kind gets alias scope metadata, so
/// LLVM keeps values in registers across stores to the others.
const AA_SLOTS: usize = 0;
const AA_TEMP: usize = 1;
const AA_INST: usize = 2;
const AA_CONST: usize = 3;
const AA_CODES: usize = 4;
const AA_N: usize = 5;

struct AaNodes {
    scope: [LLVMValueRef; AA_N],
    others: [LLVMValueRef; AA_N],
    k_scope: u32,
    k_noalias: u32,
}

/// The function being generated.
struct Cur<'p> {
    prog: &'p Program,
    /// The private globals holding constants this function reads (`Gen::host`).
    consts: std::cell::RefCell<std::collections::HashMap<yavs_ir::ConstId, LLVMValueRef>>,
    f: LLVMValueRef,
    env: LLVMValueRef,
    regs: LLVMValueRef,
    mem: LLVMValueRef,
    /// What the generator knows about each register at the current instruction (`track`).
    known: std::cell::RefCell<Vec<Option<Known>>>,
    /// Registers the function defines once, by a `Const`: that value wherever they are read
    /// (the interpreter's loop skips such a definition and relies on the same).
    fixed: Vec<Option<i64>>,
    /// The blocks that return the function's result (`exit_for`): block, phi, predecessors.
    exits: std::cell::RefCell<Vec<(LLVMBasicBlockRef, LLVMValueRef, usize)>>,
    /// The native frame's exit block (`exit_for`) and its phi.
    exit: (LLVMBasicBlockRef, LLVMValueRef),
    /// The frame's size in bytes (`Func::frame`): the slots of every frame of the function.
    frame_len: u64,
    /// The native frame, when this is a function's native-frame code (`f<id>_n`).
    nat: Option<Nat>,
    /// Frame-mode code that stores a register only where the interpreter can look at it.
    defer: Option<Defer>,
    /// Frame variables kept as values (with `defer`): slot `k` is deferred register `nregs + k`.
    scal: Option<crate::regs::Scalars>,
    /// The frame's base before each slow path being generated (`slow_path`), and whether its
    /// step may push a frame; `join` takes them back.
    joins: std::cell::RefCell<Vec<(Option<LLVMValueRef>, bool)>>,
}

/// Deferred register stores (`regs.rs`): a definition of a pure instruction is kept as a value
/// in `pend` and stored by `Gen::flush` where it is live and compiled code leaves the frame
/// to the interpreter (a failing guard, a stepped instruction, a block edge); registers the
/// interpreter preloads are never stored.
struct Defer {
    live: crate::regs::Live,
    pre: Vec<bool>,
    pend: std::cell::RefCell<Vec<Option<LLVMValueRef>>>,
    /// Registers deferred by the instruction being generated.
    fresh: std::cell::RefCell<Vec<u32>>,
    /// The pc of the instruction being generated.
    pc: std::cell::Cell<u32>,
    /// Registers carried across blocks as phis, when the function is small enough.
    plan: Option<Plan>,
    /// The instructions inside a loop. Outside loops a result is stored at once: a deferred
    /// one is stored at every block edge and guard exit it is live at (code, and codegen time
    /// beyond linear, for a function of hundreds of registers), and saves nothing that runs
    /// more than once.
    loopy: Vec<bool>,
}

/// The registers live into each block that are phis there (`regs`, ascending, and the phi of
/// each), per entry pc; empty for pcs that are no block start or are never reached.
struct Plan {
    regs: Vec<Vec<u32>>,
    phis: Vec<Vec<LLVMValueRef>>,
    /// Block starts reached by exactly one edge, from an earlier instruction: they take that
    /// edge's deferred values (`inherit`) as they are, with no phi.
    single: Vec<bool>,
    inherit: std::cell::RefCell<Vec<Option<Vec<Option<LLVMValueRef>>>>>,
}

/// Phis of a function over this many (summed over its blocks) keep the stores at block edges.
const MAX_PHIS: usize = 100_000;

/// A native frame (`docs/decisions.md` "Compiled frames"): slots in an `alloca`, the high half
/// of its frame addresses, its frame index, the temporary area's length at entry, a word for
/// the helpers that write a register, and the function.
struct Nat {
    slots: LLVMValueRef,
    fhi: LLVMValueRef,
    depth: LLVMValueRef,
    /// The high halves of the addresses of the frames one and two static links up (as
    /// `TierFrame::up`): parameters of a nested subprogram, else `u64::MAX`.
    up: [LLVMValueRef; 2],
    temp_base: LLVMValueRef,
    scratch: LLVMValueRef,
    id: FuncId,
}

impl Gen {
    fn new(m: &Module) -> Gen {
        let (ctx, module) = (m.context(), m.raw());
        // SAFETY: ctx and module live as long as `m`; the builder is disposed in Drop.
        unsafe {
            let i64 = LLVMInt64TypeInContext(ctx);
            let i32 = LLVMInt32TypeInContext(ctx);
            let ptr = LLVMPointerTypeInContext(ctx, 0);
            let intrinsic = |name: &std::ffi::CStr| {
                let id = LLVMLookupIntrinsicID(name.as_ptr(), name.count_bytes());
                assert_ne!(id, 0, "unknown intrinsic {name:?}");
                let mut tys = [i64];
                (
                    LLVMIntrinsicGetType(ctx, id, tys.as_mut_ptr(), 1),
                    LLVMGetIntrinsicDeclaration(module, id, tys.as_mut_ptr(), 1),
                )
            };
            let mut params = [ptr, ptr, ptr, i32];
            let fn_ty = LLVMFunctionType(i32, params.as_mut_ptr(), 4, 0);
            let step_name = CString::new(STEP).expect("no NUL");
            let host_name = CString::new(HOST).expect("no NUL");
            let mut hparams = [ptr, ptr, i64, i64, i64, i32];
            let host_ty = LLVMFunctionType(ptr, hparams.as_mut_ptr(), 6, 0);
            let signal_name = CString::new(SIGNAL).expect("no NUL");
            let mut sparams = [ptr, ptr, ptr, i32, i32];
            let signal_ty = LLVMFunctionType(i32, sparams.as_mut_ptr(), 5, 0);
            let drive_name = CString::new(DRIVE).expect("no NUL");
            let mut dparams = [ptr, ptr, ptr, i32, i32, i64, i64, ptr, i32];
            let drive_ty = LLVMFunctionType(i32, dparams.as_mut_ptr(), 9, 0);
            let wait_name = CString::new(WAIT).expect("no NUL");
            let mut wparams = [ptr, ptr, ptr, i32, i64, i64];
            let wait_ty = LLVMFunctionType(i32, wparams.as_mut_ptr(), 6, 0);
            let lcopy_name = CString::new(LCOPY).expect("no NUL");
            let mut lparams = [ptr, i64, i64, ptr, i64, i64, i64];
            let lcopy_ty = LLVMFunctionType(LLVMVoidTypeInContext(ctx), lparams.as_mut_ptr(), 7, 0);
            let fast_name = CString::new(FAST).expect("no NUL");
            let mut fparams = [ptr, ptr, ptr, i32, i32, i64, i64];
            let fast_ty = LLVMFunctionType(i32, fparams.as_mut_ptr(), 7, 0);
            let layout_name = CString::new(LAYOUT).expect("no NUL");
            let mut yparams = [ptr, ptr, ptr, i64, i32, i32, i64, i64, i64, i32];
            let layout_ty = LLVMFunctionType(i32, yparams.as_mut_ptr(), 10, 0);
            let mut dparams = [ptr, ptr, i32, i32, i64, i64, i64, i64];
            let deopt_ty = LLVMFunctionType(ptr, dparams.as_mut_ptr(), 8, 0);
            let mut dsparams = [ptr];
            let deopt_slots_ty = LLVMFunctionType(ptr, dsparams.as_mut_ptr(), 1, 0);
            let mut deparams = [ptr, ptr, i32];
            let deopt_end_ty = LLVMFunctionType(i32, deparams.as_mut_ptr(), 3, 0);
            let mut mparams = [ptr, ptr, i64];
            let memeq_ty = LLVMFunctionType(i64, mparams.as_mut_ptr(), 3, 0);
            let mut nparams = [ptr, ptr, i32, ptr, i32, i64, ptr];
            let native_ty = LLVMFunctionType(i32, nparams.as_mut_ptr(), 7, 0);
            let mut tparams = [ptr, ptr, i64, i32, ptr, i64];
            let troot_ty = LLVMFunctionType(i32, tparams.as_mut_ptr(), 6, 0);
            let mut eparams = [ptr, ptr, i64];
            let endstmt_ty =
                LLVMFunctionType(LLVMVoidTypeInContext(ctx), eparams.as_mut_ptr(), 3, 0);
            let mut xparams = [ptr, ptr, i32, i32, i64, ptr, ptr, i32, ptr, ptr, i32, i64];
            let xstep_ty = LLVMFunctionType(i32, xparams.as_mut_ptr(), 12, 0);
            let add = |name: &str, ty| {
                let n = CString::new(name).expect("no NUL");
                LLVMAddFunction(module, n.as_ptr(), ty)
            };
            let frame_addr = {
                let id = LLVMLookupIntrinsicID(c"llvm.frameaddress".as_ptr(), 17);
                assert_ne!(id, 0, "unknown intrinsic llvm.frameaddress");
                let mut tys = [ptr];
                (
                    LLVMIntrinsicGetType(ctx, id, tys.as_mut_ptr(), 1),
                    LLVMGetIntrinsicDeclaration(module, id, tys.as_mut_ptr(), 1),
                )
            };
            let g = Gen {
                ctx,
                module,
                b: LLVMCreateBuilderInContext(ctx),
                i8: LLVMInt8TypeInContext(ctx),
                i32,
                i64,
                f64: LLVMDoubleTypeInContext(ctx),
                ptr,
                fn_ty,
                step: LLVMAddFunction(module, step_name.as_ptr(), fn_ty),
                host_ty,
                host: LLVMAddFunction(module, host_name.as_ptr(), host_ty),
                signal_ty,
                signal: LLVMAddFunction(module, signal_name.as_ptr(), signal_ty),
                drive_ty,
                drive: LLVMAddFunction(module, drive_name.as_ptr(), drive_ty),
                wait_ty,
                wait: LLVMAddFunction(module, wait_name.as_ptr(), wait_ty),
                lcopy_ty,
                lcopy: LLVMAddFunction(module, lcopy_name.as_ptr(), lcopy_ty),
                layout_ty,
                layout: LLVMAddFunction(module, layout_name.as_ptr(), layout_ty),
                fast_ty,
                fast: LLVMAddFunction(module, fast_name.as_ptr(), fast_ty),
                deopt_ty,
                deopt: add(DEOPT, deopt_ty),
                deopt_slots_ty,
                deopt_slots: add(DEOPT_SLOTS, deopt_slots_ty),
                deopt_end_ty,
                deopt_end: add(DEOPT_END, deopt_end_ty),
                memeq_ty,
                memeq: add(MEMEQ, memeq_ty),
                native_ty,
                native: add(NATIVE, native_ty),
                troot_ty,
                troot: add(TROOT, troot_ty),
                endstmt_ty,
                endstmt: add(ENDSTMT, endstmt_ty),
                xstep_ty,
                xstep: add(XSTEP, xstep_ty),
                frame_addr,
                direct: false,
                kinds: Default::default(),
                aa: Default::default(),
                sadd: intrinsic(c"llvm.sadd.with.overflow"),
                ssub: intrinsic(c"llvm.ssub.with.overflow"),
                smul: intrinsic(c"llvm.smul.with.overflow"),
                umul: intrinsic(c"llvm.umul.with.overflow"),
                uadd: intrinsic(c"llvm.uadd.with.overflow"),
            };
            g.effects();
            g
        }
    }

    /// Attribute `name` (with `val` for an integer attribute) on function `f` or call `f` at
    /// `idx` (`LLVMAttributeFunctionIndex` for the function).
    unsafe fn attr(&self, f: LLVMValueRef, call: bool, name: &str, val: u64) {
        // SAFETY: a function or call of our context; the attribute exists in LLVM 21.
        unsafe {
            let kind = LLVMGetEnumAttributeKindForName(name.as_ptr().cast(), name.len());
            assert_ne!(kind, 0, "unknown attribute {name}");
            let a = LLVMCreateEnumAttribute(self.ctx, kind, val);
            let idx = llvm_sys::LLVMAttributeFunctionIndex;
            if call {
                LLVMAddCallSiteAttribute(f, idx, a);
            } else {
                LLVMAddAttributeAtIndex(f, idx, a);
            }
        }
    }

    /// What the runtime helpers do, so LLVM keeps values in registers across them and moves
    /// loads past them (`docs/decisions.md` "Compiled frames", effects): none unwinds (they
    /// catch panics); `yavs_rt_host` touches only `Mem`, `Env` and the interpreter's own
    /// bookkeeping, never object bytes, registers or slots; `yavs_rt_lcopy` only the blocks it
    /// is given. `yavs_rt_step` and `yavs_rt_signal` run interpreter code that may collect or
    /// change any memory: no memory attribute, so registers holding handles are reloaded after
    /// them and the precise roots (#422) see what compiled code stored.
    unsafe fn effects(&self) {
        // SAFETY: functions of our module.
        unsafe {
            for f in [
                self.step,
                self.host,
                self.signal,
                self.drive,
                self.wait,
                self.lcopy,
                self.layout,
                self.fast,
                self.deopt,
                self.deopt_slots,
                self.deopt_end,
                self.native,
                self.troot,
                self.endstmt,
                self.xstep,
            ] {
                self.attr(f, false, "nounwind", 0);
            }
            self.attr(self.memeq, false, "nounwind", 0);
            for f in [self.host, self.lcopy, self.deopt_slots, self.memeq] {
                self.attr(f, false, "willreturn", 0);
            }
            self.attr(self.memeq, false, "memory", mem_effects(REF, 0, 0));
            self.attr(self.memeq, false, "nofree", 0);
            self.attr(self.memeq, false, "nosync", 0);
            self.attr(self.host, false, "memory", mem_effects(RW, RW, 0));
            self.attr(self.lcopy, false, "memory", mem_effects(RW, 0, 0));
            self.attr(self.lcopy, false, "nofree", 0);
            self.attr(self.lcopy, false, "nosync", 0);
            self.attr(self.deopt_slots, false, "memory", mem_effects(REF, 0, 0));
        }
    }

    fn c64(&self, v: i64) -> LLVMValueRef {
        // SAFETY: constant creation.
        unsafe { LLVMConstInt(self.i64, v as u64, 1) }
    }

    fn c32(&self, v: u32) -> LLVMValueRef {
        // SAFETY: constant creation.
        unsafe { LLVMConstInt(self.i32, u64::from(v), 0) }
    }

    unsafe fn block(&self, c: &Cur) -> LLVMBasicBlockRef {
        // SAFETY: c.f is a function of our context.
        unsafe { LLVMAppendBasicBlockInContext(self.ctx, c.f, c"".as_ptr()) }
    }

    unsafe fn slot(&self, c: &Cur, r: Reg) -> LLVMValueRef {
        // SAFETY: r is below the register count (`bad_reg`).
        unsafe {
            let mut idx = [self.c64(i64::from(r.0))];
            LLVMBuildGEP2(self.b, self.i64, c.regs, idx.as_mut_ptr(), 1, c"".as_ptr())
        }
    }

    /// What the generator knows about address register `r` (not an integer).
    fn known_addr(&self, c: &Cur, r: Reg) -> Option<Known> {
        match c.known.borrow().get(r.0 as usize) {
            Some(&Some(k)) if !matches!(k, Known::Int(_)) => Some(k),
            _ => None,
        }
    }

    /// The value of register `r` when the generator knows it.
    fn kint(&self, c: &Cur, r: Reg) -> Option<i64> {
        match c.known.borrow().get(r.0 as usize) {
            Some(Some(Known::Int(v))) => Some(*v),
            _ => c.fixed.get(r.0 as usize).copied().flatten(),
        }
    }

    unsafe fn ld(&self, c: &Cur, r: Reg) -> LLVMValueRef {
        if let Some(v) = self.kint(c, r) {
            return self.c64(v);
        }
        if let Some(d) = &c.defer
            && let Some(Some(v)) = d.pend.borrow().get(r.0 as usize)
        {
            return *v;
        }
        // SAFETY: see `slot`.
        unsafe { LLVMBuildLoad2(self.b, self.i64, self.slot(c, r), c"".as_ptr()) }
    }

    /// Return `v` (an i32): a branch to the function's exit block.
    unsafe fn leave(&self, c: &Cur, v: LLVMValueRef) {
        // SAFETY: the builder is in a block of c.f; the phi is the exit block's.
        unsafe {
            let exit = self.exit_for(c, v);
            LLVMBuildBr(self.b, exit);
        }
    }

    /// The exit block to branch to from the current block with `v` (an i32) to return: `v` is
    /// added to its phi. A frame-mode function has several (`EXIT_PHI` predecessors each): one
    /// phi of thousands of incoming values makes the simplifier and the phi elimination
    /// quadratic.
    unsafe fn exit_for(&self, c: &Cur, v: LLVMValueRef) -> LLVMBasicBlockRef {
        // SAFETY: well-formed blocks of c.f.
        unsafe {
            let from = LLVMGetInsertBlock(self.b);
            let mut xs = c.exits.borrow_mut();
            if xs.last().is_none_or(|x| x.2 >= EXIT_PHI) {
                let blk = self.block(c);
                LLVMPositionBuilderAtEnd(self.b, blk);
                let phi = LLVMBuildPhi(self.b, self.i32, c"".as_ptr());
                if c.nat.is_none() {
                    LLVMBuildRet(self.b, phi);
                } else {
                    // The native frame's one exit (materialisation) takes the chunks' values.
                    let (exit, exit_pc) = c.exit;
                    LLVMBuildBr(self.b, exit);
                    LLVMAddIncoming(exit_pc, &mut { phi }, &mut { blk }, 1);
                }
                LLVMPositionBuilderAtEnd(self.b, from);
                xs.push((blk, phi, 0));
            }
            let x = xs.last_mut().expect("exit block");
            LLVMAddIncoming(x.1, &mut { v }, &mut { from }, 1);
            x.2 += 1;
            x.0
        }
    }

    unsafe fn ldf(&self, c: &Cur, r: Reg) -> LLVMValueRef {
        // SAFETY: see `slot`.
        unsafe { LLVMBuildBitCast(self.b, self.ld(c, r), self.f64, c"".as_ptr()) }
    }

    /// Store `v` in register `r` at once.
    unsafe fn st(&self, c: &Cur, r: Reg, v: LLVMValueRef) {
        if let Some(d) = &c.defer
            && let Some(p) = d.pend.borrow_mut().get_mut(r.0 as usize)
        {
            *p = None;
        }
        // SAFETY: see `slot`; `v` is an i64.
        unsafe {
            LLVMBuildStore(self.b, v, self.slot(c, r));
        }
    }

    /// Define register `r` as `v` without a store: `flush` stores it where the frame is looked
    /// at. Only in straight-line code (not between the fast and slow path of an instruction).
    unsafe fn std(&self, c: &Cur, r: Reg, v: LLVMValueRef) {
        if let Some(d) = &c.defer
            && d.loopy[d.pc.get() as usize]
            && let Some(p) = d.pend.borrow_mut().get_mut(r.0 as usize)
        {
            // The interpreter starts every frame with a register defined once by a constant
            // or an address: nothing to store.
            *p = (!d.pre[r.0 as usize]).then_some(v);
            d.fresh.borrow_mut().push(r.0);
            return;
        }
        // SAFETY: as `st`.
        unsafe { self.st(c, r, v) }
    }

    unsafe fn stfd(&self, c: &Cur, r: Reg, v: LLVMValueRef) {
        // SAFETY: `v` is a double.
        unsafe { self.std(c, r, LLVMBuildBitCast(self.b, v, self.i64, c"".as_ptr())) }
    }

    /// Store the deferred registers that `live` (a row of `Live`) has live.
    unsafe fn flush_row(&self, c: &Cur, live: &[u64]) {
        let Some(d) = &c.defer else { return };
        let mut base = None;
        for (r, v) in d.pend.borrow().iter().enumerate() {
            if let Some(v) = v
                && live.get(r / 64).is_some_and(|w| w >> (r % 64) & 1 != 0)
            {
                // SAFETY: r is a register of the function, or a slot variable.
                unsafe {
                    let nregs = c.known.borrow().len();
                    if r < nregs {
                        LLVMBuildStore(self.b, *v, self.slot(c, Reg(r as u32)));
                    } else if c.scal.as_ref().is_some_and(|s| s.stored[r - nregs]) {
                        let b = *base.get_or_insert_with(|| self.tag(self.own_slots(c), AA_SLOTS));
                        self.vslot_store_at(c, r - nregs, *v, b);
                    }
                }
            }
        }
    }

    /// Write deferred value `v` of register (or, past the registers, slot variable) `r` to the frame.
    unsafe fn store_back(&self, c: &Cur, r: usize, v: LLVMValueRef) {
        let nregs = c.known.borrow().len();
        // SAFETY: r is a register of the function, or a slot variable.
        unsafe {
            if r < nregs {
                LLVMBuildStore(self.b, v, self.slot(c, Reg(r as u32)));
            } else if c.scal.as_ref().is_some_and(|s| s.stored[r - nregs]) {
                self.vslot_store(c, r - nregs, v);
            }
        }
    }

    /// Store the registers (not the slot variables) live at `pc`; the slot variables are
    /// returned as (slot, value) pairs, for `leave_slots`.
    unsafe fn flush_regs(&self, c: &Cur, pc: u32) -> Vec<(usize, usize)> {
        let Some(d) = &c.defer else { return Vec::new() };
        let nregs = c.known.borrow().len();
        let row = d.live.row(pc as usize);
        let mut key = Vec::new();
        for (r, v) in d.pend.borrow().iter().enumerate() {
            let Some(v) = v else { continue };
            if row[r / 64] >> (r % 64) & 1 == 0 {
                continue;
            }
            if r < nregs {
                // SAFETY: r is a register of the function.
                unsafe { LLVMBuildStore(self.b, *v, self.slot(c, Reg(r as u32))) };
            } else if c.scal.as_ref().is_some_and(|s| s.stored[r - nregs]) {
                key.push((r - nregs, *v as usize));
            }
        }
        key
    }

    /// Leave with `val` (an i32), the slot variables of `key` written back first.
    unsafe fn leave_slots(&self, c: &Cur, key: Vec<(usize, usize)>, val: LLVMValueRef) {
        // SAFETY: well-formed blocks of c.f; the values dominate the builder's block.
        unsafe {
            if !key.is_empty() {
                let base = self.tag(self.own_slots(c), AA_SLOTS);
                for &(k, v) in &key {
                    self.vslot_store_at(c, k, v as LLVMValueRef, base);
                }
            }
            self.leave(c, val);
        }
    }

    /// Leave to the interpreter at `pc` (a guard failed), with the frame as it reads it.
    unsafe fn flush_exit(&self, c: &Cur, pc: u32) {
        // SAFETY: as `leave_slots`.
        unsafe {
            let key = self.flush_regs(c, pc);
            self.leave_slots(c, key, self.c32(pc));
        }
    }

    /// Store what the interpreter reads when it takes over at instruction `pc`.
    unsafe fn flush(&self, c: &Cur, pc: usize) {
        if let Some(d) = &c.defer {
            // SAFETY: as `flush_row`.
            unsafe { self.flush_row(c, d.live.row(pc)) };
        }
    }

    /// Store what the successors `pcs` read.
    unsafe fn flush_edge(&self, c: &Cur, pcs: &[usize]) {
        let Some(d) = &c.defer else { return };
        let mut row = d.live.row(pcs[0]).to_vec();
        for &p in &pcs[1..] {
            for (a, b) in row.iter_mut().zip(d.live.row(p)) {
                *a |= b;
            }
        }
        // SAFETY: as `flush_row`.
        unsafe { self.flush_row(c, &row) };
    }

    /// At a branch to the block starts `pcs`: store what they read (without `Plan`), or the
    /// values their phis take.
    unsafe fn edge_prep(&self, c: &Cur, pcs: &[usize]) -> Vec<(u32, LLVMValueRef)> {
        let Some(d) = &c.defer else { return Vec::new() };
        let Some(pl) = &d.plan else {
            // SAFETY: as `flush_edge`.
            unsafe { self.flush_edge(c, pcs) };
            return Vec::new();
        };
        let pcs: Vec<usize> = pcs.iter().copied().filter(|&p| !pl.single[p]).collect();
        let mut regs: Vec<u32> = pcs
            .iter()
            .flat_map(|&p| pl.regs[p].iter().copied())
            .collect();
        regs.sort_unstable();
        regs.dedup();
        // A deferred value no phi takes is stored for the successor, as without phis.
        let mut back: Vec<usize> = Vec::new();
        for &p in &pcs {
            let row = d.live.row(p);
            for (r, v) in d.pend.borrow().iter().enumerate() {
                if v.is_some()
                    && row[r / 64] >> (r % 64) & 1 != 0
                    && pl.regs[p].binary_search(&(r as u32)).is_err()
                    && !back.contains(&r)
                {
                    back.push(r);
                }
            }
        }
        for r in back {
            // SAFETY: as `flush_row`.
            unsafe { self.store_back(c, r, d.pend.borrow()[r].expect("deferred")) };
        }
        regs.into_iter()
            .map(|r| {
                // SAFETY: r is a register of the function.
                let v =
                    d.pend.borrow()[r as usize].unwrap_or_else(|| unsafe { self.frame_load(c, r) });
                (r, v)
            })
            .collect()
    }

    /// The edge from the current block to block start `pc`: its phis take `snap`.
    unsafe fn edge_to(&self, c: &Cur, snap: &[(u32, LLVMValueRef)], pc: usize) {
        let Some(d) = &c.defer else { return };
        let Some(pl) = &d.plan else { return };
        if pl.single[pc] {
            pl.inherit.borrow_mut()[pc].get_or_insert_with(|| d.pend.borrow().clone());
            return;
        }
        // SAFETY: the phis are of blocks of c.f and take i64.
        unsafe {
            let from = LLVMGetInsertBlock(self.b);
            for (&r, &phi) in pl.regs[pc].iter().zip(&pl.phis[pc]) {
                if let Some(&(_, mut v)) = snap.iter().find(|x| x.0 == r) {
                    LLVMAddIncoming(phi, &mut v, &mut { from }, 1);
                }
            }
        }
    }

    /// Register `r` (below the register count) or slot variable `r - nregs` as the frame has it.
    unsafe fn frame_load(&self, c: &Cur, r: u32) -> LLVMValueRef {
        let nregs = c.known.borrow().len();
        let base = c
            .defer
            .as_ref()
            .map_or(usize::MAX, |d| d.pend.borrow().len() - 1);
        // SAFETY: r is a register of the function, a slot variable, or the slots' base.
        unsafe {
            if r as usize == base {
                self.own_slots(c)
            } else if (r as usize) < nregs {
                LLVMBuildLoad2(self.b, self.i64, self.slot(c, Reg(r)), c"".as_ptr())
            } else {
                self.vslot_load(c, r as usize - nregs, false)
            }
        }
    }

    /// The address of slot variable `k` in the frame's slots.
    unsafe fn vslot_ptr(&self, c: &Cur, k: usize, cached: bool) -> (LLVMValueRef, Width) {
        let (off, w) = c.scal.as_ref().expect("slot variables").slots[k];
        // SAFETY: the slot lies within the frame (`Scalars::new`).
        unsafe {
            let base = if cached {
                self.own_slots_cached(c)
            } else {
                self.own_slots(c)
            };
            (
                self.gep(self.tag(base, AA_SLOTS), self.c64(i64::from(off))),
                w,
            )
        }
    }

    /// Slot variable `k` as the frame holds it, widened as `Load` does.
    unsafe fn vslot_load(&self, c: &Cur, k: usize, cached: bool) -> LLVMValueRef {
        // SAFETY: as `vslot_ptr`.
        unsafe {
            let (p, w) = self.vslot_ptr(c, k, cached);
            let v = self.load(self.width_ty(w), p);
            match w {
                Width::B8 => v,
                Width::B4 => LLVMBuildSExt(self.b, v, self.i64, c"".as_ptr()),
                _ => LLVMBuildZExt(self.b, v, self.i64, c"".as_ptr()),
            }
        }
    }

    /// Write slot variable `k` (widened as `vslot_load` reads it) to the frame.
    unsafe fn vslot_store(&self, c: &Cur, k: usize, v: LLVMValueRef) {
        // SAFETY: as `vslot_ptr`.
        unsafe { self.vslot_store_at(c, k, v, self.tag(self.own_slots(c), AA_SLOTS)) }
    }

    /// `vslot_store` with the frame's slots at `base`.
    unsafe fn vslot_store_at(&self, c: &Cur, k: usize, v: LLVMValueRef, base: LLVMValueRef) {
        let (off, w) = c.scal.as_ref().expect("slot variables").slots[k];
        // SAFETY: the slot lies within the frame (`Scalars::new`).
        unsafe {
            let p = self.gep(base, self.c64(i64::from(off)));
            let v = if w == Width::B8 {
                v
            } else {
                LLVMBuildTrunc(self.b, v, self.width_ty(w), c"".as_ptr())
            };
            self.store(v, p);
        }
    }

    /// The value of slot variable `k` here: the deferred one, else the frame's (kept for the block).
    unsafe fn vslot_value(&self, c: &Cur, k: usize) -> LLVMValueRef {
        let nregs = c.known.borrow().len();
        let d = c.defer.as_ref().expect("slot variables");
        if let Some(v) = d.pend.borrow()[nregs + k] {
            return v;
        }
        // SAFETY: as `vslot_load`.
        let v = unsafe { self.vslot_load(c, k, true) };
        d.pend.borrow_mut()[nregs + k] = Some(v);
        v
    }

    /// The builder is at the start of the block of `pc`: what its phis hold is in registers.
    fn begin_block(&self, c: &Cur, pc: usize) {
        self.drop_pend(c);
        if let Some(d) = &c.defer
            && let Some(pl) = &d.plan
        {
            if pl.single[pc] {
                if let Some(p) = pl.inherit.borrow_mut()[pc].take() {
                    *d.pend.borrow_mut() = p;
                }
                return;
            }
            let mut pend = d.pend.borrow_mut();
            for (&r, &phi) in pl.regs[pc].iter().zip(&pl.phis[pc]) {
                pend[r as usize] = Some(phi);
            }
        }
    }

    /// Forget the deferred registers (the frame has them all, or nothing reaches the code).
    fn drop_pend(&self, c: &Cur) {
        if let Some(d) = &c.defer {
            d.pend.borrow_mut().fill(None);
        }
    }

    unsafe fn zext(&self, v: LLVMValueRef) -> LLVMValueRef {
        // SAFETY: v is an i1.
        unsafe { LLVMBuildZExt(self.b, v, self.i64, c"".as_ptr()) }
    }

    unsafe fn icmp(&self, p: IP, a: LLVMValueRef, b: LLVMValueRef) -> LLVMValueRef {
        // SAFETY: a and b have one integer type.
        unsafe { LLVMBuildICmp(self.b, p, a, b, c"".as_ptr()) }
    }

    unsafe fn or(&self, a: LLVMValueRef, b: LLVMValueRef) -> LLVMValueRef {
        // SAFETY: two i1.
        unsafe { LLVMBuildOr(self.b, a, b, c"".as_ptr()) }
    }

    /// Leave to the interpreter at `pc` when `bad` holds; the builder continues after it.
    unsafe fn exit_if(&self, c: &Cur, bad: LLVMValueRef, pc: u32) {
        // SAFETY: well-formed blocks of c.f.
        unsafe {
            let out = self.block(c);
            let cont = self.block(c);
            LLVMBuildCondBr(self.b, bad, out, cont);
            LLVMPositionBuilderAtEnd(self.b, out);
            self.flush_exit(c, pc);
            LLVMPositionBuilderAtEnd(self.b, cont);
        }
    }

    /// `intrinsic(a, b)` with its overflow flag: (value, overflowed).
    unsafe fn ovf(
        &self,
        (ty, f): (LLVMTypeRef, LLVMValueRef),
        a: LLVMValueRef,
        b: LLVMValueRef,
    ) -> (LLVMValueRef, LLVMValueRef) {
        // SAFETY: the intrinsic takes two i64.
        unsafe {
            let mut args = [a, b];
            let r = LLVMBuildCall2(self.b, ty, f, args.as_mut_ptr(), 2, c"".as_ptr());
            (
                LLVMBuildExtractValue(self.b, r, 0, c"".as_ptr()),
                LLVMBuildExtractValue(self.b, r, 1, c"".as_ptr()),
            )
        }
    }

    /// `x op y` as `yavs_std::arith`, leaving at `pc` wherever it reports an error.
    /// `None` for `**`, which goes through the interpreter.
    unsafe fn int_op(
        &self,
        c: &Cur,
        op: IntOp,
        x: LLVMValueRef,
        y: LLVMValueRef,
        pc: u32,
    ) -> Option<LLVMValueRef> {
        // SAFETY: well-formed IR in the current block.
        unsafe {
            let checked = |i| {
                let (v, o) = self.ovf(i, x, y);
                self.exit_if(c, o, pc);
                v
            };
            Some(match op {
                IntOp::Add => checked(self.sadd),
                IntOp::Sub => checked(self.ssub),
                IntOp::Mul => checked(self.smul),
                IntOp::Div | IntOp::Rem | IntOp::Mod => {
                    let zero = self.icmp(IP::LLVMIntEQ, y, self.c64(0));
                    let m1 = self.icmp(IP::LLVMIntEQ, y, self.c64(-1));
                    if op == IntOp::Div {
                        let min = self.icmp(IP::LLVMIntEQ, x, self.c64(i64::MIN));
                        let o = LLVMBuildAnd(self.b, min, m1, c"".as_ptr());
                        self.exit_if(c, self.or(zero, o), pc);
                        return Some(LLVMBuildSDiv(self.b, x, y, c"".as_ptr()));
                    }
                    self.exit_if(c, zero, pc);
                    // x rem -1 is 0; srem would trap on MIN rem -1.
                    let ys = LLVMBuildSelect(self.b, m1, self.c64(1), y, c"".as_ptr());
                    let r = LLVMBuildSRem(self.b, x, ys, c"".as_ptr());
                    if op == IntOp::Rem {
                        return Some(r);
                    }
                    let nz = self.icmp(IP::LLVMIntNE, r, self.c64(0));
                    let rn = self.icmp(IP::LLVMIntSLT, r, self.c64(0));
                    let yn = self.icmp(IP::LLVMIntSLT, y, self.c64(0));
                    let diff = LLVMBuildXor(self.b, rn, yn, c"".as_ptr());
                    let fix = LLVMBuildAnd(self.b, nz, diff, c"".as_ptr());
                    let sum = LLVMBuildAdd(self.b, r, y, c"".as_ptr());
                    LLVMBuildSelect(self.b, fix, sum, r, c"".as_ptr())
                }
                IntOp::Pow => return None,
            })
        }
    }

    /// Generate `name` for `f`; on a verifier failure the function is deleted again.
    /// The type of `f`'s native-frame code: `(env, mem, depth, argument cells...) -> i64`.
    unsafe fn native_ty(&self, f: &Func) -> LLVMTypeRef {
        let cells: usize = f
            .params
            .iter()
            .map(|p| 1 + usize::from(p.width == Width::Sig))
            .sum();
        let mut params = vec![self.ptr, self.ptr, self.i64];
        if f.parent.is_some() {
            params.extend([self.i64, self.i64]);
        }
        params.extend(std::iter::repeat_n(self.i64, cells));
        // SAFETY: types of our context.
        unsafe { LLVMFunctionType(self.i64, params.as_mut_ptr(), params.len() as u32, 0) }
    }

    /// The function named `name` with type `ty`, declared if not yet in the module.
    unsafe fn declare(&self, name: &str, ty: LLVMTypeRef) -> LLVMValueRef {
        let cname = CString::new(name).expect("no NUL");
        // SAFETY: a function of our module.
        unsafe {
            let f = LLVMGetNamedFunction(self.module, cname.as_ptr());
            if f.is_null() {
                LLVMAddFunction(self.module, cname.as_ptr(), ty)
            } else {
                f
            }
        }
    }

    /// Generate `f` (`id`) as `name`: its frame-mode code, or with `native` its native-frame
    /// code (`docs/decisions.md` "Compiled frames"). Calls of functions in `nc` are native
    /// calls.
    #[allow(clippy::too_many_arguments)]
    unsafe fn func(
        &self,
        prog: &Program,
        f: &Func,
        id: FuncId,
        name: &str,
        native: bool,
        nc: &std::collections::HashSet<FuncId>,
    ) -> Result<(), String> {
        // SAFETY: all values are built in this function with matching types.
        unsafe {
            let fty = if native {
                self.native_ty(f)
            } else {
                self.fn_ty
            };
            let fv = self.declare(name, fty);
            // `regs` and `mem` overlap no other memory the function reaches (frame slots,
            // temporaries, objects), so LLVM may keep their values in registers between
            // calls; the callbacks get both pointers.
            let kind = LLVMGetEnumAttributeKindForName(c"noalias".as_ptr(), 7);
            let restrict: &[u32] = if native { &[2] } else { &[2, 3] };
            for &i in restrict {
                LLVMAddAttributeAtIndex(fv, i, LLVMCreateEnumAttribute(self.ctx, kind, 0));
            }
            let exit = LLVMAppendBasicBlockInContext(self.ctx, fv, c"".as_ptr());
            LLVMPositionBuilderAtEnd(self.b, exit);
            let exit_pc = LLVMBuildPhi(self.b, self.i32, c"".as_ptr());
            if !native {
                LLVMBuildRet(self.b, exit_pc);
            }
            let entry = LLVMAppendBasicBlockInContext(self.ctx, fv, c"".as_ptr());
            LLVMMoveBasicBlockBefore(entry, exit);
            LLVMPositionBuilderAtEnd(self.b, entry);
            let nregs = f.reg_ty.len();
            let (regs, mem, nat) = if native {
                let rty = LLVMArrayType2(self.i64, nregs.max(1) as u64);
                let regs = LLVMBuildAlloca(self.b, rty, c"".as_ptr());
                let sty = LLVMArrayType2(self.i64, (u64::from(f.frame)).div_ceil(8).max(1));
                let slots = LLVMBuildAlloca(self.b, sty, c"".as_ptr());
                let scratch = LLVMBuildAlloca(self.b, self.i64, c"".as_ptr());
                let tbase = LLVMBuildAlloca(self.b, self.i64, c"".as_ptr());
                let depth = LLVMGetParam(fv, 2);
                let tag = self.c64((u64::from(space::FRAME) << 29) as i64);
                let fhi = LLVMBuildOr(self.b, tag, depth, c"".as_ptr());
                let (up1, up2) = if f.parent.is_some() {
                    (LLVMGetParam(fv, 3), LLVMGetParam(fv, 4))
                } else {
                    (self.c64(-1), self.c64(-1))
                };
                let nat = Nat {
                    slots,
                    fhi,
                    depth,
                    up: [up1, up2],
                    temp_base: tbase,
                    scratch,
                    id,
                };
                (regs, LLVMGetParam(fv, 1), Some(nat))
            } else {
                (LLVMGetParam(fv, 1), LLVMGetParam(fv, 2), None)
            };
            let mut c = Cur {
                prog,
                consts: Default::default(),
                f: fv,
                env: LLVMGetParam(fv, 0),
                regs,
                mem,
                known: std::cell::RefCell::new(vec![None; f.reg_ty.len()]),
                fixed: fixed_consts(f),
                exits: std::cell::RefCell::new(if native {
                    Vec::new()
                } else {
                    vec![(exit, exit_pc, 0)]
                }),
                exit: (exit, exit_pc),
                frame_len: u64::from(f.frame),
                nat,
                defer: None,
                scal: None,
                joins: Default::default(),
            };
            // Registers the interpreter starts the frame with hold the same address everywhere.
            let mut base_known: Vec<Option<Known>> = vec![None; nregs];
            if !native {
                let pre = crate::regs::preloaded(f);
                for i in &f.code {
                    if let Inst::AddrOf { dst, space, off } = *i
                        && pre.get(dst.0 as usize) == Some(&true)
                    {
                        base_known[dst.0 as usize] = match space {
                            AddrSpace::Frame => Some(Known::Frame(off.into())),
                            AddrSpace::Const(k) => Some(Known::Const(k, off.into())),
                            AddrSpace::Inst => Some(Known::Inst(off.into())),
                            _ => None,
                        };
                    }
                }
                *c.known.borrow_mut() = base_known.clone();
            }
            let n = f.code.len();
            // Where the interpreter enters: the start and what follows a call, a wait or a report.
            let mut e0 = vec![false; n + 1];
            e0[0] = true;
            for (i, inst) in f.code.iter().enumerate() {
                if resumes(inst) {
                    e0[i + 1] = true;
                }
            }
            let rc = if native {
                None
            } else {
                let roots: Vec<usize> = (0..n).filter(|&i| e0[i]).collect();
                crate::regs::reach(prog, f, &roots)
            };
            let scal = rc.as_ref().and_then(|r| {
                crate::regs::Scalars::new(f, r, &|i| match *i {
                    Inst::Call { func, args, .. } => {
                        nc.contains(&func)
                            && prog
                                .func(func)
                                .is_some_and(|g| call_fits(f, id, g, args, false))
                    }
                    _ => false,
                })
            });
            if !native && let Some(live) = crate::regs::Live::new(prog, f, scal.as_ref()) {
                c.defer = Some(Defer {
                    live,
                    pre: crate::regs::preloaded(f),
                    // Registers, slot variables, and the base of the frame's slots last.
                    pend: std::cell::RefCell::new(vec![
                        None;
                        nregs
                            + scal
                                .as_ref()
                                .map_or(0, |s| s.slots.len())
                            + 1
                    ]),
                    fresh: Default::default(),
                    pc: Default::default(),
                    plan: None,
                    loopy: {
                        let mut l = vec![false; n + 1];
                        for &x in rc.iter().flat_map(|r| r.loops.iter().flatten()) {
                            l[x] = true;
                        }
                        l
                    },
                });
                c.scal = scal;
            }
            // Entry points: 0, block starts, and the instruction after each that may leave
            // frames to the interpreter or suspend.
            let mut leader = vec![false; n + 1];
            leader[0] = true;
            for (_, b) in f.blocks.iter() {
                if let Some(l) = leader.get_mut(b.start as usize) {
                    *l = true;
                }
            }
            // What the generator knows about registers holds until the next block: the
            // interpreter enters after a call only having run the block up to it.
            let block_start = leader.clone();
            for (i, inst) in f.code.iter().enumerate() {
                if resumes(inst) {
                    leader[i + 1] = true;
                }
            }
            // The interpreter enters a function after the constants and addresses it preloads
            // into the frame's registers (`Pre::next`): the pcs after such definitions at a
            // leader are entry points too (what is known there is what the registers hold).
            let preload_at = |i: usize| match f.code[i] {
                Inst::Const { .. } | Inst::ConstF { .. } => true,
                Inst::AddrOf { space, .. } => !matches!(space, AddrSpace::Up(_)),
                _ => false,
            };
            let mut enters = leader.clone();
            if !native {
                for i in 1..n {
                    enters[i] |= enters[i - 1] && preload_at(i - 1);
                }
            }
            // With registers carried across blocks (`Plan`) only the places the interpreter
            // can usefully enter at get an entry stub: the start, what follows a call, a wait or
            // a report, and loop headers (a hand-over comes at a jump, and the interpreter runs
            // on to the next one when the compiled code has no entry at the one it lands on).
            let mut plan_regs: Option<Vec<Vec<u32>>> = None;
            let mut plan_single: Vec<bool> = Vec::new();
            let mut is_entry = enters.clone();
            if !native
                && let Some(d) = &c.defer
                && let Some(rc) = &rc
            {
                let mut e = e0.clone();
                {
                    let (reached, head) = (&rc.reached, &rc.head);
                    for (_, b) in f.blocks.iter() {
                        if let Some(h) = e.get_mut(b.start as usize) {
                            *h |= head.get(b.start as usize) == Some(&true);
                        }
                    }
                    for i in 1..n {
                        e[i] |= e[i - 1] && preload_at(i - 1);
                    }
                    let mut at = vec![false; n + 1];
                    for i in 0..n {
                        at[i] = leader[i] || e[i];
                    }
                    let mut total = 0;
                    let deferrable = crate::regs::deferrable(f);
                    let mut loopy = vec![false; n + 1];
                    for &x in rc.loops.iter().flatten() {
                        loopy[x] = true;
                    }
                    let has_frame = f.code.iter().any(|i| {
                        matches!(
                            i,
                            Inst::AddrOf {
                                space: AddrSpace::Frame,
                                ..
                            }
                        )
                    });
                    let mut npred = vec![0u32; n + 1];
                    let mut sole = vec![usize::MAX; n + 1];
                    for j in (0..n).filter(|&j| reached[j]) {
                        for t in crate::regs::succs(prog, f, j, &f.code[j]).unwrap_or_default() {
                            npred[t as usize] += 1;
                            sole[t as usize] = j;
                        }
                    }
                    let single: Vec<bool> = (0..=n)
                        .map(|i| {
                            i < n && at[i] && reached[i] && !e[i] && npred[i] == 1 && sole[i] < i
                        })
                        .collect();
                    plan_single = single.clone();
                    let regs: Vec<Vec<u32>> = (0..n)
                        .map(|i| {
                            // Phis only in loops: elsewhere a block takes its registers from the frame.
                            if !at[i] || !reached[i] || single[i] || !loopy[i] {
                                return Vec::new();
                            }
                            let row = d.live.row(i);
                            let mut v: Vec<u32> = (0..d.pend.borrow().len() as u32 - 1)
                                .filter(|&r| {
                                    row[r as usize / 64] >> (r % 64) & 1 != 0
                                        && (r as usize >= nregs
                                            || (deferrable[r as usize]
                                                && !d.pre[r as usize]
                                                && c.fixed[r as usize].is_none()))
                                })
                                .collect();
                            // The base of the slots is carried through the loops of a function
                            // that has frame variables.
                            if has_frame {
                                v.push(d.pend.borrow().len() as u32 - 1);
                            }
                            total += v.len();
                            v
                        })
                        .collect();
                    if total <= MAX_PHIS {
                        enters = at;
                        is_entry = e;
                        plan_regs = Some(regs);
                    }
                }
            }
            let mut bbs = vec![std::ptr::null_mut(); n];
            for (i, bb) in bbs.iter_mut().enumerate() {
                if enters[i] {
                    *bb = self.block(&c);
                }
            }
            let mut plan = None;
            if let Some(regs) = plan_regs {
                let phis = regs
                    .iter()
                    .enumerate()
                    .map(|(i, rs)| {
                        if rs.is_empty() {
                            return Vec::new();
                        }
                        LLVMPositionBuilderAtEnd(self.b, bbs[i]);
                        let base = c
                            .defer
                            .as_ref()
                            .map_or(0, |d| d.pend.borrow().len() as u32 - 1);
                        rs.iter()
                            .map(|&r| {
                                let ty = if r == base { self.ptr } else { self.i64 };
                                LLVMBuildPhi(self.b, ty, c"".as_ptr())
                            })
                            .collect()
                    })
                    .collect();
                plan = Some(Plan {
                    regs,
                    phis,
                    inherit: std::cell::RefCell::new(vec![None; plan_single.len()]),
                    single: plan_single,
                });
            }
            if native && bbs.is_empty() {
                return Err("a function without instructions".into());
            }
            if native {
                // The frame as the interpreter's push makes it: zeroed slots, the arguments
                // in the parameter slots.
                let nat = c.nat.as_ref().expect("native");
                let slots = nat.slots;
                LLVMBuildMemSet(
                    self.b,
                    slots,
                    LLVMConstInt(self.i8, 0, 0),
                    self.c64(i64::from(f.frame)),
                    8,
                );
                let mut k = if f.parent.is_some() { 5 } else { 3 };
                for p in &f.params {
                    let at = self.gep(slots, self.c64(i64::from(p.slot)));
                    let v = LLVMGetParam(fv, k);
                    k += 1;
                    match p.width {
                        Width::Sig => {
                            self.store(v, at);
                            self.store(LLVMGetParam(fv, k), self.gep(at, self.c64(8)));
                            k += 1;
                        }
                        Width::B8 => self.store(v, at),
                        w => self.store(
                            LLVMBuildTrunc(self.b, v, self.width_ty(w), c"".as_ptr()),
                            at,
                        ),
                    }
                }
                let entry_len = LLVMBuildLoad2(
                    self.b,
                    self.i64,
                    self.gep(mem, self.c64(MEM_TEMP_LEN as i64)),
                    c"".as_ptr(),
                );
                LLVMBuildStore(self.b, entry_len, nat.temp_base);
                // Callees reach these slots through their frame index.
                let tab = LLVMBuildLoad2(
                    self.b,
                    self.ptr,
                    self.gep(mem, self.c64(MEM_NSLOTS as i64)),
                    c"".as_ptr(),
                );
                let ent = self.gep(
                    tab,
                    LLVMBuildMul(self.b, nat.depth, self.c64(16), c"".as_ptr()),
                );
                self.store(slots, ent);
                self.store(self.c64(i64::from(f.frame)), self.gep(ent, self.c64(8)));
                for (r, addr, v) in preloads(f) {
                    let x = match addr {
                        None => self.c64(v),
                        Some((AddrSpace::Frame, off)) => {
                            let hi = LLVMBuildShl(self.b, nat.fhi, self.c64(32), c"".as_ptr());
                            LLVMBuildOr(self.b, hi, self.c64(i64::from(off)), c"".as_ptr())
                        }
                        Some((sp, off)) => match addr_const(sp, off) {
                            Some(a) => self.c64(a as i64),
                            None => continue,
                        },
                    };
                    self.st(&c, r, x);
                }
                LLVMBuildBr(self.b, bbs[0]);
            } else {
                let pc_in = LLVMGetParam(fv, 3);
                let other = self.block(&c);
                LLVMPositionBuilderAtEnd(self.b, other);
                LLVMBuildRet(self.b, pc_in);
                LLVMPositionBuilderAtEnd(self.b, entry);
                let nl = is_entry[..n].iter().filter(|&&l| l).count();
                let sw = LLVMBuildSwitch(self.b, pc_in, other, nl as u32);
                for (i, &bb) in bbs.iter().enumerate() {
                    if !is_entry[i] {
                        continue;
                    }
                    // The interpreter's registers are in the frame: a stub loads the ones the
                    // block takes as phis.
                    let Some(pl) = &plan else {
                        LLVMAddCase(sw, self.c32(i as u32), bb);
                        continue;
                    };
                    let stub = self.block(&c);
                    LLVMAddCase(sw, self.c32(i as u32), stub);
                    LLVMPositionBuilderAtEnd(self.b, stub);
                    for (&r, &phi) in pl.regs[i].iter().zip(&pl.phis[i]) {
                        let v = self.frame_load(&c, r);
                        LLVMAddIncoming(phi, &mut { v }, &mut { stub }, 1);
                    }
                    LLVMBuildBr(self.b, bb);
                }
            }
            if let Some(d) = &mut c.defer {
                d.plan = plan;
            }
            let c = c;
            let block_bb = |b: yavs_ir::BlockId| -> Result<LLVMBasicBlockRef, String> {
                f.blocks
                    .get(b)
                    .and_then(|b| bbs.get(b.start as usize).copied())
                    .filter(|bb| !bb.is_null())
                    .ok_or_else(|| format!("branch to missing block {}", b.0))
            };
            let start_of = |b: yavs_ir::BlockId| -> Result<usize, String> {
                f.blocks
                    .get(b)
                    .map(|b| b.start as usize)
                    .ok_or_else(|| format!("branch to missing block {}", b.0))
            };
            let mut open = false;
            let mut prev = None;
            for (i, &inst) in f.code.iter().enumerate() {
                let pc = i as u32;
                if let Some(p) = prev.replace(inst) {
                    track(&mut c.known.borrow_mut(), &c.fixed, &p);
                    // What the previous instruction defined and did not defer is in the frame.
                    if let Some(d) = &c.defer {
                        let fresh = std::mem::take(&mut *d.fresh.borrow_mut());
                        let mut pend = d.pend.borrow_mut();
                        p.visit(&mut |r, role, _| {
                            if role == yavs_ir::Role::Def {
                                let sig = f.reg_ty.get(r.0 as usize) == Some(&yavs_ir::RegTy::Sig);
                                for k in [r.0, r.0 + 1].into_iter().take(1 + usize::from(sig)) {
                                    if !fresh.contains(&k)
                                        && let Some(x) = pend.get_mut(k as usize)
                                    {
                                        *x = None;
                                    }
                                }
                            }
                        });
                    }
                }
                if let Some(d) = &c.defer {
                    d.pc.set(pc);
                }
                if block_start[i] {
                    c.known.borrow_mut().clone_from(&base_known);
                }
                // What is known about an address register (`Gen::host`).
                let fk = |r: Reg| self.known_addr(&c, r);
                if enters[i] {
                    if open {
                        let snap = self.edge_prep(&c, &[i]);
                        self.edge_to(&c, &snap, i);
                        LLVMBuildBr(self.b, bbs[i]);
                    }
                    LLVMPositionBuilderAtEnd(self.b, bbs[i]);
                    self.begin_block(&c, i);
                    open = true;
                } else if !open {
                    // Dead code after a terminator: give it a block, LLVM drops it.
                    let bb = self.block(&c);
                    LLVMPositionBuilderAtEnd(self.b, bb);
                    self.drop_pend(&c);
                    open = true;
                }
                // The interpreter returns (`TierHost::ret` for a frame compiled code pushed):
                // compiled code leaves the frame to it there.
                if let Inst::Ret { v } = inst {
                    match &c.nat {
                        // A native frame returns its value itself.
                        Some(_) => {
                            let v = v.map_or_else(|| self.c64(0), |r| self.ld(&c, r));
                            LLVMBuildRet(self.b, v);
                        }
                        None => {
                            self.flush(&c, i);
                            self.leave(&c, self.c32(pc))
                        }
                    }
                    open = false;
                    continue;
                }
                match inst {
                    Inst::Const { dst, v } => self.std(&c, dst, self.c64(v)),
                    Inst::ConstF { dst, v } => self.std(&c, dst, self.c64(v.to_bits() as i64)),
                    Inst::Move { dst, src } => self.std(&c, dst, self.ld(&c, src)),
                    Inst::Select { dst, c: cond, a, b } => {
                        let t = self.icmp(IP::LLVMIntNE, self.ld(&c, cond), self.c64(0));
                        let v = LLVMBuildSelect(
                            self.b,
                            t,
                            self.ld(&c, a),
                            self.ld(&c, b),
                            c"".as_ptr(),
                        );
                        self.std(&c, dst, v);
                    }
                    Inst::Bin { op, dst, a, b } => {
                        let (x, y) = (self.ld(&c, a), self.ld(&c, b));
                        match self.int_op(&c, op, x, y, pc) {
                            Some(v) => self.std(&c, dst, v),
                            None => self.slow_inst(&c, pc, &inst, f)?,
                        }
                    }
                    Inst::BinRange {
                        op,
                        dst,
                        a,
                        b,
                        lo,
                        hi,
                        ..
                    } => {
                        let (x, y) = (self.ld(&c, a), self.ld(&c, b));
                        match self.int_op(&c, op, x, y, pc) {
                            Some(v) => {
                                self.range_exit(&c, v, self.ld(&c, lo), self.ld(&c, hi), pc);
                                self.std(&c, dst, v);
                            }
                            None => self.slow_inst(&c, pc, &inst, f)?,
                        }
                    }
                    Inst::Un { op, dst, a } => {
                        let x = self.ld(&c, a);
                        let min = self.icmp(IP::LLVMIntEQ, x, self.c64(i64::MIN));
                        self.exit_if(&c, min, pc);
                        let neg = LLVMBuildNeg(self.b, x, c"".as_ptr());
                        let v = match op {
                            IntUnOp::Neg => neg,
                            IntUnOp::Abs => {
                                let lt = self.icmp(IP::LLVMIntSLT, x, self.c64(0));
                                LLVMBuildSelect(self.b, lt, neg, x, c"".as_ptr())
                            }
                        };
                        self.std(&c, dst, v);
                    }
                    Inst::FBin { op, dst, a, b } => {
                        let (x, y) = (self.ldf(&c, a), self.ldf(&c, b));
                        let v = match op {
                            FOp::Add => LLVMBuildFAdd(self.b, x, y, c"".as_ptr()),
                            FOp::Sub => LLVMBuildFSub(self.b, x, y, c"".as_ptr()),
                            FOp::Mul => LLVMBuildFMul(self.b, x, y, c"".as_ptr()),
                            FOp::Div => {
                                let z = LLVMBuildFCmp(
                                    self.b,
                                    RP::LLVMRealOEQ,
                                    y,
                                    LLVMConstReal(self.f64, 0.0),
                                    c"".as_ptr(),
                                );
                                self.exit_if(&c, z, pc);
                                LLVMBuildFDiv(self.b, x, y, c"".as_ptr())
                            }
                            FOp::PowI | FOp::Pow => {
                                self.slow_inst(&c, pc, &inst, f)?;
                                continue;
                            }
                        };
                        // Not finite: |v| is not below infinity (NaN compares false).
                        let fin = LLVMBuildFCmp(
                            self.b,
                            RP::LLVMRealUGE,
                            self.fabs(v),
                            LLVMConstReal(self.f64, f64::INFINITY),
                            c"".as_ptr(),
                        );
                        self.exit_if(&c, fin, pc);
                        self.stfd(&c, dst, v);
                    }
                    Inst::FUn { op, dst, a } => {
                        let x = self.ldf(&c, a);
                        let v = match op {
                            FUnOp::Neg => LLVMBuildFNeg(self.b, x, c"".as_ptr()),
                            FUnOp::Abs => self.fabs(x),
                        };
                        self.stfd(&c, dst, v);
                    }
                    Inst::IToF { dst, src } => {
                        let v = LLVMBuildSIToFP(self.b, self.ld(&c, src), self.f64, c"".as_ptr());
                        self.stfd(&c, dst, v);
                    }
                    Inst::Cmp { op, dst, a, b } => {
                        let p = match op {
                            CmpOp::Eq => IP::LLVMIntEQ,
                            CmpOp::Ne => IP::LLVMIntNE,
                            CmpOp::Lt => IP::LLVMIntSLT,
                            CmpOp::Le => IP::LLVMIntSLE,
                            CmpOp::Gt => IP::LLVMIntSGT,
                            CmpOp::Ge => IP::LLVMIntSGE,
                        };
                        let v = self.icmp(p, self.ld(&c, a), self.ld(&c, b));
                        self.std(&c, dst, self.zext(v));
                    }
                    Inst::FCmp { op, dst, a, b } => {
                        // Ordered: every comparison with a NaN is false, as `partial_cmp`.
                        let p = match op {
                            CmpOp::Eq => RP::LLVMRealOEQ,
                            CmpOp::Ne => RP::LLVMRealONE,
                            CmpOp::Lt => RP::LLVMRealOLT,
                            CmpOp::Le => RP::LLVMRealOLE,
                            CmpOp::Gt => RP::LLVMRealOGT,
                            CmpOp::Ge => RP::LLVMRealOGE,
                        };
                        let (x, y) = (self.ldf(&c, a), self.ldf(&c, b));
                        let v = LLVMBuildFCmp(self.b, p, x, y, c"".as_ptr());
                        self.std(&c, dst, self.zext(v));
                    }
                    Inst::Bool { op, dst, a, b } => {
                        let x = self.icmp(IP::LLVMIntNE, self.ld(&c, a), self.c64(0));
                        let v = if op == BoolOp::Not {
                            LLVMBuildNot(self.b, x, c"".as_ptr())
                        } else {
                            let y = self.icmp(IP::LLVMIntNE, self.ld(&c, b), self.c64(0));
                            match op {
                                BoolOp::And => LLVMBuildAnd(self.b, x, y, c"".as_ptr()),
                                BoolOp::Or => self.or(x, y),
                                _ => LLVMBuildXor(self.b, x, y, c"".as_ptr()),
                            }
                        };
                        self.std(&c, dst, self.zext(v));
                    }
                    Inst::Check {
                        kind, v, lo, hi, ..
                    } => {
                        let x = self.ld(&c, v);
                        match kind {
                            CheckKind::Range | CheckKind::Index => {
                                self.range_exit(&c, x, self.ld(&c, lo), self.ld(&c, hi), pc)
                            }
                            CheckKind::Length => {
                                let ne = self.icmp(IP::LLVMIntNE, x, self.ld(&c, lo));
                                self.exit_if(&c, ne, pc);
                            }
                            CheckKind::Null => {
                                let z = self.icmp(IP::LLVMIntEQ, x, self.c64(0));
                                self.exit_if(&c, z, pc);
                            }
                        }
                    }
                    Inst::FCheck { v, lo, hi, .. } => {
                        let (x, l, h) = (self.ldf(&c, v), self.ldf(&c, lo), self.ldf(&c, hi));
                        // !(l <= x && x <= h), NaN included.
                        let a = LLVMBuildFCmp(self.b, RP::LLVMRealUGT, l, x, c"".as_ptr());
                        let b = LLVMBuildFCmp(self.b, RP::LLVMRealUGT, x, h, c"".as_ptr());
                        self.exit_if(&c, self.or(a, b), pc);
                    }
                    Inst::Index {
                        dst,
                        base,
                        pos,
                        stride,
                    } => {
                        // Address offset in the low 32 bits; anything over 4 GiB is the
                        // interpreter's error.
                        let a = self.ld(&c, base);
                        let (off, o1) =
                            self.ovf(self.umul, self.ld(&c, pos), self.c64(i64::from(stride)));
                        let lo = LLVMBuildAnd(self.b, a, self.c64(0xffff_ffff), c"".as_ptr());
                        let (sum, o2) = self.ovf(self.uadd, lo, off);
                        let big = self.icmp(IP::LLVMIntUGT, sum, self.c64(0xffff_ffff));
                        self.exit_if(&c, self.or(self.or(o1, o2), big), pc);
                        let hi = LLVMBuildAnd(self.b, a, self.c64(!0xffff_ffff), c"".as_ptr());
                        self.st(&c, dst, LLVMBuildOr(self.b, hi, sum, c"".as_ptr()));
                    }
                    Inst::AddrOf { dst, space, off } => match addr_const(space, off) {
                        Some(a) if space == AddrSpace::Frame => {
                            let hi = self.own_hi(&c);
                            let hi = LLVMBuildShl(self.b, hi, self.c64(32), c"".as_ptr());
                            self.std(
                                &c,
                                dst,
                                LLVMBuildOr(self.b, hi, self.c64(a as i64), c"".as_ptr()),
                            );
                        }
                        Some(a) => self.std(&c, dst, self.c64(a as i64)),
                        // A frame one or two static links up, from the view.
                        None => match space {
                            AddrSpace::Up(d @ 1..=2) => {
                                let (slow, join) = self.slow_path(&c, pc, &inst);
                                let hi = match &c.nat {
                                    Some(n) => n.up[usize::from(d - 1)],
                                    None => {
                                        self.mem_ld(&c, MEM_UP + 8 * u64::from(d - 1), self.i64)
                                    }
                                };
                                let ok = self.icmp(IP::LLVMIntNE, hi, self.c64(-1));
                                self.guard(&c, ok, slow);
                                let hi = LLVMBuildShl(self.b, hi, self.c64(32), c"".as_ptr());
                                let a = LLVMBuildOr(self.b, hi, self.c64(off.into()), c"".as_ptr());
                                self.st(&c, dst, a);
                                self.join(&c, join);
                            }
                            _ => self.step(&c, pc, &inst),
                        },
                    },
                    // A fat pair is recorded only when a word points into a heap
                    // (`Machine::fat_root`); the helper says when none does.
                    Inst::TempRoot { addr, ty: None, .. } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        // A native frame records the roots itself where the interpreter would.
                        let miss = if c.nat.is_some() {
                            let here = LLVMGetInsertBlock(self.b);
                            let rec = self.block(&c);
                            LLVMPositionBuilderAtEnd(self.b, rec);
                            let null = LLVMConstPointerNull(self.ptr);
                            let ok = self.troot_call(&c, self.ld(&c, addr), u32::MAX, null, 0);
                            LLVMBuildCondBr(
                                self.b,
                                self.icmp(IP::LLVMIntNE, ok, self.c32(0)),
                                join,
                                slow,
                            );
                            LLVMPositionBuilderAtEnd(self.b, here);
                            rec
                        } else {
                            slow
                        };
                        self.fast_op(&c, FAST_FAT_ROOT, 0, self.ld(&c, addr), self.c64(0), miss);
                        self.join(&c, join);
                    }
                    Inst::Stmt => {
                        // With recorded temporaries a native frame drops them itself.
                        let (slow, join) = if c.nat.is_some() {
                            let (join, here) = (self.block(&c), LLVMGetInsertBlock(self.b));
                            let rec = self.block(&c);
                            LLVMPositionBuilderAtEnd(self.b, rec);
                            let mut a = [c.env, c.mem, self.temp_base(&c)];
                            LLVMBuildCall2(
                                self.b,
                                self.endstmt_ty,
                                self.endstmt,
                                a.as_mut_ptr(),
                                3,
                                c"".as_ptr(),
                            );
                            LLVMBuildBr(self.b, join);
                            LLVMPositionBuilderAtEnd(self.b, here);
                            (rec, join)
                        } else {
                            self.slow_path(&c, pc, &inst)
                        };
                        let roots = self.mem_ld(&c, MEM_TEMP_ROOTS, self.i64);
                        self.guard(&c, self.icmp(IP::LLVMIntEQ, roots, self.c64(0)), slow);
                        let len = self.mem_ld(&c, MEM_TEMP_LEN, self.i64);
                        let base = self.temp_base(&c);
                        let lt = self.icmp(IP::LLVMIntULT, base, len);
                        let v = LLVMBuildSelect(self.b, lt, base, len, c"".as_ptr());
                        self.mem_st(&c, MEM_TEMP_LEN, v);
                        self.join(&c, join);
                    }
                    Inst::Alloc { dst, size } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let (a, _) = self.temp_alloc(&c, self.ld(&c, size), slow);
                        self.st(&c, dst, a);
                        self.join(&c, join);
                    }
                    Inst::LayoutOf { dst, ty, desc } => match static_layout(prog, ty) {
                        Some(StaticLayout::Array { dims, elem, logic }) if c.nat.is_none() => {
                            // `yavs_rt_layout` does what `Ty::layout` would; 0 is the interpreter's.
                            let (slow, join) = self.slow_path(&c, pc, &inst);
                            let stride = elem.size.next_multiple_of(elem.align.max(1));
                            let (rp, d) = self.reg_out(&c, dst);
                            let mut args = [
                                c.env,
                                rp,
                                c.mem,
                                self.ld(&c, desc),
                                self.c32(d),
                                self.c32(u32::from(dims)),
                                self.c64(stride as i64),
                                self.c64(elem.align as i64),
                                self.c64(elem.count as i64),
                                self.c32(u32::from(logic)),
                            ];
                            let ok = LLVMBuildCall2(
                                self.b,
                                self.layout_ty,
                                self.layout,
                                args.as_mut_ptr(),
                                10,
                                c"".as_ptr(),
                            );
                            self.guard(&c, self.icmp(IP::LLVMIntNE, ok, self.c32(0)), slow);
                            self.reg_in(&c, dst);
                            self.join(&c, join);
                        }
                        Some(sl) => {
                            let (slow, join) = self.slow_path(&c, pc, &inst);
                            let words = self.layout_words(&c, &sl, desc, fk(desc), slow);
                            let n = self.c64(words.len() as i64 * 8);
                            let (a, p) = self.temp_alloc(&c, n, slow);
                            for (k, w) in words.into_iter().enumerate() {
                                self.store(w, self.gep(p, self.c64(k as i64 * 8)));
                            }
                            self.st(&c, dst, a);
                            self.join(&c, join);
                        }
                        None if c.nat.is_some() => self.native_step(&c, pc, &inst, f)?,
                        None => self.step(&c, pc, &inst),
                    },
                    // A variable kept as a value (`Scalars`): no memory access until a write-back.
                    Inst::Load { dst, w, .. }
                        if c.scal.as_ref().is_some_and(|s| s.at[i].is_some()) =>
                    {
                        let _ = w;
                        let k = c.scal.as_ref().and_then(|s| s.at[i]).expect("slot") as usize;
                        let v = self.vslot_value(&c, k);
                        self.std(&c, dst, v);
                    }
                    Inst::Store { src, w, .. }
                        if c.scal.as_ref().is_some_and(|s| s.at[i].is_some()) =>
                    {
                        let k = c.scal.as_ref().and_then(|s| s.at[i]).expect("slot") as usize;
                        let mut v = self.ld(&c, src);
                        if w != Width::B8 {
                            // As the frame would hold it, read back.
                            let t = LLVMBuildTrunc(self.b, v, self.width_ty(w), c"".as_ptr());
                            v = if w == Width::B4 {
                                LLVMBuildSExt(self.b, t, self.i64, c"".as_ptr())
                            } else {
                                LLVMBuildZExt(self.b, t, self.i64, c"".as_ptr())
                            };
                        }
                        let nregs = c.known.borrow().len();
                        if let Some(d) = &c.defer {
                            d.pend.borrow_mut()[nregs + k] = Some(v);
                        }
                    }
                    // A variable of the frame: no check, no slow path, so the result can be
                    // deferred like any other (`Gen::host` takes the same case).
                    Inst::Load { dst, addr, w }
                        if w != Width::Sig
                            && matches!(fk(addr), Some(Known::Frame(o))
                                if o.checked_add(w.bytes() as u64).is_some_and(|e| e <= c.frame_len)) =>
                    {
                        let n = self.c64(i64::from(w.bytes()));
                        let p = self.host(
                            &c,
                            self.ld(&c, addr),
                            fk(addr),
                            self.c64(0),
                            n,
                            false,
                            std::ptr::null_mut(),
                        );
                        let v = self.load(self.width_ty(w), p);
                        let v = match w {
                            Width::B8 => v,
                            Width::B4 => LLVMBuildSExt(self.b, v, self.i64, c"".as_ptr()),
                            _ => LLVMBuildZExt(self.b, v, self.i64, c"".as_ptr()),
                        };
                        self.std(&c, dst, v);
                    }
                    Inst::Load { dst, addr, w } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let n = self.c64(i64::from(w.bytes()));
                        let p =
                            self.host(&c, self.ld(&c, addr), fk(addr), self.c64(0), n, false, slow);
                        match w {
                            Width::Sig => {
                                self.st(&c, dst, self.load(self.i64, p));
                                let p8 = self.gep(p, self.c64(8));
                                self.st(&c, Reg(dst.0 + 1), self.load(self.i64, p8));
                            }
                            _ => {
                                let ty = self.width_ty(w);
                                let v = self.load(ty, p);
                                let v = match w {
                                    Width::B8 => v,
                                    Width::B4 => LLVMBuildSExt(self.b, v, self.i64, c"".as_ptr()),
                                    _ => LLVMBuildZExt(self.b, v, self.i64, c"".as_ptr()),
                                };
                                self.st(&c, dst, v);
                            }
                        }
                        self.join(&c, join);
                    }
                    Inst::Store { addr, src, w } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let n = self.c64(i64::from(w.bytes()));
                        let p =
                            self.host(&c, self.ld(&c, addr), fk(addr), self.c64(0), n, true, slow);
                        match w {
                            Width::Sig => {
                                self.store(self.ld(&c, src), p);
                                let p8 = self.gep(p, self.c64(8));
                                self.store(self.ld(&c, Reg(src.0 + 1)), p8);
                            }
                            _ => {
                                let v = self.ld(&c, src);
                                let v = if w == Width::B8 {
                                    v
                                } else {
                                    LLVMBuildTrunc(self.b, v, self.width_ty(w), c"".as_ptr())
                                };
                                self.store(v, p);
                            }
                        }
                        self.join(&c, join);
                    }
                    Inst::Copy { dst, src, bytes } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let n = self.ld(&c, bytes);
                        let ps =
                            self.host(&c, self.ld(&c, src), fk(src), self.c64(0), n, false, slow);
                        let pd =
                            self.host(&c, self.ld(&c, dst), fk(dst), self.c64(0), n, true, slow);
                        LLVMBuildMemMove(self.b, pd, 1, ps, 1, n);
                        self.join(&c, join);
                    }
                    Inst::MakeDope { at, parts } => {
                        let rs = f.regs(parts).ok_or("operand list out of range")?;
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let n = self.c64(rs.len() as i64 * 8);
                        let p = self.host(&c, self.ld(&c, at), fk(at), self.c64(0), n, true, slow);
                        for (k, &r) in rs.iter().enumerate() {
                            let pk = self.gep(p, self.c64(k as i64 * 8));
                            self.store(self.ld(&c, r), pk);
                        }
                        self.join(&c, join);
                    }
                    Inst::DopeLen { dst, dope, dim } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let n24 = self.c64(24);
                        let p = self.host(
                            &c,
                            self.ld(&c, dope),
                            fk(dope),
                            self.c64(i64::from(dim) * 24),
                            n24,
                            false,
                            slow,
                        );
                        let (l, r, dir) = self.dope_words(p);
                        let (n, fits) = self.len_of(l, r, dir);
                        // A length over i64::MAX is the interpreter's error.
                        let small = self.icmp(IP::LLVMIntSGE, n, self.c64(0));
                        self.guard(&c, LLVMBuildAnd(self.b, fits, small, c"".as_ptr()), slow);
                        self.st(&c, dst, n);
                        self.join(&c, join);
                    }
                    Inst::SubtypeCheck {
                        dope, want, dims, ..
                    } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        if dims <= 4 {
                            // The lengths of both descriptors, read where they are (a native
                            // frame's own slots too); a mismatch is the interpreter's error.
                            let n24 = self.c64(24);
                            for k in 0..i64::from(dims) {
                                let len = |r: Reg| {
                                    let p = self.host(
                                        &c,
                                        self.ld(&c, r),
                                        fk(r),
                                        self.c64(k * 24),
                                        n24,
                                        false,
                                        slow,
                                    );
                                    let (l, rt, dir) = self.dope_words(p);
                                    self.len_of(l, rt, dir)
                                };
                                let ((a, fa), (b, fb)) = (len(dope), len(want));
                                let both = LLVMBuildAnd(self.b, fa, fb, c"".as_ptr());
                                let same = self.icmp(IP::LLVMIntEQ, a, b);
                                self.guard(
                                    &c,
                                    LLVMBuildAnd(self.b, both, same, c"".as_ptr()),
                                    slow,
                                );
                            }
                        } else {
                            let (d, w) = (self.ld(&c, dope), self.ld(&c, want));
                            self.fast_op(&c, FAST_SUBTYPE, u32::from(dims), d, w, slow);
                        }
                        self.join(&c, join);
                    }
                    Inst::Pos {
                        dst,
                        dope,
                        dim,
                        idx,
                        ..
                    } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let n24 = self.c64(24);
                        let p = self.host(
                            &c,
                            self.ld(&c, dope),
                            fk(dope),
                            self.c64(i64::from(dim) * 24),
                            n24,
                            false,
                            slow,
                        );
                        let (l, r, dir) = self.dope_words(p);
                        let i = self.ld(&c, idx);
                        let le = |a, b| self.icmp(IP::LLVMIntSLE, a, b);
                        let and = |a, b| LLVMBuildAnd(self.b, a, b, c"".as_ptr());
                        let up = and(
                            self.icmp(IP::LLVMIntEQ, dir, self.c64(0)),
                            and(le(l, i), le(i, r)),
                        );
                        let dn = and(
                            self.icmp(IP::LLVMIntEQ, dir, self.c64(1)),
                            and(le(r, i), le(i, l)),
                        );
                        self.guard(&c, self.or(up, dn), slow);
                        let a = LLVMBuildSub(self.b, i, l, c"".as_ptr());
                        let b = LLVMBuildSub(self.b, l, i, c"".as_ptr());
                        self.st(&c, dst, LLVMBuildSelect(self.b, up, a, b, c"".as_ptr()));
                        self.join(&c, join);
                    }
                    Inst::Field {
                        dst,
                        base,
                        layout,
                        n,
                    } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let n8 = self.c64(8);
                        let p = self.host(
                            &c,
                            self.ld(&c, layout),
                            fk(layout),
                            self.c64(16 + 8 * i64::from(n)),
                            n8,
                            false,
                            slow,
                        );
                        let off = self.load(self.i64, p);
                        let a = self.ld(&c, base);
                        let lo = LLVMBuildAnd(self.b, a, self.c64(0xffff_ffff), c"".as_ptr());
                        let (sum, o) = self.ovf(self.uadd, lo, off);
                        let big = self.icmp(IP::LLVMIntUGT, sum, self.c64(0xffff_ffff));
                        self.guard(
                            &c,
                            LLVMBuildNot(self.b, self.or(o, big), c"".as_ptr()),
                            slow,
                        );
                        let hi = LLVMBuildAnd(self.b, a, self.c64(!0xffff_ffff), c"".as_ptr());
                        self.st(&c, dst, LLVMBuildOr(self.b, hi, sum, c"".as_ptr()));
                        self.join(&c, join);
                    }
                    Inst::LGet { dst, base, idx } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let (p, sh) = self.logic_word(&c, base, fk(base), idx, false, slow);
                        let mut code = self.c64(0);
                        for k in 0..4 {
                            let w = self.load(self.i64, self.gep(p, self.c64(k * 8)));
                            let bit = LLVMBuildLShr(self.b, w, sh, c"".as_ptr());
                            let bit = LLVMBuildAnd(self.b, bit, self.c64(1), c"".as_ptr());
                            let bit = LLVMBuildShl(self.b, bit, self.c64(k), c"".as_ptr());
                            code = LLVMBuildOr(self.b, code, bit, c"".as_ptr());
                        }
                        self.st(&c, dst, code);
                        self.join(&c, join);
                    }
                    Inst::LSet { base, idx, v } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let code = self.ld(&c, v);
                        // A code beyond 'U' to '-' is the interpreter's error.
                        self.guard(&c, self.icmp(IP::LLVMIntULE, code, self.c64(8)), slow);
                        let (p, sh) = self.logic_word(&c, base, fk(base), idx, true, slow);
                        let one = LLVMBuildShl(self.b, self.c64(1), sh, c"".as_ptr());
                        let keep = LLVMBuildNot(self.b, one, c"".as_ptr());
                        for k in 0..4 {
                            let pk = self.gep(p, self.c64(k * 8));
                            let w =
                                LLVMBuildAnd(self.b, self.load(self.i64, pk), keep, c"".as_ptr());
                            let bit = LLVMBuildLShr(self.b, code, self.c64(k), c"".as_ptr());
                            let bit = LLVMBuildAnd(self.b, bit, self.c64(1), c"".as_ptr());
                            let bit = LLVMBuildShl(self.b, bit, sh, c"".as_ptr());
                            self.store(LLVMBuildOr(self.b, w, bit, c"".as_ptr()), pk);
                        }
                        self.join(&c, join);
                    }
                    // The heap's own lookup through a helper; a failure is the interpreter's.
                    Inst::Deref { dst, handle, .. } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let h = self.ld(&c, handle);
                        self.fast_reg(&c, FAST_DEREF, dst, h, self.c64(0), slow);
                        self.join(&c, join);
                    }
                    Inst::MemEq { dst, a, b, n } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let k = self.ld(&c, n);
                        let pa = self.host(&c, self.ld(&c, a), fk(a), self.c64(0), k, false, slow);
                        let pb = self.host(&c, self.ld(&c, b), fk(b), self.c64(0), k, false, slow);
                        let mut args = [pa, pb, k];
                        let v = LLVMBuildCall2(
                            self.b,
                            self.memeq_ty,
                            self.memeq,
                            args.as_mut_ptr(),
                            3,
                            c"".as_ptr(),
                        );
                        self.st(&c, dst, v);
                        self.join(&c, join);
                    }
                    Inst::SigVal {
                        dst,
                        sig,
                        offset,
                        len,
                    } => {
                        // `Machine::sig_val`: element range within u32, an instance signal.
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let (o, l) = (self.ld(&c, offset), self.ld(&c, len));
                        let max = self.c64(i64::from(u32::MAX));
                        let n = self.mem_ld(&c, MEM_NSIGS, self.i64);
                        let ok = LLVMBuildAnd(
                            self.b,
                            self.icmp(IP::LLVMIntULE, o, max),
                            self.icmp(IP::LLVMIntULE, l, max),
                            c"".as_ptr(),
                        );
                        let s = self.c64(i64::from(sig));
                        let has = self.icmp(IP::LLVMIntULT, s, n);
                        self.guard(&c, LLVMBuildAnd(self.b, ok, has, c"".as_ptr()), slow);
                        let hi = self.c64(i64::from(sig) << 32);
                        self.st(&c, dst, LLVMBuildOr(self.b, hi, o, c"".as_ptr()));
                        self.st(&c, Reg(dst.0 + 1), l);
                        self.join(&c, join);
                    }
                    Inst::Br { to } => {
                        let bb = block_bb(to)?;
                        let snap = self.edge_prep(&c, &[start_of(to)?]);
                        self.edge_to(&c, &snap, start_of(to)?);
                        LLVMBuildBr(self.b, bb);
                        open = false;
                    }
                    Inst::CondBr { c: r, t, f: e } => {
                        let x = self.icmp(IP::LLVMIntNE, self.ld(&c, r), self.c64(0));
                        let (bt, be) = (block_bb(t)?, block_bb(e)?);
                        let snap = self.edge_prep(&c, &[start_of(t)?, start_of(e)?]);
                        self.edge_to(&c, &snap, start_of(t)?);
                        self.edge_to(&c, &snap, start_of(e)?);
                        LLVMBuildCondBr(self.b, x, bt, be);
                        open = false;
                    }
                    Inst::Case { v, table } => {
                        let t = prog
                            .cases
                            .get(table)
                            .ok_or_else(|| format!("no case table {}", table.0))?;
                        let x = self.ld(&c, v);
                        let mut to_all = vec![start_of(t.default)?];
                        for &(_, _, to) in &t.arms {
                            to_all.push(start_of(to)?);
                        }
                        let snap = self.edge_prep(&c, &to_all);
                        // ponytail: a compare chain per arm; a switch for dense tables if
                        // case-heavy code ever shows in profiles.
                        for &(lo, hi, to) in &t.arms {
                            let ge = self.icmp(IP::LLVMIntSGE, x, self.c64(lo));
                            let le = self.icmp(IP::LLVMIntSLE, x, self.c64(hi));
                            let hit = LLVMBuildAnd(self.b, ge, le, c"".as_ptr());
                            let next = self.block(&c);
                            self.edge_to(&c, &snap, start_of(to)?);
                            LLVMBuildCondBr(self.b, hit, block_bb(to)?, next);
                            LLVMPositionBuilderAtEnd(self.b, next);
                        }
                        self.edge_to(&c, &snap, start_of(t.default)?);
                        LLVMBuildBr(self.b, block_bb(t.default)?);
                        open = false;
                    }
                    Inst::LCopy {
                        dst,
                        dst_off,
                        src,
                        src_off,
                        n,
                    } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let (k, so, d) =
                            (self.ld(&c, n), self.ld(&c, src_off), self.ld(&c, dst_off));
                        // Nothing to copy: no memory is touched, as in the interpreter.
                        let go = self.block(&c);
                        let z = self.icmp(IP::LLVMIntEQ, k, self.c64(0));
                        LLVMBuildCondBr(self.b, z, join, go);
                        LLVMPositionBuilderAtEnd(self.b, go);
                        let small = |v| self.icmp(IP::LLVMIntULT, v, self.c64(1 << 32));
                        let ok = LLVMBuildAnd(self.b, small(k), small(so), c"".as_ptr());
                        self.guard(&c, LLVMBuildAnd(self.b, ok, small(d), c"".as_ptr()), slow);
                        // First block, block count, first element in it.
                        let blocks = |off| {
                            let b0 = LLVMBuildLShr(self.b, off, self.c64(6), c"".as_ptr());
                            let last = LLVMBuildAdd(self.b, off, k, c"".as_ptr());
                            let last = LLVMBuildSub(self.b, last, self.c64(1), c"".as_ptr());
                            let b1 = LLVMBuildLShr(self.b, last, self.c64(6), c"".as_ptr());
                            let nb = LLVMBuildSub(self.b, b1, b0, c"".as_ptr());
                            let nb = LLVMBuildAdd(self.b, nb, self.c64(1), c"".as_ptr());
                            let first = LLVMBuildShl(self.b, b0, self.c64(6), c"".as_ptr());
                            (b0, nb, LLVMBuildSub(self.b, off, first, c"".as_ptr()))
                        };
                        let (sb0, ns, s0) = blocks(so);
                        let (db0, nd, d0) = blocks(d);
                        let max = self.c64(LCOPY_BLOCKS as i64);
                        let fit = LLVMBuildAnd(
                            self.b,
                            self.icmp(IP::LLVMIntULE, ns, max),
                            self.icmp(IP::LLVMIntULE, nd, max),
                            c"".as_ptr(),
                        );
                        self.guard(&c, fit, slow);
                        let b32 = |v| LLVMBuildShl(self.b, v, self.c64(5), c"".as_ptr());
                        let ps = self.host(
                            &c,
                            self.ld(&c, src),
                            fk(src),
                            b32(sb0),
                            b32(ns),
                            false,
                            slow,
                        );
                        let pd =
                            self.host(&c, self.ld(&c, dst), fk(dst), b32(db0), b32(nd), true, slow);
                        // Up to 64 elements: at most two blocks each side, shifted in registers. A
                        // process does it per activation; in a subprogram the 225 LLVM
                        // instructions are not worth the call they save.
                        if f.kind == FuncKind::Process {
                            let few = self.icmp(IP::LLVMIntULE, k, self.c64(64));
                            let (inl, call) = (self.block(&c), self.block(&c));
                            LLVMBuildCondBr(self.b, few, inl, call);
                            LLVMPositionBuilderAtEnd(self.b, inl);
                            self.lcopy_inline(&c, pd, nd, d0, ps, ns, s0, k);
                            LLVMBuildBr(self.b, join);
                            LLVMPositionBuilderAtEnd(self.b, call);
                        }
                        let mut args = [pd, nd, d0, ps, ns, s0, k];
                        LLVMBuildCall2(
                            self.b,
                            self.lcopy_ty,
                            self.lcopy,
                            args.as_mut_ptr(),
                            7,
                            c"".as_ptr(),
                        );
                        self.join(&c, join);
                    }
                    Inst::Cat {
                        dst,
                        n,
                        m,
                        parts,
                        scalars,
                        bias,
                        logic: true,
                        ..
                    } => {
                        let rs = f.regs(parts).ok_or("operand list out of range")?;
                        if !self.cat_block(&c, pc, &inst, dst, n, m, rs, scalars, bias, fk(dst)) {
                            self.slow_inst(&c, pc, &inst, f)?;
                        }
                    }
                    Inst::SigReadScalar { dst, sig } => self.sig_read_scalar(&c, pc, dst, sig),
                    Inst::Drive {
                        driver,
                        target: Some(t),
                        wave,
                        n,
                        mode,
                        reject: None,
                    } if c.nat.is_none() => self.drive_block(&c, pc, driver, t, wave, n, mode),
                    Inst::WaitOn { sig }
                        if c.nat.is_none()
                            && matches!(f.code.get(i + 1), Some(Inst::Suspend { .. })) =>
                    {
                        self.wait_suspend(&c, pc, sig)
                    }
                    Inst::SigRead { dst, sig }
                        if c.nat.is_none() && self.sig_read_block(&c, pc, dst, sig) => {}
                    Inst::SigRead { .. }
                    | Inst::WaitOn { .. }
                    | Inst::WaitFor { .. }
                    | Inst::DriveSlot { .. }
                    | Inst::Drive { .. }
                    | Inst::Suspend { .. } => self.signal(&c, pc, 0),
                    Inst::CallNative { native, args, dst } => {
                        match native_kind(prog, f, native, args, dst) {
                            Some(NativeKind::Table(t, rs, d)) => {
                                let (slow, join) = self.slow_path(&c, pc, &inst);
                                let mut idx = self.c64(0);
                                for r in rs {
                                    // A code beyond the nine is the native's error.
                                    let x = self.ld(&c, r);
                                    self.guard(&c, self.icmp(IP::LLVMIntULE, x, self.c64(8)), slow);
                                    let i9 = LLVMBuildMul(self.b, idx, self.c64(9), c"".as_ptr());
                                    idx = LLVMBuildAdd(self.b, i9, x, c"".as_ptr());
                                }
                                let p = self.gep(self.table(&t), idx);
                                self.st(&c, d, self.zext8(self.load(self.i8, p)));
                                self.join(&c, join);
                            }
                            Some(NativeKind::Edge(rising)) => match (f.regs(args), dst) {
                                (Some(&[s]), Some(d)) => self.edge(&c, pc, rising, s, d),
                                _ => self.signal(&c, pc, if rising { 1 } else { 2 }),
                            },
                            None if c.nat.is_some()
                                && prog
                                    .natives
                                    .get(native)
                                    .is_some_and(|n| yavs_interp::cold_native(&n.symbol)) =>
                            {
                                self.step(&c, pc, &inst)
                            }
                            None if c.nat.is_some() && nat_inline(prog, f, &inst) => {
                                self.native_direct(&c, pc, &inst, f, native, args, dst)?
                            }
                            None => self.step(&c, pc, &inst),
                        }
                    }
                    Inst::Call { func, args, dst }
                        if nc.contains(&func)
                            && prog
                                .func(func)
                                .is_some_and(|g| call_fits(f, id, g, args, false)) =>
                    {
                        let g = prog.func(func).ok_or("callee vanished")?;
                        let rs = f.regs(args).ok_or("operand list out of range")?;
                        self.native_call(&c, pc, &inst, id, f, func, g, rs, dst, None);
                    }
                    Inst::CallProt {
                        func,
                        obj,
                        args,
                        dst,
                    } if nc.contains(&func)
                        && obj.0 < f.reg_ty.len() as u32
                        && prog
                            .func(func)
                            .is_some_and(|g| call_fits(f, id, g, args, true)) =>
                    {
                        let g = prog.func(func).ok_or("callee vanished")?;
                        let rs = f.regs(args).ok_or("operand list out of range")?;
                        self.native_call(&c, pc, &inst, id, f, func, g, rs, dst, Some(obj));
                    }
                    Inst::NewFrame {
                        dst,
                        desc,
                        dsize,
                        data,
                        size,
                        fill,
                    } if c.nat.is_some() => {
                        let fk = |r: Reg| self.known_addr(&c, r);
                        self.new_frame(&c, pc, &inst, dst, desc, dsize, data, size, fill, &fk);
                    }
                    Inst::Fill { dst, elem, n, size } if c.nat.is_some() => {
                        let fk = |r: Reg| self.known_addr(&c, r);
                        self.fill(&c, pc, &inst, dst, elem, n, size, &fk);
                    }
                    Inst::Cat {
                        dst,
                        n: _,
                        m: elem,
                        parts,
                        scalars,
                        bias: _,
                        logic: false,
                        w,
                    } if c.nat.is_some() => {
                        let rs = f.regs(parts).ok_or("operand list out of range")?;
                        let fk = |r: Reg| self.known_addr(&c, r);
                        self.cat_bytes(&c, pc, &inst, dst, elem, rs, scalars, w, &fk);
                    }
                    Inst::TempRoot {
                        addr,
                        desc,
                        ty: Some(t),
                    } if c.nat.is_some() => {
                        let fk = |r: Reg| self.known_addr(&c, r);
                        let n =
                            yavs_ir::Ty::desc_size(&prog.types, t).ok_or("no descriptor size")?;
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let dp = match (n, desc) {
                            (0, _) => LLVMConstPointerNull(self.ptr),
                            (_, Some(d)) => self.host(
                                &c,
                                self.ld(&c, d),
                                fk(d),
                                self.c64(0),
                                self.c64(n as i64),
                                false,
                                slow,
                            ),
                            _ => return Err("recorded temporary without its descriptor".into()),
                        };
                        let ok = self.troot_call(&c, self.ld(&c, addr), t.0, dp, n);
                        self.guard(&c, self.icmp(IP::LLVMIntNE, ok, self.c32(0)), slow);
                        self.join(&c, join);
                    }
                    Inst::Unreachable { .. } if c.nat.is_some() => self.step(&c, pc, &inst),
                    _ if c.nat.is_some() && xstep_ok(f, &inst) => {
                        self.native_step(&c, pc, &inst, f)?;
                    }
                    _ => self.step(&c, pc, &inst),
                }
            }
            if open {
                // Falling off the end: the interpreter reports it.
                self.leave(&c, self.c32(n as u32));
            }
            if let Some(nat) = &c.nat {
                // Materialisation: record the registers, slots, pc and temporary base, and
                // return with `Mem::ndeopt` set.
                LLVMPositionBuilderAtEnd(self.b, exit);
                c.known.borrow_mut().fill(None);
                let mut args = [
                    c.env,
                    c.mem,
                    self.c32(nat.id.0),
                    exit_pc,
                    LLVMBuildLoad2(self.b, self.i64, nat.temp_base, c"".as_ptr()),
                    self.c64(nregs as i64),
                    self.c64(i64::from(f.frame)),
                    nat.up[0],
                ];
                let rp = LLVMBuildCall2(
                    self.b,
                    self.deopt_ty,
                    self.deopt,
                    args.as_mut_ptr(),
                    8,
                    c"".as_ptr(),
                );
                for r in 0..nregs {
                    let v = self.ld(&c, Reg(r as u32));
                    self.store(v, self.gep(rp, self.c64(r as i64 * 8)));
                }
                let mut env = [c.env];
                let sp = LLVMBuildCall2(
                    self.b,
                    self.deopt_slots_ty,
                    self.deopt_slots,
                    env.as_mut_ptr(),
                    1,
                    c"".as_ptr(),
                );
                LLVMBuildMemCpy(self.b, sp, 1, nat.slots, 8, self.c64(i64::from(f.frame)));
                LLVMBuildRet(self.b, self.c64(0));
            }
            // A block nothing branches to (the generator and `regs::reach` agree on all but
            // dead code) has phis without incoming values.
            if let Some(pl) = c.defer.as_ref().and_then(|d| d.plan.as_ref()) {
                for &phi in pl.phis.iter().flatten() {
                    if LLVMCountIncoming(phi) == 0 {
                        LLVMReplaceAllUsesWith(phi, LLVMGetUndef(self.i64));
                        LLVMInstructionEraseFromParent(phi);
                    }
                }
            }
            debug_assert!(c.joins.borrow().is_empty(), "a slow path was not joined");
            if LLVMVerifyFunction(fv, LLVMVerifierFailureAction::LLVMReturnStatusAction) != 0 {
                // A native function may have callers in the module already: the whole module
                // is generated again without native frames (`compile`).
                if !native {
                    LLVMDeleteFunction(fv);
                }
                return Err("the generated LLVM function does not verify (a bug in yavs)".into());
            }
            Ok(())
        }
    }

    /// Like `slow_path`, but the interpreter runs the signal instruction at `pc` through
    /// `TierHost::signal` (`edge` as in `signal`).
    unsafe fn slow_signal(
        &self,
        c: &Cur,
        pc: u32,
        edge: u32,
    ) -> (LLVMBasicBlockRef, LLVMBasicBlockRef) {
        // SAFETY: well-formed blocks of c.f.
        unsafe {
            let here = LLVMGetInsertBlock(self.b);
            let (slow, join) = (self.block(c), self.block(c));
            LLVMPositionBuilderAtEnd(self.b, slow);
            // As in `slow_path`; a signal operation pushes no frame.
            let base = c
                .defer
                .as_ref()
                .map(|d| *d.pend.borrow().last().expect("base"));
            self.signal(c, pc, edge);
            if let (Some(d), Some(b)) = (&c.defer, base) {
                *d.pend.borrow_mut().last_mut().expect("base") = b;
                c.joins.borrow_mut().push((b, false));
            }
            LLVMBuildBr(self.b, join);
            LLVMPositionBuilderAtEnd(self.b, here);
            (slow, join)
        }
    }

    /// The `TierSig` entry of the instance signal that reference registers `sig`, `sig + 1`
    /// name, and the element offset in it; taking `slow` unless the reference is a plain one
    /// of `len` elements to an instance signal the table has.
    unsafe fn sig_entry(
        &self,
        c: &Cur,
        sig: Reg,
        len: Option<i64>,
        slow: LLVMBasicBlockRef,
    ) -> (LLVMValueRef, LLVMValueRef) {
        // SAFETY: loads of `Mem` fields and of a `TierSig` the guard checked is in the table.
        unsafe {
            let (v, l) = (self.ld(c, sig), self.ld(c, Reg(sig.0 + 1)));
            let base = LLVMBuildLShr(self.b, v, self.c64(32), c"".as_ptr());
            let off = LLVMBuildAnd(self.b, v, self.c64(0xffff_ffff), c"".as_ptr());
            let n = self.mem_ld(c, MEM_SIGTAB_LEN, self.i64);
            let has = self.icmp(IP::LLVMIntULT, base, n);
            let want = match len {
                Some(k) => self.icmp(IP::LLVMIntEQ, l, self.c64(k)),
                None => self.icmp(IP::LLVMIntNE, l, self.c64(0)),
            };
            self.guard(c, LLVMBuildAnd(self.b, has, want, c"".as_ptr()), slow);
            let tab = self.mem_ld(c, MEM_SIGTAB, self.ptr);
            let at = LLVMBuildMul(self.b, base, self.c64(SIG_ENT), c"".as_ptr());
            (self.gep(tab, at), off)
        }
    }

    /// `Inst::SigReadScalar` of a logic signal that keeps its codes: the byte, in place;
    /// anything else through `TierHost::signal`.
    unsafe fn sig_read_scalar(&self, c: &Cur, pc: u32, dst: Reg, sig: Reg) {
        // SAFETY: see `sig_entry`; the guard checks the element is within the codes.
        unsafe {
            let (slow, join) = self.slow_signal(c, pc, 0);
            let (ent, off) = self.sig_entry(c, sig, None, slow);
            let len = self.load(self.i64, self.gep(ent, self.c64(SIG_LEN)));
            self.guard(c, self.icmp(IP::LLVMIntULT, off, len), slow);
            let codes = self.tag(
                self.load(self.ptr, self.gep(ent, self.c64(SIG_CODES))),
                AA_CODES,
            );
            let byte = self.load(self.i8, self.gep(codes, off));
            self.st(c, dst, self.zext8(byte));
            self.join(c, join);
        }
    }

    /// `Inst::SigRead` of a plain reference of 1 to 64 elements (the count known) to a logic
    /// signal that keeps its codes: the planes built from the code bytes in place, eight
    /// elements at a time; anything else through `TierHost::signal`. `false`: not that case,
    /// nothing generated.
    unsafe fn sig_read_block(&self, c: &Cur, pc: u32, dst: Reg, sig: Reg) -> bool {
        let Some(l) = self
            .kint(c, Reg(sig.0 + 1))
            .filter(|l| (1..=64).contains(l))
        else {
            return false;
        };
        let groups = (l as u64).div_ceil(8);
        // SAFETY: see `sig_entry`; the guards check the groups lie within the codes.
        unsafe {
            let (slow, join) = self.slow_signal(c, pc, 0);
            let (ent, off) = self.sig_entry(c, sig, Some(l), slow);
            let tl = self.load(self.i64, self.gep(ent, self.c64(SIG_LEN)));
            let end = LLVMBuildAdd(self.b, off, self.c64(8 * groups as i64), c"".as_ptr());
            self.guard(c, self.icmp(IP::LLVMIntULE, end, tl), slow);
            let codes = self.tag(
                self.load(self.ptr, self.gep(ent, self.c64(SIG_CODES))),
                AA_CODES,
            );
            let base = self.gep(codes, off);
            let mut planes = [self.c64(0); 4];
            let ones = self.c64(0x0101_0101_0101_0101);
            let spread = self.c64(0x0102_0408_1020_4080);
            for j in 0..groups {
                let w = self.load(self.i64, self.gep(base, self.c64(8 * j as i64)));
                for (p, plane) in planes.iter_mut().enumerate() {
                    let bit = LLVMBuildLShr(self.b, w, self.c64(p as i64), c"".as_ptr());
                    let bit = LLVMBuildAnd(self.b, bit, ones, c"".as_ptr());
                    let bit = LLVMBuildMul(self.b, bit, spread, c"".as_ptr());
                    let bit = LLVMBuildLShr(self.b, bit, self.c64(56), c"".as_ptr());
                    let bit = LLVMBuildShl(self.b, bit, self.c64(8 * j as i64), c"".as_ptr());
                    *plane = LLVMBuildOr(self.b, *plane, bit, c"".as_ptr());
                }
            }
            let mask = if l == 64 { !0 } else { (1u64 << l) - 1 };
            let pd = self.host(
                c,
                self.ld(c, dst),
                self.known_addr(c, dst),
                self.c64(0),
                self.c64(32),
                true,
                slow,
            );
            for (p, plane) in planes.iter().enumerate() {
                let w = LLVMBuildAnd(self.b, *plane, self.c64(mask as i64), c"".as_ptr());
                self.store(w, self.gep(pd, self.c64(8 * p as i64)));
            }
            self.join(c, join);
        }
        true
    }

    /// `rising_edge` / `falling_edge` of one element: true in place when the kernel resumed the
    /// process for that edge (`TierFrame::known_sig`), else through `TierHost::signal`.
    unsafe fn edge(&self, c: &Cur, pc: u32, rising: bool, sig: Reg, dst: Reg) {
        // SAFETY: see `sig_entry`.
        unsafe {
            let (slow, join) = self.slow_signal(c, pc, if rising { 1 } else { 2 });
            let (ent, off) = self.sig_entry(c, sig, Some(1), slow);
            let signal = self.load(self.i64, self.gep(ent, self.c64(SIG_SIGNAL)));
            let first = self.load(self.i64, self.gep(ent, self.c64(SIG_OFF)));
            let elem = LLVMBuildAdd(self.b, first, off, c"".as_ptr());
            let held = |a: LLVMValueRef, b: LLVMValueRef| self.icmp(IP::LLVMIntEQ, a, b);
            let ks = self.mem_ld(c, MEM_KNOWN_SIG, self.i64);
            let ke = self.mem_ld(c, MEM_KNOWN_ELEM, self.i64);
            let ku = self.mem_ld(c, MEM_KNOWN_UP, self.i64);
            let ok = LLVMBuildAnd(
                self.b,
                held(signal, ks),
                LLVMBuildAnd(
                    self.b,
                    held(elem, ke),
                    held(ku, self.c64(i64::from(rising))),
                    c"".as_ptr(),
                ),
                c"".as_ptr(),
            );
            self.guard(c, ok, slow);
            self.st(c, dst, self.c64(1));
            self.join(c, join);
        }
    }

    /// Execute the instruction `i` at `pc` in the interpreter; return what `yavs_rt_step`
    /// returns unless it is `GO`.
    unsafe fn step(&self, c: &Cur, pc: u32, i: &Inst) {
        if c.nat.is_some() {
            // SAFETY: as `deopt_here`.
            return unsafe { self.deopt_here(c, pc) };
        }
        // Instructions that only change the temporary area or a register go to the interpreter's own
        // function for them (`TierHost::signal`), without its generic dispatch.
        if matches!(
            i,
            Inst::TempRoot { .. }
                | Inst::NewFrame { .. }
                | Inst::Stmt
                | Inst::Deref { .. }
                | Inst::MemEq { .. }
                | Inst::Cat { .. }
        ) {
            // SAFETY: as `signal`.
            return unsafe { self.signal(c, pc, 0) };
        }
        let at = if keeps_objects(i) { pc | KEEP } else { pc };
        // SAFETY: `step` has the run-function signature.
        unsafe {
            self.call_go(
                c,
                self.fn_ty,
                self.step,
                &mut [c.env, c.regs, c.mem, self.c32(at)],
                quiet(i),
            )
        }
    }

    /// `Inst::LCopy` of `k` (1 to 64) elements from bit `s0` of the one or two plane blocks at
    /// `ps` (`ns` of them) to bit `d0` of those at `pd` (`nd`), as `copy_bits` does: the
    /// source planes are read first (the ranges may overlap), the destination bits outside
    /// the copied range stay.
    #[allow(clippy::too_many_arguments)]
    unsafe fn lcopy_inline(
        &self,
        c: &Cur,
        pd: LLVMValueRef,
        nd: LLVMValueRef,
        d0: LLVMValueRef,
        ps: LLVMValueRef,
        ns: LLVMValueRef,
        s0: LLVMValueRef,
        k: LLVMValueRef,
    ) {
        // SAFETY: the pointers cover `ns` and `nd` blocks (`host`, checked by the caller).
        unsafe {
            let i128t = LLVMIntTypeInContext(self.ctx, 128);
            let b = self.b;
            let z128 = |v| LLVMBuildZExt(b, v, i128t, c"".as_ptr());
            let c128 = |v: u64| LLVMConstInt(i128t, v, 0);
            // Low `k` bits.
            let mask = {
                let full = self.icmp(IP::LLVMIntUGE, k, self.c64(64));
                let sh = LLVMBuildShl(b, self.c64(1), k, c"".as_ptr());
                let low = LLVMBuildSub(b, sh, self.c64(1), c"".as_ptr());
                LLVMBuildSelect(b, full, self.c64(-1), low, c"".as_ptr())
            };
            let two_s = self.icmp(IP::LLVMIntEQ, ns, self.c64(2));
            let two_d = self.icmp(IP::LLVMIntEQ, nd, self.c64(2));
            // Block 1 is only read (written) when there is one.
            let word = |p: LLVMValueRef, plane: i64, blk1: LLVMValueRef| {
                let lo = self.gep(p, self.c64(plane * 8));
                let hi_at = LLVMBuildSelect(
                    b,
                    blk1,
                    self.gep(p, self.c64(32 + plane * 8)),
                    lo,
                    c"".as_ptr(),
                );
                (lo, hi_at)
            };
            let mut vals = [self.c64(0); 4];
            for (plane, v) in vals.iter_mut().enumerate() {
                let (lo, hi_at) = word(ps, plane as i64, two_s);
                let (l, h) = (self.load(self.i64, lo), self.load(self.i64, hi_at));
                let h = LLVMBuildSelect(b, two_s, h, self.c64(0), c"".as_ptr());
                let wide = LLVMBuildOr(
                    b,
                    z128(l),
                    LLVMBuildShl(b, z128(h), c128(64), c"".as_ptr()),
                    c"".as_ptr(),
                );
                let sh = LLVMBuildLShr(b, wide, z128(s0), c"".as_ptr());
                let low = LLVMBuildTrunc(b, sh, self.i64, c"".as_ptr());
                *v = LLVMBuildAnd(b, low, mask, c"".as_ptr());
            }
            let hole = LLVMBuildShl(b, z128(mask), z128(d0), c"".as_ptr());
            let keep = LLVMBuildNot(b, hole, c"".as_ptr());
            for (plane, v) in vals.iter().enumerate() {
                let (lo, hi_at) = word(pd, plane as i64, two_d);
                let (l, h) = (self.load(self.i64, lo), self.load(self.i64, hi_at));
                let h = LLVMBuildSelect(b, two_d, h, self.c64(0), c"".as_ptr());
                let wide = LLVMBuildOr(
                    b,
                    z128(l),
                    LLVMBuildShl(b, z128(h), c128(64), c"".as_ptr()),
                    c"".as_ptr(),
                );
                let put = LLVMBuildShl(b, z128(*v), z128(d0), c"".as_ptr());
                let out = LLVMBuildOr(
                    b,
                    LLVMBuildAnd(b, wide, keep, c"".as_ptr()),
                    put,
                    c"".as_ptr(),
                );
                self.store(LLVMBuildTrunc(b, out, self.i64, c"".as_ptr()), lo);
                // The second block: the high half, when the destination has one.
                let (hi_bb, done) = (self.block(c), self.block(c));
                LLVMBuildCondBr(b, two_d, hi_bb, done);
                LLVMPositionBuilderAtEnd(b, hi_bb);
                let top = LLVMBuildLShr(b, out, c128(64), c"".as_ptr());
                self.store(LLVMBuildTrunc(b, top, self.i64, c"".as_ptr()), hi_at);
                LLVMBuildBr(b, done);
                LLVMPositionBuilderAtEnd(b, done);
            }
        }
    }

    /// A `Drive` of one waveform element to a single-range target through `yavs_rt_drive`
    /// (`TierHost::drive_block`: at most 64 logic elements assigned from the element's first
    /// plane block); the instruction through `yavs_rt_signal` unless `n` is 1 and the 40 bytes
    /// of the waveform are in memory compiled code reaches.
    #[allow(clippy::too_many_arguments)]
    unsafe fn drive_block(
        &self,
        c: &Cur,
        pc: u32,
        driver: u32,
        target: Reg,
        wave: Reg,
        n: Reg,
        mode: yavs_ir::DriveMode,
    ) {
        // SAFETY: the arguments match `drive_ty`; well-formed blocks of c.f.
        unsafe {
            let (slow, join) = self.slow_signal(c, pc, 0);
            self.guard(
                c,
                self.icmp(IP::LLVMIntEQ, self.ld(c, n), self.c64(1)),
                slow,
            );
            let w = self.host(
                c,
                self.ld(c, wave),
                self.known_addr(c, wave),
                self.c64(0),
                self.c64(40),
                false,
                slow,
            );
            let inertial = u32::from(mode == yavs_ir::DriveMode::Inertial);
            let mut args = [
                c.env,
                c.regs,
                c.mem,
                self.c32(pc),
                self.c32(driver),
                self.ld(c, target),
                self.ld(c, Reg(target.0 + 1)),
                w,
                self.c32(inertial),
            ];
            self.call_go(c, self.drive_ty, self.drive, &mut args, false);
            self.join(c, join);
        }
    }

    /// `Inst::Cat` of a logic array of at most 64 elements whose piece lengths the generator
    /// knows: the planes of all pieces combined in registers and stored once, as
    /// `Machine::cat_block` does; `false` (nothing generated) where a length is not known or
    /// does not fit that pattern. A runtime case it does not take (a code beyond 'U' to '-',
    /// memory compiled code does not reach) steps the whole instruction before any store.
    #[allow(clippy::too_many_arguments)]
    unsafe fn cat_block(
        &self,
        c: &Cur,
        pc: u32,
        inst: &Inst,
        dst: Reg,
        n: Reg,
        m: Reg,
        parts: &[Reg],
        scalars: u16,
        bias: u8,
        at: Option<Known>,
    ) -> bool {
        let (Some(n), Some(m)) = (self.kint(c, n), self.kint(c, m)) else {
            return false;
        };
        let Some(total) = u64::try_from(n)
            .ok()
            .zip(u64::try_from(m).ok())
            .and_then(|(n, m)| n.checked_mul(m))
            .filter(|&t| t <= 64)
        else {
            return false;
        };
        // (register, units, scalar) per piece, empty ones dropped, as `cat_block` skips them.
        let mut pieces = Vec::new();
        let mut k = 0u64;
        for (j, pr) in parts.as_chunks::<2>().0.iter().enumerate() {
            if scalars >> j & 1 == 1 {
                pieces.push((pr[0], 1u64, true));
                k += 1;
            } else {
                let Some(len) = self.kint(c, pr[1]).and_then(|l| u64::try_from(l).ok()) else {
                    return false;
                };
                let Some(units) = len.checked_mul(m as u64).filter(|&u| u <= 64) else {
                    return false;
                };
                if units == 0 {
                    continue;
                }
                pieces.push((pr[0], units, false));
                k += units;
            }
            if k > total {
                return false;
            }
        }
        // SAFETY: loads through `host`, integer arithmetic, well-formed blocks of c.f.
        unsafe {
            let (slow, join) = self.slow_path(c, pc, inst);
            let mut words = [self.c64(0); 4];
            let mut mask = 0u64;
            let mut at_k = 0u64;
            for &(r, units, scalar) in &pieces {
                let bits: [LLVMValueRef; 4] = if scalar {
                    let code = LLVMBuildAdd(
                        self.b,
                        self.ld(c, r),
                        self.c64(i64::from(bias)),
                        c"".as_ptr(),
                    );
                    // A code beyond 'U' to '-' is the interpreter's error.
                    self.guard(c, self.icmp(IP::LLVMIntULE, code, self.c64(8)), slow);
                    std::array::from_fn(|p| {
                        let b = LLVMBuildLShr(self.b, code, self.c64(p as i64), c"".as_ptr());
                        LLVMBuildAnd(self.b, b, self.c64(1), c"".as_ptr())
                    })
                } else {
                    let ptr = self.host(
                        c,
                        self.ld(c, r),
                        self.known_addr(c, r),
                        self.c64(0),
                        self.c64(32),
                        false,
                        slow,
                    );
                    std::array::from_fn(|p| {
                        let w = self.load(self.i64, self.gep(ptr, self.c64(p as i64 * 8)));
                        let pm = if units == 64 {
                            !0u64
                        } else {
                            (1u64 << units) - 1
                        };
                        LLVMBuildAnd(self.b, w, self.c64(pm as i64), c"".as_ptr())
                    })
                };
                let off = total - at_k - units;
                let pm = if units == 64 {
                    !0u64
                } else {
                    (1u64 << units) - 1
                };
                for p in 0..4 {
                    let sh = LLVMBuildShl(self.b, bits[p], self.c64(off as i64), c"".as_ptr());
                    words[p] = LLVMBuildOr(self.b, words[p], sh, c"".as_ptr());
                }
                mask |= pm << off;
                at_k += units;
            }
            if mask == 0 {
                self.join(c, join);
                return true;
            }
            let d = self.ld(c, dst);
            let pd = self.host(c, d, at, self.c64(0), self.c64(32), true, slow);
            // A block just allocated (zeroed) and covered by the pieces needs no read.
            let fresh = matches!(at, Some(Known::Temp)) && at_k == total;
            for (p, &word) in words.iter().enumerate() {
                let pp = self.gep(pd, self.c64(p as i64 * 8));
                let v = if fresh {
                    word
                } else {
                    let cur = self.load(self.i64, pp);
                    let keep = LLVMBuildAnd(self.b, cur, self.c64(!mask as i64), c"".as_ptr());
                    LLVMBuildOr(self.b, keep, word, c"".as_ptr())
                };
                self.store(v, pp);
            }
            self.join(c, join);
        }
        true
    }

    /// A `WaitOn` and the `Suspend` after it through `yavs_rt_wait` (`TierHost::wait_suspend`).
    unsafe fn wait_suspend(&self, c: &Cur, pc: u32, sig: Reg) {
        // SAFETY: the arguments match `wait_ty`.
        unsafe {
            let mut args = [
                c.env,
                c.regs,
                c.mem,
                self.c32(pc),
                self.ld(c, sig),
                self.ld(c, Reg(sig.0 + 1)),
            ];
            self.call_go(c, self.wait_ty, self.wait, &mut args, false);
        }
    }

    /// The signal instruction at `pc` through `yavs_rt_signal` (`edge`: `TierHost::signal`).
    unsafe fn signal(&self, c: &Cur, pc: u32, edge: u32) {
        if c.nat.is_some() {
            // SAFETY: as `deopt_here`.
            return unsafe { self.deopt_here(c, pc) };
        }
        let mut args = [c.env, c.regs, c.mem, self.c32(pc), self.c32(edge)];
        // SAFETY: the arguments match `signal_ty`.
        unsafe { self.call_go(c, self.signal_ty, self.signal, &mut args, false) }
    }

    /// `Inst::Call` of `callee` (`g`) on a native frame (`docs/decisions.md` "Compiled frames"):
    /// one frame index deeper, within the depth limit and the native stack allowance, when it
    /// has native code; the call is stepped (frame mode) or the frame materialised otherwise.
    /// When the callee unwinds for materialisation, a native caller records itself at the call,
    /// a frame-mode caller pushes the recorded frames and leaves them to the interpreter.
    #[allow(clippy::too_many_arguments)]
    unsafe fn native_call(
        &self,
        c: &Cur,
        pc: u32,
        i: &Inst,
        caller: FuncId,
        f: &Func,
        callee: FuncId,
        g: &Func,
        args: &[Reg],
        dst: Option<Reg>,
        obj: Option<Reg>,
    ) {
        // SAFETY: `Mem` fields, the callee's native type, well-formed blocks of c.f.
        unsafe {
            let (slow, join) = self.slow_path(c, pc, i);
            // A protected call: the process must be allowed the shared state (else the
            // interpreter's error), and the callee allocates in the shared heap when the object
            // is there (`Machine::call_prot`).
            let flags = self.mem_ld(c, MEM_FLAGS, self.i64);
            if let Some(o) = obj {
                let serial = LLVMBuildAnd(
                    self.b,
                    flags,
                    self.c64(yavs_interp::TIER_SERIAL as i64),
                    c"".as_ptr(),
                );
                self.guard(c, self.icmp(IP::LLVMIntNE, serial, self.c64(0)), slow);
                let tag = LLVMBuildLShr(self.b, self.ld(c, o), self.c64(56), c"".as_ptr());
                let shared = self.icmp(
                    IP::LLVMIntEQ,
                    tag,
                    self.c64(i64::from(yavs_interp::SHARED_TAG)),
                );
                let inner = LLVMBuildOr(self.b, flags, self.zext(shared), c"".as_ptr());
                self.mem_st(c, MEM_FLAGS, inner);
            }
            let depth = match &c.nat {
                Some(n) => n.depth,
                None => self.mem_ld(c, MEM_FRAME_IDX, self.i64),
            };
            let d1 = LLVMBuildAdd(self.b, depth, self.c64(1), c"".as_ptr());
            let max = self.mem_ld(c, MEM_MAX_DEPTH, self.i64);
            let and = |a, b| LLVMBuildAnd(self.b, a, b, c"".as_ptr());
            let mut ok = and(
                self.icmp(IP::LLVMIntULT, d1, max),
                self.icmp(IP::LLVMIntULT, d1, self.c64(1 << 29)),
            );
            let mut zero = [self.c32(0)];
            let fa = LLVMBuildCall2(
                self.b,
                self.frame_addr.0,
                self.frame_addr.1,
                zero.as_mut_ptr(),
                1,
                c"".as_ptr(),
            );
            let fa = LLVMBuildPtrToInt(self.b, fa, self.i64, c"".as_ptr());
            let lo = self.mem_ld(c, MEM_STACK_LO, self.i64);
            ok = and(ok, self.icmp(IP::LLVMIntUGT, fa, lo));
            let ty = self.native_ty(g);
            let fp = if self.direct {
                self.declare(&format!("f{}_n", callee.0), ty)
            } else {
                let tab = self.mem_ld(c, MEM_NTAB, self.ptr);
                let at = self.gep(tab, self.c64(i64::from(callee.0) * 8));
                let fp = LLVMBuildLoad2(self.b, self.ptr, at, c"".as_ptr());
                LLVMSetOrdering(
                    fp,
                    llvm_sys::LLVMAtomicOrdering::LLVMAtomicOrderingMonotonic,
                );
                ok = and(ok, LLVMBuildIsNotNull(self.b, fp, c"".as_ptr()));
                fp
            };
            self.guard(c, ok, slow);
            // The callee collects nothing and cannot see the slot variables; the registers go to
            // the frame for the way out below.
            self.flush_regs(c, pc);
            let mut cells = vec![c.env, c.mem, d1];
            if let Some(p) = g.parent {
                // The enclosing frames: the caller's own frame when it encloses the callee, else
                // those its own static links name.
                let up = |k: u64| match &c.nat {
                    Some(n) => n.up[k as usize],
                    None => self.mem_ld(c, MEM_UP + 8 * k, self.i64),
                };
                let (u1, u2) = if p == caller {
                    (self.own_hi(c), up(0))
                } else {
                    (up(0), up(1))
                };
                debug_assert!(f.parent == Some(p) || p == caller);
                let known = self.icmp(IP::LLVMIntNE, u1, self.c64(-1));
                self.guard(c, known, slow);
                cells.extend([u1, u2]);
            }
            if let Some(o) = obj {
                cells.push(self.ld(c, o));
            }
            for (r, p) in args
                .iter()
                .zip(g.params.iter().skip(usize::from(obj.is_some())))
            {
                cells.push(self.ld(c, *r));
                if p.width == Width::Sig {
                    cells.push(self.ld(c, Reg(r.0 + 1)));
                }
            }
            let v = LLVMBuildCall2(
                self.b,
                ty,
                fp,
                cells.as_mut_ptr(),
                cells.len() as u32,
                c"".as_ptr(),
            );
            if obj.is_some() {
                self.mem_st(c, MEM_FLAGS, flags);
            }
            let nd = self.mem_ld(c, MEM_NDEOPT, self.i64);
            let (out, cont) = (self.block(c), self.block(c));
            LLVMBuildCondBr(self.b, self.icmp(IP::LLVMIntNE, nd, self.c64(0)), out, cont);
            LLVMPositionBuilderAtEnd(self.b, out);
            match &c.nat {
                Some(_) => self.leave(c, self.c32(pc)),
                None => {
                    self.flush(c, pc as usize);
                    let mut a = [c.env, c.mem, self.c32(pc)];
                    let r = LLVMBuildCall2(
                        self.b,
                        self.deopt_end_ty,
                        self.deopt_end,
                        a.as_mut_ptr(),
                        3,
                        c"".as_ptr(),
                    );
                    self.leave(c, r);
                }
            }
            LLVMPositionBuilderAtEnd(self.b, cont);
            if let Some(d) = dst {
                self.st(c, d, v);
            }
            self.join(c, join);
        }
    }

    /// `n` bytes of stack (8-aligned) allocated in the entry block, so a loop does not grow
    /// the stack; the builder stays where it is.
    unsafe fn entry_alloca(&self, c: &Cur, bytes: u64) -> LLVMValueRef {
        // SAFETY: the entry block of c.f exists; the builder returns to its block.
        unsafe {
            let here = LLVMGetInsertBlock(self.b);
            let entry = LLVMGetEntryBasicBlock(c.f);
            match LLVMGetFirstInstruction(entry) {
                first if first.is_null() => LLVMPositionBuilderAtEnd(self.b, entry),
                first => LLVMPositionBuilderBefore(self.b, first),
            }
            let ty = LLVMArrayType2(self.i64, bytes.div_ceil(8).max(1));
            let a = LLVMBuildAlloca(self.b, ty, c"".as_ptr());
            LLVMPositionBuilderAtEnd(self.b, here);
            a
        }
    }

    /// `yavs_rt_troot(addr, ty, desc, n)`: 1 when recorded.
    unsafe fn troot_call(
        &self,
        c: &Cur,
        addr: LLVMValueRef,
        ty: u32,
        desc: LLVMValueRef,
        n: u64,
    ) -> LLVMValueRef {
        // SAFETY: the arguments match `troot_ty`.
        unsafe {
            let mut a = [c.env, c.mem, addr, self.c32(ty), desc, self.c64(n as i64)];
            LLVMBuildCall2(
                self.b,
                self.troot_ty,
                self.troot,
                a.as_mut_ptr(),
                6,
                c"".as_ptr(),
            )
        }
    }

    /// `n` copies of the `size` bytes at `src` from `dst` on (`Inst::Fill`, the pattern of a
    /// `NewFrame`); `src` must not overlap `dst`.
    unsafe fn fill_loop(
        &self,
        c: &Cur,
        dst: LLVMValueRef,
        src: LLVMValueRef,
        n: LLVMValueRef,
        size: u32,
    ) {
        // SAFETY: well-formed blocks of c.f; `dst` has room for `n * size` bytes.
        unsafe {
            if size == 1 {
                let b = self.load(self.i8, src);
                LLVMBuildMemSet(self.b, dst, b, n, 1);
                return;
            }
            let (head, body, done) = (self.block(c), self.block(c), self.block(c));
            let pre = LLVMGetInsertBlock(self.b);
            LLVMBuildBr(self.b, head);
            LLVMPositionBuilderAtEnd(self.b, head);
            let i = LLVMBuildPhi(self.b, self.i64, c"".as_ptr());
            LLVMBuildCondBr(self.b, self.icmp(IP::LLVMIntULT, i, n), body, done);
            LLVMPositionBuilderAtEnd(self.b, body);
            let at = self.gep(
                dst,
                LLVMBuildMul(self.b, i, self.c64(i64::from(size)), c"".as_ptr()),
            );
            LLVMBuildMemCpy(self.b, at, 1, src, 1, self.c64(i64::from(size)));
            let next = LLVMBuildAdd(self.b, i, self.c64(1), c"".as_ptr());
            LLVMBuildBr(self.b, head);
            let mut vals = [self.c64(0), next];
            let mut from = [pre, body];
            LLVMAddIncoming(i, vals.as_mut_ptr(), from.as_mut_ptr(), 2);
            LLVMPositionBuilderAtEnd(self.b, done);
        }
    }

    /// `Inst::NewFrame` on a native frame: a frame-pinned temporary of `dsize` descriptor bytes
    /// and `size` data bytes (copied from `data`, or the `fill` bytes at `data` repeated), and
    /// the frame's statements then keep it (`temp_base`).
    #[allow(clippy::too_many_arguments)]
    unsafe fn new_frame(
        &self,
        c: &Cur,
        pc: u32,
        i: &Inst,
        dst: Reg,
        desc: Reg,
        dsize: u32,
        data: Reg,
        size: Reg,
        fill: u32,
        fk: &dyn Fn(Reg) -> Option<Known>,
    ) {
        // SAFETY: integer arithmetic on registers, `host` pointers, `Mem` fields.
        unsafe {
            let (slow, join) = self.slow_path(c, pc, i);
            let sz = self.ld(c, size);
            // Over 4 GiB is the interpreter's error; a size that is no multiple of the
            // element is its internal error.
            let mut ok = self.icmp(IP::LLVMIntULE, sz, self.c64(1 << 32));
            if fill != 0 {
                let r = LLVMBuildURem(self.b, sz, self.c64(i64::from(fill)), c"".as_ptr());
                ok = LLVMBuildAnd(
                    self.b,
                    ok,
                    self.icmp(IP::LLVMIntEQ, r, self.c64(0)),
                    c"".as_ptr(),
                );
            }
            self.guard(c, ok, slow);
            // The sources before the allocation: a failure leaves nothing changed.
            let ds = self.c64(i64::from(dsize));
            let pdesc = (dsize != 0)
                .then(|| self.host(c, self.ld(c, desc), fk(desc), self.c64(0), ds, false, slow));
            let n_src = if fill == 0 {
                sz
            } else {
                self.c64(i64::from(fill))
            };
            let psrc = self.host(
                c,
                self.ld(c, data),
                fk(data),
                self.c64(0),
                n_src,
                false,
                slow,
            );
            let total = LLVMBuildAdd(self.b, ds, sz, c"".as_ptr());
            let (at, p) = self.temp_alloc(c, total, slow);
            if let Some(pd) = pdesc {
                LLVMBuildMemCpy(self.b, p, 1, pd, 1, ds);
            }
            let val = self.gep(p, ds);
            if fill == 0 {
                LLVMBuildMemCpy(self.b, val, 1, psrc, 1, sz);
            } else {
                let count = LLVMBuildUDiv(self.b, sz, self.c64(i64::from(fill)), c"".as_ptr());
                self.fill_loop(c, val, psrc, count, fill);
            }
            // The frame's own statements must not free the object.
            if let Some(n) = &c.nat {
                let end = self.mem_ld(c, MEM_TEMP_LEN, self.i64);
                LLVMBuildStore(self.b, end, n.temp_base);
            }
            self.st(c, dst, at);
            self.join(c, join);
        }
    }

    /// `Inst::Fill` on a native frame: `n` copies of the `size` bytes at `elem` from `dst` on.
    #[allow(clippy::too_many_arguments)]
    unsafe fn fill(
        &self,
        c: &Cur,
        pc: u32,
        i: &Inst,
        dst: Reg,
        elem: Reg,
        n: Reg,
        size: u32,
        fk: &dyn Fn(Reg) -> Option<Known>,
    ) {
        // SAFETY: integer arithmetic on registers and `host` pointers.
        unsafe {
            let (slow, join) = self.slow_path(c, pc, i);
            let cnt = self.ld(c, n);
            let (total, o) = self.ovf(self.umul, cnt, self.c64(i64::from(size)));
            self.guard(c, LLVMBuildNot(self.b, o, c"".as_ptr()), slow);
            let sz = self.c64(i64::from(size));
            let pe = self.host(c, self.ld(c, elem), fk(elem), self.c64(0), sz, false, slow);
            let pd = self.host(c, self.ld(c, dst), fk(dst), self.c64(0), total, true, slow);
            // The element is read before anything is written (it may lie in the target).
            let src = if size == 1 {
                pe
            } else {
                let stage = self.entry_alloca(c, u64::from(size));
                LLVMBuildMemCpy(self.b, stage, 1, pe, 1, sz);
                stage
            };
            self.fill_loop(c, pd, src, cnt, size);
            self.join(c, join);
        }
    }

    /// `Inst::Cat` of non-logic elements on a native frame: the pieces (`rs`: pairs of an
    /// address and a count, or an element value for a scalar piece) one after another from
    /// `dst` on, `elem` bytes per element. Every address is found before anything is written.
    #[allow(clippy::too_many_arguments)]
    unsafe fn cat_bytes(
        &self,
        c: &Cur,
        pc: u32,
        i: &Inst,
        dst: Reg,
        elem: Reg,
        rs: &[Reg],
        scalars: u16,
        w: Width,
        fk: &dyn Fn(Reg) -> Option<Known>,
    ) {
        // SAFETY: integer arithmetic on registers and `host` pointers.
        unsafe {
            enum Piece {
                Scalar(LLVMValueRef, LLVMValueRef),
                Bytes(LLVMValueRef, LLVMValueRef, LLVMValueRef),
            }
            let (slow, join) = self.slow_path(c, pc, i);
            let m = self.ld(c, elem);
            let d = self.ld(c, dst);
            let mut k = self.c64(0);
            let mut pieces = Vec::new();
            for (j, pr) in rs.as_chunks::<2>().0.iter().enumerate() {
                let (off, o) = self.ovf(self.umul, k, m);
                let small = self.icmp(IP::LLVMIntULE, off, self.c64(0xffff_ffff));
                self.guard(
                    c,
                    LLVMBuildAnd(
                        self.b,
                        LLVMBuildNot(self.b, o, c"".as_ptr()),
                        small,
                        c"".as_ptr(),
                    ),
                    slow,
                );
                if scalars >> j & 1 == 1 {
                    let nb = self.c64(i64::from(w.bytes()));
                    let pd = self.host(c, d, fk(dst), off, nb, true, slow);
                    pieces.push(Piece::Scalar(pd, self.ld(c, pr[0])));
                    k = LLVMBuildAdd(self.b, k, self.c64(1), c"".as_ptr());
                } else {
                    let len = self.ld(c, pr[1]);
                    let (nbytes, o) = self.ovf(self.umul, len, m);
                    self.guard(c, LLVMBuildNot(self.b, o, c"".as_ptr()), slow);
                    let ps = self.host(
                        c,
                        self.ld(c, pr[0]),
                        fk(pr[0]),
                        self.c64(0),
                        nbytes,
                        false,
                        slow,
                    );
                    let pd = self.host(c, d, fk(dst), off, nbytes, true, slow);
                    pieces.push(Piece::Bytes(pd, ps, nbytes));
                    k = LLVMBuildAdd(self.b, k, len, c"".as_ptr());
                }
            }
            for p in pieces {
                match p {
                    Piece::Scalar(pd, v) => {
                        let v = if w == Width::B8 {
                            v
                        } else {
                            LLVMBuildTrunc(self.b, v, self.width_ty(w), c"".as_ptr())
                        };
                        self.store(v, pd);
                    }
                    Piece::Bytes(pd, ps, n) => {
                        LLVMBuildMemMove(self.b, pd, 1, ps, 1, n);
                    }
                }
            }
            self.join(c, join);
        }
    }

    /// `Inst::CallNative` of a native `yavs_interp::direct_native` allows, on a native frame:
    /// the argument cells go to `TierHost::native_direct`; where it does not run the native
    /// the frames are materialised and the interpreter executes the call.
    #[allow(clippy::too_many_arguments)]
    unsafe fn native_direct(
        &self,
        c: &Cur,
        pc: u32,
        i: &Inst,
        f: &Func,
        native: yavs_ir::NativeId,
        args: yavs_ir::Regs,
        dst: Option<Reg>,
    ) -> Result<(), String> {
        // SAFETY: the arguments match `native_ty`; well-formed blocks of c.f.
        unsafe {
            let nat = c.nat.as_ref().ok_or("native call outside a native frame")?;
            let rs = f.regs(args).ok_or("operand list out of range")?;
            let mut cells = Vec::new();
            for &r in rs {
                cells.push(r);
                if f.reg_ty.get(r.0 as usize) == Some(&yavs_ir::RegTy::Sig) {
                    cells.push(Reg(r.0 + 1));
                }
            }
            let (slow, join) = self.slow_path(c, pc, i);
            let buf = self.entry_alloca(c, cells.len() as u64 * 8);
            for (k, &r) in cells.iter().enumerate() {
                self.store(self.ld(c, r), self.gep(buf, self.c64(k as i64 * 8)));
            }
            let mut a = [
                c.env,
                c.mem,
                self.c32(native.0),
                buf,
                self.c32(cells.len() as u32),
                nat.depth,
                nat.scratch,
            ];
            let st = LLVMBuildCall2(
                self.b,
                self.native_ty,
                self.native,
                a.as_mut_ptr(),
                7,
                c"".as_ptr(),
            );
            let kind = LLVMBuildAnd(self.b, st, self.c32(0xff), c"".as_ptr());
            let mut ok = self.icmp(IP::LLVMIntNE, kind, self.c32(0));
            if dst.is_some() {
                let has = LLVMBuildAnd(self.b, st, self.c32(0x100), c"".as_ptr());
                ok = LLVMBuildAnd(
                    self.b,
                    ok,
                    self.icmp(IP::LLVMIntNE, has, self.c32(0)),
                    c"".as_ptr(),
                );
            }
            self.guard(c, ok, slow);
            if let Some(d) = dst {
                let v = LLVMBuildLoad2(self.b, self.i64, nat.scratch, c"".as_ptr());
                self.st(c, d, v);
            }
            // The native ended the process: the frames are materialised after the call.
            let (out, cont) = (self.block(c), self.block(c));
            LLVMBuildCondBr(
                self.b,
                self.icmp(IP::LLVMIntEQ, kind, self.c32(2)),
                out,
                cont,
            );
            LLVMPositionBuilderAtEnd(self.b, out);
            self.leave(c, self.c32(pc + 1));
            LLVMPositionBuilderAtEnd(self.b, cont);
            self.join(c, join);
            Ok(())
        }
    }

    /// The instruction at `pc` where the generator has no code for it: a native frame has the
    /// interpreter execute it on a frame of its own (`xstep_ok`), a frame-mode function steps it.
    unsafe fn slow_inst(&self, c: &Cur, pc: u32, i: &Inst, f: &Func) -> Result<(), String> {
        // SAFETY: as `native_step` and `step`.
        unsafe {
            if c.nat.is_some() && xstep_ok(f, i) {
                self.native_step(c, pc, i, f)
            } else {
                self.step(c, pc, i);
                Ok(())
            }
        }
    }

    /// An instruction of a native frame the interpreter executes on a frame of its own
    /// (`TierHost::native_step`); the registers it reads go in, those it writes come back.
    unsafe fn native_step(&self, c: &Cur, pc: u32, i: &Inst, f: &Func) -> Result<(), String> {
        // SAFETY: the arguments match `xstep_ty`; well-formed blocks of c.f.
        unsafe {
            let nat = c.nat.as_ref().ok_or("native step outside a native frame")?;
            let (uses, defs) = yavs_interp::inst_regs(f, i);
            let (slow, join) = self.slow_path(c, pc, i);
            let ins = self.entry_alloca(c, uses.len() as u64 * 8);
            let outs = self.entry_alloca(c, defs.len() as u64 * 8);
            for (k, &r) in uses.iter().enumerate() {
                self.store(self.ld(c, r), self.gep(ins, self.c64(k as i64 * 8)));
            }
            let idx = |rs: &[Reg]| self.u32_table(&rs.iter().map(|r| r.0).collect::<Vec<_>>());
            let mut a = [
                c.env,
                c.mem,
                self.c32(nat.id.0),
                self.c32(pc),
                nat.depth,
                idx(&uses),
                ins,
                self.c32(uses.len() as u32),
                idx(&defs),
                outs,
                self.c32(defs.len() as u32),
                self.temp_base(c),
            ];
            let st = LLVMBuildCall2(
                self.b,
                self.xstep_ty,
                self.xstep,
                a.as_mut_ptr(),
                12,
                c"".as_ptr(),
            );
            self.guard(c, self.icmp(IP::LLVMIntNE, st, self.c32(0)), slow);
            for (k, &r) in defs.iter().enumerate() {
                let v = self.load(self.i64, self.gep(outs, self.c64(k as i64 * 8)));
                self.st(c, r, v);
            }
            let (out, cont) = (self.block(c), self.block(c));
            LLVMBuildCondBr(self.b, self.icmp(IP::LLVMIntEQ, st, self.c32(2)), out, cont);
            LLVMPositionBuilderAtEnd(self.b, out);
            self.leave(c, self.c32(pc + 1));
            LLVMPositionBuilderAtEnd(self.b, cont);
            self.join(c, join);
            Ok(())
        }
    }

    /// Materialise a native frame at `pc` (its exit block); the builder goes on in a new block,
    /// which nothing reaches.
    unsafe fn deopt_here(&self, c: &Cur, pc: u32) {
        // SAFETY: well-formed blocks of c.f.
        unsafe {
            self.leave(c, self.c32(pc));
            let dead = self.block(c);
            LLVMPositionBuilderAtEnd(self.b, dead);
        }
    }

    /// The temporary area's length the frame's statements truncate it to (`Inst::Stmt`).
    unsafe fn temp_base(&self, c: &Cur) -> LLVMValueRef {
        match &c.nat {
            // SAFETY: the frame's cell is an i64 `alloca`.
            Some(n) => unsafe { LLVMBuildLoad2(self.b, self.i64, n.temp_base, c"".as_ptr()) },
            // SAFETY: a `Mem` field.
            None => unsafe { self.mem_ld(c, MEM_TEMP_BASE, self.i64) },
        }
    }

    /// The high half of the addresses of the function's own frame.
    unsafe fn own_hi(&self, c: &Cur) -> LLVMValueRef {
        match &c.nat {
            Some(n) => n.fhi,
            // SAFETY: a `Mem` field.
            None => unsafe { self.mem_ld(c, MEM_FRAME, self.i64) },
        }
    }

    /// The function's own slots, as `own_slots` finds them, once per stretch of straight-line
    /// code and carried across the blocks of a loop: the base moves only when a callback
    /// pushes a frame (`call_go`, and `join` after a slow path that may). Main-path code only
    /// (the value must dominate what follows).
    unsafe fn own_slots_cached(&self, c: &Cur) -> LLVMValueRef {
        let Some(d) = c.defer.as_ref().filter(|_| c.nat.is_none()) else {
            // SAFETY: as `own_slots`.
            return unsafe { self.own_slots(c) };
        };
        let b = d.pend.borrow().len() - 1;
        if let Some(v) = d.pend.borrow()[b] {
            return v;
        }
        // SAFETY: as `own_slots`.
        let v = unsafe { self.own_slots(c) };
        d.pend.borrow_mut()[b] = Some(v);
        v
    }

    /// The function's own slots.
    unsafe fn own_slots(&self, c: &Cur) -> LLVMValueRef {
        match &c.nat {
            Some(n) => n.slots,
            // SAFETY: a `Mem` field.
            None => unsafe { self.mem_ld(c, MEM_SLOTS, self.ptr) },
        }
    }

    /// Where a helper writes register `dst` (the registers and its index; a native frame's
    /// scratch word and 0); `reg_in` moves it into the register.
    unsafe fn reg_out(&self, c: &Cur, dst: Reg) -> (LLVMValueRef, u32) {
        match &c.nat {
            Some(n) => (n.scratch, 0),
            None => {
                // The helper writes the frame's register.
                if let Some(d) = &c.defer
                    && let Some(p) = d.pend.borrow_mut().get_mut(dst.0 as usize)
                {
                    *p = None;
                }
                (c.regs, dst.0)
            }
        }
    }

    unsafe fn reg_in(&self, c: &Cur, dst: Reg) {
        if let Some(n) = &c.nat {
            // SAFETY: the scratch word is an i64 `alloca`.
            unsafe {
                let v = LLVMBuildLoad2(self.b, self.i64, n.scratch, c"".as_ptr());
                self.st(c, dst, v);
            }
        }
    }

    /// `fast_op` of an operation that writes register `dst`.
    unsafe fn fast_reg(
        &self,
        c: &Cur,
        op: u32,
        dst: Reg,
        a: LLVMValueRef,
        b: LLVMValueRef,
        slow: LLVMBasicBlockRef,
    ) {
        // SAFETY: as `fast_op`.
        unsafe {
            let (_, d) = self.reg_out(c, dst);
            self.fast_op(c, op, d, a, b, slow);
            self.reg_in(c, dst);
        }
    }

    /// `yavs_rt_fast` operation `op` on `a` and `b` (result register or count `dst`), taking
    /// `slow` when it returns 0: the interpreter then executes the instruction.
    unsafe fn fast_op(
        &self,
        c: &Cur,
        op: u32,
        dst: u32,
        a: LLVMValueRef,
        b: LLVMValueRef,
        slow: LLVMBasicBlockRef,
    ) {
        // SAFETY: the arguments match `fast_ty`; well-formed blocks of c.f.
        unsafe {
            // A native frame's registers are not the helpers' to see.
            let rp = c.nat.as_ref().map_or(c.regs, |n| n.scratch);
            let mut args = [c.env, rp, c.mem, self.c32(op), self.c32(dst), a, b];
            let ok = LLVMBuildCall2(
                self.b,
                self.fast_ty,
                self.fast,
                args.as_mut_ptr(),
                7,
                c"".as_ptr(),
            );
            // Alloc writes the temporary area, the others only read memory besides the
            // register and the object cache.
            let other = match op {
                FAST_ALLOC => MODS,
                FAST_DEREF => 0,
                _ => REF,
            };
            self.attr(ok, true, "memory", mem_effects(RW, RW, other));
            self.attr(ok, true, "willreturn", 0);
            self.guard(c, self.icmp(IP::LLVMIntNE, ok, self.c32(0)), slow);
        }
    }

    /// Call a callback that returns `GO` or what the compiled function returns at once. A
    /// `quiet` one (an instruction that neither collects nor runs other code) cannot look at
    /// slot variables, so they are written back only when it fails.
    unsafe fn call_go(
        &self,
        c: &Cur,
        ty: LLVMTypeRef,
        f: LLVMValueRef,
        args: &mut [LLVMValueRef],
        quiet: bool,
    ) {
        // SAFETY: `args` match `ty`; well-formed blocks of c.f.
        unsafe {
            // The callback reads the registers of its instruction from the frame.
            let mut key = Vec::new();
            if let Some(d) = &c.defer {
                if quiet {
                    key = self.flush_regs(c, d.pc.get());
                } else {
                    self.flush(c, d.pc.get() as usize);
                }
            }
            let r = LLVMBuildCall2(
                self.b,
                ty,
                f,
                args.as_mut_ptr(),
                args.len() as u32,
                c"".as_ptr(),
            );
            // The callback may have pushed a frame, which moves the slots.
            if let Some(d) = &c.defer {
                *d.pend.borrow_mut().last_mut().expect("base") = None;
            }
            let cont = self.block(c);
            let ok = self.icmp(IP::LLVMIntEQ, r, self.c32(GO));
            if key.is_empty() {
                let exit = self.exit_for(c, r);
                LLVMBuildCondBr(self.b, ok, cont, exit);
            } else {
                let fail = self.block(c);
                LLVMBuildCondBr(self.b, ok, cont, fail);
                LLVMPositionBuilderAtEnd(self.b, fail);
                self.leave_slots(c, key, r);
            }
            LLVMPositionBuilderAtEnd(self.b, cont);
        }
    }

    unsafe fn range_exit(
        &self,
        c: &Cur,
        x: LLVMValueRef,
        lo: LLVMValueRef,
        hi: LLVMValueRef,
        pc: u32,
    ) {
        // SAFETY: i64 operands.
        unsafe {
            let a = self.icmp(IP::LLVMIntSLT, x, lo);
            let b = self.icmp(IP::LLVMIntSGT, x, hi);
            self.exit_if(c, self.or(a, b), pc);
        }
    }

    /// A block that steps the instruction `i` at `pc` in the interpreter and goes on at the
    /// returned join block: the slow path of a native instruction. The builder stays.
    unsafe fn slow_path(
        &self,
        c: &Cur,
        pc: u32,
        i: &Inst,
    ) -> (LLVMBasicBlockRef, LLVMBasicBlockRef) {
        // SAFETY: well-formed blocks of c.f.
        unsafe {
            let here = LLVMGetInsertBlock(self.b);
            let (slow, join) = (self.block(c), self.block(c));
            LLVMPositionBuilderAtEnd(self.b, slow);
            // The slow path is no business of the fast path's: what it found or forgot about the
            // frame's base does not hold at the join (the fast path may define the base after the
            // branch, which does not dominate the join); the join takes the value from before,
            // unless the step may push a frame.
            let base = c
                .defer
                .as_ref()
                .map(|d| *d.pend.borrow().last().expect("base"));
            self.step(c, pc, i);
            if let (Some(d), Some(b)) = (&c.defer, base) {
                *d.pend.borrow_mut().last_mut().expect("base") = b;
                c.joins.borrow_mut().push((b, !quiet(i)));
            }
            LLVMBuildBr(self.b, join);
            LLVMPositionBuilderAtEnd(self.b, here);
            (slow, join)
        }
    }

    /// Go on in a new block when `ok`, else take `slow`.
    unsafe fn guard(&self, c: &Cur, ok: LLVMValueRef, slow: LLVMBasicBlockRef) {
        // SAFETY: well-formed blocks of c.f.
        unsafe {
            let go = self.block(c);
            LLVMBuildCondBr(self.b, ok, go, slow);
            LLVMPositionBuilderAtEnd(self.b, go);
        }
    }

    /// End the fast path at the slow path's join block.
    unsafe fn join(&self, c: &Cur, join: LLVMBasicBlockRef) {
        // SAFETY: the builder is in a block of the function of `join`.
        unsafe {
            LLVMBuildBr(self.b, join);
            LLVMPositionBuilderAtEnd(self.b, join);
        }
        if let Some(d) = &c.defer
            && let Some((b, loud)) = c.joins.borrow_mut().pop()
        {
            *d.pend.borrow_mut().last_mut().expect("base") = if loud { None } else { b };
        }
    }

    /// A private constant array of bytes.
    unsafe fn table(&self, t: &[u8]) -> LLVMValueRef {
        // SAFETY: a global of our module with a matching initialiser.
        unsafe {
            let mut vals: Vec<LLVMValueRef> = t
                .iter()
                .map(|&b| LLVMConstInt(self.i8, u64::from(b), 0))
                .collect();
            let ty = LLVMArrayType2(self.i8, t.len() as u64);
            let g = LLVMAddGlobal(self.module, ty, c"".as_ptr());
            LLVMSetInitializer(
                g,
                LLVMConstArray2(self.i8, vals.as_mut_ptr(), t.len() as u64),
            );
            LLVMSetGlobalConstant(g, 1);
            LLVMSetLinkage(g, llvm_sys::LLVMLinkage::LLVMPrivateLinkage);
            g
        }
    }

    /// A private constant array of `u32`.
    unsafe fn u32_table(&self, t: &[u32]) -> LLVMValueRef {
        // SAFETY: a global of our module with a matching initialiser.
        unsafe {
            let mut vals: Vec<LLVMValueRef> = t.iter().map(|&b| self.c32(b)).collect();
            let ty = LLVMArrayType2(self.i32, t.len() as u64);
            let g = LLVMAddGlobal(self.module, ty, c"".as_ptr());
            LLVMSetInitializer(
                g,
                LLVMConstArray2(self.i32, vals.as_mut_ptr(), t.len() as u64),
            );
            LLVMSetGlobalConstant(g, 1);
            LLVMSetLinkage(g, llvm_sys::LLVMLinkage::LLVMPrivateLinkage);
            g
        }
    }

    /// A private global copy of constant `k` and its length, made once per function; `None`
    /// for a constant the interpreter expands (compact forms) or one over 4 KiB.
    unsafe fn const_global(&self, c: &Cur, k: yavs_ir::ConstId) -> Option<(LLVMValueRef, u64)> {
        let yavs_ir::ConstData::Bytes(b) = c.prog.consts.get(k)? else {
            return None;
        };
        if b.len() > 4096 {
            return None;
        }
        let mut m = c.consts.borrow_mut();
        // SAFETY: a global of our module (`table`).
        let g = *m.entry(k).or_insert_with(|| unsafe { self.table(b) });
        Some((g, b.len() as u64))
    }

    unsafe fn zext8(&self, v: LLVMValueRef) -> LLVMValueRef {
        // SAFETY: v is an i8.
        unsafe { LLVMBuildZExt(self.b, v, self.i64, c"".as_ptr()) }
    }

    unsafe fn mem_ptr(&self, c: &Cur, field: u64) -> LLVMValueRef {
        // SAFETY: `field` is an offset in `Mem`.
        unsafe { self.gep(c.mem, self.c64(field as i64)) }
    }

    unsafe fn mem_ld(&self, c: &Cur, field: u64, ty: LLVMTypeRef) -> LLVMValueRef {
        // SAFETY: the field at `field` has type `ty`.
        unsafe { LLVMBuildLoad2(self.b, ty, self.mem_ptr(c, field), c"".as_ptr()) }
    }

    unsafe fn mem_st(&self, c: &Cur, field: u64, v: LLVMValueRef) {
        // SAFETY: the field at `field` has the type of `v`.
        unsafe {
            LLVMBuildStore(self.b, v, self.mem_ptr(c, field));
        }
    }

    unsafe fn gep(&self, p: LLVMValueRef, off: LLVMValueRef) -> LLVMValueRef {
        // SAFETY: byte offset from a pointer.
        unsafe {
            let mut idx = [off];
            let g = LLVMBuildGEP2(self.b, self.i8, p, idx.as_mut_ptr(), 1, c"".as_ptr());
            let k = self.kinds.borrow().get(&(p as usize)).copied();
            if let Some(k) = k {
                self.tag(g, k);
            }
            g
        }
    }

    /// Remember that `p` points into memory of kind `k` (`AA_*`).
    fn tag(&self, p: LLVMValueRef, k: usize) -> LLVMValueRef {
        self.kinds.borrow_mut().insert(p as usize, k);
        p
    }

    /// Alias scope metadata on the access `inst` through `p`, if its kind is known.
    unsafe fn scope(&self, inst: LLVMValueRef, p: LLVMValueRef) {
        let Some(k) = self.kinds.borrow().get(&(p as usize)).copied() else {
            return;
        };
        // SAFETY: metadata nodes of our context attached to an instruction of our module.
        unsafe {
            let mut aa = self.aa.borrow_mut();
            let n = aa.get_or_insert_with(|| {
                let md = |ops: &mut [LLVMMetadataRef]| {
                    LLVMMDNodeInContext2(self.ctx, ops.as_mut_ptr(), ops.len())
                };
                let name = |s: &str| LLVMMDStringInContext2(self.ctx, s.as_ptr().cast(), s.len());
                let domain = md(&mut [name("yavs memory")]);
                let names = ["slots", "temp", "inst", "const", "codes"];
                let scopes: Vec<LLVMMetadataRef> =
                    names.iter().map(|n| md(&mut [name(n), domain])).collect();
                let v = |m| LLVMMetadataAsValue(self.ctx, m);
                let lists = |ix: &[usize]| {
                    let mut ops: Vec<LLVMMetadataRef> = ix.iter().map(|&i| scopes[i]).collect();
                    v(md(&mut ops))
                };
                AaNodes {
                    scope: std::array::from_fn(|i| lists(&[i])),
                    others: std::array::from_fn(|i| {
                        let ix: Vec<usize> = (0..AA_N).filter(|&j| j != i).collect();
                        lists(&ix)
                    }),
                    k_scope: LLVMGetMDKindIDInContext(self.ctx, c"alias.scope".as_ptr(), 11),
                    k_noalias: LLVMGetMDKindIDInContext(self.ctx, c"noalias".as_ptr(), 7),
                }
            });
            LLVMSetMetadata(inst, n.k_scope, n.scope[k]);
            LLVMSetMetadata(inst, n.k_noalias, n.others[k]);
        }
    }

    /// An unaligned load.
    unsafe fn load(&self, ty: LLVMTypeRef, p: LLVMValueRef) -> LLVMValueRef {
        // SAFETY: `p` points to a value of `ty`.
        unsafe {
            let v = LLVMBuildLoad2(self.b, ty, p, c"".as_ptr());
            LLVMSetAlignment(v, 1);
            self.scope(v, p);
            v
        }
    }

    /// An unaligned store.
    unsafe fn store(&self, v: LLVMValueRef, p: LLVMValueRef) {
        // SAFETY: `p` points to memory for a value of `v`'s type.
        unsafe {
            let s = LLVMBuildStore(self.b, v, p);
            LLVMSetAlignment(s, 1);
            self.scope(s, p);
        }
    }

    fn width_ty(&self, w: Width) -> LLVMTypeRef {
        match w {
            Width::B1 => self.i8,
            Width::B4 => self.i32,
            _ => self.i64,
        }
    }

    /// The host pointer to `n` bytes at address `a` plus `extra`, taking `slow` unless they
    /// lie in the frame's slots (inline when the generator knows the address), a constant it
    /// has a copy of, or memory `yavs_rt_host` finds: what the interpreter's `rd`/`wr` would
    /// find there without an error. `at` is what the generator knows about `a`: an offset in the
    /// frame's slots, or in a constant, whose bytes compiled code then reads from a copy of
    /// its own (constants never change).
    #[allow(clippy::too_many_arguments)]
    unsafe fn host(
        &self,
        c: &Cur,
        a: LLVMValueRef,
        at: Option<Known>,
        extra: LLVMValueRef,
        n: LLVMValueRef,
        write: bool,
        slow: LLVMBasicBlockRef,
    ) -> LLVMValueRef {
        // SAFETY: loads of `Mem` fields, integer arithmetic, well-formed blocks of c.f.
        unsafe {
            if let Some(Known::Const(k, o)) = at {
                match self.const_global(c, k) {
                    // A write to a constant is the interpreter's error.
                    Some((g, len)) if !write => {
                        let o = LLVMBuildAdd(self.b, self.c64(o as i64), extra, c"".as_ptr());
                        let len = self.c64(len as i64);
                        let fits = self.icmp(IP::LLVMIntULE, n, len);
                        let rest = LLVMBuildSub(self.b, len, n, c"".as_ptr());
                        let inside = self.icmp(IP::LLVMIntULE, o, rest);
                        self.guard(c, LLVMBuildAnd(self.b, fits, inside, c"".as_ptr()), slow);
                        return self.gep(self.tag(g, AA_CONST), o);
                    }
                    _ => {}
                }
            }
            if let Some(Known::Frame(o)) = at {
                // Within the frame whatever the frame: no check.
                if !LLVMIsAConstantInt(extra).is_null() && !LLVMIsAConstantInt(n).is_null() {
                    let (e, k) = (LLVMConstIntGetZExtValue(extra), LLVMConstIntGetZExtValue(n));
                    if o.checked_add(e)
                        .and_then(|x| x.checked_add(k))
                        .is_some_and(|end| end <= c.frame_len)
                    {
                        let at = self.c64((o + e) as i64);
                        return self.gep(self.tag(self.own_slots_cached(c), AA_SLOTS), at);
                    }
                }
                // `extra` is below 2^63, the sum cannot wrap.
                let o = LLVMBuildAdd(self.b, self.c64(o as i64), extra, c"".as_ptr());
                let len = match &c.nat {
                    Some(_) => self.c64(c.frame_len as i64),
                    None => self.mem_ld(c, MEM_SLOTS_LEN, self.i64),
                };
                let fits = self.icmp(IP::LLVMIntULE, n, len);
                let rest = LLVMBuildSub(self.b, len, n, c"".as_ptr());
                let inside = self.icmp(IP::LLVMIntULE, o, rest);
                self.guard(c, LLVMBuildAnd(self.b, fits, inside, c"".as_ptr()), slow);
                return self.gep(self.tag(self.own_slots_cached(c), AA_SLOTS), o);
            }
            // The temporary area and (reading) the instance data, whose space the generator
            // knows: the bytes the view holds, bounds checked, no call.
            let area = match at {
                Some(Known::Temp) => Some((MEM_TEMP, MEM_TEMP_LEN)),
                Some(Known::Inst(_)) if !write => Some((MEM_INST, MEM_INST_LEN)),
                _ => None,
            };
            if let Some((base, len)) = area {
                let lo = LLVMBuildAnd(self.b, a, self.c64(0xffff_ffff), c"".as_ptr());
                let o = LLVMBuildAdd(self.b, lo, extra, c"".as_ptr());
                let len = self.mem_ld(c, len, self.i64);
                let fits = self.icmp(IP::LLVMIntULE, n, len);
                let rest = LLVMBuildSub(self.b, len, n, c"".as_ptr());
                let inside = self.icmp(IP::LLVMIntULE, o, rest);
                self.guard(c, LLVMBuildAnd(self.b, fits, inside, c"".as_ptr()), slow);
                let kind = if base == MEM_TEMP { AA_TEMP } else { AA_INST };
                return self.gep(self.tag(self.mem_ld(c, base, self.ptr), kind), o);
            }
            // A native frame's own slots are found here: the interpreter has no frame for it.
            let (own, other, merge) = match &c.nat {
                Some(nat) => {
                    let (own, other, merge) = (self.block(c), self.block(c), self.block(c));
                    let hi = LLVMBuildLShr(self.b, a, self.c64(32), c"".as_ptr());
                    LLVMBuildCondBr(self.b, self.icmp(IP::LLVMIntEQ, hi, nat.fhi), own, other);
                    LLVMPositionBuilderAtEnd(self.b, own);
                    let lo = LLVMBuildAnd(self.b, a, self.c64(0xffff_ffff), c"".as_ptr());
                    let o = LLVMBuildAdd(self.b, lo, extra, c"".as_ptr());
                    let len = self.c64(c.frame_len as i64);
                    let fits = self.icmp(IP::LLVMIntULE, n, len);
                    let rest = LLVMBuildSub(self.b, len, n, c"".as_ptr());
                    let inside = self.icmp(IP::LLVMIntULE, o, rest);
                    self.guard(c, LLVMBuildAnd(self.b, fits, inside, c"".as_ptr()), slow);
                    let p = self.gep(nat.slots, o);
                    let from = LLVMGetInsertBlock(self.b);
                    LLVMBuildBr(self.b, merge);
                    LLVMPositionBuilderAtEnd(self.b, other);
                    // The slots of a native caller, registered at its entry: a frame above the
                    // interpreter's top frame and below this one.
                    let (up, rest_blk) = (self.block(c), self.block(c));
                    let hi = LLVMBuildLShr(self.b, a, self.c64(32), c"".as_ptr());
                    let top = self.mem_ld(c, MEM_FRAME_IDX, self.i64);
                    let above = self.icmp(IP::LLVMIntUGT, hi, top);
                    let below = self.icmp(IP::LLVMIntULT, hi, nat.depth);
                    LLVMBuildCondBr(
                        self.b,
                        LLVMBuildAnd(self.b, above, below, c"".as_ptr()),
                        up,
                        rest_blk,
                    );
                    LLVMPositionBuilderAtEnd(self.b, up);
                    let tab = self.mem_ld(c, MEM_NSLOTS, self.ptr);
                    let ent = self.gep(tab, LLVMBuildMul(self.b, hi, self.c64(16), c"".as_ptr()));
                    let sp = self.load(self.ptr, ent);
                    let len = self.load(self.i64, self.gep(ent, self.c64(8)));
                    let lo = LLVMBuildAnd(self.b, a, self.c64(0xffff_ffff), c"".as_ptr());
                    let o = LLVMBuildAdd(self.b, lo, extra, c"".as_ptr());
                    let fits = self.icmp(IP::LLVMIntULE, n, len);
                    let rest = LLVMBuildSub(self.b, len, n, c"".as_ptr());
                    let inside = self.icmp(IP::LLVMIntULE, o, rest);
                    self.guard(c, LLVMBuildAnd(self.b, fits, inside, c"".as_ptr()), slow);
                    let pu = self.gep(sp, o);
                    let from_up = LLVMGetInsertBlock(self.b);
                    LLVMBuildBr(self.b, merge);
                    LLVMPositionBuilderAtEnd(self.b, rest_blk);
                    (vec![(p, from), (pu, from_up)], other, merge)
                }
                None => (Vec::new(), std::ptr::null_mut(), std::ptr::null_mut()),
            };
            // Other memory through `yavs_rt_host` (`Mem::find`, then the objects the interpreter
            // is asked for). The temporary area and the instance data are reached the same way
            // unless the generator knows the space (above): an inline test of the three view
            // spaces and its select chain cost 30 LLVM instructions per access of unknown space and
            // codegen time with them; the heap, which most such accesses are, needed the call
            // anyway, and the compiled code is no slower (#432).
            let mut args = [c.env, c.mem, a, extra, n, self.c32(u32::from(write))];
            let phi = LLVMBuildCall2(
                self.b,
                self.host_ty,
                self.host,
                args.as_mut_ptr(),
                6,
                c"".as_ptr(),
            );
            self.guard(c, LLVMBuildIsNotNull(self.b, phi, c"".as_ptr()), slow);
            let mut p = phi;
            if !own.is_empty() {
                let here = LLVMGetInsertBlock(self.b);
                LLVMBuildBr(self.b, merge);
                LLVMPositionBuilderAtEnd(self.b, merge);
                let phi = LLVMBuildPhi(self.b, self.ptr, c"".as_ptr());
                let mut vals: Vec<LLVMValueRef> = own.iter().map(|o| o.0).collect();
                let mut bbs: Vec<LLVMBasicBlockRef> = own.iter().map(|o| o.1).collect();
                vals.push(p);
                bbs.push(here);
                LLVMAddIncoming(phi, vals.as_mut_ptr(), bbs.as_mut_ptr(), vals.len() as u32);
                p = phi;
            }
            let _ = other;
            p
        }
    }

    /// `n` zeroed bytes of temporary memory as `Machine::temp_alloc` makes them: the address
    /// and the host pointer, taking `slow` when the area must grow or exceeds 4 GiB.
    unsafe fn temp_alloc(
        &self,
        c: &Cur,
        n: LLVMValueRef,
        slow: LLVMBasicBlockRef,
    ) -> (LLVMValueRef, LLVMValueRef) {
        // SAFETY: integer arithmetic on `Mem` fields; the zeroed bytes lie within the capacity.
        unsafe {
            let len = self.mem_ld(c, MEM_TEMP_LEN, self.i64);
            let off = LLVMBuildAdd(self.b, len, self.c64(7), c"".as_ptr());
            let off = LLVMBuildAnd(self.b, off, self.c64(!7), c"".as_ptr());
            let (end, o) = self.ovf(self.uadd, off, n);
            let cap = self.mem_ld(c, MEM_TEMP_CAP, self.i64);
            let big = self.or(
                self.icmp(IP::LLVMIntUGT, end, self.c64(0xffff_ffff)),
                self.icmp(IP::LLVMIntUGT, end, cap),
            );
            self.guard(c, LLVMBuildNot(self.b, self.or(o, big), c"".as_ptr()), slow);
            let temp = self.tag(self.mem_ld(c, MEM_TEMP, self.ptr), AA_TEMP);
            let at = self.gep(temp, len);
            let fill = LLVMBuildSub(self.b, end, len, c"".as_ptr());
            LLVMBuildMemSet(self.b, at, LLVMConstInt(self.i8, 0, 0), fill, 1);
            self.mem_st(c, MEM_TEMP_LEN, end);
            let tag = self.c64((u64::from(space::TEMP) << 61) as i64);
            (
                LLVMBuildOr(self.b, tag, off, c"".as_ptr()),
                self.gep(temp, off),
            )
        }
    }

    /// The words `LayoutOf` writes for a layout known up to the descriptor's lengths,
    /// taking `slow` wherever `Ty::layout` would fail.
    unsafe fn layout_words(
        &self,
        c: &Cur,
        sl: &StaticLayout,
        desc: Reg,
        at: Option<Known>,
        slow: LLVMBasicBlockRef,
    ) -> Vec<LLVMValueRef> {
        // SAFETY: integer arithmetic on descriptor words read through `host`.
        unsafe {
            let dims = match sl {
                StaticLayout::Words(_) => 0,
                StaticLayout::Array { dims, .. } => *dims,
            };
            // The interpreter reads the descriptor even when it has no bytes.
            let n = self.c64(i64::from(dims) * 24);
            let d = self.ld(c, desc);
            let p = self.host(c, d, at, self.c64(0), n, false, slow);
            let (el, logic) = match sl {
                StaticLayout::Words(w) => return w.iter().map(|&x| self.c64(x as i64)).collect(),
                StaticLayout::Array { elem, logic, .. } => (elem, *logic),
            };
            let not = |x| LLVMBuildNot(self.b, x, c"".as_ptr());
            let and = |a, b| LLVMBuildAnd(self.b, a, b, c"".as_ptr());
            let mut count = self.c64(1);
            let mut ok = LLVMConstInt(LLVMInt1TypeInContext(self.ctx), 1, 0);
            for k in 0..i64::from(dims) {
                let (l, r, dir) = self.dope_words(self.gep(p, self.c64(k * 24)));
                let dir_ok = self.icmp(IP::LLVMIntULE, dir, self.c64(1));
                let (len, fits) = self.len_of(l, r, dir);
                let (m, o) = self.ovf(self.umul, count, len);
                count = m;
                ok = and(ok, and(and(dir_ok, fits), not(o)));
            }
            let (count, size, align, stride) = if logic {
                let (n, o) = self.ovf(self.umul, count, self.c64(el.count as i64));
                ok = and(ok, not(o));
                // div_ceil(n, 64) * 32 cannot overflow.
                let blocks = LLVMBuildLShr(self.b, n, self.c64(6), c"".as_ptr());
                let rest = LLVMBuildAnd(self.b, n, self.c64(63), c"".as_ptr());
                let part = self.icmp(IP::LLVMIntNE, rest, self.c64(0));
                let blocks = LLVMBuildAdd(self.b, blocks, self.zext(part), c"".as_ptr());
                let size = LLVMBuildShl(self.b, blocks, self.c64(5), c"".as_ptr());
                (n, size, self.c64(8), self.c64(0))
            } else {
                let stride = el.size.next_multiple_of(el.align.max(1));
                let (size, o) = self.ovf(self.umul, self.c64(stride as i64), count);
                ok = and(ok, not(o));
                (
                    count,
                    size,
                    self.c64(el.align as i64),
                    self.c64(stride as i64),
                )
            };
            self.guard(c, ok, slow);
            vec![size, align, stride, count]
        }
    }

    /// `yavs_ir::dim_len(l, r, dir == 1)` and whether it is not `None`.
    unsafe fn len_of(
        &self,
        l: LLVMValueRef,
        r: LLVMValueRef,
        dir: LLVMValueRef,
    ) -> (LLVMValueRef, LLVMValueRef) {
        // SAFETY: integer arithmetic.
        unsafe {
            let down = self.icmp(IP::LLVMIntEQ, dir, self.c64(1));
            let lo = LLVMBuildSelect(self.b, down, r, l, c"".as_ptr());
            let hi = LLVMBuildSelect(self.b, down, l, r, c"".as_ptr());
            let empty = self.icmp(IP::LLVMIntSLT, hi, lo);
            // hi - lo is exact as a u64 when hi >= lo; + 1 wraps only for the full i64 range.
            let d = LLVMBuildSub(self.b, hi, lo, c"".as_ptr());
            let n = LLVMBuildAdd(self.b, d, self.c64(1), c"".as_ptr());
            let fits = self.or(empty, self.icmp(IP::LLVMIntNE, n, self.c64(0)));
            (
                LLVMBuildSelect(self.b, empty, self.c64(0), n, c"".as_ptr()),
                fits,
            )
        }
    }

    /// The 32-byte bit-plane block holding element `idx` of the logic array at `base`, and
    /// the element's bit position in each plane word.
    #[allow(clippy::too_many_arguments)]
    unsafe fn logic_word(
        &self,
        c: &Cur,
        base: Reg,
        at: Option<Known>,
        idx: Reg,
        write: bool,
        slow: LLVMBasicBlockRef,
    ) -> (LLVMValueRef, LLVMValueRef) {
        // SAFETY: integer arithmetic and `host`.
        unsafe {
            let i = self.ld(c, idx);
            let blk = LLVMBuildLShr(self.b, i, self.c64(6), c"".as_ptr());
            let off = LLVMBuildShl(self.b, blk, self.c64(5), c"".as_ptr());
            let p = self.host(c, self.ld(c, base), at, off, self.c64(32), write, slow);
            (p, LLVMBuildAnd(self.b, i, self.c64(63), c"".as_ptr()))
        }
    }

    /// `left, right, dir` of the descriptor dimension at `p`.
    unsafe fn dope_words(&self, p: LLVMValueRef) -> (LLVMValueRef, LLVMValueRef, LLVMValueRef) {
        // SAFETY: `p` points to 24 readable bytes.
        unsafe {
            (
                self.load(self.i64, p),
                self.load(self.i64, self.gep(p, self.c64(8))),
                self.load(self.i64, self.gep(p, self.c64(16))),
            )
        }
    }

    unsafe fn fabs(&self, x: LLVMValueRef) -> LLVMValueRef {
        // SAFETY: the intrinsic takes one double.
        unsafe {
            let id = LLVMLookupIntrinsicID(c"llvm.fabs".as_ptr(), 9);
            let mut tys = [self.f64];
            let ty = LLVMIntrinsicGetType(self.ctx, id, tys.as_mut_ptr(), 1);
            let f = LLVMGetIntrinsicDeclaration(self.module, id, tys.as_mut_ptr(), 1);
            let mut args = [x];
            LLVMBuildCall2(self.b, ty, f, args.as_mut_ptr(), 1, c"".as_ptr())
        }
    }
}

impl Drop for Gen {
    fn drop(&mut self) {
        // SAFETY: we own the builder.
        unsafe { LLVMDisposeBuilder(self.b) }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The helper declarations carry the effects `Gen::effects` means (the encoding of
    /// `memory(...)` is LLVM's own, checked here against its printer).
    #[test]
    fn helper_effects() {
        crate::lazy::ensure().unwrap();
        let m = Module::new("effects");
        let g = Gen::new(&m);
        // SAFETY: our module; the string is disposed after the copy.
        let ir = unsafe {
            let p = LLVMPrintModuleToString(m.raw());
            let s = std::ffi::CStr::from_ptr(p).to_string_lossy().into_owned();
            LLVMDisposeMessage(p);
            s
        };
        drop(g);
        let decl = |name: &str| {
            ir.lines()
                .find(|l| l.starts_with("declare") && l.contains(name))
                .unwrap_or_else(|| panic!("no {name} in {ir}"))
                .to_string()
        };
        let group = |name: &str| {
            let d = decl(name);
            let id = d.rsplit('#').next().unwrap_or("").trim().to_string();
            ir.lines()
                .find(|l| l.starts_with(&format!("attributes #{id} ")))
                .unwrap_or_else(|| panic!("no attributes for {d}"))
                .to_string()
        };
        let host = group(HOST);
        assert!(
            host.contains("memory(argmem: readwrite, inaccessiblemem: readwrite)"),
            "{host}"
        );
        assert!(
            host.contains("nounwind") && host.contains("willreturn"),
            "{host}"
        );
        let lcopy = group(LCOPY);
        assert!(lcopy.contains("memory(argmem: readwrite)"), "{lcopy}");
        assert!(group(DEOPT_SLOTS).contains("memory(argmem: read)"));
        let step = group(STEP);
        assert!(
            step.contains("nounwind") && !step.contains("memory"),
            "{step}"
        );
    }
}
