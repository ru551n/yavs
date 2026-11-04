//! A signal's form (`code_view`, `planes_view`, `bytes_view` and the views beside them) is fixed
//! when the simulation is built: forces, releases, deposits and resolved updates write the same
//! buffers. Compiled code made per form relies on it (`InstFacts`).

mod common;

use common::*;
use std::sync::{Arc, Mutex};
use yavs_kernel::*;
use yavs_logic::StdULogic;

const NONE: ProcFlags = ProcFlags {
    postponed: false,
    serial: false,
    several_waits: false,
    concurrent_call: false,
};

type Views = [(usize, usize); 5];

/// Addresses and lengths of the views of `s`, as integers (a process stays `Send`).
fn views(c: &Ctx<'_>, s: SignalId) -> Views {
    let code = c.code_view(s).map_or((0, 0), |(p, n)| (p as usize, n));
    let plane = c.planes_view(s).map_or((0, 0), |(p, n)| (p as usize, n));
    let byte = c
        .bytes_view(s)
        .map_or((0, 0), |(p, es, n)| (p as usize, es * n));
    let stamps = (
        c.event_view(s).map_or(0, |p| p as usize),
        c.last_view(s).map_or(0, |p| p as usize),
    );
    [code, plane, byte, stamps, (c.code_cap(s), 0)]
}

#[test]
fn a_signals_form_stays_through_force_deposit_and_resolution() {
    let logic = |n: u32, v| LogicVec::new(n as usize, v);
    let mut b = Builder::new();
    let scalar = b.signal("scalar", SignalType::logic(1), &logic(1, StdULogic::Zero));
    let narrow = b.signal("narrow", SignalType::logic(32), &logic(32, StdULogic::Zero));
    let nocode = b.signal("nocode", SignalType::logic(32), &logic(32, StdULogic::Zero));
    b.no_codes(nocode);
    let bytes = b.signal("bytes", SignalType::bytes(4, 2), &[0u8; 8]);
    let resolved = |b: &mut Builder, name: &str, n| {
        b.resolved_signal(
            name,
            SignalType::logic(n),
            &logic(n, StdULogic::Z),
            Resolution::StdLogic,
        )
    };
    // A wide signal with wide sources keeps no codes, one with small sources does.
    let bus = resolved(&mut b, "bus", 128);
    let small = resolved(&mut b, "small", 128);
    let mut drivers = vec![b.driver(bus), b.driver(bus)];
    let (scalar_d, narrow_d, bytes_d) = (b.driver(scalar), b.driver(narrow), b.driver(bytes));
    drivers.extend([b.driver(small.slice(0, 8)), b.driver(small.slice(0, 8))]);
    let (d0, d1, d2) = (drivers[0], drivers[1], drivers[2]);
    let mut step = 0u8;
    b.process(
        "drv",
        NONE,
        &[d0, d1, d2, drivers[3], scalar_d, narrow_d, bytes_d],
        p(move |c| {
            step = step.wrapping_add(1);
            let v = if step.is_multiple_of(2) {
                StdULogic::One
            } else {
                StdULogic::X
            };
            c.drive(d0, &logic(128, v))?;
            c.drive(d1, &logic(128, StdULogic::Zero))?;
            c.drive(d2, &logic(8, v))?;
            c.drive(drivers[3], &logic(8, StdULogic::Zero))?;
            c.drive(scalar_d, &logic(1, v))?;
            c.drive(narrow_d, &logic(32, v))?;
            c.drive(bytes_d, &[step; 8])?;
            c.wait_for(ns(10))
        }),
    );
    let seen: Arc<Mutex<Vec<Vec<Views>>>> = Arc::default();
    let log = seen.clone();
    let all = [scalar, narrow, nocode, bytes, bus, small];
    let mut count = 0;
    b.process(
        "watch",
        NONE,
        &[],
        p(move |c| {
            count += 1;
            match count {
                3 => c.force(narrow, ForceMode::Effective, &logic(32, StdULogic::One))?,
                4 => c.release(narrow, ForceMode::Effective)?,
                5 => c.force(bus, ForceMode::Driving, &logic(128, StdULogic::Zero))?,
                6 => c.release(bus, ForceMode::Driving)?,
                _ => {}
            }
            log.lock()
                .unwrap()
                .push(all.iter().map(|&s| views(c, s)).collect());
            c.wait_for(ns(5))
        }),
    );
    let (mut sim, _trace) = build(b, Config::default());
    sim.run_until(ns(52)).unwrap();
    sim.deposit(narrow, &logic(32, StdULogic::Z)).unwrap();
    sim.deposit(bytes, &[9u8; 8]).unwrap();
    sim.force(small, ForceMode::Effective, &logic(128, StdULogic::Z))
        .unwrap();
    sim.force(bytes, ForceMode::Effective, &[7u8; 8]).unwrap();
    sim.run_until(ns(72)).unwrap();
    sim.release(small, ForceMode::Effective).unwrap();
    sim.release(bytes, ForceMode::Effective).unwrap();
    sim.run_until(ns(100)).unwrap();

    let seen = seen.lock().unwrap();
    assert!(seen.len() > 15, "{} activations", seen.len());
    assert!(
        seen.iter().all(|v| *v == seen[0]),
        "a view changed during the run"
    );
    // The forms this test means to cover.
    let [scalar, narrow, nocode, bytes, bus, small] = &seen[0][..] else {
        unreachable!()
    };
    assert!(scalar[0].0 != 0 && narrow[0].0 != 0, "logic: codes");
    assert!(nocode[0].0 == 0 && nocode[1].0 != 0, "no codes: planes");
    assert!(bytes[2].0 != 0 && bytes[1].0 == 0, "bytes: no planes");
    assert!(bus[0].0 == 0 && bus[1].0 != 0, "a wide bus: planes");
    assert!(small[0].0 != 0, "wide with small sources: codes");
}
