//! The short paths of a delta cycle: the wake list of resident waiters, the pending
//! transaction of a one-element logic assignment, the one-element finish and the gathered
//! driving values of a vector with one driver per element.

mod common;

use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};

use common::*;
use yavs_kernel::*;
use yavs_logic::StdULogic;

const NONE: ProcFlags = ProcFlags {
    postponed: false,
    serial: false,
    several_waits: false,
    concurrent_call: false,
};

fn lv(s: &str) -> LogicVec {
    s.parse().unwrap()
}

/// A byte clock with a 10 ns period.
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
fn resident_waiters_follow_a_process_that_changes_its_wait() {
    // 40 processes wait for the clock every time, so the clock keeps a wake list; one of them
    // stops waiting for it after its third event and the list must not wake it again.
    for threads in [1, 4] {
        let mut b = Builder::new();
        let clk = clock(&mut b);
        let other = b.signal("other", BYTE, &[0]);
        let counts: Vec<Arc<AtomicU32>> = (0..40).map(|_| Arc::new(AtomicU32::new(0))).collect();
        for (i, n) in counts.iter().enumerate() {
            let n = n.clone();
            b.process(
                &format!("p{i}"),
                NONE,
                &[],
                p(move |c| {
                    let seen = n.fetch_add(1, Ordering::SeqCst);
                    if i == 5 && seen >= 3 {
                        c.wait_on(other)?;
                    } else {
                        c.wait_on(clk)?;
                    }
                    Ok(())
                }),
            );
        }
        let cfg = Config {
            threads,
            ..Config::default()
        };
        let (mut sim, _) = build(b, cfg);
        sim.run_until(ns(100)).unwrap();
        // 20 clock events in 100 ns, and the initial run.
        for (i, n) in counts.iter().enumerate() {
            let want = if i == 5 { 4 } else { 21 };
            assert_eq!(
                n.load(Ordering::SeqCst),
                want,
                "process {i}, {threads} threads"
            );
        }
    }
}

#[test]
fn a_process_joining_the_wake_list_late_is_woken() {
    let mut b = Builder::new();
    let clk = clock(&mut b);
    let counts: Vec<Arc<AtomicU32>> = (0..40).map(|_| Arc::new(AtomicU32::new(0))).collect();
    for (i, n) in counts.iter().enumerate() {
        let n = n.clone();
        b.process(
            &format!("p{i}"),
            NONE,
            &[],
            p(move |c| {
                let seen = n.fetch_add(1, Ordering::SeqCst);
                // The last one starts to wait for the clock only after 8 events.
                if i == 39 && seen < 8 {
                    c.wait_for(ns(10))?;
                } else {
                    c.wait_on(clk)?;
                }
                Ok(())
            }),
        );
    }
    let (mut sim, _) = build(b, Config::default());
    sim.run_until(ns(200)).unwrap();
    // 40 events: 8 timeouts (10 ns each, the clock has run for 80 ns), then one per event.
    let late = counts[39].load(Ordering::SeqCst);
    assert!(late > 9 + 10, "late joiner ran {late} times");
    for n in &counts[..39] {
        assert_eq!(n.load(Ordering::SeqCst), 41);
    }
}

/// A logic clock with a 10 ns period.
fn logic_clock(b: &mut Builder) -> SignalId {
    let clk = b.signal("clk", SignalType::logic(1), &lv("0"));
    let d = b.driver(clk);
    b.process(
        "clkgen",
        NONE,
        &[d],
        p(move |c| {
            let v = if c.first_code(clk, false)?.unwrap() == StdULogic::One as u8 {
                StdULogic::Zero
            } else {
                StdULogic::One
            };
            c.assign_code(d, None, ns(5), v as u8, DelayMode::INERTIAL)?;
            c.wait_for(ns(5))
        }),
    );
    clk
}

/// A vector of `n` logic elements, one process and driver per element, created in the given
/// order. Each process inverts its element (1 -> 0 -> 1 ...) on a rising clock edge, and
/// element `k` is held for `k % 3` extra cycles by not changing on some edges.
fn vector(n: u32, order: impl Iterator<Item = u32>, cfg: Config) -> Vec<String> {
    let mut b = Builder::new();
    let clk = logic_clock(&mut b);
    let s = b.signal(
        "s",
        SignalType::logic(n),
        &LogicVec::new(n as usize, StdULogic::Zero),
    );
    for k in order {
        let target = s.slice(k, 1);
        let d = b.driver(target);
        let mut edges = 0u32;
        b.process(
            &format!("e{k}"),
            NONE,
            &[d],
            p(move |c| {
                if c.event(clk)? && c.first_code(clk, false)? == Some(StdULogic::One as u8) {
                    edges += 1;
                    if edges.is_multiple_of(k % 3 + 1) {
                        let cur = c.first_code(target, false)?.unwrap();
                        let new = if cur == StdULogic::One as u8 {
                            StdULogic::Zero
                        } else {
                            StdULogic::One
                        };
                        c.assign_code(d, None, Time::ZERO, new as u8, DelayMode::INERTIAL)?;
                    }
                }
                c.wait_on(clk)?;
                Ok(())
            }),
        );
    }
    let (mut sim, trace) = build(b, cfg);
    sim.run_until(ns(100)).unwrap();
    // The order of the runs of one cycle follows the driver numbering; the set does not.
    let mut lines = trace.take();
    lines.sort();
    lines.push(format!("final {}", sim.value(s).unwrap()));
    lines
}

#[test]
fn vector_with_a_driver_per_element_is_the_same_in_any_creation_order() {
    for n in [3, 64, 70, 200] {
        let up = vector(n, 0..n, Config::default());
        let down = vector(n, (0..n).rev(), Config::default());
        assert_eq!(up, down, "{n} elements");
        let four = vector(
            n,
            (0..n).rev(),
            Config {
                threads: 4,
                ..Config::default()
            },
        );
        assert_eq!(up, four, "{n} elements, 4 threads");
        // 10 rising edges: element k flipped 10 / (k % 3 + 1) times.
        let want: String = (0..n)
            .rev()
            .map(|k| {
                if (10 / (k % 3 + 1)) % 2 == 1 {
                    '1'
                } else {
                    '0'
                }
            })
            .collect();
        assert_eq!(
            up.last().unwrap(),
            &format!("final \"{want}\""),
            "{n} elements"
        );
    }
}

#[test]
fn second_assignment_of_an_activation_replaces_the_first() {
    for (second_delay, want) in [
        // '1' then '0' in the same activation, both now: nothing changes.
        (Time::ZERO, Vec::<String>::new()),
        // '1' now, then '0' after 5 ns (transport keeps both).
        (ns(5), Vec::new()),
    ] {
        let mut b = Builder::new();
        let s = b.signal("s", SignalType::logic(1), &lv("0"));
        let d = b.driver(s);
        let mut first = true;
        b.process(
            "w",
            NONE,
            &[d],
            p(move |c| {
                if first {
                    first = false;
                    // The initial run: both assignments before the first cycle.
                    c.assign_code(
                        d,
                        None,
                        Time::ZERO,
                        StdULogic::One as u8,
                        DelayMode::Transport,
                    )?;
                    c.assign_code(
                        d,
                        None,
                        second_delay,
                        StdULogic::Zero as u8,
                        DelayMode::Transport,
                    )?;
                    c.wait_for(ns(100))?;
                } else {
                    c.wait_for(ns(100))?;
                }
                Ok(())
            }),
        );
        let (mut sim, trace) = build(b, Config::default());
        sim.run_until(ns(50)).unwrap();
        let got = trace.take();
        if second_delay == Time::ZERO {
            // Inertial/transport with the same time: the later assignment wins.
            assert_eq!(got, want, "zero delay");
        } else {
            assert_eq!(got.len(), 2, "{got:?}");
            assert!(
                got[0].ends_with("s[0..1]=\"1\"") && got[1].ends_with("s[0..1]=\"0\""),
                "{got:?}"
            );
        }
    }
}

/// The byte codes of a logic signal (`Ctx::elem_code`, `Ctx::code_view`) agree with the planes
/// after every kind of update: one-element finishes of drivers that are an element's only
/// source, a vector driver, a block pass of many elements and a deposit.
#[test]
fn element_codes_follow_every_kind_of_update() {
    for n in [3u32, 64, 70, 200] {
        let mut b = Builder::new();
        let clk = logic_clock(&mut b);
        let s = b.signal(
            "s",
            SignalType::logic(n),
            &LogicVec::new(n as usize, StdULogic::Zero),
        );
        // Elements 0 and 1: one driver each (changes finished at once); the rest: a vector driver.
        for k in 0..2.min(n) {
            let target = s.slice(k, 1);
            let d = b.driver(target);
            let mut edges = 0u32;
            b.process(
                &format!("e{k}"),
                NONE,
                &[d],
                p(move |c| {
                    if c.event(clk)? && c.first_code(clk, false)? == Some(StdULogic::One as u8) {
                        edges += 1;
                        let code = [StdULogic::One, StdULogic::X, StdULogic::Zero, StdULogic::H]
                            [(edges + k) as usize % 4];
                        c.assign_code(d, None, Time::ZERO, code as u8, DelayMode::INERTIAL)?;
                    }
                    c.wait_on(clk)
                }),
            );
        }
        if n > 2 {
            let rest = s.slice(2, n - 2);
            let d = b.driver(rest);
            let mut edges = 0usize;
            b.process(
                "vector",
                NONE,
                &[d],
                p(move |c| {
                    if c.event(clk)? && c.first_code(clk, false)? == Some(StdULogic::One as u8) {
                        edges += 1;
                        let all = [StdULogic::L, StdULogic::W, StdULogic::Z, StdULogic::One];
                        let mut v = LogicVec::new((n - 2) as usize, StdULogic::Zero);
                        for i in 0..(n - 2) as usize {
                            v.set(i, all[(i * 7 + edges) % 4]);
                        }
                        c.assign(d, None, &[(Time::ZERO, &v)], DelayMode::INERTIAL)?;
                    }
                    c.wait_on(clk)
                }),
            );
        }
        let checks = Arc::new(AtomicU32::new(0));
        let seen = checks.clone();
        b.process(
            "check",
            NONE,
            &[],
            p(move |c| {
                let view = c.code_view(s);
                assert_eq!(view.map(|v| v.1), Some(n as usize), "{n} elements");
                assert!(c.code_view(clk).is_some_and(|v| v.1 == 1));
                let whole = c.logic(s)?;
                for e in 0..n {
                    let want = whole.get(e as usize) as u8;
                    assert_eq!(c.elem_code(s, e).unwrap(), want, "{n} elements, {e}");
                    assert_eq!(c.first_code(s.slice(e, 1), false).unwrap(), Some(want));
                }
                assert!(c.elem_code(s, n).is_err());
                seen.fetch_add(1, Ordering::SeqCst);
                c.wait_on(clk)
            }),
        );
        let (mut sim, _) = build(b, Config::default());
        sim.run_until(ns(300)).unwrap();
        assert!(checks.load(Ordering::SeqCst) > 30, "{n} elements");
    }
}
