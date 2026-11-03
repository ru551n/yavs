//! `--tier=clift-auto`: a function the Cranelift native-frame lowering refuses (here an integer
//! power) gets frame-mode code once a compiled caller exists, instead of an empty native-frame
//! compile that repeats and leaves every call from compiled code to the interpreter. Needs the
//! `clift` feature.
//!
//! Two tests. With every function compiled when the interpreter enters it (`--tier-heat=0,0,0`,
//! which waits for each compile) the caller is compiled before the function is called, whatever the
//! speed of the host. With the sampler the function is declined while no caller is compiled and asked
//! for again at a later look; that look comes when it comes, so the test reads the trace as the run
//! writes it and ends the run at the compile, instead of guessing a run length.
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

fn command(dir: &Path, args: &[&str]) -> std::process::Command {
    let mut c = std::process::Command::new(env!("CARGO_BIN_EXE_yavs"));
    c.args(args).env("YAVS_AUTO_TRACE", "1").current_dir(dir);
    // (The count rule has native-frame code of `**`, and no such wait: `auto_count.rs`.)
    c.env("YAVS_AUTO_POLICY", "sampled");
    c
}

fn yavs(dir: &Path, args: &[&str]) -> Output {
    let o = command(dir, args).output().expect("run yavs");
    assert!(o.status.success(), "{o:?}");
    o
}

/// The trace lines that mention `step` and `what`.
fn lines(o: &Output, what: &str) -> Vec<String> {
    step_lines(String::from_utf8_lossy(&o.stderr).lines(), what)
}

fn step_lines<'a>(all: impl Iterator<Item = &'a str>, what: &str) -> Vec<String> {
    all.filter(|l| l.contains("`step`") && l.contains(what))
        .map(str::to_string)
        .collect()
}

/// `natfall` with `n` iterations, in the library of `dir`.
fn run(dir: &Path, tier: &[&str], n: u32) -> Output {
    let n = format!("-gN={n}");
    let mut args = vec!["run", "--std=2008", "--work=work:work", &n];
    args.extend_from_slice(tier);
    args.push("natfall");
    yavs(dir, &args)
}

fn analysed() -> common::Tmp {
    let dir = tempdir("clift_natfall");
    std::fs::write(dir.join("n.vhd"), SRC).expect("write");
    yavs(
        &dir,
        &["analyze", "--std=2008", "--work=work:work", "n.vhd"],
    );
    dir
}

/// The index of the first trace line with `what`.
fn at(err: &str, what: &str) -> Option<usize> {
    err.lines().position(|l| l.contains(what))
}

/// Every function is compiled when the interpreter enters it and the run waits for each compile:
/// the process is compiled before it calls `step`, so `step` is refused its native-frame variant
/// and gets frame-mode code at once, in a run of any speed.
#[test]
fn a_function_without_a_native_variant_gets_frame_mode_code() {
    let dir = analysed();
    let eager = ["--tier=clift-auto", "--tier-heat=0,0,0"];
    let interp = run(&dir, &["--tier=interp"], 20_000);
    let cold = run(&dir, &eager, 20_000);
    assert_eq!(cold.stdout, interp.stdout);
    let err = String::from_utf8_lossy(&cold.stderr);
    assert_eq!(lines(&cold, "no native-frame variant").len(), 1, "{err}");
    let compiled = lines(&cold, " compiled `");
    assert_eq!(compiled.len(), 1, "{err}");
    assert!(compiled[0].contains(" Frame "), "{err}");
    // Its frame-mode code waited for the process that calls it, which is compiled first.
    let (p, f) = (at(&err, " compiled `_p0`"), at(&err, " compiled `step`"));
    assert!(p.is_some() && f.is_some() && p < f, "{err}");
    // The code is stored, so the next run links it, with the refusal found again at no cost.
    let warm = run(&dir, &eager, 20_000);
    assert_eq!(warm.stdout, interp.stdout);
    let err = String::from_utf8_lossy(&warm.stderr);
    let compiled = lines(&warm, " compiled `");
    assert_eq!(compiled.len(), 1, "{err}");
    assert!(
        compiled[0].contains(" hit ") && compiled[0].contains(" Frame "),
        "{err}"
    );
}

/// With the sampler `step` is refused while nothing compiled calls it, and compiled once the
/// sampler has seen it run again after its caller was compiled. That look comes when it comes, so
/// the run is long (about a minute) and the test ends it at the compile; it waits for the event,
/// and fails only if the run ends without it.
#[test]
fn a_declined_function_gets_frame_mode_code_once_its_caller_is_compiled() {
    use std::io::{BufRead, BufReader};
    let dir = analysed();
    let mut child = command(
        &dir,
        &[
            "run",
            "--std=2008",
            "--work=work:work",
            "--no-cache",
            "--tier=clift-auto",
            "-gN=50000000",
            "natfall",
        ],
    )
    .env("YAVS_AUTO_FORCE", "step,_p0")
    .stdout(std::process::Stdio::null())
    .stderr(std::process::Stdio::piped())
    .spawn()
    .expect("run yavs");
    let mut seen = Vec::new();
    for l in BufReader::new(child.stderr.take().expect("stderr")).split(b'\n') {
        let l = String::from_utf8_lossy(&l.expect("trace line")).into_owned();
        let done = l.contains(" compiled `step`");
        seen.push(l);
        if done {
            break;
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    let err = seen.join("\n");
    let compiled = step_lines(seen.iter().map(String::as_str), " compiled `");
    assert_eq!(compiled.len(), 1, "no frame-mode code for `step`:\n{err}");
    assert!(compiled[0].contains(" Frame "), "{err}");
    assert_eq!(
        step_lines(seen.iter().map(String::as_str), "no native-frame variant").len(),
        1,
        "{err}"
    );
    let (p, f) = (at(&err, " compiled `_p0`"), at(&err, " compiled `step`"));
    assert!(p.is_some() && p < f, "{err}");
}
