//! Signal storage, drivers and projected output waveforms.
//!
//! A signal is one contiguous array of `len` elements (see [`crate::SignalType`]). Everything
//! that addresses part of a signal (drivers, events, forces, ports, waiters, value-change
//! events) uses one [`Range`] of element indices (design rule 2).

use std::sync::Arc;

use smallvec::{SmallVec, smallvec};

use crate::error::ErrorKind;
use crate::time::Time;
use crate::value::{Buf, SignalType, bits, fill_bits};

/// Stable id of a signal: its index in creation (elaboration) order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SignalId(pub u32);

/// Stable id of a driver.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DriverId(pub u32);

/// A contiguous run of elements: `offset .. offset + len`. May be empty (null ranges).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct Range {
    pub offset: u32,
    pub len: u32,
}

impl Range {
    pub const fn new(offset: u32, len: u32) -> Range {
        Range { offset, len }
    }

    pub const fn end(self) -> u32 {
        self.offset + self.len
    }

    /// Share at least one element.
    pub const fn overlaps(self, o: Range) -> bool {
        self.len > 0 && o.len > 0 && self.offset < o.end() && o.offset < self.end()
    }

    pub const fn contains(self, o: Range) -> bool {
        self.offset <= o.offset && o.end() <= self.end()
    }

    pub fn intersect(self, o: Range) -> Option<Range> {
        let start = self.offset.max(o.offset);
        let end = self.end().min(o.end());
        (start < end).then(|| Range::new(start, end - start))
    }

    pub(crate) fn elems(self) -> std::ops::Range<usize> {
        self.offset as usize..self.end() as usize
    }
}

/// A whole signal or a sub-element range of it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SigRef {
    pub signal: SignalId,
    pub range: Option<Range>,
}

impl SignalId {
    /// Elements `offset .. offset + len` of this signal.
    pub const fn slice(self, offset: u32, len: u32) -> SigRef {
        SigRef {
            signal: self,
            range: Some(Range::new(offset, len)),
        }
    }
}

impl From<SignalId> for SigRef {
    fn from(signal: SignalId) -> SigRef {
        SigRef {
            signal,
            range: None,
        }
    }
}

/// A user resolution function over one segment of a resolved signal, one byte per element
/// (logic signals pass `std_ulogic` codes). `sources[i]` holds the values of the i-th
/// connected source (drivers in creation order, then ports); `out` receives the resolved
/// values. An `Err` stops the simulation. For a composite resolved as a whole
/// (`Builder::resolved_composite`) each value is the concatenation of all its leaves, in
/// leaf order, a logic leaf one code byte per element, a byte leaf its bytes.
pub type Resolver = Arc<dyn Fn(&[&[u8]], &mut [u8]) -> Result<(), String> + Send + Sync>;

/// How a resolved signal combines its sources.
#[derive(Clone)]
pub enum Resolution {
    /// IEEE 1164 `resolved`, plane-wise; logic signals only.
    StdLogic,
    Func(Resolver),
}

/// Kind of a guarded signal (LRM 6.4.2.3): with every driver disconnected, a `bus` resolves
/// an empty source list, a `register` keeps its value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum GuardKind {
    Bus,
    Register,
}

/// Force mode (LRM 10.5.2.1): `force in` overrides the effective value, `force out` the
/// driving value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ForceMode {
    Effective,
    Driving,
}

/// An override layer over a whole signal: elements whose mask bit is set take `value`.
pub(crate) struct Layer {
    pub mask: Vec<u64>,
    pub value: Buf,
}

pub(crate) struct DriverSlot {
    pub range: Range,
    /// Current value of the driver (or the converted driving value of a port).
    pub value: Buf,
    /// Disconnected elements (null transaction), relative to `range`; guarded signals only.
    pub off: Option<Vec<u64>>,
}

/// Elements with the same set of sources.
pub(crate) struct Segment {
    pub range: Range,
    /// Indices into `Signal::drivers`.
    pub drivers: SmallVec<[u32; 2]>,
}

pub(crate) struct Waiter {
    pub proc: u32,
    pub range: Range,
    pub generation: u64,
    /// Registration number on its signal: the order in which waiters wake.
    seq: u64,
}

/// Elements per waiter bucket, and the most buckets a waiter is copied into before it goes
/// to the wide list instead.
const WAIT_CHUNK: u32 = 64;
const WAIT_NARROW: u32 = 4;

/// The processes waiting on one signal, bucketed by chunk of `WAIT_CHUNK` elements, so an
/// event looks only at the waiters of the chunks it touches: one process per element of a
/// wide vector costs O(1) per event, not O(processes). Waiters over more than `WAIT_NARROW`
/// chunks (or none) sit in a wide list that every event scans. Each list keeps its count
/// right after the last compaction, so lists of signals that rarely change do not grow
/// without bound.
#[derive(Default)]
pub(crate) struct Waiters {
    /// Per chunk, then the wide list last.
    lists: SmallVec<[(SmallVec<[Waiter; 1]>, usize); 1]>,
    next: u64,
}

impl Waiters {
    fn new(len: u32) -> Waiters {
        // A signal of one chunk or less has one list: the chunk's and the wide one.
        let chunks = len.div_ceil(WAIT_CHUNK) as usize;
        let n = if chunks <= 1 { 1 } else { chunks + 1 };
        Waiters {
            lists: (0..n).map(|_| (SmallVec::new(), 0)).collect(),
            next: 0,
        }
    }

    fn lists_of(&self, r: Range) -> std::ops::Range<usize> {
        let wide = self.lists.len() - 1;
        if wide == 0 {
            return 0..1;
        }
        if r.len == 0 {
            return wide..wide + 1;
        }
        let (a, b) = (r.offset / WAIT_CHUNK, (r.end() - 1) / WAIT_CHUNK);
        if b - a >= WAIT_NARROW {
            return wide..wide + 1;
        }
        a as usize..b as usize + 1
    }

    pub fn add(
        &mut self,
        proc: u32,
        range: Range,
        generation: u64,
        live: impl Fn(&Waiter) -> bool,
    ) {
        let seq = self.next;
        self.next += 1;
        for i in self.lists_of(range) {
            let (l, n) = &mut self.lists[i];
            if l.len() >= 2 * *n + 16 {
                l.retain(|w| live(w));
                *n = l.len();
            }
            l.push(Waiter {
                proc,
                range,
                generation,
                seq,
            });
        }
    }

    /// Remove the live waiters overlapping `runs` (sorted, disjoint) and put their
    /// `(registration, process)` in `out`, in registration order, each waiter once; drop
    /// stale ones on the way. Waiters for which `hold` is true stay, not woken.
    pub fn take_woken(
        &mut self,
        runs: &[Range],
        live: impl Fn(&Waiter) -> bool,
        hold: impl Fn(&Waiter) -> bool,
        out: &mut Vec<(u64, u32)>,
    ) {
        let hit = |r: Range| {
            let i = runs.partition_point(|x| x.end() <= r.offset);
            runs.get(i).is_some_and(|x| x.overlaps(r))
        };
        out.clear();
        let wide = self.lists.len() - 1;
        let mut scanned = 0;
        let mut last = usize::MAX;
        let chunks = runs.iter().flat_map(|r| {
            (r.len > 0)
                .then(|| r.offset / WAIT_CHUNK..=(r.end() - 1) / WAIT_CHUNK)
                .into_iter()
                .flatten()
        });
        for i in chunks.map(|c| c as usize).chain([wide]) {
            if i == last {
                continue;
            }
            last = i;
            scanned += 1;
            let (l, n) = &mut self.lists[i];
            l.retain(|w| {
                if !live(w) {
                    return false;
                }
                if hit(w.range) && !hold(w) {
                    out.push((w.seq, w.proc));
                    return false;
                }
                true
            });
            *n = l.len();
        }
        if scanned > 1 {
            out.sort_unstable();
            out.dedup_by_key(|w| w.0);
        }
    }
}

pub(crate) struct Signal {
    pub name: String,
    pub ty: SignalType,
    /// Effective (current) value.
    pub cur: Buf,
    /// Driving value computed from the sources or a deposit, before forces.
    pub driving: Buf,
    pub last_value: Buf,
    /// Four per-element arrays in one buffer (`len` entries each): the cycle number of the
    /// last event, of the last activity, and the time (`Time::raw`, [`NEVER`] if never) of
    /// the last event and of the last activity.
    stamps: SmallVec<[u64; 4]>,
    /// Sources: process drivers and out-ports, in creation order.
    pub drivers: Vec<DriverSlot>,
    pub segments: SmallVec<[Segment; 1]>,
    pub resolution: Option<Resolution>,
    pub guard: Option<GuardKind>,
    /// Indices into the kernel's composite groups covering this signal: resolved together
    /// with parts of other leaves.
    pub groups: Vec<u32>,
    /// Disconnection time from a disconnection specification (default 0).
    pub disconnect: Time,
    pub force_eff: Option<Box<Layer>>,
    pub force_drv: Option<Box<Layer>>,
    /// Effective value taken from the actual of an `in`/`inout` port.
    pub port_in: Option<Box<Layer>>,
    /// Port depth: an actual has a lower level than its formals.
    pub level: u32,
    /// Cycle in which the signal was last put on the driving / effective worklist.
    pub queued_up: u64,
    pub queued_down: u64,
    pub waiters: Waiters,
    /// Elements changed in the current cycle.
    pub changed: SmallVec<[Range; 2]>,
    /// Ranges whose driving value must be recomputed this cycle.
    pub dirty: SmallVec<[Range; 2]>,
    /// Ranges whose effective value must be recomputed this cycle.
    pub eff_dirty: SmallVec<[Range; 2]>,
    /// The simulation runs (not initialising): reports of its resolution function are
    /// collected in `callouts` for the kernel to commit (#372).
    pub live: bool,
    /// Ranges where a force or release of this cycle changed the effective value, whatever
    /// the cycle's last one left: an event even when the value ends up unchanged (#382).
    pub force_ev: SmallVec<[Range; 2]>,
    pub callouts: Vec<(crate::process::Severity, String)>,
    /// An assignment of the value a source already has may be skipped (`Builder::build`).
    pub skip_ok: bool,
}

impl Signal {
    /// Every element of `r` has exactly one source.
    pub fn sole(&self, r: Range) -> bool {
        let i = self.segments.partition_point(|s| s.range.end() <= r.offset);
        (self.segments.get(i)).is_some_and(|s| s.range.contains(r) && s.drivers.len() == 1)
    }

    pub fn new(name: String, ty: SignalType, init: Buf, resolution: Option<Resolution>) -> Signal {
        let n = ty.len as usize;
        Signal {
            name,
            ty,
            cur: init.clone(),
            driving: init.clone(),
            last_value: init,
            stamps: stamps(n),
            drivers: Vec::new(),
            segments: SmallVec::new(),
            resolution,
            guard: None,
            groups: Vec::new(),
            disconnect: Time::ZERO,
            force_eff: None,
            force_drv: None,
            port_in: None,
            level: 0,
            queued_up: 0,
            queued_down: 0,
            waiters: Waiters::new(ty.len),
            changed: SmallVec::new(),
            dirty: SmallVec::new(),
            eff_dirty: SmallVec::new(),
            live: false,
            force_ev: SmallVec::new(),
            callouts: Vec::new(),
            skip_ok: false,
        }
    }

    pub fn full(&self) -> Range {
        Range::new(0, self.ty.len)
    }

    pub fn resolve(&self, r: Option<Range>) -> Range {
        r.unwrap_or(self.full())
    }

    /// The layer for `mode`, created on first use.
    pub fn layer<'a>(slot: &'a mut Option<Box<Layer>>, proto: &Buf) -> &'a mut Layer {
        slot.get_or_insert_with(|| {
            Box::new(Layer {
                mask: vec![0; proto.len().div_ceil(64)],
                value: proto.clone(),
            })
        })
    }

    /// Partition the signal into segments with identical source sets.
    pub fn build_segments(&mut self) {
        let mut cuts = vec![0, self.ty.len];
        for d in &self.drivers {
            cuts.push(d.range.offset);
            cuts.push(d.range.end());
        }
        cuts.sort_unstable();
        cuts.dedup();
        // Sweep the cuts with the set of sources covering them (kept in source order), not a
        // scan over all sources per segment: one source per element stays O(n log n).
        let mut by_start: Vec<u32> = (0..self.drivers.len() as u32)
            .filter(|&i| self.drivers[i as usize].range.len > 0)
            .collect();
        by_start.sort_by_key(|&i| self.drivers[i as usize].range.offset);
        let mut next = by_start.iter().peekable();
        let mut active = std::collections::BTreeSet::new();
        self.segments = SmallVec::with_capacity(cuts.len().saturating_sub(1));
        for w in cuts.windows(2) {
            let range = Range::new(w[0], w[1] - w[0]);
            active.retain(|&i: &u32| self.drivers[i as usize].range.end() > w[0]);
            while let Some(&&i) = next.peek() {
                if self.drivers[i as usize].range.offset > w[0] {
                    break;
                }
                active.insert(i);
                next.next();
            }
            let drivers: SmallVec<[u32; 2]> = active.iter().copied().collect();
            self.segments.push(Segment { range, drivers });
        }
    }

    /// Recompute the driving value of `r` from the sources. A leaf of a composite group is
    /// resolved by the kernel with its group (`SimControl::resolve_group`).
    pub fn compute_driving(&mut self, r: Range) -> Result<(), ErrorKind> {
        if !self.groups.is_empty() {
            return Ok(());
        }
        // Segments are sorted and disjoint: start at the first one that reaches `r`, so a
        // signal with one driver per element costs O(log n) per dirty range, not O(n).
        let first = self.segments.partition_point(|s| s.range.end() <= r.offset);
        for seg in &self.segments[first..] {
            if seg.range.offset >= r.end() {
                break;
            }
            let Some(part) = seg.range.intersect(r) else {
                continue;
            };
            match (&self.resolution, seg.drivers.as_slice()) {
                (_, []) => {}
                // The common case: one unguarded source, no allocation.
                (None, [i]) => {
                    let d = &self.drivers[*i as usize];
                    let rel = (part.offset - d.range.offset) as usize;
                    self.driving
                        .copy(part.offset as usize, &d.value, rel, part.len as usize);
                }
                (None, _) => {
                    let msg = format!("unresolved signal {} has several sources", self.name);
                    return Err(ErrorKind::Internal(msg));
                }
                (Some(res), ds) => {
                    // An unguarded signal has the one run of all its sources: no list of runs.
                    if self.guard.is_none() {
                        resolve_run(
                            &self.drivers,
                            ResOut {
                                driving: &mut self.driving,
                                callouts: &mut self.callouts,
                                name: &self.name,
                                live: self.live,
                            },
                            res,
                            part,
                            ds,
                        )?;
                        continue;
                    }
                    for (run, conn) in connected_runs(&self.drivers, true, part, ds) {
                        if conn.is_empty() && self.guard == Some(GuardKind::Register) {
                            continue;
                        }
                        resolve_run(
                            &self.drivers,
                            ResOut {
                                driving: &mut self.driving,
                                callouts: &mut self.callouts,
                                name: &self.name,
                                live: self.live,
                            },
                            res,
                            run,
                            &conn,
                        )?;
                    }
                }
            }
        }
        Ok(())
    }

    /// The effective value of `r`: driving value under `force out`, port input and
    /// `force in`, in increasing priority.
    pub fn effective_of(&self, r: Range) -> Buf {
        let mut v = self.driving.extract(r.offset as usize, r.len as usize);
        for l in [&self.force_drv, &self.port_in, &self.force_eff]
            .into_iter()
            .flatten()
        {
            v.select(
                0,
                &l.value,
                r.offset as usize,
                r.len as usize,
                &l.mask,
                Some(r.offset as usize),
            );
        }
        v
    }

    /// The driving value of `r` after `force out` (what a port passes to its actual).
    pub fn driving_of(&self, r: Range) -> Buf {
        let mut v = self.driving.extract(r.offset as usize, r.len as usize);
        if let Some(l) = &self.force_drv {
            v.select(
                0,
                &l.value,
                r.offset as usize,
                r.len as usize,
                &l.mask,
                Some(r.offset as usize),
            );
        }
        v
    }

    /// Update the effective value of `r`, recording events. Returns whether anything changed.
    pub fn update_effective(&mut self, r: Range, now: Time, cycle: u64) -> bool {
        let (off, n) = (r.offset as usize, r.len as usize);
        let layered =
            self.force_drv.is_some() || self.port_in.is_some() || self.force_eff.is_some();
        let owned;
        let (src, soff) = if layered {
            owned = self.effective_of(r);
            (&owned, 0)
        } else {
            (&self.driving, off)
        };
        let mut any = false;
        for c in (0..n).step_by(64) {
            let k = (n - c).min(64);
            let m = self.cur.diff_chunk(off + c, src, soff + c, k);
            if m == 0 {
                continue;
            }
            any = true;
            let mut w = m;
            while w != 0 {
                let e = off + c + w.trailing_zeros() as usize;
                w &= w - 1;
                set_event(&mut self.stamps, self.ty.len as usize, e, cycle, now);
                match self.changed.last_mut() {
                    Some(run) if run.end() as usize == e => run.len += 1,
                    _ => self.changed.push(Range::new(e as u32, 1)),
                }
            }
            self.last_value
                .select(off + c, &self.cur, off + c, k, &[m], Some(0));
            self.cur.select(off + c, src, soff + c, k, &[m], Some(0));
        }
        if !self.force_ev.is_empty() {
            any |= self.force_events(r, now, cycle);
        }
        any
    }

    /// Events for the elements of `r` an earlier force of this cycle changed, if the cycle's
    /// last force changed them back.
    fn force_events(&mut self, r: Range, now: Time, cycle: u64) -> bool {
        let mut any = false;
        for f in std::mem::take(&mut self.force_ev) {
            let Some(part) = f.intersect(r) else {
                self.force_ev.push(f);
                continue;
            };
            for e in part.elems() {
                if self.event_cycles()[e] == cycle {
                    continue;
                }
                any = true;
                set_event(&mut self.stamps, self.ty.len as usize, e, cycle, now);
                match self.changed.last_mut() {
                    Some(run) if run.end() as usize == e => run.len += 1,
                    _ => self.changed.push(Range::new(e as u32, 1)),
                }
            }
        }
        any
    }

    pub fn mark_active(&mut self, r: Range, now: Time, cycle: u64) {
        let n = self.ty.len as usize;
        for e in r.elems() {
            self.stamps[n + e] = cycle;
            self.stamps[3 * n + e] = now.raw();
        }
    }

    fn event_cycles(&self) -> &[u64] {
        &self.stamps[..self.ty.len as usize]
    }

    pub fn active_in(&self, r: Range, cycle: u64) -> bool {
        let n = self.ty.len as usize;
        self.stamps[n..2 * n][r.elems()].contains(&cycle)
    }

    pub fn event_in(&self, r: Range, cycle: u64) -> bool {
        self.event_cycles()[r.elems()].contains(&cycle)
    }

    /// `'last_event` of `r`: time since the most recent event, `TIME'HIGH` if none.
    pub fn last_event(&self, r: Range, now: Time) -> Time {
        let n = self.ty.len as usize;
        elapsed(now, &self.stamps[2 * n..3 * n][r.elems()])
    }

    /// `'last_active` of `r`.
    pub fn last_active(&self, r: Range, now: Time) -> Time {
        let n = self.ty.len as usize;
        elapsed(now, &self.stamps[3 * n..][r.elems()])
    }
}

/// The stamp of an element that never had an event or activity.
const NEVER: u64 = u64::MAX;

fn set_event(stamps: &mut [u64], n: usize, e: usize, cycle: u64, now: Time) {
    stamps[e] = cycle;
    stamps[2 * n + e] = now.raw();
}

fn stamps(n: usize) -> SmallVec<[u64; 4]> {
    let mut v: SmallVec<[u64; 4]> = smallvec![0; 4 * n];
    v[2 * n..].fill(NEVER);
    v
}

fn elapsed(now: Time, times: &[u64]) -> Time {
    times
        .iter()
        .filter(|&&t| t != NEVER)
        .max()
        .map_or(Time::MAX, |&t| now.since(Time::from_raw(t)))
}

/// What resolving a run writes, borrowed apart from the sources it reads.
struct ResOut<'a> {
    driving: &'a mut Buf,
    callouts: &'a mut Vec<(crate::process::Severity, String)>,
    name: &'a str,
    live: bool,
}

/// Resolve the elements of `run` from the sources `conn` into the driving value.
fn resolve_run(
    drivers: &[DriverSlot],
    out: ResOut<'_>,
    res: &Resolution,
    run: Range,
    conn: &[u32],
) -> Result<(), ErrorKind> {
    let ResOut {
        driving,
        callouts,
        name,
        live,
    } = out;
    let srcs: SmallVec<[(&Buf, usize); 4]> = conn
        .iter()
        .map(|&i| {
            let d = &drivers[i as usize];
            (&d.value, (run.offset - d.range.offset) as usize)
        })
        .collect();
    let (off, n) = (run.offset as usize, run.len as usize);
    match res {
        Resolution::StdLogic => driving.resolve_std_logic(off, n, &srcs),
        Resolution::Func(f) => {
            let codes: Vec<Vec<u8>> = srcs.iter().map(|(b, o)| b.codes(*o, n)).collect();
            let views: Vec<&[u8]> = codes.iter().map(Vec::as_slice).collect();
            let mut out = driving.codes(off, n);
            let r = if live {
                let (r, reps) = crate::callout::capture(|| f(&views, &mut out));
                callouts.extend(reps);
                r
            } else {
                f(&views, &mut out)
            };
            r.map_err(|message| ErrorKind::Resolution {
                signal: name.to_string(),
                message,
            })?;
            driving.set_codes(off, &out);
        }
    }
    Ok(())
}

/// Runs of `part` with a constant set of connected sources (all of `sources` when the
/// signal is not guarded).
fn connected_runs(
    drivers: &[DriverSlot],
    guarded: bool,
    part: Range,
    sources: &[u32],
) -> Vec<(Range, Vec<u32>)> {
    if !guarded {
        return vec![(part, sources.to_vec())];
    }
    let on = |i: u32, e: usize| {
        let d = &drivers[i as usize];
        d.off
            .as_ref()
            .is_none_or(|off| bits(off, e - d.range.offset as usize, 1) == 0)
    };
    let mut runs: Vec<(Range, Vec<u32>)> = Vec::new();
    for e in part.elems() {
        let set: Vec<u32> = sources.iter().copied().filter(|&i| on(i, e)).collect();
        match runs.last_mut() {
            Some((r, s)) if *s == set => r.len += 1,
            _ => runs.push((Range::new(e as u32, 1), set)),
        }
    }
    runs
}

/// Apply a matured transaction to a driver slot. A value covering the whole driver is
/// swapped in rather than copied; the buffer left over goes back to `spare`.
fn apply_txn(d: &mut DriverSlot, t: Txn, spare: &mut Vec<Buf>) {
    let rel = (t.range.offset - d.range.offset) as usize;
    match t.value {
        Some(mut v) => {
            if t.range == d.range && v.len() == d.value.len() {
                std::mem::swap(&mut d.value, &mut v);
            } else {
                d.value.copy(rel, &v, 0, t.range.len as usize);
            }
            // A value stored inline costs nothing to make again.
            if v.spilled() && spare.len() < SPARE_BUFS {
                spare.push(v);
            }
            if let Some(off) = &mut d.off {
                fill_bits(off, rel, t.range.len as usize, false);
            }
        }
        None => {
            if let Some(off) = &mut d.off {
                fill_bits(off, rel, t.range.len as usize, true);
            }
        }
    }
}

/// One transaction of a projected output waveform; `value: None` is a null transaction.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Txn {
    pub time: Time,
    /// Elements of the signal (not of the driver).
    pub range: Range,
    // ponytail: one heap allocation per transaction; pool per driver if profiles show it.
    pub value: Option<Buf>,
}

/// The transactions of one driver or one assignment: nearly always one.
pub(crate) type TxnList = SmallVec<[Txn; 1]>;

/// How the first element of a waveform preempts pending transactions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum DelayMode {
    Transport,
    /// Inertial with a pulse rejection limit; `None` uses the first delay.
    Inertial {
        reject: Option<Time>,
    },
}

impl DelayMode {
    pub const INERTIAL: DelayMode = DelayMode::Inertial { reject: None };
}

/// Transaction buffers a driver keeps for reuse.
const SPARE_BUFS: usize = 4;

/// The projected output waveform of one driver, sorted by time.
#[derive(Default)]
pub(crate) struct Waveform {
    pub pending: TxnList,
    /// Values of applied transactions, reused by the next assignments (no allocation per
    /// transaction in the steady state).
    pub spare: Vec<Buf>,
    /// Zero-delay assignments of this run that changed nothing and were not queued
    /// (`Ctx::assign`, `Config::skip_unchanged`).
    pub phantom: Vec<Range>,
}

/// Per element of `t` (relative), whether its value equals `first` (covering `r`).
fn same_value(t: &Txn, first: &Option<Buf>, r: Range) -> Vec<u64> {
    let n = t.range.len as usize;
    let mut eq = vec![0u64; n.div_ceil(64)];
    let Some(ov) = t.range.intersect(r) else {
        return eq;
    };
    let (to, fo, k) = (
        (ov.offset - t.range.offset) as usize,
        (ov.offset - r.offset) as usize,
        ov.len as usize,
    );
    match (&t.value, first) {
        (None, None) => fill_bits(&mut eq, to, k, true),
        (Some(a), Some(b)) => {
            let d = a.diff(to, b, fo, k);
            for i in 0..k {
                if bits(&d, i, 1) == 0 {
                    eq[(to + i) / 64] |= 1 << ((to + i) % 64);
                }
            }
        }
        _ => {}
    }
    eq
}

impl Waveform {
    /// Apply LRM 10.5.2.2 to target elements `r` for a new waveform whose first transaction is
    /// at `t1`, then insert the new transactions. `reject_from` is `t1 - reject` for inertial
    /// delay, `None` for transport.
    pub fn schedule(&mut self, r: Range, reject_from: Option<Time>, new: &mut TxnList) {
        // Nothing pending on these elements (the usual zero-delay case): nothing to preempt.
        if !self.pending.iter().any(|t| t.range.overlaps(r)) {
            for t in new.drain(..) {
                let at = self.pending.partition_point(|p| p.time <= t.time);
                self.pending.insert(at, t);
            }
            return;
        }
        let t1 = new[0].time;
        let first = &new[0].value;
        let in_window = |t: &Txn| reject_from.is_some_and(|from| t.time >= from && t.time < t1);
        let eqs: Vec<Option<Vec<u64>>> = self
            .pending
            .iter()
            .map(|t| (in_window(t) && t.range.overlaps(r)).then(|| same_value(t, first, r)))
            .collect();
        // Per element, decide which overlapping old transactions to delete.
        // ponytail: O(elements * pending); fine while waveforms stay short.
        let mut deleted: Vec<Option<Vec<u64>>> = vec![None; self.pending.len()];
        for e in r.elems() {
            let mut chain = true;
            for (i, t) in self.pending.iter().enumerate().rev() {
                if !t.range.elems().contains(&e) {
                    continue;
                }
                let k = e - t.range.offset as usize;
                let delete = if t.time >= t1 {
                    true
                } else if let Some(eq) = &eqs[i] {
                    chain &= bits(eq, k, 1) == 1;
                    !chain
                } else {
                    false
                };
                if delete {
                    let mask = deleted[i]
                        .get_or_insert_with(|| vec![0; (t.range.len as usize).div_ceil(64)]);
                    fill_bits(mask, k, 1, true);
                }
            }
        }
        if deleted.iter().any(Option::is_some) {
            let old = std::mem::take(&mut self.pending);
            for (t, del) in old.into_iter().zip(deleted) {
                match del {
                    None => self.pending.push(t),
                    Some(mask) => split_kept(t, &mask, &mut self.pending),
                }
            }
        }
        for t in new.drain(..) {
            let at = self.pending.partition_point(|p| p.time <= t.time);
            self.pending.insert(at, t);
        }
    }

    /// Apply the transactions due at or before `now` to driver slot `d`, in order, calling
    /// `applied` with each one's elements.
    pub fn apply_due(&mut self, now: Time, d: &mut DriverSlot, mut applied: impl FnMut(Range)) {
        let n = self.pending.partition_point(|t| t.time <= now);
        for t in self.pending.drain(..n) {
            applied(t.range);
            apply_txn(d, t, &mut self.spare);
        }
    }
}

/// Push the runs of `t` not marked in `mask`.
fn split_kept(t: Txn, mask: &[u64], out: &mut TxnList) {
    let n = t.range.len as usize;
    let mut start = None;
    for k in 0..=n {
        let keep = k < n && bits(mask, k, 1) == 0;
        match (keep, start) {
            (true, None) => start = Some(k),
            (false, Some(s)) => {
                out.push(Txn {
                    time: t.time,
                    range: Range::new(t.range.offset + s as u32, (k - s) as u32),
                    value: t.value.as_ref().map(|v| v.extract(s, k - s)),
                });
                start = None;
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::value::{Elem, LogicVec};

    fn txn(ns: u64, off: u32, v: &[u8]) -> Txn {
        let value = Some(Buf::from_ref(Elem::Bytes(1), v.len(), v.into()).unwrap());
        Txn {
            time: Time::ns(ns),
            range: Range::new(off, v.len() as u32),
            value,
        }
    }

    #[test]
    fn segments_list_covering_sources_in_order() {
        let ty = SignalType::bytes(1, 10);
        let mut sig = Signal::new("s".into(), ty, Buf::zeroed(ty.elem, 10), None);
        for (o, l) in [(0, 10), (5, 3), (2, 1), (4, 0), (8, 2)] {
            sig.drivers.push(DriverSlot {
                range: Range::new(o, l),
                value: Buf::zeroed(ty.elem, l as usize),
                off: None,
            });
        }
        sig.build_segments();
        let got: Vec<_> = sig
            .segments
            .iter()
            .map(|g| (g.range.offset, g.range.len, g.drivers.to_vec()))
            .collect();
        let want = vec![
            (0, 2, vec![0]),
            (2, 1, vec![0, 2]),
            (3, 1, vec![0]),
            (4, 1, vec![0]),
            (5, 3, vec![0, 1]),
            (8, 2, vec![0, 4]),
        ];
        assert_eq!(got, want);
    }

    #[test]
    fn waiters_wake_once_in_registration_order() {
        let all = |_: &Waiter| true;
        let none = |_: &Waiter| false;
        let mut w = Waiters::new(300);
        w.add(1, Range::new(200, 1), 0, all); // chunk 3
        w.add(2, Range::new(0, 300), 0, all); // five chunks: the wide list
        w.add(3, Range::new(60, 10), 0, all); // chunks 0 and 1
        w.add(4, Range::new(100, 1), 0, all); // chunk 1
        w.add(5, Range::new(0, 0), 0, all); // null: never woken
        let mut out = Vec::new();
        let procs = |o: &[(u64, u32)]| o.iter().map(|x| x.1).collect::<Vec<_>>();
        w.take_woken(
            &[Range::new(65, 1), Range::new(200, 2)],
            all,
            none,
            &mut out,
        );
        assert_eq!(procs(&out), [1, 2, 3]);
        // The woken processes resumed (their other copies are stale), and so did 4.
        let live = |w: &Waiter| w.proc > 4;
        w.take_woken(&[Range::new(0, 300)], live, none, &mut out);
        assert!(out.is_empty());
        w.add(6, Range::new(299, 1), 0, all);
        w.take_woken(&[Range::new(0, 300)], live, none, &mut out);
        assert_eq!(procs(&out), [6]);
    }

    #[test]
    fn one_chunk_signal_has_one_waiter_list() {
        let all = |_: &Waiter| true;
        let mut w = Waiters::new(40);
        assert_eq!(w.lists.len(), 1);
        w.add(1, Range::new(30, 1), 0, all);
        w.add(2, Range::new(0, 40), 0, all);
        w.add(3, Range::new(5, 0), 0, all); // null: never woken
        let mut out = Vec::new();
        w.take_woken(&[Range::new(2, 1)], all, |_| false, &mut out);
        assert_eq!(out.iter().map(|x| x.1).collect::<Vec<_>>(), [2]);
        w.take_woken(&[Range::new(30, 1)], all, |_| false, &mut out);
        assert_eq!(out.iter().map(|x| x.1).collect::<Vec<_>>(), [1]);
    }

    #[test]
    fn transport_deletes_at_or_after() {
        let mut w = Waveform::default();
        w.schedule(Range::new(0, 1), None, &mut smallvec![txn(10, 0, &[1])]);
        w.schedule(Range::new(0, 1), None, &mut smallvec![txn(20, 0, &[2])]);
        w.schedule(Range::new(0, 1), None, &mut smallvec![txn(15, 0, &[3])]);
        assert_eq!(w.pending.to_vec(), vec![txn(10, 0, &[1]), txn(15, 0, &[3])]);
    }

    #[test]
    fn inertial_keeps_same_value_chain_per_element() {
        // Old "00" at 10 ns; new "01" at 20 ns with 20 ns rejection: element 0 keeps its old
        // transaction (same value), element 1 loses it.
        let mut w = Waveform::default();
        w.pending.push(txn(10, 0, &[0, 0]));
        w.schedule(
            Range::new(0, 2),
            Some(Time::ZERO),
            &mut smallvec![txn(20, 0, &[0, 1])],
        );
        assert_eq!(
            w.pending.to_vec(),
            vec![txn(10, 0, &[0]), txn(20, 0, &[0, 1])]
        );
    }

    #[test]
    fn inertial_outside_window_kept() {
        let mut w = Waveform::default();
        w.pending.push(txn(1, 0, &[7]));
        w.schedule(
            Range::new(0, 1),
            Some(Time::ns(5)),
            &mut smallvec![txn(10, 0, &[1])],
        );
        assert_eq!(w.pending.len(), 2);
    }

    #[test]
    fn sub_element_assignment_splits_only_its_range() {
        let mut w = Waveform::default();
        w.pending.push(txn(10, 0, &[1, 2, 3, 4]));
        w.schedule(Range::new(1, 2), None, &mut smallvec![txn(5, 1, &[9, 9])]);
        assert_eq!(
            w.pending.to_vec(),
            vec![txn(5, 1, &[9, 9]), txn(10, 0, &[1]), txn(10, 3, &[4])]
        );
    }

    #[test]
    fn inertial_on_planes_and_null_transactions() {
        let lt = |ns, s: &str| {
            let l: LogicVec = s.parse().unwrap();
            Txn {
                time: Time::ns(ns),
                range: Range::new(0, l.len() as u32),
                value: Some(Buf::from_ref(Elem::Logic, l.len(), (&l).into()).unwrap()),
            }
        };
        let mut w = Waveform::default();
        w.pending.push(lt(10, "10"));
        w.schedule(
            Range::new(0, 2),
            Some(Time::ZERO),
            &mut smallvec![lt(20, "11")],
        );
        // Element 1 ('1' both times) keeps the 10 ns transaction, element 0 does not.
        assert_eq!(w.pending.len(), 2);
        assert_eq!(w.pending[0].range, Range::new(1, 1));
        let null = |ns| Txn {
            time: Time::ns(ns),
            range: Range::new(0, 2),
            value: None,
        };
        let mut w = Waveform::default();
        w.pending.push(null(10));
        w.schedule(Range::new(0, 2), Some(Time::ZERO), &mut smallvec![null(20)]);
        assert_eq!(w.pending.to_vec(), vec![null(10), null(20)]);
    }
}
