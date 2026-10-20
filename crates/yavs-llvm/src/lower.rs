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
use yavs_interp::{Stepped, TIER_SUSPEND, TIER_UNWIND, Tier, TierFrame, TierHost, space};
use yavs_ir::{
    AddrSpace, BoolOp, CheckKind, CmpOp, FOp, FUnOp, Func, FuncId, FuncKind, Inst, IntOp, IntUnOp,
    Program, Reg, Width,
};

use crate::{Error, Jit, Module, OptLevel};

const STEP: &str = "yavs_rt_step";
const HOST: &str = "yavs_rt_host";
const SIGNAL: &str = "yavs_rt_signal";
const LCOPY: &str = "yavs_rt_lcopy";
const LAYOUT: &str = "yavs_rt_layout";
const FAST: &str = "yavs_rt_fast";
const FAST_ALLOC: u32 = 0;
const FAST_DOPE_LEN: u32 = 1;
const FAST_SUBTYPE: u32 = 2;

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
    /// The temporary area's length the interpreter last gave: no hand-back while unchanged.
    temp_seen: u64,
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
const MEM_NSIGS: u64 = std::mem::offset_of!(Mem, fr.nsigs) as u64;
const MEM_TEMP_ROOTS: u64 = std::mem::offset_of!(Mem, fr.temp_roots) as u64;

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
    }
}

/// What a running compiled function reaches through `env`.
struct Env<'a> {
    host: &'a mut dyn TierHost,
    lookup: Lookup<'a>,
    /// Below this native stack address no compiled callee is entered (`NEST_STACK`).
    stack_lo: usize,
    panic: Option<Box<dyn Any + Send>>,
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
/// `b`, result register `dst`), `FAST_SUBTYPE` (`dst` dimensions of `a` against `b`). 1 when
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

/// Compiled code for the functions of one program.
pub struct LlvmTier {
    /// Keeps the code alive; never used after construction.
    _jit: Mutex<Jit>,
    funcs: Vec<Option<(RunFn, usize)>>,
    /// Functions that stay interpreted, with the reason.
    pub notes: Vec<(FuncId, String)>,
    /// Instructions of the compiled functions the interpreter executes (stepped or at an
    /// exit), per instruction kind, most frequent first; and the number compiled inline.
    pub interpreted: Vec<(String, usize)>,
    pub inline: usize,
}

impl Tier for LlvmTier {
    fn run(&self, func: FuncId, pc: u32, host: &mut dyn TierHost) -> Option<u32> {
        let &(f, nregs) = self.funcs.get(func.0 as usize)?.as_ref()?;
        let lookup = |id: FuncId| self.funcs.get(id.0 as usize).copied().flatten();
        enter(f, nregs, pc, host, &lookup)
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

/// A JIT with the runtime callbacks compiled code calls.
pub(crate) fn new_jit(opt: OptLevel) -> Result<Jit, Error> {
    Jit::new(
        opt,
        &[
            (STEP, yavs_rt_step as *const () as usize),
            (HOST, yavs_rt_host as *const () as usize),
            (SIGNAL, yavs_rt_signal as *const () as usize),
            (LCOPY, yavs_rt_lcopy as *const () as usize),
            (LAYOUT, yavs_rt_layout as *const () as usize),
            (FAST, yavs_rt_fast as *const () as usize),
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

/// Compile `f` alone into `jit`: its entry and register count, or why it stays interpreted.
pub(crate) fn compile_one(
    jit: &mut Jit,
    prog: &Program,
    id: FuncId,
    f: &Func,
    opt: OptLevel,
) -> Result<(RunFn, usize), String> {
    if let Some(r) = bad_reg(f) {
        return Err(format!("register r{} beyond the frame", r.0));
    }
    let name = format!("f{}", id.0);
    let mut m = Module::new(&name);
    // SAFETY: generation of well-formed IR in our own module; a function that fails the
    // verifier is deleted again.
    unsafe { Gen::new(&m).func(prog, f, &name) }?;
    let llvm = |e: Error| format!("LLVM: {e} (a bug in yavs)");
    m.verify().map_err(llvm)?;
    m.optimize(opt).map_err(llvm)?;
    jit.add(m).map_err(llvm)?;
    let addr = jit.lookup(&name).map_err(llvm)?;
    // SAFETY: `name` was generated by `Gen::func` with the `RunFn` signature.
    let run: RunFn = unsafe { std::mem::transmute::<usize, RunFn>(addr) };
    Ok((run, f.reg_ty.len()))
}

/// Compile every function of `prog` that the tier can take; the others (among them those over
/// `max_size` IR instructions) stay interpreted and are listed in [`LlvmTier::notes`].
pub fn compile(prog: &Program, opt: OptLevel, max_size: usize) -> Result<LlvmTier, Error> {
    let mut m = Module::new("yavs");
    let mut notes = Vec::new();
    let mut names = Vec::new();
    {
        let g = Gen::new(&m);
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
            let name = format!("f{}", id.0);
            // SAFETY: generation of well-formed IR in our own module; a function that
            // fails the verifier is deleted again.
            let ok = unsafe { g.func(prog, f, &name) };
            match ok {
                Ok(()) => names.push((id, name, f.reg_ty.len())),
                Err(why) => notes.push((id, why)),
            }
        }
    }
    m.verify()?;
    m.optimize(opt)?;
    let mut jit = new_jit(opt)?;
    jit.add(m)?;
    let mut funcs = vec![None; prog.funcs.len()];
    for (id, name, nregs) in names {
        let addr = jit.lookup(&name)?;
        // SAFETY: `name` was generated by `Gen::func` with the `RunFn` signature.
        let f: RunFn = unsafe { std::mem::transmute::<usize, RunFn>(addr) };
        funcs[id.0 as usize] = Some((f, nregs));
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
}

/// Update `known` for what `i` writes.
fn track(known: &mut [Option<Known>], i: &Inst) {
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
    /// `yavs_rt_lcopy(dst, nd, d0, src, ns, s0, n)`.
    lcopy_ty: LLVMTypeRef,
    lcopy: LLVMValueRef,
    /// `yavs_rt_layout(env, regs, mem, desc, dst, dims, stride, align, el_count, logic) -> u32`.
    layout_ty: LLVMTypeRef,
    layout: LLVMValueRef,
    /// `yavs_rt_fast(env, regs, mem, op, dst, a, b) -> u32`.
    fast_ty: LLVMTypeRef,
    fast: LLVMValueRef,
    sadd: (LLVMTypeRef, LLVMValueRef),
    ssub: (LLVMTypeRef, LLVMValueRef),
    smul: (LLVMTypeRef, LLVMValueRef),
    umul: (LLVMTypeRef, LLVMValueRef),
    uadd: (LLVMTypeRef, LLVMValueRef),
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
    /// The one block that returns: `ret` of its phi (`leave`).
    exit: LLVMBasicBlockRef,
    exit_pc: LLVMValueRef,
    /// The frame's size in bytes (`Func::frame`): the slots of every frame of the function.
    frame_len: u64,
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
            let lcopy_name = CString::new(LCOPY).expect("no NUL");
            let mut lparams = [ptr, i64, i64, ptr, i64, i64, i64];
            let lcopy_ty = LLVMFunctionType(LLVMVoidTypeInContext(ctx), lparams.as_mut_ptr(), 7, 0);
            let fast_name = CString::new(FAST).expect("no NUL");
            let mut fparams = [ptr, ptr, ptr, i32, i32, i64, i64];
            let fast_ty = LLVMFunctionType(i32, fparams.as_mut_ptr(), 7, 0);
            let layout_name = CString::new(LAYOUT).expect("no NUL");
            let mut yparams = [ptr, ptr, ptr, i64, i32, i32, i64, i64, i64, i32];
            let layout_ty = LLVMFunctionType(i32, yparams.as_mut_ptr(), 10, 0);
            Gen {
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
                lcopy_ty,
                lcopy: LLVMAddFunction(module, lcopy_name.as_ptr(), lcopy_ty),
                layout_ty,
                layout: LLVMAddFunction(module, layout_name.as_ptr(), layout_ty),
                fast_ty,
                fast: LLVMAddFunction(module, fast_name.as_ptr(), fast_ty),
                sadd: intrinsic(c"llvm.sadd.with.overflow"),
                ssub: intrinsic(c"llvm.ssub.with.overflow"),
                smul: intrinsic(c"llvm.smul.with.overflow"),
                umul: intrinsic(c"llvm.umul.with.overflow"),
                uadd: intrinsic(c"llvm.uadd.with.overflow"),
            }
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

    unsafe fn ld(&self, c: &Cur, r: Reg) -> LLVMValueRef {
        if let Some(Some(Known::Int(v))) = c.known.borrow().get(r.0 as usize) {
            return self.c64(*v);
        }
        // SAFETY: see `slot`.
        unsafe { LLVMBuildLoad2(self.b, self.i64, self.slot(c, r), c"".as_ptr()) }
    }

    /// Return `v` (an i32): a branch to the function's exit block.
    unsafe fn leave(&self, c: &Cur, v: LLVMValueRef) {
        // SAFETY: the builder is in a block of c.f; the phi is the exit block's.
        unsafe {
            let mut vals = [v];
            let mut from = [LLVMGetInsertBlock(self.b)];
            LLVMAddIncoming(c.exit_pc, vals.as_mut_ptr(), from.as_mut_ptr(), 1);
            LLVMBuildBr(self.b, c.exit);
        }
    }

    unsafe fn ldf(&self, c: &Cur, r: Reg) -> LLVMValueRef {
        // SAFETY: see `slot`.
        unsafe { LLVMBuildBitCast(self.b, self.ld(c, r), self.f64, c"".as_ptr()) }
    }

    unsafe fn st(&self, c: &Cur, r: Reg, v: LLVMValueRef) {
        // SAFETY: see `slot`; `v` is an i64.
        unsafe {
            LLVMBuildStore(self.b, v, self.slot(c, r));
        }
    }

    unsafe fn stf(&self, c: &Cur, r: Reg, v: LLVMValueRef) {
        // SAFETY: see `slot`; `v` is a double.
        unsafe { self.st(c, r, LLVMBuildBitCast(self.b, v, self.i64, c"".as_ptr())) }
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
            self.leave(c, self.c32(pc));
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
    unsafe fn func(&self, prog: &Program, f: &Func, name: &str) -> Result<(), String> {
        let cname = CString::new(name).expect("no NUL");
        // SAFETY: all values are built in this function with matching types.
        unsafe {
            let fv = LLVMAddFunction(self.module, cname.as_ptr(), self.fn_ty);
            // `regs` and `mem` overlap no other memory the function reaches (frame slots,
            // temporaries, objects), so LLVM may keep their values in registers between
            // calls; the callbacks get both pointers.
            let kind = LLVMGetEnumAttributeKindForName(c"noalias".as_ptr(), 7);
            for i in [2, 3] {
                LLVMAddAttributeAtIndex(fv, i, LLVMCreateEnumAttribute(self.ctx, kind, 0));
            }
            let exit = LLVMAppendBasicBlockInContext(self.ctx, fv, c"".as_ptr());
            LLVMPositionBuilderAtEnd(self.b, exit);
            let exit_pc = LLVMBuildPhi(self.b, self.i32, c"".as_ptr());
            LLVMBuildRet(self.b, exit_pc);
            let c = Cur {
                prog,
                consts: Default::default(),
                f: fv,
                env: LLVMGetParam(fv, 0),
                regs: LLVMGetParam(fv, 1),
                mem: LLVMGetParam(fv, 2),
                known: std::cell::RefCell::new(vec![None; f.reg_ty.len()]),
                exit,
                exit_pc,
                frame_len: u64::from(f.frame),
            };
            let pc_in = LLVMGetParam(fv, 3);
            let n = f.code.len();
            // Entry points: 0, block starts, and the instruction after each that may leave
            // frames to the interpreter or suspend.
            let mut leader = vec![false; n + 1];
            leader[0] = true;
            for (_, b) in f.blocks.iter() {
                if let Some(l) = leader.get_mut(b.start as usize) {
                    *l = true;
                }
            }
            for (i, inst) in f.code.iter().enumerate() {
                if resumes(inst) {
                    leader[i + 1] = true;
                }
            }
            let entry = self.block(&c);
            LLVMMoveBasicBlockBefore(entry, c.exit);
            let other = self.block(&c);
            let mut bbs = vec![std::ptr::null_mut(); n];
            for (i, bb) in bbs.iter_mut().enumerate() {
                if leader[i] {
                    *bb = self.block(&c);
                }
            }
            LLVMPositionBuilderAtEnd(self.b, other);
            LLVMBuildRet(self.b, pc_in);
            LLVMPositionBuilderAtEnd(self.b, entry);
            let nl = leader[..n].iter().filter(|&&l| l).count();
            let sw = LLVMBuildSwitch(self.b, pc_in, other, nl as u32);
            for (i, &bb) in bbs.iter().enumerate() {
                if !bb.is_null() {
                    LLVMAddCase(sw, self.c32(i as u32), bb);
                }
            }
            let block_bb = |b: yavs_ir::BlockId| -> Result<LLVMBasicBlockRef, String> {
                f.blocks
                    .get(b)
                    .and_then(|b| bbs.get(b.start as usize).copied())
                    .filter(|bb| !bb.is_null())
                    .ok_or_else(|| format!("branch to missing block {}", b.0))
            };
            let mut open = false;
            let mut prev = None;
            for (i, &inst) in f.code.iter().enumerate() {
                let pc = i as u32;
                if let Some(p) = prev.replace(inst) {
                    track(&mut c.known.borrow_mut(), &p);
                }
                if leader[i] {
                    c.known.borrow_mut().fill(None);
                }
                // What is known about an address register (`Gen::host`).
                let fk = |r: Reg| match c.known.borrow().get(r.0 as usize) {
                    Some(&Some(k @ (Known::Frame(_) | Known::Const(..)))) => Some(k),
                    _ => None,
                };
                if leader[i] {
                    if open {
                        LLVMBuildBr(self.b, bbs[i]);
                    }
                    LLVMPositionBuilderAtEnd(self.b, bbs[i]);
                    open = true;
                } else if !open {
                    // Dead code after a terminator: give it a block, LLVM drops it.
                    let bb = self.block(&c);
                    LLVMPositionBuilderAtEnd(self.b, bb);
                    open = true;
                }
                // The interpreter returns (`TierHost::ret` for a frame compiled code pushed):
                // compiled code leaves the frame to it there.
                if let Inst::Ret { .. } = inst {
                    self.leave(&c, self.c32(pc));
                    open = false;
                    continue;
                }
                match inst {
                    Inst::Const { dst, v } => self.st(&c, dst, self.c64(v)),
                    Inst::ConstF { dst, v } => self.st(&c, dst, self.c64(v.to_bits() as i64)),
                    Inst::Move { dst, src } => self.st(&c, dst, self.ld(&c, src)),
                    Inst::Bin { op, dst, a, b } => {
                        let (x, y) = (self.ld(&c, a), self.ld(&c, b));
                        match self.int_op(&c, op, x, y, pc) {
                            Some(v) => self.st(&c, dst, v),
                            None => self.step(&c, pc, &inst),
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
                                self.st(&c, dst, v);
                            }
                            None => self.step(&c, pc, &inst),
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
                        self.st(&c, dst, v);
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
                                self.step(&c, pc, &inst);
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
                        self.stf(&c, dst, v);
                    }
                    Inst::FUn { op, dst, a } => {
                        let x = self.ldf(&c, a);
                        let v = match op {
                            FUnOp::Neg => LLVMBuildFNeg(self.b, x, c"".as_ptr()),
                            FUnOp::Abs => self.fabs(x),
                        };
                        self.stf(&c, dst, v);
                    }
                    Inst::IToF { dst, src } => {
                        let v = LLVMBuildSIToFP(self.b, self.ld(&c, src), self.f64, c"".as_ptr());
                        self.stf(&c, dst, v);
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
                        self.st(&c, dst, self.zext(v));
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
                        self.st(&c, dst, self.zext(v));
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
                        self.st(&c, dst, self.zext(v));
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
                            let hi = self.mem_ld(&c, MEM_FRAME, self.i64);
                            let hi = LLVMBuildShl(self.b, hi, self.c64(32), c"".as_ptr());
                            self.st(
                                &c,
                                dst,
                                LLVMBuildOr(self.b, hi, self.c64(a as i64), c"".as_ptr()),
                            );
                        }
                        Some(a) => self.st(&c, dst, self.c64(a as i64)),
                        // A frame one or two static links up, from the view.
                        None => match space {
                            AddrSpace::Up(d @ 1..=2) => {
                                let (slow, join) = self.slow_path(&c, pc, &inst);
                                let hi = self.mem_ld(&c, MEM_UP + 8 * u64::from(d - 1), self.i64);
                                let ok = self.icmp(IP::LLVMIntNE, hi, self.c64(-1));
                                self.guard(&c, ok, slow);
                                let hi = LLVMBuildShl(self.b, hi, self.c64(32), c"".as_ptr());
                                let a = LLVMBuildOr(self.b, hi, self.c64(off.into()), c"".as_ptr());
                                self.st(&c, dst, a);
                                self.join(join);
                            }
                            _ => self.step(&c, pc, &inst),
                        },
                    },
                    Inst::Stmt => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let roots = self.mem_ld(&c, MEM_TEMP_ROOTS, self.i64);
                        self.guard(&c, self.icmp(IP::LLVMIntEQ, roots, self.c64(0)), slow);
                        let len = self.mem_ld(&c, MEM_TEMP_LEN, self.i64);
                        let base = self.mem_ld(&c, MEM_TEMP_BASE, self.i64);
                        let lt = self.icmp(IP::LLVMIntULT, base, len);
                        let v = LLVMBuildSelect(self.b, lt, base, len, c"".as_ptr());
                        self.mem_st(&c, MEM_TEMP_LEN, v);
                        self.join(join);
                    }
                    Inst::Alloc { dst, size } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        self.fast_op(&c, FAST_ALLOC, dst.0, self.ld(&c, size), self.c64(0), slow);
                        self.join(join);
                    }
                    Inst::LayoutOf { dst, ty, desc } => match static_layout(prog, ty) {
                        Some(StaticLayout::Array { dims, elem, logic }) => {
                            // `yavs_rt_layout` does what `Ty::layout` would; 0 is the interpreter's.
                            let (slow, join) = self.slow_path(&c, pc, &inst);
                            let stride = elem.size.next_multiple_of(elem.align.max(1));
                            let mut args = [
                                c.env,
                                c.regs,
                                c.mem,
                                self.ld(&c, desc),
                                self.c32(dst.0),
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
                            self.join(join);
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
                            self.join(join);
                        }
                        None => self.step(&c, pc, &inst),
                    },
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
                        self.join(join);
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
                        self.join(join);
                    }
                    Inst::Copy { dst, src, bytes } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let n = self.ld(&c, bytes);
                        let ps =
                            self.host(&c, self.ld(&c, src), fk(src), self.c64(0), n, false, slow);
                        let pd =
                            self.host(&c, self.ld(&c, dst), fk(dst), self.c64(0), n, true, slow);
                        LLVMBuildMemMove(self.b, pd, 1, ps, 1, n);
                        self.join(join);
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
                        self.join(join);
                    }
                    Inst::DopeLen { dst, dope, dim } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let (d, k) = (self.ld(&c, dope), self.c64(i64::from(dim)));
                        self.fast_op(&c, FAST_DOPE_LEN, dst.0, d, k, slow);
                        self.join(join);
                    }
                    Inst::SubtypeCheck {
                        dope, want, dims, ..
                    } => {
                        let (slow, join) = self.slow_path(&c, pc, &inst);
                        let (d, w) = (self.ld(&c, dope), self.ld(&c, want));
                        self.fast_op(&c, FAST_SUBTYPE, u32::from(dims), d, w, slow);
                        self.join(join);
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
                        self.join(join);
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
                        self.join(join);
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
                        self.join(join);
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
                        self.join(join);
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
                        self.join(join);
                    }
                    Inst::Br { to } => {
                        LLVMBuildBr(self.b, block_bb(to)?);
                        open = false;
                    }
                    Inst::CondBr { c: r, t, f: e } => {
                        let x = self.icmp(IP::LLVMIntNE, self.ld(&c, r), self.c64(0));
                        LLVMBuildCondBr(self.b, x, block_bb(t)?, block_bb(e)?);
                        open = false;
                    }
                    Inst::Case { v, table } => {
                        let t = prog
                            .cases
                            .get(table)
                            .ok_or_else(|| format!("no case table {}", table.0))?;
                        let x = self.ld(&c, v);
                        // ponytail: a compare chain per arm; a switch for dense tables if
                        // case-heavy code ever shows in profiles.
                        for &(lo, hi, to) in &t.arms {
                            let ge = self.icmp(IP::LLVMIntSGE, x, self.c64(lo));
                            let le = self.icmp(IP::LLVMIntSLE, x, self.c64(hi));
                            let hit = LLVMBuildAnd(self.b, ge, le, c"".as_ptr());
                            let next = self.block(&c);
                            LLVMBuildCondBr(self.b, hit, block_bb(to)?, next);
                            LLVMPositionBuilderAtEnd(self.b, next);
                        }
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
                        let mut args = [pd, nd, d0, ps, ns, s0, k];
                        LLVMBuildCall2(
                            self.b,
                            self.lcopy_ty,
                            self.lcopy,
                            args.as_mut_ptr(),
                            7,
                            c"".as_ptr(),
                        );
                        self.join(join);
                    }
                    Inst::SigReadScalar { .. }
                    | Inst::SigRead { .. }
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
                                self.join(join);
                            }
                            Some(NativeKind::Edge(rising)) => {
                                self.signal(&c, pc, if rising { 1 } else { 2 })
                            }
                            None => self.step(&c, pc, &inst),
                        }
                    }
                    _ => self.step(&c, pc, &inst),
                }
            }
            if open {
                // Falling off the end: the interpreter reports it.
                self.leave(&c, self.c32(n as u32));
            }
            if LLVMVerifyFunction(fv, LLVMVerifierFailureAction::LLVMReturnStatusAction) != 0 {
                LLVMDeleteFunction(fv);
                return Err("the generated LLVM function does not verify (a bug in yavs)".into());
            }
            Ok(())
        }
    }

    /// Execute the instruction `i` at `pc` in the interpreter; return what `yavs_rt_step`
    /// returns unless it is `GO`.
    unsafe fn step(&self, c: &Cur, pc: u32, i: &Inst) {
        let at = if keeps_objects(i) { pc | KEEP } else { pc };
        // SAFETY: `step` has the run-function signature.
        unsafe {
            self.call_go(
                c,
                self.fn_ty,
                self.step,
                &mut [c.env, c.regs, c.mem, self.c32(at)],
            )
        }
    }

    /// The signal instruction at `pc` through `yavs_rt_signal` (`edge`: `TierHost::signal`).
    unsafe fn signal(&self, c: &Cur, pc: u32, edge: u32) {
        let mut args = [c.env, c.regs, c.mem, self.c32(pc), self.c32(edge)];
        // SAFETY: the arguments match `signal_ty`.
        unsafe { self.call_go(c, self.signal_ty, self.signal, &mut args) }
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
            let mut args = [c.env, c.regs, c.mem, self.c32(op), self.c32(dst), a, b];
            let ok = LLVMBuildCall2(
                self.b,
                self.fast_ty,
                self.fast,
                args.as_mut_ptr(),
                7,
                c"".as_ptr(),
            );
            self.guard(c, self.icmp(IP::LLVMIntNE, ok, self.c32(0)), slow);
        }
    }

    /// Call a callback that returns `GO` or what the compiled function returns at once.
    unsafe fn call_go(&self, c: &Cur, ty: LLVMTypeRef, f: LLVMValueRef, args: &mut [LLVMValueRef]) {
        // SAFETY: `args` match `ty`; well-formed blocks of c.f.
        unsafe {
            let r = LLVMBuildCall2(
                self.b,
                ty,
                f,
                args.as_mut_ptr(),
                args.len() as u32,
                c"".as_ptr(),
            );
            let cont = self.block(c);
            let mut vals = [r];
            let mut from = [LLVMGetInsertBlock(self.b)];
            LLVMAddIncoming(c.exit_pc, vals.as_mut_ptr(), from.as_mut_ptr(), 1);
            LLVMBuildCondBr(
                self.b,
                self.icmp(IP::LLVMIntEQ, r, self.c32(GO)),
                cont,
                c.exit,
            );
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
            self.step(c, pc, i);
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
    unsafe fn join(&self, join: LLVMBasicBlockRef) {
        // SAFETY: the builder is in a block of the function of `join`.
        unsafe {
            LLVMBuildBr(self.b, join);
            LLVMPositionBuilderAtEnd(self.b, join);
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
            LLVMBuildGEP2(self.b, self.i8, p, idx.as_mut_ptr(), 1, c"".as_ptr())
        }
    }

    /// An unaligned load.
    unsafe fn load(&self, ty: LLVMTypeRef, p: LLVMValueRef) -> LLVMValueRef {
        // SAFETY: `p` points to a value of `ty`.
        unsafe {
            let v = LLVMBuildLoad2(self.b, ty, p, c"".as_ptr());
            LLVMSetAlignment(v, 1);
            v
        }
    }

    /// An unaligned store.
    unsafe fn store(&self, v: LLVMValueRef, p: LLVMValueRef) {
        // SAFETY: `p` points to memory for a value of `v`'s type.
        unsafe {
            let s = LLVMBuildStore(self.b, v, p);
            LLVMSetAlignment(s, 1);
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
                        return self.gep(g, o);
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
                        return self.gep(self.mem_ld(c, MEM_SLOTS, self.ptr), at);
                    }
                }
                // `extra` is below 2^63, the sum cannot wrap.
                let o = LLVMBuildAdd(self.b, self.c64(o as i64), extra, c"".as_ptr());
                let len = self.mem_ld(c, MEM_SLOTS_LEN, self.i64);
                let fits = self.icmp(IP::LLVMIntULE, n, len);
                let rest = LLVMBuildSub(self.b, len, n, c"".as_ptr());
                let inside = self.icmp(IP::LLVMIntULE, o, rest);
                self.guard(c, LLVMBuildAnd(self.b, fits, inside, c"".as_ptr()), slow);
                return self.gep(self.mem_ld(c, MEM_SLOTS, self.ptr), o);
            }
            // Other memory through `yavs_rt_host` (`Mem::find`, then the objects the interpreter
            // is asked for): a call costs about what the interpreter's own address lookup
            // does, and generating the lookup inline made the code two thirds larger and
            // twice as slow to compile for no faster run.
            let mut args = [c.env, c.mem, a, extra, n, self.c32(u32::from(write))];
            let p = LLVMBuildCall2(
                self.b,
                self.host_ty,
                self.host,
                args.as_mut_ptr(),
                6,
                c"".as_ptr(),
            );
            self.guard(c, LLVMBuildIsNotNull(self.b, p, c"".as_ptr()), slow);
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
            let temp = self.mem_ld(c, MEM_TEMP, self.ptr);
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
