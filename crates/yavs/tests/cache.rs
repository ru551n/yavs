//! The compilation cache (`docs/decisions.md` "Compilation cache"): a warm run is
//! byte-identical to a cold one, editing a dependency invalidates, read-only libraries and
//! damaged entries still work, `--no-cache` and `yavs cache clean`.

mod common;
use common::tempdir;

use std::path::{Path, PathBuf};
use std::process::Command;

// (`f` reports, so the process calls it: a function the optimiser can copy into its caller would
// not be called, and the entry would have nothing to learn.)
const PKG: &str = "package p is\n  constant K : integer := 7;\n  function f (x : integer) return integer;\nend;\n\
package body p is\n  function f (x : integer) return integer is begin assert x > 0 report \"f\"; return x * K; end;\nend;\n";

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
    tmp: common::Tmp,
    /// Environment of every command.
    env: Vec<(&'static str, &'static str)>,
}

#[derive(Debug, PartialEq, Eq)]
struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Env {
    fn new(name: &str) -> Env {
        Env::with_env(name, &[])
    }

    fn with_env(name: &str, env: &[(&'static str, &'static str)]) -> Env {
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
        let e = Env {
            tmp,
            env: env.to_vec(),
        };
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
            .envs(self.env.iter().copied())
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

    /// A run that makes the entries. The first run of a closure only leaves a note (a design that
    /// runs once does not pay for an image), the second stores; returns the second.
    fn cold(&self, top: &str, more: &[&str]) -> Out {
        let first = self.run_top(top, more);
        assert!(note(&first, "note"), "{first:?}");
        self.run_top(top, more)
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
            .filter(|l| !l.starts_with("debug: cache:") && !l.starts_with("debug: units:"))
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
    // The first run of a closure only notes it (no image, no lowered packages); the second stores.
    let first = e.run(&[]);
    assert!(note(&first, "note") && !note(&first, "store"), "{first:?}");
    assert_eq!(e.entries(), 0);
    assert_eq!(visible(&plain), visible(&first));
    let cold = e.run(&[]);
    assert!(note(&cold, "store"), "{cold:?}");
    // The analysis and the lowered design (`sem`, `ir`).
    assert_eq!(e.entries(), 2);
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
    assert_eq!(e.entries(), 2);
}

#[test]
fn editing_a_dependency_invalidates() {
    let e = Env::new("edit");
    assert!(e.cold("tb", &[]).stdout.contains("K=7 f=21"));
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
    let after = e.cold("tb", &[]);
    assert!(note(&after, "store"), "{after:?}");
    assert!(after.stdout.contains("K=8 f=24"), "{after:?}");
    assert!(note(&e.run(&[]), "hit"));
}

#[test]
fn damaged_entries_are_ignored() {
    let e = Env::new("damaged");
    let cold = e.cold("tb", &[]);
    let img = e.entry_files("sem").pop().expect("entry");
    let good = std::fs::read(&img).expect("read");
    for bad in [good[..good.len() / 2].to_vec(), b"not an entry".to_vec(), {
        let mut g = good.clone();
        let n = g.len();
        g[n / 2] ^= 0x55;
        g
    }] {
        std::fs::write(&img, bad).expect("write");
        // Only the analysis image is under test: its hit must not be told from the lowered design's.
        for f in e.entry_files("ir") {
            std::fs::remove_file(f).expect("remove");
        }
        let r = e.run(&[]);
        assert!(!note(&r, "hit"), "{r:?}");
        assert_eq!(visible(&cold), visible(&r));
        assert!(note(&r, "store"), "the entry is rewritten");
        assert!(note(&e.run(&[]), "hit"));
    }
}

/// A library with more closures than the 96 entries it used to keep (#1027): a third run reads
/// everything and writes nothing, and when every entry is then damaged the results do not change.
#[test]
fn a_library_of_many_closures_stays_warm() {
    let e = Env::new("many");
    let tops: Vec<String> = (0..50).map(|i| format!("tb{i}")).collect();
    for t in &tops {
        let f = format!("{t}.vhd");
        std::fs::write(
            e.tmp.join(&f),
            TB.replace("entity tb is", &format!("entity {t} is"))
                .replace("of tb is", &format!("of {t} is")),
        )
        .expect("write");
        assert_eq!(e.yavs(&["analyze", "--work", &e.work(), &f]).code, 0, "{f}");
    }
    let env = &e;
    let pass = |more: &[&str]| -> Vec<Out> {
        std::thread::scope(|s| {
            let runs: Vec<_> = tops
                .chunks(tops.len() / 8)
                .map(|c| {
                    s.spawn(move || c.iter().map(|t| env.run_top(t, more)).collect::<Vec<_>>())
                })
                .collect();
            runs.into_iter()
                .flat_map(|r| r.join().expect("thread"))
                .collect()
        })
    };
    let same = |a: &[Out], b: &[Out]| {
        for (x, y) in a.iter().zip(b) {
            assert_eq!(visible(x), visible(y));
        }
    };
    let plain = pass(&["--no-cache"]);
    assert!(plain.iter().all(|o| o.code == 0), "{plain:?}");
    same(&plain, &pass(&[]));
    let stored = pass(&[]);
    same(&plain, &stored);
    assert!(stored.iter().all(|o| note(o, "store")));
    assert!(e.entries() >= 2 * tops.len(), "{}", e.entries());
    let warm = pass(&[]);
    same(&plain, &warm);
    for o in &warm {
        assert!(note(o, "hit") && !note(o, "store"), "{o:?}");
    }
    let n = e.entries();
    for kind in ["sem", "ir"] {
        for f in e.entry_files(kind) {
            let mut b = std::fs::read(&f).expect("read");
            let mid = b.len() / 2;
            b[mid] ^= 0x55;
            std::fs::write(&f, b).expect("write");
        }
    }
    same(&plain, &pass(&[]));
    assert_eq!(e.entries(), n);
}

#[cfg(unix)]
#[test]
fn read_only_library_is_not_cached_into() {
    let e = Env::new("readonly");
    let work = e.tmp.join("work");
    let plain = e.run(&["--no-cache"]);
    let before = tree(&work);
    set_tree_mode(&work, 0o555);
    let writable = std::fs::File::create(work.join("probe")).is_ok();
    let r = e.run(&[]);
    set_tree_mode(&work, 0o755);
    if writable {
        return; // running as a user the mode does not bind
    }
    assert_eq!(visible(&plain), visible(&r));
    assert_eq!(
        before,
        tree(&work),
        "nothing is written into a read-only library"
    );
}

/// Every file and directory below `dir`, sorted.
#[cfg(unix)]
fn tree(dir: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                todo.push(p.clone());
            }
            out.push(p);
        }
    }
    out.sort();
    out
}

/// Set the mode of `dir` and of every directory below it (a read-only library: a read-only mount
/// or another user's tree, the cache directories `analyze` made included).
#[cfg(unix)]
fn set_tree_mode(dir: &Path, mode: u32) {
    use std::os::unix::fs::PermissionsExt;
    let mut dirs: Vec<PathBuf> = tree(dir).into_iter().filter(|p| p.is_dir()).collect();
    dirs.push(dir.to_path_buf());
    // Children first when locking, the top first when unlocking.
    if mode & 0o200 == 0 {
        dirs.reverse();
    }
    for d in dirs {
        std::fs::set_permissions(&d, std::fs::Permissions::from_mode(mode)).expect("chmod");
    }
}

#[test]
fn clean_removes_entries() {
    // `--std-cache=off`: a clean of the shared store would hit every test running at the same time.
    let e = Env::new("clean");
    e.cold("tb", &[]);
    assert_eq!(e.entries(), 2);
    let work = e.work();
    let o = e.yavs(&["cache", "clean", "--work", &work, "--std-cache=off"]);
    assert_eq!(o.code, 0, "{o:?}");
    // The two of the run and the analysed units `analyze` made.
    let n: usize = o
        .stdout
        .split_whitespace()
        .nth(2)
        .and_then(|n| n.parse().ok())
        .expect("a count");
    assert!(n > 2, "{o:?}");
    assert!(!e.cache_dir().exists());
    // The records of the library's files are not cache entries (#599).
    let lib = e
        .cache_dir()
        .parent()
        .expect("library directory")
        .to_path_buf();
    assert!(lib.join("yavs-lib.d").is_dir() && lib.join("yavs-lib.toml").is_file());
    let again = e.yavs(&["cache", "clean", "--work", &work, "--std-cache=off"]);
    assert_eq!(again.code, 0);
    assert!(again.stdout.contains("0 entries"), "{again:?}");
    assert!(e.run(&[]).stdout.contains("K=7"));
}

#[test]
fn parallel_cold_runs_agree() {
    let e = Env::new("parallel");
    let plain = visible(&e.run(&["--no-cache"]));
    e.run(&[]); // leaves the note: the runs below all make the image
    std::thread::scope(|s| {
        let hs: Vec<_> = (0..4).map(|_| s.spawn(|| visible(&e.run(&[])))).collect();
        for h in hs {
            assert_eq!(plain, h.join().expect("join"));
        }
    });
    assert_eq!(e.entries(), 2);
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
    let cold = e.cold("tb", &[]);
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
    let cold = e.cold("tb2", &[&format!("--wave={}", wave("cold.fst"))]);
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

/// A package whose function holds a periodic composite constant of several megabytes once
/// written out: the pool keeps it compact (#446), so the lowering is cached, and a warm run
/// reads the same constant.
const BIG: &str = "package bigp is\n  impure function probe(i : integer) return integer;\n  signal sg : integer := 3;\nend;\n\
package body bigp is\n  type rec_t is record\n    n : integer;\n    v : bit_vector(524287 downto 0);\n  end record;\n\
  type rec_a is array (0 to 9) of rec_t;\n\
  type holder_t is protected\n    impure function get(i : integer) return integer;\n  end protected;\n\
  type holder_t is protected body\n    impure function get(i : integer) return integer is\n\
      constant e : rec_t := (7, (others => '1'));\n      constant r : rec_a := (others => e);\n    begin\n\
      if r(i).v(i * 1000) = '1' then return r(i).n + i; end if;\n      return -1;\n    end;\n  end protected body;\n\
  shared variable h : holder_t;\n  impure function probe(i : integer) return integer is begin return h.get(i); end;\n  constant boot : integer := probe(1);\nend;\n";

const BIGTB: &str = "use work.bigp.all;\nentity bigtb is end;\narchitecture a of bigtb is\n  constant k : integer := probe(2);\nbegin\n  process\n  begin\n\
    report \"probe \" & integer'image(k) & \" sg \" & integer'image(sg);\n    wait;\n  end process;\nend;\n";

#[test]
fn periodic_constant_does_not_block_the_lowering_cache() {
    let e = Env::new("big");
    for (f, t) in [("big.vhd", BIG), ("bigtb.vhd", BIGTB)] {
        std::fs::write(e.tmp.join(f), t).expect("write");
        assert_eq!(e.yavs(&["analyze", "--work", &e.work(), f]).code, 0, "{f}");
    }
    let plain = e.run_top("bigtb", &["--no-cache"]);
    assert_eq!(plain.code, 0, "{plain:?}");
    assert!(plain.stdout.contains("probe 9 sg 3"), "{plain:?}");
    let cold = e.cold("bigtb", &[]);
    assert!(note(&cold, "store low"), "{cold:?}");
    let image = e.entry_files("low");
    assert_eq!(image.len(), 1);
    assert!(std::fs::metadata(&image[0]).expect("image").len() < 1 << 16);
    let warm = e.run_top("bigtb", &[]);
    assert!(note(&warm, "hit low"), "{warm:?}");
    assert_eq!(visible(&plain), visible(&cold));
    assert_eq!(visible(&plain), visible(&warm));
}

#[test]
fn editing_a_package_replaces_its_lowering() {
    let e = Env::new("lowedit");
    assert!(e.cold("tb2", &[]).stdout.contains("ids 11 12"));
    assert!(note(&e.run_top("tb2", &[]), "hit low"));
    std::fs::write(
        e.tmp.join("rt.vhd"),
        RT.replace("n : integer := 10", "n : integer := 20"),
    )
    .expect("write");
    assert_eq!(e.yavs(&["analyze", "--work", &e.work(), "rt.vhd"]).code, 0);
    let after = e.cold("tb2", &[]);
    assert!(note(&after, "store low"), "{after:?}");
    assert!(after.stdout.contains("ids 21 22 next 23"), "{after:?}");
    assert!(note(&e.run_top("tb2", &[]), "hit low"));
}

#[test]
fn damaged_lowering_is_ignored() {
    let e = Env::new("lowdamaged");
    let cold = e.cold("tb2", &[]);
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
    e.run_top("tb2", &[]); // leaves the note: the runs below all make the entries
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

/// Package state after initialisation (`init` entries, #407 stage 2 step 2): a protected type
/// whose state is a heap object made by the initialiser and grown by elaboration-time calls.
const HEAPY: &str = "package heapy is\n  impure function total return integer;\n  impure function push(x : integer) return integer;\nend;\n\
package body heapy is\n  type ints is access integer_vector;\n\
  type bag_t is protected\n    impure function total return integer;\n    impure function push(x : integer) return integer;\n  end protected;\n\
  type bag_t is protected body\n    variable buf : ints := new integer_vector'(1, 2, 3, 4);\n    variable n : integer := 4;\n\
    impure function total return integer is\n      variable s : integer := 0;\n    begin\n\
      for i in 0 to n - 1 loop s := s + buf(i); end loop;\n      return s;\n    end;\n\
    impure function push(x : integer) return integer is\n      variable nb : ints;\n    begin\n\
      if n = buf'length then\n        nb := new integer_vector(0 to 2 * n - 1);\n        nb(0 to n - 1) := buf.all;\n        deallocate(buf);\n        buf := nb;\n      end if;\n\
      buf(n) := x;\n      n := n + 1;\n      return n;\n    end;\n  end protected body;\n\
  shared variable bag : bag_t;\n\
  impure function total return integer is begin return bag.total; end;\n\
  impure function push(x : integer) return integer is begin return bag.push(x); end;\nend;\n";

const HEAPYTB: &str = "use work.heapy.all;\nentity heapytb is end;\narchitecture a of heapytb is\n\
  constant n1 : integer := push(5);\n  constant t1 : integer := total;\nbegin\n  process\n    variable n2, n3 : integer;\n  begin\n\
    report \"n1 \" & integer'image(n1) & \" t1 \" & integer'image(t1);\n\
    n2 := push(7);\n    n3 := push(8);\n    report \"n2 \" & integer'image(n2) & \" n3 \" & integer'image(n3) & \" total \" & integer'image(total);\n    wait;\n  end process;\nend;\n";

/// Runs the same testbench with the cache off, cold, warm and warm at 4 threads: all four agree.
fn same_everywhere(e: &Env, top: &str) -> Out {
    let plain = visible(&e.run_top(top, &["--no-cache"]));
    assert_eq!(plain.code, 0, "{plain:?}");
    let cold = e.cold(top, &[]);
    let warm = e.run_top(top, &[]);
    let warm4 = e.run_top(top, &["--threads", "4"]);
    assert_eq!(plain, visible(&cold), "cold");
    assert_eq!(plain, visible(&warm), "warm");
    assert_eq!(plain, visible(&warm4), "warm, 4 threads");
    plain
}

#[test]
fn package_state_after_initialisation_is_cached() {
    let e = Env::new("init");
    let wave = |n: &str| e.tmp.join(n).display().to_string();
    let plain = e.run_top(
        "tb2",
        &["--no-cache", &format!("--wave={}", wave("plain.fst"))],
    );
    assert!(plain.stdout.contains("ids 11 12 next 13 sg 3"), "{plain:?}");
    assert!(
        e.entry_files("init").is_empty(),
        "--no-cache must not write"
    );
    let cold = e.cold("tb2", &[&format!("--wave={}", wave("cold.fst"))]);
    assert!(note(&cold, "store init"), "{cold:?}");
    assert!(!note(&cold, "hit init"), "{cold:?}");
    assert_eq!(e.entry_files("init").len(), 1);
    let warm = e.run_top("tb2", &[&format!("--wave={}", wave("warm.fst"))]);
    assert!(note(&warm, "hit init"), "{warm:?}");
    assert!(!note(&warm, "store init"), "{warm:?}");
    assert_eq!(visible(&plain), visible(&cold));
    assert_eq!(visible(&plain), visible(&warm));
    let read = |n: &str| std::fs::read(e.tmp.join(n)).expect("wave");
    assert_eq!(read("plain.fst"), read("cold.fst"));
    assert_eq!(read("plain.fst"), read("warm.fst"));
    let w4 = e.run_top("tb2", &["--threads", "4"]);
    assert!(note(&w4, "hit init"), "{w4:?}");
    assert_eq!(visible(&plain), visible(&w4));
    // A design without run-time packages has no such entry.
    assert_eq!(e.run(&[]).code, 0);
    assert_eq!(e.entry_files("init").len(), 1);
}

/// No elaboration-time call: the simulation initialises the packages itself, and the time
/// resolution it runs with is part of the key.
const TB4: &str = "use work.reg.all;\nentity tb4 is end;\narchitecture a of tb4 is\nbegin\n  process\n  begin\n\
    report \"id \" & integer'image(get_id) & \" sg \" & integer'image(sg);\n    wait;\n  end process;\nend;\n";

#[test]
fn simulation_start_uses_the_cached_state() {
    let e = Env::new("initbuild");
    std::fs::write(e.tmp.join("tb4.vhd"), TB4).expect("write");
    assert_eq!(e.yavs(&["analyze", "--work", &e.work(), "tb4.vhd"]).code, 0);
    let plain = visible(&e.run_top("tb4", &["--no-cache"]));
    assert!(plain.stdout.contains("id 11 sg 3"), "{plain:?}");
    let cold = e.cold("tb4", &[]);
    assert!(note(&cold, "store init"), "{cold:?}");
    let warm = e.run_top("tb4", &[]);
    assert!(note(&warm, "hit init"), "{warm:?}");
    assert_eq!(plain, visible(&cold));
    assert_eq!(plain, visible(&warm));
    let ps = e.run_top("tb4", &["--resolution", "ps"]);
    assert!(note(&ps, "store init"), "{ps:?}");
    assert_eq!(e.entry_files("init").len(), 2);
    assert_eq!(plain, visible(&ps));
    assert!(note(&e.run_top("tb4", &["--resolution", "ps"]), "hit init"));
    assert!(note(&e.run_top("tb4", &["--threads", "4"]), "hit init"));
}

#[test]
fn heap_and_protected_state_is_restored() {
    let e = Env::new("heapy");
    for (f, t) in [("heapy.vhd", HEAPY), ("heapytb.vhd", HEAPYTB)] {
        std::fs::write(e.tmp.join(f), t).expect("write");
        assert_eq!(e.yavs(&["analyze", "--work", &e.work(), f]).code, 0, "{f}");
    }
    let plain = same_everywhere(&e, "heapytb");
    assert!(plain.stdout.contains("n1 5 t1 15"), "{plain:?}");
    assert!(plain.stdout.contains("n2 6 n3 7 total 30"), "{plain:?}");
    assert_eq!(e.entry_files("init").len(), 1);
    assert!(note(&e.run_top("heapytb", &[]), "hit init"));
}

#[test]
fn editing_a_package_replaces_its_state() {
    let e = Env::new("initedit");
    assert!(e.cold("tb2", &[]).stdout.contains("ids 11 12"));
    assert!(note(&e.run_top("tb2", &[]), "hit init"));
    std::fs::write(
        e.tmp.join("rt.vhd"),
        RT.replace("n : integer := 10", "n : integer := 20"),
    )
    .expect("write");
    assert_eq!(e.yavs(&["analyze", "--work", &e.work(), "rt.vhd"]).code, 0);
    let after = e.cold("tb2", &[]);
    assert!(note(&after, "store init"), "{after:?}");
    assert!(after.stdout.contains("ids 21 22 next 23"), "{after:?}");
    assert!(note(&e.run_top("tb2", &[]), "hit init"));
}

/// Initialisers that report, and one that opens a file. `{}` is the declarations of the effect.
const EFFECTS: &str = "library std; use std.textio.all;\n\
package fx is\n  impure function probe return integer;\nend;\n\
package body fx is\n  type holder_t is protected\n    impure function get return integer;\n  end protected;\n\
  type holder_t is protected body\n    variable v : integer := 5;\n    impure function get return integer is begin return v; end;\n  end protected body;\n\
  shared variable h : holder_t;\n\
  impure function say return integer is begin\n    report \"init note\";\n    report \"init warning\" severity warning;\n    report \"init error\" severity error;\n    return 3;\n  end;\n\
{}\
  impure function probe return integer is begin return h.get + noted; end;\nend;\n";

const EFFECTSTB: &str = "use work.fx.all;\nentity fxtb is end;\narchitecture a of fxtb is\n  constant k : integer := probe;\nbegin\n  process\n  begin\n\
    report \"k \" & integer'image(k);\n    wait;\n  end process;\nend;\n";

fn effects_env(name: &str, decls: &str) -> Env {
    let e = Env::new(name);
    let pkg = EFFECTS.replace("{}", decls);
    for (f, t) in [("fx.vhd", pkg.as_str()), ("fxtb.vhd", EFFECTSTB)] {
        std::fs::write(e.tmp.join(f), t).expect("write");
        assert_eq!(e.yavs(&["analyze", "--work", &e.work(), f]).code, 0, "{f}");
    }
    e
}

/// What an initialiser reports is printed again, and counted for the exit status, when its state
/// is restored: a warm run ends as a cold one does (an `error` report makes the run exit 1).
#[test]
fn reports_of_initialisers_are_replayed() {
    let e = effects_env("reports", "  constant noted : integer := say;\n");
    let plain = e.run_top("fxtb", &["--no-cache"]);
    assert_eq!(plain.code, 1, "{plain:?}");
    assert!(plain.stdout.contains("k 8"), "{plain:?}");
    let count = |o: &Out, what: &str| o.stderr.matches(what).count();
    assert!(count(&plain, "(init): init note") >= 1, "{plain:?}");
    let cold = e.cold("fxtb", &[]);
    assert!(note(&cold, "store init"), "{cold:?}");
    for threads in ["1", "4"] {
        let warm = e.run_top("fxtb", &["--threads", threads]);
        assert!(note(&warm, "hit init"), "{warm:?}");
        assert_eq!(visible(&plain), visible(&warm), "threads {threads}");
        for what in ["init note", "init warning", "init error"] {
            assert_eq!(count(&plain, what), count(&warm, what), "{what}: {warm:?}");
        }
    }
}

/// An initialiser that opens a file is not cached: a warm run does it again.
#[test]
fn package_that_opens_a_file_is_not_cached() {
    let e = effects_env(
        "file",
        "  file out_f : text open write_mode is \"fx-out.txt\";\n  constant noted : integer := 3;\n",
    );
    let plain = e.run_top("fxtb", &["--no-cache"]);
    assert_eq!(plain.code, 0, "{plain:?}");
    assert!(plain.stdout.contains("k 8"), "{plain:?}");
    e.run_top("fxtb", &[]); // the note
    for _ in 0..3 {
        let r = e.run_top("fxtb", &[]);
        assert!(!note(&r, "store init") && !note(&r, "hit init"), "{r:?}");
        assert!(r.stderr.contains("no init entry"), "{r:?}");
        assert_eq!(visible(&plain), visible(&r));
    }
    assert!(e.entry_files("init").is_empty());
}

#[test]
fn damaged_state_is_ignored() {
    let e = Env::new("initdamaged");
    let cold = e.cold("tb2", &[]);
    let img = e.entry_files("init").pop().expect("init entry");
    let good = std::fs::read(&img).expect("read");
    for bad in [good[..good.len() / 2].to_vec(), b"not an entry".to_vec()] {
        std::fs::write(&img, bad).expect("write");
        let r = e.run_top("tb2", &[]);
        assert!(!note(&r, "hit init"), "{r:?}");
        assert_eq!(visible(&cold), visible(&r));
        assert!(note(&r, "store init"), "the entry is rewritten");
        assert!(note(&e.run_top("tb2", &[]), "hit init"));
    }
}

#[test]
fn parallel_cold_runs_agree_on_state() {
    let e = Env::new("initparallel");
    let plain = visible(&e.run_top("tb2", &["--no-cache"]));
    e.run_top("tb2", &[]); // leaves the note: the runs below all make the entries
    std::thread::scope(|s| {
        let hs: Vec<_> = (0..4)
            .map(|_| s.spawn(|| visible(&e.run_top("tb2", &[]))))
            .collect();
        for h in hs {
            assert_eq!(plain, h.join().expect("join"));
        }
    });
    assert_eq!(e.entry_files("init").len(), 1);
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
    let cold = e.cold("tb", &[]);
    assert!(note(&cold, "store"), "{cold:?}");
    let warm = e.run(&[]);
    assert!(note(&warm, "hit"), "{warm:?}");
    assert_eq!(visible(&plain), visible(&warm));
}

/// A later file that redefines a unit of an earlier one (the same entity name, another context
/// clause) leaves the earlier file's manifest record with its remaining units only. Analysing the
/// earlier file again must give what the first time did: the file is parsed, its entity analysed
/// again (#577; it was registered from the architecture alone and met the other file's entity).
#[test]
fn reanalysing_a_file_whose_unit_was_replaced_gives_the_first_result() {
    let e = Env::new("replaced");
    let first = "library ieee; use ieee.std_logic_1164.all;\nentity ent is end;\n\
architecture rtl of ent is\n  signal s : std_logic;\nbegin\nend;\n";
    let second = "entity ent is end;\narchitecture other of ent is begin end;\n";
    std::fs::write(e.tmp.join("first.vhd"), first).expect("write");
    std::fs::write(e.tmp.join("second.vhd"), second).expect("write");
    for f in [
        "first.vhd",
        "second.vhd",
        "first.vhd",
        "second.vhd",
        "first.vhd",
    ] {
        let o = e.yavs(&["analyze", "--work", &e.work(), f]);
        assert_eq!(o.code, 0, "{f}: {o:?}");
    }
}

const ENT_E: &str = "entity e is end entity;\n";

fn arch_says(text: &str) -> String {
    format!(
        "architecture a of e is begin\n  process begin report \"{text}\"; wait; end process;\nend architecture;\n"
    )
}

fn pkg_c(v: u32) -> String {
    format!("package pr is constant c : integer := {v}; end package;\n")
}

const USES_E_A: &str = "entity t is end entity;\narchitecture s of t is begin u : entity work.e(a); end architecture;\n";
const USES_P: &str = "use work.pr.all;\nentity t is end entity;\narchitecture a of t is begin\n  \
process begin report \"c=\" & integer'image(c); wait; end process;\nend architecture;\n";
const USES_P_X: &str = "use work.pr.all;\nentity x is end entity;\narchitecture a of x is begin\n  \
process begin report \"x c=\" & integer'image(c); wait; end process;\nend architecture;\n";
const USES_P_AND_X: &str = "use work.pr.all;\nentity t is end entity;\narchitecture a of t is begin\n  \
u : entity work.x;\n  process begin report \"t c=\" & integer'image(c); wait; end process;\nend architecture;\n";

/// Writes `files`, analyzes `order` one call each, and runs `t` cold and warm: both print `want`.
fn runs_as(name: &str, files: &[(&str, String)], order: &[&str], want: &[&str]) {
    let e = Env::new(name);
    for (f, text) in files {
        std::fs::write(e.tmp.join(f), text).expect("write");
    }
    for f in order {
        let o = e.yavs(&["analyze", "--work", &e.work(), f]);
        assert_eq!(o.code, 0, "{name}: {f}: {o:?}");
    }
    for round in ["cold", "warm"] {
        let o = e.run_top("t", &[]);
        assert_eq!(o.code, 0, "{name} {round}: {o:?}");
        for w in want {
            assert!(o.stdout.contains(w), "{name} {round}: wants {w}: {o:?}");
        }
    }
}

/// A run uses the units the library holds: the last analyzed of each name. A unit analysed against
/// an earlier version is analysed again (13.5), and the old copy in a file that lost it never runs
/// (#1054), whatever the order of the closure. `docs/design/lrm-checks.md` L174.
#[test]
fn a_run_uses_the_last_analyzed_unit_of_each_name() {
    let f = |a: &str| arch_says(a) + &pkg_c(1);
    let s = |t: &str| t.to_string();
    // Nothing is obsolete here: `t` needs the two files in the order `g`, `f`.
    let t = format!("use work.qr.all;\nuse work.pr.all;\n{USES_E_A}");
    let q = "package qr is constant k : integer := 5; end package;\n";
    let g = arch_says("NEW") + q;
    let files = [
        ("e.vhd", s(ENT_E)),
        ("f.vhd", f("OLD")),
        ("g.vhd", g),
        ("t.vhd", t),
    ];
    runs_as(
        "reana_nothing_obsolete",
        &files,
        &["e.vhd", "f.vhd", "g.vhd", "t.vhd"],
        &["NEW"],
    );
    // The file with the old copy analysed last has the current unit.
    runs_as(
        "reana_old_file_last",
        &files,
        &["e.vhd", "g.vhd", "f.vhd", "t.vhd"],
        &["OLD"],
    );
    // The issue's case: a package replaced under a user.
    runs_as(
        "reana_package",
        &[
            ("p1.vhd", pkg_c(1)),
            ("p2.vhd", pkg_c(2)),
            ("t.vhd", s(USES_P)),
        ],
        &["p1.vhd", "t.vhd", "p2.vhd"],
        &["c=2"],
    );
    // The old copy shares a file with a user, which is analysed again against the new one.
    runs_as(
        "reana_old_copy_with_user",
        &[
            ("f.vhd", pkg_c(1) + USES_P_X),
            ("p2.vhd", pkg_c(2)),
            ("t.vhd", s(USES_P_AND_X)),
        ],
        &["f.vhd", "p2.vhd", "t.vhd"],
        &["x c=2", "t c=2"],
    );
    // One name twice in one file, with a user between the two.
    runs_as(
        "reana_twice_in_file",
        &[("f.vhd", pkg_c(1) + USES_P + &pkg_c(2))],
        &["f.vhd"],
        &["c=2"],
    );
    // An architecture replaced under the instance (and the configuration) naming it.
    let conf = "configuration t of top is\n  for s\n    for u : e use entity work.e(a); end for;\n  end for;\nend configuration;\n";
    let top = "entity top is end entity;\narchitecture s of top is\n  component e is end component;\nbegin\n  u : e;\nend architecture;\n";
    runs_as(
        "reana_arch_config",
        &[
            ("e.vhd", s(ENT_E)),
            ("f1.vhd", arch_says("F1") + conf),
            ("top.vhd", s(top)),
            ("f2.vhd", arch_says("F2")),
        ],
        &["e.vhd", "top.vhd", "f1.vhd", "f2.vhd"],
        &["F2"],
    );
    // An entity replaced: its architecture is analysed again against the new one.
    runs_as(
        "reana_entity",
        &[
            ("e.vhd", s(ENT_E)),
            ("a.vhd", arch_says("A")),
            ("t.vhd", s(USES_E_A)),
            ("e2.vhd", s(ENT_E)),
        ],
        &["e.vhd", "a.vhd", "t.vhd", "e2.vhd"],
        &["A"],
    );
    // A package body in the file of the package's old copy: analysed against the new package.
    let pf = "package pr is function f return integer; end package;\npackage body pr is\n  function f return integer is begin return 7; end function;\nend package body;\n";
    let p2 = "package pr is function f return integer; constant c : integer := 3; end package;\n";
    let tb = "use work.pr.all;\nentity t is end entity;\narchitecture a of t is begin\n  process begin report \"f=\" & integer'image(f) & \" c=\" & integer'image(c); wait; end process;\nend architecture;\n";
    runs_as(
        "reana_body",
        &[("f.vhd", s(pf)), ("p2.vhd", s(p2)), ("t.vhd", s(tb))],
        &["f.vhd", "p2.vhd", "t.vhd"],
        &["f=7 c=3"],
    );
    // A package takes the name of an entity: one name space of primary units.
    runs_as(
        "reana_other_kind",
        &[
            ("p1.vhd", s("entity pr is end entity;\n")),
            ("p2.vhd", pkg_c(2)),
            ("t.vhd", s(USES_P)),
        ],
        &["p1.vhd", "p2.vhd", "t.vhd"],
        &["c=2"],
    );
}

/// Same files, same closure order, other owner of the unit: the cache key tells them apart.
#[test]
fn analysing_a_file_again_changes_which_copy_a_warm_run_uses() {
    let e = Env::new("reana_owner");
    let g = format!(
        "use work.pr.all;\npackage qr is constant k : integer := c; end package;\n{}",
        arch_says("NEW")
    );
    let t = format!("use work.qr.all;\n{USES_E_A}");
    for (f, text) in [
        // Package `z` keeps `e.vhd` before `f.vhd` whichever file holds `a`.
        ("e.vhd", format!("{ENT_E}package z is end package;\n")),
        ("f.vhd", arch_says("OLD") + "use work.z.all;\n" + &pkg_c(1)),
        ("g.vhd", g),
        ("t.vhd", t),
    ] {
        std::fs::write(e.tmp.join(f), text).expect("write");
        assert_eq!(e.yavs(&["analyze", "--work", &e.work(), f]).code, 0, "{f}");
    }
    for (again, want) in [
        (None, "NEW"),
        (Some("f.vhd"), "OLD"),
        (Some("g.vhd"), "NEW"),
    ] {
        if let Some(f) = again {
            assert_eq!(e.yavs(&["analyze", "--work", &e.work(), f]).code, 0, "{f}");
        }
        for round in ["cold", "warm"] {
            let o = e.run_top("t", &[]);
            assert_eq!(o.code, 0, "{again:?} {round}: {o:?}");
            assert!(o.stdout.contains(want), "{again:?} {round}: {o:?}");
        }
    }
}

/// A unit that no longer analyses against what replaced its dependency fails the run, with the
/// error of that analysis (13.5: it must be analysed again before it is used).
#[test]
fn a_unit_that_does_not_fit_the_replacement_fails_the_run() {
    let e = Env::new("reana_breaks");
    let p2 = "package pr is constant d : integer := 2; end package;\n".to_string();
    for (f, text) in [
        ("p1.vhd", pkg_c(1)),
        ("t.vhd", USES_P.to_string()),
        ("p2.vhd", p2),
    ] {
        std::fs::write(e.tmp.join(f), text).expect("write");
        assert_eq!(e.yavs(&["analyze", "--work", &e.work(), f]).code, 0, "{f}");
    }
    let o = e.run_top("t", &[]);
    assert_eq!(o.code, 2, "{o:?}");
    assert!(
        o.stderr.contains("E1100") && o.stderr.contains("t.vhd"),
        "{o:?}"
    );
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
    let cold = e.cold("tb", &[]);
    assert!(note(&cold, "store"), "{cold:?}");
    assert!(cold.stdout.contains("K=7 f=21"), "{cold:?}");
    let pkg = e.tmp.join("pkg.vhd");
    std::fs::set_permissions(&pkg, std::fs::Permissions::from_mode(0o000)).expect("chmod");
    let warm = e.run(&[]);
    std::fs::set_permissions(&pkg, std::fs::Permissions::from_mode(0o644)).expect("chmod");
    assert!(note(&warm, "hit"), "{warm:?}");
    assert_eq!(visible(&cold), visible(&warm));
}

/// A source that changed without moving its size or modification time (so the hit trusts it), or
/// that cannot be read, costs the diagnostic its snippet and nothing else: code, location, labels
/// and a note stay, in every message format. Never an internal error, never the old text.
#[cfg(unix)]
#[test]
fn an_unavailable_source_drops_the_snippet_only() {
    use std::os::unix::fs::PermissionsExt;
    let e = Env::new("srcnote");
    std::fs::write(e.tmp.join("noisy.vhd"), NOISY).expect("write");
    let tb = TB.replace("use work.p.all;", "use work.p.all; use work.n.all;");
    std::fs::write(e.tmp.join("tb.vhd"), tb).expect("write");
    for f in ["noisy.vhd", "tb.vhd"] {
        age(&e.tmp.join(f));
        assert_eq!(e.yavs(&["analyze", "--work", &e.work(), f]).code, 0, "{f}");
    }
    let noisy = e.tmp.join("noisy.vhd");
    let cold = e.cold("tb", &[]);
    assert!(note(&cold, "store"), "{cold:?}");
    assert!(cold.stderr.contains("\n9 | "), "{cold:?}");
    let modified = std::fs::metadata(&noisy)
        .and_then(|m| m.modified())
        .expect("mtime");
    let check = |o: &Out, why: &str| {
        assert!(note(o, "hit"), "{o:?}");
        assert_eq!(o.code, 0, "{o:?}");
        assert!(!o.stderr.contains("internal error"), "{o:?}");
        assert!(!o.stderr.contains("panicked"), "{o:?}");
        assert!(!o.stderr.contains("\n9 | "), "no snippet: {o:?}");
        assert!(o.stderr.contains("W1404"), "{o:?}");
        assert!(o.stderr.contains("noisy.vhd:9:"), "location: {o:?}");
        assert!(
            o.stderr.contains(&format!("{why}; snippet omitted")),
            "{o:?}"
        );
    };
    // Changed, with the same size and modification time.
    std::fs::write(&noisy, NOISY.replace("false", "true_")).expect("write");
    std::fs::File::options()
        .write(true)
        .open(&noisy)
        .and_then(|f| f.set_modified(modified))
        .expect("restore mtime");
    let warm = e.run(&[]);
    check(&warm, "changed since it was analysed");
    let json = e.run(&["--message-format=json"]);
    assert!(json.stderr.contains("\"code\":\"W1404\""), "{json:?}");
    assert!(json.stderr.contains("\"line_start\":9"), "{json:?}");
    assert!(!json.stderr.contains("\\n9 | "), "{json:?}");
    let short = e.run(&["--message-format=short"]);
    assert!(short.stderr.contains("noisy.vhd:9:"), "{short:?}");
    assert!(!short.stderr.contains("internal error"), "{short:?}");
    // Unreadable.
    std::fs::write(&noisy, NOISY).expect("write");
    std::fs::File::options()
        .write(true)
        .open(&noisy)
        .and_then(|f| f.set_modified(modified))
        .expect("restore mtime");
    std::fs::set_permissions(&noisy, std::fs::Permissions::from_mode(0o000)).expect("chmod");
    let warm = e.run(&[]);
    std::fs::set_permissions(&noisy, std::fs::Permissions::from_mode(0o644)).expect("chmod");
    check(&warm, "could not be read");
}

/// Testbenches of one library share the analysis of the packages they both use: the second to
/// start with them stores a prefix image next to the library of the last package (the first
/// only leaves a note, #549), the third restores it and analyzes the rest, with the output of an
/// analysis from source. Editing a package of the prefix invalidates it.
#[test]
fn closures_share_the_analysis_of_their_first_files() {
    let tmp = tempdir("prefix");
    let dir = |l: &str| tmp.join(l);
    let spec = |l: &str| format!("{l}:{}", dir(l).display());
    let analyze = |lib: &str, map: &[&str], file: &Path| {
        let mut a = vec!["analyze".to_string(), format!("--work={}", spec(lib))];
        a.extend(map.iter().map(|m| format!("--map={}", spec(m))));
        a.push(file.display().to_string());
        let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
            .args(&a)
            .output()
            .expect("analyze");
        assert!(o.status.success(), "{a:?}: {o:?}");
    };
    // A chain of ten packages in `liba`, so that every closure starts with ten files of it.
    let pkg = |i: usize| {
        let uses = if i == 0 {
            String::new()
        } else {
            format!("use work.pa{}.all;\n", i - 1)
        };
        let prev = if i == 0 {
            "0".to_string()
        } else {
            format!("k{}", i - 1)
        };
        format!("{uses}package pa{i} is\n  constant k{i} : integer := {prev} + {i};\nend;\n")
    };
    for i in 0..10 {
        let f = tmp.join(format!("pa{i}.vhd"));
        std::fs::write(&f, pkg(i)).expect("write");
        analyze("liba", &[], &f);
    }
    let tb = |n: u32| {
        format!(
            "library liba; use liba.pa9.all;\nentity tb{n} is end;\narchitecture a of tb{n} is\nbegin\n  process\n  begin\n    \
             report \"k=\" & integer'image(k9 + {n});\n    wait;\n  end process;\nend;\n"
        )
    };
    for n in [1, 2, 3] {
        let f = tmp.join(format!("tb{n}.vhd"));
        std::fs::write(&f, tb(n)).expect("write");
        analyze("work", &["liba"], &f);
    }
    let run = |top: &str, more: &[&str]| {
        let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
            .env_remove("YAVS_STDLIB")
            .env("YAVS_LOG", "debug")
            .args(["--color", "never", "run"])
            .arg(format!("--work={}", spec("work")))
            .arg(format!("--map={}", spec("liba")))
            .args(more)
            .arg(top)
            .output()
            .expect("run");
        Out {
            code: o.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        }
    };
    let plain1 = run("tb1", &["--no-cache"]);
    let plain2 = run("tb2", &["--no-cache"]);
    let plain3 = run("tb3", &["--no-cache"]);
    assert!(plain1.stdout.contains("k=46"), "{plain1:?}");
    assert!(plain2.stdout.contains("k=47"), "{plain2:?}");
    assert!(plain3.stdout.contains("k=48"), "{plain3:?}");
    let first = run("tb1", &[]);
    assert!(!note(&first, "store prefix of"), "one sighting: {first:?}");
    assert_eq!(visible(&plain1), visible(&first));
    let second = run("tb2", &[]);
    assert!(note(&second, "store prefix of 10 files"), "{second:?}");
    assert_eq!(visible(&plain2), visible(&second));
    let third = run("tb3", &[]);
    assert!(note(&third, "hit prefix of 10 files"), "{third:?}");
    assert!(!note(&third, "store prefix of"), "{third:?}");
    assert_eq!(visible(&plain3), visible(&third));
    // The same testbench again makes its own image, the next run hits it.
    run("tb3", &[]);
    let own = run("tb3", &[]);
    assert!(
        note(&own, "hit ") && !note(&own, "hit prefix"),
        "own image: {own:?}"
    );
    // A package of the prefix changes: no prefix image applies, and the new value is what runs.
    let f = tmp.join("pa3.vhd");
    std::fs::write(&f, pkg(3).replace("+ 3", "+ 4")).expect("write");
    analyze("liba", &[], &f);
    let edited = run("tb1", &[]);
    assert!(!note(&edited, "hit prefix"), "{edited:?}");
    assert!(edited.stdout.contains("k=47"), "{edited:?}");
}

/// A run that continues from a prefix image and analyses a unit after it from source (its entry
/// is gone) stores that unit, and the entry replays as the analysis from source does (the check
/// mode of `analyze` compares the two).
#[test]
fn a_unit_analysed_on_a_prefix_image_is_stored_right() {
    let tmp = tempdir("prefix_store");
    let dir = |l: &str| tmp.join(l);
    let spec = |l: &str| format!("{l}:{}", dir(l).display());
    let yavs = |args: &[String], check: bool| {
        let mut c = Command::new(env!("CARGO_BIN_EXE_yavs"));
        c.env_remove("YAVS_STDLIB")
            .env("YAVS_LOG", "debug")
            .args(["--color", "never"])
            .args(args);
        if check {
            c.env("YAVS_UNIT_CHECK", "1");
        }
        let o = c.output().expect("yavs");
        Out {
            code: o.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        }
    };
    let analyze = |lib: &str, map: &[&str], file: &str, check: bool| {
        let mut a = vec!["analyze".to_string(), format!("--work={}", spec(lib))];
        a.extend(map.iter().map(|m| format!("--map={}", spec(m))));
        a.push(tmp.join(file).display().to_string());
        let o = yavs(&a, check);
        assert_eq!(o.code, 0, "{a:?}: {o:?}");
        o
    };
    for i in 0..10 {
        let uses = if i == 0 {
            String::new()
        } else {
            format!("use work.pa{}.all;\n", i - 1)
        };
        let prev = if i == 0 {
            "0".into()
        } else {
            format!("k{}", i - 1)
        };
        let text =
            format!("{uses}package pa{i} is\n  constant k{i} : integer := {prev} + {i};\nend;\n");
        std::fs::write(tmp.join(format!("pa{i}.vhd")), text).expect("write");
        analyze("liba", &[], &format!("pa{i}.vhd"), false);
    }
    let wp = "library liba; use liba.pa9.all;\npackage wp is\n  constant w : integer := k9 * 2;\n  \
function f (x : integer) return integer;\nend;\npackage body wp is\n  \
function f (x : integer) return integer is begin return x + w; end;\nend;\n";
    std::fs::write(tmp.join("wp.vhd"), wp).expect("write");
    analyze("work", &["liba"], "wp.vhd", false);
    for n in [1, 2, 3] {
        let tb = format!(
            "use work.wp.all;\nentity tb{n} is end;\narchitecture a of tb{n} is\nbegin\n  process\n  begin\n    \
             report \"f=\" & integer'image(f({n}));\n    wait;\n  end process;\nend;\n"
        );
        std::fs::write(tmp.join(format!("tb{n}.vhd")), tb).expect("write");
        analyze("work", &["liba"], &format!("tb{n}.vhd"), false);
    }
    let run = |top: &str| {
        yavs(
            &[
                "run".into(),
                format!("--work={}", spec("work")),
                format!("--map={}", spec("liba")),
                top.into(),
            ],
            false,
        )
    };
    run("tb1");
    let second = run("tb2");
    assert!(note(&second, "store prefix of 10 files"), "{second:?}");
    let units = dir("work").join("yavs-cache").join("units");
    for f in std::fs::read_dir(&units).expect("units").flatten() {
        std::fs::remove_file(f.path()).expect("remove");
    }
    let third = run("tb3");
    assert_eq!(third.code, 0, "{third:?}");
    assert!(third.stdout.contains("f=93"), "{third:?}");
    assert!(note(&third, "hit prefix of 10 files"), "{third:?}");
    let stored = third
        .stderr
        .lines()
        .filter(|l| l.starts_with("debug: units: store work "))
        .count();
    assert!(stored >= 3, "wp, its body and tb3: {third:?}");
    for f in ["wp.vhd", "tb3.vhd"] {
        let o = analyze("work", &["liba"], f, true);
        assert!(o.stderr.contains("debug: units: hit work."), "{f}: {o:?}");
    }
}

/// Two entities for a generate that picks one by a generic: the lowered design differs with the
/// generic, the closure does not (#479).
const PICK: &str = "entity ea is end;\narchitecture a of ea is\nbegin\n  process\n  begin\n    report \"from ea\";\n    wait;\n  end process;\nend;\n\
entity eb is end;\narchitecture a of eb is\nbegin\n  process\n  begin\n    report \"from eb\";\n    wait;\n  end process;\nend;\n\
entity pick is generic (g : boolean := true); end;\narchitecture a of pick is\nbegin\n  \
ua : if g generate\n    u : entity work.ea;\n  end generate;\n  ub : if not g generate\n    u : entity work.eb;\n  end generate;\nend;\n";

/// The lowered design (architectures and every subprogram they reach) is cached next to the
/// analysis: a warm run is identical to a cold one and to one without the cache, with a
/// package-state session (`tb2`) and without (`tb`), at `--threads 4` and with a waveform.
#[test]
fn lowered_design_is_cached() {
    let e = Env::new("ir");
    let wave = |n: &str| e.tmp.join(n).display().to_string();
    for top in ["tb", "tb2"] {
        let plain = e.run_top(
            top,
            &["--no-cache", &format!("--wave={}", wave("plain.fst"))],
        );
        assert_eq!(plain.code, 0, "{plain:?}");
        assert!(e.entry_files("ir").is_empty(), "--no-cache must not write");
        let cold = e.cold(top, &[&format!("--wave={}", wave("cold.fst"))]);
        assert!(note(&cold, "store ir"), "{top}: {cold:?}");
        let warm = e.run_top(top, &[&format!("--wave={}", wave("warm.fst"))]);
        assert!(note(&warm, "hit ir"), "{top}: {warm:?}");
        // The cold run stored the optimised copies of what it called: nothing new to learn.
        assert!(!note(&warm, "store ir"), "{top}: {warm:?}");
        assert_eq!(visible(&plain), visible(&cold), "{top}");
        assert_eq!(visible(&plain), visible(&warm), "{top}");
        let read = |n: &str| std::fs::read(e.tmp.join(n)).expect("wave");
        assert_eq!(read("plain.fst"), read("cold.fst"), "{top}");
        assert_eq!(read("plain.fst"), read("warm.fst"), "{top}");
        let nc = visible(&e.run_top(top, &["--threads", "4", "--no-cache"]));
        let w = e.run_top(top, &["--threads", "4"]);
        assert!(note(&w, "hit ir"), "{top}: {w:?}");
        assert_eq!(nc, visible(&w), "{top}");
        // With the optimiser off the stored copies are not even looked up (#491).
        let work = e.work();
        let off = Command::new(env!("CARGO_BIN_EXE_yavs"))
            .current_dir(&e.tmp)
            .env("YAVS_NO_IROPT", "1")
            .env("YAVS_LOG", "debug")
            .args(["--color", "never", "run", "--work", &work, top])
            .output()
            .expect("run");
        assert!(!String::from_utf8_lossy(&off.stderr).contains("hit ir"));
        assert_eq!(visible(&plain).stdout, String::from_utf8_lossy(&off.stdout));
        for f in e.entry_files("ir") {
            std::fs::remove_file(f).expect("remove");
        }
    }
}

/// A run that calls more functions than the ones the entry knows writes the entry again from the
/// program it read (whose function bodies are decoded on first use, #481); the next run reads
/// that entry and behaves as the run without the cache.
#[test]
fn an_entry_that_learns_is_written_again_and_read_back() {
    // (`f` stays a call: its assertion would make it a cold block that inlines.)
    let e = Env::with_env("irlearn", &[("YAVS_NO_COLD", "1")]);
    let plain = e.run(&["--no-cache"]);
    assert!(plain.stdout.contains("f=21"), "{plain:?}");
    // Stops before the process calls `f`.
    let short = ["--stop-time=500ps"];
    let _ = e.cold("tb", &short);
    let before = e.entry_files("ir");
    assert_eq!(before.len(), 1);
    let size = |p: &PathBuf| std::fs::metadata(p).expect("entry").len();
    let small = size(&before[0]);
    let grown = e.run(&[]);
    assert!(
        note(&grown, "hit ir") && note(&grown, "store ir"),
        "{grown:?}"
    );
    assert_eq!(visible(&plain), visible(&grown));
    assert!(size(&before[0]) > small, "the entry holds the call of `f`");
    for _ in 0..2 {
        let warm = e.run(&[]);
        assert!(
            note(&warm, "hit ir") && !note(&warm, "store ir"),
            "{warm:?}"
        );
        assert_eq!(visible(&plain), visible(&warm));
    }
}

/// Editing a package whose function only the architecture reaches replaces the lowered design,
/// and a damaged entry is a miss that is stored again.
#[test]
fn editing_a_subprogram_replaces_the_lowered_design() {
    let e = Env::new("iredit");
    assert!(e.cold("tb", &[]).stdout.contains("f=21"));
    assert!(note(&e.run(&[]), "hit ir"));
    std::fs::write(e.tmp.join("pkg.vhd"), PKG.replace("x * K", "x * K + 1")).expect("write");
    assert_eq!(e.yavs(&["analyze", "--work", &e.work(), "pkg.vhd"]).code, 0);
    let after = e.cold("tb", &[]);
    assert!(!note(&after, "hit ir"), "{after:?}");
    assert!(after.stdout.contains("f=22"), "{after:?}");
    let warm = e.run(&[]);
    assert!(
        note(&warm, "hit ir") && warm.stdout.contains("f=22"),
        "{warm:?}"
    );
    // The entry of the edited design is the newest one.
    let img = e
        .entry_files("ir")
        .into_iter()
        .max_by_key(|p| std::fs::metadata(p).and_then(|m| m.modified()).ok())
        .expect("ir entry");
    let good = std::fs::read(&img).expect("read");
    for bad in [good[..good.len() / 2].to_vec(), b"not an entry".to_vec()] {
        std::fs::write(&img, bad).expect("write");
        let r = e.run(&[]);
        assert!(!note(&r, "hit ir"), "{r:?}");
        assert_eq!(visible(&after), visible(&r));
        assert!(note(&r, "store ir"), "the entry is rewritten");
        assert!(note(&e.run(&[]), "hit ir"));
    }
}

const LATE_PKG: &str = "package cp is\n  component leaf is end component;\nend;\n";
const LATE_LEAF: &str = "entity leaf is end;\narchitecture a of leaf is begin\n  process begin report \"leaf VERSION\"; wait; end process;\nend;\n";
const LATE_TB: &str =
    "use work.cp.all;\nentity tbl is end;\narchitecture a of tbl is begin\n  u : leaf;\nend;\n";

/// A component bound by default to an entity outside the analysed closure (analysed after its
/// user) is loaded while the design elaborates: the stage entries are still made and read (#671), and editing that entity
/// replaces them.
#[test]
fn units_loaded_late_do_not_block_the_stage_entries() {
    let e = Env::new("irlate");
    let leaf = |v: &str| {
        std::fs::write(e.tmp.join("leaf.vhd"), LATE_LEAF.replace("VERSION", v)).expect("write");
        assert_eq!(
            e.yavs(&["analyze", "--work", &e.work(), "leaf.vhd"]).code,
            0
        );
    };
    std::fs::write(e.tmp.join("cp.vhd"), LATE_PKG).expect("write");
    std::fs::write(e.tmp.join("tbl.vhd"), LATE_TB).expect("write");
    for f in ["cp.vhd", "tbl.vhd"] {
        assert_eq!(e.yavs(&["analyze", "--work", &e.work(), f]).code, 0, "{f}");
    }
    // Analysed after its user: the closure of `tbl` does not name it.
    leaf("one");
    let plain = e.run_top("tbl", &["--no-cache"]);
    assert!(plain.stdout.contains("leaf one"), "{plain:?}");
    let cold = e.cold("tbl", &[]);
    assert!(note(&cold, "store ir"), "{cold:?}");
    let warm = e.run_top("tbl", &[]);
    assert!(note(&warm, "hit ir"), "{warm:?}");
    assert_eq!(visible(&plain), visible(&warm));
    leaf("two");
    for _ in 0..3 {
        let r = e.run_top("tbl", &[]);
        assert!(r.stdout.contains("leaf two"), "{r:?}");
    }
    assert!(note(&e.run_top("tbl", &[]), "hit ir"));
}

/// A generic picks the entity a generate instantiates: the architectures lowered differ, so
/// do the entries, and no run reads the other's.
#[test]
fn lowered_design_depends_on_what_elaboration_asked_for() {
    let e = Env::new("irpick");
    std::fs::write(e.tmp.join("pick.vhd"), PICK).expect("write");
    assert_eq!(
        e.yavs(&["analyze", "--work", &e.work(), "pick.vhd"]).code,
        0
    );
    let run = |g: &str| e.run_top("pick", &[&format!("-gg={g}")]);
    for _ in 0..3 {
        for (g, want) in [("true", "from ea"), ("false", "from eb")] {
            let r = run(g);
            assert!(
                r.stdout.contains(want) || r.stderr.contains(want),
                "{g}: {r:?}"
            );
        }
    }
    assert!(note(&run("true"), "hit ir"));
    assert!(note(&run("false"), "hit ir"));
    assert_eq!(e.entry_files("ir").len(), 2);
}

/// A unit whose objects the designs below read through external names.
const EXT_SUB: &str = r#"entity sub is end;
architecture a of sub is
  signal s : bit := '0';
  signal n : integer := 0;
  signal v : bit_vector(3 downto 0) := "0000";
  constant c : integer := 7;
begin
  s <= '1' after 1 ns;
  v <= "1010" after 1 ns;
  n <= 3 after 2 ns;
end;
"#;

/// Designs whose generate branches, chosen by generic `V`, make elaboration add to the analysis
/// in another order (implicit aliases of external names, instances of a generic entity), and
/// what each run must print. #1142: the third run of 1, 2, 1 read the lowering the second
/// stored, with ids of the other order: `E3006 no signal 4294967295`, or a wrong value.
const ELAB_ORDER: &[(&str, &str, &[&str], &[&str])] = &[
    // The reproducer of the issue: an alias of an external name in a block, read by processes
    // of both branches.
    (
        "alias",
        r#"entity top is generic (V : natural := 1); end;
architecture a of top is
begin
  u : entity work.sub;
  b : block
    alias s is <<signal .top.u.s : bit>>;
  begin
    g1 : if V = 1 generate
      p : process begin wait on s; report "ok-run V=1 s=" & bit'image(s); wait; end process;
    end generate;
    g2 : if V = 2 generate
      p : process begin wait until s = '1'; report "ok-run V=2"; wait; end process;
    end generate;
  end block;
end;
"#,
        &["ok-run V=1 s='1'"],
        &["ok-run V=2"],
    ),
    // External names used directly: a constant (the stored lowering read the wrong one, a
    // wrong value without an error), a signal in a concurrent statement and in `process (all)`.
    (
        "direct",
        r#"entity top is generic (V : natural := 1); end;
architecture a of top is
  signal o : bit;
begin
  u : entity work.sub;
  g1 : if V = 1 generate
    p : process begin wait for 1 ns; report "V=1 c=" & integer'image(<<constant .top.u.c : integer>> + 1); wait; end process;
    o <= <<signal .top.u.s : bit>>;
  end generate;
  g2 : if V = 2 generate
    p : process (all) begin report "V=2 n=" & integer'image(<<signal .top.u.n : integer>>); end process;
    q : process begin wait for 1 ns; report "V=2 c=" & integer'image(<<constant .top.u.c : integer>>); wait; end process;
  end generate;
  r : process begin wait for 3 ns; report "V=" & integer'image(V) & " o=" & bit'image(o); wait; end process;
end;
"#,
        &["V=1 c=8", "V=1 o='1'"],
        &["V=2 n=0", "V=2 n=3", "V=2 c=7", "V=2 o='0'"],
    ),
    // Aliases of a part of an external name, in the branch the generic selects and in the
    // other one (#1143): the unselected branch's aliases are made after the selected one's.
    (
        "part",
        r#"entity top is generic (V : natural := 1); end;
architecture a of top is
begin
  u : entity work.sub;
  g1 : if V = 1 generate
    alias w is <<signal .top.u.v : bit_vector(3 downto 0)>>(1);
  begin
    p : process begin wait for 2 ns; report "V=1 w=" & bit'image(w); wait; end process;
  end generate;
  g2 : if V = 2 generate
    alias w is <<signal .top.u.v : bit_vector(3 downto 0)>>(3 downto 2);
    alias k is <<constant .top.u.c : integer>>;
  begin
    p : process begin wait for 2 ns; report "V=2 w=" & to_string(w) & " k=" & integer'image(k); wait; end process;
  end generate;
end;
"#,
        &["V=1 w='1'"],
        &["V=2 w=10 k=7"],
    ),
    // An external constant: 1, 2, 1 printed `c=1` here, a wrong value without an error.
    (
        "constant",
        r#"entity top is generic (V : natural := 1); end;
architecture a of top is
begin
  u : entity work.sub;
  g1 : if V = 1 generate
    p : process begin wait for 1 ns; report "V=1 c=" & integer'image(<<constant .top.u.c : integer>> + 1); wait; end process;
  end generate;
  g2 : if V = 2 generate
    p : process begin wait for 1 ns; report "V=2 c=" & integer'image(<<constant .top.u.c : integer>>) & " s=" & bit'image(<<signal .top.u.s : bit>>); wait; end process;
  end generate;
end;
"#,
        &["V=1 c=8"],
        &["V=2 c=7 s='1'"],
    ),
    // A function of a package that reads the hierarchy, called from both branches.
    (
        "package",
        r#"package pk is
  impure function rd return integer;
end;
package body pk is
  impure function rd return integer is begin return <<signal .top.u.n : integer>>; end;
end;
use work.pk.all;
entity top is generic (V : natural := 1); end;
architecture a of top is
begin
  u : entity work.sub;
  g1 : if V = 1 generate
    p : process begin wait for 3 ns; report "V=1 rd=" & integer'image(rd) & " s=" & bit'image(<<signal .top.u.s : bit>>); wait; end process;
  end generate;
  g2 : if V = 2 generate
    p : process begin wait for 3 ns; report "V=2 rd=" & integer'image(rd + <<signal .top.u.n : integer>>); wait; end process;
  end generate;
end;
"#,
        &["V=1 rd=3 s='1'"],
        &["V=2 rd=6"],
    ),
    // No external name: instances of an entity with a generic type, made in another order.
    (
        "gtype",
        r#"package im is
  function img (x : integer) return string;
  function img (x : boolean) return string;
end;
package body im is
  function img (x : integer) return string is begin return integer'image(x); end;
  function img (x : boolean) return string is begin return boolean'image(x); end;
end;
entity gt is generic (type t; v : t; tag : string; function img (x : t) return string is <>); end;
architecture a of gt is
  signal q : t := v;
begin
  process begin wait for 1 ns; report tag & " " & img(q); wait; end process;
end;
use work.im.all;
entity top is generic (V : natural := 1); end;
use work.im.all;
architecture a of top is
begin
  g1 : if V = 1 generate
    a1 : entity work.gt generic map (t => integer, v => 4, tag => "int");
    b1 : entity work.gt generic map (t => boolean, v => true, tag => "bool");
  end generate;
  g2 : if V = 2 generate
    b2 : entity work.gt generic map (t => boolean, v => false, tag => "bool");
    a2 : entity work.gt generic map (t => integer, v => 5, tag => "int");
  end generate;
end;
"#,
        &["int 4", "bool true"],
        &["int 5", "bool false"],
    ),
];

/// The stage entries of a design hold the ids elaboration gave what it added: runs whose generics
/// make it add in another order keep entries of their own, and every run prints what a run
/// without the cache prints (#1142).
#[test]
fn lowered_design_depends_on_what_elaboration_added() {
    let mut bad = Vec::new();
    for &(name, top, one, two) in ELAB_ORDER {
        for order in [["1", "2", "1", "2", "1"], ["2", "1", "2", "1", "2"]] {
            let e = Env::new(&format!("elaborder-{name}-{}", order[0]));
            std::fs::write(e.tmp.join("sub.vhd"), EXT_SUB).expect("write");
            std::fs::write(e.tmp.join("top.vhd"), top).expect("write");
            for f in ["sub.vhd", "top.vhd"] {
                assert_eq!(e.yavs(&["analyze", "--work", &e.work(), f]).code, 0, "{f}");
            }
            let mut hit = false;
            for (k, v) in order.iter().enumerate() {
                let r = e.run_top("top", &[&format!("-gV={v}")]);
                let want = if *v == "1" { one } else { two };
                let printed = |w: &&str| r.stdout.contains(w) || r.stderr.contains(w);
                if r.code != 0 || !want.iter().all(printed) {
                    bad.push(format!(
                        "{name} {order:?} run {k} V={v}: {}{}",
                        r.stdout,
                        visible(&r).stderr
                    ));
                    break;
                }
                hit = note(&r, "hit ir");
            }
            // The cache is still used: the last run read a stored lowering.
            if !hit
                && !bad
                    .iter()
                    .any(|b| b.starts_with(&format!("{name} {order:?}")))
            {
                bad.push(format!("{name} {order:?}: the last run read no lowering"));
            }
        }
    }
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}

/// Entries that hold generated code are keyed by the optimiser and tier switches and by the
/// build (#491): another setting or another build is a miss with the right output, the same one
/// a hit.
#[test]
fn lowered_design_depends_on_the_optimiser_and_the_build() {
    let e = Env::new("irswitch");
    let plain = e.run(&["--no-cache"]);
    assert!(plain.stdout.contains("f=21"), "{plain:?}");
    let _ = e.cold("tb", &[]);
    let with = |env: &[(&str, &str)]| {
        let work = e.work();
        let mut c = Command::new(env!("CARGO_BIN_EXE_yavs"));
        c.current_dir(&e.tmp)
            .env_remove("YAVS_STDLIB")
            .env("YAVS_LOG", "debug")
            .args(["--color", "never", "run", "--work", &work])
            .arg("tb");
        for (k, v) in env {
            c.env(k, v);
        }
        let o = c.output().expect("run");
        Out {
            code: o.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        }
    };
    assert!(note(&with(&[]), "hit ir"), "the same settings hit");
    let cases = [
        ("YAVS_NO_CFG", "1"),
        ("YAVS_NO_MEMO", "1"),
        ("YAVS_NO_INLINE", "1"),
        ("YAVS_NO_COLD", "1"),
        ("YAVS_NO_IROPT", "1"),
        ("YAVS_CACHE_SALT", "another build"),
    ];
    for env in cases {
        let env = &[env];
        let first = with(env);
        assert!(!note(&first, "hit ir"), "{env:?}: {first:?}");
        assert_eq!(visible(&plain).stdout, first.stdout, "{env:?}");
        let second = with(env);
        let third = with(env);
        assert!(
            note(&second, "store ir") || note(&third, "hit ir"),
            "{env:?}: {second:?}"
        );
        assert!(note(&third, "hit ir"), "{env:?}: {third:?}");
        assert_eq!(visible(&plain).stdout, third.stdout, "{env:?}");
    }
    // The settings made first still hit: nothing was replaced.
    assert!(note(&with(&[]), "hit ir"));
}

/// A package instance nested in a generic package, whose generic map has two entries of each
/// kind: cloning the nested instance with the outer one numbers the clones in the map's order.
const NESTED: &str = "package inner_g is\n\
  generic (type t1; type t2; c1 : integer; c2 : integer; function cv (x : t1) return t2);\n\
  constant k : integer := c1 + c2;\n  function g (x : t1) return t2;\nend;\n\
package body inner_g is\n  function g (x : t1) return t2 is begin return cv(x); end;\nend;\n\
package outer_g is\n  generic (type a; type b; function f (x : a) return b);\n\
  package i is new work.inner_g generic map (t1 => a, t2 => b, c1 => 3, c2 => 4, cv => f);\nend;\n\
entity tb is end;\n\
architecture a of tb is\n  function f (x : integer) return bit is begin return '1'; end;\n\
  package o is new work.outer_g generic map (a => integer, b => bit, f => f);\nbegin\n\
  process begin report \"k=\" & integer'image(o.i.k) & \" g=\" & bit'image(o.i.g(1)); wait; end process;\nend;\n";

/// A closure with ieee packages too: more units in the store, and a body that completes a package.
const WITH_IEEE: &str = "library ieee;\nuse ieee.std_logic_1164.all;\nuse ieee.numeric_std.all;\n\
package p is\n  function inc (x : unsigned) return unsigned;\nend;\n\
package body p is\n  function inc (x : unsigned) return unsigned is begin return x + 1; end;\nend;\n\
library ieee;\nuse ieee.numeric_std.all;\nuse work.p.all;\nentity tb is end;\n\
architecture a of tb is\nbegin\n\
  process begin report \"k=\" & integer'image(to_integer(inc(to_unsigned(6, 8)))); wait; end process;\nend;\n";

/// A closure whose run-time packages are lowered into an entry of their own (`low`).
const WITH_LOW: &str = "library ieee;\nuse ieee.float_pkg.all;\nentity tb is end;\n\
architecture a of tb is\n  signal s : float32 := to_float(7.0);\nbegin\n\
  process begin report \"k=\" & integer'image(to_integer(s)); wait; end process;\nend;\n";

/// The entries do not depend on where the units came from: a shared std unit replayed
/// from an entry made after another design, or analysed from source, numbers its symbols
/// differently, and the hash maps of an image must not show that (#964).
#[test]
fn entries_do_not_depend_on_who_made_the_std_units() {
    for (name, source) in [("nested", NESTED), ("ieee", WITH_IEEE), ("low", WITH_LOW)] {
        // One directory, emptied between runs: the paths are part of the keys and the images.
        let tmp = tempdir(&format!("whostd_{name}"));
        let std_cache = tmp.join("std");
        let yavs = |args: &[&str]| {
            let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
                .current_dir(&tmp)
                .env_remove("YAVS_STDLIB")
                .env("YAVS_STD_CACHE", &std_cache)
                .args(args)
                .output()
                .expect("run");
            assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
            String::from_utf8_lossy(&o.stdout).into_owned()
        };
        let image = |before_image: &dyn Fn()| {
            let _ = std::fs::remove_dir_all(tmp.join("work"));
            std::fs::write(tmp.join("n.vhd"), source).expect("write");
            let work = format!("work:{}", tmp.join("work").display());
            yavs(&["analyze", "--work", &work, "n.vhd"]);
            assert!(yavs(&["run", "--work", &work, "tb"]).contains("k=7"));
            before_image();
            assert!(yavs(&["run", "--work", &work, "tb"]).contains("k=7"));
            let mut files: Vec<(String, Vec<u8>)> =
                std::fs::read_dir(tmp.join("work").join("yavs-cache"))
                    .expect("cache dir")
                    .flatten()
                    .filter(|f| f.file_name().to_string_lossy().ends_with(".img"))
                    .map(|f| {
                        let bytes = std::fs::read(f.path()).expect("read");
                        (f.file_name().to_string_lossy().into_owned(), bytes)
                    })
                    .collect();
            files.sort();
            assert!(!files.is_empty());
            files
        };
        let replayed = image(&|| {});
        // Hash maps seeded per process would show here without any change of the store (#973).
        let again = image(&|| {});
        if name == "low" {
            assert!(
                replayed.iter().any(|f| f.0.starts_with("low-")),
                "no low entry"
            );
        }
        let from_source = image(&|| std::fs::remove_dir_all(&std_cache).expect("clear the store"));
        // The store made by an analysis that met other names first.
        let by_another = image(&|| {
            std::fs::write(
                tmp.join("o.vhd"),
                "entity zz is end;\narchitecture a of zz is\n  type t is ('x', 'q', '?', zz_lit);\nbegin\nend;\n",
            )
            .expect("write");
            std::fs::remove_dir_all(&std_cache).expect("clear the store");
            let other = format!("other:{}", tmp.join("other").display());
            yavs(&["analyze", "--work", &other, "o.vhd"]);
        });
        for (how, files) in [
            ("again", again),
            ("from source", from_source),
            ("by another", by_another),
        ] {
            assert_eq!(files.len(), replayed.len(), "{name} {how}");
            for (a, b) in replayed.iter().zip(&files) {
                assert_eq!(a.0, b.0, "{name} {how}");
                assert!(a.1 == b.1, "{name} {how}: {} differs", a.0);
            }
        }
    }
}

/// The cache entries do not depend on the process: std hash maps are seeded per process, so
/// an id or a byte that follows their order differs between runs (#557).
#[test]
fn entries_are_the_same_in_every_process() {
    let mut first: Option<Vec<(String, Vec<u8>)>> = None;
    // One directory, emptied between runs: the paths are part of the keys and the images.
    let tmp = tempdir("bytes");
    for n in 0..6 {
        let _ = std::fs::remove_dir_all(tmp.join("work"));
        std::fs::write(tmp.join("n.vhd"), NESTED).expect("write");
        let work = format!("work:{}", tmp.join("work").display());
        let run = |args: &[&str]| {
            let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
                .current_dir(&tmp)
                .env_remove("YAVS_STDLIB")
                .args(args)
                .output()
                .expect("run");
            assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
            String::from_utf8_lossy(&o.stdout).into_owned()
        };
        run(&["analyze", "--work", &work, "n.vhd"]);
        // The first run only notes the closure, the second stores the entries.
        for _ in 0..2 {
            assert!(run(&["run", "--work", &work, "tb"]).contains("k=7"));
        }
        let mut files: Vec<(String, Vec<u8>)> =
            std::fs::read_dir(tmp.join("work").join("yavs-cache"))
                .expect("cache dir")
                .flatten()
                .filter(|f| f.file_name().to_string_lossy().ends_with(".img"))
                .map(|f| {
                    let bytes = std::fs::read(f.path()).expect("read");
                    (f.file_name().to_string_lossy().into_owned(), bytes)
                })
                .collect();
        files.sort();
        assert!(!files.is_empty());
        match &first {
            None => first = Some(files),
            Some(f0) => {
                let names =
                    |v: &[(String, Vec<u8>)]| v.iter().map(|p| p.0.clone()).collect::<Vec<_>>();
                assert_eq!(names(f0), names(&files), "process {n}");
                for (a, b) in f0.iter().zip(&files) {
                    assert!(a.1 == b.1, "{} differs in process {n}", a.0);
                }
            }
        }
    }
}

// How a file is spelled (#1031). The analysed-unit cache keys on the absolute path and an entry holds
// no spelling of the unit's own file: the replay uses the session's.

const PKG_K: &str = "package p is\n  constant K : integer := {K};\nend;\n";
const TB_K: &str = "use work.p.all;\nentity tb is end;\narchitecture a of tb is\nbegin\n  process\n    variable v : bit_vector(0 to 3);\n  begin\n    if false then v(5) := '1'; end if;\n    report \"K=\" & integer'image(K);\n    wait;\n  end process;\nend;\n";

fn at(dir: &Path, args: &[&str]) -> Out {
    let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .current_dir(dir)
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

fn ok(o: Out) -> Out {
    assert_eq!(o.code, 0, "{o:?}");
    o
}

/// A work library at `lib`, as an option.
fn work(lib: &Path) -> String {
    format!("--work=work:{}", lib.display())
}

fn pkg_k(k: u32) -> String {
    PKG_K.replace("{K}", &k.to_string())
}

fn entries_of(lib: &Path) -> usize {
    std::fs::read_to_string(lib.join("yavs-lib.toml"))
        .expect("index")
        .matches("[[file]]")
        .count()
}

/// The same relative name from two directories, with other content of the same size, never reads
/// the other's unit, in either order and however often: the wrong-result guard.
#[test]
fn a_relative_name_in_another_directory_is_another_file() {
    let t = tempdir("spell-dirs");
    let lib = t.join("lib");
    let w = work(&lib);
    for (d, k) in [("d1", 1), ("d2", 2)] {
        std::fs::create_dir(t.join(d)).unwrap();
        std::fs::write(t.join(d).join("x.vhd"), pkg_k(k)).unwrap();
        std::fs::write(t.join(d).join("tb.vhd"), TB_K).unwrap();
    }
    for (d, k) in [("d1", 1), ("d2", 2), ("d1", 1), ("d2", 2), ("d2", 2)] {
        let dir = t.join(d);
        ok(at(&dir, &["analyze", "--std=2008", &w, "x.vhd", "tb.vhd"]));
        for _ in 0..3 {
            let o = ok(at(&dir, &["run", &w, "tb"]));
            assert!(o.stdout.contains(&format!("K={k}")), "{d}: {o:?}");
        }
        let cold = ok(at(&dir, &["run", "--no-cache", &w, "tb"]));
        assert!(cold.stdout.contains(&format!("K={k}")), "{d}: {cold:?}");
    }
}

/// Files with the same text in two directories keep their own place in a diagnostic.
#[test]
fn the_same_text_in_another_directory_keeps_its_own_path() {
    let t = tempdir("spell-same");
    let lib = t.join("lib");
    let w = work(&lib);
    for d in ["d1", "d2"] {
        std::fs::create_dir(t.join(d)).unwrap();
        std::fs::write(t.join(d).join("x.vhd"), pkg_k(1)).unwrap();
        std::fs::write(t.join(d).join("tb.vhd"), TB_K).unwrap();
    }
    for (d, other) in [("d1", "d2"), ("d2", "d1"), ("d1", "d2")] {
        let dir = t.join(d);
        let o = ok(at(&dir, &["analyze", "--std=2008", &w, "x.vhd", "tb.vhd"]));
        assert!(o.stderr.contains(" --> tb.vhd:8:"), "{d}: {o:?}");
        let o = ok(at(&dir, &["elab", &w, "tb"]));
        let here = format!(" --> {}:8:", dir.join("tb.vhd").display());
        assert!(o.stderr.contains(&here), "{d}: {o:?}");
        assert!(
            !o.stderr.contains(&t.join(other).display().to_string()),
            "{d}: {o:?}"
        );
    }
}

/// `elab` and `run` read what `analyze` kept under any spelling, since the manifest records the
/// absolute form of it, and the result is the one without the cache.
#[test]
fn every_spelling_of_a_file_hits_in_elab() {
    let t = tempdir("spell-hit");
    std::fs::create_dir(t.join("sub")).unwrap();
    std::fs::write(t.join("x.vhd"), pkg_k(3)).unwrap();
    std::fs::write(t.join("tb.vhd"), TB_K).unwrap();
    let abs = t.join("x.vhd").display().to_string();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&*t, t.join("link")).unwrap();
    let spellings = [
        "x.vhd",
        "./x.vhd",
        "sub/../x.vhd",
        abs.as_str(),
        #[cfg(unix)]
        "link/x.vhd",
    ];
    for (i, s) in spellings.iter().enumerate() {
        let lib = t.join(format!("lib{i}"));
        let w = work(&lib);
        ok(at(&t, &["analyze", "--std=2008", &w, s, "tb.vhd"]));
        let o = ok(at(&t, &["elab", &w, "tb"]));
        assert!(
            o.stderr.contains("debug: units: hit work.p ")
                && o.stderr.contains("debug: units: hit work.tb/a ")
                && !o.stderr.contains("debug: units: miss work."),
            "{s}: {}",
            o.stderr
        );
        let run = ok(at(&t, &["run", &w, "tb"]));
        let cold = ok(at(&t, &["run", "--no-cache", &w, "tb"]));
        assert_eq!(run.stdout, cold.stdout, "{s}");
        assert!(run.stdout.contains("K=3"), "{s}: {run:?}");
    }
}

/// `link/../x.vhd` with `link` a symlink to another directory is that directory's parent's file: a
/// key that resolved `..` by the text would read `x.vhd` of this directory instead.
#[cfg(unix)]
#[test]
fn dots_after_a_symlink_are_the_file_the_system_opens() {
    let t = tempdir("spell-dotlink");
    let lib = t.join("lib");
    let w = work(&lib);
    std::fs::create_dir_all(t.join("far/deep")).unwrap();
    std::os::unix::fs::symlink(t.join("far/deep"), t.join("link")).unwrap();
    std::fs::write(t.join("x.vhd"), pkg_k(1)).unwrap();
    std::fs::write(t.join("far/x.vhd"), pkg_k(2)).unwrap();
    std::fs::write(t.join("tb.vhd"), TB_K).unwrap();
    for (s, k) in [
        ("x.vhd", 1),
        ("link/../x.vhd", 2),
        ("x.vhd", 1),
        ("link/../x.vhd", 2),
    ] {
        ok(at(&t, &["analyze", "--std=2008", &w, s, "tb.vhd"]));
        for _ in 0..2 {
            let o = ok(at(&t, &["run", &w, "tb"]));
            assert!(o.stdout.contains(&format!("K={k}")), "{s}: {o:?}");
        }
    }
}

/// An entry made under one spelling and read under another shows the session's spelling, in a
/// warning kept in the entry and in a label into a file only read for its text when it is shown.
#[test]
fn diagnostics_show_the_spelling_of_the_session() {
    let t = tempdir("spell-diag");
    let lib = t.join("lib");
    let w = work(&lib);
    std::fs::create_dir(t.join("sub")).unwrap();
    std::fs::write(
        t.join("a.vhd"),
        "package a is\n  constant K : integer := 7;\nend;\n",
    )
    .unwrap();
    std::fs::write(
        t.join("c.vhd"),
        "package c is\n  constant K : integer := 9;\nend;\n",
    )
    .unwrap();
    std::fs::write(t.join("tb.vhd"), TB_K.replace("work.p", "work.a")).unwrap();
    let bad = "use work.a.all;\nuse work.c.all;\nentity u is end;\narchitecture r of u is\n  constant Z : integer := K;\nbegin end;\n";
    std::fs::write(t.join("u.vhd"), bad).unwrap();
    // Made under the absolute names, and the sources old enough to be trusted without a read.
    let name = |f: &str| t.join(f).display().to_string();
    ok(at(
        &t,
        &[
            "analyze",
            "--std=2008",
            &w,
            &name("a.vhd"),
            &name("c.vhd"),
            &name("tb.vhd"),
        ],
    ));
    for f in ["a.vhd", "c.vhd", "tb.vhd"] {
        age(&t.join(f));
    }
    ok(at(
        &t,
        &[
            "analyze",
            "--std=2008",
            &w,
            &name("a.vhd"),
            &name("c.vhd"),
            &name("tb.vhd"),
        ],
    ));

    let sub = t.join("sub");
    for (dir, pre) in [(&*t, ""), (&*t, "./"), (&*t, "sub/../"), (&*sub, "../")] {
        let f = |n: &str| format!("{pre}{n}");
        let o = ok(at(
            dir,
            &["analyze", "--std=2008", &w, &f("a.vhd"), &f("tb.vhd")],
        ));
        assert!(
            o.stderr.contains(&format!(" --> {}:8:", f("tb.vhd"))),
            "{pre}: {o:?}"
        );
        assert!(o.stderr.contains("v(5) := '1'"), "{pre}: {o:?}");
        assert!(!o.stderr.contains(&name("tb.vhd")), "{pre}: {o:?}");

        let o = at(
            dir,
            &[
                "analyze",
                "--std=2008",
                &w,
                &f("a.vhd"),
                &f("c.vhd"),
                &f("u.vhd"),
            ],
        );
        assert_eq!(o.code, 1, "{pre}: {o:?}");
        assert!(
            o.stderr.contains(&format!("::: {}:2:12", f("a.vhd"))),
            "{pre}: {o:?}"
        );
        assert!(
            o.stderr.contains(&format!("::: {}:2:12", f("c.vhd"))),
            "{pre}: {o:?}"
        );
        assert!(
            o.stderr.contains("constant K : integer := 7;"),
            "{pre}: {o:?}"
        );
        assert!(
            o.stderr.contains("constant K : integer := 9;"),
            "{pre}: {o:?}"
        );
    }
}

/// What is unreadable is named as the session knows it: the manifest's path for a unit loaded on
/// demand, the spelling on the command line for a named file, never another spelling of it.
#[test]
fn cannot_read_names_the_path_of_the_session() {
    let t = tempdir("spell-read");
    let lib = t.join("lib");
    let w = work(&lib);
    std::fs::write(t.join("x.vhd"), pkg_k(1)).unwrap();
    std::fs::write(t.join("tb.vhd"), TB_K).unwrap();
    ok(at(&t, &["analyze", "--std=2008", &w, "./x.vhd", "tb.vhd"]));
    std::fs::remove_file(t.join("x.vhd")).unwrap();
    let o = at(&t, &["analyze", "--std=2008", &w, "tb.vhd"]);
    assert_eq!(o.code, 1, "{o:?}");
    let want = format!("cannot read source file `{}`", t.join("x.vhd").display());
    assert!(o.stderr.contains(&want), "{o:?}");
    let o = at(&t, &["analyze", "--std=2008", &w, "./x.vhd"]);
    assert_eq!(o.code, 1, "{o:?}");
    assert!(o.stderr.contains("./x.vhd"), "{o:?}");
    assert!(
        !o.stderr.contains(&t.join("x.vhd").display().to_string()),
        "{o:?}"
    );
}

/// A file analyzed under a second spelling replaces its entry: a unit dropped from it is unknown
/// (`L0031`), not a stale file (`L0034`).
#[test]
fn a_file_has_one_entry_however_spelled() {
    const TWO: &str = "package a is\n  constant K : integer := 7;\nend;\npackage b is\n  constant L : integer := 8;\nend;\n";
    const ONE: &str = "package a is\n  constant K : integer := 7;\nend;\n";
    let t = tempdir("spell-one");
    std::fs::create_dir(t.join("sub")).unwrap();
    std::fs::write(
        t.join("t.vhd"),
        "use work.a.all;\nentity t is end;\narchitecture r of t is begin end;\n",
    )
    .unwrap();
    std::fs::write(t.join("ub.vhd"), "use work.b.all;\nentity ub is end;\n").unwrap();
    #[cfg(unix)]
    std::os::unix::fs::symlink(&*t, t.join("link")).unwrap();
    let spellings = [
        "sub/../x.vhd",
        "./x.vhd",
        #[cfg(unix)]
        "link/x.vhd",
    ];
    for (i, s) in spellings.iter().enumerate() {
        let lib = t.join(format!("lib{i}"));
        let w = work(&lib);
        std::fs::write(t.join("x.vhd"), TWO).unwrap();
        ok(at(
            &t,
            &["analyze", "--std=2008", &w, "x.vhd", "t.vhd", "ub.vhd"],
        ));
        assert_eq!(entries_of(&lib), 3, "{s}");
        std::fs::write(t.join("x.vhd"), ONE).unwrap();
        ok(at(&t, &["analyze", "--std=2008", &w, s]));
        assert_eq!(entries_of(&lib), 3, "{s}");
        let o = at(&t, &["elab", &w, "ub"]);
        assert_ne!(o.code, 0, "{s}: {o:?}");
        assert!(
            o.stderr.contains("L0031") && !o.stderr.contains("L0034"),
            "{s}: {o:?}"
        );
        ok(at(&t, &["elab", &w, "t"]));
    }
    // Two names of one file in one call, and a symlink of another name.
    let lib = t.join("libboth");
    let w = work(&lib);
    ok(at(
        &t,
        &["analyze", "--std=2008", &w, "x.vhd", "sub/../x.vhd"],
    ));
    assert_eq!(entries_of(&lib), 1);
    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(t.join("x.vhd"), t.join("alias.vhd")).unwrap();
        ok(at(&t, &["analyze", "--std=2008", &w, "alias.vhd"]));
        assert_eq!(entries_of(&lib), 1);
        ok(at(&t, &["analyze", "--std=2008", &w, "x.vhd"]));
        assert_eq!(entries_of(&lib), 1);
    }
    // Different files with one name stay two entries.
    std::fs::write(t.join("sub/x.vhd"), "package other is\nend;\n").unwrap();
    ok(at(&t, &["analyze", "--std=2008", &w, "sub/x.vhd"]));
    assert_eq!(entries_of(&lib), 2);
}

/// #1044: an edited file the index knows by a symlink of another name has one entry, so a unit
/// dropped from it is `L0031`, not `L0034` for the symlink's entry.
#[cfg(unix)]
#[test]
fn an_edited_file_known_by_a_symlink_of_another_name_has_one_entry() {
    let t = tempdir("spell-alias");
    std::fs::write(t.join("x.vhd"), "package a is\nend;\npackage b is\nend;\n").unwrap();
    std::fs::write(t.join("ub.vhd"), "use work.b.all;\nentity ub is end;\n").unwrap();
    std::os::unix::fs::symlink(t.join("x.vhd"), t.join("alias.vhd")).unwrap();
    let lib = t.join("lib");
    let w = work(&lib);
    ok(at(
        &t,
        &["analyze", "--std=2008", &w, "alias.vhd", "ub.vhd"],
    ));
    assert_eq!(entries_of(&lib), 2);
    std::fs::write(t.join("x.vhd"), "package a is\nend;\n").unwrap();
    ok(at(&t, &["analyze", "--std=2008", &w, "x.vhd"]));
    assert_eq!(entries_of(&lib), 2);
    let o = at(&t, &["elab", &w, "ub"]);
    assert_ne!(o.code, 0, "{o:?}");
    assert!(
        o.stderr.contains("L0031") && !o.stderr.contains("L0034"),
        "{o:?}"
    );
}
