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
    run_top(dir, flags, "drives")
}

fn run_top(dir: &Path, flags: &[&str], top: &str) -> (String, Vec<String>) {
    let mut args = vec!["run", "--work=work:work"];
    args.extend_from_slice(flags);
    args.push(top);
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
        #[cfg(feature = "clift")]
        vec!["--tier=clift"],
        #[cfg(feature = "clift")]
        vec!["--tier=auto3", "--tier-heat=1,0,0"],
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

/// Element-wise drives of vectors in loops: kept and handed over together in compiled code. The
/// order against vector drives, delayed drives, waits and a second driver must be the
/// interpreter's, and a run longer than the buffer (40 elements) must work.
const BATCHES: &str = r#"library ieee; use ieee.std_logic_1164.all;
entity batches is end;
architecture a of batches is
  type tab_t is array (0 to 8) of std_logic;
  constant tab : tab_t := ('U', 'X', '0', '1', 'Z', 'W', 'L', 'H', '-');
  signal clk : std_logic := '0';
  signal sh : std_logic_vector(7 downto 0) := "00000001";
  signal mix : std_logic_vector(7 downto 0) := (others => '0');
  signal big : std_logic_vector(39 downto 0) := (others => '0');
  signal r : std_logic_vector(3 downto 0);
  signal w : std_logic_vector(3 downto 0) := "0000";
  signal d : std_logic_vector(3 downto 0) := "0000";
  signal u : std_ulogic := '0';
  signal t : std_logic_vector(3 downto 0) := "0000";
  signal cnt : natural := 0;
  procedure set1 (signal s : out std_logic_vector; i : natural) is
  begin
    s(i) <= '1';
  end procedure;
begin
  p1: process (clk) begin
    if rising_edge(clk) then
      for i in 7 downto 1 loop sh(i) <= sh(i - 1); end loop;
      sh(0) <= sh(7);
      cnt <= cnt + 1;
    end if;
  end process;
  p2: process (clk) begin
    if rising_edge(clk) then
      for i in 0 to 7 loop mix(i) <= '1'; end loop;
      mix <= "10101010";
      for i in 0 to 3 loop mix(i) <= tab((cnt + i) mod 9); end loop;
      mix(5) <= '0';
      u <= tab(cnt mod 9);
    end if;
  end process;
  p3: process (clk) begin
    if rising_edge(clk) then
      for i in big'range loop big(i) <= tab((i + cnt) mod 9); end loop;
    end if;
  end process;
  pa: process (clk) begin
    if rising_edge(clk) then
      for i in 0 to 3 loop
        if i mod 2 = 0 then r(i) <= tab((cnt + i) mod 9); else r(i) <= 'Z'; end if;
      end loop;
    end if;
  end process;
  pb: process (clk) begin
    if rising_edge(clk) then
      for i in 0 to 3 loop r(i) <= tab((cnt * 2 + i) mod 9); end loop;
    end if;
  end process;
  p6: process (clk) begin
    if rising_edge(clk) then
      for i in 0 to 3 loop
        t(i) <= '0';
        if cnt mod 3 = 0 then set1(t, i); end if;
        if i = 2 then report "mid " & to_string(cnt); end if;
        t(i) <= tab((cnt + 2 * i) mod 9);
        if cnt mod 4 = 1 then set1(t, i); end if;
      end loop;
    end if;
  end process;
  p4: process begin
    wait until rising_edge(clk);
    for i in 0 to 3 loop
      w(i) <= '1';
      wait for 1 ns;
      w(i) <= '0';
    end loop;
  end process;
  p5: process (clk) begin
    if rising_edge(clk) then
      for i in 0 to 3 loop
        d(i) <= '1';
        d(i) <= '0' after 2 ns;
        d(i) <= tab((cnt + i) mod 9) after 3 ns;
      end loop;
      d(0) <= '1';
    end if;
  end process;
  process begin
    for i in 1 to 40 loop
      clk <= '1'; wait for 5 ns; clk <= '0'; wait for 5 ns;
    end loop;
    wait;
  end process;
  process (clk) begin
    if falling_edge(clk) then
      report integer'image(cnt) & " " & to_string(sh) & " " & to_string(mix) & " " & to_string(big)
        & " " & to_string(r) & " " & to_string(w) & " " & to_string(d) & " " & std_ulogic'image(u) & " " & to_string(t);
    end if;
  end process;
end;
"#;

#[test]
fn element_drives_in_loops_match_the_interpreter() {
    let dir = tempdir("compiled_drive_batches");
    std::fs::write(dir.join("batches.vhd"), BATCHES).expect("write");
    let a = yavs(
        &dir,
        &["analyze", "--std=2008", "--work=work:work", "batches.vhd"],
    );
    assert!(a.status.success(), "{a:?}");
    let (want, _) = run_top(&dir, &["--tier=interp"], "batches");
    assert!(want.lines().count() >= 40, "{want}");
    for flags in [
        vec!["--tier=llvm"],
        vec!["--tier=auto", "--tier-heat=1,0,0"],
        vec!["--tier=auto", "--tier-heat=1,0,0", "--threads=4"],
    ] {
        let (got, _) = run_top(&dir, &flags, "batches");
        assert_eq!(got, want, "{flags:?}");
    }
}
