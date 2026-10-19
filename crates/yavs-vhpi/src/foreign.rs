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

use yavs_interp::{ForeignModel, ModelArg, ModelCall, ScalarKind};
use yavs_ir::{ForeignMode, LogicKind};

use crate::abi::with_state;
use crate::model::{Fault, Object, State};
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

    fn call(&self, id: u64) -> Option<&ModelCall> {
        self.foreign.calls.get(&id)
    }

    fn arg(&self, obj: &Object) -> Option<&ModelArg> {
        match obj {
            Object::Param { call, idx } => self.call(*call)?.args.get(*idx as usize),
            Object::Call(c) => self.call(*c)?.ret.as_ref(),
            _ => None,
        }
    }

    /// The kind of a foreign object.
    pub(crate) fn foreign_kind(&self, obj: &Object) -> vhpiClassKindT {
        match obj {
            Object::Foreign(_) => vhpiForeignfK,
            Object::Call(c) if self.call(*c).is_some_and(|c| c.ret.is_none()) => vhpiProcDeclK,
            Object::Call(_) => vhpiFuncDeclK,
            _ => match self.arg(obj).map(|a| a.mode) {
                Some(ForeignMode::In) => vhpiConstParamDeclK,
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
                Some(vhpiIntT::from(self.arg(obj)?.array))
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
                })
                .collect(),
        )
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
        Ok(read_of(a))
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
        let (call, idx) = match obj {
            Object::Param { call, idx } => (*call, Some(*idx as usize)),
            Object::Call(c) => (*c, None),
            _ => return Err(misuse("not a foreign call object".into())),
        };
        let c = self
            .foreign
            .calls
            .get_mut(&call)
            .ok_or_else(|| misuse("the foreign call has returned".into()))?;
        let a = match idx {
            Some(i) => &mut c.args[i],
            None => c
                .ret
                .as_mut()
                .ok_or_else(|| misuse("a procedure call has no result".into()))?,
        };
        if a.mode == ForeignMode::In {
            return Err(misuse(format!("parameter {} has mode in", a.name)));
        }
        if mode == vhpiSizeConstraint {
            // The element count of an array result, before its value (`numElems`).
            let n = size
                .and_then(|n| usize::try_from(n).ok())
                .ok_or_else(|| misuse("vhpiSizeConstraint needs a value with numElems".into()))?;
            if idx.is_some() || !a.array {
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
        let ty = scalar_of(&a.kind);
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
        a.cells = match r.cells {
            Cells::Int(v) => v.into_iter().map(|i| i as u64).collect(),
            Cells::Real(v) => v.into_iter().map(f64::to_bits).collect(),
        };
        a.set = true;
        Ok(())
    }
}

fn scalar_of(k: &ScalarKind) -> Scalar {
    match k {
        ScalarKind::Int => Scalar::Int,
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
    }
}

fn read_of(a: &ModelArg) -> Read {
    let ty = scalar_of(&a.kind);
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
