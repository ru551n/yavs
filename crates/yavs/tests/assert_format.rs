//! `SetVhdlAssertFormat` / `GetVhdlAssertFormat` (std.env, VHDL-2019; #922): the format is stored per
//! severity, an invalid one is `Valid => false`, and reports of that severity use it, the same on
//! every tier.

mod common;
use common::tempdir;

use std::path::Path;
use std::process::{Command, Output};

const SRC: &str = r#"use std.env.all;
entity t is end;
architecture a of t is
begin
  process
    variable valid : boolean;
  begin
    SetVhdlAssertFormat(failure, "Wacky Failure");
    assert GetVhdlAssertFormat(failure) = "Wacky Failure" severity failure;
    assert GetVhdlAssertFormat(note) = "" severity failure;
    SetVhdlAssertFormat(warning, "Wackier Warning", valid);
    assert valid severity failure;
    SetVhdlAssertFormat(warning, "{Q}", valid);
    assert not valid severity failure;
    assert GetVhdlAssertFormat(warning) = "Wackier Warning" severity failure;
    SetVhdlAssertFormat(error, "{t} foo {s} bar {r:#<10} {r:#>10} {r:#^10}", valid);
    assert valid severity failure;
    report "message" severity error;
    wait for 5 ps;
    SetVhdlAssertFormat(error, "** {S}: {r: <10} at {t:.ns} : {i}", valid);
    report "message" severity error;
    SetVhdlAssertFormat(error, "{t:.foobar}", valid);
    assert not valid severity failure;
    SetVhdlAssertFormat(error, "{s:.ns}", valid);
    assert not valid severity failure;
    SetVhdlAssertFormat(error, "{t", valid);
    assert not valid severity failure;
    SetVhdlAssertFormat(warning, "");
    report "default" severity warning;
    SetVhdlAssertFormat(warning, "{s:.sec}");
    report "not reached" severity note;
    wait;
  end process;
end;
"#;

fn yavs(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run yavs")
}

fn run(dir: &Path, tier: &str, threads: &str) -> (Option<i32>, String, String) {
    let o = yavs(
        dir,
        &[
            "run",
            "--std=2019",
            "--work=work:work",
            &format!("--tier={tier}"),
            &format!("--threads={threads}"),
            "t",
        ],
    );
    (
        o.status.code(),
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

#[test]
#[allow(clippy::single_element_loop)]
fn formats_are_stored_and_applied() {
    let dir = tempdir("assert_format");
    std::fs::write(dir.join("t.vhd"), SRC).expect("write");
    let a = yavs(
        &dir,
        &["analyze", "--std=2019", "--work=work:work", "t.vhd"],
    );
    assert!(a.status.success(), "{a:?}");
    let want = run(&dir, "interp", "1");
    for line in [
        "0 fs foo error bar message### ###message #message##",
        "** ERROR: message    at 0.005 ns : :t",
        "5000fs+0: default",
    ] {
        assert!(want.1.contains(line), "{line}: {want:?}");
    }
    // The invalid format without Valid ends the run with an error.
    assert!(want.2.contains("invalid assert format"), "{want:?}");
    assert_eq!(want.0, Some(1), "{want:?}");
    for (tier, threads) in [
        ("interp", "4"),
        #[cfg(any(feature = "llvm", feature = "clift"))]
        ("auto", "1"),
        #[cfg(feature = "clift")]
        ("clift", "1"),
        #[cfg(all(feature = "llvm", feature = "clift"))]
        ("auto3", "4"),
    ] {
        assert_eq!(run(&dir, tier, threads), want, "{tier} {threads}");
    }
}
