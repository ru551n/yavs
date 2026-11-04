//! What the key of a process's copies leaves out (`yavs-ir` `shape.rs`,
//! docs/design/specialisation.md "Shared bodies"): instances that differ in a value the process
//! only computes with share one body, and it reads the value from the running instance. Debug
//! builds also fold an input the key left out on purpose (`YAVS_SHARE_KEY_DROP`) and show the check
//! at the switch (`Program::check_copy`) notices: the mutation test of the key.

mod common;
use common::tempdir;

use std::collections::BTreeMap;
use std::path::Path;
use std::process::Command;

/// Four instances that differ in `K` and `R` (a branch, a mask), and two widths.
const SRC: &str = "entity sk_pe is
  generic (W : positive := 8; K : natural := 1; R : natural := 0);
  port (clk : in bit; q : out natural);
end;
architecture a of sk_pe is
  signal r0 : bit_vector(W - 1 downto 0) := (others => '1');
begin
  mac : process (clk)
    variable n : natural := 0;
  begin
    if clk = '1' then
      r0 <= r0(W - 2 downto 0) & r0(W - 1);
      if R mod 2 = 0 then
        n := (n + K) mod 1009;
      else
        n := (n * 3 + K + R) mod 1009;
      end if;
      if r0(W - 1) = '1' then
        n := (n + 1) mod 1009;
      end if;
      q <= n;
    end if;
  end process;
end;
entity sk is
  generic (W2 : positive := 8);
end;
architecture t of sk is
  signal clk : bit := '0';
  type qs_t is array (0 to 3) of natural;
  signal qs : qs_t;
begin
  clk <= not clk after 5 ns;
  u0 : entity work.sk_pe generic map (W => 8, K => 3, R => 0) port map (clk, qs(0));
  u1 : entity work.sk_pe generic map (W => 8, K => 37, R => 1) port map (clk, qs(1));
  u2 : entity work.sk_pe generic map (W => 8, K => 101, R => 4) port map (clk, qs(2));
  u3 : entity work.sk_pe generic map (W => W2, K => 7, R => 7) port map (clk, qs(3));
  process
  begin
    wait for 1 us;
    report integer'image(qs(0)) & \" \" & integer'image(qs(1)) & \" \" & integer'image(qs(2)) & \" \" & integer'image(qs(3));
    std.env.finish;
  end process;
end;
";

struct Run {
    ok: bool,
    out: String,
    err: String,
    counters: BTreeMap<String, u64>,
}

struct World {
    dir: common::Tmp,
}

impl World {
    fn new(tag: &str) -> World {
        World::with(tag, SRC)
    }

    fn with(tag: &str, src: &str) -> World {
        let dir = tempdir(tag);
        std::fs::write(dir.join("sk.vhd"), src).expect("write");
        let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
            .current_dir(&dir)
            .args(["analyze", "--std=2008", "--work=work:lib", "sk.vhd"])
            .output()
            .expect("run yavs");
        assert!(o.status.success(), "{o:?}");
        World { dir }
    }

    fn run(&self, env: &[(&str, &str)], args: &[&str]) -> Run {
        go(&self.dir, env, args)
    }
}

fn go(dir: &Path, env: &[(&str, &str)], args: &[&str]) -> Run {
    let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .current_dir(dir)
        .env("YAVS_COUNTERS", "1")
        .env("YAVS_SPEC_AFTER", "2")
        .envs(env.iter().copied())
        .args(["--color", "never", "run", "--std=2008", "--work=work:lib"])
        .args(args)
        .arg("sk")
        .output()
        .expect("run yavs");
    let all = String::from_utf8_lossy(&o.stderr).into_owned();
    let counters = all
        .lines()
        .filter_map(|l| l.strip_prefix("counters: ")?.split_once('='))
        .map(|(k, v)| (k.to_string(), v.parse().expect("counter value")))
        .collect();
    let err: String = all
        .lines()
        .filter(|l| !l.starts_with("counters: "))
        .map(|l| format!("{l}\n"))
        .collect();
    Run {
        ok: o.status.success(),
        out: String::from_utf8_lossy(&o.stdout).into_owned() + &err,
        err,
        counters,
    }
}

const OFF: (&str, &str) = ("YAVS_SHARE_KEY", "off");
const UNREAD: (&str, &str) = ("YAVS_SHARE_KEY", "unread");
const VALUE: (&str, &str) = ("YAVS_SHARE_KEY", "value");

fn copies(r: &Run) -> u64 {
    r.counters["spec_copies"]
}

#[test]
fn values_the_process_only_computes_with_share_a_body() {
    let w = World::new("sk_share");
    for threads in ["--threads=1", "--threads=4"] {
        let args = [threads];
        let (off, unread, value) = (
            w.run(&[OFF], &args),
            w.run(&[UNREAD], &args),
            w.run(&[VALUE], &args),
        );
        assert!(off.ok, "{}", off.err);
        assert_eq!(off.out, value.out, "values held");
        assert_eq!(off.out, unread.out, "unread left out");
        assert!(copies(&unread) <= copies(&off));
        // The tester's process, its `q` conversion and clock are the same in each; `mac` splits
        // in four without sharing and is one body with it.
        assert!(
            copies(&value) + 3 <= copies(&off),
            "{} copies, {} without",
            copies(&value),
            copies(&off)
        );
        assert_eq!(
            value.counters["spec_switched"],
            off.counters["spec_switched"]
        );
    }
}

#[test]
fn a_width_stays_in_the_key() {
    let w = World::new("sk_width");
    let same = w.run(&[VALUE], &[]);
    let other = w.run(&[VALUE], &["-gW2=9"]);
    let off = w.run(&[OFF], &["-gW2=9"]);
    assert_eq!(
        copies(&same) + 1,
        copies(&other),
        "one more copy for the other width"
    );
    assert_eq!(other.out, off.out);
}

#[test]
fn the_same_under_every_compiled_form() {
    if !cfg!(feature = "clift") {
        return;
    }
    let w = World::new("sk_tier");
    let off = w.run(&[OFF], &["--tier=clift-auto", "--tier-heat=1,0,0"]);
    let on = w.run(&[VALUE], &["--tier=clift-auto", "--tier-heat=1,0,0"]);
    let warm = w.run(&[VALUE], &["--tier=clift-auto", "--tier-heat=1,0,0"]);
    assert!(off.ok, "{}", off.err);
    assert_eq!(off.out, on.out);
    assert_eq!(off.out, warm.out);
}

#[test]
fn a_warm_run_finds_the_shared_bodies_and_checks_them() {
    let w = World::new("sk_warm");
    let check = ("YAVS_SPEC_CHECK", "1");
    let first = w.run(&[VALUE, check], &[]);
    let mut second = w.run(&[VALUE, check], &[]);
    // (The first run of a closure leaves only a note in the cache.)
    for _ in 0..3 {
        if second.counters["spec_loaded"] > 0 {
            break;
        }
        second = w.run(&[VALUE, check], &[]);
    }
    assert!(second.counters["spec_loaded"] > 0, "{:?}", second.counters);
    assert_eq!(first.out, second.out);
    assert_eq!(second.counters["spec_fresh"], 0);
}

/// A copy stored without holes is not found by a run that has them, whose zero there may be what
/// the other folded: `R => 0` and `K => 0` are the instance whose masked data equal its own.
#[test]
fn a_stored_copy_is_found_only_with_its_holes() {
    let src = SRC.replace("K => 3, R => 0", "K => 0, R => 0");
    let w = World::with("sk_holes", &src);
    let check = ("YAVS_SPEC_CHECK", "1");
    let off = w.run(&[OFF, check], &[]);
    let off2 = w.run(&[OFF, check], &[]);
    assert_eq!(off.out, off2.out);
    let on = w.run(&[VALUE, check], &[]);
    assert!(on.ok, "{}", on.err);
    assert_eq!(off.out, on.out, "the value held by the other instances");
}

/// The growth cap counts a copy's share per instance it serves: sixteen instances of two widths
/// fit, sixteen bodies do not.
#[test]
fn the_growth_cap_counts_a_copy_by_the_instances_it_serves() {
    let src = include_str!("perf/share_values.vhd").replace("share_values", "sk");
    let w = World::with("sk_cap", &src);
    let cap = ("YAVS_SPEC_CAP", "100");
    let (off, on) = (w.run(&[OFF, cap], &[]), w.run(&[VALUE, cap], &[]));
    assert!(off.ok, "{}", off.err);
    assert!(off.out.contains("sum="), "{}", off.out);
    assert_eq!(off.out, on.out);
    assert!(off.counters["spec_capped"] > 0, "{:?}", off.counters);
    assert_eq!(on.counters["spec_capped"], 0, "{:?}", on.counters);
}

/// Each mutant makes a copy that folded a value of one instance run on another.
#[cfg(debug_assertions)]
mod mutants {
    use super::*;

    const NO_CHECK: (&str, &str) = ("YAVS_SPEC_CHECK", "0");
    const CHECK: (&str, &str) = ("YAVS_SPEC_CHECK", "1");

    fn caught(r: &Run, what: &str) {
        assert!(!r.ok, "{what}: the run went through: {}", r.err);
        assert!(
            r.err.contains("internal error") && r.err.contains("instance data"),
            "{what}: {}",
            r.err
        );
    }

    fn mutant(tag: &str, drop: &str) {
        let w = World::new(tag);
        let good = w.run(&[VALUE, NO_CHECK], &[]);
        assert!(good.ok, "{}", good.err);
        let d = ("YAVS_SHARE_KEY_DROP", drop);
        caught(&w.run(&[VALUE, CHECK, d], &[]), drop);
        // And the wrong result it stopped is real.
        let bad = w.run(&[VALUE, NO_CHECK, d], &[]);
        assert_ne!(
            good.out, bad.out,
            "{drop}: without the check the output is the same"
        );
    }

    /// The optimiser folds the hole (a zero in the masked data).
    #[test]
    fn a_fold_of_a_hole() {
        mutant("sk_m_fold", "fold");
    }

    /// The key leaves a value out and the optimiser folds that of the first instance.
    #[test]
    fn a_key_without_a_folded_value() {
        mutant("sk_m_key", "key");
    }

    /// The stored copy of a run without holes is found by a run with them.
    #[test]
    fn a_stored_copy_without_its_holes() {
        let src = SRC.replace("K => 3, R => 0", "K => 0, R => 0");
        let w = World::with("sk_m_blob", &src);
        let good = w.run(&[OFF, NO_CHECK], &[]);
        for _ in 0..2 {
            assert!(w.run(&[OFF, NO_CHECK], &[]).ok);
        }
        let d = ("YAVS_SHARE_KEY_DROP", "blob");
        caught(&w.run(&[VALUE, CHECK, d], &[]), "blob");
        let bad = w.run(&[VALUE, NO_CHECK, d], &[]);
        assert_ne!(
            good.out, bad.out,
            "blob: without the check the output is the same"
        );
    }
}
