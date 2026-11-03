//! `--tier=clift-auto` with `YAVS_SPEC_AFTER=2`: processes still switch to their copies after the
//! tier has declined a function (an integer power has no native-frame variant, so its frame-mode
//! code waits for a compiled caller, `Worker::entered_from_code`). That look at every function
//! once fixed the reserved copy slots empty, and every later switch failed silently. Needs the
//! `clift` feature.
#![cfg(feature = "clift")]

mod common;
use common::tempdir;

use std::path::Path;
use std::process::Output;

const SRC: &str =
    "entity tickc is generic (K : natural := 1); port (clk : in bit; q : out natural); end;
architecture a of tickc is
begin
  process (clk)
    variable n : natural := 0;
  begin
    if clk = '1' then n := (n * 3 + K) mod 1009; q <= n; end if;
  end process;
end;
entity spec_decline is generic (N : natural := 400000); end;
architecture a of spec_decline is
  function pw(x, n : integer) return integer is
    variable r : integer := 0;
  begin
    for k in 1 to 6 loop
      r := (r + (x + k) ** n) mod 65537;
      if r mod 5 = 0 then r := r + k; end if;
    end loop;
    return r;
  end;
  function step(x : integer) return integer is
    variable t : integer := 0;
  begin
    for k in 1 to 4 loop
      t := t + pw(x mod 50 + k, 3) mod 1013;
    end loop;
    return t + x mod 7;
  end;
  signal clk : bit := '0';
  signal done : boolean := false;
  signal q1, q2, q3 : natural;
begin
  clk <= not clk after 5 ns when not done;
  u1 : entity work.tickc generic map (1) port map (clk, q1);
  u2 : entity work.tickc generic map (2) port map (clk, q2);
  u3 : entity work.tickc generic map (3) port map (clk, q3);
  process
    variable x : integer := 1;
    variable acc : integer := 0;
  begin
    for i in 1 to N loop
      x := step(x + i);
      acc := (acc + x) mod 1000003;
    end loop;
    report integer'image(acc);
    done <= true after 200 ns;
    wait;
  end process;
end;
";

fn yavs(dir: &Path, args: &[&str]) -> Output {
    let o = std::process::Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(args)
        .env("YAVS_AUTO_TRACE", "1")
        .env("YAVS_AUTO_FORCE", "step,_p0")
        .env("YAVS_COUNTERS", "1")
        .env("YAVS_SPEC_AFTER", "2")
        // (The count rule has native-frame code of `**`, and declines nothing here.)
        .env("YAVS_AUTO_POLICY", "sampled")
        .current_dir(dir)
        .output()
        .expect("run yavs");
    assert!(o.status.success(), "{o:?}");
    o
}

fn run(dir: &Path, tier: &str, n: u32) -> Output {
    let (tier, n) = (format!("--tier={tier}"), format!("-gN={n}"));
    yavs(
        dir,
        &[
            "run",
            "--std=2008",
            "--work=work:work",
            "--no-cache",
            &tier,
            &n,
            "spec_decline",
        ],
    )
}

fn counter(o: &Output, name: &str) -> u64 {
    String::from_utf8_lossy(&o.stderr)
        .lines()
        .find_map(|l| l.strip_prefix(&format!("counters: {name}=")))
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| panic!("no counter {name}"))
}

/// The run lasts seconds, the sampler declines `step` within milliseconds (the trace says so) and
/// looks at the callers at once, long before the clocked instances' second activation.
#[test]
fn processes_switch_to_copies_after_a_function_was_declined() {
    let dir = tempdir("clift_spec_decline");
    std::fs::write(dir.join("n.vhd"), SRC).expect("write");
    yavs(
        &dir,
        &["analyze", "--std=2008", "--work=work:work", "n.vhd"],
    );
    let interp = run(&dir, "interp", 400_000);
    let cold = run(&dir, "clift-auto", 400_000);
    assert_eq!(cold.stdout, interp.stdout);
    let err = String::from_utf8_lossy(&cold.stderr).into_owned();
    assert!(err.contains("no native-frame variant"), "{err}");
    let switched = counter(&cold, "spec_switched");
    assert!(
        switched >= 3,
        "{switched} processes switched to their copies:\n{err}"
    );
}
