//! The VHPI object model: the design snapshot (`design.rs`), the tool object, callbacks and
//! iterators, behind generational handles. Safe Rust only; the C ABI in `abi.rs` translates to
//! and from it.

// VHPI names are matched as written in the C standard.
#![allow(non_upper_case_globals)]

use std::collections::{HashMap, HashSet};
use std::ffi::{CStr, CString};

use yavs_diag::{ColorChoice, Diagnostic, MessageFormat, SourceMap, render};
use yavs_kernel::{ForceMode, Range, SigRef, SimError, Value};
use yavs_sem::stat::{ConstValue, Elems};

use crate::abi::with_hook;
use crate::callbacks::{Callbacks, Clock, Watched};
use crate::design::{Design, Layout, Node, NodeId, Origin, ROOT, vhpi_name};
use crate::sim::Update;
use crate::sys::*;
use crate::types::{TyId, TypeObj};
use crate::value::{self, Read, Scalar};

/// Where a declaration or sub-element lives (`State::at`).
pub(crate) struct At<'a> {
    pub(crate) node: &'a Node,
    /// The layout of the sub-element.
    pub(crate) l: &'a Layout,
    /// Signals: the first kernel leaf of the sub-element, the row-major instance of it among
    /// that leaf's repetitions and the number of repetitions.
    leaf: u32,
    k: u64,
    reps: u64,
    /// Generics and constants: the elaborated value.
    cv: Option<ConstValue>,
    /// Where the leaves are stored.
    locs: &'a [yavs_elab::LeafLoc],
}

impl At<'_> {
    /// The kernel elements of a scalar or an array of `len` scalars of type `ty`.
    pub(crate) fn signal_ref(&self, ty: &Scalar, len: u64) -> SigRef {
        // A logic leaf stores its rightmost element at offset 0 (yavs-elab `hier`).
        let first = self.k * len;
        let offset = if ty.on_planes() {
            self.reps * len - first - len
        } else {
            first
        };
        let at = self.locs[self.leaf as usize];
        at.signal.slice(at.base + offset as u32, len as u32)
    }
}

/// What a handle points at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Object {
    Tool,
    /// Argument `State::argv[i]` (`vhpiArgvK`).
    Argv(usize),
    /// Id in `State::cbs`.
    Callback(u64),
    /// A region, declaration or design unit.
    Node(NodeId),
    /// Sub-element `path` of declaration `base`: per step a record field index or a row-major
    /// array element index (`Layout::step`).
    Elem {
        base: NodeId,
        path: Box<[u64]>,
    },
    Iter(Iter),
    /// A type, range constraint, enumeration literal, record element or unit (`types.rs`).
    Type(TypeObj),
    /// Registration `i` of `vhpi_register_foreignf` (`foreign.rs`).
    Foreign(usize),
    /// The call of a foreign subprogram that a model's `execf` is running (key in
    /// `Foreign::calls`); a function call is also the function's result.
    Call(u64),
    /// Process `i` a foreign architecture's model created (`vhpi_create`, `foreign.rs`).
    Process(usize),
    /// Driver `i` a foreign architecture's model created.
    Driver(usize),
    /// Parameter `idx` of foreign call `call`, or a part of it.
    Param {
        call: u64,
        idx: u32,
        part: Part,
    },
}

/// What of a foreign call's parameter an [`Object::Param`] is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Part {
    Whole,
    /// Field `i` of a record parameter (`vhpiSelectedNames`).
    Field(u32),
    /// The caller's driver of an `out` or `inout` signal parameter (`vhpiDrivers`).
    Driver,
}

/// The state of a `vhpi_iterator`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Iter {
    List {
        items: Vec<Object>,
        next: usize,
    },
    /// Sub-elements `next..end` of the object `prefix` names (lazy: arrays can be large).
    Elems {
        base: NodeId,
        path: Box<[u64]>,
        next: u64,
        end: u64,
    },
}

impl Iter {
    /// The object `i` places after the next one.
    pub(crate) fn nth(&self, i: u64) -> Option<Object> {
        match self {
            Iter::List { items, next } => items.get(*next + usize::try_from(i).ok()?).cloned(),
            Iter::Elems {
                base,
                path,
                next,
                end,
            } => {
                let at = next.checked_add(i).filter(|a| a < end)?;
                Some(Object::Elem {
                    base: *base,
                    path: path.iter().copied().chain([at]).collect(),
                })
            }
        }
    }

    pub(crate) fn scan(&mut self) -> Option<Object> {
        let o = self.nth(0)?;
        match self {
            Iter::List { next, .. } => *next += 1,
            Iter::Elems { next, .. } => *next += 1,
        }
        Some(o)
    }
}

/// Handle arena (design rule 9). A handle is `(index, generation)`
/// packed into the pointer value: index + 1 in the low 32 bits, so a
/// handle is never NULL, and the generation in the high 32 bits, so a
/// released slot that has been reused does not answer to a stale handle.
#[derive(Default)]
pub(crate) struct Handles {
    slots: Vec<Slot>,
    free: Vec<u32>,
}

struct Slot {
    generation: u32,
    obj: Option<Object>,
}

#[cfg(not(target_pointer_width = "64"))]
compile_error!("VHPI handles pack 64 bits into a pointer");

impl Handles {
    pub(crate) fn alloc(&mut self, obj: Object) -> vhpiHandleT {
        let index = match self.free.pop() {
            Some(i) => {
                self.slots[i as usize].obj = Some(obj);
                i
            }
            None => {
                self.slots.push(Slot {
                    generation: 0,
                    obj: Some(obj),
                });
                u32::try_from(self.slots.len() - 1).expect("handle arena full")
            }
        };
        let raw = (u64::from(self.slots[index as usize].generation) << 32) | (u64::from(index) + 1);
        raw as usize as vhpiHandleT
    }

    fn slot(&self, h: vhpiHandleT) -> Option<(u32, &Slot)> {
        let raw = h as usize as u64;
        let index = (raw as u32).checked_sub(1)?;
        let slot = self.slots.get(index as usize)?;
        (slot.generation == (raw >> 32) as u32 && slot.obj.is_some()).then_some((index, slot))
    }

    pub(crate) fn get(&self, h: vhpiHandleT) -> Option<&Object> {
        self.slot(h).and_then(|(_, s)| s.obj.as_ref())
    }

    pub(crate) fn get_mut(&mut self, h: vhpiHandleT) -> Option<&mut Object> {
        let (index, _) = self.slot(h)?;
        self.slots[index as usize].obj.as_mut()
    }

    /// Frees the slot and bumps its generation. False for a stale handle.
    pub(crate) fn release(&mut self, h: vhpiHandleT) -> bool {
        let Some((index, _)) = self.slot(h) else {
            return false;
        };
        let slot = &mut self.slots[index as usize];
        slot.obj = None;
        slot.generation = slot.generation.wrapping_add(1);
        self.free.push(index);
        true
    }

    #[cfg(test)]
    fn live(&self) -> usize {
        self.slots.len() - self.free.len()
    }
}

/// A `vhpi_put_value` the kernel has not seen yet: made from a callback or before the
/// simulation, applied when the callback returns (`sim.rs`), so by the kernel's rules for
/// observer forces and deposits.
pub(crate) struct Put {
    pub(crate) at: SigRef,
    pub(crate) op: PutOp,
}

pub(crate) enum PutOp {
    Deposit(Value),
    Force(ForceMode, Value),
    Release(ForceMode),
}

/// The pending request from `vhpi_control`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Control {
    Stop,
    Finish,
}

pub(crate) struct Error {
    pub(crate) severity: vhpiSeverityT,
    pub(crate) message: CString,
}

/// Why a VHPI call failed. Every failure is readable with
/// `vhpi_check_error`; misuse and unsupported calls are also reported on
/// stderr, so a plugin that ignores errors still leaves a trace.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Fault {
    /// A bad argument: NULL, a stale handle, a wrong handle kind. Reported
    /// every time (W0903).
    Misuse,
    /// Valid VHPI that yavs does not implement yet. Reported once per
    /// distinct message (W0904).
    Unsupported,
    /// A lookup that found nothing. Plugins probe names routinely (cocotb
    /// does), so this is only reported through `vhpi_check_error`.
    NotFound,
}

/// The one-to-one relations of type objects (`types.rs`).
macro_rules! TYPE_RELATIONS {
    () => {
        vhpiBaseType | vhpiType | DEPRECATED_vhpiSubtype | vhpiElemType | DEPRECATED_vhpiElemSubtype
    };
}

/// The properties `State::int_property` and `State::str_property` implement.
pub(crate) const INT_PROPS: &[vhpiIntPropertyT] = &[
    vhpiKindP,
    vhpiStateP,
    vhpiReasonP,
    vhpiArgcP,
    vhpiLineNoP,
    vhpiModeP,
    vhpiIsCompositeP,
    vhpiSizeP,
    vhpiNumLiteralsP,
    vhpiNumDimensionsP,
    vhpiNumFieldsP,
    DEPRECATED_vhpiNumFieldsP,
    vhpiIsUnconstrainedP,
    vhpiIsScalarP,
    vhpiIsAnonymousP,
    vhpiPositionP,
    vhpiLeftBoundP,
    vhpiRightBoundP,
    vhpiIsUpP,
    vhpiIsNullP,
    vhpiStartLineNoP,
    vhpiNumPortsP,
    vhpiNumGensP,
    vhpiIsSharedP,
    vhpiGenerateIndexP,
    vhpiCapabilitiesP,
];
pub(crate) const STR_PROPS: &[vhpiStrPropertyT] = &[
    vhpiNameP,
    vhpiUnitNameP,
    vhpiCaseNameP,
    vhpiFullNameP,
    vhpiFullCaseNameP,
    vhpiKindStrP,
    vhpiFileNameP,
    vhpiDefNameP,
    vhpiToolVersionP,
    vhpiLabelNameP,
];

#[derive(Default)]
pub(crate) struct State {
    pub(crate) handles: Handles,
    pub(crate) cbs: Callbacks,
    pub(crate) foreign: crate::foreign::Foreign,
    pub(crate) clock: Clock,
    pub(crate) design: Design,
    /// The tool's arguments (`vhpiArgcP`, `vhpiArgvs`): the program name, then the plusargs.
    pub(crate) argv: Vec<String>,
    pub(crate) control: Option<Control>,
    /// Puts waiting for the kernel, in call order.
    pub(crate) puts: Vec<Put>,
    /// A read-only phase (end of time step, end of simulation): puts are refused.
    pub(crate) read_only: bool,
    pub(crate) error: Option<Error>,
    /// The interpreter side of the simulation (`set_runtime`): shared variables (#309).
    pub(crate) built: Option<yavs_elab::Built>,
    /// Strings handed out by `vhpi_get_str`. They are never freed, so the
    /// pointers stay valid; the set is small and bounded by the design.
    strings: HashMap<String, CString>,
    /// Unsupported-call messages already reported on stderr.
    reported: HashSet<String>,
}

impl State {
    pub(crate) fn fail(&mut self, fault: Fault, message: impl Into<String>) {
        let message = message.into();
        let diag = match fault {
            Fault::Misuse => Some(
                Diagnostic::warning("W0903", format!("VHPI misuse: {message}"))
                    .with_note("the plugin can read this error with vhpi_check_error"),
            ),
            Fault::Unsupported if self.reported.insert(message.clone()) => Some(
                Diagnostic::warning("W0904", format!("unsupported VHPI call: {message}"))
                    .with_note("yavs's VHPI object model is not complete until milestone M5"),
            ),
            Fault::Unsupported | Fault::NotFound => None,
        };
        if let Some(diag) = diag {
            let color = ColorChoice::Auto.use_color();
            eprint!(
                "{}",
                render(&SourceMap::default(), &diag, MessageFormat::Human, color)
            );
        }
        // A NUL inside the message would truncate it in C; nothing else can
        // make CString::new fail.
        let message = CString::new(message.replace('\0', " ")).expect("NUL removed");
        self.error = Some(Error {
            severity: vhpiError,
            message,
        });
    }

    pub(crate) fn intern(&mut self, s: String) -> *const vhpiCharT {
        self.strings
            .entry(s)
            .or_insert_with_key(|k| CString::new(k.replace('\0', " ")).expect("NUL removed"))
            .as_ptr()
            .cast()
    }

    /// `vhpi_handle_by_name`: absolute or relative to region `scope`.
    pub(crate) fn lookup(&self, name: &CStr, scope: Option<&Object>) -> Option<Object> {
        let name = name.to_str().ok()?;
        let scope = match scope {
            None => None,
            Some(Object::Node(n)) => Some(*n),
            Some(_) => return None,
        };
        if scope.is_none() && matches!(name, "" | ":") {
            return Some(Object::Node(ROOT));
        }
        let (base, path) = self.design.lookup(name, scope)?;
        Some(if path.is_empty() {
            Object::Node(base)
        } else {
            Object::Elem {
                base,
                path: path.into(),
            }
        })
    }

    /// The layout of an object (a declaration or a sub-element).
    fn layout(&self, obj: &Object) -> Option<&Layout> {
        match obj {
            Object::Node(n) => self.design.node(*n).layout.as_ref(),
            Object::Elem { base, path } => Some(self.design.elem(*base, path)?.layout),
            _ => None,
        }
    }

    /// Where `obj` (a declaration or sub-element) lives; `func` names the VHPI call for
    /// errors.
    pub(crate) fn at(&self, obj: &Object, func: &str) -> Result<At<'_>, (Fault, String)> {
        let misuse = || (Fault::Misuse, format!("{func}: the object has no value"));
        let (base, path): (NodeId, &[u64]) = match obj {
            Object::Node(n) => (*n, &[]),
            Object::Elem { base, path } => (*base, path),
            _ => return Err(misuse()),
        };
        let node = self.design.node(base);
        let mut l = node.layout.as_ref().ok_or_else(misuse)?;
        let (mut leaf, mut k, mut reps) = match node.origin {
            Origin::Signal { leaf, .. } => (leaf, 0u64, 1u64),
            _ => (0, 0, 1),
        };
        let mut cv = match &node.origin {
            Origin::Decl { value, .. } => value.clone(),
            _ => None,
        };
        for &i in path {
            match l {
                Layout::Array { elem, .. } => {
                    let n = l.count();
                    (k, reps) = (k * n + i, reps * n);
                    cv = cv.and_then(|v| const_elem(&v, i));
                    l = elem;
                }
                Layout::Record(fields) => {
                    let i = i as usize;
                    leaf += fields[..i].iter().map(|(_, f)| leaves(f)).sum::<u32>();
                    cv = match cv {
                        Some(ConstValue::Record(r)) => r.get(i).cloned(),
                        _ => None,
                    };
                    l = &fields[i].1;
                }
                _ => unreachable!("element handles follow the layout"),
            }
        }
        Ok(At {
            node,
            l,
            leaf,
            k,
            reps,
            cv,
            locs: &self.design.locs,
        })
    }

    /// The element type, element count and arrayness of a scalar or an array of scalars.
    pub(crate) fn scalars<'l>(
        l: &'l Layout,
        func: &str,
    ) -> Result<(&'l Scalar, u64, bool), (Fault, String)> {
        match l {
            Layout::Scalar(t) => Ok((t, 1, false)),
            Layout::Array { elem, .. } => match &**elem {
                Layout::Scalar(t) => Ok((t, l.count(), true)),
                _ => Err((
                    Fault::Unsupported,
                    format!("{func} of an array of composites: use its elements"),
                )),
            },
            Layout::Record(_) => Err((
                Fault::Unsupported,
                format!("{func} of a record: use its elements"),
            )),
            Layout::Open => Err((
                Fault::Misuse,
                format!("{func}: the object's bounds are not known"),
            )),
        }
    }

    /// `vhpi_get_value`: the value of a declaration or sub-element, a scalar or an array of
    /// scalars. Signals are read through `kernel` (`None` outside a simulation callback).
    pub(crate) fn read(
        &self,
        obj: &Object,
        kernel: impl FnOnce(SigRef) -> Option<Result<Value, SimError>>,
    ) -> Result<Read, (Fault, String)> {
        let misuse = |m: &str| (Fault::Misuse, format!("vhpi_get_value: {m}"));
        if matches!(obj, Object::Call(_) | Object::Param { .. }) {
            return self.foreign_read(obj);
        }
        let at = self.at(obj, "vhpi_get_value")?;
        let (ty, len, array) = Self::scalars(at.l, "vhpi_get_value")?;
        match &at.node.origin {
            Origin::Signal { .. } => {
                let v = kernel(at.signal_ref(ty, len))
                    .ok_or_else(|| misuse("signal values are read in simulation callbacks"))?
                    .map_err(|e| {
                        (
                            Fault::Misuse,
                            format!("vhpi_get_value: this is a bug in yavs: {e:?}"),
                        )
                    })?;
                Read::from_kernel(ty.clone(), array, &v).ok_or_else(|| {
                    misuse("this is a bug in yavs: the kernel value does not match the type")
                })
            }
            Origin::Decl { value: Some(_), .. } => at
                .cv
                .as_ref()
                .and_then(|v| Read::from_const(ty.clone(), v))
                .ok_or_else(|| misuse("this is a bug in yavs: no elaborated value")),
            Origin::Var { .. } => {
                let whole = self
                    .var(at.node, None)
                    .ok_or_else(|| misuse("variable values are read in simulation callbacks"))?
                    .map_err(|m| misuse(&m))?;
                descend(whole, Self::path(obj))
                    .and_then(|v| Read::from_const(ty.clone(), &v))
                    .ok_or_else(|| misuse("this is a bug in yavs: the variable's value"))
            }
            _ => Err((
                Fault::Unsupported,
                "vhpi_get_value of an object without an elaborated value".into(),
            )),
        }
    }

    /// The sub-element path of a declaration (empty) or sub-element.
    fn path(obj: &Object) -> &[u64] {
        match obj {
            Object::Elem { path, .. } => path,
            _ => &[],
        }
    }

    /// The value of variable `node` (#309), changed by `update` when given; `None` outside a
    /// simulation callback (processes are suspended only there).
    fn var(&self, node: &Node, update: Update<'_>) -> Option<Result<ConstValue, String>> {
        let Origin::Var { proc, scope, decl } = node.origin else {
            return Some(Err("this is a bug in yavs: not a variable".into()));
        };
        with_hook(|h| match proc {
            Some(p) => crate::sim::var(h, p, &node.name, update),
            None => match self.built.as_ref() {
                Some(b) => crate::sim::shared_var(b, scope, decl, update),
                None => Err("shared variables are read in a simulation".into()),
            },
        })
    }

    /// `vhpi_put_value` on a variable or one of its sub-elements (#309): a deposit, written
    /// into the variable at once (its processes are suspended).
    fn put_var(
        &self,
        obj: &Object,
        at: &At<'_>,
        mode: vhpiPutValueModeT,
        decode: impl FnOnce(&Scalar, Option<usize>) -> Result<Read, value::Error>,
    ) -> Result<(), (Fault, String)> {
        let misuse = |m: &str| (Fault::Misuse, format!("vhpi_put_value: {m}"));
        if !matches!(mode, vhpiDeposit | vhpiDepositPropagate) {
            return Err(misuse("a variable is deposited, not forced or released"));
        }
        if self.read_only {
            return Err(misuse(
                "no writes in a read-only phase (end of time step, end of simulation)",
            ));
        }
        let (ty, len, array) = Self::scalars(at.l, "vhpi_put_value")?;
        let v = decode(ty, array.then_some(len as usize)).map_err(|e| match e {
            value::Error::Unsupported(m) => (Fault::Unsupported, m),
            value::Error::Mismatch(m) => misuse(&m),
        })?;
        let path = Self::path(obj);
        let mut update = |whole: &mut ConstValue| {
            let new = v.to_const(&descend(whole.clone(), path)?)?;
            replace(whole, path, new)
        };
        self.var(at.node, Some(&mut update))
            .ok_or_else(|| misuse("variables are written in simulation callbacks"))?
            .map(|_| ())
            .map_err(|m| misuse(&m))
    }

    /// The type of a declaration or sub-element.
    fn obj_ty(&self, obj: &Object) -> Option<TyId> {
        match obj {
            Object::Node(n) => self.design.node(*n).ty,
            Object::Elem { base, path } => {
                self.design.types.elem(self.design.node(*base).ty?, path)
            }
            _ => None,
        }
    }

    /// `vhpi_put_value` on a signal, port or one of their sub-elements: checks the object,
    /// the phase and the value (`decode` turns the plugin's value into elements of the
    /// object's type, `n` of them for an array), then queues the kernel operation.
    pub(crate) fn put(
        &mut self,
        obj: &Object,
        mode: vhpiPutValueModeT,
        decode: impl FnOnce(&Scalar, Option<usize>) -> Result<Read, value::Error>,
    ) -> Result<(), (Fault, String)> {
        let misuse = |m: String| (Fault::Misuse, format!("vhpi_put_value: {m}"));
        if mode == vhpiSizeConstraint {
            return Err(misuse(
                "vhpiSizeConstraint applies to the result of a foreign function".into(),
            ));
        }
        if !matches!(
            mode,
            vhpiDeposit | vhpiDepositPropagate | vhpiForce | vhpiForcePropagate | vhpiRelease
        ) {
            return Err(misuse(format!("bad mode {mode}")));
        }
        let at = self.at(obj, "vhpi_put_value")?;
        match (&at.node.origin, at.node.kind) {
            (Origin::Signal { .. }, _) => {}
            (Origin::Var { .. }, _) => return self.put_var(obj, &at, mode, decode),
            _ => {
                let kind = self.kind(obj);
                return Err(misuse(format!(
                    "an object of kind {kind} cannot be written, only signals and ports"
                )));
            }
        }
        // Two of the three references with VHPI refuse them on signals (user, 2026-10-19).
        if matches!(mode, vhpiDeposit | vhpiForce) {
            return Err(misuse(format!(
                "the non-propagating mode {} does not apply to a signal; use {}",
                if mode == vhpiDeposit {
                    "vhpiDeposit"
                } else {
                    "vhpiForce"
                },
                if mode == vhpiDeposit {
                    "vhpiDepositPropagate"
                } else {
                    "vhpiForcePropagate"
                },
            )));
        }
        if self.read_only {
            return Err(misuse(
                "no writes in a read-only phase (end of time step, end of simulation)".into(),
            ));
        }
        let (ty, len, array) = Self::scalars(at.l, "vhpi_put_value")?;
        let r = at.signal_ref(ty, len);
        // The default force mode of a force statement (LRM 10.5.2.1): `out` for a port of
        // mode out, inout or buffer, `in` otherwise.
        let fm = match at.node.mode {
            Some(vhpiOutMode | vhpiInoutMode | vhpiBufferMode) => ForceMode::Driving,
            _ => ForceMode::Effective,
        };
        let op = if mode == vhpiRelease {
            PutOp::Release(fm)
        } else {
            let n = array.then_some(len as usize);
            let v = decode(ty, n)
                .map_err(|e| match e {
                    value::Error::Unsupported(m) => (Fault::Unsupported, m),
                    value::Error::Mismatch(m) => misuse(m),
                })?
                .to_kernel();
            match mode {
                vhpiForce | vhpiForcePropagate => PutOp::Force(fm, v),
                _ => PutOp::Deposit(v),
            }
        };
        self.puts.push(Put { at: r, op });
        Ok(())
    }

    /// The kernel elements of a signal, port or one of their sub-elements (value-change
    /// callbacks), in leaf order.
    pub(crate) fn watch(&self, obj: &Object) -> Option<Watched> {
        let at = self.at(obj, "vhpi_register_cb").ok()?;
        if !matches!(at.node.origin, Origin::Signal { .. }) {
            return None;
        }
        // Per leaf under the sub-layout: whether it is on planes and its elements per
        // instance of the sub-layout.
        fn walk(l: &Layout, m: u64, out: &mut Vec<(bool, u64)>) {
            match l {
                Layout::Scalar(t) => out.push((t.on_planes(), m)),
                Layout::Array { elem, .. } => walk(elem, m * l.count(), out),
                Layout::Record(fields) => fields.iter().for_each(|(_, f)| walk(f, m, out)),
                Layout::Open => out.push((false, 0)),
            }
        }
        let mut leaves = Vec::new();
        walk(at.l, 1, &mut leaves);
        Some(
            leaves
                .iter()
                .enumerate()
                .map(|(j, &(logic, m))| {
                    let first = at.k * m;
                    let offset = if logic {
                        at.reps * m - first - m
                    } else {
                        first
                    };
                    let loc = at.locs[at.leaf as usize + j];
                    (loc.signal, Range::new(loc.base + offset as u32, m as u32))
                })
                .collect(),
        )
    }

    /// The declaration an object is or is part of.
    fn base(obj: &Object) -> Option<NodeId> {
        match obj {
            Object::Node(n) | Object::Elem { base: n, .. } => Some(*n),
            _ => None,
        }
    }

    pub(crate) fn kind(&self, obj: &Object) -> vhpiClassKindT {
        match obj {
            Object::Tool => vhpiToolK,
            Object::Argv(_) => vhpiArgvK,
            Object::Callback(_) => vhpiCallbackK,
            Object::Iter(_) => vhpiIteratorK,
            Object::Type(o) => self.design.types.kind(*o),
            Object::Foreign(_) | Object::Call(_) | Object::Param { .. } => self.foreign_kind(obj),
            Object::Process(_) => vhpiProcessStmtK,
            Object::Driver(_) => vhpiDriverK,
            Object::Node(n) => self.design.node(*n).kind,
            Object::Elem { base, path } => {
                self.design
                    .elem(*base, path)
                    .expect("element handles are made valid")
                    .kind
            }
        }
    }

    /// Case name and full case name of a node or element.
    fn names(&self, obj: &Object) -> Option<(String, String)> {
        match obj {
            Object::Node(n) => {
                let n = self.design.node(*n);
                Some((n.name.clone(), n.full.clone()))
            }
            Object::Elem { base, path } => {
                let e = self.design.elem(*base, path)?;
                Some((e.name, e.full))
            }
            _ => None,
        }
    }

    pub(crate) fn str_property(&self, obj: &Object, prop: vhpiStrPropertyT) -> Option<String> {
        let node = Self::base(obj).map(|n| self.design.node(n));
        Some(match (obj, prop) {
            (Object::Tool, vhpiNameP) => "yavs".into(),
            (Object::Argv(i), vhpiStrValP) => self.argv[*i].clone(),
            (Object::Tool, vhpiToolVersionP) => env!("CARGO_PKG_VERSION").into(),
            (Object::Type(o), _) if prop != vhpiKindStrP => {
                return self.design.types.str_property(*o, prop);
            }
            (Object::Foreign(_) | Object::Call(_) | Object::Param { .. }, _)
                if prop != vhpiKindStrP =>
            {
                return self.foreign_str(obj, prop);
            }
            (_, vhpiKindStrP) => {
                let k = i64::from(self.kind(obj));
                VHPICLASSKINDT_VALUES
                    .iter()
                    .find(|&&(_, v)| v == k)
                    .map(|&(name, _)| name)?
                    .into()
            }
            (_, vhpiNameP) => vhpi_name(&self.names(obj)?.0),
            (_, vhpiCaseNameP) => self.names(obj)?.0,
            (_, vhpiFullNameP) => vhpi_name(&self.names(obj)?.1),
            (_, vhpiFullCaseNameP) => self.names(obj)?.1,
            (_, vhpiFileNameP) => node?.file.clone()?,
            (Object::Node(_), vhpiDefNameP) => node?.def_name.clone()?,
            // A statement's label: an instance, block, generate or process without the index.
            (Object::Node(_), vhpiLabelNameP)
                if matches!(
                    node?.kind,
                    vhpiCompInstStmtK
                        | vhpiBlockStmtK
                        | vhpiForGenerateK
                        | vhpiIfGenerateK
                        | vhpiProcessStmtK
                ) =>
            {
                let n = &node?.name;
                n.split_once('(').map_or(n.as_str(), |(l, _)| l).to_owned()
            }
            _ => return None,
        })
    }

    pub(crate) fn int_property(&self, obj: &Object, prop: vhpiIntPropertyT) -> Option<vhpiIntT> {
        let node = Self::base(obj).map(|n| self.design.node(n));
        Some(match (obj, prop) {
            (_, vhpiKindP) => self.kind(obj),
            (Object::Type(o), _) => return self.design.types.int_property(*o, prop),
            (Object::Foreign(_) | Object::Call(_) | Object::Param { .. }, _) => {
                return self.foreign_int(obj, prop);
            }
            (Object::Callback(i), vhpiStateP) => self.cbs.get(*i)?.state,
            (Object::Callback(i), vhpiReasonP) => self.cbs.get(*i)?.data.reason,
            (Object::Tool, vhpiArgcP) => vhpiIntT::try_from(self.argv.len()).ok()?,
            // What yavs provides: the hierarchy and static values, foreign models with
            // `vhpi_create`, values, puts and callbacks at run time.
            (Object::Tool, vhpiCapabilitiesP) => {
                vhpiProvidesHierarchy
                    | vhpiProvidesStaticAccess
                    | vhpiProvidesForeignModel
                    | vhpiProvidesAdvancedForeignModel
                    | vhpiProvidesDebugRuntime
            }
            (_, vhpiLineNoP | vhpiStartLineNoP) => node?.line?,
            (Object::Node(n), vhpiNumPortsP | vhpiNumGensP)
                if node?.layout.is_none() && node?.kind != vhpiAttrDeclK =>
            {
                let k = if prop == vhpiNumPortsP {
                    vhpiPortDeclK
                } else {
                    vhpiGenericDeclK
                };
                let d = self.design.node(*n);
                d.decls
                    .iter()
                    .filter(|&&x| self.design.node(x).kind == k)
                    .count() as vhpiIntT
            }
            (Object::Node(_), vhpiIsSharedP) if node?.kind == vhpiVarDeclK => match node?.origin {
                Origin::Var { proc, .. } => vhpiIntT::from(proc.is_none()),
                _ => return None,
            },
            // The value of a for generate's parameter, when it is an integer.
            (Object::Node(_), vhpiGenerateIndexP) if node?.kind == vhpiForGenerateK => {
                let (_, i) = node?.name.strip_suffix(')')?.rsplit_once('(')?;
                i.parse().ok()?
            }
            (_, vhpiModeP) => node?.mode?,
            (_, vhpiIsCompositeP) => match self.layout(obj)? {
                Layout::Scalar(_) => vhpiFalse,
                _ => vhpiTrue,
            },
            (_, vhpiSizeP) => i32::try_from(self.layout(obj)?.scalars()?).ok()?,
            _ => return None,
        })
    }

    /// `vhpi_handle(kind, reference)`. `Err(true)` when the relation is not implemented,
    /// `Err(false)` when the object has no such related object.
    pub(crate) fn one_to_one(
        &self,
        kind: vhpiOneToOneT,
        reference: Option<&Object>,
    ) -> Result<Object, bool> {
        let node = reference.and_then(Self::base).map(|n| self.design.node(n));
        match (kind, reference) {
            (vhpiRootInst, _) => Ok(Object::Node(ROOT)),
            (vhpiTool, None) => Ok(Object::Tool),
            (vhpiUpperRegion | vhpiImmRegion, Some(Object::Node(_))) => {
                node.and_then(|n| n.parent).map(Object::Node).ok_or(false)
            }
            (vhpiDesignUnit, Some(Object::Node(_)))
                if matches!(
                    node.map(|n| n.kind),
                    Some(vhpiRootInstK | vhpiCompInstStmtK | vhpiPackInstK)
                ) =>
            {
                node.and_then(|n| n.unit).map(Object::Node).ok_or(false)
            }
            (vhpiPrimaryUnit, Some(Object::Node(_)))
                if node.map(|n| n.kind) == Some(vhpiArchBodyK) =>
            {
                node.and_then(|n| n.unit).map(Object::Node).ok_or(false)
            }
            (vhpiAttrDecl, Some(Object::Node(n))) => self
                .design
                .attr_decls
                .get(n)
                .map(|&d| Object::Node(d))
                .ok_or(false),
            (vhpiPrefix | vhpiParent, Some(Object::Elem { base, path })) => {
                Ok(match &path[..path.len() - 1] {
                    [] => Object::Node(*base),
                    p => Object::Elem {
                        base: *base,
                        path: p.into(),
                    },
                })
            }
            (
                vhpiType | DEPRECATED_vhpiSubtype | vhpiBaseType,
                Some(o @ (Object::Node(_) | Object::Elem { .. })),
            ) => {
                let t = self.obj_ty(o).ok_or(false)?;
                let t = if kind == vhpiBaseType {
                    self.design.types.base(t)
                } else {
                    t
                };
                Ok(Object::Type(TypeObj::Ty(t)))
            }
            (TYPE_RELATIONS!(), Some(Object::Type(o))) => self
                .design
                .types
                .one_to_one(kind, *o)
                .map(Object::Type)
                .ok_or(false),
            (TYPE_RELATIONS!(), _) => Err(false),
            (vhpiDesignUnit | vhpiPrimaryUnit | vhpiUpperRegion | vhpiImmRegion, _) => Err(false),
            (vhpiPrefix | vhpiParent, Some(Object::Node(_))) => Err(false),
            _ => Err(true),
        }
    }

    /// `vhpi_iterator(kind, reference)`. `Err(true)` when the relation is not implemented for
    /// this object; an empty iteration is `Ok(None)` (VHPI returns NULL).
    pub(crate) fn iterate(
        &self,
        rel: vhpiOneToManyT,
        reference: Option<&Object>,
    ) -> Result<Option<Iter>, bool> {
        let list =
            |items: Vec<Object>| Ok((!items.is_empty()).then_some(Iter::List { items, next: 0 }));
        if let (vhpiParamDecls, Some(o @ Object::Call(_))) = (rel, reference) {
            return list(self.foreign_params(o).ok_or(false)?);
        }
        if let Some(o @ Object::Param { .. }) = reference {
            return list(self.foreign_iter(rel, o).ok_or(false)?);
        }
        if matches!(
            rel,
            vhpiConstraints | vhpiEnumLiterals | vhpiRecordElems | vhpiUnitDecls
        ) {
            // A declaration iterates its type's constraints.
            let t = match reference {
                Some(Object::Type(TypeObj::Ty(t))) => Some(*t),
                Some(o @ (Object::Node(_) | Object::Elem { .. })) if rel == vhpiConstraints => {
                    self.obj_ty(o)
                }
                _ => None,
            };
            let items = t
                .and_then(|t| self.design.types.iterate(rel, t))
                .ok_or(false)?;
            return list(items.into_iter().map(Object::Type).collect());
        }
        let decls = |n: NodeId, k: Option<vhpiClassKindT>| {
            list(
                self.design
                    .node(n)
                    .decls
                    .iter()
                    .filter(|&&d| k.is_none_or(|k| self.design.node(d).kind == k))
                    .map(|&d| Object::Node(d))
                    .collect(),
            )
        };
        let regions = |n: NodeId, keep: &dyn Fn(vhpiClassKindT) -> bool| {
            list(
                self.design
                    .node(n)
                    .regions
                    .iter()
                    .filter(|&&r| keep(self.design.node(r).kind))
                    .map(|&r| Object::Node(r))
                    .collect(),
            )
        };
        // A declaration's user attribute specifications are its `vhpiAttrSpecK` children.
        if let (vhpiAttrSpecs, Some(Object::Node(n))) = (rel, reference)
            && self.design.node(*n).layout.is_some()
        {
            return decls(*n, Some(vhpiAttrSpecK));
        }
        let region = match reference {
            Some(Object::Node(n)) if self.design.node(*n).layout.is_none() => Some(*n),
            _ => None,
        };
        match (rel, region) {
            (vhpiArgvs, None) if reference == Some(&Object::Tool) => {
                list((0..self.argv.len()).map(Object::Argv).collect())
            }
            (vhpiPackInsts, _) if reference.is_none() || region == Some(ROOT) => list(
                self.design
                    .packages
                    .iter()
                    .map(|&p| Object::Node(p))
                    .collect(),
            ),
            (vhpiInternalRegions, Some(n)) => regions(n, &|_| true),
            // Child statements (#316); a process has none, an empty iteration without error.
            (vhpiBlockStmts, Some(n)) => regions(n, &|k| k == vhpiBlockStmtK),
            (vhpiCompInstStmts, Some(n)) => regions(n, &|k| k == vhpiCompInstStmtK),
            (vhpiStmts, Some(n)) => regions(n, &|_| true),
            (vhpiDecls, Some(n)) => decls(n, None),
            (vhpiSigDecls, Some(n)) => decls(n, Some(vhpiSigDeclK)),
            (vhpiPortDecls, Some(n)) => decls(n, Some(vhpiPortDeclK)),
            (vhpiGenericDecls, Some(n)) => decls(n, Some(vhpiGenericDeclK)),
            (vhpiConstDecls, Some(n)) => decls(n, Some(vhpiConstDeclK)),
            (vhpiVarDecls, Some(n)) => decls(n, Some(vhpiVarDeclK)),
            (vhpiIndexedNames | vhpiSelectedNames, None) => {
                let obj = reference.ok_or(false)?;
                let (base, path) = match obj {
                    Object::Node(n) => (*n, Box::default()),
                    Object::Elem { base, path } => (*base, path.clone()),
                    _ => return Err(false),
                };
                let end = match (rel, self.layout(obj).ok_or(false)?) {
                    (vhpiIndexedNames, l @ Layout::Array { .. })
                    | (vhpiSelectedNames, l @ Layout::Record(_)) => l.count(),
                    // ponytail: bounds known only at run time come with values (#270).
                    (_, Layout::Open) => return Err(true),
                    _ => 0,
                };
                Ok((end > 0).then_some(Iter::Elems {
                    base,
                    path,
                    next: 0,
                    end,
                }))
            }
            _ => Err(true),
        }
    }
}

/// Kernel leaves of a layout: one per scalar of its record structure.
fn leaves(l: &Layout) -> u32 {
    match l {
        Layout::Scalar(_) | Layout::Open => 1,
        Layout::Array { elem, .. } => leaves(elem),
        Layout::Record(fields) => fields.iter().map(|(_, f)| leaves(f)).sum(),
    }
}

/// Sub-element `path` of value `v`: per step a record field or a row-major element.
fn descend(mut v: ConstValue, path: &[u64]) -> Option<ConstValue> {
    for &i in path {
        v = match v {
            ConstValue::Record(r) => r.get(i as usize)?.clone(),
            a @ ConstValue::Array(_) => const_elem(&a, i)?,
            _ => return None,
        };
    }
    Some(v)
}

/// Replaces sub-element `path` of `v` with `new`.
fn replace(v: &mut ConstValue, path: &[u64], new: ConstValue) -> Option<()> {
    let Some((&i, rest)) = path.split_first() else {
        *v = new;
        return Some(());
    };
    match v {
        ConstValue::Record(r) => replace(r.get_mut(i as usize)?, rest, new),
        ConstValue::Array(a) => {
            let mut xs = value::elements(a);
            replace(xs.get_mut(i as usize)?, rest, new)?;
            a.elems = Elems::Runs(xs.into_iter().map(|x| (1, x)).collect());
            Some(())
        }
        _ => None,
    }
}

/// Row-major element `i` of an array value.
fn const_elem(v: &ConstValue, i: u64) -> Option<ConstValue> {
    let ConstValue::Array(a) = v else {
        return None;
    };
    match &a.elems {
        Elems::Bytes(b) => b.get(i as usize).map(|&x| ConstValue::Enum(x.into())),
        Elems::Runs(runs) => {
            let mut at = 0;
            runs.iter().find_map(|(n, v)| {
                at += n;
                (i < at).then(|| v.clone())
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn released_handles_go_stale_and_slots_are_reused() {
        let mut h = Handles::default();
        let a = h.alloc(Object::Node(ROOT));
        let b = h.alloc(Object::Node(ROOT));
        assert_eq!(h.get(a), Some(&Object::Node(ROOT)));
        assert!(h.release(a));
        assert!(!h.release(a), "double release");
        assert_eq!(h.get(a), None);
        let c = h.alloc(Object::Tool);
        assert_ne!(a, c, "reused slot has a new generation");
        assert_eq!(h.get(c), Some(&Object::Tool));
        assert_eq!(h.get(b), Some(&Object::Node(ROOT)));
        assert_eq!(h.live(), 2);
        assert_eq!(h.get(std::ptr::null_mut()), None);
        assert_eq!(h.get(0xdead_0000_0001usize as vhpiHandleT), None);
    }
}
