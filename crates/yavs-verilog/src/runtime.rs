//! The Verilog domain: nets and regs, the Active, Inactive and NBA regions of IEEE 1364-2005 5.4,
//! and future events that keep their region. The whole domain is one serial kernel process
//! (`bridge.rs`); processes are Rust closures until lowering produces them.

use std::collections::{BTreeMap, VecDeque};

use yavs_kernel::{Ctx, DriverId, ErrorKind, LogicVec, SigRef, SimError, Time};
use yavs_logic::StdULogic;

/// A net or reg of the domain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetId(pub u32);

/// The event an `@(...)` item waits for. `Pos` and `Neg` look at element 0 (the LSB, 9.7.2).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edge {
    Any,
    Pos,
    Neg,
}

/// The posedge or negedge of one element changing from `old` to `new` (IEEE 1364-2005 Table
/// 9-9): 0 to 1, x or z is a posedge, x or z to 1 too; negedge mirrors it; x to z is neither.
/// Not `Ctx::edge`, which has the VHDL rules.
pub fn edge(old: StdULogic, new: StdULogic) -> Option<Edge> {
    let level = |c: StdULogic| match c.to_x01() {
        StdULogic::Zero => 0,
        StdULogic::One => 1,
        _ => 2,
    };
    match (level(old), level(new)) {
        (0, 1) | (0, 2) | (2, 1) => Some(Edge::Pos),
        (1, 0) | (1, 2) | (2, 0) => Some(Edge::Neg),
        _ => None,
    }
}

impl Edge {
    fn holds(self, old: StdULogic, new: StdULogic) -> bool {
        self == Edge::Any || edge(old, new) == Some(self)
    }
}

/// What a process waits for when its activation ends.
pub enum Wait {
    /// `@(...)`: an event of any item.
    On(Vec<(NetId, Edge)>),
    /// `#d`: the Active region at `now + d` (`#0` is `Zero`).
    Delay(Time),
    /// `#0`: the Inactive region of this time step.
    Zero,
    /// The end of an `initial` block.
    Done,
}

/// The body of a process: runs from where it suspended to its next wait.
pub type Body = Box<dyn FnMut(&mut Exec<'_, '_>) -> Result<Wait, SimError> + Send>;

/// Activations of one process at one time before the domain reports a zero-delay loop.
pub const EVENT_LIMIT: u32 = 10_000;

/// The Verilog part of a design: one scheduler for every instance (plan: one domain, so modules
/// never see each other's updates late).
pub struct Domain {
    pub(crate) core: Core,
    pub(crate) bodies: Vec<Body>,
    pub(crate) inputs: Vec<(SigRef, NetId)>,
    pub(crate) outputs: Vec<(NetId, DriverId)>,
    pub(crate) started: bool,
}

pub(crate) struct Net {
    pub(crate) value: LogicVec,
    waiters: Vec<Waiter>,
    /// The output port this net drives (`Domain::outputs` index).
    pub(crate) out: Option<u32>,
}

struct Waiter {
    proc: u32,
    edge: Edge,
    /// The process's `epoch` at registration: stale once it resumed.
    epoch: u32,
}

struct Proc {
    name: String,
    epoch: u32,
    /// Activations at time `at`, for the event limit.
    runs: u32,
    at: Time,
}

/// A pending write: element `at` on of `net` gets `value`.
pub(crate) struct Update {
    net: NetId,
    at: u32,
    value: LogicVec,
}

enum Item {
    Resume(u32),
    Nba(Update),
}

pub(crate) struct Core {
    pub(crate) nets: Vec<Net>,
    procs: Vec<Proc>,
    pub(crate) active: VecDeque<u32>,
    inactive: Vec<u32>,
    nba: Vec<Update>,
    /// `#d` resumptions and `<= #d` updates, in issue order per time; each enters its own region.
    future: BTreeMap<Time, Vec<Item>>,
    /// Boundary changes in order, one batch per kernel delta (`bridge.rs`).
    pub(crate) publish: VecDeque<Vec<(u32, LogicVec)>>,
    /// The batch boundary changes go to now, counted from the one this activation assigns.
    pub(crate) cursor: usize,
    limit: u32,
}

impl Default for Domain {
    fn default() -> Domain {
        Domain {
            core: Core {
                nets: Vec::new(),
                procs: Vec::new(),
                active: VecDeque::new(),
                inactive: Vec::new(),
                nba: Vec::new(),
                future: BTreeMap::new(),
                publish: VecDeque::new(),
                cursor: 0,
                limit: EVENT_LIMIT,
            },
            bodies: Vec::new(),
            inputs: Vec::new(),
            outputs: Vec::new(),
            started: false,
        }
    }
}

impl Domain {
    pub fn new() -> Domain {
        Domain::default()
    }

    /// A net or reg with its initial value (`x` for a reg, `z` for an undriven wire).
    pub fn net(&mut self, init: LogicVec) -> NetId {
        self.core.nets.push(Net {
            value: init,
            waiters: Vec::new(),
            out: None,
        });
        NetId(self.core.nets.len() as u32 - 1)
    }

    /// A process, started in the Active region at initialisation, in declaration order.
    pub fn process(
        &mut self,
        name: &str,
        body: impl FnMut(&mut Exec<'_, '_>) -> Result<Wait, SimError> + Send + 'static,
    ) {
        self.core.procs.push(Proc {
            name: name.to_string(),
            epoch: 0,
            runs: 0,
            at: Time::ZERO,
        });
        self.bodies.push(Box::new(body));
    }

    /// `always @(sens) begin f end`.
    pub fn always(
        &mut self,
        name: &str,
        sens: Vec<(NetId, Edge)>,
        mut f: impl FnMut(&mut Exec<'_, '_>) -> Result<(), SimError> + Send + 'static,
    ) {
        let mut started = false;
        self.process(name, move |x| {
            if std::mem::replace(&mut started, true) {
                f(x)?;
            }
            Ok(Wait::On(sens.clone()))
        });
    }

    /// `assign`: evaluated at initialisation and on every change of `deps`, without a delta.
    pub fn assign(
        &mut self,
        name: &str,
        deps: &[NetId],
        mut f: impl FnMut(&mut Exec<'_, '_>) -> Result<(), SimError> + Send + 'static,
    ) {
        let sens: Vec<_> = deps.iter().map(|&n| (n, Edge::Any)).collect();
        self.process(name, move |x| {
            f(x)?;
            Ok(Wait::On(sens.clone()))
        });
    }

    /// Activations of one process at one time before the run stops with an error (`EVENT_LIMIT`).
    pub fn set_event_limit(&mut self, limit: u32) {
        self.core.limit = limit;
    }

    /// The region loop of 5.4 until every region of this time step is empty: `Ok(false)` when a
    /// stop was requested first (the queues keep their work).
    pub(crate) fn settle(&mut self, ctx: &mut Ctx<'_>) -> Result<bool, SimError> {
        let core = &mut self.core;
        let mut n = 0u32;
        loop {
            if let Some(p) = core.active.pop_front() {
                n = n.wrapping_add(1);
                if n.is_multiple_of(1024) && ctx.stop_requested() {
                    core.active.push_front(p);
                    return Ok(false);
                }
                core.count(p, ctx)?;
                let w = (self.bodies[p as usize])(&mut Exec { core, ctx })?;
                core.suspend(p, w, ctx)?;
            } else if !core.inactive.is_empty() {
                core.active.extend(core.inactive.drain(..));
            } else if !core.nba.is_empty() {
                // D4: what the NBA region and its fanout change reaches the kernel a delta later.
                core.cursor += 1;
                for u in std::mem::take(&mut core.nba) {
                    core.write(u.net, u.at, &u.value);
                }
            } else {
                return Ok(true);
            }
        }
    }
}

impl Core {
    /// Future work due at `now` enters its region.
    pub(crate) fn mature(&mut self, now: Time) {
        while let Some(e) = self.future.first_entry()
            && *e.key() <= now
        {
            for item in e.remove() {
                match item {
                    Item::Resume(p) => self.active.push_back(p),
                    Item::Nba(u) => self.nba.push(u),
                }
            }
        }
    }

    pub(crate) fn next_time(&self) -> Option<Time> {
        self.future.keys().next().copied()
    }

    fn count(&mut self, p: u32, ctx: &Ctx<'_>) -> Result<(), SimError> {
        let pr = &mut self.procs[p as usize];
        if pr.at != ctx.now() {
            (pr.at, pr.runs) = (ctx.now(), 0);
        }
        pr.runs += 1;
        if pr.runs > self.limit {
            return Err(ctx.error(ErrorKind::Runtime(format!(
                "Verilog process `{}` ran {} times at time {} without time advancing: a zero-delay loop (IEEE 1364-2005 5.4) never lets the time step end",
                pr.name,
                self.limit,
                ctx.now()
            ))));
        }
        Ok(())
    }

    fn suspend(&mut self, p: u32, w: Wait, ctx: &Ctx<'_>) -> Result<(), SimError> {
        match w {
            Wait::On(items) => {
                let epoch = self.procs[p as usize].epoch;
                for (n, edge) in items {
                    let procs = &self.procs;
                    let ws = &mut self.nets[n.0 as usize].waiters;
                    // ponytail: stale waiters (a process woken by another item) are pruned when
                    // the list doubles; per-process wait lists if a profile shows the scan.
                    if ws.len() >= 8 && ws.len().is_power_of_two() {
                        ws.retain(|w| procs[w.proc as usize].epoch == w.epoch);
                    }
                    ws.push(Waiter {
                        proc: p,
                        edge,
                        epoch,
                    });
                }
            }
            Wait::Delay(d) if d > Time::ZERO => {
                let at = ctx.after(d)?;
                self.future.entry(at).or_default().push(Item::Resume(p));
            }
            Wait::Delay(_) | Wait::Zero => self.inactive.push(p),
            Wait::Done => {}
        }
        Ok(())
    }

    /// Elements `at..` of `net` take `v` at once; waiters whose item holds move to Active.
    pub(crate) fn write(&mut self, net: NetId, at: u32, v: &LogicVec) {
        let net = &mut self.nets[net.0 as usize];
        let old = net.value.get(0);
        let mut changed = false;
        // ponytail: element by element; plane-word copies when lowering makes writes hot.
        for i in 0..v.len() {
            let (e, c) = (at as usize + i, v.get(i));
            if net.value.get(e) != c {
                net.value.set(e, c);
                changed = true;
            }
        }
        if !changed {
            return;
        }
        let new = net.value.get(0);
        let (procs, active) = (&mut self.procs, &mut self.active);
        net.waiters.retain(|w| {
            let pr = &mut procs[w.proc as usize];
            if pr.epoch != w.epoch {
                return false;
            }
            if w.edge.holds(old, new) {
                pr.epoch += 1;
                active.push_back(w.proc);
                return false;
            }
            true
        });
        if let Some(o) = net.out {
            let v = net.value.clone();
            self.record(o, v);
        }
    }

    fn check(&self, net: NetId, at: u32, v: &LogicVec, ctx: &Ctx<'_>) -> Result<(), SimError> {
        match self.nets.get(net.0 as usize) {
            Some(n) if at as usize + v.len() <= n.value.len() => Ok(()),
            _ => Err(ctx.error(ErrorKind::Internal(format!(
                "Verilog write of {} elements at {at} to net {} out of its range",
                v.len(),
                net.0
            )))),
        }
    }
}

/// What a process body reads and writes while it runs.
pub struct Exec<'a, 'c> {
    core: &'a mut Core,
    ctx: &'a mut Ctx<'c>,
}

impl Exec<'_, '_> {
    pub fn get(&self, n: NetId) -> &LogicVec {
        &self.core.nets[n.0 as usize].value
    }

    pub fn now(&self) -> Time {
        self.ctx.now()
    }

    /// Blocking assignment `n = v`: the value now, waiters to Active now.
    pub fn set(&mut self, n: NetId, v: &LogicVec) -> Result<(), SimError> {
        self.core.check(n, 0, v, self.ctx)?;
        self.core.write(n, 0, v);
        Ok(())
    }

    /// Non-blocking assignment `n[at +: v.len()] <= #delay v`: target and value are taken now;
    /// the update enters the NBA region of `now + delay`.
    pub fn nba(&mut self, n: NetId, at: u32, v: LogicVec, delay: Time) -> Result<(), SimError> {
        self.core.check(n, at, &v, self.ctx)?;
        let u = Update {
            net: n,
            at,
            value: v,
        };
        if delay == Time::ZERO {
            self.core.nba.push(u);
        } else {
            let t = self.ctx.after(delay)?;
            self.core.future.entry(t).or_default().push(Item::Nba(u));
        }
        Ok(())
    }

    /// `$display`: one line of output, in the kernel's output order.
    pub fn display(&mut self, s: &str) {
        self.ctx.output(format!("{s}\n").as_bytes());
    }

    /// `$finish`.
    pub fn finish(&mut self) {
        self.ctx.finish(0);
    }
}
