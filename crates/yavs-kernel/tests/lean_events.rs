//! Lean event bookkeeping of wide signals (`Builder::lean_events`, #519): a signal that keeps
//! only each block's last event answers `'event`, `'last_event` and `'last_value` of whole
//! blocks and `'last_value` of an element like one that keeps every element's, and refuses
//! (an internal error) the history of part of a block.

mod common;

use std::sync::{Arc, Mutex};

use common::*;
use yavs_kernel::*;
use yavs_logic::StdULogic;

const NONE: ProcFlags = ProcFlags {
    postponed: false,
    serial: false,
    several_waits: false,
    concurrent_call: false,
};

/// A 200 element bus `v` with a process that flips a different set of elements every
/// nanosecond (words 0 and 2 every time, word 1 on every third, one element of word 3 on every
/// fifth) and a process that records the answers of the attributes; `lean` selects the
/// bookkeeping of `v`.
fn run(lean: bool, partial: bool) -> (Result<(), SimError>, Vec<String>) {
    let mut b = Builder::new();
    let v = b.signal(
        "v",
        SignalType::logic(200),
        &LogicVec::new(200, StdULogic::Zero),
    );
    if lean {
        b.lean_events(v);
    }
    let d = b.driver(v);
    let mut n = 0u64;
    b.process(
        "stim",
        NONE,
        &[d],
        p(move |c| {
            n += 1;
            let mut x = LogicVec::new(200, StdULogic::Zero);
            for e in 0..200 {
                let on = match e / 64 {
                    0 | 2 => !(e as u64 + n).is_multiple_of(3),
                    1 => n.is_multiple_of(3) && e % 2 == 0,
                    _ => n.is_multiple_of(5) && e == 130 + n as usize % 60,
                };
                x.set(e, if on { StdULogic::One } else { StdULogic::Zero });
            }
            c.assign(d, None, &[(ns(1), &x)], DelayMode::Transport)?;
            c.wait_for(ns(1))
        }),
    );
    let seen = Arc::new(Mutex::new(Vec::new()));
    let log = seen.clone();
    b.process(
        "watch",
        NONE,
        &[],
        p(move |c| {
            let mut l = log.lock().unwrap();
            // Whole signal, whole blocks, single elements, a part of two blocks.
            let ranges = [
                (0, 200),
                (0, 64),
                (64, 64),
                (128, 72),
                (0, 128),
                (64, 136),
                (70, 1),
            ];
            for (o, n) in ranges {
                let r = v.slice(o, n);
                l.push(format!(
                    "{} {o}+{n} event={} active={} last_active={}",
                    c.now(),
                    c.event(r)?,
                    c.active(r)?,
                    c.last_active(r)?
                ));
                if n > 1 {
                    l.push(format!("  last_event={}", c.last_event(r)?));
                }
                if n > 1 || o == 70 {
                    l.push(format!("  last_value={}", c.last_value(r)?));
                }
            }
            for (o, n) in [(60, 10), (63, 2), (100, 100), (5, 5)] {
                let r = v.slice(o, n);
                l.push(format!(
                    "{} {o}+{n} event={} active={}",
                    c.now(),
                    c.event(r)?,
                    c.active(r)?
                ));
                if partial && n > 1 {
                    l.push(format!("  part last_event={}", c.last_event(r)?));
                }
            }
            drop(l);
            c.wait_on(v)
        }),
    );
    let mut sim = b.build(Config::default()).unwrap();
    let r = sim.run_until(ns(60));
    let out = seen.lock().unwrap().clone();
    (r.map(|_| ()), out)
}

#[test]
fn a_lean_signal_answers_like_one_with_full_history() {
    let (full, a) = run(false, false);
    let (lean, b) = run(true, false);
    full.unwrap();
    lean.unwrap();
    assert!(a.len() > 400, "the watcher ran {} lines", a.len());
    assert_eq!(a, b);
}

#[test]
fn a_lean_signal_refuses_the_history_of_part_of_a_block() {
    let (full, a) = run(false, true);
    full.unwrap();
    assert!(a.iter().any(|l| l.starts_with("  part last_event=")));
    let (lean, _) = run(true, true);
    let e = lean.unwrap_err();
    let ErrorKind::Internal(m) = &e.kind else {
        panic!("{e:?}")
    };
    assert!(m.contains("last_event") && m.contains("signal v"), "{m}");
    assert!(m.contains("60..70"), "{m}");
}
