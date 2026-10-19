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

use std::collections::{BTreeSet, HashMap};
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
        let ticks = self.now.as_fs() / self.resolution.fs();
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
    /// Pending after-delay wake-ups.
    wakes: HashMap<WakeId, u64>,
    /// Kernel requests not made yet: after-delay callbacks to schedule, wake-ups to cancel.
    pub(crate) to_wake: Vec<u64>,
    pub(crate) to_cancel: Vec<WakeId>,
}

/// The reason without `Rep` and whether it repeats; `None` for a reason yavs does not
/// support. A value-change callback repeats until it is removed.
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
        | vhpiCbLastKnownDeltaCycle
        | vhpiCbEndOfTimeStep => (reason, false),
        vhpiCbRepAfterDelay => (vhpiCbAfterDelay, true),
        vhpiCbRepNextTimeStep => (vhpiCbNextTimeStep, true),
        vhpiCbRepStartOfNextCycle => (vhpiCbStartOfNextCycle, true),
        vhpiCbRepLastKnownDeltaCycle => (vhpiCbLastKnownDeltaCycle, true),
        vhpiCbRepEndOfTimeStep => (vhpiCbEndOfTimeStep, true),
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
        let cb = &self.all[&id];
        if cb.base != vhpiCbValueChange {
            let set = self.armed.entry(cb.base).or_default();
            if on {
                set.insert(id);
            } else {
                set.remove(&id);
            }
            return;
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
    /// in this cycle (`now`, `delta`), in registration order; each is marked as run.
    pub(crate) fn changed(&mut self, sig: SignalId, r: Range, now: Time, delta: u32) -> Vec<u64> {
        let Some(list) = self.watched.get(&sig) else {
            return Vec::new();
        };
        let mut ids = Vec::new();
        for &(w, id) in list {
            let cb = self.all.get_mut(&id).expect("watched callbacks exist");
            if w.overlaps(r) && cb.last != Some((now, delta)) {
                cb.last = Some((now, delta));
                ids.push(id);
            }
        }
        ids
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
        assert!(cbs.changed(a, Range::new(0, 8), t, 5).is_empty());
        assert!(!WATCHING.load(Ordering::Relaxed));
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
            cbs.register(data(vhpiCbSensitivity), 0, None, None, &clock),
            Err(RegisterError::Unsupported)
        ));
    }
}
