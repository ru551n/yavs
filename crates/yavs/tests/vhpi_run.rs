//! `yavs run --vhpi`: the test plugin of `yavs-vhpi` loaded by the CLI (#267). On Windows the
//! test builds a throwaway import library for `yavs.exe` from the generated `.def` (yavs ships none).

use std::path::PathBuf;
use std::process::Command;

const VHDL: &str = "entity top is end;\n\
    architecture a of top is begin\n\
    process begin report \"design ran\"; wait; end process;\n\
    end;\n";

fn yavs(args: &[&str]) -> (i32, String, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(["--color", "never"])
        .args(args)
        .output()
        .expect("run");
    (
        o.status.code().expect("exit code"),
        String::from_utf8_lossy(&o.stderr).into_owned(),
        String::from_utf8_lossy(&o.stdout).into_owned(),
    )
}

#[cfg(unix)]
fn build_plugin(vhpi: &std::path::Path, _tmp: &std::path::Path, so: &std::path::Path) {
    let tool = compiler();
    let status = tool
        .to_command()
        .args(["-shared", "-fPIC", "-I"])
        .arg(vhpi.join("include"))
        .arg(vhpi.join("tests/plugin/plugin.c"))
        .arg("-o")
        .arg(so)
        .status()
        .expect("cc");
    assert!(status.success());
}

/// Windows: import library for `yavs.exe` from the generated `.def` (test time only), then the
/// plugin DLL linked against it.
#[cfg(windows)]
fn build_plugin(vhpi: &std::path::Path, tmp: &std::path::Path, so: &std::path::Path) {
    let tool = compiler();
    let implib = tmp.join("yavs.lib");
    let def = env!("YAVS_VHPI_DEF");
    let mut mk = if tool.is_like_msvc() {
        let mut c = cc::windows_registry::find(env!("YAVS_TARGET"), "lib.exe").expect("lib.exe");
        c.args(["/NOLOGO", "/MACHINE:X64", "/NAME:yavs.exe"])
            .arg(format!("/DEF:{def}"))
            .arg(format!("/OUT:{}", implib.display()));
        c
    } else {
        let mut c = Command::new("dlltool");
        c.args(["-d", def, "-D", "yavs.exe", "-l"]).arg(&implib);
        c
    };
    let status = mk.status().expect("import library tool");
    assert!(status.success(), "{mk:?}");
    let mut cmd = tool.to_command();
    cmd.current_dir(tmp);
    if tool.is_like_msvc() {
        cmd.arg("/nologo")
            .arg(format!("/I{}", vhpi.join("include").display()))
            .arg(vhpi.join("tests/plugin/plugin.c"))
            .arg(format!("/Fe{}", so.display()))
            .arg("/LD")
            .arg("/link")
            .arg(&implib);
    } else {
        cmd.arg("-shared")
            .arg("-I")
            .arg(vhpi.join("include"))
            .arg(vhpi.join("tests/plugin/plugin.c"))
            .arg("-o")
            .arg(so)
            .arg(&implib);
    }
    let status = cmd.status().expect("cc");
    assert!(status.success(), "{cmd:?}");
}

fn compiler() -> cc::Tool {
    cc::Build::new()
        .target(env!("YAVS_TARGET"))
        .host(env!("YAVS_HOST"))
        .opt_level(0)
        .debug(false)
        .cargo_metadata(false)
        .cargo_warnings(false)
        .get_compiler()
}

/// Compiles the `yavs-vhpi` test plugin and analyses the design; returns (lib arg, plugin path).
fn setup() -> (String, PathBuf) {
    let tmp = std::env::temp_dir().join(format!("yavs-cli-{}-vhpi", std::process::id()));
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("tmp");
    let vhpi = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../yavs-vhpi");
    let so = tmp.join(if cfg!(windows) {
        "plugin.dll"
    } else {
        "libplugin.so"
    });
    build_plugin(&vhpi, &tmp, &so);
    let src = tmp.join("top.vhd");
    std::fs::write(&src, VHDL).expect("write");
    let lib = format!("--work=work:{}", tmp.join("lib").display());
    let (code, err, _) = yavs(&["analyze", &lib, &src.display().to_string()]);
    assert_eq!(code, 0, "{err}");
    (lib, so)
}

#[test]
fn plugin_callbacks_and_finish() {
    let (lib, so) = setup();
    for spec in [
        format!("--vhpi={}", so.display()),
        format!("--vhpi={}:plugin_bootstrap", so.display()),
    ] {
        let (code, err, out) = yavs(&["run", &lib, &spec, "top"]);
        assert_eq!(code, 0, "{err}");
        assert!(out.contains("plugin: start of simulation"), "{out}");
        // The design root is the real top-level entity (name upper-cased, case name as written).
        assert!(out.contains("plugin: root name=TOP case=top"), "{out}");
        assert!(out.contains("plugin: same object=1"), "{out}");
        // vhpi_control(vhpiFinish) from the start callback ends the run; end callbacks still fire.
        assert!(out.contains("FINISH called with status 0"), "{out}");
        assert!(
            out.contains("plugin: end of simulation, user_data=bye"),
            "{out}"
        );
    }
    assert!(
        yavs(&[
            "run",
            &lib,
            &format!("--vhpi={}:plugin_bootstrap", so.display()),
            "top"
        ])
        .2
        .contains("plugin: bootstrap entry")
    );
}

#[test]
fn plugin_load_errors_fail_early() {
    let (lib, so) = setup();
    let (code, err, out) = yavs(&["run", &lib, "--vhpi=/no/such/plugin.so", "top"]);
    assert_eq!(code, 2);
    assert!(err.contains("error[E0901]"), "{err}");
    assert!(!out.contains("design ran"), "{out}");
    let (code, err, out) = yavs(&[
        "run",
        &lib,
        &format!("--vhpi={}:no_such_entry", so.display()),
        "top",
    ]);
    assert_eq!(code, 2);
    assert!(err.contains("error[E0902]"), "{err}");
    assert!(!out.contains("design ran"), "{out}");
}
