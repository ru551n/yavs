//! The elaboration-time call cache (`docs/decisions.md` "Elaboration-time call cache", #539): a
//! pure function called while elaborating is run once per distinct arguments and closure; a
//! warm run prints what a cold one does (reports included), an edit of the package constant or
//! body is never answered with an old result, and what is not pure is run every time.

mod common;
use common::tempdir;

use std::process::Command;

const PKG: &str = "package ec is\n  constant K : integer := 7;\n  function f (x : integer) return integer;\n\
  impure function g (x : integer) return integer;\nend;\n\
package body ec is\n  function f (x : integer) return integer is begin\n\
    assert x /= 3 report \"f saw three\" severity note;\n    return x * K + 1;\n  end;\n\
  impure function g (x : integer) return integer is begin return x + K; end;\nend;\n";

/// `f(3)` twice (one report each), `f(4)` once.
const TB: &str = "use work.ec.all;\nentity tb is end;\narchitecture a of tb is\n\
  constant c1 : integer := f(3);\n  constant c2 : integer := f(3);\n  constant c3 : integer := f(4);\n\
  signal s1 : bit_vector(1 to c1);\n  signal s2 : bit_vector(1 to c2);\n  signal s3 : bit_vector(1 to c3);\nbegin\n\
  process begin\n    report \"c=\" & integer'image(s1'length) & \" \" & integer'image(s2'length) & \" \" & integer'image(s3'length);\n    wait;\n  end process;\nend;\n";

/// Only an impure function.
const TBG: &str = "use work.ec.all;\nentity tbg is end;\narchitecture a of tbg is\n\
  constant c1 : integer := g(3);\n  signal s1 : bit_vector(1 to c1);\nbegin\n  process begin\n    report \"c=\" & integer'image(s1'length);\n    wait;\n  end process;\nend;\n";

/// A table built by a function, read by a pure function (the shape of a board table in a
/// configuration package): the call runs against the package state, the table is built once.
const TPKG: &str = "package tp is\n  type iv is array (natural range <>) of integer;\n\
  function pick (i : integer) return integer;\nend;\n\
package body tp is\n  function mk (n : integer) return iv is\n    variable r : iv(0 to n - 1);\n  begin\n\
    for i in r'range loop r(i) := i * i + 1; end loop;\n    return r;\n  end;\n  constant T : iv := mk(6);\n\
  function pick (i : integer) return integer is begin return T(i); end;\nend;\n";

const TTB: &str = "use work.tp.all;\nentity ttb is end;\narchitecture a of ttb is\n\
  constant c1 : integer := pick(1);\n  constant c2 : integer := pick(2);\n  constant c3 : integer := pick(5);\n\
  signal s1 : bit_vector(1 to c1);\n  signal s2 : bit_vector(1 to c2);\n  signal s3 : bit_vector(1 to c3);\nbegin\n\
  process begin\n    report \"t=\" & integer'image(s1'length) & \" \" & integer'image(s2'length) & \" \" & integer'image(s3'length);\n    wait;\n  end process;\nend;\n";

struct Env {
    tmp: common::Tmp,
}

struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

impl Out {
    /// Without the debug notes of the caches.
    fn visible(&self) -> (i32, String, String) {
        let err = self
            .stderr
            .lines()
            .filter(|l| !l.starts_with("debug: "))
            .map(|l| format!("{l}\n"))
            .collect();
        (self.code, self.stdout.clone(), err)
    }

    fn noted(&self, what: &str) -> bool {
        self.stderr
            .lines()
            .any(|l| l.starts_with("debug: cache: ") && l.contains(what))
    }
}

impl Env {
    fn new(name: &str) -> Env {
        let e = Env { tmp: tempdir(name) };
        for (f, t) in [
            ("pkg.vhd", PKG),
            ("tb.vhd", TB),
            ("tbg.vhd", TBG),
            ("tpkg.vhd", TPKG),
            ("ttb.vhd", TTB),
        ] {
            std::fs::write(e.tmp.join(f), t).expect("write");
            e.analyze(f);
        }
        e
    }

    fn work(&self) -> String {
        format!("work:{}", self.tmp.join("work").display())
    }

    fn analyze(&self, file: &str) {
        let o = self.yavs(&["analyze", "--work", &self.work(), file]);
        assert_eq!(o.code, 0, "{file}: {}", o.stderr);
    }

    fn yavs(&self, args: &[&str]) -> Out {
        let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
            .current_dir(&self.tmp)
            .env_remove("YAVS_STDLIB")
            .env("YAVS_LOG", "debug")
            .args(["--color", "never"])
            .args(args)
            .output()
            .expect("run");
        Out {
            code: o.status.code().unwrap_or(-1),
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        }
    }

    fn run(&self, top: &str, more: &[&str]) -> Out {
        let work = self.work();
        let mut a = vec!["run", "--work", &work];
        a.extend_from_slice(more);
        a.push(top);
        self.yavs(&a)
    }
}

#[test]
fn warm_run_prints_what_a_cold_one_does() {
    let e = Env::new("ecall-warm");
    let plain = e.run("tb", &["--no-cache"]);
    assert_eq!(plain.code, 0, "{}", plain.stderr);
    let all = format!("{}{}", plain.stdout, plain.stderr);
    assert!(all.contains("c=22 22 29"), "{all}");
    // Two calls with the same arguments, two reports, with or without the cache.
    assert_eq!(all.matches("f saw three").count(), 2, "{all}");
    let first = e.run("tb", &[]);
    let second = e.run("tb", &[]);
    let third = e.run("tb", &[]);
    assert!(!first.noted("ecall"), "{}", first.stderr);
    assert!(second.noted("store ecall"), "{}", second.stderr);
    assert!(third.noted("hit ecall"), "{}", third.stderr);
    assert!(!third.noted("store ecall"), "{}", third.stderr);
    for o in [&first, &second, &third] {
        assert_eq!(o.visible(), plain.visible(), "{}", o.stderr);
    }
}

#[test]
fn editing_the_package_never_gives_an_old_result() {
    let e = Env::new("ecall-edit");
    for _ in 0..3 {
        e.run("tb", &[]);
    }
    assert!(e.run("tb", &[]).noted("hit ecall"));
    // The constant the function reads.
    std::fs::write(
        e.tmp.join("pkg.vhd"),
        PKG.replace("K : integer := 7", "K : integer := 8"),
    )
    .expect("write");
    e.analyze("pkg.vhd");
    let after = e.run("tb", &[]);
    let all = format!("{}{}", after.stdout, after.stderr);
    assert!(all.contains("c=25 25 33"), "{all}");
    assert!(!after.noted("hit ecall"), "{}", after.stderr);
    for _ in 0..2 {
        e.run("tb", &[]);
    }
    let again = e.run("tb", &[]);
    assert!(again.noted("hit ecall"), "{}", again.stderr);
    assert!(
        format!("{}{}", again.stdout, again.stderr).contains("c=25 25 33"),
        "{}",
        again.stderr
    );
    // The body.
    std::fs::write(
        e.tmp.join("pkg.vhd"),
        PKG.replace("K : integer := 7", "K : integer := 8")
            .replace("x * K + 1", "x * K + 2"),
    )
    .expect("write");
    e.analyze("pkg.vhd");
    let body = e.run("tb", &[]);
    let all = format!("{}{}", body.stdout, body.stderr);
    assert!(all.contains("c=26 26 34"), "{all}");
    assert!(!body.noted("hit ecall"), "{}", body.stderr);
}

#[test]
fn an_impure_function_is_run_every_time() {
    let e = Env::new("ecall-impure");
    for _ in 0..4 {
        let o = e.run("tbg", &[]);
        assert_eq!(o.code, 0, "{}", o.stderr);
        assert!(!o.noted("ecall"), "{}", o.stderr);
    }
}

#[test]
fn a_function_reading_a_package_table_is_answered_from_the_package_state() {
    let e = Env::new("ecall-table");
    let plain = e.run("ttb", &["--no-cache"]);
    assert_eq!(plain.code, 0, "{}", plain.stderr);
    assert!(
        format!("{}{}", plain.stdout, plain.stderr).contains("t=2 5 26"),
        "{}",
        plain.stderr
    );
    let first = e.run("ttb", &[]);
    let second = e.run("ttb", &[]);
    let third = e.run("ttb", &[]);
    assert!(second.noted("store ecall"), "{}", second.stderr);
    assert!(third.noted("hit ecall"), "{}", third.stderr);
    for o in [&first, &second, &third] {
        assert_eq!(o.visible(), plain.visible(), "{}", o.stderr);
    }
}
