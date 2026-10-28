//! Thread-count and shuffle invariance of full event traces.

mod common;

use std::sync::{Arc, Mutex};

use common::*;
use yavs_kernel::*;
use yavs_logic::StdULogic;

const N: usize = 1000;
const WORD: SignalType = SignalType::bytes(4, 1);

fn word(v: &[u8]) -> u32 {
    u32::from_le_bytes(v.try_into().unwrap())
}

/// A clock, a ring of N cells each reading its neighbour on every rising
/// edge (mixed delta, inertial and transport delays), a resolved bus with
/// 16 drivers, and optionally serial processes sharing a variable.
fn design(with_serial: bool) -> Builder {
    let mut b = Builder::new();
    let clk = b.signal("clk", SignalType::bytes(1, 1), &[0]);
    let dclk = b.driver(clk);
    b.process(
        "clkgen",
        ProcFlags::default(),
        &[dclk],
        p(move |c| {
            let v = c.value(clk)?[0] ^ 1;
            c.assign(dclk, None, &[(ns(5), &[v])], DelayMode::INERTIAL)?;
            c.wait_for(ns(5))
        }),
    );
    let cells: Vec<SignalId> = (0..N)
        .map(|i| b.signal(&format!("cell{i}"), WORD, &(i as u32).to_le_bytes()))
        .collect();
    let or = Resolution::Func(Arc::new(|srcs: &[&[u8]], out: &mut [u8]| {
        for (k, o) in out.iter_mut().enumerate() {
            *o = srcs.iter().fold(0, |a, s| a | s[k]);
        }
        Ok(())
    }));
    let bus = b.resolved_signal("bus", SignalType::bytes(1, 4), &[0; 4], or);
    for (i, &cell) in cells.iter().enumerate() {
        let left = cells[(i + N - 1) % N];
        let d = b.driver(cell);
        let mut drivers = vec![d];
        let bus_drv = (i % 64 == 0).then(|| b.driver(bus));
        drivers.extend(bus_drv);
        b.process(
            &format!("top.cell{i}"),
            ProcFlags::default(),
            &drivers,
            p(move |c| {
                if c.event(clk)? && c.value(clk)? == [1] {
                    let v = word(c.value(cell)?)
                        .wrapping_mul(31)
                        .wrapping_add(word(c.value(left)?))
                        ^ i as u32;
                    let mode = if i % 5 == 0 {
                        DelayMode::Transport
                    } else {
                        DelayMode::INERTIAL
                    };
                    c.assign(d, None, &[(ns((i % 3) as u64), &v.to_le_bytes())], mode)?;
                    if v.is_multiple_of(61) {
                        c.report(Severity::Note, format!("cell{i}={v}"));
                    }
                    if let Some(bd) = bus_drv {
                        c.drive(bd, &v.to_le_bytes())?;
                    }
                }
                c.wait_on(clk)?;
                c.wait_on(left)
            }),
        );
    }
    // A 32-bit logic bus with 16 drivers, seen through an in-port, plus implicit signals.
    let lbus = b.resolved_signal(
        "lbus",
        SignalType::logic(32),
        &LogicVec::new(32, StdULogic::U),
        Resolution::StdLogic,
    );
    let mon = b.resolved_signal(
        "top.u.mon",
        SignalType::logic(32),
        &LogicVec::new(32, StdULogic::U),
        Resolution::StdLogic,
    );
    b.port(mon, lbus, PortMode::In, PortConv::default());
    let stable = b.stable_signal(clk, ns(3));
    b.delayed_signal(cells[7], ns(2));
    for k in 0..16 {
        let d = b.driver(lbus.slice(k * 2, 4.min(32 - k * 2)));
        let src = cells[k as usize * 37];
        b.process(
            &format!("top.ldrv{k}"),
            ProcFlags::default(),
            &[d],
            p(move |c| {
                let w = word(c.value(src)?);
                let n = 4.min(32 - k * 2) as usize;
                let v = LogicVec::from_fn(n, |i| match (w >> i) & 3 {
                    0 => StdULogic::Zero,
                    1 => StdULogic::One,
                    2 => StdULogic::Z,
                    _ => StdULogic::L,
                });
                c.drive(d, &v)?;
                c.wait_on(src)
            }),
        );
    }
    b.process(
        "top.u.watch",
        ProcFlags::default(),
        &[],
        p(move |c| {
            if c.event(mon)? && c.logic(stable)?.get(0) == StdULogic::Zero {
                c.report(Severity::Note, format!("mon={}", c.logic(mon)?));
            }
            c.wait_on(mon)
        }),
    );
    if with_serial {
        // Serial by flag (k < 3) or by `Process::serial` (the interpreter's way).
        let shared = Arc::new(Mutex::new(0u64));
        for k in 0..6 {
            let shared = shared.clone();
            b.process(
                &format!("top.serial{k}"),
                ProcFlags {
                    postponed: false,
                    serial: k < 3,
                    several_waits: false,
                    concurrent_call: false,
                },
                &[],
                Serial(p(move |c| {
                    let mut v = shared.lock().unwrap();
                    *v = v.wrapping_mul(7).wrapping_add(k);
                    c.report(Severity::Note, format!("shared={v}"));
                    c.wait_on(clk)
                })),
            );
        }
    }
    b
}

struct Serial<P>(P);

impl<P: Process> Process for Serial<P> {
    fn run(&mut self, ctx: &mut Ctx<'_>) -> Result<(), SimError> {
        self.0.run(ctx)
    }
    fn serial(&self) -> bool {
        true
    }
}

fn trace(with_serial: bool, threads: usize, shuffle: Option<u64>) -> Vec<String> {
    trace_with(with_serial, threads, shuffle, false)
}

/// `pool_all`: every batch of two or more processes runs on the pool (`Config::pool_all`).
fn trace_with(
    with_serial: bool,
    threads: usize,
    shuffle: Option<u64>,
    pool_all: bool,
) -> Vec<String> {
    let cfg = Config {
        threads,
        shuffle,
        pool_all,
        ..Config::default()
    };
    let (mut sim, t) = build(design(with_serial), cfg);
    assert_eq!(sim.run_until(ns(200)).unwrap(), RunStatus::Paused);
    t.take()
}

#[test]
fn threads_give_identical_traces() {
    let one = trace(true, 1, None);
    assert!(one.len() > 10 * N, "trace too small: {}", one.len());
    for pat in ["mon=", "lbus[", "top.u.mon[", "clk'stable", "cell7'delayed"] {
        assert!(one.iter().any(|l| l.contains(pat)), "no {pat} in the trace");
    }
    assert_eq!(one, trace(true, 8, None));
    // The pooled path commits as the inline one does, for batches of any size (#497).
    for threads in [2, 4, 8] {
        assert_eq!(
            one,
            trace_with(true, threads, None, true),
            "{threads} threads, all pooled"
        );
    }
    // A batch without serial processes: every process early-committed in the workers (#481).
    let plain = trace(false, 1, None);
    for threads in [2, 4, 8] {
        assert_eq!(
            plain,
            trace_with(false, threads, None, true),
            "{threads} threads, pieces"
        );
    }
}

#[test]
fn shuffle_gives_identical_traces_for_order_independent_designs() {
    let one = trace(false, 1, None);
    for seed in [1, 2, 0xdead_beef] {
        assert_eq!(one, trace(false, 1, Some(seed)), "seed {seed}, 1 thread");
        assert_eq!(one, trace(false, 8, Some(seed)), "seed {seed}, 8 threads");
        let pooled = trace_with(false, 4, Some(seed), true);
        assert_eq!(one, pooled, "seed {seed}, 4 threads, all pooled");
    }
}

#[test]
fn shuffle_exposes_order_dependent_serial_processes() {
    assert_ne!(trace(true, 1, None), trace(true, 1, Some(1)));
    // The serial processes of a pooled batch run in the shuffled order too, so a seed gives the
    // same result at any thread count.
    let one = trace(true, 1, Some(1));
    for threads in [2, 4] {
        assert_eq!(
            one,
            trace_with(true, threads, Some(1), true),
            "{threads} threads"
        );
    }
}

/// Resolution fails on 300 independent signals in one cycle (updated in parallel at 8
/// threads): the error names the lowest signal id at every thread count.
#[test]
fn parallel_update_reports_the_lowest_failing_signal() {
    let fails = Resolution::Func(Arc::new(|srcs: &[&[u8]], _: &mut [u8]| {
        if srcs.iter().any(|s| s[0] != 0) {
            Err("conflict".into())
        } else {
            Ok(())
        }
    }));
    for threads in [1, 8] {
        let mut b = Builder::new();
        for i in 0..300 {
            let r = b.resolved_signal(
                &format!("r{i}"),
                SignalType::bytes(1, 1),
                &[0],
                fails.clone(),
            );
            let d = b.driver(r);
            b.process(
                &format!("drv{i}"),
                ProcFlags::default(),
                &[d],
                p(move |c| {
                    c.drive(d, &[1])?;
                    c.wait_for(ns(10))
                }),
            );
        }
        let cfg = Config {
            threads,
            ..Config::default()
        };
        let (mut sim, _) = build(b, cfg);
        let err = sim.run_until(ns(1)).unwrap_err();
        assert_eq!(
            err.kind,
            ErrorKind::Resolution {
                signal: "r0".into(),
                message: "conflict".into()
            },
            "{threads} threads"
        );
    }
}

/// Observer wake-ups between and on clock edges, depositing and cancelling, give the same
/// trace at any thread count.
#[test]
fn observer_wake_ups_give_identical_traces() {
    struct Waker(Arc<Mutex<Vec<String>>>, Option<WakeId>);
    impl Observer for Waker {
        fn phase(&mut self, h: &mut Hook<'_>, phase: Phase) {
            let t = h.now();
            if phase == Phase::NextTimeStep && t.as_fs() % ns(10).as_fs() == 0 && t < ns(150) {
                h.wake_at(t.checked_add(ns(3)).unwrap()).unwrap();
                h.wake_at(t.checked_add(ns(5)).unwrap()).unwrap();
                if let Some(old) = self
                    .1
                    .replace(h.wake_at(t.checked_add(ns(15)).unwrap()).unwrap())
                {
                    h.cancel(old);
                }
            }
        }
        fn wake(&mut self, h: &mut Hook<'_>, id: WakeId) {
            let v = (h.now().as_fs() / ns(1).as_fs()) as u32;
            h.deposit(SignalId(1), &v.to_le_bytes()).unwrap();
            self.0
                .lock()
                .unwrap()
                .push(format!("{} +{} wake {id:?}", h.now(), h.delta()));
        }
    }
    let run = |threads| {
        let cfg = Config {
            threads,
            ..Config::default()
        };
        let (mut sim, t) = build(design(false), cfg);
        sim.subscribe(Box::new(Waker(t.lines.clone(), None)));
        assert_eq!(sim.run_until(ns(200)).unwrap(), RunStatus::Paused);
        t.take()
    };
    let one = run(1);
    assert!(one.iter().any(|l| l.contains("wake")), "no wake-up fired");
    assert!(one.iter().any(|l| l.starts_with("13 ns +0 cell0")));
    assert_eq!(one, run(4));
    assert_eq!(one, run(8));
}
