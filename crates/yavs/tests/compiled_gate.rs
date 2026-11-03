//! The compare of a drive with the current value in Cranelift code (`TierFrame::gate`): a skip made
//! with no call gives what the interpreter gives, around everything that makes the kernel not skip.
//! Debug builds ask the kernel's full test at every such skip (`yavs_rt_gate_check`) and abort on
//! a difference; a stale gate would otherwise only show as a wrong value later.
#![cfg(feature = "clift")]

mod common;
use common::tempdir;

use std::path::Path;
use std::process::Command;

const DESIGN: &str = r#"
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity unit is
  generic (g : natural);
end entity;

architecture a of unit is
  type color is (red, green, blue);
  signal clk : std_logic := '0';
  signal n : natural := 0;

  -- Held values of each form, with a change now and then.
  signal s1 : std_logic := '0';
  signal v8 : std_logic_vector(7 downto 0) := x"00";
  signal v70 : std_logic_vector(69 downto 0) := (others => '0');
  signal w100 : std_logic_vector(99 downto 0) := (others => '0');
  signal i1 : integer := 0;
  signal c1 : color := red;
  signal r1 : real := 0.0;
  signal e8 : unsigned(7 downto 0) := x"00";

  -- The order of drives in one run.
  signal o1, o2, o3 : std_logic := '0';
  signal ov : std_logic_vector(7 downto 0) := x"00";

  -- Delays on a direct driver.
  signal d1, d2, d3, d4 : std_logic := '0';
  signal dv : std_logic_vector(3 downto 0) := x"0";
  signal kp : std_logic_vector(7 downto 0) := x"00";

  -- The fields of a record: one kernel driver, a driver of the process each.
  type pair is record
    a, b : std_logic;
    c : std_logic_vector(3 downto 0);
  end record;
  signal rc : pair := ('0', '0', x"0");
  signal rw : pair := ('0', '0', x"0");
  signal hv : std_logic_vector(7 downto 0) := x"00";
  signal hi : integer := 0;
  signal kq, kr, ks, kt : std_logic_vector(7 downto 0) := x"00";
  signal di : integer := 0;

  -- Force and release of a signal with a direct driver.
  signal f1 : std_logic := '0';
  signal fv : std_logic_vector(7 downto 0) := x"00";
  signal fi : integer := 0;

  -- Resolved, several sources.
  signal rs : std_logic := 'Z';
  signal rv : std_logic_vector(3 downto 0) := "ZZZZ";

  -- Disjoint parts of a vector, one process each.
  signal part : std_logic_vector(7 downto 0) := x"00";
begin
  clk <= not clk after 5 ns;

  count : process (clk)
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      n <= n + 1;
    end if;
  end process;

  held : process (clk)
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      if n mod 16 = 3 then
        s1 <= not s1;
        v8 <= std_logic_vector(unsigned(v8) + 1);
        v70(69 downto 66) <= v8(3 downto 0);
        w100 <= w100(98 downto 0) & not w100(99);
        i1 <= i1 + 1;
        c1 <= color'val((color'pos(c1) + 1) mod 3);
        r1 <= r1 + 0.5;
        e8 <= e8 + 1;
      else
        s1 <= s1;
        v8 <= v8;
        v70(69 downto 66) <= v70(69 downto 66);
        w100 <= w100;
        i1 <= i1;
        c1 <= c1;
        r1 <= r1;
        e8 <= e8;
      end if;
    end if;
  end process;

  orders : process (clk)
    variable k : natural := 0;
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      k := n mod 8;
      -- The value held, then another: the second wins.
      o1 <= o1;
      if n mod 5 = 1 then
        o1 <= not o1;
      end if;
      -- Another value, then the one held: the second wins, nothing changes.
      if n mod 7 = 2 then
        o2 <= not o2;
      end if;
      o2 <= o2;
      -- An element at a run-time position, then a constant one of the same driver.
      ov(k) <= '1';
      ov(3) <= ov(3);
      ov(k) <= not ov(k);
      if n mod 9 = 4 then
        ov(3) <= not ov(3);
      end if;
      -- A held value after a change of another driver of the process.
      o3 <= '1' when n mod 11 = 5 else '0';
    end if;
  end process;

  -- One way of giving a direct driver a waveform per process: each shuts the gate for good, so a
  -- site that forgets to is seen by the zero-delay assignment of the held value after it.
  quick_delay : process (clk)
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      if n mod 13 = 1 then
        d1 <= not d1 after 12 ns;
      elsif n mod 13 = 2 then
        d1 <= d1;
      end if;
    end if;
  end process;

  transport_delay : process (clk)
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      if n mod 6 = 1 then
        d2 <= transport not d2 after 17 ns;
      elsif n mod 6 = 2 then
        d2 <= transport d2;
      end if;
    end if;
  end process;

  reject_limit : process (clk)
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      if n mod 17 = 3 then
        d3 <= reject 2 ns inertial not d3 after 25 ns;
      elsif n mod 17 = 4 then
        d3 <= d3;
      end if;
    end if;
  end process;

  two_elements : process (clk)
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      if n mod 23 = 5 then
        d4 <= '1' after 13 ns, '0' after 27 ns;
      elsif n mod 23 = 6 then
        d4 <= d4;
      end if;
    end if;
  end process;

  vector_delay : process (clk)
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      if n mod 19 = 7 then
        dv <= not dv after 23 ns;
      elsif n mod 19 = 8 then
        dv <= dv;
      end if;
    end if;
  end process;

  byte_delay : process (clk)
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      if n mod 29 = 9 then
        di <= di + 1 after 23 ns;
      elsif n mod 29 = 10 then
        di <= di;
      end if;
    end if;
  end process;

  -- A field that changes, then the held value of another field of the same kernel driver.
  record_fields : process (clk)
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      if n mod 3 = 0 then
        rc.c <= not rc.c;
      end if;
      rc.a <= rc.a;
      rc.b <= rc.b;
      rc.c <= rc.c;
    end if;
  end process;

  -- The whole record, then a field held, in one run.
  whole_record : process (clk)
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      if n mod 4 = 1 then
        rw <= (not rw.a, rw.b, not rw.c);
      end if;
      rw.b <= rw.b;
      rw.a <= rw.a;
    end if;
  end process;

  -- Two drivers of the process on one kernel driver: an entry through one closes the other.
  one_kernel_driver : process (clk)
    variable k : natural := 0;
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      k := n mod 8;
      if n mod 5 = 9 then
        kq(3) <= '0';
      end if;
      kq(k) <= not kq(k);
      kq(3) <= kq(3);
      kr(k) <= not kr(k);
      kr(3) <= kr(3);
      if n mod 5 = 9 then
        kr(k) <= '0';
      end if;
      ks(3 downto 2) <= not ks(3 downto 2);
      ks(5 downto 4) <= ks(5 downto 4);
      kt(5 downto 4) <= not kt(5 downto 4);
      kt(3 downto 2) <= kt(3 downto 2);
      if n mod 5 = 9 then
        ks(k) <= '0';
        kt(k) <= '0';
      end if;
    end if;
  end process;

  -- A change through the host, then the value held of the same driver: the entry stays first.
  host_entries : process (clk)
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      if n mod 7 = 2 then
        hv <= not hv;
        hi <= hi + 1;
      end if;
      hv <= hv;
      hi <= hi;
    end if;
  end process;

  -- A kept drive at a run-time position, then the held value of a constant element of the same
  -- driver: at position 3 the second assignment wins, though the kernel has not seen the first.
  kept_position : process (clk)
    variable k : natural := 0;
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      k := n mod 8;
      kp(k) <= '1';
      kp(3) <= kp(3);
    end if;
  end process;

  owner : process (clk)
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      -- Held by the driver; `cur` differs from the driving value while forced below.
      if n = 45 then
        -- The forced value: `cur` holds it, the driver does not. Must not be skipped.
        f1 <= '1';
        fv <= x"5a";
        fi <= 77;
      elsif n = 60 then
        f1 <= '0';
        fv <= x"00";
        fi <= 0;
      else
        f1 <= f1;
        fv <= fv;
        fi <= fi;
      end if;
    end if;
  end process;

  forcer : process
  begin
    wait until n = 40;
    f1 <= force '1';
    fv <= force x"5a";
    fi <= force 77;
    wait until n = 50;
    f1 <= release;
    fv <= release;
    fi <= release;
    wait until n = 55;
    f1 <= force '1';
    fv <= force x"5a";
    fi <= force 77;
    wait until n = 57;
    f1 <= release;
    fv <= release;
    fi <= release;
    wait;
  end process;

  res1 : process (clk)
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      rs <= 'Z';
      rv <= "ZZZZ";
      if n mod 4 = 0 then
        rs <= '1';
        rv <= "01ZZ";
      end if;
    end if;
  end process;

  res2 : process (clk)
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      rs <= '0';
      rv <= "ZZ10";
      if n mod 4 = 1 then
        rs <= 'Z';
        rv <= "ZZZZ";
      end if;
    end if;
  end process;

  part_lo : process (clk)
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      part(3 downto 0) <= part(3 downto 0);
      if n mod 10 = 6 then
        part(3 downto 0) <= std_logic_vector(unsigned(part(3 downto 0)) + 1);
      end if;
    end if;
  end process;

  part_hi : process (clk)
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      part(7) <= part(7);
      part(6 downto 4) <= part(6 downto 4);
      if n mod 10 = 8 then
        part(7) <= not part(7);
      end if;
    end if;
  end process;

  -- Every value at every event, as the results to compare.
  watch : process (clk'delayed(1 ns))
  begin
    if clk'delayed(1 ns) = '1' then
      report integer'image(g) & " " & integer'image(n) & " " & std_logic'image(s1) & to_hstring(v8) & to_hstring(v70)
        & to_hstring(w100) & integer'image(i1) & color'image(c1) & real'image(r1)
        & to_hstring(e8) & " " & std_logic'image(o1) & std_logic'image(o2) & std_logic'image(o3)
        & to_hstring(ov) & " " & std_logic'image(d1) & std_logic'image(d2)
        & std_logic'image(d3) & std_logic'image(d4) & to_hstring(dv) & integer'image(di) & to_hstring(kp) & std_logic'image(rc.a)
        & std_logic'image(rc.b) & to_hstring(rc.c) & to_hstring(hv) & integer'image(hi) & to_hstring(kq) & to_hstring(kr) & to_hstring(ks) & to_hstring(kt) & std_logic'image(rw.a) & std_logic'image(rw.b)
        & to_hstring(rw.c)
        & " " & std_logic'image(f1) & to_hstring(fv) & integer'image(fi)
        & " " & std_logic'image(rs) & to_hstring(rv) & " " & to_hstring(part);
      if n = 150 then
        std.env.finish;
      end if;
    end if;
  end process;
end architecture;

entity top is
end entity;

architecture a of top is
begin
  -- A process reads a generic: it has a copy per instance, and the facts of the copy.
  u0 : entity work.unit generic map (0);
  u1 : entity work.unit generic map (1);
end architecture;
"#;

/// A run that only assigns values held keeps its delta: the report of the postponed process names
/// it, so a skip the kernel does not hear of moves the number.
const DELTAS: &str = r#"
library ieee;
use ieee.std_logic_1164.all;
entity unit is
  generic (g : natural);
end entity;
architecture a of unit is
  signal clk, x : std_logic := '0';
  signal v : std_logic_vector(3 downto 0) := x"0";
  signal t : natural := 0;
begin
  clk <= not clk after 5 ns;
  process (clk)
  begin
    if rising_edge(clk) then
      t <= t + 1;
      if t = 60 then
        std.env.finish;
      end if;
    end if;
  end process;
  -- Reads what changes, so the idle skip leaves it alone, and only assigns what is held.
  process (clk)
  begin
    if falling_edge(clk) then
      if g > 1000 or t < 0 then
        report "g";
      end if;
      x <= x;
      v <= v;
    end if;
  end process;
  postponed process (clk)
  begin
    if clk = '0' then
      report "falling " & integer'image(t);
    end if;
  end postponed process;
end architecture;
entity top is
end entity;
architecture a of top is
begin
  u0 : entity work.unit generic map (0);
end architecture;
"#;

/// A process of 70 drivers: those past the gate's 64 bits take the host's way, and a kept drive of
/// one stops a skip of none of the others wrongly.
fn many_drivers() -> String {
    let n = 70;
    let mut s = String::from(
        "library ieee;\nuse ieee.std_logic_1164.all;\nentity unit is\n  generic (g : natural);\nend entity;\narchitecture a of unit is\n  signal clk : std_logic := '0';\n  signal t : natural := 0;\n",
    );
    for k in 0..n {
        s += &format!("  signal x{k} : std_logic := '0';\n");
    }
    s += "begin\n  clk <= not clk after 5 ns;\n  process (clk)\n  begin\n    if rising_edge(clk) then\n      if g > 1000 then report \"g\"; end if;\n";
    for k in 0..n {
        s += &format!(
            "      x{k} <= x{k};\n      if t mod {} = 3 then\n        x{k} <= not x{k};\n      end if;\n",
            k + 2
        );
    }
    s += "      t <= t + 1;\n    end if;\n  end process;\n  process (clk'delayed(1 ns))\n  begin\n    if clk'delayed(1 ns) = '1' then\n      report integer'image(t)";
    for k in 0..n {
        s += &format!(" & std_logic'image(x{k})");
    }
    s += ";\n      if t = 80 then\n        std.env.finish;\n      end if;\n    end if;\n  end process;\nend architecture;\nentity top is\nend entity;\narchitecture a of top is\nbegin\n  u0 : entity work.unit generic map (0);\n  u1 : entity work.unit generic map (1);\nend architecture;\n";
    s
}

fn yavs(dir: &Path, env: &[(&str, &str)], args: &[&str]) -> (i32, String, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .current_dir(dir)
        .envs(env.iter().copied())
        .args(["--color", "never"])
        .args(args)
        .output()
        .expect("run yavs");
    (
        o.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

/// Runs `text` at every tier against the interpreter; the number of skips the check saw on one thread.
fn against_interp(name: &str, text: &str) -> u64 {
    let d = tempdir(name);
    std::fs::write(d.join("t.vhd"), text).expect("write");
    let work = format!("work:{}", d.join("lib").display());
    let a = yavs(
        &d,
        &[],
        &["analyze", "--std=2008", "--work", &work, "t.vhd"],
    );
    assert_eq!(a.0, 0, "{}", a.2);
    let run = |env: &[(&str, &str)], tier: &[&str], threads: &str| {
        let mut args = vec!["run", "--std=2008", "--work", &work];
        args.extend(tier);
        args.extend(["--threads", threads, "top"]);
        yavs(&d, env, &args)
    };
    let interp = run(&[], &["--tier=interp", "--no-cache"], "1");
    assert_eq!(interp.0, 0, "{}", interp.2);
    let spec = ("YAVS_SPEC_AFTER", "2");
    let check = ("YAVS_INLCMP", "check");
    let heat = ["--tier=clift-auto", "--tier-heat=1,0,0"];
    let cold = ["--tier=clift-auto", "--tier-heat=1,0,0", "--no-cache"];
    let auto = ["--tier=clift-auto", "--no-cache"];
    for (tier, threads) in [
        (&cold[..], "1"),
        (&cold[..], "4"),
        (&auto[..], "1"),
        // Written to the cache, then linked from it: the code holds no address of this run.
        (&heat[..], "1"),
        (&heat[..], "1"),
        (&heat[..], "4"),
    ] {
        let got = run(&[spec, check], tier, threads);
        assert_eq!(
            (&interp.0, &interp.1, &interp.2),
            (&got.0, &got.1, &got.2),
            "{tier:?} --threads {threads}"
        );
    }
    // The runs above stored the code in the work library: this one links it and still skips.
    let counters = ("YAVS_COUNTERS", "1");
    let (_, _, e) = run(&[spec, check, counters], &heat, "1");
    assert!(
        e.contains("counters: gate_checked="),
        "a warm run skipped nothing: {e}"
    );
    let (_, _, e) = run(&[spec, check, counters], &cold, "1");
    e.lines()
        .find_map(|l| l.strip_prefix("counters: gate_checked="))
        .map_or(0, |n| n.parse().expect("a number"))
}

#[test]
fn skips_in_compiled_code_change_no_result() {
    let hits = against_interp("compiled_gate", DESIGN);
    assert!(hits > 100, "the compare took only {hits} skips");
}

#[test]
fn a_run_of_skips_keeps_its_delta() {
    let hits = against_interp("compiled_gate_deltas", DELTAS);
    assert!(hits > 50, "the compare took only {hits} skips");
}

#[test]
fn a_process_of_more_drivers_than_the_gate_has_bits() {
    let hits = against_interp("compiled_gate_many", &many_drivers());
    assert!(hits > 100, "the compare took only {hits} skips");
}
