//! Copies of subprograms keyed by the descriptors of their arguments: results, error texts and the
//! stored form must not know they exist. A debug build also compares what every call passes with
//! what its copy was made for (`Machine::check_sub_args`).
#![cfg(feature = "clift")]

mod common;
use common::tempdir;

use std::path::Path;
use std::process::Command;

const DESIGN: &str = r#"
package p is
  type bvs is array (natural range <>) of bit_vector;
  type rec is record
    tag : bit_vector;
    row : bvs;
  end record;
  type recs is array (natural range <>) of rec;
  function inner(x : recs; i : natural) return natural;
  function outer(x : recs; i : natural) return natural;
  function recur(v : bit_vector) return natural;
  function pure_sum(v : bit_vector) return natural;
  procedure poke(x : inout bit_vector; i : natural);
  -- Passed as data and bounds (`Param::flat`): copies are keyed on the bounds themselves.
  type nv is array (integer range <>) of bit_vector(3 downto 0);
  function fold(x : nv) return nv;
  function nvsum(x : nv) return natural;
  function pick(x : nv; i : integer) return bit_vector;
  function put(x : nv; i : integer) return nv;
end package;

package body p is
  function inner(x : recs; i : natural) return natural is
    variable n : natural := x'length * 1000 + x(x'left).tag'length * 10;
  begin
    n := n + x(x'left).row'length * 100 + x(x'left).row(0)'length;
    if x(i).tag(x(i).tag'right) = '1' then
      n := n + 1;
    end if;
    return n;
  end function;
  -- Hands its own descriptor on: a call that a copy of a copy makes.
  function outer(x : recs; i : natural) return natural is
  begin
    return inner(x, i) + x'length;
  end function;
  -- Another length at every level: more copies than the limit of one function.
  function recur(v : bit_vector) return natural is
  begin
    if v'length <= 1 then
      return 1;
    end if;
    return v'length + recur(v(v'left to v'right - 1));
  end function;
  function pure_sum(v : bit_vector) return natural is
    variable n : natural := 0;
  begin
    for i in v'range loop
      if v(i) = '1' then
        n := n + i;
      end if;
    end loop;
    return n;
  end function;
  procedure poke(x : inout bit_vector; i : natural) is
  begin
    x(i) := not x(i);
  end procedure;
  -- Pairs in the argument's own direction, the result sized from its length.
  function fold(x : nv) return nv is
    variable r : nv(0 to (x'length + 1) / 2 - 1);
    variable j : natural := 0;
  begin
    for i in x'range loop
      r(j / 2) := r(j / 2) xor x(i);
      j := j + 1;
    end loop;
    return r;
  end function;
  function nvsum(x : nv) return natural is
    variable n : natural := x'length * 7 + (x'left mod 5) * 3;
  begin
    for i in x'range loop
      for b in 0 to 3 loop
        if x(i)(b) = '1' then
          n := n + abs (i) * 4 + b + 1;
        end if;
      end loop;
    end loop;
    return n;
  end function;
  -- An index out of the argument's range (`Pos` of the bounds a copy knows).
  function pick(x : nv; i : integer) return bit_vector is
  begin
    return x(i);
  end function;
  -- An index out of the range of a local sized from the argument.
  function put(x : nv; i : integer) return nv is
    variable r : nv(0 to x'length - 1) := x;
  begin
    r(i) := not r(i);
    return r;
  end function;
end package body;

use work.p.all;

entity sub is
  generic (w : natural := 5; l : natural := 2; bad : boolean := false; badf : natural := 0);
  port (clk : in bit);
end entity;

architecture a of sub is
  constant k : recs(0 to l - 1)(tag(w - 1 downto 0), row(0 to 1)(w downto 0)) :=
    (others => (tag => (others => '1'), row => (others => (others => '0'))));
  constant s : bit_vector(0 to w) := (others => '1');
  constant fd : nv(w downto -1) := (others => "1010");
  constant fa : nv(-2 to w + 1) := (others => "0110");
  signal sq : natural := 0;
begin
  process (clk)
    variable v : bit_vector(w - 1 downto 0) := (others => '0');
    variable n : natural := 0;
    variable sum : natural := 0;
    -- Declared in the process: copies keep their parent; a call of a sibling, a drive of the
    -- process's signal, and one that reads the process's variable (no copy).
    function nsum(x : nv) return natural is
      variable k : natural := x'length;
    begin
      for i in x'range loop
        if x(i)(i mod 4) = '1' then
          k := k + 1;
        end if;
      end loop;
      return k;
    end function;
    function nfold(x : nv) return natural is
    begin
      return nsum(fold(x)) + nsum(x) * 2;
    end function;
    impure function nup(x : nv) return natural is
    begin
      return x'length + n mod 7;
    end function;
    procedure ndrive(x : nv) is
    begin
      sq <= nsum(x) + x'left;
    end procedure;
  begin
    if clk = '1' then
      n := n + 1;
      poke(v, n mod w);
      sum := (sum * 3 + outer(k, n mod l) + inner(k, 0) + recur(s) + pure_sum(s) + pure_sum(v)) mod 1000003;
      sum := (sum + nvsum(fold(fd)) + nvsum(fold(fold(fa))) + nvsum(fa(n mod 2 to w))) mod 1000003;
      sum := (sum + nfold(fd) + nfold(fa) + nup(fa) + sq) mod 1000003;
      ndrive(fd);
      if n = 8 then
        case badf is
          when 1 => v(0) := pick(fd, w + 1 + n - 8)(0);
          when 2 => v(0) := pick(fa, n - 11)(0);
          when 3 => v(0) := put(fa, w + 4 + n - 8)(0)(0);
          when 4 => v(0) := put(fd, n - 9)(0)(0);
          when others => null;
        end case;
      end if;
      if bad and n = 8 then
        poke(v, w + 3);
      end if;
      if n mod 16 = 0 then
        report integer'image(w) & " " & integer'image(l) & " " & integer'image(sum);
      end if;
    end if;
  end process;
end architecture;

entity top is
  generic (wa : natural := 5; wb : natural := 3; bad : boolean := false; badf : natural := 0);
end entity;

architecture a of top is
  signal clk : bit := '0';
begin
  ua : entity work.sub generic map (wa, 2, bad, badf) port map (clk);
  ub : entity work.sub generic map (wb, 3, false) port map (clk);
  -- The same descriptors in two instances, and ones a generate fixes.
  uc : entity work.sub generic map (wa, 2, false) port map (clk);
  g : for i in 1 to 3 generate
    ug : entity work.sub generic map (i + 1, i, false) port map (clk);
  end generate;
  clk <= not clk after 5 ns;
  process
  begin
    wait for 1 us;
    report "end " & integer'image(wa) & " " & integer'image(wb);
    std.env.finish;
  end process;
end architecture;
"#;

fn yavs(dir: &Path, env: &[(&str, &str)], args: &[&str]) -> (i32, String, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .current_dir(dir)
        .envs(env.iter().copied())
        .args(["--color", "never"])
        .args(args)
        .output()
        .expect("run yavs");
    (
        o.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

const SPEC: (&str, &str) = ("YAVS_SPEC_AFTER", "2");
const EAGER: [&str; 2] = ["--tier=auto", "--tier-heat=1,0,0"];
const NO_SUBS: (&str, &str) = ("YAVS_NO_SUBCOPY", "1");

fn prepared(name: &str) -> (common::Tmp, String) {
    let d = tempdir(name);
    std::fs::write(d.join("t.vhd"), DESIGN).expect("write");
    let work = format!("work:{}", d.join("lib").display());
    let a = yavs(
        &d,
        &[],
        &["analyze", "--std=2008", "--work", &work, "t.vhd"],
    );
    assert_eq!(a.0, 0, "{}", a.2);
    (d, work)
}

fn run(
    d: &Path,
    work: &str,
    env: &[(&str, &str)],
    tier: &[&str],
    extra: &[&str],
    gens: &[&str],
) -> (i32, String, String) {
    let mut args = vec!["run", "--std=2008", "--work", work];
    args.extend(tier);
    args.extend(extra);
    args.extend(["--threads", "1", "top"]);
    args.extend(gens);
    yavs(d, env, &args)
}

fn counter(stderr: &str, name: &str) -> u64 {
    let p = format!("counters: {name}=");
    stderr
        .lines()
        .find_map(|l| l.strip_prefix(p.as_str()))
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| panic!("no counter {name} in: {stderr}"))
}

/// The tiers give what the interpreter gives without copies, at the switch and with no copy at all,
/// and copies are made and run on the way.
#[test]
fn every_tier_gives_the_results_without_copies() {
    let (d, work) = prepared("sub_copies_tiers");
    let want = run(
        &d,
        &work,
        &[NO_SUBS],
        &["--tier=interp"],
        &["--no-cache"],
        &[],
    );
    assert_eq!(want.0, 0, "{}", want.2);
    assert!(want.2.contains("end 5 3") || want.1.contains("end 5 3"));
    for (env, tier) in [
        (vec![SPEC], vec!["--tier=interp"]),
        (vec![], vec!["--tier=clift"]),
        (vec![], vec!["--tier=auto"]),
        (vec![SPEC], EAGER.to_vec()),
    ] {
        let got = run(&d, &work, &env, &tier, &["--no-cache"], &[]);
        assert_eq!(
            (&want.0, &want.1, &want.2),
            (&got.0, &got.1, &got.2),
            "{tier:?}"
        );
    }
    let mut env = vec![SPEC, ("YAVS_COUNTERS", "1")];
    let (_, _, e) = run(&d, &work, &env, &["--tier=interp"], &["--no-cache"], &[]);
    assert!(counter(&e, "spec_subs") > 0, "no copy of a subprogram: {e}");
    // Recursion with a new length per level stops at the limit of one function, and still runs.
    env.push(("YAVS_SUB_PER", "1"));
    let (_, _, e) = run(&d, &work, &env, &["--tier=interp"], &["--no-cache"], &[]);
    assert!(counter(&e, "spec_subs") > 0);
    let few = run(
        &d,
        &work,
        &[SPEC, ("YAVS_SUB_PER", "1")],
        &EAGER,
        &["--no-cache"],
        &[],
    );
    assert_eq!((&want.0, &want.1, &want.2), (&few.0, &few.1, &few.2));
    // No slot left: the calls stay calls of the subprogram.
    let none = run(
        &d,
        &work,
        &[SPEC, ("YAVS_SUB_SLOTS", "0")],
        &EAGER,
        &["--no-cache"],
        &[],
    );
    assert_eq!((&want.0, &want.1, &want.2), (&none.0, &none.1, &none.2));
}

/// A stored copy names a copy of a subprogram by what it is for: a warm run with other generics is
/// a cold run with them, and the first generics still get their own.
#[test]
fn a_warm_run_with_other_generics_is_a_cold_run_with_them() {
    let (d, work) = prepared("sub_copies_warm");
    let other = ["-gwa=7", "-gwb=4"];
    let expected = run(
        &d,
        &work,
        &[NO_SUBS],
        &["--tier=interp"],
        &["--no-cache"],
        &other,
    );
    assert_eq!(expected.0, 0, "{}", expected.2);
    let first = run(&d, &work, &[SPEC], &EAGER, &[], &[]);
    assert_eq!(first.0, 0, "{}", first.2);
    let warm = run(&d, &work, &[SPEC], &EAGER, &[], &other);
    assert_eq!(
        (&expected.0, &expected.1, &expected.2),
        (&warm.0, &warm.1, &warm.2)
    );
    let base = run(
        &d,
        &work,
        &[NO_SUBS],
        &["--tier=interp"],
        &["--no-cache"],
        &[],
    );
    let again = run(&d, &work, &[SPEC], &EAGER, &[], &[]);
    assert_eq!((&base.0, &base.1, &base.2), (&again.0, &again.1, &again.2));
    // The store served that run, copies of subprograms included, under ids of its own.
    let env = [SPEC, ("YAVS_COUNTERS", "1")];
    let (_, _, e) = run(&d, &work, &env, &EAGER, &[], &[]);
    assert!(
        counter(&e, "spec_loaded") > 0,
        "no copy came from the store: {e}"
    );
    assert!(counter(&e, "spec_subs") > 0, "no copy of a subprogram: {e}");
    assert_eq!(
        counter(&e, "spec_fresh"),
        0,
        "the store did not hold them all: {e}"
    );
}

/// An index out of its range in a copy fails with the text and the call path it has without one.
#[test]
fn an_error_in_a_copy_has_its_own_text() {
    let (d, work) = prepared("sub_copies_error");
    let g = ["-gbad=true"];
    let want = run(
        &d,
        &work,
        &[NO_SUBS],
        &["--tier=interp"],
        &["--no-cache"],
        &g,
    );
    assert_ne!(want.0, 0, "the index must fail: {}", want.2);
    assert!(want.2.contains("poke"), "{}", want.2);
    for (env, tier) in [
        (vec![SPEC], vec!["--tier=interp"]),
        (vec![], vec!["--tier=clift"]),
        (vec![SPEC], EAGER.to_vec()),
    ] {
        let got = run(&d, &work, &env, &tier, &["--no-cache"], &g);
        assert_eq!(
            (&want.0, &want.1, &want.2),
            (&got.0, &got.1, &got.2),
            "{tier:?}"
        );
    }
}

/// An index out of the bounds a copy keyed on flat bounds knows, ascending and descending, of
/// the parameter and of a local sized from it: the text and the call path of the plain call.
#[test]
fn an_index_error_in_a_copy_for_bounds_has_its_own_text() {
    let (d, work) = prepared("sub_copies_flat_error");
    for badf in ["1", "2", "3", "4"] {
        let g = [format!("-gbadf={badf}")];
        let g: Vec<&str> = g.iter().map(String::as_str).collect();
        let want = run(
            &d,
            &work,
            &[NO_SUBS],
            &["--tier=interp"],
            &["--no-cache"],
            &g,
        );
        assert_ne!(want.0, 0, "badf {badf}: the index must fail: {}", want.2);
        assert!(want.2.contains("index"), "badf {badf}: {}", want.2);
        for (env, tier) in [
            (vec![SPEC], vec!["--tier=interp"]),
            (vec![], vec!["--tier=clift"]),
            (vec![SPEC], EAGER.to_vec()),
        ] {
            let got = run(&d, &work, &env, &tier, &["--no-cache"], &g);
            assert_eq!(
                (&want.0, &want.1, &want.2),
                (&got.0, &got.1, &got.2),
                "badf {badf} {tier:?}"
            );
        }
    }
}

const CHECK: (&str, &str) = ("YAVS_CLIFT_PREKEY_CHECK", "1");

/// The corpus test `subcopy_shapes` analysed into a fresh library, and its interpreted output
/// without copies.
fn shapes(tag: &str) -> (common::Tmp, String, (i32, String, String)) {
    let d = tempdir(tag);
    let src = include_str!("../../../tests/corpus/composite/subcopy_shapes.vhd");
    std::fs::write(d.join("t.vhd"), src).expect("write");
    let work = format!("work:{}", d.join("lib").display());
    let a = yavs(
        &d,
        &[],
        &["analyze", "--std=2008", "--work", &work, "t.vhd"],
    );
    assert_eq!(a.0, 0, "{}", a.2);
    let want = yavs(
        &d,
        &[NO_SUBS],
        &[
            "run",
            "--std=2008",
            "--work",
            &work,
            "--tier=interp",
            "--no-cache",
            "--threads",
            "1",
            "subcopy_shapes",
        ],
    );
    assert_eq!(want.0, 0, "{}", want.2);
    (d, work, want)
}

/// A compiled run of `subcopy_shapes` with copies of everything, the counters on.
fn shapes_run(d: &Path, work: &str, threads: &str, env: &[(&str, &str)]) -> (i32, String, String) {
    let mut e = vec![
        SPEC,
        ("YAVS_SPEC_ALL", "1"),
        ("YAVS_COUNTERS", "1"),
        ("YAVS_LOG", "debug"),
        // (Debug builds check by default, and the check lowers anyway: `CHECK` turns it on.)
        ("YAVS_CLIFT_PREKEY_CHECK", "0"),
    ];
    e.extend(env);
    yavs(
        d,
        &e,
        &[
            "run",
            "--std=2008",
            "--work",
            work,
            "--tier=clift-auto",
            "--tier-heat=1,0,0",
            "--threads",
            threads,
            "subcopy_shapes",
        ],
    )
}

fn no_counters(s: &str) -> String {
    s.lines()
        .filter(|l| !l.starts_with("counters:") && !l.starts_with("debug:"))
        .collect::<Vec<_>>()
        .join("\n")
}

/// The functions a run lowered for Cranelift, copies of processes and their callers included.
fn lowered(stderr: &str) -> usize {
    stderr
        .lines()
        .filter(|l| l.starts_with("debug: cache: clif lowered"))
        .count()
}

/// Stored code is not linked for a copy that differs from the one it was made for: one copy per
/// subprogram in the third run moves the ids of the later copies and leaves some calls to the
/// subprograms (before the id symbols, the pre-key check aborted this run).
#[test]
fn compiled_copies_of_subprograms_are_not_linked_under_another_id() {
    let (d, work, want) = shapes("sub_copies_ids");
    let one = [CHECK, ("YAVS_SUB_PER", "1")];
    for env in [&[CHECK][..], &[CHECK], &one, &one, &[CHECK]] {
        let got = shapes_run(&d, &work, "1", env);
        assert_eq!(got.0, want.0, "{env:?}: {}", got.2);
        assert_eq!(got.1, want.1, "{env:?}");
        assert_eq!(no_counters(&got.2), want.2, "{env:?}");
    }
}

/// Code of copies of subprograms names their ids through symbols the linker fills per run, so a
/// warm run whose copies have other ids links all of it, lowers none and prints the same; the
/// pre-key check agrees at one and four threads.
#[test]
fn warm_runs_with_other_copy_ids_link_the_same_code() {
    let (d, work, want) = shapes("sub_copies_moved");
    for _ in 0..2 {
        let r = shapes_run(&d, &work, "1", &[]);
        assert_eq!(r.0, 0, "{}", r.2);
    }
    // (A compile the end of a run cut off comes in the next one.)
    let settled = (0..8)
        .map(|_| shapes_run(&d, &work, "1", &[]))
        .find(|r| lowered(&r.2) == 0)
        .expect("every run lowered something");
    let linked = counter(&settled.2, "clift_sub_linked");
    assert!(linked > 0, "{}", settled.2);
    assert_eq!(counter(&settled.2, "clift_sub_lowered"), 0, "{}", settled.2);
    for skip in ["1", "4"] {
        for threads in ["1", "4"] {
            let moved = [("YAVS_COPY_SKIP", skip)];
            let r = shapes_run(&d, &work, threads, &moved);
            assert_eq!((r.0, &r.1), (want.0, &want.1), "skip {skip}: {}", r.2);
            assert_eq!(no_counters(&r.2), want.2, "skip {skip}");
            assert_eq!(
                counter(&r.2, "clift_sub_lowered"),
                0,
                "skip {skip}: {}",
                r.2
            );
            // Nothing at all is lowered again: no caller names a copy by its id either.
            if threads == "1" {
                assert_eq!(lowered(&r.2), 0, "skip {skip}: {}", r.2);
            }
            // (The empty slots count among the copies: every id moved.)
            let k: u64 = skip.parse().expect("skip");
            let copies = counter(&settled.2, "spec_copies") + k;
            assert_eq!(counter(&r.2, "spec_copies"), copies, "skip {skip}");
            // (Four threads compile the same functions, in another order.)
            assert_eq!(
                counter(&r.2, "clift_sub_linked"),
                linked,
                "skip {skip}: {}",
                r.2
            );
            let c = shapes_run(&d, &work, threads, &[moved[0], CHECK]);
            assert_eq!(
                (c.0, &c.1),
                (want.0, &want.1),
                "checked skip {skip}: {}",
                c.2
            );
            assert!(
                counter(&c.2, "clift_sub_linked") > 0,
                "checked skip {skip}: {}",
                c.2
            );
        }
    }
}
