//! The compilation cache keeps the code Cranelift compiles (#481): a second run links every
//! function from the cache and prints the same, a damaged entry is a miss, `--no-cache` stores
//! nothing, and a warm `--tier=clift-auto` run links a hot process from the cache. Needs the
//! `clift` feature.
#![cfg(feature = "clift")]

mod common;
use common::tempdir;

use std::path::Path;
use std::process::{Command, Output};

const SRC: &str = "entity clift_cache is generic (N : natural := 4000000); end;
architecture a of clift_cache is
  function step(x : integer) return integer is
  begin
    return (x * 75 + 74) mod 65537;
  end;
begin
  process
    variable x : integer := 1;
    variable acc : integer := 0;
  begin
    for i in 1 to N loop
      x := step(x);
      if x mod 3 = 0 then acc := (acc + x) mod 1000003; else acc := (acc * 3 + i) mod 1000003; end if;
    end loop;
    report integer'image(acc);
    wait;
  end process;
end;
";

fn yavs(dir: &Path, args: &[&str]) -> Output {
    let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(args)
        .env("YAVS_AUTO_TRACE", "1")
        .current_dir(dir)
        .output()
        .expect("run yavs");
    assert!(o.status.success(), "{o:?}");
    o
}

/// (functions linked from the cache, functions compiled and stored) of a `--tier=clift -v` run.
fn stats(o: &Output) -> (usize, usize) {
    let err = String::from_utf8_lossy(&o.stderr);
    let l = err
        .lines()
        .find(|l| l.contains("linked from the compilation cache"))
        .unwrap_or_else(|| panic!("no cache note: {err}"));
    let n: Vec<usize> = l
        .split_whitespace()
        .filter_map(|w| w.parse().ok())
        .collect();
    (n[0], n[1])
}

/// The cache entries of compiled Cranelift code under `dir`.
fn entries(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                todo.push(p);
            } else if p
                .file_name()
                .and_then(|n| n.to_str())
                .is_some_and(|n| n.starts_with("clif-"))
            {
                out.push(p);
            }
        }
    }
    out
}

#[test]
fn a_second_eager_run_links_every_function_from_the_cache() {
    let dir = tempdir("clift_cache_eager");
    std::fs::write(dir.join("c.vhd"), SRC).expect("write");
    yavs(
        &dir,
        &["analyze", "--std=2008", "--work=work:work", "c.vhd"],
    );
    let run = |extra: &[&str]| {
        let mut args = vec![
            "run",
            "--std=2008",
            "--work=work:work",
            "--tier=clift",
            "--verbose",
        ];
        args.extend_from_slice(extra);
        args.extend(["-gN=1000", "clift_cache"]);
        yavs(&dir, &args)
    };
    let none = run(&["--no-cache"]);
    assert_eq!(stats(&none), (0, 0));
    assert!(entries(&dir).is_empty());
    let cold = run(&[]);
    let (hits, stored) = stats(&cold);
    assert!(hits == 0 && stored > 0, "{hits} {stored}");
    let warm = run(&[]);
    assert_eq!(stats(&warm), (stored, 0));
    assert_eq!(warm.stdout, cold.stdout);
    // A damaged entry is a miss, compiled and stored again.
    for e in entries(&dir) {
        std::fs::write(&e, b"not an entry").expect("damage");
    }
    let again = run(&[]);
    assert_eq!(stats(&again), (0, stored));
    assert_eq!(again.stdout, cold.stdout);
}

#[test]
fn a_warm_clift_auto_run_links_the_hot_process_from_the_cache() {
    let dir = tempdir("clift_cache_auto");
    std::fs::write(dir.join("c.vhd"), SRC).expect("write");
    yavs(
        &dir,
        &["analyze", "--std=2008", "--work=work:work", "c.vhd"],
    );
    let run = || {
        let o = yavs(
            &dir,
            &[
                "run",
                "--std=2008",
                "--work=work:work",
                "--tier=clift-auto",
                "clift_cache",
            ],
        );
        let err = String::from_utf8_lossy(&o.stderr).into_owned();
        let compiled: Vec<bool> = err
            .lines()
            .filter(|l| l.contains("compiled `"))
            .map(|l| l.contains(" hit "))
            .collect();
        (o.stdout, compiled, err)
    };
    // A run that ends while the compile thread is still compiling or linking leaves the cache
    // as it was (the compile is abandoned at exit by design), and how soon a loaded host gets
    // there is not the cache's to control: a run is repeated, up to TRIES times, until it got
    // that far. A cache that never serves a hit still fails every attempt.
    const TRIES: usize = 5;
    let (out, mut cold, mut err) = run();
    for _ in 1..TRIES {
        if !cold.is_empty() {
            break;
        }
        // (A run can store its code and end before it prints the line: start from an empty cache.)
        for e in entries(&dir) {
            std::fs::remove_file(&e).expect("remove");
        }
        (_, cold, err) = run();
    }
    assert!(
        !cold.is_empty() && !cold.contains(&true),
        "the first run compiled nothing or linked from an empty cache: {err}"
    );
    for _ in 1..TRIES {
        let (out2, warm, _) = run();
        assert_eq!(out2, out);
        if warm.contains(&true) {
            return;
        }
    }
    let (out2, warm, err) = run();
    assert_eq!(out2, out);
    assert!(warm.contains(&true), "no hit in {TRIES} warm runs: {err}");
}

/// A package of a library both closures map, called by testbenches of two work libraries.
const SHARED_PKG: &str = "package p is function step(x : integer) return integer; end;
package body p is
  function step(x : integer) return integer is
  begin
    return (x * 75 + 74) mod 65537;
  end;
end;
";

fn shared_tb(name: &str) -> String {
    format!(
        "library shr; use shr.p.all;
entity {name} is end;
architecture a of {name} is
begin
  process
    variable x : integer := 1;
  begin
    for i in 1 to 1000 loop x := step(x); end loop;
    report integer'image(x);
    wait;
  end process;
end;
"
    )
}

/// The first run of a closure in another work library links what a closure of the first one
/// compiled of the same bodies, from that library's store, and writes nothing there;
/// `YAVS_CLIFT_SHARE=0` looks in its own store alone.
#[test]
fn a_closure_of_another_work_library_links_what_the_first_compiled() {
    let dir = tempdir("clift_cache_shared");
    std::fs::write(dir.join("p.vhd"), SHARED_PKG).expect("write");
    std::fs::write(dir.join("a.vhd"), shared_tb("tb_a")).expect("write");
    std::fs::write(dir.join("b.vhd"), shared_tb("tb_b")).expect("write");
    yavs(&dir, &["analyze", "--work=shr:shr", "p.vhd"]);
    yavs(&dir, &["analyze", "--work=wa:wa", "--map=shr:shr", "a.vhd"]);
    yavs(&dir, &["analyze", "--work=wb:wb", "--map=shr:shr", "b.vhd"]);
    let maps = ["--map=shr:shr", "--map=wa:wa", "--map=wb:wb"];
    let run = |work: &str, top: &str, share: &str| {
        let mut args = vec!["run", "--tier=clift", "--verbose", work];
        args.extend(maps);
        args.push(top);
        let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
            .args(&args)
            .env("YAVS_CLIFT_SHARE", share)
            .current_dir(&dir)
            .output()
            .expect("run yavs");
        assert!(o.status.success(), "{o:?}");
        o
    };
    let a = run("--work=wa:wa", "tb_a", "1");
    let (hits, stored) = stats(&a);
    assert!(hits == 0 && stored > 0, "{hits} {stored}");
    let in_a = entries(&dir.join("wa"));
    let alone = run("--work=wb:wb", "tb_b", "0");
    assert_eq!(stats(&alone).0, 0, "linked with the switch off");
    let _ = std::fs::remove_dir_all(dir.join("wb").join("yavs-cache").join("obj"));
    let b = run("--work=wb:wb", "tb_b", "1");
    assert!(stats(&b).0 > 0, "nothing linked from the other library");
    assert_eq!(b.stdout, alone.stdout);
    assert_eq!(
        entries(&dir.join("wa")),
        in_a,
        "the other library's store was written"
    );
}
