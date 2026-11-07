//! The Verilog domain: nets and regs, the Active, Inactive and NBA regions of IEEE 1364-2005 5.4,
//! and future events that keep their region. The whole domain is one serial kernel process
//! (`bridge.rs`); processes are Rust closures until lowering produces them.
//!
//! Net model. A net holds one value. A net with one driver, or drivers on disjoint bits, is
//! written to directly. A net that needs resolution (drivers on the same bit, a driver that is
//! not strong, a pull) is a *resolved net*: each of its drivers is a private net of the same width
//! (`Domain::driver`) that the driver's process writes as it would any net, unwritten bits staying
//! `z`; the write that changes a driver net resolves the target at once, in the same region, with
//! the `std_logic` resolution of the kernel on the strength-mapped driver values (plan 3.3, D13).
//! The resolved value is a `std_ulogic` (`L`, `H` and `W` stay), read by Verilog as `TO_X01Z`.
//! A net that VHDL also drives (an `inout` port, step D) is meant to join the same list as one
//! more driver, whose value is the VHDL signal minus this domain's own driver; until then such a
//! net is refused at elaboration (risk R9 of the plan).

use std::collections::{BTreeMap, VecDeque};

use yavs_kernel::{Ctx, DriverId, ErrorKind, LogicVec, SigRef, SimError, Time};
use yavs_logic::StdULogic;

/// A net or reg of the domain.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct NetId(pub u32);

/// How strongly a driver drives one value (plan 3.3): supply and strong collapse to `Strong`
/// (`0`, `1`, `X`), pull, weak, large, medium and small to `Weak` (`L`, `H`, `W`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Drive {
    Strong,
    Weak,
    HighZ,
}

/// The `std_ulogic` a driver puts on the net for the 4-state value `c`.
fn level(c: StdULogic, zero: Drive, one: Drive) -> StdULogic {
    use StdULogic::{H, L, One, W, X, Z, Zero};
    match (c, zero, one) {
        (Zero, Drive::Strong, _) => Zero,
        (Zero, Drive::Weak, _) => L,
        (One, _, Drive::Strong) => One,
        (One, _, Drive::Weak) => H,
        (Zero | One, ..) => Z,
        (Z, ..) => Z,
        // An unknown value has the strength of the stronger side of the pair.
        _ => match (zero, one) {
            (Drive::Strong, _) | (_, Drive::Strong) => X,
            _ => W,
        },
    }
}

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
    /// The nets that drive VHDL; the flag: a `bit` port, which takes x and z as '0'.
    pub(crate) outputs: Vec<(NetId, DriverId, bool)>,
    pub(crate) started: bool,
}

pub(crate) struct Net {
    pub(crate) value: LogicVec,
    waiters: Vec<Waiter>,
    /// The output port this net drives (`Domain::outputs` index).
    pub(crate) out: Option<u32>,
    /// A resolved net: the nets that drive it and its pull.
    res: Option<Box<Res>>,
    /// A driver net: the resolved net it drives and how strongly.
    drives: Option<(u32, Drive, Drive)>,
}

struct Res {
    drivers: Vec<Dr>,
    /// `L` or `H` on every element: `tri0`, `tri1`.
    pull: Option<StdULogic>,
    /// Every driver is strong (`Dr::weak` false) and there is no pull.
    plain: bool,
}

/// A driver net of a resolved net and how it maps values to levels.
struct Dr {
    net: u32,
    weak: bool,
    zero: Drive,
    one: Drive,
}

/// Work done by resolution, for tests of the fast paths: single-driver nets add nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Stats {
    /// Writes to driver nets that changed a value and so resolved their net.
    pub resolutions: u64,
    /// Elements resolved by them.
    pub elements: u64,
}

/// Elements resolved on the stack at a time.
const CHUNK: usize = 64;

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

#[derive(Default)]
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
    pub(crate) stats: Stats,
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
                stats: Stats::default(),
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
            res: None,
            drives: None,
        });
        NetId(self.core.nets.len() as u32 - 1)
    }

    /// `drv` (a net of the width of `target`, `z` where it drives nothing) is a driver of
    /// `target`, driving `0` with strength `zero` and `1` with `one`. `target` is resolved from
    /// all its drivers whenever one of them changes.
    pub fn driver(&mut self, drv: NetId, target: NetId, zero: Drive, one: Drive) {
        self.core.nets[drv.0 as usize].drives = Some((target.0, zero, one));
        let res = self.core.nets[target.0 as usize]
            .res
            .get_or_insert_with(|| {
                Box::new(Res {
                    drivers: Vec::new(),
                    pull: None,
                    plain: true,
                })
            });
        let weak = (zero, one) != (Drive::Strong, Drive::Strong);
        res.plain &= !weak;
        res.drivers.push(Dr {
            net: drv.0,
            weak,
            zero,
            one,
        });
    }

    /// What resolution has done so far.
    pub fn stats(&self) -> Stats {
        self.core.stats
    }

    /// `target` has a pull of `l` (`L` or `H`) on every element, as a `tri0` or `tri1` net.
    pub fn pull(&mut self, target: NetId, l: StdULogic) {
        let res = self.core.nets[target.0 as usize]
            .res
            .get_or_insert_with(|| {
                Box::new(Res {
                    drivers: Vec::new(),
                    pull: None,
                    plain: true,
                })
            });
        res.pull = Some(l);
        res.plain = false;
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
            // A delay the resolution truncates to zero is `#0`.
            Wait::Delay(d) => match ctx.after(d)? {
                at if at > ctx.now() => self.future.entry(at).or_default().push(Item::Resume(p)),
                _ => self.inactive.push(p),
            },
            Wait::Zero => self.inactive.push(p),
            Wait::Done => {}
        }
        Ok(())
    }

    /// Elements `at..` of `net` take `v` at once; waiters whose item holds move to Active. A
    /// write to a driver net resolves, in the net it drives, the elements it changed.
    pub(crate) fn write(&mut self, net: NetId, at: u32, v: &LogicVec) {
        match self.nets[net.0 as usize].drives {
            None => {
                let n = &mut self.nets[net.0 as usize];
                if at == 0 && v.len() == n.value.len() {
                    // The whole net, the usual write: compared and copied as planes.
                    if n.value != *v {
                        let old = n.value.get(0);
                        n.value.clone_from(v);
                        self.changed(net, old);
                    }
                } else {
                    self.put_with(net, at, v.len(), |i| v.get(i));
                }
            }
            Some((target, ..)) => {
                let n = &mut self.nets[net.0 as usize];
                let (mut lo, mut hi) = (usize::MAX, 0);
                for i in 0..v.len() {
                    let (e, c) = (at as usize + i, v.get(i));
                    if n.value.get(e) != c {
                        n.value.set(e, c);
                        lo = lo.min(e);
                        hi = e;
                    }
                }
                if lo <= hi {
                    self.resolve(target, lo, hi + 1 - lo);
                }
            }
        }
    }

    /// Elements `at .. at + n` of resolved net `target` from its drivers and pull: a table
    /// lookup per driver and element on the stack, and for a wide change of strong drivers the
    /// kernel's plane-wise resolution.
    fn resolve(&mut self, target: u32, at: usize, n: usize) {
        let Some(res) = self.nets[target as usize].res.take() else {
            return;
        };
        self.stats.resolutions += 1;
        self.stats.elements += n as u64;
        let t = NetId(target);
        if n > CHUNK && res.plain && !res.drivers.is_empty() {
            // ponytail: allocates a slice per driver; wide changes are rare, and this is
            // the kernel's fast resolution.
            let parts: Vec<LogicVec> = res
                .drivers
                .iter()
                .map(|d| self.nets[d.net as usize].value.slice(at, n))
                .collect();
            let r = LogicVec::resolve(&parts.iter().collect::<Vec<_>>());
            self.put_with(t, at as u32, n, |i| r.get(i));
        } else {
            let mut buf = [StdULogic::Z; CHUNK];
            let mut done = 0;
            while done < n {
                let k = (n - done).min(CHUNK);
                for (j, slot) in buf[..k].iter_mut().enumerate() {
                    let e = at + done + j;
                    // The first driver (or the pull) starts the fold; a single `resolve` lookup
                    // per further driver is the common (driver, z) pair.
                    let mut r = res.pull.unwrap_or(StdULogic::Z);
                    for d in &res.drivers {
                        let c = self.nets[d.net as usize].value.get(e);
                        r = r.resolve(match d.weak {
                            false => c,
                            true => level(c, d.zero, d.one),
                        });
                    }
                    *slot = r;
                }
                let base = at + done;
                self.put_with(t, base as u32, k, |i| buf[i]);
                done += k;
            }
        }
        self.nets[target as usize].res = Some(res);
    }

    fn put_with(&mut self, net: NetId, at: u32, len: usize, get: impl Fn(usize) -> StdULogic) {
        let n = &mut self.nets[net.0 as usize];
        let old = n.value.get(0);
        let mut changed = false;
        // ponytail: a part element by element; plane-word copies if part writes show.
        for i in 0..len {
            let (e, c) = (at as usize + i, get(i));
            if n.value.get(e) != c {
                n.value.set(e, c);
                changed = true;
            }
        }
        if changed {
            self.changed(net, old);
        }
    }

    /// `net` changed from element 0 being `old`: waiters whose item holds move to Active, an
    /// output port records the new value.
    fn changed(&mut self, net: NetId, old: StdULogic) {
        let net = &mut self.nets[net.0 as usize];
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

impl<'a, 'c> Exec<'a, 'c> {
    pub(crate) fn new(core: &'a mut Core, ctx: &'a mut Ctx<'c>) -> Exec<'a, 'c> {
        Exec { core, ctx }
    }

    /// The domain's state and the kernel context (a process lowered to IR, `host.rs`).
    pub(crate) fn parts(&mut self) -> (&mut Core, &mut Ctx<'c>) {
        (self.core, self.ctx)
    }
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
        self.set_at(n, 0, v)
    }

    /// Blocking assignment to elements `at .. at + v.len()` of `n`.
    pub fn set_at(&mut self, n: NetId, at: u32, v: &LogicVec) -> Result<(), SimError> {
        self.core.check(n, at, v, self.ctx)?;
        self.core.write(n, at, v);
        Ok(())
    }

    /// `$write`: text without a line end, in the kernel's output order.
    pub fn output(&mut self, bytes: &[u8]) {
        self.ctx.output(bytes);
    }

    /// A run-time error of the running process, at this time.
    pub fn error(&self, msg: String) -> SimError {
        self.ctx.error(ErrorKind::Runtime(msg))
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
        match self.ctx.after(delay)? {
            t if t > self.ctx.now() => self.core.future.entry(t).or_default().push(Item::Nba(u)),
            _ => self.core.nba.push(u),
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

#[cfg(test)]
mod tests {
    //! Work counters and allocations of the net paths: a net with one driver costs no
    //! resolution, and resolving a narrow net allocates nothing.

    use super::*;
    use std::time::Instant;

    fn bits(s: &str) -> LogicVec {
        s.parse().expect("a logic string")
    }

    /// A target of `w` bits with two drivers (one weak when `weak`).
    fn bus(w: usize, weak: bool) -> (Domain, NetId, NetId, NetId) {
        let mut d = Domain::new();
        let t = d.net(LogicVec::new(w, StdULogic::Z));
        let (a, b) = (
            d.net(LogicVec::new(w, StdULogic::Z)),
            d.net(LogicVec::new(w, StdULogic::Z)),
        );
        d.driver(a, t, Drive::Strong, Drive::Strong);
        let b_drive = if weak { Drive::Weak } else { Drive::Strong };
        d.driver(b, t, b_drive, b_drive);
        (d, t, a, b)
    }

    #[test]
    fn a_net_without_resolution_does_no_resolution_work() {
        let mut d = Domain::new();
        let n = d.net(LogicVec::new(8, StdULogic::Z));
        for v in ["10101010", "01010101", "ZZZZZZZZ"] {
            d.core.write(n, 0, &bits(v));
        }
        assert_eq!(d.stats(), Stats::default());
    }

    #[test]
    fn resolution_covers_only_the_changed_elements_and_resolves_correctly() {
        let (mut d, t, a, b) = bus(16, false);
        d.core.write(a, 4, &bits("101"));
        assert_eq!(
            d.stats(),
            Stats {
                resolutions: 1,
                elements: 3
            }
        );
        // The same value again changes nothing.
        d.core.write(a, 4, &bits("101"));
        assert_eq!(d.stats().resolutions, 1);
        d.core.write(b, 5, &bits("1"));
        d.core.write(b, 6, &bits("1"));
        // Elements 6 to 0: a gives 1 0 1 at 6 to 4, b gives 1 at 5 and 6.
        let v = d.core.nets[t.0 as usize].value.slice(0, 7);
        assert_eq!(v, bits("1X1ZZZZ"));
        assert_eq!(d.stats().elements, 3 + 1 + 1);
    }

    /// Nanoseconds per write of a plain net, of a driver of a two-driver net (strong and
    /// weak) and of a driver of a 256-bit net: `cargo test -p yavs-verilog --lib bench_net_writes
    /// -- --ignored --nocapture`. Resolution costs a table lookup per driver and element.
    #[test]
    #[ignore]
    fn bench_net_writes() {
        const N: u32 = 200_000;
        let vals = [bits("10101010"), bits("ZZZZZZZZ"), bits("0101ZZ01")];
        let time = |label: &str, d: &mut Domain, net: NetId, vals: &[LogicVec]| {
            let t = Instant::now();
            for i in 0..N {
                d.core.write(net, 0, &vals[i as usize % vals.len()]);
            }
            let ns = t.elapsed().as_nanos() as f64 / f64::from(N);
            println!("{label}: {ns:.1} ns/write, {:?}", d.stats());
        };
        let mut d = Domain::new();
        let plain = d.net(LogicVec::new(8, StdULogic::Z));
        time("plain 8 bit", &mut d, plain, &vals);
        for weak in [false, true] {
            let (mut d, _, a, _) = bus(8, weak);
            time(&format!("two drivers, weak {weak}"), &mut d, a, &vals);
        }
        let wide: Vec<LogicVec> = ["1", "Z", "0"]
            .iter()
            .map(|c| bits(&c.repeat(256)))
            .collect();
        let (mut d, _, a, _) = bus(256, false);
        time("two drivers, 256 bit", &mut d, a, &wide);
    }
}
