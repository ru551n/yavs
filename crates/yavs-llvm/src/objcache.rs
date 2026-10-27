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

/// The kind of the run-to-run profile of a program (`profile_key`, #754).
const PROFILE: &str = "prof";

/// What the profile of a run is keyed by besides the program: the command line (the generics, the
/// stop time, the test case a runner names), set by the CLI.
static RUN_ID: Mutex<Option<String>> = Mutex::new(None);

/// Name the run for its profile (`--tier=auto` keeps one per program and run). Without it there is
/// no profile.
pub fn set_run_id(id: Option<&str>) {
    *RUN_ID.lock().unwrap_or_else(PoisonError::into_inner) = id.map(str::to_string);
}

/// The key of the profile of `prog` in the store: the build, the optimiser switches, the run's id
/// and a digest of the set of functions' identities (name, kind, parent's name and kind, size as
/// lowered, and how many equal ones come before it; the reserved slots of copies are left out, they
/// are filled in as the run goes), so the profile of a run is never read for another one but is
/// found whatever order elaboration lowered the functions in (#774). With it the ids in identity
/// order (`order[rank] = id`): the profile names a function by its rank, not by its id.
/// `None` without a store or a run id.
// ponytail: functions equal in all of that (the same label and size in two architectures) swap
// places when their order changes; a profile applied to the wrong one only mis-steers the heat.
pub(crate) fn profile_key(prog: &yavs_ir::Program) -> Option<(Key, Vec<u32>)> {
    use std::hash::{Hash, Hasher};
    let id = RUN_ID
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()?;
    if !enabled() {
        return None;
    }
    let mut count: std::collections::HashMap<u64, u32> = Default::default();
    let mut ident: Vec<(u64, u32)> = Vec::new();
    for (i, f) in prog.funcs.iter().filter(|(_, f)| !f.spare) {
        let parent = f.parent.and_then(|p| prog.funcs.get(p));
        let mut h = std::hash::DefaultHasher::new();
        (&f.name, f.kind, parent.map(|p| (&p.name, p.kind)), f.size).hash(&mut h);
        let same = count.entry(h.finish()).or_insert(0);
        *same += 1;
        same.hash(&mut h);
        ident.push((h.finish(), i.0));
    }
    ident.sort_unstable();
    let mut h = std::hash::DefaultHasher::new();
    ident.iter().for_each(|i| i.0.hash(&mut h));
    let mut k = KeyBuilder::new(PROFILE);
    k.text(&yavs_ir::opt::fingerprint())
        .text(&id)
        .part(&h.finish().to_le_bytes());
    Some((k.finish(), ident.into_iter().map(|i| i.1).collect()))
}

/// The profile stored under `key`, if any.
pub(crate) fn profile_load(key: &Key) -> Option<String> {
    let store = STORE
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()?;
    let e = store.get(PROFILE, key)?;
    String::from_utf8(e.to_vec()).ok()
}

/// Store the profile (atomically; a run that writes after another one replaces its profile).
pub(crate) fn profile_store(key: &Key, text: &str) {
    let store = STORE.lock().unwrap_or_else(PoisonError::into_inner).clone();
    if let Some(store) = store {
        store.put(PROFILE, key, text.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The tests that set the store and the run id (process-wide) take turns.
    static GLOBALS: Mutex<()> = Mutex::new(());

    /// The profile of a run is kept per program and per run id (the command line), is read back
    /// whole, and exists only with a store and an id.
    #[test]
    fn the_profile_is_kept_per_program_and_run() {
        let _g = GLOBALS.lock().unwrap_or_else(PoisonError::into_inner);
        let (prog, _) = {
            let mut pb = yavs_ir::ProgramBuilder::new();
            let f = pb.declare("p", yavs_ir::FuncKind::Process, None);
            let mut fb = pb.define(f);
            let body = fb.block();
            fb.stmt(
                yavs_ir::StmtKind::Wait,
                yavs_diag::Span::new(yavs_diag::FileId(0), 0, 1),
            );
            fb.emit(yavs_ir::Inst::Br { to: body });
            fb.at(body);
            fb.emit(yavs_ir::Inst::Suspend { resume: 0 });
            fb.emit(yavs_ir::Inst::Br { to: body });
            fb.finish();
            (pb.finish().unwrap(), f)
        };
        let dir = tempfile::tempdir().unwrap();
        set_cache(None);
        set_run_id(Some("a"));
        assert!(profile_key(&prog).is_none(), "no store");
        set_cache(Some(dir.path()));
        set_run_id(None);
        assert!(profile_key(&prog).is_none(), "no run id");
        set_run_id(Some("a"));
        let a = profile_key(&prog).expect("a key").0;
        set_run_id(Some("b"));
        let b = profile_key(&prog).expect("a key").0;
        assert_ne!(a, b);
        assert_eq!(profile_load(&a), None);
        profile_store(&a, "yavs-auto-profile 2\nbase 5\n");
        assert_eq!(
            profile_load(&a).as_deref(),
            Some("yavs-auto-profile 2\nbase 5\n")
        );
        assert_eq!(profile_load(&b), None);
        set_cache(None);
        set_run_id(None);
    }

    /// The profile key and the ranks do not depend on the order functions were lowered in (#774):
    /// the same functions in another order give the same key and the same function at each rank.
    #[test]
    fn the_profile_key_ignores_the_order_of_functions() {
        let _g = GLOBALS.lock().unwrap_or_else(PoisonError::into_inner);
        use yavs_ir::{FuncKind, Inst, ProgramBuilder, StmtKind};
        let build = |order: &[(&str, usize)]| {
            let mut pb = ProgramBuilder::new();
            let ids: Vec<_> = order
                .iter()
                .map(|&(n, _)| pb.declare(n, FuncKind::Procedure, None))
                .collect();
            for (&id, &(_, len)) in ids.iter().zip(order) {
                let mut fb = pb.define(id);
                let spin = fb.block();
                fb.stmt(
                    StmtKind::Return,
                    yavs_diag::Span::new(yavs_diag::FileId(0), 0, 1),
                );
                let ptr = fb.reg(yavs_ir::RegTy::Ptr);
                fb.emit(Inst::AddrOf {
                    dst: ptr,
                    space: yavs_ir::AddrSpace::Frame,
                    off: 0,
                });
                for _ in 0..len {
                    fb.emit(Inst::Free { ptr });
                }
                fb.emit(Inst::Br { to: spin });
                fb.at(spin);
                fb.emit(Inst::Br { to: spin });
                fb.finish();
            }
            pb.finish().unwrap()
        };
        let names = |p: &yavs_ir::Program, order: &[u32]| -> Vec<String> {
            order
                .iter()
                .map(|&i| p.funcs.get(yavs_ir::FuncId(i)).unwrap().name.clone())
                .collect()
        };
        let dir = tempfile::tempdir().unwrap();
        set_cache(Some(dir.path()));
        set_run_id(Some("a"));
        let one = build(&[("f", 3), ("g", 5), ("h", 3), ("h", 3)]);
        let two = build(&[("h", 3), ("g", 5), ("h", 3), ("f", 3)]);
        let other = build(&[("f", 3), ("g", 6), ("h", 3), ("h", 3)]);
        let (k1, o1) = profile_key(&one).unwrap();
        let (k2, o2) = profile_key(&two).unwrap();
        assert_eq!(k1, k2, "the same functions in another order");
        assert_ne!(o1, o2, "the ids do move");
        assert_eq!(names(&one, &o1), names(&two, &o2));
        assert_ne!(k1, profile_key(&other).unwrap().0, "a function differs");
        set_cache(None);
        set_run_id(None);
    }
}
