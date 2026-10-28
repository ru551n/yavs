//! The kernel observer that runs VHPI callbacks (#272): phase callbacks at the matching
//! `Phase`, after-delay callbacks as kernel wake-ups, value-change callbacks on
//! `Observer::value_change`, `vhpi_put_value` as kernel forces and deposits (#271), and
//! `vhpi_control` requests as the end of the run. Safe Rust only; `abi::fire` calls the
//! plugin.

// VHPI names are matched as written in the C standard.
#![allow(non_upper_case_globals)]

use std::sync::atomic::Ordering;

use yavs_elab::{Built, ScopeId};
use yavs_interp::{CallVal, InterpProcess};
use yavs_kernel::{EndReason, Hook, Phase, ProcId, Range, SignalId, WakeId};
use yavs_sem::DeclId;
use yavs_sem::stat::ConstValue;

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
    ) -> bool {
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
        !ids.is_empty()
    }
}

/// The kernel requests plugins left: puts (unless `hold_puts`), after-delay wake-ups,
/// cancels, the end of the run.
fn requests(s: &mut State, hook: &mut Hook<'_>, hold_puts: bool) {
    crate::foreign::pending(s, hook);
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

/// Calls `f` with kernel process `proc`, an interpreter process (observers run between
/// phases: it is suspended).
fn interp<R>(
    h: &mut Hook<'_>,
    proc: u32,
    f: impl FnOnce(&mut InterpProcess) -> Result<R, String>,
) -> Result<R, String> {
    h.with_process(ProcId(proc), |p| {
        let p = p.as_any().and_then(|a| a.downcast_mut::<InterpProcess>());
        f(p.ok_or("this is a bug in yavs: not an interpreter process")?)
    })
    .unwrap_or_else(|| Err(format!("this is a bug in yavs: no process {proc}")))
}

/// A change of a variable's whole value; `None` from it: the new value does not fit.
pub(crate) type Update<'a> = Option<&'a mut dyn FnMut(&mut ConstValue) -> Option<()>>;

/// The value of variable `name` of kernel process `proc` (#309), changed by `update` when
/// given (`None` from it: the new value does not fit).
pub(crate) fn var(
    h: &mut Hook<'_>,
    proc: u32,
    name: &str,
    update: Update<'_>,
) -> Result<ConstValue, String> {
    interp(h, proc, |p| {
        let (ty, v) = p.var(name)?;
        let (v, new) = change(p.program(), ty, &v, name, update)?;
        if let Some(new) = new {
            p.set_var(name, &new)?;
        }
        Ok(v)
    })
}

/// [`var`] of the non-protected shared variable `decl` of region `scope` (`None`: a
/// package's).
pub(crate) fn shared_var(
    b: &Built,
    scope: Option<ScopeId>,
    decl: DeclId,
    update: Update<'_>,
) -> Result<ConstValue, String> {
    let Some(&(slot, ty)) = b.vars.get(&(scope, decl)) else {
        return Err("this shared variable has no value to read (a protected object)".into());
    };
    let v = yavs_interp::shared_var(&b.rt, &b.shared, slot, ty)?;
    let (v, new) = change(&b.rt.prog, ty, &v, "shared", update)?;
    if let Some(new) = new {
        yavs_interp::set_shared_var(&b.rt, &b.shared, slot, ty, &new)?;
    }
    Ok(v)
}

/// Value `v` of IR type `ty` decoded, and with `update` the changed value encoded.
fn change(
    prog: &yavs_ir::Program,
    ty: yavs_ir::TyId,
    v: &CallVal,
    name: &str,
    update: Update<'_>,
) -> Result<(ConstValue, Option<CallVal>), String> {
    let bug = || format!("this is a bug in yavs: variable `{name}` not decoded or encoded");
    let mut whole = yavs_elab::ir_decode(prog, ty, v).ok_or_else(bug)?;
    let Some(f) = update else {
        return Ok((whole, None));
    };
    f(&mut whole).ok_or_else(bug)?;
    let new = yavs_elab::ir_encode(prog, ty, &whole).ok_or_else(bug)?;
    Ok((whole, Some(new)))
}

impl yavs_kernel::Observer for VhpiObserver {
    /// `vhpi_get_next_time` (#828).
    fn wants_next_time(&self) -> bool {
        true
    }

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
        if phase == Phase::StartOfSim {
            // Library and application models (#357) run before the phase's callbacks.
            with_state(|s| {
                s.clock.now = hook.now();
                s.clock.delta = hook.delta();
                s.read_only = false;
            });
            hooked(hook, crate::foreign::start_models);
            with_state(|s| requests(s, hook, false));
        }
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
        let mut ran = Self::dispatch(hook, false, false, |s| {
            let (now, delta) = (s.clock.now, s.clock.delta);
            s.cbs.changed(signal, range, now, delta)
        });
        // Callbacks registered by those see the cycle's changes reported before them (#496).
        while ran {
            ran = Self::dispatch(hook, false, false, |s| s.cbs.late());
        }
    }
}
