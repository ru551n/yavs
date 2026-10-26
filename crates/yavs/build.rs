// Exports the `vhpi_*` C ABI from the executable. Cargo does not forward the
// link arguments of `yavs-vhpi`'s build script to this package's binaries.
// The tests compile C (a `--load` library) for the same target as this build.
use sha2::{Digest, Sha256};
use std::path::Path;

/// Hashes the files below `dir` (sorted, with their relative names) into `h`.
fn hash_tree(h: &mut Sha256, root: &Path, dir: &Path) {
    let mut entries: Vec<_> = std::fs::read_dir(dir)
        .map(|d| d.flatten().map(|e| e.path()).collect())
        .unwrap_or_default();
    entries.sort();
    for p in entries {
        if p.is_dir() {
            hash_tree(h, root, &p);
        } else if let Ok(bytes) = std::fs::read(&p) {
            h.update(
                p.strip_prefix(root)
                    .unwrap_or(&p)
                    .to_string_lossy()
                    .as_bytes(),
            );
            h.update((bytes.len() as u64).to_le_bytes());
            h.update(bytes);
        }
    }
}

/// A hash of every source that can change what the compiler stages generate (the sources of all
/// crates and the lock file): the compilation cache names the build with it (#491). Empty when
/// the workspace is not around (a packaged crate); the cache then uses the executable's age.
fn source_hash() -> String {
    let manifest = std::env::var("CARGO_MANIFEST_DIR").unwrap_or_default();
    let Ok(top) = Path::new(&manifest).join("../..").canonicalize() else {
        return String::new();
    };
    let Ok(dirs) = std::fs::read_dir(top.join("crates")) else {
        return String::new();
    };
    let mut h = Sha256::new();
    let mut dirs: Vec<_> = dirs.flatten().map(|e| e.path()).collect();
    dirs.sort();
    for d in dirs {
        for part in ["src", "Cargo.toml", "build.rs"] {
            let p = d.join(part);
            println!("cargo:rerun-if-changed={}", p.display());
            if p.is_dir() {
                hash_tree(&mut h, &top, &p);
            } else if let Ok(b) = std::fs::read(&p) {
                h.update(
                    p.strip_prefix(&top)
                        .unwrap_or(&p)
                        .to_string_lossy()
                        .as_bytes(),
                );
                h.update(b);
            }
        }
    }
    // The vendored std/ieee sources are embedded in the binary: an edit changes the build, so a
    // store shared by builds (analysed std/ieee units, #564) never holds a stale unit.
    let lib = top.join("lib");
    println!("cargo:rerun-if-changed={}", lib.display());
    hash_tree(&mut h, &top, &lib);
    let lock = top.join("Cargo.lock");
    println!("cargo:rerun-if-changed={}", lock.display());
    h.update(std::fs::read(lock).unwrap_or_default());
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn main() {
    println!("cargo:rustc-env=YAVS_SRC_HASH={}", source_hash());
    let var = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("{k} not set"));
    match var("CARGO_CFG_TARGET_OS").as_str() {
        "windows" => {
            let def = var("DEP_YAVS_VHPI_DEF");
            println!("cargo:rustc-env=YAVS_VHPI_DEF={def}");
            // The main thread of a Windows executable has 1 MB; a run that nests memoised calls
            // needs more in a debug build (#731). 8 MB is the Linux default.
            if var("CARGO_CFG_TARGET_ENV") == "msvc" {
                println!("cargo:rustc-link-arg-bins=/DEF:{def}");
                println!("cargo:rustc-link-arg-bins=/STACK:8388608");
            } else {
                println!("cargo:rustc-link-arg-bins={def}");
                println!("cargo:rustc-link-arg-bins=-Wl,--stack,8388608");
            }
        }
        "macos" | "ios" => {}
        // -rdynamic makes lld (and so cargo-zigbuild) export from an executable;
        // the version script limits the export to the vhpi_* list.
        _ => {
            println!("cargo:rustc-link-arg-bins=-rdynamic");
            println!(
                "cargo:rustc-link-arg-bins=-Wl,--version-script={}",
                var("DEP_YAVS_VHPI_VERSION_SCRIPT")
            );
        }
    }
    for v in ["TARGET", "HOST"] {
        println!("cargo:rustc-env=YAVS_{v}={}", var(v));
    }
}
