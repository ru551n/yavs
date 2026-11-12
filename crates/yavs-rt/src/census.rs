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
    /// `yavs_rt_gather`, `yavs_rt_scatter`: a whole record read or drive with no image.
    Record,
    /// `yavs_rt_planes`: a signal read from the kernel's planes out of line.
    Planes,
    /// Not a callback: an activation that skipped the table's tags (`rt::enter`, `lean`).
    Lean,
    /// Not a callback: an activation entered through the tier's own entry (`rt::direct_entry`).
    Direct,
    /// Cranelift's cold ways (`yavs_rt_rare`): the codes of a short `SigRead`, the native
    /// call of an in-place `ieee` op.
    RareCodes,
    RareNative,
    /// `yavs_rt_steps`: the instructions of a block compiled code was told never ran.
    Steps,
    /// `yavs_rt_drive_src`: a whole value driven from where it is, with no waveform.
    DriveSrc,
    /// `yavs_rt_cat`: a concatenation of bytes.
    Cat,
    /// `yavs_rt_layout_into`: the words of a layout from a run-time descriptor.
    LayoutInto,
    /// `yavs_rt_free`: a deallocation from a native frame.
    Free,
    /// `yavs_rt_scatter_cmp`: a whole record drive that missed compiled code's compare.
    RecordCmp,
    /// `yavs_rt_scatter_part`: a zero-delay drive of a part of a record signal.
    RecordPart,
}

const N: usize = 32;

/// Whether this build counts (the generators emit the calls of `yavs_rt_rare` only then).
pub const COUNTING: bool = cfg!(any(debug_assertions, feature = "op-census"));

const NAMES: [&str; N] = [
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
    "record",
    "planes",
    "lean",
    "direct",
    "rare_codes",
    "rare_native",
    "steps",
    "drive_src",
    "cat",
    "layout_into",
    "free",
    "record_cmp",
    "record_part",
];

#[cfg(any(debug_assertions, feature = "op-census"))]
static COUNTS: [AtomicU64; N] = [const { AtomicU64::new(0) }; N];

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
