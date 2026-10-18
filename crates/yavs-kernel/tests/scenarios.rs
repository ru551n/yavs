//! LRM 14.7 scenarios with hand-written processes.

mod common;

use std::sync::Arc;

use common::*;
use yavs_kernel::*;

const NONE: ProcFlags = ProcFlags {
    postponed: false,
    serial: false,
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
            processes: vec!["osc".into()]
        }
    );
    assert_eq!(
        err.to_string(),
        "0 fs +100: delta cycle limit 100 exceeded; active processes: osc"
    );
    assert_eq!(err.exit_status(), 1);
    assert_eq!(sim.run_until(ns(1)).unwrap_err(), err);
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
            "1 ns +0 s'transaction[0..1]=[1]",
            "2 ns +0 s'transaction[0..1]=[0]"
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
            "0 fs +1 s[0..1]=[1]",
            "0 fs +1 LastKnownDelta",
            "0 fs +1 EndOfTimeStep",
            "10 ns +0 NextTimeStep",
            "10 ns +0 p0 out: bye",
            "10 ns +0 LastKnownDelta",
            "10 ns +0 EndOfTimeStep",
            "10 ns +0 EndOfSim",
        ]
    );
}

/// A VHPI-style callback depositing at the last known delta creates a delta.
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
            "5 ns +1 s[0..1]=[4]",
            "5 ns +1 p0 Note: [4]"
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
    // The process at 0 waited for TIME'HIGH: fine. Waiting again overflows.
    let mut b = Builder::new();
    b.process("q", NONE, &[], p(|c| c.wait_for(Time::MAX)));
    let (mut sim, _) = build(b, Config::default());
    let err = sim.run_until(Time::MAX).unwrap_err();
    assert!(matches!(err.kind, ErrorKind::TimeOverflow { .. }), "{err}");
    assert_eq!(err.process, Some((ProcId(0), "q".into())));
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
