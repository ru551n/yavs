//! Process-variable snapshots for waveform recording (`yavs run --wave-variables`).
//!
//! A process publishes the bytes of the variables it was asked to watch each time it
//! suspends; nothing else changes while it is suspended, so a reader between time steps sees
//! the value at the end of the step, whatever the thread count. Unwatched variables cost
//! nothing.

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use yavs_ir::{FuncId, Program, Ty, TyId};

/// A scalar local of a process function: its frame slot and IR type.
#[derive(Clone, Debug)]
pub struct ProbeLocal {
    pub name: String,
    pub slot: u32,
    pub size: u32,
    pub ty: TyId,
}

/// The scalar locals of one process and the latest published bytes of the watched ones.
pub struct ProcVars {
    pub locals: Vec<ProbeLocal>,
    snap: Mutex<Vec<Option<Vec<u8>>>>,
    watch: Mutex<Vec<bool>>,
}

impl ProcVars {
    /// The scalar locals (integer, physical, enumeration, real) of process function `func`.
    pub fn of(prog: &Program, func: FuncId) -> Arc<ProcVars> {
        let locals: Vec<ProbeLocal> = prog
            .func(func)
            .map(|f| f.locals.as_slice())
            .unwrap_or_default()
            .iter()
            .filter(|l| {
                matches!(
                    prog.types.get(l.ty),
                    Some(Ty::Int { .. } | Ty::Physical { .. } | Ty::Enum { .. } | Ty::Real)
                )
            })
            .map(|l| ProbeLocal {
                name: l.name.clone(),
                slot: l.slot,
                size: l.size,
                ty: l.ty,
            })
            .collect();
        let n = locals.len();
        Arc::new(ProcVars {
            locals,
            snap: Mutex::new(vec![None; n]),
            watch: Mutex::new(vec![false; n]),
        })
    }

    /// Publish local `i` from now on.
    pub fn watch(&self, i: usize) {
        if let Some(w) = self
            .watch
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get_mut(i)
        {
            *w = true;
        }
    }

    /// The bytes local `i` had when the process last suspended; `None` before that.
    pub fn value(&self, i: usize) -> Option<Vec<u8>> {
        self.snap
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(i)?
            .clone()
    }

    pub(crate) fn publish(&self, slots: &[u8]) {
        let watch = self.watch.lock().unwrap_or_else(PoisonError::into_inner);
        let mut snap = self.snap.lock().unwrap_or_else(PoisonError::into_inner);
        for (i, l) in self.locals.iter().enumerate().filter(|(i, _)| watch[*i]) {
            if let Some(b) = slots.get(l.slot as usize..(l.slot + l.size) as usize) {
                match &mut snap[i] {
                    Some(v) => {
                        v.clear();
                        v.extend_from_slice(b);
                    }
                    s => *s = Some(b.to_vec()),
                }
            }
        }
    }
}

/// The processes whose variables can be watched, by the index of their scope in the
/// elaborated hierarchy.
#[derive(Default)]
pub struct VarProbes(Mutex<HashMap<u32, Arc<ProcVars>>>);

impl VarProbes {
    pub fn insert(&self, scope: u32, v: Arc<ProcVars>) {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(scope, v);
    }

    pub fn get(&self, scope: u32) -> Option<Arc<ProcVars>> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .get(&scope)
            .cloned()
    }
}
