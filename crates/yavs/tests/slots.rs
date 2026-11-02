//! Constant-size temporaries of a process in its frame (`yavs_ir` `opt/slots.rs`, #668): the
//! waveforms of signal assignments, vectors read from signals, concatenations, descriptors and the
//! `'last_value` of a narrow integer element live in frame bytes, and the fused ops of both tiers
//! take the frame forms. Each design runs with and without the pass and must print the same.

mod common;
use common::tempdir as tmp;

use std::process::Command;

const DESIGN: &str = "
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity t is end;
architecture a of t is
  type st_t is (idle, run, done);
  signal clk : std_logic := '0';
  signal q, r : std_logic := '0';
  signal v, w : std_logic_vector(7 downto 0) := x\"01\";
  signal n : integer := 0;
  signal iv : integer_vector(0 to 1) := (-1, -2);
  signal st : st_t := idle;
  signal o1, o2, o3 : std_logic := '0';
begin
  clk <= not clk after 5 ns when now < 400 ns;
  process (clk) begin
    if rising_edge(clk) then
      q <= not q;
      r <= q after 2 ns;
      v <= std_logic_vector(unsigned(v) + 1);
      w <= v(3 downto 0) & w(7 downto 4);
      n <= n + 1 after 1 ns;
      iv(1) <= iv(1) - 1;
      st <= run when st = idle else done when st = run else idle;
      o1 <= q; o2 <= r; o3 <= v(0);
    end if;
  end process;
  process begin
    wait for 395 ns;
    report \"q=\" & std_logic'image(q) & \" r=\" & std_logic'image(r) & \" n=\" & integer'image(n)
      & \" v=\" & to_hstring(v) & \" w=\" & to_hstring(w) & \" st=\" & st_t'image(st)
      & \" iv=\" & integer'image(iv(1)) & \" last=\" & integer'image(iv(1)'last_value)
      & \" o=\" & std_logic'image(o1) & std_logic'image(o2) & std_logic'image(o3);
    wait;
  end process;
end;
";

fn run(dir: &std::path::Path, env: &[(&str, &str)], tier: &str) -> (i32, String) {
    let yavs = env!("CARGO_BIN_EXE_yavs");
    let work = format!("work:{}", dir.join("lib").display());
    let a = Command::new(yavs)
        .current_dir(dir)
        .args(["analyze", "--std=2008", "--work", &work, "t.vhd"])
        .output()
        .expect("analyze");
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    let o = Command::new(yavs)
        .current_dir(dir)
        .envs(env.iter().copied())
        .args(["--color", "never", "run", "--no-cache", "--work", &work])
        .args([&format!("--tier={tier}"), "t"])
        .output()
        .expect("run");
    (
        o.status.code().unwrap_or(-1),
        format!(
            "{}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        ),
    )
}

fn same_output_with_and_without_the_pass(tier: &str) {
    let d = tmp("slots");
    std::fs::write(d.join("t.vhd"), DESIGN).expect("write");
    let with = run(&d, &[], tier);
    let without = run(&d, &[("YAVS_NO_SLOTS", "1")], "interp");
    assert_eq!(with.0, 0, "{}", with.1);
    assert!(with.1.contains("iv=-41 last=-40"), "{}", with.1);
    assert_eq!(with, without, "{tier}");
}

#[test]
fn frame_temporaries_change_nothing_anyone_sees() {
    same_output_with_and_without_the_pass("interp");
    // The temporaries are in the frame: zeroed there, not allocated.
    let d = tmp("slots_dump");
    std::fs::write(d.join("t.vhd"), DESIGN).expect("write");
    let (_, dump) = run(&d, &[("YAVS_DUMPIR", "1")], "interp");
    assert!(dump.contains("Zero { addr:"), "{dump}");
    assert!(
        dump.contains("LSetDrive {") && dump.contains("size: 32808"),
        "{dump}"
    );
}

/// Scalar byte signals (integer, real, enumeration) assigned at zero delay take `ByteDrive`;
/// a delay, a second assignment of the run, a resolved signal and an element of an array
/// take its replay or the kernel's other paths, and print what the unfused ops print.
const BYTES: &str = "
entity t is end;
architecture a of t is
  type st_t is (idle, run, done);
  type res_t is array (natural range <>) of integer;
  function rf(v : res_t) return integer is
    variable m : integer := integer'low;
  begin
    for i in v'range loop
      if v(i) > m then m := v(i); end if;
    end loop;
    return m;
  end;
  subtype rint is rf integer;
  signal clk : bit := '0';
  signal c, same, twice, late, tr, narrow : integer := 0;
  signal x : real := 0.0;
  signal st : st_t := idle;
  signal arr : integer_vector(0 to 3) := (others => 7);
  signal bus1 : rint := 0;
  signal chk : integer := 0;
begin
  clk <= not clk after 5 ns when now < 400 ns;
  process (clk)
    variable n : integer := 0;
  begin
    if clk = '1' then
      n := n + 1;
      c <= n;
      same <= 5;
      twice <= 1;
      twice <= n mod 3;
      late <= n after 3 ns;
      tr <= transport n;
      narrow <= n mod 7 when n < 30 else 0;
      x <= real(n) * 0.5;
      st <= run when st = idle else done when st = run else idle;
      arr(n mod 4) <= n;
      bus1 <= n;
    end if;
  end process;
  process (clk) begin
    if clk = '1' then bus1 <= 3; end if;
  end process;
  -- every event with its time and the values, so a wrong delay shows
  process (c, twice, late, tr, narrow, x, st, arr, bus1)
    variable h : integer := 0;
  begin
    h := (h * 31 + (now / 1 ns) + c + twice * 3 + late * 5 + tr * 7 + narrow * 11 + integer(x * 2.0)
      + st_t'pos(st) * 13 + arr(0) + arr(1) * 2 + arr(2) * 3 + arr(3) * 4 + bus1 * 17) mod 1000003;
    chk <= h;
  end process;
  process begin
    wait for 395 ns;
    report \"chk=\" & integer'image(chk) & \" c=\" & integer'image(c) & \" same=\" & integer'image(same)
      & \" twice=\" & integer'image(twice) & \" late=\" & integer'image(late)
      & \" tr=\" & integer'image(tr) & \" narrow=\" & integer'image(narrow)
      & \" x=\" & real'image(x) & \" st=\" & st_t'image(st)
      & \" arr=\" & integer'image(arr(0)) & integer'image(arr(1)) & integer'image(arr(2))
      & integer'image(arr(3)) & \" bus=\" & integer'image(bus1);
    wait;
  end process;
end;
";

#[test]
fn byte_drives_run_as_one_op_and_replay_what_the_fast_path_leaves() {
    let d = tmp("slots_bytes");
    std::fs::write(d.join("t.vhd"), BYTES).expect("write");
    let with = run(&d, &[], "interp");
    let without = run(&d, &[("YAVS_NO_SLOTS", "1")], "interp");
    assert_eq!(with.0, 0, "{}", with.1);
    assert!(with.1.contains("c=39 same=5"), "{}", with.1);
    assert_eq!(with, without);
    let (_, dump) = run(&d, &[("YAVS_DUMPIR", "1")], "interp");
    assert!(
        dump.contains("ByteDrive {") && dump.contains("size: 32784"),
        "{dump}"
    );
}

#[cfg(feature = "llvm")]
#[test]
fn the_same_in_compiled_code() {
    same_output_with_and_without_the_pass("llvm");
    same_output_with_and_without_the_pass("auto");
}
