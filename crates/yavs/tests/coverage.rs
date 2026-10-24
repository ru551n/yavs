//! `yavs run --cov`, `yavs cov merge` and `yavs cov report` on `tests/cli/cov_run.vhd`, whose
//! statement and branch counts are known (M9, #131), and `tests/cli/cov_toggle.vhd`, whose
//! toggle counts are (#343).

mod common;
use common::tempdir;

use std::path::Path;
use std::process::{Command, Output};

use yavs_cov::{Database, Kind};

fn yavs(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(["--color", "never"])
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run")
}

fn read(p: &Path) -> Database {
    Database::parse(&std::fs::read_to_string(p).expect("read")).expect("parse")
}

/// (kind, path, line, count), sorted.
fn counts(db: &Database) -> Vec<(String, String, u32, u64)> {
    let mut v: Vec<_> = db
        .items
        .iter()
        .map(|(k, e)| {
            let kind = match k.kind {
                Kind::Statement => "s".to_string(),
                Kind::Branch(a) => format!("b{a}"),
                Kind::Toggle(r) => format!("t{}", u8::from(r)),
                Kind::Condition(r) => format!("c{r}"),
                Kind::Expression(r) => format!("e{r}"),
            };
            assert!(e.file.ends_with("cov_run.vhd"), "{}", e.file);
            (kind, k.path.clone(), e.line, e.count)
        })
        .collect();
    v.sort();
    v
}

const EXPECTED: &[(&str, &str, u32, u64)] = &[
    // `s <= 1 when a = 5 else 2`: runs at 0, on a = 1 and on a = 7.
    ("b0", ":cov_run", 47, 0),
    ("b1", ":cov_run", 47, 3),
    // The `for` body three times, the `while` body never, `if` takes its first arm.
    ("b0", ":cov_run", 51, 3),
    ("b0", ":cov_run", 55, 0),
    ("b0", ":cov_run", 58, 1),
    ("b1", ":cov_run", 58, 0),
    ("s", ":cov_run", 47, 3),
    ("s", ":cov_run", 51, 1),
    ("s", ":cov_run", 52, 3),
    ("s", ":cov_run", 54, 1),
    ("s", ":cov_run", 55, 1),
    ("s", ":cov_run", 56, 0),
    ("s", ":cov_run", 58, 1),
    ("s", ":cov_run", 59, 1),
    ("s", ":cov_run", 61, 0),
    ("s", ":cov_run", 63, 1),
    ("s", ":cov_run", 64, 1),
    ("s", ":cov_run", 65, 1),
    // u1 sees a = 0, 1, 7: every case arm once.
    ("b0", ":cov_run:u1", 26, 1),
    ("b1", ":cov_run:u1", 26, 1),
    ("b2", ":cov_run:u1", 26, 1),
    ("s", ":cov_run:u1", 26, 3),
    ("s", ":cov_run:u1", 27, 1),
    ("s", ":cov_run:u1", 28, 1),
    ("s", ":cov_run:u1", 29, 1),
    // u2 sees only b = 0.
    ("b0", ":cov_run:u2", 26, 1),
    ("b1", ":cov_run:u2", 26, 0),
    ("b2", ":cov_run:u2", 26, 0),
    ("s", ":cov_run:u2", 26, 1),
    ("s", ":cov_run:u2", 27, 1),
    ("s", ":cov_run:u2", 28, 0),
    ("s", ":cov_run:u2", 29, 0),
    // `sign(1 to 3)`: the `elsif` and the missing `else` never run. Nothing of ieee is
    // counted.
    ("b0", "work.cov_pkg", 9, 3),
    ("b1", "work.cov_pkg", 9, 0),
    ("b2", "work.cov_pkg", 9, 0),
    ("s", "work.cov_pkg", 9, 3),
    ("s", "work.cov_pkg", 10, 3),
    ("s", "work.cov_pkg", 12, 0),
    ("s", "work.cov_pkg", 14, 0),
];

#[test]
fn run_merge_report() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/cli/cov_run.vhd");
    let tmp = tempdir("run");
    let lib = format!("--work=work:{}/lib", tmp.display());
    let src = src.to_str().expect("utf-8 path");
    assert!(yavs(&tmp, &["analyze", &lib, src]).status.success());
    let run = |db: &str, extra: &[&str]| {
        let o = yavs(&tmp, &[&["run", &lib, "cov_run", db][..], extra].concat());
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    };
    run("--cov=a.ycov", &[]);
    let a = read(&tmp.join("a.ycov"));
    let mut want: Vec<_> = EXPECTED
        .iter()
        .map(|&(k, p, l, c)| (k.to_string(), p.to_string(), l, c))
        .collect();
    want.sort();
    assert_eq!(counts(&a), want);

    // Byte-identical at four threads.
    run("--cov=a4.ycov", &["--threads", "4"]);
    assert_eq!(
        std::fs::read(tmp.join("a.ycov")).expect("a"),
        std::fs::read(tmp.join("a4.ycov")).expect("a4")
    );

    // A second run (the `for` loop not entered), merged: every count is the sum.
    run("--cov=b.ycov", &["-gn=0"]);
    let b = read(&tmp.join("b.ycov"));
    let o = yavs(&tmp, &["cov", "merge", "a.ycov", "b.ycov", "-o", "c.ycov"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let c = read(&tmp.join("c.ycov"));
    assert_eq!(c.items.len(), a.items.len());
    for (k, e) in &c.items {
        assert_eq!(e.count, a.items[k].count + b.items[k].count, "{k:?}");
    }
    let loop_body = |db: &Database| {
        counts(db)
            .into_iter()
            .find(|x| x.0 == "b0" && x.2 == 51)
            .map(|x| x.3)
    };
    assert_eq!((loop_body(&b), loop_body(&c)), (Some(0), Some(3)));

    let o = yavs(&tmp, &["cov", "report", "a.ycov", "--cobertura", "a.xml"]);
    assert!(o.status.success());
    let text = String::from_utf8(o.stdout).expect("utf-8");
    assert!(
        text.starts_with("statements 18/24 75.0%\nbranches   8/15 53.3%\n"),
        "{text}"
    );
    assert!(text.contains(":cov_run:u2   2/4 50.0%"), "{text}");
    let xml = std::fs::read_to_string(tmp.join("a.xml")).expect("xml");
    assert!(
        xml.contains(
            "<line number=\"26\" hits=\"4\" branch=\"true\" condition-coverage=\"100% (3/3)\"/>"
        ),
        "{xml}"
    );
    assert!(
        xml.contains("<line number=\"56\" hits=\"0\" branch=\"false\"/>"),
        "{xml}"
    );

    // A missing database is E0111, exit 1.
    let o = yavs(&tmp, &["cov", "report", "nope.ycov"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).contains("error[E0111]"));

    // Without `--cov` nothing is written.
    let o = yavs(&tmp, &["run", &lib, "cov_run"]);
    assert!(o.status.success());
    assert!(!tmp.join("yavs.ycov").exists());
    let _ = std::fs::remove_dir_all(&tmp);
}

const EXCL: &str = "entity cov_excl is end entity;
architecture sim of cov_excl is
  signal s : integer := 0;
begin
  p : process
  begin
    if s = 1 then
      s <= 2;
    end if;
    -- coverage off: debug only
    s <= 4;
    s <= 5;
    -- coverage on
    s <= 3; -- coverage off-line
    wait;
  end process;
end architecture;
";

/// Analyze and run `src`, leaving the database in `a.ycov`; returns stderr of the run.
fn cov_run(tmp: &Path, src: &str) -> String {
    std::fs::write(tmp.join("excl.vhd"), src).expect("write");
    assert!(
        yavs(tmp, &["analyze", "--work=work:lib", "excl.vhd"])
            .status
            .success()
    );
    let o = yavs(tmp, &["run", "--work=work:lib", "cov_excl", "--cov=a.ycov"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    String::from_utf8_lossy(&o.stderr).into_owned()
}

/// A report without the `[reason]` ends of its excluded lines: `--uncovered=yaml` leaves the
/// reason to the user, the text rows carry the item's text as one.
fn no_reasons(s: &str) -> String {
    s.lines()
        .map(|l| l.split("  [").next().unwrap_or(l))
        .map(|l| format!("{l}\n"))
        .collect()
}

fn report(tmp: &Path, args: &[&str]) -> (Option<i32>, String, String) {
    let o = yavs(tmp, &[&["cov", "report", "a.ycov"][..], args].concat());
    (
        o.status.code(),
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

#[test]
fn exclusions() {
    let tmp = tempdir("excl");
    // Pragma regions and `off-line` leave their statements out; nothing to warn about.
    let err = cov_run(&tmp, EXCL);
    assert!(!err.contains("W01"), "{err}");
    let db = read(&tmp.join("a.ycov"));
    let texts: Vec<_> = db.items.values().map(|e| e.text.as_str()).collect();
    assert!(
        !texts
            .iter()
            .any(|t| t.contains("s <= 4") || t.contains("s <= 5") || t.contains("s <= 3")),
        "{texts:?}"
    );

    // The report lines are the exclusion file: exactly the misses go.
    let (_, rows, _) = report(&tmp, &["--uncovered"]);
    assert_eq!(rows.lines().count(), 2, "{rows}");
    assert!(
        rows.contains("s <= 2;") && rows.contains("arm 0:"),
        "{rows}"
    );
    std::fs::write(tmp.join("ex.txt"), format!("# why\n{rows}")).expect("write");
    let (code, out, err) = report(&tmp, &["--exclude", "ex.txt"]);
    assert_eq!((code, err.as_str()), (Some(0), ""), "{out}");
    assert!(out.contains("excluded (2, not in the totals)"), "{out}");
    assert!(!out.contains("not covered"), "{out}");

    // Coarse forms: `*` instance and a whole instance.
    std::fs::write(tmp.join("ex2.txt"), rows.replace(":cov_excl", "*")).expect("write");
    let (_, out, err) = report(&tmp, &["--exclude", "ex2.txt"]);
    assert!(err.is_empty() && out.contains("excluded (2"), "{out}{err}");
    std::fs::write(tmp.join("ex3.txt"), "instance :cov_excl\n").expect("write");
    let (_, out, _) = report(&tmp, &["--exclude", "ex3.txt"]);
    assert!(
        out.contains("excluded (") && !out.contains("not covered"),
        "{out}"
    );

    // The statement moves down: still excluded, with a warning naming the new line.
    cov_run(&tmp, &format!("\n\n\n{EXCL}"));
    let (code, out, err) = report(&tmp, &["--exclude", "ex.txt"]);
    assert_eq!(code, Some(0));
    assert_eq!(err.matches("warning[W0110]").count(), 2, "{err}");
    assert!(out.contains("excluded (2"), "{out}");

    // The statement changes: the row is stale, and `-Werror` fails the report.
    cov_run(&tmp, &EXCL.replace("s <= 2;", "s <= 22;"));
    let (code, out, err) = report(&tmp, &["--exclude", "ex.txt"]);
    assert_eq!(code, Some(0));
    assert!(err.contains("warning[W0111]"), "{err}");
    assert!(out.contains("not covered"), "{out}");
    let (code, _, _) = report(&tmp, &["--exclude", "ex.txt", "-Werror"]);
    assert_eq!(code, Some(1));

    // Excluded but it ran, and a malformed row.
    cov_run(&tmp, EXCL);
    std::fs::write(tmp.join("ex4.txt"), "file excl.vhd\n").expect("write");
    let (_, _, err) = report(&tmp, &["--exclude", "ex4.txt"]);
    assert!(err.contains("warning[W0114]"), "{err}");
    std::fs::write(tmp.join("ex5.txt"), "frob x\n").expect("write");
    let (code, _, err) = report(&tmp, &["--exclude", "ex5.txt"]);
    assert_eq!(code, Some(1));
    assert!(
        err.contains("error[E0112]") && err.contains("line 1"),
        "{err}"
    );

    // The YAML form of the same exclusions gives the same report; extension picks the parser.
    let (_, text_out, _) = report(&tmp, &["--exclude", "ex.txt"]);
    let (_, yaml, _) = report(&tmp, &["--uncovered=yaml"]);
    assert!(yaml.starts_with("exclude:\n  - kind: "), "{yaml}");
    assert_eq!(yaml.matches("hash: \"").count(), 2, "{yaml}");
    for name in ["ex.yaml", "ex.YML"] {
        std::fs::write(tmp.join(name), &yaml).expect("write");
        let (code, out, err) = report(&tmp, &["--exclude", name]);
        assert_eq!((code, err.as_str()), (Some(0), ""), "{name}");
        assert_eq!(out, no_reasons(&text_out), "{name}");
    }
    // Coarse kinds, a reason, and a hand-written string-typing trap (`on`).
    std::fs::write(
        tmp.join("ex6.yml"),
        "exclude:\n  - {kind: instance, instance: ':cov_excl', reason: on}\n",
    )
    .expect("write");
    let (code, out, err) = report(&tmp, &["--exclude", "ex6.yml"]);
    // Everything ran, so the exclusion warns (W0114) but applies.
    assert!(code == Some(0) && err.contains("warning[W0114]"), "{err}");
    assert!(out.contains("[on]"), "{out}");
    // A malformed YAML file names the line.
    std::fs::write(tmp.join("ex7.yaml"), "exclude:\n  - kind: frob\n").expect("write");
    let (code, _, err) = report(&tmp, &["--exclude", "ex7.yaml"]);
    assert_eq!(code, Some(1));
    assert!(
        err.contains("error[E0112]") && err.contains("ex7.yaml") && err.contains("line 2"),
        "{err}"
    );

    // An unclosed `off`.
    let err = cov_run(&tmp, &EXCL.replace("-- coverage on", "-- nothing"));
    assert!(err.contains("warning[W0112]"), "{err}");
    let _ = std::fs::remove_dir_all(&tmp);
}

/// (path, scalar, rises, falls) of `tests/cli/cov_toggle.vhd`.
const TOGGLES: &[(&str, &str, u64, u64)] = &[
    (":cov_toggle", "clk", 3, 2),
    // u1's output: 'U' -> '1' at start is no toggle; 'H' and '1' on s each give a fall.
    (":cov_toggle", "q", 0, 2),
    (":cov_toggle", "r.valid", 0, 0),
    // '0' -> 'H' rises, 'H' -> 'Z' -> '0' is no fall, '0' -> '1' rises.
    (":cov_toggle", "s", 2, 0),
    (":cov_toggle", "v(0)", 1, 1),
    (":cov_toggle", "v(1)", 0, 0),
    (":cov_toggle:u1", "d", 2, 0),
    (":cov_toggle:u1", "q", 0, 2),
];

fn toggles(db: &Database) -> Vec<(String, String, u64, u64)> {
    let mut m = std::collections::BTreeMap::new();
    for (k, e) in &db.items {
        if let Kind::Toggle(r) = k.kind {
            let c: &mut (u64, u64) = m.entry((k.path.clone(), k.sig.clone())).or_default();
            *(if r { &mut c.0 } else { &mut c.1 }) = e.count;
        }
    }
    m.into_iter().map(|((p, s), (r, f))| (p, s, r, f)).collect()
}

#[test]
fn toggle_coverage() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/cli/cov_toggle.vhd");
    let tmp = tempdir("toggle");
    let lib = format!("--work=work:{}/lib", tmp.display());
    assert!(
        yavs(&tmp, &["analyze", &lib, src.to_str().expect("utf-8")])
            .status
            .success()
    );
    let run = |extra: &[&str]| {
        let o = yavs(&tmp, &[&["run", &lib, "cov_toggle"][..], extra].concat());
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8_lossy(&o.stderr).into_owned()
    };
    run(&["--cov=a.ycov"]);
    let a = read(&tmp.join("a.ycov"));
    let want: Vec<_> = TOGGLES
        .iter()
        .map(|&(p, s, r, f)| (p.to_string(), s.to_string(), r, f))
        .collect();
    assert_eq!(toggles(&a), want);

    // Byte-identical at four threads; merge adds the counts.
    run(&["--cov=a4.ycov", "--threads", "4"]);
    assert_eq!(
        std::fs::read(tmp.join("a.ycov")).expect("a"),
        std::fs::read(tmp.join("a4.ycov")).expect("a4")
    );
    let o = yavs(&tmp, &["cov", "merge", "a.ycov", "a4.ycov", "-o", "m.ycov"]);
    assert!(o.status.success());
    let m = toggles(&read(&tmp.join("m.ycov")));
    assert_eq!(m.len(), want.len());
    for (x, y) in m.iter().zip(&want) {
        assert_eq!((x.2, x.3), (2 * y.2, 2 * y.3), "{x:?}");
    }

    // Only toggles; the cap leaves `v` (two scalars) out with W0115.
    let err = run(&["--cov=t.ycov", "--cov-kind=toggle", "--cov-toggle-max=1"]);
    assert!(
        err.contains("warning[W0115]") && err.contains(":cov_toggle:v"),
        "{err}"
    );
    let t = read(&tmp.join("t.ycov"));
    assert!(
        t.items
            .keys()
            .all(|k| matches!(k.kind, Kind::Toggle(_)) && !k.sig.starts_with('v'))
    );
    assert_eq!(toggles(&t).len(), want.len() - 2);

    // The summary, and `--uncovered` rows that exclude exactly the misses.
    let o = yavs(&tmp, &["cov", "report", "a.ycov"]);
    let text = String::from_utf8(o.stdout).expect("utf-8");
    assert!(text.contains("\ntoggles    8/16 50.0%\n"), "{text}");
    let o = yavs(&tmp, &["cov", "report", "a.ycov", "--uncovered"]);
    let rows = String::from_utf8(o.stdout).expect("utf-8");
    let toggle_rows: Vec<_> = rows.lines().filter(|l| l.starts_with("toggle ")).collect();
    assert!(
        rows.lines()
            .any(|l| l.starts_with("toggle :cov_toggle v(1)   # never toggled")),
        "{rows}"
    );
    assert!(
        rows.lines()
            .any(|l| l.starts_with("toggle :cov_toggle s fall   # never 1->0")),
        "{rows}"
    );
    assert_eq!(toggle_rows.len(), 6, "{rows}");
    std::fs::write(tmp.join("ex.txt"), &rows).expect("write");
    let o = yavs(&tmp, &["cov", "report", "a.ycov", "--exclude", "ex.txt"]);
    let (out, err) = (
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr),
    );
    assert!(err.is_empty(), "{err}");
    // Each row takes out the missed direction only; what ran stays.
    assert!(out.contains("toggles    8/8 100.0%"), "{out}");
    // The same as YAML.
    let o = yavs(&tmp, &["cov", "report", "a.ycov", "--uncovered=yaml"]);
    let y = String::from_utf8(o.stdout).expect("utf-8");
    assert!(y.starts_with("exclude:\n  - kind: toggle"), "{y}");
    assert!(y.contains("    direction: fall\n"), "{y}");
    std::fs::write(tmp.join("ex.yaml"), &y).expect("write");
    let o = yavs(&tmp, &["cov", "report", "a.ycov", "--exclude", "ex.yaml"]);
    assert!(
        o.stderr.is_empty(),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert_eq!(
        no_reasons(&String::from_utf8_lossy(&o.stdout)),
        no_reasons(&out)
    );
    // A glob row over a scalar that toggled both ways warns.
    std::fs::write(tmp.join("ex2.txt"), "toggle * v(*)\n").expect("write");
    let o = yavs(&tmp, &["cov", "report", "a.ycov", "--exclude", "ex2.txt"]);
    let err = String::from_utf8_lossy(&o.stderr);
    assert_eq!(err.matches("warning[W0114]").count(), 2, "{err}");
    // One direction: `s` fell never, so only the rise is excluded and it ran.
    std::fs::write(tmp.join("ex3.txt"), "toggle * s rise\ntoggle * s fall\n").expect("write");
    let o = yavs(&tmp, &["cov", "report", "a.ycov", "--exclude", "ex3.txt"]);
    let err = String::from_utf8_lossy(&o.stderr);
    assert_eq!(err.matches("warning[W0114]").count(), 1, "{err}");
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn per_test_attribution() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/cli/cov_run.vhd");
    let tmp = tempdir("tests");
    let lib = format!("--work=work:{}/lib", tmp.display());
    assert!(
        yavs(&tmp, &["analyze", &lib, src.to_str().expect("utf-8")])
            .status
            .success()
    );
    for (db, extra) in [
        ("a.ycov", &[][..]),
        ("b.ycov", &["-gn=0"][..]),
        ("c.ycov", &["--cov-test=named"][..]),
    ] {
        let o = yavs(
            &tmp,
            &[&["run", &lib, "cov_run", &format!("--cov={db}")][..], extra].concat(),
        );
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    }
    let o = yavs(
        &tmp,
        &["cov", "merge", "a.ycov", "b.ycov", "c.ycov", "-o", "m.ycov"],
    );
    assert!(o.status.success());
    let m = read(&tmp.join("m.ycov"));
    // Default names: the top level, with its generics; `--cov-test` overrides.
    assert_eq!(m.tests, ["cov_run", "cov_run(n=0)", "named"]);
    let who = |kind: Kind, line: u32| -> Vec<&str> {
        let (_, e) = m
            .items
            .iter()
            .find(|(k, e)| k.kind == kind && e.line == line && k.path == ":cov_run")
            .expect("item");
        e.tests
            .iter()
            .map(|&t| m.tests[t as usize].as_str())
            .collect()
    };
    // The `for` body runs only with n = 3; the final `if` in every run.
    assert_eq!(who(Kind::Branch(0), 51), ["cov_run", "named"]);
    assert_eq!(
        who(Kind::Statement, 58),
        ["cov_run", "cov_run(n=0)", "named"]
    );

    let o = yavs(&tmp, &["cov", "rank", "m.ycov"]);
    assert!(o.status.success());
    let r = String::from_utf8(o.stdout).expect("utf-8");
    assert!(r.starts_with("3 tests hit 26 of 39 items\n"), "{r}");
    assert!(r.contains("not needed: cov_run(n=0), named"), "{r}");
    assert!(r.contains("\ncov_run(n=0)        21         0\n"), "{r}");

    let o = yavs(&tmp, &["cov", "report", "m.ycov", "--tests"]);
    let t = String::from_utf8(o.stdout).expect("utf-8");
    assert!(
        t.lines().any(|l| l.starts_with("branch :cov_run ")
            && l.contains("cov_run.vhd:51")
            && l.ends_with("# tests: cov_run, named")),
        "{t}"
    );
    let _ = std::fs::remove_dir_all(&tmp);
}

/// Run `top` of `tests/cli/<file>` with `--cov`, once per `(db, extra)`.
fn cov_dbs(tmp: &Path, file: &str, top: &str, runs: &[(&str, &[&str])]) {
    let src = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/cli")
        .join(file);
    let lib = format!("--work=work:{}/lib", tmp.display());
    assert!(
        yavs(tmp, &["analyze", &lib, src.to_str().expect("utf-8")])
            .status
            .success()
    );
    for (db, extra) in runs {
        let cov = format!("--cov={db}");
        let args = [&["run", &lib, top, &cov][..], extra].concat();
        let o = yavs(tmp, &args);
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    }
}

fn html(tmp: &Path, args: &[&str]) -> Output {
    yavs(tmp, &[&["cov", "report", "--html", "h"][..], args].concat())
}

/// The directory of the sources the tests run (`--root` strips it from the pages).
fn cli_dir() -> String {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/cli")
        .display()
        .to_string()
}

fn page(tmp: &Path, name: &str) -> String {
    std::fs::read_to_string(tmp.join("h").join(name)).expect("page")
}

#[test]
fn html_report() {
    let tmp = tempdir("html");
    cov_dbs(
        &tmp,
        "cov_run.vhd",
        "cov_run",
        &[("a.ycov", &[]), ("b.ycov", &["-gn=0"])],
    );
    let o = yavs(&tmp, &["cov", "merge", "a.ycov", "b.ycov", "-o", "m.ycov"]);
    assert!(o.status.success());
    // Exclude the whole of u2 with a reason.
    std::fs::write(tmp.join("ex.txt"), "instance :cov_run:u2   # dead <b>\n").expect("write");
    let root = cli_dir();
    let o = html(&tmp, &["m.ycov", "--exclude", "ex.txt", "--root", &root]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let index = page(&tmp, "index.html");
    assert!(index.contains("<title>Coverage report</title>"), "{index}");
    assert!(index.contains(">:cov_run:u1</td>") && index.contains("href=\"src-0.html\""));
    assert!(
        !index.contains(":cov_run:u2</td>"),
        "excluded instance is not listed"
    );
    assert!(index.contains("2 tests: cov_run, cov_run(n=0)"), "{index}");
    let src = page(&tmp, "src-0.html");
    // Paths are relative to the root: heading, index and exclusion rows.
    assert!(
        src.contains("<h1>cov_run.vhd</h1>") && !src.contains("tests/cli"),
        "{src}"
    );
    assert!(
        index.contains(">cov_run.vhd</a>") && !index.contains("tests/cli"),
        "{index}"
    );
    // Line 56 never runs: a hoverable missed line whose popup has the copyable exclusion row.
    assert!(
        src.contains("<tr id=\"L56\" class=\"miss\" tabindex=\"0\" aria-describedby=\"p56\">"),
        "{src}"
    );
    assert!(
        src.contains("<code>statement :cov_run ") && src.contains(" cov_run.vhd:56 "),
        "{src}"
    );
    assert!(src.contains("class=\"cp\">copy row</button>"), "{src}");
    // The same item as a YAML entry, with its own button.
    assert!(
        src.contains("<pre>  - kind: statement") && src.contains("class=\"cp\">copy YAML</button>"),
        "{src}"
    );
    // Popups open only on click: no hover or focus rule shows one, Enter and Space toggle.
    assert!(
        src.contains("tr.pin .pop{display:block}")
            && !src.contains("tr:hover .pop")
            && !src.contains("focus-visible")
            && src.contains("e.key==' '")
            && src.contains("click a line"),
        "{src}"
    );
    // No inline item lists, and a line without items is not focusable.
    assert!(!src.contains("class=\"it ") && !src.contains("<tr id=\"L1\" class=\"\" tabindex"));
    // Counts per instance and tests in the popup.
    assert!(
        src.contains("<h4>branch arm 0</h4><table><tr class=\"\"><td>:cov_run</td><td class=\"n\">3</td><td class=\"dim\">tests: cov_run</td>"),
        "{src}"
    );
    // Excluded items are greyed, with the (escaped) reason.
    assert!(
        src.contains("<tr class=\"ex\">") && src.contains("excluded: dead &lt;b&gt;"),
        "{src}"
    );
    // The missed-only page keeps the missed and excluded lines and drops the rest.
    let miss = page(&tmp, "src-0-missed.html");
    assert!(index.contains("href=\"src-0-missed.html\""), "{index}");
    assert!(src.contains("href=\"src-0-missed.html\""), "{src}");
    assert!(miss.contains("href=\"src-0.html\""), "{miss}");
    assert!(miss.contains("<tr id=\"L56\" class=\"miss\""), "{miss}");
    assert!(miss.contains("<tr class=\"ex\">"), "{miss}");
    assert!(src.contains("<tr id=\"L1\"") && !miss.contains("<tr id=\"L1\""));
    assert!(miss.len() < src.len());
    // No external resources (the one script is inline); source text is escaped.
    for p in [&index, &src, &miss] {
        assert!(
            !p.contains("http") && !p.contains("<script src") && !p.contains("src=\""),
            "{p}"
        );
        assert_eq!(p.matches("<script>").count(), 1, "{p}");
    }
    assert!(src.contains("&lt;") && !src.contains("<=  "), "{src}");
    assert!(src.contains("prefers-color-scheme:dark"));
    // Byte-identical for the same database.
    let again = |d: &str| {
        let mut o = yavs(
            &tmp,
            &[
                "cov",
                "report",
                "m.ycov",
                "--exclude",
                "ex.txt",
                "--html",
                d,
                "--root",
                &root,
            ],
        );
        assert!(o.status.success());
        o.stdout.clear();
    };
    again("h2");
    for n in ["index.html", "src-0.html", "src-0-missed.html"] {
        assert_eq!(
            page(&tmp, n),
            std::fs::read_to_string(tmp.join("h2").join(n)).expect("h2")
        );
    }

    // Without the exclusion u2 is a live instance: line 28 ran in u1 only, a partial line.
    let o = yavs(
        &tmp,
        &["cov", "report", "m.ycov", "--html", "h3", "--root", &root],
    );
    assert!(o.status.success());
    let part = std::fs::read_to_string(tmp.join("h3/src-0.html")).expect("h3");
    assert!(part.contains("<tr id=\"L28\" class=\"inst\""), "{part}");
    assert!(
        part.contains("title=\"executed in 1 of 2 instances\">1/2</td>"),
        "{part}"
    );
    assert!(
        part.contains("<p>executed in 1 of 2 instances</p>"),
        "{part}"
    );
    assert!(
        part.contains("<tr class=\"\"><td>:cov_run:u1</td><td class=\"n\">2</td>"),
        "{part}"
    );
    assert!(part.contains(".legend .inst{"), "{part}");
    // Without --root the path is as the database has it.
    let o = yavs(&tmp, &["cov", "report", "m.ycov", "--html", "h4"]);
    assert!(o.status.success());
    let abs = std::fs::read_to_string(tmp.join("h4/src-0.html")).expect("h4");
    // Native separators on Windows.
    assert!(
        abs.replace('\\', "/")
            .contains("tests/cli/cov_run.vhd</h1>"),
        "{abs}"
    );

    // A source the database names that cannot be read is E0113 and writes nothing.
    let text = std::fs::read_to_string(tmp.join("m.ycov")).expect("db");
    let gone = text.replace("cov_run.vhd", "nope.vhd");
    std::fs::write(tmp.join("g.ycov"), gone).expect("write");
    std::fs::remove_dir_all(tmp.join("h")).expect("rm");
    let o = html(&tmp, &["g.ycov"]);
    assert_eq!(o.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&o.stderr).contains("error[E0113]"));
    assert!(!tmp.join("h").exists());
    let _ = std::fs::remove_dir_all(&tmp);
}

#[test]
fn html_toggles() {
    let tmp = tempdir("html-toggle");
    cov_dbs(&tmp, "cov_toggle.vhd", "cov_toggle", &[("t.ycov", &[])]);
    let o = html(&tmp, &["t.ycov"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let src = page(&tmp, "src-0.html");
    assert!(
        src.contains("<h4>toggle v(1) 0-&gt;1</h4><table><tr class=\"miss\"><td>:cov_toggle</td><td class=\"n\">0</td>"),
        "{src}"
    );
    assert!(
        src.contains("<code>toggle :cov_toggle v(1)   # never toggled"),
        "{src}"
    );
    assert!(page(&tmp, "index.html").contains("toggles"));
    let _ = std::fs::remove_dir_all(&tmp);
}

/// Ports collapsed into their actuals, whole or part of a vector (#413), count the same
/// toggles as ports kept apart.
#[test]
fn toggles_of_collapsed_ports() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/corpus/hierarchy/in_port_collapse_part.vhd");
    let tmp = tempdir("collapse");
    let lib = format!("--work=work:{}/lib", tmp.display());
    assert!(
        yavs(&tmp, &["analyze", &lib, src.to_str().expect("utf-8")])
            .status
            .success()
    );
    let run = |db: &str, extra: &[&str]| {
        let cov = format!("--cov={db}");
        let o = yavs(
            &tmp,
            &[&["run", &lib, &cov][..], extra, &["in_port_collapse_part"]].concat(),
        );
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        toggles(&read(&tmp.join(db)))
    };
    let collapsed = run("a.ycov", &[]);
    assert!(
        collapsed
            .iter()
            .any(|t| t.0 == ":in_port_collapse_part:g(1):u"
                && t.1 == "s(1)"
                && (t.2, t.3) == (1, 1)),
        "{collapsed:?}"
    );
    assert_eq!(collapsed, run("b.ycov", &["--no-collapse-ports"]));
}

/// Every kind counts the same in the interpreter and the LLVM tier (compiled counters), at any
/// number of threads.
#[cfg(feature = "llvm")]
#[test]
fn tiers_agree() {
    let tmp = tempdir("tiers");
    let kinds = "--cov-kind=statement,branch,toggle,condition,expression";
    cov_dbs(
        &tmp,
        "cov_run.vhd",
        "cov_run",
        &[
            ("i.ycov", &["--tier=interp", kinds]),
            ("l.ycov", &["--tier=llvm", kinds]),
            ("l4.ycov", &["--tier=llvm", "--threads=4", kinds]),
            ("a.ycov", &["--tier=auto", kinds]),
        ],
    );
    let want = std::fs::read(tmp.join("i.ycov")).expect("i");
    for db in ["l.ycov", "l4.ycov", "a.ycov"] {
        assert!(
            want == std::fs::read(tmp.join(db)).expect("db"),
            "{db} differs from the interpreter's"
        );
    }
    // The design with copies, memoised blocks and inlined functions.
    cov_dbs(
        &tmp,
        "cov_copies.vhd",
        "cov_copies",
        &[
            ("ci.ycov", &["--tier=interp", kinds]),
            ("cl.ycov", &["--tier=llvm", kinds]),
            ("cl4.ycov", &["--tier=llvm", "--threads=4", kinds]),
            ("ca.ycov", &["--tier=auto", kinds]),
        ],
    );
    let want = std::fs::read(tmp.join("ci.ycov")).expect("ci");
    for db in ["cl.ycov", "cl4.ycov", "ca.ycov"] {
        assert!(
            want == std::fs::read(tmp.join(db)).expect("db"),
            "{db} differs from the interpreter's (cov_copies)"
        );
    }
    let _ = std::fs::remove_dir_all(&tmp);
}

/// Under `--cov` the simulation replays the elaboration-time calls of package functions; those
/// ran as lowered, and the process calls after them run the optimised code (#705).
#[test]
fn replayed_calls_run_optimised() {
    let tmp = tempdir("replay");
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/cli/cov_replay.vhd");
    let lib = format!("--work=work:{}/lib", tmp.display());
    let a = yavs(
        &tmp,
        &["analyze", "--std=2008", &lib, src.to_str().expect("utf-8")],
    );
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    let o = yavs(
        &tmp,
        &[
            "run",
            "--std=2008",
            &lib,
            "cov_replay",
            "--cov=r.ycov",
            "--cov-kind=statement",
            "--tier=interp",
            "--no-cache",
        ],
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}

/// Clocked processes keep their edge triggers under coverage (#707): a process whose only wake-up
/// condition is an edge runs at initialisation and at that edge, so the `if` of a pure edge test
/// counts those runs and its false arm only the initial one. Every edge form behaves alike, at one
/// and four threads.
#[test]
fn edge_triggers_count_their_edges() {
    // (kind, line, count) of `tests/cli/cov_edge.vhd`: four rising and four falling edges.
    const WANT: &[(&str, u32, u64)] = &[
        // The clock generator: the loop body four times.
        ("b0", 16, 4),
        ("s", 16, 1),
        ("s", 17, 4),
        ("s", 18, 4),
        ("s", 19, 4),
        ("s", 20, 4),
        ("s", 22, 1),
        // `rising_edge`, `falling_edge` and `'event and = '1'`: one run at start and one per edge.
        ("b0", 27, 4),
        ("b1", 27, 1),
        ("s", 27, 5),
        ("s", 28, 4),
        ("b0", 34, 4),
        ("b1", 34, 1),
        ("s", 34, 5),
        ("s", 35, 4),
        ("b0", 41, 4),
        ("b1", 41, 1),
        ("s", 41, 5),
        ("s", 42, 4),
        // `wait until` on an edge function and on a level.
        ("s", 48, 5),
        ("s", 49, 4),
        ("s", 54, 5),
        ("s", 55, 4),
        // The checker.
        ("s", 60, 1),
        ("s", 61, 1),
        ("s", 65, 1),
        // A plain copy of the clock stays an interpreted process: once at start, once per event.
        ("s", 69, 9),
    ];
    let tmp = tempdir("edge");
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/cli/cov_edge.vhd");
    let lib = format!("--work=work:{}/lib", tmp.display());
    let a = yavs(
        &tmp,
        &["analyze", "--std=2008", &lib, src.to_str().expect("utf-8")],
    );
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    let mut dbs = Vec::new();
    for threads in ["1", "4"] {
        let db = format!("e{threads}.ycov");
        let o = yavs(
            &tmp,
            &[
                "run",
                "--std=2008",
                &lib,
                "cov_edge",
                &format!("--cov={db}"),
                "--cov-kind=statement,branch",
                "--tier=interp",
                "--threads",
                threads,
            ],
        );
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        dbs.push(std::fs::read(tmp.join(&db)).expect("db"));
    }
    assert!(dbs[0] == dbs[1], "counts differ between 1 and 4 threads");
    let mut got: Vec<(String, u32, u64)> = read(&tmp.join("e1.ycov"))
        .items
        .iter()
        .map(|(k, e)| {
            let kind = match k.kind {
                Kind::Statement => "s".to_string(),
                Kind::Branch(a) => format!("b{a}"),
                _ => panic!("unexpected {:?}", k.kind),
            };
            (kind, e.line, e.count)
        })
        .collect();
    got.sort_by(|a, b| (a.1, &a.0).cmp(&(b.1, &b.0)));
    let want: Vec<_> = WANT
        .iter()
        .map(|&(k, l, c)| (k.to_string(), l, c))
        .collect();
    assert_eq!(got, want);
}

/// Per-instance copies, memoised blocks and inlined functions count exactly what the shared
/// code counts: byte-identical databases for every kind, at one and four threads (#699).
#[test]
fn copies_count_alike() {
    let tmp = tempdir("copies");
    let kinds = "--cov-kind=statement,branch,toggle,condition,expression";
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/cli/cov_copies.vhd");
    let lib = format!("--work=work:{}/lib", tmp.display());
    assert!(
        yavs(&tmp, &["analyze", &lib, src.to_str().expect("utf-8")])
            .status
            .success()
    );
    let go = |db: &str, threads: &str, env: &[(&str, &str)]| {
        let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
            .args(["--color", "never", "run", &lib, "cov_copies"])
            .args([&format!("--cov={db}"), kinds, "--tier=interp"])
            .args(["--no-cache", "--threads", threads])
            .envs(env.iter().copied())
            .env("YAVS_COUNTERS", "1")
            .current_dir(&tmp)
            .output()
            .expect("run");
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8_lossy(&o.stderr).into_owned()
    };
    let shared = [
        ("YAVS_NO_SPEC", "1"),
        ("YAVS_NO_INLINE", "1"),
        ("YAVS_NO_MEMO", "1"),
    ];
    go("shared.ycov", "1", &shared);
    let want = std::fs::read(tmp.join("shared.ycov")).expect("shared");
    for threads in ["1", "4"] {
        let db = format!("copy{threads}.ycov");
        let err = go(&db, threads, &[("YAVS_SPEC_AFTER", "2")]);
        let switched = err
            .lines()
            .find_map(|l| l.strip_prefix("counters: spec_switched="))
            .and_then(|v| v.parse::<u64>().ok());
        assert!(switched.is_some_and(|n| n > 0), "no copy ran: {err}");
        assert!(
            want == std::fs::read(tmp.join(&db)).expect("db"),
            "{db} differs from the shared code's counts"
        );
    }
    let _ = std::fs::remove_dir_all(&tmp);
}
