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
    AddrSpace, AttrKind, BoolOp, CheckKind, CmpOp, ConstData, Dir, DriveMode, FOp, FUnOp,
    ForceKind, Func, FuncId, FuncKind, Inst, IntOp, IntUnOp, LNumOp, LOp, LayoutError, ParamClass,
    PhysOp, Program, Reg, StmtId, Ty, Width,
};
use yavs_kernel::{
    Ctx, DelayMode, Elem, ErrorKind, ForceMode, LogicVec, Severity, SigRef, SimError, Time, Value,
    ValueRef,
};
use yavs_logic::Planes;
use yavs_std::arith;

use crate::debug::{DebugSession, FrameView, ProcState, StepMode, Stop, StopReason};
use crate::heap::{self, Heap, HeapError};
use crate::{InstData, ProcDriver, ProcRoots, Runtime, SharedState};

/// An execution failure before it becomes a `SimError` with a trace.
#[derive(Debug)]
pub(crate) enum Fault {
    Runtime(String),
    /// A call of an unbound foreign subprogram: message and places searched.
    Unbound(String, String),
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

pub(crate) const FRAME: u8 = 0;
const INST: u8 = 1;
const PKG: u8 = 2;
const CONST: u8 = 3;
const HEAP: u8 = 4;
const TEMP: u8 = 5;
pub(crate) const SHARED: u8 = 6;
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
    Call(Frame),
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
    pub inst: Arc<InstData>,
    pub drivers: Vec<ProcDriver>,
    pub frames: Vec<Frame>,
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
    blocks
        .iter()
        .flat_map(|b| b.iter().flat_map(|w| w.to_le_bytes()))
        .collect()
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
            rt,
            inst,
            drivers,
            frames: Vec::new(),
            heap: Heap::new(heap::PRIVATE),
            temp: Vec::new(),
            sh: None,
            pkg_init: None,
            entry,
            dbg: None,
            waits: Waits::default(),
            pause: None,
        }
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
        self.frames.push(fr);
        Ok(())
    }

    fn frame_for(&self, f: &Func, func: FuncId, link: Option<u32>) -> R<Frame> {
        if self.frames.len() >= self.rt.limits.max_depth as usize {
            return rt_err(format!(
                "call depth limit {} exceeded calling `{}` (unbounded recursion?)",
                self.rt.limits.max_depth, f.name
            ));
        }
        Ok(Frame {
            func,
            pc: 0,
            slots: vec![0; f.frame as usize],
            regs: vec![0; f.reg_ty.len()],
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
            Fault::Unbound(message, searched) => mk(ErrorKind::Foreign { message, searched }),
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
            let pinned = self.pinned(HEAP);
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
        let pinned = self.pinned(HEAP);
        self.heap.collect(&self.rt.prog, &roots, &pinned)
    }

    /// Slot indices of `space` (a heap) that addresses in this process's frames and
    /// temporaries may reach, sorted. Any aligned word that decodes to such an address counts:
    /// a false hit only delays reuse or collection of one object.
    pub(crate) fn pinned(&self, space: u8) -> Vec<u32> {
        let words = self
            .frames
            .iter()
            .flat_map(|f| f.slots.as_chunks::<8>().0)
            .chain(self.temp.as_chunks::<8>().0);
        let mut v: Vec<u32> = words
            .map(|w| split(u64::from_le_bytes(*w)))
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
        let handles = self.frame_roots().map(|mut v| {
            v.retain(|&h| heap::tag_of(h) == heap::SHARED);
            v
        });
        let pinned = self.pinned(SHARED_HEAP);
        let Some(sh) = self.sh.as_mut() else {
            return;
        };
        let i = *slot.get_or_insert_with(|| {
            sh.procs.push(ProcRoots::default());
            sh.procs.len() - 1
        });
        sh.procs[i] = ProcRoots { handles, pinned };
        if sh.heap.as_ref().is_some_and(Heap::has_quarantine) {
            let pinned = sh.pinned();
            if let Some(h) = sh.heap.as_mut() {
                h.recycle(&pinned);
            }
        }
    }

    fn top(&mut self) -> &mut Frame {
        let n = self.frames.len();
        &mut self.frames[n - 1]
    }

    pub(crate) fn r(&self, r: Reg) -> u64 {
        self.frames[self.frames.len() - 1].regs[r.0 as usize]
    }

    fn ri(&self, r: Reg) -> i64 {
        self.r(r) as i64
    }

    fn rf(&self, r: Reg) -> f64 {
        f64::from_bits(self.r(r))
    }

    fn set(&mut self, r: Reg, v: u64) {
        self.top().regs[r.0 as usize] = v;
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
        let mem: &[u8] = match sp {
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
            CONST => match self.rt.prog.consts.get(yavs_ir::ConstId(idx)) {
                Some(ConstData::Bytes(b)) => b,
                Some(c) => {
                    let cell = self
                        .rt
                        .expanded
                        .get(idx as usize)
                        .ok_or_else(|| ice(format!("no constant c{idx}")))?;
                    if cell.get().is_none() {
                        let _ = cell.set(expand_const(c)?);
                    }
                    cell.get().map(Vec::as_slice).unwrap_or(&[])
                }
                None => return Err(ice(format!("no constant c{idx}"))),
            },
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
        };
        let end = off.saturating_add(n);
        mem.get(off as usize..end as usize).ok_or_else(|| {
            ice(format!(
                "read of {n} bytes at {off} outside a {} object of {} bytes",
                SPACE_NAMES[sp as usize],
                mem.len()
            ))
        })
    }

    pub(crate) fn wr(&mut self, a: u64, n: u64) -> R<&mut [u8]> {
        let (sp, idx, off) = split(a);
        let mem: &mut [u8] = match sp {
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
        };
        let len = mem.len();
        let end = off.saturating_add(n);
        mem.get_mut(off as usize..end as usize).ok_or_else(|| {
            ice(format!(
                "write of {n} bytes at {off} outside a {} object of {len} bytes",
                SPACE_NAMES[sp as usize]
            ))
        })
    }

    pub(crate) fn put(&mut self, a: u64, bytes: &[u8]) -> R<()> {
        self.wr(a, bytes.len() as u64)?.copy_from_slice(bytes);
        Ok(())
    }

    pub(crate) fn load(&self, a: u64, w: Width) -> R<u64> {
        let n = w.bytes().min(8) as u64;
        let b = self.rd(a, n)?;
        let mut x = [0u8; 8];
        x[..n as usize].copy_from_slice(b);
        Ok(u64::from_le_bytes(x))
    }

    pub(crate) fn store(&mut self, a: u64, w: Width, v: u64) -> R<()> {
        let n = w.bytes().min(8) as usize;
        self.put(a, &v.to_le_bytes()[..n])
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

    /// A string result: data, `1 to len` descriptor and the fat pair `[data, dope]`.
    pub(crate) fn ret_string(&mut self, s: &[u8]) -> R<u64> {
        let data = self.temp_alloc(s.len() as u64)?;
        self.put(data, s)?;
        let dope = self.temp_alloc(24)?;
        let d: Vec<u8> = [1i64, s.len() as i64, 0]
            .iter()
            .flat_map(|v| v.to_le_bytes())
            .collect();
        self.put(dope, &d)?;
        let fat = self.temp_alloc(16)?;
        self.store(fat, Width::B8, data)?;
        self.store(add_addr(fat, 8)?, Width::B8, dope)?;
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

    fn sigref(&self, r: Reg) -> R<(SigRef, yavs_kernel::SignalType)> {
        let v = self.r(r);
        let len = self.r(Reg(r.0 + 1)) as u32;
        let (base, off) = ((v >> 32) as u32, v as u32);
        let s = self
            .inst
            .sigs
            .get(base as usize)
            .ok_or_else(|| ice(format!("no instance signal {base}")))?;
        Ok((s.signal.slice(off, len), s.ty))
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
                Ok((s.signal.slice(off, len as u32), s.ty, slot))
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

    fn is_multi(&self, r: Reg) -> bool {
        self.r(r) >> 32 == MULTI
    }

    fn driver(&self, d: u32) -> R<ProcDriver> {
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

    fn check_msg(&self, m: yavs_ir::CheckMsgId) -> (String, Dir) {
        match self.rt.prog.checks.get(m) {
            Some(c) => (c.what.clone(), c.dir),
            None => (String::from("?"), Dir::To),
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
            Ok(sh.heap.get_or_insert_with(|| Heap::new(heap::SHARED)))
        } else {
            Ok(&mut self.heap)
        }
    }

    pub(crate) fn heap_of(&mut self, h: u64) -> R<(&mut Heap, u8)> {
        match heap::tag_of(h) {
            heap::PRIVATE => Ok((&mut self.heap, HEAP)),
            heap::SHARED => {
                let sh = self.shared_mut()?;
                let h = sh.heap.get_or_insert_with(|| Heap::new(heap::SHARED));
                Ok((h, SHARED_HEAP))
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
        let rt = self.rt.clone();
        let prog = &rt.prog;
        loop {
            let fr = self
                .frames
                .last()
                .ok_or_else(|| ice("no frame to execute"))?;
            let f = prog
                .func(fr.func)
                .ok_or_else(|| ice(format!("no function {}", fr.func)))?;
            let mut pc = fr.pc as usize;
            if !DEBUG && let Some(tier) = rt.tier.get() {
                pc = self.run_tier(&**tier, prog, f, pc, ctx)?;
            }
            let inst = *f
                .code
                .get(pc)
                .ok_or_else(|| ice(format!("pc {pc} beyond `{}`", f.name)))?;
            if DEBUG {
                self.debug_tick(prog, f, pc, &inst, ctx)?;
            }
            match self.step(prog, f, inst, ctx)? {
                Flow::Next => self.top().pc += 1,
                Flow::Jump(b) => {
                    let blk = f
                        .blocks
                        .get(b)
                        .ok_or_else(|| ice(format!("no block {b}")))?;
                    self.top().pc = blk.start;
                    if self.pause == Some(b) && self.frames.len() == 1 {
                        self.pause = None;
                        return Ok(Outcome::Suspended);
                    }
                }
                Flow::Call(fr) => self.frames.push(fr),
                Flow::Ret(v) => {
                    let done = self
                        .frames
                        .pop()
                        .ok_or_else(|| ice("return without frame"))?;
                    if self.frames.len() <= base {
                        return Ok(Outcome::Returned(v));
                    }
                    let c = self.top();
                    if let Some(d) = done.ret {
                        c.regs[d.0 as usize] =
                            v.ok_or_else(|| ice("function returned no value"))?;
                    }
                    c.pc += 1;
                }
                Flow::Suspend => {
                    self.top().pc += 1;
                    return Ok(Outcome::Suspended);
                }
            }
        }
    }

    /// Run compiled code of the top frame from `pc` (ir.md 5): the index of the instruction
    /// the interpreter executes next. Compiled code steps other instructions through
    /// `step`, which never changes the frame stack.
    fn run_tier(
        &mut self,
        tier: &dyn crate::Tier,
        prog: &Program,
        f: &Func,
        pc: usize,
        ctx: &mut Option<&mut Ctx<'_>>,
    ) -> R<usize> {
        let func = self.top().func;
        let mut regs = std::mem::take(&mut self.top().regs);
        let mut fault = None;
        let ran = tier.run(func, pc as u32, &mut regs, &mut |regs, at| {
            std::mem::swap(&mut self.top().regs, regs);
            self.top().pc = at;
            let r = match f.code.get(at as usize) {
                Some(&inst) => self.step(prog, f, inst, ctx),
                None => Err(ice(format!(
                    "compiled code stepped pc {at} beyond `{}`",
                    f.name
                ))),
            };
            std::mem::swap(&mut self.top().regs, regs);
            match r {
                Ok(Flow::Next) => true,
                Ok(_) => {
                    fault = Some(ice(format!(
                        "compiled code stepped a control instruction at pc {at} of `{}`",
                        f.name
                    )));
                    false
                }
                Err(e) => {
                    fault = Some(e);
                    false
                }
            }
        });
        let top = self.top();
        top.regs = regs;
        let pc = ran.map_or(pc, |p| p as usize);
        top.pc = pc as u32;
        match fault {
            Some(e) => Err(e),
            None => Ok(pc),
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
                frames: &self.frames,
                prog,
            },
            ctx: c,
        };
        let resume = session.stop(&stop);
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
        self.frames.push(fr);
        let r = self.exec::<false>(ctx, base);
        self.frames.truncate(base);
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
        Ok(Flow::Call(fr))
    }

    /// Argument cells: two for a register that holds a signal reference.
    fn arg_cells(
        &self,
        f: &Func,
        args: yavs_ir::Regs,
        callee: Option<&Func>,
        skip: usize,
    ) -> R<Vec<u64>> {
        let regs = f
            .regs(args)
            .ok_or_else(|| ice("operand list out of range"))?;
        let mut out = Vec::with_capacity(regs.len() + 2);
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
        Ok(out)
    }

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
                let bytes = expand_const(
                    prog.consts
                        .get(c)
                        .ok_or_else(|| ice(format!("no constant {c}")))?,
                )?;
                let a = self.r(dst);
                self.put(a, &bytes)?;
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
            Inst::Load { dst, addr, w } => {
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
            }
            Inst::Store { addr, src, w } => {
                let a = self.r(addr);
                if w == Width::Sig {
                    let (lo, hi) = (self.r(src), self.r(Reg(src.0 + 1)));
                    self.store(a, Width::B8, lo)?;
                    self.store(add_addr(a, 8)?, Width::B8, hi)?;
                } else {
                    let v = self.r(src);
                    self.store(a, w, v)?;
                }
            }
            Inst::Copy { dst, src, bytes } => {
                let b = self.rd(self.r(src), self.r(bytes))?.to_vec();
                self.put(self.r(dst), &b)?;
            }
            Inst::Fill { dst, elem, n, size } => {
                let e = self.rd(self.r(elem), u64::from(size))?.to_vec();
                let n = self.r(n);
                let total = n
                    .checked_mul(u64::from(size))
                    .ok_or_else(|| Fault::Runtime("object larger than 4 GiB".into()))?;
                let out = self.wr(self.r(dst), total)?;
                for c in out.chunks_exact_mut(size.max(1) as usize) {
                    c.copy_from_slice(&e);
                }
            }
            Inst::Pos {
                dst,
                dope,
                dim,
                idx,
                what,
            } => {
                let d = add_addr(self.r(dope), u64::from(dim) * 24)?;
                let (l, rr, dir) = (
                    self.load(d, Width::B8)? as i64,
                    self.load(add_addr(d, 8)?, Width::B8)? as i64,
                    self.load(add_addr(d, 16)?, Width::B8)?,
                );
                let i = self.ri(idx);
                let pos = match dir {
                    0 if l <= i && i <= rr => i - l,
                    1 if rr <= i && i <= l => l - i,
                    0 | 1 => {
                        let b = if dir == 0 {
                            format!("{l} to {rr}")
                        } else {
                            format!("{l} downto {rr}")
                        };
                        return rt_err(outside("index", &i.to_string(), &b, &self.msg(what)));
                    }
                    _ => return Err(ice(format!("direction {dir} in a descriptor"))),
                };
                self.set(dst, pos as u64);
            }
            Inst::Index {
                dst,
                base,
                pos,
                stride,
            } => {
                let off = self
                    .r(pos)
                    .checked_mul(u64::from(stride))
                    .ok_or_else(|| Fault::Runtime("object larger than 4 GiB".into()))?;
                let a = add_addr(self.r(base), off)?;
                self.set(dst, a);
            }
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
            Inst::Bin { op, dst, a, b } => {
                let (x, y) = (self.ri(a), self.ri(b));
                let v = arith_r(int_op(op, x, y))?;
                self.set(dst, v);
            }
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
            } => {
                let (x, y) = (self.ri(a), self.ri(b));
                let (l, h) = (self.ri(lo), self.ri(hi));
                match int_op(op, x, y) {
                    Ok(v) if l <= v && v <= h => self.set(dst, v as u64),
                    Err(arith::ArithError::DivisionByZero { .. }) => {
                        return rt_err("division by zero");
                    }
                    Err(e @ arith::ArithError::NegativeExponent { .. }) => {
                        return rt_err(e.to_string());
                    }
                    _ => {
                        let m = self.msg(what);
                        let t = if m.subtype.is_empty() {
                            "the result type"
                        } else {
                            &m.subtype
                        };
                        return rt_err(format!(
                            "result of {x} {} {y} cannot be represented as {t}",
                            op_symbol(op)
                        ));
                    }
                }
            }
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
                let b = blocks_of(self.rd(add_addr(self.r(base), i / 64 * 32)?, 32)?);
                let code = (0..4).fold(0u64, |c, p| c | (b[0][p] >> (i % 64) & 1) << p);
                self.set(dst, code);
            }
            Inst::LSet { base, idx, v } => {
                let i = self.r(idx);
                let code = self.r(v);
                if code > 8 {
                    return Err(ice(format!("std_ulogic code {code}")));
                }
                let a = add_addr(self.r(base), i / 64 * 32)?;
                let mut b = blocks_of(self.rd(a, 32)?);
                for p in 0..4 {
                    set_bits(&mut b, i % 64, 1, p, code >> p & 1);
                }
                self.put(a, &bytes_of(&b))?;
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
            } => {
                let x = self.ri(v);
                let (w, dir) = self.check_msg(what);
                match kind {
                    CheckKind::Range | CheckKind::Index => {
                        let (l, h) = (self.ri(lo), self.ri(hi));
                        if x < l || x > h {
                            let k = if kind == CheckKind::Index {
                                "index"
                            } else {
                                "value"
                            };
                            let m = self.msg(what);
                            return rt_err(outside(k, &x.to_string(), &bounds(l, h, dir), &m));
                        }
                    }
                    CheckKind::Length => {
                        let l = self.ri(lo);
                        if x != l {
                            return rt_err(format!(
                                "value length {x} does not match {w} length {l}"
                            ));
                        }
                    }
                    CheckKind::Null => {
                        if x == 0 {
                            return rt_err(format!("null access value: {w}"));
                        }
                    }
                }
            }
            Inst::MakeDope { at, parts } => {
                let regs = f.regs(parts).ok_or_else(|| ice("operand list"))?;
                let mut out = Vec::with_capacity(regs.len() * 8);
                for &r in regs {
                    out.extend_from_slice(&self.r(r).to_le_bytes());
                }
                self.put(self.r(at), &out)?;
            }
            Inst::DopeLen { dst, dope, dim } => {
                let d = add_addr(self.r(dope), u64::from(dim) * 24)?;
                let (l, rr, dir) = (
                    self.load(d, Width::B8)? as i64,
                    self.load(add_addr(d, 8)?, Width::B8)? as i64,
                    self.load(add_addr(d, 16)?, Width::B8)?,
                );
                let n = yavs_ir::dim_len(l, rr, dir == 1)
                    .filter(|&n| n <= i64::MAX as u64)
                    .ok_or_else(|| Fault::Runtime(format!("range {l} .. {rr} too long")))?;
                self.set(dst, n);
            }
            Inst::LayoutOf { dst, ty, desc } => {
                let dsz = Ty::desc_size(&prog.types, ty)
                    .ok_or_else(|| ice(format!("type {ty} has no descriptor size")))?;
                let d = self.rd(self.r(desc), dsz)?.to_vec();
                let (l, _) =
                    Ty::layout(&prog.types, &prog.consts, ty, &d).map_err(|e| match e {
                        LayoutError::TooLarge => Fault::Runtime(e.to_string()),
                        LayoutError::Malformed(_) => ice(e.to_string()),
                    })?;
                let mut words = vec![l.size, l.align];
                if l.offsets.is_empty() {
                    words.extend([l.stride, l.count]);
                } else {
                    words.extend(&l.offsets);
                }
                let a = self.temp_alloc(words.len() as u64 * 8)?;
                let b: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
                self.put(a, &b)?;
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
                let cells = self.arg_cells(f, args, prog.func(func), 0)?;
                return self.call(prog, func, &cells, dst);
            }
            Inst::CallIndirect { func, args, dst } => {
                let id = FuncId(self.r(func) as u32);
                let c = prog
                    .func(id)
                    .ok_or_else(|| ice(format!("indirect call of missing function {id}")))?;
                if c.kind == FuncKind::Process || c.kind == FuncKind::Elab {
                    return Err(ice(format!("indirect call of {:?} {id}", c.kind)));
                }
                let cells = self.arg_cells(f, args, Some(c), 0)?;
                return self.call(prog, id, &cells, dst);
            }
            Inst::CallProt {
                func,
                obj,
                args,
                dst,
            } => {
                self.shared()?;
                let h = self.r(obj);
                let mut cells = vec![h];
                cells.extend(self.arg_cells(f, args, prog.func(func), 1)?);
                let flow = self.call(prog, func, &cells, dst)?;
                if let Flow::Call(mut fr) = flow {
                    fr.shared_alloc |= heap::tag_of(h) == heap::SHARED;
                    return Ok(Flow::Call(fr));
                }
                return Ok(flow);
            }
            Inst::CallNative { native, args, dst } => {
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
                let cells = self.arg_cells(f, args, None, 0)?;
                let v = fun(self, ctx, n.ty, &cells)?;
                if let Some(d) = dst {
                    let v =
                        v.ok_or_else(|| ice(format!("native `{}` returned no value", n.symbol)))?;
                    self.set(d, v);
                }
                // `std.env.stop`/`finish` end the calling process at once (LRM 16.5): the
                // simulation ends after this cycle, so execute no further, as for a report
                // at the stop severity.
                if n.symbol.starts_with("yavs.env.stop.")
                    || n.symbol.starts_with("yavs.env.finish.")
                {
                    return Ok(Flow::Suspend);
                }
            }
            Inst::CallForeign { foreign, args, dst } => {
                let v = self.foreign(prog, f, foreign, args)?;
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
                let data = self.rd(self.r(init), self.r(size))?.to_vec();
                let r = self.alloc_heap(shared)?.alloc(heap::Obj::Typed(ty), data);
                let h = r.map_err(|_| {
                    Fault::Runtime("heap object limit (16M objects per heap) exceeded".into())
                })?;
                self.set(dst, h);
            }
            Inst::Free { ptr } => self.deallocate(self.r(ptr))?,
            Inst::Deref { dst, handle, what } => {
                let h = self.r(handle);
                let (w, _) = self.check_msg(what);
                if h == 0 {
                    // NVC's wording; the statement span names the place.
                    return rt_err("null access dereference");
                }
                let (hp, space) = self.heap_of(h)?;
                let i = match hp.resolve(h) {
                    Ok(i) => i,
                    Err(HeapError::Stale) => {
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
                let (o, l) = (self.ri(offset), self.ri(len));
                let (Ok(o), Ok(l)) = (u32::try_from(o), u32::try_from(l)) else {
                    return rt_err(format!("signal element range {o} + {l} out of bounds"));
                };
                if self.inst.sigs.get(sig as usize).is_none() {
                    return Err(ice(format!("no instance signal {sig}")));
                }
                self.set(dst, u64::from(sig) << 32 | u64::from(o));
                self.set(Reg(dst.0 + 1), u64::from(l));
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
                let v = match ty.elem {
                    Elem::Bytes(_) => Value::Bytes(c.value(sr)?.to_vec()),
                    Elem::Logic => Value::Logic(c.logic(sr)?),
                };
                self.put_value(self.r(dst), &v)?;
            }
            Inst::SigReadScalar { dst, sig } => {
                let (sr, ty) = match self.sigparts(sig)?.as_slice() {
                    [(sr, ty, _)] => (*sr, *ty),
                    _ => return Err(ice("scalar read of a reference of several parts")),
                };
                let c = kc!();
                let v = match ty.elem {
                    Elem::Bytes(es) if es <= 8 => {
                        let b = c.value(sr)?;
                        let mut a = [0u8; 8];
                        let n = b.len().min(8);
                        a[..n].copy_from_slice(&b[..n]);
                        u64::from_le_bytes(a)
                    }
                    Elem::Logic => {
                        let l = c.logic(sr)?;
                        if l.is_empty() {
                            return Err(ice("scalar read of a null signal range"));
                        }
                        l.get(0) as u64
                    }
                    Elem::Bytes(es) => {
                        return Err(ice(format!("scalar read of a {es}-byte element")));
                    }
                };
                self.set(dst, v);
            }
            Inst::SigLastValue { dst, sig } => {
                let parts = self.sigparts(sig)?;
                let v = self.read_parts(kc!(), &parts, true)?;
                self.put_value(self.r(dst), &v)?;
            }
            Inst::SigAttr { dst, kind, sig } => {
                // Over several parts: any event or activity, the most recent one (LRM 16.2.4).
                let parts = self.sigparts(sig)?;
                let c = kc!();
                let mut v = match kind {
                    AttrKind::Event | AttrKind::Active => 0,
                    AttrKind::LastEvent | AttrKind::LastActive => u64::MAX,
                };
                for (sr, _, _) in parts {
                    v = match kind {
                        AttrKind::Event => v | u64::from(c.event(sr)?),
                        AttrKind::Active => v | u64::from(c.active(sr)?),
                        AttrKind::LastEvent => v.min(c.last_event(sr)?.as_fs()),
                        AttrKind::LastActive => v.min(c.last_active(sr)?.as_fs()),
                    };
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
            Inst::WaitOn { sig } => {
                for (sr, _, _) in self.sigparts(sig)? {
                    kc!().wait_on(sr)?;
                    if self.dbg.is_some() {
                        self.waits.sens.push(sr);
                    }
                }
            }
            Inst::WaitFor { t } => {
                let t = to_time(self.ri(t))?;
                let c = kc!();
                c.wait_for(t)?;
                let deadline = c.after(t)?;
                if self.dbg.is_some() {
                    self.waits.timeout = Some(deadline);
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
                if let Some(fr) = self.frames.iter().find(|fr| {
                    prog.func(fr.func).is_some_and(|g| {
                        g.kind != FuncKind::Process && g.kind != FuncKind::Procedure
                    })
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
                        if !self.rt.asserts.report_elab(s) {
                            return Ok(Flow::Next);
                        }
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
            Inst::Stmt => {
                let base = self.frames[self.frames.len() - 1].temp_base;
                self.temp.truncate(base);
            }
            Inst::Elab { op, .. } => {
                return rt_err(format!(
                    "elaboration instruction {op:?} needs yavs-elab (not implemented yet)"
                ));
            }
        }
        Ok(Flow::Next)
    }
}

fn force_mode(k: ForceKind) -> ForceMode {
    match k {
        ForceKind::In => ForceMode::Effective,
        ForceKind::Out => ForceMode::Driving,
    }
}

fn store_cells(slots: &mut [u8], slot: u32, w: Width, cells: &[u64]) -> R<()> {
    let mut bytes = Vec::with_capacity(16);
    for c in cells {
        bytes.extend_from_slice(&c.to_le_bytes());
    }
    let n = w.bytes() as usize;
    let dst = slots
        .get_mut(slot as usize..slot as usize + n)
        .ok_or_else(|| ice(format!("parameter slot {slot} outside the frame")))?;
    let src = bytes
        .get(..n)
        .ok_or_else(|| ice("too few argument cells"))?;
    dst.copy_from_slice(src);
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
