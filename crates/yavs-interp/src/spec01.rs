//! Speculative two-state code: the policy every tier backend shares (docs/design/two-state.md).
//!
//! A backend may compile a logic operation on the assumption that every element is `'0'` or
//! `'1'` (std_ulogic codes 2 and 3), behind a guard; a failing guard leaves the compiled code
//! for the interpreter at the instruction (a bail). This module owns what is not backend
//! specific: the mode switch (`YAVS_SPEC01`), the per-function back-off and the counters.
//!
//! Back-off: a function whose compiled code bails sleeps for `2^k - 1` activations (the
//! interpreter runs them), then gets one retry (probation); a retry that bails doubles the
//! sleep (`k` up to [`KMAX`]), one that does not resets `k`. A function that never bailed is
//! not gated (one load per activation). Results never depend on the
//! policy: both paths compute the same values, only the speed differs.

use std::sync::OnceLock;
use std::sync::atomic::Ordering::Relaxed;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64};

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
    /// Set by the first bail, cleared by a retry that did not bail: only a function with this
    /// set pays for the gate.
    armed: AtomicBool,
    sleep: AtomicU32,
    k: AtomicU32,
    probation: AtomicBool,
    /// Bails so far, all kinds (`gate` compares it before and after a retry).
    nbail: AtomicU32,
    /// Activations seen and skipped while armed (only with `YAVS_SPEC01_STATS`), bails by kind.
    acts: AtomicU64,
    skipped: AtomicU64,
    bails: [AtomicU64; 4],
}

static SLOTS: OnceLock<Box<[Slot]>> = OnceLock::new();
/// Activations of all functions seen by the gate (with `YAVS_SPEC01_STATS`).
static TOTAL: AtomicU64 = AtomicU64::new(0);
/// Whether the gate does anything (one load and a test when it does not).
static ON: AtomicBool = AtomicBool::new(false);
/// Whether the gate counts (`YAVS_SPEC01_STATS`).
static STATS: AtomicBool = AtomicBool::new(false);

/// Size the tables for `n` functions (first call wins; every tier passes the program's count).
pub fn init(n: usize) {
    if matches!(mode(), Mode::On | Mode::Shadow)
        && SLOTS.set((0..n).map(|_| Slot::default()).collect()).is_ok()
    {
        STATS.store(std::env::var_os("YAVS_SPEC01_STATS").is_some(), Relaxed);
        ON.store(true, Relaxed);
    }
}

/// Compiled code of function `f` bailed: `kind` 0 is the two-state guard, 1 to 3 structural
/// ones (the signal is not in the table compiled code reads, the range is not in it, the
/// destination is not memory compiled code reaches). The function sleeps for `2^k - 1`
/// activations, `k` one more than after its last bail (up to [`KMAX`]).
pub fn bail(f: u32, kind: usize) {
    let Some(s) = SLOTS.get().and_then(|s| s.get(f as usize)) else {
        return;
    };
    s.bails[kind.min(3)].fetch_add(1, Relaxed);
    s.nbail.fetch_add(1, Relaxed);
    s.armed.store(true, Relaxed);
    // A second bail in an activation that already put the function to sleep changes nothing.
    if s.sleep.load(Relaxed) == 0 {
        let k = (s.k.load(Relaxed) + 1).min(KMAX);
        s.k.store(k, Relaxed);
        s.sleep.store((1 << k) - 1, Relaxed);
        s.probation.store(false, Relaxed);
    }
}

/// Run `run` (one activation of function `func`) unless the function sleeps: `None` then, and
/// the interpreter runs it. A function that never bailed costs one load and a test. `activation`
/// is false for an entry in the middle of an activation (the interpreter hands a frame to
/// compiled code again after a jump): a sleeping function declines it without using up the
/// sleep, and its result does not decide a retry.
#[inline]
pub fn gate<T>(func: u32, activation: bool, run: impl FnOnce() -> Option<T>) -> Option<T> {
    if !ON.load(Relaxed) {
        return run();
    }
    if activation && STATS.load(Relaxed) {
        TOTAL.fetch_add(1, Relaxed);
    }
    let Some(s) = SLOTS.get().and_then(|s| s.get(func as usize)) else {
        return run();
    };
    if !s.armed.load(Relaxed) {
        return run();
    }
    gate_armed(s, activation, run)
}

#[inline(never)]
fn gate_armed<T>(s: &Slot, activation: bool, run: impl FnOnce() -> Option<T>) -> Option<T> {
    let stats = STATS.load(Relaxed) && activation;
    if !activation {
        return if s.sleep.load(Relaxed) > 0 {
            None
        } else {
            run()
        };
    }
    if stats {
        s.acts.fetch_add(1, Relaxed);
    }
    let sleep = s.sleep.load(Relaxed);
    if sleep > 0 {
        if stats {
            s.skipped.fetch_add(1, Relaxed);
        }
        s.sleep.store(sleep - 1, Relaxed);
        if sleep == 1 {
            s.probation.store(true, Relaxed);
        }
        return None;
    }
    // The retry after a sleep (or an activation after a bail that did not sleep): if it does
    // not bail, the function is healthy again.
    let before = s.nbail.load(Relaxed);
    let r = run();
    if s.nbail.load(Relaxed) == before && s.probation.swap(false, Relaxed) {
        s.k.store(0, Relaxed);
        s.armed.store(false, Relaxed);
    }
    r
}

/// Totals over all functions: activations, activations skipped by back-off, bails by kind,
/// and the number of functions that ever bailed.
pub fn totals() -> (u64, u64, [u64; 4], usize) {
    let mut t = (0, 0, [0; 4], 0);
    for s in SLOTS.get().map_or(&[][..], |s| &s[..]) {
        t.0 += s.acts.load(Relaxed);
        t.1 += s.skipped.load(Relaxed);
        let b: [u64; 4] = std::array::from_fn(|k| s.bails[k].load(Relaxed));
        for (sum, n) in t.2.iter_mut().zip(b) {
            *sum += n;
        }
        t.3 += usize::from(b.iter().sum::<u64>() > 0);
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
        "spec01: mode={:?} activations={} armed_activations={acts} skipped_by_backoff={skipped} bails_guard={} bails_structural(sig,range,addr)={},{},{} functions_that_bailed={funcs}",
        mode(),
        TOTAL.load(Relaxed),
        bails[0],
        bails[1],
        bails[2],
        bails[3]
    );
    for (i, f) in SLOTS.get().map_or(&[][..], |s| &s[..]).iter().enumerate() {
        let b: Vec<u64> = f.bails.iter().map(|x| x.load(Relaxed)).collect();
        if b.iter().sum::<u64>() >= 10 {
            s += &format!(
                "\nspec01: f{i} acts={} skipped={} bails={b:?}",
                f.acts.load(Relaxed),
                f.skipped.load(Relaxed)
            );
        }
    }
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
