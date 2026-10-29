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
use yavs_lower::{FreeObj, LowerOptions, Lowerer, PackageIr};
use yavs_sem::{Db, DeclId, UnitId};

use crate::eval::Val;
use crate::hier::ScopeId;

/// A shared variable of a region (instance, block, generate): the elaborated region that
/// declares it and its declaration.
pub(crate) type VarKey = (ScopeId, DeclId);

/// How a call is given one object of an enclosing region it reads (`Lowerer::capture_free_objects`):
/// the value of a constant or generic, or a shared variable (its object's handle).
#[derive(Clone, Debug)]
pub(crate) enum FillItem {
    Val(Val),
    Var(VarKey),
}

/// A shared variable's object, created for an elaboration-time call: `items` is what its
/// creation thunk reads.
#[derive(Clone, Debug)]
pub(crate) struct Create {
    pub(crate) key: VarKey,
    pub(crate) items: Vec<(DeclId, FillItem)>,
}

/// What a call of a function declared in a region ran with besides its arguments: the objects of
/// the region (by declaration, so that any lowering of the function can lay them out), and the
/// shared variables created for it, in order, before it ran.
#[derive(Clone, Debug, Default)]
pub(crate) struct Fill {
    pub(crate) items: Vec<(DeclId, FillItem)>,
    pub(crate) creates: Vec<Create>,
}

/// The captured objects of a lowered function (or creation thunk) and the bytes of instance
/// data it reads them from.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct Needs {
    pub(crate) free: Vec<FreeObj>,
    pub(crate) size: u32,
}

/// The instance data of a call: `items` laid out as `needs` says, handles of shared variables
/// from `vars`.
pub(crate) fn inst_data(
    prog: &Program,
    needs: &Needs,
    items: &[(DeclId, FillItem)],
    vars: &HashMap<VarKey, u64>,
) -> Result<InstData, Fail> {
    let mut inst = InstData {
        data: vec![0; needs.size as usize],
        ..Default::default()
    };
    for f in &needs.free {
        let Some((_, item)) = items.iter().find(|(d, _)| *d == f.decl) else {
            return Err(Fail::Runtime(
                "internal error: no value for an object a call reads".into(),
            ));
        };
        let off = f.off as usize;
        match item {
            FillItem::Var(k) => {
                let h = vars.get(k).ok_or_else(|| {
                    Fail::Runtime("internal error: a shared variable is not created".into())
                })?;
                inst.data[off..off + 8].copy_from_slice(&h.to_le_bytes());
            }
            FillItem::Val(v) => match crate::value::encode(prog, f.ty, v) {
                Some(CallVal::Cell(c)) => {
                    let n = prog
                        .types
                        .get(f.ty)
                        .and_then(|t| t.scalar_size())
                        .unwrap_or(8) as usize;
                    inst.data[off..off + n].copy_from_slice(&c.to_le_bytes()[..n]);
                }
                Some(CallVal::Composite { data, desc }) => {
                    yavs_lower::UnitIr::write_composite(f.off, &data, &desc, &mut inst.data);
                }
                None => return Err(Fail::Unsupported("an object of this type read by a call")),
            },
        }
    }
    Ok(inst)
}

/// Run creation thunk `func` with `inst`: the handle of the new shared variable's object.
fn create_var(
    rt: &Arc<Runtime>,
    shared: &Arc<Mutex<SharedState>>,
    func: yavs_ir::FuncId,
    inst: InstData,
) -> Result<u64, Fail> {
    match yavs_interp::call_values_shared(rt.clone(), func, Arc::new(inst), &[], None, Some(shared))
        .map_err(runtime)?
    {
        Some(CallVal::Cell(h)) => Ok(h),
        _ => Err(Fail::Runtime(
            "internal error: a shared variable's thunk gave no handle".into(),
        )),
    }
}

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
    inst: Option<InstData>,
) -> Result<Val, Fail> {
    match run_with(rt, s, args, shared, inst, None)? {
        Ran::Val(v) => Ok(v),
        Ran::Restart => Err(Fail::Runtime(
            "internal error: an untraced call restarted".into(),
        )),
    }
}

/// [`run`] without shared state, saying what the call did besides computing (`CallTrace`).
pub(crate) fn run_traced(
    rt: &mut Arc<Runtime>,
    s: &Sig,
    args: &[Val],
    inst: Option<InstData>,
) -> (Result<Val, Fail>, yavs_interp::CallTrace) {
    run_restarting(rt, s, args, None, inst)
}

/// [`run_traced`] against the package state `shared` (a call of the session).
fn run_traced_shared(
    rt: &mut Arc<Runtime>,
    s: &Sig,
    args: &[Val],
    shared: &Arc<Mutex<SharedState>>,
    inst: Option<InstData>,
) -> (Result<Val, Fail>, yavs_interp::CallTrace) {
    run_restarting(rt, s, args, Some(shared), inst)
}

/// Times a call may stop to run again with the functions its loops made hot (#695): each stop
/// repeats at most the work of one budget of jumps back, and the last run does not stop.
const RESTARTS: u32 = 2;

/// A traced call. One that has only computed and whose loops outgrew the budget of lowered code
/// stops (`Ran::Restart`); the functions it listed are optimised ([`Runtime::promote_hot`], which
/// needs the runtime to itself, so no clone of `rt` outlives a run) and it runs again.
fn run_restarting(
    rt: &mut Arc<Runtime>,
    s: &Sig,
    args: &[Val],
    shared: Option<&Arc<Mutex<SharedState>>>,
    inst: Option<InstData>,
) -> (Result<Val, Fail>, yavs_interp::CallTrace) {
    let mut tries = 0;
    loop {
        Runtime::promote_hot(rt);
        let _last = (tries == RESTARTS).then(yavs_ir::ElabRaw::no_restart);
        let mut t = yavs_interp::CallTrace::default();
        match run_with(rt, s, args, shared, inst.clone(), Some(&mut t)) {
            Ok(Ran::Restart) => {
                yavs_ir::note_restart();
                tries += 1;
            }
            Ok(Ran::Val(v)) => return (Ok(v), t),
            Err(f) => return (Err(f), t),
        }
    }
}

/// What a call ended with.
enum Ran {
    Val(Val),
    /// It stopped to run again ([`run_restarting`]).
    Restart,
}

fn run_with(
    rt: &Arc<Runtime>,
    s: &Sig,
    args: &[Val],
    shared: Option<&Arc<Mutex<SharedState>>>,
    inst: Option<InstData>,
    trace: Option<&mut yavs_interp::CallTrace>,
) -> Result<Ran, Fail> {
    let prog = rt.prog.clone();
    // A one-shot evaluation: functions run as lowered until they prove hot (`promote_hot`).
    let _raw = yavs_ir::ElabRaw::on();
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
    let (inst, ret_ty) = (Arc::new(inst.unwrap_or_default()), composite.then_some(ret));
    let r = match trace {
        Some(t) => {
            let (r, tr) =
                yavs_interp::call_values_traced(rt.clone(), s.func, inst, &cvs, ret_ty, shared);
            *t = tr;
            r
        }
        None => yavs_interp::call_values_shared(rt.clone(), s.func, inst, &cvs, ret_ty, shared),
    };
    let r = match r {
        Err(e) if yavs_interp::is_restart(&e) => return Ok(Ran::Restart),
        r => r.map_err(runtime)?.ok_or(Fail::Unsupported("no result"))?,
    };
    crate::value::decode(&prog, ret, &r)
        .map(Ran::Val)
        .ok_or(Fail::Unsupported("a result of this type"))
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
    // The runtime of the last initialiser: the files the initialisers open are handed on (#834).
    let mut last: Option<Arc<Runtime>> = None;
    for (i, p) in packages.iter().enumerate().skip(first) {
        let mut d = match start {
            Some(s) => s[i].clone(),
            None => {
                let rt = Runtime::new(prog.clone(), bind(&data))?;
                if let Some(l) = &last {
                    rt.adopt_files(l);
                }
                let d = yavs_interp::init_package(
                    rt.clone(),
                    p.init,
                    p.index,
                    p.size as usize,
                    Some(shared),
                )?;
                last = Some(rt);
                d
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
    let rt = Runtime::new(prog.clone(), bind(&data))?;
    if let Some(l) = &last {
        rt.adopt_files(l);
    }
    Ok(rt)
}

/// The std/ieee revision the design runs against (INTEGER bounds, FILE_OPEN_KIND, `vhdl_version`).
pub(crate) fn revision_of(db: &Db) -> yavs_std::types::Revision {
    match db.rev {
        yavs_syntax::Std::Vhdl2019 => yavs_std::types::Revision::Vhdl2019,
        yavs_syntax::Std::Vhdl2008 => yavs_std::types::Revision::Vhdl2008,
        yavs_syntax::Std::Vhdl1993 => yavs_std::types::Revision::Vhdl1993,
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
    /// The objects of its region it read (`None`: none).
    pub(crate) fill: Option<Fill>,
}

/// The functions lowered for running calls again: their signatures, the captured objects each
/// reads, and the thunks that create the shared variables among them.
#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
pub(crate) struct Items {
    pub(crate) sigs: HashMap<DeclId, Sig>,
    /// By function, and by shared variable for its thunk.
    pub(crate) needs: HashMap<DeclId, Needs>,
    pub(crate) inits: HashMap<DeclId, yavs_ir::FuncId>,
}

fn fail_text(f: Fail) -> String {
    match f {
        Fail::Lower(m) | Fail::Runtime(m) => m,
        Fail::Unsupported(w) => format!("internal error: {w}"),
    }
}

/// Run `log` again, in order, on freshly initialised packages: the side effects of
/// elaboration. Every call must return what it returned during elaboration. Gives the
/// handles of the shared variables the calls created, in this run.
pub(crate) fn replay(
    rt: &Arc<Runtime>,
    shared: &Arc<Mutex<SharedState>>,
    items: &Items,
    log: &[Call],
) -> Result<HashMap<VarKey, u64>, String> {
    // The reports of these calls were printed when they first ran (#390).
    rt.set_replay(true);
    let r = replay_calls(rt, shared, items, log);
    rt.set_replay(false);
    r
}

fn replay_calls(
    rt: &Arc<Runtime>,
    shared: &Arc<Mutex<SharedState>>,
    items: &Items,
    log: &[Call],
) -> Result<HashMap<VarKey, u64>, String> {
    let mut vars: HashMap<VarKey, u64> = HashMap::new();
    for c in log {
        let name = &c.name;
        let s = items
            .sigs
            .get(&c.func)
            .ok_or_else(|| format!("internal error: `{name}` is not lowered for the replay"))?;
        let mut inst = None;
        if let Some(f) = &c.fill {
            let not_lowered =
                || format!("internal error: `{name}` has no objects lowered for the replay");
            for cr in &f.creates {
                let init = items.inits.get(&cr.key.1).ok_or_else(not_lowered)?;
                let needs = items.needs.get(&cr.key.1).ok_or_else(not_lowered)?;
                let data = inst_data(&rt.prog, needs, &cr.items, &vars).map_err(fail_text)?;
                let h = create_var(rt, shared, *init, data).map_err(fail_text)?;
                vars.insert(cr.key, h);
            }
            let needs = items.needs.get(&c.func).ok_or_else(not_lowered)?;
            inst = Some(inst_data(&rt.prog, needs, &f.items, &vars).map_err(fail_text)?);
        }
        match run(rt, s, &c.args, Some(shared), inst) {
            Ok(v) if v == c.result => {}
            Ok(_) => {
                return Err(format!(
                    "internal error: `{name}` returned another value when its elaboration-time \
                     call was replayed"
                ));
            }
            Err(e) => return Err(fail_text(e)),
        }
    }
    Ok(vars)
}

/// The program and package state elaboration-time calls run against.
struct Live {
    rt: Arc<Runtime>,
    shared: Arc<Mutex<SharedState>>,
    items: Items,
    /// The shared variables of regions created so far, by their handles.
    vars: HashMap<VarKey, u64>,
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
    /// Whether it lowered anything for the objects of regions: its program then differs from
    /// what the same requests give a fresh lowering (no lowering key).
    pub(crate) captured: bool,
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
    /// The shared variables of regions the calls created: their handles in `shared`.
    pub(crate) vars: Vec<(VarKey, u64)>,
}

/// Package state during elaboration and the calls made with it.
#[derive(Default)]
pub(crate) struct Session {
    pub(crate) log: Vec<Call>,
    /// What is lowered after the packages, in order: functions, and package constants and
    /// package type bounds read from the package data (`true`).
    lowered: Vec<(DeclId, bool)>,
    /// The shared variables of regions whose creation thunks were lowered after those, in order.
    inits: Vec<DeclId>,
    live: Option<Live>,
    /// A call failed after it may have changed the package state.
    dirty: bool,
    /// `run_time_packages` for a `Db` of that size (it only grows, by instantiation).
    packages: Option<((usize, usize, usize), Vec<UnitId>)>,
    /// Shared variables of regions created for the call being prepared; the next logged call
    /// takes them.
    pending: Vec<Create>,
}

impl Session {
    /// Call `f` with `args` against the package state, and log the call.
    ///
    /// With `trace`, the call also says what it did besides computing (`CallTrace`).
    pub(crate) fn call(
        &mut self,
        db: &Db,
        f: DeclId,
        args: Vec<Val>,
        trace: Option<&mut yavs_interp::CallTrace>,
    ) -> Result<Val, Fail> {
        let result = self.run(db, f, false, &args, None, trace)?;
        self.log.push(Call {
            func: f,
            name: db.decls[f].name.to_string(),
            args,
            result: result.clone(),
            fill: None,
        });
        Ok(result)
    }

    /// Call `f`, declared in a region, with `args` and the objects of the region it reads
    /// (`items`, for what [`Session::needs`] returned), and log the call with the shared
    /// variables created for it.
    pub(crate) fn call_in(
        &mut self,
        db: &Db,
        f: DeclId,
        args: Vec<Val>,
        items: Vec<(DeclId, FillItem)>,
    ) -> Result<Val, Fail> {
        let result = self.run(db, f, false, &args, Some(&items), None)?;
        self.log.push(Call {
            func: f,
            name: db.decls[f].name.to_string(),
            args,
            result: result.clone(),
            fill: Some(Fill {
                items,
                creates: std::mem::take(&mut self.pending),
            }),
        });
        Ok(result)
    }

    /// The captured objects function `f` reads, lowering it into the session if needed.
    pub(crate) fn needs(&mut self, db: &Db, f: DeclId) -> Result<Needs, Fail> {
        self.ensure(db, f, false)?;
        self.live
            .as_ref()
            .and_then(|l| l.items.needs.get(&f))
            .cloned()
            .ok_or_else(|| Fail::Runtime("internal error: not lowered".into()))
    }

    /// The captured objects the thunk that creates shared variable `var` reads.
    pub(crate) fn init_needs(&mut self, db: &Db, var: DeclId) -> Result<Needs, Fail> {
        self.ensure_init(db, var)?;
        self.live
            .as_ref()
            .and_then(|l| l.items.needs.get(&var))
            .cloned()
            .ok_or_else(|| Fail::Runtime("internal error: not lowered".into()))
    }

    /// The handle of the object of a region's shared variable, once created.
    pub(crate) fn var(&self, key: VarKey) -> Option<u64> {
        self.live.as_ref().and_then(|l| l.vars.get(&key).copied())
    }

    /// Create the object of shared variable `key.1` of region `key.0`, its thunk reading `items`
    /// (for what [`Session::init_needs`] returned); the next logged call takes the creation.
    pub(crate) fn create_var(
        &mut self,
        db: &Db,
        key: VarKey,
        items: Vec<(DeclId, FillItem)>,
    ) -> Result<u64, Fail> {
        self.ensure_init(db, key.1)?;
        let live = self
            .live
            .as_mut()
            .ok_or_else(|| Fail::Runtime("internal error: no package state".into()))?;
        let init = *live
            .items
            .inits
            .get(&key.1)
            .ok_or_else(|| Fail::Runtime("internal error: not lowered".into()))?;
        let needs = live
            .items
            .needs
            .get(&key.1)
            .ok_or_else(|| Fail::Runtime("internal error: not lowered".into()))?;
        let data = inst_data(&live.rt.prog, needs, &items, &live.vars)?;
        let r = create_var(&live.rt, &live.shared, init, data);
        self.dirty |= r.is_err();
        let h = r?;
        live.vars.insert(key, h);
        self.pending.push(Create { key, items });
        Ok(h)
    }

    /// Give up the call being prepared: what it created is not logged, so the state is
    /// rebuilt from the log before the next call.
    pub(crate) fn abandon(&mut self) {
        if !self.pending.is_empty() {
            self.pending.clear();
            self.dirty = true;
        }
    }

    /// The package state at the end of elaboration, if the simulation can start from it: no
    /// call failed without being undone, the lowering can go on, no file was opened.
    pub(crate) fn finish(&mut self) -> Option<Start> {
        if self.dirty || !self.pending.is_empty() {
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
        // What read the objects of regions was laid out for the calls, not for the design.
        let captured =
            !live.items.inits.is_empty() || live.items.needs.values().any(|n| !n.free.is_empty());
        lowerer.forget_free_objects();
        let data = live
            .rt
            .package_data()
            .iter()
            .map(|d| (**d).clone())
            .collect();
        // Moved, not copied: the session ends here, and a copy doubled the shared heap (#409).
        let shared = std::mem::take(&mut *live.shared.lock().ok()?);
        let mut vars: Vec<_> = live.vars.iter().map(|(&k, &h)| (k, h)).collect();
        vars.sort_by_key(|&((r, d), _)| (r, yavs_sem::Id::index(d)));
        Some(Start {
            lowerer,
            pkgs,
            lowered: self.lowered.clone(),
            captured,
            state: Arc::new(StartState {
                data,
                shared,
                rt: live.rt,
                vars,
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
        if self.log.len() == mark && !self.dirty && self.pending.is_empty() {
            return Ok(());
        }
        self.log.truncate(mark);
        self.pending.clear();
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
        self.run(db, d, true, &[], None, None)
    }

    /// Bound `k` of the ranges package type or subtype `d` evaluated (`Lowerer::package_range`).
    pub(crate) fn package_range(&mut self, db: &Db, d: DeclId, k: i64) -> Result<Val, Fail> {
        self.run(db, d, true, &[Val::Int(k)], None, None)
    }

    /// Lower `d` (a function, or the reader of a package constant) into the live program, or
    /// rebuild the program with it.
    fn ensure(&mut self, db: &Db, d: DeclId, constant: bool) -> Result<(), Fail> {
        if self
            .live
            .as_ref()
            .is_some_and(|l| l.items.sigs.contains_key(&d))
        {
            return Ok(());
        }
        self.lowered.push((d, constant));
        let r = self.lower_more(db, |l, items| {
            let (sig, needs) = item(l, db, d, constant)?;
            items.sigs.insert(d, sig);
            items.needs.insert(d, needs);
            Ok(())
        });
        if r.is_err() {
            self.lowered.pop();
        }
        r
    }

    /// [`Session::ensure`] for the creation thunk of shared variable `var`.
    fn ensure_init(&mut self, db: &Db, var: DeclId) -> Result<(), Fail> {
        if self
            .live
            .as_ref()
            .is_some_and(|l| l.items.inits.contains_key(&var))
        {
            return Ok(());
        }
        self.inits.push(var);
        let r = self.lower_more(db, |l, items| {
            let (func, needs) = init_item(l, var)?;
            items.inits.insert(var, func);
            items.needs.insert(var, needs);
            Ok(())
        });
        if r.is_err() {
            self.inits.pop();
        }
        r
    }

    /// Add what `lower` lowers to the live program, keeping the package state when the
    /// lowering can go on; else the program is rebuilt with `self.lowered` and `self.inits`
    /// (which the request is already part of).
    fn lower_more(
        &mut self,
        db: &Db,
        lower: impl FnOnce(&mut Lowerer, &mut Items) -> Result<(), Fail>,
    ) -> Result<(), Fail> {
        // A failed call may have left the package state half changed: rebuild it then.
        let pkgs = self.run_time_packages(db);
        let added = match self.live.as_mut() {
            Some(live) if !self.dirty => add(live, db, &pkgs, lower),
            _ => None,
        };
        match added {
            Some(r) => {
                if r.is_err() {
                    // `add` may have taken the runtime's program: the state is rebuilt.
                    self.live = None;
                }
                r
            }
            None => {
                self.live = None;
                let l = self.rebuild(db, pkgs)?;
                self.live = Some(l);
                self.dirty = false;
                Ok(())
            }
        }
    }

    /// Run `d` (a function, or the reader of a package constant) against the package state,
    /// lowering it first if needed; `items`: the objects of a region it reads.
    fn run(
        &mut self,
        db: &Db,
        d: DeclId,
        constant: bool,
        args: &[Val],
        items: Option<&[(DeclId, FillItem)]>,
        trace: Option<&mut yavs_interp::CallTrace>,
    ) -> Result<Val, Fail> {
        self.ensure(db, d, constant)?;
        if let Some(live) = self.live.as_mut() {
            yavs_interp::Runtime::promote_hot(&mut live.rt);
        }
        let live = self
            .live
            .as_mut()
            .ok_or_else(|| Fail::Runtime("internal error: no package state".into()))?;
        let s = live
            .items
            .sigs
            .get(&d)
            .ok_or_else(|| Fail::Runtime("internal error: not lowered".into()))?;
        let inst = match items {
            Some(items) => {
                let needs = live
                    .items
                    .needs
                    .get(&d)
                    .ok_or_else(|| Fail::Runtime("internal error: not lowered".into()))?;
                Some(inst_data(&live.rt.prog, needs, items, &live.vars)?)
            }
            None => None,
        };
        let r = match trace {
            Some(t) => {
                let (r, tr) = run_traced_shared(&mut live.rt, s, args, &live.shared, inst);
                *t = tr;
                r
            }
            None => run(&live.rt, s, args, Some(&live.shared), inst),
        };
        self.dirty |= r.is_err();
        r
    }

    /// Whether the package state is the same in every run of this closure: nothing that ran on
    /// it printed, read the environment, a clock or a file, or called foreign code.
    pub(crate) fn state_is_pure(&self) -> bool {
        self.live
            .as_ref()
            .is_some_and(|l| l.shared.lock().is_ok_and(|s| !s.is_tainted()))
    }

    /// Lower the packages and `lowered`, initialise the packages and replay the log.
    fn rebuild(&self, db: &Db, pkgs: Vec<UnitId>) -> Result<Live, Fail> {
        let lower = lower_fail;
        let (mut l, lib) = package_lowerer(db, &pkgs, LowerOptions::default()).map_err(lower)?;
        l.capture_free_objects(true);
        let init = init_key(db, &pkgs, &LowerOptions::default(), lib.as_ref());
        let mut items = Items::default();
        for &(d, constant) in &self.lowered {
            let (sig, needs) = item(&mut l, db, d, constant)?;
            items.sigs.insert(d, sig);
            items.needs.insert(d, needs);
        }
        for &v in &self.inits {
            let (func, needs) = init_item(&mut l, v)?;
            items.inits.insert(v, func);
            items.needs.insert(v, needs);
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
        let vars = replay(&rt, &shared, &items, &self.log).map_err(Fail::Runtime)?;
        Ok(Live {
            rt,
            shared,
            items,
            vars,
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
    let cache = db.stages().filter(|_| !opts.any_coverage())?;
    Some(InitKey {
        cache: cache.clone(),
        extent: db.stage_extent(),
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
    let cache = db.stages().filter(|_| !opts.any_coverage());
    let extent = db.stage_extent();
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

/// What `l` lowered for `f` reads of the regions' objects.
pub(crate) fn needs_of(l: &Lowerer, f: yavs_ir::FuncId) -> Needs {
    let (_, size) = l.free_objects();
    Needs {
        free: l.free_for(f),
        size,
    }
}

/// Lower function `d`, or the reader of package constant `d` or of the bounds of package type
/// or subtype `d` (`constant`), with `l`: its signature and the objects of regions it reads.
fn item(l: &mut Lowerer, db: &Db, d: DeclId, constant: bool) -> Result<(Sig, Needs), Fail> {
    let ranges = matches!(
        db.decls[d].kind,
        yavs_sem::decl::DeclKind::Type(_) | yavs_sem::decl::DeclKind::Subtype(_)
    );
    let sig = if !constant {
        sig(l, db, d).map_err(lower_fail)?
    } else if ranges {
        let int = db
            .well_known
            .integer
            .ok_or_else(|| Fail::Runtime("internal error: no INTEGER".into()))?;
        let int = l.ty(int).map_err(lower_fail)?;
        Sig {
            func: l.package_range(d).map_err(lower_fail)?,
            params: vec![int],
            ret: Some(int),
        }
    } else {
        let yavs_sem::decl::DeclKind::Object(o) = &db.decls[d].kind else {
            return Err(Fail::Runtime("internal error: not a constant".into()));
        };
        Sig {
            func: l.package_value(d).map_err(lower_fail)?,
            params: Vec::new(),
            ret: Some(l.ty(db.base(o.subtype)).map_err(lower_fail)?),
        }
    };
    let needs = needs_of(l, sig.func);
    Ok((sig, needs))
}

/// Lower the thunk that creates the object of shared variable `var` of a region, with `l`.
fn init_item(l: &mut Lowerer, var: DeclId) -> Result<(yavs_ir::FuncId, Needs), Fail> {
    let f = l.region_var_init(var).map_err(lower_fail)?;
    let needs = needs_of(l, f);
    Ok((f, needs))
}

/// Add what `lower` lowers to `live`'s program, keeping the package state. `None` when it
/// cannot be added and the session must be rebuilt: no lowering to resume, other run-time
/// packages, another shared variables area, or the runtime still in use.
fn add(
    live: &mut Live,
    db: &Db,
    now: &[UnitId],
    lower: impl FnOnce(&mut Lowerer, &mut Items) -> Result<(), Fail>,
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
    l.capture_free_objects(true);
    if let Err(e) = lower(&mut l, &mut live.items) {
        return Some(Err(e));
    }
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
    live.lowerer = Some((l.suspend(), pkgs));
    Some(Ok(()))
}
