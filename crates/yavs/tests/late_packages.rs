//! A run-time package loaded during elaboration (an architecture reached through a default-bound
//! component brings it, #873) is initialised after the elaboration-time calls made before it, and
//! every later initialisation of the packages (a rebuild of the session, the build's replay with
//! coverage counters, a warm run from the compilation cache, each tier) keeps that order: the
//! calls return what they returned, so the session never ends in an internal error, and the
//! values are those of the reference simulators.

mod common;
use common::tempdir;

use std::path::Path;
use std::process::Command;

/// Library `idl`: an id counter in package state and a leaf that reports its generic.
const ID: &str = "package idpkg is
  impure function new_id return natural;
  impure function new_id_said (what : string) return natural;
end package;
package body idpkg is
  type counter_t is protected
    impure function next_id return natural;
  end protected;
  type counter_t is protected body
    variable n : natural := 0;
    impure function next_id return natural is
    begin
      n := n + 1;
      return n;
    end function;
  end protected body;
  shared variable cnt : counter_t;
  impure function new_id return natural is
  begin
    return cnt.next_id;
  end function;
  impure function new_id_said (what : string) return natural is
    constant id : natural := cnt.next_id;
  begin
    report what & \" took id \" & integer'image(id);
    return id;
  end function;
end package body;

entity leaf is
  generic (g : natural);
end entity;
architecture a of leaf is
begin
  process
  begin
    report \"leaf g=\" & integer'image(g);
    wait;
  end process;
end architecture;
";

/// Library `other`: a package whose constant takes an id, read through a generic map.
const OTHER: &str = "library idl;
use idl.idpkg.all;
package late_pkg is
  constant c1 : natural := new_id_said(\"late_pkg\");
end package;

library idl;
entity mid is
end entity;
use work.late_pkg.all;
architecture a of mid is
begin
  l : entity idl.leaf generic map (g => c1);
end architecture;
";

const COMP: &str = "package comp_pkg is
  component child is
  end component;
end package;
";

/// Takes two ids, then reaches `other` only through the default binding of `child`.
const TOP: &str = "library idl;
use idl.idpkg.all;
use work.comp_pkg.all;
entity top is
end entity;
architecture a of top is
  constant a0 : natural := new_id;
begin
  i0 : entity idl.leaf generic map (g => new_id);
  i1 : component child;
  process
  begin
    report \"a0=\" & integer'image(a0) & \" last=\" & integer'image(new_id);
    wait;
  end process;
end architecture;
";

/// Analysed after `top`, so `top`'s closure does not hold it.
const CHILD: &str = "library other;
entity child is
end entity;
architecture a of child is
begin
  m : entity other.mid;
end architecture;
";

fn yavs(dir: &Path, args: &[&str]) -> (bool, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .current_dir(dir)
        .env_remove("YAVS_STDLIB")
        .args(["--color", "never"])
        .args(args)
        .output()
        .expect("run yavs");
    (
        o.status.success(),
        format!(
            "{}{}",
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr)
        ),
    )
}

const MAPS: [&str; 3] = ["--map=idl:idl", "--map=other:other", "--map=work:work"];

fn design(name: &str) -> common::Tmp {
    let dir = tempdir(name);
    for (lib, file, text) in [
        ("idl", "id.vhd", ID),
        ("other", "other.vhd", OTHER),
        ("work", "comp.vhd", COMP),
        ("work", "top.vhd", TOP),
        ("work", "child.vhd", CHILD),
    ] {
        std::fs::write(dir.join(file), text).expect("write");
        let work = format!("--work={lib}:{lib}");
        let mut a = vec!["analyze", "--std=2008", &work];
        a.extend(MAPS);
        a.push(file);
        let (ok, out) = yavs(&dir, &a);
        assert!(ok, "{file}: {out}");
    }
    dir
}

fn run(dir: &Path, more: &[&str]) -> String {
    let mut a = vec!["run", "--std=2008", "--work=work:work"];
    a.extend(MAPS);
    a.extend_from_slice(more);
    a.push("top");
    let (ok, out) = yavs(dir, &a);
    assert!(ok, "{more:?}: {out}");
    out
}

/// The reference simulators' values: `top`'s two calls first, then `late_pkg`'s initialiser
/// (its report once), then the process's call.
fn check(out: &str) {
    check_values(out);
    assert_eq!(out.matches("took id").count(), 1, "{out}");
}

/// [`check`] for a run whose build initialises the packages again: their reports are printed
/// again (#874).
fn check_values(out: &str) {
    for want in ["leaf g=2", "leaf g=3", "a0=1 last=4", "late_pkg took id 3"] {
        assert!(out.contains(want), "no `{want}` in\n{out}");
    }
    assert!(!out.contains("internal error"), "{out}");
    assert!(
        !out.contains("took id 1") && !out.contains("took id 2"),
        "{out}"
    );
}

#[test]
fn a_package_loaded_during_elaboration_runs_after_the_calls_before_it() {
    let dir = design("late-pkg");
    check(&run(&dir, &["--no-cache", "--tier=interp"]));
    // Cold, then warm from the compilation cache.
    let cold = run(&dir, &["--tier=interp"]);
    check(&cold);
    let warm = run(&dir, &["--tier=interp"]);
    check(&warm);
}

#[test]
fn the_build_replays_the_calls_in_the_same_order() {
    // Coverage counters: the simulation initialises the packages again and replays the calls.
    let dir = design("late-pkg-cov");
    check_values(&run(&dir, &["--no-cache", "--tier=interp", "--cov=a.ycov"]));
    check_values(&run(&dir, &["--tier=interp", "--cov=b.ycov"]));
    check_values(&run(&dir, &["--tier=interp", "--cov=c.ycov"]));
}

#[cfg(feature = "llvm")]
#[test]
fn llvm_tiers_keep_the_order() {
    let dir = design("late-pkg-llvm");
    for tier in ["--tier=llvm", "--tier=auto"] {
        check(&run(&dir, &[tier]));
        check_values(&run(&dir, &[tier, "--cov=l.ycov"]));
    }
}

#[cfg(feature = "clift")]
#[test]
fn clift_tiers_keep_the_order() {
    let dir = design("late-pkg-clift");
    for tier in ["--tier=clift", "--tier=clift-auto"] {
        check(&run(&dir, &[tier]));
        check_values(&run(&dir, &[tier, "--cov=c.ycov"]));
    }
}
