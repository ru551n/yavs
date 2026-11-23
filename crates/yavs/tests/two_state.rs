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
    let cs = if x == b'0' { "111" } else { "100" };
    format!(
        "W={w} q={} nq={} z={} lit={} flags={flags} rout=01100 cnt=0001 more=0011 cs={cs} zz={} d={}",
        hexv(w, b'1', b'0', b'1'),
        hexv(w, b'0', b'1', b'0'),
        hexv(w, b'0', b'0', b'0'),
        hexv(w, x, x, x),
        'Z',
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
                // The design's clocked process and its concurrent statements join the region.
                if mode == "--cycle" {
                    assert!(err.contains("cycle mode: 15 of 30 processes"), "{err}");
                }
            }
        }
    }
}

/// A random `--init-x`: the bits come from the seed at run time, the same in every tier and in
/// cycle mode; another seed gives other bits.
#[test]
fn a_random_init_x_gives_the_same_bits_in_both_modes() {
    let dir = tempdir("twostate-random");
    analyze(&dir, "twostate.vhd");
    let mut first: Option<Vec<String>> = None;
    for t in tiers() {
        for mode in [None, Some("--cycle")] {
            let mut f = vec!["--two-state", "--init-x=random:7"];
            f.extend(mode);
            f.extend(&t);
            let (lines, err) = notes(&dir, &f, "twostate");
            match &first {
                None => first = Some(lines),
                Some(l) => assert_eq!(&lines, l, "{f:?}: {err}"),
            }
        }
    }
    let (other, _) = notes(&dir, &["--two-state", "--init-x=random:8"], "twostate");
    assert_ne!(Some(other), first);
}

/// A71: a `'Z'` driven onto a resolved signal below the top stays `'Z'`: an event net whose
/// idle value a package drives from components below the top (it used to hang), and a tri-state
/// bus two components drive in turn (it used to read `0`s and conflicts). The same output
/// without the option, on every tier, with either `--init-x` and under `--cycle --two-state`.
/// A pull-up `'H'` is a strong `'1'` there (A65): pulling the line low gives `'X'`.
#[test]
fn a_z_driven_onto_a_resolved_signal_stays_z() {
    let dir = tempdir("twostate-bus");
    analyze(&dir, "twostate_bus.vhd");
    let want = |two: bool| {
        let mut w = vec![
            "events=6 ev=ZZ",
            "turn=0 bus=ZZZZZZZZ",
            "turn=1 bus=10100101",
            "turn=2 bus=00111100",
        ];
        w.extend(if two {
            ["pull=1", "low=X"]
        } else {
            ["pull=H", "low=0"]
        });
        w
    };
    let mut runs = vec![vec!["--tier=interp"]];
    for t in tiers() {
        for mode in [None, Some("--cycle")] {
            for x in ["--init-x=0", "--init-x=1"] {
                let mut f = vec!["--two-state", x];
                f.extend(mode);
                f.extend(t.iter().copied());
                runs.push(f);
            }
        }
    }
    for f in runs {
        let mut args = vec!["run", "--work=work:lib", "--no-cache", "--stop-time=1us"];
        args.extend(&f);
        args.push("twostate_bus");
        let o = yavs(&dir, &args);
        let out = String::from_utf8_lossy(&o.stdout);
        assert!(
            o.status.success(),
            "{f:?}: {}",
            String::from_utf8_lossy(&o.stderr)
        );
        let got: Vec<&str> = out
            .lines()
            .filter_map(|l| l.rsplit_once(": ").map(|x| x.1))
            .filter(|l| {
                ["events=", "turn=", "pull=", "low="]
                    .iter()
                    .any(|p| l.starts_with(p))
            })
            .collect();
        assert_eq!(got, want(f[0] == "--two-state"), "{f:?}: {out}");
    }
}
