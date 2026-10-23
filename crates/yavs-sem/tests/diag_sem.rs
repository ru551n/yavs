//! Runs the negative suite `tests/diag/sem` (contract in its README.md)
//! against the analyzer: each file must produce exactly its `-- expect:`
//! set of `(code, line, col)` and every `-- help:` fragment.
//!
//! Files listed in `diag_sem_xfail.txt` are expected to fail until the slice
//! that handles them lands. A listed file that passes fails the test, so the
//! list only shrinks: remove the line and commit.
//!
//! A file with a `<name>.stderr` next to it is also a rendered snapshot: the
//! human-readable output must equal it (`YAVS_BLESS=1` rewrites existing ones;
//! create an empty `.stderr` to start a new snapshot).

use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

use yavs_sem::{Db, LangRev, Options, default_stdlib};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn files(dir: &Path, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .expect("suite dir")
        .flatten()
        .map(|e| e.path())
        .collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            files(&p, out);
        } else if p.extension().is_some_and(|e| e == "vhd") {
            out.push(p);
        }
    }
}

fn rev(s: &str) -> LangRev {
    match s {
        "1993" => LangRev::Vhdl1993,
        "2019" => LangRev::Vhdl2019,
        _ => LangRev::Vhdl2008,
    }
}

/// Returns `Err(description)` when the file does not behave as its header says.
fn check(path: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    let mut file_rev = LangRev::Vhdl2008;
    let mut design_rev = None;
    let mut relaxed = false;
    let mut want = BTreeSet::new();
    let mut helps = Vec::new();
    for line in text.lines().take_while(|l| l.starts_with("--")) {
        if let Some(h) = line.strip_prefix("-- yavs-test:") {
            for w in h.split_whitespace() {
                if let Some(s) = w.strip_prefix("std=") {
                    file_rev = rev(s);
                } else if let Some(f) = w.strip_prefix("flags=") {
                    for f in f.split(',') {
                        match f {
                            "--relaxed" => relaxed = true,
                            "--design-std=2019" => design_rev = Some(LangRev::Vhdl2019),
                            other => return Err(format!("unknown flag {other}")),
                        }
                    }
                }
            }
        } else if let Some(e) = line.strip_prefix("-- expect:") {
            let mut it = e.split_whitespace();
            match (it.next(), it.next()) {
                (Some("none"), _) => {}
                (Some(code), Some(pos)) => {
                    let (l, c) = pos.split_once(':').ok_or("bad expect position")?;
                    want.insert((
                        code.to_string(),
                        l.parse::<usize>().map_err(|e| e.to_string())?,
                        c.parse::<usize>().map_err(|e| e.to_string())?,
                    ));
                }
                _ => return Err(format!("bad expect line `{line}`")),
            }
        } else if let Some(h) = line.strip_prefix("-- help:") {
            helps.push(h.trim().to_string());
        }
    }
    let mut opts = Options::new(design_rev.unwrap_or(file_rev), default_stdlib());
    opts.relaxed = relaxed;
    let mut db = Db::new(opts);
    let _ = db.analyze_file("work", path, file_rev);
    let shown = path.display().to_string();
    // An internal error is never an expected failure, not even for an xfail file.
    if let Some(d) = db
        .diagnostics()
        .iter()
        .find(|d| d.notes.iter().any(|n| n.contains("this is a bug in yavs")))
    {
        return Err(format!("ICE: {}", d.message));
    }
    let mut got = BTreeSet::new();
    let mut extra = Vec::new();
    for d in db.diagnostics() {
        match &d.primary {
            Some(p) if db.sources.get(p.span.file).path == shown => {
                let (l, c) = db.sources.get(p.span.file).line_col(p.span.start);
                got.insert((d.code.to_string(), l, c));
            }
            _ => extra.push(format!("{} {} (outside the file)", d.code, d.message)),
        }
    }
    let all_help: Vec<&String> = db.diagnostics().iter().flat_map(|d| &d.help).collect();
    let missing_help: Vec<&String> = helps
        .iter()
        .filter(|h| !all_help.iter().any(|a| a.contains(h.as_str())))
        .collect();
    if got == want && extra.is_empty() && missing_help.is_empty() {
        return snapshot(path, &db);
    }
    let fmt = |s: &BTreeSet<(String, usize, usize)>| {
        s.iter()
            .map(|(c, l, k)| format!("{c} {l}:{k}"))
            .collect::<Vec<_>>()
            .join(", ")
    };
    let mut msg = format!("want [{}], got [{}]", fmt(&want), fmt(&got));
    if !extra.is_empty() {
        msg += &format!("; also {}", extra.join("; "));
    }
    if !missing_help.is_empty() {
        msg += &format!("; missing help {missing_help:?}");
    }
    if let Some(d) = db.diagnostics().first() {
        msg += &format!("; first: {} {}", d.code, d.message);
    }
    Err(msg)
}

fn snapshot(path: &Path, db: &Db) -> Result<(), String> {
    let snap = path.with_extension("stderr");
    if !snap.exists() {
        return Ok(());
    }
    let shown = path.display().to_string();
    let rel = format!(
        "tests/diag/sem/{}",
        path.strip_prefix(repo().join("tests/diag/sem"))
            .expect("under the suite")
            .to_string_lossy()
            .replace('\\', "/")
    );
    let text: String = db
        .diagnostics()
        .iter()
        .map(|d| yavs_diag::render(&db.sources, d, yavs_diag::MessageFormat::Human, false))
        .collect::<String>()
        .replace(&shown, &rel);
    if std::env::var_os("YAVS_BLESS").is_some_and(|v| v == "1") {
        std::fs::write(&snap, &text).map_err(|e| e.to_string())?;
        return Ok(());
    }
    let expected = std::fs::read_to_string(&snap).map_err(|e| e.to_string())?;
    if expected == text {
        Ok(())
    } else {
        Err(format!(
            "rendered output differs from {}:\n{text}",
            snap.display()
        ))
    }
}

#[test]
fn negative_suite() {
    let root = repo().join("tests/diag/sem");
    let mut all = Vec::new();
    files(&root, &mut all);
    assert!(!all.is_empty(), "no files under {}", root.display());
    let xfail_text = include_str!("diag_sem_xfail.txt");
    let xfail: BTreeSet<&str> = xfail_text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .collect();
    let mut unexpected = Vec::new();
    let mut fixed = Vec::new();
    let mut failing = Vec::new();
    for p in &all {
        let rel = p
            .strip_prefix(&root)
            .expect("under root")
            .to_string_lossy()
            .replace('\\', "/");
        let r = check(p);
        let ice = r.as_ref().is_err_and(|e| e.starts_with("ICE:"));
        match (r, xfail.contains(rel.as_str()) && !ice) {
            (Ok(()), true) => fixed.push(rel),
            (Err(e), false) => unexpected.push(format!("{rel}: {e}")),
            (Err(_), true) => failing.push(rel),
            (Ok(()), false) => {}
        }
    }
    if std::env::var_os("YAVS_SEM_XFAIL_WRITE").is_some() {
        let mut list: Vec<String> = failing.clone();
        list.extend(
            unexpected
                .iter()
                .map(|u| u.split(':').next().unwrap_or_default().to_string()),
        );
        list.sort();
        let out = format!(
            "# Files of tests/diag/sem expected to fail until their slice lands (see diag_sem.rs).\n{}\n",
            list.join("\n")
        );
        std::fs::write(
            Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/diag_sem_xfail.txt"),
            out,
        )
        .expect("write xfail");
        return;
    }
    println!(
        "{} files, {} pass, {} expected failures",
        all.len(),
        all.len() - failing.len() - unexpected.len(),
        failing.len()
    );
    assert!(
        unexpected.is_empty() && fixed.is_empty(),
        "unexpected failures:\n  {}\nnow passing, remove from diag_sem_xfail.txt:\n  {}\n(YAVS_SEM_XFAIL_WRITE=1 regenerates the list)",
        unexpected.join("\n  "),
        fixed.join("\n  ")
    );
}
