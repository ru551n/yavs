// Exports the `vhpi_*` C ABI from the executable. Cargo does not forward the
// link arguments of `yavs-vhpi`'s build script to this package's binaries.
// The tests compile C (a `--load` library) for the same target as this build.
fn main() {
    let var = |k: &str| std::env::var(k).unwrap_or_else(|_| panic!("{k} not set"));
    match var("CARGO_CFG_TARGET_OS").as_str() {
        "windows" => {
            let def = var("DEP_YAVS_VHPI_DEF");
            println!("cargo:rustc-env=YAVS_VHPI_DEF={def}");
            if var("CARGO_CFG_TARGET_ENV") == "msvc" {
                println!("cargo:rustc-link-arg-bins=/DEF:{def}");
            } else {
                println!("cargo:rustc-link-arg-bins={def}");
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
