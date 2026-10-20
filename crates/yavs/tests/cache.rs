//! The compilation cache (`docs/decisions.md` "Compilation cache"): a warm run is
//! byte-identical to a cold one, editing a dependency invalidates, read-only libraries and
//! damaged entries still work, `--no-cache` and `yavs cache clean`.

use std::path::{Path, PathBuf};
use std::process::Command;

fn tempdir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("yavs-cache-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).expect("tmp");
    d
}

const PKG: &str = "package p is\n  constant K : integer := 7;\n  function f (x : integer) return integer;\nend;\n\
package body p is\n  function f (x : integer) return integer is begin return x * K; end;\nend;\n";

const OTHER: &str = "package q is\n  constant Z : integer := 1;\nend;\n";

/// Uses `p`, and has an analysis warning (a static index outside the bounds, in dead code).
const TB: &str = "library ieee; use ieee.std_logic_1164.all;\nuse work.p.all;\n\
entity tb is end;\n\
architecture a of tb is\n  signal s : std_logic := '0';\nbegin\n  process\n    variable v : bit_vector(0 to 3);\n  begin\n\
    if false then v(5) := '1'; end if;\n    s <= '1';\n    wait for 1 ns;\n\
    report \"K=\" & integer'image(K) & \" f=\" & integer'image(f(3)) & \" s=\" & std_logic'image(s);\n    wait;\n  end process;\nend;\n";

/// A package with run-time state (a signal, a shared protected variable), and a testbench that
/// calls an impure function of it during elaboration: the lowered package is cached (`low`).
const RT: &str = "package reg is\n  impure function get_id return integer;\n  signal sg : integer := 3;\nend;\n\
package body reg is\n  type counter_t is protected\n    impure function next_id return integer;\n  end protected;\n\
  type counter_t is protected body\n    variable n : integer := 10;\n\
    impure function next_id return integer is begin n := n + 1; return n; end;\n  end protected body;\n\
  shared variable ctr : counter_t;\n  impure function get_id return integer is begin return ctr.next_id; end;\nend;\n";

const TB2: &str = "use work.reg.all;\nentity tb2 is end;\narchitecture a of tb2 is\n\
  constant id1 : integer := get_id;\n  constant id2 : integer := get_id;\nbegin\n  process\n  begin\n\
    report \"ids \" & integer'image(id1) & \" \" & integer'image(id2) & \" next \" & integer'image(get_id) & \" sg \" & integer'image(sg);\n\
    sg <= 4;\n    wait for 1 ns;\n    report \"sg \" & integer'image(sg);\n    wait;\n  end process;\nend;\n";

struct Env {
    tmp: PathBuf,
}

#[derive(Debug, PartialEq, Eq)]
struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Env {
    fn new(name: &str) -> Env {
        let tmp = tempdir(name);
        let files = [
            ("pkg.vhd", PKG),
            ("other.vhd", OTHER),
            ("tb.vhd", TB),
            ("rt.vhd", RT),
            ("tb2.vhd", TB2),
        ];
        for (f, t) in files {
            std::fs::write(tmp.join(f), t).expect("write");
        }
        let e = Env { tmp };
        for (f, _) in files {
            assert_eq!(e.yavs(&["analyze", "--work", &e.work(), f]).code, 0, "{f}");
        }
        e
    }

    fn work(&self) -> String {
        format!("work:{}", self.tmp.join("work").display())
    }

    fn cache_dir(&self) -> PathBuf {
        self.tmp.join("work").join("yavs-cache")
    }

    fn entries(&self) -> usize {
        std::fs::read_dir(self.cache_dir())
            .map(|d| {
                d.flatten()
                    .filter(|e| e.file_name().to_string_lossy().ends_with(".img"))
                    .count()
            })
            .unwrap_or(0)
    }

    fn yavs(&self, args: &[&str]) -> Out {
        let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
            .current_dir(&self.tmp)
            .env_remove("YAVS_STDLIB")
            .env("YAVS_LOG", "debug")
            .args(["--color", "never"])
            .args(args)
            .output()
            .expect("run");
        Out {
            code: o.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        }
    }

    /// `yavs run` of the testbench, with more arguments.
    fn run(&self, more: &[&str]) -> Out {
        self.run_top("tb", more)
    }

    fn run_top(&self, top: &str, more: &[&str]) -> Out {
        let work = self.work();
        let mut a = vec!["run", "--work", &work];
        a.extend_from_slice(more);
        a.push(top);
        self.yavs(&a)
    }

    /// The cache entries of `kind` (`sem`, `low`).
    fn entry_files(&self, kind: &str) -> Vec<PathBuf> {
        std::fs::read_dir(self.cache_dir())
            .map(|d| {
                d.flatten()
                    .map(|e| e.path())
                    .filter(|p| {
                        let n = p.file_name().unwrap_or_default().to_string_lossy();
                        n.starts_with(&format!("{kind}-")) && n.ends_with(".img")
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// The result with the cache's own debug notes removed (they say hit or store).
fn visible(o: &Out) -> Out {
    Out {
        code: o.code,
        stdout: o.stdout.clone(),
        stderr: o
            .stderr
            .lines()
            .filter(|l| !l.starts_with("debug: cache:"))
            .map(|l| format!("{l}\n"))
            .collect(),
    }
}

fn note(o: &Out, what: &str) -> bool {
    o.stderr
        .lines()
        .any(|l| l.starts_with("debug: cache: ") && l.contains(what))
}

#[test]
fn warm_run_is_identical_to_cold() {
    let e = Env::new("warm");
    let plain = e.run(&["--no-cache"]);
    assert_eq!(plain.code, 0, "{plain:?}");
    assert!(plain.stdout.contains("K=7 f=21"), "{plain:?}");
    assert!(
        plain.stderr.contains("W1404"),
        "the design should warn: {plain:?}"
    );
    assert_eq!(e.entries(), 0, "--no-cache must not write");
    let cold = e.run(&[]);
    assert!(note(&cold, "store"), "{cold:?}");
    assert_eq!(e.entries(), 1);
    let warm = e.run(&[]);
    assert!(note(&warm, "hit"), "{warm:?}");
    assert_eq!(visible(&plain), visible(&cold));
    assert_eq!(visible(&plain), visible(&warm));
    // --threads 4 and -Werror also come out identically, and read the same entry
    for args in [&["--threads", "4"][..], &["-Werror"][..]] {
        let nc = visible(&e.run(&[args, &["--no-cache"]].concat()));
        let w = e.run(args);
        assert!(note(&w, "hit"), "{w:?}");
        assert_eq!(nc, visible(&w), "{args:?}");
    }
    assert_eq!(e.entries(), 1);
}

#[test]
fn editing_a_dependency_invalidates() {
    let e = Env::new("edit");
    assert!(e.run(&[]).stdout.contains("K=7 f=21"));
    assert!(note(&e.run(&[]), "hit"));
    // a unit the design does not use changes: still a hit (its key is not in the closure)
    std::fs::write(e.tmp.join("other.vhd"), OTHER.replace("1", "2")).expect("write");
    assert_eq!(
        e.yavs(&["analyze", "--work", &e.work(), "other.vhd"]).code,
        0
    );
    assert!(note(&e.run(&[]), "hit"));
    // a unit the design uses changes: a miss, and the new value is what runs
    std::fs::write(e.tmp.join("pkg.vhd"), PKG.replace("7", "8")).expect("write");
    assert_eq!(e.yavs(&["analyze", "--work", &e.work(), "pkg.vhd"]).code, 0);
    let after = e.run(&[]);
    assert!(note(&after, "store"), "{after:?}");
    assert!(after.stdout.contains("K=8 f=24"), "{after:?}");
    assert!(note(&e.run(&[]), "hit"));
}

#[test]
fn damaged_entries_are_ignored() {
    let e = Env::new("damaged");
    let cold = e.run(&[]);
    let img = std::fs::read_dir(e.cache_dir())
        .expect("dir")
        .flatten()
        .find(|f| f.file_name().to_string_lossy().ends_with(".img"))
        .expect("entry")
        .path();
    let good = std::fs::read(&img).expect("read");
    for bad in [good[..good.len() / 2].to_vec(), b"not an entry".to_vec(), {
        let mut g = good.clone();
        let n = g.len();
        g[n / 2] ^= 0x55;
        g
    }] {
        std::fs::write(&img, bad).expect("write");
        let r = e.run(&[]);
        assert!(!note(&r, "hit"), "{r:?}");
        assert_eq!(visible(&cold), visible(&r));
        assert!(note(&r, "store"), "the entry is rewritten");
        assert!(note(&e.run(&[]), "hit"));
    }
}

#[cfg(unix)]
#[test]
fn read_only_library_is_not_cached_into() {
    use std::os::unix::fs::PermissionsExt;
    let e = Env::new("readonly");
    let work = e.tmp.join("work");
    let plain = e.run(&["--no-cache"]);
    std::fs::set_permissions(&work, std::fs::Permissions::from_mode(0o555)).expect("chmod");
    let writable = std::fs::File::create(work.join("probe")).is_ok();
    let r = e.run(&[]);
    std::fs::set_permissions(&work, std::fs::Permissions::from_mode(0o755)).expect("chmod");
    if writable {
        return; // running as a user the mode does not bind
    }
    assert_eq!(visible(&plain), visible(&r));
    assert!(!e.cache_dir().exists());
}

#[test]
fn clean_removes_entries() {
    let e = Env::new("clean");
    e.run(&[]);
    assert_eq!(e.entries(), 1);
    let work = e.work();
    let o = e.yavs(&["cache", "clean", "--work", &work]);
    assert_eq!(o.code, 0, "{o:?}");
    assert!(o.stdout.contains("1 entry"), "{o:?}");
    assert!(!e.cache_dir().exists());
    let again = e.yavs(&["cache", "clean", "--work", &work]);
    assert_eq!(again.code, 0);
    assert!(again.stdout.contains("0 entries"), "{again:?}");
    assert!(e.run(&[]).stdout.contains("K=7"));
}

#[test]
fn parallel_cold_runs_agree() {
    let e = Env::new("parallel");
    let plain = visible(&e.run(&["--no-cache"]));
    std::thread::scope(|s| {
        let hs: Vec<_> = (0..4).map(|_| s.spawn(|| visible(&e.run(&[])))).collect();
        for h in hs {
            assert_eq!(plain, h.join().expect("join"));
        }
    });
    assert_eq!(e.entries(), 1);
    assert!(Path::new(&e.cache_dir()).is_dir());
    let leftovers = std::fs::read_dir(e.cache_dir())
        .expect("dir")
        .flatten()
        .filter(|f| f.file_name().to_string_lossy().starts_with(".tmp-"))
        .count();
    assert_eq!(leftovers, 0);
}

/// Sets the modification time of `p` back by an hour (a file that was edited long ago).
fn age(p: &Path) {
    let f = std::fs::File::options().write(true).open(p).expect("open");
    f.set_modified(std::time::SystemTime::now() - std::time::Duration::from_secs(3600))
        .expect("age");
}

/// Sources old enough to be trusted by size and modification time (`yavs_lib::Stamp`) are not
/// read again on a hit; an edit is still seen, because it moves the modification time.
#[test]
fn old_sources_are_trusted_until_they_change() {
    let e = Env::new("aged");
    for f in ["pkg.vhd", "other.vhd", "tb.vhd"] {
        age(&e.tmp.join(f));
        assert_eq!(e.yavs(&["analyze", "--work", &e.work(), f]).code, 0, "{f}");
    }
    let cold = e.run(&[]);
    assert!(note(&cold, "store"), "{cold:?}");
    let warm = e.run(&[]);
    assert!(note(&warm, "hit"), "{warm:?}");
    assert_eq!(visible(&cold), visible(&warm));
    // The same length, so only the modification time tells: the edit is found (L0034).
    std::fs::write(e.tmp.join("pkg.vhd"), PKG.replace('7', "8")).expect("write");
    let edited = e.run(&[]);
    assert_ne!(edited.code, 0, "{edited:?}");
    assert!(edited.stderr.contains("L0034"), "{edited:?}");
    // Analyzing it again records the new content and the run follows.
    assert_eq!(e.yavs(&["analyze", "--work", &e.work(), "pkg.vhd"]).code, 0);
    assert!(e.run(&[]).stdout.contains("K=8 f=24"));
}

/// The lowered run-time packages are cached next to the analysis: a warm run is identical to a
/// cold one and to one without the cache, also with `--threads 4` and with a waveform.
#[test]
fn lowered_packages_are_cached() {
    let e = Env::new("low");
    let wave = |n: &str| e.tmp.join(n).display().to_string();
    let plain = e.run_top(
        "tb2",
        &["--no-cache", &format!("--wave={}", wave("plain.fst"))],
    );
    assert_eq!(plain.code, 0, "{plain:?}");
    assert!(plain.stdout.contains("ids 11 12 next 13 sg 3"), "{plain:?}");
    assert!(e.entry_files("low").is_empty(), "--no-cache must not write");
    let cold = e.run_top("tb2", &[&format!("--wave={}", wave("cold.fst"))]);
    assert!(note(&cold, "store low"), "{cold:?}");
    assert_eq!(e.entry_files("low").len(), 1);
    let warm = e.run_top("tb2", &[&format!("--wave={}", wave("warm.fst"))]);
    assert!(note(&warm, "hit low"), "{warm:?}");
    assert_eq!(visible(&plain), visible(&cold));
    assert_eq!(visible(&plain), visible(&warm));
    let read = |n: &str| std::fs::read(e.tmp.join(n)).expect("wave");
    assert_eq!(read("plain.fst"), read("cold.fst"));
    assert_eq!(read("plain.fst"), read("warm.fst"));
    let nc = visible(&e.run_top("tb2", &["--threads", "4", "--no-cache"]));
    let w = e.run_top("tb2", &["--threads", "4"]);
    assert!(note(&w, "hit low"), "{w:?}");
    assert_eq!(nc, visible(&w));
    assert_eq!(e.entry_files("low").len(), 1);
    // A design without run-time packages has no such entry.
    assert_eq!(e.run(&[]).code, 0);
    assert_eq!(e.entry_files("low").len(), 1);
}

#[test]
fn editing_a_package_replaces_its_lowering() {
    let e = Env::new("lowedit");
    assert!(e.run_top("tb2", &[]).stdout.contains("ids 11 12"));
    assert!(note(&e.run_top("tb2", &[]), "hit low"));
    std::fs::write(
        e.tmp.join("rt.vhd"),
        RT.replace("n : integer := 10", "n : integer := 20"),
    )
    .expect("write");
    assert_eq!(e.yavs(&["analyze", "--work", &e.work(), "rt.vhd"]).code, 0);
    let after = e.run_top("tb2", &[]);
    assert!(note(&after, "store low"), "{after:?}");
    assert!(after.stdout.contains("ids 21 22 next 23"), "{after:?}");
    assert!(note(&e.run_top("tb2", &[]), "hit low"));
}

#[test]
fn damaged_lowering_is_ignored() {
    let e = Env::new("lowdamaged");
    let cold = e.run_top("tb2", &[]);
    let img = e.entry_files("low").pop().expect("low entry");
    let good = std::fs::read(&img).expect("read");
    for bad in [good[..good.len() / 2].to_vec(), b"not an entry".to_vec()] {
        std::fs::write(&img, bad).expect("write");
        let r = e.run_top("tb2", &[]);
        assert!(!note(&r, "hit low"), "{r:?}");
        assert_eq!(visible(&cold), visible(&r));
        assert!(note(&r, "store low"), "the entry is rewritten");
        assert!(note(&e.run_top("tb2", &[]), "hit low"));
    }
}

#[test]
fn parallel_cold_runs_agree_on_lowering() {
    let e = Env::new("lowparallel");
    let plain = visible(&e.run_top("tb2", &["--no-cache"]));
    std::thread::scope(|s| {
        let hs: Vec<_> = (0..4)
            .map(|_| s.spawn(|| visible(&e.run_top("tb2", &[]))))
            .collect();
        for h in hs {
            assert_eq!(plain, h.join().expect("join"));
        }
    });
    assert_eq!(e.entry_files("low").len(), 1);
    let leftovers = std::fs::read_dir(e.cache_dir())
        .expect("dir")
        .flatten()
        .filter(|f| f.file_name().to_string_lossy().starts_with(".tmp-"))
        .count();
    assert_eq!(leftovers, 0);
}

/// A package with a warning of its own (a static index outside the bounds, in dead code) after a
/// line with a non-ASCII character, so that the snippet and column come from the source text.
const NOISY: &str = "-- caf\u{e9} \u{e9}\u{e9}\npackage n is\n  function g (x : integer) return integer;\nend;\n\
package body n is\n  function g (x : integer) return integer is\n    variable v : bit_vector(0 to 3);\n  begin\n\
    if false then v(5) := '1'; end if;\n    return x;\n  end;\nend;\n";

/// A diagnostic in a dependency (not the top-level file) is rendered from source text that a hit
/// reads again on demand, identically to a cold run.
#[test]
fn warm_diagnostics_in_dependencies_render_identically() {
    let e = Env::new("deps");
    std::fs::write(e.tmp.join("noisy.vhd"), NOISY).expect("write");
    let tb = TB.replace("use work.p.all;", "use work.p.all; use work.n.all;");
    std::fs::write(e.tmp.join("tb.vhd"), tb).expect("write");
    for f in ["noisy.vhd", "tb.vhd"] {
        assert_eq!(e.yavs(&["analyze", "--work", &e.work(), f]).code, 0, "{f}");
    }
    let plain = e.run(&["--no-cache"]);
    assert_eq!(plain.code, 0, "{plain:?}");
    assert!(plain.stderr.matches("W1404").count() >= 2, "{plain:?}");
    assert!(plain.stderr.contains("noisy.vhd"), "{plain:?}");
    let cold = e.run(&[]);
    assert!(note(&cold, "store"), "{cold:?}");
    let warm = e.run(&[]);
    assert!(note(&warm, "hit"), "{warm:?}");
    assert_eq!(visible(&plain), visible(&warm));
}

/// A hit does not read the text of the sources (they are verified by size and modification time
/// and decoded lazily): a run that has nothing to show works with a source nobody can read now.
#[cfg(unix)]
#[test]
fn a_hit_without_diagnostics_does_not_read_the_sources() {
    use std::os::unix::fs::PermissionsExt;
    let e = Env::new("unread");
    // The testbench warns; drop its warning so no snippet is needed, and age every source.
    std::fs::write(e.tmp.join("tb.vhd"), TB.replace("v(5)", "v(1)")).expect("write");
    for f in ["pkg.vhd", "other.vhd", "tb.vhd"] {
        age(&e.tmp.join(f));
        assert_eq!(e.yavs(&["analyze", "--work", &e.work(), f]).code, 0, "{f}");
    }
    let cold = e.run(&[]);
    assert!(note(&cold, "store"), "{cold:?}");
    assert!(cold.stdout.contains("K=7 f=21"), "{cold:?}");
    let pkg = e.tmp.join("pkg.vhd");
    std::fs::set_permissions(&pkg, std::fs::Permissions::from_mode(0o000)).expect("chmod");
    let warm = e.run(&[]);
    std::fs::set_permissions(&pkg, std::fs::Permissions::from_mode(0o644)).expect("chmod");
    assert!(note(&warm, "hit"), "{warm:?}");
    assert_eq!(visible(&cold), visible(&warm));
}
