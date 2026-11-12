//! `LCopy` of a known length and offsets (`yavs-clift` `Gen::lcopy_const`): every length and offset
//! within a 16-element vector, slices around block edges, wider runs, the same variable as source
//! and destination, and a count and offsets known at run time only (`Gen::lcopy_inline`, the
//! runtime's copy over 64 elements), null runs included.
//! Every result is checked against a model written here, and
//! the interpreter and the compiled tiers must print it. Needs the `clift` feature.
#![cfg(feature = "clift")]

mod common;
use common::tempdir;

use std::fmt::Write;
use std::process::Command;

const CODES: &[u8] = b"UX01ZWLH-";

/// A vector of `n` elements, all nine values on every plane, different per `seed`.
fn pattern(n: usize, seed: usize) -> Vec<u8> {
    (0..n)
        .map(|i| CODES[(i * i + i * (seed * 2 + 1) + seed) % CODES.len()])
        .collect()
}

fn lit(v: &[u8]) -> String {
    format!("\"{}\"", String::from_utf8_lossy(v))
}

/// The run `s .. s + k` of `src` over `d .. d + k` of `dst`, read before it is written.
fn model(dst: &[u8], src: &[u8], d: usize, s: usize, k: usize) -> Vec<u8> {
    let mut out = dst.to_vec();
    out[d..d + k].copy_from_slice(&src[s..s + k]);
    out
}

/// One process per `CHUNK` shapes (a process is compiled whole: small ones keep the debug build's
/// compile short), one after another in time, and the lines they must report: the two copies (a
/// second variable, then the variable itself) as text.
const CHUNK: usize = 40;

fn processes(
    width: usize,
    shapes: &[(usize, usize, usize)],
    dynamic: Dyn,
    at: &mut usize,
    src: &mut String,
    want: &mut Vec<String>,
) {
    let (a, b) = (pattern(width, 1), pattern(width, 2));
    let (av, bv) = (lit(&a), lit(&b));
    for chunk in shapes.chunks(CHUNK) {
        *at += 1;
        writeln!(
            src,
            "  process
    variable a, b : std_ulogic_vector(0 to {});
    variable i, j : natural;
  begin
    wait for {at} ns;
    i := zero; j := zero2;",
            width - 1
        )
        .expect("to a string");
        for &(k, s, d) in chunk {
            // With `+ i` and `+ j` (0 at run time) the offsets, and with both the count, are not
            // known when the code is made (`- 1 + j` keeps a null run's right bound a natural).
            let (p, q) = match dynamic {
                Dyn::No => ("", ""),
                Dyn::Offsets => (" + i", " + i"),
                Dyn::Count => (" + i", " + j"),
            };
            let (se, de) = (s + k, d + k);
            if matches!(dynamic, Dyn::Count) {
                // In a function, which runs on a native frame in compiled code.
                writeln!(
                    src,
                    "    a := {av}; b := {bv};
    b := put(b, a, {d}{p}, {de} - 1{q}, {s}{p}, {se} - 1{q});
    a := put(a, a, {d}{p}, {de} - 1{q}, {s}{p}, {se} - 1{q});
    report to_string(b) & \" \" & to_string(a);"
                )
            } else {
                writeln!(
                    src,
                    "    a := {av}; b := {bv};
    b({d}{p} to {de} - 1{q}) := a({s}{p} to {se} - 1{q});
    a({d}{p} to {de} - 1{q}) := a({s}{p} to {se} - 1{q});
    report to_string(b) & \" \" & to_string(a);"
                )
            }
            .expect("to a string");
            let (x, y) = (model(&b, &a, d, s, k), model(&a, &a, d, s, k));
            want.push(format!(
                "{} {}",
                String::from_utf8_lossy(&x),
                String::from_utf8_lossy(&y)
            ));
        }
        src.push_str("    wait;\n  end process;\n");
    }
}

/// What of a copy is known only at run time.
#[derive(Clone, Copy)]
enum Dyn {
    No,
    Offsets,
    Count,
}

fn design() -> (String, Vec<String>) {
    // Every length and offset of 16 elements.
    let mut small = Vec::new();
    for k in 1..=16 {
        for s in 0..=16 - k {
            for d in 0..=16 - k {
                small.push((k, s, d));
            }
        }
    }
    // Around the edges of the first blocks (offsets count from the right): one and two blocks on
    // each side.
    let mut edges = Vec::new();
    for k in [1, 2, 3, 31, 32, 33, 62, 63, 64] {
        for s in [0, 1, 31, 62, 63, 64, 65, 66, 100] {
            for d in [0, 1, 32, 62, 63, 64, 66, 127] {
                if s + k <= 192 && d + k <= 192 {
                    edges.push((k, s, d));
                }
            }
        }
    }
    // Over 64 elements (the runtime's copy), up to where compiled code stops copying itself.
    let mut wide = Vec::new();
    for k in [65, 127, 128, 129, 200, 256, 448, 512, 513, 600] {
        for (s, d) in [(0, 0), (0, 64), (64, 0), (1, 1), (1, 70), (63, 64), (90, 3)] {
            if s + k <= 700 && d + k <= 700 {
                wide.push((k, s, d));
            }
        }
    }
    // A known count at offsets known only at run time (#1063): one and two blocks each side.
    let mut moving = Vec::new();
    for k in [1, 2, 3, 8, 31, 32, 33, 62, 63, 64] {
        for s in [0, 1, 31, 62, 63, 64, 65, 100] {
            for d in [0, 1, 32, 63, 64, 66, 127] {
                if s + k <= 192 && d + k <= 192 {
                    moving.push((k, s, d));
                }
            }
        }
    }
    // A count known at run time only (a slice of a parameter's or a generic's bounds), in a
    // function: null runs, one and two blocks each side, and over 64 elements, where the runtime
    // copies.
    let mut counted = Vec::new();
    for k in [0, 1, 2, 31, 32, 33, 63, 64, 65, 100] {
        for s in [0, 1, 62, 63, 64, 65, 90] {
            for d in [0, 1, 63, 64, 66, 91] {
                if s + k <= 192 && d + k <= 192 {
                    counted.push((k, s, d));
                }
            }
        }
    }
    let (mut body, mut want, mut at) = (String::new(), Vec::new(), 0);
    processes(16, &small, Dyn::No, &mut at, &mut body, &mut want);
    processes(192, &edges, Dyn::No, &mut at, &mut body, &mut want);
    processes(700, &wide, Dyn::No, &mut at, &mut body, &mut want);
    processes(192, &moving, Dyn::Offsets, &mut at, &mut body, &mut want);
    processes(192, &counted, Dyn::Count, &mut at, &mut body, &mut want);
    let src = format!(
        "library ieee; use ieee.std_logic_1164.all;
entity clift_lcopy_shapes is end;
architecture t of clift_lcopy_shapes is
  signal zero, zero2 : natural := 0;
  function part(a : std_ulogic_vector; s, se : integer) return std_ulogic_vector is
  begin
    return a(s to se);
  end;
  function put(b, a : std_ulogic_vector; d, de, s, se : integer) return std_ulogic_vector is
    variable r : std_ulogic_vector(b'range) := b;
  begin
    r(d to de) := part(a, s, se);
    return r;
  end;
begin
{body}end;
"
    );
    (src, want)
}

/// The reported texts of a run, in order.
fn reports(out: &str) -> Vec<String> {
    out.lines()
        .filter_map(|l| {
            // The message has no colon of its own.
            let (_, m) = l.contains("Note").then(|| l.rsplit_once(": "))??;
            Some(m.trim().to_string())
        })
        .collect()
}

#[test]
fn constant_shape_copies_match_the_model_on_every_tier() {
    let d = tempdir("clift_lcopy_shapes");
    let (src, want) = design();
    std::fs::write(d.join("s.vhd"), src).expect("write");
    let work = format!("work:{}", d.join("lib").display());
    let yavs = env!("CARGO_BIN_EXE_yavs");
    let a = Command::new(yavs)
        .current_dir(&d)
        .args(["analyze", "--std=2008", "--work", &work, "s.vhd"])
        .output()
        .expect("analyze");
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    for (tier, env) in [
        (&["--tier=interp"][..], None),
        (&["--tier=clift"][..], Some("all")),
        (
            &[
                "--tier=auto",
                "--tier-heat=1,0,0",
                "--tier-max-size=1000000",
            ][..],
            None,
        ),
    ] {
        let mut c = Command::new(yavs);
        c.current_dir(&d)
            .args(["--color", "never", "run", "--std=2008", "--no-cache"])
            .args(["--work", &work])
            .args(tier)
            .arg("clift_lcopy_shapes");
        if let Some(e) = env {
            c.env("YAVS_CLIFT_CLIF", e).env("YAVS_COUNTERS", "1");
        }
        let o = c.output().expect("run");
        let (out, err) = (
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr),
        );
        assert!(
            o.status.success(),
            "{tier:?}: {}",
            &err[err.len().saturating_sub(800)..]
        );
        // No site shifts in 128 bits: not the constant shapes, not those of run-time offsets.
        if env.is_some() {
            assert!(
                err.contains("clif of frame code"),
                "no Cranelift IR printed"
            );
            assert!(!err.contains("i128"), "a site took the general lowering");
            // The copies of a run-time count in `part` are in place (`Gen::lcopy_inline`).
            assert!(
                !err.contains("clift_lcopy_count_sites=0\n")
                    && err.contains("clift_lcopy_count_sites="),
                "no run-time count copied in place"
            );
        }
        let got = reports(&(out.to_string() + &err));
        assert_eq!(got.len(), want.len(), "{tier:?} lines: {:?}", got.first());
        for (i, (g, w)) in got.iter().zip(&want).enumerate() {
            assert_eq!(g, w, "{tier:?}, shape number {i}");
        }
    }
}
