//! Condition and expression coverage (focused expression coverage, #688) on
//! `tests/cli/cov_fec.vhd`: the rows of every condition and expression, thread identity, the
//! reports, merge with per-test attribution, and the tiers.

mod common;
use common::{Tmp, tempdir};

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

/// A fresh directory with the design analysed into `lib`.
fn setup(name: &str) -> Tmp {
    let tmp = tempdir(name);
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/cli/cov_fec.vhd");
    let o = yavs(
        &tmp,
        &["analyze", "--work=work:lib", src.to_str().expect("utf-8")],
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    tmp
}

fn run(tmp: &Path, extra: &[&str]) -> Output {
    let o = yavs(
        tmp,
        &[&["run", "--work=work:lib", "cov_fec"][..], extra].concat(),
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    o
}

/// (row kind and number, line, input, count), sorted.
fn rows(db: &Database) -> Vec<(String, u32, String, u64)> {
    let mut v: Vec<_> = db
        .items
        .iter()
        .filter_map(|(k, e)| {
            let name = match k.kind {
                Kind::Condition(r) => format!("c{r}"),
                Kind::Expression(r) => format!("e{r}"),
                _ => return None,
            };
            assert_eq!(k.path, ":cov_fec");
            Some((name, e.line, k.sig.clone(), e.count))
        })
        .collect();
    v.sort_by(|a, b| {
        (a.1, &a.0[..1], a.0[1..].parse::<u32>().ok()).cmp(&(
            b.1,
            &b.0[..1],
            b.0[1..].parse::<u32>().ok(),
        ))
    });
    v
}

const EXPECTED: &[(&str, u32, &str, u64)] = &[
    // `r <= true when a and b else false`: evaluated with (a, b) = 00, 01, 10, 11. With a false
    // `b` is not evaluated and masked.
    // Parameters of a function are inputs, not constants: `p and q` in `both`, called four times.
    ("c0", 17, "p", 2),
    ("c1", 17, "p", 1),
    ("c2", 17, "q", 1),
    ("c3", 17, "q", 1),
    ("c0", 20, "a", 2),
    ("c1", 20, "a", 1),
    ("c2", 20, "b", 1),
    ("c3", 20, "b", 1),
    // `q <= x xor y`: every input decides alone: (x, y) = 00, 'H' 'L' is 10, '1' '1' (X and U
    // evaluations count nothing).
    ("e0", 21, "x", 2),
    ("e1", 21, "x", 1),
    ("e2", 21, "y", 2),
    ("e3", 21, "y", 1),
    // `a and (b or c)` over the eight combinations; with `b` true `c` is not evaluated.
    ("c0", 38, "a", 4),
    ("c1", 38, "a", 3),
    ("c2", 38, "b", 1),
    ("c3", 38, "b", 2),
    ("c4", 38, "c", 1),
    ("c5", 38, "c", 1),
    // `not (a xor b) or c`.
    ("e0", 41, "a", 3),
    ("e1", 41, "a", 3),
    ("e2", 41, "b", 3),
    ("e3", 41, "b", 3),
    ("e4", 41, "c", 2),
    ("e5", 41, "c", 2),
    // `tick(a) and tick(b)`: the function-call inputs; `tick(b)` runs only when `tick(a)` is true.
    ("c0", 43, "tick(a)", 4),
    ("c1", 43, "tick(a)", 2),
    ("c2", 43, "tick(b)", 2),
    ("c3", 43, "tick(b)", 2),
    // `x and y` on std_logic: (x, y) = '0' '1', 'H' 'L', 'X' '1', '1' 'U'; metavalues count nothing.
    ("c0", 52, "x", 1),
    ("c1", 52, "x", 0),
    ("c2", 52, "y", 1),
    ("c3", 52, "y", 0),
    // `while k < 4 and not (n > 1000)`, `next when k = 1 or k = 3`, `exit when k = 2 and n > 1000`.
    ("c0", 57, "k < 4", 1),
    ("c1", 57, "k < 4", 4),
    ("c2", 57, "n > 1000", 4),
    ("c3", 57, "n > 1000", 0),
    ("c0", 59, "k = 1", 2),
    ("c1", 59, "k = 1", 1),
    ("c2", 59, "k = 3", 2),
    ("c3", 59, "k = 3", 1),
    ("c0", 60, "k = 2", 1),
    ("c1", 60, "k = 2", 0),
    ("c2", 60, "n > 1000", 1),
    ("c3", 60, "n > 1000", 0),
    // Never evaluated with `b` true and `a` false, and `a` on one input is no condition.
    ("c0", 67, "a", 0),
    ("c1", 67, "a", 1),
    ("c2", 67, "b", 0),
    ("c3", 67, "b", 0),
    ("c4", 67, "c", 0),
    ("c5", 67, "c", 0),
    // Constant inputs (generic `gt`, constant `kf`, `true`) are evaluated but have no rows, and
    // the row numbers keep the input's position: `(a and gt) or (b and not kf)` has rows for
    // `a` (0, 1) and `b` (4, 5) only; `b` is never evaluated (`a and gt` is true).
    ("c0", 72, "a", 0),
    ("c1", 72, "a", 1),
    ("c4", 72, "b", 0),
    ("c5", 72, "b", 0),
    // `a and gt`, `gt or kf` (line 74, 77) have a single or no non-constant input: no rows.
    // `b and true and (a or kf)`: the inputs are `b`, `true`, `a`, `kf`.
    ("c0", 81, "b", 0),
    ("c1", 81, "b", 1),
    ("c4", 81, "a", 0),
    ("c5", 81, "a", 1),
    // A constant first input (`gt`) starts the state: rows for `a` (2, 3) and `b` (4, 5).
    ("c2", 85, "a", 0),
    ("c3", 85, "a", 1),
    ("c4", 85, "b", 0),
    ("c5", 85, "b", 1),
];

fn want() -> Vec<(String, u32, String, u64)> {
    EXPECTED
        .iter()
        .map(|&(r, l, i, c)| (r.to_string(), l, i.to_string(), c))
        .collect()
}

#[test]
fn counts_threads_and_reports() {
    let tmp = setup("fec");
    let plain = run(&tmp, &[]);
    let with = run(&tmp, &["--cov=a.ycov", "--cov-kind=condition,expression"]);
    // The design behaves the same with the counters on.
    assert_eq!(plain.stdout, with.stdout);
    assert_eq!(plain.stderr, with.stderr);
    let a = read(&tmp.join("a.ycov"));
    assert_eq!(rows(&a), want());
    // Only the kinds asked for are in it.
    assert!(a.items.keys().all(|k| k.kind.is_fec()));

    // Byte-identical at four threads.
    run(
        &tmp,
        &[
            "--cov=a4.ycov",
            "--cov-kind=condition,expression",
            "--threads=4",
        ],
    );
    assert_eq!(
        std::fs::read(tmp.join("a.ycov")).expect("a"),
        std::fs::read(tmp.join("a4.ycov")).expect("a4")
    );

    // The other kinds alone, and each kind apart.
    run(&tmp, &["--cov=s.ycov", "--cov-kind=statement,branch"]);
    assert!(
        read(&tmp.join("s.ycov"))
            .items
            .keys()
            .all(|k| !k.kind.is_fec())
    );
    run(&tmp, &["--cov=c.ycov", "--cov-kind=condition"]);
    let c = rows(&read(&tmp.join("c.ycov")));
    assert!(!c.is_empty() && c.iter().all(|r| r.0.starts_with('c')));
    run(&tmp, &["--cov=e.ycov", "--cov-kind=expression"]);
    let e = rows(&read(&tmp.join("e.ycov")));
    assert!(!e.is_empty() && e.iter().all(|r| r.0.starts_with('e')));

    // The text summary and the missed rows.
    let o = yavs(&tmp, &["cov", "report", "a.ycov"]);
    let text = String::from_utf8(o.stdout).expect("utf-8");
    assert!(
        text.starts_with("statements 0/0 -\nbranches   0/0 -\ntoggles    0/0 -\nconditions 35/52 67.3%\nexpressions 10/10 100.0%\n"),
        "{text}"
    );
    assert!(
        text.contains("condition input x is 1\n") || text.contains("condition input x is 1 "),
        "{text}"
    );
    let o = yavs(&tmp, &["cov", "report", "a.ycov", "--uncovered"]);
    let miss = String::from_utf8(o.stdout).expect("utf-8");
    assert_eq!(miss.lines().count(), 17, "{miss}");
    assert!(
        miss.contains("# y is 1 (result 1) in x and y")
            && miss.contains("# n > 1000 is 1 (result 0) in k < 4 and not (n > 1000)")
            && miss.contains("# a is 0 (result 1) in (not a and b) and c"),
        "{miss}"
    );
    assert!(
        miss.lines().all(|l| l.starts_with("condition :cov_fec ")),
        "{miss}"
    );

    // HTML: a table per condition with row, input, result and hit.
    let o = yavs(&tmp, &["cov", "report", "a.ycov", "--html", "html"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let page = std::fs::read_to_string(tmp.join("html/src-0.html")).expect("page");
    assert!(page.contains("<th>row</th><th>input</th><th>result</th><th>hit</th>"));
    assert!(page.contains("<td>3</td><td>b = 1</td><td>1</td><td class=\"n\">2</td>"));
    assert!(
        page.contains(
            "<tr class=\"miss\"><td>1</td><td>x = 1</td><td>1</td><td class=\"n\">0</td>"
        )
    );
    let index = std::fs::read_to_string(tmp.join("html/index.html")).expect("index");
    assert!(index.contains("<th>conditions</th><th>expressions</th>"));

    // Cobertura: the rows of a line are its conditions.
    yavs(&tmp, &["cov", "report", "a.ycov", "--cobertura", "a.xml"]);
    let xml = std::fs::read_to_string(tmp.join("a.xml")).expect("xml");
    assert!(
        xml.contains(
            "<line number=\"38\" hits=\"0\" branch=\"true\" condition-coverage=\"100% (6/6)\"/>"
        ) && xml.contains(
            "<line number=\"67\" hits=\"0\" branch=\"true\" condition-coverage=\"16% (1/6)\"/>"
        ),
        "{xml}"
    );
}

#[test]
fn merge_keeps_tests() {
    let tmp = setup("fec_merge");
    // Two runs of the same design with different generics are not possible here: attribute the
    // same database to two test names and merge them.
    for t in ["one", "two"] {
        run(
            &tmp,
            &[
                &format!("--cov={t}.ycov"),
                "--cov-kind=condition,expression",
                &format!("--cov-test={t}"),
            ],
        );
    }
    let o = yavs(
        &tmp,
        &["cov", "merge", "one.ycov", "two.ycov", "-o", "m.ycov"],
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let m = read(&tmp.join("m.ycov"));
    assert_eq!(m.tests, ["one", "two"]);
    let one = read(&tmp.join("one.ycov"));
    for (k, e) in &m.items {
        assert_eq!(e.count, 2 * one.items[k].count, "{k:?}");
        assert_eq!(e.tests.len(), if e.count > 0 { 2 } else { 0 }, "{k:?}");
    }
}

/// The tiers count the same rows (the LLVM tier steps the counters in the interpreter).
#[cfg(feature = "llvm")]
#[test]
fn tiers_agree() {
    let tmp = setup("fec_tiers");
    for tier in ["interp", "llvm", "auto"] {
        run(
            &tmp,
            &[
                &format!("--cov={tier}.ycov"),
                "--cov-kind=condition,expression",
                &format!("--tier={tier}"),
            ],
        );
        assert_eq!(
            rows(&read(&tmp.join(format!("{tier}.ycov")))),
            want(),
            "{tier}"
        );
    }
}

fn report(tmp: &Path, args: &[&str]) -> (Option<i32>, String, String) {
    let o = yavs(tmp, &[&["cov", "report", "a.ycov"][..], args].concat());
    (
        o.status.code(),
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

/// Analyze `src` (in `d.vhd`), run it and leave the database in `a.ycov`.
fn cov_run(tmp: &Path, src: &str) {
    std::fs::write(tmp.join("d.vhd"), src).expect("write");
    let o = yavs(tmp, &["analyze", "--work=work:lib", "d.vhd"]);
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let o = yavs(
        tmp,
        &[
            "run",
            "--work=work:lib",
            "d",
            "--cov=a.ycov",
            "--cov-kind=condition,expression",
        ],
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}

const EXCL: &str = "entity d is end entity;
architecture sim of d is
  signal a, b, y : boolean := false;
begin
  p : process
  begin
    wait for 1 ns;
    if a and b then
      y <= true;
    end if;
    y <= a or b;
    wait;
  end process;
end architecture;
";

#[test]
fn exclusions_per_row() {
    let tmp = tempdir("fec_excl");
    cov_run(&tmp, EXCL);
    // `a and b` with a = b = false: row a = 0 only; `a or b`: a = 0 and b = 0 decide nothing
    // alone... flipping either makes the result true, so both are covered.
    let (_, rows, _) = report(&tmp, &["--uncovered"]);
    assert_eq!(rows.lines().count(), 5, "{rows}");
    assert!(
        rows.lines()
            .next()
            .is_some_and(|l| l.starts_with("condition :d ") && l.contains("d.vhd:8   # a is 1")),
        "{rows}"
    );
    assert!(rows.contains("expression :d "), "{rows}");

    // The text rows and the YAML entries exclude exactly the misses.
    let (_, yaml, _) = report(&tmp, &["--uncovered=yaml"]);
    assert!(
        yaml.contains("kind: condition") && yaml.contains("row: 1"),
        "{yaml}"
    );
    for (name, body) in [("ex.txt", format!("# why\n{rows}")), ("ex.yaml", yaml)] {
        std::fs::write(tmp.join(name), body).expect("write");
        let (code, out, err) = report(&tmp, &["--exclude", name]);
        assert_eq!((code, err.as_str()), (Some(0), ""), "{name}: {out}");
        assert!(out.contains("excluded (5, not in the totals)"), "{out}");
        assert!(!out.contains("not covered"), "{out}");
    }

    // Coarse forms take rows out too.
    std::fs::write(tmp.join("ex2.txt"), "instance :d\n").expect("write");
    let (_, out, err) = report(&tmp, &["--exclude", "ex2.txt"]);
    assert!(
        err.contains("W0114") || out.contains("excluded (6"),
        "{out}{err}"
    );

    // The body of the `if` changes and the condition moves down: still excluded, a moved line.
    cov_run(
        &tmp,
        &format!("\n\n{}", EXCL.replace("y <= true;", "y <= not y;")),
    );
    let (code, out, err) = report(&tmp, &["--exclude", "ex.txt"]);
    assert_eq!(code, Some(0));
    assert_eq!(err.matches("warning[W0110]").count(), 5, "{err}");
    assert!(out.contains("excluded (5"), "{out}");

    // The condition changes: its rows are stale.
    cov_run(&tmp, &EXCL.replace("a and b", "a and not b"));
    let (code, out, err) = report(&tmp, &["--exclude", "ex.txt"]);
    assert_eq!(code, Some(0));
    assert!(err.contains("warning[W0111]"), "{err}");
    assert!(out.contains("not covered"), "{out}");

    // A row that ran is excluded: W0114. A malformed row names its line.
    cov_run(&tmp, EXCL);
    let (_, rows, _) = report(&tmp, &["--tests"]);
    let hit = rows.lines().next().expect("a hit row");
    std::fs::write(tmp.join("ex3.txt"), format!("{hit}\n")).expect("write");
    let (_, _, err) = report(&tmp, &["--exclude", "ex3.txt"]);
    assert!(err.contains("warning[W0114]"), "{err}");
    std::fs::write(tmp.join("ex4.txt"), "condition :d 0123456789ab.0\n").expect("write");
    let (code, _, err) = report(&tmp, &["--exclude", "ex4.txt"]);
    assert_eq!(code, Some(1));
    assert!(
        err.contains("error[E0112]") && err.contains("line 1"),
        "{err}"
    );
    std::fs::write(
        tmp.join("ex5.yaml"),
        "exclude:\n  - kind: condition\n    instance: x\n    hash: \"0123456789ab.0\"\n",
    )
    .expect("write");
    let (code, _, err) = report(&tmp, &["--exclude", "ex5.yaml"]);
    assert_eq!(code, Some(1));
    assert!(err.contains("needs `row`"), "{err}");
}
