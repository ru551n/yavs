//! User-defined attributes as hdl-modules uses them (LRM 6.7, 7.2, 16.1), written fresh: an
//! attribute package in another library, values from function calls, a local declaration
//! hiding a use-visible one, and a design unit's attribute read from another unit in
//! constants and ranges. `tests/hdl_modules.rs` (feature `corpus`) runs the real files.

use std::path::PathBuf;

use yavs_diag::{MessageFormat, render};
use yavs_sem::decl::DeclKind;
use yavs_sem::hir::ExprKind;
use yavs_sem::stat::ConstValue;
use yavs_sem::{Db, LangRev, Options, default_stdlib};

/// The attribute package, analyzed into library `common`.
const ATTRS: &str = "
package attrs is
  attribute dont_touch : string;
  attribute use_dsp : string;
  attribute ram_style : string;
  type ram_style_t is (ram_style_block, ram_style_distributed, ram_style_auto);
  function to_attribute(r : ram_style_t) return string;
end package;

package body attrs is
  function to_attribute(r : ram_style_t) return string is
  begin
    case r is
      when ram_style_block => return \"block\";
      when ram_style_distributed => return \"distributed\";
      when others => return \"auto\";
    end case;
  end function;
end package body;
";

/// Analyze `(library, source)` in order into a fresh `Db`; every file must be clean.
fn analyze(test: &str, files: &[(&str, &str)]) -> Db {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(test);
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

/// The default value of the constant named `name` (folded keys are lower case).
fn constant(db: &Db, name: &str) -> yavs_sem::ids::ExprId {
    let found = db.decls.iter().find_map(|(_, d)| match &d.kind {
        DeclKind::Object(o) if d.name.key().as_str() == name => o.default,
        _ => None,
    });
    found.unwrap_or_else(|| panic!("no constant `{name}` with a value"))
}

fn int_value(db: &Db, name: &str) -> Option<i64> {
    db.value(constant(db, name)).and_then(ConstValue::as_int)
}

fn is_attr_value(db: &Db, name: &str) -> bool {
    matches!(
        db.exprs[constant(db, name)].kind,
        ExprKind::AttrValue { .. }
    )
}

/// `use common.attribute_pkg.all` + `attribute dont_touch of x : signal is "true";`.
#[test]
fn attribute_from_a_package_in_another_library() {
    let db = analyze(
        "dont_touch",
        &[
            ("common", ATTRS),
            (
                "work",
                "library common;
                 use common.attrs.all;
                 package p is
                   signal data_in_int : bit;
                   attribute dont_touch of data_in_int : signal is \"true\";
                   constant c : string := data_in_int'dont_touch;
                 end package;",
            ),
        ],
    );
    assert!(is_attr_value(&db, "c"));
}

/// `attribute ram_style of mem : signal is to_attribute(ram_type);`.
#[test]
fn attribute_value_from_a_function_call() {
    let db = analyze(
        "ram_style",
        &[
            ("common", ATTRS),
            (
                "work",
                "library common;
                 use common.attrs.all;
                 package p is
                   constant ram_type : ram_style_t := ram_style_auto;
                   type mem_t is array (0 to 15) of bit_vector(7 downto 0);
                   signal mem : mem_t;
                   attribute ram_style of mem : signal is to_attribute(ram_type);
                   constant c : string := mem'ram_style;
                 end package;",
            ),
        ],
    );
    assert!(is_attr_value(&db, "c"));
}

/// A local `attribute use_dsp : string;` hides the use-visible one of the attribute package
/// (12.3, 12.4): no ambiguity, and the specification decorates with the local attribute.
#[test]
fn local_attribute_hides_a_use_visible_one() {
    let db = analyze(
        "use_dsp",
        &[
            ("common", ATTRS),
            (
                "work",
                "library common;
                 use common.attrs.all;
                 package p is
                   attribute use_dsp : string;
                   signal product : bit_vector(47 downto 0);
                   attribute use_dsp of product : signal is \"yes\";
                   constant c : string := product'use_dsp;
                 end package;",
            ),
        ],
    );
    let v = db
        .attr_values
        .iter()
        .map(|(_, v)| v)
        .find(|v| db.decls[v.entity].name.key().as_str() == "product")
        .expect("product is decorated");
    assert_eq!(
        db.decls[v.attr].scope, db.decls[v.entity].scope,
        "decorated with the local `use_dsp`"
    );
}

/// `sine_lookup'latency` in its own architecture and `work.sine_lookup'latency` from another
/// unit, where `latency` is not visible, in constant declarations and ranges.
#[test]
fn design_unit_attribute_from_another_unit() {
    let db = analyze(
        "latency",
        &[(
            "work",
            "entity sine_lookup is
               attribute latency : positive;
               attribute latency of sine_lookup : entity is 3;
             end entity;

             architecture a of sine_lookup is
               constant our_latency : positive := sine_lookup'latency;
               signal valid_pipe : bit_vector(1 to our_latency);
             begin
             end architecture;

             package sine_calculator is
               constant lookup_latency : positive := work.sine_lookup'latency;
               signal pipe : bit_vector(1 to work.sine_lookup'latency - 1);
               constant pipe_length : natural := pipe'length;
             end package;",
        )],
    );
    assert_eq!(int_value(&db, "our_latency"), Some(3));
    assert_eq!(int_value(&db, "lookup_latency"), Some(3));
    assert_eq!(int_value(&db, "pipe_length"), Some(2));
}
