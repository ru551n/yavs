//! Scalar logic assignments in compiled code (`--tier=llvm`, `--tier=auto` compiling everything at
//! once; a build with the `llvm` feature): the fused drive (`yavs_interp::scalar_drives`, the
//! waveform never made) against the interpreter, for every shape of assignment: all nine values,
//! zero and positive delays, inertial, transport and reject, a resolved signal with two drivers,
//! a computed element of a vector, a signal assigned in a loop and in a procedure; reads of integer, enumeration and time signals.
#![cfg(feature = "llvm")]

mod common;
use common::tempdir;

use std::path::Path;
use std::process::{Command, Output};

const SRC: &str = r#"library ieee; use ieee.std_logic_1164.all;
entity drives is end;
architecture a of drives is
  type tab_t is array (0 to 8) of std_logic;
  constant tab : tab_t := ('U', 'X', '0', '1', 'Z', 'W', 'L', 'H', '-');
  signal clk : std_logic := '0';
  signal s0, s1, s2, s3, s4, s5, s6 : std_logic := 'U';
  signal u1 : std_ulogic := 'U';
  signal r : std_logic;
  signal v : std_logic_vector(3 downto 0) := "0000";
  signal b : bit := '0';
  signal en : boolean := false;
  signal idx : natural range 0 to 8 := 0;
  signal cnt : integer := 0;
  type st_t is (idle, run, done_st);
  signal st : st_t := idle;
  signal ia : integer_vector(0 to 3) := (others => 0);
  signal tm : time := 0 ns;
  function isum (v : integer_vector) return integer is
    variable r : integer := 0;
  begin
    for k in v'range loop r := r + v(k) * (k + 1); end loop;
    return r;
  end function;
  procedure put (signal s : out std_logic; x : std_logic) is
  begin
    s <= x;
  end procedure;
begin
  p0: process (clk) begin
    if rising_edge(clk) then s0 <= tab(idx); idx <= (idx + 1) mod 9; cnt <= cnt + 1; end if;
  end process;
  p1: process (clk) begin
    if rising_edge(clk) then s1 <= tab(idx) after 3 ns; end if;
  end process;
  p2: process (clk) begin
    if rising_edge(clk) then s2 <= transport tab((idx + 3) mod 9) after 7 ns; end if;
  end process;
  p3: process (clk) begin
    if rising_edge(clk) then s3 <= reject 2 ns inertial tab((idx + 5) mod 9) after 4 ns; end if;
  end process;
  p4: process (clk) begin
    if rising_edge(clk) then r <= tab(idx); end if;
  end process;
  p5: process (clk) begin
    if falling_edge(clk) then r <= tab((idx + 4) mod 9); end if;
  end process;
  p6: process (clk) begin
    if rising_edge(clk) then v(idx mod 4) <= tab(idx); end if;
  end process;
  p7: process (clk) begin
    if rising_edge(clk) then b <= not b; u1 <= tab((idx + 1) mod 9); end if;
  end process;
  p8: process (clk) begin
    if rising_edge(clk) then put(s6, tab((idx + 2) mod 9)); end if;
  end process;
  p9: process (clk) begin
    if rising_edge(clk) then
      ia(idx mod 4) <= cnt * 3 - 7;
      if idx = 4 then st <= run; elsif idx = 8 then st <= done_st; else st <= idle; end if;
      tm <= tm + 3 ns;
    end if;
  end process;
  s4 <= s0 xor s1;
  s5 <= '1' when en else s2;
  process begin
    for i in 1 to 60 loop
      clk <= '1'; wait for 5 ns; clk <= '0'; wait for 5 ns;
      if i = 30 then en <= true; end if;
    end loop;
    wait;
  end process;
  process (clk) begin
    if falling_edge(clk) then
      report integer'image(cnt) & " " & std_logic'image(s0) & std_logic'image(s1) & std_logic'image(s2)
        & std_logic'image(s3) & std_logic'image(s4) & std_logic'image(s5) & std_logic'image(s6)
        & " " & std_logic'image(r) & " " & to_string(v) & " " & bit'image(b) & " " & std_ulogic'image(u1)
        & " " & st_t'image(st) & " " & integer'image(isum(ia))
        & " " & time'image(tm);
    end if;
  end process;
end;
"#;

fn yavs(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(args)
        .env("YAVS_COUNTERS", "kinds")
        .current_dir(dir)
        .output()
        .expect("run yavs")
}

/// The notes of a run and its `cb_<entry>.<Kind>` counter lines.
fn run(dir: &Path, flags: &[&str]) -> (String, Vec<String>) {
    let mut args = vec!["run", "--work=work:work"];
    args.extend_from_slice(flags);
    args.push("drives");
    let o = yavs(dir, &args);
    assert!(o.status.success(), "{o:?}");
    let census = String::from_utf8_lossy(&o.stderr)
        .lines()
        .filter(|l| l.starts_with("counters: cb_") && l.contains('.'))
        .map(str::to_string)
        .collect();
    (String::from_utf8_lossy(&o.stdout).into_owned(), census)
}

#[test]
fn scalar_logic_assignments_in_compiled_code_match_the_interpreter() {
    let dir = tempdir("compiled_drives");
    std::fs::write(dir.join("drives.vhd"), SRC).expect("write");
    let a = yavs(
        &dir,
        &["analyze", "--std=2008", "--work=work:work", "drives.vhd"],
    );
    assert!(a.status.success(), "{a:?}");
    let (want, _) = run(&dir, &["--tier=interp"]);
    assert!(want.lines().count() >= 60, "{want}");
    for flags in [
        vec!["--tier=llvm"],
        vec!["--tier=auto", "--tier-heat=1,0,0"],
        vec!["--tier=auto", "--tier-heat=1,0,0", "--threads=4"],
    ] {
        let (got, census) = run(&dir, &flags);
        assert_eq!(got, want, "{flags:?}");
        // The fast path is the whole story for a zero delay: no assignment of a delay-free
        // process goes through the generic form (the delayed ones and the procedure's do).
        let slow: u64 = census
            .iter()
            .filter(|l| l.starts_with("counters: cb_drive_bit."))
            .filter_map(|l| l.rsplit('=').next()?.parse::<u64>().ok())
            .sum();
        assert!(slow <= 3 * 60 + 60, "{flags:?}: {census:?}");
    }
}
