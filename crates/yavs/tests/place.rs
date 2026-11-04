//! Logic rows and slices read in place (`yavs_ir` `opt/place.rs`): every design runs on each tier
//! and must print what it printed before the pass, the checks in the same order, the same error at
//! the same place, and overlapping copies as if the source had been read first. The pass itself
//! is tested on IR in `crates/yavs-ir/tests/opt_place.rs`.

mod common;
use common::tempdir as tmp;

use std::process::Command;

const VALUES: &str = r#"
library ieee;
use ieee.std_logic_1164.all;
entity t is end;
architecture a of t is
  subtype word is std_logic_vector(31 downto 0);
  type words is array (0 to 3) of word;
  type rec is record
    tag : integer;
    cfg : words;
  end record;
  type recs is array (natural range <>) of rec;
  signal sw : words := (others => x"0000_00A5");
  function pick(x : recs; i, j, b : integer) return std_logic is
  begin
    return x(i).cfg(j)(b);
  end;
  function pick16(x : recs; i, j : integer) return std_logic is
  begin
    return x(i).cfg(j)(16);
  end;
  -- The second 16 is a constant merged into the first, and the only reader of it the new offset.
  function both16(x : recs; i, j : integer) return std_logic is
  begin
    return x(i).cfg(j)(16) and x(j).cfg(i)(16);
  end;
begin
  process
    variable rs : recs(0 to 2);
    variable r : rec;
    variable m : words := (others => (others => '0'));
    variable v, w : std_logic_vector(0 to 199);
    variable s4 : std_logic_vector(3 downto 0);
    variable s0 : std_logic_vector(1 to 0);
    variable k : integer := 2;
    variable c : integer := 0;
    impure function bump return integer is
    begin
      r.cfg(1)(5) := '1';
      return 5;
    end;
    impure function count return integer is
    begin
      c := c + 1;
      return c;
    end;
  begin
    rs(1).cfg(2)(16) := '1';
    assert pick16(rs, 1, 2) = '1' report "field row constant" severity failure;
    assert pick16(rs, 0, 2) = 'U' report "other row" severity failure;
    rs(2).cfg(1)(16) := '1';
    assert both16(rs, 1, 2) = '1' report "merged constant index" severity failure;
    assert pick(rs, 1, 2, k + 14) = '1' report "field row variable" severity failure;
    m(2)(k) := '1';
    assert m(k)(k) = '1' and m(k)(k + 1) = '0' report "matrix" severity failure;
    assert m(2)(7 downto 0)(2) = '1' report "slice of a row" severity failure;
    assert m(2)(7 downto 1)(2) = '1' and m(2)(7 downto 3)(4) = '0' report "offset slice" severity failure;
    s4 := m(2)(3 downto 0);
    assert s4 = "0100" report "row slice into a variable" severity failure;
    s4 := m(2)(4 downto 1);
    assert s4 = "0010" report "offset slice into a variable" severity failure;
    s0 := m(2)(1 to 0);
    assert sw(1)(7) = '1' and sw(1)(6) = '0' report "signal row" severity failure;
    s4 := sw(2)(5 downto 2);
    assert s4 = "1001" report "signal slice" severity failure;

    -- The index writes what the prefix names: as without the pass.
    r.cfg := (others => (others => '0'));
    report "impure index " & std_logic'image(r.cfg(1)(bump));
    -- Index functions run once each, prefix first.
    r.cfg := (others => (others => '0'));
    r.cfg(1)(2) := '1';
    assert r.cfg(count)(count) = '1' report "order of the indices" severity failure;
    assert c = 2 report "indices run once" severity failure;

    -- Overlap: the value is the source before the assignment.
    for i in v'range loop
      v(i) := std_logic'val(2 + (i * 7 + i / 3) mod 2);
    end loop;
    w := v;
    v(0 to 3) := v(1 to 4);
    assert v(0 to 3) = w(1 to 4) and v(4) = w(4) report "narrow down" severity failure;
    w := v;
    v(1 to 4) := v(0 to 3);
    assert v(1 to 4) = w(0 to 3) and v(0) = w(0) report "narrow up" severity failure;
    w := v;
    v(0 to 149) := v(30 to 179);
    assert v(0 to 149) = w(30 to 179) and v(150) = w(150) report "wide down" severity failure;
    w := v;
    v(30 to 179) := v(0 to 149);
    assert v(30 to 179) = w(0 to 149) and v(0) = w(0) report "wide up" severity failure;
    m(1) := x"8421_00FF";
    m(1)(7 downto 0) := m(1)(8 downto 1);
    assert m(1) = x"8421_007F" report "row onto itself" severity failure;
    m(0) := m(1)(31 downto 0);
    assert m(0) = x"8421_007F" report "row into a row" severity failure;
    report "done";
    wait;
  end process;
end;
"#;

/// The assignment's length check, not a shorter copy.
const LENGTH: &str = r#"
library ieee;
use ieee.std_logic_1164.all;
entity t is end;
architecture a of t is
begin
  process
    type words is array (0 to 3) of std_logic_vector(31 downto 0);
    variable m : words := (others => (others => '1'));
    variable s4 : std_logic_vector(3 downto 0);
    variable n : integer := 4;
  begin
    s4 := m(1)(n downto 0);
    report "not reached";
    wait;
  end process;
end;
"#;

/// Two bad indices in one name: the left one reports.
const ORDER: &str = r#"
library ieee;
use ieee.std_logic_1164.all;
entity t is end;
architecture a of t is
begin
  process
    type words is array (0 to 3) of std_logic_vector(31 downto 0);
    type rec is record
      cfg : words;
    end record;
    variable r : rec;
    variable i : integer := 7;
    variable j : integer := 40;
    variable b : std_logic;
  begin
    b := r.cfg(i)(j);
    report "not reached";
    wait;
  end process;
end;
"#;

fn run(dir: &std::path::Path, tier: &str) -> (i32, String) {
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

/// What a design prints on `tier`.
fn printed(design: &str, tier: &str, name: &str) -> (i32, String) {
    let d = tmp(name);
    std::fs::write(d.join("t.vhd"), design).expect("write");
    run(&d, tier)
}

fn all_of(tier: &str) {
    let (code, out) = printed(VALUES, tier, "place_values");
    assert_eq!(code, 0, "{out}");
    assert!(
        out.contains("impure index '0'") && out.contains("done"),
        "{out}"
    );
    let (code, out) = printed(LENGTH, tier, "place_length");
    assert_eq!(code, 1, "{out}");
    assert!(!out.contains("not reached"), "{out}");
    assert!(
        out.contains("E3009]: value length 5 does not match variable S4 length 4"),
        "{out}"
    );
    let (code, out) = printed(ORDER, tier, "place_order");
    assert_eq!(code, 1, "{out}");
    assert!(!out.contains("not reached") && !out.contains("40"), "{out}");
    assert!(
        out.contains("E3009]: index 7 outside of range 0 to 3 for array"),
        "{out}"
    );
}

#[test]
fn reading_in_place_prints_what_it_printed_on_the_interpreter() {
    all_of("interp");
}

#[cfg(feature = "clift")]
#[test]
fn reading_in_place_prints_what_it_printed_in_compiled_code() {
    all_of("clift");
    all_of("clift-auto");
}
