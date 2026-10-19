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
//! ponytail: the session program is lowered again (packages plus every function called so
//! far) and the log replayed whenever a new function is called, O(functions x calls); a
//! lowerer that can add functions to a finished program would make it incremental.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use yavs_diag::Diagnostic;
use yavs_interp::{Bindings, CallVal, InstData, Runtime, SharedState, Sources};
use yavs_ir::{FuncId, Program, TyId};
use yavs_kernel::SimError;
use yavs_lower::{LowerOptions, Lowerer, PackageIr};
use yavs_sem::{Db, DeclId};

use crate::eval::Val;

/// A function lowered into a program, with the IR types of its parameters and result.
#[derive(Clone, Debug)]
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

/// Initialise `packages` of `prog` in order (each sees those before it, LRM 14.4.2) with
/// `shared`, `fix` completing each package's data, and return the runtime with all of them
/// bound.
#[allow(clippy::too_many_arguments)] // ponytail: one more setting; bundle into a struct when the next one comes
pub(crate) fn init_packages(
    prog: &Arc<Program>,
    packages: &[PackageIr],
    sources: &Sources,
    foreign: &[(yavs_ir::ForeignId, yavs_ffi::ForeignFn)],
    searched: &str,
    models: &[(yavs_ir::ForeignId, Arc<dyn yavs_interp::ForeignModel>)],
    shared: &Arc<Mutex<SharedState>>,
    resolution: yavs_std::arith::TimeResolution,
    revision: yavs_std::types::Revision,
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
        ..Default::default()
    };
    for p in packages {
        let rt = Runtime::new(prog.clone(), bind(&data))?;
        let mut d = yavs_interp::init_package(rt, p.init, p.index, p.size as usize, Some(shared))?;
        fix(p, &mut d)?;
        data.push(Arc::new(d));
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
        match self.rebuild(db) {
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
            match self.rebuild(db) {
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
    fn rebuild(&self, db: &Db) -> Result<Live, Fail> {
        let lower = |ds: Vec<Diagnostic>| {
            Fail::Lower(ds.into_iter().next().map(|d| d.message).unwrap_or_default())
        };
        let mut l = Lowerer::new(db, LowerOptions::default());
        for u in crate::sim::run_time_packages(db) {
            l.package(u).map_err(lower)?;
        }
        let mut sigs = HashMap::new();
        for &(d, constant) in &self.lowered {
            let s = if constant {
                let yavs_sem::decl::DeclKind::Object(o) = &db.decls[d].kind else {
                    return Err(Fail::Runtime("internal error: not a constant".into()));
                };
                Sig {
                    func: l.package_value(d).map_err(lower)?,
                    params: Vec::new(),
                    ret: Some(l.ty(db.base(o.subtype)).map_err(lower)?),
                }
            } else {
                sig(&mut l, db, d).map_err(lower)?
            };
            sigs.insert(d, s);
        }
        let lw = l.finish().map_err(lower)?;
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
            &lw.packages,
            &sources,
            &[],
            "",
            &[],
            &shared,
            yavs_std::arith::TimeResolution::Fs,
            revision_of(db),
            |_, _| Ok(()),
        )
        .map_err(runtime)?;
        replay(&rt, &shared, &sigs, &self.log).map_err(Fail::Runtime)?;
        Ok(Live { rt, shared, sigs })
    }
}
