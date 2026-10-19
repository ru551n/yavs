//! ABI and export tests. Both compile C at test time with the compiler
//! `cc` picks for the build target.
//!
//! - `abi_matches_c_header`: every constant and every struct size and
//!   field offset in `sys` equals what the C compiler sees through
//!   `include/vhpi_user.h`, the vendored IEEE header.
//! - `plugin_*`: a C plugin linked against nothing but the header (and,
//!   on Windows, the host's import library) is loaded by the
//!   `yavs-vhpi-host` executable and drives the stub model.

use std::collections::BTreeSet;
use std::fmt::Write as _;
use std::mem::{offset_of, size_of};
use std::path::{Path, PathBuf};
use std::process::Command;

use yavs_vhpi::sys::*;

const INCLUDE: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/include");

fn tmp() -> PathBuf {
    PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("yavs-vhpi")
}

fn compiler() -> cc::Tool {
    cc::Build::new()
        .target(env!("YAVS_VHPI_TARGET"))
        .host(env!("YAVS_VHPI_HOST"))
        .opt_level(0)
        .debug(false)
        .cargo_metadata(false)
        .cargo_warnings(false)
        .get_compiler()
}

/// Compiles one C file to an executable, or a shared library when
/// `shared`. `link` is extra linker input (an import library).
fn compile(src: &Path, include: &Path, out: &Path, shared: bool, link: Option<&Path>) {
    let tool = compiler();
    let mut cmd = tool.to_command();
    cmd.current_dir(out.parent().unwrap());
    if tool.is_like_msvc() {
        cmd.arg("/nologo")
            .arg("/W3")
            .arg(format!("/I{}", include.display()))
            .arg(src);
        cmd.arg(format!("/Fe{}", out.display()));
        if shared {
            cmd.arg("/LD");
        }
        if let Some(lib) = link {
            cmd.arg("/link").arg(lib);
        }
    } else {
        cmd.arg("-Wall")
            .arg("-Werror")
            .arg("-I")
            .arg(include)
            .arg(src)
            .arg("-o")
            .arg(out);
        if shared {
            cmd.arg("-shared");
            if !cfg!(windows) {
                cmd.arg("-fPIC");
            }
        }
        if let Some(lib) = link {
            cmd.arg(lib);
        }
    }
    let status = cmd
        .status()
        .unwrap_or_else(|e| panic!("cannot run {cmd:?}: {e}"));
    assert!(status.success(), "{cmd:?} failed: {status}");
}

fn run(cmd: &mut Command) -> (i32, String, String) {
    let out = cmd
        .output()
        .unwrap_or_else(|e| panic!("cannot run {cmd:?}: {e}"));
    let text = |b: &[u8]| String::from_utf8_lossy(b).replace("\r\n", "\n");
    (
        out.status.code().unwrap_or(-1),
        text(&out.stdout),
        text(&out.stderr),
    )
}

// ---- ABI -----------------------------------------------------------------

/// (C type, C field, Rust offset); field "" means sizeof the type.
fn layout() -> Vec<(&'static str, &'static str, usize)> {
    macro_rules! size {
        ($t:ident) => {
            (stringify!($t), "", size_of::<$t>())
        };
    }
    macro_rules! field {
        ($t:ident, $f:ident) => {
            field!($t, $f, stringify!($f))
        };
        ($t:ident, $f:ident, $c:expr) => {
            (stringify!($t), $c, offset_of!($t, $f))
        };
    }
    vec![
        size!(vhpiHandleT),
        size!(vhpiEnumT),
        size!(vhpiSmallEnumT),
        size!(vhpiIntT),
        size!(vhpiLongIntT),
        size!(vhpiCharT),
        size!(vhpiRealT),
        size!(vhpiSmallPhysT),
        size!(vhpiFormatT),
        size!(vhpiClassKindT),
        size!(vhpiPhysT),
        field!(vhpiPhysT, high),
        field!(vhpiPhysT, low),
        size!(vhpiTimeT),
        field!(vhpiTimeT, high),
        field!(vhpiTimeT, low),
        size!(vhpiValueT),
        field!(vhpiValueT, format),
        field!(vhpiValueT, bufSize),
        field!(vhpiValueT, numElems),
        field!(vhpiValueT, unit),
        field!(vhpiValueT, value),
        ("vhpiValueT.value", "", size_of::<vhpiValueU>()),
        size!(vhpiErrorInfoT),
        field!(vhpiErrorInfoT, severity),
        field!(vhpiErrorInfoT, message),
        field!(vhpiErrorInfoT, str_, "str"),
        field!(vhpiErrorInfoT, file),
        field!(vhpiErrorInfoT, line),
        size!(vhpiCbDataT),
        field!(vhpiCbDataT, reason),
        field!(vhpiCbDataT, cb_rtn),
        field!(vhpiCbDataT, obj),
        field!(vhpiCbDataT, time),
        field!(vhpiCbDataT, value),
        field!(vhpiCbDataT, user_data),
        size!(vhpiForeignDataT),
        field!(vhpiForeignDataT, kind),
        field!(vhpiForeignDataT, libraryName),
        field!(vhpiForeignDataT, modelName),
        field!(vhpiForeignDataT, elabf),
        field!(vhpiForeignDataT, execf),
    ]
}

/// The C program that prints what the compiler sees, and what it must
/// print to agree with Rust.
fn abi_program() -> (String, String) {
    let mut c = String::from(
        "#include <stdio.h>\n#include <stddef.h>\n#include \"vhpi_user.h\"\nint main(void) {\n",
    );
    let mut expected = String::new();
    for (t, f, rust) in layout() {
        let expr = match (t, f) {
            ("vhpiValueT.value", _) => "sizeof(((vhpiValueT *)0)->value)".to_owned(),
            (_, "") => format!("sizeof({t})"),
            _ => format!("offsetof({t}, {f})"),
        };
        writeln!(c, "  printf(\"{t} {f} %u\\n\", (unsigned)({expr}));").unwrap();
        writeln!(expected, "{t} {f} {rust}").unwrap();
    }
    for (name, value) in ALL_VALUES.iter().copied().flatten() {
        writeln!(c, "  printf(\"{name} %lld\\n\", (long long)({name}));").unwrap();
        writeln!(expected, "{name} {value}").unwrap();
    }
    c.push_str("  return 0;\n}\n");
    (c, expected)
}

fn check_abi_against(header_dir: &Path, tag: &str) {
    let dir = tmp().join(format!("abi-{tag}"));
    std::fs::create_dir_all(&dir).unwrap();
    let (program, expected) = abi_program();
    let src = dir.join("abi.c");
    std::fs::write(&src, program).unwrap();
    let exe = dir.join(format!("abi{}", std::env::consts::EXE_SUFFIX));
    compile(&src, header_dir, &exe, false, None);
    let (code, stdout, stderr) = run(&mut Command::new(&exe));
    assert_eq!(code, 0, "{stderr}");
    for (got, want) in stdout.lines().zip(expected.lines()) {
        assert_eq!(got, want, "C ({tag}) vs Rust");
    }
    assert_eq!(stdout.lines().count(), expected.lines().count());
}

#[test]
fn abi_matches_c_header() {
    check_abi_against(Path::new(INCLUDE), "ieee");
}

/// The header declares nothing the Rust side lacks: every enumerator and
/// numeric `#define` named vhpi* has a Rust constant.
#[test]
fn header_has_no_constant_missing_from_rust() {
    let header = std::fs::read_to_string(Path::new(INCLUDE).join("vhpi_user.h")).unwrap();
    let is_name = |s: &str| s.starts_with("vhpi") || s.starts_with("DEPRECATED_vhpi");
    let mut in_header = BTreeSet::new();
    for line in header.lines().map(str::trim) {
        let mut words = line.split_whitespace();
        match (words.next(), words.next(), words.next()) {
            (Some("#define"), Some(name), Some(_)) if is_name(name) => {
                in_header.insert(name);
            }
            (Some(name), Some("="), Some(_)) if is_name(name) => {
                in_header.insert(name);
            }
            _ => {}
        }
    }
    let in_rust: BTreeSet<&str> = ALL_VALUES
        .iter()
        .copied()
        .flatten()
        .map(|(n, _)| *n)
        .collect();
    let missing: Vec<_> = in_header.difference(&in_rust).collect();
    assert!(
        missing.is_empty(),
        "in vhpi_user.h but not in sys: {missing:?}"
    );
    assert!(in_rust.len() > 600, "parsed only {} names", in_rust.len());
}

// ---- Plugin through the host executable --------------------------------

fn host() -> Command {
    Command::new(env!("CARGO_BIN_EXE_yavs-vhpi-host"))
}

fn plugin() -> PathBuf {
    use std::sync::OnceLock;
    static PLUGIN: OnceLock<PathBuf> = OnceLock::new();
    PLUGIN
        .get_or_init(|| {
            let dir = tmp().join("plugin");
            std::fs::create_dir_all(&dir).unwrap();
            let name = format!(
                "{}vhpi_test_plugin{}",
                std::env::consts::DLL_PREFIX,
                std::env::consts::DLL_SUFFIX
            );
            let out = dir.join(name);
            let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/plugin/plugin.c");
            // Windows: resolve vhpi_* through the host's import library.
            let import_lib = option_env!("YAVS_VHPI_IMPORT_LIB").map(Path::new);
            compile(&src, Path::new(INCLUDE), &out, true, import_lib);
            out
        })
        .clone()
}

const REGISTRATION: &str = "\
plugin: start cb registered, state=0 reason=1034
plugin: disabled cb state=1
plugin: remove=0
plugin: 42 exports referenced, printable(A)=1 printable(\\n)=0
";

const SIMULATION: &str = "\
plugin: start of simulation, reason=1034 time=0/0
plugin: root name=TOP case=Top full=:TOP kind=1090
plugin: same object=1
plugin: missing: error severity=3: vhpi_handle_by_name: no object named no_such
plugin: missing=null
plugin: release=0
plugin: stale: error severity=3: vhpi_compare_handles: invalid or released handle
plugin: stale compare=0
plugin: release again=1
plugin: tool=yavs
plugin: argc=0
plugin: iterator: error severity=3: vhpi_iterator: relation 1563 of kind 1090: not implemented in yavs yet
plugin: check twice: no error
plugin: resolution=0/1 ns=0/1000000 hr=838190317/661127168
plugin: finish=0
host: finish requested
plugin: end of simulation, user_data=bye
";

#[test]
fn plugin_startup_routines() {
    let (code, stdout, stderr) = run(host().args(["--top", "Top", "--vhpi"]).arg(plugin()));
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(stdout, format!("{REGISTRATION}{SIMULATION}"));
    // Misuse and unsupported calls are reported, not only recorded; a name
    // that is not found is not.
    let misuse = "warning[W0903]: VHPI misuse: vhpi_compare_handles: invalid or released handle";
    assert!(stderr.contains(misuse), "{stderr}");
    assert!(
        stderr.contains("warning[W0903]: VHPI misuse: vhpi_release_handle"),
        "{stderr}"
    );
    let unsupported = "warning[W0904]: unsupported VHPI call: vhpi_iterator";
    assert_eq!(stderr.matches(unsupported).count(), 1, "{stderr}");
    assert!(!stderr.contains("no_such"), "{stderr}");
}

#[test]
fn plugin_named_entry() {
    let spec = format!("{}:plugin_bootstrap", plugin().display());
    let (code, stdout, stderr) = run(host().args(["--top", "Top", "--vhpi", &spec]));
    assert_eq!(code, 0, "{stderr}");
    assert_eq!(
        stdout,
        format!("plugin: bootstrap entry\n{REGISTRATION}{SIMULATION}")
    );
}

#[test]
fn plugin_missing_entry_is_an_error() {
    let spec = format!("{}:no_such_entry", plugin().display());
    let (code, stdout, stderr) = run(host().args(["--vhpi", &spec]));
    assert_eq!(code, 2);
    assert_eq!(stdout, "");
    assert!(stderr.contains("error[E0902]: VHPI plugin"), "{stderr}");
    assert!(
        stderr.contains("has no function `no_such_entry`"),
        "{stderr}"
    );
}

#[test]
fn plugin_missing_file_is_an_error() {
    let missing = tmp().join("no_such_dir").join("plugin.so");
    let (code, stdout, stderr) = run(host().arg("--vhpi").arg(&missing));
    assert_eq!(code, 2);
    assert_eq!(stdout, "");
    assert!(
        stderr.contains("error[E0901]: cannot load VHPI plugin"),
        "{stderr}"
    );
    assert!(stderr.contains("no such file"), "{stderr}");
    assert!(stderr.contains(&missing.display().to_string()), "{stderr}");
}

/// A plugin importing a vhpi_* symbol yavs does not export fails at load,
/// naming the symbol, not at its first call. (On Windows the same plugin
/// does not even link against yavs.lib.)
#[cfg(unix)]
#[test]
fn plugin_with_unresolved_symbol_is_an_error() {
    let dir = tmp().join("unresolved");
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("unresolved.c");
    std::fs::write(
        &src,
        "#include \"vhpi_user.h\"\n\
         int vhpi_sens_zero(void *);\n\
         static void reg(void) { vhpi_sens_zero(0); }\n\
         void (*vhpi_startup_routines[])(void) = {reg, 0};\n",
    )
    .unwrap();
    let lib = dir.join("libunresolved.so");
    compile(&src, Path::new(INCLUDE), &lib, true, None);
    let (code, _, stderr) = run(host().arg("--vhpi").arg(&lib));
    assert_eq!(code, 2);
    assert!(stderr.contains("error[E0901]"), "{stderr}");
    assert!(stderr.contains("vhpi_sens_zero"), "{stderr}");
}
