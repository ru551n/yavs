//! The facts of a process copy (`yavs_ir::InstFacts`): compiled code that assumes each signal's
//! form gives what the interpreter gives, for every form, through force and multi-source updates.
//! Two instances with equal generics and another form of one signal are two copies.
#![cfg(feature = "clift")]

mod common;
use common::tempdir;

use std::path::Path;
use std::process::Command;

const DESIGN: &str = r#"
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
package p is
  type color is (red, green, blue);
end package;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.p.all;
entity reader is
  generic (g : natural);
  port (
    clk, d : in std_logic;
    v : in std_logic_vector(7 downto 0);
    w : in std_logic_vector(127 downto 0);
    i : in integer;
    c : in color;
    r : in real;
    q : out std_logic;
    qv : out std_logic_vector(7 downto 0);
    qw : out std_logic_vector(127 downto 0);
    qi : out integer);
end entity;

architecture a of reader is
begin
  process (clk)
    variable sum : natural := 0;
    variable n : natural := 0;
  begin
    if rising_edge(clk) then
      n := n + 1;
      sum := (sum * 3 + g + std_logic'pos(d) + to_integer(unsigned(v))) mod 1000003;
      sum := (sum + std_logic'pos(w(g)) + to_integer(unsigned(w(g + 7 downto g)))) mod 1000003;
      sum := (sum + to_integer(unsigned(w(g + 80 downto g + 73)))) mod 1000003;
      sum := (sum + (i mod 1000) + color'pos(c) + integer(r) + std_logic'pos(v(g mod 8))) mod 1000003;
      if v'event or w'event then
        sum := (sum + 1) mod 1000003;
      end if;
      q <= d xor v(0);
      qv <= v(3 downto 0) & w(g + 3 downto g);
      qw <= w(126 downto 0) & d;
      qi <= sum;
      if n mod 16 = 0 then
        report integer'image(g) & " " & integer'image(sum);
      end if;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.p.all;
entity top is
end entity;

architecture a of top is
  signal clk, d, q1, q2, q3 : std_logic := '0';
  signal v, qv1, qv2, qv3 : std_logic_vector(7 downto 0) := x"00";
  -- Two wide sources: a bus kept as planes only. `bus_in` is driven by the stimulus and `qw3`.
  signal bus_in : std_logic_vector(127 downto 0) := (others => 'Z');
  signal w, qw1, qw2, qw3 : std_logic_vector(127 downto 0) := (others => '0');
  signal i, qi1, qi2, qi3 : integer := 0;
  signal c : color := red;
  signal r : real := 0.0;
begin
  clk <= not clk after 5 ns;
  u1 : entity work.reader generic map (2)
    port map (clk, d, v, w, i, c, r, q1, qv1, qw1, qi1);
  -- The same generics; `d` is part of a wide bus, which keeps no codes: another form.
  u2 : entity work.reader generic map (2)
    port map (clk, bus_in(5), v, w, i, c, r, q2, qv2, qw2, qi2);
  u3 : entity work.reader generic map (9)
    port map (clk, q1, qv1, qw1, qi1, c, r, q3, qv3, qw3, qi3);
  bus_in <= qw3 when c = blue else (others => 'Z');
  process
    variable k : natural := 0;
  begin
    wait until falling_edge(clk);
    k := k + 1;
    d <= not d;
    v <= std_logic_vector(to_unsigned(k * 37 mod 256, 8));
    w <= std_logic_vector(rotate_left(unsigned(w), 3)) xor (127 downto 8 => '0') & v;
    i <= i + k;
    c <= color'val(k mod 3);
    r <= real(k) / 2.0;
    if c /= blue then
      bus_in <= (others => '1');
    else
      bus_in <= (others => 'Z');
    end if;
    if k = 20 then
      v <= force x"5a";
      w <= force (others => '1');
      i <= force 77;
    elsif k = 26 then
      v <= release;
      w <= release;
      i <= release;
    elsif k = 70 then
      report "end " & integer'image(qi1) & " " & integer'image(qi2) & " " & integer'image(qi3);
      std.env.finish;
    end if;
  end process;
end architecture;
"#;

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

#[test]
fn facts_change_no_result_and_split_copies_by_form() {
    let d = tempdir("compiled_facts");
    std::fs::write(d.join("t.vhd"), DESIGN).expect("write");
    let work = format!("work:{}", d.join("lib").display());
    let a = yavs(
        &d,
        &[],
        &["analyze", "--std=2008", "--work", &work, "t.vhd"],
    );
    assert_eq!(a.0, 0, "{}", a.2);
    let run = |env: &[(&str, &str)], tier: &[&str], threads: &str| {
        let mut args = vec!["run", "--std=2008", "--no-cache", "--work", &work];
        args.extend(tier);
        args.extend(["--threads", threads, "top"]);
        yavs(&d, env, &args)
    };
    let interp = run(&[], &["--tier=interp"], "1");
    assert_eq!(interp.0, 0, "{}", interp.2);
    assert!(interp.1.contains("end ") || interp.2.contains("end "));
    let spec = ("YAVS_SPEC_AFTER", "2");
    for tier in [
        &["--tier=auto", "--tier-heat=1,0,0"][..],
        &["--tier=clift-auto", "--tier-heat=1,0,0"][..],
    ] {
        for threads in ["1", "4"] {
            let got = run(&[spec], tier, threads);
            assert_eq!(
                (&interp.0, &interp.1, &interp.2),
                (&got.0, &got.1, &got.2),
                "{tier:?} --threads {threads}"
            );
        }
    }
    // Facts make one more copy of `reader`'s process for the instance with the other form.
    let copies = |env: &[(&str, &str)]| {
        let (_, _, e) = run(env, &["--tier=auto", "--tier-heat=1,0,0"], "1");
        e.lines()
            .find_map(|l| l.strip_prefix("counters: spec_copies="))
            .expect("the counter")
            .parse::<u64>()
            .expect("a number")
    };
    let on = copies(&[spec, ("YAVS_COUNTERS", "1")]);
    let off = copies(&[spec, ("YAVS_COUNTERS", "1"), ("YAVS_NO_FACTS", "1")]);
    assert_eq!(on, off + 1, "copies with facts {on}, without {off}");
}
