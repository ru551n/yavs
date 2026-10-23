//! Speculative two-state code: the policy every tier backend shares (docs/design/two-state.md).
//!
//! A backend may compile a logic operation on the assumption that every element is `'0'` or
//! `'1'` (std_ulogic codes 2 and 3), behind a guard; a failing guard leaves the compiled code
//! for the interpreter at the instruction (a bail). This module owns what is not backend
//! specific: the mode switch (`YAVS_SPEC01`), the per-function back-off and the counters.
//!
//! Back-off: a function whose compiled code bails sleeps for `2^k - 1` activations (the
//! interpreter runs them), then gets one retry (probation); a retry that bails doubles the
//! sleep (`k` up to [`KMAX`]), one that does not resets `k`. Results never depend on the
//! policy: both paths compute the same values, only the speed differs.

use std::cell::Cell;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering::Relaxed};

/// The longest sleep is `2^KMAX - 1` activations.
pub const KMAX: u32 = 10;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    /// No speculation: the compiled code is what it was.
    Off,
    /// Guards that bail, back-off.
    On,
    /// As `On`, and every guard that passes also checks the two-state result against the
    /// general one (a mismatch is an internal error). For tests.
    Shadow,
    /// No guards at all: an upper bound of what speculation can save, wrong on any non-01
    /// value. Measurement only.
    Oracle,
}

/// The mode of this process (`YAVS_SPEC01=on|shadow|oracle`, default off), read once.
pub fn mode() -> Mode {
    static M: OnceLock<Mode> = OnceLock::new();
    *M.get_or_init(|| match std::env::var("YAVS_SPEC01").as_deref() {
        Ok("on") => Mode::On,
        Ok("shadow") => Mode::Shadow,
        Ok("oracle") => Mode::Oracle,
        _ => Mode::Off,
    })
}

#[derive(Default)]
struct Slot {
    sleep: AtomicU32,
    k: AtomicU32,
    probation: AtomicBool,
    /// Activations seen, run compiled, bails by guard kind (`bail`).
    acts: AtomicU64,
    skipped: AtomicU64,
    bails: [AtomicU64; 2],
}

static SLOTS: OnceLock<Box<[Slot]>> = OnceLock::new();

/// Size the tables for `n` functions (first call wins; every tier passes the program's count).
pub fn init(n: usize) {
    if mode() != Mode::Off {
        SLOTS.get_or_init(|| (0..n).map(|_| Slot::default()).collect());
    }
}

thread_local! {
    static BAILED: Cell<bool> = const { Cell::new(false) };
}

/// Compiled code of function `f` bailed: `kind` 0 is the two-state guard, 1 a structural one
/// (the signal is not one compiled code reads in place).
pub fn bail(f: u32, kind: usize) {
    if let Some(s) = SLOTS.get().and_then(|s| s.get(f as usize)) {
        s.bails[kind.min(1)].fetch_add(1, Relaxed);
    }
    BAILED.set(true);
}

/// Run `f` (one activation of function `func`) unless the function sleeps: `None` then, and
/// the interpreter runs it. Updates the back-off from whether compiled code bailed.
#[inline]
pub fn gate<T>(func: u32, run: impl FnOnce() -> Option<T>) -> Option<T> {
    if mode() == Mode::Off || mode() == Mode::Oracle {
        return run();
    }
    let Some(s) = SLOTS.get().and_then(|s| s.get(func as usize)) else {
        return run();
    };
    s.acts.fetch_add(1, Relaxed);
    let sleep = s.sleep.load(Relaxed);
    if sleep > 0 {
        s.skipped.fetch_add(1, Relaxed);
        s.sleep.store(sleep - 1, Relaxed);
        if sleep == 1 {
            s.probation.store(true, Relaxed);
        }
        return None;
    }
    BAILED.set(false);
    let r = run();
    if BAILED.get() {
        let k = (s.k.load(Relaxed) + 1).min(KMAX);
        s.k.store(k, Relaxed);
        s.sleep.store((1 << k) - 1, Relaxed);
        s.probation.store(false, Relaxed);
    } else if s.probation.swap(false, Relaxed) {
        s.k.store(0, Relaxed);
    }
    r
}

/// Totals over all functions: activations, activations skipped by back-off, bails by kind,
/// and the number of functions that ever bailed.
pub fn totals() -> (u64, u64, [u64; 2], usize) {
    let mut t = (0, 0, [0; 2], 0);
    for s in SLOTS.get().map_or(&[][..], |s| &s[..]) {
        t.0 += s.acts.load(Relaxed);
        t.1 += s.skipped.load(Relaxed);
        let b = [s.bails[0].load(Relaxed), s.bails[1].load(Relaxed)];
        t.2[0] += b[0];
        t.2[1] += b[1];
        t.3 += usize::from(b[0] + b[1] > 0);
    }
    t
}

#[cfg(feature = "spec-census")]
pub mod census {
    //! Counts of logic reads by the interpreter: how many see only `'0'`/`'1'`.
    use std::sync::atomic::{AtomicU64, Ordering::Relaxed};
    pub static VEC: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
    pub static SCALAR: [AtomicU64; 2] = [AtomicU64::new(0), AtomicU64::new(0)];
    pub fn vec(is01: bool) {
        VEC[usize::from(is01)].fetch_add(1, Relaxed);
    }
    pub fn scalar(is01: bool) {
        SCALAR[usize::from(is01)].fetch_add(1, Relaxed);
    }
}

/// One line per counter set, for `YAVS_SPEC01_STATS=1` (printed by the CLI after a run).
pub fn report() -> String {
    let (acts, skipped, bails, funcs) = totals();
    #[allow(unused_mut)]
    let mut s = format!(
        "spec01: mode={:?} activations={acts} skipped_by_backoff={skipped} bails_guard={} bails_structural={} functions_that_bailed={funcs}",
        mode(),
        bails[0],
        bails[1]
    );
    #[cfg(feature = "spec-census")]
    {
        use census::*;
        let g = |a: &[AtomicU64; 2]| (a[0].load(Relaxed), a[1].load(Relaxed));
        let (v, sc) = (g(&VEC), g(&SCALAR));
        s += &format!(
            "\nspec01: census vector reads non01={} all01={} scalar reads non01={} all01={}",
            v.0, v.1, sc.0, sc.1
        );
    }
    s
}
