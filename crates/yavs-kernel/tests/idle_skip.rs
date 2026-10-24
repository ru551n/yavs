//! Idle skip (`Builder::idle_skip`): a clocked process that depends on nothing but signals is left
//! suspended at an edge when none of them had an event since its last activation. The run is the
//! same with the skip, without it, and with it checked (`Config::idle_check`); the skip happens.

mod common;

use common::*;
use yavs_kernel::*;
use yavs_logic::StdULogic;

const NONE: ProcFlags = ProcFlags {
    postponed: false,
    serial: false,
    several_waits: false,
    concurrent_call: false,
};

#[derive(Clone, Copy, PartialEq)]
enum Skip {
    Off,
    On,
    Check,
}

fn logic(b: &mut Builder, name: &str, v: StdULogic) -> SignalId {
    b.signal(name, SignalType::logic(1), &LogicVec::new(1, v))
}

/// Nine elements, element 0 in use: a signal with a waiter list per element (`wake_sole`).
fn wide(b: &mut Builder, name: &str, v: StdULogic) -> SignalId {
    b.signal(name, SignalType::logic(9), &LogicVec::new(9, v))
}

/// A clock, an input `d` that changes every `every` edges, an enable `en` that is high from edge 10
/// to edge 30, and two registers: `q <= d` and `qe <= d` when `en = '1'`. Returns the value changes
/// and phases of a run to 400 ns, with `ops` applied at the given times, and the statistics.
fn run(skip: Skip, every: u64, ops: &[(u64, &str)], values: bool) -> (Vec<String>, Stats) {
    run_in(skip, every, ops, values, 1, 0, false)
}

/// `run` with `threads` and `extra` more registers `q <= d` (a batch of 32 or more processes
/// runs in the pool when there are threads); with `noise`, element 1 of `d`, which no register
/// reads, toggles at every edge.
fn run_in(
    skip: Skip,
    every: u64,
    ops: &[(u64, &str)],
    values: bool,
    threads: usize,
    extra: usize,
    noise: bool,
) -> (Vec<String>, Stats) {
    run_g(skip, every, ops, values, threads, extra, noise, 0)
}

/// `run_in` where `reg_en` declares its read of `d` conditional on `en = '1'` (`guard` 1: one
/// literal, 2: two alternatives), as `Builder::idle_skip_guarded` takes it.
#[allow(clippy::too_many_arguments)]
fn run_g(
    skip: Skip,
    every: u64,
    ops: &[(u64, &str)],
    values: bool,
    threads: usize,
    extra: usize,
    noise: bool,
    guard: u8,
) -> (Vec<String>, Stats) {
    let mut b = Builder::new();
    let clk = logic(&mut b, "clk", StdULogic::Zero);
    let dclk = b.driver(clk);
    b.process_boxed(
        "clkgen",
        NONE,
        &[dclk],
        Box::new(ToggleProcess::new(
            clk,
            0,
            dclk,
            Time::ns(5),
            DelayMode::INERTIAL,
        )),
    );
    let (d, en) = (
        wide(&mut b, "d", StdULogic::Zero),
        wide(&mut b, "en", StdULogic::Zero),
    );
    let (q, qe) = (
        logic(&mut b, "q", StdULogic::Zero),
        logic(&mut b, "qe", StdULogic::Zero),
    );
    let (dd, den, dq, dqe) = (b.driver(d), b.driver(en), b.driver(q), b.driver(qe));
    // The testbench: on every rising edge it counts, and sets `d` and `en`.
    let zero1 = Range::new(0, 1);
    let mut n = 0u64;
    b.process(
        "stim",
        NONE,
        &[dd, den],
        p(move |c| {
            if c.event(clk)? && c.elem_code(clk, 0)? == StdULogic::One as u8 {
                n += 1;
                if n.is_multiple_of(every) {
                    let v = StdULogic::from_bool(c.elem_code(d, 0)? != StdULogic::One as u8);
                    c.assign_code(dd, Some(zero1), Time::ZERO, v as u8, DelayMode::INERTIAL)?;
                }
                if noise {
                    let v = StdULogic::from_bool(n % 2 == 1);
                    let one = Some(Range::new(1, 1));
                    c.assign_code(dd, one, Time::ZERO, v as u8, DelayMode::INERTIAL)?;
                }
                if n == 10 || n == 30 {
                    let v = StdULogic::from_bool(n == 10);
                    c.assign_code(den, Some(zero1), Time::ZERO, v as u8, DelayMode::INERTIAL)?;
                }
            }
            c.wait_on(clk)
        }),
    );
    let one = Range::new(0, 1);
    // A process that only waits for `d` and `en`: their changes wake it at once
    // (`wake_sole`), which must arm the readers too.
    for sig in [d, en] {
        b.process(
            "watch",
            NONE,
            &[],
            p(move |c| {
                if !c.wait_same() {
                    c.wait_on(sig.slice(0, 1))?;
                }
                Ok(())
            }),
        );
    }
    let reg = |gate: bool| {
        p(move |c| {
            if gate {
                if c.elem_code(en, 0)? == StdULogic::One as u8 {
                    let v = c.elem_code(d, 0)?;
                    c.assign_code_now(dqe, one, v);
                }
            } else {
                let v = c.elem_code(d, 0)?;
                c.assign_code_now(dq, one, v);
            }
            if !c.wait_on_again(clk, one) {
                c.wait_on(clk)?;
            }
            Ok(())
        })
    };
    let edge = EdgeTrigger {
        signal: clk,
        elem: 0,
        rising: true,
        legacy: false,
        reset: None,
    };
    let p1 = b.process("reg", NONE, &[dq], reg(false));
    let p2 = b.process("reg_en", NONE, &[dqe], reg(true));
    let mut regs = vec![(p1, vec![(d, one)]), (p2, vec![(en, one), (d, one)])];
    for k in 0..extra {
        let qk = logic(&mut b, &format!("q{k}"), StdULogic::Zero);
        let dk = b.driver(qk);
        let pk = b.process(
            &format!("reg{k}"),
            NONE,
            &[dk],
            p(move |c| {
                let v = c.elem_code(d, 0)?;
                c.assign_code_now(dk, one, v);
                if !c.wait_on_again(clk, one) {
                    c.wait_on(clk)?;
                }
                Ok(())
            }),
        );
        regs.push((pk, vec![(d, one)]));
    }
    for (pid, reads) in regs {
        b.edge_trigger(pid, edge);
        b.idle_skip(pid, &reads);
    }
    if guard == 9 {
        // A wrong declaration: `reg` reads `d` but says it reads only `en`.
        b.idle_skip(p1, &[(en, one)]);
    } else if guard != 0 {
        let lit = |code: StdULogic| IdleLit {
            signal: en,
            elem: 0,
            code: code as u8,
            eq: true,
        };
        let alts = match guard {
            1 => vec![vec![lit(StdULogic::One)]],
            _ => vec![vec![lit(StdULogic::One)], vec![lit(StdULogic::H)]],
        };
        let read = |signal, guard| IdleRead {
            signal,
            range: one,
            guard,
        };
        b.idle_skip_guarded(p2, vec![read(en, None), read(d, Some(alts))]);
    }
    let mut sim = b
        .build(Config {
            skip_unchanged: true,
            idle_skip: skip != Skip::Off,
            idle_check: skip == Skip::Check,
            threads,
            ..Config::default()
        })
        .unwrap();
    let t = Trace {
        phases: true,
        ..Trace::default()
    };
    if values {
        sim.subscribe(Box::new(t.clone()));
    } else {
        sim.subscribe(Box::new(Cycles {
            lines: t.clone(),
            q,
            qe,
        }));
    }
    for &(time, op) in ops {
        sim.run_until(ns(time)).unwrap();
        match op {
            // The value the register's process drives is replaced from outside.
            "deposit_q" | "deposit_qe" => {
                let s = if op == "deposit_q" { q } else { qe };
                let now = sim.view(s).unwrap().code(0);
                let flipped = StdULogic::from_bool(now != StdULogic::One as u8);
                sim.deposit(s, &LogicVec::new(1, flipped)).unwrap();
            }
            _ => unreachable!(),
        }
    }
    sim.run_until(ns(400)).unwrap();
    (t.take(), sim.stats())
}

/// Records the registers at every cycle and wants no value changes: a change of `d` or `en`
/// wakes the watchers at once (`wake_sole`).
struct Cycles {
    lines: Trace,
    q: SignalId,
    qe: SignalId,
}

impl Observer for Cycles {
    fn wants_value_changes(&self) -> bool {
        false
    }
    fn wants_phase(&self, p: Phase) -> bool {
        p == Phase::NextCycle
    }
    fn phase(&mut self, h: &mut Hook<'_>, _p: Phase) {
        let (q, qe) = (
            h.view(self.q).unwrap().code(0),
            h.view(self.qe).unwrap().code(0),
        );
        self.lines
            .lines
            .lock()
            .unwrap()
            .push(format!("{} +{} q={q} qe={qe}", h.now(), h.delta()));
    }
}

fn same(every: u64, ops: &[(u64, &str)]) {
    for values in [true, false] {
        let (off, st_off) = run(Skip::Off, every, ops, values);
        let (on, st_on) = run(Skip::On, every, ops, values);
        let (check, st_check) = run(Skip::Check, every, ops, values);
        assert_eq!(off, on);
        assert_eq!(off, check);
        assert_eq!(st_off.idle_skips, 0);
        assert!(st_on.idle_skips > 0, "the skip never fired");
        assert_eq!(st_on.idle_skips, st_check.idle_skips);
    }
}

#[test]
fn registers_with_quiet_inputs_are_skipped() {
    same(8, &[]);
    same(3, &[]);
    same(1000, &[]);
}

#[test]
fn a_deposit_on_an_output_is_restored_at_the_next_edge() {
    same(
        1000,
        &[(103, "deposit_q"), (217, "deposit_qe"), (251, "deposit_q")],
    );
    same(
        8,
        &[(103, "deposit_q"), (217, "deposit_qe"), (251, "deposit_q")],
    );
}

/// A batch large enough for the pool: the activations are recorded as in the serial run, so the
/// run and the skips are the same at any thread count.
#[test]
fn skips_in_the_pool_are_those_of_the_serial_run() {
    for every in [3, 8, 1000] {
        let (serial, st1) = run_in(Skip::On, every, &[], true, 1, 40, false);
        let (pooled, st4) = run_in(Skip::On, every, &[], true, 4, 40, false);
        let (check, stc) = run_in(Skip::Check, every, &[], true, 4, 40, false);
        let (off, _) = run_in(Skip::Off, every, &[], true, 4, 40, false);
        assert_eq!(serial, pooled);
        assert_eq!(serial, check);
        assert_eq!(serial, off);
        assert!(st1.idle_skips > 100, "the skip hardly fired");
        assert_eq!(st1.idle_skips, st4.idle_skips);
        assert_eq!(st1.idle_skips, stc.idle_skips);
        assert_eq!(st1.process_runs, st4.process_runs);
    }
}

/// Events on elements of an input that a register does not read leave it skipped (#686: the
/// fields of a record share a kernel signal).
#[test]
fn events_on_other_elements_do_not_arm() {
    for every in [8, 1000] {
        let (_, quiet) = run_in(Skip::On, every, &[], true, 1, 0, false);
        let (on, st) = run_in(Skip::On, every, &[], true, 1, 0, true);
        let (check, stc) = run_in(Skip::Check, every, &[], true, 1, 0, true);
        let (off, _) = run_in(Skip::Off, every, &[], true, 1, 0, true);
        assert_eq!(on, off);
        assert_eq!(on, check);
        assert_eq!(st.idle_skips, quiet.idle_skips);
        assert_eq!(st.idle_skips, stc.idle_skips);
    }
}

/// A read under a condition (`if en = '1' then q <= d`) arms its process only while the condition
/// holds: the register is skipped for the changes of `d` that happen while `en` is low, and the
/// run is that of the unconditional reads and of no skipping (`Check` asserts every skip).
#[test]
fn guarded_reads_arm_only_while_the_condition_holds() {
    for guard in [1, 2] {
        for every in [3, 8] {
            let (_, plain) = run_in(Skip::On, every, &[], true, 1, 0, false);
            let (off, _) = run_g(Skip::Off, every, &[], true, 1, 0, false, guard);
            let (on, st) = run_g(Skip::On, every, &[], true, 1, 0, false, guard);
            let (check, stc) = run_g(Skip::Check, every, &[], true, 1, 0, false, guard);
            assert_eq!(on, off);
            assert_eq!(on, check);
            assert_eq!(st.idle_skips, stc.idle_skips);
            assert!(
                st.idle_skips > plain.idle_skips,
                "guard {guard}, every {every}: {} skips against {} without a condition",
                st.idle_skips,
                plain.idle_skips
            );
        }
    }
}

/// A read list that leaves out a signal the process reads is a bug in the caller; `idle_check`
/// stops the run at the skip that is wrong and names the process (the panic of the check was
/// attributed to the next process of the batch, or to none).
#[test]
fn a_wrong_read_list_is_caught_by_the_check_and_names_the_process() {
    let r = std::panic::catch_unwind(|| run_g(Skip::Check, 3, &[], true, 1, 0, false, 9));
    let msg = match r {
        Ok(_) => panic!("the check missed a skip that is wrong"),
        Err(p) => p
            .downcast_ref::<String>()
            .cloned()
            .or_else(|| p.downcast_ref::<&str>().map(|s| s.to_string()))
            .unwrap_or_default(),
    };
    assert!(
        msg.contains("idle skip: process reg would not have done what its last activation did"),
        "{msg}"
    );
    // The error is that of `reg`, not of `reg_en` after it in the batch.
    assert!(msg.contains("\"reg\")"), "{msg}");
}
