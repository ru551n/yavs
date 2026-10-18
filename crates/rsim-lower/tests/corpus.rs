//! The differential corpus through sem, lowering, the IR verifier, the interpreter and the
//! kernel, compared with the NVC reference outputs (`tests/corpus/**/*.expected`, canonical
//! form of `tools/difftest/README.md`).
//!
//! The elaborator is not there yet, so `run` below is a minimal stand-in for a top-level
//! architecture without ports or generics: it evaluates the instance constants and signal
//! initial values (thunks), creates scalar kernel signals and one interpreter process per
//! process. Files listed in `PASS` must match their `.expected` exactly; every other file must
//! still analyze, lower and verify or fail with a clear "not lowered" diagnostic.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use rsim_diag::{MessageFormat, render};
use rsim_interp::{Bindings, InstData, InstSig, InterpProcess, ProcDriver, Runtime};
use rsim_kernel::{
    Builder, Config, EndReason, Hook, Observer, ProcFlags, Report, RunStatus, Severity, SignalType,
    Time, Value,
};
use rsim_lower::{LowerOptions, Lowered, Lowerer, signal_elem, top_architecture};
use rsim_sem::hir::UnitHir;
use rsim_sem::loader::{UnitKey, UnitSlot};
use rsim_sem::{Db, LangRev, Options};

/// Corpus files whose output matches NVC's exactly.
const PASS: &[&str] = &[
    "scalar/enum_attrs",
    "scalar/int_attrs",
    "scalar/real_attrs",
    "scalar/real_image",
    "scalar/string_attrs",
    "scalar/subtype_range",
    "scalar/time_attrs",
    "arith/div_zero",
    "arith/int_ops",
    "arith/int_overflow",
    "arith/real_arith",
    "arith/time_arith",
    "process/case_if",
    "process/loops_exit_next",
    "process/sensitivity",
    "process/wait_forms",
    "report/assert_user_message",
    "report/concurrent_assert",
    "report/error_then_end",
    "report/failure_exit",
    "report/message_forms",
    "delta/concurrent_assign",
    "delta/delta_order",
    "delta/event_attrs",
    "delta/signal_update_order",
    "sigassign/assign_inertial",
    "sigassign/assign_reject",
    "sigassign/assign_transport",
    "sigassign/pulse_swallow",
    "sigassign/waveforms",
    "postponed/postponed_basic",
    "postponed/postponed_wait",
    "env/finish_noarg",
    "env/stop_status",
    "env/stop_time_limit",
    "run/sig_params",
];

const GROUPS: &[&str] = &[
    "scalar",
    "arith",
    "process",
    "report",
    "delta",
    "sigassign",
    "postponed",
    "env",
    "run",
];

fn corpus() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../tests/corpus"))
}

fn header(src: &str) -> (String, LangRev, Time) {
    let line = src
        .lines()
        .take(15)
        .find_map(|l| l.trim().strip_prefix("-- difftest:"))
        .expect("difftest header");
    let mut top = String::new();
    let mut rev = LangRev::Vhdl2008;
    let mut stop = Time::MAX;
    for kv in line.split_whitespace() {
        match kv.split_once('=') {
            Some(("top", v)) => top = v.to_string(),
            Some(("std", "2019")) => rev = LangRev::Vhdl2019,
            Some(("stop", v)) => {
                let ns = v.strip_suffix("ns").expect("stop in ns").parse().unwrap();
                stop = Time::ns(ns);
            }
            _ => {}
        }
    }
    (top, rev, stop)
}

/// Analyze and lower `file`; `Err` holds rendered diagnostics.
fn lower(file: &Path) -> Result<(Lowered, Vec<String>, Time), String> {
    let src = std::fs::read_to_string(file).unwrap();
    let (top, rev, stop) = header(&src);
    // The checkout's own lib/ tree, not `default_stdlib()` (a shared target dir can carry another
    // checkout's compile-time path).
    let lib = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../lib"));
    let mut opts = Options::new(rev, lib);
    let dir = std::env::temp_dir().join("rsim-lower-work");
    opts.libraries
        .add_spec(&format!("work:{}", dir.display()))
        .unwrap();
    let mut db = Db::new(opts);
    let r = db.analyze_file("work", file, rev);
    let text = |db: &Db, ds: &[rsim_diag::Diagnostic]| -> String {
        ds.iter()
            .map(|d| render(&db.sources, d, MessageFormat::Short, false))
            .collect()
    };
    if r.is_err() || db.error_count() > 0 {
        return Err(format!("analysis: {}", text(&db, db.diagnostics())));
    }
    // The elaborator loads the package bodies a design needs; load every one that exists.
    let pkgs: Vec<(String, String)> = db
        .units
        .iter()
        .filter(|(_, u)| matches!(u.hir, UnitHir::Package { .. }))
        // std has no package bodies: its subprograms are natives.
        .filter(|(_, u)| db.libs[u.lib].name.key.as_str() != "std")
        .filter(|(_, u)| {
            !matches!(
                db.libs[u.lib].units.get(&UnitKey::body(u.name.key)),
                Some(UnitSlot::Done(_))
            )
        })
        .map(|(_, u)| {
            (
                db.libs[u.lib].name.text.to_string(),
                u.name.text.to_string(),
            )
        })
        .collect();
    for (l, p) in pkgs {
        let _ = db.load_body(&l, &p);
    }
    if db.error_count() > 0 {
        return Err(format!("package bodies: {}", text(&db, db.diagnostics())));
    }
    let arch = top_architecture(&db, &top).ok_or("no architecture")?;
    let mut l = Lowerer::new(&db, LowerOptions::default());
    let u = l.architecture(arch).map_err(|d| text(&db, &d))?;
    assert_eq!(u, 0);
    let lowered = l.finish().map_err(|d| text(&db, &d))?;
    let names = lowered.units[0]
        .processes
        .iter()
        .enumerate()
        .map(|(i, p)| {
            let l = p.label.clone().unwrap_or_else(|| format!("_p{i}"));
            format!(":{}:{}", top.to_ascii_lowercase(), l.to_ascii_lowercase())
        })
        .collect();
    Ok((lowered, names, stop))
}

/// (severity, time in fs, delta, process index, message).
type Entry = (Severity, u64, u32, usize, String);

struct Reports(Arc<Mutex<Vec<Entry>>>);

impl Observer for Reports {
    fn report(&mut self, hook: &mut Hook<'_>, r: &Report) {
        self.0.lock().unwrap().push((
            r.severity,
            hook.now().as_fs(),
            hook.delta(),
            r.process.0 as usize,
            r.message.clone(),
        ));
    }
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
}

/// Run a lowered top level and return its canonical output.
fn run(l: Lowered, names: &[String], stop: Time) -> Result<String, String> {
    let prog = Arc::new(l.program);
    let u = &l.units[0];
    let rt = Runtime::new(prog.clone(), Bindings::default()).map_err(|e| e.to_string())?;
    let mut inst = InstData {
        data: vec![0; u.inst_size as usize],
        ..Default::default()
    };
    for c in &u.consts {
        let v = rsim_interp::call(rt.clone(), c.init, Arc::new(inst.clone()), &[])
            .map_err(|e| e.to_string())?
            .ok_or("constant thunk without value")?;
        let n = c.w.bytes() as usize;
        inst.data[c.off as usize..c.off as usize + n].copy_from_slice(&v.to_le_bytes()[..n]);
    }
    let mut b = Builder::new();
    for s in &u.signals {
        let v = rsim_interp::call(rt.clone(), s.init, Arc::new(inst.clone()), &[])
            .map_err(|e| e.to_string())?
            .ok_or("signal init without value")?;
        let elem = signal_elem(&prog, s.ty).ok_or("signal type")?;
        let ty = SignalType { elem, len: 1 };
        let init = match elem {
            rsim_kernel::Elem::Logic => {
                // BIT and BOOLEAN leaves hold the std_ulogic codes of '0' and '1'.
                let bias = match rsim_ir::Ty::logic_leaf(&prog.types, s.ty) {
                    Some(rsim_ir::LogicKind::StdULogic) => 0,
                    _ => 2,
                };
                let code = *rsim_logic::ALL.get(v as usize + bias).ok_or("logic code")?;
                Value::Logic(rsim_kernel::LogicVec::from_fn(1, |_| code))
            }
            rsim_kernel::Elem::Bytes(n) => Value::Bytes(v.to_le_bytes()[..n as usize].to_vec()),
        };
        let id = b.signal(&s.name, ty, &init);
        inst.sigs.push(InstSig { signal: id, ty });
    }
    let inst = Arc::new(inst);
    for (p, name) in u.processes.iter().zip(names) {
        let mut ids = Vec::new();
        let mut drivers = Vec::new();
        for &sig in &p.drivers {
            let s = inst.sigs[sig as usize];
            let id = b.driver(s.signal);
            ids.push(id);
            drivers.push(ProcDriver {
                id,
                sig,
                range: rsim_kernel::Range::new(0, s.ty.len),
            });
        }
        let proc = InterpProcess::new(rt.clone(), p.func, inst.clone(), drivers, None)
            .map_err(|e| e.to_string())?;
        let flags = ProcFlags {
            postponed: p.postponed,
            serial: false,
        };
        b.process(name, flags, &ids, proc);
    }
    let mut sim = b.build(Config::default()).map_err(|e| e.to_string())?;
    let reps = Arc::new(Mutex::new(Vec::new()));
    sim.subscribe(Box::new(Reports(reps.clone())));
    let status = sim.run_until(stop);
    let mut out = String::new();
    let mut code = 0;
    for (sev, t, d, p, m) in reps.lock().unwrap().iter() {
        let s = match sev {
            Severity::Note => "note",
            Severity::Warning => "warning",
            Severity::Error => "error",
            Severity::Failure => "failure",
        };
        if matches!(sev, Severity::Error | Severity::Failure) {
            code = 1;
        }
        writeln!(out, "diag\t{s}\t{t}\t{d}\t{}\t{}", names[*p], escape(m)).unwrap();
    }
    match status {
        Ok(RunStatus::Ended(r @ (EndReason::Stop(c) | EndReason::Finish(c)))) => {
            code = c;
            let k = if matches!(r, EndReason::Stop(_)) {
                "stop"
            } else {
                "finish"
            };
            let (t, d) = (sim.now().as_fs(), sim.delta());
            writeln!(out, "diag\t{k}\t{t}\t{d}\t-\t{c}").unwrap();
        }
        Ok(_) => {}
        Err(e) => {
            code = 1;
            let msg = match &e.kind {
                rsim_kernel::ErrorKind::Runtime(m) => m.clone(),
                k => k.to_string(),
            };
            writeln!(
                out,
                "diag\tfatal\t{}\t{}\t-\t{}",
                e.time.as_fs(),
                e.delta,
                escape(&msg)
            )
            .unwrap();
        }
    }
    Ok(format!("exit\t{code}\n{out}"))
}

#[test]
fn corpus_groups() {
    let mut summary = String::new();
    let mut failed = Vec::new();
    for g in GROUPS {
        let dir = match *g {
            // This crate's own runnable tests (`.expected` blessed from NVC with
            // `tools/difftest/difftest.py --corpus crates/rsim-lower/tests/vhdl/run --bless`).
            "run" => PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/tests/vhdl/run")),
            g => corpus().join(g),
        };
        let mut files: Vec<_> = std::fs::read_dir(dir)
            .unwrap()
            .map(|e| e.unwrap().path())
            .filter(|p| p.extension().is_some_and(|e| e == "vhd"))
            .collect();
        files.sort();
        for f in files {
            let name = format!("{g}/{}", f.file_stem().unwrap().to_string_lossy());
            let expected = std::fs::read_to_string(f.with_extension("expected")).unwrap();
            let got = lower(&f).and_then(|(l, n, stop)| run(l, &n, stop));
            let pass = matches!(&got, Ok(o) if o == &expected);
            let status = match &got {
                _ if pass => "PASS".to_string(),
                Ok(o) => format!("DIFF\n--- expected\n{expected}--- got\n{o}"),
                Err(e) => format!("FAIL {e}"),
            };
            writeln!(summary, "{name}: {status}").unwrap();
            if !pass && PASS.contains(&name.as_str()) {
                failed.push(name);
            }
        }
    }
    println!("{summary}");
    assert!(failed.is_empty(), "regressed: {failed:?}\n{summary}");
}

/// Every subprogram of a package body lowers and verifies; each instruction has a statement
/// with a source span (the verifier checks the ids, this checks the spans).
#[test]
fn package_subprograms() {
    let file = PathBuf::from(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/vhdl/pkg_scalar.vhd"
    ));
    let lib = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../lib"));
    let mut opts = Options::new(LangRev::Vhdl2008, lib);
    let dir = std::env::temp_dir().join("rsim-lower-work");
    opts.libraries
        .add_spec(&format!("work:{}", dir.display()))
        .unwrap();
    let mut db = Db::new(opts);
    db.analyze_file("work", &file, LangRev::Vhdl2008).unwrap();
    assert_eq!(db.error_count(), 0);
    let body = db
        .units
        .iter()
        .find(|(_, u)| matches!(u.hir, UnitHir::PackageBody { .. }))
        .map(|(id, _)| id)
        .unwrap();
    let mut l = Lowerer::new(&db, LowerOptions { coverage: true });
    let funcs = l.subprograms_of(body).unwrap();
    assert_eq!(funcs.len(), 5);
    let p = l.finish().unwrap().program;
    for (_, f) in p.funcs.iter() {
        for s in &f.loc {
            let info = p.stmts.get(*s).unwrap();
            assert!(info.span.end > info.span.start, "{}: empty span", f.name);
        }
    }
    // `noisy` waits, so it may suspend; `fib` is pure and recursive.
    let flags = |n: &str| p.funcs.values().iter().find(|f| f.name == n).unwrap().flags;
    assert!(flags("noisy").contains(rsim_ir::FuncFlags::MAY_SUSPEND));
    assert!(flags("fib").contains(rsim_ir::FuncFlags::PURE));
    assert!(!p.cov.is_empty());
}
