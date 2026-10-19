//! Debug hooks: conditional line breakpoints with hit counts, stepping over and into a
//! procedure, frame and local inspection, a global step across processes, signal
//! breakpoints and the process table.

mod common;

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use common::*;
use yavs_interp::*;
use yavs_ir::*;
use yavs_kernel::*;

type Script = Box<dyn Fn(&Stop<'_>, usize) -> Resume + Send + Sync>;

/// Records every stop as text and answers from a script.
struct Front {
    log: Arc<Mutex<Vec<String>>>,
    n: AtomicUsize,
    script: Script,
}

impl Debugger for Front {
    fn on_stop(&self, s: &Stop<'_>, session: &DebugSession) -> Resume {
        let f = &s.frames;
        let top = f.depth() - 1;
        let locals: Vec<String> = (0..f.depth())
            .flat_map(|i| {
                f.locals(i)
                    .into_iter()
                    .map(move |l| (i, l))
                    .collect::<Vec<_>>()
            })
            .map(|(i, l)| {
                let v = f.format(l.ty, &l.bytes).unwrap_or_default();
                format!("{}.{}={v}", f.func_name(i).unwrap_or("?"), l.name)
            })
            .collect();
        let procs = session.processes();
        let line = format!(
            "{:?} s{} {} depth {} in {} at {:?}+{:?} [{}]",
            s.reason,
            s.stmt.0,
            procs[s.process].name,
            f.depth(),
            f.func_name(top).unwrap_or("?"),
            s.now.map(|t| t.to_string()),
            s.delta,
            locals.join(" ")
        );
        self.log.lock().unwrap().push(line);
        let n = self.n.fetch_add(1, Ordering::Relaxed);
        (self.script)(s, n)
    }
}

fn session(prog: &Program, script: Script) -> (Arc<DebugSession>, Arc<Mutex<Vec<String>>>) {
    let log = Arc::new(Mutex::new(Vec::new()));
    let front = Front {
        log: log.clone(),
        n: AtomicUsize::new(0),
        script,
    };
    (DebugSession::new(prog, Box::new(front)), log)
}

/// Runs the design serially with every process attached to `s`.
fn run(d: &Design, s: &Arc<DebugSession>, limit: Time) -> (SimControl, Vec<String>) {
    let (mut sim, t, _) = d.build_with(1, |p, name| p.with_debug(s.clone(), name));
    let r = sim.run_until(limit);
    assert!(r.is_ok(), "{r:?}");
    let lines = t.0.lock().unwrap().clone();
    (sim, lines)
}

/// `for i in 1 to 5 loop report i'image; wait for 1 ns; end loop;` with a breakpoint on the
/// report, condition `i >= 3`, firing on its second hit.
#[test]
fn line_breakpoint_with_condition_and_hits() {
    let mut pb = ProgramBuilder::new();
    let int = integer(&mut pb);
    let p = pb.declare("p", FuncKind::Process, None);
    let cond = pb.declare("cond", FuncKind::Function, Some(p));
    let s_report;
    let i;
    {
        let mut fb = pb.define(p);
        i = fb.local("i", int, 8, 8);
        let (lp, done) = (fb.block(), fb.block());
        stmt(&mut fb, StmtKind::VarAssign);
        let one = fb.int(1);
        store(&mut fb, AddrSpace::Frame, i, one, Width::B8);
        fb.emit(Inst::Br { to: lp });
        fb.at(lp);
        s_report = stmt(&mut fb, StmtKind::Report);
        let v = load(&mut fb, AddrSpace::Frame, i, Width::B8);
        report_image(&mut fb, "i = ", int, v);
        wait_for(&mut fb, NS);
        stmt(&mut fb, StmtKind::Loop);
        let v = load(&mut fb, AddrSpace::Frame, i, Width::B8);
        let one = fb.int(1);
        let v = bin(&mut fb, IntOp::Add, v, one);
        store(&mut fb, AddrSpace::Frame, i, v, Width::B8);
        let five = fb.int(5);
        let c = cmp(&mut fb, CmpOp::Le, v, five);
        fb.emit(Inst::CondBr { c, t: lp, f: done });
        fb.at(done);
        wait_forever(&mut fb);
        fb.finish();
    }
    {
        let mut fb = pb.define(cond);
        stmt(&mut fb, StmtKind::Return);
        let v = load(&mut fb, AddrSpace::Up(1), i, Width::B8);
        let three = fb.int(3);
        let c = cmp(&mut fb, CmpOp::Ge, v, three);
        fb.emit(Inst::Ret { v: Some(c) });
        fb.finish();
    }
    let mut d = Design::new(pb);
    d.procs = vec![ProcSpec {
        name: "p",
        func: p,
        drives: vec![],
    }];
    let (s, log) = session(&d.prog, Box::new(|_, _| Resume::Continue));
    let bp = s
        .set_breakpoint(s_report, LineBp::new(Some(cond), HitCond::Eq(2)))
        .unwrap();
    let (_, lines) = run(&d, &s, Time::ns(100));
    assert!(has(&lines, "4 ns +0 p0 Note: i = 5"), "{lines:#?}");
    let log = log.lock().unwrap().clone();
    assert_eq!(
        log,
        [format!(
            "Breakpoint {{ hits: 2 }} s{} p depth 1 in p at Some(\"3 ns\")+Some(0) [p.i=4]",
            s_report.0
        )]
    );
    assert_eq!(bp.hits(), 3, "the condition held at i = 3, 4 and 5");
}

/// Build `p: v := 7; q(42); report; wait;` and `q(x): report x; report;`.
fn call_design() -> (Design, [StmtId; 5]) {
    let mut pb = ProgramBuilder::new();
    let int = integer(&mut pb);
    let q = pb.declare("q", FuncKind::Procedure, None);
    let p = pb.declare("p", FuncKind::Process, None);
    let (q1, q2, a, b, c);
    {
        let mut fb = pb.define(q);
        let x = fb.param("x", Width::B8, ParamClass::Value, Mode::In, Some(int));
        fb.program();
        q1 = stmt(&mut fb, StmtKind::Report);
        let v = load(&mut fb, AddrSpace::Frame, x, Width::B8);
        report_image(&mut fb, "x = ", int, v);
        q2 = stmt(&mut fb, StmtKind::Report);
        report(&mut fb, "q done");
        stmt(&mut fb, StmtKind::Return);
        fb.emit(Inst::Ret { v: None });
        fb.finish();
    }
    {
        let mut fb = pb.define(p);
        let v = fb.local("v", int, 8, 8);
        a = stmt(&mut fb, StmtKind::VarAssign);
        let seven = fb.int(7);
        store(&mut fb, AddrSpace::Frame, v, seven, Width::B8);
        b = stmt(&mut fb, StmtKind::Call);
        let arg = fb.int(42);
        let args = fb.regs(&[arg]);
        fb.emit(Inst::Call {
            func: q,
            args,
            dst: None,
        });
        c = stmt(&mut fb, StmtKind::Report);
        report(&mut fb, "after call");
        wait_forever(&mut fb);
        fb.finish();
    }
    // The procedure's parameter is a local for the debugger too.
    let mut d = Design::new(pb);
    d.procs = vec![ProcSpec {
        name: "p",
        func: p,
        drives: vec![],
    }];
    (d, [a, b, c, q1, q2])
}

#[test]
fn step_over_a_procedure_call() {
    let (d, [_, b, c, _, _]) = call_design();
    let (s, log) = session(&d.prog, Box::new(|_, _| Resume::StepOver));
    s.set_breakpoint(b, LineBp::new(None, HitCond::Always));
    run(&d, &s, Time::ns(10));
    let log = log.lock().unwrap().clone();
    // Stops at the call, then at the next statement of the process, never inside q.
    assert!(log[0].starts_with(&format!("Breakpoint {{ hits: 1 }} s{} p depth 1", b.0)));
    assert!(
        log[1].starts_with(&format!("Step s{} p depth 1 in p", c.0)),
        "{log:#?}"
    );
    assert!(log.iter().all(|l| !l.contains("in q")), "{log:#?}");
}

#[test]
fn step_in_inspects_frames_and_locals_then_steps_out() {
    let (d, [_, b, c, q1, _]) = call_design();
    let script: Script = Box::new(|s, n| {
        if n == 1 {
            // Inside q: two frames, the call statement in p's frame.
            assert_eq!(s.frames.depth(), 2);
            assert_eq!(s.frames.func_name(0), Some("p"));
            assert_eq!(s.frames.func_name(1), Some("q"));
            return Resume::StepOut;
        }
        Resume::StepIn
    });
    let (s, log) = session(&d.prog, script);
    s.set_breakpoint(b, LineBp::new(None, HitCond::Always));
    run(&d, &s, Time::ns(10));
    let log = log.lock().unwrap().clone();
    assert_eq!(
        log[0],
        format!(
            "Breakpoint {{ hits: 1 }} s{} p depth 1 in p at Some(\"0 fs\")+Some(0) [p.v=7]",
            b.0
        )
    );
    assert_eq!(
        log[1],
        format!(
            "Step s{} p depth 2 in q at Some(\"0 fs\")+Some(0) [p.v=7 q.x=42]",
            q1.0
        )
    );
    assert!(
        log[2].starts_with(&format!("Step s{} p depth 1 in p", c.0)),
        "{log:#?}"
    );
}

/// A global step stops at the next statement of whichever process runs next.
#[test]
fn global_step_across_processes() {
    let mut pb = ProgramBuilder::new();
    let mut ids = Vec::new();
    let mut procs = Vec::new();
    for name in ["a", "b"] {
        let f = pb.declare(name, FuncKind::Process, None);
        let mut fb = pb.define(f);
        let s1 = stmt(&mut fb, StmtKind::Report);
        report(&mut fb, &format!("{name}1"));
        let s2 = stmt(&mut fb, StmtKind::Report);
        report(&mut fb, &format!("{name}2"));
        wait_forever(&mut fb);
        fb.finish();
        ids.push((s1, s2));
        procs.push(f);
    }
    let mut d = Design::new(pb);
    d.procs = vec![
        ProcSpec {
            name: "a",
            func: procs[0],
            drives: vec![],
        },
        ProcSpec {
            name: "b",
            func: procs[1],
            drives: vec![],
        },
    ];
    let (s, log) = session(&d.prog, Box::new(|_, _| Resume::GlobalStep));
    s.request_global_step();
    run(&d, &s, Time::ns(10));
    let stops: Vec<String> = log
        .lock()
        .unwrap()
        .iter()
        .map(|l| l.split(" depth").next().unwrap().to_string())
        .collect();
    let (a, b) = (ids[0], ids[1]);
    assert_eq!(
        stops,
        [
            format!("GlobalStep s{} a", a.0.0),
            format!("GlobalStep s{} a", a.1.0),
            format!("GlobalStep s{} a", a.1.0 + 1),
            format!("GlobalStep s{} b", b.0.0),
            format!("GlobalStep s{} b", b.1.0),
            format!("GlobalStep s{} b", b.1.0 + 1),
        ]
    );
}

/// A signal breakpoint pauses `SimControl` after the cycle in which `clk` becomes 1; the
/// process table then shows what each process waits on.
#[test]
fn signal_breakpoint_and_process_table() {
    let mut pb = ProgramBuilder::new();
    let clkgen = pb.declare("clkgen", FuncKind::Process, None);
    let watch = pb.declare("watch", FuncKind::Process, None);
    {
        let mut fb = pb.define(clkgen);
        stmt(&mut fb, StmtKind::SigAssign);
        let v = read_scalar(&mut fb, 0);
        let one = fb.int(1);
        let nv = bin(&mut fb, IntOp::Sub, one, v);
        drive(&mut fb, 0, 0, nv, Width::B1);
        wait_for(&mut fb, 5 * NS);
        fb.emit(Inst::Br { to: BlockId(0) });
        fb.finish();
    }
    {
        let mut fb = pb.define(watch);
        wait_on_loop(&mut fb, 0, BlockId(0));
        fb.finish();
    }
    let mut d = Design::new(pb);
    d.signals = vec![("clk", SignalType::bytes(1, 1), Value::Bytes(vec![0]))];
    d.procs = vec![
        ProcSpec {
            name: "clkgen",
            func: clkgen,
            drives: vec![(0, 1)],
        },
        ProcSpec {
            name: "watch",
            func: watch,
            drives: vec![],
        },
    ];
    let (s, _) = session(&d.prog, Box::new(|_, _| Resume::Continue));
    let (mut sim, _, _) = d.build_with(1, |p, name| p.with_debug(s.clone(), name));
    let brk = SignalBreak::new(sim.interrupt()).watch(SignalId(0), Some(Value::Bytes(vec![1])));
    let hits = brk.hits();
    sim.subscribe(Box::new(brk));
    assert_eq!(sim.run_until(Time::ns(100)), Ok(RunStatus::Paused));
    assert_eq!((sim.now(), sim.delta()), (Time::ZERO, 1));
    assert_eq!(sim.run_until(Time::ns(100)), Ok(RunStatus::Paused));
    assert_eq!((sim.now(), sim.delta()), (Time::ns(10), 1));
    assert_eq!(hits.lock().unwrap().len(), 2);
    let procs = s.processes();
    assert_eq!(procs[0].name, "clkgen");
    assert_eq!(procs[0].state, ProcState::Waiting);
    assert_eq!(procs[0].timeout, Some(Time::ns(15)));
    assert!(procs[0].sensitivity.is_empty());
    assert_eq!(procs[1].state, ProcState::Waiting);
    assert_eq!(procs[1].sensitivity, [SignalId(0).slice(0, 1)]);
    assert_eq!(procs[1].timeout, None);
    // Next time step and next delta are plain SimControl steps.
    assert_eq!(sim.step_timestep(), Ok(RunStatus::Paused));
    assert_eq!(sim.now(), Time::ns(15));
}
