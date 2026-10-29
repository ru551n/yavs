//! The dispatch loop (ir.md 3.1, 3.2): frames on a heap stack, memory spaces, and the
//! instruction semantics.
//!
//! Register and operand indices are checked by the verifier, so they are indexed directly;
//! a violation panics, which the kernel turns into an internal error. Everything that
//! depends on data (addresses, sizes, indices, handles) is checked here and becomes a
//! runtime error or an ICE, never a panic.

use std::sync::Arc;
use std::sync::atomic::Ordering;

use yavs_ir::{
    AddrSpace, AttrKind, BlockId, BoolOp, CheckKind, CmpOp, ConstData, Dir, DriveMode, EdgeKind,
    FOp, FUnOp, ForceKind, Func, FuncId, FuncKind, Inst, IntOp, IntUnOp, LNumOp, LOp, LayoutError,
    ParamClass, PhysOp, Program, Reg, StmtId, Ty, TyId, Width,
};
use yavs_kernel::{
    Ctx, DelayMode, Elem, ErrorKind, ForceMode, LogicVec, Severity, SigRef, SimError, Time, Value,
    ValueRef,
};
use yavs_logic::Planes;
use yavs_std::arith;

use crate::debug::{DebugSession, FrameView, ProcState, StepMode, Stop, StopReason};
use crate::heap::{self, Heap, HeapError};
use crate::pre::{self, Op};
use crate::scan::{Scan, ScanFrom, ScanShape};
use crate::{InstData, NAT_ASK, NAT_CODE, ProcDriver, ProcRoots, Runtime, SharedState};

/// An execution failure before it becomes a `SimError` with a trace.
#[derive(Debug)]
pub(crate) enum Fault {
    Runtime(String),
    /// A call of an unbound foreign subprogram: message and places searched.
    Unbound(Box<(String, String)>),
    Internal(String),
    Kernel(SimError),
    /// A call that only computed is abandoned to run again with a function optimised
    /// (`Machine::back_budget`).
    Restart,
}

pub(crate) type R<T> = Result<T, Fault>;

/// Jumps back of a process that never suspends before its activation is reported as
/// infinite (#583, user decision 2026-10-26): high enough that a long legitimate loop without
/// a wait is not mistaken for a hang (seconds of compiled code), low enough that a stuck run is
/// diagnosed. `YAVS_SPIN_LIMIT` replaces it (tests that must reach it quickly).
pub const SPIN_LIMIT: u64 = 1_000_000_000;

/// Every this many jumps back (a power of two) such a process is trimmed (`Ctx::trim_run`: its
/// run lists would otherwise grow by an entry per assignment, #761), and compiled code leaves
/// for the interpreter, which also reports the activation at the limit.
pub const SPIN_CHECK: u64 = 1 << 20;

/// `Machine::spin` counts jumps back from the first to the second of `spin_range()`: the limit
/// in force (`SPIN_LIMIT` or `YAVS_SPIN_LIMIT`) rounded up to a multiple of `SPIN_CHECK`, so
/// compiled code needs only the test "a multiple of `SPIN_CHECK`" to leave at every trim and at
/// the limit.
fn spin_range() -> (u64, u64) {
    static R: std::sync::OnceLock<(u64, u64)> = std::sync::OnceLock::new();
    *R.get_or_init(|| {
        let limit = match std::env::var("YAVS_SPIN_LIMIT") {
            Ok(v) => v
                .parse()
                .ok()
                .filter(|&n: &u64| n > 0)
                .unwrap_or_else(|| panic!("YAVS_SPIN_LIMIT={v}: expected a positive integer")),
            Err(_) => SPIN_LIMIT,
        };
        let end = limit.next_multiple_of(SPIN_CHECK);
        (end - limit, end)
    })
}

/// Runs `f` out of line. Every error of the interpreter is built by `cold`, through the
/// macros below: an error constructor that is inlined into the loop of an op puts its message
/// and its arguments on the stack of the loop and changes how the compiler lays the rest out,
/// by up to a few percent of the instructions of a clocked process (#508).
#[cold]
#[inline(never)]
pub(crate) fn cold<T>(f: impl FnOnce() -> T) -> T {
    f()
}

/// `ice(format!(..))`, out of line.
macro_rules! ice_f {
    ($($a:tt)*) => {
        cold(move || ice(format!($($a)*)))
    };
}

/// `rt_err(format!(..))`, out of line.
macro_rules! rt_err_f {
    ($($a:tt)*) => {
        cold(move || rt_err(format!($($a)*)))
    };
}

/// `Fault::Runtime(format!(..))`, out of line.
macro_rules! runtime_f {
    ($($a:tt)*) => {
        cold(move || Fault::Runtime(format!($($a)*)))
    };
}

/// `ice(message)`, out of line.
macro_rules! ice_s {
    ($s:expr) => {
        cold(move || ice($s))
    };
}

/// `rt_err(message)`, out of line.
macro_rules! rt_err_s {
    ($s:expr) => {
        cold(move || rt_err($s))
    };
}

/// `Fault::Runtime(message)`, out of line.
macro_rules! runtime_s {
    ($s:expr) => {
        cold(move || Fault::Runtime(String::from($s)))
    };
}

pub(crate) use ice_f;

pub(crate) fn rt_err<T>(m: impl Into<String>) -> R<T> {
    Err(Fault::Runtime(m.into()))
}

pub(crate) fn ice(m: impl Into<String>) -> Fault {
    Fault::Internal(m.into())
}

impl From<SimError> for Fault {
    fn from(e: SimError) -> Fault {
        Fault::Kernel(e)
    }
}

/// The outcome of `Machine::run_quick`, small enough to travel in a register: the result
/// struct of `exec_fast` copied through `exec_in` and `exec` cost a store-forwarding stall
/// per activation. A fault waits in `Machine::quick_fault`.
pub(crate) enum Quick {
    Suspended,
    Returned,
    Fault,
}

/// What [`Machine::run_direct`] did with an activation.
pub(crate) enum Direct {
    /// Not an activation of compiled code: nothing was done, `run_quick` runs it.
    No,
    /// Compiled code ran and left the process for the interpreter (`run_quick` with `cont`).
    Continue,
    /// Compiled code ended the activation.
    Done(Quick),
}

pub(crate) enum Outcome {
    Suspended,
    Returned(Option<u64>),
}

// Address spaces as stored in the top 3 bits of an address.
/// `base` of a signal reference of several parts: the low half is the part count, the second
/// register the address of `[lo, len, slot]` entries (`Inst::SigParts`).
const MULTI: u64 = 0xffff_ffff;
const PART_BYTES: u64 = 24;
const NO_SLOT: u64 = u64::MAX;

/// The element kind shared by the parts of a reference.
fn same_elem(parts: &[(SigRef, yavs_kernel::SignalType, Option<u32>)]) -> R<Elem> {
    let mut it = parts.iter().map(|p| p.1.elem);
    let e = it.next().unwrap_or(Elem::Bytes(1));
    if it.any(|x| x != e) {
        return Err(ice_s!("signal reference parts of different element types"));
    }
    Ok(e)
}

/// Values of consecutive element ranges as one value.
fn concat(elem: Elem, vals: &[Value]) -> R<Value> {
    Ok(match elem {
        Elem::Bytes(_) => Value::Bytes(
            vals.iter()
                .flat_map(|v| v.as_bytes().unwrap_or_default().iter().copied())
                .collect(),
        ),
        Elem::Logic => {
            let ls: Vec<&LogicVec> = vals.iter().filter_map(Value::as_logic).collect();
            if ls.len() != vals.len() {
                return Err(ice_s!("logic signal part without a logic value"));
            }
            let codes: Vec<_> = ls.iter().flat_map(|l| l.iter()).collect();
            Value::Logic(LogicVec::from_fn(codes.len(), |i| codes[i]))
        }
    })
}

/// Elements `at .. at + len` of a value of `elem` elements.
fn sub_value(v: &Value, elem: Elem, at: u32, len: u32) -> R<Value> {
    let bad = || ice_s!("signal value shorter than its reference");
    Ok(match (v, elem) {
        (Value::Bytes(b), Elem::Bytes(w)) => {
            let (a, n) = (at as usize * w as usize, len as usize * w as usize);
            Value::Bytes(b.get(a..a + n).ok_or_else(bad)?.to_vec())
        }
        (Value::Logic(l), Elem::Logic) if (at + len) as usize <= l.len() => {
            Value::Logic(l.slice(at as usize, len as usize))
        }
        _ => return Err(bad()),
    })
}

pub const FRAME: u8 = 0;
pub const INST: u8 = 1;
pub const PKG: u8 = 2;
pub const CONST: u8 = 3;
pub const HEAP: u8 = 4;
pub const TEMP: u8 = 5;
pub const SHARED: u8 = 6;
pub const SHARED_HEAP: u8 = 7;

const SPACE_NAMES: [&str; 8] = [
    "frame",
    "instance",
    "package",
    "constant",
    "heap",
    "temporary",
    "shared",
    "shared heap",
];

/// `[space: 3 | index: 29 | offset: 32]`.
#[inline(always)]
pub(crate) fn mk_addr(space: u8, index: u32, off: u64) -> R<u64> {
    if off > u64::from(u32::MAX) {
        return rt_err_s!("object larger than 4 GiB");
    }
    if index >= 1 << 29 {
        return Err(ice_f!("address index {index} too large"));
    }
    Ok(u64::from(space) << 61 | u64::from(index) << 32 | off)
}

#[inline(always)]
pub(crate) fn split(a: u64) -> (u8, u32, u64) {
    (
        (a >> 61) as u8,
        ((a >> 32) & ((1 << 29) - 1)) as u32,
        a & 0xffff_ffff,
    )
}

#[inline(always)]
pub(crate) fn add_addr(a: u64, delta: u64) -> R<u64> {
    let (s, i, o) = split(a);
    match o.checked_add(delta) {
        Some(o) => mk_addr(s, i, o),
        None => rt_err_s!("object larger than 4 GiB"),
    }
}

/// `Machine::tier_go`: frames compiled code left (`Flow::Call`), a suspension.
const TIER_GO_CALL: u64 = u64::MAX;
const TIER_GO_SUSPEND: u64 = u64::MAX - 1;

/// Jumps of an interpreted function without compiled code until the fast loop asks the tier
/// again (`run_ops`).
const RECHECK: u32 = 256;
/// Events (jumps, calls, returns) between two looks of the loop that has no compiled code yet.
const WATCH: u32 = 128;
/// Iterations of a fused loop op (`Op::Loop`, the search loop) per dispatch in the watching loop,
/// so a loop that stays in one op is still looked at (the plain loop runs them in one go).
const WATCH_CHUNK: u32 = 128;

/// Every `WATCH` jumps back and calls of the watching loop: name the running function for the
/// sampler; true when it has compiled code or is to be compiled. Out of line on purpose: inlined,
/// its thread-local access cost `run_ops::<false, true>` 4 percent of cycles on neorv32 (#521).
#[cold]
#[inline(never)]
fn watch_tick(rt: &Runtime, func: FuncId, pc: u32) -> bool {
    rt.sync_native();
    crate::slot().store(
        u64::from(rt.id) << 32 | u64::from(func.0),
        Ordering::Relaxed,
    );
    crate::pc_slot().store(u64::from(func.0) << 32 | u64::from(pc), Ordering::Relaxed);
    rt.native
        .func
        .get(func.0 as usize)
        .is_some_and(|n| n.load(Ordering::Acquire) & (NAT_CODE | NAT_ASK) != 0)
}

/// Whether a call of `callee` hands the loop over to the one with the tier: a flag of it is set now
/// (`Pre::native` says it had one once) and the tier does not call it on a native frame, which the
/// loop without the tier does itself (the loop with it costs more per op for the rest of the
/// activation, #757). Out of line: it runs only for functions the tier has touched, and inlined
/// its loads cost the loop registers.
#[cold]
#[inline(never)]
fn callee_retier(rt: &Runtime, callee: u32) -> bool {
    rt.native.flagged(callee) && !rt.native.callable(callee)
}

/// Zeroes the sampling slot when dropped (`Machine::exec_fast`).
struct SlotIdle<'a>(Option<&'a std::sync::atomic::AtomicU64>);

impl Drop for SlotIdle<'_> {
    #[inline(always)]
    fn drop(&mut self) {
        if let Some(s) = self.0 {
            s.store(0, Ordering::Relaxed);
        }
    }
}

/// The body of `Machine::push_frame` and `push_frame_pre`.
macro_rules! push_frame_body {
    ($self:ident, $pre:ident, $f:ident, $func:ident, $link:ident, $ret:ident, $slots:expr) => {{
        let idx = $self.frames.len();
        if idx >= $self.rt.limits.max_depth as usize {
            return $self.depth_fail($f);
        }
        let slen = $slots as usize;
        let (rbase, sbase) = if idx == 0 {
            $self.regs.v.clear();
            $self.root.clear();
            $self.root.resize(slen, 0);
            (0, 0)
        } else {
            let sbase = $self.sstack.len();
            $self.sstack.resize(sbase + slen, 0);
            ($self.regs.v.len(), sbase)
        };
        // The stacks are indexed by `u32` (a frame is 28 bytes smaller for it).
        let (Ok(rb), Ok(sb), Ok(tb)) = (
            u32::try_from(rbase),
            u32::try_from(sbase),
            u32::try_from($self.temp.len()),
        ) else {
            return stack_fail($f);
        };
        $self.regs.base = rbase;
        if idx < 1 << 29 {
            $pre.fill_below(&mut $self.regs.v, idx as u32);
        } else {
            $pre.fill(&mut $self.regs.v, idx as u32)?;
        }
        // A root frame entered with the shared state attached is elaboration code
        // (`call_shared`, `init_package`, `call_values_shared`); a process enters its
        // root frame before it first takes the shared state, and allocates in the shared
        // heap when it is serial: what it allocates may end up in a shared variable (#303).
        let shared_alloc = match $self.frames.last() {
            Some(top) => top.shared_alloc,
            None => {
                $self.sh.is_some()
                    || $f.kind == FuncKind::Process && $self.rt.prog.uses_shared_state($func)
            }
        };
        $self.frames.push(Frame {
            func: $func,
            pc: 0,
            rbase: rb,
            sbase: sb,
            slen: $slots,
            link: $link,
            ret: $ret,
            temp_base: tb,
            shared_alloc,
        });
    }};
}

#[cold]
#[inline(never)]
fn stack_fail(f: &Func) -> R<()> {
    rt_err_f!("call stack exceeds 4 GiB calling `{}`", f.name)
}

/// The texts of the literals of one enumeration type: (type, `'image` or `to_string`, text of each).
pub(crate) type EnumText = (TyId, bool, Vec<Option<Box<[u8]>>>);

pub(crate) struct Frame {
    pub func: FuncId,
    pub pc: u32,
    /// First register of the frame in `Machine::regs`.
    pub rbase: u32,
    /// The frame's slots: all of `Machine::root` for frame 0, else `slen` bytes of
    /// `Machine::sstack` from `sbase`.
    pub sbase: u32,
    pub slen: u32,
    /// Static link: index of the lexically enclosing frame.
    pub link: Option<u32>,
    /// Caller register receiving the result.
    pub ret: Option<Reg>,
    /// Temporary-area top when the frame was entered.
    pub temp_base: u32,
    /// Allocators allocate in the shared heap (`Machine::alloc_heap`): inside a method of a
    /// protected object in the shared heap (#184), in elaboration code run with the shared
    /// state, whose private heap dies with the call (#246), and in everything they call.
    pub shared_alloc: bool,
}

/// The slots of frame `i`, from the machine's fields (so the caller can borrow others).
#[inline(always)]
fn slots_of<'a>(frames: &[Frame], root: &'a [u8], sstack: &'a [u8], i: usize) -> Option<&'a [u8]> {
    let fr = frames.get(i)?;
    if i == 0 {
        Some(root)
    } else {
        sstack.get(fr.sbase as usize..(fr.sbase + fr.slen) as usize)
    }
}

#[inline(always)]
fn slots_of_mut<'a>(
    frames: &[Frame],
    root: &'a mut [u8],
    sstack: &'a mut [u8],
    i: usize,
) -> Option<&'a mut [u8]> {
    let fr = frames.get(i)?;
    if i == 0 {
        Some(root)
    } else {
        sstack.get_mut(fr.sbase as usize..(fr.sbase + fr.slen) as usize)
    }
}

/// What an element loop does (`Machine::loop_plan`).
enum LoopPlan {
    /// The elements of the first stream copied to the second.
    Copy(LoopStream, LoopStream),
    /// The elements set to a code.
    Fill(LoopStream, u8),
}

/// The elements `e0 + step * n` of the logic vector at `base`.
#[derive(Clone, Copy)]
struct LoopStream {
    base: u64,
    e0: i64,
    step: i64,
}

/// The position of the affine index `x` (value at iteration 0, change per iteration) in the
/// dimension with bounds and direction `d`, affine too, when the index stays in range up to
/// iteration `last`.
fn loop_pos(d: (i64, i64, u64), x: (i64, i64), last: i64) -> Option<(i64, i64)> {
    let (l, r, dir) = d;
    let e1 = x.0.checked_add(x.1.checked_mul(last)?)?;
    let (lo, hi, c0, c1) = match dir {
        0 => (l, r, x.0.checked_sub(l)?, x.1),
        1 => (r, l, l.checked_sub(x.0)?, x.1.checked_neg()?),
        _ => return None,
    };
    (lo <= x.0 && x.0 <= hi && lo <= e1 && e1 <= hi).then_some((c0, c1))
}

/// The std_ulogic code of element `e` of the logic vector at byte `off` of `m` (`lget`).
#[inline(always)]
fn lget_in(m: &[u8], off: usize, e: u64) -> Option<u8> {
    let b = m.get(off + (e / 64) as usize * 32..)?.first_chunk::<32>()?;
    let s = e % 64;
    Some((0..4).fold(0u8, |c, p| c | ((plane_word(b, p) >> s & 1) as u8) << p))
}

/// Set element `e` of the logic vector at byte `off` of `m` to `code` (`lset`); false when the
/// planes are not there.
#[inline(always)]
fn lset_in(m: &mut [u8], off: usize, e: u64, code: u8) -> bool {
    let Some(b) = m
        .get_mut(off + (e / 64) as usize * 32..)
        .and_then(|b| b.first_chunk_mut::<32>())
    else {
        return false;
    };
    let s = e % 64;
    for p in 0..4 {
        let w = plane_word(b, p) & !(1 << s) | u64::from(code >> p & 1) << s;
        b[p * 8..p * 8 + 8].copy_from_slice(&w.to_le_bytes());
    }
    true
}

/// What a search loop (`pre::SCAN`) is made of.
struct ScanLoop {
    dope: u64,
    dim: u8,
    base: u64,
    stride: u32,
    w: Width,
    m: u8,
    m2: u8,
    /// The step's compare has the index as its left operand.
    idx_left: bool,
    delta: i64,
}

pub(crate) enum Flow {
    Next,
    Jump(yavs_ir::BlockId),
    /// The callee's frame was pushed.
    Call,
    Ret(Option<u64>),
    Suspend,
    /// The fast loop without a compiled tier's bookkeeping hands over to the one with it; the
    /// top frame's pc is where to go on.
    Retier,
}

/// The message of the `Pos` that op `k` starts with.
fn pos_what_at(f: &Func, code: &pre::Code, k: usize) -> yavs_ir::CheckMsgId {
    match code.at.get(k).and_then(|&p| f.code.get(p as usize)) {
        Some(&Inst::Pos { what, .. }) => what,
        _ => yavs_ir::CheckMsgId(0),
    }
}

struct Dbg {
    session: Arc<DebugSession>,
    index: usize,
    mode: StepMode,
    fuel: u32,
}

#[derive(Default)]
struct Waits {
    sens: Vec<SigRef>,
    timeout: Option<Time>,
    stmt: Option<StmtId>,
}

/// The registers of all frames in one stack; `[]` reaches the top frame's.
#[derive(Default)]
pub(crate) struct Regs {
    pub v: Vec<u64>,
    /// The top frame's first register.
    pub base: usize,
}

impl std::ops::Index<usize> for Regs {
    type Output = u64;
    #[inline(always)]
    fn index(&self, i: usize) -> &u64 {
        &self.v[self.base + i]
    }
}

impl std::ops::IndexMut<usize> for Regs {
    #[inline(always)]
    fn index_mut(&mut self, i: usize) -> &mut u64 {
        &mut self.v[self.base + i]
    }
}

#[repr(C, align(64))]
pub(crate) struct Machine {
    // The first three cache lines hold what an activation of a small process reads and writes
    // (a design of thousands of processes misses in the cache on every line of every process
    // it touches: 30 data cache misses per activation of a near-idle process before they were
    // gathered here); the order of the fields is the point of `repr(C)`.
    pub frames: Vec<Frame>,
    /// The registers of all frames, one stack (`push_frame`, `pop_frame`; never push or pop
    /// `frames` directly).
    regs: Regs,
    /// Stop when the entry frame jumps to this block (a process's body, #342).
    pub pause: Option<yavs_ir::BlockId>,
    /// The slots of frame 0 (a process's variables survive its suspensions).
    root: Vec<u8>,
    pub temp: Vec<u8>,
    pub rt: Arc<Runtime>,
    /// A second handle on `rt` that `exec` takes while it runs, so an activation does not
    /// touch the shared reference count (every process of a batch has the same runtime).
    rt_run: Option<Arc<Runtime>>,
    pub inst: Arc<InstData>,
    pub drivers: Vec<ProcDriver>,
    dbg: Option<Box<Dbg>>,
    /// Bumped whenever the machine runs or a variable is written: unchanged means every
    /// variable of the process has its last value (the waveform recorder skips it).
    pub(crate) execs: u64,
    /// `regs` and `temp` went to the thread's scratch at the last suspension
    /// ([`Machine::release_scratch`]) and are empty until [`Machine::borrow_scratch`].
    released: bool,
    /// A small process keeps its registers and temporaries between activations instead of
    /// handing them to the thread's scratch (set when it starts, [`RESIDENT_REGS`]): no
    /// swap per activation. A process whose temporaries grow past [`RESIDENT_TEMP`] stops
    /// being resident at its next suspension.
    resident: bool,
    /// The entry function is a process (set when the process starts): its root frame needs
    /// no check of the frames it waits in ([`Machine::suspend`]).
    root_process: bool,
    /// Started, not ended, no shared state and no debugger: `InterpProcess::run` goes straight
    /// to `run_quick`.
    pub(crate) hot: bool,
    /// The activation (`execs`, as a 32-bit count; `u32::MAX`: never) at which
    /// `InterpProcess::run` tries the switch to a specialised copy: in the first lines with
    /// `execs`, which it is compared with at every activation.
    pub(crate) spec_at: u32,
    /// What the frame view of compiled code reads besides the fields above, one cache line
    /// (`view_root`; it was scattered over five lines of a machine and its instance data,
    /// which cost a design of thousands of processes three cache misses per activation).
    vc: ViewCache,
    /// What the compiled tier keeps for this process between activations, inline (a box of its
    /// own was one more cache line per activation, #766), in the fifth line with `troots`, whose
    /// length every activation reads.
    pub(crate) tier_state: crate::TierState,
    /// Temporaries that hold handles or heap addresses (`Inst::TempRoot`), until the
    /// statement that made them ends (`Inst::Stmt` drops those at or above its frame's base).
    troots: Vec<TempRoot>,
    pub heap: Heap,
    /// The roots scan of the last `publish_shared`, kept for its buffers.
    rscratch: heap::Refs,
    /// A native `ieee` subprogram made a report at the stop severity: the process ends after
    /// the call, as after `Inst::Report`.
    pub ieee_stop: bool,
    /// A native `ieee` subprogram gives its call back to the VHDL body (`ieee::fallback`).
    pub ieee_fallback: bool,
    /// The slots of the native frames of compiled code, for the frame indices `nwin_from` to
    /// `nwin_to`, while a call from a native frame runs (`TierHost::native_direct`).
    nwin: Option<Box<dyn crate::NativeSlots>>,
    nwin_from: usize,
    nwin_to: usize,
    /// The slots of the other frames, one stack: no per-call allocation.
    sstack: Vec<u8>,
    pub sh: Option<SharedState>,
    /// The package being initialised (`init_package`): its index and writable data.
    pub pkg_init: Option<(u32, Vec<u8>)>,
    /// Package initialisation did something other than compute package state (printed, read the
    /// environment or a clock, touched a file, called foreign code): its result is not stored
    /// (`SharedState::tainted`, #407).
    pub(crate) tainted: Option<String>,
    /// The reports it printed, to print again when its state is restored (`Some` while a
    /// package is initialised or a traced call runs, `call_values_traced`).
    pub(crate) init_reports: Option<Vec<(u8, String)>>,
    pub entry: FuncId,
    waits: Waits,
    /// The last `LayoutOf` of each type (indexed by `TyId`): descriptor size, descriptor bytes
    /// and the layout words written. Layouts are pure functions of type and descriptor.
    layouts: Vec<Option<LayoutMemo>>,
    /// The plans of record signal gathers and scatters this machine met (`gather.rs`).
    pub(crate) plans: Vec<crate::gather::MPlan>,
    /// Argument cells of the call being made (`call_with`).
    cells: Vec<u64>,
    /// The staging buffer of `Copy` and `Fill` (`take_bytes`), reused.
    bytes: Vec<u8>,
    /// `to_string` / `'image` texts of enumeration literals made so far: (type, image, text of each literal).
    pub(crate) enum_text: Vec<EnumText>,
    /// What this process added to [`RESIDENT_USED`], given back when it is dropped.
    resident_bytes: usize,
    /// The fault of the last `run_quick`.
    quick_fault: Option<Fault>,
    /// The process may switch to a copy specialised to its instance data (`specialise`): the
    /// parameters of `for` generates in its data, which the copy does not fold.
    spec: Option<Arc<[(u32, u32)]>>,
    /// The memo of pure calls this process keeps (`pmemo.rs`), made at its first memoised call.
    pub(crate) pm: Option<Box<crate::pmemo::State>>,
    /// Jumps back of a process that never suspends, to bound the activation (#583).
    spin: u64,
}

/// The fields of a machine that `view_root` reads and the interpreter does not (see
/// `Machine::vc`).
#[repr(C, align(64))]
struct ViewCache {
    /// The instance signals as compiled code reads them (`TierFrame::sigtab`), made at the first
    /// entry of compiled code from the kernel's signals (this machine belongs to one kernel).
    tier_sigs: Vec<crate::TierSig>,
    /// This machine for compiled code (`TierFrame::owner`): unique in the process.
    owner: u64,
    /// The instance data (`TierFrame::inst`, `inst_len`) and the number of its signals, as
    /// numbers (an `Arc<InstData>` never changes its vectors).
    inst_data: usize,
    inst_len: usize,
    nsigs: usize,
    /// `Limits::max_depth` of the runtime (`TierFrame::max_depth`).
    max_depth: u64,
}

/// Registers of a process's root frame up to which it keeps its working vectors
/// ([`Machine::resident`]): at most 32 KB of registers per process. A process above the limit
/// swapped its register file with the scratch at every activation, a copy from a template that is
/// cold in the cache (neorv32's big processes: 14% of the L1 misses; 256 registers, then 2048, now
/// 4096: its control unit's `exec_comb` has 2933 and the decompressor 3064).
const RESIDENT_REGS: usize = 4096;
/// Bytes of registers all resident processes together may keep (a design of very many processes
/// swaps the rest through the thread's scratch as before).
const RESIDENT_BUDGET: usize = 96 << 20;
static RESIDENT_USED: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
/// Capacity of the temporary area above which a resident process gives its vectors to the
/// scratch.
const RESIDENT_TEMP: usize = 1024;

/// The working vectors of the processes running on this thread: registers and temporaries. A
/// suspended process owns none of them (no register or temporary is live across a suspension,
/// ir.md 1.2), so a design of thousands of processes keeps a few sets per thread instead of a
/// set each (#409).
struct Scratch {
    slots: [ScratchSlot; SCRATCH_SLOTS],
    /// The slot a set is evicted from when none is empty.
    next: usize,
}

/// Sets of vectors kept per thread: processes of a few functions that wake in turn (the
/// specialised copies of one process, a pipeline of processes) each find their own, with the
/// registers they start with in place. GRLIB LEON3 has nine large processes (3000 to 7500
/// registers each) that wake every delta cycle: with four sets each wake refilled 25 to 60 KB of
/// registers (30 GB of `memcpy` in a run).
const SCRATCH_SLOTS: usize = 16;

impl Scratch {
    /// The slot to give up when every one holds a set that is worth keeping.
    fn evict(&mut self) -> usize {
        let i = self.next;
        self.next = (i + 1) % SCRATCH_SLOTS;
        i
    }
}

struct ScratchSlot {
    regs: Vec<u64>,
    temp: Vec<u8>,
    /// Which function's root frame `regs` holds (runtime id, function id plus one; 0: none).
    /// A definition that happens once is skipped by the loop and no other instruction writes
    /// its register, so the next process of the same function starts with the values it
    /// needs already in place: no `Pre::fill`.
    key: u64,
}

thread_local! {
    static SCRATCH: std::cell::RefCell<Scratch> = const {
        std::cell::RefCell::new(Scratch {
            slots: [const { ScratchSlot { regs: Vec::new(), temp: Vec::new(), key: 0 } }; SCRATCH_SLOTS],
            next: 0,
        })
    };
}

/// A recorded temporary (`Inst::TempRoot`): a value of `ty` constrained by `desc` at offset
/// `off` of the temporary area, or with `ty` `None` a fat pair `[data, desc]`.
struct TempRoot {
    off: u32,
    ty: Option<yavs_ir::TyId>,
    desc: Box<[u8]>,
}

struct LayoutMemo {
    dsz: u64,
    /// A one-dimensional array of scalars: the element size, or 0 for a logic array (bit
    /// planes). Its layout words come straight from the bounds, without the memo's descriptor.
    simple: Option<u64>,
    desc: Vec<u8>,
    words: Vec<u8>,
    /// The address of the descriptor `words` came from when that is constant data (it never
    /// changes), else 0: a layout of the same constant needs no descriptor read.
    ckey: u64,
}

/// The layout memo of `ty` (`Machine::layouts`) made from the descriptor bytes `d` of `dsz`
/// bytes, reusing `memo` (refilled in place: strings of changing lengths miss at nearly every
/// call, and the memo must not allocate then). `ckey` is the constant descriptor's address, or 0.
fn memo_fill(
    prog: &Program,
    ty: TyId,
    mut memo: Option<LayoutMemo>,
    dsz: u64,
    d: &[u8],
    ckey: u64,
) -> R<LayoutMemo> {
    if let Some(m) = memo.as_mut()
        && let Some(n) = m.simple
        && let Some(w) = simple_words(n, d)
    {
        m.words.clear();
        for w in w {
            m.words.extend_from_slice(&w.to_le_bytes());
        }
        m.ckey = ckey;
        return memo.ok_or_else(|| ice_s!("layout memo missing"));
    }
    if memo.as_ref().is_none_or(|m| m.desc != d) {
        let (l, _) = Ty::layout(&prog.types, &prog.consts, ty, d).map_err(|e| match e {
            LayoutError::TooLarge => Fault::Runtime(e.to_string()),
            LayoutError::Malformed(_) => ice(e.to_string()),
        })?;
        let m = memo.get_or_insert_with(|| LayoutMemo {
            dsz,
            simple: match prog.types.get(ty) {
                Some(Ty::Array {
                    elem,
                    elem_desc: None,
                    dims: 1,
                }) => match prog.types.get(*elem) {
                    Some(Ty::Enum { logic: Some(_), .. }) => Some(0),
                    Some(e) if Ty::logic_leaf(&prog.types, ty).is_none() => e.elem_size(),
                    _ => None,
                },
                _ => None,
            },
            desc: Vec::new(),
            words: Vec::new(),
            ckey: 0,
        });
        m.ckey = ckey;
        m.desc.clear();
        m.desc.extend_from_slice(d);
        m.words.clear();
        let sc = [l.stride, l.count];
        let tail = if l.offsets.is_empty() {
            sc.as_slice()
        } else {
            l.offsets.as_slice()
        };
        for w in [l.size, l.align].iter().chain(tail) {
            m.words.extend_from_slice(&w.to_le_bytes());
        }
    }
    memo.ok_or_else(|| ice_s!("layout memo missing"))
}

/// The layout words `[size, align, stride, count]` of a one-dimensional array of `n`-byte
/// scalars (`n` 0: of logic elements) from its bounds; `None` for a bad direction or a size
/// beyond 64 bits (the general path reports those).
fn simple_words(n: u64, d: &[u8]) -> Option<[u64; 4]> {
    let b = d.get(..24)?;
    let dir = le_bytes(&b[16..]);
    if dir > 1 {
        return None;
    }
    let count = yavs_ir::dim_len(
        le_bytes(&b[..8]) as i64,
        le_bytes(&b[8..16]) as i64,
        dir == 1,
    )?;
    if n == 0 {
        return Some([count.div_ceil(64).checked_mul(32)?, 8, 0, count]);
    }
    Some([n.checked_mul(count)?, n, n, count])
}

/// Word `k` of layout words.
fn word_of(words: &[u8], k: u32) -> R<u64> {
    let o = k as usize * 8;
    match words.get(o..o + 8) {
        Some(b) => Ok(le_bytes(b)),
        None => Err(ice_f!(
            "read of 8 bytes at {o} outside a temporary object of {} bytes",
            words.len()
        )),
    }
}

fn sev(v: u64) -> R<Severity> {
    Ok(match v {
        0 => Severity::Note,
        1 => Severity::Warning,
        2 => Severity::Error,
        3 => Severity::Failure,
        _ => return Err(ice_f!("severity {v}")),
    })
}

#[inline(always)]
pub(crate) fn to_time(v: i64) -> R<Time> {
    Time::from_vhdl(v).map_err(|_| runtime_f!("negative time value {v} fs"))
}

/// The left, right and direction words of a dimension descriptor.
#[inline(always)]
fn desc_words(b: &[u8; 24]) -> (i64, i64, u64) {
    let c = b.as_chunks::<8>().0;
    (
        u64::from_le_bytes(c[0]) as i64,
        u64::from_le_bytes(c[1]) as i64,
        u64::from_le_bytes(c[2]),
    )
}

/// The length of a dimension (bounds and direction word, `dim_of`) in `u64` arithmetic, `None`
/// where it would overflow (the general code then decides).
#[inline(always)]
fn dim_len63((l, r, dir): (i64, i64, u64)) -> Option<u64> {
    let (lo, hi) = if dir == 1 { (r, l) } else { (l, r) };
    if hi < lo {
        Some(0)
    } else {
        hi.checked_sub(lo).and_then(|d| (d as u64).checked_add(1))
    }
}

/// The eight bytes at `off` of a memory object of `len` bytes, when they are inside it.
#[inline(always)]
fn slot_range(len: usize, off: u64) -> Option<std::ops::Range<usize>> {
    let o = usize::try_from(off).ok()?;
    (o.checked_add(8)? <= len).then(|| o..o + 8)
}

/// The little-endian value of 1, 4 or 8 bytes (fixed-size copies, no `memcpy` call).
#[inline(always)]
fn le_bytes(b: &[u8]) -> u64 {
    match *b {
        [a] => u64::from(a),
        [a, b, c, d] => u64::from(u32::from_le_bytes([a, b, c, d])),
        [a, b, c, d, e, f, g, h] => u64::from_le_bytes([a, b, c, d, e, f, g, h]),
        _ => {
            let mut x = [0u8; 8];
            let n = b.len().min(8);
            x[..n].copy_from_slice(&b[..n]);
            u64::from_le_bytes(x)
        }
    }
}

/// Store the low `b.len()` bytes of `v`, little-endian.
#[inline(always)]
fn put_le(b: &mut [u8], v: u64) {
    match b.len() {
        1 => b[0] = v as u8,
        4 => b.copy_from_slice(&(v as u32).to_le_bytes()),
        8 => b.copy_from_slice(&v.to_le_bytes()),
        n => b.copy_from_slice(&v.to_le_bytes()[..n]),
    }
}

fn arith_r(r: Result<i64, arith::ArithError>) -> R<u64> {
    r.map(|v| v as u64).map_err(|e| match e {
        arith::ArithError::DivisionByZero { .. } => runtime_s!("division by zero"),
        e => Fault::Runtime(e.to_string()),
    })
}

fn int_op(op: IntOp, x: i64, y: i64) -> Result<i64, arith::ArithError> {
    match op {
        IntOp::Add => arith::add(x, y),
        IntOp::Sub => arith::sub(x, y),
        IntOp::Mul => arith::mul(x, y),
        IntOp::Div => arith::div(x, y),
        IntOp::Rem => arith::rem(x, y),
        IntOp::Mod => arith::modulo(x, y),
        IntOp::Pow => arith::pow(x, y),
    }
}

/// `int_op` for the cases that cannot fail, inlined; `None`: use `int_op` for the result or
/// the error.
#[inline(always)]
fn fast_op(op: IntOp, x: i64, y: i64) -> Option<i64> {
    match op {
        IntOp::Add => x.checked_add(y),
        IntOp::Sub => x.checked_sub(y),
        IntOp::Mul => x.checked_mul(y),
        // Natural operands below 2^32 (the common case) divide in 32 bits: much faster.
        IntOp::Div | IntOp::Rem | IntOp::Mod if y != 0 && (x | y) as u64 >> 32 == 0 => {
            let (a, b) = (x as u32, y as u32);
            Some(i64::from(if op == IntOp::Div { a / b } else { a % b }))
        }
        IntOp::Div if y != 0 => x.checked_div(y),
        IntOp::Rem if y != 0 => Some(x.checked_rem(y).unwrap_or(0)),
        IntOp::Mod if y != 0 => {
            let r = x.checked_rem(y).unwrap_or(0);
            Some(if r != 0 && ((r < 0) != (y < 0)) {
                r + y
            } else {
                r
            })
        }
        _ => None,
    }
}

/// Checked integer arithmetic (rule 7).
#[inline(always)]
fn bin(op: IntOp, x: i64, y: i64) -> R<u64> {
    match fast_op(op, x, y) {
        Some(v) => Ok(v as u64),
        None => arith_r(int_op(op, x, y)),
    }
}

/// Whether `x` is within 2^30 (an `Aff` op's operands: no intermediate result of the
/// instructions it replaces can overflow).
#[inline(always)]
fn aff_ok(x: i64) -> bool {
    (x.wrapping_add(1 << 30) as u64) >> 31 == 0
}

/// `base + pos * stride`.
#[inline(always)]
fn index_addr(base: u64, pos: u64, stride: u32) -> R<u64> {
    let off = pos
        .checked_mul(u64::from(stride))
        .ok_or_else(|| runtime_s!("object larger than 4 GiB"))?;
    add_addr(base, off)
}

fn op_symbol(op: IntOp) -> &'static str {
    match op {
        IntOp::Add => "+",
        IntOp::Sub => "-",
        IntOp::Mul => "*",
        IntOp::Div => "/",
        IntOp::Rem => "rem",
        IntOp::Mod => "mod",
        IntOp::Pow => "**",
    }
}

/// A real as check messages show it: `real'image` (`%.15g`, sim1's form: `30000000000`).
fn real_text(x: f64) -> String {
    String::from_utf8_lossy(&yavs_std::image::real_image(x)).into_owned()
}

/// `<kind> <v> outside of [<subtype> ]range <bounds>[ for <what>]` (sim1's wording).
fn outside(kind: &str, v: &str, b: &str, m: &yavs_ir::CheckMsg) -> String {
    let mut s = format!("{kind} {v} outside of ");
    if !m.subtype.is_empty() {
        s.push_str(&m.subtype);
        s.push(' ');
    }
    s.push_str("range ");
    s.push_str(b);
    if !m.what.is_empty() {
        s.push_str(" for ");
        s.push_str(&m.what);
    }
    s
}

fn real_ok(x: f64, what: &str) -> R<u64> {
    if x.is_finite() {
        Ok(x.to_bits())
    } else {
        rt_err_f!("real {what} result {x} is out of range")
    }
}

pub(crate) fn value_size(elem: Elem, len: u32) -> u64 {
    match elem {
        Elem::Bytes(es) => u64::from(es) * u64::from(len),
        Elem::Logic => u64::from(len).div_ceil(64) * 32,
    }
}

fn blocks_of(bytes: &[u8]) -> Vec<[u64; 4]> {
    bytes
        .as_chunks::<32>()
        .0
        .iter()
        .map(|c| {
            let w = |k: usize| {
                let mut a = [0; 8];
                a.copy_from_slice(&c[k * 8..k * 8 + 8]);
                u64::from_le_bytes(a)
            };
            [w(0), w(1), w(2), w(3)]
        })
        .collect()
}

/// Whole blocks `out` from the bits of `src` from bit `sh` (1 to 63) of its first block: block
/// `j` of `out` is the high `64 - sh` bits of block `j` of `src` and the low `sh` bits of the
/// next, plane by plane (`src` has one block more than `out`).
#[inline(always)]
fn shift_merge(out: &mut [u8], src: &[u8], sh: u64) {
    let word = |b: &[u8], i: usize| plane_words_at(b, i);
    for (j, o) in out.as_chunks_mut::<32>().0.iter_mut().enumerate() {
        let (lo, hi) = (word(src, j * 32), word(src, (j + 1) * 32));
        for (k, c) in o.as_chunks_mut::<8>().0.iter_mut().enumerate() {
            *c = (lo[k] >> sh | hi[k] << (64 - sh)).to_le_bytes();
        }
    }
}

/// The four plane words of the block at byte `at` of `b`.
#[inline(always)]
fn plane_words_at(b: &[u8], at: usize) -> [u64; 4] {
    plane_words(&b[at..at + 32])
}

/// The four plane words of the block `b` (32 bytes).
#[inline(always)]
fn plane_words(b: &[u8]) -> [u64; 4] {
    let mut w = [0u64; 4];
    for (w, c) in w.iter_mut().zip(b.as_chunks::<8>().0) {
        *w = u64::from_le_bytes(*c);
    }
    w
}

/// The blocks of `bytes` (32 bytes each) into `out`.
#[inline(always)]
fn fill_blocks(bytes: &[u8], out: &mut [[u64; 4]]) {
    for (o, c) in out.iter_mut().zip(bytes.as_chunks::<32>().0) {
        for (w, b) in o.iter_mut().zip(c.as_chunks::<8>().0) {
            *w = u64::from_le_bytes(*b);
        }
    }
}

/// `n` elements of the planes `sw` from element `s0` to those of `dw` from element `d0`
/// (`Inst::LCopy`; compiled code calls it on the same memory, `yavs-llvm`).
#[inline(never)]
pub fn copy_bits(sw: &[[u64; 4]], dw: &mut [[u64; 4]], s0: u64, d0: u64, n: u64) {
    // One block on each side: shift and mask the four planes.
    if let ([s], [d]) = (sw, &mut *dw)
        && n <= 64
        && s0 + n <= 64
        && d0 + n <= 64
    {
        let mask = if n == 64 { !0 } else { (1u64 << n) - 1 };
        for (dp, sp) in d.iter_mut().zip(s) {
            *dp = *dp & !(mask << d0) | ((sp >> s0) & mask) << d0;
        }
        return;
    }
    yavs_logic::wide::copy_range(dw, d0 as usize, sw, s0 as usize, n as usize);
}

fn bytes_of(blocks: &[[u64; 4]]) -> Vec<u8> {
    let mut out = Vec::with_capacity(blocks.len() * 32);
    for w in blocks.as_flattened() {
        out.extend_from_slice(&w.to_le_bytes());
    }
    out
}

/// The bytes of constant `c`: compact constants are expanded once per runtime (rule 12).
fn const_bytes(rt: &Runtime, c: yavs_ir::ConstId) -> R<&[u8]> {
    match rt.prog.consts.get(c) {
        Some(ConstData::Bytes(b)) => Ok(b),
        Some(d) => {
            let cell = rt
                .expanded
                .get(c.0 as usize)
                .ok_or_else(|| ice_f!("no constant {c}"))?;
            if cell.get().is_none() {
                let _ = cell.set(expand_const(d)?);
            }
            Ok(cell.get().map(Vec::as_slice).unwrap_or(&[]))
        }
        None => Err(ice_f!("no constant {c}")),
    }
}

/// Plane `p` of the 64-element block at `b` (32 bytes, plane words little endian).
#[inline(always)]
fn plane_word(b: &[u8], p: usize) -> u64 {
    let mut w = [0u8; 8];
    w.copy_from_slice(&b[p * 8..p * 8 + 8]);
    u64::from_le_bytes(w)
}

fn set_bits(words: &mut [[u64; 4]], start: u64, m: u64, p: usize, v: u64) {
    let mask = if m < 64 { (1u64 << m) - 1 } else { !0 };
    let (w, s) = ((start / 64) as usize, start % 64);
    if let Some(b) = words.get_mut(w) {
        b[p] = b[p] & !(mask << s) | (v & mask) << s;
    }
    if s > 0
        && s + m > 64
        && let Some(b) = words.get_mut(w + 1)
    {
        let hm = mask >> (64 - s);
        b[p] = b[p] & !hm | (v >> (64 - s)) & hm;
    }
}

fn cmp(op: CmpOp, o: std::cmp::Ordering) -> bool {
    use std::cmp::Ordering::*;
    match op {
        CmpOp::Eq => o == Equal,
        CmpOp::Ne => o != Equal,
        CmpOp::Lt => o == Less,
        CmpOp::Le => o != Greater,
        CmpOp::Gt => o == Greater,
        CmpOp::Ge => o != Less,
    }
}

fn bounds(lo: i64, hi: i64, dir: Dir) -> String {
    match dir {
        Dir::To => format!("{lo} to {hi}"),
        Dir::Downto => format!("{hi} downto {lo}"),
    }
}

impl Drop for Machine {
    fn drop(&mut self) {
        if self.resident_bytes != 0 {
            RESIDENT_USED.fetch_sub(self.resident_bytes, Ordering::Relaxed);
        }
    }
}

/// A write count was asked for (`Machine::stamp`): from then on compiled code reports the
/// writes of the heap objects it keeps (`TIER_STAMPS`).
static STAMPS: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

impl Machine {
    pub(crate) fn new(
        rt: Arc<Runtime>,
        inst: Arc<InstData>,
        drivers: Vec<ProcDriver>,
        entry: FuncId,
    ) -> Machine {
        static OWNERS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        Machine {
            vc: ViewCache {
                tier_sigs: Vec::new(),
                owner: u64::from(rt.id) << crate::OWNER_RT_SHIFT
                    | OWNERS.fetch_add(1, Ordering::Relaxed) & u64::from(u32::MAX),
                inst_data: inst.data.as_ptr() as usize,
                inst_len: inst.data.len(),
                nsigs: inst.sigs.len(),
                max_depth: u64::from(rt.limits.max_depth),
            },
            nwin: None,
            nwin_from: 0,
            nwin_to: 0,
            rt_run: Some(rt.clone()),
            rt,
            inst,
            drivers,
            frames: Vec::new(),
            regs: Regs::default(),
            root: Vec::new(),
            sstack: Vec::new(),
            heap: Heap::new(heap::PRIVATE),
            temp: Vec::new(),
            troots: Vec::new(),
            sh: None,
            pkg_init: None,
            tainted: None,
            init_reports: None,
            entry,
            dbg: None,
            waits: Waits::default(),
            pause: None,
            ieee_stop: false,
            ieee_fallback: false,
            layouts: Vec::new(),
            plans: Vec::new(),
            cells: Vec::new(),
            bytes: Vec::new(),
            released: false,
            tier_state: crate::TierState::default(),
            resident: false,
            enum_text: Vec::new(),
            resident_bytes: 0,
            root_process: false,
            quick_fault: None,
            spec: None,
            pm: None,
            spec_at: u32::MAX,
            hot: false,
            rscratch: heap::Refs::default(),
            execs: 0,
            spin: spin_range().0,
        }
    }

    /// The `Scratch::key` of the root frame's function.
    #[inline(always)]
    fn scratch_key(&self) -> u64 {
        self.frames.first().map_or(0, |fr| {
            u64::from(self.rt.id) << 32 | (u64::from(fr.func.0) + 1)
        })
    }

    /// Before a process runs again: take this thread's scratch vectors back.
    #[inline(always)]
    pub(crate) fn borrow_scratch(&mut self) -> R<()> {
        if !self.released {
            return Ok(());
        }
        self.released = false;
        let key = self.scratch_key();
        let pristine = SCRATCH.with(|s| {
            let mut s = s.borrow_mut();
            let hit = (key != 0)
                .then(|| s.slots.iter().position(|e| e.key == key))
                .flatten();
            // (No hit: any set will do, it is filled; one with room is best.)
            let i = hit.unwrap_or_else(|| {
                let free = |e: &ScratchSlot| e.key == 0;
                (s.slots.iter())
                    .position(|e| free(e) && e.regs.capacity() > 0)
                    .or_else(|| s.slots.iter().position(free))
                    .unwrap_or_else(|| s.evict())
            });
            let e = &mut s.slots[i];
            std::mem::swap(&mut self.regs.v, &mut e.regs);
            std::mem::swap(&mut self.temp, &mut e.temp);
            // The slot now holds the machine's empty vectors.
            e.key = 0;
            hit.is_some()
        });
        self.regs.base = 0;
        if pristine {
            self.temp.clear();
            if !self.troots.is_empty() {
                self.troots.clear();
            }
            return Ok(());
        }
        self.borrow_fill()
    }

    /// The registers a frame starts with, after `borrow_scratch` found another function's.
    #[inline(never)]
    fn borrow_fill(&mut self) -> R<()> {
        let root = self.frames.first().map(|fr| fr.func);
        // Registers are written before they are read: stale values are harmless, except
        // those a frame starts with (`pre.rs`).
        let rt = &self.rt;
        match root.and_then(|id| Some((id, rt.prog.func(id)?))) {
            Some((id, f)) => {
                let pre = rt.pre(id, f).ok_or_else(|| ice_f!("no function {id}"))?;
                self.regs.v.clear();
                pre.fill(&mut self.regs.v, 0)?;
            }
            None => self.regs.v.clear(),
        }
        self.temp.clear();
        self.troots.clear();
        Ok(())
    }

    /// At a suspension in the root frame: hand the working vectors to this thread's scratch.
    /// Deeper frames keep theirs (their registers and temporaries are not touched, so a
    /// procedure that waits resumes as before).
    #[inline(always)]
    pub(crate) fn release_scratch(&mut self) {
        if self.frames.len() != 1 || self.dbg.is_some() {
            return;
        }
        // A resident process has no debugger (`set_resident`) and keeps its vectors.
        if self.resident && self.temp.capacity() <= RESIDENT_TEMP {
            self.temp.clear();
            if !self.troots.is_empty() {
                self.troots.clear();
            }
            return;
        }
        if self.released {
            return;
        }
        self.resident = false;
        self.released = true;
        if !self.troots.is_empty() {
            self.troots.clear();
        }
        let key = self.scratch_key();
        SCRATCH.with(|s| {
            let mut s = s.borrow_mut();
            let i = (s.slots.iter())
                .position(|e| e.key == 0 && e.regs.capacity() == 0)
                .or_else(|| s.slots.iter().position(|e| e.key == 0))
                .unwrap_or_else(|| s.evict());
            let e = &mut s.slots[i];
            e.regs = std::mem::take(&mut self.regs.v);
            e.temp = std::mem::take(&mut self.temp);
            e.key = key;
        });
    }

    pub(crate) fn attach(&mut self, session: Arc<DebugSession>, index: usize) {
        self.spec = None;
        self.spec_at = u32::MAX;
        self.dbg = Some(Box::new(Dbg {
            session,
            index,
            mode: StepMode::Run,
            fuel: 0,
        }));
    }

    /// Push the frame of `func` with `args` (one cell per parameter, two for a signal).
    pub(crate) fn enter(&mut self, func: FuncId, args: &[u64]) -> R<()> {
        let rt = self.rt.clone();
        let f = rt
            .prog
            .func(func)
            .ok_or_else(|| ice_f!("no function {func}"))?;
        self.push_frame(f, func, None, None)?;
        let mut k = 0;
        for p in &f.params {
            let n = if p.class == ParamClass::Signal { 2 } else { 1 };
            let cells = args
                .get(k..k + n)
                .ok_or_else(|| runtime_f!("missing argument {}", p.name))?;
            store_cells(self.top_slots(), p.slot, p.width, cells)?;
            k += n;
        }
        Ok(())
    }

    /// Push a frame for `func` (`f`) on the stacks: its registers (`Pre::fill`) and zeroed
    /// slots at the top; `ret` is the caller register receiving the result.
    pub(crate) fn push_frame(
        &mut self,
        f: &Func,
        func: FuncId,
        link: Option<u32>,
        ret: Option<Reg>,
    ) -> R<()> {
        let pre = self
            .rt
            .pre(func, f)
            .ok_or_else(|| ice_f!("no function {func}"))?;
        push_frame_body!(self, pre, f, func, link, ret, pre.slots);
        Ok(())
    }

    /// `push_frame` for a caller that has the function's `Pre` (from the `Runtime` it runs
    /// with, which is not borrowed from the machine).
    #[inline(always)]
    pub(crate) fn push_frame_pre(
        &mut self,
        f: &Func,
        func: FuncId,
        pre: &pre::Pre,
        link: Option<u32>,
        ret: Option<Reg>,
    ) -> R<()> {
        push_frame_body!(self, pre, f, func, link, ret, pre.slots);
        Ok(())
    }

    /// `push_frame_pre` for a call whose parameters go to registers only (`Pre::lean`): the
    /// frame has the slots the callee needs beside them (`Pre::fslots`).
    #[inline(always)]
    fn push_frame_lean(
        &mut self,
        f: &Func,
        func: FuncId,
        pre: &pre::Pre,
        link: Option<u32>,
        ret: Option<Reg>,
    ) -> R<()> {
        push_frame_body!(self, pre, f, func, link, ret, pre.fslots);
        Ok(())
    }

    #[cold]
    #[inline(never)]
    fn depth_fail(&self, f: &Func) -> R<()> {
        rt_err_f!(
            "call depth limit {} exceeded calling `{}` (unbounded recursion?)",
            self.rt.limits.max_depth,
            f.name
        )
    }

    /// Pop the top frame; the caller's registers become current.
    #[inline(never)]
    pub(crate) fn pop_frame(&mut self) -> Option<Frame> {
        let fr = self.pop_frame_in_place()?;
        // Back at the root: a deep recursion does not keep its stack.
        if self.frames.len() == 1 && self.sstack.capacity() > 1 << 16 {
            self.sstack.shrink_to(1 << 12);
            self.regs.v.shrink_to(1 << 10);
        }
        Some(fr)
    }

    /// `pop_frame` for a return in the fast loop: only the register that receives the result
    /// is handed back (the frame, 40 bytes, is not moved out).
    #[inline(always)]
    fn pop_ret(&mut self) -> Option<Option<Reg>> {
        let fr = self.frames.pop()?;
        self.regs.v.truncate(fr.rbase as usize);
        match self.frames.last() {
            Some(top) => {
                self.regs.base = top.rbase as usize;
                self.sstack.truncate(fr.sbase as usize);
                // Back at the root: a deep recursion does not keep its stack.
                if self.frames.len() == 1 && self.sstack.capacity() > 1 << 16 {
                    self.shrink_stacks();
                }
            }
            None => {
                self.regs.base = 0;
                self.root.clear();
            }
        }
        Some(fr.ret)
    }

    #[cold]
    #[inline(never)]
    fn shrink_stacks(&mut self) {
        self.sstack.shrink_to(1 << 12);
        self.regs.v.shrink_to(1 << 10);
    }

    /// `pop_frame` without giving memory back: the stacks do not move (compiled code of the
    /// caller holds a pointer to its registers, `TierHost::ret`).
    fn pop_frame_in_place(&mut self) -> Option<Frame> {
        let fr = self.frames.pop()?;
        self.regs.v.truncate(fr.rbase as usize);
        match self.frames.last() {
            Some(top) => {
                self.regs.base = top.rbase as usize;
                self.sstack.truncate(fr.sbase as usize);
            }
            None => {
                self.regs.base = 0;
                self.root.clear();
            }
        }
        Some(fr)
    }

    /// Pop frames down to `n`.
    pub(crate) fn truncate_frames(&mut self, n: usize) {
        while self.frames.len() > n {
            self.pop_frame();
        }
    }

    /// The slots of frame `i`.
    #[inline(always)]
    pub(crate) fn fslots(&self, i: usize) -> Option<&[u8]> {
        slots_of(&self.frames, &self.root, &self.sstack, i)
    }

    #[inline(always)]
    pub(crate) fn fslots_mut(&mut self, i: usize) -> Option<&mut [u8]> {
        slots_of_mut(&self.frames, &mut self.root, &mut self.sstack, i)
    }

    /// The slots of the top frame.
    #[inline(always)]
    fn top_slots(&mut self) -> &mut [u8] {
        let i = self.frames.len() - 1;
        match self.fslots_mut(i) {
            Some(s) => s,
            None => &mut [],
        }
    }

    /// The statement of the instruction at the top frame's pc is in a source file of the `ieee`
    /// library (also when its function was inlined: the statement stays the original one).
    fn in_ieee(&self, prog: &Program, f: &Func) -> bool {
        let pc = self.frames.last().map_or(0, |fr| fr.pc as usize);
        let path = f
            .loc
            .get(pc)
            .and_then(|&s| prog.stmts.get(s))
            .and_then(|st| self.rt.sources.path(st.span));
        match (path, crate::IEEE_DIR.get()) {
            (Some(p), Some(dir)) => std::path::Path::new(p).starts_with(dir),
            _ => false,
        }
    }

    /// The VHDL call stack, innermost first.
    fn trace(&self) -> Vec<StmtId> {
        self.frames
            .iter()
            .rev()
            .flat_map(|fr| {
                let f = self.rt.prog.func(fr.func)?;
                let pc = fr.pc as usize;
                // A function inlined at this instruction is a frame of its own.
                Some(
                    f.loc
                        .get(pc)
                        .copied()
                        .into_iter()
                        .chain(f.inlined_chain(pc).map(|n| n.site)),
                )
            })
            .flatten()
            .collect()
    }

    pub(crate) fn error(&self, f: Fault, ctx: Option<&Ctx<'_>>) -> SimError {
        let mk = |k: ErrorKind| match ctx {
            Some(c) => c.error(k),
            None => SimError::new(k),
        };
        let mut e = match f {
            Fault::Runtime(m) => mk(ErrorKind::Runtime(m)),
            Fault::Unbound(b) => {
                let (message, searched) = *b;
                mk(ErrorKind::Foreign { message, searched })
            }
            Fault::Internal(m) => mk(ErrorKind::Internal(format!("interpreter: {m}"))),
            Fault::Kernel(e) => e,
            Fault::Restart => mk(ErrorKind::Runtime(crate::RESTART.into())),
        };
        if e.trace.is_empty() {
            e.trace = self.trace();
        }
        e
    }

    pub(crate) fn run(&mut self, ctx: Option<&mut Ctx<'_>>, base: usize) -> R<Outcome> {
        let mut ctx = ctx;
        if self.dbg.is_some() {
            self.exec::<true>(&mut ctx, base)
        } else {
            self.exec::<false>(&mut ctx, base)
        }
    }

    /// Ask to switch to a copy of the entry function specialised to this machine's instance
    /// data (`Program::copy_of`) at activation `at` (`holes`: the parameters of `for` generates
    /// in the data, which the copy reads like the shared code). The process must be one a copy
    /// can stand in for (`Program::specialisable`) and run without a debugger.
    pub(crate) fn specialise_at(&mut self, at: u64, holes: Arc<[(u32, u32)]>) {
        if self.dbg.is_none() && self.rt.prog.specialisable(self.entry) {
            self.rt.prog.note_planned();
            self.spec = Some(holes);
            // First stop at the second activation (`specialise`): it tells the tier the process runs again.
            self.spec_at = u32::try_from(at.min(2)).unwrap_or(u32::MAX);
        }
    }

    /// The switch to the copy, tried once, between two activations: the process waits at its
    /// one wait statement in its root frame. The copy keeps the variables where the lowered
    /// code put them and resumes after its own wait; what the shared code kept behind the
    /// lowered frame (memo flags and values, layout slots) is dropped, and the registers are
    /// those of the copy. The process stays on the shared code when a compiled version of it
    /// exists (the tier decides what to compile) or the copy cannot be had.
    #[cold]
    #[inline(never)]
    pub(crate) fn specialise(&mut self) {
        if self.spec.is_some() && self.execs < crate::spec_after() {
            if let Some(n) = self.rt.native.resumed.get(self.entry.0 as usize) {
                n.store(true, Ordering::Relaxed);
            }
            self.spec_at = u32::try_from(crate::spec_after()).unwrap_or(u32::MAX);
            return;
        }
        let Some(holes) = self.spec.take() else {
            return;
        };
        self.spec_at = u32::MAX;
        let rt = self.rt.clone();
        let prog = &rt.prog;
        let base = self.entry;
        if self.frames.len() != 1 || self.dbg.is_some() || self.pause.is_some() || self.sh.is_some()
        {
            return;
        }
        // The shared code is compiled already: a tier that compiles copies takes the copy over at
        // once (compiled code of the copy is smaller and faster: folded layouts and descriptors,
        // #679), else a copy would run interpreted for good. (A function the tier is asked to
        // compile on entry, `NAT_ASK`, is not: the copy has the flag as well,
        // `Runtime::set_eager`.) A process whose shared code is not compiled switches at once,
        // with a tier too: the tier compiles the copy (#675).
        if !crate::spec_forced() {
            let compiled = rt
                .native
                .func
                .get(base.0 as usize)
                .is_some_and(|n| n.load(Ordering::Acquire) & NAT_CODE != 0);
            if compiled && !rt.tier.get().is_some_and(|t| t.compiles_copies()) {
                return;
            }
        }
        let Ok(id) = prog.copy_of_pkgs(base, &self.inst.data, &holes, &*rt) else {
            return;
        };
        let (Some(old), Some(new), Some(raw)) = (prog.func(base), prog.func(id), prog.raw(base))
        else {
            return;
        };
        // The wait the process stands after, and the copy's: one wait statement each.
        let pc = self.frames[0].pc as usize;
        let waits_at = |f: &Func, pc: usize| {
            pc.checked_sub(1)
                .and_then(|p| f.code.get(p))
                .is_some_and(|i| matches!(i, Inst::Suspend { .. }))
        };
        let Some(&at) = new.resume.first() else {
            return;
        };
        if !waits_at(old, pc) || !waits_at(new, at as usize) || new.frame < raw.frame {
            return;
        }
        let Some(pre) = rt.pre(id, new) else {
            return;
        };
        let keep = raw.frame as usize;
        if self.root.len() > keep {
            self.root[keep..].fill(0);
        }
        self.root.resize(pre.slots as usize, 0);
        let fr = &mut self.frames[0];
        (fr.func, fr.pc, fr.slen) = (id, at, pre.slots);
        self.entry = id;
        prog.note_switch();
        self.tier_state = crate::TierState::default();
        // A resident process holds registers filled for the shared code: fill the copy's, and
        // stay resident if they fit. A released one gets the copy's from `borrow_fill`, its
        // scratch being keyed by function.
        if !self.released {
            // (Exactly the copy's registers: a vector that doubled would hold twice the bytes in
            // every one of thousands of resident processes.)
            let n = new.reg_ty.len();
            self.regs.v.clear();
            self.regs.v.shrink_to(n);
            self.regs.v.reserve_exact(n);
            let _ = pre.fill(&mut self.regs.v, 0);
            if self.resident {
                let bytes = self.regs.v.len() * 8 + RESIDENT_TEMP;
                let old = self.resident_bytes;
                let fits = self.regs.v.len() <= RESIDENT_REGS
                    && RESIDENT_USED
                        .try_update(Ordering::Relaxed, Ordering::Relaxed, |u| {
                            let n = u.saturating_sub(old) + bytes;
                            (n <= RESIDENT_BUDGET).then_some(n)
                        })
                        .is_ok();
                if fits {
                    self.resident_bytes = bytes;
                } else {
                    RESIDENT_USED.fetch_sub(old, Ordering::Relaxed);
                    (self.resident, self.resident_bytes) = (false, 0);
                }
            }
        }
    }

    /// After the root frame is entered: a small process keeps its working vectors
    /// ([`Machine::resident`]).
    pub(crate) fn set_resident(&mut self) {
        let bytes = self.regs.v.len() * 8 + RESIDENT_TEMP;
        self.resident = self.dbg.is_none()
            && self.regs.v.len() <= RESIDENT_REGS
            && RESIDENT_USED
                .try_update(Ordering::Relaxed, Ordering::Relaxed, |u| {
                    (u + bytes <= RESIDENT_BUDGET).then_some(u + bytes)
                })
                .is_ok();
        if self.resident {
            self.resident_bytes = bytes;
        }
        self.root_process = true;
    }

    /// Whether a debugger is attached.
    pub(crate) fn has_debugger(&self) -> bool {
        self.dbg.is_some()
    }

    /// Whether `run_quick` applies (no debugger).
    pub(crate) fn can_run_quick(&self) -> bool {
        self.dbg.is_none()
    }

    /// `run` without a debugger and with base 0, the outcome as a `Quick`. `cont`: compiled code
    /// has run this activation already ([`Machine::run_direct`] said [`Direct::Continue`]) and
    /// the interpreter goes on from the frames' pcs.
    pub(crate) fn run_quick(&mut self, ctx: &mut Option<&mut Ctx<'_>>, cont: bool) -> Quick {
        let rt = self.rt_run.take().unwrap_or_else(|| self.rt.clone());
        let r = match rt.tier.get() {
            // `any` is read once: counting an activation (`count_on`) sets it too.
            Some(tier) => {
                let any = rt.native.any.load(Ordering::Acquire);
                if cont || (any && self.root_flagged(&rt)) {
                    let slot = crate::slot();
                    if !cont && let Some(fr) = self.frames.first() {
                        rt.native.activated(fr.func);
                    }
                    let r = self.exec_fast::<true, false>(&rt, Some(&**tier), None, ctx, 0);
                    slot.store(0, Ordering::Relaxed);
                    r
                } else {
                    let slot = crate::slot();
                    if any && let Some(fr) = self.frames.first() {
                        rt.native.activated(fr.func);
                    }
                    self.exec_fast::<false, true>(&rt, None, Some(slot), ctx, 0)
                }
            }
            None => self.exec_fast::<false, false>(&rt, None, None, ctx, 0),
        };
        self.rt_run = Some(rt);
        match r {
            Ok(Outcome::Suspended) => Quick::Suspended,
            Ok(Outcome::Returned(_)) => Quick::Returned,
            Err(e) => {
                self.quick_fault = Some(e);
                Quick::Fault
            }
        }
    }

    /// Whether the full tier loop runs this activation: the function the top frame is in has
    /// compiled code, or is to be compiled (state 1, 2). Code the tier has not touched runs in
    /// the plain loop even once other functions are compiled, which hands over at a call of
    /// one and every `WATCH` events (#480).
    #[inline(always)]
    fn tiered(&self, rt: &Runtime) -> bool {
        rt.native.any.load(Ordering::Acquire) && self.root_flagged(rt)
    }

    #[inline(always)]
    fn root_flagged(&self, rt: &Runtime) -> bool {
        self.frames.last().is_some_and(|fr| {
            rt.sync_native();
            rt.native
                .func
                .get(fr.func.0 as usize)
                .is_some_and(|n| n.load(Ordering::Acquire) & (NAT_CODE | NAT_ASK) != 0)
        })
    }

    /// The activation of a process whose root function has compiled code, entered where it
    /// suspended without `exec_fast`'s and `run_ops`' set-up (nothing returned by value
    /// but the outcome: a copy of a result the callee stored in pieces stalls). The first thing
    /// `InterpProcess::run` does for a process that is `hot`, so a compiled activation passes
    /// through one function frame of the interpreter's.
    #[inline(always)]
    pub(crate) fn run_direct(&mut self, ctx: &mut Option<&mut Ctx<'_>>) -> Direct {
        // Compiled code ready for the root function (one look at its flags, which the tier
        // changes when it swaps code); the activation is `run_quick`'s otherwise.
        if !self.rt.native.any.load(Ordering::Acquire) {
            return Direct::No;
        }
        let [fr] = &self.frames[..] else {
            return Direct::No;
        };
        let (func, pc) = (fr.func, fr.pc);
        let Some(flags) = self.rt.native.func.get(func.0 as usize) else {
            return Direct::No;
        };
        if flags.load(Ordering::Acquire) & (NAT_CODE | NAT_ASK) != NAT_CODE || self.pause.is_some()
        {
            return Direct::No;
        }
        let Some(rt) = self.rt_run.take() else {
            return Direct::No;
        };
        let Some(tier) = rt.tier.get() else {
            self.rt_run = Some(rt);
            return Direct::No;
        };
        let slot = crate::slot();
        rt.native.activated(func);
        let Some(f) = rt.prog.func_inline(func) else {
            self.rt_run = Some(rt);
            return Direct::No;
        };
        slot.store(
            u64::from(rt.id) << 32 | u64::from(func.0),
            Ordering::Relaxed,
        );
        let mut state = self.tier_state;
        let mut host = Host {
            m: self,
            rt: &rt,
            prog: &rt.prog,
            f,
            up: Vec::new(),
            ctx,
            fault: None,
            stop: false,
        };
        let ran = tier.run_cached(func, pc, &mut host, &mut state);
        // Tested in place: a move of the `Option<Fault>` copies it on every activation.
        if host.fault.is_some() {
            let e = host.fault.take();
            self.tier_state = state;
            self.rt_run = Some(rt);
            slot.store(0, Ordering::Relaxed);
            self.quick_fault = e;
            return Direct::Done(Quick::Fault);
        }
        // The rest of the host is references and the `None` fault: no drop call for them.
        if host.up.capacity() != 0 {
            drop(std::mem::take(&mut host.up));
        }
        std::mem::forget(host);
        self.tier_state = state;
        self.rt_run = Some(rt);
        match ran {
            Some(crate::TIER_SUSPEND) => {
                // A wait compiled code ended on the kernel itself left its pc in the record
                // (the tier writes `at` at every activation).
                match state.at {
                    crate::NO_PC => self.top().pc += 1,
                    at => self.top().pc = at + 1,
                }
                slot.store(0, Ordering::Relaxed);
                Direct::Done(Quick::Suspended)
            }
            Some(crate::TIER_UNWIND) | None => Direct::Continue,
            Some(p) => {
                self.top().pc = p;
                Direct::Continue
            }
        }
    }

    pub(crate) fn take_quick_fault(&mut self) -> Fault {
        self.quick_fault
            .take()
            .unwrap_or_else(|| ice_s!("a fault without a fault"))
    }

    /// GC of the private heap and the debug process table, after a suspension.
    #[inline(always)]
    pub(crate) fn after_suspend(&mut self) {
        // Not enough allocated since the last collection (a process that made its variables at
        // the start stays above zero for good), nothing deallocated: nothing to do.
        let quiet = self.heap.since_gc == 0
            || !heap::STRESS && self.heap.since_gc <= self.rt.limits.gc_threshold;
        if quiet && !self.heap.has_quarantine() && self.dbg.is_none() {
            return;
        }
        self.after_suspend_slow();
    }

    #[inline(never)]
    fn after_suspend_slow(&mut self) {
        if self.heap.since_gc > self.rt.limits.gc_threshold
            || heap::STRESS && self.heap.since_gc > 0
        {
            self.collect();
        } else if self.heap.has_quarantine()
            && let Some(r) = self.refs(false)
        {
            self.heap.recycle(&r.pinned(HEAP));
        }
        if let Some(d) = &self.dbg {
            let w = std::mem::take(&mut self.waits);
            d.session.update(d.index, |s| {
                s.state = ProcState::Waiting;
                s.sensitivity = w.sens;
                s.timeout = w.timeout;
                s.wait = w.stmt;
            });
        }
    }

    pub(crate) fn terminated(&mut self) {
        if let Some(d) = &self.dbg {
            d.session.update(d.index, |s| {
                s.state = ProcState::Terminated;
                s.sensitivity.clear();
                s.timeout = None;
            });
        }
    }

    /// The precise roots of this process (plan rule 6, #422): the frames' root maps, the
    /// temporaries recorded by `Inst::TempRoot` and, with `regs` (between instructions), the
    /// `Handle` and `Ptr` registers live where each frame is (`roots::live_refs`). Registers
    /// are dead at a suspension (ir.md 1.2). `None` when a root cannot be located.
    fn refs(&self, regs: bool) -> Option<heap::Refs> {
        let r = self.refs_in(regs);
        if heap::STRESS && r.is_none() && self.dbg.is_none() {
            panic!("gc-stress: the roots of a process cannot be located");
        }
        r
    }

    fn refs_in(&self, regs: bool) -> Option<heap::Refs> {
        let mut r = heap::Refs::default();
        self.refs_into(regs, &mut r).then_some(r)
    }

    /// [`Machine::refs_in`] into `r` (empty), which keeps its buffers: false when a root cannot be located.
    fn refs_into(&self, regs: bool, r: &mut heap::Refs) -> bool {
        self.refs_scan(regs, r).is_some()
    }

    fn refs_scan(&self, regs: bool, r: &mut heap::Refs) -> Option<()> {
        let prog = &self.rt.prog;
        for (k, fr) in self.frames.iter().enumerate() {
            let f = prog.func(fr.func)?;
            if !heap::slot_roots(prog, &f.roots, self.fslots(k)?, r) {
                return None;
            }
            if regs {
                let live = self.rt.live_refs(fr.func)?;
                for &reg in live.get(fr.pc)? {
                    let v = *self.regs.v.get(fr.rbase as usize + reg.0 as usize)?;
                    match f.reg_ty.get(reg.0 as usize)? {
                        yavs_ir::RegTy::Handle => r.handles.push(v),
                        yavs_ir::RegTy::Ptr => r.addrs.push(v),
                        _ if heap::STRESS && self.is_live_handle(v) => panic!(
                            "gc-stress: register {reg:?} of `{}` holds a handle at pc {} but is not a `Handle` register",
                            f.name, fr.pc
                        ),
                        _ => {}
                    }
                }
            }
        }
        for t in &self.troots {
            let data = self.temp.get(t.off as usize..)?;
            match t.ty {
                Some(ty) => {
                    if !heap::handles_in(prog, ty, &t.desc, data, &mut r.handles) {
                        return None;
                    }
                }
                None => {
                    let (a, d) = (data.get(..8)?, data.get(8..16)?);
                    r.addrs.push(u64::from_le_bytes(a.try_into().ok()?));
                    r.addrs.push(u64::from_le_bytes(d.try_into().ok()?));
                }
            }
        }
        Some(())
    }

    /// Whether `v` is the handle of a live object in this process's heaps (`gc-stress`).
    fn is_live_handle(&self, v: u64) -> bool {
        let shared = self.sh.as_ref().and_then(|sh| sh.heap.as_ref());
        self.heap.resolve(v).is_ok() || shared.is_some_and(|h| h.resolve(v).is_ok())
    }

    /// Collect the private heap from this suspended process's roots.
    pub(crate) fn collect(&mut self) -> bool {
        let Some(r) = self.refs(false) else {
            return false;
        };
        self.heap
            .collect(&self.rt.prog, &r.handles, &r.pinned(HEAP))
    }

    /// At a suspension or the end of this (serial) process: publish its shared-heap roots as
    /// entry `slot` of the shared state (assigned on the first call), for the collection at
    /// the next safepoint (`crate::SharedGc`), and make the shared heap's deallocated slots
    /// reusable that no serial process's frames still address.
    pub(crate) fn publish_shared(&mut self, slot: &mut Option<usize>) {
        // Checked first: a process without the shared state builds no roots (#356).
        if self.sh.is_none() {
            return;
        }
        #[cfg(test)]
        ROOT_BUILDS.with(|c| c.set(c.get() + 1));
        // The buffers of the last scan and of this process's entry are reused: a serial process
        // that wakes millions of times builds its roots at every one.
        let mut r = std::mem::take(&mut self.rscratch);
        r.handles.clear();
        r.addrs.clear();
        let ok = self.refs_into(false, &mut r);
        if heap::STRESS && !ok && self.dbg.is_none() {
            panic!("gc-stress: the roots of a process cannot be located");
        }
        let Some(sh) = self.sh.as_mut() else {
            self.rscratch = r;
            return;
        };
        let i = *slot.get_or_insert_with(|| {
            sh.procs.push(None);
            sh.procs.len() - 1
        });
        if ok {
            let mine = sh.procs[i].get_or_insert_with(ProcRoots::default);
            mine.handles.clear();
            mine.handles.extend(
                r.handles
                    .iter()
                    .copied()
                    .filter(|&h| heap::tag_of(h) == heap::SHARED),
            );
            r.pinned_into(SHARED_HEAP, &mut mine.pinned);
        } else {
            sh.procs[i] = None;
        }
        self.rscratch = r;
        if sh.heap.as_deref().is_some_and(Heap::has_quarantine)
            && let Some(pinned) = sh.pinned()
            && let Some(h) = sh.heap.as_mut()
        {
            h.recycle(&pinned);
        }
    }

    /// Recycle deallocated slots inside an activation once enough wait (#355), so a loop
    /// without a wait does not keep every slot it frees. Call only between instructions
    /// (before a native call, a `new` or a `deallocate`): every address the process holds is
    /// then in its frames' root maps, live registers or recorded temporaries. Other serial
    /// processes are suspended, so their published addresses are current for the shared heap.
    fn reclaim(&mut self, sim: bool) {
        // Code run for elaboration is neither collected nor recycled here: its temporaries
        // also come from Rust, which records none.
        if !sim {
            return;
        }
        self.gc_inline();
        let shared_due = self
            .sh
            .as_ref()
            .and_then(|sh| sh.heap.as_deref())
            .is_some_and(Heap::reclaim_due);
        if !(self.heap.reclaim_due() || shared_due) {
            return;
        }
        let Some(r) = self.refs(true) else {
            return;
        };
        if self.heap.reclaim_due() {
            self.heap.recycle(&r.pinned(HEAP));
        }
        if shared_due && let Some(sh) = self.sh.as_mut() {
            let Some(mut pinned) = sh.pinned() else {
                return;
            };
            pinned.extend(r.pinned(SHARED_HEAP));
            pinned.sort_unstable();
            pinned.dedup();
            if let Some(h) = sh.heap.as_mut() {
                h.recycle(&pinned);
            }
        }
    }

    /// Collect a heap that grew past its threshold inside an activation, so a process that
    /// never waits (a testbench main loop) does not keep every object it drops until the next
    /// delta cycle (memory grew without bound, #409; kept with precise roots, user,
    /// 2026-10-19, #422). Call only between instructions, as [`Machine::reclaim`]. Other
    /// serial processes are suspended, so what they published is current. Nothing is freed
    /// when a root cannot be located.
    fn gc_inline(&mut self) {
        if self.sh.as_ref().is_some_and(|sh| !sh.started) {
            return;
        }
        let t = self.rt.limits.gc_threshold;
        let private = self.heap.gc_due(t);
        let shared = self
            .sh
            .as_ref()
            .and_then(|sh| sh.heap.as_ref())
            .is_some_and(|h| h.gc_due(t));
        if !(private || shared) {
            return;
        }
        let Some(mut r) = self.refs(true) else {
            return;
        };
        if private {
            self.heap
                .collect(&self.rt.prog, &r.handles, &r.pinned(HEAP));
        }
        if shared {
            let mut pinned = r.pinned(SHARED_HEAP);
            let prog = self.rt.prog.clone();
            let Some(sh) = self.sh.as_mut() else { return };
            if !heap::slot_roots(&prog, &prog.shared_roots, &sh.vars, &mut r) {
                return;
            }
            for p in &sh.procs {
                match p {
                    Some(p) => r.handles.extend(&p.handles),
                    None => return,
                }
            }
            let Some(more) = sh.pinned() else { return };
            pinned.extend(more);
            pinned.sort_unstable();
            pinned.dedup();
            if let Some(h) = sh.heap.as_mut() {
                h.collect(&prog, &r.handles, &pinned);
            }
        }
    }

    #[inline(always)]
    fn top(&mut self) -> &mut Frame {
        let n = self.frames.len();
        &mut self.frames[n - 1]
    }

    #[inline]
    pub(crate) fn r(&self, r: Reg) -> u64 {
        self.regs[r.0 as usize]
    }

    #[inline]
    pub(crate) fn ri(&self, r: Reg) -> i64 {
        self.r(r) as i64
    }

    #[inline]
    fn rf(&self, r: Reg) -> f64 {
        f64::from_bits(self.r(r))
    }

    #[inline(always)]
    pub(crate) fn set(&mut self, r: Reg, v: u64) {
        self.regs[r.0 as usize] = v;
    }

    // ---- memory ----

    #[inline(always)]
    pub(crate) fn shared(&self) -> R<&SharedState> {
        self.sh
            .as_ref()
            .ok_or_else(|| ice_s!("shared state accessed by a process that is not marked serial"))
    }

    pub(crate) fn shared_mut(&mut self) -> R<&mut SharedState> {
        self.sh
            .as_mut()
            .ok_or_else(|| ice_s!("shared state accessed by a process that is not marked serial"))
    }

    /// Constant `idx` as bytes: stored as bytes, or once `obj` has expanded a compact one.
    #[inline(always)]
    fn const_mem(&self, idx: u32) -> Option<&[u8]> {
        match self.rt.prog.consts.get(yavs_ir::ConstId(idx)) {
            Some(ConstData::Bytes(b)) => Some(b),
            Some(_) => self
                .rt
                .expanded
                .get(idx as usize)
                .and_then(|c| c.get())
                .map(Vec::as_slice),
            None => None,
        }
    }

    /// The object of the spaces that are accessed all the time, without error paths.
    #[inline(always)]
    fn mem_fast(&self, sp: u8, idx: u32) -> Option<&[u8]> {
        match sp {
            FRAME => self.fslots(idx as usize),
            TEMP => Some(&self.temp),
            INST => Some(&self.inst.data),
            HEAP => self.heap.data(idx),
            // Constants stored as bytes; compact ones once `obj` has expanded them.
            CONST => self.const_mem(idx),
            // The state of protected objects and shared variables, and package data.
            SHARED_HEAP => self.sh.as_ref()?.heap.as_ref()?.data(idx),
            SHARED => Some(&self.sh.as_ref()?.vars),
            _ => None,
        }
    }

    /// [`Machine::mem_fast`] with temporary memory tested first: the operands and results of
    /// the native `ieee` subprograms live there (no jump table on the way).
    #[inline(always)]
    pub(crate) fn mem_temp_first(&self, sp: u8, idx: u32) -> Option<&[u8]> {
        if sp == TEMP {
            Some(&self.temp)
        } else {
            self.mem_fast(sp, idx)
        }
    }

    /// The shared heap object `idx` (`SHARED_HEAP`) or the shared variables (`SHARED`) for
    /// writing, without the error paths of `obj_mut`.
    #[inline(always)]
    fn shared_mem_mut(&mut self, sp: u8, idx: u32) -> Option<&mut [u8]> {
        let sh = self.sh.as_mut()?;
        if sp == SHARED {
            Some(&mut sh.vars)
        } else {
            sh.heap.as_mut()?.data_mut(idx)
        }
    }

    /// `rd` with the common spaces inline.
    #[inline(always)]
    pub(crate) fn rd_hot(&self, a: u64, n: u64) -> R<&[u8]> {
        let (sp, idx, off) = split(a);
        match self
            .mem_fast(sp, idx)
            .and_then(|m| m.get(off as usize..off.saturating_add(n) as usize))
        {
            Some(b) => Ok(b),
            None => self.rd_slow(a, n),
        }
    }

    /// The `n` bytes at `a`: the common spaces inline, everything else (and every error)
    /// through `rd_slow`.
    #[inline(always)]
    pub(crate) fn rd(&self, a: u64, n: u64) -> R<&[u8]> {
        self.rd_hot(a, n)
    }

    #[inline(never)]
    fn rd_slow(&self, a: u64, n: u64) -> R<&[u8]> {
        let (sp, idx, off) = split(a);
        let mem = self.obj(sp, idx)?;
        let end = off.saturating_add(n);
        mem.get(off as usize..end as usize).ok_or_else(|| {
            ice_f!(
                "read of {n} bytes at {off} outside a {} object of {} bytes",
                SPACE_NAMES[sp as usize],
                mem.len()
            )
        })
    }

    /// The memory object of space `sp` and index `idx`, for reading.
    fn obj(&self, sp: u8, idx: u32) -> R<&[u8]> {
        Ok(match sp {
            FRAME => match &self.nwin {
                Some(w) if (self.nwin_from..=self.nwin_to).contains(&(idx as usize)) => {
                    w.slots(idx as usize)
                }
                _ => self.fslots(idx as usize),
            }
            .ok_or_else(|| ice_f!("address of dead frame {idx}"))?,
            INST => &self.inst.data,
            PKG => match &self.pkg_init {
                Some((p, data)) if *p == idx => data,
                _ => {
                    &self
                        .rt
                        .pkgs
                        .get(idx as usize)
                        .ok_or_else(|| ice_f!("no package instance {idx}"))?
                        .data
                }
            },
            CONST => const_bytes(&self.rt, yavs_ir::ConstId(idx))?,
            HEAP => self
                .heap
                .data(idx)
                .ok_or_else(|| runtime_s!("access to a deallocated object"))?,
            TEMP => &self.temp,
            SHARED => &self.shared()?.vars,
            _ => self
                .shared()?
                .heap
                .as_ref()
                .and_then(|h| h.data(idx))
                .ok_or_else(|| runtime_s!("access to a deallocated object"))?,
        })
    }

    pub(crate) fn wr(&mut self, a: u64, n: u64) -> R<&mut [u8]> {
        let (sp, idx, off) = split(a);
        let mem = self.obj_mut(sp, idx)?;
        let len = mem.len();
        let end = off.saturating_add(n);
        mem.get_mut(off as usize..end as usize).ok_or_else(|| {
            ice_f!(
                "write of {n} bytes at {off} outside a {} object of {len} bytes",
                SPACE_NAMES[sp as usize]
            )
        })
    }

    /// The memory object of space `sp` and index `idx`, for writing.
    fn obj_mut(&mut self, sp: u8, idx: u32) -> R<&mut [u8]> {
        Ok(match sp {
            FRAME => match &mut self.nwin {
                Some(w) if (self.nwin_from..=self.nwin_to).contains(&(idx as usize)) => {
                    w.slots_mut(idx as usize)
                }
                _ => slots_of_mut(&self.frames, &mut self.root, &mut self.sstack, idx as usize),
            }
            .ok_or_else(|| ice_f!("address of dead frame {idx}"))?,
            HEAP => self
                .heap
                .data_mut(idx)
                .ok_or_else(|| runtime_s!("access to a deallocated object"))?,
            TEMP => &mut self.temp,
            PKG => match &mut self.pkg_init {
                Some((p, data)) if *p == idx => data,
                _ => {
                    return Err(ice_f!("write to package {idx} outside its initialisation"));
                }
            },
            SHARED => &mut self.shared_mut()?.vars,
            SHARED_HEAP => self
                .shared_mut()?
                .heap
                .as_mut()
                .and_then(|h| h.data_mut(idx))
                .ok_or_else(|| runtime_s!("access to a deallocated object"))?,
            _ => {
                return Err(ice_f!(
                    "write to read-only {} memory",
                    SPACE_NAMES[sp as usize]
                ));
            }
        })
    }

    /// The [`crate::Stamp`] of the heap object at address `a`; `None` for other memory.
    pub(crate) fn stamp(&self, a: u64) -> Option<crate::Stamp> {
        STAMPS.store(true, Ordering::Relaxed);
        let (sp, idx, _) = split(a);
        let (generation, writes) = match sp {
            HEAP => self.heap.stamp(idx)?,
            SHARED_HEAP => self.sh.as_ref()?.heap.as_ref()?.stamp(idx)?,
            _ => return None,
        };
        Some(crate::Stamp {
            addr: a,
            generation,
            writes,
        })
    }

    #[inline]
    pub(crate) fn put(&mut self, a: u64, bytes: &[u8]) -> R<()> {
        let (sp, idx, off) = split(a);
        let n = bytes.len();
        let mem = match sp {
            FRAME => self.fslots_mut(idx as usize),
            TEMP => Some(&mut self.temp[..]),
            HEAP => self.heap.data_mut(idx),
            SHARED_HEAP | SHARED => self.shared_mem_mut(sp, idx),
            _ => None,
        };
        if let Some(b) = mem.and_then(|m| m.get_mut(off as usize..off as usize + n)) {
            b.copy_from_slice(bytes);
            return Ok(());
        }
        self.wr(a, n as u64)?.copy_from_slice(bytes);
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn load(&self, a: u64, w: Width) -> R<u64> {
        let n = w.bytes().min(8) as u64;
        // Fast path: a frame (registers spill to frame slots all the time), temporary memory,
        // an object of either heap (the state of protected objects is in the shared one).
        let (sp, idx, off) = split(a);
        let mem = match sp {
            FRAME => self.fslots(idx as usize),
            TEMP => Some(&self.temp[..]),
            HEAP => self.heap.data(idx),
            SHARED_HEAP => self.sh.as_ref().and_then(|s| s.heap.as_ref()?.data(idx)),
            _ => None,
        };
        if let Some(b) = mem.and_then(|m| m.get(off as usize..(off + n) as usize)) {
            return Ok(cell_w(b, w));
        }
        self.load_slow(a, w)
    }

    /// `load` from the instance data, package data, byte constants and shared variables
    /// (dope pointers of unconstrained parameters point there): each checked in place, everything
    /// else and every error in `load_cold`. Short on purpose: no error building, so no saved
    /// registers (#674).
    #[inline(never)]
    fn load_slow(&self, a: u64, w: Width) -> R<u64> {
        let (sp, idx, off) = split(a);
        crate::census::slow_load(sp);
        let mem = match sp {
            INST => Some(&self.inst.data[..]),
            PKG => match &self.pkg_init {
                Some((p, data)) if *p == idx => Some(&data[..]),
                _ => self.rt.pkgs.get(idx as usize).map(|p| &p.data[..]),
            },
            CONST => self.const_mem(idx),
            SHARED => self.sh.as_ref().map(|s| &s.vars[..]),
            _ => None,
        };
        let n = w.bytes().min(8) as usize;
        match mem.and_then(|m| m.get(off as usize..off as usize + n)) {
            Some(b) => Ok(cell_w(b, w)),
            None => self.load_cold(a, w),
        }
    }

    /// `load_slow` for the remaining spaces and every error.
    #[inline(never)]
    fn load_cold(&self, a: u64, w: Width) -> R<u64> {
        let n = w.bytes().min(8) as u64;
        let (sp, idx, off) = split(a);
        // Package data is not in `mem_fast`: read here.
        let mem = if sp == PKG {
            self.obj(sp, idx).ok()
        } else {
            self.mem_fast(sp, idx)
        };
        if let Some(b) = mem.and_then(|m| m.get(off as usize..(off + n) as usize)) {
            return Ok(cell_w(b, w));
        }
        Ok(cell_w(self.rd(a, n)?, w))
    }

    /// `store`, out of line (the rare paths of the slot stores).
    #[inline(never)]
    fn store_slow(&mut self, a: u64, w: Width, v: u64) -> R<()> {
        self.store(a, w, v)
    }

    /// `load` of the element `pos` of `stride` bytes from `base` (`index_addr` and `load`), the
    /// common spaces without making the address.
    #[inline(always)]
    pub(crate) fn load_idx(&self, base: u64, pos: u64, stride: u32, w: Width) -> R<u64> {
        let (sp, idx, off) = split(base);
        let n = u64::from(w.bytes().min(8));
        if let Some(o) = pos
            .checked_mul(u64::from(stride))
            .and_then(|d| d.checked_add(off))
            .filter(|&o| o <= u64::from(u32::MAX))
            && let Some(b) = self
                .mem_fast(sp, idx)
                .and_then(|m| m.get(o as usize..(o + n) as usize))
        {
            return Ok(cell_w(b, w));
        }
        self.load(index_addr(base, pos, stride)?, w)
    }

    /// `store` to the element `pos` of `stride` bytes from `base`.
    #[inline(always)]
    pub(crate) fn store_idx(
        &mut self,
        base: u64,
        pos: u64,
        stride: u32,
        w: Width,
        v: u64,
    ) -> R<()> {
        let (sp, idx, off) = split(base);
        let n = w.bytes().min(8) as usize;
        if let Some(o) = pos
            .checked_mul(u64::from(stride))
            .and_then(|d| d.checked_add(off))
            .filter(|&o| o <= u64::from(u32::MAX))
        {
            let mem = match sp {
                FRAME => self.fslots_mut(idx as usize),
                HEAP => self.heap.data_mut(idx),
                TEMP => Some(&mut self.temp[..]),
                SHARED_HEAP | SHARED => self.shared_mem_mut(sp, idx),
                _ => None,
            };
            if let Some(b) = mem.and_then(|s| s.get_mut(o as usize..o as usize + n)) {
                put_le(b, v);
                return Ok(());
            }
        }
        self.store(index_addr(base, pos, stride)?, w, v)
    }

    /// Bounds and direction word of dimension `dim` of the descriptor at `base`.
    #[inline(always)]
    pub(crate) fn dim_of(&self, base: u64, dim: u64) -> R<(i64, i64, u64)> {
        let (sp, idx, off) = split(base);
        if let Some([w0, w1, w2]) = self
            .mem_fast(sp, idx)
            .and_then(|m| m.get((off as usize).checked_add((dim as usize).checked_mul(24)?)?..))
            .and_then(|s| s.first_chunk::<24>())
            .map(|b| b.as_chunks::<8>().0)
        {
            let (l, r, d) = (
                u64::from_le_bytes(*w0),
                u64::from_le_bytes(*w1),
                u64::from_le_bytes(*w2),
            );
            return Ok((l as i64, r as i64, d));
        }
        let d = add_addr(base, dim * 24)?;
        let b = self.rd_hot(d, 24)?;
        let word = |k: usize| {
            b.get(k * 8..k * 8 + 8)
                .map(le_bytes)
                .ok_or_else(|| ice_s!("short descriptor"))
        };
        Ok((word(0)? as i64, word(1)? as i64, word(2)?))
    }

    #[inline]
    pub(crate) fn store(&mut self, a: u64, w: Width, v: u64) -> R<()> {
        let n = w.bytes().min(8) as usize;
        let (sp, idx, off) = split(a);
        let mem = match sp {
            FRAME => self.fslots_mut(idx as usize),
            HEAP => self.heap.data_mut(idx),
            SHARED_HEAP | SHARED => self.shared_mem_mut(sp, idx),
            _ => None,
        };
        if let Some(b) = mem.and_then(|s| s.get_mut(off as usize..off as usize + n)) {
            put_le(b, v);
            return Ok(());
        }
        let mut x = [0u8; 8];
        put_le(&mut x[..n], v);
        self.put(a, &x[..n])
    }

    /// `Inst::TempRoot` of the fat pair `[data, desc]` at `a`: recorded only when a word
    /// points into a heap (most point into temporaries or constants).
    #[inline(always)]
    fn fat_root(&mut self, a: u64) -> R<()> {
        let (sp, _, off) = split(a);
        let w = self
            .temp
            .get(off as usize..off as usize + 16)
            .filter(|_| sp == TEMP)
            .ok_or_else(|| ice_s!("a recorded fat pair outside the temporary area"))?;
        let heap_at = |k: usize| {
            let mut b = [0; 8];
            b.copy_from_slice(&w[k..k + 8]);
            matches!(split(u64::from_le_bytes(b)).0, HEAP | SHARED_HEAP)
        };
        if heap_at(0) || heap_at(8) {
            self.temp_root(a, None, Box::default());
        }
        Ok(())
    }

    /// Record the temporary at `a` for the GC until its statement ends (`Inst::TempRoot`).
    pub(crate) fn temp_root(&mut self, a: u64, ty: Option<yavs_ir::TyId>, desc: Box<[u8]>) {
        let (_, _, off) = split(a);
        self.troots.push(TempRoot {
            off: off as u32,
            ty,
            desc,
        });
    }

    #[inline(always)]
    pub(crate) fn temp_alloc(&mut self, n: u64) -> R<u64> {
        let off = (self.temp.len() as u64).next_multiple_of(8);
        let end = off.saturating_add(n);
        if end > u64::from(u32::MAX) {
            return rt_err_s!("temporary memory of one statement exceeds 4 GiB");
        }
        let end = end as usize;
        if end - self.temp.len() <= 32 {
            // A fixed-size store, not a `memset` call for a handful of bytes.
            self.temp.extend_from_slice(&[0; 32]);
            self.temp.truncate(end);
        } else {
            self.temp.resize(end, 0);
        }
        mk_addr(TEMP, 0, off)
    }

    /// The words `f(addr)` in temporary memory, where `addr` is the address of the first one.
    #[inline(always)]
    pub(crate) fn temp_words<const N: usize>(&mut self, f: impl FnOnce(u64) -> [u64; N]) -> R<u64> {
        let off = (self.temp.len() as u64).next_multiple_of(8);
        if off + 8 * N as u64 > u64::from(u32::MAX) {
            return rt_err_s!("temporary memory of one statement exceeds 4 GiB");
        }
        // `off` is below 2^32 (checked above): the address is the space and the offset.
        let addr = u64::from(TEMP) << 61 | off;
        // Mostly a multiple of 8 already: no call to `resize`.
        if self.temp.len() as u64 != off {
            self.temp.resize(off as usize, 0);
        }
        self.temp
            .extend_from_slice(f(addr).map(u64::to_le_bytes).as_flattened());
        Ok(addr)
    }

    /// A fat pair `[a, b]` in temporary memory.
    #[inline(always)]
    fn new_pair(&mut self, a: u64, b: u64) -> R<u64> {
        let off = (self.temp.len() as u64).next_multiple_of(8);
        if off + 16 > u64::from(u32::MAX) {
            return rt_err_s!("temporary memory of one statement exceeds 4 GiB");
        }
        self.temp.resize(off as usize, 0);
        self.temp.extend_from_slice(&a.to_le_bytes());
        self.temp.extend_from_slice(&b.to_le_bytes());
        mk_addr(TEMP, 0, off)
    }

    /// A string result: the fat pair `[data, dope]`, the `1 to len` descriptor and the bytes in
    /// one block of temporary memory (`string_parts` reads it so).
    pub(crate) fn ret_string(&mut self, s: &[u8]) -> R<u64> {
        let fat = self.temp_alloc(40 + s.len() as u64)?;
        let (dope, data) = (add_addr(fat, 16)?, add_addr(fat, 40)?);
        let b = self.wr(fat, 40 + s.len() as u64)?;
        b[0..8].copy_from_slice(&data.to_le_bytes());
        b[8..16].copy_from_slice(&dope.to_le_bytes());
        b[16..24].copy_from_slice(&1i64.to_le_bytes());
        b[24..32].copy_from_slice(&(s.len() as i64).to_le_bytes());
        b[32..40].copy_from_slice(&0i64.to_le_bytes());
        b[40..].copy_from_slice(s);
        Ok(fat)
    }

    pub(crate) fn planes(&self, a: u64, n: u64) -> R<LogicVec> {
        let b = self.rd(a, n.div_ceil(64) * 32)?;
        Ok(LogicVec::from_repr(Planes::from_blocks(
            n as usize,
            blocks_of(b),
        )))
    }

    pub(crate) fn put_planes(&mut self, a: u64, v: &LogicVec) -> R<()> {
        self.put(a, &bytes_of(v.repr().blocks()))
    }

    fn put_value(&mut self, a: u64, v: &Value) -> R<()> {
        match v {
            Value::Bytes(b) => self.put(a, b),
            Value::Logic(l) => self.put_planes(a, l),
        }
    }

    fn get_value(&self, a: u64, elem: Elem, len: u32) -> R<Value> {
        Ok(match elem {
            Elem::Bytes(_) => Value::Bytes(self.rd(a, value_size(elem, len))?.to_vec()),
            Elem::Logic => Value::Logic(self.planes(a, u64::from(len))?),
        })
    }

    /// Whether the byte ranges `dlen` at `dst + dat` and `slen` at `src + sat` share no byte
    /// (different objects, or one object and ranges that do not meet).
    fn disjoint(&self, dst: u64, dat: u64, dlen: u64, src: u64, sat: u64, slen: u64) -> bool {
        let ((dsp, didx, doff), (ssp, sidx, soff)) = (split(dst), split(src));
        if (dsp, didx) != (ssp, sidx) {
            return true;
        }
        let (d, s) = (doff + dat, soff + sat);
        d + dlen <= s || s + slen <= d
    }

    /// `lcopy` of a wide range (three or more blocks on a side) whose two sides are in
    /// objects that share no byte: a partial block at each end, the whole destination blocks
    /// between them as one move (a `memcpy` when both sides start at the same bit of a block,
    /// else a shift-merge of the source words). `false`: nothing was copied (memory the
    /// machine does not hold in a field of its own).
    #[inline(never)]
    fn lcopy_wide(&mut self, dst: u64, doff: u64, src: u64, soff: u64, n: u64) -> R<bool> {
        let head = (64 - doff % 64) % 64;
        let (dpos, spos) = (doff + head, soff + head);
        let whole = (n - head) / 64;
        let (dm, sm) = (
            add_addr(dst, dpos / 64 * 32)?,
            add_addr(src, spos / 64 * 32)?,
        );
        let sh = spos % 64;
        let (dn, sn) = (
            whole as usize * 32,
            (whole as usize + usize::from(sh != 0)) * 32,
        );
        if !self.copy_pair(dm, sm, dn, sn, |out, src| {
            if sh == 0 {
                out.copy_from_slice(&src[..dn]);
            } else {
                shift_merge(out, src, sh);
            }
        }) {
            return Ok(false);
        }
        if head > 0 {
            self.lcopy(dst, doff, src, soff, head)?;
        }
        let done = head + whole * 64;
        self.lcopy(dst, doff + done, src, soff + done, n - done)?;
        Ok(true)
    }

    /// `lcopy` of elements inside one block on each side (a field or a slice of a vector, the
    /// usual copy): the source block into registers, the destination block changed where it
    /// is, no staging and no call. `false`: nothing was copied (another shape, memory the
    /// machine does not hold in a field, an address that does not resolve: `lcopy_general`
    /// copies or reports).
    #[inline(always)]
    fn lcopy1(&mut self, dst: u64, doff: u64, src: u64, soff: u64, n: u64) -> bool {
        let (s0, d0) = (soff % 64, doff % 64);
        if n == 0 || n > 64 - s0 || n > 64 - d0 {
            return false;
        }
        let (ssp, sidx, so) = split(src);
        let so = (so + soff / 64 * 32) as usize;
        let Some(sw) = self
            .mem_fast(ssp, sidx)
            .and_then(|m| m.get(so..so + 32))
            .map(plane_words)
        else {
            return false;
        };
        let (dsp, didx, dof) = split(dst);
        let dof = (dof + doff / 64 * 32) as usize;
        let mem = match dsp {
            FRAME => self.fslots_mut(didx as usize),
            TEMP => Some(&mut self.temp[..]),
            HEAP => self.heap.data_mut(didx),
            SHARED_HEAP => self
                .sh
                .as_mut()
                .and_then(|s| s.heap.as_mut()?.data_mut(didx)),
            _ => None,
        };
        let Some(db) = mem.and_then(|m| m.get_mut(dof..dof + 32)) else {
            return false;
        };
        let mask = yavs_logic::wide::nmask(n as usize);
        for (c, s) in db.as_chunks_mut::<8>().0.iter_mut().zip(sw) {
            let d = u64::from_le_bytes(*c);
            *c = (d & !(mask << d0) | (s >> s0 & mask) << d0).to_le_bytes();
        }
        true
    }

    /// `Inst::LCopy`: `n` elements from element `soff` of the logic vector at `src` to `doff` of
    /// the one at `dst`.
    #[inline(never)]
    fn lcopy(&mut self, dst: u64, doff: u64, src: u64, soff: u64, n: u64) -> R<()> {
        if self.lcopy1(dst, doff, src, soff, n) {
            return Ok(());
        }
        self.lcopy_general(dst, doff, src, soff, n)
    }

    /// `lcopy` of every other shape.
    #[inline(never)]
    fn lcopy_general(&mut self, dst: u64, doff: u64, src: u64, soff: u64, n: u64) -> R<()> {
        if n == 0 {
            return Ok(());
        }
        // The usual copy: at most 64 elements, one block on each side. Found and shifted in
        // place; anything unusual (another space, a bad address) goes on to the general code.
        let (s0, d0) = (soff & 63, doff & 63);
        if n <= 64 - s0 && n <= 64 - d0 {
            let (ssp, sidx, sbase) = split(src);
            let (dsp, didx, dbase) = split(dst);
            if let Some(so) = sbase
                .checked_add((soff >> 6) * 32)
                .filter(|&o| o <= u64::from(u32::MAX))
                && let Some(dof) = dbase
                    .checked_add((doff >> 6) * 32)
                    .filter(|&o| o <= u64::from(u32::MAX))
                && let Some(sb) = self
                    .mem_fast(ssp, sidx)
                    .and_then(|m| m.get(so as usize..))
                    .and_then(|m| m.first_chunk::<32>())
            {
                let sw: [u64; 4] = std::array::from_fn(|p| plane_word(sb, p));
                let mem = match dsp {
                    FRAME => self.fslots_mut(didx as usize),
                    TEMP => Some(&mut self.temp[..]),
                    HEAP => self.heap.data_mut(didx),
                    _ => None,
                };
                if let Some(db) = mem
                    .and_then(|m| m.get_mut(dof as usize..))
                    .and_then(|m| m.first_chunk_mut::<32>())
                {
                    let mask = if n == 64 { !0 } else { (1u64 << n) - 1 };
                    for (p, c) in db.as_chunks_mut::<8>().0.iter_mut().enumerate() {
                        let d = u64::from_le_bytes(*c);
                        *c = (d & !(mask << d0) | ((sw[p] >> s0) & mask) << d0).to_le_bytes();
                    }
                    return Ok(());
                }
            }
        }
        let last = |off: u64| {
            off.checked_add(n - 1)
                .ok_or_else(|| ice_f!("logic copy of {n} elements at {off}"))
        };
        let (sb0, sb1) = (soff / 64, last(soff)? / 64);
        let (db0, db1) = (doff / 64, last(doff)? / 64);
        let (sn, dn) = ((sb1 - sb0 + 1) as usize, (db1 - db0 + 1) as usize);
        let da = add_addr(dst, db0 * 32)?;
        let (s0, d0) = (soff - sb0 * 64, doff - db0 * 64);
        // Up to 128 elements on either side: the blocks on the stack.
        if sn <= 2 && dn <= 2 {
            let sa = add_addr(src, sb0 * 32)?;
            let mut sw = [[0u64; 4]; 2];
            let mut dw = [[0u64; 4]; 2];
            // Both in the temporary area (the vectors a statement builds): straight in it.
            let (sp, dp) = (split(sa), split(da));
            if sp.0 == TEMP && dp.0 == TEMP {
                let (so, dof) = (sp.2 as usize, dp.2 as usize);
                let t = &mut self.temp;
                if let (Some(sb), Some(db)) = (t.get(so..so + sn * 32), t.get(dof..dof + dn * 32)) {
                    fill_blocks(sb, &mut sw[..sn]);
                    fill_blocks(db, &mut dw[..dn]);
                    copy_bits(&sw[..sn], &mut dw[..dn], s0, d0, n);
                    let out = &mut t[dof..dof + dn * 32];
                    for (c, w) in out.as_chunks_mut::<8>().0.iter_mut().zip(dw.as_flattened()) {
                        *c = w.to_le_bytes();
                    }
                    return Ok(());
                }
            }
            fill_blocks(self.rd(sa, sn as u64 * 32)?, &mut sw[..sn]);
            fill_blocks(self.rd(da, dn as u64 * 32)?, &mut dw[..dn]);
            copy_bits(&sw[..sn], &mut dw[..dn], s0, d0, n);
            let mut out = [0u8; 64];
            for (c, w) in out.as_chunks_mut::<8>().0.iter_mut().zip(dw.as_flattened()) {
                *c = w.to_le_bytes();
            }
            return self.put(da, &out[..dn * 32]);
        }
        if self.disjoint(dst, db0 * 32, dn as u64 * 32, src, sb0 * 32, sn as u64 * 32)
            && self.lcopy_wide(dst, doff, src, soff, n)?
        {
            return Ok(());
        }
        let sw = blocks_of(self.rd(add_addr(src, sb0 * 32)?, sn as u64 * 32)?);
        let mut dw = blocks_of(self.rd(da, dn as u64 * 32)?);
        copy_bits(&sw, &mut dw, s0, d0, n);
        let out = self.wr(da, dn as u64 * 32)?;
        for (c, w) in out.as_chunks_mut::<8>().0.iter_mut().zip(dw.as_flattened()) {
            *c = w.to_le_bytes();
        }
        Ok(())
    }

    /// `Inst::SigRead` of a reference of one range: the signal's value into the memory at `dst`.
    #[inline(always)]
    fn sig_read_vec(&mut self, dst: Reg, sig: Reg, ctx: &mut Option<&mut Ctx<'_>>) -> R<()> {
        let (sr, ty) = self.sigref(sig)?;
        let Some(c) = ctx.as_deref_mut() else {
            return rt_err_s!("signal or wait operation during elaboration");
        };
        let n = value_size(ty.elem, sr.range.map_or(0, |r| r.len));
        c.read_into(sr, self.wr(self.r(dst), n)?)?;
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn delay_mode(&self, mode: DriveMode, reject: Option<Reg>) -> R<DelayMode> {
        Ok(match mode {
            DriveMode::Transport => DelayMode::Transport,
            DriveMode::Inertial => DelayMode::Inertial {
                reject: match reject {
                    Some(r) => Some(to_time(self.ri(r))?),
                    None => None,
                },
            },
        })
    }

    /// `Drive` and `DriveSlot`: the waveform at `wave` onto driver `d`.
    #[allow(clippy::too_many_arguments)]
    #[inline(never)]
    fn drive(
        &mut self,
        ctx: &mut Option<&mut Ctx<'_>>,
        d: ProcDriver,
        target: Option<Reg>,
        wave: Reg,
        n: Reg,
        mode: DriveMode,
        reject: Option<Reg>,
    ) -> R<()> {
        let (w, n) = (self.r(wave), self.r(n));
        self.drive_at(ctx, d, target, w, n, mode, reject)
    }

    /// `drive` with the waveform's address and element count.
    #[allow(clippy::too_many_arguments)]
    fn drive_at(
        &mut self,
        ctx: &mut Option<&mut Ctx<'_>>,
        d: ProcDriver,
        target: Option<Reg>,
        w: u64,
        n: u64,
        mode: DriveMode,
        reject: Option<Reg>,
    ) -> R<()> {
        let (sr, elem) = match target {
            Some(t) => {
                let (sr, ty) = self.sigref(t)?;
                (sr, ty.elem)
            }
            None => self.driver_ref(&d)?,
        };
        let len = sr.range.map_or(0, |r| r.len);
        let vsize = value_size(elem, len);
        let stride = 8 + vsize.next_multiple_of(8);
        // One scalar logic element: no value, no lists (a scalar assignment per activation).
        if elem == Elem::Logic && len == 1 && n == 1 {
            let t = to_time(self.load(w, Width::B8)? as i64)?;
            let b = self.rd(add_addr(w, 8)?, 32)?;
            let code = (0..4).fold(0u8, |c, p| c | (b[p * 8] & 1) << p);
            let mode = self.delay_mode(mode, reject)?;
            let range = sr.range;
            let Some(c) = ctx.as_deref_mut() else {
                return rt_err_s!("signal or wait operation during elaboration");
            };
            c.assign_code(d.id, range, t, code, mode)?;
            return Ok(());
        }
        // One waveform element (nearly every assignment): no vectors.
        if n == 1 {
            let t = to_time(self.load(w, Width::B8)? as i64)?;
            let mode = self.delay_mode(mode, reject)?;
            let range = sr.range;
            if elem == Elem::Logic {
                // The plane blocks go to the kernel as they are in memory: no value is built.
                let b = self.rd(add_addr(w, 8)?, u64::from(len).div_ceil(64) * 32)?;
                let Some(c) = ctx.as_deref_mut() else {
                    return rt_err_s!("signal or wait operation during elaboration");
                };
                c.assign_blocks(d.id, range, t, b, mode)?;
                return Ok(());
            }
            if let Elem::Bytes(es) = elem
                && es <= 8
                && len == 1
            {
                // One scalar of at most 8 bytes: the bytes go to the kernel as they are.
                let b = self.rd(add_addr(w, 8)?, u64::from(es))?;
                let Some(c) = ctx.as_deref_mut() else {
                    return rt_err_s!("signal or wait operation during elaboration");
                };
                c.assign_bytes(d.id, range, t, b, mode)?;
                return Ok(());
            }
            let v = self.get_value(add_addr(w, 8)?, elem, len)?;
            let Some(c) = ctx.as_deref_mut() else {
                return rt_err_s!("signal or wait operation during elaboration");
            };
            c.assign(d.id, range, &[(t, v.as_ref())], mode)?;
            return Ok(());
        }
        let mut items = Vec::new();
        for k in 0..n {
            let at = add_addr(w, k * stride)?;
            let t = to_time(self.load(at, Width::B8)? as i64)?;
            items.push((t, self.get_value(add_addr(at, 8)?, elem, len)?));
        }
        let refs: Vec<(Time, ValueRef<'_>)> = items.iter().map(|(t, v)| (*t, v.as_ref())).collect();
        let mode = self.delay_mode(mode, reject)?;
        let range = sr.range;
        let Some(c) = ctx.as_deref_mut() else {
            return rt_err_s!("signal or wait operation during elaboration");
        };
        c.assign(d.id, range, &refs, mode)?;
        Ok(())
    }

    /// `LSet` of element 0 and `Drive` of the one-element waveform on a signal reference of one
    /// scalar logic element, given the code: the assignment without the waveform's
    /// planes. `false`: not that case, nothing was done.
    fn drive_bit(
        &mut self,
        ctx: &mut Option<&mut Ctx<'_>>,
        d: ProcDriver,
        target: Reg,
        wave: Reg,
        code: u8,
        mode: DriveMode,
    ) -> R<bool> {
        let time = self.load(self.r(wave), Width::B8)?;
        self.drive_bit_at(ctx, d, target, time, code, mode)
    }

    /// The usual one-element logic assignment with zero delay, straight to the kernel's fast case
    /// (`Ctx::assign_code_now`, which takes inertial and transport alike at zero delay); false:
    /// nothing was done and `drive_bit_at` runs.
    #[inline(always)]
    fn drive_code_now(
        &mut self,
        ctx: &mut Option<&mut Ctx<'_>>,
        driver: u32,
        target: Reg,
        code: u8,
    ) -> bool {
        let (v, len) = (self.r(target), self.r(Reg(target.0 + 1)));
        if len != 1 || v >> 32 == MULTI {
            return false;
        }
        let (Some(d), Some(s), Some(c)) = (
            self.drivers.get(driver as usize),
            self.inst.sigs.get((v >> 32) as usize),
            ctx.as_deref_mut(),
        ) else {
            return false;
        };
        s.ty.elem == Elem::Logic
            && c.assign_code_now(d.id, yavs_kernel::Range::new(s.off + v as u32, 1), code)
    }

    /// The drives of `run`, from the front, that change nothing: each is skipped as `LSetDrive`
    /// would skip it, with the registers its ops write set. Returns how many; the next one, or
    /// the whole run when the count is short or the process has no kernel, is left to its ops.
    #[inline(never)]
    fn drive_run(
        &mut self,
        run: &pre::DriveRun,
        top: usize,
        ctx: &mut Option<&mut Ctx<'_>>,
    ) -> usize {
        let (Some(c), Some(slots)) = (
            ctx.as_deref_mut(),
            slots_of(&self.frames, &self.root, &self.sstack, top),
        ) else {
            return 0;
        };
        let (drivers, sigs, regs) = (&self.drivers, &self.inst.sigs, &mut self.regs);
        for (j, d) in run.drives.iter().enumerate() {
            let (o, l) = (regs[d.offset as usize], regs[d.len as usize]);
            let (Ok(o), 1) = (u32::try_from(o), l) else {
                return j;
            };
            let (Some(dr), Some(s)) = (drivers.get(d.driver as usize), sigs.get(d.sig as usize))
            else {
                return j;
            };
            if s.ty.elem != Elem::Logic || regs[d.time as usize] != 0 {
                return j;
            }
            let (at, n) = (d.off as usize, d.w.bytes() as usize);
            let Some(b) = slots.get(at..at + n) else {
                return j;
            };
            let x = cell_w(b, d.w);
            let code = match d.aff {
                Some((_, ca, c0)) if aff_ok(x as i64) => {
                    (i64::from(c0) + i64::from(ca) * x as i64) as u64
                }
                Some(_) => return j,
                None => x,
            };
            if code > 8
                || !c.skip_code_now(dr.id, yavs_kernel::Range::new(s.off + o, 1), code as u8)
            {
                return j;
            }
            regs[d.sv as usize] = u64::from(d.sig) << 32 | u64::from(o);
            regs[d.sv as usize + 1] = 1;
            regs[d.ld as usize] = x;
            if let Some((dst, ..)) = d.aff {
                regs[dst as usize] = code;
            }
        }
        run.drives.len()
    }

    /// The `SigVal` of the first drive of a run that a `DriveRun` stands in place of.
    #[inline(never)]
    fn drive_run_head(&mut self, run: &pre::DriveRun) -> R<()> {
        let Some(d) = run.drives.first() else {
            return Err(ice_s!("a drive run without drives"));
        };
        let (o, l) = (self.regs[d.offset as usize], self.regs[d.len as usize]);
        let (a, b) = self.sig_val(d.sig, o as i64, l as i64)?;
        self.regs[d.sv as usize] = a;
        self.regs[d.sv as usize + 1] = b;
        Ok(())
    }

    /// `drive_bit` with the delay itself instead of the waveform.
    #[inline(never)]
    fn drive_bit_at(
        &mut self,
        ctx: &mut Option<&mut Ctx<'_>>,
        d: ProcDriver,
        target: Reg,
        time: u64,
        code: u8,
        mode: DriveMode,
    ) -> R<bool> {
        let (sr, ty) = self.sigref(target)?;
        if ty.elem != Elem::Logic || sr.range.map_or(0, |r| r.len) != 1 {
            return Ok(false);
        }
        let t = to_time(time as i64)?;
        let mode = self.delay_mode(mode, None)?;
        let Some(c) = ctx.as_deref_mut() else {
            return rt_err_s!("signal or wait operation during elaboration");
        };
        c.assign_code(d.id, sr.range, t, code, mode)?;
        Ok(true)
    }

    /// The head of the waveform `LSetDrive` dropped (its time, in a temporary of `size` bytes
    /// at `w`, the element address in `e`).
    #[inline(never)]
    fn waveform_head(&mut self, size: u16, time: u16, w: u16, e: u16) -> R<()> {
        let a = self.wave_at(size, time, w)?;
        if size & pre::FRAME_WAVE == 0 {
            self.regs[e as usize] = add_addr(a, 8)?;
        }
        Ok(())
    }

    /// The start of a dropped waveform with its time written, as its first instructions made it:
    /// an `Alloc` of the size in register `size` into `w`, or (`pre::FRAME_WAVE`) the zeroing of
    /// the temporary in the frame `w` points to, whose registers stay as the frame set them.
    #[inline(never)]
    fn wave_at(&mut self, size: u16, time: u16, w: u16) -> R<u64> {
        let a = if size & pre::FRAME_WAVE != 0 {
            let a = self.regs[w as usize];
            self.wr(a, u64::from(size & !pre::FRAME_WAVE))?.fill(0);
            a
        } else {
            let a = self.temp_alloc(self.regs[size as usize])?;
            self.regs[w as usize] = a;
            a
        };
        self.store(a, Width::B8, self.regs[time as usize])?;
        Ok(a)
    }

    /// The `LSet` at IR pc `pc` and the `Drive` after it, as the IR has them.
    #[cold]
    #[inline(never)]
    fn replay_lset_drive(
        &mut self,
        prog: &Program,
        f: &Func,
        pc: u32,
        top: usize,
        ctx: &mut Option<&mut Ctx<'_>>,
    ) -> R<()> {
        for p in pc as usize..f.code.len() {
            let inst = f.code[p];
            self.frames[top].pc = p as u32;
            if !matches!(self.step_out(prog, f, inst, ctx)?, Flow::Next) {
                return Err(ice_s!("a signal assignment left its straight line"));
            }
            if matches!(inst, Inst::Drive { .. }) {
                return Ok(());
            }
        }
        Err(ice_s!("a signal assignment without its drive"))
    }

    #[inline(always)]
    pub(crate) fn sigref(&self, r: Reg) -> R<(SigRef, yavs_kernel::SignalType)> {
        self.sigref_words(self.r(r), self.r(Reg(r.0 + 1)))
    }

    /// `sigref` of a single-range reference given as its two words.
    #[inline(always)]
    pub(crate) fn sigref_words(&self, v: u64, len: u64) -> R<(SigRef, yavs_kernel::SignalType)> {
        let len = len as u32;
        let (base, off) = ((v >> 32) as u32, v as u32);
        let s = self
            .inst
            .sigs
            .get(base as usize)
            .ok_or_else(|| ice_f!("no instance signal {base}"))?;
        Ok((s.signal.slice(s.off + off, len), s.ty))
    }

    /// The raw entries `[lo, len, slot]` of signal reference `(lo, hi)` in its two registers:
    /// one for a single range (no slot), the table of a reference of several parts.
    fn raw_parts(&self, lo: u64, hi: u64) -> R<Vec<[u64; 3]>> {
        if lo >> 32 != MULTI {
            return Ok(vec![[lo, hi, NO_SLOT]]);
        }
        let n = lo & 0xffff_ffff;
        let b = self.rd(hi, n * PART_BYTES)?;
        Ok(b.as_chunks::<24>()
            .0
            .iter()
            .map(|e| {
                std::array::from_fn(|k| u64::from_le_bytes(std::array::from_fn(|j| e[k * 8 + j])))
            })
            .collect())
    }

    /// The kernel ranges of signal reference `r`, in order, with the driver slot each came
    /// with (`None`: a single range, driven through the instruction's own slot).
    fn sigparts(&self, r: Reg) -> R<Vec<(SigRef, yavs_kernel::SignalType, Option<u32>)>> {
        let (lo, hi) = (self.r(r), self.r(Reg(r.0 + 1)));
        let multi = lo >> 32 == MULTI;
        self.raw_parts(lo, hi)?
            .into_iter()
            .map(|[lo, len, slot]| {
                let (base, off) = ((lo >> 32) as u32, lo as u32);
                let s = self
                    .inst
                    .sigs
                    .get(base as usize)
                    .ok_or_else(|| ice_f!("no instance signal {base}"))?;
                let slot = match (multi, u32::try_from(slot)) {
                    (true, Ok(s)) => Some(s),
                    _ => None,
                };
                Ok((s.signal.slice(s.off + off, len as u32), s.ty, slot))
            })
            .collect()
    }

    /// Write raw part entries as a reference of several parts in temporary memory.
    fn set_parts(&mut self, dst: Reg, parts: &[[u64; 3]]) -> R<()> {
        let a = self.temp_alloc(parts.len() as u64 * PART_BYTES)?;
        let b: Vec<u8> = parts
            .iter()
            .flatten()
            .flat_map(|w| w.to_le_bytes())
            .collect();
        self.put(a, &b)?;
        self.set(dst, MULTI << 32 | parts.len() as u64);
        self.set(Reg(dst.0 + 1), a);
        Ok(())
    }

    /// The value of the parts of a reference, concatenated in order (`effective` or
    /// `'last_value`).
    fn read_parts(
        &self,
        c: &Ctx<'_>,
        parts: &[(SigRef, yavs_kernel::SignalType, Option<u32>)],
        last: bool,
    ) -> R<Value> {
        let elem = same_elem(parts)?;
        let mut vals = Vec::with_capacity(parts.len());
        for &(sr, ty, _) in parts {
            vals.push(if last {
                c.last_value(sr)?
            } else {
                match ty.elem {
                    Elem::Bytes(_) => Value::Bytes(c.value(sr)?.to_vec()),
                    Elem::Logic => Value::Logic(c.logic(sr)?),
                }
            });
        }
        concat(elem, &vals)
    }

    /// `Drive`/`DriveSlot` onto a reference of several parts: each part's elements of the
    /// waveform through the driver slot it came with.
    #[allow(clippy::too_many_arguments)]
    #[inline(never)]
    fn drive_parts(
        &mut self,
        ctx: &mut Option<&mut Ctx<'_>>,
        target: Reg,
        wave: Reg,
        n: Reg,
        mode: DriveMode,
        reject: Option<Reg>,
    ) -> R<()> {
        let parts = self.sigparts(target)?;
        let elem = same_elem(&parts)?;
        let total: u32 = parts.iter().map(|p| p.0.range.map_or(0, |r| r.len)).sum();
        let stride = 8 + value_size(elem, total).next_multiple_of(8);
        let w = self.r(wave);
        let mut items = Vec::new();
        for k in 0..self.r(n) {
            let at = add_addr(w, k * stride)?;
            let t = to_time(self.load(at, Width::B8)? as i64)?;
            items.push((t, self.get_value(add_addr(at, 8)?, elem, total)?));
        }
        let mode = match mode {
            DriveMode::Transport => DelayMode::Transport,
            DriveMode::Inertial => DelayMode::Inertial {
                reject: match reject {
                    Some(r) => Some(to_time(self.ri(r))?),
                    None => None,
                },
            },
        };
        let mut at = 0;
        for (sr, _, slot) in parts {
            let len = sr.range.map_or(0, |r| r.len);
            let slot = slot.ok_or_else(|| ice_s!("a signal part without a driver is driven"))?;
            let d = self.driver(slot)?;
            let vals = items
                .iter()
                .map(|(t, v)| Ok((*t, sub_value(v, elem, at, len)?)))
                .collect::<R<Vec<(Time, Value)>>>()?;
            let refs: Vec<(Time, ValueRef<'_>)> =
                vals.iter().map(|(t, v)| (*t, v.as_ref())).collect();
            let Some(c) = ctx.as_deref_mut() else {
                return rt_err_s!("signal or wait operation during elaboration");
            };
            c.assign(d.id, sr.range, &refs, mode)?;
            at += len;
        }
        Ok(())
    }

    #[inline(always)]
    pub(crate) fn is_multi(&self, r: Reg) -> bool {
        self.r(r) >> 32 == MULTI
    }

    #[inline(always)]
    pub(crate) fn driver(&self, d: u32) -> R<ProcDriver> {
        self.drivers
            .get(d as usize)
            .copied()
            .ok_or_else(|| ice_f!("no driver slot {d}"))
    }

    fn driver_ref(&self, d: &ProcDriver) -> R<(SigRef, Elem)> {
        let s = self
            .inst
            .sigs
            .get(d.sig as usize)
            .ok_or_else(|| ice_f!("no instance signal {}", d.sig))?;
        Ok((s.signal.slice(d.range.offset, d.range.len), s.ty.elem))
    }

    fn msg(&self, m: yavs_ir::CheckMsgId) -> yavs_ir::CheckMsg {
        self.rt
            .prog
            .checks
            .get(m)
            .cloned()
            .unwrap_or(yavs_ir::CheckMsg {
                what: "?".into(),
                dir: Dir::To,
                subtype: String::new(),
            })
    }

    fn check_msg(&self, m: yavs_ir::CheckMsgId) -> (&str, Dir) {
        match self.rt.prog.checks.get(m) {
            Some(c) => (&c.what, c.dir),
            None => ("?", Dir::To),
        }
    }

    /// `deallocate` of the access object at `a`: free its object and set it to null.
    pub(crate) fn deallocate(&mut self, a: u64) -> R<()> {
        let h = self.load(a, Width::B8)?;
        if h == 0 {
            return Ok(());
        }
        let (hp, _) = self.heap_of(h)?;
        match hp.free(h) {
            Ok(()) => {}
            Err(HeapError::Stale) => {
                return rt_err(
                    "deallocate of an access value whose object was already deallocated",
                );
            }
            Err(e) => return Err(ice_f!("deallocate: {e:?}")),
        }
        self.store(a, Width::B8, 0)
    }

    /// The heap a new object goes into: the shared one for `shared` or in a frame marked
    /// `shared_alloc`, else the private one. Every allocation of an access value's object
    /// goes through here.
    /// The tag of the heap [`Machine::alloc_heap`] would use.
    pub(crate) fn alloc_tag(&self, shared: bool) -> u8 {
        if shared || self.frames.last().is_some_and(|f| f.shared_alloc) {
            heap::SHARED
        } else {
            heap::PRIVATE
        }
    }

    pub(crate) fn alloc_heap(&mut self, shared: bool) -> R<&mut Heap> {
        if shared || self.frames.last().is_some_and(|f| f.shared_alloc) {
            let sh = self.shared_mut()?;
            Ok(&mut **sh
                .heap
                .get_or_insert_with(|| Box::new(Heap::new(heap::SHARED))))
        } else {
            Ok(&mut self.heap)
        }
    }

    pub(crate) fn heap_of(&mut self, h: u64) -> R<(&mut Heap, u8)> {
        match heap::tag_of(h) {
            heap::PRIVATE => Ok((&mut self.heap, HEAP)),
            heap::SHARED => {
                let sh = self.shared_mut()?;
                let h = sh
                    .heap
                    .get_or_insert_with(|| Box::new(Heap::new(heap::SHARED)));
                Ok((&mut **h, SHARED_HEAP))
            }
            t => Err(ice_f!("access value with heap tag {t}")),
        }
    }

    // ---- dispatch ----

    fn exec<const DEBUG: bool>(
        &mut self,
        ctx: &mut Option<&mut Ctx<'_>>,
        base: usize,
    ) -> R<Outcome> {
        let rt = self.rt_run.take().unwrap_or_else(|| self.rt.clone());
        let r = self.exec_in::<DEBUG>(&rt, ctx, base);
        self.rt_run = Some(rt);
        r
    }

    /// The layout words of `ty` constrained by the descriptor at `desc`
    /// (`[size, align, offsets...]` or `[size, align, stride, count]`), from `layouts`, which
    /// is brought up to date first.
    #[inline(never)]
    fn layout_words(&mut self, prog: &Program, ty: TyId, desc: u64) -> R<&[u8]> {
        let t = ty.0 as usize;
        if self.layouts.len() <= t {
            self.layouts.resize_with(t + 1, || None);
        }
        // A memo made from the same descriptor bytes (a loop asks for the same layout again and
        // again), or an array of scalars whose words come straight from the bounds: answered
        // in place, without taking the memo apart.
        if let Some(m) = &self.layouts[t]
            && let Ok(d) = self.rd_hot(desc, m.dsz)
        {
            let words = match m.simple {
                Some(n) => simple_words(n, d),
                None if m.desc == d => Some([0; 4]),
                None => None,
            };
            let simple = m.simple.is_some();
            if let Some(w) = words {
                let m = self.layouts[t]
                    .as_mut()
                    .ok_or_else(|| ice_s!("layout memo missing"))?;
                m.ckey = if split(desc).0 == CONST { desc } else { 0 };
                if simple {
                    m.words.clear();
                    for w in w {
                        m.words.extend_from_slice(&w.to_le_bytes());
                    }
                }
                return Ok(&m.words);
            }
        }
        self.layout_build(prog, ty, desc)
    }

    /// `layout_words` for a descriptor the memo of `ty` is known not to answer: the memo is made
    /// again from it.
    #[inline(never)]
    fn layout_build(&mut self, prog: &Program, ty: TyId, desc: u64) -> R<&[u8]> {
        let t = ty.0 as usize;
        if self.layouts.len() <= t {
            self.layouts.resize_with(t + 1, || None);
        }
        let memo = self.layouts[t].take();
        let dsz = match &memo {
            Some(m) => m.dsz,
            None => Ty::desc_size(&prog.types, ty)
                .ok_or_else(|| ice_f!("type {ty} has no descriptor size"))?,
        };
        let d = self.rd(desc, dsz)?;
        let ckey = if split(desc).0 == CONST { desc } else { 0 };
        let m = memo_fill(prog, ty, memo, dsz, d, ckey)?;
        let m = self.layouts[t].insert(m);
        Ok(&m.words)
    }

    /// Word `k` of the layout of `ty` for the descriptor bytes `d` (`TierHost::layout_word`): the
    /// memo's word when it was made from the same bytes, else the memo made again from them.
    pub(crate) fn layout_word_of(&mut self, prog: &Program, ty: TyId, d: &[u8], k: u32) -> R<u64> {
        let t = ty.0 as usize;
        if self.layouts.len() <= t {
            self.layouts.resize_with(t + 1, || None);
        }
        if let Some(m) = &self.layouts[t]
            && m.dsz == d.len() as u64
        {
            match m.simple {
                Some(n) => {
                    if let Some(&w) = simple_words(n, d).as_ref().and_then(|w| w.get(k as usize)) {
                        return Ok(w);
                    }
                }
                None if m.desc == d => return word_of(&m.words, k),
                None => {}
            }
        }
        let memo = self.layouts[t].take();
        let m = memo_fill(prog, ty, memo, d.len() as u64, d, 0)?;
        word_of(&self.layouts[t].insert(m).words, k)
    }

    /// `Inst::LayoutOf`: the layout words in temporary memory.
    #[inline(never)]
    fn layout_of(&mut self, prog: &Program, ty: TyId, desc: u64) -> R<u64> {
        let n = self.layout_words(prog, ty, desc)?.len() as u64;
        let a = self.temp_alloc(n)?;
        let t = ty.0 as usize;
        let (temp, words) = (&mut self.temp, &self.layouts[t]);
        let words = words.as_ref().map(|m| &m.words[..]).unwrap_or_default();
        let off = split(a).2 as usize;
        temp.get_mut(off..off + words.len())
            .ok_or_else(|| ice_s!("layout words outside the temporary area"))?
            .copy_from_slice(words);
        Ok(a)
    }

    /// Word `k` of the layout of `ty` constrained by the descriptor at `desc`, without
    /// writing the layout to memory (a `LayoutOf` whose result only one load reads).
    #[inline(never)]
    fn layout_word(&mut self, prog: &Program, ty: TyId, desc: u64, k: u32) -> R<u64> {
        if let Some(Some(m)) = self.layouts.get(ty.0 as usize)
            && m.ckey == desc
            && desc != 0
            && let Some(b) = m.words.get(k as usize * 8..k as usize * 8 + 8)
        {
            return Ok(le_bytes(b));
        }
        if let Some(Some(m)) = self.layouts.get(ty.0 as usize)
            && let Ok(d) = self.rd_hot(desc, m.dsz)
        {
            match m.simple {
                // A memoized array of scalars: the word straight from the bounds.
                Some(n) => {
                    if let Some(w) = simple_words(n, d)
                        && let Some(&v) = w.get(k as usize)
                    {
                        return Ok(v);
                    }
                }
                // The same descriptor bytes as the memo: its word, without `layout_words`
                // (a constant descriptor goes there once, to set `ckey`) (#518).
                None if split(desc).0 != CONST => {
                    if m.desc == d {
                        if let Some(b) = m.words.get(k as usize * 8..k as usize * 8 + 8) {
                            return Ok(le_bytes(b));
                        }
                    } else {
                        // Another descriptor than the memo's: no second comparison.
                        let words = self.layout_build(prog, ty, desc)?;
                        return word_of(words, k);
                    }
                }
                None => {}
            }
        }
        word_of(self.layout_words(prog, ty, desc)?, k)
    }

    /// `Inst::LGet`: the std_ulogic code of element `i` of the logic vector at `base`.
    #[inline(always)]
    fn lget(&self, base: u64, i: u64) -> R<u64> {
        let (sp, idx, off) = split(base);
        if let Some(b) = self
            .mem_fast(sp, idx)
            .and_then(|m| m.get((off + i / 64 * 32) as usize..))
            .and_then(|m| m.first_chunk::<32>())
        {
            let (w, s) = (b.as_chunks::<8>().0, i % 64);
            let bit = |p: usize| u64::from_le_bytes(w[p]) >> s & 1;
            return Ok(bit(0) | bit(1) << 1 | bit(2) << 2 | bit(3) << 3);
        }
        let b = self.rd(add_addr(base, i / 64 * 32)?, 32)?;
        Ok((0..4).fold(0u64, |c, p| c | (plane_word(b, p) >> (i % 64) & 1) << p))
    }

    /// `Inst::LSet`: set element `i` of the logic vector at `base` to `code`. The block in a
    /// frame, temporary memory or a heap object is found without the general lookup.
    #[inline(always)]
    fn lset(&mut self, base: u64, i: u64, code: u64) -> R<()> {
        if code > 8 {
            return Err(ice_f!("std_ulogic code {code}"));
        }
        let (sp, idx, off) = split(base);
        let mem = match sp {
            FRAME => self.fslots_mut(idx as usize),
            TEMP => Some(&mut self.temp[..]),
            HEAP => self.heap.data_mut(idx),
            SHARED_HEAP | SHARED => self.shared_mem_mut(sp, idx),
            _ => None,
        };
        if let Some(o) = off
            .checked_add(i / 64 * 32)
            .filter(|&o| o <= u64::from(u32::MAX))
            && let Some(b) = mem
                .and_then(|m| m.get_mut(o as usize..))
                .and_then(|m| m.first_chunk_mut::<32>())
        {
            let s = i % 64;
            for (p, c) in b.as_chunks_mut::<8>().0.iter_mut().enumerate() {
                let w = u64::from_le_bytes(*c) & !(1 << s) | (code >> p & 1) << s;
                *c = w.to_le_bytes();
            }
            return Ok(());
        }
        self.lset_slow(base, i, code)
    }

    #[inline(never)]
    fn lset_slow(&mut self, base: u64, i: u64, code: u64) -> R<()> {
        let a = add_addr(base, i / 64 * 32)?;
        let b = self.wr(a, 32)?;
        let s = i % 64;
        for p in 0..4 {
            let w = plane_word(b, p) & !(1 << s) | (code >> p & 1) << s;
            b[p * 8..p * 8 + 8].copy_from_slice(&w.to_le_bytes());
        }
        Ok(())
    }

    /// `Inst::LFill`: set the `n` elements from `i` of the logic vector at `base` to `code`, a
    /// plane word at a time.
    fn lfill(&mut self, base: u64, mut i: u64, mut n: u64, code: u64) -> R<()> {
        if code > 8 {
            return Err(ice_f!("std_ulogic code {code}"));
        }
        if n >= 512 {
            return self.lfill_wide(base, i, n, code);
        }
        while n > 0 {
            let s = i % 64;
            let take = (64 - s).min(n);
            let mask = if take == 64 {
                !0
            } else {
                ((1u64 << take) - 1) << s
            };
            let a = add_addr(base, i / 64 * 32)?;
            let b = self.wr(a, 32)?;
            for p in 0..4 {
                let set = if code >> p & 1 == 1 { mask } else { 0 };
                let w = plane_word(b, p) & !mask | set;
                b[p * 8..p * 8 + 8].copy_from_slice(&w.to_le_bytes());
            }
            i += take;
            n -= take;
        }
        Ok(())
    }

    /// `lfill` of 512 elements or more: a partial block at each end by the plane words, the
    /// whole blocks between them as one slice of the same 32 bytes.
    #[inline(never)]
    fn lfill_wide(&mut self, base: u64, i: u64, n: u64, code: u64) -> R<()> {
        let head = (64 - i % 64) % 64;
        if head > 0 {
            self.lfill(base, i, head, code)?;
        }
        let whole = (n - head) / 64;
        let mut pat = [0u8; 32];
        for p in 0..4 {
            if code >> p & 1 == 1 {
                pat[p * 8..p * 8 + 8].fill(0xff);
            }
        }
        let b = self.wr(add_addr(base, (i + head) / 64 * 32)?, whole * 32)?;
        b.as_chunks_mut::<32>().0.fill(pat);
        let done = head + whole * 64;
        self.lfill(base, i + done, n - done, code)
    }

    /// `Inst::Cat`: the pieces `ps` (pairs of an address or element and a count) one after
    /// another into the array of `n` elements at `d`, by the stores of the `LCopy`, `LSet`,
    /// `Copy` and `Store` it stands for.
    #[allow(clippy::too_many_arguments)]
    #[inline(never)]
    fn cat(
        &mut self,
        d: u64,
        eoff: u64,
        n: u64,
        m: u64,
        ps: &[Reg],
        scalars: u16,
        bias: u8,
        logic: bool,
        w: Width,
    ) -> R<()> {
        let big = || runtime_s!("object larger than 4 GiB");
        let total = n.wrapping_mul(m);
        if logic && total <= 64 && eoff == 0 && self.cat_block(d, total, m, ps, scalars, bias)? {
            return Ok(());
        }
        let mut k = 0u64;
        for (j, pr) in ps.as_chunks::<2>().0.iter().enumerate() {
            let v = self.r(pr[0]);
            let scalar = scalars >> j & 1 == 1;
            let len = if scalar { 1 } else { self.r(pr[1]) };
            if logic {
                let units = if scalar { 1 } else { len.wrapping_mul(m) };
                let off = eoff
                    .wrapping_add(total)
                    .wrapping_sub(k.wrapping_mul(m))
                    .wrapping_sub(units);
                if scalar {
                    self.lset(d, off, v.wrapping_add(u64::from(bias)))?;
                } else {
                    self.lcopy(d, off, v, 0, units)?;
                }
            } else {
                let at = index_addr(d, k.checked_mul(m).ok_or_else(big)?, 1)?;
                if scalar {
                    self.store(at, w, v)?;
                } else {
                    self.copy_mem(at, v, len.wrapping_mul(m))?;
                }
            }
            k = k.wrapping_add(len);
        }
        Ok(())
    }

    /// `Inst::Cat` of a logic array of at most 64 plane elements (one block of planes): the
    /// planes of all pieces combined in registers and written once. `false` when a piece
    /// does not fit the pattern (nothing is written; the caller stores piece by piece).
    #[inline(never)]
    fn cat_block(
        &mut self,
        d: u64,
        total: u64,
        m: u64,
        ps: &[Reg],
        scalars: u16,
        bias: u8,
    ) -> R<bool> {
        let Some((words, mask, k)) = self.cat_words(total, m, ps, scalars, bias)? else {
            return Ok(false);
        };
        if mask == 0 {
            return Ok(true);
        }
        // The block changed where it is, when it is in memory the machine holds in a field.
        let (sp, idx, off) = split(d);
        let mem = match sp {
            FRAME => self.fslots_mut(idx as usize),
            TEMP => Some(&mut self.temp[..]),
            HEAP => self.heap.data_mut(idx),
            _ => None,
        };
        if let Some(b) = mem.and_then(|m| m.get_mut(off as usize..off as usize + 32)) {
            let mut cur = [0u64; 4];
            if !(sp == TEMP && k == total) {
                fill_blocks(b, std::slice::from_mut(&mut cur));
            }
            for ((o, c), w) in b.as_chunks_mut::<8>().0.iter_mut().zip(cur).zip(words) {
                *o = (c & !mask | w).to_le_bytes();
            }
            return Ok(true);
        }
        // A block just allocated (zeroed) and covered by the pieces needs no read.
        let mut cur = [0u64; 4];
        if !(split(d).0 == TEMP && k == total) {
            fill_blocks(self.rd_hot(d, 32)?, std::slice::from_mut(&mut cur));
        }
        let mut out = [0u8; 32];
        for ((o, c), w) in out.as_chunks_mut::<8>().0.iter_mut().zip(cur).zip(words) {
            *o = (c & !mask | w).to_le_bytes();
        }
        self.put(d, &out)?;
        Ok(true)
    }

    /// The four planes of the block of 32 bytes at `a`. Out of line: inlined in a loop, the reads of
    /// every memory space load their fields before the loop (60 instructions per call of `cat_words`).
    #[inline(never)]
    fn block32(&self, a: u64) -> R<[u64; 4]> {
        let mut b = [0u64; 4];
        fill_blocks(self.rd_hot(a, 32)?, std::slice::from_mut(&mut b));
        Ok(b)
    }

    /// The planes of the pieces of a `Cat` of a logic array of at most 64 elements, combined:
    /// the words, the mask of the elements they cover and how many elements the pieces
    /// have. `None` when a piece does not fit the pattern.
    #[inline(never)]
    fn cat_words(
        &self,
        total: u64,
        m: u64,
        ps: &[Reg],
        scalars: u16,
        bias: u8,
    ) -> R<Option<([u64; 4], u64, u64)>> {
        let mut words = [0u64; 4];
        let mut mask = 0u64;
        let mut k = 0u64;
        for (j, pr) in ps.as_chunks::<2>().0.iter().enumerate() {
            let v = self.r(pr[0]);
            let (units, bits) = if scalars >> j & 1 == 1 {
                let code = v.wrapping_add(u64::from(bias));
                if code > 8 {
                    return Ok(None);
                }
                (1, [code & 1, code >> 1 & 1, code >> 2 & 1, code >> 3 & 1])
            } else {
                let units = self.r(pr[1]).wrapping_mul(m);
                if units == 0 {
                    continue;
                }
                if units > 64 {
                    return Ok(None);
                }
                (units, self.block32(v)?)
            };
            if k.wrapping_add(units) > total {
                return Ok(None);
            }
            let off = total - k - units;
            let pm = if units == 64 { !0 } else { (1u64 << units) - 1 };
            for (w, b) in words.iter_mut().zip(bits) {
                *w |= (b & pm) << off;
            }
            mask |= pm << off;
            k += units;
        }
        Ok(Some((words, mask, k)))
    }

    /// Whether a logic `Cat` of `total` elements can be built in place from element `doff` of
    /// `d`: the pieces `ps` cover exactly `total` elements and share no byte with the blocks
    /// it writes (a piece that did could be overwritten before it is read).
    #[inline(never)]
    fn cat_apart(&self, d: u64, doff: u64, total: u64, m: u64, ps: &[Reg], scalars: u16) -> bool {
        let Some(last) = doff.checked_add(total) else {
            return false;
        };
        let (at, dbytes) = (doff / 64 * 32, (last.div_ceil(64) - doff / 64) * 32);
        let mut covered = 0u64;
        for (j, pr) in ps.as_chunks::<2>().0.iter().enumerate() {
            let units = if scalars >> j & 1 == 1 {
                1
            } else {
                let units = self.r(pr[1]).wrapping_mul(m);
                if units != 0
                    && !self.disjoint(d, at, dbytes, self.r(pr[0]), 0, units.div_ceil(64) * 32)
                {
                    return false;
                }
                units
            };
            covered = covered.wrapping_add(units);
        }
        covered == total
    }

    /// A block of 32 bytes in the temporary area holding `b` (an `Alloc` of 32 and the stores
    /// of all of it, without clearing it first).
    #[inline(always)]
    fn temp_push32(&mut self, b: &[u8; 32]) -> R<u64> {
        let len = self.temp.len();
        if len & 7 == 0 && len < (u32::MAX - 32) as usize {
            self.temp.extend_from_slice(b);
            return Ok(u64::from(TEMP) << 61 | len as u64);
        }
        let off = (self.temp.len() as u64).next_multiple_of(8);
        if off + 32 > u64::from(u32::MAX) {
            return rt_err_s!("temporary memory of one statement exceeds 4 GiB");
        }
        self.temp.resize(off as usize, 0);
        self.temp.extend_from_slice(b);
        mk_addr(TEMP, 0, off)
    }

    /// `Inst::Copy`: `n` bytes from `s` to `d`.
    #[inline(always)]
    fn copy_mem(&mut self, d: u64, s: u64, n: u64) -> R<()> {
        if self.copy_fast(d, s, n) {
            return Ok(());
        }
        self.copy_staged(d, s, n)
    }

    /// The bytes through a staging buffer, errors included.
    #[inline(never)]
    fn copy_staged(&mut self, d: u64, s: u64, n: u64) -> R<()> {
        let mut b = self.take_bytes();
        let r = self
            .rd(s, n)
            .map(|s| b.extend_from_slice(s))
            .and_then(|()| self.put(d, &b));
        self.give_bytes(b);
        r
    }

    /// Copy directly when both ends are in memory the machine holds in fields of their own:
    /// inside one object (`copy_within`), or from frame, temporary, instance, constant or heap
    /// memory into temporary, frame or heap memory. `false`: nothing was copied (another
    /// combination, or an address that does not resolve: the staged path reports it).
    #[inline(always)]
    fn copy_fast(&mut self, d: u64, s: u64, n: u64) -> bool {
        let (dsp, didx, doff) = split(d);
        let (ssp, sidx, soff) = split(s);
        let (n, doff, soff) = (n as usize, doff as usize, soff as usize);
        // A length that overflows an end is no copy for this path: the staged one reports it.
        if doff.checked_add(n).is_none() || soff.checked_add(n).is_none() {
            return false;
        }
        let Machine {
            frames,
            root,
            sstack,
            temp,
            heap,
            sh,
            ..
        } = self;
        if dsp == ssp && didx == sidx {
            let b = match dsp {
                FRAME => slots_of_mut(frames, root, sstack, didx as usize),
                TEMP => Some(&mut temp[..]),
                HEAP => heap.data_mut(didx),
                SHARED_HEAP => sh.as_mut().and_then(|s| s.heap.as_mut()?.data_mut(didx)),
                _ => None,
            };
            let Some(b) = b else { return false };
            if doff + n <= b.len() && soff + n <= b.len() {
                b.copy_within(soff..soff + n, doff);
                return true;
            }
            return false;
        }
        self.copy_pair(d, s, n, n, |out, src| out.copy_from_slice(src))
    }

    /// `f(dst, src)` on the `dn` bytes at `d` and the `sn` bytes at `s` when they are in two
    /// different objects the machine holds in fields of their own (as `copy_fast`). `false`:
    /// `f` was not called.
    #[inline(always)]
    fn copy_pair(
        &mut self,
        d: u64,
        s: u64,
        dn: usize,
        sn: usize,
        f: impl FnOnce(&mut [u8], &[u8]),
    ) -> bool {
        let (dsp, didx, doff) = split(d);
        let (ssp, sidx, soff) = split(s);
        let (doff, soff) = (doff as usize, soff as usize);
        if doff.checked_add(dn).is_none() || soff.checked_add(sn).is_none() {
            return false;
        }
        let Machine {
            frames,
            root,
            sstack,
            temp,
            heap,
            inst,
            rt,
            sh,
            ..
        } = self;
        // Two objects of one heap: both slots at once.
        if dsp == ssp && matches!(dsp, HEAP | SHARED_HEAP) {
            let h = if dsp == HEAP {
                Some(heap)
            } else {
                sh.as_mut().and_then(|s| s.heap.as_deref_mut())
            };
            return match h.and_then(|h| h.data_pair(didx, sidx)) {
                Some((out, src)) if soff + sn <= src.len() => match out.get_mut(doff..doff + dn) {
                    Some(out) => {
                        f(out, &src[soff..soff + sn]);
                        true
                    }
                    None => false,
                },
                _ => false,
            };
        }
        let dst = match dsp {
            TEMP => {
                let src = match ssp {
                    FRAME => slots_of(frames, root, sstack, sidx as usize),
                    INST => Some(&inst.data[..]),
                    CONST => const_bytes(rt, yavs_ir::ConstId(sidx)).ok(),
                    HEAP => heap.data(sidx),
                    SHARED_HEAP => sh.as_ref().and_then(|s| s.heap.as_ref()?.data(sidx)),
                    SHARED => sh.as_ref().map(|s| &s.vars[..]),
                    _ => None,
                };
                (src, temp.get_mut(doff..doff + dn))
            }
            FRAME => {
                let src = match ssp {
                    TEMP => Some(&temp[..]),
                    INST => Some(&inst.data[..]),
                    CONST => const_bytes(rt, yavs_ir::ConstId(sidx)).ok(),
                    HEAP => heap.data(sidx),
                    SHARED_HEAP => sh.as_ref().and_then(|s| s.heap.as_ref()?.data(sidx)),
                    SHARED => sh.as_ref().map(|s| &s.vars[..]),
                    _ => None,
                };
                (
                    src,
                    slots_of_mut(frames, root, sstack, didx as usize)
                        .and_then(|b| b.get_mut(doff..doff + dn)),
                )
            }
            HEAP => {
                let src = match ssp {
                    FRAME => slots_of(frames, root, sstack, sidx as usize),
                    TEMP => Some(&temp[..]),
                    INST => Some(&inst.data[..]),
                    CONST => const_bytes(rt, yavs_ir::ConstId(sidx)).ok(),
                    SHARED_HEAP => sh.as_ref().and_then(|s| s.heap.as_ref()?.data(sidx)),
                    SHARED => sh.as_ref().map(|s| &s.vars[..]),
                    _ => None,
                };
                (
                    src,
                    heap.data_mut(didx).and_then(|b| b.get_mut(doff..doff + dn)),
                )
            }
            SHARED_HEAP => {
                let src = match ssp {
                    FRAME => slots_of(frames, root, sstack, sidx as usize),
                    TEMP => Some(&temp[..]),
                    INST => Some(&inst.data[..]),
                    CONST => const_bytes(rt, yavs_ir::ConstId(sidx)).ok(),
                    HEAP => heap.data(sidx),
                    _ => None,
                };
                (
                    src,
                    sh.as_mut()
                        .and_then(|s| s.heap.as_mut()?.data_mut(didx))
                        .and_then(|b| b.get_mut(doff..doff + dn)),
                )
            }
            _ => return false,
        };
        match dst {
            (Some(src), Some(out)) if soff + sn <= src.len() => {
                f(out, &src[soff..soff + sn]);
                true
            }
            _ => false,
        }
    }

    /// `Inst::MakeDope`: the values of `parts` as words at `at`.
    #[inline(always)]
    fn make_dope(&mut self, at: u64, parts: &[Reg]) -> R<()> {
        // In the temporary area (a descriptor a statement builds): word by word, in place.
        let (sp, _, off) = split(at);
        if sp == TEMP
            && let Some(dst) = self
                .temp
                .get_mut(off as usize..(off as usize) + parts.len() * 8)
        {
            for (c, &r) in dst.as_chunks_mut::<8>().0.iter_mut().zip(parts) {
                *c = self.regs[r.0 as usize].to_le_bytes();
            }
            return Ok(());
        }
        // In the frame (a temporary of a process there, `yavs_ir` `opt/slots.rs`): the same.
        if sp == FRAME {
            let vals: [u64; 8] = std::array::from_fn(|k| parts.get(k).map_or(0, |r| self.r(*r)));
            if parts.len() <= 8
                && let Some(dst) = self.frame_bytes(at, parts.len() * 8)
            {
                for (c, v) in dst.as_chunks_mut::<8>().0.iter_mut().zip(vals) {
                    *c = v.to_le_bytes();
                }
                return Ok(());
            }
        }
        let mut buf = [0u8; 64];
        if parts.len() <= 8 {
            for (c, &r) in buf.as_chunks_mut::<8>().0.iter_mut().zip(parts) {
                *c = self.r(r).to_le_bytes();
            }
            return self.put(at, &buf[..parts.len() * 8]);
        }
        let mut out = Vec::with_capacity(parts.len() * 8);
        for &r in parts {
            out.extend_from_slice(&self.r(r).to_le_bytes());
        }
        self.put(at, &out)
    }

    /// `Inst::CopyConst`: constant `c` to the memory at `a`.
    #[inline(always)]
    fn copy_const(&mut self, prog: &Program, a: u64, c: yavs_ir::ConstId) -> R<()> {
        // A compact constant is written straight to its copy, never expanded and kept in the
        // runtime (#446); `prog` is the running runtime's program, borrowed apart from the
        // machine, so no handle is cloned per call.
        if let Some(d @ (ConstData::Repeat { .. } | ConstData::Runs(_))) = prog.consts.get(c) {
            let n = compact_len(d)?;
            fill_compact(d, self.wr(a, n)?);
            return Ok(());
        }
        let n = const_bytes(&self.rt, c)?.len() as u64;
        self.copy_mem(a, mk_addr(CONST, c.0, 0)?, n)
    }

    /// `Inst::SubtypeCheck`: the length of each dimension of the descriptor at `dope` against
    /// the one at `want`. Out of line, with the usual case (one dimension, both descriptors in
    /// memory, equal bounds or lengths that fit 63 bits) before the general loop.
    #[inline(never)]
    fn subtype_check(&self, dope: u64, want: u64, dims: u8, what: yavs_ir::CheckMsgId) -> R<()> {
        // Equal descriptors (mostly a temporary against instance data), or equal lengths, are
        // the usual case.
        if dims == 1
            && let (Some(a), Some(b)) = (self.desc24(dope), self.desc24(want))
            && (a == b
                || matches!((dim_len63(desc_words(a)), dim_len63(desc_words(b))),
                    (Some(x), Some(y)) if x == y))
        {
            return Ok(());
        }
        self.subtype_check_slow(dope, want, dims, what)
    }

    /// The 24 bytes of the first dimension of the descriptor at `a`, in temporary memory or the
    /// instance data (the usual homes of a value's and a target's descriptor).
    #[inline(always)]
    fn desc24(&self, a: u64) -> Option<&[u8; 24]> {
        let (sp, idx, off) = split(a);
        let mem = match sp {
            TEMP => &self.temp[..],
            INST => &self.inst.data[..],
            _ => self.mem_fast(sp, idx)?,
        };
        mem.get(off as usize..)?.first_chunk::<24>()
    }

    #[inline(never)]
    fn subtype_check_slow(
        &self,
        dope: u64,
        want: u64,
        dims: u8,
        what: yavs_ir::CheckMsgId,
    ) -> R<()> {
        if dims == 1
            && let (Ok(a), Ok(b)) = (self.dim_of(dope, 0), self.dim_of(want, 0))
        {
            if a == b {
                return Ok(());
            }
            if let (Some(x), Some(y)) = (dim_len63(a), dim_len63(b)) {
                return if x == y {
                    Ok(())
                } else {
                    self.subtype_fail(x, y, 0, 1, what)
                };
            }
        }
        for k in 0..u64::from(dims) {
            let len = |base: u64| -> R<u64> {
                let (l, r, dir) = self.dim_of(base, k)?;
                yavs_ir::dim_len(l, r, dir == 1).ok_or_else(|| runtime_s!("range too long"))
            };
            let (a, b) = (len(dope)?, len(want)?);
            if a != b {
                return self.subtype_fail(a, b, k, dims, what);
            }
        }
        Ok(())
    }

    #[cold]
    #[inline(never)]
    fn subtype_fail(&self, a: u64, b: u64, k: u64, dims: u8, what: yavs_ir::CheckMsgId) -> R<()> {
        let (w, _) = self.check_msg(what);
        let dim = if dims > 1 {
            format!(" for dimension {}", k + 1)
        } else {
            String::new()
        };
        rt_err_f!("value length {a}{dim} does not match {w} length {b}")
    }

    /// `Inst::Deref`: the address of the object the access value `h` designates.
    #[inline(always)]
    fn deref(&mut self, h: u64, what: yavs_ir::CheckMsgId) -> R<u64> {
        if h == 0 {
            // sim1's wording; the statement span names the place.
            return rt_err_s!("null access dereference");
        }
        // The heaps as they are, without `heap_of`'s creation of an absent shared heap.
        let hp = match heap::tag_of(h) {
            heap::PRIVATE => Some((&self.heap, HEAP)),
            heap::SHARED => self
                .sh
                .as_ref()
                .and_then(|s| s.heap.as_deref())
                .map(|hp| (hp, SHARED_HEAP)),
            _ => None,
        };
        let Some((hp, space)) = hp else {
            return self.deref_other(h, what);
        };
        match hp.resolve(h) {
            // A slot index is below `heap::MAX_INDEX`, far below the limit of an address.
            Ok(i) => Ok(u64::from(space) << 61 | u64::from(i) << 32),
            Err(e) => self.deref_fail(e, what),
        }
    }

    /// `deref` of a handle of a heap that is not there yet, or of no heap.
    #[cold]
    #[inline(never)]
    fn deref_other(&mut self, h: u64, what: yavs_ir::CheckMsgId) -> R<u64> {
        let (hp, space) = self.heap_of(h)?;
        match hp.resolve(h) {
            Ok(i) => mk_addr(space, i, 0),
            Err(e) => self.deref_fail(e, what),
        }
    }

    #[cold]
    #[inline(never)]
    fn deref_fail(&self, e: HeapError, what: yavs_ir::CheckMsgId) -> R<u64> {
        match e {
            HeapError::Stale => {
                let (w, _) = self.check_msg(what);
                rt_err_f!("access value designates an object that was deallocated: {w}")
            }
            e => Err(ice_f!("dereference: {e:?}")),
        }
    }

    /// `Inst::Field`: the address of field `n` of the record at `base`, laid out by `layout`.
    #[inline(always)]
    fn field_addr(&self, base: u64, layout: u64, n: u32) -> R<u64> {
        let off = self.load(add_addr(layout, 16 + 8 * u64::from(n))?, Width::B8)?;
        add_addr(base, off)
    }

    /// `step` out of line: the fast loop stays small.
    #[inline(never)]
    fn step_out(
        &mut self,
        prog: &Program,
        f: &Func,
        inst: Inst,
        ctx: &mut Option<&mut Ctx<'_>>,
    ) -> R<Flow> {
        // The jump back of a process that never suspends: a stop request (Ctrl-C) ends the
        // activation as a `wait;` would (`pre::Code::new` steps those jumps). The activation is
        // bounded too: without a wait the process never gives control back, so time cannot
        // advance and the run would spin forever (#583).
        if let Inst::Br { to } = inst {
            if f.kind == FuncKind::Process {
                if !f.flags.contains(yavs_ir::FuncFlags::MAY_SUSPEND) {
                    if ctx.as_deref().is_some_and(|c| c.stop_requested()) {
                        return Ok(Flow::Suspend);
                    }
                    self.spin += 1;
                    if self.spin >= spin_range().1 {
                        return rt_err_s!(
                            "process has no wait statement and no sensitivity list: it never suspends, so simulation time cannot advance (an infinite activation)"
                        );
                    }
                    if self.spin.is_multiple_of(SPIN_CHECK)
                        && let Some(c) = ctx.as_deref_mut()
                    {
                        c.trim_run();
                    }
                }
            } else if let Some(fr) = self.frames.last()
                && let Some(over) = prog.back_edge(fr.func)
            {
                // A jump back of a function that runs as lowered (`pre::count_back_edges`).
                if over {
                    self.back_budget()?;
                }
                return Ok(Flow::Jump(to));
            }
        }
        self.step(prog, f, inst, ctx)
    }

    /// A lowered function took its budget of jumps back (`Program::back_edge`, #695): it is
    /// listed to be optimised before the next call. A call that has only computed (a pure entry
    /// function, no report, nothing tainted, no coverage) stops to run again with it.
    #[cold]
    #[inline(never)]
    fn back_budget(&mut self) -> R<()> {
        let flags = |f: FuncId| self.rt.prog.funcs.get(f).map(|f| f.flags);
        let pure = self
            .frames
            .first()
            .and_then(|fr| flags(fr.func))
            .is_some_and(|fl| {
                fl.contains(yavs_ir::FuncFlags::PURE)
                    && !fl.contains(yavs_ir::FuncFlags::TOUCHES_SHARED)
                    && !fl.contains(yavs_ir::FuncFlags::TOUCHES_FILES)
            });
        if pure
            && yavs_ir::ElabRaw::restartable()
            && self.tainted.is_none()
            && self.init_reports.as_ref().is_some_and(Vec::is_empty)
            && self.rt.cov.is_empty()
        {
            return Err(Fault::Restart);
        }
        Ok(())
    }

    /// `Inst::DopeLen`: the element count of dimension `dim` of the descriptor at `dope`.
    #[inline(always)]
    fn dope_len(&self, dope: u64, dim: u8) -> R<u64> {
        let (l, rr, dir) = self.dim_of(dope, u64::from(dim))?;
        match dim_len63((l, rr, dir)) {
            Some(n) if n <= i64::MAX as u64 => Ok(n),
            _ => yavs_ir::dim_len(l, rr, dir == 1)
                .filter(|&n| n <= i64::MAX as u64)
                .ok_or_else(|| runtime_f!("range {l} .. {rr} too long")),
        }
    }

    /// `Inst::SigVal`: the two registers of a reference to elements `o .. o + l` of instance
    /// signal `sig`.
    #[inline(always)]
    fn sig_val(&self, sig: u32, o: i64, l: i64) -> R<(u64, u64)> {
        let (Ok(o), Ok(l)) = (u32::try_from(o), u32::try_from(l)) else {
            return rt_err_f!("signal element range {o} + {l} out of bounds");
        };
        if self.inst.sigs.get(sig as usize).is_none() {
            return Err(ice_f!("no instance signal {sig}"));
        }
        Ok((u64::from(sig) << 32 | u64::from(o), u64::from(l)))
    }

    /// The empty staging buffer of `Copy` and `Fill`; hand it back with `give_bytes`.
    pub(crate) fn take_bytes(&mut self) -> Vec<u8> {
        let mut b = std::mem::take(&mut self.bytes);
        b.clear();
        b
    }

    /// Keep a staging buffer for the next `Copy` or `Fill`, unless it grew large.
    pub(crate) fn give_bytes(&mut self, b: Vec<u8>) {
        if b.capacity() <= 1 << 16 {
            self.bytes = b;
        }
    }

    /// `Inst::SigReadScalar`: the value of the one element `sig` refers to.
    #[inline(never)]
    fn sig_read_scalar(&mut self, sig: Reg, ctx: &mut Option<&mut Ctx<'_>>) -> R<u64> {
        // A table of one part (an association of one element) reads like a single range.
        let (sr, ty) = if self.is_multi(sig) {
            match self.sigparts(sig)?.as_slice() {
                [(sr, ty, _)] => (*sr, *ty),
                _ => return Err(ice_s!("scalar read of a reference of several parts")),
            }
        } else {
            self.sigref(sig)?
        };
        Self::read_elem(sr, ty, ctx)
    }

    /// `Inst::Edge`: the clock-edge test `kind` of the one element `sig` refers to.
    #[inline(never)]
    fn edge_inst(&mut self, kind: EdgeKind, sig: Reg, ctx: &mut Option<&mut Ctx<'_>>) -> R<u64> {
        let sr = if self.is_multi(sig) {
            match self.sigparts(sig)?.as_slice() {
                [(sr, _, _)] => *sr,
                _ => return Err(ice_s!("edge test of a reference of several parts")),
            }
        } else {
            self.sigref(sig)?.0
        };
        Self::edge_elem(sr, kind, ctx)
    }

    /// `Inst::WaitEdge`: the coming wait resumes the process only in a cycle where the edge
    /// `kind` of the one element `sig` refers to holds.
    #[inline(never)]
    fn wait_edge_inst(&mut self, code: u8, sig: Reg, ctx: &mut Option<&mut Ctx<'_>>) -> R<()> {
        let Some(c) = ctx.as_deref_mut() else {
            return rt_err_s!("signal or wait operation during elaboration");
        };
        if !self.is_multi(sig) {
            c.wait_edge(self.sigref(sig)?.0, code);
        }
        Ok(())
    }

    /// The value of the one element of `sr`.
    #[inline(always)]
    fn read_elem(
        sr: SigRef,
        ty: yavs_kernel::SignalType,
        ctx: &mut Option<&mut Ctx<'_>>,
    ) -> R<u64> {
        let Some(c) = ctx.as_deref_mut() else {
            return rt_err_s!("signal or wait operation during elaboration");
        };
        let v = match ty.elem {
            Elem::Bytes(es) if es <= 8 => cell(c.value(sr)?),
            Elem::Logic => {
                let Some(r) = sr.range.filter(|r| r.len > 0) else {
                    return Err(ice_s!("scalar read of a null signal range"));
                };
                u64::from(c.elem_code(sr.signal, r.offset)?)
            }
            Elem::Bytes(es) => {
                return Err(ice_f!("scalar read of a {es}-byte element"));
            }
        };
        Ok(v)
    }

    /// Element `o` of instance signal `sig`: what `SigVal` of one element makes of its
    /// registers (`sig_val`, `sigref`).
    #[inline(always)]
    fn sig_elem(&self, sig: u32, o: u64) -> R<(SigRef, yavs_kernel::SignalType)> {
        let Ok(o) = u32::try_from(o as i64) else {
            return rt_err_f!("signal element range {} + 1 out of bounds", o as i64);
        };
        let s = self
            .inst
            .sigs
            .get(sig as usize)
            .ok_or_else(|| ice_f!("no instance signal {sig}"))?;
        Ok((s.signal.slice(s.off + o, 1), s.ty))
    }

    /// The clock-edge test `kind` of the first element of `sr` (`Inst::Edge`).
    #[inline(always)]
    fn edge_elem(sr: SigRef, kind: EdgeKind, ctx: &mut Option<&mut Ctx<'_>>) -> R<u64> {
        let Some(c) = ctx.as_deref_mut() else {
            return rt_err_s!("signal read during elaboration");
        };
        let Some(hit) = c.edge_by(sr, kind.code())? else {
            return Err(ice_s!("edge of a null signal range"));
        };
        Ok(u64::from(hit))
    }

    /// Add `sr` to the sensitivity of the coming wait.
    #[inline(always)]
    fn wait_elem(sr: SigRef, ctx: &mut Option<&mut Ctx<'_>>) -> R<()> {
        let Some(c) = ctx.as_deref_mut() else {
            return rt_err_s!("signal or wait operation during elaboration");
        };
        // A clocked process waits on what it waited on: nothing to register again.
        if sr.range.is_some_and(|r| c.wait_on_again(sr.signal, r)) {
            return Ok(());
        }
        c.wait_on(sr)?;
        Ok(())
    }

    /// The current value of element `o` of instance signal `sig`: the one-element reads of an
    /// RTL process. The signal and the element are taken apart as plain numbers (no
    /// `SigRef` or type to keep in memory); whatever is not a plain logic read, and every fault,
    /// is `read_sig1_slow`.
    #[inline(always)]
    fn read_sig1(&self, sig: u32, o: u64, ctx: &mut Option<&mut Ctx<'_>>) -> R<u64> {
        if let Some(s) = self.inst.sigs.get(sig as usize)
            && s.ty.elem == Elem::Logic
            && let Ok(o) = u32::try_from(o)
            && let Some(e) = s.off.checked_add(o)
            && let Some(c) = ctx.as_deref()
            && let Ok(code) = c.elem_code(s.signal, e)
        {
            return Ok(u64::from(code));
        }
        self.read_sig1_slow(sig, o, ctx)
    }

    #[inline(never)]
    fn read_sig1_slow(&self, sig: u32, o: u64, ctx: &mut Option<&mut Ctx<'_>>) -> R<u64> {
        // A scalar of at most 8 bytes (an enum, an integer): its bytes, no reference to build.
        if let Some(s) = self.inst.sigs.get(sig as usize)
            && matches!(s.ty.elem, Elem::Bytes(es) if es <= 8)
            && let Ok(o) = u32::try_from(o)
            && let Some(e) = s.off.checked_add(o)
            && let Some(c) = ctx.as_deref()
            && let Some(b) = c.elem_bytes(s.signal, e)
        {
            return Ok(cell(b));
        }
        let (sr, ty) = self.sig_elem(sig, o)?;
        Self::read_elem(sr, ty, ctx)
    }

    /// The clock-edge test `kind` of element `o` of instance signal `sig` (as `read_sig1`).
    #[inline(always)]
    fn edge_sig1(
        &self,
        sig: u32,
        o: u64,
        kind: EdgeKind,
        ctx: &mut Option<&mut Ctx<'_>>,
    ) -> R<u64> {
        if let Some(s) = self.inst.sigs.get(sig as usize)
            && let Ok(o) = u32::try_from(o)
            && let Some(e) = s.off.checked_add(o)
            && let Some(c) = ctx.as_deref()
        {
            if c.edge_known(s.signal, e, kind.code()) {
                return Ok(1);
            }
            if let Ok(Some(hit)) = c.edge_by(s.signal.slice(e, 1), kind.code()) {
                return Ok(u64::from(hit));
            }
        }
        self.edge_sig1_slow(sig, o, kind, ctx)
    }

    #[inline(never)]
    fn edge_sig1_slow(
        &self,
        sig: u32,
        o: u64,
        kind: EdgeKind,
        ctx: &mut Option<&mut Ctx<'_>>,
    ) -> R<u64> {
        let (sr, _) = self.sig_elem(sig, o)?;
        Self::edge_elem(sr, kind, ctx)
    }

    /// Add element `o` of instance signal `sig` to the sensitivity of the coming wait (as
    /// `read_sig1`).
    #[inline(always)]
    fn wait_sig1(&self, sig: u32, o: u64, ctx: &mut Option<&mut Ctx<'_>>) -> R<()> {
        if let Some(s) = self.inst.sigs.get(sig as usize)
            && let Ok(o) = u32::try_from(o)
            && let Some(e) = s.off.checked_add(o)
            && let Some(c) = ctx.as_deref_mut()
            && c.wait_on_again(s.signal, yavs_kernel::Range::new(e, 1))
        {
            return Ok(());
        }
        self.wait_sig1_slow(sig, o, ctx)
    }

    #[inline(never)]
    fn wait_sig1_slow(&self, sig: u32, o: u64, ctx: &mut Option<&mut Ctx<'_>>) -> R<()> {
        let (sr, _) = self.sig_elem(sig, o)?;
        Self::wait_elem(sr, ctx)
    }

    /// `Inst::WaitEdge` and `Inst::WaitOn` of element `o` of instance signal `sig` (as `wait_sig1`).
    #[inline(always)]
    fn wait_edge_sig1(&self, sig: u32, o: u64, kind: u8, ctx: &mut Option<&mut Ctx<'_>>) -> R<()> {
        if let Some(s) = self.inst.sigs.get(sig as usize)
            && let Ok(o) = u32::try_from(o)
            && let Some(e) = s.off.checked_add(o)
            && let Some(c) = ctx.as_deref_mut()
        {
            c.wait_edge(s.signal.slice(e, 1), kind);
            if c.wait_on_again(s.signal, yavs_kernel::Range::new(e, 1)) {
                return Ok(());
            }
        }
        self.wait_edge1_slow(sig, o, kind, ctx)?;
        self.wait_sig1_slow(sig, o, ctx)
    }

    #[inline(never)]
    fn wait_edge1_slow(&self, sig: u32, o: u64, kind: u8, ctx: &mut Option<&mut Ctx<'_>>) -> R<()> {
        let (sr, _) = self.sig_elem(sig, o)?;
        let Some(c) = ctx.as_deref_mut() else {
            return rt_err_s!("signal or wait operation during elaboration");
        };
        c.wait_edge(sr, kind);
        Ok(())
    }

    /// `Inst::WaitFor`.
    fn wait_for(&mut self, t: Reg, ctx: &mut Option<&mut Ctx<'_>>) -> R<()> {
        let t = to_time(self.ri(t))?;
        let Some(c) = ctx.as_deref_mut() else {
            return rt_err_s!("signal or wait operation during elaboration");
        };
        c.wait_for(t)?;
        if self.dbg.is_some() {
            self.waits.timeout = c.after(t).ok();
        }
        Ok(())
    }

    /// `Inst::WaitOn`.
    #[inline(never)]
    fn wait_on(&mut self, sig: Reg, ctx: &mut Option<&mut Ctx<'_>>) -> R<()> {
        let Some(c) = ctx.as_deref_mut() else {
            return rt_err_s!("signal or wait operation during elaboration");
        };
        if self.is_multi(sig) {
            for (sr, _, _) in self.sigparts(sig)? {
                c.wait_on(sr)?;
                if self.dbg.is_some() {
                    self.waits.sens.push(sr);
                }
            }
        } else {
            let sr = self.sigref(sig)?.0;
            if !sr.range.is_some_and(|r| c.wait_on_again(sr.signal, r)) {
                c.wait_on(sr)?;
            }
            if self.dbg.is_some() {
                self.waits.sens.push(sr);
            }
        }
        Ok(())
    }

    /// The checks of `Inst::Suspend`; the caller then suspends.
    #[inline(always)]
    fn suspend(&mut self, prog: &Program, f: &Func, ctx: &Option<&mut Ctx<'_>>) -> R<()> {
        // The root frame of a process waits: no other frame to check.
        if self.root_process && self.frames.len() == 1 && ctx.is_some() && self.dbg.is_none() {
            return Ok(());
        }
        self.suspend_checks(prog, f, ctx)
    }

    #[inline(never)]
    fn suspend_checks(&mut self, prog: &Program, f: &Func, ctx: &Option<&mut Ctx<'_>>) -> R<()> {
        if let Some(fr) = self.frames.iter().find(|fr| {
            prog.func(fr.func)
                .is_some_and(|g| g.kind != FuncKind::Process && g.kind != FuncKind::Procedure)
        }) {
            return Err(ice_f!("wait inside {}, which cannot suspend", fr.func));
        }
        if ctx.is_none() {
            return rt_err_s!("wait statement executed during elaboration");
        }
        // LRM 11.3: a procedure a process with a sensitivity list calls shall not wait.
        if self.frames.len() > 1
            && prog
                .func(self.entry)
                .is_some_and(|g| g.flags.contains(yavs_ir::FuncFlags::SENS_LIST))
        {
            return rt_err(
                "wait statement in a procedure called by a process with a sensitivity list",
            );
        }
        if self.dbg.is_some() {
            let fr = &self.frames[self.frames.len() - 1];
            self.waits.stmt = f.loc.get(fr.pc as usize).copied();
        }
        Ok(())
    }

    /// `Inst::TempRoot`.
    fn temp_root_inst(
        &mut self,
        prog: &Program,
        addr: Reg,
        desc: Option<Reg>,
        ty: Option<yavs_ir::TyId>,
    ) -> R<()> {
        let Some(t) = ty else {
            return self.fat_root(self.r(addr));
        };
        let a = self.r(addr);
        if split(a).0 != TEMP {
            return Err(ice_s!("a recorded temporary outside the temporary area"));
        }
        let n = Ty::desc_size(&prog.types, t).ok_or_else(|| ice_f!("no descriptor size of {t}"))?;
        let desc = match desc {
            Some(d) if n > 0 => self.rd(self.r(d), n)?.into(),
            None if n > 0 => {
                return Err(ice_f!("recorded temporary of {t} without its descriptor"));
            }
            _ => Box::default(),
        };
        self.temp_root(a, Some(t), desc);
        Ok(())
    }

    /// Append the `ds` bytes at `d` and then the `n` bytes at `s` to the temporary area, 8-byte
    /// aligned, and return the address of the first. `None`: nothing was appended (a size or
    /// an address the general path reports).
    fn temp_append_copy(&mut self, d: u64, ds: u64, s: u64, n: u64) -> Option<u64> {
        let off = (self.temp.len() as u64).next_multiple_of(8);
        if off.checked_add(ds)?.checked_add(n)? > u64::from(u32::MAX) {
            return None;
        }
        let was = self.temp.len();
        self.temp.resize(off as usize, 0);
        if (ds == 0 || self.temp_append(d, ds as usize)) && self.temp_append(s, n as usize) {
            return mk_addr(TEMP, 0, off).ok();
        }
        self.temp.truncate(was);
        None
    }

    /// Append the `n` bytes at `s` to the temporary area. `false`: nothing was appended.
    #[inline(always)]
    fn temp_append(&mut self, s: u64, n: usize) -> bool {
        let (ssp, sidx, soff) = split(s);
        let soff = soff as usize;
        let Some(end) = soff.checked_add(n) else {
            return false;
        };
        let Machine {
            frames,
            root,
            sstack,
            temp,
            heap,
            inst,
            rt,
            sh,
            ..
        } = self;
        let src = match ssp {
            TEMP if end <= temp.len() => {
                temp.extend_from_within(soff..end);
                return true;
            }
            FRAME => slots_of(frames, root, sstack, sidx as usize),
            INST => Some(&inst.data[..]),
            CONST => const_bytes(rt, yavs_ir::ConstId(sidx)).ok(),
            HEAP => heap.data(sidx),
            SHARED_HEAP => sh.as_ref().and_then(|s| s.heap.as_ref()?.data(sidx)),
            SHARED => sh.as_ref().map(|s| &s.vars[..]),
            _ => None,
        };
        match src.and_then(|b| b.get(soff..end)) {
            Some(b) => {
                temp.extend_from_slice(b);
                true
            }
            None => false,
        }
    }

    /// `Inst::NewFrame`.
    #[inline(never)]
    fn new_frame(
        &mut self,
        dst: Reg,
        desc: Reg,
        dsize: u32,
        data: Reg,
        size: Reg,
        fill: u32,
    ) -> R<()> {
        let size = self.r(size);
        if size > 1 << 32 {
            return Err(runtime_s!("object larger than 4 GiB"));
        }
        if fill != 0 && !size.is_multiple_of(u64::from(fill)) {
            return Err(ice_s!("NewFrame size is not a multiple of its element"));
        }
        let ds = u64::from(dsize);
        // A copy of the descriptor and the value is appended as it is: the zero fill
        // `temp_alloc` would do first is written over at once (#792).
        if fill == 0
            && let Some(at) = self.temp_append_copy(self.r(desc), ds, self.r(data), size)
        {
            let end = u32::try_from(self.temp.len())
                .map_err(|_| runtime_s!("temporary memory exceeds 4 GiB"))?;
            self.top().temp_base = end;
            self.set(dst, at);
            return Ok(());
        }
        let at = self.temp_alloc(ds + size)?;
        if ds != 0 {
            self.copy_mem(at, self.r(desc), ds)?;
        }
        let val = add_addr(at, ds)?;
        if fill == 0 {
            self.copy_mem(val, self.r(data), size)?;
        } else if size != 0 {
            // A pattern of `fill` bytes: the first copy, then doubling; all zero
            // needs nothing (the allocation is zeroed).
            let src = self.rd(self.r(data), u64::from(fill))?;
            if src.iter().any(|&b| b != 0) {
                if let [b] = *src {
                    // One byte (the planes of a logic vector): a memset.
                    self.wr(val, size)?.fill(b);
                } else {
                    let src = src.to_vec();
                    let out = self.wr(val, size)?;
                    for c in out.chunks_exact_mut(fill as usize) {
                        c.copy_from_slice(&src);
                    }
                }
            }
        }
        // The frame's own statements must not free the object.
        let end = u32::try_from(self.temp.len())
            .map_err(|_| runtime_s!("temporary memory exceeds 4 GiB"))?;
        self.top().temp_base = end;
        self.set(dst, at);
        Ok(())
    }

    /// Give `tf` the temporary area's current pointer, length, capacity, recorded roots and
    /// base (what an instruction that only touches the temporary area changes).
    #[inline(always)]
    fn tier_temp(&mut self, tf: &mut crate::TierFrame) {
        tf.temp = self.temp.as_mut_ptr();
        tf.temp_len = self.temp.len() as u64;
        tf.temp_cap = self.temp.capacity().min(u32::MAX as usize) as u64;
        tf.temp_roots = self.troots.len() as u64;
        tf.temp_base = u64::from(self.frames[self.frames.len() - 1].temp_base);
        self.tier_heaps(tf);
    }

    /// `TierFrame::heap_epoch`, `shared_epoch` and `heap_log`: the epochs change when an object
    /// of the process heap or the shared heap is deallocated or edited in place.
    #[inline(always)]
    fn tier_heaps(&self, tf: &mut crate::TierFrame) {
        tf.heap_epoch = self.heap.epoch();
        tf.heap_log[0] = self.heap.log().as_ptr();
        // A `hot` process has no shared state: its line of the machine stays unread.
        let shared = if self.hot {
            None
        } else {
            self.sh.as_ref().and_then(|sh| sh.heap.as_deref())
        };
        (tf.shared_epoch, tf.heap_log[1]) = match shared {
            Some(h) => (h.epoch(), h.log().as_ptr()),
            None => (0, std::ptr::null()),
        };
    }

    /// `Inst::CallNative`: `Flow::Next`, or the flow a native ends with (a VHDL body it hands
    /// the call to, the end of the process).
    fn call_native(
        &mut self,
        prog: &Program,
        f: &Func,
        native: yavs_ir::NativeId,
        args: yavs_ir::Regs,
        dst: Option<Reg>,
        ctx: &mut Option<&mut Ctx<'_>>,
    ) -> R<Flow> {
        self.reclaim(ctx.is_some());
        let n = prog
            .natives
            .get(native)
            .ok_or_else(|| ice_f!("no native {native}"))?;
        let Some(fun) = self.rt.natives.get(native.0 as usize).copied().flatten() else {
            return rt_err_f!(
                "native subprogram `{}` is not implemented in yavs yet",
                n.symbol
            );
        };
        if (self.pkg_init.is_some() || self.init_reports.is_some())
            && self.tainted.is_none()
            && !crate::native::pure(&n.symbol)
        {
            self.tainted = Some(format!("it calls `{}`", n.symbol));
        }
        // The argument cells go through `Machine::cells`: a native call allocates no vector.
        let mut cells = std::mem::take(&mut self.cells);
        cells.clear();
        self.push_args(&mut cells, f, args, None, 0)?;
        let v = fun(self, ctx, n.ty, &cells);
        let v = match v {
            Ok(v) => v,
            Err(e) => {
                self.cells = cells;
                return Err(e);
            }
        };
        if std::mem::take(&mut self.ieee_fallback) {
            let r = self.ieee_body(prog, &n.symbol, &cells, dst);
            self.cells = cells;
            return r;
        }
        self.cells = cells;
        if let Some(d) = dst {
            let v = v.ok_or_else(|| ice_f!("native `{}` returned no value", n.symbol))?;
            self.set(d, v);
        }
        // `std.env.stop`/`finish` end the calling process at once (LRM 16.5): the
        // simulation ends after this cycle, so execute no further, as for a report
        // at the stop severity.
        if std::mem::take(&mut self.ieee_stop) {
            return Ok(Flow::Suspend);
        }
        if n.symbol.starts_with("yavs.env.")
            && (n.symbol.starts_with("yavs.env.stop.") || n.symbol.starts_with("yavs.env.finish."))
        {
            return Ok(Flow::Suspend);
        }
        Ok(Flow::Next)
    }

    /// `Op::Uniform`: the generator in place when both seeds load and are in range, else the
    /// native call (which has changed nothing by then, and reports or faults).
    #[inline(never)]
    fn op_uniform(
        &mut self,
        prog: &Program,
        f: &Func,
        vars: [u64; 3],
        native: u32,
        start: u32,
        ctx: &mut Option<&mut Ctx<'_>>,
    ) -> R<Flow> {
        let [pa, pb, px] = vars;
        // The three variables in one frame (the process's or an enclosing subprogram's): one
        // lookup of its slots instead of five loads and stores.
        let ((sa, ia, oa), (sb, ib, ob), (sx, ix, ox)) = (split(pa), split(pb), split(px));
        if sa == FRAME
            && sb == FRAME
            && sx == FRAME
            && ia == ib
            && ia == ix
            && let Some(m) = self.fslots_mut(ia as usize)
            && let Some(wa) = slot_range(m.len(), oa)
            && let Some(wb) = slot_range(m.len(), ob)
            && let Some(wx) = slot_range(m.len(), ox)
            && let (a, b) = (
                le_bytes(&m[wa.clone()]) as i64,
                le_bytes(&m[wb.clone()]) as i64,
            )
            && a <= 2147483562
            && b <= 2147483398
        {
            let (t1, t2, v) = crate::ieee::uniform_step(a, b);
            m[wa].copy_from_slice(&t1.to_le_bytes());
            m[wb].copy_from_slice(&t2.to_le_bytes());
            m[wx].copy_from_slice(&v.to_le_bytes());
            return Ok(Flow::Next);
        }
        if let (Ok(a), Ok(b)) = (self.load(pa, Width::B8), self.load(pb, Width::B8))
            && (a as i64) <= 2147483562
            && (b as i64) <= 2147483398
        {
            let (t1, t2, v) = crate::ieee::uniform_step(a as i64, b as i64);
            self.store(pa, Width::B8, t1 as u64)?;
            self.store(pb, Width::B8, t2 as u64)?;
            self.store(px, Width::B8, v)?;
            return Ok(Flow::Next);
        }
        let args = yavs_ir::Regs { start, len: 3 };
        self.call_native(prog, f, yavs_ir::NativeId(native), args, None, ctx)
    }

    /// `Op::Image`: `'image` of an integer or enumeration value as a string result.
    #[inline(never)]
    fn op_image(&mut self, ty: u32, v: u64) -> R<u64> {
        crate::native::text_result(self, Some(yavs_ir::TyId(ty)), v, true)
    }

    /// The data address, descriptor address and length of a string result `ret_string` made.
    #[inline(always)]
    fn string_parts(&self, fat: u64) -> R<[u64; 3]> {
        let n = self.load(add_addr(fat, 24)?, Width::B8)?;
        Ok([add_addr(fat, 40)?, add_addr(fat, 16)?, n])
    }

    /// `is_x` of `n` elements at element `soff` of the vector block at `src`, on its planes where
    /// they are; `None` when they are not in one readable block (`Op::IsXSlice`).
    #[inline(always)]
    fn is_x_planes(&self, src: u32, soff: u32, n: u32) -> Option<u64> {
        let (cnt, so) = (self.regs[n as usize], self.regs[soff as usize]);
        if cnt == 0 || cnt > 64 || so % 64 + cnt > 64 {
            return None;
        }
        let sa = add_addr(self.regs[src as usize], so / 64 * 32).ok()?;
        let b = self.rd_hot(sa, 32).ok()?;
        let (s0, mask) = (so % 64, if cnt == 64 { !0 } else { (1u64 << cnt) - 1 });
        let x = (!(plane_word(b, 1) >> s0) | plane_word(b, 3) >> s0) & mask;
        Some(u64::from(x != 0))
    }

    /// `Op::SliceAff`: the ends `k * x` and `k * x + c` and the position of that slice, when both
    /// ends are within 32 bits and inside the ascending descriptor at `dope`.
    #[inline(always)]
    fn slice_aff(&self, x: i64, dope: u64, k: i8, c: i8) -> Option<(i64, i64, u64)> {
        let l = x.checked_mul(i64::from(k))?;
        let r = l + i64::from(c);
        if l < i64::from(i32::MIN) || r > i64::from(i32::MAX) {
            return None;
        }
        let (dl, dr, dd) = self.dim_of(dope, 0).ok()?;
        (dd == 0 && dl <= l && r <= dr).then(|| (l, r, l.wrapping_sub(dl) as u64))
    }

    /// The temporaries of the statement that ended (`Inst::Stmt`).
    #[inline(always)]
    fn end_stmt(&mut self) {
        let base = self.top().temp_base as usize;
        self.temp.truncate(base);
        if !self.troots.is_empty() {
            self.troots.retain(|t| (t.off as usize) < base);
        }
    }

    fn exec_in<const DEBUG: bool>(
        &mut self,
        rt: &Runtime,
        ctx: &mut Option<&mut Ctx<'_>>,
        base: usize,
    ) -> R<Outcome> {
        let prog = &rt.prog;
        if !DEBUG {
            return match rt.tier.get() {
                Some(tier) if self.tiered(rt) => {
                    let r = self.exec_fast::<true, false>(rt, Some(&**tier), None, ctx, base);
                    // Nothing interpreted on this thread until the next run.
                    crate::slot().store(0, Ordering::Relaxed);
                    r
                }
                Some(_) => {
                    let slot = crate::slot();
                    self.exec_fast::<false, true>(rt, None, Some(slot), ctx, base)
                }
                None => self.exec_fast::<false, false>(rt, None, None, ctx, base),
            };
        }
        loop {
            // The top frame changes only at calls and returns: look its function up once.
            let fr = self
                .frames
                .last()
                .ok_or_else(|| ice_s!("no frame to execute"))?;
            let f = prog
                .func(fr.func)
                .ok_or_else(|| ice_f!("no function {}", fr.func))?;
            let pre = rt
                .pre(fr.func, f)
                .ok_or_else(|| ice_f!("no function {}", fr.func))?;
            let mut pc = fr.pc as usize;
            loop {
                // Skip the definitions the frame started with (`pre.rs`).
                pc = *pre
                    .next
                    .get(pc)
                    .ok_or_else(|| ice_f!("pc {pc} beyond `{}`", f.name))?
                    as usize;
                self.top().pc = pc as u32;
                let inst = f
                    .code
                    .get(pc)
                    .ok_or_else(|| ice_f!("pc {pc} beyond `{}`", f.name))?;
                if DEBUG {
                    self.debug_tick(prog, f, pc, inst, ctx)?;
                }
                // The common instructions are executed here; the rest go through `step`.
                macro_rules! jump {
                    ($b:expr) => {{
                        let b = $b;
                        let blk = f.blocks.get(b).ok_or_else(|| ice_f!("no block {b}"))?;
                        pc = blk.start as usize;
                        if self.pause == Some(b) && self.frames.len() == 1 {
                            self.pause = None;
                            return Ok(Outcome::Suspended);
                        }
                        continue;
                    }};
                }
                // The next instruction that is not skipped, with its pc.
                macro_rules! peek {
                    ($p:expr) => {
                        pre.next
                            .get($p)
                            .and_then(|&n| f.code.get(n as usize).map(|i| (n as usize, i)))
                    };
                }
                match *inst {
                    Inst::Br { to } => jump!(to),
                    Inst::CondBr { c, t, f: e } => jump!(if self.r(c) != 0 { t } else { e }),
                    Inst::Stmt => {
                        self.end_stmt();
                        pc += 1;
                        continue;
                    }
                    Inst::Move { dst, src } => {
                        let v = self.r(src);
                        self.set(dst, v);
                        pc += 1;
                        continue;
                    }
                    Inst::TempRoot { addr, ty: None, .. } => {
                        self.fat_root(self.r(addr))?;
                        pc += 1;
                        continue;
                    }
                    Inst::Load { dst, addr, w } => {
                        self.do_load(dst, addr, w)?;
                        pc += 1;
                        // A load, an offset from what it loaded, a load through that: the
                        // field of a descriptor or a record behind a pointer.
                        if !DEBUG
                            && let Some((
                                n1,
                                &Inst::Index {
                                    dst: d1,
                                    base,
                                    pos,
                                    stride,
                                },
                            )) = peek!(pc)
                            && base == dst
                        {
                            self.do_index(d1, base, pos, stride)?;
                            pc = n1 + 1;
                            if let Some((
                                n2,
                                &Inst::Load {
                                    dst: d2,
                                    addr: a2,
                                    w: w2,
                                },
                            )) = peek!(pc)
                                && a2 == d1
                            {
                                self.do_load(d2, a2, w2)?;
                                pc = n2 + 1;
                            }
                        }
                        continue;
                    }
                    Inst::Store { addr, src, w } => {
                        self.do_store(addr, src, w)?;
                        pc += 1;
                        continue;
                    }
                    Inst::Index {
                        dst,
                        base,
                        pos,
                        stride,
                    } => {
                        self.do_index(dst, base, pos, stride)?;
                        pc += 1;
                        if !DEBUG
                            && let Some((n1, &Inst::Load { dst: d1, addr, w })) = peek!(pc)
                            && addr == dst
                        {
                            self.do_load(d1, addr, w)?;
                            pc = n1 + 1;
                        }
                        continue;
                    }
                    Inst::Pos {
                        dst,
                        dope,
                        dim,
                        idx,
                        what,
                    } => {
                        self.do_pos(dst, dope, dim, idx, what)?;
                        pc += 1;
                        continue;
                    }
                    Inst::Bin { op, dst, a, b } => {
                        self.do_bin(op, dst, a, b)?;
                        pc += 1;
                        continue;
                    }
                    Inst::BinRange {
                        op,
                        dst,
                        a,
                        b,
                        lo,
                        hi,
                        what,
                    } => {
                        self.do_bin_range(op, dst, a, b, lo, hi, what)?;
                        pc += 1;
                        continue;
                    }
                    Inst::Cmp { op, dst, a, b } => {
                        let v = cmp(op, self.ri(a).cmp(&self.ri(b)));
                        self.set(dst, v.into());
                        pc += 1;
                        // The branch on the result.
                        if !DEBUG
                            && let Some((_, &Inst::CondBr { c, t, f: e })) = peek!(pc)
                            && c == dst
                        {
                            jump!(if v { t } else { e });
                        }
                        continue;
                    }
                    Inst::Check {
                        kind,
                        v,
                        lo,
                        hi,
                        what,
                    } => {
                        self.do_check(kind, v, lo, hi, what)?;
                        pc += 1;
                        continue;
                    }
                    _ => {}
                }
                match self.step(prog, f, *inst, ctx)? {
                    Flow::Next => pc += 1,
                    Flow::Jump(b) => {
                        let blk = f.blocks.get(b).ok_or_else(|| ice_f!("no block {b}"))?;
                        pc = blk.start as usize;
                        if self.pause == Some(b) && self.frames.len() == 1 {
                            self.pause = None;
                            return Ok(Outcome::Suspended);
                        }
                    }
                    Flow::Call => break,
                    Flow::Ret(v) => {
                        let done = self
                            .pop_frame()
                            .ok_or_else(|| ice_s!("return without frame"))?;
                        if self.frames.len() <= base {
                            return Ok(Outcome::Returned(v));
                        }
                        if let Some(d) = done.ret {
                            self.regs[d.0 as usize] =
                                v.ok_or_else(|| ice_s!("function returned no value"))?;
                        }
                        self.top().pc += 1;
                        break;
                    }
                    Flow::Suspend => {
                        self.top().pc += 1;
                        return Ok(Outcome::Suspended);
                    }
                    Flow::Retier => {
                        return Err(ice_s!("a hand-over of the fast loop under a debugger"));
                    }
                }
            }
        }
    }

    /// The dispatch loop on predecoded code (`pre::Code`, #431), without a debugger or a
    /// compiled tier: the top frame's function runs from its ops until a call, a return or a
    /// suspension, which are handled here as in `exec_in`.
    #[inline(always)]
    fn exec_fast<const TIER: bool, const W: bool>(
        &mut self,
        rt: &Runtime,
        tier: Option<&dyn crate::Tier>,
        slot: Option<&std::sync::atomic::AtomicU64>,
        ctx: &mut Option<&mut Ctx<'_>>,
        base: usize,
    ) -> R<Outcome> {
        let prog = &rt.prog;
        // Nothing interpreted on this thread after the loop (a store in the caller, between the
        // call and the match on the result, made it copy the result).
        let _idle = W.then(|| SlotIdle(slot));
        // `W`: this thread's sampling `slot` names the function the loop goes on in; `run_ops`
        // names it again only every `WATCH` calls and jumps back (#525).
        let tag = u64::from(rt.id) << 32;
        loop {
            let top = self
                .frames
                .len()
                .checked_sub(1)
                .ok_or_else(|| ice_s!("no frame to execute"))?;
            let func = self.frames[top].func;
            if W && let Some(s) = slot {
                s.store(tag | u64::from(func.0), Ordering::Relaxed);
            }
            let f = prog
                .func(func)
                .ok_or_else(|| ice_f!("no function {func}"))?;
            let pre = rt
                .pre(func, f)
                .ok_or_else(|| ice_f!("no function {func}"))?;
            match self.run_ops::<TIER, W>(rt, tier, func, f, &pre.code, top, base, ctx)? {
                Flow::Call => {}
                Flow::Ret(v) => {
                    let done = self
                        .pop_frame()
                        .ok_or_else(|| ice_s!("return without frame"))?;
                    if self.frames.len() <= base {
                        return Ok(Outcome::Returned(v));
                    }
                    if let Some(d) = done.ret {
                        self.regs[d.0 as usize] =
                            v.ok_or_else(|| ice_s!("function returned no value"))?;
                    }
                    self.top().pc += 1;
                }
                Flow::Suspend => {
                    self.top().pc += 1;
                    return Ok(Outcome::Suspended);
                }
                Flow::Retier => {
                    if !TIER && let Some(t) = rt.tier.get() {
                        rt.native.retiers.fetch_add(1, Ordering::Relaxed);
                        return self.exec_fast::<true, false>(rt, Some(&**t), None, ctx, base);
                    }
                }
                // A jump to the block the machine pauses at.
                Flow::Next | Flow::Jump(_) => return Ok(Outcome::Suspended),
            }
        }
    }

    /// Run the ops of frame `top` (function `f`) from its pc. Returns at a call, a return, a
    /// suspension (`Flow::Suspend`) or a pause (`Flow::Next`), with the frame's pc the IR pc
    /// of the instruction; on a fault the frame's pc is the failing op's IR pc.
    ///
    /// With `TIER` (a compiled tier is set, `--tier=llvm|auto`): the thread's sampling slot
    /// names the function being interpreted (`Runtime::sampler`); on entry and after every
    /// jump, if the function has compiled code, that runs from the op's IR pc, and the
    /// interpreter goes on at the op of the IR pc it returns (stepping IR instructions inside
    /// a fused op).
    #[allow(clippy::too_many_arguments)]
    fn run_ops<const TIER: bool, const W: bool>(
        &mut self,
        rt: &Runtime,
        tier: Option<&dyn crate::Tier>,
        func: FuncId,
        f: &Func,
        code: &pre::Code,
        top: usize,
        base: usize,
        ctx: &mut Option<&mut Ctx<'_>>,
    ) -> R<Flow> {
        let (mut func, mut f, mut code, mut top) = (func, f, code, top);
        let mut ops: &[Op] = &code.ops;
        macro_rules! entry {
            ($pc:expr) => {{
                let pc: u32 = $pc;
                code.entry
                    .get(pc as usize)
                    .copied()
                    .filter(|&k| k != pre::NONE)
                    .ok_or_else(|| ice_f!("pc {pc} of `{}` inside a fused operation", f.name))
            }};
        }
        let prog = &*rt.prog;
        // A frame compiled code left inside a fused op (`TIER_UNWIND`) steps up to the next op.
        let pc = self.frames[top].pc as usize;
        let mut k = match code.entry.get(pc) {
            Some(&k) if k != pre::NONE => k as usize,
            _ => match self.op_at(prog, f, code, top, pc, ctx)? {
                Ok((k, _)) => k,
                Err(flow) => return Ok(flow),
            },
        };
        let nat: &[std::sync::atomic::AtomicU8] = &rt.native.func;
        // A tier is set but has no compiled code yet: the loop without its bookkeeping looks
        // for some every `RECHECK` jumps and hands over to the one with (`Flow::Retier`).
        let watch = W && !TIER;
        // Whether compiled code can run now (`NAT_ASK` without code: the tier is asked to
        // compile the function).
        macro_rules! ready {
            () => {
                match nat.get(func.0 as usize) {
                    Some(n) => {
                        let st = n.load(Ordering::Acquire);
                        if st & (NAT_CODE | NAT_ASK) == NAT_ASK {
                            if let Some(t) = tier {
                                n.fetch_and(!NAT_ASK, Ordering::Relaxed);
                                t.hot(func);
                            }
                        }
                        st & NAT_CODE != 0 && !self.is_memo_frame(top)
                    }
                    None => false,
                }
            };
        }
        // `TIER`: this thread's sampling slot names the function being run, and whether it runs
        // compiled (`Runtime::sampler`); compiled code entered from compiled code keeps the
        // slot naming its caller, which calls it in place.
        let slot = if TIER { Some(crate::slot()) } else { None };
        let tag = u64::from(rt.id) << 32;
        // Whether the function has compiled code to run (asked on entry, then every
        // `RECHECK` jumps while it has none: it may be compiled while its loop runs).
        let mut native = false;
        let mut recheck = if watch { WATCH } else { RECHECK };
        // The function is entered: mark it, look for its compiled code.
        macro_rules! bind {
            () => {
                if TIER {
                    native = ready!();
                    if let Some(s) = slot {
                        s.store(tag | u64::from(func.0), Ordering::Relaxed);
                    }
                }
            };
        }
        bind!();
        // Run compiled code from op `k` when the function has some (`native`): at the start,
        // after a jump and after an op compiled code leaves to the interpreter.
        macro_rules! enter {
            () => {
                // Not while the machine pauses at a block (`prestart`): compiled code would jump
                // past it.
                if TIER
                    && native
                    && self.pause.is_none()
                    && let Some(tier) = tier
                {
                    match self.tier_go(tier, rt, func, f, code, top, k, ctx)? {
                        TIER_GO_CALL => return Ok(Flow::Call),
                        TIER_GO_SUSPEND => return Ok(Flow::Suspend),
                        go => {
                            k = (go >> 1) as usize;
                            native = go & 1 != 0;
                        }
                    }
                }
            };
        }
        // The function changed (a call or a return in this loop).
        macro_rules! rebind {
            () => {
                bind!();
                enter!();
            };
        }
        // `watch`: a call of a function with compiled code or one to be compiled is made by the
        // loop with the tier (the frame's pc is the call's); one the tier calls on a native frame
        // is made here (`native_tier`).
        macro_rules! callee_look {
            ($callee:expr, $cpre:expr) => {
                if watch && $cpre.native.load(Ordering::Relaxed) && callee_retier(rt, $callee) {
                    return Ok(Flow::Retier);
                }
            };
        }
        // The callee's parameters may go to registers only and its ops start past the prologue
        // (`Pre::lean`, `Code::skip`): always without the tier, with it for a callee it has not
        // touched. One it flags meanwhile is entered by `enter!` at that op, as the watching
        // loop's hand-over does.
        macro_rules! plain_callee {
            ($cpre:expr) => {
                !TIER || !$cpre.native.load(Ordering::Relaxed)
            };
        }
        // The tier that may call a callee on a native frame (`Tier::call`): the loop's, and in the
        // loop without it (`watch`) the runtime's for a callee the tier has touched.
        macro_rules! native_tier {
            ($cpre:expr) => {
                if TIER {
                    tier
                } else if watch && $cpre.native.load(Ordering::Relaxed) {
                    rt.tier.get().map(|t| &**t)
                } else {
                    None
                }
            };
        }
        // `watch`: every `WATCH` jumps back and calls, name the function being run for the
        // sampler and look for compiled code to hand over to (the next op is `k` of frame `top`).
        macro_rules! tick {
            () => {
                if watch {
                    recheck -= 1;
                    if recheck == 0 {
                        recheck = WATCH;
                        if watch_tick(rt, func, code.at.get(k).copied().unwrap_or(0)) {
                            self.frames[top].pc = code.at.get(k).copied().unwrap_or(0);
                            return Ok(Flow::Retier);
                        }
                    }
                }
            };
        }
        // A jump to op `to`. `watch` counts jumps back only: a run of code without a call, a
        // return or a jump back is short, and forward jumps cost nothing for the tier.
        macro_rules! go {
            ($to:expr) => {{
                if TIER && !native {
                    recheck -= 1;
                    if recheck == 0 {
                        recheck = RECHECK;
                        native = ready!();
                    }
                }
                let to = $to as usize;
                let back = to <= k;
                k = to;
                if back {
                    tick!();
                }
                enter!();
            }};
        }
        // A fault leaves the IR pc of the op that failed in the frame.
        macro_rules! t {
            ($e:expr) => {
                match $e {
                    Ok(v) => v,
                    Err(e) => return Err(self.fault_at(top, code, k, e)),
                }
            };
        }
        // The callers of the frames this loop pushed (innermost last): their function and code,
        // so a return looks neither up. Compiled code leaves this loop when it keeps frames
        // (`TIER_UNWIND`), so the list stays right.
        let mut saved: [Option<(FuncId, &Func, &pre::Code)>; 8] = [None; 8];
        let mut depth = 0usize;
        macro_rules! save_caller {
            () => {
                if depth < saved.len() {
                    saved[depth] = Some((func, f, code));
                }
                depth += 1;
            };
        }
        // The frame's slots (frame 0's are `root`, `slots_of`).
        macro_rules! fslots {
            () => {
                if top == 0 {
                    Some(&self.root[..])
                } else {
                    self.fslots(top)
                }
            };
        }
        macro_rules! fslots_mut {
            () => {
                if top == 0 {
                    Some(&mut self.root[..])
                } else {
                    self.fslots_mut(top)
                }
            };
        }
        // The eight bytes at `off` of the frame's slots.
        macro_rules! slot8 {
            ($off:expr) => {{
                let off: u32 = $off;
                match fslots!()
                    .and_then(|s| s.get(off as usize..))
                    .and_then(|s| s.first_chunk::<8>())
                {
                    Some(b) => u64::from_le_bytes(*b),
                    None => t!(mk_addr(FRAME, top as u32, off.into())
                        .and_then(|a| self.load_slow(a, Width::B8))),
                }
            }};
        }
        // Store register `src` to the frame's slot at `off`.
        macro_rules! store_slot {
            ($src:expr, $off:expr, $w:expr) => {{
                let (off, w): (u32, Width) = ($off, $w);
                let v = self.regs[$src as usize];
                let o = off as usize;
                if w == Width::B8
                    && let Some(b) = fslots_mut!()
                        .and_then(|s| s.get_mut(o..))
                        .and_then(|s| s.first_chunk_mut::<8>())
                {
                    *b = v.to_le_bytes();
                } else {
                    let n = w.bytes() as usize;
                    match fslots_mut!().and_then(|s| s.get_mut(o..o + n)) {
                        Some(b) => put_le(b, v),
                        None => t!(mk_addr(FRAME, top as u32, off.into())
                            .and_then(|a| self.store_slow(a, w, v))),
                    }
                }
            }};
        }
        // The message of the `Deref` at op `$j` (a fused op, on its failing path).
        macro_rules! deref_what {
            ($j:expr) => {
                match code.at.get($j).and_then(|&p| f.code.get(p as usize)) {
                    Some(&Inst::Deref { what, .. }) => what,
                    _ => yavs_ir::CheckMsgId(0),
                }
            };
        }
        // The message of the `Pos` the current op starts with.
        macro_rules! pos_what {
            () => {
                match code.at.get(k).and_then(|&p| f.code.get(p as usize)) {
                    Some(&Inst::Pos { what, .. }) => what,
                    _ => yavs_ir::CheckMsgId(0),
                }
            };
        }
        // A process resumes at the jump back to its first statement: go there at once.
        if let Some(&(Op::Br { to } | Op::BrBody { to, .. })) = ops.get(k)
            && self.pause.is_none()
        {
            k = to as usize;
        }
        // `Op::Call`, and `Op::CallM` for a callee the memo of pure calls may answer (`$memo`).
        macro_rules! call_arm {
            ($callee:expr, $start:expr, $len:expr, $dst:expr, $memo:expr) => {{
                let (callee, start, len, dst) = ($callee, $start, $len, $dst);
                self.frames[top].pc = code.at.get(k).copied().unwrap_or(0);
                let args = yavs_ir::Regs { start, len };
                let dst = (dst != pre::NONE).then_some(Reg(dst));
                let cid = FuncId(callee);
                let cf = prog
                    .func(cid)
                    .ok_or_else(|| ice_f!("call of missing function {cid}"))?;
                let cpre = rt.pre(cid, cf).ok_or_else(|| ice_f!("no function {cid}"))?;
                callee_look!(callee, cpre);
                // The memo of pure calls answers a repeated call before the call is made, by the
                // compiled tier or not (out of line: it keeps this loop's frame as it was).
                if $memo
                    && !cpre.memo_off.load(Ordering::Relaxed)
                    && let Some(sig) = prog.memo_sig(cid)
                {
                    let tier = native_tier!(cpre);
                    match t!(self.memo_call(prog, rt, tier, f, cid, cf, sig, args, dst, ctx)) {
                        crate::pmemo::Memoed::No => {}
                        crate::pmemo::Memoed::Hit => {
                            k += 1;
                            enter!();
                            continue;
                        }
                        crate::pmemo::Memoed::Tiered => {
                            k = entry!(self.frames[top].pc + 1)? as usize;
                            continue;
                        }
                        crate::pmemo::Memoed::Flow(flow) => return Ok(flow),
                    }
                }
                // A function the compiled tier calls on a native frame (`Tier::call`).
                if let Some(t) = native_tier!(cpre)
                    && rt.native.callable(callee)
                    && let Some(flow) = self.tier_call(t, rt, f, cid, cf, args, dst, None, ctx)?
                {
                    if let Flow::Next = flow {
                        k = entry!(self.frames[top].pc + 1)? as usize;
                        continue;
                    }
                    return Ok(flow);
                }
                let plain = plain_callee!(cpre);
                if plain && cpre.lean {
                    let regs = f
                        .regs(args)
                        .ok_or_else(|| ice_s!("operand list out of range"))?;
                    self.call_lean(cf, cid, cpre, regs, dst, None)?;
                } else {
                    self.call_pre(f, cf, cid, cpre, args, dst, None)?;
                }
                save_caller!();
                // The callee's ops go on in this loop.
                top += 1;
                func = cid;
                f = cf;
                code = &cpre.code;
                ops = &code.ops;
                k = match code.first {
                    pre::NONE => entry!(0)? as usize,
                    // The parameters are in their registers (`call_pre`): not loaded again.
                    _ if plain && code.skip != pre::NONE => code.skip as usize,
                    first => first as usize,
                };
                rebind!();
                tick!();
            }};
        }
        enter!();
        loop {
            // Compiled out of a release build: even an empty call moved the loop's code (+9 percent of
            // the instructions of `calls`).
            #[cfg(any(debug_assertions, feature = "op-census"))]
            crate::census::tick(ops, k, &f.code, &f.name);
            match ops[k] {
                Op::Step(pc) => {
                    let inst = *f
                        .code
                        .get(pc as usize)
                        .ok_or_else(|| ice_f!("pc {pc} beyond `{}`", f.name))?;
                    self.frames[top].pc = pc;
                    match self.step_out(prog, f, inst, ctx)? {
                        Flow::Next => {
                            k += 1;
                            enter!();
                        }
                        Flow::Jump(b) => {
                            let blk = f.blocks.get(b).ok_or_else(|| ice_f!("no block {b}"))?;
                            if self.pause == Some(b) && self.frames.len() == 1 {
                                self.pause = None;
                                return Ok(Flow::Next);
                            }
                            go!(entry!(blk.start)?);
                        }
                        flow => {
                            return Ok(flow);
                        }
                    }
                }
                Op::Case { v, tab } => {
                    let x = self.regs[v as usize] as i64;
                    let c = code
                        .cases
                        .get(tab as usize)
                        .ok_or_else(|| ice_f!("no case table {tab}"))?;
                    let i = c.arms.partition_point(|a| a.1 < x);
                    go!(match c.arms.get(i) {
                        Some(&(lo, _, b)) if lo <= x => b,
                        _ => c.default,
                    });
                }
                Op::Stmt => {
                    self.end_stmt();
                    k += 1;
                }
                Op::Cover { item } => {
                    t!(self.op_cover(item));
                    k += 1;
                }
                Op::CoverIn {
                    dst,
                    acc,
                    src,
                    shift,
                    ulogic,
                } => {
                    self.op_cover_in(dst, acc, src, shift, ulogic);
                    k += 1;
                }
                Op::CoverCond { cond, state } => {
                    t!(self.op_cover_cond(prog, cond, state));
                    k += 1;
                }
                Op::Move { dst, src } => {
                    self.regs[dst as usize] = self.regs[src as usize];
                    k += 1;
                }
                Op::Select { dst, c, a, b } => {
                    let from = if self.regs[c as usize] != 0 { a } else { b };
                    self.regs[dst as usize] = self.regs[from as usize];
                    k += 1;
                }
                Op::Load { dst, addr, w } => {
                    self.regs[dst as usize] = t!(self.load(self.regs[addr as usize], w));
                    k += 1;
                }
                Op::Store { addr, src, w } => {
                    t!(self.store(self.regs[addr as usize], w, self.regs[src as usize]));
                    k += 1;
                }
                Op::LoadSlot { dst, off, w } => {
                    let n = w.bytes() as usize;
                    let o = off as usize;
                    self.regs[dst as usize] = match fslots!().and_then(|s| s.get(o..o + n)) {
                        Some(b) => cell_w(b, w),
                        None => t!(mk_addr(FRAME, top as u32, off.into())
                            .and_then(|a| self.load_slow(a, w))),
                    };
                    k += 1;
                }
                Op::LoadSlot8 { dst, off } => {
                    self.regs[dst as usize] = slot8!(off);
                    k += 1;
                }
                Op::StmtLoadSlot8 { dst, off } => {
                    self.end_stmt();
                    self.regs[dst as usize] = slot8!(off);
                    k += 1;
                }
                Op::LoadSlot8x2 { d1, o1, d2, o2 } => {
                    self.regs[d1 as usize] = slot8!(o1);
                    self.regs[d2 as usize] = slot8!(o2);
                    k += 1;
                }
                Op::LoadSlot8CmpBr {
                    ld,
                    off,
                    other,
                    t,
                    f: e,
                    m,
                    side,
                } => {
                    let v = slot8!(off);
                    self.regs[ld as usize] = v;
                    let o = self.regs[other as usize];
                    let (x, y) = if side == 0 { (v, o) } else { (o, v) };
                    go!(if pre::cmp_holds(m, x as i64, y as i64) {
                        t
                    } else {
                        e
                    });
                }
                Op::StoreSlotStmt { src, off, w } => {
                    store_slot!(src, off, w);
                    self.end_stmt();
                    k += 1;
                }
                Op::StoreSlotBr { src, off, to, w } => {
                    store_slot!(src, off, w);
                    go!(to);
                }
                Op::StoreSlotIndexImm {
                    src,
                    off,
                    dst,
                    base,
                    add,
                    w,
                } => {
                    store_slot!(src, off, w);
                    self.regs[dst as usize] = t!(add_addr(self.regs[base as usize], add.into()));
                    k += 1;
                }
                Op::MoveBr { dst, src, to } => {
                    self.regs[dst as usize] = self.regs[src as usize];
                    go!(to);
                }
                Op::PosIndex {
                    dst,
                    dope,
                    idx,
                    base,
                    stride,
                    dim,
                } => {
                    t!(self.op_pos_index(f, code, k, dst, dope, idx, base, stride, dim));
                    k += 1;
                }
                Op::PosIndexLoad {
                    dst,
                    dope,
                    idx,
                    base,
                    stride,
                    w,
                    dim,
                } => {
                    let p = t!(self.pos_of_with(
                        self.regs[dope as usize],
                        dim,
                        self.regs[idx as usize] as i64,
                        || pos_what!()
                    ));
                    self.regs[dst as usize] =
                        t!(self.load_idx(self.regs[base as usize], p, stride, w));
                    k += 1;
                }
                Op::PosIndexLoadIndex {
                    dst,
                    dope,
                    idx,
                    base,
                    dst2,
                    base2,
                    stride,
                    stride2,
                    w,
                    dim,
                } => {
                    let p = t!(self.pos_of_with(
                        self.regs[dope as usize],
                        dim,
                        self.regs[idx as usize] as i64,
                        || pos_what!()
                    ));
                    self.regs[dst as usize] =
                        t!(self.load_idx(self.regs[base as usize], p, stride.into(), w));
                    k += 1;
                    self.regs[dst2 as usize] = t!(index_addr(
                        self.regs[base2 as usize],
                        self.regs[dst as usize],
                        stride2.into()
                    ));
                    k += 1;
                }
                Op::PosIndexStore {
                    src,
                    dope,
                    idx,
                    base,
                    stride,
                    w,
                    dim,
                } => {
                    t!(self.op_pos_index_store(f, code, k, src, dope, idx, base, stride, w, dim));
                    k += 1;
                }
                Op::IndexLoad {
                    dst,
                    base,
                    pos,
                    stride,
                    w,
                } => {
                    self.regs[dst as usize] = t!(self.load_idx(
                        self.regs[base as usize],
                        self.regs[pos as usize],
                        stride,
                        w
                    ));
                    k += 1;
                }
                Op::IndexStore {
                    src,
                    base,
                    pos,
                    stride,
                    w,
                } => {
                    t!(self.op_index_store(src, base, pos, stride, w));
                    k += 1;
                }
                Op::StoreSlot { src, off, w } => {
                    store_slot!(src, off, w);
                    k += 1;
                }
                Op::LoadInst { dst, off, w } => {
                    let n = w.bytes() as usize;
                    let o = off as usize;
                    self.regs[dst as usize] = match self.inst.data.get(o..o + n) {
                        Some(b) => cell_w(b, w),
                        None => t!(mk_addr(INST, 0, off.into()).and_then(|a| self.load(a, w))),
                    };
                    k += 1;
                }
                Op::LoadPair { d1, d2, base, off } => {
                    let a = self.regs[base as usize];
                    let (sp, idx, o) = split(a);
                    let o = o + u64::from(off);
                    if let Some(b) = self
                        .mem_fast(sp, idx)
                        .and_then(|m| m.get(o as usize..o as usize + 16))
                        .and_then(|b| b.as_chunks::<8>().0.first_chunk::<2>())
                    {
                        let (x, y) = (u64::from_le_bytes(b[0]), u64::from_le_bytes(b[1]));
                        self.regs[d1 as usize] = x;
                        self.regs[d2 as usize] = y;
                    } else {
                        let x = t!(add_addr(a, off.into()).and_then(|a| self.load(a, Width::B8)));
                        self.regs[d1 as usize] = x;
                        let y =
                            add_addr(a, u64::from(off) + 8).and_then(|a| self.load(a, Width::B8));
                        self.regs[d2 as usize] = match y {
                            Ok(y) => y,
                            Err(e) => return Err(self.fault_at(top, code, k + 1, e)),
                        };
                    }
                    k += 2;
                }
                Op::LoadPkg { dst, pkg, off, w } => {
                    let n = w.bytes() as usize;
                    let o = off as usize;
                    let mem = match &self.pkg_init {
                        Some((p, data)) if *p == pkg => Some(&data[..]),
                        _ => self.rt.pkgs.get(pkg as usize).map(|p| &p.data[..]),
                    };
                    self.regs[dst as usize] = match mem.and_then(|m| m.get(o..o + n)) {
                        Some(b) => cell_w(b, w),
                        None => t!(mk_addr(PKG, pkg, off.into()).and_then(|a| self.load(a, w))),
                    };
                    k += 1;
                }
                Op::LoadOff { dst, base, off, w } => {
                    let a = t!(add_addr(self.regs[base as usize], off.into()));
                    self.regs[dst as usize] = t!(self.load(a, w));
                    k += 1;
                }
                Op::StoreOff { base, off, src, w } => {
                    let a = t!(add_addr(self.regs[base as usize], off.into()));
                    t!(self.store(a, w, self.regs[src as usize]));
                    k += 1;
                }
                Op::Index {
                    dst,
                    base,
                    pos,
                    stride,
                } => {
                    self.regs[dst as usize] = t!(index_addr(
                        self.regs[base as usize],
                        self.regs[pos as usize],
                        stride
                    ));
                    k += 1;
                }
                Op::IndexImm { dst, base, off } => {
                    self.regs[dst as usize] = t!(add_addr(self.regs[base as usize], off.into()));
                    k += 1;
                }
                Op::Pos {
                    dst,
                    dope,
                    idx,
                    what,
                    dim,
                } => {
                    self.regs[dst as usize] = t!(self.pos_of(
                        self.regs[dope as usize],
                        dim,
                        self.regs[idx as usize] as i64,
                        what
                    ));
                    k += 1;
                }
                Op::SlicePos(pc) => {
                    let inst = f.code[pc as usize];
                    let p = t!(self.slice_pos(&inst));
                    if let Inst::SlicePos { dst, .. } = inst {
                        self.regs[dst.0 as usize] = p;
                    }
                    k += 1;
                }
                Op::Bin { op, dst, a, b } => {
                    self.regs[dst as usize] = t!(bin(
                        op,
                        self.regs[a as usize] as i64,
                        self.regs[b as usize] as i64
                    ));
                    k += 1;
                }
                Op::Aff1 { dst, a, ca, c0 } => {
                    let x = self.regs[a as usize] as i64;
                    if aff_ok(x) {
                        self.regs[dst as usize] = (i64::from(c0) + i64::from(ca) * x) as u64;
                    } else {
                        self.aff_replay(prog, f, code, top, k, ctx)?;
                    }
                    k += 1;
                }
                Op::Aff2 {
                    dst,
                    a,
                    b,
                    ca,
                    cb,
                    c0,
                } => {
                    let (x, y) = (self.regs[a as usize] as i64, self.regs[b as usize] as i64);
                    if aff_ok(x) && aff_ok(y) {
                        self.regs[dst as usize] =
                            (i64::from(c0) + i64::from(ca) * x + i64::from(cb) * y) as u64;
                    } else {
                        self.aff_replay(prog, f, code, top, k, ctx)?;
                    }
                    k += 1;
                }
                Op::AffP {
                    dst,
                    dope,
                    idx,
                    cp,
                    c0,
                } => {
                    match self.pos_first(self.regs[dope as usize], self.regs[idx as usize] as i64) {
                        Some(p) if aff_ok(p) => {
                            self.regs[dst as usize] = (i64::from(c0) + i64::from(cp) * p) as u64;
                        }
                        _ => {
                            self.aff_replay(prog, f, code, top, k, ctx)?;
                        }
                    }
                    k += 1;
                }
                Op::AffL { dst, dope, cl, c0 } => {
                    match self.dope_len_of(self.regs[dope as usize]) {
                        Some(n) if aff_ok(n) => {
                            self.regs[dst as usize] = (i64::from(c0) + i64::from(cl) * n) as u64;
                        }
                        _ => {
                            self.aff_replay(prog, f, code, top, k, ctx)?;
                        }
                    }
                    k += 1;
                }
                Op::AffPL {
                    dst,
                    dope,
                    idx,
                    cp,
                    cl,
                    c0,
                } => {
                    match self.pos_len(self.regs[dope as usize], self.regs[idx as usize] as i64) {
                        Some((p, n)) if aff_ok(p) && aff_ok(n) => {
                            self.regs[dst as usize] =
                                (i64::from(c0) + i64::from(cp) * p + i64::from(cl) * n) as u64;
                        }
                        _ => {
                            self.aff_replay(prog, f, code, top, k, ctx)?;
                        }
                    }
                    k += 1;
                }
                Op::AffPR {
                    dst,
                    dope,
                    idx,
                    a,
                    cp,
                    ca,
                    c0,
                } => {
                    let x = self.regs[a as usize] as i64;
                    match self.pos_len(self.regs[dope as usize], self.regs[idx as usize] as i64) {
                        Some((p, _)) if aff_ok(p) && aff_ok(x) => {
                            self.regs[dst as usize] =
                                (i64::from(c0) + i64::from(cp) * p + i64::from(ca) * x) as u64;
                        }
                        _ => {
                            self.aff_replay(prog, f, code, top, k, ctx)?;
                        }
                    }
                    k += 1;
                }
                Op::WaveBuf { w, size, time, e } => {
                    t!(self.op_wave_buf(w, size, time, e));
                    k += 1;
                }
                Op::ReadPL {
                    dst,
                    dope,
                    idx,
                    sig,
                    cp,
                    cl,
                    c0,
                } => {
                    let off = match self
                        .pos_len(self.regs[dope as usize], self.regs[idx as usize] as i64)
                    {
                        Some((p, n)) if aff_ok(p) && aff_ok(n) => {
                            i64::from(c0) + i64::from(cp) * p + i64::from(cl) * n
                        }
                        _ => self.aff_replay(prog, f, code, top, k, ctx)? as i64,
                    };
                    let (sr, ty) = t!(self.sig_elem(sig.into(), off as u64));
                    self.regs[dst as usize] = t!(Self::read_elem(sr, ty, ctx));
                    k += 1;
                    enter!();
                }
                Op::ReadPR {
                    dst,
                    dope,
                    idx,
                    a,
                    sig,
                    cp,
                    c0,
                } => {
                    let x = self.regs[a as usize] as i64;
                    let off = match self
                        .pos_len(self.regs[dope as usize], self.regs[idx as usize] as i64)
                    {
                        Some((p, _)) if aff_ok(p) && aff_ok(x) => {
                            i64::from(c0) + i64::from(cp) * p + x
                        }
                        _ => self.aff_replay(prog, f, code, top, k, ctx)? as i64,
                    };
                    let (sr, ty) = t!(self.sig_elem(sig.into(), off as u64));
                    self.regs[dst as usize] = t!(Self::read_elem(sr, ty, ctx));
                    k += 1;
                    enter!();
                }
                Op::BinRangeK {
                    op,
                    dst,
                    a,
                    b,
                    lo,
                    hi,
                } => {
                    let (x, y) = (self.regs[a as usize] as i64, self.regs[b as usize] as i64);
                    let (l, h) = (i64::from(lo), i64::from(hi));
                    self.regs[dst as usize] = match fast_op(op, x, y) {
                        Some(v) if l <= v && v <= h => v as u64,
                        _ => t!(self.bin_range_slow(op, x, y, l, h, || {
                            match code.at.get(k).and_then(|&p| f.code.get(p as usize)) {
                                Some(&Inst::BinRange { what, .. }) => what,
                                _ => yavs_ir::CheckMsgId(0),
                            }
                        })),
                    };
                    k += 1;
                }
                Op::BinRange {
                    op,
                    dst,
                    a,
                    b,
                    lo,
                    hi,
                } => {
                    t!(self.op_bin_range(f, code, k, op, dst, a, b, lo, hi));
                    k += 1;
                }
                Op::Cmp { m, dst, a, b } => {
                    let v = pre::cmp_holds(
                        m,
                        self.regs[a as usize] as i64,
                        self.regs[b as usize] as i64,
                    );
                    self.regs[dst as usize] = v.into();
                    k += 1;
                }
                Op::CmpBr {
                    m,
                    dst,
                    a,
                    b,
                    t,
                    f: e,
                } => {
                    let v = pre::cmp_holds(
                        m,
                        self.regs[a as usize] as i64,
                        self.regs[b as usize] as i64,
                    );
                    self.regs[dst as usize] = v.into();
                    go!(if v { t } else { e });
                }
                Op::CmpBrStep {
                    m,
                    delta,
                    a,
                    b,
                    dst,
                    src,
                    t,
                    head,
                } => {
                    if pre::cmp_holds(
                        m,
                        self.regs[a as usize] as i64,
                        self.regs[b as usize] as i64,
                    ) {
                        go!(t);
                    } else {
                        match bin(IntOp::Add, self.regs[src as usize] as i64, i64::from(delta)) {
                            Ok(v) => self.regs[dst as usize] = v,
                            Err(e) => {
                                self.frames[top].pc = code.at.get(k + 1).copied().unwrap_or(0);
                                return Err(e);
                            }
                        }
                        go!(head);
                    }
                }
                Op::PosIndexLoadCmpBr {
                    m,
                    w,
                    dim,
                    dst,
                    dope,
                    idx,
                    base,
                    b,
                    stride,
                    t,
                    f: e,
                } => {
                    if dim & pre::SCAN != 0
                        && let Some(Op::CmpBrStep {
                            m: m2,
                            delta,
                            a: sa,
                            b: sb,
                            t: end,
                            ..
                        }) = ops.get(e as usize).copied()
                    {
                        // A search loop: run its iterations here, off the dispatch.
                        let other = if sa == idx { sb } else { sa };
                        let mut limit = if watch { WATCH_CHUNK } else { 1 << 16 };
                        if TIER && !native {
                            limit = limit.min(recheck.saturating_sub(1));
                        }
                        let sc = self.scan_loop(
                            &ScanLoop {
                                dope: self.regs[dope as usize],
                                dim: dim & !pre::SCAN,
                                base: self.regs[base as usize],
                                stride: u32::from(stride),
                                w,
                                m,
                                m2,
                                idx_left: sa == idx,
                                delta: i64::from(delta),
                            },
                            self.regs[idx as usize] as i64,
                            self.regs[b as usize] as i64,
                            self.regs[other as usize] as i64,
                            limit,
                        );
                        if sc.iters > 0 {
                            self.regs[dst as usize] = sc.ch;
                            self.regs[idx as usize] = sc.i as u64;
                            if TIER && !native {
                                recheck -= sc.iters.min(recheck - 1);
                            }
                            match sc.hit {
                                1 => go!(t),
                                2 => go!(end),
                                // The next iteration is the generic one (a fault, a long loop).
                                _ => {}
                            }
                            continue;
                        }
                    }
                    let dim = dim & !pre::SCAN;
                    let p = t!(self.pos_of_with(
                        self.regs[dope as usize],
                        dim,
                        self.regs[idx as usize] as i64,
                        || pos_what!()
                    ));
                    let a = t!(index_addr(self.regs[base as usize], p, u32::from(stride)));
                    let v = t!(self.load(a, w));
                    self.regs[dst as usize] = v;
                    go!(
                        if pre::cmp_holds(m, v as i64, self.regs[b as usize] as i64) {
                            t
                        } else {
                            e
                        }
                    );
                }
                Op::Bool { op, dst, a, b } => {
                    let (x, y) = (self.regs[a as usize] != 0, self.regs[b as usize] != 0);
                    self.regs[dst as usize] = match op {
                        BoolOp::And => x && y,
                        BoolOp::Or => x || y,
                        BoolOp::Xor => x != y,
                        BoolOp::Not => !x,
                    }
                    .into();
                    k += 1;
                    enter!();
                }
                Op::LCopy {
                    dst,
                    doff,
                    src,
                    soff,
                    n,
                } => {
                    t!(self.op_lcopy(dst, doff, src, soff, n));
                    k += 1;
                    enter!();
                }
                Op::LEq { dst, a, b, n } => {
                    self.regs[dst as usize] = t!(self.op_leq(a, b, n));
                    k += 1;
                }
                Op::Cat => {
                    let pc = code.at.get(k).copied().unwrap_or(0);
                    let Some(&Inst::Cat {
                        dst,
                        n,
                        m,
                        parts,
                        scalars,
                        bias,
                        logic,
                        w,
                    }) = f.code.get(pc as usize)
                    else {
                        return Err(ice_f!("pc {pc} is not a concatenation"));
                    };
                    let ps = t!(f.regs(parts).ok_or_else(|| ice_s!("operand list")));
                    t!(self.cat(
                        self.regs[dst.0 as usize],
                        0,
                        self.regs[n.0 as usize],
                        self.regs[m.0 as usize],
                        ps,
                        scalars,
                        bias,
                        logic,
                        w
                    ));
                    k += 1;
                    enter!();
                }
                Op::SigReadVec { dst, sig } => {
                    if self.is_multi(Reg(sig)) {
                        // A reference of several parts: the step.
                        let pc = code.at.get(k).copied().unwrap_or(0);
                        let inst = *f
                            .code
                            .get(pc as usize)
                            .ok_or_else(|| ice_f!("pc {pc} beyond `{}`", f.name))?;
                        self.frames[top].pc = pc;
                        match self.step_out(prog, f, inst, ctx)? {
                            Flow::Next => {}
                            flow => return Ok(flow),
                        }
                    } else {
                        t!(self.sig_read_vec(Reg(dst), Reg(sig), ctx));
                    }
                    k += 1;
                    enter!();
                }
                Op::Check {
                    kind,
                    v,
                    lo,
                    hi,
                    what,
                } => {
                    t!(self.check_val(
                        kind,
                        self.regs[v as usize] as i64,
                        self.regs[lo as usize] as i64,
                        self.regs[hi as usize] as i64,
                        what
                    ));
                    k += 1;
                }
                Op::CheckK {
                    kind,
                    v,
                    lo,
                    hi,
                    what,
                } => {
                    let x = self.regs[v as usize] as i64;
                    if x < i64::from(lo) || x > i64::from(hi) {
                        t!(self.check_fail(kind, x, lo.into(), hi.into(), what));
                    }
                    k += 1;
                }
                Op::Layout { dst, ty, desc } => {
                    self.regs[dst as usize] =
                        t!(self.layout_of(prog, TyId(ty), self.regs[desc as usize]));
                    k += 1;
                }
                Op::LayoutWord {
                    dst,
                    ty,
                    desc,
                    k: w,
                } => {
                    self.regs[dst as usize] =
                        t!(self.layout_word(prog, TyId(ty), self.regs[desc as usize], w));
                    k += 1;
                }
                Op::FieldOf {
                    dst,
                    base,
                    ty,
                    desc,
                    n,
                } => {
                    let off = t!(self.layout_word(prog, TyId(ty), self.regs[desc as usize], 2 + n));
                    self.regs[dst as usize] = t!(add_addr(self.regs[base as usize], off));
                    k += 1;
                }
                Op::Br { to } => go!(to),
                Op::Loop { id, to } => {
                    // Iterations the kernel may run before the loop goes back to counting.
                    let mut limit = if watch {
                        u64::from(WATCH_CHUNK)
                    } else {
                        i64::MAX as u64
                    };
                    if TIER && !native {
                        limit = limit.min(u64::from(recheck.saturating_sub(1)));
                    }
                    let n = t!(self.op_loop(code, top, id, limit));
                    if n > 0 && TIER && !native {
                        recheck -= (n as u32).min(recheck - 1);
                    }
                    go!(to);
                }
                Op::BrBody { to, block } => {
                    if self.pause == Some(BlockId(block)) && self.frames.len() == 1 {
                        // Stopped before the jump, which runs again on resumption.
                        self.frames[top].pc = code.at.get(k).copied().unwrap_or(0);
                        self.pause = None;
                        return Ok(Flow::Next);
                    }
                    go!(to);
                }
                Op::SigVal {
                    dst,
                    sig,
                    offset,
                    len,
                } => {
                    let (a, b) = t!(self.sig_val(
                        sig,
                        self.regs[offset as usize] as i64,
                        self.regs[len as usize] as i64
                    ));
                    self.regs[dst as usize] = a;
                    self.regs[dst as usize + 1] = b;
                    k += 1;
                    enter!();
                }
                Op::DopeLen { dst, dope, dim } => {
                    self.regs[dst as usize] = t!(self.dope_len(self.regs[dope as usize], dim));
                    k += 1;
                }
                Op::Call {
                    func: callee,
                    start,
                    len,
                    dst,
                } => call_arm!(callee, start, len, dst, false),
                Op::CallM {
                    func: callee,
                    start,
                    len,
                    dst,
                } => call_arm!(callee, start, len, dst, true),
                Op::CallProt {
                    func: callee,
                    obj,
                    start,
                    len,
                    dst,
                } => {
                    self.frames[top].pc = code.at.get(k).copied().unwrap_or(0);
                    t!(self.shared());
                    let h = self.regs[obj as usize];
                    let args = yavs_ir::Regs { start, len };
                    let dst = (dst != pre::NONE).then_some(Reg(dst));
                    let cid = FuncId(callee);
                    let cf = prog
                        .func(cid)
                        .ok_or_else(|| ice_f!("call of missing function {cid}"))?;
                    let cpre = rt.pre(cid, cf).ok_or_else(|| ice_f!("no function {cid}"))?;
                    callee_look!(callee, cpre);
                    // A function the compiled tier calls on a native frame (`Tier::call`).
                    if let Some(t) = native_tier!(cpre)
                        && rt.native.callable(callee)
                        && let Some(flow) =
                            self.tier_call(t, rt, f, cid, cf, args, dst, Some(h), ctx)?
                    {
                        if let Flow::Next = flow {
                            k = entry!(self.frames[top].pc + 1)? as usize;
                            continue;
                        }
                        return Ok(flow);
                    }
                    let plain = plain_callee!(cpre);
                    if plain && cpre.lean {
                        let regs = f
                            .regs(args)
                            .ok_or_else(|| ice_s!("operand list out of range"))?;
                        self.call_lean(cf, cid, cpre, regs, dst, Some(h))?;
                    } else {
                        self.call_pre(f, cf, cid, cpre, args, dst, Some(h))?;
                    }
                    self.top().shared_alloc |= heap::tag_of(h) == heap::SHARED;
                    save_caller!();
                    top += 1;
                    func = cid;
                    f = cf;
                    code = &cpre.code;
                    ops = &code.ops;
                    k = match code.first {
                        pre::NONE => entry!(0)? as usize,
                        // The parameters are in their registers (`call_pre`): not loaded again.
                        _ if plain && code.skip != pre::NONE => code.skip as usize,
                        first => first as usize,
                    };
                    rebind!();
                    tick!();
                }
                Op::Ret { v } => {
                    let v = (v != pre::NONE).then(|| self.regs[v as usize]);
                    // Back into the caller, unless this activation began here.
                    if top <= base {
                        return Ok(Flow::Ret(v));
                    }
                    let ret = self
                        .pop_ret()
                        .ok_or_else(|| ice_s!("return without frame"))?;
                    top -= 1;
                    if let Some(d) = ret {
                        self.regs[d.0 as usize] =
                            v.ok_or_else(|| ice_s!("function returned no value"))?;
                    }
                    let caller = &mut self.frames[top];
                    caller.pc += 1;
                    let pc = caller.pc;
                    // The caller this loop entered the callee from: no lookups.
                    if depth > 0 {
                        depth -= 1;
                        if let Some(&Some((sf, sff, sc))) = saved.get(depth) {
                            (func, f, code) = (sf, sff, sc);
                            ops = &code.ops;
                            k = entry!(pc)? as usize;
                            if TIER {
                                rebind!();
                            }
                            continue;
                        }
                    }
                    func = caller.func;
                    f = prog
                        .func(func)
                        .ok_or_else(|| ice_f!("no function {func}"))?;
                    code = &rt
                        .pre(func, f)
                        .ok_or_else(|| ice_f!("no function {func}"))?
                        .code;
                    ops = &code.ops;
                    k = entry!(pc)? as usize;
                    rebind!();
                }
                Op::SigReadScalar { dst, sig } => {
                    self.regs[dst as usize] = t!(self.sig_read_scalar(Reg(sig), ctx));
                    k += 1;
                    enter!();
                }
                Op::Drive {
                    driver,
                    target,
                    wave,
                    n,
                    reject,
                    inertial,
                } => {
                    t!(self.op_drive(driver, target, wave, n, reject, inertial, ctx));
                    k += 1;
                    enter!();
                }
                Op::CopyDrive {
                    driver,
                    target,
                    src,
                    bytes,
                    time,
                    size,
                    w,
                    n,
                    inertial,
                } => {
                    let mode = if inertial {
                        DriveMode::Inertial
                    } else {
                        DriveMode::Transport
                    };
                    let tg = Reg(target.into());
                    let multi = self.is_multi(tg);
                    let nbytes = self.regs[bytes as usize];
                    let mut done = false;
                    if !multi {
                        let d = t!(self.driver(driver.into()));
                        let (sr, ty) = t!(self.sigref(tg));
                        let len = sr.range.map_or(0, |r| r.len);
                        // The value as the waveform's planes would hold it: one block per 64
                        // elements, whole.
                        if ty.elem == Elem::Logic && nbytes == u64::from(len).div_ceil(64) * 32 {
                            let t0 = t!(to_time(self.regs[time as usize] as i64));
                            let b = t!(self.rd(self.regs[src as usize], nbytes));
                            let Some(c) = ctx.as_deref_mut() else {
                                t!(rt_err_s!("signal or wait operation during elaboration"))
                            };
                            if len > 1
                                && t0 == Time::ZERO
                                && let [blk] = b.as_chunks::<32>().0
                                && c.assign_planes_now(
                                    d.id,
                                    sr.range,
                                    std::array::from_fn(|p| {
                                        u64::from_le_bytes(std::array::from_fn(|j| blk[p * 8 + j]))
                                    }),
                                )
                            {
                                // The usual vector at zero delay: the kernel's fast case.
                            } else if len == 1 {
                                let code = (0..4).fold(0u8, |c, p| c | (b[p * 8] & 1) << p);
                                let dm = t!(self.delay_mode(mode, None));
                                t!(c.assign_code(d.id, sr.range, t0, code, dm)
                                    .map_err(Fault::from));
                            } else {
                                let dm = t!(self.delay_mode(mode, None));
                                t!(c.assign_blocks(d.id, sr.range, t0, b, dm)
                                    .map_err(Fault::from));
                            }
                            done = true;
                        }
                    }
                    if !done {
                        // The waveform as the dropped instructions made it.
                        let a = t!(self.wave_at(size, time, w));
                        let at = t!(add_addr(a, 8));
                        t!(self.copy_mem(at, self.regs[src as usize], nbytes));
                        if multi {
                            t!(self.drive_parts(ctx, tg, Reg(w.into()), Reg(n.into()), mode, None));
                        } else {
                            let d = t!(self.driver(driver.into()));
                            t!(self.drive(
                                ctx,
                                d,
                                Some(tg),
                                Reg(w.into()),
                                Reg(n.into()),
                                mode,
                                None
                            ));
                        }
                    }
                    k += 1;
                    enter!();
                }
                Op::LCopyDrive {
                    driver,
                    target,
                    src,
                    soff,
                    cnt,
                    time,
                    size,
                    w,
                    e,
                    n,
                    inertial,
                } => {
                    t!(self.op_lcopy_drive(
                        ctx,
                        [driver, target, src, soff, cnt, time, size, w, e, n],
                        inertial
                    ));
                    k += 1;
                    enter!();
                }
                Op::LSetDrive {
                    v,
                    driver,
                    target,
                    time,
                    size,
                    w,
                    e,
                    inertial,
                } => {
                    let mode = if inertial {
                        DriveMode::Inertial
                    } else {
                        DriveMode::Transport
                    };
                    let bit = self.regs[v as usize];
                    let tg = Reg(target.into());
                    let done = bit <= 8
                        && (self.regs[time as usize] == 0
                            && self.drive_code_now(ctx, driver.into(), tg, bit as u8)
                            || !self.is_multi(tg) && {
                                let d = t!(self.driver(driver.into()));
                                let at = self.regs[time as usize];
                                t!(self.drive_bit_at(ctx, d, tg, at, bit as u8, mode))
                            });
                    if !done {
                        // Not a scalar logic signal (or a bad code): the instructions as the IR has them.
                        t!(self.waveform_head(size, time, w, e));
                        self.replay_lset_drive(
                            prog,
                            f,
                            code.at.get(k).copied().unwrap_or(0),
                            top,
                            ctx,
                        )?;
                    }
                    k += 1;
                    enter!();
                }
                Op::SigRead1 { dst, sig, offset } => {
                    self.regs[dst as usize] =
                        t!(self.read_sig1(sig, self.regs[offset as usize], ctx));
                    k += 1;
                    enter!();
                }
                Op::SigRead1Slot { dst, sig, off } => {
                    let o = slot8!(off);
                    self.regs[dst as usize] = t!(self.read_sig1(sig, o, ctx));
                    k += 1;
                    enter!();
                }
                Op::EdgeSig {
                    dst,
                    sig,
                    offset,
                    kind,
                } => {
                    self.regs[dst as usize] =
                        t!(self.edge_sig1(sig, self.regs[offset as usize], kind, ctx));
                    k += 1;
                    enter!();
                }
                Op::WaitSig { sig, offset } => {
                    t!(self.wait_sig1(sig, self.regs[offset as usize], ctx));
                    k += 1;
                    enter!();
                }
                Op::Gate { g, dst, a, b } => {
                    self.regs[dst as usize] = t!(crate::ieee::scalar_gate(
                        g,
                        self.regs[a as usize],
                        self.regs[b as usize]
                    ));
                    k += 1;
                    enter!();
                }
                Op::NotL { dst, a } => {
                    self.regs[dst as usize] = t!(crate::ieee::scalar_not(self.regs[a as usize]));
                    k += 1;
                    enter!();
                }
                Op::WaitOn { sig } => {
                    t!(self.wait_on(Reg(sig), ctx));
                    k += 1;
                    enter!();
                }
                Op::WaitEdge { sig, kind } => {
                    t!(self.wait_edge_inst(kind, Reg(sig), ctx));
                    k += 1;
                    enter!();
                }
                Op::WaitStatic { to } => {
                    self.end_stmt();
                    k = if ctx.as_deref_mut().is_some_and(|c| c.wait_same()) {
                        to as usize
                    } else {
                        k + 1
                    };
                    enter!();
                }
                Op::DriveRun { run, stmt } => {
                    if stmt {
                        self.end_stmt();
                    }
                    let r = code
                        .runs
                        .get(run as usize)
                        .ok_or_else(|| ice_f!("no drive run {run}"))?;
                    let done = self.drive_run(r, top, ctx);
                    k = match done {
                        0 if stmt => k + 1,
                        0 => {
                            t!(self.drive_run_head(r));
                            k + 1
                        }
                        d if d == r.drives.len() => r.end as usize,
                        d => r.starts[d] as usize,
                    };
                    enter!();
                }
                Op::Suspend => {
                    self.frames[top].pc = code.at.get(k).copied().unwrap_or(0);
                    self.suspend(prog, f, ctx)?;
                    return Ok(Flow::Suspend);
                }
                Op::CallNative {
                    native,
                    start,
                    len,
                    dst,
                } => {
                    self.frames[top].pc = code.at.get(k).copied().unwrap_or(0);
                    let args = yavs_ir::Regs { start, len };
                    let dst = (dst != pre::NONE).then_some(Reg(dst));
                    match self.call_native(prog, f, yavs_ir::NativeId(native), args, dst, ctx)? {
                        Flow::Next => {
                            k += 1;
                            enter!();
                        }
                        flow => {
                            return Ok(flow);
                        }
                    }
                }
                Op::Uniform {
                    s1,
                    s2,
                    x,
                    native,
                    start,
                } => {
                    self.frames[top].pc = code.at.get(k).copied().unwrap_or(0);
                    let vars = [
                        self.regs[s1 as usize],
                        self.regs[s2 as usize],
                        self.regs[x as usize],
                    ];
                    match self.op_uniform(prog, f, vars, native, start, ctx)? {
                        Flow::Next => {
                            k += 1;
                            enter!();
                        }
                        flow => {
                            return Ok(flow);
                        }
                    }
                }
                Op::Image { dst, a, ty } => {
                    self.regs[dst as usize] = t!(self.op_image(ty, self.regs[a as usize]));
                    k += 1;
                    enter!();
                }
                Op::ImageStr { a, ty, x, p, q, l } => {
                    let fat = t!(self.op_image(ty, self.regs[a as usize]));
                    let [data, dope, len] = t!(self.string_parts(fat));
                    self.regs[x as usize] = fat;
                    self.regs[p as usize] = data;
                    self.regs[q as usize] = dope;
                    self.regs[l as usize] = len;
                    k += 4;
                    enter!();
                }
                Op::CallIeee {
                    native,
                    dst,
                    len,
                    bit,
                    regs,
                } => {
                    let rv = &self.regs.v[self.regs.base..];
                    let mut cells: [u64; pre::IEEE_CELLS] =
                        std::array::from_fn(|i| rv[usize::from(regs[i])]);
                    // The one-word path reports nothing and fails nowhere: no frame pc, no flags.
                    if let Some(fast) = self.rt.fast.get(usize::from(native)).copied().flatten()
                        && let Some(v) = fast(self, &cells)
                    {
                        if dst != pre::NONE16 {
                            self.regs[usize::from(dst)] = v;
                        }
                        k += 1;
                        enter!();
                        continue;
                    }
                    self.frames[top].pc = code.at.get(k).copied().unwrap_or(0);
                    let Some(fun) = self.rt.natives.get(usize::from(native)).copied().flatten()
                    else {
                        return rt_err_f!(
                            "native subprogram `{}` is not implemented in yavs yet",
                            Self::native_symbol(prog, native)
                        );
                    };
                    let cells = &mut cells[..usize::from(len)];
                    // Only the natives of `numeric_bit` look at the type, and only whether there is one.
                    let v = fun(self, ctx, bit.then_some(yavs_ir::TyId(0)), cells)?;
                    if self.ieee_fallback | self.ieee_stop {
                        match self.ieee_after(prog, native, cells, dst, v)? {
                            Flow::Next => {
                                k += 1;
                                enter!();
                            }
                            flow => return Ok(flow),
                        }
                    } else {
                        if dst != pre::NONE16 {
                            let Some(v) = v else {
                                return Err(ice_f!(
                                    "native `{}` returned no value",
                                    Self::native_symbol(prog, native)
                                ));
                            };
                            self.regs[usize::from(dst)] = v;
                        }
                        k += 1;
                        enter!();
                    }
                }
                Op::CmpSelect { m, dst, a, b, x, y } => {
                    let c = pre::cmp_holds(
                        m,
                        self.regs[a as usize] as i64,
                        self.regs[b as usize] as i64,
                    );
                    self.regs[dst as usize] = self.regs[if c { x } else { y } as usize];
                    k += 2;
                }
                Op::DerefLoadOff {
                    w,
                    d1,
                    handle,
                    what,
                    d2,
                    off,
                } => {
                    let a = t!(self.deref(self.regs[handle as usize], what));
                    self.regs[d1 as usize] = a;
                    match self.load(a + u64::from(off), w) {
                        Ok(v) => self.regs[d2 as usize] = v,
                        Err(e) => {
                            self.frames[top].pc = code.at.get(k + 1).copied().unwrap_or(0);
                            return Err(e);
                        }
                    }
                    k += 2;
                }
                Op::DerefImm {
                    d1,
                    handle,
                    d2,
                    off,
                } => {
                    let hv = self.regs[handle as usize];
                    let a = match self.deref(hv, yavs_ir::CheckMsgId(0)) {
                        Ok(a) => a,
                        Err(_) => {
                            let what = deref_what!(k);
                            t!(self.deref(hv, what))
                        }
                    };
                    self.regs[d1 as usize] = a;
                    // `deref` gives offset 0: a 32-bit `off` cannot overflow the offset field.
                    self.regs[d2 as usize] = a + u64::from(off);
                    k += 2;
                }
                Op::LoadDerefImm {
                    base,
                    h,
                    d1,
                    d2,
                    o1,
                    o2,
                } => {
                    let a = t!(add_addr(self.regs[base as usize], o1.into()));
                    let hv = t!(self.load(a, Width::B8));
                    self.regs[h as usize] = hv;
                    let a = match self.deref(hv, yavs_ir::CheckMsgId(0)) {
                        Ok(a) => a,
                        Err(_) => {
                            self.frames[top].pc = code.at.get(k + 1).copied().unwrap_or(0);
                            let what = deref_what!(k + 1);
                            return Err(self.deref(hv, what).err().unwrap_or_else(|| {
                                ice_s!("dereference failed once and then succeeded")
                            }));
                        }
                    };
                    self.regs[d1 as usize] = a;
                    self.regs[d2 as usize] = a + u64::from(o2);
                    k += 3;
                }
                Op::MoveCondBr { dst, src, t, f: e } => {
                    let v = self.regs[src as usize];
                    self.regs[dst as usize] = v;
                    go!(if v != 0 { t } else { e });
                }
                Op::EdgeBr {
                    dst,
                    kind,
                    stmt,
                    sig,
                    offset,
                    t,
                    f: e,
                } => {
                    if stmt {
                        self.end_stmt();
                        k += 1;
                    }
                    let v = t!(self.edge_sig1(sig, self.regs[offset as usize], kind, ctx));
                    self.regs[dst as usize] = v;
                    go!(if v != 0 { t } else { e });
                }
                Op::WaitSuspend {
                    sig,
                    offset,
                    at,
                    stmt,
                } => {
                    k = at as usize;
                    if stmt {
                        self.end_stmt();
                        k += 1;
                    }
                    t!(self.wait_sig1(sig, self.regs[offset as usize], ctx));
                    self.frames[top].pc = code.at.get(k + 1).copied().unwrap_or(0);
                    self.suspend(prog, f, ctx)?;
                    return Ok(Flow::Suspend);
                }
                Op::WaitEdgeSuspend {
                    sig,
                    offset,
                    kind,
                    at,
                    stmt,
                    body,
                } => {
                    if body != pre::NONE
                        && self.pause == Some(BlockId(body))
                        && self.frames.len() == 1
                    {
                        // Stopped before the jump, which runs again on resumption.
                        self.frames[top].pc = code.at.get(k).copied().unwrap_or(0);
                        self.pause = None;
                        return Ok(Flow::Next);
                    }
                    k = at as usize;
                    if stmt {
                        self.end_stmt();
                        k += 1;
                    }
                    let o = self.regs[offset as usize];
                    t!(self.wait_edge_sig1(sig, o, kind, ctx));
                    self.frames[top].pc = code.at.get(k + 3).copied().unwrap_or(0);
                    self.suspend(prog, f, ctx)?;
                    return Ok(Flow::Suspend);
                }
                Op::FlagBr { dst, off, t, f: e } => {
                    self.end_stmt();
                    let v = slot8!(off);
                    self.regs[dst as usize] = v;
                    go!(if v != 0 { t } else { e });
                }
                Op::SigValSlot {
                    d,
                    off,
                    dst,
                    sig,
                    len,
                } => {
                    let o = slot8!(off);
                    self.regs[d as usize] = o;
                    let (a, b) = t!(self.sig_val(sig, o as i64, self.regs[len as usize] as i64));
                    self.regs[dst as usize] = a;
                    self.regs[dst as usize + 1] = b;
                    k += 2;
                    enter!();
                }
                Op::CatLCopy { tmp, size, dst, n } => {
                    let pc = code.at.get(k + 1).copied().unwrap_or(0);
                    let Some(&Inst::Cat {
                        dst: _,
                        n: cn,
                        m,
                        parts,
                        scalars,
                        bias,
                        logic,
                        w,
                    }) = f.code.get(pc as usize)
                    else {
                        return Err(ice_f!("pc {pc} is not a concatenation"));
                    };
                    let lpc = code.at.get(k + 2).copied().unwrap_or(0);
                    let Some(&Inst::LCopy {
                        dst_off, src_off, ..
                    }) = f.code.get(lpc as usize)
                    else {
                        return Err(ice_f!("pc {lpc} is not a logic copy"));
                    };
                    let (doff, soff) = (self.r(dst_off), self.r(src_off));
                    let ps = t!(f.regs(parts).ok_or_else(|| ice_s!("operand list")));
                    let d = self.regs[dst as usize];
                    let (mv, cnt) = (self.regs[m.0 as usize], self.regs[n as usize]);
                    let total = self.regs[cn.0 as usize].wrapping_mul(mv);
                    // The whole concatenation copied: it can be built in place.
                    let whole = logic && soff == 0 && cnt == total;
                    // One block of planes into memory that is not a fresh temporary: written
                    // once after every piece is read, so a piece may be in `d` itself.
                    let direct = whole
                        && total <= 64
                        && doff == 0
                        && split(d).0 != TEMP
                        && t!(self.cat_block(d, total, mv, ps, scalars, bias));
                    if direct {
                        k += 3;
                    } else if whole && total > 64 && self.cat_apart(d, doff, total, mv, ps, scalars)
                    {
                        // Wide: the pieces straight into the destination, no temporary and no
                        // second copy.
                        t!(self.cat(
                            d,
                            doff,
                            self.regs[cn.0 as usize],
                            mv,
                            ps,
                            scalars,
                            bias,
                            logic,
                            w
                        ));
                        k += 3;
                    } else {
                        let size = if size & pre::SIZE_REG != 0 {
                            self.regs[(size & !pre::SIZE_REG) as usize]
                        } else {
                            size.into()
                        };
                        let a = t!(self.temp_alloc(size));
                        self.regs[tmp as usize] = a;
                        t!(self.cat(
                            a,
                            0,
                            self.regs[cn.0 as usize],
                            mv,
                            ps,
                            scalars,
                            bias,
                            logic,
                            w
                        ));
                        t!(self.lcopy(d, doff, a, soff, cnt));
                        k += 3;
                    }
                }
                Op::FatRoot { addr } => {
                    t!(self.fat_root(self.regs[addr as usize]));
                    k += 1;
                }
                Op::NewFrame(pc) => {
                    let Some(&Inst::NewFrame {
                        dst,
                        desc,
                        dsize,
                        data,
                        size,
                        fill,
                    }) = f.code.get(pc as usize)
                    else {
                        return Err(ice_f!("pc {pc} is not a NewFrame"));
                    };
                    t!(self.new_frame(dst, desc, dsize, data, size, fill));
                    k += 1;
                }
                Op::MemEq { dst, a, b, n } => {
                    t!(self.op_mem_eq(dst, a, b, n));
                    k += 1;
                }
                Op::AllocLCopy {
                    dst,
                    doff,
                    src,
                    soff,
                    n,
                } => {
                    // `AllocK` of one block and the copy of up to 64 elements from one block of
                    // the source into it, built in registers.
                    let (cnt, so, d0) = (
                        self.regs[n as usize],
                        self.regs[soff as usize],
                        self.regs[doff as usize],
                    );
                    let mut done = false;
                    if cnt != 0 && cnt <= 64 && so % 64 + cnt <= 64 && d0 < 64 && d0 + cnt <= 64 {
                        let sa = t!(add_addr(self.regs[src as usize], so / 64 * 32));
                        if let Ok(b) = self.rd_hot(sa, 32) {
                            // The new block is zero but for the copied elements.
                            let (s0, mask) =
                                (so % 64, if cnt == 64 { !0 } else { (1u64 << cnt) - 1 });
                            let mut out = [0u8; 32];
                            for (p, c) in out.as_chunks_mut::<8>().0.iter_mut().enumerate() {
                                *c = ((plane_word(b, p) >> s0 & mask) << d0).to_le_bytes();
                            }
                            self.regs[dst as usize] = t!(self.temp_push32(&out));
                            done = true;
                        }
                    }
                    if done {
                        k += 2;
                    } else {
                        self.regs[dst as usize] = t!(self.temp_alloc(32));
                        k += 1;
                    }
                }
                Op::IsXSlice {
                    tmp,
                    src,
                    soff,
                    n,
                    res,
                } => {
                    // `AllocLCopy` and `is_x` of the copy, on the planes of the source: the
                    // elements of one block, tested where they are.
                    if let Some(x) = self.is_x_planes(src, soff, n) {
                        self.regs[res as usize] = x;
                        k += 3;
                    } else {
                        self.regs[tmp as usize] = t!(self.temp_alloc(32));
                        k += 1;
                    }
                }
                Op::SliceAff {
                    i,
                    dope,
                    a,
                    b,
                    p,
                    k: kc,
                    c,
                } => {
                    let x = self.regs[i as usize] as i64;
                    if let Some((l, r, pos)) = self.slice_aff(x, self.regs[dope as usize], kc, c) {
                        self.regs[a as usize] = l as u64;
                        self.regs[b as usize] = r as u64;
                        self.regs[p as usize] = pos;
                        k += 3;
                    } else {
                        // The first op alone (`Op::BinRangeK`), which reports what it finds.
                        let (y, lo, hi) = (i64::from(kc), i64::from(i32::MIN), i64::from(i32::MAX));
                        self.regs[a as usize] = match fast_op(IntOp::Mul, x, y) {
                            Some(v) if lo <= v && v <= hi => v as u64,
                            _ => t!(self.bin_range_slow(IntOp::Mul, x, y, lo, hi, || {
                                match code.at.get(k).and_then(|&p| f.code.get(p as usize)) {
                                    Some(&Inst::BinRange { what, .. }) => what,
                                    _ => yavs_ir::CheckMsgId(0),
                                }
                            })),
                        };
                        k += 1;
                    }
                }
                Op::StmtIsXBr {
                    tmp,
                    src,
                    soff,
                    n,
                    res,
                    t: yes,
                    f: no,
                } => {
                    self.end_stmt();
                    if let Some(x) = self.is_x_planes(src.into(), soff.into(), n.into()) {
                        self.regs[res as usize] = x;
                        go!(if x != 0 { yes } else { no });
                    } else {
                        self.regs[tmp as usize] = t!(self.temp_alloc(32));
                        k += 2;
                    }
                }
                Op::AffLCopy {
                    ad,
                    a,
                    b,
                    ld,
                    src,
                    soff,
                    n,
                    ca,
                    cb,
                    c0,
                } => {
                    let (x, y) = (self.regs[a as usize] as i64, self.regs[b as usize] as i64);
                    if aff_ok(x) && aff_ok(y) {
                        self.regs[ad as usize] =
                            (i64::from(c0) + i64::from(ca) * x + i64::from(cb) * y) as u64;
                        k += 1;
                        t!(self.op_lcopy(ld.into(), ad.into(), src.into(), soff.into(), n.into()));
                    } else {
                        self.aff_replay(prog, f, code, top, k, ctx)?;
                    }
                    k += 1;
                    enter!();
                }
                Op::StmtIsXSlice {
                    tmp,
                    src,
                    soff,
                    n,
                    res,
                } => {
                    // The same after the marker of its statement.
                    self.end_stmt();
                    if let Some(x) = self.is_x_planes(src, soff, n) {
                        self.regs[res as usize] = x;
                        k += 4;
                    } else {
                        self.regs[tmp as usize] = t!(self.temp_alloc(32));
                        k += 2;
                    }
                }
                Op::AllocReadVec { dst, sig } => {
                    // `AllocK` of one block and the read of a logic signal into it.
                    let mut done = false;
                    if !self.is_multi(Reg(sig)) {
                        let (sr, ty) = t!(self.sigref(Reg(sig)));
                        if ty.elem == Elem::Logic
                            && sr.range.is_some_and(|r| (1..=64).contains(&r.len))
                            && let Some(c) = ctx.as_deref_mut()
                        {
                            let mut buf = [0u8; 32];
                            t!(c.read_into(sr, &mut buf).map_err(Fault::from));
                            self.regs[dst as usize] = t!(self.temp_push32(&buf));
                            done = true;
                        }
                    }
                    if done {
                        k += 2;
                    } else {
                        self.regs[dst as usize] = t!(self.temp_alloc(32));
                        k += 1;
                    }
                }
                Op::AllocCat {
                    dst,
                    n,
                    m,
                    parts,
                    len,
                    scalars,
                    bias,
                } => {
                    // `AllocK` of one block and the `Cat` into it of at most 64 elements.
                    let ps = t!(f
                        .regs(yavs_ir::Regs {
                            start: parts,
                            len: len.into()
                        })
                        .ok_or_else(|| ice_s!("operand list")));
                    let (mv, total) = (
                        self.regs[m as usize],
                        self.regs[n as usize].wrapping_mul(self.regs[m as usize]),
                    );
                    let words = if total <= 64 {
                        t!(self.cat_words(total, mv, ps, scalars, bias))
                    } else {
                        None
                    };
                    match words {
                        Some((words, mask, _)) => {
                            let mut out = [0u8; 32];
                            for (o, w) in out.as_chunks_mut::<8>().0.iter_mut().zip(words) {
                                *o = (w & mask).to_le_bytes();
                            }
                            self.regs[dst as usize] = t!(self.temp_push32(&out));
                            k += 2;
                        }
                        None => {
                            self.regs[dst as usize] = t!(self.temp_alloc(32));
                            k += 1;
                        }
                    }
                }
                Op::AllocK { dst, n } => {
                    self.regs[dst as usize] = t!(self.temp_alloc(n.into()));
                    k += 1;
                }
                Op::NewPair { dst, a, b } => {
                    let (x, y) = (self.regs[a as usize], self.regs[b as usize]);
                    self.regs[dst as usize] = t!(self.new_pair(x, y));
                    k += 3;
                }
                Op::NewDope { at, start, len } => {
                    let parts = t!(f
                        .regs(yavs_ir::Regs { start, len })
                        .ok_or_else(|| ice_s!("operand list")));
                    let a = t!(self.temp_alloc(u64::from(len) * 8));
                    self.regs[at as usize] = a;
                    t!(self.make_dope(a, parts));
                    k += 2;
                }
                Op::Alloc { dst, size } => {
                    self.regs[dst as usize] = t!(self.temp_alloc(self.regs[size as usize]));
                    k += 1;
                }
                Op::Const { dst, v } => {
                    self.regs[dst as usize] = v as u64;
                    k += 1;
                }
                Op::FCmp { op, dst, a, b } => {
                    let (x, y) = (
                        f64::from_bits(self.regs[a as usize]),
                        f64::from_bits(self.regs[b as usize]),
                    );
                    self.regs[dst as usize] =
                        u64::from(x.partial_cmp(&y).is_some_and(|o| cmp(op, o)));
                    k += 1;
                }
                Op::AddrFrame { dst, off } => {
                    self.regs[dst as usize] = t!(mk_addr(FRAME, top as u32, off.into()));
                    k += 1;
                }
                Op::AddrUp { dst, depth, off } => {
                    let mut i = top;
                    for _ in 0..depth {
                        i = t!(self.frames[i]
                            .link
                            .ok_or_else(|| ice_s!("static link missing")))
                            as usize;
                    }
                    self.regs[dst as usize] = t!(mk_addr(FRAME, i as u32, off.into()));
                    k += 1;
                }
                Op::LoadUp { dst, depth, w, off } => {
                    let mut i = top;
                    for _ in 0..depth {
                        i = t!(self.frames[i]
                            .link
                            .ok_or_else(|| ice_s!("static link missing")))
                            as usize;
                    }
                    let n = w.bytes().min(8) as usize;
                    let o = off as usize;
                    self.regs[dst as usize] =
                        match self.fslots(i).and_then(|s| s.get(o..o + n)) {
                            Some(b) => cell_w(b, w),
                            None => t!(mk_addr(FRAME, i as u32, off.into())
                                .and_then(|a| self.load_slow(a, w))),
                        };
                    k += 2;
                }
                Op::StoreUp { src, depth, w, off } => {
                    let mut i = top;
                    for _ in 0..depth {
                        i = t!(self.frames[i]
                            .link
                            .ok_or_else(|| ice_s!("static link missing")))
                            as usize;
                    }
                    let v = self.regs[src as usize];
                    let n = w.bytes() as usize;
                    let o = off as usize;
                    match self.fslots_mut(i).and_then(|s| s.get_mut(o..o + n)) {
                        Some(b) => put_le(b, v),
                        None => t!(mk_addr(FRAME, i as u32, off.into())
                            .and_then(|a| self.store_slow(a, w, v))),
                    }
                    k += 2;
                }
                Op::Copy { dst, src, bytes } => {
                    t!(self.copy_mem(
                        self.regs[dst as usize],
                        self.regs[src as usize],
                        self.regs[bytes as usize]
                    ));
                    k += 1;
                }
                Op::CopyConst { dst, c } => {
                    t!(self.op_copy_const(prog, dst, c));
                    k += 1;
                }
                Op::MakeDope { at, start, len } => {
                    t!(self.op_make_dope(f, at, start, len));
                    k += 1;
                }
                Op::SubtypeCheck {
                    dope,
                    want,
                    dims,
                    what,
                } => {
                    t!(self.subtype_check(
                        self.regs[dope as usize],
                        self.regs[want as usize],
                        dims,
                        what
                    ));
                    k += 1;
                }
                Op::Deref { dst, handle, what } => {
                    self.regs[dst as usize] = t!(self.deref(self.regs[handle as usize], what));
                    k += 1;
                }
                Op::Field {
                    dst,
                    base,
                    layout,
                    n,
                } => {
                    t!(self.op_field(dst, base, layout, n));
                    k += 1;
                }
                Op::LGet { dst, base, idx } => {
                    self.regs[dst as usize] =
                        t!(self.lget(self.regs[base as usize], self.regs[idx as usize]));
                    k += 1;
                }
                Op::LSet { base, idx, v } => {
                    t!(self.lset(
                        self.regs[base as usize],
                        self.regs[idx as usize],
                        self.regs[v as usize]
                    ));
                    k += 1;
                }
                Op::CondBr { c, t, f: e } => {
                    go!(if self.regs[c as usize] != 0 { t } else { e });
                }
                Op::ZeroSlot { w, off, n } => {
                    t!(self.op_zero_slot(top, w, off, n));
                    k += 1;
                }
                Op::ZeroReadVec { dst, sig, off } => {
                    k += t!(self.op_zero_read_vec(ctx.as_deref_mut(), top, dst, sig, off));
                }
                op @ (Op::ZeroCat { .. } | Op::ZeroLCopy { .. } | Op::ZeroCatLCopy { .. }) => {
                    k += t!(self.op_zero_fill(f, code, k, op));
                }
                Op::SlotBr { dst, off, t, f: e } => {
                    let v = slot8!(off);
                    self.regs[dst as usize] = v;
                    go!(if v != 0 { t } else { e });
                }
            }
        }
    }

    /// `Op::PosIndex`, out of line.
    #[inline(never)]
    #[allow(clippy::too_many_arguments)]
    fn op_pos_index(
        &mut self,
        f: &Func,
        code: &pre::Code,
        k: usize,
        dst: u32,
        dope: u32,
        idx: u32,
        base: u32,
        stride: u32,
        dim: u8,
    ) -> R<()> {
        let p = self.pos_of_with(
            self.regs[dope as usize],
            dim,
            self.regs[idx as usize] as i64,
            || pos_what_at(f, code, k),
        )?;
        self.regs[dst as usize] = index_addr(self.regs[base as usize], p, stride)?;
        Ok(())
    }

    /// `Op::PosIndexStore`, out of line.
    #[inline(never)]
    #[allow(clippy::too_many_arguments)]
    fn op_pos_index_store(
        &mut self,
        f: &Func,
        code: &pre::Code,
        k: usize,
        src: u32,
        dope: u32,
        idx: u32,
        base: u32,
        stride: u32,
        w: Width,
        dim: u8,
    ) -> R<()> {
        let p = self.pos_of_with(
            self.regs[dope as usize],
            dim,
            self.regs[idx as usize] as i64,
            || pos_what_at(f, code, k),
        )?;
        self.store_idx(
            self.regs[base as usize],
            p,
            stride,
            w,
            self.regs[src as usize],
        )?;
        Ok(())
    }

    /// `Op::IndexStore`, out of line.
    #[inline(never)]
    fn op_index_store(&mut self, src: u32, base: u32, pos: u32, stride: u32, w: Width) -> R<()> {
        self.store_idx(
            self.regs[base as usize],
            self.regs[pos as usize],
            stride,
            w,
            self.regs[src as usize],
        )?;
        Ok(())
    }

    /// `Inst::Cover`, out of line: one relaxed add on the counter of the instance.
    #[inline(never)]
    fn op_cover(&mut self, item: u32) -> R<()> {
        let i = self.inst.cov_base as usize + item as usize;
        self.rt
            .cov
            .get(i)
            .ok_or_else(|| ice_f!("no coverage counter {i}"))?
            .fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    /// `Inst::CoverIn`, out of line.
    #[inline(never)]
    fn op_cover_in(&mut self, dst: u32, acc: u32, src: u32, shift: u8, ulogic: bool) {
        let p = self.regs[src as usize];
        let code = match (ulogic, p) {
            (true, 2 | 6) | (false, 0) => 1,
            (true, 3 | 7) | (false, 1) => 2,
            _ => 3,
        };
        let before = if shift == 0 {
            0
        } else {
            self.regs[acc as usize]
        };
        self.regs[dst as usize] = before | code << shift;
    }

    /// `Inst::CoverCond`, out of line: count the rows the state covers.
    #[inline(never)]
    fn op_cover_cond(&mut self, prog: &Program, cond: u32, state: u32) -> R<()> {
        let c = prog
            .cov_conds
            .get(cond as usize)
            .ok_or_else(|| ice_f!("no coverage condition {cond}"))?;
        let st = self.regs[state as usize];
        let mut hits = match self
            .rt
            .cov_memo
            .get(cond as usize)
            .and_then(|m| m.get(st as usize))
        {
            Some(m) => match m.load(Ordering::Relaxed) {
                u64::MAX => {
                    let h = c.hits(st);
                    m.store(h, Ordering::Relaxed);
                    h
                }
                h => h,
            },
            None => c.hits(st),
        };
        while hits != 0 {
            let row = hits.trailing_zeros();
            hits &= hits - 1;
            let i = self.inst.cov_base as usize + c.first.0 as usize + row as usize;
            self.rt
                .cov
                .get(i)
                .ok_or_else(|| ice_f!("no coverage counter {i}"))?
                .fetch_add(1, Ordering::Relaxed);
        }
        Ok(())
    }

    /// `Op::ZeroReadVec`: `AllocReadVec` into the block in the frame's slots at `off`. The number
    /// of ops done.
    #[inline(never)]
    fn op_zero_read_vec(
        &mut self,
        ctx: Option<&mut Ctx<'_>>,
        top: usize,
        dst: u32,
        sig: u32,
        off: u32,
    ) -> R<usize> {
        if !self.is_multi(Reg(sig)) {
            let (sr, ty) = self.sigref(Reg(sig))?;
            if ty.elem == Elem::Logic
                && sr.range.is_some_and(|r| (1..=64).contains(&r.len))
                && let Some(c) = ctx
            {
                let mut buf = [0u8; 32];
                c.read_into(sr, &mut buf).map_err(Fault::from)?;
                let slots = if top == 0 {
                    Some(&mut self.root[..])
                } else {
                    self.fslots_mut(top)
                };
                if let Some(b) = slots.and_then(|s| s.get_mut(off as usize..off as usize + 32)) {
                    b.copy_from_slice(&buf);
                    return Ok(2);
                }
            }
        }
        self.zero_at(dst, 32)?;
        Ok(1)
    }

    /// The frame forms of `AllocCat`, `AllocLCopy` and `CatLCopy` (`Op::ZeroCat` and the others;
    /// `Op::ZeroReadVec` is an arm of the loop): the block is the temporary in the frame the
    /// register points to (its `ZeroSlot` is this op), so the fast paths write it in place and the
    /// others zero it and go on with the next op. The number of ops done.
    #[inline(never)]
    fn op_zero_fill(&mut self, f: &Func, code: &pre::Code, k: usize, op: Op) -> R<usize> {
        let put = |m: &mut Self, dst: u32, b: &[u8; 32]| -> R<usize> {
            let a = m.regs[dst as usize];
            match m.frame_bytes(a, 32) {
                Some(d) => d.copy_from_slice(b),
                None => m.wr(a, 32)?.copy_from_slice(b),
            }
            Ok(2)
        };
        let cat_at = |pc: u32| match f.code.get(pc as usize) {
            Some(&Inst::Cat {
                n,
                m,
                parts,
                scalars,
                bias,
                logic,
                w,
                ..
            }) => Ok((n, m, parts, scalars, bias, logic, w)),
            _ => Err(ice_f!("pc {pc} is not a concatenation")),
        };
        let next = code.at.get(k + 1).copied().unwrap_or(0);
        match op {
            Op::ZeroCat { dst } => {
                let (n, m, parts, scalars, bias, _, _) = cat_at(next)?;
                let ps = f.regs(parts).ok_or_else(|| ice_s!("operand list"))?;
                let (mv, total) = (
                    self.regs[m.0 as usize],
                    self.regs[n.0 as usize].wrapping_mul(self.regs[m.0 as usize]),
                );
                if total <= 64
                    && let Some((words, mask, _)) = self.cat_words(total, mv, ps, scalars, bias)?
                {
                    let mut out = [0u8; 32];
                    for (o, w) in out.as_chunks_mut::<8>().0.iter_mut().zip(words) {
                        *o = (w & mask).to_le_bytes();
                    }
                    return put(self, dst, &out);
                }
                self.zero_at(dst, 32)?;
            }
            Op::ZeroLCopy {
                dst,
                doff,
                src,
                soff,
                n,
            } => {
                let (cnt, so, d0) = (
                    self.regs[n as usize],
                    self.regs[soff as usize],
                    self.regs[doff as usize],
                );
                // As `AllocLCopy`.
                if cnt != 0 && cnt <= 64 && so % 64 + cnt <= 64 && d0 < 64 && d0 + cnt <= 64 {
                    let sa = add_addr(self.regs[src as usize], so / 64 * 32)?;
                    if let Ok(b) = self.rd_hot(sa, 32) {
                        let (s0, mask) = (so % 64, if cnt == 64 { !0 } else { (1u64 << cnt) - 1 });
                        let mut out = [0u8; 32];
                        for (p, c) in out.as_chunks_mut::<8>().0.iter_mut().enumerate() {
                            *c = ((plane_word(b, p) >> s0 & mask) << d0).to_le_bytes();
                        }
                        return put(self, dst, &out);
                    }
                }
                self.zero_at(dst, 32)?;
            }
            Op::ZeroCatLCopy { tmp, size, dst, n } => {
                let (cn, m, parts, scalars, bias, logic, w) = cat_at(next)?;
                let ps = f.regs(parts).ok_or_else(|| ice_s!("operand list"))?;
                let d = self.regs[dst as usize];
                let total = self.regs[cn.0 as usize].wrapping_mul(self.regs[m.0 as usize]);
                // As `CatLCopy`: one block of planes written into `d` once every piece is read.
                let direct = logic
                    && total <= 64
                    && self.regs[n as usize] == total
                    && split(d).0 != TEMP
                    && self.cat_block(d, total, self.regs[m.0 as usize], ps, scalars, bias)?;
                if !direct {
                    self.zero_at(tmp, size)?;
                    let a = self.regs[tmp as usize];
                    let (cnt, mv) = (self.regs[cn.0 as usize], self.regs[m.0 as usize]);
                    self.cat(a, 0, cnt, mv, ps, scalars, bias, logic, w)?;
                    self.lcopy(d, 0, a, 0, self.regs[n as usize])?;
                }
                return Ok(3);
            }
            _ => return Err(ice_s!("not a frame fill")),
        }
        Ok(1)
    }

    /// `Inst::Zero` of `n` bytes at the address in register `w`.
    #[inline(never)]
    fn zero_at(&mut self, w: u32, n: u32) -> R<()> {
        let a = self.regs[w as usize];
        match self.frame_bytes(a, n as usize) {
            Some(d) => d.fill(0),
            None => self.wr(a, n.into())?.fill(0),
        }
        Ok(())
    }

    /// The `n` bytes at `a` when they are in the slots of a frame (`None`: anywhere else, or
    /// outside the slots, for the general path and its errors).
    #[inline(always)]
    fn frame_bytes(&mut self, a: u64, n: usize) -> Option<&mut [u8]> {
        let (sp, i, off) = split(a);
        if sp != FRAME {
            return None;
        }
        let slots = if i == 0 {
            Some(&mut self.root[..])
        } else {
            self.fslots_mut(i as usize)
        };
        slots?.get_mut(off as usize..(off as usize).checked_add(n)?)
    }

    /// `Op::ZeroSlot`: `n` bytes at `off` of the slots of frame `top` (its address in `w`).
    #[inline(never)]
    fn op_zero_slot(&mut self, top: usize, w: u32, off: u32, n: u32) -> R<()> {
        let slots = if top == 0 {
            Some(&mut self.root[..])
        } else {
            self.fslots_mut(top)
        };
        match slots.and_then(|s| s.get_mut(off as usize..(off as usize + n as usize))) {
            Some(b) => b.fill(0),
            None => self.zero_at(w, n)?,
        }
        Ok(())
    }

    /// `Op::WaveBuf`, out of line.
    #[inline(never)]
    fn op_wave_buf(&mut self, w: u32, size: u32, time: u32, e: u32) -> R<()> {
        let a = self.temp_alloc(self.regs[size as usize])?;
        self.regs[w as usize] = a;
        self.store(a, Width::B8, self.regs[time as usize])?;
        self.regs[e as usize] = add_addr(a, 8)?;
        Ok(())
    }

    /// `Op::LCopyDrive`, out of line. `r` is `driver, target, src, soff, cnt, time, size, w, e,
    /// n` of the op.
    #[inline(never)]
    fn op_lcopy_drive(
        &mut self,
        ctx: &mut Option<&mut Ctx<'_>>,
        r: [u16; 10],
        inertial: bool,
    ) -> R<()> {
        let [driver, target, src, soff, cnt, time, size, w, e, n] = r;
        let mode = if inertial {
            DriveMode::Inertial
        } else {
            DriveMode::Transport
        };
        let tg = Reg(target.into());
        let multi = self.is_multi(tg);
        let (count, from) = (self.regs[cnt as usize], self.regs[soff as usize]);
        if !multi && self.regs[time as usize] == 0 && (2..=64).contains(&count) {
            let (s0, base) = (from % 64, from / 64);
            let (sr, ty) = self.sigref(tg)?;
            if ty.elem == Elem::Logic
                && u64::from(sr.range.map_or(0, |r| r.len)) == count
                && s0 + count <= 64
            {
                let d = self.driver(driver.into())?;
                let a = add_addr(self.regs[src as usize], base * 32)?;
                if let Ok(b) = self.rd_hot(a, 32) {
                    let mask = if count == 64 { !0 } else { (1u64 << count) - 1 };
                    let planes: [u64; 4] = std::array::from_fn(|p| plane_word(b, p) >> s0 & mask);
                    let Some(c) = ctx.as_deref_mut() else {
                        return rt_err_s!("signal or wait operation during elaboration");
                    };
                    if c.assign_planes_now(d.id, sr.range, planes) {
                        return Ok(());
                    }
                }
            }
        }
        // The waveform as the three dropped instructions made it.
        self.waveform_head(size, time, w, e)?;
        self.lcopy(
            self.regs[e as usize],
            0,
            self.regs[src as usize],
            from,
            count,
        )?;
        if multi {
            self.drive_parts(ctx, tg, Reg(w.into()), Reg(n.into()), mode, None)
        } else {
            let d = self.driver(driver.into())?;
            self.drive(ctx, d, Some(tg), Reg(w.into()), Reg(n.into()), mode, None)
        }
    }

    /// `Op::BinRange`, out of line.
    #[inline(never)]
    #[allow(clippy::too_many_arguments)]
    fn op_bin_range(
        &mut self,
        f: &Func,
        code: &pre::Code,
        k: usize,
        op: IntOp,
        dst: u32,
        a: u32,
        b: u32,
        lo: u32,
        hi: u32,
    ) -> R<()> {
        let (x, y) = (self.regs[a as usize] as i64, self.regs[b as usize] as i64);
        let (l, h) = (self.regs[lo as usize] as i64, self.regs[hi as usize] as i64);
        self.regs[dst as usize] = match fast_op(op, x, y) {
            Some(v) if l <= v && v <= h => v as u64,
            _ => self.bin_range_slow(op, x, y, l, h, || {
                match code.at.get(k).and_then(|&p| f.code.get(p as usize)) {
                    Some(&Inst::BinRange { what, .. }) => what,
                    _ => yavs_ir::CheckMsgId(0),
                }
            })?,
        };
        Ok(())
    }

    /// `Op::Drive`, out of line.
    #[inline(never)]
    #[allow(clippy::too_many_arguments)]
    fn op_drive(
        &mut self,
        driver: u32,
        target: u32,
        wave: u32,
        n: u32,
        reject: u32,
        inertial: bool,
        ctx: &mut Option<&mut Ctx<'_>>,
    ) -> R<()> {
        let mode = if inertial {
            DriveMode::Inertial
        } else {
            DriveMode::Transport
        };
        let reject = (reject != pre::NONE).then_some(Reg(reject));
        let (target, wave, n) = (Reg(target), Reg(wave), Reg(n));
        if self.is_multi(target) {
            self.drive_parts(ctx, target, wave, n, mode, reject)?;
        } else {
            let d = self.driver(driver)?;
            self.drive(ctx, d, Some(target), wave, n, mode, reject)?;
        }
        Ok(())
    }

    /// `Op::MemEq`, out of line.
    #[inline(never)]
    fn op_mem_eq(&mut self, dst: u32, a: u32, b: u32, n: u32) -> R<()> {
        let n = self.regs[n as usize];
        let v = self.rd(self.regs[a as usize], n)? == self.rd(self.regs[b as usize], n)?;
        self.regs[dst as usize] = v.into();
        Ok(())
    }

    /// `Op::CopyConst`, out of line.
    #[inline(never)]
    fn op_copy_const(&mut self, prog: &Program, dst: u32, c: u32) -> R<()> {
        self.copy_const(prog, self.regs[dst as usize], yavs_ir::ConstId(c))?;
        Ok(())
    }

    /// `Op::MakeDope`, out of line.
    #[inline(never)]
    fn op_make_dope(&mut self, f: &Func, at: u32, start: u32, len: u32) -> R<()> {
        let parts = f
            .regs(yavs_ir::Regs { start, len })
            .ok_or_else(|| ice_s!("operand list"))?;
        let a = self.regs[at as usize];
        self.make_dope(a, parts)?;
        Ok(())
    }

    /// `Op::Field`, out of line.
    #[inline(never)]
    fn op_field(&mut self, dst: u32, base: u32, layout: u32, n: u32) -> R<()> {
        self.regs[dst as usize] =
            self.field_addr(self.regs[base as usize], self.regs[layout as usize], n)?;
        Ok(())
    }

    /// A fault of op `k` of `code`: the IR pc of the op stays in the frame.
    #[cold]
    #[inline(never)]
    fn fault_at(&mut self, top: usize, code: &pre::Code, k: usize, e: Fault) -> Fault {
        self.frames[top].pc = code.at.get(k).copied().unwrap_or(0);
        e
    }

    /// `Op::LEq`, out of line: the planes of the `n` elements at the two addresses equal, without
    /// making either vector (bits beyond `n` are not compared).
    #[inline(never)]
    fn op_leq(&self, a: u32, b: u32, n: u32) -> R<u64> {
        let n = self.regs[n as usize];
        let bytes = n.div_ceil(64) * 32;
        let (x, y) = (
            self.rd(self.regs[a as usize], bytes)?,
            self.rd(self.regs[b as usize], bytes)?,
        );
        let tail = match n % 64 {
            0 => !0,
            r => (1u64 << r) - 1,
        };
        let last = bytes as usize / 32;
        let eq = (0..last).all(|blk| {
            let mask = if blk + 1 == last { tail } else { !0 };
            (0..4).all(|p| {
                let o = blk * 32;
                (plane_word(&x[o..], p) ^ plane_word(&y[o..], p)) & mask == 0
            })
        });
        Ok(eq.into())
    }

    /// `Op::LCopy`, out of line.
    #[inline(never)]
    fn op_lcopy(&mut self, dst: u32, doff: u32, src: u32, soff: u32, n: u32) -> R<()> {
        self.lcopy(
            self.regs[dst as usize],
            self.regs[doff as usize],
            self.regs[src as usize],
            self.regs[soff as usize],
            self.regs[n as usize],
        )
    }

    /// The iterations of an element loop (`pre::ElemLoop`) but the last, at its entry: when the
    /// loop counts by 1 or -1 to its bound and every position, index and element of those
    /// iterations is in range, they are done here (a copy of the elements or a fill) and the
    /// loop variable is set to where the generic ops go on; anything else leaves the loop to
    /// them from its first iteration, which then fail where and as they would have. Returns the
    /// iterations done, at most `limit`.
    #[inline(never)]
    fn op_loop(&mut self, code: &pre::Code, top: usize, id: u32, limit: u64) -> R<u64> {
        let lp = code
            .loops
            .get(id as usize)
            .ok_or_else(|| ice_f!("no element loop {id}"))?;
        let i0 = self.regs[lp.ind as usize] as i64;
        let bound = self.regs[lp.bound as usize] as i64;
        let delta = match lp.delta {
            pre::Delta::K(d) => d,
            pre::Delta::R(r) => self.regs[r as usize] as i64,
        };
        // The iterations that go round again, counted from the compare's outcomes: those with
        // the variable on the near side of the bound, then the one at the bound if that goes
        // round, and none if the far side does (the loop would run until it fails).
        let (near, far) = match delta {
            1 => (0b001, 0b100),
            -1 => (0b100, 0b001),
            _ => return Ok(0),
        };
        let x0 = if lp.after {
            i0.checked_add(delta)
        } else {
            Some(i0)
        };
        let dist = x0
            .and_then(|x| bound.checked_sub(x))
            .and_then(|d| d.checked_mul(delta));
        let again = match dist {
            Some(d) if d >= 0 => {
                if lp.cmask & near == 0 {
                    0
                } else if lp.cmask & 0b010 == 0 {
                    d
                } else if lp.cmask & far != 0 {
                    return Ok(0);
                } else {
                    d.saturating_add(1)
                }
            }
            Some(_) if lp.cmask & far == 0 => 0,
            _ => return Ok(0),
        };
        let m = again.min(limit.min(i64::MAX as u64) as i64);
        if m < 2 {
            return Ok(0);
        }
        let Some(plan) = self.loop_plan(lp, top, i0, delta, m) else {
            if code.trace {
                eprintln!("element loop {id}: no plan for {m} iterations");
            }
            return Ok(0);
        };
        let done = self.loop_exec(&plan, m)?;
        if code.trace {
            eprintln!("element loop {id}: {done} of {m} iterations at once");
        }
        if done > 0 {
            self.regs[lp.ind as usize] = i0.wrapping_add(delta * done) as u64;
        }
        Ok(done as u64)
    }

    /// Evaluate the steps of `lp` for iterations `0..m` (each value affine in the iteration
    /// number) and check every position, atom and element range at both ends; `None` when
    /// anything is not as the generic ops would find it.
    fn loop_plan(
        &self,
        lp: &pre::ElemLoop,
        top: usize,
        i0: i64,
        delta: i64,
        m: i64,
    ) -> Option<LoopPlan> {
        use pre::{LStep as S, V};
        let last = m - 1;
        // (value at iteration 0, change per iteration): the value at `last` fits i64 too.
        let fit = |c0: Option<i64>, c1: Option<i64>| -> Option<(i64, i64)> {
            let (c0, c1) = (c0?, c1?);
            c0.checked_add(c1.checked_mul(last)?)?;
            Some((c0, c1))
        };
        let atom = |a: (i64, i64)| -> Option<(i64, i64)> {
            (aff_ok(a.0) && aff_ok(a.0.checked_add(a.1.checked_mul(last)?)?)).then_some(a)
        };
        let mut val = [(0i64, 0i64); pre::LOOP_STEPS];
        let mut plan = None;
        let mut gets: Option<(u64, (i64, i64))> = None;
        // The memory the steps read besides the vectors: (space, object, first byte, end).
        let mut loads = [(0u8, 0u32, 0u64, 0u64); pre::LOOP_LOADS];
        let mut nloads = 0;
        for (j, st) in lp.steps.iter().enumerate() {
            let get = |v: V, val: &[(i64, i64); pre::LOOP_STEPS]| match v {
                V::Inv(r) => (self.regs[r as usize] as i64, 0),
                V::Ind => (i0, delta),
                V::Slot(s) => val[s as usize],
            };
            let r = match *st {
                S::Mov(v) => get(v, &val),
                S::K(k) => (k, 0),
                S::Bin(op, a, b, bound) => {
                    let (x, y) = (get(a, &val), get(b, &val));
                    let r = match op {
                        IntOp::Add => fit(x.0.checked_add(y.0), x.1.checked_add(y.1))?,
                        IntOp::Sub => fit(x.0.checked_sub(y.0), x.1.checked_sub(y.1))?,
                        _ => {
                            // One of them does not change.
                            let (k, v) = if x.1 == 0 { (x.0, y) } else { (y.0, x) };
                            fit(k.checked_mul(v.0), k.checked_mul(v.1))?
                        }
                    };
                    let (lo, hi) = match bound {
                        pre::Bound::None => (i64::MIN, i64::MAX),
                        pre::Bound::K(l, h) => (i64::from(l), i64::from(h)),
                        pre::Bound::R(l, h) => {
                            (self.regs[l as usize] as i64, self.regs[h as usize] as i64)
                        }
                    };
                    let end = r.0 + r.1 * last;
                    if !(lo <= r.0 && r.0 <= hi && lo <= end && end <= hi) {
                        return None;
                    }
                    r
                }
                // The atoms are within 2^30, the coefficients within 2^15 and 2^31.
                S::Lin1(a, ca, c0) => {
                    let x = atom(get(a, &val))?;
                    (c0 + ca * x.0, ca * x.1)
                }
                S::Lin2(a, b, ca, cb, c0) => {
                    let (x, y) = (atom(get(a, &val))?, atom(get(b, &val))?);
                    (c0 + ca * x.0 + cb * y.0, ca * x.1 + cb * y.1)
                }
                S::Pos { dope, dim, idx } => {
                    let x = get(idx, &val);
                    let d = self.dim_of(get(dope, &val).0 as u64, u64::from(dim)).ok()?;
                    loop_pos(d, x, last)?
                }
                S::PosL { dope, idx } => {
                    let x = get(idx, &val);
                    let d = self.dim_of(get(dope, &val).0 as u64, 0).ok()?;
                    yavs_ir::dim_len(d.0, d.1, d.2 == 1).filter(|&n| n <= i64::MAX as u64)?;
                    loop_pos(d, x, last)?
                }
                S::Len { dope, dim } => {
                    (self.dope_len(get(dope, &val).0 as u64, dim).ok()? as i64, 0)
                }
                S::Ld(base, off, w) => {
                    let a = add_addr(get(base, &val).0 as u64, u64::from(off)).ok()?;
                    let v = self.load(a, w).ok()?;
                    let (sp, idx, o) = split(a);
                    loads[nloads] = (sp, idx, o, o + u64::from(w.bytes()));
                    nloads += 1;
                    (v as i64, 0)
                }
                S::LdSlot(off, w) => {
                    let a = mk_addr(FRAME, top as u32, u64::from(off)).ok()?;
                    let v = self.load(a, w).ok()?;
                    loads[nloads] = (
                        FRAME,
                        top as u32,
                        u64::from(off),
                        u64::from(off + w.bytes()),
                    );
                    nloads += 1;
                    (v as i64, 0)
                }
                S::Addr(base, off) => (
                    add_addr(get(base, &val).0 as u64, u64::from(off)).ok()? as i64,
                    0,
                ),
                S::Cmp(m, a, b) => (
                    i64::from(pre::cmp_holds(m, get(a, &val).0, get(b, &val).0)),
                    0,
                ),
                S::Sel(c, a, b) => {
                    if get(c, &val).0 != 0 {
                        get(a, &val)
                    } else {
                        get(b, &val)
                    }
                }
                S::Get { base, idx } => {
                    let (b, x) = (get(base, &val), get(idx, &val));
                    if b.1 != 0 {
                        return None;
                    }
                    gets = Some((b.0 as u64, x));
                    (0, 0)
                }
                S::Set { base, idx, v } => {
                    let (b, x) = (get(base, &val), get(idx, &val));
                    if b.1 != 0 {
                        return None;
                    }
                    let dst = self.loop_stream(b.0 as u64, x, last, true)?;
                    // Nothing it read is among the elements it writes.
                    let (dsp, didx, doff) = split(dst.base);
                    let e1 = dst.e0 + dst.step * last;
                    let (lo, hi) = (
                        doff + dst.e0.min(e1) as u64 / 64 * 32,
                        doff + dst.e0.max(e1) as u64 / 64 * 32 + 32,
                    );
                    if loads[..nloads]
                        .iter()
                        .any(|&(sp, idx, a, z)| (sp, idx) == (dsp, didx) && a < hi && lo < z)
                    {
                        return None;
                    }
                    plan = Some(match (v, gets) {
                        (V::Slot(_), Some((sb, sx))) => {
                            LoopPlan::Copy(self.loop_stream(sb, sx, last, false)?, dst)
                        }
                        (V::Inv(r), _) => {
                            let code = self.regs[r as usize];
                            if code > 8 {
                                return None;
                            }
                            LoopPlan::Fill(dst, code as u8)
                        }
                        _ => return None,
                    });
                    (0, 0)
                }
            };
            val[j] = r;
        }
        plan
    }

    /// The elements `e0 + step * n` of the logic vector at `base`, `n` up to `last`: all in
    /// memory of the common spaces (for writing, those that may be written).
    fn loop_stream(&self, base: u64, e: (i64, i64), last: i64, write: bool) -> Option<LoopStream> {
        let e1 = e.0.checked_add(e.1.checked_mul(last)?)?;
        if e.0 < 0 || e1 < 0 {
            return None;
        }
        let (sp, idx, off) = split(base);
        if write && !matches!(sp, FRAME | TEMP | HEAP | SHARED_HEAP | SHARED) {
            return None;
        }
        let mem = self.mem_fast(sp, idx)?;
        let end = off.checked_add(e.0.max(e1) as u64 / 64 * 32 + 32)?;
        (end <= mem.len() as u64).then_some(LoopStream {
            base,
            e0: e.0,
            step: e.1,
        })
    }

    /// Run the iterations `plan` stands for (`m` of them); returns how many were done.
    fn loop_exec(&mut self, plan: &LoopPlan, m: i64) -> R<i64> {
        match *plan {
            LoopPlan::Fill(d, code) => {
                let at = |n: i64| d.e0 + d.step * n;
                if d.step.abs() <= 1 {
                    // One run of elements.
                    let lo = at(0).min(at(m - 1));
                    let n = if d.step == 0 { 1 } else { m };
                    self.lfill(d.base, lo as u64, n as u64, u64::from(code))?;
                    return Ok(m);
                }
                let (sp, idx, off) = split(d.base);
                let Some(mem) = self.mem_mut_fast(sp, idx) else {
                    return Ok(0);
                };
                for n in 0..m {
                    if !lset_in(mem, off as usize, at(n) as u64, code) {
                        return Ok(n);
                    }
                }
                Ok(m)
            }
            LoopPlan::Copy(s, d) => {
                let (ssp, sidx, soff) = split(s.base);
                let (dsp, didx, doff) = split(d.base);
                let same = (ssp, sidx) == (dsp, didx);
                let span = |x: &LoopStream, off: u64| {
                    let (a, b) = (x.e0, x.e0 + x.step * (m - 1));
                    let (lo, hi) = (a.min(b) as u64, a.max(b) as u64);
                    (off + lo / 64 * 32, off + hi / 64 * 32 + 32)
                };
                let (sr, dr) = (span(&s, soff), span(&d, doff));
                let apart = !same || sr.1 <= dr.0 || dr.1 <= sr.0;
                if s.step == d.step && s.step.abs() == 1 && apart {
                    // The same run of elements, in the same order: a copy of planes.
                    let (sa, da) = if s.step == 1 {
                        (s.e0, d.e0)
                    } else {
                        (s.e0 - (m - 1), d.e0 - (m - 1))
                    };
                    if self
                        .copy_in_block(d.base, da as u64, s.base, sa as u64, m as u64)
                        .is_none()
                    {
                        self.lcopy(d.base, da as u64, s.base, sa as u64, m as u64)?;
                    }
                    return Ok(m);
                }
                let (es, ed) = (|n: i64| s.e0 + s.step * n, |n: i64| d.e0 + d.step * n);
                if same {
                    let Some(mem) = self.mem_mut_fast(dsp, didx) else {
                        return Ok(0);
                    };
                    for n in 0..m {
                        let Some(c) = lget_in(mem, soff as usize, es(n) as u64) else {
                            return Ok(n);
                        };
                        if c > 8 || !lset_in(mem, doff as usize, ed(n) as u64, c) {
                            return Ok(n);
                        }
                    }
                    return Ok(m);
                }
                // Different objects: a run of codes read, then written.
                let mut buf = [0u8; 64];
                let mut n = 0;
                while n < m {
                    let c = (m - n).min(64);
                    let Some(sm) = self.mem_fast(ssp, sidx) else {
                        return Ok(n);
                    };
                    let mut got = 0;
                    while got < c {
                        match lget_in(sm, soff as usize, es(n + got) as u64) {
                            Some(x) if x <= 8 => buf[got as usize] = x,
                            _ => break,
                        }
                        got += 1;
                    }
                    let Some(dm) = self.mem_mut_fast(dsp, didx) else {
                        return Ok(n);
                    };
                    for j in 0..got {
                        if !lset_in(dm, doff as usize, ed(n + j) as u64, buf[j as usize]) {
                            return Ok(n + j);
                        }
                    }
                    n += got;
                    if got < c {
                        return Ok(n);
                    }
                }
                Ok(m)
            }
        }
    }

    /// `n` elements from element `se` of the logic vector at `s` to element `de` of the one at
    /// `d`, when each run lies inside one block of planes and the two are not the same bytes
    /// (the caller checked): the planes of the run moved as words. `None` leaves it to `lcopy`.
    #[inline(always)]
    fn copy_in_block(&mut self, d: u64, de: u64, s: u64, se: u64, n: u64) -> Option<()> {
        if n > 64 || se % 64 + n > 64 || de % 64 + n > 64 {
            return None;
        }
        let mask = if n == 64 { !0 } else { (1u64 << n) - 1 };
        let (ssp, sidx, soff) = split(s);
        let sb = soff as usize + (se / 64) as usize * 32;
        let b = self.mem_fast(ssp, sidx)?.get(sb..sb + 32)?;
        let w: [u64; 4] = std::array::from_fn(|p| plane_word(b, p) >> (se % 64) & mask);
        let (dsp, didx, doff) = split(d);
        let db = doff as usize + (de / 64) as usize * 32;
        let sh = de % 64;
        let b = self.mem_mut_fast(dsp, didx)?.get_mut(db..db + 32)?;
        for (p, w) in w.iter().enumerate() {
            let v = plane_word(b, p) & !(mask << sh) | w << sh;
            b[p * 8..p * 8 + 8].copy_from_slice(&v.to_le_bytes());
        }
        Some(())
    }

    /// The object of the spaces that may be written all the time, without error paths.
    #[inline(always)]
    fn mem_mut_fast(&mut self, sp: u8, idx: u32) -> Option<&mut [u8]> {
        match sp {
            FRAME => self.fslots_mut(idx as usize),
            TEMP => Some(&mut self.temp),
            HEAP => self.heap.data_mut(idx),
            SHARED_HEAP | SHARED => self.shared_mem_mut(sp, idx),
            _ => None,
        }
    }

    /// The iterations of a search loop (`pre::SCAN`) while neither exit is taken: the position
    /// of the index in the descriptor, the element there, its compare with `x`, the compare of
    /// the index with `e` and the step. The descriptor and the array are looked up once.
    /// Stops at the first thing the generic ops should handle (an index outside the bounds,
    /// memory out of the common spaces, an overflow): the result says how far it got.
    #[inline(never)]
    fn scan_loop(&self, p: &ScanLoop, i0: i64, x: i64, e: i64, limit: u32) -> Scan {
        let none = Scan {
            i: i0,
            ..Scan::default()
        };
        let Ok(dim) = self.dim_of(p.dope, u64::from(p.dim)) else {
            return none;
        };
        let (sp, sidx, soff) = split(p.base);
        let Some(mem) = self.mem_fast(sp, sidx) else {
            return none;
        };
        let shape = ScanShape {
            stride: p.stride,
            w: p.w,
            m: p.m,
            m2: p.m2,
            idx_left: p.idx_left,
            delta: p.delta,
        };
        crate::scan::scan_bytes(&shape, dim, mem, soff, ScanFrom { i0, x, e, limit })
    }

    /// Run the compiled code of frame `top` (`func`, `f`) from op `k` (`run_ops`, kept out of
    /// its loop): the op to go on at and whether compiled code may run again before the next
    /// jump (`k << 1 | again`), or the flow the run ends with (frames compiled code left,
    /// `TIER_GO_CALL`; a suspension, `TIER_GO_SUSPEND`). One word, not nested enums: copying
    /// those out of the return slot stalled on the callee's narrower stores.
    #[inline(never)]
    #[allow(clippy::too_many_arguments)]
    fn tier_go(
        &mut self,
        tier: &dyn crate::Tier,
        rt: &Runtime,
        func: FuncId,
        f: &Func,
        code: &pre::Code,
        top: usize,
        k: usize,
        ctx: &mut Option<&mut Ctx<'_>>,
    ) -> R<u64> {
        // The registers of a waveform buffer that is not run are undefined until the op that
        // drives it: not an entry for compiled code (it is asked again after the next jump).
        let at = code.no_tier.partition_point(|r| r.1 < k as u32);
        if code.no_tier.get(at).is_some_and(|r| r.0 <= k as u32) {
            return Ok((k as u64) << 1);
        }
        let pc = code.at.get(k).copied().unwrap_or(0) as usize;
        let prog = &*rt.prog;
        let (next, ran) = self.run_tier(tier, rt, f, pc, ctx)?;
        // Frames compiled code pushed and did not finish: the interpreter goes on with the
        // top one.
        if next == crate::TIER_UNWIND as usize {
            return Ok(TIER_GO_CALL);
        }
        if next == crate::TIER_SUSPEND as usize {
            return Ok(TIER_GO_SUSPEND);
        }
        // No progress: `pc` is no entry of the compiled code; interpret to the next jump.
        let native = ran && next != pc;
        Ok(match self.op_at(prog, f, code, top, next, ctx)? {
            Ok((to, true)) => (to as u64) << 1 | u64::from(tier.ready(func)),
            Ok((to, false)) => (to as u64) << 1 | u64::from(native),
            Err(Flow::Call) => TIER_GO_CALL,
            Err(Flow::Suspend) => TIER_GO_SUSPEND,
            Err(_) => return Err(ice_s!("compiled code left a jump or a return to step")),
        })
    }

    /// An `Aff` op that cannot take its fast path (an atom beyond 2^30, an index out of range,
    /// a range too long): the instructions it stands for, run as the IR has them up to the one
    /// defining the op's result (a failure is theirs, at their pc).
    #[cold]
    #[inline(never)]
    fn aff_replay(
        &mut self,
        prog: &Program,
        f: &Func,
        code: &pre::Code,
        top: usize,
        k: usize,
        ctx: &mut Option<&mut Ctx<'_>>,
    ) -> R<u64> {
        let &(_, from, root) = code
            .aff
            .binary_search_by_key(&(k as u32), |e| e.0)
            .ok()
            .and_then(|i| code.aff.get(i))
            .ok_or_else(|| ice_s!("no source run for a linear op"))?;
        let mut result = 0;
        for pc in from..=root {
            let inst = *f
                .code
                .get(pc as usize)
                .ok_or_else(|| ice_f!("pc {pc} beyond `{}`", f.name))?;
            self.frames[top].pc = pc;
            if !matches!(self.step_out(prog, f, inst, ctx)?, Flow::Next) {
                return Err(ice_s!("a linear run left its straight line"));
            }
            if let Inst::Bin { dst, .. } | Inst::Pos { dst, .. } | Inst::DopeLen { dst, .. } = inst
            {
                result = self.regs[dst.0 as usize];
            }
        }
        Ok(result)
    }

    /// The length of the first dimension of the descriptor at `dope` (`pos_len`).
    #[inline(always)]
    fn dope_len_of(&self, dope: u64) -> Option<i64> {
        let d = self.dim_of(dope, 0).ok()?;
        Some(dim_len63(d).filter(|&n| n <= i64::MAX as u64)? as i64)
    }

    /// The position of index `i` in the first dimension of the descriptor at `dope` (`pos_len`
    /// without the length, which a position does not depend on); `None` when the index is
    /// outside the range or the descriptor cannot be read.
    #[inline(always)]
    fn pos_first(&self, dope: u64, i: i64) -> Option<i64> {
        let (l, rr, dir) = self.dim_of(dope, 0).ok()?;
        match dir {
            0 if l <= i && i <= rr => Some(i.wrapping_sub(l)),
            1 if rr <= i && i <= l => Some(l.wrapping_sub(i)),
            _ => None,
        }
    }

    /// The position of index `i` in the first dimension of the descriptor at `dope` and its
    /// length; `None` when the index is outside the range, the length does not fit 63 bits or
    /// the descriptor cannot be read (the instructions then give the error).
    #[inline(always)]
    fn pos_len(&self, dope: u64, i: i64) -> Option<(i64, i64)> {
        let (l, rr, dir) = self.dim_of(dope, 0).ok()?;
        let n = dim_len63((l, rr, dir)).filter(|&n| n <= i64::MAX as u64)? as i64;
        let pos = match dir {
            0 if l <= i && i <= rr => i.wrapping_sub(l),
            1 if rr <= i && i <= l => l.wrapping_sub(i),
            _ => return None,
        };
        Some((pos, n))
    }

    /// The op of `code` (function `f`, frame `top`) that starts at IR pc `pc`, after stepping
    /// the IR instructions from `pc` up to the next op when `pc` lies inside a fused op
    /// (compiled code stopped there), and whether a jump was taken on the way; or the flow an
    /// instruction ended the run with.
    #[allow(clippy::too_many_arguments)]
    fn op_at(
        &mut self,
        prog: &Program,
        f: &Func,
        code: &pre::Code,
        top: usize,
        mut pc: usize,
        ctx: &mut Option<&mut Ctx<'_>>,
    ) -> R<Result<(usize, bool), Flow>> {
        let mut jumped = false;
        while code.entry.get(pc).is_some_and(|&o| o == pre::NONE) {
            let inst = *f
                .code
                .get(pc)
                .ok_or_else(|| ice_f!("pc {pc} beyond `{}`", f.name))?;
            self.frames[top].pc = pc as u32;
            match self.step_out(prog, f, inst, ctx)? {
                Flow::Next => pc += 1,
                Flow::Jump(b) => {
                    pc = f.blocks.get(b).ok_or_else(|| ice_f!("no block {b}"))?.start as usize;
                    jumped = true;
                }
                flow => return Ok(Err(flow)),
            }
        }
        match code.entry.get(pc) {
            Some(&k) if k != pre::NONE => Ok(Ok((k as usize, jumped))),
            _ => Err(ice_f!("pc {pc} of `{}` beyond its code", f.name)),
        }
    }

    /// `Call` (`CallProt` on `obj`) of `callee` from the top frame through `Tier::call`: the flow
    /// it ends with (`Flow::Next` when the call returned, its result in `dst`), or `None` when the
    /// interpreter makes the call.
    #[inline(never)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn tier_call(
        &mut self,
        tier: &dyn crate::Tier,
        rt: &Runtime,
        f: &Func,
        callee: FuncId,
        cf: &Func,
        args: yavs_ir::Regs,
        dst: Option<Reg>,
        obj: Option<u64>,
        ctx: &mut Option<&mut Ctx<'_>>,
    ) -> R<Option<Flow>> {
        // One cell per parameter, two for a signal (`call_with`).
        let mut cells = [0u64; 16];
        let mut n = usize::from(obj.is_some());
        cells[0] = obj.unwrap_or(0);
        let regs = f
            .regs(args)
            .ok_or_else(|| ice_s!("operand list out of range"))?;
        for (r, p) in regs.iter().zip(cf.params.iter().skip(n)) {
            let w = 1 + usize::from(p.width == Width::Sig);
            if n + w > cells.len() {
                return Ok(None);
            }
            cells[n] = self.regs[r.0 as usize];
            if w == 2 {
                cells[n + 1] = self.regs[r.0 as usize + 1];
            }
            n += w;
        }
        let pc = self.top().pc;
        let mut host = Host {
            m: self,
            rt,
            prog: &rt.prog,
            f,
            up: Vec::new(),
            ctx,
            fault: None,
            stop: false,
        };
        // A profile names the callee while its native frame runs (`Runtime::profile_sampler`).
        let marked = crate::PROFILE_NATIVE.load(Ordering::Relaxed).then(|| {
            let slot = crate::pc_slot();
            let mark = u64::from(callee.0) << 32 | u64::from(crate::NATIVE_PC);
            (slot, slot.swap(mark, Ordering::Relaxed))
        });
        let r = tier.call(callee, &cells[..n], obj, pc, &mut host);
        if let Some((slot, before)) = marked {
            slot.store(before, Ordering::Relaxed);
        }
        if let Some(e) = host.fault {
            return Err(e);
        }
        Ok(match r {
            None => None,
            Some(crate::Called::Done(v)) => {
                if let Some(d) = dst {
                    self.regs[d.0 as usize] = v;
                }
                Some(Flow::Next)
            }
            Some(crate::Called::Pushed) => Some(Flow::Call),
            Some(crate::Called::Suspend) => Some(Flow::Suspend),
            Some(crate::Called::Fault) => {
                return Err(ice_s!("compiled call failed without a fault"));
            }
        })
    }

    /// Run compiled code of the top frame from `pc` (ir.md 5): the index of the instruction
    /// the interpreter executes next (or `TIER_UNWIND`, `TIER_SUSPEND`, frames' pcs set), and
    /// whether the function has compiled code. Compiled code steps other instructions through
    /// `TierHost::step`; the frames those push it runs on or leaves to the interpreter.
    fn run_tier(
        &mut self,
        tier: &dyn crate::Tier,
        rt: &Runtime,
        f: &Func,
        pc: usize,
        ctx: &mut Option<&mut Ctx<'_>>,
    ) -> R<(usize, bool)> {
        let func = self.top().func;
        let mut host = Host {
            m: self,
            rt,
            prog: &rt.prog,
            f,
            up: Vec::new(),
            ctx,
            fault: None,
            stop: false,
        };
        let ran = tier.run(func, pc as u32, &mut host);
        let fault = host.fault;
        let pc = match ran {
            Some(p) if p == crate::TIER_UNWIND || p == crate::TIER_SUSPEND => p as usize,
            // A fault leaves the pc where it was recorded (`TierHost::signal` may record it at
            // the instruction after the one compiled code called it for).
            Some(p) if fault.is_some() => p as usize,
            Some(p) => {
                self.top().pc = p;
                p as usize
            }
            None => pc,
        };
        match fault {
            Some(e) => Err(e),
            None => Ok((pc, ran.is_some())),
        }
    }

    /// The top frame's slots (`TierFrame::slots`).
    fn slots_ptr(&mut self) -> *mut u8 {
        let top = self.frames.len() - 1;
        let sbase = self.frames[top].sbase as usize;
        let slots = if top == 0 {
            &mut self.root
        } else {
            &mut self.sstack
        };
        slots.as_mut_ptr().wrapping_add(sbase)
    }

    /// The view of a process in its root frame, every field written straight from the machine's
    /// fields (`TierHost::view_again`); `false` (nothing written) when it is not one.
    #[inline]
    fn view_root(&mut self, tf: &mut crate::TierFrame, c: &Ctx<'_>) -> bool {
        let m = self;
        if m.frames.len() != 1 || m.vc.tier_sigs.len() != m.vc.nsigs {
            return false;
        }
        let fr = &m.frames[0];
        let (rbase, slen, temp_base) = (fr.rbase as usize, fr.slen, fr.temp_base);
        let shared_alloc = fr.shared_alloc;
        tf.slots = m.root.as_mut_ptr().wrapping_add(fr.sbase as usize);
        tf.slots_len = u64::from(slen);
        // `tier_view` writes these three together: the root frame's values stay until a deeper
        // frame's view replaces them.
        const ROOT: u64 = (FRAME as u64) << 29;
        if tf.frame_hi != ROOT {
            tf.frame = 0;
            tf.frame_hi = ROOT;
            tf.up = [u64::MAX; 2];
        }
        tf.inst = m.vc.inst_data as *const u8;
        tf.inst_len = m.vc.inst_len as u64;
        tf.nsigs = m.vc.nsigs as u64;
        tf.sigtab = m.vc.tier_sigs.as_ptr();
        tf.sigtab_len = m.vc.tier_sigs.len() as u64;
        m.reserve_temp();
        tf.temp = m.temp.as_mut_ptr();
        tf.temp_len = m.temp.len() as u64;
        tf.temp_cap = m.temp.capacity().min(u32::MAX as usize) as u64;
        tf.temp_base = u64::from(temp_base);
        tf.temp_roots = m.troots.len() as u64;
        tf.regs = m.regs.v.as_mut_ptr().wrapping_add(rbase);
        tf.nregs = (m.regs.v.len() - rbase) as u64;
        tf.stack = m.regs.v.as_ptr();
        tf.max_depth = m.vc.max_depth;
        tf.owner = m.vc.owner;
        m.tier_cov(tf);
        tf.stop = c.interrupt_flags();
        tf.spin = &raw mut m.spin;
        tf.drivers = m.drivers.as_ptr();
        tf.ndrivers = m.drivers.len() as u64;
        // A process that is `hot` has no shared state (`sh`), which the call paths of a serial
        // process do not promise: its epoch and flags come from the heap and the frame alone.
        m.tier_heaps(tf);
        let flags = if m.hot {
            u64::from(shared_alloc) * crate::TIER_SHARED_ALLOC
                + u64::from(STAMPS.load(Ordering::Relaxed)) * crate::TIER_STAMPS
        } else {
            m.tier_flags()
        };
        // `frames.len() == 1` above; `tier_flags` (deeper views) never sets it.
        let root_wait = m.root_process && m.dbg.is_none();
        tf.flags = flags | (u64::from(root_wait) * crate::TIER_ROOT_WAIT);
        tf.cycle = c.cycle();
        tf.known = c.resumed_edge_key();
        true
    }

    /// `TierFrame::flags` of the top frame.
    fn tier_flags(&self) -> u64 {
        let shared = self.frames.last().is_some_and(|f| f.shared_alloc);
        u64::from(shared) * crate::TIER_SHARED_ALLOC
            + u64::from(self.sh.is_some()) * crate::TIER_SERIAL
            + u64::from(STAMPS.load(Ordering::Relaxed)) * crate::TIER_STAMPS
    }

    /// Whether a native call or an allocation would collect or recycle now (`reclaim`): native
    /// frames hold their registers outside the machine, so compiled code leaves them to the
    /// interpreter first.
    fn reclaim_pending(&self) -> bool {
        let t = self.rt.limits.gc_threshold;
        if self.hot {
            return self.heap.gc_due(t) || self.heap.reclaim_due();
        }
        let shared = self.sh.as_ref().and_then(|sh| sh.heap.as_deref());
        let started = self.sh.as_ref().is_none_or(|sh| sh.started);
        started && (self.heap.gc_due(t) || shared.is_some_and(|h| h.gc_due(t)))
            || self.heap.reclaim_due()
            || shared.is_some_and(Heap::reclaim_due)
    }

    /// The coverage counters of this machine's instance in the view.
    #[inline]
    fn tier_cov(&self, tf: &mut crate::TierFrame) {
        // Compiled `Inst::Cover` exists only with counters, so without them the view is not read.
        if self.rt.cov.is_empty() {
            return;
        }
        let cov = self
            .rt
            .cov
            .get(self.inst.cov_base as usize..)
            .unwrap_or(&[]);
        tf.cov = cov.as_ptr().cast();
        tf.cov_len = cov.len() as u64;
    }

    /// The temporary area has room for a statement's small allocations when compiled code takes
    /// its view: the interpreter's fused ops (a waveform, a string) never grow it, so a
    /// capacity of 0 made every `Alloc` of compiled code a step, or the end of a native frame.
    #[inline(always)]
    fn reserve_temp(&mut self) {
        const TIER_TEMP: usize = 512;
        if self.temp.capacity() < TIER_TEMP {
            self.temp.reserve(TIER_TEMP - self.temp.len());
        }
    }

    /// Make `tf` the top frame's view (`TierFrame`).
    fn tier_view(&mut self, tf: &mut crate::TierFrame) {
        let top = self.frames.len() - 1;
        let fr = &self.frames[top];
        let (rbase, slen, temp_base) = (fr.rbase as usize, fr.slen, fr.temp_base);
        tf.slots = self.slots_ptr();
        tf.slots_len = slen as u64;
        tf.frame = top as u64;
        tf.frame_hi = u64::from(FRAME) << 29 | top as u64;
        self.reserve_temp();
        tf.temp = self.temp.as_mut_ptr();
        tf.temp_len = self.temp.len() as u64;
        tf.temp_cap = self.temp.capacity().min(u32::MAX as usize) as u64;
        tf.temp_base = temp_base as u64;
        tf.temp_roots = self.troots.len() as u64;
        tf.inst = self.inst.data.as_ptr();
        tf.inst_len = self.inst.data.len() as u64;
        tf.nsigs = self.inst.sigs.len() as u64;
        tf.regs = self.regs.v.as_mut_ptr().wrapping_add(rbase);
        tf.nregs = (self.regs.v.len() - rbase) as u64;
        tf.stack = self.regs.v.as_ptr();
        tf.max_depth = u64::from(self.rt.limits.max_depth);
        self.tier_heaps(tf);
        tf.flags = self.tier_flags();
        tf.owner = self.vc.owner;
        let cov = self
            .rt
            .cov
            .get(self.inst.cov_base as usize..)
            .unwrap_or(&[]);
        tf.cov = cov.as_ptr().cast();
        tf.cov_len = cov.len() as u64;
        let mut link = self.frames[top].link;
        for k in 0..2 {
            tf.up_len[k] = 0;
            tf.up[k] = match link {
                Some(i) if i < 1 << 29 => {
                    link = self.frames.get(i as usize).and_then(|f| f.link);
                    // Not the slots of a frame a native window holds elsewhere.
                    let native = self.nwin.is_some()
                        && (self.nwin_from..=self.nwin_to).contains(&(i as usize));
                    if !native && let Some(s) = self.fslots_mut(i as usize) {
                        tf.up_slots[k] = s.as_mut_ptr();
                        tf.up_len[k] = s.len() as u64;
                    }
                    u64::from(FRAME) << 29 | u64::from(i)
                }
                _ => {
                    link = None;
                    u64::MAX
                }
            };
        }
    }

    fn debug_tick(
        &mut self,
        prog: &Program,
        f: &Func,
        pc: usize,
        inst: &Inst,
        ctx: &mut Option<&mut Ctx<'_>>,
    ) -> R<()> {
        let Some(d) = self.dbg.as_mut() else {
            return Ok(());
        };
        d.fuel = d.fuel.wrapping_add(1);
        if d.fuel.is_multiple_of(4096) && d.session.pause_pending() {
            d.mode = StepMode::In;
        }
        if !matches!(inst, Inst::Stmt) {
            return Ok(());
        }
        let session = d.session.clone();
        let index = d.index;
        let stmt = f.loc[pc];
        let depth = self.frames.len();
        let mut reason = None;
        if session.take_pause() {
            reason = Some(StopReason::Pause);
        } else if session.take_global_step() {
            reason = Some(StopReason::GlobalStep);
        } else if d.mode.stops(depth) {
            reason = Some(StopReason::Step);
        }
        if reason.is_none()
            && let Some(bp) = session.line(stmt)
        {
            let held = match bp.cond {
                None => Ok(true),
                Some(c) => self.eval_cond(c, ctx).map(|v| v != 0),
            };
            match held {
                Ok(true) if bp.hit() => {
                    reason = Some(StopReason::Breakpoint { hits: bp.hits() });
                }
                Ok(_) => {}
                Err(e) => {
                    reason = Some(StopReason::ConditionError(self.error(e, None).to_string()))
                }
            }
        }
        let Some(reason) = reason else {
            return Ok(());
        };
        session.update(index, |s| s.state = ProcState::Running);
        let c = ctx.as_deref();
        let stop = Stop {
            reason,
            stmt,
            process: index,
            now: c.map(|c| c.now()),
            delta: c.map(|c| c.delta()),
            frames: FrameView {
                m: std::cell::RefCell::new(&mut *self),
                prog,
            },
            ctx: c,
        };
        let resume = session.stop(&stop);
        if resume == crate::Resume::Skip {
            return Ok(());
        }
        if resume == crate::Resume::GlobalStep {
            session.request_global_step();
        }
        if let Some(d) = self.dbg.as_mut() {
            d.mode = StepMode::after(resume, depth);
        }
        Ok(())
    }

    /// Evaluate a breakpoint condition: a function nested in the current frame's function.
    fn eval_cond(&mut self, c: FuncId, ctx: &mut Option<&mut Ctx<'_>>) -> R<u64> {
        let rt = self.rt.clone();
        let f = rt
            .prog
            .func(c)
            .ok_or_else(|| ice_f!("no condition function {c}"))?;
        let link = self.static_link(f, None)?;
        let base = self.frames.len();
        self.push_frame(f, c, link, None)?;
        let r = self.exec::<false>(ctx, base);
        self.truncate_frames(base);
        match r? {
            Outcome::Returned(Some(v)) => Ok(v),
            _ => Err(ice_s!("breakpoint condition did not return a value")),
        }
    }

    fn static_link(&self, callee: &Func, _caller: Option<usize>) -> R<Option<u32>> {
        let Some(parent) = callee.parent else {
            return Ok(None);
        };
        let mut k = self.frames.len().checked_sub(1);
        while let Some(i) = k {
            let fr = &self.frames[i];
            // (The root frame of a process that switched to a specialised copy holds the copy.)
            if fr.func == parent || i == 0 && self.rt.prog.base_of(fr.func) == parent {
                return Ok(Some(i as u32));
            }
            // Search the caller's static chain, then outward along the dynamic stack.
            k = match fr.link {
                Some(l) if (l as usize) < i => Some(l as usize),
                _ => i.checked_sub(1),
            };
        }
        Err(ice_f!(
            "frame of {parent}, the parent of `{}`, is not on the stack",
            callee.name
        ))
    }

    /// The symbol of native `native` (for messages).
    #[cold]
    fn native_symbol(prog: &Program, native: u16) -> &str {
        prog.natives
            .get(yavs_ir::NativeId(u32::from(native)))
            .map_or("?", |n| n.symbol.as_str())
    }

    /// What `Op::CallIeee` does when the native gave the call back to the VHDL body
    /// (`ieee_fallback`) or ended the process (`ieee_stop`); clears both.
    #[cold]
    fn ieee_after(
        &mut self,
        prog: &Program,
        native: u16,
        cells: &[u64],
        dst: u16,
        v: Option<u64>,
    ) -> R<Flow> {
        let dst = (dst != pre::NONE16).then_some(Reg(u32::from(dst)));
        if std::mem::take(&mut self.ieee_fallback) {
            let symbol = Self::native_symbol(prog, native);
            return self.ieee_body(prog, symbol, cells, dst);
        }
        if let Some(d) = dst {
            let v = v.ok_or_else(|| {
                ice_f!(
                    "native `{}` returned no value",
                    Self::native_symbol(prog, native)
                )
            })?;
            self.set(d, v);
        }
        if std::mem::take(&mut self.ieee_stop) {
            return Ok(Flow::Suspend);
        }
        Ok(Flow::Next)
    }

    /// Call the VHDL body of a native `ieee` subprogram that gave its call back: the symbol
    /// ends in `@<function id>`, array arguments become data and descriptor.
    #[inline(never)]
    fn ieee_body(
        &mut self,
        prog: &Program,
        symbol: &str,
        cells: &[u64],
        dst: Option<Reg>,
    ) -> R<Flow> {
        let (key, id) = symbol
            .strip_prefix("yavs.ieee.")
            .and_then(|k| k.rsplit_once('@'))
            .ok_or_else(|| ice_s!("ieee fallback without a body"))?;
        let func = FuncId(
            id.parse()
                .map_err(|_| ice_s!("ieee fallback function id"))?,
        );
        let mut out = Vec::with_capacity(cells.len());
        let (mut pos, mut i) = (0, 0);
        while pos < cells.len() {
            if yavs_ir::ieee::array_param(key, i) {
                let &[data, left, right, dir] = cells
                    .get(pos..pos + yavs_ir::ieee::ARRAY_REGS)
                    .ok_or_else(|| ice_s!("ieee fallback array argument"))?
                else {
                    return Err(ice_s!("ieee fallback array argument"));
                };
                // The body takes the descriptor in memory.
                let dope = self.temp_alloc(yavs_ir::DOPE_DIM_BYTES)?;
                let w = self.wr(dope, yavs_ir::DOPE_DIM_BYTES)?;
                for (d, v) in w.as_chunks_mut::<8>().0.iter_mut().zip([left, right, dir]) {
                    d.copy_from_slice(&v.to_le_bytes());
                }
                out.extend([data, dope]);
                pos += yavs_ir::ieee::ARRAY_REGS;
            } else {
                out.push(cells[pos]);
                pos += 1;
            }
            i += 1;
        }
        self.call(prog, func, &out, dst)
    }

    fn call(&mut self, prog: &Program, func: FuncId, cells: &[u64], dst: Option<Reg>) -> R<Flow> {
        let f = prog
            .func(func)
            .ok_or_else(|| ice_f!("call of missing function {func}"))?;
        let link = self.static_link(f, None)?;
        self.push_frame(f, func, link, dst)?;
        let mut k = 0;
        for p in &f.params {
            let n = if p.width == Width::Sig { 2 } else { 1 };
            let c = cells
                .get(k..k + n)
                .ok_or_else(|| ice_f!("missing argument {}", p.name))?;
            store_cells(self.top_slots(), p.slot, p.width, c)?;
            k += n;
        }
        Ok(Flow::Call)
    }

    /// `Inst::CallProt`: a method of the protected object `obj` designates.
    fn call_prot(
        &mut self,
        prog: &Program,
        f: &Func,
        func: FuncId,
        obj: Reg,
        args: yavs_ir::Regs,
        dst: Option<Reg>,
    ) -> R<Flow> {
        self.shared()?;
        let h = self.r(obj);
        let flow = self.call_with(prog, f, func, Some(h), args, dst)?;
        if let Flow::Call = flow {
            self.top().shared_alloc |= heap::tag_of(h) == heap::SHARED;
        }
        Ok(flow)
    }

    /// `Inst::Fill`: `n` copies of the `size` bytes at `elem` from `dst` on.
    fn fill(&mut self, dst: Reg, elem: Reg, n: Reg, size: u32) -> R<()> {
        let mut e = self.take_bytes();
        let r = self
            .rd(self.r(elem), u64::from(size))
            .map(|s| e.extend_from_slice(s))
            .and_then(|()| {
                let total = self
                    .r(n)
                    .checked_mul(u64::from(size))
                    .ok_or_else(|| runtime_s!("object larger than 4 GiB"))?;
                let out = self.wr(self.r(dst), total)?;
                for c in out.chunks_exact_mut(size.max(1) as usize) {
                    c.copy_from_slice(&e);
                }
                Ok(())
            });
        self.give_bytes(e);
        r
    }

    /// `Inst::Call` of a function by its IR id (compiled code's calls, `TierHost::step`).
    fn call_regs(
        &mut self,
        prog: &Program,
        f: &Func,
        func: FuncId,
        args: yavs_ir::Regs,
        dst: Option<Reg>,
        rt: &Runtime,
    ) -> R<Flow> {
        let callee = prog
            .func(func)
            .ok_or_else(|| ice_f!("call of missing function {func}"))?;
        let pre = rt
            .pre(func, callee)
            .ok_or_else(|| ice_f!("no function {func}"))?;
        self.call_pre(f, callee, func, pre, args, dst, None)?;
        Ok(Flow::Call)
    }

    /// `Inst::Call` of function `func` (`callee`, with its `pre`): the arguments go from the
    /// caller's registers (`f`) straight into the parameter slots of the new frame, no
    /// argument cells in between.
    #[inline(always)]
    #[allow(clippy::too_many_arguments)]
    fn call_pre(
        &mut self,
        f: &Func,
        callee: &Func,
        func: FuncId,
        pre: &pre::Pre,
        args: yavs_ir::Regs,
        dst: Option<Reg>,
        first: Option<u64>,
    ) -> R<()> {
        let link = if pre.nested {
            self.static_link(callee, None)?
        } else {
            None
        };
        let caller = self.regs.base;
        self.push_frame_pre(callee, func, pre, link, dst)?;
        let fr = &self.frames[self.frames.len() - 1];
        let (sbase, slen) = (fr.sbase as usize, fr.slen as usize);
        let regs = f
            .regs(args)
            .ok_or_else(|| ice_s!("operand list out of range"))?;
        let slots = self
            .sstack
            .get_mut(sbase..sbase + slen)
            .ok_or_else(|| ice_s!("frame outside the slot stack"))?;
        let mut params = pre.params.iter();
        // The registers the callee's first ops would load the parameters into (`Code::skip`).
        let mut pro = pre.pro.iter();
        let base = self.regs.base;
        // The protected object of a method call is its first parameter.
        if let Some(h) = first
            && let Some(&(slot, w)) = params.next()
        {
            let n = w.bytes() as usize;
            let dst = slots
                .get_mut(slot as usize..slot as usize + n)
                .ok_or_else(|| ice_f!("parameter slot {slot} outside the frame"))?;
            put_le(dst, h);
            if let Some(&r) = pro.next()
                && r != pre::NONE
            {
                self.regs.v[base + r as usize] = h;
            }
        }
        for &r in regs {
            let Some(&(slot, w)) = params.next() else {
                break;
            };
            let v = self.regs.v[caller + r.0 as usize];
            let n = w.bytes() as usize;
            let dst = slots
                .get_mut(slot as usize..slot as usize + n)
                .ok_or_else(|| ice_f!("parameter slot {slot} outside the frame"))?;
            if w == Width::Sig {
                dst[..8].copy_from_slice(&v.to_le_bytes());
                dst[8..].copy_from_slice(&self.regs.v[caller + r.0 as usize + 1].to_le_bytes());
                pro.next();
            } else {
                put_le(dst, v);
                if let Some(&r) = pro.next()
                    && r != pre::NONE
                {
                    self.regs.v[base + r as usize] = v;
                }
            }
        }
        if params.next().is_some() {
            let name = &callee.params[regs.len() + usize::from(first.is_some())].name;
            return Err(ice_f!("missing argument {name}"));
        }
        Ok(())
    }

    /// `call_pre` for a callee that takes its parameters in registers (`Pre::lean`): the
    /// argument registers go to the registers its prologue would load them into, and the
    /// parameters' slots stay as they are.
    #[inline(always)]
    fn call_lean(
        &mut self,
        callee: &Func,
        func: FuncId,
        pre: &pre::Pre,
        args: &[Reg],
        dst: Option<Reg>,
        first: Option<u64>,
    ) -> R<()> {
        let link = if pre.nested {
            self.static_link(callee, None)?
        } else {
            None
        };
        let caller = self.regs.base;
        self.push_frame_lean(callee, func, pre, link, dst)?;
        let base = self.regs.base;
        let mut pro = pre.pro.iter();
        if let Some(h) = first
            && let Some(&r) = pro.next()
        {
            self.regs.v[base + r as usize] = h;
        }
        for (&r, &p) in args.iter().zip(pro.by_ref()) {
            let v = self.regs.v[caller + r.0 as usize];
            self.regs.v[base + p as usize] = v;
        }
        if pro.next().is_some() {
            return Err(ice_f!("missing argument of `{}`", callee.name));
        }
        Ok(())
    }

    /// Call `func` with the argument cells of `args` after `first` (a protected object).
    /// The cells go through `Machine::cells`, so a call allocates no argument vector.
    fn call_with(
        &mut self,
        prog: &Program,
        f: &Func,
        func: FuncId,
        first: Option<u64>,
        args: yavs_ir::Regs,
        dst: Option<Reg>,
    ) -> R<Flow> {
        let mut cells = std::mem::take(&mut self.cells);
        cells.clear();
        cells.extend(first);
        let r = self
            .push_args(&mut cells, f, args, prog.func(func), first.map_or(0, |_| 1))
            .and_then(|()| self.call(prog, func, &cells, dst));
        self.cells = cells;
        r
    }

    /// Push the frame of the call `func(args)` and run it until it returns (`Outcome::Returned`
    /// with its value, the frame popped but `dst` not written) or the process stops inside it
    /// (a report at the stop severity: the frames stay).
    pub(crate) fn call_to_end(
        &mut self,
        prog: &Program,
        f: &Func,
        func: FuncId,
        args: yavs_ir::Regs,
        dst: Option<Reg>,
        ctx: &mut Option<&mut Ctx<'_>>,
    ) -> R<Outcome> {
        let base = self.frames.len();
        self.call_with(prog, f, func, None, args, dst)?;
        self.exec::<false>(ctx, base)
    }

    fn push_args(
        &self,
        out: &mut Vec<u64>,
        f: &Func,
        args: yavs_ir::Regs,
        callee: Option<&Func>,
        skip: usize,
    ) -> R<()> {
        let regs = f
            .regs(args)
            .ok_or_else(|| ice_s!("operand list out of range"))?;
        for (i, &r) in regs.iter().enumerate() {
            out.push(self.r(r));
            // Natives have no parameter list: a signal argument is a `Sig` register.
            let sig = match callee {
                Some(c) => c
                    .params
                    .get(i + skip)
                    .is_some_and(|p| p.width == Width::Sig),
                None => f.reg_ty.get(r.0 as usize) == Some(&yavs_ir::RegTy::Sig),
            };
            if sig {
                out.push(self.r(Reg(r.0 + 1)));
            }
        }
        Ok(())
    }

    #[inline(always)]
    fn do_load(&mut self, dst: Reg, addr: Reg, w: Width) -> R<()> {
        let a = self.r(addr);
        if w == Width::Sig {
            let lo = self.load(a, Width::B8)?;
            let hi = self.load(add_addr(a, 8)?, Width::B8)?;
            self.set(dst, lo);
            self.set(Reg(dst.0 + 1), hi);
        } else {
            let v = self.load(a, w)?;
            self.set(dst, v);
        }
        Ok(())
    }

    #[inline(always)]
    fn do_store(&mut self, addr: Reg, src: Reg, w: Width) -> R<()> {
        let a = self.r(addr);
        if w == Width::Sig {
            let (lo, hi) = (self.r(src), self.r(Reg(src.0 + 1)));
            self.store(a, Width::B8, lo)?;
            self.store(add_addr(a, 8)?, Width::B8, hi)?;
        } else {
            let v = self.r(src);
            self.store(a, w, v)?;
        }
        Ok(())
    }

    #[inline(always)]
    fn do_index(&mut self, dst: Reg, base: Reg, pos: Reg, stride: u32) -> R<()> {
        let a = index_addr(self.r(base), self.r(pos), stride)?;
        self.set(dst, a);
        Ok(())
    }

    #[inline(always)]
    fn do_pos(
        &mut self,
        dst: Reg,
        dope: Reg,
        dim: u8,
        idx: Reg,
        what: yavs_ir::CheckMsgId,
    ) -> R<()> {
        let pos = self.pos_of(self.r(dope), dim, self.ri(idx), what)?;
        self.set(dst, pos);
        Ok(())
    }

    /// The checked position of index `i` in dimension `dim` of the descriptor at `dope`.
    #[inline(always)]
    fn pos_of(&self, dope: u64, dim: u8, i: i64, what: yavs_ir::CheckMsgId) -> R<u64> {
        self.pos_of_with(dope, dim, i, || what)
    }

    /// `pos_of` with the failure message looked up only when it fails.
    #[inline(always)]
    fn pos_of_with(
        &self,
        dope: u64,
        dim: u8,
        i: i64,
        what: impl FnOnce() -> yavs_ir::CheckMsgId,
    ) -> R<u64> {
        let (l, rr, dir) = self.dim_of(dope, u64::from(dim))?;
        let pos = match dir {
            // The distance fits a u64 even when it does not fit an i64.
            0 if l <= i && i <= rr => i.wrapping_sub(l),
            1 if rr <= i && i <= l => l.wrapping_sub(i),
            _ => return self.pos_fail(l, rr, dir, i, what()).map(|()| 0),
        };
        Ok(pos as u64)
    }

    /// `Inst::SlicePos`: the start position of a slice, checked unless the slice is null.
    #[inline(always)]
    fn slice_pos(&self, inst: &Inst) -> R<u64> {
        let &Inst::SlicePos {
            dope,
            left,
            right,
            dir,
            what,
            dir_what,
            ..
        } = inst
        else {
            return Err(ice_s!("slice position of another instruction"));
        };
        let (l, r, w) = (self.ri(left), self.ri(right), self.ri(dir));
        if if w == 1 { l < r } else { l > r } {
            return Ok(0);
        }
        let dp = self.r(dope);
        // Both ends inside the range and the slice in the array's direction: one look at the
        // descriptor. Anything else is checked, and reported, by the general sequence.
        if let Ok((dl, dr, dd)) = self.dim_of(dp, 0)
            && dd == w as u64
        {
            let pos = |i: i64| match dd {
                0 if dl <= i && i <= dr => Some(i.wrapping_sub(dl)),
                1 if dr <= i && i <= dl => Some(dl.wrapping_sub(i)),
                _ => None,
            };
            if let (Some(p), Some(_)) = (pos(l), pos(r)) {
                return Ok(p as u64);
            }
        }
        self.slice_pos_checked(dp, l, r, w, what, dir_what)
    }

    #[inline(never)]
    fn slice_pos_checked(
        &self,
        dp: u64,
        l: i64,
        r: i64,
        w: i64,
        what: yavs_ir::CheckMsgId,
        dir_what: yavs_ir::CheckMsgId,
    ) -> R<u64> {
        let p = self.pos_of(dp, 0, l, what)?;
        self.pos_of(dp, 0, r, what)?;
        let (_, _, d) = self.dim_of(dp, 0)?;
        self.check_val(CheckKind::Range, w, d as i64, d as i64, dir_what)?;
        Ok(p)
    }

    #[cold]
    #[inline(never)]
    fn pos_fail(&self, l: i64, rr: i64, dir: u64, i: i64, what: yavs_ir::CheckMsgId) -> R<()> {
        match dir {
            0 | 1 => {
                let b = if dir == 0 {
                    format!("{l} to {rr}")
                } else {
                    format!("{l} downto {rr}")
                };
                rt_err(outside("index", &i.to_string(), &b, &self.msg(what)))
            }
            _ => Err(ice_f!("direction {dir} in a descriptor")),
        }
    }

    #[inline(always)]
    fn do_bin(&mut self, op: IntOp, dst: Reg, a: Reg, b: Reg) -> R<()> {
        let v = bin(op, self.ri(a), self.ri(b))?;
        self.set(dst, v);
        Ok(())
    }

    #[inline(always)]
    #[allow(clippy::too_many_arguments)]
    fn do_bin_range(
        &mut self,
        op: IntOp,
        dst: Reg,
        a: Reg,
        b: Reg,
        lo: Reg,
        hi: Reg,
        what: yavs_ir::CheckMsgId,
    ) -> R<()> {
        let (x, y) = (self.ri(a), self.ri(b));
        let (l, h) = (self.ri(lo), self.ri(hi));
        let v = match fast_op(op, x, y) {
            Some(v) if l <= v && v <= h => v as u64,
            _ => self.bin_range_slow(op, x, y, l, h, || what)?,
        };
        self.set(dst, v);
        Ok(())
    }

    /// `BinRange` off the fast path; `what` gives its message (the fast loop reads it from
    /// the IR only here).
    #[cold]
    #[inline(never)]
    fn bin_range_slow(
        &self,
        op: IntOp,
        x: i64,
        y: i64,
        l: i64,
        h: i64,
        what: impl FnOnce() -> yavs_ir::CheckMsgId,
    ) -> R<u64> {
        match int_op(op, x, y) {
            Ok(v) if l <= v && v <= h => Ok(v as u64),
            r => self.bin_range_fail(op, x, y, r, what()).map(|()| 0),
        }
    }

    fn bin_range_fail(
        &self,
        op: IntOp,
        x: i64,
        y: i64,
        r: Result<i64, arith::ArithError>,
        what: yavs_ir::CheckMsgId,
    ) -> R<()> {
        match r {
            Err(arith::ArithError::DivisionByZero { .. }) => rt_err_s!("division by zero"),
            Err(e @ arith::ArithError::NegativeExponent { .. }) => rt_err(e.to_string()),
            _ => {
                let m = self.msg(what);
                let t = if m.subtype.is_empty() {
                    "the result type"
                } else {
                    &m.subtype
                };
                rt_err_f!(
                    "result of {x} {} {y} cannot be represented as {t}",
                    op_symbol(op)
                )
            }
        }
    }

    #[inline(always)]
    fn do_check(
        &mut self,
        kind: CheckKind,
        v: Reg,
        lo: Reg,
        hi: Reg,
        what: yavs_ir::CheckMsgId,
    ) -> R<()> {
        self.check_val(kind, self.ri(v), self.ri(lo), self.ri(hi), what)
    }

    #[inline(always)]
    fn check_val(
        &self,
        kind: CheckKind,
        x: i64,
        l: i64,
        h: i64,
        what: yavs_ir::CheckMsgId,
    ) -> R<()> {
        let bad = match kind {
            CheckKind::Range | CheckKind::Index => x < l || x > h,
            CheckKind::Length => x != l,
            CheckKind::Null => x == 0,
        };
        if bad {
            return self.check_fail(kind, x, l, h, what);
        }
        Ok(())
    }

    #[cold]
    #[inline(never)]
    fn check_fail(
        &self,
        kind: CheckKind,
        x: i64,
        l: i64,
        h: i64,
        what: yavs_ir::CheckMsgId,
    ) -> R<()> {
        let (w, dir) = self.check_msg(what);
        match kind {
            CheckKind::Range | CheckKind::Index => {
                let k = if kind == CheckKind::Index {
                    "index"
                } else {
                    "value"
                };
                let m = self.msg(what);
                rt_err(outside(k, &x.to_string(), &bounds(l, h, dir), &m))
            }
            CheckKind::Length => {
                // `subtype` is the lead word of the message when set ("expression").
                let lead = self
                    .rt
                    .prog
                    .checks
                    .get(what)
                    .map_or("", |c| c.subtype.as_str());
                let lead = if lead.is_empty() { "value" } else { lead };
                rt_err_f!("{lead} length {x} does not match {w} length {l}")
            }
            CheckKind::Null => rt_err_f!("null access value: {w}"),
        }
    }

    #[inline(always)]
    fn step(
        &mut self,
        prog: &Program,
        f: &Func,
        inst: Inst,
        ctx: &mut Option<&mut Ctx<'_>>,
    ) -> R<Flow> {
        macro_rules! kc {
            () => {
                match ctx.as_deref_mut() {
                    Some(c) => c,
                    None => return rt_err_s!("signal or wait operation during elaboration"),
                }
            };
        }
        match inst {
            Inst::Const { dst, v } => self.set(dst, v as u64),
            Inst::ConstF { dst, v } => self.set(dst, v.to_bits()),
            Inst::Move { dst, src } => {
                let v = self.r(src);
                self.set(dst, v);
            }
            Inst::Select { dst, c, a, b } => {
                let v = if self.r(c) != 0 { self.r(a) } else { self.r(b) };
                self.set(dst, v);
            }
            Inst::AddrOf { dst, space, off } => {
                let off = u64::from(off);
                let a = match space {
                    AddrSpace::Frame => mk_addr(FRAME, self.frames.len() as u32 - 1, off)?,
                    AddrSpace::Up(d) => {
                        let mut i = self.frames.len() - 1;
                        for _ in 0..d {
                            i = self.frames[i]
                                .link
                                .ok_or_else(|| ice_s!("static link missing"))?
                                as usize;
                        }
                        mk_addr(FRAME, i as u32, off)?
                    }
                    AddrSpace::Inst => mk_addr(INST, 0, off)?,
                    AddrSpace::Pkg(p) => mk_addr(PKG, p, off)?,
                    AddrSpace::Const(c) => mk_addr(CONST, c.0, off)?,
                    AddrSpace::Shared => mk_addr(SHARED, 0, off)?,
                };
                self.set(dst, a);
            }
            Inst::CopyConst { dst, c } => self.copy_const(prog, self.r(dst), c)?,
            Inst::Alloc { dst, size } => {
                let a = self.temp_alloc(self.r(size))?;
                self.set(dst, a);
            }
            Inst::TempRoot { addr, desc, ty } => self.temp_root_inst(prog, addr, desc, ty)?,
            Inst::PkgAlloc { dst, size } => {
                let n = self.r(size);
                let Some((p, data)) = self.pkg_init.as_mut() else {
                    return Err(ice_s!(
                        "package allocation outside a package initialisation"
                    ));
                };
                let off = (data.len() as u64).next_multiple_of(8);
                let end = off.saturating_add(n);
                if end > u64::from(u32::MAX) {
                    return rt_err_s!("package data exceeds 4 GiB");
                }
                data.resize(end as usize, 0);
                let a = mk_addr(PKG, *p, off)?;
                self.set(dst, a);
            }
            Inst::Load { dst, addr, w } => self.do_load(dst, addr, w)?,
            Inst::Store { addr, src, w } => self.do_store(addr, src, w)?,
            Inst::Copy { dst, src, bytes } => {
                self.copy_mem(self.r(dst), self.r(src), self.r(bytes))?;
            }
            Inst::Fill { dst, elem, n, size } => self.fill(dst, elem, n, size)?,
            Inst::Zero { addr, bytes } => self.wr(self.r(addr), u64::from(bytes))?.fill(0),
            Inst::Pos {
                dst,
                dope,
                dim,
                idx,
                what,
            } => self.do_pos(dst, dope, dim, idx, what)?,
            Inst::SlicePos { dst, .. } => {
                let p = self.slice_pos(&inst)?;
                self.set(dst, p);
            }
            Inst::Index {
                dst,
                base,
                pos,
                stride,
            } => self.do_index(dst, base, pos, stride)?,
            Inst::Field {
                dst,
                base,
                layout,
                n,
            } => {
                let a = self.field_addr(self.r(base), self.r(layout), n)?;
                self.set(dst, a);
            }
            Inst::Bin { op, dst, a, b } => self.do_bin(op, dst, a, b)?,
            Inst::Un { op, dst, a } => {
                let x = self.ri(a);
                let v = arith_r(match op {
                    IntUnOp::Neg => arith::neg(x),
                    IntUnOp::Abs => arith::abs(x),
                })?;
                self.set(dst, v);
            }
            Inst::FBin { op, dst, a, b } => {
                let x = self.rf(a);
                let v = match op {
                    FOp::Add => x + self.rf(b),
                    FOp::Sub => x - self.rf(b),
                    FOp::Mul => x * self.rf(b),
                    FOp::Div => {
                        let y = self.rf(b);
                        if y == 0.0 {
                            return rt_err_f!("division of {x} by zero");
                        }
                        x / y
                    }
                    FOp::PowI => match i32::try_from(self.ri(b)) {
                        Ok(e) => x.powi(e),
                        Err(_) => x.powf(self.ri(b) as f64),
                    },
                    FOp::Pow => x.powf(self.rf(b)),
                };
                let v = real_ok(v, "arithmetic")?;
                self.set(dst, v);
            }
            Inst::FUn { op, dst, a } => {
                let x = self.rf(a);
                let v = match op {
                    FUnOp::Neg => -x,
                    FUnOp::Abs => x.abs(),
                };
                self.set(dst, v.to_bits());
            }
            Inst::Phys { op, dst, a, b } => {
                let (x, y) = (self.ri(a), self.rf(b));
                let v = arith_r(match op {
                    PhysOp::MulReal => arith::phys_mul_real(x, y),
                    PhysOp::DivReal => arith::phys_div_real(x, y),
                })?;
                self.set(dst, v);
            }
            Inst::IToF { dst, src } => {
                let v = (self.ri(src) as f64).to_bits();
                self.set(dst, v);
            }
            Inst::FToI { dst, src } => {
                let v = arith_r(arith::real_to_int(arith::Op::Mul, self.rf(src)))?;
                self.set(dst, v);
            }
            Inst::FToIRange {
                dst,
                src,
                lo,
                hi,
                what,
            } => {
                let (x, l, h) = (self.rf(src), self.ri(lo), self.ri(hi));
                match arith::real_to_int(arith::Op::Mul, x) {
                    Ok(v) if l <= v && v <= h => self.set(dst, v as u64),
                    _ => {
                        let m = self.msg(what);
                        return rt_err(outside("value", &real_text(x), &bounds(l, h, m.dir), &m));
                    }
                }
            }
            Inst::FCheck { v, lo, hi, what } => {
                let (x, l, h) = (self.rf(v), self.rf(lo), self.rf(hi));
                if !(l <= x && x <= h) {
                    let m = self.msg(what);
                    let b = match m.dir {
                        Dir::To => format!("{} to {}", real_text(l), real_text(h)),
                        Dir::Downto => format!("{} downto {}", real_text(h), real_text(l)),
                    };
                    return rt_err(outside("value", &real_text(x), &b, &m));
                }
            }
            Inst::BinRange {
                op,
                dst,
                a,
                b,
                lo,
                hi,
                what,
            } => self.do_bin_range(op, dst, a, b, lo, hi, what)?,
            Inst::Cmp { op, dst, a, b } => {
                let v = cmp(op, self.ri(a).cmp(&self.ri(b)));
                self.set(dst, v.into());
            }
            Inst::FCmp { op, dst, a, b } => {
                let v = self
                    .rf(a)
                    .partial_cmp(&self.rf(b))
                    .is_some_and(|o| cmp(op, o));
                self.set(dst, v.into());
            }
            Inst::Bool { op, dst, a, b } => {
                let (x, y) = (self.r(a) != 0, self.r(b) != 0);
                let v = match op {
                    BoolOp::And => x && y,
                    BoolOp::Or => x || y,
                    BoolOp::Xor => x != y,
                    BoolOp::Not => !x,
                };
                self.set(dst, v.into());
            }
            Inst::LogicOp { op, dst, a, b, n } => {
                let n = self.r(n);
                let x = self.planes(self.r(a), n)?;
                let unary = matches!(op, LOp::Not | LOp::ToX01 | LOp::ToX01Z | LOp::ToUx01);
                let y = if unary {
                    x.clone()
                } else {
                    self.planes(self.r(b), n)?
                };
                let v = match op {
                    LOp::And => x.and(&y),
                    LOp::Or => x.or(&y),
                    LOp::Nand => x.nand(&y),
                    LOp::Nor => x.nor(&y),
                    LOp::Xor => x.xor(&y),
                    LOp::Xnor => x.xnor(&y),
                    LOp::Not => x.not(),
                    LOp::ToX01 => x.to_x01(),
                    LOp::ToX01Z => x.to_x01z(),
                    LOp::ToUx01 => x.to_ux01(),
                    LOp::Resolve => LogicVec::resolve(&[&x, &y]),
                };
                self.put_planes(self.r(dst), &v)?;
            }
            Inst::LNum {
                op,
                dst,
                a,
                b,
                n,
                signed,
            } => {
                let n = self.r(n);
                let x = self.planes(self.r(a), n)?;
                let y = self.planes(self.r(b), n)?;
                match op {
                    LNumOp::Add | LNumOp::Sub => {
                        let v = x.add_num(&y, signed, op == LNumOp::Sub);
                        self.put_planes(self.r(dst), &v)?;
                    }
                    LNumOp::Cmp(c) => {
                        let v = x.cmp_num(&y, signed).is_some_and(|o| cmp(c, o));
                        self.set(dst, v.into());
                    }
                }
            }
            Inst::LEq { dst, a, b, n } => {
                let n = self.r(n);
                let v = self.planes(self.r(a), n)? == self.planes(self.r(b), n)?;
                self.set(dst, v.into());
            }
            Inst::MemEq { dst, a, b, n } => {
                let n = self.r(n);
                let v = self.rd(self.r(a), n)? == self.rd(self.r(b), n)?;
                self.set(dst, v.into());
            }
            Inst::LGet { dst, base, idx } => {
                let code = self.lget(self.r(base), self.r(idx))?;
                self.set(dst, code);
            }
            Inst::LSet { base, idx, v } => self.lset(self.r(base), self.r(idx), self.r(v))?,
            Inst::LFill { base, idx, n, v } => {
                self.lfill(self.r(base), self.r(idx), self.r(n), self.r(v))?;
            }
            Inst::Cat {
                dst,
                n,
                m,
                parts,
                scalars,
                bias,
                logic,
                w,
            } => {
                let ps = f.regs(parts).ok_or_else(|| ice_s!("operand list"))?;
                self.cat(
                    self.r(dst),
                    0,
                    self.r(n),
                    self.r(m),
                    ps,
                    scalars,
                    bias,
                    logic,
                    w,
                )?;
            }
            Inst::LCopy {
                dst,
                dst_off,
                src,
                src_off,
                n,
            } => {
                let (d, doff, s, soff, n) = (
                    self.r(dst),
                    self.r(dst_off),
                    self.r(src),
                    self.r(src_off),
                    self.r(n),
                );
                self.lcopy(d, doff, s, soff, n)?;
            }
            Inst::Check {
                kind,
                v,
                lo,
                hi,
                what,
            } => self.do_check(kind, v, lo, hi, what)?,
            Inst::MakeDope { at, parts } => {
                let regs = f.regs(parts).ok_or_else(|| ice_s!("operand list"))?;
                self.make_dope(self.r(at), regs)?;
            }
            Inst::DopeLen { dst, dope, dim } => {
                let n = self.dope_len(self.r(dope), dim)?;
                self.set(dst, n);
            }
            Inst::LayoutOf { dst, ty, desc, .. } => {
                let a = self.layout_of(prog, ty, self.r(desc))?;
                self.set(dst, a);
            }
            Inst::SubtypeCheck {
                dope,
                want,
                dims,
                what,
            } => self.subtype_check(self.r(dope), self.r(want), dims, what)?,
            Inst::Br { to } => return Ok(Flow::Jump(to)),
            Inst::CondBr { c, t, f } => {
                return Ok(Flow::Jump(if self.r(c) != 0 { t } else { f }));
            }
            Inst::Case { v, table } => {
                let t = prog
                    .cases
                    .get(table)
                    .ok_or_else(|| ice_f!("no case table {table}"))?;
                let x = self.ri(v);
                let i = t.arms.partition_point(|a| a.1 < x);
                let to = match t.arms.get(i) {
                    Some(&(lo, _, b)) if lo <= x => b,
                    _ => t.default,
                };
                return Ok(Flow::Jump(to));
            }
            Inst::Ret { v } => return Ok(Flow::Ret(v.map(|r| self.r(r)))),
            Inst::Unreachable { what } => {
                let (w, _) = self.check_msg(what);
                return rt_err(w);
            }
            Inst::Call { func, args, dst } => {
                return self.call_with(prog, f, func, None, args, dst);
            }
            Inst::CallIndirect { func, args, dst } => {
                let id = FuncId(self.r(func) as u32);
                let c = prog
                    .func(id)
                    .ok_or_else(|| ice_f!("indirect call of missing function {id}"))?;
                if c.kind == FuncKind::Process || c.kind == FuncKind::Elab {
                    return Err(ice_f!("indirect call of {:?} {id}", c.kind));
                }
                return self.call_with(prog, f, id, None, args, dst);
            }
            Inst::CallProt {
                func,
                obj,
                args,
                dst,
            } => {
                return self.call_prot(prog, f, func, obj, args, dst);
            }
            Inst::CallNative { native, args, dst } => {
                return self.call_native(prog, f, native, args, dst, ctx);
            }
            Inst::CallForeign { foreign, args, dst } => {
                let v = self.foreign(prog, f, foreign, args, ctx)?;
                if let Some(d) = dst {
                    self.set(d, v);
                }
            }
            Inst::New {
                dst,
                ty,
                init,
                size,
                shared,
            } => {
                self.reclaim(ctx.is_some());
                let data = self.rd(self.r(init), self.r(size))?.to_vec();
                let r = self.alloc_heap(shared)?.alloc(heap::Obj::Typed(ty), data);
                let h =
                    r.map_err(|_| runtime_s!("heap object limit (16M objects per heap) exceeded"))?;
                self.set(dst, h);
            }
            Inst::NewFrame {
                dst,
                desc,
                dsize,
                data,
                size,
                fill,
            } => self.new_frame(dst, desc, dsize, data, size, fill)?,
            Inst::NewFill {
                dst,
                ty,
                desc,
                dsize,
                data,
                size,
                fill,
                shared,
            } => {
                self.reclaim(ctx.is_some());
                let size = self.r(size);
                if size > 1 << 32 {
                    return Err(runtime_s!("object larger than 4 GiB"));
                }
                if fill != 0 && !size.is_multiple_of(u64::from(fill)) {
                    return Err(ice_s!("NewFill size is not a multiple of its element"));
                }
                let (ds, size) = (dsize as usize, size as usize);
                let dbytes = self.rd(self.r(desc), u64::from(dsize))?.to_vec();
                let src = self.rd(
                    self.r(data),
                    if fill == 0 {
                        size as u64
                    } else {
                        u64::from(fill)
                    },
                )?;
                // An all-zero fill is a zeroed allocation: the OS maps its pages on first
                // touch, so a large array that is only sparsely used stays small (#486).
                let v = if fill != 0 && src.iter().all(|&b| b == 0) {
                    let mut v = vec![0u8; ds + size];
                    v[..ds].copy_from_slice(&dbytes);
                    v
                } else {
                    let mut v = Vec::with_capacity(ds + size);
                    v.extend_from_slice(&dbytes);
                    if fill == 0 {
                        v.extend_from_slice(src);
                    } else if fill == 1 {
                        v.resize(ds + size, src[0]);
                    } else {
                        for _ in 0..size / fill as usize {
                            v.extend_from_slice(src);
                        }
                    }
                    v
                };
                let r = self.alloc_heap(shared)?.alloc(heap::Obj::Typed(ty), v);
                let h =
                    r.map_err(|_| runtime_s!("heap object limit (16M objects per heap) exceeded"))?;
                self.set(dst, h);
            }
            Inst::Free { ptr } => {
                self.reclaim(ctx.is_some());
                self.deallocate(self.r(ptr))?;
            }
            Inst::Deref { dst, handle, what } => {
                let a = self.deref(self.r(handle), what)?;
                self.set(dst, a);
            }
            Inst::SigVal {
                dst,
                sig,
                offset,
                len,
            } => {
                let (a, b) = self.sig_val(sig, self.ri(offset), self.ri(len))?;
                self.set(dst, a);
                self.set(Reg(dst.0 + 1), b);
            }
            Inst::SigParts { dst, parts, n } => {
                // Flatten nested references, order by first element, check the tiling.
                let (base, n) = (self.r(parts), self.r(n));
                let mut flat = Vec::new();
                for k in 0..n {
                    let e = add_addr(base, k * u64::from(yavs_ir::SIG_PART_BYTES))?;
                    let (lo, hi) = (
                        self.load(e, Width::B8)?,
                        self.load(add_addr(e, 8)?, Width::B8)?,
                    );
                    let slot = self.load(add_addr(e, 16)?, Width::B8)?;
                    let mut first = self.load(add_addr(e, 24)?, Width::B8)?;
                    let nested = lo >> 32 == MULTI;
                    for [plo, plen, pslot] in self.raw_parts(lo, hi)? {
                        flat.push((first, [plo, plen, if nested { pslot } else { slot }]));
                        first += plen;
                    }
                }
                flat.sort_by_key(|p| p.0);
                let mut at = 0;
                for (first, [_, len, _]) in &flat {
                    if *first != at {
                        return Err(ice_s!("signal reference parts do not tile the object"));
                    }
                    at += len;
                }
                let raw: Vec<[u64; 3]> = flat.into_iter().map(|p| p.1).collect();
                self.set_parts(dst, &raw)?;
            }
            Inst::SigSlice {
                dst,
                sig,
                offset,
                len,
            } => {
                let (o, l) = (self.ri(offset), self.ri(len));
                let (Ok(o), Ok(l)) = (u64::try_from(o), u64::try_from(l)) else {
                    return rt_err_f!("signal element range {o} + {l} out of bounds");
                };
                let (lo, hi) = (self.r(sig), self.r(Reg(sig.0 + 1)));
                if lo >> 32 != MULTI {
                    if o + l > hi || (lo as u32 as u64) + o > u64::from(u32::MAX) {
                        return rt_err_f!("signal element range {o} + {l} out of bounds");
                    }
                    self.set(dst, lo + o);
                    self.set(Reg(dst.0 + 1), l);
                } else {
                    // The pieces of the parts that overlap `o .. o + l`, with their slots.
                    let mut out = Vec::new();
                    let mut at = 0;
                    for [plo, plen, slot] in self.raw_parts(lo, hi)? {
                        let a = o.max(at);
                        let z = (o + l).min(at + plen);
                        if a < z {
                            out.push([plo + (a - at), z - a, slot]);
                        }
                        at += plen;
                    }
                    if o + l > at {
                        return rt_err_f!("signal element range {o} + {l} out of bounds");
                    }
                    self.set_parts(dst, &out)?;
                }
            }
            Inst::SigRead { dst, sig } if self.is_multi(sig) => {
                let parts = self.sigparts(sig)?;
                let v = self.read_parts(kc!(), &parts, false)?;
                self.put_value(self.r(dst), &v)?;
            }
            Inst::SigRead { dst, sig } => self.sig_read_vec(dst, sig, ctx)?,
            Inst::SigReadScalar { dst, sig } => {
                let v = self.sig_read_scalar(sig, ctx)?;
                self.set(dst, v);
            }
            Inst::SigGather {
                dst,
                desc,
                tab,
                part,
            } => self.sig_gather(prog, dst, desc, tab, part, ctx)?,
            Inst::SigScatter {
                driver,
                src,
                desc,
                tab,
                ops,
                mode,
                has,
            } => self.sig_scatter(prog, f, driver, src, desc, tab, ops, mode, has, ctx)?,
            Inst::Edge { dst, kind, sig } => {
                let v = self.edge_inst(kind, sig, ctx)?;
                self.set(dst, v);
            }
            Inst::WaitEdge { sig, kind } => self.wait_edge_inst(kind.code(), sig, ctx)?,
            Inst::SigLastValue { dst, sig } => {
                // One element of a logic signal (`clk'last_value`): its code, as the block of
                // planes of a one-element value, without building a `Value`.
                let code = if self.is_multi(sig) {
                    None
                } else {
                    let (sr, ty) = self.sigref(sig)?;
                    if ty.elem == Elem::Logic {
                        kc!().last_code(sr)?
                    } else {
                        None
                    }
                };
                if let Some(code) = code {
                    let out = self.wr(self.r(dst), 32)?;
                    for (p, w) in out.as_chunks_mut::<8>().0.iter_mut().enumerate() {
                        *w = u64::from(code >> p & 1).to_le_bytes();
                    }
                } else {
                    let parts = self.sigparts(sig)?;
                    let v = self.read_parts(kc!(), &parts, true)?;
                    self.put_value(self.r(dst), &v)?;
                }
            }
            Inst::SigAttr { dst, kind, sig } => {
                // Over several parts: any event or activity, the most recent one (LRM 16.2.4).
                let c = kc!();
                let mut v = match kind {
                    AttrKind::Event | AttrKind::Active => 0,
                    AttrKind::LastEvent | AttrKind::LastActive => u64::MAX,
                };
                let fold = |v: u64, sr| -> R<u64> {
                    Ok(match kind {
                        AttrKind::Event => v | u64::from(c.event(sr)?),
                        AttrKind::Active => v | u64::from(c.active(sr)?),
                        AttrKind::LastEvent => v.min(c.last_event(sr)?.as_fs()),
                        AttrKind::LastActive => v.min(c.last_active(sr)?.as_fs()),
                    })
                };
                if self.is_multi(sig) {
                    for (sr, _, _) in self.sigparts(sig)? {
                        v = fold(v, sr)?;
                    }
                } else {
                    v = fold(v, self.sigref(sig)?.0)?;
                }
                self.set(dst, v);
            }
            Inst::Driving { dst, driver } => {
                let d = self.driver(driver)?;
                let v = kc!().driving(d.id)?;
                self.set(dst, v.into());
            }
            Inst::DrivingValue { dst, driver } => {
                let d = self.driver(driver)?;
                // The slot's elements of its kernel driver (one driver serves the leaves of a
                // pack a process drives, #686).
                let (sr, _) = self.driver_ref(&d)?;
                let v = kc!().driving_value_of(d.id, sr.range)?;
                self.put_value(self.r(dst), &v)?;
            }
            Inst::Drive {
                target: Some(target),
                wave,
                n,
                mode,
                reject,
                ..
            }
            | Inst::DriveSlot {
                target,
                wave,
                n,
                mode,
                reject,
                ..
            } if self.is_multi(target) => {
                self.drive_parts(ctx, target, wave, n, mode, reject)?;
            }
            Inst::Drive {
                driver,
                target,
                wave,
                n,
                mode,
                reject,
            } => {
                let d = self.driver(driver)?;
                self.drive(ctx, d, target, wave, n, mode, reject)?;
            }
            Inst::DriveSlot {
                slot,
                target,
                wave,
                n,
                mode,
                reject,
            } => {
                let slot =
                    u32::try_from(self.r(slot)).map_err(|_| ice_s!("driver slot out of range"))?;
                let d = self.driver(slot)?;
                self.drive(ctx, d, Some(target), wave, n, mode, reject)?;
            }
            Inst::Disconnect {
                driver,
                target,
                after,
                mode,
            } => {
                if target.is_some_and(|t| self.is_multi(t)) {
                    return Err(ice_s!("disconnect of a signal reference of several parts"));
                }
                let d = self.driver(driver)?;
                let range = match target {
                    Some(t) => self.sigref(t)?.0.range,
                    None => None,
                };
                let after = match after {
                    Some(a) => Some(to_time(self.ri(a))?),
                    None => None,
                };
                let mode = match mode {
                    DriveMode::Transport => DelayMode::Transport,
                    DriveMode::Inertial => DelayMode::INERTIAL,
                };
                kc!().disconnect(d.id, range, after, mode)?;
            }
            Inst::Force { sig, kind, value } => {
                let parts = self.sigparts(sig)?;
                let elem = same_elem(&parts)?;
                let total = parts.iter().map(|p| p.0.range.map_or(0, |r| r.len)).sum();
                let v = self.get_value(self.r(value), elem, total)?;
                let m = force_mode(kind);
                let mut at = 0;
                for (sr, _, _) in parts {
                    let len = sr.range.map_or(0, |r| r.len);
                    kc!().force(sr, m, &sub_value(&v, elem, at, len)?)?;
                    at += len;
                }
            }
            Inst::Release { sig, kind } => {
                for (sr, _, _) in self.sigparts(sig)? {
                    kc!().release(sr, force_mode(kind))?;
                }
            }
            Inst::WaitOn { sig } => self.wait_on(sig, ctx)?,
            Inst::WaitFor { t } => self.wait_for(t, ctx)?,
            Inst::WaitAt { t } => {
                let t = to_time(self.ri(t))?;
                kc!().wait_at(t)?;
                if self.dbg.is_some() {
                    self.waits.timeout = Some(t);
                }
            }
            Inst::Suspend { .. } => {
                self.suspend(prog, f, ctx)?;
                return Ok(Flow::Suspend);
            }
            Inst::TimedOut { dst } => {
                let v = kc!().timed_out();
                self.set(dst, v.into());
            }
            Inst::Now { dst } => {
                let v = kc!().now().as_fs();
                self.set(dst, v);
            }
            Inst::Delta { dst } => {
                let v = kc!().delta();
                self.set(dst, v.into());
            }
            Inst::Report { msg, len, severity } => {
                if let Some(p) = self.pm.as_mut() {
                    p.reports += 1;
                }
                let s = sev(self.r(severity))?;
                // `--ieee-warnings=off`: the warnings of the ieee bodies, also inlined ones.
                if s == yavs_kernel::Severity::Warning
                    && crate::ieee_warnings_off()
                    && self.in_ieee(prog, f)
                {
                    return Ok(Flow::Next);
                }
                let text: String = self
                    .rd(self.r(msg), self.r(len))?
                    .iter()
                    .map(|&b| b as char)
                    .collect();
                match ctx.as_deref_mut() {
                    Some(c) => {
                        // A severity disabled by SetVhdlAssertEnable is neither shown nor
                        // counted (as sim1).
                        if !self.rt.asserts.report(c, s) {
                            return Ok(Flow::Next);
                        }
                        c.report(s, text);
                        // The simulation ends after this cycle: execute no further.
                        if s >= c.stop_severity() {
                            return Ok(Flow::Suspend);
                        }
                    }
                    // No kernel (elaboration, or a function run by a thunk): the message goes
                    // to stderr at once, ahead of the first cycle, as the reference does. Severity failure stops
                    // elaboration as it stops a run.
                    None => {
                        if self.rt.replay.load(std::sync::atomic::Ordering::Relaxed) {
                            return Ok(Flow::Next);
                        }
                        if !self.rt.asserts.report_elab(s) {
                            return Ok(Flow::Next);
                        }
                        // A function the kernel calls during the run (resolution, port
                        // conversion): the kernel prints it like any report (#372).
                        let text = match yavs_kernel::callout::push(s, text) {
                            Ok(()) => return Ok(Flow::Next),
                            Err(text) => text,
                        };
                        crate::env::print_elab(s, &text);
                        if let Some(r) = self.init_reports.as_mut() {
                            r.push((s as u8, text.clone()));
                        }
                        if s >= yavs_kernel::Severity::Failure {
                            return rt_err_f!("report of severity failure: {text}");
                        }
                    }
                }
            }
            Inst::Output { addr, len } => {
                let b = self.rd(self.r(addr), self.r(len))?.to_vec();
                kc!().output(&b);
            }
            Inst::Cover { item } => self.op_cover(item.0)?,
            Inst::CoverIn {
                dst,
                acc,
                src,
                shift,
                ulogic,
            } => self.op_cover_in(dst.0, acc.0, src.0, shift, ulogic),
            Inst::CoverCond { cond, state } => self.op_cover_cond(prog, cond, state.0)?,
            Inst::Stmt => self.end_stmt(),
            Inst::Elab { op, .. } => {
                return rt_err_f!(
                    "elaboration instruction {op:?} needs yavs-elab (not implemented yet)"
                );
            }
        }
        Ok(Flow::Next)
    }
}

/// The interpreter under compiled code (`Machine::run_tier`, `Tier::run_cached`).
pub struct Host<'a, 'c, 'x> {
    m: &'a mut Machine,
    rt: &'a Runtime,
    prog: &'a Program,
    /// The top frame's function; `up` those of the frames below it that compiled code
    /// pushed through `step`.
    f: &'a Func,
    up: Vec<&'a Func>,
    ctx: &'a mut Option<&'c mut Ctx<'x>>,
    fault: Option<Fault>,
    /// A native a native frame ran ended the process (`NativeCall::Stop`): the materialisation
    /// that follows suspends it.
    stop: bool,
}

impl Host<'_, '_, '_> {
    /// `TierHost::materialise`: push `frames` on the top frame, which waits at its call at `pc`.
    fn push_native(&mut self, pc: u32, frames: &[crate::NativeFrame]) -> R<()> {
        let (m, prog) = (&mut *self.m, self.prog);
        m.top().pc = pc;
        for nf in frames {
            let top = m.frames.len() - 1;
            let (cf, at) = (m.frames[top].func, m.frames[top].pc);
            let (dst, prot) = match prog.func(cf).and_then(|c| c.code.get(at as usize)) {
                Some(&Inst::Call { dst, .. }) => (dst, None),
                Some(&Inst::CallProt { dst, obj, .. }) => (dst, Some(obj)),
                _ => {
                    return Err(ice_f!("native frame of {} not called from a call", nf.func));
                }
            };
            let g = prog
                .func(nf.func)
                .ok_or_else(|| ice_f!("no function {}", nf.func))?;
            let pre = self
                .rt
                .pre(nf.func, g)
                .ok_or_else(|| ice_f!("no function {}", nf.func))?;
            let parent_rbase = m.frames[top].rbase as usize;
            // The enclosing frame of a nested subprogram: `link` holds its address's high half.
            let link = (nf.link != u64::MAX).then_some((nf.link & 0x1fff_ffff) as u32);
            m.push_frame_pre(g, nf.func, pre, link, dst)?;
            let idx = m.frames.len() - 1;
            // As `call_prot` marks the frame of a method of a shared protected object.
            if let Some(obj) = prot {
                let h = m
                    .regs
                    .v
                    .get(parent_rbase + obj.0 as usize)
                    .copied()
                    .unwrap_or(0);
                m.frames[idx].shared_alloc |= heap::tag_of(h) == heap::SHARED;
            }
            let rbase = m.frames[idx].rbase as usize;
            let regs = &mut m.regs.v[rbase..];
            if regs.len() != nf.regs.len() {
                return Err(ice_f!(
                    "native frame of {} with {} registers",
                    g.name,
                    nf.regs.len()
                ));
            }
            // A register defined once by a constant or an address is preloaded (`Pre`): the
            // interpreter never executes its definition, compiled code may not have yet.
            for (k, (d, &v)) in regs.iter_mut().zip(&nf.regs).enumerate() {
                if !pre.preloaded.get(k).copied().unwrap_or(false) {
                    *d = v;
                }
            }
            match m.fslots_mut(idx) {
                Some(s) if s.len() == nf.slots.len() => s.copy_from_slice(&nf.slots),
                _ => {
                    return Err(ice_f!(
                        "native frame of {} with {} slot bytes",
                        g.name,
                        nf.slots.len()
                    ));
                }
            }
            let fr = &mut m.frames[idx];
            fr.pc = nf.pc;
            fr.temp_base =
                u32::try_from(nf.temp_base).map_err(|_| ice_s!("temporary base beyond 4 GiB"))?;
            self.up.push(std::mem::replace(&mut self.f, g));
        }
        Ok(())
    }
}

impl Host<'_, '_, '_> {
    /// The assignment of a fused scalar logic drive through the kernel's general entry
    /// (`Ctx::assign_code`): false (nothing done) where that is not the case (a code that is no
    /// code, a reference of several parts, a signal that is not a scalar logic one).
    fn drive_bit_try(
        &mut self,
        driver: u32,
        target: [u64; 2],
        delay: i64,
        code: u64,
        inertial: bool,
    ) -> R<bool> {
        let m = &mut *self.m;
        if code > 8 || target[0] >> 32 == MULTI {
            return Ok(false);
        }
        let d = m.driver(driver)?;
        let (sr, ty) = m.sigref_words(target[0], target[1])?;
        if ty.elem != Elem::Logic || sr.range.map_or(0, |r| r.len) != 1 {
            return Ok(false);
        }
        let t = to_time(delay)?;
        let mode = if inertial {
            DelayMode::INERTIAL
        } else {
            DelayMode::Transport
        };
        let Some(c) = self.ctx.as_deref_mut() else {
            return rt_err_s!("signal or wait operation during elaboration");
        };
        c.assign_code(d.id, sr.range, t, code as u8, mode)?;
        Ok(true)
    }

    /// The assignment of a fused scalar logic drive (`pre::ScalarDrive`) whose `Drive` is at
    /// `at`, as the five instructions it stands for would do it: the registers they read get
    /// the values compiled code computed, the waveform is made, the `LSet` and the `Drive` are
    /// stepped. Every case the fast path leaves: a reference of several parts, a signal that is
    /// not a scalar logic one, a code that is no code.
    #[cold]
    #[inline(never)]
    fn drive_bit_replay(
        &mut self,
        tf: &mut crate::TierFrame,
        at: u32,
        target: [u64; 2],
        delay: i64,
        code: u64,
    ) -> crate::Stepped {
        let (m, f) = (&mut *self.m, self.f);
        let at = at as usize;
        let r = (|| -> R<()> {
            let Some(&Inst::Drive {
                target: Some(t),
                wave,
                ..
            }) = f.code.get(at)
            else {
                return Err(ice_s!("a fused drive without its Drive"));
            };
            let before = |from: usize, hit: &dyn Fn(&Inst) -> bool| {
                (0..from).rev().find(|&p| hit(&f.code[p]))
            };
            let lset = before(at, &|i| matches!(i, Inst::LSet { .. }))
                .ok_or_else(|| ice_s!("a fused drive without its LSet"))?;
            let Inst::LSet { base, v, idx } = f.code[lset] else {
                return Err(ice_s!("a fused drive without its LSet"));
            };
            let u = |x: u32| u16::try_from(x).map_err(|_| ice_s!("a register beyond 65535"));
            let head = before(
                lset,
                &|i| matches!(i, Inst::Alloc { dst, .. } | Inst::Zero { addr: dst, .. } if *dst == wave),
            )
            .ok_or_else(|| ice_s!("a fused drive without its waveform"))?;
            let store = (head + 1..lset)
                .find(|&p| matches!(f.code[p], Inst::Store { addr, .. } if addr == wave))
                .map(|p| f.code[p]);
            // In the frame the time register is the `Store`'s, or (none: the time is 0) the
            // `LSet`'s index, which holds 0. Coupled to `pre::peephole`, which fuses only an
            // `LSet` at the constant index 0: if that widens, this needs a zero register of its own.
            let (size, time) = match (f.code[head], store) {
                (Inst::Alloc { size, .. }, Some(Inst::Store { src, .. })) => (u(size.0)?, src),
                (Inst::Zero { bytes, .. }, Some(Inst::Store { src, .. })) => {
                    (pre::FRAME_WAVE | u(bytes)?, src)
                }
                (Inst::Zero { bytes, .. }, None) => (pre::FRAME_WAVE | u(bytes)?, idx),
                _ => return Err(ice_s!("a fused drive without its waveform")),
            };
            m.regs[t.0 as usize] = target[0];
            m.regs[t.0 as usize + 1] = target[1];
            if store.is_some() {
                m.regs[time.0 as usize] = delay as u64;
            }
            m.regs[v.0 as usize] = code;
            m.waveform_head(size, u(time.0)?, u(wave.0)?, u(base.0)?)?;
            m.top().pc = lset as u32;
            m.replay_lset_drive(self.prog, f, lset as u32, m.frames.len() - 1, self.ctx)
        })();
        match r {
            Ok(()) => {
                m.tier_temp(tf);
                crate::Stepped::Next
            }
            Err(e) => {
                self.fault = Some(e);
                crate::Stepped::Fault
            }
        }
    }
}

impl Host<'_, '_, '_> {
    /// [`crate::TierHost::view_again`] without the virtual call, for [`crate::Tier::run_cached`]
    /// (inlined into the tier: the call and its prologue were about 25 instructions of every
    /// compiled activation, #766).
    #[inline]
    pub fn view_root_again(&mut self, tf: &mut crate::TierFrame) -> bool {
        let Some(c) = self.ctx.as_deref_mut() else {
            return false;
        };
        tf.kctx = (c as *mut Ctx<'_>).cast();
        self.m.view_root(tf, c)
    }
}

impl crate::TierHost for Host<'_, '_, '_> {
    fn view(&mut self, tf: &mut crate::TierFrame) {
        self.m.tier_view(tf);
        // What compiled code reads of the kernel itself: the codes of logic signals and the
        // edge this activation was resumed for.
        let m = &mut *self.m;
        tf.drivers = m.drivers.as_ptr();
        tf.ndrivers = m.drivers.len() as u64;
        let Some(c) = self.ctx.as_deref_mut() else {
            tf.sigtab_len = 0;
            tf.known = u64::MAX;
            tf.stop = crate::NO_STOP.as_ptr();
            tf.spin = &raw mut m.spin;
            tf.kctx = std::ptr::null_mut();
            return;
        };
        tf.kctx = (c as *mut Ctx<'_>).cast();
        let c = &*c;
        tf.stop = c.interrupt_flags();
        tf.spin = &raw mut m.spin;
        if m.vc.tier_sigs.len() != m.vc.nsigs {
            m.vc.tier_sigs = m
                .inst
                .sigs
                .iter()
                .map(|s| {
                    let view = c.code_view(s.signal).filter(|_| s.ty.elem == Elem::Logic);
                    let stamps = c.event_view(s.signal).map_or(0, |p| p as u64);
                    let last = c
                        .last_view(s.signal)
                        .filter(|_| view.is_some())
                        .map_or(0, |p| p as u64);
                    let (planes, mut plen) = c
                        .planes_view(s.signal)
                        .filter(|_| s.ty.elem == Elem::Logic)
                        .map_or((0, 0), |(p, n)| {
                            (p as u64, n.saturating_sub(s.off as usize) as u64)
                        });
                    let (bytes, es) = match (s.ty.elem, c.bytes_view(s.signal)) {
                        (Elem::Bytes(w), Some((p, es, n))) if es == w as usize => {
                            plen = n.saturating_sub(s.off as usize) as u64;
                            (p as u64, es as u64)
                        }
                        _ => (0, 0),
                    };
                    let (codes, len, cap) = match view {
                        // The instance signal's own elements start at `s.off`.
                        Some((p, n)) if (s.off as usize) < n => (
                            p as u64 + u64::from(s.off),
                            (n - s.off as usize) as u64,
                            c.code_cap(s.signal).saturating_sub(s.off as usize) as u64,
                        ),
                        _ => (0, 0, 0),
                    };
                    crate::TierSig {
                        codes,
                        len,
                        signal: u64::from(s.signal.0),
                        off: u64::from(s.off),
                        cap,
                        stamps,
                        last,
                        planes,
                        plen,
                        bytes,
                        es,
                    }
                })
                .collect();
        }
        tf.sigtab = m.vc.tier_sigs.as_ptr();
        tf.sigtab_len = m.vc.tier_sigs.len() as u64;
        tf.cycle = c.cycle();
        tf.known = c.resumed_edge_key();
    }

    fn tier_state(&self) -> crate::TierState {
        self.m.tier_state
    }

    fn set_tier_state(&mut self, st: crate::TierState) {
        self.m.tier_state = st;
    }

    fn view_again(&mut self, tf: &mut crate::TierFrame) -> bool {
        self.view_root_again(tf)
    }

    fn step(&mut self, tf: &mut crate::TierFrame, at: u32) -> crate::Stepped {
        use crate::Stepped;
        let (m, f) = (&mut *self.m, self.f);
        crate::tier_census::note("step", &f.name, at, f.code.get(at as usize));
        m.top().pc = at;
        let r = match f.code.get(at as usize) {
            // As the fast loop calls (`Op::Call`).
            Some(&Inst::Call { func, args, dst }) => {
                m.call_regs(self.prog, f, func, args, dst, self.rt)
            }
            // Without the generic dispatch of `step`.
            Some(&Inst::CallNative { native, args, dst }) => {
                m.call_native(self.prog, f, native, args, dst, self.ctx)
            }
            // Direct helpers for the heap and protected calls (`step`'s own bodies).
            Some(&Inst::Free { ptr }) => {
                m.reclaim(self.ctx.is_some());
                m.deallocate(m.r(ptr)).map(|()| Flow::Next)
            }
            Some(&Inst::Fill { dst, elem, n, size }) => {
                m.fill(dst, elem, n, size).map(|()| Flow::Next)
            }
            Some(&Inst::CallProt {
                func,
                obj,
                args,
                dst,
            }) => m.call_prot(self.prog, f, func, obj, args, dst),
            Some(&inst) => m.step(self.prog, f, inst, self.ctx),
            None => Err(ice_f!("compiled code stepped pc {at} beyond `{}`", f.name)),
        };
        // Without a new frame only the temporary area changes, unless the instruction ran
        // VHDL code (a native calling back) and moved the stacks.
        match r {
            Ok(Flow::Next) if m.regs.v.as_ptr() == tf.stack && m.slots_ptr() == tf.slots => {
                m.tier_temp(tf)
            }
            _ => m.tier_view(tf),
        }
        match r {
            Ok(Flow::Next) => Stepped::Next,
            Ok(Flow::Call) => {
                let func = m.frames[m.frames.len() - 1].func;
                match self.prog.func(func) {
                    Some(g) => {
                        self.up.push(std::mem::replace(&mut self.f, g));
                        Stepped::Call(func)
                    }
                    None => {
                        self.fault = Some(ice_f!("no function {func}"));
                        Stepped::Fault
                    }
                }
            }
            Ok(Flow::Suspend) => Stepped::Suspend,
            Ok(_) => {
                self.fault = Some(ice_f!(
                    "compiled code stepped a control instruction at pc {at} of `{}`",
                    f.name
                ));
                Stepped::Fault
            }
            Err(e) => {
                self.fault = Some(e);
                Stepped::Fault
            }
        }
    }

    fn ret(&mut self, tf: &mut crate::TierFrame, pc: u32) -> bool {
        let m = &mut *self.m;
        let top = m.frames.len() - 1;
        let v = match self.f.code.get(pc as usize) {
            Some(&Inst::Ret { v })
                if self.fault.is_none()
                    && top > 0
                    && (v.is_some() || m.frames[top].ret.is_none()) =>
            {
                v.map(|r| m.regs[r.0 as usize])
            }
            // Anything else (an error the interpreter reports) is the interpreter's.
            _ => {
                m.frames[top].pc = pc;
                return false;
            }
        };
        // As `exec_fast` returns, without moving the stacks.
        let done = m.pop_frame_in_place();
        if let (Some(d), Some(v)) = (done.and_then(|fr| fr.ret), v) {
            m.regs[d.0 as usize] = v;
        }
        m.tier_view(tf);
        if let Some(g) = self.up.pop() {
            self.f = g;
        }
        true
    }

    fn temp(&mut self) -> &mut Vec<u8> {
        &mut self.m.temp
    }

    #[allow(clippy::too_many_arguments)]
    fn drive_block(
        &mut self,
        tf: &mut crate::TierFrame,
        at: u32,
        driver: u32,
        target: [u64; 2],
        delay: i64,
        block: [u64; 4],
        inertial: bool,
    ) -> crate::Stepped {
        let m = &mut *self.m;
        crate::tier_census::note(
            "drive_block",
            &self.f.name,
            at,
            self.f.code.get(at as usize),
        );
        let r = (|| -> R<bool> {
            if target[0] >> 32 == MULTI {
                return Ok(false);
            }
            let d = m.driver(driver)?;
            let (sr, ty) = m.sigref_words(target[0], target[1])?;
            let n = sr.range.map_or(0, |r| r.len);
            if ty.elem != Elem::Logic || n == 0 || n > 64 {
                return Ok(false);
            }
            let t = to_time(delay)?;
            let mode = if inertial {
                DelayMode::INERTIAL
            } else {
                DelayMode::Transport
            };
            let Some(c) = self.ctx.as_deref_mut() else {
                return rt_err_s!("signal or wait operation during elaboration");
            };
            if n == 1 {
                // One element: the scalar path (the code is the bit of each plane).
                let code = (block[0] & 1)
                    | (block[1] & 1) << 1
                    | (block[2] & 1) << 2
                    | (block[3] & 1) << 3;
                c.assign_code(d.id, sr.range, t, code as u8, mode)?;
                return Ok(true);
            }
            let mask = yavs_logic::wide::nmask(n as usize);
            let mut bytes = [0u8; 32];
            for (o, w) in bytes.as_chunks_mut::<8>().0.iter_mut().zip(block) {
                *o = (w & mask).to_le_bytes();
            }
            c.assign_blocks(d.id, sr.range, t, &bytes, mode)?;
            Ok(true)
        })();
        match r {
            Ok(true) => crate::Stepped::Next,
            Ok(false) => self.signal(tf, at, 0),
            Err(e) => {
                self.fault = Some(e);
                crate::Stepped::Fault
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn drive_wide(
        &mut self,
        tf: &mut crate::TierFrame,
        at: u32,
        driver: u32,
        target: [u64; 2],
        delay: i64,
        planes: &[u8],
        inertial: bool,
    ) -> crate::Stepped {
        let m = &mut *self.m;
        crate::tier_census::note("drive_wide", &self.f.name, at, self.f.code.get(at as usize));
        let r = (|| -> R<bool> {
            if target[0] >> 32 == MULTI {
                return Ok(false);
            }
            let d = m.driver(driver)?;
            let (sr, ty) = m.sigref_words(target[0], target[1])?;
            let n = sr.range.map_or(0, |r| r.len);
            if ty.elem != Elem::Logic
                || n <= 64
                || planes.len() as u64 != u64::from(n).div_ceil(64) * 32
            {
                return Ok(false);
            }
            let t = to_time(delay)?;
            let mode = if inertial {
                DelayMode::INERTIAL
            } else {
                DelayMode::Transport
            };
            let Some(c) = self.ctx.as_deref_mut() else {
                return rt_err_s!("signal or wait operation during elaboration");
            };
            c.assign_blocks(d.id, sr.range, t, planes, mode)?;
            Ok(true)
        })();
        match r {
            Ok(true) => crate::Stepped::Next,
            Ok(false) => self.signal(tf, at, 0),
            Err(e) => {
                self.fault = Some(e);
                crate::Stepped::Fault
            }
        }
    }

    fn drive_now(&mut self, driver: u32, target: [u64; 2], delay: i64, block: [u64; 4]) -> bool {
        let m = &*self.m;
        if delay != 0 || target[1] == 0 || target[1] > 64 || target[0] >> 32 == MULTI {
            return false;
        }
        let (Some(d), Some(s), Some(c)) = (
            m.drivers.get(driver as usize),
            m.inst.sigs.get((target[0] >> 32) as usize),
            self.ctx.as_deref_mut(),
        ) else {
            return false;
        };
        let r = yavs_kernel::Range::new(s.off + target[0] as u32, target[1] as u32);
        // One scalar of at most 8 bytes: the first bytes of the cell.
        if let Elem::Bytes(es) = s.ty.elem {
            return target[1] == 1
                && es <= 8
                && c.assign_bytes_now(d.id, r, &block[0].to_le_bytes()[..es as usize]);
        }
        if s.ty.elem != Elem::Logic {
            return false;
        }
        if target[1] == 1 {
            let code =
                (block[0] & 1) | (block[1] & 1) << 1 | (block[2] & 1) << 2 | (block[3] & 1) << 3;
            return c.assign_code_now(d.id, r, code as u8);
        }
        c.assign_planes_now(d.id, Some(r), block)
    }

    fn drive_try(
        &mut self,
        driver: u32,
        target: [u64; 2],
        delay: i64,
        block: [u64; 4],
        inertial: bool,
    ) -> bool {
        let m = &*self.m;
        if target[1] == 0 || target[1] > 64 || target[0] >> 32 == MULTI {
            return false;
        }
        let (Some(d), Some(s), Some(c)) = (
            m.drivers.get(driver as usize),
            m.inst.sigs.get((target[0] >> 32) as usize),
            self.ctx.as_deref_mut(),
        ) else {
            return false;
        };
        let Ok(t) = to_time(delay) else {
            return false;
        };
        let mode = if inertial {
            DelayMode::INERTIAL
        } else {
            DelayMode::Transport
        };
        let r = Some(yavs_kernel::Range::new(
            s.off + target[0] as u32,
            target[1] as u32,
        ));
        if let Elem::Bytes(es) = s.ty.elem {
            return target[1] == 1
                && es <= 8
                && c.assign_bytes(d.id, r, t, &block[0].to_le_bytes()[..es as usize], mode)
                    .is_ok();
        }
        if s.ty.elem != Elem::Logic {
            return false;
        }
        if target[1] == 1 {
            let code =
                (block[0] & 1) | (block[1] & 1) << 1 | (block[2] & 1) << 2 | (block[3] & 1) << 3;
            return c.assign_code(d.id, r, t, code as u8, mode).is_ok();
        }
        let mask = yavs_logic::wide::nmask(target[1] as usize);
        let mut bytes = [0u8; 32];
        for (o, w) in bytes.as_chunks_mut::<8>().0.iter_mut().zip(block) {
            *o = (w & mask).to_le_bytes();
        }
        c.assign_blocks(d.id, r, t, &bytes, mode).is_ok()
    }

    fn drive_wide_try(
        &mut self,
        driver: u32,
        target: [u64; 2],
        delay: i64,
        planes: &[u8],
        inertial: bool,
    ) -> bool {
        let m = &*self.m;
        if target[1] <= 64
            || target[0] >> 32 == MULTI
            || planes.len() as u64 != target[1].div_ceil(64) * 32
        {
            return false;
        }
        let (Some(d), Some(s), Some(c)) = (
            m.drivers.get(driver as usize),
            m.inst.sigs.get((target[0] >> 32) as usize),
            self.ctx.as_deref_mut(),
        ) else {
            return false;
        };
        let (Ok(t), true) = (to_time(delay), s.ty.elem == Elem::Logic) else {
            return false;
        };
        let mode = if inertial {
            DelayMode::INERTIAL
        } else {
            DelayMode::Transport
        };
        let r = Some(yavs_kernel::Range::new(
            s.off + target[0] as u32,
            target[1] as u32,
        ));
        c.assign_blocks(d.id, r, t, planes, mode).is_ok()
    }

    fn drive_bit_now(&mut self, driver: u32, target: [u64; 2], code: u64) -> bool {
        if code > 8 || target[1] != 1 || target[0] >> 32 == MULTI {
            return false;
        }
        let m = &*self.m;
        let (Some(d), Some(s), Some(c)) = (
            m.drivers.get(driver as usize),
            m.inst.sigs.get((target[0] >> 32) as usize),
            self.ctx.as_deref_mut(),
        ) else {
            return false;
        };
        s.ty.elem == Elem::Logic
            && c.assign_code_now(
                d.id,
                yavs_kernel::Range::new(s.off + target[0] as u32, 1),
                code as u8,
            )
    }

    #[allow(clippy::too_many_arguments)]
    fn drive_bit(
        &mut self,
        tf: &mut crate::TierFrame,
        at: u32,
        driver: u32,
        target: [u64; 2],
        delay: i64,
        code: u64,
        inertial: bool,
    ) -> crate::Stepped {
        crate::tier_census::note("drive_bit", &self.f.name, at, self.f.code.get(at as usize));
        match self.drive_bit_try(driver, target, delay, code, inertial) {
            Ok(true) => crate::Stepped::Next,
            Ok(false) => self.drive_bit_replay(tf, at, target, delay, code),
            Err(e) => {
                self.fault = Some(e);
                crate::Stepped::Fault
            }
        }
    }

    fn drive_bits(&mut self, entries: &[[u64; 2]]) -> Option<u32> {
        let mut at = 0;
        while at < entries.len() {
            let rest = &entries[at..];
            let [t, w] = rest[0];
            let (driver, pc) = ((w & 0xffff) as u32, (w >> 32) as u32);
            let m = &*self.m;
            // The fast case takes a run of elements of one driver at once.
            if let (Some(d), Some(s), Some(c)) = (
                m.drivers.get(driver as usize),
                m.inst.sigs.get((t >> 32) as usize),
                self.ctx.as_deref_mut(),
            ) && s.ty.elem == Elem::Logic
            {
                let n = rest
                    .iter()
                    .take_while(|e| e[0] >> 32 == t >> 32 && e[1] & 0xffff == u64::from(driver))
                    .count();
                let taken = c.assign_codes_now(d.id, n, |k| {
                    (s.off + rest[k][0] as u32, (rest[k][1] >> 16 & 0xff) as u8)
                });
                if taken > 0 {
                    at += taken;
                    continue;
                }
            }
            // The kernel's general path, as `drive_bit` takes it.
            let (inertial, code) = (w >> 24 & 1 != 0, w >> 16 & 0xff);
            match self.drive_bit_try(driver, [t, 1], 0, code, inertial) {
                Ok(true) => at += 1,
                Ok(false) => {
                    self.fault = Some(ice_s!("a kept scalar drive is no scalar logic assignment"));
                    return Some(pc);
                }
                Err(e) => {
                    self.fault = Some(e);
                    return Some(pc);
                }
            }
        }
        None
    }

    fn wait_on_again(&mut self, sig: [u64; 2]) -> bool {
        let m = &mut *self.m;
        if sig[0] >> 32 == MULTI || m.dbg.is_some() {
            return false;
        }
        let (Some(s), Some(c)) = (
            m.inst.sigs.get((sig[0] >> 32) as usize),
            self.ctx.as_deref_mut(),
        ) else {
            return false;
        };
        c.wait_on_again(
            s.signal,
            yavs_kernel::Range::new(s.off + sig[0] as u32, sig[1] as u32),
        )
    }

    fn wait_same(&mut self, pc: u32) -> bool {
        let m = &mut *self.m;
        if m.dbg.is_some() || !m.root_process || m.frames.len() != 1 {
            crate::tier_census::note("wait_same_refused_frame", &self.f.name, pc, None);
            return false;
        }
        let Some(c) = self.ctx.as_deref_mut() else {
            return false;
        };
        if !c.wait_same() {
            crate::tier_census::note("wait_same_refused_kernel", &self.f.name, pc, None);
            return false;
        }
        m.frames[0].pc = pc;
        true
    }

    fn wait_again(&mut self, pc: u32, sig: [u64; 2]) -> bool {
        let m = &mut *self.m;
        if sig[0] >> 32 == MULTI || m.dbg.is_some() || !m.root_process || m.frames.len() != 1 {
            return false;
        }
        let (Some(s), Some(c)) = (
            m.inst.sigs.get((sig[0] >> 32) as usize),
            self.ctx.as_deref_mut(),
        ) else {
            return false;
        };
        let r = yavs_kernel::Range::new(s.off + sig[0] as u32, sig[1] as u32);
        if !c.wait_on_again(s.signal, r) {
            return false;
        }
        m.frames[0].pc = pc + 1;
        true
    }

    fn wait_suspend(
        &mut self,
        tf: &mut crate::TierFrame,
        at: u32,
        sig: [u64; 2],
    ) -> crate::Stepped {
        let m = &mut *self.m;
        crate::tier_census::note(
            "wait_suspend",
            &self.f.name,
            at,
            self.f.code.get(at as usize),
        );
        if sig[0] >> 32 == MULTI || m.dbg.is_some() {
            return self.signal(tf, at, 0);
        }
        let r = (|| -> R<()> {
            let (sr, _) = m.sigref_words(sig[0], sig[1])?;
            Machine::wait_elem(sr, self.ctx)?;
            m.suspend(self.prog, self.f, self.ctx)
        })();
        match r {
            Ok(()) => {
                m.top().pc = at + 1;
                crate::Stepped::Suspend
            }
            Err(e) => {
                self.fault = Some(e);
                crate::Stepped::Fault
            }
        }
    }

    fn signal(&mut self, tf: &mut crate::TierFrame, at: u32, edge: u32) -> crate::Stepped {
        let (m, f) = (&mut *self.m, self.f);
        crate::tier_census::note("signal", &f.name, at, f.code.get(at as usize));
        let r = match f.code.get(at as usize) {
            Some(&Inst::SigReadScalar { dst, sig }) => {
                m.sig_read_scalar(sig, self.ctx).map(|v| m.set(dst, v))
            }
            Some(&Inst::SigRead { dst, sig }) if !m.is_multi(sig) => {
                m.sig_read_vec(dst, sig, self.ctx)
            }
            Some(&Inst::SigGather {
                dst,
                desc,
                tab,
                part,
            }) => m.sig_gather(self.prog, dst, desc, tab, part, self.ctx),
            Some(&Inst::SigScatter {
                driver,
                src,
                desc,
                tab,
                ops,
                mode,
                has,
            }) => m.sig_scatter(
                self.prog, f, driver, src, desc, tab, ops, mode, has, self.ctx,
            ),
            Some(&Inst::Edge { dst, kind, sig }) => {
                m.edge_inst(kind, sig, self.ctx).map(|v| m.set(dst, v))
            }
            Some(&Inst::WaitEdge { sig, kind }) => m.wait_edge_inst(kind.code(), sig, self.ctx),
            Some(&Inst::Drive {
                driver,
                target,
                wave,
                n,
                mode,
                reject,
            }) if !target.is_some_and(|t| m.is_multi(t)) => m.driver(driver).and_then(|d| {
                // The `LSet` of element 0 of a one-element waveform just before: the scalar
                // logic assignment of the predecoder's `LSetDrive`, from the code itself.
                if let (Some(t), None, Some(&Inst::LSet { base, idx, v })) = (
                    target,
                    reject,
                    (at as usize).checked_sub(1).and_then(|p| f.code.get(p)),
                ) && m.r(idx) == 0
                    && m.r(n) == 1
                    && m.r(base) == m.r(wave).wrapping_add(8)
                    && m.r(v) <= 8
                    && m.drive_bit(self.ctx, d, t, wave, m.r(v) as u8, mode)?
                {
                    return Ok(());
                }
                m.drive(self.ctx, d, target, wave, n, mode, reject)
            }),
            // A `wait on` and the suspension after it (a process with a sensitivity list) in
            // one call.
            Some(&Inst::WaitOn { sig }) => match m.wait_on(sig, self.ctx) {
                Ok(()) if matches!(f.code.get(at as usize + 1), Some(Inst::Suspend { .. })) => {
                    return self.signal(tf, at + 1, 0);
                }
                r => r,
            },
            Some(&Inst::WaitFor { t }) => match m.wait_for(t, self.ctx) {
                Ok(()) if matches!(f.code.get(at as usize + 1), Some(Inst::Suspend { .. })) => {
                    return self.signal(tf, at + 1, 0);
                }
                r => r,
            },
            Some(&Inst::DriveSlot {
                slot,
                target,
                wave,
                n,
                mode,
                reject,
            }) if !m.is_multi(target) => u32::try_from(m.r(slot))
                .map_err(|_| ice_s!("driver slot out of range"))
                .and_then(|s| m.driver(s))
                .and_then(|d| m.drive(self.ctx, d, Some(target), wave, n, mode, reject)),
            Some(&Inst::Suspend { .. }) => match m.suspend(self.prog, f, self.ctx) {
                Ok(()) => {
                    m.top().pc = at;
                    return crate::Stepped::Suspend;
                }
                Err(e) => Err(e),
            },
            // Instructions that change only the temporary area: the roots a statement
            // recorded (`TempRoot`) and dropped at its end (`Stmt` with roots), and a frame's
            // own object (`NewFrame`).
            Some(&Inst::TempRoot { addr, desc, ty }) => {
                let r = m.temp_root_inst(self.prog, addr, desc, ty);
                m.tier_temp(tf);
                r
            }
            Some(&Inst::Stmt) => {
                m.end_stmt();
                m.tier_temp(tf);
                Ok(())
            }
            Some(&Inst::CoverCond { cond, state }) => m.op_cover_cond(self.prog, cond, state.0),
            Some(&Inst::NewFrame {
                dst,
                desc,
                dsize,
                data,
                size,
                fill,
            }) => {
                let r = m.new_frame(dst, desc, dsize, data, size, fill);
                m.tier_temp(tf);
                r
            }
            // A concatenation writes memory only (it frees nothing).
            Some(&Inst::Cat {
                dst,
                n,
                m: mm,
                parts,
                scalars,
                bias,
                logic,
                w,
            }) => match f.regs(parts) {
                Some(ps) => {
                    let (d, n, mm) = (m.r(dst), m.r(n), m.r(mm));
                    m.cat(d, 0, n, mm, ps, scalars, bias, logic, w)
                }
                None => Err(ice_s!("operand list")),
            },
            // Reads of heap objects and memory that change no register but the result.
            Some(&Inst::Deref { dst, handle, what }) => {
                let h = m.r(handle);
                m.deref(h, what).map(|a| m.set(dst, a))
            }
            Some(&Inst::MemEq { dst, a, b, n }) => {
                let n = m.r(n);
                match (m.rd(m.r(a), n), m.rd(m.r(b), n)) {
                    (Ok(x), Ok(y)) => {
                        let eq = x == y;
                        m.set(dst, eq.into());
                        Ok(())
                    }
                    (Err(e), _) | (_, Err(e)) => Err(e),
                }
            }
            // `call_native` of the edge native, without the collection point (it allocates
            // nothing).
            Some(&Inst::CallNative {
                args,
                dst: Some(dst),
                ..
            }) if edge != 0 => match f.regs(args) {
                Some(&[s]) if f.reg_ty.get(s.0 as usize) == Some(&yavs_ir::RegTy::Sig) => {
                    let (c0, c1) = (m.r(s), m.r(Reg(s.0 + 1)));
                    crate::ieee::edge_of(m, self.ctx, c0, c1, edge == 1)
                        .map(|v| m.set(dst, u64::from(v)))
                }
                _ => return self.step(tf, at),
            },
            _ => return self.step(tf, at),
        };
        match r {
            Ok(()) => crate::Stepped::Next,
            Err(e) => {
                m.top().pc = at;
                self.fault = Some(e);
                crate::Stepped::Fault
            }
        }
    }

    fn materialise(
        &mut self,
        tf: &mut crate::TierFrame,
        pc: u32,
        frames: &[crate::NativeFrame],
    ) -> crate::Stepped {
        let r = self.push_native(pc, frames);
        self.m.tier_view(tf);
        match r {
            Ok(()) if std::mem::take(&mut self.stop) => crate::Stepped::Suspend,
            Ok(()) => crate::Stepped::Next,
            Err(e) => {
                self.fault = Some(e);
                crate::Stepped::Fault
            }
        }
    }

    fn native_direct(
        &mut self,
        tf: &mut crate::TierFrame,
        native: u32,
        cells: &[u64],
        depth: u64,
        slots: &mut Option<Box<dyn crate::NativeSlots>>,
    ) -> crate::NativeCall {
        use crate::NativeCall::{Done, Slow, Stop};
        let (m, prog, rt) = (&mut *self.m, self.prog, self.rt);
        let Some(n) = prog.natives.get(yavs_ir::NativeId(native)) else {
            return Slow;
        };
        let Some(fun) = rt.natives.get(native as usize).copied().flatten() else {
            return Slow;
        };
        // Package initialisation notes the natives it calls; a collection needs every
        // frame's registers in the machine.
        if m.pkg_init.is_some() || self.ctx.is_some() && m.reclaim_pending() {
            return Slow;
        }
        let top = m.frames.len();
        if (depth as usize) < top {
            return Slow;
        }
        // The native frames' slots are reached by address while the native runs.
        m.nwin = slots.take();
        (m.nwin_from, m.nwin_to) = (top, depth as usize);
        let last = top - 1;
        let saved = std::mem::replace(
            &mut m.frames[last].shared_alloc,
            tf.flags & crate::TIER_SHARED_ALLOC != 0,
        );
        let r = fun(m, self.ctx, n.ty, cells);
        m.frames[last].shared_alloc = saved;
        *slots = m.nwin.take();
        let fallback = std::mem::take(&mut m.ieee_fallback);
        let stop = std::mem::take(&mut m.ieee_stop);
        // The stacks may have moved (the temporary area).
        if m.regs.v.as_ptr() == tf.stack && m.slots_ptr() == tf.slots {
            m.tier_temp(tf);
        } else {
            m.tier_view(tf);
        }
        match r {
            Ok(v) if !fallback => {
                if stop {
                    self.stop = true;
                    Stop(v)
                } else {
                    Done(v)
                }
            }
            _ => Slow,
        }
    }

    fn native_frame(
        &mut self,
        tf: &mut crate::TierFrame,
        at: u32,
        native: u32,
        cells: &[u64],
    ) -> crate::NativeCall {
        use crate::NativeCall::{Done, Slow, Stop};
        let (m, prog, rt) = (&mut *self.m, self.prog, self.rt);
        let (Some(n), Some(fun)) = (
            prog.natives.get(yavs_ir::NativeId(native)),
            rt.natives.get(native as usize).copied().flatten(),
        ) else {
            return Slow;
        };
        // As `native_direct`, for the top frame: no native frames above it, so no slot windows;
        // the frame's own allocation mode is the view's.
        // A `hot` process is not initialising a package: the lines of those fields stay unread.
        if !m.hot && (m.pkg_init.is_some() || m.init_reports.is_some())
            || self.ctx.is_some() && m.reclaim_pending()
        {
            return Slow;
        }
        crate::tier_census::note("native", &n.symbol, at, None);
        m.top().pc = at;
        let r = fun(m, self.ctx, n.ty, cells);
        let fallback = std::mem::take(&mut m.ieee_fallback);
        let stop = std::mem::take(&mut m.ieee_stop);
        // A native run this way pushes no frame (it would be `Slow`): the stacks stay.
        debug_assert!(m.regs.v.as_ptr() == tf.stack && m.slots_ptr() == tf.slots);
        m.tier_temp(tf);
        match r {
            Ok(v) if !fallback && stop => {
                self.stop = true;
                Stop(v)
            }
            Ok(v) if !fallback => Done(v),
            _ => Slow,
        }
    }

    fn layout_word(&mut self, ty: u32, desc: &[u8], k: u32) -> Option<u64> {
        self.m.layout_word_of(self.prog, TyId(ty), desc, k).ok()
    }

    fn native_light(
        &mut self,
        tf: &mut crate::TierFrame,
        native: u32,
        cells: &[u64],
        at: Option<u32>,
        depth: u64,
        slots: &mut Option<Box<dyn crate::NativeSlots>>,
    ) -> crate::NativeCall {
        use crate::NativeCall::{Done, Slow, Stop};
        let (m, prog, rt) = (&mut *self.m, self.prog, self.rt);
        let (Some(n), Some(fun)) = (
            prog.natives.get(yavs_ir::NativeId(native)),
            rt.natives.get(native as usize).copied().flatten(),
        ) else {
            return Slow;
        };
        // Reports of package initialisation and elaboration are noted (as in `native_frame`);
        // a `hot` process has neither, and its lines stay unread. No collection check: a light
        // native allocates only temporary memory, as for `Op::CallIeee`.
        let top = m.frames.len();
        if !m.hot && (m.pkg_init.is_some() || m.init_reports.is_some()) {
            return Slow;
        }
        match at {
            Some(pc) => m.top().pc = pc,
            None => {
                if (depth as usize) < top {
                    return Slow;
                }
                // The native frames' slots are reached by address while the native runs.
                m.nwin = slots.take();
                (m.nwin_from, m.nwin_to) = (top, depth as usize);
            }
        }
        let r = fun(m, self.ctx, n.ty, cells);
        if at.is_none() {
            *slots = m.nwin.take();
        }
        let fallback = std::mem::take(&mut m.ieee_fallback);
        let stop = std::mem::take(&mut m.ieee_stop);
        // Only the temporary area can have moved or grown.
        tf.temp = m.temp.as_mut_ptr();
        tf.temp_len = m.temp.len() as u64;
        tf.temp_cap = m.temp.capacity().min(u32::MAX as usize) as u64;
        match r {
            Ok(v) if !fallback && stop => {
                self.stop = true;
                Stop(v)
            }
            Ok(v) if !fallback => Done(v),
            _ => Slow,
        }
    }

    fn memo_probe(
        &mut self,
        tf: &mut crate::TierFrame,
        func: FuncId,
        cells: &[u64],
        depth: Option<u64>,
        slots: &mut Option<Box<dyn crate::NativeSlots>>,
    ) -> crate::MemoLook {
        use crate::MemoLook::No;
        let (m, prog, rt) = (&mut *self.m, self.prog, self.rt);
        let (Some(sig), Some(c)) = (prog.memo_sig(func), prog.func(func)) else {
            return No;
        };
        if m.pkg_init.is_some()
            || rt
                .pre(func, c)
                .is_none_or(|p| p.memo_off.load(Ordering::Relaxed))
        {
            return No;
        }
        let top = m.frames.len();
        if let Some(d) = depth {
            if (d as usize) < top {
                return No;
            }
            m.nwin = slots.take();
            (m.nwin_from, m.nwin_to) = (top, d as usize);
        }
        let r = m.memo_probe(
            prog,
            func,
            sig,
            cells.len(),
            |_, i| cells[i],
            self.ctx.is_some(),
        );
        if depth.is_some() {
            *slots = m.nwin.take();
        }
        // Only the temporary area can have moved or grown.
        tf.temp = m.temp.as_mut_ptr();
        tf.temp_len = m.temp.len() as u64;
        tf.temp_cap = m.temp.capacity().min(u32::MAX as usize) as u64;
        match r {
            Ok(crate::pmemo::Probe::Hit(v)) => crate::MemoLook::Hit(v),
            Ok(crate::pmemo::Probe::Miss(n)) => crate::MemoLook::Miss(n),
            Ok(crate::pmemo::Probe::No) => No,
            // Recorded: the run ends with it when compiled code returns.
            Err(e) => {
                self.fault = Some(e);
                No
            }
        }
    }

    fn memo_keep(
        &mut self,
        _tf: &mut crate::TierFrame,
        func: FuncId,
        cells: &[u64],
        depth: Option<u64>,
        slots: &mut Option<Box<dyn crate::NativeSlots>>,
        reports: u64,
        v: u64,
    ) {
        let (m, prog) = (&mut *self.m, self.prog);
        let Some(sig) = prog.memo_sig(func) else {
            return;
        };
        let top = m.frames.len();
        if let Some(d) = depth {
            if (d as usize) < top {
                return;
            }
            m.nwin = slots.take();
            (m.nwin_from, m.nwin_to) = (top, d as usize);
        }
        m.memo_keep(func, sig, cells.len(), |_, i| cells[i], reports, v);
        if depth.is_some() {
            *slots = m.nwin.take();
        }
    }

    fn native_step(
        &mut self,
        tf: &mut crate::TierFrame,
        func: u32,
        pc: u32,
        depth: u64,
        regs: (&[u32], &[u32]),
        ins: &[u64],
        outs: &mut [u64],
        slots: &mut Option<Box<dyn crate::NativeSlots>>,
        temp_base: u64,
    ) -> crate::NativeCall {
        use crate::NativeCall::{Done, Slow, Stop};
        let (m, prog) = (&mut *self.m, self.prog);
        let Some(f) = prog.func(FuncId(func)) else {
            return Slow;
        };
        let Some(&inst) = f.code.get(pc as usize) else {
            return Slow;
        };
        crate::tier_census::note("xstep", &f.name, pc, Some(&inst));
        let top = m.frames.len();
        let (uses, defs) = regs;
        let gc = matches!(
            inst,
            Inst::New { .. } | Inst::NewFill { .. } | Inst::Free { .. }
        );
        // Registers up to the highest one named, in a frame of their own at the machine's top.
        let nregs = uses
            .iter()
            .chain(defs)
            .map(|&r| r as usize + 1)
            .max()
            .unwrap_or(0);
        // The registers of the frame-mode callers are pointed to by compiled code: the register
        // stack may not move.
        if (depth as usize) < top
            || m.regs.v.capacity() - m.regs.v.len() < nregs
            || uses.len() != ins.len()
            || defs.len() != outs.len()
            // The stack a report shows is the machine's frames.
            || matches!(inst, Inst::Report { .. }) && depth as usize != top
            || gc && self.ctx.is_some() && m.reclaim_pending()
        {
            return Slow;
        }
        let rbase = m.regs.v.len();
        m.regs.v.resize(rbase + nregs, 0);
        let caller = m.regs.base;
        m.regs.base = rbase;
        m.frames.push(Frame {
            func: FuncId(func),
            pc,
            rbase: rbase as u32,
            sbase: m.sstack.len() as u32,
            slen: 0,
            link: None,
            ret: None,
            temp_base: temp_base as u32,
            shared_alloc: tf.flags & crate::TIER_SHARED_ALLOC != 0,
        });
        for (&r, &v) in uses.iter().zip(ins) {
            m.regs.v[rbase + r as usize] = v;
        }
        m.nwin = slots.take();
        (m.nwin_from, m.nwin_to) = (top, depth as usize);
        let r = m.step(prog, f, inst, self.ctx);
        *slots = m.nwin.take();
        for (o, &r) in outs.iter_mut().zip(defs) {
            *o = m.regs.v[rbase + r as usize];
        }
        m.frames.pop();
        m.regs.v.truncate(rbase);
        m.regs.base = caller;
        if m.regs.v.as_ptr() == tf.stack && m.slots_ptr() == tf.slots {
            m.tier_temp(tf);
        } else {
            m.tier_view(tf);
        }
        match r {
            Ok(Flow::Next) => Done(None),
            Ok(Flow::Suspend) => {
                self.stop = true;
                Stop(None)
            }
            _ => Slow,
        }
    }

    fn temp_root(
        &mut self,
        tf: &mut crate::TierFrame,
        addr: u64,
        ty: Option<u32>,
        desc: &[u8],
    ) -> bool {
        let (m, prog) = (&mut *self.m, self.prog);
        let ok = match ty {
            None => m.fat_root(addr).is_ok(),
            Some(t) => {
                let t = yavs_ir::TyId(t);
                let n = Ty::desc_size(&prog.types, t);
                let ok = split(addr).0 == TEMP && n.is_some_and(|n| n as usize == desc.len());
                if ok {
                    m.temp_root(addr, Some(t), desc.into());
                }
                ok
            }
        };
        m.tier_temp(tf);
        ok
    }

    fn end_stmt(&mut self, tf: &mut crate::TierFrame, base: u64) {
        let m = &mut *self.m;
        let base = base as usize;
        m.temp.truncate(base);
        if !m.troots.is_empty() {
            m.troots.retain(|t| (t.off as usize) < base);
        }
        m.tier_temp(tf);
    }

    fn deref(&mut self, h: u64) -> Option<u64> {
        if h == 0 {
            return None;
        }
        let (hp, space) = self.m.heap_of(h).ok()?;
        let i = hp.resolve(h).ok()?;
        mk_addr(space, i, 0).ok()
    }

    fn object(&mut self, addr: u64, write: bool) -> Option<(*mut u8, usize)> {
        let (sp, idx, _) = split(addr);
        // The top frame's slots and the temporary area are in the `TierFrame`.
        if sp == TEMP || sp == FRAME && idx as usize + 1 == self.m.frames.len() {
            return None;
        }
        if write {
            let o = self.m.obj_mut(sp, idx).ok()?;
            Some((o.as_mut_ptr(), o.len()))
        } else {
            let o = self.m.obj(sp, idx).ok()?;
            Some((o.as_ptr().cast_mut(), o.len()))
        }
    }
}

fn force_mode(k: ForceKind) -> ForceMode {
    match k {
        ForceKind::In => ForceMode::Effective,
        ForceKind::Out => ForceMode::Driving,
        ForceKind::Default => ForceMode::Default,
    }
}

/// A little-endian memory cell of up to 8 bytes as a register value: a 4-byte cell is
/// sign-extended (narrow integer array elements, `Width::B4`), others zero-extended.
#[inline]
pub(crate) fn cell(b: &[u8]) -> u64 {
    if let Ok(x) = <[u8; 4]>::try_from(b) {
        return i64::from(i32::from_le_bytes(x)) as u64;
    }
    let mut x = [0u8; 8];
    let n = b.len().min(8);
    x[..n].copy_from_slice(&b[..n]);
    u64::from_le_bytes(x)
}

/// [`cell`] of `w.bytes()` bytes, `w` known: `B4` sign-extends.
#[inline(always)]
pub(crate) fn cell_w(b: &[u8], w: Width) -> u64 {
    // `b` is exactly `min(w.bytes(), 8)` long: fixed sizes compile to plain loads.
    match w {
        Width::B1 => b.first().map_or(0, |&x| u64::from(x)),
        Width::B4 => <[u8; 4]>::try_from(b).map_or(0, |x| i64::from(i32::from_le_bytes(x)) as u64),
        _ => <[u8; 8]>::try_from(b).map_or(0, u64::from_le_bytes),
    }
}

fn store_cells(slots: &mut [u8], slot: u32, w: Width, cells: &[u64]) -> R<()> {
    let n = w.bytes() as usize;
    if n > cells.len() * 8 {
        return Err(ice_s!("too few argument cells"));
    }
    let dst = slots
        .get_mut(slot as usize..slot as usize + n)
        .ok_or_else(|| ice_f!("parameter slot {slot} outside the frame"))?;
    for (d, c) in dst.chunks_mut(8).zip(cells) {
        d.copy_from_slice(&c.to_le_bytes()[..d.len()]);
    }
    Ok(())
}

/// The size of a `Repeat` or `Runs` constant once expanded.
fn compact_len(c: &ConstData) -> R<u64> {
    let too_big = || runtime_s!("constant larger than 4 GiB");
    let total = match c {
        ConstData::Repeat { elem, n } => (elem.len() as u64).checked_mul(*n),
        ConstData::Runs(runs) => runs.iter().try_fold(0u64, |t, (n, e)| {
            t.checked_add((e.len() as u64).checked_mul(*n)?)
        }),
        _ => Some(0),
    }
    .ok_or_else(too_big)?;
    if total > u64::from(u32::MAX) {
        return Err(too_big());
    }
    Ok(total)
}

/// Write a `Repeat` or `Runs` constant to `out`, which is [`compact_len`] bytes.
fn fill_compact(c: &ConstData, out: &mut [u8]) {
    let mut at = 0;
    let mut run = |n: u64, e: &[u8]| {
        for _ in 0..n {
            out[at..at + e.len()].copy_from_slice(e);
            at += e.len();
        }
    };
    match c {
        ConstData::Repeat { elem, n } => run(*n, elem),
        ConstData::Runs(runs) => runs.iter().for_each(|(n, e)| run(*n, e)),
        _ => {}
    }
}

/// Expand compact constant data (rule 12) into bytes.
fn expand_const(c: &ConstData) -> R<Vec<u8>> {
    let too_big = || runtime_s!("constant larger than 4 GiB");
    Ok(match c {
        ConstData::Bytes(b) => b.clone(),
        ConstData::Repeat { .. } | ConstData::Runs(_) => {
            let mut out = vec![0; compact_len(c)? as usize];
            fill_compact(c, &mut out);
            out
        }
        ConstData::Logic(runs) => {
            let total: u64 = runs.iter().map(|r| r.0).fold(0, u64::saturating_add);
            if total > u64::from(u32::MAX) {
                return Err(too_big());
            }
            let mut blocks = vec![[0u64; 4]; total.div_ceil(64) as usize];
            let mut at = 0u64;
            for &(n, code) in runs {
                if code > 8 {
                    return Err(ice_f!("std_ulogic code {code} in a constant"));
                }
                let mut k = 0;
                while k < n {
                    let m = (n - k).min(64 - (at + k) % 64);
                    for p in 0..4 {
                        let v = if code >> p & 1 == 1 { !0 } else { 0 };
                        set_bits(&mut blocks, at + k, m, p, v);
                    }
                    k += m;
                }
                at += n;
            }
            bytes_of(&blocks)
        }
    })
}

#[cfg(test)]
thread_local! {
    /// Calls of `Machine::publish_shared` that went on to build roots (#356).
    static ROOT_BUILDS: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What an activation of a small process reads is in the first five cache lines of a
    /// machine: the hot three, the line `view_root` reads for compiled code (#634) and the tier's
    /// record with `troots` (#766). A new
    /// field goes after `heap` unless an activation reads it.
    #[test]
    fn activation_fields_are_in_the_first_lines() {
        use std::mem::offset_of;
        for (name, at, size) in [
            ("frames", offset_of!(Machine, frames), 24),
            ("regs", offset_of!(Machine, regs), 32),
            ("root", offset_of!(Machine, root), 24),
            ("temp", offset_of!(Machine, temp), 24),
            ("rt", offset_of!(Machine, rt), 8),
            ("inst", offset_of!(Machine, inst), 8),
            ("execs", offset_of!(Machine, execs), 8),
            ("hot", offset_of!(Machine, hot), 1),
            ("spec_at", offset_of!(Machine, spec_at), 4),
        ] {
            assert!(
                at + size <= 192,
                "{name} is at {at}, past the third cache line"
            );
        }
        assert_eq!(
            offset_of!(Machine, vc),
            192,
            "the view cache is the fourth line"
        );
        assert!(size_of::<ViewCache>() <= 64);
        // The tier's record and `troots` share the fifth line (#766).
        assert_eq!(offset_of!(Machine, tier_state), 256);
        assert!(offset_of!(Machine, troots) + 24 <= 320);
        assert!(offset_of!(Machine, heap) >= 256);
        // The kernel prefetches the first five lines of the process object (#766).
        assert_eq!(offset_of!(crate::InterpProcess, m), 0);
    }

    /// A suspension of a process without the shared state builds no roots (#356).
    #[test]
    fn publish_shared_needs_shared_state() {
        let rt = Runtime::new(Arc::new(Program::default()), crate::Bindings::default()).unwrap();
        let mut m = Machine::new(rt, Arc::default(), vec![], FuncId(0));
        let mut slot = None;
        m.publish_shared(&mut slot);
        assert_eq!((ROOT_BUILDS.get(), slot), (0, None));
        m.sh = Some(SharedState::new(vec![]));
        m.publish_shared(&mut slot);
        assert_eq!((ROOT_BUILDS.get(), slot), (1, Some(0)));
    }

    /// An error built inline in the source of the op loop shifts the cost of unrelated ops by
    /// percent (#508): every error with a message is built by `cold`, through `ice_f!` and its
    /// siblings.
    #[test]
    fn errors_are_built_out_of_line() {
        let calls = [
            ("ice", "(format!("),
            ("rt_err", "(format!("),
            ("Fault::Runtime", "(format!("),
        ];
        let lits = [("ice", "(\""), ("rt_err", "(\""), ("Fault::Runtime", "(\"")];
        for (n, l) in include_str!("exec.rs").lines().enumerate() {
            if l.trim_start().starts_with("//") || l.contains("cold(move ||") {
                continue;
            }
            for (a, b) in calls.iter().chain(&lits) {
                assert!(
                    !l.contains(&format!("{a}{b}")),
                    "exec.rs:{}: `{a}{b}..` builds an error inline, use `{}_f!` or `{}_s!`",
                    n + 1,
                    a.trim_start_matches("Fault::").to_lowercase(),
                    a.trim_start_matches("Fault::").to_lowercase()
                );
            }
        }
    }
}
