//! sem.md 2.10: the native symbols of the bodiless std subprograms, computed
//! by `mangle::native_symbol` from the vendored std sources of both
//! revisions, are exactly the 235 symbols of `lib/NATIVE.md` section 1.

use std::collections::BTreeSet;
use std::path::PathBuf;

use yavs_diag::SourceMap;
use yavs_sem::mangle::native_symbol;
use yavs_syntax::ast::{Decl, LibraryUnit, TypeDef};
use yavs_syntax::{Options, Std};

/// The checkout's `lib/` (the source of the built-in copy, and the home of `NATIVE.md`).
fn lib() -> PathBuf {
    PathBuf::from(concat!(env!("CARGO_MANIFEST_DIR"), "/../../lib"))
}

fn symbols_of(rev: Std, pkg: &str, out: &mut BTreeSet<String>) {
    let path = lib().join(format!("std/{}/{pkg}.vhdl", rev.as_str()));
    let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let mut sources = SourceMap::default();
    let (file, _) = yavs_syntax::load_source(
        &mut sources,
        path.display().to_string(),
        bytes,
        yavs_syntax::Encoding::Latin1,
    );
    let opts = Options {
        std: rev,
        defines: Vec::new(),
        psl: false,
    };
    let parsed = yavs_syntax::parse_file(file, sources.get(file), &opts);
    let design = parsed
        .design
        .unwrap_or_else(|| panic!("{} does not parse: {:?}", path.display(), parsed.diags));
    for unit in &design.units {
        let LibraryUnit::Package(p) = &unit.unit else {
            continue;
        };
        for d in &p.decls {
            match d {
                Decl::Subprogram(spec) if pkg != "reflection" => {
                    out.insert(native_symbol(pkg, None, spec));
                }
                Decl::Type(t) => {
                    if let Some(TypeDef::Protected(pt)) = &t.def {
                        for m in &pt.decls {
                            if let Decl::Subprogram(spec) = m {
                                out.insert(native_symbol(pkg, Some(t.name.key.as_str()), spec));
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }
}

#[test]
fn std_natives_match_native_md() {
    let mut got = BTreeSet::new();
    for pkg in ["standard", "textio", "env"] {
        symbols_of(Std::Vhdl2008, pkg, &mut got);
        symbols_of(Std::Vhdl2019, pkg, &mut got);
    }
    symbols_of(Std::Vhdl2019, "reflection", &mut got);

    let md = std::fs::read_to_string(lib().join("NATIVE.md")).expect("lib/NATIVE.md");
    let section = md
        .split("## 1.")
        .nth(1)
        .and_then(|s| s.split("## 2.").next())
        .expect("NATIVE.md section 1");
    let want: Vec<String> = section
        .lines()
        .filter_map(|l| l.strip_prefix("| `yavs."))
        .map(|l| format!("yavs.{}", &l[..l.find('`').expect("closing backtick")]))
        .collect();
    let want_set: BTreeSet<String> = want.iter().cloned().collect();
    assert_eq!(want.len(), 235, "NATIVE.md section 1 rows");
    assert_eq!(want_set.len(), 235, "NATIVE.md symbols are unique");
    let missing: Vec<_> = want_set.difference(&got).collect();
    let extra: Vec<_> = got.difference(&want_set).collect();
    assert!(
        missing.is_empty() && extra.is_empty(),
        "missing from analysis: {missing:#?}\nnot in NATIVE.md: {extra:#?}"
    );
}
