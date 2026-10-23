//! `Builder::guard_process`: GUARD changes in the cycle of the event on a signal of its condition,
//! and a guard whose condition reads another GUARD follows in the same cycle.

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

fn model(kernel_guard: bool) -> Vec<String> {
    let mut b = Builder::new();
    let one = Range::new(0, 1);
    let v = b.signal("v", BYTE, &[0]);
    let bool_sig = |b: &mut Builder, name: &str, init: StdULogic| {
        b.signal(name, SignalType::logic(1), &LogicVec::new(1, init))
    };
    let g1 = bool_sig(&mut b, "g1", StdULogic::One);
    let g2 = bool_sig(&mut b, "g2", StdULogic::One);
    let (dv, d1, d2) = (b.driver(v), b.driver(g1), b.driver(g2));
    // The testbench: v := 3 at 0 ns (delta 1), 10 at 1 ns.
    let mut step = 0;
    b.process(
        "stim",
        NONE,
        &[dv],
        p(move |c| {
            c.assign(
                dv,
                None,
                &[(Time::ZERO, &[[3u8, 10][step % 2]][..])],
                DelayMode::INERTIAL,
            )?;
            step += 1;
            if step < 2 {
                c.wait_for(ns(1))?;
            }
            Ok(())
        }),
    );
    // g1 <= v < 10; g2 <= g1 and v < 5, each waiting for its condition's signals.
    let guard = |dst: DriverId,
                 reads: Vec<SignalId>,
                 f: fn(&Ctx<'_>, SignalId, SignalId) -> bool,
                 g1: SignalId| {
        p(move |c| {
            let on = f(c, v, g1);
            let code = if on { StdULogic::One } else { StdULogic::Zero } as u8;
            c.assign_code_now(dst, one, code);
            for &r in &reads {
                if !c.wait_same() {
                    c.wait_on(r)?;
                }
            }
            Ok(())
        })
    };
    let p1 = b.process(
        "_guard1",
        NONE,
        &[d1],
        guard(d1, vec![v], |c, v, _| c.value(v).unwrap()[0] < 10, g1),
    );
    let p2 = b.process(
        "_guard2",
        NONE,
        &[d2],
        guard(
            d2,
            vec![v, g1],
            |c, v, g1| {
                c.elem_code(g1, 0).unwrap() == StdULogic::One as u8 && c.value(v).unwrap()[0] < 5
            },
            g1,
        ),
    );
    if kernel_guard {
        b.guard_process(p1, &[(v, Range::new(0, 1))]);
        b.guard_process(p2, &[(v, Range::new(0, 1)), (g1, one)]);
    }
    let (mut sim, trace) = build(
        b,
        Config {
            edge_triggers: false,
            ..Config::default()
        },
    );
    sim.run_until(ns(5)).unwrap();
    trace.take()
}

#[test]
fn guard_changes_in_the_cycle_of_its_condition() {
    let late = model(false);
    let same = model(true);
    // As a process, GUARD follows `v` a delta later and the second guard another one later.
    let at = |t: &[String], key: &str| {
        t.iter()
            .filter(|l| l.contains(key) && !l.starts_with("0 fs +0"))
            .cloned()
            .collect::<Vec<_>>()
    };
    let v = at(&same, "v[");
    let g1 = at(&same, "g1[");
    let g2 = at(&same, "g2[");
    let stamp = |l: &String| l.split(' ').take(3).collect::<Vec<_>>().join(" ");
    assert!(!g1.is_empty() && !g2.is_empty(), "{same:?}");
    // The same time and delta as the event on `v`, for both guards.
    assert_eq!(stamp(&g1[0]), stamp(&v[1]), "{same:?}");
    assert_eq!(stamp(&g2[0]), stamp(&v[1]), "{same:?}");
    let g1_late = at(&late, "g1[");
    assert_ne!(stamp(&g1_late[0]), stamp(&at(&late, "v[")[1]), "{late:?}");
    // And the values are the same either way.
    let values = |t: &[String]| {
        t.iter()
            .filter(|l| l.contains("g1[") || l.contains("g2["))
            .map(|l| l.rsplit('=').next().unwrap().to_string())
            .collect::<Vec<_>>()
    };
    assert_eq!(values(&late), values(&same));
}
