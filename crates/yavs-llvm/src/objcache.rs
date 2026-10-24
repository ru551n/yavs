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

/// Whether compiled code is kept in a cache.
pub(crate) fn enabled() -> bool {
    STORE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .is_some()
}

/// Run `f` with `add` linking only stored objects: a module the cache does not have fails
/// instead of compiling.
pub(crate) fn cached_only<T>(f: impl FnOnce() -> T) -> T {
    CACHED_ONLY.set(true);
    let r = f();
    CACHED_ONLY.set(false);
    r
}

fn debug(what: &str, k: &Key) {
    if std::env::var("YAVS_LOG").is_ok_and(|v| v == "debug") {
        eprintln!("debug: cache: obj {what} {}", k.hex());
    }
}

fn key(m: &Module, opt: OptLevel, full: bool) -> Key {
    static TARGET: OnceLock<String> = OnceLock::new();
    let mut k = KeyBuilder::new(KIND);
    k.text(&yavs_ir::opt::fingerprint())
        .text(TARGET.get_or_init(crate::jit::target_id))
        .text(&format!("{opt:?} full={full}"))
        .part(&m.bitcode());
    k.finish()
}

/// Add `m` to `jit` at `opt` after `prepare` (verify and `optimize_as(opt, full)`); with a cache,
/// link its stored object instead when there is one, else prepare it, compile it to an object,
/// store that and link it.
pub(crate) fn add(
    jit: &mut Jit,
    mut m: Module,
    opt: OptLevel,
    full: bool,
    prepare: impl FnOnce(&mut Module) -> Result<(), Error>,
) -> Result<(), Error> {
    let store = STORE.lock().unwrap_or_else(PoisonError::into_inner).clone();
    let Some(store) = store else {
        prepare(&mut m)?;
        return jit.add(m);
    };
    let k = key(&m, opt, full);
    // An object ORC does not take is a miss like any other bad entry.
    if let Some(obj) = store.get(KIND, &k)
        && jit.add_object(&obj).is_ok()
    {
        HITS.fetch_add(1, Ordering::Relaxed);
        debug("hit", &k);
        return Ok(());
    }
    if CACHED_ONLY.get() {
        return Err(Error("not in the compilation cache".into()));
    }
    prepare(&mut m)?;
    let obj = m.emit(opt)?;
    if let Some(dir) = std::env::var_os("YAVS_LLVM_OBJ") {
        // Tuning aid: the object files, for `objdump` next to a `perf` profile.
        static N: AtomicUsize = AtomicUsize::new(0);
        let n = N.fetch_add(1, Ordering::Relaxed);
        let _ = std::fs::write(Path::new(&dir).join(format!("{n}.o")), &obj);
    }
    store.put(KIND, &k, &obj);
    STORED.fetch_add(1, Ordering::Relaxed);
    debug("stored", &k);
    jit.add_object(&obj)
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
