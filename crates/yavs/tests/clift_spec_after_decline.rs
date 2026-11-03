//! `--tier=clift-auto` with `YAVS_SPEC_AFTER=2`: processes still switch to their copies after the
//! tier has declined a function (an integer power has no native-frame variant, so its frame-mode
//! code waits for a compiled caller, `Worker::entered_from_code`). That look at every function
//! once fixed the reserved copy slots empty, and every later switch failed silently. Needs the
//! `clift` feature.
//!
//! The first test needs the sampler and `YAVS_SPEC_AFTER=2`. The second (`copies_are_not_lost_at_scale`)
//! is the production path at a design's size: no environment, the default number of activations,
//! and every function compiled when the interpreter enters it, so that no sampler look and no
//! wall-clock enters the count.
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
        // (New code has a variant for every function: this is the rule for what the old has not.)
        .env("YAVS_CLIFT_EXITS", "old")
        .env("YAVS_AUTO_FORCE", "step,_p0")
        .env("YAVS_COUNTERS", "1")
        .env("YAVS_SPEC_AFTER", "2")
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

/// Many instances of clocked processes, a declined function called while they run, and no
/// `YAVS_SPEC_AFTER`: each process switches to its copy at its 1024th activation. Every function
/// is compiled when the interpreter enters it and the run waits for each compile
/// (`--tier-heat=0,0,0`), so the integer power is declined before any copy is made, on every host.
const SCALE: &str =
    "entity tickd is generic (K : natural := 1); port (clk : in bit; q : out natural); end;
architecture a of tickd is
begin
  process (clk)
    variable n : natural := 0;
  begin
    if clk = '1' then n := (n * 3 + K) mod 1009; q <= n; end if;
  end process;
end;
entity spec_scale is end;
architecture a of spec_scale is
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
  type nat_vec is array (0 to 23) of natural;
  signal q : nat_vec;
begin
  clk <= not clk after 5 ns when not done;
  g : for i in q'range generate
    u : entity work.tickd generic map (i mod 5 + 1) port map (clk, q(i));
  end generate;
  process
    variable acc : integer := 0;
  begin
    for i in 1 to 2000 loop
      acc := (acc + step(i)) mod 1000003;
      wait for 10 ns;
    end loop;
    for i in q'range loop acc := (acc + q(i) * (i + 1)) mod 1000003; end loop;
    report integer'image(acc);
    done <= true;
    wait;
  end process;
end;
";

#[test]
fn copies_are_not_lost_at_scale() {
    let dir = tempdir("clift_spec_scale");
    std::fs::write(dir.join("s.vhd"), SCALE).expect("write");
    let go = |env: &[(&str, &str)], args: &[&str]| {
        let o = std::process::Command::new(env!("CARGO_BIN_EXE_yavs"))
            .args(args)
            .env("YAVS_COUNTERS", "1")
            .envs(env.iter().copied())
            .current_dir(&dir)
            .output()
            .expect("run yavs");
        assert!(o.status.success(), "{o:?}");
        o
    };
    go(&[], &["analyze", "--std=2008", "--work=work:work", "s.vhd"]);
    let run = |env: &[(&str, &str)], tier: &[&str]| {
        let mut args = vec!["run", "--std=2008", "--work=work:work", "--no-cache"];
        args.extend_from_slice(tier);
        args.push("spec_scale");
        go(env, &args)
    };
    let interp = run(&[], &["--tier=interp"]);
    for threads in ["--threads=1", "--threads=4"] {
        let tier = ["--tier=clift-auto", "--tier-heat=0,0,0", threads];
        let o = run(&[("YAVS_AUTO_TRACE", "1")], &tier);
        let err = String::from_utf8_lossy(&o.stderr);
        assert_eq!(o.stdout, interp.stdout, "{threads}");
        assert!(
            err.contains("no native-frame variant of `step`"),
            "{threads}: {err}"
        );
        // Every one of the 24 processes of the instances switches (the other process runs
        // 2000 activations and may switch too).
        let switched = counter(&o, "spec_switched");
        assert!(
            switched >= 24,
            "{threads}: {switched} processes switched:\n{err}"
        );
        // And the copies run compiled: one compile for each distinct generic (five).
        let compiled = err.matches(", copy of f").count();
        assert!(
            compiled >= 5,
            "{threads}: {compiled} compiled copies:\n{err}"
        );
    }
}
