//! Stub VHPI object model: a fake design root, the tool object and
//! callbacks, behind generational handles. Safe Rust only; the C ABI in
//! `abi.rs` translates to and from it. The real model replaces the
//! `Object` variants in M5, the handle arena stays.

// VHPI names are matched as written in the C standard.
#![allow(non_upper_case_globals)]

use std::collections::{HashMap, HashSet};
use std::ffi::{CStr, CString};

use yavs_diag::{ColorChoice, Diagnostic, MessageFormat, SourceMap, render};

use crate::sys::*;

/// What a handle points at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Object {
    Tool,
    Root,
    /// Index into `State::callbacks`.
    Callback(usize),
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

    pub(crate) fn get(&self, h: vhpiHandleT) -> Option<Object> {
        self.slot(h).and_then(|(_, s)| s.obj)
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

pub(crate) struct Callback {
    pub(crate) data: vhpiCbDataT,
    pub(crate) state: vhpiStateT,
    pub(crate) removed: bool,
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

#[derive(Default)]
pub(crate) struct State {
    pub(crate) handles: Handles,
    pub(crate) callbacks: Vec<Callback>,
    pub(crate) root_name: String,
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
                    .with_note("yavs's VHPI object model is a stub until milestone M5"),
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

    pub(crate) fn lookup(&mut self, name: &CStr) -> Option<Object> {
        // VHDL names are case insensitive; accept "top", ":top" and ":".
        let name = name.to_str().ok()?;
        let name = name.strip_prefix(':').unwrap_or(name);
        (name.is_empty() || name.eq_ignore_ascii_case(&self.root_name)).then_some(Object::Root)
    }

    pub(crate) fn str_property(&self, obj: Object, prop: vhpiStrPropertyT) -> Option<String> {
        let root = &self.root_name;
        Some(match (obj, prop) {
            (Object::Root, vhpiNameP) => root.to_ascii_uppercase(),
            (Object::Root, vhpiCaseNameP) => root.clone(),
            (Object::Root, vhpiFullNameP) => format!(":{}", root.to_ascii_uppercase()),
            (Object::Root, vhpiFullCaseNameP) => format!(":{root}"),
            (Object::Tool, vhpiNameP) => "yavs".into(),
            (Object::Tool, vhpiToolVersionP) => env!("CARGO_PKG_VERSION").into(),
            (_, vhpiKindStrP) => match obj {
                Object::Tool => "vhpiToolK",
                Object::Root => "vhpiRootInstK",
                Object::Callback(_) => "vhpiCallbackK",
            }
            .into(),
            _ => return None,
        })
    }

    pub(crate) fn int_property(&self, obj: Object, prop: vhpiIntPropertyT) -> Option<vhpiIntT> {
        Some(match (obj, prop) {
            (Object::Tool, vhpiKindP) => vhpiToolK,
            (Object::Root, vhpiKindP) => vhpiRootInstK,
            (Object::Callback(_), vhpiKindP) => vhpiCallbackK,
            (Object::Callback(i), vhpiStateP) => self.callbacks[i].state,
            (Object::Callback(i), vhpiReasonP) => self.callbacks[i].data.reason,
            (Object::Tool, vhpiArgcP) => 0,
            _ => return None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn released_handles_go_stale_and_slots_are_reused() {
        let mut h = Handles::default();
        let a = h.alloc(Object::Root);
        let b = h.alloc(Object::Root);
        assert_eq!(h.get(a), Some(Object::Root));
        assert!(h.release(a));
        assert!(!h.release(a), "double release");
        assert_eq!(h.get(a), None);
        let c = h.alloc(Object::Tool);
        assert_ne!(a, c, "reused slot has a new generation");
        assert_eq!(h.get(c), Some(Object::Tool));
        assert_eq!(h.get(b), Some(Object::Root));
        assert_eq!(h.live(), 2);
        assert_eq!(h.get(std::ptr::null_mut()), None);
        assert_eq!(h.get(0xdead_0000_0001usize as vhpiHandleT), None);
    }
}
