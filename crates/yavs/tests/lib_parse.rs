//! The standard libraries vendored under `lib/` parse without any diagnostic.

use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use yavs_diag::SourceMap;
use yavs_syntax::{Encoding, Options, Std, load_source, parse_file};

fn vhdl_files(dir: &Path, out: &mut Vec<PathBuf>) {
    for e in std::fs::read_dir(dir).unwrap_or_else(|e| panic!("cannot read {}: {e}", dir.display()))
    {
        let p = e.expect("directory entry").path();
        if p.is_dir() {
            vhdl_files(&p, out);
        } else if p.extension().is_some_and(|e| e == "vhd" || e == "vhdl") {
            out.push(p);
        }
    }
}

#[test]
fn lib() {
    let lib = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib");
    let mut problems = String::new();
    for (dir, std) in [
        ("std/1993", Std::Vhdl1993),
        ("ieee/1993", Std::Vhdl1993),
        ("std/2008", Std::Vhdl2008),
        ("ieee/2008", Std::Vhdl2008),
        ("std/2019", Std::Vhdl2019),
        ("ieee/2019", Std::Vhdl2019),
    ] {
        let mut files = Vec::new();
        vhdl_files(&lib.join(dir), &mut files);
        assert!(!files.is_empty(), "no VHDL files in lib/{dir}");
        for f in files {
            let mut sources = SourceMap::default();
            let bytes = std::fs::read(&f).expect("read");
            let (file, bad) = load_source(&mut sources, "", bytes, Encoding::Auto);
            assert!(bad.is_none());
            let opts = Options {
                std,
                defines: Vec::new(),
                psl: false,
            };
            let parsed = parse_file(file, sources.get(file), &opts);
            for d in &parsed.diags {
                writeln!(problems, "{}: {} {}", f.display(), d.code, d.message).expect("write");
            }
        }
    }
    assert!(problems.is_empty(), "{problems}");
}
