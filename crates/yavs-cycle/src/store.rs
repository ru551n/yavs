//! The analysis cache: the selection of a design's region (checker, selection, lowering) in the
//! cycle store of the work library, so that a warm run skips them and the value passes of
//! elaboration. A hit must be what the analysis would give: the key holds everything it depends
//! on (AGENTS.md "The analysis cache").

use std::path::Path;
use std::sync::{Mutex, PoisonError};

use yavs_cache::{Key, KeyBuilder, Store};
use yavs_diag::Diagnostic;
use yavs_elab::{Elaborated, GenericReads, ValuesHook, watch};
use yavs_sem::Db;

use crate::region::{self, Selection};

/// The entry kinds: the selection, and the index of the generics the value passes and the
/// selection read besides the walk's.
const KIND: &str = "cyc3";
const INDEX: &str = "cyc3i";
/// Of the payload's encoding: changed with any type of `Selection`.
const FORMAT: &str = "1";
/// Environment variables that change nothing the analysis gives (the test runner's token server
/// differs per invocation; the others print or count).
const ENV_SKIP: &[&str] = &[
    "YAVS_CYCLE_CACHE_CHECK",
    "YAVS_CYCLE_STATS",
    "YAVS_STD_CACHE",
    "YAVS_LOG",
    "YAVS_TOKENS",
    "YAVS_CONSOLE_HOOK",
    "YAVS_COUNTERS",
];

/// `check::Reason` as stored: its code, a `&'static str`, as text.
#[derive(serde::Serialize, serde::Deserialize)]
struct ReasonData {
    code: String,
    span: yavs_diag::Span,
    text: String,
}

impl serde::Serialize for crate::check::Reason {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        ReasonData {
            code: self.code.to_string(),
            span: self.span,
            text: self.text.clone(),
        }
        .serialize(s)
    }
}

impl<'de> serde::Deserialize<'de> for crate::check::Reason {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let r = ReasonData::deserialize(d)?;
        use crate::codes::*;
        let code = [
            LEFT_OUT,
            NOT_WITH,
            REPORT_FILE,
            WAIT,
            DELAY,
            HISTORY,
            NOT_TWO_STATE,
            TESTBENCH,
            GUARDED,
            FORCE,
            CALL,
            HIGH_Z,
            LATCH,
            STATE,
            SENSITIVITY,
            LOOP,
            SHAPE,
            SHARED,
            CLOCK_MADE,
            ANALYSIS,
            HISTORY_READ,
        ]
        .into_iter()
        .find(|c| *c == r.code)
        .ok_or_else(|| serde::de::Error::custom("an unknown code"))?;
        Ok(crate::check::Reason {
            code,
            span: r.span,
            text: r.text,
        })
    }
}

struct Setup {
    store: Store,
    /// What the command line gave that the analysis depends on, besides the generics.
    part: Vec<u8>,
    /// The `-g` overrides, in order (the last of a name wins).
    overrides: Vec<(String, String)>,
    check: bool,
}

/// What the hook found, for `select`.
enum Found {
    /// The selection from the cache, its payload, the generics its index named and its key.
    Hit(Box<Selection>, Vec<u8>, u64, Key),
    /// Not in the cache: the key's parts, to store it under.
    Miss(Vec<u8>, Key),
}

// ponytail: one design per process, as the compilation cache of the region's code.
static SETUP: Mutex<Option<Setup>> = Mutex::new(None);
static FOUND: Mutex<Option<Found>> = Mutex::new(None);
/// The key of the selection the region is built from, when it is cached: the region's machine
/// code is kept under it too (`code_get`), whether the code checks the entry.
static CODE: Mutex<Option<(Store, Key, bool)>> = Mutex::new(None);

/// The machine code of the region's functions, `(alignment, bytes)` in order, for a region of
/// the target `isa` (its triple and flags): kept whole under the selection's key, so a hit needs
/// no function body built.
pub(crate) type Blob = Vec<(u64, Vec<u8>)>;

fn code_key(k: &Key, isa: &str) -> Key {
    let mut b = KeyBuilder::new("cyc3c");
    b.text(&k.hex()).text(cranelift_codegen::VERSION).text(isa);
    b.finish()
}

/// The region's code from the cache, and whether a new compile must be compared with it
/// (`YAVS_CYCLE_CACHE_CHECK`).
pub(crate) fn code_get(isa: &str) -> (Option<Blob>, bool) {
    let c = CODE.lock().unwrap_or_else(PoisonError::into_inner);
    let Some((s, k, check)) = c.as_ref() else {
        return (None, false);
    };
    let b = (s.get("cyc3c", &code_key(k, isa))).and_then(|e| postcard::from_bytes(&e).ok());
    (b, *check)
}

/// Keep the region's code (nothing when the selection is not cached).
pub(crate) fn code_put(isa: &str, b: &Blob) {
    let c = CODE.lock().unwrap_or_else(PoisonError::into_inner);
    if let Some((s, k, _)) = c.as_ref()
        && let Ok(p) = postcard::to_allocvec(b)
    {
        s.put("cyc3c", &code_key(k, isa), &p);
    }
}

/// Keep the analysis in the cycle store of library directory `lib` (`None`: no cache). `part`
/// names what the command line asked that the analysis depends on, `overrides` the `-g` values.
/// The hook goes into `ElabConfig::values`: on a hit elaboration leaves its value passes out.
pub fn analysis_cache(
    lib: Option<&Path>,
    part: Vec<u8>,
    overrides: Vec<(String, String)>,
) -> Option<ValuesHook> {
    *FOUND.lock().unwrap_or_else(PoisonError::into_inner) = None;
    let setup = lib.map(|l| Setup {
        store: Store::cycle_of_library(l),
        part,
        overrides,
        check: std::env::var_os("YAVS_CYCLE_CACHE_CHECK").is_some_and(|v| v == "1"),
    });
    let on = setup.is_some();
    *SETUP.lock().unwrap_or_else(PoisonError::into_inner) = setup;
    // A design whose elaboration reads a file, the environment, a clock or foreign code is not
    // kept: none of that is in the key.
    yavs_interp::watch_outside(on);
    on.then(|| ValuesHook(std::sync::Arc::new(look)))
}

/// Everything the analysis depends on but the generics: the yavs build (in every key), the
/// analysis of the closure (its files, their hashes, the options) and what elaboration added to
/// the `Db`, the source files in order (spans name them by number), the command line, the
/// environment variables of yavs.
fn base(db: &Db, s: &Setup) -> Option<Vec<u8>> {
    let id = db
        .stages()?
        .identity(&db.stage_extent(), KIND, FORMAT.as_bytes())?;
    let mut b = Vec::new();
    let mut put = |x: &[u8]| {
        b.extend((x.len() as u64).to_le_bytes());
        b.extend(x);
    };
    put(id.as_bytes());
    for i in 0..db.sources.len() {
        put(db.sources.get(yavs_diag::FileId(i as u32)).path.as_bytes());
    }
    put(&s.part);
    let mut env: Vec<(String, String)> = std::env::vars_os()
        .filter_map(|(k, v)| Some((k.into_string().ok()?, v.to_string_lossy().into_owned())))
        .filter(|(k, _)| k.starts_with("YAVS_") && !ENV_SKIP.contains(&k.as_str()))
        .collect();
    env.sort();
    for (k, v) in env {
        put(k.as_bytes());
        put(v.as_bytes());
    }
    Some(b)
}

/// The key of `base` with the values of the generics of `bits` (of `reads.decls`).
fn key(
    kind: &str,
    base: &[u8],
    db: &Db,
    reads: &GenericReads,
    ov: &[(String, String)],
    bits: u64,
) -> Key {
    let mut k = KeyBuilder::new(kind);
    k.part(base).text(&reads.decls.len().to_string());
    for (i, &d) in reads.decls.iter().enumerate() {
        if i >= watch::MAX || bits >> i & 1 == 1 {
            let name = db.decls[d].name.key().as_str().to_string();
            let v = (ov.iter().rev()).find(|(n, _)| yavs_lib::fold(n) == name);
            k.text(&name)
                .text(v.map_or("\0default", |(_, v)| v.as_str()));
        }
    }
    k.finish()
}

fn stat(what: impl FnOnce() -> String) {
    if std::env::var_os("YAVS_CYCLE_STATS").is_some() {
        eprintln!("cycle stats: {}", what());
    }
}

/// `ElabConfig::values`: whether the analysis is in the cache (`true`: leave the value passes out).
fn look(db: &Db, reads: &GenericReads) -> bool {
    let setup = SETUP.lock().unwrap_or_else(PoisonError::into_inner);
    let mut found = FOUND.lock().unwrap_or_else(PoisonError::into_inner);
    *found = None;
    let Some(s) = setup.as_ref() else {
        return false;
    };
    if yavs_interp::outside_calls() > 0 {
        stat(|| {
            "analysis not cached: elaboration read a file, the environment or foreign code".into()
        });
        return false;
    }
    // The selection writes the file of `YAVS_CYCLE_SEEDS` (developers): a hit would not.
    if std::env::var_os("YAVS_CYCLE_SEEDS").is_some() {
        return false;
    }
    let Some(b) = base(db, s) else {
        return false;
    };
    let k1 = key(INDEX, &b, db, reads, &s.overrides, reads.walk);
    let hit = (s.store.get(INDEX, &k1))
        .and_then(|ix| Some(u64::from_le_bytes(ix.get(..8)?.try_into().ok()?)))
        .and_then(|extra| {
            let k2 = key(KIND, &b, db, reads, &s.overrides, reads.walk | extra);
            let p = s.store.get(KIND, &k2)?;
            let sel: Selection = postcard::from_bytes(&p).ok()?;
            Some(Found::Hit(Box::new(sel), p.to_vec(), extra, k2))
        });
    let skip = hit.is_some() && !s.check;
    *found = Some(hit.unwrap_or(Found::Miss(b, k1)));
    skip
}

/// The selection of `e`: from the cache when the hook found it, else made (and stored).
pub(crate) fn select(db: &Db, e: &Elaborated) -> Result<Selection, Vec<Diagnostic>> {
    let found = FOUND.lock().unwrap_or_else(PoisonError::into_inner).take();
    // From here on nothing elaborates: natives need no test.
    yavs_interp::watch_outside(false);
    *CODE.lock().unwrap_or_else(PoisonError::into_inner) = None;
    let code = |k: &Key| {
        let setup = SETUP.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(s) = setup.as_ref() {
            *CODE.lock().unwrap_or_else(PoisonError::into_inner) =
                Some((s.store.clone(), *k, s.check));
        }
    };
    if e.generic_reads.skipped {
        return match found {
            Some(Found::Hit(sel, _, _, k2)) => {
                stat(|| "analysis from the cache".into());
                code(&k2);
                if SETUP
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .as_ref()
                    .is_some_and(|s| s.check)
                {
                    watch::resume();
                }
                Ok(*sel)
            }
            _ => Err(ice(
                "the value passes were left out without a cached analysis",
            )),
        };
    }
    watch::resume();
    let sel = region::select(db, e);
    let read = watch::reads();
    // `YAVS_CYCLE_CACHE_CHECK`: the region's code is kept under this key, so building the region
    // must read no generic either (`built`).
    if (SETUP
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref())
    .is_some_and(|s| s.check)
    {
        watch::resume();
    } else {
        watch::stop();
    }
    let reads = &e.generic_reads;
    let extra = (reads.values | read) & !reads.walk;
    let setup = SETUP.lock().unwrap_or_else(PoisonError::into_inner);
    match (found, setup.as_ref()) {
        (Some(Found::Hit(_, bytes, x, k2)), Some(_)) => {
            let now = postcard::to_allocvec(&sel).map_err(|m| ice(&m.to_string()))?;
            if now != bytes || x != extra {
                return Err(ice(
                    "YAVS_CYCLE_CACHE_CHECK: the analysis in the cache differs from the one made now",
                ));
            }
            stat(|| "analysis from the cache checked: the same".into());
            drop(setup);
            code(&k2);
        }
        (Some(Found::Miss(b, k1)), Some(s)) => {
            if yavs_interp::outside_calls() > 0 || reads.reported {
                stat(|| "analysis not cached: the value passes read outside or reported".into());
            } else if let Ok(p) = postcard::to_allocvec(&sel) {
                let k2 = key(KIND, &b, db, reads, &s.overrides, reads.walk | extra);
                s.store.put(KIND, &k2, &p);
                s.store.put(INDEX, &k1, &extra.to_le_bytes());
                *CODE.lock().unwrap_or_else(PoisonError::into_inner) =
                    Some((s.store.clone(), k2, s.check));
                stat(|| {
                    let names: Vec<String> = (reads.decls.iter().enumerate())
                        .filter(|(i, _)| *i >= watch::MAX || (reads.walk | extra) >> i & 1 == 1)
                        .map(|(_, &d)| db.decls[d].name.to_string())
                        .collect();
                    format!(
                        "analysis stored ({} bytes), its key holds the generics {}",
                        p.len(),
                        if names.is_empty() {
                            "(none)".into()
                        } else {
                            names.join(", ")
                        }
                    )
                });
            }
        }
        _ => {}
    }
    Ok(sel)
}

fn ice(m: &str) -> Vec<Diagnostic> {
    vec![
        Diagnostic::error("E9999", format!("cycle mode's analysis cache: {m}"))
            .with_note("this is a bug in yavs"),
    ]
}

/// After the region is built: under `YAVS_CYCLE_CACHE_CHECK` an error when the build read a
/// top-level generic, which the key of the region's code would not hold.
pub(crate) fn built() -> Result<(), Vec<Diagnostic>> {
    let check = (SETUP
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .as_ref())
    .is_some_and(|s| s.check);
    if !check {
        return Ok(());
    }
    let read = watch::reads();
    watch::stop();
    if read != 0 {
        return Err(ice(
            "YAVS_CYCLE_CACHE_CHECK: building the region read a top-level generic",
        ));
    }
    Ok(())
}
