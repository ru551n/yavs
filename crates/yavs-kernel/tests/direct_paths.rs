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
