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
}

fn sim_err(m: String) -> SimError {
    SimError::new(ErrorKind::Usage(m))
}

impl Elaborated {
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
        let lowered = l.finish()?;
        let mut units = HashMap::new();
        for (a, i) in archs.into_iter().zip(idx) {
            units.insert(a, lowered.units[i].clone());
        }
        self.ir = Some(Ir {
            program: Arc::new(lowered.program),
            units,
        });
        Ok(())
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
        self.build_with(cfg, |p, ds| {
            let i = self.instance_of(p.scope);
            let ScopeKind::Instance { arch, .. } = self.hier.scope(i).kind else {
                return Err(sim_err("process outside an instance".into()));
            };
            let u = ir
                .units
                .get(&arch)
                .ok_or_else(|| sim_err("architecture not lowered".into()))?;
            let inst = match insts.get(&i) {
                Some(d) => d.clone(),
                None => {
                    let d = Arc::new(self.inst_data(&rt, u, i)?);
                    insts.insert(i, d.clone());
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
                let pos =
                    p.drivers.iter().position(|x| x == d).ok_or_else(|| {
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
        })
    }

    /// Instance data of instance `i`: constants from their thunks, the signal table.
    fn inst_data(&self, rt: &Arc<Runtime>, u: &UnitIr, i: ScopeId) -> Result<InstData, SimError> {
        let mut inst = InstData {
            data: vec![0; u.inst_size as usize],
            ..Default::default()
        };
        for c in &u.consts {
            let v = yavs_interp::call(rt.clone(), c.init, Arc::new(inst.clone()), &[])?
                .ok_or_else(|| sim_err(format!("constant `{}` has no value", c.name)))?;
            let n = c.w.bytes() as usize;
            inst.data[c.off as usize..c.off as usize + n].copy_from_slice(&v.to_le_bytes()[..n]);
        }
        for s in &u.signals {
            let o = self
                .signal(i, s.decl)
                .ok_or_else(|| sim_err(format!("signal `{}` was not elaborated", s.name)))?;
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
