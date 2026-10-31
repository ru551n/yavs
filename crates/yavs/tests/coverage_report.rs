//! The text output of `yavs cov report` (#877) on `tests/cli/cov_report.vhd`: relative paths,
//! named branch arms, aligned columns, `--kind`, `--total` and `--fail-under`.

mod common;
use common::{Tmp, tempdir};

use std::path::Path;
use std::process::{Command, Output};

fn yavs(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(["--color", "never"])
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run")
}

fn text(o: &[u8]) -> String {
    String::from_utf8(o.to_vec()).expect("utf-8")
}

/// A directory holding the design and its database (statements and branches, 19/23 and 10/16).
fn database(name: &str) -> Tmp {
    let tmp = tempdir(name);
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/cli/cov_report.vhd");
    std::fs::copy(&src, tmp.join("cov_report.vhd")).expect("copy");
    let lib = format!("--work=work:{}/lib", tmp.display());
    let o = yavs(&tmp, &["analyze", &lib, "cov_report.vhd"]);
    assert!(o.status.success(), "{}", text(&o.stderr));
    let o = yavs(
        &tmp,
        &[
            "run",
            &lib,
            "--cov=a.ycov",
            "--cov-kind=statement,branch",
            "cov_report",
        ],
    );
    assert!(o.status.success(), "{}", text(&o.stderr));
    tmp
}

#[test]
fn text_report_names_arms_and_aligns_columns() {
    let tmp = database("text");
    let o = yavs(&tmp, &["cov", "report", "a.ycov"]);
    assert!(o.status.success());
    let out = text(&o.stdout);
    let want = "\
not covered:
  cov_report.vhd:25  branch     if rst = '1'    :cov_report
  cov_report.vhd:25  branch     elsif n > 100   :cov_report
  cov_report.vhd:26  statement  n <= 0;         :cov_report
  cov_report.vhd:30  statement  n <= 0;         :cov_report
  cov_report.vhd:37  branch     when others     :cov_report
  cov_report.vhd:40  statement  v := 3;         :cov_report
  cov_report.vhd:42  branch     else            :cov_report
  cov_report.vhd:45  statement  v := 0;         :cov_report
  cov_report.vhd:53  branch     (no arm taken)  :cov_report
  cov_report.vhd:54  branch     when others     :cov_report
";
    assert!(out.ends_with(want), "{out}");
    assert!(
        out.starts_with("statements 19/23 82.6%\nbranches   10/16 62.5%\n"),
        "{out}"
    );
}

#[test]
fn arm_names_are_the_comments_of_the_uncovered_rows() {
    let tmp = database("rows");
    let o = yavs(&tmp, &["cov", "report", "a.ycov", "--uncovered"]);
    let out = text(&o.stdout);
    for arm in [
        "arm 0: if rst = '1'",
        "arm 2: elsif n > 100",
        "arm 1: else",
        "arm 1: (no arm taken)",
        "arm 2: when others",
    ] {
        assert!(out.contains(&format!("# {arm}\n")), "{arm}\n{out}");
    }
    // Rows carry the same relative path as the report.
    assert!(out.contains(" cov_report.vhd:25   # "), "{out}");
}

#[test]
fn root_changes_the_prefix_stripped() {
    let tmp = database("root");
    let o = yavs(&tmp, &["cov", "report", "a.ycov", "--root", "elsewhere"]);
    let out = text(&o.stdout);
    // The source is outside the root: its full path stays.
    assert!(out.contains("/cov_report.vhd:25"), "{out}");
    assert!(!out.contains("\n  cov_report.vhd:25"), "{out}");
    let o = yavs(&tmp, &["cov", "report", "a.ycov", "--root", "."]);
    assert!(
        text(&o.stdout).contains("\n  cov_report.vhd:25  "),
        "relative"
    );
}

#[test]
fn total_and_kind() {
    let tmp = database("total");
    let total = |args: &[&str]| {
        let o = yavs(&tmp, &[&["cov", "report", "a.ycov"][..], args].concat());
        (o.status.code(), text(&o.stdout), text(&o.stderr))
    };
    // 29 of 39 items, rounded down: a gate never rounds a missing item up.
    assert_eq!(total(&["--total"]), (Some(0), "74.3%\n".into(), "".into()));
    assert_eq!(
        total(&["--total", "--kind=statement"]),
        (Some(0), "82.6%\n".into(), "".into())
    );
    assert_eq!(total(&["--total", "--kind=branch,statement"]).1, "74.3%\n");
    // The normal report with a kind filter has only that kind.
    let o = total(&["--kind=branch"]);
    assert!(
        o.1.starts_with("statements 0/0 -\nbranches   10/16 62.5%\n"),
        "{}",
        o.1
    );
    assert!(!o.1.contains("statement  "), "{}", o.1);
    // A kind the database lacks has nothing to total: E0115, nothing on stdout.
    let (code, out, err) = total(&["--total", "--kind=toggle"]);
    assert_eq!((code, out.as_str()), (Some(1), ""));
    assert!(err.contains("error[E0115]"), "{err}");
    // A total is not a listing.
    assert_eq!(total(&["--total", "--uncovered"]).0, Some(2));
}

#[test]
fn total_counts_after_exclusions() {
    let tmp = database("excl");
    // Excluding a statement that never ran: 19 of 22.
    let rows = text(&yavs(&tmp, &["cov", "report", "a.ycov", "--uncovered"]).stdout);
    let row = rows
        .lines()
        .find(|l| l.starts_with("statement ") && l.contains(":26"))
        .expect("row");
    std::fs::write(tmp.join("ex.txt"), format!("{row}\n")).expect("write");
    let o = yavs(
        &tmp,
        &[
            "cov",
            "report",
            "a.ycov",
            "--exclude",
            "ex.txt",
            "--total",
            "--kind=statement",
        ],
    );
    assert_eq!(text(&o.stdout), "86.3%\n", "{}", text(&o.stderr));
}

#[test]
fn fail_under() {
    let tmp = database("gate");
    let gate = |pct: &str, more: &[&str]| {
        let o = yavs(
            &tmp,
            &[
                &["cov", "report", "a.ycov", "--kind=statement"][..],
                more,
                &[&format!("--fail-under={pct}")],
            ]
            .concat(),
        );
        (o.status.code(), text(&o.stdout), text(&o.stderr))
    };
    // Statements are at 82.6%: at the threshold passes, just above fails.
    let (code, out, err) = gate("82.6", &[]);
    assert_eq!(code, Some(0), "{err}");
    assert!(out.starts_with("statements 19/23 82.6%\n"), "{out}");
    assert!(err.is_empty(), "{err}");
    assert_eq!(gate("82.5", &[]).0, Some(0));
    assert_eq!(gate("0", &[]).0, Some(0));
    // Below: the report is printed as usual and the message follows on stderr.
    let (code, out, err) = gate("82.7", &[]);
    assert_eq!(code, Some(1));
    assert!(out.starts_with("statements 19/23 82.6%\n"), "{out}");
    assert!(
        err.contains("error[E0114]: coverage 82.6% is below the required 82.7%"),
        "{err}"
    );
    // 99.95 would round to 100.0: a gate at 100 still fails with an item missing.
    assert_eq!(gate("100", &[]).0, Some(1));
    // With --total the percentage is printed too.
    let (code, out, err) = gate("90", &["--total"]);
    assert_eq!((code, out.as_str()), (Some(1), "82.6%\n"));
    assert!(err.contains("below the required 90.0%"), "{err}");
    // Without a kind filter it checks everything: 74.3%.
    let o = yavs(
        &tmp,
        &["cov", "report", "a.ycov", "--total", "--fail-under=74.3"],
    );
    assert_eq!(o.status.code(), Some(0));
    let o = yavs(
        &tmp,
        &["cov", "report", "a.ycov", "--total", "--fail-under=74.4"],
    );
    assert_eq!(o.status.code(), Some(1));
    // Not a percentage: a usage error, exit 2, nothing printed.
    for bad in ["abc", "-1", "100.5", "nan", ""] {
        let (code, out, err) = gate(bad, &[]);
        assert_eq!((code, out.as_str()), (Some(2), ""), "{bad}");
        assert!(err.contains("--fail-under"), "{bad}: {err}");
    }
    // An empty selection cannot be checked.
    let o = yavs(
        &tmp,
        &[
            "cov",
            "report",
            "a.ycov",
            "--kind=toggle",
            "--fail-under=50",
        ],
    );
    assert_eq!(o.status.code(), Some(1));
    assert!(text(&o.stderr).contains("E0115"), "{}", text(&o.stderr));
}

#[test]
fn fail_under_in_json_messages() {
    let tmp = database("json");
    let o = yavs(
        &tmp,
        &[
            "--message-format=json",
            "cov",
            "report",
            "a.ycov",
            "--total",
            "--fail-under=99",
        ],
    );
    assert_eq!(o.status.code(), Some(1));
    let err = text(&o.stderr);
    assert!(
        err.contains("\"E0114\"") && err.contains("below the required 99.0%"),
        "{err}"
    );
}
