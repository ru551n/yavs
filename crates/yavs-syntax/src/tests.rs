//! Parser tests, one group per grammar area.

use yavs_diag::{FileId, Severity};

use crate::ast::*;
use crate::{Encoding, Options, Std, load_source, parse_file, parse_str};

fn parse_std(src: &str, std: Std) -> crate::Parsed {
    parse_str(
        FileId(0),
        src,
        &Options {
            std,
            defines: Vec::new(),
            psl: true,
        },
    )
}

/// Parse and require no diagnostics at all.
fn ok_std(src: &str, std: Std) -> DesignFile {
    let p = parse_std(src, std);
    assert!(p.diags.is_empty(), "unexpected diagnostics: {:#?}", p.diags);
    p.design.expect("design")
}

fn ok(src: &str) -> DesignFile {
    ok_std(src, Std::Vhdl2008)
}

fn ok19(src: &str) -> DesignFile {
    ok_std(src, Std::Vhdl2019)
}

/// Error messages, requiring at least one error and no tree.
fn errors_std(src: &str, std: Std) -> Vec<String> {
    let p = parse_std(src, std);
    assert!(p.design.is_none(), "expected errors");
    p.diags
        .iter()
        .filter(|d| d.severity == Severity::Error)
        .map(|d| d.message.clone())
        .collect()
}

fn errors(src: &str) -> Vec<String> {
    errors_std(src, Std::Vhdl2008)
}

/// Wrap declarations in a package.
fn pkg(decls: &str) -> String {
    format!("package p is\n{decls}\nend package;")
}

/// Wrap sequential statements in a process.
fn seq(stmts: &str) -> String {
    format!("architecture a of e is begin process begin\n{stmts}\nend process; end;")
}

/// Wrap concurrent statements in an architecture.
fn conc(stmts: &str) -> String {
    format!("architecture a of e is begin\n{stmts}\nend;")
}

fn pkg_decls(d: &DesignFile) -> &[Decl] {
    match &d.units[0].unit {
        LibraryUnit::Package(p) => &p.decls,
        u => panic!("not a package: {u:?}"),
    }
}

fn arch_stmts(d: &DesignFile) -> &[ConcStmt] {
    match &d.units[0].unit {
        LibraryUnit::Architecture(a) => &a.stmts,
        u => panic!("not an architecture: {u:?}"),
    }
}

fn process_stmts(d: &DesignFile) -> &[SeqStmt] {
    match &arch_stmts(d)[0].kind {
        ConcStmtKind::Process(p) => &p.stmts,
        k => panic!("not a process: {k:?}"),
    }
}

/// The default value of the first object declaration in a package.
fn first_default(src: &str, std: Std) -> Expr {
    let d = ok_std(&pkg(src), std);
    let mut units = d.units;
    let LibraryUnit::Package(p) = units.remove(0).unit else {
        panic!()
    };
    match p.decls.into_iter().next() {
        Some(Decl::Object(o)) => o.default.expect("default"),
        d => panic!("not an object: {d:?}"),
    }
}

// Design units and context clauses.

#[test]
fn design_units() {
    let d = ok("library ieee, work; use ieee.std_logic_1164.all; context work.ctx;
        entity e is generic (n : natural := 4); port (a : in bit; b : out bit); begin assert true; end entity e;
        architecture rtl of e is begin end architecture rtl;
        package p is end package p;
        package body p is end package body p;
        package q is new work.gp generic map (n => 2);
        configuration c of e is for rtl end for; end configuration c;
        context ctx is library ieee; use ieee.numeric_std.all; end context ctx;");
    assert_eq!(d.units.len(), 7);
    assert_eq!(d.units[0].context.len(), 3);
    let LibraryUnit::Entity(e) = &d.units[0].unit else {
        panic!()
    };
    assert_eq!(e.generics.as_ref().map(Vec::len), Some(1));
    assert_eq!(e.ports.as_ref().map(Vec::len), Some(2));
    assert_eq!(e.stmts.len(), 1);
    assert!(matches!(d.units[4].unit, LibraryUnit::PackageInstance(_)));
    assert!(matches!(d.units[6].unit, LibraryUnit::Context(_)));
}

#[test]
fn identifiers_keep_spelling() {
    let d = ok("entity MyEnt is end; entity \\MyEnt\\ is end;");
    let LibraryUnit::Entity(a) = &d.units[0].unit else {
        panic!()
    };
    let LibraryUnit::Entity(b) = &d.units[1].unit else {
        panic!()
    };
    assert_eq!(a.name.text.as_str(), "MyEnt");
    assert_eq!(a.name.key.as_str(), "myent");
    assert_eq!(b.name.text.as_str(), "\\MyEnt\\");
    assert_eq!(b.name.key.as_str(), "\\MyEnt\\");
    // End labels compare by key.
    ok("entity Foo is end entity FOO;");
}

#[test]
fn configuration() {
    let d = ok("configuration c of e is
          use work.all;
          for rtl
            for u1, u2 : comp use entity work.ent(arch) generic map (g => 1) port map (a => b); end for;
            for all : comp2 use configuration work.cfg; end for;
            for others : comp3 use open; end for;
            for gen(1)
              for inner : c4 end for;
            end for;
          end for;
        end;");
    let LibraryUnit::Configuration(c) = &d.units[0].unit else {
        panic!()
    };
    let block = c.block.as_ref().expect("block");
    assert_eq!(block.items.len(), 4);
    assert!(matches!(&block.items[3], ConfigItem::Block(_)));
    ok("architecture a of e is for u1 : comp use entity work.x; begin end;");
}

#[test]
fn architecture_of_selected_entity_name() {
    let d = ok("architecture a of work.e is begin end;");
    let LibraryUnit::Architecture(a) = &d.units[0].unit else {
        panic!()
    };
    assert_eq!(
        a.entity_lib.map(|l| l.text.to_string()).as_deref(),
        Some("work")
    );
    assert_eq!(a.entity.text.as_str(), "e");
}

// Declarations.

#[test]
fn types() {
    let d = ok(&pkg("type state is (idle, 'x', run);
        type int is range 0 to 255;
        type flt is range -1.0 to 1.0;
        type dist is range 0 to 1e9 units nm; um = 1000 nm; mm = 1000 um; end units dist;
        type arr is array (natural range <>, integer range <>) of bit;
        type carr is array (0 to 7) of bit_vector(3 downto 0);
        type rec is record a, b : integer; c : bit; end record rec;
        type ptr is access rec;
        type ft is file of integer;
        type inc;
        type prot is protected procedure inc; end protected prot;
        type prot is protected body variable n : integer := 0; procedure inc is begin n := n + 1; end; end protected body;"));
    let decls = pkg_decls(&d);
    assert_eq!(decls.len(), 12);
    let Decl::Type(t) = &decls[3] else { panic!() };
    let Some(TypeDef::Physical(phys)) = &t.def else {
        panic!()
    };
    assert_eq!(phys.secondary.len(), 2);
    let Decl::Type(t) = &decls[4] else { panic!() };
    assert!(
        matches!(&t.def, Some(TypeDef::Array { indexes: ArrayIndexes::Unconstrained(v), .. }) if v.len() == 2)
    );
    let Decl::Type(t) = &decls[9] else { panic!() };
    assert!(t.def.is_none());
}

#[test]
fn subtype_indications() {
    let d = ok(&pkg("subtype a is std_logic_vector(7 downto 0);
        subtype b is resolved std_ulogic;
        subtype c is (resolved) std_ulogic_vector;
        subtype d is integer range 0 to 7;
        subtype e is rec_t(f1(3 downto 0), f2(open));
        subtype f is arr_t(open)(7 downto 0);
        subtype g is arr_t(idx_t, x'range);
        subtype h is t'subtype;
        subtype i is (elem resolved, other f) rec_t;
        subtype j is arr_t(g(g'low)(g(g'low)'high).f'range);
        subtype k is rec_t(f1(open)(3 downto 0), f2(1 to 2));"));
    let decls = pkg_decls(&d);
    let st = |i: usize| match &decls[i] {
        Decl::Subtype(s) => &s.subtype,
        d => panic!("{d:?}"),
    };
    assert!(matches!(st(0).constraint, Some(Constraint::Array { .. })));
    assert!(matches!(st(1).resolution, Some(Resolution::Function(_))));
    assert!(matches!(st(2).resolution, Some(Resolution::Element(_))));
    assert!(matches!(st(3).constraint, Some(Constraint::Range(_))));
    assert!(matches!(&st(4).constraint, Some(Constraint::Record(v, _)) if v.len() == 2));
    assert!(matches!(
        &st(5).constraint,
        Some(Constraint::Array {
            indexes: None,
            element: Some(_),
            ..
        })
    ));
    let Some(Constraint::Array {
        indexes: Some(ix), ..
    }) = &st(6).constraint
    else {
        panic!()
    };
    assert!(
        ix.iter()
            .all(|r| matches!(r, DiscreteRange::Range(Range::Name(_))))
    );
    assert!(matches!(st(7).type_mark.kind, NameKind::Attribute { .. }));
    assert!(matches!(&st(8).resolution, Some(Resolution::Record(v)) if v.len() == 2));
    assert!(matches!(
        &st(9).constraint,
        Some(Constraint::Array { indexes: Some(ix), .. })
            if matches!(ix[..], [DiscreteRange::Range(Range::Name(_))])
    ));
    assert!(matches!(&st(10).constraint, Some(Constraint::Record(v, _)) if v.len() == 2));
}

/// `arr(0)'subtype` is the subtype of an indexed object name: the parentheses
/// belong to the name, not to an index constraint (#616).
#[test]
fn subtype_of_an_indexed_name() {
    let d = ok(&pkg("subtype s is arr(0)'subtype;
        subtype t is arr(0 to 1);"));
    let decls = pkg_decls(&d);
    let st = |i: usize| match &decls[i] {
        Decl::Subtype(s) => &s.subtype,
        d => panic!("{d:?}"),
    };
    let NameKind::Attribute { prefix, attr } = &st(0).type_mark.kind else {
        panic!("{:?}", st(0).type_mark.kind)
    };
    assert_eq!(attr.key.as_str(), "subtype");
    assert!(matches!(prefix.kind, NameKind::Apply { .. }));
    assert!(st(0).constraint.is_none());
    assert!(matches!(st(1).constraint, Some(Constraint::Array { .. })));
}

#[test]
fn objects_and_aliases() {
    let d = ok(&pkg(
        "constant c : integer := 1;
        constant d : integer;
        signal s, t : bit register := '0';
        shared variable v : prot;
        file f : text open read_mode is \"in.txt\";
        alias a : bit_vector(0 to 3) is s(4 to 7);
        alias plus is \"+\" [integer, integer return integer];
        alias 'x' is pkg.'x';
        attribute keep : boolean;
        attribute keep of s, t : signal is true;
        attribute foo of \"+\" [integer return integer] : function is 1;
        attribute bar of others : label is 2;
        group pair is (signal, signal <>);
        group g1 : pair (s, t);
        disconnect s : bit after 1 ns;
        component c is generic (n : natural); port (a : in bit); end component c;",
    ));
    let decls = pkg_decls(&d);
    assert_eq!(decls.len(), 16);
    let Decl::Alias(a) = &decls[6] else { panic!() };
    assert!(matches!(a.name.kind, NameKind::Signature { .. }));
    let Decl::Object(o) = &decls[2] else { panic!() };
    assert_eq!(o.signal_kind, Some(SignalKind::Register));
    assert_eq!(o.names.len(), 2);
}

#[test]
fn subprograms() {
    let d = ok(&pkg("function f(a : integer; constant b : in bit := '0') return integer;
        pure function g return bit;
        impure function h parameter (x : integer) return boolean;
        procedure p(signal s : out bit; variable v : inout integer; file f : text);
        function \"+\"(a, b : t) return t;
        function gf generic (type t; function eq(a, b : t) return boolean is <>) parameter (x : t) return t;
        procedure gp is new work.base generic map (n => 1);
        function fi is new gf generic map (t => integer);
        function body_f(x : integer) return integer is variable y : integer; begin return x + y; end function body_f;
        procedure body_p is begin null; end procedure;
        function \"and\"(a, b : t) return t is begin return a; end \"and\";"));
    let decls = pkg_decls(&d);
    assert!(matches!(&decls[0], Decl::Subprogram(s) if s.params.as_ref().map(Vec::len) == Some(2)));
    let Decl::Subprogram(gf) = &decls[5] else {
        panic!()
    };
    assert_eq!(gf.generics.as_ref().map(Vec::len), Some(2));
    assert!(matches!(&decls[6], Decl::SubprogramInstance(_)));
    assert!(matches!(&decls[7], Decl::SubprogramInstance(_)));
    assert!(matches!(&decls[8], Decl::SubprogramBody(b) if b.stmts.len() == 1));
}

#[test]
fn interfaces() {
    let d = ok("entity e is
        generic (type t; constant n : natural := 2; function f return t; procedure p is <>;
                 package gp is new work.gpkg generic map (<>); package gq is new work.gpkg generic map (default);
                 package gr is new work.gpkg generic map (n => 1));
        port (a, b : in bit := '0'; c : out t; d : inout bit_vector; e : buffer bit; f : linkage bit; g : bit bus);
        end;");
    let LibraryUnit::Entity(e) = &d.units[0].unit else {
        panic!()
    };
    let gens = e.generics.as_ref().expect("generics");
    assert!(matches!(gens[0], InterfaceDecl::Type(_)));
    assert!(
        matches!(&gens[3], InterfaceDecl::Subprogram(s) if matches!(s.default, Some(SubprogramDefault::Box)))
    );
    assert!(
        matches!(&gens[4], InterfaceDecl::Package(p) if matches!(p.generic_map, InterfacePackageMap::Box))
    );
    assert!(
        matches!(&gens[5], InterfaceDecl::Package(p) if matches!(p.generic_map, InterfacePackageMap::Default))
    );
    let ports = e.ports.as_ref().expect("ports");
    assert!(
        matches!(&ports[0], InterfaceDecl::Object(o) if o.names.len() == 2 && matches!(o.mode, Some(Mode::In)))
    );
    assert!(matches!(&ports[5], InterfaceDecl::Object(o) if o.bus && o.mode.is_none()));
}

// Expressions and names.

fn expr(src: &str) -> Expr {
    first_default(&format!("constant c : t := {src};"), Std::Vhdl2008)
}

/// A compact rendering of an expression tree, for precedence checks.
fn show(e: &Expr) -> String {
    match &e.kind {
        ExprKind::Binary { op, left, right } => format!("({} {op:?} {})", show(left), show(right)),
        ExprKind::Unary { op, operand } => format!("({op:?} {})", show(operand)),
        ExprKind::Name(n) => show_name(n),
        ExprKind::Literal(l) => match &l.kind {
            LiteralKind::Abstract(s) | LiteralKind::BitString(s) => s.to_string(),
            LiteralKind::String(s) => format!("{s:?}"),
            LiteralKind::Char(c) => format!("{c:?}"),
            LiteralKind::Physical { value, unit } => format!("{value} {}", unit.text),
            LiteralKind::Null => "null".into(),
        },
        ExprKind::Paren(e) => format!("[{}]", show(e)),
        ExprKind::Aggregate(v) => format!("agg{}", v.len()),
        ExprKind::Qualified { type_mark, operand } => {
            format!("{}'{}", show_name(type_mark), show(operand))
        }
        ExprKind::New(_) => "new".into(),
        ExprKind::Conditional(v) => format!("cond{}", v.len()),
        ExprKind::Unaffected => "unaffected".into(),
        ExprKind::Error => "error".into(),
    }
}

fn show_name(n: &Name) -> String {
    match &n.kind {
        NameKind::Simple(i) => i.text.to_string(),
        NameKind::Operator(s) => format!("{s:?}"),
        NameKind::Character(c) => format!("{c:?}"),
        NameKind::Selected { prefix, suffix } => {
            let s = match suffix {
                Suffix::Ident(i) => i.text.to_string(),
                Suffix::Char(c) => format!("{:?}", c.value),
                Suffix::Operator(s, _) => format!("{s:?}"),
                Suffix::All(_) => "all".into(),
            };
            format!("{}.{s}", show_name(prefix))
        }
        NameKind::Apply { prefix, args } => format!("{}({})", show_name(prefix), args.len()),
        NameKind::GenericMap { prefix, .. } => format!("{} generic map", show_name(prefix)),
        NameKind::Attribute { prefix, attr } => format!("{}'{}", show_name(prefix), attr.text),
        NameKind::Signature { prefix, .. } => format!("{}[sig]", show_name(prefix)),
        NameKind::External(_) => "<<external>>".into(),
    }
}

#[test]
fn precedence() {
    assert_eq!(show(&expr("a + b * c ** 2")), "(a Add (b Mul (c Pow 2)))");
    assert_eq!(show(&expr("-a * b + c")), "((Minus (a Mul b)) Add c)");
    assert_eq!(show(&expr("a and b and c")), "((a And b) And c)");
    assert_eq!(show(&expr("a = b or not c")), "((a Eq b) Or (Not c))");
    assert_eq!(show(&expr("a sll 2 & b")), "(a Sll (2 Concat b))");
    assert_eq!(show(&expr("abs a mod 3")), "((Abs a) Mod 3)");
    assert_eq!(show(&expr("a ?= b")), "(a MatchEq b)");
    assert_eq!(show(&expr("?? s")), "(Condition s)");
    assert_eq!(show(&expr("and v")), "(And v)");
    assert_eq!(show(&expr("(a + b) * c")), "([(a Add b)] Mul c)");
}

#[test]
fn mixed_logical_operators_need_parens() {
    let e = errors(&pkg("constant c : boolean := a and b or c;"));
    assert_eq!(e, ["mixed logical operators need parentheses"]);
    let e = errors(&pkg("constant c : boolean := a nand b nand c;"));
    assert_eq!(e, ["mixed logical operators need parentheses"]);
}

#[test]
fn names_are_ambiguous() {
    // Call, index, slice and conversion are all `Apply`.
    assert_eq!(show(&expr("f(x)")), "f(1)");
    assert_eq!(show(&expr("a(1 to 3)")), "a(1)");
    assert_eq!(show(&expr("integer(r)")), "integer(1)");
    assert_eq!(show(&expr("f(a => 1, b => open)")), "f(2)");
    assert_eq!(
        show(&expr("ieee.numeric_std.\"+\"(a, b)")),
        "ieee.numeric_std.\"+\"(2)"
    );
    assert_eq!(show(&expr("x'length(1)")), "x'length(1)");
    assert_eq!(show(&expr("x'range")), "x'range");
    assert_eq!(show(&expr("rec.field(2).sub")), "rec.field(1).sub");
    assert_eq!(
        show(&expr("f [integer return bit]'path_name")),
        "f[sig]'path_name"
    );
    assert_eq!(show(&expr("'1' [return bit]'image")), "'1'[sig]'image");
    assert_eq!(show(&expr("<< signal .tb.dut.s : bit >>")), "<<external>>");
    assert_eq!(
        show(&expr("<< constant ^.^.g(2).c : integer >>")),
        "<<external>>"
    );
    assert_eq!(
        show(&expr("<< variable @lib.pkg.v : integer >>")),
        "<<external>>"
    );
    let e = expr("a(t range 0 to 3)");
    let ExprKind::Name(Name {
        kind: NameKind::Apply { args, .. },
        ..
    }) = e.kind
    else {
        panic!()
    };
    assert!(matches!(args[0].actual, Actual::Subtype(_)));
}

#[test]
fn literals() {
    assert_eq!(show(&expr("16#FF#")), "16#FF#");
    assert_eq!(show(&expr("1.5e-3")), "1.5e-3");
    assert_eq!(show(&expr("10 ns")), "10 ns");
    assert_eq!(show(&expr("12X\"0F\"")), "12X\"0F\"");
    assert_eq!(show(&expr("\"say \"\"hi\"\"\"")), "\"say \\\"hi\\\"\"");
    assert_eq!(show(&expr("'a'")), "'a'");
    assert_eq!(show(&expr("null")), "null");
}

#[test]
fn missing_space_after_number_warns() {
    let p = parse_std(&pkg("constant t : time := 10ns;"), Std::Vhdl2008);
    assert!(p.design.is_some());
    assert_eq!(p.diags.len(), 1);
    assert_eq!(p.diags[0].severity, Severity::Warning);
}

#[test]
fn aggregates_and_qualified() {
    assert_eq!(show(&expr("(others => '0')")), "agg1");
    assert_eq!(show(&expr("(1, 2, 3)")), "agg3");
    assert_eq!(
        show(&expr(
            "(a | b => 1, 2 to 5 => 0, natural range 6 to 7 => 1)"
        )),
        "agg3"
    );
    assert_eq!(show(&expr("t'(others => 0)")), "t'agg1");
    assert_eq!(show(&expr("t'(x)")), "t'[x]");
    assert_eq!(show(&expr("new t'(1, 2)")), "new");
    assert_eq!(show(&expr("new string(1 to 3)")), "new");
}

// Sequential statements.

#[test]
fn sequential_statements() {
    let d = ok(&seq("wait on a, b until c = '1' for 10 ns;
        wait;
        assert x report \"msg\" severity error;
        report \"hi\";
        lbl: if a then null; elsif b then x := 1; else y <= 2; end if lbl;
        case s is when 1 | 2 => null; when 3 to 5 => null; when others => null; end case;
        case? v is when \"1-\" => null; when others => null; end case?;
        l1: for i in 0 to 7 loop next when i = 2; exit l1; end loop l1;
        while c loop end loop;
        loop exit; end loop;
        p(1, 2);
        p;
        (a, b) := pair;
        s <= transport a after 1 ns, b after 2 ns;
        s <= reject 1 ns inertial a;
        s <= a when c else b when d else e;
        s <= force in '1';
        s <= release out;
        v := a when c else b;
        with sel select s <= a when '0', b when others;
        with sel select? v := 1 when \"1-\", 0 when others;
        return;"));
    let stmts = process_stmts(&d);
    assert_eq!(stmts.len(), 22);
    assert!(
        matches!(&stmts[4].kind, SeqStmtKind::If { branches, else_: Some(_), .. } if branches.len() == 2)
    );
    assert_eq!(stmts[4].label.map(|l| l.text.as_str()), Some("lbl"));
    assert!(matches!(
        &stmts[6].kind,
        SeqStmtKind::Case { matching: true, .. }
    ));
    assert!(matches!(
        &stmts[12].kind,
        SeqStmtKind::VariableAssign {
            target: Expr {
                kind: ExprKind::Aggregate(_),
                ..
            },
            ..
        }
    ));
    assert!(
        matches!(&stmts[15].kind, SeqStmtKind::SignalAssign(SignalAssign { kind: SignalAssignKind::Waveform { rhs: Rhs::Conditional(v), .. }, .. }) if v.len() == 3)
    );
    assert!(matches!(
        &stmts[16].kind,
        SeqStmtKind::SignalAssign(SignalAssign {
            kind: SignalAssignKind::Force {
                mode: Some(ForceMode::In),
                ..
            },
            ..
        })
    ));
    assert!(matches!(
        &stmts[20].kind,
        SeqStmtKind::VariableAssign {
            rhs: Rhs::Selected { matching: true, .. },
            ..
        }
    ));
}

// Concurrent statements.

#[test]
fn concurrent_statements() {
    let d = ok(&conc("p1: process (clk, rst) is variable v : integer; begin end process p1;
        process (all) begin end process;
        postponed process begin wait; end postponed process;
        b: block (en = '1') is generic (g : natural); generic map (g => 1); port (p : bit); port map (p => s);
           begin s <= guarded '1'; end block b;
        u1: entity work.ent(rtl) generic map (n => 1) port map (a => open, b => inertial c, d(0) => e, to_bit(f) => g);
        u2: component comp port map (x);
        u3: comp generic map (1);
        u4: configuration work.cfg;
        assert s = '1';
        p(s);
        s <= a when c else b;
        with sel select s <= a when '0', unaffected when others;
        g1: for i in 0 to 3 generate signal t : bit; begin t <= s; end generate g1;
        g2: if a: x = 1 generate s <= '1'; elsif b: x = 2 generate begin end b; else c: generate end generate g2;
        g3: case n generate when one: 1 => s <= '1'; when others => end generate;"));
    let stmts = arch_stmts(&d);
    assert_eq!(stmts.len(), 15);
    assert!(matches!(
        &stmts[1].kind,
        ConcStmtKind::Process(ProcessStmt {
            sensitivity: Some(Sensitivity::All(_)),
            ..
        })
    ));
    assert!(stmts[2].postponed);
    assert!(
        matches!(&stmts[3].kind, ConcStmtKind::Block(b) if b.guard.is_some() && b.port_map.is_some())
    );
    let ConcStmtKind::Instance(u1) = &stmts[4].kind else {
        panic!()
    };
    assert!(matches!(
        &u1.unit,
        InstantiatedUnit::Entity { arch: Some(_), .. }
    ));
    let pm = u1.port_map.as_ref().expect("port map");
    assert!(matches!(pm[0].actual, Actual::Open(_)));
    assert!(matches!(pm[1].actual, Actual::Inertial(_)));
    assert!(matches!(&stmts[6].kind, ConcStmtKind::Instance(_)));
    assert!(matches!(&stmts[9].kind, ConcStmtKind::ProcedureCall(_)));
    let ConcStmtKind::ForGenerate { body, .. } = &stmts[12].kind else {
        panic!()
    };
    assert_eq!(body.decls.len(), 1);
    let ConcStmtKind::IfGenerate { branches, else_ } = &stmts[13].kind else {
        panic!()
    };
    assert_eq!(branches.len(), 2);
    assert_eq!(branches[1].2.end_label.map(|l| l.text.as_str()), Some("b"));
    assert!(else_.is_some());
    assert!(matches!(&stmts[14].kind, ConcStmtKind::CaseGenerate { alts, .. } if alts.len() == 2));
}

// VHDL-2019.

#[test]
fn vhdl2019_generic_types() {
    let d = ok19(
        "entity e is generic (
        type t1 is private; type t2 is <>; type t3 is (<>); type t4 is range <>;
        type t5 is units <>; type t6 is range <> . <>; type t7 is array (t3 range <>) of t1;
        type t8 is access type is private; type t9 is file of t1;
        type t10 is array (type is (<>)) of type is private;); end;",
    );
    let LibraryUnit::Entity(e) = &d.units[0].unit else {
        panic!()
    };
    let defs: Vec<_> = e
        .generics
        .as_ref()
        .expect("generics")
        .iter()
        .map(|g| match g {
            InterfaceDecl::Type(t) => t.def.as_ref().expect("def"),
            g => panic!("{g:?}"),
        })
        .collect();
    assert!(matches!(defs[0], GenericTypeDef::Private));
    assert!(matches!(defs[5], GenericTypeDef::Floating));
    assert!(matches!(
        defs[7],
        GenericTypeDef::Access(IncompleteSubtype::Anonymous(..))
    ));
    assert!(matches!(
        defs[9],
        GenericTypeDef::Array {
            indexes: ArrayIndexes::Anonymous(_),
            ..
        }
    ));
    let e = errors("entity e is generic (type t is <>); end;");
    assert_eq!(e, ["a generic type definition requires VHDL-2019"]);
}

#[test]
fn vhdl2019_mode_views() {
    let d = ok19(
        "package p is
        type rec is record a : bit; b : bit; end record;
        view src of rec is a : out; b : in; end view src;
        alias sink is src'converse;
        end package;
        entity e is port (s : view src; t : view src of rec; u : view (src) of rec_array); end;",
    );
    let decls = pkg_decls(&d);
    assert!(matches!(&decls[1], Decl::View(v) if v.elements.len() == 2));
    let Decl::Alias(a) = &decls[2] else { panic!() };
    assert!(
        matches!(&a.name.kind, NameKind::Attribute { attr, .. } if attr.key.as_str() == "converse")
    );
    let LibraryUnit::Entity(e) = &d.units[1].unit else {
        panic!()
    };
    let ports = e.ports.as_ref().expect("ports");
    assert!(matches!(&ports[0], InterfaceDecl::Object(o) if o.subtype.is_none()));
    assert!(matches!(
        &ports[2],
        InterfaceDecl::Object(InterfaceObject {
            mode: Some(Mode::View { array: true, .. }),
            ..
        })
    ));
    // `view` is an identifier in VHDL-2008.
    ok("package p is signal view : bit; end;");
}

#[test]
fn vhdl2019_index_attribute_type_mark() {
    // `A'INDEX(n)` keeps its dimension; it is not an index constraint (#174).
    let d = ok19("package p is signal i : t'index(2); signal j : t'index range 0 to 1; end;");
    let decls = pkg_decls(&d);
    let Decl::Object(o) = &decls[0] else { panic!() };
    assert!(o.subtype.constraint.is_none());
    assert!(
        matches!(&o.subtype.type_mark.kind, NameKind::Apply { prefix, args }
        if args.len() == 1 && matches!(&prefix.kind, NameKind::Attribute { attr, .. } if attr.key.as_str() == "index"))
    );
    let Decl::Object(o) = &decls[1] else { panic!() };
    assert!(o.subtype.constraint.is_some());
}

#[test]
fn vhdl2019_protected_private() {
    let d = ok19(
        "package p is type pt is protected
        private variable count : integer;
        procedure inc;
        end protected; end;",
    );
    let Decl::Type(t) = &pkg_decls(&d)[0] else {
        panic!()
    };
    let Some(TypeDef::Protected(pd)) = &t.def else {
        panic!()
    };
    assert!(matches!(&pd.decls[0], Decl::Object(o) if o.private));
}

#[test]
fn vhdl2019_protected_instance() {
    let d = ok19("package p is type ib is new work.g.box generic map (t => integer); end;");
    let Decl::Type(t) = &pkg_decls(&d)[0] else {
        panic!()
    };
    assert!(matches!(
        &t.def,
        Some(TypeDef::ProtectedInstance { generic_map: Some(m), .. }) if m.len() == 1
    ));
    let e = errors(&pkg(
        "type ib is new work.g.box generic map (t => integer);",
    ));
    assert!(e.iter().any(|m| m.contains("2019")), "{e:?}");
}

#[test]
fn vhdl2019_relaxations() {
    // Sequential block, `end;` forms, trailing `;`, named return, empty record.
    ok19(&seq(
        "blk: block is variable v : integer; begin v := 1; end block blk;",
    ));
    ok19("package p is component c is port (a : bit;); end; type r is record end record; end;");
    let d = ok19("package p is function f return r of bit_vector; end;");
    let Decl::Subprogram(s) = &pkg_decls(&d)[0] else {
        panic!()
    };
    assert_eq!(s.return_ident.map(|i| i.text.as_str()), Some("r"));
    // Conditional expressions and `unaffected` in variable assignments.
    let e = first_default("constant c : integer := 1 when g else 2;", Std::Vhdl2019);
    assert!(matches!(&e.kind, ExprKind::Conditional(v) if v.len() == 2));
    ok19(&seq("v := 1 when now > 1 ns else unaffected;"));
    // Calls of uninstantiated subprograms.
    let e = first_default(
        "constant c : t := f generic map (integer)(x);",
        Std::Vhdl2019,
    );
    assert_eq!(show(&e), "f generic map(1)");
    // All of them are errors in VHDL-2008.
    for src in [
        seq("block begin end block;"),
        "package p is component c is port (a : bit;); end component; end;".to_string(),
        "package p is type r is record end record; end;".to_string(),
        "package p is function f return r of t; end;".to_string(),
    ] {
        let e = errors(&src);
        assert!(
            e.iter().any(|m| m.ends_with("requires VHDL-2019")),
            "{src}: {e:?}"
        );
    }
}

#[test]
fn vhdl2019_conditional_expression_positions() {
    // lrm-checks.md P2: the LRM positions, plus interface defaults (U-5).
    for src in [
        pkg("constant c : integer := 1 when g else 2;"),
        pkg("signal s : integer := (1 when g else 2);"),
        pkg("function f (x : integer := 1 when g else 2) return integer;"),
        pkg("function f (x : integer := (1 when g else 2)) return integer;"),
        "entity e is generic (h : integer := 1 when g else 2); end;".to_string(),
        seq("v := 1 when g else unaffected;"),
        seq("s <= force 1 when g else unaffected;"),
        seq("pr(1 when g else 2, x => 3 when g else 4);"),
        seq("v := 3 + (1 when g else 2);"),
        seq("v := integer'((1 when g else 2));"),
        seq("if (a when g else b) then end if;"),
        "architecture a of e is begin u: entity work.s port map (p => inertial 1 when g else 2); end;"
            .to_string(),
        "package body p is function f return integer is begin return 1 when g else 2; end; end;"
            .to_string(),
        "package body p is function f return integer is begin return 1 when g; return 2; end; end;"
            .to_string(),
        "package body p is procedure q is begin return when g; end; end;".to_string(),
    ] {
        ok19(&src);
    }
    // Not allowed: the operand of a qualified expression, a missing `else`
    // outside assignments and `return`, `unaffected` in initial values or
    // selected variable assignments.
    let e = errors_std(&seq("v := integer'(1 when g else 2);"), Std::Vhdl2019);
    assert!(e[0].contains("qualified expression"), "{e:?}");
    for src in [
        pkg("constant c : integer := 1 when g;"),
        pkg("constant c : integer := 1 when g else unaffected;"),
        seq("with x select v := unaffected when 1, 2 when others;"),
    ] {
        errors_std(&src, Std::Vhdl2019);
    }
    let e = errors("package body p is procedure q is begin return when g; end; end;");
    assert!(e.iter().any(|m| m.ends_with("requires VHDL-2019")), "{e:?}");
    let e = errors(&seq("v := (1 when g else 2);"));
    assert!(e.iter().any(|m| m.ends_with("requires VHDL-2019")), "{e:?}");
}

#[test]
fn vhdl2019_only_end_component_optional() {
    // LCS-2016-055a: only `end component` may drop its keyword (lrm-checks.md P1).
    for src in [
        pkg("type r is record a : bit; end;"),
        pkg("type t is range 0 to 1 units u; end;"),
        pkg("type pt is protected end;"),
        seq("if true then null; end;"),
        seq("l: loop exit; end;"),
        seq("case 1 is when others => null; end;"),
    ] {
        let e = errors_std(&src, Std::Vhdl2019);
        assert!(
            e.iter().any(|m| m.starts_with("expected `")),
            "{src}: {e:?}"
        );
    }
    let p = parse_std(&pkg("type r is record a : bit; end;"), Std::Vhdl2019);
    assert!(p.diags[0].help.iter().any(|h| h.contains("LCS-2016-055a")));
}

#[test]
fn conditional_analysis() {
    let src = "`if VHDL_VERSION >= \"2019\" then
        constant v : integer := 2019;
        `else
        constant v : integer := 2008;
        `end if";
    for (std, val) in [(Std::Vhdl2008, "2008"), (Std::Vhdl2019, "2019")] {
        let e = first_default(src, std);
        assert_eq!(show(&e), val);
    }
}

fn psl_warnings(src: &str) -> Vec<String> {
    let p = parse_std(src, Std::Vhdl2008);
    assert!(
        p.design.is_some(),
        "PSL must not be an error: {:?}",
        p.diags
    );
    p.diags.iter().map(|d| d.code.to_string()).collect()
}

#[test]
fn psl_formal_only_directives_are_skipped_silently() {
    // Nothing to run in simulation: no W0004, and the rest of the unit still parses.
    for f in [
        "restrict {a[*]; b};",
        "restrict_guarantee {a};",
        "assume_guarantee always a;",
        "l: restrict {a; b}[*];",
        "fairness a;",
        "strong fairness a, b;",
        "assume always (nondet(1, 2) = 1);",
        "assume always (nondet_vector(3, 0 to 7) = x);",
        "assert always (a -> next (nondet(0, 1) = 0));",
        "assert nondet(0, 1) = 0;",
        "cover {a; nondet(0, 1) = 1};",
    ] {
        let src = conc(&format!("{f}\ns <= '1';"));
        assert_eq!(psl_warnings(&src), Vec::<String>::new(), "{f}");
        let p = parse_std(&src, Std::Vhdl2008);
        let LibraryUnit::Architecture(a) = &p.design.unwrap().units[0].unit else {
            panic!()
        };
        assert_eq!(a.stmts.len(), 1, "{f}: the assignment survives");
    }
    // A directive that is not formal-only still warns, also after one that is.
    let src = conc("fairness a;\nassert eventually! a;");
    assert_eq!(psl_warnings(&src), ["W0004"]);
}

#[test]
fn psl_is_skipped_with_one_warning_per_unit() {
    // Every PSL form is skipped and the rest of the file still parses.
    let forms = [
        "property p is always (a -> eventually! b);",
        "sequence s (bit x) is {x; b[*2]; c};",
        "property q (numeric r) is always {r = 1; r = 1} |=> r = 2;",
        "assert always (a -> next[1](b));",
        "assert never {a; b};",
        "l: assume always a;",
        "cover {a; b};",
        "assert always a report \"x\" severity error;",
        "one: assert (always (a -> next [2] (b))) until! areset;",
        "assert always (a -> next (not a or (b before_ a)));",
        "assert {a; b} |-> {c};",
        "assert eventually! a;",
    ];
    for f in forms {
        let src = conc(&format!("{f}\ns <= '1';"));
        assert_eq!(psl_warnings(&src), ["W0004"], "{f}");
        let p = parse_std(&src, Std::Vhdl2008);
        let LibraryUnit::Architecture(a) = &p.design.unwrap().units[0].unit else {
            panic!()
        };
        assert_eq!(a.stmts.len(), 1, "{f}: the assignment survives");
    }
    // Declaration forms, in a package.
    for f in &forms[..3] {
        let w = psl_warnings(&pkg(&format!("{f}\nconstant c : integer := 1;")));
        assert_eq!(w, ["W0004"], "{f}");
    }
    // Several constructs in one unit still give one warning.
    let src = conc("assert always a;\ncover {a; b};");
    assert_eq!(psl_warnings(&src), ["W0004"]);
    // Two units, two warnings.
    let src = "vunit v (e) { assert always a; }\nvmode m (e) { assume always {a; b}; }\nvprop p (e) { }\nentity e is end;";
    assert_eq!(psl_warnings(src), ["W0004", "W0004", "W0004"]);
    // A plain VHDL assertion is not PSL, even on a signal named `always`, `abort` or `before`.
    assert!(psl_warnings(&conc("assert always;\nassert a report \"x\";")).is_empty());
    for f in [
        "assert abort = '0' report \"x\";",
        "assert before report \"x\";",
        "assert (x = until_) report \"x\";",
        "assert iff or x report \"x\";",
    ] {
        assert!(psl_warnings(&conc(f)).is_empty(), "{f}");
    }
}

#[test]
fn psl_pragma_needs_the_flag() {
    let src = "architecture a of e is\n-- psl default clock is rising_edge(clk);\nbegin\n-- psl assert always a;\nend;";
    let off = |psl| {
        let p = parse_str(
            FileId(0),
            src,
            &Options {
                std: Std::Vhdl2008,
                defines: Vec::new(),
                psl,
            },
        );
        let LibraryUnit::Architecture(a) = &p.design.unwrap().units[0].unit else {
            panic!()
        };
        a.stmts.len()
    };
    assert_eq!((off(false), off(true)), (0, 1));
}

/// The architecture's statements: (label, process) of each checker, `None` for others.
fn checkers(src: &str) -> Vec<(Option<String>, usize, usize)> {
    assert_eq!(psl_warnings(src), Vec::<String>::new(), "{src}");
    let p = parse_std(src, Std::Vhdl2008);
    let LibraryUnit::Architecture(a) = &p.design.unwrap().units[0].unit else {
        panic!()
    };
    a.stmts
        .iter()
        .map(|s| match &s.kind {
            ConcStmtKind::Process(p) => (
                s.label.map(|l| l.text.to_string()),
                p.decls.len(),
                p.stmts.len(),
            ),
            k => panic!("not a checker: {k:?}"),
        })
        .collect()
}

#[test]
fn psl_subset_becomes_checker_processes() {
    // (label, declarations, statements): `wait until` plus the check; `next[n]` adds one
    // variable declaration and n shift statements.
    let src = "architecture a of e is
        default clock is rising_edge(clk);
        begin
        p1: assert always a;
        assert never (b and c) report \"no\";
        assume always (a -> b) severity warning;
        assert always a -> next b;
        p4: assert always (a -> next[3] (b)) @falling_edge(clk);
        assert (always a) @rising_edge(clk);
        cover {a = '1'} report \"hit\";
        cover a;
        end;";
    assert_eq!(
        checkers(src),
        [
            (Some("p1".into()), 0, 2),
            (None, 0, 2),
            (None, 0, 2),
            (None, 1, 3),
            (Some("p4".into()), 1, 5),
            (None, 0, 2),
            (None, 0, 2),
            (None, 0, 2),
        ]
    );
    // `-- psl` pragmas, one statement over several comment lines.
    let src = "architecture a of e is
        -- psl default clock is clk'event and clk = '1';
        begin
        -- psl l: assert never
        --   (a = 1)
        --   report \"x\";
        -- PSL assert failed: prose, not a pragma
        -- psl is great
        end;";
    assert_eq!(checkers(src), [(Some("l".into()), 0, 2)]);
}

#[test]
fn psl_sequences_become_checker_processes() {
    // Every form is one checker process and no warning.
    let forms = [
        "cover {a; b};",
        "cover {a; b[*2 to 3]; c[+]} report \"x\";",
        "cover {a[->2]; b[=1]; c[*]};",
        "cover {a : b};",
        "cover {{a; b} : {b; c}};",
        "cover {{a; b} | {c}};",
        "cover {{a; b} && {c; c}};",
        "cover {{a; b} & {c[*3]}};",
        "cover {b within {a; c; a}};",
        "cover {{a; b}[*2]};",
        "cover {a[*0 to 2]; b};",
        "assert never {a; b};",
        "assert always {a; b} |-> c;",
        "assert always {a; b} |=> {c; a};",
        "assert always {a} |=> next[2] b;",
        "assert always {a; b};",
        "assert {a; b} |-> {c};",
        "assert (always {a} |=> {b[*1 to 2]; c});",
        "assert always ({a; b} |-> c) @falling_edge(clk);",
        "l: assume always {a} |-> {b} report \"r\" severity warning;",
        "cover {rose(a); stable(b)};",
        "cover {[*]; a; b};",
        "cover {[*2]; a};",
        "assert always a |=> b;",
        "assert a |-> next b;",
        "assert never a[*3];",
        "assert never (a = '1')[->2];",
        "cover a[*2];",
    ];
    for f in forms {
        let src =
            format!("architecture a of e is\ndefault clock is rising_edge(clk);\nbegin\n{f}\nend;");
        let c = checkers(&src);
        assert_eq!(c.len(), 1, "{f}");
    }
    // Outside the sequence forms: skipped, one warning.
    for f in [
        "cover {a; b}!;",
        "assert always {a; b} |-> c until! d;",
        "assert always {{a} | {b} && {c}};",
        "cover {a[*70]};",
        "cover {{a[*60]}[*60]};",
    ] {
        let src =
            format!("architecture a of e is\ndefault clock is rising_edge(clk);\nbegin\n{f}\nend;");
        assert_eq!(psl_warnings(&src), ["W0004"], "{f}");
    }
}

#[test]
fn psl_temporal_properties() {
    let pre = "architecture a of e is\ndefault clock is rising_edge(clk);\nbegin\n";
    for f in [
        "assert always (a -> next_a[1 to 3] (b));",
        "assert always (a -> next_e[2 to 4] (b = '1')) report \"x\";",
        "assert always (a -> next_event(c)(b));",
        "assert always (a -> next_event(c)[2](b));",
        "assert always (a -> next_event_a(c)[1 to 3](b));",
        "assert always (a -> next_event_e(c)[1 to 3](b));",
        "assert always (a -> (b until c));",
        "assert always (a -> (b until_ c));",
        "assert always (a -> (b before c));",
        "assert always (a -> (b before_ c));",
        "assert always (a <-> b);",
        "assert always (a iff b);",
        "assert always ((a -> next b) and (a -> next[2] (c)));",
        "assert always ((a -> next b) or (a -> next[2] (c)));",
        "assert always ((next[2] (b)) sync_abort c);",
        "assert (always (a -> next b)) sync_abort c;",
        "assert always ((a -> next b) abort c);",
        "assert always ((a -> next b) async_abort c);",
        "assert (always (a -> next b)) abort c;",
        "assert always {a; b} |=> (c until d);",
        "assert always {a} |-> next_e[1 to 3] (b);",
        "assert (a -> next_a[1 to 2] (b));",
        "assume always (a -> next (b -> next[3] (c)));",
        "l: assert always (a -> next (always (b -> next c))) report \"x\" severity warning;",
    ] {
        let src = format!("{pre}{f}\nend;");
        let c = checkers(&src);
        assert_eq!(c.len(), 1, "{f}");
    }
    // Strong operators and what the checker cannot hold: skipped.
    for f in [
        "assert always (a -> eventually! b);",
        "assert always (a -> next! b);",
        "assert always (a -> (b until! c));",
        "assert always (a -> next_a![1 to 3] (b));",
        "assert always (a -> (b before! c));",
        "assert always (a -> next_a[1 to 99] (b));",
        "assert always ((a until b) -> c);",
        "assert always (a -> next_event(c)[0](b));",
    ] {
        let src = format!("{pre}{f}\nend;");
        assert_eq!(psl_warnings(&src), ["W0004"], "{f}");
    }
}

#[test]
fn psl_default_clock_as_statement() {
    // The clock may be declared among the concurrent statements too.
    let src =
        "architecture a of e is\nbegin\ndefault clock is rising_edge(clk);\nassert always a;\nend;";
    assert_eq!(checkers(src).len(), 1);
}

#[test]
fn psl_named_sequences_and_properties() {
    let pre = "architecture a of e is\ndefault clock is rising_edge(clk);\nbegin\n";
    // Declarations produce nothing; every use is one checker process.
    let src = format!(
        "{pre}sequence s1 is {{a; b}};
        sequence s2 (boolean x, y) is {{x; y[*2]}};
        sequence s3 (boolean x; const n) is {{x[*n]; s1}};
        sequence s4 (sequence p) is {{p; p}};
        property p1 is always (a -> next b);
        property p2 (boolean x, y) is always (x -> next y);
        property p3 is always (s1 |=> c);
        property p4 (property q) is always (a -> next q);
        property p5 (property q, sequence s) is always (s |=> q);
        c1: cover s1;
        c2: cover s2(a, b);
        c3: cover s3(a, 2);
        c4: cover {{s4({{a; b}}); c}};
        c5: assert p1;
        c6: assert p2(a = '1', b) report \"x\";
        c7: assert p3 @falling_edge(clk);
        c8: assert always s1 |=> c;
        c9: assert p4(b);
        c10: assert p4((c until d));
        c11: assert p5(next_e[1 to 2] (b), {{a; b}});
        end;"
    );
    let c = checkers(&src);
    assert_eq!(c.len(), 11, "{c:?}");
    // Not supported, so skipped with one warning: property and typed formals, a wrong
    // number of actuals, a recursive sequence, a declaration with an operator the subset
    // does not run, a built-in on a formal.
    for (decl, uses) in [
        ("property p (property q) is always q;", "assert p(a, b);"),
        ("sequence s (bit x) is {x; x};", "cover s(a);"),
        ("sequence s (boolean x) is {x; x};", "cover s(a, b);"),
        ("sequence s is {a; s};", "cover s;"),
        ("property p is always (a -> eventually! b);", "assert p;"),
        ("sequence s (boolean x) is {rose(x); b};", "cover s(a);"),
    ] {
        let src = format!("{pre}{decl}\n{uses}\nend;");
        assert_eq!(psl_warnings(&src), ["W0004"], "{decl} {uses}");
    }
}

#[test]
fn psl_outside_the_subset_is_skipped() {
    let pre = "architecture a of e is\ndefault clock is rising_edge(clk);\nproperty p is always a;\nbegin\n";
    for f in [
        "assert always p;",
        "assert always a -> next_e[1 to 2] b;",
        "assert always (a -> b) until! rst;",
        "assert always a -> next[300] b;",
        "assert always a -> next a -> next b;",
        "assert eventually! a;",
        "cover {a; b}!;",
    ] {
        let src = format!("{pre}{f}\nend;");
        assert_eq!(psl_warnings(&src), ["W0004"], "{f}");
    }
    // A directive that only names a skipped property is skipped too (#593).
    let src =
        "architecture a of e is\nproperty p0 is always a;\nbegin\nl: assert p0 report \"x\";\nend;";
    assert_eq!(psl_warnings(src), ["W0004"]);
    // Without a clock: skipped.
    assert_eq!(psl_warnings(&conc("assert always a;")), ["W0004"]);
    // A default clock holds in its block only.
    let src =
        conc("b: block default clock is clk; begin assert always a; end block;\nassert always a;");
    assert_eq!(psl_warnings(&src), ["W0004"]);
}

// Error recovery.

#[test]
fn reports_independent_errors() {
    let e = errors(&seq("x := ;\ny := 1;\nz <= ;\nw := 2;"));
    assert_eq!(e.len(), 2, "{e:?}");
}

#[test]
fn missing_semicolon_is_inserted() {
    let e = errors(&seq("x := a\ny := 2;"));
    assert_eq!(e, ["expected `;`, found identifier"]);
}

#[test]
fn unclosed_construct_points_at_opener() {
    let p = parse_std(
        "architecture a of e is begin process begin wait;",
        Std::Vhdl2008,
    );
    let d = p
        .diags
        .iter()
        .find(|d| d.message.starts_with("expected `end`"))
        .expect("diag");
    assert_eq!(d.secondary.len(), 1);
    assert!(d.secondary[0].message.contains("process"), "{d:?}");
}

#[test]
fn end_label_mismatch() {
    let e = errors("entity foo is end entity bar;");
    assert_eq!(e, ["end label `bar` does not match `foo`"]);
}

#[test]
fn deep_nesting_does_not_overflow() {
    let deep = format!(
        "constant c : integer := {}1{};",
        "(".repeat(5000),
        ")".repeat(5000)
    );
    let e = std::thread::Builder::new()
        .stack_size(2 << 20)
        .spawn(move || errors(&pkg(&deep)))
        .expect("spawn")
        .join()
        .expect("no stack overflow");
    assert_eq!(e, ["nesting is too deep"]);
}

/// A chain of parenthesised groups like GRLIB's `can_top.vhd` has: one level of parentheses is one
/// level of depth, so 60 are fine and 70 are too deep (#604).
#[test]
fn parenthesised_chain_is_deep_but_bounded() {
    let chain = |n: usize| {
        format!(
            "constant c : boolean := {}true{};",
            "(".repeat(n),
            " and true)".repeat(n)
        )
    };
    let run = move |n: usize| {
        std::thread::Builder::new()
            .stack_size(2 << 20)
            .spawn(move || parse_std(&pkg(&chain(n)), Std::Vhdl2008).design.is_some())
            .expect("spawn")
            .join()
            .expect("no stack overflow")
    };
    assert!(run(40));
    assert!(run(60));
    assert!(!run(70));
}

#[test]
fn garbage_does_not_panic() {
    for src in [
        "",
        ";;;",
        "end",
        "entity",
        "(((",
        "begin end",
        "'",
        "`if",
        "<< signal",
        "\\",
        "\"",
        "architecture a of",
    ] {
        let _ = parse_std(src, Std::Vhdl2019);
    }
}

// Source encoding.

/// Load `bytes` with `encoding` and parse them as a package of declarations.
fn parse_bytes(bytes: &[u8], encoding: Encoding) -> (yavs_diag::SourceMap, crate::Parsed) {
    let mut sources = yavs_diag::SourceMap::default();
    let (file, d) = load_source(&mut sources, "t.vhd", bytes.to_vec(), encoding);
    assert!(d.is_none(), "{d:?}");
    let parsed = parse_file(file, sources.get(file), &Options::default());
    (sources, parsed)
}

/// The value of the first string constant in `bytes`.
fn string_value(bytes: &[u8], encoding: Encoding) -> String {
    let (_, p) = parse_bytes(bytes, encoding);
    assert!(p.diags.is_empty(), "{:?}", p.diags);
    let d = p.design.expect("design");
    let LibraryUnit::Package(pk) = &d.units[0].unit else {
        panic!()
    };
    let Decl::Object(o) = &pk.decls[0] else {
        panic!()
    };
    let Some(Expr {
        kind:
            ExprKind::Literal(Literal {
                kind: LiteralKind::String(s),
                ..
            }),
        ..
    }) = &o.default
    else {
        panic!("{:?}", o.default)
    };
    s.as_str().to_string()
}

const E_ACUTE_PKG: &str = "package p is constant c : string := \"\u{e9}\"; end;";

#[test]
fn utf8_latin1_character_is_one_character() {
    let v = string_value(E_ACUTE_PKG.as_bytes(), Encoding::Auto);
    assert_eq!(v.chars().count(), 1);
    assert_eq!(v, "\u{e9}");
}

#[test]
fn non_latin1_in_string_is_an_error() {
    let src = "package p is constant c : string := \"a \u{2192} b\"; end;";
    let (sources, p) = parse_bytes(src.as_bytes(), Encoding::Auto);
    assert!(p.design.is_none());
    assert_eq!(p.diags.len(), 1);
    let d = &p.diags[0];
    assert_eq!(d.code, crate::codes::NOT_LATIN1);
    assert_eq!(
        d.message,
        "character `\u{2192}` (U+2192) is not in ISO-8859-1"
    );
    let span = d.primary.as_ref().expect("span").span;
    assert_eq!(
        &sources.get(span.file).bytes()[span.start as usize..span.end as usize],
        "\u{2192}".as_bytes()
    );
    assert!(d.help[0].contains("--encoding=latin1"));
    // The same in a character literal, an extended identifier and an identifier.
    for src in [
        "constant c : character := '\u{2192}';",
        "signal \\a\u{2192}\\ : bit;",
        "signal \u{3b1} : bit;",
    ] {
        let (_, p) = parse_bytes(pkg(src).as_bytes(), Encoding::Auto);
        assert_eq!(
            p.diags[0].code,
            crate::codes::NOT_LATIN1,
            "{src}: {:?}",
            p.diags
        );
    }
}

#[test]
fn anything_goes_in_comments() {
    let src = "-- \u{2192} \u{1f600}\npackage p is /* \u{2192} */ end;";
    let (_, p) = parse_bytes(src.as_bytes(), Encoding::Auto);
    assert!(p.diags.is_empty(), "{:?}", p.diags);
}

#[test]
fn invalid_utf8_is_read_as_latin1() {
    // 0xE9 alone is not UTF-8: the file is Latin-1 and the string is `é`.
    let bytes = b"package p is constant c : string := \"\xe9\"; end;";
    assert_eq!(string_value(bytes, Encoding::Auto), "\u{e9}");
    // Spans stay byte offsets into the file: the error after the Latin-1
    // character points at the right byte.
    let bad = b"package p is constant c : string := \"\xe9\" & ; end;";
    let (sources, p) = parse_bytes(bad, Encoding::Auto);
    let span = p.diags[0].primary.as_ref().expect("span").span;
    assert_eq!(bad[span.start as usize], b';');
    assert!(sources.get(span.file).is_latin1());
}

#[test]
fn encoding_latin1_reads_bytes() {
    // UTF-8 `é` is two bytes, so two Latin-1 characters.
    assert_eq!(
        string_value(E_ACUTE_PKG.as_bytes(), Encoding::Latin1),
        "\u{c3}\u{a9}"
    );
    // A character beyond Latin-1 is just bytes, so no error.
    let src = "package p is constant c : string := \"\u{2192}\"; end;";
    assert_eq!(
        string_value(src.as_bytes(), Encoding::Latin1)
            .chars()
            .count(),
        3
    );
}

#[test]
fn encoding_utf8_rejects_invalid_utf8() {
    let mut sources = yavs_diag::SourceMap::default();
    let (_, d) = load_source(
        &mut sources,
        "t.vhd",
        b"-- ok\n\xe9".to_vec(),
        Encoding::Utf8,
    );
    let d = d.expect("diagnostic");
    assert_eq!(d.code, crate::codes::INVALID_UTF8);
    assert_eq!(d.primary.expect("span").span.start, 6);
}

// ---- VHDL-1993 (#629) ----

fn ok93(src: &str) -> DesignFile {
    ok_std(src, Std::Vhdl1993)
}

fn codes93(src: &str) -> Vec<String> {
    let p = parse_std(src, Std::Vhdl1993);
    assert!(p.design.is_none(), "expected errors");
    p.diags.iter().map(|d| d.code.to_string()).collect()
}

#[test]
fn vhdl_1993_reserved_words_are_identifiers() {
    ok93(
        "package p is constant default : integer := 1; signal context, force, release : bit; end;",
    );
    assert!(!errors("package p is constant default : integer := 1; end;").is_empty());
}

#[test]
fn vhdl_1993_rejects_2008_statements() {
    for src in [
        seq("wait;").replace("process begin", "process (all) begin"),
        seq("s <= '1' when c else '0';"),
        seq("with c select s <= '1' when true, '0' when others;"),
        seq("v := 1 when c else 2;"),
        "architecture a of e is begin g: if c generate begin elsif d generate begin end generate; end;"
            .to_string(),
        "architecture a of e is begin g: case c generate when true => s <= '1'; end generate; end;"
            .to_string(),
        pkg("").replace("package p is", "package p is generic (n : integer := 1);"),
        "package q is new work.p generic map (n => 1);".to_string(),
    ] {
        assert_eq!(codes93(&src), ["E0011"], "{src}");
    }
    // 2008 takes them.
    ok(&seq("s <= '1' when c else '0';"));
}

#[test]
fn vhdl_1993_keeps_its_own_statements() {
    ok93(&seq(
        "s <= '1' after 1 ns; if c then v := 1; elsif d then v := 2; end if; wait on s;",
    ));
    ok93("architecture a of e is begin g: if c generate begin end generate; end;");
    ok93(
        "architecture a of e is begin s <= '1' when c else '0'; with c select t <= '1' when true, '0' when others; end;",
    );
}

const ENVELOPE: &str = "library ieee;\nuse ieee.std_logic_1164.all;\n\
`protect begin_protected\n`protect version = 1\n`protect encrypt_agent = \"ACME\"\n\
`protect key_keyowner = \"V1\", key_keyname = \"K1\", key_method = \"rsa\"\n`protect key_block\n\
gXD3eJb5QXOvO8ipSY44ZeDNBPOmJ+mNfS3w8DY0bWv1SMCs 12ns 0x\"\n`protect key_block\nAAAA\n\
`protect data_method = \"AES128-CBC\"\n`protect data_block\n1ApGWQyrhNARS7n9r5ePL6NW\n\
`protect end_protected\n";

#[test]
fn encrypted_envelope_is_one_error() {
    for src in [
        ENVELOPE.to_string(),
        ENVELOPE.replace("`protect begin", "`PRAGMA Protect Begin"),
    ] {
        let p = parse_std(&src, Std::Vhdl2008);
        assert!(p.design.is_none());
        let codes: Vec<_> = p.diags.iter().map(|d| d.code.to_string()).collect();
        assert_eq!(codes, ["E0012"], "{:?}", p.diags);
        let span = p.diags[0].primary.as_ref().expect("span").span;
        assert_eq!(span.start as usize, src.find('`').unwrap());
        assert!(
            src[span.start as usize..span.end as usize]
                .to_lowercase()
                .ends_with("begin_protected")
        );
        assert!(p.diags[0].notes[0].contains("ACME"));
    }
}

#[test]
fn plaintext_protect_regions_are_not_envelopes() {
    // `protect begin/end mark text to encrypt; the text itself stays source.
    let p = parse_std(
        "`protect begin\nentity e is end;\n`protect end\n",
        Std::Vhdl2008,
    );
    assert!(p.diags.iter().all(|d| d.code != "E0012"));
}
