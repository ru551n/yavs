//! Parse corpus tests.
//!
//! Built only with `--features corpus`: `cargo test -p yavs --features corpus`.
//! It parses the corpora fetched by `tools/fetch-corpus.sh` into the
//! directory named by `YAVS_CORPUS` (and fails if that is unset), and compares
//! every file with the checked-in list `tests/parse-corpus/<corpus>.txt`:
//!
//! ```text
//! PASS 2008 sim1/test/regress/wait1.vhd
//! FAIL 2019 sim1/test/parse/error1.vhd  # E0002 expected `;`, found `end`
//! ```
//!
//! Any difference (a new failure, a new pass, a file added or gone) fails the
//! test. `YAVS_BLESS=1` rewrites the lists from the current results. In the
//! zero-error corpora (VUnit, OSVVM, rust_hdl's libraries) no failure is allowed.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use yavs_diag::Severity;
use yavs_diag::SourceMap;
use yavs_syntax::{Encoding, Options, Std, load_source, parse_file};

/// The lists name the reference corpora by label; the fetched checkouts keep the
/// upstream directory names (`tools/fetch-corpus.sh`).
const LABELS: &[(&str, &str)] = &[("sim1/", "nvc/"), ("sim2/", "ghdl/")];

fn to_listed(rel: &str) -> String {
    for (label, dir) in LABELS {
        if let Some(r) = rel.strip_prefix(dir) {
            return format!("{label}{r}");
        }
    }
    rel.to_string()
}

fn from_listed(rel: &str) -> String {
    for (label, dir) in LABELS {
        if let Some(r) = rel.strip_prefix(label) {
            return format!("{dir}{r}");
        }
    }
    rel.to_string()
}

struct Corpus {
    name: &'static str,
    /// Directories relative to `YAVS_CORPUS`, and whether to recurse.
    dirs: &'static [(&'static str, bool)],
    /// No file may fail.
    zero_errors: bool,
}

const CORPORA: &[Corpus] = &[
    Corpus {
        name: "vunit",
        dirs: &[("vunit/vunit/vhdl", true)],
        zero_errors: true,
    },
    Corpus {
        name: "osvvm",
        dirs: &[("osvvm", true)],
        zero_errors: true,
    },
    Corpus {
        name: "rust_hdl",
        dirs: &[("rust_hdl/vhdl_libraries", true)],
        zero_errors: true,
    },
    Corpus {
        name: "sim1",
        dirs: &[("nvc/test/regress", false), ("nvc/test/parse", false)],
        zero_errors: false,
    },
    Corpus {
        name: "vests",
        // vhdl-ams is VHDL-AMS (IEEE 1076.1), which yavs does not implement.
        dirs: &[("vests/vhdl-93", true)],
        zero_errors: false,
    },
    Corpus {
        name: "sim2",
        dirs: &[("ghdl/testsuite/gna", true)],
        zero_errors: false,
    },
];

fn repo_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn vhdl_files(dir: &Path, recurse: bool, out: &mut Vec<PathBuf>) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
        .map(|e| e.expect("directory entry").path())
        .collect();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            if recurse {
                vhdl_files(&p, true, out);
            }
        } else if p
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("vhd") || e.eq_ignore_ascii_case("vhdl"))
        {
            out.push(p);
        }
    }
}

struct Outcome {
    /// First error, `None` if the file parsed.
    error: Option<String>,
    bytes: usize,
}

fn parse(path: &Path, std: Std) -> Outcome {
    let bytes =
        std::fs::read(path).unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    let len = bytes.len();
    let mut sources = SourceMap::default();
    let (file, bad) = load_source(&mut sources, "", bytes, Encoding::Auto);
    assert!(bad.is_none());
    let opts = Options {
        std,
        defines: Vec::new(),
        psl: false,
    };
    let parsed = parse_file(file, sources.get(file), &opts);
    let error = parsed
        .diags
        .iter()
        .find(|d| d.severity == Severity::Error)
        .map(|d| format!("{} {}", d.code, d.message.replace('\n', " ")));
    assert_eq!(error.is_none(), parsed.design.is_some());
    Outcome { error, bytes: len }
}

/// Run `f` on a thread with the same stack size as the `yavs` main thread.
fn with_stack<T: Send>(f: impl FnOnce() -> T + Send) -> T {
    std::thread::scope(|s| {
        std::thread::Builder::new()
            .stack_size(8 << 20)
            .spawn_scoped(s, f)
            .expect("spawn")
            .join()
            .expect("corpus thread panicked")
    })
}

fn std_name(std: Std) -> &'static str {
    std.as_str()
}

fn std_from(s: &str) -> Option<Std> {
    match s {
        "2008" => Some(Std::Vhdl2008),
        "2019" => Some(Std::Vhdl2019),
        _ => None,
    }
}

/// Guess the standard of a file that is not in the list yet (when blessing).
fn infer_std(root: &Path, rel: &str) -> Std {
    let path = root.join(rel);
    let stem = path
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default()
        .to_string();
    if rel.starts_with("nvc/test/regress/") {
        let list =
            std::fs::read_to_string(root.join("nvc/test/regress/testlist.txt")).unwrap_or_default();
        let is_2019 = list.lines().any(|l| {
            let mut it = l.split_whitespace();
            it.next() == Some(stem.as_str())
                && it.next().is_some_and(|f| f.split(',').any(|x| x == "2019"))
        });
        return if is_2019 {
            Std::Vhdl2019
        } else {
            Std::Vhdl2008
        };
    }
    if rel.starts_with("nvc/test/parse/") {
        let c = std::fs::read_to_string(root.join("nvc/test/test_parse.c")).unwrap_or_default();
        let mut std = Std::Vhdl2008;
        for l in c.lines() {
            if l.starts_with("START_TEST") {
                std = Std::Vhdl2008;
            } else if l.contains("set_standard(STD_19)") {
                std = Std::Vhdl2019;
            } else if l.contains(&format!("/parse/{stem}.vhd\"")) {
                return std;
            }
        }
        return Std::Vhdl2008;
    }
    if rel.starts_with("ghdl/") {
        let script = path.with_file_name("testsuite.sh");
        let s = std::fs::read_to_string(script).unwrap_or_default();
        if s.contains("std=19") {
            return Std::Vhdl2019;
        }
    }
    Std::Vhdl2008
}

struct Entry {
    pass: bool,
    std: Std,
}

fn read_list(path: &Path) -> BTreeMap<String, Entry> {
    let mut map = BTreeMap::new();
    let Ok(text) = std::fs::read_to_string(path) else {
        return map;
    };
    for (n, line) in text.lines().enumerate() {
        // The `# reason` comment is informational.
        let body = line.split_once('#').map_or(line, |(b, _)| b);
        let mut it = body.split_whitespace();
        let (Some(status), Some(std), Some(file)) = (it.next(), it.next(), it.next()) else {
            assert!(
                body.trim().is_empty(),
                "{}:{}: malformed line",
                path.display(),
                n + 1
            );
            continue;
        };
        let pass = match status {
            "PASS" => true,
            "FAIL" => false,
            _ => panic!("{}:{}: status must be PASS or FAIL", path.display(), n + 1),
        };
        let std = std_from(std)
            .unwrap_or_else(|| panic!("{}:{}: bad standard `{std}`", path.display(), n + 1));
        map.insert(from_listed(file), Entry { pass, std });
    }
    map
}

#[test]
fn external() {
    let Some(root) = std::env::var_os("YAVS_CORPUS").map(PathBuf::from) else {
        panic!(
            "YAVS_CORPUS is not set. Fetch the corpora with `tools/fetch-corpus.sh DIR` \
             (default DIR: ~/.cache/rsim-corpus) and run with `YAVS_CORPUS=DIR`."
        );
    };
    let bless = std::env::var_os("YAVS_BLESS").is_some_and(|v| v == "1");
    let lists = repo_root().join("tests/parse-corpus");
    let mut failures = String::new();
    for corpus in CORPORA {
        let list_path = lists.join(format!("{}.txt", corpus.name));
        let expected = read_list(&list_path);
        let mut files = Vec::new();
        for (dir, recurse) in corpus.dirs {
            let d = root.join(dir);
            assert!(
                d.is_dir(),
                "{} is missing; run tools/fetch-corpus.sh {}",
                d.display(),
                root.display()
            );
            vhdl_files(&d, *recurse, &mut files);
        }
        let rels: Vec<String> = files
            .iter()
            .map(|f| {
                f.strip_prefix(&root)
                    .expect("under root")
                    .to_string_lossy()
                    .replace('\\', "/")
            })
            .collect();
        let jobs: Vec<(String, Std)> = rels
            .iter()
            .map(|r| {
                let std = expected
                    .get(r)
                    .map_or_else(|| infer_std(&root, r), |e| e.std);
                (r.clone(), std)
            })
            .collect();
        let root2 = root.clone();
        let (results, elapsed) = with_stack(move || {
            let t = Instant::now();
            let r: Vec<Outcome> = jobs
                .iter()
                .map(|(rel, std)| parse(&root2.join(rel), *std))
                .collect();
            (r.into_iter().zip(jobs).collect::<Vec<_>>(), t.elapsed())
        });
        report_throughput(corpus.name, &results, elapsed);

        let mut out = String::new();
        let mut diffs = Vec::new();
        let mut hard = Vec::new();
        for (outcome, (rel, std)) in &results {
            let pass = outcome.error.is_none();
            match &outcome.error {
                None => writeln!(out, "PASS {} {}", std_name(*std), to_listed(rel)),
                Some(e) => writeln!(out, "FAIL {} {}  # {e}", std_name(*std), to_listed(rel)),
            }
            .expect("write");
            match expected.get(rel) {
                None => diffs.push(format!(
                    "new file {rel}: {}",
                    outcome.error.as_deref().unwrap_or("PASS")
                )),
                Some(e) if e.pass != pass => diffs.push(format!(
                    "{rel} --std {}: expected {}, got {}",
                    std_name(*std),
                    if e.pass { "PASS" } else { "FAIL" },
                    outcome.error.as_deref().unwrap_or("PASS")
                )),
                Some(_) => {}
            }
            if corpus.zero_errors
                && let Some(e) = &outcome.error
            {
                hard.push(format!("{rel}: must parse without errors: {e}"));
            }
        }
        for rel in expected.keys() {
            if !rels.contains(rel) {
                diffs.push(format!("{rel}: listed but not in the corpus"));
            }
        }
        if bless {
            std::fs::create_dir_all(&lists).expect("mkdir");
            std::fs::write(&list_path, out).expect("write list");
        }
        let fails = results.iter().filter(|(o, _)| o.error.is_some()).count();
        println!(
            "{}: {} files, {} pass, {} fail",
            corpus.name,
            results.len(),
            results.len() - fails,
            fails
        );
        if bless {
            diffs.clear();
        }
        diffs.extend(hard);
        if !diffs.is_empty() {
            writeln!(
                failures,
                "{}: {} differences from {}",
                corpus.name,
                diffs.len(),
                list_path.display()
            )
            .expect("write");
            for d in diffs.iter().take(40) {
                writeln!(failures, "  {d}").expect("write");
            }
        }
    }
    assert!(
        failures.is_empty(),
        "{failures}\nIf the changes are intended, rerun with YAVS_BLESS=1 and review the list diff."
    );
}

fn report_throughput(name: &str, results: &[(Outcome, (String, Std))], elapsed: Duration) {
    let bytes: usize = results.iter().map(|(o, _)| o.bytes).sum();
    let secs = elapsed.as_secs_f64().max(1e-9);
    println!(
        "{name}: parsed {} files ({:.1} MB) in {:.3} s: {:.0} files/s, {:.1} MB/s",
        results.len(),
        bytes as f64 / 1e6,
        secs,
        results.len() as f64 / secs,
        bytes as f64 / 1e6 / secs
    );
}
