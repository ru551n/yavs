//! Compiled code kept across runs (#505, a slice of the compilation cache #407;
//! `docs/decisions.md` "Compiled code in the compilation cache").
//!
//! A module's machine code is a function of its LLVM IR, the optimisation level and pipeline,
//! the target (triple, CPU baseline, LLVM version, code generator switches) and the yavs build, so the object file is stored under a
//! key made of exactly those (with the switches of `yavs_ir::opt::fingerprint`, #491) and the
//! next module with the same IR links the stored object instead of being optimised and compiled.
//! The IR is what the generator made, before the pass pipeline: anything the code depends on is
//! in it, so a hit is the code a compile would make, and a module that differs in anything is a
//! miss. Nothing absolute is baked into the IR (the runtime callbacks are symbols ORC resolves when
//! it links the object, constants are data of the module), so a stored object links in any
//! process. The store is the `yavs-cache` objects store of one library directory
//! ([`set_cache`]); a missing, damaged or foreign entry is a miss, a write that fails is silent.

use std::cell::Cell;
use std::path::Path;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, OnceLock, PoisonError};

use yavs_cache::{Key, KeyBuilder, Store};

use crate::{Error, Jit, Module, OptLevel};

const KIND: &str = "obj";

// ponytail: one store per process (a run simulates one design); a store per library of origin
// would share code between designs in different work libraries.
static STORE: Mutex<Option<Store>> = Mutex::new(None);
static HITS: AtomicUsize = AtomicUsize::new(0);
static STORED: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    /// Set by [`cached_only`]: `add` links what the cache has and compiles nothing.
    static CACHED_ONLY: Cell<bool> = const { Cell::new(false) };
}

/// Keep the compiled code in the cache of library directory `lib` from now on (`None`: compile
/// every module).
pub fn set_cache(lib: Option<&Path>) {
    *STORE.lock().unwrap_or_else(PoisonError::into_inner) = lib.map(Store::objects_of_library);
}

/// Modules linked from the cache, and modules compiled and stored, so far in this process.
pub fn cache_stats() -> (usize, usize) {
    (HITS.load(Ordering::Relaxed), STORED.load(Ordering::Relaxed))
}

/// A module that is the code of one linked before counts as linked from the cache.
pub(crate) fn note_twin() {
    HITS.fetch_add(1, Ordering::Relaxed);
}

/// The store, if compiled code is kept in one (it keeps the run-to-run profile too).
pub(crate) fn store() -> Option<Store> {
    STORE.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

/// Whether compiled code is kept in a cache.
pub(crate) fn enabled() -> bool {
    STORE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .is_some()
}

/// While on, `add` links only stored objects: a module the cache does not have fails instead of
/// compiling.
pub(crate) fn set_cached_only(on: bool) {
    CACHED_ONLY.set(on);
}

fn debug(what: &str, k: &Key) {
    if std::env::var("YAVS_LOG").is_ok_and(|v| v == "debug") {
        eprintln!("debug: cache: obj {what} {}", k.hex());
    }
}

/// The key of the object of `m` at `opt` and `full` (`ir`: a digest of its bitcode made already).
pub(crate) fn key(m: &Module, opt: OptLevel, full: bool, ir: Option<&str>) -> Key {
    static TARGET: OnceLock<String> = OnceLock::new();
    let mut k = KeyBuilder::new(KIND);
    k.text(&yavs_ir::opt::fingerprint())
        .text(TARGET.get_or_init(crate::jit::target_id))
        .text(&format!("{opt:?} full={full}"));
    match ir {
        Some(d) => k.text(d),
        None => k.part(&m.bitcode()),
    };
    k.finish()
}

fn times() -> bool {
    static TIMES: OnceLock<bool> = OnceLock::new();
    *TIMES.get_or_init(|| std::env::var_os("YAVS_LLVM_TIMES").is_some())
}

/// Add `m` to `jit` at `opt` after `prepare` (verify and `optimize_as(opt, full)`); with a cache,
/// link its stored object instead when there is one, else prepare it, compile it to an object,
/// store that and link it, and return the object's key. `ir`: a digest of `m`'s bitcode the caller
/// made already (a copy's `twin` key, which names the module), keyed instead of writing the
/// bitcode again.
pub(crate) fn add(
    jit: &mut Jit,
    mut m: Module,
    opt: OptLevel,
    full: bool,
    ir: Option<&str>,
    prepare: impl FnOnce(&mut Module) -> Result<(), Error>,
) -> Result<Option<Key>, Error> {
    let store = STORE.lock().unwrap_or_else(PoisonError::into_inner).clone();
    let Some(store) = store else {
        prepare(&mut m)?;
        // ORC makes its own target machine, which has no FastISel setting for `FastRa`.
        return if opt == OptLevel::FastRa {
            jit.add_object(&m.emit(opt)?)
        } else {
            jit.add(m)
        }
        .map(|()| None);
    };
    let t0 = std::time::Instant::now();
    let k = key(&m, opt, full, ir);
    let t1 = std::time::Instant::now();
    // An object ORC does not take is a miss like any other bad entry.
    if let Some(obj) = store.get(KIND, &k) {
        let t2 = std::time::Instant::now();
        if jit.add_object(&obj).is_ok() {
            HITS.fetch_add(1, Ordering::Relaxed);
            debug("hit", &k);
            let t3 = std::time::Instant::now();
            drop(m);
            if times() {
                // Tuning aid: the microseconds of the key, the look-up, the link and dropping
                // the module of a module linked from the cache.
                let us = |a: std::time::Instant, b: std::time::Instant| (b - a).as_micros();
                eprintln!(
                    "objtimes hit key={} get={} link={} drop={} obj={}",
                    us(t0, t1),
                    us(t1, t2),
                    us(t2, t3),
                    t3.elapsed().as_micros(),
                    obj.len()
                );
            }
            return Ok(Some(k));
        }
    }
    if CACHED_ONLY.get() {
        return Err(Error("not in the compilation cache".into()));
    }
    let t2 = std::time::Instant::now();
    prepare(&mut m)?;
    let t3 = std::time::Instant::now();
    let obj = m.emit(opt)?;
    let t4 = std::time::Instant::now();
    if let Some(dir) = std::env::var_os("YAVS_LLVM_OBJ") {
        // Tuning aid: the object files, for `objdump` next to a `perf` profile.
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let _ = std::fs::write(Path::new(&dir).join(format!("{n}.o")), &obj);
    }
    store.put(KIND, &k, &obj);
    let t5 = std::time::Instant::now();
    STORED.fetch_add(1, Ordering::Relaxed);
    debug("stored", &k);
    let r = jit.add_object(&obj);
    if times() {
        // Tuning aid next to `compile_one`'s line: the microseconds of the key, the look-up,
        // the passes, the code generator, the store and the link of a module that was compiled.
        let us = |a: std::time::Instant, b: std::time::Instant| (b - a).as_micros();
        eprintln!(
            "objtimes key={} get={} prepare={} emit={} put={} link={} obj={}",
            us(t0, t1),
            us(t1, t2),
            us(t2, t3),
            us(t3, t4),
            us(t4, t5),
            t5.elapsed().as_micros(),
            obj.len()
        );
    }
    r.map(|()| Some(k))
}

/// The kind of the entries that name a function's object before its code is generated (#785).
const PRE: &str = "pre";

/// The tuning switches that change the generated code (and `YAVS_LLVM_DUMP`, which dumps what is
/// generated): a run with one of them set has no pre-keys (it generates and keys the bitcode).
const GEN_SWITCHES: [&str; 7] = [
    "YAVS_LLVM_ABLATE",
    "YAVS_LLVM_DUMP",
    "YAVS_LLVM_ELOOP_HELPER",
    "YAVS_LLVM_KINDLOC",
    "YAVS_LLVM_KINDS",
    "YAVS_LLVM_NO_ELOOP",
    "YAVS_LLVM_PCLOC",
];

/// What a pre-key names (`pre_get`): the key of the object, whether its module has native-frame
/// code (the generator drops code that does not verify), and a copy's twin key (which names its
/// symbols, `compile_one`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Pre {
    pub obj: Key,
    pub native: bool,
    pub twin: Option<String>,
}

/// `YAVS_LLVM_PREKEY_CHECK` (CI; set, not empty and not `0`): a function whose pre-key names an
/// object is generated anyway, and a module other than the one named is an internal error
/// (`pre_note`).
pub(crate) fn check() -> bool {
    static CHECK: OnceLock<bool> = OnceLock::new();
    *CHECK.get_or_init(|| {
        std::env::var("YAVS_LLVM_PREKEY_CHECK").is_ok_and(|v| !v.is_empty() && v != "0")
    })
}

/// The key that names the object of function `f` (`id` of `prog`) before its code is generated
/// (#785, maintainer 2026-10-27): the build (in every key), the optimiser switches, the target,
/// the program's identity (`Program::identity`: its stage-cache entry, everything the generator
/// reads of the program), the process `id` is or is a copy of, the function as handed out (its
/// optimised code: a copy's folded data among it) and `what` (level, `full`, part). `None` without
/// a store, for a program not cached, or with a switch of `GEN_SWITCHES`.
pub(crate) fn pre_key(
    prog: &yavs_ir::Program,
    id: yavs_ir::FuncId,
    f: &yavs_ir::Func,
    what: &str,
) -> Option<Key> {
    static TARGET: OnceLock<String> = OnceLock::new();
    static SWITCHED: OnceLock<bool> = OnceLock::new();
    let identity = prog.identity.as_deref()?;
    if !enabled()
        || *SWITCHED.get_or_init(|| GEN_SWITCHES.iter().any(|v| std::env::var_os(v).is_some()))
    {
        return None;
    }
    let mut k = KeyBuilder::new(PRE);
    k.text(&yavs_ir::opt::fingerprint())
        .text(TARGET.get_or_init(crate::jit::target_id))
        .text(identity)
        .text(what)
        .part(&prog.base_of(id).0.to_le_bytes())
        .part(&postcard::to_stdvec(f).ok()?);
    Some(k.finish())
}

/// The entry of pre-key `k`, if any.
pub(crate) fn pre_get(k: &Key) -> Option<Pre> {
    let store = STORE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()?;
    let e = store.get(PRE, k)?;
    let text = std::str::from_utf8(&e).ok()?;
    let mut w = text.split(' ');
    let obj = Key::from_hex(w.next()?)?;
    let native = match w.next()? {
        "1" => true,
        "0" => false,
        _ => return None,
    };
    let twin = match w.next()? {
        "-" => None,
        t => Some(t.to_string()),
    };
    w.next().is_none().then_some(Pre { obj, native, twin })
}

/// Record that pre-key `k` names `now`, the module just generated; `stored` is what its entry
/// named. Under `check` an entry that named another module is an internal error.
pub(crate) fn pre_note(k: &Key, stored: Option<&Pre>, now: &Pre, what: impl Fn() -> String) {
    match stored {
        Some(s) if s == now => debug("pre checked", k),
        Some(s) if check() => {
            eprintln!(
                "internal error: the compilation cache's pre-key {} of {} names object {} \
                 (native {}, twin {:?}), but the code generated now is object {} (native {}, twin \
                 {:?}): an input of the generator is missing from the pre-key; this is a bug in yavs",
                k.hex(),
                what(),
                s.obj.hex(),
                s.native,
                s.twin,
                now.obj.hex(),
                now.native,
                now.twin
            );
            std::process::abort();
        }
        _ => {
            let store = STORE.lock().unwrap_or_else(PoisonError::into_inner).clone();
            if let Some(store) = store {
                let twin = now.twin.as_deref().unwrap_or("-");
                let text = format!("{} {} {twin}", now.obj.hex(), u8::from(now.native));
                store.put(PRE, k, text.as_bytes());
            }
        }
    }
}

/// Link the stored object `obj` into `jit`; whether it was there and linked.
pub(crate) fn link(jit: &mut Jit, obj: &Key) -> bool {
    let store = STORE.lock().unwrap_or_else(PoisonError::into_inner).clone();
    let t0 = std::time::Instant::now();
    let Some(bytes) = store.and_then(|s| s.get(KIND, obj)) else {
        return false;
    };
    let t1 = std::time::Instant::now();
    if jit.add_object(&bytes).is_err() {
        return false;
    }
    HITS.fetch_add(1, Ordering::Relaxed);
    debug("prehit", obj);
    if times() {
        // Tuning aid: the microseconds of the look-up and the link of an object a pre-key named.
        eprintln!(
            "objtimes prehit get={} link={} obj={}",
            (t1 - t0).as_micros(),
            t1.elapsed().as_micros(),
            bytes.len()
        );
    }
    true
}

/// The kind of the list of compiled functions (`compiled_list`).
const LIST: &str = "list";

/// The most functions the list names (the most recent ones).
const LIST_MAX: usize = 1024;

/// The key of the list of the store: one per library directory.
fn list_key() -> Key {
    let mut k = KeyBuilder::new(LIST);
    k.text("functions");
    k.finish()
}

/// The functions `--tier=auto` stored compiled code of in this store, as name and number of IR
/// instructions, most recent first: a look-up in the cache (`Worker::probe`) generates the
/// function's code first, so a function not named here is not looked up (a run whose functions
/// were never compiled does not even open the LLVM library). Another design's function of the
/// same name and size is a look-up that misses.
pub(crate) fn compiled_list() -> std::collections::HashSet<(String, u32)> {
    let store = STORE.lock().unwrap_or_else(PoisonError::into_inner).clone();
    let Some(e) = store.and_then(|s| s.get(LIST, &list_key())) else {
        return Default::default();
    };
    String::from_utf8_lossy(&e)
        .lines()
        .filter_map(|l| {
            let (len, name) = l.split_once('\t')?;
            Some((name.to_string(), len.parse().ok()?))
        })
        .collect()
}

/// Put `name` (of `len` IR instructions) in front of the list of `compiled_list`.
pub(crate) fn add_compiled(name: &str, len: u32) {
    let store = STORE.lock().unwrap_or_else(PoisonError::into_inner).clone();
    let Some(store) = store else {
        return;
    };
    let old = store.get(LIST, &list_key());
    let mut text = format!("{len}\t{name}\n");
    let line = text.clone();
    if let Some(old) = old {
        let old = String::from_utf8_lossy(&old);
        for l in old.split_inclusive('\n').take(LIST_MAX - 1) {
            if l != line {
                text += l;
            }
        }
    }
    store.put(LIST, &list_key(), text.as_bytes());
}
