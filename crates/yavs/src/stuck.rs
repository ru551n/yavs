//! Hang reports of `yavs run`: the SIGUSR1 request and the end-of-run decision.

use std::sync::atomic::{AtomicBool, Ordering};

use yavs_kernel::{Interrupt, ProcWait, StuckReport};

/// A report was asked for (SIGUSR1) since the last `take_request`.
static REQUESTED: AtomicBool = AtomicBool::new(false);

/// The SIGUSR1 hook of `yavs_wave::interrupt` (its thread, no thread of ours): sets the flag and
/// pauses the kernel between two cycles, `yavs run` reports and continues.
pub(crate) fn on_request(pause: Interrupt) -> Box<dyn Fn() + Send> {
    Box::new(move || {
        REQUESTED.store(true, Ordering::SeqCst);
        pause.pause();
    })
}

/// Whether a report was asked for; clears the request.
pub(crate) fn take_request() -> bool {
    REQUESTED.swap(false, Ordering::SeqCst)
}

/// A run that ran out of events ends cleanly when every process is finished or at `wait;`;
/// a process still waiting for a signal is worth a report.
pub(crate) fn idle_is_stuck(r: &StuckReport) -> bool {
    r.procs.iter().any(|p| p.state == ProcWait::Never)
}
