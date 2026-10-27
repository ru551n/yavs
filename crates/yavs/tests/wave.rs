//! `yavs run --wave` (docs/cli-contract.md 5 and 6): every FST is read back with wellen.

mod common;
use common::tempdir as scratch;

use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use wellen::simple::read;
use wellen::{ScopeType, VarDirection, VarType};

fn yavs() -> Command {
    Command::new(env!("CARGO_BIN_EXE_yavs"))
}

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn analyze(dir: &Path, file: &Path, std: &str) -> bool {
    let work = format!("--work=work:{}", dir.join("lib").display());
    yavs()
        .args(["analyze", &format!("--std={std}"), &work])
        .arg(file)
        .status()
        .expect("yavs")
        .success()
}

fn run(dir: &Path, args: &[&str]) -> Output {
    let work = format!("--work=work:{}", dir.join("lib").display());
    yavs()
        .current_dir(dir)
        .args(["run", &work])
        .args(args)
        .output()
        .expect("yavs")
}

/// Hierarchy, types and every change of every variable, as text.
fn dump(path: &Path) -> String {
    let mut w = read(path).unwrap_or_else(|e| panic!("wellen cannot read {path:?}: {e}"));
    let h = w.hierarchy();
    let mut s = String::new();
    let vars: Vec<_> = h.all_vars().map(|r| &h[r]).collect();
    for v in &vars {
        writeln!(
            s,
            "{} {:?} {:?} {:?} {:?}",
            v.full_name(h),
            v.var_type(),
            v.direction(),
            v.length(h),
            v.vhdl_type_name(h)
        )
        .unwrap();
    }
    let refs: Vec<_> = vars.iter().map(|v| v.signal_ref()).collect();
    let names: Vec<_> = vars.iter().map(|v| v.full_name(h)).collect();
    if refs.is_empty() || w.time_table().is_empty() {
        // wellen cannot load signals of a file without changes (variables of a run that
        // failed before its first suspension)
        return s;
    }
    w.load_signals(&refs);
    let tt = w.time_table().to_vec();
    for (n, r) in names.iter().zip(&refs) {
        write!(s, "{n}:").unwrap();
        for (t, v) in w.get_signal(*r).expect("loaded").iter_changes() {
            let v = v.to_bit_string().unwrap_or_else(|| v.to_string());
            write!(s, " {}={v}", tt[t as usize]).unwrap();
        }
        s.push('\n');
    }
    s
}

/// The changes of the variable named `name` (`a.b.c`) in a dump.
fn changes<'a>(dump: &'a str, name: &str) -> &'a str {
    dump.lines()
        .find_map(|l| l.strip_prefix(&format!("{name}:")))
        .unwrap_or_else(|| panic!("no variable {name} in\n{dump}"))
        .trim()
}

fn design(name: &str) -> common::Tmp {
    let d = scratch(name);
    assert!(analyze(&d, &repo().join("tests/cli/wave.vhd"), "2008"));
    d
}

#[test]
fn waves_of_a_design() {
    let d = design("design");
    let fst = d.join("a.fst");
    let out = run(
        &d,
        &[
            &format!("--wave={}", fst.display()),
            "--wave-max-array=64",
            "wave_tb",
        ],
    );
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}");
    assert!(stderr.contains("warning[W0802]: `wave_tb.big` is not recorded"));

    let w = read(&fst).expect("wellen");
    {
        let h = w.hierarchy();
        let scope = |p: &[&str]| &h[h.lookup_scope(p).unwrap_or_else(|| panic!("{p:?}"))];
        let top = scope(&["wave_tb"]);
        assert_eq!(top.scope_type(), ScopeType::VhdlArchitecture);
        assert_eq!(top.component(h), Some("wave_tb(sim)"));
        assert_eq!(
            scope(&["wave_tb", "u1"]).component(h),
            Some("wave_sub(rtl)")
        );
        assert_eq!(
            scope(&["wave_tb", "g(1)"]).scope_type(),
            ScopeType::VhdlForGenerate
        );
        assert_eq!(
            scope(&["wave_tb", "rec"]).scope_type(),
            ScopeType::VhdlRecord
        );
        assert_eq!(scope(&["wave_tb", "mem"]).scope_type(), ScopeType::SvArray);
        assert_eq!(
            scope(&["wave_tb", "g(1)", "u"]).component(h),
            Some("wave_sub(rtl)"),
            "an instance in a generate"
        );
        let recs = ["wave_tb", "u2", "recs", "recs(1)"];
        assert_eq!(scope(&recs).scope_type(), ScopeType::VhdlRecord);
        let var = |p: &[&str], n: &str| &h[h.lookup_var(p, n).unwrap_or_else(|| panic!("{n}"))];
        let clk = var(&["wave_tb"], "Clk");
        assert_eq!(clk.var_type(), VarType::StdLogic, "resolved std_ulogic");
        let cin = var(&["wave_tb", "u1"], "Clk_In");
        assert_eq!(cin.direction(), VarDirection::Input);
        assert_eq!(
            var(&["wave_tb", "u1"], "q").direction(),
            VarDirection::Output
        );
        assert_eq!(
            var(&["wave_tb"], "count").vhdl_type_name(h),
            Some("UNSIGNED")
        );
        let (ename, lits) = var(&["wave_tb"], "state").enum_type(h).expect("enum table");
        assert_eq!(ename, "state_t");
        assert_eq!(
            lits,
            [("00", "IDLE"), ("01", "Run_State"), ("10", "DONE")],
            "original spelling"
        );
        assert_eq!(var(&["wave_tb"], "flag").var_type(), VarType::Boolean);
        assert_eq!(var(&["wave_tb"], "b").var_type(), VarType::Bit);
        assert_eq!(var(&["wave_tb"], "r").var_type(), VarType::Real);
        assert_eq!(var(&["wave_tb"], "t").var_type(), VarType::Time);
        assert!(h.lookup_var(&["wave_tb"], "big").is_none(), "over the cap");
        // Integers: the subtype's name, the width its range needs.
        let int = |n: &str| {
            let v = var(&["wave_tb"], n);
            (
                v.vhdl_type_name(h).map(str::to_ascii_lowercase),
                v.length(h),
            )
        };
        assert_eq!(int("i"), (Some("integer".into()), Some(32)));
        assert_eq!(int("nat"), (Some("natural".into()), Some(32)));
        assert_eq!(int("sm"), (Some("small_t".into()), Some(9)));
        assert_eq!(int("neg"), (Some("integer".into()), Some(4)), "-8 to 7");
        assert_eq!(var(&["wave_tb"], "t").length(h), Some(64));
        // The chosen alternative of a case or if generate keeps its label.
        assert!(h.lookup_scope(&["wave_tb", "cg.alt1"]).is_some());
        assert!(h.lookup_var(&["wave_tb", "cg.alt1"], "cgs").is_some());
        assert!(h.lookup_var(&["wave_tb", "ig.alt_b"], "igs").is_some());
        assert!(h.lookup_scope(&["wave_tb", "cg"]).is_none());
    }

    // One value per time step: the final one (FST has no deltas), so time 0 shows the
    // values after the first deltas, not the declared initial ones.
    let s = dump(&fst);
    let clk = "0=1 5000000=0 10000000=1 15000000=0 20000000=1 25000000=0 30000000=1 35000000=0";
    assert_eq!(changes(&s, "wave_tb.Clk"), clk);
    assert_eq!(
        changes(&s, "wave_tb.u1.Clk_In"),
        clk,
        "port: its own signal"
    );
    assert_eq!(changes(&s, "wave_tb.g(1).gs"), clk);
    assert_eq!(changes(&s, "wave_tb.g(0).gs"), "0=0");
    assert_eq!(changes(&s, "wave_tb.g(0).u.Clk_In"), "0=0");
    assert_eq!(changes(&s, "wave_tb.g(1).u.Clk_In"), clk);
    let count = "0=0001 10000000=0010 20000000=0011 30000000=0100";
    assert_eq!(changes(&s, "wave_tb.count"), count, "downto, left first");
    assert_eq!(changes(&s, "wave_tb.u1.q"), count);
    assert_eq!(changes(&s, "wave_tb.g(1).u.cnt"), count);
    assert_eq!(changes(&s, "wave_tb.g(0).u.cnt"), "0=0000");
    assert_eq!(
        changes(&s, "wave_tb.state"),
        "0=00 40000000=01 41000000=10",
        "deltas at 41 ns collapse to the final value"
    );
    assert_eq!(changes(&s, "wave_tb.flag"), "0=0 40000000=1");
    assert_eq!(changes(&s, "wave_tb.b"), "0=0 40000000=1");
    assert_eq!(
        changes(&s, "wave_tb.bv"),
        "0=001 40000000=110",
        "to, left first"
    );
    assert_eq!(changes(&s, "wave_tb.r"), "0=0 40000000=2.5");
    assert_eq!(
        changes(&s, "wave_tb.i"),
        format!("0={} 40000000={:032b}", "0".repeat(32), -7i32)
    );
    assert_eq!(
        changes(&s, "wave_tb.t"),
        format!("0={} 40000000={:064b}", "0".repeat(64), 3_000_000i64)
    );
    assert_eq!(changes(&s, "wave_tb.rec.a"), "0=u 40000000=x");
    assert_eq!(
        changes(&s, "wave_tb.rec.n"),
        format!("0={} 40000000={:032b}", "0".repeat(32), 42)
    );
    // wellen shows the elements of an array scope without the array's name.
    assert_eq!(
        changes(&s, "wave_tb.mem.(2)"),
        "0=00000000 40000000=10100101"
    );
    assert_eq!(changes(&s, "wave_tb.mem.(3)"), "0=00000000");
    assert_eq!(changes(&s, "wave_tb.grid.(1,1)"), "0=0");
    assert_eq!(changes(&s, "wave_tb.grid.(1,2)"), "0=0 40000000=1");
    assert_eq!(changes(&s, "wave_tb.u2.recs.recs(0).a"), "0=0");
    assert_eq!(changes(&s, "wave_tb.u2.recs.recs(1).a"), "0=1");
    assert_eq!(
        changes(&s, "wave_tb.u2.recs.recs(1).n"),
        format!("0={}", "1".repeat(32))
    );

    // --threads 4 records the same waves.
    let fst4 = d.join("t4.fst");
    let out = run(
        &d,
        &[
            &format!("--wave={}", fst4.display()),
            "--wave-max-array=64",
            "--threads=4",
            "wave_tb",
        ],
    );
    assert!(out.status.success());
    assert_eq!(dump(&fst4), s);
}

#[test]
fn wave_variables() {
    let d = design("variables");
    let fst = d.join("a.fst");
    let args = |f: &Path, extra: &[&str]| {
        let mut v = vec![format!("--wave={}", f.display())];
        v.extend(extra.iter().map(|s| s.to_string()));
        v.push("wave_tb".to_string());
        v
    };
    let go = |f: &Path, extra: &[&str]| {
        let a = args(f, extra);
        let a: Vec<&str> = a.iter().map(String::as_str).collect();
        let out = run(&d, &a);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    // Off by default.
    go(&fst, &[]);
    assert!(!dump(&fst).contains("main"));
    go(&fst, &["--wave-variables"]);
    let s = dump(&fst);
    assert_eq!(
        changes(&s, "wave_tb.main.cnt"),
        "0=00001 10000000=00010 20000000=00011 30000000=00100",
        "natural range 0 to 15: 5 bits (a zero sign bit), the value at the end of each step"
    );
    assert_eq!(
        changes(&s, "wave_tb.main.vs"),
        "0=00 40000000=01 41000000=10"
    );
    assert_eq!(changes(&s, "wave_tb.main.vr"), "0=0 40000000=2.5");
    assert_eq!(changes(&s, "wave_tb.main.vl"), "0=0 40000000=x");
    assert_eq!(
        changes(&s, "wave_tb.main.vi"),
        format!("0={} 40000000={:08b}", "0".repeat(8), -100i8),
        "-100 to 100: 8 bits"
    );
    {
        let w = read(&fst).expect("wellen");
        let h = w.hierarchy();
        let v = h.lookup_var(&["wave_tb", "main"], "cnt").expect("cnt");
        let v = &h[v];
        assert_eq!(
            v.vhdl_type_name(h).map(str::to_ascii_lowercase),
            Some("natural".into())
        );
        assert_eq!(
            h[h.lookup_scope(&["wave_tb", "main"]).expect("process scope")].scope_type(),
            ScopeType::VhdlProcess
        );
        let (ename, _) = h[h.lookup_var(&["wave_tb", "main"], "vs").expect("vs")]
            .enum_type(h)
            .expect("enum table");
        assert_eq!(ename, "state_t");
    }
    // The globs apply to variables, and the file does not depend on the thread count.
    let sel = d.join("sel.fst");
    go(
        &sel,
        &["--wave-variables", "--wave-include=wave_tb.main.v*"],
    );
    let names: Vec<String> = dump(&sel)
        .lines()
        .filter_map(|l| l.split_once(':').map(|(n, _)| n.to_string()))
        .collect();
    assert_eq!(
        names,
        [
            "wave_tb.main.vs",
            "wave_tb.main.vr",
            "wave_tb.main.vl",
            "wave_tb.main.vi"
        ]
    );
    let t4 = d.join("t4.fst");
    go(&t4, &["--wave-variables", "--threads=4"]);
    assert_eq!(dump(&t4), s);
}

#[test]
fn wave_globs() {
    let d = design("globs");
    let fst = d.join("a.fst");
    let out = run(
        &d,
        &[
            &format!("--wave={}", fst.display()),
            "--wave-include=WAVE_TB.u*",
            "--wave-include=wave_tb.clk",
            "--wave-exclude=**.q",
            "wave_tb",
        ],
    );
    assert!(out.status.success());
    let s = dump(&fst);
    let vars: Vec<&str> = s
        .lines()
        .filter_map(|l| l.split_once(':').map(|(n, _)| n))
        .collect();
    assert_eq!(
        vars,
        [
            "wave_tb.Clk",
            "wave_tb.u1.Clk_In",
            "wave_tb.u1.cnt",
            "wave_tb.u2.recs.recs(0).a",
            "wave_tb.u2.recs.recs(0).n",
            "wave_tb.u2.recs.recs(1).a",
            "wave_tb.u2.recs.recs(1).n",
        ]
    );
}

#[test]
fn wave_unwritable() {
    let d = design("unwritable");
    let out = run(&d, &["--wave=no/such/dir/a.fst", "wave_tb"]);
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("error[E0801]: cannot write the waveform file `no/such/dir/a.fst`"),
        "{stderr}"
    );
}

/// Blocks end by size only unless `YAVS_WAVE_FLUSH` is a duration (reproducible bytes by
/// default, #666); `never` is the default spelled out; another value is an error.
#[test]
fn wave_flush_never() {
    let d = design("flush");
    let go = |v: Option<&str>, f: &str| {
        let work = format!("--work=work:{}", d.join("lib").display());
        let mut c = yavs();
        c.current_dir(&*d).env_remove("YAVS_WAVE_FLUSH");
        if let Some(v) = v {
            c.env("YAVS_WAVE_FLUSH", v);
        }
        c.args(["run", &work, &format!("--wave={f}"), "wave_tb"])
            .output()
            .expect("yavs")
    };
    let bytes = |f: &str| std::fs::read(d.join(f)).unwrap();
    let ok = |o: &std::process::Output| assert!(o.status.success(), "{o:?}");
    // The default and `never` are the same, run after run.
    for (v, f) in [(None, "a.fst"), (None, "b.fst"), (Some("never"), "n.fst")] {
        ok(&go(v, f));
    }
    assert_eq!(bytes("a.fst"), bytes("b.fst"));
    assert_eq!(bytes("a.fst"), bytes("n.fst"));
    // A duration is accepted and the file still reads back with the same changes.
    for (v, f) in [("0s", "z.fst"), ("500ms", "m.fst"), ("5s", "s.fst")] {
        ok(&go(Some(v), f));
        assert_eq!(dump(&d.join(f)), dump(&d.join("a.fst")), "{v}");
    }
    for v in ["5", "s", "-1s", "1.5s", "5m"] {
        assert_eq!(go(Some(v), "x.fst").status.code(), Some(2), "{v}");
    }
    let bad = go(Some("sometimes"), "c.fst");
    assert_eq!(bad.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&bad.stderr);
    assert!(
        stderr.contains("error[E0801]") && stderr.contains("YAVS_WAVE_FLUSH=sometimes"),
        "{stderr}"
    );
}

/// Ctrl-C ends the run between cycles and leaves a finished file, with the default (no
/// wall-clock flush) and with one.
#[cfg(unix)]
#[test]
fn wave_sigint() {
    wave_signal("sigint", "-INT", None);
    wave_signal("sigint-flush", "-INT", Some("1s"));
}

/// SIGTERM does the same (the file is finished, the run ends between cycles).
#[cfg(unix)]
#[test]
fn wave_sigterm() {
    wave_signal("sigterm", "-TERM", None);
}

#[cfg(unix)]
fn wave_signal(tag: &str, sig: &str, flush: Option<&str>) {
    let d = scratch(tag);
    let src = d.join("clock.vhd");
    std::fs::write(
        &src,
        "entity clock is end entity;\n\
         architecture a of clock is signal c : bit; begin\n\
         c <= not c after 5 ns;\n\
         end architecture;\n",
    )
    .unwrap();
    assert!(analyze(&d, &src, "2008"));
    let fst = d.join("a.fst");
    let mut cmd = yavs();
    cmd.env_remove("YAVS_WAVE_FLUSH");
    if let Some(v) = flush {
        cmd.env("YAVS_WAVE_FLUSH", v);
    }
    let child = cmd
        .args(["run", &format!("--work=work:{}", d.join("lib").display())])
        .arg(format!("--wave={}", fst.display()))
        .arg("clock")
        .stdout(std::process::Stdio::piped())
        .spawn()
        .expect("yavs");
    // The handler is installed right after the file is created.
    for _ in 0..600 {
        if fst.exists() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    std::thread::sleep(std::time::Duration::from_secs(2));
    let kill = Command::new("kill")
        .args([sig, &child.id().to_string()])
        .status()
        .expect("kill");
    assert!(kill.success());
    let out = child.wait_with_output().expect("wait");
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("simulation interrupted"), "{stdout}");
    let s = dump(&fst);
    assert!(changes(&s, "clock.c").split(' ').count() > 10, "{s}");
}

/// Every corpus testbench that simulates writes an FST that wellen reads in full (#129).
#[test]
fn corpus_waves() {
    let root = repo().join("tests/corpus");
    let mut files = Vec::new();
    for g in std::fs::read_dir(&root).unwrap() {
        let g = g.unwrap().path();
        if g.is_dir() {
            for f in std::fs::read_dir(&g).unwrap() {
                let f = f.unwrap().path();
                if f.extension().is_some_and(|e| e == "vhd") {
                    files.push(f);
                }
            }
        }
    }
    files.sort();
    // Four at a time: each run is a separate process.
    let next = std::sync::atomic::AtomicUsize::new(0);
    let results: Vec<(usize, bool)> = std::thread::scope(|sc| {
        let workers: Vec<_> = (0..4)
            .map(|_| {
                sc.spawn(|| {
                    let mut done = Vec::new();
                    loop {
                        let i = next.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                        let Some(f) = files.get(i) else { break done };
                        done.push((i, corpus_wave(f)));
                    }
                })
            })
            .collect();
        workers
            .into_iter()
            .flat_map(|w| w.join().expect("worker"))
            .collect()
    });
    let read_back = results.iter().filter(|r| r.1).count();
    let not_run: Vec<_> = results
        .iter()
        .filter(|r| !r.1)
        .map(|r| &files[r.0])
        .collect();
    eprintln!("corpus waves: {read_back} read back, not simulated: {not_run:?}");
    assert!(
        read_back * 10 >= files.len() * 8,
        "{read_back} of {}",
        files.len()
    );
}

/// Simulate one corpus file with `--wave` and read the file back; false if it does not
/// simulate yet.
fn corpus_wave(f: &Path) -> bool {
    let text = std::fs::read_to_string(f).unwrap();
    let header = text
        .lines()
        .take(15)
        .find_map(|l| l.strip_prefix("-- difftest:"))
        .unwrap_or_else(|| panic!("{f:?}: no header"));
    let field = |k: &str| {
        header
            .split_whitespace()
            .find_map(|w| w.strip_prefix(&format!("{k}=")))
    };
    let top = field("top").expect("top");
    let std = field("std").unwrap_or("2008");
    let d = scratch(&format!("c-{}", f.file_stem().unwrap().to_string_lossy()));
    if !matches!(std, "2008" | "2019") || !analyze(&d, f, std) {
        return false;
    }
    let fst = d.join("a.fst");
    let mut args = vec![
        format!("--wave={}", fst.display()),
        "--wave-variables".to_string(),
        format!("--std={std}"),
    ];
    if let Some(t) = field("stop") {
        args.push(format!("--stop-time={t}"));
    }
    args.push(top.to_string());
    let mut args: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = run(&d, &args);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("E0801") && !stderr.contains("internal compiler error"),
        "{f:?}: {stderr}"
    );
    // A crash (a signal, a stack overflow) leaves an unfinished file: name it (#731).
    assert!(
        matches!(out.status.code(), Some(0..=125)),
        "{f:?}: the run ended with {:?}\n{stderr}",
        out.status
    );
    if !fst.exists() {
        // Elaboration not supported yet (exit 2, no simulation).
        assert_eq!(out.status.code(), Some(2), "{f:?}: {stderr}");
        return false;
    }
    // A file that cannot be read says how the run ended (#731).
    if let Err(e) = read(&fst) {
        let dir: Vec<_> = std::fs::read_dir(&d)
            .map(|r| {
                r.flatten()
                    .map(|f| (f.file_name(), f.metadata().map(|m| m.len())))
                    .collect()
            })
            .unwrap_or_default();
        panic!(
            "{f:?}: wellen: {e:?}; run {:?}; open {:?}; dir {dir:?}\nstderr: {stderr}\nstdout: {}",
            out.status,
            std::fs::File::open(&fst).map(|_| ()),
            String::from_utf8_lossy(&out.stdout)
        );
    }
    let collapsed = dump(&fst);
    // Ports collapsed into their actuals (the default) show the same waves as ports kept
    // apart (#413): the groups with port hierarchies.
    let group = f.parent().and_then(Path::file_name).unwrap_or_default();
    if group == "hierarchy" || group == "force" {
        let apart = d.join("b.fst");
        let wave = format!("--wave={}", apart.display());
        args[0] = &wave;
        args.insert(1, "--no-collapse-ports");
        let out = run(&d, &args);
        assert!(
            apart.exists(),
            "{f:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            collapsed,
            dump(&apart),
            "{f:?}: waves differ with --no-collapse-ports"
        );
    }
    let _ = std::fs::remove_dir_all(&d);
    true
}

/// Composite, shared and generic-sized variables (#321): records and arrays are scopes like
/// signals, shared variables sit in their region or package, and a range that depends on a
/// generic gives the width of its elaborated range.
#[test]
fn wave_composite_and_shared_variables() {
    let d = scratch("varsx");
    let work = format!("--work=work:{}", d.join("lib").display());
    let ok = yavs()
        .args(["analyze", "--std=2008", "--relaxed", &work])
        .arg(repo().join("tests/cli/wave_vars.vhd"))
        .status()
        .expect("yavs")
        .success();
    assert!(ok);
    let go = |f: &Path, extra: &[&str]| -> String {
        let wave = format!("--wave={}", f.display());
        let mut a = vec!["--relaxed", wave.as_str(), "--wave-variables"];
        a.extend_from_slice(extra);
        a.push("wave_vars_tb");
        let out = run(&d, &a);
        let err = String::from_utf8_lossy(&out.stderr).to_string();
        assert!(out.status.success(), "{err}");
        err
    };
    let fst = d.join("a.fst");
    let err = go(&fst, &[]);
    let s = dump(&fst);
    // `0 to DEPTH - 1` with DEPTH = 16: 5 bits; `DEPTH / 4 - 1 downto 0`: 4 elements.
    assert!(
        s.contains("wave_vars_tb.u.main.cnt Integer Implicit Some(5)"),
        "{s}"
    );
    assert_eq!(
        changes(&s, "wave_vars_tb.u.main.cnt"),
        "0=00001 10000000=00010 20000000=00011"
    );
    assert_eq!(
        changes(&s, "wave_vars_tb.u.main.vec"),
        "0=1111 10000000=0000 20000000=1111"
    );
    assert_eq!(
        changes(&s, "wave_vars_tb.u.main.bv"),
        "0=010 10000000=100 20000000=001"
    );
    // A record is a scope with its fields, an array one with its elements.
    assert_eq!(changes(&s, "wave_vars_tb.u.main.pr.a"), "0=1");
    assert_eq!(
        changes(&s, "wave_vars_tb.u.main.pr.n"),
        "0=0001 10000000=0010 20000000=0011"
    );
    assert_eq!(
        changes(&s, "wave_vars_tb.u.main.ints.(2)"),
        format!("0={:032b} 20000000={:032b}", 10, 30)
    );
    assert_eq!(
        changes(&s, "wave_vars_tb.u.main.s.(1)"),
        format!("0={:08b} 10000000={:08b} 20000000={:08b}", b'b', b'c', b'd')
    );
    // Large composites are compared before they are decoded: a write that stores the value
    // already there records nothing.
    assert_eq!(
        changes(&s, "wave_vars_tb.u.main.big.(1)"),
        format!("0={:032b}", 0)
    );
    assert_eq!(
        changes(&s, "wave_vars_tb.u.main.big.(7)"),
        format!("0={:032b} 10000000={:032b}", 0, 77)
    );
    assert_eq!(
        changes(&s, "wave_vars_tb.u.tab.(3)"),
        format!("0={:032b} 20000000={:032b}", 5, 6)
    );
    // Logic arrays nest: an array of vectors, an array of booleans (one bit-plane element each),
    // a record of vectors, booleans, reals, bit vectors and times.
    assert_eq!(changes(&s, "wave_vars_tb.u.main.sa.(0)"), "0=101");
    assert_eq!(
        changes(&s, "wave_vars_tb.u.main.sa.(1)"),
        "0=111 20000000=010"
    );
    assert_eq!(changes(&s, "wave_vars_tb.u.main.ba.(1)"), "0=0");
    assert_eq!(changes(&s, "wave_vars_tb.u.main.ba.(2)"), "0=1 20000000=0");
    assert_eq!(changes(&s, "wave_vars_tb.u.main.r2.v"), "0=1010");
    assert_eq!(changes(&s, "wave_vars_tb.u.main.r2.b"), "0=1");
    assert_eq!(changes(&s, "wave_vars_tb.u.main.r2.r"), "0=1.5");
    assert_eq!(
        changes(&s, "wave_vars_tb.u.main.r2.arr"),
        "0=101 20000000=110"
    );
    assert_eq!(
        changes(&s, "wave_vars_tb.u.main.r2.t"),
        format!("0={:064b}", 5_000_000)
    );
    // Shared variables: an architecture's in its instance, a package's in its package.
    assert_eq!(
        changes(&s, "wave_vars_tb.u.acc"),
        format!("0={:032b} 10000000={:032b} 20000000={:032b}", 1, 3, 6)
    );
    assert_eq!(
        changes(&s, "work.wv_pkg.pcount"),
        format!("0={:032b} 10000000={:032b} 20000000={:032b}", 3, 4, 5)
    );
    {
        let w = read(&fst).expect("wellen");
        let h = w.hierarchy();
        let pr = h
            .lookup_scope(&["wave_vars_tb", "u", "main", "pr"])
            .expect("record scope");
        assert_eq!(h[pr].scope_type(), ScopeType::VhdlRecord);
        h.lookup_var(&["wave_vars_tb", "u"], "acc")
            .expect("shared variable");
    }
    // A protected object and a line are not recorded; a variable whose bounds call an
    // impure function is named in a warning.
    assert!(!s.contains("cell") && !s.contains("main.l "), "{s}");
    assert!(
        err.contains("`wave_vars_tb.u.main.dyn` is not recorded")
            && err.contains("known only when the simulation runs"),
        "{err}"
    );
    // The globs and the size cap apply to variables.
    let sel = d.join("sel.fst");
    let err = go(
        &sel,
        &[
            "--wave-exclude=**.pr",
            "--wave-max-array=3",
            "--wave-include=**.main.*",
        ],
    );
    let names: Vec<String> = dump(&sel)
        .lines()
        .filter_map(|l| l.split_once(':').map(|(n, _)| n.to_string()))
        .collect();
    assert_eq!(
        names,
        [
            "wave_vars_tb.u.main.cnt",
            "wave_vars_tb.u.main.bv",
            "wave_vars_tb.u.main.ints.(1)",
            "wave_vars_tb.u.main.ints.(2)",
            "wave_vars_tb.u.main.sa.(0)",
            "wave_vars_tb.u.main.sa.(1)",
            "wave_vars_tb.u.main.ba.(1)",
            "wave_vars_tb.u.main.ba.(2)",
            "wave_vars_tb.u.main.ba.(3)",
            "wave_vars_tb.u.main.r2.b",
            "wave_vars_tb.u.main.r2.r",
            "wave_vars_tb.u.main.r2.arr",
            "wave_vars_tb.u.main.r2.t",
            "wave_vars_tb.u.main.s.(1)",
            "wave_vars_tb.u.main.s.(2)"
        ]
    );
    assert!(
        err.contains("`wave_vars_tb.u.main.vec` is not recorded"),
        "{err}"
    );
    // The same file at any thread count.
    let t4 = d.join("t4.fst");
    go(&t4, &["--threads=4"]);
    assert_eq!(dump(&t4), s);
}

/// `--wave` under `--dap` (#325): the debugger records the same file as a plain run.
#[test]
fn wave_under_dap() {
    use std::io::BufReader;
    use std::process::Stdio;
    let dir = scratch("dap");
    let src = repo().join("crates/yavs/tests/dap/tb.vhd");
    assert!(analyze(&dir, &src, "2008"));
    let plain = run(&dir, &["--wave=plain.fst", "tb"]);
    assert!(plain.status.success(), "{plain:?}");
    let mut child = yavs()
        .current_dir(&dir)
        .args(["run", &format!("--work=work:{}", dir.join("lib").display())])
        .args(["--wave=dap.fst", "--dap=stdio", "tb"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .spawn()
        .expect("yavs");
    let mut w = child.stdin.take().expect("stdin");
    let mut r = BufReader::new(child.stdout.take().expect("stdout"));
    let mut req = |seq: i64, command: &str| {
        let m =
            serde_json::json!({"seq": seq, "type": "request", "command": command, "arguments": {}});
        yavs_dap::write_message(&mut w, &m).expect("write");
    };
    req(1, "initialize");
    req(2, "launch");
    req(3, "configurationDone");
    while let Some(m) = yavs_dap::read_message(&mut r).expect("read") {
        if m["event"] == "terminated" {
            break;
        }
    }
    req(4, "disconnect");
    drop(w);
    assert!(child.wait().expect("wait").success());
    assert_eq!(dump(&dir.join("dap.fst")), dump(&dir.join("plain.fst")));
    let _ = std::fs::remove_dir_all(&dir);
}

/// #433: an enumeration whose literals are all logic characters is a logic scalar for the
/// writer; the recorder must hand it one position byte.
#[test]
fn wave_enum_of_logic_characters() {
    let d = scratch("logic_chars");
    let src = d.join("rm3.vhd");
    std::fs::write(
        &src,
        "entity rm3 is end;\narchitecture t of rm3 is\n  type lv is ('X', '0', '1', 'Z');\n  \
         signal a : lv := 'Z';\nbegin\n  process begin wait for 1 ns; a <= '1'; wait; end process;\nend;\n",
    )
    .expect("write");
    assert!(analyze(&d, &src, "2008"));
    let fst = d.join("a.fst");
    let out = run(&d, &[&format!("--wave={}", fst.display()), "rm3"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(changes(&dump(&fst), "rm3.a"), "0=z 1000000=1");
}

/// A port collapsed into its actual and the actual are one value chain in the FST (an alias),
/// and the file reads back with both names and the same changes.
#[test]
fn wave_collapsed_port_is_an_alias() {
    let d = scratch("alias");
    let src = d.join("al.vhd");
    std::fs::write(
        &src,
        "library ieee; use ieee.std_logic_1164.all;\n\
         entity sub is port (v : in std_logic_vector(3 downto 0); e : in boolean); end;\n\
         architecture rtl of sub is begin end;\n\
         library ieee; use ieee.std_logic_1164.all;\n\
         entity al is end;\narchitecture t of al is\n  \
         signal v : std_logic_vector(3 downto 0) := \"0000\"; signal e : boolean := false;\n\
         begin\n  u1: entity work.sub port map (v => v, e => e);\n  \
         process begin wait for 1 ns; v <= \"01ZX\"; e <= true; wait; end process;\nend;\n",
    )
    .expect("write");
    assert!(analyze(&d, &src, "2008"));
    let fst = d.join("a.fst");
    let out = run(&d, &[&format!("--wave={}", fst.display()), "al"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = dump(&fst);
    for (a, b) in [("al.v", "al.u1.v"), ("al.e", "al.u1.e")] {
        assert_eq!(changes(&text, a), changes(&text, b), "{a} and {b}");
    }
    assert_eq!(changes(&text, "al.v"), "0=0000 1000000=01zx");
    let w = read(&fst).expect("wellen");
    let h = w.hierarchy();
    let (va, vb) = (h.lookup_var(&["al"], "v"), h.lookup_var(&["al", "u1"], "v"));
    assert_eq!(
        h[va.expect("v")].signal_ref(),
        h[vb.expect("u1.v")].signal_ref()
    );
    let (ea, eb) = (h.lookup_var(&["al"], "e"), h.lookup_var(&["al", "u1"], "e"));
    assert_eq!(
        h[ea.expect("e")].signal_ref(),
        h[eb.expect("u1.e")].signal_ref()
    );
}

/// A non-negative integer range is written with a zero sign bit (two's complement readers,
/// #783): `natural range 0 to 31` holding 31 reads `011111`, never -1.
#[test]
fn wave_non_negative_integer_keeps_sign_bit() {
    let d = scratch("nonneg");
    let src = d.join("nn.vhd");
    std::fs::write(
        &src,
        "entity nn is end;\narchitecture a of nn is\n  \
         signal n : natural range 0 to 31 := 31; signal z : natural range 0 to 0 := 0;\n\
         begin\nend;\n",
    )
    .expect("write");
    assert!(analyze(&d, &src, "2008"));
    let fst = d.join("a.fst");
    let out = run(&d, &[&format!("--wave={}", fst.display()), "nn"]);
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let text = dump(&fst);
    assert_eq!(changes(&text, "nn.n"), "0=011111");
    assert_eq!(changes(&text, "nn.z"), "0=0");
}
