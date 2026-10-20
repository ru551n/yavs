//! `--tier=auto`: compile the functions that become hot, one at a time on one background
//! thread, while the simulation keeps running them in the interpreter (#125).
//!
//! The interpreter counts the instructions it runs per function (`Runtime::set_heat`) and
//! calls [`Tier::hot`] once a function reaches its [`Heat`] threshold; the function is queued,
//! compiled in a module of its own and published through a `OnceLock`. The interpreter asks
//! the tier after every jump, call and return, so the swap-in happens there in any frame of
//! that function: compiled code runs on the interpreter frame's registers and enters at any
//! pc (an instruction that is not an entry point is left to the interpreter), so output is
//! identical whenever compilation finishes (rule 5). At the end of a run `stop(false)` does
//! not wait for a function being compiled; the CLI then ends the process with [`exit_now`].

use std::panic::resume_unwind;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::thread::JoinHandle;

use yavs_interp::{Runtime, Tier, TierFrame, TierHost};
use yavs_ir::{FuncId, FuncKind};

use crate::lower::{RunFn, compile_one, enter, new_jit, too_big};
use crate::{Error, Jit, OptLevel};

/// When a function is hot: once the interpreter has run `base` of its instructions plus
/// `per_instr` per IR instruction the function has (`Runtime::set_heat` counts them).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Heat {
    pub base: u32,
    pub per_instr: u32,
}

impl Heat {
    /// Compiling a function costs about 2 ms plus 40 us per IR instruction at O0 (codegen
    /// nearly all of it), the time the interpreter needs for about 500k and 10k instructions:
    /// a function is compiled once its interpreted run time has reached its compile time.
    pub const DEFAULT: Heat = Heat {
        base: 500_000,
        per_instr: 10_000,
    };

    /// The threshold of `f`, saturating.
    pub fn threshold(self, f: &yavs_ir::Func) -> u32 {
        let n = u32::try_from(f.code.len()).unwrap_or(u32::MAX);
        self.base.saturating_add(self.per_instr.saturating_mul(n))
    }
}

/// What happened to a hot function: compiled (with the time it took), or why it stays
/// interpreted.
pub type Note = Box<dyn FnMut(FuncId, Result<std::time::Duration, String>) + Send>;

/// Hot functions compiled on a background thread; see the module docs.
pub struct AutoTier {
    funcs: Arc<[OnceLock<(RunFn, usize)>]>,
    queue: Mutex<Option<Sender<FuncId>>>,
    shared: Arc<Shared>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

/// What the tier and its compile thread share.
#[derive(Default)]
struct Shared {
    /// Set by `stop`: compile nothing more.
    halt: AtomicBool,
    /// The thread is compiling a function (or about to look at `halt` before it does).
    busy: AtomicBool,
    /// The JIT once the thread has ended: compiled code lives as long as the tier.
    jit: Mutex<Option<Jit>>,
}

impl AutoTier {
    /// Start the compile thread and make the tier the runtime's (`Runtime::set_tier`) with
    /// hotness counting at `heat`; functions over `max_size` IR instructions stay interpreted.
    /// `note` is called on the compile thread for every hot function.
    pub fn start(
        rt: &Runtime,
        opt: OptLevel,
        max_size: usize,
        heat: Heat,
        mut note: Note,
    ) -> Result<Arc<AutoTier>, Error> {
        let prog = rt.prog.clone();
        let funcs: Arc<[OnceLock<(RunFn, usize)>]> =
            (0..prog.funcs.len()).map(|_| OnceLock::new()).collect();
        let (tx, rx) = channel::<FuncId>();
        let shared = Arc::new(Shared::default());
        let (table, sh) = (funcs.clone(), shared.clone());
        let worker = std::thread::Builder::new()
            .name("yavs-tier-up".into())
            .spawn(move || {
                // Made with the first hot function: a run that never gets hot pays nothing.
                let mut jit: Option<Jit> = None;
                let mut seen = vec![false; prog.funcs.len()];
                // `busy` before `halt` here and `halt` before `busy` in `stop` (both SeqCst):
                // either this thread sees the halt or `stop` sees it busy.
                while let Ok(id) = rx.recv() {
                    sh.busy.store(true, Ordering::SeqCst);
                    if sh.halt.load(Ordering::SeqCst) {
                        break;
                    }
                    // Racy counters may report a function twice; elaboration code never
                    // runs again.
                    let first = seen
                        .get_mut(id.0 as usize)
                        .is_some_and(|s| !std::mem::replace(s, true));
                    let todo = prog.func(id).filter(|f| first && f.kind != FuncKind::Elab);
                    if let Some(f) = todo {
                        let t0 = std::time::Instant::now();
                        let r = match (too_big(f, max_size), &mut jit) {
                            (Some(why), _) => Err(why),
                            (None, Some(j)) => compile_one(j, &prog, id, f, opt),
                            (None, j @ None) => match new_jit(opt) {
                                Ok(n) => compile_one(j.insert(n), &prog, id, f, opt),
                                Err(e) => Err(format!("LLVM: {e} (a bug in yavs)")),
                            },
                        };
                        match r {
                            Ok(entry) => {
                                let _ = table[id.0 as usize].set(entry);
                                note(id, Ok(t0.elapsed()));
                            }
                            Err(why) => note(id, Err(why)),
                        }
                    }
                    sh.busy.store(false, Ordering::SeqCst);
                }
                *sh.jit.lock().unwrap_or_else(PoisonError::into_inner) = jit;
                sh.busy.store(false, Ordering::SeqCst);
            })
            .map_err(|e| Error(format!("cannot start the compile thread: {e}")))?;
        let tier = Arc::new(AutoTier {
            funcs,
            queue: Mutex::new(Some(tx)),
            shared,
            worker: Mutex::new(Some(worker)),
        });
        if !rt.set_tier(tier.clone()) || !rt.set_heat(|f| heat.threshold(f)) {
            tier.stop(true);
            return Err(Error(
                "the runtime has a tier already (a bug in yavs)".into(),
            ));
        }
        Ok(tier)
    }

    /// End tier-up: functions still queued stay interpreted, and compiled code stays usable.
    /// Call it when the simulation is over. Returns `true` once the compile thread has ended;
    /// `false` if it is compiling a function and `wait` is false: it then finishes that on its
    /// own, and the process must not run its exit handlers under the running LLVM
    /// ([`exit_now`]).
    pub fn stop(&self, wait: bool) -> bool {
        self.shared.halt.store(true, Ordering::SeqCst);
        drop(
            self.queue
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take(),
        );
        if !wait && self.shared.busy.load(Ordering::SeqCst) {
            return false;
        }
        let worker = self
            .worker
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(Err(p)) = worker.map(JoinHandle::join) {
            resume_unwind(p);
        }
        true
    }

    /// The functions compiled so far.
    pub fn compiled(&self) -> usize {
        self.funcs.iter().filter(|f| f.get().is_some()).count()
    }
}

impl Drop for AutoTier {
    fn drop(&mut self) {
        self.stop(true);
    }
}

/// End the process with `code` at once, without exit handlers (after `AutoTier::stop`
/// returned `false`: the static destructors of LLVM must not run under the compile thread).
/// Flushes the Rust and C standard streams first.
pub fn exit_now(code: u8) -> ! {
    use std::io::Write;
    let _ = std::io::stdout().flush();
    let _ = std::io::stderr().flush();
    unsafe extern "C" {
        fn fflush(f: *mut std::ffi::c_void) -> std::ffi::c_int;
        fn _exit(code: std::ffi::c_int) -> !;
    }
    // SAFETY: `fflush(NULL)` flushes every C stream; `_exit` ends the process.
    unsafe {
        fflush(std::ptr::null_mut());
        _exit(code.into())
    }
}

impl Tier for AutoTier {
    fn run(
        &self,
        func: FuncId,
        pc: u32,
        fr: &mut TierFrame,
        host: &mut dyn TierHost,
    ) -> Option<u32> {
        let &(f, nregs) = self.funcs.get(func.0 as usize)?.get()?;
        enter(f, nregs, pc, fr, host)
    }

    fn ready(&self, func: FuncId) -> bool {
        self.funcs
            .get(func.0 as usize)
            .is_some_and(|f| f.get().is_some())
    }

    fn hot(&self, func: FuncId) {
        if let Some(q) = &*self.queue.lock().unwrap_or_else(PoisonError::into_inner) {
            // The worker is gone only after `stop`: nothing to compile any more.
            let _ = q.send(func);
        }
    }
}
