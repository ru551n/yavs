//! The stages shared by closures that start with the same library files (`docs/decisions.md`
//! "Library stages", #407): the lowered packages and their subprograms, the state of the packages
//! after initialisation and the optimised copies of library functions, kept next to the library
//! of the last file of the prefix. Every run must print exactly what a run without the cache
//! prints, in whatever order the testbenches come, at any number of threads.

mod common;
use common::tempdir;

use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Debug, PartialEq, Eq)]
struct Out {
    code: i32,
    stdout: String,
    stderr: String,
}

/// The result without the cache's own debug notes.
fn visible(o: &Out) -> Out {
    Out {
        code: o.code,
        stdout: o.stdout.clone(),
        stderr: o
            .stderr
            .lines()
            .filter(|l| !l.starts_with("debug: cache:") && !l.starts_with("debug: units:"))
            .map(|l| format!("{l}\n"))
            .collect(),
    }
}

fn note(o: &Out, what: &str) -> bool {
    o.stderr
        .lines()
        .any(|l| l.starts_with("debug: cache: ") && l.contains(what))
}

/// How many different closures must ask for a prefix's library before it is made
/// (`yavs-elab` `libstage::SIGHTINGS`).
const SIGHTINGS: u32 = 4;

/// A library `liba` of ten packages (so that every closure starts with ten files of it), one of
/// which keeps state (a signal, a protected counter) and one of which has a deferred constant,
/// and a work library of testbenches.
struct Libs {
    tmp: common::Tmp,
}

const COUNTER: &str = "package pa4 is\n  constant k4 : integer := 4;\n  function f4 (x : integer) return integer;\n  \
impure function next_id return integer;\n  function lsum (x : integer) return integer;\n  signal sg : integer := 3;\nend;\n\
package body pa4 is\n  type counter_t is protected\n    impure function bump return integer;\n  end protected;\n  \
type counter_t is protected body\n    variable n : integer := 10;\n    \
impure function bump return integer is begin n := n + 1; return n; end;\n  end protected body;\n  \
shared variable ctr : counter_t;\n  function f4 (x : integer) return integer is begin return x + k4; end;\n  \
impure function next_id return integer is begin return ctr.bump; end;\n  \
function lsum (x : integer) return integer is\n    variable a : integer := 0;\n  begin\n    for i in 1 to x loop\n      if i mod 2 = 0 then a := a + i; elsif i mod 3 = 0 then a := a - 1; else a := a + 2; end if;\n      \
if a > 1000000 then a := 0; end if;\n      if i mod 5 = 0 then a := a + 7; elsif i mod 7 = 0 then a := a + 3; end if;\n      \
if i mod 11 = 0 then a := a + 1; elsif i mod 13 = 0 then a := a + 2; end if;\n    end loop;\n    return a;\n  end;\nend;\n";

impl Drop for Libs {
    fn drop(&mut self) {
        // The libraries are tens of megabytes of cache by the end of a test.
        let _ = std::fs::remove_dir_all(&self.tmp);
    }
}

impl Libs {
    fn spec(&self, l: &str) -> String {
        format!("{l}:{}", self.tmp.join(l).display())
    }

    fn yavs(&self, args: &[String]) -> Out {
        self.yavs_env(args, &[])
    }

    fn yavs_env(&self, args: &[String], env: &[(&str, &str)]) -> Out {
        let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
            .current_dir(&self.tmp)
            .env_remove("YAVS_STDLIB")
            .envs(env.iter().copied())
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

    fn analyze(&self, lib: &str, map: &[&str], file: &Path) {
        let mut a = vec!["analyze".to_string(), format!("--work={}", self.spec(lib))];
        a.extend(map.iter().map(|m| format!("--map={}", self.spec(m))));
        a.push(file.display().to_string());
        let o = self.yavs(&a);
        assert_eq!(o.code, 0, "{a:?}: {o:?}");
    }

    fn write(&self, name: &str, text: &str) -> PathBuf {
        let f = self.tmp.join(name);
        std::fs::write(&f, text).expect("write");
        f
    }

    /// `liba`: pa0 to pa9, a chain; pa4 holds state. With `late_body`, pa7 declares a deferred
    /// constant `kb`, whose value the body gives, in a file of its own that needs a unit of
    /// another library (so it cannot follow the declaration in the order of the closure).
    fn new(name: &str, late_body: bool) -> Libs {
        let l = Libs { tmp: tempdir(name) };
        for i in 0..10 {
            let uses = if i == 0 {
                String::new()
            } else {
                format!("use work.pa{}.all;\n", i - 1)
            };
            let prev = if i == 0 {
                "0".to_string()
            } else {
                format!("k{}", i - 1)
            };
            let late = i == 7 && late_body;
            let decl = if i == 4 {
                COUNTER
                    .replace(
                        "constant k4 : integer := 4;",
                        &format!("constant k4 : integer := {prev} + 4;"),
                    )
                    .replace("package pa4 is", &format!("{uses}package pa4 is"))
            } else {
                let kb = if late {
                    "  constant kb : integer;\n"
                } else {
                    ""
                };
                let body = if late {
                    String::new()
                } else {
                    format!(
                        "package body pa{i} is\n  function f{i} (x : integer) return integer is begin return x + k{i}; end;\nend;\n"
                    )
                };
                format!(
                    "{uses}package pa{i} is\n  constant k{i} : integer := {prev} + {i};\n  function f{i} (x : integer) return integer;\n{kb}end;\n{body}"
                )
            };
            let f = l.write(&format!("pa{i}.vhd"), &decl);
            if late {
                let vals = l.write(
                    "vals.vhd",
                    "package vals is\n  constant lib_value : integer := 1;\nend;\n",
                );
                l.analyze("libb", &[], &vals);
                l.analyze("liba", &["libb"], &f);
                let bf = l.write(
                    "pa7_body.vhd",
                    "library libb; use libb.vals.all;\nuse work.pa6.all;\npackage body pa7 is\n  \
                     function f7 (x : integer) return integer is begin return x + k7; end;\n  \
                     constant kb : integer := 100 + lib_value;\nend;\n",
                );
                l.analyze("liba", &["libb"], &bf);
            } else {
                l.analyze("liba", &[], &f);
            }
        }
        l
    }

    /// A testbench `tb<n>`; with `session` it calls `next_id` while it is elaborated.
    fn tb(&self, n: u32, session: bool) -> String {
        let name = format!("tb{n}");
        let (decl, id) = if session {
            ("constant id1 : integer := next_id;\n", "id1")
        } else {
            ("", "0")
        };
        let late = if self.tmp.join("pa7_body.vhd").exists() {
            " & \" kb=\" & integer'image(kb)"
        } else {
            ""
        };
        let text = format!(
            "library liba; use liba.pa9.all; use liba.pa4.all;\n{}entity {name} is end;\narchitecture a of {name} is\n  {decl}begin\n  process\n    variable v : integer := {n};\n  begin\n    \
             report \"k=\" & integer'image(k9 + {n}) & \" f=\" & integer'image(f9({n}) + f4({n})) & \" id=\" & integer'image({id}) & \" next=\" & integer'image(next_id) & \" sg=\" & integer'image(sg) & \" ls=\" & integer'image(lsum(v)){late};\n    \
             wait;\n  end process;\nend;\n",
            if late.is_empty() {
                ""
            } else {
                "use liba.pa7.all;\n"
            }
        );
        let f = self.write(&format!("{name}.vhd"), &text);
        let mut maps = vec!["liba"];
        if self.tmp.join("vals.vhd").exists() {
            maps.push("libb");
        }
        self.analyze("work", &maps, &f);
        name
    }

    fn run(&self, top: &str, more: &[&str]) -> Out {
        self.run_env(top, more, &[])
    }

    fn run_env(&self, top: &str, more: &[&str], env: &[(&str, &str)]) -> Out {
        let mut a = vec![
            "run".to_string(),
            format!("--work={}", self.spec("work")),
            format!("--map={}", self.spec("liba")),
        ];
        if self.tmp.join("vals.vhd").exists() {
            a.push(format!("--map={}", self.spec("libb")));
        }
        a.extend(more.iter().map(|s| s.to_string()));
        a.push(top.to_string());
        self.yavs_env(&a, env)
    }

    fn entries(&self, kind: &str) -> Vec<PathBuf> {
        let d = self.tmp.join("liba").join("yavs-cache");
        std::fs::read_dir(d)
            .map(|d| {
                d.flatten()
                    .map(|e| e.path())
                    .filter(|p| {
                        let n = p.file_name().unwrap_or_default().to_string_lossy();
                        n.starts_with(&format!("{kind}-")) && n.ends_with(".img")
                    })
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Testbenches of one library share what the library's files give: the request of the fourth
/// closure for the prefix makes the lowered library (`lib`), the next runs read it, the state of its packages
/// after initialisation (`libinit`) and the optimised copies of the library functions the runs
/// called (`libopt`). Every run prints what a run without the cache prints, with an
/// elaboration-time call of a package function (a session) and without, at one thread and four.
/// Once the library is made every run of a closure reads them, and no run makes entries of its own.
#[test]
fn closures_share_the_lowered_library_and_its_state() {
    let l = Libs::new("share", false);
    let tops: Vec<String> = (1..=10).map(|n| l.tb(n, n % 2 == 1)).collect();
    let plain: Vec<Out> = tops
        .iter()
        .map(|t| visible(&l.run(t, &["--no-cache"])))
        .collect();
    for (n, (p, t)) in plain.iter().zip(&tops).enumerate() {
        assert!(p.stdout.contains(&format!("k={}", 46 + n)), "{t}: {p:?}");
        assert_eq!(p.code, 0, "{p:?}");
    }
    // The first closure only notes the prefix, the second makes its image (#549); the next ones,
    // up to the one that makes the library, lower what they reach themselves.
    let r1 = l.run(&tops[0], &[]);
    assert!(!note(&r1, "store prefix of 10 files"), "{r1:?}");
    for (i, t) in tops.iter().enumerate().take(SIGHTINGS as usize - 1) {
        let r = l.run(t, &[]);
        assert_eq!(note(&r, "store prefix of 10 files"), i == 1, "{t}: {r:?}");
        assert!(
            !note(&r, "store prefix lib ") && !note(&r, "hit prefix lib "),
            "{t}: {r:?}"
        );
        assert_eq!(visible(&r), plain[i], "{t}");
    }
    assert!(l.entries("lib").is_empty());
    // The fourth closure makes the lowered library.
    let k = SIGHTINGS as usize - 1;
    let r4 = l.run(&tops[k], &[]);
    assert!(note(&r4, "store prefix lib "), "{r4:?}");
    assert_eq!(visible(&r4), plain[k]);
    assert_eq!(l.entries("lib").len(), 1);
    // The next ones read it, with the state of the packages and the optimised copies.
    let mut seen = std::collections::HashSet::new();
    for (i, t) in tops.iter().enumerate().skip(k + 1) {
        let more: &[&str] = if i % 2 == 0 { &[] } else { &["--threads=4"] };
        let r = l.run(t, more);
        assert!(note(&r, "hit prefix lib "), "{t}: {r:?}");
        assert!(!note(&r, "store prefix lib "), "{t}: {r:?}");
        assert_eq!(visible(&r), plain[i], "{t}");
        for kind in ["libinit", "libopt"] {
            if note(&r, &format!("hit prefix {kind}")) {
                seen.insert(kind);
            }
        }
    }
    assert_eq!(seen.len(), 2, "{seen:?}");
    assert_eq!(l.entries("libopt").len(), 1, "one entry names the lowering");
    assert_eq!(l.entries("libinit").len(), 1);
    // The same closure again keeps reading the library's stages and makes no entry of its own
    // (a closure's copy of the library is slower to read and fills the directory). The entries
    // there are those of tops[0], which ran twice before the library was made.
    let own = || {
        std::fs::read_dir(l.tmp.join("work").join("yavs-cache"))
            .map(|d| {
                d.flatten()
                    .filter(|e| e.file_name().to_string_lossy().ends_with(".img"))
                    .count()
            })
            .unwrap_or(0)
    };
    let before = own();
    for _ in 0..3 {
        let again = l.run(&tops[9], &[]);
        assert!(note(&again, "hit prefix lib "), "{again:?}");
        assert!(!note(&again, "store "), "{again:?}");
        assert_eq!(visible(&again), plain[9]);
    }
    assert_eq!(own(), before, "no entries of the closure's own");
}

/// The lowered library depends on the files of the prefix: editing a package of it makes a new
/// prefix, whose runs do not read the old entries.
#[test]
fn editing_a_package_of_the_prefix_replaces_the_library() {
    let l = Libs::new("edit", false);
    for n in 1..=SIGHTINGS {
        let t = l.tb(n, true);
        l.run(&t, &[]);
    }
    let t = l.tb(SIGHTINGS + 1, true);
    assert!(note(&l.run(&t, &[]), "hit prefix lib "));
    let f = l.tmp.join("pa9.vhd");
    let text = std::fs::read_to_string(&f).expect("read");
    std::fs::write(&f, text.replace("+ 9", "+ 90")).expect("write");
    l.analyze("liba", &[], &f);
    // The new prefix is made by the fourth closure that sees it and read by the next.
    let from = SIGHTINGS + 2;
    for n in from..=from + SIGHTINGS {
        let t = l.tb(n, true);
        let plain = visible(&l.run(&t, &["--no-cache"]));
        assert!(
            plain.stdout.contains(&format!("k={}", 126 + n)),
            "{plain:?}"
        );
        let r = l.run(&t, &[]);
        assert_eq!(visible(&r), plain, "{t}");
        assert_eq!(
            note(&r, "hit prefix lib "),
            n == from + SIGHTINGS,
            "{t}: {r:?}"
        );
        assert_eq!(
            note(&r, "store prefix lib "),
            n == from + SIGHTINGS - 1,
            "{t}: {r:?}"
        );
    }
    assert_eq!(
        l.entries("lib").len(),
        2,
        "the old library stays until it ages out"
    );
}

/// A package of the prefix whose body is in a file that cannot follow it (it needs a unit of
/// another library that comes later) is completed after the prefix: what the prefix lowered
/// could have used the body's constant, so it is not reused, and a changed body changes what
/// every run prints.
#[test]
fn a_package_completed_after_the_prefix_is_not_reused() {
    let l = Libs::new("late", true);
    let tops: Vec<String> = (1..=4).map(|n| l.tb(n, true)).collect();
    for t in &tops {
        let plain = visible(&l.run(t, &["--no-cache"]));
        assert!(plain.stdout.contains("kb=101"), "{t}: {plain:?}");
        for round in 0..3 {
            let r = l.run(t, &[]);
            assert!(
                !note(&r, "hit prefix lib ") && !note(&r, "store prefix lib "),
                "{t}: {r:?}"
            );
            // Only the first run of a closure asks for the library.
            assert_eq!(
                note(&r, "a package body follows the prefix"),
                round == 0,
                "{t}: {r:?}"
            );
            assert_eq!(visible(&r), plain, "{t}");
        }
    }
    // The body changes: the prefix does not, and every run prints the new value.
    let body = l.tmp.join("pa7_body.vhd");
    let text = std::fs::read_to_string(&body).expect("read");
    std::fs::write(&body, text.replace("100 +", "200 +")).expect("write");
    l.analyze("liba", &["libb"], &body);
    for n in 5..=7 {
        let t = l.tb(n, true);
        let r = l.run(&t, &[]);
        assert!(r.stdout.contains("kb=201"), "{t}: {r:?}");
        assert_eq!(visible(&r), visible(&l.run(&t, &["--no-cache"])), "{t}");
    }
    for t in &tops {
        let r = l.run(t, &[]);
        assert!(r.stdout.contains("kb=201"), "{t}: {r:?}");
    }
    assert!(l.entries("lib").is_empty());
}

/// Testbenches that start together on a new prefix all print what a run without the cache prints,
/// the library is made once (the others do not wait for it), and nothing is left half written.
#[test]
fn parallel_first_runs_into_one_library_agree() {
    let l = Libs::new("parallel", false);
    let tops: Vec<String> = (1..=8).map(|n| l.tb(n, n % 2 == 0)).collect();
    let plain: Vec<Out> = tops
        .iter()
        .map(|t| visible(&l.run(t, &["--no-cache"])))
        .collect();
    // The prefix has been seen by all but one of the closures that make the library, so one of
    // the runs below makes it while the others go on.
    let before = SIGHTINGS as usize - 1;
    for (t, p) in tops.iter().zip(&plain).take(before) {
        assert_eq!(visible(&l.run(t, &[])), *p);
    }
    std::thread::scope(|s| {
        let hs: Vec<_> = tops
            .iter()
            .skip(before)
            .map(|t| s.spawn(|| visible(&l.run(t, &[]))))
            .collect();
        for (h, p) in hs.into_iter().zip(plain.iter().skip(before)) {
            assert_eq!(*p, h.join().expect("join"));
        }
    });
    assert_eq!(l.entries("lib").len(), 1);
    let leftovers = std::fs::read_dir(l.tmp.join("liba").join("yavs-cache"))
        .expect("dir")
        .flatten()
        .filter(|f| {
            let n = f.file_name().to_string_lossy().into_owned();
            n.starts_with(".tmp-") || n.ends_with(".making")
        })
        .count();
    assert_eq!(leftovers, 0);
    // Four more at once read it.
    let more: Vec<String> = (9..=12).map(|n| l.tb(n, n % 2 == 0)).collect();
    let plain: Vec<Out> = more
        .iter()
        .map(|t| visible(&l.run(t, &["--no-cache"])))
        .collect();
    std::thread::scope(|s| {
        let hs: Vec<_> = more
            .iter()
            .map(|t| s.spawn(|| l.run(t, &["--threads=4"])))
            .collect();
        for (h, p) in hs.into_iter().zip(&plain) {
            let r = h.join().expect("join");
            assert!(note(&r, "hit prefix lib "), "{r:?}");
            assert_eq!(*p, visible(&r));
        }
    });
}

/// A damaged entry of the library stages is a miss: the run lowers what it needs itself, prints
/// what it printed, and the entry is made again.
#[test]
fn damaged_library_entries_are_ignored() {
    let l = Libs::new("damaged", false);
    for n in 1..=SIGHTINGS {
        let t = l.tb(n, true);
        l.run(&t, &[]);
    }
    let mut n = SIGHTINGS;
    for kind in ["lib", "libinit", "libopt"] {
        let img = l
            .entries(kind)
            .pop()
            .unwrap_or_else(|| panic!("no {kind} entry"));
        let good = std::fs::read(&img).expect("read");
        for bad in [
            good[..good.len() / 2].to_vec(),
            b"not an entry".to_vec(),
            Vec::new(),
        ] {
            std::fs::write(&img, bad).expect("write");
            n += 1;
            let t = l.tb(n, true);
            let r = l.run(&t, &[]);
            assert_eq!(
                visible(&r),
                visible(&l.run(&t, &["--no-cache"])),
                "{kind}: {r:?}"
            );
            std::fs::write(&img, &good).expect("restore");
        }
    }
    let t = l.tb(n + 1, true);
    let r = l.run(&t, &[]);
    assert!(note(&r, "hit prefix lib "), "{r:?}");
    assert_eq!(visible(&r), visible(&l.run(&t, &["--no-cache"])));
}

/// The library stages hold generated code: another optimiser switch or another build never reads
/// them (#491), and the settings that made them still do.
#[test]
fn library_stages_depend_on_the_optimiser_and_the_build() {
    let l = Libs::new("switch", false);
    let tops: Vec<String> = (1..=10).map(|n| l.tb(n, n % 2 == 1)).collect();
    let plain = |i: usize| visible(&l.run(&tops[i], &["--no-cache"]));
    // The first closure leaves the prefix image, the fourth makes the library.
    let _ = l.run(&tops[0], &[]);
    for t in tops.iter().take(SIGHTINGS as usize) {
        let _ = l.run(t, &[]);
    }
    // A closure's second run uses entries of its own: each probe is the first run of a closure.
    let probes = [
        (4, Some(("YAVS_NO_CFG", "1"))),
        (5, Some(("YAVS_NO_MEMO", "1"))),
        (6, Some(("YAVS_NO_IROPT", "1"))),
        (7, Some(("YAVS_CACHE_SALT", "another build"))),
        (8, None),
    ];
    for (i, env) in probes {
        let envs: Vec<_> = env.into_iter().collect();
        let r = l.run_env(&tops[i], &[], &envs);
        assert_eq!(note(&r, "hit prefix lib "), env.is_none(), "{env:?}: {r:?}");
        assert!(
            env.is_none() || !note(&r, "hit prefix libopt"),
            "{env:?}: {r:?}"
        );
        assert_eq!(visible(&r), plain(i), "{env:?}");
    }
}
