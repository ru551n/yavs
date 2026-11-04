//! The perf map of the compiled tiers (#1074, `YAVS_CLIFT_PERFMAP`, `YAVS_LLVM_PERFMAP`): one line
//! `address size name` per function of generated code, whether it was compiled in the run or linked
//! from the compilation cache, named after its process (instance path) or subprogram, and listed
//! once with the sizes the machine code has. Needs the `clift` or the `llvm` feature and a Unix host (the map is
//! written to `/tmp`, where `perf` reads it).
#![cfg(all(unix, any(feature = "clift", feature = "llvm")))]

mod common;
use common::tempdir;

use std::path::Path;
use std::process::{Command, Stdio};

/// Two instances of a process (equal code, one function) that call a function.
const SRC: &str = "entity leaf is generic (N : natural := 4000000); end;
architecture a of leaf is
  function step(x : integer) return integer is
  begin
    return (x * 75 + 74) mod 65537;
  end;
begin
  worker : process
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

entity perfmap is generic (N : natural := 4000000); end;
architecture a of perfmap is
begin
  u1 : entity work.leaf generic map (N => N);
  u2 : entity work.leaf generic map (N => N);
end;
";

/// A short run for the tiers that compile before time 0.
const SHORT: &[&str] = &["-gN=1000"];

/// One run's map: (address, size, name) per line, and the stderr of the run.
struct Map {
    lines: Vec<(usize, usize, String)>,
    err: String,
}

impl Map {
    fn names(&self) -> Vec<&str> {
        let mut n: Vec<&str> = self.lines.iter().map(|l| l.2.as_str()).collect();
        n.sort_unstable();
        n
    }
}

fn setup(name: &str) -> common::Tmp {
    let dir = tempdir(name);
    std::fs::write(dir.join("p.vhd"), SRC).expect("write");
    let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(["analyze", "--std=2008", "--work=work:work", "p.vhd"])
        .current_dir(&dir)
        .output()
        .expect("run yavs");
    assert!(o.status.success(), "{o:?}");
    dir
}

/// Run the design on `tier` with the perf map on (`var`), `jit_lists`: as under `perf record`,
/// which sets `PERF_BUILDID_DIR`. Reads and removes `/tmp/perf-<pid>.map`.
fn run(dir: &Path, tier: &str, var: &str, jit_lists: bool, gens: &[&str]) -> Map {
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_yavs"));
    cmd.args(["run", "--std=2008", "--work=work:work", tier])
        .args(gens)
        .arg("perfmap")
        .env(var, "1")
        .env("YAVS_AUTO_TRACE", "1")
        .env_remove("PERF_BUILDID_DIR")
        .current_dir(dir)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if jit_lists {
        cmd.env("PERF_BUILDID_DIR", dir);
    }
    let child = cmd.spawn().expect("run yavs");
    let path = format!("/tmp/perf-{}.map", child.id());
    let o = child.wait_with_output().expect("wait");
    assert!(o.status.success(), "{o:?}");
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"));
    let _ = std::fs::remove_file(&path);
    let hex = |s: &str| usize::from_str_radix(s, 16).unwrap_or_else(|_| panic!("`{s}` in {text}"));
    let lines = text
        .lines()
        .map(|l| {
            let mut w = l.splitn(3, ' ');
            let (a, s, n) = (w.next().unwrap(), w.next().unwrap(), w.next().unwrap());
            (hex(a), hex(s), n.to_string())
        })
        .collect();
    Map {
        lines,
        err: String::from_utf8_lossy(&o.stderr).into_owned(),
    }
}

/// Every line has a size and a readable name (kind, function id, process by instance path or
/// subprogram, or one of the module's stubs, `cold_*`), no address twice, no two functions overlap.
fn well_formed(m: &Map) {
    assert!(!m.lines.is_empty(), "empty map: {}", m.err);
    let mut sorted: Vec<_> = m.lines.iter().collect();
    sorted.sort();
    for (a, s, n) in &sorted {
        let named = n
            .strip_prefix(['f', 'n', 'c'])
            .and_then(|r| r.split_once('_'))
            .is_some_and(|(id, label)| {
                !id.is_empty() && id.chars().all(|c| c.is_ascii_digit()) && !label.is_empty()
            });
        assert!(
            (named || n.starts_with("cold_")) && *s > 0,
            "{a:x} {s:x} {n}"
        );
    }
    for w in sorted.windows(2) {
        assert!(
            w[0].0 + w[0].1 <= w[1].0,
            "{:x?} overlaps {:x?}",
            w[0],
            w[1]
        );
    }
    let names = m.names();
    assert!(
        names
            .iter()
            .any(|n| n.starts_with('f') && n.ends_with("_perfmap.u1.worker+1")),
        "the process, by instance path, once for both instances: {names:?}"
    );
    assert!(
        names
            .iter()
            .any(|n| n.starts_with('f') && n.ends_with("_step")),
        "{names:?}"
    );
}

/// A tier that compiles every function before time 0: the second run links all of them from the
/// compilation cache and lists the same code under the same names.
fn eager(tier: &str, var: &str, adapter: bool) {
    let dir = setup(&format!("perfmap_{tier}"));
    let tier = format!("--tier={tier}");
    let cold = run(&dir, &tier, var, false, SHORT);
    well_formed(&cold);
    let warm = run(&dir, &tier, var, false, SHORT);
    well_formed(&warm);
    assert_eq!(warm.names(), cold.names());
    if adapter {
        assert!(
            cold.names().iter().any(|n| n.starts_with('c')),
            "{:?}",
            cold.names()
        );
    }
    // As under `perf record`: the JIT module lists its functions itself, once, under the names.
    #[cfg(feature = "clift")]
    if var == "YAVS_CLIFT_PERFMAP" {
        let under_perf = run(&dir, &tier, var, true, SHORT);
        well_formed(&under_perf);
        assert_eq!(under_perf.names(), cold.names());
        let sizes = |m: &Map| m.lines.iter().map(|l| l.1).collect::<Vec<_>>();
        let (mut a, mut b) = (sizes(&cold), sizes(&under_perf));
        a.sort_unstable();
        b.sort_unstable();
        assert_eq!(a, b);
    }
}

/// A tier that compiles hot functions on a thread: the second run links the process from the
/// cache and still lists it.
fn auto(tier: &str, var: &str) {
    let dir = setup(&format!("perfmap_{tier}"));
    let tier = format!("--tier={tier}");
    let hits = |m: &Map| {
        m.err
            .lines()
            .filter(|l| l.contains("compiled `") && l.contains(" hit "))
            .count()
    };
    let cold = run(&dir, &tier, var, false, &[]);
    assert_eq!(hits(&cold), 0, "{}", cold.err);
    let warm = run(&dir, &tier, var, false, &[]);
    assert!(hits(&warm) > 0, "no hit in the warm run: {}", warm.err);
    for m in [&cold, &warm] {
        assert!(!m.lines.is_empty(), "{}", m.err);
        let names = m.names();
        assert!(
            names
                .iter()
                .any(|n| n.starts_with('f') && n.contains("_perfmap.u") && n.contains(".worker")),
            "{names:?}"
        );
        assert!(
            m.lines.iter().all(|l| l.1 > 0 && !l.2.is_empty()),
            "{:?}",
            m.lines
        );
    }
}

#[cfg(feature = "clift")]
#[test]
fn clift_lists_compiled_and_cached_code_by_name() {
    eager("clift", "YAVS_CLIFT_PERFMAP", true);
}

#[cfg(feature = "clift")]
#[test]
fn clift_auto_lists_cached_code_by_name() {
    auto("clift-auto", "YAVS_CLIFT_PERFMAP");
}

#[cfg(feature = "llvm")]
#[test]
fn llvm_lists_compiled_and_cached_code_by_name() {
    eager("llvm", "YAVS_LLVM_PERFMAP", false);
}

#[cfg(feature = "llvm")]
#[test]
fn llvm_auto_lists_cached_code_by_name() {
    auto("auto", "YAVS_LLVM_PERFMAP");
}
