//! Direct transactions (a driver that is the only source of its elements of a plain logic
//! signal): zero-delay assignments kept by the process and written at commit, their
//! preemption by a later assignment of the same run, forces in the same cycle, and the clock
//! batch of resident processes with a several-wait process on the same clock.

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

const ONE: u8 = StdULogic::One as u8;
const ZERO: u8 = StdULogic::Zero as u8;

/// One run at 0 fs of a process that drives `s` (one `std_ulogic`, sole source) with `body`,
/// then waits forever; the value changes of `s` up to 10 ns.
fn one_run(
    body: impl Fn(&mut Ctx<'_>, DriverId) -> Result<(), SimError> + Send + 'static,
) -> Vec<String> {
    let mut b = Builder::new();
    let s = b.signal("s", SignalType::logic(1), &"U".parse::<LogicVec>().unwrap());
    let d = b.driver(s);
    let mut done = false;
    b.process(
        "p",
        NONE,
        &[d],
        p(move |c| {
            if !std::mem::replace(&mut done, true) {
                body(c, d)?;
            }
            Ok(())
        }),
    );
    let (mut sim, trace) = build(b, Config::default());
    sim.run_until(ns(10)).unwrap();
    trace.take()
}

#[test]
fn a_later_zero_delay_assignment_of_the_run_wins() {
    let got = one_run(|c, d| {
        c.assign_code(d, None, Time::ZERO, ONE, DelayMode::INERTIAL)?;
        c.assign_code(d, None, Time::ZERO, ZERO, DelayMode::INERTIAL)
    });
    assert_eq!(got, ["0 fs +1 s[0..1]=\"0\""]);
}

#[test]
fn a_delayed_assignment_preempts_the_direct_one_of_the_run() {
    // Inertial: the zero-delay '1' is in the rejection window of the '0' after 2 ns.
    let got = one_run(|c, d| {
        c.assign_code(d, None, Time::ZERO, ONE, DelayMode::INERTIAL)?;
        c.assign_code(d, None, ns(2), ZERO, DelayMode::INERTIAL)
    });
    assert_eq!(got, ["2 ns +0 s[0..1]=\"0\""]);
    // Transport keeps it.
    let got = one_run(|c, d| {
        c.assign_code(d, None, Time::ZERO, ONE, DelayMode::Transport)?;
        c.assign_code(d, None, ns(2), ZERO, DelayMode::Transport)
    });
    assert_eq!(got, ["0 fs +1 s[0..1]=\"1\"", "2 ns +0 s[0..1]=\"0\""]);
}

#[test]
fn a_force_in_the_cycle_of_a_direct_transaction_wins() {
    let got = one_run(|c, d| {
        c.assign_code(d, None, Time::ZERO, ONE, DelayMode::INERTIAL)?;
        let s = SignalId(0);
        c.force(s, ForceMode::Effective, &"0".parse::<LogicVec>().unwrap())
    });
    assert_eq!(got, ["0 fs +1 s[0..1]=\"0\""]);
}

/// 40 processes register a rising edge of a logic clock in order, a several-wait process
/// after them, and the clock's own process; the reports of one edge come in that order at
/// any thread count.
fn clock_batch(threads: usize) -> Vec<String> {
    let mut b = Builder::new();
    let clk = b.signal(
        "clk",
        SignalType::logic(1),
        &"0".parse::<LogicVec>().unwrap(),
    );
    let cd = b.driver(clk);
    let q = b.signal(
        "q",
        SignalType::logic(40),
        &LogicVec::new(40, StdULogic::Zero),
    );
    for i in 0..40u32 {
        let d = b.driver(q.slice(i, 1));
        let mut n = 0u32;
        b.process(
            &format!("r{i}"),
            NONE,
            &[d],
            p(move |c| {
                if c.edge(clk, true)? == Some(true) {
                    n += 1;
                    if i % 10 == 0 {
                        c.report(Severity::Note, format!("r{i} {n}"));
                    }
                    let v = if n % 2 == 1 { ONE } else { ZERO };
                    c.assign_code(
                        d,
                        Some(Range::new(i, 1)),
                        Time::ZERO,
                        v,
                        DelayMode::INERTIAL,
                    )?;
                }
                c.wait_on(clk)
            }),
        );
    }
    let several = ProcFlags {
        several_waits: true,
        ..NONE
    };
    let mut n = 0u32;
    b.process(
        "tb",
        several,
        &[],
        p(move |c| {
            if c.edge(clk, true)? == Some(true) {
                n += 1;
                c.report(Severity::Note, format!("tb {n}"));
            }
            c.wait_on(clk)
        }),
    );
    b.process(
        "clkgen",
        NONE,
        &[cd],
        p(move |c| {
            let v = if c.first_code(clk, false)?.unwrap() == ONE {
                ZERO
            } else {
                ONE
            };
            c.assign_code(cd, None, ns(5), v, DelayMode::INERTIAL)?;
            c.wait_for(ns(5))
        }),
    );
    let cfg = Config {
        threads,
        ..Config::default()
    };
    let (mut sim, trace) = build(b, cfg);
    sim.run_until(ns(60)).unwrap();
    let mut lines: Vec<String> = trace
        .take()
        .into_iter()
        .filter(|l| l.contains(" p"))
        .collect();
    lines.push(format!("final {}", sim.value(q).unwrap()));
    lines
}

#[test]
fn a_clock_batch_keeps_the_report_order() {
    let one = clock_batch(1);
    // Edges at 5, 15, 25, 35, 45, 55 ns: the plain processes in order, then the other one.
    assert_eq!(one.len(), 6 * 5 + 1, "{one:?}");
    assert_eq!(
        one[..5],
        [
            "5 ns +0 p0 Note: r0 1",
            "5 ns +0 p10 Note: r10 1",
            "5 ns +0 p20 Note: r20 1",
            "5 ns +0 p30 Note: r30 1",
            "5 ns +0 p40 Note: tb 1",
        ]
    );
    assert_eq!(one, clock_batch(4));
}

/// Direct assignments that change nothing: with `skip_unchanged` the commit writes nothing for
/// one element (a vector is written as before), yet every delta, phase and report is where it
/// was, also when a later assignment of the run preempts the unchanged one.
fn unchanged_direct(skip: bool) -> Vec<String> {
    let mut b = Builder::new();
    let lv = |s: &str| s.parse::<LogicVec>().unwrap();
    let (s, t, v) = (
        b.signal("s", SignalType::logic(1), &lv("1")),
        b.signal("t", SignalType::logic(1), &lv("0")),
        b.signal("v", SignalType::logic(4), &lv("0101")),
    );
    let (ds, dt, dv) = (b.driver(s), b.driver(t), b.driver(v));
    // The same value at 0 fs (an empty delta follows) and at 5 ns.
    b.process(
        "same",
        NONE,
        &[ds, dv],
        p(move |c| {
            c.assign_code(ds, None, Time::ZERO, ONE, DelayMode::INERTIAL)?;
            c.assign(dv, None, &[(Time::ZERO, &lv("0101"))], DelayMode::INERTIAL)?;
            if c.now() == Time::ZERO {
                c.wait_for(ns(5))?;
            }
            Ok(())
        }),
    );
    // The same value, then a different one after a delay that preempts it; at 4 ns the same
    // value, then a different one at once.
    b.process(
        "preempted",
        NONE,
        &[dt],
        p(move |c| {
            if c.now() == Time::ZERO {
                c.assign_code(dt, None, Time::ZERO, ZERO, DelayMode::INERTIAL)?;
                c.assign_code(dt, None, ns(2), ONE, DelayMode::INERTIAL)?;
                c.wait_for(ns(4))?;
            } else if c.now() == ns(4) {
                c.assign_code(dt, None, Time::ZERO, ONE, DelayMode::INERTIAL)?;
                c.assign_code(dt, None, Time::ZERO, ZERO, DelayMode::INERTIAL)?;
            }
            Ok(())
        }),
    );
    b.process(
        "watch",
        ProcFlags {
            postponed: true,
            ..NONE
        },
        &[],
        p(move |c| {
            c.report(Severity::Note, format!("t={:?}", c.logic(t)?.to_string()));
            c.wait_on(s)?;
            c.wait_on(t)?;
            c.wait_on(v)?;
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
fn unchanged_direct_assignments_keep_the_deltas() {
    let (off, on) = (unchanged_direct(false), unchanged_direct(true));
    assert!(off.iter().any(|l| l.contains("+1 NextCycle")), "{off:?}");
    assert_eq!(off, on);
}

/// Four direct drivers of one element and four of two elements (vector entries), assigning at
/// every ns, with forces of the driving value, releases and deposits in between; each process
/// reports its `'driving_value`. The direct commits write only the driving value
/// (`Signal::lag`), so the same run on signals with an identity resolution function (no direct
/// driver) must give the same trace.
fn forced_drivers(direct: bool) -> Vec<String> {
    let mut b = Builder::new();
    let lv = |s: &str| s.parse::<LogicVec>().unwrap();
    let same: Resolver = std::sync::Arc::new(|srcs: &[&[u8]], out: &mut [u8]| {
        out.copy_from_slice(srcs[0]);
        Ok(())
    });
    let mut sig = |name: &str, n: u32| {
        let (ty, init) = (
            SignalType::logic(n),
            LogicVec::new(n as usize, StdULogic::U),
        );
        if direct {
            b.signal(name, ty, &init)
        } else {
            b.resolved_signal(name, ty, &init, Resolution::Func(same.clone()))
        }
    };
    let (v, w) = (sig("v", 4), sig("w", 8));
    for i in 0..4u32 {
        let d = b.driver(v.slice(i, 1));
        let mut t = 0u32;
        b.process(
            &format!("p{i}"),
            NONE,
            &[d],
            p(move |c| {
                t += 1;
                if t > 1 {
                    let got = c.driving_value(d)?;
                    c.report(Severity::Note, format!("{got:?}"));
                }
                let code = [ONE, ZERO, StdULogic::X as u8][((t + i) % 3) as usize];
                let r = Some(Range::new(i, 1));
                c.assign_code(d, r, Time::ZERO, code, DelayMode::INERTIAL)?;
                c.wait_for(ns(1))
            }),
        );
        let dw = b.driver(w.slice(2 * i, 2));
        let mut t = 0u32;
        b.process(
            &format!("q{i}"),
            NONE,
            &[dw],
            p(move |c| {
                t += 1;
                if t > 1 {
                    let got = c.driving_value(dw)?;
                    c.report(Severity::Note, format!("{got:?}"));
                }
                let val = lv(["01", "1X", "00", "Z1"][((t + i) % 4) as usize]);
                c.assign(dw, None, &[(Time::ZERO, &val)], DelayMode::INERTIAL)?;
                c.wait_for(ns(1))
            }),
        );
    }
    let mut t = 0u32;
    b.process(
        "ctl",
        NONE,
        &[],
        p(move |c| {
            t += 1;
            match t - 1 {
                2 => {
                    c.force(v.slice(1, 2), ForceMode::Driving, &lv("01"))?;
                    c.force(w.slice(1, 4), ForceMode::Driving, &lv("0101"))?;
                }
                4 => {
                    c.release(v.slice(1, 1), ForceMode::Driving)?;
                    c.release(w.slice(1, 2), ForceMode::Driving)?;
                }
                5 => {
                    c.release(v.slice(2, 1), ForceMode::Driving)?;
                    c.release(w.slice(3, 2), ForceMode::Driving)?;
                }
                6 => {
                    c.deposit(v.slice(0, 2), &lv("ZZ"))?;
                    c.deposit(w.slice(5, 3), &lv("LLL"))?;
                }
                7 => c.force(v.slice(3, 1), ForceMode::Effective, &lv("H"))?,
                _ => {}
            }
            c.wait_for(ns(1))
        }),
    );
    let (mut sim, trace) = build(b, Config::default());
    sim.run_until(ns(10)).unwrap();
    let mut lines = trace.take();
    let (fv, fw) = (sim.value(v).unwrap(), sim.value(w).unwrap());
    lines.push(format!("final {fv} {fw}"));
    lines
}

#[test]
fn direct_drivers_keep_their_values_through_forces_and_deposits() {
    let direct = forced_drivers(true);
    assert!(direct.len() > 80, "{direct:?}");
    assert_eq!(direct, forced_drivers(false));
}

/// Random zero-delay and delayed assignments (scalar and vector) of processes that are the only
/// source of their elements, every nanosecond, with each process reporting its `'driving_value`.
/// `explicit` names the rejection limit of a delayed inertial assignment (the delay itself, the
/// same semantics), which sends it down the general path; the delayed one-element and vector
/// assignments to an idle driver otherwise wait as a quick transaction for their time.
fn random_delays(explicit: bool, seed: u64) -> Vec<String> {
    let mut b = Builder::new();
    let lv = |s: &str| s.parse::<LogicVec>().unwrap();
    let a = b.signal("a", SignalType::logic(1), &lv("0"));
    let v = b.signal("v", SignalType::logic(6), &lv("000000"));
    let w = b.signal("w", SignalType::logic(2), &lv("00"));
    let drivers = [
        b.driver(a),
        b.driver(v),
        b.driver(w.slice(0, 1)),
        b.driver(w.slice(1, 1)),
    ];
    for (k, d) in drivers.into_iter().enumerate() {
        let mut rng = seed ^ (k as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        b.process(
            &format!("p{k}"),
            NONE,
            &[d],
            p(move |c| {
                // One to three assignments per run, so a delayed one meets the run's own
                // zero-delay entries and earlier delayed transactions.
                for _ in 0..1 + rng % 3 {
                    rng = rng
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add(1442695040888963407);
                    let r = rng >> 33;
                    let (delay, bits) = (r % 5, (r >> 8) as u32);
                    let mode = if explicit && delay > 0 {
                        DelayMode::Inertial {
                            reject: Some(ns(delay)),
                        }
                    } else {
                        DelayMode::INERTIAL
                    };
                    if r.is_multiple_of(7) {
                        continue;
                    }
                    if k == 1 {
                        let planes: LogicVec = (0..6)
                            .map(|i| if bits >> i & 1 == 1 { '1' } else { '0' })
                            .collect::<String>()
                            .parse()
                            .unwrap();
                        c.assign(d, None, &[(ns(delay), &planes)], mode)?;
                    } else {
                        let code = if bits & 1 == 1 { ONE } else { ZERO };
                        c.assign_code(d, None, ns(delay), code, mode)?;
                    }
                }
                c.report(
                    Severity::Note,
                    format!("p{k} driving {}", c.driving_value(d)?),
                );
                c.wait_for(ns(1))
            }),
        );
    }
    let (mut sim, trace) = build(b, Config::default());
    sim.run_until(ns(300)).unwrap();
    trace.take()
}

#[test]
fn delayed_assignments_to_idle_drivers_match_the_general_path() {
    for seed in 0..8 {
        let (fast, general) = (random_delays(false, seed), random_delays(true, seed));
        assert!(fast.len() > 600, "{}", fast.len());
        assert_eq!(fast, general, "seed {seed}");
    }
}

/// Random zero-delay and delayed assignments of processes that are the only source of an
/// integer and of the elements of an enumeration array (byte signals), reporting the signals'
/// events, last values and last event times. `general` gives the signals an identity
/// resolution function (they are updated through the dirty ranges) and the delayed inertial
/// assignments an explicit rejection limit (the general assignment path).
fn random_bytes(general: bool, seed: u64) -> Vec<String> {
    let mut b = Builder::new();
    let same: Resolver = std::sync::Arc::new(|srcs: &[&[u8]], out: &mut [u8]| {
        out.copy_from_slice(srcs[0]);
        Ok(())
    });
    let mut sig = |name: &str, ty: SignalType, init: &[u8]| {
        if general {
            b.resolved_signal(name, ty, init, Resolution::Func(same.clone()))
        } else {
            b.signal(name, ty, init)
        }
    };
    let n = sig("n", SignalType::bytes(4, 1), &[0; 4]);
    let e = sig("e", SignalType::bytes(1, 3), &[0; 3]);
    let drivers = [
        b.driver(n),
        b.driver(e.slice(0, 1)),
        b.driver(e.slice(1, 1)),
    ];
    for (k, d) in drivers.into_iter().enumerate() {
        let mut rng = seed ^ (k as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let sref = if k == 0 {
            n.slice(0, 1)
        } else {
            e.slice(k as u32 - 1, 1)
        };
        b.process(
            &format!("p{k}"),
            NONE,
            &[d],
            p(move |c| {
                for _ in 0..1 + rng % 3 {
                    rng = rng
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add(1442695040888963407);
                    let r = rng >> 33;
                    let (delay, v) = (r % 5, (r >> 8) as u32 % 3);
                    let mode = if general && delay > 0 {
                        DelayMode::Inertial {
                            reject: Some(ns(delay)),
                        }
                    } else {
                        DelayMode::INERTIAL
                    };
                    if r.is_multiple_of(7) {
                        continue;
                    }
                    if k == 0 {
                        c.assign_bytes(d, None, ns(delay), &v.to_le_bytes(), mode)?;
                    } else {
                        c.assign_bytes(d, None, ns(delay), &[v as u8], mode)?;
                    }
                }
                c.report(
                    Severity::Note,
                    format!(
                        "p{k} {:?} ev={} last={:?} at {:?} active={}",
                        c.value(sref)?,
                        c.event(sref)?,
                        c.last_value(sref)?,
                        c.last_event(sref)?,
                        c.active(sref)?,
                    ),
                );
                c.wait_for(ns(1))
            }),
        );
    }
    let (mut sim, trace) = build(
        b,
        Config {
            skip_unchanged: true,
            ..Config::default()
        },
    );
    sim.run_until(ns(300)).unwrap();
    trace.take()
}

#[test]
fn byte_signal_assignments_match_the_general_path() {
    for seed in 0..8 {
        let (fast, general) = (random_bytes(false, seed), random_bytes(true, seed));
        assert!(fast.len() > 500, "{}", fast.len());
        assert_eq!(fast, general, "seed {seed}");
    }
}

/// Four direct drivers of one enumeration element and two of one integer, assigning at every ns
/// (some values repeat), with forces of the driving value, releases and deposits in between;
/// each process reports its `'driving_value`, and a postponed process what it sees at the end
/// of every time step. The direct commits write only the driving value (`Signal::lag`), so the
/// same run on signals with an identity resolution function (no direct driver) must give the
/// same trace.
fn forced_byte_drivers(direct: bool) -> Vec<String> {
    let mut b = Builder::new();
    let same: Resolver = std::sync::Arc::new(|srcs: &[&[u8]], out: &mut [u8]| {
        out.copy_from_slice(srcs[0]);
        Ok(())
    });
    let mut sig = |name: &str, es: u32, n: u32| {
        let (ty, init) = (SignalType::bytes(es, n), vec![0u8; (es * n) as usize]);
        if direct {
            b.signal(name, ty, &init)
        } else {
            b.resolved_signal(name, ty, &init, Resolution::Func(same.clone()))
        }
    };
    let (a, n) = (sig("a", 1, 4), sig("n", 4, 2));
    for i in 0..4u32 {
        let d = b.driver(a.slice(i, 1));
        let mut t = 0u32;
        b.process(
            &format!("p{i}"),
            NONE,
            &[d],
            p(move |c| {
                t += 1;
                if t > 1 {
                    let got = c.driving_value(d)?;
                    c.report(
                        Severity::Note,
                        format!("{got:?} ev={}", c.event(a.slice(i, 1))?),
                    );
                }
                let v = [1u8, 0, 2, 2, 1][((t + i) % 5) as usize];
                c.assign_bytes(d, None, Time::ZERO, &[v], DelayMode::INERTIAL)?;
                c.wait_for(ns(1))
            }),
        );
    }
    for i in 0..2u32 {
        let d = b.driver(n.slice(i, 1));
        let mut t = 0u32;
        b.process(
            &format!("q{i}"),
            NONE,
            &[d],
            p(move |c| {
                t += 1;
                if t > 1 {
                    let got = c.driving_value(d)?;
                    c.report(Severity::Note, format!("{got:?}"));
                }
                let v = [7u32, 7, 9, 100000, 7][((t + i) % 5) as usize];
                c.assign_bytes(d, None, Time::ZERO, &v.to_le_bytes(), DelayMode::INERTIAL)?;
                // A delayed one now and then, on the same driver.
                if t.is_multiple_of(4) {
                    c.assign_bytes(d, None, ns(2), &(v + 1).to_le_bytes(), DelayMode::INERTIAL)?;
                }
                c.wait_for(ns(1))
            }),
        );
    }
    let mut t = 0u32;
    b.process(
        "ctl",
        NONE,
        &[],
        p(move |c| {
            t += 1;
            match t - 1 {
                2 => {
                    c.force(a.slice(1, 2), ForceMode::Driving, &[2u8, 1][..])?;
                    c.force(n.slice(1, 1), ForceMode::Driving, &55u32.to_le_bytes()[..])?;
                }
                4 => c.release(a.slice(1, 1), ForceMode::Driving)?,
                5 => {
                    c.release(a.slice(2, 1), ForceMode::Driving)?;
                    c.release(n.slice(1, 1), ForceMode::Driving)?;
                }
                6 => {
                    c.deposit(a.slice(0, 2), &[2u8, 2][..])?;
                    c.deposit(n.slice(0, 1), &66u32.to_le_bytes()[..])?;
                }
                7 => c.force(a.slice(3, 1), ForceMode::Effective, &[1u8][..])?,
                _ => {}
            }
            c.wait_for(ns(1))
        }),
    );
    b.process(
        "watch",
        ProcFlags {
            postponed: true,
            ..NONE
        },
        &[],
        p(move |c| {
            c.report(
                Severity::Note,
                format!("{:?} {:?}", c.value(a)?, c.value(n)?),
            );
            c.wait_on(a)?;
            c.wait_on(n)?;
            c.wait_for(ns(1))
        }),
    );
    let (mut sim, trace) = build(b, Config::default());
    sim.run_until(ns(14)).unwrap();
    let mut lines = trace.take();
    let (fa, fn_) = (sim.value(a).unwrap(), sim.value(n).unwrap());
    lines.push(format!("final {fa:?} {fn_:?}"));
    lines
}

#[test]
fn byte_direct_drivers_keep_their_values_through_forces_and_deposits() {
    let direct = forced_byte_drivers(true);
    assert!(direct.len() > 80, "{direct:?}");
    assert_eq!(direct, forced_byte_drivers(false));
}

/// Direct byte assignments that change nothing: with `skip_unchanged` the commit writes nothing,
/// yet every delta, phase and report is where it was, also when a later assignment of the run
/// preempts the unchanged one.
fn unchanged_bytes(skip: bool) -> Vec<String> {
    let mut b = Builder::new();
    let (s, t) = (
        b.signal("s", SignalType::bytes(1, 1), &[1]),
        b.signal("t", SignalType::bytes(4, 1), &[5, 0, 0, 0]),
    );
    let (ds, dt) = (b.driver(s), b.driver(t));
    b.process(
        "same",
        NONE,
        &[ds],
        p(move |c| {
            c.assign_bytes(ds, None, Time::ZERO, &[1], DelayMode::INERTIAL)?;
            if c.now() == Time::ZERO {
                c.wait_for(ns(5))?;
            }
            Ok(())
        }),
    );
    b.process(
        "preempted",
        NONE,
        &[dt],
        p(move |c| {
            let five = 5u32.to_le_bytes();
            if c.now() == Time::ZERO {
                c.assign_bytes(dt, None, Time::ZERO, &five, DelayMode::INERTIAL)?;
                c.assign_bytes(dt, None, ns(2), &6u32.to_le_bytes(), DelayMode::INERTIAL)?;
                c.wait_for(ns(4))?;
            } else if c.now() == ns(4) {
                c.assign_bytes(
                    dt,
                    None,
                    Time::ZERO,
                    &6u32.to_le_bytes(),
                    DelayMode::INERTIAL,
                )?;
                c.assign_bytes(dt, None, Time::ZERO, &five, DelayMode::INERTIAL)?;
            }
            Ok(())
        }),
    );
    b.process(
        "watch",
        ProcFlags {
            postponed: true,
            ..NONE
        },
        &[],
        p(move |c| {
            c.report(Severity::Note, format!("t={:?}", c.value(t)?));
            c.wait_on(s)?;
            c.wait_on(t)?;
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
fn unchanged_direct_byte_assignments_keep_the_deltas() {
    let (off, on) = (unchanged_bytes(false), unchanged_bytes(true));
    assert!(off.iter().any(|l| l.contains("+1 NextCycle")), "{off:?}");
    assert_eq!(off, on);
}

/// Random zero-delay assignments of one process to one 100-element logic signal it alone drives:
/// single elements, runs of single elements in one call, vectors of up to 64 elements and a
/// vector of more than 64, in any order, with overlaps. The value the signal holds at each run is the one a model that applies the
/// assignments one after the other (the last wins) gives, whichever of the direct entry kinds
/// (`out`, `outv`) the assignments went to.
fn mixed_entries(skip: bool, extra: u32, seed: u64) -> Vec<String> {
    let mut b = Builder::new();
    let s = b.signal(
        "s",
        SignalType::logic(100),
        &LogicVec::new(100, StdULogic::Zero),
    );
    let d = b.driver(s);
    // Many drivers make the process keep marks of its entries (`ProcState::marks`).
    let pad = b.signal(
        "pad",
        SignalType::logic(extra.max(1)),
        &LogicVec::new(extra.max(1) as usize, StdULogic::Zero),
    );
    let mut owned = vec![d];
    owned.extend((0..extra).map(|i| b.driver(pad.slice(i, 1))));
    let bad = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let bad2 = bad.clone();
    let mut model = [ZERO; 100];
    let mut rng = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    let mut run = 0u32;
    b.process(
        "p",
        NONE,
        &owned,
        p(move |c| {
            run += 1;
            let got: String = (0..100)
                .map(|e| Ok(if c.elem_code(s, e)? == ONE { '1' } else { '0' }))
                .collect::<Result<_, SimError>>()?;
            let want: String = model
                .iter()
                .map(|&x| if x == ONE { '1' } else { '0' })
                .collect();
            if got != want {
                bad2.lock()
                    .unwrap()
                    .push(format!("run {run}: {got} != {want}"));
            }
            let mut next = || {
                rng = rng
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                rng >> 33
            };
            for _ in 0..2 + next() % 6 {
                let r = next();
                match r % 5 {
                    4 => {
                        // A run of single elements in one call (`assign_codes_now`, compiled
                        // code's kept drives), from a few elements so that repeats happen; the
                        // ones it does not take go one by one, as `drive_bits` does.
                        let at = (next() % 96) as u32;
                        let run: Vec<(u32, u8)> = (0..1 + next() % 8)
                            .map(|_| {
                                let e = at + (next() % 4) as u32;
                                (e, if next() & 1 == 1 { ONE } else { ZERO })
                            })
                            .collect();
                        let taken = c.assign_codes_now(d, run.len(), |k| run[k]);
                        for &(e, v) in &run[taken..] {
                            c.assign_code(
                                d,
                                Some(Range::new(e, 1)),
                                Time::ZERO,
                                v,
                                DelayMode::INERTIAL,
                            )?;
                        }
                        for &(e, v) in &run {
                            model[e as usize] = v;
                        }
                    }
                    0 | 1 => {
                        // One element.
                        let e = (next() % 100) as u32;
                        let v = if next() & 1 == 1 { ONE } else { ZERO };
                        c.assign_code(
                            d,
                            Some(Range::new(e, 1)),
                            Time::ZERO,
                            v,
                            DelayMode::INERTIAL,
                        )?;
                        model[e as usize] = v;
                    }
                    k => {
                        // A vector: up to 64 elements, or (k == 3) more.
                        let len = if k == 3 {
                            65 + (next() % 36) as u32
                        } else {
                            1 + (next() % 64) as u32
                        };
                        let off = (next() % u64::from(101 - len)) as u32;
                        let bits = next() ^ next() << 20;
                        let mut blocks = vec![0u8; (len as usize).div_ceil(64) * 32];
                        for j in 0..len {
                            let one = (bits.rotate_left(j) & 1 == 1) as u64;
                            model[(off + j) as usize] = if one == 1 { ONE } else { ZERO };
                            // Codes of '0' and '1' in the four planes.
                            let code = u64::from(if one == 1 { ONE } else { ZERO });
                            for plane in 0..4 {
                                let w = (j as usize / 64) * 32 + plane * 8;
                                let mut x =
                                    u64::from_le_bytes(blocks[w..w + 8].try_into().unwrap());
                                x |= (code >> plane & 1) << (j % 64);
                                blocks[w..w + 8].copy_from_slice(&x.to_le_bytes());
                            }
                        }
                        c.assign_blocks(
                            d,
                            Some(Range::new(off, len)),
                            Time::ZERO,
                            &blocks,
                            DelayMode::INERTIAL,
                        )?;
                    }
                }
            }
            c.wait_for(ns(1))
        }),
    );
    let (mut sim, _trace) = build(
        b,
        Config {
            skip_unchanged: skip,
            ..Config::default()
        },
    );
    sim.run_until(ns(400)).unwrap();
    bad.lock().unwrap().clone()
}

#[test]
fn mixed_direct_entry_kinds_apply_in_assignment_order() {
    for seed in 0..16 {
        for (skip, extra) in [(false, 0), (true, 0), (false, 6), (true, 6)] {
            let bad = mixed_entries(skip, extra, seed);
            assert!(
                bad.is_empty(),
                "seed {seed} skip {skip} extra {extra}: {:?}",
                &bad[..bad.len().min(3)]
            );
        }
    }
}

/// A run of single elements in one call (`assign_codes_now`) that assigns an element twice, the
/// second time the code the signal holds: the first entry is pending, so the second is no skip
/// and the element ends with the second code.
#[test]
fn a_batched_element_assigned_twice_keeps_the_last_code() {
    let mut b = Builder::new();
    let s = b.signal(
        "s",
        SignalType::logic(8),
        &LogicVec::new(8, StdULogic::Zero),
    );
    let d = b.driver(s);
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen2 = seen.clone();
    let mut run = 0;
    b.process(
        "p",
        NONE,
        &[d],
        p(move |c| {
            run += 1;
            match run {
                1 => {
                    let r = [(5u32, ONE), (6u32, ZERO), (5u32, ZERO)];
                    assert_eq!(c.assign_codes_now(d, 3, |k| r[k]), 3);
                }
                2 => seen2.lock().unwrap().push(c.elem_code(s, 5)?),
                _ => {}
            }
            c.wait_for(ns(1))
        }),
    );
    let (mut sim, _t) = build(
        b,
        Config {
            skip_unchanged: true,
            ..Config::default()
        },
    );
    sim.run_until(ns(5)).unwrap();
    assert_eq!(*seen.lock().unwrap(), vec![ZERO]);
}

/// A batch of single elements that covers a run (`assign_codes_now` makes one vector entry of
/// it), in any order and with an element twice, then one more element of the run on its own:
/// the last assignment of each element wins, as one at a time.
#[test]
fn a_batched_run_of_elements_is_one_entry_with_the_last_codes() {
    let mut b = Builder::new();
    let s = b.signal(
        "s",
        SignalType::logic(24),
        &LogicVec::new(24, StdULogic::Zero),
    );
    let d = b.driver(s);
    let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let seen2 = seen.clone();
    let mut run = 0u32;
    b.process(
        "p",
        NONE,
        &[d],
        p(move |c| {
            run += 1;
            let got: String = (0..24)
                .map(|e| Ok(if c.elem_code(s, e)? == ONE { '1' } else { '0' }))
                .collect::<Result<_, SimError>>()?;
            seen2.lock().unwrap().push(got);
            // Elements 4 to 19 backwards, element 7 again with the other value, then 9 alone.
            let v = |e: u32| {
                if (e + run).is_multiple_of(3) {
                    ONE
                } else {
                    ZERO
                }
            };
            let mut r: Vec<(u32, u8)> = (4..20).rev().map(|e| (e, v(e))).collect();
            r.push((7, if v(7) == ONE { ZERO } else { ONE }));
            assert_eq!(c.assign_codes_now(d, r.len(), |k| r[k]), r.len());
            c.assign_code(
                d,
                Some(Range::new(9, 1)),
                Time::ZERO,
                ONE,
                DelayMode::INERTIAL,
            )?;
            c.wait_for(ns(1))
        }),
    );
    let (mut sim, _t) = build(
        b,
        Config {
            skip_unchanged: true,
            ..Config::default()
        },
    );
    sim.run_until(ns(4)).unwrap();
    // Two vector entries an activation (the run, then element 9), no one-element entry.
    let st = sim.stats();
    assert_eq!((st.direct_codes, st.direct_planes), (0, 10));
    let want = |run: u32| -> String {
        (0..24u32)
            .map(|e| match e {
                9 => '1',
                7 if (7 + run).is_multiple_of(3) => '0',
                7 => '1',
                4..20 if (e + run).is_multiple_of(3) => '1',
                _ => '0',
            })
            .collect()
    };
    let seen = seen.lock().unwrap();
    assert_eq!(seen[0], "0".repeat(24));
    for (k, got) in seen.iter().enumerate().skip(1) {
        assert_eq!(*got, want(k as u32), "activation {}", k + 1);
    }
}

/// Parts of a vector assigned next to each other in one run (a loop over the bytes of a vector)
/// join the run's last vector entry, up or down, across a plane word, up to 64 elements; a
/// scalar next to it joins it too, and a part inside it is an entry of its own that wins.
#[test]
fn adjacent_parts_join_one_vector_entry() {
    let mut b = Builder::new();
    let zeros = |n: usize| LogicVec::new(n, StdULogic::Zero);
    let s = b.signal("s", SignalType::logic(100), &zeros(100));
    let t = b.signal("t", SignalType::logic(80), &zeros(80));
    let (ds, dt) = (b.driver(s), b.driver(t));
    let bad = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let bad2 = bad.clone();
    let mut run = 0u32;
    let v = |e: u32, run: u32| (e * 7 + run).is_multiple_of(3);
    let code = |one: bool| if one { ONE } else { ZERO };
    b.process(
        "p",
        NONE,
        &[ds, dt],
        p(move |c| {
            // What the last activation assigned.
            if run > 0 {
                for e in 0..100 {
                    let want = match e {
                        70 => !v(70, run),
                        24..48 | 60..85 => v(e, run),
                        _ => false,
                    };
                    if c.elem_code(s, e)? != code(want) {
                        bad2.lock().unwrap().push(format!("run {run}: s({e})"));
                    }
                }
                for e in 0..80 {
                    if c.elem_code(t, e)? != code(v(e, run)) {
                        bad2.lock().unwrap().push(format!("run {run}: t({e})"));
                    }
                }
            }
            run += 1;
            let part = |c: &mut Ctx<'_>, d: DriverId, off: u32, len: u32| {
                let mut planes = [0u64; 4];
                for j in 0..len {
                    let k = code(v(off + j, run));
                    for (p, w) in planes.iter_mut().enumerate() {
                        *w |= u64::from(k >> p & 1) << j;
                    }
                }
                assert!(c.assign_planes_now(d, Some(Range::new(off, len)), planes));
            };
            let one = |c: &mut Ctx<'_>, e: u32, k: u8| {
                c.assign_code(
                    ds,
                    Some(Range::new(e, 1)),
                    Time::ZERO,
                    k,
                    DelayMode::INERTIAL,
                )
            };
            // Up across the plane word at 64, then a scalar after it: one entry of 60..85.
            for off in [60, 68, 76] {
                part(c, ds, off, 8);
            }
            one(c, 84, code(v(84, run)))?;
            // Inside that entry: one of its own, the later value.
            one(c, 70, code(!v(70, run)))?;
            // Down: one entry of 24..48.
            for off in [40, 32, 24] {
                part(c, ds, off, 8);
            }
            // Ten bytes up: 64 elements, then 16.
            for off in (0..80).step_by(8) {
                part(c, dt, off, 8);
            }
            c.wait_for(ns(1))
        }),
    );
    let (mut sim, _t) = build(b, Config::default());
    sim.run_until(ns(4)).unwrap();
    assert_eq!(*bad.lock().unwrap(), Vec::<String>::new());
    // Five activations of five vector entries.
    let st = sim.stats();
    assert_eq!((st.direct_codes, st.direct_planes), (0, 25));
}

/// A scalar and a vector direct driver whose values change at every activation for a while
/// (their skip tests miss, so they stop asking it, `ProcState::same`), then hold for longer
/// than the tests are left out, then change again. The changes and the delta cycles are those
/// of the run that never skips.
fn changing_then_held(skip: bool) -> Vec<String> {
    let mut b = Builder::new();
    let lv = |s: &str| s.parse::<LogicVec>().unwrap();
    let (s, v) = (
        b.signal("s", SignalType::logic(1), &lv("0")),
        b.signal("v", SignalType::logic(4), &lv("0000")),
    );
    let (ds, dv) = (b.driver(s), b.driver(v));
    let mut k = 0u32;
    b.process(
        "regs",
        NONE,
        &[ds, dv],
        p(move |c| {
            k += 1;
            let moving = !(20..320).contains(&k);
            let code = if moving && k % 2 == 1 { ONE } else { ZERO };
            let vec = if moving {
                ["0110", "1001", "0011"][k as usize % 3]
            } else {
                "1111"
            };
            c.assign_code(ds, None, Time::ZERO, code, DelayMode::INERTIAL)?;
            c.assign(dv, None, &[(Time::ZERO, &lv(vec))], DelayMode::INERTIAL)?;
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
    sim.run_until(ns(340)).unwrap();
    trace.take()
}

#[test]
fn drivers_that_stop_asking_the_skip_test_keep_values_and_deltas() {
    let (off, on) = (changing_then_held(false), changing_then_held(true));
    assert!(
        off.iter().any(|l| l == "320 ns +1 s[0..1]=\"1\""),
        "{off:?}"
    );
    assert_eq!(off, on);
}

/// Processes that drive signals with an in port (not direct: a scalar, a vector and an integer)
/// one to four times a run with values the signals often hold, at zero delay and later: the
/// assignments the skip test drops (`skip`) leave phantoms that later ones of the run meet.
/// Reports the driving and effective values; returns the trace and `Stats::general_assigns`.
fn repeated_drives(skip: bool, seed: u64) -> (Vec<String>, u64) {
    let mut b = Builder::new();
    let lv = |s: &str| s.parse::<LogicVec>().unwrap();
    let a = b.signal("a", SignalType::logic(1), &lv("0"));
    let v = b.signal("v", SignalType::logic(3), &lv("000"));
    let n = b.signal("n", SignalType::bytes(4, 1), &[0; 4]);
    let fa = b.signal("fa", SignalType::logic(1), &lv("0"));
    let fv = b.signal("fv", SignalType::logic(3), &lv("000"));
    let f_n = b.signal("fn", SignalType::bytes(4, 1), &[0; 4]);
    for (f, s) in [(fa, a), (fv, v), (f_n, n)] {
        b.port(f, s, PortMode::In, PortConv::default());
    }
    let drivers = [b.driver(a), b.driver(v), b.driver(n)];
    for (k, d) in drivers.into_iter().enumerate() {
        let mut rng = seed ^ (k as u64 + 1).wrapping_mul(0x9E37_79B9_7F4A_7C15);
        let sref = [a, v, n][k];
        b.process(
            &format!("p{k}"),
            NONE,
            &[d],
            p(move |c| {
                for _ in 0..1 + rng % 4 {
                    rng = rng
                        .wrapping_mul(6364136223846793005)
                        .wrapping_add(1442695040888963407);
                    let r = rng >> 33;
                    // Mostly zero delay, mostly the value the signal holds.
                    let delay = if r.is_multiple_of(5) {
                        ns(r % 3)
                    } else {
                        Time::ZERO
                    };
                    let x = u32::from((r >> 8).is_multiple_of(6));
                    match k {
                        0 => {
                            let code = if x == 1 { ONE } else { ZERO };
                            c.assign_code(d, None, delay, code, DelayMode::INERTIAL)?;
                        }
                        1 => {
                            let val = lv(if x == 1 { "101" } else { "000" });
                            c.assign(d, None, &[(delay, &val)], DelayMode::INERTIAL)?;
                        }
                        _ => {
                            c.assign_bytes(d, None, delay, &x.to_le_bytes(), DelayMode::INERTIAL)?
                        }
                    }
                }
                let val = if k == 2 {
                    format!("{:?}", c.value(sref)?)
                } else {
                    c.logic(sref)?.to_string()
                };
                c.report(
                    Severity::Note,
                    format!(
                        "p{k} {val} driving {} active={}",
                        c.driving_value(d)?,
                        c.active(sref)?,
                    ),
                );
                c.wait_for(ns(1))
            }),
        );
    }
    let (mut sim, trace) = build(
        b,
        Config {
            skip_unchanged: skip,
            ..Config::default()
        },
    );
    sim.run_until(ns(200)).unwrap();
    (trace.take(), sim.stats().general_assigns)
}

#[test]
fn repeated_unchanged_drives_of_a_run_keep_values_and_deltas() {
    for seed in 0..8 {
        let ((skip, g_skip), (plain, g_plain)) =
            (repeated_drives(true, seed), repeated_drives(false, seed));
        assert!(skip.len() > 400, "{}", skip.len());
        assert_eq!(skip, plain, "seed {seed}");
        // The repeats of a skipped assignment stay on the fast paths.
        assert!(
            g_skip * 10 < g_plain * 8,
            "seed {seed}: {g_skip} vs {g_plain}"
        );
    }
}

/// Random assignments of one process to a 140-element part of a 150-element logic signal (across
/// plane words): single elements with any code, batches of single elements with holes and repeats
/// (`assign_codes_now`), vectors of up to 64 elements across a word boundary, wider vectors, and
/// delayed assignments after and before zero-delay ones of the same run. The process reports its
/// `'driving_value`, a second process waiting on the signal its value, `'event`, `'last_value`,
/// `'last_event` (and `'active` when nothing allows skipping). `direct` false gives the signal an
/// identity resolution function (no direct driver: the general paths), which must give the same.
fn direct_twin(direct: bool, skip: bool, seed: u64) -> Vec<String> {
    let mut b = Builder::new();
    let init = LogicVec::new(150, StdULogic::Zero);
    let s = if direct {
        b.signal("s", SignalType::logic(150), &init)
    } else {
        let same: Resolver = std::sync::Arc::new(|srcs: &[&[u8]], out: &mut [u8]| {
            out.copy_from_slice(srcs[0]);
            Ok(())
        });
        b.resolved_signal("s", SignalType::logic(150), &init, Resolution::Func(same))
    };
    let d = b.driver(s.slice(5, 140));
    let mut rng = seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1;
    b.process(
        "p",
        NONE,
        &[d],
        p(move |c| {
            let mut next = || {
                rng = rng
                    .wrapping_mul(6364136223846793005)
                    .wrapping_add(1442695040888963407);
                rng >> 33
            };
            let mut delayed = false;
            for _ in 0..1 + next() % 7 {
                let r = next();
                let code = (next() % 9) as u8;
                // Zero delay mostly; a delayed one sends the later ones of the run to the
                // general path, which must order them after the staged ones.
                let delay = if r.is_multiple_of(11) {
                    ns(1 + next() % 3)
                } else {
                    Time::ZERO
                };
                delayed |= delay != Time::ZERO;
                match r % 4 {
                    0 => {
                        let e = 5 + (next() % 140) as u32;
                        let t = Some(Range::new(e, 1));
                        c.assign_code(d, t, delay, code, DelayMode::INERTIAL)?;
                    }
                    1 if delay == Time::ZERO => {
                        let at = 5 + (next() % 70) as u32;
                        let run: Vec<(u32, u8)> = (0..1 + next() % 10)
                            .map(|_| (at + (next() % 70) as u32, (next() % 9) as u8))
                            .collect();
                        let taken = if delayed {
                            0
                        } else {
                            c.assign_codes_now(d, run.len(), |k| run[k])
                        };
                        for &(e, v) in &run[taken..] {
                            let t = Some(Range::new(e, 1));
                            c.assign_code(d, t, Time::ZERO, v, DelayMode::INERTIAL)?;
                        }
                    }
                    _ => {
                        let len = if r % 8 == 2 {
                            65 + (next() % 76) as u32
                        } else {
                            1 + (next() % 64) as u32
                        };
                        let off = 5 + (next() % u64::from(141 - len)) as u32;
                        let v: LogicVec = (0..len)
                            .map(|_| "UX01ZWLH-".as_bytes()[(next() % 9) as usize] as char)
                            .collect::<String>()
                            .parse()
                            .unwrap();
                        let t = Some(Range::new(off, len));
                        c.assign(d, t, &[(delay, &v)], DelayMode::INERTIAL)?;
                    }
                }
            }
            c.report(Severity::Note, format!("drv {}", c.driving_value(d)?));
            c.wait_for(ns(1))
        }),
    );
    b.process(
        "q",
        NONE,
        &[],
        p(move |c| {
            let mut line = format!(
                "{} ev {} lv {:?} le {:?}",
                c.logic(s)?,
                c.event(s)?,
                c.last_value(s)?,
                c.last_event(s)?
            );
            if !skip {
                line += &format!(" act {}", c.active(s)?);
            }
            c.report(Severity::Note, line);
            c.wait_on(s)
        }),
    );
    let (mut sim, trace) = build(
        b,
        Config {
            skip_unchanged: skip,
            ..Config::default()
        },
    );
    sim.run_until(ns(200)).unwrap();
    // The reports only: the order of an observer's changed ranges within one cycle is not fixed.
    trace
        .take()
        .into_iter()
        .filter(|l| l.contains("Note:"))
        .collect()
}

#[test]
fn direct_drives_match_the_general_path() {
    for seed in 0..12 {
        for skip in [false, true] {
            let direct = direct_twin(true, skip, seed);
            assert!(direct.len() > 300, "{}", direct.len());
            assert_eq!(
                direct,
                direct_twin(false, skip, seed),
                "seed {seed} skip {skip}"
            );
        }
    }
}
