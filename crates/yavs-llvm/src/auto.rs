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
//!
//! After the swap of a clocked process the thread measures what the compiled code did for the run
//! (#596, [`Worker::trial`]): process runs per CPU second with it and with the interpreter running
//! the process, a few windows in turn, and takes it back for good when the run did not progress
//! faster. The static rules before the swap (size, instances, share, callbacks, search loops) are
//! the first guess; the measure corrects what they cannot see.

use std::panic::resume_unwind;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use yavs_interp::{Runtime, Tier, TierHost};
use yavs_ir::{FuncId, FuncKind};

use crate::lower::{Part, compile_one, native_set, new_jit, too_big};
use crate::rt::{RunFn, call, enter};
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
/// helpers a testbench calls millions of times ran slower compiled (#432). Not a function the
/// interpreter calls on a native frame (`called_natively`): that call costs less than its own.
fn too_short(prog: &yavs_ir::Program, f: &yavs_ir::Func, id: FuncId) -> Option<String> {
    (f.code.len() < SHORT && !loops(prog, f) && !called_natively(prog, f, id)).then(|| {
        format!(
            "{} IR instructions without a loop: entering compiled code would cost about as much",
            f.code.len()
        )
    })
}

/// Whether the interpreter calls `f` (`id`) on a native frame once compiled (`Tier::call`): it
/// runs on native frames and has no static link.
fn called_natively(prog: &yavs_ir::Program, f: &yavs_ir::Func, id: FuncId) -> bool {
    f.parent.is_none()
        && matches!(f.kind, FuncKind::Function | FuncKind::Procedure)
        && native_set(prog, [id]).contains(&id)
}

/// Why `f` stays interpreted although hot: every loop of it only reads one array position per
/// iteration and compares (a search for an element or an end), the loop the interpreter runs as
/// one fused op (`pre::SCAN`, about 1.5 cycles an element). Compiled it takes 3 to 4 cycles an
/// element (the position check and the object lookup of every access), so the string
/// searches of a logging library ran 2 times slower compiled (#573). ponytail: the shape of
/// the IR loop, not the interpreter's own test; a function with other work in a loop is
/// compiled.
fn fused_search(prog: &yavs_ir::Program, f: &yavs_ir::Func) -> Option<String> {
    use yavs_ir::{Inst, IntOp};
    let mut inside = vec![false; f.code.len()];
    for r in back_edges(prog, f) {
        inside[r].fill(true);
    }
    let (mut pos, mut load, mut index, mut step, mut cmp) = (0, 0, 0, 0, 0);
    for (i, _) in f.code.iter().zip(&inside).filter(|(_, b)| **b) {
        match i {
            Inst::Pos { .. } => pos += 1,
            Inst::Load { .. } => load += 1,
            Inst::Index { .. } => index += 1,
            Inst::Bin {
                op: IntOp::Add | IntOp::Sub,
                ..
            } => step += 1,
            Inst::Cmp { .. } => cmp += 1,
            Inst::CondBr { .. } | Inst::Br { .. } | Inst::Const { .. } | Inst::Move { .. } => {}
            _ => return None,
        }
    }
    (pos > 0 && pos == load && pos == index && step <= pos && cmp <= 2 * pos)
        .then(|| "its loops are searches the interpreter runs as one op".into())
}

/// The instructions of the loops of `f`, as ranges of the blocks of each natural loop: per back
/// edge of a depth-first walk from the entry, the blocks that reach its latch without passing its
/// header. A process's way back from its suspension is no loop (as in `loops`). A jump to a lower
/// pc is not enough: the optimiser appends blocks (preheaders, inlined bodies) after the code.
fn back_edges(prog: &yavs_ir::Program, f: &yavs_ir::Func) -> Vec<std::ops::Range<usize>> {
    use yavs_ir::Inst;
    let blocks: Vec<std::ops::Range<usize>> = f
        .blocks
        .iter()
        .map(|(_, b)| b.start as usize..(b.end as usize).min(f.code.len()))
        .collect();
    let nb = blocks.len();
    let waits: Vec<bool> = blocks
        .iter()
        .map(|r| {
            f.kind == FuncKind::Process
                && f.code[r.clone()]
                    .iter()
                    .any(|i| matches!(i, Inst::Suspend { .. }))
        })
        .collect();
    let succ = |b: usize| -> Vec<usize> {
        if waits[b] {
            return Vec::new();
        }
        let mut to = Vec::new();
        for i in &f.code[blocks[b].clone()] {
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
        to.retain(|&t| t < nb);
        to
    };
    let mut preds = vec![Vec::new(); nb];
    for b in 0..nb {
        for t in succ(b) {
            preds[t].push(b);
        }
    }
    let mut edges = Vec::new();
    let mut state = vec![0u8; nb]; // 0 new, 1 on the path, 2 done
    if nb > 0 {
        let mut stack = vec![(0usize, succ(0), 0usize)];
        state[0] = 1;
        while let Some((b, next, k)) = stack.last_mut() {
            match next.get(*k).copied() {
                Some(t) => {
                    *k += 1;
                    match state[t] {
                        1 => edges.push((*b, t)),
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
    let mut out = Vec::new();
    let mut seen = vec![usize::MAX; nb];
    for (k, (latch, h)) in edges.into_iter().enumerate() {
        seen[h] = k;
        let mut work = vec![latch];
        while let Some(b) = work.pop() {
            if std::mem::replace(&mut seen[b], k) == k && b != latch {
                continue;
            }
            out.push(blocks[b].clone());
            if b != h {
                work.extend(preds[b].iter().copied().filter(|&p| seen[p] != k));
            }
        }
        out.push(blocks[h].clone());
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
    // Blocks the optimiser left unreachable (a self-jump) are no loop of the process.
    let mut reach = vec![false; blocks.len()];
    let mut work = vec![0usize];
    while let Some(b) = work.pop() {
        if b >= blocks.len() || std::mem::replace(&mut reach[b], true) {
            continue;
        }
        for i in blocks[b] {
            match i {
                Inst::Br { to } => work.push(to.0 as usize),
                Inst::CondBr { t, f: e, .. } => work.extend([t.0 as usize, e.0 as usize]),
                Inst::Case { table, .. } => {
                    if let Some(t) = prog.cases.get(*table) {
                        work.push(t.default.0 as usize);
                        work.extend(t.arms.iter().map(|a| a.2.0 as usize));
                    }
                }
                _ => {}
            }
        }
    }
    // Depth-first search for an edge back to a block on the current path.
    let mut state = vec![0u8; blocks.len()]; // 0 new, 1 on the path, 2 done
    for root in 0..blocks.len() {
        if state[root] != 0 || !reach[root] {
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
pub(crate) fn compute(prog: &yavs_ir::Program, f: &yavs_ir::Func) -> bool {
    let mut inside = vec![false; f.code.len()];
    if loops(prog, f) {
        for r in back_edges(prog, f) {
            inside[r].fill(true);
        }
    } else {
        return false;
    }
    let total = inside.iter().filter(|&&b| b).count();
    let nc = calls_native(prog, f);
    let stepped = (0..f.code.len())
        .filter(|&pc| inside[pc] && crate::lower::leaves(prog, f, pc, &nc))
        .count();
    stepped * 20 <= total
}

/// The functions `f` calls on every run of it: a call that every path from the entry to a `Ret` (in
/// a process, to a `Suspend`) passes, so as often as `f` runs. The calls on error paths and in
/// branches are not. A callee that short never shows in the sampler while its caller is
/// interpreted, yet each call of it from compiled code that finds it interpreted costs a step out
/// of the native frame (`Worker::hot`).
fn must_calls(prog: &yavs_ir::Program, f: &yavs_ir::Func) -> Vec<FuncId> {
    use yavs_ir::Inst;
    let n = f.code.len();
    let exit = |pc: usize| matches!(f.code[pc], Inst::Ret { .. } | Inst::Suspend { .. });
    let callee = |pc: usize| match &f.code[pc] {
        Inst::Call { func, .. } | Inst::CallProt { func, .. } => Some(*func),
        _ => None,
    };
    // Whether an exit is reached from the entry without a call of `skip`.
    let escapes = |skip: Option<FuncId>| -> bool {
        let mut seen = vec![false; n];
        let mut work = vec![0usize];
        while let Some(pc) = work.pop() {
            if pc >= n
                || skip.is_some() && callee(pc) == skip
                || std::mem::replace(&mut seen[pc], true)
            {
                continue;
            }
            if exit(pc) {
                return true;
            }
            if let Some(s) = crate::regs::succs(prog, f, pc, &f.code[pc]) {
                work.extend(s.into_iter().map(|t| t as usize));
            }
        }
        false
    };
    let mut out = Vec::new();
    // ponytail: one search per distinct callee; a function over `HOT_ROOT_MAX` instructions
    // brings none along, a dominator tree if one needs it.
    if n > 0 && n <= HOT_ROOT_MAX && escapes(None) {
        for c in (0..n).filter_map(callee) {
            if !out.contains(&c) && !escapes(Some(c)) {
                out.push(c);
            }
        }
    }
    out
}

/// The functions `f` calls that run on native frames once compiled (a call of one is direct).
fn calls_native(prog: &yavs_ir::Program, f: &yavs_ir::Func) -> std::collections::HashSet<FuncId> {
    native_set(
        prog,
        f.code.iter().filter_map(|i| match i {
            yavs_ir::Inst::Call { func, .. } | yavs_ir::Inst::CallProt { func, .. } => Some(*func),
            _ => None,
        }),
    )
}

/// The instructions an activation of process `f` runs once it has suspended (those reached from
/// the instruction after each `Suspend`, up to the next one), and how many of them leave the
/// compiled code for a callback into the interpreter's functions: every instruction it does not
/// run inline (`is_inline`: steps, natives, signal operations), and each `Stmt` where
/// temporaries are recorded as roots (it is stepped then).
fn callbacks(prog: &yavs_ir::Program, f: &yavs_ir::Func) -> (usize, usize) {
    use yavs_ir::Inst;
    let n = f.code.len();
    let mut seen = vec![false; n];
    let mut work: Vec<usize> = (0..n)
        .filter(|&pc| matches!(f.code[pc], Inst::Suspend { .. }))
        .map(|pc| pc + 1)
        .collect();
    while let Some(pc) = work.pop() {
        if pc >= n || std::mem::replace(&mut seen[pc], true) {
            continue;
        }
        let i = &f.code[pc];
        if matches!(i, Inst::Suspend { .. }) {
            continue;
        }
        if let Some(s) = crate::regs::succs(prog, f, pc, i) {
            work.extend(s.into_iter().map(|t| t as usize));
        }
    }
    let roots = (0..n).any(|pc| seen[pc] && matches!(f.code[pc], Inst::TempRoot { .. }));
    let nc = calls_native(prog, f);
    let out = (0..n)
        .filter(|&pc| {
            seen[pc]
                && (crate::lower::leaves(prog, f, pc, &nc)
                    || roots && matches!(f.code[pc], Inst::Stmt))
        })
        .count();
    (out, seen.iter().filter(|&&b| b).count())
}

/// Why hot process `f` stays interpreted: more than one in `CALLBACKS` of the instructions an
/// activation runs leave the compiled code (`callbacks`), and each way out and back costs more
/// than interpreting the instruction, so the compiled process ran slower than the interpreted
/// one (#507). ponytail: a static count over every path of the body, not weighted by what runs;
/// a measured comparison of both forms if a process near the line needs it.
fn too_many_callbacks(prog: &yavs_ir::Program, f: &yavs_ir::Func, id: FuncId) -> Option<String> {
    if f.kind == FuncKind::Process {
        let (out, total) = callbacks(prog, f);
        return (out * CALLBACKS > total).then(|| {
            format!(
                "{out} of the {total} IR instructions an activation runs leave the compiled \
                 code: it would run slower than interpreted"
            )
        });
    }
    // A function on frame-mode code: the same rule over its instructions (a call of a function
    // that runs on native frames is a direct call).
    if called_natively(prog, f, id) {
        return None;
    }
    let nc = calls_native(prog, f);
    let out = (0..f.code.len())
        .filter(|&pc| crate::lower::leaves(prog, f, pc, &nc))
        .count();
    (out * CALLBACKS > f.code.len()).then(|| {
        format!(
            "{out} of its {} IR instructions leave the compiled code: it would run slower than \
             interpreted",
            f.code.len()
        )
    })
}

/// A process stays interpreted when more than one in this many of its instructions per
/// activation leave the compiled code (`too_many_callbacks`): the clocked processes that run
/// faster compiled have one in eleven or fewer, the ones that ran slower one in nine or more.
const CALLBACKS: usize = 10;

/// Whether `f` is the body of a process that runs once per activation (a clocked or
/// combinational process: no loop of its own, at least `SHORT` IR instructions). Its time goes
/// to the signal and kernel operations compiled code reaches directly: it is compiled as soon as
/// it has run for a few milliseconds, within the budget of a compute loop, at the final level
/// (O0 code ran 15 to 25 percent slower than O2 on these bodies, so a trial of it said little), and
/// measured after the swap (`Worker::trial`, #596).
fn rtl(prog: &yavs_ir::Program, f: &yavs_ir::Func) -> bool {
    f.kind == FuncKind::Process && (RTL_MIN..=RTL_MAX).contains(&f.code.len()) && !loops(prog, f)
}

/// The smallest process `rtl` takes: below this the way into and out of compiled code costs
/// more than interpreting the body (the 90-instruction one of `rtl_chain` loses; decisions.md
/// "round 3").
const RTL_MIN: usize = 120;

/// A clocked process (`rtl`) of fewer instances than this stays interpreted: compiled, the
/// processes of a CPU design (most made once) ran 6 to 8 percent slower in cycles at the same
/// instructions, those of designs with many instances of one process 7 to 34 percent faster
/// (#510). ponytail: the instance count stands for how much an activation's code is reused;
/// a measured comparison per process if a design sits near the line.
const RTL_INSTANCES: u32 = 16;

/// A clocked process (`rtl`) seen for less than this part of the sampled time stays interpreted
/// (`cold`): between its activations so much other code runs that its compiled code starts cold
/// every time. The processes of 16 to 63 instances compiled in the CPU designs, a wide-vector
/// design and two testbenches (0.2 to 6.5 percent of the run when decided) each cost 200 to 900
/// instructions more per activation than interpreted; the processes that gain are half of the
/// run or more (#532). ponytail: the share when the decision is made, early in the run; a
/// measured comparison per process if a design sits near the line.
const RTL_SHARE: u64 = 8;

/// Why an `rtl` process made `n` times and seen for `seen` of `total` sampled microseconds would
/// run its compiled code cold, so slower than interpreted (`RTL_INSTANCES`, `RTL_SHARE`).
fn cold(n: u32, seen: u64, total: u64) -> Option<String> {
    if n < RTL_INSTANCES {
        Some(format!(
            "a clocked process made {n} times: compiled code of few instances ran slower (its \
             code does not stay in the caches)"
        ))
    } else if seen.saturating_mul(RTL_SHARE) < total {
        Some(format!(
            "a clocked process seen for {seen} of {total} us: compiled code of a process that is \
             a small part of the run ran slower (its code does not stay in the caches)"
        ))
    } else {
        None
    }
}

/// The largest process `rtl` takes.
const RTL_MAX: usize = 600;

/// The bodies of clocked processes that gained from compiled code in trials are 160 to 300 IR
/// instructions long (#575): `rtl_lfsr` (206) and variants of `rtl_ff` (228, 274) gain a tenth to a
/// fifth, `rtl_ff` itself (136) loses a tenth. Length stands for the work of an activation, which it
/// does not (`lfsr64`, 132, gains 11 percent at ten times its default length), so the measurement
/// after the swap judges all of them; the band only says how long a process must have been seen
/// before it is tried outside it (`band_wait`), because a trial that takes the code back costs the
/// run about 40 ms (+4 percent of `rtl_ff`'s 0.75 s, nothing at ten times that).
const RTL_BAND: std::ops::RangeInclusive<usize> = 160..=300;

/// The sampled microseconds a clocked process outside `RTL_BAND` must have been seen interpreted
/// before it is compiled (user, 2026-10-23: 0.25 s, was 1 s; the gain of the bodies that pay
/// starts that much earlier, the trial of a loser costs the same).
const BAND_WAIT: u64 = 250_000;

/// The sampled microseconds a clocked process of `len` IR instructions must have been seen
/// interpreted before it is compiled: none inside `RTL_BAND`, `BAND_WAIT` outside.
fn band_wait(len: usize) -> u64 {
    if RTL_BAND.contains(&len) {
        0
    } else {
        BAND_WAIT
    }
}

/// A compiled function seen for at least this part of the sampled time is measured after its swap
/// (`Worker::trial`): a smaller part moves the progress of the run less than the measure can see.
const TRIAL_SHARE: u64 = 8;

/// One window of a trial: at least this much CPU time of the simulation threads and `Rule::runs`
/// of progress, and as many times the largest step the progress made between two looks (a
/// millisecond apart; it is published once per delta cycle, so a window of few cycles counts too
/// coarsely); else the run cannot be measured (it has no clock to speak of) and the compiled code
/// stays. `TRIAL_MAX` bounds a window in wall time.
const TRIAL_WINDOW: Duration = Duration::from_millis(25);
const TRIAL_MAX: Duration = Duration::from_millis(250);
const TRIAL_STEPS: u64 = 30;

/// What a trial measures and when it decides.
struct Rule {
    /// The progress a window needs.
    runs: u64,
    /// A ratio outside `TRIAL_CLEAR` after the first pair or two of windows decides at once.
    early: bool,
    /// The compiled code stays when the run progressed at least this many times as fast with it as
    /// without.
    keep: f64,
}

/// A clocked process: the process runs of the kernel are the progress, and compiled code that
/// does not raise it by 5 percent is not worth its lower IPC (a 504 IR body kept at 1.03 ran 8
/// percent slower than the interpreter).
const BY_RUNS: Rule = Rule {
    runs: 1000,
    early: true,
    keep: 1.05,
};

/// A process that loops between its waits (or is too long or short for `rtl`): its own activations
/// are the progress. They come in bursts: a window varies by 10 to 25 percent with the code
/// identical, so all three pairs decide and only a ratio clearly below parity takes the code back
/// (one process is a part of the run, a gain of it raises the run's progress by less).
const BY_ENTRIES: Rule = Rule {
    runs: 500,
    early: false,
    keep: 0.95,
};

/// A ratio below the first of these after any pair takes the code back, one above the second after
/// the first pair or two keeps it (`Rule::early`).
const TRIAL_CLEAR: (f64, f64) = (0.9, 1.15);

/// What a trial says about the compiled code.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Verdict {
    /// The run progressed faster with it: it stays.
    Keep(f64),
    /// It did not: it is taken back for good.
    Drop(f64),
}

/// What the windows of a trial say: progress per CPU second with compiled code and with the
/// interpreter, in pairs taken in turn. `None` while the pairs so far do not decide.
fn decide(compiled: &[f64], interp: &[f64], rule: &Rule) -> Option<Verdict> {
    let mean = |v: &[f64]| v.iter().sum::<f64>() / v.len().max(1) as f64;
    let n = compiled.len().min(interp.len());
    if n == 0 || mean(interp) <= 0.0 {
        return None;
    }
    let r = mean(compiled) / mean(interp);
    if r < TRIAL_CLEAR.0 {
        // Not even O2 makes up for that.
        return Some(Verdict::Drop(r));
    }
    // Clear after one pair or two, else a third pair and the margin.
    let keep = |r: f64| Some(Verdict::Keep(r));
    match n {
        1 => (r >= TRIAL_CLEAR.1).then_some(Verdict::Keep(r)),
        2 if r >= TRIAL_CLEAR.1 => keep(r),
        2 if r >= TRIAL_CLEAR.0 + 0.05 => None,
        2 => Some(Verdict::Drop(r)),
        _ => Some(if r >= rule.keep {
            Verdict::Keep(r)
        } else {
            Verdict::Drop(r)
        }),
    }
}

/// CPU time of a clock (`clock_gettime`); `None` where it cannot be read.
#[cfg(target_os = "linux")]
fn cpu_read(clock: i32) -> Option<Duration> {
    #[repr(C)]
    struct Timespec {
        sec: i64,
        nsec: i64,
    }
    unsafe extern "C" {
        fn clock_gettime(clock: i32, t: *mut Timespec) -> i32;
    }
    let mut t = Timespec { sec: 0, nsec: 0 };
    // SAFETY: `t` is a `struct timespec` (64-bit `time_t`, as on the 64-bit Linux targets).
    let ok = unsafe { clock_gettime(clock, &mut t) } == 0;
    ok.then(|| Duration::new(t.sec as u64, t.nsec as u32))
}

/// CPU time of the calling thread (the trace's compile CPU).
fn cpu_thread() -> Option<Duration> {
    #[cfg(target_os = "linux")]
    return cpu_read(CPU_THREAD);
    #[cfg(not(target_os = "linux"))]
    None
}

/// CLOCK_PROCESS_CPUTIME_ID, CLOCK_THREAD_CPUTIME_ID
#[cfg(target_os = "linux")]
const CPU_PROCESS: i32 = 2;
#[cfg(target_os = "linux")]
const CPU_THREAD: i32 = 3;

/// CPU time of the simulation threads: that of the process less that of the calling thread (the
/// compile thread, which sleeps during a trial). `None` where the platform has no such clock.
#[cfg(target_os = "linux")]
fn sim_cpu() -> Option<Duration> {
    cpu_read(CPU_PROCESS)?.checked_sub(cpu_read(CPU_THREAD)?)
}

#[cfg(not(target_os = "linux"))]
fn sim_cpu() -> Option<Duration> {
    None
}

/// The CPU clock of the calling thread, for another thread to read (0 where there is none).
#[cfg(target_os = "linux")]
fn own_cpu_clock() -> i32 {
    unsafe extern "C" {
        fn pthread_self() -> usize;
        fn pthread_getcpuclockid(thread: usize, clock: *mut i32) -> i32;
    }
    let mut clock = 0;
    // SAFETY: `pthread_self` is the calling thread; `clock` holds the clock id.
    let rc = unsafe { pthread_getcpuclockid(pthread_self(), &mut clock) };
    if rc == 0 { clock } else { 0 }
}

#[cfg(not(target_os = "linux"))]
fn own_cpu_clock() -> i32 {
    0
}

/// CPU time of the simulation threads as the sampler thread sees it: the process's less its own
/// and the compile thread's (`compile_clock`, 0: none yet). `None` where there is no such clock.
#[cfg(target_os = "linux")]
fn sim_cpu_sampler(compile_clock: i32) -> Option<Duration> {
    // (The three reads are not at one instant: a difference that comes out negative is zero.)
    let rest = cpu_read(CPU_PROCESS)?.saturating_sub(cpu_read(CPU_THREAD)?);
    if compile_clock == 0 {
        return Some(rest);
    }
    Some(rest.saturating_sub(cpu_read(compile_clock)?))
}

#[cfg(not(target_os = "linux"))]
fn sim_cpu_sampler(_: i32) -> Option<Duration> {
    None
}

/// The progress per CPU second of the simulation threads in one window (per second of wall time
/// where `cpu` has no clock), `None` when it ended with too little progress to say or the tier
/// stopped.
fn window(
    progress: &dyn Fn() -> u64,
    cpu: &dyn Fn() -> Option<Duration>,
    halt: &dyn Fn() -> bool,
    min: Duration,
    max: Duration,
    min_runs: u64,
) -> Option<f64> {
    let (t0, p0, c0) = (std::time::Instant::now(), progress(), cpu());
    let (mut last, mut step) = (p0, 0);
    loop {
        std::thread::sleep(Duration::from_millis(1));
        let (wall, p) = (t0.elapsed(), progress());
        if halt() {
            return None;
        }
        step = step.max(p.wrapping_sub(last));
        last = p;
        let dt = match (cpu(), c0) {
            (Some(c), Some(c0)) => c.saturating_sub(c0),
            _ => wall,
        };
        let runs = p.wrapping_sub(p0);
        if dt >= min && runs >= min_runs && runs >= TRIAL_STEPS * step {
            return Some(runs as f64 / dt.as_secs_f64());
        }
        if wall >= max {
            return None;
        }
    }
}

/// The windows of a trial (progress per CPU second with the compiled code and with the
/// interpreter, in the order measured) and what they say.
#[derive(Debug, Default)]
struct Trial {
    verdict: Option<Verdict>,
    compiled: Vec<f64>,
    interp: Vec<f64>,
}

/// Run the windows of a trial: `swap(true)` puts the compiled code in (it is in when the trial
/// starts), `swap(false)` the interpreter. A first window with the compiled code warms it up; then
/// the interpreter and the compiled code in turn, three pairs at most. `verdict` is `None` when a
/// window could not be measured.
fn run_trial(
    swap: &mut dyn FnMut(bool),
    progress: &dyn Fn() -> u64,
    cpu: &dyn Fn() -> Option<Duration>,
    halt: &dyn Fn() -> bool,
    rule: &Rule,
) -> Trial {
    let w = || window(progress, cpu, halt, TRIAL_WINDOW, TRIAL_MAX, rule.runs);
    let mut t = Trial::default();
    if w().is_none() {
        return t;
    }
    for _ in 0..3 {
        swap(false);
        let Some(i) = w() else { return t };
        t.interp.push(i);
        swap(true);
        let Some(c) = w() else { return t };
        t.compiled.push(c);
        let d = decide(&t.compiled, &t.interp, rule);
        if let Some(d) = d.filter(|_| rule.early || t.interp.len() == 3) {
            t.verdict = Some(d);
            break;
        }
    }
    t
}

/// Microseconds of compile time per IR instruction of a clocked process (`rtl`; 130 to 270 IR
/// instructions took 31 to 92 ms at O2, 360 to 500 took 28 to 34 ms with the `Fast` level).
const US_PER_INSTR_RTL: f64 = 200.0;

/// A compute loop gets this many times the budget: it runs five to ten times faster compiled
/// (the interpreter's time goes to dispatch), so the payback rule alone limits it, and the
/// compile is on another thread (#432).
const COMPUTE_BUDGET: u64 = 50;

/// A function the interpreter calls on a native frame (`called_natively`) gets this many times
/// the budget: its compiled code runs about two to three times faster than the interpreter
/// (`Tier::call`, the object table; #510), so the payback rule limits it. Sixteen times is
/// 32 percent of the sampled time: with `PAYBACK_NATIVE` at 2 the former eight (16 percent) was
/// what the class ran into in the first seconds of a cold run (#572).
const NATIVE_BUDGET: u64 = 16;

/// An `rtl` process gets this many times the budget (its compiled code can be a third faster).
const RTL_BUDGET: u64 = 16;

/// Whether a callee that was seen running for `seen` microseconds and has `size` IR instructions is
/// left out of the compile of its caller: with a budget (the sampled times mean something),
/// never seen and not small. Compiling the closure of calls of a function brought every
/// error-reporting helper along, a fifth of the compile time of the window generator, and
/// priced `pop` out of the budget for seconds (#493).
fn left_out(budget: u32, seen: u64, size: usize) -> bool {
    budget != 0 && seen == 0 && size > UNSEEN_MIN
}

/// Whether compiling for `cost` more microseconds keeps the compile time spent within `budget`
/// percent (`factor` times that) of the `total` microseconds the sampler saw running; budget 0
/// is no limit.
fn within_budget(budget: u32, factor: u64, spent: u64, cost: u64, total: u64) -> bool {
    budget == 0 || (spent + cost) * 100 <= total * u64::from(budget) * factor
}

/// The first of the candidates, given as `(compile cost, factor)` in the order to try them, that
/// fits the budget: its position. Else, per candidate, the sampled time at which it will (the
/// budget grows with the run).
fn first_fit(
    prices: &[(u64, u64)],
    budget: u32,
    spent: u64,
    total: u64,
) -> Result<usize, Vec<u64>> {
    if let Some(k) = prices
        .iter()
        .position(|&(cost, factor)| within_budget(budget, factor, spent, cost, total))
    {
        return Ok(k);
    }
    Err(prices
        .iter()
        .map(|&(cost, factor)| {
            let per = u64::from(budget).saturating_mul(factor).max(1);
            (spent + cost).saturating_mul(100) / per
        })
        .collect())
}

/// See `Heat::o2_threshold`.
const O2_AFTER: u64 = 30;

/// The microseconds of CPU time a compile at `OptLevel::FastRa` takes (the optimised register
/// allocator over thousands of blocks is more than linear): 100 per IR instruction and 0.05 more
/// for every instruction of the function. 400 instructions took 50 ms, 4 500 1.4 s, 17 500 17 s
/// (`Fast`: 0.1 to 0.15 s per thousand; #715).
fn ra_cost(ir: usize) -> u64 {
    let n = ir as f64;
    (n * (100.0 + 0.05 * n)) as u64
}

/// A function that ran at `OptLevel::Fast` is compiled again at `FastRa` once it was seen running
/// compiled for this many times that compile: its code is 6 to 13 percent faster, and the rest of
/// the run is expected to be as long as the past (`PAYBACK`), so the compile repays at about
/// twelve times its cost. A run too short for that never pays for it.
const PAYBACK_RA: u64 = 12;

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

    /// Back to the interpreter from the next entry; the code stays valid for frames running it.
    fn clear(&self) {
        self.run.store(0, Ordering::Release);
    }
}

/// Hot functions compiled on a background thread; see the module docs.
pub struct AutoTier {
    funcs: Arc<[Entry]>,
    ntab: Arc<[AtomicUsize]>,
    /// Per function, its `CallFn` once its native-frame code is published (`Tier::call`).
    ctab: Arc<[AtomicUsize]>,
    /// Functions `hot` sent to the thread already.
    queued: Box<[AtomicBool]>,
    queue: Mutex<Option<Sender<Msg>>>,
    shared: Arc<Shared>,
    worker: Mutex<Option<JoinHandle<()>>>,
}

/// What the compile thread is told.
enum Msg {
    /// `Tier::hot`: compile this function (eager mode, tests).
    Hot(FuncId),
    /// Looks of the sampler thread, in order (`Look`).
    Looks(Vec<Look>),
}

/// One look of the sampler thread: the CPU time the simulation threads used since the last look
/// (microseconds) and what they were running. The compile thread works through the looks in order
/// whenever it is free, so what it decides depends on the sequence of looks and not on how long a
/// compile or a trial took (#670).
struct Look {
    dt: u64,
    running: Vec<(FuncId, bool)>,
}

/// What the looks have found for the compile thread to do.
#[derive(Default)]
struct Pending {
    /// Functions `Tier::hot` asked for (eager mode, tests) and `YAVS_AUTO_FORCE` names.
    asked: std::collections::VecDeque<FuncId>,
    /// Hot functions waiting for the budget (`Worker::pick`).
    cands: Vec<FuncId>,
    /// Functions to compile again at O2.
    later: Vec<FuncId>,
    /// Functions to look up in the compilation cache (`Worker::probe`).
    probes: Vec<FuncId>,
}

/// Looks sent together: the compile thread wakes once per this many.
const BATCH: usize = 8;

/// What the tier and its compile thread share.
#[derive(Default)]
struct Shared {
    /// Set by `stop`: compile nothing more.
    halt: AtomicBool,
    /// The thread is compiling a function (or about to look at `halt` before it does).
    busy: AtomicBool,
    /// A trial is running: its swaps would show as the code running, so the sampler sends no looks.
    trial: AtomicBool,
    /// The CPU clock of the compile thread (`pthread_getcpuclockid`; 0 until it is set), which the
    /// sampler subtracts from the process's to get the simulation's.
    compile_clock: std::sync::atomic::AtomicI32,
    /// Eager mode: per function, its compile (or the decision to leave it interpreted) is over,
    /// `Tier::hot` returns then.
    finished: Vec<AtomicBool>,
    /// The JITs once the thread has ended: compiled code lives as long as the tier.
    jit: Mutex<Vec<Jit>>,
}

/// Microseconds of compile time per IR instruction of a function, as the policy prices it. Fixed
/// numbers, not learned from the compiles the run makes: what the compile thread measures depends on
/// the load of the host and on whether the compilation cache hit, and a policy that prices by it
/// decides differently from run to run (#670, #673, #680). 800 is the price the learned rate started
/// at and stayed near when the compile thread shared a core: the wall time of a cold compile of a
/// function that runs on native frames, both modules (docs/STATUS.md "`--tier=auto` decisions").
const US_PER_INSTR: f64 = 800.0;
/// `OptLevel::Fast` (`FAST_MIN` IR instructions or more that are not compute loops) costs 70 to 130
/// per IR instruction, which the single learned rate never saw (#680). Only the budget uses it
/// (`Worker::price`); the payback rule keeps `US_PER_INSTR` (`Worker::payback_cost`).
const US_PER_INSTR_FAST: f64 = 110.0;
/// A module linked from the compilation cache (generation, key and link), per IR instruction.
const US_PER_INSTR_HIT: f64 = 20.0;

/// Microseconds of opening the LLVM library (about 40 million cycles), counted with the first
/// compile or look-up in the compilation cache: a run too short to pay for it loads nothing.
const LOAD_US: u64 = 10_000;

/// A look-up in the compilation cache generates the code of the function, about this part of a
/// compile (generation and the key: 10 to 20 us per IR instruction).
const PROBE_PART: u64 = 50;

/// A function must have been seen interpreted this many times as long as compiling it takes:
/// its remaining time is expected to be about its past time and compiled code saves about a
/// quarter of it.
const PAYBACK: u64 = 4;

/// How long the compile thread waits before it samples (not for eager tiers).
const START_AFTER: Duration = Duration::from_millis(5);

/// A compute loop runs five to ten times faster compiled, so seen for twice its compile time
/// it has repaid it (`Worker::threshold`).
const PAYBACK_COMPUTE: u64 = 2;

/// A function the interpreter calls on a native frame runs two to three times faster compiled,
/// so it is compiled once seen for twice its compile time (four times before #572).
const PAYBACK_NATIVE: u64 = 2;

/// A callee of a hot function that was never seen running and has more IR instructions than this
/// is left out of the compile (`left_out`).
const UNSEEN_MIN: usize = 32;

/// Callees compiled with a hot function (`Worker::tree`): at most this many IR instructions
/// each and in all.
const CALLEE_MAX: usize = 200;
const TREE_MAX: usize = 1500;

/// A callee of at most `CALLEE_MAX` IR instructions that a candidate calls on every run of it
/// (`must_calls`, transitively) is compiled with it even when it was never seen running
/// (`Worker::hot`): it is called as often as the candidate, which is hot. A function of more IR
/// instructions than this is not searched for such calls.
const HOT_ROOT_MAX: usize = 4000;

/// The most IR instructions of a function and the functions it calls on native frames that
/// compile together (`Worker::tree`).
const NATIVE_MAX: usize = 3000;

/// The compile thread's state.
struct Worker {
    prog: Arc<yavs_ir::Program>,
    table: Arc<[Entry]>,
    /// Native-frame code per function (`Mem::ntab`), published before the entry.
    ntab: Arc<[AtomicUsize]>,
    /// `CallFn` per function, published with its native-frame code.
    ctab: Arc<[AtomicUsize]>,
    native: Arc<yavs_interp::NativeFlags>,
    heat: Heat,
    opt: OptLevel,
    max_size: usize,
    /// Made with the first function compiled at their level: a run that never gets hot pays
    /// nothing.
    jit: Option<Jit>,
    jit2: Option<Jit>,
    /// For functions at `OptLevel::Fast`.
    jitf: Option<Jit>,
    /// For functions recompiled at `OptLevel::FastRa`.
    jitr: Option<Jit>,
    /// Per function: compiled at the first level, at O2.
    done: Vec<[bool; 2]>,
    /// Per function: its first-level code is `OptLevel::Fast`, so the second compile is `FastRa`.
    fast: Vec<bool>,
    /// Per function: the address of its native-frame code once compiled (0: none), and whether
    /// `ntab` names it. Published once everything it calls natively is compiled too: a missing
    /// callee would materialise the frames at every call.
    nat: Vec<(usize, bool)>,
    /// Per function: the address of its `CallFn` (0: none), published with `nat`.
    calls: Vec<usize>,
    /// Microseconds each function was seen interpreted, and running compiled.
    time: Vec<[u64; 2]>,
    /// Per process (by `Program::base_of`), the microseconds it and all its copies were seen
    /// interpreted: the time every policy decision about a copy goes by (#661).
    group: Vec<u64>,
    /// Per function: it became a candidate (`start`).
    asked_for: Vec<bool>,
    /// Microseconds the simulation threads were seen running, and the compile thread spent
    /// compiling.
    total: u64,
    spent: u64,
    /// `compute` per function, once asked.
    compute: Vec<std::cell::OnceCell<bool>>,
    /// `native_set` of each function alone, once asked (`nset`).
    nsets: Vec<std::cell::OnceCell<std::collections::HashSet<FuncId>>>,
    /// The sampled time `pick` waits for: the cheapest candidate it last refused fits the budget
    /// from then on.
    pick_at: u64,
    note: Note,
    /// Per short function that runs on native frames, the interpreted caller its time counts
    /// for (`charge`); emptied when a function is first seen.
    leaf: std::collections::HashMap<FuncId, Option<FuncId>>,
    seen: usize,
    /// Per function: `pick` said once that the budget refused it.
    told: Vec<bool>,
    /// Per function: `tree` left it out of a compile as a callee (`unseen`); the native-frame code
    /// of a caller is published without it (`publish`).
    left: Vec<std::cell::Cell<bool>>,
    /// `must_calls` per function, once asked.
    musts: Vec<std::cell::OnceCell<Vec<FuncId>>>,
    /// Per function: a trial found the run no faster with its compiled code (`trial`): it stays
    /// interpreted, whatever else asks for it.
    lost: Vec<bool>,
    /// A trial found the run not measurable (`window`): none of the others is tried.
    unmeasurable: bool,
    /// Per function: looked up in the compilation cache already (`probe`).
    probed: Vec<bool>,
    /// A `probe` is running: a failure is no note and leaves the function to compile later.
    probing: bool,
    /// The warning that the LLVM library cannot be loaded was printed.
    warned: bool,
    /// The LLVM library was opened (its time counts as compile time spent).
    loaded: bool,
    /// The functions the compilation cache has compiled code of (`objcache::compiled_list`):
    /// the others are not looked up.
    listed: std::collections::HashSet<(String, u32)>,
    trace: Option<Trace>,
}

/// `YAVS_AUTO_TRACE=1`, a tuning aid: the compile thread's timeline on stderr (a line per compile
/// and per refusal, the most sampled functions at the end).
struct Trace {
    t0: std::time::Instant,
    /// Per function, the milliseconds into the run it was first seen interpreted and became a
    /// candidate.
    first: std::collections::HashMap<FuncId, u128>,
    cand: std::collections::HashMap<FuncId, u128>,
}

impl Trace {
    fn ms(&self) -> u128 {
        self.t0.elapsed().as_millis()
    }

    fn line(&self, what: String) {
        eprintln!("trace {:>8} ms: {what}", self.ms());
    }

    /// Milliseconds since `at[id]`, -1 if never.
    fn since(&self, at: &std::collections::HashMap<FuncId, u128>, id: FuncId) -> i128 {
        at.get(&id).map_or(-1, |&c| self.ms() as i128 - c as i128)
    }
}

impl Worker {
    /// Take one look of the sampler: charge the CPU time it covers to what the simulation threads
    /// were running, and note what became hot. Only the look and the state decide; nothing here
    /// reads a clock.
    fn see(&mut self, look: &Look, upgrade: bool, pending: &mut Pending) {
        if look.running.is_empty() {
            return;
        }
        let dt = look.dt / look.running.len() as u64;
        for &(id, compiled) in &look.running {
            let id = if compiled { id } else { self.charge(id) };
            let Some(t) = self.time.get_mut(id.0 as usize) else {
                continue;
            };
            let t = &mut t[usize::from(compiled)];
            let was = *t;
            if was == 0
                && !compiled
                && let Some(tr) = &mut self.trace
            {
                let now = tr.ms();
                tr.first.entry(id).or_insert(now);
            }
            *t += dt;
            self.total += dt;
            if was == 0 && !compiled && forced(&self.prog, id) {
                pending.asked.push_back(id);
            }
            if compiled {
                let (now, at) = (*t, self.threshold(id, true));
                if was < at && now >= at && (upgrade || self.fast[id.0 as usize]) {
                    pending.later.push(id);
                }
                continue;
            }
            // The time of a process counts for every copy of it (#661): the process is hot when
            // its copies together are, and each copy that runs is a candidate then.
            let g = self.prog.base_of(id).0 as usize;
            self.group[g] += dt;
            let (now, at) = (self.group[g], self.threshold(id, false));
            if self.wants_probe(id, now) {
                pending.probes.push(id);
            }
            if now >= at && !std::mem::replace(&mut self.asked_for[id.0 as usize], true) {
                pending.cands.push(id);
                self.pick_at = 0;
                if let Some(tr) = &mut self.trace {
                    let now = tr.ms();
                    tr.cand.insert(id, now);
                }
            }
        }
    }

    /// A worker with nothing seen yet.
    #[allow(clippy::too_many_arguments)]
    fn new(
        prog: Arc<yavs_ir::Program>,
        table: Arc<[Entry]>,
        ntab: Arc<[AtomicUsize]>,
        ctab: Arc<[AtomicUsize]>,
        native: Arc<yavs_interp::NativeFlags>,
        heat: Heat,
        opt: OptLevel,
        max_size: usize,
        note: Note,
    ) -> Worker {
        let nfuncs = prog.funcs.len();
        Worker {
            time: vec![[0; 2]; nfuncs],
            group: vec![0; nfuncs],
            asked_for: vec![false; nfuncs],
            done: vec![[false; 2]; nfuncs],
            fast: vec![false; nfuncs],
            nat: vec![(0, false); nfuncs],
            calls: vec![0; nfuncs],
            prog,
            table,
            ntab,
            ctab,
            native,
            heat,
            opt,
            max_size,
            jit: None,
            jit2: None,
            jitf: None,
            jitr: None,
            total: 0,
            spent: 0,
            compute: vec![Default::default(); nfuncs],
            nsets: vec![Default::default(); nfuncs],
            pick_at: 0,
            note,
            leaf: Default::default(),
            seen: 0,
            probed: vec![false; nfuncs],
            lost: vec![false; nfuncs],
            unmeasurable: false,
            told: vec![false; nfuncs],
            left: vec![Default::default(); nfuncs],
            musts: vec![Default::default(); nfuncs],
            probing: false,
            warned: false,
            loaded: false,
            listed: Default::default(),
            trace: std::env::var_os("YAVS_AUTO_TRACE").map(|_| Trace {
                t0: std::time::Instant::now(),
                first: Default::default(),
                cand: Default::default(),
            }),
        }
    }

    /// The function the sampled interpreted time of `id` counts for: a short nested function
    /// that runs on native frames counts for the interpreted caller seen longest that calls it
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
            // The interpreter calls any other function on a native frame (`Tier::call`), so
            // compiling it alone gains; a nested one is entered with its frame-mode code.
            if f.parent.is_none()
                || too_short(prog, f, id).is_none()
                || !crate::lower::native_set(prog, [id]).contains(&id)
            {
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

    /// Whether `id` is a clocked process (`rtl`). A copy of a process (`Program::copy_of`) counts
    /// as the process it is a copy of, so every copy gets the decisions the process would get.
    fn is_rtl(&self, id: FuncId) -> bool {
        self.prog
            .func(self.prog.base_of(id))
            .is_some_and(|f| rtl(&self.prog, f))
    }

    /// The modules `tree` makes of `id` as a root: native-frame code for a function that runs on
    /// native frames, with frame-mode code too when it has a loop, else frame-mode code.
    fn natural_part(&self, id: FuncId) -> Part {
        match self.prog.func(id) {
            Some(f) if f.parent.is_none() && self.nset(id).contains(&id) => {
                if loops(&self.prog, f) {
                    Part::Both
                } else {
                    Part::Native
                }
            }
            _ => Part::Frame,
        }
    }

    /// The microseconds of compile time to expect for `id` as `tree` compiles it: what the
    /// budget is charged.
    fn cost(&self, id: FuncId) -> u64 {
        self.price(id, self.natural_part(id))
    }

    /// The microseconds per IR instruction of `id` for the budget (`fast`: `Fast` code at its
    /// own rate) or for the payback rule.
    fn rate(&self, id: FuncId, fast: bool) -> f64 {
        if self.is_rtl(id) {
            US_PER_INSTR_RTL
        } else if fast
            && self.prog.func(id).is_some_and(|f| {
                crate::lower::level_for(&self.prog, f, OptLevel::O2) == OptLevel::Fast
            })
        {
            US_PER_INSTR_FAST
        } else {
            US_PER_INSTR
        }
    }

    /// The compile time the payback rule and the order of the candidates go by (`threshold`,
    /// `pick`): the price of `id` with `Fast` code at the rate of the other code, so that a big
    /// process, which the code generator compiles in a seventh of the time, is compiled as late as
    /// it always was. Whether big processes compile is the user's decision (#679), not a side
    /// effect of pricing them right for the budget (#680).
    fn payback_cost(&self, id: FuncId) -> u64 {
        (self.rate(id, false) * self.prog.func(id).map_or(0, |f| f.code.len()) as f64) as u64
    }

    /// The microseconds of compile time to expect for the modules `part` of `id` (the budget's
    /// price): the price of the function, shared by the two modules of a function that has both.
    fn price(&self, id: FuncId, part: Part) -> u64 {
        let Some(f) = self.prog.func(id) else {
            return 0;
        };
        let whole = self.rate(id, true) * f.code.len() as f64;
        let both = self.natural_part(id) == Part::Both;
        (if part != Part::Both && both {
            whole / 2.0
        } else {
            whole
        }) as u64
    }

    /// `native_set` of `id` alone (the functions it reaches on native frames, `id` among them
    /// when it runs on one), worked out once: the worker asks for it at every look at its
    /// candidates.
    fn nset(&self, id: FuncId) -> &std::collections::HashSet<FuncId> {
        self.nsets[id.0 as usize].get_or_init(|| native_set(&self.prog, [id]))
    }

    /// `called_natively`, worked out once.
    fn natively(&self, id: FuncId) -> bool {
        self.prog.func(id).is_some_and(|f| {
            f.parent.is_none()
                && matches!(f.kind, FuncKind::Function | FuncKind::Procedure)
                && self.nset(id).contains(&id)
        })
    }

    /// Whether `id` is a compute loop (`compute`), worked out once.
    fn is_compute(&self, id: FuncId) -> bool {
        *self.compute[id.0 as usize]
            .get_or_init(|| self.prog.func(id).is_some_and(|f| compute(&self.prog, f)))
    }

    /// IR instructions of the process `id` is, or is a copy of: what the policy goes by, so the
    /// copies of a process all get the decision the process gets.
    fn base_len(&self, id: FuncId) -> usize {
        self.prog
            .func(self.prog.base_of(id))
            .map_or(0, |f| f.code.len())
    }

    /// The time `id` must be seen at its level before it is compiled (again).
    fn threshold(&self, id: FuncId, compiled: bool) -> u64 {
        let Some(f) = self.prog.func(id) else {
            return u64::MAX;
        };
        if compiled {
            // A function that ran at `Fast` is compiled again at `FastRa` once it repays (#715).
            if self.fast[id.0 as usize] && self.heat.budget != 0 {
                return PAYBACK_RA.saturating_mul(ra_cost(f.code.len()));
            }
            return self.heat.o2_threshold(f);
        }
        // (A copy's seen time is its process's, `Worker::group`: its threshold is the process's too.)
        let at = self
            .prog
            .func(self.prog.base_of(id))
            .map_or(u64::MAX, |b| self.heat.threshold(b));
        if self.heat.budget == 0 {
            at
        } else if self.is_rtl(id) {
            at.max(band_wait(self.base_len(id)))
        } else if self.is_compute(id) {
            at.max(PAYBACK_COMPUTE * self.payback_cost(id))
        } else if self.natively(id) {
            at.max(PAYBACK_NATIVE * self.payback_cost(id))
        } else {
            at.max(PAYBACK * self.payback_cost(id))
        }
    }

    /// Whether callee `i` of a candidate is left out of its compile: never seen running (the
    /// error paths of a library function), so a call to it is rare and leaves the compiled code
    /// when it happens, and it compiles by itself once it gets hot.
    fn unseen(&self, i: FuncId, hot: &std::collections::HashSet<FuncId>) -> bool {
        let k = i.0 as usize;
        let size = self.prog.func(i).map_or(0, |f| f.code.len());
        let out = self.lost[k]
            || !hot.contains(&i)
                && left_out(self.heat.budget, self.time[k][0] + self.time[k][1], size);
        if out {
            self.left[k].set(true);
        }
        out
    }

    /// The small functions `root` calls on every run, and they in turn (`must_calls`, at most
    /// `CALLEE_MAX` IR instructions each): hot with `root` whether the sampler saw them or not.
    fn hot(&self, root: FuncId) -> std::collections::HashSet<FuncId> {
        let mut out = std::collections::HashSet::new();
        let mut work = vec![root];
        while let Some(g) = work.pop() {
            let calls = self.musts[g.0 as usize].get_or_init(|| {
                self.prog
                    .func(g)
                    .map_or(Vec::new(), |f| must_calls(&self.prog, f))
            });
            for &c in calls {
                if self
                    .prog
                    .func(c)
                    .is_some_and(|f| f.code.len() <= CALLEE_MAX)
                    && out.insert(c)
                {
                    work.push(c);
                }
            }
        }
        out
    }

    /// `root` and the functions it calls that compile with it: a call from compiled code to
    /// one that is not leaves the compiled code (`TIER_UNWIND`). A function that runs on native
    /// frames brings every function it calls on native frames, compiled for those frames only,
    /// so its native-frame code can be published; a closure over `NATIVE_MAX` instructions is
    /// left (the root gets frame-mode code only).
    fn tree(&self, root: FuncId) -> Vec<(FuncId, Part)> {
        let size = |i: FuncId| self.prog.func(i).map_or(0, |f| f.code.len());
        let hot = self.hot(root);
        let nc = self.nset(root);
        if nc.contains(&root) {
            // (A hash set iterates in a different order in every process: sorted, so the callees
            // compile in the same order in every run, #670.)
            let mut todo: Vec<FuncId> = nc
                .iter()
                .copied()
                .filter(|&i| i != root && self.nat[i.0 as usize].0 == 0 && !self.unseen(i, &hot))
                .collect();
            todo.sort_unstable_by_key(|i| i.0);
            let total = size(root) + todo.iter().map(|&i| size(i)).sum::<usize>();
            if total <= NATIVE_MAX {
                // The interpreter calls the root on a native frame (`Tier::call`) when it has
                // no static link; a nested root is entered with its frame-mode code, and a root
                // with a loop gets that too, for the interpreter to enter again where a native
                // frame of it was left to the interpreter.
                let part = match self.prog.func(root) {
                    Some(f) if f.parent.is_none() && !loops(&self.prog, f) => Part::Native,
                    Some(f) if f.parent.is_none() => Part::Both,
                    _ => Part::Frame,
                };
                let mut ids = vec![(root, part)];
                ids.extend(todo.into_iter().map(|i| (i, Part::Native)));
                return ids;
            }
            return vec![(root, Part::Frame)];
        }
        let mut ids = vec![root];
        let mut natives: Vec<(FuncId, Part)> = Vec::new();
        let mut total = size(root);
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
                    && !self.unseen(*func, &hot)
                    && g.kind != FuncKind::Elab
                    && g.code.len() <= CALLEE_MAX
                    && total + g.code.len() <= TREE_MAX
                {
                    total += g.code.len();
                    ids.push(*func);
                } else if self.done[func.0 as usize][0]
                    && self.nat[func.0 as usize].0 == 0
                    && !natives.iter().any(|n| n.0 == *func)
                    && g.code.len() <= CALLEE_MAX
                    && total + g.code.len() <= TREE_MAX
                    && self.nset(*func).contains(func)
                    && !self.unseen(*func, &hot)
                {
                    // Compiled for the interpreter's calls (frame-mode code only): its
                    // native-frame code, for the calls of this function.
                    total += g.code.len();
                    natives.push((*func, Part::Native));
                }
            }
        }
        // A callee that runs on native frames is called natively by its compiled callers and
        // by the interpreter; frame-mode code only for one with a loop or a static link.
        let mut out: Vec<(FuncId, Part)> = ids
            .into_iter()
            .enumerate()
            .map(|(k, i)| {
                let native_only = k > 0
                    && self.prog.func(i).is_some_and(|f| {
                        f.parent.is_none() && !loops(&self.prog, f) && self.nset(i).contains(&i)
                    });
                (
                    i,
                    if native_only {
                        Part::Native
                    } else {
                        Part::Both
                    },
                )
            })
            .collect();
        out.extend(natives);
        out
    }

    /// Publish the native-frame code whose callees are all compiled.
    fn publish(&mut self) {
        for k in 0..self.nat.len() {
            let (addr, done) = self.nat[k];
            if addr == 0 || done || self.lost[k] {
                continue;
            }
            let ready = self
                .nset(FuncId(k as u32))
                .iter()
                .all(|c| self.nat[c.0 as usize].0 != 0 || self.left[c.0 as usize].get());
            if ready {
                self.ntab[k].store(addr, Ordering::Release);
                self.nat[k].1 = true;
                if self.calls[k] != 0 {
                    self.ctab[k].store(self.calls[k], Ordering::Release);
                    self.native.set_call(FuncId(k as u32));
                }
            }
        }
    }

    /// The function to compile next and what goes with it: the hottest relative to its cost
    /// among the candidates the budget has room for (`first_fit`). One that has none waits for
    /// the sampled time it fits at (`pick_at`) without holding up the others (#572).
    fn pick(&mut self, cands: &mut Vec<FuncId>) -> Option<Vec<(FuncId, Part)>> {
        // A function whose frame-mode code compiled in the meantime (as the callee of another
        // candidate) needs no pick; one with native-frame code only may still need its other part.
        cands.retain(|c| !self.done[c.0 as usize][0]);
        if self.total < self.pick_at || cands.is_empty() {
            return None;
        }
        let heat = |w: &Self, id: FuncId| {
            let cost = w.payback_cost(id).max(1);
            w.time[id.0 as usize][0] * 1000 / cost
        };
        let mut order: Vec<usize> = (0..cands.len()).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(heat(self, cands[i])));
        let mut trees = Vec::new();
        let mut prices = Vec::new();
        for &i in &order {
            let ids = self.tree(cands[i]);
            let cost: u64 =
                ids.iter().map(|&(i, p)| self.price(i, p)).sum::<u64>() + self.load_cost();
            prices.push((cost, self.factor(cands[i])));
            trees.push(ids);
        }
        match first_fit(&prices, self.heat.budget, self.spent, self.total) {
            Ok(k) => {
                cands.swap_remove(order[k]);
                Some(trees.swap_remove(k))
            }
            Err(waits) => {
                self.pick_at = waits.iter().copied().min().unwrap_or(0);
                for (k, at) in waits.into_iter().enumerate() {
                    let id = cands[order[k]];
                    // Said once per function (`--verbose`), so a hot function that never compiles
                    // says why.
                    if let Some(t) = self.told.get_mut(id.0 as usize)
                        && !std::mem::replace(t, true)
                    {
                        let why = format!(
                            "the compile budget has no room for it yet (about {} us of compiling, \
                             possible after {at} us of the run seen)",
                            prices[k].0
                        );
                        (self.note)(id, self.opt, Err(why));
                    }
                    if let Some(tr) = &self.trace {
                        let mut big: Vec<(usize, &str)> = trees[k]
                            .iter()
                            .filter_map(|&(i, _)| self.prog.func(i))
                            .map(|f| (f.code.len(), f.name.as_str()))
                            .collect();
                        big.sort_by_key(|b| std::cmp::Reverse(b.0));
                        tr.line(format!(
                            "refused f{} `{}`, cost {} us, seen {} us, fits at {at} of {} us seen, spent {}; \
                             {} functions of {} IR, the biggest {:?}",
                            id.0,
                            self.prog.func(id).map_or("?", |f| f.name.as_str()),
                            prices[k].0,
                            self.time[id.0 as usize][0],
                            self.total,
                            self.spent,
                            big.len(),
                            big.iter().map(|b| b.0).sum::<usize>(),
                            &big[..big.len().min(4)]
                        ));
                    }
                }
                None
            }
        }
    }

    /// Whether `id` (interpreted, seen for `seen` microseconds) should be looked up in the
    /// compilation cache now: once it is as hot as compiling it at all needs
    /// (`Heat::threshold`), whatever the payback rule and the budget say about compiling it (a hit
    /// costs no code generation), within the budget for the look-up itself (#505, #510).
    fn wants_probe(&mut self, id: FuncId, seen: u64) -> bool {
        let Some(f) = self.prog.func(id) else {
            return false;
        };
        let k = id.0 as usize;
        let wait = if self.is_rtl(id) {
            band_wait(self.base_len(id))
        } else {
            0
        };
        let at = self
            .prog
            .func(self.prog.base_of(id))
            .map_or(u64::MAX, |b| self.heat.threshold(b));
        if self.probed[k] || self.done[k][0] || seen < at.max(wait) {
            return false;
        }
        // The look-up generates the function's code: within the budget, as a compile is, with at
        // least the factor of a function called natively (code compiled before for a function
        // hot before is likely to pay); asked again at the next sample while there is no room.
        let go = crate::objcache::enabled()
            && self.listed.contains(&(f.name.clone(), f.code.len() as u32))
            && within_budget(
                self.heat.budget,
                self.factor(id).max(NATIVE_BUDGET),
                self.spent,
                self.load_cost() + self.cost(id) / PROBE_PART,
                self.total,
            );
        self.probed[k] = go;
        go
    }

    /// How many times the budget compiling `id` gets: its compiled code runs that much faster.
    fn factor(&self, id: FuncId) -> u64 {
        match self.prog.func(id) {
            _ if self.is_compute(id) => COMPUTE_BUDGET,
            Some(_) if self.is_rtl(id) => RTL_BUDGET,
            Some(_) if self.natively(id) => NATIVE_BUDGET,
            _ => 1,
        }
    }

    /// `LOAD_US` until the LLVM library is open.
    fn load_cost(&self) -> u64 {
        if self.loaded { 0 } else { LOAD_US }
    }

    /// Link `id` from the compilation cache if it is there, at once and outside the budget (a
    /// hit costs no codegen), with what compiles with it (`tree`: callees that miss are compiled);
    /// whether it was there. A miss leaves it to the payback rule; its generation counts as
    /// compile time spent.
    fn probe(&mut self, id: FuncId) -> bool {
        let ids = self.tree(id);
        let Some(&(root, part)) = ids.first() else {
            return false;
        };
        self.probing = true;
        let hit = crate::objcache::cached_only(|| self.compile(root, false, true, part));
        self.probing = false;
        if !hit {
            self.spent += self.cost(id) / PROBE_PART;
            return false;
        }
        // A compute loop's callees compile (it runs five to ten times faster compiled), and so do
        // the native-frame callees of a native root (its native-frame code is published only once
        // all of them exist, #510); other callees link what the cache has and leave the rest to
        // the payback rule.
        let compute = self.is_compute(root);
        for &(i, p) in &ids[1..] {
            if compute || p == Part::Native && self.nset(root).contains(&root) {
                self.compile(i, false, false, p);
            } else {
                self.probing = true;
                crate::objcache::cached_only(|| self.compile(i, false, false, p));
                self.probing = false;
            }
        }
        true
    }

    /// Measure what the compiled code of `id`, just published, does for the run (#596, #652): the
    /// progress per CPU second with it and with the interpreter running `id`, in turn, a window
    /// each (`run_trial`). The simulation is bound by the CPU, so the progress per second is what
    /// faster code raises, kernel and cache effects of the compiled code included. The progress
    /// is the process runs of the kernel for a clocked process (`rtl`: its activations are the
    /// work), else the activations of the process `id` itself (`NativeFlags::count_on`); a function
    /// is not measured. Compiled code that did not raise it
    /// (`Rule::keep`) is taken back for good. Only a function seen for a good part of the run
    /// (`TRIAL_SHARE`) is measured, and only in a run that makes enough progress per window; the
    /// compile thread waits meanwhile.
    fn trial(&mut self, id: FuncId, halt: &AtomicBool) {
        let k = id.0 as usize;
        let Some(f) = self.prog.func(id) else {
            return;
        };
        let clocked = self.is_rtl(id);
        // A function is not measured: compiled callers use its native code (taking it back makes
        // each of their calls materialise a frame), its entries vary with the phase of the run
        // (the window generator's `get`: 1.5 M a second early, 50 k late, so a late trial took it
        // back at 0.93 and doubled the run's instructions), and its static rules are enough
        // where nothing compiled calls it. A process nothing calls is measured by its activations.
        if !clocked && f.kind != FuncKind::Process {
            return;
        }
        if self.heat.budget == 0
            || self.heat.eager()
            || clocked && self.unmeasurable
            || self.time[k][0].saturating_mul(TRIAL_SHARE) < self.total
        {
            return;
        }
        let (run, (_, published)) = (self.table[k].get(), self.nat[k]);
        let native = self.ntab[k].load(Ordering::Acquire);
        let call = self.ctab[k].load(Ordering::Acquire);
        // Nothing of its own to swap, or its native-frame code still waits for its callees.
        if run.is_none() && native == 0 || self.nat[k].0 != 0 && !published {
            return;
        }
        // Compiled callers find a callee without native-frame code (null) and leave to the interpreter.
        let mut swap = |compiled: bool| {
            if compiled {
                if let Some(r) = run {
                    self.table[k].set(r);
                    self.native.set(id);
                }
                self.ntab[k].store(native, Ordering::Release);
                self.ctab[k].store(call, Ordering::Release);
                if call != 0 {
                    self.native.set_call(id);
                }
            } else {
                self.native.clear_call(id);
                self.ctab[k].store(0, Ordering::Release);
                self.ntab[k].store(0, Ordering::Release);
                if run.is_some() {
                    self.native.clear(id);
                    self.table[k].clear();
                }
            }
        };
        let t0 = std::time::Instant::now();
        let counts = self.native.clone();
        let halted = || halt.load(Ordering::SeqCst);
        let t = if clocked {
            run_trial(
                &mut swap,
                &yavs_interp::sim_progress,
                &sim_cpu,
                &halted,
                &BY_RUNS,
            )
        } else {
            counts.count_on(id);
            let t = run_trial(
                &mut swap,
                &|| counts.entries(),
                &sim_cpu,
                &halted,
                &BY_ENTRIES,
            );
            counts.count_off(id);
            t
        };
        let r = t.verdict;
        let drop = matches!(r, Some(Verdict::Drop(_)));
        swap(!drop);
        // No clock to speak of: none of the clocked processes can be measured. A function that is
        // entered too rarely says nothing about the others.
        if r.is_none() && clocked {
            self.unmeasurable = true;
        }
        if let Some(tr) = &self.trace {
            let name = &f.name;
            tr.line(format!(
                "trial f{k} `{name}`: {} after {} ms (per CPU second, interpreter:{}, compiled:{})",
                match r {
                    Some(Verdict::Keep(x)) =>
                        format!("kept, {x:.3} times the progress of the interpreter"),
                    Some(Verdict::Drop(x)) =>
                        format!("taken back, {x:.3} times the progress of the interpreter"),
                    None => "not measurable, kept".to_string(),
                },
                t0.elapsed().as_millis(),
                t.interp
                    .iter()
                    .map(|x| x.round())
                    .fold(String::new(), |a, x| format!("{a} {x}")),
                t.compiled
                    .iter()
                    .map(|x| x.round())
                    .fold(String::new(), |a, x| format!("{a} {x}"))
            ));
        }
        if let Some(Verdict::Drop(x)) = r {
            self.lost[k] = true;
            // The copies of a process are one process to the policy (#661): the ones not compiled
            // yet stay interpreted too.
            let base = self.prog.base_of(id);
            for j in 0..self.lost.len() {
                if self.prog.base_of(FuncId(j as u32)) == base && !self.done[j][0] {
                    self.lost[j] = true;
                }
            }
            (self.note)(
                id,
                self.opt,
                Err(format!(
                    "the run progressed {x:.2} times as fast with its compiled code as with the \
                     interpreter, so it was taken back"
                )),
            );
        }
    }

    /// Compile `id` at the first level, or at O2 when `o2`; whether it was compiled.
    fn compile(&mut self, id: FuncId, o2: bool, sampled: bool, part: Part) -> bool {
        // Native-frame code and frame-mode code are modules of their own, so the compilation
        // cache holds each one whatever was compiled with it (#510). Native-frame code is made
        // once.
        if self.lost.get(id.0 as usize).copied().unwrap_or(false) {
            return false;
        }
        let native_done = self.nat.get(id.0 as usize).is_some_and(|n| n.0 != 0);
        if part == Part::Both {
            if !native_done && self.nset(id).contains(&id) {
                let ok = self.compile(id, o2, sampled, Part::Native);
                // Refused (a sampled function too short, say): no frame-mode code either.
                if !ok && sampled && !self.probing {
                    return false;
                }
            }
            let frame = self.compile(id, o2, sampled, Part::Frame);
            return frame || self.nat.get(id.0 as usize).is_some_and(|n| n.0 != 0);
        }
        // Each level once; elaboration code never runs again.
        let first = if part == Part::Native {
            !native_done
        } else {
            self.done
                .get_mut(id.0 as usize)
                .is_some_and(|d| !std::mem::replace(&mut d[usize::from(o2)], true))
        };
        let Some(f) = self
            .prog
            .func(id)
            .filter(|f| first && f.kind != FuncKind::Elab)
        else {
            return false;
        };
        let level = if o2 { OptLevel::O2 } else { self.opt };
        let level = crate::lower::level_for(&self.prog, f, level);
        // The second compile of a function that ran at `Fast` (#715).
        let level = if o2 && level == OptLevel::Fast {
            OptLevel::FastRa
        } else {
            level
        };
        let few = self
            .is_rtl(id)
            .then(|| {
                let seen = self.group[self.prog.base_of(id).0 as usize];
                cold(
                    self.native.instances(self.prog.base_of(id)),
                    seen,
                    self.total,
                )
            })
            .flatten();
        let short = (sampled && !o2)
            .then(|| {
                too_short(&self.prog, f, id)
                    .or_else(|| fused_search(&self.prog, f))
                    .or_else(|| too_many_callbacks(&self.prog, f, id))
                    .or(few)
            })
            .flatten();
        let short_only = short.is_some() && too_big(f, self.max_size).is_none();
        let refused = too_big(f, self.max_size).or(short);
        // A refused function does not open the LLVM library.
        let loaded = if refused.is_none() {
            let loaded = crate::lazy::ensure();
            if !std::mem::replace(&mut self.loaded, true) {
                self.spent += LOAD_US;
            }
            loaded
        } else {
            Ok(())
        };
        if let Err(e) = loaded {
            // Said once, not for every hot function; they all stay interpreted.
            if !std::mem::replace(&mut self.warned, true) {
                eprintln!("warning: --tier=auto: {e}; hot functions stay interpreted");
            }
            (self.note)(id, level, Err(e.to_string()));
            return false;
        }
        let j = if level == OptLevel::Fast {
            &mut self.jitf
        } else if level == OptLevel::FastRa {
            &mut self.jitr
        } else if o2 {
            &mut self.jit2
        } else {
            &mut self.jit
        };
        let t0 = std::time::Instant::now();
        let cpu0 = self.trace.as_ref().and_then(|_| cpu_thread());
        let hits = crate::objcache::cache_stats().0;
        let r = match (refused, j) {
            (Some(why), _) => Err(why),
            (None, Some(j)) => compile_one(j, &self.prog, id, f, level, part),
            (None, j @ None) => match new_jit(level) {
                Ok(n) => compile_one(j.insert(n), &self.prog, id, f, level, part),
                Err(e) => Err(format!("LLVM: {e} (a bug in yavs)")),
            },
        };
        match r {
            Ok(entry) => {
                let took = t0.elapsed();
                let cpu = cpu0
                    .and_then(|c| cpu_thread()?.checked_sub(c))
                    .map_or(0, |d| d.as_micros());
                let hit = crate::objcache::cache_stats().0 != hits;
                // Charged by the model, not by the time it took (#670).
                self.spent += if hit {
                    let per = US_PER_INSTR_HIT;
                    (per * f.code.len() as f64) as u64
                } else {
                    self.price(id, part)
                };
                if let Some(tr) = &self.trace {
                    let seen = self.time.get(id.0 as usize).map_or([0, 0], |t| *t);
                    let leave = (f.kind == FuncKind::Process)
                        .then(|| callbacks(&self.prog, f))
                        .map_or(String::new(), |(o, t)| format!(", {o} of {t} leave"));
                    tr.line(format!(
                        "{id} compiled `{}` ({} IR{leave}) {level:?} {part:?}{} in {} us, cpu {} us (model {}), first seen {} ms and candidate {} ms ago, \
                         seen {}/{} us, spent {} of {} us",
                        f.name,
                        f.code.len(),
                        if hit { " hit" } else { "" },
                        took.as_micros(),
                        cpu,
                        self.cost(id),
                        tr.since(&tr.first, id),
                        tr.since(&tr.cand, id),
                        seen[0],
                        seen[1],
                        self.spent,
                        self.total
                    ));
                }
                // Only frame-mode code is recompiled (#715): a function called natively runs its
                // native-frame code.
                if level == OptLevel::Fast && part != Part::Native && !self.natively(id) {
                    self.fast[id.0 as usize] = true;
                }
                let key = (f.name.clone(), f.code.len() as u32);
                if !hit && crate::objcache::enabled() && self.listed.insert(key) {
                    crate::objcache::add_compiled(&f.name, f.code.len() as u32);
                }
                if entry.native != 0 {
                    // Published when what it calls is compiled (`publish`).
                    self.nat[id.0 as usize] = (entry.native, false);
                    self.calls[id.0 as usize] = entry.call;
                    self.publish();
                }
                if let Some(run) = entry.run {
                    self.table[id.0 as usize].set((run, entry.nregs));
                    self.native.set(id);
                }
                (self.note)(id, level, Ok(took));
                true
            }
            Err(_) if self.probing => {
                if part != Part::Native {
                    self.done[id.0 as usize][usize::from(o2)] = false;
                }
                false
            }
            Err(why) => {
                // A short function stays interpreted as long as nothing compiled calls it;
                // it compiles with a caller (`tree`).
                if short_only && part != Part::Native {
                    self.done[id.0 as usize][0] = false;
                }
                (self.note)(id, level, Err(why));
                false
            }
        }
    }
}

/// Measuring aid: `YAVS_AUTO_FORCE=name@min#first,...` compiles every function of that name with
/// at least `min` IR instructions and an id of at least `first` (copies of processes have the
/// last ids) the first time it is sampled, whatever the budget says.
fn forced(prog: &yavs_ir::Program, id: FuncId) -> bool {
    static LIST: std::sync::OnceLock<Vec<(String, usize, u32)>> = std::sync::OnceLock::new();
    let list = LIST.get_or_init(|| {
        std::env::var("YAVS_AUTO_FORCE")
            .map(|v| {
                v.split(',')
                    .filter_map(|e| {
                        let (n, rest) = e.split_once('@').unwrap_or((e, "0"));
                        let (m, first) = rest.split_once('#').unwrap_or((rest, "0"));
                        Some((n.to_string(), m.parse().ok()?, first.parse().ok()?))
                    })
                    .collect()
            })
            .unwrap_or_default()
    });
    !list.is_empty()
        && prog.func(id).is_some_and(|f| {
            list.iter()
                .any(|(n, m, first)| f.name == *n && f.code.len() >= *m && id.0 >= *first)
        })
}

/// Sleep for `d`, or until the thread is unparked and `halt` is set (the compile thread does that when
/// it ends): ending a run must not wait out a sampling period.
fn nap(sh: &Shared, d: Duration) {
    let end = std::time::Instant::now() + d;
    while !sh.halt.load(Ordering::SeqCst) {
        let left = end.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return;
        }
        std::thread::park_timeout(left);
    }
}

/// The sampler thread (#670): every `period` it reads which function each simulation thread runs and
/// the CPU time they used since the last look, and sends the looks to the compile thread in batches
/// of `BATCH`. It runs while the compile thread compiles or measures a trial (no looks are sent
/// during a trial), so the compile thread's view of the run does not depend on how long it was busy.
/// The CPU time of the simulation threads is the clock of the sampling, not the wall time: a host
/// under load, and the compile thread sharing the core, do not make a function look hot sooner.
/// The compile thread starts it after `START_AFTER` (#426) and ends it.
fn sampler_loop(
    sample: impl Fn(&mut Vec<(FuncId, bool)>),
    period: Duration,
    tx: Sender<Msg>,
    sh: &Shared,
) {
    let clock = || sim_cpu_sampler(sh.compile_clock.load(Ordering::Acquire));
    // Without a CPU clock (not Linux) the wall time between looks is the time.
    let (mut last_cpu, mut last_wall) = (clock(), std::time::Instant::now());
    let mut batch = Vec::with_capacity(BATCH);
    while !sh.halt.load(Ordering::SeqCst) {
        nap(sh, period);
        if sh.halt.load(Ordering::SeqCst) {
            return;
        }
        let (cpu, wall) = (clock(), std::time::Instant::now());
        let dt = match (cpu, last_cpu) {
            (Some(c), Some(l)) => c.saturating_sub(l),
            _ => wall - last_wall,
        };
        (last_cpu, last_wall) = (cpu, wall);
        if sh.trial.load(Ordering::SeqCst) {
            continue;
        }
        let mut running = Vec::new();
        sample(&mut running);
        batch.push(Look {
            dt: u64::try_from(dt.as_micros()).unwrap_or(u64::MAX),
            running,
        });
        if batch.len() == BATCH && tx.send(Msg::Looks(std::mem::take(&mut batch))).is_err() {
            return;
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
        // The LLVM library is opened by the compile thread at the first compile (`Worker::compile`): a
        // run in which nothing gets hot never pays for loading it (about 40 million cycles).
        let prog = rt.prog.clone();
        let funcs: Arc<[Entry]> = (0..prog.funcs.len()).map(|_| Entry::default()).collect();
        let (tx, rx) = channel::<Msg>();
        let shared = Arc::new(Shared {
            finished: (0..prog.funcs.len())
                .map(|_| AtomicBool::new(false))
                .collect(),
            ..Shared::default()
        });
        let ntab: Arc<[AtomicUsize]> = (0..prog.funcs.len()).map(|_| AtomicUsize::new(0)).collect();
        let ctab: Arc<[AtomicUsize]> = (0..prog.funcs.len()).map(|_| AtomicUsize::new(0)).collect();
        let (table, sh) = (funcs.clone(), shared.clone());
        let (sample, sampler_tx) = (rt.sampler(), tx.clone());
        let native = rt.native_flags();
        // How long the compile thread waits for news before it looks at `halt`.
        let period = if heat.eager() {
            Duration::from_micros(1 << 20)
        } else {
            Duration::from_millis(20)
        };
        let sample_every = Duration::from_micros(heat.period.into());
        let mut w = Worker::new(
            prog,
            table,
            ntab.clone(),
            ctab.clone(),
            native,
            heat,
            opt,
            max_size,
            note,
        );
        let worker = std::thread::Builder::new()
            .name("yavs-tier-up".into())
            .spawn(move || {
                // Nothing a run of a few milliseconds samples pays for a compile, and moving the
                // thread, reading the cache's list and waking every millisecond cost a tenth of
                // such a run: do none of it before `START_AFTER` (#426). `stop` ends the wait.
                sh.compile_clock.store(own_cpu_clock(), Ordering::Release);
                let mut pending = Pending::default();
                let mut looks: Vec<Look> = Vec::new();
                let t0 = std::time::Instant::now();
                match rx.recv_timeout(if heat.eager() {
                    Duration::ZERO
                } else {
                    START_AFTER
                }) {
                    Err(RecvTimeoutError::Disconnected) => return,
                    Ok(Msg::Hot(id)) => pending.asked.push_back(id),
                    Ok(Msg::Looks(l)) => looks = l,
                    Err(RecvTimeoutError::Timeout) => {}
                }
                // The sampler of a run that ends within `START_AFTER` is never started.
                let sampler = (!heat.eager() && !sh.halt.load(Ordering::SeqCst)).then(|| {
                    let sh = sh.clone();
                    std::thread::Builder::new()
                        .name("yavs-sampler".into())
                        .spawn(move || {
                            nap(&sh, START_AFTER.saturating_sub(t0.elapsed()));
                            if !sh.halt.load(Ordering::SeqCst) {
                                sampler_loop(sample, sample_every, sampler_tx, &sh);
                            }
                        })
                        .unwrap_or_else(|e| panic!("cannot start the sampler thread: {e}"))
                });
                w.listed = crate::objcache::compiled_list();
                let upgrade = opt != OptLevel::O2;
                // Batches of looks not worked through yet, oldest first.
                let mut inbox: std::collections::VecDeque<Vec<Look>> = Default::default();
                let route =
                    |m: Msg,
                     pending: &mut Pending,
                     inbox: &mut std::collections::VecDeque<Vec<Look>>| match m
                    {
                        Msg::Hot(id) => pending.asked.push_back(id),
                        Msg::Looks(l) => inbox.push_back(l),
                    };
                if !looks.is_empty() {
                    inbox.push_back(std::mem::take(&mut looks));
                }
                loop {
                    while let Ok(m) = rx.try_recv() {
                        route(m, &mut pending, &mut inbox);
                    }
                    if inbox.is_empty() && pending.asked.is_empty() {
                        match rx.recv_timeout(period) {
                            Ok(m) => route(m, &mut pending, &mut inbox),
                            Err(RecvTimeoutError::Timeout) => {}
                            Err(RecvTimeoutError::Disconnected) => break,
                        }
                    }
                    // One batch at a time, each followed by the decisions: what is decided depends
                    // on the looks in order and not on how long a compile took (#670).
                    for look in inbox.pop_front().unwrap_or_default() {
                        w.see(&look, upgrade, &mut pending);
                    }
                    loop {
                        while let Ok(m) = rx.try_recv() {
                            route(m, &mut pending, &mut inbox);
                        }
                        let probe = if pending.asked.is_empty() {
                            pending.probes.pop()
                        } else {
                            None
                        };
                        let Some((ids, o2, sampled)) = probe
                            .map(|id| (vec![(id, Part::Both)], false, true))
                            .or_else(|| {
                                pending
                                    .asked
                                    .pop_front()
                                    .map(|id| (vec![(id, Part::Both)], false, false))
                            })
                            .or_else(|| {
                                pending
                                    .later
                                    .pop()
                                    .map(|id| (vec![(id, Part::Both)], true, false))
                            })
                            .or_else(|| w.pick(&mut pending.cands).map(|ids| (ids, false, true)))
                        else {
                            break;
                        };
                        // `busy` before `halt` here and `halt` before `busy` in `stop` (both
                        // SeqCst): either this thread sees the halt or `stop` sees it busy.
                        sh.busy.store(true, Ordering::SeqCst);
                        if sh.halt.load(Ordering::SeqCst) {
                            break;
                        }
                        if let Some(id) = probe {
                            if w.probe(id) {
                                pending.cands.retain(|&c| c != id);
                                sh.trial.store(true, Ordering::SeqCst);
                                w.trial(id, &sh.halt);
                                sh.trial.store(false, Ordering::SeqCst);
                            }
                            sh.busy.store(false, Ordering::SeqCst);
                            continue;
                        }
                        let first = ids.first().map(|&(id, _)| id);
                        let mut root_done = false;
                        for (i, (id, part)) in ids.into_iter().enumerate() {
                            let done = w.compile(id, o2, sampled && i == 0, part);
                            root_done |= done && i == 0;
                            // Its callees compile for its sake only.
                            if !done && i == 0 {
                                break;
                            }
                            // Eager (tests): swap the O2 code in during the run too.
                            if done && heat.eager() && (upgrade || w.fast[id.0 as usize]) && !o2 {
                                pending.later.push(id);
                            }
                        }
                        if let (Some(id), true) = (first, root_done && sampled && !o2) {
                            sh.trial.store(true, Ordering::SeqCst);
                            w.trial(id, &sh.halt);
                            sh.trial.store(false, Ordering::SeqCst);
                        }
                        if let Some(f) = first.and_then(|id| sh.finished.get(id.0 as usize)) {
                            f.store(true, Ordering::Release);
                        }
                        sh.busy.store(false, Ordering::SeqCst);
                    }
                    if sh.halt.load(Ordering::SeqCst) {
                        break;
                    }
                }
                if let Some(s) = sampler {
                    s.thread().unpark();
                    if let Err(p) = s.join() {
                        resume_unwind(p);
                    }
                }
                if let Some(tr) = &w.trace {
                    let mut v: Vec<usize> = (0..w.time.len()).collect();
                    v.sort_by_key(|&k| std::cmp::Reverse(w.time[k][0] + w.time[k][1]));
                    for &k in v.iter().take(40) {
                        if let Some(f) = w.prog.func(FuncId(k as u32)) {
                            tr.line(format!(
                                "seen f{k} `{}` ({} IR): interpreted {} us, compiled {} us",
                                f.name,
                                f.code.len(),
                                w.time[k][0],
                                w.time[k][1],
                            ));
                        }
                    }
                    tr.line(format!(
                        "sampled {} us, compiled for {} us",
                        w.total, w.spent
                    ));
                }
                *sh.jit.lock().unwrap_or_else(PoisonError::into_inner) = w
                    .jit
                    .into_iter()
                    .chain(w.jit2)
                    .chain(w.jitf)
                    .chain(w.jitr)
                    .collect();
                sh.busy.store(false, Ordering::SeqCst);
            })
            .map_err(|e| Error(format!("cannot start the compile thread: {e}")))?;
        let tier = Arc::new(AutoTier {
            queued: (0..funcs.len()).map(|_| AtomicBool::new(false)).collect(),
            funcs,
            ntab,
            ctab,
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
        // (An empty batch wakes the compile thread, which sees `halt`.)
        if let Some(q) = self
            .queue
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            let _ = q.send(Msg::Looks(Vec::new()));
        }
        if !wait && self.shared.busy.load(Ordering::SeqCst) {
            return false;
        }
        let worker = self
            .worker
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        // (The compile thread ends the sampler.)
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
        enter(f, nregs, pc, host, &lookup, self.ntab.as_ptr().cast(), None)
    }

    fn run_cached(
        &self,
        func: FuncId,
        pc: u32,
        host: &mut dyn TierHost,
        state: &mut Option<Box<yavs_interp::TierState>>,
    ) -> Option<u32> {
        let (f, nregs) = self.funcs.get(func.0 as usize)?.get()?;
        let lookup = |id: FuncId| self.funcs.get(id.0 as usize)?.get();
        enter(
            f,
            nregs,
            pc,
            host,
            &lookup,
            self.ntab.as_ptr().cast(),
            Some(state),
        )
    }

    fn ready(&self, func: FuncId) -> bool {
        self.funcs
            .get(func.0 as usize)
            .is_some_and(|f| f.run.load(Ordering::Relaxed) != 0)
    }

    fn call(
        &self,
        func: FuncId,
        cells: &[u64],
        obj: Option<u64>,
        pc: u32,
        host: &mut dyn TierHost,
    ) -> Option<yavs_interp::Called> {
        let c = self.ctab.get(func.0 as usize)?.load(Ordering::Acquire);
        let lookup = |id: FuncId| self.funcs.get(id.0 as usize)?.get();
        call(c, cells, obj, pc, host, &lookup, self.ntab.as_ptr().cast())
    }

    fn hot(&self, func: FuncId) {
        let Some(q) = self.queued.get(func.0 as usize) else {
            return;
        };
        if !q.swap(true, Ordering::Relaxed)
            && let Some(q) = &*self.queue.lock().unwrap_or_else(PoisonError::into_inner)
        {
            // The worker is gone only after `stop`: nothing to compile any more.
            let _ = q.send(Msg::Hot(func));
        }
        // Eager mode is for tests: the function runs compiled from this call on (or is known
        // to stay interpreted), so a run cannot end before its compile and the notes.
        // ponytail: polls; a condvar if a test ever needs thousands of functions.
        let done = &self.shared.finished[func.0 as usize];
        while !done.load(Ordering::Acquire) && !self.shared.halt.load(Ordering::SeqCst) {
            // A compile thread that died (a panic) never finishes it.
            let dead = self
                .worker
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .as_ref()
                .is_none_or(JoinHandle::is_finished);
            if dead {
                break;
            }
            std::thread::sleep(Duration::from_micros(50));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        BY_ENTRIES, BY_RUNS, COMPUTE_BUDGET, NATIVE_BUDGET, UNSEEN_MIN, band_wait, cold, compute,
        decide, first_fit, fused_search, left_out, loops, must_calls, run_trial,
        too_many_callbacks, within_budget,
    };
    use yavs_diag::{FileId, Span};
    use yavs_ir::{Inst, ProgramBuilder, StmtKind};

    /// A function whose second block jumps to itself: reached from the first, that is a loop;
    /// left unreachable by the optimiser, it is none (a process full of dead self-jumps was
    /// kept out of tier-up).
    fn self_jump(reached: bool) -> (yavs_ir::Program, yavs_ir::FuncId) {
        let mut pb = ProgramBuilder::new();
        let f = pb.declare("f", yavs_ir::FuncKind::Procedure, None);
        let mut fb = pb.define(f);
        let dead = fb.block();
        fb.stmt(StmtKind::Return, Span::new(FileId(0), 0, 1));
        if reached {
            fb.emit(Inst::Br { to: dead });
        } else {
            fb.emit(Inst::Ret { v: None });
        }
        fb.at(dead);
        fb.emit(Inst::Br { to: dead });
        fb.finish();
        (pb.finish().unwrap(), f)
    }

    /// A function of `n` frees whose last block loops on itself: a compute loop for
    /// `compute`, whatever its size.
    fn padded_loop(n: usize) -> (yavs_ir::Program, yavs_ir::FuncId) {
        let mut pb = ProgramBuilder::new();
        let f = pb.declare("f", yavs_ir::FuncKind::Procedure, None);
        let mut fb = pb.define(f);
        let spin = fb.block();
        fb.stmt(StmtKind::Return, Span::new(FileId(0), 0, 1));
        let ptr = fb.reg(yavs_ir::RegTy::Ptr);
        fb.emit(Inst::AddrOf {
            dst: ptr,
            space: yavs_ir::AddrSpace::Frame,
            off: 0,
        });
        for _ in 0..n {
            fb.emit(Inst::Free { ptr });
        }
        fb.emit(Inst::Br { to: spin });
        fb.at(spin);
        fb.emit(Inst::Br { to: spin });
        fb.finish();
        (pb.finish().unwrap(), f)
    }

    /// A function of 1 500 IR instructions or more is compiled at `Fast` even when its loops make
    /// it a compute loop: optimised code generation of such a function takes minutes (#630).
    #[test]
    fn a_huge_function_with_a_short_loop_is_compiled_fast() {
        for (n, level) in [(400, crate::OptLevel::O2), (1_600, crate::OptLevel::Fast)] {
            let (prog, f) = padded_loop(n);
            let f = prog.func(f).unwrap();
            assert!(f.code.len() >= n, "{n}");
            assert!(compute(&prog, f), "{n}");
            assert_eq!(
                crate::lower::level_for(&prog, f, crate::OptLevel::O2),
                level,
                "{n}"
            );
        }
    }

    /// `f` calls `always` before a branch, `rare` in one arm and `each` in both: only the call
    /// every path passes counts (`must_calls`), as for a function that runs per call.
    #[test]
    fn calls_on_every_path_are_must_calls() {
        use yavs_ir::{AddrSpace, Mode, ParamClass, Width};
        let mut pb = ProgramBuilder::new();
        let ids: Vec<_> = ["f", "always", "rare", "each"]
            .iter()
            .map(|n| pb.declare(n, yavs_ir::FuncKind::Procedure, None))
            .collect();
        for &g in &ids[1..] {
            let mut fb = pb.define(g);
            fb.stmt(StmtKind::Return, Span::new(FileId(0), 0, 1));
            // Something of its own, so the optimiser keeps the call.
            let keep = fb.slot(8, 8);
            let at = fb.addr(AddrSpace::Frame, keep);
            fb.emit(Inst::Store {
                addr: at,
                src: at,
                w: Width::B8,
            });
            fb.emit(Inst::Ret { v: None });
            fb.finish();
        }
        let mut fb = pb.define(ids[0]);
        let slot = fb.param("a", Width::B8, ParamClass::Value, Mode::In, None);
        let (a, b, end) = (fb.block(), fb.block(), fb.block());
        fb.stmt(StmtKind::Return, Span::new(FileId(0), 0, 1));
        let p = fb.addr(AddrSpace::Frame, slot);
        let v = fb.reg(yavs_ir::RegTy::I64);
        fb.emit(Inst::Load {
            dst: v,
            addr: p,
            w: Width::B8,
        });
        let zero = fb.int(0);
        let c = fb.reg(yavs_ir::RegTy::I64);
        fb.emit(Inst::Cmp {
            op: yavs_ir::CmpOp::Eq,
            dst: c,
            a: v,
            b: zero,
        });
        let call = |fb: &mut yavs_ir::FuncBuilder<'_>, func| {
            let args = fb.regs(&[]);
            fb.emit(Inst::Call {
                func,
                args,
                dst: None,
            });
        };
        call(&mut fb, ids[1]);
        fb.emit(Inst::CondBr { c, t: a, f: b });
        fb.at(a);
        call(&mut fb, ids[2]);
        call(&mut fb, ids[3]);
        fb.emit(Inst::Br { to: end });
        fb.at(b);
        call(&mut fb, ids[3]);
        fb.emit(Inst::Br { to: end });
        fb.at(end);
        fb.emit(Inst::Ret { v: None });
        fb.finish();
        let prog = pb.finish().unwrap();
        let calls = must_calls(&prog, prog.func(ids[0]).unwrap());
        assert_eq!(calls, vec![ids[1], ids[3]]);
    }

    #[test]
    fn an_unreachable_self_jump_is_no_loop() {
        for (reached, is_loop) in [(true, true), (false, false)] {
            let (prog, f) = self_jump(reached);
            assert_eq!(loops(&prog, prog.func(f).unwrap()), is_loop, "{reached}");
        }
    }

    /// A loop of inline additions entered through a preheader block appended after the code (as
    /// loop-invariant code motion makes them), followed by code the compiled code hands to the
    /// interpreter: the jump from the preheader to the loop is no loop, so the code after the
    /// loop does not count against it (#547).
    #[test]
    fn a_preheader_appended_after_the_code_is_no_loop() {
        let mut pb = ProgramBuilder::new();
        let f = pb.declare("f", yavs_ir::FuncKind::Procedure, None);
        let mut fb = pb.define(f);
        let head = fb.block();
        let exit = fb.block();
        let pre = fb.block();
        fb.stmt(StmtKind::Return, Span::new(FileId(0), 0, 1));
        let x = fb.int(1);
        let ptr = fb.reg(yavs_ir::RegTy::Ptr);
        fb.emit(Inst::AddrOf {
            dst: ptr,
            space: yavs_ir::AddrSpace::Frame,
            off: 0,
        });
        fb.emit(Inst::Br { to: pre });
        fb.at(head);
        for _ in 0..20 {
            fb.emit(Inst::Bin {
                op: yavs_ir::IntOp::Add,
                dst: x,
                a: x,
                b: x,
            });
        }
        let (y, c) = (fb.reg(yavs_ir::RegTy::I64), fb.reg(yavs_ir::RegTy::I64));
        fb.emit(Inst::Load {
            dst: y,
            addr: ptr,
            w: yavs_ir::Width::B8,
        });
        fb.emit(Inst::Cmp {
            op: yavs_ir::CmpOp::Lt,
            dst: c,
            a: x,
            b: y,
        });
        fb.emit(Inst::CondBr {
            c,
            t: exit,
            f: head,
        });
        fb.at(exit);
        for _ in 0..10 {
            fb.emit(Inst::Free { ptr });
        }
        fb.emit(Inst::Ret { v: None });
        fb.at(pre);
        fb.emit(Inst::Br { to: head });
        fb.finish();
        let prog = pb.finish().unwrap();
        assert!(compute(&prog, prog.func(f).unwrap()));
    }

    /// A process whose activation (after its `Suspend`) runs `ops` inline additions, an address
    /// and `outs` instructions the compiled code hands to the interpreter (`Free`, analysed only).
    fn process(ops: usize, outs: usize) -> (yavs_ir::Program, yavs_ir::FuncId) {
        let mut pb = ProgramBuilder::new();
        let f = pb.declare("p", yavs_ir::FuncKind::Process, None);
        let mut fb = pb.define(f);
        let body = fb.block();
        fb.stmt(StmtKind::Wait, Span::new(FileId(0), 0, 1));
        fb.emit(Inst::Br { to: body });
        fb.at(body);
        fb.emit(Inst::Suspend { resume: 0 });
        let x = fb.int(1);
        for _ in 0..ops {
            fb.emit(Inst::Bin {
                op: yavs_ir::IntOp::Add,
                dst: x,
                a: x,
                b: x,
            });
        }
        let ptr = fb.reg(yavs_ir::RegTy::Ptr);
        fb.emit(Inst::AddrOf {
            dst: ptr,
            space: yavs_ir::AddrSpace::Frame,
            off: 0,
        });
        for _ in 0..outs {
            fb.emit(Inst::Free { ptr });
        }
        fb.emit(Inst::Br { to: body });
        fb.finish();
        (pb.finish().unwrap(), f)
    }

    /// A loop that reads one array position per iteration and compares (a search) is the loop
    /// the interpreter fuses: left interpreted (#573). The same loop that also stores is not.
    #[test]
    fn a_search_loop_stays_interpreted() {
        for (stores, refused) in [(false, true), (true, false)] {
            let mut pb = ProgramBuilder::new();
            let f = pb.declare("f", yavs_ir::FuncKind::Function, None);
            let what = pb.check_msg("index of s", yavs_ir::Dir::To);
            let mut fb = pb.define(f);
            let (head, hit, next, step, exit) =
                (fb.block(), fb.block(), fb.block(), fb.block(), fb.block());
            fb.stmt(StmtKind::Return, Span::new(FileId(0), 0, 1));
            let i = fb.int(1);
            let (dope, base) = (fb.reg(yavs_ir::RegTy::Ptr), fb.reg(yavs_ir::RegTy::Ptr));
            let want = fb.int(65);
            let (p, a, ch) = (
                fb.reg(yavs_ir::RegTy::I64),
                fb.reg(yavs_ir::RegTy::Ptr),
                fb.reg(yavs_ir::RegTy::I64),
            );
            let (c1, c2) = (fb.reg(yavs_ir::RegTy::I64), fb.reg(yavs_ir::RegTy::I64));
            for (r, off) in [(dope, 0), (base, 0)] {
                fb.emit(Inst::AddrOf {
                    dst: r,
                    space: yavs_ir::AddrSpace::Frame,
                    off,
                });
            }
            fb.emit(Inst::Br { to: head });
            fb.at(head);
            fb.emit(Inst::Pos {
                dst: p,
                dope,
                dim: 0,
                idx: i,
                what,
            });
            fb.emit(Inst::Index {
                dst: a,
                base,
                pos: p,
                stride: 1,
            });
            fb.emit(Inst::Load {
                dst: ch,
                addr: a,
                w: yavs_ir::Width::B1,
            });
            fb.emit(Inst::Cmp {
                op: yavs_ir::CmpOp::Eq,
                dst: c1,
                a: ch,
                b: want,
            });
            fb.emit(Inst::CondBr {
                c: c1,
                t: hit,
                f: next,
            });
            fb.at(next);
            if stores {
                fb.emit(Inst::Store {
                    addr: a,
                    src: want,
                    w: yavs_ir::Width::B1,
                });
            }
            fb.emit(Inst::Cmp {
                op: yavs_ir::CmpOp::Eq,
                dst: c2,
                a: i,
                b: want,
            });
            fb.emit(Inst::CondBr {
                c: c2,
                t: exit,
                f: step,
            });
            fb.at(step);
            fb.emit(Inst::Bin {
                op: yavs_ir::IntOp::Add,
                dst: i,
                a: i,
                b: want,
            });
            fb.emit(Inst::Br { to: head });
            fb.at(hit);
            fb.emit(Inst::Ret { v: Some(i) });
            fb.at(exit);
            fb.emit(Inst::Ret { v: Some(want) });
            fb.finish();
            let prog = pb.finish().unwrap();
            let why = fused_search(&prog, prog.func(f).unwrap());
            assert_eq!(why.is_some(), refused, "stores {stores}: {why:?}");
        }
    }

    /// A process whose activation leaves the compiled code for more than one instruction in ten
    /// stays interpreted (#507); one with fewer is compiled.
    #[test]
    fn a_process_that_leaves_its_compiled_code_often_stays_interpreted() {
        for (ops, outs, refused) in [(100, 5, false), (50, 9, true), (0, 1, true)] {
            let (prog, f) = process(ops, outs);
            let why = too_many_callbacks(&prog, prog.func(f).unwrap(), f);
            assert_eq!(why.is_some(), refused, "{ops} {outs}: {why:?}");
        }
    }

    /// The signal operations compiled code answers in place (reads, `'event`, the wait that ends an
    /// activation) are no callbacks (#633); `'last_event` and a wait in the middle are.
    #[test]
    fn signal_operations_answered_in_place_do_not_count_as_callbacks() {
        use yavs_ir::{AttrKind, Inst, RegTy};
        for (inplace, refused) in [(true, false), (false, true)] {
            let mut pb = ProgramBuilder::new();
            let f = pb.declare("p", yavs_ir::FuncKind::Process, None);
            let mut fb = pb.define(f);
            let body = fb.block();
            fb.stmt(StmtKind::Wait, Span::new(FileId(0), 0, 1));
            fb.emit(Inst::Br { to: body });
            fb.at(body);
            fb.emit(Inst::Suspend { resume: 0 });
            let (s, d, ptr) = (fb.reg(RegTy::Sig), fb.reg(RegTy::I64), fb.reg(RegTy::Ptr));
            let (off, len) = (fb.int(0), fb.int(1));
            fb.emit(Inst::AddrOf {
                dst: ptr,
                space: yavs_ir::AddrSpace::Frame,
                off: 0,
            });
            fb.emit(Inst::SigVal {
                dst: s,
                sig: 0,
                offset: off,
                len,
            });
            for _ in 0..4 {
                fb.emit(Inst::SigReadScalar { dst: d, sig: s });
                fb.emit(Inst::SigRead { dst: ptr, sig: s });
                fb.emit(Inst::SigAttr {
                    dst: d,
                    kind: if inplace {
                        AttrKind::Event
                    } else {
                        AttrKind::LastEvent
                    },
                    sig: s,
                });
            }
            fb.emit(Inst::WaitOn { sig: s });
            fb.emit(Inst::Suspend { resume: 1 });
            fb.emit(Inst::Br { to: body });
            fb.finish();
            let prog = pb.finish().unwrap();
            let why = too_many_callbacks(&prog, prog.func(f).unwrap(), f);
            assert_eq!(why.is_some(), refused, "{inplace}: {why:?}");
        }
    }

    /// Compiled code stays when the run progressed faster with it (by a margin), goes when it did
    /// not; the first pair of windows decides only when it is clear (#596).
    #[test]
    fn a_trial_decides_on_the_progress_ratio() {
        let decide_entries = |c: &[f64], i: &[f64]| decide(c, i, &BY_ENTRIES);
        use super::Verdict::{Drop, Keep};
        let decide = |c: &[f64], i: &[f64]| decide(c, i, &BY_RUNS);
        assert_eq!(decide(&[], &[]), None);
        assert_eq!(decide(&[100.0], &[100.0]), None);
        assert_eq!(decide(&[100.0], &[70.0]), Some(Keep(100.0 / 70.0)));
        assert_eq!(decide(&[80.0], &[100.0]), Some(Drop(0.8)));
        // Two pairs near parity: a third.
        assert_eq!(decide(&[100.0, 101.0], &[100.0, 100.0]), None);
        assert_eq!(decide(&[120.0, 120.0], &[100.0, 100.0]), Some(Keep(1.2)));
        assert_eq!(decide(&[92.0, 92.0], &[100.0, 100.0]), Some(Drop(0.92)));
        assert_eq!(
            decide(&[103.0, 103.0, 103.0], &[100.0, 100.0, 100.0]),
            Some(Drop(1.03))
        );
        assert_eq!(
            decide(&[106.0, 106.0, 106.0], &[100.0, 100.0, 100.0]),
            Some(Keep(1.06))
        );
        // The entries of a function: parity stays, a clear loss goes.
        let entries = |c: f64| decide_entries(&[c, c, c], &[100.0, 100.0, 100.0]);
        assert_eq!(entries(98.0), Some(Keep(0.98)));
        assert_eq!(entries(93.0), Some(Drop(0.93)));
    }

    /// The windows of a trial swap the code in turn and measure the run's progress each time: with
    /// progress that depends on the code, the verdict follows; without progress there is none.
    #[test]
    fn a_trial_swaps_in_turn_and_measures_the_run() {
        use super::Verdict;
        use std::cell::Cell;
        use std::time::Instant;
        for (rate_compiled, keep) in [(1.5e6, true), (0.7e6, false)] {
            let compiled = Cell::new(true);
            let acc = Cell::new((0.0f64, Instant::now()));
            let progress = || {
                let (p, t) = acc.get();
                let now = Instant::now();
                let rate = if compiled.get() { rate_compiled } else { 1.0e6 };
                let p = p + (now - t).as_secs_f64() * rate;
                acc.set((p, now));
                p as u64
            };
            let mut swap = |on: bool| {
                progress();
                compiled.set(on);
            };
            let r = run_trial(&mut swap, &progress, &|| None, &|| false, &BY_RUNS).verdict;
            assert_eq!(matches!(r, Some(Verdict::Keep(_))), keep, "{r:?}");
            assert!(r.is_some());
        }
        let mut swap = |_: bool| {};
        let t = run_trial(&mut swap, &|| 0, &|| None, &|| true, &BY_RUNS);
        assert!(t.verdict.is_none());
    }

    /// A trial that counts the entries of a function takes all three pairs of windows, however
    /// clear the first one is: its windows vary with the burst they fall in.
    #[test]
    fn a_trial_of_entries_takes_three_pairs() {
        use std::cell::Cell;
        use std::time::Instant;
        for (rule, pairs) in [(&BY_RUNS, 1), (&BY_ENTRIES, 3)] {
            let compiled = Cell::new(true);
            let acc = Cell::new((0.0f64, Instant::now()));
            let progress = || {
                let (p, t) = acc.get();
                let now = Instant::now();
                let rate = if compiled.get() { 2.0e6 } else { 1.0e6 };
                let p = p + (now - t).as_secs_f64() * rate;
                acc.set((p, now));
                p as u64
            };
            let mut swap = |on: bool| {
                progress();
                compiled.set(on);
            };
            let t = run_trial(&mut swap, &progress, &|| None, &|| false, rule);
            assert_eq!(t.compiled.len(), pairs);
            assert!(matches!(t.verdict, Some(super::Verdict::Keep(_))));
        }
    }

    /// A clocked process stays interpreted unless it has many instances and is a good part of
    /// the run: the processes of a CPU design are a few percent of it each (#532).
    #[test]
    fn a_clocked_process_that_is_a_small_part_of_the_run_stays_interpreted() {
        assert!(cold(4096, 44_000, 44_000).is_none());
        assert!(cold(64, 23_000, 44_000).is_none());
        assert!(cold(32, 11_700, 310_000).is_some());
        assert!(cold(1, 44_000, 44_000).is_some());
    }

    /// A clocked process outside the band of bodies that gained is tried only in a run that used it
    /// for a quarter of a second (#575, #596).
    #[test]
    fn a_clocked_process_outside_the_band_waits_for_a_quarter_second() {
        for (len, wait) in [
            (120, 250_000),
            (159, 250_000),
            (160, 0),
            (300, 0),
            (301, 250_000),
            (600, 250_000),
        ] {
            assert_eq!(band_wait(len), wait, "{len}");
        }
    }

    /// A process that interprets one loop for seconds without ever suspending: the sampler sees
    /// all of it, and a compute loop of a few hundred instructions (about 180 ms to compile)
    /// is compiled once it was seen for as long as the compile takes; other code still waits for
    /// the plain 2 percent, and nothing is compiled before it could repay.
    #[test]
    fn long_compute_loop_fits_the_budget() {
        let cost = 177_000;
        assert!(!within_budget(2, COMPUTE_BUDGET, 0, cost, 150_000));
        assert!(within_budget(2, COMPUTE_BUDGET, 0, cost, 180_000));
        assert!(!within_budget(2, 1, 0, cost, 1_500_000));
        assert!(within_budget(2, 1, 0, cost, 9_000_000));
        assert!(within_budget(0, 1, 0, cost, 0));
    }

    /// Only a callee that was never seen running, with a budget, and not small is left out.
    #[test]
    fn an_unseen_big_callee_is_left_out() {
        assert!(left_out(2, 0, UNSEEN_MIN + 1));
        assert!(!left_out(2, 0, UNSEEN_MIN));
        assert!(!left_out(2, 1, 500));
        assert!(!left_out(0, 0, 500));
    }

    /// A candidate the budget has no room for must not hold up a cheaper one behind it (#572):
    /// 32 percent (2 times `NATIVE_BUDGET`) of 200 ms sampled is 64 ms of compiling.
    #[test]
    fn a_candidate_without_room_does_not_hold_up_the_next() {
        let f = NATIVE_BUDGET;
        let big = (80_000, f);
        let small = (10_000, f);
        assert_eq!(first_fit(&[big, small], 2, 0, 200_000), Ok(1));
        assert_eq!(first_fit(&[small, big], 2, 0, 200_000), Ok(0));
        // Nothing fits: each says at which sampled time it will, the budget grows with the run.
        assert_eq!(
            first_fit(&[big, (70_000, f)], 2, 0, 200_000),
            Err(vec![250_000, 218_750])
        );
        // What was spent counts: 60 ms spent leave no room for 10 ms before 218.75 ms.
        assert_eq!(first_fit(&[small], 2, 60_000, 200_000), Err(vec![218_750]));
        assert_eq!(first_fit(&[small], 2, 60_000, 220_000), Ok(0));
    }
    /// A worker over the program of `padded_loop(n)`.
    fn worker(n: usize) -> (super::Worker, yavs_ir::FuncId) {
        let (prog, f) = padded_loop(n);
        let rt = yavs_interp::Runtime::new(std::sync::Arc::new(prog), Default::default()).unwrap();
        let table: std::sync::Arc<[super::Entry]> = (0..rt.prog.funcs.len())
            .map(|_| super::Entry::default())
            .collect();
        let tab = || -> std::sync::Arc<[std::sync::atomic::AtomicUsize]> {
            (0..rt.prog.funcs.len())
                .map(|_| std::sync::atomic::AtomicUsize::new(0))
                .collect()
        };
        let w = super::Worker::new(
            rt.prog.clone(),
            table,
            tab(),
            tab(),
            rt.native_flags(),
            super::Heat::DEFAULT,
            crate::OptLevel::O2,
            usize::MAX,
            Box::new(|_, _, _| {}),
        );
        (w, f)
    }

    /// What becomes hot, and when, is a function of the looks of the sampler and the program, and
    /// of nothing else (#670): not of how fast the looks are worked through, how they are batched,
    /// or what was compiled meanwhile. The compile cost is a fixed price of the function.
    #[test]
    fn what_is_hot_depends_on_the_looks_only() {
        let look = |ms: u64| super::Look {
            dt: ms * 1000,
            running: vec![(yavs_ir::FuncId(0), false)],
        };
        let hot_after = |pause: bool| {
            let (mut w, f) = worker(100);
            let price = w.prog.func(f).unwrap().code.len() as u64 * 800;
            assert_eq!(w.cost(f), price, "a compute loop has a fixed price");
            assert_eq!(
                w.payback_cost(f),
                w.prog.func(f).unwrap().code.len() as u64 * 800
            );
            let mut pending = super::Pending::default();
            for n in 1.. {
                if pause && n % 7 == 0 {
                    std::thread::sleep(std::time::Duration::from_millis(3));
                }
                w.see(&look(1), false, &mut pending);
                if !pending.cands.is_empty() {
                    assert_eq!(pending.cands, [f]);
                    assert_eq!(w.cost(f), price, "{n}");
                    return (n, w.total, w.payback_cost(f));
                }
                assert!(n < 100_000);
            }
            unreachable!()
        };
        let (a, b) = (hot_after(false), hot_after(true));
        assert_eq!(a, b);
        // A compute loop is compiled once seen for twice what the payback rule holds it to cost.
        assert!((2 * a.2..2 * a.2 + 1000).contains(&a.1), "{a:?}");
        assert_eq!(a.0 * 1000, a.1);
    }

    /// The CPU time of a look is shared by the functions running in it, and the time seen running
    /// compiled code is kept apart.
    #[test]
    fn a_look_shares_its_time_between_the_running_functions() {
        let (mut w, f) = worker(100);
        let mut pending = super::Pending::default();
        let look = super::Look {
            dt: 1000,
            running: vec![(f, false), (f, true)],
        };
        w.see(&look, false, &mut pending);
        assert_eq!(w.time[f.0 as usize], [500, 500]);
        assert_eq!(w.total, 1000);
    }
    /// A function that ran at `Fast` is compiled again at `FastRa` once it was seen running compiled
    /// for `PAYBACK_RA` times that compile, in a run that does not recompile at O2 (#715); one that
    /// did not run at `Fast` is not.
    #[test]
    fn a_fast_function_is_recompiled_when_the_register_allocator_repays() {
        let (mut w, f) = worker(1_600);
        let at = super::PAYBACK_RA * super::ra_cost(w.prog.func(f).unwrap().code.len());
        let look = |us: u64| super::Look {
            dt: us,
            running: vec![(f, true)],
        };
        let mut pending = super::Pending::default();
        w.see(&look(at), false, &mut pending);
        assert!(pending.later.is_empty(), "not a function that ran at Fast");
        w.time[f.0 as usize] = [0, 0];
        w.fast[f.0 as usize] = true;
        assert_eq!(w.threshold(f, true), at);
        w.see(&look(at - 1), false, &mut pending);
        assert!(pending.later.is_empty());
        w.see(&look(1), false, &mut pending);
        assert_eq!(pending.later, [f]);
        w.see(&look(at), false, &mut pending);
        assert_eq!(pending.later, [f], "asked once");
    }
}
