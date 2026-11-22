//! `--init-unset` and `--init-x`: start values of what VHDL leaves at `'U'`, and values of the
//! metavalue literals a design assigns. The same results on every tier, at four threads, in cycle
//! mode and from a warm cache; the seed of `random` printed; `--cycle-report` names the registers.

mod common;

use std::path::Path;
use std::process::{Command, Output};

use common::tempdir;

fn yavs(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yavs"))
        .current_dir(dir)
        .env("YAVS_STD_CACHE", "off")
        // The checks of cycle mode's own tests: imports read again, every step guarded and every
        // skip run anyway and compared.
        .env("YAVS_CYCLE_IMPORT_CHECK", "1")
        .env("YAVS_CYCLE_GUARD", "all")
        .env("YAVS_CYCLE_SKIP_CHECK", "full")
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

/// The notes of a run (`** Note: ... W=...`), its exit status and standard error.
fn run(dir: &Path, flags: &[&str], top: &str) -> (Vec<String>, i32, String) {
    let mut args = vec!["run", "--work=work:lib"];
    args.extend(flags);
    args.push(top);
    let o = yavs(dir, &args);
    let out = String::from_utf8_lossy(&o.stdout);
    let lines = out
        .lines()
        .filter(|l| l.starts_with("** Note: "))
        .filter_map(|l| l.split_once(": W=").map(|(_, r)| format!("W={r}")))
        .collect();
    (
        lines,
        o.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

/// The modes each value runs in: normal, cycle mode, its exact contract and uncollapsed ports.
const MODES: [&[&str]; 4] = [
    &[],
    &["--cycle"],
    &["--cycle", "--cycle-exact-meta"],
    &["--no-collapse-ports"],
];

/// `to_hstring` of the bits `b` (left first).
fn hex(b: &str) -> String {
    let pad = "0".repeat((4 - b.len() % 4) % 4) + b;
    (0..pad.len() / 4)
        .map(|i| {
            format!(
                "{:X}",
                u8::from_str_radix(&pad[4 * i..4 * i + 4], 2).unwrap()
            )
        })
        .collect()
}

/// The line of width `w`.
fn of(lines: &[String], w: u32) -> &str {
    let p = format!("W={w} ");
    lines.iter().find(|l| l.starts_with(&p)).expect(&p)
}

#[test]
fn init_unset_gives_registers_without_reset_their_start_value() {
    let dir = tempdir("initunset");
    analyze(&dir, "initunset.vhd");
    // Without the option the registers stay 'U': every check fails.
    let (lines, code, _) = run(&dir, &["--no-cache", "--tier=interp"], "initunset");
    assert_eq!(code, 1);
    assert_eq!(of(&lines, 1), "W=1 q=X r=X v=X t=XX n=5 s='U' w='U'");
    let f128 = "F".repeat(32);
    let z128 = "0".repeat(32);
    let mut seen = Vec::new();
    for value in ["0", "1", "random:5"] {
        let opt = format!("--init-unset={value}");
        let mut first: Option<Vec<String>> = None;
        for t in tiers() {
            for mode in MODES {
                let cycle = mode.first() == Some(&"--cycle");
                let mut f = vec!["--no-cache", opt.as_str()];
                f.extend(&t);
                f.extend(mode);
                let (lines, code, err) = run(&dir, &f, "initunset");
                assert_eq!(lines.len(), 5, "{f:?}: {err}");
                // Only a start at 0 passes the checks.
                assert_eq!(code, i32::from(value != "0"), "{f:?}: {err}");
                let note = "signal and port elements without an initial value start at the chosen";
                assert!(
                    err.contains(&format!("`{opt}`: ")) && err.contains(note),
                    "{f:?}: {err}"
                );
                if cycle {
                    assert!(err.contains("cycle mode: 35 of 52"), "{f:?}: {err}");
                    // No 'U' start is read as 0 any more.
                    assert!(!err.contains("read as 0"), "{f:?}: {err}");
                }
                match &first {
                    None => first = Some(lines),
                    Some(l) => assert_eq!(&lines, l, "{f:?} differs"),
                }
            }
        }
        let lines = first.unwrap();
        // The variables after five edges: the scalar, the record's field and the middle word
        // toggled, the other words kept.
        let t = |w: usize, a: char, b: char| {
            let (a, b) = (a.to_string(), b.to_string());
            hex(&(a.clone() + &a.repeat(w) + &b.repeat(w) + &a.repeat(w) + &b.repeat(w)))
        };
        match value {
            "0" => {
                assert_eq!(
                    of(&lines, 1),
                    format!("W=1 q=1 r=1 v=1 t={} n=5 s='1' w='0'", t(1, '1', '0'))
                );
                let w = format!(
                    "W=128 q={}5 r={f128} v={f128} t={} n=5 s='1' w='0'",
                    "0".repeat(31),
                    t(128, '1', '0')
                );
                assert_eq!(of(&lines, 128), w);
            }
            "1" => {
                assert_eq!(
                    of(&lines, 1),
                    format!("W=1 q=0 r=0 v=0 t={} n=5 s='0' w='1'", t(1, '0', '1'))
                );
                let w = format!(
                    "W=128 q={}4 r={z128} v={z128} t={} n=5 s='0' w='1'",
                    "0".repeat(31),
                    t(128, '0', '1')
                );
                assert_eq!(of(&lines, 128), w);
            }
            _ => {
                // A bit of its own per register, instance and element.
                let field =
                    |w: u32, k: usize| of(&lines, w).split(' ').nth(k).unwrap()[2..].to_string();
                let (r, v, t) = (field(128, 2), field(128, 3), field(128, 4));
                assert_ne!(r, v);
                assert_ne!(v[17..], field(65, 3)[1..]);
                for x in [&r, &v, &t] {
                    assert!(
                        x.chars().any(|c| c != '0') && x.chars().any(|c| c != 'F'),
                        "{x}"
                    );
                }
            }
        }
        seen.push(lines);
    }
    // Another seed, other bits.
    let (other, _, _) = run(&dir, &["--no-cache", "--init-unset=random:6"], "initunset");
    assert_ne!(other, seen[2]);
}

#[test]
fn init_unset_random_prints_a_new_seed_that_repeats_the_run() {
    let dir = tempdir("initseed");
    analyze(&dir, "initunset.vhd");
    let (a, _, err) = run(&dir, &["--no-cache", "--init-unset=random"], "initunset");
    let seed = err
        .split("`--init-unset=random:")
        .nth(1)
        .and_then(|r| r.split('`').next())
        .expect(&err)
        .to_string();
    let (b, _, _) = run(
        &dir,
        &["--no-cache", &format!("--init-unset=random:{seed}")],
        "initunset",
    );
    assert_eq!(a, b);
    for bad in ["rand", "2", "random:x", "random:-1"] {
        let o = yavs(
            &dir,
            &[
                "run",
                "--work=work:lib",
                &format!("--init-unset={bad}"),
                "initunset",
            ],
        );
        let err = String::from_utf8_lossy(&o.stderr);
        assert_eq!(o.status.code(), Some(2), "{err}");
        assert!(err.contains("is not a"), "{err}");
    }
    let o = yavs(
        &dir,
        &["run", "--work=work:lib", "--init-x=rando", "initunset"],
    );
    assert!(
        String::from_utf8_lossy(&o.stderr).contains("a value with a similar name exists: `random`"),
        "{}",
        String::from_utf8_lossy(&o.stderr)
    );
}

#[test]
fn init_unset_warm_runs_follow_the_option() {
    let dir = tempdir("initwarm");
    analyze(&dir, "initunset.vhd");
    for cycle in [&[][..], &["--cycle"][..]] {
        // Each value after runs with others and without one, so each reads what one stored; the
        // run without the option must find no code that calls the option's natives.
        let values = [
            "", "", "0", "", "1", "0", "1", "random:3", "", "random:4", "random:3", "",
        ];
        for value in values {
            let opt = format!("--init-unset={value}");
            let mut f: Vec<&str> = cycle.to_vec();
            if !value.is_empty() {
                f.push(&opt);
            }
            let (lines, code, err) = run(&dir, &f, "initunset");
            let (cold, cold_code, _) = run(&dir, &[&f[..], &["--no-cache"]].concat(), "initunset");
            assert_eq!((&lines, code), (&cold, cold_code), "{f:?}: {err}");
            if value.is_empty() {
                // 'U' in the normal mode, 0 in cycle mode, which passes.
                assert_eq!(code, i32::from(cycle.is_empty()), "{f:?}: {err}");
            }
        }
    }
}

#[test]
fn cycle_report_lists_the_registers_init_unset_sets() {
    let dir = tempdir("initreport");
    analyze(&dir, "initunset.vhd");
    let o = yavs(
        &dir,
        &[
            "run",
            "--work=work:lib",
            "--no-cache",
            "--cycle-report",
            "initunset",
        ],
    );
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(o.status.success(), "{err}");
    let head = "25 register signals and 20 clocked variables start at 'U'";
    assert!(err.contains(head), "{err}");
    // The rows of the section: the lines after its head that start with four spaces.
    let part: String = (err.split(head).nth(1).unwrap().lines().skip(1))
        .take_while(|l| l.starts_with("    "))
        .map(|l| format!("{l}\n"))
        .collect();
    for i in 0..5 {
        for sig in ["dut.cnt", "dut.rec.a", "dut.bit1", "v", "t"] {
            assert!(
                part.contains(&format!("    initunset.g({i}).{sig}\n")),
                "{sig} of {i}: {err}"
            );
        }
        for v in ["acc", "mem", "rv", "sc"] {
            let l = format!("    initunset.g({i}).dut.process (initunset.vhd:80): variable {v}\n");
            assert!(part.contains(&l), "{v} of {i}: {err}");
        }
    }
    // An integer field, a signal with an initial value and one of processes in the event kernel
    // are not listed.
    assert!(
        !part.contains("rec.n") && !part.contains(".keep") && !part.contains(".w2"),
        "{err}"
    );
}

/// The lines of a run of `initx` without what differs between the modes by contract: `m`, read
/// from a `'-'` mask the region reads as 0.
fn no_mask(lines: &[String]) -> Vec<String> {
    lines
        .iter()
        .map(|l| l.split(" m=").next().unwrap().to_string() + &l[l.find(" tb=").unwrap()..])
        .collect()
}

/// The line of width `w` at `sel`.
fn at(lines: &[String], w: u32, sel: &str) -> String {
    let p = format!("W={w} sel={sel} ");
    lines.iter().find(|l| l.starts_with(&p)).expect(&p).clone()
}

#[test]
fn init_x_gives_metavalue_literals_a_value() {
    let dir = tempdir("initx");
    analyze(&dir, "initx.vhd");
    let (plain, code, err) = run(&dir, &["--no-cache", "--tier=interp"], "initx");
    assert_eq!((code, plain.len()), (0, 15), "{err}");
    // sel = "10": the default branch's 'X', the last else's '-', the 'U'; then the 'W' register.
    assert_eq!(
        at(&plain, 1, "10"),
        "W=1 sel=10 y=X q=X b='-' u='U' m=true tb='X'"
    );
    assert_eq!(
        at(&plain, 1, "01"),
        "W=1 sel=01 y=0 q=X b='-' u='1' m=true tb='X'"
    );
    let (f, z) = ("F".repeat(32), "0".repeat(32));
    for value in ["0", "1", "random:9"] {
        let opt = format!("--init-x={value}");
        let mut first: Option<Vec<String>> = None;
        for t in tiers() {
            for cycle in [false, true] {
                let mut fl = vec!["--no-cache", opt.as_str()];
                fl.extend(&t);
                if cycle {
                    fl.push("--cycle");
                }
                let (lines, code, err) = run(&dir, &fl, "initx");
                assert_eq!((code, lines.len()), (0, 15), "{fl:?}: {err}");
                if cycle {
                    assert!(err.contains("cycle mode: 35 of 42"), "{fl:?}: {err}");
                }
                assert!(
                    err.contains(&format!("`{opt}`: metavalue literals")),
                    "{err}"
                );
                if !cycle {
                    // The mask outside a default branch keeps its '-'.
                    assert!(
                        lines.iter().all(|l| l.contains(" m=true ")),
                        "{fl:?}: {lines:?}"
                    );
                }
                // The testbench's own literal stays.
                assert!(
                    lines.iter().all(|l| l.ends_with("tb='X'")),
                    "{fl:?}: {lines:?}"
                );
                let lines = no_mask(&lines);
                match &first {
                    None => first = Some(lines),
                    Some(l) => assert_eq!(&lines, l, "{fl:?} differs"),
                }
            }
        }
        let lines = first.unwrap();
        match value {
            "0" => {
                assert_eq!(at(&lines, 1, "10"), "W=1 sel=10 y=0 q=0 b='0' u='0' tb='X'");
                assert_eq!(
                    at(&lines, 128, "01"),
                    format!("W=128 sel=01 y={z} q={z} b='0' u='1' tb='X'")
                );
            }
            "1" => {
                assert_eq!(at(&lines, 1, "10"), "W=1 sel=10 y=1 q=1 b='1' u='1' tb='X'");
                assert_eq!(
                    at(&lines, 128, "01"),
                    format!("W=128 sel=01 y={z} q={f} b='1' u='1' tb='X'")
                );
            }
            _ => {
                // Per bit: the 128-element 'X' and 'W' literals are neither all 0 nor all 1, and differ.
                let l = at(&lines, 128, "10");
                let y = &l.split(' ').nth(2).unwrap()[2..];
                let q = &l.split(' ').nth(3).unwrap()[2..];
                assert_ne!(y, q);
                for x in [y, q] {
                    assert!(
                        x.chars().any(|c| c != '0') && x.chars().any(|c| c != 'F'),
                        "{l}"
                    );
                }
            }
        }
    }
}

#[test]
fn a_two_state_region_compares_with_init_x_bits() {
    let dir = tempdir("metacmp");
    analyze(&dir, "metacmp.vhd");
    // The normal mode: a two-state value never equals a metavalue, '-' matches anything.
    for g in ["-gONES=false", "-gONES=true"] {
        let (lines, code, err) = run(&dir, &["--no-cache", g], "metacmp");
        assert_eq!((code, lines.len()), (0, 5), "{err}");
        assert!(
            lines.iter().all(|l| l.ends_with(" hit='0' c='0' m='1'")),
            "{lines:?}"
        );
    }
    for t in tiers() {
        for (x, g, hit) in [
            (None, "-gONES=false", "'1'"),
            (Some("--init-x=0"), "-gONES=false", "'1'"),
            (Some("--init-x=1"), "-gONES=true", "'1'"),
            (Some("--init-x=1"), "-gONES=false", "'0'"),
        ] {
            let mut f = vec!["--no-cache", "--cycle", g];
            f.extend(&t);
            f.extend(x);
            let (lines, code, err) = run(&dir, &f, "metacmp");
            assert_eq!((code, lines.len()), (0, 5), "{f:?}: {err}");
            // Every process of the design joins; the literals read as `--init-x` gives them.
            assert!(err.contains("cycle mode: 11 of 18"), "{f:?}: {err}");
            for l in &lines {
                assert!(
                    l.ends_with(&format!(" hit={hit} c={hit} m='1'")),
                    "{f:?}: {lines:?}"
                );
            }
        }
    }
    // The report counts the literals; the exact contract keeps the process out.
    let report = |flags: &[&str]| {
        let mut a = vec!["run", "--work=work:lib", "--no-cache"];
        a.extend(flags);
        a.extend(["--cycle-report", "metacmp"]);
        String::from_utf8_lossy(&yavs(&dir, &a).stderr).into_owned()
    };
    let r = report(&["--init-x=1"]);
    assert!(r.contains("literals with a metavalue read as 1"), "{r}");
    assert!(report(&[]).contains("literals with a metavalue read as 0"));
    let r = report(&["--cycle-exact-meta"]);
    assert!(r.contains("a comparison with a metavalue literal"), "{r}");
}

#[test]
fn a_testbench_checker_keeps_its_metavalue_compares() {
    let dir = tempdir("topmeta");
    analyze(&dir, "topmeta.vhd");
    for t in tiers() {
        for x in [None, Some("--init-x=0"), Some("--init-x=1")] {
            let mut f = vec!["--no-cache", "--cycle"];
            f.extend(&t);
            f.extend(x);
            let mut a = vec!["run", "--work=work:lib"];
            a.extend(&f);
            a.push("topmeta");
            let o = yavs(&dir, &a);
            let (out, err) = (
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr),
            );
            // The checkers stay in the event kernel and never fail; the design joins.
            assert!(
                o.status.success() && !out.contains("Error"),
                "{f:?}: {out}{err}"
            );
            assert!(err.contains("cycle mode: 15 of 22"), "{f:?}: {err}");
        }
    }
}

/// The warnings of a run of `initxassert`, as `TIME W=.. what` (the delta left out), and stderr.
fn warnings(dir: &Path, flags: &[&str]) -> (Vec<String>, String) {
    let mut a = vec!["run", "--work=work:lib", "--no-cache"];
    a.extend(flags);
    a.push("initxassert");
    let o = yavs(dir, &a);
    let (out, err) = (
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    );
    assert!(o.status.success(), "{flags:?}: {out}{err}");
    let lines = (out.lines())
        .filter_map(|l| l.strip_prefix("** Warning: "))
        .map(|l| {
            let (time, rest) = l.split_once('+').expect("time+delta");
            format!("{time} {}", rest.split_once(": ").expect("message").1)
        })
        .collect();
    (lines, err)
}

/// A62 with step 2: an assert below the top that compares with a metavalue literal is evaluated
/// in a two-state region with the literal's `--init-x` bits, so `d /= (d'range => 'X')` fails
/// where `d` is 0 (1 with `--init-x=1`); the normal mode and `--cycle-exact-meta` never fail, and
/// `--no-asserts` drops them with any `--init-x`.
#[test]
fn design_asserts_compare_with_the_init_x_bits_in_a_two_state_region() {
    let dir = tempdir("initxassert");
    analyze(&dir, "initxassert.vhd");
    // d is all 0 at the edges at 5 and 25 ns, all 1 at 15 and 35 ns.
    let at = |times: [&str; 2]| -> Vec<String> {
        let mut v = Vec::new();
        for t in times {
            for w in [1, 63, 64, 65, 128] {
                v.push(format!("{t}fs W={w} d is X"));
                v.push(format!("{t}fs W={w} d(0) is U"));
            }
        }
        v.sort();
        v
    };
    let sorted = |mut v: Vec<String>| {
        v.sort();
        v
    };
    for x in [
        &[][..],
        &["--init-x=0"],
        &["--init-x=1"],
        &["--no-asserts", "--init-x=1"],
    ] {
        let (lines, err) = warnings(&dir, x);
        assert!(lines.is_empty(), "{x:?}: {lines:?}\n{err}");
    }
    for t in tiers() {
        let cases: [(&[&str], Vec<String>); 7] = [
            (&[], at(["5000000", "25000000"])),
            (&["--init-x=0"], at(["5000000", "25000000"])),
            (&["--init-x=1"], at(["15000000", "35000000"])),
            (&["--no-asserts"], vec![]),
            (&["--no-asserts", "--init-x=1"], vec![]),
            (&["--cycle-exact-meta"], vec![]),
            (&["--cycle-exact-meta", "--init-x=1"], vec![]),
        ];
        for (x, want) in cases {
            let mut f = vec!["--cycle"];
            f.extend(&t);
            f.extend(x);
            let (lines, err) = warnings(&dir, &f);
            assert_eq!(sorted(lines), want, "{f:?}: {err}");
            if !x.contains(&"--cycle-exact-meta") {
                // The checking process of each instance joins the region.
                assert!(err.contains("cycle mode: 5 of"), "{f:?}: {err}");
            }
            if x.contains(&"--no-asserts") {
                assert!(err.contains("`--no-asserts`: 10 assert"), "{f:?}: {err}");
            }
            if x.contains(&"--init-x=1") {
                assert!(err.contains("`--init-x=1`"), "{f:?}: {err}");
            }
        }
        // random: the one-element literal has one bit in every instance, so `d(0) /= 'U'` fails
        // at one pair of edges per width; a wide random `'X'` aggregate is neither all 0 nor all 1.
        let mut f = vec!["--cycle", "--init-x=random:7"];
        f.extend(&t);
        let (lines, err) = warnings(&dir, &f);
        for w in [1, 63, 64, 65, 128] {
            let n = |what: &str| {
                let s = format!(" W={w} {what}");
                lines.iter().filter(|l| l.ends_with(&s)).count()
            };
            assert_eq!(n("d(0) is U"), 2, "{f:?}: {lines:?}\n{err}");
            assert_eq!(n("d is X"), if w == 1 { 2 } else { 0 }, "{f:?}: {lines:?}");
        }
    }
}
