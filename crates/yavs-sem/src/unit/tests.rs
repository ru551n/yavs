//! Tests of design units, associations, binding and configurations
//! (binding.rs). Each test analyzes real files, written to its own
//! temporary directory (never into the repo), so parallel tests and other
//! worktrees' runs never overwrite each other's files.

use yavs_lib::{UnitKind as LibKind, UnitRecord, UnitRef};

use crate::hir::{
    Actual, BlockTarget, ConfigItem, GenSelect, InstTarget, InstanceSel, StmtKind, UnitHir,
};
use crate::loader::UnitKind;
use crate::{Db, LangRev, Options};

struct Run {
    db: Db,
    records: Vec<Vec<UnitRecord>>,
}

impl Run {
    /// `(code, line)` of every diagnostic.
    fn codes(&self) -> Vec<(&'static str, usize)> {
        self.db
            .diagnostics()
            .iter()
            .map(|d| {
                let line = d.primary.as_ref().map_or(0, |p| {
                    self.db.sources.get(p.span.file).line_col(p.span.start).0
                });
                (d.code, line)
            })
            .collect()
    }

    fn clean(&self) {
        let text: Vec<String> = self
            .db
            .diagnostics()
            .iter()
            .map(|d| format!("{} {}", d.code, d.message))
            .collect();
        assert!(text.is_empty(), "{text:#?}");
    }

    fn unit(&self, kind: UnitKind, name: &str) -> &crate::loader::Unit {
        self.db
            .units
            .iter()
            .map(|(_, u)| u)
            .find(|u| u.kind == kind && u.name.key.as_str() == name)
            .unwrap_or_else(|| panic!("no {kind:?} {name}"))
    }

    fn arch_stmts(&self, name: &str) -> Vec<crate::ids::StmtId> {
        match &self.unit(UnitKind::Architecture, name).hir {
            UnitHir::Architecture { stmts, .. } => stmts.to_vec(),
            _ => panic!("architecture {name} has no HIR"),
        }
    }
}

/// Analyze `(library, source)` files in order, in one session.
fn run(test: &str, files: &[(&str, &str)]) -> Run {
    let tmp = tempfile::tempdir().expect("scratch dir");
    let mut db = Db::new(Options::new(LangRev::Vhdl2008, crate::default_stdlib()));
    let mut records = Vec::new();
    for (k, (lib, src)) in files.iter().enumerate() {
        let path = tmp.path().join(format!("{test}_{k}.vhd"));
        std::fs::write(&path, src).expect("write source");
        records.push(
            db.analyze_file(lib, &path, LangRev::Vhdl2008)
                .unwrap_or_default(),
        );
    }
    Run { db, records }
}

fn one(test: &str, src: &str) -> Run {
    run(test, &[("work", src)])
}

const SUB: &str = "
entity sub is
  generic (n : integer := 2; m : integer);
  port (i : in bit_vector(0 to 1); o : out bit; io : inout integer := 0);
end entity;
architecture rtl of sub is begin o <= i(0); end architecture;
";

#[test]
fn entity_interface_and_statement_part() {
    let r = one(
        "entity",
        "entity e is
           generic (w : natural := 4);
           port (a : in bit; b : out bit_vector(w - 1 downto 0));
           constant k : natural := w;
         begin
           assert w > 0 report \"w\" severity failure;
         end entity;",
    );
    r.clean();
    let UnitHir::Entity {
        generics,
        ports,
        stmts,
    } = &r.unit(UnitKind::Entity, "e").hir
    else {
        panic!("entity HIR");
    };
    assert_eq!((generics.len(), ports.len(), stmts.len()), (1, 2, 1));
}

#[test]
fn positional_named_partial_and_open_associations() {
    let src = format!(
        "{SUB}
entity top is end entity;
architecture a of top is
  signal x, y, z, z2 : bit;
  signal q : integer;
begin
  u1 : entity work.sub generic map (3, m => 1)
       port map (i(0) => x, i(1) => y, o => z, io => open);
  u2 : entity work.sub(rtl) generic map (m => 1) port map (\"01\", z2, q);
end architecture;"
    );
    let r = one("assoc", &src);
    r.clean();
    let stmts = r.arch_stmts("a");
    let StmtKind::Instance {
        generic_map,
        port_map,
        target,
        ..
    } = &r.db.stmts[stmts[0]].kind
    else {
        panic!("instance");
    };
    assert!(matches!(target, InstTarget::Entity { arch: None, .. }));
    assert!(matches!(generic_map[0].actual, Actual::Expr(_)));
    assert!(matches!(generic_map[1].actual, Actual::Expr(_)));
    match &port_map[0].actual {
        Actual::Individual(parts) => assert_eq!(parts.len(), 2),
        _ => panic!("partial association expected"),
    }
    assert!(matches!(port_map[2].actual, Actual::Open));
    let StmtKind::Instance { generic_map, .. } = &r.db.stmts[stmts[1]].kind else {
        panic!("instance");
    };
    // Defaults fill the formals not associated, in formal order.
    assert!(matches!(generic_map[0].actual, Actual::Default));
}

#[test]
fn conversion_functions_in_port_maps() {
    let r = one(
        "conv",
        "package p is
           function to_int (b : bit) return integer;
           function to_bit (i : integer) return bit;
         end package;
         use work.p.all;
         entity sub is port (i : in integer; o : out integer); end entity;
         use work.p.all;
         entity top is end entity;
         architecture a of top is
           signal x, y : bit;
         begin
           u : entity work.sub port map (i => to_int(x), to_bit(o) => y);
         end architecture;",
    );
    r.clean();
    let s = r.arch_stmts("a")[0];
    let StmtKind::Instance { port_map, .. } = &r.db.stmts[s].kind else {
        panic!("instance");
    };
    assert!(matches!(
        port_map[0].actual,
        Actual::Converted {
            formal_conv: None,
            actual_conv: Some(_),
            ..
        }
    ));
    assert!(matches!(
        port_map[1].actual,
        Actual::Converted {
            formal_conv: Some(_),
            actual_conv: None,
            ..
        }
    ));
}

#[test]
fn operator_actual_is_an_expression() {
    // `not x` is not `function_name(actual)` syntax (6.5.7.1): an expression actual.
    let r = one(
        "opact",
        "entity sub is port (i : in bit); end entity;
         entity top is end entity;
         architecture a of top is
           signal x : bit;
         begin
           u : entity work.sub port map (i => not x);
         end architecture;",
    );
    r.clean();
    let s = r.arch_stmts("a")[0];
    let StmtKind::Instance { port_map, .. } = &r.db.stmts[s].kind else {
        panic!("instance");
    };
    assert!(matches!(port_map[0].actual, Actual::Expr(_)));
}

#[test]
fn association_errors() {
    let src = format!(
        "{SUB}
entity top is end entity;
architecture a of top is
  signal x : bit;
  constant c : integer := 0;
begin
  u1 : entity work.sub generic map (m => 1, 2) port map (\"00\", x);
  u2 : entity work.sub generic map (n => 1, m => open) port map (\"00\", x);
  u3 : entity work.sub generic map (m => 1) port map (open, x);
  u4 : entity work.sub generic map (m => 1) port map (\"00\", x, c);
  u5 : entity work.sub generic map (1, 2, 3) port map (\"00\", x);
end architecture;"
    );
    let r = one("assoc_err", &src);
    let base = SUB.lines().count() + 7;
    let want = vec![
        ("E1310", base),     // positional after named
        ("E1502", base + 1), // generic without default left open
        ("E1311", base + 2), // in port without default left open
        ("E1360", base + 3), // constant for an inout port
        ("E1100", base + 4), // too many actuals
    ];
    assert_eq!(r.codes(), want);
}

#[test]
fn component_instance_with_specification_and_dependencies() {
    let r = run(
        "comp",
        &[
            ("work", SUB),
            (
                "work",
                "entity top is end entity;
                 architecture a of top is
                   component sub is
                     generic (m : integer);
                     port (i : in bit_vector(0 to 1); o : out bit);
                   end component;
                   for u1 : sub use entity work.sub(rtl);
                   signal z : bit;
                 begin
                   u1 : sub generic map (m => 1) port map (\"00\", z);
                   u2 : component sub generic map (m => 2) port map (\"11\", open);
                 end architecture;",
            ),
        ],
    );
    r.clean();
    let a = r.unit(UnitKind::Architecture, "a");
    let specs = &r.db.config_specs[&a.scope];
    assert_eq!(specs.len(), 1);
    assert!(matches!(&specs[0].instances, InstanceSel::Labels(l) if l.len() == 1));
    // The binding's default port map leaves `io` (with a default) to its default.
    let b = specs[0].binding.as_ref().expect("binding");
    assert_eq!(b.port_map.len(), 3);
    let arch = r.records[1]
        .iter()
        .find(|u| u.kind == LibKind::Architecture)
        .expect("architecture record");
    assert!(
        arch.depends_on
            .contains(&UnitRef::new("work", LibKind::Entity, "sub"))
    );
    assert!(
        arch.depends_on
            .contains(&UnitRef::new("work", LibKind::Architecture, "sub/rtl"))
    );
}

#[test]
fn default_binding_prefers_the_directly_visible_entity() {
    let r = run(
        "default_binding",
        &[
            (
                "other",
                "entity sub is end entity; architecture x of sub is begin end architecture;",
            ),
            (
                "work",
                "entity sub is end entity;
                      architecture r1 of sub is begin end architecture;
                      architecture r2 of sub is begin end architecture;",
            ),
            (
                "work",
                "library other; use other.sub;
                 entity top is end entity;
                 architecture a of top is
                   component sub is end component;
                 begin
                   u : sub;
                 end architecture;",
            ),
        ],
    );
    assert_eq!(r.codes(), vec![("W1703", 6)]);
    let s = r.arch_stmts("a")[0];
    let StmtKind::Instance {
        target: InstTarget::Component(c),
        scope,
        ..
    } = r.db.stmts[s].kind
    else {
        panic!("component instance");
    };
    assert_eq!(scope, r.unit(UnitKind::Architecture, "a").scope);
    let mut db = r.db;
    let (e, arch) = db.default_binding(scope, c).expect("bound");
    assert_eq!(db.libs[db.units[e].lib].name.key.as_str(), "other");
    assert_eq!(arch.map(|a| a.to_string()).as_deref(), Some("x"));
}

/// A component declared in a package binds by default to the entity in the package's library,
/// also when the design unit's own library has none (#291) or has another one (`W1703`).
#[test]
fn default_binding_uses_the_library_of_the_component_declaration() {
    let pkg = (
        "liba",
        "package comp_pkg is component leaf is end component; end package;
         entity leaf is end entity; architecture x of leaf is begin end architecture;",
    );
    let user = (
        "work",
        "library liba; use liba.comp_pkg.all;
         entity top is end entity;
         architecture a of top is begin u : leaf; end architecture;",
    );
    for own in [
        None,
        Some("entity leaf is end entity; architecture y of leaf is begin end architecture;"),
    ] {
        let mut files = vec![pkg];
        files.extend(own.map(|o| ("work", o)));
        files.push(user);
        let r = run("default_decl_lib", &files);
        assert_eq!(
            r.codes(),
            if own.is_some() {
                vec![("W1703", 3)]
            } else {
                vec![]
            }
        );
        let StmtKind::Instance {
            target: InstTarget::Component(c),
            scope,
            ..
        } = r.db.stmts[r.arch_stmts("a")[0]].kind
        else {
            panic!("component instance");
        };
        let mut db = r.db;
        let (e, arch) = db.default_binding(scope, c).expect("bound");
        assert_eq!(db.libs[db.units[e].lib].name.key.as_str(), "liba");
        assert_eq!(arch.map(|a| a.to_string()).as_deref(), Some("x"));
    }
}

#[test]
fn default_binding_takes_the_last_architecture() {
    let r = run(
        "default_arch",
        &[
            (
                "work",
                "entity sub is end entity;
                      architecture r1 of sub is begin end architecture;
                      architecture r2 of sub is begin end architecture;",
            ),
            (
                "work",
                "entity top is end entity;
                 architecture a of top is
                   component sub is end component;
                 begin
                   u : sub;
                 end architecture;",
            ),
        ],
    );
    r.clean();
    let s = r.arch_stmts("a")[0];
    let StmtKind::Instance {
        target: InstTarget::Component(c),
        scope,
        ..
    } = r.db.stmts[s].kind
    else {
        panic!("component instance");
    };
    assert_eq!(scope, r.unit(UnitKind::Architecture, "a").scope);
    let mut db = r.db;
    let (_, arch) = db.default_binding(scope, c).expect("bound");
    assert_eq!(arch.map(|a| a.to_string()).as_deref(), Some("r2"));
}

#[test]
fn configuration_with_blocks_and_generates() {
    let r = one(
        "config",
        "entity leaf is port (x : in bit := '0'); end entity;
         architecture rtl of leaf is begin end architecture;
         entity top is end entity;
         architecture a of top is
           component leaf is port (x : in bit := '0'); end component;
         begin
           b : block
             generic (w : integer);
             generic map (w => 2);
             port (p : in bit := '1');
             port map (p => open);
           begin
             u0 : leaf;
           end block;
           g : for i in 0 to 3 generate
             u1 : leaf;
           end generate;
         end architecture;
         configuration cfg of top is
           for a
             for b
               for u0 : leaf use entity work.leaf(rtl); end for;
             end for;
             for g(2)
               for all : leaf
                 use entity work.leaf(rtl);
                 for rtl end for;
               end for;
             end for;
           end for;
         end configuration;
         entity tb is end entity;
         architecture t of tb is
         begin
           dut : configuration work.cfg;
         end architecture;",
    );
    r.clean();
    let UnitHir::Configuration(bc) = &r.unit(UnitKind::Configuration, "cfg").hir else {
        panic!("configuration HIR");
    };
    assert!(matches!(bc.target, BlockTarget::Architecture(_)));
    assert_eq!(bc.items.len(), 2);
    assert!(
        matches!(&bc.items[0], ConfigItem::Block(b) if matches!(b.target, BlockTarget::Block(_)))
    );
    let ConfigItem::Block(g) = &bc.items[1] else {
        panic!("generate block")
    };
    assert!(matches!(
        g.target,
        BlockTarget::Generate {
            select: Some(GenSelect::Index(_)),
            ..
        }
    ));
    let ConfigItem::Component(c) = &g.items[0] else {
        panic!("component configuration")
    };
    assert!(matches!(c.instances, InstanceSel::All));
    assert!(c.block.is_some());
    let tb = r.arch_stmts("t")[0];
    assert!(matches!(
        r.db.stmts[tb].kind,
        StmtKind::Instance {
            target: InstTarget::Configuration(_),
            ..
        }
    ));
}

#[test]
fn configuration_errors() {
    let r = one(
        "config_err",
        "entity top is end entity;
         architecture a of top is
           component c is end component;
           signal s : bit;
         begin
           u : c;
         end architecture;
         configuration c1 of top is
           for a
             for s : c end for;
             for nope end for;
           end for;
         end configuration;",
    );
    assert_eq!(r.codes(), vec![("E1701", 10), ("E1701", 11)]);
}

#[test]
fn instance_target_errors() {
    let r = one(
        "inst_err",
        "package p is end package;
         entity top is end entity;
         architecture a of top is
           signal s : bit;
         begin
           u1 : entity work.p;
           u2 : configuration work.top;
           u3 : s;
         end architecture;",
    );
    assert_eq!(r.codes(), vec![("E1008", 6), ("E1008", 7), ("E1700", 8)]);
}

#[test]
fn ports_and_generics_class_and_mode() {
    let r = one(
        "iface",
        "entity e is
           generic (constant g : out integer := 1);
           port (variable v : in integer);
         end entity;",
    );
    assert_eq!(r.codes(), vec![("E1360", 2), ("E1360", 3)]);
}

#[test]
fn architecture_of_an_entity_in_another_library() {
    let r = run(
        "arch_other_lib",
        &[
            ("other", "entity e is end entity;"),
            (
                "work",
                "library other;\narchitecture a of other.e is begin end architecture;",
            ),
        ],
    );
    assert_eq!(r.codes(), vec![("E1004", 2)]);
}
