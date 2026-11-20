//! Predecoded code kept across runs: the stage entry `pdec` of a cached program (`yavs-elab`
//! `sim.rs` reads it into `Program::stored_pre` and stores what [`entry`] makes at the end of a
//! run). A record holds a function's [`Pre`] as `Pre::new` made it from the code
//! `Program::code_digest` names; the entry's key names the program (`Program::identity`), the
//! build and the optimiser's switches, so everything else `Pre::new` reads (callees' memo
//! signatures and flags, case tables, natives) is the same. Only the plain form of a function
//! not run as lowered is kept: no arms, no copies.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering::Relaxed};

use yavs_diag::Slice;
use yavs_ir::{Func, FuncId, Program};

use crate::pre::Pre;

/// The stage entry kind.
pub const KIND: &str = "pdec";

/// What the entry's key holds besides the program, the build and the optimiser's switches: the
/// format of the records.
pub fn fingerprint() -> &'static str {
    "pdec 1"
}

/// Whether predecoded code is read and stored: `YAVS_PDEC=0` says no, and so does `YAVS_DUMPIR`,
/// whose dump of each function's ops comes from predecoding it.
pub fn enabled() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| {
        std::env::var_os("YAVS_DUMPIR").is_none()
            && std::env::var("YAVS_PDEC").map_or(true, |v| v != "0")
    })
}

/// `YAVS_PDEC_CHECK` (not empty and not `0`): a stored record is not used; the function is
/// predecoded and a record that differs is an internal error.
fn check() -> bool {
    static CHECK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *CHECK.get_or_init(|| std::env::var("YAVS_PDEC_CHECK").is_ok_and(|v| !v.is_empty() && v != "0"))
}

static HITS: AtomicU64 = AtomicU64::new(0);
static MADE: AtomicU64 = AtomicU64::new(0);
static OTHER: AtomicU64 = AtomicU64::new(0);

/// Work counters (`YAVS_COUNTERS`): functions whose facts came from the entry, those made that a
/// store would keep, and those made that no entry keeps (as lowered under elaboration, copies,
/// callers of copies, programs without an entry).
pub fn stats() -> (u64, u64, u64) {
    let n = |c: &AtomicU64| c.load(Relaxed);
    (n(&HITS), n(&MADE), n(&OTHER))
}

/// The records of an entry, by function id.
pub fn read(bytes: Option<&yavs_diag::Buffer>) -> yavs_ir::StoredPre {
    let recs: Vec<(u32, u64, Slice)> = bytes
        .and_then(|b| yavs_diag::sharing(b, || postcard::from_bytes(b).ok()))
        .unwrap_or_default();
    yavs_ir::StoredPre(recs.into_iter().map(|(f, d, s)| (f, (d, s))).collect())
}

/// The facts of function `id` (`f`, as `Program::func` hands it out): the stored record when it
/// is of this code, else made now and listed in `made` for [`entry`].
pub(crate) fn pre(prog: &Program, made: &Mutex<Vec<(u32, u64)>>, id: FuncId, f: &Func) -> Pre {
    let raw = prog.runs_raw(id);
    let Some((stored, d)) = prog
        .stored_pre
        .as_ref()
        .filter(|_| !raw)
        .and_then(|s| Some((s, prog.code_digest(id, f)?)))
    else {
        OTHER.fetch_add(1, Relaxed);
        return Pre::new(f, prog, raw, &[]);
    };
    let rec = stored.0.get(&id.0).filter(|(sd, _)| *sd == d);
    let hit = rec.and_then(|(_, b)| Pre::stored(b.get(), f));
    if !check()
        && let Some(p) = hit
    {
        HITS.fetch_add(1, Relaxed);
        return p;
    }
    let p = Pre::new(f, prog, false, &[]);
    match (rec, hit) {
        (Some((_, b)), Some(h)) => {
            let same = |x: &Pre| postcard::to_stdvec(x).is_ok_and(|v| v == b.get());
            assert!(
                same(&p) && same(&h),
                "internal error: the stored predecoded code of function {} (`{}`) is not the one \
                 the predecoder makes; this is a bug in yavs",
                id.0,
                f.name
            );
        }
        _ => {
            MADE.fetch_add(1, Relaxed);
            made.lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .push((id.0, d));
        }
    }
    p
}

/// The entry to store: the records `prog` was read with, those of `now` (the entry as stored now:
/// other runs may have added to it) and the ones this run made (`pre` gives a function's facts if
/// it has them), in that order; `None` when the first two hold every one this run made.
pub(crate) fn entry<'a>(
    prog: &Program,
    made: &Mutex<Vec<(u32, u64)>>,
    pre: impl Fn(FuncId) -> Option<&'a Pre>,
    now: Option<&yavs_diag::Buffer>,
) -> Option<Vec<u8>> {
    let made = std::mem::take(
        &mut *made
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner),
    );
    let now = read(now).0;
    let mut all: std::collections::BTreeMap<u32, (u64, Slice)> = prog
        .stored_pre
        .as_ref()?
        .0
        .iter()
        .map(|(&f, r)| (f, r.clone()))
        .collect();
    // (This run's records win over the stored ones of other code.)
    all.extend(now);
    let mut new = false;
    for (f, d) in made {
        if all.get(&f).is_some_and(|r| r.0 == d) {
            continue;
        }
        // (Still the code it was made from: a function promoted since got new facts.)
        let id = FuncId(f);
        let Some(p) = pre(id).filter(|p| !p.raw) else {
            continue;
        };
        if prog.func(id).and_then(|g| prog.code_digest(id, g)) != Some(d) {
            continue;
        }
        if let Ok(b) = postcard::to_stdvec(p) {
            all.insert(f, (d, b.into()));
            new = true;
        }
    }
    if !new {
        return None;
    }
    let recs: Vec<(u32, u64, Slice)> = all.into_iter().map(|(f, (d, s))| (f, d, s)).collect();
    postcard::to_stdvec(&recs).ok()
}
