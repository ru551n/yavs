//! A warm Cranelift run links its code by a key made before the body is lowered
//! (`yavs-clift` `cache.rs`, docs/decisions.md "Cranelift: compiled code found before the body is
//! made"). Every run here compiles at first entry (`--tier-heat=1,0,0`), so nothing depends on the
//! sampler. Debug builds also leave an input out of the key on purpose (`YAVS_CLIFT_PREKEY_DROP`)
//! and show the check notices: the mutation test of the key.
#![cfg(feature = "clift")]

mod common;
use common::tempdir;

use std::path::{Path, PathBuf};
use std::process::Command;

/// Calls that inline, a table of constants, copies of a process with other generics and facts.
const SRC: &str = "entity pk_acc is
  generic (W : positive := 8; K : natural := 1);
  port (clk : in bit; q : out bit_vector(W - 1 downto 0));
end;
architecture a of pk_acc is
  signal r : bit_vector(W - 1 downto 0) := (others => '1');
begin
  process (clk) begin
    if clk = '1' then
      r <= r(W - 2 downto 0) & (r(W - 1) xor r(K));
    end if;
  end process;
  q <= r;
end;

entity pk is end;
architecture a of pk is
  type tab_t is array (0 to 7) of integer;
  signal clk : bit := '0';
  signal n : integer := 0;
  signal q8a, q8b : bit_vector(7 downto 0);
  signal q12 : bit_vector(11 downto 0);
  function one(x : integer) return integer is
    variable v : integer := x;
  begin
    for i in 1 to 3 loop
      v := (v * 75 + 74) mod 65537;
    end loop;
    return v;
  end;
  function two(x : integer) return integer is
    variable v : integer := x;
  begin
    for i in 1 to 3 loop
      v := (v * 75 + 74) mod 65537;
    end loop;
    return v;
  end;
  function small(x : integer) return integer is
    variable a, b : integer;
  begin
    a := (x + 1) mod 1009;
    b := (a * 3 + 7) mod 1013;
    a := (b * 5 + a) mod 1019;
    b := (a * 7 + b) mod 1021;
    a := (b * 11 + a) mod 1031;
    b := (a * 13 + b) mod 1033;
    return (a + b) mod 2039;
  end;
  function pw(x : integer) return integer is
    variable a, b : integer;
  begin
    a := (x + 1) mod 1009;
    b := (a * 3 + 7) mod 1013;
    a := (b * 5 + a) mod 1019;
    b := (a * 7 + b) mod 1021;
    a := (b * 11 + a) mod 1031;
    return (a mod 5 + 1) ** (b mod 3 + 1);
  end;
  function pick(i : integer) return integer is
    constant tab : tab_t := (11, 12, 13, 14, 15, 16, 17, 18);
  begin
    return tab(i);
  end;
  function enc(b : boolean) return string is
  begin
    if b then
      return \"TTTT\";
    else
      return \"FFFF\";
    end if;
  end;
  function outer(x : integer) return integer is
  begin
    return small(one(x)) + small(two(x)) + pw(x);
  end;
begin
  clk <= not clk after 5 ns when n < 60;
  u0 : entity work.pk_acc generic map (8, 1) port map (clk, q8a);
  u1 : entity work.pk_acc generic map (8, 3) port map (clk, q8b);
  u2 : entity work.pk_acc generic map (12, 5) port map (clk, q12);
  process (clk)
    variable acc : integer := 1;
    variable i : integer := 0;
  begin
    if clk = '1' then
      i := (i + 1) mod 8;
      acc := outer(acc + pick(i));
      n <= n + 1;
      if n = 50 then
        report integer'image(acc) & enc(acc mod 2 = 0) & enc(acc mod 3 = 0);
      end if;
    end if;
  end process;
end;
";

struct Run {
    ok: bool,
    stdout: String,
    stderr: String,
}

impl Run {
    fn count(&self, what: &str) -> usize {
        let start = format!("debug: cache: clif {what}");
        self.stderr
            .lines()
            .filter(|l| l.starts_with(&start))
            .count()
    }
}

struct World {
    dir: common::Tmp,
    work: String,
    tier: &'static str,
}

impl World {
    fn new(tag: &str, tier: &'static str) -> World {
        let dir = tempdir(tag);
        let work = format!("work:{}", dir.join("lib").display());
        let w = World { dir, work, tier };
        w.analyze(SRC);
        w
    }

    fn analyze(&self, src: &str) {
        std::fs::write(self.dir.join("pk.vhd"), src).expect("write");
        let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
            .current_dir(&self.dir)
            .args(["analyze", "--work", &self.work, "pk.vhd"])
            .output()
            .expect("run yavs");
        assert!(o.status.success(), "{o:?}");
    }

    fn run(&self, threads: &str, env: &[(&str, &str)]) -> Run {
        let tier = format!("--tier={}", self.tier);
        let mut args = vec!["run", "--work", &self.work, &tier];
        if self.tier == "clift-auto" {
            args.push("--tier-heat=1,0,0");
        }
        args.extend(["--threads", threads, "--shuffle", "pk"]);
        let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
            .current_dir(&self.dir)
            .envs([("YAVS_LOG", "debug"), ("YAVS_SPEC_AFTER", "2")])
            .envs(env.iter().copied())
            .args(args)
            .output()
            .expect("run yavs");
        Run {
            ok: o.status.success(),
            stdout: String::from_utf8_lossy(&o.stdout).into_owned(),
            stderr: String::from_utf8_lossy(&o.stderr).into_owned(),
        }
    }

    /// The first run has no stored program, so no pre-key (the closure's entry comes with its
    /// second); the second writes the entries.
    fn warmed(&self, env: &[(&str, &str)]) {
        for _ in 0..2 {
            let r = self.run("1", env);
            assert!(r.ok, "{}", r.stderr);
        }
    }

    /// A run that lowered nothing. (A compile the end of a run cut off comes in the next one, so
    /// allow a few, whatever the load of the host.)
    fn settled(&self, threads: &str) -> Run {
        for _ in 0..8 {
            let r = self.run(threads, &[NO_CHECK]);
            assert!(r.ok, "{}", r.stderr);
            if r.count("lowered") == 0 {
                return r;
            }
        }
        panic!("every run lowered something");
    }

    fn objects(&self, kind: &str) -> Vec<PathBuf> {
        let dir = self.dir.join("lib").join("yavs-cache").join("obj");
        let prefix = format!("{kind}-");
        std::fs::read_dir(dir)
            .expect("the objects store")
            .flatten()
            .map(|e| e.path())
            .filter(|p| name_of(p).starts_with(&prefix))
            .collect()
    }
}

fn name_of(p: &Path) -> String {
    p.file_name()
        .map_or(String::new(), |n| n.to_string_lossy().into_owned())
}

const NO_CHECK: (&str, &str) = ("YAVS_CLIFT_PREKEY_CHECK", "0");
const CHECK: (&str, &str) = ("YAVS_CLIFT_PREKEY_CHECK", "1");

#[test]
fn a_warm_run_lowers_nothing_and_the_check_agrees() {
    for (tag, tier) in [("pk_auto", "clift-auto"), ("pk_eager", "clift")] {
        let w = World::new(tag, tier);
        let cold = w.run("1", &[NO_CHECK]);
        assert!(cold.ok, "{}", cold.stderr);
        // (No program identity yet: every function is lowered and nothing is named.)
        assert!(cold.count("lowered") > 0, "{}", cold.stderr);
        assert_eq!(cold.count("prehit"), 0, "{}", cold.stderr);
        assert!(w.run("1", &[NO_CHECK]).ok);
        for threads in ["1", "4"] {
            let warm = w.settled(threads);
            assert!(warm.count("prehit") > 0, "{}", warm.stderr);
            assert_eq!(warm.stdout, cold.stdout);
            let checked = w.run(threads, &[CHECK]);
            assert!(checked.ok, "{}", checked.stderr);
            assert!(checked.count("lowered") > 0, "{}", checked.stderr);
            assert!(checked.count("pre checked") > 0, "{}", checked.stderr);
            // (The check goes on by the entry, so the link is what runs under it too.)
            assert!(checked.count("prehit") > 0, "{}", checked.stderr);
            assert_eq!(checked.stdout, cold.stdout);
        }
    }
}

/// A caller the cache does not have, over a variant it does: the callee's body is made for the
/// inliner, and the caller comes out as the cold run made it (the check holds it to the entry).
#[test]
fn a_caller_that_misses_gets_the_body_of_a_linked_callee() {
    let w = World::new("pk_mixed", "clift-auto");
    w.warmed(&[NO_CHECK]);
    let big = w.objects("clifpre");
    assert!(big.len() >= 2, "{big:?}");
    let reference = w.settled("1");
    // Each entry alone: every caller and every callee takes its turn at missing.
    for p in &big {
        let keep = std::fs::read(p).expect("entry");
        std::fs::remove_file(p).expect("remove");
        let r = w.run("1", &[NO_CHECK]);
        assert!(r.ok, "{}", r.stderr);
        assert_eq!(r.stdout, reference.stdout);
        // The entry is back, the same one.
        assert_eq!(
            std::fs::read(p).ok().as_ref(),
            Some(&keep),
            "{}",
            name_of(p)
        );
        let c = w.run("1", &[CHECK]);
        assert!(c.ok, "{}", c.stderr);
    }
}

/// Anything wrong with what the store holds is a miss: the run lowers, prints the same and the
/// next run links again. Another format or build never reads the entry.
#[test]
fn a_bad_or_foreign_entry_is_a_miss() {
    let w = World::new("pk_bad", "clift-auto");
    w.warmed(&[NO_CHECK]);
    let reference = w.settled("1");
    assert!(reference.count("prehit") > 0, "{}", reference.stderr);
    let again = |what: &str| {
        let r = w.run("1", &[NO_CHECK]);
        assert!(r.ok, "{what}: {}", r.stderr);
        assert!(r.count("lowered") > 0, "{what}: {}", r.stderr);
        assert_eq!(r.stdout, reference.stdout, "{what}");
        let n = w.settled("1");
        assert!(n.count("prehit") > 0, "{what}: {}", n.stderr);
    };
    // Another format version in the header (magic, then format).
    for p in w.objects("clifpre") {
        let mut b = std::fs::read(&p).expect("entry");
        b[8] ^= 0xff;
        std::fs::write(&p, b).expect("write");
    }
    again("another format");
    // A payload that is not an entry, under a valid header: the checksum says so.
    for p in w.objects("clifpre") {
        let mut b = std::fs::read(&p).expect("entry");
        let last = b.len() - 1;
        b[last] ^= 0xff;
        std::fs::write(&p, b).expect("write");
    }
    again("damaged payload");
    // The code an entry names has gone.
    for p in w.objects("clif") {
        std::fs::remove_file(p).expect("remove");
    }
    again("no code");
    // Another build: no entry of this one is read.
    let salted = [NO_CHECK, ("YAVS_CACHE_SALT", "another build")];
    let r = w.run("1", &salted);
    assert!(r.ok, "{}", r.stderr);
    assert_eq!(r.count("prehit"), 0, "{}", r.stderr);
    assert_eq!(r.stdout, reference.stdout);
}

/// With input `drop` left out of the key, a change of it reaches an entry made without it, and
/// the check stops the run.
#[cfg(debug_assertions)]
mod mutations {
    use super::*;

    const CAUGHT: &str = "missing from the pre-key";

    fn caught(r: &Run, what: &str) {
        assert!(
            !r.ok && r.stderr.contains(CAUGHT),
            "{what}: not caught: {}",
            r.stderr
        );
    }

    /// `drop` out of the key; the run under `change` must be stopped, and with the whole key not.
    fn by_env(tag: &str, tier: &'static str, drop: &str, change: (&str, &str)) {
        let w = World::new(tag, tier);
        let d = ("YAVS_CLIFT_PREKEY_DROP", drop);
        w.warmed(&[d, NO_CHECK]);
        caught(&w.run("1", &[d, CHECK, change]), drop);
        let whole = World::new(&format!("{tag}_whole"), tier);
        whole.warmed(&[]);
        let r = whole.run("1", &[CHECK, change]);
        assert!(r.ok, "{drop}: the whole key: {}", r.stderr);
    }

    /// As `by_env`, the change being an edit of the design.
    fn by_edit(tag: &str, drop: &str, from: &str, to: &str) {
        assert!(SRC.contains(from));
        let w = World::new(tag, "clift-auto");
        let d = ("YAVS_CLIFT_PREKEY_DROP", drop);
        w.warmed(&[d, NO_CHECK]);
        w.analyze(&SRC.replace(from, to));
        // (The first run of the new closure has no stored program, so no pre-key.)
        assert!(w.run("1", &[d, NO_CHECK]).ok);
        caught(&w.run("1", &[d, CHECK]), drop);
        let whole = World::new(&format!("{tag}_whole"), "clift-auto");
        whole.warmed(&[]);
        whole.analyze(&SRC.replace(from, to));
        for _ in 0..2 {
            let r = whole.run("1", &[CHECK]);
            assert!(r.ok, "{drop}: the whole key: {}", r.stderr);
        }
    }

    #[test]
    fn the_facts_of_a_copy() {
        by_env("pk_m_facts", "clift-auto", "facts", ("YAVS_NO_FACTS", "1"));
    }

    #[test]
    fn a_switch_of_the_lowering() {
        by_env(
            "pk_m_sw",
            "clift-auto",
            "switches",
            ("YAVS_CLIFT_ONCE", "0"),
        );
    }

    /// `YAVS_AUTO_POLICY` decides whether `**` runs on a native frame: read outside the lowering.
    #[test]
    fn the_compile_policy() {
        by_env(
            "pk_m_policy",
            "clift-auto",
            "switches",
            ("YAVS_AUTO_POLICY", "sampled"),
        );
    }

    /// The other way: a refusal stored by a run of the old policy does not hold for the rule.
    #[test]
    fn a_refusal_of_the_old_policy() {
        let old = [("YAVS_AUTO_POLICY", "sampled")];
        let w = World::new("pk_m_policy_back", "clift-auto");
        let d = ("YAVS_CLIFT_PREKEY_DROP", "switches");
        w.warmed(&[d, NO_CHECK, old[0]]);
        caught(&w.run("1", &[d, CHECK]), "switches");
        let whole = World::new("pk_m_policy_back_whole", "clift-auto");
        whole.warmed(&old);
        let r = whole.run("1", &[CHECK]);
        assert!(r.ok, "the whole key: {}", r.stderr);
    }

    #[test]
    fn the_flags_of_the_code_generator() {
        by_env(
            "pk_m_flags",
            "clift-auto",
            "target,switches",
            ("YAVS_CLIFT_OPT", "none"),
        );
    }

    /// The route and the callees' bodies: `YAVS_CLIFT_NTAB` calls through the table.
    #[test]
    fn the_route() {
        by_env(
            "pk_m_route",
            "clift",
            "callees,switches",
            ("YAVS_CLIFT_NTAB", "1"),
        );
    }

    /// The body of an inlined callee, which only the callee's own key says.
    #[test]
    fn an_inlined_callee() {
        by_env(
            "pk_m_inl",
            "clift",
            "callees,switches",
            ("YAVS_CLIFT_INLINE", "100000"),
        );
    }

    /// The optimiser's switches change the program and every body.
    #[test]
    fn the_fingerprint() {
        let w = World::new("pk_m_fp", "clift-auto");
        let d = ("YAVS_CLIFT_PREKEY_DROP", "fingerprint,identity,func");
        w.warmed(&[d, NO_CHECK]);
        caught(
            &w.run("1", &[d, CHECK, ("YAVS_NO_INLINE", "1")]),
            "fingerprint",
        );
    }

    /// Another value of a constant: the code is the same, the module data it refers to is not.
    /// This one would be a wrong result with no error at all.
    #[test]
    fn a_constant_of_the_program() {
        by_edit("pk_m_const", "identity", "TTTT", "UUUU");
    }

    #[test]
    fn the_body_of_the_function() {
        by_edit(
            "pk_m_body",
            "identity,func",
            "(v * 75 + 74)",
            "(v * 76 + 74)",
        );
    }

    /// `one` and `two` differ in their names and their ids alone: a native frame names its id.
    /// Left out, the second finds the entry of the first.
    #[test]
    fn the_id_of_the_function() {
        let w = World::new("pk_m_id", "clift");
        let d = ("YAVS_CLIFT_PREKEY_DROP", "base,func");
        assert!(w.run("1", &[d, NO_CHECK]).ok);
        caught(&w.run("1", &[d, CHECK]), "base");
    }

    /// `--tier=clift` makes the frame-mode code and the native variant of a function.
    #[test]
    fn frame_mode_or_native() {
        let w = World::new("pk_m_nat", "clift");
        let d = ("YAVS_CLIFT_PREKEY_DROP", "native");
        assert!(w.run("1", &[d, NO_CHECK]).ok);
        caught(&w.run("1", &[d, CHECK]), "native");
    }
}
