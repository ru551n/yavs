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

/// The state of a paused or ended simulation, for a hang report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct StuckReport {
    pub now: Time,
    pub delta: u32,
    /// Every process, in id order.
    pub procs: Vec<StuckProc>,
    /// The name of every signal, by id.
    pub names: Vec<String>,
    /// The signals that a process in state [`ProcWait::Never`] waits on, in id order.
    pub stuck: Vec<StuckSignal>,
    /// Edge-triggered signals (clocks) that cannot toggle again while a process waits for them.
    pub clocks: Vec<DeadClock>,
}

impl SimControl {
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
        let (waits, drives) = self.wait_graph();
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
                        signal: self.signals[s as usize].name.clone(),
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

    /// Per signal the processes that wait on it, per process the signals it drives (both
    /// ascending, without repeats).
    fn wait_graph(&self) -> (Vec<Vec<u32>>, Vec<Vec<u32>>) {
        let waits = self
            .waiters
            .iter()
            .map(|w| {
                let mut v: Vec<u32> = w
                    .iter()
                    .filter(|w| alive(&self.procs, w))
                    .map(|w| w.proc)
                    .collect();
                v.sort_unstable();
                v.dedup();
                v
            })
            .collect();
        let mut drives = vec![Vec::new(); self.procs.len()];
        for d in &self.drivers {
            drives[d.owner.0 as usize].push(d.signal.0);
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
        let (waits, drives) = self.wait_graph();
        let nsig = self.signals.len();
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
                Entry::Driver(d) => sig_live[self.drivers[d as usize].signal.0 as usize] = true,
                Entry::Timeout(p, _) => {
                    let o = &mut timeout[p as usize];
                    *o = Some(o.map_or(t, |x| x.min(t)));
                }
                Entry::Implicit(i) => sig_live[self.implicits[i as usize].sig.0 as usize] = true,
            }
        }
        for (s, g) in self.signals.iter().enumerate() {
            // A force or deposit, or a driver that has not been applied yet.
            if !g.dirty.is_empty() || !g.eff_dirty.is_empty() {
                sig_live[s] = true;
            }
        }
        for o in &self.ops {
            sig_live[o.signal.0 as usize] = true;
        }
        // Ports and implicit signals carry events from signal to signal.
        let mut links: Vec<(u32, u32)> = Vec::new();
        for e in &self.ports {
            let parts = |v: &[(SignalId, Range)]| v.iter().map(|x| x.0.0).collect::<Vec<_>>();
            let (f, a) = (parts(&e.formal), parts(&e.actual));
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
            for src in std::iter::once(i.src).chain(i.more.iter().map(|m| m.0)) {
                links.push((src.0, i.sig.0));
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
        let driven_by = |s: u32| -> Vec<ProcId> {
            let mut v: Vec<ProcId> = self
                .drivers
                .iter()
                .filter(|d| d.signal.0 == s)
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
        let last_event = |s: usize| {
            let g = &self.signals[s];
            let since = g.last_event(Range::new(0, g.ty.len), now);
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
                let g = &self.signals[s as usize];
                let value = match g.ty.elem {
                    Elem::Logic if g.ty.len <= 16 => {
                        let v = g.cur.extract(0, g.ty.len as usize).to_value();
                        Some(match g.ty.len {
                            1 => format!("'{}'", v.to_string().trim_matches('"')),
                            _ => v.to_string(),
                        })
                    }
                    _ => None,
                };
                StuckSignal {
                    id: SignalId(s),
                    name: g.name.clone(),
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
            .map(|t| t.signal.0)
            .filter(|&s| !sig_live[s as usize])
            .collect();
        clocks.sort_unstable();
        clocks.dedup();
        let clocks = clocks
            .into_iter()
            .filter_map(|s| {
                let g = &self.signals[s as usize];
                if g.ty.len != 1 {
                    return None;
                }
                let waiters: Vec<ProcId> = waits[s as usize].iter().map(|&p| ProcId(p)).collect();
                Some(DeadClock {
                    signal: SignalId(s),
                    name: g.name.clone(),
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
            names: self.signals.iter().map(|g| g.name.clone()).collect(),
            stuck,
            clocks,
        }
    }
}
