//! Kernel asks K1 to K7 of docs/design/ir.md (K6, buffered user files, was dropped: #82).

mod common;

use std::sync::Arc;

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

/// A process that assigns `(delay ns, value)` pairs once, transport.
fn stim(
    b: &mut Builder,
    name: &str,
    target: impl Into<SigRef>,
    wf: &'static [(u64, u8)],
) -> DriverId {
    let d = b.driver(target);
    b.process(
        name,
        NONE,
        &[d],
        p(move |c| {
            let wf: Vec<(Time, [u8; 1])> = wf.iter().map(|&(t, v)| (ns(t), [v])).collect();
            let wf: Vec<(Time, &[u8])> = wf.iter().map(|(t, v)| (*t, &v[..])).collect();
            c.assign(d, None, &wf, DelayMode::Transport)
        }),
    );
    d
}

fn lines_with(t: &Trace, pat: &str) -> Vec<String> {
    t.take().into_iter().filter(|l| l.contains(pat)).collect()
}

// ---- K1: ports ------------------------------------------------------------

#[test]
fn in_and_out_ports_propagate_without_a_delta() {
    let mut b = Builder::new();
    let a = b.signal("top.a", BYTE, &[0]);
    let y = b.signal("top.y", BYTE, &[0]);
    let f1 = b.signal("top.u.i", BYTE, &[0]);
    let f2 = b.signal("top.u.v.i", BYTE, &[0]);
    let o = b.signal("top.u.o", BYTE, &[0]);
    b.port(f1, a, PortMode::In, PortConv::default());
    b.port(f2, f1, PortMode::In, PortConv::default());
    b.port(o, y, PortMode::Out, PortConv::default());
    stim(&mut b, "top.stim", a, &[(1, 1), (2, 7)]);
    let d = b.driver(o);
    b.process(
        "top.u.v.copy",
        NONE,
        &[d],
        p(move |c| {
            let v = c.value(f2)?.to_vec();
            c.drive(d, &v)?;
            c.wait_on(f2)
        }),
    );
    let (mut sim, t) = build(b, Config::default());
    sim.run_until(ns(5)).unwrap();
    assert_eq!(
        t.take(),
        [
            "1 ns +0 top.a[0..1]=[1]",
            "1 ns +0 top.u.i[0..1]=[1]",
            "1 ns +0 top.u.v.i[0..1]=[1]",
            "1 ns +1 top.y[0..1]=[1]",
            "1 ns +1 top.u.o[0..1]=[1]",
            "2 ns +0 top.a[0..1]=[7]",
            "2 ns +0 top.u.i[0..1]=[7]",
            "2 ns +0 top.u.v.i[0..1]=[7]",
            "2 ns +1 top.y[0..1]=[7]",
            "2 ns +1 top.u.o[0..1]=[7]",
        ]
    );
}

#[test]
fn inout_port_on_a_resolved_logic_bus() {
    let mut b = Builder::new();
    let bus = b.resolved_signal(
        "bus",
        SignalType::logic(4),
        &lv("UUUU"),
        Resolution::StdLogic,
    );
    let io = b.resolved_signal(
        "u.io",
        SignalType::logic(4),
        &lv("UUUU"),
        Resolution::StdLogic,
    );
    b.port(io, bus, PortMode::Inout, PortConv::default());
    let dt = b.driver(bus);
    b.process("top", NONE, &[dt], p(move |c| c.drive(dt, &lv("1Z1Z"))));
    let du = b.driver(io);
    let mut first = true;
    b.process(
        "u.p",
        NONE,
        &[du],
        p(move |c| {
            if std::mem::take(&mut first) {
                c.drive(du, &lv("0Z0Z"))?;
            } else {
                assert_eq!(c.logic(io)?, lv("XZXZ"));
                assert_eq!(c.driving_value(du)?, Value::Logic(lv("0Z0Z")));
            }
            c.wait_on(io)
        }),
    );
    let (mut sim, t) = build(b, Config::default());
    sim.run_until(ns(1)).unwrap();
    assert_eq!(sim.value(bus).unwrap(), Value::Logic(lv("XZXZ")));
    assert_eq!(sim.driving_value(io).unwrap(), Value::Logic(lv("0Z0Z")));
    assert_eq!(
        t.take(),
        ["0 fs +1 bus[0..4]=\"XZXZ\"", "0 fs +1 u.io[0..4]=\"XZXZ\""]
    );
}

#[test]
fn conversion_functions_in_both_directions() {
    // Formal: an 8-bit count as one byte; actual: an 8-element logic vector.
    let to_formal: Converter = Arc::new(|v| {
        let l = v[0].as_logic().ok_or("not logic")?;
        Ok(vec![Value::Bytes(vec![
            l.to_u64().ok_or("metavalue")? as u8
        ])])
    });
    let to_actual: Converter = Arc::new(|v| {
        Ok(vec![Value::Logic(LogicVec::from_u64(
            8,
            u64::from(v[0].as_bytes().ok_or("not bytes")?[0]),
        ))])
    });
    let mut b = Builder::new();
    let a = b.signal("a", SignalType::logic(8), &lv("00000000"));
    let f = b.signal("u.f", BYTE, &[0]);
    let y = b.signal("y", SignalType::logic(8), &lv("00000000"));
    let o = b.signal("u.o", BYTE, &[0]);
    b.port(
        f,
        a,
        PortMode::In,
        PortConv {
            to_formal: Some(to_formal),
            to_actual: None,
        },
    );
    b.port(
        o,
        y,
        PortMode::Out,
        PortConv {
            to_formal: None,
            to_actual: Some(to_actual),
        },
    );
    let da = b.driver(a);
    b.process(
        "stim",
        NONE,
        &[da],
        p(move |c| c.assign(da, None, &[(ns(1), &lv("00000101"))], DelayMode::Transport)),
    );
    let d = b.driver(o);
    b.process(
        "u.inc",
        NONE,
        &[d],
        p(move |c| {
            let v = c.value(f)?[0];
            c.drive(d, &[v + 1])?;
            c.wait_on(f)
        }),
    );
    let (mut sim, _) = build(b, Config::default());
    sim.run_until(ns(2)).unwrap();
    assert_eq!(sim.value(f).unwrap(), Value::Bytes(vec![5]));
    assert_eq!(sim.value(y).unwrap(), Value::Logic(lv("00000110")));
}

#[test]
fn multi_part_conversions_run_in_the_same_delta() {
    // A record port of two leaves (a byte and a logic element) converted to and from two
    // actual signals at different port levels: one conversion call per cycle sees both
    // parts, and everything settles in the delta of the source.
    let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let seen = calls.clone();
    let swap: Converter = Arc::new(move |v| {
        seen.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let n = v[0].as_bytes().ok_or("not bytes")?[0];
        let l = v[1].as_logic().ok_or("not logic")?.get(0);
        let b = u8::from(l == StdULogic::One);
        Ok(vec![
            Value::Logic(LogicVec::new(
                1,
                if n & 1 == 1 {
                    StdULogic::One
                } else {
                    StdULogic::Zero
                },
            )),
            Value::Bytes(vec![n + b]),
        ])
    });
    let mut b = Builder::new();
    let n = b.signal("n", BYTE, &[0]);
    let l = b.signal("l", SignalType::logic(1), &lv("0"));
    // `l` is fed through a port of its own, so its level differs from `n`.
    let lsrc = b.signal("lsrc", SignalType::logic(1), &lv("0"));
    let fa = b.signal("u.r.a", SignalType::logic(1), &lv("0"));
    let fb = b.signal("u.r.b", BYTE, &[0]);
    b.port(l, lsrc, PortMode::In, PortConv::default());
    b.port_parts(
        &[fa.into(), fb.into()],
        &[n.into(), l.into()],
        PortMode::In,
        PortConv {
            to_formal: Some(swap),
            to_actual: None,
        },
    );
    stim(&mut b, "sn", n, &[(1, 3), (2, 4)]);
    let dl = b.driver(lsrc);
    b.process(
        "sl",
        NONE,
        &[dl],
        p(move |c| c.assign(dl, None, &[(ns(2), &lv("1"))], DelayMode::Transport)),
    );
    let (mut sim, t) = build(b, Config::default());
    sim.run_until(ns(3)).unwrap();
    assert_eq!(sim.value(fb).unwrap(), Value::Bytes(vec![5]));
    assert_eq!(sim.value(fa).unwrap(), Value::Logic(lv("0")));
    // Initialisation, 1 ns and 2 ns: once per cycle although both parts change at 2 ns.
    assert_eq!(calls.load(std::sync::atomic::Ordering::Relaxed), 3);
    let got = lines_with(&t, "u.r.");
    assert_eq!(
        got,
        [
            "1 ns +0 u.r.a[0..1]=\"1\"",
            "1 ns +0 u.r.b[0..1]=[3]",
            "2 ns +0 u.r.a[0..1]=\"0\"",
            "2 ns +0 u.r.b[0..1]=[5]",
        ]
    );
}

#[test]
fn multi_part_out_conversion_feeds_every_actual_part() {
    let split: Converter = Arc::new(|v| {
        let n = v[0].as_bytes().ok_or("not bytes")?[0];
        Ok(vec![Value::Bytes(vec![n / 16]), Value::Bytes(vec![n % 16])])
    });
    let mut b = Builder::new();
    let hi = b.signal("hi", BYTE, &[0]);
    let lo = b.signal("lo", BYTE, &[0]);
    let o = b.signal("u.o", BYTE, &[0]);
    b.port_parts(
        &[o.into()],
        &[hi.into(), lo.into()],
        PortMode::Out,
        PortConv {
            to_formal: None,
            to_actual: Some(split),
        },
    );
    stim(&mut b, "so", o, &[(1, 0x35)]);
    let (mut sim, t) = build(b, Config::default());
    sim.run_until(ns(2)).unwrap();
    assert_eq!(sim.value(hi).unwrap(), Value::Bytes(vec![3]));
    assert_eq!(sim.value(lo).unwrap(), Value::Bytes(vec![5]));
    assert_eq!(
        lines_with(&t, "1 ns"),
        [
            "1 ns +0 hi[0..1]=[3]",
            "1 ns +0 lo[0..1]=[5]",
            "1 ns +0 u.o[0..1]=[53]"
        ]
    );
}

#[test]
fn multi_part_ports_need_a_conversion() {
    let mut b = Builder::new();
    let a = b.signal("a", BYTE, &[0]);
    let c = b.signal("c", BYTE, &[0]);
    let f = b.signal("u.f", SignalType::bytes(1, 2), &[0, 0]);
    b.port_parts(
        &[f.into()],
        &[a.into(), c.into()],
        PortMode::In,
        PortConv::default(),
    );
    let err = b.build(Config::default()).err().unwrap();
    assert!(err.to_string().contains("no conversion"), "{err}");
}

#[test]
fn conversion_failure_stops_the_simulation() {
    let bad: Converter = Arc::new(|_| Ok(vec![Value::Bytes(vec![1, 2])]));
    let mut b = Builder::new();
    let a = b.signal("a", BYTE, &[0]);
    let f = b.signal("u.f", BYTE, &[0]);
    b.port(
        f,
        a,
        PortMode::In,
        PortConv {
            to_formal: Some(bad),
            to_actual: None,
        },
    );
    let err = b.build(Config::default()).err().unwrap();
    assert!(
        matches!(err.kind, ErrorKind::Conversion { ref formal, .. } if formal == "u.f"),
        "{err}"
    );
}

#[test]
fn port_model_errors() {
    let mut b = Builder::new();
    let a = b.signal("a", BYTE, &[0]);
    let f = b.signal("f", BYTE, &[0]);
    b.port(f, a, PortMode::In, PortConv::default());
    b.port(a, f, PortMode::In, PortConv::default());
    assert!(b.build(Config::default()).is_err(), "cycle");

    let mut b = Builder::new();
    let a = b.signal("a", BYTE, &[0]);
    let f = b.signal("f", BYTE, &[0]);
    b.port(f, a, PortMode::In, PortConv::default());
    let d = b.driver(f);
    b.process("p", NONE, &[d], p(|_| Ok(())));
    assert!(b.build(Config::default()).is_err(), "driven in-port");

    let mut b = Builder::new();
    let a = b.signal("a", SignalType::logic(2), &lv("00"));
    let f = b.signal("f", BYTE, &[0]);
    b.port(f, a, PortMode::In, PortConv::default());
    assert!(
        b.build(Config::default()).is_err(),
        "type mismatch without conversion"
    );
}

// ---- K2: implicit signals ------------------------------------------------

#[test]
fn stable_quiet_and_delayed() {
    let mut b = Builder::new();
    let s = b.signal("s", BYTE, &[0]);
    b.stable_signal(s, ns(5));
    b.quiet_signal(s, ns(5));
    b.delayed_signal(s, ns(3));
    let st0 = b.stable_signal(s, Time::ZERO);
    // Event at 10, transaction without event at 20, event at 25.
    stim(&mut b, "stim", s, &[(10, 1), (20, 1), (25, 0)]);
    let mut seen = Vec::new();
    b.process(
        "watch",
        NONE,
        &[],
        p(move |c| {
            seen.push((c.now(), c.delta(), c.logic(st0)?.get(0) as u8 - 2));
            if c.now() == ns(10) && c.delta() == 1 {
                assert_eq!(seen[seen.len() - 2..], [(ns(10), 0, 0), (ns(10), 1, 1)]);
            }
            c.wait_on(st0)
        }),
    );
    let (mut sim, t) = build(b, Config::default());
    sim.run_until(ns(40)).unwrap();
    let all = t.take();
    let of = |attr: &str| {
        all.iter()
            .filter(|l| l.contains(attr))
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(
        of("'stable(5 ns)"),
        [
            "10 ns +0 s'stable(5 ns)[0..1]=\"0\"",
            "15 ns +0 s'stable(5 ns)[0..1]=\"1\"",
            "25 ns +0 s'stable(5 ns)[0..1]=\"0\"",
            "30 ns +0 s'stable(5 ns)[0..1]=\"1\"",
        ]
    );
    assert_eq!(
        of("'quiet"),
        [
            "10 ns +0 s'quiet(5 ns)[0..1]=\"0\"",
            "15 ns +0 s'quiet(5 ns)[0..1]=\"1\"",
            "20 ns +0 s'quiet(5 ns)[0..1]=\"0\"",
            "30 ns +0 s'quiet(5 ns)[0..1]=\"1\"",
        ]
    );
    assert_eq!(
        of("'delayed"),
        [
            "13 ns +0 s'delayed(3 ns)[0..1]=[1]",
            "28 ns +0 s'delayed(3 ns)[0..1]=[0]"
        ]
    );
    assert_eq!(of("'stable(0 fs)").len(), 4);
}

#[test]
fn implicit_signals_over_several_sources() {
    // A record of two fields: 'stable and 'quiet see events and activity on either.
    let mut b = Builder::new();
    let x = b.signal("x", BYTE, &[0]);
    let y = b.signal("y", BYTE, &[0]);
    let st = b.stable_signal(x, ns(5));
    b.implicit_source(st, y);
    let q = b.quiet_signal(x, ns(5));
    b.implicit_source(q, y);
    let d = b.delayed_signal(x, ns(1));
    b.implicit_source(d, y);
    stim(&mut b, "sx", x, &[(10, 1)]);
    stim(&mut b, "sy", y, &[(20, 1), (40, 1)]);
    assert!(
        b.build(Config::default()).is_err(),
        "'delayed has one source"
    );

    let mut b = Builder::new();
    let x = b.signal("x", BYTE, &[0]);
    let y = b.signal("y", BYTE, &[0]);
    let st = b.stable_signal(x, ns(5));
    b.implicit_source(st, y);
    let q = b.quiet_signal(x, ns(5));
    b.implicit_source(q, y);
    stim(&mut b, "sx", x, &[(10, 1)]);
    stim(&mut b, "sy", y, &[(20, 1), (40, 1)]);
    let (mut sim, t) = build(b, Config::default());
    sim.run_until(ns(60)).unwrap();
    let all = t.take();
    let of = |attr: &str| {
        all.iter()
            .filter(|l| l.contains(attr))
            .cloned()
            .collect::<Vec<_>>()
    };
    assert_eq!(
        of("'stable"),
        [
            "10 ns +0 x'stable(5 ns)[0..1]=\"0\"",
            "15 ns +0 x'stable(5 ns)[0..1]=\"1\"",
            "20 ns +0 x'stable(5 ns)[0..1]=\"0\"",
            "25 ns +0 x'stable(5 ns)[0..1]=\"1\"",
        ]
    );
    assert_eq!(
        of("'quiet"),
        [
            "10 ns +0 x'quiet(5 ns)[0..1]=\"0\"",
            "15 ns +0 x'quiet(5 ns)[0..1]=\"1\"",
            "20 ns +0 x'quiet(5 ns)[0..1]=\"0\"",
            "25 ns +0 x'quiet(5 ns)[0..1]=\"1\"",
            "40 ns +0 x'quiet(5 ns)[0..1]=\"0\"",
            "45 ns +0 x'quiet(5 ns)[0..1]=\"1\"",
        ]
    );
}

// ---- K3, K4: driving value, guarded signals, null transactions -----------

fn or_bytes() -> Resolution {
    Resolution::Func(Arc::new(|srcs: &[&[u8]], out: &mut [u8]| {
        for (i, o) in out.iter_mut().enumerate() {
            *o = srcs.iter().fold(0, |a, s| a | s[i]);
        }
        Ok(())
    }))
}

/// Drivers A (1) and B (2) on a guarded signal; A disconnects at 10 ns (disconnection time
/// 3 ns, so effective at 13), B at 20 ns with an explicit 0 delay.
fn guarded(kind: GuardKind) -> (Vec<String>, Vec<u8>) {
    let mut b = Builder::new();
    let s = b.resolved_signal("g", BYTE, &[0], or_bytes());
    b.guard(s, kind, ns(3));
    let da = b.driver(s);
    let db = b.driver(s);
    let mut n = 0;
    b.process(
        "a",
        NONE,
        &[da],
        p(move |c| {
            n += 1;
            match n {
                1 => {
                    c.drive(da, &[1])?;
                    c.wait_for(ns(10))
                }
                2 => {
                    assert!(c.driving(da)?);
                    c.disconnect(da, None, None, DelayMode::INERTIAL)?;
                    c.wait_for(ns(5))
                }
                _ => {
                    assert!(!c.driving(da)?);
                    let e = c.driving_value(da).unwrap_err();
                    assert!(matches!(e.kind, ErrorKind::Runtime(_)), "{e}");
                    Ok(())
                }
            }
        }),
    );
    let mut m = 0;
    b.process(
        "b",
        NONE,
        &[db],
        p(move |c| {
            m += 1;
            if m == 1 {
                c.drive(db, &[2])?;
                c.wait_for(ns(20))
            } else {
                c.disconnect(db, None, Some(Time::ZERO), DelayMode::Transport)
            }
        }),
    );
    let (mut sim, t) = build(b, Config::default());
    sim.run_until(ns(30)).unwrap();
    let v = v(&sim, s);
    (t.take(), v)
}

#[test]
fn guarded_bus_and_register() {
    let (bus, v) = guarded(GuardKind::Bus);
    assert_eq!(
        bus,
        [
            "0 fs +1 g[0..1]=[3]",
            "13 ns +0 g[0..1]=[2]",
            "20 ns +1 g[0..1]=[0]"
        ]
    );
    assert_eq!(v, [0]);
    let (reg, v) = guarded(GuardKind::Register);
    assert_eq!(reg, ["0 fs +1 g[0..1]=[3]", "13 ns +0 g[0..1]=[2]"]);
    assert_eq!(v, [2]);
}

#[test]
fn null_transaction_rules() {
    // Unguarded signal: an error. Unresolved guarded signal: a build error.
    let mut b = Builder::new();
    let s = b.signal("s", BYTE, &[0]);
    let d = b.driver(s);
    b.process(
        "p",
        NONE,
        &[d],
        p(move |c| c.disconnect(d, None, None, DelayMode::INERTIAL)),
    );
    let (mut sim, _) = build(b, Config::default());
    assert!(matches!(
        sim.run_until(ns(1)).unwrap_err().kind,
        ErrorKind::InvalidAssign(_)
    ));
    let mut b = Builder::new();
    let s = b.signal("s", BYTE, &[0]);
    b.guard(s, GuardKind::Bus, Time::ZERO);
    assert!(b.build(Config::default()).is_err());

    // A logic bus with every driver disconnected resolves to Z.
    let mut b = Builder::new();
    let s = b.resolved_signal("l", SignalType::logic(2), &lv("UU"), Resolution::StdLogic);
    b.guard(s, GuardKind::Bus, Time::ZERO);
    let d = b.driver(s);
    let mut first = true;
    b.process(
        "p",
        NONE,
        &[d],
        p(move |c| {
            if std::mem::take(&mut first) {
                c.drive(d, &lv("10"))?;
                c.wait_for(ns(1))
            } else {
                c.disconnect(d, Some(Range::new(1, 1)), None, DelayMode::INERTIAL)
            }
        }),
    );
    let (mut sim, _) = build(b, Config::default());
    sim.run_until(ns(2)).unwrap();
    assert_eq!(sim.value(s).unwrap(), Value::Logic(lv("Z0")));
}

#[test]
fn driving_value_under_force() {
    let mut b = Builder::new();
    let s = b.signal("s", BYTE, &[0]);
    let d = b.driver(s);
    let mut n = 0;
    b.process(
        "p",
        NONE,
        &[d],
        p(move |c| {
            n += 1;
            match n {
                1 => {
                    c.drive(d, &[1])?;
                    c.force(s, ForceMode::Effective, &[5])?;
                    c.wait_for(ns(1))
                }
                _ => {
                    assert_eq!(c.value(s)?, [5]);
                    assert_eq!(c.driving_value(d)?, Value::Bytes(vec![1]));
                    assert!(c.driving(d)?);
                    c.report(Severity::Note, "checked");
                    Ok(())
                }
            }
        }),
    );
    let (mut sim, t) = build(b, Config::default());
    sim.run_until(ns(2)).unwrap();
    assert_eq!(lines_with(&t, "checked").len(), 1);
}

// ---- K5: null ranges -------------------------------------------------------

#[test]
fn zero_length_signals_and_null_slices() {
    let mut b = Builder::new();
    let e = b.signal("empty", SignalType::bytes(4, 0), &[0u8; 0]);
    let l = b.signal(
        "lempty",
        SignalType::logic(0),
        &LogicVec::new(0, StdULogic::U),
    );
    let s = b.signal("s", BYTE, &[0]);
    let de = b.driver(e);
    let ds = b.driver(s);
    let mut runs = 0;
    b.process(
        "p",
        NONE,
        &[de, ds],
        p(move |c| {
            runs += 1;
            assert_eq!(c.value(e)?, [0u8; 0]);
            assert_eq!(c.logic(l)?.len(), 0);
            assert!(!c.event(e)? && !c.event(s.slice(1, 0))?);
            assert_eq!(c.last_event(e)?, Time::MAX);
            c.drive(de, &[0u8; 0])?;
            c.drive(ds, &[runs])?;
            c.wait_on(e)?;
            c.wait_on(s.slice(1, 0))?;
            c.wait_on(l)
        }),
    );
    let (mut sim, t) = build(b, Config::default());
    assert_eq!(
        sim.run_until(ns(1)).unwrap(),
        RunStatus::Ended(EndReason::Idle)
    );
    // The process ran once: nothing it waits on can have an event.
    assert_eq!(t.take(), ["0 fs +1 s[0..1]=[1]"]);
    assert!(sim.value(s.slice(2, 0)).is_err());
}

// ---- K7: bit-plane logic signals -------------------------------------------

#[test]
fn logic_signals_on_planes() {
    let mut b = Builder::new();
    let v = b.signal(
        "v",
        SignalType::logic(150),
        &LogicVec::new(150, StdULogic::Zero),
    );
    let d = b.driver(v);
    let mut n = 0;
    b.process(
        "stim",
        NONE,
        &[d],
        p(move |c| {
            n += 1;
            match n {
                1 => {
                    // Elements 60..70 straddle the first block boundary.
                    let ones = LogicVec::new(10, StdULogic::One);
                    c.assign(
                        d,
                        Some(Range::new(60, 10)),
                        &[(ns(1), &ones)],
                        DelayMode::INERTIAL,
                    )?;
                    // Rejected pulse on element 149: '1' at 2 ns, then '0' at 3 ns (inertial).
                    c.assign(
                        d,
                        Some(Range::new(149, 1)),
                        &[(ns(2), &lv("1"))],
                        DelayMode::INERTIAL,
                    )?;
                    c.wait_for(ns(1))
                }
                2 => c.assign(
                    d,
                    Some(Range::new(149, 1)),
                    &[(ns(2), &lv("0"))],
                    DelayMode::INERTIAL,
                ),
                _ => Ok(()),
            }
        }),
    );
    let mut seen = 0;
    b.process(
        "watch",
        NONE,
        &[],
        p(move |c| {
            if seen > 0 {
                assert!(c.event(v.slice(64, 1))? && !c.event(v.slice(70, 1))?);
                assert_eq!(c.logic(v.slice(58, 14))?, lv("00111111111100"));
                assert_eq!(c.last_value(v.slice(60, 2))?, Value::Logic(lv("00")));
            }
            seen += 1;
            c.wait_on(v.slice(60, 10))
        }),
    );
    let (mut sim, t) = build(b, Config::default());
    sim.force(v.slice(100, 3), ForceMode::Effective, &lv("XZX"))
        .unwrap();
    sim.run_until(ns(5)).unwrap();
    assert_eq!(
        t.take(),
        [
            "0 fs +1 v[100..103]=\"XZX\"",
            "1 ns +0 v[60..70]=\"1111111111\""
        ]
    );
    assert!(
        sim.force(v.slice(0, 2), ForceMode::Effective, &[0u8, 0])
            .is_err()
    );
}
