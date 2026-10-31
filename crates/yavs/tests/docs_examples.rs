//! The commands and examples of the user guide, on the files in `docs/examples`: what a page says
//! they print and how they end is what they do.

mod common;
use common::{Tmp, tempdir};

use std::path::{Path, PathBuf};
use std::process::Command;

fn docs() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs")
}

/// One scratch directory, with its own std/ieee store.
struct Env(Tmp);

/// Exit status, standard output, standard error.
type Out = (i32, String, String);

impl Env {
    fn new(name: &str) -> Env {
        Env(tempdir(name))
    }

    /// A copy of `docs/examples/<from>`, relative to the scratch directory.
    fn copy(&self, from: &str, files: &[&str]) {
        for f in files {
            let to = self.0.join(f);
            std::fs::create_dir_all(to.parent().unwrap()).unwrap();
            std::fs::copy(docs().join("examples").join(from).join(f), to).unwrap();
        }
    }

    fn yavs(&self, args: &[&str]) -> Out {
        let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
            .args(args)
            .current_dir(&*self.0)
            .env("YAVS_STD_CACHE", self.0.join("std"))
            .env("NO_COLOR", "1")
            .env_remove("YAVS_TOKENS")
            .output()
            .expect("run yavs");
        (
            o.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&o.stdout).into_owned(),
            String::from_utf8_lossy(&o.stderr).into_owned(),
        )
    }

    fn ok(&self, args: &[&str]) -> String {
        let (code, out, err) = self.yavs(args);
        assert_eq!(code, 0, "yavs {args:?}\n{out}\n{err}");
        out
    }
}

/// Every line of `out` is on the page, as the page shows it.
fn on_page(page: &str, out: &str) {
    let text = std::fs::read_to_string(docs().join(page)).unwrap();
    for line in out.lines().filter(|l| !l.trim().is_empty()) {
        assert!(
            text.lines().any(|l| l.trim() == line.trim()),
            "docs/{page} does not show this output line: `{line}`\nFix the page."
        );
    }
}

#[test]
fn getting_started() {
    let e = Env::new("docs_hello");
    e.copy("", &["hello_tb.vhd", "hello_error.vhd"]);
    assert_eq!(
        e.ok(&["analyze", "--work=work:build/work", "hello_tb.vhd"]),
        ""
    );
    let out = e.ok(&["run", "--work=work:build/work", "hello_tb"]);
    on_page("getting-started.rst", &out);
    let (code, out, _) = e.yavs(&["run", "--work=work:build/work", "hello_tb", "-gEXPECTED=5"]);
    assert_eq!(code, 1);
    on_page("getting-started.rst", &out);
    let (code, _, err) = e.yavs(&["analyze", "--work=work:build/work", "hello_error.vhd"]);
    assert_eq!(code, 1);
    assert!(err.contains("error[E1100]"), "{err}");
}

const WORK: &str = "--work=work:build/work";
const RTL: &str = "--map=rtl:build/rtl";

fn project(name: &str) -> Env {
    let e = Env::new(name);
    e.copy(
        "project",
        &[
            "rtl/counter_pkg.vhd",
            "rtl/counter.vhd",
            "tb/counter_tb.vhd",
        ],
    );
    e
}

/// The analyze commands of `docs/examples/project/run.sh`.
fn build(e: &Env) {
    e.ok(&[
        "analyze",
        "--work=rtl:build/rtl",
        "rtl/counter_pkg.vhd",
        "rtl/counter.vhd",
    ]);
    e.ok(&["analyze", WORK, RTL, "tb/counter_tb.vhd"]);
}

#[test]
fn walkthrough_order_and_mapping() {
    let e = project("docs_order");
    let (code, _, err) = e.yavs(&[
        "analyze",
        "--work=rtl:build/rtl",
        "rtl/counter.vhd",
        "rtl/counter_pkg.vhd",
    ]);
    assert_eq!(code, 1);
    assert!(err.contains("error[E1002]"), "{err}");
    e.ok(&[
        "analyze",
        "--work=rtl:build/rtl",
        "rtl/counter_pkg.vhd",
        "rtl/counter.vhd",
    ]);
    let (code, _, err) = e.yavs(&["analyze", WORK, "tb/counter_tb.vhd"]);
    assert_eq!(code, 1);
    assert!(err.contains("error[E1001]"), "{err}");
}

#[test]
fn walkthrough_generics_edit_waves_coverage() {
    let e = project("docs_project");
    build(&e);
    let out = e.ok(&["run", WORK, RTL, "counter_tb"]);
    on_page("project-walkthrough.rst", &out);
    e.ok(&["run", WORK, RTL, "counter_tb", "-gWIDTH=3"]);
    e.ok(&[
        "run",
        WORK,
        RTL,
        "counter_tb",
        "-gEXPECTED_WRAPS=3",
        "-gCLOCK_PERIOD=2ns",
    ]);

    // An edit stops `run`; analyzing the file alone is enough.
    let pkg = e.0.join("rtl/counter_pkg.vhd");
    let text = std::fs::read_to_string(&pkg).unwrap();
    std::fs::write(&pkg, format!("-- edited\n{text}")).unwrap();
    let (code, _, err) = e.yavs(&["run", WORK, RTL, "counter_tb"]);
    assert_eq!(code, 2);
    assert!(err.contains("error[L0034]"), "{err}");
    e.ok(&["analyze", "--work=rtl:build/rtl", "rtl/counter_pkg.vhd"]);
    e.ok(&["run", WORK, RTL, "counter_tb"]);

    // Waves and coverage, passed on as the run script passes them.
    e.ok(&[
        "run",
        WORK,
        RTL,
        "counter_tb",
        "--wave=build/tb.fst",
        "--cov=build/tb.ycov",
        "--cov-kind=statement,branch",
    ]);
    assert!(std::fs::metadata(e.0.join("build/tb.fst")).unwrap().len() > 0);
    let report = e.ok(&["cov", "report", "build/tb.ycov"]);
    on_page("project-walkthrough.rst", &report);

    // An edit that takes away what a dependent uses: `run` reports the dependent's error.
    std::fs::write(&pkg, text.replace("all_ones", "every_one")).unwrap();
    e.ok(&["analyze", "--work=rtl:build/rtl", "rtl/counter_pkg.vhd"]);
    let (code, _, err) = e.yavs(&["run", WORK, RTL, "counter_tb"]);
    assert_ne!(code, 0);
    assert!(
        err.contains("error[E1100]") && err.contains("counter.vhd"),
        "{err}"
    );

    // The cache can go at any time.
    e.ok(&[
        "cache",
        "clean",
        "--work=rtl:build/rtl",
        WORK,
        "--std-cache=off",
    ]);
}

#[test]
fn mixing_standards() {
    let e = Env::new("docs_mixed");
    e.copy("mixed", &["old_pkg.vhd", "mixed_tb.vhd"]);
    let (code, _, err) = e.yavs(&["analyze", WORK, "old_pkg.vhd"]);
    assert_eq!(code, 1);
    assert!(err.contains("error[E0002]"), "{err}");
    let (code, _, err) = e.yavs(&["analyze", "--std=93", WORK, "mixed_tb.vhd"]);
    assert_eq!(code, 1);
    assert!(err.contains("error[E0011]"), "{err}");
    e.ok(&["analyze", "--std=93", WORK, "old_pkg.vhd"]);
    e.ok(&["analyze", "--std=2008", WORK, "mixed_tb.vhd"]);
    let out = e.ok(&["run", WORK, "mixed_tb"]);
    on_page("mixing-standards.rst", &out);
}

/// Every form in the table of `docs/generics.rst`.
#[test]
fn generic_values() {
    const TB: &str = "library ieee; use ieee.std_logic_1164.all;\n\
entity g_tb is\n  generic (n : integer := 0; gain : real := 0.0; flag : boolean := false;\n\
    period : time := 1 ns; name : string := \"x\"; bits : std_logic_vector(3 downto 0) := \"0000\";\n\
    one : std_logic := '0'; c : character := 'a');\nend;\n\
architecture a of g_tb is begin\n  process begin\n\
    report integer'image(n) & \" \" & real'image(gain) & \" \" & boolean'image(flag) & \" \" & \
time'image(period) & \" \" & name & \" \" & to_string(bits) & \" \" & std_logic'image(one) & \" \" & \
character'image(c);\n    wait;\n  end process;\nend;\n";
    const REQUIRED: &str = "entity r_tb is generic (must : integer); end;\n\
architecture a of r_tb is begin end;\n";
    let e = Env::new("docs_generics");
    std::fs::write(e.0.join("g.vhd"), TB).unwrap();
    std::fs::write(e.0.join("r.vhd"), REQUIRED).unwrap();
    e.ok(&["analyze", WORK, "g.vhd", "r.vhd"]);
    let run = |g: &[&str]| {
        let mut args = vec!["run", WORK, "g_tb"];
        args.extend_from_slice(g);
        e.yavs(&args)
    };
    let (code, out, err) = run(&[
        "-gN=16#ff#",
        "-gGAIN=2.25",
        "-gFLAG=TRUE",
        "-gPERIOD=5ns",
        "-gNAME=a b",
        "-gBITS=1010",
        "-gONE='1'",
        "-gC='z'",
    ]);
    assert_eq!(code, 0, "{err}");
    assert!(
        out.contains("255 2.25 true 5000000 fs a b 1010 '1' 'z'"),
        "{out}"
    );
    let (code, out, _) = run(&["-g", "PERIOD=5 ns", "-gNAME=\"q\"", "-gN=1", "-gN=2"]);
    assert_eq!(code, 0);
    assert!(out.contains(" false 5000000 fs \"q\" "), "{out}");
    assert!(out.contains("0fs+0: 2 "), "{out}");
    for bad in ["-gBITS=101", "-gBITS=x\"A\"", "-gONE=1", "-gN=abc"] {
        let (code, _, err) = run(&[bad]);
        assert_eq!(code, 2, "{bad}");
        assert!(err.contains("error[E2104]"), "{err}");
    }
    let (code, _, err) = run(&["-gNOPE=1"]);
    assert_eq!(code, 0);
    assert!(err.contains("warning[W2105]"), "{err}");
    let (code, _, err) = run(&["-gNOPE=1", "-Werror=W2105"]);
    assert_ne!(code, 0);
    assert!(err.contains("W2105"), "{err}");
    let (code, _, err) = e.yavs(&["run", WORK, "r_tb"]);
    assert_eq!(code, 2);
    assert!(err.contains("error[E2103]"), "{err}");
    e.ok(&["elab", WORK, "r_tb", "-gmust=1"]);
    std::fs::write(
        e.0.join("args.txt"),
        "-gNAME=a long value with \"quotes\"\n",
    )
    .unwrap();
    let (code, out, _) = run(&["@args.txt"]);
    assert_eq!(code, 0);
    assert!(out.contains("a long value with \"quotes\""), "{out}");
}

/// What `docs/migrating.rst` says about time and exit status.
#[test]
fn time_and_exit_status() {
    let e = Env::new("docs_time");
    std::fs::write(
        e.0.join("t.vhd"),
        "entity clk_tb is end;\narchitecture a of clk_tb is signal c : bit; begin c <= not c after 5 ns; end;\n\
         use std.env.all;\nentity stop_tb is end;\narchitecture a of stop_tb is begin process begin wait for 1 ns; stop(3); wait; end process; end;\n\
         entity err_tb is end;\narchitecture a of err_tb is begin process begin report \"e\" severity error; report \"more\"; wait; end process; end;\n",
    )
    .unwrap();
    e.ok(&["analyze", WORK, "t.vhd"]);
    assert_eq!(e.yavs(&["run", WORK, "clk_tb", "--stop-time=20ns"]).0, 0);
    assert_eq!(e.yavs(&["run", WORK, "clk_tb", "--stop-time=20 ns"]).0, 0);
    let (code, _, err) = e.yavs(&["run", WORK, "clk_tb", "--stop-time=20"]);
    assert_eq!(code, 2);
    assert!(err.contains("error[E0105]"), "{err}");
    let (code, out, _) = e.yavs(&["run", WORK, "stop_tb", "--resolution=ps"]);
    assert_eq!(code, 3);
    assert!(out.contains("1000000fs+0"), "{out}");
    let (code, out, _) = e.yavs(&["run", WORK, "err_tb"]);
    assert_eq!(code, 1);
    assert!(out.contains("more"), "{out}");
    let (code, out, _) = e.yavs(&["run", WORK, "err_tb", "--exit-severity=error"]);
    assert_eq!(code, 1);
    assert!(!out.contains("more"), "{out}");
}
