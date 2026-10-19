//! The kernel observer that runs VHPI callbacks (#272): phase callbacks at the matching
//! `Phase`, after-delay callbacks as kernel wake-ups, value-change callbacks on
//! `Observer::value_change`, `vhpi_put_value` as kernel forces and deposits (#271), and
//! `vhpi_control` requests as the end of the run. Safe Rust only; `abi::fire` calls the
//! plugin.

// VHPI names are matched as written in the C standard.
#![allow(non_upper_case_globals)]

use std::sync::atomic::Ordering;

use yavs_kernel::{EndReason, Hook, Phase, Range, SignalId, WakeId};

use crate::abi::{fire, hooked, with_state};
use crate::callbacks::WATCHING;
use crate::model::{Control, PutOp, State};
use crate::sys::*;

/// Subscribe one to the simulation that runs the loaded plugins.
pub struct VhpiObserver;

impl VhpiObserver {
    /// Runs the callbacks `due` picks at the hook's time, then makes the kernel requests
    /// they left. One lock when nothing is due. In a `read_only` phase `vhpi_put_value` is
    /// refused. Before the update (`early`: after-delay, next time step, start of cycle) puts
    /// wait for the next call after it, so they make a new delta as in the references with
    /// VHPI (#319).
    fn dispatch(
        hook: &mut Hook<'_>,
        read_only: bool,
        early: bool,
        due: impl FnOnce(&mut State) -> Vec<u64>,
    ) {
        let ids = with_state(|s| {
            s.clock.now = hook.now();
            s.clock.delta = hook.delta();
            s.read_only = read_only;
            let ids = due(s);
            if ids.is_empty() {
                requests(s, hook, early);
            }
            ids
        });
        if !ids.is_empty() {
            hooked(hook, || fire(&ids));
            with_state(|s| requests(s, hook, early));
        }
    }
}

/// The kernel requests plugins left: puts (unless `hold_puts`), after-delay wake-ups,
/// cancels, the end of the run.
fn requests(s: &mut State, hook: &mut Hook<'_>, hold_puts: bool) {
    let puts = if hold_puts {
        Vec::new()
    } else {
        std::mem::take(&mut s.puts)
    };
    for p in puts {
        let done = match &p.op {
            PutOp::Deposit(v) => hook.deposit(p.at, v),
            PutOp::Force(m, v) => hook.force(p.at, *m, v),
            PutOp::Release(m) => hook.release(p.at, *m),
        };
        // `vhpi_put_value` checked the elements and the value against the object.
        done.expect("VHPI put refused by the kernel: this is a bug in yavs");
    }
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
            Phase::StartOfProcesses => &[vhpiCbStartOfProcesses],
            Phase::EndOfProcesses => &[vhpiCbEndOfProcesses],
            Phase::LastKnownDelta => &[vhpiCbLastKnownDeltaCycle],
            Phase::StartOfPostponed => &[vhpiCbStartOfPostponed],
            Phase::EndOfTimeStep => &[vhpiCbEndOfTimeStep],
            Phase::EndOfSim => &[vhpiCbEndOfSimulation],
        };
        // The read-only phases: nothing may change the design any more at this time.
        let read_only = matches!(
            phase,
            Phase::StartOfPostponed | Phase::EndOfTimeStep | Phase::EndOfSim
        );
        // Puts from before the update are handed over at `StartOfProcesses` (#319).
        let early = matches!(phase, Phase::NextTimeStep | Phase::NextCycle);
        for &r in reasons {
            Self::dispatch(hook, read_only, early, |s| s.cbs.due(r));
        }
    }

    fn wake(&mut self, hook: &mut Hook<'_>, id: WakeId) {
        Self::dispatch(hook, false, true, |s| s.cbs.woken(id).into_iter().collect());
    }

    fn value_change(&mut self, hook: &mut Hook<'_>, signal: SignalId, range: Range) {
        // #123: no lock per event while no value-change callback is enabled.
        if !WATCHING.load(Ordering::Relaxed) {
            return;
        }
        Self::dispatch(hook, false, false, |s| {
            let (now, delta) = (s.clock.now, s.clock.delta);
            s.cbs.changed(signal, range, now, delta)
        });
    }
}
