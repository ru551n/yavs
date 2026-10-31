//! `yavs vendor vivado` (#119): the errors without an install and the build on a small tree made
//! here run everywhere; the primitives of a real install run when `XILINX_VIVADO` names one.

mod common;
use common::tempdir;

use std::path::Path;
use std::process::{Command, Output};

fn yavs(dir: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_yavs"));
    c.env_remove("XILINX_VIVADO")
        .env("YAVS_STD_CACHE", "off")
        .current_dir(dir);
    c
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

fn checked(c: &mut Command) -> String {
    let o = c.output().expect("start yavs");
    assert!(o.status.success(), "{}", text(&o));
    text(&o)
}

#[test]
fn no_install_names_the_fix() {
    let t = tempdir("vendor-none");
    let o = yavs(&t)
        .args(["vendor", "vivado", "--out=o"])
        .output()
        .unwrap();
    let e = text(&o);
    assert_eq!(o.status.code(), Some(2), "{e}");
    assert!(e.contains("error[E0129]"), "{e}");
    assert!(e.contains("XILINX_VIVADO"), "{e}");
    assert!(e.contains("--vivado=DIR"), "{e}");
    assert!(e.contains("settings64.sh"), "{e}");
    assert!(!t.join("o").exists(), "nothing is created on an error");

    for mut c in [
        {
            let mut c = yavs(&t);
            c.args(["vendor", "vivado", "--out=o", "--vivado=nowhere"]);
            c
        },
        {
            let mut c = yavs(&t);
            c.args(["vendor", "vivado", "--out=o"])
                .env("XILINX_VIVADO", "nowhere");
            c
        },
    ] {
        let o = c.output().unwrap();
        let e = text(&o);
        assert_eq!(o.status.code(), Some(2), "{e}");
        assert!(e.contains("error[E0129]"), "{e}");
        assert!(e.contains("nowhere"), "the path is named: {e}");
    }
}

/// A list file that is missing names itself.
#[test]
fn missing_list_is_named() {
    let t = tempdir("vendor-list");
    std::fs::create_dir_all(t.join("inst/data/vhdl/src/unisims")).unwrap();
    let o = yavs(&t)
        .args(["vendor", "vivado", "--out=o", "--vivado=inst"])
        .output()
        .unwrap();
    let e = text(&o);
    assert_eq!(o.status.code(), Some(2), "{e}");
    assert!(e.contains("error[E0129]"), "{e}");
    assert!(e.contains("vhdl_analyze_order"), "{e}");
}

/// An install tree of two libraries with one primitive, `value` its output.
fn fake_install(root: &Path, value: char, extra: bool) {
    let src = root.join("data/vhdl/src");
    let w = |p: &str, s: &str| {
        let p = src.join(p);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, s).unwrap();
    };
    w("unisims/unisim_VPKG.vhd", "package vpkg is end;\n");
    w(
        "unisims/unisim_retarget_VCOMP.vhd",
        "package vcomponents is\n  component prim port (o : out bit); end component;\nend;\n",
    );
    w(
        "unisims/primitive/vhdl_analyze_order",
        if extra {
            "prim.vhd\nold.vhd\n"
        } else {
            "prim.vhd\n"
        },
    );
    w(
        "unisims/primitive/prim.vhd",
        &format!(
            "entity prim is port (o : out bit); end;\narchitecture a of prim is begin o <= '{value}'; end;\n"
        ),
    );
    if extra {
        w("unisims/primitive/old.vhd", "package old is end;\n");
    }
    w("unisims/retarget/vhdl_analyze_order", "");
    w("unimacro/vhdl_analyze_order", "mac.vhd\n");
    w(
        "unimacro/mac.vhd",
        "library unisim;\nuse unisim.vpkg.all;\npackage mac is end;\n",
    );
    w("unifast/primitive/vhdl_analyze_order", "");
}

const USE: &str = "library unisim;\nuse unisim.vcomponents.all;\nentity top is end;\n\
architecture a of top is\n  signal s : bit;\nbegin\n  u : prim port map (o => s);\n  \
process begin wait for 1 ns; report bit'image(s); wait; end process;\nend;\n";

fn run_top(t: &Path) -> String {
    checked(yavs(t).args([
        "analyze",
        "--work=work:w",
        "--map=unisim:o/unisim",
        "top.vhd",
    ]));
    checked(yavs(t).args(["run", "--work=work:w", "--map=unisim:o/unisim", "top"]))
}

#[test]
fn builds_from_an_install_and_again_from_another() {
    let t = tempdir("vendor-fake");
    std::fs::write(t.join("top.vhd"), USE).unwrap();
    fake_install(&t.join("a"), '1', true);
    fake_install(&t.join("b"), '0', false);

    let out = checked(yavs(&t).args(["vendor", "vivado", "--out=o", "--vivado=a"]));
    assert!(out.contains("unisim: 4 files"), "{out}");
    assert!(out.contains("unimacro: 1 files"), "{out}");
    assert!(out.contains("--map=unisim:o"), "{out}");
    assert!(run_top(&t).contains("'1'"));

    // The same again, through the environment; the library still holds the same file.
    let again = checked(
        yavs(&t)
            .args(["vendor", "vivado", "--out=o"])
            .env("XILINX_VIVADO", t.join("a")),
    );
    assert_eq!(again, out);

    // Another install: nothing of the first stays, not even a file it alone has.
    checked(yavs(&t).args(["vendor", "vivado", "--out=o", "--vivado=b"]));
    assert!(run_top(&t).contains("'0'"));
    let index = std::fs::read_to_string(t.join("o/unisim/yavs-lib.toml")).unwrap();
    assert!(!index.contains("old.vhd"), "{index}");

    // The sources are not copied: the libraries name them in the install.
    assert!(index.contains("prim.vhd"), "{index}");
    assert!(!t.join("o/unisim/prim.vhd").exists());
}

#[test]
fn a_source_that_fails_stops_the_build() {
    let t = tempdir("vendor-bad");
    fake_install(&t.join("a"), '1', false);
    std::fs::write(t.join("a/data/vhdl/src/unimacro/mac.vhd"), "package mac is").unwrap();
    let o = yavs(&t)
        .args(["vendor", "vivado", "--out=o", "--vivado=a"])
        .output()
        .unwrap();
    let e = text(&o);
    assert!(!o.status.success(), "{e}");
    assert!(e.contains("mac.vhd"), "{e}");
    assert!(!e.contains("Use them with"), "{e}");
}

/// The testbenches of `tests/vendor` on the libraries of the install in `XILINX_VIVADO`: their
/// checks are the values of the vendor models. Skipped without an install.
#[test]
fn primitives_of_a_real_install() {
    let Some(vivado) = std::env::var_os("XILINX_VIVADO").filter(|v| !v.is_empty()) else {
        eprintln!("skipped: XILINX_VIVADO is not set");
        return;
    };
    if !Path::new(&vivado).join("data/vhdl/src/unisims").is_dir() {
        eprintln!("skipped: XILINX_VIVADO does not name an install");
        return;
    }
    let t = tempdir("vendor-real");
    let lib_dir = t.join("v");
    let mut build = yavs(&t);
    build
        .args(["vendor", "vivado", "--out"])
        .arg(&lib_dir)
        .arg("--vivado")
        .arg(&vivado);
    checked(&mut build);
    let map = [
        format!("--map=unisim:{}", lib_dir.join("unisim").display()),
        format!("--map=unimacro:{}", lib_dir.join("unimacro").display()),
    ];
    let here = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/vendor");
    std::thread::scope(|s| {
        for tb in [
            "tb_v_basic",
            "tb_v_clock",
            "tb_v_bram",
            "tb_v_dsp",
            "tb_v_fifo",
            "tb_v_ddr",
            "tb_v_macro",
        ] {
            let (t, map, here) = (&t, &map, &here);
            s.spawn(move || {
                let work = format!("--work=work:w_{tb}");
                let mut a = yavs(t);
                a.arg("analyze")
                    .arg(&work)
                    .args(map)
                    .arg(here.join(format!("{tb}.vhd")));
                checked(&mut a);
                let mut r = yavs(t);
                r.arg("run")
                    .arg(&work)
                    .arg("--stop-time=1ms")
                    .args(map)
                    .arg(tb);
                let out = checked(&mut r);
                assert!(out.contains(&format!("done {tb}")), "{out}");
            });
        }
    });
}
