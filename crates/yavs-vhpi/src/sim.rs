//! The kernel observer that runs VHPI callbacks (#272): phase callbacks at the matching
//! `Phase`, after-delay callbacks as kernel wake-ups, and `vhpi_control` requests as the end
//! of the run. Safe Rust only; `abi::fire` calls the plugin.
//!
//! Value-change callbacks (`Observer::value_change`) come with the object model (#269).

// VHPI names are matched as written in the C standard.
#![allow(non_upper_case_globals)]

use yavs_kernel::{EndReason, Hook, Phase, WakeId};

use crate::abi::{fire, hooked, with_state};
use crate::model::{Control, State};
use crate::sys::*;

/// Subscribe one to the simulation that runs the loaded plugins.
pub struct VhpiObserver;

impl VhpiObserver {
    /// Runs the callbacks `due` picks at the hook's time, then makes the kernel requests
    /// they left. One lock when nothing is due.
    fn dispatch(hook: &mut Hook<'_>, due: impl FnOnce(&mut State) -> Vec<u64>) {
        let ids = with_state(|s| {
            s.clock.now = hook.now();
            s.clock.delta = hook.delta();
            let ids = due(s);
            if ids.is_empty() {
                requests(s, hook);
            }
            ids
        });
        if !ids.is_empty() {
            hooked(hook, || fire(&ids));
            with_state(|s| requests(s, hook));
        }
    }
}

/// The kernel requests plugins left: after-delay wake-ups, cancels, the end of the run.
fn requests(s: &mut State, hook: &mut Hook<'_>) {
    for id in std::mem::take(&mut s.cbs.to_wake) {
        if let Some(at) = s.cbs.due_at(id) {
            // `at` is now plus a delay, taken at a time no later than this one.
            let w = hook
                .wake_at(at)
                .expect("after-delay callback due in the past: this is a bug in yavs");
            s.cbs.scheduled(id, w);
        }
    }
    for w in std::mem::take(&mut s.cbs.to_cancel) {
        hook.cancel(w);
    }
    match s.control.take() {
        Some(Control::Finish) => hook.end(EndReason::Finish(0)),
        Some(Control::Stop) => hook.end(EndReason::Stop(0)),
        None => {}
    }
}

impl yavs_kernel::Observer for VhpiObserver {
    fn phase(&mut self, hook: &mut Hook<'_>, phase: Phase) {
        let reasons: &[i32] = match phase {
            Phase::StartOfSim => &[vhpiCbStartOfSimulation, vhpiCbStartOfInitialization],
            Phase::EndOfInit => &[vhpiCbEndOfInitialization],
            Phase::NextTimeStep => &[vhpiCbNextTimeStep],
            Phase::NextCycle => &[vhpiCbStartOfNextCycle],
            Phase::LastKnownDelta => &[vhpiCbLastKnownDeltaCycle],
            Phase::EndOfTimeStep => &[vhpiCbEndOfTimeStep],
            Phase::EndOfSim => &[vhpiCbEndOfSimulation],
        };
        for &r in reasons {
            Self::dispatch(hook, |s| s.cbs.due(r));
        }
    }

    fn wake(&mut self, hook: &mut Hook<'_>, id: WakeId) {
        Self::dispatch(hook, |s| s.cbs.woken(id).into_iter().collect());
    }
}
