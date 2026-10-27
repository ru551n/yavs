//! Signal storage, drivers and projected output waveforms.
//!
//! A signal is one contiguous array of `len` elements (see [`crate::SignalType`]). Everything
//! that addresses part of a signal (drivers, events, forces, ports, waiters, value-change
//! events) uses one [`Range`] of element indices (design rule 2).

use std::sync::Arc;

use smallvec::{SmallVec, smallvec};

use crate::error::ErrorKind;
use crate::time::Time;
use crate::value::{
    Buf, Elem, SignalType, bits, bytes_eq, fill_bits, plane_code, plane_is, put_bits, put_bytes,
    put_plane_code,
};
use yavs_logic::Block;
use yavs_logic::wide::{nmask, put_block};

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
    #[inline]
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
/// driving value. `Default` (no mode written, the target found only at run time, an external
/// name) is `Driving` for the formal of a port of mode out, inout or buffer and `Effective`
/// otherwise.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ForceMode {
    Effective,
    Driving,
    Default,
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
    /// Some element of `range` has another source too (`build_segments`).
    pub shared: bool,
}

impl DriverSlot {
    /// Store the at most 64 logic elements of a vector quick transaction at `rel` (relative to
    /// the driver) and connect them.
    #[inline]
    fn put_planes(&mut self, rel: usize, len: usize, planes: [u64; 4]) {
        if let Buf::Planes { b, .. } = &mut self.value {
            put_block(b, rel, len, planes, !0);
        }
        if let Some(off) = &mut self.off {
            fill_bits(off, rel, len, false);
        }
    }
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
    /// Resident waiter (`ProcMeta::perm`): its epoch, 0 for a plain one.
    pub perm: u64,
    /// The edge trigger of its process (`ProcMeta::tid`).
    pub tid: u32,
    /// Registration number on its signal: the order in which waiters wake.
    seq: u64,
    /// The elements it waits for as bits, on a signal of at most 64 elements (else 0).
    mask: u64,
}

/// What a waiter takes from its process when it is registered.
#[derive(Clone, Copy, Default)]
pub(crate) struct Reg {
    pub generation: u64,
    /// Epoch of a resident waiter, 0 for a plain one (`ProcMeta::perm`).
    pub perm: u64,
    pub trigger: Option<crate::EdgeTrigger>,
    /// `ProcMeta::tid`.
    pub tid: u32,
}

/// The most chunk lists with waiters a signal tracks (`Waiters::chunk_ix`).
const CHUNK_IX: usize = 16;

/// The most distinct edge triggers a signal's wake list (`Cache`) can hold: a clock read by the
/// processes of a CPU has one per reset signal and edge.
const MAX_TIDS: usize = 32;

/// The most buckets a waiter is copied into before it goes to the wide list instead.
const WAIT_NARROW: u32 = 4;

/// The changed runs of an event, ready to be tested against waiters: as bits on a signal of at
/// most 64 elements, else as the sorted runs.
pub(crate) enum Hits<'a> {
    Mask(u64),
    Runs(&'a [Range]),
}

impl<'a> Hits<'a> {
    #[inline(always)]
    pub(crate) fn of(len: u32, runs: &'a [Range]) -> Hits<'a> {
        if len <= 64 {
            Hits::Mask(
                runs.iter()
                    .fold(0, |m, r| m | nmask(r.len as usize) << r.offset),
            )
        } else {
            Hits::Runs(runs)
        }
    }

    /// Whether the event touches an element `w` waits for.
    #[inline(always)]
    pub(crate) fn hit(&self, w: &Waiter) -> bool {
        match self {
            Hits::Mask(m) => w.mask & m != 0,
            Hits::Runs(runs) => runs_hit(runs, w.range),
        }
    }
}

/// Whether a range of `runs` (sorted, disjoint) shares an element with `r`.
#[inline(always)]
pub(crate) fn runs_hit(runs: &[Range], r: Range) -> bool {
    match runs {
        [x] => x.overlaps(r),
        _ => {
            let i = runs.partition_point(|x| x.end() <= r.offset);
            runs.get(i).is_some_and(|x| x.overlaps(r))
        }
    }
}

/// The processes waiting on one signal, bucketed by chunk of 8 elements (64 for a signal over
/// 4096 elements, one bucket for one of at most 8), so an event looks only at the waiters of
/// the chunks it touches: one process per element of a wide vector costs O(1) per event, not
/// O(processes). Waiters over more than `WAIT_NARROW` chunks (or none) sit in a wide list that
/// every event scans. Each list keeps its count right after the last compaction, so lists of
/// signals that rarely change do not grow without bound. The lists are made when the first
/// process waits: most signals never have one.
#[derive(Default)]
pub(crate) struct Waiters {
    /// Per chunk, then the wide list last; empty until the first `add`.
    lists: Vec<(SmallVec<[Waiter; 1]>, usize)>,
    next: u64,
    /// Elements of the signal, and log2 of the chunk size.
    len: u32,
    shift: u32,
    /// The edge trigger every waiter ever added shares (see `uniform_trigger`).
    trig: Trig,
    /// The wake list of a signal whose waiters all are resident, alive and wait for the whole
    /// signal (a clock and its processes): what a scan would produce, kept until a waiter is
    /// added or a process stops waiting this way.
    cache: Option<Cache>,
    /// A waiter was ever added to a list other than the wide one: an event must look at the
    /// lists of the elements it hit. Without, only the wide list is scanned, whatever the
    /// number of runs (a wide bus whose processes all wait for all of it, #519).
    chunked: bool,
    /// The chunk lists that may hold a waiter (at most `CHUNK_IX`; `chunk_over` when more were
    /// used): an event on a wide signal looks only at these, not at every chunk it spans.
    chunk_ix: SmallVec<[u32; 4]>,
    chunk_over: bool,
}

/// `Cache::batch`: the plain processes (one suspension point, not postponed) in list order,
/// the same by trigger id, and the others with their trigger ids.
pub(crate) struct Batch {
    pub plain: Vec<u32>,
    /// `plain` by trigger id, each in list order.
    pub by_tid: SmallVec<[(u32, Vec<u32>); 8]>,
    pub others: Vec<(u32, u32)>,
}

struct Cache {
    /// `(registration, process, trigger id)` of every waiter, in list order.
    list: Vec<(u64, u32, u32)>,
    /// The distinct trigger ids in `list` (0: none), at most `MAX_TIDS`.
    tids: SmallVec<[u32; 8]>,
    /// The processes of `list` that run as one batch in list order when they all wake, and
    /// the others, which wake one by one (`SimControl::batch_of`); `None` inside when the list
    /// is not in process order. Set by the kernel at first use.
    batch: Option<Option<Batch>>,
}

/// What the waiters of a signal have in common: no waiter, all the same edge trigger, or not.
#[derive(Clone, Copy, Default, PartialEq)]
enum Trig {
    #[default]
    Empty,
    Same(crate::EdgeTrigger),
    Mixed,
}

impl Waiters {
    pub(crate) fn new(len: u32) -> Waiters {
        // One list for up to 8 elements, one per element up to 1024 (a process per element of a
        // vector then scans one waiter per event), chunks of 8 and 64 beyond.
        let shift = match len {
            0..=8 => 3,
            9..=1024 => 0,
            1025..=4096 => 3,
            _ => 6,
        };
        Waiters {
            lists: Vec::new(),
            next: 0,
            len,
            shift,
            trig: Trig::Empty,
            cache: None,
            chunked: false,
            chunk_ix: SmallVec::new(),
            chunk_over: false,
        }
    }

    /// Every waiter in every list (a waiter in several chunk lists comes up once per list); for
    /// the hang report, which is cold.
    pub fn iter(&self) -> impl Iterator<Item = &Waiter> {
        self.lists.iter().flat_map(|(l, _)| l.iter())
    }

    /// The one list every waiter is in, when none was ever put in a chunk's list: the only list of
    /// a small signal, or the wide list of a big one whose waiters all wait for most of it.
    #[inline(always)]
    fn only(&self) -> Option<&[Waiter]> {
        if self.chunked {
            return None;
        }
        self.lists.last().map(|l| l.0.as_slice())
    }

    /// No process has ever waited on the signal.
    #[inline(always)]
    pub fn is_unwatched(&self) -> bool {
        self.lists.is_empty()
    }

    /// The one waiter of element `e`, when its list holds exactly one resident waiter that
    /// covers it and no waiter is in the wide list (a process per element, waiting for its
    /// element): what `take_woken` of an event on `e` alone would look at.
    #[inline]
    pub fn sole_resident(&self, e: u32) -> Option<&Waiter> {
        let (wide, chunks) = self.lists.split_last()?;
        let list = chunks.get((e >> self.shift) as usize)?;
        if !wide.0.is_empty() {
            return None;
        }
        match list.0.as_slice() {
            [w] if w.perm != 0 && w.range.overlaps(Range::new(e, 1)) => Some(w),
            _ => None,
        }
    }

    /// The signal has one waiter, a resident one of process `p`.
    #[inline]
    pub fn only_resident(&self, p: u32) -> bool {
        matches!(self.only(), Some([w]) if w.proc == p && w.perm != 0)
    }

    /// The waiters of a signal that has one list of at most `max` (a clock read by a few
    /// processes), in registration order.
    #[inline]
    pub fn small_list(&self, max: usize) -> Option<&[Waiter]> {
        self.only().filter(|l| l.len() <= max)
    }

    /// Forget the wake list (a waiter changed).
    pub fn drop_cache(&mut self) {
        self.cache = None;
    }

    /// The edge trigger of every waiter, when the signal has one list of waiters and they all
    /// have the same: when it does not hold, no event of this cycle wakes any of them, and the
    /// list need not be scanned. Stale waiters have the trigger of their process, too.
    pub fn uniform_trigger(&self) -> Option<crate::EdgeTrigger> {
        match self.trig {
            Trig::Same(t) if self.only().is_some() => Some(t),
            _ => None,
        }
    }

    /// The distinct trigger ids of the wake list, if it is valid.
    pub fn cache_tids(&self) -> Option<SmallVec<[u32; 8]>> {
        self.cache.as_ref().map(|c| c.tids.clone())
    }

    /// Take the wake list (after `cache_tids` said there is one); give it back with
    /// `restore_cache`.
    pub fn take_cache(&mut self) -> Vec<(u64, u32, u32)> {
        self.cache
            .as_mut()
            .map(|c| std::mem::take(&mut c.list))
            .unwrap_or_default()
    }

    /// The batches of the wake list (`Cache::batch`), if known.
    pub fn cache_batch(&self) -> Option<Option<&Batch>> {
        self.cache
            .as_ref()
            .and_then(|c| c.batch.as_ref().map(Option::as_ref))
    }

    pub fn set_cache_batch(&mut self, b: Option<Batch>) {
        if let Some(c) = &mut self.cache {
            c.batch = Some(b);
        }
    }

    pub fn restore_cache(&mut self, list: Vec<(u64, u32, u32)>) {
        if let Some(c) = &mut self.cache {
            c.list = list;
        }
    }

    fn init(&mut self) {
        // A signal of one chunk or less has one list: the chunk's and the wide one.
        let chunks = self.len.div_ceil(1 << self.shift) as usize;
        let n = if chunks <= 1 { 1 } else { chunks + 1 };
        self.lists = (0..n).map(|_| (SmallVec::new(), 0)).collect();
    }

    fn lists_of(&self, r: Range) -> std::ops::Range<usize> {
        let wide = self.lists.len() - 1;
        if wide == 0 {
            return 0..1;
        }
        if r.len == 0 {
            return wide..wide + 1;
        }
        let (a, b) = (r.offset >> self.shift, (r.end() - 1) >> self.shift);
        if b - a >= WAIT_NARROW {
            return wide..wide + 1;
        }
        a as usize..b as usize + 1
    }

    pub fn add(&mut self, proc: u32, range: Range, reg: Reg, live: impl Fn(&Waiter) -> bool) {
        let Reg {
            generation,
            perm,
            trigger,
            tid,
        } = reg;
        if self.lists.is_empty() {
            self.init();
        }
        self.trig = match (self.trig, trigger) {
            (Trig::Empty, Some(t)) => Trig::Same(t),
            (Trig::Same(u), Some(t)) if u == t => Trig::Same(u),
            _ => Trig::Mixed,
        };
        let seq = self.next;
        self.next += 1;
        // A resident waiter for the whole signal, like the others, joins the wake list.
        let joins = perm != 0 && self.len > 0 && range == Range::new(0, self.len);
        match &mut self.cache {
            Some(c)
                if joins && !self.chunked && (c.tids.contains(&tid) || c.tids.len() < MAX_TIDS) =>
            {
                if !c.tids.contains(&tid) {
                    c.tids.push(tid);
                }
                c.list.push((seq, proc, tid));
            }
            cache => *cache = None,
        }
        let wide = self.lists.len() - 1;
        for i in self.lists_of(range) {
            if i != wide {
                self.chunked = true;
                if !self.chunk_over && !self.chunk_ix.contains(&(i as u32)) {
                    if self.chunk_ix.len() == CHUNK_IX {
                        self.chunk_over = true;
                        self.chunk_ix.clear();
                    } else {
                        self.chunk_ix.push(i as u32);
                    }
                }
            }
            let (l, n) = &mut self.lists[i];
            if l.len() >= 2 * *n + 16 {
                l.retain(|w| live(w));
                *n = l.len();
            }
            l.push(Waiter {
                proc,
                range,
                generation,
                perm,
                tid,
                seq,
                mask: if self.len <= 64 {
                    nmask(range.len as usize) << range.offset
                } else {
                    0
                },
            });
        }
    }

    /// Make the wake list after a scan of the one list, if all its waiters are resident and wait
    /// for the whole signal. The scan removed the stale ones that an event of the whole signal
    /// hits, so the rest are alive.
    fn build_cache(&mut self) {
        let full = Range::new(0, self.len);
        let Some(l) = self.lists.last().map(|l| &l.0) else {
            return;
        };
        if full.len == 0 || l.len() < 16 || !l.iter().all(|w| w.perm != 0 && w.range == full) {
            return;
        }
        let mut tids: SmallVec<[u32; 8]> = SmallVec::new();
        for w in l.iter() {
            if !tids.contains(&w.tid) {
                if tids.len() == MAX_TIDS {
                    return;
                }
                tids.push(w.tid);
            }
        }
        let list = l.iter().map(|w| (w.seq, w.proc, w.tid)).collect();
        self.cache = Some(Cache {
            list,
            tids,
            batch: None,
        });
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
        ordered: bool,
    ) {
        let hit = |r: Range| runs_hit(runs, r);
        let hits = Hits::of(self.len, runs);
        out.clear();
        let Some(wide) = self.lists.len().checked_sub(1) else {
            return;
        };
        // One element changed and its list holds one resident waiter, nothing in the wide list (a
        // process per element, waiting for its element): no scan, no rewrite.
        if let [r] = runs
            && r.len == 1
            && wide > 0
            && self.lists[wide].0.is_empty()
        {
            let c = (r.offset >> self.shift) as usize;
            if c < wide
                && let (l, n) = &mut self.lists[c]
                && let [w] = l.as_slice()
                && w.perm != 0
            {
                if w.range.overlaps(*r) {
                    if !live(w) {
                        l.clear();
                        *n = 0;
                    } else if !hold(w) {
                        out.push((w.seq, w.proc));
                    }
                }
                return;
            }
        }
        let shift = self.shift;
        let mut scanned = 0;
        // A list that went empty: the index of chunks with waiters is out of date.
        let mut emptied = false;
        // One list: the waiters hit are taken in list order.
        let mut scan = |(l, n): &mut (SmallVec<[Waiter; 1]>, usize)| {
            if l.is_empty() {
                return;
            }
            scanned += 1;
            // Read the list first: resident waiters that wake stay, so most scans change nothing
            // and need no rewrite of the list. The cheap range test comes first: a stale waiter
            // off the events stays until `add` compacts the list, and the process table is not
            // touched for it.
            let mut remove = false;
            for w in l.iter() {
                if !hits.hit(w) {
                    continue;
                }
                if !live(w) {
                    remove = true;
                } else if !hold(w) {
                    out.push((w.seq, w.proc));
                    remove |= w.perm == 0;
                }
            }
            if remove {
                l.retain(|w| !hits.hit(w) || (live(w) && (hold(w) || w.perm != 0)));
                emptied |= l.is_empty();
            }
            *n = l.len();
        };
        if let [r] = runs
            && r.len == 1
        {
            // One element changed (the usual event of a process per element): its list, then the
            // wide one.
            let c = (r.offset >> shift) as usize;
            if c != wide {
                scan(&mut self.lists[c]);
            }
            scan(&mut self.lists[wide]);
        } else if !self.chunked {
            scan(&mut self.lists[wide]);
        } else if !self.chunk_over {
            // The few chunk lists with waiters, those an event spans, then the wide list.
            let len = self.len;
            let mut ix = std::mem::take(&mut self.chunk_ix);
            for &c in &ix {
                let lo = c << shift;
                if hit(Range::new(lo, (1 << shift).min(len - lo))) {
                    scan(&mut self.lists[c as usize]);
                }
            }
            scan(&mut self.lists[wide]);
            if emptied {
                ix.retain(|&mut c| !self.lists[c as usize].0.is_empty());
                self.chunked = !ix.is_empty();
            }
            self.chunk_ix = ix;
        } else {
            let mut last = usize::MAX;
            let chunks = runs.iter().flat_map(|r| {
                (r.len > 0)
                    .then(|| r.offset >> shift..=(r.end() - 1) >> shift)
                    .into_iter()
                    .flatten()
            });
            for i in chunks.map(|c| c as usize).chain([wide]) {
                if i == last {
                    continue;
                }
                last = i;
                scan(&mut self.lists[i]);
            }
        }
        if self.only().is_some_and(|l| l.is_empty()) {
            self.trig = Trig::Empty;
        }
        if self.cache.is_none()
            && self.only().is_some_and(|l| l.len() >= 16)
            && scanned == 1
            && !runs.is_empty()
        {
            self.build_cache();
        }
        // `ordered`: the wake order is the registration order, each waiter once. Otherwise a waiter in
        // several lists comes up once per list and the order is that of the scan: waking is
        // idempotent and the batch is ordered by process, whatever order it was woken in.
        if ordered && scanned > 1 && out.len() > 1 && !out.is_sorted_by(|a, b| a.0 < b.0) {
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
    /// last event, of the last activity, and the time (`stamp`, 0 if never) of
    /// the last event and of the last activity.
    stamps: SmallVec<[u64; 4]>,
    /// A signal of more than 64 elements: per block of 64 elements, the cycle and time stamp of
    /// the last `mark_active` that covered the whole block. The activity of an element is the
    /// later of its own stamps and its block's, so marking a wide range costs a word per block
    /// (#519).
    blk_active: Vec<[u64; 2]>,
    /// A signal of more than 64 elements: per block of 64 elements, the events of
    /// `update_word` whose elements have not changed since, each with those elements (the
    /// masks of a block are disjoint, so a block has at most 64). The last event of an element
    /// is the later of its own stamps and the record that lists it: a wide bus that changes in
    /// most cycles writes no element stamps (#519).
    blk_event: Vec<SmallVec<[EvRec; 4]>>,
    /// Lean event bookkeeping (`set_lean_events`): per block of 64 elements, only the last
    /// event of the block (its cycle and time, and the elements that changed in it), in place
    /// of `blk_event`. Empty unless `lean`.
    blk_last: Vec<EvRec>,
    /// The event history of the elements is not kept: only each block's last event
    /// (`blk_last`). `'last_event` and `'last_value` of a range that is not whole blocks
    /// cannot be answered (`history_exact`); `'event`, edge tests and the rest are exact.
    lean: bool,
    /// Sources: process drivers and out-ports, in creation order.
    pub drivers: Vec<DriverSlot>,
    pub segments: SmallVec<[Segment; 1]>,
    pub resolution: Option<Resolution>,
    pub guard: Option<GuardKind>,
    /// The resolution function touches shared state: the signal is never updated in parallel.
    pub serial_res: bool,
    /// The formal of a port of mode out, inout or buffer (the default force mode is `out`).
    pub out_port: bool,
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
    /// Elements changed in the current cycle.
    pub changed: Vec<Range>,
    /// Ranges whose driving value must be recomputed this cycle.
    pub dirty: Vec<Range>,
    /// Ranges whose driving value was written when the transaction was applied (a vector quick
    /// transaction on a sole source, `apply_driver`): only the effective value is left to update.
    pub fast_dirty: Vec<Range>,
    /// Ranges whose effective value must be recomputed this cycle.
    pub eff_dirty: Vec<Range>,
    /// The simulation runs (not initialising): reports of its resolution function are
    /// collected in `callouts` for the kernel to commit (#372).
    pub live: bool,
    /// Ranges where a force or release of this cycle changed the effective value, whatever
    /// the cycle's last one left: an event even when the value ends up unchanged (#382).
    pub force_ev: Vec<Range>,
    pub callouts: Vec<(crate::process::Severity, String)>,
    /// An assignment of the value a source already has may be skipped (`Builder::build`).
    pub skip_ok: bool,
    /// What `plain` asks that never changes after the build (`SimControl::new`).
    pub static_plain: bool,
    /// The segment the last `compute_driving` started in: the next dirty range is usually there
    /// or in the next one.
    seg_hint: usize,
    /// The `std_ulogic` code of every element of `cur`, one byte each (rounded up to whole
    /// plane blocks), for logic signals of at most `CODES_MAX` elements unless `no_codes` or a bus
    /// (`CODES_SOURCE_MAX`); empty otherwise. Kept in step with `cur` (`sync_codes`, `refresh_codes`): a read of one element
    /// is one load.
    codes: Vec<u8>,
    /// The elaborator found no read of a single element of the signal (`Builder::no_codes`):
    /// refreshing `codes` on every change would cost more than it saves.
    no_codes: bool,
    /// Quick transactions of this cycle finished at once (`apply_driver`), and whether one of
    /// them made an event not yet reported to the kernel.
    fuse_n: u32,
    pub fused_event: bool,
    /// A process waits on the signal, or an observer wants its value changes: `changed` lists
    /// the runs of elements that changed in a cycle. Otherwise only `nruns` counts them.
    pub watched: bool,
    /// Something needs the exact runs in `changed`: a process waits for a part of the signal, or
    /// an observer wants the value changes. Else `changed` holds the cover of the runs
    /// (one range) and `nruns` counts them.
    pub fine: bool,
    /// Ascending element offsets, strictly inside the signal, where a waited range starts or
    /// ends (`watch_part`). Any: the signal is `fine` and `changed` lists per cell, not per run,
    /// for a signal of more than 64 elements whose waiters wait for unions of the cells between
    /// them: a change of a cell is any non-empty part of it.
    pub cuts: Box<[u32]>,
    pub nruns: u64,
    /// Elements whose driving value a direct transaction wrote for the next update
    /// (`commit_code`, `commit_planes`),
    /// one bit each (made at the first one), and the words with a bit set.
    pend: Vec<u64>,
    pend_words: Vec<u32>,
    /// One element is pending and it is not in `pend`: the one `last_commit` holds.
    pend_one: bool,
    /// Direct commits wrote the driving value only: the values of the sole-source drivers
    /// (`DriverSlot::shared` false) may be older than `driving`, which holds them
    /// (`sync_drivers` copies them back). Never with `exact`.
    pub lag: bool,
    /// A deposit made the driving value differ from the drivers: direct commits write the driver
    /// value too, as before.
    pub exact: bool,
    /// Nothing reads the activity of this signal (`Config::skip_unchanged`, and no implicit
    /// signal has it as its prefix): `finish_pend` leaves the activity stamps alone.
    pub inactive: bool,
    /// The element and code of the last `commit_code`.
    last_commit: (u32, u8),
}

/// The elements of a block of 64 whose last event was one `update_word`, with its cycle and
/// time stamp (`Signal::blk_event`).
#[derive(Clone, Copy, Default)]
struct EvRec {
    cycle: u64,
    time: u64,
    mask: u64,
}

/// Element run `e .. e + len` changed: listed in `changed` if anything looks at the list
/// (`Signal::watched`), else only counted.
#[inline(always)]
fn note_run(
    changed: &mut Vec<Range>,
    nruns: &mut u64,
    watched: bool,
    fine: bool,
    e: usize,
    len: u32,
) {
    if !watched {
        *nruns += 1;
        return;
    }
    if !fine {
        cover(changed, nruns, e, len, 1);
        return;
    }
    match changed.last_mut() {
        Some(run) if run.end() as usize == e => run.len += len,
        _ => changed.push(Range::new(e as u32, len)),
    }
}

/// `changed` as one range: the elements `e .. e + len`, `runs` runs of them, joined to the cover of
/// the runs so far. The cover counts as one event itself, so `nruns` leaves it out and
/// the cycle's count of events (`changed` plus `nruns`) stays the number of runs.
#[inline(always)]
fn cover(changed: &mut Vec<Range>, nruns: &mut u64, e: usize, len: u32, runs: u64) {
    *nruns += runs;
    match changed.first_mut() {
        Some(c) => {
            let lo = c.offset.min(e as u32);
            let hi = c.end().max(e as u32 + len);
            *c = Range::new(lo, hi - lo);
        }
        None => {
            changed.push(Range::new(e as u32, len));
            *nruns -= 1;
        }
    }
}

/// Elements `m` (bit `i` is element `base + i`) of a block changed, on a signal that lists
/// cells (`Signal::cells`): one note in each cell with a change. A waiter of a part waits for
/// whole cells, so a note anywhere in a changed cell wakes it exactly when the exact runs
/// would. A few runs are noted as they are; more are noted as the hull of the changes cut at
/// the cuts, which are rare in a block.
#[inline(always)]
fn note_cells(changed: &mut Vec<Range>, nruns: &mut u64, cuts: &[u32], base: usize, m: u64) {
    let runs = u64::from((m & !(m << 1)).count_ones());
    if runs <= 2 {
        let mut v = m;
        while v != 0 {
            let s = v.trailing_zeros();
            let run = (v >> s).trailing_ones();
            note_run(changed, nruns, true, true, base + s as usize, run);
            v &= !(nmask(run as usize) << s);
        }
        return;
    }
    let (a, z) = (
        base + m.trailing_zeros() as usize,
        base + 64 - m.leading_zeros() as usize,
    );
    let before = changed.len();
    let mut i = cuts.partition_point(|&c| c as usize <= a);
    if cuts.get(i).is_none_or(|&c| c as usize >= z) {
        note_run(changed, nruns, true, true, a, (z - a) as u32);
    } else {
        let mut lo = a;
        while lo < z {
            let hi = cuts.get(i).map_or(z, |&c| (c as usize).min(z));
            if m & (nmask(hi - lo) << (lo - base)) != 0 {
                note_run(changed, nruns, true, true, lo, (hi - lo) as u32);
            }
            lo = hi;
            i += 1;
        }
    }
    // The count of events stays the number of runs (the notes count themselves).
    *nruns += runs.saturating_sub((changed.len() - before) as u64);
}

/// The longest logic signal that keeps `Signal::codes`.
const CODES_MAX: usize = 1 << 20;

/// Whether `cur` holds `code` at element `e`: from the codes, else (a signal that keeps none) the
/// planes, inline: a signal without codes must skip as cheaply as one with (#734).
#[inline(always)]
fn held(codes: &[u8], cur: &Buf, e: usize, code: u8) -> bool {
    match (codes.get(e), cur) {
        (Some(&c), _) => c == code,
        (None, Buf::Planes { b, .. }) => plane_is(b, e, code),
        (None, _) => bytes_held(cur, e, code),
    }
}

#[inline(never)]
fn bytes_held(cur: &Buf, e: usize, code: u8) -> bool {
    cur.code(e) == code
}

/// A logic signal of more than 64 elements (several plane blocks) keeps its byte codes only when
/// every source (driver or out-port) has at most this many elements. A wider source makes it a
/// bus, updated a block at a time: refreshing 64 codes on every change costs more than the
/// plane reads of the rare single element do. A vector of registers (sources of one element,
/// or a byte lane) keeps them: its updates are single elements and its elements are read singly.
const CODES_SOURCE_MAX: usize = 16;

/// Elements `elem .. elem + len` (`len <= 64`) into the pending bit mask and its list of words.
#[inline(always)]
fn mark_bits(pend: &mut [u64], words: &mut Vec<u32>, elem: u32, len: u32) {
    let (w, s, m) = (elem as usize / 64, elem % 64, nmask(len as usize));
    if pend[w] == 0 {
        words.push(w as u32);
    }
    pend[w] |= m << s;
    if s + len > 64 {
        if pend[w + 1] == 0 {
            words.push(w as u32 + 1);
        }
        pend[w + 1] |= m >> (64 - s);
    }
}

/// The pending words of a signal in element order. Processes of the elements of a vector
/// often commit in descending element order (the batch is in process order): reversed, not
/// sorted.
fn order_words(words: &mut [u32]) {
    if words.is_sorted() {
        return;
    }
    if words.is_sorted_by(|a, b| a >= b) {
        words.reverse();
    } else {
        words.sort_unstable();
    }
}

/// Byte `i` of `SPREAD[x]` is bit `i` of `x`.
const SPREAD: [u64; 256] = {
    let mut t = [0u64; 256];
    let mut x = 0;
    while x < 256 {
        let mut i = 0;
        while i < 8 {
            t[x] |= (((x >> i) & 1) as u64) << (8 * i);
            i += 1;
        }
        x += 1;
    }
    t
};

/// `codes` of the groups of eight elements of block `c` (elements `base ..`) that hold a changed
/// element `m`: a sparse change leaves the groups between its ends alone.
#[inline]
fn refresh_changed_codes(codes: &mut [u8], c: &Block, base: usize, m: u64) {
    if codes.is_empty() {
        return;
    }
    let Some(out) = codes.get_mut(base..base + 64) else {
        return;
    };
    let mut left = m;
    while left != 0 {
        let sh = (left.trailing_zeros() & !7) as usize;
        left &= !(0xff << sh);
        let at = |p: usize| SPREAD[(c[p] >> sh & 0xff) as usize] << p;
        let v = at(0) | at(1) | at(2) | at(3);
        out[sh..sh + 8].copy_from_slice(&v.to_le_bytes());
    }
}

/// Recompute `codes` for elements `lo .. hi` of `cur`, in groups of eight.
#[inline]
fn refresh_codes(codes: &mut [u8], cur: &Buf, lo: usize, hi: usize) {
    if let Buf::Planes { b, .. } = cur {
        refresh_block_codes(codes, b, lo, hi);
    }
}

/// `refresh_codes` of the plane blocks `b`.
#[inline]
fn refresh_block_codes(codes: &mut [u8], b: &[Block], lo: usize, hi: usize) {
    if codes.is_empty() || lo >= hi {
        return;
    }
    let (g0, g1) = (lo / 8, (hi - 1) / 8);
    // One plane block (64 elements, eight groups) at a time.
    for blk in g0 / 8..=g1 / 8 {
        let (Some(w), Some(out)) = (b.get(blk), codes.get_mut(blk * 64..blk * 64 + 64)) else {
            return;
        };
        let (first, last) = (g0.max(blk * 8) % 8, g1.min(blk * 8 + 7) % 8);
        for j in first..=last {
            let sh = j * 8;
            let at = |p: usize| SPREAD[(w[p] >> sh & 0xff) as usize] << p;
            let v = at(0) | at(1) | at(2) | at(3);
            out[sh..sh + 8].copy_from_slice(&v.to_le_bytes());
        }
    }
}

impl Signal {
    /// Every element of `r` has exactly one source.
    pub fn sole(&self, r: Range) -> bool {
        let i = self.segments.partition_point(|s| s.range.end() <= r.offset);
        (self.segments.get(i)).is_some_and(|s| s.range.contains(r) && s.drivers.len() == 1)
    }

    pub fn new(name: String, ty: SignalType, init: Buf, resolution: Option<Resolution>) -> Signal {
        let n = ty.len as usize;
        let mut sig = Signal {
            name,
            ty,
            cur: init.clone(),
            driving: init.clone(),
            last_value: init,
            stamps: stamps(n),
            blk_active: if n > 64 {
                vec![[0; 2]; n.div_ceil(64)]
            } else {
                Vec::new()
            },
            blk_event: if n > 64 {
                vec![SmallVec::new(); n.div_ceil(64)]
            } else {
                Vec::new()
            },
            blk_last: Vec::new(),
            lean: false,
            drivers: Vec::new(),
            segments: SmallVec::new(),
            resolution,
            guard: None,
            serial_res: false,
            out_port: false,
            groups: Vec::new(),
            disconnect: Time::ZERO,
            force_eff: None,
            force_drv: None,
            port_in: None,
            level: 0,
            queued_up: 0,
            queued_down: 0,
            changed: Vec::new(),
            dirty: Vec::new(),
            fast_dirty: Vec::new(),
            eff_dirty: Vec::new(),
            live: false,
            force_ev: Vec::new(),
            callouts: Vec::new(),
            skip_ok: false,
            static_plain: false,
            seg_hint: 0,
            codes: Vec::new(),
            no_codes: false,
            fuse_n: 0,
            fused_event: false,
            watched: false,
            fine: false,
            cuts: Box::default(),
            nruns: 0,
            pend: Vec::new(),
            pend_words: Vec::new(),
            pend_one: false,
            lag: false,
            exact: false,
            inactive: false,
            last_commit: (u32::MAX, 0),
        };
        sig.sync_codes();
        sig
    }

    /// Rebuild `codes` from `cur` (after `cur` was replaced as a whole).
    pub fn sync_codes(&mut self) {
        self.codes.clear();
        if let Buf::Planes { len, b } = &self.cur
            && *len <= CODES_MAX
            && !self.no_codes
            && (*len <= 64
                || self
                    .drivers
                    .iter()
                    .all(|d| d.range.len as usize <= CODES_SOURCE_MAX))
        {
            self.codes.resize(b.len() * 64, 0);
            refresh_codes(&mut self.codes, &self.cur, 0, *len);
        }
    }

    /// Keep no byte codes: nothing reads a single element of the signal (before the first event).
    pub fn set_no_codes(&mut self) {
        self.no_codes = true;
        self.sync_codes();
    }

    /// Whether `set_no_codes` was called.
    pub fn keeps_no_codes(&self) -> bool {
        self.no_codes
    }

    /// A process waits for, or reads, the part `r` (not the whole) of the signal: `changed` lists
    /// what changed in it. With `cells` (and more than 64 elements) it lists per cell: the ends of
    /// `r` become cuts, so every waited range stays a union of cells.
    pub fn watch_part(&mut self, r: Range, cells: bool) {
        self.fine = true;
        if cells && self.ty.len > 64 {
            for p in [r.offset, r.end()] {
                if p > 0
                    && p < self.ty.len
                    && let Err(i) = self.cuts.binary_search(&p)
                {
                    let mut v = std::mem::take(&mut self.cuts).into_vec();
                    v.insert(i, p);
                    self.cuts = v.into_boxed_slice();
                }
            }
        }
    }

    /// Element run `e .. e + len` changed (`note_run`).
    #[inline(always)]
    fn note_run(&mut self, e: usize, len: u32) {
        note_run(
            &mut self.changed,
            &mut self.nruns,
            self.watched,
            self.fine,
            e,
            len,
        );
    }

    /// Put `changed` in ascending order and join the runs that touch or overlap (cells overlap
    /// the exact runs of other updates of the cycle).
    #[inline(never)]
    pub fn canonical_changed(&mut self) {
        let runs = &mut self.changed;
        if runs.len() < 2 {
            return;
        }
        if !runs.is_sorted_by_key(|r| r.offset) {
            runs.sort_unstable_by_key(|r| r.offset);
        }
        let mut k = 0;
        for i in 1..runs.len() {
            if runs[i].offset <= runs[k].end() {
                runs[k].len = runs[k].end().max(runs[i].end()) - runs[k].offset;
            } else {
                k += 1;
                runs[k] = runs[i];
            }
        }
        runs.truncate(k + 1);
    }

    /// `codes` as a pointer and the number of elements, if the signal keeps them.
    pub fn codes_view(&self) -> Option<(*const u8, usize)> {
        (!self.codes.is_empty()).then_some((self.codes.as_ptr(), self.ty.len as usize))
    }

    /// The bytes `codes_view` can be read from its start: whole plane blocks, so the group of
    /// eight codes around the last element is there (the bytes past the last element are 0),
    /// 0 for a signal that keeps no codes.
    pub fn codes_cap(&self) -> usize {
        self.codes.len()
    }

    /// The cycle number of the last event of each element (`event_in`) of a signal of at most 64
    /// elements, which compiled code compares with the current cycle for `'event` (a wider signal
    /// also keeps event records per block). The address stays for the whole simulation; the
    /// numbers change only while no process runs.
    pub fn event_view(&self) -> Option<*const u64> {
        (self.ty.len <= 64).then(|| self.stamps.as_ptr())
    }

    /// The plane blocks of the current value of a logic signal and its number of elements, for
    /// reads of compiled code (four words per block, element `e` in bit `e % 64` of block
    /// `e / 64`). The address stays for the whole simulation; the bits change only while no
    /// process runs.
    pub fn planes_view(&self) -> Option<(*const Block, usize)> {
        match &self.cur {
            Buf::Planes { len, b } if !b.is_empty() => Some((b.as_ptr(), *len)),
            _ => None,
        }
    }

    /// The bytes of the current value of a signal of enumeration, integer, boolean or real
    /// elements, the size of an element and the number of elements (address and bytes as for
    /// `planes_view`).
    pub fn bytes_view(&self) -> Option<(*const u8, usize, usize)> {
        match &self.cur {
            Buf::Bytes { es, data } if *es > 0 => Some((data.as_ptr(), *es, data.len() / *es)),
            _ => None,
        }
    }

    /// The plane block of `last_value` of a logic signal of at most 64 elements that keeps its
    /// codes, for `rising_edge` of compiled code (the address as for `event_view`).
    pub fn last_view(&self) -> Option<*const Block> {
        match &self.last_value {
            Buf::Planes { len, b } if *len <= 64 && !self.codes.is_empty() && !b.is_empty() => {
                Some(b.as_ptr())
            }
            _ => None,
        }
    }

    /// The code of element `e` of `cur` from `codes`, if the signal keeps them and has the element.
    #[inline(always)]
    pub fn mirrored_code(&self, e: usize) -> Option<u8> {
        let c = *self.codes.get(e)?;
        // Past the last element the byte is padding.
        (e < self.ty.len as usize).then(|| {
            debug_assert_eq!(c, self.cur.code(e), "codes of {} out of step", self.name);
            c
        })
    }

    /// The `std_ulogic` code of element `e` of the current value of a logic signal.
    #[inline(always)]
    pub fn cur_code(&self, e: usize) -> u8 {
        match self.codes.get(e) {
            Some(&c) => {
                debug_assert_eq!(c, self.cur.code(e), "codes of {} out of step", self.name);
                c
            }
            None => self.cur.code(e),
        }
    }

    #[inline]
    pub fn full(&self) -> Range {
        Range::new(0, self.ty.len)
    }

    #[inline]
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
        for d in &mut self.drivers {
            d.shared = false;
        }
        for seg in &self.segments {
            if seg.drivers.len() > 1 {
                for &i in &seg.drivers {
                    self.drivers[i as usize].shared = true;
                }
            }
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
        let at = |i: usize| {
            self.segments
                .get(i)
                .is_some_and(|s| s.range.end() > r.offset)
        };
        // The first segment that reaches `r`: the last one's, a neighbour (dirty ranges come
        // in element order, ascending or descending), or a search.
        let found = |i: usize| at(i) && (i == 0 || !at(i - 1));
        let h = self.seg_hint;
        let first = [h, h + 1, h.wrapping_sub(1)]
            .into_iter()
            .find(|&i| found(i))
            .unwrap_or_else(|| self.segments.partition_point(|s| s.range.end() <= r.offset));
        self.seg_hint = first;
        let mut si = first;
        while let Some(seg) = self.segments.get(si) {
            si += 1;
            if seg.range.offset >= r.end() {
                break;
            }
            // A run of one-element segments with one driver of one element each (a process
            // per element of a vector): gathered into plane words, one write per 64 elements.
            if self.one_driver_elem(si - 1, r) {
                let ones = self.gather_ones(si - 1, r);
                si += ones - 1;
                continue;
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
                // One unguarded source of a std_logic signal: the resolution is the identity.
                (Some(Resolution::StdLogic), [i]) if self.guard.is_none() => {
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

    /// Whether a quick transaction of this cycle may be finished at once (`finish_quick`): the
    /// signal has no layer, group, guard or resolution function to wait for, and not too many
    /// elements of it change in one cycle (a block pass is cheaper for those).
    #[inline(always)]
    fn fuse_ok(&mut self) -> bool {
        // ponytail: four per cycle and signal; more go through the dirty ranges.
        const FUSE_MAX: u32 = 4;
        self.fuse_n < FUSE_MAX && self.fuse_plain()
    }

    fn fuse_plain(&mut self) -> bool {
        if !self.plain() {
            return false;
        }
        self.fuse_n += 1;
        true
    }

    /// A logic signal with no layer, group, guard or resolution function to wait for: a sole
    /// source's value is the signal's.
    #[inline(always)]
    fn plain(&self) -> bool {
        self.static_plain
            && self.force_ev.is_empty()
            && self.force_drv.is_none()
            && self.force_eff.is_none()
    }

    /// A direct transaction waits to be finished (`finish_pend`).
    #[inline(always)]
    pub fn has_pending(&self) -> bool {
        self.pend_one || !self.pend_words.is_empty()
    }

    /// Something besides direct transactions is left to finish (`finish_signal`).
    #[inline(always)]
    pub fn unfinished(&self) -> bool {
        !self.dirty.is_empty() || !self.fast_dirty.is_empty() || self.fuse_n != 0
    }

    /// The cycle's quick transactions are applied: the next cycle may finish some at once again.
    #[inline(always)]
    pub fn end_apply(&mut self) -> bool {
        self.fuse_n = 0;
        std::mem::take(&mut self.fused_event)
    }

    /// `finish_one` for element `e` of the sole source, given its new `code`.
    #[inline]
    fn finish_quick(&mut self, e: usize, code: u8, now: Time, cycle: u64) {
        // A plain signal is logic: three plane writes and the code byte, no storage dispatch.
        let (Some(drv), Some(cur), Some(last)) = (
            self.driving.planes_mut(),
            self.cur.planes_mut(),
            self.last_value.planes_mut(),
        ) else {
            unreachable!("finish_quick of byte storage")
        };
        put_plane_code(drv, e, code);
        let old = match self.codes.get_mut(e) {
            Some(c) => std::mem::replace(c, code),
            None => plane_code(cur, e),
        };
        if old == code {
            return;
        }
        put_plane_code(cur, e, code);
        put_plane_code(last, e, old);
        set_event(&mut self.stamps, self.ty.len as usize, e, cycle, now);
        self.note_run(e, 1);
        self.fused_event = true;
    }

    /// `compute_driving` and `update_effective` of the one logic element `e`, when its only
    /// source is a driver and nothing is layered on it (the case of a process per element of a
    /// vector): a code compare and four plane bits instead of the general paths. `None`: take
    /// the general ones.
    pub fn finish_one(&mut self, e: u32, now: Time, cycle: u64) -> Option<bool> {
        if !matches!(self.ty.elem, Elem::Logic)
            || !self.groups.is_empty()
            || self.guard.is_some()
            || !self.force_ev.is_empty()
            || self.force_drv.is_some()
            || self.port_in.is_some()
            || self.force_eff.is_some()
            || !matches!(self.resolution, None | Some(Resolution::StdLogic))
        {
            return None;
        }
        let h = self.seg_hint;
        let hit = |i: usize| {
            self.segments
                .get(i)
                .is_some_and(|s| s.range.offset <= e && e < s.range.end())
        };
        let i = [h, h + 1, h.wrapping_sub(1)]
            .into_iter()
            .find(|&i| hit(i))
            .unwrap_or_else(|| self.segments.partition_point(|s| s.range.end() <= e));
        let seg = self.segments.get(i)?;
        let ([d], true) = (seg.drivers.as_slice(), hit(i)) else {
            return None;
        };
        let d = &self.drivers[*d as usize];
        let code = d.value.code((e - d.range.offset) as usize);
        self.seg_hint = i;
        let e = e as usize;
        self.driving.put_code(e, code);
        let old = self.cur.swap_code(e, code);
        if old == code {
            return Some(false);
        }
        if let Some(c) = self.codes.get_mut(e) {
            *c = code;
        }
        set_event(&mut self.stamps, self.ty.len as usize, e, cycle, now);
        self.note_run(e, 1);
        self.last_value.put_code(e, old);
        Some(true)
    }

    /// Segment `i` is one element of `r` with one driver of one logic element, copied as it is.
    #[inline]
    fn one_driver_elem(&self, i: usize, r: Range) -> bool {
        let seg = &self.segments[i];
        seg.range.len == 1
            && r.contains(seg.range)
            && matches!(self.ty.elem, Elem::Logic)
            && self.guard.is_none()
            && matches!(self.resolution, None | Some(Resolution::StdLogic))
            && matches!(seg.drivers.as_slice(), [d] if self.drivers[*d as usize].range.len == 1)
    }

    /// Copy the driving values of the run of `one_driver_elem` segments from segment `i` that
    /// stays in one 64-element block; returns how many segments it took.
    fn gather_ones(&mut self, i: usize, r: Range) -> usize {
        let first = self.segments[i].range.offset as usize;
        let room = 64 - first % 64;
        let (mut planes, mut k) = ([0u64; 4], 0);
        while k < room && i + k < self.segments.len() {
            let seg = &self.segments[i + k];
            if seg.range.offset as usize != first + k || !self.one_driver_elem(i + k, r) {
                break;
            }
            let [d] = seg.drivers.as_slice() else {
                break;
            };
            let Some(p) = self.drivers[*d as usize].value.first_planes() else {
                break;
            };
            for (x, b) in planes.iter_mut().zip(p) {
                *x |= b << k;
            }
            k += 1;
        }
        self.driving.put_planes(first, k, planes);
        k
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
        if !layered
            && off.is_multiple_of(64)
            && self.force_ev.is_empty()
            && let Some(any) = self.update_planes(off, n, now, cycle)
        {
            return any;
        }
        let owned;
        let (src, soff) = if layered {
            owned = self.effective_of(r);
            (&owned, 0)
        } else {
            (&self.driving, off)
        };
        let mut any = false;
        let cells = !self.cuts.is_empty();
        for c in (0..n).step_by(64) {
            let k = (n - c).min(64);
            let m = self.cur.diff_chunk(off + c, src, soff + c, k);
            if m == 0 {
                continue;
            }
            any = true;
            // Runs of changed elements: stamps filled and the event list extended per run.
            let mut w = m;
            if !self.watched {
                // Nothing lists the runs: the stamps of each changed element, the runs counted.
                let n = self.ty.len as usize;
                let (cycles, rest) = self.stamps.split_at_mut(2 * n);
                let base = off + c;
                while w != 0 {
                    let e = base + w.trailing_zeros() as usize;
                    w &= w - 1;
                    cycles[e] = cycle;
                    rest[e] = stamp(now);
                }
                self.nruns += u64::from((m & !(m << 1)).count_ones());
            }
            while w != 0 {
                let s = w.trailing_zeros();
                let len = (w >> s).trailing_ones();
                let e = off + c + s as usize;
                let n = self.ty.len as usize;
                // Event cycle and event time of the run, one pass (runs are short).
                let (cycles, rest) = self.stamps.split_at_mut(2 * n);
                let times = &mut rest[e..e + len as usize];
                for (c, t) in cycles[e..e + len as usize].iter_mut().zip(times) {
                    *c = cycle;
                    *t = stamp(now);
                }
                if !cells {
                    note_run(
                        &mut self.changed,
                        &mut self.nruns,
                        self.watched,
                        self.fine,
                        e,
                        len,
                    );
                }
                w &= !(nmask(len as usize) << s);
            }
            if cells {
                note_cells(&mut self.changed, &mut self.nruns, &self.cuts, off + c, m);
            }
            let (lo, hi) = (m.trailing_zeros() as usize, 64 - m.leading_zeros() as usize);
            if k == 1 {
                // m is 1: one element changed.
                self.last_value.copy(off + c, &self.cur, off + c, 1);
                self.cur.copy(off + c, src, soff + c, 1);
            } else {
                self.last_value
                    .select(off + c, &self.cur, off + c, k, &[m], Some(0));
                self.cur.select(off + c, src, soff + c, k, &[m], Some(0));
            }
            refresh_codes(&mut self.codes, &self.cur, off + c + lo, off + c + hi);
        }
        if !self.force_ev.is_empty() {
            any |= self.force_events(r, now, cycle);
        }
        any
    }

    /// `update_effective` of `off .. off + n` (`off` on a block boundary) of a logic signal
    /// whose effective value is its driving value: whole plane words instead of the element
    /// helpers. `None` for byte storage.
    fn update_planes(&mut self, off: usize, n: usize, now: Time, cycle: u64) -> Option<bool> {
        if !matches!(
            (&self.cur, &self.driving, &self.last_value),
            (Buf::Planes { .. }, Buf::Planes { .. }, Buf::Planes { .. })
        ) {
            return None;
        }
        let mut any = false;
        for c in (0..n).step_by(64) {
            any |= self.update_word((off + c) / 64, nmask((n - c).min(64)), now, cycle);
        }
        Some(any)
    }

    /// `update_effective` of the elements `m` of plane word `w` of a plain logic signal: the
    /// changed ones by XOR, `last_value` and `cur` blended, then the stamps, runs and codes.
    #[inline(always)]
    fn update_word(&mut self, w: usize, m: u64, now: Time, cycle: u64) -> bool {
        let (Buf::Planes { b: cur, .. }, Buf::Planes { b: drv, .. }, Buf::Planes { b: last, .. }) =
            (&mut self.cur, &self.driving, &mut self.last_value)
        else {
            return false;
        };
        let (Some(cw), Some(&y), Some(lw)) = (cur.get_mut(w), drv.get(w), last.get_mut(w)) else {
            return false;
        };
        let x = *cw;
        let m = ((x[0] ^ y[0]) | (x[1] ^ y[1]) | (x[2] ^ y[2]) | (x[3] ^ y[3])) & m;
        if m == 0 {
            return false;
        }
        let l = *lw;
        *lw = std::array::from_fn(|p| l[p] & !m | x[p] & m);
        let c: Block = std::array::from_fn(|p| x[p] & !m | y[p] & m);
        *cw = c;
        let len = self.ty.len as usize;
        let base = w * 64;
        let st = self.stamps.as_mut_slice();
        let t = stamp(now);
        // Event cycle and time of the changed elements, then the runs.
        if self.lean {
            // Only the block's last event: this cycle's grows, an older one is replaced.
            if let Some(r) = self.blk_last.get_mut(w) {
                if r.cycle == cycle {
                    r.mask |= m;
                } else {
                    *r = EvRec {
                        cycle,
                        time: t,
                        mask: m,
                    };
                }
            }
        } else if let Some(b) = self.blk_event.get_mut(w) {
            // The changed elements leave the older records, which go once empty; records
            // stay in cycle order, so this cycle's, if any, is the last.
            b.retain(|r| {
                r.mask &= !m;
                r.mask != 0
            });
            match b.last_mut() {
                Some(x) if x.cycle == cycle => x.mask |= m,
                _ => b.push(EvRec {
                    cycle,
                    time: t,
                    mask: m,
                }),
            }
        } else {
            let mut v = m;
            while v != 0 {
                let e = base + v.trailing_zeros() as usize;
                v &= v - 1;
                st[e] = cycle;
                st[2 * len + e] = t;
            }
        }
        if self.watched && !self.cuts.is_empty() {
            note_cells(&mut self.changed, &mut self.nruns, &self.cuts, base, m);
        } else if self.watched && self.fine {
            let mut v = m;
            while v != 0 {
                let s = v.trailing_zeros();
                let run = (v >> s).trailing_ones();
                note_run(
                    &mut self.changed,
                    &mut self.nruns,
                    true,
                    true,
                    base + s as usize,
                    run,
                );
                v &= !(nmask(run as usize) << s);
            }
        } else if self.watched {
            // Only whole-signal waiters: the cover of the changed elements tells them all
            // they need; the runs are counted.
            let (lo, hi) = (m.trailing_zeros() as usize, 64 - m.leading_zeros() as usize);
            let runs = u64::from((m & !(m << 1)).count_ones());
            cover(
                &mut self.changed,
                &mut self.nruns,
                base + lo,
                (hi - lo) as u32,
                runs,
            );
        } else {
            self.nruns += u64::from((m & !(m << 1)).count_ones());
        }
        if m.is_power_of_two() {
            let i = m.trailing_zeros() as usize;
            if let Some(code) = self.codes.get_mut(base + i) {
                *code = ((c[0] >> i & 1)
                    | (c[1] >> i & 1) << 1
                    | (c[2] >> i & 1) << 2
                    | (c[3] >> i & 1) << 3) as u8;
            }
        } else {
            refresh_changed_codes(&mut self.codes, &c, base, m);
        }
        true
    }

    /// Whether `commit_code` of `code` to element `elem` through the direct driver in `slot` would
    /// find the driver and the signal holding it already, on a signal that allows skipping.
    #[inline(always)]
    pub(crate) fn direct_same(&self, slot: u32, elem: u32, code: u8) -> bool {
        if !self.skip_ok || !held(&self.codes, &self.cur, elem as usize, code) {
            return false;
        }
        if self.exact {
            let d = &self.drivers[slot as usize];
            matches!(&d.value, Buf::Planes { b, .. } if plane_is(b, (elem - d.range.offset) as usize, code))
        } else {
            matches!(&self.driving, Buf::Planes { b, .. } if plane_is(b, elem as usize, code))
        }
    }

    /// [`Signal::direct_same`] for `len` elements from `elem` (at most 64): `planes`, masked to
    /// `len`, is what `cur` and the driving value (the driver's, after a deposit) hold there.
    #[inline]
    pub(crate) fn direct_same_planes(
        &self,
        slot: u32,
        elem: u32,
        len: u32,
        planes: [u64; 4],
    ) -> bool {
        let (e, n) = (elem as usize, len as usize);
        if !self.skip_ok {
            return false;
        }
        let Buf::Planes { b: cur, .. } = &self.cur else {
            return false;
        };
        if yavs_logic::wide::get_block(cur, e, n) != planes {
            return false;
        }
        if self.exact {
            let d = &self.drivers[slot as usize];
            matches!(&d.value, Buf::Planes { b, .. }
                if yavs_logic::wide::get_block(b, e - d.range.offset as usize, n) == planes)
        } else {
            matches!(&self.driving, Buf::Planes { b, .. }
                if yavs_logic::wide::get_block(b, e, n) == planes)
        }
    }

    /// A zero-delay transaction of one element of a direct driver (`DriverInfo::direct`),
    /// committed: the driver value and the driving value now (the update phase is over, and
    /// nothing reads them before the next one), the element waiting in `pend` for the next
    /// update (`finish_pend`). True for the signal's first one. `None`: the driver and the
    /// signal already have the value and the signal allows skipping (`skip_ok`): nothing to
    /// write or finish, only the delta it causes.
    #[inline(always)]
    pub(crate) fn commit_code(&mut self, slot: u32, elem: u32, code: u8) -> Option<bool> {
        let same = self.skip_ok && held(&self.codes, &self.cur, elem as usize, code);
        if self.exact {
            let d = &mut self.drivers[slot as usize];
            if let Buf::Planes { b, .. } = &mut d.value {
                let rel = (elem - d.range.offset) as usize;
                if same && plane_is(b, rel, code) {
                    return None;
                }
                put_plane_code(b, rel, code);
            }
        }
        // The driver is the element's only source: the driving value is its value (`lag`).
        if let Buf::Planes { b, .. } = &mut self.driving {
            if !self.exact && same && plane_is(b, elem as usize, code) {
                return None;
            }
            put_plane_code(b, elem as usize, code);
        }
        self.lag |= !self.exact;
        // The signal's first element of the cycle stays in `last_commit` alone (`pend_one`);
        // a second one moves it to the bit mask.
        if !self.pend_one && self.pend_words.is_empty() {
            self.pend_one = true;
            self.last_commit = (elem, code);
            return Some(true);
        }
        self.spill_pend_one();
        self.last_commit = (elem, code);
        Some(self.mark_pend(elem, 1))
    }

    /// A zero-delay transaction of one element of a byte signal's direct driver
    /// (`DriverInfo::bdirect`), committed as `commit_code` commits a logic one: the driving
    /// value now (the driver value too after a deposit), the element waiting for the next update
    /// (`finish_pend`). True for the signal's first one; `None` when the driver and the signal
    /// already have the bytes and the signal allows skipping.
    #[inline]
    pub(crate) fn commit_bytes(
        &mut self,
        slot: u32,
        elem: u32,
        es: usize,
        val: &[u8; 8],
    ) -> Option<bool> {
        let v = &val[..es];
        let at = elem as usize * es;
        let (Buf::Bytes { data: drv, .. }, Buf::Bytes { data: cur, .. }) =
            (&mut self.driving, &self.cur)
        else {
            unreachable!("commit_bytes of logic storage")
        };
        let same = bytes_eq(&cur[at..at + es], v);
        if self.exact {
            let d = &mut self.drivers[slot as usize];
            let rel = (elem - d.range.offset) as usize * es;
            if let Buf::Bytes { data, .. } = &mut d.value {
                if same && self.skip_ok && bytes_eq(&data[rel..rel + es], v) {
                    return None;
                }
                data[rel..rel + es].copy_from_slice(v);
            }
        } else if same && self.skip_ok && bytes_eq(&drv[at..at + es], v) {
            return None;
        }
        put_bytes(&mut drv[at..at + es], v);
        self.lag |= !self.exact;
        // The first element of the cycle stays in `last_commit` alone (`pend_one`).
        if !self.pend_one && self.pend_words.is_empty() {
            self.pend_one = true;
            self.last_commit = (elem, 0);
            return Some(true);
        }
        self.spill_pend_one();
        self.last_commit = (elem, 0);
        Some(self.mark_pend(elem, 1))
    }

    /// `commit_code` of entries of this signal in order (a vector with a process per element that
    /// ran in one batch), its storage looked up once. True when the signal had nothing pending
    /// and has now.
    pub(crate) fn commit_codes(&mut self, outs: &[crate::process::Out], empty: &mut bool) -> bool {
        let first = !self.pend_one && self.pend_words.is_empty();
        self.spill_pend_one();
        if self.pend.is_empty() {
            self.pend = vec![0; (self.ty.len as usize).div_ceil(64)];
        }
        let exact = self.exact;
        self.lag |= !exact;
        let Signal {
            drivers,
            driving,
            cur,
            codes,
            pend,
            pend_words,
            last_commit,
            skip_ok,
            ..
        } = self;
        let is_held = |e: usize, code: u8| held(codes, cur, e, code);
        let Buf::Planes { b: drv, .. } = driving else {
            return false;
        };
        let (drv, pend): (&mut [Block], &mut [u64]) = (drv, pend);
        for o in outs {
            let (e, code) = (o.elem as usize, o.code as u8);
            if exact {
                let d = &mut drivers[o.slot as usize];
                let Buf::Planes { b: dv, .. } = &mut d.value else {
                    continue;
                };
                let rel = e - d.range.offset as usize;
                if *skip_ok && is_held(e, code) && plane_is(dv, rel, code) {
                    *empty = true;
                    continue;
                }
                put_plane_code(dv, rel, code);
            } else if *skip_ok && is_held(e, code) && plane_is(drv, e, code) {
                *empty = true;
                continue;
            }
            put_plane_code(drv, e, code);
            let w = e / 64;
            if pend[w] == 0 {
                pend_words.push(w as u32);
            }
            pend[w] |= 1 << (e % 64);
            *last_commit = (o.elem, code);
        }
        first && !pend_words.is_empty()
    }

    /// The driving values the direct commits wrote back into the drivers they came from (all
    /// sole sources, `lag`), before something reads the drivers or makes the driving value
    /// differ from them.
    pub(crate) fn sync_drivers(&mut self) {
        if !std::mem::take(&mut self.lag) {
            return;
        }
        let Signal {
            drivers, driving, ..
        } = self;
        for d in drivers.iter_mut().filter(|d| !d.shared) {
            d.value
                .copy(0, driving, d.range.offset as usize, d.range.len as usize);
        }
    }

    /// The element `pend_one` holds, into the bit mask.
    #[inline(always)]
    fn spill_pend_one(&mut self) {
        if std::mem::take(&mut self.pend_one) {
            self.mark_pend(self.last_commit.0, 1);
        }
    }

    /// `commit_code` of at most 64 elements given as plane words.
    #[inline]
    pub(crate) fn commit_planes(
        &mut self,
        slot: u32,
        elem: u32,
        len: u32,
        planes: [u64; 4],
    ) -> bool {
        if self.exact {
            let d = &mut self.drivers[slot as usize];
            d.put_planes((elem - d.range.offset) as usize, len as usize, planes);
        }
        self.lag |= !self.exact;
        if let Buf::Planes { b, .. } = &mut self.driving {
            put_block(b, elem as usize, len as usize, planes, !0);
        }
        let first = !self.pend_one;
        self.spill_pend_one();
        self.last_commit.0 = u32::MAX;
        self.mark_pend(elem, len) && first
    }

    #[inline(always)]
    fn mark_pend(&mut self, elem: u32, len: u32) -> bool {
        if self.pend.is_empty() {
            self.pend = vec![0; (self.ty.len as usize).div_ceil(64)];
        }
        let first = self.pend_words.is_empty();
        mark_bits(&mut self.pend, &mut self.pend_words, elem, len);
        first
    }

    /// `commit_planes` of entries of this signal in order (processes of the elements of an
    /// array of vectors, in one batch), its storage looked up once. True when the signal had
    /// nothing pending.
    pub(crate) fn commit_planes_run(&mut self, outs: &[crate::process::OutV]) -> bool {
        let first = !self.pend_one && self.pend_words.is_empty();
        self.spill_pend_one();
        self.last_commit.0 = u32::MAX;
        if self.pend.is_empty() {
            self.pend = vec![0; (self.ty.len as usize).div_ceil(64)];
        }
        let exact = self.exact;
        self.lag |= !exact;
        let Signal {
            drivers,
            driving,
            pend,
            pend_words,
            ..
        } = self;
        let Buf::Planes { b: drv, .. } = driving else {
            return false;
        };
        let drv: &mut [Block] = drv;
        for o in outs {
            if exact {
                let d = &mut drivers[o.slot as usize];
                d.put_planes((o.elem - d.range.offset) as usize, o.len as usize, o.planes);
            }
            // Within one plane word (a whole word of a bus, a register of an array of them):
            // one mask for the blend and the pending bits. The planes are masked to the length.
            let (w, sh) = (o.elem as usize / 64, o.elem % 64);
            if sh + o.len <= 64 {
                let m = nmask(o.len as usize) << sh;
                let b = &mut drv[w];
                for (x, p) in b.iter_mut().zip(o.planes) {
                    *x = *x & !m | p << sh;
                }
                if pend[w] == 0 {
                    pend_words.push(w as u32);
                }
                pend[w] |= m;
                continue;
            }
            put_block(drv, o.elem as usize, o.len as usize, o.planes, !0);
            mark_bits(pend, pend_words, o.elem, o.len);
        }
        first
    }

    /// The elements committed directly (`commit_code`, `commit_planes`) matured: active, and
    /// their effective values a word at a time in element order, on a signal with no layer;
    /// otherwise (`slow`, or a force layer) they become dirty ranges for the general paths.
    /// Whether any changed, `None` with nothing pending.
    #[inline(always)]
    pub(crate) fn finish_pend(&mut self, now: Time, cycle: u64, slow: bool) -> Option<bool> {
        let plain = !slow
            && self.force_eff.is_none()
            && self.force_drv.is_none()
            && self.force_ev.is_empty();
        if !plain {
            // The general paths compute the driving value from the drivers.
            self.sync_drivers();
        }
        if matches!(self.ty.elem, Elem::Bytes(_)) {
            return self.finish_pend_bytes(now, cycle, plain);
        }
        // One element (one stage of a chain): no bit mask.
        if self.pend_one {
            let (e, code) = self.last_commit;
            if plain && let Some(&old) = self.codes.get(e as usize) {
                self.pend_one = false;
                return Some(self.finish_code(e as usize, code, old, now, cycle));
            }
            self.spill_pend_one();
        }
        // A lean bus that keeps no codes and no activity stamps: the words in one pass.
        if plain
            && self.lean
            && self.inactive
            && self.codes.is_empty()
            && matches!(
                (&self.cur, &self.driving, &self.last_value),
                (Buf::Planes { .. }, Buf::Planes { .. }, Buf::Planes { .. })
            )
        {
            if self.pend_words.is_empty() {
                return None;
            }
            let mut words = std::mem::take(&mut self.pend_words);
            // Runs are listed in element order for the waiters of parts (cells are sorted and
            // joined by `canonical_changed`).
            if self.fine && self.cuts.is_empty() {
                order_words(&mut words);
            }
            let any = self.finish_words_lean(&words, now, cycle);
            words.clear();
            self.pend_words = words;
            return Some(any);
        }
        // One word (a register, or one stage of a chain): no list to sort or walk.
        if let [w] = self.pend_words[..] {
            self.pend_words.clear();
            let m = std::mem::take(&mut self.pend[w as usize]);
            return Some(self.finish_word(w as usize, m, now, cycle, plain));
        }
        if self.pend_words.is_empty() {
            return None;
        }
        let mut words = std::mem::take(&mut self.pend_words);
        // Each word is finished on its own; only the changed runs and dirty ranges want
        // element order.
        if !plain || self.watched {
            order_words(&mut words);
        }
        let mut any = false;
        for &w in &words {
            let m = std::mem::take(&mut self.pend[w as usize]);
            any |= self.finish_word(w as usize, m, now, cycle, plain);
        }
        words.clear();
        self.pend_words = words;
        Some(any)
    }

    /// `finish_pend` of a byte signal: each pending element's driving bytes against its current
    /// ones (an event if they differ), or, with a layer on the signal, dirty ranges for the
    /// general paths.
    fn finish_pend_bytes(&mut self, now: Time, cycle: u64, plain: bool) -> Option<bool> {
        let one = |s: &mut Signal, e: usize| -> bool {
            if plain {
                s.finish_bytes_elem(e, now, cycle)
            } else {
                let r = Range::new(e as u32, 1);
                s.mark_active(r, now, cycle);
                s.push_dirty(r);
                false
            }
        };
        if self.pend_one {
            self.pend_one = false;
            return Some(one(self, self.last_commit.0 as usize));
        }
        if self.pend_words.is_empty() {
            return None;
        }
        let mut words = std::mem::take(&mut self.pend_words);
        if !plain || self.watched {
            order_words(&mut words);
        }
        let mut any = false;
        for &w in &words {
            let mut m = std::mem::take(&mut self.pend[w as usize]);
            while m != 0 {
                let e = w as usize * 64 + m.trailing_zeros() as usize;
                m &= m - 1;
                any |= one(self, e);
            }
        }
        words.clear();
        self.pend_words = words;
        Some(any)
    }

    /// An element of a plain byte signal committed directly: active, and an event when its
    /// driving bytes differ from the current ones.
    #[inline]
    fn finish_bytes_elem(&mut self, e: usize, now: Time, cycle: u64) -> bool {
        let n = self.ty.len as usize;
        if !self.inactive {
            let st = self.stamps.as_mut_slice();
            st[n + e] = cycle;
            st[3 * n + e] = stamp(now);
        }
        let (
            Buf::Bytes { es, data: drv },
            Buf::Bytes { data: cur, .. },
            Buf::Bytes { data: last, .. },
        ) = (&self.driving, &mut self.cur, &mut self.last_value)
        else {
            unreachable!("finish_bytes_elem of logic storage")
        };
        let es = *es;
        let (d, c) = (&drv[e * es..(e + 1) * es], &mut cur[e * es..(e + 1) * es]);
        if bytes_eq(c, d) {
            return false;
        }
        put_bytes(&mut last[e * es..(e + 1) * es], c);
        put_bytes(c, d);
        set_event(&mut self.stamps, n, e, cycle, now);
        self.note_run(e, 1);
        true
    }

    /// `finish_pend` when only the element `pend_one` holds is pending, on a plain signal with
    /// nothing else to finish: whether it changed. `None` (nothing done) otherwise.
    #[inline(always)]
    pub(crate) fn finish_pend_one(&mut self, now: Time, cycle: u64) -> Option<bool> {
        let (e, code) = self.last_commit;
        if !self.pend_one
            || self.force_eff.is_some()
            || self.force_drv.is_some()
            || !self.force_ev.is_empty()
            || self.unfinished()
        {
            return None;
        }
        if matches!(self.ty.elem, Elem::Bytes(_)) {
            self.pend_one = false;
            return Some(self.finish_bytes_elem(e as usize, now, cycle));
        }
        let &old = self.codes.get(e as usize)?;
        self.pend_one = false;
        Some(self.finish_code(e as usize, code, old, now, cycle))
    }

    /// `finish_pend` of one element `e` whose driving value is `code` and current value `old`
    /// (a plain logic signal that keeps `codes`).
    #[inline(always)]
    fn finish_code(&mut self, e: usize, code: u8, old: u8, now: Time, cycle: u64) -> bool {
        let n = self.ty.len as usize;
        if !self.inactive {
            let st = self.stamps.as_mut_slice();
            st[n + e] = cycle;
            st[3 * n + e] = stamp(now);
        }
        if code == old {
            return false;
        }
        self.codes[e] = code;
        if let (Some(cur), Some(last)) = (self.cur.planes_mut(), self.last_value.planes_mut()) {
            put_plane_code(cur, e, code);
            put_plane_code(last, e, old);
        }
        set_event(&mut self.stamps, n, e, cycle, now);
        self.note_run(e, 1);
        true
    }

    /// `finish_word` of each pending word of a lean signal (`blk_last`) that keeps no codes and no
    /// activity stamps, on a plain signal: `update_word` with what does not depend on the word
    /// looked up once. The words are independent, so their order does not matter. Planes only.
    fn finish_words_lean(&mut self, words: &[u32], now: Time, cycle: u64) -> bool {
        let Signal {
            cur,
            driving,
            last_value,
            blk_last,
            pend,
            changed,
            nruns,
            watched,
            fine,
            cuts,
            ..
        } = self;
        let (Buf::Planes { b: cur, .. }, Buf::Planes { b: drv, .. }, Buf::Planes { b: last, .. }) =
            (cur, &*driving, last_value)
        else {
            return false;
        };
        let (cur, drv, last): (&mut [Block], &[Block], &mut [Block]) = (cur, drv, last);
        let t = stamp(now);
        let (mut any, mut lo, mut hi, mut runs) = (false, usize::MAX, 0, 0);
        for &w in words {
            let w = w as usize;
            let pm = std::mem::take(&mut pend[w]);
            let (Some(cw), Some(&y), Some(lw)) = (cur.get_mut(w), drv.get(w), last.get_mut(w))
            else {
                continue;
            };
            let x = *cw;
            let m = ((x[0] ^ y[0]) | (x[1] ^ y[1]) | (x[2] ^ y[2]) | (x[3] ^ y[3])) & pm;
            if m == 0 {
                continue;
            }
            let l = *lw;
            *lw = std::array::from_fn(|p| l[p] & !m | x[p] & m);
            *cw = std::array::from_fn(|p| x[p] & !m | y[p] & m);
            if let Some(r) = blk_last.get_mut(w) {
                if r.cycle == cycle {
                    r.mask |= m;
                } else {
                    *r = EvRec {
                        cycle,
                        time: t,
                        mask: m,
                    };
                }
            }
            any = true;
            let base = w * 64;
            if *watched && !cuts.is_empty() {
                note_cells(changed, nruns, cuts, base, m);
            } else if *watched && *fine {
                let mut v = m;
                while v != 0 {
                    let s = v.trailing_zeros();
                    let run = (v >> s).trailing_ones();
                    note_run(changed, nruns, true, true, base + s as usize, run);
                    v &= !(nmask(run as usize) << s);
                }
            } else {
                runs += u64::from((m & !(m << 1)).count_ones());
                lo = lo.min(base + m.trailing_zeros() as usize);
                hi = hi.max(base + 64 - m.leading_zeros() as usize);
            }
        }
        if any && !(*watched && *fine) {
            // Whole-signal waiters: the cover of the changed elements tells them all they need.
            if *watched {
                cover(changed, nruns, lo, (hi - lo) as u32, runs);
            } else {
                *nruns += runs;
            }
        }
        any
    }

    /// `finish_pend` of the elements `m` of word `w`.
    #[inline(always)]
    fn finish_word(&mut self, w: usize, m: u64, now: Time, cycle: u64, plain: bool) -> bool {
        let base = w * 64;
        // One element: its codes, as a quick transaction finished at once.
        if plain && m.is_power_of_two() {
            let e = base + m.trailing_zeros() as usize;
            if let (Some(&old), Buf::Planes { b: drv, .. }) = (self.codes.get(e), &self.driving) {
                // The code the last one-element commit wrote, if it was this element.
                let code = match self.last_commit {
                    (le, c) if le as usize == e => c,
                    _ => plane_code(drv, e),
                };
                return self.finish_code(e, code, old, now, cycle);
            }
        }
        if plain {
            if !self.inactive {
                let (n, t) = (self.ty.len as usize, stamp(now));
                let st = self.stamps.as_mut_slice();
                let mut v = m;
                while v != 0 {
                    let s = v.trailing_zeros() as usize;
                    let run = (v >> s).trailing_ones() as usize;
                    let e = base + s;
                    st[n + e..n + e + run].fill(cycle);
                    st[3 * n + e..3 * n + e + run].fill(t);
                    v &= !(nmask(run) << s);
                }
            }
            return self.update_word(w, m, now, cycle);
        }
        let mut v = m;
        while v != 0 {
            let s = v.trailing_zeros();
            let run = (v >> s).trailing_ones();
            let r = Range::new((base + s as usize) as u32, run);
            self.mark_active(r, now, cycle);
            self.push_dirty(r);
            v &= !(nmask(run as usize) << s);
        }
        false
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
            // The parts of `f` outside `r` wait for the update of their own range.
            let (end, pend) = (f.offset + f.len, part.offset + part.len);
            if part.offset > f.offset {
                self.force_ev
                    .push(Range::new(f.offset, part.offset - f.offset));
            }
            if end > pend {
                self.force_ev.push(Range::new(pend, end - pend));
            }
            for e in part.elems() {
                if self.event_cycle(e) == cycle {
                    continue;
                }
                any = true;
                set_event(&mut self.stamps, self.ty.len as usize, e, cycle, now);
                self.note_run(e, 1);
            }
        }
        any
    }

    /// Apply the transactions of one driver (`w`, its waveform; `slot` its index in `drivers`)
    /// that are due, marking the elements active and dirty.
    pub fn apply_driver(&mut self, w: &mut Waveform, slot: u32, now: Time, cycle: u64) {
        if let Some(q) = w.quick
            && q.time <= now
        {
            w.quick = None;
            self.apply_quick(q, slot, now, cycle);
            return;
        }
        if let Some(q) = w.bquick
            && q.time <= now
        {
            w.bquick = None;
            self.apply_bquick(q, slot, now, cycle);
            return;
        }
        self.apply_rest(w, slot, now, cycle);
    }

    /// The due byte transaction `q` of driver `slot`: as `apply_quick`.
    fn apply_bquick(&mut self, q: BQuick, slot: u32, now: Time, cycle: u64) {
        let (es, e) = (q.es as usize, q.elem as usize);
        let d = &mut self.drivers[slot as usize];
        let rel = (q.elem - d.range.offset) as usize;
        if let Buf::Bytes { data, .. } = &mut d.value {
            data[rel * es..(rel + 1) * es].copy_from_slice(&q.val[..es]);
        }
        if let Some(off) = &mut d.off {
            fill_bits(off, rel, 1, false);
        }
        let shared = d.shared;
        let n = self.ty.len as usize;
        let st = self.stamps.as_mut_slice();
        st[n + e] = cycle;
        st[3 * n + e] = stamp(now);
        if !shared && self.plain_bytes() {
            // The element's only source: its driving and effective values follow now.
            self.fuse_n += 1;
            self.finish_bquick(e, &q.val[..es], now, cycle);
        } else {
            self.push_dirty(Range::new(q.elem, 1));
        }
    }

    /// `finish_quick` for an element of byte storage.
    fn finish_bquick(&mut self, e: usize, v: &[u8], now: Time, cycle: u64) {
        let es = v.len();
        let (
            Buf::Bytes { data: drv, .. },
            Buf::Bytes { data: cur, .. },
            Buf::Bytes { data: last, .. },
        ) = (&mut self.driving, &mut self.cur, &mut self.last_value)
        else {
            unreachable!("finish_bquick of logic storage")
        };
        drv[e * es..(e + 1) * es].copy_from_slice(v);
        let c = &mut cur[e * es..(e + 1) * es];
        if bytes_eq(c, v) {
            return;
        }
        last[e * es..(e + 1) * es].copy_from_slice(c);
        c.copy_from_slice(v);
        set_event(&mut self.stamps, self.ty.len as usize, e, cycle, now);
        self.note_run(e, 1);
        self.fused_event = true;
    }

    /// The part of `plain` that holds for the whole run: a logic signal with no group, guard,
    /// port layer or resolution function but `std_logic`'s.
    pub(crate) fn set_static_plain(&mut self) {
        self.static_plain = matches!(self.ty.elem, Elem::Logic)
            && self.groups.is_empty()
            && self.guard.is_none()
            && self.port_in.is_none()
            && matches!(self.resolution, None | Some(Resolution::StdLogic));
    }

    /// A byte signal with no layer, group, guard or resolution function to wait for: a sole
    /// source's value is the signal's (`plain` for byte storage).
    #[inline(always)]
    fn plain_bytes(&self) -> bool {
        matches!(self.ty.elem, Elem::Bytes(_))
            && self.groups.is_empty()
            && self.guard.is_none()
            && self.force_ev.is_empty()
            && self.force_drv.is_none()
            && self.port_in.is_none()
            && self.force_eff.is_none()
            && self.resolution.is_none()
    }

    /// The due quick transaction `q` of driver `slot` (taken from its waveform): the driver
    /// value, the activity, and the driving and effective values or a dirty range.
    #[inline(always)]
    pub(crate) fn apply_quick(&mut self, q: Quick, slot: u32, now: Time, cycle: u64) {
        let d = &mut self.drivers[slot as usize];
        let rel = (q.elem - d.range.offset) as usize;
        d.value.put_code(rel, q.code);
        if let Some(off) = &mut d.off {
            fill_bits(off, rel, 1, false);
        }
        let shared = d.shared;
        let (n, e) = (self.ty.len as usize, q.elem as usize);
        let st = self.stamps.as_mut_slice();
        st[n + e] = cycle;
        st[3 * n + e] = stamp(now);
        if !shared && self.fuse_ok() {
            // The element's only source: its driving and effective values follow now.
            self.finish_quick(e, q.code, now, cycle);
        } else {
            self.push_dirty(Range::new(q.elem, 1));
        }
    }

    /// `apply_driver` without a quick transaction.
    fn apply_rest(&mut self, w: &mut Waveform, slot: u32, now: Time, cycle: u64) {
        if let Some(q) = w.vquick
            && q.time <= now
        {
            w.vquick = None;
            let d = &mut self.drivers[slot as usize];
            d.put_planes((q.elem - d.range.offset) as usize, q.len as usize, q.planes);
            let (sole, r) = (!d.shared, Range::new(q.elem, q.len));
            if sole && self.plain() {
                // The elements' only source: the driving value follows now, the activity and
                // the effective value in `finish_signal` (one pass over the joined runs).
                if let Buf::Planes { b, .. } = &mut self.driving {
                    put_block(b, q.elem as usize, q.len as usize, q.planes, !0);
                }
                push_range(&mut self.fast_dirty, r);
            } else {
                self.mark_active(r, now, cycle);
                self.push_dirty(r);
            }
            return;
        }
        // The usual case of a vector driver: one transaction, due now.
        if let [t] = w.pending.as_slice()
            && t.time <= now
            && let Some(t) = w.pending.pop()
        {
            let r = t.range;
            apply_txn(&mut self.drivers[slot as usize], t, &mut w.spare);
            self.mark_active(r, now, cycle);
            self.push_dirty(r);
            return;
        }
        let mut ranges: SmallVec<[Range; 4]> = SmallVec::new();
        w.apply_due(now, &mut self.drivers[slot as usize], |r| ranges.push(r));
        for r in ranges.drain(..) {
            self.mark_active(r, now, cycle);
            self.push_dirty(r);
        }
    }

    /// Note that the driving value of `r` must be recomputed. A range that continues the last
    /// one extends it: one process per element of a vector gives one range, not one per element.
    pub fn push_dirty(&mut self, r: Range) {
        push_range(&mut self.dirty, r);
    }

    pub fn mark_active(&mut self, r: Range, now: Time, cycle: u64) {
        let (n, t) = (self.ty.len as usize, stamp(now));
        let (lo, hi) = (r.offset as usize, r.end() as usize);
        // Whole blocks of a wide signal: their block stamps; the elements of the ends.
        let (b0, b1) = (lo.div_ceil(64), hi / 64);
        let mut fill = |a: usize, b: usize| {
            self.stamps[n + a..n + b].fill(cycle);
            self.stamps[3 * n + a..3 * n + b].fill(t);
        };
        if !self.blk_active.is_empty() && b0 < b1 {
            fill(lo, b0 * 64);
            fill(b1 * 64, hi);
            self.blk_active[b0..b1].fill([cycle, t]);
        } else {
            fill(lo, hi);
        }
    }

    /// The cycle of the last activity of element `e`.
    #[inline]
    fn active_cycle(&self, e: usize) -> u64 {
        let own = self.stamps[self.ty.len as usize + e];
        self.blk_active.get(e / 64).map_or(own, |b| own.max(b[0]))
    }

    /// The elements of `r` that are not active in `cycle`, or `None` when all are (a port
    /// passes activity on element by element, LRM 14.7.3.1).
    pub fn idle_elems(&self, r: Range, cycle: u64) -> Option<Vec<bool>> {
        let (n, lo, hi) = (self.ty.len as usize, r.offset as usize, r.end() as usize);
        let own_idle = |a: usize, b: usize| self.stamps[n + a..n + b].iter().any(|&c| c != cycle);
        let any_idle = if self.blk_active.is_empty() {
            own_idle(lo, hi)
        } else {
            block_span(r).any(|k| {
                self.blk_active[k][0] != cycle && own_idle(lo.max(k * 64), hi.min(k * 64 + 64))
            })
        };
        any_idle.then(|| r.elems().map(|e| self.active_cycle(e) != cycle).collect())
    }

    /// `mark_active` of the elements of `r` that `idle` (from `idle_elems`) does not exclude.
    pub fn mark_active_except(&mut self, r: Range, idle: &[bool], now: Time, cycle: u64) {
        let n = self.ty.len as usize;
        for (e, _) in r.elems().zip(idle).filter(|&(_, &i)| !i) {
            self.stamps[n + e] = cycle;
            self.stamps[3 * n + e] = stamp(now);
        }
    }

    /// `'last_value` of `r`: the value before the last change of the whole of `r` (LRM 16.2.4),
    /// so an element that did not change in that cycle keeps its current value.
    pub fn last_value_of(&self, r: Range) -> Buf {
        let mut b = self.last_value.extract(r.offset as usize, r.len as usize);
        let ev: Vec<u64> = r.elems().map(|e| self.event_cycle(e)).collect();
        if let Some(&latest) = ev.iter().max() {
            for (i, _) in ev.iter().enumerate().filter(|&(_, &c)| c != latest) {
                b.copy(i, &self.cur, r.offset as usize + i, 1);
            }
        }
        b
    }

    /// The cycle of the last event of element `e`.
    fn event_cycle(&self, e: usize) -> u64 {
        let own = self.stamps[e];
        let bit = 1 << (e % 64);
        self.block_recs(e / 64).map_or(own, |b| {
            b.iter()
                .find(|x| x.mask & bit != 0)
                .map_or(own, |x| own.max(x.cycle))
        })
    }

    /// The event records of block `k` (at most one when lean), `None` for a signal of up to 64
    /// elements.
    #[inline]
    fn block_recs(&self, k: usize) -> Option<&[EvRec]> {
        if self.lean {
            let r = self.blk_last.get(k)?;
            Some(if r.mask != 0 {
                std::slice::from_ref(r)
            } else {
                &[]
            })
        } else {
            self.blk_event.get(k).map(SmallVec::as_slice)
        }
    }

    /// Whether the signal keeps records per block (more than 64 elements).
    #[inline]
    fn has_blocks(&self) -> bool {
        !(self.blk_event.is_empty() && self.blk_last.is_empty())
    }

    /// The event records of the blocks of `r` that list an element of `r`.
    fn block_events(&self, r: Range) -> impl Iterator<Item = &EvRec> {
        let (lo, hi) = (r.offset as usize, r.end() as usize);
        let span = if self.has_blocks() {
            block_span(r)
        } else {
            0..0
        };
        span.flat_map(move |k| {
            let (a, b) = (lo.max(k * 64) - k * 64, hi.min(k * 64 + 64) - k * 64);
            let m = nmask(b - a) << a;
            self.block_recs(k)
                .unwrap_or_default()
                .iter()
                .filter(move |x| x.mask & m != 0)
        })
    }

    pub fn active_in(&self, r: Range, cycle: u64) -> bool {
        let n = self.ty.len as usize;
        (!self.blk_active.is_empty()
            && self.blk_active[block_span(r)].iter().any(|b| b[0] == cycle))
            || self.stamps[n..2 * n][r.elems()].contains(&cycle)
    }

    /// Keep only the last event of each block of 64 elements (`lean`), for a signal of more
    /// than 64 elements nothing asks the history of single elements of (`'last_event`, and
    /// `'last_value` of more than one element, of a part of a block). Before the first event.
    pub fn set_lean_events(&mut self) {
        if self.ty.len > 64 && !self.lean {
            self.lean = true;
            self.blk_event = Vec::new();
            self.blk_last = vec![EvRec::default(); (self.ty.len as usize).div_ceil(64)];
        }
    }

    /// Whether the signal keeps only each block's last event (`set_lean_events`).
    pub fn is_lean(&self) -> bool {
        self.lean
    }

    /// Whether `'last_event` and `'last_value` of `r` are exact: always, unless the signal is
    /// lean and `r` is not whole blocks (a one-element `'last_value` is the plane's).
    pub fn history_exact(&self, r: Range, last_value: bool) -> bool {
        !self.lean
            || (last_value && r.len == 1)
            || r.len == 0
            || (r.offset.is_multiple_of(64)
                && (r.end().is_multiple_of(64) || r.end() == self.ty.len))
    }

    #[inline]
    pub fn event_in(&self, r: Range, cycle: u64) -> bool {
        // One element (an edge test): a load and a compare, not a scan.
        (if r.len == 1 {
            self.stamps[r.offset as usize] == cycle
        } else {
            self.stamps[r.elems()].contains(&cycle)
        }) || (self.has_blocks() && self.block_events(r).any(|x| x.cycle == cycle))
    }

    /// `'last_event` of `r`: time since the most recent event, `TIME'HIGH` if none.
    pub fn last_event(&self, r: Range, now: Time) -> Time {
        let n = self.ty.len as usize;
        let own = elapsed(now, &self.stamps[2 * n..3 * n][r.elems()]);
        let blk: SmallVec<[u64; 8]> = self.block_events(r).map(|x| x.time).collect();
        own.min(elapsed(now, &blk))
    }

    /// `'last_active` of `r`.
    pub fn last_active(&self, r: Range, now: Time) -> Time {
        let n = self.ty.len as usize;
        let own = elapsed(now, &self.stamps[3 * n..][r.elems()]);
        if self.blk_active.is_empty() {
            return own;
        }
        let blk: SmallVec<[u64; 8]> = self.blk_active[block_span(r)]
            .iter()
            .map(|b| b[1])
            .collect();
        own.min(elapsed(now, &blk))
    }
}

/// The stamp of an element that never had an event or activity.
/// A time as stored in the per-element stamps: its femtoseconds plus one, so that 0 is "never"
/// and a fresh stamp array is all zeros (a large signal's untouched stamps cost no memory).
#[inline(always)]
fn stamp(t: Time) -> u64 {
    t.raw() + 1
}

/// Add `r` to a list of ranges, extending the last one when `r` continues it (ascending, or
/// descending: processes numbered the other way round).
#[inline]
fn push_range(list: &mut Vec<Range>, r: Range) {
    match list.last_mut() {
        Some(l) if l.end() == r.offset => l.len += r.len,
        Some(l) if r.end() == l.offset => {
            l.offset = r.offset;
            l.len += r.len;
        }
        _ => list.push(r),
    }
}

/// The blocks of 64 elements that `r` touches.
fn block_span(r: Range) -> std::ops::Range<usize> {
    if r.len == 0 {
        return 0..0;
    }
    r.offset as usize / 64..(r.end() as usize - 1) / 64 + 1
}

fn set_event(stamps: &mut [u64], n: usize, e: usize, cycle: u64, now: Time) {
    stamps[e] = cycle;
    stamps[2 * n + e] = stamp(now);
}

fn stamps(n: usize) -> SmallVec<[u64; 4]> {
    smallvec![0; 4 * n]
}

fn elapsed(now: Time, times: &[u64]) -> Time {
    times
        .iter()
        .max()
        .filter(|&&t| t != 0)
        .map_or(Time::MAX, |&t| now.since(Time::from_raw(t - 1)))
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
    let rel = |i: u32| (run.offset - drivers[i as usize].range.offset) as usize;
    let (off, n) = (run.offset as usize, run.len as usize);
    match res {
        Resolution::StdLogic => driving.resolve_std_logic_by(off, n, conn.len(), |k| {
            (&drivers[conn[k] as usize].value, rel(conn[k]))
        }),
        Resolution::Func(f) => {
            let codes: Vec<Vec<u8>> = conn
                .iter()
                .map(|&i| drivers[i as usize].value.codes(rel(i), n))
                .collect();
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
    pub pending: Vec<Txn>,
    /// The one pending transaction of a zero-delay one-element logic assignment
    /// (`Ctx::assign_code`), kept apart from `pending` (which is empty then) because it costs
    /// no list entry and no value buffer. It matures in the next update; anything that
    /// schedules or inspects `pending` goes through `spill` or `idle` first.
    pub quick: Option<Quick>,
    /// The vector counterpart of `quick`: a zero-delay assignment of at most 64 logic elements
    /// (`Ctx::assign`, `Ctx::assign_blocks`) as plane words, no list entry, no value buffer.
    /// At most one of `quick`, `vquick` and `pending` is in use; `spill` and `idle` as for `quick`.
    pub vquick: Option<VQuick>,
    /// The byte-signal counterpart: an assignment of one element of at most 8 bytes (an enum,
    /// an integer, a boolean, a real). Same rules as `quick`.
    pub bquick: Option<BQuick>,
    /// Values of applied transactions, reused by the next assignments (no allocation per
    /// transaction in the steady state).
    pub spare: Vec<Buf>,
    /// Zero-delay assignments of this run that changed nothing and were not queued
    /// (`Ctx::assign`, `Config::skip_unchanged`).
    pub phantom: Vec<Range>,
}

/// A pending zero-delay assignment of one `std_ulogic` code to one element.
#[derive(Clone, Copy)]
pub(crate) struct Quick {
    pub time: Time,
    /// The element, of the signal (not of the driver).
    pub elem: u32,
    pub code: u8,
}

/// A pending assignment of one element of a byte signal, `es` (at most 8) bytes in `val`.
#[derive(Clone, Copy)]
pub(crate) struct BQuick {
    pub time: Time,
    pub elem: u32,
    pub es: u8,
    pub val: [u8; 8],
}

/// A pending zero-delay assignment of at most 64 logic elements.
#[derive(Clone, Copy)]
pub(crate) struct VQuick {
    pub time: Time,
    /// The first element, of the signal (not of the driver).
    pub elem: u32,
    pub len: u32,
    /// The plane words, element `elem` in bit 0, bits from `len` up zero.
    pub planes: [u64; 4],
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
            for c in (0..k).step_by(64) {
                let m = (k - c).min(64);
                put_bits(&mut eq, to + c, m, !bits(&d, c, m));
            }
        }
        _ => {}
    }
    eq
}

impl Waveform {
    /// Nothing is pending.
    #[inline]
    pub fn idle(&self) -> bool {
        self.pending.is_empty()
            && self.quick.is_none()
            && self.vquick.is_none()
            && self.bquick.is_none()
    }

    /// The time of the first pending transaction.
    #[inline]
    pub fn first_time(&self) -> Option<Time> {
        match (&self.quick, &self.vquick, &self.bquick) {
            (Some(q), _, _) => Some(q.time),
            (_, Some(q), _) => Some(q.time),
            (_, _, Some(q)) => Some(q.time),
            _ => self.pending.first().map(|t| t.time),
        }
    }

    /// Turn the quick transaction into a list entry, for the paths that preempt and merge.
    pub fn spill(&mut self) {
        if let Some(q) = self.bquick.take() {
            self.pending.insert(
                0,
                Txn {
                    time: q.time,
                    range: Range::new(q.elem, 1),
                    value: Some(Buf::Bytes {
                        es: q.es as usize,
                        data: SmallVec::from_slice(&q.val[..q.es as usize]),
                    }),
                },
            );
        }
        if let Some(q) = self.vquick.take() {
            self.pending.insert(
                0,
                Txn {
                    time: q.time,
                    range: Range::new(q.elem, q.len),
                    value: Some(Buf::Planes {
                        len: q.len as usize,
                        b: smallvec![q.planes],
                    }),
                },
            );
        }
        if let Some(q) = self.quick.take() {
            self.pending.insert(
                0,
                Txn {
                    time: q.time,
                    range: Range::new(q.elem, 1),
                    value: Some(Buf::one_code(q.code)),
                },
            );
        }
    }

    /// Apply LRM 10.5.2.2 to target elements `r` for a new waveform whose first transaction is
    /// at `t1`, then insert the new transactions. `reject_from` is `t1 - reject` for inertial
    /// delay, `None` for transport.
    pub fn schedule(&mut self, r: Range, reject_from: Option<Time>, new: &mut TxnList) {
        self.spill();
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
        // Which elements of the overlapping old transactions to delete, newest transaction
        // first, 64 elements at a time: those at or after `t1`, and in the rejection window
        // those from the first one (going back) whose value differs from the new one. `chain`
        // has a bit per element of `r`: every window transaction so far had the new value.
        let mut deleted: Vec<Option<Vec<u64>>> = vec![None; self.pending.len()];
        let mut chain = vec![!0u64; (r.len as usize).div_ceil(64)];
        for (i, t) in self.pending.iter().enumerate().rev() {
            let Some(ov) = t.range.intersect(r) else {
                continue;
            };
            let (to, ro, k) = (
                (ov.offset - t.range.offset) as usize,
                (ov.offset - r.offset) as usize,
                ov.len as usize,
            );
            let mask = || vec![0u64; (t.range.len as usize).div_ceil(64)];
            if t.time >= t1 {
                fill_bits(deleted[i].get_or_insert_with(mask), to, k, true);
            } else if let Some(eq) = &eqs[i] {
                for c in (0..k).step_by(64) {
                    let m = (k - c).min(64);
                    let ch = bits(&chain, ro + c, m) & bits(eq, to + c, m);
                    put_bits(&mut chain, ro + c, m, ch);
                    let del = !ch & nmask(m);
                    if del != 0 {
                        let d = deleted[i].get_or_insert_with(mask);
                        let old = bits(d, to + c, m);
                        put_bits(d, to + c, m, old | del);
                    }
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
        if let Some(q) = self.quick {
            if q.time > now {
                return self.apply_pending(now, d, applied);
            }
            self.quick = None;
            d.value.put_code((q.elem - d.range.offset) as usize, q.code);
            if let Some(off) = &mut d.off {
                fill_bits(off, (q.elem - d.range.offset) as usize, 1, false);
            }
            applied(Range::new(q.elem, 1));
            return;
        }
        if let Some(q) = self.bquick {
            if q.time > now {
                return self.apply_pending(now, d, applied);
            }
            self.bquick = None;
            let (rel, es) = ((q.elem - d.range.offset) as usize, q.es as usize);
            if let Buf::Bytes { data, .. } = &mut d.value {
                data[rel * es..(rel + 1) * es].copy_from_slice(&q.val[..es]);
            }
            if let Some(off) = &mut d.off {
                fill_bits(off, rel, 1, false);
            }
            applied(Range::new(q.elem, 1));
            return;
        }
        if let Some(q) = self.vquick {
            if q.time > now {
                return self.apply_pending(now, d, applied);
            }
            self.vquick = None;
            let rel = (q.elem - d.range.offset) as usize;
            d.put_planes(rel, q.len as usize, q.planes);
            applied(Range::new(q.elem, q.len));
            return;
        }
        self.apply_pending(now, d, applied);
    }

    fn apply_pending(&mut self, now: Time, d: &mut DriverSlot, mut applied: impl FnMut(Range)) {
        let n = self.pending.partition_point(|t| t.time <= now);
        for t in self.pending.drain(..n) {
            applied(t.range);
            apply_txn(d, t, &mut self.spare);
        }
    }
}

/// Push the runs of `t` not marked in `mask`.
fn split_kept(t: Txn, mask: &[u64], out: &mut Vec<Txn>) {
    let n = t.range.len as usize;
    // The length of the run of bits equal to `set` from `k` on, a word at a time.
    let run = |k: usize, set: bool| {
        let mut len = 0;
        while k + len < n {
            let m = (n - k - len).min(64);
            let w = bits(mask, k + len, m);
            let t = (if set { w } else { !w & nmask(m) }).trailing_ones() as usize;
            len += t;
            if t < m {
                break;
            }
        }
        len
    };
    let mut k = run(0, true);
    while k < n {
        let len = run(k, false);
        out.push(Txn {
            time: t.time,
            range: Range::new(t.range.offset + k as u32, len as u32),
            value: t.value.as_ref().map(|v| v.extract(k, len)),
        });
        k += len;
        k += run(k, true);
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
                shared: false,
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
        w.add(1, Range::new(200, 1), Reg::default(), all); // chunk 3
        w.add(2, Range::new(0, 300), Reg::default(), all); // five chunks: the wide list
        w.add(3, Range::new(60, 10), Reg::default(), all); // chunks 0 and 1
        w.add(4, Range::new(100, 1), Reg::default(), all); // chunk 1
        w.add(5, Range::new(0, 0), Reg::default(), all); // null: never woken
        let mut out = Vec::new();
        let procs = |o: &[(u64, u32)]| o.iter().map(|x| x.1).collect::<Vec<_>>();
        w.take_woken(
            &[Range::new(65, 1), Range::new(200, 2)],
            all,
            none,
            &mut out,
            true,
        );
        assert_eq!(procs(&out), [1, 2, 3]);
        // The woken processes resumed (their other copies are stale), and so did 4.
        let live = |w: &Waiter| w.proc > 4;
        w.take_woken(&[Range::new(0, 300)], live, none, &mut out, true);
        assert!(out.is_empty());
        w.add(6, Range::new(299, 1), Reg::default(), all);
        w.take_woken(&[Range::new(0, 300)], live, none, &mut out, true);
        assert_eq!(procs(&out), [6]);
    }

    #[test]
    fn resident_waiters_stay_after_waking() {
        let all = |_: &Waiter| true;
        let mut w = Waiters::new(8);
        w.add(
            1,
            Range::new(0, 1),
            Reg {
                perm: 7,
                ..Reg::default()
            },
            all,
        ); // resident
        w.add(2, Range::new(0, 1), Reg::default(), all);
        let mut out = Vec::new();
        for want in [vec![1, 2], vec![1]] {
            w.take_woken(&[Range::new(0, 1)], all, |_| false, &mut out, true);
            assert_eq!(out.iter().map(|x| x.1).collect::<Vec<_>>(), want);
        }
        // Its process waited for something else: the resident waiter is stale.
        w.take_woken(
            &[Range::new(0, 1)],
            |w: &Waiter| w.perm == 0,
            |_| false,
            &mut out,
            true,
        );
        assert!(out.is_empty());
    }

    #[test]
    fn one_chunk_signal_has_one_waiter_list() {
        let all = |_: &Waiter| true;
        let mut w = Waiters::new(8);
        assert!(w.lists.is_empty());
        w.add(1, Range::new(6, 1), Reg::default(), all);
        assert_eq!(w.lists.len(), 1);
        w.add(2, Range::new(0, 8), Reg::default(), all);
        w.add(3, Range::new(5, 0), Reg::default(), all); // null: never woken
        let mut out = Vec::new();
        w.take_woken(&[Range::new(2, 1)], all, |_| false, &mut out, true);
        assert_eq!(out.iter().map(|x| x.1).collect::<Vec<_>>(), [2]);
        w.take_woken(&[Range::new(6, 1)], all, |_| false, &mut out, true);
        assert_eq!(out.iter().map(|x| x.1).collect::<Vec<_>>(), [1]);
    }

    #[test]
    fn unordered_wake_list_holds_the_same_processes() {
        let all = |_: &Waiter| true;
        let mut w = Waiters::new(300);
        for (p, r) in [
            (1, Range::new(200, 1)),
            (2, Range::new(0, 300)),
            (3, Range::new(60, 10)),
        ] {
            w.add(p, r, Reg::default(), all);
        }
        let runs = [Range::new(65, 1), Range::new(200, 2)];
        let (mut a, mut b) = (Vec::new(), Vec::new());
        w.take_woken(&runs, all, |_| false, &mut a, true);
        let mut w = Waiters::new(300);
        for (p, r) in [
            (1, Range::new(200, 1)),
            (2, Range::new(0, 300)),
            (3, Range::new(60, 10)),
        ] {
            w.add(p, r, Reg::default(), all);
        }
        w.take_woken(&runs, all, |_| false, &mut b, false);
        let set = |o: &[(u64, u32)]| {
            let mut v: Vec<u32> = o.iter().map(|x| x.1).collect();
            v.sort_unstable();
            v.dedup();
            v
        };
        assert_eq!(set(&a), set(&b));
        assert_eq!(set(&a), [1, 2, 3]);
    }

    #[test]
    fn wide_list_waiters_wake_on_many_runs() {
        let all = |_: &Waiter| true;
        let none = |_: &Waiter| false;
        let mut w = Waiters::new(300);
        w.add(1, Range::new(0, 300), Reg::default(), all);
        let runs: Vec<Range> = (0..100).map(|i| Range::new(3 * i, 1)).collect();
        let mut out = Vec::new();
        w.take_woken(&runs, all, none, &mut out, true);
        assert_eq!(out.iter().map(|x| x.1).collect::<Vec<_>>(), [1]);
        // A waiter for one element: the lists of the runs are scanned again.
        w.add(2, Range::new(1, 299), Reg::default(), all);
        w.add(3, Range::new(150, 1), Reg::default(), all);
        w.take_woken(&runs, all, none, &mut out, true);
        assert_eq!(out.iter().map(|x| x.1).collect::<Vec<_>>(), [2, 3]);
        w.take_woken(
            &[Range::new(0, 1), Range::new(2, 1)],
            all,
            none,
            &mut out,
            true,
        );
        assert!(out.is_empty());
    }

    #[test]
    fn activity_of_a_wide_signal_by_blocks() {
        let ty = SignalType::logic(200);
        let mut sig = Signal::new("s".into(), ty, Buf::zeroed(ty.elem, 200), None);
        let r = |o, l| Range::new(o, l);
        // Elements 10 to 159: block 1 by its block stamp, the ends by their own.
        sig.mark_active(r(10, 150), Time::ns(1), 3);
        assert!(sig.active_in(r(70, 1), 3) && sig.active_in(r(10, 1), 3));
        assert!(sig.active_in(r(159, 1), 3) && sig.active_in(r(0, 11), 3));
        assert!(!sig.active_in(r(0, 10), 3) && !sig.active_in(r(160, 40), 3));
        assert_eq!(sig.idle_elems(r(10, 150), 3), None);
        let idle = sig.idle_elems(r(8, 160), 3).unwrap();
        let want: Vec<bool> = (8..168).map(|e| !(10..160).contains(&e)).collect();
        assert_eq!(idle, want);
        assert_eq!(sig.last_active(r(64, 64), Time::ns(4)), Time::ns(3));
        assert_eq!(sig.last_active(r(0, 10), Time::ns(4)), Time::MAX);
        // A later activity of one element in the block wins over the block stamp.
        sig.mark_active(r(100, 1), Time::ns(2), 5);
        assert_eq!(sig.last_active(r(64, 64), Time::ns(4)), Time::ns(2));
        assert_eq!(sig.last_active(r(64, 30), Time::ns(4)), Time::ns(3));
        assert_eq!(sig.idle_elems(r(99, 3), 5), Some(vec![true, false, true]));
        assert!(!sig.active_in(r(64, 30), 5));
        // A whole block again: its block stamp wins over the older element stamp.
        sig.mark_active(r(64, 64), Time::ns(3), 7);
        assert_eq!(sig.idle_elems(r(64, 64), 7), None);
        assert_eq!(sig.last_active(r(100, 1), Time::ns(4)), Time::ns(1));
    }

    /// A lean signal (only each block's last event) answers what it promises like one that
    /// keeps every element's: `'event` of any range, `'last_event` and `'last_value` of whole
    /// blocks, `'last_value` of one element; for other ranges `history_exact` says no.
    #[test]
    fn lean_events_of_a_wide_signal() {
        let ty = SignalType::logic(200);
        let mk = |lean: bool| {
            let mut s = Signal::new("s".into(), ty, Buf::zeroed(ty.elem, 200), None);
            if lean {
                s.set_lean_events();
            }
            s
        };
        let (mut full, mut lean) = (mk(false), mk(true));
        assert!(lean.is_lean() && !full.is_lean());
        let mut x = 0x2545_f491_4f6c_dd1du64;
        for c in 1..80u64 {
            let now = Time::ns(c);
            for e in 0..200usize {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                // Busy and quiet stretches per block, and some elements never change.
                if e % 17 != 3 && x % 5 < (c / 8 + (e as u64 / 64)) % 4 {
                    for s in [&mut full, &mut lean] {
                        let v = if s.driving.code(e) == 2 { 3 } else { 2 };
                        s.driving.put_code(e, v);
                    }
                }
            }
            // Whole signal at once, or word by word (a partial update of a block is possible).
            let parts: &[(u32, u32)] = if c % 3 == 0 {
                &[(0, 64), (64, 64), (128, 72)]
            } else {
                &[(0, 200)]
            };
            for &(o, l) in parts {
                let (a, b) = (
                    full.update_effective(Range::new(o, l), now, c),
                    lean.update_effective(Range::new(o, l), now, c),
                );
                assert_eq!(a, b, "cycle {c}");
            }
            // More elements settle later in the same cycle (another part of the signal).
            if c % 5 == 0 {
                for e in (c as usize % 7..200).step_by(9) {
                    for s in [&mut full, &mut lean] {
                        let v = if s.driving.code(e) == 2 { 3 } else { 2 };
                        s.driving.put_code(e, v);
                    }
                }
                for (o, l) in [(64, 136), (0, 64)] {
                    let r = Range::new(o, l);
                    let (a, b) = (
                        full.update_effective(r, now, c),
                        lean.update_effective(r, now, c),
                    );
                    assert_eq!(a, b, "cycle {c}");
                }
            }
            for (o, l) in [(0, 200), (0, 64), (64, 64), (128, 72), (0, 128), (64, 136)] {
                let r = Range::new(o, l);
                assert!(lean.history_exact(r, false) && lean.history_exact(r, true));
                assert_eq!(lean.event_in(r, c), full.event_in(r, c));
                assert_eq!(lean.last_event(r, now), full.last_event(r, now), "{o}+{l}");
                let (a, b) = (lean.last_value_of(r), full.last_value_of(r));
                assert!((0..l as usize).all(|i| a.code(i) == b.code(i)), "{o}+{l}");
            }
            for (o, l) in [
                (60, 10),
                (63, 2),
                (100, 100),
                (130, 1),
                (1, 199),
                (0, 199),
                (5, 64),
            ] {
                let r = Range::new(o, l);
                assert_eq!(lean.event_in(r, c), full.event_in(r, c), "{o}+{l}");
                assert!(!lean.history_exact(r, false), "{o}+{l}");
                assert_eq!(lean.history_exact(r, true), l == 1, "{o}+{l}");
            }
            for e in 0..200u32 {
                let r = Range::new(e, 1);
                assert_eq!(lean.event_in(r, c), full.event_in(r, c), "element {e}");
                assert!(lean.history_exact(r, true));
                let (a, b) = (lean.last_value_of(r), full.last_value_of(r));
                assert_eq!(a.code(0), b.code(0), "cycle {c} element {e}");
            }
        }
        // A signal of up to 64 elements is not lean: it keeps its stamps.
        let mut small = Signal::new(
            "t".into(),
            SignalType::logic(64),
            Buf::zeroed(Elem::Logic, 64),
            None,
        );
        small.set_lean_events();
        assert!(!small.is_lean());
    }

    #[test]
    fn events_of_a_wide_signal_by_words() {
        // Random changes of a 200-element signal, cycle c at c ns, against a per-element model
        // of the last event; the update takes whole plane words (`update_word`).
        let ty = SignalType::logic(200);
        let mut sig = Signal::new("s".into(), ty, Buf::zeroed(ty.elem, 200), None);
        let mut last = [0u64; 200];
        let mut x = 0x9e37_79b9_7f4a_7c15u64;
        for c in 1..60u64 {
            for (e, l) in last.iter_mut().enumerate() {
                x ^= x << 13;
                x ^= x >> 7;
                x ^= x << 17;
                // Busy and quiet stretches: some cycles change nothing in a block.
                if x % 7 < (c % 4) {
                    let v = if sig.driving.code(e) == 2 { 3 } else { 2 };
                    sig.driving.put_code(e, v);
                    *l = c;
                }
            }
            let now = Time::ns(c);
            let any = sig.update_effective(Range::new(0, 200), now, c);
            assert_eq!(any, last.contains(&c));
            // A second update in the same cycle (another part of the signal settles later).
            if c % 5 == 0 {
                for e in (c as usize % 7..200).step_by(9) {
                    let v = if sig.driving.code(e) == 2 { 3 } else { 2 };
                    sig.driving.put_code(e, v);
                    last[e] = c;
                }
                assert!(sig.update_effective(Range::new(64, 136), now, c));
                assert!(sig.update_effective(Range::new(0, 64), now, c));
            }
            for e in 0..200u32 {
                let r = Range::new(e, 1);
                assert_eq!(
                    sig.event_in(r, c),
                    last[e as usize] == c,
                    "cycle {c} element {e}"
                );
                let want = match last[e as usize] {
                    0 => Time::MAX,
                    l => Time::ns(c - l),
                };
                assert_eq!(sig.last_event(r, now), want, "cycle {c} element {e}");
            }
            for (o, l) in [(0, 200), (60, 10), (63, 2), (100, 100), (130, 1)] {
                let r = Range::new(o, l);
                let span = &last[o as usize..(o + l) as usize];
                assert_eq!(sig.event_in(r, c), span.contains(&c));
                let want = match span.iter().max() {
                    Some(&l) if l > 0 => Time::ns(c - l),
                    _ => Time::MAX,
                };
                assert_eq!(sig.last_event(r, now), want);
            }
            // 'last_value: the elements whose last event is the latest of the range keep the
            // value before it, the others their current value.
            if c % 3 == 0 {
                let r = Range::new(30, 120);
                let lv = sig.last_value_of(r);
                let latest = last[30..150].iter().copied().max().unwrap_or(0);
                for i in 0..120 {
                    let e = 30 + i;
                    let want = if last[e] == latest && latest > 0 {
                        sig.last_value.code(e)
                    } else {
                        sig.cur.code(e)
                    };
                    assert_eq!(lv.code(i), want, "cycle {c} element {e}");
                }
            }
        }
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
    fn preemption_of_wide_transactions_per_element() {
        // Random old transactions over 200 elements against a per-element model of LRM
        // 10.5.2.2: at or after `t1` deleted, in the window deleted from the first value
        // (going back) that differs from the new one.
        let mut x = 0x2545_f491_4f6c_dd1du64;
        let mut rnd = |n: u64| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            x % n
        };
        for round in 0..300 {
            let mut w = Waveform::default();
            let mut model: Vec<Vec<(u64, u8)>> = vec![Vec::new(); 200];
            for _ in 0..1 + rnd(5) {
                let off = rnd(190) as u32;
                let len = 1 + rnd(200 - u64::from(off)) as u32;
                let ns = 5 * (1 + rnd(5));
                let v: Vec<u8> = (0..len).map(|_| rnd(2) as u8).collect();
                let t = txn(ns, off, &v);
                let at = w.pending.partition_point(|p| p.time <= t.time);
                w.pending.insert(at, t);
            }
            for t in &w.pending {
                for (k, e) in t.range.elems().enumerate() {
                    let b = t.value.as_ref().unwrap().bytes(k, 1).unwrap()[0];
                    model[e].push((t.time.raw(), b));
                }
            }
            let off = rnd(150) as u32;
            let r = Range::new(off, 1 + rnd(200 - u64::from(off)) as u32);
            let v: Vec<u8> = (0..r.len).map(|_| rnd(2) as u8).collect();
            let reject = (round % 3 != 0).then(|| Time::ns(5 * rnd(4)));
            let t1 = Time::ns(20);
            for e in r.elems() {
                let new = v[e - off as usize];
                let mut chain = true;
                let mut keep: Vec<(u64, u8)> = Vec::new();
                for &(t, b) in model[e].iter().rev() {
                    let del = if t >= t1.raw() {
                        true
                    } else if reject.is_some_and(|f| t >= f.raw()) {
                        chain &= b == new;
                        !chain
                    } else {
                        false
                    };
                    if !del {
                        keep.insert(0, (t, b));
                    }
                }
                keep.push((t1.raw(), new));
                model[e] = keep;
            }
            w.schedule(r, reject, &mut smallvec![txn(20, off, &v)]);
            let mut got: Vec<Vec<(u64, u8)>> = vec![Vec::new(); 200];
            for t in &w.pending {
                for (k, e) in t.range.elems().enumerate() {
                    let b = t.value.as_ref().unwrap().bytes(k, 1).unwrap()[0];
                    got[e].push((t.time.raw(), b));
                }
            }
            assert_eq!(got, model, "round {round}");
        }
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
