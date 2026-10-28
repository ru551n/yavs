//! `yavs run --vhpi`: the test plugin of `yavs-vhpi` loaded by the CLI (#267). On Windows the
//! test builds a throwaway import library for `yavs.exe` from the generated `.def` (yavs ships none).

mod common;
use common::tempdir;

use std::path::PathBuf;
use std::process::Command;

const VHDL: &str = "entity top is end;\n\
    architecture a of top is begin\n\
    process begin report \"design ran\"; wait; end process;\n\
    end;\n\
    entity tb is end;\n\
    architecture a of tb is\n\
    signal s : bit := '0';\n\
    begin\n\
    s <= '1' after 10 ns, '0' after 20 ns;\n\
    process (s) begin report \"s=\" & bit'image(s); end process;\n\
    postponed process (s) begin report \"postponed s=\" & bit'image(s); end process;\n\
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
fn build_plugin(vhpi: &std::path::Path, _tmp: &std::path::Path, so: &std::path::Path, name: &str) {
    let tool = compiler();
    let status = tool
        .to_command()
        .args(["-shared", "-fPIC", "-I"])
        .arg(vhpi.join("include"))
        .arg(vhpi.join(format!("tests/plugin/{name}.c")))
        .arg("-o")
        .arg(so)
        .status()
        .expect("cc");
    assert!(status.success());
}

/// Windows: import library for `yavs.exe` from the generated `.def` (test time only), then the
/// plugin DLL linked against it.
#[cfg(windows)]
fn build_plugin(vhpi: &std::path::Path, tmp: &std::path::Path, so: &std::path::Path, name: &str) {
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
            .arg(vhpi.join(format!("tests/plugin/{name}.c")))
            .arg(format!("/Fe{}", so.display()))
            .arg("/LD")
            .arg("/link")
            .arg(&implib);
    } else {
        cmd.arg("-shared")
            .arg("-I")
            .arg(vhpi.join("include"))
            .arg(vhpi.join(format!("tests/plugin/{name}.c")))
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
fn setup() -> (String, Plugin) {
    setup_plugin("plugin")
}

/// Compiles `yavs-vhpi/tests/plugin/<name>.c` and analyses the design.
fn setup_plugin(name: &str) -> (String, Plugin) {
    setup_with(name, VHDL)
}

/// The compiled plugin; its scratch directory goes when it does.
struct Plugin {
    so: PathBuf,
    _tmp: common::Tmp,
}

impl std::ops::Deref for Plugin {
    type Target = std::path::Path;
    fn deref(&self) -> &std::path::Path {
        &self.so
    }
}

fn vhpi_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../yavs-vhpi")
}

/// Compiles `yavs-vhpi/tests/plugin/<name>.c` and analyses `vhdl`.
fn setup_with(name: &str, vhdl: &str) -> (String, Plugin) {
    setup_args(name, vhdl, &[])
}

/// [`setup_with`], analysing with `args`.
fn setup_args(name: &str, vhdl: &str, args: &[&str]) -> (String, Plugin) {
    let tmp = tempdir("vhpi");
    let vhpi = vhpi_dir();
    let so = tmp.join(if cfg!(windows) {
        format!("{name}.dll")
    } else {
        format!("lib{name}.so")
    });
    build_plugin(&vhpi, &tmp, &so, name);
    let src = tmp.join("top.vhd");
    std::fs::write(&src, vhdl).expect("write");
    let lib = format!("--work=work:{}", tmp.join("lib").display());
    let src = src.display().to_string();
    let mut a = vec!["analyze", &lib];
    a.extend_from_slice(args);
    a.push(&src);
    let (code, err, _) = yavs(&a);
    assert_eq!(code, 0, "{err}");
    (lib, Plugin { so, _tmp: tmp })
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

/// A plugin observes the run, so no assignment of the value a signal holds is dropped
/// (`skip_signals`, `Config::skip_unchanged`).
#[test]
fn a_plugin_turns_assignment_skipping_off() {
    let (lib, so) = setup();
    let skip = |extra: &[String]| -> u64 {
        let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
            .env("YAVS_COUNTERS", "1")
            .args(["--color", "never", "run", &lib])
            .args(extra)
            .arg("tb")
            .output()
            .expect("run");
        let err = String::from_utf8_lossy(&o.stderr).into_owned();
        err.lines()
            .find_map(|l| l.strip_prefix("counters: skip_signals="))
            .unwrap_or_else(|| panic!("no counter: {err}"))
            .parse()
            .expect("number")
    };
    assert!(skip(&[]) > 0);
    assert_eq!(skip(&[format!("--vhpi={}", so.display())]), 0);
}

#[test]
fn plusargs_reach_the_plugin() {
    let (lib, so) = setup();
    let spec = format!("--vhpi={}", so.display());
    let (code, err, out) = yavs(&["run", &lib, &spec, "top", "+foo=bar", "+test1", "-gN=1"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("plugin: argc=3\n"), "{out}");
    assert!(out.contains("plugin: argv yavs\n"), "{out}");
    assert!(out.contains("plugin: argv +foo=bar\n"), "{out}");
    assert!(out.contains("plugin: argv +test1\n"), "{out}");
    // Anything after the top level that is not `+NAME` or `-gNAME=VALUE` is a usage error.
    let (code, err, _) = yavs(&["run", &lib, "top", "stray"]);
    assert_eq!(code, 2, "{err}");
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

/// Runs `tb` with the scheduling plugin entry `entry`; returns (exit code, stderr, stdout).
fn sched(entry: &str, extra: &[&str]) -> (i32, String, String) {
    let (lib, so) = setup_plugin("sched");
    let spec = format!("--vhpi={}:{entry}", so.display());
    let mut args = vec!["run", &lib, &spec];
    args.extend_from_slice(extra);
    args.push("tb");
    yavs(&args)
}

/// The `vhpi:` lines and reports of a run, without the report process lines.
fn trace(out: &str) -> String {
    out.lines()
        .filter(|l| !l.starts_with("    Process"))
        .map(|l| format!("{l}\n"))
        .collect()
}

#[test]
fn after_delay_finishes_at_t_in_resolution_units() {
    let (code, err, out) = sched("timed_finish", &["--resolution=ns"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("vhpi: resolution=1000000\n"), "{out}");
    assert!(out.contains("vhpi: finish t=30 d=0 cb_time=30\n"), "{out}");
    assert!(out.contains("FINISH called with status 0"), "{out}");
    assert!(out.contains("vhpi: end t=30 d=0"), "{out}");
    // Without --resolution times count in femtoseconds: 30 fs, before the first event.
    let (code, err, out) = sched("timed_finish", &[]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("vhpi: resolution=1\n"), "{out}");
    assert!(out.contains("vhpi: finish t=30 d=0"), "{out}");
    assert!(!out.contains("s='1'"), "{out}");
}

#[test]
fn after_delay_chains_and_repeats() {
    let (code, err, out) = sched("chain", &["--resolution=ns"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        trace(&out),
        "vhpi: start t=0 d=0 cb_time=0\n\
         vhpi: remove pending=0\n\
         ** Note: 0fs+0: s='0'\n\
         ** Note: 0fs+0: postponed s='0'\n\
         vhpi: rep t=7 d=0 cb_time=7\n\
         vhpi: link t=10 d=0 cb_time=10\n\
         ** Note: 10000000fs+0: s='1'\n\
         ** Note: 10000000fs+0: postponed s='1'\n\
         vhpi: rep t=14 d=0 cb_time=14\n\
         vhpi: link t=20 d=0 cb_time=20\n\
         ** Note: 20000000fs+0: s='0'\n\
         ** Note: 20000000fs+0: postponed s='0'\n\
         vhpi: rep t=21 d=0 cb_time=21\n\
         vhpi: remove rep=0\n\
         vhpi: link t=30 d=0 cb_time=30\n\
         vhpi: link t=40 d=0 cb_time=40\n\
         vhpi: end t=40 d=0 cb_time=40\n"
    );
}

#[test]
fn read_write_before_read_only_and_phase_order() {
    let (code, err, out) = sched("order", &["--resolution=ns"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(trace(&out), ORDER);
    for threads in ["--threads=1", "--threads=4"] {
        let (code, err, again) = sched("order", &["--resolution=ns", threads]);
        assert_eq!(code, 0, "{err}");
        assert_eq!(again, out, "{threads}");
    }
}

/// Every phase once per occurrence. Start and end of processes bracket each cycle's processes
/// (#317; end of processes also fires after initialization's process run), start of postponed only when postponed processes run. The read-write and read-only
/// phases also run at time 0 when no delta follows initialization (#347). The read-write phase (last known delta) runs before the
/// postponed processes and the read-only phase (end of time step) after them; an after-delay of
/// 0 from it creates one more delta; a callback registered in the read-only phase for the same
/// phase runs at the next time step.
const ORDER: &str = "vhpi: start_of_sim t=0 d=0 cb_time=0\n\
     vhpi: start_of_init t=0 d=0 cb_time=0\n\
     ** Note: 0fs+0: s='0'\n\
     ** Note: 0fs+0: postponed s='0'\n\
     vhpi: end_procs t=0 d=0 cb_time=0\n\
     vhpi: end_of_init t=0 d=0 cb_time=0\n\
     vhpi: read_write t=0 d=0 cb_time=0\n\
     vhpi: next_cycle t=0 d=1 cb_time=0\n\
     vhpi: after0 t=0 d=1 cb_time=0\n\
     vhpi: start_procs t=0 d=1 cb_time=0\n\
     vhpi: end_procs t=0 d=1 cb_time=0\n\
     vhpi: read_write t=0 d=1 cb_time=0\n\
     vhpi: read_only t=0 d=1 cb_time=0\n\
     vhpi: next_time_step_once t=10 d=0 cb_time=10\n\
     vhpi: next_time_step t=10 d=0 cb_time=10\n\
     vhpi: next_cycle t=10 d=0 cb_time=10\n\
     vhpi: start_procs t=10 d=0 cb_time=10\n\
     ** Note: 10000000fs+0: s='1'\n\
     vhpi: end_procs t=10 d=0 cb_time=10\n\
     vhpi: read_write t=10 d=0 cb_time=10\n\
     vhpi: start_postponed t=10 d=0 cb_time=10\n\
     ** Note: 10000000fs+0: postponed s='1'\n\
     vhpi: read_only t=10 d=0 cb_time=10\n\
     vhpi: late_eots t=10 d=0 cb_time=10\n\
     vhpi: next_time_step t=20 d=0 cb_time=20\n\
     vhpi: next_cycle t=20 d=0 cb_time=20\n\
     vhpi: start_procs t=20 d=0 cb_time=20\n\
     ** Note: 20000000fs+0: s='0'\n\
     vhpi: end_procs t=20 d=0 cb_time=20\n\
     vhpi: read_write t=20 d=0 cb_time=20\n\
     vhpi: start_postponed t=20 d=0 cb_time=20\n\
     ** Note: 20000000fs+0: postponed s='0'\n\
     vhpi: read_only t=20 d=0 cb_time=20\n\
     vhpi: end t=20 d=0 cb_time=20\n";

#[test]
fn enable_disable_and_cheap_disabled_callbacks() {
    let (code, err, out) = sched("toggle", &["--resolution=ns"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(trace(&out), TOGGLE);
    let (code, err, again) = sched("toggle", &["--resolution=ns", "--threads=4"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(again, out);
}

/// The disabled callback runs from 25 to 40 ns only: it is enabled after the 20 ns time step
/// began and disabled after the 40 ns one; the 100000 disabled callbacks never run.
const TOGGLE: &str = "vhpi: registered state=1\n\
     vhpi: get_cb_info=0\n\
     vhpi: info reason=1013 user_data=next_time_step\n\
     vhpi: start_of_sim t=0 d=0 cb_time=0\n\
     ** Note: 0fs+0: s='0'\n\
     ** Note: 0fs+0: postponed s='0'\n\
     ** Note: 10000000fs+0: s='1'\n\
     ** Note: 10000000fs+0: postponed s='1'\n\
     vhpi: enable t=20 d=0 cb_time=20\n\
     vhpi: enable=0\n\
     vhpi: enabled state=0\n\
     vhpi: matured state=2 remove=0\n\
     ** Note: 20000000fs+0: s='0'\n\
     ** Note: 20000000fs+0: postponed s='0'\n\
     vhpi: next_time_step t=25 d=0 cb_time=25\n\
     vhpi: next_time_step t=30 d=0 cb_time=30\n\
     vhpi: next_time_step t=35 d=0 cb_time=35\n\
     vhpi: next_time_step t=40 d=0 cb_time=40\n\
     vhpi: disable t=40 d=0 cb_time=40\n\
     vhpi: disable=0\n\
     vhpi: disabled state=1\n\
     vhpi: stop t=60 d=0 cb_time=60\n\
     vhpi: ticks=11\n\
     ** Note: 60000000fs+0: FINISH called with status 0\n";

#[test]
fn callback_misuse_is_reported() {
    let (code, err, out) = sched("errors", &[]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("vhpi: no time=null\n"), "{out}");
    assert!(
        out.contains("vhpi: error: vhpi_register_cb: after-delay callback without time"),
        "{out}"
    );
    assert!(err.contains("warning[W0903]"), "{err}");
    assert!(
        out.contains("vhpi: remove=0\nvhpi: remove again=1\n"),
        "{out}"
    );
}

/// The design hierarchy object model (#269): a plugin walks regions, declarations and
/// sub-elements and looks objects up by name and index; its output is
/// `yavs-vhpi/tests/hier/expected.txt` (`YAVS_BLESS=1` rewrites it).
#[test]
fn plugin_walks_the_design_hierarchy() {
    let dir = vhpi_dir().join("tests/hier");
    let vhdl = std::fs::read_to_string(dir.join("design.vhd")).expect("design");
    let (lib, so) = setup_with("hier", &vhdl);
    let (code, err, out) = yavs(&["run", &lib, &format!("--vhpi={}", so.display()), "top"]);
    assert_eq!(code, 0, "{err}");
    // Only the deliberate compare of a released handle is reported.
    assert_eq!(err.matches("warning[").count(), 1, "{err}");
    assert!(err.contains("W0903"), "{err}");
    let expected = dir.join("expected.txt");
    if std::env::var_os("YAVS_BLESS").is_some() {
        std::fs::write(&expected, &out).expect("bless");
    }
    let want = std::fs::read_to_string(&expected)
        .expect("expected dump")
        .replace("\r\n", "\n");
    assert_eq!(out, want);
}

/// `vhpi_get_value` (#270): a plugin reads scalars, logic vectors, integers, reals, time,
/// physical and enumeration values, strings, record fields and 2-D arrays in every format,
/// with the buffer-size protocol; its output is `yavs-vhpi/tests/values/expected.txt`
/// (`YAVS_BLESS=1` rewrites it).
#[test]
fn plugin_reads_values_in_every_format() {
    let dir = vhpi_dir().join("tests/values");
    let vhdl = std::fs::read_to_string(dir.join("design.vhd")).expect("design");
    let (lib, so) = setup_with("values", &vhdl);
    let (code, err, out) = yavs(&["run", &lib, &format!("--vhpi={}", so.display()), "top"]);
    assert_eq!(code, 0, "{err}");
    // Formats that do not apply are misuse; records and pointers are not values.
    assert!(err.contains("W0903"), "{err}");
    assert!(
        err.contains("unsupported VHPI call: vhpi_get_value of a record"),
        "{err}"
    );
    assert!(!err.contains("bug in yavs"), "{err}");
    let expected = dir.join("expected.txt");
    if std::env::var_os("YAVS_BLESS").is_some() {
        std::fs::write(&expected, &out).expect("bless");
    }
    let want = std::fs::read_to_string(&expected)
        .expect("expected dump")
        .replace("\r\n", "\n");
    assert_eq!(out, want);
}

/// Runs `top` of `yavs-vhpi/tests/put/design.vhd` with the `put` plugin entry `entry`.
fn put(entry: &str, extra: &[&str]) -> (i32, String, String) {
    let vhdl = std::fs::read_to_string(vhpi_dir().join("tests/put/design.vhd")).expect("design");
    let (lib, so) = setup_with("put", &vhdl);
    let spec = format!("--vhpi={}:{entry}", so.display());
    let mut args = vec!["run", &lib, &spec, "--resolution=ns"];
    args.extend_from_slice(extra);
    args.push("top");
    yavs(&args)
}

/// A counter whose clock and reset come only from `vhpi_put_value`, value-change callbacks on
/// signals, ports and sub-elements, and force, release and deposit on a driven signal (#271,
/// #272). The trace is `yavs-vhpi/tests/put/expected.txt` (`YAVS_BLESS=1` rewrites it), the
/// same at 1 and 4 threads.
#[test]
fn vhpi_drives_a_clocked_design() {
    let (code, err, out) = put("clocked", &[]);
    assert_eq!(code, 0, "{err}");
    assert!(!err.contains("warning["), "{err}");
    let expected = vhpi_dir().join("tests/put/expected.txt");
    if std::env::var_os("YAVS_BLESS").is_some() {
        std::fs::write(&expected, &out).expect("bless");
    }
    let want = std::fs::read_to_string(&expected)
        .expect("expected trace")
        .replace("\r\n", "\n");
    assert_eq!(out, want);
    let (code, err, again) = put("clocked", &["--threads=4"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(again, out, "--threads=4");
}

/// Constants, generics, records and read-only phases cannot be written; bad values are
/// misuse (W0903).
#[test]
fn vhpi_put_value_misuse_is_reported() {
    let (code, err, out) = put("errors", &[]);
    assert_eq!(code, 0, "{err}");
    let expected = vhpi_dir().join("tests/put/errors.txt");
    if std::env::var_os("YAVS_BLESS").is_some() {
        std::fs::write(&expected, &out).expect("bless");
    }
    let want = std::fs::read_to_string(&expected)
        .expect("expected trace")
        .replace("\r\n", "\n");
    assert_eq!(out, want);
    assert!(err.contains("warning[W0903]"), "{err}");
    assert!(err.contains("warning[W0904]"), "{err}");
}

/// `vhpiStrVal` text fills a string from the left: shorter text is padded with NUL, longer
/// text is cut to the array (#345).
#[test]
fn vhpi_put_string_shorter_or_longer_than_the_array() {
    let (code, err, out) = put("strings", &[]);
    assert_eq!(code, 0, "{err}");
    assert!(!err.contains("warning["), "{err}");
    let want = "vhpi: str n=8 97 98 99 100 101 102 103 104\n\
                vhpi: Testing: ok\n\
                vhpi: str n=8 84 101 115 116 105 110 103 0\n\
                vhpi: ab: ok\n\
                vhpi: str n=8 97 98 0 0 0 0 0 0\n\
                vhpi: TooLongStr: ok\n\
                vhpi: str n=8 84 111 111 76 111 110 103 83\n";
    assert!(out.contains(want), "{out}");
}

/// A value-change callback that removes itself and registers a new one fires once per cycle,
/// though the vector's changed elements form several runs (#346).
#[test]
fn vhpi_value_change_rearmed_fires_once_per_cycle() {
    let (code, err, out) = put("edges", &[]);
    assert_eq!(code, 0, "{err}");
    let fires: Vec<_> = out.lines().filter(|l| l.starts_with("vhpi: e=")).collect();
    assert_eq!(
        fires,
        ["vhpi: e=01011010 t=10 d=0", "vhpi: e=00000000 t=20 d=0"],
        "{out}"
    );
}

/// The lines of `out` that start with `prefix` and name one of `sigs`.
fn vhpi_lines<'a>(out: &'a str, prefix: &str, sigs: &[&str]) -> Vec<&'a str> {
    out.lines()
        .filter(|l| {
            l.strip_prefix(prefix)
                .and_then(|r| r.split_whitespace().next())
                .is_some_and(|s| sigs.contains(&s))
        })
        .collect()
}

/// The kernel reports the changed signals of a cycle in declaration order, whatever the order
/// the callbacks were registered in (the report order the next tests depend on, #496).
#[test]
fn vhpi_reports_changed_signals_in_declaration_order() {
    let (code, err, out) = put("order", &[]);
    assert_eq!(code, 0, "{err}");
    let (code, err, again) = put("order", &["--threads=4"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(again, out, "--threads=4");
    let at10: Vec<_> = out
        .lines()
        .filter(|l| l.starts_with("vhpi: f") && l.contains("t=10 "))
        .collect();
    assert_eq!(
        at10,
        [
            "vhpi: fa changed t=10 d=0",
            "vhpi: fb changed t=10 d=0",
            "vhpi: fc changed t=10 d=0",
            "vhpi: fd changed t=10 d=0",
        ],
        "{out}"
    );
}

/// The first of two triggers that fire in the same cycle, awaited again from inside the
/// callback (removing both and registering both anew), sees the other one's change too, but a
/// re-armed trigger on a signal already reported in the cycle waits for the next (#496). The
/// pair fd, fc registers the trigger that is reported second first: same result either way.
#[test]
fn vhpi_repeated_first_of_coincident_triggers_misses_no_edge() {
    let (code, err, out) = put("first", &[]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(
        vhpi_lines(&out, "vhpi: first ", &["d", "e"]),
        [
            "vhpi: first d t=10 d=0",
            "vhpi: first e t=10 d=0",
            "vhpi: first e t=20 d=0",
            "vhpi: first d t=30 d=0",
            "vhpi: first d t=50 d=0",
            "vhpi: first d t=70 d=0",
        ],
        "{out}"
    );
    let each_time = |a: &str, b: &str| -> Vec<String> {
        [10, 30, 50]
            .iter()
            .flat_map(|t| [a, b].map(|s| format!("vhpi: first {s} t={t} d=0")))
            .collect()
    };
    assert_eq!(
        vhpi_lines(&out, "vhpi: first ", &["fa", "fb"]),
        each_time("fa", "fb"),
        "{out}"
    );
    // fc is reported before fd although fd was registered first.
    assert_eq!(
        vhpi_lines(&out, "vhpi: first ", &["fc", "fd"]),
        each_time("fc", "fd"),
        "{out}"
    );
    let (code, err, again) = put("first", &["--threads=4"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(again, out, "--threads=4");
}

/// A callback registered from a value-change callback on a signal that changes in the same
/// cycle sees that change once, also when the kernel reported the signal before the one whose
/// callback registers it, and re-arming it from inside itself does not repeat the change
/// (#496). The same at 4 threads.
#[test]
fn vhpi_late_registration_sees_a_change_reported_before_it() {
    let (code, err, out) = put("seq", &[]);
    assert_eq!(code, 0, "{err}");
    let want = |a: &str, b: &str| -> Vec<String> {
        [10, 30, 50]
            .iter()
            .flat_map(|t| [a, b].map(|s| format!("vhpi: seq {s} t={t} d=0")))
            .collect()
    };
    // fa then fb: the second signal is reported after the first; fd then fc: before it.
    assert_eq!(
        vhpi_lines(&out, "vhpi: seq ", &["fa", "fb"]),
        want("fa", "fb"),
        "{out}"
    );
    // At 10 ns fc was reported before fd registered its callback; later it is there already.
    assert_eq!(
        vhpi_lines(&out, "vhpi: seq ", &["fc", "fd"]),
        [
            "vhpi: seq fd t=10 d=0",
            "vhpi: seq fc t=10 d=0",
            "vhpi: seq fc t=30 d=0",
            "vhpi: seq fd t=30 d=0",
            "vhpi: seq fc t=50 d=0",
            "vhpi: seq fd t=50 d=0",
        ],
        "{out}"
    );
    let (code, err, again) = put("seq", &["--threads=4"]);
    assert_eq!(code, 0, "{err}");
    assert_eq!(again, out, "--threads=4");
}

/// A write from the read-write phase registered at load time lands at time 0 even though the
/// design has no delta at time 0, and reaches an output port through a transport delay 10 ns
/// later (#347).
#[test]
fn vhpi_read_write_at_time_zero_reaches_a_transport_delay() {
    let vhdl = "entity top is port (ti : in integer; tout : out integer); end;\n\
                architecture a of top is begin tout <= transport ti after 10 ns; end;\n";
    let (lib, so) = setup_with("put", vhdl);
    let spec = format!("--vhpi={}:transport", so.display());
    let (code, err, out) = yavs(&["run", &lib, &spec, "--resolution=ns", "top"]);
    assert_eq!(code, 0, "{err}");
    assert!(
        out.contains("vhpi: read-write t=0 d=0\nvhpi: put ti: ok\n"),
        "{out}"
    );
    assert!(out.contains("vhpi: tout=0 t=10 d=0\n"), "{out}");
}

/// Runs entity `top` of `yavs-vhpi/tests/foreign/design.vhd` with the `foreign` plugin.
fn foreign(top: &str, extra: &[&str]) -> (i32, String, String) {
    let vhdl =
        std::fs::read_to_string(vhpi_dir().join("tests/foreign/design.vhd")).expect("design");
    let (lib, so) = setup_with("foreign", &vhdl);
    let spec = format!("--vhpi={}", so.display());
    let extras = format!("--vhpi={}:foreign_extras", so.display());
    let mut args = vec!["run", &lib, &spec, &extras];
    args.extend_from_slice(extra);
    args.push(top);
    yavs(&args)
}

/// Foreign functions and procedures registered with `vhpi_register_foreignf` and called from
/// VHDL (#274): scalar and array parameters, results through `vhpi_put_value` (array results
/// sized with `vhpiSizeConstraint`, #330), `out` and `inout` parameters, record fields, access
/// values and signal parameters driven through the caller's driver (#330), registration
/// queries and registration misuse. The plugin's output is
/// `yavs-vhpi/tests/foreign/expected.txt` (`YAVS_BLESS=1` rewrites it).
#[test]
fn vhpi_foreign_subprograms() {
    let (code, err, out) = foreign("top", &[]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("foreign ok"), "{out}{err}");
    assert!(out.contains("signals ok"), "{out}{err}");
    let expected = vhpi_dir().join("tests/foreign/expected.txt");
    let plugin: String = out
        .lines()
        .filter(|l| !l.starts_with("**") && !l.starts_with("    Process"))
        .map(|l| format!("{l}\n"))
        .collect();
    if std::env::var_os("YAVS_BLESS").is_some() {
        std::fs::write(&expected, &plugin).expect("bless");
    }
    let want = std::fs::read_to_string(&expected)
        .expect("expected trace")
        .replace("\r\n", "\n");
    assert_eq!(plugin, want);
    // The write to an in parameter and the registration misuse are reported.
    assert!(err.matches("warning[W0903]").count() >= 3, "{err}");
    // Same result with more threads.
    let (code, _, again) = foreign("top", &["--threads=4"]);
    assert_eq!(code, 0);
    assert_eq!(again, out);
}

/// A model that is not registered warns before time 0 and fails only when called; a function
/// that sets no result is a runtime error.
#[test]
fn vhpi_foreign_missing_model_and_result() {
    let (code, err, out) = foreign("lazy", &[]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("lazy ok"), "{out}");
    assert!(err.contains("warning[W0108]"), "{err}");
    assert!(err.contains("VHPI model `plug missing`"), "{err}");
    let (code, err, _) = foreign("miss", &[]);
    assert_ne!(code, 0);
    assert!(err.contains("error[E0107]"), "{err}");
    let (code, err, _) = foreign("nores", &[]);
    assert_ne!(code, 0);
    assert!(err.contains("did not set its result"), "{err}");
    // An unconstrained array result needs its size first (#330).
    let (code, err, out) = foreign("nosize", &[]);
    assert_ne!(code, 0);
    assert!(out.contains("vhpiSizeConstraint first"), "{out}");
    assert!(err.contains("did not set its result"), "{err}");
}

/// Runs entity `top` of `yavs-vhpi/tests/farch/design.vhd` with the `farch` plugin.
fn farch(top: &str, extra: &[&str]) -> (i32, String, String) {
    let vhdl = std::fs::read_to_string(vhpi_dir().join("tests/farch/design.vhd")).expect("design");
    let (lib, so) = setup_with("farch", &vhdl);
    let spec = format!("--vhpi={}", so.display());
    let mut args = vec!["run", &lib, &spec];
    args.extend_from_slice(extra);
    args.push(top);
    yavs(&args)
}

/// A foreign architecture (#330): `elabf` and `execf` run before the simulation, the model
/// creates a process and drivers (`vhpi_create`) and drives its ports with
/// `vhpi_schedule_transaction` (inertial pulse rejection, transport) from value-change
/// callbacks; two instances get one process each. The plugin's output is
/// `yavs-vhpi/tests/farch/expected.txt` (`YAVS_BLESS=1` rewrites it), the same at 4 threads.
/// A model no plugin registered is `E0905`.
#[test]
fn vhpi_foreign_architecture() {
    let (code, err, out) = farch("top", &[]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("farch ok"), "{out}{err}");
    let expected = vhpi_dir().join("tests/farch/expected.txt");
    if std::env::var_os("YAVS_BLESS").is_some() {
        std::fs::write(&expected, &out).expect("bless");
    }
    let want = std::fs::read_to_string(&expected)
        .expect("expected trace")
        .replace("\r\n", "\n");
    assert_eq!(out, want);
    let (code, _, again) = farch("top", &["--threads=4"]);
    assert_eq!(code, 0);
    assert_eq!(again, out);
    let (code, err, out) = farch("two", &[]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("two ok"), "{out}{err}");
    let (code, err, _) = farch("lost", &[]);
    assert_eq!(code, 2);
    assert!(err.contains("error[E0905]"), "{err}");
    assert!(err.contains("`plug lost`"), "{err}");
}

/// Runs entity `top` of `yavs-vhpi/tests/libapp/design.vhd` with the `libapp` plugin.
fn libapp(top: &str, extra: &[&str]) -> (i32, String, String) {
    let vhdl = std::fs::read_to_string(vhpi_dir().join("tests/libapp/design.vhd")).expect("design");
    let (lib, so) = setup_with("libapp", &vhdl);
    let spec = format!("--vhpi={}", so.display());
    let mut args = vec!["run", &lib, &spec];
    args.extend_from_slice(extra);
    args.push(top);
    yavs(&args)
}

/// Library and application foreign models (#357): a library model's `elabf` runs when a
/// foreign model of its library is bound and its `execf` at the start of the simulation; an
/// application model's `elabf` and `execf` run at the start of the simulation, with or
/// without foreign models in the design; both before the start-of-simulation callbacks; a
/// library model whose library the design does not use never runs. The same at 4 threads.
#[test]
fn vhpi_library_and_application_models() {
    let lines = |out: &str| -> Vec<String> {
        out.lines()
            .filter(|l| !l.starts_with("**") && !l.starts_with("    Process"))
            .map(str::to_string)
            .collect()
    };
    let (code, err, out) = libapp("top", &[]);
    assert_eq!(code, 0, "{err}");
    let expected = vhpi_dir().join("tests/libapp/expected.txt");
    let got: String = lines(&out).iter().map(|l| format!("{l}\n")).collect();
    if std::env::var_os("YAVS_BLESS").is_some() {
        std::fs::write(&expected, &got).expect("bless");
    }
    let want = std::fs::read_to_string(&expected)
        .expect("expected trace")
        .replace("\r\n", "\n");
    assert_eq!(got, want);
    assert!(!out.contains("idle"), "{out}");
    let (code, _, again) = libapp("top", &["--threads=4"]);
    assert_eq!(code, 0);
    assert_eq!(again, out);
    // No foreign model in the design: the application model still runs, the library model
    // does not.
    let (code, err, out) = libapp("none", &[]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("app elabf"), "{out}");
    assert!(out.contains("app execf"), "{out}");
    assert!(!out.contains("used"), "{out}");
}

/// Process and shared variables (#309): read at the end of initialization and in callbacks,
/// deposited (scalars, a logic vector and one of its elements, record fields, a string, a
/// variable of a process suspended in a procedure, an architecture's and a package's shared
/// variable) and seen by the processes when they resume; forces and read-only phases are
/// misuse. The trace is `yavs-vhpi/tests/vars/expected.txt`
/// (`YAVS_BLESS=1` rewrites it), the same at 1 and 4 threads.
#[test]
fn vhpi_reads_and_deposits_variables() {
    let dir = vhpi_dir().join("tests/vars");
    let vhdl = std::fs::read_to_string(dir.join("design.vhd")).expect("design");
    let (lib, so) = setup_args("vars", &vhdl, &["--relaxed"]);
    let spec = format!("--vhpi={}:vars", so.display());
    let run = |threads: &str| {
        let args = [
            "run",
            "--relaxed",
            &lib,
            &spec,
            "--resolution=ns",
            threads,
            "top",
        ];
        yavs(&args)
    };
    // Compiled code too: a process whose declarative part runs early (`prestart`) stops
    // before its statements, also when it is compiled.
    let tier = |t: &str| {
        let args = [
            "run",
            "--relaxed",
            &lib,
            &spec,
            "--resolution=ns",
            t,
            "--tier-heat=1,0,0",
            "top",
        ];
        yavs(&args)
    };
    let (code, err, out) = run("--threads=1");
    assert_eq!(code, 0, "{err}");
    assert!(!err.contains("bug in yavs"), "{err}");
    let expected = dir.join("expected.txt");
    if std::env::var_os("YAVS_BLESS").is_some() {
        std::fs::write(&expected, &out).expect("bless");
    }
    let want = std::fs::read_to_string(&expected)
        .expect("expected trace")
        .replace("\r\n", "\n");
    assert_eq!(out, want);
    let (code, err, again) = run("--threads=4");
    assert_eq!(code, 0, "{err}");
    assert_eq!(again, out, "--threads=4");
    // The profiler wraps every process: the plugin still reaches their variables (#580).
    let (code, err, again) = run("--profile");
    assert_eq!(code, 0, "{err}");
    assert_eq!(again, out, "--profile");
    assert!(err.contains("profile: processes"), "{err}");
    if cfg!(feature = "llvm") {
        for t in ["--tier=llvm", "--tier=auto"] {
            let (code, err, again) = tier(t);
            assert_eq!(code, 0, "{err}");
            assert_eq!(again, out, "{t}");
        }
    }
}

/// `--vhpi` under `--dap` (#325): the plugin runs and ends the simulation. Over a port: the
/// plugin's own prints go to stdout.
#[test]
fn plugin_under_dap() {
    use std::io::{BufRead, BufReader, Read};
    use std::process::Stdio;
    let (lib, so) = setup();
    let mut child = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(["--color", "never", "run", &lib])
        .arg(format!("--vhpi={}", so.display()))
        .args(["--dap=0", "top"])
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("yavs");
    let mut err = BufReader::new(child.stderr.take().expect("stderr"));
    let mut line = String::new();
    err.read_line(&mut line).expect("stderr");
    let port = line
        .trim()
        .strip_prefix("yavs: DAP listening on 127.0.0.1:")
        .unwrap_or_else(|| panic!("first stderr line: {line}"));
    let s = std::net::TcpStream::connect(format!("127.0.0.1:{port}")).expect("connect");
    let mut w = s.try_clone().expect("clone");
    let mut r = BufReader::new(s);
    let mut req = |seq: i64, command: &str| {
        let m =
            serde_json::json!({"seq": seq, "type": "request", "command": command, "arguments": {}});
        yavs_dap::write_message(&mut w, &m).expect("write");
    };
    req(1, "initialize");
    req(2, "launch");
    req(3, "configurationDone");
    let mut text = String::new();
    let exit = loop {
        let m = yavs_dap::read_message(&mut r)
            .expect("read")
            .unwrap_or_else(|| panic!("end of input; output so far: {text}"));
        if m["event"] == "output" {
            text += m["body"]["output"].as_str().unwrap_or_default();
        }
        if m["event"] == "exited" {
            break m;
        }
    };
    assert_eq!(exit["body"]["exitCode"], 0, "{text}");
    assert!(text.contains("FINISH called with status 0"), "{text}");
    req(4, "disconnect");
    while yavs_dap::read_message(&mut r).expect("read").is_some() {}
    let mut out = String::new();
    child
        .stdout
        .take()
        .expect("stdout")
        .read_to_string(&mut out)
        .expect("stdout");
    assert!(child.wait().expect("wait").success());
    assert!(out.contains("plugin: start of simulation"), "{out}");
}

/// #389: a plugin's prints must not reach the DAP stream on standard output; every byte of
/// stdout is a framed message, and the prints come out on stderr.
#[test]
fn plugin_prints_stay_off_the_dap_stream() {
    use std::io::Write as _;
    use std::process::Stdio;
    let (lib, so) = setup();
    let mut child = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args([
            "run",
            &lib,
            &format!("--vhpi={}", so.display()),
            "--dap=stdio",
            "tb",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("yavs");
    let mut w = child.stdin.take().expect("stdin");
    let mut seq = 0;
    // The adapter ends the session itself after configurationDone when the design finishes;
    // `disconnect` makes that deterministic.
    for c in ["initialize", "launch", "configurationDone", "disconnect"] {
        seq += 1;
        let b = format!(r#"{{"seq":{seq},"type":"request","command":"{c}","arguments":{{}}}}"#);
        if write!(w, "Content-Length: {}\r\n\r\n{b}", b.len())
            .and_then(|()| w.flush())
            .is_err()
        {
            break;
        }
    }
    drop(w);
    let o = child.wait_with_output().expect("wait");
    let mut rest = o.stdout.as_slice();
    let mut messages = 0;
    while !rest.is_empty() {
        let s = std::str::from_utf8(rest).expect("utf-8");
        let (head, tail) = s.split_once("\r\n\r\n").expect("a header");
        let n: usize = head
            .strip_prefix("Content-Length: ")
            .and_then(|n| n.parse().ok())
            .unwrap_or_else(|| panic!("stray output on the DAP stream: {head:?}"));
        rest = &tail.as_bytes()[n..];
        messages += 1;
    }
    assert!(messages >= 4, "{}", String::from_utf8_lossy(&o.stdout));
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(err.contains("plugin: start of simulation"), "{err}");
}

/// Design of the `extras` plugin: user attributes, a generic, a for generate, a process
/// variable and a signal that toggles every 10 ns until 30 ns.
const EXTRAS: &str = "entity top is generic (g : integer := 3); end;\n\
    architecture a of top is\n\
    attribute mark : string;\n\
    attribute weight : integer;\n\
    signal s : bit := '0';\n\
    attribute mark of s : signal is \"fast\";\n\
    attribute weight of s : signal is 7;\n\
    constant c : integer := 1;\n\
    constant c2 : integer := 2;\n\
    attribute weight of c2 : constant is g * 2;\n\
    begin\n\
    g1: for i in 2 to 3 generate\n\
    signal t : bit;\n\
    begin end generate;\n\
    p: process variable v : integer := 0; begin\n\
    for k in 1 to 3 loop wait for 10 ns; s <= not s; end loop; wait;\n\
    end process;\n\
    end;\n";

/// Runs `top` of [`EXTRAS`] with the `extras` plugin entry `entry`.
fn extras(entry: &str) -> (i32, String, String) {
    let (lib, so) = setup_with("extras", EXTRAS);
    let spec = format!("--vhpi={}:{entry}", so.display());
    yavs(&["run", &lib, &spec, "--resolution=ns", "top"])
}

/// User attribute specifications (`vhpiAttrSpecs`, `vhpiAttrDecl`, their values) and the
/// tool and region properties of #143.
#[test]
fn vhpi_user_attributes_and_properties() {
    let (code, err, out) = extras("attrs");
    assert_eq!(code, 0, "{err}");
    assert!(!err.contains("warning["), "{err}");
    let lines: Vec<&str> = out
        .lines()
        .filter(|l| !l.starts_with("    Process"))
        .collect();
    assert_eq!(
        lines,
        [
            "top:s: iterator yes error=0",
            "  vhpiAttrSpecK mark full=:top:s'mark decl=vhpiAttrDeclK mark",
            "    value: str fast",
            "  vhpiAttrSpecK weight full=:top:s'weight decl=vhpiAttrDeclK weight",
            "    value: int 7",
            "top:c: iterator NULL error=0",
            "by name s'weight: vhpiAttrSpecK",
            "root: gens=1 ports=0",
            "g1(3): label=g1 index=3 line=12",
            "p:v: shared=0",
            "tool: capabilities=307",
        ],
        "{out}"
    );
}

/// What does not apply to user attributes is misuse, never a crash or an internal error.
#[test]
fn vhpi_user_attribute_misuse() {
    let (code, err, out) = extras("attrs_misuse");
    assert_eq!(code, 0, "{err}");
    assert!(!err.contains("bug in yavs"), "{err}");
    let lines: Vec<&str> = out
        .lines()
        .filter(|l| !l.starts_with("    Process"))
        .collect();
    assert_eq!(
        lines,
        [
            "put: rc=1 error=1",
            "value change: NULL error=1",
            "decl value:",
            "    value: error",
            "generic-valued weight:",
            "    value: error",
        ],
        "{out}"
    );
    assert_eq!(err.matches("W0903").count(), 3, "{err}");
    // An attribute value analysis cannot fold (it depends on a generic) is not read yet.
    assert!(
        err.contains("W0904") && err.contains("without an elaborated value"),
        "{err}"
    );
}

/// The foreign-model callback reasons: a time-out, a repeating one removed after two, and a
/// sensitivity callback on a signal.
#[test]
fn vhpi_time_out_and_sensitivity_callbacks() {
    let (code, err, out) = extras("timeout");
    assert_eq!(code, 0, "{err}");
    let lines: Vec<&str> = out
        .lines()
        .filter(|l| !l.starts_with("    Process"))
        .collect();
    assert_eq!(
        lines,
        [
            "timeout t=5",
            "rep t=8",
            "sensitivity t=10 s=1",
            "rep t=16",
            "sensitivity t=20 s=0",
            "sensitivity t=30 s=1",
        ],
        "{out}"
    );
}

/// `vhpi_get_next_time` (#828): the next activity in each end-of-time-step phase, none after
/// the last; refused outside that phase.
#[test]
fn vhpi_next_time_in_the_read_only_phase() {
    let (code, err, out) = extras("nexttime");
    assert_eq!(code, 0, "{err}");
    assert_eq!(err.matches("W0903").count(), 1, "{err}");
    let lines: Vec<&str> = out
        .lines()
        .filter(|l| !l.starts_with("    Process"))
        .collect();
    assert_eq!(
        lines,
        [
            "at load: rc=1 error=1",
            "end of time step t=0: rc=0 next=10",
            "end of time step t=10: rc=0 next=20",
            "end of time step t=20: rc=0 next=30",
            "end of time step t=30: rc=-1 next=0",
        ],
        "{out}"
    );
}

/// `vhpi_format_value` between formats with no object: integers to radix strings, bit
/// strings to logic vectors and hex; an integer is not a real, a real has no bits (`W0903`).
#[test]
fn vhpi_format_value_converts_between_formats() {
    let (code, err, out) = extras("format");
    assert_eq!(code, 0, "{err}");
    assert_eq!(err.matches("W0903").count(), 3, "{err}");
    let lines: Vec<&str> = out
        .lines()
        .filter(|l| !l.starts_with("    Process"))
        .collect();
    assert_eq!(
        lines,
        [
            "int 10 -> 1: 1010",
            "int 255 -> 4: FF",
            "int -42 -> 3: -42",
            "int 3 -> 8: rc=-1 error=1",
            "bin 10X1 -> logic vec: rc=0 n=4 3 2 1 3",
            "bin 10100101 -> hex: rc=0 A5",
            "real -> bin: rc=-1 error=1",
            "NULL: rc=-1 error=1",
        ],
        "{out}"
    );
}
