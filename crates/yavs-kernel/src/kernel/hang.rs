//! What a stuck simulation looks like from the inside: the loop behind a delta cycle limit error
//! and the report of waiting processes (`SimControl::stuck_report`). Cold code: nothing here runs
//! unless an error or a report is being made, and nothing is recorded during the run.

use super::*;

/// How a process takes part in a delta cycle loop: it drives `signal`, and the next process of
/// the loop waits for it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LoopLink {
    pub process: ProcId,
    pub name: String,
    pub signal: String,
}

/// Why the delta cycle limit was reached, as far as the kernel can tell.
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub enum LoopCause {
    /// Processes that wake each other through signals, in order; the last one wakes the first
    /// (one entry: a process that waits on a signal it drives itself).
    Cycle(Vec<LoopLink>),
    /// Processes that wait for a timeout of zero (`wait for 0 ns;`) and so run again in the
    /// next delta.
    ZeroWait(Vec<(ProcId, String)>),
    /// No loop found among the processes (a loop through forces, deposits or an observer).
    #[default]
    Unknown,
}

/// What a process is doing, between two cycles.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcWait {
    /// A timeout is pending, so it runs again.
    Timed,
    /// It waits on signals that can still change.
    Waiting,
    /// It waits on signals that nothing can change any more and has no timeout.
    Never,
    /// It waits on nothing: it ended, or executed `wait;`.
    Ended,
}

/// One process of a [`StuckReport`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StuckProc {
    pub id: ProcId,
    pub name: String,
    pub state: ProcWait,
    /// The signals it waits on, in id order.
    pub on: Vec<SignalId>,
    /// When the pending timeout expires.
    pub timeout: Option<Time>,
}

/// A signal some process waits on that has no future event.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StuckSignal {
    pub id: SignalId,
    pub name: String,
    /// Its current value as a VHDL literal, for logic signals of at most 16 elements.
    pub value: Option<String>,
    /// The processes with a driver on it.
    pub drivers: Vec<ProcId>,
    /// Signals connected to it through ports that can still carry an event.
    pub fed_by: Vec<SignalId>,
    /// Time of its last event, `None` if it never had one.
    pub last_event: Option<Time>,
}

/// A signal that behaved like a clock and cannot toggle again.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DeadClock {
    pub signal: SignalId,
    pub name: String,
    pub last_event: Time,
    /// The processes still waiting for it.
    pub waiters: Vec<ProcId>,
    /// The processes that drive it.
    pub drivers: Vec<ProcId>,
}

/// The state of a paused or ended simulation, for a hang report. Its `SignalId`s number the
/// parts of the kernel signals (`parts`): a signal with named parts (`SimControl::name_parts`,
/// the fields of a record pack) is one entry per part, any other signal one entry, in order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StuckReport {
    pub now: Time,
    pub delta: u32,
    /// Every process, in id order.
    pub procs: Vec<StuckProc>,
    /// The name of every part, by id.
    pub names: Vec<String>,
    /// The kernel signal and elements of every part, by id.
    pub parts: Vec<(SignalId, Range)>,
    /// The signals that a process in state [`ProcWait::Never`] waits on, in id order.
    pub stuck: Vec<StuckSignal>,
    /// Edge-triggered signals (clocks) that cannot toggle again while a process waits for them.
    pub clocks: Vec<DeadClock>,
}

/// The parts of the kernel signals: per part its signal, elements and name; per signal its first
/// part (one more entry at the end).
struct Parts {
    of: Vec<(u32, Range, String)>,
    start: Vec<u32>,
}

impl Parts {
    /// The parts of signal `s` that elements `r` overlap (all of them for a null range, which
    /// names the whole signal in a few places).
    fn at(&self, s: u32, r: Range) -> impl Iterator<Item = u32> + '_ {
        let (a, b) = (self.start[s as usize], self.start[s as usize + 1]);
        (a..b).filter(move |&k| r.len == 0 || self.of[k as usize].1.overlaps(r))
    }

    fn all(&self, s: u32) -> std::ops::Range<u32> {
        self.start[s as usize]..self.start[s as usize + 1]
    }
}

impl SimControl {
    /// Name parts of signal `s` (the fields of a record pack): hang and loop reports speak of
    /// them instead of the whole signal. The parts must cover the signal in order.
    pub fn name_parts(&mut self, s: SignalId, parts: Vec<(Range, String)>) {
        self.part_names.insert(s.0, parts);
    }

    fn parts(&self) -> Parts {
        let mut p = Parts {
            of: Vec::with_capacity(self.signals.len()),
            start: Vec::with_capacity(self.signals.len() + 1),
        };
        for (s, g) in self.signals.iter().enumerate() {
            p.start.push(p.of.len() as u32);
            match self.part_names.get(&(s as u32)) {
                Some(v) => {
                    p.of.extend(v.iter().map(|(r, n)| (s as u32, *r, n.clone())))
                }
                None => {
                    p.of.push((s as u32, Range::new(0, g.ty.len), g.name.clone()))
                }
            }
        }
        p.start.push(p.of.len() as u32);
        p
    }

    /// The loop behind a delta cycle limit error: starting from the processes that ran in the
    /// last delta, follow driver to waiter until a process comes up twice.
    pub(super) fn loop_cause(&mut self) -> LoopCause {
        let mut ran = self.last_ran.clone();
        ran.sort_unstable();
        ran.dedup();
        // Zero-time timeouts first: those processes run in every delta by themselves.
        let mut zero: Vec<u32> = Vec::new();
        for e in &self.queue.next_delta {
            if let Entry::Timeout(p, g) = *e
                && self.procs[p as usize].generation == g
            {
                zero.push(p);
            }
        }
        zero.sort_unstable();
        zero.dedup();
        let parts = self.parts();
        let (waits, drives) = self.wait_graph(&parts);
        let mut best: Option<Vec<(u32, u32)>> = None;
        for &start in &ran {
            // Breadth first, remembering how each process was reached.
            let mut from: std::collections::HashMap<u32, (u32, u32)> = Default::default();
            let mut queue = VecDeque::from([start]);
            'bfs: while let Some(p) = queue.pop_front() {
                for &s in &drives[p as usize] {
                    for &q in &waits[s as usize] {
                        if q == start {
                            // Close the loop: walk back from p.
                            let mut path = vec![(p, s)];
                            let mut at = p;
                            while at != start {
                                let (prev, sig) = from[&at];
                                path.push((prev, sig));
                                at = prev;
                            }
                            path.reverse();
                            if best.as_ref().is_none_or(|b| path.len() < b.len()) {
                                best = Some(path);
                            }
                            break 'bfs;
                        }
                        if let std::collections::hash_map::Entry::Vacant(v) = from.entry(q) {
                            v.insert((p, s));
                            queue.push_back(q);
                        }
                    }
                }
            }
        }
        if let Some(path) = best {
            return LoopCause::Cycle(
                path.into_iter()
                    .map(|(p, s)| LoopLink {
                        process: ProcId(p),
                        name: self.names[p as usize].clone(),
                        signal: parts.of[s as usize].2.clone(),
                    })
                    .collect(),
            );
        }
        if !zero.is_empty() {
            return LoopCause::ZeroWait(
                zero.into_iter()
                    .map(|p| (ProcId(p), self.names[p as usize].clone()))
                    .collect(),
            );
        }
        LoopCause::Unknown
    }

    /// Per part the processes that wait on it, per process the parts it drives (both
    /// ascending, without repeats).
    fn wait_graph(&self, parts: &Parts) -> (Vec<Vec<u32>>, Vec<Vec<u32>>) {
        let mut waits: Vec<Vec<u32>> = vec![Vec::new(); parts.of.len()];
        for (s, ws) in self.waiters.iter().enumerate() {
            for w in ws.iter().filter(|w| alive(&self.procs, w)) {
                for k in parts.at(s as u32, w.range) {
                    waits[k as usize].push(w.proc);
                }
            }
        }
        for v in &mut waits {
            v.sort_unstable();
            v.dedup();
        }
        let mut drives = vec![Vec::new(); self.procs.len()];
        for d in &self.drivers {
            drives[d.owner.0 as usize].extend(parts.at(d.signal.0, d.range));
        }
        for v in &mut drives {
            v.sort_unstable();
            v.dedup();
        }
        (waits, drives)
    }

    /// The report of a simulation that is stuck, paused or has ended: who waits for what and
    /// what can still wake them. Reads the state between cycles; changes nothing but drops
    /// stale queue entries.
    pub fn stuck_report(&mut self) -> StuckReport {
        let parts = self.parts();
        let (waits, drives) = self.wait_graph(&parts);
        let nsig = parts.of.len();
        let nproc = self.procs.len();
        // Pending events: signals with a transaction due, processes with a timeout.
        let mut sig_live = vec![false; nsig];
        let mut timeout: Vec<Option<Time>> = vec![None; nproc];
        let now = self.now;
        let mut entries: Vec<(Time, Entry)> =
            self.queue.next_delta.iter().map(|&e| (now, e)).collect();
        entries.extend(self.queue.later.v.iter().map(|l| (l.time, l.entry())));
        for (t, e) in entries {
            if !self.live(t, e) {
                continue;
            }
            match e {
                Entry::Driver(d) => {
                    let d = &self.drivers[d as usize];
                    parts
                        .at(d.signal.0, d.range)
                        .for_each(|k| sig_live[k as usize] = true);
                }
                Entry::Timeout(p, _) => {
                    let o = &mut timeout[p as usize];
                    *o = Some(o.map_or(t, |x| x.min(t)));
                }
                Entry::Implicit(i) => {
                    let s = self.implicits[i as usize].sig.0;
                    parts.all(s).for_each(|k| sig_live[k as usize] = true);
                }
            }
        }
        for (s, g) in self.signals.iter().enumerate() {
            // A force or deposit, or a driver that has not been applied yet.
            if !g.dirty.is_empty() || !g.eff_dirty.is_empty() {
                parts
                    .all(s as u32)
                    .for_each(|k| sig_live[k as usize] = true);
            }
        }
        for o in &self.ops {
            parts
                .all(o.signal.0)
                .for_each(|k| sig_live[k as usize] = true);
        }
        // Ports and implicit signals carry events from signal to signal.
        let mut links: Vec<(u32, u32)> = Vec::new();
        for e in &self.ports {
            let of = |v: &[(SignalId, Range)]| {
                (v.iter())
                    .flat_map(|x| parts.at(x.0.0, x.1))
                    .collect::<Vec<_>>()
            };
            let (f, a) = (of(&e.formal), of(&e.actual));
            for &x in &f {
                for &y in &a {
                    match e.mode {
                        PortMode::In => links.push((y, x)),
                        PortMode::Out | PortMode::Buffer => links.push((x, y)),
                        PortMode::Inout => {
                            links.push((x, y));
                            links.push((y, x));
                        }
                    }
                }
            }
        }
        for i in &self.implicits {
            let srcs = std::iter::once((i.src, i.range)).chain(i.more.iter().copied());
            for (src, r) in srcs {
                for x in parts.at(src.0, r) {
                    parts.all(i.sig.0).for_each(|y| links.push((x, y)));
                }
            }
        }
        // Everything that can still run, to a fixed point: a process with a timeout runs; a
        // signal with an event wakes its waiters; a process that runs may drive its signals.
        let mut proc_live: Vec<bool> = (0..nproc)
            .map(|p| timeout[p].is_some() || self.procs[p].ready)
            .collect();
        let mut changed = true;
        while changed {
            changed = false;
            for &(a, b) in &links {
                if sig_live[a as usize] && !sig_live[b as usize] {
                    sig_live[b as usize] = true;
                    changed = true;
                }
            }
            for (s, w) in waits.iter().enumerate() {
                if sig_live[s] {
                    for &p in w {
                        if !proc_live[p as usize] {
                            proc_live[p as usize] = true;
                            changed = true;
                        }
                    }
                }
            }
            for p in 0..nproc {
                if proc_live[p] {
                    for &s in &drives[p] {
                        if !sig_live[s as usize] {
                            sig_live[s as usize] = true;
                            changed = true;
                        }
                    }
                }
            }
        }
        let mut on: Vec<Vec<SignalId>> = vec![Vec::new(); nproc];
        for (s, w) in waits.iter().enumerate() {
            for &p in w {
                on[p as usize].push(SignalId(s as u32));
            }
        }
        let procs: Vec<StuckProc> = (0..nproc)
            .map(|p| StuckProc {
                id: ProcId(p as u32),
                name: self.names[p].clone(),
                state: match () {
                    _ if timeout[p].is_some() => ProcWait::Timed,
                    _ if on[p].is_empty() => ProcWait::Ended,
                    _ if proc_live[p] => ProcWait::Waiting,
                    _ => ProcWait::Never,
                },
                on: std::mem::take(&mut on[p]),
                timeout: timeout[p],
            })
            .collect();
        let driven_by = |k: u32| -> Vec<ProcId> {
            let (s, r, _) = &parts.of[k as usize];
            let mut v: Vec<ProcId> = self
                .drivers
                .iter()
                .filter(|d| d.signal.0 == *s && d.range.overlaps(*r))
                .map(|d| d.owner)
                .collect();
            v.sort_unstable();
            v.dedup();
            v
        };
        let fed_by = |s: u32| -> Vec<SignalId> {
            let mut v: Vec<SignalId> = links
                .iter()
                .filter(|l| l.1 == s && l.0 != s)
                .map(|l| SignalId(l.0))
                .collect();
            v.sort_unstable();
            v.dedup();
            v
        };
        let last_event = |k: usize| {
            let (s, r, _) = &parts.of[k];
            let since = self.signals[*s as usize].last_event(*r, now);
            (since != Time::MAX).then(|| now.since(since))
        };
        // The signals the stuck processes wait on and nothing can change.
        let mut stuck: Vec<u32> = procs
            .iter()
            .filter(|p| p.state == ProcWait::Never)
            .flat_map(|p| p.on.iter().map(|s| s.0))
            .filter(|&s| !sig_live[s as usize])
            .collect();
        stuck.sort_unstable();
        stuck.dedup();
        let stuck = stuck
            .into_iter()
            .map(|s| {
                let (sig, r, name) = &parts.of[s as usize];
                let g = &self.signals[*sig as usize];
                let value = match g.ty.elem {
                    Elem::Logic if r.len <= 16 => {
                        let v = g.cur.extract(r.offset as usize, r.len as usize).to_value();
                        Some(match r.len {
                            1 => format!("'{}'", v.to_string().trim_matches('"')),
                            _ => v.to_string(),
                        })
                    }
                    _ => None,
                };
                StuckSignal {
                    id: SignalId(s),
                    name: name.clone(),
                    value,
                    drivers: driven_by(s),
                    fed_by: fed_by(s),
                    last_event: last_event(s as usize),
                }
            })
            .collect();
        // Clocks: the signals an edge trigger watches, that had an event and cannot have one
        // again, while some process still waits for them.
        let mut clocks: Vec<u32> = self
            .triggers
            .iter()
            .flat_map(|t| parts.at(t.signal.0, Range::new(t.elem, 1)))
            .filter(|&s| !sig_live[s as usize])
            .collect();
        clocks.sort_unstable();
        clocks.dedup();
        let clocks = clocks
            .into_iter()
            .filter_map(|s| {
                let (_, r, name) = &parts.of[s as usize];
                if r.len != 1 {
                    return None;
                }
                let waiters: Vec<ProcId> = waits[s as usize].iter().map(|&p| ProcId(p)).collect();
                Some(DeadClock {
                    signal: SignalId(s),
                    name: name.clone(),
                    last_event: last_event(s as usize).filter(|t| *t > Time::ZERO)?,
                    waiters: (!waiters.is_empty()).then_some(waiters)?,
                    drivers: driven_by(s),
                })
            })
            .collect();
        StuckReport {
            now,
            delta: self.delta,
            procs,
            names: parts.of.iter().map(|p| p.2.clone()).collect(),
            parts: parts.of.into_iter().map(|p| (SignalId(p.0), p.1)).collect(),
            stuck,
            clocks,
        }
    }
}
