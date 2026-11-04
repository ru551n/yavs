//! Signal reads in compiled code against the interpreter: a read of a constant element is fixed at
//! compile time except for one test of its codes (#1058), so every storage form has to come out the
//! same through that test: codes, planes only (one body with instances of both forms), bytes of
//! each size, an element of a port collapsed into part of its actual, a resolved signal, a force
//! and a release, `'event`, and a read that follows an element index in a register.
#![cfg(feature = "clift")]

mod common;
use common::tempdir;

use std::path::Path;
use std::process::{Command, Output};

const SRC: &str = r#"library ieee; use ieee.std_logic_1164.all;
entity leaf is
  generic (w : positive; tag : string);
  port (clk : in std_ulogic; d : in std_ulogic_vector(w - 1 downto 0); one : in std_ulogic);
end;
architecture a of leaf is
begin
  -- The same body for instances whose `d` keeps codes and whose `d` does not.
  process (clk) begin
    if clk = '1' then
      report tag & " " & std_ulogic'image(d(0)) & std_ulogic'image(d(1))
        & std_ulogic'image(d(w - 1)) & std_ulogic'image(one);
    end if;
  end process;
end;

library ieee; use ieee.std_logic_1164.all;
entity reads is end;
architecture a of reads is
  type st_t is (idle, run, done);
  signal clk : std_ulogic := '0';
  signal sl : std_logic := 'Z';
  signal su : std_ulogic := 'U';
  signal v8 : std_ulogic_vector(7 downto 0) := x"5a";
  -- More than 64 elements driven by a source of more than 16: no codes, planes only.
  signal wide : std_ulogic_vector(99 downto 0) := (others => '0');
  signal small : std_ulogic_vector(3 downto 0) := "0101";
  signal part : std_ulogic_vector(15 downto 0) := x"a5c3";
  signal e : st_t := idle;
  signal b : boolean := false;
  signal bb : bit := '0';
  signal c : character := 'a';
  signal i : integer := -7;
  signal t : time := 3 ns;
  signal r : real := 1.5;
  signal s : std_ulogic := '0';
  signal fv : std_ulogic := '0';
begin
  sl <= '1' after 12 ns, '0' after 32 ns;
  sl <= 'Z', '0' after 22 ns, 'Z' after 42 ns;
  l0 : entity work.leaf generic map (100, "wide") port map (clk, wide, su);
  l1 : entity work.leaf generic map (4, "small") port map (clk, small, sl);
  -- Port collapsed into part of its actual: elements start at its offset.
  l2 : entity work.leaf generic map (4, "part") port map (clk, part(11 downto 8), part(3));

  process begin
    for k in 1 to 12 loop
      clk <= '1'; wait for 5 ns;
      clk <= '0';
      su <= '1'; v8 <= v8(6 downto 0) & v8(7);
      wide <= wide(98 downto 0) & not wide(99);
      small <= small(2 downto 0) & small(3);
      part <= part(14 downto 0) & part(15);
      e <= st_t'val((st_t'pos(e) + 1) mod 3);
      b <= not b; bb <= not bb; c <= character'succ(c);
      i <= i * 3 + 1; t <= t + 1 ns; r <= r * 1.5;
      s <= not s;
      wait for 5 ns;
    end loop;
    wait;
  end process;

  -- Force, release and a deposit-like update must be what the next read sees.
  process begin
    wait for 26 ns; fv <= force '1';
    wait for 20 ns; fv <= release;
    wait for 20 ns; fv <= '1';
    wait;
  end process;

  process (clk)
    variable n : integer := 3;
    variable x : std_ulogic;
  begin
    if clk = '1' then
      x := v8(n);
      report std_ulogic'image(sl) & std_ulogic'image(su) & std_ulogic'image(v8(0))
        & std_ulogic'image(v8(7)) & std_ulogic'image(x) & std_ulogic'image(wide(0))
        & std_ulogic'image(wide(99)) & std_ulogic'image(wide(n)) & std_ulogic'image(fv);
      report st_t'image(e) & " " & boolean'image(b) & " " & bit'image(bb) & " " & character'image(c)
        & " " & integer'image(i) & " " & time'image(t) & " " & real'image(r)
        & " " & boolean'image(s'event) & " " & boolean'image(clk'event);
      n := (n + 1) mod 8;
    end if;
  end process;
end;
"#;

fn run(dir: &Path, flags: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yavs"))
        .current_dir(dir)
        .args(["run", "--std=2008", "--work=work:out/work", "--no-cache"])
        .args(flags)
        .arg("reads")
        .env("YAVS_STD_CACHE", "off")
        .output()
        .unwrap()
}

#[test]
fn every_form_reads_the_same_as_the_interpreter() {
    let dir = tempdir("compiled_sigreads");
    std::fs::write(dir.join("r.vhd"), SRC).unwrap();
    let out = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .current_dir(&dir)
        .args(["analyze", "--std=2008", "--work=work:out/work", "r.vhd"])
        .env("YAVS_STD_CACHE", "off")
        .output()
        .unwrap();
    assert!(out.status.success(), "{out:?}");
    let want = run(&dir, &["--tier=interp", "--threads=1"]);
    assert!(want.status.success(), "{want:?}");
    let text =
        String::from_utf8_lossy(&want.stderr).into_owned() + &String::from_utf8_lossy(&want.stdout);
    assert!(
        text.contains("wide") && text.contains("part") && text.contains("run"),
        "{text}"
    );
    for flags in [
        &["--tier=clift"][..],
        &["--tier=clift", "--threads=4"],
        &["--tier=clift-auto", "--tier-heat=1,0,0"],
    ] {
        let got = run(&dir, flags);
        assert_eq!(
            (&got.stdout, &got.stderr, got.status.code()),
            (&want.stdout, &want.stderr, want.status.code()),
            "{flags:?}"
        );
    }
    // Shuffled, the processes of a delta report in another order; the lines stay the same.
    let sorted = |o: &[u8]| {
        let mut l: Vec<String> = String::from_utf8_lossy(o)
            .lines()
            .map(String::from)
            .collect();
        l.sort();
        l
    };
    let got = run(&dir, &["--tier=clift", "--threads=4", "--shuffle"]);
    assert_eq!(sorted(&got.stdout), sorted(&want.stdout));
    assert_eq!(got.status.code(), want.status.code());
}
