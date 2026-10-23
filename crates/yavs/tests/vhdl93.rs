//! `--std=93` end to end (#629): analysis records the standard per file, `run` picks the design
//! revision from the closure, and a design of 1993 files runs against the 1993 libraries.

mod common;
use common::tempdir as tmp;

use std::path::Path;
use std::process::Command;

fn yavs(dir: &Path, args: &[&str]) -> (i32, String, String) {
    let p = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .current_dir(dir)
        .args(args)
        .output()
        .expect("run yavs");
    (
        p.status.code().expect("exit status"),
        String::from_utf8_lossy(&p.stdout).into_owned(),
        String::from_utf8_lossy(&p.stderr).into_owned(),
    )
}

/// Overloads on `std_logic_vector` and `std_ulogic_vector`, a shared variable of a plain
/// type, and a name 2008 reserves: all of it is only legal in 1993.
const TOP93: &str = "
library ieee; use ieee.std_logic_1164.all;
package pk is
  function img (v : std_logic_vector) return string;
  function img (v : std_ulogic_vector) return string;
end;
package body pk is
  function img (v : std_logic_vector) return string is begin return \"slv\"; end;
  function img (v : std_ulogic_vector) return string is begin return \"sulv\"; end;
end;
library ieee; use ieee.std_logic_1164.all; use work.pk.all;
entity top93 is end;
architecture a of top93 is
  signal s : std_logic_vector(1 downto 0) := \"10\";
  signal u : std_ulogic_vector(1 downto 0) := \"01\";
  shared variable count : integer := 0;
  constant default : integer := 4;
begin
  process begin
    count := count + default;
    report img(s) & \" \" & img(u) & \" \" & integer'image(count);
    wait;
  end process;
end;
";

#[test]
fn std93_design_runs_with_the_1993_libraries() {
    let dir = tmp("std93_run");
    std::fs::write(dir.join("top93.vhd"), TOP93).expect("write");
    let work = format!("work:{}", dir.join("lib").display());
    let (c, _, e) = yavs(&dir, &["analyze", "--std=93", "--work", &work, "top93.vhd"]);
    assert_eq!(c, 0, "{e}");
    let manifest = std::fs::read_to_string(dir.join("lib/yavs-lib.toml")).expect("manifest");
    assert!(manifest.contains("std = \"1993\""), "{manifest}");
    // No --std: the revision comes from the manifest.
    let (c, out, e) = yavs(&dir, &["run", "--work", &work, "top93"]);
    assert_eq!(c, 0, "{e}");
    assert!(out.contains("slv sulv 4"), "{out}");
    // The same text is not a 2008 design.
    let (c, _, e) = yavs(&dir, &["run", "--std=2008", "--work", &work, "top93"]);
    assert_ne!(c, 0, "{e}");
}

#[test]
fn std93_is_a_2008_error_and_87_is_unknown() {
    let dir = tmp("std93_2008");
    std::fs::write(dir.join("top93.vhd"), TOP93).expect("write");
    let work = format!("work:{}", dir.join("lib").display());
    let (c, _, e) = yavs(
        &dir,
        &["analyze", "--std=2008", "--work", &work, "top93.vhd"],
    );
    assert_eq!(c, 1, "{e}");
    let (c, _, e) = yavs(&dir, &["analyze", "--std=87", "--work", &work, "top93.vhd"]);
    assert_eq!(c, 2, "{e}");
}

#[test]
fn a_93_file_with_a_2008_construct_is_e0011() {
    let dir = tmp("std93_e0011");
    std::fs::write(
        dir.join("t.vhd"),
        "entity t is end; architecture a of t is signal s : bit; begin process (all) begin null; end process; end;",
    )
    .expect("write");
    let work = format!("work:{}", dir.join("lib").display());
    let (c, _, e) = yavs(&dir, &["analyze", "--std=93", "--work", &work, "t.vhd"]);
    assert_eq!(c, 1, "{e}");
    assert!(
        e.contains("E0011") && e.contains("requires VHDL-2008"),
        "{e}"
    );
}

#[test]
fn a_93_file_in_a_2008_design_is_checked_as_93() {
    let dir = tmp("std93_mix");
    let lib = "entity leaf is end; architecture a of leaf is begin \
         process begin report to_string(1); wait; end process; end;";
    std::fs::write(dir.join("leaf.vhd"), lib).expect("write");
    std::fs::write(
        dir.join("top.vhd"),
        "entity top is end; architecture a of top is begin l : entity work.leaf; end;",
    )
    .expect("write");
    let work = format!("work:{}", dir.join("lib").display());
    let (c, _, e) = yavs(&dir, &["analyze", "--std=93", "--work", &work, "leaf.vhd"]);
    assert_eq!(c, 1, "to_string does not exist in 1993: {e}");
    let (c, _, e) = yavs(
        &dir,
        &["analyze", "--std=2008", "--work", &work, "leaf.vhd"],
    );
    assert_eq!(c, 0, "{e}");
    let (c, _, e) = yavs(&dir, &["analyze", "--std=93", "--work", &work, "top.vhd"]);
    assert_eq!(c, 0, "{e}");
    // The newest standard of the closure (2008 for leaf) decides the design revision.
    let (c, out, e) = yavs(&dir, &["run", "--work", &work, "top"]);
    assert_eq!(c, 0, "{e}");
    assert!(out.contains('1'), "{out}");
}
