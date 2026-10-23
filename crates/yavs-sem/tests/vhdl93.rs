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

/// An entity with `out` ports `p` (bit), `w` (4 bits) and `r` (a record) and an architecture
/// whose statement part is `body`; `decls` go before `begin`.
fn out_ports(decls: &str, body: &str) -> String {
    format!(
        "entity leaf is generic (g : bit := '0'); port (i : in bit := '0'; o : out bit); end;
architecture a of leaf is begin o <= i; end;
entity leaf2 is port (io : inout bit); end;
architecture a of leaf2 is begin end;
package defs is
  type rec is record f : bit; h : bit; end record;
  function f1 (x : bit) return bit;
  procedure po (signal x : out bit);
  procedure pi (signal x : in bit);
  procedure pv (x : in bit);
end package;
use work.defs.all;
entity e is
  port (p : out bit; w : out bit_vector(3 downto 0); r : out rec);
end;
architecture a of e is
  component cl is port (i : in bit := '0'; o : out bit); end component;
  signal s : bit;
  signal v : bit_vector(1 downto 0);
  {decls}
begin
  {body}
end;"
    )
}

/// Reads of an `out` port in a 1993 file are `E1360` (LRM 1993 4.3.2), each one exactly once;
/// in a 2008 file they are legal (6.5.2).
#[test]
fn reading_an_out_port_is_e1360_in_1993() {
    let reads = [
        ("conc_assign", "", "s <= p;"),
        ("conc_cond", "", "s <= '1' when p = '1' else '0';"),
        (
            "conc_sel",
            "",
            "with p select s <= '1' when '1', '0' when others;",
        ),
        (
            "if",
            "",
            "process begin if p = '1' then null; end if; wait; end process;",
        ),
        (
            "case",
            "",
            "process begin case p is when '1' => null; when others => null; end case; wait; end process;",
        ),
        (
            "wait_until",
            "",
            "process begin wait until p = '1'; end process;",
        ),
        ("wait_on", "", "process begin wait on p; end process;"),
        ("sensitivity", "", "process (p) begin null; end process;"),
        (
            "report",
            "",
            "process begin report bit'image(p); wait; end process;",
        ),
        (
            "assert",
            "",
            "process begin assert p = '1'; wait; end process;",
        ),
        (
            "var_assign",
            "",
            "process variable b : bit; begin b := p; wait; end process;",
        ),
        ("sig_assign", "", "process begin s <= p; wait; end process;"),
        (
            "wait_for",
            "",
            "process begin wait for 1 ns * bit'pos(p); wait; end process;",
        ),
        (
            "for_bound",
            "",
            "process begin for k in 0 to bit'pos(p) loop null; end loop; wait; end process;",
        ),
        (
            "fn_arg",
            "",
            "process variable b : bit; begin b := f1(p); wait; end process;",
        ),
        ("proc_in_arg", "", "process begin pv(p); wait; end process;"),
        (
            "proc_signal_in_arg",
            "",
            "process begin pi(p); wait; end process;",
        ),
        ("conc_proc_in_arg", "", "pi(p);"),
        ("port_map_in", "", "u : entity work.leaf port map (i => p);"),
        (
            "port_map_inout",
            "",
            "u : entity work.leaf2 port map (io => p);",
        ),
        ("component_in", "", "u : cl port map (i => p);"),
        (
            "generic_map",
            "",
            "u : entity work.leaf generic map (g => p) port map (i => s);",
        ),
        ("generate_cond", "", "g : if p = '1' generate end generate;"),
        ("block_guard", "", "b : block (p = '1') begin end block;"),
        (
            "block_port_map",
            "",
            "b : block port (x : in bit); port map (x => p); begin end block;",
        ),
        (
            "init_signal",
            "",
            "b : block signal t : bit := p; begin end block;",
        ),
        (
            "init_constant",
            "",
            "b : block constant t : bit := p; begin end block;",
        ),
        ("qualified", "", "s <= bit'(p);"),
        (
            "conversion",
            "",
            "process variable n : integer; begin n := bit'pos(p); wait; end process;",
        ),
        ("index", "", "s <= w(1);"),
        ("slice", "", "v <= w(1 downto 0);"),
        (
            "index_in_target",
            "",
            "process begin v(bit'pos(p)) <= '1'; wait; end process;",
        ),
        ("field", "", "s <= r.f;"),
        ("attr_event", "", "s <= '1' when p'event else '0';"),
        (
            "attr_last_value",
            "",
            "process variable b : bit; begin b := p'last_value; wait; end process;",
        ),
        (
            "attr_active",
            "",
            "process variable b : boolean; begin b := p'active; wait; end process;",
        ),
        (
            "attr_last_event",
            "",
            "process variable b : time; begin b := p'last_event; wait; end process;",
        ),
        (
            "attr_last_active",
            "",
            "process variable b : time; begin b := p'last_active; wait; end process;",
        ),
        ("attr_stable", "", "s <= '1' when p'stable(1 ns) else '0';"),
        ("attr_quiet", "", "s <= '1' when p'quiet(1 ns) else '0';"),
        ("attr_delayed", "", "s <= p'delayed(1 ns);"),
        (
            "attr_transaction",
            "",
            "process begin wait on p'transaction; end process;",
        ),
    ];
    for (name, decls, body) in reads {
        let src = out_ports(decls, body);
        assert_eq!(std93(&format!("outread_{name}"), &src), ["E1360"], "{name}");
        let ok = codes(
            LangRev::Vhdl2008,
            LangRev::Vhdl2008,
            &format!("outread08_{name}"),
            &src,
        );
        assert!(ok.is_empty(), "{name} is legal in 2008: {ok:?}");
    }
}

/// Writes, the actuals of `out` formals and the attributes that do not read the value are not
/// reads of an `out` port; a read through an alias is not tracked (L138).
#[test]
fn writing_an_out_port_is_not_a_read_in_1993() {
    let writes = [
        ("target", "p <= '1';"),
        ("proc_target", "process begin p <= '1'; wait; end process;"),
        ("index_target", "w(1) <= '1';"),
        ("slice_target", "w(1 downto 0) <= \"11\";"),
        ("aggregate_target", "(p, s) <= bit_vector'(\"11\");"),
        ("field_target", "r.f <= '1';"),
        ("out_actual", "process begin po(p); wait; end process;"),
        ("conc_out_actual", "po(p);"),
        ("port_map_out", "u : entity work.leaf port map (o => p);"),
        ("component_out", "u : cl port map (o => p);"),
        (
            "range",
            "process begin for k in w'range loop null; end loop; wait; end process;",
        ),
        (
            "length",
            "process variable n : integer; begin n := w'length + w'left + w'high; wait; end process;",
        ),
        (
            "subtype_range",
            "process variable b : bit_vector(w'range); begin wait; end process;",
        ),
        (
            "driving",
            "process variable b : boolean; begin b := p'driving; wait; end process;",
        ),
        (
            "driving_value",
            "process variable b : bit; begin b := p'driving_value; wait; end process;",
        ),
        (
            "simple_name",
            "process variable n : integer; begin n := p'path_name'length; wait; end process;",
        ),
        ("alias", "b : block alias q : bit is p; begin end block;"),
    ];
    for (name, body) in writes {
        let src = out_ports("", body);
        let c = std93(&format!("outwrite_{name}"), &src);
        assert!(c.is_empty(), "{name}: {c:?}");
    }
}

/// An `out` parameter of a procedure is read in the same way (LRM 1993 4.3.2).
#[test]
fn reading_an_out_parameter_is_e1360_in_1993() {
    let body = |stmt: &str| {
        format!(
            "entity e is end;
architecture a of e is
  procedure p (x : out integer; signal y : out bit) is
    variable n : integer;
    variable b : bit;
  begin
    {stmt}
  end;
begin end;"
        )
    };
    for (name, stmt) in [
        ("variable", "x := 1; n := x;"),
        ("signal", "y <= '1'; b := y;"),
        ("signal_event", "y <= '1'; assert y'event;"),
    ] {
        let src = body(stmt);
        assert_eq!(
            std93(&format!("outparam_{name}"), &src),
            ["E1360"],
            "{name}"
        );
        assert!(
            codes(
                LangRev::Vhdl2008,
                LangRev::Vhdl2008,
                &format!("outparam08_{name}"),
                &src
            )
            .is_empty(),
            "{name}"
        );
    }
    for (name, stmt) in [
        ("write_only", "x := 1; y <= '1';"),
        ("passed_on", "x := 1; y <= '1'; q(x, y);"),
    ] {
        let src = body(stmt).replace(
            "  procedure p (",
            "  procedure q (a : out integer; signal c : out bit) is begin a := 1; c <= '1'; end;
  procedure p (",
        );
        let c = std93(&format!("outparam_ok_{name}"), &src);
        assert!(c.is_empty(), "{name}: {c:?}");
    }
}

/// The revision of the file with the read decides, not that of the entity.
#[test]
fn the_reading_file_decides_whether_an_out_port_may_be_read() {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("outread_files");
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let ent = dir.join("ent.vhd");
    std::fs::write(&ent, "entity e is port (p : out bit); end;").expect("write");
    let arch = dir.join("arch.vhd");
    std::fs::write(
        &arch,
        "architecture a of e is signal s : bit; begin s <= p; end;",
    )
    .expect("write");
    for (rev, want) in [(LangRev::Vhdl2008, 0), (LangRev::Vhdl1993, 1)] {
        let mut db = Db::new(Options::new(LangRev::Vhdl2008, default_stdlib()));
        let _ = db.analyze_file("work", &ent, LangRev::Vhdl1993);
        let _ = db.analyze_file("work", &arch, rev);
        assert_eq!(db.diagnostics().len(), want, "{rev:?}");
    }
}

/// The actual of an `out` formal of an overloaded procedure is no read, whichever overload wins.
#[test]
fn an_out_parameter_as_the_actual_of_an_overloaded_out_formal_is_no_read_in_1993() {
    let src = "entity e is end;
architecture a of e is
  procedure p (x : out integer; y : out bit; z : out bit_vector(1 downto 0)) is
    variable l : line;
  begin
    read(l, x);
    read(l, y);
    read(l, z);
  end;
begin end;";
    let src = format!("use std.textio.all;\n{src}");
    let c = std93("out_overloaded", &src);
    assert!(c.is_empty(), "{c:?}");
}
