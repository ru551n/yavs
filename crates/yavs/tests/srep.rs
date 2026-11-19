//! The elaboration-time session replayed (`docs/decisions.md` "The elaboration-time session is
//! replayed"): impure constructors called in constant declarations leave package state; a run
//! whose calls are all stored takes their results and that state, and prints what a run that
//! makes the calls prints. Another generic, an impure input, or the switches make it run them.

mod common;
use common::tempdir;

use std::process::Command;

/// A counter in a protected object; `new_id` bumps it, `count` reads it (at run time).
const PKG: &str = "package idp is\n  impure function new_id(name : string; step : integer) return integer;\n\
  impure function count return integer;\n  impure function say(x : integer) return integer;\nend;\n\
package body idp is\n  type counter_t is protected\n    impure function bump(step : integer) return integer;\n  end protected;\n\
  type counter_t is protected body\n    variable n : integer := 0;\n\
    impure function bump(step : integer) return integer is begin n := n + step; return n; end;\n  end protected body;\n\
  shared variable cnt : counter_t;\n\
  impure function new_id(name : string; step : integer) return integer is\n  begin\n    return cnt.bump(step) * 100 + name'length;\n  end;\n\
  impure function count return integer is begin return cnt.bump(0); end;\n\
  impure function say(x : integer) return integer is\n    use std.textio.all;\n    variable l : line;\n  begin\n\
    write(l, string'(\"say \") & integer'image(x));\n    writeline(output, l);\n    return cnt.bump(x);\n  end;\nend;\n";

const TB: &str = "use work.idp.all;\nentity tb is\n  generic (g : integer := 1);\nend;\narchitecture a of tb is\n\
  constant a1 : integer := new_id(\"alpha\", 1);\n  constant a2 : integer := new_id(\"be\", g);\n\
  constant a3 : integer := new_id(\"gamma\", 3);\n  signal s : bit_vector(1 to a3 mod 100);\nbegin\n\
  process begin\n    report \"a=\" & integer'image(a1) & \" \" & integer'image(a2) & \" \" & integer'image(a3)\n\
      & \" len=\" & integer'image(s'length) & \" count=\" & integer'image(count);\n    wait;\n  end process;\nend;\n";

/// A call that writes to the standard output: never stored.
const TBW: &str = "use work.idp.all;\nentity tbw is end;\narchitecture a of tbw is\n\
  constant a1 : integer := new_id(\"alpha\", 1);\n  constant a2 : integer := say(4);\nbegin\n\
  process begin\n    report \"w=\" & integer'image(a1) & \" \" & integer'image(a2) & \" count=\" & integer'image(count);\n\
    wait;\n  end process;\nend;\n";

struct Env {
    tmp: common::Tmp,
}

struct Out {
    code: i32,
    text: String,
    counters: String,
}

impl Out {
    fn counter(&self, name: &str) -> u64 {
        let key = format!("{name}=");
        self.counters
            .lines()
            .find_map(|l| l.split_whitespace().find_map(|w| w.strip_prefix(&key)))
            .and_then(|v| v.parse().ok())
            .unwrap_or_else(|| panic!("no counter {name}: {}", self.counters))
    }
}

impl Env {
    fn new(name: &str) -> Env {
        let e = Env { tmp: tempdir(name) };
        for (f, t) in [("pkg.vhd", PKG), ("tb.vhd", TB), ("tbw.vhd", TBW)] {
            std::fs::write(e.tmp.join(f), t).expect("write");
            let o = e.yavs(&["analyze", "--std=2008", "--work", &e.work(), f], &[]);
            assert_eq!(o.code, 0, "{f}: {}", o.text);
        }
        e
    }

    fn work(&self) -> String {
        format!("work:{}", self.tmp.join("work").display())
    }

    fn yavs(&self, args: &[&str], env: &[(&str, &str)]) -> Out {
        let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
            .current_dir(&self.tmp)
            .env_remove("YAVS_STDLIB")
            .env_remove("YAVS_SREP")
            .env_remove("YAVS_SREP_CHECK")
            .env("YAVS_COUNTERS", "1")
            .envs(env.iter().copied())
            .args(["--color", "never"])
            .args(args)
            .output()
            .expect("run");
        let err = String::from_utf8_lossy(&o.stderr);
        let (counters, rest): (Vec<&str>, Vec<&str>) =
            err.lines().partition(|l| l.contains("counters:"));
        Out {
            code: o.status.code().unwrap_or(-1),
            text: format!("{}{}", String::from_utf8_lossy(&o.stdout), rest.join("\n")),
            counters: counters.join("\n"),
        }
    }

    fn run(&self, top: &str, more: &[&str], env: &[(&str, &str)]) -> Out {
        let work = self.work();
        let mut a = vec!["run", "--tier=interp", "--work", &work];
        a.extend_from_slice(more);
        a.push(top);
        self.yavs(&a, env)
    }
}

#[test]
fn a_replayed_session_leaves_the_state_the_calls_leave() {
    let e = Env::new("srep-same");
    let plain = e.run("tb", &["--no-cache"], &[]);
    assert_eq!(plain.code, 0, "{}", plain.text);
    assert!(
        plain.text.contains("a=105 202 505 len=5 count=5"),
        "{}",
        plain.text
    );
    let runs: Vec<Out> = (0..5).map(|_| e.run("tb", &[], &[])).collect();
    for o in &runs {
        assert_eq!((o.code, &o.text), (plain.code, &plain.text));
    }
    // The second run stores, the later ones take every result and the state after them.
    assert_eq!(runs[0].counter("srep_steps"), 0);
    assert_eq!(runs[1].counter("srep_stored"), 1);
    let last = &runs[4];
    assert_eq!(last.counter("srep_steps"), 3, "{}", last.counters);
    assert_eq!(last.counter("srep_restored"), 1, "{}", last.counters);
    assert_eq!(last.counter("srep_missed"), 0, "{}", last.counters);
    // The check runs the calls and compares them with what is stored.
    let check = e.run("tb", &[], &[("YAVS_SREP_CHECK", "1")]);
    assert_eq!((check.code, &check.text), (plain.code, &plain.text));
    assert_eq!(check.counter("srep_steps"), 0);
    let off = e.run("tb", &[], &[("YAVS_SREP", "0")]);
    assert_eq!((off.code, &off.text), (plain.code, &plain.text));
    assert_eq!(off.counter("srep_restored"), 0);
}

#[test]
fn another_generic_runs_the_calls_after_the_stored_steps() {
    let e = Env::new("srep-generic");
    for _ in 0..4 {
        e.run("tb", &[], &[]);
    }
    let plain = e.run("tb", &["--no-cache", "-gg=5"], &[]);
    assert!(
        plain.text.contains("a=105 602 905 len=5 count=9"),
        "{}",
        plain.text
    );
    // The first step is stored, the second is not: the first call runs after all.
    let first = e.run("tb", &["-gg=5"], &[]);
    assert_eq!((first.code, &first.text), (plain.code, &plain.text));
    assert_eq!(first.counter("srep_steps"), 1, "{}", first.counters);
    assert_eq!(first.counter("srep_missed"), 1, "{}", first.counters);
    let mut last = first;
    for _ in 0..3 {
        last = e.run("tb", &["-gg=5"], &[]);
        assert_eq!((last.code, &last.text), (plain.code, &plain.text));
    }
    assert_eq!(last.counter("srep_restored"), 1, "{}", last.counters);
    // The first generic is still served.
    let back = e.run("tb", &[], &[]);
    assert!(
        back.text.contains("a=105 202 505 len=5 count=5"),
        "{}",
        back.text
    );
    assert_eq!(back.counter("srep_restored"), 1, "{}", back.counters);
}

#[test]
fn a_call_that_writes_is_run_every_time() {
    let e = Env::new("srep-write");
    let plain = e.run("tbw", &["--no-cache"], &[]);
    assert!(plain.text.contains("say 4"), "{}", plain.text);
    assert!(plain.text.contains("w=105 5 count=5"), "{}", plain.text);
    for _ in 0..4 {
        let o = e.run("tbw", &[], &[]);
        assert_eq!((o.code, &o.text), (plain.code, &plain.text));
        assert_eq!(o.counter("srep_stored"), 0, "{}", o.counters);
        assert_eq!(o.counter("srep_steps"), 0, "{}", o.counters);
    }
}
