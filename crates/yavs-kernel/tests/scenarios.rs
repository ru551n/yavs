//! LRM 14.7 scenarios with hand-written processes.

mod common;

use std::sync::Arc;

use common::*;
use yavs_kernel::*;
use yavs_logic::StdULogic;

const NONE: ProcFlags = ProcFlags {
    postponed: false,
    serial: false,
    several_waits: false,
    concurrent_call: false,
};

fn u32_of(v: &[u8]) -> u32 {
    u32::from_le_bytes(v.try_into().unwrap())
}

/// Clock with a 10 ns period, rising at 5, 15, 25, ...
fn clock(b: &mut Builder) -> SignalId {
    let clk = b.signal("clk", BYTE, &[0]);
    let d = b.driver(clk);
    b.process(
        "clkgen",
        NONE,
        &[d],
        p(move |c| {
            let v = c.value(clk)?[0] ^ 1;
            c.assign(d, None, &[(ns(5), &[v])], DelayMode::INERTIAL)?;
            c.wait_for(ns(5))
        }),
    );
    clk
}

#[test]
fn clock_and_counter() {
    let mut b = Builder::new();
    let clk = clock(&mut b);
    let cnt = b.signal("cnt", SignalType::bytes(4, 1), &[0; 4]);
    let d = b.driver(cnt);
    b.process(
        "counter",
        NONE,
        &[d],
        p(move |c| {
            if c.event(clk)? && c.value(clk)? == [1] {
                assert_eq!(c.last_value(clk)?, Value::Bytes(vec![0]));
                assert_eq!(c.last_event(clk)?, Time::ZERO);
                c.drive(d, &(u32_of(c.value(cnt)?) + 1).to_le_bytes())?;
            }
            c.wait_on(clk)?;
            Ok(())
        }),
    );
    let (mut sim, trace) = build(b, Config::default());
    assert_eq!(sim.run_until(ns(100)).unwrap(), RunStatus::Paused);
    assert_eq!(sim.now(), ns(100));
    assert_eq!(u32_of(&v(&sim, cnt)), 10);
    let lines = trace.take();
    assert_eq!(
        lines[..3],
        [
            "5 ns +0 clk[0..1]=[1]",
            "5 ns +1 cnt[0..1]=[1, 0, 0, 0]",
            "10 ns +0 clk[0..1]=[0]"
        ]
    );
    assert_eq!(sim.run_for(ns(10)).unwrap(), RunStatus::Paused);
    assert_eq!(u32_of(&v(&sim, cnt)), 11);
    assert_eq!(sim.signal_id("cnt"), Some(cnt));
}

/// `s <= 1 after 10 ns` at 0 ns, then `s <= 0 after 10 ns` with `mode` at 2 ns.
fn pulse(mode: DelayMode) -> Vec<String> {
    let mut b = Builder::new();
    let s = b.signal("s", BYTE, &[0]);
    let d = b.driver(s);
    let mut step = 0;
    b.process(
        "stim",
        NONE,
        &[d],
        p(move |c| {
            step += 1;
            match step {
                1 => {
                    c.assign(d, None, &[(ns(10), &[1])], DelayMode::INERTIAL)?;
                    c.wait_for(ns(2))
                }
                2 => c.assign(d, None, &[(ns(10), &[0])], mode),
                _ => Ok(()),
            }
        }),
    );
    let (mut sim, trace) = build(b, Config::default());
    assert_eq!(
        sim.run_until(ns(100)).unwrap(),
        RunStatus::Ended(EndReason::Idle)
    );
    trace.take()
}

#[test]
fn inertial_pulse_rejection() {
    let both = ["10 ns +0 s[0..1]=[1]", "12 ns +0 s[0..1]=[0]"];
    assert!(pulse(DelayMode::INERTIAL).is_empty());
    assert!(
        pulse(DelayMode::Inertial {
            reject: Some(ns(5))
        })
        .is_empty()
    );
    assert_eq!(
        pulse(DelayMode::Inertial {
            reject: Some(ns(1))
        }),
        both
    );
}

#[test]
fn transport_delay_keeps_pulse() {
    assert_eq!(
        pulse(DelayMode::Transport),
        ["10 ns +0 s[0..1]=[1]", "12 ns +0 s[0..1]=[0]"]
    );
}

#[test]
fn reject_limit_above_delay_is_an_error() {
    let mut b = Builder::new();
    let s = b.signal("s", BYTE, &[0]);
    let d = b.driver(s);
    b.process(
        "bad",
        NONE,
        &[d],
        p(move |c| {
            c.assign(
                d,
                None,
                &[(ns(1), &[1])],
                DelayMode::Inertial {
                    reject: Some(ns(2)),
                },
            )
        }),
    );
    let (mut sim, _) = build(b, Config::default());
    let err = sim.run_until(ns(10)).unwrap_err();
    assert_eq!(
        err.kind,
        ErrorKind::InvalidAssign("pulse rejection limit exceeds the delay".into())
    );
    assert_eq!(err.process, Some((ProcId(0), "bad".into())));
}

#[test]
fn resolution_with_several_drivers() {
    let or = Resolution::Func(Arc::new(|srcs: &[&[u8]], out: &mut [u8]| {
        assert_eq!(srcs.len(), 2);
        for (i, o) in out.iter_mut().enumerate() {
            *o = srcs.iter().fold(0, |a, s| a | s[i]);
        }
        Ok(())
    }));
    let mut b = Builder::new();
    let bus = b.resolved_signal("bus", SignalType::bytes(1, 4), &[0; 4], or);
    let da = b.driver(bus);
    let db = b.driver(bus.slice(0, 2));
    let dc = b.driver(bus.slice(2, 2));
    b.process("a", NONE, &[da], p(move |c| c.drive(da, &[1, 0, 0, 0])));
    let mut step = 0;
    b.process(
        "b",
        NONE,
        &[db],
        p(move |c| {
            step += 1;
            if step == 1 {
                c.drive(db, &[2, 2])?;
                c.wait_for(ns(5))?;
            } else {
                c.assign(
                    db,
                    Some(Range::new(1, 1)),
                    &[(Time::ZERO, &[16])],
                    DelayMode::INERTIAL,
                )?;
            }
            Ok(())
        }),
    );
    b.process("c", NONE, &[dc], p(move |c| c.drive(dc, &[4, 8])));
    let (mut sim, trace) = build(b, Config::default());
    sim.run_until(ns(1)).unwrap();
    assert_eq!(v(&sim, bus), [3, 2, 4, 8]);
    sim.run_until(ns(10)).unwrap();
    assert_eq!(v(&sim, bus), [3, 16, 4, 8]);
    assert_eq!(
        trace.take(),
        ["0 fs +1 bus[0..4]=[3, 2, 4, 8]", "5 ns +1 bus[1..2]=[16]"]
    );
}

#[test]
fn unresolved_signal_rejects_overlapping_drivers() {
    let mut b = Builder::new();
    let s = b.signal("s", SignalType::bytes(1, 4), &[0; 4]);
    let d1 = b.driver(s.slice(0, 2));
    let d2 = b.driver(s.slice(2, 2));
    b.process("p1", NONE, &[d1], p(|_| Ok(())));
    b.process("p2", NONE, &[d2], p(|_| Ok(())));
    let d3 = b.driver(s.slice(1, 1));
    b.process("p3", NONE, &[d3], p(|_| Ok(())));
    let err = b.build(Config::default()).err().unwrap();
    assert!(matches!(err.kind, ErrorKind::Usage(_)), "{err}");
}

/// `wait on s until s = 1 for 20 ns` against a stimulus waveform.
fn wait_until(stimulus: &'static [(u64, u8)]) -> Vec<String> {
    let mut b = Builder::new();
    let s = b.signal("s", BYTE, &[0]);
    let d = b.driver(s);
    b.process(
        "stim",
        NONE,
        &[d],
        p(move |c| {
            let wf: Vec<(Time, [u8; 1])> = stimulus.iter().map(|&(t, v)| (ns(t), [v])).collect();
            let wf: Vec<(Time, &[u8])> = wf.iter().map(|(t, v)| (*t, &v[..])).collect();
            c.assign(d, None, &wf, DelayMode::Transport)
        }),
    );
    let mut deadline = None;
    b.process(
        "waiter",
        NONE,
        &[],
        p(move |c| {
            match deadline {
                None => deadline = Some(c.after(ns(20))?),
                Some(_) if c.value(s)? == [1] || c.timed_out() => {
                    c.report(
                        Severity::Note,
                        format!("resumed timed_out={}", c.timed_out()),
                    );
                    return Ok(());
                }
                Some(_) => {}
            }
            c.wait_on(s)?;
            c.wait_at(deadline.unwrap())?;
            Ok(())
        }),
    );
    let (mut sim, trace) = build(b, Config::default());
    sim.run_until(ns(100)).unwrap();
    trace
        .take()
        .into_iter()
        .filter(|l| l.contains("resumed"))
        .collect()
}

#[test]
fn wait_until_with_timeout() {
    assert_eq!(
        wait_until(&[(5, 2), (30, 1)]),
        ["20 ns +0 p1 Note: resumed timed_out=true"]
    );
    assert_eq!(
        wait_until(&[(5, 2), (10, 1)]),
        ["10 ns +0 p1 Note: resumed timed_out=false"]
    );
}

#[test]
fn postponed_runs_once_at_end_of_time_step() {
    let mut b = Builder::new();
    let s = b.signal("s", BYTE, &[0]);
    let t = b.signal("t", BYTE, &[0]);
    let ds = b.driver(s);
    let dt = b.driver(t);
    let post = ProcFlags {
        postponed: true,
        serial: false,
        several_waits: false,
        concurrent_call: false,
    };
    b.process(
        "watch",
        post,
        &[],
        p(move |c| {
            c.report(
                Severity::Note,
                format!("s={:?} t={:?}", c.value(s)?, c.value(t)?),
            );
            c.wait_on(s)?;
            c.wait_on(t)?;
            Ok(())
        }),
    );
    b.process("a", NONE, &[ds], p(move |c| c.drive(ds, &[1])));
    b.process(
        "copy",
        NONE,
        &[dt],
        p(move |c| {
            let v = c.value(s)?.to_vec();
            c.drive(dt, &v)?;
            c.report(Severity::Note, "copy");
            c.wait_on(s)?;
            Ok(())
        }),
    );
    let (mut sim, trace) = build(b, Config::default());
    sim.run_until(ns(1)).unwrap();
    assert_eq!(
        trace.take(),
        [
            "0 fs +0 p0 Note: s=[0] t=[0]",
            "0 fs +0 p2 Note: copy",
            "0 fs +1 s[0..1]=[1]",
            "0 fs +1 p2 Note: copy",
            "0 fs +2 t[0..1]=[1]",
            "0 fs +2 p0 Note: s=[1] t=[1]",
        ]
    );
}

#[test]
fn postponed_zero_delay_is_an_error() {
    let mut b = Builder::new();
    let s = b.signal("s", BYTE, &[0]);
    let d = b.driver(s);
    let t = b.signal("t", BYTE, &[0]);
    let dt = b.driver(t);
    b.process(
        "kick",
        NONE,
        &[dt],
        p(move |c| c.assign(dt, None, &[(ns(1), &[1])], DelayMode::INERTIAL)),
    );
    b.process(
        "post",
        ProcFlags {
            postponed: true,
            serial: false,
            several_waits: false,
            concurrent_call: false,
        },
        &[d],
        p(move |c| {
            if c.event(t)? {
                c.drive(d, &[1])?;
            }
            c.wait_on(t)?;
            Ok(())
        }),
    );
    let (mut sim, _) = build(b, Config::default());
    let err = sim.run_until(ns(5)).unwrap_err();
    assert_eq!(
        (err.kind.clone(), err.time, err.process.clone()),
        (
            ErrorKind::PostponedDelta,
            ns(1),
            Some((ProcId(1), "post".into()))
        )
    );
    assert_eq!(
        err.to_string(),
        "1 ns +0 in post: postponed process caused a delta cycle"
    );
}

#[test]
fn force_release_and_deposit() {
    let mut b = Builder::new();
    let s = b.signal("s", BYTE, &[0]);
    let u = b.signal("u", BYTE, &[0]);
    let d = b.driver(s);
    b.process(
        "drv",
        NONE,
        &[d],
        p(move |c| {
            c.assign(
                d,
                None,
                &[(Time::ZERO, &[1]), (ns(50), &[2])],
                DelayMode::Transport,
            )
        }),
    );
    let mut step = 0;
    b.process(
        "ctl",
        NONE,
        &[],
        p(move |c| {
            step += 1;
            match step {
                2 => c.force(s, ForceMode::Effective, &[5])?,
                3 => c.release(s, ForceMode::Effective)?,
                4 => c.force(s, ForceMode::Driving, &[7])?,
                5 => c.release(s, ForceMode::Driving)?,
                _ => {}
            }
            if step < 5 { c.wait_for(ns(10)) } else { Ok(()) }
        }),
    );
    let (mut sim, trace) = build(b, Config::default());
    let mut at = |t, val: u8, drv: u8| {
        sim.run_until(ns(t)).unwrap();
        assert_eq!(
            (v(&sim, s), sim.driving_value(s).unwrap()),
            (vec![val], Value::Bytes(vec![drv])),
            "at {t} ns"
        );
    };
    at(5, 1, 1);
    at(15, 5, 1);
    at(25, 1, 1);
    at(35, 7, 7);
    at(45, 1, 1);
    sim.deposit(s, &[3]).unwrap();
    sim.deposit(u, &[9]).unwrap();
    sim.run_until(ns(46)).unwrap();
    assert_eq!((v(&sim, s), v(&sim, u)), (vec![3], vec![9]));
    // A release after a deposit returns to the sources' value (#310); without sources the
    // deposit stays.
    sim.force(s, ForceMode::Effective, &[8]).unwrap();
    sim.force(u, ForceMode::Effective, &[8]).unwrap();
    sim.run_until(ns(47)).unwrap();
    assert_eq!((v(&sim, s), v(&sim, u)), (vec![8], vec![8]));
    sim.release(s, ForceMode::Effective).unwrap();
    sim.release(u, ForceMode::Effective).unwrap();
    sim.run_until(ns(48)).unwrap();
    assert_eq!((v(&sim, s), v(&sim, u)), (vec![1], vec![9]));
    sim.run_until(ns(60)).unwrap();
    assert_eq!((v(&sim, s), v(&sim, u)), (vec![2], vec![9]));
    assert!(trace.take().contains(&"10 ns +1 s[0..1]=[5]".to_string()));
    assert!(
        sim.force(s.slice(1, 1), ForceMode::Effective, &[0])
            .is_err()
    );
}

#[test]
fn zero_delay_oscillation_hits_delta_limit() {
    let mut b = Builder::new();
    let s = b.signal("s", BYTE, &[0]);
    let d = b.driver(s);
    b.process(
        "osc",
        NONE,
        &[d],
        p(move |c| {
            c.drive(d, &[c.value(s)?[0] ^ 1])?;
            c.wait_on(s)?;
            Ok(())
        }),
    );
    let cfg = Config {
        max_deltas: 100,
        ..Config::default()
    };
    let (mut sim, _) = build(b, cfg);
    let err = sim.run_until(ns(1)).unwrap_err();
    assert_eq!(
        err.kind,
        ErrorKind::DeltaLimit {
            limit: 100,
            processes: vec!["osc".into()],
            cause: yavs_kernel::LoopCause::Cycle(vec![yavs_kernel::LoopLink {
                process: ProcId(0),
                name: "osc".into(),
                signal: "s".into(),
            }]),
        }
    );
    assert_eq!(
        err.to_string(),
        "0 fs +100: delta cycle limit 100 exceeded; active processes: osc"
    );
    assert_eq!(err.exit_status(), 1);
    assert_eq!(sim.run_until(ns(1)).unwrap_err(), err);
}

/// Two processes that wake each other: the loop names both and the signals between them, and
/// the stuck report of the end state is the same at any time.
#[test]
fn delta_limit_names_the_loop() {
    let mut b = Builder::new();
    let (x, y) = (b.signal("x", BYTE, &[0]), b.signal("y", BYTE, &[0]));
    let (dx, dy) = (b.driver(x), b.driver(y));
    b.process(
        "p",
        NONE,
        &[dx],
        p(move |c| {
            c.drive(dx, &[c.value(y)?[0] ^ 1])?;
            c.wait_on(y)?;
            Ok(())
        }),
    );
    b.process(
        "q",
        NONE,
        &[dy],
        p(move |c| {
            c.drive(dy, &[c.value(x)?[0]])?;
            c.wait_on(x)?;
            Ok(())
        }),
    );
    let cfg = Config {
        max_deltas: 50,
        ..Config::default()
    };
    let (mut sim, _) = build(b, cfg);
    let err = sim.run_until(ns(1)).unwrap_err();
    let ErrorKind::DeltaLimit { cause, .. } = &err.kind else {
        panic!("{err}")
    };
    let link = |i, n: &str, s: &str| yavs_kernel::LoopLink {
        process: ProcId(i),
        name: n.into(),
        signal: s.into(),
    };
    let yavs_kernel::LoopCause::Cycle(l) = cause else {
        panic!("{cause:?}")
    };
    // Either process may be the one that ran last; the loop is the same.
    assert!(
        *l == [link(0, "p", "x"), link(1, "q", "y")]
            || *l == [link(1, "q", "y"), link(0, "p", "x")],
        "{l:?}"
    );
}

#[test]
fn sub_element_sensitivity_and_events() {
    let mut b = Builder::new();
    let v = b.signal("v", SignalType::bytes(1, 4), &[0; 4]);
    let d = b.driver(v);
    b.process(
        "stim",
        NONE,
        &[d],
        p(move |c| {
            c.assign(
                d,
                Some(Range::new(0, 1)),
                &[(ns(1), &[1])],
                DelayMode::Transport,
            )?;
            c.assign(
                d,
                Some(Range::new(3, 1)),
                &[(ns(2), &[1])],
                DelayMode::Transport,
            )
        }),
    );
    let mut first = true;
    b.process(
        "watch",
        NONE,
        &[],
        p(move |c| {
            if !first {
                assert!(c.event(v.slice(3, 1))? && !c.event(v.slice(0, 1))? && c.event(v)?);
                assert_eq!(c.last_event(v.slice(0, 1))?, ns(1));
                c.report(Severity::Note, "v(3)");
            }
            first = false;
            c.wait_on(v.slice(3, 1))?;
            Ok(())
        }),
    );
    let (mut sim, trace) = build(b, Config::default());
    sim.run_until(ns(10)).unwrap();
    assert_eq!(
        trace.take(),
        [
            "1 ns +0 v[0..1]=[1]",
            "2 ns +0 v[3..4]=[1]",
            "2 ns +0 p1 Note: v(3)"
        ]
    );
}

#[test]
fn transaction_signal_toggles_without_events() {
    let mut b = Builder::new();
    let s = b.signal("s", BYTE, &[0]);
    let tr = b.transaction_signal(s);
    let d = b.driver(s);
    b.process(
        "stim",
        NONE,
        &[d],
        p(move |c| {
            c.assign(
                d,
                None,
                &[(ns(1), &[0]), (ns(2), &[0])],
                DelayMode::Transport,
            )
        }),
    );
    let mut n = 0;
    b.process(
        "watch",
        NONE,
        &[],
        p(move |c| {
            if n > 0 {
                assert!(c.active(s)? && !c.event(s)? && c.last_active(s)? == Time::ZERO);
            }
            n += 1;
            c.wait_on(tr)?;
            Ok(())
        }),
    );
    let (mut sim, trace) = build(b, Config::default());
    sim.run_until(ns(10)).unwrap();
    assert_eq!(
        trace.take(),
        [
            "1 ns +0 s'transaction[0..1]=\"1\"",
            "2 ns +0 s'transaction[0..1]=\"0\""
        ]
    );
}

#[test]
fn phases_steps_and_end() {
    let mut b = Builder::new();
    let s = b.signal("s", BYTE, &[0]);
    let d = b.driver(s);
    let mut step = 0;
    b.process(
        "p",
        NONE,
        &[d],
        p(move |c| {
            step += 1;
            match step {
                1 => {
                    c.drive(d, &[1])?;
                    c.wait_for(ns(10))
                }
                _ => {
                    c.output(b"bye");
                    c.finish(3);
                    Ok(())
                }
            }
        }),
    );
    let mut sim = b.build(Config::default()).unwrap();
    let trace = Trace {
        phases: true,
        ..Trace::default()
    };
    sim.subscribe(Box::new(trace.clone()));
    assert_eq!(sim.step_delta().unwrap(), RunStatus::Paused);
    assert_eq!((sim.now(), sim.delta()), (Time::ZERO, 1));
    assert_eq!(
        sim.step_timestep().unwrap(),
        RunStatus::Ended(EndReason::Finish(3))
    );
    assert_eq!(sim.now(), ns(10));
    assert_eq!(
        sim.run_until(ns(200)).unwrap(),
        RunStatus::Ended(EndReason::Finish(3))
    );
    assert_eq!(
        trace.take(),
        [
            "0 fs +0 StartOfSim",
            "0 fs +0 EndOfProcesses",
            "0 fs +0 EndOfInit",
            "0 fs +1 NextCycle",
            "0 fs +1 s[0..1]=[1]",
            "0 fs +1 StartOfProcesses",
            "0 fs +1 EndOfProcesses",
            "0 fs +1 LastKnownDelta",
            "0 fs +1 EndOfTimeStep",
            "10 ns +0 NextTimeStep",
            "10 ns +0 NextCycle",
            "10 ns +0 StartOfProcesses",
            "10 ns +0 p0 out: bye",
            "10 ns +0 EndOfProcesses",
            "10 ns +0 LastKnownDelta",
            "10 ns +0 EndOfTimeStep",
            "10 ns +0 EndOfSim",
        ]
    );
}

/// A VHPI-style callback depositing at the last known delta creates a delta (already at time 0:
/// the last-delta phases run after initialization when no delta follows, #347).
#[test]
fn hook_deposit_at_last_known_delta() {
    struct Dep(SignalId, bool);
    impl Observer for Dep {
        fn phase(&mut self, h: &mut Hook<'_>, phase: Phase) {
            if phase == Phase::LastKnownDelta && !self.1 {
                self.1 = true;
                h.deposit(self.0, &[4]).unwrap();
                assert!(h.deposit(SignalId(99), &[4]).is_err());
            }
        }
    }
    let mut b = Builder::new();
    let s = b.signal("s", BYTE, &[0]);
    b.process(
        "w",
        NONE,
        &[],
        p(move |c| {
            c.report(Severity::Note, format!("{:?}", c.value(s)?));
            c.wait_on(s)?;
            Ok(())
        }),
    );
    let mut first = true;
    b.process(
        "t",
        NONE,
        &[],
        p(move |c| {
            if std::mem::take(&mut first) {
                c.wait_for(ns(5))?;
            }
            Ok(())
        }),
    );
    let (mut sim, trace) = build(b, Config::default());
    sim.subscribe(Box::new(Dep(s, false)));
    assert_eq!(
        sim.run_until(ns(10)).unwrap(),
        RunStatus::Ended(EndReason::Idle)
    );
    assert_eq!(v(&sim, s), [4]);
    assert_eq!(
        trace.take(),
        [
            "0 fs +0 p0 Note: [0]",
            "0 fs +1 s[0..1]=[4]",
            "0 fs +1 p0 Note: [4]"
        ]
    );
}

#[test]
fn severity_and_interrupt_end_simulation() {
    let mut b = Builder::new();
    clock(&mut b);
    let mut n = 0;
    b.process(
        "chk",
        NONE,
        &[],
        p(move |c| {
            n += 1;
            if n == 3 {
                c.report(Severity::Error, "boom");
            }
            c.wait_for(ns(7))
        }),
    );
    let cfg = Config {
        stop_severity: Severity::Error,
        ..Config::default()
    };
    let (mut sim, _) = build(b, cfg);
    assert_eq!(
        sim.run_until(ns(100)).unwrap(),
        RunStatus::Ended(EndReason::Severity(Severity::Error))
    );
    assert_eq!(sim.now(), ns(14));

    let mut b = Builder::new();
    clock(&mut b);
    let (mut sim, _) = build(b, Config::default());
    sim.interrupt().pause();
    assert_eq!(sim.run_until(ns(100)).unwrap(), RunStatus::Paused);
    assert_eq!(sim.now(), Time::ZERO);
    sim.interrupt().stop();
    assert_eq!(
        sim.run_until(ns(100)).unwrap(),
        RunStatus::Ended(EndReason::Requested)
    );
}

#[test]
fn time_overflow_is_an_error_value() {
    let mut b = Builder::new();
    b.process("p", NONE, &[], p(|c| c.wait_for(Time::MAX)));
    let (mut sim, _) = build(b, Config::default());
    sim.run_until(ns(1)).unwrap();
    assert!(matches!(
        sim.run_for(Time::MAX).unwrap_err().kind,
        ErrorKind::TimeOverflow { .. }
    ));
}

#[test]
fn waits_and_transactions_past_time_high_never_mature() {
    // At TIME'HIGH the process assigns 1 ns later and waits 1 ns: both are past TIME'HIGH,
    // so the transaction is dropped, the timeout never expires and the run ends normally.
    let mut b = Builder::new();
    let s = b.signal("s", BYTE, &[0]);
    let d = b.driver(s);
    let mut step = 0;
    b.process(
        "q",
        NONE,
        &[d],
        p(move |c| {
            step += 1;
            match step {
                1 => c.wait_for(Time::MAX),
                2 => {
                    c.assign(d, None, &[(ns(1), &[1])], DelayMode::Transport)?;
                    c.wait_for(ns(1))
                }
                _ => panic!("resumed past TIME'HIGH"),
            }
        }),
    );
    let (mut sim, _) = build(b, Config::default());
    sim.run_until(Time::MAX).unwrap();
    assert_eq!(sim.now(), Time::MAX);
}

#[test]
fn bad_references_are_errors() {
    let mut b = Builder::new();
    let s = b.signal("s", BYTE, &[0]);
    b.process(
        "p",
        NONE,
        &[],
        p(move |c| {
            let e = c.value(s.slice(0, 2)).unwrap_err();
            assert_eq!(
                e.kind,
                ErrorKind::BadRef("elements 0+2 out of range for s (length 1)".into())
            );
            assert!(c.drive(DriverId(7), &[1]).is_err());
            assert!(c.wait_at(Time::ZERO).is_ok());
            c.value(SignalId(5)).map(|_| ())
        }),
    );
    let (mut sim, _) = build(b, Config::default());
    assert!(sim.value(SignalId(3)).is_err());
    assert!(sim.process_name(ProcId(3)).is_err());
    let err = sim.run_until(ns(1)).unwrap_err();
    assert_eq!(err.kind, ErrorKind::BadRef("no signal 5".into()));
    // The simulation stays stopped with the same error.
    assert_eq!(sim.step_delta().unwrap_err(), err);
    assert!(sim.deposit(s, &[1]).is_err());

    let mut b = Builder::new();
    b.signal("bad", BYTE, &[0, 0]);
    assert!(b.build(Config::default()).is_err());
}

#[test]
fn process_panic_and_resolution_failure_become_errors() {
    let mut b = Builder::new();
    b.process("boom", NONE, &[], p(|_| panic!("kaboom")));
    let (mut sim, _) = build(b, Config::default());
    let err = sim.run_until(ns(1)).unwrap_err();
    assert_eq!(
        err.kind,
        ErrorKind::Internal("process panicked: kaboom".into())
    );
    assert_eq!(err.exit_status(), 101);

    // The batch has one unwind guard: the error names the process that panicked, not the
    // first of the batch.
    let mut b = Builder::new();
    b.process("fine", NONE, &[], p(|_| Ok(())));
    b.process("boom", NONE, &[], p(|_| panic!("kaboom")));
    let (mut sim, _) = build(b, Config::default());
    let err = sim.run_until(ns(1)).unwrap_err();
    let who = err.process.as_ref().map(|(p, n)| (p.0, n.as_str()));
    assert_eq!(who, Some((1, "boom")));

    let fails = Resolution::Func(Arc::new(|_: &[&[u8]], _: &mut [u8]| Err("conflict".into())));
    let mut b = Builder::new();
    let r = b.resolved_signal("r", BYTE, &[0], fails);
    let d = b.driver(r);
    b.process("drv", NONE, &[d], p(move |c| c.drive(d, &[1])));
    assert_eq!(
        b.build(Config::default()).err().unwrap().kind,
        ErrorKind::Resolution {
            signal: "r".into(),
            message: "conflict".into()
        }
    );
}

/// A record `(a : std_ulogic_vector(2 downto 0); n : integer)` resolved as a whole (#83): the
/// logic leaf stays on bit-planes, the resolver sees whole records (3 code bytes, then 4).
fn composite(only_n: bool) -> Result<(SimControl, SignalId, SignalId), SimError> {
    let mut b = Builder::new();
    let a = b.signal("r.a", SignalType::logic(3), &LogicVec::new(3, StdULogic::U));
    let n = b.signal("r.n", SignalType::bytes(4, 1), &[0; 4]);
    let one = StdULogic::One as u8;
    b.resolved_composite(
        &[a, n],
        Arc::new(move |srcs: &[&[u8]], out: &mut [u8]| {
            // a: '1' where any source has '1', else '0'; n: the sum.
            for k in 0..3 {
                let any = srcs.iter().any(|s| s[k] == one);
                out[k] = if any { one } else { StdULogic::Zero as u8 };
            }
            let sum: i32 = srcs
                .iter()
                .map(|s| i32::from_le_bytes(s[3..7].try_into().unwrap()))
                .sum();
            out[3..7].copy_from_slice(&sum.to_le_bytes());
            Ok(())
        }),
    );
    for (k, (bits, val)) in [("100", 3i32), ("001", 4)].into_iter().enumerate() {
        let only = only_n && k == 1;
        let da = (!only).then(|| b.driver(a));
        let dn = b.driver(n);
        let ds: Vec<DriverId> = da.into_iter().chain([dn]).collect();
        b.process(
            &format!("p{k}"),
            ProcFlags::default(),
            &ds,
            p(move |c| {
                let at = ns(k as u64 + 1);
                if let Some(da) = da {
                    let l: LogicVec = bits.parse().unwrap();
                    c.assign(da, None, &[(at, &l)], DelayMode::Transport)?;
                }
                c.assign(dn, None, &[(at, &val.to_le_bytes())], DelayMode::Transport)?;
                Ok(())
            }),
        );
    }
    Ok((b.build(Config::default())?, a, n))
}

#[test]
fn composite_resolved_as_a_whole() {
    let (mut sim, a, n) = composite(false).unwrap();
    let get = |sim: &SimControl| {
        let Value::Logic(l) = sim.value(a).unwrap() else {
            panic!("logic leaf not on planes")
        };
        let n = i32::from_le_bytes(v(sim, n).try_into().unwrap());
        (l.to_string(), n)
    };
    // Initialisation resolves the drivers' initial values (U, 0): '0' and 0.
    assert_eq!(get(&sim), ("000".into(), 0));
    sim.run_until(ns(1)).unwrap();
    assert_eq!(get(&sim), ("100".into(), 3));
    sim.run_until(ns(3)).unwrap();
    assert_eq!(get(&sim), ("101".into(), 7));

    let Err(e) = composite(true) else {
        panic!("a process driving only r.n must be refused")
    };
    assert!(
        matches!(&e.kind, ErrorKind::Usage(m) if m.contains("does not share the sources")),
        "{e:?}"
    );
}

/// An array of two records `(b : std_ulogic; n : integer)` resolved element by element, the
/// record subtype resolved as a whole: one group per element over slices of the two leaves.
/// Element 0 has two sources, element 1 one: each group sees only its own.
#[test]
fn composite_groups_over_slices() {
    let mut b = Builder::new();
    let bl = b.signal("a.b", SignalType::logic(2), &LogicVec::new(2, StdULogic::U));
    let n = b.signal("a.n", SignalType::bytes(4, 2), &[0; 8]);
    // Sum of n and the source count in b ('1' for two or more sources).
    let f: Resolver = Arc::new(|srcs: &[&[u8]], out: &mut [u8]| {
        out[0] = if srcs.len() > 1 {
            StdULogic::One
        } else {
            StdULogic::Zero
        } as u8;
        let sum: i32 = srcs
            .iter()
            .map(|s| i32::from_le_bytes(s[1..5].try_into().unwrap()))
            .sum();
        out[1..5].copy_from_slice(&sum.to_le_bytes());
        Ok(())
    });
    // Logic leaves are mirrored: element 0 is offset 1.
    for (lb, ln) in [(1, 0), (0, 4)] {
        b.resolved_composite(&[bl.slice(lb, 1), n.slice(ln / 4, 1)], f.clone());
    }
    // p0 drives element 0, p1 the whole array.
    for (k, whole) in [false, true].into_iter().enumerate() {
        let (db, dn) = if whole {
            (b.driver(bl), b.driver(n))
        } else {
            (b.driver(bl.slice(1, 1)), b.driver(n.slice(0, 1)))
        };
        b.process(
            &format!("p{k}"),
            ProcFlags::default(),
            &[db, dn],
            p(move |c| {
                let v: Vec<u8> = if whole {
                    [5i32, 7].iter().flat_map(|x| x.to_le_bytes()).collect()
                } else {
                    3i32.to_le_bytes().to_vec()
                };
                c.assign(dn, None, &[(ns(1), &v)], DelayMode::Transport)?;
                Ok(())
            }),
        );
    }
    let mut sim = b.build(Config::default()).unwrap();
    sim.run_until(ns(1)).unwrap();
    let Value::Logic(l) = sim.value(bl).unwrap() else {
        panic!("logic leaf not on planes")
    };
    // Element 0 (offset 1): two sources; element 1: one.
    assert_eq!(l.to_string(), "10");
    assert_eq!(
        v(&sim, n),
        [8i32, 7]
            .iter()
            .flat_map(|x| x.to_le_bytes())
            .collect::<Vec<_>>()
    );
}

/// A composite resolved as a whole without sources keeps its initial value: the resolver
/// is not called (LRM 14.7.3.2 a, #328).
#[test]
fn composite_without_sources() {
    let mut b = Builder::new();
    let n = b.signal("r.n", SignalType::bytes(4, 1), &7i32.to_le_bytes());
    b.resolved_composite(
        &[n],
        Arc::new(|_: &[&[u8]], _: &mut [u8]| Err("called without sources".into())),
    );
    let mut sim = b.build(Config::default()).unwrap();
    sim.run_until(ns(1)).unwrap();
    assert_eq!(v(&sim, n), 7i32.to_le_bytes());
}

/// #252: at initialisation elaboration order; then the processes with one suspension point in
/// elaboration order, then the others in reverse, those resumed by a signal before those
/// resumed by a timeout. The same at any thread count and with shuffled execution.
#[test]
fn process_order_by_shape() {
    let run = |cfg: Config| {
        let mut b = Builder::new();
        let s = b.signal("s", BYTE, &[0]);
        let d = b.driver(s);
        // (several waits, resumed by a timeout)
        for (k, (several, timeout)) in [
            (true, false),
            (false, false),
            (true, true),
            (true, false),
            (false, false),
        ]
        .into_iter()
        .enumerate()
        {
            let mut started = false;
            let flags = ProcFlags {
                several_waits: several,
                concurrent_call: k == 0, // p0: a concurrent call goes first among the several-waits ones
                ..NONE
            };
            b.process(
                &format!("p{k}"),
                flags,
                &[],
                p(move |c| {
                    if started {
                        c.report(Severity::Note, "resumed");
                        return Ok(());
                    }
                    started = true;
                    c.report(Severity::Note, "init");
                    if timeout {
                        c.wait_for(ns(1))
                    } else {
                        c.wait_on(s)
                    }
                }),
            );
        }
        b.process(
            "drv",
            NONE,
            &[d],
            p(move |c| c.assign(d, None, &[(ns(1), &[1])], DelayMode::INERTIAL)),
        );
        let (mut sim, trace) = build(b, cfg);
        sim.run_until(ns(5)).unwrap();
        trace
            .take()
            .into_iter()
            .filter(|l| l.contains("Note"))
            .collect::<Vec<_>>()
    };
    let want: Vec<String> = [
        "0 fs +0 p0 Note: init",
        "0 fs +0 p1 Note: init",
        "0 fs +0 p2 Note: init",
        "0 fs +0 p3 Note: init",
        "0 fs +0 p4 Note: init",
        "1 ns +0 p1 Note: resumed",
        "1 ns +0 p4 Note: resumed",
        "1 ns +0 p0 Note: resumed",
        "1 ns +0 p3 Note: resumed",
        "1 ns +0 p2 Note: resumed",
    ]
    .map(String::from)
    .to_vec();
    assert_eq!(run(Config::default()), want);
    let threads = Config {
        threads: 4,
        ..Config::default()
    };
    assert_eq!(run(threads), want);
    let shuffled = Config {
        shuffle: Some(7),
        threads: 4,
        ..Config::default()
    };
    assert_eq!(run(shuffled), want);
}

/// An observer that calls `f` with `None` at `StartOfSim` and with the id of each wake-up.
struct Waker<F>(F);

impl<F: FnMut(&mut Hook<'_>, Option<WakeId>) + Send> Observer for Waker<F> {
    fn phase(&mut self, h: &mut Hook<'_>, phase: Phase) {
        if phase == Phase::StartOfSim {
            (self.0)(h, None);
        }
    }
    fn wake(&mut self, h: &mut Hook<'_>, id: WakeId) {
        (self.0)(h, Some(id));
    }
}

/// A process reporting `s` on every change, and nothing else scheduled.
fn quiet_design() -> (Builder, SignalId) {
    let mut b = Builder::new();
    let s = b.signal("s", BYTE, &[0]);
    b.process(
        "w",
        NONE,
        &[],
        p(move |c| {
            c.report(Severity::Note, format!("{:?}", c.value(s)?));
            c.wait_on(s)
        }),
    );
    (b, s)
}

fn phase_trace(b: Builder) -> (SimControl, Trace) {
    let mut sim = b.build(Config::default()).unwrap();
    let trace = Trace {
        phases: true,
        ..Trace::default()
    };
    sim.subscribe(Box::new(trace.clone()));
    (sim, trace)
}

/// A wake-up gives a cycle at its time with the usual phases, a deposit from it applies in
/// that cycle, and the simulation is not idle before it.
#[test]
fn observer_wake_up_defers_idle_end() {
    let (b, s) = quiet_design();
    let (mut sim, trace) = phase_trace(b);
    let lines = trace.lines.clone();
    sim.subscribe(Box::new(Waker(
        move |h: &mut Hook<'_>, id: Option<WakeId>| match id {
            None => assert_eq!(h.wake_at(ns(7)).unwrap().time(), ns(7)),
            Some(id) => {
                lines.lock().unwrap().push(format!("wake at {}", id.time()));
                h.deposit(s, &[4]).unwrap();
            }
        },
    )));
    assert_eq!(sim.run_until(ns(5)).unwrap(), RunStatus::Paused);
    assert_eq!(
        sim.run_until(ns(20)).unwrap(),
        RunStatus::Ended(EndReason::Idle)
    );
    assert_eq!(sim.now(), ns(7));
    assert_eq!(
        trace.take(),
        [
            "0 fs +0 StartOfSim",
            "0 fs +0 p0 Note: [0]",
            "0 fs +0 EndOfProcesses",
            "0 fs +0 EndOfInit",
            "0 fs +0 LastKnownDelta",
            "0 fs +0 EndOfTimeStep",
            "7 ns +0 NextTimeStep",
            "7 ns +0 NextCycle",
            "wake at 7 ns",
            "7 ns +0 s[0..1]=[4]",
            "7 ns +0 StartOfProcesses",
            "7 ns +0 p0 Note: [4]",
            "7 ns +0 EndOfProcesses",
            "7 ns +0 LastKnownDelta",
            "7 ns +0 EndOfTimeStep",
            "7 ns +0 EndOfSim",
        ]
    );
}

/// A wake-up in the past is an error (E3005); one at `now` fires in the next delta.
#[test]
fn observer_wake_up_in_the_past_or_now() {
    let (b, _) = quiet_design();
    let (mut sim, trace) = phase_trace(b);
    let lines = trace.lines.clone();
    sim.subscribe(Box::new(Waker(
        move |h: &mut Hook<'_>, id: Option<WakeId>| match id {
            None => _ = h.wake_at(ns(7)).unwrap(),
            Some(_) => {
                lines
                    .lock()
                    .unwrap()
                    .push(format!("wake {} +{}", h.now(), h.delta()));
                if h.delta() == 0 {
                    let e = h.wake_at(ns(6)).unwrap_err();
                    assert_eq!(e.kind.code(), "E3005");
                    h.wake_at(h.now()).unwrap();
                }
            }
        },
    )));
    assert_eq!(
        sim.run_until(ns(20)).unwrap(),
        RunStatus::Ended(EndReason::Idle)
    );
    assert_eq!(
        trace.take()[4..],
        [
            "0 fs +0 LastKnownDelta",
            "0 fs +0 EndOfTimeStep",
            "7 ns +0 NextTimeStep",
            "7 ns +0 NextCycle",
            "wake 7 ns +0",
            "7 ns +0 StartOfProcesses",
            "7 ns +0 EndOfProcesses",
            "7 ns +1 NextCycle",
            "wake 7 ns +1",
            "7 ns +1 StartOfProcesses",
            "7 ns +1 EndOfProcesses",
            "7 ns +1 LastKnownDelta",
            "7 ns +1 EndOfTimeStep",
            "7 ns +1 EndOfSim",
        ]
    );
}

/// A cancelled wake-up neither fires nor keeps the simulation alive.
#[test]
fn observer_wake_up_cancel() {
    let (b, _) = quiet_design();
    let (mut sim, trace) = phase_trace(b);
    let lines = trace.lines.clone();
    let mut later = None;
    sim.subscribe(Box::new(Waker(
        move |h: &mut Hook<'_>, id: Option<WakeId>| match id {
            None => {
                let gone = h.wake_at(ns(1)).unwrap();
                assert!(h.cancel(gone));
                assert!(!h.cancel(gone));
                h.wake_at(ns(3)).unwrap();
                later = Some(h.wake_at(ns(6)).unwrap());
            }
            Some(id) => {
                lines.lock().unwrap().push(format!("wake at {}", id.time()));
                assert!(!h.cancel(id), "a fired wake-up is no longer pending");
                assert!(h.cancel(later.take().unwrap()));
            }
        },
    )));
    assert_eq!(
        sim.run_until(ns(20)).unwrap(),
        RunStatus::Ended(EndReason::Idle)
    );
    assert_eq!(sim.now(), ns(3));
    assert_eq!(
        trace
            .take()
            .into_iter()
            .filter(|l| l.starts_with("wake"))
            .collect::<Vec<_>>(),
        ["wake at 3 ns"]
    );
}

/// Each observer gets only its own wake-ups, in time order, then registration order.
#[test]
fn observer_wake_ups_of_many_observers() {
    let (b, _) = quiet_design();
    let mut sim = b.build(Config::default()).unwrap();
    let lines = Arc::new(std::sync::Mutex::new(Vec::new()));
    for tag in 0..3u64 {
        let lines = lines.clone();
        let mut mine = Vec::new();
        sim.subscribe(Box::new(Waker(
            move |h: &mut Hook<'_>, id: Option<WakeId>| match id {
                None => {
                    mine.push(h.wake_at(ns(3 - tag)).unwrap());
                    mine.push(h.wake_at(ns(5)).unwrap());
                }
                Some(id) => {
                    assert!(mine.contains(&id), "observer {tag} got a foreign wake-up");
                    lines.lock().unwrap().push(format!("{} o{tag}", id.time()));
                }
            },
        )));
    }
    assert_eq!(
        sim.run_until(ns(20)).unwrap(),
        RunStatus::Ended(EndReason::Idle)
    );
    assert_eq!(
        *lines.lock().unwrap(),
        [
            "1 ns o2", "2 ns o1", "3 ns o0", "5 ns o0", "5 ns o1", "5 ns o2"
        ]
    );
}

/// A clocked process with an edge trigger runs only on its edge, and sees the same edges as
/// without the trigger (`To_X01` of value and `'last_value`: 'H' after 'L' rises, 'X' does not).
#[test]
fn edge_trigger_skips_other_events() {
    let run = |triggers: bool, rising: bool| {
        let mut b = Builder::new();
        let clk = b.signal(
            "clk",
            SignalType::logic(1),
            &LogicVec::new(1, StdULogic::Zero),
        );
        let d = b.driver(clk);
        let mut step = 0;
        b.process(
            "stim",
            NONE,
            &[d],
            p(move |c| {
                if let Some(ch) = "1H0L1X1L0H".chars().nth(step) {
                    let l: LogicVec = ch.to_string().parse().unwrap();
                    c.assign(d, None, &[(Time::ZERO, &l)], DelayMode::INERTIAL)?;
                    step += 1;
                    c.wait_for(ns(5))?;
                }
                Ok(())
            }),
        );
        let (mut runs, mut edges) = (0u32, 0u32);
        let seen = Arc::new(std::sync::Mutex::new((0, 0)));
        let out = seen.clone();
        let reg = b.process(
            "reg",
            NONE,
            &[],
            p(move |c| {
                runs += 1;
                let to_x01 = |l: LogicVec| match l.get(0) {
                    StdULogic::One | StdULogic::H => Some(true),
                    StdULogic::Zero | StdULogic::L => Some(false),
                    _ => None,
                };
                let now = to_x01(c.logic(clk)?);
                let Value::Logic(before) = c.last_value(clk)? else {
                    unreachable!()
                };
                if c.event(clk)? && now == Some(rising) && to_x01(before) == Some(!rising) {
                    edges += 1;
                }
                *out.lock().unwrap() = (runs, edges);
                c.wait_on(clk)
            }),
        );
        b.edge_trigger(
            reg,
            EdgeTrigger {
                signal: clk,
                elem: 0,
                rising,
                legacy: false,
                reset: None,
            },
        );
        let cfg = Config {
            edge_triggers: triggers,
            ..Config::default()
        };
        let (mut sim, _) = build(b, cfg);
        sim.run_until(ns(100)).unwrap();
        *seen.lock().unwrap()
    };
    // Rising: 0->1, L->1, 0->H (X->1 is not one). Falling: H->0, 1->L. Events: 10.
    assert_eq!(run(false, true), (11, 3));
    assert_eq!(run(true, true), (4, 3));
    assert_eq!(run(false, false), (11, 2));
    assert_eq!(run(true, false), (3, 2));
}

#[test]
fn edge_query_is_the_same_with_and_without_triggers() {
    let run = |triggers: bool, rising: bool| {
        let mut b = Builder::new();
        let clk = b.signal(
            "clk",
            SignalType::logic(1),
            &LogicVec::new(1, StdULogic::Zero),
        );
        let d = b.driver(clk);
        let mut step = 0;
        b.process(
            "stim",
            NONE,
            &[d],
            p(move |c| {
                if let Some(ch) = "1H0L1X1L0H".chars().nth(step) {
                    let l: LogicVec = ch.to_string().parse().unwrap();
                    c.assign(d, None, &[(Time::ZERO, &l)], DelayMode::INERTIAL)?;
                    step += 1;
                    c.wait_for(ns(5))?;
                }
                Ok(())
            }),
        );
        let seen = Arc::new(std::sync::Mutex::new(0u32));
        let out = seen.clone();
        let reg = b.process(
            "reg",
            NONE,
            &[],
            p(move |c| {
                if c.edge(clk.slice(0, 1), rising)? == Some(true) {
                    *out.lock().unwrap() += 1;
                }
                c.wait_on(clk)
            }),
        );
        b.edge_trigger(
            reg,
            EdgeTrigger {
                signal: clk,
                elem: 0,
                rising,
                legacy: false,
                reset: None,
            },
        );
        let cfg = Config {
            edge_triggers: triggers,
            ..Config::default()
        };
        let (mut sim, _) = build(b, cfg);
        sim.run_until(ns(100)).unwrap();
        *seen.lock().unwrap()
    };
    for rising in [true, false] {
        assert_eq!(run(true, rising), run(false, rising));
    }
    assert_eq!(run(true, true), 3);
    assert_eq!(run(true, false), 2);
}

/// A wait that names the edge it needs (`Ctx::wait_edge`) resumes its process only in a cycle
/// with that edge, also for events of the other signals of the wait; the edge may change from
/// wait to wait. With `Config::edge_triggers` off every event resumes it.
#[test]
fn wait_edge_resumes_only_on_the_edge() {
    let run = |triggers: bool| {
        let mut b = Builder::new();
        let clk = b.signal(
            "clk",
            SignalType::logic(1),
            &LogicVec::new(1, StdULogic::Zero),
        );
        let v = b.signal(
            "v",
            SignalType::logic(1),
            &LogicVec::new(1, StdULogic::Zero),
        );
        let (dc, dv) = (b.driver(clk), b.driver(v));
        // `clk` toggles every 10 ns, `v` 3 ns after each of its edges and at 1 ns after that.
        let mut step = 0u32;
        b.process(
            "stim",
            NONE,
            &[dc, dv],
            p(move |c| {
                let (l1, l0): (LogicVec, LogicVec) = ("1".parse().unwrap(), "0".parse().unwrap());
                let hi = step.is_multiple_of(2);
                let l = if hi { &l1 } else { &l0 };
                c.assign(dc, None, &[(Time::ZERO, l)], DelayMode::INERTIAL)?;
                c.assign(dv, None, &[(ns(3), l)], DelayMode::INERTIAL)?;
                step += 1;
                c.wait_for(ns(10))
            }),
        );
        let seen = Arc::new(std::sync::Mutex::new((0u32, 0u32)));
        let out = seen.clone();
        let (mut runs, mut hits) = (0u32, 0u32);
        let mut started = false;
        b.process(
            "reg",
            NONE,
            &[],
            p(move |c| {
                runs += 1;
                // Four waits for the rising edge, then four for the falling one, each a loop that
                // waits again when the edge does not hold (a `wait until` with an extra signal).
                if std::mem::replace(&mut started, true) {
                    let rising = hits < 4;
                    if c.edge(clk.slice(0, 1), rising)? == Some(true) {
                        hits += 1;
                    }
                }
                *out.lock().unwrap() = (runs, hits);
                c.wait_edge(clk.slice(0, 1), u8::from(hits < 4));
                c.wait_on(clk)?;
                c.wait_on(v)
            }),
        );
        let cfg = Config {
            edge_triggers: triggers,
            ..Config::default()
        };
        let (mut sim, _) = build(b, cfg);
        sim.run_until(ns(100)).unwrap();
        *seen.lock().unwrap()
    };
    let (with, without) = (run(true), run(false));
    // The same edges are found either way; the runs for events without the edge are not made.
    assert!(with.0 < without.0, "{with:?} {without:?}");
    assert_eq!(with.1, without.1, "{with:?} {without:?}");
    assert!(with.1 >= 6, "{with:?}");
}

#[test]
fn hook_names_the_reporting_process() {
    struct Names(Arc<std::sync::Mutex<Vec<String>>>);
    impl Observer for Names {
        fn report(&mut self, h: &mut Hook<'_>, r: &Report) {
            let name = h.process_name(r.process.unwrap()).unwrap_or("?");
            self.0.lock().unwrap().push(name.to_string());
        }
    }
    let mut b = Builder::new();
    for name in [":top:a", ":top:b"] {
        b.process(
            name,
            ProcFlags::default(),
            &[],
            p(|c| {
                c.report(Severity::Note, "hi");
                c.wait_for(ns(1))?;
                Ok(())
            }),
        );
    }
    let mut sim = b.build(Config::default()).unwrap();
    let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
    sim.subscribe(Box::new(Names(seen.clone())));
    sim.run_until(ns(0)).unwrap();
    assert_eq!(*seen.lock().unwrap(), [":top:a", ":top:b"]);
}

/// Assignments that change nothing: with `skip_unchanged` they queue no transaction, yet every
/// delta, phase and report is where it was.
fn unchanged_scenario(skip: bool) -> Vec<String> {
    let mut b = Builder::new();
    let (s, t, u) = (
        b.signal("s", BYTE, &[3]),
        b.signal("t", BYTE, &[0]),
        b.signal("u", BYTE, &[0]),
    );
    let (ds, dt, du) = (b.driver(s), b.driver(t), b.driver(u));
    let post = ProcFlags {
        postponed: true,
        ..NONE
    };
    // Same value, in the first delta (an empty delta follows) and at 5 ns.
    b.process(
        "same",
        NONE,
        &[ds],
        p(move |c| {
            c.drive(ds, &[3])?;
            if c.now() == Time::ZERO {
                c.wait_for(ns(5))?;
            }
            Ok(())
        }),
    );
    // Same value, then a different one after a delay that preempts the first (no delta at all),
    // then a different value at once after the same one.
    b.process(
        "preempted",
        NONE,
        &[dt],
        p(move |c| {
            if c.now() == Time::ZERO {
                c.drive(dt, &[0])?;
                c.assign(dt, None, &[(ns(2), &[1u8])], DelayMode::INERTIAL)?;
                c.wait_for(ns(4))?;
            } else if c.now() == ns(4) {
                c.drive(dt, &[1])?;
                c.drive(dt, &[7])?;
            }
            Ok(())
        }),
    );
    b.process(
        "late",
        NONE,
        &[du],
        p(move |c| {
            c.wait_for(ns(9))?;
            c.drive(du, &[0])
        }),
    );
    b.process(
        "watch",
        post,
        &[],
        p(move |c| {
            c.report(Severity::Note, format!("t={:?}", c.value(t)?));
            c.wait_on(s)?;
            c.wait_on(t)?;
            c.wait_on(u)?;
            c.wait_for(ns(1))
        }),
    );
    let mut sim = b
        .build(Config {
            skip_unchanged: skip,
            ..Config::default()
        })
        .unwrap();
    let trace = Trace {
        phases: true,
        ..Trace::default()
    };
    sim.subscribe(Box::new(trace.clone()));
    sim.run_until(ns(20)).unwrap();
    trace.take()
}

#[test]
fn skipped_unchanged_assignments_keep_the_deltas() {
    let (off, on) = (unchanged_scenario(false), unchanged_scenario(true));
    assert!(off.iter().any(|l| l.contains("+1 NextCycle")));
    assert_eq!(off, on);
}

/// The legacy form (`clk'event and clk = '1'`) and an asynchronous reset level: the process
/// runs exactly when its condition can be true, and the same number of times it matters.
#[test]
fn legacy_and_reset_triggers_skip_other_events() {
    let logic = |ch: char| -> LogicVec { ch.to_string().parse().unwrap() };
    let run = |triggers: bool| {
        let mut b = Builder::new();
        let mk = |b: &mut Builder, name: &str, seq: &'static str| {
            let s = b.resolved_signal(
                name,
                SignalType::logic(1),
                &LogicVec::new(1, StdULogic::Zero),
                Resolution::StdLogic,
            );
            let d = b.driver(s);
            let mut step = 0;
            b.process(
                name,
                NONE,
                &[d],
                p(move |c| {
                    if let Some(ch) = seq.chars().nth(step) {
                        c.assign(d, None, &[(Time::ZERO, &logic(ch))], DelayMode::INERTIAL)?;
                        step += 1;
                        c.wait_for(ns(5))?;
                    }
                    Ok(())
                }),
            );
            s
        };
        // clk: 0 1 X 1 0 1 H 0 1 1 (legacy rising = event ending on '1': 1, X->1 and 1 = no
        // event, 0->1, H->0 no, 0->1); rst: '1' for the 4th and 5th step.
        let clk = mk(&mut b, "clk", "1X1010H011");
        let rst = mk(&mut b, "rst", "0001100000");
        let seen = Arc::new(std::sync::Mutex::new((0u32, 0u32)));
        let out = seen.clone();
        let (mut runs, mut needed) = (0u32, 0u32);
        let reg = b.process(
            "reg",
            NONE,
            &[],
            p(move |c| {
                runs += 1;
                let one = |s| c.logic(s).map(|l| l.get(0) == StdULogic::One);
                if (c.event(clk)? && one(clk)?) || one(rst)? {
                    needed += 1;
                }
                *out.lock().unwrap() = (runs, needed);
                c.wait_on(clk)?;
                c.wait_on(rst)
            }),
        );
        b.edge_trigger(
            reg,
            EdgeTrigger {
                signal: clk,
                elem: 0,
                rising: true,
                legacy: true,
                reset: Some((rst, 0, StdULogic::One as u8)),
            },
        );
        let cfg = Config {
            edge_triggers: triggers,
            ..Config::default()
        };
        let (mut sim, _) = build(b, cfg);
        sim.run_until(ns(100)).unwrap();
        *seen.lock().unwrap()
    };
    let (all, needed) = run(false);
    let (held, needed_held) = run(true);
    assert_eq!(needed, needed_held);
    assert!(needed > 0 && all > needed + 1);
    // The initial run, then only the activations that could do something.
    assert_eq!(held, needed + 1);
}

/// A wait on exactly the range the last wait registered (`Ctx::wait_on_again`) is the same wait:
/// three counters on the clock, one with `wait_on`, one asking `wait_on_again` first, one that
/// now and then waits on a second signal too, count the same edges.
#[test]
fn a_wait_on_the_last_range_is_the_same_wait() {
    let mut b = Builder::new();
    let clk = clock(&mut b);
    let other = b.signal("other", BYTE, &[0]);
    let counts: Vec<SignalId> = (0..3)
        .map(|i| b.signal(&format!("n{i}"), SignalType::bytes(4, 1), &[0; 4]))
        .collect();
    for (i, &n) in counts.iter().enumerate() {
        let d = b.driver(n);
        let mut k = 0u32;
        b.process(
            &format!("counter{i}"),
            NONE,
            &[d],
            p(move |c| {
                if c.event(clk)? && c.value(clk)? == [1] {
                    c.drive(d, &(u32_of(c.value(n)?) + 1).to_le_bytes())?;
                }
                k += 1;
                match i {
                    0 => c.wait_on(clk),
                    1 => {
                        if !c.wait_on_again(clk, Range::new(0, 1)) {
                            c.wait_on(clk)?;
                        }
                        Ok(())
                    }
                    _ => {
                        // Every fifth wait also names `other` (never changes), so the waits
                        // differ from the last one now and then.
                        if k.is_multiple_of(5) {
                            c.wait_on(other)?;
                            c.wait_on(clk)
                        } else if !c.wait_on_again(clk, Range::new(0, 1)) {
                            c.wait_on(clk)
                        } else {
                            Ok(())
                        }
                    }
                }
            }),
        );
    }
    let (mut sim, _) = build(b, Config::default());
    assert_eq!(sim.run_until(ns(1000)).unwrap(), RunStatus::Paused);
    for &n in &counts {
        assert_eq!(u32_of(&v(&sim, n)), 100);
    }
}

/// A wait that starts with `wait_on_again` and names more (another signal, a timeout) is the
/// wait of everything it names: the processes count how often they wake.
#[test]
fn a_wait_that_adds_to_the_last_one_keeps_everything() {
    let mut b = Builder::new();
    let clk = clock(&mut b);
    let other = b.signal("other", BYTE, &[0]);
    let od = b.driver(other);
    b.process(
        "toggler",
        NONE,
        &[od],
        p(move |c| {
            let v = c.value(other)?[0] ^ 1;
            c.assign(od, None, &[(ns(7), &[v])], DelayMode::INERTIAL)?;
            c.wait_for(ns(7))
        }),
    );
    // Waiters 0 and 1 name clk and other, 2 also a timeout that never fires first, 3 and 4 name
    // clk and a timeout that does; the even ones make the clk part with `wait_on_again`.
    let counts: Vec<SignalId> = (0..5)
        .map(|i| b.signal(&format!("n{i}"), SignalType::bytes(4, 1), &[0; 4]))
        .collect();
    for (i, &n) in counts.iter().enumerate() {
        let d = b.driver(n);
        b.process(
            &format!("waiter{i}"),
            NONE,
            &[d],
            p(move |c| {
                c.drive(d, &(u32_of(c.value(n)?) + 1).to_le_bytes())?;
                if i % 2 == 1 || !c.wait_on_again(clk, Range::new(0, 1)) {
                    c.wait_on(clk)?;
                }
                match i {
                    0 | 1 => c.wait_on(other),
                    2 => {
                        c.wait_on(other)?;
                        c.wait_for(ns(33))
                    }
                    _ => c.wait_for(ns(3)),
                }
            }),
        );
    }
    let (mut sim, _) = build(b, Config::default());
    assert_eq!(sim.run_until(ns(1000)).unwrap(), RunStatus::Paused);
    let n: Vec<u32> = counts.iter().map(|&s| u32_of(&v(&sim, s))).collect();
    assert_eq!(n[0], n[1], "{n:?}");
    assert_eq!(n[3], n[4], "{n:?}");
    assert!(n[3] > n[0] && n[2] == n[0], "{n:?}");
}

#[test]
fn postponed_processes_run_in_process_order_when_no_other_process_resumes() {
    // At 5 ns two postponed processes resume through different signals and no ordinary process
    // runs in that cycle: they still run in elaboration order, not in the order they woke.
    let mut b = Builder::new();
    let s = b.signal("s", BYTE, &[0]);
    let t = b.signal("t", BYTE, &[0]);
    let (ds, dt) = (b.driver(s), b.driver(t));
    let post = ProcFlags {
        postponed: true,
        serial: false,
        several_waits: false,
        concurrent_call: false,
    };
    for (name, sig) in [("on_t", t), ("on_s", s)] {
        b.process(
            name,
            post,
            &[],
            p(move |c| {
                c.report(Severity::Note, format!("{name} {:?}", c.value(sig)?));
                c.wait_on(sig)
            }),
        );
    }
    b.process(
        "src",
        NONE,
        &[ds, dt],
        p(move |c| {
            c.assign(ds, None, &[(ns(5), &[1][..])], DelayMode::INERTIAL)?;
            c.assign(dt, None, &[(ns(5), &[1][..])], DelayMode::INERTIAL)
        }),
    );
    let (mut sim, trace) = build(b, Config::default());
    sim.run_until(ns(10)).unwrap();
    let lines = trace.take();
    let late: Vec<_> = lines.iter().filter(|l| l.contains("Note")).collect();
    assert!(
        late[2].contains("on_t") && late[3].contains("on_s"),
        "{lines:?}"
    );
}

/// A wait of several ranges named again one by one (`Ctx::wait_on_again`, falling back to
/// `wait_on`) is the wait of the ranges it names, whatever it names after how many of the last
/// wait's: a prefix, another order, more, a timeout. The twin of each process names the same
/// ranges with `wait_on` only; both count the same wakes.
#[test]
fn a_wait_of_several_ranges_named_again_is_the_same_wait() {
    let mut b = Builder::new();
    let clk = clock(&mut b);
    let sigs: Vec<SignalId> = (0..3)
        .map(|i| b.signal(&format!("s{i}"), BYTE, &[0]))
        .collect();
    for (i, &s) in sigs.iter().enumerate() {
        let d = b.driver(s);
        let period = [7u64, 11, 13][i];
        b.process(
            &format!("toggler{i}"),
            NONE,
            &[d],
            p(move |c| {
                let v = c.value(s)?[0] ^ 1;
                c.assign(d, None, &[(ns(period), &[v])], DelayMode::INERTIAL)?;
                c.wait_for(ns(period))
            }),
        );
    }
    // What each activation waits on: indices into [clk, s0, s1, s2], and a timeout.
    let plan: [(&[usize], bool); 8] = [
        (&[0, 1, 2, 3], false),
        (&[0, 1, 2, 3], false),
        (&[0, 1], false),
        (&[0, 1, 2, 3], false),
        (&[1, 0, 2], false),
        (&[0, 1, 2, 3, 0], false),
        (&[0, 1, 2], true),
        (&[0, 1, 2, 3], false),
    ];
    let all = [clk, sigs[0], sigs[1], sigs[2]];
    let counts: Vec<SignalId> = (0..2)
        .map(|i| b.signal(&format!("n{i}"), SignalType::bytes(4, 1), &[0; 4]))
        .collect();
    for (i, &n) in counts.iter().enumerate() {
        let d = b.driver(n);
        let mut k = 0usize;
        b.process(
            &format!("waiter{i}"),
            NONE,
            &[d],
            p(move |c| {
                c.drive(d, &(u32_of(c.value(n)?) + 1).to_le_bytes())?;
                let (set, timed) = plan[k % plan.len()];
                k += 1;
                for &x in set {
                    let r = Range::new(0, 1);
                    // The twin (i = 1) never asks.
                    if i == 1 || !c.wait_on_again(all[x], r) {
                        c.wait_on(all[x])?;
                    }
                }
                if timed {
                    c.wait_for(ns(5))?;
                }
                Ok(())
            }),
        );
    }
    let (mut sim, _) = build(b, Config::default());
    assert_eq!(sim.run_until(ns(5000)).unwrap(), RunStatus::Paused);
    let n: Vec<u32> = counts.iter().map(|&s| u32_of(&v(&sim, s))).collect();
    assert_eq!(n[0], n[1], "{n:?}");
    assert!(n[0] > 100, "{n:?}");
}
