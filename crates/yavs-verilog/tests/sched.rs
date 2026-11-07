//! The Verilog scheduling prototype (#776 step A): regions, the boundary to VHDL (decision D4)
//! and the kernel contract it rests on. VHDL processes are kernel closures, Verilog processes
//! domain closures; traces hold value changes and `$display` lines with time and delta.

use std::sync::{Arc, Mutex};

use yavs_kernel::{
    Builder, Config, Ctx, DelayMode, EndReason, ErrorKind, Hook, Interrupt, LogicVec, Observer,
    ProcFlags, ProcId, Process, Range, RunStatus, SignalId, SignalType, SimControl, SimError, Time,
    Value,
};
use yavs_logic::StdULogic::{self, *};
use yavs_verilog::{Domain, Edge, NetId, Wait, edge};

fn v(s: &str) -> LogicVec {
    s.parse().unwrap()
}

fn num(l: &LogicVec) -> u64 {
    (0..l.len()).fold(0, |a, i| a | (u64::from(l.get(i) == One) << i))
}

fn lv(n: u64, w: usize) -> LogicVec {
    LogicVec::from_fn(w, |i| if n >> i & 1 == 1 { One } else { Zero })
}

fn ns(n: u64) -> Time {
    Time::ns(n)
}

/// Gives a closure the higher-ranked signature `Process` needs.
fn p(f: impl FnMut(&mut Ctx<'_>) -> Result<(), SimError> + Send + 'static) -> impl Process {
    f
}

fn sig(b: &mut Builder, name: &str, init: &str) -> SignalId {
    b.signal(name, SignalType::logic(init.len() as u32), &v(init))
}

#[derive(Clone, Default)]
struct Trace(Arc<Mutex<Vec<String>>>);

impl Trace {
    fn lines(&self) -> Vec<String> {
        self.0.lock().unwrap().clone()
    }

    /// The lines that mention one of `names` (`name=` or an output line starting with it).
    fn of(&self, names: &[&str]) -> Vec<String> {
        self.lines()
            .into_iter()
            .filter(|l| {
                names
                    .iter()
                    .any(|n| l.contains(&format!(" {n}=")) || l.contains(&format!("out: {n}")))
            })
            .collect()
    }
}

impl Observer for Trace {
    fn value_change(&mut self, h: &mut Hook<'_>, s: SignalId, _r: Range) {
        let v = match h.value(s).unwrap() {
            Value::Logic(l) => l.to_string(),
            Value::Bytes(b) => format!("{b:?}"),
        };
        let line = format!(
            "{} +{} {}={v}",
            h.now(),
            h.delta(),
            h.signal_name(s).unwrap()
        );
        self.0.lock().unwrap().push(line);
    }

    fn output(&mut self, h: &mut Hook<'_>, _p: ProcId, bytes: &[u8]) {
        let text = String::from_utf8_lossy(bytes);
        let line = format!("{} +{} out: {}", h.now(), h.delta(), text.trim_end());
        self.0.lock().unwrap().push(line);
    }
}

fn build(b: Builder, cfg: Config) -> (SimControl, Trace) {
    let mut sim = b.build(cfg).unwrap();
    let t = Trace::default();
    sim.subscribe(Box::new(t.clone()));
    (sim, t)
}

fn run(b: Builder, until: Time) -> Trace {
    let (mut sim, t) = build(b, Config::default());
    sim.run_until(until).unwrap();
    t
}

/// The domain as kernel process `top.v` owning `drivers`.
fn add(b: &mut Builder, d: Domain, drivers: &[yavs_kernel::DriverId]) {
    b.process("top.v", ProcFlags::default(), drivers, d);
}

/// A VHDL flip-flop: `q <= d` on a rising edge of `clk`.
fn vhdl_flop(b: &mut Builder, name: &str, clk: SignalId, d: SignalId, q: SignalId) {
    let dq = b.driver(q);
    b.process(
        name,
        ProcFlags::default(),
        &[dq],
        p(move |c| {
            if c.event(clk)? && c.logic(clk)?.get(0) == One {
                let v = c.logic(d)?;
                c.assign(dq, None, &[(Time::ZERO, &v)], DelayMode::INERTIAL)?;
            }
            c.wait_on(clk)
        }),
    );
}

/// `initial begin clk = 0; forever #half clk = ~clk; end`
fn verilog_clock(d: &mut Domain, clk: NetId, half: u64) {
    let mut first = true;
    d.process("clkgen", move |x| {
        let next = match std::mem::replace(&mut first, false) {
            true => v("0"),
            false => x.get(clk).not(),
        };
        x.set(clk, &next)?;
        Ok(Wait::Delay(ns(half)))
    });
}

/// The two kernel behaviours the bridge rests on: a wait with a zero timeout resumes in the
/// next delta of the same time, and a timeout can be named again after an earlier wake-up.
#[test]
fn kernel_contract() {
    let mut b = Builder::new();
    let x = sig(&mut b, "x", "0");
    let dx = b.driver(x);
    let seen = Arc::new(Mutex::new(Vec::new()));
    let s = seen.clone();
    let mut step = 0;
    b.process(
        "probe",
        ProcFlags::default(),
        &[],
        p(move |c| {
            s.lock().unwrap().push((c.now(), c.delta(), c.timed_out()));
            step += 1;
            c.wait_on(x)?;
            match step {
                1 => c.wait_for(Time::ZERO),
                2..4 => c.wait_at(ns(10)),
                _ => Ok(()),
            }
        }),
    );
    let mut first = true;
    b.process(
        "poke",
        ProcFlags::default(),
        &[dx],
        p(move |c| {
            if std::mem::replace(&mut first, false) {
                c.assign(dx, None, &[(ns(5), &v("1"))], DelayMode::INERTIAL)?;
            }
            Ok(())
        }),
    );
    run(b, ns(20));
    assert_eq!(
        *seen.lock().unwrap(),
        [
            (Time::ZERO, 0, false),
            (Time::ZERO, 1, true),
            (ns(5), 0, false),
            (ns(10), 0, true),
        ]
    );
}

#[test]
fn edge_table() {
    let codes = [Zero, One, X, Z];
    let mut got = Vec::new();
    for a in codes {
        for b in codes {
            match edge(a, b) {
                Some(Edge::Pos) => got.push(format!("{a}{b}+")),
                Some(Edge::Neg) => got.push(format!("{a}{b}-")),
                _ => {}
            }
        }
    }
    // IEEE 1364-2005 Table 9-9; x and z alike, x to z no edge.
    assert_eq!(
        got,
        [
            "01+", "0X+", "0Z+", "10-", "1X-", "1Z-", "X0-", "X1+", "Z0-", "Z1+"
        ]
    );
    // VHDL codes reach Verilog as TO_X01Z: L is 0, H is 1, U and W are x.
    assert_eq!(edge(StdULogic::L, StdULogic::H), Some(Edge::Pos));
    assert_eq!(edge(StdULogic::H, StdULogic::U), Some(Edge::Neg));
}

#[test]
fn zero_delay_runs_before_nba() {
    zero_delay_case(Time::ZERO, Config::default());
    // A delay below the resolution is `#0` (and `<= #0`).
    let ns_res = Config {
        resolution: yavs_kernel::Unit::Ns,
        ..Config::default()
    };
    zero_delay_case(Time::fs(1).unwrap(), ns_res);
}

fn zero_delay_case(delay: Time, cfg: Config) {
    let mut d = Domain::new();
    let x = d.net(v("0"));
    let mut step = 0;
    d.process("a", move |e| {
        step += 1;
        if step == 1 {
            e.nba(x, 0, v("1"), delay)?;
            return Ok(Wait::Delay(delay));
        }
        e.display(&format!("a after #0: x={}", e.get(x)));
        Ok(Wait::Done)
    });
    d.always("b", vec![(x, Edge::Any)], move |e| {
        e.display(&format!("b: x={}", e.get(x)));
        Ok(())
    });
    let mut b = Builder::new();
    add(&mut b, d, &[]);
    let (mut sim, t) = build(b, cfg);
    sim.run_until(ns(1)).unwrap();
    assert_eq!(
        t.lines(),
        ["0 fs +0 out: a after #0: x=0", "0 fs +0 out: b: x=1"]
    );
}

/// `#3 $display($time); $finish;`
#[test]
fn finish_ends_the_run() {
    let mut d = Domain::new();
    let mut step = 0;
    d.process("p", move |e| {
        step += 1;
        if step == 1 {
            return Ok(Wait::Delay(ns(3)));
        }
        e.display(&format!("t={}", e.now()));
        e.finish();
        Ok(Wait::Done)
    });
    let mut b = Builder::new();
    add(&mut b, d, &[]);
    let (mut sim, t) = build(b, Config::default());
    assert_eq!(
        sim.run_until(ns(10)).unwrap(),
        RunStatus::Ended(EndReason::Finish(0))
    );
    assert_eq!(t.lines(), ["3 ns +0 out: t=3 ns"]);
}

#[test]
fn blocking_write_is_seen_at_once() {
    let mut d = Domain::new();
    let (x, y, z) = (d.net(v("0")), d.net(v("0")), d.net(v("0")));
    // Woken by the blocking write in the same Active region, before the NBA region.
    d.always("b", vec![(x, Edge::Any)], move |e| {
        e.display(&format!("b: x={} y={} z={}", e.get(x), e.get(y), e.get(z)));
        Ok(())
    });
    d.process("a", move |e| {
        e.set(x, &v("1"))?;
        e.display(&format!("a: x={}", e.get(x)));
        let xv = e.get(x).clone();
        e.set(y, &xv)?;
        e.nba(z, 0, v("1"), Time::ZERO)?;
        Ok(Wait::Done)
    });
    // Declared after `a`: it only waits once `a` ran, so it misses the write.
    d.always("late", vec![(x, Edge::Any)], move |e| {
        e.display("late woken");
        Ok(())
    });
    let mut b = Builder::new();
    add(&mut b, d, &[]);
    assert_eq!(
        run(b, ns(1)).lines(),
        ["0 fs +0 out: a: x=1", "0 fs +0 out: b: x=1 y=1 z=0"]
    );
}

/// `i = 0; a = 1; m[i] <= a; i = 1; a = 0; m[i] <= a;`: each NBA keeps the index and the value
/// of its own execution.
#[test]
fn nba_takes_value_and_index_when_it_runs() {
    let mut d = Domain::new();
    let (m, i, a) = (d.net(v("XXXX")), d.net(v("XX")), d.net(v("X")));
    let mut step = 0;
    d.process("p", move |e| {
        step += 1;
        if step == 2 {
            e.display(&format!("m={}", e.get(m)));
            return Ok(Wait::Done);
        }
        for (k, bit) in [(0, "1"), (1, "0")] {
            e.set(i, &lv(k, 2))?;
            e.set(a, &v(bit))?;
            let (at, val) = (num(e.get(i)) as u32, e.get(a).clone());
            e.nba(m, at, val, Time::ZERO)?;
        }
        e.set(a, &v("Z"))?;
        Ok(Wait::Delay(ns(1)))
    });
    let mut b = Builder::new();
    add(&mut b, d, &[]);
    assert_eq!(run(b, ns(2)).lines(), ["1 ns +0 out: m=XX01"]);
}

/// `q <= #10 1; q <= #5 0;`: Verilog keeps both. The same pair of transport assignments on a VHDL
/// driver keeps only the second (LRM 10.5.2.2), so future NBAs must not use the kernel's
/// transaction queue.
#[test]
fn future_nbas_are_not_replaced() {
    let mut b = Builder::new();
    let (vq, w) = (sig(&mut b, "vq", "U"), sig(&mut b, "w", "U"));
    let (dq, dw) = (b.driver(vq), b.driver(w));
    let mut d = Domain::new();
    let q = d.net(v("X"));
    d.output(q, dq);
    d.process("p", move |e| {
        e.nba(q, 0, v("1"), ns(10))?;
        e.nba(q, 0, v("0"), ns(5))?;
        Ok(Wait::Done)
    });
    add(&mut b, d, &[dq]);
    let mut first = true;
    b.process(
        "vhdl",
        ProcFlags::default(),
        &[dw],
        p(move |c| {
            if std::mem::replace(&mut first, false) {
                c.assign(dw, None, &[(ns(10), &v("1"))], DelayMode::Transport)?;
                c.assign(dw, None, &[(ns(5), &v("0"))], DelayMode::Transport)?;
            }
            Ok(())
        }),
    );
    let t = run(b, ns(20));
    assert_eq!(
        t.of(&["vq"]),
        ["0 fs +1 vq=X", "5 ns +2 vq=0", "10 ns +2 vq=1"]
    );
    assert_eq!(t.of(&["w"]), ["5 ns +0 w=0"]);
}

/// D4 with a clock from Verilog: the clock change of the Active region reaches VHDL a delta
/// before the register results of the NBA region, so a VHDL flip-flop on that clock samples the
/// old value; a Verilog flip-flop behind `assign clkb = clk` samples the old value too (a
/// continuous assignment costs no delta).
#[test]
fn verilog_clock_and_vhdl_flop() {
    let mut b = Builder::new();
    let (vclk, vcnt, vcnt2, vs) = (
        sig(&mut b, "vclk", "U"),
        sig(&mut b, "vcnt", "UU"),
        sig(&mut b, "vcnt2", "UU"),
        sig(&mut b, "vs", "UU"),
    );
    let (dclk, dcnt, dcnt2) = (b.driver(vclk), b.driver(vcnt), b.driver(vcnt2));
    let mut d = Domain::new();
    let (clk, clkb, cnt, cnt2) = (d.net(v("X")), d.net(v("Z")), d.net(v("00")), d.net(v("XX")));
    verilog_clock(&mut d, clk, 5);
    d.assign("bufg", &[clk], move |e| {
        let c = e.get(clk).clone();
        e.set(clkb, &c)
    });
    d.always("cnt", vec![(clk, Edge::Pos)], move |e| {
        let n = lv(num(e.get(cnt)) + 1, 2);
        e.nba(cnt, 0, n, Time::ZERO)
    });
    d.always("cnt2", vec![(clkb, Edge::Pos)], move |e| {
        let n = e.get(cnt).clone();
        e.nba(cnt2, 0, n, Time::ZERO)
    });
    d.output(clk, dclk);
    d.output(cnt, dcnt);
    d.output(cnt2, dcnt2);
    add(&mut b, d, &[dclk, dcnt, dcnt2]);
    vhdl_flop(&mut b, "top.ff", vclk, vcnt, vs);
    assert_eq!(
        run(b, ns(16)).lines(),
        [
            "0 fs +1 vclk=0",
            "0 fs +1 vcnt=00",
            "0 fs +1 vcnt2=XX",
            "5 ns +1 vclk=1",
            "5 ns +2 vcnt=01",
            "5 ns +2 vcnt2=00",
            "5 ns +2 vs=00",
            "10 ns +1 vclk=0",
            "15 ns +1 vclk=1",
            "15 ns +2 vcnt=10",
            "15 ns +2 vcnt2=01",
            "15 ns +2 vs=01",
        ]
    );
}

/// D4 with a clock from VHDL: the register result is published one delta after the edge
/// reaches the domain, whether or not the Active region changed an output (fixed: Active
/// results at `d+1`, NBA results at `d+2`).
#[test]
fn vhdl_clock_and_verilog_flop() {
    let mut b = Builder::new();
    let (vclk, vq, vs) = (
        sig(&mut b, "vclk", "0"),
        sig(&mut b, "vq", "0"),
        sig(&mut b, "vs", "0"),
    );
    let (dclk, dq) = (b.driver(vclk), b.driver(vq));
    b.process(
        "top.clk",
        ProcFlags::default(),
        &[dclk],
        p(move |c| {
            let n = c.logic(vclk)?.not();
            c.assign(dclk, None, &[(ns(5), &n)], DelayMode::INERTIAL)?;
            c.wait_on(vclk)
        }),
    );
    let mut d = Domain::new();
    let (clk, q) = (d.net(v("0")), d.net(v("0")));
    d.input(vclk, clk);
    d.output(q, dq);
    d.always("tff", vec![(clk, Edge::Pos)], move |e| {
        let n = e.get(q).not();
        e.nba(q, 0, n, Time::ZERO)
    });
    add(&mut b, d, &[dq]);
    vhdl_flop(&mut b, "top.ff", vclk, vq, vs);
    assert_eq!(
        run(b, ns(16)).of(&["vq", "vs"]),
        ["5 ns +2 vq=1", "15 ns +1 vs=1", "15 ns +2 vq=0"]
    );
}

/// VHDL `req` to Verilog `assign ack = req` back to VHDL, which drops `req` on `ack`: the loop
/// settles in four deltas and the simulation ends idle.
#[test]
fn vhdl_verilog_vhdl_feedback_settles() {
    let mut b = Builder::new();
    let (vreq, vack) = (sig(&mut b, "vreq", "0"), sig(&mut b, "vack", "0"));
    let (dreq, dack) = (b.driver(vreq), b.driver(vack));
    let mut step = 0;
    b.process(
        "top.req",
        ProcFlags::default(),
        &[dreq],
        p(move |c| {
            step += 1;
            match step {
                1 => c.wait_for(ns(10)),
                2 => {
                    c.assign(dreq, None, &[(Time::ZERO, &v("1"))], DelayMode::INERTIAL)?;
                    c.wait_on(vack)
                }
                _ if c.logic(vack)?.get(0) == One => {
                    c.assign(dreq, None, &[(Time::ZERO, &v("0"))], DelayMode::INERTIAL)?;
                    c.wait_on(vack)
                }
                _ => Ok(()),
            }
        }),
    );
    let mut d = Domain::new();
    let (req, ack) = (d.net(v("0")), d.net(v("0")));
    d.input(vreq, req);
    d.output(ack, dack);
    d.assign("ack", &[req], move |e| {
        let r = e.get(req).clone();
        e.set(ack, &r)
    });
    add(&mut b, d, &[dack]);
    let (mut sim, t) = build(b, Config::default());
    assert_eq!(
        sim.run_until(Time::MAX).unwrap(),
        RunStatus::Ended(EndReason::Idle)
    );
    assert_eq!(
        t.lines(),
        [
            "10 ns +1 vreq=1",
            "10 ns +2 vack=1",
            "10 ns +3 vreq=0",
            "10 ns +4 vack=0",
        ]
    );
}

/// posedge, negedge and any change of a VHDL input under the Verilog rules (Table 9-9 after
/// TO_X01Z), and of a vector input, whose edges are those of element 0.
#[test]
fn xz_edges_from_vhdl() {
    let mut b = Builder::new();
    let (vs, vv) = (sig(&mut b, "vs", "0"), sig(&mut b, "vv", "00"));
    let (ds, dv) = (b.driver(vs), b.driver(vv));
    let seq = "X1Z0LHUWZ1";
    let mut k = 0;
    b.process(
        "top.stim",
        ProcFlags::default(),
        &[ds, dv],
        p(move |c| {
            if k > 0 {
                let code = &seq[k - 1..k];
                c.assign(ds, None, &[(Time::ZERO, &v(code))], DelayMode::INERTIAL)?;
                let vec = ["10", "11", "01"].get(k - 1).copied().unwrap_or("01");
                c.assign(dv, None, &[(Time::ZERO, &v(vec))], DelayMode::INERTIAL)?;
            }
            k += 1;
            if k <= seq.len() {
                c.wait_for(ns(1))?;
            }
            Ok(())
        }),
    );
    let mut d = Domain::new();
    let (s, vecn) = (d.net(v("X")), d.net(v("XX")));
    d.input(vs, s);
    d.input(vv, vecn);
    for (name, item) in [
        ("pos", (s, Edge::Pos)),
        ("neg", (s, Edge::Neg)),
        ("any", (s, Edge::Any)),
        ("vpos", (vecn, Edge::Pos)),
        ("vany", (vecn, Edge::Any)),
    ] {
        d.always(name, vec![item], move |e| {
            let val = e.get(item.0).to_string();
            e.display(&format!("{name} {val}"));
            Ok(())
        });
    }
    add(&mut b, d, &[]);
    let got: Vec<String> = run(b, ns(20))
        .lines()
        .into_iter()
        .filter(|l| l.contains("out: "))
        .collect();
    assert_eq!(
        got,
        [
            "1 ns +1 out: pos X",
            "1 ns +1 out: any X",
            "1 ns +1 out: vany 10",
            "2 ns +1 out: pos 1",
            "2 ns +1 out: any 1",
            "2 ns +1 out: vpos 11",
            "2 ns +1 out: vany 11",
            "3 ns +1 out: neg Z",
            "3 ns +1 out: any Z",
            "3 ns +1 out: vany 01",
            "4 ns +1 out: neg 0",
            "4 ns +1 out: any 0",
            "6 ns +1 out: pos 1",
            "6 ns +1 out: any 1",
            "7 ns +1 out: neg X",
            "7 ns +1 out: any X",
            "9 ns +1 out: any Z",
            "10 ns +1 out: pos 1",
            "10 ns +1 out: any 1",
        ]
    );
}

/// Initialisation: the VHDL initial value of an input is where it starts (no event); every
/// output is published at delta 1, with the value of the Active region of time 0 or its initial
/// value, and an NBA of time 0 a delta later.
#[test]
fn initialisation() {
    let mut b = Builder::new();
    let (vin, vr, vw, vn) = (
        sig(&mut b, "vin", "1"),
        sig(&mut b, "vr", "U"),
        sig(&mut b, "vw", "U"),
        sig(&mut b, "vn", "U"),
    );
    let (dr, dw, dn) = (b.driver(vr), b.driver(vw), b.driver(vn));
    let mut d = Domain::new();
    let (inp, r, w, n) = (d.net(v("X")), d.net(v("X")), d.net(v("Z")), d.net(v("X")));
    d.input(vin, inp);
    d.output(r, dr);
    d.output(w, dw);
    d.output(n, dn);
    d.always("watch", vec![(inp, Edge::Any)], |e| {
        e.display("in changed");
        Ok(())
    });
    d.process("init", move |e| {
        e.display(&format!("in={}", e.get(inp)));
        e.set(r, &v("1"))?;
        e.nba(n, 0, v("0"), Time::ZERO)?;
        Ok(Wait::Done)
    });
    add(&mut b, d, &[dr, dw, dn]);
    assert_eq!(
        run(b, ns(1)).lines(),
        [
            "0 fs +0 out: in=1",
            "0 fs +1 vr=1",
            "0 fs +1 vw=Z",
            "0 fs +1 vn=X",
            "0 fs +2 vn=0",
        ]
    );
}

/// At 10 ns: a VHDL input event, two `#10` resumptions and a `<= #10` NBA in one activation,
/// each in its region.
#[test]
fn events_at_the_same_time() {
    let mut b = Builder::new();
    let vx = sig(&mut b, "vx", "0");
    let dx = b.driver(vx);
    let mut first = true;
    b.process(
        "top.x",
        ProcFlags::default(),
        &[dx],
        p(move |c| {
            if std::mem::replace(&mut first, false) {
                c.assign(dx, None, &[(ns(10), &v("1"))], DelayMode::INERTIAL)?;
            }
            Ok(())
        }),
    );
    let mut d = Domain::new();
    let (x, a, bb, c) = (d.net(v("0")), d.net(v("0")), d.net(v("0")), d.net(v("0")));
    d.input(vx, x);
    for (name, net) in [("a", a), ("b", bb)] {
        let mut step = 0;
        d.process(name, move |e| {
            step += 1;
            if step == 1 {
                return Ok(Wait::Delay(ns(10)));
            }
            e.set(net, &v("1"))?;
            Ok(Wait::Done)
        });
    }
    d.process("c", move |e| {
        e.nba(c, 0, v("1"), ns(10))?;
        Ok(Wait::Done)
    });
    let all = vec![
        (x, Edge::Any),
        (a, Edge::Any),
        (bb, Edge::Any),
        (c, Edge::Any),
    ];
    d.always("watch", all, move |e| {
        let s = [x, a, bb, c].map(|n| e.get(n).to_string()).concat();
        e.display(&format!("xabc={s}"));
        Ok(())
    });
    add(&mut b, d, &[]);
    assert_eq!(
        run(b, ns(20)).lines(),
        [
            "10 ns +0 vx=1",
            "10 ns +0 out: xabc=1000",
            "10 ns +0 out: xabc=1110",
            "10 ns +0 out: xabc=1111",
        ]
    );
}

fn ping_pong(limit: u32) -> Result<RunStatus, SimError> {
    let mut d = Domain::new();
    let (a, bn) = (d.net(v("X")), d.net(v("0")));
    d.always("ping", vec![(a, Edge::Any)], move |e| {
        let n = e.get(bn).not();
        e.set(bn, &n)
    });
    d.always("pong", vec![(bn, Edge::Any)], move |e| {
        let n = e.get(a).not();
        e.set(a, &n)
    });
    d.process("kick", move |e| {
        e.set(a, &v("1"))?;
        Ok(Wait::Done)
    });
    d.set_event_limit(limit);
    let mut b = Builder::new();
    add(&mut b, d, &[]);
    build(b, Config::default()).0.run_until(ns(1))
}

/// An Active loop never reaches the kernel's delta limit: the domain's own limit stops it with
/// an error naming the process and the time; `forever #0` loops too.
#[test]
fn zero_delay_loops_hit_the_event_limit() {
    let e = ping_pong(100).unwrap_err();
    assert_eq!(e.kind.code(), "E3009");
    let ErrorKind::Runtime(msg) = &e.kind else {
        panic!("{e}")
    };
    assert_eq!(
        msg,
        "Verilog process `ping` ran 100 times at time 0 fs without time advancing: a zero-delay loop (IEEE 1364-2005 5.4) never lets the time step end"
    );
    assert_eq!(e.process.as_ref().unwrap().1, "top.v");
    let mut d = Domain::new();
    d.process("spin", |_| Ok(Wait::Zero));
    d.set_event_limit(50);
    let mut b = Builder::new();
    add(&mut b, d, &[]);
    let e = build(b, Config::default()).0.run_until(ns(1)).unwrap_err();
    assert!(e.to_string().contains("`spin` ran 50 times"), "{e}");
}

/// A stop request (Ctrl-C) is seen inside the region loop: the run ends as requested.
#[test]
fn stop_request_inside_the_loop() {
    let intr = Arc::new(Mutex::new(None::<Interrupt>));
    let i2 = intr.clone();
    let mut d = Domain::new();
    let mut n = 0;
    d.process("spin", move |_| {
        n += 1;
        if n == 5000 {
            i2.lock().unwrap().as_ref().unwrap().stop();
        }
        Ok(Wait::Zero)
    });
    d.set_event_limit(u32::MAX);
    let mut b = Builder::new();
    add(&mut b, d, &[]);
    let (mut sim, _) = build(b, Config::default());
    *intr.lock().unwrap() = Some(sim.interrupt());
    assert_eq!(
        sim.run_until(ns(1)).unwrap(),
        RunStatus::Ended(EndReason::Requested)
    );
}

/// Verilog clock, counter and register chain; VHDL flip-flops in parallel sample them and feed
/// one back into the domain. The full trace is the same on one thread and on a pool.
fn mixed(cfg: Config) -> (Vec<String>, u64) {
    let mut b = Builder::new();
    let (vclk, vcnt, vback, vecho) = (
        sig(&mut b, "vclk", "U"),
        sig(&mut b, "vcnt", "UUUU"),
        sig(&mut b, "vback", "0000"),
        sig(&mut b, "vecho", "UUUU"),
    );
    let (dclk, dcnt, decho) = (b.driver(vclk), b.driver(vcnt), b.driver(vecho));
    let mut d = Domain::new();
    let (clk, cnt, back, echo) = (
        d.net(v("X")),
        d.net(v("0000")),
        d.net(v("0000")),
        d.net(v("XXXX")),
    );
    verilog_clock(&mut d, clk, 5);
    d.always("cnt", vec![(clk, Edge::Pos)], move |e| {
        let n = lv(num(e.get(cnt)) + 1, 4);
        e.display(&format!("cnt {n} back {}", e.get(back)));
        e.nba(cnt, 0, n, Time::ZERO)
    });
    d.assign("echo", &[back], move |e| {
        let n = lv(num(e.get(back)) ^ 5, 4);
        e.set(echo, &n)
    });
    d.input(vback, back);
    d.output(clk, dclk);
    d.output(cnt, dcnt);
    d.output(echo, decho);
    add(&mut b, d, &[dclk, dcnt, decho]);
    vhdl_flop(&mut b, "top.back", vclk, vcnt, vback);
    let mut prev = vcnt;
    for k in 0..8 {
        let s = sig(&mut b, &format!("top.s{k}"), "UUUU");
        vhdl_flop(&mut b, &format!("top.ff{k}"), vclk, prev, s);
        prev = s;
    }
    let (mut sim, t) = build(b, cfg);
    sim.run_until(ns(200)).unwrap();
    (t.lines(), sim.stats().pooled_batches)
}

#[test]
fn threads_give_the_same_trace() {
    let (one, _) = mixed(Config::default());
    let (pooled, n) = mixed(Config {
        threads: 4,
        pool_all: true,
        ..Config::default()
    });
    assert!(n > 0, "the pool never ran");
    assert!(one.len() > 200, "{}", one.len());
    assert!(one.iter().any(|l| l.contains("vecho=")));
    assert_eq!(one, pooled);
    // The kernel settings of a plain `yavs run` (unchanged assignments skipped) and a shuffled
    // execution order change nothing either.
    let (run_like, _) = mixed(Config {
        skip_unchanged: true,
        shuffle: Some(7),
        ..Config::default()
    });
    assert_eq!(one, run_like);
}
