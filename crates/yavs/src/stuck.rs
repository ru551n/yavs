//! Hang reports of `yavs run`: the SIGUSR1 request and the end-of-run decision.

use std::sync::atomic::{AtomicBool, Ordering};

use yavs_diag::Diagnostic;
use yavs_kernel::{Interrupt, ProcWait, StuckReport};

/// A report was asked for (SIGUSR1) since the last `take_request`.
static REQUESTED: AtomicBool = AtomicBool::new(false);

/// Print a report of the running simulation on SIGUSR1: a thread of this module sets a flag and
/// pauses the kernel between two cycles, `yavs run` reports and continues. Nothing on Windows.
#[cfg(unix)]
pub(crate) fn on_signal(pause: Interrupt) -> Result<(), Box<Diagnostic>> {
    use signal_hook::{consts::SIGUSR1, iterator::Signals};
    let fail = |e: std::io::Error| {
        Box::new(
            Diagnostic::warning("W0116", "cannot install the SIGUSR1 handler for hang reports")
                .with_note(e.to_string()),
        )
    };
    let mut signals = Signals::new([SIGUSR1]).map_err(fail)?;
    std::thread::Builder::new()
        .name("yavs-sigusr1".into())
        .spawn(move || {
            for _ in signals.forever() {
                REQUESTED.store(true, Ordering::SeqCst);
                pause.pause();
            }
        })
        .map(drop)
        .map_err(fail)
}

#[cfg(not(unix))]
pub(crate) fn on_signal(_pause: Interrupt) -> Result<(), Box<Diagnostic>> {
    Ok(())
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
