//! Elaboration tests on real VHDL, analyzed from a per-process temporary directory.

use yavs_kernel::{Elem, PortMode, Range};
use yavs_sem::{Db, LangRev, Options};

use crate::{Case, ElabConfig, Elaborated, ScopeKind, Sel, codes};

fn analyze(name: &str, src: &str) -> Db {
    analyze_rev(name, src, LangRev::Vhdl2008)
}

fn analyze_rev(_name: &str, src: &str, rev: LangRev) -> Db {
    let tmp = tempfile::tempdir().expect("scratch dir");
    let p = tmp.path().join("t.vhd");
    std::fs::write(&p, src).expect("write");
    let mut db = Db::new(Options::new(rev, yavs_sem::default_stdlib()));
    let r = db.analyze_file("work", &p, rev);
    let msgs: Vec<String> = db
        .diagnostics()
        .iter()
        .map(|d| format!("{} {}", d.code, d.message))
        .collect();
    assert!(r.is_ok() && msgs.is_empty(), "analysis: {msgs:#?}");
    db
}

type Errs = Vec<(&'static str, String)>;

/// Ports kept apart: the tests of the uncollapsed net graph (plan rule 3).
const UNCOLLAPSED: ElabConfig = ElabConfig {
    max_depth: 1000,
    collapse_ports: false,
};

fn try_elab(
    name: &str,
    src: &str,
    top: &str,
    gens: &[(&str, &str)],
) -> (Db, Result<Elaborated, Errs>) {
    try_elab_cfg(name, src, top, gens, &ElabConfig::default())
}

fn try_elab_cfg(
    name: &str,
    src: &str,
    top: &str,
    gens: &[(&str, &str)],
    cfg: &ElabConfig,
) -> (Db, Result<Elaborated, Errs>) {
    let mut db = analyze(name, src);
    let gens: Vec<(String, String)> = gens
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
    let r = crate::find_top(&mut db, "work", top)
        .and_then(|t| crate::elaborate(&mut db, t, &gens, cfg))
        .map_err(|v| v.into_iter().map(|d| (d.code, d.message)).collect());
    (db, r)
}

fn elab(name: &str, src: &str, top: &str, gens: &[(&str, &str)]) -> (Db, Elaborated) {
    let (db, r) = try_elab(name, src, top, gens);
    match r {
        Ok(e) => (db, e),
        Err(v) => panic!("elaboration failed: {v:#?}"),
    }
}

fn elab_ports(name: &str, src: &str, top: &str, gens: &[(&str, &str)]) -> (Db, Elaborated) {
    match try_elab_cfg(name, src, top, gens, &UNCOLLAPSED) {
        (db, Ok(e)) => (db, e),
        (_, Err(v)) => panic!("elaboration failed: {v:#?}"),
    }
}

fn err_codes(name: &str, src: &str, top: &str, gens: &[(&str, &str)]) -> Vec<&'static str> {
    match try_elab(name, src, top, gens).1 {
        Ok(_) => panic!("elaboration succeeded"),
        Err(v) => v.into_iter().map(|(c, _)| c).collect(),
    }
}

/// Every process path, in elaboration order.
fn procs(e: &Elaborated) -> Vec<String> {
    e.procs
        .iter()
        .map(|p| e.hier.path_name(p.scope, Case::Lrm))
        .collect()
}

fn sig_paths(e: &Elaborated) -> Vec<String> {
    e.nets
        .objs
        .iter()
        .map(|o| format!("{}{}", e.hier.path_name(o.scope, Case::Lrm), o.name.key))
        .collect()
}

const PATHS: &str = "
entity Leaf is end;
architecture A of leaf is
  signal S : bit;
begin
  process begin wait; end process;
end;
entity instance_path_names is end;
architecture t of instance_path_names is
begin
  A1 : entity work.leaf;
  g : for i in 0 to 1 generate b : entity work.leaf; end generate;
end;
";

#[test]
fn path_and_instance_names() {
    let (db, e) = elab("paths", PATHS, "instance_path_names", &[]);
    assert_eq!(
        procs(&e),
        [
            ":instance_path_names:a1:_p0:",
            ":instance_path_names:g(0):b:_p0:",
            ":instance_path_names:g(1):b:_p0:",
        ]
    );
    let p = e.procs[1].scope;
    let inst = e.hier.scope(p).parent.expect("parent");
    assert_eq!(
        e.hier.instance_name(inst, &db, Case::Lrm),
        ":instance_path_names(t):g(0):b@leaf(a):"
    );
    // Original spelling for VHPI and waves (rule 8).
    let a1 = e.hier.scope(e.procs[0].scope).parent.expect("parent");
    assert_eq!(
        e.hier.path_name(a1, Case::Original),
        ":instance_path_names:A1:"
    );
    assert_eq!(
        e.hier.instance_name(a1, &db, Case::Original),
        ":instance_path_names(t):A1@Leaf(A):"
    );
    assert_eq!(e.nets.objs.len(), 3);
    assert_eq!(e.leaf_name(crate::LeafId(0)), ":instance_path_names:A1:S");
}

const GENERICS: &str = "
entity child is
  generic (W : natural; NAME : string := \"none\");
  port (i : in bit_vector(W - 1 downto 0));
end;
architecture a of child is
  constant D : natural := W * 2;
  signal s : bit_vector(D - 1 downto 0);
begin
  g : for k in 1 to W generate
    process begin wait; end process;
  end generate;
  big : if W > 2 generate
    process begin wait; end process;
  else small : generate
  end generate;
end;
entity gtop is
  generic (N : positive := 2; T : time := 1 ns; MSG : string := \"x\");
end;
architecture t of gtop is
  signal v : bit_vector(N - 1 downto 0);
  signal w : bit_vector(2 * N - 1 downto 0);
begin
  c1 : entity work.child generic map (W => N) port map (i => v);
  c2 : entity work.child generic map (W => 2 * N, NAME => MSG) port map (i => w);
end;
";

#[test]
fn generics_defaults_maps_and_generates() {
    let (_, e) = elab_ports("generics", GENERICS, "gtop", &[]);
    let c1 = e.hier.scope(crate::ScopeId(0)).children[0];
    let c2 = e.hier.scope(crate::ScopeId(0)).children[1];
    // c1: W = 2, so 2 for-generate processes, the else branch of `big`.
    assert_eq!(e.hier.scope(c1).children.len(), 3);
    // c2: W = 4, 4 processes plus `big`.
    assert_eq!(e.hier.scope(c2).children.len(), 5);
    assert_eq!(
        procs(&e),
        [
            ":gtop:c1:g(1):_p0:",
            ":gtop:c1:g(2):_p0:",
            ":gtop:c2:g(1):_p0:",
            ":gtop:c2:g(2):_p0:",
            ":gtop:c2:g(3):_p0:",
            ":gtop:c2:g(4):_p0:",
            ":gtop:c2:big:_p0:",
        ]
    );
    // Constant D = 2 * W sizes `s`: 4 and 8 elements.
    let lens: Vec<u32> = e.nets.leaves.iter().map(|l| l.ty.len).collect();
    assert_eq!(lens, [2, 4, 2, 4, 4, 8]);
    assert!(e.nets.leaves.iter().all(|l| l.ty.elem == Elem::Logic));
    // Ports are uncollapsed: one edge per formal.
    assert_eq!(e.nets.ports.len(), 2);
    assert_eq!(e.nets.ports[0].mode, PortMode::In);
}

#[test]
fn generic_overrides_use_the_value_parser() {
    let (_, e) = elab(
        "overrides",
        GENERICS,
        "gtop",
        &[
            ("n", "3"),
            ("T", "2.5 ns"),
            ("msg", "active python runner : a,b/c\\d.e"),
        ],
    );
    let vals = &e.model_vals[0];
    assert_eq!(e.nets.leaves[0].ty.len, 3);
    // A string generic is the raw text, verbatim (cli-contract section 5).
    let msg = vals
        .values()
        .find_map(|v| match v {
            crate::Val::Array(a) => match &a.elems {
                yavs_sem::stat::Elems::Bytes(b) => {
                    Some(b.iter().map(|&c| c as char).collect::<String>())
                }
                _ => None,
            },
            _ => None,
        })
        .expect("string generic");
    assert_eq!(msg, "active python runner : a,b/c\\d.e");
    assert!(vals.values().any(|v| *v == crate::Val::Phys(2_500_000)));
    assert!(e.warnings.is_empty());

    let (_, e) = elab("overrides2", GENERICS, "gtop", &[("nope", "1")]);
    assert_eq!(e.warnings[0].code, codes::UNKNOWN_OVERRIDE);
    assert_eq!(
        err_codes("overrides3", GENERICS, "gtop", &[("N", "two")]),
        [codes::BAD_OVERRIDE]
    );
    // positive: 0 is out of range.
    assert_eq!(
        err_codes("overrides4", GENERICS, "gtop", &[("N", "0")]),
        [codes::BAD_OVERRIDE]
    );
}

#[test]
fn missing_generic_value() {
    let src = "entity c is generic (W : natural); end; architecture a of c is begin end;
               entity t is end; architecture a of t is begin u : entity work.c; end;";
    // Analysis already rejects the instance without W; the top level itself needs -g.
    assert_eq!(
        err_codes(
            "nogeneric",
            &src[..src.find("entity t").expect("t")],
            "c",
            &[]
        ),
        [codes::GENERIC_NO_VALUE]
    );
    let (_, e) = elab(
        "nogeneric2",
        &src[..src.find("entity t").expect("t")],
        "c",
        &[("w", "7")],
    );
    assert!(e.procs.is_empty());
}

const BINDING: &str = "
entity inv is port (a : in bit; y : out bit); end;
architecture rtl of inv is begin y <= not a; end;
architecture other of inv is begin y <= a; end;
entity btop is end;
architecture t of btop is
  component inv is port (a : in bit; y : out bit); end component;
  signal x, y1, y2, y3 : bit;
  for u2 : inv use entity work.inv(rtl);
begin
  u1 : inv port map (x, y1);
  u2 : inv port map (a => x, y => y2);
  u3 : component inv port map (x, y3);
end;
configuration cfg of btop is
  for t
    for u1 : inv use entity work.inv(rtl); end for;
  end for;
end;
";

fn arch_of(db: &Db, e: &Elaborated, label: &str) -> String {
    let s = e
        .hier
        .scopes
        .iter()
        .find(|s| s.name.key.as_str() == label && matches!(s.kind, ScopeKind::Instance { .. }))
        .expect("instance");
    let ScopeKind::Instance { arch, .. } = s.kind else {
        unreachable!()
    };
    db.units[arch].name.key.to_string()
}

#[test]
fn component_binding_default_spec_and_configuration() {
    let (db, e) = elab_ports("binding", BINDING, "btop", &[]);
    // Default binding: the most recently analyzed architecture.
    assert_eq!(arch_of(&db, &e, "u1"), "other");
    // Configuration specification.
    assert_eq!(arch_of(&db, &e, "u2"), "rtl");
    assert_eq!(arch_of(&db, &e, "u3"), "other");
    assert_eq!(
        procs(&e),
        [":btop:u1:_p0:", ":btop:u2:_p0:", ":btop:u3:_p0:"]
    );
    // Component ports between the actual and the entity's formal: two edges per port.
    assert_eq!(e.nets.ports.len(), 12);

    let (db, e) = elab_ports("binding_cfg", BINDING, "cfg", &[]);
    assert!(e.top.config.is_some());
    assert_eq!(arch_of(&db, &e, "u1"), "rtl");
    assert_eq!(arch_of(&db, &e, "u2"), "rtl");

    let (db, e) = elab_ports("binding_arch", BINDING, "inv.rtl", &[]);
    assert_eq!(db.units[e.top.arch].name.key.as_str(), "rtl");
}

#[test]
fn unbound_component_is_a_warning() {
    let src = "entity t is end; architecture a of t is
      component nothere is port (a : in bit); end component;
      signal s : bit;
    begin u : nothere port map (s); end;";
    let (_, e) = elab("unbound", src, "t", &[]);
    assert_eq!(e.warnings[0].code, codes::UNBOUND);
}

#[test]
fn ports_take_unconstrained_bounds_from_the_actual() {
    let src = "entity c is port (i : in bit_vector; o : out bit_vector); end;
      architecture a of c is begin end;
      entity t is end; architecture a of t is
        signal s : bit_vector(7 downto 4);
        signal r : bit_vector(0 to 9);
      begin u : entity work.c port map (i => s(6 downto 5), o => r(2 to 3)); end;";
    let (_, e) = elab("unconstrained", src, "t", &[]);
    let i = &e.nets.objs[2];
    assert_eq!(i.name.key.as_str(), "i");
    let crate::Shape::Array { dims, .. } = &i.shape else {
        panic!("array")
    };
    assert_eq!((dims[0].left, dims[0].right), (6, 5));
    // s(6 downto 5): row-major positions 1..3 of 4; logic leaves are mirrored.
    assert_eq!(e.nets.ports[0].actual, (crate::LeafId(0), Range::new(1, 2)));
    // r(2 to 3): positions 2..4 of 10, mirrored to 6..8.
    assert_eq!(e.nets.ports[1].actual, (crate::LeafId(1), Range::new(6, 2)));
    assert_eq!(e.nets.ports[1].mode, PortMode::Out);
}

/// An open or unassociated unconstrained `in` port with a default takes the default's bounds
/// (#218). Without a default sem rejects it first (E1311, tests/diag/sem/ovl).
#[test]
fn open_unconstrained_in_port_takes_the_default_bounds() {
    let sub = "entity c is port (i : in bit_vector := \"101\"); end;
      architecture a of c is begin end;
      entity t is end; architecture a of t is begin ";
    for map in [
        "u : entity work.c port map (i => open); end;",
        "u : entity work.c; end;",
    ] {
        let (_, e) = elab("open_default", &format!("{sub}{map}"), "t", &[]);
        let i = &e.nets.objs[0];
        let crate::Shape::Array { dims, .. } = &i.shape else {
            panic!("array")
        };
        assert_eq!((dims[0].left, dims[0].right), (0, 2));
    }
}

#[test]
fn port_length_mismatch() {
    let src = "entity c is port (i : in bit_vector(3 downto 0)); end;
      architecture a of c is begin end;
      entity t is end; architecture a of t is
        signal s : bit_vector(7 downto 0);
      begin u : entity work.c port map (i => s); end;";
    let (_, r) = try_elab("mismatch", src, "t", &[]);
    let v = r.err().expect("error");
    assert!(v.iter().any(|(c, _)| *c == codes::PORT_MISMATCH), "{v:?}");
}

#[test]
fn unresolved_signal_with_two_drivers() {
    // Sem checks the statements of a region (E1812); elaboration sees every generate iteration.
    let src = "entity t is end; architecture a of t is
        signal s : integer;
      begin
        g : for i in 0 to 1 generate s <= i; end generate;
      end;";
    assert_eq!(
        err_codes("drivers", src, "t", &[]),
        [codes::MULTIPLE_SOURCES]
    );
    // Distinct elements of one unresolved vector are fine; one process may drive overlapping
    // names (one driver per scalar).
    let src = "entity t is end; architecture a of t is
        signal s : bit_vector(0 to 3);
      begin
        s(0) <= '1';
        process begin s(1 to 3) <= \"000\"; s(2) <= '1'; wait; end process;
      end;";
    let (_, e) = elab("drivers_ok", src, "t", &[]);
    assert_eq!(e.nets.drivers.len(), 2);
    assert_eq!(e.procs[1].lsps.len(), 2);
    // s(1 to 3): positions 1..4 of 4, mirrored to 0..3.
    assert_eq!(e.nets.drivers[1].range, Range::new(0, 3));
}

#[test]
fn std_logic_is_resolved_planes() {
    let src = "library ieee; use ieee.std_logic_1164.all;
      entity t is end; architecture a of t is
        signal s : std_logic_vector(3 downto 0) := \"01ZX\";
        signal u : std_ulogic;
      begin
        s <= \"ZZZZ\";
        s <= \"LLLL\";
      end;";
    let (_, e) = elab("stdlogic", src, "t", &[]);
    assert!(matches!(
        e.nets.leaves[0].resolution,
        Some(crate::LeafRes::StdLogic)
    ));
    assert!(e.nets.leaves[1].resolution.is_none());
    let yavs_kernel::Value::Logic(v) = &e.nets.leaves[0].init else {
        panic!("logic")
    };
    assert_eq!(v.to_string(), "01ZX");
}

#[test]
fn records_and_sub_element_map() {
    let src = "entity t is end; architecture a of t is
        type rec is record a : integer; b : bit_vector(0 to 1); end record;
        type arr is array (0 to 2) of rec;
        signal r : arr;
      begin end;";
    let (_, e) = elab("records", src, "t", &[]);
    // Struct of arrays: one leaf per field.
    let lens: Vec<(u32, Elem)> = e
        .nets
        .leaves
        .iter()
        .map(|l| (l.ty.len, l.ty.elem))
        .collect();
    assert_eq!(lens, [(3, Elem::Bytes(8)), (6, Elem::Logic)]);
    let o = crate::ObjId(0);
    let (r, _) = e.nets.select(o, &[Sel::Index(vec![1])]).expect("r(1)");
    assert_eq!(
        r,
        [
            (crate::LeafId(0), Range::new(1, 1)),
            (crate::LeafId(1), Range::new(2, 2))
        ]
    );
    let (r, _) = e
        .nets
        .select(
            o,
            &[Sel::Index(vec![2]), Sel::Field(1), Sel::Index(vec![0])],
        )
        .expect("r(2).b(0)");
    assert_eq!(r, [(crate::LeafId(1), Range::new(1, 1))]);
    let (r, sh) = e
        .nets
        .select(
            o,
            &[Sel::Slice(crate::Bounds {
                left: 1,
                dir: yavs_sem::hir::Direction::To,
                right: 2,
            })],
        )
        .expect("r(1 to 2)");
    assert_eq!(
        r,
        [
            (crate::LeafId(0), Range::new(1, 2)),
            (crate::LeafId(1), Range::new(0, 4))
        ]
    );
    assert_eq!(sh.len(), 2);
    assert!(e.nets.select(o, &[Sel::Index(vec![3])]).is_err());
}

#[test]
fn package_constant_calling_its_own_function() {
    let src = "package p is
        function f return integer;
        constant c : integer := f;
      end;
      package body p is function f return integer is begin return 1; end; end;
      use work.p.all;
      entity t is end; architecture a of t is
        signal s : bit_vector(1 to c);
      begin end;";
    let (_, r) = try_elab("body_order", src, "t", &[]);
    let v = r.err().expect("error");
    assert_eq!(
        v.last().map(|x| x.0),
        Some(codes::BODY_NOT_ELABORATED),
        "{v:?}"
    );
}

#[test]
fn top_not_found_and_no_architecture() {
    assert_eq!(
        err_codes("top1", PATHS, "nothing", &[]).last(),
        Some(&codes::TOP_NOT_FOUND)
    );
    assert_eq!(
        err_codes("top2", "entity e is end;", "e", &[]),
        [codes::NO_ARCHITECTURE]
    );
}

struct Idle;
impl yavs_kernel::Process for Idle {
    fn run(&mut self, ctx: &mut yavs_kernel::Ctx<'_>) -> Result<(), yavs_kernel::SimError> {
        ctx.wait_for(yavs_kernel::Time::ns(1))
    }
}

/// Rule 4 (GHDL #1184): elaborating twice in one process, and building twice from one
/// elaboration, give identical results.
#[test]
fn restart_is_identical() {
    let run = |e: &Elaborated| {
        let mut sim = e
            .build_with(yavs_kernel::Config::default(), |_, _| Ok(Box::new(Idle)))
            .expect("build");
        sim.run_for(yavs_kernel::Time::ns(10)).expect("run");
        (0..sim.signal_count())
            .map(|i| {
                let s = yavs_kernel::SignalId(i as u32);
                format!(
                    "{} {}",
                    sim.signal_name(s).expect("name"),
                    sim.value(s).expect("value")
                )
            })
            .collect::<Vec<_>>()
    };
    for collapse in [false, true] {
        let cfg = ElabConfig {
            collapse_ports: collapse,
            ..ElabConfig::default()
        };
        let e = || match try_elab_cfg("restart", BINDING, "btop", &[], &cfg) {
            (_, Ok(e)) => e,
            (_, Err(v)) => panic!("elaboration failed: {v:#?}"),
        };
        let (a, b) = (e(), e());
        assert_eq!(procs(&a), procs(&b));
        assert_eq!(sig_paths(&a), sig_paths(&b));
        assert_eq!(a.nets.ports, b.nets.ports);
        let first = run(&a);
        assert_eq!(first, run(&a));
        assert_eq!(first, run(&b));
        // A collapsed `in` port is no kernel signal of its own.
        assert_eq!(
            first.iter().any(|s| s.starts_with(":btop:u1:a")),
            !collapse,
            "{first:?}"
        );
    }
}

struct Collect(std::sync::Arc<std::sync::Mutex<Vec<String>>>);
impl yavs_kernel::Observer for Collect {
    fn report(&mut self, hook: &mut yavs_kernel::Hook<'_>, r: &yavs_kernel::Report) {
        self.0.lock().expect("lock").push(format!(
            "{} {} {}",
            hook.now().as_fs(),
            r.process.map_or(u32::MAX, |p| p.0),
            r.message
        ));
    }
}

/// Processes through yavs-lower and yavs-interp; building twice restarts the simulation.
#[test]
fn interpreted_processes_restart() {
    let src = "entity t is end; architecture a of t is
        signal s : integer := 3;
      begin
        s <= 5 after 2 ns;
        mon : process (s) begin report \"s=\" & integer'image(s); end process;
      end;";
    let (mut db, mut e) = elab("interp", src, "t", &[]);
    e.lower(&mut db).expect("lower");
    let run = |e: &Elaborated| {
        let mut sim = e.build(yavs_kernel::Config::default()).expect("build");
        let got = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        sim.subscribe(Box::new(Collect(got.clone())));
        sim.run_until(yavs_kernel::Time::MAX).expect("run");
        got.lock().expect("lock").clone()
    };
    let first = run(&e);
    assert_eq!(first, ["0 1 s=3", "2000000 1 s=5"]);
    assert_eq!(first, run(&e));
}

/// Impure calls during elaboration run against package state in order (#77, LRM 14.4), and
/// every build starts from that state (#383), or replays the calls on fresh packages where it
/// cannot (coverage counters), so the processes see their side effects after a restart too.
#[test]
fn elaboration_calls_with_package_state_restart() {
    let src = "package reg is
        type counter_t is protected
          impure function bump return natural;
          impure function count return natural;
        end protected;
        impure function next_id return natural;
        impure function ids return natural;
      end;
      package body reg is
        type counter_t is protected body
          variable n : natural := 0;
          impure function bump return natural is begin n := n + 1; return n; end;
          impure function count return natural is begin return n; end;
        end protected body;
        shared variable c : counter_t;
        impure function next_id return natural is begin return c.bump; end;
        impure function ids return natural is begin return c.count; end;
      end;
      use work.reg.all;
      entity child is generic (id : natural); end;
      architecture a of child is
        signal v : bit_vector(1 to id);
      begin
        process begin report \"id=\" & integer'image(v'length); wait; end process;
      end;
      use work.reg.all;
      entity t is end; architecture a of t is
        constant first : natural := next_id;
      begin
        u : entity work.child generic map (id => next_id);
        process begin report \"ids=\" & integer'image(ids); wait; end process;
      end;";
    let (mut db, mut e) = elab("pkgcalls", src, "t", &[]);
    e.lower(&mut db).expect("lower");
    let run = |e: &Elaborated| {
        let mut sim = e.build(yavs_kernel::Config::default()).expect("build");
        let got = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        sim.subscribe(Box::new(Collect(got.clone())));
        sim.run_until(yavs_kernel::Time::MAX).expect("run");
        let mut v = got.lock().expect("lock").clone();
        v.sort();
        v
    };
    let first = run(&e);
    assert_eq!(first, ["0 0 id=2", "0 1 ids=2"]);
    assert_eq!(first, run(&e));
    assert!(e.ir.as_ref().is_some_and(|ir| ir.start.is_some()));
    let (mut db, mut e) = elab("pkgcalls_cov", src, "t", &[]);
    e.lower_with(
        &mut db,
        yavs_lower::LowerOptions {
            coverage: true,
            ..yavs_lower::LowerOptions::default()
        },
    )
    .expect("lower");
    assert!(e.ir.as_ref().is_some_and(|ir| ir.start.is_none()));
    assert_eq!(first, run(&e));
    assert_eq!(first, run(&e));
}

/// A constant whose impure initial value is a record with an array element is evaluated
/// during elaboration like the others (#220): its call runs once and in declaration order.
#[test]
fn impure_record_with_array_in_declaration_order() {
    let src = "package reg is
        type counter_t is protected
          impure function bump return natural;
          impure function count return natural;
        end protected;
        type named_t is record s : string(1 to 2); id : natural; end record;
        impure function next_id return natural;
        impure function new_named return named_t;
        impure function ids return natural;
      end;
      package body reg is
        type counter_t is protected body
          variable n : natural := 0;
          impure function bump return natural is begin n := n + 1; return n; end;
          impure function count return natural is begin return n; end;
        end protected body;
        shared variable c : counter_t;
        impure function next_id return natural is begin return c.bump; end;
        impure function new_named return named_t is begin return (\"ab\", c.bump); end;
        impure function ids return natural is begin return c.count; end;
      end;
      use work.reg.all;
      entity t is end; architecture a of t is
        constant first : natural := next_id;
        constant named : named_t := new_named;
        constant third : natural := next_id;
      begin
        process begin
          report \"first=\" & integer'image(first) & \" named=\" & integer'image(named.id)
            & \" third=\" & integer'image(third) & \" ids=\" & integer'image(ids);
          wait;
        end process;
      end;";
    let (mut db, mut e) = elab("unfoldable", src, "t", &[]);
    e.lower(&mut db).expect("lower");
    let mut sim = e.build(yavs_kernel::Config::default()).expect("build");
    let got = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    sim.subscribe(Box::new(Collect(got.clone())));
    sim.run_until(yavs_kernel::Time::MAX).expect("run");
    assert_eq!(
        *got.lock().expect("lock"),
        ["0 0 first=1 named=2 third=3 ids=3"]
    );
}

/// An impure initial value elaboration cannot evaluate yet (`'PATH_NAME` of an object)
/// is left to its run-time thunk with W2118: the call made while trying is undone, so it runs
/// once, after the elaboration-time calls.
#[test]
fn impure_constant_at_run_time_warns() {
    let src = "package reg is
        type counter_t is protected
          impure function bump return natural;
        end protected;
        impure function next_id return natural;
        impure function probe(signal s : bit) return natural;
      end;
      package body reg is
        type counter_t is protected body
          variable n : natural := 0;
          impure function bump return natural is begin n := n + 1; return n; end;
        end protected body;
        shared variable c : counter_t;
        impure function next_id return natural is begin return c.bump; end;
        impure function probe(signal s : bit) return natural is
        begin return c.bump + bit'pos(s); end;
      end;
      use work.reg.all;
      entity t is end; architecture a of t is
        signal x : bit := '1';
        constant first : natural := next_id;
        constant late : natural := next_id + x'path_name'length;
        constant third : natural := next_id;
      begin
        process begin
          report \"first=\" & integer'image(first) & \" late=\" & integer'image(late)
            & \" third=\" & integer'image(third);
          wait;
        end process;
      end;";
    let (mut db, mut e) = elab("impure_late", src, "t", &[]);
    let w: Vec<&str> = e.warnings.iter().map(|d| d.code).collect();
    assert_eq!(w, [codes::IMPURE_AT_RUN_TIME]);
    e.lower(&mut db).expect("lower");
    let mut sim = e.build(yavs_kernel::Config::default()).expect("build");
    let got = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    sim.subscribe(Box::new(Collect(got.clone())));
    sim.run_until(yavs_kernel::Time::MAX).expect("run");
    assert_eq!(*got.lock().expect("lock"), ["0 0 first=1 late=7 third=2"]);
}

/// BIT and BOOLEAN leaves store '0'/'1' as the std_ulogic codes 2/3 (docs/decisions.md).
#[test]
fn bit_and_boolean_use_std_ulogic_codes() {
    let src = "entity t is end; architecture a of t is
        signal v : bit_vector(0 to 2) := \"011\";
        signal b : boolean := true;
      begin end;";
    let (_, e) = elab("bitcodes", src, "t", &[]);
    let yavs_kernel::Value::Logic(v) = &e.nets.leaves[0].init else {
        panic!("logic")
    };
    // Rightmost element first: '1', '1', '0' as codes 3, 3, 2.
    let codes: Vec<u8> = (0..3).map(|i| v.get(i) as u8).collect();
    assert_eq!(codes, [3, 3, 2]);
    let yavs_kernel::Value::Logic(b) = &e.nets.leaves[1].init else {
        panic!("logic")
    };
    assert_eq!(b.get(0) as u8, 3);
}

#[test]
fn guarded_blocks_and_bus_signals() {
    let src = "library ieee; use ieee.std_logic_1164.all;
      entity t is end; architecture a of t is
        signal en : boolean := false;
        signal b : std_logic bus;
      begin
        g1 : block (en) begin b <= guarded '1'; end block;
      end;";
    let (_, e) = elab("guarded", src, "t", &[]);
    assert_eq!(procs(&e), [":t:g1:_guard:", ":t:g1:_p0:"]);
    let b = e
        .nets
        .leaves
        .iter()
        .find(|l| l.resolution.is_some())
        .expect("b");
    assert_eq!(b.guard, Some(yavs_kernel::GuardKind::Bus));
    // The guard process drives the block's GUARD signal.
    let gd = &e.nets.drivers[e.procs[0].drivers[0]];
    let obj = e.nets.obj(e.nets.leaf(gd.leaf).obj);
    assert_eq!(obj.name.key.as_str(), "guard");
}

/// #77: user functions called during elaboration run in the interpreter.
#[test]
fn elaboration_calls_user_functions() {
    let src = "package util is
        function clog2(n : positive) return natural;
      end;
      package body util is
        function clog2(n : positive) return natural is
          variable r : natural := 0;
          variable v : natural := 1;
        begin
          while v < n loop v := v * 2; r := r + 1; end loop;
          return r;
        end;
      end;
      use work.util.all;
      entity t is generic (N : positive := 100); end;
      architecture a of t is
        function twice(x : integer) return integer is begin return 2 * x; end;
        constant W : natural := clog2(N);
        signal s : bit_vector(twice(W) - 1 downto 0);
      begin
        g : for i in 1 to clog2(N + 1) generate end generate;
      end;";
    let (_, e) = elab("calls", src, "t", &[]);
    assert_eq!(e.nets.leaves[0].ty.len, 14);
    assert_eq!(e.hier.scope(crate::ScopeId(0)).children.len(), 7);
    let (_, e) = elab("calls2", src, "t", &[("N", "1000")]);
    assert_eq!(e.nets.leaves[0].ty.len, 20);
}

/// A package whose subprograms need a body that does not exist.
#[test]
fn package_body_missing() {
    let src = "package p is function f return integer; end;
      use work.p.all;
      entity t is end; architecture a of t is begin end;";
    assert_eq!(err_codes("nobody", src, "t", &[]), [codes::BODY_MISSING]);
}

/// ieee function calls fold at elaboration too (math_real from source).
#[test]
fn elaboration_calls_math_real() {
    let src = "library ieee; use ieee.math_real.all;
      entity t is generic (N : positive := 100); end;
      architecture a of t is
        constant W : natural := natural(ceil(log2(real(N))));
        signal s : bit_vector(W - 1 downto 0);
      begin end;";
    let (_, e) = elab("mathreal", src, "t", &[]);
    assert_eq!(e.nets.leaves[0].ty.len, 7);
}

/// Implicit signals are kernel signals over their prefix's elements (#76). Sem does not
/// create them from names yet, so the test declares them with `attr::implicit_signal`.
#[test]
fn implicit_signals() {
    use yavs_sem::hir::{Expr, ExprKind, PredefAttr, Staticness};
    let src = "entity t is end; architecture a of t is
        signal s : bit_vector(0 to 3);
      begin end;";
    let mut db = analyze("implicit", src);
    let arch = db
        .units
        .iter()
        .find(|(_, u)| matches!(u.hir, yavs_sem::hir::UnitHir::Architecture { .. }))
        .map(|(_, u)| u.scope)
        .expect("arch");
    let s = db.scopes[arch].order[0];
    let yavs_sem::decl::DeclKind::Object(o) = &db.decls[s].kind else {
        panic!("signal")
    };
    let (st, span) = (o.subtype, db.decls[s].span);
    let prefix = db.exprs.push(Expr {
        kind: ExprKind::Obj(s),
        subtype: st,
        span,
        stat: Staticness::Locally,
        value: None,
    });
    let time = db.types[db.well_known.time.expect("time")].first_subtype;
    let c = db.consts.push(yavs_sem::stat::ConstValue::Phys(2_000_000));
    let two_ns = db.exprs.push(Expr {
        kind: ExprKind::Const(c),
        subtype: time,
        span,
        stat: Staticness::Locally,
        value: Some(c),
    });
    let boolean = db.types[db.well_known.boolean.expect("boolean")].first_subtype;
    for (attr, param, sub) in [
        (PredefAttr::Stable, Some(two_ns), boolean),
        (PredefAttr::Transaction, None, boolean),
        (PredefAttr::Delayed, Some(two_ns), st),
    ] {
        yavs_sem::attr::implicit_signal(&mut db, attr, false, arch, prefix, param, sub, span)
            .expect("implicit signal");
    }
    let top = crate::find_top(&mut db, "work", "t").expect("top");
    let e = crate::elaborate(&mut db, top, &[], &ElabConfig::default()).expect("elab");
    let kinds: Vec<_> = e.nets.leaves.iter().filter_map(|l| l.implicit).collect();
    assert_eq!(
        kinds,
        [
            (
                crate::ImplicitKind::Stable(yavs_kernel::Time::ns(2)),
                crate::LeafId(0),
                Range::new(0, 4)
            ),
            (
                crate::ImplicitKind::Transaction,
                crate::LeafId(0),
                Range::new(0, 4)
            ),
            (
                crate::ImplicitKind::Delayed(yavs_kernel::Time::ns(2)),
                crate::LeafId(0),
                Range::new(0, 4)
            ),
        ]
    );
    let sim = e
        .build_with(yavs_kernel::Config::default(), |_, _| Ok(Box::new(Idle)))
        .expect("build");
    let names: Vec<String> = (0..sim.signal_count())
        .map(|i| {
            sim.signal_name(yavs_kernel::SignalId(i as u32))
                .expect("name")
                .to_string()
        })
        .collect();
    assert_eq!(names.len(), 4);
    assert!(names[1].contains("'stable"), "{names:?}");
    assert!(names[3].contains("'delayed"), "{names:?}");
}

/// Signals declared in a package are elaborated once, on first use, as `:lib:pkg:s` (#76).
#[test]
fn package_signals() {
    let src = "package p is signal pk : integer := 7; end;
      use work.p.all;
      entity c is port (i : in integer); end; architecture a of c is begin end;
      use work.p.all;
      entity t is end; architecture a of t is
      begin
        pk <= 1;
        u1 : entity work.c port map (pk);
        u2 : entity work.c port map (i => pk);
      end;";
    let (_, e) = elab_ports("pkgsig", src, "t", &[]);
    assert_eq!(sig_paths(&e), [":work:p:pk", ":t:u1:i", ":t:u2:i"]);
    assert_eq!(e.nets.ports[0].actual.0, e.nets.ports[1].actual.0);
    let yavs_kernel::Value::Bytes(b) = &e.nets.leaves[0].init else {
        panic!("bytes")
    };
    assert_eq!(b[0], 7);
}

/// External names bind against the elaborated hierarchy (#76, GHDL #548). Sem does not
/// analyze them in port maps yet, so the test binds HIR externals directly.
#[test]
fn external_names() {
    use yavs_syntax::ast::{ExternalClass, PathKind};
    let src = "entity d is end; architecture a of d is signal s : integer := 4; begin end;
      entity t is end; architecture a of t is
      begin
        dut : entity work.d;
        g : for k in 0 to 1 generate inner : entity work.d; end generate;
      end;";
    let (mut db, e) = elab("external", src, "t", &[]);
    let id = |db: &mut Db, kind, names: &[(&str, Option<i64>)]| {
        let mut path = Vec::new();
        for (n, ix) in names {
            let sym = yavs_syntax::Symbol::intern(n);
            let ix = ix.map(|v| {
                let c = db.consts.push(yavs_sem::stat::ConstValue::Int(v));
                db.exprs.push(yavs_sem::hir::Expr {
                    kind: yavs_sem::hir::ExprKind::Const(c),
                    subtype: db.poison.subtype,
                    span: Default::default(),
                    stat: yavs_sem::hir::Staticness::Locally,
                    value: Some(c),
                })
            });
            path.push((
                yavs_syntax::ast::Ident {
                    text: sym,
                    key: sym,
                    span: Default::default(),
                },
                ix,
            ));
        }
        db.externals.push(yavs_sem::hir::External {
            class: ExternalClass::Signal,
            path_kind: kind,
            path: path.into(),
            subtype: db.poison.subtype,
            span: Default::default(),
        })
    };
    let abs = id(
        &mut db,
        PathKind::Absolute,
        &[("t", None), ("dut", None), ("s", None)],
    );
    let gn = id(
        &mut db,
        PathKind::Absolute,
        &[("t", None), ("g", Some(1)), ("inner", None), ("s", None)],
    );
    let rel = id(
        &mut db,
        PathKind::Relative(0),
        &[("dut", None), ("s", None)],
    );
    let bad = id(
        &mut db,
        PathKind::Absolute,
        &[("t", None), ("nope", None), ("s", None)],
    );
    let mut m = crate::Model {
        hier: e.hier,
        nets: e.nets,
        vals: e.model_vals,
        sigs: e.sigs,
        ..Default::default()
    };
    let top = crate::ScopeId(0);
    let name = |m: &crate::Model, o: crate::ObjId| {
        let o = m.nets.obj(o);
        format!("{}{}", m.hier.path_name(o.scope, Case::Lrm), o.name.key)
    };
    let o = m.external(&db, abs, top).expect("absolute");
    assert_eq!(name(&m, o), ":t:dut:s");
    let o = m.external(&db, gn, top).expect("generate index");
    assert_eq!(name(&m, o), ":t:g(1):inner:s");
    let o = m.external(&db, rel, top).expect("relative");
    assert_eq!(name(&m, o), ":t:dut:s");
    let err = m.external(&db, bad, top).expect_err("missing region");
    assert_eq!(err.code, codes::EXTERNAL_NOT_FOUND);
}

/// An external name as an `in` port actual is a signal name actual (#186, LRM 8.7): a port
/// edge, no equivalent process.
#[test]
fn external_port_actual_is_a_signal_name() {
    let src = "entity leaf is end; architecture a of leaf is signal hidden : bit; begin end;
      entity mon is port (i : in bit); end; architecture a of mon is begin end;
      entity t is end; architecture a of t is
      begin
        l : entity work.leaf;
        m : entity work.mon port map (i => << signal .t.l.hidden : bit >>);
      end;";
    let (_, e) = elab_ports("external_port", src, "t", &[]);
    let id = |n: &str| {
        (0..e.nets.leaves.len() as u32)
            .map(crate::LeafId)
            .find(|&l| e.leaf_name(l).ends_with(n))
            .expect(n)
    };
    let (hidden, i) = (id(":l:hidden"), id(":m:i"));
    let one = Range::new(0, 1);
    assert_eq!(e.nets.ports.len(), 1);
    let p = &e.nets.ports[0];
    assert_eq!((p.formal, p.actual), ((i, one), (hidden, one)));
}

/// Blocks with generic and port clauses (#75).
#[test]
fn block_generics_and_ports() {
    let src = "entity t is end; architecture a of t is
        signal x : bit_vector(3 downto 0);
      begin
        b : block
          generic (W : natural);
          generic map (W => 4);
          port (p : in bit_vector(W - 1 downto 0));
          port map (p => x);
          signal inner : bit_vector(2 * W - 1 downto 0);
        begin
        end block;
      end;";
    let (_, e) = elab_ports("blockports", src, "t", &[]);
    assert_eq!(sig_paths(&e), [":t:x", ":t:b:p", ":t:b:inner"]);
    assert_eq!(e.nets.leaves[2].ty.len, 8);
    assert_eq!(e.nets.ports.len(), 1);
}

/// Incremental binding (LRM 7.3.2.3): a binding indication without an entity aspect in a
/// configuration adds a generic map to the primary binding of a specification (#75).
#[test]
fn incremental_binding() {
    // LRM 7.3.2.3: u2's configuration adds a generic map to the primary binding of its
    // configuration specification; u1 keeps the default binding.
    let src = "entity inv is generic (W : natural := 1); end;
      architecture rtl of inv is signal s : bit_vector(W - 1 downto 0); begin end;
      entity top is end;
      architecture t of top is
        component inv is generic (W : natural := 1); end component;
        for u2 : inv use entity work.inv(rtl);
      begin
        u1 : inv;
        u2 : inv;
      end;
      configuration cfg of top is
        for t
          for u2 : inv generic map (W => 3); end for;
        end for;
      end;";
    let (_, e) = elab("incremental", src, "cfg", &[]);
    let lens: Vec<u32> = e.nets.leaves.iter().map(|l| l.ty.len).collect();
    assert_eq!(lens, [1, 3]);
}

/// Conversion functions and type conversions in port maps run in the update phase (#74).
#[test]
fn port_conversions() {
    let src = "package conv is
        function to_int(b : bit) return integer;
        function to_bit(i : integer) return bit;
      end;
      package body conv is
        function to_int(b : bit) return integer is
        begin if b = '1' then return 10; else return 20; end if; end;
        function to_bit(i : integer) return bit is
        begin if i > 0 then return '1'; else return '0'; end if; end;
      end;
      entity c is port (i : in integer; o : out integer := 5; r : in real); end;
      architecture a of c is begin end;
      use work.conv.all;
      entity t is end; architecture a of t is
        signal s : bit := '1';
        signal y : bit;
        signal n : integer := 3;
      begin
        u : entity work.c port map (i => to_int(s), to_bit(o) => y, r => real(n));
      end;";
    let (_, e) = elab("convs", src, "t", &[]);
    assert_eq!(e.convs.len(), 3);
    let mut sim = e
        .build_with(yavs_kernel::Config::default(), |_, _| Ok(Box::new(Idle)))
        .expect("build");
    sim.run_for(yavs_kernel::Time::ns(1)).expect("run");
    let v = |sim: &yavs_kernel::SimControl, name: &str| {
        let id = sim.signal_id(name).unwrap_or_else(|| panic!("{name}"));
        sim.value(id).expect("value")
    };
    assert_eq!(
        v(&sim, ":t:u:i"),
        yavs_kernel::Value::Bytes(10i64.to_le_bytes().to_vec())
    );
    assert_eq!(
        v(&sim, ":t:u:r"),
        yavs_kernel::Value::Bytes(3f64.to_le_bytes().to_vec())
    );
    // o = 5 > 0, so y is '1' (code 3).
    let yavs_kernel::Value::Logic(y) = v(&sim, ":t:y") else {
        panic!("logic")
    };
    assert_eq!(y.get(0) as u8, 3);
}

/// Nested element and record element resolution indications (#76).
#[test]
fn nested_element_resolution() {
    let src = "package r is
        function rb(v : bit_vector) return bit;
        type arr2 is array (0 to 1) of bit_vector(0 to 1);
        subtype rarr2 is ((rb)) arr2;
        type rec is record a : bit; b : integer; end record;
      end;
      package body r is
        function rb(v : bit_vector) return bit is begin return v(v'left); end;
      end;
      use work.r.all;
      entity t is end; architecture a of t is
        signal s : rarr2;
        signal u : bit_vector(0 to 1);
      begin end;";
    let (_, e) = elab("nestedres", src, "t", &[]);
    assert!(matches!(
        e.nets.leaves[0].resolution,
        Some(crate::LeafRes::Func(_))
    ));
    assert!(e.nets.leaves[1].resolution.is_none());
}

#[test]
fn resolution_function_in_a_block_fails_early() {
    let src = "
      entity t is end;
      architecture a of t is
      begin
        b : block
          type iv is array (natural range <>) of integer;
          function r(v : iv) return integer is begin return v(v'low); end;
          subtype ri is r integer;
          signal s : ri := 0;
        begin
          s <= 1;
        end block;
      end;";
    let (mut db, mut e) = elab("res_block", src, "t", &[]);
    let codes: Vec<_> = e
        .lower(&mut db)
        .expect_err("not implemented")
        .into_iter()
        .map(|d| d.code)
        .collect();
    assert_eq!(codes, [codes::NOT_IMPLEMENTED]);
}

#[test]
fn composite_values_at_elaboration() {
    let src = "
      package p is
        type iv is array (natural range <>) of integer;
        function rev(v : iv) return iv;
      end;
      package body p is
        function rev(v : iv) return iv is
          variable r : iv(v'range);
        begin
          for i in v'range loop r(i) := v(v'high - i + v'low); end loop;
          return r;
        end;
      end;
      use work.p.all;
      entity t is end;
      architecture a of t is
        constant c : iv := rev((1, 2) & 3);
        signal s : bit_vector(c(0) downto 1);
      begin
      end;";
    let (_, e) = elab("composite_vals", src, "t", &[]);
    let o = e
        .nets
        .objs
        .iter()
        .find(|o| o.name.key.as_str() == "s")
        .expect("s");
    assert_eq!(o.shape.len(), 3);

    let md = "
      entity t is end;
      architecture a of t is
        type m is array (0 to 1, 0 to 1) of integer;
        constant c : m := ((1, 2), (3, 4));
        signal s : bit_vector(c(1, 0) downto 0);
      begin
      end;";
    let (_, e) = elab("composite_md", md, "t", &[]);
    let o = e
        .nets
        .objs
        .iter()
        .find(|o| o.name.key.as_str() == "s")
        .expect("s");
    assert_eq!(o.shape.len(), 4);

    // Shifts run the simulation's native (#220).
    let sh = "
      entity t is end;
      architecture a of t is
        constant c : bit_vector := not (\"01\" and \"11\");
        constant d : bit_vector := c srl 1;
        signal s : bit_vector(d'length + 2 * bit'pos(d(1)) downto 0);
      begin
      end;";
    let (_, e) = elab("composite_op", sh, "t", &[]);
    let o = e
        .nets
        .objs
        .iter()
        .find(|o| o.name.key.as_str() == "s")
        .expect("s");
    assert_eq!(o.shape.len(), 5);
}

/// 2019 mode views give each record element its own port mode; linkage ports read like `in`.
#[test]
fn mode_view_and_linkage_ports() {
    let src = "
      package p is
        type bus_t is record req : bit; addr : integer; ack : bit; end record;
        view master of bus_t is req, addr : out; ack : in; end view;
      end;
      use work.p.all;
      entity m is port (b : view master; l : linkage bit); end;
      architecture a of m is begin end;
      use work.p.all;
      entity t is end;
      architecture a of t is
        signal s : bus_t;
        signal x : bit;
      begin
        u : entity work.m port map (s, x);
      end;";
    let mut db = analyze_rev("views", src, LangRev::Vhdl2019);
    let top = crate::find_top(&mut db, "work", "t").expect("top");
    let e = crate::elaborate(&mut db, top, &[], &UNCOLLAPSED).expect("elab");
    let modes: Vec<PortMode> = e.nets.ports.iter().map(|p| p.mode).collect();
    assert_eq!(
        modes,
        [PortMode::Out, PortMode::Out, PortMode::In, PortMode::In]
    );
}

/// A record subtype resolved as a whole (LRM 4.6): every leaf is resolved by the function,
/// and a process driving one field drives every field (LRM 14.7.2, #193).
#[test]
fn composite_resolved_as_a_whole() {
    let src = "
      entity t is end;
      architecture a of t is
        type rec is record b : bit; n : integer; end record;
        type rec_vec is array (natural range <>) of rec;
        function res(v : rec_vec) return rec is begin return v(v'low); end;
        subtype rrec is res rec;
        signal s : rrec;
      begin
        s.n <= 1;
      end;";
    let (_, e) = elab("whole_res", src, "t", &[]);
    assert!(
        e.nets
            .leaves
            .iter()
            .all(|l| matches!(l.resolution, Some(crate::LeafRes::Whole(_))))
    );
    let driven: Vec<u32> = e.nets.drivers.iter().map(|d| d.leaf.0).collect();
    assert_eq!(driven, [0, 1]);
}

/// Composites resolved as a whole inside another composite: one group per array element,
/// and a process driving part of an element drives that element only (#333, LRM 14.7.2).
/// The resolution of a field below a resolved composite is not used (LRM 14.7.3.1).
#[test]
fn nested_composite_resolution() {
    let src = "
      library ieee; use ieee.std_logic_1164.all;
      entity t is end;
      architecture a of t is
        type rec is record b : bit; n : integer; l : std_logic; end record;
        type rec_vec is array (natural range <>) of rec;
        function res(v : rec_vec) return rec is begin return v(v'low); end;
        subtype rrec is res rec;
        type rarr is array (0 to 1) of rrec;
        signal s : rarr;
      begin
        s(1).n <= 1;
      end;";
    let (_, e) = elab("nested_whole", src, "t", &[]);
    assert!(
        e.nets
            .leaves
            .iter()
            .all(|l| matches!(l.resolution, Some(crate::LeafRes::Whole(_))))
    );
    let [w] = &e.nets.wholes[..] else {
        panic!("one composite resolved as a whole")
    };
    assert_eq!(w.instances.len(), 2);
    // Element 1: offset 0 of the logic leaves (mirrored), 1 of the integer leaf.
    let driven: Vec<(u32, u32, u32)> = (e.nets.drivers.iter())
        .map(|d| (d.leaf.0, d.range.offset, d.range.len))
        .collect();
    assert_eq!(driven, [(0, 0, 1), (1, 1, 1), (2, 0, 1)]);
}

/// 2019 signals of unconstrained subtypes take their bounds from the initial value, element
/// and record element bounds included (LRM 6.4.2.3, #187).
#[test]
fn signal_bounds_from_the_initial_value() {
    let src = "
      entity t is end;
      architecture a of t is
        type bv_array is array (natural range <>) of bit_vector;
        type rec is record v : bit_vector; n : integer; end record;
        signal s : bit_vector := (3 to 5 => '1');
        signal m : bv_array := (\"01\", \"10\", \"11\");
        signal r : rec := (v => \"1011\", n => 5);
      begin
      end;";
    let mut db = analyze_rev("sigbounds", src, LangRev::Vhdl2019);
    let top = crate::find_top(&mut db, "work", "t").expect("top");
    let e = crate::elaborate(&mut db, top, &[], &ElabConfig::default()).expect("elab");
    let dims = |sh: &crate::Shape| match sh {
        crate::Shape::Array { dims, .. } => dims.iter().map(|b| (b.left, b.right)).collect(),
        _ => Vec::new(),
    };
    assert_eq!(dims(&e.nets.objs[0].shape), [(3, 5)]);
    let crate::Shape::Array { elem, .. } = &e.nets.objs[1].shape else {
        panic!("array")
    };
    assert_eq!(dims(&e.nets.objs[1].shape), [(0, 2)]);
    assert_eq!(dims(elem), [(0, 1)]);
    let crate::Shape::Record { fields, .. } = &e.nets.objs[2].shape else {
        panic!("record")
    };
    assert_eq!(dims(&fields[0].1), [(0, 3)]);
}

/// Entities with generic types and subprograms (#75): one clone per distinct map of those
/// actuals, with its own mangled identity; a formal generic subprogram is a valid actual.
#[test]
fn generic_type_entities() {
    let src = "
      entity leaf is
        generic (type t; init : t; function f(x : t) return t);
        port (q : out t);
      end;
      architecture a of leaf is begin q <= f(init); end;
      entity mid is
        generic (type e; v : e; function g(x : e) return e);
      end;
      architecture a of mid is
        signal s : e;
      begin
        u : entity work.leaf generic map (t => e, init => v, f => g) port map (s);
      end;
      entity t is end;
      architecture a of t is
        function inc(x : integer) return integer is begin return x + 1; end;
        function neg(x : boolean) return boolean is begin return not x; end;
        signal a, b, c : integer;
        signal d : boolean;
      begin
        u1 : entity work.leaf generic map (t => integer, init => 1, f => inc) port map (a);
        u2 : entity work.leaf generic map (t => integer, init => 2, f => inc) port map (b);
        u3 : entity work.leaf generic map (t => boolean, init => true, f => neg) port map (d);
        m : entity work.mid generic map (e => integer, v => 5, g => inc);
      end;";
    let mut db = analyze_rev("gentypes", src, LangRev::Vhdl2019);
    let top = crate::find_top(&mut db, "work", "t").expect("top");
    let e = crate::elaborate(&mut db, top, &[], &ElabConfig::default()).expect("elab");
    let arch = |label: &str| {
        e.hier
            .scopes
            .iter()
            .find_map(|s| match s.kind {
                ScopeKind::Instance { arch, .. } if s.name.key.as_str() == label => Some(arch),
                _ => None,
            })
            .expect("instance")
    };
    // u1 and u2 differ only in a generic constant: one clone, instance data apart.
    assert_eq!(arch("u1"), arch("u2"));
    assert_ne!(arch("u1"), arch("u3"));
    // The leaf inside `m` gets `t => integer` and `f => inc` through `mid`'s formals.
    assert_eq!(arch("u"), arch("u1"));
    let name = |a| yavs_sem::mangle::region(&db, db.units[a].scope);
    assert!(name(arch("u1")).starts_with("work.leaf[t=>std.standard.integer,f=>"));
    assert!(name(arch("u3")).starts_with("work.leaf[t=>std.standard.boolean,f=>"));
    assert!(name(arch("u1")).ends_with("].a"), "{}", name(arch("u1")));
    assert_eq!(sig_paths(&e).len(), 9);
}

/// A top-level entity with a generic type has no actual for it: `-g` gives only constants.
#[test]
fn top_level_generic_type_has_no_actual() {
    let src = "entity t is generic (type g); end; architecture a of t is begin end;";
    let mut db = analyze_rev("gentop", src, LangRev::Vhdl2019);
    let top = crate::find_top(&mut db, "work", "t").expect("top");
    let Err(errs) = crate::elaborate(&mut db, top, &[], &ElabConfig::default()) else {
        panic!("elaboration succeeded");
    };
    let got: Vec<&str> = errs.iter().map(|d| d.code).collect();
    assert_eq!(got, [codes::GENERIC_NO_VALUE]);
}

/// Components with generic types and subprograms (#222): the component is cloned per map,
/// the bound entity gets the component's actuals through the default binding.
#[test]
fn generic_type_components() {
    let src = "
      entity leaf is
        generic (type t; init : t; function f(x : t) return t);
        port (q : out t);
      end;
      architecture a of leaf is begin q <= f(init); end;
      entity t is end;
      architecture a of t is
        component leaf is
          generic (type t; init : t; function f(x : t) return t);
          port (q : out t);
        end component;
        function inc(x : integer) return integer is begin return x + 1; end;
        function neg(x : boolean) return boolean is begin return not x; end;
        signal a, b : integer;
        signal d : boolean;
      begin
        u1 : leaf generic map (t => integer, init => 1, f => inc) port map (a);
        u2 : leaf generic map (t => integer, init => 2, f => inc) port map (b);
        u3 : leaf generic map (t => boolean, init => true, f => neg) port map (d);
      end;";
    let mut db = analyze_rev("gencomp", src, LangRev::Vhdl2019);
    let top = crate::find_top(&mut db, "work", "t").expect("top");
    let e = crate::elaborate(&mut db, top, &[], &ElabConfig::default()).expect("elab");
    let arch = |label: &str| {
        e.hier
            .scopes
            .iter()
            .find_map(|s| match s.kind {
                ScopeKind::Instance { arch, .. } if s.name.key.as_str() == label => Some(arch),
                _ => None,
            })
            .expect("instance")
    };
    assert_eq!(arch("u1"), arch("u2"));
    assert_ne!(arch("u1"), arch("u3"));
    // Three signals, three component ports, three entity ports.
    assert_eq!(sig_paths(&e).len(), 9);
}

/// A bound entity's generic type that the component does not have gets no actual: E2103.
#[test]
fn component_without_the_entitys_generic_type() {
    let src = "
      entity leaf is generic (type t; n : natural); end;
      architecture a of leaf is begin end;
      entity t is end;
      architecture a of t is
        component leaf is generic (n : natural); end component;
      begin
        u : leaf generic map (n => 1);
      end;";
    let mut db = analyze_rev("gencomp_missing", src, LangRev::Vhdl2019);
    let top = crate::find_top(&mut db, "work", "t").expect("top");
    let Err(errs) = crate::elaborate(&mut db, top, &[], &ElabConfig::default()) else {
        panic!("elaboration succeeded");
    };
    let got: Vec<&str> = errs.iter().map(|d| d.code).collect();
    assert_eq!(got, [codes::GENERIC_NO_VALUE]);
}

/// `others` in a configuration leaves out the instances earlier component configurations
/// name, so it may bind an instance next to one a specification binds (#75).
#[test]
fn configuration_others_after_incremental_binding() {
    let src = "entity inv is generic (W : natural := 1); end;
      architecture rtl of inv is signal s : bit_vector(W - 1 downto 0); begin end;
      entity top is end;
      architecture t of top is
        component inv is generic (W : natural := 1); end component;
        for u2 : inv use entity work.inv(rtl) generic map (W => 7);
      begin
        u1 : inv;
        u2 : inv;
        u3 : inv;
      end;
      configuration cfg of top is
        for t
          for u2 : inv generic map (W => 5); end for;
          for others : inv use entity work.inv(rtl) generic map (W => 2); end for;
        end for;
      end;";
    let (_, e) = elab("others_cfg", src, "cfg", &[]);
    let lens: Vec<u32> = e.nets.leaves.iter().map(|l| l.ty.len).collect();
    assert_eq!(lens, [2, 5, 2]);
}

/// An alias of an external signal binds where it is declared, in elaboration order (LRM 8.7,
/// 14.4.1): in the architecture before the instance it names it is an error, in a process
/// after it the alias denotes the instance's signal.
#[test]
fn external_signal_alias_binds_in_elaboration_order() {
    let sub = "entity sub is end; architecture a of sub is signal s : bit; begin end;";
    let early = format!(
        "{sub} entity t is end; architecture a of t is
           alias s is <<signal .t.u.s : bit>>;
         begin u : entity work.sub; end;"
    );
    assert_eq!(
        err_codes("ext_early", &early, "t", &[]),
        [codes::EXTERNAL_NOT_FOUND]
    );
    let late = format!(
        "{sub} entity t is end; architecture a of t is
         begin
           u : entity work.sub;
           p : process alias s is <<signal .t.u.s : bit>>; begin s <= '1'; wait; end process;
         end;"
    );
    let (db, e) = elab("ext_late", &late, "t", &[]);
    let alias = db
        .decls
        .iter()
        .find(|(id, _)| db.external_signal_alias(*id).is_some())
        .map(|(id, _)| id)
        .expect("the alias");
    let root = crate::hier::ScopeId(0);
    let o = e.signal(root, alias).expect("bound");
    assert_eq!(e.hier.scope(e.nets.obj(o).scope).name.text.as_str(), "u");
}

const VAR_PORT_PKG: &str = "
  package pk is type prot_t is protected procedure p; end protected; end;
  package body pk is
    type prot_t is protected body procedure p is begin end; end protected body;
  end;";

/// A top-level variable port (2019) has no actual to denote (LRM 6.5.6.3, #188).
#[test]
fn top_level_variable_port_has_no_actual() {
    let src = format!(
        "{VAR_PORT_PKG} use work.pk.all; entity t is port (variable x : inout prot_t); end;
         architecture a of t is begin end;"
    );
    let mut db = analyze_rev("varport_top", &src, LangRev::Vhdl2019);
    let top = crate::find_top(&mut db, "work", "t").expect("top");
    let Err(errs) = crate::elaborate(&mut db, top, &[], &ElabConfig::default()) else {
        panic!("elaboration succeeded");
    };
    let got: Vec<&str> = errs.iter().map(|d| d.code).collect();
    assert_eq!(got, [codes::VARIABLE_PORT_OPEN]);
}

/// Every variable port of a chain denotes the shared variable at its end (#188).
#[test]
fn variable_ports_denote_the_shared_variable() {
    let src = format!(
        "{VAR_PORT_PKG} use work.pk.all;
         entity leaf is port (variable x : inout prot_t); end;
         architecture a of leaf is begin end;
         use work.pk.all;
         entity mid is port (variable y : inout prot_t); end;
         architecture a of mid is begin u : entity work.leaf port map (x => y); end;
         use work.pk.all;
         entity t is end;
         architecture a of t is
           shared variable sv : prot_t;
         begin
           m : entity work.mid port map (y => sv);
           l : entity work.leaf port map (x => sv);
         end;"
    );
    let mut db = analyze_rev("varport_chain", &src, LangRev::Vhdl2019);
    let top = crate::find_top(&mut db, "work", "t").expect("top");
    let e = crate::elaborate(&mut db, top, &[], &ElabConfig::default()).expect("elaborates");
    let objs: Vec<_> = e.var_ports.values().collect();
    assert_eq!(objs.len(), 3);
    assert!(objs.iter().all(|v| matches!(
        v,
        crate::VarObj::Region { region, .. } if *region == crate::ScopeId(0)
    )));
    assert!(objs.windows(2).all(|w| w[0] == w[1]));
}

/// External names used directly in a process bind once the hierarchy is complete (#93): a
/// process before the instance reads and drives its signal; a second elaboration of the same
/// design binds the same names again (rule 4); a name with no target is E2117.
#[test]
fn external_names_in_processes_bind_late() {
    let sub = "entity sub is end; architecture a of sub is
        signal s : bit; constant c : integer := 4; begin end;";
    let src = format!(
        "{sub} entity t is end; architecture a of t is
         begin
           p : process
             variable v : integer := <<constant .t.u.c : integer>>;
           begin
             <<signal .t.u.s : bit>> <= '1';
             wait on <<signal .t.u.s : bit>>;
             wait;
           end process;
           u : entity work.sub;
         end;"
    );
    let (mut db, e) = elab("ext_direct", &src, "t", &[]);
    let p = &e.procs[0];
    let s = e.nets.drivers[p.drivers[0]].leaf;
    assert!(e.leaf_name(s).ends_with(":u:s"));
    let top = crate::Top {
        entity: e.top.entity,
        arch: e.top.arch,
        config: e.top.config,
    };
    let again = crate::elaborate(&mut db, top, &[], &ElabConfig::default()).expect("again");
    assert_eq!(again.nets.drivers[again.procs[0].drivers[0]].leaf, s);
    assert_eq!(again.model_vals, e.model_vals);
    let missing = format!(
        "{sub} entity t is end; architecture a of t is
         begin
           p : process begin report bit'image(<<signal .t.u.nope : bit>>); wait; end process;
           u : entity work.sub;
         end;"
    );
    assert_eq!(
        err_codes("ext_missing", &missing, "t", &[]),
        [codes::EXTERNAL_NOT_FOUND]
    );
    // A declaration binds in elaboration order: before the instance it names it is E2117.
    let early = format!(
        "{sub} entity t is end; architecture a of t is
           constant k : integer := <<constant .t.u.c : integer>>;
         begin u : entity work.sub; end;"
    );
    assert_eq!(
        err_codes("ext_const_early", &early, "t", &[]),
        [codes::EXTERNAL_NOT_FOUND]
    );
}

/// An expression actual of an unconstrained port needs a globally static subtype (LRM
/// 6.5.6.3, #74); a type conversion of a composite port keeps every leaf's length.
#[test]
fn port_actual_shape_errors() {
    let src = "
entity sub is port (v : in bit_vector); end;
architecture a of sub is begin end;
entity t is end;
architecture a of t is
  signal b : bit_vector(3 downto 0);
begin
  u : entity work.sub port map (v => not b);
end;
";
    let (_, r) = try_elab("unconstrained_expr", src, "t", &[]);
    let Err(v) = r else {
        panic!("elaboration succeeded")
    };
    assert_eq!(v.len(), 1, "{v:?}");
    assert_eq!(v[0].0, codes::PORT_UNCONSTRAINED);
    assert!(v[0].1.contains("globally static subtype"), "{v:?}");
    let src = "
package p is
  type pair is record a : integer; b : bit; end record;
  type pairs is array (natural range <>) of pair;
  type pairs2 is array (natural range <>) of pair;
end;
use work.p.all;
entity sub is port (v : in pairs(0 to 1)); end;
architecture a of sub is begin end;
use work.p.all;
entity t is end;
architecture a of t is
  signal s : pairs2(0 to 2);
begin
  u : entity work.sub port map (v => pairs(s));
end;
";
    assert_eq!(
        err_codes("conv_length", src, "t", &[]),
        [codes::PORT_MISMATCH]
    );
}

/// External names in package subprograms bind once the hierarchy exists, as package objects
/// (#255): the function reads the design's constant and signal at run time and a restart reads
/// them again; one that cannot be bound fails only when it is called; a package declaration
/// naming the hierarchy is E2117 (packages are elaborated first).
#[test]
fn external_names_in_package_subprograms() {
    let pkg = "package p is impure function get return integer; impure function bad return integer;
        end package;
        package body p is
          impure function get return integer is
          begin return <<constant .t.u.c : integer>> + <<signal .t.u.s : integer>>; end function;
          impure function bad return integer is
          begin return <<constant .t.nope : integer>>; end function;
        end package body;
        entity sub is end; architecture a of sub is
          signal s : integer := 1; constant c : integer := 40; begin s <= 2 after 1 ns; end;";
    let src = format!(
        "{pkg} use work.p.all; entity t is end; architecture a of t is begin
           u : entity work.sub;
           process begin
             report integer'image(get); wait for 2 ns; report integer'image(get); wait;
           end process;
         end;"
    );
    let (mut db, mut e) = elab("ext_pkg", &src, "t", &[]);
    assert_eq!(e.pkg_ext.len(), 2);
    assert_eq!(e.ext_unbound.len(), 1);
    e.lower(&mut db).expect("lower");
    let run = |e: &Elaborated| {
        let mut sim = e.build(yavs_kernel::Config::default()).expect("build");
        let got = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        sim.subscribe(Box::new(Collect(got.clone())));
        sim.run_until(yavs_kernel::Time::MAX).expect("run");
        got.lock().expect("lock").clone()
    };
    let first = run(&e);
    assert_eq!(first, ["0 1 41", "2000000 1 42"]);
    assert_eq!(first, run(&e));
    // Calling the function whose name cannot be bound is a run-time error.
    let calls_bad = format!(
        "{pkg} use work.p.all; entity t is end; architecture a of t is begin
           u : entity work.sub;
           process begin report integer'image(bad); wait; end process;
         end;"
    );
    let (mut db, mut e) = elab("ext_pkg_bad", &calls_bad, "t", &[]);
    e.lower(&mut db).expect("lower");
    let mut sim = e.build(yavs_kernel::Config::default()).expect("build");
    let err = sim.run_until(yavs_kernel::Time::MAX).expect_err("unbound");
    assert!(err.to_string().contains("t.nope"), "{err}");
    let decl = "package q is constant k : integer := <<constant .t.c : integer>>; end package;
        use work.q.all; entity t is end; architecture a of t is constant c : integer := 1;
        begin end;";
    assert_eq!(
        err_codes("ext_pkg_decl", decl, "t", &[]),
        [codes::EXTERNAL_NOT_FOUND]
    );
}

#[test]
fn shared_variable_holding_access_values_is_lowered() {
    // `--relaxed` accepts it; its objects live in the shared heap (#303).
    let src = "
      package p is
        type ptr is access integer;
        type rec is record n : integer; q : ptr; end record;
        shared variable v : rec;
      end package;
      use work.p.all;
      entity t is end;
      architecture a of t is
      begin
        process begin v.n := 1; wait; end process;
      end;";
    let tmp = tempfile::tempdir().expect("scratch dir");
    let path = tmp.path().join("t.vhd");
    std::fs::write(&path, src).expect("write");
    let mut opts = Options::new(LangRev::Vhdl2008, yavs_sem::default_stdlib());
    opts.relaxed = true;
    let mut db = Db::new(opts);
    assert!(db.analyze_file("work", &path, LangRev::Vhdl2008).is_ok());
    assert_eq!(db.error_count(), 0);
    let top = crate::find_top(&mut db, "work", "t").expect("top");
    let mut e = crate::elaborate(&mut db, top, &[], &ElabConfig::default()).expect("elab");
    e.lower(&mut db).expect("lowered");
}

/// A record port with an unconstrained element associated individually takes the element's
/// bounds from its actual (LRM 6.5.7.1, #288); left out, the element has none.
#[test]
fn record_port_parts_shape_unconstrained_elements() {
    let src = "
package p is
  type r is record a : bit; d : bit_vector; end record;
end;
use work.p.all;
entity sub is port (pin : in r); end;
architecture a of sub is begin end;
entity t is end;
architecture a of t is
  signal sa : bit;
  signal sd : bit_vector(7 downto 0);
begin
  u : entity work.sub port map (pin.a => sa, pin.d => sd);
  w : entity work.sub port map (pin.a => sa, pin.d => \"1010\");
end;
";
    let (_, e) = elab("record_parts_open", src, "t", &[]);
    let lens: Vec<_> = e
        .nets
        .objs
        .iter()
        .filter(|o| o.name.text.as_str() == "pin")
        .map(|o| match &o.shape {
            crate::hier::Shape::Record { fields, .. } => fields[1].1.len(),
            s => panic!("{s:?}"),
        })
        .collect();
    assert_eq!(lens, [8, 4]);
    let src = src.replace(", pin.d => \"1010\"", "");
    let (_, r) = try_elab("record_parts_missing", &src, "t", &[]);
    let Err(v) = r else {
        panic!("elaboration succeeded")
    };
    assert_eq!(v.len(), 1, "{v:?}");
    assert_eq!(v[0].0, codes::PORT_UNCONSTRAINED);
    assert!(v[0].1.contains("element `d` of port `pin`"), "{v:?}");
}

#[test]
fn record_port_element_bounds_check_the_index() {
    // #304: the bounds of `a(i).d` come from the port's actual, the index is still checked.
    let src = "package p is
        type r_t is record d : bit_vector; end record;
        type ra_t is array (natural range <>) of r_t;
      end;
      use work.p.all;
      entity c is port (a : in ra_t); end;
      architecture x of c is
        signal ok : bit_vector(a(1).d'range);
        signal bad : bit_vector(a(2).d'range);
      begin end;
      use work.p.all;
      entity t is end; architecture x of t is
        signal s : ra_t(0 to 1)(d(3 downto 0));
      begin u : entity work.c port map (a => s); end;";
    let (_, r) = try_elab("recidx", src, "t", &[]);
    let v = r.err().expect("error");
    assert_eq!(
        v,
        [(
            codes::EVAL_FAILED,
            "index 2 is out of the bounds of the prefix".to_string()
        )]
    );
}

#[test]
fn clocked_processes_get_edge_triggers() {
    let src = "library ieee; use ieee.std_logic_1164.all;
      entity t is end; architecture a of t is
        signal clk, rst : std_logic := '0';
        signal v : std_logic_vector(3 downto 0);
        signal q, r, s : std_logic;
      begin
        p1 : process (clk) begin if rising_edge(clk) then q <= '1'; end if; end process;
        p2 : process (rst) begin if falling_edge(rst) then r <= '1'; end if; end process;
        p5 : process (v(2)) begin if falling_edge(v(2)) then v(0) <= '1'; end if; end process;
        p3 : process (clk, rst) begin if rising_edge(clk) then s <= '1'; end if; end process;
        p4 : process (clk) begin if rising_edge(clk) then q <= '0'; else q <= '1'; end if; end process;
      end;";
    let (_, e) = elab("edges", src, "t", &[]);
    let t: Vec<_> = e
        .procs
        .iter()
        .map(|p| p.trigger.map(|(rising, _, elem)| (rising, elem)))
        .collect();
    // Not (yet) an element of a composite (p5).
    assert_eq!(t, [Some((true, 0)), Some((false, 0)), None, None, None]);
}

/// R4: an `in` port whose actual is whole signal leaves is an alias of the actual (through
/// the component port too); a part of a leaf, a forced port and an `out` port keep their
/// edges.
#[test]
fn in_ports_collapse() {
    let src = "
entity leaf is
  port (a : in bit; v : in bit_vector(3 downto 0); p : in bit; f : in bit; y : out bit);
end;
architecture rtl of leaf is begin
  y <= a xor p xor f;
  process begin f <= force '1'; wait; end process;
end;
entity top is end;
architecture t of top is
  component leaf is
    port (a : in bit; v : in bit_vector(3 downto 0); p : in bit; f : in bit; y : out bit);
  end component;
  signal x, y, z : bit;
  signal w : bit_vector(0 to 3);
  signal link : bit_vector(0 to 1);
begin
  u : leaf port map (a => x, v => w, p => link(1), f => z, y => y);
end;
";
    let obj = |e: &Elaborated, path: &str| {
        let k = sig_paths(e).iter().position(|p| p == path).expect(path);
        e.nets.objs[k].leaves
    };
    let (_, apart) = elab_ports("collapse", src, "top", &[]);
    let (_, e) = elab("collapse", src, "top", &[]);
    assert_eq!(obj(&e, ":top:u:a"), obj(&e, ":top:x"));
    assert_eq!(obj(&e, ":top:u:v"), obj(&e, ":top:w"));
    assert_ne!(obj(&e, ":top:u:p"), obj(&e, ":top:link"));
    assert_ne!(obj(&e, ":top:u:f"), obj(&e, ":top:z"));
    // Component and entity ports of `a` and `v`, the entity's `p` (its actual is the whole
    // component port) and the component's `f` (only the entity's is forced).
    assert_eq!(apart.nets.leaves.len() - e.nets.leaves.len(), 6);
    assert_eq!(apart.nets.ports.len() - e.nets.ports.len(), 6);
    for p in &e.nets.ports {
        assert!(
            p.formal.0.index() < e.nets.leaves.len() && p.actual.0.index() < e.nets.leaves.len()
        );
    }
}
