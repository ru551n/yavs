//! Diagnostics for simulations that are stuck: the loop behind a delta cycle limit error, the
//! hang report (`yavs run --hang-report`, SIGUSR1) and the warning for a process that can never
//! suspend. The kernel finds the facts (`SimControl::stuck_report`, `ErrorKind::DeltaLimit`);
//! this module words them and adds the source places of the processes.

use std::collections::HashMap;

use yavs_diag::{Diagnostic, Span};
use yavs_ir::{FuncFlags, FuncId, Program};
use yavs_kernel::{
    LoopCause, ProcId, ProcWait, SignalId, SimError, StuckProc, StuckReport, StuckSignal, Time,
};

use crate::Elaborated;
use crate::hier::ScopeKind;

/// What a process is, by kernel process id: its IR function.
#[derive(Clone, Default)]
pub struct ProcTable {
    funcs: Vec<Option<FuncId>>,
}

/// Why a [`StuckReport`] was made.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StuckWhy {
    /// The run reached `--stop-time` without a `stop` or `finish`.
    StopTime(Time),
    /// Nothing is left to do, and no `stop` or `finish` ended the run.
    Idle,
    /// The user asked (SIGUSR1).
    Requested,
}

/// The first statement of a process body, the place diagnostics point at.
fn proc_span(prog: &Program, f: FuncId) -> Option<Span> {
    let f = prog.funcs.get(f)?.full();
    let at = f
        .body
        .and_then(|b| f.blocks.get(b))
        .map_or(0, |b| b.start as usize);
    let s = *f.loc.get(at).or(f.loc.first())?;
    prog.stmts.get(s).map(|i| i.span)
}

impl Elaborated {
    /// The IR function of each process, by kernel process id. Call it before
    /// [`Elaborated::release_for_run`], which frees what it reads.
    pub fn proc_table(&self) -> ProcTable {
        let Some(ir) = &self.ir else {
            return ProcTable::default();
        };
        let mut by_stmt: HashMap<_, HashMap<_, FuncId>> = HashMap::new();
        let funcs = self
            .procs
            .iter()
            .map(|p| {
                let i = self.instance_of(p.scope);
                let ScopeKind::Instance { arch, .. } = self.hier.scope(i).kind else {
                    return None;
                };
                let u = ir.units.get(&arch)?;
                match &p.actual {
                    Some(pa) => u.processes.iter().find_map(|x| {
                        x.actual
                            .as_ref()
                            .filter(|a| a.expr == pa.expr)
                            .map(|_| x.func)
                    }),
                    None => by_stmt
                        .entry(arch)
                        .or_insert_with(|| {
                            let mut m = HashMap::new();
                            for x in u.processes.iter().filter(|x| x.actual.is_none()) {
                                m.entry(x.stmt).or_insert(x.func);
                            }
                            m
                        })
                        .get(&p.stmt)
                        .copied(),
                }
            })
            .collect();
        ProcTable { funcs }
    }

    /// `W3102` for every process that can never suspend: no wait statement, no sensitivity
    /// list and no call of a procedure that waits. It runs forever inside its first activation,
    /// so simulation time never advances.
    pub fn no_wait_warnings(&self, t: &ProcTable) -> Vec<Diagnostic> {
        let Some(ir) = &self.ir else {
            return Vec::new();
        };
        let prog = &*ir.program;
        let mut out = Vec::new();
        for (k, f) in t.funcs.iter().enumerate() {
            let Some(f) = *f else { continue };
            let Some(func) = prog.funcs.get(f).map(|f| f.full()) else {
                continue;
            };
            if func.flags.contains(FuncFlags::MAY_SUSPEND) {
                continue;
            }
            let name = self.proc_name(&self.procs[k]);
            let mut d = Diagnostic::warning(
                "W3102",
                format!("process {name} has no wait statement and no sensitivity list"),
            );
            if let Some(span) = proc_span(prog, f) {
                d = d.with_primary(span, "this process never suspends");
            }
            out.push(
                d.with_note(
                    "a process runs from its first statement to its last and starts again; \
                     without a wait it never gives control back, so simulation time cannot \
                     advance (only `stop` or `finish` can end the run)",
                )
                .with_help(
                    "add a `wait for ...;` or `wait on ...;` at the end, or a sensitivity list; \
                     a process that should run once ends with `wait;`",
                ),
            );
        }
        out
    }

    /// [`Elaborated::runtime_diagnostic`] with the places of the processes a delta cycle loop
    /// names.
    pub fn runtime_diagnostic_in(&self, e: &SimError, procs: &ProcTable) -> Diagnostic {
        let mut d = self.runtime_diagnostic(e);
        let yavs_kernel::ErrorKind::DeltaLimit { cause, .. } = &e.kind else {
            return d;
        };
        let Some(ir) = &self.ir else { return d };
        let span = |p: ProcId| {
            procs
                .funcs
                .get(p.0 as usize)
                .copied()
                .flatten()
                .and_then(|f| proc_span(&ir.program, f))
        };
        let implicit = |p: ProcId| {
            procs
                .funcs
                .get(p.0 as usize)
                .copied()
                .flatten()
                .and_then(|f| ir.program.funcs.get(f))
                .is_some_and(|f| f.full().flags.contains(FuncFlags::IMPLICIT))
        };
        let (links, zero): (&[_], &[_]) = match cause {
            LoopCause::Cycle(l) => (l, &[]),
            LoopCause::ZeroWait(z) => (&[], z),
            LoopCause::Unknown => (&[], &[]),
        };
        for (k, l) in links.iter().enumerate() {
            let next = &links[(k + 1) % links.len()];
            let text = format!(
                "`{}` assigns {}, which wakes `{}`",
                l.name, l.signal, next.name
            );
            match span(l.process) {
                Some(s) if d.primary.is_none() => d = d.with_primary(s, text),
                Some(s) => d = d.with_secondary(s, text),
                None => {}
            }
        }
        for (p, name) in zero {
            if let Some(s) = span(*p) {
                let text = format!("`{name}` waits for 0 ns and runs again in the next delta");
                d = if d.primary.is_none() {
                    d.with_primary(s, text)
                } else {
                    d.with_secondary(s, text)
                };
            }
        }
        d = d.with_note(match cause {
            LoopCause::Cycle(l) => cycle_text(l, implicit),
            LoopCause::ZeroWait(z) => format!(
                "{} {} again in every delta: {} for 0 ns (`wait for 0 ns;`) in a loop",
                names(z.iter().map(|(_, n)| n.as_str()), "process", "processes"),
                if z.len() == 1 { "runs" } else { "run" },
                if z.len() == 1 {
                    "it waits"
                } else {
                    "they wait"
                }
            ),
            LoopCause::Unknown => "no loop of processes waking each other was found; look for \
                 a force, a deposit or a foreign callback that assigns in every delta"
                .to_string(),
        });
        let fix = if matches!(cause, LoopCause::ZeroWait(_)) {
            "wait for a real time or for a signal instead of `wait for 0 ns;` in a loop"
        } else {
            "break the loop: give an assignment on it a delay (`after 1 ns`), or let the \
             processes wait on a clock instead of on each other's outputs"
        };
        d.with_help(fix).with_help("if the design needs more delta cycles at one time, raise the limit with `--max-deltas=N`")
    }

    /// The hang report of a stuck, finished-early or interrupted run. At most `max` processes
    /// are listed in each part.
    pub fn stuck_diagnostic(
        &self,
        r: &StuckReport,
        why: StuckWhy,
        procs: &ProcTable,
        max: usize,
    ) -> Diagnostic {
        let prog = self.ir.as_ref().map(|ir| &*ir.program);
        let span = |p: ProcId| {
            let f = procs.funcs.get(p.0 as usize).copied().flatten()?;
            proc_span(prog?, f)
        };
        describe(r, why, max, &span)
    }
}

/// Words for the processes and signals of a report; `span` finds the source place of a process.
fn describe(
    r: &StuckReport,
    why: StuckWhy,
    max: usize,
    span: &dyn Fn(ProcId) -> Option<Span>,
) -> Diagnostic {
    let count = |s: ProcWait| r.procs.iter().filter(|p| p.state == s).count();
    let (timed, waiting, never, ended) = (
        count(ProcWait::Timed),
        count(ProcWait::Waiting),
        count(ProcWait::Never),
        count(ProcWait::Ended),
    );
    let at = format!("{}, delta {}", r.now, r.delta);
    let head = match why {
        StuckWhy::StopTime(t) => {
            format!("the simulation reached --stop-time {t} without `stop` or `finish` ending it")
        }
        StuckWhy::Idle => format!(
            "the simulation ran out of events at {} without `stop` or `finish` ending it",
            r.now
        ),
        StuckWhy::Requested => format!("simulation status at {}", r.now),
    };
    let mut d = Diagnostic::warning("W3101", head);
    let name = |p: ProcId| r.procs[p.0 as usize].name.as_str();
    let on = |p: &StuckProc| -> String {
        let names: Vec<String> = p.on.iter().take(4).map(|s| signal_name(r, *s)).collect();
        let more = p.on.len().saturating_sub(4);
        let mut t = names.join(", ");
        if more > 0 {
            t.push_str(&format!(" and {more} more"));
        }
        t
    };
    d = d.with_note(format!(
        "at {at}: {} processes, {timed} waiting for a timeout, {waiting} for signals that can \
         still change, {never} that can never run again, {ended} ended or at `wait;`",
        r.procs.len()
    ));
    // Processes that can never run again, with their places.
    let stuck: Vec<&StuckProc> = r
        .procs
        .iter()
        .filter(|p| p.state == ProcWait::Never)
        .collect();
    let mut labelled = 0;
    for p in stuck.iter().take(max) {
        let text = format!("waits for an event on {}, which nothing can cause", on(p));
        d = d.with_note(format!("process {} {text}", p.name));
        if let Some(s) = span(p.id).filter(|_| labelled < 8) {
            let l = format!("{} waits for {}", p.name, on(p));
            d = if d.primary.is_none() {
                d.with_primary(s, l)
            } else {
                d.with_secondary(s, l)
            };
            labelled += 1;
        }
    }
    if stuck.len() > max {
        d = d.with_note(format!("and {} more such processes", stuck.len() - max));
    }
    // A stuck clock is described by its own note below.
    let plain: Vec<&StuckSignal> = r
        .stuck
        .iter()
        .filter(|s| !r.clocks.iter().any(|c| c.signal == s.id))
        .collect();
    for s in plain.iter().take(max) {
        d = d.with_note(stuck_signal(r, s));
    }
    if plain.len() > max {
        d = d.with_note(format!("and {} more such signals", plain.len() - max));
    }
    for c in &r.clocks {
        let w: Vec<&str> = c.waiters.iter().take(3).map(|&p| name(p)).collect();
        let mut text = format!(
            "signal {} behaved like a clock but stopped toggling after {}, and {} still {} for its edge ({}{})",
            c.name,
            c.last_event,
            plural(c.waiters.len(), "process", "processes"),
            if c.waiters.len() == 1 {
                "waits"
            } else {
                "wait"
            },
            w.join(", "),
            if c.waiters.len() > 3 { ", ..." } else { "" },
        );
        text.push_str(&match c.drivers.as_slice() {
            [] => "; nothing drives it".to_string(),
            [p] => format!("; its driver is {}", driver_text(r, *p)),
            ps => format!(
                "; its drivers are {}",
                ps.iter()
                    .take(3)
                    .map(|&p| driver_text(r, p))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
        });
        d = d.with_note(text);
    }
    // Processes that wait for a timeout.
    let timed_procs: Vec<&StuckProc> = r
        .procs
        .iter()
        .filter(|p| p.state == ProcWait::Timed)
        .collect();
    for p in timed_procs.iter().take(max.min(5)) {
        if let Some(t) = p.timeout {
            let mut text = format!("process {} runs again at {t}", p.name);
            if !p.on.is_empty() {
                text.push_str(&format!(" or on an event on {}", on(p)));
            }
            d = d.with_note(text);
        }
    }
    if timed_procs.len() > max.min(5) {
        d = d.with_note(format!(
            "and {} more processes waiting for a timeout",
            timed_procs.len() - max.min(5)
        ));
    }
    if never > 0 || !r.clocks.is_empty() {
        d = d.with_help(
            "find out why the signal never changes: is it driven, is its driver reached, did the \
             process that should drive it end? `--hang-report` prints this at the end of a run",
        );
    }
    d
}

fn signal_name(r: &StuckReport, s: SignalId) -> String {
    r.names
        .get(s.0 as usize)
        .cloned()
        .unwrap_or_else(|| format!("signal {}", s.0))
}

fn plural(n: usize, one: &str, many: &str) -> String {
    format!("{n} {}", if n == 1 { one } else { many })
}

/// A driver process with its own state.
fn driver_text(r: &StuckReport, p: ProcId) -> String {
    let q = &r.procs[p.0 as usize];
    match q.state {
        ProcWait::Never => format!("{} (can never run again)", q.name),
        ProcWait::Ended => format!("{} (ended or at `wait;`)", q.name),
        _ => q.name.clone(),
    }
}

fn stuck_signal(r: &StuckReport, s: &StuckSignal) -> String {
    let mut t = format!("signal {}", s.name);
    if let Some(v) = &s.value {
        t.push_str(&format!(" is {v}"));
    }
    t.push_str(match s.last_event {
        Some(_) => "",
        None => " and never had an event",
    });
    if let Some(e) = s.last_event {
        t.push_str(&format!(", last event at {e}"));
    }
    match (s.drivers.as_slice(), s.fed_by.as_slice()) {
        ([], []) => t.push_str("; it has no driver"),
        (ps, fed) => {
            if !ps.is_empty() {
                t.push_str(&format!(
                    "; driven by {}",
                    ps.iter()
                        .take(4)
                        .map(|&p| driver_text(r, p))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
            if !fed.is_empty() {
                t.push_str(&format!(
                    "; connected through a port to {}",
                    fed.iter()
                        .take(4)
                        .map(|&f| signal_name(r, f))
                        .collect::<Vec<_>>()
                        .join(", ")
                ));
            }
        }
    }
    t
}

/// `process A`, `processes A and B`, `processes A, B and C`.
fn names<'a>(n: impl Iterator<Item = &'a str>, one: &str, many: &str) -> String {
    let v: Vec<String> = n.map(|x| format!("`{x}`")).collect();
    match v.as_slice() {
        [a] => format!("{one} {a}"),
        [a, b] => format!("{many} {a} and {b}"),
        [init @ .., last] => format!("{many} {} and {last}", init.join(", ")),
        [] => many.to_string(),
    }
}

fn cycle_text(l: &[yavs_kernel::LoopLink], implicit: impl Fn(ProcId) -> bool) -> String {
    let sigs = l.iter().map(|x| x.signal.as_str()).collect::<Vec<_>>();
    let sig_list = match sigs.as_slice() {
        [a] => a.to_string(),
        [a, b] => format!("{a} and {b}"),
        [init @ .., last] => format!("{} and {last}", init.join(", ")),
        [] => String::new(),
    };
    let who = names(l.iter().map(|x| x.name.as_str()), "process", "processes");
    let all_implicit = l.iter().all(|x| implicit(x.process));
    match l {
        [one] if implicit(one.process) => format!(
            "the concurrent statement `{}` is sensitive to {} and assigns it with no delay: a \
             combinational loop",
            one.name, one.signal
        ),
        [one] => format!(
            "process `{}` waits on {}, which it assigns itself with no delay, so each \
             assignment wakes it again",
            one.name, one.signal
        ),
        _ if all_implicit => format!(
            "{who} are concurrent statements that wake each other through {sig_list} with no \
             delay: a combinational loop"
        ),
        _ => format!("{who} wake each other through {sig_list} with no delay"),
    }
}
