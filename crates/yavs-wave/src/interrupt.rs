//! SIGINT/SIGTERM (and Ctrl-C on Windows) support.
//!
//! The first signal only calls `stop`: `yavs run` passes the kernel's `Interrupt::stop`, the
//! kernel ends between cycles, and the recorder finishes the file. A second signal exits at
//! once with status 1 (a process stuck inside one cycle never polls); the file is then readable
//! up to its last flush.

use std::sync::atomic::{AtomicBool, Ordering};

/// Install the handler once per process.
pub fn install(stop: impl Fn() + Send + 'static) -> Result<(), ctrlc::Error> {
    let seen = AtomicBool::new(false);
    ctrlc::set_handler(move || {
        if seen.swap(true, Ordering::SeqCst) {
            std::process::exit(1);
        }
        stop();
    })
}
