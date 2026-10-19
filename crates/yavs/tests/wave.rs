//! `yavs run --wave` (docs/cli-contract.md 5 and 6): every FST is read back with wellen.

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

fn scratch(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("yavs-wave-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("tmp");
    d
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
    if refs.is_empty() {
        return s; // wellen cannot load signals of a file without changes
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

fn design(name: &str) -> PathBuf {
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
        assert_eq!(int("nat"), (Some("natural".into()), Some(31)));
        assert_eq!(int("sm"), (Some("small_t".into()), Some(8)));
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
        "0=0001 10000000=0010 20000000=0011 30000000=0100",
        "natural range 0 to 15: 4 bits, the value at the end of each step"
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

/// Ctrl-C ends the run between cycles and leaves a finished file.
#[cfg(unix)]
#[test]
fn wave_sigint() {
    let d = scratch("sigint");
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
    let child = yavs()
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
    std::thread::sleep(std::time::Duration::from_millis(500));
    let kill = Command::new("kill")
        .args(["-INT", &child.id().to_string()])
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
    let mut args = vec![format!("--wave={}", fst.display()), format!("--std={std}")];
    if let Some(t) = field("stop") {
        args.push(format!("--stop-time={t}"));
    }
    args.push(top.to_string());
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let out = run(&d, &args);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        !stderr.contains("E0801") && !stderr.contains("internal compiler error"),
        "{f:?}: {stderr}"
    );
    if !fst.exists() {
        // Elaboration not supported yet (exit 2, no simulation).
        assert_eq!(out.status.code(), Some(2), "{f:?}: {stderr}");
        return false;
    }
    dump(&fst);
    let _ = std::fs::remove_dir_all(&d);
    true
}
