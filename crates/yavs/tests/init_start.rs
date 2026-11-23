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

/// The tiers of this build, with one thread, with four and with the processes shuffled.
fn tiers() -> Vec<Vec<&'static str>> {
    let mut t = vec![
        vec!["--tier=interp"],
        vec!["--tier=interp", "--threads", "4"],
        vec!["--tier=interp", "--shuffle=3"],
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

/// The modes each value runs in: normal, two-state cycle mode, cycle mode with metavalues and
/// uncollapsed ports.
const MODES: [&[&str]; 4] = [
    &[],
    &["--cycle", "--two-state"],
    &["--cycle"],
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
                // The bits of seed 5, the same on every system and checkout (A68): a change here
                // changes what a user's printed seed repeats.
                assert_eq!(
                    of(&lines, 65),
                    "W=65 q=0F916298850FB9293 r=16814943BD4318F75 v=17F804028886F113A \
                     t=0434E280CFB894D910FE43A2608BD7169C86DC4C746396F80AFE32D99F916DDC9F n=5 \
                     s='1' w='1'"
                );
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
    for cycle in [&[][..], &["--cycle", "--two-state"][..]] {
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
            "--two-state",
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
                    fl.extend(["--cycle", "--two-state"]);
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
                // The bits of seed 9 on every system and checkout (A68).
                assert_eq!(
                    at(&lines, 65, "10"),
                    "W=65 sel=10 y=14537CE6211D03CFF q=0C2083BC9D4EC5121 b='0' u='1' tb='X'"
                );
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

/// A74: in both modes a two-state value never equals a metavalue literal it is compared with
/// (`= (others => 'X')`, a `case` choice "0X"), whatever `--init-x` gives assigned literals, and
/// `'-'` in `std_match` matches anything; the processes still join the region.
#[test]
fn a_two_state_value_never_equals_a_metavalue_literal() {
    let dir = tempdir("metacmp");
    analyze(&dir, "metacmp.vhd");
    for g in ["-gONES=false", "-gONES=true"] {
        let (lines, code, err) = run(&dir, &["--no-cache", g], "metacmp");
        assert_eq!((code, lines.len()), (0, 5), "{err}");
        assert!(
            lines.iter().all(|l| l.ends_with(" hit='0' c='0' m='1'")),
            "{lines:?}"
        );
    }
    for t in tiers() {
        for mode in [&[][..], &["--cycle"][..]] {
            for (x, g) in [
                (None, "-gONES=false"),
                (Some("--init-x=0"), "-gONES=false"),
                (Some("--init-x=1"), "-gONES=true"),
                (Some("--init-x=1"), "-gONES=false"),
                (Some("--init-x=random:7"), "-gONES=true"),
            ] {
                let mut f = vec!["--no-cache", "--two-state", g];
                f.extend(mode);
                f.extend(&t);
                f.extend(x);
                let (lines, code, err) = run(&dir, &f, "metacmp");
                assert_eq!((code, lines.len()), (0, 5), "{f:?}: {err}");
                if !mode.is_empty() {
                    // Every process of the design joins.
                    assert!(err.contains("cycle mode: 11 of 18"), "{f:?}: {err}");
                }
                assert!(
                    lines.iter().all(|l| l.ends_with(" hit='0' c='0' m='1'")),
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
    let r = report(&["--two-state", "--init-x=1"]);
    assert!(r.contains("literals with a metavalue read as 1"), "{r}");
    assert!(report(&["--two-state"]).contains("literals with a metavalue read as 0"));
    let r = report(&[]);
    assert!(r.contains("a comparison with a metavalue literal"), "{r}");
}

#[test]
fn a_testbench_checker_keeps_its_metavalue_compares() {
    let dir = tempdir("topmeta");
    analyze(&dir, "topmeta.vhd");
    for t in tiers() {
        for x in [None, Some("--init-x=0"), Some("--init-x=1")] {
            let mut f = vec!["--no-cache", "--cycle", "--two-state"];
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

/// A74 (amends A62): an assert below the top that compares with a metavalue literal never sees a
/// two-state value equal to it, so `d /= (d'range => 'X')` and `d(0) /= 'U'` hold with every
/// `--init-x`, in the normal mode and in a two-state region (whose checking processes still join);
/// `--no-asserts` drops them.
#[test]
fn design_asserts_never_see_a_two_state_value_equal_a_metavalue() {
    let dir = tempdir("initxassert");
    analyze(&dir, "initxassert.vhd");
    for t in tiers() {
        for mode in [&[][..], &["--cycle"][..]] {
            for x in [
                &[][..],
                &["--init-x=0"],
                &["--init-x=1"],
                &["--init-x=random:7"],
                &["--two-state"],
                &["--two-state", "--init-x=0"],
                &["--two-state", "--init-x=1"],
                &["--two-state", "--init-x=random:7"],
                &["--two-state", "--no-asserts", "--init-x=1"],
            ] {
                let mut f = mode.to_vec();
                f.extend(&t);
                f.extend(x);
                let (lines, err) = warnings(&dir, &f);
                assert!(lines.is_empty(), "{f:?}: {lines:?}\n{err}");
                if !mode.is_empty() && x.contains(&"--two-state") {
                    // The checking process of each instance joins the region.
                    assert!(err.contains("cycle mode: 5 of"), "{f:?}: {err}");
                }
                if x.contains(&"--no-asserts") {
                    assert!(err.contains("`--no-asserts`: 10 assert"), "{f:?}: {err}");
                }
            }
        }
    }
}

/// A run with extra environment variables: its notes as [`run`] gives them, and stderr.
fn run_env(dir: &Path, env: &[(&str, &str)], flags: &[&str], top: &str) -> (Vec<String>, String) {
    let mut c = Command::new(env!("CARGO_BIN_EXE_yavs"));
    c.current_dir(dir)
        .env("YAVS_STD_CACHE", "off")
        .envs(env.iter().copied())
        .args(["--color", "never", "run", "--work=work:lib"])
        .args(flags)
        .arg(top);
    let o = c.output().expect("run yavs");
    let lines = String::from_utf8_lossy(&o.stdout)
        .lines()
        .filter(|l| l.starts_with("** Note: "))
        .filter_map(|l| l.split_once(": W=").map(|(_, r)| format!("W={r}")))
        .collect();
    (lines, String::from_utf8_lossy(&o.stderr).into_owned())
}

fn strs(f: &[String]) -> Vec<&str> {
    f.iter().map(String::as_str).collect()
}

/// The files of a directory tree, by path and size.
fn files(dir: &Path) -> std::collections::BTreeSet<(std::path::PathBuf, u64)> {
    let mut out = std::collections::BTreeSet::new();
    let mut todo = vec![dir.to_path_buf()];
    while let Some(d) = todo.pop() {
        for e in std::fs::read_dir(&d).unwrap() {
            let p = e.unwrap().path();
            if p.is_dir() {
                todo.push(p);
            } else {
                out.insert((p.clone(), std::fs::metadata(&p).unwrap().len()));
            }
        }
    }
    out
}

/// A68: the seed is a run-time input. A second seed reuses every cache entry the first one made
/// (nothing compiled again, the cycle analysis and region from the cache) and gives the values of
/// a cold run with that seed.
#[test]
fn a_second_seed_reuses_the_compiled_code() {
    let dir = tempdir("initreuse");
    analyze(&dir, "initx.vhd");
    for mode in [&["--tier=interp"][..], &["--cycle"][..]] {
        let seed = |s: u32| {
            let mut f: Vec<String> = mode.iter().map(|m| m.to_string()).collect();
            f.push(format!("--init-x=random:{s}"));
            f.push(format!("--init-unset=random:{s}"));
            f
        };
        // Twice: the cache stores an entry the second time it is asked for.
        run_env(&dir, &[], &strs(&seed(1)), "initx");
        let (first, _) = run_env(&dir, &[], &strs(&seed(1)), "initx");
        let before = files(&dir.join("lib"));
        let stats = [("YAVS_CYCLE_STATS", "1")];
        let (second, err) = run_env(&dir, &stats, &strs(&seed(2)), "initx");
        assert_eq!(
            files(&dir.join("lib")),
            before,
            "{mode:?}: a new seed wrote to the cache"
        );
        if mode == ["--cycle"] {
            for hit in ["analysis from the cache", "region from the cache"] {
                assert!(err.contains(&format!("cycle stats: {hit}")), "{hit}: {err}");
            }
        }
        let (cold, _) = run_env(
            &dir,
            &[],
            &[&strs(&seed(2))[..], &["--no-cache"]].concat(),
            "initx",
        );
        assert_eq!(second, cold, "{mode:?}");
        assert_eq!(second.len(), 15, "{mode:?}: {err}");
        assert_ne!(first, second, "{mode:?}: the seed changed nothing");
    }
}

/// A68: the bits depend on a source site by file name, line and column, so a checkout elsewhere,
/// in a directory of another depth and with CRLF line ends, gives the same values.
#[test]
fn random_bits_do_not_depend_on_the_checkout() {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/cycle");
    let mut seen: Option<Vec<String>> = None;
    for (sub, crlf) in [("a", false), ("b/c/d", true)] {
        let dir = tempdir("initcheckout");
        let at = dir.join(sub);
        std::fs::create_dir_all(&at).unwrap();
        for name in ["initunset.vhd", "initx.vhd"] {
            let text = std::fs::read_to_string(src.join(name))
                .unwrap()
                .replace("\r\n", "\n");
            let text = if crlf {
                text.replace('\n', "\r\n")
            } else {
                text
            };
            std::fs::write(at.join(name), text).unwrap();
            let file = format!("{sub}/{name}");
            let o = yavs(&dir, &["analyze", "--std=2008", "--work=work:lib", &file]);
            assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        }
        let mut lines = Vec::new();
        for (top, mode) in [
            ("initunset", &[][..]),
            ("initx", &[][..]),
            ("initx", &["--cycle"][..]),
        ] {
            let mut f = vec!["--no-cache", "--init-unset=random:7", "--init-x=random:7"];
            f.extend(mode);
            let (l, _, err) = run(&dir, &f, top);
            assert!(!l.is_empty(), "{err}");
            lines.extend(if top == "initx" { no_mask(&l) } else { l });
        }
        match &seen {
            None => seen = Some(lines),
            Some(s) => assert_eq!(&lines, s, "{sub} with CRLF line ends"),
        }
    }
}

/// A68: the drawn start of a copied clock decides whether it has an edge at time 0. Cycle mode
/// reaches the normal mode's count for each seed, also where its cache holds the analysis of a
/// seed whose start gave the other count.
#[test]
fn a_drawn_clock_start_keeps_the_cached_analysis_right() {
    let dir = tempdir("initclk");
    analyze(&dir, "initclk.vhd");
    // One seed of each start: a copy that starts at '0' counts the edge at time 0.
    let mut by: std::collections::BTreeMap<String, u32> = Default::default();
    for s in 1..64 {
        let opt = format!("--init-unset=random:{s}");
        let (l, code, err) = run(&dir, &["--no-cache", "--tier=interp", &opt], "initclk");
        assert_eq!((code, l.len()), (0, 1), "{err}");
        by.entry(l[0].clone()).or_insert(s);
        if by.len() == 2 {
            break;
        }
    }
    assert_eq!(
        by.keys().cloned().collect::<Vec<_>>(),
        ["W=1 n=3 m=3", "W=1 n=4 m=3"],
        "both starts among the seeds"
    );
    let seeds: Vec<(String, u32)> = by.into_iter().collect();
    for order in [[0, 1], [1, 0]] {
        let dir = tempdir("initclkw");
        analyze(&dir, "initclk.vhd");
        for &k in &order {
            let (want, s) = &seeds[k];
            let opt = format!("--init-unset=random:{s}");
            let (l, code, err) = run(&dir, &["--cycle", &opt], "initclk");
            assert_eq!(
                (code, l.join("")),
                (0, want.clone()),
                "seed {s} after {order:?}: {err}"
            );
        }
    }
}

/// A metavalue aggregate inside an expression of a two-state region takes a bit of its own per
/// element under `--init-x=random`, not one bit spread over the vector; `1` gives `not d`.
#[test]
fn a_random_aggregate_in_an_expression_gives_each_element_a_bit() {
    let dir = tempdir("splatxor");
    analyze(&dir, "splatxor.vhd");
    let mut first: Option<Vec<String>> = None;
    for t in tiers() {
        let mut f = vec!["--no-cache", "--cycle", "--two-state", "--init-x=random:7"];
        f.extend(&t);
        let (lines, code, err) = run(&dir, &f, "splatxor");
        assert_eq!((code, lines.len()), (0, 5), "{f:?}: {err}");
        match &first {
            None => first = Some(lines),
            Some(l) => assert_eq!(&lines, l, "{f:?} differs"),
        }
    }
    let lines = first.unwrap();
    let q = of(&lines, 128).split("q=").nth(1).unwrap().to_string();
    assert!(
        q.chars().any(|c| c != '0') && q.chars().any(|c| c != 'F'),
        "{q}"
    );
    let (ones, _, err) = run(
        &dir,
        &["--no-cache", "--cycle", "--two-state", "--init-x=1"],
        "splatxor",
    );
    assert_eq!(
        of(&ones, 128),
        format!("W=128 q={}", "F".repeat(32)),
        "{err}"
    );
}
