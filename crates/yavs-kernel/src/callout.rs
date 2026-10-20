//! Reports made by VHDL functions the kernel calls on a process's behalf (resolution functions
//! and port conversions): they run outside any process, so the interpreter hands its
//! `report`s to the kernel through this thread-local and the kernel commits them in
//! deterministic order after the update phase (#372).

use crate::process::Severity;
use std::cell::RefCell;
use std::sync::atomic::{AtomicBool, Ordering};

thread_local! {
    static CAPTURE: RefCell<Option<Vec<(Severity, String)>>> = const { RefCell::new(None) };
}

/// Set when a capture found reports: lets the kernel skip its scan of the signals otherwise.
pub(crate) static PENDING: AtomicBool = AtomicBool::new(false);

/// Run `f`, collecting what [`push`] gets meanwhile.
pub(crate) fn capture<T>(f: impl FnOnce() -> T) -> (T, Vec<(Severity, String)>) {
    let outer = CAPTURE.with(|c| c.replace(Some(Vec::new())));
    let r = f();
    let got = CAPTURE.with(|c| c.replace(outer)).unwrap_or_default();
    if !got.is_empty() {
        PENDING.store(true, Ordering::Relaxed);
    }
    (r, got)
}

/// How many reports the running capture holds so far, `None` outside one (at initialisation
/// reports are made at once): a function call that leaves it unchanged reported nothing, so a
/// caller may remember its result (R3 memo).
pub fn captured() -> Option<usize> {
    CAPTURE.with(|c| c.borrow().as_ref().map(Vec::len))
}

/// A `report` of a function the kernel is calling (see the module docs). Outside a capture,
/// e.g. at initialisation, the message comes back for the caller to report at once.
pub fn push(severity: Severity, message: String) -> Result<(), String> {
    CAPTURE.with(|c| match c.borrow_mut().as_mut() {
        Some(v) => {
            v.push((severity, message));
            Ok(())
        }
        None => Err(message),
    })
}
