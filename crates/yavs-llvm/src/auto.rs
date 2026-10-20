//! `--tier=auto`: compile the functions where the interpreter spends its time, one at a time
//! on one background thread, while the simulation keeps running them in the interpreter
//! (#125).
//!
//! The thread samples what the simulation threads interpret (`Runtime::sampler`) every
//! [`Heat::period`] microseconds and compiles a function once it was seen for about as long
//! as compiling it takes ([`Heat`]); the function is compiled in a module of its own and
//! published through an atomic entry. A function then seen running its compiled code 30
//! times as long is compiled again at O2 ([`Heat::o2_threshold`]) and its entry replaced (#432).
//! The interpreter asks the tier on entry and now and then at
//! jumps, so the swap-in happens there in any frame of that function: compiled code runs on
//! the interpreter frame's registers and enters at any pc (an instruction that is not an entry
//! point is left to the interpreter), so output is identical whenever compilation finishes
//! (rule 5). At the end of a run `stop(false)` does not wait for a function being compiled;
//! the CLI then ends the process with [`exit_now`].

use std::panic::resume_unwind;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use yavs_interp::{Runtime, Tier, TierHost};
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

    /// The time `f` must be seen running its first-tier code before it is compiled again at
    /// O2: `O2_AFTER` times `threshold`, about ten times its O2 compile time (O2 costs
    /// about seven times O0).
    pub fn o2_threshold(self, f: &yavs_ir::Func) -> u64 {
        self.threshold(f).saturating_mul(O2_AFTER)
    }

    /// Whether every function is compiled when the interpreter enters it.
    pub fn eager(self) -> bool {
        self.period == 0
    }
}

/// Below this many IR instructions a function without a loop stays interpreted (`too_short`).
const SHORT: usize = 64;

/// Why `f` stays interpreted although hot: shorter than `SHORT` IR instructions and without a
/// loop (`loops`), so each entry into compiled code from the interpreter (the frame view, the
/// entry switch, the way back) would cost about what interpreting its body costs: the small
/// helpers a testbench calls millions of times ran slower compiled (#432), and so did the
/// one-statement processes of the kernel design `fan`.
fn too_short(prog: &yavs_ir::Program, f: &yavs_ir::Func) -> Option<String> {
    (!loops(prog, f) && f.code.len() < SHORT).then(|| {
        format!(
            "{} IR instructions without a loop: entering compiled code would cost about as much",
            f.code.len()
        )
    })
}

/// Whether the blocks of `f` form a cycle, not counting a process's way through its
/// suspension (each activation runs its body once).
fn loops(prog: &yavs_ir::Program, f: &yavs_ir::Func) -> bool {
    use yavs_ir::Inst;
    let blocks: Vec<&[Inst]> = f
        .blocks
        .iter()
        .map(|(_, b)| f.code.get(b.start as usize..b.end as usize).unwrap_or(&[]))
        .collect();
    let waits: Vec<bool> = blocks
        .iter()
        .map(|c| f.kind == FuncKind::Process && c.iter().any(|i| matches!(i, Inst::Suspend { .. })))
        .collect();
    let succ = |b: usize| -> Vec<usize> {
        let mut to = Vec::new();
        for i in blocks[b] {
            match i {
                Inst::Br { to: t } => to.push(t.0 as usize),
                Inst::CondBr { t, f: e, .. } => to.extend([t.0 as usize, e.0 as usize]),
                Inst::Case { table, .. } => {
                    if let Some(t) = prog.cases.get(*table) {
                        to.push(t.default.0 as usize);
                        to.extend(t.arms.iter().map(|a| a.2.0 as usize));
                    }
                }
                _ => {}
            }
        }
        to.retain(|&t| t < blocks.len() && !waits[b] && !waits[t]);
        to
    };
    // Depth-first search for an edge back to a block on the current path.
    let mut state = vec![0u8; blocks.len()]; // 0 new, 1 on the path, 2 done
    for root in 0..blocks.len() {
        if state[root] != 0 {
            continue;
        }
        let mut stack = vec![(root, succ(root), 0)];
        state[root] = 1;
        while let Some((b, next, k)) = stack.last_mut() {
            match next.get(*k).copied() {
                Some(t) => {
                    *k += 1;
                    match state[t] {
                        1 => return true,
                        0 => {
                            state[t] = 1;
                            stack.push((t, succ(t), 0));
                        }
                        _ => {}
                    }
                }
                None => {
                    state[*b] = 2;
                    stack.pop();
                }
            }
        }
    }
    false
}

/// The functions `f` calls directly.
fn callees(f: &yavs_ir::Func) -> impl Iterator<Item = FuncId> + '_ {
    f.code.iter().filter_map(|i| match i {
        yavs_ir::Inst::Call { func, .. } => Some(*func),
        _ => None,
    })
}

/// The direct callers of every function, by callee.
fn callers_of(prog: &yavs_ir::Program) -> Vec<Vec<FuncId>> {
    let mut by = vec![Vec::new(); prog.funcs.len()];
    for (id, _) in prog.funcs.iter() {
        for g in prog.func(id).into_iter().flat_map(callees) {
            if let Some(v) = by.get_mut(g.0 as usize) {
                v.push(id);
            }
        }
    }
    by
}

/// See `Heat::o2_threshold`.
const O2_AFTER: u64 = 30;

/// What happened to a hot function: compiled at that level (with the time it took), or why
/// it stays interpreted (or at its first level).
pub type Note = Box<dyn FnMut(FuncId, OptLevel, Result<std::time::Duration, String>) + Send>;

/// The compiled code of a function: its entry (0 while it has none) and register count. The
/// O2 recompile replaces the entry; code once published stays valid as long as the tier.
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

/// Hot functions compiled on a background thread; see the module docs.
pub struct AutoTier {
    funcs: Arc<[Entry]>,
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
    /// The JITs once the thread has ended: compiled code lives as long as the tier.
    jit: Mutex<Vec<Jit>>,
}

impl AutoTier {
    /// Start the compile thread and make the tier the runtime's (`Runtime::set_tier`),
    /// compiling the functions `heat` finds hot at `opt`, and those that stay hot compiled
    /// again at O2 (`Heat::o2_threshold`; at once with an eager `heat`); functions over
    /// `max_size` IR instructions stay interpreted. `note` is called on the compile thread for
    /// every compile.
    pub fn start(
        rt: &Runtime,
        opt: OptLevel,
        max_size: usize,
        heat: Heat,
        mut note: Note,
    ) -> Result<Arc<AutoTier>, Error> {
        let prog = rt.prog.clone();
        let funcs: Arc<[Entry]> = (0..prog.funcs.len()).map(|_| Entry::default()).collect();
        let (tx, rx) = channel::<FuncId>();
        let shared = Arc::new(Shared::default());
        let (table, sh) = (funcs.clone(), shared.clone());
        let sample = rt.sampler();
        let native = rt.native_flags();
        let period = Duration::from_micros(if heat.eager() {
            1 << 20
        } else {
            heat.period.into()
        });
        let worker = std::thread::Builder::new()
            .name("yavs-tier-up".into())
            .spawn(move || {
                // Made with the first function compiled at their level: a run that never gets
                // hot pays nothing.
                let (mut jit, mut jit2): (Option<Jit>, Option<Jit>) = (None, None);
                // Per function: compiled at the first level, at O2.
                let mut done = vec![[false; 2]; prog.funcs.len()];
                // Microseconds each function was seen interpreted, and running compiled.
                let mut time = vec![[0u64; 2]; prog.funcs.len()];
                let mut running = Vec::new();
                // Functions to compile at the first level, in order, and whether the sampler
                // found them (a short one then stays interpreted; one eager `Tier::hot` asks for
                // is compiled); then those to compile again at O2.
                let mut hot: std::collections::VecDeque<(FuncId, bool)> = Default::default();
                let mut later: Vec<FuncId> = Vec::new();
                // Functions found hot but left interpreted for being short, and the direct
                // callers of each function (made when first needed).
                let mut short_hot = vec![false; prog.funcs.len()];
                let mut callers: Option<Vec<Vec<FuncId>>> = None;
                let upgrade = opt != OptLevel::O2;
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
                                let t = &mut t[usize::from(compiled)];
                                let was = *t;
                                *t += u64::from(heat.period);
                                let at = if compiled {
                                    heat.o2_threshold(f)
                                } else {
                                    heat.threshold(f)
                                };
                                if was < at && *t >= at {
                                    if !compiled {
                                        hot.push_back((id, true));
                                    } else if upgrade {
                                        later.push(id);
                                    }
                                }
                            }
                        }
                        Err(RecvTimeoutError::Disconnected) => break,
                    }
                    loop {
                        while let Ok(id) = rx.try_recv() {
                            hot.push_back((id, false));
                        }
                        let Some((id, again, sampled)) = hot
                            .pop_front()
                            .map(|(id, sampled)| (id, false, sampled))
                            .or_else(|| later.pop().map(|id| (id, true, false)))
                        else {
                            break;
                        };
                        // `busy` before `halt` here and `halt` before `busy` in `stop` (both
                        // SeqCst): either this thread sees the halt or `stop` sees it busy.
                        sh.busy.store(true, Ordering::SeqCst);
                        if sh.halt.load(Ordering::SeqCst) {
                            break;
                        }
                        // A process without a loop of its own (clocked RTL) at O2 at once: its
                        // first-level code ran no faster than the interpreter, and it runs for
                        // the whole simulation. One with loops (a testbench) gains at O0 already.
                        let rtl = prog
                            .func(id)
                            .is_some_and(|f| f.kind == FuncKind::Process && !loops(&prog, f));
                        let o2 = again || rtl && upgrade;
                        // Each level once; elaboration code never runs again.
                        let first = done.get_mut(id.0 as usize).is_some_and(|d| {
                            let first = !std::mem::replace(&mut d[usize::from(again)], true);
                            d[1] |= o2;
                            first
                        });
                        let todo = prog.func(id).filter(|f| first && f.kind != FuncKind::Elab);
                        if let Some(f) = todo {
                            let level = if o2 { OptLevel::O2 } else { opt };
                            let j = if o2 { &mut jit2 } else { &mut jit };
                            let t0 = std::time::Instant::now();
                            let short = (sampled && !again).then(|| too_short(&prog, f)).flatten();
                            let big = too_big(f, max_size);
                            let short_now = big.is_none() && short.is_some();
                            let r = match (big.or(short), j) {
                                (Some(why), _) => Err(why),
                                (None, Some(j)) => compile_one(j, &prog, id, f, level),
                                (None, j @ None) => match new_jit(level) {
                                    Ok(n) => compile_one(j.insert(n), &prog, id, f, level),
                                    Err(e) => Err(format!("LLVM: {e} (a bug in yavs)")),
                                },
                            };
                            match r {
                                Ok(entry) => {
                                    table[id.0 as usize].set(entry);
                                    native.set(id);
                                    note(id, level, Ok(t0.elapsed()));
                                    // The hot short functions it calls: called from
                                    // compiled code they cost no entry from the interpreter.
                                    for g in callees(f) {
                                        if short_hot.get(g.0 as usize).is_some_and(|s| *s)
                                            && table
                                                .get(g.0 as usize)
                                                .is_some_and(|e| e.get().is_none())
                                        {
                                            later.push(g);
                                        }
                                    }
                                    // Eager (tests): swap the O2 code in during the run too.
                                    if heat.eager() && upgrade && !o2 {
                                        later.push(id);
                                    }
                                }
                                Err(why) => {
                                    // A short function stays interpreted while no compiled
                                    // function calls it (each call from compiled code would
                                    // leave that code and enter it again).
                                    let mut called = false;
                                    if short_now {
                                        short_hot[id.0 as usize] = true;
                                        done[id.0 as usize][0] = false;
                                        let callers =
                                            callers.get_or_insert_with(|| callers_of(&prog));
                                        called = callers[id.0 as usize].iter().any(|c| {
                                            table
                                                .get(c.0 as usize)
                                                .is_some_and(|e| e.get().is_some())
                                        });
                                    }
                                    if called {
                                        later.push(id);
                                    } else {
                                        note(id, level, Err(why));
                                    }
                                }
                            }
                        }
                        sh.busy.store(false, Ordering::SeqCst);
                    }
                    if sh.halt.load(Ordering::SeqCst) {
                        break;
                    }
                }
                *sh.jit.lock().unwrap_or_else(PoisonError::into_inner) =
                    jit.into_iter().chain(jit2).collect();
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
    fn run(&self, func: FuncId, pc: u32, host: &mut dyn TierHost) -> Option<u32> {
        let (f, nregs) = self.funcs.get(func.0 as usize)?.get()?;
        let lookup = |id: FuncId| self.funcs.get(id.0 as usize)?.get();
        enter(f, nregs, pc, host, &lookup)
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
            // The worker is gone only after `stop`: nothing to compile any more.
            let _ = q.send(func);
        }
    }
}
