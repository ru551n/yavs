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
    AddrSpace, AttrKind, BlockId, BoolOp, CheckKind, CmpOp, ConstData, Dir, DriveMode, FOp, FUnOp,
    ForceKind, Func, FuncId, FuncKind, Inst, IntOp, IntUnOp, LNumOp, LOp, LayoutError, ParamClass,
    PhysOp, Program, Reg, StmtId, Ty, TyId, Width,
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
use crate::{InstData, ProcDriver, ProcRoots, Runtime, SharedState};

/// An execution failure before it becomes a `SimError` with a trace.
#[derive(Debug)]
pub(crate) enum Fault {
    Runtime(String),
    /// A call of an unbound foreign subprogram: message and places searched.
    Unbound(Box<(String, String)>),
    Internal(String),
    Kernel(SimError),
}

pub(crate) type R<T> = Result<T, Fault>;

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
        return Err(ice("signal reference parts of different element types"));
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
                return Err(ice("logic signal part without a logic value"));
            }
            let codes: Vec<_> = ls.iter().flat_map(|l| l.iter()).collect();
            Value::Logic(LogicVec::from_fn(codes.len(), |i| codes[i]))
        }
    })
}

/// Elements `at .. at + len` of a value of `elem` elements.
fn sub_value(v: &Value, elem: Elem, at: u32, len: u32) -> R<Value> {
    let bad = || ice("signal value shorter than its reference");
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
pub(crate) const HEAP: u8 = 4;
pub const TEMP: u8 = 5;
pub const SHARED: u8 = 6;
pub(crate) const SHARED_HEAP: u8 = 7;

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
pub(crate) fn mk_addr(space: u8, index: u32, off: u64) -> R<u64> {
    if off > u64::from(u32::MAX) {
        return rt_err("object larger than 4 GiB");
    }
    if index >= 1 << 29 {
        return Err(ice(format!("address index {index} too large")));
    }
    Ok(u64::from(space) << 61 | u64::from(index) << 32 | off)
}

pub(crate) fn split(a: u64) -> (u8, u32, u64) {
    (
        (a >> 61) as u8,
        ((a >> 32) & ((1 << 29) - 1)) as u32,
        a & 0xffff_ffff,
    )
}

pub(crate) fn add_addr(a: u64, delta: u64) -> R<u64> {
    let (s, i, o) = split(a);
    match o.checked_add(delta) {
        Some(o) => mk_addr(s, i, o),
        None => rt_err("object larger than 4 GiB"),
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

/// The body of `Machine::push_frame` and `push_frame_pre`.
macro_rules! push_frame_body {
    ($self:ident, $pre:ident, $f:ident, $func:ident, $link:ident, $ret:ident) => {{
        let idx = $self.frames.len();
        if idx >= $self.rt.limits.max_depth as usize {
            return $self.depth_fail($f);
        }
        let slen = $pre.slots as usize;
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
        $pre.fill(&mut $self.regs.v, idx as u32)?;
        // A root frame entered with the shared state attached is elaboration code
        // (`call_shared`, `init_package`, `call_values_shared`); a process enters its
        // root frame before it first takes the shared state, and allocates in the shared
        // heap when it is serial: what it allocates may end up in a shared variable (#303).
        let shared_alloc = match $self.frames.last() {
            Some(top) => top.shared_alloc,
            None => {
                $self.sh.is_some() || $f.kind == FuncKind::Process && $self.rt.prog.is_serial($func)
            }
        };
        $self.frames.push(Frame {
            func: $func,
            pc: 0,
            rbase: rb,
            sbase: sb,
            slen: $pre.slots,
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
    rt_err(format!("call stack exceeds 4 GiB calling `{}`", f.name))
}

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

enum Flow {
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

pub(crate) struct Machine {
    pub rt: Arc<Runtime>,
    /// A second handle on `rt` that `exec` takes while it runs, so an activation does not
    /// touch the shared reference count (every process of a batch has the same runtime).
    rt_run: Option<Arc<Runtime>>,
    pub inst: Arc<InstData>,
    pub drivers: Vec<ProcDriver>,
    pub frames: Vec<Frame>,
    /// The registers of all frames, one stack (`push_frame`, `pop_frame`; never push or pop
    /// `frames` directly).
    regs: Regs,
    /// The slots of frame 0 (a process's variables survive its suspensions).
    root: Vec<u8>,
    /// The slots of the other frames, one stack: no per-call allocation.
    sstack: Vec<u8>,
    pub heap: Heap,
    pub temp: Vec<u8>,
    /// Temporaries that hold handles or heap addresses (`Inst::TempRoot`), until the
    /// statement that made them ends (`Inst::Stmt` drops those at or above its frame's base).
    troots: Vec<TempRoot>,
    pub sh: Option<SharedState>,
    /// The package being initialised (`init_package`): its index and writable data.
    pub pkg_init: Option<(u32, Vec<u8>)>,
    /// Package initialisation did something other than compute package state (printed, read the
    /// environment or a clock, touched a file, called foreign code): its result is not stored
    /// (`SharedState::tainted`, #407).
    pub(crate) tainted: Option<String>,
    /// The reports it printed, to print again when its state is restored (`Some` while a
    /// package is initialised).
    pub(crate) init_reports: Option<Vec<(u8, String)>>,
    pub entry: FuncId,
    dbg: Option<Dbg>,
    waits: Waits,
    /// Stop when the entry frame jumps to this block (a process's body, #342).
    pub pause: Option<yavs_ir::BlockId>,
    /// A native `ieee` subprogram made a report at the stop severity: the process ends after
    /// the call, as after `Inst::Report`.
    pub ieee_stop: bool,
    /// A native `ieee` subprogram gives its call back to the VHDL body (`ieee::fallback`).
    pub ieee_fallback: bool,
    /// The last `LayoutOf` of each type (indexed by `TyId`): descriptor size, descriptor bytes
    /// and the layout words written. Layouts are pure functions of type and descriptor.
    layouts: Vec<Option<LayoutMemo>>,
    /// Argument cells of the call being made (`call_with`).
    cells: Vec<u64>,
    /// The staging buffer of `Copy` and `Fill` (`take_bytes`), reused.
    bytes: Vec<u8>,
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
    /// The fault of the last `run_quick`.
    quick_fault: Option<Fault>,
}

/// Registers of a process's root frame up to which it keeps its working vectors
/// ([`Machine::resident`]): at most 384 bytes of registers per process.
const RESIDENT_REGS: usize = 48;
/// Capacity of the temporary area above which a resident process gives its vectors to the
/// scratch.
const RESIDENT_TEMP: usize = 1024;

/// The working vectors of the process running on this thread: registers and temporaries. A suspended process owns none of them (no register or temporary is
/// live across a suspension, ir.md 1.2), so a design of thousands of processes keeps one set
/// per thread instead of a set each (#409).
#[derive(Default)]
struct Scratch {
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
            regs: Vec::new(),
            temp: Vec::new(),
            key: 0,
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

fn sev(v: u64) -> R<Severity> {
    Ok(match v {
        0 => Severity::Note,
        1 => Severity::Warning,
        2 => Severity::Error,
        3 => Severity::Failure,
        _ => return Err(ice(format!("severity {v}"))),
    })
}

fn to_time(v: i64) -> R<Time> {
    Time::from_vhdl(v).map_err(|_| Fault::Runtime(format!("negative time value {v} fs")))
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
        arith::ArithError::DivisionByZero { .. } => Fault::Runtime("division by zero".into()),
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
        .ok_or_else(|| Fault::Runtime("object larger than 4 GiB".into()))?;
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

/// A real as check messages show it: `real'image` (`%.15g`, NVC's form: `30000000000`).
fn real_text(x: f64) -> String {
    String::from_utf8_lossy(&yavs_std::image::real_image(x)).into_owned()
}

/// `<kind> <v> outside of [<subtype> ]range <bounds>[ for <what>]` (NVC's wording).
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
        rt_err(format!("real {what} result {x} is out of range"))
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

/// The blocks of `bytes` (32 bytes each) into `out`.
fn fill_blocks(bytes: &[u8], out: &mut [[u64; 4]]) {
    for (o, c) in out.iter_mut().zip(bytes.as_chunks::<32>().0) {
        for (w, b) in o.iter_mut().zip(c.as_chunks::<8>().0) {
            *w = u64::from_le_bytes(*b);
        }
    }
}

/// `n` elements of the planes `sw` from element `s0` to those of `dw` from element `d0`
/// (`Inst::LCopy`; compiled code calls it on the same memory, `yavs-llvm`).
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
                .ok_or_else(|| ice(format!("no constant {c}")))?;
            if cell.get().is_none() {
                let _ = cell.set(expand_const(d)?);
            }
            Ok(cell.get().map(Vec::as_slice).unwrap_or(&[]))
        }
        None => Err(ice(format!("no constant {c}"))),
    }
}

/// Plane `p` of the 64-element block at `b` (32 bytes, plane words little endian).
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

impl Machine {
    pub(crate) fn new(
        rt: Arc<Runtime>,
        inst: Arc<InstData>,
        drivers: Vec<ProcDriver>,
        entry: FuncId,
    ) -> Machine {
        Machine {
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
            cells: Vec::new(),
            bytes: Vec::new(),
            released: false,
            resident: false,
            root_process: false,
            quick_fault: None,
        }
    }

    /// The `Scratch::key` of the root frame's function.
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
        self.borrow_scratch_slow()
    }

    #[inline(never)]
    fn borrow_scratch_slow(&mut self) -> R<()> {
        self.released = false;
        let root = self.frames.first().map(|fr| fr.func);
        let key = self.scratch_key();
        let pristine = SCRATCH.with(|s| {
            let mut s = s.borrow_mut();
            std::mem::swap(&mut self.regs.v, &mut s.regs);
            std::mem::swap(&mut self.temp, &mut s.temp);
            // The scratch now holds the machine's empty vectors.
            std::mem::replace(&mut s.key, 0) == key && key != 0
        });
        self.regs.base = 0;
        if pristine {
            self.temp.clear();
            self.troots.clear();
            return Ok(());
        }
        // Registers are written before they are read: stale values are harmless, except
        // those a frame starts with (`pre.rs`).
        let rt = &self.rt;
        match root.and_then(|id| Some((id, rt.prog.func(id)?))) {
            Some((id, f)) => {
                let pre = rt
                    .pre(id, f)
                    .ok_or_else(|| ice(format!("no function {id}")))?;
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
        // A resident process has no debugger (`set_resident`) and keeps its vectors.
        if self.resident && self.frames.len() == 1 && self.temp.capacity() <= RESIDENT_TEMP {
            self.temp.clear();
            if !self.troots.is_empty() {
                self.troots.clear();
            }
            return;
        }
        self.release_scratch_slow();
    }

    #[inline(never)]
    fn release_scratch_slow(&mut self) {
        if self.frames.len() != 1 || self.released || self.dbg.is_some() {
            return;
        }
        self.resident = false;
        self.released = true;
        self.troots.clear();
        let key = self.scratch_key();
        SCRATCH.with(|s| {
            let mut s = s.borrow_mut();
            s.regs = std::mem::take(&mut self.regs.v);
            s.temp = std::mem::take(&mut self.temp);
            s.key = key;
        });
    }

    pub(crate) fn attach(&mut self, session: Arc<DebugSession>, index: usize) {
        self.dbg = Some(Dbg {
            session,
            index,
            mode: StepMode::Run,
            fuel: 0,
        });
    }

    /// Push the frame of `func` with `args` (one cell per parameter, two for a signal).
    pub(crate) fn enter(&mut self, func: FuncId, args: &[u64]) -> R<()> {
        let rt = self.rt.clone();
        let f = rt
            .prog
            .func(func)
            .ok_or_else(|| ice(format!("no function {func}")))?;
        self.push_frame(f, func, None, None)?;
        let mut k = 0;
        for p in &f.params {
            let n = if p.class == ParamClass::Signal { 2 } else { 1 };
            let cells = args
                .get(k..k + n)
                .ok_or_else(|| Fault::Runtime(format!("missing argument {}", p.name)))?;
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
            .ok_or_else(|| ice(format!("no function {func}")))?;
        push_frame_body!(self, pre, f, func, link, ret);
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
        push_frame_body!(self, pre, f, func, link, ret);
        Ok(())
    }

    #[cold]
    #[inline(never)]
    fn depth_fail(&self, f: &Func) -> R<()> {
        rt_err(format!(
            "call depth limit {} exceeded calling `{}` (unbounded recursion?)",
            self.rt.limits.max_depth, f.name
        ))
    }

    /// Pop the top frame; the caller's registers become current.
    pub(crate) fn pop_frame(&mut self) -> Option<Frame> {
        let fr = self.pop_frame_in_place()?;
        // Back at the root: a deep recursion does not keep its stack.
        if self.frames.len() == 1 && self.sstack.capacity() > 1 << 16 {
            self.sstack.shrink_to(1 << 12);
            self.regs.v.shrink_to(1 << 10);
        }
        Some(fr)
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

    /// The VHDL call stack, innermost first.
    fn trace(&self) -> Vec<StmtId> {
        self.frames
            .iter()
            .rev()
            .filter_map(|fr| {
                let f = self.rt.prog.func(fr.func)?;
                f.loc.get(fr.pc as usize).copied()
            })
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

    /// After the root frame is entered: a small process keeps its working vectors
    /// ([`Machine::resident`]).
    pub(crate) fn set_resident(&mut self) {
        self.resident = self.dbg.is_none() && self.regs.v.len() <= RESIDENT_REGS;
        self.root_process = true;
    }

    /// Whether `run_quick` applies (no debugger).
    pub(crate) fn can_run_quick(&self) -> bool {
        self.dbg.is_none()
    }

    /// `run` without a debugger and with base 0, the outcome as a `Quick`.
    pub(crate) fn run_quick(&mut self, ctx: &mut Option<&mut Ctx<'_>>) -> Quick {
        let rt = self.rt_run.take().unwrap_or_else(|| self.rt.clone());
        let r = match rt.tier.get() {
            Some(tier) if rt.native.any.load(Ordering::Acquire) => {
                let r = self.exec_fast::<true>(&rt, Some(&**tier), ctx, 0);
                // Nothing interpreted on this thread until the next run.
                crate::slot().store(0, Ordering::Relaxed);
                r
            }
            tier => {
                let r = self.exec_fast::<false>(&rt, tier.map(|t| &**t), ctx, 0);
                if tier.is_some() {
                    crate::slot().store(0, Ordering::Relaxed);
                }
                r
            }
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

    pub(crate) fn take_quick_fault(&mut self) -> Fault {
        self.quick_fault
            .take()
            .unwrap_or_else(|| ice("a fault without a fault"))
    }

    /// GC of the private heap and the debug process table, after a suspension.
    #[inline(always)]
    pub(crate) fn after_suspend(&mut self) {
        // Nothing allocated since the last collection, nothing deallocated: nothing to do.
        if self.heap.since_gc == 0 && !self.heap.has_quarantine() && self.dbg.is_none() {
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
        let prog = &self.rt.prog;
        let mut r = heap::Refs::default();
        for (k, fr) in self.frames.iter().enumerate() {
            let f = prog.func(fr.func)?;
            if !heap::slot_roots(prog, &f.roots, self.fslots(k)?, &mut r) {
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
        Some(r)
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
        let mine = self.refs(false).map(|r| ProcRoots {
            pinned: r.pinned(SHARED_HEAP),
            handles: r
                .handles
                .into_iter()
                .filter(|&h| heap::tag_of(h) == heap::SHARED)
                .collect(),
        });
        let Some(sh) = self.sh.as_mut() else {
            return;
        };
        let i = *slot.get_or_insert_with(|| {
            sh.procs.push(None);
            sh.procs.len() - 1
        });
        sh.procs[i] = mine;
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

    #[inline]
    fn top(&mut self) -> &mut Frame {
        let n = self.frames.len();
        &mut self.frames[n - 1]
    }

    #[inline]
    pub(crate) fn r(&self, r: Reg) -> u64 {
        self.regs[r.0 as usize]
    }

    #[inline]
    fn ri(&self, r: Reg) -> i64 {
        self.r(r) as i64
    }

    #[inline]
    fn rf(&self, r: Reg) -> f64 {
        f64::from_bits(self.r(r))
    }

    #[inline]
    fn set(&mut self, r: Reg, v: u64) {
        self.regs[r.0 as usize] = v;
    }

    // ---- memory ----

    pub(crate) fn shared(&self) -> R<&SharedState> {
        self.sh
            .as_ref()
            .ok_or_else(|| ice("shared state accessed by a process that is not marked serial"))
    }

    pub(crate) fn shared_mut(&mut self) -> R<&mut SharedState> {
        self.sh
            .as_mut()
            .ok_or_else(|| ice("shared state accessed by a process that is not marked serial"))
    }

    /// The object of the spaces that are accessed all the time, without error paths.
    #[inline(always)]
    fn mem_fast(&self, sp: u8, idx: u32) -> Option<&[u8]> {
        match sp {
            FRAME => self.fslots(idx as usize),
            TEMP => Some(&self.temp),
            INST => Some(&self.inst.data),
            HEAP => self.heap.data(idx),
            // Constants stored as bytes (compact ones are expanded by `obj`).
            CONST => match self.rt.prog.consts.get(yavs_ir::ConstId(idx)) {
                Some(ConstData::Bytes(b)) => Some(b),
                _ => None,
            },
            _ => None,
        }
    }

    /// `rd` with the common spaces inline.
    #[inline(always)]
    fn rd_hot(&self, a: u64, n: u64) -> R<&[u8]> {
        let (sp, idx, off) = split(a);
        match self
            .mem_fast(sp, idx)
            .and_then(|m| m.get(off as usize..(off + n) as usize))
        {
            Some(b) => Ok(b),
            None => self.rd(a, n),
        }
    }

    pub(crate) fn rd(&self, a: u64, n: u64) -> R<&[u8]> {
        let (sp, idx, off) = split(a);
        let mem = self.obj(sp, idx)?;
        let end = off.saturating_add(n);
        mem.get(off as usize..end as usize).ok_or_else(|| {
            ice(format!(
                "read of {n} bytes at {off} outside a {} object of {} bytes",
                SPACE_NAMES[sp as usize],
                mem.len()
            ))
        })
    }

    /// The memory object of space `sp` and index `idx`, for reading.
    fn obj(&self, sp: u8, idx: u32) -> R<&[u8]> {
        Ok(match sp {
            FRAME => self
                .fslots(idx as usize)
                .ok_or_else(|| ice(format!("address of dead frame {idx}")))?,
            INST => &self.inst.data,
            PKG => match &self.pkg_init {
                Some((p, data)) if *p == idx => data,
                _ => {
                    &self
                        .rt
                        .pkgs
                        .get(idx as usize)
                        .ok_or_else(|| ice(format!("no package instance {idx}")))?
                        .data
                }
            },
            CONST => const_bytes(&self.rt, yavs_ir::ConstId(idx))?,
            HEAP => self
                .heap
                .data(idx)
                .ok_or_else(|| Fault::Runtime("access to a deallocated object".into()))?,
            TEMP => &self.temp,
            SHARED => &self.shared()?.vars,
            _ => self
                .shared()?
                .heap
                .as_ref()
                .and_then(|h| h.data(idx))
                .ok_or_else(|| Fault::Runtime("access to a deallocated object".into()))?,
        })
    }

    pub(crate) fn wr(&mut self, a: u64, n: u64) -> R<&mut [u8]> {
        let (sp, idx, off) = split(a);
        let mem = self.obj_mut(sp, idx)?;
        let len = mem.len();
        let end = off.saturating_add(n);
        mem.get_mut(off as usize..end as usize).ok_or_else(|| {
            ice(format!(
                "write of {n} bytes at {off} outside a {} object of {len} bytes",
                SPACE_NAMES[sp as usize]
            ))
        })
    }

    /// The memory object of space `sp` and index `idx`, for writing.
    fn obj_mut(&mut self, sp: u8, idx: u32) -> R<&mut [u8]> {
        Ok(match sp {
            FRAME => self
                .fslots_mut(idx as usize)
                .ok_or_else(|| ice(format!("address of dead frame {idx}")))?,
            HEAP => self
                .heap
                .data_mut(idx)
                .ok_or_else(|| Fault::Runtime("access to a deallocated object".into()))?,
            TEMP => &mut self.temp,
            PKG => match &mut self.pkg_init {
                Some((p, data)) if *p == idx => data,
                _ => {
                    return Err(ice(format!(
                        "write to package {idx} outside its initialisation"
                    )));
                }
            },
            SHARED => &mut self.shared_mut()?.vars,
            SHARED_HEAP => self
                .shared_mut()?
                .heap
                .as_mut()
                .and_then(|h| h.data_mut(idx))
                .ok_or_else(|| Fault::Runtime("access to a deallocated object".into()))?,
            _ => {
                return Err(ice(format!(
                    "write to read-only {} memory",
                    SPACE_NAMES[sp as usize]
                )));
            }
        })
    }

    /// The [`crate::Stamp`] of the heap object at address `a`; `None` for other memory.
    pub(crate) fn stamp(&self, a: u64) -> Option<crate::Stamp> {
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
            _ => None,
        };
        if let Some(b) = mem.and_then(|m| m.get_mut(off as usize..off as usize + n)) {
            b.copy_from_slice(bytes);
            return Ok(());
        }
        self.wr(a, n as u64)?.copy_from_slice(bytes);
        Ok(())
    }

    #[inline]
    pub(crate) fn load(&self, a: u64, w: Width) -> R<u64> {
        let n = w.bytes().min(8) as u64;
        // Fast path: a frame (registers spill to frame slots all the time), temporary or
        // instance memory.
        let (sp, idx, off) = split(a);
        let mem = self.mem_fast(sp, idx);
        if let Some(b) = mem.and_then(|m| m.get(off as usize..(off + n) as usize)) {
            return Ok(cell_w(b, w));
        }
        Ok(cell_w(self.rd(a, n)?, w))
    }

    /// Bounds and direction word of dimension `dim` of the descriptor at `base`.
    #[inline(always)]
    fn dim_of(&self, base: u64, dim: u64) -> R<(i64, i64, u64)> {
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
                .ok_or_else(|| ice("short descriptor"))
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
    fn fat_root(&mut self, a: u64) -> R<()> {
        let (sp, _, off) = split(a);
        let w = self
            .temp
            .get(off as usize..off as usize + 16)
            .filter(|_| sp == TEMP)
            .ok_or_else(|| ice("a recorded fat pair outside the temporary area"))?;
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

    pub(crate) fn temp_alloc(&mut self, n: u64) -> R<u64> {
        let off = (self.temp.len() as u64).next_multiple_of(8);
        let end = off.saturating_add(n);
        if end > u64::from(u32::MAX) {
            return rt_err("temporary memory of one statement exceeds 4 GiB");
        }
        self.temp.resize(end as usize, 0);
        mk_addr(TEMP, 0, off)
    }

    /// A string result: the fat pair `[data, dope]`, the `1 to len` descriptor and the bytes in
    /// one block of temporary memory.
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

    fn lcopy(&mut self, dst: u64, doff: u64, src: u64, soff: u64, n: u64) -> R<()> {
        if n == 0 {
            return Ok(());
        }
        let last = |off: u64| {
            off.checked_add(n - 1)
                .ok_or_else(|| ice(format!("logic copy of {n} elements at {off}")))
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
        let sw = blocks_of(self.rd(add_addr(src, sb0 * 32)?, sn as u64 * 32)?);
        let mut dw = blocks_of(self.rd(da, dn as u64 * 32)?);
        copy_bits(&sw, &mut dw, s0, d0, n);
        self.put(da, &bytes_of(&dw))
    }

    /// `Inst::SigRead` of a reference of one range: the signal's value into the memory at `dst`.
    #[inline(always)]
    fn sig_read_vec(&mut self, dst: Reg, sig: Reg, ctx: &mut Option<&mut Ctx<'_>>) -> R<()> {
        let (sr, ty) = self.sigref(sig)?;
        let Some(c) = ctx.as_deref_mut() else {
            return rt_err("signal or wait operation during elaboration");
        };
        let n = value_size(ty.elem, sr.range.map_or(0, |r| r.len));
        c.read_into(sr, self.wr(self.r(dst), n)?)?;
        Ok(())
    }

    fn delay_mode(&self, mode: DriveMode, reject: Option<Reg>) -> R<DelayMode> {
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
        let w = self.r(wave);
        // One scalar logic element: no value, no lists (a scalar assignment per activation).
        if elem == Elem::Logic && len == 1 && self.r(n) == 1 {
            let t = to_time(self.load(w, Width::B8)? as i64)?;
            let b = self.rd(add_addr(w, 8)?, 32)?;
            let code = (0..4).fold(0u8, |c, p| c | (b[p * 8] & 1) << p);
            let mode = self.delay_mode(mode, reject)?;
            let range = target.map(|_| sr.range).unwrap_or(None);
            let Some(c) = ctx.as_deref_mut() else {
                return rt_err("signal or wait operation during elaboration");
            };
            c.assign_code(d.id, range, t, code, mode)?;
            return Ok(());
        }
        // One waveform element (nearly every assignment): no vectors.
        if self.r(n) == 1 {
            let t = to_time(self.load(w, Width::B8)? as i64)?;
            let v = self.get_value(add_addr(w, 8)?, elem, len)?;
            let mode = self.delay_mode(mode, reject)?;
            let range = target.map(|_| sr.range).unwrap_or(None);
            let Some(c) = ctx.as_deref_mut() else {
                return rt_err("signal or wait operation during elaboration");
            };
            c.assign(d.id, range, &[(t, v.as_ref())], mode)?;
            return Ok(());
        }
        let mut items = Vec::new();
        for k in 0..self.r(n) {
            let at = add_addr(w, k * stride)?;
            let t = to_time(self.load(at, Width::B8)? as i64)?;
            items.push((t, self.get_value(add_addr(at, 8)?, elem, len)?));
        }
        let refs: Vec<(Time, ValueRef<'_>)> = items.iter().map(|(t, v)| (*t, v.as_ref())).collect();
        let mode = self.delay_mode(mode, reject)?;
        let range = target.map(|_| sr.range).unwrap_or(None);
        let Some(c) = ctx.as_deref_mut() else {
            return rt_err("signal or wait operation during elaboration");
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
        let (sr, ty) = self.sigref(target)?;
        if ty.elem != Elem::Logic || sr.range.map_or(0, |r| r.len) != 1 {
            return Ok(false);
        }
        let t = to_time(self.load(self.r(wave), Width::B8)? as i64)?;
        let mode = self.delay_mode(mode, None)?;
        let Some(c) = ctx.as_deref_mut() else {
            return rt_err("signal or wait operation during elaboration");
        };
        c.assign_code(d.id, sr.range, t, code, mode)?;
        Ok(true)
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
                return Err(ice("a signal assignment left its straight line"));
            }
            if matches!(inst, Inst::Drive { .. }) {
                return Ok(());
            }
        }
        Err(ice("a signal assignment without its drive"))
    }

    pub(crate) fn sigref(&self, r: Reg) -> R<(SigRef, yavs_kernel::SignalType)> {
        let v = self.r(r);
        let len = self.r(Reg(r.0 + 1)) as u32;
        let (base, off) = ((v >> 32) as u32, v as u32);
        let s = self
            .inst
            .sigs
            .get(base as usize)
            .ok_or_else(|| ice(format!("no instance signal {base}")))?;
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
                    .ok_or_else(|| ice(format!("no instance signal {base}")))?;
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
            let slot = slot.ok_or_else(|| ice("a signal part without a driver is driven"))?;
            let d = self.driver(slot)?;
            let vals = items
                .iter()
                .map(|(t, v)| Ok((*t, sub_value(v, elem, at, len)?)))
                .collect::<R<Vec<(Time, Value)>>>()?;
            let refs: Vec<(Time, ValueRef<'_>)> =
                vals.iter().map(|(t, v)| (*t, v.as_ref())).collect();
            let Some(c) = ctx.as_deref_mut() else {
                return rt_err("signal or wait operation during elaboration");
            };
            c.assign(d.id, sr.range, &refs, mode)?;
            at += len;
        }
        Ok(())
    }

    pub(crate) fn is_multi(&self, r: Reg) -> bool {
        self.r(r) >> 32 == MULTI
    }

    pub(crate) fn driver(&self, d: u32) -> R<ProcDriver> {
        self.drivers
            .get(d as usize)
            .copied()
            .ok_or_else(|| ice(format!("no driver slot {d}")))
    }

    fn driver_ref(&self, d: &ProcDriver) -> R<(SigRef, Elem)> {
        let s = self
            .inst
            .sigs
            .get(d.sig as usize)
            .ok_or_else(|| ice(format!("no instance signal {}", d.sig)))?;
        Ok((
            s.signal.slice(s.off + d.range.offset, d.range.len),
            s.ty.elem,
        ))
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
            Err(e) => return Err(ice(format!("deallocate: {e:?}"))),
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
            t => Err(ice(format!("access value with heap tag {t}"))),
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
    fn layout_words(&mut self, prog: &Program, ty: TyId, desc: u64) -> R<&[u8]> {
        let t = ty.0 as usize;
        if self.layouts.len() <= t {
            self.layouts.resize_with(t + 1, || None);
        }
        let mut memo = self.layouts[t].take();
        let dsz = match &memo {
            Some(m) => m.dsz,
            None => Ty::desc_size(&prog.types, ty)
                .ok_or_else(|| ice(format!("type {ty} has no descriptor size")))?,
        };
        let d = self.rd(desc, dsz)?;
        let ckey = if split(desc).0 == CONST { desc } else { 0 };
        if let Some(m) = memo.as_mut()
            && let Some(n) = m.simple
            && let Some(w) = simple_words(n, d)
        {
            m.words.clear();
            for w in w {
                m.words.extend_from_slice(&w.to_le_bytes());
            }
            m.ckey = ckey;
            let m = self.layouts[t].insert(memo.take().ok_or_else(|| ice("layout memo missing"))?);
            return Ok(&m.words);
        }
        if memo.as_ref().is_none_or(|m| m.desc != d) {
            let (l, _) = Ty::layout(&prog.types, &prog.consts, ty, d).map_err(|e| match e {
                LayoutError::TooLarge => Fault::Runtime(e.to_string()),
                LayoutError::Malformed(_) => ice(e.to_string()),
            })?;
            // Refill the old memo in place: strings of changing lengths miss at nearly every
            // call, and the memo must not allocate then.
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
        let m = self.layouts[t].insert(memo.ok_or_else(|| ice("layout memo missing"))?);
        Ok(&m.words)
    }

    /// `Inst::LayoutOf`: the layout words in temporary memory.
    fn layout_of(&mut self, prog: &Program, ty: TyId, desc: u64) -> R<u64> {
        let n = self.layout_words(prog, ty, desc)?.len() as u64;
        let a = self.temp_alloc(n)?;
        let t = ty.0 as usize;
        let (temp, words) = (&mut self.temp, &self.layouts[t]);
        let words = words.as_ref().map(|m| &m.words[..]).unwrap_or_default();
        let off = split(a).2 as usize;
        temp.get_mut(off..off + words.len())
            .ok_or_else(|| ice("layout words outside the temporary area"))?
            .copy_from_slice(words);
        Ok(a)
    }

    /// Word `k` of the layout of `ty` constrained by the descriptor at `desc`, without
    /// writing the layout to memory (a `LayoutOf` whose result only one load reads).
    fn layout_word(&mut self, prog: &Program, ty: TyId, desc: u64, k: u32) -> R<u64> {
        if let Some(Some(m)) = self.layouts.get(ty.0 as usize)
            && m.ckey == desc
            && desc != 0
            && let Some(b) = m.words.get(k as usize * 8..k as usize * 8 + 8)
        {
            return Ok(le_bytes(b));
        }
        // A memoized array of scalars: the word straight from the bounds.
        if let Some(Some(m)) = self.layouts.get(ty.0 as usize)
            && let Some(n) = m.simple
            && let Ok(d) = self.rd_hot(desc, m.dsz)
            && let Some(w) = simple_words(n, d)
            && let Some(&v) = w.get(k as usize)
        {
            return Ok(v);
        }
        let words = self.layout_words(prog, ty, desc)?;
        let o = k as usize * 8;
        match words.get(o..o + 8) {
            Some(b) => Ok(le_bytes(b)),
            None => Err(ice(format!(
                "read of 8 bytes at {o} outside a temporary object of {} bytes",
                words.len()
            ))),
        }
    }

    /// `Inst::LGet`: the std_ulogic code of element `i` of the logic vector at `base`.
    #[inline(always)]
    fn lget(&self, base: u64, i: u64) -> R<u64> {
        let b = self.rd(add_addr(base, i / 64 * 32)?, 32)?;
        Ok((0..4).fold(0u64, |c, p| c | (plane_word(b, p) >> (i % 64) & 1) << p))
    }

    /// `Inst::LSet`: set element `i` of the logic vector at `base` to `code`.
    #[inline(always)]
    fn lset(&mut self, base: u64, i: u64, code: u64) -> R<()> {
        if code > 8 {
            return Err(ice(format!("std_ulogic code {code}")));
        }
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
            return Err(ice(format!("std_ulogic code {code}")));
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

    /// `Inst::Cat`: the pieces `ps` (pairs of an address or element and a count) one after
    /// another into the array of `n` elements at `d`, by the stores of the `LCopy`, `LSet`,
    /// `Copy` and `Store` it stands for.
    #[allow(clippy::too_many_arguments)]
    fn cat(
        &mut self,
        d: u64,
        n: u64,
        m: u64,
        ps: &[Reg],
        scalars: u16,
        bias: u8,
        logic: bool,
        w: Width,
    ) -> R<()> {
        let big = || Fault::Runtime("object larger than 4 GiB".into());
        let total = n.wrapping_mul(m);
        if logic && total <= 64 && self.cat_block(d, total, m, ps, scalars, bias)? {
            return Ok(());
        }
        let mut k = 0u64;
        for (j, pr) in ps.as_chunks::<2>().0.iter().enumerate() {
            let v = self.r(pr[0]);
            let scalar = scalars >> j & 1 == 1;
            let len = if scalar { 1 } else { self.r(pr[1]) };
            if logic {
                let units = if scalar { 1 } else { len.wrapping_mul(m) };
                let off = total.wrapping_sub(k.wrapping_mul(m)).wrapping_sub(units);
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
    fn cat_block(
        &mut self,
        d: u64,
        total: u64,
        m: u64,
        ps: &[Reg],
        scalars: u16,
        bias: u8,
    ) -> R<bool> {
        let mut words = [0u64; 4];
        let mut mask = 0u64;
        let mut k = 0u64;
        for (j, pr) in ps.as_chunks::<2>().0.iter().enumerate() {
            let v = self.r(pr[0]);
            let (units, bits) = if scalars >> j & 1 == 1 {
                let code = v.wrapping_add(u64::from(bias));
                if code > 8 {
                    return Ok(false);
                }
                (1, [code & 1, code >> 1 & 1, code >> 2 & 1, code >> 3 & 1])
            } else {
                let units = self.r(pr[1]).wrapping_mul(m);
                if units == 0 {
                    continue;
                }
                if units > 64 {
                    return Ok(false);
                }
                let mut b = [0u64; 4];
                fill_blocks(self.rd_hot(v, 32)?, std::slice::from_mut(&mut b));
                (units, b)
            };
            if k.wrapping_add(units) > total {
                return Ok(false);
            }
            let off = total - k - units;
            let pm = if units == 64 { !0 } else { (1u64 << units) - 1 };
            for (w, b) in words.iter_mut().zip(bits) {
                *w |= (b & pm) << off;
            }
            mask |= pm << off;
            k += units;
        }
        if mask == 0 {
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
            inst,
            rt,
            ..
        } = self;
        if dsp == ssp && didx == sidx {
            let b = match dsp {
                FRAME => slots_of_mut(frames, root, sstack, didx as usize),
                TEMP => Some(&mut temp[..]),
                HEAP => heap.data_mut(didx),
                _ => None,
            };
            let Some(b) = b else { return false };
            if doff + n <= b.len() && soff + n <= b.len() {
                b.copy_within(soff..soff + n, doff);
                return true;
            }
            return false;
        }
        let dst = match dsp {
            TEMP => {
                let src = match ssp {
                    FRAME => slots_of(frames, root, sstack, sidx as usize),
                    INST => Some(&inst.data[..]),
                    CONST => const_bytes(rt, yavs_ir::ConstId(sidx)).ok(),
                    HEAP => heap.data(sidx),
                    _ => None,
                };
                (src, temp.get_mut(doff..doff + n))
            }
            FRAME => {
                let src = match ssp {
                    TEMP => Some(&temp[..]),
                    INST => Some(&inst.data[..]),
                    CONST => const_bytes(rt, yavs_ir::ConstId(sidx)).ok(),
                    HEAP => heap.data(sidx),
                    _ => None,
                };
                (
                    src,
                    slots_of_mut(frames, root, sstack, didx as usize)
                        .and_then(|b| b.get_mut(doff..doff + n)),
                )
            }
            HEAP => {
                let src = match ssp {
                    FRAME => slots_of(frames, root, sstack, sidx as usize),
                    TEMP => Some(&temp[..]),
                    INST => Some(&inst.data[..]),
                    CONST => const_bytes(rt, yavs_ir::ConstId(sidx)).ok(),
                    _ => None,
                };
                (
                    src,
                    heap.data_mut(didx).and_then(|b| b.get_mut(doff..doff + n)),
                )
            }
            _ => return false,
        };
        match dst {
            (Some(src), Some(out)) if soff + n <= src.len() => {
                out.copy_from_slice(&src[soff..soff + n]);
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
    /// the one at `want`.
    #[inline(always)]
    fn subtype_check(&self, dope: u64, want: u64, dims: u8, what: yavs_ir::CheckMsgId) -> R<()> {
        for k in 0..u64::from(dims) {
            let len = |base: u64| -> R<u64> {
                let (l, r, dir) = self.dim_of(base, k)?;
                yavs_ir::dim_len(l, r, dir == 1)
                    .ok_or_else(|| Fault::Runtime("range too long".into()))
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
        rt_err(format!(
            "value length {a}{dim} does not match {w} length {b}"
        ))
    }

    /// `Inst::Deref`: the address of the object the access value `h` designates.
    #[inline(always)]
    fn deref(&mut self, h: u64, what: yavs_ir::CheckMsgId) -> R<u64> {
        if h == 0 {
            // NVC's wording; the statement span names the place.
            return rt_err("null access dereference");
        }
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
                rt_err(format!(
                    "access value designates an object that was deallocated: {w}"
                ))
            }
            e => Err(ice(format!("dereference: {e:?}"))),
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
        self.step(prog, f, inst, ctx)
    }

    /// `Inst::DopeLen`: the element count of dimension `dim` of the descriptor at `dope`.
    #[inline(always)]
    fn dope_len(&self, dope: u64, dim: u8) -> R<u64> {
        let (l, rr, dir) = self.dim_of(dope, u64::from(dim))?;
        yavs_ir::dim_len(l, rr, dir == 1)
            .filter(|&n| n <= i64::MAX as u64)
            .ok_or_else(|| Fault::Runtime(format!("range {l} .. {rr} too long")))
    }

    /// `Inst::SigVal`: the two registers of a reference to elements `o .. o + l` of instance
    /// signal `sig`.
    #[inline(always)]
    fn sig_val(&self, sig: u32, o: i64, l: i64) -> R<(u64, u64)> {
        let (Ok(o), Ok(l)) = (u32::try_from(o), u32::try_from(l)) else {
            return rt_err(format!("signal element range {o} + {l} out of bounds"));
        };
        if self.inst.sigs.get(sig as usize).is_none() {
            return Err(ice(format!("no instance signal {sig}")));
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
    fn sig_read_scalar(&mut self, sig: Reg, ctx: &mut Option<&mut Ctx<'_>>) -> R<u64> {
        // A table of one part (an association of one element) reads like a single range.
        let (sr, ty) = if self.is_multi(sig) {
            match self.sigparts(sig)?.as_slice() {
                [(sr, ty, _)] => (*sr, *ty),
                _ => return Err(ice("scalar read of a reference of several parts")),
            }
        } else {
            self.sigref(sig)?
        };
        Self::read_elem(sr, ty, ctx)
    }

    /// The value of the one element of `sr`.
    #[inline(always)]
    fn read_elem(
        sr: SigRef,
        ty: yavs_kernel::SignalType,
        ctx: &mut Option<&mut Ctx<'_>>,
    ) -> R<u64> {
        let Some(c) = ctx.as_deref_mut() else {
            return rt_err("signal or wait operation during elaboration");
        };
        let v = match ty.elem {
            Elem::Bytes(es) if es <= 8 => cell(c.value(sr)?),
            Elem::Logic => {
                let Some(r) = sr.range.filter(|r| r.len > 0) else {
                    return Err(ice("scalar read of a null signal range"));
                };
                u64::from(c.elem_code(sr.signal, r.offset)?)
            }
            Elem::Bytes(es) => {
                return Err(ice(format!("scalar read of a {es}-byte element")));
            }
        };
        Ok(v)
    }

    /// Element `o` of instance signal `sig`: what `SigVal` of one element makes of its
    /// registers (`sig_val`, `sigref`).
    #[inline(always)]
    fn sig_elem(&self, sig: u32, o: u64) -> R<(SigRef, yavs_kernel::SignalType)> {
        let Ok(o) = u32::try_from(o as i64) else {
            return rt_err(format!(
                "signal element range {} + 1 out of bounds",
                o as i64
            ));
        };
        let s = self
            .inst
            .sigs
            .get(sig as usize)
            .ok_or_else(|| ice(format!("no instance signal {sig}")))?;
        Ok((s.signal.slice(s.off + o, 1), s.ty))
    }

    /// `rising_edge` / `falling_edge` of the first element of `sr` (the native `edge`).
    #[inline(always)]
    fn edge_elem(sr: SigRef, rising: bool, ctx: &mut Option<&mut Ctx<'_>>) -> R<u64> {
        let Some(c) = ctx.as_deref_mut() else {
            return rt_err("signal read during elaboration");
        };
        let Some(hit) = c.edge(sr, rising)? else {
            return Err(ice("edge of a null signal range"));
        };
        Ok(u64::from(hit))
    }

    /// Add `sr` to the sensitivity of the coming wait.
    #[inline(always)]
    fn wait_elem(sr: SigRef, ctx: &mut Option<&mut Ctx<'_>>) -> R<()> {
        let Some(c) = ctx.as_deref_mut() else {
            return rt_err("signal or wait operation during elaboration");
        };
        c.wait_on(sr)?;
        Ok(())
    }

    /// `Inst::WaitFor`.
    fn wait_for(&mut self, t: Reg, ctx: &mut Option<&mut Ctx<'_>>) -> R<()> {
        let t = to_time(self.ri(t))?;
        let Some(c) = ctx.as_deref_mut() else {
            return rt_err("signal or wait operation during elaboration");
        };
        c.wait_for(t)?;
        if self.dbg.is_some() {
            self.waits.timeout = c.after(t).ok();
        }
        Ok(())
    }

    /// `Inst::WaitOn`.
    fn wait_on(&mut self, sig: Reg, ctx: &mut Option<&mut Ctx<'_>>) -> R<()> {
        let Some(c) = ctx.as_deref_mut() else {
            return rt_err("signal or wait operation during elaboration");
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
            c.wait_on(sr)?;
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
            return Err(ice(format!(
                "wait inside {}, which cannot suspend",
                fr.func
            )));
        }
        if ctx.is_none() {
            return rt_err("wait statement executed during elaboration");
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
            return Err(ice("a recorded temporary outside the temporary area"));
        }
        let n = Ty::desc_size(&prog.types, t)
            .ok_or_else(|| ice(format!("no descriptor size of {t}")))?;
        let desc = match desc {
            Some(d) if n > 0 => self.rd(self.r(d), n)?.into(),
            None if n > 0 => {
                return Err(ice(format!(
                    "recorded temporary of {t} without its descriptor"
                )));
            }
            _ => Box::default(),
        };
        self.temp_root(a, Some(t), desc);
        Ok(())
    }

    /// `Inst::NewFrame`.
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
            return Err(Fault::Runtime("object larger than 4 GiB".into()));
        }
        if fill != 0 && !size.is_multiple_of(u64::from(fill)) {
            return Err(ice("NewFrame size is not a multiple of its element"));
        }
        let ds = u64::from(dsize);
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
                let src = src.to_vec();
                let out = self.wr(val, size)?;
                for c in out.chunks_exact_mut(fill as usize) {
                    c.copy_from_slice(&src);
                }
            }
        }
        // The frame's own statements must not free the object.
        let end = u32::try_from(self.temp.len())
            .map_err(|_| Fault::Runtime("temporary memory exceeds 4 GiB".into()))?;
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
        tf.temp_cap = self.temp.capacity() as u64;
        tf.temp_roots = self.troots.len() as u64;
        tf.temp_base = u64::from(self.frames[self.frames.len() - 1].temp_base);
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
            .ok_or_else(|| ice(format!("no native {native}")))?;
        let Some(fun) = self.rt.natives.get(native.0 as usize).copied().flatten() else {
            return rt_err(format!(
                "native subprogram `{}` is not implemented in yavs yet",
                n.symbol
            ));
        };
        if self.pkg_init.is_some() && self.tainted.is_none() && !crate::native::pure(&n.symbol) {
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
            let v = v.ok_or_else(|| ice(format!("native `{}` returned no value", n.symbol)))?;
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
                Some(tier) if rt.native.any.load(Ordering::Acquire) => {
                    let r = self.exec_fast::<true>(rt, Some(&**tier), ctx, base);
                    // Nothing interpreted on this thread until the next run.
                    crate::slot().store(0, Ordering::Relaxed);
                    r
                }
                tier => {
                    let r = self.exec_fast::<false>(rt, tier.map(|t| &**t), ctx, base);
                    if tier.is_some() {
                        crate::slot().store(0, Ordering::Relaxed);
                    }
                    r
                }
            };
        }
        loop {
            // The top frame changes only at calls and returns: look its function up once.
            let fr = self
                .frames
                .last()
                .ok_or_else(|| ice("no frame to execute"))?;
            let f = prog
                .func(fr.func)
                .ok_or_else(|| ice(format!("no function {}", fr.func)))?;
            let pre = rt
                .pre(fr.func, f)
                .ok_or_else(|| ice(format!("no function {}", fr.func)))?;
            let mut pc = fr.pc as usize;
            loop {
                // Skip the definitions the frame started with (`pre.rs`).
                pc = *pre
                    .next
                    .get(pc)
                    .ok_or_else(|| ice(format!("pc {pc} beyond `{}`", f.name)))?
                    as usize;
                self.top().pc = pc as u32;
                let inst = f
                    .code
                    .get(pc)
                    .ok_or_else(|| ice(format!("pc {pc} beyond `{}`", f.name)))?;
                if DEBUG {
                    self.debug_tick(prog, f, pc, inst, ctx)?;
                }
                // The common instructions are executed here; the rest go through `step`.
                macro_rules! jump {
                    ($b:expr) => {{
                        let b = $b;
                        let blk = f
                            .blocks
                            .get(b)
                            .ok_or_else(|| ice(format!("no block {b}")))?;
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
                        let blk = f
                            .blocks
                            .get(b)
                            .ok_or_else(|| ice(format!("no block {b}")))?;
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
                            .ok_or_else(|| ice("return without frame"))?;
                        if self.frames.len() <= base {
                            return Ok(Outcome::Returned(v));
                        }
                        if let Some(d) = done.ret {
                            self.regs[d.0 as usize] =
                                v.ok_or_else(|| ice("function returned no value"))?;
                        }
                        self.top().pc += 1;
                        break;
                    }
                    Flow::Suspend => {
                        self.top().pc += 1;
                        return Ok(Outcome::Suspended);
                    }
                    Flow::Retier => {
                        return Err(ice("a hand-over of the fast loop under a debugger"));
                    }
                }
            }
        }
    }

    /// The dispatch loop on predecoded code (`pre::Code`, #431), without a debugger or a
    /// compiled tier: the top frame's function runs from its ops until a call, a return or a
    /// suspension, which are handled here as in `exec_in`.
    #[inline(always)]
    fn exec_fast<const TIER: bool>(
        &mut self,
        rt: &Runtime,
        tier: Option<&dyn crate::Tier>,
        ctx: &mut Option<&mut Ctx<'_>>,
        base: usize,
    ) -> R<Outcome> {
        let prog = &rt.prog;
        loop {
            let top = self
                .frames
                .len()
                .checked_sub(1)
                .ok_or_else(|| ice("no frame to execute"))?;
            let func = self.frames[top].func;
            let f = prog
                .func(func)
                .ok_or_else(|| ice(format!("no function {func}")))?;
            let pre = rt
                .pre(func, f)
                .ok_or_else(|| ice(format!("no function {func}")))?;
            match self.run_ops::<TIER>(rt, tier, func, f, &pre.code, top, base, ctx)? {
                Flow::Call => {}
                Flow::Ret(v) => {
                    let done = self
                        .pop_frame()
                        .ok_or_else(|| ice("return without frame"))?;
                    if self.frames.len() <= base {
                        return Ok(Outcome::Returned(v));
                    }
                    if let Some(d) = done.ret {
                        self.regs[d.0 as usize] =
                            v.ok_or_else(|| ice("function returned no value"))?;
                    }
                    self.top().pc += 1;
                }
                Flow::Suspend => {
                    self.top().pc += 1;
                    return Ok(Outcome::Suspended);
                }
                Flow::Retier => {
                    if let Some(t) = tier
                        && !TIER
                    {
                        return self.exec_fast::<true>(rt, Some(t), ctx, base);
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
    fn run_ops<const TIER: bool>(
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
                    .ok_or_else(|| ice(format!("pc {pc} of `{}` inside a fused operation", f.name)))
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
        let watch = !TIER && tier.is_some();
        // Whether compiled code can run now (1; 2: the tier is asked to compile the function).
        macro_rules! ready {
            () => {
                match nat.get(func.0 as usize) {
                    Some(n) => {
                        let st = n.load(Ordering::Acquire);
                        if st == 2 {
                            if let Some(t) = tier {
                                n.store(0, Ordering::Relaxed);
                                t.hot(func);
                            }
                        }
                        st == 1
                    }
                    None => false,
                }
            };
        }
        // `TIER`: this thread's sampling slot names the function being run, and whether it runs
        // compiled (`Runtime::sampler`); compiled code entered from compiled code keeps the
        // slot naming its caller, which calls it in place.
        let slot = if TIER || watch {
            Some(crate::slot())
        } else {
            None
        };
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
        if watch && let Some(s) = slot {
            s.store(tag | u64::from(func.0), Ordering::Relaxed);
        }
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
        // `watch`: every `WATCH` jumps, calls and returns, name the function being run for the
        // sampler and look for compiled code to hand over to (the next op is `k` of frame `top`).
        macro_rules! tick {
            () => {
                if watch {
                    recheck -= 1;
                    if recheck == 0 {
                        recheck = WATCH;
                        if let Some(s) = slot {
                            s.store(tag | u64::from(func.0), Ordering::Relaxed);
                        }
                        if rt.native.any.load(Ordering::Acquire) {
                            self.frames[top].pc = code.at.get(k).copied().unwrap_or(0);
                            return Ok(Flow::Retier);
                        }
                    }
                }
            };
        }
        // A jump to op `to`.
        macro_rules! go {
            ($to:expr) => {{
                if TIER && !native {
                    recheck -= 1;
                    if recheck == 0 {
                        recheck = RECHECK;
                        native = ready!();
                    }
                }
                k = $to as usize;
                tick!();
                enter!();
            }};
        }
        // A fault leaves the IR pc of the op that failed in the frame.
        macro_rules! t {
            ($e:expr) => {
                match $e {
                    Ok(v) => v,
                    Err(e) => {
                        self.frames[top].pc = code.at.get(k).copied().unwrap_or(0);
                        return Err(e);
                    }
                }
            };
        }
        // The callers of the frames this loop pushed (innermost last): their function, code
        // and the op after the call, so a return looks nothing up. Compiled code leaves this
        // loop when it keeps frames (`TIER_UNWIND`), so the list stays right.
        let mut saved: [Option<(FuncId, &Func, &pre::Code, u32)>; 8] = [None; 8];
        let mut depth = 0usize;
        macro_rules! save_caller {
            () => {
                if depth < saved.len() {
                    let ret = code
                        .at
                        .get(k)
                        .and_then(|&p| code.entry.get(p as usize + 1))
                        .copied()
                        .unwrap_or(pre::NONE);
                    saved[depth] = Some((func, f, code, ret));
                }
                depth += 1;
            };
        }
        // The eight bytes at `off` of the frame's slots.
        macro_rules! slot8 {
            ($off:expr) => {{
                let off: u32 = $off;
                match self
                    .fslots(top)
                    .and_then(|s| s.get(off as usize..))
                    .and_then(|s| s.first_chunk::<8>())
                {
                    Some(b) => u64::from_le_bytes(*b),
                    None => t!(mk_addr(FRAME, top as u32, off.into())
                        .and_then(|a| self.load(a, Width::B8))),
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
                    && let Some(b) = self
                        .fslots_mut(top)
                        .and_then(|s| s.get_mut(o..))
                        .and_then(|s| s.first_chunk_mut::<8>())
                {
                    *b = v.to_le_bytes();
                } else {
                    let n = w.bytes() as usize;
                    match self.fslots_mut(top).and_then(|s| s.get_mut(o..o + n)) {
                        Some(b) => put_le(b, v),
                        None => t!(mk_addr(FRAME, top as u32, off.into())
                            .and_then(|a| self.store(a, w, v))),
                    }
                }
            }};
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
        enter!();
        loop {
            match ops[k] {
                Op::Step(pc) => {
                    let inst = *f
                        .code
                        .get(pc as usize)
                        .ok_or_else(|| ice(format!("pc {pc} beyond `{}`", f.name)))?;
                    self.frames[top].pc = pc;
                    match self.step_out(prog, f, inst, ctx)? {
                        Flow::Next => {
                            k += 1;
                            enter!();
                        }
                        Flow::Jump(b) => {
                            let blk = f
                                .blocks
                                .get(b)
                                .ok_or_else(|| ice(format!("no block {b}")))?;
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
                Op::Stmt => {
                    self.end_stmt();
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
                    self.regs[dst as usize] = match self.fslots(top).and_then(|s| s.get(o..o + n)) {
                        Some(b) => le_bytes(b),
                        None => t!(
                            mk_addr(FRAME, top as u32, off.into()).and_then(|a| self.load(a, w))
                        ),
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
                    let p = t!(self.pos_of_with(
                        self.regs[dope as usize],
                        dim,
                        self.regs[idx as usize] as i64,
                        || pos_what!()
                    ));
                    self.regs[dst as usize] = t!(index_addr(self.regs[base as usize], p, stride));
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
                    let a = t!(index_addr(self.regs[base as usize], p, stride));
                    self.regs[dst as usize] = t!(self.load(a, w));
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
                    let p = t!(self.pos_of_with(
                        self.regs[dope as usize],
                        dim,
                        self.regs[idx as usize] as i64,
                        || pos_what!()
                    ));
                    let a = t!(index_addr(self.regs[base as usize], p, stride));
                    t!(self.store(a, w, self.regs[src as usize]));
                    k += 1;
                }
                Op::IndexLoad {
                    dst,
                    base,
                    pos,
                    stride,
                    w,
                } => {
                    let a = t!(index_addr(
                        self.regs[base as usize],
                        self.regs[pos as usize],
                        stride
                    ));
                    self.regs[dst as usize] = t!(self.load(a, w));
                    k += 1;
                }
                Op::IndexStore {
                    src,
                    base,
                    pos,
                    stride,
                    w,
                } => {
                    let a = t!(index_addr(
                        self.regs[base as usize],
                        self.regs[pos as usize],
                        stride
                    ));
                    t!(self.store(a, w, self.regs[src as usize]));
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
                        Some(b) => le_bytes(b),
                        None => t!(mk_addr(INST, 0, off.into()).and_then(|a| self.load(a, w))),
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
                    match self.pos_len(self.regs[dope as usize], self.regs[idx as usize] as i64) {
                        Some((p, _)) if aff_ok(p) => {
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
                    let a = t!(self.temp_alloc(self.regs[size as usize]));
                    self.regs[w as usize] = a;
                    t!(self.store(a, Width::B8, self.regs[time as usize]));
                    self.regs[e as usize] = t!(add_addr(a, 8));
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
                    let (x, y) = (self.regs[a as usize] as i64, self.regs[b as usize] as i64);
                    let (l, h) = (self.regs[lo as usize] as i64, self.regs[hi as usize] as i64);
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
                    t!(self.lcopy(
                        self.regs[dst as usize],
                        self.regs[doff as usize],
                        self.regs[src as usize],
                        self.regs[soff as usize],
                        self.regs[n as usize]
                    ));
                    k += 1;
                    enter!();
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
                        return Err(ice(format!("pc {pc} is not a concatenation")));
                    };
                    let ps = t!(f.regs(parts).ok_or_else(|| ice("operand list")));
                    t!(self.cat(
                        self.regs[dst.0 as usize],
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
                            .ok_or_else(|| ice(format!("pc {pc} beyond `{}`", f.name)))?;
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
                } => {
                    self.frames[top].pc = code.at.get(k).copied().unwrap_or(0);
                    let args = yavs_ir::Regs { start, len };
                    let dst = (dst != pre::NONE).then_some(Reg(dst));
                    let cid = FuncId(callee);
                    let cf = prog
                        .func(cid)
                        .ok_or_else(|| ice(format!("call of missing function {cid}")))?;
                    let cpre = rt
                        .pre(cid, cf)
                        .ok_or_else(|| ice(format!("no function {cid}")))?;
                    self.call_pre(f, cf, cid, cpre, args, dst, None)?;
                    save_caller!();
                    // The callee's ops go on in this loop.
                    top += 1;
                    func = cid;
                    f = cf;
                    code = &cpre.code;
                    ops = &code.ops;
                    k = match code.first {
                        pre::NONE => entry!(0)? as usize,
                        first => first as usize,
                    };
                    rebind!();
                    tick!();
                }
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
                        .ok_or_else(|| ice(format!("call of missing function {cid}")))?;
                    let cpre = rt
                        .pre(cid, cf)
                        .ok_or_else(|| ice(format!("no function {cid}")))?;
                    self.call_pre(f, cf, cid, cpre, args, dst, Some(h))?;
                    self.top().shared_alloc |= heap::tag_of(h) == heap::SHARED;
                    save_caller!();
                    top += 1;
                    func = cid;
                    f = cf;
                    code = &cpre.code;
                    ops = &code.ops;
                    k = match code.first {
                        pre::NONE => entry!(0)? as usize,
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
                    let done = self
                        .pop_frame()
                        .ok_or_else(|| ice("return without frame"))?;
                    top -= 1;
                    if let Some(d) = done.ret {
                        self.regs[d.0 as usize] =
                            v.ok_or_else(|| ice("function returned no value"))?;
                    }
                    let caller = &mut self.frames[top];
                    caller.pc += 1;
                    let pc = caller.pc;
                    // The caller this loop entered the callee from: no lookups.
                    if depth > 0 {
                        depth -= 1;
                        if let Some(&Some((sf, sff, sc, rk))) = saved.get(depth)
                            && rk != pre::NONE
                        {
                            (func, f, code, k) = (sf, sff, sc, rk as usize);
                            ops = &code.ops;
                            if TIER {
                                rebind!();
                            }
                            tick!();
                            continue;
                        }
                    }
                    func = caller.func;
                    f = prog
                        .func(func)
                        .ok_or_else(|| ice(format!("no function {func}")))?;
                    code = &rt
                        .pre(func, f)
                        .ok_or_else(|| ice(format!("no function {func}")))?
                        .code;
                    ops = &code.ops;
                    k = entry!(pc)? as usize;
                    rebind!();
                    tick!();
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
                    let mode = if inertial {
                        DriveMode::Inertial
                    } else {
                        DriveMode::Transport
                    };
                    let reject = (reject != pre::NONE).then_some(Reg(reject));
                    let (target, wave, n) = (Reg(target), Reg(wave), Reg(n));
                    if self.is_multi(target) {
                        t!(self.drive_parts(ctx, target, wave, n, mode, reject));
                    } else {
                        let d = t!(self.driver(driver));
                        t!(self.drive(ctx, d, Some(target), wave, n, mode, reject));
                    }
                    k += 1;
                    enter!();
                }
                Op::LSetDrive {
                    v,
                    driver,
                    target,
                    wave,
                    inertial,
                } => {
                    let mode = if inertial {
                        DriveMode::Inertial
                    } else {
                        DriveMode::Transport
                    };
                    let bit = self.regs[v as usize];
                    let (tg, wv) = (Reg(target), Reg(wave));
                    let done = bit <= 8
                        && !self.is_multi(tg)
                        && t!(self
                            .driver(driver)
                            .and_then(|d| self.drive_bit(ctx, d, tg, wv, bit as u8, mode)));
                    if !done {
                        // Not a scalar logic signal (or a bad code): the two instructions.
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
                    let (sr, ty) = t!(self.sig_elem(sig, self.regs[offset as usize]));
                    self.regs[dst as usize] = t!(Self::read_elem(sr, ty, ctx));
                    k += 1;
                    enter!();
                }
                Op::EdgeSig {
                    dst,
                    sig,
                    offset,
                    rising,
                } => {
                    let (sr, _) = t!(self.sig_elem(sig, self.regs[offset as usize]));
                    self.regs[dst as usize] = t!(Self::edge_elem(sr, rising, ctx));
                    k += 1;
                    enter!();
                }
                Op::WaitSig { sig, offset } => {
                    let (sr, _) = t!(self.sig_elem(sig, self.regs[offset as usize]));
                    t!(Self::wait_elem(sr, ctx));
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
                Op::Alloc { dst, size } => {
                    self.regs[dst as usize] = t!(self.temp_alloc(self.regs[size as usize]));
                    k += 1;
                }
                Op::Const { dst, v } => {
                    self.regs[dst as usize] = v as u64;
                    k += 1;
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
                    t!(self.copy_const(prog, self.regs[dst as usize], yavs_ir::ConstId(c)));
                    k += 1;
                }
                Op::MakeDope { at, start, len } => {
                    let parts = t!(f
                        .regs(yavs_ir::Regs { start, len })
                        .ok_or_else(|| ice("operand list")));
                    let a = self.regs[at as usize];
                    t!(self.make_dope(a, parts));
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
                    self.regs[dst as usize] = t!(self.field_addr(
                        self.regs[base as usize],
                        self.regs[layout as usize],
                        n
                    ));
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
            }
        }
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
            Err(_) => return Err(ice("compiled code left a jump or a return to step")),
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
            .ok_or_else(|| ice("no source run for a linear op"))?;
        let mut result = 0;
        for pc in from..=root {
            let inst = *f
                .code
                .get(pc as usize)
                .ok_or_else(|| ice(format!("pc {pc} beyond `{}`", f.name)))?;
            self.frames[top].pc = pc;
            if !matches!(self.step_out(prog, f, inst, ctx)?, Flow::Next) {
                return Err(ice("a linear run left its straight line"));
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
        let (l, rr, dir) = self.dim_of(dope, 0).ok()?;
        Some(yavs_ir::dim_len(l, rr, dir == 1).filter(|&n| n <= i64::MAX as u64)? as i64)
    }

    /// The position of index `i` in the first dimension of the descriptor at `dope` and its
    /// length; `None` when the index is outside the range, the length does not fit 63 bits or
    /// the descriptor cannot be read (the instructions then give the error).
    #[inline(always)]
    fn pos_len(&self, dope: u64, i: i64) -> Option<(i64, i64)> {
        let (l, rr, dir) = self.dim_of(dope, 0).ok()?;
        let n = yavs_ir::dim_len(l, rr, dir == 1).filter(|&n| n <= i64::MAX as u64)? as i64;
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
                .ok_or_else(|| ice(format!("pc {pc} beyond `{}`", f.name)))?;
            self.frames[top].pc = pc as u32;
            match self.step_out(prog, f, inst, ctx)? {
                Flow::Next => pc += 1,
                Flow::Jump(b) => {
                    pc = f
                        .blocks
                        .get(b)
                        .ok_or_else(|| ice(format!("no block {b}")))?
                        .start as usize;
                    jumped = true;
                }
                flow => return Ok(Err(flow)),
            }
        }
        match code.entry.get(pc) {
            Some(&k) if k != pre::NONE => Ok(Ok((k as usize, jumped))),
            _ => Err(ice(format!("pc {pc} of `{}` beyond its code", f.name))),
        }
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

    /// Make `tf` the top frame's view (`TierFrame`).
    fn tier_view(&mut self, tf: &mut crate::TierFrame) {
        let top = self.frames.len() - 1;
        let fr = &self.frames[top];
        let (rbase, slen, temp_base) = (fr.rbase as usize, fr.slen, fr.temp_base);
        tf.slots = self.slots_ptr();
        tf.slots_len = slen as u64;
        tf.frame = top as u64;
        tf.frame_hi = u64::from(FRAME) << 29 | top as u64;
        tf.temp = self.temp.as_mut_ptr();
        tf.temp_len = self.temp.len() as u64;
        tf.temp_cap = self.temp.capacity() as u64;
        tf.temp_base = temp_base as u64;
        tf.temp_roots = self.troots.len() as u64;
        tf.inst = self.inst.data.as_ptr();
        tf.inst_len = self.inst.data.len() as u64;
        tf.nsigs = self.inst.sigs.len() as u64;
        tf.regs = self.regs.v.as_mut_ptr().wrapping_add(rbase);
        tf.nregs = (self.regs.v.len() - rbase) as u64;
        tf.stack = self.regs.v.as_ptr();
        let mut link = self.frames[top].link;
        for u in &mut tf.up {
            *u = match link {
                Some(i) if i < 1 << 29 => {
                    link = self.frames.get(i as usize).and_then(|f| f.link);
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
            .ok_or_else(|| ice(format!("no condition function {c}")))?;
        let link = self.static_link(f, None)?;
        let base = self.frames.len();
        self.push_frame(f, c, link, None)?;
        let r = self.exec::<false>(ctx, base);
        self.truncate_frames(base);
        match r? {
            Outcome::Returned(Some(v)) => Ok(v),
            _ => Err(ice("breakpoint condition did not return a value")),
        }
    }

    fn static_link(&self, callee: &Func, _caller: Option<usize>) -> R<Option<u32>> {
        let Some(parent) = callee.parent else {
            return Ok(None);
        };
        let mut k = self.frames.len().checked_sub(1);
        while let Some(i) = k {
            let fr = &self.frames[i];
            if fr.func == parent {
                return Ok(Some(i as u32));
            }
            // Search the caller's static chain, then outward along the dynamic stack.
            k = match fr.link {
                Some(l) if (l as usize) < i => Some(l as usize),
                _ => i.checked_sub(1),
            };
        }
        Err(ice(format!(
            "frame of {parent}, the parent of `{}`, is not on the stack",
            callee.name
        )))
    }

    /// Call the VHDL body of a native `ieee` subprogram that gave its call back: the symbol
    /// ends in `@<function id>`, array arguments become data and descriptor.
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
            .ok_or_else(|| ice("ieee fallback without a body"))?;
        let func = FuncId(id.parse().map_err(|_| ice("ieee fallback function id"))?);
        let mut out = Vec::with_capacity(cells.len() + 2);
        for (i, &c) in cells.iter().enumerate() {
            if yavs_ir::ieee::array_param(key, i) {
                out.push(self.load(c, Width::B8)?);
                out.push(self.load(add_addr(c, 8)?, Width::B8)?);
            } else {
                out.push(c);
            }
        }
        self.call(prog, func, &out, dst)
    }

    fn call(&mut self, prog: &Program, func: FuncId, cells: &[u64], dst: Option<Reg>) -> R<Flow> {
        let f = prog
            .func(func)
            .ok_or_else(|| ice(format!("call of missing function {func}")))?;
        let link = self.static_link(f, None)?;
        self.push_frame(f, func, link, dst)?;
        let mut k = 0;
        for p in &f.params {
            let n = if p.width == Width::Sig { 2 } else { 1 };
            let c = cells
                .get(k..k + n)
                .ok_or_else(|| ice(format!("missing argument {}", p.name)))?;
            store_cells(self.top_slots(), p.slot, p.width, c)?;
            k += n;
        }
        Ok(Flow::Call)
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
            .ok_or_else(|| ice(format!("call of missing function {func}")))?;
        let pre = rt
            .pre(func, callee)
            .ok_or_else(|| ice(format!("no function {func}")))?;
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
            .ok_or_else(|| ice("operand list out of range"))?;
        let slots = self
            .sstack
            .get_mut(sbase..sbase + slen)
            .ok_or_else(|| ice("frame outside the slot stack"))?;
        let mut params = pre.params.iter();
        // The protected object of a method call is its first parameter.
        if let Some(h) = first
            && let Some(&(slot, w)) = params.next()
        {
            let n = w.bytes() as usize;
            let dst = slots
                .get_mut(slot as usize..slot as usize + n)
                .ok_or_else(|| ice(format!("parameter slot {slot} outside the frame")))?;
            put_le(dst, h);
        }
        for &r in regs {
            let Some(&(slot, w)) = params.next() else {
                break;
            };
            let v = self.regs.v[caller + r.0 as usize];
            let n = w.bytes() as usize;
            let dst = slots
                .get_mut(slot as usize..slot as usize + n)
                .ok_or_else(|| ice(format!("parameter slot {slot} outside the frame")))?;
            if w == Width::Sig {
                dst[..8].copy_from_slice(&v.to_le_bytes());
                dst[8..].copy_from_slice(&self.regs.v[caller + r.0 as usize + 1].to_le_bytes());
            } else {
                put_le(dst, v);
            }
        }
        if params.next().is_some() {
            let name = &callee.params[regs.len() + usize::from(first.is_some())].name;
            return Err(ice(format!("missing argument {name}")));
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
            .ok_or_else(|| ice("operand list out of range"))?;
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
            _ => Err(ice(format!("direction {dir} in a descriptor"))),
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
            Err(arith::ArithError::DivisionByZero { .. }) => rt_err("division by zero"),
            Err(e @ arith::ArithError::NegativeExponent { .. }) => rt_err(e.to_string()),
            _ => {
                let m = self.msg(what);
                let t = if m.subtype.is_empty() {
                    "the result type"
                } else {
                    &m.subtype
                };
                rt_err(format!(
                    "result of {x} {} {y} cannot be represented as {t}",
                    op_symbol(op)
                ))
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
            CheckKind::Length => rt_err(format!("value length {x} does not match {w} length {l}")),
            CheckKind::Null => rt_err(format!("null access value: {w}")),
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
                    None => return rt_err("signal or wait operation during elaboration"),
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
                                .ok_or_else(|| ice("static link missing"))?
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
                    return Err(ice("package allocation outside a package initialisation"));
                };
                let off = (data.len() as u64).next_multiple_of(8);
                let end = off.saturating_add(n);
                if end > u64::from(u32::MAX) {
                    return rt_err("package data exceeds 4 GiB");
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
            Inst::Fill { dst, elem, n, size } => {
                let mut e = self.take_bytes();
                let r = self
                    .rd(self.r(elem), u64::from(size))
                    .map(|s| e.extend_from_slice(s))
                    .and_then(|()| {
                        let total = self
                            .r(n)
                            .checked_mul(u64::from(size))
                            .ok_or_else(|| Fault::Runtime("object larger than 4 GiB".into()))?;
                        let out = self.wr(self.r(dst), total)?;
                        for c in out.chunks_exact_mut(size.max(1) as usize) {
                            c.copy_from_slice(&e);
                        }
                        Ok(())
                    });
                self.give_bytes(e);
                r?;
            }
            Inst::Pos {
                dst,
                dope,
                dim,
                idx,
                what,
            } => self.do_pos(dst, dope, dim, idx, what)?,
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
                            return rt_err(format!("division of {x} by zero"));
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
                let ps = f.regs(parts).ok_or_else(|| ice("operand list"))?;
                self.cat(
                    self.r(dst),
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
                let regs = f.regs(parts).ok_or_else(|| ice("operand list"))?;
                self.make_dope(self.r(at), regs)?;
            }
            Inst::DopeLen { dst, dope, dim } => {
                let n = self.dope_len(self.r(dope), dim)?;
                self.set(dst, n);
            }
            Inst::LayoutOf { dst, ty, desc } => {
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
                    .ok_or_else(|| ice(format!("no case table {table}")))?;
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
                    .ok_or_else(|| ice(format!("indirect call of missing function {id}")))?;
                if c.kind == FuncKind::Process || c.kind == FuncKind::Elab {
                    return Err(ice(format!("indirect call of {:?} {id}", c.kind)));
                }
                return self.call_with(prog, f, id, None, args, dst);
            }
            Inst::CallProt {
                func,
                obj,
                args,
                dst,
            } => {
                self.shared()?;
                let h = self.r(obj);
                let flow = self.call_with(prog, f, func, Some(h), args, dst)?;
                if let Flow::Call = flow {
                    self.top().shared_alloc |= heap::tag_of(h) == heap::SHARED;
                }
                return Ok(flow);
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
                let h = r.map_err(|_| {
                    Fault::Runtime("heap object limit (16M objects per heap) exceeded".into())
                })?;
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
                    return Err(Fault::Runtime("object larger than 4 GiB".into()));
                }
                if fill != 0 && !size.is_multiple_of(u64::from(fill)) {
                    return Err(ice("NewFill size is not a multiple of its element"));
                }
                let (ds, size) = (dsize as usize, size as usize);
                let mut v = Vec::with_capacity(ds + size);
                v.extend_from_slice(self.rd(self.r(desc), u64::from(dsize))?);
                let src = self.rd(
                    self.r(data),
                    if fill == 0 {
                        size as u64
                    } else {
                        u64::from(fill)
                    },
                )?;
                if fill == 0 {
                    v.extend_from_slice(src);
                } else if src.iter().all(|&b| b == 0) {
                    v.resize(ds + size, 0);
                } else if fill == 1 {
                    v.resize(ds + size, src[0]);
                } else {
                    for _ in 0..size / fill as usize {
                        v.extend_from_slice(src);
                    }
                }
                let r = self.alloc_heap(shared)?.alloc(heap::Obj::Typed(ty), v);
                let h = r.map_err(|_| {
                    Fault::Runtime("heap object limit (16M objects per heap) exceeded".into())
                })?;
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
                        return Err(ice("signal reference parts do not tile the object"));
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
                    return rt_err(format!("signal element range {o} + {l} out of bounds"));
                };
                let (lo, hi) = (self.r(sig), self.r(Reg(sig.0 + 1)));
                if lo >> 32 != MULTI {
                    if o + l > hi || (lo as u32 as u64) + o > u64::from(u32::MAX) {
                        return rt_err(format!("signal element range {o} + {l} out of bounds"));
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
                        return rt_err(format!("signal element range {o} + {l} out of bounds"));
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
            Inst::SigLastValue { dst, sig } => {
                let parts = self.sigparts(sig)?;
                let v = self.read_parts(kc!(), &parts, true)?;
                self.put_value(self.r(dst), &v)?;
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
                let v = kc!().driving_value(d.id)?;
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
                    u32::try_from(self.r(slot)).map_err(|_| ice("driver slot out of range"))?;
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
                    return Err(ice("disconnect of a signal reference of several parts"));
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
                let s = sev(self.r(severity))?;
                let text: String = self
                    .rd(self.r(msg), self.r(len))?
                    .iter()
                    .map(|&b| b as char)
                    .collect();
                match ctx.as_deref_mut() {
                    Some(c) => {
                        // A severity disabled by SetVhdlAssertEnable is neither shown nor
                        // counted (as NVC).
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
                            return rt_err(format!("report of severity failure: {text}"));
                        }
                    }
                }
            }
            Inst::Output { addr, len } => {
                let b = self.rd(self.r(addr), self.r(len))?.to_vec();
                kc!().output(&b);
            }
            Inst::Cover { item } => {
                let i = self.inst.cov_base as usize + item.0 as usize;
                self.rt
                    .cov
                    .get(i)
                    .ok_or_else(|| ice(format!("no coverage counter {i}")))?
                    .fetch_add(1, Ordering::Relaxed);
            }
            Inst::Stmt => self.end_stmt(),
            Inst::Elab { op, .. } => {
                return rt_err(format!(
                    "elaboration instruction {op:?} needs yavs-elab (not implemented yet)"
                ));
            }
        }
        Ok(Flow::Next)
    }
}

/// The interpreter under compiled code (`Machine::run_tier`).
struct Host<'a, 'c, 'x> {
    m: &'a mut Machine,
    rt: &'a Runtime,
    prog: &'a Program,
    /// The top frame's function; `up` those of the frames below it that compiled code
    /// pushed through `step`.
    f: &'a Func,
    up: Vec<&'a Func>,
    ctx: &'a mut Option<&'c mut Ctx<'x>>,
    fault: Option<Fault>,
}

impl crate::TierHost for Host<'_, '_, '_> {
    fn view(&mut self, tf: &mut crate::TierFrame) {
        self.m.tier_view(tf);
    }

    fn step(&mut self, tf: &mut crate::TierFrame, at: u32) -> crate::Stepped {
        use crate::Stepped;
        let (m, f) = (&mut *self.m, self.f);
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
            Some(&inst) => m.step(self.prog, f, inst, self.ctx),
            None => Err(ice(format!(
                "compiled code stepped pc {at} beyond `{}`",
                f.name
            ))),
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
                        self.fault = Some(ice(format!("no function {func}")));
                        Stepped::Fault
                    }
                }
            }
            Ok(Flow::Suspend) => Stepped::Suspend,
            Ok(_) => {
                self.fault = Some(ice(format!(
                    "compiled code stepped a control instruction at pc {at} of `{}`",
                    f.name
                )));
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

    fn signal(&mut self, tf: &mut crate::TierFrame, at: u32, edge: u32) -> crate::Stepped {
        let (m, f) = (&mut *self.m, self.f);
        let r = match f.code.get(at as usize) {
            Some(&Inst::SigReadScalar { dst, sig }) => {
                m.sig_read_scalar(sig, self.ctx).map(|v| m.set(dst, v))
            }
            Some(&Inst::SigRead { dst, sig }) if !m.is_multi(sig) => {
                m.sig_read_vec(dst, sig, self.ctx)
            }
            Some(&Inst::Drive {
                driver,
                target,
                wave,
                n,
                mode,
                reject,
            }) if !target.is_some_and(|t| m.is_multi(t)) => m
                .driver(driver)
                .and_then(|d| m.drive(self.ctx, d, target, wave, n, mode, reject)),
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
                .map_err(|_| ice("driver slot out of range"))
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
                    m.cat(d, n, mm, ps, scalars, bias, logic, w)
                }
                None => Err(ice("operand list")),
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
fn cell_w(b: &[u8], w: Width) -> u64 {
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
        return Err(ice("too few argument cells"));
    }
    let dst = slots
        .get_mut(slot as usize..slot as usize + n)
        .ok_or_else(|| ice(format!("parameter slot {slot} outside the frame")))?;
    for (d, c) in dst.chunks_mut(8).zip(cells) {
        d.copy_from_slice(&c.to_le_bytes()[..d.len()]);
    }
    Ok(())
}

/// The size of a `Repeat` or `Runs` constant once expanded.
fn compact_len(c: &ConstData) -> R<u64> {
    let too_big = || Fault::Runtime("constant larger than 4 GiB".into());
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
    let too_big = || Fault::Runtime("constant larger than 4 GiB".into());
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
                    return Err(ice(format!("std_ulogic code {code} in a constant")));
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
}
