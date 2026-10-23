//! Element loops (#520): loops over the elements of logic vectors that copy or fill run all
//! iterations but the last at once when every index of them is in range, and the generic ops run
//! the rest, so results and failures are those of the plain loop. Each function below is a
//! loop of a different shape; the test checks the results, that the iterations ran at once
//! (`YAVS_DUMPIR` says what each entry of a loop did) and the message of a loop that leaves the vector.

mod common;
use common::tempdir as tmp;

use std::process::Command;

const LOOPS: &str = "
library ieee; use ieee.std_logic_1164.all;
entity loops is end;
architecture a of loops is
  type slm is array (natural range <>, natural range <>) of std_logic;
  function img(v : std_logic_vector) return string is
    variable s : string(1 to v'length);
    variable j : natural := 1;
  begin
    for i in v'range loop s(j) := std_logic'image(v(i))(2); j := j + 1; end loop;
    return s;
  end function;
  function copy(a : std_logic_vector) return std_logic_vector is
    variable r : std_logic_vector(a'range);
  begin
    for i in a'range loop r(i) := a(i); end loop;
    return r;
  end function;
  function rev(a : std_logic_vector) return std_logic_vector is
    variable r : std_logic_vector(a'range);
  begin
    for i in a'range loop r(i) := a(a'high + a'low - i); end loop;
    return r;
  end function;
  function fill(n : natural; c : std_logic) return std_logic_vector is
    variable r : std_logic_vector(n - 1 downto 0) := (others => '0');
  begin
    for i in r'range loop r(i) := c; end loop;
    return r;
  end function;
  function evens(a : std_logic_vector) return std_logic_vector is
    variable r : std_logic_vector(a'range) := (others => '-');
  begin
    for i in 0 to a'length / 2 - 1 loop r(i * 2) := a(i); end loop;
    return r;
  end function;
  function row(m : slm; k : natural) return std_logic_vector is
    variable r : std_logic_vector(m'range(2));
  begin
    for i in r'range loop r(i) := m(k, i); end loop;
    return r;
  end function;
  function over(a : std_logic_vector; n : natural) return std_logic_vector is
    variable r : std_logic_vector(a'range) := a;
  begin
    for i in 0 to n loop r(i) := a(i); end loop;
    return r;
  end function;
  procedure shift(v : inout std_logic_vector) is
  begin
    for i in v'low to v'high - 1 loop v(i + 1) := v(i); end loop;
  end procedure;
  procedure shiftdown(v : inout std_logic_vector) is
  begin
    for i in v'low to v'high - 1 loop v(i) := v(i + 1); end loop;
  end procedure;
begin
  process
    variable c70 : std_logic_vector(0 to 69) :=
      \"1100110011001100110011001100110011001100110011001100110011001100110011\";
    variable v : std_logic_vector(15 downto 0) := \"10XZ01LH-WU1100X\";
    variable w : std_logic_vector(0 to 15);
    variable m : slm(0 to 2, 7 downto 0) := (others => (others => '0'));
    variable u : std_logic_vector(7 downto 0);
  begin
    report \"copy \" & img(copy(v));
    report \"copy70 \" & img(copy(c70));
    report \"rev \" & img(rev(v));
    report \"fill \" & img(fill(40, '1'));
    report \"evens \" & img(evens(v));
    for k in 0 to 2 loop
      for j in 7 downto 0 loop m(k, j) := v(j + k); end loop;
    end loop;
    report \"row1 \" & img(row(m, 1));
    report \"row2 \" & img(row(m, 2));
    report \"over \" & img(over(v, 15));
    w := v; shift(w); report \"shift \" & img(w);
    w := v; shiftdown(w); report \"shiftdown \" & img(w);
    u := (others => '0');
    for i in 3 to 3 loop u(i) := '1'; end loop;
    for i in 3 to 4 loop u(i) := 'H'; end loop;
    for i in 7 downto 5 loop u(i) := 'L'; end loop;
    for i in 4 to 2 loop u(i) := 'X'; end loop;
    report \"short \" & img(u);
    report \"over \" & img(over(v, 16));
    wait;
  end process;
end;
";

fn run(d: &std::path::Path, env_dump: bool) -> (String, String) {
    let yavs = env!("CARGO_BIN_EXE_yavs");
    let work = format!("work:{}", d.join("lib").display());
    let a = Command::new(yavs)
        .current_dir(d)
        .args(["analyze", "--work", &work, "loops.vhd"])
        .output()
        .expect("analyze");
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    let mut c = Command::new(yavs);
    c.current_dir(d)
        .args(["--color", "never", "run", "--no-cache", "--work", &work])
        .args(["--tier=interp", "loops"]);
    if env_dump {
        c.env("YAVS_DUMPIR", "1");
    }
    let o = c.output().expect("run");
    (
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

#[test]
fn element_loops_give_the_results_of_the_plain_loops() {
    let d = tmp("loops");
    std::fs::write(d.join("loops.vhd"), LOOPS).expect("write");
    let (out, err) = run(&d, true);
    let all = format!("{out}\n{err}");
    // Worked out by hand from the sources above.
    for want in [
        "copy 10XZ01LH-WU1100X",
        "copy70 1100110011001100110011001100110011001100110011001100110011001100110011",
        "rev X0011UW-HL10ZX01",
        "fill 1111111111111111111111111111111111111111",
        "evens ---W-U-1-1-0-0-X",
        "row1 H-WU1100",
        "row2 LH-WU110",
        "over 10XZ01LH-WU1100X",
        "shift 1111111111111111",
        "shiftdown 0XZ01LH-WU1100XX",
        "short LLLHH000",
    ] {
        assert!(all.contains(want), "no `{want}`\n{all}");
    }
    // The loops over more than two elements ran at once, all iterations but the last.
    let at_once: Vec<_> = err
        .lines()
        .filter(|l| l.contains("iterations at once"))
        .collect();
    for want in [
        "15 of 15", "69 of 69", "39 of 39", "7 of 7", "14 of 14", "16 of 16",
    ] {
        assert!(
            at_once.iter().any(|l| l.contains(want)),
            "no loop with {want} iterations at once\n{err}"
        );
    }
    // A loop that leaves the vector fails at the same index as the plain loop would.
    assert!(
        all.contains("index 16 outside of range 15 downto 0 for a"),
        "{all}"
    );
}
