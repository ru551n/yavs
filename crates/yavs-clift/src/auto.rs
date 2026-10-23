//! `--tier=clift-auto`: translate the functions where the interpreter spends its time, one at
//! a time on one background thread, while the simulation keeps running them in the interpreter.
//! The same scheme as `yavs-llvm`'s `AutoTier` (sampling through `Runtime::sampler`, entries
//! published through an atomic word, output independent of when a function finishes) with one
//! level and Cranelift's much lower compile cost in [`Heat`].

use std::panic::resume_unwind;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use yavs_interp::{Runtime, Tier, TierHost};
use yavs_ir::{FuncId, FuncKind};
use yavs_llvm::rt::{RunFn, enter, too_short};

use crate::lower::{Error, Jit};

/// Callees translated with a hot function: at most this many IR instructions each and in all.
const CALLEE_MAX: usize = 200;
const TREE_MAX: usize = 1500;

/// A function is hot once the sampler has seen the interpreter in it for `base` microseconds
/// plus `per_instr` nanoseconds per IR instruction, sampling every `period` microseconds;
/// `period` 0: every function the interpreter enters is translated at once (tests).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Heat {
    pub base: u32,
    pub per_instr: u32,
    pub period: u32,
}

impl Heat {
    /// Translating costs about 40 us per IR instruction (backtracking register allocation, no
    /// optimisation): a function is translated once the interpreter has spent about twice that
    /// in it, and 2 ms in any case, as the LLVM tier does (measured: STATUS.md "clift").
    pub const DEFAULT: Heat = Heat {
        base: 2_000,
        per_instr: 90_000,
        period: 1_000,
    };

    fn threshold(self, f: &yavs_ir::Func) -> u64 {
        let n = f.code.len() as u64;
        u64::from(self.base).saturating_add(u64::from(self.per_instr).saturating_mul(n) / 1000)
    }

    pub fn eager(self) -> bool {
        self.period == 0
    }
}

/// What happened to a hot function: translated (with the time it took) or why it stays
/// interpreted.
pub type Note = Box<dyn FnMut(FuncId, Result<Duration, String>) + Send>;

#[derive(Default)]
struct Entry {
    run: AtomicUsize,
    nregs: AtomicUsize,
}

impl Entry {
    fn get(&self) -> Option<(RunFn, usize)> {
        let run = self.run.load(Ordering::Acquire);
        // SAFETY: a nonzero `run` was stored by `set` from a `RunFn`.
        (run != 0).then(|| unsafe {
            (
                std::mem::transmute::<usize, RunFn>(run),
                self.nregs.load(Ordering::Relaxed),
            )
        })
    }

    fn set(&self, (run, nregs): (RunFn, usize)) {
        self.nregs.store(nregs, Ordering::Relaxed);
        self.run.store(run as usize, Ordering::Release);
    }
}

/// Hot functions translated on a background thread; see the module docs.
pub struct AutoTier {
    funcs: Arc<[Entry]>,
    queued: Box<[AtomicBool]>,
    queue: Mutex<Option<Sender<FuncId>>>,
    halt: Arc<AtomicBool>,
    worker: Mutex<Option<JoinHandle<Option<Jit>>>>,
    /// The JIT once the thread has ended: compiled code lives as long as the tier.
    jit: Mutex<Option<Jit>>,
}

impl AutoTier {
    pub fn start(
        rt: &Runtime,
        max_size: usize,
        heat: Heat,
        mut note: Note,
    ) -> Result<Arc<AutoTier>, Error> {
        let prog = rt.prog.clone();
        let funcs: Arc<[Entry]> = (0..prog.funcs.len()).map(|_| Entry::default()).collect();
        let (tx, rx) = channel::<FuncId>();
        let halt = Arc::new(AtomicBool::new(false));
        let (table, stop) = (funcs.clone(), halt.clone());
        let sample = rt.sampler();
        let native = rt.native_flags();
        let period = Duration::from_micros(if heat.eager() {
            1 << 20
        } else {
            heat.period.into()
        });
        let worker = std::thread::Builder::new()
            .name("yavs-clift-up".into())
            .spawn(move || {
                // Off the CPUs a pinned run restricts the simulation to (as the LLVM tier's thread).
                yavs_llvm::rt::off_simulation_cpus();
                let mut jit: Option<Jit> = None;
                let mut done = vec![false; prog.funcs.len()];
                let mut time = vec![0u64; prog.funcs.len()];
                let mut running = Vec::new();
                let mut hot: std::collections::VecDeque<(FuncId, bool)> = Default::default();
                loop {
                    match rx.recv_timeout(period) {
                        Ok(id) => hot.push_back((id, false)),
                        Err(RecvTimeoutError::Timeout) => {
                            running.clear();
                            sample(&mut running);
                            for &(id, compiled) in &running {
                                let (Some(t), Some(f)) =
                                    (time.get_mut(id.0 as usize), prog.func(id))
                                else {
                                    continue;
                                };
                                if compiled {
                                    continue;
                                }
                                let was = *t;
                                *t += u64::from(heat.period);
                                let at = heat.threshold(f);
                                if was < at && *t >= at {
                                    hot.push_back((id, true));
                                }
                            }
                        }
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                    while let Ok(id) = rx.try_recv() {
                        hot.push_back((id, false));
                    }
                    while let Some((id, sampled)) = hot.pop_front() {
                        // A hot function with the small functions it calls: a call from
                        // translated code into an interpreted function leaves the translated
                        // code every time (the LLVM tier's `Worker::tree`).
                        let mut ids = vec![id];
                        let mut size = prog.func(id).map_or(0, |f| f.code.len());
                        let mut i = 0;
                        while i < ids.len() && ids.len() < 16 {
                            let Some(f) = prog.func(ids[i]) else { break };
                            i += 1;
                            for inst in &f.code {
                                let (yavs_ir::Inst::Call { func, .. }
                                | yavs_ir::Inst::CallProt { func, .. }) = inst
                                else {
                                    continue;
                                };
                                let Some(g) = prog.func(*func) else { continue };
                                if !ids.contains(func)
                                    && !done[func.0 as usize]
                                    && g.kind != FuncKind::Elab
                                    && g.code.len() <= CALLEE_MAX
                                    && size + g.code.len() <= TREE_MAX
                                {
                                    size += g.code.len();
                                    ids.push(*func);
                                }
                            }
                        }
                        for (n, id) in ids.into_iter().enumerate() {
                            let sampled = sampled && n == 0;
                            if stop.load(Ordering::SeqCst) {
                                break;
                            }
                            let first = done.get_mut(id.0 as usize).is_some_and(|d| !std::mem::replace(d, true));
                            let Some(f) = prog.func(id).filter(|f| first && f.kind != FuncKind::Elab)
                            else {
                                continue;
                            };
                            let t0 = std::time::Instant::now();
                            let short = sampled.then(|| too_short(&prog, f)).flatten();
                            let r = if let Some(why) = short {
                                Err(why)
                            } else if f.code.len() > max_size {
                                Err(format!(
                                    "{} IR instructions, over the size cap of {max_size} (--tier-max-size)",
                                    f.code.len()
                                ))
                            } else {
                                if jit.is_none() {
                                    match Jit::new() {
                                        Ok(j) => jit = Some(j),
                                        Err(e) => {
                                            note(id, Err(format!("Cranelift: {e}")));
                                            continue;
                                        }
                                    }
                                }
                                let j = jit.as_mut().expect("made above");
                                j.define(&prog, id, f).and_then(|(cid, _)| {
                                    j.finalize()?;
                                    Ok((j.entry(cid), f.reg_ty.len()))
                                })
                            };
                            match r {
                                Ok(entry) => {
                                    table[id.0 as usize].set(entry);
                                    native.set(id);
                                    note(id, Ok(t0.elapsed()));
                                }
                                Err(why) => note(id, Err(why)),
                            }
                        }
                    }
                    if stop.load(Ordering::SeqCst) {
                        break;
                    }
                }
                jit
            })
            .map_err(|e| format!("cannot start the compile thread: {e}"))?;
        let tier = Arc::new(AutoTier {
            queued: (0..funcs.len()).map(|_| AtomicBool::new(false)).collect(),
            funcs,
            queue: Mutex::new(Some(tx)),
            halt,
            worker: Mutex::new(Some(worker)),
            jit: Mutex::new(None),
        });
        if !rt.set_tier(tier.clone()) {
            tier.stop();
            return Err("the runtime has a tier already (a bug in yavs)".into());
        }
        if heat.eager() {
            rt.set_eager();
        }
        Ok(tier)
    }

    /// End tier-up: functions still queued stay interpreted, compiled code stays usable. Waits
    /// for a function being translated (a Cranelift compile is short, and its module has no
    /// static destructors that must not run under it).
    pub fn stop(&self) {
        self.halt.store(true, Ordering::SeqCst);
        drop(
            self.queue
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take(),
        );
        let worker = self
            .worker
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        match worker.map(JoinHandle::join) {
            Some(Ok(j)) => *self.jit.lock().unwrap_or_else(PoisonError::into_inner) = j,
            Some(Err(p)) => resume_unwind(p),
            None => {}
        }
    }

    /// The functions translated so far.
    pub fn compiled(&self) -> usize {
        self.funcs.iter().filter(|f| f.get().is_some()).count()
    }
}

impl Drop for AutoTier {
    fn drop(&mut self) {
        self.stop();
    }
}

impl AutoTier {
    fn go(
        &self,
        func: FuncId,
        pc: u32,
        host: &mut dyn TierHost,
        state: Option<&mut Option<Box<yavs_interp::TierState>>>,
    ) -> Option<u32> {
        let (f, nregs) = self.funcs.get(func.0 as usize)?.get()?;
        let lookup = |id: FuncId| self.funcs.get(id.0 as usize)?.get();
        enter(f, nregs, pc, host, &lookup, std::ptr::null(), state)
    }
}

impl Tier for AutoTier {
    fn run(&self, func: FuncId, pc: u32, host: &mut dyn TierHost) -> Option<u32> {
        self.go(func, pc, host, None)
    }

    fn run_cached(
        &self,
        func: FuncId,
        pc: u32,
        host: &mut dyn TierHost,
        state: &mut Option<Box<yavs_interp::TierState>>,
    ) -> Option<u32> {
        self.go(func, pc, host, Some(state))
    }

    fn ready(&self, func: FuncId) -> bool {
        self.funcs
            .get(func.0 as usize)
            .is_some_and(|f| f.run.load(Ordering::Relaxed) != 0)
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
            let _ = q.send(func);
        }
    }
}
