//! Processes: lowering the architectures that own processes (yavs-lower) and building one
//! interpreter process per process instance (yavs-interp), with per-instance data.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

/// Most results one element-wise user resolution function remembers.
const RES_MEMO: usize = 1 << 14;

use yavs_diag::Diagnostic;
use yavs_interp::{
    CallVal, DebugSession, InstData, InstSig, InterpProcess, ProcDriver, Runtime, SharedState,
};
use yavs_kernel::{Config, ErrorKind, SimControl, SimError};
use yavs_lower::{LowerOptions, Lowerer, UnitIr};
use yavs_sem::{Db, UnitId};

use crate::Elaborated;
use crate::hier::{LeafRes, ScopeId, ScopeKind};

/// The IR of the design: one program, one `UnitIr` per architecture that owns processes.
pub struct Ir {
    pub program: Arc<yavs_ir::Program>,
    pub units: HashMap<UnitId, UnitIr>,
    /// The function of each user resolution function of a signal.
    pub resolvers: HashMap<yavs_sem::DeclId, yavs_ir::FuncId>,
    /// IR types of the parameter and the result of each user resolution function (#193,
    /// #293).
    pub res_types: HashMap<yavs_sem::DeclId, (yavs_ir::TyId, yavs_ir::TyId)>,
    /// Left bound and direction (`true` for `downto`) of the index subtype of each user
    /// resolution function's parameter: the driving values arrive as an array from that left
    /// bound in that direction (lrm-checks L101).
    pub res_index: HashMap<yavs_sem::DeclId, (i64, bool)>,
    /// Leaves resolved by a function declared in an entity or architecture: the instance
    /// whose data the function reads (its constants and generics, #164).
    pub res_regions: std::collections::BTreeMap<crate::LeafId, ScopeId>,
    /// Instances without processes whose declarative part runs at the start (constants that
    /// can fail).
    pub init_regions: Vec<ScopeId>,
    /// `'path_name`/`'instance_name` strings per (region, named entity, instance?).
    pub names: HashMap<(ScopeId, yavs_sem::DeclId, bool), String>,
    /// Packages with run-time objects, in initialisation order (#183).
    pub packages: Vec<yavs_lower::PackageIr>,
    /// Source files of the program's statements, for `std.env` call paths.
    pub sources: yavs_interp::Sources,
    /// The std/ieee revision of the design.
    pub revision: yavs_std::types::Revision,
    /// Bytes of the shared variables area (file objects of packages).
    pub shared_size: u32,
    /// The C function of each `program.foreign` entry, bound by the caller (`yavs run
    /// --load`) before [`Elaborated::build`].
    pub foreign: Vec<(yavs_ir::ForeignId, yavs_ffi::ForeignFn)>,
    /// Where the unbound ones were looked for (reported when one is called).
    pub foreign_searched: String,
    /// The plugin-registered model of each `'FOREIGN "VHPI library model"` entry of
    /// `program.foreign`, bound by the caller like `foreign`.
    pub models: Vec<(yavs_ir::ForeignId, Arc<dyn yavs_interp::ForeignModel>)>,
    /// Thunks returning the handle of each package shared variable a variable port denotes.
    pub var_pkgs: HashMap<yavs_sem::DeclId, yavs_ir::FuncId>,
    /// The functions of `Elaborated::elab_calls`, for the replay.
    pub(crate) elab_sigs: HashMap<yavs_sem::DeclId, crate::pkgcall::Sig>,
    /// The slot thunk (packages) and value type of each non-protected shared variable
    /// (`Lowerer::var_slots`), for [`Built::vars`].
    pub var_slots: Vec<yavs_lower::VarSlot>,
    /// The package state elaboration left (the program extends the session's): the
    /// simulation starts from a copy of it instead of initialising the packages and replaying
    /// the elaboration-time calls (#383).
    pub start: Option<Arc<crate::pkgcall::StartState>>,
}

/// The interpreter side of a built simulation (`Elaborated::build_full`).
#[derive(Clone)]
pub struct Built {
    pub rt: Arc<Runtime>,
    pub shared: Arc<Mutex<SharedState>>,
    /// The slot address and value type of each non-protected shared variable, by declaring
    /// region (`None` for a package's) and declaration: VHPI and waves read it there
    /// (`yavs_interp::shared_var`, #309). Every region's is listed, used by a process or not
    /// (#342).
    pub vars: HashMap<(Option<ScopeId>, yavs_sem::DeclId), (u64, yavs_ir::TyId)>,
}

fn sim_err(m: String) -> SimError {
    SimError::new(ErrorKind::Usage(m))
}

impl Elaborated {
    /// A runtime error as a diagnostic (plan: yavs-diag): the kernel's code, the failing
    /// statement as the primary span, one "called from here" label per outer VHDL frame, and
    /// the time, delta and process as a note.
    pub fn runtime_diagnostic(&self, e: &SimError) -> Diagnostic {
        let mut d = Diagnostic::error(e.kind.code(), e.kind.to_string());
        let prog = self.ir.as_ref().map(|ir| &*ir.program);
        let spans = e
            .trace
            .iter()
            .filter_map(|&s| prog.and_then(|p| p.stmts.get(s)).map(|i| i.span));
        for (k, span) in spans.enumerate() {
            d = if k == 0 {
                d.with_primary(span, "")
            } else {
                d.with_secondary(span, "called from here")
            };
        }
        let mut at = format!("at {}, delta {}", e.time, e.delta);
        if let Some((_, path)) = &e.process {
            at.push_str(&format!(", in process {path}"));
        }
        if let ErrorKind::Foreign { searched, .. } = &e.kind {
            d = d
                .with_note(format!("searched {searched}"))
                .with_help("load the shared library that defines it with `--load=LIB`");
        }
        d.with_note(at)
    }

    /// Rows of the coverage counters (`Runtime::coverage`), `Program::cov.len()` counters
    /// each: row 0 counts code that runs without instance data (package initialisation),
    /// row `r.index() + 1` code running with region `r`'s instance data.
    // ponytail: dense rows for every scope, most of them unused; give rows only to regions
    // with instance data if large designs need the memory.
    pub fn cov_rows(&self) -> usize {
        self.hier.scopes.len() + 1
    }

    /// The instance scope a scope belongs to.
    pub fn instance_of(&self, s: ScopeId) -> ScopeId {
        self.hier.instance_of(s)
    }

    /// Lower the architectures that own processes, and the package bodies they may call.
    /// Fail early: lowering diagnostics are errors.
    pub fn lower(&mut self, db: &mut Db) -> Result<(), Vec<Diagnostic>> {
        self.lower_with(db, LowerOptions::default())
    }

    /// [`Elaborated::lower`] with options (coverage counters).
    pub fn lower_with(&mut self, db: &mut Db, opts: LowerOptions) -> Result<(), Vec<Diagnostic>> {
        let mut archs: Vec<UnitId> = Vec::new();
        let mut with_procs = std::collections::HashSet::new();
        for p in &self.procs {
            let i = self.instance_of(p.scope);
            with_procs.insert(i);
            if let ScopeKind::Instance { arch, .. } = self.hier.scope(i).kind
                && !archs.contains(&arch)
            {
                archs.push(arch);
            }
        }
        // A constant whose value only the run-time evaluation gives can fail (a call, an
        // overflow, `'value` of a bad string): the declarative part of an instance without
        // processes runs too (LRM 14.4.1).
        let mut init_regions = Vec::new();
        for (k, sc) in self.hier.scopes.iter().enumerate() {
            if let ScopeKind::Instance { entity, arch } = sc.kind
                && !with_procs.contains(&ScopeId(k as u32))
                && [entity, arch]
                    .iter()
                    .any(|&u| runtime_constant(db, db.units[u].scope))
            {
                if !archs.contains(&arch) {
                    archs.push(arch);
                }
                init_regions.push(ScopeId(k as u32));
            }
        }
        // A resolution function declared in an entity or architecture reads that instance's
        // data, so its architecture is lowered even when it owns no process.
        let mut res_regions = std::collections::BTreeMap::new();
        for (k, leaf) in self.nets.leaves.iter().enumerate() {
            let Some(LeafRes::Func(d) | LeafRes::Whole(d)) = leaf.resolution else {
                continue;
            };
            let Some(region) = self.res_region(db, d, self.nets.obj(leaf.obj).scope)? else {
                continue;
            };
            if let ScopeKind::Instance { arch, .. } = self.hier.scope(region).kind
                && !archs.contains(&arch)
            {
                archs.push(arch);
            }
            res_regions.insert(crate::LeafId(k as u32), region);
        }
        // The shared variable a variable port denotes lives in its region's instance data,
        // so that region's architecture is lowered even when it owns no process (#188).
        for v in self.var_ports.values() {
            if let crate::VarObj::Region { region, .. } = *v
                && let ScopeKind::Instance { arch, .. } =
                    self.hier.scope(self.instance_of(region)).kind
                && !archs.contains(&arch)
            {
                archs.push(arch);
            }
        }
        // The regions of external shared variables hold their objects (#93), those of
        // external constants their run-time values (#255).
        for &(t, _) in self.ext_vars.values().chain(self.ext_consts.values()) {
            if let ScopeKind::Instance { arch, .. } = self.hier.scope(self.instance_of(t)).kind
                && !archs.contains(&arch)
            {
                archs.push(arch);
            }
        }
        let packages = run_time_packages(db);
        // A declaration's constant calling a subprogram of its own package runs before the
        // body is elaborated (LRM 14.4.2): E2114, also for constants only processes read
        // (#197).
        for &u in &packages {
            let scope = db.units[u].scope;
            for d in db.decls_with_instances(scope) {
                if let yavs_sem::decl::DeclKind::Object(o) = &db.decls[d].kind
                    && o.class == yavs_sem::decl::ObjClass::Constant
                    && let Some(init) = o.default
                {
                    crate::eval::Model::check_body_elaborated(db, init, scope)
                        .map_err(|d| vec![*d])?;
                }
            }
        }
        if archs.is_empty() {
            return Ok(());
        }
        // The program goes on from the elaboration session's when it can, so that the
        // simulation starts from the package state elaboration left (#383): same packages, no
        // coverage counters, no subprogram with external names lowered before they were bound.
        let mut start = None;
        let mut resumed = None;
        if let Some(s) = self.start.take()
            && !opts.coverage
            && s.pkgs == packages
        {
            let l = Lowerer::resume(db, s.lowerer);
            if !l.externals_early() {
                start = Some(s.state);
                resumed = Some(l);
            }
        }
        let mut l = match resumed {
            Some(l) => l,
            // Package objects first (#183), in analysis order, which puts a package after
            // those its declaration uses (LRM 14.4.2).
            // ponytail: a body using a package analyzed after its declaration is
            // initialised too early; order by the dependency graph if a design needs it.
            None => crate::pkgcall::package_lowerer(db, &packages, opts)?,
        };
        l.externals_bound(self.ext_unbound.clone());
        for p in &self.procs {
            if let Some(a) = &p.actual {
                l.port_actual(p.stmt, a.expr, a.formal, a.port);
            }
        }
        let mut var_pkgs = HashMap::new();
        for v in self.var_ports.values() {
            if let crate::VarObj::Package(d) = *v
                && !var_pkgs.contains_key(&d)
            {
                var_pkgs.insert(d, l.package_value(d)?);
            }
        }
        let mut elab_sigs = HashMap::new();
        for c in &self.elab_calls {
            if let std::collections::hash_map::Entry::Vacant(v) = elab_sigs.entry(c.func) {
                v.insert(crate::pkgcall::sig(&mut l, db, c.func)?);
            }
        }
        let mut idx = Vec::new();
        for &a in &archs {
            idx.push(l.architecture(a)?);
        }
        let var_slots = l.var_slots()?;
        let mut resolvers = HashMap::new();
        let mut res_types = HashMap::new();
        let mut res_index = HashMap::new();
        for leaf in &self.nets.leaves {
            if let Some(LeafRes::Func(d) | LeafRes::Whole(d)) = leaf.resolution
                && !resolvers.contains_key(&d)
            {
                resolvers.insert(d, l.subprogram(d)?);
            }
            if let Some(LeafRes::Func(d) | LeafRes::Whole(d)) = leaf.resolution
                && !res_types.contains_key(&d)
                && let Some(sp) = db.decls[d].kind.subprogram()
                && let ([p], Some(r)) = (&sp.params[..], sp.ret)
            {
                let tys = (l.ty(db.base(p.subtype))?, l.ty(db.base(r))?);
                res_types.insert(d, tys);
                res_index.insert(d, res_index_of(db, p.subtype));
            }
        }
        let lowered = l.finish()?;
        let mut units = HashMap::new();
        for (a, i) in archs.into_iter().zip(idx) {
            units.insert(a, lowered.units[i].clone());
        }
        let mut names = HashMap::new();
        // The regions processes run in, and those holding the targets of external names
        // (their thunks may name entities, #255).
        let regions = self
            .procs
            .iter()
            .map(|p| {
                let i = self.instance_of(p.scope);
                self.hier.scope(p.scope).parent.unwrap_or(i)
            })
            .chain(
                self.ext_vars
                    .values()
                    .chain(self.ext_consts.values())
                    .map(|&(t, _)| t),
            );
        for region in regions {
            let ScopeKind::Instance { arch, .. } = self.hier.scope(self.instance_of(region)).kind
            else {
                continue;
            };
            for n in units
                .get(&arch)
                .map(|u: &UnitIr| u.names.as_slice())
                .unwrap_or(&[])
            {
                let text = self.name_of(db, region, n.decl, n.instance);
                names.insert((region, n.decl, n.instance), text);
            }
        }
        let sources = yavs_interp::Sources::of(&lowered.program, &db.sources);
        self.ir = Some(Ir {
            sources,
            revision: crate::pkgcall::revision_of(db),
            program: Arc::new(lowered.program),
            units,
            resolvers,
            res_types,
            res_index,
            res_regions,
            init_regions,
            names,
            packages: lowered.packages,
            shared_size: lowered.shared_size,
            foreign: Vec::new(),
            foreign_searched: String::new(),
            models: Vec::new(),
            var_pkgs,
            elab_sigs,
            var_slots,
            start,
        });
        Ok(())
    }

    /// The instance whose data resolution function `d` reads when it resolves a signal
    /// declared in `obj_scope`: `None` for a function declared in a package (no instance
    /// data). Functions declared in blocks, generates or subprograms are not supported yet.
    fn res_region(
        &self,
        db: &Db,
        d: yavs_sem::DeclId,
        obj_scope: ScopeId,
    ) -> Result<Option<ScopeId>, Vec<Diagnostic>> {
        use yavs_sem::scope::ScopeKind as S;
        let ds = db.decls[d].scope;
        match db.scopes[ds].kind {
            S::Package | S::PackageBody => return Ok(None),
            S::Entity | S::Architecture => {}
            _ => {
                return Err(vec![
                    Diagnostic::error(
                        crate::codes::NOT_IMPLEMENTED,
                        format!(
                            "resolution function `{}` declared in a block, generate or \
                             subprogram is not implemented yet",
                            db.decls[d].name
                        ),
                    )
                    .with_primary(db.decls[d].span, "declared here")
                    .with_note("declare it in the architecture or a package (#164)"),
                ]);
            }
        }
        let i = self.instance_of(obj_scope);
        match self.hier.scope(i).kind {
            ScopeKind::Instance { entity, arch }
                if db.units[arch].scope == ds || db.units[entity].scope == ds =>
            {
                Ok(Some(i))
            }
            _ => Err(vec![
                Diagnostic::error(
                    crate::codes::INTERNAL,
                    format!(
                        "internal error: no instance declares resolution function `{}`",
                        db.decls[d].name
                    ),
                )
                .with_primary(db.decls[d].span, "this is a bug in yavs"),
            ]),
        }
    }

    /// `'path_name` (or `'instance_name`) of named entity `d` seen from `region` (LRM 16.2.5,
    /// lower case): a design unit names its instance, an object the region declaring it, a
    /// process label its process.
    fn name_of(&self, db: &Db, region: ScopeId, d: yavs_sem::DeclId, instance: bool) -> String {
        use yavs_sem::decl::DeclKind;
        let text = |s: ScopeId| {
            if instance {
                self.hier.instance_name(s, db, crate::Case::Lrm)
            } else {
                self.hier.path_name(s, crate::Case::Lrm)
            }
        };
        let chain: Vec<ScopeId> =
            std::iter::successors(Some(region), |&s| self.hier.scope(s).parent).collect();
        match &db.decls[d].kind {
            DeclKind::Unit(_) => {
                let i = self.instance_of(region);
                text(i)
            }
            DeclKind::Label(stmt) => {
                let p = chain.iter().flat_map(|&s| self.hier.scope(s).children.iter()).find(|&&c| {
                    matches!(self.hier.scope(c).kind, ScopeKind::Process { stmt: x } if x == *stmt)
                });
                match p {
                    Some(&p) => text(p),
                    // A block or generate label names its own scope (16.2.5), the nearest
                    // one: the region itself or a child of it, outwards.
                    None => {
                        let key = db.decls[d].name.key();
                        let own = chain.iter().find_map(|&s| {
                            std::iter::once(s)
                                .chain(self.hier.scope(s).children.iter().copied())
                                .find(|&c| {
                                    let sc = self.hier.scope(c);
                                    sc.name.key == key
                                        && matches!(
                                            sc.kind,
                                            ScopeKind::Block
                                                | ScopeKind::ForGenerate { .. }
                                                | ScopeKind::Generate { .. }
                                        )
                                })
                        });
                        match own {
                            Some(c) => text(c),
                            None => format!("{}{key}:", text(region)),
                        }
                    }
                }
            }
            _ => {
                let declared = chain.iter().copied().find(|&s| {
                    self.sigs.contains_key(&(s, d))
                        || self
                            .model_vals
                            .get(s.index())
                            .is_some_and(|m| m.contains_key(&d))
                });
                let name = db.decls[d].name.to_string().to_ascii_lowercase();
                let own = match &db.decls[d].kind {
                    // A subprogram names itself with its profile and a trailing colon.
                    DeclKind::Subprogram(_) => format!("{}:", subprogram_element(db, d)),
                    _ => name,
                };
                // An item of a package or package instance is named by its package (16.2.5):
                // `:lib:pkg:` for a library unit, else the declaring region plus the instance.
                let (pkg, lib) = package_prefix(db, db.decls[d].scope);
                if let Some(lib) = lib {
                    return format!("{lib}{pkg}{own}");
                }
                let local = if declared.is_none() || !pkg.is_empty() {
                    local_prefix(db, db.decls[d].scope)
                } else {
                    String::new()
                };
                let local = if pkg.is_empty() { local } else { pkg };
                format!("{}{local}{own}", text(declared.unwrap_or(region)))
            }
        }
    }

    /// [`Elaborated::build_full`] for a run that builds once: the package state elaboration
    /// left (#383) is moved into the runtime instead of copied, so the two copies never
    /// coexist (#409). A later build initialises the packages again and replays the
    /// elaboration-time calls (same result, slower).
    pub fn build_full_once(
        &mut self,
        cfg: Config,
    ) -> Result<(SimControl, Option<Built>), SimError> {
        let start = self.ir.as_mut().and_then(|ir| ir.start.take());
        self.build_runtime_with(cfg, None, start)
    }

    /// The kernel model with interpreter processes (after [`Elaborated::lower`]). Can be
    /// called again for an in-process restart.
    pub fn build(&self, cfg: Config) -> Result<SimControl, SimError> {
        self.build_runtime(cfg).map(|(s, _)| s)
    }

    /// [`Elaborated::build`], plus the interpreter runtime of the processes (none for a design
    /// without them): the CLI reads the VHDL assert counts from it at the end of the run.
    pub fn build_runtime(
        &self,
        cfg: Config,
    ) -> Result<(SimControl, Option<Arc<Runtime>>), SimError> {
        self.build_full(cfg).map(|(s, b)| (s, b.map(|b| b.rt)))
    }

    /// [`Elaborated::build_runtime`] with the shared state and where the shared variables
    /// are (VHPI and waves, #309).
    pub fn build_full(&self, cfg: Config) -> Result<(SimControl, Option<Built>), SimError> {
        let start = self.ir.as_ref().and_then(|ir| ir.start.clone());
        self.build_runtime_with(cfg, None, start)
    }

    /// [`Elaborated::build_runtime`] with every process attached to the debug session (ir.md
    /// 3.4), named by [`Elaborated::proc_name`] in the session's process table. Run it with
    /// one thread: stops inside a process need it.
    pub fn build_debug(
        &self,
        cfg: Config,
        session: &Arc<DebugSession>,
    ) -> Result<(SimControl, Option<Built>), SimError> {
        let start = self.ir.as_ref().and_then(|ir| ir.start.clone());
        self.build_runtime_with(cfg, Some(session), start)
    }

    /// Free what only the build and the tools that inspect the design (a plugin, the waveform
    /// recorder, coverage, the debugger) read, for a run that has built its simulation: the
    /// hierarchy, the net graph, the process instances and the per-instance values (#421).
    /// What remains answers [`Elaborated::runtime_diagnostic`].
    pub fn release_for_run(&mut self) {
        macro_rules! free {
            ($($f:ident),*) => {$(drop(std::mem::take(&mut self.$f));)*};
        }
        free!(
            hier,
            nets,
            procs,
            foreign_archs,
            ext_procs,
            model_vals,
            sigs,
            regions,
            var_ports,
            ext_vars,
            ext_consts,
            pkg_ext,
            ext_unbound,
            var_shapes,
            shared_vars,
            convs,
            elab_calls
        );
        self.start = None;
    }

    /// The path of a process instance as reports and the debugger show it: `:top:u1:p`.
    pub fn proc_name(&self, p: &crate::ProcInst) -> String {
        let n = self.hier.path_name(p.scope, crate::Case::Lrm);
        n.strip_suffix(':').unwrap_or(&n).to_string()
    }

    fn build_runtime_with(
        &self,
        cfg: Config,
        dbg: Option<&Arc<DebugSession>>,
        start: Option<Arc<crate::pkgcall::StartState>>,
    ) -> Result<(SimControl, Option<Built>), SimError> {
        let attach = |proc: InterpProcess, p: &crate::ProcInst| match dbg {
            Some(s) => proc.with_debug(s.clone(), &self.proc_name(p)),
            None => proc,
        };
        let Some(ir) = &self.ir else {
            if self.procs.is_empty() {
                return self
                    .build_with(cfg, |_, _| Err(sim_err("no IR".into())))
                    .map(|s| (s, None));
            }
            return Err(sim_err(
                "the design has processes but was not lowered".into(),
            ));
        };
        // The start state's shared heap and variables: moved when this build owns the state.
        let mut start_shared = None;
        let start = start.map(|a| match Arc::try_unwrap(a) {
            Ok(mut s) => {
                start_shared = Some(std::mem::take(&mut s.shared));
                Arc::new(s)
            }
            Err(a) => {
                start_shared = Some(a.shared.clone());
                a
            }
        });
        let shared = Arc::new(Mutex::new(match start_shared {
            Some(mut st) => {
                st.vars.resize(ir.shared_size as usize, 0);
                st
            }
            None => SharedState::new(vec![0; ir.shared_size as usize]),
        }));
        // Cells of instance-level constants and shared variables, computed once per instance.
        let mut once: HashMap<(ScopeId, yavs_sem::DeclId), CallVal> = HashMap::new();
        // Package data, created once in order: each initialisation sees the packages before it.
        let rt = crate::pkgcall::init_packages(
            &ir.program,
            self.cov_rows() * ir.program.cov.len(),
            &ir.packages,
            &ir.sources,
            &ir.foreign,
            &ir.foreign_searched,
            &ir.models,
            &shared,
            time_resolution(cfg.resolution),
            ir.revision,
            start.as_ref().map(|s| &s.data[..]),
            |p, d| {
                // Descriptors of the package's array signals (none for an unbound alias of an
                // external signal: its subprogram fails when called).
                for s in &p.signals {
                    let Some(off) = s.dope else { continue };
                    if self.ext_unbound.contains_key(&s.decl) {
                        continue;
                    }
                    let o = self
                        .sigs
                        .iter()
                        .find(|((_, x), _)| *x == s.decl)
                        .map(|(_, &o)| o)
                        .ok_or_else(|| {
                            sim_err(format!("package signal `{}` not elaborated", s.name))
                        })?;
                    write_desc(
                        &ir.program,
                        &s.name,
                        &self.nets.obj(o).shape,
                        off,
                        &mut d.data,
                    )?;
                }
                Ok(())
            },
        )?;
        match &start {
            // The packages hold what elaboration-time calls left; so does the assert state.
            Some(s) => rt.inherit(&s.rt),
            // The side effects of elaboration-time calls on package state, in their order
            // (#77).
            None => crate::pkgcall::replay(&rt, &shared, &ir.elab_sigs, &self.elab_calls)
                .map_err(|m| SimError::new(ErrorKind::Runtime(m)))?,
        }
        let mut insts: HashMap<ScopeId, Arc<InstData>> = HashMap::new();
        // Instance data waiting for its external shared variables (#93).
        let mut base: HashMap<ScopeId, InstData> = HashMap::new();
        // The cells of the external constants and shared variables package subprograms name,
        // before any process runs (#255).
        for p in &ir.packages {
            for v in &p.ext_consts {
                let Some(&ps) = self.pkg_ext.get(&v.decl) else {
                    continue; // unbound: its subprogram fails when called
                };
                let cv = match self.value(ps, v.decl) {
                    Some(val) => encode_value(&rt.prog, v, val)?,
                    None => {
                        let &(t, td) = self.ext_consts.get(&(ps, v.decl)).ok_or_else(|| {
                            sim_err(format!("external constant `{}` not bound", v.name))
                        })?;
                        self.target_const(
                            &rt, ir, t, td, &shared, &mut once, &mut insts, &mut base,
                        )?
                    }
                };
                let mut s = shared
                    .lock()
                    .map_err(|_| sim_err("shared state poisoned".into()))?;
                write_cell(&rt.prog, v, cv, &mut s.vars)?;
            }
            for v in &p.ext_vars {
                let Some(&ps) = self.pkg_ext.get(&v.decl) else {
                    continue;
                };
                let &(region, decl) = self
                    .ext_vars
                    .get(&(ps, v.decl))
                    .ok_or_else(|| sim_err(format!("external variable `{}` not bound", v.name)))?;
                let obj = crate::VarObj::Region { region, decl };
                let h = self.var_handle(&rt, ir, obj, &shared, &mut once, &mut insts, &mut base)?;
                let at = v.off as usize;
                let mut s = shared
                    .lock()
                    .map_err(|_| sim_err("shared state poisoned".into()))?;
                s.vars
                    .get_mut(at..at + 8)
                    .ok_or_else(|| sim_err("shared variables area too short".into()))?
                    .copy_from_slice(&h.to_le_bytes());
            }
        }
        for &region in &ir.init_regions {
            match self.region_data(&rt, ir, region, &shared, &mut once, &mut insts, &mut base) {
                // ponytail: a constant initialised from a signal has no signal context here
                // (#453); the instance is left unevaluated, as before, until that is fixed.
                Err(e)
                    if matches!(&e.kind, ErrorKind::Runtime(m)
                        if m == "signal or wait operation during elaboration") => {}
                r => {
                    r?;
                }
            }
        }
        // Instance data of the regions whose resolution functions read it (#164).
        let mut res_inst: HashMap<crate::LeafId, Arc<InstData>> = HashMap::new();
        for (&leaf, &region) in &ir.res_regions {
            let d = self.region_data(&rt, ir, region, &shared, &mut once, &mut insts, &mut base)?;
            res_inst.insert(leaf, d);
        }
        let res = |d: yavs_sem::DeclId,
                   leaf: crate::LeafId|
         -> Result<(yavs_kernel::Resolver, bool), SimError> {
            let f = *ir
                .resolvers
                .get(&d)
                .ok_or_else(|| sim_err("resolution function not lowered".into()))?;
            let rt = rt.clone();
            // A function touching shared variables runs with the shared state.
            let serial = rt.prog.is_serial(f);
            let sh = serial.then(|| shared.clone());
            let inst = res_inst.get(&leaf).cloned().unwrap_or_default();
            let index = *ir
                .res_index
                .get(&d)
                .ok_or_else(|| sim_err("resolution index not lowered".into()))?;
            if let Some(LeafRes::Whole(_)) = self.nets.leaf(leaf).resolution {
                let tys = *ir.res_types.get(&d).ok_or_else(|| {
                    sim_err("types of a composite resolution function not lowered".into())
                })?;
                let w = self.nets.whole_of(leaf).ok_or_else(|| {
                    sim_err("a leaf resolved as a whole outside any composite".into())
                })?;
                let r = crate::whole::resolver(rt, f, inst, tys, index, &w.shape, sh);
                return r.map(|r| (r, serial)).map_err(sim_err);
            }
            // Element-wise: one call per element of the leaf (#293).
            let (param, _) = *ir
                .res_types
                .get(&d)
                .ok_or_else(|| sim_err("types of a resolution function not lowered".into()))?;
            let pes = match rt.prog.types.get(param) {
                Some(yavs_ir::Ty::Array { elem, .. }) => {
                    rt.prog.types.get(*elem).and_then(yavs_ir::Ty::elem_size)
                }
                _ => None,
            };
            let (w, logic) = match self.nets.leaf(leaf).ty.elem {
                yavs_kernel::Elem::Bytes(es) => (es as usize, None),
                yavs_kernel::Elem::Logic => {
                    let (_, ret) = *ir.res_types.get(&d).ok_or_else(|| {
                        sim_err("types of a resolution function not lowered".into())
                    })?;
                    let k = yavs_ir::Ty::logic_leaf(&rt.prog.types, ret).ok_or_else(|| {
                        sim_err("logic signal resolved by a non-logic function".into())
                    })?;
                    (1, Some(k))
                }
            };
            // A resolution function is pure (E1225): the same driving values give the same
            // result, so one that reported nothing is remembered (R3, `docs/decisions.md`).
            // One that touches shared state is not pure: no memo.
            let memo = Mutex::new((HashMap::<Vec<u8>, Vec<u8>>::new(), Vec::new()));
            let f = Arc::new(move |vals: &[&[u8]], out: &mut [u8]| {
                let (memo, key) = &mut *memo.lock().unwrap_or_else(|e| e.into_inner());
                for (k, o) in out.chunks_mut(w).enumerate() {
                    let el: Vec<&[u8]> = vals
                        .iter()
                        .map(|v| v.get(k * w..(k + 1) * w).unwrap_or(&[]))
                        .collect();
                    key.clear();
                    el.iter().for_each(|v| key.extend_from_slice(v));
                    if let Some(r) = memo.get(&*key) {
                        o.copy_from_slice(r);
                        continue;
                    }
                    let reports = yavs_kernel::callout::captured();
                    let es = pes.unwrap_or(w as u64);
                    let a = (rt.clone(), f, inst.clone());
                    yavs_interp::resolve(a.0, a.1, a.2, &el, o, logic, index, es, sh.as_ref())
                        .map_err(|e| e.to_string())?;
                    if reports.is_some()
                        && sh.is_none()
                        && yavs_kernel::callout::captured() == reports
                        && memo.len() < RES_MEMO
                    {
                        memo.insert(key.clone(), o.to_vec());
                    }
                }
                Ok(())
            });
            Ok((f, serial))
        };
        let mut proc_index: HashMap<_, HashMap<_, usize>> = HashMap::new();
        let mut s = self.build_res(
            cfg,
            |p, ds| {
                let i = self.instance_of(p.scope);
                let ScopeKind::Instance { arch, .. } = self.hier.scope(i).kind else {
                    return Err(sim_err("process outside an instance".into()));
                };
                let u = ir
                    .units
                    .get(&arch)
                    .ok_or_else(|| sim_err("architecture not lowered".into()))?;
                // One instance data per region (instance, block, generate iteration): values and
                // signals are looked up from the process's enclosing region outwards.
                let region = self.hier.scope(p.scope).parent.unwrap_or(i);
                let inst =
                    self.region_data(&rt, ir, region, &shared, &mut once, &mut insts, &mut base)?;
                if let Some(a) = &p.actual {
                    let proc = self.actual_process(&rt, u, a, p, ds, inst)?;
                    return Ok(Box::new(attach(proc, p)));
                }
                // Processes by statement, indexed once per architecture (#435).
                let by_stmt = proc_index.entry(arch).or_insert_with(|| {
                    let mut m = HashMap::new();
                    for (i, x) in u.processes.iter().enumerate() {
                        if x.actual.is_none() {
                            m.entry(x.stmt).or_insert(i);
                        }
                    }
                    m
                });
                let pir = by_stmt
                    .get(&p.stmt)
                    .map(|&i| &u.processes[i])
                    .ok_or_else(|| {
                        sim_err(format!(
                            "{}: processes inside generates and blocks are not lowered yet",
                            self.hier.path_name(p.scope, crate::Case::Original)
                        ))
                    })?;
                // One driver slot per kernel signal of each longest static prefix, in order
                // (a record prefix covers one leaf per field), matching `ProcessIr::drivers`.
                let entries: Vec<_> = p.lsps.iter().flatten().collect();
                if entries.len() != pir.drivers.len() {
                    return Err(sim_err(format!(
                        "{}: {} driver slots lowered, {} elaborated",
                        self.hier.path_name(p.scope, crate::Case::Original),
                        pir.drivers.len(),
                        entries.len()
                    )));
                }
                let mut drivers = Vec::with_capacity(pir.drivers.len());
                for (&sig, (d, _, range)) in pir.drivers.iter().zip(entries) {
                    let pos = p.drivers.iter().position(|x| x == d).ok_or_else(|| {
                        SimError::new(ErrorKind::Internal("driver not owned".into()))
                    })?;
                    drivers.push(ProcDriver {
                        id: ds[pos],
                        sig,
                        range: *range,
                    });
                }
                let serial = ir.program.is_serial(pir.func);
                let proc = InterpProcess::new(
                    rt.clone(),
                    pir.func,
                    inst,
                    drivers,
                    serial.then(|| shared.clone()),
                )?;
                Ok(Box::new(attach(proc, p)))
            },
            res,
        )?;
        // Where each shared variable's slot is: a region's is the value its thunk gave
        // (`once`, the cell's content), when a process of the design uses it; a package's
        // comes from its slot thunk.
        let mut vars = HashMap::new();
        for &(d, f, ty) in &ir.var_slots {
            match f {
                Some(f) => {
                    let inst = Arc::new(InstData::default());
                    let v = yavs_interp::call_values_shared(
                        rt.clone(),
                        f,
                        inst,
                        &[],
                        None,
                        Some(&shared),
                    )?;
                    let Some(CallVal::Cell(slot)) = v else {
                        return Err(sim_err("shared variable slot not found".into()));
                    };
                    vars.insert((None, d), (slot, ty));
                }
                None => {
                    // Sorted: the thunks of regions no process uses run in a fixed order.
                    let mut regions: Vec<_> = self
                        .shared_vars
                        .iter()
                        .filter(|&&(_, x)| x == d)
                        .map(|&(r, _)| r)
                        .collect();
                    regions.sort();
                    for r in regions {
                        // A region no process uses has no slot yet (#342).
                        let v = self.target_const(
                            &rt, ir, r, d, &shared, &mut once, &mut insts, &mut base,
                        )?;
                        if let CallVal::Cell(slot) = v {
                            vars.insert((Some(r), d), (slot, ty));
                        }
                    }
                }
            }
        }
        // The shared heap is collected between cycles (#303).
        s.subscribe(Box::new(yavs_interp::SharedGc {
            rt: rt.clone(),
            shared: shared.clone(),
        }));
        Ok((s, Some(Built { rt, shared, vars })))
    }

    /// The equivalent process of an expression actual (`ProcInst::actual`): the region's
    /// instance data plus the formal as one more signal, which driver slot 0 drives, and the
    /// formal's descriptor for the array check.
    fn actual_process(
        &self,
        rt: &Arc<Runtime>,
        u: &UnitIr,
        pa: &crate::PortActual,
        p: &crate::ProcInst,
        ds: &[yavs_kernel::DriverId],
        inst: Arc<InstData>,
    ) -> Result<InterpProcess, SimError> {
        let path = || self.hier.path_name(p.scope, crate::Case::Original);
        let (pir, a) = u
            .processes
            .iter()
            .find_map(|x| {
                x.actual
                    .as_ref()
                    .filter(|a| a.expr == pa.expr)
                    .map(|a| (x, a))
            })
            .ok_or_else(|| sim_err(format!("{}: port actual not lowered", path())))?;
        let mut inst = (*inst).clone();
        if let Some(off) = a.dope {
            write_desc(&rt.prog, &path(), &pa.shape, (off, a.ty), &mut inst.data)?;
        }
        if let Some(off) = a.bounds {
            let (Some(b), Some(d)) = (
                pa.bounds,
                inst.data.get_mut(off as usize..off as usize + 24),
            ) else {
                return Err(sim_err(format!(
                    "{}: port actual range not elaborated",
                    path()
                )));
            };
            for (w, x) in d.as_chunks_mut::<8>().0.iter_mut().zip(b) {
                *w = x.to_le_bytes();
            }
        }
        // Slot `k` drives the formal's leaf `k` (one for a scalar or an array of scalars).
        let mut drivers = Vec::new();
        for (d, leaf, range) in p.lsps.concat() {
            let pos = p.drivers.iter().position(|&x| x == d);
            let Some(&id) = pos.and_then(|k| ds.get(k)) else {
                return Err(sim_err(format!("{}: port actual driver not owned", path())));
            };
            drivers.push(ProcDriver {
                id,
                sig: inst.sigs.len() as u32,
                range,
            });
            inst.sigs.push(InstSig {
                signal: yavs_kernel::SignalId(leaf.0),
                ty: self.nets.leaf(leaf).ty,
                off: 0,
            });
        }
        InterpProcess::new(rt.clone(), pir.func, Arc::new(inst), drivers, None)
    }

    /// The instance data of `region` (instance, block, generate iteration), built once.
    /// Aliases of external shared variables copy the variable's cell from its region's data,
    /// taken before that region's own aliases are filled in (`base`), so regions may name
    /// each other's variables (#93).
    #[allow(clippy::too_many_arguments)]
    fn region_data(
        &self,
        rt: &Arc<Runtime>,
        ir: &Ir,
        region: ScopeId,
        shared: &Arc<Mutex<SharedState>>,
        once: &mut HashMap<(ScopeId, yavs_sem::DeclId), CallVal>,
        insts: &mut HashMap<ScopeId, Arc<InstData>>,
        base: &mut HashMap<ScopeId, InstData>,
    ) -> Result<Arc<InstData>, SimError> {
        if let Some(d) = insts.get(&region) {
            return Ok(d.clone());
        }
        let u = unit_ir(self, ir, region)?;
        let mut d = match base.remove(&region) {
            Some(d) => d,
            None => self.inst_data(rt, ir, u, region, shared, once, insts, base)?,
        };
        for v in &u.ext_vars {
            let Some((t, td)) = self.ext_var(region, v.decl) else {
                continue; // an alias of a nested region: never read from this one
            };
            let tu = unit_ir(self, ir, t)?;
            let c = tu
                .consts
                .iter()
                .find(|c| c.decl == td)
                .ok_or_else(|| sim_err(format!("shared variable `{}` not lowered", v.name)))?;
            let from = c.off as usize;
            let mut cell = [0u8; 8];
            if t == region {
                cell.copy_from_slice(&d.data[from..from + 8]);
            } else {
                if !insts.contains_key(&t) && !base.contains_key(&t) {
                    let x = self.inst_data(rt, ir, tu, t, shared, once, insts, base)?;
                    base.insert(t, x);
                }
                let tdata = match insts.get(&t) {
                    Some(x) => &x.data,
                    None => &base[&t].data,
                };
                cell.copy_from_slice(&tdata[from..from + 8]);
            }
            let at = v.off as usize;
            d.data[at..at + 8].copy_from_slice(&cell);
        }
        // Aliases of external constants only a run-time thunk computes (#255).
        for v in &u.values {
            let Some((t, td)) = self.ext_const(region, v.decl) else {
                continue;
            };
            let cv = self.target_const(rt, ir, t, td, shared, once, insts, base)?;
            write_cell(&rt.prog, v, cv, &mut d.data)?;
        }
        let d = Arc::new(d);
        insts.insert(region, d.clone());
        Ok(d)
    }

    /// The constant alias `d` of `scope` (or of a region around it) is bound to, when only
    /// the constant's run-time thunk computes its value.
    fn ext_const(
        &self,
        scope: ScopeId,
        d: yavs_sem::DeclId,
    ) -> Option<(ScopeId, yavs_sem::DeclId)> {
        let mut cur = Some(scope);
        while let Some(s) = cur {
            if let Some(&t) = self.ext_consts.get(&(s, d)) {
                return Some(t);
            }
            cur = self.hier.scope(s).parent;
        }
        None
    }

    /// The value constant `td` of region `t` got from its thunk (the region's instance data
    /// built first if needed, #255).
    #[allow(clippy::too_many_arguments)]
    fn target_const(
        &self,
        rt: &Arc<Runtime>,
        ir: &Ir,
        t: ScopeId,
        td: yavs_sem::DeclId,
        shared: &Arc<Mutex<SharedState>>,
        once: &mut HashMap<(ScopeId, yavs_sem::DeclId), CallVal>,
        insts: &mut HashMap<ScopeId, Arc<InstData>>,
        base: &mut HashMap<ScopeId, InstData>,
    ) -> Result<CallVal, SimError> {
        let tu = unit_ir(self, ir, t)?;
        let c = tu
            .consts
            .iter()
            .find(|c| c.decl == td)
            .ok_or_else(|| sim_err("external constant without a run-time value".into()))?;
        let key = if c.instance_level {
            (self.instance_of(t), td)
        } else {
            (t, td)
        };
        if !once.contains_key(&key) && !insts.contains_key(&t) && !base.contains_key(&t) {
            let x = self.inst_data(rt, ir, tu, t, shared, once, insts, base)?;
            base.insert(t, x);
        }
        once.get(&key)
            .cloned()
            .ok_or_else(|| sim_err(format!("constant `{}` has no value", c.name)))
    }

    /// The shared variable alias `d` of `scope` (or of a region around it) is bound to.
    fn ext_var(&self, scope: ScopeId, d: yavs_sem::DeclId) -> Option<(ScopeId, yavs_sem::DeclId)> {
        let mut cur = Some(scope);
        while let Some(s) = cur {
            if let Some(&t) = self.ext_vars.get(&(s, d)) {
                return Some(t);
            }
            cur = self.hier.scope(s).parent;
        }
        None
    }

    /// The handle of protected object `obj` of a variable port: read from
    /// the instance data of the shared variable's region (built first if needed: it is an
    /// enclosing region), or from its package (#188).
    #[allow(clippy::too_many_arguments)]
    fn var_handle(
        &self,
        rt: &Arc<Runtime>,
        ir: &Ir,
        obj: crate::VarObj,
        shared: &Arc<Mutex<SharedState>>,
        once: &mut HashMap<(ScopeId, yavs_sem::DeclId), CallVal>,
        insts: &mut HashMap<ScopeId, Arc<InstData>>,
        base: &mut HashMap<ScopeId, InstData>,
    ) -> Result<u64, SimError> {
        match obj {
            crate::VarObj::Region { region, decl } => {
                let data = self.region_data(rt, ir, region, shared, once, insts, base)?;
                let ScopeKind::Instance { arch, .. } =
                    self.hier.scope(self.instance_of(region)).kind
                else {
                    return Err(sim_err("region outside an instance".into()));
                };
                let off = ir
                    .units
                    .get(&arch)
                    .and_then(|u| u.consts.iter().find(|c| c.decl == decl))
                    .map(|c| c.off as usize)
                    .ok_or_else(|| {
                        sim_err("shared variable of a variable port not lowered".into())
                    })?;
                let mut b = [0; 8];
                b.copy_from_slice(&data.data[off..off + 8]);
                Ok(u64::from_le_bytes(b))
            }
            crate::VarObj::Package(decl) => {
                let f = *ir
                    .var_pkgs
                    .get(&decl)
                    .ok_or_else(|| sim_err("package shared variable not lowered".into()))?;
                match yavs_interp::call_values_shared(
                    rt.clone(),
                    f,
                    Arc::new(InstData::default()),
                    &[],
                    None,
                    Some(shared),
                )? {
                    Some(CallVal::Cell(h)) => Ok(h),
                    _ => Err(sim_err("package shared variable has no handle".into())),
                }
            }
        }
    }

    /// Instance data of region `i`: constants from their thunks, the signal table.
    #[allow(clippy::too_many_arguments)]
    fn inst_data(
        &self,
        rt: &Arc<Runtime>,
        ir: &Ir,
        u: &UnitIr,
        i: ScopeId,
        shared: &Arc<Mutex<SharedState>>,
        once: &mut HashMap<(ScopeId, yavs_sem::DeclId), CallVal>,
        insts: &mut HashMap<ScopeId, Arc<InstData>>,
        base: &mut HashMap<ScopeId, InstData>,
    ) -> Result<InstData, SimError> {
        let cov_base = u32::try_from((i.index() + 1) * rt.prog.cov.len())
            .map_err(|_| sim_err("too many coverage counters".into()))?;
        let mut inst = InstData {
            data: vec![0; u.inst_size as usize],
            cov_base,
            ..Default::default()
        };
        // Generics and constants elaboration computed; the rest come from their thunks.
        let mut known = std::collections::HashSet::new();
        for v in &u.values {
            if let Some(val) = self.value(i, v.decl) {
                let w = UnitIr::write_value(&rt.prog, v, val, &mut inst.data).or_else(|m| {
                    // Records (of scalars and records): marshalled like call arguments.
                    match crate::value::encode(&rt.prog, v.ty, val) {
                        Some(CallVal::Composite { data, desc }) => {
                            UnitIr::write_composite(v.off, &data, &desc, &mut inst.data);
                            Ok(())
                        }
                        _ => Err(m),
                    }
                });
                match w {
                    Ok(()) => {
                        known.insert(v.decl);
                    }
                    // A composite value with no encoding here comes from its thunk instead.
                    Err(_)
                        if u.consts
                            .iter()
                            .any(|c| c.decl == v.decl && c.comp.is_some()) => {}
                    Err(m) => return Err(sim_err(format!("instance data: {m}"))),
                }
            }
            // ponytail: a value without an elaborated value here belongs to a nested region
            // (a generate parameter, say) that processes of this region never read; values of
            // this region that elaboration could not fold are E2106 in elaboration already.
        }
        if let Some(ir) = &self.ir {
            for n in &u.names {
                if let Some(t) = ir.names.get(&(i, n.decl, n.instance)) {
                    UnitIr::write_string(n.off, t, &mut inst.data)
                        .map_err(|m| sim_err(format!("instance data: {m}")))?;
                }
            }
        }
        // Signals first: a constant's thunk may read a port's or signal's bounds from its
        // descriptor (`p'length`, `p'element'length`).
        for s in &u.signals {
            let Some(o) = self.signal(i, s.decl) else {
                // A signal of a nested region (block, generate): processes of this region never
                // name it, so its slot holds no kernel signal (any use is a BadRef error).
                inst.sigs.push(InstSig {
                    signal: yavs_kernel::SignalId(u32::MAX),
                    ty: yavs_kernel::SignalType::bytes(1, 0),
                    off: 0,
                });
                continue;
            };
            if let Some(off) = s.dope {
                // The signal's descriptor in this instance (ports: the actual's bounds).
                write_desc(
                    &rt.prog,
                    &s.name,
                    &self.nets.obj(o).shape,
                    off,
                    &mut inst.data,
                )?;
            }
            // The leaf the entry names (a record signal has one entry per field leaf), by
            // position: a collapsed port's leaves are its actual's (`collapse.rs`).
            let obj = self.nets.obj(o);
            let k = match s.path.is_empty() {
                true => Some(0),
                false => (obj.shape.leaves().iter()).position(|(p, _, _)| *p == s.path),
            };
            let Some(leaf) = k.map(|k| crate::LeafId(obj.leaves.0.0 + k as u32)) else {
                return Err(sim_err(format!(
                    "signal `{}` has no kernel signal for field path {:?}",
                    s.name, s.path
                )));
            };
            // A port collapsed into part of its actual's leaf: that part (`collapse.rs`).
            let mut ty = self.nets.leaf(leaf).ty;
            let off = obj.part.map_or(0, |r| {
                ty.len = r.len;
                r.offset
            });
            inst.sigs.push(InstSig {
                signal: yavs_kernel::SignalId(leaf.0),
                ty,
                off,
            });
        }
        // Variable ports before the constants, whose thunks may call their methods. A port
        // of a nested region (a block) has no object seen from here; processes of this
        // region never name it.
        for &(d, off) in &u.var_ports {
            if let Some(obj) = self.var_port(i, d) {
                let h = self.var_handle(rt, ir, obj, shared, once, insts, base)?;
                inst.data[off as usize..off as usize + 8].copy_from_slice(&h.to_le_bytes());
            }
        }
        for c in u.consts.iter().filter(|c| !known.contains(&c.decl)) {
            // An instance-level constant or shared variable is one object for all regions of
            // the instance (#190); a shared variable of a block or generate one per elaboration
            // of its declarative part, shared by the nested regions (#249): its thunk runs once
            // per declaring region.
            let key = if c.instance_level {
                Some((self.instance_of(i), c.decl))
            } else if self.shared_vars.iter().any(|&(_, d)| d == c.decl) {
                let mut cur = Some(i);
                while let Some(r) = cur.filter(|&r| !self.shared_vars.contains(&(r, c.decl))) {
                    cur = self.hier.scope(r).parent;
                }
                // Declared in a region nested in this one: no process of this region names it.
                let Some(r) = cur else { continue };
                Some((r, c.decl))
            } else {
                // Kept for the aliases of external constants naming it (#255).
                Some((i, c.decl))
            };
            let cached = key.and_then(|k| once.get(&k).cloned());
            let v = match cached {
                Some(v) => v,
                None => {
                    // With the shared state: a shared protected variable's thunk creates its
                    // object.
                    let v = yavs_interp::call_values_shared(
                        rt.clone(),
                        c.init,
                        Arc::new(inst.clone()),
                        &[],
                        c.comp,
                        Some(shared),
                    )?
                    .ok_or_else(|| sim_err(format!("constant `{}` has no value", c.name)))?;
                    if let Some(k) = key {
                        once.insert(k, v.clone());
                    }
                    v
                }
            };
            match v {
                CallVal::Cell(v) => {
                    let n = c.w.bytes() as usize;
                    inst.data[c.off as usize..c.off as usize + n]
                        .copy_from_slice(&v.to_le_bytes()[..n]);
                }
                CallVal::Composite { data, desc } => {
                    UnitIr::write_composite(c.off, &data, &desc, &mut inst.data);
                }
            }
        }
        Ok(inst)
    }
}

/// Write the descriptor of signal `s` of shape `shape` at `off` of `data`: the IR
/// descriptor layout of its type `ty` (`Ty::desc_size`): an array's dimensions, then its
/// element's descriptor unless the type fixes it; a record's fields without a fixed one.
fn write_desc(
    prog: &yavs_ir::Program,
    name: &str,
    shape: &crate::hier::Shape,
    (off, ty): (u32, yavs_ir::TyId),
    data: &mut [u8],
) -> Result<(), SimError> {
    use crate::hier::Shape;
    fn go(
        types: &yavs_ir::IdVec<yavs_ir::TyId, yavs_ir::Ty>,
        ty: yavs_ir::TyId,
        shape: &Shape,
        out: &mut Vec<u8>,
    ) -> Option<()> {
        match (types.get(ty)?, shape) {
            (
                yavs_ir::Ty::Array {
                    elem, elem_desc, ..
                },
                Shape::Array { dims, elem: es, .. },
            ) => {
                for b in dims {
                    let dir = i64::from(b.dir == yavs_sem::hir::Direction::Downto);
                    for x in [b.left, b.right, dir] {
                        out.extend_from_slice(&x.to_le_bytes());
                    }
                }
                if elem_desc.is_none() {
                    go(types, *elem, es, out)?;
                }
            }
            (yavs_ir::Ty::Record { fields }, Shape::Record { fields: fs, .. }) => {
                for (f, (_, fs)) in fields.iter().zip(fs) {
                    if f.desc.is_none() {
                        go(types, f.ty, fs, out)?;
                    }
                }
            }
            (yavs_ir::Ty::Array { .. } | yavs_ir::Ty::Record { .. }, _) => return None,
            _ => {}
        }
        Some(())
    }
    let mut desc = Vec::new();
    go(&prog.types, ty, shape, &mut desc)
        .ok_or_else(|| sim_err(format!("shape of signal `{name}` does not match its type")))?;
    let n = yavs_ir::Ty::desc_size(&prog.types, ty)
        .ok_or_else(|| sim_err(format!("no descriptor size for `{name}`")))? as usize;
    let at = off as usize;
    if desc.len() != n || data.len() < at + n {
        return Err(sim_err(format!("descriptor of signal `{name}`")));
    }
    data[at..at + n].copy_from_slice(&desc);
    Ok(())
}

/// Packages outside std with objects that exist at run time: signals, shared variables,
/// files, or constants sem did not fold (in the declaration or the body), or the aliases of
/// external names its subprograms use (#255).
pub(crate) fn run_time_packages(db: &Db) -> Vec<UnitId> {
    use yavs_sem::decl::{DeclKind, ObjClass};
    let run_time = |d: yavs_sem::DeclId| match &db.decls[d].kind {
        DeclKind::Object(o) => match o.class {
            ObjClass::Constant => o.default.is_some_and(|e| db.exprs[e].value.is_none()),
            _ => true,
        },
        _ => db.decl_subprograms(d).into_iter().any(|m| {
            !db.external_uses(yavs_sem::external::ExtOwner::Sub(m))
                .is_empty()
        }),
    };
    db.units
        .iter()
        .filter(|(_, u)| db.libs[u.lib].name.key.as_str() != "std")
        .filter(|&(id, _)| {
            db.package_regions(id).is_some_and(|rs| {
                rs.into_iter()
                    .any(|s| db.decls_with_instances(s).into_iter().any(run_time))
            })
        })
        .map(|(id, _)| id)
        .collect()
}

/// `name[type, type return type]` (LRM 16.2.5, lower case): a subprogram's path element.
fn subprogram_element(db: &Db, d: yavs_sem::DeclId) -> String {
    let decl = &db.decls[d];
    let sub = decl.kind.subprogram().expect("a subprogram declaration");
    let ty = |st| db.type_name(db.base(st)).to_ascii_lowercase();
    let params: Vec<String> = sub.params.iter().map(|p| ty(p.subtype)).collect();
    let mut sig = params.join(",");
    if let Some(r) = sub.ret {
        if !sig.is_empty() {
            sig.push(' ');
        }
        sig += &format!("return {}", ty(r));
    }
    format!("{}[{sig}]", decl.name.to_string().to_ascii_lowercase())
}

/// The path elements of the subprograms and local package (instance) an item declared in
/// `scope` sits in, outermost first, each with its colon, when it sits in a package; the
/// second value is `:lib:pkg:` when that package is a library unit (then the elements start
/// below it). Empty and `None` for an item outside any package.
fn package_prefix(db: &Db, scope: yavs_sem::ScopeId) -> (String, Option<String>) {
    use yavs_sem::scope::ScopeKind as S;
    let mut parts = Vec::new();
    let mut sc = Some(scope);
    while let Some(s) = sc {
        let sd = &db.scopes[s];
        match (sd.kind, sd.owner) {
            (S::Subprogram, Some(o)) => parts.push(format!("{}:", subprogram_element(db, o))),
            (S::Loop | S::SeqBlock | S::PackageBody, _) => {}
            (S::Package, _) => {
                if let Some(u) = db.units.iter().map(|(_, u)| u).find(|u| u.scope == s) {
                    let lib = db.libs[u.lib].name.key.to_string();
                    let lib = format!(":{lib}:{}:", u.name.key);
                    return (parts.iter().rev().map(String::as_str).collect(), Some(lib));
                }
                parts.push(format!(
                    "{}:",
                    sd.owner.map_or(String::new(), |o| db.decls[o]
                        .name
                        .to_string()
                        .to_ascii_lowercase())
                ));
                return (parts.iter().rev().map(String::as_str).collect(), None);
            }
            (S::Instance, Some(o))
                if matches!(db.decls[o].kind, yavs_sem::decl::DeclKind::PackageInst(_)) =>
            {
                parts.push(format!(
                    "{}:",
                    db.decls[o].name.to_string().to_ascii_lowercase()
                ));
                return (parts.iter().rev().map(String::as_str).collect(), None);
            }
            _ => break,
        }
        sc = sd.parent;
    }
    (String::new(), None)
}

/// The path elements between a region and an object declared in `scope` inside a process or
/// subprogram, outermost first, each with its colon: `main:pr[integer]:`. An unlabeled
/// process contributes an empty element (`::`), as the reference simulators print it.
fn local_prefix(db: &Db, scope: yavs_sem::ScopeId) -> String {
    use yavs_sem::scope::ScopeKind as S;
    let mut parts = Vec::new();
    let mut sc = Some(scope);
    while let Some(s) = sc {
        let sd = &db.scopes[s];
        match (sd.kind, sd.owner) {
            (S::Process, o) => parts.push(match o {
                Some(l) => format!("{}:", db.decls[l].name.to_string().to_ascii_lowercase()),
                None => ":".into(),
            }),
            (S::Subprogram, Some(o)) => parts.push(format!("{}:", subprogram_element(db, o))),
            (S::Loop | S::SeqBlock, _) => {}
            _ => break,
        }
        sc = sd.parent;
    }
    parts.iter().rev().map(String::as_str).collect()
}

/// The interpreter's `resolution_limit` for the kernel's resolution (`yavs run --resolution`).
fn time_resolution(u: yavs_kernel::Unit) -> yavs_std::arith::TimeResolution {
    use yavs_kernel::Unit;
    use yavs_std::arith::TimeResolution as R;
    match u {
        Unit::Fs => R::Fs,
        Unit::Ps => R::Ps,
        Unit::Ns => R::Ns,
        Unit::Us => R::Us,
        Unit::Ms => R::Ms,
        // ponytail: the CLI offers fs..sec only; min and hr do not occur
        Unit::Sec | Unit::Min | Unit::Hr => R::Sec,
    }
}

/// The lowered architecture of the instance `region` belongs to.
fn unit_ir<'a>(e: &Elaborated, ir: &'a Ir, region: ScopeId) -> Result<&'a UnitIr, SimError> {
    let ScopeKind::Instance { arch, .. } = e.hier.scope(e.instance_of(region)).kind else {
        return Err(sim_err("region outside an instance".into()));
    };
    ir.units
        .get(&arch)
        .ok_or_else(|| sim_err("architecture not lowered".into()))
}

/// Value `v` of constant `iv` as call cells: a scalar's cell, a composite's elements and
/// descriptor.
fn encode_value(
    prog: &yavs_ir::Program,
    iv: &yavs_lower::InstValue,
    v: &crate::Val,
) -> Result<CallVal, SimError> {
    crate::value::encode(prog, iv.ty, v)
        .ok_or_else(|| sim_err(format!("`{}`: value does not fit its type", iv.name)))
}

/// Write `cv` into the cell of `iv` in `data`: a scalar's bytes, or a composite appended
/// with its offsets at `iv.off` (`UnitIr::write_composite`).
fn write_cell(
    prog: &yavs_ir::Program,
    iv: &yavs_lower::InstValue,
    cv: CallVal,
    data: &mut Vec<u8>,
) -> Result<(), SimError> {
    match cv {
        CallVal::Cell(c) => {
            let n = prog
                .types
                .get(iv.ty)
                .and_then(|t| t.scalar_size())
                .ok_or_else(|| sim_err(format!("`{}`: not a scalar", iv.name)))?
                as usize;
            let at = iv.off as usize;
            data.get_mut(at..at + n)
                .ok_or_else(|| sim_err(format!("no cell for `{}`", iv.name)))?
                .copy_from_slice(&c.to_le_bytes()[..n]);
        }
        CallVal::Composite { data: e, desc } => UnitIr::write_composite(iv.off, &e, &desc, data),
    }
    Ok(())
}

/// Left bound and direction of the index subtype of a resolution function's parameter type
/// `param` (an unconstrained one-dimensional array).
fn res_index_of(db: &Db, param: yavs_sem::SubtypeId) -> (i64, bool) {
    use yavs_sem::stat::ConstValue;
    let yavs_sem::types::TypeKind::Array { indexes, .. } = &db.types[db.base(param)].kind else {
        return (0, false);
    };
    match indexes.first().and_then(|&ix| db.static_bounds(ix)) {
        Some((ConstValue::Int(l) | ConstValue::Phys(l), dir, _)) => {
            (l, dir == yavs_sem::types::Direction::Downto)
        }
        Some((ConstValue::Enum(l), dir, _)) => {
            (l.into(), dir == yavs_sem::types::Direction::Downto)
        }
        // ponytail: a non-static index subtype (a generic-dependent range) keeps 0 upwards;
        // evaluate it per instance if a design needs its bounds.
        _ => (0, false),
    }
}

/// Whether `scope` declares a constant with an initial value that has no value at analysis time.
fn runtime_constant(db: &Db, scope: yavs_sem::ScopeId) -> bool {
    db.scopes[scope].order.iter().any(|&d| {
        matches!(&db.decls[d].kind, yavs_sem::decl::DeclKind::Object(o)
            if o.class == yavs_sem::decl::ObjClass::Constant
                && o.role == yavs_sem::decl::ObjRole::Plain
                && o.default.is_some_and(|e| db.value(e).is_none()))
    })
}
