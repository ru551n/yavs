//! Tests that ran on the LLVM tier before it was removed (user, 2026-11-08), on the Cranelift
//! tier: each design runs on the interpreter and on `--tier=clift`, `--tier=clift-auto` compiling
//! at the first entry (`--tier-heat=1,0,0`) and the same on four threads, and every tier prints
//! what the interpreter prints, exit status and messages included. Where the design is about a
//! path compiled code takes by itself, the host-call counters (`YAVS_COUNTERS=kinds`, debug
//! builds) say it did.
#![cfg(feature = "clift")]

mod common;
use common::tempdir;

use std::path::Path;
use std::process::{Command, Output};

/// The compiled tiers: all at once, at the first entry, and the same on four threads.
const TIERS: [&[&str]; 3] = [
    &["--tier=clift"],
    &["--tier=clift-auto", "--tier-heat=1,0,0"],
    &["--tier=clift-auto", "--tier-heat=1,0,0", "--threads=4"],
];

fn yavs(dir: &Path, env: &[(&str, &str)], args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(args)
        .envs(env.iter().copied())
        .current_dir(dir)
        .output()
        .expect("run yavs")
}

/// A directory holding `src` (file `d.vhd`), analysed into library `work`.
fn design(name: &str, src: &str) -> common::Tmp {
    let dir = tempdir(name);
    std::fs::write(dir.join("d.vhd"), src).expect("write");
    let a = yavs(
        &dir,
        &[],
        &["analyze", "--std=2008", "--work=work:work", "d.vhd"],
    );
    assert!(a.status.success(), "{a:?}");
    dir
}

/// What a run printed: exit status, standard output, standard error without the counters, and the
/// `cb_` counters (the calls compiled code made into the host, in all and by entry and
/// instruction kind as `cb_<entry>.<Kind>`).
#[derive(PartialEq, Debug)]
struct Run {
    code: Option<i32>,
    out: String,
    err: String,
    census: Vec<String>,
}

fn run(dir: &Path, top: &str, flags: &[&str], extra: &[&str]) -> Run {
    let mut args = vec!["run", "--std=2008", "--work=work:work"];
    args.extend_from_slice(flags);
    args.extend_from_slice(extra);
    args.push(top);
    let o = yavs(dir, &[("YAVS_COUNTERS", "kinds")], &args);
    let err = String::from_utf8_lossy(&o.stderr);
    let census = err
        .lines()
        .filter(|l| l.starts_with("counters: cb_"))
        .map(str::to_string)
        .collect();
    let err = err
        .lines()
        .filter(|l| !l.starts_with("counters: "))
        .collect::<Vec<_>>()
        .join("\n");
    Run {
        code: o.status.code(),
        out: String::from_utf8_lossy(&o.stdout).into_owned(),
        err,
        census,
    }
}

/// The interpreter's run of `top` and the run of every compiled tier, which must print the same.
fn parity(dir: &Path, top: &str, extra: &[&str]) -> (Run, Vec<Run>) {
    let want = run(dir, top, &["--tier=interp"], extra);
    let got = TIERS
        .iter()
        .map(|flags| {
            let got = run(dir, top, flags, extra);
            assert_eq!(
                (got.code, &got.out, &got.err),
                (want.code, &want.out, &want.err),
                "{flags:?} {extra:?}"
            );
            got
        })
        .collect();
    (want, got)
}

/// The sum of the counters whose line contains `pat` (0 in a release build, which counts none).
fn calls(r: &Run, pat: &str) -> u64 {
    (r.census.iter())
        .filter(|l| l.contains(pat))
        .filter_map(|l| l.rsplit('=').next()?.parse::<u64>().ok())
        .sum()
}

/// A debug build counts the calls of compiled code; a release build counts none.
fn counted(r: &Run) -> bool {
    !cfg!(debug_assertions) || !r.census.is_empty()
}

/// A sensitivity list of array slices and scalars, a variable of generic width in the body.
const WAITS: &str = r#"library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity waits is generic (N : natural := 8); end;
architecture a of waits is
  signal a, b, c : std_logic_vector(N-1 downto 0) := (others => '0');
  signal d, e : std_logic := '0';
  signal q : std_logic_vector(N-1 downto 0) := (others => '0');
  signal cnt : natural := 0;
begin
  process (a(N-1 downto 0), b(N-1 downto 1), c, d, e)
    variable x : std_logic_vector(N-1 downto 0);
  begin
    x := a xor b xor c;
    if d = '1' then x := not x; end if;
    q <= x;
    if e = '1' then x := x and a; end if;
    q <= q xor x;
  end process;
  process begin
    for i in 1 to 2000 loop
      a <= std_logic_vector(to_unsigned(i, N)); wait for 1 ns;
      b <= std_logic_vector(to_unsigned(i*3, N)); c <= not a; d <= not d;  e <= not e; wait for 1 ns;
    end loop;
    report "q=" & to_hstring(q);
    wait;
  end process;
end;
"#;
#[test]
fn sensitivity_lists_wait_without_naming_their_signals() {
    let dir = design("clift_waits", WAITS);
    let (want, got) = parity(&dir, "waits", &[]);
    assert!(want.out.contains("q="), "{want:?}");
    for r in got {
        assert!(counted(&r), "{r:?}");
        // 4000 activations of the process, 5 signals in its list: the waits are not named again
        // (the first ones, before the kernel holds a wait, are).
        assert!(calls(&r, "counters: cb_wait_on=") < 100, "{:?}", r.census);
    }
}

const INSTANCES: &str = r#"entity cell is
  generic (seed : integer);
  port (clk : in bit; sum : out integer);
end;
architecture a of cell is
  type ints_t is array (natural range <>) of integer;
  type ints_ptr_t is access ints_t;
  constant tab : ints_t(0 to 7) := (3, 1, 4, 1, 5, 9, 2, 6);
begin
  process (clk)
    variable buf : ints_ptr_t := new ints_t'(0 to 7 => 0);
    variable aux : ints_ptr_t := new ints_t'(0 to 3 => 0);
    variable n   : integer := 0;
    variable acc : integer := seed;
  begin
    if clk = '1' then
      n := n + 1;
      buf(n mod 8) := (buf((n + 3) mod 8) + seed * tab(n mod 8) + n) mod 100003;
      aux(n mod 4) := (aux(n mod 4) + buf(n mod 8)) mod 1009;
      -- Every few edges the heap changes: the old objects go, new ones take their slots.
      if n mod (3 + seed) = 0 then
        deallocate(aux);
        aux := new ints_t'(seed, n, seed + n, 7);
      end if;
      acc := (acc + buf(n mod 8) + aux(n mod 4)) mod 1000003;
      sum <= acc;
    end if;
  end process;
end;

entity instances is
end;
architecture a of instances is
  type sums_t is array (1 to 4) of integer;
  signal sums : sums_t;
  signal clk : bit := '0';
begin
  cells : for k in 1 to 4 generate
    u : entity work.cell generic map (seed => k) port map (clk => clk, sum => sums(k));
  end generate;
  process
  begin
    for i in 1 to 200 loop
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
    end loop;
    for k in sums'range loop
      report "sum " & integer'image(k) & " " & integer'image(sums(k));
    end loop;
    wait;
  end process;
end;
"#;
#[test]
fn instances_keep_their_own_objects_in_compiled_code() {
    let dir = design("clift_instances", INSTANCES);
    let (want, _) = parity(&dir, "instances", &[]);
    assert_eq!(want.code, Some(0), "{want:?}");
    assert_eq!(want.out.matches("sum ").count(), 4, "{want:?}");
}

const DRIVES: &str = r#"library ieee; use ieee.std_logic_1164.all;
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
#[test]
fn scalar_logic_assignments_in_compiled_code_match_the_interpreter() {
    let dir = design("clift_drives", DRIVES);
    let (want, got) = parity(&dir, "drives", &[]);
    assert!(want.out.lines().count() >= 60, "{want:?}");
    for r in got {
        // The fast path is the whole story for a zero delay: no assignment of a delay-free
        // process goes through the generic form (the delayed ones and the procedure's do).
        assert!(counted(&r), "{r:?}");
        assert!(calls(&r, "cb_drive_bit.") <= 3 * 60 + 60, "{:?}", r.census);
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
    let dir = design("clift_drive_batches", BATCHES);
    let (want, _) = parity(&dir, "batches", &[]);
    assert!(want.out.lines().count() >= 40, "{want:?}");
}

/// One process per idiom, counting its edges in `n`; the stimulus drives the clock through
/// `X`, `U`, `L` and `H` as well, and resets and a second clock in between. Slot of each form:
/// 1 `rising_edge`, 2 `clk'event and clk = '1'`, 3 `falling_edge`, 4 and 5 the same behind an
/// asynchronous reset, 6 and 7 on a `std_ulogic`, 8 on a `bit`, 9 two edges ored, 10 `wait until
/// rising_edge`, 11 `wait until clk = '1'`, 12 `clk'event and clk = '0'`, 13 `not clk'stable and
/// clk = '1'`, 14 and 15 `rising_edge` and `falling_edge` of a BIT, 16 a process of two signals
/// that tests `clk = '1'` alone (no edge).
const EDGES: &str = r#"library ieee; use ieee.std_logic_1164.all;
entity edges is end;
architecture a of edges is
  signal clk : std_logic := '0';
  signal cu : std_ulogic := 'U';
  signal cb : bit := '0';
  signal rst : std_logic := '1';
  type cnt_t is array (1 to 16) of integer;
  signal n : cnt_t := (others => 0);
begin
  process begin
    wait for 3 ns; rst <= '0';
    for i in 1 to 40 loop
      clk <= '1'; cb <= '1'; wait for 5 ns;
      case i mod 5 is
        when 0 => clk <= 'X'; wait for 1 ns; clk <= '1'; wait for 4 ns; clk <= 'L'; cu <= 'H'; wait for 5 ns; clk <= 'H'; cu <= '0'; wait for 5 ns;
        when 1 => clk <= 'U'; wait for 1 ns; clk <= '0'; wait for 4 ns;
        when 2 => rst <= '1'; clk <= '0'; cb <= '0'; wait for 5 ns; rst <= '0'; wait for 1 ns;
        when others => clk <= '0'; cb <= '0'; wait for 5 ns;
      end case;
      cu <= '1'; wait for 1 ns; cu <= '0';
    end loop;
    wait for 10 ns;
    for k in n'range loop report integer'image(k) & " = " & integer'image(n(k)); end loop;
    wait;
  end process;
  p1: process(clk) begin if rising_edge(clk) then n(1) <= n(1) + 1; end if; end process;
  p2: process(clk) begin if clk'event and clk = '1' then n(2) <= n(2) + 1; end if; end process;
  p3: process(clk) begin if falling_edge(clk) then n(3) <= n(3) + 1; end if; end process;
  p4: process(clk, rst) begin if rst = '1' then n(4) <= 0; elsif rising_edge(clk) then n(4) <= n(4) + 1; end if; end process;
  p5: process(clk, rst) begin if rst = '1' then n(5) <= 100; elsif falling_edge(clk) then n(5) <= n(5) + 1; end if; end process;
  p6: process(cu) begin if rising_edge(cu) then n(6) <= n(6) + 1; end if; end process;
  p7: process(cu) begin if falling_edge(cu) then n(7) <= n(7) + 1; end if; end process;
  p8: process(cb) begin if cb'event and cb = '1' then n(8) <= n(8) + 1; end if; end process;
  p9: process(clk, cu) begin if rising_edge(clk) or falling_edge(cu) then n(9) <= n(9) + 1; end if; end process;
  p10: process begin wait until rising_edge(clk); n(10) <= n(10) + 1; end process;
  p11: process begin wait until clk = '1'; n(11) <= n(11) + 1; end process;
  p12: process(clk) begin if clk'event and clk = '0' then n(12) <= n(12) + 1; end if; end process;
  p13: process(clk) begin if not clk'stable and clk = '1' then n(13) <= n(13) + 1; end if; end process;
  p14: process(cb) begin if rising_edge(cb) then n(14) <= n(14) + 1; end if; end process;
  p15: process(cb) begin if falling_edge(cb) then n(15) <= n(15) + 1; end if; end process;
  p16: process(clk, cu) begin if clk = '1' then n(16) <= n(16) + 1; end if; end process;
end;
"#;
/// The edge counts a run printed: (slot, count).
fn edge_counts(r: &Run) -> Vec<(u32, u32)> {
    (r.out.lines())
        .filter_map(|l| {
            let (k, n) = l.strip_prefix("** Note: ")?.split_once(" = ")?;
            Some((k.rsplit(' ').next()?.parse().ok()?, n.trim().parse().ok()?))
        })
        .collect()
}

#[test]
fn clock_edge_forms_in_compiled_code_match_the_interpreter() {
    let dir = design("clift_edges", EDGES);
    let (want, got) = parity(&dir, "edges", &[]);
    let want = edge_counts(&want);
    assert_eq!(want.len(), 16, "{want:?}");
    let n = |k: usize| want[k - 1].1;
    // The forms that mean the same count the same; `rising_edge` is stricter than the event
    // form (an `X` or `U` before the `1` is no rising edge).
    assert_eq!(n(1), n(10), "{want:?}");
    assert_eq!(n(2), n(11), "{want:?}");
    // `not clk'stable` is `clk'event`; `rising_edge` of a BIT is the event ending in '1'.
    assert_eq!(n(2), n(13), "{want:?}");
    assert_eq!(n(8), n(14), "{want:?}");
    assert!(n(1) < n(2) && n(1) > 0 && n(3) > 0 && n(15) > 0, "{want:?}");
    for r in got {
        assert_eq!(edge_counts(&r), want);
        // The edge tests and `'event` are answered in compiled code (the `Drive` and `SigReadScalar`
        // calls left are the integer counters).
        assert!(counted(&r), "{r:?}");
        for l in &r.census {
            assert!(
                !l.contains(".SigAttr=") && !l.contains(".CallNative=") && !l.contains(".Edge="),
                "{:?}",
                r.census
            );
        }
    }
}

const ARRAYS: &str = r#"entity arrays is
  generic (bad : integer := 0);
end;
architecture a of arrays is
  type rec_t is record
    x, y : integer;
    c    : character;
  end record;
  type rec_vec_t is array (integer range <>) of rec_t;
  type rec_ptr_t is access rec_vec_t;
  type ints_t is array (integer range <>) of integer;
  type int_ptr_t is access ints_t;
  type tab_t is array (integer range <>) of int_ptr_t;
  type tab_ptr_t is access tab_t;

  procedure sum_up (variable p : inout int_ptr_t; variable s : inout integer) is
  begin
    for i in p'range loop s := s + p(i) * (i + 3); end loop;
  end procedure;

  procedure sum_rec (variable p : inout rec_ptr_t; variable s : inout integer) is
  begin
    for i in p'range loop s := s + p(i).x * 7 + p(i).y - character'pos(p(i).c); end loop;
  end procedure;

  procedure at_index (variable p : inout int_ptr_t; i : integer; variable s : inout integer) is
  begin
    s := s + p(i);
  end procedure;

  procedure via_table (variable t : inout tab_ptr_t; r, i : integer; variable s : inout integer) is
  begin
    s := s - t(r)(i);
  end procedure;
begin
  process
    variable up    : int_ptr_t := new ints_t(5 to 14);
    variable down  : int_ptr_t := new ints_t(9 downto 0);
    variable lone  : int_ptr_t := new ints_t(-3 to -3);
    variable empty : int_ptr_t := new ints_t(1 to 0);
    variable emptd : int_ptr_t := new ints_t(0 downto 3);
    variable recs  : rec_ptr_t := new rec_vec_t(-2 to 6);
    variable rdown : rec_ptr_t := new rec_vec_t(4 downto 1);
    variable t     : tab_ptr_t := new tab_t(0 to 2);
    variable acc   : integer := 0;
  begin
    for i in up'range loop up(i) := i * 5 - 20; end loop;
    for i in down'range loop down(i) := 100 - i * i; end loop;
    lone(-3) := 42;
    for i in recs'range loop recs(i) := (x => i, y => i * i, c => character'val(65 + (i + 2))); end loop;
    for i in rdown'range loop rdown(i) := (x => -i, y => 3 * i, c => character'val(97 + i)); end loop;
    t(0) := up; t(1) := down; t(2) := lone;
    for round in 1 to 40 loop
      sum_up(up, acc); sum_up(down, acc); sum_up(lone, acc); sum_up(empty, acc); sum_up(emptd, acc);
      sum_rec(recs, acc); sum_rec(rdown, acc);
      for i in 5 to 14 loop at_index(up, i, acc); via_table(t, 0, i, acc); end loop;
      for i in 0 to 9 loop at_index(down, i, acc); via_table(t, 1, i, acc); end loop;
      at_index(lone, -3, acc); via_table(t, 2, -3, acc);
    end loop;
    report "sum " & integer'image(acc);
    if bad = 1 then at_index(up, 15, acc); end if;
    if bad = 2 then at_index(down, -1, acc); end if;
    if bad = 3 then at_index(empty, 1, acc); end if;
    if bad = 4 then at_index(emptd, 3, acc); end if;
    if bad = 5 then sum_rec(recs, acc); at_index(up, rdown(5).x, acc); end if;
    if bad = 6 then via_table(t, 3, 0, acc); end if;
    if bad = 7 then at_index(up, 2**30, acc); end if;
    if bad = 8 then at_index(up, -(2**30), acc); end if;
    report "end " & integer'image(acc);
    wait;
  end process;
end;
"#;
#[test]
fn element_access_in_compiled_code_matches_the_interpreter() {
    let dir = design("clift_arrays", ARRAYS);
    for bad in 0..=8 {
        let g = format!("-gbad={bad}");
        let (want, _) = parity(&dir, "arrays", &[&g]);
        assert!(want.out.contains("sum "), "{want:?}");
        assert_eq!(want.code == Some(0), bad == 0, "bad {bad}: {want:?}");
    }
}

const NATIVES: &str = r#"library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity t is end;
architecture a of t is
begin
  process
    variable u : unsigned(3 downto 0) := "1011";
    variable m : unsigned(3 downto 0) := "10X1";
    variable n : integer := 0;
    variable s : string(1 to 3);
  begin
    for i in 0 to 5 loop
      n := n + to_integer(u + i);
      report "i=" & integer'image(i) & " n=" & integer'image(n) & " u=" & integer'image(to_integer(u));
    end loop;
    s := "a" & character'image('b')(2) & "c";
    report s & " " & natural'image(s'length);
    n := to_integer(m);
    report "after the warning " & integer'image(n);
    wait;
  end process;
end;
"#;
#[test]
fn natives_in_compiled_code_match_the_interpreter() {
    let dir = design("clift_natives", NATIVES);
    for severity in ["failure", "warning"] {
        let (want, _) = parity(&dir, "t", &[&format!("--exit-severity={severity}")]);
        assert!(want.out.contains("i=5 n="), "{want:?}");
        // The warning is the last thing that happens at the stop severity.
        assert_eq!(
            want.out.contains("after the warning"),
            severity == "failure",
            "{want:?}"
        );
    }
}

/// Light `ieee` natives called from functions (native frames) and from a process (frame mode):
/// vector results that grow the temporary area many times over, a metavalue warning inside a
/// function, and the stop severity.
const LIGHT: &str = r#"library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity l is end;
architecture a of l is
  function step(u : unsigned; k : natural) return natural is
    variable w : unsigned(u'length - 1 downto 0) := u;
  begin
    for i in 1 to 40 loop
      w := (w + k) xor (w srl 1);
    end loop;
    return to_integer(w(7 downto 0));
  end function;
  function bad(m : unsigned) return integer is
  begin
    return to_integer(m) + 1;
  end function;
begin
  process
    variable u : unsigned(31 downto 0) := x"12345678";
    variable m : unsigned(3 downto 0) := "1X01";
    variable n : integer := 0;
  begin
    for i in 0 to 300 loop
      n := n + step(u, i);
      u := u + to_unsigned(n, 32);
    end loop;
    report "n=" & integer'image(n) & " u=" & to_string(u);
    n := bad(m);
    report "after the warning " & integer'image(n);
    wait;
  end process;
end;
"#;
#[test]
fn light_natives_in_compiled_code_match_the_interpreter() {
    let dir = design("clift_light", LIGHT);
    for severity in ["failure", "warning"] {
        let (want, _) = parity(&dir, "l", &[&format!("--exit-severity={severity}")]);
        assert!(want.out.contains("n="), "{want:?}");
        assert_eq!(
            want.out.contains("after the warning"),
            severity == "failure",
            "{want:?}"
        );
    }
}

/// `numeric_std` operators and conversions computed in place by compiled code (small operands
/// without a metavalue) and by the native (everything else): vectors of every length class
/// (1 to 66 elements), with and without `L`, `H` and metavalues, ascending and descending, a lower
/// bound other than zero, in a function (native frame) and in a process (frame mode).
const INLINE: &str = r#"library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity n is end;
architecture a of n is
  -- A hash of the bounds and the elements of a result.
  function h(u : unsigned) return integer is
    variable r : integer := u'length * 7 + u'left * 3 + u'right;
  begin
    if u'ascending then r := r + 1; end if;
    for k in u'range loop
      case u(k) is
        when '0' => r := (r * 3 + 1) mod 99991;
        when '1' => r := (r * 3 + 2) mod 99991;
        when 'L' => r := (r * 3 + 3) mod 99991;
        when 'H' => r := (r * 3 + 4) mod 99991;
        when 'X' => r := (r * 3 + 5) mod 99991;
        when others => r := (r * 3 + 6) mod 99991;
      end case;
    end loop;
    return r;
  end function;
  function h(s : signed) return integer is
  begin
    return h(unsigned(s));
  end function;
  function h(v : std_logic_vector) return integer is
  begin
    return h(unsigned(v));
  end function;
  function h(x : boolean) return integer is
  begin
    if x then return 1; else return 2; end if;
  end function;
  function gen(len : natural; seed : natural; meta : boolean) return unsigned is
    variable r : unsigned(len + 2 downto 3);
    variable s : natural := seed;
  begin
    for k in r'range loop
      s := (s * 75 + 74) mod 65537;
      case (s / 7) mod 8 is
        when 0 | 1 | 2 => r(k) := '0';
        when 3 | 4 | 5 => r(k) := '1';
        when 6 => r(k) := 'L';
        when others => r(k) := 'H';
      end case;
      if meta and s mod 29 = 0 then r(k) := 'X'; end if;
    end loop;
    return r;
  end function;
  function gen_up(len : natural; seed : natural) return unsigned is
    variable r : unsigned(2 to len + 1);
    variable s : natural := seed;
  begin
    for k in r'range loop
      s := (s * 75 + 74) mod 65537;
      if (s / 3) mod 2 = 0 then r(k) := '0'; else r(k) := '1'; end if;
    end loop;
    return r;
  end function;
  -- Comparisons, resizes, conversions from integers, shifts and logical operators.
  function calc2(a, b : unsigned; k : natural) return integer is
    variable sa : signed(a'length - 1 downto 0) := signed(a);
    variable sb : signed(b'length - 1 downto 0) := signed(b);
    variable la : std_logic_vector(a'length - 1 downto 0) := std_logic_vector(a);
    variable lb : std_logic_vector(a'length - 1 downto 0) := std_logic_vector(resize(b, a'length));
    variable same : unsigned(a'length - 1 downto 0);
    variable c : natural := k mod 70;
    variable r : integer := 0;
    procedure mix(x : integer) is
    begin
      r := (r * 31 + x) mod 99991;
    end procedure;
  begin
    same := resize(b, a'length);
    mix(h(a = b)); mix(h(a /= b)); mix(h(a < b)); mix(h(a <= b)); mix(h(a > b)); mix(h(a >= b));
    mix(h(a = k)); mix(h(a < k)); mix(h(a >= k)); mix(h(k = b)); mix(h(k > b)); mix(h(k <= b));
    mix(h(sa = sb)); mix(h(sa < sb)); mix(h(sa >= sb)); mix(h(sa < (k - 40))); mix(h(sa >= (k - 40)));
    mix(h((k - 40) = sb)); mix(h((k - 40) > sb));
    mix(h(resize(a, c + 1))); mix(h(resize(sa, c + 1)));
    mix(h(to_unsigned(k, c + 1))); mix(h(to_signed(k - 40, c + 1)));
    mix(h(shift_left(a, c mod 9))); mix(h(shift_right(a, c mod 11)));
    mix(h(rotate_left(a, c))); mix(h(rotate_right(sa, c)));
    mix(h(shift_right(sa, c mod 5))); mix(h(a sll (c mod 7)));
    mix(h(a srl (c mod 3))); mix(h(sa rol (c mod 13)));
    mix(h(a and same)); mix(h(a or same)); mix(h(a xor same));
    mix(h(a nand same)); mix(h(a nor same)); mix(h(sa xnor signed(same)));
    mix(h(not a)); mix(h(not sb));
    mix(h(la and lb)); mix(h(la xor lb)); mix(h(not la));
    return r;
  end function;
  -- All the operations on one pair of operands, one line of text.
  function calc(a, b : unsigned; k : natural) return string is
    variable sa : signed(a'length - 1 downto 0) := signed(a);
    variable sb : signed(b'length - 1 downto 0) := signed(b);
    variable r : integer := 0;
  begin
    if a'length <= 31 then r := to_integer(a) / 4; end if;
    if b'length <= 31 then r := r + to_integer(b) / 4; end if;
    if a'length <= 32 then r := r + to_integer(sa) / 4; end if;
    return integer'image(calc2(a, b, k)) & " " & integer'image(r) & " " & integer'image(h(a + b)) & " " &
           integer'image(h(a - b)) & " " & integer'image(h(a + k)) & " " & integer'image(h(k + b)) & " " &
           integer'image(h(a - k)) & " " & integer'image(h(k - b)) & " " & integer'image(h(sa + sb)) & " " &
           integer'image(h(sa - sb)) & " " & integer'image(h(sa + (k - 40))) & " " &
           integer'image(h((k - 40) + sb)) & " " & integer'image(h(sa - (k - 40)));
  end function;
begin
  process
    variable a, b : unsigned(70 downto 0);
    variable n : integer := 0;
    constant lens : integer_vector := (1, 2, 3, 8, 9, 16, 17, 32, 33, 63, 64, 65);
  begin
    for i in lens'range loop
      for j in 0 to 2 loop
        for seed in 0 to 4 loop
          report integer'image(lens(i)) & "/" & integer'image(lens((i + j * 5) mod lens'length)) & "/" &
                 integer'image(seed) & ": " &
                 calc(gen(lens(i), seed, seed mod 3 = 0),
                      gen(lens((i + j * 5) mod lens'length), seed + 11, seed mod 4 = 0),
                      seed * 37 + j);
        end loop;
      end loop;
    end loop;
    for i in 1 to 20 loop
      report "up " & integer'image(i) & ": " & calc(gen_up(i, i), gen_up(21 - i, i + 3), i) &
             " " & integer'image(to_integer(gen_up(i mod 31 + 1, i)));
    end loop;
    -- In the process itself: constants and variables of the frame.
    a(7 downto 0) := x"a5";
    b(7 downto 0) := x"3c";
    for k in 0 to 300 loop
      a(7 downto 0) := a(7 downto 0) + k;
      b(7 downto 0) := b(7 downto 0) - to_unsigned(k, 8);
      n := n + to_integer(a(7 downto 0)) + to_integer(signed(b(7 downto 0)));
      if k mod 50 = 0 then
        report integer'image(n) & " " & integer'image(h(a(7 downto 0) + b(7 downto 0))) & " " &
               integer'image(h(5 + b(7 downto 0)));
      end if;
    end loop;
    report "done " & integer'image(n);
    wait;
  end process;
end;
"#;
#[test]
fn numeric_std_in_place_in_compiled_code_matches_the_interpreter() {
    let dir = design("clift_inline", INLINE);
    let (want, _) = parity(&dir, "n", &["--exit-severity=failure"]);
    assert!(want.out.contains("done "), "{want:?}");
    assert!(
        want.out.contains("metavalue"),
        "the metavalue warnings are part of the test"
    );
}

/// `is_x` and the reductions of `std_logic_1164`, `numeric_std` and `std_logic_misc`, computed in
/// place by compiled code (1 to 64 elements) and by the native (null, 65 and more): every code in
/// the vectors, ascending and descending, in a function (native frame) and in a process (frame
/// mode), and `is_x` of each of the nine scalar codes.
const REDUCE: &str = r#"library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use ieee.std_logic_misc.all;
entity r is end;
architecture a of r is
  function gen(len : natural; seed : natural; kinds : natural) return std_ulogic_vector is
    variable r : std_ulogic_vector(len + 2 downto 3);
    variable s : natural := seed;
  begin
    for k in r'range loop
      s := (s * 75 + 74) mod 65537;
      r(k) := std_ulogic'val(2 + (s / 7) mod 2 + 4 * ((s / 11) mod 2));
      if kinds > 0 and s mod kinds = 0 then r(k) := std_ulogic'val((s / 13) mod 9); end if;
    end loop;
    return r;
  end function;
  function code(x : std_ulogic) return integer is
  begin
    return std_ulogic'pos(x);
  end function;
  function b(x : boolean) return integer is
  begin
    if x then return 1; else return 0; end if;
  end function;
  -- One digit per result: `is_x`, the six gates of each package.
  function all_of(v : std_ulogic_vector) return string is
    variable u : unsigned(v'length - 1 downto 0) := unsigned(v);
    variable l : std_logic_vector(v'range) := std_logic_vector(v);
    variable up : std_ulogic_vector(1 to v'length) := v;
  begin
    return integer'image(b(is_x(v))) & integer'image(b(is_x(l))) & integer'image(b(is_x(up))) &
           integer'image(code(and v)) & integer'image(code(nand v)) & integer'image(code(or v)) &
           integer'image(code(nor v)) & integer'image(code(xor v)) & integer'image(code(xnor v)) &
           integer'image(code(and up)) & integer'image(code(xor up)) &
           integer'image(code(and u)) & integer'image(code(or u)) & integer'image(code(xnor u)) &
           integer'image(code(and_reduce(l))) & integer'image(code(or_reduce(l))) &
           integer'image(code(xor_reduce(l))) & integer'image(code(nor_reduce(l)));
  end function;
begin
  process
    constant lens : integer_vector := (0, 1, 2, 3, 8, 9, 31, 32, 33, 63, 64, 65, 66);
    constant every : integer_vector := (0, 2, 5, 23);
    variable v8 : std_ulogic_vector(7 downto 0) := "01LH01HL";
    variable n : integer := 0;
  begin
    for i in lens'range loop
      for kinds in 0 to 3 loop
        for seed in 0 to 6 loop
          report integer'image(lens(i)) & "/" & integer'image(kinds) & "/" & integer'image(seed) &
                 ": " & all_of(gen(lens(i), seed + 17 * kinds, every(kinds)));
        end loop;
      end loop;
    end loop;
    -- Metavalues with no deciding element, and with one.
    report "fixed " & all_of("UUU1") & " " & all_of("U1H1") & " " & all_of("XX11") & " " &
           all_of("-WZ1") & " " & all_of("UX") & " " & all_of("U0X1") & " " & all_of("HHHH") & " " &
           all_of("L0L0") & " " & all_of("1U") & " " & all_of("Z") & " " & all_of("U") & " " &
           all_of("0UX") & " " & all_of("1XU");
    for c in std_ulogic loop
      report "is_x " & std_ulogic'image(c) & " " & integer'image(b(is_x(c)));
    end loop;
    -- In the process itself.
    for k in 0 to 80 loop
      v8(k mod 8) := std_ulogic'val(k mod 9);
      n := n * 3 + b(is_x(v8)) + code(and v8) + 2 * code(or v8) + 3 * code(xor v8) + b(is_x(v8(k mod 8)));
      n := n mod 99991;
    end loop;
    report "done " & integer'image(n);
    wait;
  end process;
end;
"#;
#[test]
fn is_x_and_reductions_in_compiled_code_match_the_interpreter() {
    let dir = design("clift_reduce", REDUCE);
    let (want, _) = parity(&dir, "r", &[]);
    assert!(want.out.contains("done "), "{want:?}");
    assert!(want.out.contains("is_x 'U' 1"), "{want:?}");
}

/// Concatenations of bytes computed in place by compiled code (`Gen::cat_bytes`): strings of
/// characters and strings (null ones among them), arrays of integers, reals and booleans, the
/// most pieces one `Cat` has (16; a longer chain is copies and stores), in a function (native
/// frame) and in a process (frame mode), and one whose result does not fit its target (the
/// interpreter's error).
const CAT: &str = r#"entity c is end;
architecture a of c is
  function img(n : integer) return string is
  begin
    return integer'image(n);
  end function;
  function words(k : natural) return string is
    constant e : string(1 to 0) := "";
  begin
    return "<" & img(k) & ":" & e & character'val(65 + k mod 26) & "," & img(k * 7) & e & ">";
  end function;
  function many(k : natural) return string is
  begin
    return img(k) & "a" & "b" & img(k + 1) & "c" & "d" & img(k + 2) & "e" & "f" & img(k + 3) &
           "g" & "h" & img(k + 4) & "i" & "j" & img(k + 5);
  end function;
  function more(k : natural) return string is
    variable c : character := character'val(97 + k mod 26);
  begin
    return img(k) & c & img(k + 1) & c & img(k + 2) & c & img(k + 3) & c & img(k + 4) & c &
           img(k + 5) & c & img(k + 6) & c & img(k + 7) & c & img(k + 8);
  end function;
  function ints(k : integer) return integer_vector is
    variable v : integer_vector(1 to 3) := (k, -k, k * k);
  begin
    return v & k & (k + 1, k + 2) & v(2 to 3);
  end function;
  function reals(x : real) return real_vector is
  begin
    return x & (x * 2.0, x / 3.0) & (-x);
  end function;
  function bools(k : natural) return boolean_vector is
  begin
    return (k mod 2 = 0) & (true, k mod 3 = 0) & false;
  end function;
  function sum(v : integer_vector) return integer is
    variable r : integer := 0;
  begin
    for i in v'range loop r := (r * 31 + v(i)) mod 99991; end loop;
    return r;
  end function;
begin
  process
    variable s : string(1 to 5);
    variable n : integer := 0;
    variable rv : real_vector(1 to 4);
    variable bv : boolean_vector(1 to 4);
  begin
    for k in 0 to 40 loop
      report words(k) & " " & many(k) & " " & more(k) & " " & img(sum(ints(k - 20)));
      rv := reals(real(k) + 0.5);
      bv := bools(k);
      n := (n * 7 + integer(rv(3) * 100.0) + boolean'pos(bv(1)) + boolean'pos(bv(3))) mod 99991;
      -- In the process itself.
      s := "ab" & character'val(48 + k mod 10) & "de";
      report s & img(n) & ("x" & s(2 to 3) & "y");
    end loop;
    s := "abc" & img(n mod 1000 + 1000);
    report "unreachable " & s;
    wait;
  end process;
end;
"#;
#[test]
fn byte_concatenations_in_compiled_code_match_the_interpreter() {
    let dir = design("clift_cat", CAT);
    let (want, _) = parity(&dir, "c", &[]);
    assert!(want.out.contains("<40:O,280>"), "{want:?}");
    assert!(!want.out.contains("unreachable"), "{want:?}");
    assert_ne!(want.code, Some(0), "{want:?}");
}

/// Layouts from run-time descriptors made by compiled code from the layout memo
/// (`Gen::layout_into`): records with unconstrained fields of several constraints read in a
/// function (native frame) and an array of such records; next to them a record of generic
/// constraint in the process (frame mode, stepped).
const LAYOUT: &str = r#"entity l is generic (N : positive := 3); end;
architecture a of l is
  type rec is record
    a : integer_vector;
    b : string;
    k : integer;
  end record;
  type recs is array (natural range <>) of rec;
  function f(r : rec) return integer is
  begin
    return r.a(r.a'left) * 3 + r.b'length + r.a'length + r.k + character'pos(r.b(r.b'right));
  end function;
  function g(v : recs) return integer is
    variable s : integer := 0;
  begin
    for i in v'range loop
      s := (s * 7 + f(v(i))) mod 99991;
    end loop;
    return s;
  end function;
begin
  process
    variable r1 : rec(a(1 to 3), b(1 to 2));
    variable r2 : rec(a(0 to 4), b(1 to 7));
    variable v : recs(1 to 2)(a(1 to 2), b(1 to 3));
    variable s : integer := 0;
    variable q : rec(a(1 to N), b(1 to N + 1));
  begin
    for k in 0 to 99 loop
      r1 := ((k, 2, 3), "xy", k);
      r2 := ((-k, 1, 2, 3, 4), "abcdefg", 2 * k);
      v := (((k, 1), "abc", 1), ((2, k), "def", k));
      q.a(1 + k mod N) := k;
      q.k := q.a(1) + q.b'length;
      s := (s * 31 + f(r1) + f(r2) + g(v) + q.k) mod 99991;
      wait for 1 ns;
    end loop;
    report "s=" & integer'image(s);
    wait;
  end process;
end;
"#;
#[test]
fn layouts_from_run_time_descriptors_in_compiled_code_match_the_interpreter() {
    let dir = design("clift_layout", LAYOUT);
    let (want, _) = parity(&dir, "l", &[]);
    assert!(want.out.contains("s=34398"), "{want:?}");
    let (want, _) = parity(&dir, "l", &["-gN=6"]);
    assert!(want.out.contains("s="), "{want:?}");
}

/// Reals rounded to integers by compiled code (`Gen::f_to_i`): halves away from zero, the largest
/// fraction under a half, both ends of `integer`, a range-checked subtype, in a function (native
/// frame) and in a process (frame mode); and each way out of range (the interpreter's error).
const REAL_INT: &str = r#"entity r is generic (BAD : integer := 0); end;
architecture a of r is
  subtype small is integer range -50 to 50;
  function r(x : real) return integer is
  begin
    return integer(x);
  end function;
  function rs(x : real) return small is
    variable s : small;
  begin
    s := small(x);
    return s;
  end function;
begin
  process
    constant xs : real_vector := (0.0, -0.0, 0.5, -0.5, 1.5, 2.5, -2.5, 0.49999999999999994,
                                  -0.49999999999999994, 2147483647.4, -2147483648.4,
                                  1048575.5, -1048576.5, 49.5, -49.5, 1.0e-300, 7.25, -7.75);
    variable s, k : integer := 0;
  begin
    for i in xs'range loop
      s := r(xs(i));
      report integer'image(s);
      if abs xs(i) < 50.0 then
        k := k + rs(xs(i)) + integer(xs(i) * 3.0);
      end if;
      wait for 1 ns;
    end loop;
    report "k=" & integer'image(k);
    if BAD = 1 then
      report integer'image(r(1.0e300));
    elsif BAD = 2 then
      report integer'image(rs(50.5));
    elsif BAD = 3 then
      k := r(-2147483648.5);
    elsif BAD = 4 then
      k := r(2147483647.5);
    end if;
    wait;
  end process;
end;
"#;
#[test]
fn reals_to_integers_in_compiled_code_match_the_interpreter() {
    let dir = design("clift_real_int", REAL_INT);
    let (want, _) = parity(&dir, "r", &[]);
    assert!(want.out.contains("k=5"), "{want:?}");
    for bad in 1..=4 {
        let g = format!("-gBAD={bad}");
        let (want, _) = parity(&dir, "r", &[&g]);
        assert!(want.err.contains("outside of"), "{bad}: {want:?}");
    }
}

/// `deallocate` on native frames (`Gen::free`): of a local, of a null value, of a field of another
/// heap object, of a protected type's own variable (the shared heap); and the interpreter's errors:
/// an object deallocated twice, and a cell inside the object it frees.
const FREE: &str = r#"entity fr is generic (BAD : integer := 0); end;
architecture a of fr is
  type node;
  type node_ptr is access node;
  type node is record
    v : integer;
    nx : node_ptr;
  end record;
  type int_ptr is access integer;
  function churn(n : integer) return integer is
    variable p, q : int_ptr;
    variable h : node_ptr;
    variable s : integer := 0;
  begin
    for i in 1 to n loop
      p := new integer'(i);
      s := s + p.all;
      deallocate(p);
      if p = null then
        s := s + 1;
      end if;
      deallocate(q);
      h := new node'(i, null);
      h.nx := new node'(2 * i, null);
      s := s + h.nx.v;
      deallocate(h.nx);
      if h.nx = null then
        s := s + 3;
      end if;
      deallocate(h);
    end loop;
    return s;
  end function;
  function twice return integer is
    variable p, q : int_ptr;
  begin
    p := new integer'(5);
    q := p;
    deallocate(p);
    deallocate(q);
    return 0;
  end function;
  function inside return integer is
    variable h : node_ptr;
  begin
    h := new node'(1, null);
    h.nx := h;
    deallocate(h.nx);
    return 7;
  end function;
  type store is protected
    procedure put(x : integer);
    impure function take return integer;
  end protected;
  type store is protected body
    variable p : int_ptr;
    procedure put(x : integer) is
    begin
      deallocate(p);
      p := new integer'(x);
    end procedure;
    impure function take return integer is
      variable r : integer;
    begin
      r := p.all;
      deallocate(p);
      return r;
    end function;
  end protected body;
  shared variable sv : store;
begin
  process
    variable s : integer := 0;
  begin
    for k in 1 to 30 loop
      s := (s * 7 + churn(k mod 5 + 1)) mod 99991;
      sv.put(k);
      sv.put(k + 1);
      s := s + sv.take;
      wait for 1 ns;
    end loop;
    report "s=" & integer'image(s);
    if BAD = 1 then
      s := twice;
    elsif BAD = 2 then
      s := inside;
      report "inside " & integer'image(s);
    end if;
    wait;
  end process;
end;
"#;
#[test]
fn deallocations_in_compiled_code_match_the_interpreter() {
    let dir = design("clift_free", FREE);
    let (want, got) = parity(&dir, "fr", &[]);
    assert!(want.out.contains("s="), "{want:?}");
    if counted(&got[0]) {
        assert!(
            calls(&got[0], "counters: cb_free=") > 0,
            "{:?}",
            got[0].census
        );
        assert_eq!(calls(&got[0], "cb_xstep.Free"), 0, "{:?}", got[0].census);
    }
    for bad in 1..=2 {
        let g = format!("-gBAD={bad}");
        parity(&dir, "fr", &[&g]);
    }
}

/// Allocations in compiled code (`Gen::new_obj`, `yavs_rt_new`) on native frames (the functions)
/// and in frame mode (the process): records holding access values, qualified aggregates and
/// literals (`New` and `NewFill` without a fill), objects of a subtype filled with a default of
/// one byte, of eight (`integer`, not zero) and with `'U'`, a null range, a range aggregate, a
/// protected type's own list (the shared heap), with collections in between; and the
/// interpreter's error after a deallocation.
const NEW: &str = r#"library ieee; use ieee.std_logic_1164.all;
entity nw is generic (BAD : integer := 0); end;
architecture a of nw is
  type node;
  type node_ptr is access node;
  type node is record
    v : integer;
    nx : node_ptr;
  end record;
  type iv_ptr is access integer_vector;
  type str_ptr is access string;
  type slv_ptr is access std_logic_vector;
  type int_ptr is access integer;
  procedure len(variable p : in node_ptr; variable n : out integer) is
    variable q : node_ptr := p;
  begin
    n := 0;
    while q /= null loop
      n := n + q.v;
      q := q.nx;
    end loop;
  end procedure;
  function churn(k, n : integer) return integer is
    variable h : node_ptr;
    variable iv : iv_ptr;
    variable s : str_ptr;
    variable l : slv_ptr;
    variable p : int_ptr;
    variable t : integer := k;
    variable m : integer;
  begin
    for i in 1 to n loop
      h := new node'(i, h);
      iv := new integer_vector'(i, k, 3);
      t := t + iv(1) + iv'length;
      iv := new integer_vector(0 to i);
      t := t + iv(i) mod 1000 + iv'length;
      iv := new integer_vector'(0 to i => k + i);
      t := t + iv(i);
      s := new string(1 to i);
      t := t + character'pos(s(i)) + s'length;
      s := new string'("ab" & integer'image(i));
      t := t + s'length;
      s := new string(1 to 0);
      t := t + s'length;
      l := new std_logic_vector(0 to i + 70);
      t := t + std_logic'pos(l(i)) + l'length;
      p := new integer;
      t := t + p.all mod 1000;
      p := new integer'(i);
      t := t + p.all;
      deallocate(p);
      if i mod 3 = 0 then
        deallocate(iv);
      end if;
    end loop;
    len(h, m);
    return t + m;
  end function;
  type store is protected
    procedure put(x : integer);
    impure function sum return integer;
  end protected;
  type store is protected body
    variable h : node_ptr;
    procedure put(x : integer) is
    begin
      h := new node'(x, h);
    end procedure;
    impure function sum return integer is
      variable m : integer;
    begin
      len(h, m);
      return m;
    end function;
  end protected body;
  shared variable sv : store;
begin
  process
    variable s : integer := 0;
    variable h : node_ptr;
    variable iv : iv_ptr;
    variable st : str_ptr;
    variable m : integer;
  begin
    for k in 0 to 60 loop
      s := (s * 7 + churn(k, k mod 6 + 1)) mod 99991;
      h := new node'(k, h);
      iv := new integer_vector(0 to k);
      st := new string(1 to k mod 4);
      len(h, m);
      s := (s + iv'length + st'length + m) mod 99991;
      iv := new integer_vector'(k, s, 7);
      s := (s + iv(1)) mod 99991;
      sv.put(k);
      s := (s + sv.sum) mod 99991;
      wait for 1 ns;
    end loop;
    report "s=" & integer'image(s);
    if BAD = 1 then
      deallocate(iv);
      s := iv(0);
    end if;
    wait;
  end process;
end;
"#;

#[test]
fn allocations_in_compiled_code_match_the_interpreter() {
    let dir = design("clift_new", NEW);
    let (want, got) = parity(&dir, "nw", &[]);
    assert!(want.out.contains("s="), "{want:?}");
    if counted(&got[0]) {
        assert!(
            calls(&got[0], "counters: cb_new=") > 0,
            "{:?}",
            got[0].census
        );
        assert_eq!(calls(&got[0], "cb_xstep.New"), 0, "{:?}", got[0].census);
    }
    for gc in ["--gc-threshold=1", "--gc-threshold=300"] {
        parity(&dir, "nw", &[gc]);
    }
    parity(&dir, "nw", &["-gBAD=1"]);
}

/// `ieee` ops of 65 to 192 elements computed in place (`Gen::ieee_wide`), the gates on any
/// element value, and concatenations of two blocks (`Gen::cat_block`), with metavalues, `L` and
/// `H` now and then: in a function (native frame) and in a process (frame mode). The arithmetic
/// with a metavalue goes to the native, as does a product over 192 elements.
const WIDE: &str = r#"library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity w is end;
architecture a of w is
  function h(v : std_ulogic_vector) return integer is
    variable r : integer := 7;
  begin
    for i in v'range loop
      r := (r * 11 + std_ulogic'pos(v(i))) mod 99991;
    end loop;
    return r;
  end function;
  function hu(v : unsigned) return integer is begin return h(std_ulogic_vector(v)); end function;
  function hs(v : signed) return integer is begin return h(std_ulogic_vector(v)); end function;
  function c(x : std_ulogic) return integer is begin return std_ulogic'pos(x); end function;
  function line(k, n : integer) return string is
  begin
    return integer'image(k) & " " & integer'image(n);
  end function;
  function mix(x, y : std_ulogic_vector(63 downto 0); k : integer) return integer is
    variable a65 : std_ulogic_vector(64 downto 0) := (x(63) and y(0)) & x;
    variable b65 : std_ulogic_vector(64 downto 0) := y(5) & y;
    variable a100 : std_ulogic_vector(99 downto 0) := x(35 downto 0) & y;
    variable a128 : std_ulogic_vector(127 downto 0) := x & y;
    variable b128 : std_ulogic_vector(127 downto 0) := y & x(31 downto 0) & y(63 downto 32);
    variable a33 : signed(32 downto 0) := signed(x(32 downto 0));
    variable b33 : signed(32 downto 0) := signed(y(40 downto 8));
    variable n : integer;
  begin
    n := (h(x and y) + h(x nand y) + h(x nor y) + h(x xnor y) + h(not x)
          + h(x(43 downto 0) and not y(43 downto 0)) + h(a65 and b65) + h(a65 or b65)
          + h(a128 xor b128) + h(a65 nand b65) + h(a128 nor b128) + h(not a128)
          + hu(unsigned(a65) and unsigned(b65)) + hu(not unsigned(a128))
          + hs(signed(a128) xnor signed(b128))) mod 99991;
    n := (n * 3 + c(or a65) + 2 * c(and a65) + 3 * c(xor a65) + 5 * c(nand a128) + 7 * c(nor a128)
          + 11 * c(xnor a100) + 13 * c(and b128(127 downto 63)) + 17 * c(or b128(127 downto 63))
          + 19 * boolean'pos(is_x(a65)) + 23 * boolean'pos(is_x(a128))) mod 99991;
    n := (n * 3 + hu(unsigned(a65) + unsigned(b65)) + hu(unsigned(a128) - unsigned(b128))
          + hs(signed(a65) + signed(b128)) + hs(signed(a100) - signed(a65)) + hu(unsigned(a65) + 1)
          + hu(unsigned('0' & not x) + 1) + hu(k - unsigned(a100) + k) + hs(signed(a128) + (-k))
          + hs(k - signed(a65)) + hu(unsigned(a128) + unsigned(x)) + hs(signed(x) - signed(a65)))
         mod 99991;
    n := (n * 3 + hs(signed(a65) * signed(b65)) + hu(unsigned(a65) * unsigned(b65)) + hs(a33 * b33)
          + hu(unsigned(x) * unsigned(y)) + hs(signed(a128) * signed(x))
          + hu(unsigned(a100) * unsigned(y(27 downto 0)))
          + hs(signed(a128) * signed(b128(127 downto 64))) + hu(unsigned(a128) * unsigned(b128))
          + hu(unsigned(a33) * unsigned(a100(30 downto 0)))) mod 99991;
    n := (n * 3 + h(x(29 downto 0) & y(49 downto 0) & x(39 downto 0)) + h(y(0) & x & y(62 downto 0))
          + h('1' & x & 'U') + h(a65 & y(62 downto 0))
          + h(x(10 downto 0) & a100(99 downto 2) & y(3)) + h(y(63 downto 1) & x(63) & x)) mod 99991;
    return n;
  end function;
begin
  process
    variable seed : unsigned(63 downto 0) := x"9e3779b97f4a7c15";
    variable x, y : std_ulogic_vector(63 downto 0);
    variable a65 : std_ulogic_vector(64 downto 0);
    variable p130 : std_ulogic_vector(129 downto 0);
    variable d : std_ulogic_vector(127 downto 0);
    variable u : unsigned(64 downto 0);
    variable n : integer := 0;
  begin
    for k in 0 to 120 loop
      seed := seed(62 downto 0) & (seed(63) xor seed(62) xor seed(60) xor seed(59));
      x := std_ulogic_vector(seed);
      y := std_ulogic_vector(seed rol 17);
      if k mod 7 = 3 then
        x(k mod 64) := std_ulogic'val(k mod 9);
      end if;
      if k mod 11 = 4 then
        y((k * 5) mod 64) := 'H';
        y((k * 3) mod 64) := 'L';
      end if;
      if k mod 13 = 6 then
        y(63) := std_ulogic'val((k / 13) mod 9);
      end if;
      -- In the process itself.
      a65 := (x(63) and y(9)) & x;
      p130 := std_ulogic_vector(signed(a65) * signed(y(0) & y));
      d := std_ulogic_vector(unsigned(x & y) + unsigned(p130(127 downto 0)));
      u := unsigned('0' & not x) + 1;
      n := (n * 3 + mix(x, y, k) + h(p130) + h(d) + h(std_ulogic_vector(u))
            + c(or d(127 downto 63)) + c(and d(127 downto 63))
            + h(x(20 downto 0) & y & x(63 downto 21)) + h(x and y) + h(not y)) mod 99991;
      report line(k, n);
    end loop;
    report line(-1, n);
    wait;
  end process;
end;
"#;
#[test]
fn wide_ieee_ops_and_concatenations_in_compiled_code_match_the_interpreter() {
    let dir = design("clift_wide", WIDE);
    let (want, got) = parity(&dir, "w", &[]);
    assert!(want.out.contains(": -1 "), "{want:?}");
    // Every concatenation is computed in place, in either mode.
    let forced = &got[0];
    assert!(counted(forced), "{forced:?}");
    assert_eq!(calls(forced, ".Cat"), 0, "{:?}", forced.census);
}

/// Vectors of every length class read from signals in a clocked process, among them the ones
/// whose last group of eight codes is partly past the signal (1, 4, 9, 12 elements) and the
/// ones that fill their plane block (64) or need two (65).
const SIGNALS: &str = r#"library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity s is end;
architecture a of s is
  signal clk : std_logic := '0';
  signal v1 : std_logic_vector(0 downto 0) := "1";
  signal v4 : std_logic_vector(3 downto 0) := "10X1";
  signal v9 : unsigned(8 downto 0) := (others => '0');
  signal v12 : std_logic_vector(11 downto 0) := (others => 'Z');
  signal v33 : unsigned(32 downto 0) := (others => '0');
  signal v64 : unsigned(63 downto 0) := (others => '0');
  signal v65 : unsigned(64 downto 0) := (others => '0');
begin
  clk <= not clk after 5 ns when now < 80 ns else clk;
  process (clk)
    variable n : integer := 0;
  begin
    if rising_edge(clk) then
      n := n + 1;
      v1 <= std_logic_vector(to_unsigned(n, 1));
      v4 <= std_logic_vector(to_unsigned(n, 4));
      v9 <= v9 + 37;
      v12 <= std_logic_vector(to_unsigned(n * 111, 12));
      v33 <= v33 + 1000003;
      v64 <= (v64 sll 3) + to_unsigned(n * 1000003, 64);
      v65 <= (v65 sll 5) + to_unsigned(n * 7919, 65);
      report std_logic'image(v1(0)) & " " & to_string(v4) & " " & to_string(v9) & " " &
             to_string(v12) & " " & to_string(v33) & " " & to_string(v64) & " " & to_string(v65);
    end if;
  end process;
end;
"#;
#[test]
fn signal_reads_in_compiled_code_match_the_interpreter() {
    let dir = design("clift_signals", SIGNALS);
    let (want, _) = parity(&dir, "s", &["--stop-time=100ns"]);
    assert!(want.out.contains("** Note"), "{want:?}");
}

const KINDS: &str = r#"library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity k is end;
architecture a of k is
  type st_t is (idle, run, done, fail);
  signal clk : std_logic := '0';
  signal st : st_t := idle;
  signal cnt : integer := -5;
  signal flag : boolean := false;
  signal r : real := 1.5;
  signal w128 : std_logic_vector(127 downto 0) := (others => '0');
  signal w200 : unsigned(199 downto 0) := (others => '0');
  signal t : time := 3 ns;
  signal small : natural range 0 to 200 := 7;
begin
  clk <= not clk after 5 ns when now < 200 ns else clk;
  process (clk)
    variable n : integer := 0;
    variable acc : integer := 0;
  begin
    if rising_edge(clk) then
      n := n + 1;
      case st is
        when idle => st <= run;
        when run => if cnt > 0 then st <= done; end if;
        when done => st <= fail;
        when fail => st <= idle;
      end case;
      cnt <= cnt + 2;
      flag <= not flag;
      if flag then r <= r * 2.0; end if;
      if cnt < 0 then acc := acc + 1; end if;
      w128 <= w128(126 downto 0) & clk;
      w200 <= w200 + to_unsigned(n, 200);
      t <= t + 1 ns;
      small <= (small * 3) mod 201;
      report st_t'image(st) & " " & integer'image(cnt) & " " & boolean'image(flag) & " " &
             real'image(r) & " " & to_hstring(w128) & " " & to_hstring(std_logic_vector(w200)) &
             " " & time'image(t) & " " & integer'image(small) & " " & integer'image(acc);
    end if;
  end process;
end;
"#;
#[test]
fn scalar_and_wide_signal_reads_in_compiled_code_match_the_interpreter() {
    let dir = design("clift_kinds", KINDS);
    let (want, got) = parity(&dir, "k", &[]);
    assert!(want.out.matches("** Note").count() >= 20, "{want:?}");
    // The reads of all the kinds are answered in compiled code.
    for r in got {
        assert!(counted(&r), "{r:?}");
        assert!(
            calls(&r, ".SigRead=") + calls(&r, ".SigReadScalar=") == 0,
            "{:?}",
            r.census
        );
    }
}

const COMPARES: &str = r#"library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity q is end;
architecture a of q is
  signal clk : std_logic := '0';
  signal a1, b1 : std_logic_vector(0 downto 0) := "1";
  signal a5, b5 : std_logic_vector(4 downto 0) := "10X1Z";
  signal a64, b64 : std_logic_vector(63 downto 0) := (others => '1');
  signal a65, b65 : std_logic_vector(64 downto 0) := (others => '0');
  signal a100, b100 : std_logic_vector(99 downto 0) := (others => 'U');
begin
  clk <= not clk after 5 ns when now < 120 ns else clk;
  process (clk)
    variable n : integer := 0;
    variable s : string(1 to 10);
    procedure mark (k : positive; v : boolean) is
    begin
      if v then s(k) := '1'; else s(k) := '0'; end if;
    end;
  begin
    if rising_edge(clk) then
      n := n + 1;
      mark(1, a1 = b1);
      mark(2, a5 = b5);
      mark(3, a64 = b64);
      mark(4, a65 = b65);
      mark(5, a100 = b100);
      mark(6, a5(3 downto 1) = b5(3 downto 1));
      mark(7, a64(62 downto 0) = b64(62 downto 0));
      mark(8, a64(63 downto 1) /= b64(63 downto 1));
      mark(9, a100(70 downto 3) = b100(70 downto 3));
      mark(10, a5 /= "10X1Z");
      report s;
      a1(0) <= to_unsigned(n, 1)(0);
      b1(0) <= to_unsigned(n / 2, 1)(0);
      a5 <= std_logic_vector(to_unsigned(n, 5));
      b5 <= std_logic_vector(to_unsigned(n / 3, 5)) when n mod 4 /= 0 else a5;
      b64 <= (others => to_unsigned(n, 1)(0)) when n mod 3 = 0 else a64;
      a64 <= (others => to_unsigned(n / 2, 1)(0)) when n mod 5 /= 0 else (others => 'X');
      a65(n mod 65) <= '1';
      b65(n mod 7) <= '1';
      a100 <= (others => '0');
      b100 <= (others => '0') when n mod 2 = 0 else (others => 'H');
      -- Runs set to one code: inside a block, across two, a whole one.
      if n mod 3 = 0 then
        a65(10 downto 2) <= (others => 'H');
        a100(99 downto 30) <= (others => '-');
        b64 <= (others => 'Z');
      elsif n mod 3 = 1 then
        a5(2 downto 1) <= (others => 'L');
        b100(64 downto 63) <= (others => 'X');
        a64(63 downto 0) <= (others => 'U');
      end if;
    end if;
  end process;
end;
"#;
#[test]
fn logic_equality_in_compiled_code_matches_the_interpreter() {
    let dir = design("clift_compares", COMPARES);
    let (want, _) = parity(&dir, "q", &[]);
    assert!(want.out.matches("** Note").count() >= 10, "{want:?}");
}

const RECORDS: &str = r#"library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity lc is end;
architecture a of lc is
  type rec_t is record
    d : std_logic_vector;
    n : integer;
    e : std_logic_vector;
  end record;
  subtype r8 is rec_t(d(7 downto 0), e(3 downto 0));
  subtype r16 is rec_t(d(15 downto 0), e(7 downto 0));
  signal clk : std_logic := '0';
  signal q8  : std_logic_vector(7 downto 0) := (others => '0');
  signal q16 : std_logic_vector(15 downto 0) := (others => '0');
  signal qe  : std_logic_vector(11 downto 0) := (others => '0');
begin
  clk <= not clk after 5 ns when now < 400 ns else clk;
  process (clk)
    variable x : r8;
    variable y : r16;
    variable k : integer := 0;
  begin
    if rising_edge(clk) then
      k := k + 1;
      x.d := std_logic_vector(to_unsigned(k, 8));
      y.d := std_logic_vector(to_unsigned(k * 257, 16));
      x.n := k;
      y.n := x.n * 2;
      x.e := y.d(3 downto 0);
      y.e := x.d(7 downto 4) & x.e;
      q8 <= x.d xor y.d(7 downto 0);
      q16 <= y.d(15 downto 8) & x.d;
      qe <= x.e & y.e;
      report integer'image(x.n) & " " & integer'image(y.n) & " " & to_hstring(q8) & " " & to_hstring(q16) & " " & to_hstring(qe) & " " & to_hstring(x.d) & to_hstring(y.e);
    end if;
  end process;
end;
"#;
#[test]
fn record_layouts_kept_in_the_frame_match_the_interpreter() {
    let dir = design("clift_records", RECORDS);
    let (want, _) = parity(&dir, "lc", &[]);
    assert!(want.out.matches("** Note").count() >= 20, "{want:?}");
}

/// A chain of eight functions, a nested function reading its parent's variable, a protected
/// counter, and a chain that fails at depth 5 when `fail` is set (the frames of the callers are
/// materialised for the error trace).
const MIXED: &str = r#"
entity mixed is generic (fail : boolean := false); end;
architecture a of mixed is
  type counter_t is protected
    procedure bump (by : integer);
    impure function get return integer;
  end protected;
  type counter_t is protected body
    variable n : integer := 0;
    procedure bump (by : integer) is begin n := n + by; end;
    impure function get return integer is begin return n; end;
  end protected body;
  shared variable c : counter_t;

  function f8 (x : integer) return integer is
    variable r : integer range 0 to 100000;
  begin
    r := x * 2;
    return r;
  end;
  function f7 (x : integer) return integer is begin return f8(x + 1) + 1; end;
  function f6 (x : integer) return integer is begin return f7(x + 1) + 1; end;
  function f5 (x : integer) return integer is begin return f6(x + 1) + 1; end;
  function f4 (x : integer) return integer is begin return f5(x + 1) + 1; end;
  function f3 (x : integer) return integer is begin return f4(x + 1) + 1; end;
  function f2 (x : integer) return integer is begin return f3(x + 1) + 1; end;
  function f1 (x : integer) return integer is begin return f2(x + 1) + 1; end;

  impure function nested (k : integer) return integer is
    variable base : integer := k * 3;
    impure function inner (j : integer) return integer is begin return base + j; end;
    impure function inner2 (j : integer) return integer is begin return inner(j) + inner(j + 1); end;
  begin
    return inner2(k) + inner2(base);
  end;

  procedure tick (by : integer) is begin c.bump(by); end;
  procedure tick2 (by : integer) is begin tick(by); tick(by + 1); end;
begin
  process
    variable acc : integer := 0;
  begin
    for i in 1 to 3000 loop
      acc := acc + f1(i mod 100);
      acc := acc + nested(i mod 50);
      tick2(i mod 7);
    end loop;
    report "acc=" & integer'image(acc) & " c=" & integer'image(c.get);
    if fail then
      acc := f1(60000);
      report "not reached " & integer'image(acc);
    end if;
    wait;
  end process;
end;
"#;

#[test]
fn calls_between_native_frames_fail_deep_in_a_chain_as_the_interpreter_does() {
    let dir = design("clift_chain", MIXED);
    for fail in ["-gfail=false", "-gfail=true"] {
        let (want, _) = parity(&dir, "mixed", &[fail]);
        assert!(want.out.contains("acc="), "{want:?}");
        assert_eq!(want.code != Some(0), fail == "-gfail=true", "{want:?}");
    }
}

/// One process: a loop of integer arithmetic, then the final wait. (It reads a generic, so it
/// can have a copy specialised to its instance data.) The process never comes back for the
/// switch to its copy, so the tier must not hold its shared code back for it.
const WARM: &str = "-- One process: a loop of integer arithmetic, then the final wait. (It reads a generic, so it
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

/// The compiles of a run, from its trace: (linked from the cache, microseconds the function was
/// seen running when it was compiled).
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

/// The cache entries of compiled Cranelift code under `dir`.
fn entries(dir: &Path) -> Vec<std::path::PathBuf> {
    let mut out = Vec::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        for e in std::fs::read_dir(&d).into_iter().flatten().flatten() {
            let p = e.path();
            if p.is_dir() {
                todo.push(p);
            } else if p
                .file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with("clif-"))
            {
                out.push(p);
            }
        }
    }
    out
}

/// The seen time (microseconds) from which the policy compiles the process, from the line it
/// ends its trace with.
fn candidate_at(err: &str) -> u64 {
    err.lines()
        .filter(|l| l.contains(": seen f") && l.contains("`_p0`"))
        .find_map(|l| {
            l.rsplit("candidate at ")
                .next()?
                .split(' ')
                .next()?
                .parse()
                .ok()
        })
        .unwrap_or_else(|| panic!("no candidate time of the process in {err}"))
}

/// `--tier=auto`, warm run: a process that runs one long loop and waits once never comes back for
/// the switch to its copy, so a second run links the code from the compilation cache as soon as
/// the process is hot, and from a threshold lower than the compile's: what the cache holds is
/// priced at a link, not held to the floor everything else waits for (the clock is work, so the
/// figures do not depend on the host's load).
#[test]
fn a_process_that_never_resumes_is_linked_from_the_cache_early() {
    let dir = design("clift_warm", WARM);
    let go = || {
        let o = yavs(
            &dir,
            &[("YAVS_AUTO_TRACE", "1")],
            &[
                "run",
                "--std=2008",
                "--work=work:work",
                "--tier=clift-auto",
                "auto_warm",
            ],
        );
        assert!(o.status.success(), "{o:?}");
        let err = String::from_utf8_lossy(&o.stderr).into_owned();
        (compiles(&err), candidate_at(&err))
    };
    // A run that ends before the compile thread got to the process compiles nothing: repeat it
    // on an empty cache, as clift_cache.rs does.
    let (mut cold, mut cold_at) = go();
    for _ in 0..4 {
        if !cold.is_empty() {
            break;
        }
        for e in entries(&dir) {
            std::fs::remove_file(e).expect("remove");
        }
        (cold, cold_at) = go();
    }
    assert!(!cold.is_empty(), "the first run compiled nothing");
    let (warm, warm_at) = go();
    assert!(
        matches!(warm[..], [(true, seen)] if seen < 300_000),
        "cold {cold:?}, warm {warm:?}"
    );
    assert!(warm_at < cold_at, "cold {cold_at} us, warm {warm_at} us");
}

/// A subprogram that loops for a long time in one call, made at the start of the run (large
/// enough not to be inlined, its argument a signal so the call is not folded).
const LOOPSUB: &str = "entity loopsub is generic (N : natural := 6000000); end;
architecture a of loopsub is
  signal len : natural := N;
  function churn(n : natural) return natural is
    variable a, b, c : natural := 0;
  begin
    for i in 1 to n loop
      a := (a * 3 + i mod 7) mod 1000003;
      if a mod 5 = 0 then b := (b + a) mod 65521; elsif a mod 3 = 1 then c := (c + i) mod 4093; end if;
      case i mod 4 is
        when 0 => b := (b * 7 + 1) mod 65521;
        when 1 => c := (c * 5 + 2) mod 4093;
        when 2 => a := (a + b) mod 1000003;
        when others => a := (a + c) mod 1000003;
      end case;
    end loop;
    return (a + b + c) mod 1000003;
  end;
begin
  process begin
    report \"a=\" & integer'image(churn(len));
    wait;
  end process;
end;
";

/// `--tier=auto`: a subprogram whose one call loops for most of the run is named hot by the run's
/// profile (the interpreter saw it running at two looks in a row), so the next run compiles it
/// early, frame-mode code with the native-frame code, and the call running by then goes on in
/// compiled code from a loop head. Every run prints what the interpreter prints.
#[test]
fn a_subprogram_that_loops_in_one_call_runs_compiled_in_the_next_run() {
    let dir = design("clift_loopsub", LOOPSUB);
    let want = run(&dir, "loopsub", &["--tier=interp"], &[]);
    let go = || {
        let o = yavs(
            &dir,
            &[("YAVS_AUTO_TRACE", "1"), ("YAVS_COUNTERS", "1")],
            &["run", "--std=2008", "--work=work:work", "loopsub"],
        );
        let err = String::from_utf8_lossy(&o.stderr).into_owned();
        let out = String::from_utf8_lossy(&o.stdout).into_owned();
        assert_eq!((o.status.code(), &out), (want.code, &want.out), "{err}");
        let count = |name: &str| {
            err.lines()
                .find_map(|l| l.strip_prefix(&format!("counters: {name}=")))
                .and_then(|v| v.parse::<u64>().ok())
                .unwrap_or_else(|| panic!("no {name} in {err}"))
        };
        // The microseconds `churn` ran compiled, from the line the trace ends with.
        let compiled = err
            .lines()
            .filter(|l| l.contains(": seen f") && l.contains("`churn`"))
            .find_map(|l| {
                l.split("compiled ")
                    .nth(1)?
                    .split(' ')
                    .next()?
                    .parse::<u64>()
                    .ok()
            })
            .unwrap_or(0);
        (
            count("tier_looped"),
            count("tier_loop_early"),
            compiled,
            err,
        )
    };
    let (looped, early, _, err) = go();
    assert!(looped >= 1, "{err}");
    assert_eq!(early, 0, "{err}");
    // A run is written to the profile only where the simulation CPU can be read (not on Windows):
    // elsewhere the next run decides as the first did, and only the outputs are compared.
    if !err.contains("profile: stored") {
        let (_, early, _, err) = go();
        assert_eq!(early, 0, "{err}");
        return;
    }
    let (_, early, compiled, err) = go();
    assert_eq!(early, 1, "{err}");
    assert!(compiled > 0, "{err}");
}
