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

pub(crate) struct Frame {
    pub func: FuncId,
    pub pc: u32,
    pub slots: Vec<u8>,
    pub regs: Vec<u64>,
    /// Static link: index of the lexically enclosing frame.
    pub link: Option<u32>,
    /// Caller register receiving the result.
    pub ret: Option<Reg>,
    /// Temporary-area top when the frame was entered.
    pub temp_base: usize,
    /// Allocators allocate in the shared heap (`Machine::alloc_heap`): inside a method of a
    /// protected object in the shared heap (#184), in elaboration code run with the shared
    /// state, whose private heap dies with the call (#246), and in everything they call.
    pub shared_alloc: bool,
}

enum Flow {
    Next,
    Jump(yavs_ir::BlockId),
    /// The callee's frame was pushed.
    Call,
    Ret(Option<u64>),
    Suspend,
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

pub(crate) struct Machine {
    pub rt: Arc<Runtime>,
    /// A second handle on `rt` that `exec` takes while it runs, so an activation does not
    /// touch the shared reference count (every process of a batch has the same runtime).
    rt_run: Option<Arc<Runtime>>,
    pub inst: Arc<InstData>,
    pub drivers: Vec<ProcDriver>,
    pub frames: Vec<Frame>,
    /// Registers of the top frame, kept out of it for direct access (`push_frame`,
    /// `pop_frame`); the top frame's own `regs` is empty meanwhile.
    regs: Vec<u64>,
    pub heap: Heap,
    pub temp: Vec<u8>,
    pub sh: Option<SharedState>,
    /// The package being initialised (`init_package`): its index and writable data.
    pub pkg_init: Option<(u32, Vec<u8>)>,
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
    /// Slot and register vectors of returned frames, reused by the next calls.
    spare: Vec<(Vec<u8>, Vec<u64>)>,
    /// Argument cells of the call being made (`call_with`).
    cells: Vec<u64>,
    /// The staging buffer of `Copy` and `Fill` (`take_bytes`), reused.
    bytes: Vec<u8>,
    /// `regs`, `temp` and `spare` went to the thread's scratch at the last suspension
    /// ([`Machine::release_scratch`]) and are empty until [`Machine::borrow_scratch`].
    released: bool,
}

/// The working vectors of the process running on this thread: registers, temporaries and
/// spare frame vectors. A suspended process owns none of them (no register or temporary is
/// live across a suspension, ir.md 1.2), so a design of thousands of processes keeps one set
/// per thread instead of a set each (#409).
#[derive(Default)]
struct Scratch {
    regs: Vec<u64>,
    temp: Vec<u8>,
    spare: Vec<(Vec<u8>, Vec<u64>)>,
}

thread_local! {
    static SCRATCH: std::cell::RefCell<Scratch> = std::cell::RefCell::new(Scratch::default());
}

/// Returned frames whose vectors `Machine::spare` keeps.
const SPARE_FRAMES: usize = 16;

struct LayoutMemo {
    dsz: u64,
    desc: Vec<u8>,
    words: Vec<u8>,
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

fn get_bits(words: &[[u64; 4]], start: u64, m: u64, p: usize) -> u64 {
    let (w, s) = ((start / 64) as usize, start % 64);
    let mut v = words.get(w).map_or(0, |b| b[p]) >> s;
    if s > 0 {
        v |= words.get(w + 1).map_or(0, |b| b[p]) << (64 - s);
    }
    if m < 64 { v & ((1 << m) - 1) } else { v }
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
            regs: Vec::new(),
            heap: Heap::new(heap::PRIVATE),
            temp: Vec::new(),
            sh: None,
            pkg_init: None,
            entry,
            dbg: None,
            waits: Waits::default(),
            pause: None,
            ieee_stop: false,
            ieee_fallback: false,
            layouts: Vec::new(),
            spare: Vec::new(),
            cells: Vec::new(),
            bytes: Vec::new(),
            released: false,
        }
    }

    /// Before a process runs again: take this thread's scratch vectors back.
    pub(crate) fn borrow_scratch(&mut self) -> R<()> {
        if !self.released {
            return Ok(());
        }
        self.released = false;
        SCRATCH.with(|s| {
            let mut s = s.borrow_mut();
            std::mem::swap(&mut self.regs, &mut s.regs);
            std::mem::swap(&mut self.temp, &mut s.temp);
            std::mem::swap(&mut self.spare, &mut s.spare);
        });
        // Registers are written before they are read: stale values are harmless, except
        // those a frame starts with (`pre.rs`).
        let rt = &self.rt;
        let root = self.frames.first().map(|fr| fr.func);
        match root.and_then(|id| Some((id, rt.prog.func(id)?))) {
            Some((id, f)) => {
                let pre = rt
                    .pre(id, f)
                    .ok_or_else(|| ice(format!("no function {id}")))?;
                pre.fill(&mut self.regs, 0)?;
            }
            None => self.regs.clear(),
        }
        self.temp.clear();
        Ok(())
    }

    /// At a suspension in the root frame: hand the working vectors to this thread's scratch.
    /// Deeper frames keep theirs (their registers and temporaries are not touched, so a
    /// procedure that waits resumes as before).
    pub(crate) fn release_scratch(&mut self) {
        if self.frames.len() != 1 || self.released || self.dbg.is_some() {
            return;
        }
        self.released = true;
        SCRATCH.with(|s| {
            let mut s = s.borrow_mut();
            s.regs = std::mem::take(&mut self.regs);
            s.temp = std::mem::take(&mut self.temp);
            s.spare = std::mem::take(&mut self.spare);
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
        let mut fr = self.frame_for(f, func, None)?;
        let mut k = 0;
        for p in &f.params {
            let n = if p.class == ParamClass::Signal { 2 } else { 1 };
            let cells = args
                .get(k..k + n)
                .ok_or_else(|| Fault::Runtime(format!("missing argument {}", p.name)))?;
            store_cells(&mut fr.slots, p.slot, p.width, cells)?;
            k += n;
        }
        self.push_frame(fr);
        Ok(())
    }

    /// Push `fr` as the new top frame, moving its registers to `Machine::regs`.
    pub(crate) fn push_frame(&mut self, mut fr: Frame) {
        let regs = std::mem::take(&mut fr.regs);
        if let Some(top) = self.frames.last_mut() {
            top.regs = std::mem::replace(&mut self.regs, regs);
        } else {
            self.regs = regs;
        }
        self.frames.push(fr);
    }

    /// Pop the top frame with its registers; the caller's registers become current.
    pub(crate) fn pop_frame(&mut self) -> Option<Frame> {
        let mut fr = self.frames.pop()?;
        let below = self
            .frames
            .last_mut()
            .map(|top| std::mem::take(&mut top.regs))
            .unwrap_or_default();
        fr.regs = std::mem::replace(&mut self.regs, below);
        Some(fr)
    }

    /// Pop frames down to `n`.
    pub(crate) fn truncate_frames(&mut self, n: usize) {
        while self.frames.len() > n {
            self.pop_frame();
        }
    }

    fn frame_for(&mut self, f: &Func, func: FuncId, link: Option<u32>) -> R<Frame> {
        if self.frames.len() >= self.rt.limits.max_depth as usize {
            return rt_err(format!(
                "call depth limit {} exceeded calling `{}` (unbounded recursion?)",
                self.rt.limits.max_depth, f.name
            ));
        }
        let (mut slots, mut regs) = self.spare.pop().unwrap_or_default();
        slots.clear();
        slots.resize(f.frame as usize, 0);
        let pre = self
            .rt
            .pre(func, f)
            .ok_or_else(|| ice(format!("no function {func}")))?;
        pre.fill(&mut regs, self.frames.len() as u32)?;
        Ok(Frame {
            func,
            pc: 0,
            slots,
            regs,
            link,
            ret: None,
            temp_base: self.temp.len(),
            // A root frame entered with the shared state attached is elaboration code
            // (`call_shared`, `init_package`, `call_values_shared`); a process enters its
            // root frame before it first takes the shared state, and allocates in the shared
            // heap when it is serial: what it allocates may end up in a shared variable (#303).
            shared_alloc: self.frames.last().map_or_else(
                || self.sh.is_some() || f.kind == FuncKind::Process && self.rt.prog.is_serial(func),
                |f| f.shared_alloc,
            ),
        })
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

    /// GC of the private heap and the debug process table, after a suspension.
    pub(crate) fn after_suspend(&mut self) {
        if self.heap.since_gc > self.rt.limits.gc_threshold {
            self.collect();
        } else if self.heap.has_quarantine() {
            let pinned = self.pinned(HEAP, false);
            self.heap.recycle(&pinned);
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

    /// Handles in the frames' root maps (registers are dead at a suspension point, ir.md
    /// 1.2); `None` when one cannot be located.
    fn frame_roots(&self) -> Option<Vec<u64>> {
        let prog = &self.rt.prog;
        let mut roots = Vec::new();
        for fr in &self.frames {
            let f = prog.func(fr.func)?;
            if !heap::slot_roots(prog, &f.roots, &fr.slots, &mut roots) {
                return None;
            }
        }
        Some(roots)
    }

    /// Collect the private heap from the frames' root maps.
    pub(crate) fn collect(&mut self) -> bool {
        let Some(roots) = self.frame_roots() else {
            return false;
        };
        let pinned = self.pinned(HEAP, false);
        self.heap.collect(&self.rt.prog, &roots, &pinned)
    }

    /// Slot indices of `space` (a heap) that addresses in this process's frames and
    /// temporaries, and with `regs` its registers, may reach, sorted. Any aligned word that
    /// decodes to such an address counts: a false hit only delays reuse or collection of one
    /// object. Registers are dead at a suspension, live between instructions.
    pub(crate) fn pinned(&self, space: u8, regs: bool) -> Vec<u32> {
        let words = self
            .frames
            .iter()
            .flat_map(|f| f.slots.as_chunks::<8>().0)
            .chain(self.temp.as_chunks::<8>().0)
            .map(|w| u64::from_le_bytes(*w))
            .chain(
                self.frames
                    .iter()
                    .filter(|_| regs)
                    .flat_map(|f| f.regs.iter().copied())
                    .chain(self.regs.iter().copied().filter(|_| regs)),
            );
        let mut v: Vec<u32> = words
            .map(split)
            .filter(|&(sp, _, _)| sp == space)
            .map(|(_, i, _)| i)
            .collect();
        v.sort_unstable();
        v.dedup();
        v
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
        let handles = self.frame_roots().map(|mut v| {
            v.retain(|&h| heap::tag_of(h) == heap::SHARED);
            v
        });
        let pinned = self.pinned(SHARED_HEAP, false);
        let Some(sh) = self.sh.as_mut() else {
            return;
        };
        let i = *slot.get_or_insert_with(|| {
            sh.procs.push(ProcRoots::default());
            sh.procs.len() - 1
        });
        sh.procs[i] = ProcRoots { handles, pinned };
        if sh.heap.as_deref().is_some_and(Heap::has_quarantine) {
            let pinned = sh.pinned();
            if let Some(h) = sh.heap.as_mut() {
                h.recycle(&pinned);
            }
        }
    }

    /// Recycle deallocated slots inside an activation once enough wait (#355), so a loop
    /// without a wait does not keep every slot it frees. Call only between instructions
    /// (before a native call or a `deallocate`): every address the process holds is then in
    /// its frames, registers or temporaries. Other serial processes are suspended, so their
    /// published addresses are current for the shared heap.
    fn reclaim(&mut self) {
        self.gc_inline();
        if self.heap.reclaim_due() {
            let pinned = self.pinned(HEAP, true);
            self.heap.recycle(&pinned);
        }
        let due = self
            .sh
            .as_ref()
            .and_then(|sh| sh.heap.as_deref())
            .is_some_and(Heap::reclaim_due);
        if due {
            let mut pinned = self.pinned(SHARED_HEAP, true);
            if let Some(sh) = self.sh.as_mut() {
                pinned.extend(sh.pinned());
                pinned.sort_unstable();
                pinned.dedup();
                if let Some(h) = sh.heap.as_mut() {
                    h.recycle(&pinned);
                }
            }
        }
    }

    /// Every aligned word of the frames, registers and temporaries that has the tag of an
    /// access handle: roots for a collection inside an activation, where a handle may sit in a
    /// register or a temporary that no root map names. A false hit only keeps an object.
    fn handle_words(&self) -> Vec<u64> {
        let tags = [heap::PRIVATE, heap::SHARED];
        let words = self
            .frames
            .iter()
            .flat_map(|f| f.slots.as_chunks::<8>().0)
            .chain(self.temp.as_chunks::<8>().0)
            .map(|w| u64::from_le_bytes(*w))
            .chain(
                self.frames
                    .iter()
                    .flat_map(|f| f.regs.iter().copied())
                    .chain(self.regs.iter().copied()),
            );
        words
            .filter(|&w| w != 0 && tags.contains(&heap::tag_of(w)))
            .collect()
    }

    /// Collect a heap that grew past its threshold inside an activation, so a process that
    /// never waits (a testbench main loop) does not keep every object it drops until the next
    /// delta cycle (memory grew without bound). Call only between instructions, as
    /// [`Machine::reclaim`]. Other serial processes are suspended, so what they published is
    /// current. Nothing is freed when a root cannot be located.
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
        let Some(mut roots) = self.frame_roots() else {
            return;
        };
        roots.extend(self.handle_words());
        if private {
            let pinned = self.pinned(HEAP, true);
            self.heap.collect(&self.rt.prog, &roots, &pinned);
        }
        if shared {
            let mut pinned = self.pinned(SHARED_HEAP, true);
            let prog = self.rt.prog.clone();
            let Some(sh) = self.sh.as_mut() else { return };
            if !heap::slot_roots(&prog, &prog.shared_roots, &sh.vars, &mut roots) {
                return;
            }
            for p in &sh.procs {
                match &p.handles {
                    Some(h) => roots.extend(h),
                    None => return,
                }
            }
            pinned.extend(sh.pinned());
            pinned.sort_unstable();
            pinned.dedup();
            if let Some(h) = sh.heap.as_mut() {
                h.collect(&prog, &roots, &pinned);
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
            FRAME => {
                &self
                    .frames
                    .get(idx as usize)
                    .ok_or_else(|| ice(format!("address of dead frame {idx}")))?
                    .slots
            }
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
            FRAME => {
                &mut self
                    .frames
                    .get_mut(idx as usize)
                    .ok_or_else(|| ice(format!("address of dead frame {idx}")))?
                    .slots
            }
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

    pub(crate) fn put(&mut self, a: u64, bytes: &[u8]) -> R<()> {
        self.wr(a, bytes.len() as u64)?.copy_from_slice(bytes);
        Ok(())
    }

    #[inline]
    pub(crate) fn load(&self, a: u64, w: Width) -> R<u64> {
        let n = w.bytes().min(8) as u64;
        // Fast path: a frame (registers spill to frame slots all the time), temporary or
        // instance memory.
        let (sp, idx, off) = split(a);
        let mem: Option<&[u8]> = match sp {
            FRAME => self.frames.get(idx as usize).map(|f| &f.slots[..]),
            TEMP => Some(&self.temp),
            INST => Some(&self.inst.data),
            _ => None,
        };
        if let Some(b) = mem.and_then(|m| m.get(off as usize..(off + n) as usize)) {
            return Ok(cell_w(b, w));
        }
        Ok(cell_w(self.rd(a, n)?, w))
    }

    /// Bounds and direction word of dimension `dim` of the descriptor at `base`.
    #[inline(always)]
    fn dim_of(&self, base: u64, dim: u64) -> R<(i64, i64, u64)> {
        let d = add_addr(base, dim * 24)?;
        let b = self.rd(d, 24)?;
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
        if sp == FRAME
            && let Some(b) = self
                .frames
                .get_mut(idx as usize)
                .and_then(|f| f.slots.get_mut(off as usize..off as usize + n))
        {
            put_le(b, v);
            return Ok(());
        }
        let mut x = [0u8; 8];
        put_le(&mut x[..n], v);
        self.put(a, &x[..n])
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
        let (sb0, sb1) = (soff / 64, (soff + n - 1) / 64);
        let (db0, db1) = (doff / 64, (doff + n - 1) / 64);
        let sw = blocks_of(self.rd(add_addr(src, sb0 * 32)?, (sb1 - sb0 + 1) * 32)?);
        let da = add_addr(dst, db0 * 32)?;
        let mut dw = blocks_of(self.rd(da, (db1 - db0 + 1) * 32)?);
        let (s0, d0) = (soff - sb0 * 64, doff - db0 * 64);
        let mut k = 0;
        while k < n {
            let m = (n - k).min(64);
            for p in 0..4 {
                let v = get_bits(&sw, s0 + k, m, p);
                set_bits(&mut dw, d0 + k, m, p, v);
            }
            k += m;
        }
        self.put(da, &bytes_of(&dw))
    }

    /// `Drive` and `DriveSlot`: the waveform at `wave` onto driver `d`.
    #[allow(clippy::too_many_arguments)]
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
        let mut items = Vec::new();
        for k in 0..self.r(n) {
            let at = add_addr(w, k * stride)?;
            let t = to_time(self.load(at, Width::B8)? as i64)?;
            items.push((t, self.get_value(add_addr(at, 8)?, elem, len)?));
        }
        let refs: Vec<(Time, ValueRef<'_>)> = items.iter().map(|(t, v)| (*t, v.as_ref())).collect();
        let mode = match mode {
            DriveMode::Transport => DelayMode::Transport,
            DriveMode::Inertial => DelayMode::Inertial {
                reject: match reject {
                    Some(r) => Some(to_time(self.ri(r))?),
                    None => None,
                },
            },
        };
        let range = target.map(|_| sr.range).unwrap_or(None);
        let Some(c) = ctx.as_deref_mut() else {
            return rt_err("signal or wait operation during elaboration");
        };
        c.assign(d.id, range, &refs, mode)?;
        Ok(())
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
        if memo.as_ref().is_none_or(|m| m.desc != d) {
            let (l, _) = Ty::layout(&prog.types, &prog.consts, ty, d).map_err(|e| match e {
                LayoutError::TooLarge => Fault::Runtime(e.to_string()),
                LayoutError::Malformed(_) => ice(e.to_string()),
            })?;
            let mut words = vec![l.size, l.align];
            if l.offsets.is_empty() {
                words.extend([l.stride, l.count]);
            } else {
                words.extend(&l.offsets);
            }
            memo = Some(LayoutMemo {
                dsz,
                desc: d.to_vec(),
                words: words.iter().flat_map(|w| w.to_le_bytes()).collect(),
            });
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
    fn take_bytes(&mut self) -> Vec<u8> {
        let mut b = std::mem::take(&mut self.bytes);
        b.clear();
        b
    }

    /// Keep a staging buffer for the next `Copy` or `Fill`, unless it grew large.
    fn give_bytes(&mut self, b: Vec<u8>) {
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
        let Some(c) = ctx.as_deref_mut() else {
            return rt_err("signal or wait operation during elaboration");
        };
        let v = match ty.elem {
            Elem::Bytes(es) if es <= 8 => cell(c.value(sr)?),
            Elem::Logic => {
                let Some(r) = sr.range.filter(|r| r.len > 0) else {
                    return Err(ice("scalar read of a null signal range"));
                };
                // The first element in one block: its code is bit 0 of each plane.
                let mut b = [0u8; 32];
                c.read_into(sr.signal.slice(r.offset, 1), &mut b)?;
                (0..4).fold(0, |v, p| v | u64::from(b[p * 8] & 1) << p)
            }
            Elem::Bytes(es) => {
                return Err(ice(format!("scalar read of a {es}-byte element")));
            }
        };
        Ok(v)
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
    fn suspend(&mut self, prog: &Program, f: &Func, ctx: &Option<&mut Ctx<'_>>) -> R<()> {
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
        if self.dbg.is_some() {
            let fr = &self.frames[self.frames.len() - 1];
            self.waits.stmt = f.loc.get(fr.pc as usize).copied();
        }
        Ok(())
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
        self.reclaim();
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
        let base = self.top().temp_base;
        self.temp.truncate(base);
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
                Some(tier) => self.exec_fast::<true>(rt, Some(&**tier), ctx, base),
                None => self.exec_fast::<false>(rt, None, ctx, base),
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
                        if self.spare.len() < SPARE_FRAMES {
                            self.spare.push((done.slots, done.regs));
                        }
                        break;
                    }
                    Flow::Suspend => {
                        self.top().pc += 1;
                        return Ok(Outcome::Suspended);
                    }
                }
            }
        }
    }

    /// The dispatch loop on predecoded code (`pre::Code`, #431), without a debugger or a
    /// compiled tier: the top frame's function runs from its ops until a call, a return or a
    /// suspension, which are handled here as in `exec_in`.
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
            match self.run_ops::<TIER>(rt, tier, func, f, &pre.code, top, ctx)? {
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
                    if self.spare.len() < SPARE_FRAMES {
                        self.spare.push((done.slots, done.regs));
                    }
                }
                Flow::Suspend => {
                    self.top().pc += 1;
                    return Ok(Outcome::Suspended);
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
    /// With `TIER` (a compiled tier is set, `--tier=llvm|auto`): the function's tier-up count
    /// grows by the ops run between jumps and shrinks at every op compiled code would leave
    /// to the interpreter (`Runtime::set_heat`); on entry and after every jump, if the
    /// function has compiled code, that runs from the op's IR pc, and the interpreter goes
    /// on at the op of the IR pc it returns (stepping IR instructions inside a fused op).
    #[allow(clippy::too_many_arguments)]
    fn run_ops<const TIER: bool>(
        &mut self,
        rt: &Runtime,
        tier: Option<&dyn crate::Tier>,
        func: FuncId,
        f: &Func,
        code: &pre::Code,
        top: usize,
        ctx: &mut Option<&mut Ctx<'_>>,
    ) -> R<Flow> {
        let ops = &code.ops[..];
        let entry = |pc: u32| {
            code.entry
                .get(pc as usize)
                .copied()
                .filter(|&k| k != pre::NONE)
                .ok_or_else(|| ice(format!("pc {pc} of `{}` inside a fused operation", f.name)))
        };
        let mut k = entry(self.frames[top].pc)? as usize;
        let prog = &*rt.prog;
        let count = if TIER { rt.count(func) } else { None };
        let cool = if TIER { rt.cool() } else { 0 };
        // Whether compiled code can run now.
        let ready = || tier.is_some_and(|t| t.ready(func));
        // What the count still lacks (at most 0: reached, or no counting, `--tier=llvm`) and
        // what this run added since it was last written (kept local: the atomic counter is
        // touched only when the threshold may be reached and when the run ends).
        let mut left = count.map_or(0, Runtime::left);
        let mut spent: i64 = 0;
        let mut native = TIER && left <= 0 && ready();
        // The op the run since the last jump started at.
        let mut since = k;
        // Run compiled code from op `k` when the function has some (`native`): at the start,
        // after a jump and after an op compiled code leaves to the interpreter.
        macro_rules! enter {
            () => {
                if TIER
                    && native
                    && let Some(tier) = tier
                {
                    let pc = code.at.get(k).copied().unwrap_or(0) as usize;
                    let (mut next, ran) = self.run_tier(tier, prog, f, pc, ctx)?;
                    // No progress: `pc` is no entry of the compiled code; interpret to the next jump.
                    native = ran && next != pc;
                    // An IR pc inside a fused op: step its instructions up to the next op.
                    while code.entry.get(next).is_some_and(|&o| o == pre::NONE) {
                        let inst = *f
                            .code
                            .get(next)
                            .ok_or_else(|| ice(format!("pc {next} beyond `{}`", f.name)))?;
                        self.frames[top].pc = next as u32;
                        match self.step_out(prog, f, inst, ctx)? {
                            Flow::Next => next += 1,
                            Flow::Jump(b) => {
                                next = f
                                    .blocks
                                    .get(b)
                                    .ok_or_else(|| ice(format!("no block {b}")))?
                                    .start as usize;
                                native = ready();
                            }
                            flow => return Ok(flow),
                        }
                    }
                    k = entry(next as u32)? as usize;
                    since = k;
                }
            };
        }
        // A jump to op `to`.
        macro_rules! go {
            ($to:expr) => {{
                let to = $to as usize;
                if TIER {
                    if left > 0 {
                        spent += (k + 1).saturating_sub(since) as i64;
                        if spent >= left
                            && let Some(c) = count
                        {
                            left = rt.heat(func, c, spent);
                            spent = 0;
                            native = left <= 0 && ready();
                        }
                    } else {
                        native = ready();
                    }
                    since = to;
                }
                k = to;
                enter!();
            }};
        }
        // An op compiled code leaves to the interpreter.
        macro_rules! slow {
            () => {
                if TIER && left > 0 {
                    spent -= cool;
                }
            };
        }
        // The end of the run.
        macro_rules! leave {
            () => {
                if TIER
                    && left > 0
                    && let Some(c) = count
                {
                    spent += (k + 1).saturating_sub(since) as i64;
                    rt.heat(func, c, spent);
                }
            };
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
        enter!();
        loop {
            match ops[k] {
                Op::Step(pc) => {
                    slow!();
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
                            go!(entry(blk.start)?);
                        }
                        flow => {
                            leave!();
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
                    self.regs[dst as usize] = match self.frames[top].slots.get(o..o + n) {
                        Some(b) => le_bytes(b),
                        None => t!(
                            mk_addr(FRAME, top as u32, off.into()).and_then(|a| self.load(a, w))
                        ),
                    };
                    k += 1;
                }
                Op::StoreSlot { src, off, w } => {
                    let n = w.bytes() as usize;
                    let o = off as usize;
                    let v = self.regs[src as usize];
                    match self.frames[top].slots.get_mut(o..o + n) {
                        Some(b) => put_le(b, v),
                        None => t!(mk_addr(FRAME, top as u32, off.into())
                            .and_then(|a| self.store(a, w, v))),
                    }
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
                Op::Cmp { op, dst, a, b } => {
                    let v = cmp(
                        op,
                        (self.regs[a as usize] as i64).cmp(&(self.regs[b as usize] as i64)),
                    );
                    self.regs[dst as usize] = v.into();
                    k += 1;
                }
                Op::CmpBr {
                    op,
                    dst,
                    a,
                    b,
                    t,
                    f: e,
                } => {
                    let v = cmp(
                        op,
                        (self.regs[a as usize] as i64).cmp(&(self.regs[b as usize] as i64)),
                    );
                    self.regs[dst as usize] = v.into();
                    go!(if v { t } else { e });
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
                    slow!();
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
                    func,
                    start,
                    len,
                    dst,
                } => {
                    slow!();
                    leave!();
                    self.frames[top].pc = code.at.get(k).copied().unwrap_or(0);
                    let args = yavs_ir::Regs { start, len };
                    let dst = (dst != pre::NONE).then_some(Reg(dst));
                    return self.call_with(prog, f, FuncId(func), None, args, dst);
                }
                Op::Ret { v } => {
                    slow!();
                    leave!();
                    let v = (v != pre::NONE).then(|| self.regs[v as usize]);
                    return Ok(Flow::Ret(v));
                }
                Op::SigReadScalar { dst, sig } => {
                    slow!();
                    self.regs[dst as usize] = t!(self.sig_read_scalar(Reg(sig), ctx));
                    k += 1;
                    enter!();
                }
                Op::WaitOn { sig } => {
                    slow!();
                    t!(self.wait_on(Reg(sig), ctx));
                    k += 1;
                    enter!();
                }
                Op::Suspend => {
                    slow!();
                    leave!();
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
                    slow!();
                    self.frames[top].pc = code.at.get(k).copied().unwrap_or(0);
                    let args = yavs_ir::Regs { start, len };
                    let dst = (dst != pre::NONE).then_some(Reg(dst));
                    match self.call_native(prog, f, yavs_ir::NativeId(native), args, dst, ctx)? {
                        Flow::Next => {
                            k += 1;
                            enter!();
                        }
                        flow => {
                            leave!();
                            return Ok(flow);
                        }
                    }
                }
                Op::Alloc { dst, size } => {
                    self.regs[dst as usize] = t!(self.temp_alloc(self.regs[size as usize]));
                    k += 1;
                }
                Op::CondBr { c, t, f: e } => {
                    go!(if self.regs[c as usize] != 0 { t } else { e });
                }
            }
        }
    }

    /// Run compiled code of the top frame from `pc` (ir.md 5): the index of the instruction
    /// the interpreter executes next, and whether the function has compiled code. Compiled
    /// code steps other instructions through `step`, which never changes the frame stack.
    fn run_tier(
        &mut self,
        tier: &dyn crate::Tier,
        prog: &Program,
        f: &Func,
        pc: usize,
        ctx: &mut Option<&mut Ctx<'_>>,
    ) -> R<(usize, bool)> {
        let top = self.frames.len() - 1;
        let fr = &mut self.frames[top];
        let func = fr.func;
        let mut tf = crate::TierFrame {
            regs: std::mem::take(&mut self.regs),
            slots: std::mem::take(&mut fr.slots),
            frame: top as u32,
            temp: std::mem::take(&mut self.temp),
            temp_base: fr.temp_base,
            inst: &self.inst.data[..] as *const [u8],
        };
        let mut host = Host {
            m: self,
            prog,
            f,
            ctx,
            fault: None,
        };
        let ran = tier.run(func, pc as u32, &mut tf, &mut host);
        let fault = host.fault;
        self.swap_tier(&mut tf);
        let pc = ran.map_or(pc, |p| p as usize);
        self.top().pc = pc as u32;
        match fault {
            Some(e) => Err(e),
            None => Ok((pc, ran.is_some())),
        }
    }

    /// Exchange the top frame's registers and slots and the temporary area with `tf`.
    fn swap_tier(&mut self, tf: &mut crate::TierFrame) {
        std::mem::swap(&mut self.regs, &mut tf.regs);
        std::mem::swap(&mut self.top().slots, &mut tf.slots);
        std::mem::swap(&mut self.temp, &mut tf.temp);
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
        let fr = self.frame_for(f, c, link)?;
        let base = self.frames.len();
        self.push_frame(fr);
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
        let mut fr = self.frame_for(f, func, link)?;
        fr.ret = dst;
        let mut k = 0;
        for p in &f.params {
            let n = if p.width == Width::Sig { 2 } else { 1 };
            let c = cells
                .get(k..k + n)
                .ok_or_else(|| ice(format!("missing argument {}", p.name)))?;
            store_cells(&mut fr.slots, p.slot, p.width, c)?;
            k += n;
        }
        self.push_frame(fr);
        Ok(Flow::Call)
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
        let (l, rr, dir) = self.dim_of(dope, u64::from(dim))?;
        let pos = match dir {
            // The distance fits a u64 even when it does not fit an i64.
            0 if l <= i && i <= rr => i.wrapping_sub(l),
            1 if rr <= i && i <= l => l.wrapping_sub(i),
            _ => return self.pos_fail(l, rr, dir, i, what).map(|()| 0),
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
            Inst::CopyConst { dst, c } => {
                let rt = self.rt.clone();
                let a = self.r(dst);
                self.put(a, const_bytes(&rt, c)?)?;
            }
            Inst::Alloc { dst, size } => {
                let a = self.temp_alloc(self.r(size))?;
                self.set(dst, a);
            }
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
                let mut b = self.take_bytes();
                let r = self
                    .rd(self.r(src), self.r(bytes))
                    .map(|s| b.extend_from_slice(s))
                    .and_then(|()| self.put(self.r(dst), &b));
                self.give_bytes(b);
                r?;
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
                let off = self.load(add_addr(self.r(layout), 16 + 8 * u64::from(n))?, Width::B8)?;
                let a = add_addr(self.r(base), off)?;
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
            Inst::LGet { dst, base, idx } => {
                let i = self.r(idx);
                let b = self.rd(add_addr(self.r(base), i / 64 * 32)?, 32)?;
                let code = (0..4).fold(0u64, |c, p| c | (plane_word(b, p) >> (i % 64) & 1) << p);
                self.set(dst, code);
            }
            Inst::LSet { base, idx, v } => {
                let i = self.r(idx);
                let code = self.r(v);
                if code > 8 {
                    return Err(ice(format!("std_ulogic code {code}")));
                }
                let a = add_addr(self.r(base), i / 64 * 32)?;
                let b = self.wr(a, 32)?;
                let s = i % 64;
                for p in 0..4 {
                    let w = plane_word(b, p) & !(1 << s) | (code >> p & 1) << s;
                    b[p * 8..p * 8 + 8].copy_from_slice(&w.to_le_bytes());
                }
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
                let mut out = Vec::with_capacity(regs.len() * 8);
                for &r in regs {
                    out.extend_from_slice(&self.r(r).to_le_bytes());
                }
                self.put(self.r(at), &out)?;
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
            } => {
                for k in 0..u64::from(dims) {
                    let len = |m: &Machine, base: u64| -> R<u64> {
                        let d = add_addr(base, k * 24)?;
                        let l = m.load(d, Width::B8)? as i64;
                        let r = m.load(add_addr(d, 8)?, Width::B8)? as i64;
                        let dir = m.load(add_addr(d, 16)?, Width::B8)?;
                        yavs_ir::dim_len(l, r, dir == 1)
                            .ok_or_else(|| Fault::Runtime("range too long".into()))
                    };
                    let (a, b) = (len(self, self.r(dope))?, len(self, self.r(want))?);
                    if a != b {
                        let (w, _) = self.check_msg(what);
                        let dim = if dims > 1 {
                            format!(" for dimension {}", k + 1)
                        } else {
                            String::new()
                        };
                        return rt_err(format!(
                            "value length {a}{dim} does not match {w} length {b}"
                        ));
                    }
                }
            }
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
                self.reclaim();
                let data = self.rd(self.r(init), self.r(size))?.to_vec();
                let r = self.alloc_heap(shared)?.alloc(heap::Obj::Typed(ty), data);
                let h = r.map_err(|_| {
                    Fault::Runtime("heap object limit (16M objects per heap) exceeded".into())
                })?;
                self.set(dst, h);
            }
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
                self.reclaim();
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
                self.reclaim();
                self.deallocate(self.r(ptr))?;
            }
            Inst::Deref { dst, handle, what } => {
                let h = self.r(handle);
                if h == 0 {
                    // NVC's wording; the statement span names the place.
                    return rt_err("null access dereference");
                }
                let (hp, space) = self.heap_of(h)?;
                let i = match hp.resolve(h) {
                    Ok(i) => i,
                    Err(HeapError::Stale) => {
                        let (w, _) = self.check_msg(what);
                        return rt_err(format!(
                            "access value designates an object that was deallocated: {w}"
                        ));
                    }
                    Err(e) => return Err(ice(format!("dereference: {e:?}"))),
                };
                let a = mk_addr(space, i, 0)?;
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
            Inst::SigRead { dst, sig } => {
                let (sr, ty) = self.sigref(sig)?;
                let c = kc!();
                let n = value_size(ty.elem, sr.range.map_or(0, |r| r.len));
                c.read_into(sr, self.wr(self.r(dst), n)?)?;
            }
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
            Inst::WaitFor { t } => {
                let t = to_time(self.ri(t))?;
                let c = kc!();
                c.wait_for(t)?;
                if self.dbg.is_some() {
                    self.waits.timeout = c.after(t).ok();
                }
            }
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
                        use std::io::Write;
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
                        let name = ["Note", "Warning", "Error", "Failure"][s as usize];
                        let mut o = std::io::stderr().lock();
                        let _ = writeln!(o, "** {name}: (init): {text}");
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
    prog: &'a Program,
    f: &'a Func,
    ctx: &'a mut Option<&'c mut Ctx<'x>>,
    fault: Option<Fault>,
}

impl crate::TierHost for Host<'_, '_, '_> {
    fn step(&mut self, tf: &mut crate::TierFrame, at: u32) -> bool {
        let (m, f) = (&mut *self.m, self.f);
        m.swap_tier(tf);
        m.top().pc = at;
        let r = match f.code.get(at as usize) {
            Some(&inst) => m.step(self.prog, f, inst, self.ctx),
            None => Err(ice(format!(
                "compiled code stepped pc {at} beyond `{}`",
                f.name
            ))),
        };
        m.swap_tier(tf);
        match r {
            Ok(Flow::Next) => true,
            Ok(_) => {
                self.fault = Some(ice(format!(
                    "compiled code stepped a control instruction at pc {at} of `{}`",
                    f.name
                )));
                false
            }
            Err(e) => {
                self.fault = Some(e);
                false
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
    let mut x = [0u8; 8];
    x[..b.len()].copy_from_slice(b);
    let v = u64::from_le_bytes(x);
    if w == Width::B4 {
        v as u32 as i32 as i64 as u64
    } else {
        v
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

/// Expand compact constant data (rule 12) into bytes.
fn expand_const(c: &ConstData) -> R<Vec<u8>> {
    let too_big = || Fault::Runtime("constant larger than 4 GiB".into());
    Ok(match c {
        ConstData::Bytes(b) => b.clone(),
        ConstData::Repeat { elem, n } => {
            let total = (elem.len() as u64).checked_mul(*n).ok_or_else(too_big)?;
            if total > u64::from(u32::MAX) {
                return Err(too_big());
            }
            elem.repeat(*n as usize)
        }
        ConstData::Runs(runs) => {
            let mut out = Vec::new();
            for (n, e) in runs {
                if (out.len() as u64).saturating_add(n.saturating_mul(e.len() as u64))
                    > u64::from(u32::MAX)
                {
                    return Err(too_big());
                }
                for _ in 0..*n {
                    out.extend_from_slice(e);
                }
            }
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

    /// Between instructions the registers hold addresses too: the in-activation recycle
    /// counts them, the one at a suspension does not (#355).
    #[test]
    fn registers_pin_between_instructions() {
        let rt = Runtime::new(Arc::new(Program::default()), crate::Bindings::default()).unwrap();
        let mut m = Machine::new(rt, Arc::default(), vec![], FuncId(0));
        m.frames.push(Frame {
            func: FuncId(0),
            pc: 0,
            slots: mk_addr(HEAP, 3, 8).unwrap().to_le_bytes().to_vec(),
            regs: vec![mk_addr(HEAP, 5, 0).unwrap(), 7],
            link: None,
            ret: None,
            temp_base: 0,
            shared_alloc: false,
        });
        assert_eq!(m.pinned(HEAP, false), [3]);
        assert_eq!(m.pinned(HEAP, true), [3, 5]);
    }
}
