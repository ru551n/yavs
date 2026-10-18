//! The differential corpus through sem, lowering, the IR verifier, the interpreter and the
//! kernel, compared with the NVC reference outputs (`tests/corpus/**/*.expected`, canonical
//! form of `tools/difftest/README.md`).
//!
//! Each file is elaborated with yavs-elab (which lowers through this crate) and run on the
//! kernel, the same path as `yavs run`. Files listed in `PASS` must match their `.expected`
//! exactly; the others are printed with their difference or diagnostic.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use yavs_diag::{MessageFormat, render};
use yavs_kernel::{Config, EndReason, Hook, Observer, Report, RunStatus, Severity, Time};
use yavs_lower::{LowerOptions, Lowerer};
use yavs_sem::hir::UnitHir;
use yavs_sem::{Db, LangRev, Options};

/// Corpus files whose output matches NVC's exactly.
const PASS: &[&str] = &[
    "access/access_string_array",
    "access/linked_list",
    "access/null_deref",
    "arith/div_zero",
    "arith/int_ops",
    "arith/int_overflow",
    "arith/real_arith",
    "arith/real_to_int_range",
    "arith/time_arith",
    "composite/arrays_aggregates",
    "composite/records",
    "composite/slv_register",
    "composite/strings_ops",
    "composite/unconstrained_records",
    "delta/concurrent_assign",
    "delta/delta_chain",
    "delta/delta_order",
    "delta/event_attrs",
    "delta/signal_update_order",
    "env/finish_in_function_call_order",
    "env/finish_noarg",
    "env/finish_status",
    "env/stop_status",
    "env/stop_time_limit",
    "force/force_drivers",
    "force/force_in",
    "force/force_out_port",
    "generics/for_generate",
    "generics/generic_map",
    "generics/if_case_generate",
    "generics/package_generics",
    "hierarchy/component_binding",
    "hierarchy/instance_path_names",
    "hierarchy/nested_instances",
    "hierarchy/port_modes_conversion",
    "postponed/postponed_basic",
    "postponed/postponed_wait",
    "process/case_if",
    "process/loops_exit_next",
    "process/sensitivity",
    "process/subprograms",
    "process/wait_forms",
    "protected/counter",
    "protected/generic_package_prot",
    "protected/stack",
    "reflect/arrays",
    "reflect/multidim",
    "reflect/scalars",
    "report/assert_user_message",
    "report/concurrent_assert",
    "report/error_then_end",
    "report/failure_exit",
    "report/message_forms",
    "resolution/bus_guarded",
    "resolution/custom_resolution",
    "resolution/resolved_record_port",
    "resolution/std_logic_drivers",
    "resolution/std_logic_table",
    "run/case_forms",
    "run/cond_2019",
    "run/expr_forms",
    "run/sig_arrays",
    "run/sig_attrs",
    "run/sig_out",
    "run/sig_params",
    "scalar/array_attrs",
    "scalar/enum_attrs",
    "scalar/int_attrs",
    "scalar/real_attrs",
    "scalar/real_image",
    "scalar/string_attrs",
    "scalar/subtype_range",
    "scalar/time_attrs",
    "sigassign/assign_inertial",
    "sigassign/assign_reject",
    "sigassign/assign_transport",
    "sigassign/pulse_swallow",
    "sigassign/waveforms",
    "textio/file_roundtrip",
    "textio/std_logic_textio",
    "textio/write_read_line",
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
    "generics",
    "hierarchy",
    "composite",
    "access",
    "protected",
    "resolution",
    "force",
    "textio",
    "run",
    "reflect",
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

/// (severity, time in fs, delta, process index, message).
type Entry = (Severity, u64, u32, usize, String);

struct Reports(Arc<Mutex<Vec<Entry>>>, Arc<Mutex<Vec<u8>>>);

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

    fn output(&mut self, _hook: &mut Hook<'_>, _p: yavs_kernel::ProcId, bytes: &[u8]) {
        self.1.lock().unwrap().extend_from_slice(bytes);
    }
}

fn escape(s: &str) -> String {
    s.replace('\\', "\\\\")
        .replace('\n', "\\n")
        .replace('\t', "\\t")
}

/// Analyze, elaborate (yavs-elab, which lowers through this crate) and run `file`; its
/// canonical output, or the rendered diagnostics.
fn simulate(file: &Path) -> Result<String, String> {
    let src = std::fs::read_to_string(file).unwrap();
    let (top, rev, stop) = header(&src);
    // The checkout's own lib/ tree, not `default_stdlib()` (#7).
    let lib = PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../lib"));
    let mut opts = Options::new(rev, lib);
    let dir = std::env::temp_dir().join(format!("yavs-lower-work-{}", std::process::id()));
    opts.libraries
        .add_spec(&format!("work:{}", dir.display()))
        .unwrap();
    let mut db = Db::new(opts);
    let r = db.analyze_file("work", file, rev);
    let text = |db: &Db, ds: &[yavs_diag::Diagnostic]| -> String {
        ds.iter()
            .map(|d| render(&db.sources, d, MessageFormat::Short, false))
            .collect()
    };
    if r.is_err() || db.error_count() > 0 {
        return Err(format!("analysis: {}", text(&db, db.diagnostics())));
    }
    let mut e = yavs_elab::find_top(&mut db, "work", &top)
        .and_then(|t| yavs_elab::elaborate(&mut db, t, &[], &yavs_elab::ElabConfig::default()))
        .map_err(|d| text(&db, &d))?;
    e.lower(&mut db).map_err(|d| text(&db, &d))?;
    let names: Vec<String> = e
        .procs
        .iter()
        .map(|p| {
            let n = e.hier.path_name(p.scope, yavs_elab::Case::Lrm);
            n.strip_suffix(':').unwrap_or(&n).to_string()
        })
        .collect();
    let mut sim = e.build(Config::default()).map_err(|e| e.to_string())?;
    let reps = Arc::new(Mutex::new(Vec::new()));
    let output = Arc::new(Mutex::new(Vec::new()));
    sim.subscribe(Box::new(Reports(reps.clone(), output.clone())));
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
                yavs_kernel::ErrorKind::Runtime(m) => m.clone(),
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
    let text = String::from_utf8_lossy(&output.lock().unwrap()).into_owned();
    for line in text.lines() {
        writeln!(out, "out\t{line}").unwrap();
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
            // `tools/difftest/difftest.py --corpus crates/yavs-lower/tests/vhdl/run --bless`).
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
            let got = simulate(&f);
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
    let dir = std::env::temp_dir().join(format!("yavs-lower-work-{}", std::process::id()));
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
    assert!(flags("noisy").contains(yavs_ir::FuncFlags::MAY_SUSPEND));
    assert!(flags("fib").contains(yavs_ir::FuncFlags::PURE));
    assert!(!p.cov.is_empty());
}
