//! The analysed-unit cache (`docs/decisions.md` "Analysed-unit cache"): `yavs analyze` of one
//! file per call reads the units it depends on from their entries; the diagnostics and results
//! are those of an analysis from source, an edited dependency invalidates everything below it,
//! read-only libraries and damaged entries still work, and `run` never writes an entry.

mod common;
use common::tempdir;

use std::path::PathBuf;
use std::process::Command;

const PKG: &str = "library ieee; use ieee.std_logic_1164.all;\npackage p is\n  constant K : integer := 7;\n\
  function f (x : std_logic_vector) return integer;\nend;\n\
package body p is\n  function f (x : std_logic_vector) return integer is begin return x'length * K; end;\nend;\n";

/// Uses `p`; warns (a static index outside the bounds, in dead code).
const MID: &str = "use work.p.all;\npackage m is\n  constant M : integer := K + 1;\nend;\n";

const TB: &str = "library ieee; use ieee.std_logic_1164.all;\nuse work.p.all; use work.m.all;\n\
entity tb is end;\narchitecture a of tb is\nbegin\n  process\n    variable v : bit_vector(0 to 3);\n  begin\n\
    if false then v(5) := '1'; end if;\n\
    report \"K=\" & integer'image(K) & \" M=\" & integer'image(M) & \" f=\" & integer'image(f(\"0101\"));\n\
    wait;\n  end process;\nend;\n";

struct Env {
    tmp: common::Tmp,
}

#[derive(Debug, PartialEq, Eq)]
struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Out {
    /// Without the debug notes of the caches.
    fn visible(&self) -> (i32, String, String) {
        let err = self
            .stderr
            .lines()
            .filter(|l| !l.starts_with("debug: "))
            .map(|l| format!("{l}\n"))
            .collect();
        (self.code, self.stdout.clone(), err)
    }

    fn notes(&self, what: &str) -> usize {
        self.stderr
            .lines()
            .filter(|l| l.starts_with("debug: units: ") && l.contains(what))
            .count()
    }
}

impl Env {
    fn new(name: &str) -> Env {
        let tmp = tempdir(name);
        for (f, t) in [("pkg.vhd", PKG), ("mid.vhd", MID), ("tb.vhd", TB)] {
            std::fs::write(tmp.join(f), t).expect("write");
        }
        Env { tmp }
    }

    fn lib(&self, name: &str) -> String {
        format!("{name}:{}", self.tmp.join(name).display())
    }

    fn units(&self, lib: &str) -> Vec<PathBuf> {
        std::fs::read_dir(self.tmp.join(lib).join("yavs-cache").join("units"))
            .map(|d| d.flatten().map(|e| e.path()).collect())
            .unwrap_or_default()
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

    /// `yavs analyze` of each file, one call per file, into `work` (with `more` arguments).
    fn analyze(&self, files: &[&str], more: &[&str]) -> Vec<Out> {
        let work = self.lib("work");
        files
            .iter()
            .map(|f| {
                let mut a = vec!["analyze", "--work", &work];
                a.extend_from_slice(more);
                a.push(f);
                let o = self.yavs(&a);
                assert_eq!(o.code, 0, "{f}: {o:?}");
                o
            })
            .collect()
    }

    fn run(&self, more: &[&str]) -> Out {
        let work = self.lib("work");
        let mut a = vec!["run", "--work", &work];
        a.extend_from_slice(more);
        a.push("tb");
        self.yavs(&a)
    }
}

const ALL: [&str; 3] = ["pkg.vhd", "mid.vhd", "tb.vhd"];

#[test]
fn cached_analysis_reports_what_source_does() {
    let e = Env::new("units_same");
    let plain = Env::new("units_same_plain");
    let cold = e.analyze(&ALL, &[]);
    let reference = plain.analyze(&ALL, &["--no-cache"]);
    assert!(plain.units("work").is_empty(), "--no-cache writes nothing");
    assert!(!e.units("work").is_empty());
    // The warning of the testbench comes out the same, from source and from its entry.
    assert!(reference[2].stderr.contains("W1404"), "{:?}", reference[2]);
    let warm = e.analyze(&ALL, &[]);
    for i in 0..ALL.len() {
        let strip = |o: &Out| o.visible().2.replace(&e.tmp.display().to_string(), "T");
        let strip_p = |o: &Out| o.visible().2.replace(&plain.tmp.display().to_string(), "T");
        assert_eq!(strip(&cold[i]), strip_p(&reference[i]), "{}", ALL[i]);
        assert_eq!(strip(&warm[i]), strip_p(&reference[i]), "{}", ALL[i]);
    }
    // The second time every unit of the files is read back.
    assert_eq!(warm[2].notes("hit work.tb ("), 1, "{:?}", warm[2]);
    assert_eq!(warm[2].notes("miss"), 0, "{:?}", warm[2]);
    // The file itself is not parsed: its units have entries made from it (no leading empty
    // segment may hide the file step, which cost 15 ms of every first run of a library testbench).
    assert_eq!(warm[2].notes("tb.vhd not parsed"), 1, "{:?}", warm[2]);
    assert_eq!(cold[2].notes("not parsed"), 0, "{:?}", cold[2]);
    // The check mode analyses again without the cache and compares the two.
    let o = e.yavs(&["analyze", "--work", &e.lib("work"), "tb.vhd"]);
    assert_eq!(o.code, 0);
    let mut c = Command::new(env!("CARGO_BIN_EXE_yavs"));
    c.current_dir(&e.tmp)
        .env_remove("YAVS_STDLIB")
        .env("YAVS_UNIT_CHECK", "1")
        .args(["analyze", "--work", &e.lib("work"), "mid.vhd", "tb.vhd"]);
    assert!(c.status().expect("run").success());
    let r = e.run(&[]);
    assert_eq!(r.code, 0, "{r:?}");
    assert!(r.stdout.contains("K=7 M=8 f=28"), "{r:?}");
    // `analyze` stored the std/ieee package bodies elaboration loads (the files here were
    // analysed by relative path, so their own units are entries of another identity than the
    // closure's absolute paths).
    assert_eq!(r.notes("miss ieee") + r.notes("miss std"), 0, "{r:?}");
    assert!(r.notes("hit ieee.std_logic_1164/body") == 1, "{r:?}");
}

#[test]
fn editing_a_dependency_invalidates_its_dependents() {
    let e = Env::new("units_edit");
    e.analyze(&ALL, &[]);
    std::fs::write(e.tmp.join("pkg.vhd"), PKG.replace(":= 7", ":= 9")).expect("write");
    // `p` changed: re-analysing the files below it reads nothing stale.
    let again = e.analyze(&ALL, &[]);
    assert_eq!(again[1].notes("hit work.m"), 0, "{:?}", again[1]);
    assert!(again[1].notes("store work") >= 1, "{:?}", again[1]);
    let r = e.run(&[]);
    assert!(r.stdout.contains("K=9 M=10 f=36"), "{r:?}");
    // A dependent analysed before the dependency is analysed again is not read either: its
    // entry names the old `p`.
    std::fs::write(e.tmp.join("pkg.vhd"), PKG.replace(":= 7", ":= 3")).expect("write");
    e.analyze(&["pkg.vhd"], &[]);
    let tb = e.analyze(&["tb.vhd"], &[]);
    assert_eq!(tb[0].notes("hit work.m"), 0, "{:?}", tb[0]);
    let m = e.analyze(&["mid.vhd"], &[]);
    assert_eq!(
        m[0].notes("hit"),
        m[0].notes("hit std") + m[0].notes("hit ieee") + m[0].notes("hit work.p")
    );
    let r = e.run(&[]);
    assert!(r.stdout.contains("K=3 M=4 f=12"), "{r:?}");
}

/// One call with a dependency and its dependents, the dependency edited since the library was made:
/// the entries the call read before the edit's file is analysed again are not used for the files
/// after it (the session keeps the entries it read, #748), and the files before a failing one stay
/// recorded (the index is written once, after the failure, #747).
#[test]
fn a_batch_after_an_edit_reads_nothing_stale_and_records_what_it_did() {
    let e = Env::new("units_batch_edit");
    let all = e.abs(&ALL);
    let all: Vec<&str> = all.iter().map(String::as_str).collect();
    let work = e.lib("work");
    let call = |files: &[&str]| {
        let mut a = vec!["analyze", "--work", work.as_str()];
        a.extend_from_slice(files);
        e.yavs(&a)
    };
    let cold = call(&all);
    assert_eq!(cold.code, 0, "{cold:?}");
    let warm = call(&all);
    assert_eq!(warm.notes("verified work.m "), 1, "{warm:?}");
    std::fs::write(e.tmp.join("pkg.vhd"), PKG.replace(":= 7", ":= 9")).expect("write");
    let edited = call(&all);
    assert_eq!(edited.code, 0, "{edited:?}");
    assert_eq!(edited.notes("verified work.m "), 0, "{edited:?}");
    assert_eq!(edited.notes("hit work.m"), 0, "{edited:?}");
    let r = e.run(&[]);
    assert!(r.stdout.contains("K=9 M=10 f=36"), "{r:?}");
    // A failing second file: the first stays recorded.
    let fresh = Env::new("units_batch_fail");
    std::fs::write(fresh.tmp.join("bad.vhd"), "entity broken is\n").expect("write");
    let files = fresh.abs(&["pkg.vhd", "bad.vhd", "mid.vhd"]);
    let files: Vec<&str> = files.iter().map(String::as_str).collect();
    let work = fresh.lib("work");
    let mut a = vec!["analyze", "--work", work.as_str()];
    a.extend_from_slice(&files);
    let o = fresh.yavs(&a);
    assert_eq!(o.code, 1, "{o:?}");
    let index =
        std::fs::read_to_string(fresh.tmp.join("work").join("yavs-lib.toml")).expect("index");
    assert!(
        index.contains("pkg.vhd") && !index.contains("mid.vhd"),
        "{index}"
    );
}

impl Env {
    /// The files by absolute path, as integrations pass them (a dependency is found by the path the
    /// manifest records).
    fn abs(&self, files: &[&str]) -> Vec<String> {
        files
            .iter()
            .map(|f| self.tmp.join(f).display().to_string())
            .collect()
    }

    /// The index of `work` with each record it names in its place, and this directory made
    /// anonymous (the digests that name the records depend on it).
    fn manifest(&self) -> String {
        let dir = self.tmp.join("work");
        let index = std::fs::read_to_string(dir.join("yavs-lib.toml")).expect("manifest");
        let mut out = String::new();
        for line in index.lines() {
            match line.strip_prefix("record = \"") {
                Some(d) => {
                    let name = format!("{}.toml", d.trim_end_matches('"'));
                    out.push_str("record:\n");
                    out.push_str(
                        &std::fs::read_to_string(dir.join("yavs-lib.d").join(name))
                            .expect("record"),
                    );
                }
                None => {
                    out.push_str(line);
                    out.push('\n');
                }
            }
        }
        // Paths with a backslash (Windows) are escaped in the text.
        let tmp = self.tmp.display().to_string();
        out.replace(&tmp.replace('\\', "\\\\"), "T")
            .replace(&tmp, "T")
    }
}

/// A file analysed again whose units have entries that hold, which report nothing and which the
/// manifest records as they were, is not loaded at all: its dependencies are not replayed to give
/// back the records the manifest has. What it writes and prints is what a replay writes and prints.
#[test]
fn warm_files_are_verified_not_replayed() {
    let e = Env::new("units_verified");
    let plain = Env::new("units_verified_plain");
    let two = e.abs(&["pkg.vhd", "mid.vhd"]);
    let two: Vec<&str> = two.iter().map(String::as_str).collect();
    let two_plain = plain.abs(&["pkg.vhd", "mid.vhd"]);
    let two_plain: Vec<&str> = two_plain.iter().map(String::as_str).collect();
    let cold = e.analyze(&two, &[]);
    plain.analyze(&two_plain, &["--no-cache"]);
    assert_eq!(cold[1].notes("verified work."), 0, "{:?}", cold[1]);
    let warm = e.analyze(&two, &[]);
    for (i, f) in ["work.p", "work.m"].iter().enumerate() {
        assert_eq!(warm[i].notes(&format!("verified {f} ")), 1, "{:?}", warm[i]);
        assert_eq!(warm[i].notes("hit"), 0, "{:?}", warm[i]);
        assert_eq!(warm[i].notes("not parsed"), 1, "{:?}", warm[i]);
        assert_eq!(warm[i].visible().2, "", "{:?}", warm[i]);
    }
    // The manifest is the one an analysis from source writes.
    assert_eq!(e.manifest(), plain.manifest());
    // One call with both files, and a third that needs what the first two registered: the units
    // are loaded when a later file needs them, and its warning comes out once.
    let three = e.abs(&["pkg.vhd", "mid.vhd", "tb.vhd"]);
    let mut args = vec!["analyze".to_string(), "--work".into(), e.lib("work")];
    args.extend(three);
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    let tb_abs = e.abs(&["tb.vhd"]);
    e.analyze(&[tb_abs[0].as_str()], &[]);
    let batch = e.yavs(&args);
    assert_eq!(batch.code, 0, "{batch:?}");
    assert_eq!(batch.notes("verified work.p "), 1, "{batch:?}");
    assert_eq!(batch.notes("verified work.m "), 1, "{batch:?}");
    assert_eq!(batch.notes("hit work.tb ("), 1, "{batch:?}");
    assert_eq!(batch.stderr.matches("W1404").count(), 1, "{batch:?}");
    // The check mode analyses without the cache and compares: the same on a warm library.
    let mut c = Command::new(env!("CARGO_BIN_EXE_yavs"));
    c.current_dir(&e.tmp)
        .env_remove("YAVS_STDLIB")
        .env("YAVS_UNIT_CHECK", "1")
        .args(["analyze", "--work", &e.lib("work")])
        .args(&two);
    assert!(c.status().expect("run").success());
}

/// Warnings are shown for the files named in the call only (#584): a dependency's warning is not
/// repeated by the files that use it, with the cache or without. A file whose own units report
/// something is read back (and prints it); one whose units are quiet is verified even when a
/// dependency has a warning. An edited dependency is not trusted.
#[test]
fn verification_needs_quiet_units_and_unchanged_sources() {
    let e = Env::new("units_verified_edge");
    let plain = Env::new("units_verified_edge_plain");
    let wa = "package wa is\n  shared variable v : time;\nend;\n";
    let wb = "use work.wa.all;\npackage wb is\n  constant c : integer := 1;\nend;\n";
    for env in [&e, &plain] {
        std::fs::write(env.tmp.join("wa.vhd"), wa).expect("write");
        std::fs::write(env.tmp.join("wb.vhd"), wb).expect("write");
    }
    let files = e.abs(&["wa.vhd", "wb.vhd"]);
    let files: Vec<&str> = files.iter().map(String::as_str).collect();
    let files_plain = plain.abs(&["wa.vhd", "wb.vhd"]);
    let files_plain: Vec<&str> = files_plain.iter().map(String::as_str).collect();
    let cold = e.analyze(&files, &["--relaxed"]);
    let warm = e.analyze(&files, &["--relaxed"]);
    let reference = plain.analyze(&files_plain, &["--relaxed", "--no-cache"]);
    let strip = |o: &Out, env: &Env| o.visible().2.replace(&env.tmp.display().to_string(), "T");
    for (i, shows) in [true, false].into_iter().enumerate() {
        for o in [&cold[i], &warm[i]] {
            assert_eq!(o.stderr.contains("W1904"), shows, "{i}: {o:?}");
            assert_eq!(strip(o, &e), strip(&reference[i], &plain), "{i}");
        }
    }
    assert_eq!(warm[0].notes("verified work."), 0, "{:?}", warm[0]);
    assert_eq!(warm[1].notes("verified work.wb "), 1, "{:?}", warm[1]);
    // An edited dependency: the file below it is analysed from source, then verified again.
    let e = Env::new("units_verified_edit");
    let all = e.abs(&["pkg.vhd", "mid.vhd"]);
    let all: Vec<&str> = all.iter().map(String::as_str).collect();
    e.analyze(&all, &[]);
    std::fs::write(e.tmp.join("pkg.vhd"), PKG.replace(":= 7", ":= 9")).expect("write");
    let mid = e.analyze(&all[1..], &[]);
    assert_eq!(mid[0].notes("verified work."), 0, "{:?}", mid[0]);
    e.analyze(&all, &[]);
    let again = e.analyze(&all, &[]);
    assert_eq!(again[0].notes("verified work.p "), 1, "{:?}", again[0]);
    assert_eq!(again[1].notes("verified work.m "), 1, "{:?}", again[1]);
}

#[test]
fn damaged_entries_are_misses() {
    let e = Env::new("units_damaged");
    let reference = e.analyze(&ALL, &[]);
    for f in e.units("work") {
        let mut b = std::fs::read(&f).expect("read");
        let n = b.len();
        b[n / 2] ^= 0x55;
        std::fs::write(&f, b).expect("write");
    }
    let again = e.analyze(&ALL, &[]);
    for i in 0..ALL.len() {
        assert_eq!(again[i].visible(), reference[i].visible());
    }
    assert!(e.run(&[]).stdout.contains("K=7 M=8 f=28"));
}

#[test]
fn run_never_writes_entries() {
    let e = Env::new("units_run");
    e.analyze(&ALL, &[]);
    let before: Vec<_> = e.units("work");
    for f in &before {
        std::fs::remove_file(f).expect("remove");
    }
    let r = e.run(&[]);
    assert_eq!(r.code, 0, "{r:?}");
    assert!(e.units("work").is_empty(), "{r:?}");
}

#[cfg(unix)]
#[test]
fn read_only_libraries_are_read_and_never_fail() {
    use std::os::unix::fs::PermissionsExt;
    let e = Env::new("units_ro");
    let shared = e.lib("common");
    // By absolute path, as integrations pass them: a unit read as a dependency is found by the
    // path the manifest records.
    for f in ["pkg.vhd", "mid.vhd"] {
        let f = e.tmp.join(f).display().to_string();
        assert_eq!(e.yavs(&["analyze", "--work", &shared, &f]).code, 0);
    }
    let lib = e.tmp.join("common");
    let tree: Vec<PathBuf> = vec![
        lib.clone(),
        lib.join("yavs-cache"),
        lib.join("yavs-cache").join("units"),
    ];
    let mode = |m: u32| {
        for d in tree.iter().rev() {
            std::fs::set_permissions(d, std::fs::Permissions::from_mode(m)).expect("chmod");
        }
    };
    mode(0o555);
    let writable = std::fs::File::create(lib.join("probe")).is_ok();
    // Into another library, reading `common` (read-only): its units come from their entries.
    let other = format!("other:{}", e.tmp.join("other").display());
    let tb = format!("library common;\n{}", TB.replace("work.", "common."));
    std::fs::write(e.tmp.join("tb_ro.vhd"), tb).expect("write");
    let a = e.yavs(&["analyze", "--work", &other, "--map", &shared, "tb_ro.vhd"]);
    let r = e.yavs(&["run", "--work", &other, "--map", &shared, "tb"]);
    mode(0o755);
    if writable {
        return; // running as a user the mode does not bind
    }
    assert_eq!(a.code, 0, "{a:?}");
    assert!(
        a.notes("hit common.p ") + a.notes("hit common.m ") >= 1 && a.notes("miss common") == 0,
        "{a:?}"
    );
    assert_eq!(r.code, 0, "{r:?}");
    assert!(r.stdout.contains("K=7 M=8 f=28"), "{r:?}");
}

/// Forces, `'active` reads and a package-path external name are facts of the elaborator
/// (`Db::facts`); an entry stores them, so a replayed unit need not be scanned. The check mode
/// compares them with those of an analysis from source.
#[test]
fn entries_carry_the_facts_of_their_unit() {
    let e = Env::new("units_facts");
    let sp = "library ieee; use ieee.std_logic_1164.all;\npackage sp is\n  signal s : std_logic;\nend;\n";
    let tb = "library ieee; use ieee.std_logic_1164.all;\nuse work.sp.all;\n\
entity ftb is end;\narchitecture a of ftb is\n  signal t : std_logic;\nbegin\n  process\n  begin\n\
    t <= force '1';\n    wait for 1 ns;\n    t <= release;\n    wait for 1 ns;\n\
    assert t'active or true;\n    report \"S=\" & std_logic'image(<<signal @work.sp.s : std_logic>>);\n    wait;\n\
  end process;\nend;\n";
    std::fs::write(e.tmp.join("sp.vhd"), sp).expect("write");
    std::fs::write(e.tmp.join("ftb.vhd"), tb).expect("write");
    // By absolute path, as integrations pass them: a dependency is found by the path the
    // manifest records.
    let paths: Vec<String> = ["sp.vhd", "ftb.vhd"]
        .iter()
        .map(|f| e.tmp.join(f).display().to_string())
        .collect();
    let files: Vec<&str> = paths.iter().map(String::as_str).collect();
    e.analyze(&files, &[]);
    let mut c = Command::new(env!("CARGO_BIN_EXE_yavs"));
    c.current_dir(&e.tmp)
        .env_remove("YAVS_STDLIB")
        .env("YAVS_UNIT_CHECK", "1")
        .args(["analyze", "--work", &e.lib("work")])
        .args(files);
    let o = c.output().expect("run");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let work = e.lib("work");
    let r = e.yavs(&["run", "--work", &work, "ftb"]);
    assert_eq!(r.code, 0, "{r:?}");
    assert!(r.stdout.contains("S=") || r.stderr.contains("S="), "{r:?}");
}

#[test]
fn a_package_body_removes_the_generics_its_declaration_made() {
    // The body of a generic package takes the generics of its declaration out of the set of unused
    // ones (and warns about those left): a change of a key another unit made, which the entry of
    // the body records, so that `run` reads the body back instead of analysing it again.
    let e = Env::new("units_generic_body");
    let gp = "package gp is\n  generic (used : integer; unused : integer := 0);\n  \
function twice (x : integer) return integer;\nend;\n\
package body gp is\n  function twice (x : integer) return integer is begin return x * used; end;\nend;\n";
    let tb = "entity gtb is end;\narchitecture a of gtb is\n  package ip is new work.gp generic map (used => 3);\n\
begin\n  process\n  begin\n    report \"twice=\" & integer'image(ip.twice(5));\n    wait;\n  end process;\nend;\n";
    std::fs::write(e.tmp.join("gp.vhd"), gp).expect("write");
    std::fs::write(e.tmp.join("gtb.vhd"), tb).expect("write");
    let plain = Env::new("units_generic_body_plain");
    std::fs::write(plain.tmp.join("gp.vhd"), gp).expect("write");
    std::fs::write(plain.tmp.join("gtb.vhd"), tb).expect("write");
    // By absolute path, as integrations pass them (the closure of `run` names its files so).
    let abs = |e: &Env| -> Vec<String> {
        ["gp.vhd", "gtb.vhd"]
            .iter()
            .map(|f| e.tmp.join(f).display().to_string())
            .collect()
    };
    let (files, pfiles) = (abs(&e), abs(&plain));
    let files: Vec<&str> = files.iter().map(String::as_str).collect();
    let pfiles: Vec<&str> = pfiles.iter().map(String::as_str).collect();
    let cold = e.analyze(&files, &[]);
    assert_eq!(cold[0].notes("not stored"), 0, "{:?}", cold[0]);
    assert!(cold[0].stderr.contains("W1501"), "{:?}", cold[0]);
    let work = e.lib("work");
    let r = e.yavs(&["run", "--work", &work, "gtb"]);
    assert_eq!(r.code, 0, "{r:?}");
    assert!(r.stdout.contains("twice=15"), "{r:?}");
    assert_eq!(r.notes("hit work.gp/body"), 1, "{r:?}");
    assert_eq!(r.notes("miss work.gp/body"), 0, "{r:?}");
    // Without the cache the same output comes out.
    plain.analyze(&pfiles, &["--no-cache"]);
    let pwork = plain.lib("work");
    let p = plain.yavs(&["run", "--work", &pwork, "--no-cache", "gtb"]);
    let strip = |o: &Out, e: &Env| o.visible().2.replace(&e.tmp.display().to_string(), "T");
    assert_eq!(strip(&r, &e), strip(&p, &plain));
    assert_eq!(r.stdout, p.stdout);
}

#[test]
fn a_package_body_in_a_file_of_its_own_removes_the_generics_too() {
    // The declaration is loaded inside the analysis of the body here, so the generics it made are
    // put in and taken out again within one unit: the entry of the body still has to take them
    // out after the declaration is replayed (the check mode compares the two analyses).
    let e = Env::new("units_generic_body_split");
    let decl = "package gp is\n  generic (used : integer; unused : integer := 0);\n  \
function twice (x : integer) return integer;\nend;\n";
    let body = "package body gp is\n  function twice (x : integer) return integer is begin return x * used; end;\nend;\n";
    let tb = "entity gtb is end;\narchitecture a of gtb is\n  package ip is new work.gp generic map (used => 3);\n\
begin\n  process\n  begin\n    report \"twice=\" & integer'image(ip.twice(5));\n    wait;\n  end process;\nend;\n";
    for (f, t) in [("gp.vhd", decl), ("gpb.vhd", body), ("gtb.vhd", tb)] {
        std::fs::write(e.tmp.join(f), t).expect("write");
    }
    let paths: Vec<String> = ["gp.vhd", "gpb.vhd", "gtb.vhd"]
        .iter()
        .map(|f| e.tmp.join(f).display().to_string())
        .collect();
    let files: Vec<&str> = paths.iter().map(String::as_str).collect();
    e.analyze(&files, &[]);
    for _ in 0..2 {
        let mut c = Command::new(env!("CARGO_BIN_EXE_yavs"));
        c.current_dir(&e.tmp)
            .env_remove("YAVS_STDLIB")
            .env("YAVS_UNIT_CHECK", "1")
            .args(["analyze", "--work", &e.lib("work")])
            .args(&files);
        let o = c.output().expect("run");
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    }
    let work = e.lib("work");
    let r = e.yavs(&["run", "--work", &work, "gtb"]);
    assert_eq!(r.code, 0, "{r:?}");
    assert!(r.stdout.contains("twice=15"), "{r:?}");
}

/// An entry serves a session with the other `--relaxed` unless its unit's analysis asked for it
/// (`Db::relaxed`): a library analysed without it is read by a relaxed run, a unit that needed
/// the relaxation is analysed again from source without it and fails as from source.
#[test]
fn relaxed_is_asked_only_where_it_changes_the_analysis() {
    let e = Env::new("units_relaxed");
    let all = e.abs(&ALL);
    let all: Vec<&str> = all.iter().map(String::as_str).collect();
    e.analyze(&all, &[]);
    let r = e.run(&["--relaxed"]);
    assert_eq!(r.code, 0, "{r:?}");
    assert!(r.stdout.contains("K=7 M=8 f=28"), "{r:?}");
    // The library's own units are read; a std/ieee body with an assignment to an unconstrained
    // target asked (R5 gives such a target its run-time bounds), so it has an entry per setting.
    assert_eq!(r.notes("miss work"), 0, "{r:?}");
    assert_eq!(r.notes("hit work.tb/a"), 1, "{r:?}");
    // A shared variable of a non-protected type needs `--relaxed` (W1904): its entry says so.
    let sv = e.tmp.join("sv.vhd");
    std::fs::write(
        &sv,
        "entity sv is end;\narchitecture a of sv is\n  shared variable n : integer := 3;\n\
         begin\n  process begin report \"n=\" & integer'image(n); wait; end process;\nend;\n",
    )
    .expect("write");
    let sv = sv.display().to_string();
    e.analyze(&[&sv], &["--relaxed"]);
    let work = e.lib("work");
    let relaxed = e.yavs(&["run", "--relaxed", "--work", &work, "sv"]);
    assert!(relaxed.stdout.contains("n=3"), "{relaxed:?}");
    assert_eq!(relaxed.notes("hit work.sv/a"), 1, "{relaxed:?}");
    let strict = e.yavs(&["run", "--work", &work, "sv"]);
    assert_ne!(strict.code, 0, "{strict:?}");
    assert!(strict.stderr.contains("E1904"), "{strict:?}");
    assert_eq!(strict.notes("hit work.sv/a"), 0, "{strict:?}");
}
