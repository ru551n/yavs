//! Processes: lowering the architectures that own processes (yavs-lower) and building one
//! interpreter process per process instance (yavs-interp), with per-instance data.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use yavs_diag::Diagnostic;
use yavs_interp::{
    Bindings, CallVal, InstData, InstSig, InterpProcess, ProcDriver, Runtime, SharedState,
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
    /// IR types of the parameter and the result of each function resolving a composite as
    /// a whole (#193).
    pub whole_types: HashMap<yavs_sem::DeclId, (yavs_ir::TyId, yavs_ir::TyId)>,
    /// Leaves resolved by a function declared in an entity or architecture: the instance
    /// whose data the function reads (its constants and generics, #164).
    pub res_regions: std::collections::BTreeMap<crate::LeafId, ScopeId>,
    /// `'path_name`/`'instance_name` strings per (region, named entity, instance?).
    pub names: HashMap<(ScopeId, yavs_sem::DeclId, bool), String>,
    /// Packages with run-time objects, in initialisation order (#183).
    pub packages: Vec<yavs_lower::PackageIr>,
    /// Source files of the program's statements, for `std.env` call paths.
    pub sources: yavs_interp::Sources,
    /// Bytes of the shared variables area (file objects of packages).
    pub shared_size: u32,
    /// The C function of each `program.foreign` entry, bound by the caller (`yavs run
    /// --load`) before [`Elaborated::build`].
    pub foreign: Vec<(yavs_ir::ForeignId, yavs_ffi::ForeignFn)>,
    /// Where the unbound ones were looked for (reported when one is called).
    pub foreign_searched: String,
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

    /// The instance scope a scope belongs to.
    pub fn instance_of(&self, mut s: ScopeId) -> ScopeId {
        loop {
            let sc = self.hier.scope(s);
            match (&sc.kind, sc.parent) {
                (ScopeKind::Instance { .. }, _) | (_, None) => return s,
                (_, Some(p)) => s = p,
            }
        }
    }

    /// Lower the architectures that own processes, and the package bodies they may call.
    /// Fail early: lowering diagnostics are errors.
    pub fn lower(&mut self, db: &mut Db) -> Result<(), Vec<Diagnostic>> {
        let mut archs: Vec<UnitId> = Vec::new();
        for p in &self.procs {
            let i = self.instance_of(p.scope);
            if let ScopeKind::Instance { arch, .. } = self.hier.scope(i).kind
                && !archs.contains(&arch)
            {
                archs.push(arch);
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
        let packages = run_time_packages(db);
        // A declaration's constant calling a subprogram of its own package runs before the
        // body is elaborated (LRM 14.4.2): E2114, also for constants only processes read
        // (#197).
        for &u in &packages {
            let scope = db.units[u].scope;
            for &d in &db.scopes[scope].order {
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
        let mut l = Lowerer::new(db, LowerOptions::default());
        // Package objects first (#183), in analysis order, which puts a package after those
        // its declaration uses (LRM 14.4.2).
        // ponytail: a body using a package analyzed after its declaration is initialised too
        // early; order by the dependency graph if a design needs it.
        for u in packages {
            l.package(u)?;
        }
        let mut idx = Vec::new();
        for &a in &archs {
            idx.push(l.architecture(a)?);
        }
        let mut resolvers = HashMap::new();
        let mut whole_types = HashMap::new();
        for leaf in &self.nets.leaves {
            if let Some(LeafRes::Func(d) | LeafRes::Whole(d)) = leaf.resolution
                && !resolvers.contains_key(&d)
            {
                resolvers.insert(d, l.subprogram(d)?);
            }
            if let Some(LeafRes::Whole(d)) = leaf.resolution
                && !whole_types.contains_key(&d)
                && let Some(sp) = db.decls[d].kind.subprogram()
                && let ([p], Some(r)) = (&sp.params[..], sp.ret)
            {
                let tys = (l.ty(db.base(p.subtype))?, l.ty(db.base(r))?);
                whole_types.insert(d, tys);
            }
        }
        let lowered = l.finish()?;
        let mut units = HashMap::new();
        for (a, i) in archs.into_iter().zip(idx) {
            units.insert(a, lowered.units[i].clone());
        }
        let mut names = HashMap::new();
        for p in &self.procs {
            let i = self.instance_of(p.scope);
            let ScopeKind::Instance { arch, .. } = self.hier.scope(i).kind else {
                continue;
            };
            let region = self.hier.scope(p.scope).parent.unwrap_or(i);
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
            program: Arc::new(lowered.program),
            units,
            resolvers,
            whole_types,
            res_regions,
            names,
            packages: lowered.packages,
            shared_size: lowered.shared_size,
            foreign: Vec::new(),
            foreign_searched: String::new(),
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
                    None => format!(
                        "{}{}",
                        text(region),
                        db.decls[d].name.to_string().to_ascii_lowercase()
                    ),
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
                let local = if declared.is_none() {
                    local_prefix(db, db.decls[d].scope)
                } else {
                    String::new()
                };
                format!("{}{local}{own}", text(declared.unwrap_or(region)))
            }
        }
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
        let shared = Arc::new(Mutex::new(SharedState::new(vec![
            0;
            ir.shared_size as usize
        ])));
        // Cells of instance-level constants and shared variables, computed once per instance.
        let mut once: HashMap<(ScopeId, yavs_sem::DeclId), CallVal> = HashMap::new();
        // Package data, created once in order: each initialisation sees the packages before it.
        let mut packages: Vec<Arc<InstData>> = Vec::new();
        for p in &ir.packages {
            let b = Bindings {
                packages: packages.clone(),
                sources: ir.sources.clone(),
                foreign: ir.foreign.clone(),
                searched: ir.foreign_searched.clone(),
                ..Default::default()
            };
            let rt = Runtime::new(ir.program.clone(), b)?;
            let d = yavs_interp::init_package(rt, p.init, p.index, p.size as usize, Some(&shared))?;
            packages.push(Arc::new(d));
        }
        let rt = Runtime::new(
            ir.program.clone(),
            Bindings {
                packages,
                sources: ir.sources.clone(),
                foreign: ir.foreign.clone(),
                searched: ir.foreign_searched.clone(),
                ..Default::default()
            },
        )?;
        let mut insts: HashMap<ScopeId, Arc<InstData>> = HashMap::new();
        // Instance data of the regions whose resolution functions read it (#164).
        let mut res_inst: HashMap<crate::LeafId, Arc<InstData>> = HashMap::new();
        for (&leaf, &region) in &ir.res_regions {
            let d = match insts.get(&region) {
                Some(d) => d.clone(),
                None => {
                    let ScopeKind::Instance { arch, .. } = self.hier.scope(region).kind else {
                        return Err(sim_err("resolution region is not an instance".into()));
                    };
                    let u = ir
                        .units
                        .get(&arch)
                        .ok_or_else(|| sim_err("architecture not lowered".into()))?;
                    let d = Arc::new(self.inst_data(&rt, u, region, &shared, &mut once)?);
                    insts.insert(region, d.clone());
                    d
                }
            };
            res_inst.insert(leaf, d);
        }
        let res =
            |d: yavs_sem::DeclId, leaf: crate::LeafId| -> Result<yavs_kernel::Resolver, SimError> {
                let f = *ir
                    .resolvers
                    .get(&d)
                    .ok_or_else(|| sim_err("resolution function not lowered".into()))?;
                let rt = rt.clone();
                let inst = res_inst.get(&leaf).cloned().unwrap_or_default();
                if let Some(LeafRes::Whole(_)) = self.nets.leaf(leaf).resolution {
                    let tys = *ir.whole_types.get(&d).ok_or_else(|| {
                        sim_err("types of a composite resolution function not lowered".into())
                    })?;
                    let shape = &self.nets.obj(self.nets.leaf(leaf).obj).shape;
                    return crate::whole::resolver(rt, f, inst, tys, shape).map_err(sim_err);
                }
                Ok(Arc::new(move |vals: &[&[u8]], out: &mut [u8]| {
                    yavs_interp::resolve(rt.clone(), f, inst.clone(), vals, out)
                        .map_err(|e| e.to_string())
                }))
            };
        self.build_res(
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
                let inst = match insts.get(&region) {
                    Some(d) => d.clone(),
                    None => {
                        let d = Arc::new(self.inst_data(&rt, u, region, &shared, &mut once)?);
                        insts.insert(region, d.clone());
                        d
                    }
                };
                let pir = u
                    .processes
                    .iter()
                    .find(|x| x.stmt == p.stmt)
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
                Ok(Box::new(proc))
            },
            res,
        )
        .map(|s| (s, Some(rt)))
    }

    /// Instance data of instance `i`: constants from their thunks, the signal table.
    fn inst_data(
        &self,
        rt: &Arc<Runtime>,
        u: &UnitIr,
        i: ScopeId,
        shared: &Arc<Mutex<SharedState>>,
        once: &mut HashMap<(ScopeId, yavs_sem::DeclId), CallVal>,
    ) -> Result<InstData, SimError> {
        let mut inst = InstData {
            data: vec![0; u.inst_size as usize],
            ..Default::default()
        };
        // Generics and constants elaboration computed; the rest come from their thunks.
        let mut known = std::collections::HashSet::new();
        for v in &u.values {
            if let Some(val) = self.value(i, v.decl) {
                match UnitIr::write_value(&rt.prog, v, val, &mut inst.data) {
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
        for c in u.consts.iter().filter(|c| !known.contains(&c.decl)) {
            // An instance-level constant or shared variable is one object for all regions of
            // the instance (#190): its thunk runs once per instance.
            let key = (self.instance_of(i), c.decl);
            let cached = if c.instance_level {
                once.get(&key).cloned()
            } else {
                None
            };
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
                    if c.instance_level {
                        once.insert(key, v.clone());
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
        for s in &u.signals {
            let Some(o) = self.signal(i, s.decl) else {
                // A signal of a nested region (block, generate): processes of this region never
                // name it, so its slot holds no kernel signal (any use is a BadRef error).
                inst.sigs.push(InstSig {
                    signal: yavs_kernel::SignalId(u32::MAX),
                    ty: yavs_kernel::SignalType::bytes(1, 0),
                });
                continue;
            };
            if let Some(off) = s.dope {
                // The array's descriptor in this instance (ports: the actual's bounds).
                let mut desc = Vec::new();
                let mut shape = &self.nets.obj(o).shape;
                while let crate::hier::Shape::Array { dims, elem, .. } = shape {
                    for b in dims {
                        let dir = i64::from(b.dir == yavs_sem::hir::Direction::Downto);
                        for x in [b.left, b.right, dir] {
                            desc.extend_from_slice(&x.to_le_bytes());
                        }
                    }
                    shape = elem;
                }
                let n = yavs_ir::Ty::desc_size(&rt.prog.types, s.ty)
                    .ok_or_else(|| sim_err(format!("no descriptor size for `{}`", s.name)))?
                    as usize;
                let at = off as usize;
                if desc.len() < n || inst.data.len() < at + n {
                    return Err(sim_err(format!("descriptor of signal `{}`", s.name)));
                }
                inst.data[at..at + n].copy_from_slice(&desc[..n]);
            }
            // The leaf the entry names (a record signal has one entry per field leaf).
            let (a, b) = self.nets.obj(o).leaves;
            let Some(leaf) = (a.0..b.0)
                .map(crate::LeafId)
                .find(|&l| self.nets.leaf(l).path == s.path)
            else {
                return Err(sim_err(format!(
                    "signal `{}` has no kernel signal for field path {:?}",
                    s.name, s.path
                )));
            };
            inst.sigs.push(InstSig {
                signal: yavs_kernel::SignalId(leaf.0),
                ty: self.nets.leaf(leaf).ty,
            });
        }
        Ok(inst)
    }
}

/// Packages outside std with objects that exist at run time: signals, shared variables,
/// files, or constants sem did not fold (in the declaration or the body).
fn run_time_packages(db: &Db) -> Vec<UnitId> {
    use yavs_sem::decl::{DeclKind, ObjClass};
    use yavs_sem::hir::UnitHir;
    let run_time = |scope: yavs_sem::ScopeId| {
        db.scopes[scope]
            .order
            .iter()
            .any(|&d| match &db.decls[d].kind {
                DeclKind::Object(o) => match o.class {
                    ObjClass::Constant => o.default.is_some_and(|e| db.exprs[e].value.is_none()),
                    _ => true,
                },
                _ => false,
            })
    };
    db.units
        .iter()
        .filter(|(_, u)| matches!(u.hir, UnitHir::Package { .. }))
        .filter(|(_, u)| db.libs[u.lib].name.key.as_str() != "std")
        .filter(|&(id, u)| {
            run_time(u.scope)
                || db.units.iter().any(|(_, b)| {
                    matches!(b.hir, UnitHir::PackageBody { package } if package == id)
                        && run_time(b.scope)
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
