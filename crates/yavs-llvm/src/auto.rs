//! `--tier=auto`: compile the functions where the interpreter spends its time, one at a time
//! on one background thread, while the simulation keeps running them in the interpreter
//! (#125).
//!
//! The thread samples what the simulation threads interpret (`Runtime::sampler`) every
//! [`Heat::period`] microseconds and compiles a function once it was seen for about as long
//! as compiling it takes ([`Heat`]); the function is compiled in a module of its own and
//! published through a `OnceLock`. The interpreter asks the tier on entry and now and then at
//! jumps, so the swap-in happens there in any frame of that function: compiled code runs on
//! the interpreter frame's registers and enters at any pc (an instruction that is not an entry
//! point is left to the interpreter), so output is identical whenever compilation finishes
//! (rule 5). At the end of a run `stop(false)` does not wait for a function being compiled;
//! the CLI then ends the process with [`exit_now`].

use std::panic::resume_unwind;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex, OnceLock, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use yavs_interp::{Runtime, Tier, TierFrame, TierHost};
use yavs_ir::{FuncId, FuncKind};

use crate::lower::{RunFn, compile_one, enter, new_jit, too_big};
use crate::{Error, Jit, OptLevel};

/// When a function is hot: once the sampler has seen the interpreter in it for `base`
/// microseconds plus `per_instr` nanoseconds per IR instruction of the function (about the
/// time compiling it takes), sampling every `period` microseconds. `period` 0: no sampling,
/// every function the interpreter enters is compiled at once (tests).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Heat {
    pub base: u32,
    pub per_instr: u32,
    pub period: u32,
}

impl Heat {
    /// Compiling a function costs about 2 ms plus 40 us per IR instruction at O0 (codegen
    /// nearly all of it): a function is compiled once the interpreter has spent that long in
    /// it. One sample a millisecond costs the compile thread a few microseconds.
    pub const DEFAULT: Heat = Heat {
        base: 2_000,
        per_instr: 40_000,
        period: 1_000,
    };

    /// The time `f` must be seen for, in microseconds, saturating.
    pub fn threshold(self, f: &yavs_ir::Func) -> u64 {
        let n = f.code.len() as u64;
        u64::from(self.base).saturating_add(u64::from(self.per_instr).saturating_mul(n) / 1000)
    }

    /// Whether every function is compiled when the interpreter enters it.
    pub fn eager(self) -> bool {
        self.period == 0
    }
}

/// What happened to a hot function: compiled (with the time it took), or why it stays
/// interpreted.
pub type Note = Box<dyn FnMut(FuncId, Result<std::time::Duration, String>) + Send>;

/// Hot functions compiled on a background thread; see the module docs.
pub struct AutoTier {
    funcs: Arc<[OnceLock<(RunFn, usize)>]>,
    /// Functions `hot` sent to the thread already.
    queued: Box<[AtomicBool]>,
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
    /// Start the compile thread and make the tier the runtime's (`Runtime::set_tier`),
    /// compiling the functions `heat` finds hot; functions over `max_size` IR instructions stay
    /// interpreted. `note` is called on the compile thread for every hot function.
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
        let sample = rt.sampler();
        let period = Duration::from_micros(if heat.eager() {
            1 << 20
        } else {
            heat.period.into()
        });
        let worker = std::thread::Builder::new()
            .name("yavs-tier-up".into())
            .spawn(move || {
                // Made with the first hot function: a run that never gets hot pays nothing.
                let mut jit: Option<Jit> = None;
                let mut seen = vec![false; prog.funcs.len()];
                // Microseconds each function was seen in the interpreter.
                let mut time = vec![0u64; prog.funcs.len()];
                let mut running = Vec::new();
                let mut hot = Vec::new();
                loop {
                    match rx.recv_timeout(period) {
                        Ok(id) => hot.push(id),
                        Err(RecvTimeoutError::Timeout) => {
                            running.clear();
                            sample(&mut running);
                            for &id in &running {
                                let (Some(t), Some(f)) =
                                    (time.get_mut(id.0 as usize), prog.func(id))
                                else {
                                    continue;
                                };
                                let was = *t;
                                *t += u64::from(heat.period);
                                let at = heat.threshold(f);
                                if was < at && *t >= at {
                                    hot.push(id);
                                }
                            }
                        }
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                    for id in hot.drain(..) {
                        // `busy` before `halt` here and `halt` before `busy` in `stop` (both
                        // SeqCst): either this thread sees the halt or `stop` sees it busy.
                        sh.busy.store(true, Ordering::SeqCst);
                        if sh.halt.load(Ordering::SeqCst) {
                            break;
                        }
                        // Elaboration code never runs again.
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
                    if sh.halt.load(Ordering::SeqCst) {
                        break;
                    }
                }
                *sh.jit.lock().unwrap_or_else(PoisonError::into_inner) = jit;
                sh.busy.store(false, Ordering::SeqCst);
            })
            .map_err(|e| Error(format!("cannot start the compile thread: {e}")))?;
        let tier = Arc::new(AutoTier {
            queued: (0..funcs.len()).map(|_| AtomicBool::new(false)).collect(),
            funcs,
            queue: Mutex::new(Some(tx)),
            shared,
            worker: Mutex::new(Some(worker)),
        });
        if !rt.set_tier(tier.clone()) {
            tier.stop(true);
            return Err(Error(
                "the runtime has a tier already (a bug in yavs)".into(),
            ));
        }
        if heat.eager() {
            rt.set_eager();
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
        let lookup = |id: FuncId| self.funcs.get(id.0 as usize)?.get().copied();
        enter(f, nregs, pc, fr, host, &lookup)
    }

    fn ready(&self, func: FuncId) -> bool {
        self.funcs
            .get(func.0 as usize)
            .is_some_and(|f| f.get().is_some())
    }

    fn hot(&self, func: FuncId) {
        if self
            .queued
            .get(func.0 as usize)
            .is_none_or(|q| q.swap(true, Ordering::Relaxed))
        {
            return;
        }
        if let Some(q) = &*self.queue.lock().unwrap_or_else(PoisonError::into_inner) {
            // The worker is gone only after `stop`: nothing to compile any more.
            let _ = q.send(func);
        }
    }
}
