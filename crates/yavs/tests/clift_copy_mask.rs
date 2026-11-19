//! A process copy (`YAVS_SPEC_ALL`) whose compiled code tests whether a vector copy changes the
//! driven value (`Gen::value_differs`): the mask of the elements at widths 1, 63 and 64, where 63
//! once overflowed in its making (an internal error in debug builds).
#![cfg(feature = "clift")]

mod common;
use common::tempdir;

use std::process::Command;

const DESIGN: &str = r#"
library ieee;
use ieee.std_logic_1164.all;
entity mask_unit is
  generic (W : positive);
  port (clk : in std_ulogic; d : in std_ulogic_vector(W - 1 downto 0));
end;
architecture a of mask_unit is
  signal q : std_ulogic_vector(W - 1 downto 0) := (others => '0');
  signal n : natural := 0;
begin
  process (clk)
  begin
    if rising_edge(clk) then
      q <= d;
      n <= n + 1;
    end if;
  end process;
  process (n)
  begin
    if n = 30 then
      report "W=" & integer'image(W) & " q=" & to_hstring(q);
    end if;
  end process;
end;

library ieee;
use ieee.std_logic_1164.all;
entity mask_top is end;
architecture a of mask_top is
  signal clk : std_ulogic := '0';
  signal d : std_ulogic_vector(127 downto 0) := (others => '0');
  signal k : natural := 0;
begin
  clk <= not clk after 5 ns when k < 64;
  process (clk)
  begin
    if falling_edge(clk) then
      k <= k + 1;
      if k mod 3 /= 0 then
        d(k mod 64) <= not d(k mod 64);
      end if;
    end if;
  end process;
  u1 : entity work.mask_unit generic map (1) port map (clk, d(0 downto 0));
  u63 : entity work.mask_unit generic map (63) port map (clk, d(62 downto 0));
  u64 : entity work.mask_unit generic map (64) port map (clk, d(63 downto 0));
end;
"#;

fn yavs(dir: &std::path::Path, env: &[(&str, &str)], args: &[&str]) -> (i32, String, String) {
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
fn a_copied_vector_drive_of_63_elements_compiles_and_runs_as_interpreted() {
    let d = tempdir("clift_copy_mask");
    std::fs::write(d.join("m.vhd"), DESIGN).expect("write");
    let work = format!("work:{}", d.join("lib").display());
    let a = yavs(
        &d,
        &[],
        &["analyze", "--std=2008", "--work", &work, "m.vhd"],
    );
    assert_eq!(a.0, 0, "{}", a.2);
    let run = |tier: &str, env: &[(&str, &str)]| {
        let base = [
            "run",
            "--std=2008",
            "--work",
            &work,
            "--no-cache",
            "--threads",
            "1",
        ];
        let mut args: Vec<&str> = base.to_vec();
        args.extend(tier.split(' '));
        args.push("mask_top");
        yavs(&d, env, &args)
    };
    let want = run("--tier=interp", &[]);
    assert_eq!(want.0, 0, "{}", want.2);
    assert_eq!(want.1.matches("q=").count(), 3, "{}", want.1);
    let spec = [("YAVS_SPEC_ALL", "1"), ("YAVS_SPEC_AFTER", "2")];
    let got = run("--tier=clift-auto --tier-heat=1,0,0", &spec);
    assert_eq!((got.0, &got.1), (0, &want.1), "{}", got.2);
}
