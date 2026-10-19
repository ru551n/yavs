//! Callback registry (#272): registration, enable, disable and remove, and which callbacks
//! are due in a phase or at a wake-up. Safe Rust only; `abi.rs` calls the plugin.
//!
//! Only enabled phase callbacks sit in the per-reason sets, so a disabled callback costs
//! nothing per cycle however many there are (#123). After-delay callbacks are kernel
//! wake-ups (`Hook::wake_at`); registering or removing one outside an observer call queues
//! the kernel request for the next one (`to_wake`, `to_cancel`).

// VHPI names are matched as written in the C standard.
#![allow(non_upper_case_globals)]

use std::collections::{BTreeSet, HashMap};

use yavs_kernel::{Time, Unit, WakeId};

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
}

#[derive(Default)]
pub(crate) struct Callbacks {
    all: HashMap<u64, Callback>,
    next: u64,
    /// Enabled phase callbacks per base reason, in registration order.
    armed: HashMap<i32, BTreeSet<u64>>,
    /// Pending after-delay wake-ups.
    wakes: HashMap<WakeId, u64>,
    /// Kernel requests not made yet: after-delay callbacks to schedule, wake-ups to cancel.
    pub(crate) to_wake: Vec<u64>,
    pub(crate) to_cancel: Vec<WakeId>,
}

/// The reason without `Rep` and whether it repeats; `None` for a reason yavs does not
/// support. `vhpiCbValueChange` waits for the object model (#269).
fn split(reason: i32) -> Option<(i32, bool)> {
    Some(match reason {
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
    /// reasons need it). Returns the callback's id.
    pub(crate) fn register(
        &mut self,
        data: vhpiCbDataT,
        flags: i32,
        delay: Option<vhpiTimeT>,
        clock: &Clock,
    ) -> Result<u64, RegisterError> {
        let (base, rep) = split(data.reason).ok_or(RegisterError::Unsupported)?;
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
            },
        );
        if base == vhpiCbAfterDelay {
            // Time runs while it is disabled: a disabled one is skipped when due.
            self.to_wake.push(id);
        } else if state == vhpiEnable {
            self.armed.entry(base).or_default().insert(id);
        }
        Ok(id)
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
            let set = self.armed.entry(cb.base).or_default();
            if on {
                set.insert(id);
            } else {
                set.remove(&id);
            }
        }
    }

    /// Removes a callback, also a matured one.
    pub(crate) fn remove(&mut self, id: u64) {
        let Some(cb) = self.all.remove(&id) else {
            return;
        };
        if let Some(set) = self.armed.get_mut(&cb.base) {
            set.remove(&id);
        }
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
        let ok = cbs.register(data(reason), flags, None, &Clock::default());
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

    #[test]
    fn after_delay_times_follow_the_resolution() {
        let mut cbs = Callbacks::default();
        let clock = Clock {
            now: Time::ns(3),
            delta: 0,
            resolution: Unit::Ns,
        };
        let d = vhpiTimeT { high: 0, low: 4 };
        let Ok(id) = cbs.register(data(vhpiCbAfterDelay), 0, Some(d), &clock) else {
            panic!("register");
        };
        assert_eq!(cbs.due_at(id), Some(Time::ns(7)));
        assert_eq!(std::mem::take(&mut cbs.to_wake), [id]);
        assert_eq!((clock.vhpi_now().high, clock.vhpi_now().low), (0, 3));
        let neg = vhpiTimeT { high: -1, low: 0 };
        assert!(matches!(
            cbs.register(data(vhpiCbAfterDelay), 0, Some(neg), &clock),
            Err(RegisterError::Misuse(_))
        ));
        assert!(matches!(
            cbs.register(data(vhpiCbAfterDelay), 0, None, &clock),
            Err(RegisterError::Misuse(_))
        ));
        assert!(matches!(
            cbs.register(data(vhpiCbValueChange), 0, None, &clock),
            Err(RegisterError::Unsupported)
        ));
    }
}
