//! Callback registry (#272): registration, enable, disable and remove, and which callbacks
//! are due in a phase or at a wake-up. Safe Rust only; `abi.rs` calls the plugin.
//!
//! Only enabled phase callbacks sit in the per-reason sets, so a disabled callback costs
//! nothing per cycle however many there are (#123). After-delay callbacks are kernel
//! wake-ups (`Hook::wake_at`); registering or removing one outside an observer call queues
//! the kernel request for the next one (`to_wake`, `to_cancel`). Value-change callbacks are
//! keyed by the kernel signals they watch; with none enabled, `WATCHING` is false and the
//! observer does not even take the state lock for a value change.

// VHPI names are matched as written in the C standard.
#![allow(non_upper_case_globals)]

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::atomic::{AtomicBool, Ordering};

use yavs_kernel::{Range, SignalId, Time, Unit, WakeId};

use crate::model::Object;
use crate::sys::*;

/// Simulation time as plugins see it, updated by the observer before each callback.
pub(crate) struct Clock {
    pub(crate) now: Time,
    pub(crate) delta: u32,
    /// The resolution limit (`yavs run --resolution`): VHPI times count in this unit.
    pub(crate) resolution: Unit,
}

impl Default for Clock {
    fn default() -> Self {
        Clock {
            now: Time::ZERO,
            delta: 0,
            resolution: Unit::Fs,
        }
    }
}

impl Clock {
    /// `now` in units of the resolution limit.
    pub(crate) fn vhpi_now(&self) -> vhpiTimeT {
        self.vhpi_time(self.now)
    }

    /// Kernel time `t` in units of the resolution limit.
    pub(crate) fn vhpi_time(&self, t: Time) -> vhpiTimeT {
        let ticks = t.as_fs() / self.resolution.fs();
        vhpiTimeT {
            high: (ticks >> 32) as i32,
            low: ticks as u32,
        }
    }

    /// A VHPI delay (units of the resolution limit) as a kernel time.
    fn delay(&self, t: vhpiTimeT) -> Option<Time> {
        let high = u64::try_from(t.high).ok()?;
        let ticks = (high << 32) | u64::from(t.low);
        Time::new(ticks, self.resolution).ok()
    }
}

pub(crate) struct Callback {
    pub(crate) data: vhpiCbDataT,
    pub(crate) state: vhpiStateT,
    /// The reason without `Rep`, and whether the callback repeats.
    base: i32,
    rep: bool,
    /// A handle was returned and not released: a matured callback stays readable until then.
    handle: bool,
    /// After-delay callbacks: the delay, when it is due, and the pending wake-up.
    delay: Time,
    at: Time,
    wake: Option<WakeId>,
    /// Value-change callbacks: the kernel elements of the object, and the cycle (time,
    /// delta) the callback last ran in.
    watch: Watched,
    last: Option<(Time, u32)>,
    /// The cycle epoch (`Callbacks::vc_epoch`) in which the callback was registered or
    /// enabled while that cycle's value changes were being dispatched; 0 for none.
    fresh: u64,
    /// The object of a value-change callback, whose value fills `data.value`.
    pub(crate) obj: Option<Object>,
}

/// The kernel elements a value-change callback watches.
pub(crate) type Watched = Box<[(SignalId, Range)]>;

/// Some value-change callback is enabled. Read without the state lock.
pub(crate) static WATCHING: AtomicBool = AtomicBool::new(false);

#[derive(Default)]
pub(crate) struct Callbacks {
    all: HashMap<u64, Callback>,
    next: u64,
    /// Enabled phase callbacks per base reason, in registration order.
    armed: HashMap<i32, BTreeSet<u64>>,
    /// Enabled value-change callbacks per kernel signal: the elements each watches, in
    /// registration order.
    watched: HashMap<SignalId, Vec<(Range, u64)>>,
    /// The cycle whose value changes are being dispatched (#346, #496) and its number
    /// (`vc_epoch`, counting cycles with value changes). `vc_in_bus`: a callback of this
    /// report is running, so a registration now is "fresh". `vc_runs`: per kernel signal the
    /// runs reported so far in the cycle. `vc_served`: the watch sets (the identity of a
    /// trigger) some callback was delivered in the cycle. `vc_pending`: fresh callbacks not
    /// offered the cycle's earlier changes yet.
    vc_cycle: Option<(Time, u32)>,
    vc_epoch: u64,
    vc_in_bus: bool,
    vc_runs: HashMap<SignalId, (u64, Vec<Range>)>,
    vc_served: HashSet<Watched>,
    vc_pending: Vec<u64>,
    /// Pending after-delay wake-ups.
    wakes: HashMap<WakeId, u64>,
    /// Kernel requests not made yet: after-delay callbacks to schedule, wake-ups to cancel.
    pub(crate) to_wake: Vec<u64>,
    pub(crate) to_cancel: Vec<WakeId>,
}

/// The reason without `Rep` (the reason it behaves as) and whether it repeats; `None` for a
/// reason yavs does not support. A value-change callback repeats until it is removed.
fn split(reason: i32) -> Option<(i32, bool)> {
    Some(match reason {
        vhpiCbValueChange => (reason, true),
        vhpiCbStartOfSimulation
        | vhpiCbEndOfSimulation
        | vhpiCbStartOfInitialization
        | vhpiCbEndOfInitialization
        | vhpiCbAfterDelay
        | vhpiCbNextTimeStep
        | vhpiCbStartOfNextCycle
        | vhpiCbStartOfProcesses
        | vhpiCbEndOfProcesses
        | vhpiCbLastKnownDeltaCycle
        | vhpiCbStartOfPostponed
        | vhpiCbEndOfTimeStep => (reason, false),
        vhpiCbRepAfterDelay => (vhpiCbAfterDelay, true),
        vhpiCbRepNextTimeStep => (vhpiCbNextTimeStep, true),
        vhpiCbRepStartOfNextCycle => (vhpiCbStartOfNextCycle, true),
        vhpiCbRepStartOfProcesses => (vhpiCbStartOfProcesses, true),
        vhpiCbRepEndOfProcesses => (vhpiCbEndOfProcesses, true),
        vhpiCbRepLastKnownDeltaCycle => (vhpiCbLastKnownDeltaCycle, true),
        vhpiCbRepStartOfPostponed => (vhpiCbStartOfPostponed, true),
        vhpiCbRepEndOfTimeStep => (vhpiCbEndOfTimeStep, true),
        // The foreign-model reasons: a time-out is an after-delay callback, a sensitivity
        // callback on a signal is a value-change callback (an event on it).
        vhpiCbTimeOut => (vhpiCbAfterDelay, false),
        vhpiCbRepTimeOut => (vhpiCbAfterDelay, true),
        vhpiCbSensitivity => (vhpiCbValueChange, true),
        _ => return None,
    })
}

/// Why a registration failed.
pub(crate) enum RegisterError {
    Unsupported,
    Misuse(&'static str),
}

impl Callbacks {
    /// Registers a callback; `delay` is the `time` field the caller passed (after-delay
    /// reasons need it), `watch` the object of a value-change callback and its kernel
    /// elements. Returns the callback's id.
    pub(crate) fn register(
        &mut self,
        data: vhpiCbDataT,
        flags: i32,
        delay: Option<vhpiTimeT>,
        watch: Option<(Object, Watched)>,
        clock: &Clock,
    ) -> Result<u64, RegisterError> {
        let (base, rep) = split(data.reason).ok_or(RegisterError::Unsupported)?;
        let (obj, watch) = match (base, watch) {
            (vhpiCbValueChange, None) => {
                return Err(RegisterError::Misuse(
                    "a value-change callback needs a signal, a port or a sub-element of one",
                ));
            }
            (_, w) => w.map_or((None, Box::default()), |(o, w)| (Some(o), w)),
        };
        let (delay, at) = if base == vhpiCbAfterDelay {
            let d = delay.ok_or(RegisterError::Misuse("after-delay callback without time"))?;
            let d = clock
                .delay(d)
                .ok_or(RegisterError::Misuse("negative or too large delay"))?;
            let at = clock
                .now
                .checked_add(d)
                .map_err(|_| RegisterError::Misuse("delay goes beyond TIME'HIGH"))?;
            (d, at)
        } else {
            (Time::ZERO, Time::ZERO)
        };
        let id = self.next;
        self.next += 1;
        let state = if flags & vhpiDisableCb != 0 {
            vhpiDisable
        } else {
            vhpiEnable
        };
        self.all.insert(
            id,
            Callback {
                data,
                state,
                base,
                rep,
                handle: flags & vhpiReturnCb != 0,
                delay,
                at,
                wake: None,
                watch,
                last: None,
                fresh: 0,
                obj,
            },
        );
        if base == vhpiCbAfterDelay {
            // Time runs while it is disabled: a disabled one is skipped when due.
            self.to_wake.push(id);
        } else if state == vhpiEnable {
            self.arm(id, true);
        }
        Ok(id)
    }

    /// Puts an enabled callback into the set its phase or value changes look at, or takes
    /// it out.
    fn arm(&mut self, id: u64, on: bool) {
        let cb = self.all.get_mut(&id).expect("armed callbacks exist");
        if cb.base != vhpiCbValueChange {
            let set = self.armed.entry(cb.base).or_default();
            if on {
                set.insert(id);
            } else {
                set.remove(&id);
            }
            return;
        }
        if on && self.vc_in_bus {
            cb.fresh = self.vc_epoch;
            self.vc_pending.push(id);
        }
        for &(sig, r) in &cb.watch {
            let list = self.watched.entry(sig).or_default();
            if on {
                list.push((r, id));
                // Registration order: an older callback may be enabled again later.
                list.sort_by_key(|&(_, i)| i);
            } else {
                list.retain(|&(_, i)| i != id);
                if list.is_empty() {
                    self.watched.remove(&sig);
                }
            }
        }
        WATCHING.store(!self.watched.is_empty(), Ordering::Relaxed);
    }

    pub(crate) fn get(&self, id: u64) -> Option<&Callback> {
        self.all.get(&id)
    }

    /// Enables or disables a callback; a matured one stays mature.
    pub(crate) fn set_enabled(&mut self, id: u64, on: bool) {
        let Some(cb) = self.all.get_mut(&id) else {
            return;
        };
        let (from, to) = if on {
            (vhpiDisable, vhpiEnable)
        } else {
            (vhpiEnable, vhpiDisable)
        };
        if cb.state != from {
            return;
        }
        cb.state = to;
        if cb.base != vhpiCbAfterDelay {
            self.arm(id, on);
        }
    }

    /// Removes a callback, also a matured one.
    pub(crate) fn remove(&mut self, id: u64) {
        if self.all.get(&id).is_some_and(|cb| cb.state == vhpiEnable) {
            self.arm(id, false);
        }
        let Some(cb) = self.all.remove(&id) else {
            return;
        };
        if let Some(w) = cb.wake {
            self.wakes.remove(&w);
            self.to_cancel.push(w);
        }
    }

    /// Its handle was released: a matured callback is gone for good.
    pub(crate) fn handle_released(&mut self, id: u64) {
        if let Some(cb) = self.all.get_mut(&id) {
            cb.handle = false;
            if cb.state == vhpiMature {
                self.all.remove(&id);
            }
        }
    }

    /// The enabled callbacks of `reason` (without `Rep`), in registration order. A snapshot:
    /// callbacks registered while these run wait for the next occurrence.
    pub(crate) fn due(&self, reason: i32) -> Vec<u64> {
        self.armed
            .get(&reason)
            .map_or_else(Vec::new, |s| s.iter().copied().collect())
    }

    /// The enabled value-change callbacks watching elements `r` of `sig` that have not run
    /// in this cycle (`now`, `delta`), in registration order; each is marked as run. The
    /// kernel reports a cycle's changes signal by signal in signal order, a vector's changed
    /// runs one by one (#346).
    ///
    /// A callback registered or enabled while the cycle's changes are dispatched (a framework
    /// re-arming a trigger from inside its callback) is *fresh* (#496). It is a new trigger,
    /// so it sees this change like any callback registered before the cycle, unless it is a
    /// re-arm: it watches the same elements as a callback already delivered in the cycle
    /// (`vc_served`), whose change it must not see again, nor the later runs of its vector.
    /// [`Callbacks::late`] offers it the changes reported before it was registered.
    pub(crate) fn changed(&mut self, sig: SignalId, r: Range, now: Time, delta: u32) -> Vec<u64> {
        let cycle = Some((now, delta));
        if self.vc_cycle != cycle {
            self.vc_cycle = cycle;
            self.vc_epoch += 1;
            self.vc_served.clear();
            self.vc_pending.clear();
        }
        let epoch = self.vc_epoch;
        let runs = self.vc_runs.entry(sig).or_default();
        if runs.0 != epoch {
            runs.0 = epoch;
            runs.1.clear();
        }
        runs.1.push(r);
        let mut ids = Vec::new();
        if let Some(list) = self.watched.get(&sig) {
            for &(w, id) in list {
                let cb = self.all.get_mut(&id).expect("watched callbacks exist");
                let rearm = cb.fresh == epoch && self.vc_served.contains(&cb.watch);
                if w.overlaps(r) && cb.last != cycle && !rearm {
                    cb.last = cycle;
                    ids.push(id);
                }
            }
        }
        self.serve(&ids);
        self.vc_in_bus = !ids.is_empty();
        ids
    }

    /// The fresh callbacks registered since the last call that watch an element changed
    /// earlier in this cycle and are not re-arms (see [`Callbacks::changed`]); each is marked
    /// as run. Call it after each batch of callbacks `changed` returned, until it returns
    /// nothing: the callbacks it returns can register more. Which signal the kernel reported
    /// first does not matter: a callback sees the cycle's change of a signal reported before
    /// or after its registration alike (the result is the same in every report order).
    pub(crate) fn late(&mut self) -> Vec<u64> {
        let (cycle, epoch) = (self.vc_cycle, self.vc_epoch);
        let mut ids = Vec::new();
        for id in std::mem::take(&mut self.vc_pending) {
            let Some(cb) = self.all.get_mut(&id) else {
                continue;
            };
            let seen = |&(s, w): &(SignalId, Range)| {
                let runs = self.vc_runs.get(&s);
                runs.is_some_and(|(e, v)| *e == epoch && v.iter().any(|&r| w.overlaps(r)))
            };
            if cb.state == vhpiEnable
                && cb.last != cycle
                && !self.vc_served.contains(&cb.watch)
                && cb.watch.iter().any(seen)
            {
                cb.last = cycle;
                ids.push(id);
            }
        }
        self.serve(&ids);
        self.vc_in_bus = !ids.is_empty();
        ids
    }

    /// The watch sets of callbacks delivered a change in this cycle, after a batch (so
    /// callbacks of one batch watching the same elements all see it).
    fn serve(&mut self, ids: &[u64]) {
        for id in ids {
            let w = &self.all[id].watch;
            if !self.vc_served.contains(w) {
                self.vc_served.insert(w.clone());
            }
        }
    }

    /// When the after-delay callback `id`, waiting for a kernel wake-up, is due.
    pub(crate) fn due_at(&self, id: u64) -> Option<Time> {
        self.all.get(&id).map(|cb| cb.at)
    }

    /// The kernel scheduled `w` for after-delay callback `id`.
    pub(crate) fn scheduled(&mut self, id: u64, w: WakeId) {
        if let Some(cb) = self.all.get_mut(&id) {
            cb.wake = Some(w);
            self.wakes.insert(w, id);
        }
    }

    /// The after-delay callback whose wake-up `w` is due.
    pub(crate) fn woken(&mut self, w: WakeId) -> Option<u64> {
        let id = self.wakes.remove(&w)?;
        if let Some(cb) = self.all.get_mut(&id) {
            cb.wake = None;
        }
        Some(id)
    }

    /// Callback `id` is due now. Returns its data when it is enabled and should run. A
    /// one-shot callback matures (and goes away unless a handle refers to it); a repeating
    /// after-delay callback is scheduled again.
    pub(crate) fn fire(&mut self, id: u64, now: Time) -> Option<vhpiCbDataT> {
        let cb = self.all.get_mut(&id)?;
        let enabled = cb.state == vhpiEnable;
        let after_delay = cb.base == vhpiCbAfterDelay;
        if !enabled && !after_delay {
            return None;
        }
        let data = cb.data;
        if cb.rep {
            // Past TIME'HIGH the next occurrence can never come.
            if let (true, Ok(at)) = (after_delay, now.checked_add(cb.delay)) {
                cb.at = at;
                self.to_wake.push(id);
            }
        } else {
            cb.state = vhpiMature;
            if let Some(set) = self.armed.get_mut(&cb.base) {
                set.remove(&id);
            }
            if !cb.handle {
                self.all.remove(&id);
            }
        }
        enabled.then_some(data)
    }

    #[cfg(test)]
    fn live(&self) -> usize {
        self.all.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::sync::{Mutex, MutexGuard};

    fn data(reason: i32) -> vhpiCbDataT {
        extern "C" fn nop(_: *const vhpiCbDataT) {}
        vhpiCbDataT {
            reason,
            cb_rtn: Some(nop),
            obj: std::ptr::null_mut(),
            time: std::ptr::null_mut(),
            value: std::ptr::null_mut(),
            user_data: std::ptr::null_mut(),
        }
    }

    /// `WATCHING` is global: the tests that register value-change callbacks run one at a time.
    fn serial() -> MutexGuard<'static, ()> {
        static LOCK: Mutex<()> = Mutex::new(());
        LOCK.lock().unwrap_or_else(|e| e.into_inner())
    }

    fn register(cbs: &mut Callbacks, reason: i32, flags: i32) -> u64 {
        let ok = cbs.register(data(reason), flags, None, None, &Clock::default());
        ok.unwrap_or_else(|_| panic!("register {reason}"))
    }

    /// #123: disabled callbacks are not in the per-reason sets, so a cycle never looks at them.
    #[test]
    fn disabled_callbacks_cost_nothing_per_cycle() {
        let mut cbs = Callbacks::default();
        for _ in 0..100_000 {
            register(
                &mut cbs,
                vhpiCbRepNextTimeStep,
                vhpiDisableCb | vhpiReturnCb,
            );
        }
        let on = register(&mut cbs, vhpiCbRepNextTimeStep, vhpiReturnCb);
        assert_eq!(cbs.due(vhpiCbNextTimeStep), [on]);
        cbs.set_enabled(on, false);
        assert!(cbs.due(vhpiCbNextTimeStep).is_empty());
        cbs.set_enabled(5, true);
        assert_eq!(cbs.due(vhpiCbNextTimeStep), [5]);
        assert_eq!(cbs.live(), 100_001);
    }

    #[test]
    fn one_shot_matures_and_repeating_stays() {
        let mut cbs = Callbacks::default();
        let once = register(&mut cbs, vhpiCbEndOfTimeStep, vhpiReturnCb);
        let gone = register(&mut cbs, vhpiCbEndOfTimeStep, 0);
        let rep = register(&mut cbs, vhpiCbRepEndOfTimeStep, 0);
        assert_eq!(cbs.due(vhpiCbEndOfTimeStep), [once, gone, rep]);
        for id in [once, gone, rep] {
            assert!(cbs.fire(id, Time::ZERO).is_some());
        }
        assert_eq!(cbs.get(once).map(|c| c.state), Some(vhpiMature));
        assert!(cbs.get(gone).is_none(), "no handle: dropped when mature");
        assert_eq!(cbs.due(vhpiCbEndOfTimeStep), [rep]);
        // Removing a matured callback is legal; enabling it changes nothing.
        cbs.set_enabled(once, true);
        assert!(cbs.due(vhpiCbEndOfTimeStep) == [rep]);
        cbs.remove(once);
        assert!(cbs.get(once).is_none());
    }

    /// Value-change callbacks: keyed by signal, once per cycle, only while enabled.
    #[test]
    fn value_changes_by_signal_once_per_cycle() {
        let _serial = serial();
        let mut cbs = Callbacks::default();
        let clock = Clock::default();
        let (a, b) = (SignalId(3), SignalId(4));
        let mut watch = |w: &[(SignalId, Range)]| {
            let ok = cbs.register(
                data(vhpiCbValueChange),
                vhpiReturnCb,
                None,
                Some((Object::Tool, w.into())),
                &clock,
            );
            ok.unwrap_or_else(|_| panic!("register"))
        };
        let whole = watch(&[(a, Range::new(0, 8))]);
        let bit = watch(&[(a, Range::new(2, 1))]);
        let rec = watch(&[(a, Range::new(4, 4)), (b, Range::new(0, 1))]);
        assert!(WATCHING.load(Ordering::Relaxed));
        let t = Time::ZERO;
        assert_eq!(cbs.changed(a, Range::new(0, 2), t, 1), [whole]);
        assert_eq!(cbs.changed(a, Range::new(2, 4), t, 1), [bit, rec]);
        assert!(
            cbs.changed(b, Range::new(0, 1), t, 1).is_empty(),
            "once per cycle"
        );
        assert_eq!(cbs.changed(b, Range::new(0, 1), t, 2), [rec]);
        cbs.set_enabled(whole, false);
        assert_eq!(cbs.changed(a, Range::new(0, 8), t, 3), [bit, rec]);
        cbs.set_enabled(whole, true);
        assert_eq!(cbs.changed(a, Range::new(0, 8), t, 4), [whole, bit, rec]);
        assert!(cbs.fire(whole, t).is_some(), "value change repeats");
        for id in [whole, bit, rec] {
            cbs.remove(id);
        }
        assert!(cbs.changed(a, Range::new(0, 8), t, 6).is_empty());
        assert!(!WATCHING.load(Ordering::Relaxed));
    }

    /// A value-change callback on `w`, registered at any time.
    fn watch(cbs: &mut Callbacks, w: &[(SignalId, Range)]) -> u64 {
        let ok = cbs.register(
            data(vhpiCbValueChange),
            vhpiReturnCb,
            None,
            Some((Object::Tool, w.into())),
            &Clock::default(),
        );
        ok.unwrap_or_else(|_| panic!("register"))
    }

    /// One cycle (`delta`): the kernel's reports in the given order, each followed by the
    /// callbacks it delivers and by those they make due (`late`), as `VhpiObserver` does.
    /// `body` plays the callback; callbacks removed by earlier ones do not run. Returns the
    /// ids that ran, in order.
    fn cycle(
        cbs: &mut Callbacks,
        delta: u32,
        reports: &[(SignalId, Range)],
        mut body: impl FnMut(&mut Callbacks, u64),
    ) -> Vec<u64> {
        let mut ran = Vec::new();
        for &(sig, r) in reports {
            let mut ids = cbs.changed(sig, r, Time::ZERO, delta);
            while !ids.is_empty() {
                for id in ids {
                    if cbs.get(id).is_some() {
                        ran.push(id);
                        body(cbs, id);
                    }
                }
                ids = cbs.late();
            }
        }
        ran
    }

    const WHOLE: Range = Range::new(0, 8);

    /// Waiting repeatedly for the first of two triggers that fire in the same cycle (remove
    /// both, register both anew from the callback of the first) sees each once, whichever the
    /// kernel reports first (#496), and the re-armed trigger of the signal being reported
    /// does not see its own change again (#346).
    #[test]
    fn first_of_two_coincident_triggers_in_either_report_order() {
        let _serial = serial();
        let (a, b) = (SignalId(1), SignalId(2));
        for (delta, order) in [(1, [a, b]), (2, [b, a])] {
            let mut cbs = Callbacks::default();
            let ids = Cell::new([0u64; 2]);
            let arm = |cbs: &mut Callbacks| {
                ids.set([watch(cbs, &[(a, WHOLE)]), watch(cbs, &[(b, WHOLE)])]);
            };
            arm(&mut cbs);
            let reports = order.map(|s| (s, WHOLE));
            let ran = cycle(&mut cbs, delta, &reports, |cbs, _| {
                ids.get().iter().for_each(|&i| cbs.remove(i));
                arm(cbs);
            });
            // The first one reported runs; its re-armed trigger on the other signal runs at
            // the other's report (or at once, when that was reported before); nothing else.
            let sigs: Vec<_> = ran.iter().map(|i| if i % 2 == 0 { a } else { b }).collect();
            assert_eq!(sigs, order, "order {order:?}");
        }
    }

    /// A callback registered from the callback of one signal on another signal that changes
    /// in the same cycle sees that change once, also when the kernel reported it first, and
    /// re-arming it from inside itself does not repeat the change (#496). Registered outside
    /// the dispatch of the changes it sees nothing of the cycle.
    #[test]
    fn late_registration_sees_a_change_reported_before_it() {
        let _serial = serial();
        let (a, b) = (SignalId(1), SignalId(2));
        for (delta, order) in [(1, [a, b]), (2, [b, a])] {
            let mut cbs = Callbacks::default();
            let x = watch(&mut cbs, &[(a, WHOLE)]);
            let second = Cell::new(None);
            let reports = order.map(|s| (s, WHOLE));
            let mut ran = cycle(&mut cbs, delta, &reports, |cbs, id| {
                if id == x {
                    second.set(Some(watch(cbs, &[(b, WHOLE)])));
                } else {
                    // Re-arm from inside itself, as frameworks do.
                    cbs.remove(id);
                    second.set(Some(watch(cbs, &[(b, WHOLE)])));
                }
            });
            let y = ran.pop().expect("second ran");
            assert_eq!(ran, [x], "order {order:?}");
            // The re-armed one is not run again in the cycle, and runs in the next one.
            let y2 = second.get().expect("re-armed");
            assert_ne!(y, y2);
            let mut next = cycle(&mut cbs, delta + 10, &reports, |_, _| {});
            next.sort_unstable();
            assert_eq!(next, [x, y2]);
            // Registered after the dispatch (a read-write or timed callback): not run again
            // for the changes of the cycle, only by the next one.
            let g = watch(&mut cbs, &[(a, WHOLE)]);
            assert!(cbs.late().is_empty());
            assert!(!cbs.changed(b, WHOLE, Time::ZERO, delta + 10).contains(&g));
            assert!(cbs.changed(a, WHOLE, Time::ZERO, delta + 11).contains(&g));
        }
    }

    /// The runs of a vector come one by one (#346): the callback re-armed on the vector sees
    /// neither the run being reported nor a later one; a new one on other elements sees the
    /// runs reported before it and after it, once.
    #[test]
    fn vector_runs_after_a_registration() {
        let _serial = serial();
        let a = SignalId(1);
        let mut cbs = Callbacks::default();
        let whole = watch(&mut cbs, &[(a, WHOLE)]);
        let (lo, hi) = (Range::new(0, 2), Range::new(5, 2));
        let (mut again, mut bit_lo, mut bit_hi) = (0, 0, 0);
        let ran = cycle(&mut cbs, 1, &[(a, lo), (a, hi)], |cbs, id| {
            if id == whole {
                cbs.remove(id);
                again = watch(cbs, &[(a, WHOLE)]);
                bit_lo = watch(cbs, &[(a, Range::new(1, 1))]);
                bit_hi = watch(cbs, &[(a, Range::new(6, 1))]);
            }
        });
        assert_eq!(ran, [whole, bit_lo, bit_hi]);
        assert!(!ran.contains(&again));
        // The next cycle: everyone, once.
        assert_eq!(
            cycle(&mut cbs, 2, &[(a, lo), (a, hi)], |_, _| {}),
            [again, bit_lo, bit_hi]
        );
    }

    /// A callback enabled while the dispatch runs is fresh too.
    #[test]
    fn enabling_during_the_dispatch_is_a_registration() {
        let _serial = serial();
        let (a, b) = (SignalId(1), SignalId(2));
        let mut cbs = Callbacks::default();
        let x = watch(&mut cbs, &[(a, WHOLE)]);
        let off = cbs
            .register(
                data(vhpiCbValueChange),
                vhpiReturnCb | vhpiDisableCb,
                None,
                Some((Object::Tool, [(b, WHOLE)][..].into())),
                &Clock::default(),
            )
            .unwrap_or_else(|_| panic!("register"));
        let ran = cycle(&mut cbs, 1, &[(b, WHOLE), (a, WHOLE)], |cbs, id| {
            if id == x {
                cbs.set_enabled(off, true);
            }
        });
        assert_eq!(ran, [x, off]);
    }

    #[test]
    fn after_delay_times_follow_the_resolution() {
        let mut cbs = Callbacks::default();
        let clock = Clock {
            now: Time::ns(3),
            delta: 0,
            resolution: Unit::Ns,
        };
        let d = vhpiTimeT { high: 0, low: 4 };
        let Ok(id) = cbs.register(data(vhpiCbAfterDelay), 0, Some(d), None, &clock) else {
            panic!("register");
        };
        assert_eq!(cbs.due_at(id), Some(Time::ns(7)));
        assert_eq!(std::mem::take(&mut cbs.to_wake), [id]);
        assert_eq!((clock.vhpi_now().high, clock.vhpi_now().low), (0, 3));
        let neg = vhpiTimeT { high: -1, low: 0 };
        assert!(matches!(
            cbs.register(data(vhpiCbAfterDelay), 0, Some(neg), None, &clock),
            Err(RegisterError::Misuse(_))
        ));
        assert!(matches!(
            cbs.register(data(vhpiCbAfterDelay), 0, None, None, &clock),
            Err(RegisterError::Misuse(_))
        ));
        assert!(matches!(
            cbs.register(data(vhpiCbStmt), 0, None, None, &clock),
            Err(RegisterError::Unsupported)
        ));
    }
}
