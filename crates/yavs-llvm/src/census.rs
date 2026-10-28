//! Work counters: the callbacks compiled code makes into the host, by kind (`YAVS_COUNTERS`,
//! `crates/yavs/tests/perf.rs`). Counted in debug builds and with the `op-census` feature; in
//! a release build `cb` is empty. A callback is a call out of compiled code that costs about
//! as much as the instructions it replaces: fewer is the point of the native-frame work.

#[cfg(any(debug_assertions, feature = "op-census"))]
use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Clone, Copy)]
pub(crate) enum Cb {
    Step,
    Signal,
    Deopt,
    Drive,
    Wait,
    WaitOn,
    WaitSame,
    Native,
    Troot,
    Endstmt,
    Xstep,
    Lcopy,
    Host,
    Layout,
    Fast,
    Scan,
    Memo,
    Region,
    Eloop,
    /// Not a callback: an activation that skipped the table's tags (`rt::enter`, `lean`).
    Lean,
}

const NAMES: [&str; 20] = [
    "step",
    "signal",
    "deopt",
    "drive",
    "wait",
    "wait_on",
    "wait_same",
    "native",
    "troot",
    "endstmt",
    "xstep",
    "lcopy",
    "host",
    "layout",
    "fast",
    "scan",
    "memo",
    "region",
    "eloop",
    "lean",
];

#[cfg(any(debug_assertions, feature = "op-census"))]
static COUNTS: [AtomicU64; 20] = [const { AtomicU64::new(0) }; 20];

#[inline(always)]
pub(crate) fn cb(c: Cb) {
    #[cfg(any(debug_assertions, feature = "op-census"))]
    COUNTS[c as usize].fetch_add(1, Ordering::Relaxed);
    #[cfg(not(any(debug_assertions, feature = "op-census")))]
    let _ = c;
}

/// `(kind, calls)` so far in this process; `None` in a release build without `op-census`.
pub fn callback_counts() -> Option<Vec<(&'static str, u64)>> {
    #[cfg(any(debug_assertions, feature = "op-census"))]
    return Some(
        NAMES
            .iter()
            .zip(&COUNTS)
            .map(|(n, c)| (*n, c.load(Ordering::Relaxed)))
            .collect(),
    );
    #[cfg(not(any(debug_assertions, feature = "op-census")))]
    {
        let _ = NAMES;
        None
    }
}
