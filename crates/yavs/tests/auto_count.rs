//! The count rule of `--tier=auto` on Cranelift (`Worker::waits`, `NativeFlags::arm_count`,
//! docs/decisions.md "`--tier=auto`: the count rule"). `YAVS_AUTO_SYNC` makes a run wait for what its
//! count asks for and `YAVS_AUTO_GATE=0` opens the gate, so nothing here depends on a sampler or a
//! clock. Each test names a counter that falls to zero when the mechanism stops working: the copy-slot
//! bug passed every gate with the output right, only slower.
#![cfg(feature = "clift")]

mod common;
use common::tempdir;

use std::path::Path;
use std::process::Output;

fn yavs(dir: &Path, env: &[(&str, &str)], args: &[&str]) -> Output {
    let mut c = std::process::Command::new(env!("CARGO_BIN_EXE_yavs"));
    c.args(args)
        .env("YAVS_COUNTERS", "1")
        .env_remove("YAVS_SPEC_AFTER")
        .env_remove("YAVS_AUTO_POLICY")
        .current_dir(dir);
    for (k, v) in env {
        c.env(k, v);
    }
    c.output().expect("run yavs")
}

fn counter(o: &Output, name: &str) -> u64 {
    String::from_utf8_lossy(&o.stderr)
        .lines()
        .find_map(|l| l.strip_prefix(&format!("counters: {name}=")))
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| panic!("no counter {name}"))
}

/// Analyses `src` in a new directory.
fn setup(name: &str, src: &str) -> common::Tmp {
    let dir = tempdir(name);
    std::fs::write(dir.join("t.vhd"), src).expect("write");
    let a = yavs(
        &dir,
        &[],
        &["analyze", "--std=2008", "--work=work:work", "t.vhd"],
    );
    assert!(a.status.success(), "{a:?}");
    dir
}

/// A run under the rule with a count of `count`, its output equal to the interpreter's.
fn counted(dir: &Path, top: &str, count: &str, extra: &[(&str, &str)]) -> Output {
    let run = |tier: &str, env: &[(&str, &str)]| {
        let o = yavs(
            dir,
            env,
            &[
                "run",
                "--std=2008",
                "--work=work:work",
                "--no-cache",
                "--threads=1",
                "--verbose",
                tier,
                top,
            ],
        );
        assert!(o.status.success(), "{o:?}");
        o
    };
    let interp = run("--tier=interp", &[]);
    let mut env = vec![
        ("YAVS_AUTO_COUNT", count),
        ("YAVS_AUTO_GATE", "0"),
        ("YAVS_AUTO_SYNC", "1"),
    ];
    env.extend_from_slice(extra);
    let o = run("--tier=clift-auto", &env);
    assert_eq!(o.stdout, interp.stdout);
    o
}

fn err(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

/// Whether the tier said it compiled the function `name`.
fn compiled(o: &Output, name: &str) -> bool {
    err(o)
        .lines()
        .any(|l| l.starts_with(&format!("note: `{name}` ")) && l.contains("is hot: compiled by"))
}

const CALLS: &str = "entity counted is end;
architecture a of counted is
  function plain(x : integer) return integer is
    variable a, b : integer;
  begin
    a := (x * 3 + 1) mod 1009;
    b := (a * 5 + x) mod 1013;
    if b > 500 then a := a + b; else a := a - b; end if;
    b := (a * 7 + b) mod 1019;
    if b > 300 then a := a + 2 * b; else a := a - 2 * b; end if;
    b := (a * 11 + b) mod 1021;
    return (a + b) mod 1031;
  end;
  type box is protected
    procedure add(n : integer);
    impure function get(n : integer) return integer;
  end protected;
  type box is protected body
    variable v : integer := 0;
    procedure add(n : integer) is
      variable a, b : integer;
    begin
      a := (n * 3 + v) mod 1009;
      b := (a * 5 + n) mod 1013;
      if b > 500 then a := a + b; else a := a - b; end if;
      b := (a * 7 + b) mod 1019;
      if b > 300 then a := a + 2 * b; else a := a - 2 * b; end if;
      v := (v + a + b) mod 65521;
    end;
    impure function get(n : integer) return integer is
      variable a, b : integer;
    begin
      a := (n * 3 + v) mod 1009;
      b := (a * 5 + n) mod 1013;
      if b > 500 then a := a + b; else a := a - b; end if;
      b := (a * 7 + b) mod 1019;
      if b > 300 then a := a + 2 * b; else a := a - 2 * b; end if;
      return (v + a + b) mod 65521;
    end;
  end protected body;
  shared variable b : box;
  impure function through(x : integer) return integer is
    variable a : integer;
  begin
    a := (x * 3 + 1) mod 1009;
    if a > 500 then a := a + x; else a := a - x; end if;
    b.add(a);
    a := (a + b.get(a)) mod 1013;
    if a > 300 then a := a + 2 * x; else a := a - 2 * x; end if;
    return (a * 7 + x) mod 1019;
  end;
  signal clk : bit := '0';
begin
  clk <= not clk after 5 ns;
  process
    variable x : integer := 1;
  begin
    for i in 1 to 300 loop
      wait until clk = '1';
      x := plain(x) + through(x);
    end loop;
    report integer'image(x);
    std.env.finish;
  end process;
end;
";

/// The count reaches the process and every path that calls: a plain call, a method of a protected
/// object, which the prototype of this rule never counted: its callers then left to the interpreter
/// at every call (2.76 times the instructions of the window generator).
#[test]
fn the_count_asks_for_a_process_and_what_it_calls() {
    let dir = setup("auto_count_calls", CALLS);
    let o = counted(&dir, "counted", "20", &[]);
    for f in ["plain", "through", "add", "get"] {
        assert!(compiled(&o, f), "`{f}` was not compiled:\n{}", err(&o));
    }
    assert!(counter(&o, "count_asked") >= 5, "{}", err(&o));
    assert!(counter(&o, "count_compiled") >= 5, "{}", err(&o));
}

/// No frames left to the interpreter once a callee has its code: a callee that is never asked for
/// leaves one per call for the rest of the run.
#[test]
fn compiled_callers_stay_in_native_code() {
    let dir = setup("auto_count_native", CALLS);
    let o = counted(&dir, "counted", "20", &[]);
    let m = counter(&o, "materialised");
    assert!(m < 100, "{m} native frames were left to the interpreter");
}

const COPIES: &str =
    "entity tick is generic (K : natural := 1); port (clk : in bit; q : out natural); end;
architecture a of tick is
begin
  process (clk)
    variable n : natural := 0;
  begin
    if clk = '1' then
      n := (n * 3 + K) mod 1009;
      q <= n;
    end if;
  end process;
end;
entity copies is end;
architecture a of copies is
  signal clk : bit := '0';
  signal q1, q2, q3 : natural;
begin
  clk <= not clk after 5 ns;
  u1 : entity work.tick generic map (1) port map (clk, q1);
  u2 : entity work.tick generic map (2) port map (clk, q2);
  u3 : entity work.tick generic map (3) port map (clk, q3);
  process
  begin
    wait for 5 us;
    report integer'image(q1) & integer'image(q2) & integer'image(q3);
    std.env.finish;
  end process;
end;
";

/// A process with instance data switches to its copy after half the count, and the copy, not the
/// shared code, is what is compiled after the other half.
#[test]
fn a_process_switches_to_a_copy_and_the_copy_compiles() {
    let dir = setup("auto_count_copies", COPIES);
    let o = counted(&dir, "copies", "40", &[]);
    assert_eq!(counter(&o, "spec_switched"), 3, "{}", err(&o));
    assert_eq!(counter(&o, "count_copies"), 3, "{}", err(&o));
}

/// At a design's size: 24 instances, each with its own copy, none lost on the way. (The copy-slot bug
/// lost them without a change in the output.)
#[test]
fn every_instance_has_its_copy_compiled() {
    let src = COPIES
        .replace("signal q1, q2, q3 : natural;", "type qs is array (1 to 24) of natural;\n  signal q : qs;")
        .replace(
            "u1 : entity work.tick generic map (1) port map (clk, q1);\n  u2 : entity work.tick generic map (2) port map (clk, q2);\n  u3 : entity work.tick generic map (3) port map (clk, q3);",
            "g : for i in 1 to 24 generate\n    u : entity work.tick generic map (i * 3) port map (clk, q(i));\n  end generate;",
        )
        .replace("integer'image(q1) & integer'image(q2) & integer'image(q3)", "integer'image(q(1)) & integer'image(q(24))");
    let dir = setup("auto_count_many", &src);
    let o = counted(&dir, "copies", "40", &[("YAVS_SPEC_CAP", "0")]);
    assert_eq!(counter(&o, "spec_switched"), 24, "{}", err(&o));
    assert_eq!(
        counter(&o, "count_copies"),
        counter(&o, "spec_copies"),
        "{}",
        err(&o)
    );
    assert!(counter(&o, "spec_copies") >= 1, "{}", err(&o));
    // Where a cap stops the copies, the shared code is what the count asks for.
    let o = counted(&dir, "copies", "40", &[("YAVS_SPEC_CAP", "1")]);
    assert_eq!(counter(&o, "spec_switched"), 0, "{}", err(&o));
    assert_eq!(counter(&o, "count_compiled"), 1, "{}", err(&o));
}

/// Half the count: the switch and the copy's own count follow it.
#[test]
fn the_switch_comes_at_half_the_count() {
    let dir = setup("auto_count_half", COPIES);
    // Five us are 1000 rising and falling edges of the clock, so 1000 activations of a process.
    let late = counted(&dir, "copies", "1500", &[]);
    assert_eq!(counter(&late, "spec_switched"), 3, "{}", err(&late));
    assert_eq!(counter(&late, "count_copies"), 0, "{}", err(&late));
    let never = counted(&dir, "copies", "3000", &[]);
    assert_eq!(counter(&never, "spec_switched"), 0, "{}", err(&never));
}

const COPY_CALLS: &str =
    "entity many is generic (K : natural := 1); port (clk : in bit; q : out natural); end;
architecture a of many is
  function mix(x : integer) return integer is
    variable a, b : integer;
  begin
    a := (x * 3 + K) mod 1009;
    b := (a * 5 + x) mod 1013;
    if b > 500 then a := a + b; else a := a - b; end if;
    b := (a * 7 + b) mod 1019;
    if b > 300 then a := a + 2 * b; else a := a - 2 * b; end if;
    b := (a * 11 + b) mod 1021;
    return (a + b) mod 1031;
  end;
begin
  process (clk)
    variable n : integer := 0;
  begin
    if clk = '1' then
      for i in 1 to 10 loop
        n := mix(n + i + K);
      end loop;
      q <= n;
    end if;
  end process;
end;
entity copy_calls is end;
architecture a of copy_calls is
  signal clk : bit := '0';
  signal q : natural;
begin
  clk <= not clk after 5 ns;
  u : entity work.many generic map (2) port map (clk, q);
  process
  begin
    wait for 2 us;
    report integer'image(q);
    std.env.finish;
  end process;
end;
";

/// A function whose count ran out long before the only process that calls it has a copy: the hold
/// ends when the copy is compiled, though it was not there when the callers were worked out.
#[test]
fn a_callee_is_freed_by_a_compiled_copy() {
    let dir = setup("auto_count_copy_calls", COPY_CALLS);
    let o = counted(&dir, "copy_calls", "40", &[]);
    assert_eq!(counter(&o, "spec_switched"), 1, "{}", err(&o));
    assert_eq!(counter(&o, "count_copies"), 1, "{}", err(&o));
    assert!(counter(&o, "count_held") >= 1, "{}", err(&o));
    assert!(compiled(&o, "mix"), "{}", err(&o));
}

const LOOPS: &str = "entity one is generic (K : natural := 3); end;
architecture a of one is
begin
  process
    variable x : integer := K;
  begin
    for i in 1 to 40000 loop
      x := (x * 3 + i) mod 65521;
    end loop;
    report integer'image(x);
    wait for 1 ns;
    report integer'image(x + K);
    std.env.finish;
  end process;
end;
";

/// A long loop in a process with one activation and instance data, which would have a copy: the count
/// was waiting for the copy's own, which never comes (`compute_h2h`, `xor_sig`, `xor_types` lost 5 to
/// 8 times so under the plain count). Back edges count, and such a process is asked for at once.
#[test]
fn a_long_loop_in_a_process_entered_once_is_asked_for() {
    let dir = setup("auto_count_loop", LOOPS);
    let o = counted(&dir, "one", "20", &[]);
    assert!(counter(&o, "count_compiled") >= 1, "{}", err(&o));
    assert_eq!(counter(&o, "spec_switched"), 0, "{}", err(&o));
}

/// A gate no test reaches: everything waits, nothing is lost and the output stays.
#[test]
fn nothing_compiles_by_the_count_before_the_gate() {
    let dir = setup("auto_count_gate", CALLS);
    let o = counted(&dir, "counted", "20", &[("YAVS_AUTO_GATE", "100000000000")]);
    assert_eq!(counter(&o, "count_compiled"), 0, "{}", err(&o));
    assert!(counter(&o, "count_held") >= 1, "{}", err(&o));
}

const LEAVES: &str = "entity leaves is end;
architecture a of leaves is
  type ints is array (0 to 3) of integer;
  signal s : ints;
  procedure drive_it(signal t : out ints; x : integer) is
    variable v : ints;
  begin
    v := (x, x + 1, x + 2, x + 3);
    t <= v;
  end;
  signal clk : bit := '0';
begin
  clk <= not clk after 5 ns;
  process
    variable x : integer := 1;
  begin
    for i in 1 to 300 loop
      wait until clk = '1';
      drive_it(s, i);
      x := (x + s(1)) mod 1009;
    end loop;
    report integer'image(x);
    std.env.finish;
  end process;
end;
";

/// A native frame that leaves at every call: the frames are counted, and whether the guard takes it
/// back at any moment (a share that any frame reaches; when depends on the looks) the output stays.
#[test]
fn native_code_that_leaves_every_time_is_counted() {
    let dir = setup("auto_count_leaves", LEAVES);
    let o = counted(&dir, "leaves", "20", &[("YAVS_AUTO_GUARD_SHARE", "0")]);
    assert!(counter(&o, "materialised") >= 200, "{}", err(&o));
    counted(
        &dir,
        "leaves",
        "20",
        &[("YAVS_AUTO_GUARD_SHARE", "1000000000")],
    );
}

/// What the cache lists is a link: a warm run does not wait for the gate for it, and with it switched
/// on again (`YAVS_AUTO_WARM_GATE`) waits.
#[test]
fn what_the_cache_lists_does_not_wait_for_the_gate() {
    let dir = setup("auto_count_warm", CALLS);
    let run = |env: &[(&str, &str)]| {
        let o = yavs(
            &dir,
            env,
            &[
                "run",
                "--std=2008",
                "--work=work:work",
                "--threads=1",
                "--tier=clift-auto",
                "counted",
            ],
        );
        assert!(o.status.success(), "{o:?}");
        o
    };
    let open = [
        ("YAVS_AUTO_COUNT", "20"),
        ("YAVS_AUTO_GATE", "0"),
        ("YAVS_AUTO_SYNC", "1"),
    ];
    for _ in 0..3 {
        run(&open);
    }
    let shut = [
        ("YAVS_AUTO_COUNT", "20"),
        ("YAVS_AUTO_GATE", "100000000000"),
        ("YAVS_AUTO_SYNC", "1"),
    ];
    let warm = run(&shut);
    assert!(counter(&warm, "count_compiled") >= 5, "{}", err(&warm));
    let mut held = shut.to_vec();
    held.push(("YAVS_AUTO_WARM_GATE", "1"));
    assert_eq!(counter(&run(&held), "count_compiled"), 0);
}

/// A gate no test reaches keeps the copies back too, up to the activation they would come at without
/// the rule (`spec_after`): a short run does not pay for what it would not get back.
#[test]
fn a_short_run_makes_no_copies() {
    let dir = setup("auto_count_no_copies", COPIES);
    let o = counted(&dir, "copies", "40", &[("YAVS_AUTO_GATE", "100000000000")]);
    assert_eq!(counter(&o, "spec_switched"), 0, "{}", err(&o));
}

const POW: &str = "entity pow is end;
architecture a of pow is
  type rows is array (natural range <>) of bit_vector;
  function pw(x, n : integer) return integer is
    variable a, b : integer;
  begin
    a := (x * 3 + 1) mod 1009;
    b := (a * 5 + x) mod 1013;
    if b > 500 then a := a + b; else a := a - b; end if;
    b := (a * 7 + b) mod 1019;
    if b > 300 then a := a + 2 * b; else a := a - 2 * b; end if;
    return ((a + b) mod 7 + 2) ** n;
  end;
  function bits(r : rows) return integer is
    variable t : rows(r'range)(r'element'range);
    variable n : integer := 0;
  begin
    t := r;
    t(t'left)(0) := not t(t'left)(0);
    for i in t'range loop
      if t(i)(0) = '1' then
        n := n + t(i)'length;
      end if;
    end loop;
    return n;
  end;
  signal clk : bit := '0';
begin
  clk <= not clk after 5 ns;
  process
    variable x : integer := 1;
    variable m : rows(0 to 2)(0 to 3);
  begin
    for i in 1 to 200 loop
      wait until clk = '1';
      x := (x + pw(i, 3) + bits(m)) mod 1009;
    end loop;
    report integer'image(x);
    std.env.finish;
  end process;
end;
";

/// `**` and a layout from a run-time descriptor have native-frame code under the rule (nothing to
/// leave for at every call), and the old policy keeps the old refusal.
#[test]
fn the_two_instructions_run_on_native_frames() {
    let dir = setup("auto_count_arms", POW);
    let o = counted(&dir, "pow", "20", &[("YAVS_CLIFT_DUMP", "1")]);
    let e = err(&o);
    assert!(compiled(&o, "pw"), "{e}");
    assert!(!e.contains("no native variant of"), "{e}");
    assert!(
        e.lines()
            .any(|l| l.contains("native variant") && l.ends_with(" pw")),
        "{e}"
    );
    assert!(
        e.lines()
            .any(|l| l.contains("native variant") && l.ends_with(" bits")),
        "{e}"
    );
    let eager = yavs(
        &dir,
        &[("YAVS_CLIFT_DUMP", "1"), ("YAVS_AUTO_POLICY", "sampled")],
        &[
            "run",
            "--std=2008",
            "--work=work:work",
            "--no-cache",
            "--tier=clift",
            "pow",
        ],
    );
    let e = err(&eager);
    for f in ["pw", "bits"] {
        assert!(
            e.lines()
                .any(|l| l.starts_with("no native variant of") && l.contains(&format!(" {f}: "))),
            "{e}"
        );
    }
}

/// The old policy is one switch away: no count, the switch to a copy at activation 1024.
#[test]
fn the_old_policy_counts_nothing() {
    let dir = setup("auto_count_old", COPIES);
    let o = counted(&dir, "copies", "20", &[("YAVS_AUTO_POLICY", "sampled")]);
    assert_eq!(counter(&o, "count_asked"), 0, "{}", err(&o));
    assert_eq!(counter(&o, "spec_switched"), 0, "{}", err(&o));
}
