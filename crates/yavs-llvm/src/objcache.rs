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
    store.put(KIND, &k, &obj);
    STORED.fetch_add(1, Ordering::Relaxed);
    debug("stored", &k);
    jit.add_object(&obj)
}
