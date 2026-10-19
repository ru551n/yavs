//! The VHPI object model: the design snapshot (`design.rs`), the tool object, callbacks and
//! iterators, behind generational handles. Safe Rust only; the C ABI in `abi.rs` translates to
//! and from it.

// VHPI names are matched as written in the C standard.
#![allow(non_upper_case_globals)]

use std::collections::{HashMap, HashSet};
use std::ffi::{CStr, CString};

use yavs_diag::{ColorChoice, Diagnostic, MessageFormat, SourceMap, render};

use crate::callbacks::{Callbacks, Clock};
use crate::design::{Design, Layout, NodeId, ROOT, vhpi_name};
use crate::sys::*;

/// What a handle points at.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Object {
    Tool,
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
];
pub(crate) const STR_PROPS: &[vhpiStrPropertyT] = &[
    vhpiNameP,
    vhpiCaseNameP,
    vhpiFullNameP,
    vhpiFullCaseNameP,
    vhpiKindStrP,
    vhpiFileNameP,
    vhpiDefNameP,
    vhpiToolVersionP,
];

#[derive(Default)]
pub(crate) struct State {
    pub(crate) handles: Handles,
    pub(crate) cbs: Callbacks,
    pub(crate) clock: Clock,
    pub(crate) design: Design,
    pub(crate) control: Option<Control>,
    pub(crate) error: Option<Error>,
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
            Object::Callback(_) => vhpiCallbackK,
            Object::Iter(_) => vhpiIteratorK,
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
            (Object::Tool, vhpiToolVersionP) => env!("CARGO_PKG_VERSION").into(),
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
            _ => return None,
        })
    }

    pub(crate) fn int_property(&self, obj: &Object, prop: vhpiIntPropertyT) -> Option<vhpiIntT> {
        let node = Self::base(obj).map(|n| self.design.node(n));
        Some(match (obj, prop) {
            (_, vhpiKindP) => self.kind(obj),
            (Object::Callback(i), vhpiStateP) => self.cbs.get(*i)?.state,
            (Object::Callback(i), vhpiReasonP) => self.cbs.get(*i)?.data.reason,
            (Object::Tool, vhpiArgcP) => 0,
            (_, vhpiLineNoP) => node?.line?,
            (_, vhpiModeP) => node?.mode?,
            (_, vhpiIsCompositeP) => match self.layout(obj)? {
                Layout::Scalar => vhpiFalse,
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
            (vhpiPrefix | vhpiParent, Some(Object::Elem { base, path })) => {
                Ok(match &path[..path.len() - 1] {
                    [] => Object::Node(*base),
                    p => Object::Elem {
                        base: *base,
                        path: p.into(),
                    },
                })
            }
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
        let region = match reference {
            Some(Object::Node(n)) if self.design.node(*n).layout.is_none() => Some(*n),
            _ => None,
        };
        match (rel, region) {
            (vhpiPackInsts, _) if reference.is_none() || region == Some(ROOT) => list(
                self.design
                    .packages
                    .iter()
                    .map(|&p| Object::Node(p))
                    .collect(),
            ),
            (vhpiInternalRegions, Some(n)) => list(
                self.design
                    .node(n)
                    .regions
                    .iter()
                    .map(|&r| Object::Node(r))
                    .collect(),
            ),
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
