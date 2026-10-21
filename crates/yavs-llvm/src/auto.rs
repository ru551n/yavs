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
    /// The compile thread may spend at most this percent of the time the simulation was seen
    /// running (0: no limit). With a limit a function is compiled only when it was seen
    /// interpreted for `PAYBACK` times as long as compiling it takes.
    pub budget: u32,
}

impl Heat {
    /// Compiling a function costs about 2 ms plus 40 us per IR instruction at O0 (codegen
    /// nearly all of it): a function is compiled once the interpreter has spent that long in
    /// it. One sample a millisecond costs the compile thread a few microseconds.
    pub const DEFAULT: Heat = Heat {
        base: 2_000,
        per_instr: 40_000,
        period: 1_000,
        budget: 2,
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
/// jump back, so each entry into compiled code from the interpreter (the frame view, the
/// entry switch, the way back) would cost about what interpreting its body costs: the small
/// helpers a testbench calls millions of times ran slower compiled (#432).
fn too_short(prog: &yavs_ir::Program, f: &yavs_ir::Func) -> Option<String> {
    (f.code.len() < SHORT && !loops(prog, f)).then(|| {
        format!(
            "{} IR instructions without a loop: entering compiled code would cost about as much",
            f.code.len()
        )
    })
}

/// The backward jumps of `f` as ranges of instructions (a jump back to a block start and the jump).
fn back_edges(prog: &yavs_ir::Program, f: &yavs_ir::Func) -> Vec<std::ops::Range<usize>> {
    use yavs_ir::Inst;
    let start = |b: yavs_ir::BlockId| f.blocks.get(b).map_or(0, |b| b.start as usize);
    let mut out = Vec::new();
    for (pc, i) in f.code.iter().enumerate() {
        let targets: Vec<usize> = match i {
            Inst::Br { to } => vec![start(*to)],
            Inst::CondBr { t, f: e, .. } => vec![start(*t), start(*e)],
            Inst::Case { table, .. } => prog.cases.get(*table).map_or(Vec::new(), |t| {
                std::iter::once(start(t.default))
                    .chain(t.arms.iter().map(|a| start(a.2)))
                    .collect()
            }),
            _ => Vec::new(),
        };
        out.extend(targets.into_iter().filter(|&t| t <= pc).map(|t| t..pc + 1));
    }
    out
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

/// Whether the loops of `f` are compute: at most one instruction in twenty in them is one the
/// compiled code leaves to the interpreter (a call other than a native-frame one, a native, a
/// report). Such a function runs
/// about 1.3 times faster compiled (the interpreter spends its time dispatching them), so it
/// is worth compiling earlier than testbench code, whose time goes to what the interpreter's
/// own functions do (#432).
fn compute(prog: &yavs_ir::Program, f: &yavs_ir::Func) -> bool {
    let mut inside = vec![false; f.code.len()];
    if loops(prog, f) {
        for r in back_edges(prog, f) {
            inside[r].fill(true);
        }
    } else {
        return false;
    }
    let total = inside.iter().filter(|&&b| b).count();
    // A call of a function that runs on native frames is a direct call.
    let nc = crate::lower::native_set(
        prog,
        f.code.iter().filter_map(|i| match i {
            yavs_ir::Inst::Call { func, .. } => Some(*func),
            _ => None,
        }),
    );
    let stepped = f
        .code
        .iter()
        .zip(&inside)
        .filter(|(i, b)| {
            **b && !crate::lower::is_inline(prog, f, i)
                && !matches!(i, yavs_ir::Inst::Call { func, .. } if nc.contains(func))
        })
        .count();
    stepped * 20 <= total
}

/// A compute loop gets this many times the budget: it runs several times faster compiled (the
/// interpreter's time goes to dispatch), so a long-running one repays a compile of a size the
/// plain budget would only allow after minutes, and the compile is on another thread (#432).
const COMPUTE_BUDGET: u64 = 16;

/// Whether compiling for `cost` more microseconds keeps the compile time spent within `budget`
/// percent (`COMPUTE_BUDGET` times that for compute) of the `total` microseconds the sampler saw
/// running; budget 0 is no limit.
fn within_budget(budget: u32, compute: bool, spent: u64, cost: u64, total: u64) -> bool {
    let percent = u64::from(budget) * if compute { COMPUTE_BUDGET } else { 1 };
    budget == 0 || (spent + cost) * 100 <= total * percent
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
    ntab: Arc<[AtomicUsize]>,
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

/// Microseconds of compile time per IR instruction to start with (O2, one function in a
/// module of its own); the thread replaces it by what it measures.
const US_PER_INSTR: f64 = 400.0;

/// A function must have been seen interpreted this many times as long as compiling it takes:
/// its remaining time is expected to be about its past time and compiled code saves about a
/// quarter of it.
const PAYBACK: u64 = 4;

/// Callees compiled with a hot function (`Worker::tree`): at most this many IR instructions
/// each and in all.
const CALLEE_MAX: usize = 200;
const TREE_MAX: usize = 1500;

/// The compile thread's state.
struct Worker {
    prog: Arc<yavs_ir::Program>,
    table: Arc<[Entry]>,
    /// Native-frame code per function (`Mem::ntab`), published before the entry.
    ntab: Arc<[AtomicUsize]>,
    native: Arc<yavs_interp::NativeFlags>,
    heat: Heat,
    opt: OptLevel,
    max_size: usize,
    /// Made with the first function compiled at their level: a run that never gets hot pays
    /// nothing.
    jit: Option<Jit>,
    jit2: Option<Jit>,
    /// Per function: compiled at the first level, at O2.
    done: Vec<[bool; 2]>,
    /// Microseconds each function was seen interpreted, and running compiled.
    time: Vec<[u64; 2]>,
    /// Microseconds the simulation threads were seen running, and the compile thread spent
    /// compiling.
    total: u64,
    spent: u64,
    us_per_instr: f64,
    note: Note,
    /// Per short function that runs on native frames, the interpreted caller its time counts
    /// for (`charge`); emptied when a function is first seen.
    leaf: std::collections::HashMap<FuncId, Option<FuncId>>,
    seen: usize,
}

impl Worker {
    /// The function the sampled interpreted time of `id` counts for: a short function that
    /// runs on native frames counts for the interpreted caller seen longest that calls it
    /// (compiled, that caller calls it directly, so compiling the callee alone gains nothing:
    /// docs/decisions.md "Compiled frames"); any other function for itself.
    fn charge(&mut self, id: FuncId) -> FuncId {
        let seen = self.time.iter().filter(|t| t[0] > 0).count();
        if seen != self.seen {
            self.seen = seen;
            self.leaf.clear();
        }
        let (prog, time, table) = (&self.prog, &self.time, &self.table);
        let to = *self.leaf.entry(id).or_insert_with(|| {
            let f = prog.func(id)?;
            if too_short(prog, f).is_none() || !crate::lower::native_set(prog, [id]).contains(&id) {
                return None;
            }
            (0..time.len())
                .filter(|&k| k != id.0 as usize && time[k][0] > 0)
                .filter(|&k| table[k].run.load(Ordering::Relaxed) == 0)
                .filter(|&k| {
                    prog.func(FuncId(k as u32)).is_some_and(|g| {
                        g.code
                            .iter()
                            .any(|i| matches!(i, yavs_ir::Inst::Call { func, .. } if *func == id))
                    })
                })
                .max_by_key(|&k| time[k][0])
                .map(|k| FuncId(k as u32))
        });
        to.filter(|c| self.table[c.0 as usize].run.load(Ordering::Relaxed) == 0)
            .unwrap_or(id)
    }

    /// The microseconds of compile time to expect for `f`.
    fn cost(&self, f: &yavs_ir::Func) -> u64 {
        (self.us_per_instr * f.code.len() as f64) as u64
    }

    /// The time `id` must be seen at its level before it is compiled (again).
    fn threshold(&self, id: FuncId, compiled: bool) -> u64 {
        let Some(f) = self.prog.func(id) else {
            return u64::MAX;
        };
        if compiled {
            return self.heat.o2_threshold(f);
        }
        let at = self.heat.threshold(f);
        if self.heat.budget == 0 {
            at
        } else {
            at.max(PAYBACK * self.cost(f))
        }
    }

    /// `root` and the functions it calls that compile with it: a call from compiled code to
    /// one that is not leaves the compiled code (`TIER_UNWIND`).
    fn tree(&self, root: FuncId) -> Vec<FuncId> {
        let mut ids = vec![root];
        let mut size = self.prog.func(root).map_or(0, |f| f.code.len());
        let mut i = 0;
        while i < ids.len() && ids.len() < 16 {
            let Some(f) = self.prog.func(ids[i]) else {
                break;
            };
            i += 1;
            for inst in &f.code {
                let (yavs_ir::Inst::Call { func, .. } | yavs_ir::Inst::CallProt { func, .. }) =
                    inst
                else {
                    continue;
                };
                let Some(g) = self.prog.func(*func) else {
                    continue;
                };
                if !ids.contains(func)
                    && !self.done[func.0 as usize][0]
                    && g.kind != FuncKind::Elab
                    && g.code.len() <= CALLEE_MAX
                    && size + g.code.len() <= TREE_MAX
                {
                    size += g.code.len();
                    ids.push(*func);
                }
            }
        }
        ids
    }

    /// The function to compile next and what goes with it, if the budget has room for it.
    fn pick(&mut self, cands: &mut Vec<FuncId>) -> Option<Vec<FuncId>> {
        // The hottest relative to its cost first.
        let best = (0..cands.len()).max_by_key(|&i| {
            let id = cands[i];
            let f = self.prog.func(id);
            self.time[id.0 as usize][0] * 1000 / f.map_or(1, |f| self.cost(f).max(1))
        })?;
        let id = cands[best];
        let ids = self.tree(id);
        let cost: u64 = ids
            .iter()
            .filter_map(|&i| self.prog.func(i))
            .map(|f| self.cost(f))
            .sum();
        let compute = self.prog.func(id).is_some_and(|f| compute(&self.prog, f));
        if !within_budget(self.heat.budget, compute, self.spent, cost, self.total) {
            return None;
        }
        cands.swap_remove(best);
        Some(ids)
    }

    /// Compile `id` at the first level, or at O2 when `o2`; whether it was compiled.
    fn compile(&mut self, id: FuncId, o2: bool, sampled: bool) -> bool {
        // Each level once; elaboration code never runs again.
        let first = self
            .done
            .get_mut(id.0 as usize)
            .is_some_and(|d| !std::mem::replace(&mut d[usize::from(o2)], true));
        let Some(f) = self
            .prog
            .func(id)
            .filter(|f| first && f.kind != FuncKind::Elab)
        else {
            return false;
        };
        let level = if o2 { OptLevel::O2 } else { self.opt };
        let j = if o2 { &mut self.jit2 } else { &mut self.jit };
        let t0 = std::time::Instant::now();
        let short = (sampled && !o2).then(|| too_short(&self.prog, f)).flatten();
        let short_only = short.is_some() && too_big(f, self.max_size).is_none();
        let r = match (too_big(f, self.max_size).or(short), j) {
            (Some(why), _) => Err(why),
            (None, Some(j)) => compile_one(j, &self.prog, id, f, level),
            (None, j @ None) => match new_jit(level) {
                Ok(n) => compile_one(j.insert(n), &self.prog, id, f, level),
                Err(e) => Err(format!("LLVM: {e} (a bug in yavs)")),
            },
        };
        match r {
            Ok(entry) => {
                let took = t0.elapsed();
                self.spent += u64::try_from(took.as_micros()).unwrap_or(u64::MAX);
                if level == OptLevel::O2 {
                    let per = took.as_micros() as f64 / f.code.len().max(1) as f64;
                    self.us_per_instr = (self.us_per_instr + per) / 2.0;
                }
                let (run, nregs, native) = entry;
                if native != 0 {
                    self.ntab[id.0 as usize].store(native, Ordering::Release);
                }
                self.table[id.0 as usize].set((run, nregs));
                self.native.set(id);
                (self.note)(id, level, Ok(took));
                true
            }
            Err(why) => {
                // A short function stays interpreted as long as nothing compiled calls it;
                // it compiles with a caller (`tree`).
                if short_only {
                    self.done[id.0 as usize][0] = false;
                }
                (self.note)(id, level, Err(why));
                false
            }
        }
    }
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
        note: Note,
    ) -> Result<Arc<AutoTier>, Error> {
        crate::lazy::ensure()?;
        let prog = rt.prog.clone();
        let funcs: Arc<[Entry]> = (0..prog.funcs.len()).map(|_| Entry::default()).collect();
        let (tx, rx) = channel::<FuncId>();
        let shared = Arc::new(Shared::default());
        let ntab: Arc<[AtomicUsize]> = (0..prog.funcs.len()).map(|_| AtomicUsize::new(0)).collect();
        let (table, sh) = (funcs.clone(), shared.clone());
        let sample = rt.sampler();
        let native = rt.native_flags();
        let period = Duration::from_micros(if heat.eager() {
            1 << 20
        } else {
            heat.period.into()
        });
        let mut w = Worker {
            time: vec![[0; 2]; prog.funcs.len()],
            done: vec![[false; 2]; prog.funcs.len()],
            prog,
            table,
            ntab: ntab.clone(),
            native,
            heat,
            opt,
            max_size,
            jit: None,
            jit2: None,
            total: 0,
            spent: 0,
            us_per_instr: US_PER_INSTR,
            note,
            leaf: Default::default(),
            seen: 0,
        };
        let worker = std::thread::Builder::new()
            .name("yavs-tier-up".into())
            .spawn(move || {
                let mut running = Vec::new();
                let mut last = std::time::Instant::now();
                // Functions `Tier::hot` asked for (eager), those the sampler found hot, those to
                // compile again at O2.
                let mut asked: std::collections::VecDeque<FuncId> = Default::default();
                let mut cands: Vec<FuncId> = Vec::new();
                let mut later: Vec<FuncId> = Vec::new();
                let upgrade = opt != OptLevel::O2;
                loop {
                    match rx.recv_timeout(period) {
                        Ok(id) => asked.push_back(id),
                        Err(RecvTimeoutError::Timeout) => {
                            // The time since the last look, whatever the compile thread did.
                            let now = std::time::Instant::now();
                            let dt = u64::try_from((now - last).as_micros()).unwrap_or(u64::MAX);
                            last = now;
                            running.clear();
                            sample(&mut running);
                            for &(id, compiled) in &running {
                                let id = if compiled { id } else { w.charge(id) };
                                let Some(t) = w.time.get_mut(id.0 as usize) else {
                                    continue;
                                };
                                let t = &mut t[usize::from(compiled)];
                                let was = *t;
                                *t += dt;
                                w.total += dt;
                                let (now, at) = (*t, w.threshold(id, compiled));
                                if was < at && now >= at {
                                    if !compiled {
                                        cands.push(id);
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
                            asked.push_back(id);
                        }
                        let Some((ids, o2, sampled)) = asked
                            .pop_front()
                            .map(|id| (vec![id], false, false))
                            .or_else(|| later.pop().map(|id| (vec![id], true, false)))
                            .or_else(|| w.pick(&mut cands).map(|ids| (ids, false, true)))
                        else {
                            break;
                        };
                        // `busy` before `halt` here and `halt` before `busy` in `stop` (both
                        // SeqCst): either this thread sees the halt or `stop` sees it busy.
                        sh.busy.store(true, Ordering::SeqCst);
                        if sh.halt.load(Ordering::SeqCst) {
                            break;
                        }
                        for (i, id) in ids.into_iter().enumerate() {
                            let done = w.compile(id, o2, sampled && i == 0);
                            // Its callees compile for its sake only.
                            if !done && i == 0 {
                                break;
                            }
                            // Eager (tests): swap the O2 code in during the run too.
                            if done && heat.eager() && upgrade && !o2 {
                                later.push(id);
                            }
                        }
                        sh.busy.store(false, Ordering::SeqCst);
                    }
                    if sh.halt.load(Ordering::SeqCst) {
                        break;
                    }
                }
                *sh.jit.lock().unwrap_or_else(PoisonError::into_inner) =
                    w.jit.into_iter().chain(w.jit2).collect();
                sh.busy.store(false, Ordering::SeqCst);
            })
            .map_err(|e| Error(format!("cannot start the compile thread: {e}")))?;
        let tier = Arc::new(AutoTier {
            queued: (0..funcs.len()).map(|_| AtomicBool::new(false)).collect(),
            funcs,
            ntab,
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
        // `AtomicUsize` has the layout of `usize`: compiled code loads the entries atomically.
        enter(f, nregs, pc, host, &lookup, self.ntab.as_ptr().cast())
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

#[cfg(test)]
mod tests {
    use super::within_budget;

    /// A process that interprets one loop for seconds without ever suspending: the sampler sees
    /// all of it, and a compute loop of a few hundred instructions (about 180 ms to compile)
    /// is compiled once the payback rule allows (about half a second); other code still waits for the plain
    /// 2 percent, and nothing is compiled before it could repay.
    #[test]
    fn long_compute_loop_fits_the_budget() {
        let cost = 177_000;
        assert!(!within_budget(2, true, 0, cost, 500_000));
        assert!(within_budget(2, true, 0, cost, 600_000));
        assert!(!within_budget(2, false, 0, cost, 1_500_000));
        assert!(within_budget(2, false, 0, cost, 9_000_000));
        assert!(within_budget(0, false, 0, cost, 0));
    }
}
