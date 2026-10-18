//! Elaboration tests on real VHDL, analyzed into `~/.cache/rsim-elab/unit-tests`.

use std::path::PathBuf;

use yavs_kernel::{Elem, PortMode, Range};
use yavs_sem::{Db, LangRev, Options};

use crate::{Case, ElabConfig, Elaborated, ScopeKind, Sel, codes};

fn dir(name: &str) -> PathBuf {
    let home = std::env::var_os("HOME").expect("HOME");
    let d = PathBuf::from(home)
        .join(".cache/rsim-elab/unit-tests")
        .join(name);
    std::fs::create_dir_all(&d).expect("scratch dir");
    d
}

fn analyze(name: &str, src: &str) -> Db {
    let p = dir(name).join("t.vhd");
    std::fs::write(&p, src).expect("write");
    let mut db = Db::new(Options::new(LangRev::Vhdl2008, yavs_sem::default_stdlib()));
    let r = db.analyze_file("work", &p, LangRev::Vhdl2008);
    let msgs: Vec<String> = db
        .diagnostics()
        .iter()
        .map(|d| format!("{} {}", d.code, d.message))
        .collect();
    assert!(r.is_ok() && msgs.is_empty(), "analysis: {msgs:#?}");
    db
}

type Errs = Vec<(&'static str, String)>;

fn try_elab(
    name: &str,
    src: &str,
    top: &str,
    gens: &[(&str, &str)],
) -> (Db, Result<Elaborated, Errs>) {
    let mut db = analyze(name, src);
    let gens: Vec<(String, String)> = gens
        .iter()
        .map(|(a, b)| (a.to_string(), b.to_string()))
        .collect();
    let r = crate::find_top(&mut db, "work", top)
        .and_then(|t| crate::elaborate(&mut db, t, &gens, &ElabConfig::default()))
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
    let (_, e) = elab("generics", GENERICS, "gtop", &[]);
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
    let (db, e) = elab("binding", BINDING, "btop", &[]);
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

    let (db, e) = elab("binding_cfg", BINDING, "cfg", &[]);
    assert!(e.top.config.is_some());
    assert_eq!(arch_of(&db, &e, "u1"), "rtl");
    assert_eq!(arch_of(&db, &e, "u2"), "rtl");

    let (db, e) = elab("binding_arch", BINDING, "inv.rtl", &[]);
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
    let (_, a) = elab("restart", BINDING, "btop", &[]);
    let (_, b) = elab("restart", BINDING, "btop", &[]);
    assert_eq!(procs(&a), procs(&b));
    assert_eq!(sig_paths(&a), sig_paths(&b));
    assert_eq!(a.nets.ports, b.nets.ports);
    let first = run(&a);
    assert_eq!(first, run(&a));
    assert_eq!(first, run(&b));
    assert!(
        first.iter().any(|s| s.starts_with(":btop:u1:a")),
        "{first:?}"
    );
}

struct Collect(std::sync::Arc<std::sync::Mutex<Vec<String>>>);
impl yavs_kernel::Observer for Collect {
    fn report(&mut self, hook: &mut yavs_kernel::Hook<'_>, r: &yavs_kernel::Report) {
        self.0.lock().expect("lock").push(format!(
            "{} {} {}",
            hook.now().as_fs(),
            r.process.0,
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
    let (_, e) = elab("pkgsig", src, "t", &[]);
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
    let (_, e) = elab("blockports", src, "t", &[]);
    assert_eq!(sig_paths(&e), [":t:x", ":t:b:p", ":t:b:inner"]);
    assert_eq!(e.nets.leaves[2].ty.len, 8);
    assert_eq!(e.nets.ports.len(), 1);
}

/// Incremental binding (LRM 7.3.2.3): a configuration's generic map over the default binding
/// (#75). The parser does not take a binding indication without an
/// entity aspect yet, so the test removes the aspect from the analyzed HIR.
#[test]
fn incremental_binding() {
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
          for u1 : inv use entity work.inv generic map (W => 3); end for;
        end for;
      end;";
    let mut db = analyze("incremental", src);
    let top = crate::find_top(&mut db, "work", "cfg").expect("top");
    let cu = top.config.expect("configuration");
    let yavs_sem::hir::UnitHir::Configuration(b) = &mut db.units[cu].hir else {
        panic!("configuration")
    };
    for it in &mut b.items {
        if let yavs_sem::hir::ConfigItem::Component(c) = it {
            c.binding.as_mut().expect("binding").aspect = None;
        }
    }
    let e = crate::elaborate(&mut db, top, &[], &ElabConfig::default()).expect("elab");
    let lens: Vec<u32> = e.nets.leaves.iter().map(|l| l.ty.len).collect();
    // u1: W from the incremental map; u2 (bound by the specification): the default.
    assert_eq!(lens, [3, 1]);
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
