//! The per-clause LRM checks of IEEE 1364-2005: `tests/verilog/lrm/<clause>_<topic>.v`, one rule per file.
//!
//! A file starts with `//` header lines, then the Verilog:
//!
//! ```text
//! // LRM <clause>: <the rule>                      (required)
//! // yavs-test: args=-I inc -D NAME[=VALUE]         (optional: the options of the call)
//! // units: a,b                                     (optional: the exact units recorded, in order)
//! // gap: <id> <text>                               (optional: the LRM disagrees with the front end; the current behaviour is pinned)
//! // expect: <CODE> [<FILE>:]<LINE>:<COL> <why>     (one per error or warning, or `// expect: none`)
//! ```
//!
//! The diagnostics must be exactly the `expect` lines (errors and warnings, order free, notes not compared). Every file is
//! recorded in `docs/design/lrm-checks.md` and every row there names a file.

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use yavs_verilog::{CODES, Options, Severity, analyze};

const DOC: &str = include_str!("../../../docs/design/lrm-checks.md");

fn dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/verilog/lrm")
}

fn files() -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir())
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| matches!(p.extension().and_then(|e| e.to_str()), Some("v" | "sv")))
        .collect();
    v.sort();
    v
}

struct Header {
    args: Vec<String>,
    units: Option<Vec<String>>,
    gap: bool,
    expect: Vec<(String, String)>,
    rule: bool,
}

fn header(text: &str) -> Header {
    let mut h = Header {
        args: Vec::new(),
        units: None,
        gap: false,
        expect: Vec::new(),
        rule: false,
    };
    for l in text.lines().take_while(|l| l.starts_with("//")) {
        let l = l[2..].trim();
        if l.starts_with("LRM ") {
            h.rule = true;
        } else if let Some(a) = l.strip_prefix("yavs-test: args=") {
            h.args = a.split_whitespace().map(String::from).collect();
        } else if let Some(u) = l.strip_prefix("units: ") {
            h.units = Some(u.split(',').map(|s| s.trim().to_string()).collect());
        } else if l.starts_with("gap: ") {
            h.gap = true;
        } else if let Some(e) = l.strip_prefix("expect: ")
            && e != "none"
        {
            let mut w = e.split_whitespace();
            let code = w.next().unwrap().to_string();
            h.expect
                .push((code, w.next().expect("a position").to_string()));
        }
    }
    h
}

fn options(args: &[String], base: &Path) -> Options {
    let mut o = Options::default();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        match a.as_str() {
            "-I" => o.include_dirs.push(base.join(it.next().unwrap())),
            "-D" => {
                let d = it.next().unwrap();
                match d.split_once('=') {
                    Some((n, v)) => o.defines.push((n.into(), Some(v.into()))),
                    None => o.defines.push((d.clone(), None)),
                }
            }
            other => panic!("unknown argument {other}"),
        }
    }
    o
}

/// `[FILE:]LINE:COL` of a byte offset in a file, columns in characters.
fn position(file: &Path, own: &Path, at: usize) -> String {
    let t = std::fs::read(file).unwrap();
    let at = at.min(t.len());
    let line = t[..at].iter().filter(|&&c| c == b'\n').count() + 1;
    let start = t[..at]
        .iter()
        .rposition(|&c| c == b'\n')
        .map_or(0, |i| i + 1);
    let col = String::from_utf8_lossy(&t[start..at]).chars().count() + 1;
    if file == own {
        format!("{line}:{col}")
    } else {
        let rel = file.strip_prefix(dir()).unwrap_or(file);
        format!("{}:{line}:{col}", rel.display())
    }
}

#[test]
fn every_check_gives_exactly_its_expected_diagnostics() {
    let base = dir();
    let all = files();
    assert!(all.len() > 100, "only {} checks found", all.len());
    let mut bad = Vec::new();
    for f in &all {
        let name = f.file_name().unwrap().to_string_lossy().into_owned();
        let text = std::fs::read_to_string(f).unwrap();
        let h = header(&text);
        if !h.rule {
            bad.push(format!("{name}: no `// LRM <clause>: <rule>` line"));
        }
        if !name.starts_with(char::is_numeric) {
            bad.push(format!("{name}: the name starts with the clause number"));
        }
        let r = analyze(std::slice::from_ref(f), &options(&h.args, &base));
        let mut got: Vec<(String, String)> = r
            .diagnostics
            .iter()
            .filter(|d| d.severity != Severity::Note)
            .map(|d| {
                assert!(
                    CODES.iter().any(|c| c.code == d.code),
                    "{name}: unregistered {}",
                    d.code
                );
                let pos = d.span.map_or("-".into(), |s| {
                    position(&r.files[s.file], f, s.start as usize)
                });
                (d.code.to_string(), pos)
            })
            .collect();
        let mut want = h.expect.clone();
        got.sort();
        want.sort();
        if got != want {
            bad.push(format!("{name}: expected {want:?}, got {got:?}"));
        }
        if let Some(u) = &h.units {
            let units: Vec<&str> = r.units.iter().map(|u| u.name.as_str()).collect();
            if units != u.iter().map(String::as_str).collect::<Vec<_>>() {
                bad.push(format!("{name}: expected units {u:?}, got {units:?}"));
            }
        }
        if h.gap && h.expect.iter().any(|(c, _)| c.starts_with('E')) {
            bad.push(format!(
                "{name}: a gap file pins an accepted construct, it has no error"
            ));
        }
    }
    assert!(
        bad.is_empty(),
        "{} failures:\n{}",
        bad.len(),
        bad.join("\n")
    );
}

/// Every check file is a row of the Verilog section of lrm-checks.md, and every row names a file.
#[test]
fn every_check_is_recorded_in_the_lrm_document() {
    let section = DOC
        .split("\n## Verilog (")
        .nth(1)
        .expect("the Verilog section");
    let listed: BTreeSet<String> = section
        .split('`')
        .filter(|s| s.starts_with(char::is_numeric) && (s.ends_with(".v") || s.ends_with(".sv")))
        .map(String::from)
        .collect();
    let have: BTreeSet<String> = files()
        .iter()
        .map(|f| f.file_name().unwrap().to_string_lossy().into_owned())
        .collect();
    let missing: Vec<_> = have.difference(&listed).collect();
    let stale: Vec<_> = listed.difference(&have).collect();
    assert!(
        missing.is_empty(),
        "not in docs/design/lrm-checks.md: {missing:?}"
    );
    assert!(
        stale.is_empty(),
        "in docs/design/lrm-checks.md but no file: {stale:?}"
    );
}
