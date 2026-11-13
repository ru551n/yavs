//! A signal declared `real_valued` has an event only when its value changes as an IEEE double:
//! `0.0` to `-0.0` and back is a transaction, not an event (#1145). A signal of the same size
//! that is not real-valued keeps comparing bytes.

mod common;

use common::*;
use yavs_kernel::*;

const NONE: ProcFlags = ProcFlags {
    postponed: false,
    serial: false,
    several_waits: false,
    concurrent_call: false,
};

fn bits(x: f64) -> [u8; 8] {
    x.to_le_bytes()
}

/// `f` (real-valued), `g` (real-valued, two elements, delayed drives) and `i` (8-byte integer,
/// bytes compared) are driven through 0.0, -0.0, 0.0 and 1.0 (the integer one through the same
/// bit patterns). Returns the value changes the observer saw.
fn run(skip: bool) -> Vec<String> {
    let mut b = Builder::new();
    let f = b.signal("f", SignalType::bytes(8, 1), &bits(0.0));
    let g = b.signal(
        "g",
        SignalType::bytes(8, 2),
        &[bits(0.0), bits(0.0)].concat(),
    );
    let i = b.signal("i", SignalType::bytes(8, 1), &bits(0.0));
    b.real_valued(f);
    b.real_valued(g);
    let (df, dg, di) = (b.driver(f), b.driver(g), b.driver(i));
    let steps = [-0.0, 0.0, 1.0];
    b.process(
        "drive",
        NONE,
        &[df, dg, di],
        p(move |c| {
            if let Some(k) = (0..steps.len()).find(|&k| c.now() == ns(k as u64)) {
                let v = bits(steps[k]);
                c.assign_bytes(df, None, Time::ZERO, &v, DelayMode::INERTIAL)?;
                c.assign_bytes(di, None, Time::ZERO, &v, DelayMode::INERTIAL)?;
                let w = [v, bits(0.0)].concat();
                c.assign_bytes(dg, None, ns(1), &w, DelayMode::Transport)?;
            }
            c.wait_for(ns(1))
        }),
    );
    let cfg = Config {
        skip_unchanged: skip,
        ..Config::default()
    };
    let (mut sim, trace) = build(b, cfg);
    sim.run_until(ns(8)).unwrap();
    trace.take()
}

#[test]
fn zero_signs_are_no_event_on_a_real_valued_signal() {
    for skip in [false, true] {
        let lines = run(skip);
        let of = |n: &str| lines.iter().filter(|l| l.contains(n)).count();
        // f: only the change to 1.0; i: every change of bytes; g: its first element, delayed
        // by 1 ns, goes -0.0, 0.0, 1.0 and only the last is a change.
        assert_eq!(of(" f["), 1, "{lines:?}");
        assert_eq!(of(" i["), 3, "{lines:?}");
        assert_eq!(of(" g["), 1, "{lines:?}");
    }
}

/// With no event the signal keeps its value (`cur` changes on events only, which the record
/// images, the waves and compiled code's compare rely on): a real-valued signal driven with
/// `-0.0` still holds `0.0`; a signal compared by bytes holds `-0.0`. The drive was a
/// transaction all the same (the driver holds it).
#[test]
fn a_zero_of_the_other_sign_leaves_the_value() {
    let mut b = Builder::new();
    let f = b.signal("f", SignalType::bytes(8, 1), &bits(0.0));
    let i = b.signal("i", SignalType::bytes(8, 1), &bits(0.0));
    b.real_valued(f);
    let (df, di) = (b.driver(f), b.driver(i));
    b.process(
        "drive",
        NONE,
        &[df, di],
        p(move |c| {
            if c.now() == Time::ZERO {
                c.assign_bytes(df, None, ns(1), &bits(-0.0), DelayMode::INERTIAL)?;
                c.assign_bytes(di, None, ns(1), &bits(-0.0), DelayMode::INERTIAL)?;
            }
            c.wait_for(ns(5))
        }),
    );
    let (mut sim, _) = build(b, Config::default());
    sim.run_until(ns(3)).unwrap();
    assert_eq!(sim.value(f).unwrap(), Value::Bytes(bits(0.0).to_vec()));
    assert_eq!(sim.value(i).unwrap(), Value::Bytes(bits(-0.0).to_vec()));
}
