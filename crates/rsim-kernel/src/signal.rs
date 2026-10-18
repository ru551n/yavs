//! Signal storage, drivers and projected output waveforms.
//!
//! A signal is one contiguous array of `len` elements of `elem_size` bytes.
//! Values are opaque bytes; the element encoding (packed std_logic, integer,
//! record) belongs to the caller. Everything that addresses part of a signal
//! (drivers, events, forces, waiters, value-change events) uses one
//! [`Range`] of element indices (design rule 2).

use std::sync::Arc;

use crate::error::ErrorKind;
use crate::time::Time;

/// Stable id of a signal: its index in creation (elaboration) order.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SignalId(pub u32);

/// Stable id of a driver.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DriverId(pub u32);

/// A contiguous run of elements: `offset .. offset + len`.
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

    pub const fn overlaps(self, o: Range) -> bool {
        self.offset < o.end() && o.offset < self.end()
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

/// The small type descriptor a signal carries until rsim-logic and the IR
/// provide real types: element size in bytes and element count.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SignalType {
    pub elem_size: u32,
    pub len: u32,
}

/// A resolution function over one segment of a resolved signal.
///
/// `sources[i]` holds the values of driver `i` (in driver creation order) for
/// the elements being resolved; `out` receives the resolved values. Both have
/// the same length, a whole number of elements. For an element-resolved type
/// such as `std_logic_vector` the function resolves element by element; the
/// kernel batches contiguous elements with the same sources into one call.
/// An `Err` (a VHDL resolution function that failed) stops the simulation.
pub type Resolver = Arc<dyn Fn(&[&[u8]], &mut [u8]) -> Result<(), String> + Send + Sync>;

/// Force mode (LRM 10.5.2.1): `force in` overrides the effective value,
/// `force out` the driving value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ForceMode {
    Effective,
    Driving,
}

#[derive(Default)]
pub(crate) struct Forced {
    pub mask: Vec<bool>,
    pub value: Vec<u8>,
}

pub(crate) struct DriverSlot {
    pub range: Range,
    /// Current value of the driver.
    pub value: Vec<u8>,
}

/// Elements with the same set of drivers.
pub(crate) struct Segment {
    pub range: Range,
    /// Indices into `Signal::drivers`.
    pub drivers: Vec<u32>,
}

pub(crate) struct Waiter {
    pub proc: u32,
    pub range: Range,
    pub generation: u64,
}

pub(crate) struct Signal {
    pub name: String,
    pub ty: SignalType,
    /// Effective (current) value.
    pub cur: Vec<u8>,
    /// Driving value computed from the drivers or a deposit, before forces.
    pub driving: Vec<u8>,
    pub last_value: Vec<u8>,
    /// Per element: cycle number of the last event / activity.
    pub event_cycle: Vec<u64>,
    pub active_cycle: Vec<u64>,
    /// Per element: time of the last event / activity, `None` if never.
    pub last_event: Vec<Option<Time>>,
    pub last_active: Vec<Option<Time>>,
    pub drivers: Vec<DriverSlot>,
    pub segments: Vec<Segment>,
    pub resolver: Option<Resolver>,
    pub force_eff: Option<Box<Forced>>,
    pub force_drv: Option<Box<Forced>>,
    pub waiters: Vec<Waiter>,
    /// Waiter count right after the last compaction (see `add_waiter`).
    pub waiters_live: usize,
    /// Implicit `'transaction` signal of this signal.
    pub transaction: Option<SignalId>,
    /// Elements changed in the current cycle.
    pub changed: Vec<Range>,
    pub dirty: Vec<Range>,
}

impl Signal {
    pub fn new(name: String, ty: SignalType, init: &[u8], resolver: Option<Resolver>) -> Signal {
        let n = ty.len as usize;
        Signal {
            name,
            ty,
            cur: init.to_vec(),
            driving: init.to_vec(),
            last_value: init.to_vec(),
            event_cycle: vec![0; n],
            active_cycle: vec![0; n],
            last_event: vec![None; n],
            last_active: vec![None; n],
            drivers: Vec::new(),
            segments: Vec::new(),
            resolver,
            force_eff: None,
            force_drv: None,
            waiters: Vec::new(),
            waiters_live: 0,
            transaction: None,
            changed: Vec::new(),
            dirty: Vec::new(),
        }
    }

    pub fn full(&self) -> Range {
        Range::new(0, self.ty.len)
    }

    pub fn bytes(&self, r: Range) -> std::ops::Range<usize> {
        let es = self.ty.elem_size as usize;
        r.offset as usize * es..r.end() as usize * es
    }

    pub fn resolve(&self, r: Option<Range>) -> Range {
        r.unwrap_or(self.full())
    }

    /// Partition the signal into segments with identical driver sets.
    pub fn build_segments(&mut self) {
        let mut cuts = vec![0, self.ty.len];
        for d in &self.drivers {
            cuts.push(d.range.offset);
            cuts.push(d.range.end());
        }
        cuts.sort_unstable();
        cuts.dedup();
        self.segments = cuts
            .windows(2)
            .map(|w| {
                let range = Range::new(w[0], w[1] - w[0]);
                let drivers = (0..self.drivers.len() as u32)
                    .filter(|&i| self.drivers[i as usize].range.contains(range))
                    .collect();
                Segment { range, drivers }
            })
            .collect();
    }

    /// Recompute the driving value of `r` from the drivers.
    pub fn compute_driving(&mut self, r: Range) -> Result<(), ErrorKind> {
        let es = self.ty.elem_size as usize;
        for seg in &self.segments {
            let Some(part) = seg.range.intersect(r) else {
                continue;
            };
            let out_bytes = part.offset as usize * es..part.end() as usize * es;
            let src = |i: u32| {
                let d = &self.drivers[i as usize];
                let off = (part.offset - d.range.offset) as usize * es;
                &d.value[off..off + part.len as usize * es]
            };
            match (&self.resolver, seg.drivers.as_slice()) {
                (_, []) => {}
                (Some(res), ds) => {
                    let sources: Vec<&[u8]> = ds.iter().map(|&i| src(i)).collect();
                    res(&sources, &mut self.driving[out_bytes]).map_err(|message| {
                        ErrorKind::Resolution {
                            signal: self.name.clone(),
                            message,
                        }
                    })?;
                }
                (None, [i]) => {
                    let v = src(*i);
                    self.driving[out_bytes].copy_from_slice(v);
                }
                (None, _) => {
                    return Err(ErrorKind::Internal(format!(
                        "unresolved signal {} has several drivers",
                        self.name
                    )));
                }
            }
        }
        Ok(())
    }

    /// The effective value of element `e` after forces.
    fn effective_elem(&self, e: usize) -> &[u8] {
        let es = self.ty.elem_size as usize;
        let b = e * es..(e + 1) * es;
        for f in [&self.force_eff, &self.force_drv].into_iter().flatten() {
            if f.mask[e] {
                return &f.value[b];
            }
        }
        &self.driving[b]
    }

    /// The driving value of element `e` after `force out`.
    pub fn driving_elem(&self, e: usize) -> &[u8] {
        let es = self.ty.elem_size as usize;
        let b = e * es..(e + 1) * es;
        match &self.force_drv {
            Some(f) if f.mask[e] => &f.value[b],
            _ => &self.driving[b],
        }
    }

    /// Update the effective value of `r`, recording events.
    pub fn update_effective(&mut self, r: Range, now: Time, cycle: u64) {
        let es = self.ty.elem_size as usize;
        for e in r.elems() {
            let b = e * es..(e + 1) * es;
            if self.effective_elem(e) == &self.cur[b.clone()] {
                continue;
            }
            let new = self.effective_elem(e).to_vec();
            self.last_value[b.clone()].copy_from_slice(&self.cur[b.clone()]);
            self.cur[b].copy_from_slice(&new);
            self.event_cycle[e] = cycle;
            self.last_event[e] = Some(now);
            match self.changed.last_mut() {
                Some(run) if run.end() as usize == e => run.len += 1,
                _ => self.changed.push(Range::new(e as u32, 1)),
            }
        }
    }

    pub fn mark_active(&mut self, r: Range, now: Time, cycle: u64) {
        for e in r.elems() {
            self.active_cycle[e] = cycle;
            self.last_active[e] = Some(now);
        }
    }

    /// Register a waiter, compacting stale entries so lists of signals that
    /// rarely change do not grow without bound.
    pub fn add_waiter(&mut self, w: Waiter, live: impl Fn(&Waiter) -> bool) {
        if self.waiters.len() >= 2 * self.waiters_live + 16 {
            self.waiters.retain(&live);
            self.waiters_live = self.waiters.len();
        }
        self.waiters.push(w);
    }
}

/// One transaction of a projected output waveform.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Txn {
    pub time: Time,
    /// Elements of the signal (not of the driver).
    pub range: Range,
    // ponytail: one heap allocation per transaction; pool per driver if profiles show it.
    pub value: Vec<u8>,
}

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

/// The projected output waveform of one driver, sorted by time.
#[derive(Default)]
pub(crate) struct Waveform {
    pub pending: Vec<Txn>,
}

impl Waveform {
    /// Apply LRM 10.5.2.2 to target elements `r` for a new waveform whose
    /// first transaction is at `t1`, then insert the new transactions.
    /// `reject_from` is `t1 - reject` for inertial delay, `None` for transport.
    pub fn schedule(&mut self, es: usize, r: Range, reject_from: Option<Time>, new: Vec<Txn>) {
        let t1 = new[0].time;
        let first = &new[0].value;
        // Per element, decide which overlapping old transactions to delete.
        // ponytail: O(elements * pending); fine while waveforms stay short.
        let mut deleted: Vec<Option<Vec<bool>>> = vec![None; self.pending.len()];
        for e in r.elems() {
            let new_val = &first[(e - r.offset as usize) * es..][..es];
            let mut chain = true;
            for (i, t) in self.pending.iter().enumerate().rev() {
                if !t.range.elems().contains(&e) {
                    continue;
                }
                let k = e - t.range.offset as usize;
                let delete = if t.time >= t1 {
                    true
                } else if reject_from.is_some_and(|from| t.time >= from) {
                    chain &= &t.value[k * es..][..es] == new_val;
                    !chain
                } else {
                    false
                };
                if delete {
                    deleted[i].get_or_insert_with(|| vec![false; t.range.len as usize])[k] = true;
                }
            }
        }
        if deleted.iter().any(Option::is_some) {
            let old = std::mem::take(&mut self.pending);
            for (t, del) in old.into_iter().zip(deleted) {
                match del {
                    None => self.pending.push(t),
                    Some(mask) => split_kept(t, &mask, es, &mut self.pending),
                }
            }
        }
        for t in new {
            let at = self.pending.partition_point(|p| p.time <= t.time);
            self.pending.insert(at, t);
        }
    }

    /// Remove and return the transactions due at or before `now`.
    pub fn take_due(&mut self, now: Time) -> std::vec::Drain<'_, Txn> {
        let n = self.pending.partition_point(|t| t.time <= now);
        self.pending.drain(..n)
    }
}

/// Push the runs of `t` not marked in `mask`.
fn split_kept(t: Txn, mask: &[bool], es: usize, out: &mut Vec<Txn>) {
    let mut start = None;
    for k in 0..=mask.len() {
        let keep = k < mask.len() && !mask[k];
        match (keep, start) {
            (true, None) => start = Some(k),
            (false, Some(s)) => {
                out.push(Txn {
                    time: t.time,
                    range: Range::new(t.range.offset + s as u32, (k - s) as u32),
                    value: t.value[s * es..k * es].to_vec(),
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

    fn txn(ns: u64, off: u32, v: &[u8]) -> Txn {
        Txn {
            time: Time::ns(ns),
            range: Range::new(off, v.len() as u32),
            value: v.to_vec(),
        }
    }

    #[test]
    fn transport_deletes_at_or_after() {
        let mut w = Waveform::default();
        w.schedule(1, Range::new(0, 1), None, vec![txn(10, 0, &[1])]);
        w.schedule(1, Range::new(0, 1), None, vec![txn(20, 0, &[2])]);
        w.schedule(1, Range::new(0, 1), None, vec![txn(15, 0, &[3])]);
        assert_eq!(w.pending, vec![txn(10, 0, &[1]), txn(15, 0, &[3])]);
    }

    #[test]
    fn inertial_keeps_same_value_chain_per_element() {
        // Old "00" at 10 ns; new "01" at 20 ns with 20 ns rejection: element 0
        // keeps its old transaction (same value), element 1 loses it.
        let mut w = Waveform::default();
        w.pending.push(txn(10, 0, &[0, 0]));
        w.schedule(
            1,
            Range::new(0, 2),
            Some(Time::ZERO),
            vec![txn(20, 0, &[0, 1])],
        );
        assert_eq!(w.pending, vec![txn(10, 0, &[0]), txn(20, 0, &[0, 1])]);
    }

    #[test]
    fn inertial_outside_window_kept() {
        let mut w = Waveform::default();
        w.pending.push(txn(1, 0, &[7]));
        w.schedule(
            1,
            Range::new(0, 1),
            Some(Time::ns(5)),
            vec![txn(10, 0, &[1])],
        );
        assert_eq!(w.pending.len(), 2);
    }

    #[test]
    fn sub_element_assignment_splits_only_its_range() {
        let mut w = Waveform::default();
        w.pending.push(txn(10, 0, &[1, 2, 3, 4]));
        w.schedule(1, Range::new(1, 2), None, vec![txn(5, 1, &[9, 9])]);
        assert_eq!(
            w.pending,
            vec![txn(5, 1, &[9, 9]), txn(10, 0, &[1]), txn(10, 3, &[4])]
        );
    }
}
