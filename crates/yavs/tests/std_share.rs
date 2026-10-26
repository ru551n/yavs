//! The std/ieee units shared by all libraries (`docs/decisions.md` "Shared std/ieee units", #564):
//! a fresh library reads them from the shared store, concurrent cold analyses agree, a damaged
//! entry is rebuilt, an unusable store is an error, and a user's own `ieee` is never shared.

mod common;
use common::{Tmp, tempdir};

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Output, Stdio};

const TB: &str = "library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;\n\
entity tb is end;\narchitecture a of tb is\nbegin\n  process\n    variable v : unsigned(3 downto 0) := x\"5\";\n  begin\n\
    report \"v=\" & integer'image(to_integer(v + 1));\n    wait;\n  end process;\nend;\n";

/// One scratch directory with a testbench file `tb.vhd` and the shared store `shared`.
struct Env {
    tmp: Tmp,
}

fn text(o: &Output) -> (i32, String, String) {
    let err = String::from_utf8_lossy(&o.stderr)
        .lines()
        .filter(|l| !l.starts_with("debug: "))
        .map(|l| format!("{l}\n"))
        .collect();
    (
        o.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&o.stdout).into_owned(),
        err,
    )
}

fn stored(o: &Output) -> usize {
    String::from_utf8_lossy(&o.stderr)
        .lines()
        .filter(|l| l.starts_with("debug: units: store "))
        .count()
}

impl Env {
    fn new(name: &str) -> Env {
        let tmp = tempdir(name);
        std::fs::write(tmp.join("tb.vhd"), TB).expect("write");
        Env { tmp }
    }

    fn shared(&self) -> PathBuf {
        self.tmp.join("shared")
    }

    /// The entries of the shared store.
    fn entries(&self) -> Vec<PathBuf> {
        entries_in(&self.shared())
    }

    fn command(&self, shared: &str, dir: &Path, args: &[&str]) -> Command {
        let mut c = Command::new(env!("CARGO_BIN_EXE_yavs"));
        c.current_dir(dir)
            .env_remove("YAVS_STDLIB")
            .env("YAVS_LOG", "debug")
            .env("YAVS_STD_CACHE", shared)
            .args(["--color", "never"])
            .args(args);
        c
    }

    /// `yavs analyze tb.vhd` into library `lib` of directory `dir` (the working directory).
    fn analyze_in(&self, shared: &str, dir: &Path, lib: &str, more: &[&str]) -> Child {
        let work = format!("{lib}:{}", dir.join(lib).display());
        let mut a = vec!["analyze", "--work", &work];
        a.extend_from_slice(more);
        a.push("tb.vhd");
        self.command(shared, dir, &a)
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .expect("spawn")
    }

    fn analyze(&self, lib: &str, more: &[&str]) -> Output {
        let s = self.shared().display().to_string();
        self.analyze_in(&s, &self.tmp, lib, more)
            .wait_with_output()
            .expect("wait")
    }
}

fn entries_in(shared: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(shared.join("yavs-cache").join("units"))
        .map(|d| {
            d.flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|e| e == "img"))
                .collect()
        })
        .unwrap_or_default()
}

#[test]
fn a_fresh_library_reads_std_and_ieee_from_the_shared_store() {
    let e = Env::new("share_fresh");
    let reference = e.analyze("ref", &["--no-cache"]);
    assert_eq!(reference.status.code(), Some(0));
    let first = e.analyze("a", &[]);
    assert_eq!(text(&first), text(&reference));
    let made = e.entries().len();
    assert!(made >= 3, "the std/ieee units are stored: {made}");
    assert!(stored(&first) >= made + 2, "{:?}", text(&first));
    // Another library: nothing of std/ieee is analysed or stored again, and the library's own
    // directory holds none of it.
    let second = e.analyze("b", &[]);
    assert_eq!(text(&second), text(&reference));
    assert_eq!(
        stored(&second),
        2,
        "only the testbench: {:?}",
        second.stderr
    );
    assert_eq!(e.entries().len(), made);
    let own = entries_in(&e.tmp.join("b")).len();
    assert_eq!(own, 2, "the library keeps its own units only");
    // `run` reads them and writes nothing.
    let work = format!("b:{}", e.tmp.join("b").display());
    let r = e
        .command(
            &e.shared().display().to_string(),
            &e.tmp,
            &["run", "--work", &work, "tb"],
        )
        .output()
        .expect("run");
    assert!(String::from_utf8_lossy(&r.stdout).contains("v=6"), "{r:?}");
    assert_eq!(e.entries().len(), made);
}

#[test]
fn concurrent_cold_analyses_agree() {
    const N: usize = 6;
    let e = Env::new("share_concurrent");
    // Each process has a directory of its own (the diagnostics show the same file name) and all
    // start on the empty shared store at once.
    let dirs: Vec<PathBuf> = (0..N)
        .map(|i| {
            let d = e.tmp.join(format!("p{i}"));
            std::fs::create_dir(&d).expect("dir");
            std::fs::write(d.join("tb.vhd"), TB).expect("write");
            d
        })
        .collect();
    let s = e.tmp.join("fresh").display().to_string();
    let children: Vec<Child> = dirs
        .iter()
        .map(|d| e.analyze_in(&s, d, "work", &[]))
        .collect();
    let outs: Vec<Output> = children
        .into_iter()
        .map(|c| c.wait_with_output().expect("wait"))
        .collect();
    let reference = text(&e.analyze("ref", &["--no-cache"]));
    for o in &outs {
        assert_eq!(
            text(o),
            reference,
            "{:?}",
            String::from_utf8_lossy(&o.stderr)
        );
    }
    // One entry of each unit, all valid: an audited analysis on top of them stores nothing and
    // reads them all.
    let fresh = entries_in(&e.tmp.join("fresh"));
    let sequential = {
        let one = Env::new("share_concurrent_seq");
        one.analyze("a", &[]);
        one.entries().len()
    };
    assert_eq!(fresh.len(), sequential);
    let d = &dirs[0];
    let w2 = format!("w2:{}", d.join("w2").display());
    let mut c = e.command(&s, d, &["analyze", "--work", &w2, "tb.vhd"]);
    let o = c.env("YAVS_UNIT_CHECK", "1").output().expect("run");
    assert_eq!(
        o.status.code(),
        Some(0),
        "{:?}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert_eq!(stored(&o), 2, "{:?}", String::from_utf8_lossy(&o.stderr));
}

#[test]
fn a_damaged_shared_entry_is_rebuilt() {
    let e = Env::new("share_damaged");
    let reference = e.analyze("a", &[]);
    let entries = e.entries();
    for (i, f) in entries.iter().enumerate() {
        let mut b = std::fs::read(f).expect("read");
        if i % 2 == 0 {
            let n = b.len();
            b[n / 2] ^= 0x55;
        } else {
            b.truncate(b.len() / 3);
        }
        std::fs::write(f, b).expect("write");
    }
    let again = e.analyze("b", &[]);
    assert_eq!(text(&again), text(&reference));
    assert!(stored(&again) > 2, "the damaged entries are made again");
    // And now they are whole: nothing is stored a third time.
    let third = e.analyze("c", &[]);
    assert_eq!(text(&third), text(&reference));
    assert_eq!(
        stored(&third),
        2,
        "{:?}",
        String::from_utf8_lossy(&third.stderr)
    );
}

#[test]
fn an_unusable_store_is_an_error_naming_the_directory() {
    let e = Env::new("share_unusable");
    let file = e.tmp.join("not_a_dir");
    std::fs::write(&file, "x").expect("write");
    let below = file.join("store");
    let o = e
        .analyze_in(&below.display().to_string(), &e.tmp, "a", &[])
        .wait_with_output()
        .expect("wait");
    let (code, _, err) = text(&o);
    assert_eq!(code, 2, "{err}");
    assert!(
        err.contains("E0126") && err.contains(&below.display().to_string()),
        "{err}"
    );
    assert!(err.contains("YAVS_STD_CACHE"), "the fix is named: {err}");
    // An empty value is an error too; `off` keeps the units in the library.
    let o = e
        .analyze_in("", &e.tmp, "a", &[])
        .wait_with_output()
        .expect("wait");
    assert_eq!(o.status.code(), Some(2));
    assert!(text(&o).2.contains("E0126"));
    let off = e
        .analyze_in("off", &e.tmp, "a", &[])
        .wait_with_output()
        .expect("wait");
    assert_eq!(off.status.code(), Some(0), "{:?}", text(&off));
    assert!(
        entries_in(&e.tmp.join("a")).len() >= 5,
        "std/ieee are the library's own"
    );
    // A run only reads: a store that cannot be created is a miss.
    let work = format!("a:{}", e.tmp.join("a").display());
    let r = e
        .command(
            &below.display().to_string(),
            &e.tmp,
            &["run", "--work", &work, "tb"],
        )
        .output()
        .expect("run");
    assert!(String::from_utf8_lossy(&r.stdout).contains("v=6"), "{r:?}");
}

#[test]
fn a_users_own_ieee_is_never_shared() {
    let e = Env::new("share_own_ieee");
    let mine = "package std_logic_1164 is\n  constant MINE : integer := 42;\nend;\n";
    std::fs::write(e.tmp.join("my_ieee.vhd"), mine).expect("write");
    let s = e.shared().display().to_string();
    e.analyze("a", &[]);
    let before = e.entries().len();
    let lib = format!("ieee:{}", e.tmp.join("myieee").display());
    let o = e
        .command(
            &s,
            &e.tmp,
            &["analyze", "--bootstrap", "--work", &lib, "my_ieee.vhd"],
        )
        .output()
        .expect("run");
    assert_eq!(o.status.code(), Some(0), "{:?}", text(&o));
    assert_eq!(
        e.entries().len(),
        before,
        "nothing of it in the shared store"
    );
    assert!(
        !entries_in(&e.tmp.join("myieee")).is_empty(),
        "it is the library's own"
    );
    let tb = "library ieee; use ieee.std_logic_1164.all;\nentity t2 is end;\narchitecture a of t2 is\n\
        constant C : integer := MINE;\nbegin\nend;\n";
    std::fs::write(e.tmp.join("t2.vhd"), tb).expect("write");
    let work = format!("work:{}", e.tmp.join("w").display());
    let o = e
        .command(
            &s,
            &e.tmp,
            &[
                "analyze",
                "--bootstrap",
                "--work",
                &work,
                "--map",
                &lib,
                "t2.vhd",
            ],
        )
        .output()
        .expect("run");
    assert_eq!(
        o.status.code(),
        Some(0),
        "uses the user's package: {:?}",
        text(&o)
    );
}

#[test]
fn the_flag_wins_over_the_environment_and_clean_clears_the_store() {
    let e = Env::new("share_flag");
    let env_store = e.tmp.join("from_env").display().to_string();
    let flag_store = e.tmp.join("from_flag");
    let flag = format!("--std-cache={}", flag_store.display());
    let o = e
        .analyze_in(&env_store, &e.tmp, "a", &[&flag])
        .wait_with_output()
        .expect("wait");
    assert_eq!(o.status.code(), Some(0), "{:?}", text(&o));
    assert!(
        entries_in(&e.tmp.join("from_env")).is_empty(),
        "env store untouched"
    );
    let made = entries_in(&flag_store).len();
    assert!(made >= 3, "the flag names the store: {made}");
    // `run` takes the flag too (reads only: a miss would still pass, so check the library is not it).
    let work = format!("a:{}", e.tmp.join("a").display());
    let r = e
        .command(&env_store, &e.tmp, &["run", &flag, "--work", &work, "tb"])
        .output()
        .expect("run");
    assert_eq!(r.status.code(), Some(0), "{:?}", text(&r));
    // `--std-cache=off` keeps the units in the library; an empty flag is E0126 even with a good env.
    let off = e
        .analyze_in(&env_store, &e.tmp, "b", &["--std-cache=off"])
        .wait_with_output()
        .expect("wait");
    assert_eq!(off.status.code(), Some(0));
    assert!(entries_in(&e.tmp.join("b")).len() >= 5);
    let empty = e
        .analyze_in(&env_store, &e.tmp, "c", &["--std-cache="])
        .wait_with_output()
        .expect("wait");
    assert_eq!(empty.status.code(), Some(2));
    assert!(text(&empty).2.contains("E0126"));
    // `cache clean` removes the library's entries and the shared store's; `off` leaves it.
    let lib = format!("a:{}", e.tmp.join("a").display());
    let keep = e
        .command(
            &env_store,
            &e.tmp,
            &["cache", "clean", "--work", &lib, "--std-cache=off"],
        )
        .output()
        .expect("clean");
    assert_eq!(keep.status.code(), Some(0), "{:?}", text(&keep));
    assert_eq!(entries_in(&flag_store).len(), made, "off leaves the store");
    let clean = e
        .command(
            &env_store,
            &e.tmp,
            &["cache", "clean", "--work", &lib, &flag],
        )
        .output()
        .expect("clean");
    assert_eq!(clean.status.code(), Some(0), "{:?}", text(&clean));
    assert!(
        entries_in(&flag_store).is_empty(),
        "the shared store is cleared"
    );
    // By environment alone it clears the store the environment names.
    let again = e
        .analyze_in(&env_store, &e.tmp, "d", &[])
        .wait_with_output()
        .expect("wait");
    assert_eq!(again.status.code(), Some(0));
    assert!(!entries_in(&e.tmp.join("from_env")).is_empty());
    let lib = format!("d:{}", e.tmp.join("d").display());
    let clean = e
        .command(&env_store, &e.tmp, &["cache", "clean", "--work", &lib])
        .output()
        .expect("clean");
    assert_eq!(clean.status.code(), Some(0), "{:?}", text(&clean));
    assert!(entries_in(&e.tmp.join("from_env")).is_empty());
}
