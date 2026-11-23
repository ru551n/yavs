//! `--two-state` in the event kernel: the design below the top-level unit holds only `0` and `1`
//! (what it reads from the testbench mapped, its literals and start values given), the testbench
//! keeps the nine values. The same results on every tier, at four threads, from a warm cache and
//! under `--cycle --two-state`.

mod common;

use std::path::Path;
use std::process::{Command, Output};

use common::tempdir;

fn yavs(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yavs"))
        .current_dir(dir)
        .env("YAVS_STD_CACHE", "off")
        // Every logic operand of two-state code checked to be 0 or 1.
        .env("YAVS_TWO_CHECK", "1")
        .args(["--color", "never"])
        .args(args)
        .output()
        .expect("run yavs")
}

fn analyze(dir: &Path, name: &str) {
    let src = Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/cycle")
        .join(name);
    let o = yavs(
        dir,
        &[
            "analyze",
            "--std=2008",
            "--work=work:lib",
            src.to_str().unwrap(),
        ],
    );
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
}

/// The tiers of this build, with one thread and with four.
fn tiers() -> Vec<Vec<&'static str>> {
    let mut t = vec![
        vec!["--tier=interp"],
        vec!["--tier=interp", "--threads", "4"],
    ];
    if cfg!(feature = "clift") {
        t.push(vec!["--tier=auto"]);
        t.push(vec!["--tier=clift"]);
    }
    t
}

/// The `W=...` notes of a run, sorted by width, and its standard error.
fn notes(dir: &Path, flags: &[&str], top: &str) -> (Vec<String>, String) {
    let mut args = vec!["run", "--work=work:lib"];
    args.extend(flags);
    args.push(top);
    let o = yavs(dir, &args);
    let err = String::from_utf8_lossy(&o.stderr).into_owned();
    assert!(o.status.success(), "{flags:?}: {err}");
    let mut lines: Vec<String> = String::from_utf8_lossy(&o.stdout)
        .lines()
        .filter_map(|l| l.split_once(": W=").map(|(_, r)| format!("W={r}")))
        .collect();
    lines.sort_by_key(|l| l[2..].split(' ').next().unwrap().parse::<u32>().unwrap());
    (lines, err)
}

/// `to_hstring` of a vector of `w` elements whose leftmost element is `hi` and rightmost `lo`,
/// the rest `mid`.
fn hexv(w: u32, hi: u8, mid: u8, lo: u8) -> String {
    let bits: String = (0..w)
        .map(|i| match i {
            0 => hi,
            _ if i == w - 1 => lo,
            _ => mid,
        } as char)
        .collect();
    let bits = if w == 1 {
        (lo as char).to_string()
    } else {
        bits
    };
    let pad = "0".repeat((4 - bits.len() % 4) % 4) + &bits;
    (0..pad.len() / 4)
        .map(|i| {
            format!(
                "{:X}",
                u8::from_str_radix(&pad[4 * i..4 * i + 4], 2).unwrap()
            )
        })
        .collect()
}

/// The note of width `w` under `--two-state` with `--init-x` bits `x` (`b'0'` or `b'1'`).
fn want(w: u32, x: u8) -> String {
    let flags = if x == b'0' { "00110100" } else { "10100100" };
    format!(
        "W={w} q={} nq={} z={} lit={} flags={flags} rout=01100 cnt=0001 more=0011 d={}",
        hexv(w, b'1', b'0', b'1'),
        hexv(w, b'0', b'1', b'0'),
        hexv(w, b'0', b'0', b'0'),
        hexv(w, x, x, x),
        if w == 1 { "H" } else { "1" },
    )
}

#[test]
fn two_state_maps_what_the_design_below_the_top_reads() {
    let dir = tempdir("twostate");
    analyze(&dir, "twostate.vhd");
    const WS: [u32; 5] = [1, 63, 64, 65, 128];
    // Without the option the metavalues of the testbench reach the design.
    let (lines, _) = notes(&dir, &["--no-cache", "--tier=interp"], "twostate");
    assert_ne!(lines[2], want(64, b'0'), "{lines:?}");
    for (x, extra) in [(b'0', None), (b'1', Some("--init-x=1"))] {
        let want: Vec<String> = WS.iter().map(|&w| want(w, x)).collect();
        for t in tiers() {
            // Cold, then twice from the cache (the second run reads what the first stored),
            // and cycle mode, whose region has the same values.
            for mode in ["--no-cache", "", "", "--cycle"] {
                let mut f = vec!["--two-state"];
                if !mode.is_empty() {
                    f.push(mode);
                }
                f.extend(extra);
                f.extend(&t);
                let (lines, err) = notes(&dir, &f, "twostate");
                assert_eq!(lines, want, "{f:?}: {err}");
            }
        }
    }
}
