//! The VHDL-1993 mode (#629, `docs/design/vhdl-93.md`): what a 1993 file means in a 1993 design,
//! in a 2008 design, and what 2008 makes of the same text.

use std::path::PathBuf;

use yavs_sem::{Db, LangRev, Options, default_stdlib};

/// Analyze `src` as one file of standard `file_rev` in a design of revision `design`;
/// the codes of its diagnostics (parse errors included).
fn codes(design: LangRev, file_rev: LangRev, test: &str, src: &str) -> Vec<String> {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(test);
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let mut db = Db::new(Options::new(design, default_stdlib()));
    let path = dir.join("f.vhd");
    std::fs::write(&path, src).expect("write source");
    let _ = db.analyze_file("work", &path, file_rev);
    db.diagnostics()
        .iter()
        .map(|d| d.code.to_string())
        .collect()
}

fn std93(test: &str, src: &str) -> Vec<String> {
    codes(LangRev::Vhdl1993, LangRev::Vhdl1993, test, src)
}

/// STD_LOGIC_VECTOR is a type of its own: overloads on both vector types are not homographs.
const OVERLOADS: &str = "library ieee; use ieee.std_logic_1164.all;
package ovl is
  procedure hr (v : std_logic_vector);
  procedure hr (v : std_ulogic_vector);
end package;";

#[test]
fn slv_and_sulv_overloads_are_not_homographs_in_1993() {
    assert!(std93("ovl93", OVERLOADS).is_empty());
    let c = codes(LangRev::Vhdl2008, LangRev::Vhdl2008, "ovl08", OVERLOADS);
    assert!(!c.is_empty(), "homographs in 2008: {c:?}");
}

#[test]
fn slv_converts_to_sulv_only_by_conversion_in_1993() {
    let src = "library ieee; use ieee.std_logic_1164.all;
entity e is end;
architecture a of e is
  signal s : std_logic_vector(3 downto 0);
  signal u : std_ulogic_vector(3 downto 0);
begin
  u <= std_ulogic_vector(s);
end;";
    assert!(std93("conv93", src).is_empty());
    let direct = src.replace("std_ulogic_vector(s)", "s");
    assert!(
        !std93("direct93", &direct).is_empty(),
        "no implicit subtype in 1993"
    );
    assert!(codes(LangRev::Vhdl2008, LangRev::Vhdl2008, "direct08", &direct).is_empty());
}

#[test]
fn predefined_2008_names_are_free_in_1993() {
    let src = "package p is
  type integer_vector is array (natural range <>) of integer;
  function to_string (x : integer) return string;
  function maximum (a, b : integer) return integer;
  constant default : integer := 1;
end package;
use work.p.all;
entity e is end;
architecture a of e is
  signal context, force, release, protected, property : integer;
  signal v : integer_vector(1 to 2);
begin
  process begin
    report to_string(maximum(1, 2));
    wait;
  end process;
end;";
    assert!(std93("names93", src).is_empty());
    assert!(!codes(LangRev::Vhdl2008, LangRev::Vhdl2008, "names08", src).is_empty());
}

#[test]
fn shared_variable_of_any_type_in_1993() {
    let src = "entity e is end;
architecture a of e is
  shared variable v : integer;
begin end;";
    assert!(std93("shared93", src).is_empty());
    assert_eq!(
        codes(LangRev::Vhdl2008, LangRev::Vhdl2008, "shared08", src),
        ["E1904"]
    );
}

#[test]
fn rules_2008_added_are_errors_in_1993() {
    for (name, what, src) in [
        (
            "all",
            "process (all)",
            "entity e is end; architecture a of e is signal s: bit; begin process (all) begin null; end process; end;",
        ),
        (
            "cond",
            "conditional assignment in a process",
            "entity e is end; architecture a of e is signal s: bit; begin process begin s <= '1' when s = '0' else '0'; wait; end process; end;",
        ),
        (
            "sel",
            "selected assignment in a process",
            "entity e is end; architecture a of e is signal s: bit; begin process begin with s select s <= '1' when '0', '0' when others; wait; end process; end;",
        ),
        (
            "elsif",
            "elsif generate",
            "entity e is end; architecture a of e is begin g: if true generate begin elsif false generate begin end generate; end;",
        ),
        (
            "pkggen",
            "package generics",
            "package p is generic (n : integer := 1); end package;",
        ),
        (
            "matching",
            "?= operator",
            "entity e is end; architecture a of e is constant b : bit := bit'('1') ?= bit'('1'); begin end;",
        ),
    ] {
        let c = std93(&format!("gate_{name}"), src);
        assert!(!c.is_empty(), "{what} must be rejected in 1993");
        let ok = codes(
            LangRev::Vhdl2008,
            LangRev::Vhdl2008,
            &format!("gate08_{name}"),
            src,
        );
        assert!(
            ok.iter().all(|c| !c.starts_with('E')),
            "{what} is legal in 2008: {ok:?}"
        );
    }
}

#[test]
fn a_1993_file_in_a_2008_design_does_not_see_2008_names() {
    let src = "entity e is end;
architecture a of e is begin
  process begin report to_string(1); wait; end process;
end;";
    assert_eq!(
        codes(LangRev::Vhdl2008, LangRev::Vhdl1993, "mix_to_string", src),
        ["E1007"]
    );
    assert!(
        codes(
            LangRev::Vhdl2008,
            LangRev::Vhdl2008,
            "mix_to_string_ok",
            src
        )
        .is_empty()
    );
}

#[test]
fn a_1993_file_naming_a_2008_package_is_e1007() {
    let src = "library ieee; use ieee.numeric_std_unsigned.all;
entity e is end; architecture a of e is begin end;";
    assert_eq!(
        codes(LangRev::Vhdl2008, LangRev::Vhdl1993, "mix_unit", src),
        ["E1007"]
    );
}
