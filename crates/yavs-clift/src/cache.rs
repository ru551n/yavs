//! Compiled code kept across runs (#481), on the design of the LLVM tier's `objcache.rs`: the
//! machine code of each Cranelift function is stored in the `yavs-cache` objects store of the work
//! library ([`set_cache`]) and the next function with the same body links it instead of being
//! compiled (`Jit::define_code`).
//!
//! The key is Cranelift's own (`incremental_cache::compute_cache_key`: the function body and the
//! target, not the names of the functions and data it refers to, which are relocations the module
//! resolves when it links the code) with the yavs build and the optimiser's switches
//! (`yavs_ir::opt::fingerprint`). The body is what the lowering and the inliner made, so a hit is
//! the code a compile would make. A missing, damaged or foreign entry is a miss, a write that fails
//! is silent.
//!
//! The body also says which native callees it calls directly (and may inline) and which through
//! `Mem::ntab`. Under `--tier=clift-auto` that follows what the policy compiles with the function,
//! a function of the samples, so [`Routes`] keeps the first answer: an unchanged design then makes
//! the same bodies every run (#1035).
//!
//! That key needs the body. A warm run finds the code before the body is made: [`pre_key`] names
//! every input of the lowering, and the entry under it ([`Pre`]) names the Cranelift key and, in
//! terms of any run, the symbols the body refers to. `YAVS_CLIFT_PREKEY_CHECK` ([`check`], always
//! in debug builds) makes the body anyway and aborts when it is not what the entry says.

use std::borrow::Cow;
use std::collections::{BTreeMap, HashMap};
use std::path::Path;
use std::rc::Rc;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Mutex, PoisonError};

use cranelift_codegen::CompiledCode;
use cranelift_codegen::incremental_cache::{CacheKvStore, compute_cache_key, try_finish_recompile};
use cranelift_codegen::ir::Function;
use cranelift_codegen::isa::TargetIsa;
use yavs_cache::{Key, KeyBuilder, Store};

const KIND: &str = "clif";

// ponytail: one store per process, as the LLVM tier's (a run simulates one design).
static STORE: Mutex<Option<Store>> = Mutex::new(None);
static HITS: AtomicUsize = AtomicUsize::new(0);
static STORED: AtomicUsize = AtomicUsize::new(0);

/// Keep the compiled code in the cache of library directory `lib` from now on (`None`: compile
/// every function).
pub fn set_cache(lib: Option<&Path>) {
    *STORE.lock().unwrap_or_else(PoisonError::into_inner) = lib.map(Store::objects_of_library);
}

/// Functions linked from the cache, and functions compiled and stored, so far in this process.
pub fn cache_stats() -> (usize, usize) {
    (HITS.load(Ordering::Relaxed), STORED.load(Ordering::Relaxed))
}

pub(crate) fn store() -> Option<Store> {
    STORE.lock().unwrap_or_else(PoisonError::into_inner).clone()
}

pub(crate) fn note_hit() {
    HITS.fetch_add(1, Ordering::Relaxed);
}

/// The store as Cranelift's cache of compiled functions.
pub(crate) struct Kv(pub Store);

impl Kv {
    fn key(k: &[u8]) -> Key {
        let mut b = KeyBuilder::new(KIND);
        b.text(&yavs_ir::opt::fingerprint()).part(k);
        b.finish()
    }

    /// Whether the store has the compiled code of `func`.
    pub(crate) fn has(&self, isa: &dyn TargetIsa, func: &Function) -> bool {
        self.0
            .payload_len(KIND, &Self::key(&clif_key(isa, func)))
            .is_some_and(|n| n > 0)
    }
}

/// Cranelift's key of `func` for this target, as bytes.
pub(crate) fn clif_key(isa: &dyn TargetIsa, func: &Function) -> [u8; 32] {
    // (The hash has no accessor, only its `Display`: `CacheKeyHash:[b0, b1, ...]`.)
    let text = compute_cache_key(isa, func).to_string();
    let mut k = [0; 32];
    let digits = text
        .trim_start_matches("CacheKeyHash:[")
        .trim_end_matches(']')
        .split(", ");
    for (out, d) in k.iter_mut().zip(digits) {
        *out = d.parse().expect("the Display of Cranelift's cache key");
    }
    k
}

impl CacheKvStore for Kv {
    fn get(&self, key: &[u8]) -> Option<Cow<'_, [u8]>> {
        self.0
            .get(KIND, &Self::key(key))
            .map(|e| Cow::Owned(e.to_vec()))
    }

    fn insert(&mut self, key: &[u8], val: Vec<u8>) {
        self.0.put(KIND, &Self::key(key), &val);
        STORED.fetch_add(1, Ordering::Relaxed);
    }
}

/// The kind of the entries of [`pre_key`].
const PRE: &str = "clifpre";

/// The switches whose value changes the code the lowering or Cranelift makes: in every pre-key.
pub(crate) const KEYED: [&str; 9] = [
    "YAVS_CLIFT_OPT",
    "YAVS_CLIFT_VERIFY",
    "YAVS_CLIFT_RA",
    "YAVS_CLIFT_NATIVE",
    "YAVS_CLIFT_INLINE",
    "YAVS_CLIFT_PIN",
    "YAVS_CLIFT_NTAB",
    "YAVS_CLIFT_NO_SCAN",
    "YAVS_CLIFT_ONCE",
];

/// With one of these set a run has no pre-keys: they want the code made (dumps, source locations)
/// or make it depend on what was compiled before (`YAVS_CLIFT_NATIVE_MAX`).
pub(crate) const NO_PRE: [&str; 5] = [
    "YAVS_CLIFT_DUMP",
    "YAVS_CLIFT_CLIF",
    "YAVS_CLIFT_ASM",
    "YAVS_CLIFT_CODEDIR",
    "YAVS_CLIFT_NATIVE_MAX",
];

/// A symbol a body refers to, as any run names it: its ids are this module's.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
pub(crate) enum Name {
    /// A runtime callback or a native variant `n<id>`, by symbol.
    Func(String),
    /// Module data: a table, a constant, the program of an element loop.
    Data { bytes: Vec<u8>, align8: bool },
}

/// What a pre-key names.
#[derive(serde::Serialize, serde::Deserialize, Debug, PartialEq)]
pub(crate) enum Pre {
    /// The code under Cranelift's key `clif`, the symbols of the body in the order of its
    /// references, and the kinds of instruction it steps with their counts.
    Code {
        clif: [u8; 32],
        names: Vec<Name>,
        stepped: BTreeMap<String, u32>,
    },
    /// The lowering refuses the function, for this reason.
    Refused(String),
}

/// Every input of the lowering of a function, up to what [`Program::identity`] names
/// (docs/decisions.md "Cranelift: compiled code found before the body is made").
///
/// [`Program::identity`]: yavs_ir::Program::identity
pub(crate) struct PreInputs<'a> {
    /// `yavs_ir::opt::fingerprint`.
    pub fingerprint: &'a str,
    /// [`pre_target`]: Cranelift's version, the instruction set, its flags and the host's features.
    pub target: &'a str,
    /// [`pre_target`]: the values of the switches of the code generator.
    pub switches: &'a str,
    /// The program: its types, constants, tables and other functions, ids and all.
    pub identity: &'a str,
    /// The function, or the process a copy is of: a copy's own id is in no code it has.
    pub base: u32,
    /// Native-frame variant or frame-mode code.
    pub native: bool,
    /// The function as handed out, serialised: a copy's folded data is in its code.
    pub func: &'a [u8],
    /// The facts of a copy, which are in neither of the above.
    pub facts: &'a [u8],
    /// The route: per native callee, through `Mem::ntab` (`None`) or directly, with its pre-key,
    /// since its body may be inlined. In callee order.
    pub callees: &'a [(u32, Option<Key>)],
}

/// The key of the entry of a function, made before anything is lowered.
pub(crate) fn pre_key(i: &PreInputs) -> Key {
    pre_key_without(i, dropped())
}

/// `YAVS_CLIFT_PREKEY_DROP` (debug builds, the test of the check): the inputs left out of the key,
/// as if the key had forgotten them.
fn dropped() -> &'static str {
    static DROP: OnceLock<String> = OnceLock::new();
    if !cfg!(debug_assertions) {
        return "";
    }
    DROP.get_or_init(|| std::env::var("YAVS_CLIFT_PREKEY_DROP").unwrap_or_default())
}

fn pre_key_without(i: &PreInputs, drop: &str) -> Key {
    let on = |class: &str| !drop.split(',').any(|d| d == class);
    let mut k = KeyBuilder::new(PRE);
    let mut text = |class: &str, s: &str| {
        k.text(if on(class) { s } else { "" });
    };
    text("fingerprint", i.fingerprint);
    text("target", i.target);
    text("switches", i.switches);
    text("identity", i.identity);
    if on("base") {
        k.part(&i.base.to_le_bytes());
    }
    if on("native") {
        k.part(&[u8::from(i.native)]);
    }
    k.part(if on("func") { i.func } else { &[] });
    k.part(if on("facts") { i.facts } else { &[] });
    if on("callees") {
        for (g, n) in i.callees {
            k.part(&g.to_le_bytes());
            k.text(&n.map_or(String::new(), |n| n.hex()));
        }
    }
    k.finish()
}

/// A value's `Hash` bytes (`InstFacts` has no other form, and a field added to it comes along).
pub(crate) struct HashBytes(pub Vec<u8>);

impl std::hash::Hasher for HashBytes {
    fn write(&mut self, bytes: &[u8]) {
        self.0.extend_from_slice(bytes);
    }

    fn finish(&self) -> u64 {
        0
    }
}

/// The target and the switches of a module's pre-keys, `None` when a switch of [`NO_PRE`] is set.
/// `inlcmp` is the mode of the compare of drives, whose default depends on the build as the verifier's does.
pub(crate) fn pre_target(isa: &dyn TargetIsa, inlcmp: &str) -> Option<(String, String)> {
    if NO_PRE.iter().any(|v| std::env::var_os(v).is_some()) {
        return None;
    }
    // (A body's Cranelift key has the instruction set, its flags and the host's features, not
    // the version: `VersionMarker` hashes nothing.)
    let target = format!(
        "{} {}",
        cranelift_codegen::VERSION,
        compute_cache_key(isa, &Function::new())
    );
    let mut switches = format!("debug={} inlcmp={inlcmp}", cfg!(debug_assertions));
    for v in KEYED {
        switches += &format!(" {v}={:?}", std::env::var_os(v));
    }
    Some((target, switches))
}

/// The key of `Jit::bodies`: a body and the module's functions and data it refers to.
pub(crate) fn body_key(clif: &[u8; 32], func: &Function) -> String {
    let names: Vec<(u32, u32)> = func
        .params
        .user_named_funcs()
        .values()
        .map(|n| (n.namespace, n.index))
        .collect();
    format!("{clif:?} {names:?}")
}

/// `YAVS_CLIFT_PREKEY_CHECK` (not empty and not `0`; on in debug builds when unset): a function
/// whose pre-key has an entry is lowered anyway, and a body other than the one the entry names is
/// an internal error.
pub(crate) fn check() -> bool {
    static CHECK: OnceLock<bool> = OnceLock::new();
    *CHECK.get_or_init(|| match std::env::var("YAVS_CLIFT_PREKEY_CHECK") {
        Ok(v) => !v.is_empty() && v != "0",
        Err(_) => cfg!(debug_assertions),
    })
}

fn debug(what: &str, text: impl FnOnce() -> String) {
    if std::env::var("YAVS_LOG").is_ok_and(|v| v == "debug") {
        eprintln!("debug: cache: clif {what} {}", text());
    }
}

/// The entry of pre-key `k`; a missing, damaged or foreign one is none.
pub(crate) fn pre_get(k: &Key) -> Option<Pre> {
    let e = store()?.get(PRE, k)?;
    postcard::from_bytes(e.as_ref()).ok()
}

/// A body of `f<id>` or `n<id>` is being made.
pub(crate) fn note_lowered(id: u32, native: bool) {
    debug("lowered", || format!("{id} native={native}"));
}

/// Code was linked by a pre-key, with no body made.
pub(crate) fn note_prehit(clif: &[u8; 32]) {
    debug("prehit", || format!("{clif:?}"));
}

/// Under [`check`], `now` is what the lowering made for pre-key `k`: an entry that names anything
/// else is an internal error.
pub(crate) fn pre_check(k: &Key, stored: Option<&Pre>, now: &Pre, name: &str, id: u32, nat: bool) {
    let Some(s) = stored.filter(|_| check()) else {
        return;
    };
    if s == now {
        debug("pre checked", || k.hex());
        return;
    }
    eprintln!(
        "internal error: the compilation cache's pre-key {} of `{name}` (f{id}, native {nat}) \
         names {s:?}, but the lowering made {now:?} now: an input of the lowering is missing from \
         the pre-key; this is a bug in yavs",
        k.hex()
    );
    std::process::abort();
}

/// Record that pre-key `k` names `now`, what the lowering just made.
pub(crate) fn pre_note(k: &Key, stored: Option<&Pre>, now: &Pre, name: &str, id: u32, nat: bool) {
    pre_check(k, stored, now, name, id, nat);
    if stored != Some(now)
        && let (Some(store), Ok(bytes)) = (store(), postcard::to_stdvec(now))
    {
        store.put(PRE, k, &bytes);
    }
}

/// The code stored under Cranelift's key `clif`, its references named by `func`'s parameters.
pub(crate) fn stored_code(clif: &[u8; 32], func: &Function) -> Option<CompiledCode> {
    let e = store()?.get(KIND, &Kv::key(clif))?;
    try_finish_recompile(func, e.as_ref()).ok()
}

/// The kinds of stepped instruction with their counts, as an entry keeps them.
pub(crate) fn stepped_counts(stepped: &[String]) -> BTreeMap<String, u32> {
    let mut m = BTreeMap::new();
    for s in stepped {
        *m.entry(s.clone()).or_insert(0) += 1;
    }
    m
}

/// The stepped kinds of an entry, for what the census counts.
pub(crate) fn stepped_list(stepped: &BTreeMap<String, u32>) -> Vec<String> {
    stepped
        .iter()
        .flat_map(|(s, &n)| std::iter::repeat_n(s.clone(), n as usize))
        .collect()
}

/// The kind of the list of compiled functions (`compiled_list`), shared with the LLVM tier's list
/// under its own key.
const LIST: &str = "list";

/// The most functions the list names (the most recent ones).
const LIST_MAX: usize = 1024;

fn list_key() -> Key {
    let mut k = KeyBuilder::new(LIST);
    k.text("clift functions");
    k.finish()
}

/// The functions `--tier=clift-auto` stored compiled code of, as name and number of IR
/// instructions, most recent first: the policy looks up only these (`Worker::probe`).
pub(crate) fn compiled_list() -> std::collections::HashSet<(String, u32)> {
    let Some(e) = store().and_then(|s| s.get(LIST, &list_key())) else {
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
    let Some(store) = store() else {
        return;
    };
    let line = format!("{len}\t{name}\n");
    let mut text = line.clone();
    if let Some(old) = store.get(LIST, &list_key()) {
        let old = String::from_utf8_lossy(&old);
        for l in old.split_inclusive('\n').take(LIST_MAX - 1) {
            if l != line {
                text += l;
            }
        }
    }
    store.put(LIST, &list_key(), text.as_bytes());
}

/// The kind of the entry of [`Routes`].
const ROUTES: &str = "route";

/// How a function reaches its native callees: per callee id, directly (`true`) or through
/// `Mem::ntab`.
pub(crate) type Route = Rc<HashMap<u32, bool>>;

/// The routes by function and whether it is the native-frame variant.
type Known = HashMap<(u64, bool), Route>;

/// The first route of each function compiled in a program (`--tier=clift-auto`), one text entry per
/// program in the objects store, a line `<function> <0|1 native> <callee>+|- ...` each. A function is
/// named by its id, a copy of a process by its `Program::copy_tag`: its id depends on the run. Both
/// routes are the same semantics, so a line applied to another program only mis-steers a call.
#[derive(Default)]
pub(crate) struct Routes {
    /// The entry's key and what it held at the first look; `None`: not looked yet.
    loaded: Option<(Key, Known)>,
    /// The lines not stored yet.
    fresh: String,
}

impl Routes {
    fn parse(text: &str) -> Known {
        let line = |l: &str| {
            let mut w = l.split(' ');
            let f = w.next()?.parse().ok()?;
            let native = w.next()? == "1";
            let calls = w
                .map(|c| {
                    let (g, how) = c.split_at_checked(c.len().checked_sub(1)?)?;
                    Some((g.parse().ok()?, how == "+"))
                })
                .collect::<Option<HashMap<u32, bool>>>()?;
            Some(((f, native), Rc::new(calls)))
        };
        text.lines().filter_map(line).collect()
    }

    /// The route stored for a function of `prog`.
    pub(crate) fn get(&mut self, prog: &yavs_ir::Program, f: u64, native: bool) -> Option<Route> {
        let loaded = self.loaded.get_or_insert_with(|| {
            let mut k = KeyBuilder::new(ROUTES);
            k.text(&yavs_ir::opt::fingerprint())
                .part(&yavs_llvm::program_digest(prog).to_le_bytes());
            let k = k.finish();
            let known = store()
                .and_then(|s| s.get(ROUTES, &k))
                .map(|e| Self::parse(&String::from_utf8_lossy(&e)))
                .unwrap_or_default();
            (k, known)
        });
        loaded.1.get(&(f, native)).cloned()
    }

    /// Remember the route a function of no stored route was just made with.
    pub(crate) fn add(&mut self, f: u64, native: bool, calls: impl Iterator<Item = (u32, bool)>) {
        let mut calls: Vec<_> = calls.collect();
        calls.sort_unstable();
        let mut line = format!("{f} {}", u8::from(native));
        for (g, direct) in &calls {
            line += &format!(" {g}{}", if *direct { '+' } else { '-' });
        }
        self.fresh += &line;
        self.fresh.push('\n');
        if let Some((_, known)) = &mut self.loaded {
            known.insert((f, native), Rc::new(calls.into_iter().collect()));
        }
    }

    /// Store the new lines behind what the entry holds now: another run's line for a function wins,
    /// since it is the first one.
    pub(crate) fn flush(&mut self) {
        let (Some((k, _)), Some(store)) = (&self.loaded, store()) else {
            return;
        };
        if self.fresh.is_empty() {
            return;
        }
        let mut text = store
            .get(ROUTES, k)
            .map(|e| String::from_utf8_lossy(&e).into_owned())
            .unwrap_or_default();
        let there = Self::parse(&text);
        for l in std::mem::take(&mut self.fresh).lines() {
            if Self::parse(l).keys().all(|f| !there.contains_key(f)) {
                text += l;
                text.push('\n');
            }
        }
        store.put(ROUTES, k, text.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every input of the key moves it.
    #[test]
    fn every_input_of_the_pre_key_moves_it() {
        let callee = KeyBuilder::new("x").finish();
        let mut other = KeyBuilder::new("y");
        other.text("y");
        let other = other.finish();
        let base = |callees: &'static [(u32, Option<Key>)]| PreInputs {
            fingerprint: "f",
            target: "t",
            switches: "s",
            identity: "i",
            base: 1,
            native: false,
            func: b"code",
            facts: b"facts",
            callees,
        };
        // (Leaked: a test, and the slices must outlive the closure above.)
        let leak = |c: Vec<(u32, Option<Key>)>| -> &'static [(u32, Option<Key>)] { c.leak() };
        let c0 = leak(vec![(3, Some(callee))]);
        let k0 = pre_key_without(&base(c0), "");
        assert_eq!(k0, pre_key_without(&base(c0), ""));
        let cases: Vec<(&str, PreInputs)> = vec![
            (
                "fingerprint",
                PreInputs {
                    fingerprint: "g",
                    ..base(c0)
                },
            ),
            (
                "target",
                PreInputs {
                    target: "u",
                    ..base(c0)
                },
            ),
            (
                "switches",
                PreInputs {
                    switches: "z",
                    ..base(c0)
                },
            ),
            (
                "identity",
                PreInputs {
                    identity: "j",
                    ..base(c0)
                },
            ),
            (
                "base",
                PreInputs {
                    base: 2,
                    ..base(c0)
                },
            ),
            (
                "native",
                PreInputs {
                    native: true,
                    ..base(c0)
                },
            ),
            (
                "func",
                PreInputs {
                    func: b"other",
                    ..base(c0)
                },
            ),
            (
                "facts",
                PreInputs {
                    facts: b"other",
                    ..base(c0)
                },
            ),
            ("callees", base(leak(vec![]))),
            ("callees", base(leak(vec![(4, Some(callee))]))),
            ("callees", base(leak(vec![(3, None)]))),
            ("callees", base(leak(vec![(3, Some(other))]))),
        ];
        for (class, i) in &cases {
            assert_ne!(k0, pre_key_without(i, ""), "{class} is not in the key");
            // And the test's way of leaving one out does what it says.
            assert_eq!(
                pre_key_without(&base(c0), class),
                pre_key_without(i, class),
                "{class}"
            );
        }
    }

    /// The whole of what names code goes through the store, and back.
    #[test]
    fn an_entry_survives_the_store_and_garbage_does_not_parse() {
        let e = Pre::Code {
            clif: [7; 32],
            names: vec![
                Name::Func("n12".into()),
                Name::Data {
                    bytes: vec![1, 2],
                    align8: true,
                },
            ],
            stepped: stepped_counts(&["Call".to_string(), "Call".to_string(), "Cat".to_string()]),
        };
        let bytes = postcard::to_stdvec(&e).expect("entry");
        assert_eq!(postcard::from_bytes::<Pre>(&bytes).ok().as_ref(), Some(&e));
        assert!(postcard::from_bytes::<Pre>(&bytes[..3]).is_err());
        let Pre::Code { stepped, .. } = &e else {
            unreachable!()
        };
        assert_eq!(stepped_list(stepped), ["Call", "Call", "Cat"]);
    }

    /// Every environment switch the lowering and its shared analyses read is accounted for: a new
    /// one must go into [`KEYED`] or [`NO_PRE`], or here when it does not change the bytes.
    #[test]
    fn every_switch_of_the_lowering_is_accounted_for() {
        // The mode of the compare goes into the key by value (`pre_target`).
        let others = [
            "YAVS_INLCMP",
            "YAVS_IMAGE_INLINE",
            "YAVS_IMAGE_CHECK",
            "YAVS_CLIFT_TRACE",
            "YAVS_CLIFT_PERFMAP",
            "YAVS_CLIFT_HUGE",
            "YAVS_CLIFT_PREKEY_CHECK",
            "YAVS_CLIFT_PREKEY_DROP",
            "YAVS_LOG",
        ];
        for (file, text) in [
            ("lower.rs", include_str!("lower.rs")),
            ("cold.rs", include_str!("cold.rs")),
            ("cache.rs", include_str!("cache.rs")),
            ("lib.rs", include_str!("lib.rs")),
            ("rt.rs", include_str!("../../yavs-llvm/src/rt.rs")),
            ("regs.rs", include_str!("../../yavs-llvm/src/regs.rs")),
            ("native.rs", include_str!("../../yavs-llvm/src/native.rs")),
        ] {
            for (at, _) in text.match_indices("\"YAVS_") {
                let rest = &text[at + 1..];
                let name = &rest[..rest.find('"').expect("a closing quote")];
                // (This test's own pattern is the bare prefix.)
                assert!(
                    name == "YAVS_"
                        || KEYED.contains(&name)
                        || NO_PRE.contains(&name)
                        || others.contains(&name),
                    "{file}: `{name}` is not accounted for: does it change the machine code?"
                );
            }
        }
    }
}
