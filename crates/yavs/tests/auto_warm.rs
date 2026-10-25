//! `--tier=auto`, warm run (#749): a process that runs one long loop and waits once never comes
//! back for the switch to its copy, so the tier must not hold its shared code back for it: a
//! second run links the code from the compilation cache as soon as the process is hot, not
//! after the 400 ms the wait for the switch takes. Needs the `llvm` feature; the amounts of
//! work are instructions (the tier's work clock), so they do not depend on the host's load.
#![cfg(feature = "llvm")]

mod common;
use common::tempdir;

use std::process::Command;

const SRC: &str = "-- One process: a loop of integer arithmetic, then the final wait. (It reads a generic, so it
-- can have a copy specialised to its instance data.)
entity auto_warm is generic (N : natural := 4000000); end;
architecture a of auto_warm is
begin
  process
    variable x : integer := 1;
    variable acc : integer := 0;
  begin
    for i in 1 to N loop
      x := (x * 75 + 74) mod 65537;
      if x mod 3 = 0 then acc := (acc + x) mod 1000003; else acc := (acc * 3 + i) mod 1000003; end if;
    end loop;
    report integer'image(acc);
    wait;
  end process;
end;
";

/// The trace lines of the compiles of a run: "compiled ... seen <work>/<n> us".
fn compiles(err: &str) -> Vec<(bool, u64)> {
    err.lines()
        .filter(|l| l.contains("compiled `"))
        .map(|l| {
            let seen = l.rsplit("seen ").next().and_then(|s| s.split('/').next());
            (
                l.contains(" hit "),
                seen.and_then(|s| s.parse().ok())
                    .unwrap_or_else(|| panic!("no work in {l}")),
            )
        })
        .collect()
}

#[test]
fn a_process_that_never_resumes_is_linked_from_the_cache_early() {
    let dir = tempdir("auto_warm");
    std::fs::write(dir.join("auto_warm.vhd"), SRC).expect("write");
    let yavs = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_yavs"))
            .args(args)
            .env("YAVS_AUTO_TRACE", "1")
            .current_dir(&dir)
            .output()
            .expect("run yavs")
    };
    let a = yavs(&["analyze", "--std=2008", "--work=work:work", "auto_warm.vhd"]);
    assert!(a.status.success(), "{a:?}");
    let run = || {
        let o = yavs(&[
            "run",
            "--std=2008",
            "--work=work:work",
            "--tier=auto",
            "auto_warm",
        ]);
        assert!(o.status.success(), "{o:?}");
        compiles(&String::from_utf8_lossy(&o.stderr))
    };
    let cold = run();
    assert!(
        !cold.is_empty(),
        "the first run compiled nothing: raise the loop count"
    );
    let warm = run();
    // Before the fix the process was seen for the whole wait (400 000 us) before it was linked.
    // (Where the system gives no instruction counter the clock is CPU time and the figure is
    // larger: 110 000 us on the Windows runner.)
    assert!(
        matches!(warm[..], [(true, seen)] if seen < 300_000),
        "cold {cold:?}, warm {warm:?}"
    );
}
