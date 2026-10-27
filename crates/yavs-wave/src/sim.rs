//! The kernel side (M7, #129): the elaborated design as a [`Hierarchy`] and a [`Recorder`]
//! that subscribes to the kernel's value-change bus.
//!
//! Every recorded value is one sub-element of one kernel leaf (rule 11): a scalar, or a
//! one-dimensional array of `std_ulogic`/`bit` written as one vector. Records are scopes, arrays
//! of anything else are array scopes with one entry per element. The recorder marks the
//! entries a change touches and writes their final value once per time step (FST has no
//! deltas), in entry order, so the file does not depend on `--threads`.
//!
//! Variables (`--wave-variables`: process variables and non-protected shared variables) are
//! not on the bus: each is read once per time step, at its end, when every process is
//! suspended (`Hook::with_process`, `yavs_interp::shared_var`), and a sub-element is written
//! when it differs from the last written value. Their shapes are the elaborated ones
//! (`Elaborated::var_shapes`), so records and arrays are scopes as for signals.

use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use yavs_elab::{Built, DataLeaf, Elaborated, ObjKind, ScopeId, Shape, leaf_locs};
use yavs_interp::{CallVal, InterpProcess, Stamp};
use yavs_kernel::{
    Elem, Hook, Observer, Phase, PortMode, ProcId, Range, SigRef, SigView, SimControl, SimError,
};
use yavs_sem::Db;
use yavs_sem::decl::Designator;
use yavs_sem::decl::{DeclKind, ObjClass as SemClass};
use yavs_sem::hir::{Direction, StmtKind};
use yavs_sem::types::TypeKind;

use crate::model::*;
use crate::{Error, Selection, WaveConfig, WaveWriter};

/// How one entry's kernel value becomes the byte layout of its [`SigType`] (crate docs).
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
enum Conv {
    /// Logic elements, left first, as positions: the code minus `base` (2 for BIT, whose
    /// '0'/'1' are std_ulogic codes 2/3 in the kernel).
    Logic { base: u8 },
    /// BOOLEAN (a logic leaf) as a `u32` enum position.
    Bool,
    /// A one-byte enum as a `u32` position.
    Enum8,
    /// Already in the layout: 4-byte enums, integers, physicals, reals.
    Raw,
    /// A narrow integer array element (4 bytes in the kernel, `Ty::elem_size`) as an `i64`.
    Int32,
}

enum Src {
    /// Kernel signal, offset and element count.
    Sig { sig: u32, offset: u32, len: u32 },
    /// Sub-element of a variable: its place in the variable's value is `Var::plan`.
    Var,
}

/// Where a variable is read.
enum VarAt {
    /// Variable `name` of kernel process `proc`.
    Proc { proc: ProcId, name: String },
    /// The slot and IR type of a shared variable.
    Shared { slot: u64, ty: yavs_ir::TyId },
}

/// The path of each sub-element of a variable from the variable itself: per step an array
/// element (row major) or a record field; a logic vector is the whole array at the end of
/// the path. All in one vector, `ends[j]` the end of entry `j`'s.
#[derive(Default)]
struct Paths {
    data: Vec<u32>,
    ends: Vec<u32>,
}

/// A recorded variable, the run of entries of its sub-elements and the values last written.
struct Var {
    at: VarAt,
    entries: std::ops::Range<u32>,
    /// The paths of the entries until the first read has made the plan.
    paths: Option<Paths>,
    /// Where each entry's value is in the raw value (`yavs_elab::leaf_locs`).
    plan: Vec<DataLeaf>,
    /// The descriptor the plan was made for.
    plan_desc: Vec<u8>,
    /// The values last written to the entries, one after the other (empty until `primed`).
    last: Vec<u8>,
    /// Every entry has been written once.
    primed: bool,
    /// The process's `execs` at the last read (`u64::MAX`: never read): same means unchanged.
    execs: u64,
    /// The raw value at the last read: an equal one needs no decoding.
    raw: Option<CallVal>,
    /// The raw value before it, until the changes since are written.
    old: Option<CallVal>,
    /// The stamp of the data when `raw` was read.
    stamp: Option<Stamp>,
}

/// What reading a variable found.
enum Read {
    /// A new value (`Var::raw`).
    Changed,
    /// The same value as before.
    Same,
    /// Its process has no frame or has not run since the last read, so no variable of it has
    /// changed.
    Idle,
}

/// Variables read by the recorder, and variables skipped because their process was found idle
/// (work counters, `var_reads` and `var_idle_skips` of `YAVS_COUNTERS`).
static VAR_READS: AtomicU64 = AtomicU64::new(0);
static VAR_IDLE: AtomicU64 = AtomicU64::new(0);

/// `(variables read, variables skipped as idle)` by this process's recorders so far.
pub fn var_counts() -> (u64, u64) {
    (
        VAR_READS.load(Ordering::Relaxed),
        VAR_IDLE.load(Ordering::Relaxed),
    )
}

const NO_ENTRY: u32 = u32::MAX;

/// Entries of one leaf that do not overlap, by offset.
struct Layer {
    ids: Vec<u32>,
    /// `Some((base, stride))` if entry `k` is exactly elements `base + k * stride` and the
    /// next `stride` (an array of equal elements): found by division, not by search.
    tiles: Option<(u32, u32)>,
}

struct Entry {
    src: Src,
    conv: Conv,
}

struct State {
    writer: Option<Box<dyn Sink>>,
    entries: Vec<Entry>,
    /// Per kernel signal: its entries in layers, each by offset and disjoint (a port collapsed
    /// into its actual shares the actual's elements: another layer).
    by_sig: Vec<Vec<Layer>>,
    /// Per kernel signal: its only entry (`NO_ENTRY`: none or several), the common case.
    solo: Vec<u32>,
    /// Value bytes of the entry being written.
    bytes: Vec<u8>,
    dirty: Vec<bool>,
    pending: Vec<u32>,
    /// The time of the pending changes, fs.
    at: u64,
    vars: Vec<Var>,
    /// The shared state, for shared variables.
    built: Option<Built>,
    /// The time step in progress, fs (variables are written at its end).
    step: u64,
    error: Option<Error>,
}

/// The waveform subscriber. Clone it into [`SimControl::subscribe`] and keep one handle for
/// [`Recorder::finish`].
#[derive(Clone)]
pub struct Recorder(Arc<Mutex<State>>);

/// Where a [`Recorder`] puts the values: the FST [`WaveWriter`], or another consumer (the DAP
/// server's live stream). Values follow the byte layouts of the crate docs.
pub trait Sink: Send {
    /// Times do not decrease.
    fn change(&mut self, time: u64, id: SignalId, value: &[u8]) -> Result<(), Error>;
    /// Every change of the time step at `time` has been given.
    fn step(&mut self, _time: u64) {}
    /// Make what was given so far readable by others (a debugger stop); default: nothing.
    fn sync(&mut self) -> Result<(), Error> {
        Ok(())
    }
    fn finish(self: Box<Self>) -> Result<(), Error>;
}

impl Sink for WaveWriter {
    fn change(&mut self, time: u64, id: SignalId, value: &[u8]) -> Result<(), Error> {
        WaveWriter::change(self, time, id, value)
    }

    fn sync(&mut self) -> Result<(), Error> {
        self.flush()
    }

    fn finish(self: Box<Self>) -> Result<(), Error> {
        WaveWriter::finish(*self)
    }
}

/// Build the hierarchy of `e` under `sel`, create the FST at `path` and return the recorder
/// and the arrays left out by the size cap.
pub fn record(
    db: &Db,
    e: &Elaborated,
    path: &Path,
    sel: Selection,
    cfg: WaveConfig,
    built: Option<&Built>,
) -> Result<(Recorder, Vec<Skipped>), Error> {
    // A file names equal values once (FST aliases); a live stream keeps one id per name.
    record_with(db, e, sel, built, true, |h| {
        Ok(Box::new(WaveWriter::create(path, h, cfg)?))
    })
    .map(|(r, h)| (r, h.skipped))
}

/// [`record`] into the sink `sink` makes for the hierarchy; returns the recorder and the
/// hierarchy.
pub fn record_to(
    db: &Db,
    e: &Elaborated,
    sel: Selection,
    built: Option<&Built>,
    sink: impl FnOnce(&Hierarchy) -> Result<Box<dyn Sink>, Error>,
) -> Result<(Recorder, Hierarchy), Error> {
    record_with(db, e, sel, built, false, sink)
}

fn record_with(
    db: &Db,
    e: &Elaborated,
    sel: Selection,
    built: Option<&Built>,
    alias: bool,
    sink: impl FnOnce(&Hierarchy) -> Result<Box<dyn Sink>, Error>,
) -> Result<(Recorder, Hierarchy), Error> {
    let mut d = Design {
        db,
        e,
        built,
        b: HierarchyBuilder::new(sel),
        entries: Vec::new(),
        enums: HashMap::new(),
        vars: Vec::new(),
        pkg_vars: BTreeMap::new(),
        vpath: Vec::new(),
        paths: Paths::default(),
        scalars: HashMap::new(),
        alias: alias.then(HashMap::new),
        vectors: HashMap::new(),
    };
    d.package_vars();
    for (i, s) in e.hier.scopes.iter().enumerate() {
        if s.parent.is_none() {
            d.scope(ScopeId(i as u32));
        }
    }
    // Packages whose library has no scope in the design.
    let libs: Vec<_> = d.pkg_vars.keys().map(|&u| db.units[u].lib).collect();
    for lib in libs {
        if d.pkg_vars.keys().any(|&u| db.units[u].lib == lib) {
            d.b.scope(db.libs[lib].name.text.as_str(), ScopeKind::Block, "");
            d.packages_of(lib);
            d.b.up();
        }
    }
    let Design {
        b, entries, vars, ..
    } = d;
    let h = b.finish();
    let writer = sink(&h)?;
    let mut flat = vec![Vec::new(); e.nets.signal_count()];
    for (i, en) in entries.iter().enumerate() {
        if let Src::Sig { sig, .. } = &en.src {
            flat[*sig as usize].push(i as u32);
        }
    }
    let by_sig: Vec<Vec<Layer>> = (flat.into_iter())
        .map(|mut v| {
            v.sort_by_key(|&i| entries[i as usize].span());
            let mut layers: Vec<Vec<u32>> = Vec::new();
            for i in v {
                let (a, _) = entries[i as usize].span();
                let end = |l: &Vec<u32>| {
                    l.last().map_or(0, |&j| {
                        let (b, n) = entries[j as usize].span();
                        b + n
                    })
                };
                match layers.iter_mut().find(|l| end(l) <= a) {
                    Some(l) => l.push(i),
                    None => layers.push(vec![i]),
                }
            }
            let tiles = |ids: &[u32]| {
                let (base, stride) = entries[*ids.first()? as usize].span();
                let tiled = (ids.iter().enumerate()).all(|(k, &i)| {
                    entries[i as usize].span() == (base + k as u32 * stride, stride)
                });
                tiled.then_some((base, stride))
            };
            (layers.into_iter())
                .map(|ids| Layer {
                    tiles: tiles(&ids),
                    ids,
                })
                .collect()
        })
        .collect();
    let solo = (by_sig.iter().enumerate())
        .map(|(s, l)| match &l[..] {
            [Layer { ids, .. }] if ids.len() == 1 => {
                let n = e.nets.signal_len(yavs_kernel::SignalId(s as u32));
                let covers = entries[ids[0] as usize].span() == (0, n);
                if covers { ids[0] } else { NO_ENTRY }
            }
            _ => NO_ENTRY,
        })
        .collect();
    let n = entries.len();
    let st = State {
        writer: Some(writer),
        entries,
        by_sig,
        solo,
        bytes: Vec::new(),
        dirty: vec![false; n],
        pending: Vec::new(),
        at: 0,
        vars,
        built: built.cloned(),
        step: 0,
        error: None,
    };
    Ok((Recorder(Arc::new(Mutex::new(st))), h))
}

impl Recorder {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.0.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Record every signal's value from `now` (fs) on, for a recorder subscribed after the
    /// start of the simulation (`Phase::StartOfSim` does this otherwise).
    pub fn start(&self, now: u64) {
        self.lock().start(now);
    }

    /// Write the completed time steps to disk now, so a reader of the file sees every change up
    /// to the last one (a debugger stop); a step still running appears once it ends.
    pub fn sync(&self) {
        let mut guard = self.lock();
        let st = &mut *guard;
        if st.error.is_none()
            && let Some(w) = st.writer.as_mut()
            && let Err(e) = w.sync()
        {
            st.error = Some(e);
        }
    }

    /// Write what is still pending (a run that paused or failed inside a time step), close the
    /// file and return the first error of the run, if any.
    pub fn finish(&self, sim: &SimControl) -> Result<(), Error> {
        let mut st = self.lock();
        st.flush(|r| sim.view(r));
        if let Some(e) = st.error.take() {
            st.writer = None; // drop finishes what can be finished
            return Err(e);
        }
        match st.writer.take() {
            Some(w) => w.finish(),
            None => Ok(()),
        }
    }
}

impl Entry {
    /// Kernel offset and length of a signal entry.
    fn span(&self) -> (u32, u32) {
        match self.src {
            Src::Sig { offset, len, .. } => (offset, len),
            Src::Var => (0, 0),
        }
    }
}

impl State {
    fn start(&mut self, now: u64) {
        self.at = now;
        self.step = now;
        for i in 0..self.entries.len() as u32 {
            if matches!(self.entries[i as usize].src, Src::Sig { .. }) {
                self.mark(i);
            }
        }
    }

    fn step_done(&mut self) {
        if let (None, Some(w)) = (&self.error, &mut self.writer) {
            w.step(self.step);
        }
    }

    /// Write the variable sub-elements that changed since the last sample, at the time of the
    /// step that just ended (every process is suspended).
    fn sample_vars(&mut self, hook: &mut Hook<'_>) {
        let mut bytes = Vec::new();
        let mut idle: Option<ProcId> = None;
        for k in 0..self.vars.len() {
            if self.error.is_some() {
                return;
            }
            if let VarAt::Proc { proc, .. } = &self.vars[k].at
                && Some(*proc) == idle
            {
                VAR_IDLE.fetch_add(1, Ordering::Relaxed);
                continue;
            }
            VAR_READS.fetch_add(1, Ordering::Relaxed);
            match self.read_var(hook, k) {
                Ok(Read::Changed) => {}
                Ok(Read::Same) => continue,
                // The process has not started yet, has ended or has not run since: its last
                // values stay, and so do those of its other variables (they were read together).
                Ok(Read::Idle) => {
                    if let VarAt::Proc { proc, .. } = &self.vars[k].at {
                        idle = Some(*proc);
                    }
                    continue;
                }
                Err(m) => {
                    self.error = Some(Error::Value(m));
                    return;
                }
            }
            let State {
                vars,
                entries,
                writer,
                step,
                ..
            } = self;
            let var = &mut vars[k];
            let cell;
            let data = match &var.raw {
                Some(CallVal::Composite { data, .. }) => data.as_slice(),
                Some(CallVal::Cell(c)) => {
                    cell = c.to_le_bytes();
                    &cell[..]
                }
                None => continue,
            };
            let bug = |what: &str| {
                Error::Value(format!(
                    "this is a bug in yavs: a recorded variable's value: {what}"
                ))
            };
            // Entries whose bytes are unchanged since the value before need no look: count
            // the changed 64-byte chunks up to each chunk.
            let old = var.old.take();
            let dirty = match (&old, var.primed) {
                (Some(o), true) => chunk_counts(o, &var.raw),
                _ => None,
            };
            let mut at = 0;
            for (leaf, i) in var.plan.iter().zip(var.entries.clone()) {
                let logic = matches!(entries[i as usize].conv, Conv::Logic { .. });
                if let Some(cnt) = &dirty {
                    let (lo, hi) = leaf.span();
                    if cnt[(hi - 1) / CHUNK + 1] == cnt[lo / CHUNK] {
                        at += leaf.width(logic);
                        continue;
                    }
                }
                bytes.clear();
                if leaf.bytes(data, logic, &mut bytes).is_none() {
                    self.error = Some(bug("it does not hold an entry"));
                    return;
                }
                let end = at + bytes.len();
                if var.primed {
                    match var.last.get_mut(at..end) {
                        Some(last) if *last == bytes[..] => {
                            at = end;
                            continue;
                        }
                        Some(last) => last.copy_from_slice(&bytes),
                        None => {
                            self.error = Some(bug("an entry changed its width"));
                            return;
                        }
                    }
                } else {
                    var.last.extend_from_slice(&bytes);
                }
                at = end;
                let w = writer.as_mut().expect("writer open while recording");
                if let Err(e) = w.change(*step, SignalId(i), &bytes) {
                    self.error = Some(e);
                    return;
                }
            }
            var.primed = true;
        }
    }

    /// Read variable `k` if it may differ from the last read: whether its raw value changed
    /// (then `Var::raw` is the new one and the plan is made). `false` while its process has
    /// no frame, has not run since, or the value is unchanged.
    fn read_var(&mut self, hook: &mut Hook<'_>, k: usize) -> Result<Read, String> {
        let bug =
            |what: &str| format!("this is a bug in yavs: reading a recorded variable: {what}");
        let var = &mut self.vars[k];
        let Var {
            execs,
            raw,
            old,
            stamp,
            paths,
            plan,
            plan_desc,
            ..
        } = var;
        match &var.at {
            VarAt::Proc { proc, name } => hook
                .with_process(*proc, |p| {
                    let p = p
                        .as_any()
                        .and_then(|a| a.downcast_mut::<InterpProcess>())
                        .ok_or_else(|| bug("not an interpreter process"))?;
                    if !p.has_vars() || p.execs() == *execs {
                        return Ok(Read::Idle);
                    }
                    // Compared in place: most variables of a process that ran are unchanged.
                    if let Some(r) = raw.as_ref()
                        && p.var_is(name, r, stamp).map_err(|m| bug(&m))?
                    {
                        *execs = p.execs();
                        return Ok(Read::Same);
                    }
                    let (ty, cv) = p.var(name).map_err(|m| bug(&m))?;
                    *execs = p.execs();
                    make_plan(paths, plan, plan_desc, p.program(), ty, &cv)?;
                    *old = raw.replace(cv);
                    Ok(Read::Changed)
                })
                .unwrap_or_else(|| Err(bug("no process"))),
            VarAt::Shared { slot, ty } => {
                let b = self.built.as_ref().ok_or_else(|| bug("no shared state"))?;
                if let Some(r) = raw.as_ref()
                    && yavs_interp::shared_var_is(&b.rt, &b.shared, *slot, *ty, r, stamp)
                        .map_err(|m| bug(&m))?
                {
                    return Ok(Read::Same);
                }
                let cv =
                    yavs_interp::shared_var(&b.rt, &b.shared, *slot, *ty).map_err(|m| bug(&m))?;
                make_plan(paths, plan, plan_desc, &b.rt.prog, *ty, &cv)?;
                *old = raw.replace(cv);
                Ok(Read::Changed)
            }
        }
    }

    /// Mark the entries that elements `range` of kernel signal `sig` touch.
    fn mark_range(&mut self, sig: u32, range: Range) {
        let Some(layers) = self.by_sig.get(sig as usize) else {
            return;
        };
        let (lo, hi) = (range.offset, range.offset + range.len);
        for Layer { ids, tiles } in layers {
            let (first, last) = match *tiles {
                Some((base, stride)) => {
                    if hi <= base {
                        continue;
                    }
                    let k0 = (lo.saturating_sub(base) / stride) as usize;
                    (k0, (((hi - 1 - base) / stride) as usize + 1).min(ids.len()))
                }
                None => {
                    let first = ids.partition_point(|&i| {
                        let (o, n) = self.entries[i as usize].span();
                        o + n <= lo
                    });
                    let past = ids[first..]
                        .iter()
                        .position(|&i| self.entries[i as usize].span().0 >= hi);
                    (first, past.map_or(ids.len(), |p| first + p))
                }
            };
            for &i in ids.get(first..last).unwrap_or_default() {
                if !self.dirty[i as usize] {
                    self.dirty[i as usize] = true;
                    self.pending.push(i);
                }
            }
        }
    }

    fn mark(&mut self, i: u32) {
        if !self.dirty[i as usize] {
            self.dirty[i as usize] = true;
            self.pending.push(i);
        }
    }

    fn flush<'h>(&mut self, read: impl Fn(SigRef) -> Result<SigView<'h>, SimError>) {
        if self.pending.is_empty() {
            return;
        }
        let mut pending = std::mem::take(&mut self.pending);
        // Marked in kernel signal order, which is mostly entry order.
        if !pending.is_sorted() {
            pending.sort_unstable();
        }
        let mut bytes = std::mem::take(&mut self.bytes);
        for &i in &pending {
            self.dirty[i as usize] = false;
            if self.error.is_some() {
                continue;
            }
            let en = &self.entries[i as usize];
            let Src::Sig { sig, offset, len } = en.src else {
                continue;
            };
            let r = yavs_kernel::SignalId(sig).slice(offset, len);
            let v = match read(r) {
                Ok(v) => v,
                Err(e) => {
                    self.error = Some(Error::Value(format!("reading a signal: {e:?}")));
                    continue;
                }
            };
            bytes.clear();
            convert(en, &v, &mut bytes);
            let w = self.writer.as_mut().expect("writer open while recording");
            if let Err(e) = w.change(self.at, SignalId(i), &bytes) {
                self.error = Some(e);
            }
        }
        pending.clear();
        self.pending = pending;
        self.bytes = bytes;
    }
}

fn convert(en: &Entry, v: &SigView<'_>, out: &mut Vec<u8>) {
    match (en.conv, v.bytes()) {
        // Kernel offset 0 is the rightmost element (yavs-elab `NetGraph::select`).
        (Conv::Logic { base }, None) => logic_rev(v, base, out),
        (Conv::Bool, None) => {
            out.extend_from_slice(&u32::from(v.code(0)).wrapping_sub(2).to_le_bytes())
        }
        (Conv::Enum8, Some(b)) => {
            out.extend_from_slice(&u32::from(b[0]).to_le_bytes());
        }
        (Conv::Raw, Some(b)) => out.extend_from_slice(b),
        (Conv::Int32, Some(b)) => {
            for c in b.as_chunks::<4>().0 {
                out.extend_from_slice(&i64::from(i32::from_le_bytes(*c)).to_le_bytes());
            }
        }
        // A mismatch reaches the writer as a wrong length, which it reports.
        _ => {}
    }
}

/// Bit `j` of the low byte of `b` as byte `j` of the result (0 or 1).
#[inline(always)]
fn spread(b: u64) -> u64 {
    let y = (b & 0xff).wrapping_mul(0x0101_0101_0101_0101) & 0x8040_2010_0804_0201;
    ((y + 0x7f7f_7f7f_7f7f_7f7f) >> 7) & 0x0101_0101_0101_0101
}

/// The codes of the elements, last first, minus `base`, eight elements per step from the
/// plane words. Codes below `base` (they do not occur) come out as an out-of-range position
/// the writer refuses.
fn logic_rev(v: &SigView<'_>, base: u8, out: &mut Vec<u8>) {
    const ONES: u64 = 0x0101_0101_0101_0101;
    let n = v.len();
    let start = out.len();
    out.resize(start + n, 0);
    let dst = &mut out[start..];
    let sub = u64::from(base) * ONES;
    let mut c = 0;
    while c < n {
        let k = (n - c).min(64);
        let Some(p) = v.planes(c, k) else {
            out.truncate(start);
            return;
        };
        // Only '0' and '1' (codes 2 and 3): the position is the value bit plus 2 - base.
        let m = if k == 64 { !0 } else { (1u64 << k) - 1 };
        let binary = base <= 2 && p[1] & m == m && (p[2] | p[3]) & m == 0;
        let zero = u64::from(2 - base.min(2)) * ONES;
        for j in (0..k.div_ceil(8)).rev() {
            let sh = 8 * j;
            let w = if binary {
                zero | spread(p[0] >> sh)
            } else {
                let w = spread(p[0] >> sh)
                    | spread(p[1] >> sh) << 1
                    | spread(p[2] >> sh) << 2
                    | spread(p[3] >> sh) << 3;
                ((w | 0x8080_8080_8080_8080) - sub) & 0x7f7f_7f7f_7f7f_7f7f
            };
            let e = c + sh;
            // Byte 0 of `r` is the highest element of the group.
            let r = w.swap_bytes().to_le_bytes();
            if k - sh >= 8 {
                dst[n - e - 8..n - e].copy_from_slice(&r);
            } else {
                let m = k - sh;
                dst[n - e - m..n - e].copy_from_slice(&r[8 - m..]);
            }
        }
        c += k;
    }
}

const CHUNK: usize = 64;

/// For the raw values `old` and `new` of equal size: the number of chunks of `CHUNK` bytes
/// that differ before each chunk, and in all as the last element. `None` if they differ in
/// shape.
fn chunk_counts(old: &CallVal, new: &Option<CallVal>) -> Option<Vec<u32>> {
    let (a, b): (&[u8], &[u8]) = match (old, new.as_ref()?) {
        (CallVal::Composite { data: a, .. }, CallVal::Composite { data: b, .. }) => (a, b),
        _ => return None,
    };
    if a.len() != b.len() {
        return None;
    }
    let mut cnt = Vec::with_capacity(a.len() / CHUNK + 2);
    cnt.push(0u32);
    for (x, y) in a.chunks(CHUNK).zip(b.chunks(CHUNK)) {
        cnt.push(cnt[cnt.len() - 1] + u32::from(x != y));
    }
    Some(cnt)
}

/// Make the plan of a variable at its first read: where each entry's value is in a raw
/// value of IR type `ty`, from the entries' paths. Later values have the same descriptor.
fn make_plan(
    paths: &mut Option<Paths>,
    plan: &mut Vec<DataLeaf>,
    plan_desc: &mut Vec<u8>,
    prog: &yavs_ir::Program,
    ty: yavs_ir::TyId,
    cv: &CallVal,
) -> Result<(), String> {
    let bug = |what: &str| format!("this is a bug in yavs: a recorded variable's value: {what}");
    let desc: &[u8] = match cv {
        CallVal::Composite { desc, .. } => desc,
        CallVal::Cell(_) => &[],
    };
    let Some(ps) = paths.take() else {
        return if desc == plan_desc.as_slice() {
            Ok(())
        } else {
            Err(bug("its bounds changed"))
        };
    };
    let mut start = 0;
    let slices: Vec<&[u32]> = ps
        .ends
        .iter()
        .map(|&e| {
            let sl = &ps.data[start..e as usize];
            start = e as usize;
            sl
        })
        .collect();
    *plan = leaf_locs(prog, ty, desc, &slices).ok_or_else(|| bug("it has another shape"))?;
    *plan_desc = desc.to_vec();
    Ok(())
}

impl Observer for Recorder {
    fn wants_phase(&self, phase: Phase) -> bool {
        matches!(
            phase,
            Phase::StartOfSim | Phase::NextTimeStep | Phase::EndOfTimeStep | Phase::EndOfSim
        )
    }

    fn phase(&mut self, hook: &mut Hook<'_>, phase: Phase) {
        let mut st = self.lock();
        match phase {
            Phase::StartOfSim => st.start(hook.now().as_fs()),
            Phase::NextTimeStep => {
                // The step in progress is complete: its signals, then its variables.
                st.flush(|r| hook.view(r));
                st.sample_vars(hook);
                st.step_done();
                st.step = hook.now().as_fs();
            }
            Phase::EndOfTimeStep => st.flush(|r| hook.view(r)),
            Phase::EndOfSim => {
                st.flush(|r| hook.view(r));
                st.sample_vars(hook);
                st.step_done();
            }
            Phase::EndOfInit
            | Phase::NextCycle
            | Phase::StartOfProcesses
            | Phase::EndOfProcesses
            | Phase::LastKnownDelta
            | Phase::StartOfPostponed => {}
        }
        if st.error.is_some() {
            hook.stop();
        }
    }

    fn value_change(&mut self, hook: &mut Hook<'_>, signal: yavs_kernel::SignalId, range: Range) {
        let mut st = self.lock();
        st.at = hook.now().as_fs();
        st.mark_range(signal.0, range);
    }

    fn value_changes(&mut self, hook: &mut Hook<'_>, signals: &[u32]) {
        let mut guard = self.lock();
        let st = &mut *guard;
        st.at = hook.now().as_fs();
        for &s in signals {
            // A leaf with one entry that covers it: any change is a change of the entry.
            let i = st.solo.get(s as usize).copied().unwrap_or(NO_ENTRY);
            if i != NO_ENTRY {
                if !st.dirty[i as usize] && !hook.changed(yavs_kernel::SignalId(s)).is_empty() {
                    st.dirty[i as usize] = true;
                    st.pending.push(i);
                }
                continue;
            }
            for &range in hook.changed(yavs_kernel::SignalId(s)) {
                st.mark_range(s, range);
            }
        }
    }
}

/// Walks the elaborated hierarchy into a [`HierarchyBuilder`].
struct Design<'a> {
    db: &'a Db,
    e: &'a Elaborated,
    b: HierarchyBuilder,
    entries: Vec<Entry>,
    enums: HashMap<yavs_sem::TypeId, Arc<EnumType>>,
    /// `Some` with `--wave-variables`.
    built: Option<&'a Built>,
    vars: Vec<Var>,
    /// The recorded shared variables of each package not yet declared, with their slots.
    pkg_vars: BTreeMap<yavs_sem::UnitId, Vec<(yavs_sem::DeclId, u64, yavs_ir::TyId)>>,
    /// The path of the variable sub-element being declared (`Paths`), and the paths of the
    /// variable's entries so far.
    vpath: Vec<u32>,
    paths: Paths,
    scalars: HashMap<ScalarKey, (Arc<SigType>, Conv)>,
    vectors: HashMap<VectorKey, (Arc<SigType>, Conv)>,
    /// With `alias`: the entry already declared for the same kernel elements, conversion and
    /// type (a port collapsed into its actual and the actual itself), which a second
    /// declaration then names again: the sink gets one change, an FST one value chain.
    alias: Option<HashMap<AliasKey, SignalId>>,
}

/// Kernel leaf, offset and length, conversion, type (by identity), object class.
type AliasKey = (u32, u32, u32, Conv, usize, ObjClass);

type ScalarKey = (
    yavs_sem::TypeId,
    Elem,
    bool,
    Option<(yavs_sem::SubtypeId, Option<(i64, i64, bool)>)>,
);
type VectorKey = (yavs_sem::TypeId, yavs_sem::TypeId, bool, (i64, i64, bool));

/// Where a node's values come from.
#[derive(Clone, Copy)]
enum At {
    /// Row-major instance `k` of the node in the leaves from `leaf` on; `part`: the kernel
    /// range of `leaf` a port collapsed into part of its actual's leaf covers
    /// (yavs-elab `SigObj::part`).
    Sig {
        leaf: u32,
        k: u64,
        part: Option<yavs_kernel::Range>,
    },
    /// A sub-element of a variable; its path is `Design::vpath`.
    Var,
}

impl Design<'_> {
    /// Element `j` of an array node of `n` elements; pair with [`Self::up_path`].
    fn elem(&mut self, at: At, n: u64, j: u64) -> At {
        match at {
            At::Sig { leaf, k, part } => At::Sig {
                leaf,
                k: k * n + j,
                part,
            },
            At::Var => {
                self.vpath.push(j as u32);
                At::Var
            }
        }
    }

    /// Field `i` of a record node, whose leaves start `skip` leaves after its own; pair with
    /// [`Self::up_path`].
    fn field(&mut self, at: At, i: u64, skip: u32) -> At {
        match at {
            At::Sig { leaf, k, part } => At::Sig {
                leaf: leaf + skip,
                k,
                part,
            },
            At::Var => {
                self.vpath.push(i as u32);
                At::Var
            }
        }
    }

    fn up_path(&mut self, at: At) {
        if matches!(at, At::Var) {
            self.vpath.pop();
        }
    }

    fn scope(&mut self, id: ScopeId) {
        let sc = self.e.hier.scope(id);
        // A component scope holds the component ports; its bound instance carries the label.
        let transparent = matches!(sc.kind, yavs_elab::ScopeKind::Component { .. });
        if !transparent {
            let (kind, comp) = match &sc.kind {
                yavs_elab::ScopeKind::Instance { entity, arch } => (
                    ScopeKind::Instance,
                    format!(
                        "{}({})",
                        self.db.units[*entity].name.text, self.db.units[*arch].name.text
                    ),
                ),
                yavs_elab::ScopeKind::Block => (ScopeKind::Block, String::new()),
                yavs_elab::ScopeKind::ForGenerate { .. } => (ScopeKind::ForGenerate, String::new()),
                yavs_elab::ScopeKind::Generate { .. } => (ScopeKind::Generate, String::new()),
                yavs_elab::ScopeKind::Process { .. } => (ScopeKind::Process, String::new()),
                // ponytail: FST has no library scope; a block keeps `lib.pkg.sig` paths.
                yavs_elab::ScopeKind::Library => (ScopeKind::Block, String::new()),
                yavs_elab::ScopeKind::Package { unit } => (
                    ScopeKind::Package,
                    self.db.units[*unit].name.text.to_string(),
                ),
                yavs_elab::ScopeKind::Component { .. } => unreachable!(),
            };
            let mut name = sc.name.text.to_string();
            if let Some(i) = &sc.index {
                name = format!("{name}({i})");
            }
            // The label of the chosen alternative stays in the name: `g.alt1`.
            if let yavs_elab::ScopeKind::Generate { alt: Some(a) } = &sc.kind {
                name = format!("{name}.{a}");
            }
            self.b.scope(&name, kind, &comp);
            for &o in &sc.objs {
                self.object(o);
            }
            match &sc.kind {
                yavs_elab::ScopeKind::Process { stmt } => self.variables(id, *stmt),
                yavs_elab::ScopeKind::Package { unit } => {
                    for (d, slot, ty) in self.pkg_vars.remove(unit).unwrap_or_default() {
                        self.variable(d, VarAt::Shared { slot, ty }, None);
                    }
                }
                _ => {}
            }
            self.shared_vars(id);
        }
        for &c in &sc.children {
            self.scope(c);
        }
        if let yavs_elab::ScopeKind::Library = sc.kind {
            let lib = self
                .db
                .units
                .iter()
                .map(|(_, u)| u.lib)
                .find(|&l| self.db.libs[l].name.text == sc.name.text);
            if let Some(lib) = lib {
                self.packages_of(lib);
            }
        }
        if !transparent {
            self.b.up();
        }
    }

    /// The recorded shared variables of packages, by package, before the walk (none without
    /// `--wave-variables`).
    fn package_vars(&mut self) {
        let Some(b) = self.built else { return };
        let db = self.db;
        let mut unit_of = HashMap::new();
        for (u, _) in db.units.iter() {
            for r in db.package_regions(u).unwrap_or_default() {
                unit_of.insert(r, u);
            }
        }
        let mut vs: Vec<_> = b
            .vars
            .iter()
            .filter(|((r, _), _)| r.is_none())
            .map(|(&(_, d), &(slot, ty))| (d, slot, ty))
            .collect();
        vs.sort_by_key(|v| v.0);
        for v in vs {
            // ponytail: a variable of a package instance nested in a package is left out;
            // walk the instance regions too if one is ever wanted.
            if let Some(&u) = unit_of.get(&db.decls[v.0].scope) {
                self.pkg_vars.entry(u).or_default().push(v);
            }
        }
    }

    /// Scopes for the packages of library `lib` with recorded shared variables that the
    /// design has no scope for (no signal of theirs is elaborated).
    fn packages_of(&mut self, lib: yavs_sem::LibId) {
        let us: Vec<_> = self
            .pkg_vars
            .keys()
            .copied()
            .filter(|&u| self.db.units[u].lib == lib)
            .collect();
        for u in us {
            let name = self.db.units[u].name.text.to_string();
            self.b.scope(&name, ScopeKind::Package, &name);
            for (d, slot, ty) in self.pkg_vars.remove(&u).unwrap_or_default() {
                self.variable(d, VarAt::Shared { slot, ty }, None);
            }
            self.b.up();
        }
    }

    /// The recorded shared variables of region `id`, in declaration order.
    fn shared_vars(&mut self, id: ScopeId) {
        let Some(b) = self.built else { return };
        let mut vs: Vec<_> = b
            .vars
            .iter()
            .filter(|((r, _), _)| *r == Some(id))
            .map(|(&(_, d), &(slot, ty))| (d, slot, ty))
            .collect();
        vs.sort_by_key(|v| v.0);
        for (d, slot, ty) in vs {
            self.variable(d, VarAt::Shared { slot, ty }, Some(id));
        }
    }

    fn object(&mut self, o: yavs_elab::ObjId) {
        let obj = self.e.nets.obj(o);
        let port = match obj.kind {
            ObjKind::Signal => PortDir::None,
            ObjKind::Port(m) => match m {
                PortMode::In => PortDir::In,
                PortMode::Out => PortDir::Out,
                PortMode::Inout => PortDir::InOut,
                PortMode::Buffer => PortDir::Buffer,
            },
            // Component ports are the hidden middle of an instance; implicit signals are not
            // declared objects.
            ObjKind::ComponentPort(_) | ObjKind::Implicit => return,
        };
        let at = At::Sig {
            leaf: obj.leaves.0.0,
            k: 0,
            part: obj.part,
        };
        self.node(
            &obj.shape,
            obj.name.text.as_str(),
            at,
            port,
            ObjClass::Signal,
        );
    }

    /// `shape` named `name`, its values from `at`.
    fn node(&mut self, shape: &Shape, name: &str, at: At, port: PortDir, class: ObjClass) {
        match shape {
            Shape::Scalar {
                ty,
                sub,
                range,
                elem,
                res,
                ..
            } => {
                let (sty, conv) = self.scalar(*ty, *elem, res.is_some(), Some((*sub, *range)));
                self.signal(name, &sty, port, class, at, 1, conv);
            }
            Shape::Array {
                ty,
                dims,
                elem,
                res: whole,
            } => {
                if let (
                    [d],
                    Shape::Scalar {
                        ty: et,
                        elem: Elem::Logic,
                        res,
                        ..
                    },
                ) = (dims.as_slice(), &**elem)
                    && Some(*et) != self.db.well_known.boolean
                {
                    if d.is_empty() {
                        return;
                    }
                    let (sty, conv) = self.vector(*ty, *et, res.is_some() || whole.is_some(), *d);
                    self.signal(name, &sty, port, class, at, d.len(), conv);
                    return;
                }
                let n = shape.len();
                if n == 0 {
                    return;
                }
                if self.b.array(name, n) {
                    let mut child = String::new();
                    for j in 0..n {
                        child.clear();
                        child.push_str(name);
                        child.push('(');
                        index_image(dims, j, &mut child);
                        child.push(')');
                        let at = self.elem(at, n, j);
                        self.node(elem, &child, at, port, class);
                        self.up_path(at);
                    }
                }
                self.b.up();
            }
            Shape::Record { fields, .. } => {
                self.b.scope(name, ScopeKind::Record, "");
                let mut skip = 0;
                for (i, (fname, f)) in fields.iter().enumerate() {
                    let at = self.field(at, i as u64, skip);
                    self.node(f, fname.text.as_str(), at, port, class);
                    self.up_path(at);
                    // Only kernel signals are addressed by leaf.
                    if matches!(at, At::Sig { .. }) {
                        skip += f.leaves().len() as u32;
                    }
                }
                self.b.up();
            }
        }
    }

    /// Declare signal `name`: for a signal over row-major elements `k*len .. (k+1)*len` of
    /// `leaf`. It gets an entry only if the selection keeps it, so unrecorded signals cost
    /// nothing at run time.
    #[allow(clippy::too_many_arguments)]
    fn signal(
        &mut self,
        name: &str,
        ty: &Arc<SigType>,
        port: PortDir,
        class: ObjClass,
        at: At,
        len: u64,
        conv: Conv,
    ) {
        let src = match at {
            At::Sig { leaf, k, part } => {
                let (base, n) = match part {
                    Some(r) => (r.offset as u64, r.len as u64),
                    None => (0, self.e.nets.leaves[leaf as usize].ty.len as u64),
                };
                let first = k * len;
                let offset = base
                    + match conv {
                        Conv::Logic { .. } | Conv::Bool => n - first - len,
                        _ => first,
                    };
                let (sig, r) = self.e.nets.kpart(
                    yavs_elab::LeafId(leaf),
                    Range::new(offset as u32, len as u32),
                );
                Src::Sig {
                    sig: sig.0,
                    offset: r.offset,
                    len: r.len,
                }
            }
            At::Var => Src::Var,
        };
        let id = SignalId(self.entries.len() as u32);
        let mut key = None;
        if let (Some(m), Src::Sig { sig, offset, len }) = (&self.alias, &src) {
            let kk = (*sig, *offset, *len, conv, Arc::as_ptr(ty) as usize, class);
            if let Some(&first) = m.get(&kk) {
                self.b.signal_shared(name, first, ty, port, class);
                return;
            }
            key = Some(kk);
        }
        if !self.b.signal_shared(name, id, ty, port, class) {
            return;
        }
        if let (Some(m), Some(kk)) = (&mut self.alias, key) {
            m.insert(kk, id);
        }
        if matches!(src, Src::Var) {
            self.paths.data.extend_from_slice(&self.vpath);
            self.paths.ends.push(self.paths.data.len() as u32);
        }
        self.entries.push(Entry { src, conv });
    }

    /// The variables of process `scope` (statement `stmt`).
    fn variables(&mut self, scope: ScopeId, stmt: yavs_sem::StmtId) {
        let (db, e) = (self.db, self.e);
        if self.built.is_none() {
            return;
        }
        let StmtKind::Process { scope: region, .. } = &db.stmts[stmt].kind else {
            return;
        };
        let Some(proc) = e
            .procs
            .iter()
            .position(|p| p.scope == scope && p.actual.is_none())
        else {
            return;
        };
        let parent = e.hier.scope(scope).parent;
        for &d in &db.scopes[*region].order {
            if matches!(&db.decls[d].kind, DeclKind::Object(o) if o.class == SemClass::Variable) {
                let at = VarAt::Proc {
                    proc: ProcId(proc as u32),
                    name: db.decls[d].name.to_string(),
                };
                self.variable(d, at, parent);
            }
        }
    }

    /// Variable `d` of region `region` (`None`: a package's), read at `at`, with the
    /// sub-elements the selection keeps.
    fn variable(&mut self, d: yavs_sem::DeclId, at: VarAt, region: Option<ScopeId>) {
        let name = self.db.decls[d].name.to_string();
        let Some(shape) = self.e.var_shapes.get(&(region, d)) else {
            // Not a scalar or composite (an access, a protected object), or its bounds are
            // known only when the simulation runs.
            if let DeclKind::Object(o) = &self.db.decls[d].kind {
                let k = &self.db.types[self.db.base(o.subtype)].kind;
                if k.is_scalar() || matches!(k, TypeKind::Array { .. } | TypeKind::Record { .. }) {
                    self.b.unknown(&name);
                }
            }
            return;
        };
        let first = self.entries.len() as u32;
        self.vpath.clear();
        self.paths = Paths::default();
        self.node(shape, &name, At::Var, PortDir::None, ObjClass::Variable);
        let last = self.entries.len() as u32;
        if last > first {
            self.vars.push(Var {
                at,
                entries: first..last,
                paths: Some(std::mem::take(&mut self.paths)),
                plan: Vec::new(),
                plan_desc: Vec::new(),
                last: Vec::new(),
                primed: false,
                execs: u64::MAX,
                raw: None,
                old: None,
                stamp: None,
            });
        }
    }

    /// [`Self::scalar_type`], made once per distinct arguments so that signals of one type
    /// share it.
    fn scalar(
        &mut self,
        t: yavs_sem::TypeId,
        elem: Elem,
        resolved: bool,
        decl: Option<(yavs_sem::SubtypeId, Option<yavs_elab::Bounds>)>,
    ) -> (Arc<SigType>, Conv) {
        let range = decl.map(|(st, r)| (st, r.map(|b| (b.left, b.right, b.dir == Direction::To))));
        let key = (t, elem, resolved, range);
        if let Some(c) = self.scalars.get(&key) {
            return c.clone();
        }
        let (ty, conv) = self.scalar_type(t, elem, resolved, decl);
        let c = (Arc::new(ty), conv);
        self.scalars.insert(key, c.clone());
        c
    }

    /// The type of a logic vector `ty` of `et` elements, made once per distinct arguments.
    fn vector(
        &mut self,
        ty: yavs_sem::TypeId,
        et: yavs_sem::TypeId,
        resolved: bool,
        d: yavs_elab::Bounds,
    ) -> (Arc<SigType>, Conv) {
        let key = (ty, et, resolved, (d.left, d.right, d.dir == Direction::To));
        if let Some(c) = self.vectors.get(&key) {
            return c.clone();
        }
        let (e, conv) = self.scalar(et, Elem::Logic, resolved, None);
        let SigType::Scalar(ScalarType::Enum(e)) = &*e else {
            unreachable!("logic scalars are enums")
        };
        let sty = SigType::Vector {
            elem: e.clone(),
            bounds: bounds(d),
            type_name: Some(self.vector_name(ty, resolved)),
        };
        let c = (Arc::new(sty), conv);
        self.vectors.insert(key, c.clone());
        c
    }

    /// `decl`: the declared subtype and its bounds, which name and size integers.
    fn scalar_type(
        &mut self,
        t: yavs_sem::TypeId,
        elem: Elem,
        resolved: bool,
        decl: Option<(yavs_sem::SubtypeId, Option<yavs_elab::Bounds>)>,
    ) -> (SigType, Conv) {
        let db = self.db;
        let td = &db.types[t];
        let tname = td.name.text.to_string();
        let sc = match &td.kind {
            TypeKind::Enum { lits } => {
                let e = self
                    .enums
                    .entry(t)
                    .or_insert_with(|| {
                        let lits: Vec<String> = lits
                            .iter()
                            .map(|&l| match db.decls[l].name {
                                Designator::Char(c) => format!("'{c}'"),
                                Designator::Ident(i) => i.text.to_string(),
                                Designator::Operator { text, .. } => text.to_string(),
                            })
                            .collect();
                        Arc::new(EnumType {
                            name: tname.clone(),
                            literals: lits,
                        })
                    })
                    .clone();
                let wk = &db.well_known;
                let conv = match elem {
                    Elem::Logic if Some(t) == wk.boolean => Conv::Bool,
                    Elem::Logic if Some(t) == wk.bit => Conv::Logic { base: 2 },
                    Elem::Logic => Conv::Logic { base: 0 },
                    // The writer takes an enum of logic characters (`'X' '0' '1' 'Z'`) as a
                    // logic scalar: one position byte, not the 4 bytes of an enum-table value.
                    Elem::Bytes(1) if e.logic_chars().is_some() => Conv::Raw,
                    Elem::Bytes(1) => Conv::Enum8,
                    Elem::Bytes(_) => Conv::Raw,
                };
                // A resolved std_ulogic is std_logic.
                let e = if resolved && matches!(conv, Conv::Logic { base: 0 }) {
                    Arc::new(EnumType {
                        name: "std_logic".into(),
                        literals: e.literals.clone(),
                    })
                } else {
                    e
                };
                return (SigType::Scalar(ScalarType::Enum(e)), conv);
            }
            TypeKind::Floating | TypeKind::UniversalReal => ScalarType::Real,
            TypeKind::Physical { .. } if Some(t) == db.well_known.time => ScalarType::Time,
            // The declared subtype's name and the width its range needs; 64 bits and the base
            // type name when there is neither (an anonymous subtype keeps the base name).
            _ => ScalarType::Integer {
                name: decl
                    .and_then(|(st, _)| subtype_name(db, st))
                    .unwrap_or(tname),
                bits: decl.and_then(|(_, r)| r).map_or(64, int_bits),
            },
        };
        let conv = match (&sc, elem) {
            (ScalarType::Integer { .. }, Elem::Bytes(4)) => Conv::Int32,
            _ => Conv::Raw,
        };
        (SigType::Scalar(sc), conv)
    }

    /// The type name of a logic vector as users write it: `std_logic_vector`, `unsigned`.
    fn vector_name(&self, t: yavs_sem::TypeId, resolved: bool) -> String {
        let n = self.db.types[t].name.text.to_string();
        let l = n.to_ascii_lowercase();
        match (resolved, l.as_str()) {
            (true, "std_ulogic_vector") => "std_logic_vector".into(),
            (true, _) if l.starts_with("unresolved_") => n["unresolved_".len()..].to_string(),
            _ => n,
        }
    }
}

/// The name of `st`, else of the nearest named subtype it was derived from (`natural range
/// 0 to 15` is a `natural`).
fn subtype_name(db: &Db, mut st: yavs_sem::SubtypeId) -> Option<String> {
    loop {
        if let Some(n) = db.subtypes[st].name {
            return Some(n.text.to_string());
        }
        st = db.subtypes[st].parent?;
    }
}

/// The width of the two's complement encoding that holds every value of `b`.
fn int_bits(b: yavs_elab::Bounds) -> u32 {
    let (lo, hi) = b.low_high();
    if hi < lo {
        return 1;
    }
    // `!lo` is -lo - 1, the largest magnitude a negative value needs; a non-negative range
    // keeps a zero sign bit (0 to 31 is 6 bits), or a reader of two's complement shows 31 as -1.
    let mag = (if lo < 0 { !lo as u64 } else { 0 }).max(hi.max(0) as u64);
    65 - mag.leading_zeros()
}

fn bounds(d: yavs_elab::Bounds) -> Bounds {
    Bounds {
        left: d.left,
        right: d.right,
        dir: match d.dir {
            Direction::To => Dir::To,
            Direction::Downto => Dir::Downto,
        },
    }
}

/// `3` or `1,2`: the index values of row-major element `j` of an array with `dims`, appended
/// to `out`.
fn index_image(dims: &[yavs_elab::Bounds], mut j: u64, out: &mut String) {
    use std::fmt::Write;
    let mut ix = [0i64; 8];
    let mut spill = Vec::new();
    let ix: &mut [i64] = if dims.len() <= ix.len() {
        &mut ix[..dims.len()]
    } else {
        spill.resize(dims.len(), 0);
        &mut spill
    };
    for (d, slot) in dims.iter().zip(ix.iter_mut()).rev() {
        let n = d.len();
        let p = (j % n) as i64;
        j /= n;
        *slot = match d.dir {
            Direction::To => d.left + p,
            Direction::Downto => d.left - p,
        };
    }
    for (k, x) in ix.iter().enumerate() {
        if k > 0 {
            out.push(',');
        }
        let _ = write!(out, "{x}");
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn int_bits_is_twos_complement_width() {
        use super::Direction::To;
        use yavs_elab::Bounds;
        let w = |lo, hi| {
            super::int_bits(Bounds {
                left: lo,
                right: hi,
                dir: To,
            })
        };
        assert_eq!(
            (w(0, 31), w(0, 0), w(0, 1), w(-8, 7), w(-9, 7), w(5, 4)),
            (6, 1, 2, 4, 5, 1)
        );
        assert_eq!((w(0, i64::MAX), w(i64::MIN, i64::MAX)), (64, 64));
    }

    #[test]
    fn spread_puts_bit_j_in_byte_j() {
        for b in 0..256u64 {
            let want: u64 = (0..8).map(|j| (b >> j & 1) << (8 * j)).sum();
            assert_eq!(super::spread(b), want, "{b:#x}");
            assert_eq!(super::spread(b | 0xabcd00), want, "{b:#x} with upper bits");
        }
    }
}
