//! `yavs run --cov`, `yavs cov merge` and `yavs cov report` on `tests/cli/cov_run.vhd`, whose
//! statement and branch counts are known (M9, #131).

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use yavs_cov::{Database, Kind};

fn tempdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("yavs-cov-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("mkdir");
    d
}

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

    // An unclosed `off`.
    let err = cov_run(&tmp, &EXCL.replace("-- coverage on", "-- nothing"));
    assert!(err.contains("warning[W0112]"), "{err}");
    let _ = std::fs::remove_dir_all(&tmp);
}
