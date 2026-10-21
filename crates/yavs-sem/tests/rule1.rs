//! Rule 1 review (docs/plan.md rule 1, sem.md section 2, `docs/design/rule1-review.md`):
//! constraints are runtime data on every composite, with one subtype representation.
//! Each test analyzes a small design written for the review and asserts the shape of the
//! HIR subtypes: where every bound comes from, which bounds are folded, element
//! constraints and resolution. `dump` prints the compact form the review document quotes.
//! Tests marked `ignore` assert the correct shape for a finding of the review (issue in
//! the reason). Case 6 (real code) needs `--features corpus` and tools/fetch-corpus.sh.

use std::path::PathBuf;

use yavs_diag::{MessageFormat, render};
use yavs_sem::decl::{Alias, DeclKind};
use yavs_sem::hir::AttrPrefix;
use yavs_sem::ids::{DeclId, ExprId, SubtypeId};
use yavs_sem::types::{Constraint, RangeC, Resolution};
use yavs_sem::{Db, LangRev, Options, default_stdlib};

/// Analyze `(library, source)` in order into a fresh `Db`; every file must be clean.
fn analyze(test: &str, files: &[(&str, &str)]) -> Db {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("rule1_{test}"));
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let mut db = Db::new(Options::new(LangRev::Vhdl2008, default_stdlib()));
    for (i, (lib, src)) in files.iter().enumerate() {
        let path = dir.join(format!("f{i}.vhd"));
        std::fs::write(&path, src).expect("write source");
        let r = db.analyze_file(lib, &path, LangRev::Vhdl2008);
        let text: String = db
            .diagnostics()
            .iter()
            .map(|d| render(&db.sources, d, MessageFormat::Human, false))
            .collect();
        assert!(
            r.is_ok() && db.diagnostics().is_empty(),
            "{test} f{i}:\n{text}"
        );
    }
    db
}

/// The source text of an expression, whitespace collapsed.
fn src(db: &Db, e: ExprId) -> String {
    let sp = db.exprs[e].span;
    let b = &db.sources.get(sp.file).bytes()[sp.start as usize..sp.end as usize];
    String::from_utf8_lossy(b)
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// A bound: its folded value, or `{source}` when it is evaluated at run time.
fn bound(db: &Db, e: ExprId) -> String {
    use yavs_sem::stat::ConstValue;
    match db.value(e) {
        Some(ConstValue::Int(v)) => v.to_string(),
        Some(ConstValue::Enum(v)) => format!("pos {v}"),
        Some(v) => format!("{v:?}"),
        None => format!("{{{}}}", src(db, e)),
    }
}

fn range(db: &Db, r: &RangeC) -> String {
    match r {
        RangeC::Explicit { left, dir, right } => {
            let d = match dir {
                yavs_sem::types::Direction::To => "to",
                yavs_sem::types::Direction::Downto => "downto",
            };
            format!("{} {d} {}", bound(db, *left), bound(db, *right))
        }
        RangeC::Attr {
            prefix,
            reverse,
            dim,
        } => {
            let p = match prefix {
                AttrPrefix::Expr(e) => src(db, *e),
                AttrPrefix::Subtype(st) => db.subtype_name(*st),
                AttrPrefix::Entity(d) => db.decls[*d].name.to_string(),
            };
            let a = if *reverse { "reverse_range" } else { "range" };
            format!("{{{p}'{a}({dim})}}")
        }
    }
}

/// The constraint of type `t`, record elements by name.
fn constraint(db: &Db, st: SubtypeId, c: &Constraint) -> String {
    use yavs_sem::types::TypeKind;
    let t = db.base(st);
    match c {
        Constraint::None => String::new(),
        Constraint::Range(r) => format!(" range {}", range(db, r)),
        Constraint::Array { dims, elem } => {
            let d = match dims {
                None => "(open)".to_string(),
                Some(ds) => format!(
                    "({})",
                    ds.iter()
                        .map(|r| range(db, r))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            };
            let e = match (&db.types[t].kind, &**elem) {
                (_, Constraint::None) => String::new(),
                (TypeKind::Array { elem: es, .. }, c) => constraint(db, *es, c),
                _ => "?".into(),
            };
            let e = e
                .strip_prefix(" range ")
                .map_or(e.clone(), |r| format!("[{r}]"));
            format!("{d}{e}")
        }
        Constraint::Record { elems } => {
            let TypeKind::Record { elems: decl } = &db.types[t].kind else {
                return "?".into();
            };
            let parts: Vec<String> = elems
                .iter()
                .map(|(i, c)| {
                    let e = &decl[*i as usize];
                    format!("{}{}", e.name.text, constraint(db, e.subtype, c))
                })
                .collect();
            format!("({})", parts.join(", "))
        }
        Constraint::FromObject { expr, elem_depth } => {
            format!(" from-object({}, depth {elem_depth})", src(db, *expr))
        }
    }
}

fn resolution(db: &Db, r: &Resolution) -> String {
    match r {
        Resolution::Function(f) => db.decls[*f].name.to_string(),
        Resolution::Element(r) => format!("({})", resolution(db, r)),
        Resolution::Record(rs) => format!(
            "({})",
            rs.iter()
                .map(|(i, r)| format!("#{i} {}", resolution(db, r)))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

/// `[resolution] base(constraint) [constrainedness]`.
fn subtype(db: &Db, st: SubtypeId) -> String {
    let s = &db.subtypes[st];
    let res = s
        .resolution
        .as_ref()
        .map_or(String::new(), |r| format!("{} ", resolution(db, r)));
    let c = constraint(db, st, &s.constraint);
    format!(
        "{res}{}{c} [{:?}]",
        db.type_name(db.base(st)),
        db.constrainedness(st)
    )
}

/// The user declarations named `name` (folded), in the test's own files.
fn decls(db: &Db, name: &str) -> Vec<DeclId> {
    db.decls
        .iter()
        .filter(|(_, d)| d.name.key().as_str() == name)
        .filter(|(_, d)| {
            let p = &db.sources.get(d.span.file).path;
            p.contains("rule1_")
        })
        .map(|(id, _)| id)
        .collect()
}

/// The one declaration named `name`, dumped.
fn dump(db: &Db, name: &str) -> String {
    let ds = decls(db, name);
    let [d] = ds.as_slice() else {
        panic!("{name}: {} declarations", ds.len());
    };
    dump_decl(db, *d)
}

fn dump_decl(db: &Db, d: DeclId) -> String {
    match &db.decls[d].kind {
        DeclKind::Object(o) => format!(
            "{:?} {:?}: {}, bounds from {:?}",
            o.role,
            o.class,
            subtype(db, o.subtype),
            o.bounds
        ),
        DeclKind::Subtype(st) => format!("subtype: {}", subtype(db, *st)),
        DeclKind::Type(t) => format!("type: {}", subtype(db, db.types[*t].first_subtype)),
        DeclKind::Alias(Alias::Object { target, subtype: s }) => format!(
            "alias of {} : {}, own subtype {}",
            src(db, *target),
            subtype(db, db.exprs[*target].subtype),
            s.map_or("none".into(), |s| subtype(db, s))
        ),
        DeclKind::Subprogram(s) => format!(
            "subprogram: return {}",
            s.ret.map_or("none".into(), |r| subtype(db, r))
        ),
        _ => "other".into(),
    }
}

/// An expression: its subtype, plus how an aggregate gets its bounds.
fn expr(db: &Db, e: ExprId) -> String {
    use yavs_sem::hir::{AggBounds, ExprKind};
    let agg = match &db.exprs[e].kind {
        ExprKind::Aggregate { bounds, .. } => match bounds {
            AggBounds::Context => " agg-bounds context",
            AggBounds::Positional { .. } => " agg-bounds positional",
            AggBounds::Named { .. } => " agg-bounds named",
            AggBounds::Concat => " agg-bounds concat",
            AggBounds::Record => " agg-bounds record",
        },
        _ => "",
    };
    let folded = if db.exprs[e].value.is_some() {
        " folded"
    } else {
        ""
    };
    format!("{}{agg}{folded}", subtype(db, db.exprs[e].subtype))
}

/// Every declaration, assignment, return and association in the test's files, dumped.
fn dump_all(db: &Db) -> Vec<String> {
    use yavs_sem::hir::{Actual, Rhs, StmtKind, Target, Waveform};
    let ours = |sp: yavs_diag::Span| db.sources.get(sp.file).path.contains("rule1_");
    let mut out = Vec::new();
    for (d, decl) in db.decls.iter() {
        if ours(decl.span) && !decl.implicit && !matches!(dump_decl(db, d).as_str(), "other") {
            out.push(format!("{}: {}", decl.name, dump_decl(db, d)));
        }
    }
    let args = |out: &mut Vec<String>, what: &str, m: &[yavs_sem::hir::Arg]| {
        for a in m {
            if let Actual::Expr(e) = &a.actual {
                out.push(format!(
                    "{what} #{} => {}: {}",
                    a.formal,
                    src(db, *e),
                    expr(db, *e)
                ));
            }
        }
    };
    for (_, s) in db.stmts.iter() {
        let tgt = |t: &Target| format!("{} : {}", src(db, t.expr), expr(db, t.expr));
        match &s.kind {
            StmtKind::VarAssign {
                target,
                rhs: Rhs::Simple(e),
            } => out.push(format!(
                "{} := {}: {}",
                tgt(target),
                src(db, *e),
                expr(db, *e)
            )),
            StmtKind::SignalAssign {
                target,
                rhs: Rhs::Simple(Waveform::Elements(w)),
                ..
            }
            | StmtKind::ConcSignalAssign {
                target,
                rhs: Rhs::Simple(Waveform::Elements(w)),
                ..
            } => {
                for el in w.iter() {
                    if let Some(e) = el.value {
                        out.push(format!(
                            "{} <= {}: {}",
                            tgt(target),
                            src(db, e),
                            expr(db, e)
                        ));
                    }
                }
            }
            StmtKind::Return(Some(e)) => {
                out.push(format!("return {}: {}", src(db, *e), expr(db, *e)))
            }
            StmtKind::Instance {
                port_map,
                generic_map,
                ..
            } => {
                args(&mut out, "generic", generic_map);
                args(&mut out, "port", port_map);
            }
            _ => {}
        }
    }
    out
}

// -- case 1: records with unconstrained array elements ----------------------

const RECORDS: &str = "
library ieee; use ieee.std_logic_1164.all;
package rec_pkg is
  type rec_t is record
    data : std_ulogic_vector;
    keep : std_ulogic_vector;
    last : std_ulogic;
  end record;
  subtype rec_full_t is rec_t(data(7 downto 0), keep(0 to 0));
  subtype rec_part_t is rec_t(data(open), keep(0 to 0));
  subtype rec_part2_t is rec_part_t(data(15 downto 0));
  type outer_t is record
    hdr : rec_t;
    payload : std_ulogic_vector;
  end record;
  subtype outer_full_t is outer_t(hdr(data(3 downto 0), keep(0 to 0)), payload(31 downto 0));
  constant c_full : rec_full_t := (data => x\"00\", keep => \"1\", last => '0');
  constant c_init : rec_t := (data => x\"0000\", keep => \"11\", last => '1');
  constant c_outer : outer_full_t := (hdr => (data => x\"0\", keep => \"0\", last => '0'),
                                      payload => x\"00000000\");
end package;

library ieee; use ieee.std_logic_1164.all;
use work.rec_pkg.all;
entity rec_ent is
  generic (W : positive := 8);
  port (p_full : in rec_full_t; p_part : in rec_part_t; p_un : out rec_t);
end entity;
architecture a of rec_ent is
  signal s_full, s_full2 : rec_full_t;
  signal s_dyn : rec_t(data(W - 1 downto 0), keep(W / 8 - 1 downto 0));
  signal s_outer : outer_full_t;
begin
  s_full <= (data => x\"00\", keep => \"1\", last => '0');
  s_full2.data <= (s_full2.data'range => '0');
  s_dyn <= (data => (W - 1 downto 0 => '0'), keep => (W / 8 - 1 downto 0 => '1'), last => '0');
  p_un.data <= (p_un.data'range => '0');
  process
    variable v_full : rec_full_t;
  begin
    v_full := p_full;
    v_full.keep := p_part.keep;
    wait;
  end process;
end architecture;
";

// -- case 2: arrays of unconstrained arrays ------------------------------------

const ARRAYS: &str = "
library ieee; use ieee.std_logic_1164.all;
package arr_pkg is
  type slvv_t is array (natural range <>) of std_ulogic_vector;
  subtype byte_vec_t is slvv_t(open)(7 downto 0);
  subtype four_t is slvv_t(0 to 3)(open);
  subtype four_bytes_t is slvv_t(0 to 3)(7 downto 0);
  subtype four_bytes2_t is byte_vec_t(0 to 3);
  subtype four_bytes3_t is four_t(open)(7 downto 0);
  type mat_t is array (natural range <>, natural range <>) of std_ulogic_vector;
  subtype mat23_t is mat_t(0 to 1, 0 to 2)(3 downto 0);
  type cube_t is array (natural range <>) of slvv_t;
  subtype cube_full_t is cube_t(0 to 1)(0 to 3)(7 downto 0);
  subtype cube_mid_t is cube_t(open)(0 to 3)(open);
  constant c_rows : slvv_t(0 to 1) := (\"UX01\", \"ZWLH\");
  constant c_all : slvv_t := (x\"12\", x\"34\", x\"56\");
end package;

library ieee; use ieee.std_logic_1164.all;
use work.arr_pkg.all;
entity arr_ent is
  generic (N : positive := 4; W : positive := 8);
  port (rows : in slvv_t; bytes : in byte_vec_t; m : out mat23_t);
end entity;
architecture a of arr_ent is
  signal s_ram : slvv_t(0 to N - 1)(W - 1 downto 0);
  signal s_four : four_bytes2_t;
  signal s_cube : cube_full_t;
begin
  s_ram <= (others => (others => '0'));
  s_four(0) <= bytes(0);
  s_cube(1)(2) <= x\"FF\";
end architecture;
";

// -- case 3: interface objects, returns, constants, aliases --------------------

const INTERFACES: &str = "
library ieee; use ieee.std_logic_1164.all;
package if_pkg is
  function invert(x : std_ulogic_vector) return std_ulogic_vector;
  procedure fill(o : out std_ulogic_vector; v : std_ulogic);
  procedure flip(io : inout std_ulogic_vector);
  constant c_word : std_ulogic_vector := x\"AB\";
end package;
package body if_pkg is
  function invert(x : std_ulogic_vector) return std_ulogic_vector is
    variable r : std_ulogic_vector(x'range);
  begin
    r := not x;
    return r;
  end function;
  procedure fill(o : out std_ulogic_vector; v : std_ulogic) is
  begin
    o := (o'range => v);
  end procedure;
  procedure flip(io : inout std_ulogic_vector) is
    constant tmp : std_ulogic_vector := io;
  begin
    io := invert(tmp);
  end procedure;
end package body;

library ieee; use ieee.std_logic_1164.all;
use work.if_pkg.all;
entity leaf is
  generic (g_init : std_ulogic_vector);
  port (d : in std_ulogic_vector; q : out std_ulogic_vector);
end entity;
architecture a of leaf is
  alias a_all is d;
  alias a_rev : std_ulogic_vector(0 to d'length - 1) is d;
  alias a_un : std_ulogic_vector is d;
begin
  q <= invert(a_all) xor g_init;
end architecture;

library ieee; use ieee.std_logic_1164.all;
use work.if_pkg.all;
entity top3 is
end entity;
architecture a of top3 is
  signal sd, sq : std_ulogic_vector(7 downto 0);
begin
  u : entity work.leaf generic map (g_init => x\"0F\") port map (d => sd, q => sq);
  process
    variable v : std_ulogic_vector(3 downto 0);
  begin
    fill(v, '1');
    flip(v);
    wait;
  end process;
end architecture;
";

// -- case 4: subtype attributes and generic types ------------------------------

const ATTRIBUTES: &str = "
package gen_pkg is
  generic (type t);
  function ident(x : t) return t;
end package;
package body gen_pkg is
  function ident(x : t) return t is
  begin
    return x;
  end function;
end package body;

library ieee; use ieee.std_logic_1164.all;
package inst_un is new work.gen_pkg generic map (t => std_ulogic_vector);
library ieee; use ieee.std_logic_1164.all;
use work.rec_pkg.all;
use work.arr_pkg.all;
entity attrs is
  port (bus_p : inout rec_t; rows : in slvv_t);
end entity;
architecture a of attrs is
  signal s_bus : bus_p'subtype;
  signal s_data : bus_p.data'subtype;
  signal s_row : rows'element;
  signal s_rng : std_ulogic_vector(bus_p.data'range);
  signal s_full : rec_full_t;
  signal s_same : s_full'subtype;
  constant c_len : natural := s_row'length;
begin
end architecture;
";

// -- case 5: the tracker reproducers (written fresh) ----------------------------

/// sim2 #641: `'subtype` and `'range` of an element of a record port (the alias of the
/// element is finding F6, `SIM2_641_ALIAS`).
const SIM2_641: &str = "
library ieee; use ieee.std_logic_1164.all;
package bus641_pkg is
  type bus_rec_t is record
    address : std_logic_vector;
    data : std_logic_vector;
  end record;
end package;
library ieee; use ieee.std_logic_1164.all;
use work.bus641_pkg.all;
entity e641 is
  port (axi : inout bus_rec_t);
end entity;
architecture a of e641 is
  signal address : std_logic_vector(axi.address'range);
  signal data2 : axi.data'subtype;
begin
end architecture;
";

/// sim2 #1224: array of unconstrained vectors, element bounds from the initial value.
const SIM2_1224: &str = "
library ieee; use ieee.std_logic_1164.all;
entity e1224 is
end entity;
architecture a of e1224 is
  type mat_t is array (natural range <>) of std_logic_vector;
begin
  process
    constant m : mat_t(0 to 1) := (\"UX01Z\", \"WLH-0\");
  begin
    for i in m'range loop
      for j in m(i)'range loop
        report std_logic'image(m(i)(j));
      end loop;
    end loop;
    report integer'image(m(0)'length);
    wait;
  end process;
end architecture;
";

/// sim2 #2422: a type conversion on an unconstrained `out` formal.
const SIM2_2422: &str = "
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity comp2422 is
  port (o : out unsigned);
end entity;
architecture a of comp2422 is
begin
  o <= (o'range => '0');
end architecture;
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity top2422 is
end entity;
architecture a of top2422 is
  signal sig : std_logic_vector(7 downto 0);
begin
  inst : entity work.comp2422 port map (std_logic_vector(o) => sig);
end architecture;
";

/// sim2 #2179: `others` in an aggregate for an element of a record port whose element
/// is unconstrained, and for a record signal declared with `'subtype`.
const SIM2_2179: &str = "
package p2179 is
  type rec is record
    elem : bit_vector;
  end record;
end package;
entity i2179 is
  port (p : out work.p2179.rec);
end entity;
architecture a of i2179 is
  signal s1 : work.p2179.rec(elem(3 downto 0));
  signal s2 : s1'subtype;
begin
  s1.elem <= (others => '0');
  s2.elem <= (others => '0');
  p.elem <= (others => '0');
end architecture;
entity e2179 is
end entity;
architecture a of e2179 is
  signal s : work.p2179.rec(elem(7 downto 0));
begin
  inst : entity work.i2179 port map (p => s);
end architecture;
";

/// sim1 #1589: `others` in a record aggregate covering vectors of different widths, after
/// a named nested record element.
const SIM1_1589: &str = "
library ieee; use ieee.std_logic_1164.all;
entity e1589 is
end entity;
architecture a of e1589 is
  type inner_t is record
    i : std_logic_vector(3 downto 0);
  end record;
  constant c_inner : inner_t := (i => (others => '0'));
  type r_t is record
    a : std_logic_vector(5 downto 0);
    x : std_logic;
    b : std_logic_vector(9 downto 0);
    inner : inner_t;
    c : std_logic_vector(31 downto 0);
  end record;
  constant c_r : r_t := (x => '0', inner => c_inner, others => (others => '1'));
  signal s : r_t := c_r;
begin
  assert c_r.b = \"1111111111\" and c_r.c = x\"FFFFFFFF\" report \"wrong widths\" severity failure;
end architecture;
";

/// sim1 #1543: an external name alias with an unconstrained subtype; `'element'length`.
const SIM1_1543: &str = "
library ieee; use ieee.std_logic_1164.all;
package p1543 is
  type ram_t is array (natural range <>) of std_logic_vector;
end package;
library ieee; use ieee.std_logic_1164.all;
use work.p1543.all;
entity inner1543 is
  port (content : out ram_t);
end entity;
architecture a of inner1543 is
  signal ram : ram_t(0 to 15)(31 downto 0) := (others => (others => '0'));
begin
  content <= ram;
end architecture;
library ieee; use ieee.std_logic_1164.all;
use work.p1543.all;
entity outer1543 is
end entity;
architecture a of outer1543 is
  signal content : ram_t(4 to 19)(63 downto 32);
begin
  u : entity work.inner1543 port map (content => content);
  blk : block
    alias ram_un is << signal .outer1543.u.ram : ram_t >>;
    alias ram_c is << signal .outer1543.u.ram : ram_t(0 to 15)(31 downto 0) >>;
    constant width : natural := ram_un'element'length;
  begin
  end block;
end architecture;
";

/// sim1 #1576: an allocator of a record whose element is an array of unconstrained arrays.
const SIM1_1576: &str = "
library ieee; use ieee.std_logic_1164.all;
entity e1576 is
end entity;
architecture a of e1576 is
  type data_t is array (natural range <>) of std_ulogic_vector;
  type container_t is record
    data : data_t;
  end record;
  type container_ptr_t is access container_t;
begin
  process
    variable container : container_ptr_t;
    constant cap : positive := 4;
    subtype container_c_t is container_t(data(0 to cap - 1)(7 downto 0));
  begin
    container := new container_c_t;
    container.data(1) := x\"02\";
    wait;
  end process;
end architecture;
";

/// sim1 #1463: an array of records whose element is constrained in the port.
const SIM1_1463: &str = "
package p1463 is
  type int_arr_t is array (integer range <>) of integer;
  type my_rec_t is record
    data : int_arr_t;
  end record;
  type rec_arr_t is array (integer range <>) of my_rec_t;
end package;
use work.p1463.all;
entity e1463 is
  port (sig : in rec_arr_t(0 to 1)(data(0 to 1)));
end entity;
architecture a of e1463 is
  signal first : integer;
begin
  first <= sig(0).data(0);
end architecture;
";

fn value_shape(v: &yavs_sem::stat::ConstValue) -> String {
    use yavs_sem::stat::{ConstValue, Elems};
    match v {
        ConstValue::Array(a) => {
            let b: Vec<String> = a
                .bounds
                .iter()
                .map(|(l, d, r)| {
                    format!(
                        "{l} {} {r}",
                        if *d == yavs_sem::types::Direction::To {
                            "to"
                        } else {
                            "downto"
                        }
                    )
                })
                .collect();
            match &a.elems {
                Elems::Bytes(_) => format!("[{}]", b.join(", ")),
                Elems::Runs(rs) => format!(
                    "[{}]({})",
                    b.join(", "),
                    rs.iter()
                        .map(|(n, v)| format!("{n} x {}", value_shape(v)))
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
            }
        }
        ConstValue::Record(es) => format!(
            "({})",
            es.iter().map(value_shape).collect::<Vec<_>>().join(", ")
        ),
        _ => "s".into(),
    }
}

// -- sources for the findings ---------------------------------------------------

/// F2: `others` for record aggregate elements whose bounds the context's record
/// constraint gives (all three reference simulators accept it).
const RECORD_AGG_CONTEXT: &str = "
library ieee; use ieee.std_logic_1164.all;
use work.rec_pkg.all;
entity f2_ent is
end entity;
architecture a of f2_ent is
  signal s_outer : outer_full_t;
  constant c_outer2 : outer_full_t := (hdr => (data => x\"0\", keep => \"0\", last => '0'),
                                       payload => (others => '0'));
begin
  s_outer <= (hdr => (data => (others => '1'), keep => \"1\", last => '0'), payload => (others => '0'));
end architecture;
";

/// F3: `others` for a selected element of a fully constrained record signal.
const FIELD_OTHERS: &str = "
library ieee; use ieee.std_logic_1164.all;
use work.rec_pkg.all;
entity f3_ent is
end entity;
architecture a of f3_ent is
  signal s_full : rec_full_t;
  constant c_len : natural := s_full.data'length;
begin
  s_full.data <= (others => '0');
end architecture;
";

/// F5: a constrained subtype indication as a generic type actual.
const GENERIC_CONSTRAINED_ACTUAL: &str = "
library ieee; use ieee.std_logic_1164.all;
package inst_c is new work.gen_pkg generic map (t => std_ulogic_vector(7 downto 0));
";

/// F4: attributes whose prefix is an attribute (`'element'range`, `T'base'high`).
const ATTRIBUTE_OF_ATTRIBUTE: &str = "
library ieee; use ieee.std_logic_1164.all;
use work.arr_pkg.all;
entity f4_ent is
  port (rows : in slvv_t);
end entity;
architecture a of f4_ent is
  signal s_copy : slvv_t(rows'range)(rows'element'range);
  constant c_len : natural := rows'element'length;
  constant c_hi : natural := natural'base'high;
begin
end architecture;
";

/// F6: an alias of a record element (the sim2 #641 alias).
const SIM2_641_ALIAS: &str = "
library ieee; use ieee.std_logic_1164.all;
use work.bus641_pkg.all;
entity e641b is
  port (axi : inout bus_rec_t);
end entity;
architecture a of e641b is
  alias axi_data is axi.data;
  signal data1 : axi_data'subtype;
begin
end architecture;
";

/// Unconstrained `out` parameter assigned an `others` aggregate (LRM: an error; all three
/// reference simulators reject it).
const OUT_PARAM_OTHERS: &str = "
library ieee; use ieee.std_logic_1164.all;
package out_pkg is
  procedure clear(o : out std_ulogic_vector);
end package;
package body out_pkg is
  procedure clear(o : out std_ulogic_vector) is
  begin
    o := (others => '0');
  end procedure;
end package body;
";

/// F7: a signal of a generic type, instantiated with an unconstrained actual.
const GENERIC_SIGNAL: &str = "
package gsig_pkg is
  generic (type t);
  signal s : t;
end package;
library ieee; use ieee.std_logic_1164.all;
package gsig_un is new work.gsig_pkg generic map (t => std_ulogic_vector);
";

/// Analyze, expecting errors: `(code, line)` of every diagnostic.
fn errors(test: &str, files: &[(&str, &str)]) -> Vec<(String, u32)> {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(format!("rule1_{test}"));
    std::fs::create_dir_all(&dir).expect("tmp dir");
    let mut db = Db::new(Options::new(LangRev::Vhdl2008, default_stdlib()));
    for (i, (lib, src)) in files.iter().enumerate() {
        let path = dir.join(format!("f{i}.vhd"));
        std::fs::write(&path, src).expect("write source");
        let _ = db.analyze_file(lib, &path, LangRev::Vhdl2008);
    }
    db.diagnostics()
        .iter()
        .map(|d| {
            let sp = d.primary.as_ref().expect("primary span").span;
            let line = db.sources.get(sp.file).line_col(sp.start).0;
            (d.code.to_string(), line as u32)
        })
        .collect()
}

/// Compare `name -> dump` pairs, reporting every mismatch with the actual dump.
fn check(db: &Db, want: &[(&str, &str)]) {
    let bad: Vec<String> = want
        .iter()
        .filter_map(|(n, w)| {
            let got = dump(db, n);
            (got != *w).then(|| format!("(\"{n}\", \"{got}\"),"))
        })
        .collect();
    assert!(bad.is_empty(), "mismatches:\n{}", bad.join("\n"));
}

/// The lines of `dump_all` that start with `prefix`.
fn lines(db: &Db, prefix: &str) -> Vec<String> {
    dump_all(db)
        .into_iter()
        .filter(|l| l.starts_with(prefix))
        .collect()
}

// -- the tests: shapes yavs produces (cases 1 to 5) ------------------------------

/// Case 1: record element constraints are merged once at the subtype declaration,
/// partial ones keep `open`, and ports with a not fully constrained subtype take their
/// bounds from the actual. Static bounds are folded, `W - 1` stays an expression.
#[test]
fn records_with_unconstrained_elements() {
    let db = analyze("records", &[("work", RECORDS)]);
    check(
        &db,
        &[
            ("rec_t", "type: rec_t [Unconstrained]"),
            (
                "rec_full_t",
                "subtype: rec_t(data(7 downto 0), keep(0 to 0)) [Full]",
            ),
            (
                "rec_part_t",
                "subtype: rec_t(data(open), keep(0 to 0)) [Partial]",
            ),
            (
                "rec_part2_t",
                "subtype: rec_t(data(15 downto 0), keep(0 to 0)) [Full]",
            ),
            (
                "outer_full_t",
                "subtype: outer_t(hdr(data(3 downto 0), keep(0 to 0)), payload(31 downto 0)) [Full]",
            ),
            (
                "c_full",
                "Plain Constant: rec_t(data(7 downto 0), keep(0 to 0)) [Full], bounds from Subtype",
            ),
            (
                "c_init",
                "Plain Constant: rec_t [Unconstrained], bounds from Initializer",
            ),
            (
                "p_full",
                "Port Signal: rec_t(data(7 downto 0), keep(0 to 0)) [Full], bounds from Subtype",
            ),
            (
                "p_part",
                "Port Signal: rec_t(data(open), keep(0 to 0)) [Partial], bounds from Actual",
            ),
            (
                "p_un",
                "Port Signal: rec_t [Unconstrained], bounds from Actual",
            ),
            (
                "s_dyn",
                "Plain Signal: rec_t(data({W - 1} downto 0), keep({W / 8 - 1} downto 0)) [Full], bounds from Subtype",
            ),
            (
                "s_outer",
                "Plain Signal: outer_t(hdr(data(3 downto 0), keep(0 to 0)), payload(31 downto 0)) [Full], bounds from Subtype",
            ),
        ],
    );
    // Record aggregates: bounds `Record`, element values carry their own bounds.
    assert_eq!(
        lines(&db, "s_dyn :"),
        [
            "s_dyn : rec_t(data({W - 1} downto 0), keep({W / 8 - 1} downto 0)) [Full] <= \
          (data => (W - 1 downto 0 => '0'), keep => (W / 8 - 1 downto 0 => '1'), last => '0'): \
          rec_t(data({W - 1} downto 0), keep({W / 8 - 1} downto 0)) [Full] agg-bounds record"
        ]
    );
    assert_eq!(
        lines(&db, "s_full :"),
        [
            "s_full : rec_t(data(7 downto 0), keep(0 to 0)) [Full] <= (data => x\"00\", keep => \"1\", \
          last => '0'): rec_t(data(7 downto 0), keep(0 to 0)) [Full] agg-bounds record folded"
        ]
    );
}

/// Case 2: element constraints, merging `(0 to 3)(open)` with `(open)(7 downto 0)`,
/// matrices, three levels, element bounds from an initial value, dynamic bounds.
#[test]
fn arrays_of_unconstrained_arrays() {
    let db = analyze("arrays", &[("work", ARRAYS)]);
    check(
        &db,
        &[
            ("slvv_t", "type: slvv_t(open) [Unconstrained]"),
            ("byte_vec_t", "subtype: slvv_t(open)(7 downto 0) [Partial]"),
            ("four_t", "subtype: slvv_t(0 to 3)(open) [Partial]"),
            ("four_bytes_t", "subtype: slvv_t(0 to 3)(7 downto 0) [Full]"),
            (
                "four_bytes2_t",
                "subtype: slvv_t(0 to 3)(7 downto 0) [Full]",
            ),
            (
                "four_bytes3_t",
                "subtype: slvv_t(0 to 3)(7 downto 0) [Full]",
            ),
            (
                "mat23_t",
                "subtype: mat_t(0 to 1, 0 to 2)(3 downto 0) [Full]",
            ),
            (
                "cube_full_t",
                "subtype: cube_t(0 to 1)(0 to 3)(7 downto 0) [Full]",
            ),
            (
                "cube_mid_t",
                "subtype: cube_t(open)(0 to 3)(open) [Partial]",
            ),
            (
                "c_rows",
                "Plain Constant: slvv_t(0 to 1) [Partial], bounds from Initializer",
            ),
            (
                "c_all",
                "Plain Constant: slvv_t(open) [Unconstrained], bounds from Initializer",
            ),
            (
                "rows",
                "Port Signal: slvv_t(open) [Unconstrained], bounds from Actual",
            ),
            (
                "bytes",
                "Port Signal: slvv_t(open)(7 downto 0) [Partial], bounds from Actual",
            ),
            (
                "s_ram",
                "Plain Signal: slvv_t(0 to {N - 1})({W - 1} downto 0) [Full], bounds from Subtype",
            ),
        ],
    );
    // An element of a partially constrained port keeps the element constraint;
    // `others` in a nested aggregate gets its bounds from the context.
    assert_eq!(
        lines(&db, "s_four(0)"),
        [
            "s_four(0) : STD_ULOGIC_VECTOR(7 downto 0) [Full] <= bytes(0): STD_ULOGIC_VECTOR(7 downto 0) [Full]"
        ]
    );
    assert_eq!(
        lines(&db, "s_ram :"),
        [
            "s_ram : slvv_t(0 to {N - 1})({W - 1} downto 0) [Full] <= (others => (others => '0')): \
          slvv_t(0 to {N - 1})({W - 1} downto 0) [Full] agg-bounds context"
        ]
    );
}

/// Case 3: generics, ports, parameters and the function result take their bounds from the
/// actual or the returned value at run time; constants from their initial value; an alias
/// without subtype is its object, one with a subtype is a view with those bounds.
#[test]
fn interface_objects_results_constants_aliases() {
    let db = analyze("interfaces", &[("work", INTERFACES)]);
    let all = dump_all(&db);
    let has = |l: &str| {
        assert!(
            all.iter().any(|x| x == l),
            "missing `{l}` in\n{}",
            all.join("\n")
        )
    };
    has("g_init: Generic Constant: STD_ULOGIC_VECTOR(open) [Unconstrained], bounds from Actual");
    has("d: Port Signal: STD_ULOGIC_VECTOR(open) [Unconstrained], bounds from Actual");
    has("q: Port Signal: STD_ULOGIC_VECTOR(open) [Unconstrained], bounds from Actual");
    has("x: Param Constant: STD_ULOGIC_VECTOR(open) [Unconstrained], bounds from Actual");
    has("o: Param Variable: STD_ULOGIC_VECTOR(open) [Unconstrained], bounds from Actual");
    has("io: Param Variable: STD_ULOGIC_VECTOR(open) [Unconstrained], bounds from Actual");
    has("invert: subprogram: return STD_ULOGIC_VECTOR(open) [Unconstrained]");
    has("return r: STD_ULOGIC_VECTOR({x'range(1)}) [Full]");
    has("c_word: Plain Constant: STD_ULOGIC_VECTOR(open) [Unconstrained], bounds from Initializer");
    has("tmp: Plain Constant: STD_ULOGIC_VECTOR(open) [Unconstrained], bounds from Initializer");
    has("a_all: alias of d : STD_ULOGIC_VECTOR(open) [Unconstrained], own subtype none");
    has(
        "a_rev: alias of d : STD_ULOGIC_VECTOR(open) [Unconstrained], own subtype \
         STD_ULOGIC_VECTOR(0 to {d'length - 1}) [Full]",
    );
    has(
        "a_un: alias of d : STD_ULOGIC_VECTOR(open) [Unconstrained], own subtype \
         STD_ULOGIC_VECTOR(open) [Unconstrained]",
    );
    has("generic #0 => x\"0F\": STD_ULOGIC_VECTOR(open) [Unconstrained] folded");
    has("port #0 => sd: STD_ULOGIC_VECTOR(7 downto 0) [Full]");
}

/// Case 4: `'subtype` of a fully constrained object is its subtype; of a not fully
/// constrained one it is `FromObject` (bounds read from the object at run time), as is
/// `'element`; `'range` keeps the prefix. An unconstrained generic type actual maps `t`.
#[test]
fn subtype_attributes_and_generic_types() {
    let db = analyze(
        "attributes",
        &[("work", RECORDS), ("work", ARRAYS), ("work", ATTRIBUTES)],
    );
    check(
        &db,
        &[
            (
                "s_bus",
                "Plain Signal: rec_t from-object(bus_p, depth 0) [Full], bounds from Subtype",
            ),
            (
                "s_data",
                "Plain Signal: STD_ULOGIC_VECTOR from-object(bus_p.data, depth 0) [Full], bounds from Subtype",
            ),
            (
                "s_row",
                "Plain Signal: STD_ULOGIC_VECTOR from-object(rows, depth 1) [Full], bounds from Subtype",
            ),
            (
                "s_rng",
                "Plain Signal: STD_ULOGIC_VECTOR({bus_p.data'range(1)}) [Full], bounds from Subtype",
            ),
            (
                "s_same",
                "Plain Signal: rec_t(data(7 downto 0), keep(0 to 0)) [Full], bounds from Subtype",
            ),
        ],
    );
    let (_, inst) = db
        .insts
        .iter()
        .find(|(_, i)| db.decls[i.site].name.key().as_str() == "inst_un")
        .expect("inst_un");
    let map: Vec<String> = inst
        .map
        .types
        .values()
        .map(|&st| subtype(&db, st))
        .collect();
    assert_eq!(map, ["STD_ULOGIC_VECTOR(open) [Unconstrained]"]);
    let ident = db.scopes[inst.scope].names[&yavs_syntax::Symbol::intern("ident")][0];
    assert_eq!(
        dump_decl(&db, ident),
        "subprogram: return STD_ULOGIC_VECTOR(open) [Unconstrained]"
    );
}

/// sim2 #641: `'subtype` of an element of an unconstrained record port is `FromObject`;
/// `'range` of it is an attribute range. Both are fully constrained at run time.
#[test]
fn sim2_641_subtype_of_record_port_element() {
    let db = analyze("g641", &[("work", SIM2_641)]);
    check(
        &db,
        &[
            (
                "axi",
                "Port Signal: bus_rec_t [Unconstrained], bounds from Actual",
            ),
            (
                "address",
                "Plain Signal: (resolved) STD_ULOGIC_VECTOR({axi.address'range(1)}) [Full], bounds from Subtype",
            ),
            (
                "data2",
                "Plain Signal: (resolved) STD_ULOGIC_VECTOR from-object(axi.data, depth 0) [Full], bounds from Subtype",
            ),
        ],
    );
}

/// sim2 #1224: a constant of an array of unconstrained vectors with only the outer range
/// given: element bounds come from the initial value; loops range over attributes.
#[test]
fn sim2_1224_element_bounds_from_initial_value() {
    let db = analyze("g1224", &[("work", SIM2_1224)]);
    check(
        &db,
        &[
            (
                "m",
                "Plain Constant: mat_t(0 to 1) [Partial], bounds from Initializer",
            ),
            (
                "j",
                "LoopParam Constant: INTEGER range {m(i)'range(1)} [Full], bounds from Subtype",
            ),
        ],
    );
}

/// sim2 #2422: a type conversion on an unconstrained `out` formal is `E1312`
/// (finding F8, #111).
#[test]
fn sim2_2422_formal_conversion_on_unconstrained_port() {
    // F8 (#111), decided: rejected like sim2 and sim3; no bounds flow back through
    // a conversion on the formal side.
    let errs = errors("g2422", &[("work", SIM2_2422)]);
    assert_eq!(errs, [("E1312".into(), 16)], "{errs:?}");
}

/// sim2 #2179: `others` for an element of an unconstrained record port is an error (all
/// three reference simulators agree); `s2 : s1'subtype` of a static subtype is that subtype.
#[test]
fn sim2_2179_others_for_record_port_element() {
    assert_eq!(
        errors("g2179", &[("work", SIM2_2179)]),
        [("E1325".into(), 16)]
    );
    let decls_only = SIM2_2179
        .replace("  s1.elem <= (others => '0');\n", "")
        .replace("  s2.elem <= (others => '0');\n", "")
        .replace("  p.elem <= (others => '0');\n", "");
    let db = analyze("g2179b", &[("work", &decls_only)]);
    check(
        &db,
        &[
            (
                "s1",
                "Plain Signal: rec(elem(3 downto 0)) [Full], bounds from Subtype",
            ),
            (
                "s2",
                "Plain Signal: rec(elem(3 downto 0)) [Full], bounds from Subtype",
            ),
            ("p", "Port Signal: rec [Unconstrained], bounds from Actual"),
        ],
    );
}

/// sim1 #1543: aliases of external names take the external name's subtype (6.6.2: no subtype
/// indication of their own); an unconstrained one gets its bounds from the target at
/// elaboration, never guessed at analysis.
#[test]
fn sim1_1543_external_name_alias_subtypes() {
    let db = analyze("n1543", &[("work", SIM1_1543)]);
    check(
        &db,
        &[
            (
                "ram_un",
                "alias of << signal .outer1543.u.ram : ram_t >> : ram_t(open) [Unconstrained], own subtype none",
            ),
            (
                "ram_c",
                "alias of << signal .outer1543.u.ram : ram_t(0 to 15)(31 downto 0) >> : ram_t(0 to 15)(31 downto 0) [Full], own subtype none",
            ),
            (
                "width",
                "Plain Constant: INTEGER range 0 to 2147483647 [Full], bounds from Subtype",
            ),
        ],
    );
}

/// sim1 #1576: `new` of a record subtype whose element is an array of unconstrained arrays,
/// constrained in a local subtype with a dynamic bound folded from a constant.
#[test]
fn sim1_1576_allocator_of_constrained_record() {
    use yavs_sem::hir::{ExprKind, NewInit};
    let db = analyze("n1576", &[("work", SIM1_1576)]);
    check(
        &db,
        &[(
            "container_c_t",
            "subtype: container_t(data(0 to 3)(7 downto 0)) [Full]",
        )],
    );
    let news: Vec<String> = db
        .exprs
        .iter()
        .filter_map(|(_, e)| match &e.kind {
            ExprKind::New {
                init: NewInit::Subtype(st),
            } => Some(subtype(&db, *st)),
            _ => None,
        })
        .collect();
    assert_eq!(news, ["container_t(data(0 to 3)(7 downto 0)) [Full]"]);
}

/// sim1 #1463: an array of records constrained in the port, `(0 to 1)(data(0 to 1))`: the
/// element constraint is a record constraint, and indexing keeps it.
#[test]
fn sim1_1463_array_of_records_constrained_in_port() {
    use yavs_sem::hir::ExprKind;
    let db = analyze("n1463", &[("work", SIM1_1463)]);
    check(
        &db,
        &[(
            "sig",
            "Port Signal: rec_arr_t(0 to 1)(data(0 to 1)) [Full], bounds from Subtype",
        )],
    );
    let idx: Vec<String> = db
        .exprs
        .iter()
        .filter(|(e, x)| matches!(x.kind, ExprKind::Index { .. }) && src(&db, *e) == "sig(0)")
        .map(|(_, x)| subtype(&db, x.subtype))
        .collect();
    assert_eq!(idx, ["my_rec_t(data(0 to 1)) [Full]"]);
}

/// The unconstrained `out` parameter with `others` is an error, as in sim1, sim2, sim3.
#[test]
fn out_parameter_others_is_an_error() {
    assert_eq!(
        errors("outp", &[("work", OUT_PARAM_OTHERS)]),
        [("E1325".into(), 9)]
    );
}

// -- findings: the correct shape, ignored until fixed (docs/design/rule1-review.md) -----

/// F1 (sim1 #1589): one `others` value shared by elements of different widths is resolved
/// and folded once, for the first element: `b` and `c` get 6 bits.
#[test]
fn f1_record_others_resolved_per_element() {
    let db = analyze("n1589", &[("work", SIM1_1589)]);
    let DeclKind::Object(o) = &db.decls[decls(&db, "c_r")[0]].kind else {
        panic!("c_r is an object")
    };
    let v = db.value(o.default.expect("initial value")).expect("folded");
    assert_eq!(
        value_shape(v),
        "([5 downto 0], s, [9 downto 0], ([3 downto 0]), [31 downto 0])"
    );
}

/// F2: record aggregate element values are resolved in the element subtype the
/// context's record constraint gives.
#[test]
fn f2_record_aggregate_uses_context_constraint() {
    analyze("f2", &[("work", RECORDS), ("work", RECORD_AGG_CONTEXT)]);
}

/// F3 (fixed by f1582e5, #150): a selected element of a record object takes the record
/// constraint of its prefix.
#[test]
fn f3_selected_element_keeps_record_constraint() {
    let db = analyze("f3", &[("work", RECORDS), ("work", FIELD_OTHERS)]);
    assert_eq!(
        lines(&db, "s_full.data :")[0].split(" <= ").next(),
        Some("s_full.data : STD_ULOGIC_VECTOR(7 downto 0) [Full]")
    );
    let DeclKind::Object(o) = &db.decls[decls(&db, "c_len")[0]].kind else {
        panic!("c_len is an object")
    };
    assert!(
        db.value(o.default.expect("initial value")).is_some(),
        "static length"
    );
    assert_eq!(
        errors("g2179f", &[("work", SIM2_2179)]),
        [("E1325".into(), 16)]
    );
}

/// F4: attributes of `'element` and `'base` prefixes.
#[test]
fn f4_attribute_of_attribute() {
    analyze("f4", &[("work", ARRAYS), ("work", ATTRIBUTE_OF_ATTRIBUTE)]);
}

/// F5: a constrained subtype indication as generic type actual.
#[test]
fn f5_constrained_generic_type_actual() {
    let gen_pkg = ATTRIBUTES.split("library ieee").next().expect("gen_pkg");
    let db = analyze(
        "f5",
        &[("work", gen_pkg), ("work", GENERIC_CONSTRAINED_ACTUAL)],
    );
    let (_, inst) = db.insts.iter().next().expect("inst_c");
    let map: Vec<String> = inst
        .map
        .types
        .values()
        .map(|&st| subtype(&db, st))
        .collect();
    assert_eq!(map, ["STD_ULOGIC_VECTOR(7 downto 0) [Full]"]);
}

/// F6 (fixed by 260729c): an alias of a record element (sim2 #641's `alias AxiData is
/// Axi.Data`); `'subtype` of it reads the bounds from the alias at run time.
#[test]
fn f6_alias_of_record_element() {
    let db = analyze("f6", &[("work", SIM2_641), ("work", SIM2_641_ALIAS)]);
    check(
        &db,
        &[(
            "data1",
            "Plain Signal: (resolved) STD_ULOGIC_VECTOR from-object(axi_data, depth 0) [Full], bounds from Subtype",
        )],
    );
}

/// F7: after instantiation, `BoundsFrom` and the fully-constrained check follow the actual.
#[test]
fn f7_instance_rechecks_constraints() {
    let db = analyze(
        "f7",
        &[
            (
                "work",
                ATTRIBUTES.split("library ieee").next().expect("gen_pkg"),
            ),
            (
                "work",
                "library ieee; use ieee.std_logic_1164.all;\npackage inst_un is new work.gen_pkg generic map (t => std_ulogic_vector);\n",
            ),
        ],
    );
    let (_, inst) = db.insts.iter().next().expect("inst_un");
    let ident = db.scopes[inst.scope].names[&yavs_syntax::Symbol::intern("ident")][0];
    let DeclKind::Subprogram(s) = &db.decls[ident].kind else {
        panic!("ident is a function")
    };
    let x = s.params[0].decl.expect("param decl");
    assert!(
        dump_decl(&db, x).ends_with("bounds from Actual"),
        "{}",
        dump_decl(&db, x)
    );
    let errs = errors("f7b", &[("work", GENERIC_SIGNAL)]);
    assert_eq!(
        errs.first().map(|e| e.0.as_str()),
        Some("E1212"),
        "{errs:?}"
    );
}

// -- case 6: real code from the corpora (`--features corpus`, tools/fetch-corpus.sh) ----

#[cfg(feature = "corpus")]
mod corpus {
    use super::*;

    /// Analyze the files of `tools/corpus/<manifest>.toml` in order, up to and including the
    /// one whose path ends with `target`. Earlier files may fail (the corpus runner tracks
    /// them); `target` must analyze clean.
    pub(super) fn analyze_upto(manifest: &str, target: &str) -> Db {
        let root = std::env::var_os("YAVS_CORPUS").map_or_else(
            || {
                std::env::home_dir()
                    .expect("home directory")
                    .join(".cache/rsim-corpus")
            },
            PathBuf::from,
        );
        let toml = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join(format!("../../tools/corpus/{manifest}.toml"));
        let text = std::fs::read_to_string(&toml).expect("manifest");
        let field = |l: &str, k: &str| -> String {
            let at = l.find(&format!("{k} = \"")).expect("manifest field") + k.len() + 4;
            l[at..]
                .split('"')
                .next()
                .expect("closing quote")
                .to_string()
        };
        let mut db = Db::new(Options::new(LangRev::Vhdl2008, default_stdlib()));
        for l in text
            .lines()
            .filter(|l| l.trim_start().starts_with("{ lib = "))
        {
            let (lib, path) = (field(l, "lib"), field(l, "path"));
            let file = root.join(field(l, "source")).join(&path);
            assert!(
                file.is_file(),
                "{} not found: run tools/fetch-corpus.sh",
                file.display()
            );
            let before = db.diagnostics().len();
            let r = db.analyze_file(&lib, &file, LangRev::Vhdl2008);
            if path.ends_with(target) {
                let text: String = db.diagnostics()[before..]
                    .iter()
                    .map(|d| render(&db.sources, d, MessageFormat::Human, false))
                    .collect();
                assert!(
                    r.is_ok() && db.diagnostics().len() == before,
                    "{path}:\n{text}"
                );
                return db;
            }
        }
        panic!("{target} is not in {manifest}");
    }

    /// Every user declaration named `name` in a file whose path ends with `file`, dumped.
    pub(super) fn dumps(db: &Db, file: &str, name: &str) -> Vec<String> {
        db.decls
            .iter()
            .filter(|(_, d)| {
                d.name.key().as_str() == name && db.sources.get(d.span.file).path.ends_with(file)
            })
            .map(|(id, _)| dump_decl(db, id))
            .collect()
    }

    /// hdl-modules: an array of unconstrained vectors (types_pkg.vhd:20), and a function
    /// whose result bounds come from the returned variable, itself sized by a function call
    /// (axi_stream_pkg.vhd:109-114).
    #[test]
    fn hdl_modules_types_and_axi_stream() {
        let db = analyze_upto("hdl-modules-2008", "common/src/types_pkg.vhd");
        assert_eq!(
            dumps(&db, "types_pkg.vhd", "slv_vec_t"),
            ["type: slv_vec_t(open) [Unconstrained]"]
        );
        let db = analyze_upto("hdl-modules-2008", "axi_stream/src/axi_stream_pkg.vhd");
        let f = "axi_stream_pkg.vhd";
        assert_eq!(
            dumps(&db, f, "to_slv"),
            ["subprogram: return STD_ULOGIC_VECTOR(open) [Unconstrained]"; 2]
        );
        assert!(dumps(&db, f, "result").contains(
            &"Plain Variable: STD_ULOGIC_VECTOR({axi_stream_m2s_sz(data_width, user_width) - 1} \
              downto 0) [Full], bounds from Subtype"
                .to_string()
        ));
        assert!(
            dumps(&db, f, "data").contains(
                &"Param Constant: STD_ULOGIC_VECTOR(open) [Unconstrained], bounds from Actual"
                    .to_string()
            )
        );
    }

    /// UVVM: `t_byte_array is t_slv_array(open)(7 downto 0)` and an access type to an
    /// unconstrained array of unconstrained arrays (uvvm_util/src/types_pkg.vhd:38-42).
    #[test]
    fn uvvm_types_pkg() {
        let db = analyze_upto("uvvm-2008", "uvvm_util/src/types_pkg.vhd");
        let f = "types_pkg.vhd";
        assert_eq!(
            dumps(&db, f, "t_slv_array"),
            ["type: t_slv_array(open) [Unconstrained]"]
        );
        assert_eq!(
            dumps(&db, f, "t_byte_array"),
            ["subtype: t_slv_array(open)(7 downto 0) [Partial]"]
        );
    }

    /// OSVVM: ScoreboardPkg_slv instantiates ScoreboardGenericPkg with the unconstrained
    /// `std_logic_vector` (ScoreboardPkg_slv.vhd:52-55). The map is right; the cloned
    /// parameters keep `BoundsFrom::Subtype` from the generic (finding F7), e.g. `Item` of
    /// `Push` (ScoreboardGenericPkg.vhd:194-197).
    #[test]
    fn osvvm_scoreboard_slv_instance() {
        use yavs_sem::types::{BoundsFrom, Constrainedness};
        let db = analyze_upto("osvvm-2008", "osvvm/ScoreboardPkg_slv.vhd");
        let (_, inst) = db
            .insts
            .iter()
            .find(|(_, i)| {
                let site = &db.decls[i.site];
                db.sources
                    .get(site.span.file)
                    .path
                    .ends_with("ScoreboardPkg_slv.vhd")
            })
            .expect("ScoreboardPkg_slv");
        let map: Vec<String> = inst
            .map
            .types
            .values()
            .map(|&st| subtype(&db, st))
            .collect();
        assert_eq!(
            map,
            ["(resolved) STD_ULOGIC_VECTOR(open) [Unconstrained]"; 2]
        );
        // Objects declared in this instance (its region or one nested in it).
        let inside = |mut sc: Option<yavs_sem::ids::ScopeId>| {
            while let Some(x) = sc {
                if x == inst.scope {
                    return true;
                }
                sc = db.scopes[x].parent;
            }
            false
        };
        let stale = db
            .decls
            .iter()
            .filter(|(_, d)| inside(Some(d.scope)))
            .filter(|(_, d)| match &d.kind {
                DeclKind::Object(o) => {
                    o.bounds == BoundsFrom::Subtype
                        && db.constrainedness(o.subtype) != Constrainedness::Full
                }
                _ => false,
            })
            .count();
        assert_eq!(stale, 0, "objects with stale BoundsFrom (F7, #109)");
    }

    /// VUnit: a record of unconstrained vectors (axi_stream_pkg.vhd:369-377) passed as an
    /// unconstrained `out` variable whose elements are assigned in the body (1044-1056).
    #[test]
    fn vunit_axi_stream_transaction() {
        let db = analyze_upto(
            "vunit-2008",
            "verification_components/src/axi_stream_pkg.vhd",
        );
        let f = "verification_components/src/axi_stream_pkg.vhd";
        assert_eq!(
            dumps(&db, f, "axi_stream_transaction_t"),
            ["type: axi_stream_transaction_t [Unconstrained]"]
        );
        let params = dumps(&db, f, "axi_stream_transaction");
        assert!(
            params.contains(
                &"Param Variable: axi_stream_transaction_t [Unconstrained], bounds from Actual"
                    .to_string()
            )
        );
        assert!(
            params.iter().all(|p| p.ends_with("bounds from Actual")),
            "{params:?}"
        );
    }
}
