//! Compiles the variadic C entry points and turns `src/exports.txt` into
//! the per-OS export files:
//!
//! - `exports.rs`: the Rust table that references every exported symbol,
//!   so none is dropped at link time.
//! - `yavs-vhpi.map` (Linux): a linker version script, passed as
//!   `-rdynamic -Wl,--version-script` (`--dynamic-list` is rejected by cargo-zigbuild).
//! - `exports.def` (Windows): passed to the linker of the executable.
//! - `yavs-vhpi-host.lib` (Windows): import library for this crate's tests,
//!   naming the spike host. No import library for `yavs.exe` is built or
//!   shipped (plugins bring their own).
//!
//! This package's own binaries get the link arguments directly. Cargo
//! does not forward `rustc-link-arg-bins` to dependents, so the `yavs`
//! CLI's `build.rs` reads the paths from `DEP_YAVS_VHPI_*`.

use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=src/exports.txt");
    println!("cargo:rerun-if-changed=src/shim.c");

    let out = PathBuf::from(env::var("OUT_DIR").unwrap());
    let target = env::var("TARGET").unwrap();
    let target_os = env::var("CARGO_CFG_TARGET_OS").unwrap();
    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();

    cc::Build::new()
        .file("src/shim.c")
        .warnings(true)
        .compile("yavs_vhpi_shim");

    // (name, is_data)
    let exports: Vec<(String, bool)> = fs::read_to_string("src/exports.txt")
        .unwrap()
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| match l.split_once(' ') {
            Some((name, "DATA")) => (name.to_owned(), true),
            None => (l.to_owned(), false),
            Some(_) => panic!("bad line in exports.txt: {l}"),
        })
        .collect();

    let mut rs = String::from(
        "pub(crate) fn exported_symbols() -> Vec<(&'static str, *const ())> {\n    vec![\n",
    );
    let mut dynlist = String::from("{\n  global:\n");
    let mut def = String::from("EXPORTS\n");
    for (name, data) in &exports {
        let addr = if *data {
            format!("(&raw const {name}).cast()")
        } else {
            format!("{name} as *const ()")
        };
        writeln!(rs, "        (\"{name}\", {addr}),").unwrap();
        writeln!(dynlist, "    {name};").unwrap();
        writeln!(def, "    {name}{}", if *data { " DATA" } else { "" }).unwrap();
    }
    rs.push_str("    ]\n}\n");
    dynlist.push_str("  local:\n    *;\n};\n");
    fs::write(out.join("exports.rs"), rs).unwrap();
    let dynlist_path = out.join("yavs-vhpi.map");
    fs::write(&dynlist_path, dynlist).unwrap();
    let def_path = out.join("exports.def");
    fs::write(&def_path, def).unwrap();

    println!("cargo:version_script={}", dynlist_path.display());
    println!("cargo:def={}", def_path.display());
    println!(
        "cargo:include={}",
        Path::new(&env::var("CARGO_MANIFEST_DIR").unwrap())
            .join("include")
            .display()
    );
    // The tests compile C with the same target as this build.
    println!("cargo:rustc-env=YAVS_VHPI_TARGET={target}");
    println!(
        "cargo:rustc-env=YAVS_VHPI_HOST={}",
        env::var("HOST").unwrap()
    );

    match target_os.as_str() {
        "windows" => {
            if target_env == "msvc" {
                println!("cargo:rustc-link-arg-bins=/DEF:{}", def_path.display());
            } else {
                // GNU ld reads a .def file given as an input.
                println!("cargo:rustc-link-arg-bins={}", def_path.display());
            }
            let lib = out.join("yavs-vhpi-host.lib");
            if let Err(e) = import_lib(&target, &target_env, &def_path, "yavs-vhpi-host.exe", &lib)
            {
                println!("cargo:warning=no import library {}: {e}", lib.display());
            }
            println!("cargo:import_lib_dir={}", out.display());
            println!(
                "cargo:rustc-env=YAVS_VHPI_IMPORT_LIB={}",
                out.join("yavs-vhpi-host.lib").display()
            );
        }
        "macos" | "ios" => {}
        _ => {
            println!("cargo:rustc-link-arg-bins=-rdynamic");
            println!(
                "cargo:rustc-link-arg-bins=-Wl,--version-script={}",
                dynlist_path.display()
            );
        }
    }
}

/// Writes an import library for `dll` (an .exe here) from `def`. Tries
/// MSVC `lib.exe`, then `llvm-dlltool`, then MinGW `dlltool`. MSVC and
/// llvm-dlltool write short-format import libraries; MinGW dlltool writes
/// the long format. MSVC link and GNU ld accept both.
fn import_lib(
    target: &str,
    target_env: &str,
    def: &Path,
    dll: &str,
    out: &Path,
) -> Result<(), String> {
    let mut tries: Vec<Command> = Vec::new();
    if target_env == "msvc"
        && let Some(mut lib) = cc::windows_registry::find(target, "lib.exe")
    {
        lib.arg(format!("/DEF:{}", def.display()))
            .arg(format!("/OUT:{}", out.display()))
            .arg(format!("/NAME:{dll}"))
            .arg("/MACHINE:X64")
            .arg("/NOLOGO");
        tries.push(lib);
    }
    for tool in [
        "llvm-dlltool",
        "llvm-dlltool-21",
        "x86_64-w64-mingw32-dlltool",
        "dlltool",
    ] {
        let mut cmd = Command::new(tool);
        if tool.starts_with("llvm") {
            cmd.args(["-m", "i386:x86-64"]);
        }
        cmd.arg("-d").arg(def).arg("-D").arg(dll).arg("-l").arg(out);
        tries.push(cmd);
    }
    let mut errors = Vec::new();
    for mut cmd in tries {
        match cmd.status() {
            Ok(s) if s.success() => return Ok(()),
            Ok(s) => errors.push(format!("{:?}: {s}", cmd.get_program())),
            Err(e) => errors.push(format!("{:?}: {e}", cmd.get_program())),
        }
    }
    Err(errors.join("; "))
}
