//! The vendor primitive libraries of a real install, analyzed with plain `yavs analyze` as the user
//! guide says (#119), and self-checking testbenches of the common primitives on them. Their checks
//! are the values of the vendor's models. Skipped unless `XILINX_VIVADO` names an install.

mod common;
use common::tempdir;

use std::path::{Path, PathBuf};
use std::process::Command;

fn yavs(dir: &Path) -> Command {
    let mut c = Command::new(env!("CARGO_BIN_EXE_yavs"));
    c.current_dir(dir);
    c
}

fn checked(c: &mut Command) -> String {
    let o = c.output().expect("start yavs");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(o.status.success(), "{text}");
    text
}

/// The `.vhd` files of `dir`, sorted as a shell glob sorts them.
fn glob(dir: &Path) -> Vec<PathBuf> {
    let mut files: Vec<PathBuf> = std::fs::read_dir(dir)
        .expect("read directory")
        .map(|e| e.expect("read directory").path())
        .filter(|p| p.extension().is_some_and(|x| x == "vhd"))
        .collect();
    files.sort();
    files
}

#[test]
fn primitives_of_a_real_install() {
    let Some(vivado) = std::env::var_os("XILINX_VIVADO").filter(|v| !v.is_empty()) else {
        eprintln!("skipped: XILINX_VIVADO is not set");
        return;
    };
    let src = Path::new(&vivado).join("data/vhdl/src");
    let u = src.join("unisims");
    if !u.is_dir() {
        eprintln!("skipped: XILINX_VIVADO does not name an install");
        return;
    }
    let t = tempdir("vendor-real");
    // A response file: the command line of the whole library is too long for some platforms.
    let mut unisim = vec![
        u.join("unisim_VPKG.vhd"),
        u.join("unisim_retarget_VCOMP.vhd"),
    ];
    unisim.extend(glob(&u.join("primitive")));
    unisim.extend(glob(&u.join("retarget")));
    let list: Vec<String> = unisim.iter().map(|p| p.display().to_string()).collect();
    std::fs::write(t.join("unisim.rsp"), list.join("\n")).unwrap();
    checked(yavs(&t).args(["analyze", "--work=unisim:l/unisim", "@unisim.rsp"]));
    // The component package first, as the macros use it.
    let mut unimacro = vec![src.join("unimacro/unimacro_VCOMP.vhd")];
    unimacro.extend(glob(&src.join("unimacro")));
    checked(
        yavs(&t)
            .args([
                "analyze",
                "--work=unimacro:l/unimacro",
                "--map=unisim:l/unisim",
            ])
            .args(&unimacro),
    );

    let map = ["--map=unisim:l/unisim", "--map=unimacro:l/unimacro"];
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
