//! `ToggleProcess` (a clock generator `x <= not x after T;`) behaves as the process it stands
//! for: same value changes, with the signal alone, with a second source, and with a delay that
//! quantizes to zero.

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

/// Value changes of a clock `c` up to `until`, made by the kernel's process (`native`) or by
/// the closure it replaces, with `other` as a second source (a resolved signal) when given.
fn clock(native: bool, init: &str, delay: Time, other: Option<&[(u64, StdULogic)]>) -> Vec<String> {
    let mut b = Builder::new();
    let init = init.parse::<LogicVec>().unwrap();
    let c = if other.is_some() {
        b.resolved_signal("c", SignalType::logic(1), &init, Resolution::StdLogic)
    } else {
        b.signal("c", SignalType::logic(1), &init)
    };
    let d = b.driver(c);
    let proc: Box<dyn Process> = if native {
        Box::new(ToggleProcess::new(c, 0, d, delay, DelayMode::INERTIAL))
    } else {
        Box::new(p(move |ctx| {
            let next = StdULogic::from_code_pub(ctx.elem_code(c, 0)?).not() as u8;
            ctx.assign_code(d, None, delay, next, DelayMode::INERTIAL)?;
            if !ctx.wait_same() {
                ctx.wait_on(c)?;
            }
            Ok(())
        }))
    };
    b.process_boxed("clk", NONE, &[d], proc);
    if let Some(steps) = other {
        let d2 = b.driver(c);
        // First 'Z' now, then each step: wait, then drive.
        let steps = steps.to_vec();
        let mut i = 0;
        b.process(
            "other",
            NONE,
            &[d2],
            p(move |ctx| {
                if i == 0 {
                    ctx.assign_code(
                        d2,
                        None,
                        Time::ZERO,
                        StdULogic::Z as u8,
                        DelayMode::INERTIAL,
                    )?;
                } else if let Some(&(_, v)) = steps.get(i - 1) {
                    ctx.assign_code(d2, None, Time::ZERO, v as u8, DelayMode::INERTIAL)?;
                }
                if let Some(&(t, _)) = steps.get(i) {
                    ctx.wait_for(ns(t))?;
                }
                i += 1;
                Ok(())
            }),
        );
    }
    let (mut sim, trace) = build(b, Config::default());
    sim.run_until(ns(100)).unwrap();
    trace.take()
}

fn same(init: &str, delay: Time, other: Option<&[(u64, StdULogic)]>) {
    let (a, b) = (
        clock(true, init, delay, other),
        clock(false, init, delay, other),
    );
    assert_eq!(a, b);
    if !matches!(init, "U" | "X") {
        assert!(a.len() > 2, "{a:?}");
    }
}

#[test]
fn a_lone_clock() {
    same("0", ns(5), None);
    same("1", ns(3), None);
    same("H", ns(4), None);
}

#[test]
fn an_unknown_clock_stays_unknown() {
    same("U", ns(5), None);
    same("X", ns(5), None);
}

#[test]
fn a_second_source_preempts() {
    // The other source fights the clock's value while it has a transaction pending.
    let other = [
        (6, StdULogic::One),
        (4, StdULogic::Z),
        (13, StdULogic::Zero),
        (4, StdULogic::Z),
    ];
    same("0", ns(5), Some(&other));
}

#[test]
fn a_delay_below_the_resolution_is_a_delta() {
    // 100 fs truncated to a 1 ps resolution is zero: the clock toggles in every delta, until
    // the delta limit ends the run on both sides alike.
    let run = |native: bool| {
        let mut b = Builder::new();
        let c = b.signal("c", SignalType::logic(1), &"0".parse::<LogicVec>().unwrap());
        let d = b.driver(c);
        let delay = Time::fs(100).unwrap();
        if native {
            b.process_boxed(
                "clk",
                NONE,
                &[d],
                Box::new(ToggleProcess::new(c, 0, d, delay, DelayMode::INERTIAL)),
            );
        } else {
            b.process(
                "clk",
                NONE,
                &[d],
                p(move |ctx| {
                    let next = StdULogic::from_code_pub(ctx.elem_code(c, 0)?).not() as u8;
                    ctx.assign_code(d, None, delay, next, DelayMode::INERTIAL)?;
                    if !ctx.wait_same() {
                        ctx.wait_on(c)?;
                    }
                    Ok(())
                }),
            );
        }
        let (mut sim, _) = build(
            b,
            Config {
                max_deltas: 50,
                resolution: Unit::Ps,
                ..Config::default()
            },
        );
        let e = sim.run_until(ns(10)).unwrap_err();
        format!("{e:?}")
    };
    assert_eq!(run(true), run(false));
}

/// A clock with readers (a counter on every event, a counter on rising edges), deposits, a force and a
/// release on it in the middle of the run: the kernel re-arms the generator
/// itself between those, and must resume it when something else changes the clock.
fn clock_with_ops(native: bool) -> Vec<String> {
    let mut b = Builder::new();
    let c = b.signal("c", SignalType::logic(1), &"0".parse::<LogicVec>().unwrap());
    let d = b.driver(c);
    let delay = ns(5);
    let proc: Box<dyn Process> = if native {
        Box::new(ToggleProcess::new(c, 0, d, delay, DelayMode::INERTIAL))
    } else {
        Box::new(p(move |ctx| {
            let next = StdULogic::from_code_pub(ctx.elem_code(c, 0)?).not() as u8;
            ctx.assign_code(d, None, delay, next, DelayMode::INERTIAL)?;
            if !ctx.wait_same() {
                ctx.wait_on(c)?;
            }
            Ok(())
        }))
    };
    b.process_boxed("clk", NONE, &[d], proc);
    for (name, rising) in [("any", false), ("rise", true)] {
        let n = b.signal(name, SignalType::bytes(1, 1), &[0]);
        let dn = b.driver(n);
        let pid = b.process(
            name,
            NONE,
            &[dn],
            p(move |ctx| {
                let k = ctx.value(n)?[0];
                ctx.assign(dn, None, &[(Time::ZERO, &[k + 1][..])], DelayMode::INERTIAL)?;
                if !ctx.wait_same() {
                    ctx.wait_on(c)?;
                }
                Ok(())
            }),
        );
        if rising {
            b.edge_trigger(
                pid,
                EdgeTrigger {
                    signal: c,
                    elem: 0,
                    rising: true,
                    legacy: false,
                    reset: None,
                },
            );
        }
    }
    let (mut sim, trace) = build(b, Config::default());
    let one = "1".parse::<LogicVec>().unwrap();
    let zero = "0".parse::<LogicVec>().unwrap();
    sim.run_until(ns(23)).unwrap();
    sim.deposit(c, &one).unwrap();
    sim.run_until(ns(47)).unwrap();
    sim.deposit(c, &zero).unwrap();
    sim.run_until(ns(71)).unwrap();
    sim.force(c, ForceMode::Driving, &one).unwrap();
    sim.run_until(ns(83)).unwrap();
    sim.release(c, ForceMode::Driving).unwrap();
    sim.run_until(ns(120)).unwrap();
    trace.take()
}

#[test]
fn forces_and_deposits_resume_a_rearmed_clock() {
    let (a, b) = (clock_with_ops(true), clock_with_ops(false));
    assert_eq!(a, b);
    assert!(a.len() > 30, "{a:?}");
}

/// Clocks that edge at the same times (4, 5, 5 and 6 ns half periods), a process waiting 10 ns
/// (its timeouts fall on edges), readers of every clock, and ops on one of them: the cycles with
/// several due entries are updated together and each generator re-arms itself.
fn clocks_together(native: bool) -> Vec<String> {
    let mut b = Builder::new();
    let one = "1".parse::<LogicVec>().unwrap();
    let zero = "0".parse::<LogicVec>().unwrap();
    let mut clocks = Vec::new();
    for (k, half) in [4, 5, 5, 6].into_iter().enumerate() {
        let c = b.signal(&format!("c{k}"), SignalType::logic(1), &zero);
        let d = b.driver(c);
        let delay = ns(half);
        let proc: Box<dyn Process> = if native {
            Box::new(ToggleProcess::new(c, 0, d, delay, DelayMode::INERTIAL))
        } else {
            Box::new(p(move |ctx| {
                let next = StdULogic::from_code_pub(ctx.elem_code(c, 0)?).not() as u8;
                ctx.assign_code(d, None, delay, next, DelayMode::INERTIAL)?;
                if !ctx.wait_same() {
                    ctx.wait_on(c)?;
                }
                Ok(())
            }))
        };
        b.process_boxed(&format!("clk{k}"), NONE, &[d], proc);
        let n = b.signal(&format!("n{k}"), SignalType::bytes(1, 1), &[0]);
        let dn = b.driver(n);
        let pid = b.process(
            &format!("rise{k}"),
            NONE,
            &[dn],
            p(move |ctx| {
                let v = ctx.value(n)?[0];
                ctx.assign(dn, None, &[(Time::ZERO, &[v + 1][..])], DelayMode::INERTIAL)?;
                if !ctx.wait_same() {
                    ctx.wait_on(c)?;
                }
                Ok(())
            }),
        );
        b.edge_trigger(
            pid,
            EdgeTrigger {
                signal: c,
                elem: 0,
                rising: true,
                legacy: false,
                reset: None,
            },
        );
        clocks.push(c);
    }
    let t = b.signal("t", SignalType::bytes(1, 1), &[0]);
    let dt = b.driver(t);
    b.process(
        "ticker",
        NONE,
        &[dt],
        p(move |ctx| {
            let v = ctx.value(t)?[0];
            ctx.assign(dt, None, &[(Time::ZERO, &[v + 1][..])], DelayMode::INERTIAL)?;
            ctx.wait_for(ns(10))
        }),
    );
    let (mut sim, trace) = build(b, Config::default());
    sim.run_until(ns(37)).unwrap();
    sim.deposit(clocks[1], &one).unwrap();
    sim.run_until(ns(61)).unwrap();
    sim.force(clocks[2], ForceMode::Driving, &one).unwrap();
    sim.run_until(ns(83)).unwrap();
    sim.release(clocks[2], ForceMode::Driving).unwrap();
    sim.run_until(ns(250)).unwrap();
    trace.take()
}

#[test]
fn clocks_that_edge_together() {
    let (a, b) = (clocks_together(true), clocks_together(false));
    assert_eq!(a, b);
    assert!(a.len() > 200, "{}", a.len());
}
