//! Procedures and functions that drive signals, read vector signals or slices, run on native frames
//! of compiled code (`--tier=llvm`, `--tier=auto` compiling everything at once; a build with the
//! `llvm` feature) and give the interpreter's results: scalar, vector (also wider than 64
//! elements), integer, real, time, enumeration and array targets, delayed, transport and rejecting
//! assignments, a resolved signal with two drivers, nested procedures, reads of vectors of 8 to
//! 100 elements and of slices of an unconstrained signal parameter. Where the kernel's fast answer
//! is missing (an array of integers, a negative delay) the frames are materialised and the
//! interpreter runs the assignment, error and call trace included.
#![cfg(any(feature = "llvm", feature = "clift"))]

mod common;
use common::tempdir;

use std::path::Path;
use std::process::{Command, Output};

const SRC: &str = r#"library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity native_signals is end;
architecture a of native_signals is
  type st_t is (idle, run, done_st);
  signal sl, sl2 : std_logic := 'U';
  signal sv8 : std_logic_vector(7 downto 0) := (others => '0');
  signal sv100 : std_logic_vector(99 downto 0) := (others => '0');
  signal sv1 : std_logic_vector(0 downto 0) := "0";
  signal n : integer := 0;
  signal bo : boolean := false;
  signal st : st_t := idle;
  signal rl : real := 0.0;
  signal tm : time := 0 ns;
  signal iv : integer_vector(0 to 3) := (others => 0);
  signal res : std_logic;
  signal nested_n : integer := 0;
  signal nested_q : std_logic_vector(3 downto 0) := "0000";
  signal rd8 : std_logic_vector(7 downto 0) := x"a5";
  signal rd100 : std_logic_vector(99 downto 0) := (99 => '1', 70 => '1', 3 downto 0 => '1', others => '0');
  signal rd65 : std_logic_vector(64 downto 0) := (64 => '1', 1 => '1', others => '0');

  procedure p_sl (signal s : out std_logic; v : std_logic) is begin s <= v; end procedure;
  procedure p_after (signal s : out std_logic; v : std_logic; t : time) is begin s <= v after t; end procedure;
  procedure p_transport (signal s : out std_logic; v : std_logic) is begin s <= transport v after 3 ns; end procedure;
  procedure p_reject (signal s : out std_logic; v : std_logic) is begin s <= reject 1 ns inertial v after 3 ns; end procedure;
  procedure p_sv (signal s : out std_logic_vector; v : std_logic_vector) is begin s <= v; end procedure;
  procedure p_elem (signal s : out std_logic_vector; i : natural; v : std_logic) is begin s(i) <= v; end procedure;
  procedure p_n (signal s : out integer; v : integer) is begin s <= v; end procedure;
  procedure p_bo (signal s : out boolean; v : boolean) is begin s <= v; end procedure;
  procedure p_st (signal s : out st_t; v : st_t) is begin s <= v; end procedure;
  procedure p_rl (signal s : out real; v : real) is begin s <= v; end procedure;
  procedure p_tm (signal s : out time; v : time) is begin s <= v; end procedure;
  procedure p_iv (signal s : out integer_vector; v : integer_vector) is begin s <= v; end procedure;
  procedure p_two (signal a : out std_logic; signal b : out integer; x : integer) is
  begin
    if x mod 2 = 0 then a <= '1'; else a <= '0'; end if;
    b <= x;
  end procedure;

  function f_cnt (signal s : std_logic_vector) return integer is
    variable c : integer := 0;
  begin
    for k in s'range loop if s(k) = '1' then c := c + 1; end if; end loop;
    return c;
  end function;
  function f_low (signal s : std_logic_vector) return integer is
  begin
    return to_integer(unsigned(s(3 downto 0)));
  end function;
  function f_slice (signal s : std_logic_vector; hi, lo : natural) return integer is
    variable c : integer := 0;
  begin
    for k in lo to hi loop if s(k) = '1' then c := c + k; end if; end loop;
    return c;
  end function;
  function f_all (signal s : std_logic_vector) return std_logic_vector is
  begin
    return s;
  end function;
begin
  -- a second driver of `res`, and a reader process
  process begin
    res <= 'Z';
    wait for 4 ns;
    res <= '0';
    wait for 7 ns;
    res <= 'Z';
    wait;
  end process;

  process
    procedure nest (k : integer) is
    begin
      nested_n <= k * 3;
      if k mod 2 = 0 then nested_q(k mod 4) <= '1'; else nested_q(k mod 4) <= '0'; end if;
    end procedure;
    variable acc : integer := 0;
    variable v100 : std_logic_vector(99 downto 0);
  begin
    for i in 0 to 29 loop
      p_sl(sl, std_logic'val(i mod 9));
      p_after(sl2, std_logic'val((i + 3) mod 9), (i mod 3) * 2 ns);
      p_sv(sv8, std_logic_vector(to_unsigned(i * 7, 8)));
      v100 := (others => '0'); v100(i) := '1'; v100(99 - i) := '1';
      p_sv(sv100, v100);
      p_elem(sv8, i mod 8, '1');
      p_n(n, i * i - 50);
      p_bo(bo, i mod 2 = 0);
      p_st(st, st_t'val(i mod 3));
      p_rl(rl, real(i) / 4.0);
      p_tm(tm, i * 1 ns);
      p_iv(iv, (i, i + 1, i + 2, -i));
      p_two(sl2, n, i);
      p_transport(sl, std_logic'val((i + 1) mod 9));
      p_reject(res, '1');
      p_sv(sv1, "1");
      nest(i);
      acc := acc + f_cnt(rd8) + f_low(rd8) + f_cnt(rd100) + f_low(rd100) + f_cnt(rd65) + f_slice(rd100, 7, 2) + f_slice(rd8, 5, 1);
      acc := acc + f_cnt(sv8) + f_cnt(sv100) + to_integer(unsigned(f_all(rd8)));
      wait for 1 ns;
      report integer'image(i) & " " & std_logic'image(sl) & std_logic'image(sl2) & " " & to_string(sv8) & " " & to_string(sv100)
        & " " & integer'image(n) & " " & boolean'image(bo) & " " & st_t'image(st) & " " & real'image(rl) & " " & time'image(tm)
        & " " & integer'image(iv(0)) & integer'image(iv(3)) & " " & std_logic'image(res) & " " & integer'image(nested_n)
        & " " & to_string(nested_q) & " " & to_string(sv1) & " " & integer'image(acc);
    end loop;
    wait;
  end process;
end;
"#;

const NEG: &str = r#"library ieee; use ieee.std_logic_1164.all;
entity neg is end;
architecture a of neg is
  signal n : integer := 0;
  signal s : std_logic := '0';
  procedure p_n (signal x : out integer; v : integer; t : time) is begin x <= v after t; end procedure;
  procedure p_s (signal x : out std_logic; v : std_logic; t : time) is begin x <= v after t; end procedure;
begin
  process begin
    for i in 0 to 9 loop
      p_n(n, i, i * 1 ns);
      p_s(s, '1', i * 1 ns);
      wait for 20 ns;
    end loop;
    report "before";
    p_s(s, '0', -1 ns);
    report "unreachable";
    wait;
  end process;
end;
"#;

fn yavs(dir: &Path, args: &[&str], counters: bool) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_yavs"));
    c.args(args).current_dir(dir);
    if counters {
        c.env("YAVS_COUNTERS", "kinds");
    }
    c.output().expect("run yavs")
}

fn analyze(dir: &Path, file: &str) {
    let a = yavs(
        dir,
        &["analyze", "--std=2008", "--work=work:work", file],
        false,
    );
    assert!(a.status.success(), "{a:?}");
}

/// The value of counter `name` of a run with `YAVS_COUNTERS=kinds` (0 when it is not printed).
fn counter(stderr: &str, name: &str) -> u64 {
    let key = format!("counters: {name}=");
    stderr
        .lines()
        .find_map(|l| l.strip_prefix(&key)?.parse().ok())
        .unwrap_or(0)
}

const TIERS: &[&[&str]] = &[
    &["--tier=interp"],
    #[cfg(feature = "llvm")]
    &["--tier=llvm"],
    #[cfg(feature = "llvm")]
    &["--tier=auto", "--tier-heat=1,0,0"],
    #[cfg(feature = "llvm")]
    &["--tier=auto", "--tier-heat=1,0,0", "--threads=4"],
    #[cfg(feature = "clift")]
    &["--tier=clift"],
    #[cfg(feature = "clift")]
    &["--tier=clift-auto", "--tier-heat=1,0,0", "--threads=4"],
    #[cfg(all(feature = "llvm", feature = "clift"))]
    &["--tier=auto3", "--tier-heat=1,0,0"],
];

fn run(dir: &Path, flags: &[&str], top: &str) -> Output {
    let mut args = vec!["run", "--std=2008", "--work=work:work"];
    args.extend_from_slice(flags);
    args.push(top);
    yavs(dir, &args, true)
}

#[test]
fn signal_procedures_and_vector_functions_on_native_frames_match_the_interpreter() {
    let dir = tempdir("compiled_native_signals");
    std::fs::write(dir.join("native_signals.vhd"), SRC).expect("write");
    analyze(&dir, "native_signals.vhd");
    let want = run(&dir, TIERS[0], "native_signals");
    assert!(want.status.success(), "{want:?}");
    let want = String::from_utf8_lossy(&want.stdout).into_owned();
    assert!(want.lines().count() >= 60, "{want}");
    for flags in &TIERS[1..] {
        let o = run(&dir, flags, "native_signals");
        assert!(o.status.success(), "{flags:?}: {o:?}");
        assert_eq!(String::from_utf8_lossy(&o.stdout), want, "{flags:?}");
        if flags == &["--tier=llvm"] || flags == &["--tier=clift"] {
            // The procedures and functions run on native frames: the calls are not steps of
            // the interpreter (only the rejecting procedure and the report calls are).
            let err = String::from_utf8_lossy(&o.stderr);
            if flags == &["--tier=llvm"] {
                assert!(counter(&err, "llvm_native_frames") >= 14, "{err}");
            }
            assert!(counter(&err, "cb_step.Call") <= 32, "{err}");
        }
    }
}

#[test]
fn a_failing_assignment_on_a_native_frame_reports_as_the_interpreter_does() {
    let dir = tempdir("compiled_native_signals_neg");
    std::fs::write(dir.join("neg.vhd"), NEG).expect("write");
    analyze(&dir, "neg.vhd");
    let text = |flags: &[&str]| {
        let mut args = vec!["run", "--std=2008", "--work=work:work"];
        args.extend_from_slice(flags);
        args.push("neg");
        let o = yavs(&dir, &args, false);
        assert!(!o.status.success(), "{o:?}");
        let mut s = String::from_utf8_lossy(&o.stdout).into_owned();
        s.push_str(&String::from_utf8_lossy(&o.stderr));
        // The directory of the test is in the source path of the diagnostic.
        s.replace(dir.to_str().expect("utf-8 path"), "DIR")
    };
    let want = text(TIERS[0]);
    assert!(want.contains("E3009"), "{want}");
    assert!(want.contains("called from here"), "{want}");
    for flags in &TIERS[1..] {
        assert_eq!(text(flags), want, "{flags:?}");
    }
}
