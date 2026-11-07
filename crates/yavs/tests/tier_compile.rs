//! `--tier-compile` (#1123): by default the run waits while a function compiles (`wait`, the
//! counter `tier_waited`), `background` compiles alongside it (`tier_alongside`) on a thread at
//! the lowest priority; the output is the interpreter's either way. The count rule with a count of
//! 1 and an open gate asks for the loop's function at once, so the compile does not wait for a
//! sampler; the loop runs in one activation, so the run waits at the interpreter's look in it.
#![cfg(feature = "clift")]

mod common;
use common::tempdir;

use std::path::Path;
use std::process::{Command, Output};

const SRC: &str = "entity tc is end;
architecture a of tc is
  function mix(x : integer; y : integer) return integer is
    variable v : integer := x;
  begin
    for i in 0 to 15 loop
      v := (v * 31 + y + i) mod 1000003;
    end loop;
    return v;
  end;
begin
  process
    variable acc : integer := 0;
  begin
    for j in 0 to ITER loop
      acc := mix(acc, j);
    end loop;
    report \"acc \" & integer'image(acc);
    wait;
  end process;
end;
";

fn yavs(dir: &Path, args: &[&str]) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_yavs"));
    c.args(args)
        .env("YAVS_COUNTERS", "1")
        .env("YAVS_AUTO_COUNT", "1")
        .env("YAVS_AUTO_GATE", "0")
        // (What the looks repaid is `auto.rs`'s test: here every asked function compiles.)
        .env("YAVS_AUTO_WAIT_PAYBACK", "0")
        .env_remove("YAVS_AUTO_SYNC")
        .env_remove("YAVS_AUTO_POLICY")
        .current_dir(dir);
    c
}

fn setup(name: &str, iter: u32) -> common::Tmp {
    let dir = tempdir(name);
    std::fs::write(dir.join("t.vhd"), SRC.replace("ITER", &iter.to_string())).expect("write");
    let a = yavs(&dir, &["analyze", "--work=work:work", "t.vhd"])
        .output()
        .expect("run yavs");
    assert!(a.status.success(), "{a:?}");
    dir
}

const RUN: [&str; 4] = ["run", "--work=work:work", "--no-cache", "--threads=1"];

fn run(dir: &Path, extra: &[&str]) -> Output {
    let o = yavs(dir, &[&RUN[..], extra, &["tc"]].concat())
        .output()
        .expect("run yavs");
    assert!(o.status.success(), "{o:?}");
    o
}

fn counter(o: &Output, name: &str) -> u64 {
    String::from_utf8_lossy(&o.stderr)
        .lines()
        .find_map(|l| l.strip_prefix(&format!("counters: {name}=")))
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| panic!("no counter {name}"))
}

#[test]
fn the_run_waits_for_its_compiles_by_default_and_prints_the_same() {
    let dir = setup("tier-compile", 300_000);
    let interp = run(&dir, &["--tier=interp"]);
    let wait = run(&dir, &["--tier=clift-auto"]);
    assert_eq!(wait.stdout, interp.stdout);
    assert!(counter(&wait, "tier_waited") > 0, "{wait:?}");
    assert_eq!(counter(&wait, "tier_alongside"), 0);
    let bg = run(&dir, &["--tier=clift-auto", "--tier-compile=background"]);
    assert_eq!(bg.stdout, interp.stdout);
    assert_eq!(counter(&bg, "tier_waited"), 0);
    assert!(counter(&bg, "tier_alongside") > 0, "{bg:?}");
}

/// The nice value of a thread of `pid` named `name`, from `/proc`.
#[cfg(target_os = "linux")]
fn nice_of(pid: u32, name: &str) -> Option<i64> {
    for t in std::fs::read_dir(format!("/proc/{pid}/task"))
        .ok()?
        .flatten()
    {
        let comm = std::fs::read_to_string(t.path().join("comm")).unwrap_or_default();
        if comm.trim_end() == name {
            let stat = std::fs::read_to_string(t.path().join("stat")).ok()?;
            // Field 19 is the nice value; the name in parentheses may hold spaces.
            let rest = &stat[stat.rfind(')')? + 2..];
            return rest.split(' ').nth(16)?.parse().ok();
        }
    }
    None
}

#[cfg(target_os = "linux")]
#[test]
fn the_background_compile_thread_runs_at_the_lowest_priority() {
    // Long enough for the test to look at the thread; it is killed once seen.
    let dir = setup("tier-compile-nice", 100_000_000);
    let mut child = yavs(
        &dir,
        &[
            &RUN[..],
            &["--tier=clift-auto", "--tier-compile=background", "tc"],
        ]
        .concat(),
    )
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::null())
    .spawn()
    .expect("start yavs");
    let t0 = std::time::Instant::now();
    let mut seen = None;
    while t0.elapsed() < std::time::Duration::from_secs(120) {
        if let Some(19) = nice_of(child.id(), "yavs-tier-up") {
            seen = Some(nice_of(child.id(), "yavs"));
            break;
        }
        if child.try_wait().expect("wait").is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let _ = child.kill();
    let _ = child.wait();
    // The compile thread is at 19 and the simulation's own thread keeps the test's priority.
    let stat = std::fs::read_to_string("/proc/self/stat").expect("own stat");
    let ours = stat[stat.rfind(')').expect("stat") + 2..]
        .split(' ')
        .nth(16)
        .and_then(|v| v.parse().ok());
    assert_eq!(
        seen,
        Some(ours),
        "no compile thread at nice 19 within the run"
    );
}
