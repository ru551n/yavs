//! Every clock-edge idiom on `std_logic`, `std_ulogic`, `bit` and `boolean` against an oracle
//! written out in VHDL (`edge/idioms.vhd`, #632): the forms that are an edge test
//! (`rising_edge`, `x'event and x = '1'` either way round, `not x'stable and ...`, `wait until`
//! forms, an edge inside a larger condition) and the look-alikes that must not be one (a level,
//! a bare `'event`, another signal, a user `rising_edge`). The design checks itself, the stimulus
//! takes the clock through every pair of `std_ulogic` values, so `rising_edge` (old value '0' or
//! 'L') and `'event` (any old value) differ. Run on every tier, with the edge triggers off
//! (coverage), and with several threads.

mod common;
use common::tempdir;

use std::path::Path;
use std::process::{Command, Output};

const SRC: &str = include_str!("edge/idioms.vhd");

fn yavs(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run yavs")
}

fn check(dir: &Path, flags: &[&str]) {
    let mut args = vec!["run", "--work=work:work"];
    args.extend_from_slice(flags);
    args.push("idioms");
    let o = yavs(dir, &args);
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(o.status.success(), "{flags:?}: {o:?}");
    // One line per type; a form that disagrees with its oracle is a failure before it.
    let agree = out
        .lines()
        .filter(|l| l.contains(" forms on a signal and on an element agree"));
    assert_eq!(agree.count(), 4, "{flags:?}: {out}");
}

#[test]
fn edge_idioms_match_the_oracle() {
    let dir = tempdir("edge_idioms");
    std::fs::write(dir.join("idioms.vhd"), SRC).expect("write");
    let a = yavs(
        &dir,
        &["analyze", "--std=2008", "--work=work:work", "idioms.vhd"],
    );
    assert!(a.status.success(), "{a:?}");
    check(&dir, &["--tier=interp"]);
    check(&dir, &["--tier=interp", "--threads=4"]);
    check(&dir, &["--tier=interp", "--cov=cov.ycov"]);
    #[cfg(feature = "llvm")]
    for tier in [
        vec!["--tier=llvm"],
        vec!["--tier=auto", "--tier-heat=1,0,0"],
        vec!["--tier=auto", "--tier-heat=1,0,0", "--threads=4"],
    ] {
        check(&dir, &tier);
    }
}
