//! `--tier=auto`: compile the functions where the interpreter spends its time, one at a time
//! on one background thread, while the simulation keeps running them in the interpreter
//! (#125).
//!
//! The thread samples what the simulation threads interpret (`Runtime::sampler`) every
//! [`Heat::period`] microseconds and compiles a function once it was seen for about as long
//! as compiling it takes ([`Heat`]); the function is compiled in a module of its own and
//! published through an atomic entry.
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
//!
//! With Cranelift as the only generator the count rule comes on top (`Worker::waits`, [`count_rule_on`],
//! docs/decisions.md "`--tier=auto`: the count rule"): the interpreter counts the entries of each
//! function and asks for it (`Tier::asked`); the looks keep deciding what the count does not reach.

use std::panic::resume_unwind;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;
use std::time::Duration;

use yavs_interp::{CountStat, Runtime, Tier, TierHost};
use yavs_ir::{FuncId, FuncKind};

use crate::generator::{Generator, StartError};
use crate::native::{Part, native_set, too_big};
use crate::profile::{self, Profile, Snapshot};
use crate::rt::{RunFn, back_edges, call, enter, loops};

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
    /// interpreted for `Model::payback` times as long as compiling it takes.
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

    /// Whether every function is compiled when the interpreter enters it.
    pub fn eager(self) -> bool {
        self.period == 0
    }
}

/// Why `f` stays interpreted although hot: `rt::too_short` (shorter than `SHORT` IR instructions
/// and without a jump back, so each entry into compiled code from the interpreter would cost about
/// what interpreting its body costs: the small helpers a testbench calls millions of times ran
/// slower compiled, #432), except a function the interpreter calls on a native frame once
/// compiled (`called_natively`): that call costs less than its own.
fn too_short(prog: &yavs_ir::Program, f: &yavs_ir::Func, id: FuncId) -> Option<String> {
    crate::rt::too_short(prog, f).filter(|_| !called_natively(prog, f, id))
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

/// The share of `f`'s IR instructions inside its loops (`back_edges`), in percent.
fn loop_share(prog: &yavs_ir::Program, f: &yavs_ir::Func) -> usize {
    let mut inside = vec![false; f.code.len()];
    if loops(prog, f) {
        for r in back_edges(prog, f) {
            inside[r].fill(true);
        }
    }
    inside.iter().filter(|&&b| b).count() * 100 / f.code.len().max(1)
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
        .filter(|&pc| inside[pc] && crate::native::leaves(prog, f, pc, &nc))
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
fn callbacks(prog: &yavs_ir::Program, f: &yavs_ir::Func, same: bool) -> (usize, usize) {
    let (out, total) = leaving(prog, f, same);
    (out.len(), total)
}

/// The kinds of the instructions `callbacks` counts as leaving, most frequent first (trace only).
fn leave_kinds(prog: &yavs_ir::Program, f: &yavs_ir::Func, same: bool) -> String {
    let mut m = std::collections::BTreeMap::<String, usize>::new();
    for pc in leaving(prog, f, same).0 {
        let d = format!("{:?}", f.code[pc]);
        *m.entry(d.split([' ', '{', '(']).next().unwrap_or("").to_string())
            .or_default() += 1;
    }
    let mut v: Vec<_> = m.into_iter().collect();
    v.sort_by_key(|a| std::cmp::Reverse(a.1));
    v.iter()
        .map(|(k, n)| format!("{k} {n}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// The pcs `callbacks` counts as leaving, and the instructions an activation runs. With `same`
/// a wait compiled code skips (`yavs_rt_wait_same`) counts once, not once per `WaitOn`.
fn leaving(prog: &yavs_ir::Program, f: &yavs_ir::Func, same: bool) -> (Vec<usize>, usize) {
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
    let skipped: Vec<(usize, usize)> = if same {
        yavs_interp::static_wait(prog, f, 1)
    } else {
        Vec::new()
    };
    // The first `WaitOn` of such a wait still counts: `yavs_rt_wait_same` is a call out too.
    let skip = |pc: usize| {
        matches!(f.code[pc], Inst::WaitOn { .. })
            && skipped.iter().any(|&(st, s)| {
                st < pc
                    && pc < s
                    && f.code[st + 1..pc]
                        .iter()
                        .any(|i| matches!(i, Inst::WaitOn { .. }))
            })
    };
    let out = (0..n)
        .filter(|&pc| {
            seen[pc]
                && !skip(pc)
                && (crate::native::leaves(prog, f, pc, &nc)
                    || roots && matches!(f.code[pc], Inst::Stmt))
        })
        .collect();
    (out, seen.iter().filter(|&&b| b).count())
}

/// Why hot process `f` stays interpreted: more than one in `CALLBACKS` of the instructions an
/// activation runs leave the compiled code (`callbacks`), and each way out and back costs more
/// than interpreting the instruction, so the compiled process ran slower than the interpreted
/// one (#507). ponytail: a static count over every path of the body, not weighted by what runs;
/// a measured comparison of both forms if a process near the line needs it.
fn too_many_callbacks(
    prog: &yavs_ir::Program,
    f: &yavs_ir::Func,
    id: FuncId,
    same: bool,
) -> Option<String> {
    if f.kind == FuncKind::Process {
        let (out, total) = callbacks(prog, f, same);
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
        .filter(|&pc| crate::native::leaves(prog, f, pc, &nc))
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

/// A clocked process (`rtl`) seen for less than this part of the sampled time stays interpreted
/// (`cold`): between its activations so much other code runs that its compiled code starts cold
/// every time. The processes of 16 to 63 instances compiled in the CPU designs, a wide-vector
/// design and two testbenches (0.2 to 6.5 percent of the run when decided) each cost 200 to 900
/// instructions more per activation than interpreted; the processes that gain are half of the
/// run or more (#532). ponytail: the share when the decision is made, early in the run; a
/// measured comparison per process if a design sits near the line.
const RTL_SHARE: u64 = 8;
/// A subprogram whose count was spent by looks joins the profile when it took at least this part
/// of the sampled time: compiled early in the next run, a smaller one moves that run's compile
/// budget for nothing (`uvvm_axistream_vvc`: one at 7 percent made run 3 recompile 110 ms that
/// run 2 no longer reached).
const LOOP_SHARE: u64 = 8;

/// Why an `rtl` process seen for `seen` of `total` sampled microseconds would run its compiled code
/// cold, so slower than interpreted (`Tune::share`).
fn cold(seen: u64, total: u64, t: Tune) -> Option<String> {
    (seen.saturating_mul(t.share) < total).then(|| {
        format!(
            "a clocked process seen for {seen} of {total} us: compiled code of a process that is \
             a small part of the run ran slower (its code does not stay in the caches)"
        )
    })
}

/// The rules for clocked processes (`rtl`) of the code generator. Cranelift makes its code fast and
/// the trial takes back what does not gain, so it takes short processes and processes of few
/// instances and of a small part of the run: on the CPU-like designs these compile when they are
/// seen long enough to repay the translation, which the payback rule decides.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct Tune {
    /// The smallest process `rtl` takes: below this the way into and out of compiled code costs
    /// more than interpreting the body (decisions.md "round 3"); Cranelift's code ran `rtl_ff`'s
    /// 136-instruction process in fewer instructions than the interpreter.
    rtl_min: usize,
    /// Whether a process outside `RTL_BAND` waits `BAND_WAIT` (`band_wait`).
    band: bool,
    /// A clocked process of fewer instances than this is `Kind::RtlFew`: compiled, the processes
    /// of a CPU design (most made once) ran 6 to 8 percent slower in cycles at the same
    /// instructions, those of designs with many instances of one process 7 to 34 percent faster
    /// (#510). So it is expected to gain little and the measure after the swap (`Worker::trial`)
    /// takes the code back when it does not pay. ponytail: the instance count stands for how much
    /// an activation's code is reused. With `share` (`RTL_SHARE`) in `cold`.
    instances: u32,
    share: u64,
    /// How many times the price of the callees a compile makes with its root (`Worker::tree`) the
    /// root must have been seen interpreted (`repaid`), on top of the payback of its own price (not
    /// for a nested subprogram entered in frame mode, which has no native variant); 0:
    /// the callees are not counted. A first-level generator makes a root's callees cheaply and
    /// they gained nothing in instructions on the verification libraries (#718).
    tree_payback: u64,
    /// How many times its price a clocked process (`rtl`) is seen interpreted before it compiles,
    /// on top of `band_wait`: its compiled code gained 5 percent in the trials that kept it.
    payback_rtl: u64,
    /// A large subprogram (`native::big`) is `Kind::Big` or `BigLoop`. Not a process: Cranelift's code of
    /// large processes gains (`fletcher_bufferwriter_512`'s `input_proc`, 3.7 percent of the run).
    big: bool,
    /// Whether a `Kind::Frame` process is compiled at all (its callers are not affected: a process
    /// is called by nothing).
    frame: bool,
    /// Whether a `Kind::Frame` subprogram is compiled at all. Its frame-mode code moves the rest of
    /// every process that calls it into the interpreter's loop for compiled code
    /// (`run_ops::<true, _>`), which its modelled gain of 9 percent does not cover (#481).
    frame_calls: bool,
    /// Whether the `WaitOn`s of a wait that names the same signals every activation
    /// (`yavs_interp::static_wait`) count as staying in `callbacks`: compiled code skips them
    /// with one `yavs_rt_wait_same` call (#1046).
    wait_same: bool,
}

impl Tune {
    const FIRST: Tune = Tune {
        rtl_min: 64,
        band: false,
        instances: 4,
        share: 32,
        tree_payback: 8,
        payback_rtl: 16,
        big: true,
        frame: true,
        frame_calls: false,
        wait_same: true,
    };

    /// `FIRST`, or what `YAVS_FIRST_TUNE=rtl_min=N,band=0|1,instances=N,share=N,
    /// tree_payback=N,payback_rtl=N,big=0|1,frame=0|1,frame_calls=0|1,wait_same=0|1` changes of it (a measuring aid; an entry that is not understood is ignored).
    fn first() -> Tune {
        static T: std::sync::OnceLock<Tune> = std::sync::OnceLock::new();
        *T.get_or_init(|| {
            let mut t = Tune::FIRST;
            let spec = std::env::var("YAVS_FIRST_TUNE").unwrap_or_default();
            for (k, v) in spec.split(',').filter_map(|e| e.split_once('=')) {
                match (k, v.parse::<u64>()) {
                    ("rtl_min", Ok(n)) => t.rtl_min = n as usize,
                    ("band", Ok(n)) => t.band = n != 0,
                    ("instances", Ok(n)) => t.instances = n as u32,
                    ("share", Ok(n)) => t.share = n,
                    ("tree_payback", Ok(n)) => t.tree_payback = n,
                    ("payback_rtl", Ok(n)) => t.payback_rtl = n,
                    ("big", Ok(n)) => t.big = n != 0,
                    ("frame", Ok(n)) => t.frame = n != 0,
                    ("frame_calls", Ok(n)) => t.frame_calls = n != 0,
                    ("wait_same", Ok(n)) => t.wait_same = n != 0,
                    _ => {}
                }
            }
            t
        })
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

/// CLOCK_THREAD_CPUTIME_ID
#[cfg(target_os = "linux")]
const CPU_THREAD: i32 = 3;

/// The simulation CPU: the CPU time of the kernel thread (`clock`, `Shared::sim_clock`), not of
/// the process, whose pool workers spin between batches. `None` where there is no such clock.
// ponytail: a pooled batch's wait past the join spin sleeps uncounted; count the workers' busy
// time if long pooled batches must weigh in full.
#[cfg(target_os = "linux")]
fn sim_cpu(clock: i32) -> Option<Duration> {
    if clock == 0 { None } else { cpu_read(clock) }
}

#[cfg(not(target_os = "linux"))]
fn sim_cpu(_: i32) -> Option<Duration> {
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

/// The wall clock and the pause between two reads of a trial's windows: the real ones, or a test's
/// own (#717: the number of pairs a trial takes must not depend on scheduling).
struct Pace<'a> {
    nap: &'a dyn Fn(),
    wall: &'a dyn Fn() -> Duration,
}

/// The progress per CPU second of the simulation threads in one window (per second of wall time
/// where `cpu` has no clock), `None` when it ended with too little progress to say or the tier
/// stopped.
fn window(
    progress: &dyn Fn() -> u64,
    cpu: &dyn Fn() -> Option<Duration>,
    halt: &dyn Fn() -> bool,
    pace: &Pace,
    min: Duration,
    max: Duration,
    min_runs: u64,
) -> Option<f64> {
    let (t0, p0, c0) = ((pace.wall)(), progress(), cpu());
    let (mut last, mut step) = (p0, 0);
    loop {
        (pace.nap)();
        let (wall, p) = ((pace.wall)() - t0, progress());
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
    pace: &Pace,
    rule: &Rule,
) -> Trial {
    let w = || {
        window(
            progress,
            cpu,
            halt,
            pace,
            TRIAL_WINDOW,
            TRIAL_MAX,
            rule.runs,
        )
    };
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

/// A compute loop gets this many times the budget: it runs five to ten times faster compiled
/// (the interpreter's time goes to dispatch), so the payback rule alone limits it, and the
/// compile is on another thread (#432).
const COMPUTE_BUDGET: u64 = 50;

/// A function the interpreter calls on a native frame (`called_natively`) gets this many times
/// the budget: its compiled code runs about two to three times faster than the interpreter
/// (`Tier::call`, the object table; #510), so the payback rule limits it. Sixteen times is
/// 32 percent of the sampled time: with a payback of 2 for native functions the former eight (16 percent) was
/// what the class ran into in the first seconds of a cold run (#572).
const NATIVE_BUDGET: u64 = 16;

/// An `rtl` process gets this many times the budget (its compiled code can be a third faster).
const RTL_BUDGET: u64 = 16;

/// A `Kind::Frame` process (a large combinational one) gets this many times the budget: at one
/// time the 2 percent refused every one of them in a first cold run (#1046).
const FRAME_BUDGET: u64 = 4;

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

/// The seen time still owed before a candidate may compile with its callees: it must have been seen
/// for `need` (the payback of its own price) and `tree_payback` times the price of the callees
/// compiled with it (`Tune::tree_payback`); 0 when it has.
fn repaid(need: u64, tree_payback: u64, extra: u64, seen: u64) -> u64 {
    need.saturating_add(tree_payback.saturating_mul(extra))
        .saturating_sub(seen)
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

/// What happened to a hot function: compiled at that level (with the time it took), or why
/// it stays interpreted (or at its first level).
pub type Note = Box<dyn FnMut(FuncId, Result<std::time::Duration, String>) + Send>;

/// The compiled code of a function: its entry (0 while it has none) and register count, with
/// `LEAN` set for a process `regs::no_table` passes. The compile again replaces the entry; code
/// once published stays valid as long as the tier.
#[derive(Default)]
struct Entry {
    run: AtomicUsize,
    nregs: AtomicUsize,
}

/// The bit of `Entry::nregs` that says the function is `lean` (`rt::enter`).
const LEAN: usize = 1 << (usize::BITS - 1);

impl Entry {
    fn get(&self) -> Option<(RunFn, usize)> {
        self.get_lean().map(|(f, n, _)| (f, n))
    }

    #[inline(always)]
    fn get_lean(&self) -> Option<(RunFn, usize, bool)> {
        let run = self.run.load(Ordering::Acquire);
        let n = self.nregs.load(Ordering::Relaxed);
        // SAFETY: a nonzero `run` was stored by `set` from a `RunFn`.
        (run != 0).then(|| unsafe {
            (
                std::mem::transmute::<usize, RunFn>(run),
                n & !LEAN,
                n & LEAN != 0,
            )
        })
    }

    fn set(&self, (run, nregs): (RunFn, usize), lean: bool) {
        self.nregs
            .store(nregs | (usize::from(lean) * LEAN), Ordering::Relaxed);
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
    /// `Tier::asked` waits for the compile (`YAVS_AUTO_SYNC`, tests).
    sync: bool,
    queue: Mutex<Option<Sender<Msg>>>,
    shared: Arc<Shared>,
    /// The compile thread: not started until [`AutoTier::launch`] (a run of a few milliseconds
    /// never needs it).
    worker: Mutex<Thread>,
}

/// The state of the compile thread: how it is started (the argument says whether it waits
/// `START_AFTER` first), the thread, or neither any more.
enum Thread {
    Idle(Box<dyn FnOnce(bool) -> Result<JoinHandle<()>, StartError> + Send>),
    Running(JoinHandle<()>),
    Ended,
}

/// What the compile thread is told.
enum Msg {
    /// `Tier::hot`: compile this function (eager mode, tests).
    Hot(FuncId),
    /// Looks of the sampler thread, in order (`Look`).
    Looks(Vec<Look>),
}

/// One look of the sampler thread: what the simulation threads were running, each with the work it
/// did since the last look (microseconds: `work`, else CPU time shared by the running threads). The
/// compile thread works through the looks in order whenever it is free, so what it decides depends
/// on the sequence of looks and not on how long a compile or a trial took (#670).
struct Look {
    running: Vec<(FuncId, bool, u64)>,
    /// The CPU time of the kernel thread since the look before (microseconds, `sim_cpu`; where
    /// there is no such clock, the busiest thread's work), whatever it was running.
    cpu: u64,
}

/// The look of `total` microseconds shared equally by the running functions (the CPU time of the
/// simulation threads is one number for all of them).
fn shared_look(total: u64, running: &[(usize, FuncId, bool)]) -> Look {
    let n = running.len().max(1) as u64;
    Look {
        running: running.iter().map(|&(_, f, c)| (f, c, total / n)).collect(),
        cpu: total,
    }
}

/// What the looks have found for the compile thread to do.
#[derive(Default)]
struct Pending {
    /// Functions `Tier::hot` asked for (eager mode, tests) and `YAVS_AUTO_FORCE` names.
    asked: std::collections::VecDeque<FuncId>,
    /// Hot functions waiting for the budget (`Worker::pick`).
    cands: Vec<FuncId>,
    /// Functions to look up in the compilation cache (`Worker::probe`).
    probes: Vec<FuncId>,
    /// Functions the profile of the last run called hot, seen running now, with the microseconds
    /// the last run saw them (`Worker::next_early`).
    early: Vec<(u64, FuncId)>,
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
    /// The CPU clock of the kernel thread, the one that made the tier (`pthread_getcpuclockid`;
    /// 0: none), whose CPU time is the simulation CPU (`sim_cpu`).
    sim_clock: i32,
    /// Why the sampler counts CPU time and not instructions (`work::activate`), once it started;
    /// `None` while the work clock counts.
    clock_off: Mutex<Option<String>>,
    /// The thread id of the kernel thread (`work::thread_id`).
    sim_tid: i32,
    /// What the sampler has read so far, for the trace and the calibration of `work::INSTR_PER_US`:
    /// the simulation CPU (`sim_cpu`, microseconds) and the instructions of the same thread (all
    /// threads' would count the pool's workers against the kernel thread's CPU).
    read_cpu_us: std::sync::atomic::AtomicU64,
    read_instr: std::sync::atomic::AtomicU64,
    /// Eager mode: per function, its compile (or the decision to leave it interpreted) is over,
    /// `Tier::hot` returns then.
    finished: Vec<AtomicBool>,
    /// The generator once the thread has ended: compiled code lives as long as the tier.
    generator: Mutex<Option<Box<dyn Generator>>>,
    /// The run-to-run profile of this run until `stop` writes it (`profile.rs`).
    profile: Mutex<Option<ProfileJob>>,
    /// Where functions compile (`COMPILE_*`, `AutoTier::set_compile`).
    compile: std::sync::atomic::AtomicU8,
}

/// The profile of a run in the making: where it goes, what the runs before left, the simulation CPU
/// time at the start of the compile thread (microseconds), and what the run has found so far.
struct ProfileJob {
    store: yavs_cache::Store,
    key: yavs_cache::Key,
    /// The ids in identity order (`objcache::profile_key`): the stored profile names ranks.
    order: Vec<u32>,
    earlier: Profile,
    cpu0: u64,
    snap: Snapshot,
}

/// The kinds of function the policy prices and judges apart (#673, user decision 2026-10-24: a price
/// per kind only together with the benefit expected of the kind).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    /// A function called on native frames, under `SHORT` IR instructions, without a loop.
    Tiny,
    /// A function called on native frames, without a loop (native-frame code only).
    Native,
    /// A function called on native frames with a loop that is not a compute loop (its frame-mode
    /// code later, `Worker::frame_at`).
    Loop,
    /// A compute loop (`compute`) called on native frames: native-frame code, full pipeline.
    Compute,
    /// Any other function or process, frame-mode code (the nested subprograms, the processes that
    /// are no clocked process, whatever has a loop and is not called on native frames).
    Frame,
    /// A clocked process (`rtl`) made of `Tune::instances` or more instances.
    Rtl,
    /// A clocked process made of fewer instances: its compiled code starts cold more often (few
    /// activations share it) and gains little, but a big part of the run (the carry chain of an
    /// adder made 8 times) still pays (docs/decisions.md).
    RtlFew,
    /// A large function (`native::big`, `Tune::big`): the large combinational processes and the
    /// subprograms of that size.
    Big,
    /// A compute loop in frame-mode code (a process with a loop, a nested subprogram): its compile
    /// costs a fraction of the native-frame code of the same size.
    ComputeFrame,
    /// A `Big` function with `BIG_LOOP_SHARE` percent or more of its IR instructions in loops
    /// (`loop_share`): its compiled code runs the loops faster, where that of a straight-line
    /// large process is level with the interpreter (#679).
    BigLoop,
}

/// The share of its IR instructions in loops (percent) from which a `Big` function is `BigLoop`.
/// Measured (#679): every large process of 12 percent or more ran 1.03 to 1.6 times faster compiled,
/// below 9 percent they lost or gained alike (0.86 to 1.36), so the line is in the gap.
const BIG_LOOP_SHARE: usize = 10;

/// What the policy expects of compiling a kind of function: the CPU microseconds a compile costs
/// (`fixed` plus `per_ir` per IR instruction: measured from the cold compiles of tools/realbench,
/// CPU time of the compile thread, which shares the simulation's core) and how many times faster its
/// compiled code runs than the interpreter (`speedup`). Seen interpreted for `price * speedup /
/// (speedup - 1)` microseconds a function is expected to have repaid its compile if the rest of the
/// run is like the part seen: the saving is `(speedup - 1) / speedup` of the time.
#[derive(Clone, Copy, Debug)]
struct Model {
    fixed: f64,
    per_ir: f64,
    speedup: f64,
}

impl Model {
    fn price(&self, n: usize) -> u64 {
        (self.fixed + self.per_ir * n as f64) as u64
    }

    /// How many times its price a function must be seen interpreted before it is compiled.
    fn payback(&self) -> f64 {
        self.speedup / (self.speedup - 1.0)
    }
}

const fn model(fixed: f64, per_ir: f64, speedup: f64) -> Model {
    Model {
        fixed,
        per_ir,
        speedup,
    }
}

/// The models, by `Kind` (`MODELS[kind as usize]`). Prices: cold compiles of tools/realbench
/// (CPU of the compile thread: a function of 20 IR instructions 6 to 9 ms, `get` of 61 14 ms, `push`
/// of 263 193 ms, native-frame code 450 to 620 us an IR instruction and rising with the size,
/// frame-mode code 210 to 340, `Fast` 90 to 115, #673; compute loops, #757: native-frame code 33 to
/// 62 ms for 79 to 153 IR, frame-mode code 21 to 50 ms for 91 to 558). Speedups: what the thresholds
/// of the flat price had (`Tiny` 1.2: the benefit of a small native function is lower than the factor
/// 2 of the others, a compile that costs 3 to 9 ms for it is more than it saves; the rest as
/// `PAYBACK*` were: 2 for functions called on native frames and compute loops, 4/3 for the rest),
/// `Big` pinned to the old threshold of 3 200 us an IR instruction (compiled code of the large
/// processes is level with the interpreter, #679; whether they compile is the user's decision, no
/// size cap), `BigLoop` at the median of its measured class (1.15: large processes with loops, forced
/// compiled one by one, #679). A function called on native frames is priced for its native-frame code only: its
/// frame-mode code is compiled when it is needed (`Worker::frame_at`). `Compute` (native frames) is
/// priced at one and a half times what it measures (33 to 62 ms for 79 to 153 IR, #757): at the
/// measured price a first run of 0.8 s compiled a string loop too late to repay it, and the callees
/// such a function brings are paid by `Worker::repays` (docs/decisions.md).
const MODELS: [Model; 10] = [
    model(2500.0, 200.0, 1.2),
    model(2000.0, 550.0, 1.55),
    model(4000.0, 550.0, 2.1),
    model(12000.0, 510.0, 2.4),
    model(2000.0, 250.0, 1.09),
    model(0.0, 200.0, 1.5),
    model(0.0, 200.0, 1.1),
    model(4000.0, 110.0, 1.037),
    model(15000.0, 110.0, 1.8),
    model(4000.0, 110.0, 1.15),
];

/// `MODELS`, with `YAVS_AUTO_MODEL=kind=fixed,per_ir,speedup/..` (a tuning aid) applied.
fn models() -> &'static [Model; 10] {
    static M: std::sync::OnceLock<[Model; 10]> = std::sync::OnceLock::new();
    M.get_or_init(|| {
        let mut m = MODELS;
        let names = [
            "tiny",
            "native",
            "loop",
            "compute",
            "frame",
            "rtl",
            "rtlfew",
            "big",
            "computeframe",
            "bigloop",
        ];
        for part in std::env::var("YAVS_AUTO_MODEL")
            .unwrap_or_default()
            .split('/')
        {
            let Some((k, v)) = part.split_once('=') else {
                continue;
            };
            let v: Vec<f64> = v.split(',').filter_map(|x| x.parse().ok()).collect();
            if let (Some(i), [fixed, per_ir, speedup]) =
                (names.iter().position(|n| *n == k.trim()), v.as_slice())
            {
                m[i] = model(*fixed, *per_ir, *speedup);
            }
        }
        m
    })
}

/// A module linked from the compilation cache: the body is generated again and its code linked,
/// about 4 us an IR instruction (neorv32 eager: 1 804 functions, 143 723 IR instructions in 536 ms
/// warm against 4 079 ms compiled, #481). A function whose code is in the cache is priced so
/// (`Worker::price`, user, 2026-10-25: a warm run uses cached code as soon as it is seen), not at
/// what compiling it costs. ponytail: the first run after the program changed has no pre-keys yet
/// and generates; the price ignores that one run.
const FIRST_HIT: Model = model(0.0, 4.0, 1.0);

/// `FIRST_HIT` since a hit links without the body (a pre-key), under the count rule (the old policy
/// keeps its numbers, for comparing): about 0.26 us an IR instruction by the work clock on the
/// measured designs, rounded up.
const FIRST_LINK: Model = model(0.0, 0.3, 1.0);

/// A look-up in the compilation cache generates the code of the function, about this part of a
/// compile (generation and the key: 10 to 20 us per IR instruction).
const PROBE_PART: u64 = 50;

/// Microseconds of translation time per IR instruction of a first-level `Generator` (Cranelift:
/// 40 to 45), its price for every kind of function (`Worker::price_of`).
const FIRST_US_PER_INSTR: f64 = 45.0;

/// What a first-level generator (Cranelift) takes per IR instruction, in CPU microseconds of the
/// compile thread, measured on cold compiles of GRLIB, the neorv32 SoC, the window generator, OSVVM
/// and LFSR (2026-11-08, 455 compiles): frame-mode code 21 to 34 by kind (median 26, rising past
/// 3 000 IR to 38), native-frame code 61 at under 100 IR rising to 148 at 1 000 to 3 000 (the
/// callees made with it included): `FIRST_NATIVE_US` plus an IR instruction per
/// `FIRST_NATIVE_GROWTH` instructions. What pacing charges (`Worker::cpu_price`), not the price the
/// policy's thresholds read (`FIRST_US_PER_INSTR`).
const FIRST_FRAME_US: u64 = 28;
const FIRST_NATIVE_US: u64 = 55;
const FIRST_NATIVE_GROWTH: u64 = 16;

/// How long a run lasts before the compile thread is wanted: it samples nothing earlier (not for
/// eager tiers). [`AutoTier::start_deferred`] leaves the start to the caller, who calls
/// [`AutoTier::launch`] after this time.
pub const START_AFTER: Duration = Duration::from_millis(5);

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

/// Interpreted entries before the count rule asks for a function, a process: half of them before it
/// switches to its copy, which has the other half (`NativeFlags::arm_count`). Half and double move
/// nothing measurable (`docs/decisions.md` "`--tier=auto`: the count rule").
const COUNT: u32 = 100;
/// The work the simulation threads did, in the looks' microseconds (`work::per_us`), before the count
/// rule compiles anything: a test of a few hundred milliseconds does not get a compile back.
const COUNT_GATE: u64 = 160_000;
/// The CPU time of its last run from which a program does not wait for that gate (microseconds, as the
/// run-to-run profile has it): the gate's work is about half of this, and half of this lets more short
/// suite tests link what they do not get back (OSVVM warm 1.14 against 1.09 in sum).
const COUNT_GATE_CPU: u64 = 400_000;

/// The part of its price a function must be seen interpreted before a run that stops for its
/// compiles (`--tier-compile=wait`) compiles it for the count (`Worker::unpaid`);
/// `YAVS_AUTO_WAIT_PAYBACK` overrides it (a tuning aid).
const WAIT_PAYBACK: f64 = 1.0;
/// The budget of a run that stops for its compiles (`Worker::wait_share`): a quarter of the simulation
/// CPU. Measured (all-thread instructions, three-run series from cold, 2026-11-08): a half compiled the
/// first run of `uvvm_axistream_vvc` as dear as no budget (48.5 G against 42.1), an eighth left GRLIB's
/// first run 11 percent dearer (267.8 G against 240.8) and the cache settling a run later.
const WAIT_SHARE: f64 = 0.25;
/// The budget once the looks hold `WAIT_LONG` of work (`Worker::share`): the whole simulation CPU, as
/// the pacing before it. Measured (all-thread instructions, paired with the build before): a quarter
/// all through cost GRLIB's first run 5 to 11 percent; with the whole after `WAIT_LONG` its first run
/// is 0.96, a half 0.98, and `uvvm_axistream_vvc`'s first run keeps 0.97 of the 0.78 that a quarter
/// gives it.
const WAIT_LONG_SHARE: f64 = 1.0;
/// What compiled code saves of the time of what it compiles (`Worker::unrepaid`): the whole-run gain of
/// settled cached code over the interpreter, measured 0.27 (`osvvm_axi4_random`), 0.37
/// (`neorv32_dual_full`), 0.41 (`uvvm_axistream_vvc`), 0.8 (the window generator, a compute design).
/// Half and double: 0.15 level with 0.3, 0.6 lets `osvvm_axi4_random` run 2 and 3 cost 10 and 20
/// percent more.
const WAIT_GAIN: f64 = 0.3;
/// `Worker::unrepaid`: the runs in a row a function's process is held before it compiles anyway
/// (`Worker::overdue`), outside the budget (`WAIT_HELD_FREE`). Never by default: a held body compiles
/// once the time the runs saw it in repays it, so no later run pays the earlier runs' bodies at once
/// (after one run, `neorv32_dual_full` run 5 was 1.13 and `neorv32_single_cache` run 4 1.07).
const WAIT_HELD_RUNS: u32 = u32::MAX;
/// `Worker::unrepaid` judges a held function with the callees that wait only for it
/// (`Worker::unlocked`): `uvvm_axistream_vvc`'s command executor seen 0.2 percent of its price by
/// itself frees about a second of callees a run.
const WAIT_SUBTREE: bool = true;
/// `Worker::unpaid` holds a process in the first run past `WAIT_LONG` until the tree its compile pulls
/// in was seen for this factor of its price (`Worker::tree_unpaid`). At 2 sampling noise decided whether
/// `uvvm_axistream_vvc`'s first run paid its tree at the very end; 2.5 is a margin over that edge.
const WAIT_TREE: f64 = 2.5;
const WAIT_HELD_FREE: bool = true;
/// `Worker::next_asked` orders by heat per price: in oldest-first order a large one-shot sequencer held
/// the budget (`uvvm_axistream_vvc` run 2 29.4 G against 25.0).
const WAIT_ORDER: bool = true;
/// The work in the looks (microseconds) after which a run that stops for its compiles compiles what
/// the count asks for as one that does not: a run that long is likely to repay it.
const WAIT_LONG: u64 = 1_000_000;
/// Past `WAIT_LONG`, the part of its price the first run must still have seen a process and its
/// copies for (`Worker::unpaid`): a process whose time is in its callees (`uvvm_axistream_vvc`'s
/// sequencer and command executor, 0.2 and 2 percent) repays nothing, and its compile frees its
/// callees' (1.2 s). GRLIB's first run is level from 0.05 to 0.2 and 1.6 percent dearer at 0.5.
const WAIT_LONG_FLOOR: f64 = 0.1;

/// Work between two looks at the native frames left to the interpreter (`Worker::guard`), in the looks'
/// microseconds, and instructions a frame left costs: pushing it, and running the rest of its
/// function interpreted (about 8 000 on the window generator).
const GUARD_STRETCH: u64 = 50_000;
const LEFT_COST: u64 = 8_000;
/// Native-frame code is suspect when the frames it leaves cost this part of the work of a stretch...
const GUARD_SHARE: u64 = 32;
/// ...and stays taken back if, per unit of work, the interpreter called the function more often than
/// the native code left (a tenth more, or the run is no faster without it) but less than this many
/// times as often (at that many calls per frame left a native call pays for the frames).
const GUARD_CALLS: u64 = 16;
/// Where the count of a function under trial starts: it never runs out.
const GUARD_ARM: u32 = 1 << 31;

/// Whether `--tier=auto` asks by the count rule (`YAVS_AUTO_POLICY=sampled`: as before it, with no
/// native variant of the two instructions, for comparing the two; read once).
pub fn count_rule_on() -> bool {
    static ON: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ON.get_or_init(|| std::env::var("YAVS_AUTO_POLICY").map_or(true, |v| v != "sampled"))
}

/// `YAVS_AUTO_GUARD_SHARE` (0: no guard), `YAVS_AUTO_GUARD_CALLS`: tests and sensitivity runs.
fn guard_knobs() -> (u64, u64) {
    let num = |name: &str| std::env::var(name).ok().and_then(|v| v.parse::<u64>().ok());
    (
        num("YAVS_AUTO_GUARD_SHARE").unwrap_or(GUARD_SHARE),
        num("YAVS_AUTO_GUARD_CALLS").unwrap_or(GUARD_CALLS),
    )
}

/// The rule's numbers, and whether the tier is asked and waits for the compile (`YAVS_AUTO_COUNT`,
/// `YAVS_AUTO_GATE`, `YAVS_AUTO_SYNC`: tests and sensitivity runs).
fn count_knobs() -> (u32, u64, bool) {
    let num = |name: &str| std::env::var(name).ok().and_then(|v| v.parse::<u64>().ok());
    (
        num("YAVS_AUTO_COUNT").map_or(COUNT, |n| n.clamp(1, u64::from(u32::MAX)) as u32),
        num("YAVS_AUTO_GATE").unwrap_or(COUNT_GATE),
        std::env::var_os("YAVS_AUTO_SYNC").is_some(),
    )
}

/// Who calls whom (any `Call` or `CallProt`), from the code of every function optimised so far
/// (`Program::newly_optimised`, the first `seen` of its list): a function that runs or has code
/// was optimised first, so every caller the policy asks about is in it, and the rest of the
/// program is not optimised for it. A copy of a subprogram is called only by copies.
struct Calls {
    callers: Vec<Vec<u32>>,
    callees: Vec<Vec<u32>>,
    seen: usize,
}

/// The compile thread's state.
struct Worker {
    /// The rules of the generator that makes the first code of a function (`Tune`).
    tune: Tune,
    prog: Arc<yavs_ir::Program>,
    table: Arc<[Entry]>,
    /// Native-frame code per function (`Mem::ntab`), published before the entry.
    ntab: Arc<[AtomicUsize]>,
    /// `CallFn` per function, published with its native-frame code.
    ctab: Arc<[AtomicUsize]>,
    native: Arc<yavs_interp::NativeFlags>,
    heat: Heat,
    max_size: usize,
    /// The code generator, opened by the first function it compiles: a run that never gets hot
    /// pays nothing.
    generator: Box<dyn Generator>,
    /// Per function: compiled.
    done: Vec<bool>,
    /// Per function: the address of its native-frame code once compiled (0: none), and whether
    /// `ntab` names it. Published once everything it calls natively is compiled too: a missing
    /// callee would materialise the frames at every call.
    nat: Vec<(usize, bool)>,
    /// Per function: its native-frame code was made.
    nat_by: Vec<bool>,
    /// Per function: the generator found it has no native-frame variant (its static `native_set`
    /// says it runs on one, the generator's lowering refuses it or a callee). Such a function gets
    /// frame-mode code instead (`compile`).
    no_native: Vec<bool>,
    /// Functions in `no_native`.
    refusals: usize,
    /// Per function: its frame-mode code was left for later because nothing compiled calls it
    /// yet (`frame_instead`).
    declined: Vec<bool>,
    /// Who calls whom (`Calls`), from the code of the functions optimised so far, and the marks of
    /// `unlocked`'s walk. Under the count rule `copy_compiled` also stands for a process's copies.
    graph: std::cell::RefCell<Calls>,
    check_calls: bool,
    /// Optimise every function the processes reach by calls at the first question about callers:
    /// a run without the last run's list of what it needed (`prefetch`). Done beside the
    /// simulation, it optimises ahead what the simulation calls later.
    force_reach: std::cell::Cell<bool>,
    /// Every function optimised so far, in that order (the first of `Program::newly_optimised`), for
    /// the profile's `ran` (`snapshot`); apart from `graph`, which a short run never asks.
    order: std::cell::RefCell<(usize, Vec<u32>)>,
    walked: std::cell::RefCell<Vec<bool>>,
    copy_compiled: Vec<bool>,
    /// Per function: the address of its `CallFn` (0: none), published with `nat`.
    calls: Vec<usize>,
    /// Microseconds each function was seen interpreted, and running compiled.
    time: Vec<[u64; 2]>,
    /// Per process (by `Program::base_of`), the microseconds it and all its copies were seen
    /// interpreted: the time every policy decision about a copy goes by (#661).
    group: Vec<u64>,
    /// Per function: it became a candidate (`start`).
    asked_for: Vec<bool>,
    /// Per function compiled as native-frame code only that has a loop: the interpreted time seen at
    /// which its frame-mode code is compiled too (0: none). A native frame of it left to the
    /// interpreter goes on interpreted, which is what this time measures (#757).
    frame_at: Vec<u64>,
    /// Functions with a loop the count asked for and got native-frame code alone (`asked_part`):
    /// their frame-mode code is asked for once a native frame of one is left to the interpreter.
    deferred: Vec<FuncId>,

    /// Microseconds the simulation threads were seen running, and the compile thread spent
    /// compiling.
    total: u64,
    spent: u64,
    /// The simulation CPU the looks covered (`Look::cpu`), and what the compiles cost by `cpu_price`
    /// (microseconds): what pacing compares.
    sim_us: u64,
    paid: u64,
    /// `compute` per function, once asked.
    compute: Vec<std::cell::OnceCell<bool>>,
    /// `kind` per function, once asked.
    kinds: Vec<std::cell::OnceCell<Kind>>,
    /// `cached`, worked out once per function.
    in_cache: Vec<std::cell::OnceCell<bool>>,
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
    /// The warning that a generator cannot be opened was printed.
    warned: bool,
    /// The generator was opened (its time counts as compile time spent).
    loaded: bool,
    /// The functions the compilation cache has compiled code of (`objcache::compiled_list`):
    /// the others are not looked up.
    listed: std::collections::HashSet<(String, u32)>,
    /// The run-to-run profile (`profile.rs`): its key in the store, the processes and functions it
    /// names as hot (not lost), by id, with the microseconds the last run saw them, and what this
    /// run has found for the next one (`snapshot`).
    profile_key: Option<yavs_cache::Key>,
    /// The store the profile is in (`load_profile`).
    store: Option<yavs_cache::Store>,
    profile_order: Vec<u32>,
    earlier: Profile,
    early: std::collections::HashMap<FuncId, u64>,
    /// Per process (by `Program::base_of`), the microseconds it and its copies were seen, interpreted
    /// or compiled, and the ones that passed `profile::HOT_MIN_US` so far.
    bseen: Vec<u64>,
    hotish: Vec<u32>,
    /// The copies of processes seen running (a copy twice when seen in both tiers): `snapshot` asks
    /// their kind (`big_loop`).
    copies_seen: Vec<FuncId>,
    /// The processes a trial took back, and the functions compiled at once because of the profile
    /// (a root being compiled that way, `early_root`, is charged no budget and is not held to `cold`).
    lost_bases: Vec<u32>,
    early_done: Vec<FuncId>,
    early_root: bool,
    /// The count rule is on (`count_rule_on`, a run with a budget and Cranelift alone): a function
    /// the interpreter entered `COUNT` times is asked for (`Tier::asked`) and compiled unless `waits`.
    counted: bool,
    /// Functions asked for that wait (`waits`), and those that did at some time.
    waiting: Vec<FuncId>,
    held: Vec<bool>,
    /// Functions `unrepaid` held at some time (`CountStat::Unrepaid`), in the order held; and what the
    /// earlier runs' profile says of those it held: interpreted microseconds seen summed over them,
    /// and in how many runs in a row (`Profile::held`), by process (`Program::base_of`).
    unrepaid_seen: Vec<bool>,
    /// Functions `unpaid` held past `WAIT_LONG` at some time (`CountStat::UnpaidLong`), and the ones
    /// compiled because of the callees they unlock (`CountStat::Unlocked`).
    unpaid_long_seen: Vec<bool>,
    unlocked_seen: Vec<bool>,
    /// Processes `tree_unpaid` held at some time (`CountStat::TreeHeld`).
    tree_seen: Vec<bool>,
    unrepaid_ids: Vec<FuncId>,
    carried: std::collections::HashMap<FuncId, (u64, u32)>,
    /// Callees that keep compiled native-frame code unpublished, to ask for (`publish`), and whether
    /// they were.
    want: Vec<FuncId>,
    wanted: Vec<bool>,
    /// The sampled work before anything is compiled by the count (`COUNT_GATE`), the count, and the
    /// CPU time of the last run from which a warm run does not wait (`COUNT_GATE_CPU`).
    gate: u64,
    count: u32,
    gate_cpu: u64,
    /// `Shared::sim_clock`: the clock of the trials' windows.
    sim_clock: i32,
    /// `Worker::guard`: the work at its last look, per function the frames it left by then and whether
    /// it was published then, the functions under trial, the ones found to gain, and its numbers
    /// (0: no guard).
    guard_at: u64,
    left_seen: Vec<u64>,
    live: Vec<bool>,
    trying: Vec<Trying>,
    proven: Vec<bool>,
    guard_share: u64,
    guard_calls: u64,
    /// `--tier-compile=wait` (paced): the run stops for every compile, so a compile the count asks for
    /// waits to be repaid by the time seen (`unpaid`), and `wait_payback` is the part of its price.
    stops: bool,
    wait_payback: f64,
    /// `wait_payback`'s part past `WAIT_LONG` (`YAVS_AUTO_WAIT_LONG_FLOOR`).
    wait_long_floor: f64,
    /// The budget of the compiles of a run that stops for them: `paid` at most this part of the
    /// simulation CPU looked at (`YAVS_AUTO_WAIT_SHARE`); `unrepaid`'s factor
    /// (`YAVS_AUTO_WAIT_GAIN`); whether `next_asked` orders by heat per price
    /// (`YAVS_AUTO_WAIT_ORDER`).
    wait_share: f64,
    /// `wait_share` once the run is long (`share`, `YAVS_AUTO_WAIT_LONG_SHARE`).
    wait_long_share: f64,
    wait_gain: f64,
    wait_order: bool,
    /// `unrepaid` judges a function with the callees it unlocks (`unlocked`,
    /// `YAVS_AUTO_WAIT_SUBTREE`; 0: by its own time and price).
    wait_subtree: bool,
    /// `unpaid` holds a process past `WAIT_LONG` until the tree its compile pulls in repays this
    /// factor of its price (`tree_unpaid`, `YAVS_AUTO_WAIT_TREE`; 0: off).
    wait_tree: f64,
    /// The runs in a row `unrepaid` holds a function before it compiles it anyway
    /// (`YAVS_AUTO_WAIT_HELD_RUNS`): the count asked for it in each of them.
    wait_held_runs: u32,
    /// An overdue function (`overdue`) needs no room in the budget (`YAVS_AUTO_WAIT_HELD_FREE`).
    wait_held_free: bool,
    /// `unrepaid` holds only a body that costs more than this part of the rest of the run
    /// (`YAVS_AUTO_WAIT_BIG`, the inverse part; 0: every body).
    wait_big: f64,
    trace: Option<Trace>,
}

/// Native-frame code taken back to see how often it is called (`Worker::guard`): since the work `at`,
/// after `frames` left in the work `over`.
struct Trying {
    id: FuncId,
    at: u64,
    frames: u64,
    over: u64,
}

/// `YAVS_AUTO_TRACE=1`, a tuning aid: the compile thread's timeline on stderr (a line per compile
/// and per refusal, the most sampled functions at the end).
struct Trace {
    t0: std::time::Instant,
    /// Per function, the milliseconds into the run it was first seen interpreted and became a
    /// candidate.
    first: std::collections::HashMap<FuncId, u128>,
    cand: std::collections::HashMap<FuncId, u128>,
    /// Functions a refusal line was printed for (`Worker::compile`): once each.
    refused: std::collections::HashSet<FuncId>,
}

/// `YAVS_AUTO_TRACE`: the compile in flight, for the line `stop` prints when the run ends during it.
static IN_FLIGHT: Mutex<Option<(String, std::time::Instant)>> = Mutex::new(None);

fn in_flight(what: Option<String>) {
    *IN_FLIGHT.lock().unwrap_or_else(PoisonError::into_inner) =
        what.map(|w| (w, std::time::Instant::now()));
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
    fn see(&mut self, look: &Look, pending: &mut Pending) {
        self.sim_us += look.cpu;
        if look.running.is_empty() {
            return;
        }
        for &(id, compiled, dt) in &look.running {
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
            let b = self.prog.base_of(id).0 as usize;
            let before = self.bseen[b];
            self.bseen[b] += dt;
            if before < profile::HOT_MIN_US && self.bseen[b] >= profile::HOT_MIN_US {
                self.hotish.push(b as u32);
            }
            if was == 0 && b != id.0 as usize {
                self.copies_seen.push(id);
            }
            if was == 0 && !compiled && forced(&self.prog, id) {
                pending.asked.push_back(id);
            }
            if compiled {
                continue;
            }
            let k = id.0 as usize;
            let seen_now = *t;
            if self.declined[k] && !self.done[k] && !self.lost[k] && self.entered_from_code(id) {
                self.declined[k] = false;
                pending.cands.push(id);
                self.pick_at = 0;
            }
            if self.frame_at[k] != 0 && seen_now >= self.frame_at[k] {
                self.frame_at[k] = 0;
                self.asked_for[k] = true;
                if !self.done[k] && !self.lost[k] {
                    pending.cands.push(id);
                    self.pick_at = 0;
                }
            }
            // The time of a process counts for every copy of it (#661): the process is hot when
            // its copies together are, and each copy that runs is a candidate then.
            let g = self.prog.base_of(id).0 as usize;
            self.group[g] += dt;
            // The class rules of `threshold` only raise the base, and they analyse everything the
            // function can call (`natively`): not for one still below the base (#544).
            let now = self.group[g];
            let base = self.floor(id);
            let at = if now < base { base } else { self.threshold(id) };
            // What the last run found hot is compiled at once, whatever the payback rule and the
            // budget say (the profile decides, not the looks: `next_early`). Not the shared code of a
            // process that can have copies: its processes switch to them, which the tier compiles.
            // A copy is held to the profile's threshold by its own kind (`load_profile`).
            if let Some(&us) = self.early.get(&self.prog.base_of(id))
                && !self.lost[id.0 as usize]
                && !(self.prog.base_of(id) == id && self.prog.specialisable(id))
                && (self.prog.base_of(id) == id || us >= self.threshold(id))
                && !std::mem::replace(&mut self.asked_for[id.0 as usize], true)
            {
                pending.early.push((us, id));
                continue;
            }
            if self.wants_probe(id, now) {
                pending.probes.push(id);
            }
            // A copy that is not a clocked process repays its own compile (copies differ in their
            // constants, so each one is a module of its own): by its own seen time (#757).
            let own =
                |w: &Self| w.prog.base_of(id) == id || w.is_rtl(id) || w.time[k][0] >= w.need(id);
            if now >= at
                && own(self)
                && !std::mem::replace(&mut self.asked_for[id.0 as usize], true)
            {
                // Native-frame code of it exists (linked from the cache, compiled as a callee): its
                // frame-mode code waits for `frame_at` like that of a root.
                if self.nat[k].0 != 0 && !self.done[k] && self.natural_part(id) == Part::Native {
                    if self.frame_at[k] == 0
                        && self.prog.func(id).is_some_and(|f| loops(&self.prog, f))
                    {
                        self.frame_at[k] = self.time[k][0] + self.frame_need(id);
                    }
                    continue;
                }
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
        max_size: usize,
        generator: Box<dyn Generator>,
        note: Note,
    ) -> Worker {
        let nfuncs = prog.funcs.len();
        Worker {
            tune: Tune::first(),
            time: vec![[0; 2]; nfuncs],
            group: vec![0; nfuncs],
            asked_for: vec![false; nfuncs],
            frame_at: vec![0; nfuncs],
            deferred: Vec::new(),
            done: vec![false; nfuncs],
            nat: vec![(0, false); nfuncs],
            nat_by: vec![false; nfuncs],
            no_native: vec![false; nfuncs],
            refusals: 0,
            declined: vec![false; nfuncs],
            check_calls: std::env::var_os("YAVS_AUTO_CHECK_CALLS").is_some(),
            graph: std::cell::RefCell::new(Calls {
                callers: vec![Vec::new(); nfuncs],
                callees: vec![Vec::new(); nfuncs],
                seen: 0,
            }),
            order: Default::default(),
            force_reach: std::cell::Cell::new(true),
            walked: std::cell::RefCell::new(vec![false; nfuncs]),
            copy_compiled: vec![false; nfuncs],
            calls: vec![0; nfuncs],
            prog,
            table,
            ntab,
            ctab,
            native,
            heat,
            max_size,
            generator,
            total: 0,
            spent: 0,
            sim_us: 0,
            paid: 0,
            compute: vec![Default::default(); nfuncs],
            kinds: vec![Default::default(); nfuncs],
            in_cache: vec![Default::default(); nfuncs],
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
            profile_key: None,
            store: None,
            profile_order: Vec::new(),
            earlier: Profile::default(),
            early: Default::default(),
            bseen: vec![0; nfuncs],
            hotish: Vec::new(),
            copies_seen: Vec::new(),
            lost_bases: Vec::new(),
            early_done: Vec::new(),
            early_root: false,
            counted: false,
            waiting: Vec::new(),
            held: vec![false; nfuncs],
            unrepaid_seen: vec![false; nfuncs],
            unpaid_long_seen: vec![false; nfuncs],
            tree_seen: vec![false; nfuncs],
            unlocked_seen: vec![false; nfuncs],
            unrepaid_ids: Vec::new(),
            carried: Default::default(),
            want: Vec::new(),
            wanted: vec![false; nfuncs],
            gate: 0,
            count: 0,
            gate_cpu: COUNT_GATE_CPU,
            sim_clock: 0,
            guard_at: 0,
            left_seen: vec![0; nfuncs],
            live: vec![false; nfuncs],
            trying: Vec::new(),
            proven: vec![false; nfuncs],
            guard_share: GUARD_SHARE,
            guard_calls: GUARD_CALLS,
            stops: false,
            wait_share: WAIT_SHARE,
            wait_long_share: WAIT_LONG_SHARE,
            wait_gain: WAIT_GAIN,
            wait_subtree: WAIT_SUBTREE,
            wait_tree: WAIT_TREE,
            wait_order: WAIT_ORDER,
            wait_held_runs: WAIT_HELD_RUNS,
            wait_held_free: WAIT_HELD_FREE,
            wait_big: 0.0,
            wait_payback: WAIT_PAYBACK,
            wait_long_floor: WAIT_LONG_FLOOR,
            trace: std::env::var_os("YAVS_AUTO_TRACE").map(|_| Trace {
                t0: std::time::Instant::now(),
                first: Default::default(),
                cand: Default::default(),
                refused: Default::default(),
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
                || !crate::native::native_set(prog, [id]).contains(&id)
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

    /// Whether `id` is a clocked process (`rtl`): a process that runs once per activation (no
    /// loop of its own, `Tune::rtl_min` to `RTL_MAX` IR instructions), whose time goes to the
    /// signal and kernel operations compiled code reaches directly; it is measured after the swap
    /// (`Worker::trial`, #596). A copy of a process (`Program::copy_of`) counts as the process it
    /// is a copy of, so every copy gets the decisions the process would get.
    fn is_rtl(&self, id: FuncId) -> bool {
        self.prog.func(self.prog.base_of(id)).is_some_and(|f| {
            f.kind == FuncKind::Process
                && (self.tune.rtl_min..=RTL_MAX).contains(&f.code.len())
                && !loops(&self.prog, f)
        })
    }

    /// Whether a function the count asked for waits: the run is still short (`gate`, read from the
    /// looks), or a subprogram no compiled code calls (a call between interpreted caller and
    /// compiled callee costs what the callee saves, the natfall lesson as a rule).
    fn waits(&self, id: FuncId) -> bool {
        let sub = self
            .prog
            .func(id)
            .is_some_and(|f| f.kind != FuncKind::Process);
        (self.total < self.gate && !self.link_early(id))
            || (sub && !self.entered_from_code(id))
            || self.unpaid(id)
            || self.unrepaid(id)
    }

    /// A later run of a program that stops for its compiles (`stops`, a profile of an earlier run)
    /// does not stop for a function the cache does not hold whose compile (`body_price`) its share
    /// of the looks would not save in the rest of the run the profile predicts (its last run's
    /// simulation CPU less what the looks covered), times `wait_gain` (what compiled code saves of
    /// the time) and `profile::EARLY_RUNS` (the cache serves the runs after). Read from the looks
    /// and the profile; `wait_gain` 0 turns it off.
    fn unrepaid(&self, id: FuncId) -> bool {
        self.unrepaid_by(id, self.wait_subtree)
    }

    /// `unrepaid`, and when `subtree` judged again with the callees `id` unlocks (`unlocked`): held
    /// only if neither repays, so the credit never holds what its own time repays.
    fn unrepaid_by(&self, id: FuncId, subtree: bool) -> bool {
        let Some(base) = self.earlier.base else {
            return false;
        };
        if !self.stops || self.wait_gain <= 0.0 || self.cached(id) {
            return false;
        }
        if self.overdue(id) {
            return false;
        }
        let carried = self.carried.get(&self.prog.base_of(id)).map_or(0, |c| c.0);
        let rest = base.saturating_sub(self.sim_us) as f64;
        // (The time the earlier runs saw it in, in this run's CPU per look microsecond.)
        let before = carried as f64 * self.sim_us.max(1) as f64 / self.total.max(1) as f64;
        let held = |(us, price): (u64, u64)| {
            // (A body that takes a small part of the rest of the run is not held: only a dear one is.)
            if self.wait_big > 0.0 && (price as f64) * self.wait_big <= rest {
                return false;
            }
            let share = us as f64 / self.total.max(1) as f64;
            (share * rest * (profile::EARLY_RUNS as f64) + before) * self.wait_gain < price as f64
        };
        held((self.time[id.0 as usize][0], self.body_price(id)))
            && (!subtree || held(self.unlocked(id)))
    }

    /// The interpreted time and `body_price` of `id` with those of the subprograms that wait only
    /// for it: not compiled, called by no compiled code (`entered_from_code`), and so on down; a
    /// callee's share split among its uncompiled callers that ran. The sampler books a call's time
    /// to the callee, so a dispatcher looks cold by its own time, and compiling it is what lets its
    /// callees compile. A callee never seen adds no price: it would not compile.
    fn unlocked(&self, id: FuncId) -> (u64, u64) {
        self.force_once();
        let mut walked = self.walked.borrow_mut();
        let (mut us, mut price) = (self.time[id.0 as usize][0], self.body_price(id));
        let mut todo = vec![id.0];
        walked[id.0 as usize] = true;
        let mut i = 0;
        while let Some(&f) = todo.get(i) {
            i += 1;
            // (Its calls are known once it is optimised: a callee never seen running is optimised
            // here, the rest of the program is not.)
            let code = self.prog.func(FuncId(f)).map_or(&[][..], |g| &g.code[..]);
            if cfg!(debug_assertions) && self.check_calls {
                let mut want: Vec<u32> =
                    code.iter()
                        .filter_map(|i| match i {
                            yavs_ir::Inst::Call { func, .. }
                            | yavs_ir::Inst::CallProt { func, .. } => Some(func.0),
                            _ => None,
                        })
                        .collect();
                let mut have = self.graph().callees[f as usize].clone();
                want.sort_unstable();
                want.dedup();
                have.sort_unstable();
                assert_eq!(have, want, "callees of f{f}");
            }
            for j in 0.. {
                let Some(c) = self.graph().callees[f as usize].get(j).copied() else {
                    break;
                };
                let k = c as usize;
                if walked[k]
                    || self.done[k]
                    || self.nat[k].0 != 0
                    || self
                        .prog
                        .funcs
                        .get(FuncId(c))
                        .is_none_or(|g| g.kind == FuncKind::Process)
                    || self.entered_from_code(FuncId(c))
                {
                    continue;
                }
                walked[k] = true;
                todo.push(c);
                let t = self.time[k][0];
                if t == 0 {
                    continue;
                }
                let ran = |g: u32| {
                    let g = g as usize;
                    !self.done[g] && (self.held[g] || self.time[g][0] > 0)
                };
                let n = self.graph().callers[k].iter().filter(|&&g| ran(g)).count();
                if cfg!(debug_assertions) && self.check_calls {
                    assert_eq!(n, self.callers_by_walk(FuncId(c), ran), "callers of {c}");
                }
                let n = n.max(1) as u64;
                us += t / n;
                price += self.body_price(FuncId(c)) / n;
            }
        }
        for &f in &todo {
            walked[f as usize] = false;
        }
        (us, price)
    }

    /// Count `id` once as compiled because of the callees it unlocks (`CountStat::Unlocked`).
    fn note_unlocked(&mut self, id: FuncId) {
        if self.wait_subtree
            && !self.unlocked_seen[id.0 as usize]
            && self.unrepaid_by(id, false)
            && !self.unrepaid_by(id, true)
        {
            self.unlocked_seen[id.0 as usize] = true;
            self.native.count_stat(CountStat::Unlocked);
        }
    }

    /// Count `id` once as held by `unrepaid` (`CountStat::Unrepaid`), and by `unpaid` past `WAIT_LONG`
    /// (`CountStat::UnpaidLong`).
    fn note_unrepaid(&mut self, id: FuncId) {
        if !self.unrepaid_seen[id.0 as usize] && self.unrepaid(id) {
            self.unrepaid_seen[id.0 as usize] = true;
            self.native.count_stat(CountStat::Unrepaid);
            self.unrepaid_ids.push(id);
        }
        if self.total >= WAIT_LONG
            && self.unpaid(id)
            && !std::mem::replace(&mut self.unpaid_long_seen[id.0 as usize], true)
        {
            self.native.count_stat(CountStat::UnpaidLong);
        }
        if self.total >= WAIT_LONG
            && !self.tree_seen[id.0 as usize]
            && self.stops
            && self.earlier.base.is_none()
            && !self.cached(id)
            && self.tree_unpaid(id)
        {
            self.tree_seen[id.0 as usize] = true;
            self.native.count_stat(CountStat::TreeHeld);
        }
    }

    /// The part of the simulation CPU the compiles of a run that stops for them may take: `wait_share`,
    /// `wait_long_share` once the looks hold `WAIT_LONG` of work (a long run repays more).
    fn share(&self) -> f64 {
        if self.total >= WAIT_LONG {
            self.wait_long_share
        } else {
            self.wait_share
        }
    }

    /// The earlier runs held `id`'s process for `wait_held_runs` runs in a row: it compiles now.
    fn overdue(&self, id: FuncId) -> bool {
        self.earlier.base.is_some()
            && self
                .carried
                .get(&self.prog.base_of(id))
                .is_some_and(|c| c.1 >= self.wait_held_runs)
    }

    /// `cpu_price` of what the count compiles of `id` (`asked_part`), a link when the cache holds it.
    fn body_price(&self, id: FuncId) -> u64 {
        let hit = self.cached(id);
        match self.asked_part(id) {
            Part::Native => self.cpu_price(id, Part::Native, hit),
            _ => {
                let native = self.nat[id.0 as usize].0 == 0 && self.nset(id).contains(&id);
                self.cpu_price(id, Part::Frame, hit)
                    + if native {
                        self.cpu_price(id, Part::Native, hit)
                    } else {
                        0
                    }
            }
        }
    }

    /// The next of the functions the count asked for (none waits) to compile in a run that stops for
    /// its compiles: links first, then the most seen per `body_price` among those whose price fits
    /// the budget (`paid` at most `wait_share` of the simulation CPU looked at), the oldest first
    /// otherwise; `None` when none fits. Not ordered with `wait_order` off.
    fn next_asked(&self, asked: &std::collections::VecDeque<FuncId>) -> Option<usize> {
        if !self.stops || !self.counted {
            return (!asked.is_empty()).then_some(0);
        }
        let room = self.sim_us as f64 * self.share();
        let fits = |id: FuncId| {
            (self.wait_held_free && self.overdue(id))
                || (self.paid + self.body_price(id)) as f64 <= room
        };
        if !self.wait_order {
            return asked
                .front()
                .filter(|&&id| self.cached(id) || fits(id))
                .map(|_| 0);
        }
        if let Some(k) = asked.iter().position(|&id| self.cached(id)) {
            return Some(k);
        }
        let heat =
            |id: FuncId| self.time[id.0 as usize][0] as f64 / self.body_price(id).max(1) as f64;
        (0..asked.len())
            .filter(|&k| fits(asked[k]))
            .max_by(|&a, &b| heat(asked[a]).total_cmp(&heat(asked[b])).then(b.cmp(&a)))
    }

    /// The trace line of a function the count asked for that `waits`: which of its rules hold it.
    fn held_why(&self, id: FuncId) -> String {
        let f = self.prog.func(id);
        let sub = f.is_some_and(|f| f.kind != FuncKind::Process);
        format!(
            "held f{} `{}` ({} IR): gate {}, uncalled subprogram {}, unpaid {} (seen {} of price {} us, cached {}), unrepaid {} (body {} us, with what it unlocks {:?} us), seen total {} us",
            id.0,
            f.map_or("?", |f| f.name.as_str()),
            f.map_or(0, |f| f.code.len()),
            self.total < self.gate && !self.link_early(id),
            sub && !self.entered_from_code(id),
            self.unpaid(id),
            self.time[id.0 as usize][0],
            self.price_of(id, false),
            self.cached(id),
            self.unrepaid(id),
            self.body_price(id),
            self.unlocked(id),
            self.total
        )
    }

    /// The first run of a program (no profile of an earlier run) that stops for its compiles
    /// (`stops`) pays each one in full: a function the cache does not hold waits until the looks saw
    /// it interpreted for `wait_payback` of its price. Once the run has lasted `WAIT_LONG` (a long run
    /// repays what the count asks for) only a process waits, while the looks saw it and its copies
    /// for less than `wait_long_floor` of that (one whose time is in its callees repays nothing; a
    /// subprogram waits as before until compiled code calls it). The runs after it compile as before,
    /// so the cache gets what pays over the runs (user, 2026-11-07). Read from the looks and the
    /// profile.
    fn unpaid(&self, id: FuncId) -> bool {
        if !self.stops || self.earlier.base.is_some() || self.cached(id) {
            return false;
        }
        let price = self.price_of(id, false) as f64 * self.wait_payback;
        if self.total < WAIT_LONG {
            return (self.time[id.0 as usize][0] as f64) < price;
        }
        self.prog
            .func(id)
            .is_some_and(|f| f.kind == FuncKind::Process)
            && ((self.bseen[self.prog.base_of(id).0 as usize] as f64)
                < price * self.wait_long_floor
                || self.tree_unpaid(id))
    }

    /// Past `WAIT_LONG`, a process whose compile pulls in the callees that wait only for it
    /// (`unlocked`) waits until the looks saw that tree for `wait_tree` times its price (and
    /// `wait_payback`): compiling it compiles them, all in this run.
    fn tree_unpaid(&self, id: FuncId) -> bool {
        if self.wait_tree <= 0.0 {
            return false;
        }
        let (us, price) = self.unlocked(id);
        price > self.body_price(id)
            && (us as f64) < price as f64 * self.wait_tree * self.wait_payback
    }

    /// What the cache lists is a link, and the last run of this program was long enough: it does not
    /// wait for the gate. The list alone says too little: all the tests of a library share it, and a
    /// short one lost four times over linking what longer ones had compiled.
    fn link_early(&self, id: FuncId) -> bool {
        self.cached(id) && (self.long_before() || self.early.contains_key(&self.prog.base_of(id)))
    }

    /// The last run named hot processes of its own (`early`): a shorter run than the gate links what
    /// the cache holds of them, and makes copies, from the start.
    fn named_hot(&self) -> bool {
        self.earlier.base.is_some() && !self.early.is_empty()
    }

    /// The last run of this program (`Profile::base`, CPU microseconds) passed the gate.
    fn long_before(&self) -> bool {
        self.earlier.base.is_some_and(|b| b >= self.gate_cpu)
    }

    /// The functions that wait no more, and the callees `publish` wants.
    fn freed(&mut self) -> Vec<FuncId> {
        if self.total >= self.gate {
            self.native.open_copies();
        }
        if self.gate != 0
            && self.total >= self.gate
            && let Some(tr) = &self.trace
        {
            tr.line(format!(
                "count gate open at {} us of work, simulation CPU {:?}",
                self.total,
                sim_cpu(self.sim_clock)
            ));
            self.gate = 0;
        }
        let mut go = std::mem::take(&mut self.want);
        if !self.deferred.is_empty() {
            let mut deferred = std::mem::take(&mut self.deferred);
            deferred.retain(|&i| {
                let k = i.0 as usize;
                if self.done[k] || self.lost[k] || self.nat[k].0 == 0 {
                    return false;
                }
                if self.native.materialised(i) == 0 {
                    return true;
                }
                self.native.count_stat(CountStat::FrameLater);
                go.push(i);
                false
            });
            self.deferred = deferred;
        }
        if !self.waiting.is_empty() {
            let wait = std::mem::take(&mut self.waiting);
            let (stay, free): (Vec<_>, Vec<_>) = wait.into_iter().partition(|&i| self.waits(i));
            for &i in &stay {
                self.note_unrepaid(i);
            }
            self.waiting = stay;
            go.extend(free);
        }
        go
    }

    /// Stop paying for native-frame code that leaves to the interpreter at most of its calls. A left
    /// frame is seen, a native call is not (a count in the code would cost every call), so code whose
    /// frames cost a good part of the work over a whole stretch is taken back, and its calls are
    /// counted in the interpreter for a stretch. Many more calls than frames per work: native code
    /// left rarely and gains. About as many or fewer: every call left, and the run is no faster
    /// interpreted. Both bring it back. In between it left on most calls and the run is faster without
    /// it, so it stays out. A function called only by direct calls of other native code counts few
    /// calls and comes back: it leaves through those callers, which are judged by their own frames.
    /// Read from the looks' work and the frames, no clock.
    fn guard(&mut self) {
        let work = self.total - self.guard_at;
        if !self.counted || self.guard_share == 0 || work < GUARD_STRETCH {
            return;
        }
        self.guard_at = self.total;
        let (total, calls_per) = (self.total, self.guard_calls);
        let mut back = false;
        let native = self.native.clone();
        self.trying.retain(|t| {
            let over = total - t.at;
            if over < GUARD_STRETCH {
                return true;
            }
            let calls = u64::from(GUARD_ARM - native.count_left(t.id));
            native.set_count(t.id, 0);
            let (now, before) = (calls * t.over, t.frames * over);
            let gains = now >= calls_per * before || now * 10 < before * 11;
            if gains {
                let k = t.id.0 as usize;
                self.lost[k] = false;
                self.proven[k] = true;
                back = true;
            } else {
                native.count_stat(CountStat::Withdrawn);
            }
            if let Some(tr) = &self.trace {
                tr.line(format!(
                    "{} native-frame code {}: {calls} calls in {over} us, {} frames left in {} us before",
                    t.id,
                    if gains { "back" } else { "taken back for good" },
                    t.frames,
                    t.over,
                ));
            }
            false
        });
        if back {
            self.publish();
        }
        for k in 0..self.nat.len() {
            let id = FuncId(k as u32);
            let left = self.native.materialised(id);
            let frames = left - std::mem::replace(&mut self.left_seen[k], left);
            // (Published over the whole stretch: `live`.)
            let whole = std::mem::replace(&mut self.live[k], self.nat[k].1);
            if frames == 0
                || !whole
                || !self.nat[k].1
                || self.lost[k]
                || self.proven[k]
                || frames * LEFT_COST < work * crate::work::per_us() / self.guard_share
            {
                continue;
            }
            self.lost[k] = true;
            self.nat[k].1 = false;
            self.ntab[k].store(0, Ordering::Release);
            self.ctab[k].store(0, Ordering::Release);
            self.native.clear_call(id);
            self.native.set_count(id, GUARD_ARM);
            self.trying.push(Trying {
                id,
                at: total,
                frames,
                over: work,
            });
        }
    }

    /// `asked_part` of a function the count asked for, noting a frame-mode part it leaves for later.
    fn ask(&mut self, id: FuncId) -> Part {
        let part = self.asked_part(id);
        if self.counted
            && part == Part::Native
            && self.prog.func(id).is_some_and(|f| loops(&self.prog, f))
        {
            self.deferred.push(id);
            self.native.count_stat(CountStat::FrameDeferred);
        }
        part
    }

    /// What the count compiles of `id`: native-frame code where it can run on one, the interpreter
    /// entering it there; else frame-mode code, and native-frame code too if it has. The frame-mode
    /// code of one with a loop waits for a native frame of it left to the interpreter (`freed`),
    /// unless its native-frame code exists already or there is no budget (`--tier-heat`).
    fn asked_part(&self, id: FuncId) -> Part {
        if !self.counted {
            return Part::Both;
        }
        match self.prog.func(id) {
            Some(f) if f.parent.is_none() && self.native_ok(id) => {
                if loops(&self.prog, f) && (self.heat.budget == 0 || self.nat[id.0 as usize].0 != 0)
                {
                    Part::Both
                } else {
                    Part::Native
                }
            }
            _ => Part::Both,
        }
    }

    /// The modules `tree` first makes of `id` as a root: native-frame code for a function that runs
    /// on native frames, else frame-mode code. The frame-mode code of one with a loop (for the
    /// interpreter to enter again where a native frame was left to it) waits until the function is
    /// seen interpreted after that (`frame_at`); with `--tier-heat` (no budget) both at once.
    fn natural_part(&self, id: FuncId) -> Part {
        match self.prog.func(id) {
            Some(f) if f.parent.is_none() && self.early_loop(id, f) => Part::Frame,
            Some(f) if f.parent.is_none() && self.native_ok(id) => {
                if self.heat.budget == 0 && loops(&self.prog, f) {
                    Part::Both
                } else {
                    Part::Native
                }
            }
            _ => Part::Frame,
        }
    }

    /// A subprogram of the profile (one the watching loop marked, `snapshot`) is compiled early
    /// only with a loop of its own (the mark is a sample: a busy subprogram without one is marked
    /// too) and of a kind the profile takes processes of: a modelled speedup of `EARLY_SPEEDUP` or
    /// more, or `Kind::BigLoop`. Judged here, in the run that compiles it, not in the run that names it.
    fn early_kind(&self, id: FuncId) -> bool {
        if self
            .prog
            .raw(id)
            .is_none_or(|f| f.kind == FuncKind::Process)
        {
            return true;
        }
        self.prog.func(id).is_some_and(|f| loops(&self.prog, f))
            && (self.model(id).speedup >= profile::EARLY_SPEEDUP || self.kind(id) == Kind::BigLoop)
    }

    /// A subprogram the profile names hot (`early`; only one whose count was spent by looks, with a
    /// loop of its own, gets there): compiled early as frame-mode code alone, which the activation
    /// running when it is ready enters at a loop head. Its native-frame code is left to the count
    /// as any function's: made at the first look it would leave to the interpreter at every callee
    /// this run had not seen yet (6 000 frames a run on `olo_fix_cordic_rot`).
    fn early_loop(&self, id: FuncId, f: &yavs_ir::Func) -> bool {
        f.kind != FuncKind::Process && self.early.contains_key(&id)
    }

    /// The microseconds of compile time to expect for `id` as `tree` compiles it: what the
    /// budget is charged.
    fn cost(&self, id: FuncId) -> u64 {
        self.price(id)
    }

    /// The kind of `id` (a copy of a process counts as the process it is a copy of), worked out once.
    fn kind(&self, id: FuncId) -> Kind {
        *self.kinds[id.0 as usize].get_or_init(|| self.kind_of(id))
    }

    fn kind_of(&self, id: FuncId) -> Kind {
        let Some(f) = self.prog.func(id) else {
            return Kind::Frame;
        };
        if self.is_rtl(id) {
            if self.native.instances(self.prog.base_of(id)) < self.tune.instances {
                Kind::RtlFew
            } else {
                Kind::Rtl
            }
        } else if self.tune.big && f.kind != FuncKind::Process && crate::native::big(&self.prog, f)
        {
            if loop_share(&self.prog, f) >= BIG_LOOP_SHARE {
                Kind::BigLoop
            } else {
                Kind::Big
            }
        } else if self.is_compute(id) {
            if !self.natively(id) {
                Kind::ComputeFrame
            } else {
                Kind::Compute
            }
        } else if self.natively(id) {
            if loops(&self.prog, f) {
                Kind::Loop
            } else if f.code.len() < crate::rt::SHORT {
                Kind::Tiny
            } else {
                Kind::Native
            }
        } else {
            Kind::Frame
        }
    }

    /// The model of `id`'s kind.
    fn model(&self, id: FuncId) -> &'static Model {
        &models()[self.kind(id) as usize]
    }

    /// The interpreted time `id`, compiled as native-frame code only, must be seen for before its
    /// frame-mode code is compiled (`frame_at`): what that code would repay at its kind's price.
    fn frame_need(&self, id: FuncId) -> u64 {
        (self.price_of(id, false) as f64 * self.model(id).payback()) as u64
    }

    /// The microseconds of compile time to expect for a module of `id` (the budget's price): the price
    /// of its kind, or of a link when the cache has it.
    fn price(&self, id: FuncId) -> u64 {
        self.price_of(id, self.cached(id))
    }

    /// Whether the compilation cache holds the object of `id`: an earlier run compiled a function
    /// of its name and size (`listed`). A function of the cache costs `FIRST_HIT`, whatever its kind.
    /// Never with `YAVS_AUTO_PRICE=model` (the model's price and `Heat::threshold` for every
    /// function, cold or warm: counts that do not depend on the cache, #728), and never for a
    /// subprogram the profile lost (compiled early, no faster, `early_loop`): its code stays in
    /// the cache, and priced as a link every later run would take it at once. Worked out once.
    fn cached(&self, id: FuncId) -> bool {
        static MODEL: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        *self.in_cache[id.0 as usize].get_or_init(|| {
            !*MODEL.get_or_init(|| std::env::var("YAVS_AUTO_PRICE").is_ok_and(|v| v == "model"))
                && !(self.earlier.lost.contains(&id.0)
                    && self
                        .prog
                        .raw(id)
                        .is_some_and(|f| f.kind != FuncKind::Process))
                && self.generator.cache_enabled()
                && self
                    .prog
                    .func(id)
                    .is_some_and(|f| self.listed.contains(&self.list_key(id, f)))
        })
    }

    /// The time any function must be seen before it is compiled: `Heat::threshold` of the process
    /// `id` is or is a copy of. None for a function whose object is in the cache under the default
    /// policy (maintainer 2026-10-27, #728): a warm run links it once its cache price repays
    /// (`need`). `--tier-heat` keeps it.
    fn floor(&self, id: FuncId) -> u64 {
        if self.heat.budget != 0 && self.cached(id) {
            return 0;
        }
        self.prog
            .func(self.prog.base_of(id))
            .map_or(u64::MAX, |b| self.heat.threshold(b))
    }

    /// How the list of the compilation cache names `id` (`f`): name and size, and for a copy of a
    /// process its `Program::copy_tag` too. Copies of one process share name and size but are
    /// modules of their own (their constants differ), so one compiled copy says nothing of another.
    fn list_key(&self, id: FuncId, f: &yavs_ir::Func) -> (String, u32) {
        let name = match self.prog.copy_tag(id) {
            Some(tag) => format!("{}#{tag:x}", f.name),
            None => f.name.clone(),
        };
        (name, f.code.len() as u32)
    }

    /// `price` of a function in the cache (`cached`) or compiled by the model of its kind.
    fn price_of(&self, id: FuncId, cached: bool) -> u64 {
        let Some(f) = self.prog.func(id) else {
            return 0;
        };
        if cached {
            let link = self.counted && self.prog.named_before;
            if link { FIRST_LINK } else { FIRST_HIT }.price(f.code.len())
        } else {
            (FIRST_US_PER_INSTR * f.code.len() as f64) as u64
        }
    }

    /// What compiling `part` of `id` costs in CPU microseconds (`hit`: a link from the cache), for
    /// pacing and the budget of the count's compiles (`paid`): a first-level generator's measured
    /// cost (`FIRST_FRAME_US`, `FIRST_NATIVE_US`), else `price_of`.
    fn cpu_price(&self, id: FuncId, part: Part, hit: bool) -> u64 {
        let Some(f) = self.prog.func(id) else {
            return 0;
        };
        if hit {
            return self.price_of(id, hit);
        }
        let n = f.code.len() as u64;
        if part == Part::Native {
            n * (FIRST_NATIVE_US + n / FIRST_NATIVE_GROWTH)
        } else {
            n * FIRST_FRAME_US
        }
    }

    /// `band_wait` of the clocked process `id`, none when its object is in the cache (#728): the wait
    /// keeps a run from paying the compile and the trial of a process outside the band that may lose,
    /// and linking it costs 3 ms.
    /// None either for a first-level generator (`Tune::band`: the trial judges every process).
    fn band(&self, id: FuncId) -> u64 {
        if self.cached(id) || !self.tune.band {
            0
        } else {
            band_wait(self.base_len(id))
        }
    }

    /// The sampled microseconds `id` must have been seen interpreted before it is compiled, by the
    /// payback of its kind (`Model::payback`): the price of the function times what its compiled
    /// code is expected to save.
    fn need(&self, id: FuncId) -> u64 {
        if self.kind(id) == Kind::Frame {
            let process = self
                .prog
                .func(self.prog.base_of(id))
                .is_some_and(|f| f.kind == FuncKind::Process);
            let on = if process {
                self.tune.frame
            } else {
                self.tune.frame_calls
            };
            if !on {
                return u64::MAX;
            }
        }
        (self.cost(id) as f64 * self.model(id).payback()).ceil() as u64
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

    /// Whether the generator found that `id`, or a function it reaches on native frames, has no
    /// native-frame variant (`no_native`): `id` cannot have one either, since a native variant is
    /// made only when every native callee can be called.
    fn refused(&self, id: FuncId) -> bool {
        self.refusals > 0 && self.nset(id).iter().any(|c| self.no_native[c.0 as usize])
    }

    /// `native_set` says `id` runs on native frames and the generator has not found otherwise:
    /// the part the policy chooses for it (`natural_part`, `tree`).
    fn native_ok(&self, id: FuncId) -> bool {
        self.nset(id).contains(&id) && !self.refused(id)
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
    fn threshold(&self, id: FuncId) -> u64 {
        // (A copy's seen time is its process's, `Worker::group`: its threshold is the process's too.)
        let at = self.floor(id);
        if self.heat.budget == 0 {
            at
        } else if self.is_rtl(id) {
            let few = if self.kind(id) == Kind::RtlFew {
                self.need(id)
            } else {
                0
            };
            at.max(self.band(id))
                .max(few)
                .max(self.tune.payback_rtl * self.cost(id))
        } else {
            at.max(self.need(id))
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
        let native = nc.contains(&root) && !self.refused(root);
        // The frame-mode code `frame_at` asks for: the root alone, the callees its native-frame code
        // left out stay so (they were never seen running).
        if native
            && self.nat[root.0 as usize].0 != 0
            && self.prog.func(root).is_some_and(|f| loops(&self.prog, f))
        {
            return vec![(root, Part::Frame)];
        }
        if native {
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
                    Some(f) if f.parent.is_none() => self.natural_part(root),
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
                    && !self.done[func.0 as usize]
                    && !self.unseen(*func, &hot)
                    && g.kind != FuncKind::Elab
                    && g.code.len() <= CALLEE_MAX
                    && total + g.code.len() <= TREE_MAX
                {
                    total += g.code.len();
                    ids.push(*func);
                } else if self.done[func.0 as usize]
                    && self.nat[func.0 as usize].0 == 0
                    && !natives.iter().any(|n| n.0 == *func)
                    && g.code.len() <= CALLEE_MAX
                    && total + g.code.len() <= TREE_MAX
                    && self.native_ok(*func)
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
                        f.parent.is_none() && !loops(&self.prog, f) && self.native_ok(i)
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

    /// Tell the generators which functions are about to be compiled together (`Generator::tree`).
    fn announce(&mut self, ids: &[(FuncId, Part)]) {
        let ids: Vec<FuncId> = ids.iter().map(|&(i, _)| i).collect();
        self.generator.tree(&ids);
    }

    /// The function that keeps the native-frame code of `id` unpublished: every one it calls natively
    /// must be compiled, and they in turn. A callee left out of its compile or never seen running ends
    /// the walk: nothing compiles it until it runs, so it kept `id` and every native caller of `id`
    /// unpublished for the whole run, each of their calls a step out of compiled code. Once it runs, it
    /// is seen and compiled like any other function, and the call reaches it through `ntab`.
    fn missing(&self, id: FuncId) -> Option<FuncId> {
        let nc = self.nset(id);
        let mut seen = std::collections::HashSet::from([id]);
        let mut todo = vec![id];
        while let Some(g) = todo.pop() {
            for inst in self.prog.func(g).map_or(&[][..], |f| &f.code[..]) {
                let (yavs_ir::Inst::Call { func, .. } | yavs_ir::Inst::CallProt { func, .. }) =
                    *inst
                else {
                    continue;
                };
                if !nc.contains(&func) || !seen.insert(func) {
                    continue;
                }
                let k = func.0 as usize;
                if self.nat[k].0 != 0 {
                    todo.push(func);
                } else if !self.left[k].get()
                    && (self.time[k] != [0, 0]
                        || (self.counted && self.native.count_left(func) != self.count))
                {
                    // (Under the count rule "it ran" is the count, no matter of what a look caught.)
                    return Some(func);
                }
            }
        }
        None
    }

    /// Publish the native-frame code whose callees are all compiled.
    fn publish(&mut self) {
        for k in 0..self.nat.len() {
            let (addr, done) = self.nat[k];
            if addr == 0 || done || self.lost[k] {
                continue;
            }
            let missing = self.missing(FuncId(k as u32));
            // Under the count rule a callee that ran is what compiled code calls: it does not wait for
            // its count, or a rare one would keep its callers unpublished.
            if let Some(c) = missing
                && self.counted
                && !std::mem::replace(&mut self.wanted[c.0 as usize], true)
            {
                self.want.push(c);
            }
            if missing.is_none() {
                self.ntab[k].store(addr, Ordering::Release);
                self.nat[k].1 = true;
                if let Some(tr) = &self.trace {
                    tr.line(format!(
                        "f{k} native-frame code published, call {}",
                        self.calls[k] != 0
                    ));
                }
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
        cands.retain(|c| !self.done[c.0 as usize]);
        if self.total < self.pick_at || cands.is_empty() {
            return None;
        }
        let heat = |w: &Self, id: FuncId| {
            let cost = w.need(id).max(1);
            w.time[id.0 as usize][0] * 1000 / cost
        };
        let mut order: Vec<usize> = (0..cands.len()).collect();
        order.sort_by_key(|&i| std::cmp::Reverse(heat(self, cands[i])));
        let mut trees = Vec::new();
        let mut prices = Vec::new();
        let mut waits = Vec::new();
        let mut links = Vec::new();
        order.retain(|&i| {
            let ids = self.tree(cands[i]);
            let (saves, costs) = self.repays(cands[i], &ids);
            if saves < costs {
                waits.push((cands[i], ids.len(), saves, costs));
                return false;
            }
            let cost: u64 = ids.iter().map(|&(i, _)| self.price(i)).sum::<u64>() + self.load_cost();
            prices.push((cost, self.factor(cands[i])));
            links.push(ids.iter().all(|&(i, _)| self.cached(i)));
            trees.push(ids);
            true
        });
        if let Some(tr) = &mut self.trace {
            for (id, n, saves, costs) in waits {
                if tr.refused.insert(id) {
                    tr.line(format!(
                        "waits f{} `{}`: its compile of {n} functions costs {costs:.0} us, \
                         saves {saves:.0} us of a rest of the run like the part seen",
                        id.0,
                        self.prog.func(id).map_or("?", |f| f.name.as_str()),
                    ));
                }
            }
        }
        if order.is_empty() {
            return None;
        }
        // A tree the cache holds whole is a link: it does not wait for the budget (maintainer
        // 2026-10-27, #728: a warm run uses cached code as soon as its cache price repays).
        if self.heat.budget != 0
            && let Some(k) = links.iter().position(|&l| l)
        {
            cands.swap_remove(order[k]);
            return Some(trees.swap_remove(k));
        }
        // The payback rule counts what the callees cost too (`Tune::tree_payback`).
        let mut ready: Vec<usize> = (0..order.len()).collect();
        if self.tune.tree_payback != 0 {
            let load = self.load_cost();
            let mut wait = u64::MAX;
            ready.retain(|&k| {
                let id = cands[order[k]];
                // (A nested subprogram entered in frame mode has no native variant to pay for.)
                if self.is_rtl(id) || matches!(trees[k].first(), Some((_, Part::Frame))) {
                    return true;
                }
                let own = trees[k].first().map_or(0, |&(i, _)| self.price(i));
                let extra = (prices[k].0 - load).saturating_sub(own);
                let seen = self.time[id.0 as usize][0];
                let need = (own as f64 * self.model(id).payback()) as u64;
                let owed = repaid(need, self.tune.tree_payback, extra, seen);
                wait = wait.min(owed);
                owed == 0
            });
            if ready.is_empty() {
                // (Seen time cannot grow faster than the sampled total.)
                self.pick_at = self.total.saturating_add(wait);
                return None;
            }
        }
        let fit: Vec<(u64, u64)> = ready.iter().map(|&k| prices[k]).collect();
        match first_fit(&fit, self.heat.budget, self.spent, self.total) {
            Ok(j) => {
                let k = ready[j];
                cands.swap_remove(order[k]);
                Some(trees.swap_remove(k))
            }
            Err(waits) => {
                self.pick_at = waits.iter().copied().min().unwrap_or(0);
                for (j, at) in waits.into_iter().enumerate() {
                    let k = ready[j];
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
                        (self.note)(id, Err(why));
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

    /// What compiling `root` with its tree `ids` saves in a rest of the run like the part seen (each
    /// function's seen time over its kind's payback) and what it costs (the prices of the tree), in
    /// microseconds: compiled when the first is at least the second. The payback rule prices the root
    /// alone, and the callees a native-frame root brings cost up to six times its own price (#757).
    /// A program the profile says ran before shares the cost among `profile::EARLY_RUNS` runs (the
    /// compilation cache makes it a link in the next ones). The root's seen time is
    /// what `threshold` reads (a process's pooled over its copies, a copy's own unless it is a clocked
    /// process); a clocked process keeps its own rules, its seen time pays for its callees only.
    fn repays(&self, root: FuncId, ids: &[(FuncId, Part)]) -> (f64, f64) {
        if self.heat.budget == 0 {
            return (0.0, 0.0);
        }
        let runs = if self.earlier.base.is_some() {
            profile::EARLY_RUNS as f64
        } else {
            1.0
        };
        let rtl = self.is_rtl(root);
        let base = self.prog.base_of(root);
        let (mut saves, mut costs) = (0.0, 0.0);
        for &(i, _) in ids {
            let seen = if i == root && (rtl || base == root) {
                self.group[base.0 as usize]
            } else {
                self.time[i.0 as usize][0]
            };
            saves += seen as f64 / self.model(i).payback();
            if i != root || !rtl {
                costs += self.price(i) as f64 / runs;
            }
        }
        (saves, costs)
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
        let wait = if self.is_rtl(id) { self.band(id) } else { 0 };
        let at = self
            .prog
            .func(self.prog.base_of(id))
            .map_or(u64::MAX, |b| self.heat.threshold(b));
        if self.probed[k] || self.done[k] || seen < at.max(wait) {
            return false;
        }
        // The look-up generates the function's code: within the budget, as a compile is, with at
        // least the factor of a function called natively (code compiled before for a function
        // hot before is likely to pay); asked again at the next sample while there is no room.
        let go = self.generator.cache_enabled()
            && self.listed.contains(&self.list_key(id, f))
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
            Some(_) if self.kind(id) == Kind::Frame => FRAME_BUDGET,
            _ => 1,
        }
    }

    /// What opening the generator costs until it is open.
    fn load_cost(&self) -> u64 {
        if self.loaded {
            0
        } else {
            self.generator.load_us()
        }
    }

    /// Whether the generator only links from its compilation cache.
    fn cached_only(&mut self, on: bool) {
        self.generator.cached_only(on);
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
        self.announce(&ids);
        self.probing = true;
        self.cached_only(true);
        let hit = self.compile(root, true, part);
        self.cached_only(false);
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
                self.compile(i, false, p);
            } else {
                self.probing = true;
                self.cached_only(true);
                self.compile(i, false, p);
                self.cached_only(false);
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
        let clock = self.sim_clock;
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
        // The shared code of a process that can have copies is not measured: its processes switch
        // to their copies, which the tier compiles and measures, so the windows would time code
        // that no longer runs (`rtl_lfsr` taken back at 1.0, #675).
        if self.prog.base_of(id) == id && self.prog.specialisable(id) {
            return;
        }
        if self.heat.budget == 0
            || self.heat.eager()
            || clocked && self.unmeasurable
            || self.time[k][0].saturating_mul(TRIAL_SHARE) < self.total
        {
            return;
        }
        let (run, (_, published)) = (self.table[k].get_lean(), self.nat[k]);
        let native = self.ntab[k].load(Ordering::Acquire);
        let call = self.ctab[k].load(Ordering::Acquire);
        // Nothing of its own to swap, or its native-frame code still waits for its callees.
        if run.is_none() && native == 0 || self.nat[k].0 != 0 && !published {
            return;
        }
        // Compiled callers find a callee without native-frame code (null) and leave to the interpreter.
        let mut swap = |compiled: bool| {
            if compiled {
                if let Some((f, n, lean)) = run {
                    self.table[k].set((f, n), lean);
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
        let start = std::time::Instant::now();
        let pace = Pace {
            nap: &|| std::thread::sleep(Duration::from_millis(1)),
            wall: &|| start.elapsed(),
        };
        let t = if clocked {
            run_trial(
                &mut swap,
                &yavs_interp::sim_progress,
                &|| sim_cpu(clock),
                &halted,
                &pace,
                &BY_RUNS,
            )
        } else {
            counts.count_on(id);
            let t = run_trial(
                &mut swap,
                &|| counts.entries(),
                &|| sim_cpu(clock),
                &halted,
                &pace,
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
            self.lost_bases.push(self.prog.base_of(id).0);
            // The copies of a process are one process to the policy (#661): the ones not compiled
            // yet stay interpreted too.
            // (Not when it was the shared code: the copies fold what the shared code cannot, and
            // the processes that switched to one no longer run the code the trial measured.)
            let base = self.prog.base_of(id);
            for j in 0..self.lost.len() {
                if base != id && self.prog.base_of(FuncId(j as u32)) == base && !self.done[j] {
                    self.lost[j] = true;
                }
            }
            (self.note)(
                id,
                Err(format!(
                    "the run progressed {x:.2} times as fast with its compiled code as with the \
                     interpreter, so it was taken back"
                )),
            );
        }
    }

    /// Read the profile of an earlier run of this program (`profile.rs`): what it found hot, and
    /// not lost, is compiled when first seen running (`see`, `next_early`). Nothing in a test or
    /// tuning mode (`--tier-heat`), without a compilation cache, or with `YAVS_AUTO_PROFILE=off`.
    fn load_profile(&mut self) {
        static OFF: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
        let off = *OFF.get_or_init(|| {
            std::env::var("YAVS_AUTO_PROFILE").is_ok_and(|v| v.eq_ignore_ascii_case("off"))
        });
        if off || self.heat.budget == 0 || self.heat.eager() {
            return;
        }
        self.store = self.generator.store();
        let Some(store) = &self.store else {
            return;
        };
        let Some((key, order)) = profile::key(&self.prog) else {
            return;
        };
        self.profile_order = order;
        let Some(mut p) = profile::load(store, &key).and_then(|t| Profile::parse(&t)) else {
            self.profile_key = Some(key);
            return;
        };
        self.profile_key = Some(key);
        p.remap(|rank| self.profile_order.get(rank as usize).copied());
        // A function is compiled early once the time the earlier runs saw it, summed, repays its
        // compile by the payback rule (`threshold`: the rule's price, the object cache making it a
        // link), maintainer 2026-10-27 (#679): the compile is paid once and the cache serves the runs
        // after. A process that can have copies is held to it by each copy's own kind when the copy
        // runs (`see`): a `BigLoop` copy of a `Big` process.
        for &(id, us, _) in &p.hot {
            let id = FuncId(id);
            let fine = self.prog.funcs.get(id).is_some_and(|f| !f.spare)
                && self.prog.base_of(id) == id
                && !p.lost.contains(&id.0)
                && (self.prog.specialisable(id) || us >= self.threshold(id))
                && self.early_kind(id);
            if fine {
                self.early.insert(id, us);
            }
        }
        if let Some(tr) = &self.trace {
            tr.line(format!(
                "profile: {} hot, {} lost, last run {:?} us",
                self.early.len(),
                p.lost.len(),
                p.base
            ));
        }
        self.carried = p
            .held
            .iter()
            .map(|&(id, us, runs)| (FuncId(id), (us, runs)))
            .collect();
        self.earlier = p;
    }

    /// The next function of the profile to compile, the one the last run saw longest first, with
    /// what compiles with it; `None` when none is waiting.
    fn next_early(&mut self, pending: &mut Pending) -> Option<Vec<(FuncId, Part)>> {
        while !pending.early.is_empty() {
            let k = (0..pending.early.len())
                .max_by_key(|&i| (pending.early[i].0, std::cmp::Reverse(pending.early[i].1.0)))?;
            let (_, id) = pending.early.swap_remove(k);
            if !self.done[id.0 as usize] && !self.lost[id.0 as usize] {
                return Some(self.tree(id));
            }
        }
        None
    }

    /// Whether process `b` or a copy of it seen running is `Kind::BigLoop` (a copy has a loop share
    /// of its own: `walk_control`'s copy 12 percent, its shared code 6). The profile takes these
    /// whatever their speedup (maintainer, 2026-10-27, #679).
    fn big_loop(&self, b: FuncId) -> bool {
        match self.kind(b) {
            Kind::BigLoop => true,
            Kind::Big => self
                .copies_seen
                .iter()
                .any(|&c| self.prog.base_of(c) == b && self.kind(c) == Kind::BigLoop),
            _ => false,
        }
    }

    /// What this run has found so far for the profile (`profile::next` makes the profile of it at
    /// the end of the run).
    fn snapshot(&self) -> Snapshot {
        let seen = self
            .hotish
            .iter()
            .map(|&b| (b, self.bseen[b as usize]))
            .filter(|&(b, us)| {
                // A subprogram whose count was spent by looks: a later run compiles it early, and an
                // activation running when it is ready enters its frame-mode code at a loop head.
                if self.native.looped(FuncId(b))
                    && self
                        .prog
                        .func(FuncId(b))
                        .is_some_and(|f| f.kind != FuncKind::Process)
                {
                    return us.saturating_mul(LOOP_SHARE) >= self.total;
                }
                self.prog
                    .func(FuncId(b))
                    .is_some_and(|f| f.kind == FuncKind::Process)
                    && (self.model(FuncId(b)).speedup >= profile::EARLY_SPEEDUP
                        || self.big_loop(FuncId(b)))
                    && (!self.is_rtl(FuncId(b)) || us.saturating_mul(RTL_SHARE) >= self.total)
            })
            .collect();
        let mut early: Vec<u32> = self
            .early_done
            .iter()
            .map(|&f| self.prog.base_of(f).0)
            .collect();
        early.sort_unstable();
        early.dedup();
        // (What an earlier run held and this one has not compiled yet stays, its run counted again.)
        let mut ids = self.unrepaid_ids.clone();
        ids.extend(self.carried.keys().filter(|b| {
            self.prog.func(**b).is_some()
                && !self
                    .copy_compiled
                    .get(b.0 as usize)
                    .copied()
                    .unwrap_or(false)
        }));
        ids.sort_unstable();
        ids.dedup();
        let held = ids
            .iter()
            .filter(|&&f| {
                let k = f.0 as usize;
                !self.done[k] && self.nat[k].0 == 0
            })
            .fold(Vec::<(u32, u64)>::new(), |mut v, &f| {
                // (By process: a copy's id is new in every run, the process's is not.)
                let (b, us) = (self.prog.base_of(f).0, self.time[f.0 as usize][0]);
                match v.iter_mut().find(|e| e.0 == b) {
                    Some(e) => e.1 += us,
                    None => v.push((b, us)),
                }
                v
            });
        // (Copies have new ids in every run.)
        let mut order = self.order.borrow_mut();
        let (at, list) = &mut *order;
        list.extend(self.prog.newly_optimised(at).iter().map(|f| f.0));
        let ran = list
            .iter()
            .copied()
            .filter(|&f| {
                let k = f as usize;
                self.prog.funcs.get(FuncId(f)).is_some_and(|g| !g.spare)
                    && (self.time[k] != [0, 0]
                        || self.done[k]
                        || self.nat[k].0 != 0
                        || (self.counted && self.native.count_left(FuncId(f)) != self.count))
            })
            .collect();
        Snapshot {
            total: self.total,
            seen,
            lost: self.lost_bases.clone(),
            early,
            held,
            ran,
        }
    }

    /// Compile `id`; whether it was compiled. Native-frame code
    /// of a function with a loop that the cache had brings its frame-mode code along when the cache has
    /// that too (a link costs a millisecond; `frame_at` decides only about compiling it).
    fn compile(&mut self, id: FuncId, sampled: bool, part: Part) -> bool {
        let k = id.0 as usize;
        // A function with no native-frame variant gets frame-mode code instead, as one the
        // static `native_set` did not allow: its callers reach it through the interpreter, which
        // runs that code (a compile of native-frame code that makes none is not a compile).
        if part == Part::Native && self.refused(id) {
            return self.frame_instead(id, sampled);
        }
        let hits = self.generator.cache_hits();
        let ok = self.compile_part(id, sampled, part);
        if part == Part::Native && !ok && self.refused(id) {
            return self.frame_instead(id, sampled);
        }
        if ok
            && part == Part::Native
            && self.generator.cache_hits() != hits
            && !self.done[k]
            && self.natural_part(id) == Part::Native
            && self.prog.func(id).is_some_and(|f| loops(&self.prog, f))
        {
            // (Inside a probe the generators link only already.)
            let probing = std::mem::replace(&mut self.probing, true);
            if !probing {
                self.cached_only(true);
            }
            self.compile_part(id, false, Part::Frame);
            if !probing {
                self.cached_only(false);
            }
            self.probing = probing;
        }
        ok
    }

    /// The frame-mode code of a function that has no native-frame variant, when compiled code
    /// calls it: the call is a step out of that code either way, and the step lands in code
    /// instead of an interpreter frame. Called by the interpreter alone it gains little (entering
    /// compiled code costs about what it saves, the reason `Tune::frame_calls` is off) and it can
    /// lose: the frame-mode code of a function with a loop of natives, called from interpreted
    /// callers, ran 1.4 times the instructions of the interpreter. Left for later: `see` asks for
    /// it again once a caller is compiled.
    fn frame_instead(&mut self, id: FuncId, sampled: bool) -> bool {
        if !self.counted && !self.entered_from_code(id) {
            if let Some(tr) = &self.trace
                && !self.declined[id.0 as usize]
            {
                tr.line(format!(
                    "{id} `{}`: frame-mode code waits for a compiled caller",
                    self.prog.func(id).map_or("?", |f| f.name.as_str())
                ));
            }
            self.declined[id.0 as usize] = true;
            return false;
        }
        self.declined[id.0 as usize] = false;
        self.compile_part(id, sampled, Part::Frame)
    }

    /// Whether a function that calls `id` has compiled code that `id` is called from: frame-mode
    /// code, or native-frame code that is published.
    fn entered_from_code(&self, id: FuncId) -> bool {
        let from = |c: u32| {
            c != id.0
                && (self.table[c as usize].run.load(Ordering::Relaxed) != 0
                    || self.nat[c as usize].1
                    || self.copy_compiled[c as usize]
                    // (Native-frame code waits to be published for what it calls, so under the count
                    // rule made is enough, or each would wait for the other.)
                    || (self.counted && self.nat[c as usize].0 != 0))
        };
        self.force_once();
        let yes = self.graph().callers[id.0 as usize].iter().any(|&c| from(c));
        if cfg!(debug_assertions) && self.check_calls {
            assert_eq!(
                yes,
                self.callers_by_walk(id, from) > 0,
                "entered_from_code({id})"
            );
        }
        yes
    }

    /// `YAVS_AUTO_CHECK_CALLS` (debug builds): the callers of `id` that `keep` holds, by a walk over
    /// the code of every function, each of them optimised already, as `graph` assumes.
    fn callers_by_walk(&self, id: FuncId, keep: impl Fn(u32) -> bool) -> usize {
        let mut n = 0;
        for (k, raw) in self.prog.funcs.iter() {
            if !keep(k.0) {
                continue;
            }
            assert!(
                raw.opt.get().is_some(),
                "{k} is a caller the policy asks about but was not optimised"
            );
            n += usize::from(self.prog.func(k).is_some_and(|f| {
                f.code.iter().any(|i| {
                    matches!(i, yavs_ir::Inst::Call { func, .. } | yavs_ir::Inst::CallProt { func, .. } if *func == id)
                })
            }));
        }
        n
    }

    /// `force_reach`, once: the processes and what their optimised code calls, and so on down.
    /// The rest of the program (library subprograms no process calls) cannot run but by an
    /// indirect call, which `Calls` does not hold either.
    fn force_once(&self) {
        if !self.force_reach.replace(false) {
            return;
        }
        let mut seen = vec![false; self.prog.funcs.len()];
        let mut todo: Vec<u32> = (self.prog.funcs.iter())
            .filter(|(_, f)| f.kind == FuncKind::Process && !f.spare)
            .map(|(k, _)| k.0)
            .collect();
        for &k in &todo {
            seen[k as usize] = true;
        }
        let mut i = 0;
        while let Some(&f) = todo.get(i) {
            i += 1;
            if self
                .prog
                .funcs
                .get(FuncId(f))
                .is_some_and(|g| g.opt.get().is_none())
            {
                self.native.count_stat(CountStat::ForceReach);
            }
            for inst in self.prog.func(FuncId(f)).map_or(&[][..], |g| &g.code[..]) {
                if let yavs_ir::Inst::Call { func, .. } | yavs_ir::Inst::CallProt { func, .. } =
                    inst
                    && seen.get(func.0 as usize) == Some(&false)
                {
                    seen[func.0 as usize] = true;
                    todo.push(func.0);
                }
            }
        }
    }

    /// Optimise ahead, beside the simulation, the functions the last run needed (`Profile::ran`),
    /// in the order it first optimised them, instead of what the processes reach (`force_reach`).
    fn prefetch(&mut self) {
        if self.earlier.ran.is_empty() {
            return;
        }
        self.force_reach.set(false);
        for &k in &self.earlier.ran {
            let _ = self.prog.func(FuncId(k));
        }
    }

    /// Who calls whom (`Calls`), with the functions optimised since the last ask taken in.
    fn graph(&self) -> std::cell::Ref<'_, Calls> {
        let new = self.prog.newly_optimised(&mut self.graph.borrow_mut().seen);
        if !new.is_empty() {
            let mut m = self.graph.borrow_mut();
            for f in new {
                let k = f.0;
                let Some(code) = self.prog.func(f).map(|g| &g.code) else {
                    continue;
                };
                let copy = self.prog.base_of(f) != f;
                for i in code {
                    if let yavs_ir::Inst::Call { func, .. } | yavs_ir::Inst::CallProt { func, .. } =
                        i
                        && !m.callees[k as usize].contains(&func.0)
                    {
                        m.callers[func.0 as usize].push(k);
                        m.callees[k as usize].push(func.0);
                        if copy {
                            self.native.count_stat(CountStat::CopyCalls);
                        }
                    }
                }
            }
        }
        self.graph.borrow()
    }

    /// Take over the native-frame code a compile made of other functions on the way.
    fn keep_extra(&mut self, entry: &crate::native::Compiled) {
        for &(g, native, call) in &entry.extra {
            let k = g.0 as usize;
            if native != 0 && self.nat.get(k).is_some_and(|n| n.0 == 0) {
                // Made with the root, so its compile call later finds nothing to do and
                // charges nothing.
                self.spent += self.price(g);
                self.nat[k] = (native, false);
                self.nat_by[k] = true;
                self.calls[k] = call;
            }
        }
    }

    fn compile_part(&mut self, id: FuncId, sampled: bool, part: Part) -> bool {
        // Native-frame code and frame-mode code are modules of their own, so the compilation
        // cache holds each one whatever was compiled with it (#510). Native-frame code is made
        // once.
        if self.lost.get(id.0 as usize).copied().unwrap_or(false) {
            return false;
        }
        let native_done = self.nat.get(id.0 as usize).is_some_and(|n| n.0 != 0)
            && self.nat_by.get(id.0 as usize).is_some_and(|&made| made);
        if part == Part::Both {
            if !native_done && self.nset(id).contains(&id) {
                let ok = self.compile(id, sampled, Part::Native);
                // No native-frame variant: `compile` made its frame-mode code.
                if self.refused(id) {
                    return ok;
                }
                // Refused (a sampled function too short, say): no frame-mode code either.
                if !ok && sampled && !self.probing {
                    return false;
                }
            }
            let frame = self.compile(id, sampled, Part::Frame);
            return frame || self.nat.get(id.0 as usize).is_some_and(|n| n.0 != 0);
        }
        // Each level once; elaboration code never runs again.
        let first = if part == Part::Native {
            !native_done
        } else {
            self.done
                .get_mut(id.0 as usize)
                .is_some_and(|d| !std::mem::replace(d, true))
        };
        let Some(f) = self
            .prog
            .func(id)
            .filter(|f| first && f.kind != FuncKind::Elab)
        else {
            return false;
        };
        let few = (self.is_rtl(id) && !self.early_root)
            .then(|| {
                let seen = self.group[self.prog.base_of(id).0 as usize];
                cold(seen, self.total, self.tune)
            })
            .flatten();
        let waived = called_natively(&self.prog, f, id) && !self.refused(id);
        let short = sampled
            .then(|| {
                // (A function with no native-frame variant is not called natively: no waiver.)
                crate::rt::too_short(&self.prog, f)
                    .filter(|_| !waived)
                    .or_else(|| fused_search(&self.prog, f))
                    .or_else(|| too_many_callbacks(&self.prog, f, id, self.tune.wait_same))
                    .or(few)
            })
            .flatten();
        let short_only = short.is_some() && too_big(f, self.max_size).is_none();
        let refused = too_big(f, self.max_size).or(short);
        if let (Some(why), Some(tr)) = (&refused, &mut self.trace)
            && tr.refused.insert(id)
        {
            let seen = self.time.get(id.0 as usize).map_or(0, |t| t[0]);
            tr.line(format!(
                "refused f{} `{}` ({} IR), seen {seen} us of {}: {why}{}",
                id.0,
                f.name,
                f.code.len(),
                self.total,
                if f.kind == FuncKind::Process && why.contains("leave the compiled") {
                    format!(" [{}]", leave_kinds(&self.prog, f, self.tune.wait_same))
                } else {
                    String::new()
                }
            ));
        }
        // (What a compile and a link from the cache are charged: the generator borrows `self` below.)
        let (prices, charges, model_cost, kind, list_key, frame_need) = (
            [self.price_of(id, false), self.price_of(id, true)],
            [
                self.cpu_price(id, part, false),
                self.cpu_price(id, part, true),
            ],
            self.cost(id),
            self.kind(id),
            self.list_key(id, f),
            self.frame_need(id),
        );
        // A refused function does not open the generator.
        let gen_ = &mut self.generator;
        let opened = if refused.is_none() {
            let opened = gen_.open();
            if !std::mem::replace(&mut self.loaded, true) {
                self.spent += gen_.load_us();
            }
            opened
        } else {
            Ok(())
        };
        if let Err(e) = opened {
            // Said once, not for every hot function; they all stay interpreted.
            if !std::mem::replace(&mut self.warned, true) {
                eprintln!("warning: --tier=auto: {e}; hot functions stay interpreted");
            }
            (self.note)(id, Err(e));
            return false;
        }
        let t0 = std::time::Instant::now();
        let cpu0 = self.trace.as_ref().and_then(|_| cpu_thread());
        let hits = gen_.cache_hits();
        if self.trace.is_some() && refused.is_none() {
            in_flight(Some(format!(
                "{id} `{}` ({} IR) {part:?}",
                f.name,
                f.code.len()
            )));
        }
        let r = match refused {
            Some(why) => Err(why),
            None => gen_.compile(&self.prog, id, f, part),
        };
        if self.trace.is_some() {
            in_flight(None);
        }
        match r {
            // Native-frame code that came to nothing: the generator refused the function or a
            // callee it must call directly. Nothing was compiled, so nothing is charged or
            // listed; the callees it did make go on below, and `compile` makes frame-mode code.
            Ok(entry) if part == Part::Native && entry.native == 0 && entry.run.is_none() => {
                let k = id.0 as usize;
                if !self.no_native[k] {
                    self.no_native[k] = true;
                    self.refusals += 1;
                }
                if let Some(tr) = &self.trace {
                    tr.line(format!(
                        "{id} no native-frame variant of `{}` ({} IR)",
                        f.name,
                        f.code.len()
                    ));
                }
                self.keep_extra(&entry);
                if !entry.extra.is_empty() {
                    self.publish();
                }
                false
            }
            Ok(entry) => {
                let took = t0.elapsed();
                let cpu = cpu0
                    .and_then(|c| cpu_thread()?.checked_sub(c))
                    .map_or(0, |d| d.as_micros());
                // A module linked from the compilation cache says nothing about compile cost.
                let hit = gen_.cache_hits() != hits;
                // Charged by the model, not by the time it took (#670). What the profile asked for
                // is not charged: the budget is for what the looks find.
                if !self.early_root {
                    self.spent += prices[usize::from(hit)];
                }
                self.paid += charges[usize::from(hit)];
                if let Some(tr) = &self.trace {
                    let seen = self.time.get(id.0 as usize).map_or([0, 0], |t| *t);
                    let base = match self.prog.base_of(id) {
                        b if b != id => format!(", copy of {b}"),
                        _ => String::new(),
                    };
                    let base = format!(
                        "{base}{}{}",
                        if f.parent.is_some() { ", nested" } else { "" },
                        if loops(&self.prog, f) { ", loops" } else { "" }
                    );
                    let leave = (f.kind == FuncKind::Process)
                        .then(|| callbacks(&self.prog, f, self.tune.wait_same))
                        .map_or(String::new(), |(o, t)| {
                            format!(
                                ", {o} of {t} leave [{}]",
                                leave_kinds(&self.prog, f, self.tune.wait_same)
                            )
                        });
                    tr.line(format!(
                        "{id} compiled{} `{}` ({} IR{leave}{base}) {part:?}{} {:?} in {} us, cpu {} us (model {}), first seen {} ms and candidate {} ms ago, \
                         seen {}/{} us, spent {} of {} us",
                        if self.early_root { " early" } else { "" },
                        f.name,
                        f.code.len(),
                        if hit { " hit" } else { "" },
                        kind,
                        took.as_micros(),
                        cpu,
                        model_cost,
                        tr.since(&tr.first, id),
                        tr.since(&tr.cand, id),
                        seen[0],
                        seen[1],
                        self.spent,
                        self.total
                    ));
                }
                // (A copy linked as the twin of another is listed under its own name too.)
                let key = list_key;
                if gen_.cache_enabled() && self.listed.insert(key.clone()) {
                    gen_.cache_note(&key.0, key.1);
                    self.in_cache[id.0 as usize] = Default::default();
                }
                if part != Part::Native && self.early_loop(id, f) {
                    self.native.count_stat(CountStat::LoopEarly);
                }
                if part == Part::Native
                    && sampled
                    && !self.done[id.0 as usize]
                    && loops(&self.prog, f)
                {
                    self.frame_at[id.0 as usize] = self.time[id.0 as usize][0] + frame_need;
                }
                self.keep_extra(&entry);
                if entry.native != 0 {
                    let k = id.0 as usize;
                    // Published when what it calls is compiled (`publish`); code already
                    // published is replaced at once.
                    let was = self.nat[k].1;
                    self.nat[k] = (entry.native, was);
                    self.nat_by[k] = true;
                    self.calls[k] = entry.call;
                    if was {
                        self.ntab[k].store(entry.native, Ordering::Release);
                        if entry.call != 0 {
                            self.ctab[k].store(entry.call, Ordering::Release);
                            self.native.set_call(id);
                        }
                    }
                }
                if entry.native != 0 || !entry.extra.is_empty() {
                    self.publish();
                }
                if let Some(run) = entry.run {
                    let lean = self.prog.func(id).is_some_and(crate::regs::no_table);
                    self.table[id.0 as usize].set((run, entry.nregs), lean);
                    self.native.set(id);
                    if self.counted {
                        self.copy_compiled[self.prog.base_of(id).0 as usize] = true;
                    }
                }
                if self.early_root && !self.early_done.contains(&id) {
                    self.early_done.push(id);
                }
                (self.note)(id, Ok(took));
                true
            }
            Err(_) if self.probing => {
                if part != Part::Native {
                    self.done[id.0 as usize] = false;
                }
                false
            }
            Err(why) => {
                // A short function stays interpreted as long as nothing compiled calls it;
                // it compiles with a caller (`tree`).
                if short_only && part != Part::Native {
                    self.done[id.0 as usize] = false;
                }
                (self.note)(id, Err(why));
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

/// The sampler thread (#670): at every look it reads which function each simulation thread runs and
/// the work it did since the last look, and sends the looks to the compile thread in batches of
/// `BATCH`. It runs while the compile thread compiles or measures a trial (no looks are sent during a
/// trial), so the compile thread's view of the run does not depend on how long it was busy.
///
/// The work is the instructions each simulation thread retired (`work`), charged to the function in
/// that thread's slot; a look is taken when the busiest running thread has done `period` microseconds
/// of work, not after a wall time, so a loaded host moves the looks in time and not in the program
/// (the polls that find the point are a fraction of a period apart). Without the counters the clock
/// is the CPU time of the simulation threads, shared by the running functions, and the wall time
/// between looks without that.
fn sampler_loop(
    sample: impl Fn(&mut Vec<(usize, FuncId, bool)>),
    base_period: Duration,
    tx: Sender<Msg>,
    sh: &Shared,
) {
    timer_slack_1us();
    // (The counters are opened here, after START_AFTER: a run under 5 ms opens none.)
    let work = match crate::work::activate() {
        Ok(()) => true,
        Err(why) => {
            if std::env::var_os("YAVS_AUTO_TRACE").is_some() {
                eprintln!("trace: work clock off, CPU time instead: {why}");
            }
            *sh.clock_off.lock().unwrap_or_else(PoisonError::into_inner) = Some(why);
            false
        }
    };
    let clock = || sim_cpu(sh.sim_clock);
    let (mut last_cpu, mut last_wall) = (clock(), std::time::Instant::now());
    let mut last: Vec<(usize, u64)> = Vec::new();
    crate::work::per_thread(&mut last);
    // The work seen so far (microseconds), which sets how far apart the looks are (`stretch`).
    let mut done_us: u64 = 0;
    let mut cpu_carry: u64 = 0;
    let mut batch = Vec::with_capacity(BATCH);
    let mut running = Vec::new();
    let mut counts = Vec::new();
    let mut slots = Vec::new();
    while !sh.halt.load(Ordering::SeqCst) {
        let period = base_period * stretch(done_us);
        let target = u64::try_from(period.as_micros())
            .unwrap_or(u64::MAX)
            .saturating_mul(crate::work::per_us());
        let since = |counts: &[(usize, u64)], last: &[(usize, u64)], i: usize| {
            let at = |v: &[(usize, u64)]| v.iter().find(|e| e.0 == i).map_or(0, |e| e.1);
            at(counts).saturating_sub(at(last))
        };
        if work {
            // Poll until the busiest running thread has done a period of work: the first sleep a
            // little short of a period, the next ones as long as the rate seen says is left.
            let t0 = std::time::Instant::now();
            let mut step = period.mul_f64(0.95);
            loop {
                nap(sh, step);
                running.clear();
                sample(&mut running);
                slots.clear();
                slots.extend(running.iter().map(|r| r.0));
                counts.clear();
                crate::work::of_threads(&slots, &mut counts);
                let busiest = running
                    .iter()
                    .map(|r| since(&counts, &last, r.0))
                    .max()
                    .unwrap_or(0);
                let waited = t0.elapsed();
                if busiest >= target || waited >= period * 4 || sh.halt.load(Ordering::SeqCst) {
                    break;
                }
                let rate = busiest as f64 / waited.as_micros().max(1) as f64;
                step = if rate > 0.0 {
                    Duration::from_micros(((target - busiest) as f64 / rate * 0.95) as u64)
                        .max(Duration::from_micros(150))
                        .min(period)
                } else {
                    period
                };
            }
        } else {
            nap(sh, period);
            if sh.halt.load(Ordering::SeqCst) {
                return;
            }
            running.clear();
            sample(&mut running);
            counts.clear();
        }
        let (cpu, wall) = (clock(), std::time::Instant::now());
        if let Some(n) = crate::work::of_thread(sh.sim_tid) {
            sh.read_instr.store(n, Ordering::Relaxed);
        }
        sh.read_cpu_us.store(
            cpu.map_or(0, |c| u64::try_from(c.as_micros()).unwrap_or(u64::MAX)),
            Ordering::Relaxed,
        );
        let mut look = if work {
            let running: Vec<_> = running
                .iter()
                .map(|&(i, f, c)| {
                    // (A worker that spun before it got to run counts for at most two periods.)
                    let d = since(&counts, &last, i).min(2 * target);
                    (f, c, crate::work::work_us(d))
                })
                .collect();
            let cpu = running.iter().map(|r| r.2).max().unwrap_or(0);
            Look { running, cpu }
        } else {
            let dt = match (cpu, last_cpu) {
                (Some(c), Some(l)) => c.saturating_sub(l),
                _ => wall - last_wall,
            };
            shared_look(u64::try_from(dt.as_micros()).unwrap_or(u64::MAX), &running)
        };
        if let (Some(c), Some(l)) = (cpu, last_cpu) {
            look.cpu = u64::try_from(c.saturating_sub(l).as_micros()).unwrap_or(u64::MAX);
        }
        // (The CPU time of a look not sent goes with the next one sent.)
        look.cpu += std::mem::take(&mut cpu_carry);
        (last_cpu, last_wall) = (cpu, wall);
        done_us += look.running.iter().map(|r| r.2).max().unwrap_or(0);
        for &(i, c) in &counts {
            match last.iter_mut().find(|e| e.0 == i) {
                Some(e) => e.1 = c,
                None => last.push((i, c)),
            }
        }
        // (Nor a look with no work while a compile holds the simulation, nor any look timed by the
        // wall clock then: the batches end where they would with the compile on another core.)
        let wall_timed = !work && cpu.is_none();
        if sh.trial.load(Ordering::SeqCst)
            || (yavs_interp::simulation_held()
                && (wall_timed || look.running.iter().all(|r| r.2 == 0)))
        {
            cpu_carry = look.cpu;
            continue;
        }
        batch.push(look);
        if batch.len() == BATCH && tx.send(Msg::Looks(std::mem::take(&mut batch))).is_err() {
            return;
        }
    }
}

/// How many periods apart the looks are once `done_us` microseconds of work were seen: the first
/// 300 ms are looked at every period, to 1 s every second one, then every fourth (the sampler shares the
/// simulation's core under `taskset`, and a run that has not found a candidate by then is unlikely
/// to, so each look costs more than it finds). A function of the work seen, so decisions stay a
/// function of the looks.
fn stretch(done_us: u64) -> u32 {
    match done_us {
        0..STRETCH_2 => 1,
        STRETCH_2..STRETCH_4 => 2,
        _ => 4,
    }
}

const STRETCH_2: u64 = 300_000;
const STRETCH_4: u64 = 1_000_000;

/// The calling thread's timer slack to 1 us (the default 50 us late-wakes every sleep): the
/// sampler's polls land close to the work they wait for.
#[cfg(target_os = "linux")]
fn timer_slack_1us() {
    unsafe extern "C" {
        fn prctl(option: i32, ...) -> i32;
    }
    const PR_SET_TIMERSLACK: i32 = 29;
    // SAFETY: a plain prctl of the calling thread; a failure leaves the default slack.
    unsafe { prctl(PR_SET_TIMERSLACK, 1000u64) };
}

#[cfg(not(target_os = "linux"))]
fn timer_slack_1us() {}

/// The calling thread to the lowest OS priority (`--tier-compile=background`): a host with every
/// core busy runs the simulations and the desktop first. A failure leaves the priority as it is.
#[cfg(target_os = "linux")]
fn lowest_priority() {
    unsafe extern "C" {
        fn setpriority(which: u32, who: u32, prio: i32) -> i32;
    }
    const PRIO_PROCESS: u32 = 0;
    // SAFETY: with `who` 0 Linux sets the nice value of the calling thread only (it is per thread).
    unsafe { setpriority(PRIO_PROCESS, 0, 19) };
}

#[cfg(windows)]
fn lowest_priority() {
    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetCurrentThread() -> *mut std::ffi::c_void;
        fn SetThreadPriority(thread: *mut std::ffi::c_void, priority: i32) -> i32;
    }
    const THREAD_PRIORITY_LOWEST: i32 = -2;
    // SAFETY: the pseudo handle of the calling thread needs no closing.
    unsafe { SetThreadPriority(GetCurrentThread(), THREAD_PRIORITY_LOWEST) };
}

#[cfg(not(any(target_os = "linux", windows)))]
fn lowest_priority() {}

/// Where `--tier=auto` compiles (`AutoTier::set_compile`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Compile {
    /// The simulation waits at its next pause point (`yavs_interp::pause_point`) while a function
    /// compiles: the run keeps at most its own threads busy.
    Wait,
    /// Alongside the run, on the tier-up thread at the lowest OS priority.
    Background,
}

/// `Shared::compile`: 0 not set (the tests: alongside the run at the usual priority), or a `Compile`.
const COMPILE_WAIT: u8 = 1;
const COMPILE_BACKGROUND: u8 = 2;

impl AutoTier {
    /// Start the compile thread and make the tier the runtime's (`Runtime::set_tier`),
    /// compiling the functions `heat` finds hot with `generator`; functions over
    /// `max_size` IR instructions stay interpreted. `note` is called on the compile thread for
    /// every compile. The thread waits [`START_AFTER`] before it does anything.
    pub fn start_with(
        rt: &Runtime,
        max_size: usize,
        heat: Heat,
        note: Note,
        generator: Box<dyn Generator>,
    ) -> Result<Arc<AutoTier>, StartError> {
        let tier = Self::start_deferred_with(rt, max_size, heat, note, generator)?;
        tier.start_thread(true)?;
        Ok(tier)
    }

    /// Like [`AutoTier::start_with`], but the compile thread is not started: a thread costs a twentieth
    /// of a run of a millisecond, and most runs end before it would look at anything. The caller
    /// calls [`AutoTier::launch`] once the run has lasted [`START_AFTER`] (from a thread it has
    /// anyway); [`AutoTier::stop`] needs no launch. An eager `heat` (tests) starts it at once.
    pub fn start_deferred_with(
        rt: &Runtime,
        max_size: usize,
        heat: Heat,
        note: Note,
        generator: Box<dyn Generator>,
    ) -> Result<Arc<AutoTier>, StartError> {
        // The generator is opened by the compile thread at the first compile
        // (`Worker::compile`): a run in which nothing gets hot never pays for opening it.
        let prog = rt.prog.clone();
        let funcs: Arc<[Entry]> = (0..prog.funcs.len()).map(|_| Entry::default()).collect();
        let (tx, rx) = channel::<Msg>();
        let shared = Arc::new(Shared {
            finished: (0..prog.funcs.len())
                .map(|_| AtomicBool::new(false))
                .collect(),
            // (The tier is made on the kernel thread, the one that runs the simulation.)
            sim_clock: own_cpu_clock(),
            sim_tid: crate::work::thread_id(),
            ..Shared::default()
        });
        let ntab: Arc<[AtomicUsize]> = (0..prog.funcs.len()).map(|_| AtomicUsize::new(0)).collect();
        let ctab: Arc<[AtomicUsize]> = (0..prog.funcs.len()).map(|_| AtomicUsize::new(0)).collect();
        // Threads that get a sampling slot note their ids; the sampler opens their counters later
        // (`work`), so a run of a few milliseconds pays one system call per thread.
        if !heat.eager() {
            crate::work::install();
        }
        let (table, sh) = (funcs.clone(), shared.clone());
        let (sample, sampler_tx) = (rt.thread_sampler(), tx.clone());
        let native = rt.native_flags();
        // How long the compile thread waits for news before it looks at `halt`.
        let period = if heat.eager() {
            Duration::from_micros(1 << 20)
        } else {
            Duration::from_millis(20)
        };
        let sample_every = Duration::from_micros(heat.period.into());
        // The count rule is not on in the test mode without a budget.
        let counted = count_rule_on() && heat.budget != 0;
        let (count, gate, sync) = count_knobs();
        let sync = counted && sync;
        if counted {
            rt.native_flags().arm_count(
                &rt.prog,
                count,
                u64::from(count / 2).max(2),
                (count / 2).max(1),
            );
            if gate == 0 {
                rt.native_flags().open_copies();
            }
        }
        let mut w = Worker::new(
            prog,
            table,
            ntab.clone(),
            ctab.clone(),
            native,
            heat,
            max_size,
            generator,
            note,
        );
        (w.counted, w.gate, w.count) = (counted, gate, count);
        w.sim_clock = shared.sim_clock;
        if let Some(n) = std::env::var("YAVS_AUTO_WARM_GATE")
            .ok()
            .and_then(|v| v.parse().ok())
        {
            w.gate_cpu = n;
        }
        (w.guard_share, w.guard_calls) = guard_knobs();
        let knob = |name: &str| std::env::var(name).ok().and_then(|v| v.parse::<f64>().ok());
        if let Some(p) = knob("YAVS_AUTO_WAIT_PAYBACK") {
            w.wait_payback = p;
        }
        if let Some(p) = knob("YAVS_AUTO_WAIT_LONG_FLOOR") {
            w.wait_long_floor = p;
        }
        if let Some(p) = knob("YAVS_AUTO_WAIT_SHARE") {
            w.wait_share = p;
        }
        if let Some(p) = knob("YAVS_AUTO_WAIT_LONG_SHARE") {
            w.wait_long_share = p;
        }
        if let Some(p) = knob("YAVS_AUTO_WAIT_GAIN") {
            w.wait_gain = p;
        }
        if let Some(p) = knob("YAVS_AUTO_WAIT_SUBTREE") {
            w.wait_subtree = p != 0.0;
        }
        if let Some(p) = knob("YAVS_AUTO_WAIT_TREE") {
            w.wait_tree = p;
        }
        if let Some(p) = knob("YAVS_AUTO_WAIT_HELD_RUNS") {
            w.wait_held_runs = p as u32;
        }
        if let Some(p) = knob("YAVS_AUTO_WAIT_BIG") {
            w.wait_big = p;
        }
        if let Some(p) = knob("YAVS_AUTO_WAIT_HELD_FREE") {
            w.wait_held_free = p != 0.0;
        }
        if let Some(p) = knob("YAVS_AUTO_WAIT_ORDER") {
            w.wait_order = p != 0.0;
        }
        let spawn = move |wait: bool| {
            std::thread::Builder::new()
                .name("yavs-tier-up".into())
                .spawn(move || {
                    // Nothing a run of a few milliseconds samples pays for a compile, and moving the
                    // thread, reading the cache's list and waking every millisecond cost a tenth of
                    // such a run: do none of it before `START_AFTER` (#426). `stop` ends the wait.
                    let mut pending = Pending::default();
                    let mut looks: Vec<Look> = Vec::new();
                    let t0 = std::time::Instant::now();
                    match rx.recv_timeout(if heat.eager() || !wait {
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
                                if wait {
                                    nap(&sh, START_AFTER.saturating_sub(t0.elapsed()));
                                }
                                if !sh.halt.load(Ordering::SeqCst) {
                                    sampler_loop(sample, sample_every, sampler_tx, &sh);
                                }
                            })
                            .unwrap_or_else(|e| panic!("cannot start the sampler thread: {e}"))
                    });
                    // (The end of the run wakes this thread: nothing to list then.)
                    if !sh.halt.load(Ordering::SeqCst) {
                        w.listed = w.generator.cache_list();
                        w.load_profile();
                        w.prefetch();
                        if w.counted && (w.long_before() || w.named_hot()) {
                            w.native.open_copies();
                        }
                    }
                    if let (Some(key), Some(store)) = (w.profile_key, w.store.clone()) {
                        let cpu0 = sim_cpu(sh.sim_clock).map_or(0, |c| u64::try_from(c.as_micros()).unwrap_or(0));
                        *sh.profile.lock().unwrap_or_else(PoisonError::into_inner) =
                            Some(ProfileJob {
                                store,
                                key,
                                order: w.profile_order.clone(),
                                earlier: w.earlier.clone(),
                                cpu0,
                                snap: Snapshot::default(),
                            });
                    }
                    let publish = |w: &Worker| {
                        if w.profile_key.is_some()
                            && let Some(job) = sh
                                .profile
                                .lock()
                                .unwrap_or_else(PoisonError::into_inner)
                                .as_mut()
                        {
                            job.snap = w.snapshot();
                        }
                    };
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
                    let mut unfinished: Vec<FuncId> = Vec::new();
                    let mut lowered = false;
                    // `--tier-compile=wait` paces the compiles (below): wait for the next looks.
                    let mut paced = false;
                    loop {
                        while let Ok(m) = rx.try_recv() {
                            route(m, &mut pending, &mut inbox);
                        }
                        if inbox.is_empty() && (pending.asked.is_empty() || paced) {
                            match rx.recv_timeout(period) {
                                Ok(m) => route(m, &mut pending, &mut inbox),
                                Err(RecvTimeoutError::Timeout) => {}
                                Err(RecvTimeoutError::Disconnected) => break,
                            }
                        }
                        w.stops = sh.compile.load(Ordering::Relaxed) == COMPILE_WAIT
                            && heat.budget != 0
                            && !sync;
                        // One batch at a time, each followed by the decisions: what is decided depends
                        // on the looks in order and not on how long a compile took (#670).
                        for look in inbox.pop_front().unwrap_or_default() {
                            w.see(&look, &mut pending);
                        }
                        let free = w.freed();
                        pending.asked.extend(free);
                        w.guard();
                        loop {
                            publish(&w);
                            while let Ok(m) = rx.try_recv() {
                                route(m, &mut pending, &mut inbox);
                            }
                            // A run that waits for its compiles gets no more of them than a thread
                            // of their own could make: the compile time (the model's) at most the
                            // simulation time seen. Not with a waiting `Tier::hot` (eager, sync).
                            let was = paced;
                            paced = sh.compile.load(Ordering::Relaxed) == COMPILE_WAIT
                                && heat.budget != 0
                                && !sync
                                && w.paid as f64 > w.sim_us as f64 * w.share();
                            if let (true, Some(tr)) = (was != paced, &w.trace) {
                                tr.line(format!(
                                    "paced {}: paid {} us, spent {} us, seen {} us, looked {} us, simulation CPU {:?}, {} asked",
                                    if paced { "on" } else { "off" },
                                    w.paid,
                                    w.spent,
                                    w.total,
                                    w.sim_us,
                                    sim_cpu(sh.sim_clock),
                                    pending.asked.len()
                                ));
                            }
                            if paced {
                                break;
                            }
                            let over = std::cell::Cell::new(false);
                            let probe = if pending.asked.is_empty() {
                                pending.probes.pop()
                            } else {
                                None
                            };
                            let mut early = false;
                            let Some((ids, sampled)) = probe
                                .map(|id| (vec![(id, Part::Both)], true))
                                .or_else(|| {
                                    let mut k = 0;
                                    while let Some(&id) = pending.asked.get(k) {
                                        if !w.counted || !w.waits(id) {
                                            k += 1;
                                            continue;
                                        }
                                        pending.asked.remove(k);
                                        w.note_unrepaid(id);
                                        if !w.held[id.0 as usize] {
                                            w.held[id.0 as usize] = true;
                                            w.native.count_stat(CountStat::Held);
                                            if let Some(tr) = &w.trace {
                                                tr.line(w.held_why(id));
                                            }
                                        }
                                        w.waiting.push(id);
                                        // (A run that waits for a compile goes on interpreted.)
                                        if let Some(f) = sh.finished.get(id.0 as usize) {
                                            f.store(true, Ordering::Release);
                                        }
                                    }
                                    let Some(k) = w.next_asked(&pending.asked) else {
                                        // (None fits the budget: wait for more looks.)
                                        if !pending.asked.is_empty() {
                                            over.set(true);
                                            w.native.count_stat(CountStat::OverBudget);
                                        }
                                        return None;
                                    };
                                    let id = pending.asked.remove(k)?;
                                    w.note_unlocked(id);
                                    Some((vec![(id, w.ask(id))], false))
                                })
                                .or_else(|| {
                                    w.next_early(&mut pending).map(|ids| {
                                        early = true;
                                        (ids, true)
                                    })
                                })
                                .or_else(|| {
                                    w.pick(&mut pending.cands).map(|ids| (ids, true))
                                })
                            else {
                                paced = over.get();
                                // (All that was left to do for them waits, `YAVS_AUTO_SYNC`.)
                                for id in unfinished.drain(..) {
                                    if let Some(f) = sh.finished.get(id.0 as usize) {
                                        f.store(true, Ordering::Release);
                                    }
                                }
                                break;
                            };
                            // `--tier-compile=wait`: the compile (not a trial) runs while the run
                            // waits at a pause point; `background`: alongside it, at low priority.
                            let held = match sh.compile.load(Ordering::Relaxed) {
                                COMPILE_WAIT => match yavs_interp::hold_simulation(&sh.halt) {
                                    Some(h) => Some(h),
                                    None => break,
                                },
                                COMPILE_BACKGROUND => {
                                    if !lowered {
                                        lowest_priority();
                                        lowered = true;
                                    }
                                    None
                                }
                                _ => None,
                            };
                            w.native.count_stat(if held.is_some() {
                                CountStat::Waited
                            } else {
                                CountStat::Alongside
                            });
                            // `busy` before `halt` here and `halt` before `busy` in `stop` (both
                            // SeqCst): either this thread sees the halt or `stop` sees it busy.
                            sh.busy.store(true, Ordering::SeqCst);
                            if sh.halt.load(Ordering::SeqCst) {
                                break;
                            }
                            if let Some(id) = probe {
                                let linked = w.probe(id);
                                drop(held);
                                if linked {
                                    pending.cands.retain(|&c| c != id);
                                    sh.trial.store(true, Ordering::SeqCst);
                                    w.trial(id, &sh.halt);
                                    sh.trial.store(false, Ordering::SeqCst);
                                }
                                sh.busy.store(false, Ordering::SeqCst);
                                continue;
                            }
                            let first = ids.first().map(|&(id, _)| id);
                            w.announce(&ids);
                            let mut root_done = false;
                            w.early_root = early;
                            for (i, (id, part)) in ids.into_iter().enumerate() {
                                let done = w.compile(id, sampled && i == 0, part);
                                root_done |= done && i == 0;
                                // Its callees compile for its sake only.
                                if !done && i == 0 {
                                    break;
                                }
                            }
                            w.early_root = false;
                            drop(held);
                            if let (Some(id), true) = (first, w.counted && root_done && !sampled) {
                                w.native.count_stat(CountStat::Compiled);
                                if w.prog.base_of(id) != id {
                                    w.native.count_stat(CountStat::Copies);
                                }
                            }
                            let free = w.freed();
                            pending.asked.extend(free);
                            // (Not a root the profile asked for: its share of the run is not known
                            // yet, and the run as a whole is judged at its end.)
                            if let (Some(id), true) =
                                (first, root_done && sampled && !early)
                            {
                                sh.trial.store(true, Ordering::SeqCst);
                                w.trial(id, &sh.halt);
                                sh.trial.store(false, Ordering::SeqCst);
                            }
                            // `YAVS_AUTO_SYNC`: what this compile freed comes before the run goes on.
                            if sync && !pending.asked.is_empty() {
                                unfinished.extend(first);
                                continue;
                            }
                            for id in unfinished.drain(..).chain(first) {
                                if let Some(f) = sh.finished.get(id.0 as usize) {
                                    f.store(true, Ordering::Release);
                                }
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
                                let id = FuncId(k as u32);
                                tr.line(format!(
                                    "seen f{k} `{}` ({} IR{}, loops {}%): interpreted {} us, compiled {} us, {:?}, candidate at {} us",
                                    f.name,
                                    f.code.len(),
                                    match w.prog.base_of(id) {
                                        b if b.0 as usize != k => format!(", copy of {b}"),
                                        _ => String::new(),
                                    },
                                    loop_share(&w.prog, f),
                                    w.time[k][0],
                                    w.time[k][1],
                                    w.kind(id),
                                    w.threshold(id),
                                ));
                            }
                        }
                        let mut m: Vec<(u64, usize)> = (0..w.time.len())
                            .map(|k| (w.native.materialised(FuncId(k as u32)), k))
                            .filter(|x| x.0 != 0)
                            .collect();
                        m.sort_unstable();
                        for &(n, k) in m.iter().rev().take(20) {
                            let name = w.prog.func(FuncId(k as u32)).map_or("?", |f| f.name.as_str());
                            tr.line(format!(
                                "f{k} `{name}`: {n} native frames left to the interpreter, seen {:?}",
                                w.time[k]
                            ));
                        }
                        tr.line(format!(
                            "sampled {} us, compiled for {} us, paid {} us of {} us looked at (clock: {} instructions and {} us of CPU time of the kernel thread)",
                            w.total,
                            w.spent,
                            w.paid,
                            w.sim_us,
                            sh.read_instr.load(Ordering::Relaxed),
                            sh.read_cpu_us.load(Ordering::Relaxed),
                        ));
                    }
                    *sh.generator.lock().unwrap_or_else(PoisonError::into_inner) =
                        Some(w.generator);
                    sh.busy.store(false, Ordering::SeqCst);
                })
                .map_err(|e| StartError(format!("cannot start the compile thread: {e}")))
        };
        let tier = Arc::new(AutoTier {
            queued: (0..funcs.len()).map(|_| AtomicBool::new(false)).collect(),
            sync,
            funcs,
            ntab,
            ctab,
            queue: Mutex::new(Some(tx)),
            shared,
            worker: Mutex::new(Thread::Idle(Box::new(spawn))),
        });
        if !rt.set_tier(tier.clone()) {
            tier.stop(true);
            return Err(StartError(
                "the runtime has a tier already (a bug in yavs)".into(),
            ));
        }
        if heat.eager() {
            rt.set_eager();
        }
        if heat.eager() || sync {
            // `Tier::hot` of an eager tier waits for the compile: the thread cannot wait for the run.
            tier.start_thread(false)?;
        }
        Ok(tier)
    }

    /// Why the policy's clock is the CPU time of the simulation threads and not their instructions
    /// retired (`work.rs`), when it is; `None` with the work clock.
    pub fn clock_note(&self) -> Option<String> {
        self.shared
            .clock_off
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// End tier-up: functions still queued stay interpreted, and compiled code stays usable.
    /// Call it when the simulation is over. Returns `true` once the compile thread has ended;
    /// `false` if it is compiling a function and `wait` is false: it then finishes that on its
    /// own, and the process must not run its exit handlers under the running LLVM
    /// ([`exit_now`]).
    pub fn stop(&self, wait: bool) -> bool {
        // (The simulation's CPU time first: the compile thread's clock is gone once it has ended.)
        let cpu = self.sim_cpu_now();
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
        self.write_profile(cpu);
        if std::env::var_os("YAVS_AUTO_TRACE").is_some()
            && let Some((what, t0)) = &*IN_FLIGHT.lock().unwrap_or_else(PoisonError::into_inner)
        {
            eprintln!(
                "trace: the run ends while compiling {what}, {} ms in; {}",
                t0.elapsed().as_millis(),
                if wait {
                    "waits for it"
                } else {
                    "it is not finished"
                }
            );
        }
        // A compile thread that died in a compile (a panic) is joined: its panic is the run's.
        let dead = matches!(
            &*self.worker.lock().unwrap_or_else(PoisonError::into_inner),
            Thread::Running(h) if h.is_finished()
        );
        if !wait && !dead && self.shared.busy.load(Ordering::SeqCst) {
            return false;
        }
        let worker = std::mem::replace(
            &mut *self.worker.lock().unwrap_or_else(PoisonError::into_inner),
            Thread::Ended,
        );
        // (The compile thread ends the sampler.) One that was never started has nothing to end.
        if let Thread::Running(h) = worker
            && let Err(p) = h.join()
        {
            resume_unwind(p);
        }
        true
    }

    /// The simulation CPU so far (`sim_cpu`), in microseconds; `None` where there is no such clock.
    fn sim_cpu_now(&self) -> Option<u64> {
        u64::try_from(sim_cpu(self.shared.sim_clock)?.as_micros()).ok()
    }

    /// Write the profile of this run (`profile::next`) from what the compile thread published, once:
    /// from here and not from the compile thread, which may be in the middle of a compile when the
    /// run ends. Nothing for a run whose CPU time is not known or too short to judge.
    fn write_profile(&self, cpu: Option<u64>) {
        let job = self
            .shared
            .profile
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        let (Some(job), Some(end)) = (job, cpu) else {
            return;
        };
        let cpu = end.saturating_sub(job.cpu0);
        if cpu < profile::JUDGE_MIN_US {
            return;
        }
        let mut p = profile::next(&job.earlier, &job.snap, cpu);
        if std::env::var_os("YAVS_AUTO_TRACE").is_some() {
            eprintln!(
                "trace: profile: stored {} hot ({} judged), {} lost; this run {cpu} us, last {:?} us, {} compiled early",
                p.hot.len(),
                p.hot.iter().filter(|h| h.2).count(),
                p.lost.len(),
                job.earlier.base,
                job.snap.early.len()
            );
        }
        let rank: std::collections::HashMap<u32, u32> =
            (0..).zip(&job.order).map(|(r, &id)| (id, r)).collect();
        p.remap(|id| rank.get(&id).copied());
        profile::save(&job.store, &job.key, &p.render());
    }

    /// Start the compile thread of [`AutoTier::start_deferred`], which then begins at once; does
    /// nothing if it has been started or the run is over. Call it once the run has lasted
    /// [`START_AFTER`].
    pub fn launch(&self) -> Result<(), StartError> {
        self.start_thread(false)
    }

    fn start_thread(&self, wait: bool) -> Result<(), StartError> {
        let mut state = self.worker.lock().unwrap_or_else(PoisonError::into_inner);
        // (`stop` sets `halt` before it takes the state: a launch after it finds `halt`, one
        // before it is joined.)
        if self.shared.halt.load(Ordering::SeqCst) || !matches!(*state, Thread::Idle(_)) {
            return Ok(());
        }
        let Thread::Idle(spawn) = std::mem::replace(&mut *state, Thread::Ended) else {
            return Ok(());
        };
        *state = Thread::Running(spawn(wait)?);
        Ok(())
    }

    /// Where functions compile; the tier compiles alongside the run at the usual priority until
    /// this is called (the tests).
    pub fn set_compile(&self, compile: Compile) {
        let v = match compile {
            Compile::Wait => COMPILE_WAIT,
            Compile::Background => COMPILE_BACKGROUND,
        };
        self.shared.compile.store(v, Ordering::Relaxed);
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
    fn compiles_copies(&self) -> bool {
        true
    }

    fn run(&self, func: FuncId, pc: u32, host: &mut dyn TierHost) -> Option<u32> {
        let (f, nregs) = self.funcs.get(func.0 as usize)?.get()?;
        let lookup = |id: FuncId| self.funcs.get(id.0 as usize)?.get();
        // `AtomicUsize` has the layout of `usize`: compiled code loads the entries atomically.
        enter(
            f,
            nregs,
            pc,
            host,
            &lookup,
            self.ntab.as_ptr().cast(),
            false,
        )
    }

    fn direct(&self) -> Option<yavs_interp::DirectFn> {
        Some(crate::rt::direct_entry::<Self>)
    }

    #[inline(always)]
    fn run_cached(
        &self,
        func: FuncId,
        pc: u32,
        host: &mut yavs_interp::Host<'_, '_, '_>,
    ) -> Option<u32> {
        let (f, nregs, lean) = self.funcs.get(func.0 as usize)?.get_lean()?;
        let lookup = |id: FuncId| self.funcs.get(id.0 as usize)?.get();
        enter(f, nregs, pc, host, &lookup, self.ntab.as_ptr().cast(), lean)
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

    fn asked(&self, func: FuncId) {
        if self.sync {
            return self.hot(func);
        }
        let Some(q) = self.queued.get(func.0 as usize) else {
            return;
        };
        if !q.swap(true, Ordering::Relaxed)
            && let Some(q) = &*self.queue.lock().unwrap_or_else(PoisonError::into_inner)
        {
            let _ = q.send(Msg::Hot(func));
        }
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
            let dead = match &*self.worker.lock().unwrap_or_else(PoisonError::into_inner) {
                Thread::Running(h) => h.is_finished(),
                _ => true,
            };
            if dead {
                break;
            }
            // (`--tier-compile=wait`: the compile this waits for holds the simulation.)
            yavs_interp::pause_point();
            std::thread::sleep(Duration::from_micros(50));
        }
    }
}

#[cfg(test)]
mod policy_tests;

#[cfg(test)]
mod tests {
    /// The simulation CPU is the kernel thread's: a pool worker that spins while the kernel
    /// thread waits does not make the run look dearer to the policy.
    #[cfg(target_os = "linux")]
    #[test]
    fn the_simulation_cpu_leaves_out_other_threads() {
        use std::time::Duration;
        let clock = super::own_cpu_clock();
        let c0 = super::sim_cpu(clock).unwrap();
        std::thread::spawn(|| {
            let t0 = super::cpu_thread().unwrap();
            while super::cpu_thread().unwrap() - t0 < Duration::from_millis(60) {}
        })
        .join()
        .unwrap();
        let spent = super::sim_cpu(clock).unwrap() - c0;
        assert!(spent < Duration::from_millis(30), "{spent:?}");
    }

    /// A worker of the count rule: no generator, so nothing compiles; the rule's decisions only.
    fn counted(prog: yavs_ir::Program, gate: u64) -> super::Worker {
        let rt = yavs_interp::Runtime::new(std::sync::Arc::new(prog), Default::default()).unwrap();
        let n = rt.prog.funcs.len();
        let tab = || -> std::sync::Arc<[std::sync::atomic::AtomicUsize]> {
            (0..n)
                .map(|_| std::sync::atomic::AtomicUsize::new(0))
                .collect()
        };
        let mut w = super::Worker::new(
            rt.prog.clone(),
            (0..n).map(|_| super::Entry::default()).collect(),
            tab(),
            tab(),
            rt.native_flags(),
            super::Heat::DEFAULT,
            usize::MAX,
            Box::new(super::policy_tests::Stub),
            Box::new(|_, _| {}),
        );
        w.counted = true;
        w.gate = gate;
        w
    }

    /// A process that calls a function.
    fn caller_callee() -> (yavs_ir::Program, yavs_ir::FuncId, yavs_ir::FuncId) {
        let mut pb = ProgramBuilder::new();
        let g = pb.declare("g", yavs_ir::FuncKind::Procedure, None);
        let p = pb.declare("p", yavs_ir::FuncKind::Process, None);
        let mut fb = pb.define(g);
        fb.stmt(StmtKind::Return, Span::new(FileId(0), 0, 1));
        // Something of its own, so the optimiser keeps the call.
        let keep = fb.slot(8, 8);
        let at = fb.addr(yavs_ir::AddrSpace::Frame, keep);
        fb.emit(Inst::Store {
            addr: at,
            src: at,
            w: yavs_ir::Width::B8,
        });
        fb.emit(Inst::Ret { v: None });
        fb.finish();
        let mut fb = pb.define(p);
        fb.stmt(StmtKind::Return, Span::new(FileId(0), 0, 1));
        let args = fb.regs(&[]);
        fb.emit(Inst::Call {
            func: g,
            args,
            dst: None,
        });
        fb.emit(Inst::Ret { v: None });
        fb.finish();
        (pb.finish().unwrap(), p, g)
    }

    fn look(us: u64) -> super::Look {
        super::Look {
            running: vec![(yavs_ir::FuncId(0), false, us)],
            cpu: us,
        }
    }

    /// The hold: a subprogram waits for a compiled caller, whichever kind of code that is; a process
    /// does not.
    #[test]
    fn a_subprogram_waits_for_a_compiled_caller() {
        let (prog, caller, callee) = caller_callee();
        let mut w = counted(prog, 0);
        assert!(!w.waits(caller));
        assert!(w.waits(callee));
        w.waiting.push(callee);
        assert!(w.freed().is_empty());
        assert_eq!(w.waiting, [callee]);
        w.nat[caller.0 as usize].0 = 64;
        assert!(!w.waits(callee));
        assert_eq!(w.freed(), [callee]);
        assert!(w.waiting.is_empty());
    }

    /// A copy made after the first ask still counts as a caller: a subprogram called only from
    /// copies (copies of subprograms are) is freed once one of them has code.
    #[test]
    fn a_copy_made_later_frees_what_it_calls() {
        use yavs_ir::{AddrSpace, RegTy, Width};
        let mut pb = ProgramBuilder::new();
        let g = pb.declare("g", yavs_ir::FuncKind::Procedure, None);
        let p = pb.declare("p", yavs_ir::FuncKind::Process, None);
        let mut fb = pb.define(g);
        fb.stmt(StmtKind::Return, Span::new(FileId(0), 0, 1));
        let keep = fb.slot(8, 8);
        let at = fb.addr(AddrSpace::Frame, keep);
        fb.emit(Inst::Store {
            addr: at,
            src: at,
            w: Width::B8,
        });
        fb.emit(Inst::Ret { v: None });
        fb.finish();
        let mut fb = pb.define(p);
        fb.stmt(StmtKind::Wait, Span::new(FileId(0), 0, 1));
        let head = fb.block();
        fb.body(head);
        fb.emit(Inst::Br { to: head });
        fb.at(head);
        let a = fb.addr(AddrSpace::Inst, 0);
        let v = fb.reg(RegTy::I64);
        fb.emit(Inst::Load {
            dst: v,
            addr: a,
            w: Width::B8,
        });
        let args = fb.regs(&[]);
        fb.emit(Inst::Call {
            func: g,
            args,
            dst: None,
        });
        fb.emit(Inst::Suspend { resume: 0 });
        fb.emit(Inst::Br { to: head });
        fb.finish();
        let mut prog = pb.finish().unwrap();
        prog.reserve_copies(1);
        let mut w = counted(prog, 0);
        assert!(w.waits(g));
        let copy = w.prog.copy_of(p, &3i64.to_le_bytes(), &[]).expect("a copy");
        // (Compiled code is made from the optimised code, which lists the copy's calls.)
        assert!(w.prog.func(copy).is_some());
        w.nat[copy.0 as usize].0 = 64;
        w.waiting.push(g);
        assert_eq!(w.freed(), [g]);
    }

    /// With the last run's list of what it needed, asking who calls a function optimises no other
    /// function (`graph`, `prefetch`): a caller is known once it is optimised, which compiled code
    /// always is. Without the list the first ask optimises what the processes reach (`force_reach`).
    #[test]
    fn asking_for_callers_optimises_only_what_the_last_run_needed() {
        let opt = |w: &super::Worker, f: yavs_ir::FuncId| {
            w.prog.funcs.get(f).unwrap().opt.get().is_some()
        };
        let (prog, caller, callee) = caller_callee();
        let mut w = counted(prog, 0);
        w.check_calls = true;
        w.earlier.ran = vec![callee.0];
        w.prefetch();
        assert!(opt(&w, callee), "listed");
        assert!(w.waits(callee));
        assert!(
            !opt(&w, caller),
            "the caller was optimised to find the callers"
        );
        assert!(w.prog.func(caller).is_some());
        w.nat[caller.0 as usize].0 = 64;
        assert!(!w.waits(callee));
        let (prog, caller, callee) = caller_callee();
        let w = counted(prog, 0);
        assert!(w.waits(callee));
        assert!(opt(&w, caller), "no list: what the processes reach");
    }

    /// Without the last run's list, the first ask optimises the processes and what they call, and
    /// not a subprogram no process calls, nor what that one calls.
    #[test]
    fn the_first_ask_optimises_only_what_the_processes_reach() {
        let mut pb = ProgramBuilder::new();
        let body = |pb: &mut ProgramBuilder, f, call: Option<yavs_ir::FuncId>| {
            let mut fb = pb.define(f);
            fb.stmt(StmtKind::Return, Span::new(FileId(0), 0, 1));
            let keep = fb.slot(8, 8);
            let at = fb.addr(yavs_ir::AddrSpace::Frame, keep);
            fb.emit(Inst::Store {
                addr: at,
                src: at,
                w: yavs_ir::Width::B8,
            });
            if let Some(func) = call {
                let args = fb.regs(&[]);
                fb.emit(Inst::Call {
                    func,
                    args,
                    dst: None,
                });
            }
            fb.emit(Inst::Ret { v: None });
            fb.finish();
        };
        let g = pb.declare("g", yavs_ir::FuncKind::Procedure, None);
        let k = pb.declare("k", yavs_ir::FuncKind::Procedure, None);
        let h = pb.declare("h", yavs_ir::FuncKind::Procedure, None);
        let p = pb.declare("p", yavs_ir::FuncKind::Process, None);
        body(&mut pb, g, None);
        body(&mut pb, k, None);
        body(&mut pb, h, Some(k));
        body(&mut pb, p, Some(g));
        let mut w = counted(pb.finish().unwrap(), 0);
        w.check_calls = true;
        assert!(w.waits(g));
        let opt = |f: yavs_ir::FuncId| w.prog.funcs.get(f).unwrap().opt.get().is_some();
        assert!(opt(p) && opt(g), "reached");
        assert!(!opt(h) && !opt(k), "no process calls h");
        let reached = w.native.count_stats()[yavs_interp::CountStat::ForceReach as usize];
        assert!(reached > 0, "tier_force_reach");
    }

    /// A process that calls a procedure with a loop of its own.
    fn caller_looper() -> (yavs_ir::Program, yavs_ir::FuncId, yavs_ir::FuncId) {
        let mut pb = ProgramBuilder::new();
        let g = pb.declare("g", yavs_ir::FuncKind::Procedure, None);
        let p = pb.declare("p", yavs_ir::FuncKind::Process, None);
        let mut fb = pb.define(g);
        // loop n := n - 1; exit when n = 0; end loop (n a parameter).
        let n = fb.param(
            "n",
            yavs_ir::Width::B8,
            yavs_ir::ParamClass::Value,
            yavs_ir::Mode::In,
            None,
        );
        fb.stmt(StmtKind::Return, Span::new(FileId(0), 0, 1));
        let head = fb.block();
        let done = fb.block();
        fb.emit(Inst::Br { to: head });
        fb.at(head);
        let at = fb.addr(yavs_ir::AddrSpace::Frame, n);
        let v = fb.reg(yavs_ir::RegTy::I64);
        fb.emit(Inst::Load {
            dst: v,
            addr: at,
            w: yavs_ir::Width::B8,
        });
        let one = fb.int(1);
        let v1 = fb.reg(yavs_ir::RegTy::I64);
        fb.emit(Inst::Bin {
            op: yavs_ir::IntOp::Sub,
            dst: v1,
            a: v,
            b: one,
        });
        fb.emit(Inst::Store {
            addr: at,
            src: v1,
            w: yavs_ir::Width::B8,
        });
        fb.emit(Inst::CondBr {
            c: v1,
            t: head,
            f: done,
        });
        fb.at(done);
        fb.emit(Inst::Ret { v: None });
        fb.finish();
        let mut fb = pb.define(p);
        fb.stmt(StmtKind::Return, Span::new(FileId(0), 0, 1));
        let k = fb.int(1000);
        let args = fb.regs(&[k]);
        fb.emit(Inst::Call {
            func: g,
            args,
            dst: None,
        });
        fb.emit(Inst::Ret { v: None });
        fb.finish();
        (pb.finish().unwrap(), p, g)
    }

    /// A subprogram the watching loop found looping in one activation joins the profile's hot
    /// list (processes alone did before); a later run compiles it early as frame-mode code alone,
    /// for the running activation to enter.
    #[test]
    fn a_subprogram_that_loops_in_one_activation_is_hot_for_the_next_run() {
        let (prog, _, g) = caller_looper();
        let mut w = counted(prog, 0);
        w.bseen[g.0 as usize] = super::profile::HOT_MIN_US;
        w.hotish.push(g.0);
        assert!(
            w.snapshot().seen.is_empty(),
            "a subprogram that did not loop"
        );
        w.native.mark_looped(g);
        w.total = super::profile::HOT_MIN_US * super::LOOP_SHARE + 1;
        assert!(w.snapshot().seen.is_empty(), "under its share of the run");
        w.total = super::profile::HOT_MIN_US * super::LOOP_SHARE;
        assert_eq!(w.snapshot().seen, [(g.0, super::profile::HOT_MIN_US)]);
        let f = w.prog.func(g).unwrap();
        assert!(super::loops(&w.prog, f));
        assert!(w.native_ok(g));
        assert_eq!(w.natural_part(g), super::Part::Native);
        assert_eq!(w.asked_part(g), super::Part::Native);
        w.early.insert(g, super::profile::HOT_MIN_US);
        assert_eq!(w.natural_part(g), super::Part::Frame);
        assert_eq!(w.asked_part(g), super::Part::Native);
    }

    /// A subprogram of the profile is compiled early only with a loop of its own: the mark is a
    /// sample, and a busy subprogram without a loop is marked too.
    #[test]
    fn only_a_subprogram_with_a_loop_is_compiled_early() {
        // A straight subprogram long enough to be `Kind::Native` (a speedup the filter takes).
        let mut pb = ProgramBuilder::new();
        let g = pb.declare("g", yavs_ir::FuncKind::Function, None);
        let mut fb = pb.define(g);
        let x = fb.param(
            "x",
            yavs_ir::Width::B8,
            yavs_ir::ParamClass::Value,
            yavs_ir::Mode::In,
            None,
        );
        fb.stmt(StmtKind::Return, Span::new(FileId(0), 0, 1));
        let at = fb.addr(yavs_ir::AddrSpace::Frame, x);
        let mut r = fb.reg(yavs_ir::RegTy::I64);
        fb.emit(Inst::Load {
            dst: r,
            addr: at,
            w: yavs_ir::Width::B8,
        });
        for k in 0..2 * crate::rt::SHORT as i64 {
            let c = fb.int(k + 3);
            let d = fb.reg(yavs_ir::RegTy::I64);
            fb.emit(Inst::Bin {
                op: yavs_ir::IntOp::Mul,
                dst: d,
                a: r,
                b: c,
            });
            r = d;
        }
        fb.emit(Inst::Ret { v: Some(r) });
        fb.finish();
        let w = counted(pb.finish().unwrap(), 0);
        assert!(
            w.model(g).speedup >= super::profile::EARLY_SPEEDUP,
            "{:?}",
            w.kind(g)
        );
        assert!(!w.early_kind(g), "no loop of its own");
        let (prog, p, g) = caller_looper();
        let w = counted(prog, 0);
        assert!(w.early_kind(g));
        assert!(w.early_kind(p), "processes keep their own filter");
    }

    /// A subprogram the profile lost is not priced as a link of the cache: its code stays there,
    /// and a later run would link it at once though compiling it early gained nothing.
    #[test]
    fn a_subprogram_the_profile_lost_is_not_priced_as_a_link() {
        struct Cached;
        impl crate::Generator for Cached {
            fn compile(
                &mut self,
                _: &yavs_ir::Program,
                _: yavs_ir::FuncId,
                _: &yavs_ir::Func,
                _: super::Part,
            ) -> Result<crate::native::Compiled, String> {
                Err("stub".into())
            }
            fn cache_enabled(&self) -> bool {
                true
            }
        }
        let fresh = |lost: bool| {
            let (prog, p, g) = caller_looper();
            let mut w = counted(prog, 0);
            w.generator = Box::new(Cached);
            for id in [g, p] {
                let key = w.list_key(id, w.prog.func(id).unwrap());
                w.listed.insert(key);
                if lost {
                    w.earlier.lost.push(id.0);
                }
            }
            (w, p, g)
        };
        let (w, p, g) = fresh(false);
        assert!(w.cached(g) && w.cached(p));
        let (w, p, g) = fresh(true);
        assert!(!w.cached(g), "a subprogram the profile lost");
        assert!(w.cached(p), "a process lost by its trial keeps its price");
    }

    /// The profile lists what a run needed, in the order it was optimised: what ran or compiled.
    #[test]
    fn the_profile_lists_what_ran() {
        let (prog, caller, callee) = caller_callee();
        let mut w = counted(prog, 0);
        w.force_reach.set(false);
        assert!(w.prog.func(callee).is_some() && w.prog.func(caller).is_some());
        assert!(w.snapshot().ran.is_empty(), "optimised, not run");
        w.time[caller.0 as usize][0] = 1;
        w.nat[callee.0 as usize].0 = 64;
        assert_eq!(w.snapshot().ran, [callee.0, caller.0]);
    }

    /// `YAVS_AUTO_CHECK_CALLS` stops at a caller with code that was never optimised: the map would
    /// not know it.
    #[test]
    #[should_panic(expected = "was not optimised")]
    fn the_callers_check_stops_at_a_caller_the_map_cannot_know() {
        let (prog, caller, callee) = caller_callee();
        let mut w = counted(prog, 0);
        (w.check_calls, w.force_reach) = (true, false.into());
        w.nat[caller.0 as usize].0 = 64;
        let _ = w.entered_from_code(callee);
    }

    /// The count gives a function with a loop native-frame code alone; its frame-mode code (for the
    /// interpreter to enter again where a native frame of it was left) is asked for once one was.
    #[test]
    fn frame_code_of_an_asked_loop_waits_for_a_left_frame() {
        use super::{CountStat, Part};
        let (prog, f) = padded_loop(100);
        let mut w = counted(prog, 0);
        assert_eq!(w.ask(f), Part::Native);
        assert_eq!(w.deferred, [f]);
        let k = f.0 as usize;
        w.nat[k].0 = 64;
        assert!(w.freed().is_empty());
        assert_eq!(w.deferred, [f]);
        w.native.note_left(f);
        assert_eq!(w.freed(), [f]);
        assert!(w.deferred.is_empty());
        // Asked again: native-frame code exists, so the frame-mode code now.
        assert_eq!(w.ask(f), Part::Both);
        let stats = w.native.count_stats();
        assert_eq!(stats[CountStat::FrameDeferred as usize], 1);
        assert_eq!(stats[CountStat::FrameLater as usize], 1);
        // Without a budget (`--tier-heat`) both parts at once, as for a sampled root.
        let (prog, g) = padded_loop(100);
        let mut h = counted(prog, 0);
        h.heat.budget = 0;
        assert_eq!(h.ask(g), Part::Both);
        assert!(h.deferred.is_empty());
    }

    /// `--tier-compile=wait` (`stops`): what the count asks for and the cache does not hold waits
    /// until the looks saw it for `WAIT_PAYBACK` of its price (once the run lasted `WAIT_LONG`, only a
    /// process seen under `WAIT_LONG_FLOOR` of that), in the first run of a program only; a cached
    /// function and a run that does not stop are as before.
    #[test]
    fn a_run_that_stops_compiles_what_its_looks_repaid() {
        let (prog, caller, callee) = caller_callee();
        let mut w = counted(prog, 0);
        let k = caller.0 as usize;
        let at = (w.price_of(caller, false) as f64 * super::WAIT_PAYBACK).ceil() as u64;
        assert!(at > 0);
        assert!(!w.waits(caller), "a run that does not stop");
        w.stops = true;
        w.time[k][0] = at - 1;
        assert!(w.waits(caller));
        w.total = super::WAIT_LONG;
        // Past it a process waits only while it and its copies were seen under the floor.
        w.bseen[k] = (at as f64 * super::WAIT_LONG_FLOOR).ceil() as u64;
        assert!(!w.waits(caller), "a long run");
        w.bseen[k] -= 1;
        assert!(w.waits(caller), "a long run, a process hardly seen");
        assert!(!w.unpaid(callee), "a long run, a subprogram");
        w.bseen[k] = 0;
        w.total = 0;
        w.earlier.base = Some(1);
        assert!(!w.waits(caller), "a run after an earlier one");
        w.earlier.base = None;
        w.time[k][0] = at;
        assert!(!w.waits(caller));
        w.gate = u64::MAX;
        assert!(w.waits(caller), "the gate");
        // A link: not seen at all, it waits for the gate only.
        w.time[k][0] = 0;
        w.in_cache = vec![Default::default(); w.in_cache.len()];
        w.in_cache[k].set(true).unwrap();
        assert!(w.waits(caller));
        w.gate = 0;
        assert!(!w.waits(caller));
    }

    /// Past `WAIT_LONG` the first run also holds a process until the callees its compile pulls in
    /// (`unlocked`) were seen with it for `WAIT_TREE` times their price (`tree_unpaid`); not with the
    /// rule off, nor in a later run.
    #[test]
    fn a_first_run_holds_a_process_until_the_tree_it_pulls_in_repays() {
        let (prog, caller, callee) = caller_callee();
        let mut w = counted(prog, 0);
        let (k, j) = (caller.0 as usize, callee.0 as usize);
        (w.stops, w.total) = (true, super::WAIT_LONG);
        // Past the floor by itself.
        (w.bseen[k], w.time[k][0]) = (w.price_of(caller, false), 1);
        assert!(!w.waits(caller), "no callee seen");
        w.time[j][0] = 1;
        let price = w.body_price(caller) + w.body_price(callee);
        assert_eq!(w.unlocked(caller), (2, price));
        assert!(w.waits(caller), "a tree seen for less than its price");
        w.note_unrepaid(caller);
        assert_eq!(
            w.native.count_stats()[yavs_interp::CountStat::TreeHeld as usize],
            1
        );
        w.time[j][0] = (price as f64 * super::WAIT_TREE * super::WAIT_PAYBACK).ceil() as u64;
        assert!(!w.waits(caller), "the tree repaid");
        w.time[j][0] = 1;
        w.wait_tree = 0.0;
        assert!(!w.waits(caller), "the rule off");
        w.wait_tree = super::WAIT_TREE;
        w.earlier.base = Some(1);
        assert!(!w.unpaid(caller), "a later run");
    }

    /// A later run that stops for its compiles (`unrepaid`) holds a function the cache does not hold
    /// while its share of the looks times the rest of the run the profile predicts, times the gain and
    /// the runs the cache serves, is below its compile; the first run, a run that does not stop, a
    /// link and a gain of 0 are as before.
    #[test]
    fn a_later_run_compiles_what_the_rest_of_it_repays() {
        let (prog, caller, _) = caller_callee();
        let mut w = counted(prog, 0);
        let k = caller.0 as usize;
        let body = w.body_price(caller) as f64;
        assert!(body > 0.0);
        (w.stops, w.wait_gain, w.wait_payback) = (true, 0.5, 0.0);
        // A tenth of the looks: the rest must hold body / (0.1 * 0.5 * EARLY_RUNS).
        (w.total, w.time[k][0]) = (1000, 100);
        let need = (body / (0.1 * 0.5 * super::profile::EARLY_RUNS as f64)).ceil() as u64;
        assert!(!w.waits(caller), "no profile: the first run's rules");
        w.sim_us = 7;
        w.earlier.base = Some(need + 7 - 1);
        assert!(w.waits(caller), "the rest is too short");
        w.earlier.base = Some(need + 7 + 1);
        assert!(!w.waits(caller));
        w.sim_us = 10;
        assert!(w.waits(caller), "the run went on");
        // What the earlier runs that held it saw counts, and so do the runs it was held in.
        w.wait_held_runs = u32::MAX;
        let before = (body / 0.5).ceil() as u64 * w.total / w.sim_us;
        w.carried.insert(caller, (before, 1));
        assert!(!w.waits(caller), "the earlier runs saw it for its price");
        w.carried.insert(caller, (0, 2));
        assert!(w.waits(caller));
        w.wait_held_runs = 2;
        assert!(!w.waits(caller), "held in two runs");
        (w.wait_held_runs, w.carried) = (super::WAIT_HELD_RUNS, Default::default());
        w.wait_gain = 0.0;
        assert!(!w.waits(caller), "off");
        w.wait_gain = 0.5;
        w.stops = false;
        assert!(!w.waits(caller), "a run that does not stop");
        w.stops = true;
        w.in_cache = vec![Default::default(); w.in_cache.len()];
        w.in_cache[k].set(true).unwrap();
        assert!(!w.waits(caller), "a link");
    }

    /// A later run judges a held caller with the callees that wait only for it (`unlocked`): their
    /// time and their price; not once compiled code calls them, nor with the rule off.
    #[test]
    fn a_later_run_credits_a_caller_with_the_callees_it_unlocks() {
        let (prog, caller, callee) = caller_callee();
        let mut w = counted(prog, 0);
        let (k, j) = (caller.0 as usize, callee.0 as usize);
        let price = w.body_price(caller) + w.body_price(callee);
        (w.stops, w.wait_gain, w.wait_payback) = (true, 0.5, 0.0);
        (w.total, w.sim_us, w.time[k][0]) = (1000, 10, 1);
        w.earlier.base = Some(10 + (price as f64 / (0.5 * 0.5 * 4.0)).ceil() as u64 + 1);
        assert_eq!(super::profile::EARLY_RUNS, 4);
        assert_eq!(
            w.unlocked(caller),
            (1, w.body_price(caller)),
            "a callee never seen"
        );
        assert!(w.waits(caller));
        w.time[j][0] = 499;
        assert_eq!(w.unlocked(caller), (500, price));
        assert!(!w.waits(caller), "half of the looks are in what it unlocks");
        w.note_unlocked(caller);
        assert_eq!(
            w.native.count_stats()[yavs_interp::CountStat::Unlocked as usize],
            1
        );
        w.wait_subtree = false;
        assert!(w.waits(caller), "by its own time");
        w.wait_subtree = true;
        w.nat[j].0 = 64;
        assert_eq!(w.unlocked(caller).0, 1, "a compiled callee");
        assert!(w.waits(caller));
    }

    /// The next compile of a run that stops for them (`next_asked`): links first, then the most seen
    /// per price of those that fit the budget, none when none fits; the oldest when not ordered.
    #[test]
    fn a_run_that_stops_compiles_the_hottest_per_price_that_fits() {
        let (prog, caller, callee) = caller_callee();
        let mut w = counted(prog, 0);
        let (a, b) = (w.body_price(caller), w.body_price(callee));
        assert!(a > 0 && b > 0);
        let q: std::collections::VecDeque<_> = [caller, callee].into();
        (w.stops, w.wait_order, w.wait_share, w.wait_long_share) = (true, true, 1.0, 1.0);
        w.time[caller.0 as usize][0] = a;
        w.time[callee.0 as usize][0] = 2 * b;
        w.sim_us = a + b;
        assert_eq!(w.next_asked(&q), Some(1), "the callee is hotter per price");
        w.sim_us = a.min(b) - 1;
        assert_eq!(w.next_asked(&q), None, "none fits");
        // A function whose process the earlier runs held needs no room.
        w.earlier.base = Some(1);
        w.wait_held_runs = 1;
        w.carried.insert(w.prog.base_of(caller), (0, 1));
        assert_eq!(w.next_asked(&q), Some(0), "overdue");
        w.wait_held_runs = super::WAIT_HELD_RUNS;
        (w.earlier.base, w.carried) = (None, Default::default());
        // A long run gets the long run's budget.
        // (Share enough for both, whatever their prices.)
        let both = 1.0 + a.max(b) as f64 / a.min(b) as f64;
        (w.wait_long_share, w.total) = (both, super::WAIT_LONG);
        assert_eq!(w.next_asked(&q), Some(1));
        (w.wait_long_share, w.total) = (1.0, 0);
        w.sim_us = a.max(b);
        w.paid = a.max(b) - a.min(b);
        let cheap = if a <= b { 0 } else { 1 };
        assert_eq!(w.next_asked(&q), Some(cheap), "only the cheaper fits");
        w.paid = 0;
        w.wait_order = false;
        w.sim_us = a + b;
        assert_eq!(w.next_asked(&q), Some(0), "the oldest");
        w.wait_order = true;
        w.in_cache = vec![Default::default(); w.in_cache.len()];
        w.in_cache[caller.0 as usize].set(true).unwrap();
        w.sim_us = 0;
        assert_eq!(w.next_asked(&q), Some(0), "a link needs no room");
        w.stops = false;
        assert_eq!(
            w.next_asked(&q),
            Some(0),
            "a run that does not stop takes them in order"
        );
    }

    /// A warm run links what the cache lists before the gate only if its own last run was long: the
    /// list is shared by all the tests of a library.
    #[test]
    fn only_a_long_last_run_links_before_the_gate() {
        let (prog, caller, _) = caller_callee();
        let mut w = counted(prog, u64::MAX);
        w.in_cache[caller.0 as usize].set(true).unwrap();
        assert!(w.waits(caller), "no last run");
        w.earlier.base = Some(super::COUNT_GATE_CPU - 1);
        assert!(w.waits(caller), "a short last run");
        w.earlier.base = Some(super::COUNT_GATE_CPU);
        assert!(!w.waits(caller));
        w.in_cache = vec![Default::default(); w.in_cache.len()];
        w.in_cache[caller.0 as usize].set(false).unwrap();
        assert!(w.waits(caller), "not in the cache");
    }

    /// A short last run links before the gate what its own profile named hot, and nothing else the
    /// cache lists.
    #[test]
    fn a_short_last_run_links_its_own_hot_processes() {
        let (prog, caller, callee) = caller_callee();
        let mut w = counted(prog, u64::MAX);
        w.in_cache[caller.0 as usize].set(true).unwrap();
        w.in_cache[callee.0 as usize].set(true).unwrap();
        w.earlier.base = Some(super::COUNT_GATE_CPU - 1);
        assert!(w.waits(caller) && !w.named_hot());
        w.early.insert(caller, 1);
        assert!(!w.waits(caller), "named hot");
        assert!(w.named_hot());
        assert!(w.waits(callee), "not named");
    }

    /// A function that ran keeps its compiled native callers unpublished until it has code: it is
    /// asked for at once, not left to wait for its count (a rare callee would hold them for good).
    #[test]
    fn a_callee_that_holds_up_publication_is_wanted() {
        let (prog, top, leaf) = native_chain();
        let mut w = counted(prog, 0);
        w.nat[top.0 as usize] = (64, false);
        w.count = 100;
        w.native.set_count(leaf, 100);
        w.publish();
        assert_ne!(
            w.ntab[top.0 as usize].load(std::sync::atomic::Ordering::Acquire),
            0,
            "the callee has not run"
        );
        w.nat[top.0 as usize] = (64, false);
        w.ntab[top.0 as usize].store(0, std::sync::atomic::Ordering::Release);
        w.native.tick(leaf.0);
        w.publish();
        assert_eq!(
            w.ntab[top.0 as usize].load(std::sync::atomic::Ordering::Acquire),
            0
        );
        assert_eq!(w.freed(), [leaf]);
        w.publish();
        assert!(w.freed().is_empty(), "asked for once");
        w.nat[leaf.0 as usize] = (32, false);
        w.publish();
        assert_eq!(
            w.ntab[top.0 as usize].load(std::sync::atomic::Ordering::Acquire),
            64
        );
    }

    /// A procedure that runs on native frames and calls another.
    fn native_chain() -> (yavs_ir::Program, yavs_ir::FuncId, yavs_ir::FuncId) {
        let mut pb = ProgramBuilder::new();
        let leaf = pb.declare("leaf", yavs_ir::FuncKind::Procedure, None);
        let top = pb.declare("top", yavs_ir::FuncKind::Procedure, None);
        let mut fb = pb.define(leaf);
        fb.stmt(StmtKind::Return, Span::new(FileId(0), 0, 1));
        let keep = fb.slot(8, 8);
        let at = fb.addr(yavs_ir::AddrSpace::Frame, keep);
        fb.emit(Inst::Store {
            addr: at,
            src: at,
            w: yavs_ir::Width::B8,
        });
        fb.emit(Inst::Ret { v: None });
        fb.finish();
        let mut fb = pb.define(top);
        fb.stmt(StmtKind::Return, Span::new(FileId(0), 0, 1));
        let args = fb.regs(&[]);
        fb.emit(Inst::Call {
            func: leaf,
            args,
            dst: None,
        });
        fb.emit(Inst::Ret { v: None });
        fb.finish();
        (pb.finish().unwrap(), top, leaf)
    }

    /// The gate is a count of work, read from the looks: not a clock.
    #[test]
    fn the_gate_opens_with_the_work_the_looks_saw() {
        let (prog, caller, _) = caller_callee();
        let mut w = counted(prog, 1000);
        let mut pending = super::Pending::default();
        assert!(w.waits(caller));
        w.see(&look(999), &mut pending);
        assert!(w.waits(caller));
        w.see(&look(1), &mut pending);
        assert!(!w.waits(caller));
    }

    /// A worker with published native-frame code of `f`, which the look at `f` will judge.
    fn guarded() -> (super::Worker, yavs_ir::FuncId) {
        let (prog, _, g) = caller_callee();
        let mut w = counted(prog, 0);
        let k = g.0 as usize;
        w.nat[k] = (64, true);
        w.ntab[k].store(64, std::sync::atomic::Ordering::Release);
        w.ctab[k].store(128, std::sync::atomic::Ordering::Release);
        w.native.set_call(g);
        (w, g)
    }

    fn work_on(w: &mut super::Worker, us: u64) {
        let mut pending = super::Pending::default();
        w.see(&look(us), &mut pending);
    }

    fn published(w: &super::Worker, g: yavs_ir::FuncId) -> bool {
        w.ntab[g.0 as usize].load(std::sync::atomic::Ordering::Acquire) != 0
    }

    /// Frames enough to be an eighth of `us` of work.
    fn frames_for(us: u64) -> usize {
        (us * crate::work::per_us() / 8 / super::LEFT_COST) as usize
    }

    /// A worker that took `f` back after a stretch of frames, its calls now counted.
    fn on_trial() -> (super::Worker, yavs_ir::FuncId, usize) {
        let (mut w, g) = guarded();
        let n = frames_for(100_000);
        // The first look: published since then, so the first stretch is not judged.
        work_on(&mut w, 100_000);
        w.guard();
        for _ in 0..n {
            w.native.note_left(g);
        }
        work_on(&mut w, 100_000);
        w.guard();
        assert!(!published(&w, g));
        assert!(w.lost[g.0 as usize]);
        (w, g, n)
    }

    /// Few frames: the code stays. A stretch it was not published for all of: not judged.
    #[test]
    fn native_code_that_leaves_now_and_then_stays() {
        let (mut w, g) = guarded();
        work_on(&mut w, 100_000);
        for _ in 0..frames_for(100_000) {
            w.native.note_left(g);
        }
        w.guard();
        assert!(published(&w, g));
        work_on(&mut w, 100_000);
        w.native.note_left(g);
        w.guard();
        assert!(published(&w, g));
    }

    /// More calls than frames per work, but not many: it left at most calls and the run gains without
    /// it, so it stays out for good.
    #[test]
    fn native_code_that_leaves_at_most_calls_is_taken_back() {
        let (mut w, g, n) = on_trial();
        for _ in 0..2 * n {
            w.native.tick(g.0);
        }
        work_on(&mut w, 100_000);
        w.guard();
        assert!(!published(&w, g));
        assert!(w.lost[g.0 as usize]);
        assert!(w.trying.is_empty());
        assert_eq!(w.native.count_left(g), 0);
        assert_eq!(
            w.native.count_stats()[yavs_interp::CountStat::Withdrawn as usize],
            1
        );
    }

    /// Many calls per frame: native code gains, back it goes, and it is not judged again.
    #[test]
    fn native_code_called_often_comes_back() {
        let (mut w, g, n) = on_trial();
        for _ in 0..n * super::GUARD_CALLS as usize {
            w.native.tick(g.0);
        }
        work_on(&mut w, 100_000);
        w.guard();
        assert!(published(&w, g));
        assert!(!w.lost[g.0 as usize]);
        assert_eq!(
            w.native.count_stats()[yavs_interp::CountStat::Withdrawn as usize],
            0
        );
        work_on(&mut w, 100_000);
        for _ in 0..n {
            w.native.note_left(g);
        }
        w.guard();
        assert!(published(&w, g));
    }

    /// As many calls as frames per work, or fewer: the run is no faster without it. Back it goes.
    #[test]
    fn native_code_that_is_no_slower_comes_back() {
        let (mut w, g, n) = on_trial();
        for _ in 0..n {
            w.native.tick(g.0);
        }
        work_on(&mut w, 100_000);
        w.guard();
        assert!(published(&w, g));
    }

    /// The looks go on deciding under the rule: a loop the count does not reach still becomes a
    /// candidate by its time seen (`compute_h2h`, `xor_sig`, `xor_types` under a plain count).
    #[test]
    fn a_long_loop_stays_with_the_sampler_under_the_count() {
        let (prog, f) = padded_loop(40);
        let mut w = counted(prog, u64::MAX);
        let mut pending = super::Pending::default();
        w.see(
            &super::Look {
                running: vec![(f, false, 10_000_000)],
                cpu: 10_000_000,
            },
            &mut pending,
        );
        assert_eq!(pending.cands, [f]);
    }

    use super::{
        BY_ENTRIES, BY_RUNS, COMPUTE_BUDGET, NATIVE_BUDGET, Pace, Rule, Trial, Tune, UNSEEN_MIN,
        band_wait, cold, compute, decide, first_fit, fused_search, left_out, loops, must_calls,
        run_trial, too_many_callbacks, within_budget,
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
            let why = too_many_callbacks(&prog, prog.func(f).unwrap(), f, false);
            assert_eq!(why.is_some(), refused, "{ops} {outs}: {why:?}");
        }
    }

    /// The `WaitOn`s of a sensitivity list (one per signal) are no callbacks when compiled code
    /// skips the whole wait (`yavs_rt_wait_same`, #1046): a body of 100 instructions and a list of
    /// 30 signals is compiled with `wait_same`, refused without.
    #[test]
    fn a_long_sensitivity_list_is_one_way_out() {
        use yavs_ir::{Inst, RegTy};
        let mut pb = ProgramBuilder::new();
        let f = pb.declare("p", yavs_ir::FuncKind::Process, None);
        let mut fb = pb.define(f);
        fb.flag(yavs_ir::FuncFlags::SENS_LIST);
        fb.stmt(StmtKind::Wait, Span::new(FileId(0), 0, 1));
        let x = fb.int(1);
        for _ in 0..100 {
            fb.emit(Inst::Bin {
                op: yavs_ir::IntOp::Add,
                dst: x,
                a: x,
                b: x,
            });
        }
        let wait = fb.block();
        fb.emit(Inst::Br { to: wait });
        fb.at(wait);
        fb.stmt(StmtKind::Wait, Span::new(FileId(0), 0, 1));
        let (s, off, len) = (fb.reg(RegTy::Sig), fb.int(0), fb.int(1));
        fb.emit(Inst::SigVal {
            dst: s,
            sig: 0,
            offset: off,
            len,
        });
        for _ in 0..30 {
            fb.emit(Inst::WaitOn { sig: s });
        }
        fb.emit(Inst::Suspend { resume: 0 });
        fb.emit(Inst::Br { to: wait });
        fb.finish();
        let prog = pb.finish().unwrap();
        let p = prog.func(f).unwrap();
        assert_eq!(yavs_interp::static_wait(&prog, p, 1).len(), 1);
        assert!(too_many_callbacks(&prog, p, f, false).is_some());
        assert_eq!(too_many_callbacks(&prog, p, f, true), None);
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
            let why = too_many_callbacks(&prog, prog.func(f).unwrap(), f, false);
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

    /// A run that exists in virtual time (#717): a nap is a millisecond of its clock, and the
    /// progress between two reads is the time passed at the rate of the code that runs.
    struct Sim {
        /// (progress, microseconds now, microseconds at the last read of the progress)
        s: std::cell::Cell<(f64, u64, u64)>,
        compiled: std::cell::Cell<bool>,
        rate_compiled: f64,
    }

    impl Sim {
        fn new(rate_compiled: f64) -> Sim {
            Sim {
                s: std::cell::Cell::new((0.0, 0, 0)),
                compiled: std::cell::Cell::new(true),
                rate_compiled,
            }
        }

        fn progress(&self) -> u64 {
            let (p, now, last) = self.s.get();
            let rate = if self.compiled.get() {
                self.rate_compiled
            } else {
                1.0e6
            };
            let p = p + rate * (now - last) as f64 / 1e6;
            self.s.set((p, now, now));
            p as u64
        }

        fn nap(&self) {
            let (p, now, last) = self.s.get();
            self.s.set((p, now + 1000, last));
        }

        fn wall(&self) -> std::time::Duration {
            std::time::Duration::from_micros(self.s.get().1)
        }

        /// `run_trial` over the run.
        fn trial(&self, rule: &Rule) -> Trial {
            let mut swap = |on: bool| {
                self.progress();
                self.compiled.set(on);
            };
            run_trial(
                &mut swap,
                &|| self.progress(),
                &|| None,
                &|| false,
                &Pace {
                    nap: &|| self.nap(),
                    wall: &|| self.wall(),
                },
                rule,
            )
        }
    }

    /// The windows of a trial swap the code in turn and measure the run's progress each time: with
    /// progress that depends on the code, the verdict follows; without progress there is none.
    #[test]
    fn a_trial_swaps_in_turn_and_measures_the_run() {
        use super::Verdict;
        for (rate_compiled, keep) in [(1.5e6, true), (0.7e6, false)] {
            let r = Sim::new(rate_compiled).trial(&BY_RUNS).verdict;
            assert_eq!(matches!(r, Some(Verdict::Keep(_))), keep, "{r:?}");
            assert!(r.is_some());
        }
        let sim = Sim::new(1.0e6);
        let mut swap = |_: bool| {};
        let pace = Pace {
            nap: &|| sim.nap(),
            wall: &|| sim.wall(),
        };
        let t = run_trial(&mut swap, &|| 0, &|| None, &|| true, &pace, &BY_RUNS);
        assert!(t.verdict.is_none());
    }

    /// A trial that counts the entries of a function takes all three pairs of windows, however
    /// clear the first one is: its windows vary with the burst they fall in.
    #[test]
    fn a_trial_of_entries_takes_three_pairs() {
        for (rule, pairs) in [(&BY_RUNS, 1), (&BY_ENTRIES, 3)] {
            let t = Sim::new(2.0e6).trial(rule);
            assert_eq!(t.compiled.len(), pairs);
            assert!(matches!(t.verdict, Some(super::Verdict::Keep(_))));
        }
    }

    /// The looks of the sampler get sparser as the run goes on, by the work seen (#675).
    #[test]
    fn looks_get_sparser_with_the_work_seen() {
        use super::{STRETCH_2, STRETCH_4, stretch};
        assert_eq!(stretch(0), 1);
        assert_eq!(stretch(STRETCH_2 - 1), 1);
        assert_eq!(stretch(STRETCH_2), 2);
        assert_eq!(stretch(STRETCH_4 - 1), 2);
        assert_eq!(stretch(STRETCH_4), 4);
        assert_eq!(stretch(u64::MAX), 4);
    }

    /// A clocked process stays interpreted unless it has many instances and is a good part of
    /// the run: the processes of a CPU design are a few percent of it each (#532).
    #[test]
    fn a_clocked_process_that_is_a_small_part_of_the_run_stays_interpreted() {
        // A process takes a thirty-second of the run.
        assert!(cold(10_000, 310_000, Tune::FIRST).is_none());
        assert!(cold(9_000, 310_000, Tune::FIRST).is_some());
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

    /// A candidate owes the seen time of its own payback and of its callees' (`Tune::tree_payback`).
    #[test]
    fn a_root_owes_the_payback_of_its_callees() {
        assert_eq!(super::repaid(20_000, 0, 50_000, 30_000), 0);
        assert_eq!(super::repaid(20_000, 0, 50_000, 5_000), 15_000);
        assert_eq!(super::repaid(20_000, 8, 50_000, 30_000), 390_000);
        assert_eq!(super::repaid(20_000, 8, 50_000, 420_000), 0);
        assert_eq!(super::repaid(20_000, 8, 0, 20_000), 0, "no callees");
    }

    /// `--tier-compile=background` (#1123): the thread that calls `lowest_priority` runs at the
    /// lowest priority, the others keep theirs.
    #[test]
    #[cfg(any(target_os = "linux", windows))]
    fn lowest_priority_lowers_the_calling_thread_only() {
        #[cfg(target_os = "linux")]
        fn own() -> i32 {
            unsafe extern "C" {
                fn getpriority(which: u32, who: u32) -> i32;
            }
            // SAFETY: the nice value of the calling thread.
            unsafe { getpriority(0, 0) }
        }
        #[cfg(windows)]
        fn own() -> i32 {
            #[link(name = "kernel32")]
            unsafe extern "system" {
                fn GetCurrentThread() -> *mut std::ffi::c_void;
                fn GetThreadPriority(thread: *mut std::ffi::c_void) -> i32;
            }
            // SAFETY: the pseudo handle of the calling thread.
            unsafe { GetThreadPriority(GetCurrentThread()) }
        }
        let before = std::thread::spawn(own).join().unwrap();
        let lowered = std::thread::spawn(|| {
            super::lowest_priority();
            own()
        })
        .join()
        .unwrap();
        assert_eq!(lowered, if cfg!(windows) { -2 } else { 19 });
        assert_eq!(own(), before);
    }
}
