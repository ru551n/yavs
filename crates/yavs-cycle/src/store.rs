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
/// At most this many analyses are kept per design (K1): a test bench whose every test case reads
/// generics of its own (a case name) would otherwise write an analysis and the region's code,
/// megabytes each, per test case, and push the code its test cases share out of the store.
const PER_DESIGN: usize = 16;
/// Of the payload's encoding: changed with any type of `Selection` or `Built`.
const FORMAT: &str = "12";
/// The payloads of `cyc3` and `cyc3c` entries, by a hash of their bytes: an entry holds the hex
/// of its payload's key, so test cases whose analyses come out the same keep it once.
const PAYLOAD: &str = "cyc3p";

/// Keep `bytes` as the entry `key` of `kind`: the payload first, then the entry naming it.
fn put_shared(s: &Store, kind: &str, key: &Key, bytes: &[u8]) {
    let mut b = KeyBuilder::new(PAYLOAD);
    b.part(bytes);
    let p = b.finish();
    s.put(PAYLOAD, &p, bytes);
    s.put(kind, key, p.hex().as_bytes());
}

/// The payload of entry `key` of `kind`; a missing or damaged entry or payload is a miss.
fn get_shared(s: &Store, kind: &str, key: &Key) -> Option<yavs_cache::Entry> {
    let e = s.get(kind, key)?;
    let p = Key::from_hex(std::str::from_utf8(&e).ok()?)?;
    s.get(PAYLOAD, &p)
}
/// `YAVS_CYCLE_CACHE_MIB`: the bytes the cycle store of a library keeps, a whole number of MiB of
/// at least 1; unset or empty [`yavs_cache::CYCLE_MAX_BYTES`], another value an error.
pub(crate) fn store_bytes() -> Result<u64, String> {
    let v = std::env::var_os("YAVS_CYCLE_CACHE_MIB").unwrap_or_default();
    let v = v.to_string_lossy();
    if v.is_empty() {
        return Ok(yavs_cache::CYCLE_MAX_BYTES);
    }
    v.parse::<u64>()
        .ok()
        .filter(|&n| n > 0)
        .and_then(|n| n.checked_mul(1 << 20))
        .ok_or_else(|| format!("YAVS_CYCLE_CACHE_MIB takes a whole number of MiB of at least 1, such as `4096`, not `{v}`"))
}

/// The cycle store of library directory `lib`, kept to [`store_bytes`] (the default on a bad
/// value: `plan` reports it).
pub(crate) fn cycle_store(lib: &Path) -> Store {
    Store::cycle_of_library(lib)
        .with_max_bytes(store_bytes().unwrap_or(yavs_cache::CYCLE_MAX_BYTES))
}

/// Environment variables that change nothing the analysis gives (the test runner's token server
/// differs per invocation; the others print, count, size the store or set the threads the code
/// is made on).
const ENV_SKIP: &[&str] = &[
    "YAVS_CYCLE_CACHE_CHECK",
    "YAVS_CYCLE_CACHE_MIB",
    "YAVS_CYCLE_STATS",
    "YAVS_CYCLE_THREADS",
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
    also: Option<(yavs_diag::Span, String)>,
}

impl serde::Serialize for crate::check::Reason {
    fn serialize<S: serde::Serializer>(&self, s: S) -> Result<S::Ok, S::Error> {
        ReasonData {
            code: self.code.to_string(),
            span: self.span,
            text: self.text.clone(),
            also: self.also.clone(),
        }
        .serialize(s)
    }
}

impl<'de> serde::Deserialize<'de> for crate::check::Reason {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        let r = ReasonData::deserialize(d)?;
        let code = (crate::codes::REASONS.iter())
            .map(|c| c.0)
            .find(|c| *c == r.code)
            .ok_or_else(|| serde::de::Error::custom("an unknown code"))?;
        Ok(crate::check::Reason {
            code,
            span: r.span,
            text: r.text,
            also: r.also,
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
    /// A hit may give the built region whole (`--cycle`: no report, no errors need the
    /// selection).
    fast: bool,
}

/// What the hook found, for `select`.
enum Found {
    /// The selection from the cache, its payload, the generics its index named and its key.
    Hit(Box<Selection>, Vec<u8>, u64, Key),
    /// The region built, with its machine code (`fast`).
    Fast(Box<Fast>),
    /// Not in the cache: the key's parts, to store it under, and the files the walk read.
    Miss(Vec<u8>, Key, usize),
}

/// A region from the cache: what `build` made, its check sites (their encoding) and the machine
/// code of its functions.
pub(crate) type Fast = (crate::Built, Vec<u8>, Blob);

/// The entry of a built region: its parts, each with its length before it; the machine code as
/// bytes, not as a sequence (postcard decodes those a byte at a time).
fn encode(b: &crate::Built, blob: &Blob) -> Option<Vec<u8>> {
    let mut v = Vec::new();
    let mut put = |x: &[u8]| {
        v.extend((x.len() as u64).to_le_bytes());
        v.extend(x);
    };
    put(&postcard::to_allocvec(b).ok()?);
    put(&postcard::to_allocvec(&b.checks).ok()?);
    put(&(blob.len() as u64).to_le_bytes());
    for (align, code) in blob {
        put(&align.to_le_bytes());
        put(code);
    }
    Some(v)
}

fn decode(mut e: &[u8]) -> Option<Fast> {
    let mut take = || -> Option<&[u8]> {
        let n = u64::from_le_bytes(e.get(..8)?.try_into().ok()?) as usize;
        let x = e.get(8..8usize.checked_add(n)?)?;
        e = &e[8 + n..];
        Some(x)
    };
    let word = |x: &[u8]| Some(u64::from_le_bytes(x.try_into().ok()?));
    let b: crate::Built = postcard::from_bytes(take()?).ok()?;
    let sites = take()?.to_vec();
    let n = word(take()?)?;
    let mut blob = Vec::new();
    for _ in 0..n {
        let align = word(take()?)?;
        blob.push((align, take()?.to_vec()));
    }
    Some((b, sites, blob))
}

/// What the analysis gives `plan`: the selection, or on a hit of `--cycle` the region built.
pub(crate) enum Got {
    Sel(Box<Selection>),
    Built(Box<Fast>),
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

/// The region built from the selection of key `k` and its machine code ([`encode`]), for the
/// code's target.
const BUILT: &str = "cyc3c";

fn code_key(k: &Key) -> Option<Key> {
    let mut b = KeyBuilder::new(BUILT);
    b.text(&k.hex())
        .text(cranelift_codegen::VERSION)
        .text(&crate::codegen::isa_id()?);
    Some(b.finish())
}

/// The region built from the selection of `k`, whole and fitting its code.
fn fast_get(s: &Store, k: &Key) -> Option<Box<Fast>> {
    let e = get_shared(s, BUILT, &code_key(k)?)?;
    let f = decode(&e)?;
    (f.2.len() == f.0.code.functions()).then(|| Box::new(f))
}

/// The region's code from the cache, and whether a new compile must be compared with it
/// (`YAVS_CYCLE_CACHE_CHECK`).
pub(crate) fn code_get() -> (Option<Blob>, bool) {
    let c = CODE.lock().unwrap_or_else(PoisonError::into_inner);
    let Some((s, k, check)) = c.as_ref() else {
        return (None, false);
    };
    (fast_get(s, k).map(|f| f.2), *check)
}

/// Keep the region `b` with the machine code `made` compiled now (nothing when the selection is
/// not cached, or when the code took the cached machine code). `YAVS_CYCLE_CACHE_CHECK`: what is
/// in the cache must be what was built and compiled now.
pub(crate) fn keep_built(b: &crate::Built, made: Option<Blob>) -> Result<(), String> {
    let c = CODE.lock().unwrap_or_else(PoisonError::into_inner);
    let Some((s, k)) = c.as_ref().map(|c| (&c.0, &c.1)) else {
        return Ok(());
    };
    let check = c.as_ref().is_some_and(|c| c.2);
    let Some(key) = code_key(k) else {
        return Ok(());
    };
    let now = made.as_ref().and_then(|m| encode(b, m));
    match (get_shared(s, BUILT, &key), now) {
        (Some(e), now) if check => {
            if now.as_deref() != Some(&e[..]) {
                return Err(
                    "YAVS_CYCLE_CACHE_CHECK: the region in the cache differs from the one built now"
                        .into(),
                );
            }
            stat(|| "region from the cache checked: the same".into());
        }
        (None, Some(p)) => {
            put_shared(s, BUILT, &key, &p);
            stat(|| format!("region stored ({} bytes)", p.len()));
        }
        _ => {}
    }
    Ok(())
}

/// Keep the analysis in the cycle store of library directory `lib` (`None`: no cache). `part`
/// names what the command line asked that the analysis depends on, `overrides` the `-g` values.
/// The hook goes into `ElabConfig::values`: on a hit elaboration leaves its value passes out.
pub fn analysis_cache(
    lib: Option<&Path>,
    part: Vec<u8>,
    overrides: Vec<(String, String)>,
    run: bool,
) -> Option<ValuesHook> {
    *FOUND.lock().unwrap_or_else(PoisonError::into_inner) = None;
    let setup = lib.map(|l| Setup {
        store: cycle_store(l),
        part,
        overrides,
        check: std::env::var_os("YAVS_CYCLE_CACHE_CHECK").is_some_and(|v| v == "1"),
        // `YAVS_CYCLE_DISASM` prints the code it compiles.
        fast: run && std::env::var_os("YAVS_CYCLE_DISASM").is_none(),
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
/// environment variables of yavs, and the text files the elaboration read (`files`: name as
/// given and bytes, with the working directory they are found from).
fn base(db: &Db, s: &Setup, files: &[(Vec<u8>, Option<Vec<u8>>)]) -> Option<Vec<u8>> {
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
    if !files.is_empty() {
        put(std::env::current_dir().ok()?.as_os_str().as_encoded_bytes());
        for (name, bytes) in files {
            put(name);
            match bytes {
                Some(x) => put(x),
                None => put(b"\0no such file"),
            }
        }
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
    // Text files read from disk are in the key with their bytes.
    if yavs_interp::outside_calls() > yavs_interp::file_calls() {
        stat(|| {
            "analysis not cached: elaboration wrote a file or read the environment, the clock or \
             foreign code"
                .into()
        });
        return false;
    }
    let files = yavs_interp::files_read();
    // The selection writes the file of `YAVS_CYCLE_SEEDS` (developers): a hit would not.
    if std::env::var_os("YAVS_CYCLE_SEEDS").is_some() {
        return false;
    }
    let Some(b) = base(db, s, &files) else {
        return false;
    };
    let k1 = key(INDEX, &b, db, reads, &s.overrides, reads.walk);
    let hit = (s.store.get(INDEX, &k1))
        .and_then(|ix| Some(u64::from_le_bytes(ix.get(..8)?.try_into().ok()?)))
        .and_then(|extra| {
            let k2 = key(KIND, &b, db, reads, &s.overrides, reads.walk | extra);
            // The region built whole; else the selection, to build it from.
            if let Some(f) = (s.fast && !s.check)
                .then(|| fast_get(&s.store, &k2))
                .flatten()
            {
                return Some(Found::Fast(f));
            }
            let p = get_shared(&s.store, KIND, &k2)?;
            let sel: Selection = postcard::from_bytes(&p).ok()?;
            Some(Found::Hit(Box::new(sel), p.to_vec(), extra, k2))
        });
    let skip = hit.is_some() && !s.check;
    *found = Some(hit.unwrap_or(Found::Miss(b, k1, files.len())));
    skip
}

/// The selection of `e`: from the cache when the hook found it, else made (and stored); on a hit
/// of `--cycle` the region built.
pub(crate) fn select(db: &Db, e: &Elaborated) -> Result<Got, Vec<Diagnostic>> {
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
            Some(Found::Fast(f)) => {
                stat(|| "analysis from the cache".into());
                stat(|| "region from the cache".into());
                Ok(Got::Built(f))
            }
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
                Ok(Got::Sel(sel))
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
        (Some(Found::Miss(b, k1, files)), Some(s)) => {
            let k2 = key(KIND, &b, db, reads, &s.overrides, reads.walk | extra);
            // Files read after the walk are not in the key.
            if yavs_interp::outside_calls() > yavs_interp::file_calls()
                || yavs_interp::files_read_count() != files
                || reads.reported
            {
                stat(|| "analysis not cached: the value passes read outside or reported".into());
            } else if s.store.sightings(KIND, &k1, &k2) > PER_DESIGN {
                stat(|| {
                    format!("analysis not cached: {PER_DESIGN} analyses of this design are kept")
                });
            } else if let Ok(p) = postcard::to_allocvec(&sel) {
                put_shared(&s.store, KIND, &k2, &p);
                s.store.put(INDEX, &k1, &extra.to_le_bytes());
                *CODE.lock().unwrap_or_else(PoisonError::into_inner) =
                    Some((s.store.clone(), k2, s.check));
                stat(|| {
                    // Each generic with the first part that read it.
                    let names: Vec<String> = (reads.decls.iter().enumerate())
                        .filter(|(i, _)| *i >= watch::MAX || (reads.walk | extra) >> i & 1 == 1)
                        .map(|(i, &d)| {
                            let by = match i {
                                i if i >= watch::MAX || reads.walk >> i & 1 == 1 => "walk",
                                i if reads.values >> i & 1 == 1 => "value passes",
                                _ => "selection",
                            };
                            format!("{} ({by})", db.decls[d].name)
                        })
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
    Ok(Got::Sel(Box::new(sel)))
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
