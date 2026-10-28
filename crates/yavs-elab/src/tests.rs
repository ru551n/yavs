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
    packs: true,
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
    let (_, e) = elab_ports("unconstrained", src, "t", &[]);
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
    // `integer` is 4 bytes in the kernel (`Ty::elem_size`).
    assert_eq!(lens, [(3, Elem::Bytes(4)), (6, Elem::Logic)]);
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

/// Rule 4 (sim2 #1184): elaborating twice in one process, and building twice from one
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

/// A function that reads a protected shared variable of its architecture is called during
/// elaboration (#512): the variable is created for the call, later calls and the processes see
/// its state, also when the simulation cannot start from the session's state and replays the
/// calls (`start` dropped), and when a call is undone and the state rebuilt from the log.
#[test]
fn elaboration_calls_with_region_shared_variable() {
    let src = "entity leaf is generic (base : natural); end;
      architecture a of leaf is
        type counter_t is protected
          impure function bump return natural;
        end protected;
        type counter_t is protected body
          variable n : natural := 0;
          impure function bump return natural is begin n := n + 1; return n; end;
        end protected body;
        shared variable c : counter_t;
        impure function next_id return natural is begin return base + c.bump; end;
        signal x : bit := '1';
        constant first : natural := next_id;
        constant late : natural := next_id + x'path_name'length;
        constant third : natural := next_id;
      begin
        process begin
          report \"first=\" & integer'image(first) & \" late=\" & integer'image(late)
            & \" third=\" & integer'image(third) & \" next=\" & integer'image(c.bump);
          wait;
        end process;
      end;
      entity t is end; architecture a of t is begin
        u : entity work.leaf generic map (base => 10);
        v : entity work.leaf generic map (base => 20);
      end;";
    for replay in [false, true] {
        let (mut db, mut e) = elab("region_var", src, "t", &[]);
        let w: Vec<&str> = e.warnings.iter().map(|d| d.code).collect();
        assert_eq!(w, [codes::IMPURE_AT_RUN_TIME; 2]);
        if replay {
            e.start = None;
        }
        e.lower(&mut db).expect("lower");
        let mut sim = e.build(yavs_kernel::Config::default()).expect("build");
        let got = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        sim.subscribe(Box::new(Collect(got.clone())));
        sim.run_until(yavs_kernel::Time::MAX).expect("run");
        let mut got = got.lock().expect("lock").clone();
        got.sort();
        // `late` runs its call at the start of the simulation: after the elaboration-time ones.
        assert_eq!(
            got,
            [
                "0 0 first=11 late=19 third=12 next=4",
                "0 1 first=21 late=29 third=22 next=4"
            ],
            "replay={replay}"
        );
    }
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

/// External names bind against the elaborated hierarchy (#76, sim2 #548). Sem does not
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
        yavs_kernel::Value::Bytes(10i32.to_le_bytes().to_vec())
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

/// An external name's subtype indication must give the object's type and the lengths of its
/// constrained array dimensions (LRM 8.7, #463); an unconstrained one takes the object's.
#[test]
fn external_name_subtype_must_match() {
    let sub = "entity sub is end; architecture a of sub is
        signal s : bit_vector(3 downto 0); signal b : bit;
        constant c : integer := 4; begin end;";
    let one = |name: &str, alias: &str| {
        let src = format!(
            "{sub} entity t is end; architecture a of t is
             begin u : entity work.sub;
               p : process {alias} begin wait; end process;
             end;"
        );
        match try_elab(name, &src, "t", &[]).1 {
            Ok(_) => Vec::new(),
            Err(v) => v.into_iter().map(|(c, _)| c).collect(),
        }
    };
    for ok in [
        "alias x is <<signal .t.u.s : bit_vector(3 downto 0)>>;",
        "alias x is <<signal .t.u.s : bit_vector(0 to 3)>>;",
        "alias x is <<signal .t.u.s : bit_vector>>;",
        "alias x is <<constant .t.u.c : natural>>;",
    ] {
        assert!(one("ext_ok", ok).is_empty(), "{ok}");
    }
    for bad in [
        "alias x is <<signal .t.u.s : bit_vector(7 downto 0)>>;",
        "alias x is <<signal .t.u.b : integer>>;",
        "alias x is <<signal .t.u.s : bit>>;",
        "alias x is <<constant .t.u.c : bit>>;",
    ] {
        assert_eq!(one("ext_bad", bad), [codes::EXTERNAL_MISMATCH], "{bad}");
    }
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
fn index_constraint_outside_the_index_subtype() {
    // #637, LRM 5.3.2.2: a non-null index range from a generic must lie inside POSITIVE; a null
    // one may have any bounds.
    let src = "entity t is generic (g : integer := 0); end;
      architecture x of t is
        signal ok : string(g + 1 to 3);
        signal null_r : string(g to -1);
        signal bad : string(g to 3);
      begin end;";
    let (_, r) = try_elab("idxsub", src, "t", &[]);
    let v = r.err().expect("error");
    assert_eq!(
        v,
        [(
            codes::EVAL_FAILED,
            "index range 0 to 3 is outside index subtype `POSITIVE` (1 to 2147483647)".to_string()
        )]
    );
}

#[test]
fn unused_constant_index_constraint_is_checked() {
    // #825, LRM 5.3.2.2, 14.4.2.1: the subtype indication of a constant nothing reads is
    // elaborated with its declaration, so its index range is checked too.
    let src = "entity t is end;
      architecture x of t is
        function n return integer is begin return 0; end;
        constant ok : string(n + 1 to 3) := \"abc\";
        constant bad : string(n to 2) := \"abc\";
      begin end;";
    let (_, r) = try_elab("unusedc", src, "t", &[]);
    let v = r.err().expect("error");
    assert_eq!(
        v,
        [(
            codes::EVAL_FAILED,
            "index range 0 to 2 is outside index subtype `POSITIVE` (1 to 2147483647)".to_string()
        )]
    );
}

#[test]
fn signal_initial_value_outside_the_element_subtype() {
    // #821: each scalar of a signal's initial value lies in its (element) subtype, also when
    // the value comes from a generic.
    let src = "entity t is generic (g : integer := 0); end;
      architecture x of t is
        type aray is array (0 to 3) of positive;
        signal ok : aray := (others => g + 1);
        signal bad : aray := (others => g);
      begin end;";
    let (_, r) = try_elab("sigrange", src, "t", &[]);
    let v = r.err().expect("error");
    assert_eq!(
        v,
        [(
            codes::EVAL_FAILED,
            "initial value of `bad` does not fit: value 0 is outside subtype `POSITIVE` (1 to 2147483647)".to_string()
        )]
    );
}

#[test]
fn clocked_processes_get_edge_triggers() {
    let src = "library ieee; use ieee.std_logic_1164.all;
      entity t is end; architecture a of t is
        signal clk, rst, en : std_logic := '0';
        signal v : std_logic_vector(3 downto 0);
        signal q, r, s : std_logic;
        signal bt, bt2 : bit;
        signal ul : std_ulogic;
        signal bo : boolean;
      begin
        p1 : process (clk) begin if rising_edge(clk) then q <= '1'; end if; end process;
        p2 : process (rst) begin if falling_edge(rst) then r <= '1'; end if; end process;
        p3 : process (v) begin if falling_edge(v(2)) then v(0) <= '1'; end if; end process;
        p4 : process (clk, rst) begin if rising_edge(clk) then s <= '1'; end if; end process;
        p5 : process (clk) begin if rising_edge(clk) then q <= '0'; else q <= '1'; end if; end process;
        p6 : process (clk) begin if rising_edge(clk) and en = '1' then q <= '0'; end if; end process;
        p7 : process (clk) begin if en = '1' and rising_edge(clk) then q <= '0'; end if; end process;
        p8 : process begin wait until rising_edge(clk); q <= '1'; end process;
        p9 : process begin wait until rising_edge(clk); wait for 1 ns; end process;
        p10 : process (clk, rst) begin if rst = '1' then q <= '0'; elsif rising_edge(clk) then q <= '1'; end if; end process;
        g : for i in 0 to 1 generate
          p11 : process (v) begin if rising_edge(v(i + 1)) then q <= '1'; end if; end process;
        end generate;
        p12 : process (clk) begin if clk'event and clk = '1' then q <= '1'; end if; end process;
        p13 : process (clk) begin if clk = '0' and clk'event then q <= '1'; end if; end process;
        p14 : process (clk) begin if clk'event and clk = 'H' then q <= '1'; end if; end process;
        p15 : process (clk, rst) begin
          if rst = '0' then q <= '0'; elsif falling_edge(clk) then q <= '1'; end if;
        end process;
        p16 : process (bt) begin if bt'event and bt = '1' then q <= '1'; end if; end process;
        p17 : process (clk, rst) begin
          if rst = '1' then q <= '0'; elsif clk'event and clk = '1' and en = '1' then q <= '1'; end if;
        end process;
        p18 : process (clk, rst) begin
          if rst = '1' then q <= '0'; elsif rising_edge(clk) then q <= '1'; else q <= '0'; end if;
        end process;
        p19 : process (clk, rst) begin
          if rst = en then q <= '0'; elsif rising_edge(clk) then q <= '1'; end if;
        end process;
        p20 : process (clk) begin if not clk'stable and clk = '1' then q <= '1'; end if; end process;
        p21 : process (clk) begin if clk = '0' and not clk'stable then q <= '1'; end if; end process;
        p22 : process (bt) begin if rising_edge(bt) then q <= '1'; end if; end process;
        p23 : process (bt) begin if falling_edge(bt) then q <= '1'; end if; end process;
        p24 : process begin wait until clk = '1'; q <= '1'; end process;
        p25 : process begin wait until '0' = clk; q <= '1'; end process;
        p26 : process begin wait until bt = '1'; q <= '1'; end process;
        p27 : process begin wait on clk until clk = '1'; q <= '1'; end process;
        p28 : process begin wait until clk = '1' and en = '1'; q <= '1'; end process;
        p29 : process (ul) begin if rising_edge(ul) then q <= '1'; end if; end process;
        p30 : process (ul) begin if ul'event and ul = '1' then q <= '1'; end if; end process;
        p31 : process (bt, bt2) begin
          if bt2 = '1' then q <= '0'; elsif rising_edge(bt) then q <= '1'; end if;
        end process;
        p32 : process begin wait until bt'event and bt = '0'; q <= '1'; end process;
        p33 : process (bo) begin if rising_edge(bo) then q <= '1'; end if; end process;
        p34 : process (bt) begin if not bt'stable and bt = '1' then q <= '1'; end if; end process;
        p35 : process (clk) begin if clk = '1' then q <= '1'; end if; end process;
        p36 : process begin wait until rising_edge(clk) and en = '1'; q <= '1'; end process;
        p37 : process begin wait on clk, rst until rising_edge(clk); q <= '1'; end process;
        p38 : process begin wait until en = '1' and rising_edge(clk); q <= '1'; end process;
        p39 : process begin wait until rising_edge(bt) and bt2 = '1'; q <= '1'; end process;
        p40 : process begin wait until clk'event and clk = '1' and en = '1'; q <= '1'; end process;
        p41 : process begin wait until falling_edge(ul); q <= '1'; end process;
        p42 : process begin wait until rising_edge(clk) or rst = '1'; q <= '1'; end process;
        p43 : process begin wait until bt'event and bt = '1' and rst = '1'; q <= '1'; end process;
      end;";
    let (_, e) = elab("edges", src, "t", &[]);
    let t: Vec<_> = e
        .procs
        .iter()
        .map(|p| {
            p.trigger.map(|t| {
                (
                    t.rising,
                    t.elem,
                    t.wait,
                    t.legacy,
                    t.reset.map(|r| (r.1, r.2)),
                )
            })
        })
        .collect();
    let (edge, wait, legacy) = ((false, false), (true, false), (false, true));
    let x = |rising, elem, (w, l): (bool, bool), reset| Some((rising, elem, w, l, reset));
    assert_eq!(
        t,
        [
            x(true, 0, edge, None),
            x(false, 0, edge, None),
            x(false, 2, edge, None),
            x(true, 0, edge, None),
            None,
            x(true, 0, edge, None),
            x(true, 0, edge, None),
            x(true, 0, wait, None),
            None,
            x(true, 0, edge, Some((0, 3))),
            x(true, 1, edge, None),
            x(true, 2, edge, None),
            x(true, 0, legacy, None),
            x(false, 0, legacy, None),
            None,
            x(false, 0, edge, Some((0, 2))),
            x(true, 0, legacy, None),
            x(true, 0, legacy, Some((0, 3))),
            None,
            None,
            x(true, 0, legacy, None),
            x(false, 0, legacy, None),
            x(true, 0, edge, None),
            x(false, 0, edge, None),
            x(true, 0, (true, true), None),
            x(false, 0, (true, true), None),
            x(true, 0, (true, true), None),
            x(true, 0, (true, true), None),
            None,
            x(true, 0, edge, None),
            x(true, 0, legacy, None),
            x(true, 0, edge, Some((0, 3))),
            x(false, 0, (true, true), None),
            x(true, 0, edge, None),
            x(true, 0, legacy, None),
            None,
            x(true, 0, (true, false), None),
            x(true, 0, (true, false), None),
            x(true, 0, (true, false), None),
            x(true, 0, (true, false), None),
            x(true, 0, (true, true), None),
            x(false, 0, (true, false), None),
            None,
            x(true, 0, (true, true), None),
        ]
    );
}

/// R4: an `in` port whose actual is whole signal leaves, or part of one leaf, is an alias of
/// the actual (through the component port too); a forced port keeps its edges.
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
        (e.nets.objs[k].leaves, e.nets.objs[k].part)
    };
    let (_, apart) = elab_ports("collapse", src, "top", &[]);
    let (_, e) = elab("collapse", src, "top", &[]);
    assert_eq!(obj(&e, ":top:u:a"), obj(&e, ":top:x"));
    assert_eq!(obj(&e, ":top:u:v"), obj(&e, ":top:w"));
    // `link(1)`: the rightmost element of a logic leaf is kernel offset 0.
    let (link, _) = obj(&e, ":top:link");
    assert_eq!(obj(&e, ":top:u:p"), (link, Some(Range::new(0, 1))));
    assert_ne!(obj(&e, ":top:u:f").0, obj(&e, ":top:z").0);
    // Component and entity ports of `a`, `v`, `p` and `y` (an `out` port, #528), and the
    // component's `f` (only the entity's is forced).
    assert_eq!(apart.nets.leaves.len() - e.nets.leaves.len(), 9);
    assert_eq!(apart.nets.ports.len() - e.nets.ports.len(), 9);
    for p in &e.nets.ports {
        assert!(
            p.formal.0.index() < e.nets.leaves.len() && p.actual.0.index() < e.nets.leaves.len()
        );
    }
}

/// #528: an `out` or `buffer` port that is the only source of its actual's elements is an
/// alias of the actual, which takes the formal's initial value; an `inout` port, an actual
/// with another source, a forced actual and an actual fed from above keep their edges.
#[test]
fn out_ports_collapse() {
    let src = "library ieee; use ieee.std_logic_1164.all;
entity leaf is
  port (y : out bit := '1'; v : out bit_vector(3 downto 0); bf : buffer bit;
        io : inout bit; s : out std_logic);
end;
architecture rtl of leaf is begin
  y <= '0'; v <= \"0101\"; bf <= not bf after 1 ns; io <= '0'; s <= '1';
end;
library ieee; use ieee.std_logic_1164.all;
entity mid is port (y : inout bit); end;
architecture a of mid is begin
  u : entity work.leaf port map (y => y, v => open, bf => open, io => open, s => open);
end;
library ieee; use ieee.std_logic_1164.all;
entity top is end;
architecture t of top is
  signal y, b, io, f, m : bit;
  signal w : bit_vector(0 to 7);
  signal s1, s2 : std_logic;
begin
  u : entity work.leaf port map (y => y, v => w(2 to 5), bf => b, io => io, s => s1);
  k : entity work.leaf port map (y => f, v => open, bf => open, io => open, s => s2);
  s2 <= 'Z';
  i : entity work.mid port map (y => m);
  process begin f <= force '1'; wait; end process;
end;
";
    let obj = |e: &Elaborated, path: &str| {
        let k = sig_paths(e).iter().position(|p| p == path).expect(path);
        (e.nets.objs[k].leaves, e.nets.objs[k].part)
    };
    let init = |e: &Elaborated, path: &str| e.nets.leaf(obj(e, path).0.0).init.clone();
    let (_, apart) = elab_ports("outcollapse", src, "top", &[]);
    let (_, e) = elab("outcollapse", src, "top", &[]);
    assert_eq!(obj(&e, ":top:u:y"), obj(&e, ":top:y"));
    assert_eq!(init(&e, ":top:y"), init(&apart, ":top:u:y"));
    assert_ne!(init(&apart, ":top:y"), init(&apart, ":top:u:y"));
    // `w(2 to 5)`: kernel offsets 2 to 5 of the mirrored logic leaf.
    let (wl, _) = obj(&e, ":top:w");
    assert_eq!(obj(&e, ":top:u:v"), (wl, Some(Range::new(2, 4))));
    assert_eq!(obj(&e, ":top:u:bf"), obj(&e, ":top:b"));
    assert_eq!(obj(&e, ":top:u:s"), obj(&e, ":top:s1"));
    for kept in [":top:u:io", ":top:k:s", ":top:k:y", ":top:i:u:y"] {
        let (l, _) = obj(&e, kept);
        assert!(e.nets.ports.iter().any(|p| p.formal.0 == l.0), "{kept}");
    }
}

#[test]
fn scope_vals_keep_entries_sorted_by_declaration() {
    use yavs_sem::ids::Id;
    let d = |i| yavs_sem::DeclId::from_index(i);
    let mut v = crate::ScopeVals::default();
    for (i, x) in [(5, 50), (2, 20), (9, 90), (2, 21), (7, 70)] {
        v.insert(d(i), crate::Val::Int(x));
    }
    assert_eq!(v.len(), 4);
    assert_eq!(v.get(&d(2)), Some(&crate::Val::Int(21)));
    assert_eq!(v.get(&d(7)), Some(&crate::Val::Int(70)));
    assert!(!v.contains_key(&d(3)));
    let order: Vec<_> = v.iter().map(|(k, _)| k.index()).collect();
    assert_eq!(order, [2, 5, 7, 9]);
}

/// A concurrent `target <= source;` of static signal names (whole signals, record fields,
/// indexed elements, static slices, through an `in` port that collapses into its actual) is a
/// copy the kernel runs itself (`ProcInst::copy`, #529); conditional, delayed, guarded or
/// computed ones, aggregate targets and a target subtype that checks the value are not. Copies give the same reports as the interpreted statements
/// (`Config::edge_triggers` off keeps them interpreted).
#[test]
fn signal_copies_run_in_the_kernel() {
    let src = "library ieee; use ieee.std_logic_1164.all;
      package p is
        type rec is record a : std_logic; b : std_logic_vector(7 downto 0); n : integer; end record;
        type arr is array (0 to 3) of rec;
      end;
      library ieee; use ieee.std_logic_1164.all; use work.p.all;
      entity inner is port (i : in rec; o : out rec); end;
      architecture a of inner is begin
        o <= i;
      end;
      library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all; use work.p.all;
      entity t is end; architecture a of t is
        signal s, x, y : rec := ('0', x\"00\", 0);
        signal ar : arr;
        signal v : std_logic_vector(0 to 7) := x\"0F\";
        signal w, z : std_logic_vector(7 downto 0);
        signal n1, n2 : integer := 0;
        signal nn : natural;
        signal un : unsigned(7 downto 0);
        signal zz : std_logic_vector(3 downto 0);
      begin
        u : entity work.inner port map (i => s, o => x);
        ar(2) <= x;
        w <= v;
        n1 <= s.n;
        y.b <= ar(2).b;
        z <= v when s.a = '1' else not v;
        n2 <= s.n after 1 ns;
        z(1 downto 0) <= w(3 downto 2);
        nn <= n1;
        un <= unsigned(w);
        zz <= std_logic_vector(un(5 downto 2));
        process begin
          for i in 1 to 4 loop
            s <= (std_logic'val(2 + i mod 2), x\"A5\", i);
            v <= not v;
            wait for 2 ns;
            report std_logic'image(x.a) & to_string(x.b) & integer'image(ar(2).n)
              & to_string(w) & integer'image(n1) & integer'image(n2) & to_string(y.b) & to_string(z)
              & to_string(std_logic_vector(un)) & to_string(zz);
          end loop;
          wait;
        end process;
      end;";
    let (mut db, mut e) = elab("copies", src, "t", &[]);
    let copies: Vec<bool> = e.procs.iter().map(|p| p.copy.is_some()).collect();
    // inner's `o <= i`, then the statements of t in order, then its process.
    assert_eq!(
        copies,
        [
            true, true, true, true, true, false, false, true, false, true, true, false
        ]
    );
    e.lower(&mut db).expect("lower");
    let run = |e: &Elaborated, cfg: yavs_kernel::Config| {
        let mut sim = e.build(cfg).expect("build");
        let got = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        sim.subscribe(Box::new(Collect(got.clone())));
        sim.run_until(yavs_kernel::Time::MAX).expect("run");
        got.lock().expect("lock").clone()
    };
    let kernel = run(&e, yavs_kernel::Config::default());
    let interp = run(
        &e,
        yavs_kernel::Config {
            edge_triggers: false,
            ..Default::default()
        },
    );
    assert_eq!(kernel.len(), 4);
    assert_eq!(kernel, interp);
}

/// A concurrent `x <= not x after T;` on `std_ulogic`, BIT or BOOLEAN with a static delay is a
/// kernel `ToggleProcess`; other shapes (a computed or signal-dependent delay, another source,
/// a rejection limit, a constrained subtype, vector `not`) stay processes. Same reports either
/// way (`Config::edge_triggers` off keeps them interpreted).
#[test]
fn clock_generators_run_in_the_kernel() {
    let src = "library ieee; use ieee.std_logic_1164.all;
      entity t is generic (h : time := 3 ns); end; architecture a of t is
        constant k : time := h * 2;
        signal a : std_logic := '0';
        signal b : std_ulogic := '1';
        signal c : bit;
        signal d : boolean;
        signal v : std_logic_vector(1 downto 0) := \"01\";
        signal e : std_logic := '0';
        signal f, g, i, j : std_logic := '0';
        signal l : std_logic range '0' to '1' := '0';
        signal per : time := 2 ns;
        signal m, n : std_logic := '0';
        signal o : bit_vector(0 to 1);
      begin
        a <= not a after h;
        b <= not b after k;
        c <= transport not c after 5 ns;
        d <= not d after k / 2;
        v(1) <= not v(1) after 4 ns;
        e <= not f after 4 ns;
        f <= '1' after 2 ns;
        g <= reject 1 ns inertial not g after 4 ns;
        i <= not i after per;
        j <= not i;
        l <= not l after 3 ns;
        m <= not m after 3 ns;
        m <= '1' after 20 ns;
        n <= not n after 3 ns, '1' after 9 ns;
        o <= not o after 1 ns;
        process (a, b, c, d, v, e, f, g, i, j, l, m, n) begin
          report std_logic'image(a) & std_ulogic'image(b) & bit'image(c) & boolean'image(d)
            & to_string(v) & std_logic'image(e) & std_logic'image(f) & std_logic'image(g)
            & std_logic'image(i) & std_logic'image(j) & std_logic'image(l) & std_logic'image(m)
            & std_logic'image(n);
        end process;
        process begin wait for 40 ns; report \"end\"; wait; end process;
      end;";
    let (mut db, mut e) = elab("toggles", src, "t", &[]);
    let toggles: Vec<bool> = e.procs.iter().map(|p| p.toggle.is_some()).collect();
    // The statements in order (one process each), then the two processes.
    assert_eq!(
        toggles,
        [
            true, true, true, true, true, false, false, false, false, false, false, true, false,
            false, false, false, false
        ]
    );
    e.lower(&mut db).expect("lower");
    let run = |e: &Elaborated, cfg: yavs_kernel::Config| {
        let mut sim = e.build(cfg).expect("build");
        let got = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        sim.subscribe(Box::new(Collect(got.clone())));
        sim.run_until(yavs_kernel::Time::ns(40)).expect("run");
        got.lock().expect("lock").clone()
    };
    let kernel = run(&e, yavs_kernel::Config::default());
    let interp = run(
        &e,
        yavs_kernel::Config {
            edge_triggers: false,
            ..Default::default()
        },
    );
    assert!(kernel.len() > 40);
    assert_eq!(kernel, interp);
}

/// A concurrent assignment of one logical operator on static signal names (`std_ulogic`, BIT,
/// BOOLEAN and their vectors, wider than a plane word, a resolved target with two such drivers,
/// through an `in` port, inertial and transport `after`) is a gate the kernel runs itself
/// (`ProcInst::gate`, #779); a reduction, a mixed scalar and vector operand, another package's
/// operator, a constrained target, a rejection limit, two waveform elements, a
/// condition, a guard or `postponed` are not. Reports, with time and delta, are those of the
/// interpreted statements over all 81 pairs of values.
#[test]
fn logic_gates_run_in_the_kernel() {
    let src = "library ieee; use ieee.std_logic_1164.all;
      entity inner is port (i : in std_logic; o : out std_logic); end;
      architecture a of inner is begin
        o <= not i;
      end;
      library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
      entity t is end; architecture a of t is
        signal a, b, en : std_logic := '0';
        signal y1, y2, y3, y4, y5, y6, y7, y8, y9, yi, r : std_logic;
        signal v1 : std_logic_vector(3 downto 0);
        signal v2 : std_logic_vector(0 to 3);
        signal v3 : std_logic_vector(3 downto 0);
        signal v4 : std_ulogic_vector(1 downto 0);
        signal w1, w2, w3 : std_logic_vector(69 downto 0);
        signal ba, bc, bb : bit;
        signal bl, bo : boolean;
        signal bv1, bv2, bv3 : bit_vector(2 downto 0);
        signal n1, n2 : std_logic;
        signal n3 : std_logic_vector(3 downto 0);
        signal un1, un2, un3 : unsigned(3 downto 0);
        signal cl, c1 : std_logic range '0' to '1' := '0';
      begin
        u : entity work.inner port map (i => a, o => yi);
        y1 <= not a;
        y2 <= a and b;
        y3 <= a nand b after 2 ns;
        y4 <= transport a xor b after 3 ns;
        v3 <= v1 or v2;
        v4 <= v1(3 downto 2) nor v2(0 to 1);
        w3 <= w1 xnor w2;
        bb <= ba xnor bc;
        bo <= not bl;
        bv3 <= bv1 and bv2;
        r <= a and b;
        r <= a or b;
        n1 <= and v1;
        n3 <= v1 and a;
        un3 <= un1 and un2;
        y5 <= a and '1';
        c1 <= not cl;
        y6 <= reject 1 ns inertial a and b after 2 ns;
        y7 <= a and b, '0' after 4 ns;
        y8 <= a and b when en = '1' else '0';
        blk : block (en = '1') begin y9 <= guarded a or b; end block;
        postponed n2 <= a or b after 1 ns;
        process begin
          for i in 0 to 80 loop
            a <= std_logic'val(i / 9);
            b <= std_logic'val(i mod 9);
            en <= std_logic'val(i mod 4 + 2);
            v1 <= (std_logic'val(i mod 9), std_logic'val(i / 9), std_logic'val((i + 4) mod 9), '1');
            v2 <= (std_logic'val((i * 7) mod 9), 'H', std_logic'val(i / 9), std_logic'val(i mod 9));
            w1 <= (others => std_logic'val(i mod 9));
            w1(64) <= std_logic'val(i / 9);
            w2 <= (others => std_logic'val(i / 9));
            ba <= bit'val(i mod 2);
            bc <= bit'val(i / 2 mod 2);
            bl <= i mod 3 = 0;
            bv1 <= (bit'val(i mod 2), bit'val(i / 2 mod 2), '1');
            bv2 <= (bit'val(i / 4 mod 2), '1', bit'val(i mod 2));
            un1 <= to_unsigned(i mod 16, 4);
            un2 <= to_unsigned(i / 5 mod 16, 4);
            wait for 1 ns;
          end loop;
          wait;
        end process;
        process (y1, y2, y3, y4, y5, y6, y7, y8, y9, yi, r, v3, v4, w3, bb, bo, bv3, n1, n2, n3, un3, c1) begin
          report to_string(y1) & to_string(y2) & to_string(y3) & to_string(y4) & to_string(y5)
            & to_string(y6) & to_string(y7) & to_string(y8) & to_string(y9) & to_string(yi)
            & to_string(r) & ' ' & to_string(v3) & ' ' & to_string(v4) & ' ' & to_string(w3)
            & ' ' & to_string(bb) & boolean'image(bo) & to_string(bv3) & to_string(n1)
            & to_string(n2) & to_string(n3) & to_string(un3) & to_string(c1);
        end process;
      end;";
    let (mut db, mut e) = elab("gates", src, "t", &[]);
    let gates: Vec<bool> = e.procs.iter().map(|p| p.gate.is_some()).collect();
    // inner's `o <= not i`, then the statements of t in order (the block's `_guard` before
    // its statement), then its two processes.
    let mut want = vec![true; 13];
    want.extend([false; 3]);
    // `y5 <= a and '1'` has a literal operand (#786).
    want.push(true);
    want.extend([false; 9]);
    assert_eq!(gates, want);
    e.lower(&mut db).expect("lower");
    struct Seen(std::sync::Arc<std::sync::Mutex<Vec<String>>>);
    impl yavs_kernel::Observer for Seen {
        fn report(&mut self, hook: &mut yavs_kernel::Hook<'_>, r: &yavs_kernel::Report) {
            let (t, d) = (hook.now().as_fs(), hook.delta());
            self.0
                .lock()
                .expect("lock")
                .push(format!("{t} {d} {}", r.message));
        }
    }
    let run = |e: &Elaborated, cfg: yavs_kernel::Config| {
        let mut sim = e.build(cfg).expect("build");
        let got = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        sim.subscribe(Box::new(Seen(got.clone())));
        sim.run_until(yavs_kernel::Time::MAX).expect("run");
        got.lock().expect("lock").clone()
    };
    let kernel = run(&e, yavs_kernel::Config::default());
    let interp = run(
        &e,
        yavs_kernel::Config {
            edge_triggers: false,
            ..Default::default()
        },
    );
    assert!(kernel.len() > 140, "{}", kernel.len());
    assert_eq!(kernel, interp);
}

/// Conditional assignments with one condition on a logic element, relational operators and gates
/// with a literal operand (#786) run in the kernel (`ProcInst::mux`, `ProcInst::gate`); the
/// shapes next to them are programs (three arms, a compound condition, an expression arm: #788) or
/// stay processes (numeric_std's `=`, `unaffected`, a guard, `postponed`, a rejection limit,
/// constants only). Reports,
/// with time and delta, are those of the interpreted statements over all value triples
/// (the corpus case `sigassign/mux_gates`).
#[test]
fn muxes_and_relations_run_in_the_kernel() {
    let src = include_str!("../../../tests/corpus/sigassign/mux_gates.vhd");
    let (mut db, mut e) = elab("muxes", src, "tb_mux_gates", &[]);
    let kernel_shapes: Vec<bool> = e
        .procs
        .iter()
        .map(|p| p.gate.is_some() || p.mux.is_some())
        .collect();
    // The inner entity's mux, the 30 muxes of t (m8 has two drivers), the 12 relations, then
    // numeric_std's `=` and `?=` (processes), the 9 gates; then the near shapes (the block's
    // `_guard` before its statement) and the two processes of the testbench.
    let mut want = vec![true; 1 + 30 + 12];
    want.extend([false; 2]);
    want.extend([true; 9]);
    want.extend([false; 12]);
    assert_eq!(kernel_shapes.len(), want.len(), "{kernel_shapes:?}");
    assert_eq!(kernel_shapes, want, "{kernel_shapes:?}");
    e.lower(&mut db).expect("lower");
    struct Seen(std::sync::Arc<std::sync::Mutex<Vec<String>>>);
    impl yavs_kernel::Observer for Seen {
        fn report(&mut self, hook: &mut yavs_kernel::Hook<'_>, r: &yavs_kernel::Report) {
            let (t, d) = (hook.now().as_fs(), hook.delta());
            self.0
                .lock()
                .expect("lock")
                .push(format!("{t} {d} {}", r.message));
        }
    }
    let run = |e: &Elaborated, cfg: yavs_kernel::Config| {
        let mut sim = e.build(cfg).expect("build");
        let got = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        sim.subscribe(Box::new(Seen(got.clone())));
        sim.run_until(yavs_kernel::Time::MAX).expect("run");
        got.lock().expect("lock").clone()
    };
    let kernel = run(&e, yavs_kernel::Config::default());
    let interp = run(
        &e,
        yavs_kernel::Config {
            edge_triggers: false,
            ..Default::default()
        },
    );
    assert!(kernel.len() > 2000, "{}", kernel.len());
    assert_eq!(kernel, interp);
}

/// Concurrent assignments of expression trees, compound conditions and chains of arms run as
/// `yavs_kernel::ProgProcess` (#788); the near shapes next to them stay processes. Reports, with
/// time and delta, are those of the interpreted statements over all value triples (the corpus case
/// `sigassign/prog_exprs`).
#[test]
fn expression_trees_run_in_the_kernel() {
    let src = include_str!("../../../tests/corpus/sigassign/prog_exprs.vhd");
    let (mut db, mut e) = elab("progs", src, "tb_prog_exprs", &[]);
    let shapes: String = e
        .procs
        .iter()
        .map(|p| match (&p.gate, &p.mux, &p.prog) {
            (Some(_), ..) => 'g',
            (_, Some(_), _) => 'm',
            (.., Some(_)) => 'p',
            _ => '.',
        })
        .collect();
    // The inner entity's two, the 34 trees and BOOLEAN trees with delays (the gate `tm <= a and
    // b` is not one of them), the second driver of `tm`, `tq`, the 14 conditions and the 12
    // chains, a mux (the first `mr`) and the second `mr`, the 10 tests of enumeration, integer and character signals; then the near shapes (the block's
    // `_guard` before its statement) and the testbench's two processes.
    let want = format!(
        "{}g{}mp{}{}",
        "p".repeat(34),
        "p".repeat(28),
        "p".repeat(10),
        ".".repeat(11)
    );
    assert_eq!(shapes, want);
    e.lower(&mut db).expect("lower");
    struct Seen(std::sync::Arc<std::sync::Mutex<Vec<String>>>);
    impl yavs_kernel::Observer for Seen {
        fn report(&mut self, hook: &mut yavs_kernel::Hook<'_>, r: &yavs_kernel::Report) {
            let (t, d) = (hook.now().as_fs(), hook.delta());
            self.0
                .lock()
                .expect("lock")
                .push(format!("{t} {d} {}", r.message));
        }
    }
    let run = |e: &Elaborated, cfg: yavs_kernel::Config| {
        let mut sim = e.build(cfg).expect("build");
        let got = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        sim.subscribe(Box::new(Seen(got.clone())));
        sim.run_until(yavs_kernel::Time::MAX).expect("run");
        got.lock().expect("lock").clone()
    };
    let kernel = run(&e, yavs_kernel::Config::default());
    let interp = run(
        &e,
        yavs_kernel::Config {
            edge_triggers: false,
            ..Default::default()
        },
    );
    assert!(kernel.len() > 2000, "{}", kernel.len());
    assert_eq!(kernel, interp);
}

/// The scans for the signals and shared variables of loaded packages skip std and ieee
/// (`walk.rs` `builtin_lib`, #426): none of their packages may declare one.
#[test]
fn builtin_packages_declare_no_signals_or_shared_variables() {
    let uses: String = [
        "std.textio",
        "std.env",
        "ieee.std_logic_1164",
        "ieee.std_logic_textio",
        "ieee.numeric_std",
        "ieee.numeric_bit",
        "ieee.numeric_std_unsigned",
        "ieee.numeric_bit_unsigned",
        "ieee.math_real",
        "ieee.math_complex",
        "ieee.fixed_pkg",
        "ieee.float_pkg",
        "ieee.std_logic_arith",
        "ieee.std_logic_misc",
        "ieee.std_logic_signed",
        "ieee.std_logic_unsigned",
        "ieee.vital_timing",
        "ieee.vital_primitives",
        "ieee.vital_memory",
    ]
    .iter()
    .map(|p| format!("use {p}.all;\n"))
    .collect();
    let src = format!("library ieee;\n{uses}entity t is end; architecture a of t is begin end;");
    for rev in [LangRev::Vhdl2008, LangRev::Vhdl2019] {
        builtin_packages_without_signals(analyze_rev("builtin", &src, rev));
    }
}

fn builtin_packages_without_signals(db: Db) {
    use yavs_sem::decl::{DeclKind, ObjClass};
    let mut packages = 0;
    for (u, unit) in db.units.iter() {
        if !matches!(
            db.libs[unit.lib].source,
            yavs_sem::loader::LibSource::StdDir(_)
        ) {
            continue;
        }
        for region in db.package_regions(u).unwrap_or_default() {
            packages += 1;
            for d in db.decls_with_instances(region) {
                if let DeclKind::Object(o) = &db.decls[d].kind {
                    assert!(
                        !matches!(o.class, ObjClass::Signal | ObjClass::SharedVariable),
                        "a built-in package declares a signal or shared variable: {:?}",
                        db.decls[d].name
                    );
                }
            }
        }
    }
    assert!(
        packages > 20,
        "only {packages} built-in package regions loaded"
    );
}

/// The packages with run-time objects are found when the image is made and come back with it,
/// as long as the design has not grown (#426).
#[test]
fn run_time_packages_come_with_the_image() {
    let src = "package p is signal s : bit; end;
      package q is constant c : integer := 1; end;
      entity t is end; architecture a of t is begin end;";
    let mut db = analyze("rtp", src);
    db.settle();
    let found = db.run_time_package_set();
    assert_eq!(found.len(), 1, "only p has a run-time object");
    assert!(db.stored_run_time_packages().is_none());
    let image = db.to_image().expect("image");
    assert_eq!(db.stored_run_time_packages(), Some(&found[..]));
    let opts = Options::new(LangRev::Vhdl2008, yavs_sem::default_stdlib());
    let image = yavs_sem::image::Image::new(yavs_diag::Buffer::new(image));
    let back = Db::from_image(opts, image, &|_| true).expect("restore");
    assert_eq!(back.stored_run_time_packages(), Some(&found[..]));
    assert_eq!(back.run_time_package_set(), found);
}

/// A subprogram whose external name is bound at elaboration makes its package a run-time one
/// without growing the design: the stored set must not be used then (CI: ename6).
#[test]
fn stored_run_time_packages_are_not_used_with_external_names() {
    let src = "package pack is procedure p; end package;
      package body pack is
        procedure p is
          alias s1 is << signal .t.s : bit >>;
        begin
          s1 <= force '1';
        end procedure;
      end package body;
      entity t is end;
      use work.pack.all;
      architecture a of t is
        signal s : bit;
      begin
        process begin p; wait; end process;
      end;";
    let mut db = analyze("rtp-ext", src);
    db.settle();
    db.to_image().expect("image");
    assert!(db.stored_run_time_packages().is_none());
    // Before the external name is bound the package is not a run-time one, after it is.
    let before = db.run_time_package_set();
    let top = crate::find_top(&mut db, "work", "t").expect("top");
    crate::elaborate(&mut db, top, &[], &ElabConfig::default()).expect("elaborate");
    let after = db.run_time_package_set();
    assert!(after.len() > before.len(), "{before:?} {after:?}");
}

/// The names of the wide leaves that keep only each block's last event (`lean.rs`).
fn lean_leaves(src: &str) -> Vec<String> {
    let (_, e) = elab("lean", src, "t", &[]);
    let mut v: Vec<String> = e
        .lean
        .iter()
        .enumerate()
        .filter(|&(_, &l)| l)
        .map(|(i, _)| e.leaf_name(crate::LeafId(i as u32)))
        .collect();
    v.sort();
    v
}

/// A design of wide signals `a`, `b` and `r` (100 elements); `decls` are architecture
/// declarations, `stmts` the statements of a process with variables `t1`, `e`, `s` and `v`.
fn wide_design(decls: &str, stmts: &str) -> String {
    format!(
        "library ieee; use ieee.std_logic_1164.all;
entity t is end;
architecture x of t is
  signal a, b, r : std_logic_vector(99 downto 0);
  signal i : integer range 0 to 99 := 0;
  {decls}
begin
  process
    variable t1 : time;
    variable e : boolean;
    variable s : std_logic;
    variable v : std_logic_vector(7 downto 0);
    variable w : std_logic_vector(99 downto 0);
  begin
    {stmts}
    wait;
  end process;
end;"
    )
}

const ABR: [&str; 3] = [":t:a", ":t:b", ":t:r"];

#[test]
fn wide_signals_nothing_asks_the_history_of_are_lean() {
    assert_eq!(lean_leaves(&wide_design("", "null;")), ABR);
    // Attributes that need no history of single elements.
    let ok = lean_leaves(&wide_design(
        "",
        "t1 := a'last_event; e := a'event or a(5)'event or a(8 downto 2)'active or b'stable(1 ns);
         t1 := a(7 downto 3)'last_active; s := a(5)'last_value; t1 := b'last_event;",
    ));
    assert_eq!(ok, ABR);
    // The last value of a whole signal.
    assert_eq!(lean_leaves(&wide_design("", "w := a'last_value;")), ABR);
}

#[test]
fn a_part_asked_the_history_of_is_not_lean() {
    for (stmts, who) in [
        ("t1 := a(5)'last_event;", ":t:a"),
        ("t1 := a(7 downto 3)'last_event;", ":t:a"),
        ("t1 := a(i)'last_event;", ":t:a"),
        ("v := a(7 downto 0)'last_value;", ":t:a"),
        ("t1 := b'last_event; t1 := r(0 to 0)'last_event;", ":t:r"),
    ] {
        let l = lean_leaves(&wide_design("", stmts));
        let want: Vec<_> = ABR.into_iter().filter(|&s| s != who).collect();
        assert_eq!(l, want, "{stmts}");
    }
}

#[test]
fn history_through_an_alias_a_field_or_a_port_part() {
    let decls = "alias al : std_logic_vector(49 downto 0) is a(49 downto 0);
      alias whole is r;
      type rec is record n : integer; d : std_logic_vector(99 downto 0); end record;
      signal g : rec;";
    let l = lean_leaves(&wide_design(
        decls,
        "t1 := al'last_event; t1 := whole'last_event; t1 := g.d'last_event;",
    ));
    // The alias of a part marks a, the alias of the whole signal does not mark r, a field marks
    // the record.
    assert_eq!(l, [":t:b", ":t:r"]);
    // An alias with a subtype indication re-bases the index range: marked like a part.
    let l = lean_leaves(&wide_design(
        "alias w2 : std_logic_vector(99 downto 0) is b;",
        "t1 := w2'last_event;",
    ));
    assert_eq!(l, [":t:a", ":t:r"]);
    // A whole formal that is part of its actual marks the actual.
    let src = "library ieee; use ieee.std_logic_1164.all;
entity child is port (d : in std_logic_vector(99 downto 0)); end;
architecture c of child is begin
  process (d) variable t1 : time; begin t1 := d'last_event; end process;
end;
library ieee; use ieee.std_logic_1164.all;
entity t is end;
architecture x of t is
  signal a, b : std_logic_vector(199 downto 0);
begin
  u1 : entity work.child port map (d => a(199 downto 100));
  u2 : entity work.child port map (d => b(99 downto 0));
end;";
    assert_eq!(lean_leaves(src), Vec::<String>::new());
    // Whole signals through ports are whole: the formal is the actual.
    let src = "library ieee; use ieee.std_logic_1164.all;
entity child is port (d : in std_logic_vector(99 downto 0)); end;
architecture c of child is begin
  process (d) variable t1 : time; begin t1 := d'last_event; end process;
end;
library ieee; use ieee.std_logic_1164.all;
entity t is end;
architecture x of t is
  signal a, b : std_logic_vector(99 downto 0);
begin
  u1 : entity work.child port map (d => a);
end;";
    assert_eq!(lean_leaves(src), [":t:a", ":t:b"]);
}

#[test]
fn a_whole_array_of_composites_is_asked_part_by_part() {
    // `m'last_event` of an array of vectors or records is one query per element.
    let decls = "type mem is array (0 to 3) of std_logic_vector(31 downto 0);
      type rec is record n : integer; d : std_logic_vector(99 downto 0); end record;
      type recs is array (0 to 1) of rec;
      signal m : mem; signal rs : recs; signal g : rec;";
    let l = lean_leaves(&wide_design(
        decls,
        "t1 := m'last_event; t1 := rs'last_event; t1 := g'last_event;",
    ));
    // m has 128 elements in one leaf, marked; rs is marked; g is a record of whole leaves.
    assert_eq!(l, [":t:a", ":t:b", ":t:g.d", ":t:r"]);
}

#[test]
fn a_signal_parameter_marks_every_signal_unless_scalar_last_value() {
    let decls = "procedure p (signal s : in std_logic_vector) is variable t1 : time; begin t1 := s'last_event; end;";
    assert!(lean_leaves(&wide_design(decls, "p(a);")).is_empty());
    // The last value of a scalar parameter needs no history (the edge functions of the standard
    // packages are that).
    let decls = "procedure p (signal s : in std_logic) is variable c : std_logic; begin c := s'last_value; end;";
    assert_eq!(lean_leaves(&wide_design(decls, "p(a(3));")), ABR);
}

/// Runs of logic leaves of a record signal share a kernel signal (#686); leaves of another kind,
/// of more than 64 elements, past the 64-element cap, forced or with an implicit signal do not.
#[test]
fn record_leaves_share_kernel_signals() {
    let src = "
      library ieee; use ieee.std_logic_1164.all;
      entity t is end; architecture a of t is
        type rec is record
          a : std_logic; b : std_ulogic; v : std_logic_vector(3 downto 0);
          n : integer; c : bit; w : std_logic_vector(99 downto 0);
          x, y : std_logic_vector(39 downto 0);
        end record;
        signal r, f, s : rec;
      begin
        process begin
          f.a <= force '1';
          report boolean'image(s'stable);
          wait;
        end process;
      end;";
    let locs = |cfg: &ElabConfig| {
        let (_, e) = try_elab_cfg("packs", src, "t", &[], cfg);
        let e = e.expect("elaborates");
        let obj = |name: &str| {
            (e.nets.objs.iter())
                .find(|o| o.name.text.as_str() == name)
                .expect("object")
                .leaves
        };
        let at = |(a, b): (crate::LeafId, crate::LeafId)| -> Vec<(u32, u32)> {
            (a.0..b.0)
                .map(|l| {
                    let at = e.nets.loc[l as usize];
                    (at.signal.0, at.base)
                })
                .collect()
        };
        (at(obj("r")), at(obj("f")), at(obj("s")))
    };
    let (r, f, s) = locs(&ElabConfig::default());
    let k = r[0].0;
    // a, b, v together; n alone; c starts a pack that w (100 elements) cannot join; x, y apart.
    assert_eq!(
        r,
        [
            (k, 0),
            (k, 1),
            (k, 2),
            (k + 1, 0),
            (k + 2, 0),
            (k + 3, 0),
            (k + 4, 0),
            (k + 5, 0)
        ]
    );
    for (i, w) in f.windows(2).chain(s.windows(2)).enumerate() {
        assert_ne!(
            w[0].0, w[1].0,
            "pair {i}: a forced leaf or an implicit prefix shares a signal"
        );
    }
    let off = ElabConfig {
        packs: false,
        ..ElabConfig::default()
    };
    let (r, _, _) = locs(&off);
    assert!(r.windows(2).all(|w| w[0].0 + 1 == w[1].0 && w[1].1 == 0));
}
