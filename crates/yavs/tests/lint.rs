//! `yavs lint`: every rule against its regression files (`tests/lint/*.vhd`, a line with
//! `-- expect: lint_NNN` is where a rule reports, and nothing else may), then the configuration,
//! waivers, reports, libraries and exit statuses of the command.

mod common;
use common::tempdir;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn yavs(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(["--color", "never"])
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run")
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

fn status(o: &Output) -> i32 {
    o.status.code().expect("exit status")
}

fn case(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/lint")
        .join(name)
}

/// (line, rule) of each finding in a short format report.
fn findings(text: &str) -> Vec<(usize, String)> {
    text.lines()
        .filter_map(|l| {
            // `FILE:LINE:COL: severity[lint_NNN]: message`
            let (head, rule) = l.split_once("[lint_")?;
            let (rule, _) = rule.split_once(']')?;
            let mut parts = head.rsplit(':');
            parts.next(); // severity
            parts.next(); // column
            Some((parts.next()?.parse().ok()?, format!("lint_{rule}")))
        })
        .collect()
}

fn regression(name: &str) {
    regression_with(name, None);
}

fn regression_with(name: &str, config: Option<&str>) {
    let path = case(name);
    let text = std::fs::read_to_string(&path).unwrap();
    let mut want: Vec<(usize, String)> = text
        .lines()
        .enumerate()
        .flat_map(|(i, l)| {
            l.split_once("-- expect: ")
                .map_or(Vec::new(), |(_, rules)| {
                    rules
                        .split(", ")
                        .map(|r| (i + 1, r.trim().to_string()))
                        .collect()
                })
        })
        .collect();
    want.sort();
    let dir = tempdir("lint");
    if let Some(c) = config {
        std::fs::write(dir.join("yavs-lint.yaml"), c).unwrap();
    }
    let o = yavs(
        &dir,
        &["lint", "--message-format=short", path.to_str().unwrap()],
    );
    let got = stderr(&o);
    let mut found = findings(&got);
    found.sort();
    assert_eq!(found, want, "{got}");
    assert_eq!(status(&o), i32::from(!want.is_empty()), "{got}");
}

/// lint_602 and lint_603 report every register of the file, so they have a test of their own.
const EVERY_CLASS: &str = "rule:\n  group:\n    advisory:\n      disable: false\n    experimental:\n      disable: false\n  lint_713:\n    disable: false\n";

#[test]
fn design() {
    regression_with("design.vhd", Some(EVERY_CLASS));
}

/// The default run takes none of those rules.
#[test]
fn only_the_definite_class_by_default() {
    let dir = tempdir("lint");
    let o = yavs(
        &dir,
        &[
            "lint",
            "--message-format=short",
            case("design.vhd").to_str().unwrap(),
        ],
    );
    assert_eq!(findings(&stderr(&o)), Vec::new(), "{}", stderr(&o));
    assert_eq!(status(&o), 0);
}

#[test]
fn design_facts() {
    let dir = tempdir("lint");
    let o = yavs(
        &dir,
        &[
            "lint",
            "--facts=f.json",
            case("design.vhd").to_str().unwrap(),
        ],
    );
    assert_eq!(status(&o), 0, "{}", stderr(&o));
    let facts: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("f.json")).unwrap()).unwrap();
    assert_eq!(facts["schema_version"], 1);
    let a = facts["architectures"]
        .as_array()
        .unwrap()
        .iter()
        .find(|a| a["unit"] == "design(rtl)")
        .unwrap();
    let fsm = a["state_machines"].as_array().unwrap();
    let machine = fsm.iter().find(|m| m["type"] == "state_t").unwrap();
    assert_eq!(machine["states"].as_array().unwrap().len(), 4);
    assert_eq!(machine["reset_state"], "idle");
    assert!(
        machine["transitions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["from"] == "idle" && t["to"] == "running")
    );
    let clocks = a["clock_domains"].as_array().unwrap();
    assert!(clocks.iter().any(|c| {
        c["clock"] == "clk2"
            && c["registers"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r == "in_b")
    }));
    let resets = a["reset_domains"].as_array().unwrap();
    assert!(resets.iter().any(|c| c["reset"] == "rst"));
    let crossing = a["crossings"].as_array().unwrap();
    assert!(crossing.iter().any(|c| c["kind"] == "clock"
        && c["signal"] == "q_a"
        && c["from"] == "clk"
        && c["to"] == "clk2"));
}

/// lint_602 and lint_603 take their lists from the configuration.
#[test]
fn register_names_from_the_configuration() {
    let dir = tempdir("lint");
    std::fs::write(
        dir.join("r.vhd"),
        "library ieee; use ieee.std_logic_1164.all;\nentity r is port (clk, d : in std_logic); end entity;\narchitecture a of r is\n  signal x_p1, y : std_logic;\nbegin\n  process (clk) begin\n    if rising_edge(clk) then\n      x_p1 <= d;\n      y <= x_p1;\n    end if;\n  end process;\nend architecture;\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("yavs-lint.yaml"),
        "rule:\n  lint_602:\n    disable: false\n    suffixes: ['_p*']\n",
    )
    .unwrap();
    let o = yavs(&dir, &["lint", "--message-format=short", "r.vhd"]);
    assert_eq!(
        findings(&stderr(&o)),
        vec![(9, "lint_602".to_string())],
        "{}",
        stderr(&o)
    );
    std::fs::write(
        dir.join("yavs-lint.yaml"),
        "rule:\n  lint_770:\n    suffixes: ['_p']\n",
    )
    .unwrap();
    let o = yavs(&dir, &["lint", "r.vhd"]);
    assert!(
        status(&o) == 2 && stderr(&o).contains("E0134") && stderr(&o).contains("`suffixes`"),
        "`suffixes` belongs to lint_602 alone: {}",
        stderr(&o)
    );
}

const ADVISORY: &str = "rule:\n  group:\n    advisory:\n      disable: false\n";

#[test]
fn latches() {
    regression_with(
        "latches.vhd",
        Some("rule:\n  lint_600:\n    disable: false\n"),
    );
}

#[test]
fn drivers() {
    regression_with(
        "drivers.vhd",
        Some("rule:\n  lint_601:\n    disable: false\n"),
    );
}

#[test]
fn clock_and_reset_domains() {
    regression_with(
        "domains.vhd",
        Some(
            "rule:\n  lint_700:\n    disable: false\n  lint_701:\n    disable: false\n  lint_702:\n    disable: false\n  lint_703:\n    disable: false\n",
        ),
    );
}

#[test]
fn state_machines_and_others() {
    regression_with(
        "machines.vhd",
        Some(
            "rule:\n  lint_710:\n    disable: false\n  lint_711:\n    disable: false\n  lint_712:\n    disable: false\n  lint_713:\n    disable: false\n",
        ),
    );
}

/// The facts list one entry per register, whatever form it has, and say when it cannot be read.
#[test]
fn facts_of_every_form_of_machine() {
    let dir = tempdir("lint");
    let o = yavs(
        &dir,
        &[
            "lint",
            "--facts=f.json",
            case("machines.vhd").to_str().unwrap(),
        ],
    );
    assert!(dir.join("f.json").is_file(), "{}", stderr(&o));
    let facts: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("f.json")).unwrap()).unwrap();
    let machines = |unit: &str| {
        facts["architectures"]
            .as_array()
            .unwrap()
            .iter()
            .find(|a| a["unit"] == unit)
            .unwrap_or_else(|| panic!("no {unit} in {facts}"))["state_machines"]
            .as_array()
            .unwrap()
            .clone()
    };
    let two = machines("machines(two_process)");
    assert_eq!(two.len(), 1, "{two:?}");
    assert_eq!(two[0]["register"], "state");
    assert_eq!(
        two[0]["signals"],
        serde_json::json!(["state", "next_state"])
    );
    assert_eq!(two[0]["readable"], true);
    assert_eq!(two[0]["reset_state"], "idle");
    assert!(
        two[0]["transitions"]
            .as_array()
            .unwrap()
            .iter()
            .any(|t| t["from"] == "run" && t["to"] == "stuck")
    );
    assert_eq!(machines("machines(from_package)")[0]["type"], "shared_t");
    let regs = machines("machines(two_registers)");
    assert_eq!(regs.len(), 2, "{regs:?}");
    // A next state from a function cannot be read; a clockless signal is no machine.
    let narrowed = machines("machines(narrowed)");
    let other = narrowed.iter().find(|m| m["register"] == "other").unwrap();
    assert_eq!(other["readable"], false);
    assert!(machines("machines(no_clock)").is_empty());
}

#[test]
fn instances_recursion_and_sensitivity() {
    regression_with(
        "connected.vhd",
        Some(
            "rule:\n  lint_730:\n    disable: false\n  lint_750:\n    disable: false\n  lint_760:\n    disable: false\n  lint_001:\n    disable: false\n  lint_002:\n    disable: false\n",
        ),
    );
}

/// Every driver and every signal of a loop is a place of its own, not text in the message.
#[test]
fn the_other_places_of_a_finding() {
    let dir = tempdir("lint");
    std::fs::write(dir.join("yavs-lint.yaml"), ADVISORY).unwrap();
    std::fs::write(
        dir.join("r.vhd"),
        "library ieee; use ieee.std_logic_1164.all;\nentity r is port (a, b : in std_logic); end entity;\narchitecture x of r is\n  signal q, y, z : std_logic;\nbegin\n  q <= a;\n  q <= b;\n  y <= z;\n  z <= y;\nend architecture;\n",
    )
    .unwrap();
    let o = yavs(&dir, &["lint", "--sarif=r.sarif", "r.vhd"]);
    let text = stderr(&o);
    assert!(text.contains("error[lint_601]"), "{text}");
    assert!(text.contains("7 |   q <= b;"), "{text}");
    assert!(text.contains("also here"), "{text}");
    assert!(!text.contains("lines 6"), "{text}");
    let sarif: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(dir.join("r.sarif")).unwrap()).unwrap();
    let results = sarif["runs"][0]["results"].as_array().unwrap();
    let related = |rule: &str| {
        let r = results.iter().find(|r| r["ruleId"] == rule).unwrap();
        r["relatedLocations"].as_array().unwrap().len()
    };
    assert_eq!(related("lint_601"), 1);
    assert_eq!(related("lint_720"), 1);
}

/// The message of lint_790 says which branch of a generate a signal could move to.
#[test]
fn a_generate_branch_is_named() {
    let dir = tempdir("lint");
    std::fs::write(
        dir.join("yavs-lint.yaml"),
        "rule:\n  lint_790:\n    disable: false\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("r.vhd"),
        "entity r is generic (g : boolean := true; n : natural := 1); end entity;\narchitecture x of r is\n  signal s1, s2, s3, s4, y1, y2, y3, y4 : bit;\nbegin\n  e1 : if g generate\n    y1 <= s1;\n  elsif not g generate\n    y2 <= s2;\n  else generate\n    y3 <= s3;\n  end generate;\n  c1 : case n generate\n    when 1 | 2 =>\n      y4 <= s4;\n    when others =>\n  end generate;\nend architecture;\n",
    )
    .unwrap();
    let o = yavs(&dir, &["lint", "--message-format=short", "r.vhd"]);
    let text = stderr(&o);
    for (name, place) in [
        ("s1", "inside generate `e1`"),
        ("s2", "inside an elsif branch of generate `e1`"),
        ("s3", "inside the else branch of generate `e1`"),
        ("s4", "inside the `when 1 | 2` branch of generate `c1`"),
    ] {
        assert!(
            text.contains(&format!("`{name}` is only used {place}")),
            "{name}: {text}"
        );
    }
}

/// `rtl:` and `testbench:` choose rules per kind of file: what the project lists first, then what
/// the code and the name say.
#[test]
fn testbench_and_rtl_blocks() {
    let dir = tempdir("lint");
    std::fs::create_dir_all(dir.join("verify")).unwrap();
    let body = "entity E is port (a : in bit); end entity;\narchitecture x of E is\n  signal q : bit;\nbegin\n  process (a) begin\n    if a = '1' then\n      q <= a;\n    end if;\n  end process;\nend architecture;\n";
    let file = |name: &str, unit: &str, head: &str| {
        std::fs::write(dir.join(name), format!("{head}{}", body.replace('E', unit))).unwrap();
    };
    file("adder.vhd", "adder", "");
    file("tb_named.vhd", "named", "");
    file("verify/listed.vhd", "listed", "");
    file("marked.vhd", "marked", "-- yavs: testbench\n");
    file(
        "using.vhd",
        "using",
        "package bitvis_helpers is end package;\n",
    );
    file(
        "commented.vhd",
        "commented",
        "-- osvvm is only named here\n",
    );
    std::fs::write(
        dir.join("noports.vhd"),
        "entity noports is end entity;\narchitecture x of noports is\n  signal q, a : bit;\nbegin\n  process (a) begin\n    if a = '1' then\n      q <= a;\n    end if;\n  end process;\nend architecture;\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("yavs-lint.yaml"),
        "testbench_files: ['verify/**']\nrtl:\n  rule:\n    lint_600:\n      disable: false\ntestbench:\n  rule:\n    lint_600:\n      disable: true\n",
    )
    .unwrap();
    let files = [
        "adder.vhd",
        "tb_named.vhd",
        "verify/listed.vhd",
        "marked.vhd",
        "using.vhd",
        "commented.vhd",
        "noports.vhd",
    ];
    let mut args = vec!["lint", "--message-format=short"];
    args.extend(files);
    let o = yavs(&dir, &args);
    let text = stderr(&o);
    let with: Vec<&&str> = files.iter().filter(|f| text.contains(**f)).collect();
    assert_eq!(with, [&"adder.vhd", &"commented.vhd"], "{text}");

    // A library named as one.
    std::fs::write(
        dir.join("yavs-lint.yaml"),
        "testbench_libraries: [tb_lib]\nrtl:\n  rule:\n    lint_600:\n      disable: false\n",
    )
    .unwrap();
    let o = yavs(
        &dir,
        &[
            "lint",
            "--work=tb_lib:lib",
            "--message-format=short",
            "adder.vhd",
        ],
    );
    assert_eq!(findings(&stderr(&o)), Vec::new(), "{}", stderr(&o));
    let o = yavs(&dir, &["lint", "--message-format=short", "adder.vhd"]);
    assert_eq!(findings(&stderr(&o)).len(), 1, "{}", stderr(&o));

    // A wrong key names itself.
    std::fs::write(dir.join("yavs-lint.yaml"), "testbench:\n  rules: {}\n").unwrap();
    let o = yavs(&dir, &["lint", "adder.vhd"]);
    assert_eq!(status(&o), 2);
    assert!(
        stderr(&o).contains("E0134") && stderr(&o).contains("`rules`"),
        "{}",
        stderr(&o)
    );
}

/// A naming rule with no list says nothing, and a list can hold a regular expression.
#[test]
fn register_names_only_when_asked() {
    let dir = tempdir("lint");
    std::fs::write(
        dir.join("r.vhd"),
        "entity r is port (clk, d : in bit); end entity;\narchitecture a of r is\n  signal a_p1, b_p12, c_comb, r_x, wire : bit;\nbegin\n  wire <= d;\n  process (clk) begin\n    if clk'event and clk = '1' then\n      a_p1 <= d;\n      b_p12 <= d;\n      c_comb <= d;\n      r_x <= d;\n    end if;\n  end process;\nend architecture;\n",
    )
    .unwrap();
    let run = |cfg: &str| {
        std::fs::write(dir.join("yavs-lint.yaml"), cfg).unwrap();
        let o = yavs(&dir, &["lint", "--message-format=short", "r.vhd"]);
        (status(&o), findings(&stderr(&o)), stderr(&o))
    };
    let (_, found, text) =
        run("rule:\n  lint_602:\n    disable: false\n  lint_603:\n    disable: false\n");
    assert_eq!(found, Vec::new(), "{text}");
    let (_, found, text) =
        run("rule:\n  lint_602:\n    disable: false\n    suffixes: ['re:_P[0-9]+']\n");
    let lines: Vec<usize> = found.iter().map(|f| f.0).collect();
    // `wire` is combinational: no register, whatever its name.
    assert_eq!(lines, [10, 11], "{text}");
    let (_, found, text) = run("rule:\n  lint_603:\n    disable: false\n    prefixes: ['r_']\n");
    assert_eq!(found.len(), 3, "{text}");
    let (status, _, text) = run("rule:\n  lint_602:\n    disable: false\n    suffixes: ['re:(']\n");
    assert_eq!(status, 2, "{text}");
    assert!(text.contains("not a regular expression"), "{text}");
}

#[test]
fn loops() {
    regression_with(
        "loops.vhd",
        Some("rule:\n  lint_720:\n    disable: false\n"),
    );
}

/// The caret of lint_771 is on the name in the body, with a spec or without one.
#[test]
fn a_function_is_reported_at_its_name() {
    let dir = tempdir("lint");
    std::fs::write(
        dir.join("f.vhd"),
        "package p is\n  function spec (w : natural) return natural;\nend package;\npackage body p is\n  function spec (w : natural) return natural is begin end;\n  function own (w : natural) return natural is begin end;\nend package body;\n",
    )
    .unwrap();
    let o = yavs(&dir, &["lint", "--message-format=short", "f.vhd"]);
    let text = stderr(&o);
    assert!(text.contains("f.vhd:5:12: error[lint_771]"), "{text}");
    assert!(text.contains("f.vhd:6:12: error[lint_771]"), "{text}");
}

#[test]
fn flow() {
    regression("flow.vhd");
}

#[test]
fn suspend() {
    regression("suspend.vhd");
}

#[test]
fn width() {
    regression("width.vhd");
}

#[test]
fn evaluated() {
    regression("evaluated.vhd");
}

#[test]
fn configuration() {
    regression("configuration.vhd");
}

#[test]
fn instance_names_an_architecture() {
    regression("instance.vhd");
}

#[test]
fn signal_declared_too_wide() {
    regression_with(
        "scope.vhd",
        Some("rule:\n  lint_790:\n    disable: false\n"),
    );
}

/// `exclude` is relative to the configuration file, covers directories and globs, and a file it
/// covers can still be needed by another.
#[test]
fn exclude_in_the_configuration() {
    let dir = tempdir("lint");
    std::fs::create_dir_all(dir.join("gen/deep")).unwrap();
    std::fs::create_dir_all(dir.join("work")).unwrap();
    two(&dir);
    two(&dir.join("gen/deep"));
    std::fs::write(dir.join("old_pk.vhd"), PKG).unwrap();
    std::fs::write(dir.join("a_user.vhd"), USER).unwrap();
    std::fs::write(
        dir.join("yavs-lint.yaml"),
        "exclude:\n  - gen\n  - 'old_*.vhd'\n",
    )
    .unwrap();
    // From another directory: the patterns are the configuration file's.
    let args = [
        "lint",
        "--message-format=short",
        "--config=../yavs-lint.yaml",
    ];
    let o = yavs(
        &dir.join("work"),
        &[&args[..], &["../gen/deep/two.vhd"]].concat(),
    );
    assert_eq!(findings(&stderr(&o)), Vec::new(), "{}", stderr(&o));
    assert_eq!(status(&o), 0, "nothing left to check is no error");
    let o = yavs(&dir.join("work"), &[&args[..], &["../two.vhd"]].concat());
    assert_eq!(findings(&stderr(&o)).len(), 2, "{}", stderr(&o));
    // The excluded package is read for the file that uses it, and its finding is not reported.
    let o = yavs(
        &dir.join("work"),
        &[&args[..], &["../old_pk.vhd", "../a_user.vhd"]].concat(),
    );
    assert_eq!(
        findings(&stderr(&o)),
        vec![(6, "lint_770".to_string())],
        "{}",
        stderr(&o)
    );
    // The same for what a library records.
    let analyze = yavs(
        &dir,
        &["analyze", "--work=mine:lib", "old_pk.vhd", "a_user.vhd"],
    );
    assert!(analyze.status.success(), "{}", stderr(&analyze));
    let o = yavs(&dir, &["lint", "--message-format=short", "--work=mine:lib"]);
    assert_eq!(
        findings(&stderr(&o)),
        vec![(6, "lint_770".to_string())],
        "{}",
        stderr(&o)
    );
}

/// The files of the findings in the short format, sorted.
fn files_found(text: &str) -> Vec<String> {
    let mut out: Vec<String> = text
        .lines()
        .filter(|l| l.contains("lint_"))
        .map(|l| l.split(':').next().unwrap().replace('\\', "/"))
        .collect();
    out.sort();
    out
}

fn file_list_tree() -> common::Tmp {
    let dir = tempdir("lint");
    for d in ["src/deep", "src/.hidden", "other"] {
        std::fs::create_dir_all(dir.join(d)).unwrap();
    }
    for f in [
        "src/a.vhd",
        "src/b.vhdl",
        "src/deep/c.vhd",
        "src/.hidden/d.vhd",
        "other/e.vhd",
    ] {
        let name = Path::new(f).file_stem().unwrap().to_str().unwrap();
        let text = format!(
            "entity {name} is end;\narchitecture a of {name} is\nbegin\n  process begin report \"x\"; end process;\nend;\n"
        );
        std::fs::write(dir.join(f), text).unwrap();
    }
    dir
}

const SHORT: &str = "--message-format=short";

/// `file_list`: paths, globs, `**`, variables, a directory; hidden entries are skipped.
#[test]
fn file_list_names_what_to_check() {
    let dir = file_list_tree();
    let run = |list: &str| {
        std::fs::write(dir.join("yavs-lint.yaml"), list).unwrap();
        let o = yavs(&dir, &["lint", SHORT]);
        assert_eq!(status(&o), 1, "{}", stderr(&o));
        files_found(&stderr(&o))
    };
    assert_eq!(run("file_list:\n  - other/e.vhd\n"), ["other/e.vhd"]);
    assert_eq!(run("file_list: ['src/*.vhd']\n"), ["src/a.vhd"]);
    assert_eq!(
        run("file_list: ['src/**/*.vhd']\n"),
        ["src/a.vhd", "src/deep/c.vhd"],
        "`**` crosses directories, and a hidden directory is skipped"
    );
    assert_eq!(
        run("file_list: ['src/?.vhd*']\n"),
        ["src/a.vhd", "src/b.vhdl"]
    );
    // A directory reads as on the command line.
    assert_eq!(run("file_list: [src]\n"), ["src/a.vhd", "src/b.vhdl"]);
    let o = {
        std::fs::write(dir.join("yavs-lint.yaml"), "file_list: [src]\n").unwrap();
        yavs(&dir, &["lint", SHORT, "--recursive"])
    };
    assert_eq!(
        files_found(&stderr(&o)),
        [
            "src/.hidden/d.vhd",
            "src/a.vhd",
            "src/b.vhdl",
            "src/deep/c.vhd"
        ],
        "as a directory on the command line, which reads hidden directories too"
    );
    std::fs::write(
        dir.join("yavs-lint.yaml"),
        "file_list: ['$LINT_TEST_LIST_DIR/e.vhd', '${LINT_TEST_LIST_DIR}/e.vhd']\n",
    )
    .unwrap();
    let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(["--color", "never", "lint", SHORT])
        .env("LINT_TEST_LIST_DIR", "other")
        .current_dir(&*dir)
        .output()
        .unwrap();
    assert_eq!(files_found(&stderr(&o)), ["other/e.vhd"], "{}", stderr(&o));
}

/// Files on the command line are checked as well as the list, once each; `exclude` wins over it;
/// `--no-file-list` leaves it out; a library's files keep the library they are recorded in.
#[test]
fn file_list_with_files_on_the_command_line() {
    let dir = file_list_tree();
    let cfg = dir.join("yavs-lint.yaml");
    std::fs::write(&cfg, "file_list: [src/a.vhd, other/e.vhd]\n").unwrap();
    let o = yavs(&dir, &["lint", SHORT, "src/deep/c.vhd", "./src/a.vhd"]);
    assert_eq!(
        files_found(&stderr(&o)),
        ["./src/a.vhd", "other/e.vhd", "src/deep/c.vhd"],
        "named once, the first way: {}",
        stderr(&o)
    );
    let o = yavs(&dir, &["lint", SHORT, "--no-file-list", "src/deep/c.vhd"]);
    assert_eq!(
        files_found(&stderr(&o)),
        ["src/deep/c.vhd"],
        "{}",
        stderr(&o)
    );
    // Nothing given and the list left out: the usual error.
    let o = yavs(&dir, &["lint", SHORT, "--no-file-list"]);
    assert_eq!(status(&o), 2);
    assert!(stderr(&o).contains("nothing to check"), "{}", stderr(&o));
    std::fs::write(
        &cfg,
        "file_list: [src/a.vhd, other/e.vhd]\nexclude: [other]\n",
    )
    .unwrap();
    let o = yavs(&dir, &["lint", SHORT]);
    assert_eq!(files_found(&stderr(&o)), ["src/a.vhd"], "{}", stderr(&o));

    // The list wins over the default of a library: with `--work` it says what is checked.
    std::fs::write(&cfg, "file_list: [src/a.vhd]\n").unwrap();
    let analyze = yavs(&dir, &["analyze", "--work=mine:lib", "other/e.vhd"]);
    assert!(analyze.status.success(), "{}", stderr(&analyze));
    let o = yavs(&dir, &["lint", SHORT, "--work=mine:lib"]);
    assert_eq!(files_found(&stderr(&o)), ["src/a.vhd"], "{}", stderr(&o));
    let o = yavs(&dir, &["lint", SHORT, "--work=mine:lib", "--no-file-list"]);
    let text = stderr(&o);
    assert!(text.contains("e.vhd") && !text.contains("a.vhd"), "{text}");
}

/// Entries are relative to the working directory, not the configuration file.
#[test]
fn file_list_is_relative_to_the_working_directory() {
    let dir = file_list_tree();
    std::fs::write(dir.join("other/list.yaml"), "file_list: [src/a.vhd]\n").unwrap();
    let o = yavs(&dir, &["lint", SHORT, "--config=other/list.yaml"]);
    assert_eq!(files_found(&stderr(&o)), ["src/a.vhd"], "{}", stderr(&o));
    // The configuration found in an ancestor: the entry does not exist from here.
    std::fs::write(dir.join("yavs-lint.yaml"), "file_list: [src/a.vhd]\n").unwrap();
    let o = yavs(&dir.join("other"), &["lint", SHORT]);
    assert_eq!(status(&o), 2);
    assert!(
        stderr(&o).contains("relative to the working directory"),
        "{}",
        stderr(&o)
    );
    let o = yavs(
        &dir.join("other"),
        &["lint", SHORT, "--no-file-list", "e.vhd"],
    );
    assert_eq!(status(&o), 1, "{}", stderr(&o));
}

/// An entry that gives no file is E0134 naming the entry, the configuration and the directory it is relative to, with a near name; so is a bad form of the key.
#[test]
fn file_list_entry_that_is_missing() {
    let dir = file_list_tree();
    let cfg = dir.join("yavs-lint.yaml");
    let bad = |list: &str| {
        std::fs::write(&cfg, list).unwrap();
        let o = yavs(&dir, &["lint", SHORT]);
        assert_eq!(status(&o), 2, "{}", stderr(&o));
        let text = stderr(&o);
        assert!(text.contains("E0134"), "{text}");
        assert!(text.contains("yavs-lint.yaml"), "{text}");
        text
    };
    let text = bad("file_list:\n  - src/a.vhd\n  - src/deep/no_such.vhd\n");
    assert!(text.contains("`src/deep/no_such.vhd`"), "{text}");
    assert!(text.contains("working directory"), "{text}");
    assert!(findings(&text).is_empty(), "nothing is checked: {text}");
    let text = bad("file_list: [src/deep/cc.vhd]\n");
    assert!(
        text.contains("a path with a similar name exists: `c.vhd`"),
        "{text}"
    );
    let text = bad("file_list: ['sources/*.vhd', src/a.vhd]\n");
    assert!(text.contains("`sources/*.vhd` matches no file"), "{text}");
    let text = bad("file_list: ['srd/*.vhd']\n");
    assert!(
        text.contains("a path with a similar name exists: `src`"),
        "{text}"
    );
    let text = bad("file_list: ['src/*.txt']\n");
    assert!(text.contains("`src/*.txt`"), "{text}");
    let text = bad("file_list: ['$LINT_TEST_NOT_SET/a.vhd']\n");
    assert!(text.contains("`$LINT_TEST_NOT_SET/a.vhd`"), "{text}");
    std::fs::create_dir_all(dir.join("empty")).unwrap();
    let text = bad("file_list: [empty]\n");
    assert!(text.contains("no VHDL file"), "{text}");
    let text = bad("file_list: src\n");
    assert!(text.contains("must be a list"), "{text}");
    let text = bad("file_list:\n  - src/a.vhd:\n      rule: {}\n");
    assert!(
        text.contains("not implemented") && text.contains("`src/a.vhd`"),
        "{text}"
    );
    // `--list-rules` and `--explain` read no files.
    std::fs::write(&cfg, "file_list: [no_such.vhd]\n").unwrap();
    let o = yavs(&dir, &["lint", "--list-rules"]);
    assert_eq!(status(&o), 0, "{}", stderr(&o));
}

/// lint_782 and lint_006 name what analysis rejects first (E1403, E0002), so the run says so with
/// that code, and the rules stay only for a configuration to name.
#[test]
fn what_analysis_rejects_first() {
    let dir = tempdir("lint");
    for (name, source, code) in [
        (
            "range.vhd",
            "entity a is end entity;\narchitecture r of a is\n  signal s : integer range 0 to 7;\nbegin\n  s <= 9;\nend architecture;\n",
            "E1403",
        ),
        (
            "config.vhd",
            "entity t is end entity;\narchitecture s of t is begin end architecture;\nconfiguration c of t is\n  for no_such\n  end for;\nend configuration;\n",
            "E1002",
        ),
        ("context.vhd", "library ieee;\n", "E0002"),
    ] {
        std::fs::write(dir.join(name), source).unwrap();
        let o = yavs(&dir, &["lint", "--message-format=short", name]);
        assert!(stderr(&o).contains(code), "{}", stderr(&o));
        assert_eq!(status(&o), 2);
    }
    let cfg = "rule:\n  lint_006:\n    disable: true\n  lint_751:\n    disable: true\n  lint_782:\n    disable: true\n";
    std::fs::write(dir.join("yavs-lint.yaml"), cfg).unwrap();
    let o = yavs(&dir, &["lint", "--list-rules"]);
    assert_eq!(status(&o), 0, "a configuration may name it");
}

const TWO_FINDINGS: &str = "entity two is end entity;\narchitecture r of two is\nbegin\n  p : process begin report \"x\"; end process;\n  q : process begin report \"y\"; end process;\nend architecture;\n";

fn two(dir: &Path) {
    std::fs::write(dir.join("two.vhd"), TWO_FINDINGS).unwrap();
}

#[test]
fn configuration_turns_rules_off_and_lowers_severity() {
    let dir = tempdir("lint");
    two(&dir);
    std::fs::write(
        dir.join("yavs-lint.yaml"),
        "rule:\n  lint_770:\n    severity: warning\n",
    )
    .unwrap();
    let o = yavs(&dir, &["lint", "--message-format=short", "two.vhd"]);
    assert_eq!(findings(&stderr(&o)).len(), 2);
    assert!(stderr(&o).contains("warning[lint_770]"), "{}", stderr(&o));
    assert_eq!(status(&o), 0, "only warnings");
    let o = yavs(&dir, &["lint", "-Werror", "two.vhd"]);
    assert_eq!(status(&o), 1, "-Werror");
    let o = yavs(&dir, &["lint", "-Werror=lint_770", "two.vhd"]);
    assert_eq!(status(&o), 1, "-Werror=RULE");
    let o = yavs(
        &dir,
        &["lint", "-Wno-lint_770", "two.vhd", "--message-format=short"],
    );
    assert_eq!(findings(&stderr(&o)), Vec::new());
    assert_eq!(status(&o), 0);
    let o = yavs(&dir, &["lint", "-Wno-E0001", "two.vhd"]);
    assert_eq!(status(&o), 2, "{}", stderr(&o));

    std::fs::write(
        dir.join("yavs-lint.yaml"),
        "rule:\n  group:\n    definite:\n      disable: true\n",
    )
    .unwrap();
    let o = yavs(&dir, &["lint", "two.vhd"]);
    assert_eq!(status(&o), 0);
    let list = yavs(&dir, &["lint", "--list-rules"]);
    assert!(
        String::from_utf8_lossy(&list.stdout).contains("lint_770 definite error off"),
        "{}",
        String::from_utf8_lossy(&list.stdout)
    );

    let o = yavs(&dir, &["lint", "--config", "missing.yaml", "two.vhd"]);
    assert_eq!(status(&o), 2);
    assert!(stderr(&o).contains("E0134"), "{}", stderr(&o));
}

#[test]
fn waivers_next_to_the_configuration_drop_findings() {
    let dir = tempdir("lint");
    two(&dir);
    std::fs::write(
        dir.join("yavs-lint-waivers.yaml"),
        "waivers:\n  - rule: lint_770\n    files: 'two.vhd'\n    lines: [4]\n    reason: a test\n",
    )
    .unwrap();
    let o = yavs(&dir, &["lint", "--message-format=short", "two.vhd"]);
    let found = findings(&stderr(&o));
    assert_eq!(found, vec![(5, "lint_770".to_string())], "{}", stderr(&o));
    assert_eq!(status(&o), 1);
    std::fs::write(
        dir.join("yavs-lint-waivers.yaml"),
        "waivers:\n  - rule: '*'\n    reason: all\n",
    )
    .unwrap();
    let o = yavs(&dir, &["lint", "two.vhd"]);
    assert_eq!(status(&o), 0);
    assert!(stderr(&o).contains("2 waived"), "{}", stderr(&o));
    std::fs::write(
        dir.join("yavs-lint-waivers.yaml"),
        "waivers:\n  - reason: x\n",
    )
    .unwrap();
    let o = yavs(&dir, &["lint", "two.vhd"]);
    assert_eq!(status(&o), 2);
    assert!(stderr(&o).contains("E0134"), "{}", stderr(&o));
}

/// An unknown key, rule or rule group at any level of either file is E0134, status 2, with the
/// line and a suggestion; every one of them is listed in one run, whatever `-W` says.
#[test]
fn unknown_keys_are_errors_listing_all_of_them() {
    let dir = tempdir("lint");
    two(&dir);
    let cfg = dir.join("yavs-lint.yaml");
    for (text, name) in [
        ("indent:\n  tokens: {a: [1]}\n", "`indent`"),
        ("rule:\n  length_001:\n    length: 120\n", "`length_001`"),
        ("rule:\n  lint_100:\n    disable: false\n", "`lint_100`"),
        (
            "rule:\n  group:\n    alignment:\n      disable: true\n",
            "`alignment`",
        ),
        ("rule:\n  global:\n    fixable: true\n", "`fixable`"),
        (
            "rule:\n  group:\n    definite:\n      fixable: [a]\n",
            "`fixable`",
        ),
        (
            "rule:\n  lint_700:\n    synchronizers: [a]\n",
            "`synchronizers`",
        ),
        ("rtl:\n  other: 1\n", "`other`"),
        ("testbench:\n  rule:\n    port_007: {}\n", "`port_007`"),
    ] {
        std::fs::write(&cfg, text).unwrap();
        let o = yavs(&dir, &["lint", SHORT, "two.vhd"]);
        let said = stderr(&o);
        assert_eq!(status(&o), 2, "{said}");
        assert_eq!(findings(&said).len(), 0, "nothing is checked: {said}");
        assert_eq!(said.matches("error[E0134]").count(), 1, "{said}");
        assert!(said.contains(name) && said.contains("unknown"), "{said}");
        assert!(
            said.contains("yavs-lint.yaml") && said.contains("line "),
            "{said}"
        );
        assert!(!said.contains("W0139"), "{said}");
    }
    for flag in ["-Werror", "--list-rules"] {
        let o = yavs(&dir, &["lint", flag, "two.vhd"]);
        assert_eq!(status(&o), 2, "{flag}: {}", stderr(&o));
    }

    // A name close to a real one gets a suggestion.
    std::fs::write(&cfg, "rule:\n  lnt_771:\n    disable: true\n").unwrap();
    let o = yavs(&dir, &["lint", SHORT, "two.vhd"]);
    assert!(
        stderr(&o).contains("a rule with a similar name exists: `lint_771`"),
        "{}",
        stderr(&o)
    );

    // Many in one file: every name on its own line in the human format, and in the short one all
    // on the one line of the diagnostic.
    let mut many = "rule:\n".to_string();
    for i in 0..90 {
        many += &format!("  style_{i:03}: {{disable: true}}\n");
    }
    many += "indent: {}\n";
    std::fs::write(&cfg, many).unwrap();
    for format in [SHORT, "--message-format=human"] {
        let o = yavs(&dir, &["lint", format, "two.vhd"]);
        let said = stderr(&o);
        assert_eq!(status(&o), 2, "{said}");
        assert_eq!(said.matches("[E0134]").count(), 1, "{said}");
        if format == SHORT {
            let lines = said.lines().filter(|l| !l.contains("[lint_")).count();
            assert_eq!(lines, 1, "one line per diagnostic: {said}");
        }
        for i in 0..90 {
            assert!(said.contains(&format!("`style_{i:03}`")), "{said}");
        }
        assert!(said.contains("`indent`"), "{said}");
    }

    // A bad value of a key it knows stops the run.
    for text in [
        "rule:\n  lint_770:\n    severity: fatal\n",
        "rule:\n  lint_770:\n    disable: maybe\n",
        "rule: [a]\n",
        "exclude: a\n",
    ] {
        std::fs::write(&cfg, text).unwrap();
        let o = yavs(&dir, &["lint", "two.vhd"]);
        assert_eq!(status(&o), 2, "{text}: {}", stderr(&o));
        assert!(stderr(&o).contains("E0134"), "{}", stderr(&o));
    }
    std::fs::remove_file(&cfg).unwrap();

    // The waiver file: unknown keys and rules are errors too, all listed, none applied.
    let waivers = dir.join("yavs-lint-waivers.yaml");
    std::fs::write(
        &waivers,
        "waive: []\nwaivers:\n  - rule: [length_001, lint_770]\n    lines: [4]\n    reson: x\n  - rule: port_007\n",
    )
    .unwrap();
    let o = yavs(&dir, &["lint", SHORT, "two.vhd"]);
    let said = stderr(&o);
    assert_eq!(status(&o), 2, "{said}");
    assert_eq!(findings(&said).len(), 0, "{said}");
    assert_eq!(said.matches("error[E0134]").count(), 1, "{said}");
    for name in ["`waive`", "`length_001`", "`reson`", "`port_007`"] {
        assert!(said.contains(name), "{name}: {said}");
    }
    assert!(
        said.contains("a key with a similar name exists: `reason`"),
        "{said}"
    );
    assert!(said.contains("yavs-lint-waivers.yaml"), "{said}");
    // A misspelled `files` must not leave a waiver that covers every file.
    std::fs::write(&waivers, "waivers:\n  - rule: lint_770\n    fils: x.vhd\n").unwrap();
    let o = yavs(&dir, &["lint", SHORT, "two.vhd"]);
    assert_eq!(status(&o), 2, "{}", stderr(&o));
    assert!(
        stderr(&o).contains("a key with a similar name exists: `files`"),
        "{}",
        stderr(&o)
    );
    std::fs::write(&waivers, "waivers:\n  - rule: lint_770\n    lines: three\n").unwrap();
    let o = yavs(&dir, &["lint", "two.vhd"]);
    assert_eq!(status(&o), 2);
    assert!(stderr(&o).contains("E0134"), "{}", stderr(&o));
}

/// Only `yavs lint` reads the two files: a broken one beside the design changes nothing for the
/// commands that analyse and run it.
#[test]
fn only_lint_reads_its_files() {
    let dir = tempdir("lint");
    std::fs::write(
        dir.join("e.vhd"),
        "entity e is end entity;\narchitecture a of e is\nbegin\n  process begin report \"ran\"; wait; end process;\nend architecture;\n",
    )
    .unwrap();
    std::fs::write(dir.join("yavs-lint.yaml"), "indent: [\nrule: 5\n").unwrap();
    std::fs::write(dir.join("yavs-lint-waivers.yaml"), "waivers: nonsense\n").unwrap();
    let o = yavs(&dir, &["analyze", "--work=w:out/w", "e.vhd"]);
    assert_eq!(status(&o), 0, "{}", stderr(&o));
    let o = yavs(&dir, &["run", "--work=w:out/w", "e"]);
    assert_eq!(status(&o), 0, "{}", stderr(&o));
    assert!(!stderr(&o).contains("yavs-lint"), "{}", stderr(&o));
    let o = yavs(&dir, &["lint", "--work=w:out/w"]);
    assert_eq!(status(&o), 2, "lint does read them: {}", stderr(&o));
}

/// Rules off for whole directories and patterns, from any working directory and in a library run.
#[test]
fn waivers_of_directories_and_lists_of_rules() {
    let dir = tempdir("lint");
    let source = |n: &str| {
        format!(
            "entity {n} is end entity;\narchitecture r of {n} is\nbegin\n  p : process begin report \"x\"; end process;\nend architecture;\n"
        )
    };
    for (d, n) in [
        ("src/legacy/deep", "old"),
        ("src/new", "fresh"),
        ("third", "lib"),
    ] {
        std::fs::create_dir_all(dir.join(d)).unwrap();
        std::fs::write(dir.join(d).join(format!("{n}.vhd")), source(n)).unwrap();
    }
    let files = [
        "src/legacy/deep/old.vhd",
        "src/new/fresh.vhd",
        "third/lib.vhd",
    ];
    let waivers = |text: &str| std::fs::write(dir.join("yavs-lint-waivers.yaml"), text).unwrap();
    let left = |cwd: &Path, args: &[&str]| {
        let mut all = vec!["lint", "--message-format=short"];
        all.extend_from_slice(args);
        let o = yavs(cwd, &all);
        let text = stderr(&o);
        let mut found: Vec<_> = ["old.vhd", "fresh.vhd", "lib.vhd"]
            .into_iter()
            .filter(|f| text.contains(&format!("{f}:4:3: error[lint_770]")))
            .collect();
        found.sort_unstable();
        (found, status(&o), text)
    };
    let mut args = vec![];
    args.extend_from_slice(&files);

    // A plain directory, with and without the slash, covers everything under it, like `exclude`.
    for form in ["src/legacy", "src/legacy/", "./src/legacy"] {
        waivers(&format!(
            "waivers:\n  - rule: lint_770\n    files: {form}\n"
        ));
        let (found, code, text) = left(&dir, &args);
        assert_eq!(found, ["fresh.vhd", "lib.vhd"], "{form}: {text}");
        assert_eq!(code, 1);
    }
    // Lists of rules and of patterns, in one entry.
    waivers(
        "waivers:\n  - rule: [lint_771, lint_770]\n    files: [src/legacy, 'third/*.vhd']\n    reason: x\n",
    );
    let (found, _, text) = left(&dir, &args);
    assert_eq!(found, ["fresh.vhd"], "{text}");
    // The same in dashed lists, nested in the entry.
    waivers(
        "waivers:\n  - rule:\n      - lint_771\n      - lint_770\n    files:\n      - src/legacy\n      - 'third/*.vhd'\n    reason: x\n",
    );
    let (found, _, text) = left(&dir, &args);
    assert_eq!(found, ["fresh.vhd"], "{text}");
    // A rule the entry does not name is not waived.
    waivers(
        "waivers:\n  - rule:\n      - lint_771\n      - lint_772\n    files:\n      - src\n      - third\n",
    );
    let (found, _, text) = left(&dir, &args);
    assert_eq!(found.len(), 3, "{text}");
    // `files_exclude` wins over `files`, dashed or in brackets, a directory or a pattern.
    for form in [
        "files_exclude:\n      - src/new\n",
        "files_exclude: [src/new]\n",
        "files_exclude:\n      - 'src/new/*.vhd'\n",
        "files_exclude: src/new\n",
    ] {
        waivers(&format!(
            "waivers:\n  - rule: lint_770\n    files:\n      - src\n      - third\n    {form}"
        ));
        let (found, _, text) = left(&dir, &args);
        assert_eq!(found, ["fresh.vhd"], "{form}: {text}");
    }
    // Without `files`, every file except those.
    waivers("waivers:\n  - rule: lint_770\n    files_exclude:\n      - src/legacy\n");
    let (found, _, text) = left(&dir, &args);
    assert_eq!(found, ["old.vhd"], "{text}");
    // A single word in `files`, as an existing waiver file has it.
    waivers(
        "waivers:\n  - rule: lint_770\n    files: 'third/lib.vhd'\n    lines: [4]\n    reason: x\n",
    );
    let (found, _, text) = left(&dir, &args);
    assert_eq!(found, ["fresh.vhd", "old.vhd"], "{text}");

    // Relative to the waiver file: from another directory, and for the absolute paths a library records.
    waivers("waivers:\n  - rule: lint_770\n    files: ['src/legacy/**/*.vhd', third]\n");
    let (found, _, text) = left(
        &dir.join("src"),
        &["legacy/deep/old.vhd", "new/fresh.vhd", "../third/lib.vhd"],
    );
    assert_eq!(found, ["fresh.vhd"], "{text}");
    let lib = dir.join("out/lib");
    let work = format!("--work=lib:{}", lib.display());
    let mut analyze = vec!["analyze", work.as_str()];
    analyze.extend_from_slice(&files);
    assert_eq!(status(&yavs(&dir, &analyze)), 0);
    let (found, _, text) = left(&dir, &[work.as_str()]);
    assert_eq!(found, ["fresh.vhd"], "{text}");

    // A pattern that covers nothing is reported and changes no status, unless asked to.
    waivers(
        "waivers:\n  - rule: lint_770\n    files: [src, thrid]\n  - rule: lint_770\n    files: third/lib.vhd\n",
    );
    let (found, code, text) = left(&dir, &args);
    assert!(found.is_empty(), "{text}");
    assert_eq!(code, 0, "{text}");
    assert_eq!(text.matches("W0132").count(), 1, "{text}");
    assert!(text.contains("`thrid` (line 3"), "{text}");
    let mut strict = args.clone();
    strict.push("-Werror");
    assert_eq!(left(&dir, &strict).1, 1);
    waivers("waivers:\n  - rule: lint_770\n    files: src\n    files_exclude: [no_such]\n");
    let text = left(&dir, &args).2;
    assert!(
        text.contains("W0132") && text.contains("`no_such`"),
        "{text}"
    );
    waivers(
        "waivers:\n  - rule: lint_770\n    files: [src, thrid]\n  - rule: lint_770\n    files: third/lib.vhd\n",
    );
    // A file on disk that this run did not read is no stale pattern.
    let (found, _, text) = left(&dir, &["src/new/fresh.vhd"]);
    assert!(found.is_empty(), "{text}");
    assert_eq!(text.matches("W0132").count(), 1, "{text}");
}

/// One process driving two signals that others drive too: both are reported, in the same order
/// every run.
#[test]
fn two_findings_of_a_rule_at_one_place() {
    let dir = tempdir("lint");
    let source = "library ieee;\nuse ieee.std_logic_1164.all;\nentity e is end entity;\narchitecture r of e is\n  signal a, b : std_logic;\nbegin\n  p : process begin a <= '1'; b <= '1'; wait; end process;\n  a <= '0';\n  b <= '0';\nend architecture;\n";
    std::fs::write(dir.join("e.vhd"), source).unwrap();
    std::fs::write(
        dir.join("yavs-lint.yaml"),
        "rule:\n  lint_601:\n    disable: false\n",
    )
    .unwrap();
    let args = ["lint", "--message-format=short", "e.vhd"];
    let first = stderr(&yavs(&dir, &args));
    assert_eq!(first.matches("error[lint_601]").count(), 2, "{first}");
    assert!(first.contains("`a`") && first.contains("`b`"), "{first}");
    for _ in 0..5 {
        assert_eq!(stderr(&yavs(&dir, &args)), first);
    }
}

#[test]
fn every_report_format() {
    let dir = tempdir("lint");
    two(&dir);
    let o = yavs(
        &dir,
        &[
            "lint",
            "two.vhd",
            "--json=r.json",
            "--sarif=r.sarif",
            "--junit=r.xml",
            "--quality-report=q.json",
            "--sonarqube=s.json",
        ],
    );
    assert_eq!(status(&o), 1, "{}", stderr(&o));
    let read = |f: &str| std::fs::read_to_string(dir.join(f)).unwrap();
    let json: serde_json::Value = serde_json::from_str(&read("r.json")).unwrap();
    assert_eq!(json["files"][0]["file_path"], "two.vhd");
    assert_eq!(json["files"][0]["violations"][1]["rule"], "lint_770");
    assert_eq!(json["files"][0]["violations"][1]["linenumber"], 5);
    assert_eq!(json["files"][0]["violations"][1]["severity"], "Error");
    let sarif: serde_json::Value = serde_json::from_str(&read("r.sarif")).unwrap();
    assert_eq!(sarif["version"], "2.1.0");
    let run = &sarif["runs"][0];
    assert_eq!(
        run["tool"]["driver"]["rules"][0]["properties"]["tags"][0],
        "definite"
    );
    assert_eq!(run["results"][0]["ruleId"], "lint_770");
    assert_eq!(
        run["results"][0]["locations"][0]["physicalLocation"]["region"]["startLine"],
        4
    );
    let quality: serde_json::Value = serde_json::from_str(&read("q.json")).unwrap();
    assert_eq!(quality[0]["categories"][0], "Bug Risk");
    assert_eq!(quality[0]["location"]["lines"]["begin"], 4);
    assert_ne!(quality[0]["fingerprint"], quality[1]["fingerprint"]);
    let sonar: serde_json::Value = serde_json::from_str(&read("s.json")).unwrap();
    assert_eq!(sonar["issues"][0]["type"], "BUG");
    assert_eq!(
        sonar["issues"][0]["primaryLocation"]["textRange"]["startColumn"],
        2
    );
    let junit = read("r.xml");
    assert!(junit.contains("failures=\"1\" tests=\"1\""), "{junit}");
    assert!(junit.contains("lint_770: 4 :"), "{junit}");
    let o = yavs(&dir, &["lint", "two.vhd", "--json=no/such/dir/r.json"]);
    assert_eq!(status(&o), 2);
    assert!(stderr(&o).contains("E0135"), "{}", stderr(&o));
}

const PKG: &str = "package pk is\n  function f return integer;\nend package;\npackage body pk is\n  function f return integer is\n  begin\n  end function;\nend package body;\n";
const USER: &str = "use work.pk.all;\nentity user is end entity;\narchitecture r of user is\n  constant c : integer := 1;\nbegin\n  p : process begin report integer'image(c); end process;\nend architecture;\n";

/// A library whose order no longer works, as later files replaced a unit (#999): an earlier file
/// reads it from the last, a file between brings its own, and the last is then read again. The
/// cache changes nothing.
#[test]
fn a_replaced_unit_read_again_with_and_without_the_cache() {
    let dir = tempdir("lint");
    std::fs::write(dir.join("x.vhd"), "entity leaf is end entity;\n").unwrap();
    std::fs::write(
        dir.join("user.vhd"),
        "entity top is end entity;\narchitecture r of top is begin u : entity work.leaf; end architecture;\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("later.vhd"),
        "entity leaf is end entity;\narchitecture a of leaf is begin end architecture;\n",
    )
    .unwrap();
    let analyze = yavs(
        &dir,
        &[
            "analyze",
            "--work=work:lib",
            "x.vhd",
            "user.vhd",
            "later.vhd",
        ],
    );
    assert!(analyze.status.success(), "{}", stderr(&analyze));
    for extra in [&[][..], &["--no-cache"][..]] {
        let mut args = vec!["lint", "--work=work:lib", "--message-format=short"];
        args.extend(extra);
        let o = yavs(&dir, &args);
        assert_eq!(stderr(&o), "", "{extra:?}");
        assert_eq!(status(&o), 0, "{extra:?}");
    }
}

#[test]
fn files_of_a_library_in_its_order_and_nothing_written() {
    let dir = tempdir("lint");
    std::fs::write(dir.join("b_pk.vhd"), PKG).unwrap();
    std::fs::write(dir.join("a_user.vhd"), USER).unwrap();
    let analyze = yavs(
        &dir,
        &["analyze", "--work=mine:lib", "b_pk.vhd", "a_user.vhd"],
    );
    assert!(analyze.status.success(), "{}", stderr(&analyze));
    let listing = || {
        let mut v: Vec<_> = walk(&dir.join("lib"));
        v.sort();
        v
    };
    let before = listing();

    // No file named: every file of the library.
    let o = yavs(&dir, &["lint", "--work=mine:lib", "--message-format=short"]);
    let found = findings(&stderr(&o));
    assert_eq!(found.len(), 2, "{}", stderr(&o));
    assert_eq!(status(&o), 1);

    // A directory: its files in the order of the library, though `a_user` sorts first.
    let o = yavs(
        &dir,
        &["lint", "--work=mine:lib", "--message-format=short", "."],
    );
    assert_eq!(findings(&stderr(&o)).len(), 2, "{}", stderr(&o));

    // One file, its dependency read from the library; a finding of the other is not reported.
    let o = yavs(
        &dir,
        &[
            "lint",
            "--work=mine:lib",
            "--message-format=short",
            "a_user.vhd",
        ],
    );
    assert_eq!(
        findings(&stderr(&o)),
        vec![(6, "lint_770".to_string())],
        "{}",
        stderr(&o)
    );

    assert_eq!(
        listing(),
        before,
        "a lint run writes nothing into a library"
    );

    // The same with another library named by `--map`.
    let o = yavs(&dir, &["lint", "--work=other:lib2", "--map=mine:lib"]);
    assert_eq!(status(&o), 2, "the work library records nothing");
    assert!(stderr(&o).contains("E0133"), "{}", stderr(&o));
}

fn walk(d: &Path) -> Vec<(PathBuf, u64)> {
    let mut out = Vec::new();
    for e in std::fs::read_dir(d).unwrap() {
        let p = e.unwrap().path();
        if p.is_dir() {
            out.extend(walk(&p));
        } else {
            let m = std::fs::metadata(&p).unwrap();
            out.push((p, m.len()));
        }
    }
    out
}

#[test]
fn directories_and_recursion() {
    let dir = tempdir("lint");
    std::fs::create_dir_all(dir.join("src/deep")).unwrap();
    two(&dir.join("src"));
    std::fs::write(dir.join("src/deep/pk.vhdl"), PKG).unwrap();
    std::fs::write(dir.join("src/notes.txt"), "not vhdl").unwrap();
    let o = yavs(&dir, &["lint", "--message-format=short", "src"]);
    assert_eq!(findings(&stderr(&o)).len(), 2, "{}", stderr(&o));
    let o = yavs(
        &dir,
        &["lint", "--message-format=short", "--recursive", "src"],
    );
    assert_eq!(findings(&stderr(&o)).len(), 3, "{}", stderr(&o));
    let empty = dir.join("empty");
    std::fs::create_dir(&empty).unwrap();
    let o = yavs(&dir, &["lint", "empty"]);
    assert_eq!(status(&o), 2);
    assert!(stderr(&o).contains("E0133"), "{}", stderr(&o));
}

#[test]
fn analysis_errors_stop_the_file_not_the_run() {
    let dir = tempdir("lint");
    two(&dir);
    std::fs::write(
        dir.join("bad.vhd"),
        "entity bad is\n  port (a : no_such_type);\nend entity;\n",
    )
    .unwrap();
    let o = yavs(
        &dir,
        &["lint", "--message-format=short", "bad.vhd", "two.vhd"],
    );
    assert_eq!(status(&o), 2);
    assert!(stderr(&o).contains("error[E"), "{}", stderr(&o));
    assert_eq!(findings(&stderr(&o)).len(), 2, "{}", stderr(&o));
}

/// The problem matcher of the VS Code extension takes the short format apart as it is written.
#[test]
fn problem_matcher_reads_the_short_format() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let manifest = std::fs::read_to_string(root.join("editors/vscode/package.json")).unwrap();
    let manifest: serde_json::Value = serde_json::from_str(&manifest).unwrap();
    let pattern = &manifest["contributes"]["problemMatchers"][0]["pattern"];
    assert_eq!(
        pattern["regexp"].as_str().unwrap(),
        r"^(.+?):(\d+):(\d+): (error|warning|note)\[([^\]]*)\]: (.*)$"
    );
    let dir = tempdir("lint");
    two(&dir);
    let o = yavs(&dir, &["lint", "--message-format=short", "two.vhd"]);
    assert!(
        stderr(&o).starts_with("two.vhd:4:3: error[lint_770]: Process `p` can never suspend"),
        "{}",
        stderr(&o)
    );
}

#[test]
fn usage() {
    let dir = tempdir("lint");
    let o = yavs(&dir, &["lint"]);
    assert_eq!(status(&o), 2);
    assert!(stderr(&o).contains("E0133"), "{}", stderr(&o));
    let o = yavs(&dir, &["lint", "no_such.vhd"]);
    assert_eq!(status(&o), 2);
    assert!(stderr(&o).contains("no_such.vhd"), "{}", stderr(&o));

    let o = yavs(&dir, &["lint", "--explain", "lint_740"]);
    assert_eq!(status(&o), 0);
    let text = String::from_utf8_lossy(&o.stdout);
    assert!(text.starts_with("lint_740 (definite error):"), "{text}");
    let top = yavs(&dir, &["--explain", "lint_740"]);
    assert_eq!(top.stdout, o.stdout, "the same text through the top level");
    let o = yavs(&dir, &["lint", "--explain", "lint_100"]);
    assert_eq!(status(&o), 2);
    assert!(stderr(&o).contains("`lint_100`"), "{}", stderr(&o));
    let o = yavs(&dir, &["lint", "-Wno-lint_100", "--list-rules"]);
    assert_eq!(
        status(&o),
        0,
        "--list-rules looks at no option but the configuration"
    );
    let o = yavs(&dir, &["lint", "--work=std:x", "a.vhd"]);
    assert_eq!(status(&o), 2);
    assert!(stderr(&o).contains("E0102"), "{}", stderr(&o));
    // The short format, which prints no notes, still names what is wrong.
    let bad = "rule:\n  lint_770:\n    severity: fatal\n";
    std::fs::write(dir.join("bad.yaml"), bad).unwrap();
    let o = yavs(
        &dir,
        &[
            "lint",
            "--message-format=short",
            "--config=bad.yaml",
            "a.vhd",
        ],
    );
    assert_eq!(status(&o), 2);
    assert!(stderr(&o).contains("`fatal`"), "{}", stderr(&o));
}

/// #1037: a waiver or `exclude` pattern names a file however the file, or the pattern, passes a
/// symlink or `..`.
#[cfg(unix)]
#[test]
fn patterns_match_a_file_spelled_through_a_symlink() {
    let dir = tempdir("lint-symlink");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::os::unix::fs::symlink(dir.join("src"), dir.join("link")).unwrap();
    std::fs::write(
        dir.join("src/t.vhd"),
        "entity t is end entity;\narchitecture r of t is\nbegin\n  p : process begin report \"x\"; end process;\nend architecture;\n",
    )
    .unwrap();
    let reported = |file: &str| {
        let o = yavs(&dir, &["lint", "--message-format=short", file]);
        stderr(&o).contains("lint_770")
    };
    for file in ["src/t.vhd", "sub/../src/t.vhd", "link/t.vhd"] {
        assert!(reported(file), "{file}");
        for pattern in ["src/t.vhd", "link/t.vhd", "src", "link/", "sub/../link"] {
            std::fs::write(
                dir.join("yavs-lint.yaml"),
                format!("exclude: ['{pattern}']\n"),
            )
            .unwrap();
            assert!(!reported(file), "exclude {pattern} for {file}");
            std::fs::remove_file(dir.join("yavs-lint.yaml")).unwrap();
            std::fs::write(
                dir.join("yavs-lint-waivers.yaml"),
                format!("waivers:\n  - rule: lint_770\n    files: '{pattern}'\n"),
            )
            .unwrap();
            assert!(!reported(file), "waiver {pattern} for {file}");
            std::fs::remove_file(dir.join("yavs-lint-waivers.yaml")).unwrap();
        }
    }
}

/// #1041: a file the work library records keeps the library's standard however it is named. The
/// file uses `context` as a name, which VHDL-2008 reserves.
#[cfg(unix)]
#[test]
fn a_recorded_file_named_through_a_symlink_keeps_its_standard() {
    let dir = tempdir("lint-recorded");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    std::os::unix::fs::symlink(dir.join("src"), dir.join("link")).unwrap();
    std::fs::write(
        dir.join("src/c.vhd"),
        "entity c is end entity;\narchitecture r of c is\n  signal context : bit;\nbegin\n  p : process begin report \"x\"; end process;\nend architecture;\n",
    )
    .unwrap();
    let o = yavs(
        &dir,
        &["analyze", "--std", "93", "--work=work:w", "src/c.vhd"],
    );
    assert_eq!(status(&o), 0, "{}", stderr(&o));
    for file in ["src/c.vhd", "link/c.vhd", "sub/../src/c.vhd"] {
        let o = yavs(
            &dir,
            &["lint", "--message-format=short", "--work=work:w", file],
        );
        let said = stderr(&o);
        assert!(said.contains("lint_770"), "{file}: {said}");
        assert!(!said.contains("E0002"), "{file}: {said}");
    }
}

/// #1151: a source with Windows line endings is linted as its LF twin: no U+240D, the same
/// report, carets and columns.
#[test]
fn crlf_source_reports_match_lf() {
    let text = std::fs::read_to_string(case("latches.vhd")).unwrap();
    let cfg = "rule:\n  lint_600:\n    disable: false\n";
    let run = |text: String, format: &str| {
        let dir = tempdir("lint_crlf");
        std::fs::write(dir.join("yavs-lint.yaml"), cfg).unwrap();
        std::fs::write(dir.join("l.vhd"), text).unwrap();
        let o = yavs(
            &dir,
            &["lint", &format!("--message-format={format}"), "l.vhd"],
        );
        assert_eq!(status(&o), 1, "{}", stderr(&o));
        stderr(&o)
    };
    for format in ["human", "short"] {
        let lf = run(text.clone(), format);
        let crlf = run(text.replace('\n', "\r\n"), format);
        assert!(lf.contains("lint_600"), "{lf}");
        assert!(!crlf.contains('\u{240d}'), "{crlf}");
        assert_eq!(lf, crlf);
    }
}
