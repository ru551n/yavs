//! Processes: lowering the architectures that own processes (yavs-lower) and building one
//! interpreter process per process instance (yavs-interp), with per-instance data.

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use yavs_diag::Diagnostic;
use yavs_interp::{Bindings, InstData, InstSig, InterpProcess, ProcDriver, Runtime, SharedState};
use yavs_kernel::{Config, ErrorKind, SimControl, SimError};
use yavs_lower::{LowerOptions, Lowerer, UnitIr};
use yavs_sem::{Db, UnitId};

use crate::Elaborated;
use crate::hier::{ScopeId, ScopeKind};

/// The IR of the design: one program, one `UnitIr` per architecture that owns processes.
pub struct Ir {
    pub program: Arc<yavs_ir::Program>,
    pub units: HashMap<UnitId, UnitIr>,
    /// The function of each user resolution function of a signal.
    pub resolvers: HashMap<yavs_sem::DeclId, yavs_ir::FuncId>,
    /// `'path_name`/`'instance_name` strings per (region, named entity, instance?).
    pub names: HashMap<(ScopeId, yavs_sem::DeclId, bool), String>,
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
        if archs.is_empty() {
            return Ok(());
        }
        let mut l = Lowerer::new(db, LowerOptions::default());
        let mut idx = Vec::new();
        for &a in &archs {
            idx.push(l.architecture(a)?);
        }
        let mut resolvers = HashMap::new();
        for leaf in &self.nets.leaves {
            if let Some(crate::hier::LeafRes::Func(d)) = leaf.resolution
                && !resolvers.contains_key(&d)
            {
                resolvers.insert(d, l.subprogram(d)?);
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
        self.ir = Some(Ir {
            program: Arc::new(lowered.program),
            units,
            resolvers,
            names,
        });
        Ok(())
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
                format!("{}{name}", text(declared.unwrap_or(region)))
            }
        }
    }

    /// The kernel model with interpreter processes (after [`Elaborated::lower`]). Can be
    /// called again for an in-process restart.
    pub fn build(&self, cfg: Config) -> Result<SimControl, SimError> {
        let Some(ir) = &self.ir else {
            if self.procs.is_empty() {
                return self.build_with(cfg, |_, _| Err(sim_err("no IR".into())));
            }
            return Err(sim_err(
                "the design has processes but was not lowered".into(),
            ));
        };
        let rt = Runtime::new(ir.program.clone(), Bindings::default())?;
        let mut insts: HashMap<ScopeId, Arc<InstData>> = HashMap::new();
        let shared = Arc::new(Mutex::new(SharedState::new(Vec::new())));
        let res = |d: yavs_sem::DeclId| -> Result<yavs_kernel::Resolver, SimError> {
            let f = *ir
                .resolvers
                .get(&d)
                .ok_or_else(|| sim_err("resolution function not lowered".into()))?;
            let rt = rt.clone();
            Ok(Arc::new(move |vals: &[&[u8]], out: &mut [u8]| {
                // ponytail: no instance data, so a resolution function declared in an
                // architecture cannot read its constants; per-instance resolvers if needed.
                yavs_interp::resolve(rt.clone(), f, Arc::new(InstData::default()), vals, out)
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
                        let d = Arc::new(self.inst_data(&rt, u, region, &shared)?);
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
                let mut drivers = Vec::with_capacity(pir.drivers.len());
                for (k, &sig) in pir.drivers.iter().enumerate() {
                    let [(d, _, range)] = p.lsps.get(k).map(|v| v.as_slice()).unwrap_or(&[]) else {
                        return Err(sim_err(format!(
                            "driver {k} of {} spans several kernel signals (records, M3)",
                            self.hier.path_name(p.scope, crate::Case::Original)
                        )));
                    };
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
    }

    /// Instance data of instance `i`: constants from their thunks, the signal table.
    fn inst_data(
        &self,
        rt: &Arc<Runtime>,
        u: &UnitIr,
        i: ScopeId,
        shared: &Arc<Mutex<SharedState>>,
    ) -> Result<InstData, SimError> {
        let mut inst = InstData {
            data: vec![0; u.inst_size as usize],
            ..Default::default()
        };
        // Generics and constants elaboration computed; the rest come from their thunks.
        let mut known = std::collections::HashSet::new();
        for v in &u.values {
            if let Some(val) = self.value(i, v.decl) {
                UnitIr::write_value(&rt.prog, v, val, &mut inst.data)
                    .map_err(|m| sim_err(format!("instance data: {m}")))?;
                known.insert(v.decl);
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
            // With the shared state: a shared protected variable's thunk creates its object.
            let v = yavs_interp::call_shared(
                rt.clone(),
                c.init,
                Arc::new(inst.clone()),
                &[],
                Some(shared),
            )?
            .ok_or_else(|| sim_err(format!("constant `{}` has no value", c.name)))?;
            let n = c.w.bytes() as usize;
            inst.data[c.off as usize..c.off as usize + n].copy_from_slice(&v.to_le_bytes()[..n]);
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
            let (a, b) = self.nets.obj(o).leaves;
            if b.0 != a.0 + 1 {
                return Err(sim_err(format!(
                    "signal `{}` has several kernel signals (records, M3)",
                    s.name
                )));
            }
            inst.sigs.push(InstSig {
                signal: yavs_kernel::SignalId(a.0),
                ty: self.nets.leaf(a).ty,
            });
        }
        Ok(inst)
    }
}
