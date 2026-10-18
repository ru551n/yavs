//! Thread-count and shuffle invariance of full event traces.

mod common;

use std::sync::{Arc, Mutex};

use common::*;
use rsim_kernel::*;

const N: usize = 1000;
const WORD: SignalType = SignalType {
    elem_size: 4,
    len: 1,
};

fn word(v: &[u8]) -> u32 {
    u32::from_le_bytes(v.try_into().unwrap())
}

/// A clock, a ring of N cells each reading its neighbour on every rising
/// edge (mixed delta, inertial and transport delays), a resolved bus with
/// 16 drivers, and optionally serial processes sharing a variable.
fn design(with_serial: bool) -> Builder {
    let mut b = Builder::new();
    let clk = b.signal(
        "clk",
        SignalType {
            elem_size: 1,
            len: 1,
        },
        &[0],
    );
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
    let or: Resolver = Arc::new(|srcs: &[&[u8]], out: &mut [u8]| {
        for (k, o) in out.iter_mut().enumerate() {
            *o = srcs.iter().fold(0, |a, s| a | s[k]);
        }
        Ok(())
    });
    let bus = b.resolved_signal(
        "bus",
        SignalType {
            elem_size: 1,
            len: 4,
        },
        &[0; 4],
        or,
    );
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
    if with_serial {
        let shared = Arc::new(Mutex::new(0u64));
        for k in 0..5 {
            let shared = shared.clone();
            b.process(
                &format!("top.serial{k}"),
                ProcFlags {
                    postponed: false,
                    serial: true,
                },
                &[],
                p(move |c| {
                    let mut v = shared.lock().unwrap();
                    *v = v.wrapping_mul(7).wrapping_add(k);
                    c.report(Severity::Note, format!("shared={v}"));
                    c.wait_on(clk)
                }),
            );
        }
    }
    b
}

fn trace(with_serial: bool, threads: usize, shuffle: Option<u64>) -> Vec<String> {
    let cfg = Config {
        threads,
        shuffle,
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
    assert_eq!(one, trace(true, 8, None));
}

#[test]
fn shuffle_gives_identical_traces_for_order_independent_designs() {
    let one = trace(false, 1, None);
    for seed in [1, 2, 0xdead_beef] {
        assert_eq!(one, trace(false, 1, Some(seed)), "seed {seed}, 1 thread");
        assert_eq!(one, trace(false, 8, Some(seed)), "seed {seed}, 8 threads");
    }
}

#[test]
fn shuffle_exposes_order_dependent_serial_processes() {
    assert_ne!(trace(true, 1, None), trace(true, 1, Some(1)));
}
