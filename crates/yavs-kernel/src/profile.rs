//! The sampling profiler's side of the kernel (`yavs run --profile`).
//!
//! With a profile installed (`Profile::install`), every process is wrapped once when the kernel
//! is built (`Profile::wrap`): the wrapper stores the process's number in its thread's slot before
//! `Process::run` and 0 after it. A sampler thread of the CLI reads the slots (`Profile::sample`).
//! Without the option nothing is wrapped and no hot path knows the profiler exists.

use crate::error::SimError;
use crate::process::{Ctx, Process};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

/// One slot per thread that ever ran a profiled process, never freed (4 bytes): the process
/// number plus 1, 0 while the thread runs no process.
static SLOTS: Mutex<Vec<&'static AtomicU32>> = Mutex::new(Vec::new());

thread_local! {
    static SLOT: std::cell::Cell<Option<&'static AtomicU32>> = const { std::cell::Cell::new(None) };
}

#[inline]
fn slot() -> &'static AtomicU32 {
    SLOT.get().unwrap_or_else(new_slot)
}

#[cold]
fn new_slot() -> &'static AtomicU32 {
    let s: &'static AtomicU32 = Box::leak(Box::default());
    SLOTS.lock().unwrap_or_else(PoisonError::into_inner).push(s);
    SLOT.set(Some(s));
    s
}

/// The processes of a profiled kernel: their names and, for interpreted ones, their entry
/// function (`Process::entry_func`).
#[derive(Debug, Default)]
pub struct Profile {
    procs: Mutex<Vec<(String, Option<u32>)>>,
}

static INSTALLED: Mutex<Option<Arc<Profile>>> = Mutex::new(None);

pub(crate) fn installed() -> Option<Arc<Profile>> {
    INSTALLED
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
}

impl Profile {
    /// Profiles every kernel built from now on. A global rather than a `Config` field: the
    /// field moved the layout of the hot `SimControl` (7 percent of cycles on a long delta chain).
    pub fn install() -> Arc<Profile> {
        let p = Arc::new(Profile::default());
        *INSTALLED.lock().unwrap_or_else(PoisonError::into_inner) = Some(p.clone());
        p
    }

    /// Appends the number (`ProcId.0`) of the process each thread runs now.
    pub fn sample(&self, out: &mut Vec<u32>) {
        for s in SLOTS.lock().unwrap_or_else(PoisonError::into_inner).iter() {
            if let Some(p) = s.load(Ordering::Relaxed).checked_sub(1) {
                out.push(p);
            }
        }
    }

    /// The name and entry function of each process, by number.
    pub fn processes(&self) -> Vec<(String, Option<u32>)> {
        self.procs
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    pub(crate) fn wrap(&self, name: &str, inner: Box<dyn Process>) -> Box<dyn Process> {
        let mut procs = self.procs.lock().unwrap_or_else(PoisonError::into_inner);
        procs.push((name.to_string(), inner.entry_func()));
        Box::new(Profiled {
            inner,
            tag: procs.len() as u32,
        })
    }
}

struct Profiled {
    inner: Box<dyn Process>,
    tag: u32,
}

impl Process for Profiled {
    fn run(&mut self, ctx: &mut Ctx<'_>) -> Result<(), SimError> {
        let s = slot();
        s.store(self.tag, Ordering::Relaxed);
        let r = self.inner.run(ctx);
        s.store(0, Ordering::Relaxed);
        r
    }

    fn serial(&self) -> bool {
        self.inner.serial()
    }

    fn several_waits(&self) -> bool {
        self.inner.several_waits()
    }

    fn concurrent_call(&self) -> bool {
        self.inner.concurrent_call()
    }

    fn entry_func(&self) -> Option<u32> {
        self.inner.entry_func()
    }

    fn as_any(&mut self) -> Option<&mut dyn std::any::Any> {
        self.inner.as_any()
    }
}
