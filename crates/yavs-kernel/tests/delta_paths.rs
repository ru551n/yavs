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
/// source, a vector driver, a block pass of many elements and a deposit. A signal that
/// `Builder::no_codes` names keeps none, nor does one of more than 64 elements with a wide source
/// (a bus): its elements are read from the planes.
#[test]
fn element_codes_follow_every_kind_of_update() {
    for (n, none) in [3u32, 64, 70, 200]
        .into_iter()
        .flat_map(|n| [(n, false), (n, true)])
    {
        let mut b = Builder::new();
        let clk = logic_clock(&mut b);
        let s = b.signal(
            "s",
            SignalType::logic(n),
            &LogicVec::new(n as usize, StdULogic::Zero),
        );
        if none {
            b.no_codes(s);
        }
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
                assert_eq!(
                    view.map(|v| v.1),
                    (!none && n <= 64).then_some(n as usize),
                    "{n} elements"
                );
                assert!(c.code_view(clk).is_some_and(|v| v.1 == 1));
                // Whole plane blocks of codes can be read: compiled code reads groups of eight.
                assert_eq!(
                    c.code_cap(s),
                    if none || n > 64 {
                        0
                    } else {
                        (n as usize).next_multiple_of(64)
                    },
                    "{n} elements"
                );
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

/// The value process `k` drives onto its `len` elements at its `edge`th rising clock edge.
fn pattern(k: u32, edge: u32, len: u32) -> LogicVec {
    LogicVec::from_fn(len as usize, |i| {
        StdULogic::from_code_pub(((i as u32 + k * 3 + edge * (k + 1)) % 9) as u8)
    })
}

/// The plane blocks of `v` as `Ctx::assign_blocks` takes them.
fn blocks_of(v: &LogicVec) -> Vec<u8> {
    v.repr()
        .blocks()
        .iter()
        .flat_map(|b| b.iter().flat_map(|w| w.to_le_bytes()))
        .collect()
}

/// A vector driven by several processes with one slice each, each assigning a whole slice (at
/// most 64 elements, some across a plane word boundary, some through `assign_blocks`) at every
/// rising edge. The slices are zero-delay assignments to idle drivers, kept as plane words.
fn slices(order: impl Iterator<Item = usize>, cfg: Config) -> (Vec<String>, String) {
    const SLICES: [(u32, u32); 5] = [(0, 16), (16, 1), (17, 47), (64, 8), (72, 28)];
    let mut b = Builder::new();
    let clk = logic_clock(&mut b);
    let s = b.signal(
        "s",
        SignalType::logic(100),
        &LogicVec::new(100, StdULogic::U),
    );
    for k in order {
        let (off, len) = SLICES[k];
        let target = s.slice(off, len);
        let d = b.driver(target);
        let mut edges = 0u32;
        b.process(
            &format!("e{k}"),
            NONE,
            &[d],
            p(move |c| {
                if c.event(clk)? && c.first_code(clk, false)? == Some(StdULogic::One as u8) {
                    edges += 1;
                    // Repeat a value now and then: an unchanged assignment is skipped.
                    let v = pattern(k as u32, edges / 2, len);
                    if k % 2 == 0 {
                        c.assign(d, None, &[(Time::ZERO, &v)], DelayMode::INERTIAL)?;
                    } else {
                        c.assign_blocks(d, None, Time::ZERO, &blocks_of(&v), DelayMode::INERTIAL)?;
                    }
                }
                c.wait_on(clk)?;
                Ok(())
            }),
        );
    }
    let (mut sim, trace) = build(b, cfg);
    sim.run_until(ns(100)).unwrap();
    let mut lines = trace.take();
    lines.sort();
    (lines, sim.value(s).unwrap().to_string())
}

#[test]
fn vector_slices_are_assigned_whole_in_any_order_and_thread_count() {
    let (lines, last) = slices(0..5, Config::default());
    // 10 rising edges, the last value of each slice at the end.
    let mut want = LogicVec::new(100, StdULogic::U);
    for (k, (off, len)) in [(0u32, 16u32), (16, 1), (17, 47), (64, 8), (72, 28)]
        .into_iter()
        .enumerate()
    {
        let v = pattern(k as u32, 5, len);
        for i in 0..len {
            want.set((off + i) as usize, v.get(i as usize));
        }
    }
    assert_eq!(last, Value::Logic(want).to_string());
    assert!(lines.len() > 20);
    assert_eq!(
        slices((0..5).rev(), Config::default()),
        (lines.clone(), last.clone())
    );
    let four = Config {
        threads: 4,
        ..Config::default()
    };
    assert_eq!(slices(0..5, four), (lines, last));
}

#[test]
fn vector_assignment_then_a_later_one_keeps_both() {
    let mut b = Builder::new();
    let s = b.signal("s", SignalType::logic(4), &lv("UUUU"));
    let d = b.driver(s);
    b.process(
        "p",
        NONE,
        &[d],
        p(move |c| {
            // The first stays a plain zero-delay assignment until the second one schedules.
            c.assign(d, None, &[(Time::ZERO, &lv("0101"))], DelayMode::Transport)?;
            c.assign(d, None, &[(ns(2), &lv("1111"))], DelayMode::Transport)?;
            c.wait_for(ns(100))
        }),
    );
    let (mut sim, trace) = build(b, Config::default());
    sim.run_until(ns(5)).unwrap();
    assert_eq!(
        trace.take(),
        [
            "0 fs +1 s[0..4]=\"0101\"",
            "2 ns +0 s[1..2]=\"1\"",
            "2 ns +0 s[3..4]=\"1\""
        ]
    );
}

#[test]
fn assign_blocks_longer_than_a_word_and_wrong_sizes() {
    let mut b = Builder::new();
    let s = b.signal("s", SignalType::logic(70), &LogicVec::new(70, StdULogic::U));
    let d = b.driver(s);
    let v = LogicVec::from_fn(70, |i| StdULogic::from_code_pub((i % 9) as u8));
    let bytes = blocks_of(&v);
    b.process(
        "p",
        NONE,
        &[d],
        p(move |c| {
            assert!(
                c.assign_blocks(d, None, Time::ZERO, &bytes[..32], DelayMode::INERTIAL)
                    .is_err()
            );
            c.assign_blocks(d, None, Time::ZERO, &bytes, DelayMode::INERTIAL)?;
            c.wait_for(ns(100))
        }),
    );
    let (mut sim, _) = build(b, Config::default());
    sim.run_until(ns(5)).unwrap();
    assert_eq!(sim.value(s).unwrap(), Value::Logic(v));
}

/// Wide registers (130 and 70 elements, whole and partly assigned through `assign_blocks` and
/// `assign`) driven by one process each, every nanosecond, with forces, releases and deposits in
/// between and each process reporting its `'driving_value`. A plain signal takes the wide
/// assignments as one plane-word entry per 64 elements; with an identity resolution function
/// (no direct driver) the same run takes the general path and must give the same trace.
fn wide_registers(direct: bool, cfg: Config) -> (Vec<String>, Stats) {
    wide_registers_with(direct, false, cfg)
}

/// `wide_registers`, with the bus lean (`Builder::lean_events`) and three processes that wait on
/// the whole bus, on a part inside one block and on a part across two blocks, each reporting
/// what woke it.
fn wide_registers_with(direct: bool, lean: bool, cfg: Config) -> (Vec<String>, Stats) {
    wide_registers_run(direct, lean, lean, false, cfg)
}

/// `wide_registers_with` with the watchers apart from `lean`, and with only the reports
/// observed (`reports_only`: nobody asks for value changes, so a wide signal lists its changes
/// per cell, `Config::cells`).
fn wide_registers_run(
    direct: bool,
    lean: bool,
    watch: bool,
    reports_only: bool,
    cfg: Config,
) -> (Vec<String>, Stats) {
    let mut b = Builder::new();
    let same: Resolver = Arc::new(|srcs: &[&[u8]], out: &mut [u8]| {
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
    let w = sig("w", 200);
    if lean {
        b.lean_events(w);
    }
    for (k, (off, len)) in [(0u32, 130u32), (130, 70)].into_iter().enumerate() {
        let d = b.driver(w.slice(off, len));
        let mut t = 0u32;
        b.process(
            &format!("p{k}"),
            NONE,
            &[d],
            p(move |c| {
                t += 1;
                if t > 1 {
                    c.report(Severity::Note, format!("{}", c.driving_value(d)?));
                }
                // Whole, then a part now and then, then a delayed whole one (the general path).
                let (r, n) = match t % 4 {
                    1 => (None, len),
                    2 => (Some(Range::new(off + 3, len - 9)), len - 9),
                    3 => (Some(Range::new(off, len)), len),
                    _ => (Some(Range::new(off + 64, len - 64)), len - 64),
                };
                let v = pattern(k as u32, t, n);
                let delay = if t.is_multiple_of(7) {
                    ns(1)
                } else {
                    Time::ZERO
                };
                if (t + k as u32).is_multiple_of(3) {
                    c.assign(d, r, &[(delay, &v)], DelayMode::INERTIAL)?;
                } else {
                    c.assign_blocks(d, r, delay, &blocks_of(&v), DelayMode::INERTIAL)?;
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
            let ones = |n: u32| LogicVec::new(n as usize, StdULogic::One);
            match t - 1 {
                2 => c.force(w.slice(60, 80), ForceMode::Driving, &ones(80))?,
                4 => c.release(w.slice(60, 80), ForceMode::Driving)?,
                5 => c.deposit(w.slice(120, 30), &ones(30))?,
                6 => c.force(w.slice(0, 200), ForceMode::Effective, &ones(200))?,
                8 => c.release(w.slice(0, 200), ForceMode::Effective)?,
                _ => {}
            }
            c.wait_for(ns(1))
        }),
    );
    if watch {
        for (name, off, len) in [
            ("all", 0, 200),
            ("inner", 70, 10),
            ("across", 120, 20),
            ("edge", 64, 1),
            ("wide", 3, 190),
        ] {
            let r = w.slice(off, len);
            b.process(
                name,
                NONE,
                &[],
                p(move |c| {
                    let e = c.event(r)?;
                    c.report(Severity::Note, format!("{name} woke, event={e}"));
                    c.wait_on(r)
                }),
            );
        }
    }
    let (mut sim, trace) = if reports_only {
        let mut sim = b.build(cfg).unwrap();
        let t = Trace::default();
        sim.subscribe(Box::new(ReportsOnly(t.clone())));
        (sim, t)
    } else {
        build(b, cfg)
    };
    sim.run_until(ns(40)).unwrap();
    let mut lines = trace.take();
    lines.push(format!("final {}", sim.value(w).unwrap()));
    (lines, sim.stats())
}

/// `Trace` for the reports only: it does not ask for value changes.
struct ReportsOnly(Trace);

impl Observer for ReportsOnly {
    fn report(&mut self, h: &mut Hook<'_>, r: &Report) {
        self.0.report(h, r);
    }

    fn wants_value_changes(&self) -> bool {
        false
    }
}

#[test]
fn wide_registers_are_direct_entries_per_plane_word() {
    let (direct, st) = wide_registers(true, Config::default());
    let (general, st_general) = wide_registers(false, Config::default());
    assert!(direct.len() > 100, "{}", direct.len());
    // The ranges of one cycle's changes may be listed in another order (a deposit and direct
    // entries of a cycle are finished in different passes): the same set, and the reports in
    // the same order.
    let reports = |l: &[String]| {
        l.iter()
            .filter(|x| x.contains(" Note"))
            .cloned()
            .collect::<Vec<_>>()
    };
    let sorted = |l: &[String]| {
        let mut l = l.to_vec();
        l.sort();
        l
    };
    assert_eq!(reports(&direct), reports(&general));
    assert_eq!(sorted(&direct), sorted(&general));
    assert!(st.direct_planes > 40, "{st:?}");
    assert_eq!(st_general.direct_planes, 0);
    let four = Config {
        threads: 4,
        ..Config::default()
    };
    assert_eq!(wide_registers(true, four).0, direct);
}

/// A lean bus (one event record per block) driven through plane-word entries wakes the same
/// processes at the same times and lists the same changed ranges as one with every element's
/// history, whole-signal waiters and waiters of parts alike.
#[test]
fn a_lean_wide_bus_wakes_and_changes_like_one_with_full_history() {
    let (full, _) = wide_registers_with(true, false, Config::default());
    // Nothing reads the activity (`skip_unchanged`): the lean bus is finished a pass for all its words.
    let quiet = Config {
        skip_unchanged: true,
        ..Config::default()
    };
    let (lean, st) = wide_registers_with(true, true, quiet.clone());
    assert_eq!(st.lean_signals, 1);
    assert_eq!(wide_registers_with(true, true, Config::default()).0, lean);
    let woke = lean.iter().filter(|l| l.contains(" woke")).count();
    assert!(woke > 20, "{woke}");
    // The full-history run has no watchers: compare the ranges and values only.
    let rest = |l: &[String]| {
        l.iter()
            .filter(|x| !x.contains(" woke"))
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(rest(&lean), full);
    let four = Config {
        threads: 4,
        ..quiet
    };
    assert_eq!(wide_registers_with(true, true, four).0, lean);
}

/// A wide bus that processes wait on parts of lists its changes per cell between the waited
/// ranges, not per run: the same processes wake at the same times, with the value changes
/// observed (exact runs) or not, on both update paths, lean or not, at 1 and 4 threads.
#[test]
fn a_wide_bus_wakes_the_same_per_cell_as_per_run() {
    for (direct, lean) in [(true, true), (true, false), (false, true), (false, false)] {
        let exact = Config {
            cells: false,
            ..Config::default()
        };
        let (want, _) = wide_registers_run(direct, lean, true, true, exact);
        let woke = want.iter().filter(|l| l.contains(" woke")).count();
        assert!(woke > 40, "direct={direct} lean={lean}: {woke}");
        for threads in [1, 4] {
            let cfg = Config {
                threads,
                ..Config::default()
            };
            let (got, _) = wide_registers_run(direct, lean, true, true, cfg.clone());
            let at = got.iter().zip(&want).position(|(a, b)| a != b);
            assert_eq!(
                at.map(|i| (&got[i], &want[i])),
                None,
                "direct={direct} lean={lean} threads={threads}"
            );
            assert_eq!(got.len(), want.len());
            // With value changes observed the runs are exact.
            let (seen, _) = wide_registers_run(direct, lean, true, false, cfg);
            let wokes = |l: &[String]| l.iter().filter(|x| x.contains(" woke")).count();
            assert_eq!(wokes(&seen), woke, "direct={direct} lean={lean}");
        }
    }
}

/// A force over two plane words of a wide signal in a cycle where a direct entry finishes only
/// the first: the force's events in the second word are still made (they were lost when the
/// update of the first word's range took the force's whole event range, #789). `commit` makes
/// the entry; the force alone is the reference.
fn force_across_a_committed_word(commit: bool) -> Vec<String> {
    let mut b = Builder::new();
    // '1' from element 128 on: the force changes nothing there.
    let init = LogicVec::from_fn(150, |i| {
        if i < 128 {
            StdULogic::Zero
        } else {
            StdULogic::One
        }
    });
    let w = b.signal("w", SignalType::logic(150), &init);
    let d = b.driver(w.slice(64, 64));
    let mut t = 0u32;
    b.process(
        "p",
        NONE,
        &[d],
        p(move |c| {
            t += 1;
            if commit && t == 3 {
                let v = LogicVec::new(64, StdULogic::H);
                c.assign_blocks(d, None, Time::ZERO, &blocks_of(&v), DelayMode::INERTIAL)?;
            }
            c.wait_for(ns(1))
        }),
    );
    b.process(
        "ctl",
        NONE,
        &[],
        p(move |c| {
            if c.now() == ns(2) {
                let ones = LogicVec::new(40, StdULogic::One);
                c.force(w.slice(100, 40), ForceMode::Effective, &ones)?;
            }
            c.wait_for(ns(1))
        }),
    );
    let e = w.slice(135, 1);
    b.process(
        "e135",
        NONE,
        &[],
        p(move |c| {
            c.report(Severity::Note, format!("e135 event={}", c.event(e)?));
            c.wait_on(e)
        }),
    );
    let (mut sim, trace) = build(b, Config::default());
    sim.run_until(ns(5)).unwrap();
    trace
        .take()
        .into_iter()
        .filter(|l| l.contains("e135"))
        .collect()
}

#[test]
fn a_force_keeps_its_events_beyond_a_committed_word() {
    let (with, alone) = (
        force_across_a_committed_word(true),
        force_across_a_committed_word(false),
    );
    assert_eq!(with, alone);
    assert!(with.iter().any(|l| l.contains("event=true")), "{with:?}");
}
