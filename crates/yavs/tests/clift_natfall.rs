//! `--tier=clift-auto`: a function the Cranelift native-frame lowering refuses (here an integer
//! power) gets frame-mode code once a compiled caller exists, instead of an empty native-frame
//! compile that repeats and leaves every call from compiled code to the interpreter. Needs the
//! `clift` feature.
#![cfg(feature = "clift")]

mod common;
use common::tempdir;

use std::path::Path;
use std::process::Output;

const SRC: &str = "entity natfall is generic (N : natural := 400000); end;
architecture a of natfall is
  function pw(x, n : integer) return integer is
    variable r : integer := 0;
  begin
    for k in 1 to 6 loop
      r := (r + (x + k) ** n) mod 65537;
      if r mod 5 = 0 then r := r + k; end if;
    end loop;
    return r;
  end;
  function step(x : integer) return integer is
    variable t : integer := 0;
  begin
    for k in 1 to 4 loop
      t := t + pw(x mod 50 + k, 3) mod 1013;
    end loop;
    return t + x mod 7;
  end;
begin
  process
    variable x : integer := 1;
    variable acc : integer := 0;
  begin
    for i in 1 to N loop
      x := step(x + i);
      acc := (acc + x) mod 1000003;
    end loop;
    report integer'image(acc);
    wait;
  end process;
end;
";

fn yavs(dir: &Path, args: &[&str]) -> Output {
    let o = std::process::Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(args)
        .env("YAVS_AUTO_TRACE", "1")
        .env("YAVS_AUTO_FORCE", "step,_p0")
        .current_dir(dir)
        .output()
        .expect("run yavs");
    assert!(o.status.success(), "{o:?}");
    o
}

/// The trace lines that mention `step` and `what`.
fn lines(o: &Output, what: &str) -> Vec<String> {
    String::from_utf8_lossy(&o.stderr)
        .lines()
        .filter(|l| l.contains("`step`") && l.contains(what))
        .map(str::to_string)
        .collect()
}

#[test]
fn a_function_without_a_native_variant_gets_frame_mode_code() {
    let dir = tempdir("clift_natfall");
    std::fs::write(dir.join("n.vhd"), SRC).expect("write");
    yavs(
        &dir,
        &["analyze", "--std=2008", "--work=work:work", "n.vhd"],
    );
    let run = || {
        yavs(
            &dir,
            &[
                "run",
                "--std=2008",
                "--work=work:work",
                "--tier=clift-auto",
                "natfall",
            ],
        )
    };
    let ti = std::time::Instant::now();
    let interp = yavs(
        &dir,
        &[
            "run",
            "--std=2008",
            "--work=work:work",
            "--tier=interp",
            "natfall",
        ],
    );
    let interp_took = ti.elapsed();
    let t0 = std::time::Instant::now();
    let cold = run();
    let took = t0.elapsed();
    if std::env::var_os("PATH").is_some() {
        panic!(
            "DIAG interp took {interp_took:?} cold took {took:?}\n{}",
            String::from_utf8_lossy(&cold.stderr)
        );
    }
    assert_eq!(cold.stdout, interp.stdout);
    assert_eq!(lines(&cold, "no native-frame variant").len(), 1, "{cold:?}");
    let compiled = lines(&cold, " compiled `");
    assert_eq!(compiled.len(), 1, "{compiled:?}");
    assert!(compiled[0].contains(" Frame "), "{compiled:?}");
    // Its frame-mode code waited for the process that calls it, which is compiled first.
    let err = String::from_utf8_lossy(&cold.stderr);
    let at = |what: &str| err.lines().position(|l| l.contains(what));
    let (p, f) = (at(" compiled `_p0`"), at(" compiled `step`"));
    assert!(p.is_some() && f.is_some() && p < f, "{err}");
    // The code is stored, so the next run links it, with the refusal found again at no cost.
    let warm = run();
    assert_eq!(warm.stdout, interp.stdout);
    let compiled = lines(&warm, " compiled `");
    assert_eq!(compiled.len(), 1, "{compiled:?}");
    assert!(
        compiled[0].contains(" hit ") && compiled[0].contains(" Frame "),
        "{compiled:?}"
    );
}
