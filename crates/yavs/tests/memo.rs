//! Values that depend only on generics and generate parameters are computed once per instance
//! by a process (`yavs-ir` `opt/memo.rs`): the run is the same as without it, and a failing
//! check fails at the same time with the same text.

mod common;
use common::tempdir as tmp;

use std::path::Path;
use std::process::Command;

const GOOD: &str = "
entity good is generic (G : natural := 5); end;
architecture a of good is
  signal clk : bit := '0';
  signal v : bit_vector(0 to 3) := \"1010\";
  signal o : bit_vector(0 to 3);
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;
  gen: for i in 0 to 3 generate
    process (clk) begin
      if clk = '1' then o(i) <= v((i + G) mod 4) xor v((i + 1) mod 4); end if;
    end process;
  end generate;
  process begin
    wait for 100 ns;
    report \"o=\" & to_string(o);
    done <= true; wait;
  end process;
end;
";

const BAD: &str = "
entity bad is generic (G : natural := 5); end;
architecture a of bad is
  signal clk : bit := '0';
  signal v : bit_vector(0 to 2) := \"101\";
  signal o : bit_vector(0 to 2);
begin
  clk <= not clk after 5 ns;
  gen: for i in 0 to 2 generate
    process (clk)
      variable n : natural := 0;
    begin
      if clk = '1' then
        n := n + 1;
        if n = 3 then o(i) <= v((i + G) mod 4); end if;
      end if;
    end process;
  end generate;
end;
";

fn run(dir: &Path, top: &str, src: &str, no_memo: bool) -> (i32, String, String) {
    std::fs::write(dir.join(format!("{top}.vhd")), src).expect("write");
    let work = format!("work:{}", dir.join("lib").display());
    let yavs = env!("CARGO_BIN_EXE_yavs");
    let a = Command::new(yavs)
        .current_dir(dir)
        .args(["analyze", "--work", &work, &format!("{top}.vhd")])
        .output()
        .expect("analyze");
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    let mut c = Command::new(yavs);
    c.current_dir(dir)
        .args(["--color", "never", "run", "--no-cache", "--work", &work])
        .args(["--stop-time=200ns", top]);
    if no_memo {
        c.env("YAVS_NO_MEMO", "1");
    }
    let o = c.output().expect("run");
    (
        o.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

#[test]
fn instance_constants_do_not_change_a_run() {
    let d = tmp("good");
    let with = run(&d, "good", GOOD, false);
    let without = run(&d, "good", GOOD, true);
    assert_eq!(with.0, 0, "{with:?}");
    assert!(with.1.contains("o=") || with.2.contains("o="), "{with:?}");
    assert_eq!(with, without);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_failing_instance_check_fails_at_the_same_time() {
    let d = tmp("bad");
    let with = run(&d, "bad", BAD, false);
    let without = run(&d, "bad", BAD, true);
    assert_ne!(with.0, 0, "{with:?}");
    assert!(
        with.2.contains("25 ns") || with.2.contains("25000000fs"),
        "{with:?}"
    );
    assert_eq!(with, without);
    let _ = std::fs::remove_dir_all(&d);
}

/// Slice and concatenation of array elements (#534): the optimiser shares the positions of the
/// element across the arms of the `if`, which must still leave the memoised offsets of the
/// instance (slots read by fused ops) and the waveform of the assignment built, copied into and
/// driven by one op (`CopyDrive`), not by a `WaveBuf`, a `Copy` and a `Drive` over block ends.
const LFSR: &str = "
library ieee; use ieee.std_logic_1164.all;
entity lfsr is end;
architecture a of lfsr is
  signal clk : std_logic := '0';
  type arr is array (natural range <>) of std_logic_vector(15 downto 0);
  signal r : arr(0 to 3);
begin
  clk <= not clk after 5 ns;
  g: for i in 0 to 3 generate
    process (clk) begin
      if rising_edge(clk) then
        if r(i)(0) = 'U' then
          r(i) <= x\"ACE1\";
        else
          r(i) <= r(i)(14 downto 0) & (r(i)(15) xor r(i)(13) xor r(i)(12) xor r(i)(10));
        end if;
      end if;
    end process;
  end generate;
end;
";

#[test]
fn a_shifted_element_assignment_keeps_its_fused_ops() {
    let d = tmp("lfsr");
    std::fs::write(d.join("lfsr.vhd"), LFSR).expect("write");
    let work = format!("work:{}", d.join("lib").display());
    let yavs = env!("CARGO_BIN_EXE_yavs");
    let a = Command::new(yavs)
        .current_dir(&d)
        .args(["analyze", "--std=2008", "--work", &work, "lfsr.vhd"])
        .output()
        .expect("analyze");
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    let o = Command::new(yavs)
        .current_dir(&d)
        .env("YAVS_DUMPIR", "1")
        .args([
            "--color",
            "never",
            "run",
            "--no-cache",
            "--std=2008",
            "--work",
            &work,
        ])
        .args(["--tier=interp", "--stop-time=100ns", "lfsr"])
        .output()
        .expect("run");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let err = String::from_utf8_lossy(&o.stderr);
    for want in ["SigRead1Slot", "CopyDrive"] {
        assert!(err.contains(want), "no {want}:\n{err}");
    }
    assert!(!err.contains("WaveBuf"), "{err}");
    let _ = std::fs::remove_dir_all(&d);
}

/// Slices with generic bounds (the descriptors the process builds are instance-constant, so the
/// process keeps them in its frame) and the null slice (which checks nothing): the run is the
/// same as without the memoisation, the slice that fails fails at the same time with the same
/// text, and the direction of a slice is checked against the array's.
const SLICES: &str = "
entity slices is generic (HI : natural := 5; LO : natural := 2; BAD : natural := 99); end;
architecture a of slices is
  signal clk : bit := '0';
  signal v : bit_vector(7 downto 0) := \"10110100\";
  signal o : bit_vector(HI downto LO);
  signal n : natural := 0;
begin
  clk <= not clk after 5 ns;
  process (clk) begin
    if clk = '1' then
      n <= n + 1;
      o <= v(HI downto LO) xor v(HI - LO + 2 downto 2);
      if n = 3 then o <= v(BAD downto LO); end if;
      if n = 2 then o(LO + 1 downto LO + 2) <= \"\"; end if;
    end if;
  end process;
  process (v(HI downto LO)) begin report \"slice sensitivity\"; end process;
end;
";

#[test]
fn slices_of_generic_bounds_do_not_change_a_run() {
    let d = tmp("slices");
    let with = run(&d, "slices", SLICES, false);
    let without = run(&d, "slices", SLICES, true);
    assert_ne!(with.0, 0, "{with:?}");
    // The fourth rising edge, at 35 ns, is the slice outside the array.
    assert!(
        with.2.contains("35 ns") || with.2.contains("35000000fs"),
        "{with:?}"
    );
    assert!(with.2.contains("outside of range"), "{with:?}");
    assert_eq!(with, without);
    let _ = std::fs::remove_dir_all(&d);
}

/// Whole-array signal copies sized by generics, one after the other in a clocked process: the
/// sizes of the waveforms are instance-constant (memoised), and the waveform of each copy is
/// built, filled and driven by one op whose size register must still hold its own size.
const COPIES: &str = "
entity copies is generic (N : natural := 9); end;
architecture a of copies is
  signal clk : bit := '0';
  type arr_t is array (0 to N - 1) of integer;
  signal a, b, c, d : arr_t := (others => 3);
  signal e : arr_t := (others => 0);
begin
  clk <= not clk after 5 ns;
  process (clk) begin
    if clk = '1' then
      a <= e;
      b <= a;
      c <= b;
      d <= c;
      e <= (others => e(0) + 1);
    end if;
  end process;
  process begin
    wait for 100 ns;
    report integer'image(a(0)) & \" \" & integer'image(b(N - 1)) & \" \" & integer'image(d(4));
    wait;
  end process;
end;
";

#[test]
fn memoised_array_copies_drive_their_own_sizes() {
    let d = tmp("copies");
    let with = run(&d, "copies", COPIES, false);
    let without = run(&d, "copies", COPIES, true);
    assert_eq!(with.0, 0, "{with:?}");
    assert!(
        with.1.contains("9 8 6") || with.2.contains("9 8 6"),
        "{with:?}"
    );
    assert_eq!(with, without);
    let _ = std::fs::remove_dir_all(&d);
}

/// References to parts of instance signals (`SigVal`) whose bounds are instance-constant are
/// remembered by the process like the numbers they come from; a reference to one element is
/// not, as the interpreter fuses it with the read or wait that uses it. The run is the same as
/// without the memoisation.
const REFS: &str = "
entity refs is generic (W : natural := 4; OFF : natural := 2); end;
architecture a of refs is
  signal clk : bit := '0';
  signal v : bit_vector(0 to 7) := \"10110010\";
  signal o : bit_vector(0 to 7) := (others => '0');
  signal q : bit_vector(0 to 3) := \"0110\";
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;
  process (clk) begin
    if clk = '1' then
      v <= not v;
      o(OFF to OFF + W - 1) <= v(0 to W - 1) xor v(OFF to OFF + W - 1);
    end if;
  end process;
  g: for i in 0 to 2 generate
    process (clk) begin
      if clk = '1' then q(i + 1) <= not q(i); end if;
    end process;
  end generate;
  process (v(OFF to OFF + W - 1)) begin report \"change \" & to_string(v(OFF to OFF + W - 1)); end process;
  process begin
    wait for 60 ns;
    report \"o=\" & to_string(o) & \" q=\" & to_string(q);
    done <= true; wait;
  end process;
end;
";

#[test]
fn signal_references_of_instance_constant_bounds_do_not_change_a_run() {
    let d = tmp("refs");
    let with = run(&d, "refs", REFS, false);
    let without = run(&d, "refs", REFS, true);
    assert_eq!(with.0, 0, "{with:?}");
    assert!(with.1.contains("o=") || with.2.contains("o="), "{with:?}");
    assert_eq!(with, without);
    // The clocked processes of single elements keep the fused read and wait; the references
    // of the vector slices are loaded from the frame.
    let yavs = env!("CARGO_BIN_EXE_yavs");
    let work = format!("work:{}", d.join("lib").display());
    let o = Command::new(yavs)
        .current_dir(&d)
        .env("YAVS_DUMPIR", "1")
        .args(["--color", "never", "run", "--no-cache", "--work", &work])
        .args(["--tier=interp", "--stop-time=60ns", "refs"])
        .output()
        .expect("run");
    let err = String::from_utf8_lossy(&o.stderr);
    for want in ["SigRead1Slot", "WaitSuspend", "LoadSlot8x2"] {
        assert!(err.contains(want), "no {want}:\n{err}");
    }
    let _ = std::fs::remove_dir_all(&d);
}
