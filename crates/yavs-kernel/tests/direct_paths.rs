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
