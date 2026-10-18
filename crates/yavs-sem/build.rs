//! Embeds `lib/std` and `lib/ieee` into the binary, so a release is one executable.
//! `$YAVS_STDLIB` still points yavs at a tree on disk (`default_stdlib`).

use std::fmt::Write;
use std::path::Path;

fn main() {
    let lib = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../lib");
    let mut files = Vec::new();
    for top in ["std", "ieee"] {
        let dir = lib.join(top);
        println!("cargo:rerun-if-changed={}", dir.display());
        for rev in std::fs::read_dir(&dir).expect("lib/ tree") {
            let rev = rev.expect("lib/ entry").path();
            println!("cargo:rerun-if-changed={}", rev.display());
            for f in std::fs::read_dir(&rev).expect("lib/ revision dir") {
                let f = f.expect("lib/ file").path();
                if f.extension().is_some_and(|e| e == "vhdl") {
                    let rel = f.strip_prefix(&lib).expect("under lib/");
                    let key: Vec<_> = rel
                        .iter()
                        .map(|c| c.to_str().expect("UTF-8 name"))
                        .collect();
                    files.push((key.join("/"), f.canonicalize().expect("lib/ file")));
                }
            }
        }
    }
    files.sort();
    let mut out = String::from("pub(crate) static FILES: &[(&str, &[u8])] = &[\n");
    for (key, path) in &files {
        writeln!(
            out,
            "    ({key:?}, include_bytes!({:?})),",
            path.display().to_string()
        )
        .unwrap();
    }
    out.push_str("];\n");
    let dest = Path::new(&std::env::var("OUT_DIR").unwrap()).join("stdlib_files.rs");
    std::fs::write(dest, out).unwrap();
}
