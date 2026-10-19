//! Foreign models registered through VHPI (#274): `vhpi_register_foreignf`, the registrations
//! behind `vhpi_get_foreignf_info`, and the calls of `'FOREIGN "VHPI library model"`
//! subprograms. Safe Rust only except the one call of the plugin's `execf`/`elabf`.
//!
//! A call is a [`ModelCall`] the interpreter hands over; while the plugin's `execf` runs it
//! lives in `State::foreign` as the object behind the callback's `obj` handle
//! ([`Object::Call`]) and the handles of `vhpi_iterator(vhpiParamDecls, call)`
//! ([`Object::Param`]). `vhpi_get_value` reads them, `vhpi_put_value` writes `out` and
//! `inout` parameters and the function result (the call handle itself).

// VHPI names are matched as written in the C standard.
#![allow(non_upper_case_globals)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex, PoisonError};

use yavs_interp::{ForeignModel, ModelArg, ModelCall, ModelTxn, ScalarKind};
use yavs_ir::{ForeignMode, LogicKind};

use yavs_kernel::{DriverId, Hook, SigRef, Value};

use crate::abi::{with_hook, with_state};
use crate::design::{NodeId, Origin};
use crate::model::{Fault, Object, Part, State};
use crate::sys::*;
use crate::value::{Cells, Lit, Read, Scalar};

/// One `vhpi_register_foreignf` registration.
pub(crate) struct Reg {
    pub(crate) kind: vhpiForeignKindT,
    pub(crate) library: String,
    pub(crate) model: String,
    pub(crate) elabf: vhpiCbRoutineT,
    pub(crate) execf: vhpiCbRoutineT,
}

#[derive(Default)]
pub(crate) struct Foreign {
    regs: Vec<Reg>,
    calls: HashMap<u64, ModelCall>,
    next_call: u64,
    /// The foreign architecture instance whose `elabf` or `execf` runs: processes and
    /// drivers are created then (`vhpi_create`).
    elaborating: Option<NodeId>,
    /// Instances of foreign architectures and their registrations.
    archs: Vec<(NodeId, usize)>,
    /// Processes of foreign architectures: their instance.
    procs: Vec<NodeId>,
    drivers: Vec<Driver>,
    /// Transactions scheduled before the simulation (driver, transaction): made at the first
    /// observer call.
    pending: Vec<(usize, ModelTxn)>,
    /// `vhpiLibF` registrations whose `elabf` ran (#357), in that order: their `execf` runs
    /// at the start of the simulation.
    libs: Vec<usize>,
}

/// A driver a foreign architecture's model created.
pub(crate) struct Driver {
    proc: usize,
    at: SigRef,
    ty: Scalar,
    len: Option<usize>,
    /// Set when the simulation is built.
    id: Option<DriverId>,
}

/// Plugin code runs one foreign call at a time, whatever the kernel's thread count: VHPI has
/// no notion of concurrent callbacks.
static EXEC: Mutex<()> = Mutex::new(());

impl State {
    /// `vhpi_register_foreignf`: the index of the new registration.
    pub(crate) fn register_foreign(&mut self, d: &vhpiForeignDataT) -> Result<usize, String> {
        let text = |p: *mut std::ffi::c_char, what: &str| {
            if p.is_null() {
                return Err(format!("NULL {what}"));
            }
            // SAFETY: non-NULL and NUL-terminated per the VHPI contract of the registration.
            Ok(unsafe { std::ffi::CStr::from_ptr(p) }
                .to_string_lossy()
                .into_owned())
        };
        let library = text(d.libraryName, "libraryName")?;
        let model = text(d.modelName, "modelName")?;
        if !matches!(
            d.kind,
            vhpiArchF | vhpiFuncF | vhpiProcF | vhpiLibF | vhpiAppF
        ) {
            return Err(format!("bad foreign kind {}", d.kind));
        }
        if matches!(d.kind, vhpiFuncF | vhpiProcF) && d.execf.is_none() {
            return Err("a function or procedure needs an execf".into());
        }
        if self
            .foreign
            .regs
            .iter()
            .any(|r| r.kind == d.kind && r.library == library && r.model == model)
        {
            return Err(format!("{library} {model} is already registered"));
        }
        self.foreign.regs.push(Reg {
            kind: d.kind,
            library,
            model,
            elabf: d.elabf,
            execf: d.execf,
        });
        Ok(self.foreign.regs.len() - 1)
    }

    /// `vhpi_get_foreignf_info`: fills `d` from registration `i`.
    pub(crate) fn foreign_info(&mut self, i: usize, d: &mut vhpiForeignDataT) -> Option<()> {
        let r = self.foreign.regs.get(i)?;
        let (kind, elabf, execf) = (r.kind, r.elabf, r.execf);
        let (lib, model) = (r.library.clone(), r.model.clone());
        *d = vhpiForeignDataT {
            kind,
            libraryName: self.intern(lib).cast_mut().cast(),
            modelName: self.intern(model).cast_mut().cast(),
            elabf,
            execf,
        };
        Some(())
    }

    /// The registration of foreign architecture instance `n`.
    pub(crate) fn foreign_arch(&self, n: NodeId) -> Option<usize> {
        self.foreign
            .archs
            .iter()
            .find(|&&(x, _)| x == n)
            .map(|&(_, i)| i)
    }

    fn call(&self, id: u64) -> Option<&ModelCall> {
        self.foreign.calls.get(&id)
    }

    /// The parameter, record field or function result `obj` is (a driver: its parameter).
    fn arg(&self, obj: &Object) -> Option<&ModelArg> {
        match obj {
            Object::Param { call, idx, part } => {
                let a = self.call(*call)?.args.get(*idx as usize)?;
                match part {
                    Part::Field(f) => a.fields.get(*f as usize),
                    Part::Whole | Part::Driver => Some(a),
                }
            }
            Object::Call(c) => self.call(*c)?.ret.as_ref(),
            _ => None,
        }
    }

    fn arg_mut(&mut self, obj: &Object) -> Option<&mut ModelArg> {
        match obj {
            Object::Param { call, idx, part } => {
                let a = self
                    .foreign
                    .calls
                    .get_mut(call)?
                    .args
                    .get_mut(*idx as usize)?;
                match part {
                    Part::Field(f) => a.fields.get_mut(*f as usize),
                    Part::Whole | Part::Driver => Some(a),
                }
            }
            Object::Call(c) => self.foreign.calls.get_mut(c)?.ret.as_mut(),
            _ => None,
        }
    }

    /// The kind of a foreign object.
    pub(crate) fn foreign_kind(&self, obj: &Object) -> vhpiClassKindT {
        match obj {
            Object::Foreign(_) => vhpiForeignfK,
            Object::Call(c) if self.call(*c).is_some_and(|c| c.ret.is_none()) => vhpiProcDeclK,
            Object::Call(_) => vhpiFuncDeclK,
            Object::Param {
                part: Part::Field(_),
                ..
            } => vhpiSelectedNameK,
            Object::Param {
                part: Part::Driver, ..
            } => vhpiDriverK,
            _ => match self.arg(obj) {
                Some(a) if a.signal.is_some() => vhpiSigParamDeclK,
                // An access parameter is a variable.
                Some(a) if a.mode == ForeignMode::In && a.kind != ScalarKind::Access => {
                    vhpiConstParamDeclK
                }
                _ => vhpiVarParamDeclK,
            },
        }
    }

    pub(crate) fn foreign_str(&self, obj: &Object, prop: vhpiStrPropertyT) -> Option<String> {
        match (obj, prop) {
            (Object::Foreign(i), vhpiNameP | vhpiCaseNameP) => {
                Some(self.foreign.regs.get(*i)?.model.clone())
            }
            (Object::Foreign(i), vhpiLogicalNameP) => {
                Some(self.foreign.regs.get(*i)?.library.clone())
            }
            (Object::Call(c), vhpiNameP | vhpiCaseNameP) => {
                let n = &self.call(*c)?.subprogram;
                Some(if prop == vhpiNameP {
                    n.to_ascii_uppercase()
                } else {
                    n.clone()
                })
            }
            (Object::Param { .. }, vhpiNameP | vhpiFullNameP) => {
                Some(self.arg(obj)?.name.to_ascii_uppercase())
            }
            (Object::Param { .. }, vhpiCaseNameP | vhpiFullCaseNameP) => {
                Some(self.arg(obj)?.name.clone())
            }
            _ => None,
        }
    }

    pub(crate) fn foreign_int(&self, obj: &Object, prop: vhpiIntPropertyT) -> Option<vhpiIntT> {
        match (obj, prop) {
            (Object::Foreign(i), vhpiForeignKindP) => Some(self.foreign.regs.get(*i)?.kind),
            (Object::Param { .. }, vhpiModeP) => Some(match self.arg(obj)?.mode {
                ForeignMode::In => vhpiInMode,
                ForeignMode::Out => vhpiOutMode,
                ForeignMode::InOut => vhpiInoutMode,
            }),
            (Object::Param { .. } | Object::Call(_), vhpiIsCompositeP) => {
                let a = self.arg(obj)?;
                Some(vhpiIntT::from(a.array || a.kind == ScalarKind::Record))
            }
            // An unconstrained array result has no size until the model sets one.
            (Object::Param { .. } | Object::Call(_), vhpiSizeP) => match self.arg(obj) {
                None => Some(0),
                Some(a) => vhpiIntT::try_from(a.len?).ok(),
            },
            _ => None,
        }
    }

    /// `vhpi_iterator(vhpiParamDecls, call)`.
    pub(crate) fn foreign_params(&self, obj: &Object) -> Option<Vec<Object>> {
        let Object::Call(c) = obj else { return None };
        let n = self.call(*c)?.args.len();
        Some(
            (0..n)
                .map(|idx| Object::Param {
                    call: *c,
                    idx: idx as u32,
                    part: Part::Whole,
                })
                .collect(),
        )
    }

    /// `vhpi_iterator(rel, param)`: the fields of a record parameter (`vhpiSelectedNames`),
    /// the caller's driver of an `out` or `inout` signal parameter (`vhpiDrivers`).
    pub(crate) fn foreign_iter(&self, rel: vhpiOneToManyT, obj: &Object) -> Option<Vec<Object>> {
        let &Object::Param {
            call,
            idx,
            part: Part::Whole,
        } = obj
        else {
            return None;
        };
        let a = self.arg(obj)?;
        match rel {
            vhpiSelectedNames if a.kind == ScalarKind::Record => Some(
                (0..a.fields.len() as u32)
                    .map(|f| Object::Param {
                        call,
                        idx,
                        part: Part::Field(f),
                    })
                    .collect(),
            ),
            vhpiDrivers if a.signal.is_some() && a.mode != ForeignMode::In => {
                Some(vec![Object::Param {
                    call,
                    idx,
                    part: Part::Driver,
                }])
            }
            _ => None,
        }
    }

    /// The address an access parameter holds (`vhpiPtrVal`), `None` for anything else.
    pub(crate) fn foreign_access(&self, obj: &Object) -> Option<u64> {
        match obj {
            Object::Param {
                part: Part::Whole, ..
            } => self
                .arg(obj)
                .filter(|a| a.kind == ScalarKind::Access)
                .map(|a| a.cells[0]),
            _ => None,
        }
    }

    /// `vhpi_get_value` on a parameter or a function result.
    pub(crate) fn foreign_read(&self, obj: &Object) -> Result<Read, (Fault, String)> {
        let misuse = |m: &str| (Fault::Misuse, format!("vhpi_get_value: {m}"));
        let a = self
            .arg(obj)
            .ok_or_else(|| misuse("the foreign call has returned or has no such value"))?;
        if !a.set {
            return Err(misuse("the function result has not been set"));
        }
        if matches!(
            obj,
            Object::Param {
                part: Part::Driver,
                ..
            }
        ) {
            return Err(misuse("a driver has no value"));
        }
        let ty = scalar_of(a)?;
        if let Some(sig) = &a.signal {
            return Read::from_kernel(ty, a.array, &sig.value).ok_or_else(|| {
                misuse("this is a bug in yavs: a signal parameter's value does not decode")
            });
        }
        Ok(read_of(ty, a))
    }

    /// `vhpi_put_value` on an `out` or `inout` parameter, or on the function result.
    /// `size` is the value's `numElems` (`vhpiSizeConstraint`).
    pub(crate) fn foreign_put(
        &mut self,
        obj: &Object,
        mode: vhpiPutValueModeT,
        size: Option<i32>,
        decode: impl FnOnce(&Scalar, Option<usize>) -> Result<Read, crate::value::Error>,
    ) -> Result<(), (Fault, String)> {
        let misuse = |m: String| (Fault::Misuse, format!("vhpi_put_value: {m}"));
        if !matches!(
            mode,
            vhpiDeposit | vhpiDepositPropagate | vhpiSizeConstraint
        ) {
            return Err(misuse(format!(
                "mode {mode} does not apply to a foreign call (only vhpiDeposit, \
                 vhpiDepositPropagate and vhpiSizeConstraint)"
            )));
        }
        let result = matches!(obj, Object::Call(_));
        if matches!(
            obj,
            Object::Param {
                part: Part::Driver,
                ..
            }
        ) {
            return Err(misuse(
                "a driver takes values with vhpi_schedule_transaction".into(),
            ));
        }
        let a = self.arg_mut(obj).ok_or_else(|| {
            misuse("the foreign call has returned, or a procedure call has no result".into())
        })?;
        if a.mode == ForeignMode::In {
            return Err(misuse(format!("parameter {} has mode in", a.name)));
        }
        if mode == vhpiSizeConstraint {
            // The element count of an array result, before its value (`numElems`).
            let n = size
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| misuse("vhpiSizeConstraint needs a value with numElems".into()))?;
            if !result || !a.array {
                return Err(misuse(
                    "vhpiSizeConstraint applies only to an array function result".into(),
                ));
            }
            if let Some(l) = a.len
                && l != n
            {
                return Err(misuse(format!(
                    "the result has {l} elements; vhpiSizeConstraint gives {n}"
                )));
            }
            a.len = Some(n);
            return Ok(());
        }
        let ty = scalar_of(a).map_err(|(_, m)| misuse(m))?;
        // A parameter keeps its length, an array result has the size the model set.
        let n = match (a.array, a.len) {
            (false, _) => None,
            (true, Some(n)) => Some(n),
            (true, None) => {
                return Err(misuse(
                    "the size of an unconstrained array result is not set: put \
                     vhpiSizeConstraint first"
                        .into(),
                ));
            }
        };
        let r = decode(&ty, n).map_err(|e| match e {
            crate::value::Error::Unsupported(m) => (Fault::Unsupported, m),
            crate::value::Error::Mismatch(m) => misuse(m),
        })?;
        // An out signal parameter: a deposit on the actual, after the call.
        if let Some(sig) = &mut a.signal {
            sig.deposit = Some(r.to_kernel());
            return Ok(());
        }
        a.cells = match r.cells {
            Cells::Int(v) => v.into_iter().map(|i| i as u64).collect(),
            Cells::Real(v) => v.into_iter().map(f64::to_bits).collect(),
        };
        a.set = true;
        Ok(())
    }

    /// `vhpi_create(kind, a, b)` (#330): a process of the foreign architecture being
    /// elaborated (`a` its instance or design unit, `b` NULL), or a driver of a process for a
    /// signal or port that is not `in` (`a` and `b` the two, in either order).
    pub(crate) fn create(
        &mut self,
        kind: vhpiClassKindT,
        a: Option<Object>,
        b: Option<Object>,
    ) -> Result<Object, (Fault, String)> {
        let misuse = |m: &str| (Fault::Misuse, format!("vhpi_create: {m}"));
        let Some(inst) = self.foreign.elaborating else {
            return Err(misuse(
                "processes and drivers are created while a foreign architecture is \
                 elaborated (its elabf or execf)",
            ));
        };
        match (kind, a, b) {
            (vhpiProcessStmtK, Some(Object::Node(n)), None)
                if n == inst || self.design.node(inst).unit == Some(n) =>
            {
                self.foreign.procs.push(inst);
                Ok(Object::Process(self.foreign.procs.len() - 1))
            }
            (vhpiProcessStmtK, ..) => Err(misuse(
                "a process belongs to the foreign architecture being elaborated: its instance \
                 or design unit, and no second handle",
            )),
            (vhpiDriverK, Some(Object::Process(p)), Some(sig))
            | (vhpiDriverK, Some(sig), Some(Object::Process(p))) => {
                let at = self.at(&sig, "vhpi_create")?;
                if !matches!(at.node.origin, Origin::Signal { .. }) {
                    return Err(misuse("a driver needs a signal or port"));
                }
                if at.node.mode == Some(vhpiInMode) {
                    return Err(misuse("a port of mode in has no driver"));
                }
                let (ty, n, array) = Self::scalars(at.l, "vhpi_create")?;
                let d = Driver {
                    proc: p,
                    at: at.signal_ref(ty, n),
                    ty: ty.clone(),
                    len: array.then_some(n as usize),
                    id: None,
                };
                self.foreign.drivers.push(d);
                Ok(Object::Driver(self.foreign.drivers.len() - 1))
            }
            (vhpiDriverK, ..) => Err(misuse("a driver needs a created process and a signal")),
            _ => Err((
                Fault::Unsupported,
                format!("vhpi_create of class {kind}: only processes and drivers"),
            )),
        }
    }

    /// `vhpi_schedule_transaction` on the driver of an `out` signal parameter: a transaction
    /// the caller schedules on the actual after the call.
    pub(crate) fn foreign_schedule(
        &mut self,
        obj: &Object,
        delay: yavs_kernel::Time,
        mode: yavs_kernel::DelayMode,
        decode: impl FnOnce(&Scalar, Option<usize>) -> Result<Read, crate::value::Error>,
    ) -> Result<(), (Fault, String)> {
        let misuse = |m: &str| (Fault::Misuse, format!("vhpi_schedule_transaction: {m}"));
        if let Object::Driver(k) = *obj {
            let d = &self.foreign.drivers[k];
            let r = decode(&d.ty, d.len).map_err(|e| match e {
                crate::value::Error::Unsupported(m) => (Fault::Unsupported, m),
                crate::value::Error::Mismatch(m) => misuse(&m),
            })?;
            let txn = ModelTxn {
                value: r.to_kernel(),
                delay,
                mode,
            };
            // In a simulation callback at once, else (an execf) at the first one.
            let Some(id) = d.id else {
                self.foreign.pending.push((k, txn));
                return Ok(());
            };
            return match with_hook(|h| h.assign(id, None, &[(txn.delay, &txn.value)], txn.mode)) {
                Some(Ok(())) => Ok(()),
                Some(Err(e)) => Err(misuse(&e.kind.to_string())),
                None => {
                    self.foreign.pending.push((k, txn));
                    Ok(())
                }
            };
        }
        if !matches!(
            obj,
            Object::Param {
                part: Part::Driver,
                ..
            }
        ) {
            return Err(misuse(
                "not a driver (an out signal parameter's driver is in its vhpiDrivers)",
            ));
        }
        let a = self
            .arg_mut(obj)
            .ok_or_else(|| misuse("the foreign call has returned"))?;
        let ty = scalar_of(a)?;
        let r = decode(&ty, a.array.then_some(a.len.unwrap_or(0))).map_err(|e| match e {
            crate::value::Error::Unsupported(m) => (Fault::Unsupported, m),
            crate::value::Error::Mismatch(m) => misuse(&m),
        })?;
        let Some(sig) = &mut a.signal else {
            return Err(misuse("this is a bug in yavs: a driver of a non-signal"));
        };
        sig.txns.push(ModelTxn {
            value: r.to_kernel(),
            delay,
            mode,
        });
        Ok(())
    }
}

/// The scalar type of an argument's value or elements; a record or access value has none.
fn scalar_of(a: &ModelArg) -> Result<Scalar, (Fault, String)> {
    Ok(match &a.kind {
        ScalarKind::Record => {
            return Err((
                Fault::Unsupported,
                "a record parameter as a whole: use its fields (vhpiSelectedNames)".into(),
            ));
        }
        ScalarKind::Access => {
            return Err((
                Fault::Misuse,
                "an access parameter has only a vhpiPtrVal value".into(),
            ));
        }
        // A signal holds a narrow integer array element in 4 bytes (`Ty::elem_size`).
        ScalarKind::Int => Scalar::Int {
            narrow: match (&a.signal, a.len) {
                (Some(s), Some(n)) => {
                    matches!(&s.value, Value::Bytes(b) if n > 0 && b.len() == 4 * n)
                }
                _ => false,
            },
        },
        ScalarKind::Real => Scalar::Real,
        ScalarKind::Phys { time } => Scalar::Phys { time: *time },
        ScalarKind::Enum { lits, logic } => {
            // CHARACTER is the 256-literal type with the plain 7-bit names; its literal at
            // position `p` shows as byte `p`.
            let character = lits.len() == 256 && logic.is_none();
            Scalar::Enum {
                lits: lits
                    .iter()
                    .enumerate()
                    .map(|(p, name)| {
                        let b = name.as_bytes();
                        Lit {
                            name: name.clone(),
                            ch: if character {
                                u8::try_from(p).ok()
                            } else if b.len() == 3 && b[0] == b'\'' && b[2] == b'\'' {
                                Some(b[1])
                            } else {
                                None
                            },
                        }
                    })
                    .collect::<Arc<[Lit]>>(),
                logic: matches!(logic, Some(LogicKind::StdULogic | LogicKind::Bit)),
                character,
                bit01: matches!(logic, Some(LogicKind::Bit | LogicKind::Boolean)),
            }
        }
    })
}

fn read_of(ty: Scalar, a: &ModelArg) -> Read {
    let cells = match ty {
        Scalar::Real => Cells::Real(a.cells.iter().map(|&c| f64::from_bits(c)).collect()),
        _ => Cells::Int(a.cells.iter().map(|&c| c as i64).collect()),
    };
    Read {
        ty,
        array: a.array,
        cells,
    }
}

/// The plugin side of a bound model: calls its `execf`.
struct Model {
    execf: vhpiCbRoutineT,
}

impl ForeignModel for Model {
    fn call(&self, c: &mut ModelCall) -> Result<(), String> {
        let Some(execf) = self.execf else {
            return Err("the model has no execution callback".into());
        };
        let _one_at_a_time = EXEC.lock().unwrap_or_else(PoisonError::into_inner);
        let (id, h, mut time) = with_state(|s| {
            let id = s.foreign.next_call;
            s.foreign.next_call += 1;
            s.foreign.calls.insert(id, c.clone());
            (id, s.handles.alloc(Object::Call(id)), s.clock.vhpi_now())
        });
        let data = vhpiCbDataT {
            reason: 0,
            cb_rtn: None,
            obj: h,
            time: &mut time,
            value: std::ptr::null_mut(),
            user_data: std::ptr::null_mut(),
        };
        // SAFETY: the plugin registered `execf` for exactly this call; `data` outlives it.
        unsafe { execf(&data) };
        with_state(|s| {
            s.handles.release(h);
            match s.foreign.calls.remove(&id) {
                Some(done) => {
                    *c = done;
                    Ok(())
                }
                None => Err("the call vanished".into()),
            }
        })
    }
}

/// The model a plugin registered for `library model`, for a function (`function`) or a
/// procedure; runs its elaboration callback (once per bound subprogram) before returning.
/// `Ok(None)` when no plugin registered it, `Err` when the registration has the wrong kind.
pub(crate) fn model(
    library: &str,
    model: &str,
    function: bool,
) -> Result<Option<Arc<dyn ForeignModel>>, String> {
    let found = with_state(|s| {
        let i = s.foreign.regs.iter().position(|r| {
            matches!(r.kind, vhpiFuncF | vhpiProcF) && r.library == library && r.model == model
        })?;
        let r = &s.foreign.regs[i];
        Some((
            i,
            r.kind,
            r.elabf,
            r.execf,
            s.handles.alloc(Object::Foreign(i)),
        ))
    });
    let Some((_, kind, elabf, execf, h)) = found else {
        return Ok(None);
    };
    elab_libs(library);
    if (kind == vhpiFuncF) != function {
        return Err(format!(
            "`{library} {model}` is registered as a {} but declared as a {}",
            if kind == vhpiFuncF {
                "function"
            } else {
                "procedure"
            },
            if function { "function" } else { "procedure" },
        ));
    }
    if let Some(elabf) = elabf {
        let data = vhpiCbDataT {
            reason: 0,
            cb_rtn: None,
            obj: h,
            time: std::ptr::null_mut(),
            value: std::ptr::null_mut(),
            user_data: std::ptr::null_mut(),
        };
        // SAFETY: the plugin registered `elabf` for this call.
        unsafe { elabf(&data) };
    }
    with_state(|s| s.handles.release(h));
    Ok(Some(Arc::new(Model { execf })))
}

/// Runs the models of the foreign architectures of `e` (#330): for each instance in
/// elaboration order the `elabf` of its `vhpiArchF` registration, then each `execf`, with the
/// instance as `obj`. The processes and drivers they create are returned for the build. An
/// instance whose model no plugin registered is an error naming it.
pub(crate) fn elaborate(e: &yavs_elab::Elaborated) -> Result<Vec<yavs_elab::ExtProc>, String> {
    let mut insts = Vec::new();
    for fa in &e.foreign_archs {
        let path = e.hier.path_name(fa.scope, yavs_elab::Case::Lrm);
        let found = with_state(|s| {
            let n = s.design.scope_node(fa.scope)?;
            let i = s.foreign.regs.iter().position(|r| {
                r.kind == vhpiArchF && r.library == fa.library && r.model == fa.model
            })?;
            s.foreign.archs.push((n, i));
            let r = &s.foreign.regs[i];
            Some((n, r.elabf, r.execf))
        });
        let Some(f) = found else {
            return Err(format!(
                "foreign architecture `{} {}` of instance {} is not registered",
                fa.library, fa.model, path
            ));
        };
        insts.push((path, f));
    }
    for fa in &e.foreign_archs {
        elab_libs(&fa.library);
    }
    for pass in 0..2 {
        for (_, (n, elabf, execf)) in &insts {
            let Some(f) = (if pass == 0 { *elabf } else { *execf }) else {
                continue;
            };
            let h = with_state(|s| {
                s.foreign.elaborating = Some(*n);
                s.handles.alloc(Object::Node(*n))
            });
            let data = vhpiCbDataT {
                reason: 0,
                cb_rtn: None,
                obj: h,
                time: std::ptr::null_mut(),
                value: std::ptr::null_mut(),
                user_data: std::ptr::null_mut(),
            };
            // SAFETY: the plugin registered this function for the architecture.
            unsafe { f(&data) };
            with_state(|s| {
                s.handles.release(h);
                s.foreign.elaborating = None;
            });
        }
    }
    Ok(with_state(|s| {
        (0..s.foreign.procs.len())
            .map(|p| {
                let inst = s.foreign.procs[p];
                let path = insts
                    .iter()
                    .find(|(_, (n, ..))| *n == inst)
                    .map_or("", |(path, _)| path.as_str());
                let mine: Vec<usize> = (0..s.foreign.drivers.len())
                    .filter(|&k| s.foreign.drivers[k].proc == p)
                    .collect();
                yavs_elab::ExtProc {
                    name: format!("{path}vhpi({p})"),
                    drivers: mine.iter().map(|&k| s.foreign.drivers[k].at).collect(),
                    built: Arc::new(move |ids: &[DriverId]| {
                        with_state(|s| {
                            for (&k, &id) in mine.iter().zip(ids) {
                                s.foreign.drivers[k].id = Some(id);
                            }
                        })
                    }),
                }
            })
            .collect()
    }))
}

/// Makes the transactions models scheduled before the simulation (in an `execf`).
pub(crate) fn pending(s: &mut State, hook: &mut Hook<'_>) {
    for (k, t) in std::mem::take(&mut s.foreign.pending) {
        let r = match s.foreign.drivers[k].id {
            Some(id) => hook
                .assign(id, None, &[(t.delay, &t.value)], t.mode)
                .map_err(|e| e.kind.to_string()),
            None => Err("this is a bug in yavs: a driver that was not built".into()),
        };
        if let Err(m) = r {
            s.fail(Fault::Misuse, format!("vhpi_schedule_transaction: {m}"));
        }
    }
}

/// Calls `f` of registration `i` with the registration handle as `obj` (library and
/// application models, #357).
fn call_reg(i: usize, f: vhpiCbRoutineT) {
    let Some(f) = f else { return };
    let h = with_state(|s| s.handles.alloc(Object::Foreign(i)));
    let data = vhpiCbDataT {
        reason: 0,
        cb_rtn: None,
        obj: h,
        time: std::ptr::null_mut(),
        value: std::ptr::null_mut(),
        user_data: std::ptr::null_mut(),
    };
    // SAFETY: the plugin registered `f` for this registration.
    unsafe { f(&data) };
    with_state(|s| s.handles.release(h));
}

/// A foreign model of library `library` is being bound or elaborated: the `elabf` of its
/// `vhpiLibF` registrations runs, once each (#357).
fn elab_libs(library: &str) {
    let due: Vec<(usize, vhpiCbRoutineT)> = with_state(|s| {
        let due: Vec<usize> = (0..s.foreign.regs.len())
            .filter(|i| {
                let r = &s.foreign.regs[*i];
                r.kind == vhpiLibF && r.library == library && !s.foreign.libs.contains(i)
            })
            .collect();
        s.foreign.libs.extend(&due);
        due.iter().map(|&i| (i, s.foreign.regs[i].elabf)).collect()
    });
    for (i, f) in due {
        call_reg(i, f);
    }
}

/// At the start of the simulation (#357): the `execf` of the library models whose `elabf`
/// ran, then the `elabf` and `execf` of every application model, in registration order.
/// Called inside a `hooked` observer call, before the start-of-simulation callbacks.
pub(crate) fn start_models() {
    let (libs, apps): (Vec<_>, Vec<_>) = with_state(|s| {
        let regs = &s.foreign.regs;
        (
            s.foreign.libs.iter().map(|&i| (i, regs[i].execf)).collect(),
            (0..regs.len())
                .filter(|&i| regs[i].kind == vhpiAppF)
                .collect(),
        )
    });
    for (i, f) in libs {
        call_reg(i, f);
    }
    for i in apps {
        let (e, x) = with_state(|s| (s.foreign.regs[i].elabf, s.foreign.regs[i].execf));
        call_reg(i, e);
        call_reg(i, x);
    }
}
