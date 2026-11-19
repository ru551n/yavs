//! The elaboration-time session replayed (stage entries `sreq` and `send`, `docs/decisions.md`
//! "The elaboration-time session is replayed"): a testbench that creates its verification
//! components in constant declarations makes the same calls with the same arguments in every
//! test, and they leave the same package state. Each call of the session is a step named by a
//! digest of the steps before it and of its callee and arguments (`Trail::next`); `sreq` keeps the
//! result of every step earlier runs of the closure made, `send` the package state after the last
//! step (the image of `SharedState::state_image`), by the digest of the whole session and of the
//! stored optimised copies the program took. A run whose steps are all found returns the stored
//! results without running the calls and starts the simulation from the stored state; a step
//! that is not found runs the calls logged so far on the package state first (`Session::
//! materialise`) and goes on as without a replay.

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use yavs_sem::{Db, DeclId, StageCache};

use crate::eval::Val;

const STEPS: &str = "sreq";
const ENDS: &str = "send";
/// A results entry over this size keeps only the steps of the run that writes it.
const STEPS_MAX: usize = 4 << 20;
/// End states stored per closure at most (each is the whole package state, about 3 MB for a
/// verification-component testbench): a sweep over many generics stores the first ones only.
const ENDS_MAX: u32 = 32;

static REPLAYED: AtomicU64 = AtomicU64::new(0);
static RESTORED: AtomicU64 = AtomicU64::new(0);
static MISSED: AtomicU64 = AtomicU64::new(0);
static STORED: AtomicU64 = AtomicU64::new(0);

/// Work counters (`YAVS_COUNTERS`): steps answered from `sreq`, sessions whose end state came
/// from `send`, replays that had to run the calls after all, end states stored.
pub fn stats() -> (u64, u64, u64, u64) {
    let l = |a: &AtomicU64| a.load(Ordering::Relaxed);
    (l(&REPLAYED), l(&RESTORED), l(&MISSED), l(&STORED))
}

fn flag(name: &str) -> Option<bool> {
    std::env::var(name).ok().map(|v| !v.is_empty() && v != "0")
}

/// `YAVS_SREP=0`: no replay and nothing stored.
fn off() -> bool {
    static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *OFF.get_or_init(|| flag("YAVS_SREP") == Some(false))
}

/// `YAVS_SREP_CHECK` (not empty and not `0`): every call runs; a result or end state that
/// differs from the stored one is an internal error.
pub(crate) fn check() -> bool {
    static CHECK: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *CHECK.get_or_init(|| flag("YAVS_SREP_CHECK") == Some(true))
}

/// Whether `now` read during elaboration (always 0 fs) counts as an input of a step
/// (`YAVS_SREP_NOW=0`: it makes the session impure, so it is never stored).
pub(crate) fn now_is_input() -> bool {
    static NOW: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *NOW.get_or_init(|| flag("YAVS_SREP_NOW") != Some(false))
}

#[derive(Default, PartialEq, Eq, Clone, Copy)]
enum Mode {
    /// No call yet.
    #[default]
    Idle,
    /// The logged calls have not run: the package state is the one before the first.
    Replay,
    /// The calls run.
    Run,
}

struct Entry {
    cache: Arc<dyn StageCache>,
    extent: Vec<usize>,
    results: HashMap<u64, Vec<u8>>,
    /// End states stored so far.
    ends: u32,
}

/// The steps of one session ([`crate::pkgcall::Session`]).
#[derive(Default)]
pub(crate) struct Trail {
    entry: Option<Option<Entry>>,
    mode: Mode,
    /// The digest after the last step.
    chain: u64,
    /// A digest of the stored optimised copies the session's program took (`Copies`): the
    /// state calls leave depends on the code they ran (temporaries in the shared heap).
    code: u64,
    /// This run's steps: digest and result.
    steps: Vec<(u64, Vec<u8>)>,
    /// Why this session is not stored.
    spoilt: Option<String>,
}

impl Trail {
    fn entry(&mut self, db: &Db) -> Option<&Entry> {
        self.entry
            .get_or_insert_with(|| {
                if off() {
                    return None;
                }
                // A closure's first run keeps nothing of its own.
                let cache = db.stages().filter(|c| c.closure())?.clone();
                let extent = db.stage_extent();
                let (rows, ends): (Vec<(u64, Vec<u8>)>, u32) = cache
                    .get(&extent, STEPS, &[])
                    .and_then(|b| postcard::from_bytes(&b).ok())
                    .unwrap_or_default();
                Some(Entry {
                    cache,
                    extent,
                    results: rows.into_iter().collect(),
                    ends,
                })
            })
            .as_ref()
    }

    /// The package state is stale: the logged calls have not run.
    pub(crate) fn replaying(&self) -> bool {
        self.mode == Mode::Replay
    }

    /// Whether a call that runs should be traced to be stored.
    pub(crate) fn recording(&self) -> bool {
        self.spoilt.is_none() && self.entry.as_ref().is_some_and(|e| e.is_some())
    }

    /// The digest of a step calling `f` with `args`; the first step of a session starts from
    /// `seed` (the packages and the `Db`).
    pub(crate) fn next(
        &mut self,
        db: &Db,
        seed: impl FnOnce() -> u64,
        f: DeclId,
        args: &[Val],
    ) -> u64 {
        if self.mode == Mode::Idle {
            self.chain = seed();
        }
        let mut h = std::hash::DefaultHasher::new();
        self.chain.hash(&mut h);
        // Ids appended during elaboration come in an order the generics choose (#1142).
        db.extent().hash(&mut h);
        yavs_sem::Id::index(f).hash(&mut h);
        match postcard::to_stdvec(args) {
            Ok(b) => b.hash(&mut h),
            Err(_) => self.spoil("an argument does not serialise"),
        }
        let d = h.finish();
        if self.mode == Mode::Idle {
            let found = !check() && self.entry(db).is_some_and(|e| e.results.contains_key(&d));
            self.mode = if found { Mode::Replay } else { Mode::Run };
        }
        d
    }

    /// The stored result of step `d`, while replaying.
    pub(crate) fn stored(&self, d: u64) -> Option<Val> {
        let e = self.entry.as_ref()?.as_ref()?;
        postcard::from_bytes(e.results.get(&d)?).ok()
    }

    /// Step `d` gave `v` (run, answered from the call cache, or replayed).
    pub(crate) fn took(&mut self, d: u64, v: &Val, replayed: bool) {
        self.chain = d;
        if replayed {
            REPLAYED.fetch_add(1, Ordering::Relaxed);
        }
        let Ok(b) = postcard::to_stdvec(v) else {
            return self.spoil("a result does not serialise");
        };
        if check()
            && let Some(Some(e)) = &self.entry
            && let Some(s) = e.results.get(&d)
        {
            assert!(
                *s == b,
                "internal error: the stored result of an elaboration-time call is not the one \
                 the call returns; this is a bug in yavs"
            );
        }
        if self.spoilt.is_none() {
            self.steps.push((d, b));
        }
    }

    /// The session cannot be stored.
    pub(crate) fn spoil(&mut self, why: impl Into<String>) {
        if self.spoilt.is_none() {
            self.spoilt = Some(why.into());
        }
    }

    /// The calls run from here on.
    pub(crate) fn run(&mut self) {
        if self.mode == Mode::Replay {
            MISSED.fetch_add(1, Ordering::Relaxed);
        }
        self.mode = Mode::Run;
    }

    /// The program took `n` stored optimised copies at a step with digest `d`.
    pub(crate) fn code(&mut self, d: u64, n: usize) {
        if n > 0 {
            let mut h = std::hash::DefaultHasher::new();
            (self.code, d, n).hash(&mut h);
            self.code = h.finish();
        }
    }

    /// The name of the end state: the steps, the stored copies taken, and `prog`, a digest of
    /// everything the session's program was asked for (a state is valid for its program only:
    /// heap objects hold type ids, and package constants read are no steps).
    fn end_part(&self, prog: u64) -> [u8; 24] {
        let mut p = [0; 24];
        p[..8].copy_from_slice(&self.chain.to_le_bytes());
        p[8..16].copy_from_slice(&self.code.to_le_bytes());
        p[16..].copy_from_slice(&prog.to_le_bytes());
        p
    }

    /// The stored state after the session, when every step was replayed.
    pub(crate) fn end(&self, prog: u64) -> Option<yavs_diag::Buffer> {
        if self.mode != Mode::Replay || self.steps.is_empty() {
            return None;
        }
        let e = self.entry.as_ref()?.as_ref()?;
        e.cache.get(&e.extent, ENDS, &self.end_part(prog))
    }

    /// The state after the session was restored: it is the calls' from here on.
    pub(crate) fn restored(&mut self) {
        RESTORED.fetch_add(1, Ordering::Relaxed);
        self.mode = Mode::Run;
    }

    /// Store the steps the entry lacks and the state after them (`image` makes it), unless
    /// something spoilt the session.
    pub(crate) fn store(&mut self, prog: u64, image: impl FnOnce() -> Result<Vec<u8>, String>) {
        let Some(Some(e)) = &self.entry else { return };
        if self.mode != Mode::Run || self.steps.is_empty() {
            return;
        }
        if let Some(why) = &self.spoilt {
            e.cache.note(&format!("no session replay: {why}"));
            return;
        }
        let part = self.end_part(prog);
        let had = e.cache.get(&e.extent, ENDS, &part);
        let mut ends = e.ends;
        if had.is_none() && ends >= ENDS_MAX {
            e.cache
                .note("no session replay: the closure holds the most end states");
            return;
        }
        if had.is_none() || check() {
            match image() {
                Ok(b) => {
                    if let Some(h) = &had {
                        assert!(
                            h[..] == b[..],
                            "internal error: the stored package state after the elaboration-time \
                             calls is not the one they leave; this is a bug in yavs"
                        );
                    } else {
                        e.cache.put(&e.extent, ENDS, &part, &b);
                        STORED.fetch_add(1, Ordering::Relaxed);
                        ends += 1;
                    }
                }
                Err(why) => {
                    e.cache.note(&format!("no session replay: {why}"));
                    return;
                }
            }
        }
        if ends == e.ends && self.steps.iter().all(|(d, _)| e.results.contains_key(d)) {
            return;
        }
        let mut all = e.results.clone();
        let size = |m: &HashMap<u64, Vec<u8>>| m.values().map(|v| v.len() + 8).sum::<usize>();
        if size(&all) > STEPS_MAX {
            all.clear();
        }
        all.extend(self.steps.iter().cloned());
        let mut rows: Vec<(u64, Vec<u8>)> = all.into_iter().collect();
        rows.sort_unstable_by_key(|r| r.0);
        if let Ok(b) = postcard::to_stdvec(&(rows, ends)) {
            e.cache.put(&e.extent, STEPS, &[], &b);
        }
    }
}
