// Lazy loading of the shared LLVM library (feature `llvm`, Linux). Linking against libLLVM makes
// the dynamic loader relocate it in every process that starts, a few milliseconds per run of any
// command, although only `--tier=llvm|auto` uses it. So the executable gets a small stub for each
// function the library exports under the C API (`LLVM*`): it jumps through a table that
// `lazy::ensure` fills with `dlsym` the first time LLVM is needed. The stubs satisfy the
// references of `llvm-sys`, so the linker (which drops libraries nothing needs) leaves libLLVM
// out of the executable's dependencies. This script lists the exports and writes the stubs.
use std::fmt::Write as _;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::{env, fs};

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=LLVM_SYS_211_PREFIX");
    println!("cargo:rustc-check-cfg=cfg(lazy_llvm)");
    let var = |k: &str| env::var(k).unwrap_or_default();
    let arch = var("CARGO_CFG_TARGET_ARCH");
    if var("CARGO_FEATURE_LLVM").is_empty()
        || var("CARGO_CFG_TARGET_OS") != "linux"
        || var("CARGO_CFG_TARGET_ENV") != "gnu"
        || !matches!(arch.as_str(), "x86_64" | "aarch64")
    {
        return;
    }
    let config = llvm_config();
    let out = |args: &[&str]| run(&config, args);
    let lib = PathBuf::from(
        out(&["--link-shared", "--libfiles"])
            .split_whitespace()
            .next()
            .expect("llvm-config lists no shared library"),
    );
    let bindir = PathBuf::from(out(&["--bindir"]).trim());
    println!("cargo:rerun-if-changed={}", lib.display());
    let real = lib
        .canonicalize()
        .unwrap_or_else(|e| panic!("{}: {e}", lib.display()));
    // The file name the dynamic loader finds it by: the SONAME.
    let soname = Command::new(bindir.join("llvm-readelf"))
        .arg("-d")
        .arg(&real)
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .and_then(|s| {
            let at = s.find("soname: [")? + 9;
            Some(s[at..at + s[at..].find(']')?].to_string())
        })
        .unwrap_or_else(|| real.file_name().unwrap().to_string_lossy().into_owned());
    let nm = Command::new(bindir.join("llvm-nm"))
        .args(["-D", "--defined-only"])
        .arg(&real)
        .output()
        .or_else(|_| {
            Command::new("nm")
                .args(["-D", "--defined-only"])
                .arg(&real)
                .output()
        })
        .expect("cannot run llvm-nm or nm");
    assert!(nm.status.success(), "llvm-nm failed on {}", real.display());
    let mut names: Vec<String> = String::from_utf8(nm.stdout)
        .unwrap()
        .lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace().rev();
            let (name, kind) = (f.next()?, f.next()?);
            let name = name.split('@').next()?; // `LLVMFoo@@LLVM_21.1`
            (kind == "T" && name.starts_with("LLVM")).then(|| name.to_string())
        })
        .collect();
    names.sort();
    names.dedup();
    assert!(
        names.len() > 500,
        "{} exports only {} C API functions",
        real.display(),
        names.len()
    );

    let mut asm = String::new();
    let (jump, ty, sec) = match arch.as_str() {
        "x86_64" => ("jmp *yavs_llvm_slots+{o}(%rip)", "@function", "@progbits"),
        _ => (
            "adrp x16, yavs_llvm_slots+{o}\n ldr x16, [x16, :lo12:yavs_llvm_slots+{o}]\n br x16",
            "%function",
            "%progbits",
        ),
    };
    for (i, n) in names.iter().enumerate() {
        let body = jump.replace("{o}", &(8 * i).to_string());
        writeln!(asm, ".pushsection .text.{n},\"ax\",{sec}\n.globl {n}\n.type {n},{ty}\n{n}:\n {body}\n.size {n},.-{n}\n.popsection").unwrap();
    }
    writeln!(asm, ".pushsection .data.yavs_llvm_slots,\"aw\",{sec}\n.balign 8\n.globl yavs_llvm_slots\n.hidden yavs_llvm_slots\nyavs_llvm_slots:\n .rept {}\n .quad yavs_llvm_unloaded\n .endr\n.popsection", names.len()).unwrap();
    let mut rs = String::new();
    writeln!(
        rs,
        "const SONAME: &str = {soname:?};\nconst PATH: &str = {:?};\nconst COUNT: usize = {};",
        real.display().to_string(),
        names.len()
    )
    .unwrap();
    writeln!(
        rs,
        "static NAMES: &str = \"{}\";",
        names.iter().map(|n| format!("{n}\\0")).collect::<String>()
    )
    .unwrap();
    writeln!(rs, "std::arch::global_asm!({asm:?}, options(att_syntax));").unwrap();
    if arch == "aarch64" {
        // `options(att_syntax)` is x86 only.
        rs = rs.replace(", options(att_syntax)", "");
    }
    fs::write(Path::new(&var("OUT_DIR")).join("stubs.rs"), rs).unwrap();
    println!("cargo:rustc-cfg=lazy_llvm");
}

/// The `llvm-config` of the LLVM that `llvm-sys` links: the prefix variable, else the path.
fn llvm_config() -> PathBuf {
    let mut c = Vec::new();
    if let Some(p) = env::var_os("LLVM_SYS_211_PREFIX") {
        c.push(Path::new(&p).join("bin/llvm-config"));
    }
    c.extend(["llvm-config-21", "llvm-config"].map(PathBuf::from));
    c.into_iter()
        .find(|p| {
            Command::new(p)
                .arg("--version")
                .output()
                .is_ok_and(|o| o.status.success())
        })
        .expect("no llvm-config for LLVM 21 (set LLVM_SYS_211_PREFIX)")
}

fn run(config: &Path, args: &[&str]) -> String {
    let o = Command::new(config)
        .args(args)
        .output()
        .unwrap_or_else(|e| panic!("{}: {e}", config.display()));
    assert!(o.status.success(), "{} {args:?} failed", config.display());
    String::from_utf8(o.stdout).unwrap()
}
