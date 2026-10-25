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
    let (out, cold, err) = run();
    assert!(
        !cold.is_empty() && !cold.contains(&true),
        "the first run compiled nothing or linked from an empty cache: {err}"
    );
    let (out2, warm, err) = run();
    assert_eq!(out2, out);
    assert!(warm.contains(&true), "no hit in the warm run: {err}");
}
