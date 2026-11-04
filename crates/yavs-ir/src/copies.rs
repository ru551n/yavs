//! Specialised copies of process bodies (docs/design/specialisation.md).
//!
//! A process function is shared by every instance of its design unit and reads the instance's
//! frozen data (generics, constants, descriptors) at run time. A copy is the same process
//! optimised for one such data: the optimiser folds the reads (`opt.rs` `Opt::inst`), so slice
//! bounds, widths and branches on generics are constants. Copies are content addressed (equal
//! data share one) and optimised when first asked for. A copy is a function of the program like
//! any other, so a runtime and a tier that exist already run it through the tables they have:
//! the elaborator reserves empty function slots at the end of the program
//! ([`Program::reserve_copies`], `Func::spare`) and a copy takes the next one. Which process
//! switches to its copy, and when, is the interpreter's business (`yavs-interp`).

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use crate::{AddrSpace, Func, FuncId, FuncKind, Inst, Program};

/// The data of a package once it is initialised: constants and the objects it made (addressed by
/// `AddrSpace::Pkg`), and a digest of it.
#[derive(Clone)]
pub struct PkgData {
    pub bytes: Arc<dyn AsRef<[u8]> + Send + Sync>,
    pub digest: u64,
}

/// Where a copy finds the data of the packages its process reads (`Program::copy_of_pkgs`).
pub trait PkgSource {
    /// The data of package `p`, or `None` when it is not known (a copy does not fold its reads).
    fn package(&self, p: u32) -> Option<PkgData>;
}

struct NoPkgs;

impl PkgSource for NoPkgs {
    fn package(&self, _: u32) -> Option<PkgData> {
        None
    }
}

/// The form the kernel keeps an instance signal in, fixed from the build of the simulation on
/// (`yavs-kernel` `tests/signal_forms.rs`): what the code of a copy may assume without looking.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct SigFact {
    /// `TierSig::codes`, `planes`, `stamps` and `last` are offered.
    pub codes: bool,
    pub planes: bool,
    pub stamps: bool,
    pub last: bool,
    /// The bytes of an element of a signal kept as bytes (`TierSig::es`), else 0.
    pub es: u8,
    /// The elements of the instance signal, which the table's `len` and `plen` cover.
    pub len: u32,
}

/// What holds for a process instance for the whole run and is not in its data: the facts of the
/// instance signals its body names, by `SigVal::sig`, in order. A signal with none has no
/// assumption made about it. Part of a copy's key: instances with equal facts share a body.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash)]
pub struct InstFacts {
    sigs: Box<[(u32, SigFact)]>,
}

impl InstFacts {
    pub fn new(mut sigs: Vec<(u32, SigFact)>) -> InstFacts {
        sigs.sort_unstable_by_key(|s| s.0);
        InstFacts { sigs: sigs.into() }
    }

    pub fn sig(&self, i: u32) -> Option<SigFact> {
        let at = self.sigs.binary_search_by_key(&i, |s| s.0).ok()?;
        Some(self.sigs[at].1)
    }
}

/// One copy: the process it is a copy of and the data it was made for (the parameters of `for`
/// generates, the `holes`, zeroed: they are not folded). Its function is the reserved slot of
/// the same index, whose `opt` holds the optimised body once made. `pkgs` is the data of the
/// packages the process's declarative part reads when the copy was asked for, and `digest`
/// names them (it is part of the key: a package made another way is another copy), as are
/// `facts`, which the IR does not depend on: they are for the code generators.
pub(crate) struct Copy {
    pub base: FuncId,
    pub data: Box<[u8]>,
    pub holes: Arc<[(u32, u32)]>,
    /// The byte ranges of the data the optimiser folded, once the body is made (`check_copy`).
    pub folded: OnceLock<Box<[(u32, u32)]>>,
    /// Against the cap: `.0` bodies for `.1` instances.
    pub share: (u64, u64),
    pub pkgs: Box<[(u32, PkgData)]>,
    pub digest: u64,
    pub facts: InstFacts,
}

/// An optimised copy as stored between runs (the stage cache, kind `spec`): the process, the
/// data it is for (masked, as `Program::copy_of` keys it), the digest of the package data it was
/// made with and the serialised function.
#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct CopyBlob {
    pub base: FuncId,
    #[serde(with = "crate::bytes")]
    pub data: Vec<u8>,
    /// What the data leave unfolded: part of the key, a zero there may be a value it did fold.
    pub holes: Vec<(u32, u32)>,
    pub folded: Vec<(u32, u32)>,
    pub digest: u64,
    pub bytes: yavs_diag::Slice,
}

/// Where a run finds the copies earlier runs of the same program stored: called at most once,
/// when the first copy is optimised.
pub type CopyLoader = Box<dyn FnOnce() -> Vec<CopyBlob> + Send + Sync>;

/// The copies of earlier runs: by hash of (process, data), with a flag for those used.
struct Stored {
    blobs: Vec<(CopyBlob, bool)>,
    by_hash: HashMap<u64, Vec<usize>>,
}

/// The copies made so far, and what limits them.
#[derive(Default)]
pub struct Copies {
    loader: Mutex<Option<CopyLoader>>,
    stored: OnceLock<Mutex<Stored>>,
    /// Copies optimised in this run, and copies read from `stored`.
    fresh: AtomicUsize,
    loaded: AtomicUsize,
    /// The id of the first reserved slot; a copy's id is that plus its index in `list`.
    first: u32,
    list: Box<[OnceLock<Copy>]>,
    count: AtomicUsize,
    /// Copies by a hash of (process, masked data); equal means the same bytes, checked.
    by_hash: Mutex<HashMap<u64, Vec<FuncId>>>,
    /// Which processes can have copies (`Program::specialisable`).
    suitable: Mutex<HashMap<FuncId, bool>>,
    /// The packages the declarative part of each process reads (`Program::pkg_deps`).
    deps: Mutex<HashMap<FuncId, Arc<[u32]>>>,
    /// The instance signals each process names (`Program::sig_uses`).
    sig_uses: Mutex<HashMap<FuncId, Arc<[u32]>>>,
    /// What elaboration knows of the instances of each process (`Program::note_instance`).
    kin: Mutex<HashMap<FuncId, Kin>>,
    shared_ir: AtomicU64,
    copy_ir: AtomicU64,
    charged: AtomicU64,
    capped: AtomicU64,
    full: AtomicU64,
    switched: AtomicU64,
    planned: AtomicU64,
}

impl Clone for Copies {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl std::fmt::Debug for Copies {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Copies({})", self.count.load(Ordering::Relaxed))
    }
}

/// What the copies cost: counted in optimised IR instructions.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct CopyStats {
    /// Copies made.
    pub copies: usize,
    /// Optimised IR instructions of the process functions that ran (the shared code).
    pub shared_ir: u64,
    /// Optimised IR instructions of the copies.
    pub copy_ir: u64,
    /// What they count against the cap: a copy's share per instance it serves.
    pub charged: u64,
    /// Copies refused because they would exceed [`cap_percent`] of `shared_ir`.
    pub capped: u64,
    /// Copies refused because every reserved slot was taken.
    pub full: u64,
    /// Slots reserved ([`Program::reserve_copies`]).
    pub reserved: usize,
    /// Processes that switched to a copy ([`Program::note_switch`]).
    pub switched: u64,
    /// Processes that were to try the switch ([`Program::note_planned`]).
    pub planned: u64,
    /// Copies optimised in this run, and copies of earlier runs read from the stage cache.
    pub fresh: usize,
    pub loaded: usize,
}

/// Why no copy was made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NoCopy {
    /// The process cannot have copies (`Program::specialisable`).
    Unsuitable,
    /// The copies would exceed the growth cap.
    Cap,
    /// Every reserved slot holds a copy.
    Full,
}

/// The growth cap: the optimised IR of all copies stays within this percent of the shared
/// process code (`YAVS_SPEC_CAP`, default 250, 0: no cap).
pub fn cap_percent() -> u64 {
    static CAP: OnceLock<u64> = OnceLock::new();
    *CAP.get_or_init(|| {
        std::env::var("YAVS_SPEC_CAP")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(250)
    })
}

fn key_hash(base: FuncId, data: &[u8], holes: &[(u32, u32)], digest: u64) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    base.hash(&mut h);
    data.hash(&mut h);
    holes.hash(&mut h);
    digest.hash(&mut h);
    h.finish()
}

/// The instances of one process, and the holes all its copies get once worked out.
struct Kin {
    scalars: Arc<[(u32, u32)]>,
    insts: Vec<Arc<dyn AsRef<[u8]> + Send + Sync>>,
    plan: Option<Plan>,
}

/// What a process's copies leave out of their key, and how many instances one copy serves on
/// average: `bodies` for `instances`.
#[derive(Clone)]
struct Plan {
    holes: Arc<[(u32, u32)]>,
    bodies: u64,
    instances: u64,
}

fn masked(data: &[u8], holes: &[(u32, u32)]) -> Box<[u8]> {
    let mut d = data.to_vec();
    for &(o, n) in holes {
        // A hole outside the data is a producer bug, not something to skip.
        let Some(b) = d.get_mut(o as usize..o as usize + n as usize) else {
            panic!(
                "internal error: a generate parameter at {o}+{n} lies outside the {} bytes of \
                 instance data; this is a bug in yavs",
                data.len()
            );
        };
        b.fill(0);
    }
    d.into()
}

impl Copies {
    pub(crate) fn get(&self, f: FuncId) -> Option<&Copy> {
        let i = f.0.checked_sub(self.first)? as usize;
        self.list.get(i)?.get()
    }

    /// Account the optimised shared body of a process (`Program::optimized`).
    pub(crate) fn shared(&self, ir: usize) {
        self.shared_ir.fetch_add(ir as u64, Ordering::Relaxed);
    }

    /// Account an optimised copy, which serves `instances` for each of `bodies`.
    pub(crate) fn made(&self, ir: usize, (bodies, instances): (u64, u64)) {
        self.copy_ir.fetch_add(ir as u64, Ordering::Relaxed);
        self.charged
            .fetch_add(ir as u64 * bodies / instances, Ordering::Relaxed);
    }

    /// The stored function of the copy (`base`, masked `data`, package `digest`) if an earlier
    /// run made it.
    pub(crate) fn take_stored(
        &self,
        base: FuncId,
        data: &[u8],
        holes: &[(u32, u32)],
        digest: u64,
    ) -> Option<(yavs_diag::Slice, Vec<(u32, u32)>)> {
        let st = self.stored.get_or_init(|| {
            let blobs = self
                .loader
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .take()
                .map_or_else(Vec::new, |load| load());
            let mut by_hash: HashMap<u64, Vec<usize>> = HashMap::new();
            for (i, b) in blobs.iter().enumerate() {
                by_hash
                    .entry(key_hash(
                        b.base,
                        &b.data,
                        if dropped("blob") { &[] } else { &b.holes },
                        b.digest,
                    ))
                    .or_default()
                    .push(i);
            }
            Mutex::new(Stored {
                blobs: blobs.into_iter().map(|b| (b, false)).collect(),
                by_hash,
            })
        });
        let mut st = st.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
        let i = st
            .by_hash
            .get(&key_hash(
                base,
                data,
                if dropped("blob") { &[] } else { holes },
                digest,
            ))?
            .iter()
            .copied()
            .find(|&i| {
                let b = &st.blobs[i].0;
                b.base == base
                    && b.data == data
                    && (b.holes == holes || dropped("blob"))
                    && b.digest == digest
            })?;
        st.blobs[i].1 = true;
        self.loaded.fetch_add(1, Ordering::Relaxed);
        Some((st.blobs[i].0.bytes.clone(), st.blobs[i].0.folded.clone()))
    }

    pub fn stats(&self) -> CopyStats {
        CopyStats {
            copies: self.count.load(Ordering::Relaxed),
            shared_ir: self.shared_ir.load(Ordering::Relaxed),
            copy_ir: self.copy_ir.load(Ordering::Relaxed),
            charged: self.charged.load(Ordering::Relaxed),
            capped: self.capped.load(Ordering::Relaxed),
            full: self.full.load(Ordering::Relaxed),
            reserved: self.list.len(),
            switched: self.switched.load(Ordering::Relaxed),
            planned: self.planned.load(Ordering::Relaxed),
            fresh: self.fresh.load(Ordering::Relaxed),
            loaded: self.loaded.load(Ordering::Relaxed),
        }
    }
}

impl Program {
    /// Can process `base` have copies: a process that has one wait statement (the resume point
    /// of a suspended process maps to the copy's one, `Machine::specialise`) and reads instance
    /// data; and the optimiser's specialisation is on (`opt::specialise_on`). Subprograms nested
    /// in the process and shared state (files, the shared heap) do not matter: the copy keeps the
    /// frame layout, so a nested subprogram reaches its variables, and the optimiser assumes
    /// nothing about frame bytes they write (`opt/decl.rs`).
    pub fn specialisable(&self, base: FuncId) -> bool {
        if !crate::opt::specialise_on() {
            return false;
        }
        let mut s = self
            .copies
            .suitable
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *s.entry(base).or_insert_with(|| {
            let Some(raw) = self.funcs.get(base).map(Func::full) else {
                return false;
            };
            raw.kind == FuncKind::Process
                && self.single_suspend(base)
                // (Test aid: a process that reads no instance data gains facts only, which the
                // checks of compiled code want to see on any design.)
                && (std::env::var_os("YAVS_SPEC_ALL").is_some()
                    || raw.code.iter().any(|i| {
                        matches!(
                            i,
                            Inst::AddrOf {
                                space: AddrSpace::Inst,
                                ..
                            }
                        )
                    }))
        })
    }

    /// The copy of process `base` for the instance data `data`, made if no earlier call asked
    /// for the same: the id of a function that runs like `base` on an instance whose data is
    /// `data`, up to the parameters of `for` generates at `holes` (offset, bytes), which it
    /// reads from the data like `base` does. Its body is optimised when [`Program::func`]
    /// first hands it out.
    pub fn copy_of(
        &self,
        base: FuncId,
        data: &[u8],
        holes: &[(u32, u32)],
    ) -> Result<FuncId, NoCopy> {
        self.copy_of_pkgs(base, data, holes, &NoPkgs, InstFacts::default())
    }

    /// The instance signals (`SigVal::sig`) the lowered code of process `base` names: the ones
    /// whose facts can matter to its copies, so others do not split them.
    pub fn sig_uses(&self, base: FuncId) -> Arc<[u32]> {
        let mut m = self
            .copies
            .sig_uses
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        m.entry(base)
            .or_insert_with(|| {
                let Some(raw) = self.funcs.get(base).map(Func::full) else {
                    return Arc::new([]);
                };
                let mut v: Vec<u32> = raw
                    .code
                    .iter()
                    .filter_map(|i| match *i {
                        Inst::SigVal { sig, .. } => Some(sig),
                        _ => None,
                    })
                    .collect();
                v.sort_unstable();
                v.dedup();
                v.into()
            })
            .clone()
    }

    /// The facts the code of function `f` may assume: those of a copy, none for anything else.
    pub fn facts(&self, f: FuncId) -> Option<&InstFacts> {
        self.copies.get(f).map(|c| &c.facts)
    }

    /// The packages (`AddrSpace::Pkg` numbers) whose data the declarative part of process
    /// `base` reads.
    pub fn pkg_deps(&self, base: FuncId) -> Arc<[u32]> {
        let mut m = self
            .copies
            .deps
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        m.entry(base)
            .or_insert_with(|| {
                let Some(raw) = self.funcs.get(base).map(Func::full) else {
                    return Arc::new([]);
                };
                let end = raw
                    .body
                    .and_then(|b| raw.blocks.get(b))
                    .map_or(0, |b| b.start as usize);
                let mut v: Vec<u32> = raw.code[..end.min(raw.code.len())]
                    .iter()
                    .filter_map(|i| match *i {
                        Inst::AddrOf {
                            space: AddrSpace::Pkg(p),
                            ..
                        } => Some(p),
                        _ => None,
                    })
                    .collect();
                v.sort_unstable();
                v.dedup();
                v.into()
            })
            .clone()
    }

    /// [`Program::copy_of`] with the data of the packages the process's declarative part
    /// reads (`pkg_deps`) taken from `pkgs`: the copy folds what that part makes of their
    /// constants, and a digest of them is part of its key, so a run whose packages came out
    /// otherwise (an initialiser that read the environment) makes, and reads from the stage
    /// cache, another copy. Call it once package initialisation is over. `facts` are the
    /// instance's: other facts are another copy of the same optimised IR.
    pub fn copy_of_pkgs(
        &self,
        base: FuncId,
        data: &[u8],
        holes: &[(u32, u32)],
        pkgs: &dyn PkgSource,
        facts: InstFacts,
    ) -> Result<FuncId, NoCopy> {
        if !self.specialisable(base) {
            return Err(NoCopy::Unsuitable);
        }
        let c = &self.copies;
        let raw = data;
        let gen_holes = holes;
        let plan = self.holes_of(base, holes);
        let (holes, share) = (plan.holes, (plan.bodies, plan.instances));
        let data = masked(raw, &holes);
        let pkgs: Box<[(u32, PkgData)]> = self
            .pkg_deps(base)
            .iter()
            .filter_map(|&p| Some((p, pkgs.package(p)?)))
            .collect();
        let digest = {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            for (p, d) in &*pkgs {
                (p, d.digest).hash(&mut h);
            }
            h.finish()
        };
        let h = key_hash(base, &data, &holes, digest);
        let mut by = c
            .by_hash
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if let Some(&id) = by.get(&h).and_then(|v| {
            v.iter().find(|&&id| {
                c.get(id).is_some_and(|x| {
                    x.base == base
                        && ((x.data == data && x.holes == holes) || dropped("key"))
                        && x.digest == digest
                        && x.facts == facts
                })
            })
        }) {
            return Ok(id);
        }
        let n = c.count.load(Ordering::Relaxed);
        if n >= c.list.len() {
            c.full.fetch_add(1, Ordering::Relaxed);
            return Err(NoCopy::Full);
        }
        let cap = cap_percent();
        if cap != 0 {
            // A new copy is about as big as the shared body it replaces, and what it costs is
            // shared by the instances it serves (a design of 1000 instances and 16 widths).
            let est = self.func(base).map_or(0, |f| f.code.len()) as u64 * share.0 / share.1;
            let st = c.stats();
            if (st.charged + est) * 100 > st.shared_ir * cap {
                c.capped.fetch_add(1, Ordering::Relaxed);
                return Err(NoCopy::Cap);
            }
        }
        let id = FuncId(c.first + n as u32);
        let (data, holes) = if dropped("key") {
            (masked(raw, gen_holes), gen_holes.into())
        } else {
            (data, holes)
        };
        let set = c.list[n].set(Copy {
            base,
            data,
            holes,
            folded: OnceLock::new(),
            share,
            pkgs,
            digest,
            facts,
        });
        assert!(
            set.is_ok(),
            "internal error: a copy slot was taken twice; this is a bug in yavs"
        );
        c.count.store(n + 1, Ordering::Relaxed);
        by.entry(h).or_default().push(id);
        Ok(id)
    }

    /// The process a function is a copy of, else the function itself.
    pub fn base_of(&self, f: FuncId) -> FuncId {
        self.copies.get(f).map_or(f, |c| c.base)
    }

    /// A number that names the copy `f` by what it is for, the same in every run: a hash of its
    /// process and masked instance data (`None` for any other function). Copy ids depend on
    /// which process switched first, so compiled code is named and keyed by this.
    pub fn copy_tag(&self, f: FuncId) -> Option<u64> {
        self.copies.get(f).map(|c| {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            key_hash(c.base, &c.data, &c.holes, c.digest).hash(&mut h);
            c.facts.hash(&mut h);
            h.finish()
        })
    }

    /// The function's lowered code and header: that of the process for a copy.
    pub fn raw(&self, f: FuncId) -> Option<&Func> {
        self.funcs.get(self.base_of(f)).map(Func::full)
    }

    /// Where to find the copies earlier runs of this program stored, read when the first copy is
    /// optimised (`CopyBlob`; only when copies are portable between runs).
    pub fn set_copy_loader(&self, load: CopyLoader) {
        if crate::opt::copies_are_portable() {
            *self
                .copies
                .loader
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner) = Some(load);
        }
    }

    /// How many copies this run optimised: a stored set that does not hold them is out of date.
    pub fn fresh_copies(&self) -> usize {
        self.copies.fresh.load(Ordering::Relaxed)
    }

    /// The copies made so far and the stored ones not used, serialised, for the next run. Empty
    /// when copies are not portable.
    pub fn export_copies(&self) -> Vec<CopyBlob> {
        if !crate::opt::copies_are_portable() {
            return Vec::new();
        }
        let c = &self.copies;
        // (Copies that differ in their facts alone are one stored function.)
        let mut seen = std::collections::HashSet::new();
        let mut out: Vec<CopyBlob> = (0..c.count.load(Ordering::Relaxed))
            .filter_map(|n| {
                let e = c.list.get(n)?.get()?;
                let Some(Some(o)) = self.funcs.get(FuncId(c.first + n as u32))?.opt.get() else {
                    return None;
                };
                if !seen.insert((e.base, &e.data, &e.holes, e.digest)) {
                    return None;
                }
                let bytes = postcard::to_stdvec(&**o).ok()?;
                Some(CopyBlob {
                    base: e.base,
                    data: e.data.to_vec(),
                    holes: e.holes.to_vec(),
                    folded: e.folded.get().map_or_else(Vec::new, |f| f.to_vec()),
                    digest: e.digest,
                    bytes: bytes.into(),
                })
            })
            .collect();
        if let Some(st) = c.stored.get() {
            let st = st.lock().unwrap_or_else(std::sync::PoisonError::into_inner);
            type Key = (FuncId, Vec<u8>, Vec<(u32, u32)>, u64);
            let have: std::collections::HashSet<Key> = out
                .iter()
                .map(|b| (b.base, b.data.clone(), b.holes.clone(), b.digest))
                .collect();
            out.extend(
                st.blobs
                    .iter()
                    .filter(|(b, _)| {
                        !have.contains(&(b.base, b.data.clone(), b.holes.clone(), b.digest))
                    })
                    .map(|(b, _)| b.clone()),
            );
        }
        out
    }

    /// A process is to try the switch to a copy (for [`CopyStats::planned`]).
    pub fn note_planned(&self) {
        self.copies.planned.fetch_add(1, Ordering::Relaxed);
    }

    /// A process switched to a copy (for [`CopyStats::switched`]).
    pub fn note_switch(&self) {
        self.copies.switched.fetch_add(1, Ordering::Relaxed);
    }

    /// What the copies made so far cost.
    pub fn copy_stats(&self) -> CopyStats {
        self.copies.stats()
    }
}

impl Program {
    /// Reserve `n` empty function slots at the end of the program for copies (once, before a
    /// runtime is made from it: the runtime's tables then have a place for each copy). Nothing
    /// when copies are off (`opt::specialise_on`).
    pub fn reserve_copies(&mut self, n: usize) {
        if n == 0 || !crate::opt::specialise_on() || !self.copies.list.is_empty() {
            return;
        }
        let first = self.funcs.len();
        self.funcs.reserve_exact(n);
        for _ in 0..n {
            self.funcs.push(Func::spare());
        }
        self.copies.first = first as u32;
        self.copies.list = (0..n).map(|_| OnceLock::new()).collect();
    }

    /// The optimised body of copy `c`: an earlier run's if the stage cache had it, else made now.
    pub(crate) fn optimize_copy_of(&self, c: &Copy) -> Option<Box<Func>> {
        let stored = crate::opt::copies_are_portable()
            .then(|| self.copies.take_stored(c.base, &c.data, &c.holes, c.digest))
            .flatten()
            .and_then(|(b, folded)| {
                let f = postcard::from_bytes::<Func>(b.get()).ok()?;
                Some((Box::new(f), folded))
            });
        let (o, folded) = match stored {
            Some((f, folded)) => (Some(f), folded),
            None => {
                self.copies.fresh.fetch_add(1, Ordering::Relaxed);
                let pkgs: Vec<(u32, &[u8])> = c
                    .pkgs
                    .iter()
                    .map(|(p, d)| (*p, (*d.bytes).as_ref()))
                    .collect();
                crate::opt::optimize_copy(self, c.base, &c.data, &c.holes, &pkgs)
            }
        };
        let _ = c.folded.set(folded.into());
        // A pass that left the process alone: the copy is the lowered code.
        let o = o.or_else(|| self.funcs.get(c.base).map(|f| Box::new(f.full().clone())))?;
        self.copies.made(o.code.len(), c.share);
        Some(o)
    }
}

impl Program {
    /// Elaboration tells which instance data process `base` runs on, and which byte ranges of them
    /// are scalar values (generics, constants): the instances decide what the key of the copies
    /// leaves out, so that is the same for every instance whichever switches first.
    pub fn note_instance(
        &self,
        base: FuncId,
        scalars: &Arc<[(u32, u32)]>,
        data: Arc<dyn AsRef<[u8]> + Send + Sync>,
    ) {
        let mut m = self
            .copies
            .kin
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let k = m.entry(base).or_insert_with(|| Kin {
            scalars: scalars.clone(),
            insts: Vec::new(),
            plan: None,
        });
        // (The processes of one region come in a row and share its data.)
        if !k.insts.last().is_some_and(|l| Arc::ptr_eq(l, &data)) {
            k.insts.push(data);
        }
    }

    /// The holes of every copy of process `base`: `gen`, the parameters of `for` generates, and
    /// what [`crate::shape::classify`] says the key need not hold.
    fn holes_of(&self, base: FuncId, gen_holes: &[(u32, u32)]) -> Plan {
        let mode = crate::shape::mode();
        let mut m = self
            .copies
            .kin
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let Some(k) = m.get_mut(&base).filter(|_| mode != crate::shape::Mode::Off) else {
            return Plan {
                holes: gen_holes.into(),
                bodies: 1,
                instances: 1,
            };
        };
        if let Some(p) = &k.plan {
            return p.clone();
        }
        let raw = self.funcs.get(base).map(Func::full);
        let (holes, bodies) = raw.map_or_else(
            || (gen_holes.to_vec(), 1),
            |raw| crate::shape::plan(self, raw, gen_holes, &k.scalars, &k.insts, mode),
        );
        let p = Plan {
            holes: holes.into(),
            bodies: bodies as u64,
            instances: k.insts.len().max(bodies).max(1) as u64,
        };
        k.plan = Some(p.clone());
        k.insts = Vec::new();
        p
    }

    /// The internal error of a copy made of bytes that are not those of the instance switching to
    /// it: a folded byte the key left out (`YAVS_SPEC_CHECK`), or a folded byte in a hole.
    pub fn check_copy(&self, id: FuncId, data: &[u8]) -> Result<(), String> {
        let Some(c) = self.copies.get(id) else {
            return Ok(());
        };
        let Some(folded) = c.folded.get() else {
            return Err(format!("the copy f{} is not made yet", id.0));
        };
        for &(o, n) in &**folded {
            let (b, e) = (o as usize, o as usize + n as usize);
            if c.holes.iter().any(|&(h, z)| h < o + n && o < h + z) {
                return Err(format!(
                    "a copy of process f{} folded bytes {b}..{e} of the instance data, which its \
                     key leaves out",
                    c.base.0
                ));
            }
            if data.get(b..e) != c.data.get(b..e) {
                return Err(format!(
                    "a copy of process f{} folded bytes {b}..{e} of the instance data as {:?}, \
                     and this instance holds {:?}",
                    c.base.0,
                    c.data.get(b..e),
                    data.get(b..e)
                ));
            }
        }
        Ok(())
    }
}

/// `YAVS_SHARE_KEY_DROP` (debug builds, the test of `check_copy`): `fold`, `key` or `blob`, the
/// input a copy is made from as if the key had forgotten it.
pub(crate) fn dropped(what: &str) -> bool {
    static DROP: OnceLock<String> = OnceLock::new();
    cfg!(debug_assertions)
        && DROP.get_or_init(|| std::env::var("YAVS_SHARE_KEY_DROP").unwrap_or_default()) == what
}

/// `check_copy` at each switch: on in debug builds, else with `YAVS_SPEC_CHECK=1`.
pub fn copy_check_on() -> bool {
    static ON: OnceLock<bool> = OnceLock::new();
    *ON.get_or_init(|| match std::env::var("YAVS_SPEC_CHECK") {
        Ok(v) => !v.is_empty() && v != "0",
        Err(_) => cfg!(debug_assertions),
    })
}
