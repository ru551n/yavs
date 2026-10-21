//! Elaboration-time calls of functions that use package state (#77, LRM 14.4): impure
//! functions that read or update shared variables of packages (`get_logger`-style
//! registries), called from generic maps, constant and signal declarations.
//!
//! The packages with run-time objects are elaborated first (LRM 14.4.1, `init_package`), and
//! every such call runs against their state in elaboration order, so each call sees the side
//! effects of those before it. The calls are logged with their argument values; building the
//! simulation initialises the packages again and replays the log, so the processes start with
//! the same package state and `build` stays repeatable (rule 4). A replayed call that returns
//! a different value is an internal error.
//!
//! A function called for the first time is lowered into the session's program (the lowerer
//! is kept, `Lowerer::suspend`), and the runtime moves to the grown program with the package
//! state as it is (`Runtime::extend`, #383). Only `Session::undo`, a failed lowering or a
//! change of the run-time packages lower everything again, initialise the packages and replay
//! the log.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use yavs_diag::Diagnostic;
use yavs_interp::{Bindings, CallVal, InstData, Runtime, SharedState, Sources};
use yavs_ir::{FuncId, Program, TyId};
use yavs_kernel::SimError;
use yavs_lower::{LowerOptions, Lowerer, PackageIr};
use yavs_sem::{Db, DeclId, UnitId};

use crate::eval::Val;

/// A function lowered into a program, with the IR types of its parameters and result.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(crate) struct Sig {
    pub(crate) func: FuncId,
    pub(crate) params: Vec<TyId>,
    pub(crate) ret: Option<TyId>,
}

/// Lower function `f` (and what it calls) with `l`.
pub(crate) fn sig(l: &mut Lowerer, db: &Db, f: DeclId) -> Result<Sig, Vec<Diagnostic>> {
    let func = l.subprogram(f)?;
    let sp = db.decls[f].kind.subprogram();
    let mut params = Vec::new();
    for p in sp.map(|s| &s.params[..]).unwrap_or(&[]) {
        params.push(l.ty(db.base(p.subtype))?);
    }
    // A named return value: the result subtype's array comes last (`Evaluator::call`).
    if let Some(s) = sp
        && let yavs_sem::decl::Body::Hir { scope, .. } = &s.body
        && db.ret_names.contains_key(scope)
        && let Some(r) = s.ret
    {
        params.push(l.ty(db.base(r))?);
    }
    let ret = match sp.and_then(|s| s.ret) {
        Some(r) => Some(l.ty(db.base(r))?),
        None => None,
    };
    Ok(Sig { func, params, ret })
}

/// Why a call did not give a value.
pub(crate) enum Fail {
    /// Lowering the function failed (first diagnostic's message).
    Lower(String),
    /// A value the call cannot take or return yet.
    Unsupported(&'static str),
    /// The call failed at run time.
    Runtime(String),
}

fn runtime(e: SimError) -> Fail {
    Fail::Runtime(match &e.kind {
        yavs_kernel::ErrorKind::Runtime(m) => m.clone(),
        k => k.to_string(),
    })
}

/// Call `s` with `args`, with access to `shared` (shared variables and the shared heap).
pub(crate) fn run(
    rt: &Arc<Runtime>,
    s: &Sig,
    args: &[Val],
    shared: Option<&Arc<Mutex<SharedState>>>,
) -> Result<Val, Fail> {
    let prog = rt.prog.clone();
    let mut cvs = Vec::with_capacity(args.len());
    for (v, &t) in args.iter().zip(&s.params) {
        let scalar = prog.types.get(t).is_some_and(|t| t.scalar_size().is_some());
        cvs.push(match v {
            // A scalar named return value's range (`Evaluator::call`): a descriptor only.
            Val::Array(a) if scalar => CallVal::Composite {
                data: Vec::new(),
                desc: crate::value::dope(&a.bounds),
            },
            _ => crate::value::encode(&prog, t, v)
                .ok_or(Fail::Unsupported("an argument value of this type"))?,
        });
    }
    let ret = s.ret.ok_or(Fail::Unsupported("not a function"))?;
    let composite = prog
        .types
        .get(ret)
        .is_some_and(|t| t.scalar_size().is_none());
    let r = yavs_interp::call_values_shared(
        rt.clone(),
        s.func,
        Arc::new(InstData::default()),
        &cvs,
        composite.then_some(ret),
        shared,
    )
    .map_err(runtime)?
    .ok_or(Fail::Unsupported("no result"))?;
    crate::value::decode(&prog, ret, &r).ok_or(Fail::Unsupported("a result of this type"))
}

/// Where the package state after initialisation is kept (#407, stage 2 step 2): next to the lowered
/// packages (`package_lowerer`) in the stage cache, for the same packages, options and `Db`; and
/// the state of the packages of a library prefix, which every closure of it shares.
#[derive(Clone, Debug)]
pub(crate) struct InitKey {
    cache: Arc<dyn yavs_sem::StageCache>,
    extent: Vec<usize>,
    part: Vec<u8>,
    lib: Option<LibInit>,
}

/// The state of the first `packages` packages of the design: those the library prefix lowered
/// (`libstage`).
#[derive(Clone)]
struct LibInit {
    cache: Arc<dyn yavs_sem::StageCache>,
    level: usize,
    part: Vec<u8>,
    packages: usize,
    /// Bytes of the shared variables area those packages use.
    shared: u32,
}

impl std::fmt::Debug for LibInit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "LibInit({} packages)", self.packages)
    }
}

impl InitKey {
    fn part(&self, resolution: yavs_std::arith::TimeResolution) -> Vec<u8> {
        let mut p = self.part.clone();
        p.push(resolution as u8);
        p
    }

    fn get(&self, resolution: yavs_std::arith::TimeResolution) -> Option<yavs_diag::Buffer> {
        self.cache.get(&self.extent, "init", &self.part(resolution))
    }

    fn put(&self, resolution: yavs_std::arith::TimeResolution, bytes: &[u8]) {
        self.cache
            .put(&self.extent, "init", &self.part(resolution), bytes);
    }
}

impl LibInit {
    /// The key part for the state of the first `n` packages (the session's initialisation caches
    /// them all, a simulation start those before the first with a descriptor to write).
    fn part(&self, resolution: yavs_std::arith::TimeResolution, n: usize) -> Vec<u8> {
        let mut p = self.part.clone();
        p.push(resolution as u8);
        p.extend((n as u32).to_le_bytes());
        p
    }
}

/// Initialise `packages` of `prog` in order (each sees those before it, LRM 14.4.2) with
/// `shared`, `fix` completing each package's data, and return the runtime with all of them
/// bound. With `start`, the packages' data is a copy of it instead (initialised already, by
/// the elaboration session, [`Start`]). With `cache` (and no `start`), the state the
/// initialisers made is read from the stage cache and they do not run; a cold run stores it
/// unless they did more than compute. `fix` must then change nothing the initialisers read.
#[allow(clippy::too_many_arguments)] // ponytail: one more setting; bundle into a struct when the next one comes
pub(crate) fn init_packages(
    prog: &Arc<Program>,
    cov_counters: usize,
    packages: &[PackageIr],
    sources: &Sources,
    foreign: &[(yavs_ir::ForeignId, yavs_ffi::ForeignFn)],
    searched: &str,
    models: &[(yavs_ir::ForeignId, Arc<dyn yavs_interp::ForeignModel>)],
    shared: &Arc<Mutex<SharedState>>,
    resolution: yavs_std::arith::TimeResolution,
    revision: yavs_std::types::Revision,
    start: Option<&[InstData]>,
    cache: Option<&InitKey>,
    fix_writes: bool,
    mut fix: impl FnMut(&PackageIr, &mut InstData) -> Result<(), SimError>,
) -> Result<Arc<Runtime>, SimError> {
    let mut data: Vec<Arc<InstData>> = Vec::new();
    let bind = |data: &Vec<Arc<InstData>>| Bindings {
        packages: data.clone(),
        sources: sources.clone(),
        foreign: foreign.to_vec(),
        models: models.to_vec(),
        searched: searched.to_string(),
        resolution,
        revision,
        cov_counters: Some(cov_counters),
        ..Default::default()
    };
    if start.is_some_and(|s| s.len() != packages.len()) {
        return Err(SimError::new(yavs_kernel::ErrorKind::Internal(
            "the elaboration session has other packages".into(),
        )));
    }
    let cache = cache.filter(|_| start.is_none() && !packages.is_empty());
    // The descriptors `fix` writes (`fix_writes`) are read by the initialisers that follow (a
    // package array signal's bounds): the state is cached only up to the packages without any.
    let dope = |ps: &[PackageIr]| {
        fix_writes
            && ps
                .iter()
                .any(|p| p.signals.iter().any(|s| s.dope.is_some()))
    };
    let whole = cache.filter(|_| !dope(packages));
    if let Some(k) = whole
        && let Some(bytes) = k.get(resolution)
        && let vars = shared.lock().map_or(0, |s| s.vars.len())
        && let Some((d, st)) = yavs_interp::restore_state(&bytes, packages.len(), vars)
    {
        st.replay_init_reports();
        *shared.lock().map_err(|_| {
            SimError::new(yavs_kernel::ErrorKind::Internal(
                "shared state poisoned".into(),
            ))
        })? = st;
        let data: Vec<Arc<InstData>> = d.into_iter().map(Arc::new).collect();
        return Runtime::new(prog.clone(), bind(&data));
    }
    // The packages of the library prefix up to the first with such a signal: read back, or made
    // now and stored for the next closure.
    let lib = cache.and_then(|k| k.lib.as_ref()).and_then(|l| {
        let n = packages[..l.packages.min(packages.len())]
            .iter()
            .position(|p| dope(std::slice::from_ref(p)))
            .unwrap_or(l.packages.min(packages.len()));
        (n > 0).then_some((l, n))
    });
    let mut first = 0;
    if let Some((l, n)) = lib
        && let Some(bytes) =
            l.cache
                .prefix_get(l.level, crate::libstage::INIT, &l.part(resolution, n))
        && let total = shared.lock().map_or(0, |s| s.vars.len())
        && let Some((d, st)) =
            yavs_interp::restore_state_prefix(&bytes, n, l.shared as usize, total)
    {
        st.replay_init_reports();
        *shared.lock().map_err(|_| {
            SimError::new(yavs_kernel::ErrorKind::Internal(
                "shared state poisoned".into(),
            ))
        })? = st;
        data = d.into_iter().map(Arc::new).collect();
        first = n;
    }
    for (i, p) in packages.iter().enumerate().skip(first) {
        let mut d = match start {
            Some(s) => s[i].clone(),
            None => {
                let rt = Runtime::new(prog.clone(), bind(&data))?;
                yavs_interp::init_package(rt, p.init, p.index, p.size as usize, Some(shared))?
            }
        };
        fix(p, &mut d)?;
        data.push(Arc::new(d));
        if let Some((l, n)) = lib
            && first == 0
            && data.len() == n
            && let Ok(s) = shared.lock()
        {
            match s.state_image_prefix(&data, l.shared as usize) {
                Ok(image) => {
                    l.cache.prefix_put(
                        l.level,
                        crate::libstage::INIT,
                        &l.part(resolution, n),
                        &image,
                    );
                }
                Err(why) => l.cache.note(&format!("no library init entry: {why}")),
            }
        }
    }
    if let Some(k) = whole
        && let Ok(s) = shared.lock()
    {
        match s.state_image(&data) {
            Ok(image) => k.put(resolution, &image),
            Err(why) => k.cache.note(&format!("no init entry: {why}")),
        }
    }
    Runtime::new(prog.clone(), bind(&data))
}

/// The std/ieee revision the design runs against (INTEGER bounds, FILE_OPEN_KIND, `vhdl_version`).
pub(crate) fn revision_of(db: &Db) -> yavs_std::types::Revision {
    match db.rev {
        yavs_syntax::Std::Vhdl2019 => yavs_std::types::Revision::Vhdl2019,
        yavs_syntax::Std::Vhdl2008 => yavs_std::types::Revision::Vhdl2008,
    }
}

/// One call that ran with package state during elaboration, in elaboration order.
#[derive(Clone, Debug)]
pub struct Call {
    pub(crate) func: DeclId,
    /// The function's name, for messages.
    pub(crate) name: String,
    pub(crate) args: Vec<Val>,
    pub(crate) result: Val,
}

/// Run `log` again, in order, on freshly initialised packages: the side effects of
/// elaboration. Every call must return what it returned during elaboration.
pub(crate) fn replay(
    rt: &Arc<Runtime>,
    shared: &Arc<Mutex<SharedState>>,
    sigs: &HashMap<DeclId, Sig>,
    log: &[Call],
) -> Result<(), String> {
    // The reports of these calls were printed when they first ran (#390).
    rt.set_replay(true);
    let r = replay_calls(rt, shared, sigs, log);
    rt.set_replay(false);
    r
}

fn replay_calls(
    rt: &Arc<Runtime>,
    shared: &Arc<Mutex<SharedState>>,
    sigs: &HashMap<DeclId, Sig>,
    log: &[Call],
) -> Result<(), String> {
    for c in log {
        let name = &c.name;
        let s = sigs
            .get(&c.func)
            .ok_or_else(|| format!("internal error: `{name}` is not lowered for the replay"))?;
        match run(rt, s, &c.args, Some(shared)) {
            Ok(v) if v == c.result => {}
            Ok(_) => {
                return Err(format!(
                    "internal error: `{name}` returned another value when its elaboration-time \
                     call was replayed"
                ));
            }
            Err(Fail::Runtime(m)) => return Err(m),
            Err(Fail::Lower(m)) => return Err(m),
            Err(Fail::Unsupported(w)) => {
                return Err(format!("internal error: replaying `{name}`: {w}"));
            }
        }
    }
    Ok(())
}

/// The program and package state elaboration-time calls run against.
struct Live {
    rt: Arc<Runtime>,
    shared: Arc<Mutex<SharedState>>,
    sigs: HashMap<DeclId, Sig>,
    /// The lowering that made `rt.prog`, with the run-time packages it lowered, to add
    /// functions to; `None` after a failed lowering.
    lowerer: Option<(yavs_lower::Suspended, Vec<UnitId>)>,
    shared_size: u32,
    /// The library prefix the program starts with (`libstage`).
    lib: Option<crate::libstage::LibUse>,
}

/// The package state elaboration left, for the simulation to start from instead of
/// initialising the packages again and replaying the calls: its program extends the
/// session's (`lowerer`), so the data is valid there.
pub(crate) struct Start {
    pub(crate) lowerer: yavs_lower::Suspended,
    pub(crate) pkgs: Vec<UnitId>,
    /// What the session lowered after the packages, in order (`Session::lowered`): with the
    /// packages, it says what `lowerer`'s program holds.
    pub(crate) lowered: Vec<(DeclId, bool)>,
    pub(crate) state: Arc<StartState>,
    /// The library prefix the program starts with (`libstage`).
    pub(crate) lib: Option<crate::libstage::LibUse>,
}

/// [`Start`]'s state, copied by every build.
pub struct StartState {
    pub(crate) data: Vec<InstData>,
    pub(crate) shared: SharedState,
    /// The session's runtime, for its VHDL assert state.
    pub(crate) rt: Arc<Runtime>,
}

/// Package state during elaboration and the calls made with it.
#[derive(Default)]
pub(crate) struct Session {
    pub(crate) log: Vec<Call>,
    /// What is lowered after the packages, in order: functions, and package constants read
    /// from the package data (`true`).
    lowered: Vec<(DeclId, bool)>,
    live: Option<Live>,
    /// A call failed after it may have changed the package state.
    dirty: bool,
    /// `run_time_packages` for a `Db` of that size (it only grows, by instantiation).
    packages: Option<((usize, usize, usize), Vec<UnitId>)>,
}

impl Session {
    /// Call `f` with `args` against the package state, and log the call.
    pub(crate) fn call(&mut self, db: &Db, f: DeclId, args: Vec<Val>) -> Result<Val, Fail> {
        let result = self.run(db, f, false, &args)?;
        self.log.push(Call {
            func: f,
            name: db.decls[f].name.to_string(),
            args,
            result: result.clone(),
        });
        Ok(result)
    }

    /// The package state at the end of elaboration, if the simulation can start from it: no
    /// call failed without being undone, the lowering can go on, no file was opened.
    pub(crate) fn finish(&mut self) -> Option<Start> {
        if self.dirty {
            return None;
        }
        let mut live = self.live.take()?;
        let (mut lowerer, pkgs) = live.lowerer.take()?;
        if live.rt.files_touched() {
            return None;
        }
        // The lowering goes on from the program the runtime holds: moved over when nothing
        // else holds it (the runtime only keeps its assert state from here on), else copied.
        lowerer.restore(
            Arc::get_mut(&mut live.rt)
                .and_then(Runtime::take_program)
                .unwrap_or_else(|| (*live.rt.prog).clone()),
        );
        let data = live
            .rt
            .package_data()
            .iter()
            .map(|d| (**d).clone())
            .collect();
        // Moved, not copied: the session ends here, and a copy doubled the shared heap (#409).
        let shared = std::mem::take(&mut *live.shared.lock().ok()?);
        Some(Start {
            lowerer,
            pkgs,
            lowered: self.lowered.clone(),
            state: Arc::new(StartState {
                data,
                shared,
                rt: live.rt,
            }),
            lib: live.lib,
        })
    }

    /// The run-time packages of `db`, found again only when `db` has grown.
    fn run_time_packages(&mut self, db: &Db) -> Vec<UnitId> {
        let key = (db.units.len(), db.decls.len(), db.exprs.len());
        match &self.packages {
            Some((k, p)) if *k == key => p.clone(),
            _ => {
                let p = crate::sim::run_time_packages(db);
                self.packages = Some((key, p.clone()));
                p
            }
        }
    }

    /// A point to return to with [`Session::undo`].
    pub(crate) fn mark(&self) -> usize {
        self.log.len()
    }

    /// Undo the calls made since `mark`: the package state is rebuilt from the earlier calls.
    pub(crate) fn undo(&mut self, db: &Db, mark: usize) -> Result<(), String> {
        if self.log.len() == mark && !self.dirty {
            return Ok(());
        }
        self.log.truncate(mark);
        self.live = None;
        let pkgs = self.run_time_packages(db);
        match self.rebuild(db, pkgs) {
            Ok(l) => {
                self.live = Some(l);
                self.dirty = false;
                Ok(())
            }
            Err(Fail::Lower(m) | Fail::Runtime(m)) => Err(m),
            Err(Fail::Unsupported(w)) => Err(w.into()),
        }
    }

    /// The value of package constant `d` in the package data (its initial value ran when
    /// the package was elaborated).
    pub(crate) fn package_const(&mut self, db: &Db, d: DeclId) -> Result<Val, Fail> {
        self.run(db, d, true, &[])
    }

    /// Run `d` (a function, or the reader of a package constant) against the package state,
    /// lowering it first if needed.
    fn run(&mut self, db: &Db, d: DeclId, constant: bool, args: &[Val]) -> Result<Val, Fail> {
        if self.live.as_ref().is_none_or(|l| !l.sigs.contains_key(&d)) {
            self.lowered.push((d, constant));
            // A failed call may have left the package state half changed: rebuild it then.
            let pkgs = self.run_time_packages(db);
            let added = match self.live.as_mut() {
                Some(live) if !self.dirty => add(live, db, &pkgs, d, constant),
                _ => None,
            };
            if !matches!(added, Some(Ok(()))) {
                // `add` may have taken the runtime's program: the state is rebuilt.
                self.live = None;
            }
            if let Some(r) = added {
                if let Err(e) = r {
                    self.lowered.pop();
                    return Err(e);
                }
            } else {
                match self.rebuild(db, pkgs) {
                    Ok(l) => {
                        self.live = Some(l);
                        self.dirty = false;
                    }
                    Err(e) => {
                        self.lowered.pop();
                        return Err(e);
                    }
                }
            }
        }
        let live = self
            .live
            .as_ref()
            .ok_or_else(|| Fail::Runtime("internal error: no package state".into()))?;
        let s = live
            .sigs
            .get(&d)
            .ok_or_else(|| Fail::Runtime("internal error: not lowered".into()))?;
        let r = run(&live.rt, s, args, Some(&live.shared));
        self.dirty |= r.is_err();
        r
    }

    /// Lower the packages and `lowered`, initialise the packages and replay the log.
    fn rebuild(&self, db: &Db, pkgs: Vec<UnitId>) -> Result<Live, Fail> {
        let lower = lower_fail;
        let (mut l, lib) = package_lowerer(db, &pkgs, LowerOptions::default()).map_err(lower)?;
        let init = init_key(db, &pkgs, &LowerOptions::default(), lib.as_ref());
        let mut sigs = HashMap::new();
        for &(d, constant) in &self.lowered {
            sigs.insert(d, item(&mut l, db, d, constant)?);
        }
        let lw = l.snapshot_take().map_err(lower)?;
        let lowerer = Some((l.suspend(), pkgs));
        let prog = Arc::new(lw.program);
        let shared = Arc::new(Mutex::new(SharedState::new(vec![
            0;
            lw.shared_size as usize
        ])));
        let sources = Sources::of(&prog, &db.sources);
        // ponytail: foreign subprograms are bound only for the simulation (`yavs run
        // --load`), so calling one during elaboration is a runtime error.
        let rt = init_packages(
            &prog,
            0,
            &lw.packages,
            &sources,
            &[],
            "",
            &[],
            &shared,
            yavs_std::arith::TimeResolution::Fs,
            revision_of(db),
            None,
            init.as_ref(),
            false,
            |_, _| Ok(()),
        )
        .map_err(runtime)?;
        replay(&rt, &shared, &sigs, &self.log).map_err(Fail::Runtime)?;
        Ok(Live {
            rt,
            shared,
            sigs,
            lowerer,
            shared_size: lw.shared_size,
            lib,
        })
    }
}

/// The smallest lowered-package image worth storing: packages that only declare constants lower to
/// almost nothing.
const MIN_IMAGE: usize = 256;

/// The part of a stage-cache key that names `packages` and the lowering options.
fn stage_part(packages: &[UnitId], opts: &LowerOptions) -> Vec<u8> {
    let mut part: Vec<u8> = packages
        .iter()
        .flat_map(|u| yavs_sem::Id::index(*u).to_le_bytes())
        .collect();
    part.push(opts.no_native_ieee as u8);
    part
}

/// Where the state of `packages` after their initialisation is cached, if `db` has a stage cache
/// and the run has no coverage counters.
pub(crate) fn init_key(
    db: &Db,
    packages: &[UnitId],
    opts: &LowerOptions,
    lib: Option<&crate::libstage::LibUse>,
) -> Option<InitKey> {
    let cache = db.stages().filter(|_| !opts.coverage)?;
    Some(InitKey {
        cache: cache.clone(),
        extent: db.extent(),
        part: stage_part(packages, opts),
        lib: lib.map(|l| {
            let (cache, level, part) = l.init_entry();
            LibInit {
                cache,
                level,
                part,
                packages: l.packages,
                shared: l.shared,
            }
        }),
    })
}

/// A lowerer with the objects of `packages` lowered, in order: read back from the stage cache
/// (`Options::stages`, #407) when `db` has not grown since its analysis, else lowered (and
/// stored). Coverage counters are lowered every time.
pub(crate) fn package_lowerer<'a>(
    db: &'a Db,
    packages: &[UnitId],
    opts: LowerOptions,
) -> Result<(Lowerer<'a>, Option<crate::libstage::LibUse>), Vec<Diagnostic>> {
    if let Some(r) = crate::libstage::lowerer(db, packages, opts) {
        let (l, lib) = r?;
        return Ok((l, Some(lib)));
    }
    let cache = db.stages().filter(|_| !opts.coverage);
    let extent = db.extent();
    let part = stage_part(packages, &opts);
    if let Some(c) = cache
        && let Some(bytes) = c.get(&extent, "low", &part)
        && let Some(l) = Lowerer::from_image(db, opts, &bytes)
    {
        return Ok((l, None));
    }
    let mut l = Lowerer::new(db, opts);
    for &u in packages {
        l.package(u)?;
    }
    if let Some(c) = cache
        && let Some(image) = l.image()?
        && image.len() >= MIN_IMAGE
    {
        c.put(&extent, "low", &part, &image);
    }
    Ok((l, None))
}

fn lower_fail(ds: Vec<Diagnostic>) -> Fail {
    Fail::Lower(ds.into_iter().next().map(|d| d.message).unwrap_or_default())
}

/// Lower function `d`, or the reader of package constant `d` (`constant`), with `l`.
fn item(l: &mut Lowerer, db: &Db, d: DeclId, constant: bool) -> Result<Sig, Fail> {
    if !constant {
        return sig(l, db, d).map_err(lower_fail);
    }
    let yavs_sem::decl::DeclKind::Object(o) = &db.decls[d].kind else {
        return Err(Fail::Runtime("internal error: not a constant".into()));
    };
    Ok(Sig {
        func: l.package_value(d).map_err(lower_fail)?,
        params: Vec::new(),
        ret: Some(l.ty(db.base(o.subtype)).map_err(lower_fail)?),
    })
}

/// Add `d` (as [`item`]) to `live`'s program, keeping the package state. `None` when it
/// cannot be added and the session must be rebuilt: no lowering to resume, other run-time
/// packages, another shared variables area, or the runtime still in use.
fn add(
    live: &mut Live,
    db: &Db,
    now: &[UnitId],
    d: DeclId,
    constant: bool,
) -> Option<Result<(), Fail>> {
    let (mut st, pkgs) = live.lowerer.take()?;
    if pkgs != now {
        return None;
    }
    // The program moves from the runtime to the lowerer and back, never copied. Every exit
    // below this point leaves the runtime without it: the caller drops the session's state.
    let rt = Arc::get_mut(&mut live.rt)?;
    let stmts = rt.prog.stmts.len();
    st.restore(rt.take_program()?);
    let mut l = Lowerer::resume(db, st);
    let s = match item(&mut l, db, d, constant) {
        Ok(s) => s,
        Err(e) => return Some(Err(e)),
    };
    let lw = match l.snapshot_take() {
        Ok(lw) => lw,
        Err(ds) => return Some(Err(lower_fail(ds))),
    };
    if lw.shared_size != live.shared_size {
        return None;
    }
    if let Err(e) = rt.extend_from(Arc::new(lw.program), stmts, &db.sources) {
        return Some(Err(runtime(e)));
    }
    live.sigs.insert(d, s);
    live.lowerer = Some((l.suspend(), pkgs));
    Some(Ok(()))
}
