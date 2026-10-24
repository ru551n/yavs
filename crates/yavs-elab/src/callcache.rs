//! The results of pure elaboration-time calls in the stage cache (#539, `docs/decisions.md`
//! "Elaboration-time call cache"): one entry of kind `ecall` per closure, holding every call
//! `(callee, argument values) -> (result, reports)` that earlier elaborations of it made.

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use yavs_sem::{Db, DeclId, StageCache};

use crate::eval::Val;

/// The stage entry's kind and part.
const KIND: &str = "ecall";
const PART: &[u8] = b"v1";
/// What one entry keeps (the newest calls win): a design sweeping its generics would otherwise grow it.
const MAX_CALLS: usize = 8192;
const MAX_BYTES: usize = 4 << 20;
/// A bigger result is not worth its bytes.
const MAX_RESULT: usize = 64 << 10;

static HITS: AtomicU64 = AtomicU64::new(0);
static RUNS: AtomicU64 = AtomicU64::new(0);

/// Elaboration-time calls of this process answered from the cache, and calls that ran the
/// interpreter although they had a key (work counters, `YAVS_COUNTERS`).
pub fn stats() -> (u64, u64) {
    (HITS.load(Ordering::Relaxed), RUNS.load(Ordering::Relaxed))
}

pub(crate) fn count(hit: bool) {
    (if hit { &HITS } else { &RUNS }).fetch_add(1, Ordering::Relaxed);
}

/// A call's outcome: the value and the reports it made (printed again on a hit).
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub(crate) struct Hit {
    pub val: Val,
    pub reports: Vec<(u8, String)>,
}

/// Loaded on the first call of an elaboration, written back by `flush` when it learnt something.
#[derive(Default)]
pub(crate) enum CallCache {
    #[default]
    Unset,
    Off,
    On {
        cache: Arc<dyn StageCache>,
        /// `Db::extent` when the entry was read: a call is looked up or stored only while the
        /// `Db` has that extent (instantiation grows it).
        extent: Vec<usize>,
        /// `Db::stage_extent` at the same time: the key of the entry.
        stage: Vec<usize>,
        calls: HashMap<Vec<u8>, Hit>,
        /// New calls in order, as `(key, result bytes)`.
        new: Vec<(Vec<u8>, Vec<u8>)>,
        /// Old calls in the entry's order, kept for the next entry.
        old: Vec<(Vec<u8>, Vec<u8>)>,
    },
}

/// Whether a call of `callee` may be kept: a function declared `pure` in a design-unit-level
/// package (not in an instance, so nothing of an elaborated region is read; the caller also
/// requires that lowering found no such object). What it does at run time is checked by the
/// call itself (`CallTrace::tainted`).
pub(crate) fn cacheable(db: &Db, callee: DeclId) -> bool {
    use yavs_sem::decl::SubprogramKind;
    use yavs_sem::scope::ScopeKind::{Context, Package, PackageBody, Root};
    let d = &db.decls[callee];
    let pure = d
        .kind
        .subprogram()
        .is_some_and(|s| matches!(s.kind, SubprogramKind::Function { pure: true }));
    let mut scope = Some(d.scope);
    while let Some(s) = scope {
        if !matches!(db.scopes[s].kind, Package | PackageBody | Context | Root) {
            return false;
        }
        scope = db.scopes[s].parent;
    }
    pure
}

/// The key of a call: the callee's index in the `Db` (the extent says which `Db`) and the
/// arguments.
pub(crate) fn key(callee: DeclId, args: &[Val]) -> Option<Vec<u8>> {
    postcard::to_stdvec(&(callee, args))
        .ok()
        .filter(|k| k.len() <= MAX_RESULT)
}

impl CallCache {
    fn load(&mut self, db: &Db) {
        if !matches!(self, CallCache::Unset) {
            return;
        }
        *self = match db.stages() {
            None => CallCache::Off,
            Some(c) => {
                let mut calls = HashMap::new();
                let mut old = Vec::new();
                let entry = c.get(&db.stage_extent(), KIND, PART);
                let rows: Vec<(Vec<u8>, Vec<u8>)> = entry
                    .and_then(|b| postcard::from_bytes(&b).ok())
                    .unwrap_or_default();
                for (k, v) in rows {
                    if let Ok(h) = postcard::from_bytes::<Hit>(&v) {
                        calls.insert(k.clone(), h);
                        old.push((k, v));
                    }
                }
                CallCache::On {
                    cache: c.clone(),
                    extent: db.extent(),
                    stage: db.stage_extent(),
                    calls,
                    new: Vec::new(),
                    old,
                }
            }
        };
    }

    /// The outcome of the call with `key`, if an earlier elaboration made it.
    pub(crate) fn get(&mut self, db: &Db, key: &[u8]) -> Option<Hit> {
        self.load(db);
        match self {
            CallCache::On { extent, calls, .. } if db.extent() == *extent => {
                calls.get(key).cloned()
            }
            _ => None,
        }
    }

    pub(crate) fn put(&mut self, db: &Db, key: Vec<u8>, hit: &Hit) {
        self.load(db);
        let CallCache::On {
            extent, calls, new, ..
        } = self
        else {
            return;
        };
        if db.extent() != *extent || calls.contains_key(&key) {
            return;
        }
        let Ok(bytes) = postcard::to_stdvec(hit) else {
            return;
        };
        if bytes.len() <= MAX_RESULT {
            calls.insert(key.clone(), hit.clone());
            new.push((key, bytes));
        }
    }

    /// Write the entry when this elaboration made calls it did not know.
    // ponytail: no re-read before the write, so two runs that learn different calls at once keep
    // the later one's; the next run learns the rest again.
    pub(crate) fn flush(&mut self) {
        let CallCache::On {
            cache,
            stage,
            new,
            old,
            ..
        } = self
        else {
            return;
        };
        if new.is_empty() {
            return;
        }
        let mut rows = std::mem::take(old);
        rows.append(new);
        let mut bytes = 0;
        let mut keep = rows.len();
        for (i, (k, v)) in rows.iter().enumerate().rev() {
            bytes += k.len() + v.len();
            if bytes > MAX_BYTES || rows.len() - i > MAX_CALLS {
                break;
            }
            keep = i;
        }
        if let Ok(b) = postcard::to_stdvec(&rows[keep..]) {
            cache.put(stage, KIND, PART, &b);
        }
    }
}
