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
    assert_eq!(status(&o), 2, "`suffixes` belongs to lint_602 alone");
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

/// lint_782 names what analysis rejects first (E1403), so the run says so with that code, and the
/// rule stays only for a configuration to name.
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
    ] {
        std::fs::write(dir.join(name), source).unwrap();
        let o = yavs(&dir, &["lint", "--message-format=short", name]);
        assert!(stderr(&o).contains(code), "{}", stderr(&o));
        assert_eq!(status(&o), 2);
    }
    let cfg = "rule:\n  lint_751:\n    disable: true\n  lint_782:\n    disable: true\n";
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

    let other = dir.join("other.yaml");
    std::fs::write(&other, "rule:\n  no_such_rule:\n    disable: true\n").unwrap();
    let o = yavs(&dir, &["lint", "--config", "other.yaml", "two.vhd"]);
    assert_eq!(status(&o), 2);
    assert!(stderr(&o).contains("E0134"), "{}", stderr(&o));
    assert!(stderr(&o).contains("`no_such_rule`"), "{}", stderr(&o));
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
}
