//! A region an address parameter of a Cranelift native frame points into is held over calls
//! (`Gen::hold_resolve`, `hold_recheck`): a native caller's slots, a process's composite variable, a
//! heap object; the temporary area and the interpreter's other frames, which move, are not. The
//! callee here calls between its accesses: functions that allocate, a protected object, one that
//! frees the very object the parameter points into. Every tier gives the same output, debug builds
//! hold every access against the lookup (`yavs_rt_hold_check`), and with the recheck left out
//! (`YAVS_CLIFT_HOLD_DROP`, the mutation test) that check catches the stale pointer. A temporary that
//! outgrows the area goes through a materialised frame or moves it under the frame; held or not, it
//! must read the same.
#![cfg(feature = "clift")]

mod common;
use common::tempdir;

use std::path::Path;
use std::process::{Command, Output};

const SRC: &str = r#"use std.textio.all;
entity hold is
  generic (bad : integer := 0);
end;
architecture a of hold is
  type ints_t is array (natural range <>) of integer;
  type ints_ptr_t is access ints_t;
  type rec_t is record
    a, b, c, d : integer;
    p : ints_ptr_t;
  end record;

  type prot_t is protected
    procedure set (i, v : integer);
    impure function get (i : integer) return integer;
  end protected;
  type prot_t is protected body
    type table_t is array (natural range <>) of ints_ptr_t;
    type table_ptr_t is access table_t;
    variable t : table_ptr_t := new table_t'(0 to 3 => null);
    procedure set (i, v : integer) is
      variable old : ints_ptr_t;
    begin
      -- Replaces the object at every call: the heaps change under the caller.
      old := t(i mod 4);
      t(i mod 4) := new ints_t'(0 to 3 => v);
      if old /= null then deallocate(old); end if;
    end procedure;
    impure function get (i : integer) return integer is
    begin
      if t(i mod 4) = null then return -1; end if;
      return t(i mod 4)(1);
    end function;
  end protected body;
  shared variable st : prot_t;

  function pad (s : string; n : natural) return string is
    variable r : string(1 to s'length + n) := (others => '!');
  begin
    r(1 to s'length) := s;
    return r;
  end function;

  -- Between the accesses: the temporary area grows, by far on some rounds.
  procedure ints_calls (x : ints_t; n : natural; variable sum : inout integer) is
  begin
    sum := sum + x(x'left);
    sum := sum + pad("ab", n)'length;
    sum := sum + x(x'left + 1);
    sum := sum + pad("cd", n)'length;
    sum := sum + x(x'left + 2);
  end procedure;

  function twice (s : string) return string is
  begin
    return s & s;
  end function;

  function ints2 (x : ints_t) return ints_t is
  begin
    return x & x;
  end function;

  -- Between the accesses: a concatenation the runtime makes, beyond what the temporary area has.
  procedure ints_cat (x : ints_t; big : string; variable sum : inout integer) is
  begin
    sum := sum + x(x'left);
    sum := sum + twice(big)'length;
    sum := sum + x(x'left + 1);
    sum := sum + twice(big)'length;
    sum := sum + x(x'left + 2);
    sum := sum mod 1000;
  end procedure;

  -- Between the accesses: a protected call that frees and allocates in the shared heap.
  procedure ints_prot (x : ints_t; variable sum : inout integer) is
  begin
    sum := sum + x(x'left);
    st.set(sum, x(x'left + 1));
    sum := sum + st.get(sum);
    sum := sum + x(x'left + 2);
    st.set(sum, x(x'left));
  end procedure;

  -- A record by reference, written and read over calls.
  procedure rec_calls (variable r : inout rec_t; variable sum : inout integer) is
  begin
    r.a := r.a + 1;
    st.set(r.a, r.b);
    r.b := r.b + st.get(r.a);
    sum := sum + r.a + pad("x", 3)'length;
    r.c := r.c + r.a;
    st.set(r.c, r.c);
    r.d := r.d + r.b + r.c;
    sum := sum + r.d;
  end procedure;

  procedure grow (variable p : inout ints_ptr_t) is
    variable q : ints_ptr_t := new ints_t'(0 to p'length => 7);
  begin
    q(p'range) := p.all;
    deallocate(p);
    p := q;
  end procedure;

  -- `x` is `p.all` at a call with `go` true: the object goes away under the parameter.
  procedure reader (x : ints_t; variable p : inout ints_ptr_t; go : boolean;
                    variable sum : inout integer) is
  begin
    sum := sum + x(x'left);
    sum := sum + x(x'left + 1);
    if go then grow(p); end if;
    sum := sum + x(x'left + 2);
    sum := sum + x(x'left + 3);
  end procedure;

  -- The same with the free in the function itself: a step on the runtime's own frame.
  procedure reader_free (x : ints_t; variable p : inout ints_ptr_t; variable sum : inout integer) is
  begin
    sum := sum + x(x'left);
    sum := sum + x(x'left + 1);
    deallocate(p);
    sum := sum + x(x'left + 2);
    sum := sum + x(x'left + 3);
  end procedure;

  -- A bad index and a bad slice of array parameters (bounds in registers), after a call whose flat
  -- result comes back through a helper when the callee has no code yet.
  procedure reader_index (x : ints_t; n : natural; variable sum : inout integer) is
  begin
    sum := sum + x(x'left);
    sum := sum + twice("ab")'length;
    sum := sum + ints2(x)(x'left + 1);
    sum := sum + x(x'right + n);
  end procedure;

  procedure reader_slice (x : ints_t; n : natural; variable sum : inout integer) is
    constant y : ints_t := ints2(x);
  begin
    sum := sum + x(x'left);
    sum := sum + y(y'left to y'right + n)'length;
    sum := sum + x(x'left + 1);
  end procedure;

  -- The arguments are this subprogram's own locals: a native caller's frame.
  procedure from_frame (variable total : inout integer) is
    variable res : ints_t(0 to 6) := (1, 2, 3, 4, 5, 6, 7);
    variable sum : integer := 0;
    variable r   : rec_t := (1, 2, 3, 4, null);
  begin
    ints_calls(res, 2, sum);
    ints_prot(res, sum);
    rec_calls(r, sum);
    total := total + sum + r.d;
  end procedure;
begin
  process
    variable total : integer := 0;
    variable sum   : integer := 0;
    variable v     : ints_t(1 to 9) := (others => 3);
    variable r     : rec_t := (5, 6, 7, 8, null);
    variable heap  : ints_ptr_t := new ints_t'(0 to 4 => 11);
    variable other : ints_ptr_t := new ints_t'(0 to 4 => 13);
    variable big   : line := new string'(1 to 16000000 => 'x');
  begin
    -- The temporary area is small yet: the concatenation moves it under `x`.
    ints_cat(ints2(v), big.all, sum);
    for round in 1 to 40 loop
      from_frame(total);
      -- a process's composite variables
      ints_calls(v, 2, sum);
      ints_prot(v, sum);
      rec_calls(r, sum);
      -- the temporary area, moved by a large temporary now and then
      ints_calls(ints2(v), 2 + (round mod 8) * 200000, sum);
      if round mod 8 = 0 then
        ints_cat(ints2(v), big.all(1 to round * 400000), sum);
      else
        ints_cat(ints2(v), "abc", sum);
      end if;
      ints_prot(ints2(v), sum);
      -- a heap object
      ints_calls(heap.all, 2, sum);
      ints_prot(heap.all, sum);
      reader(heap.all, other, true, sum);
      total := (total + sum + r.d + heap(2)) mod 1000000;
      sum := sum mod 1000000;
      r.a := r.a mod 1000;
      r.b := r.b mod 1000;
      r.c := r.c mod 1000;
      r.d := r.d mod 1000;
      -- (An activation per round: what the count rule counts.)
      wait for 1 ns;
    end loop;
    report "total " & integer'image(total) & " sum " & integer'image(sum);
    if bad = 1 then
      reader(heap.all, heap, true, sum);
      report "sum " & integer'image(sum);
    end if;
    if bad = 2 then
      reader_free(heap.all, heap, sum);
      report "sum " & integer'image(sum);
    end if;
    if bad = 3 then
      reader_index(heap.all, 1, sum);
    end if;
    if bad = 4 then
      reader_slice(v, 1, sum);
    end if;
    report "end";
    wait;
  end process;
end;
"#;

fn yavs(dir: &Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_yavs"));
    c.args(args).current_dir(dir);
    for (k, v) in env {
        c.env(k, v);
    }
    c.output().expect("run yavs")
}

fn run(
    dir: &Path,
    flags: &[&str],
    bad: u32,
    env: &[(&str, &str)],
) -> (String, String, Option<i32>) {
    let g = format!("-gbad={bad}");
    let mut args = vec!["run", "--std=2008", "--work=work:work"];
    args.extend_from_slice(flags);
    args.extend(["hold", &g]);
    let o = yavs(dir, &args, env);
    let err = String::from_utf8_lossy(&o.stderr);
    (
        String::from_utf8_lossy(&o.stdout).into_owned(),
        err.lines()
            .filter(|l| !l.starts_with("counters: "))
            .collect::<Vec<_>>()
            .join("\n"),
        o.status.code(),
    )
}

fn analyzed(name: &str) -> common::Tmp {
    let dir = tempdir(name);
    std::fs::write(dir.join("hold.vhd"), SRC).expect("write");
    let a = yavs(
        &dir,
        &["analyze", "--std=2008", "--work=work:work", "hold.vhd"],
        &[],
    );
    assert!(a.status.success(), "{a:?}");
    dir
}

#[test]
fn a_held_region_matches_the_interpreter() {
    let dir = analyzed("clift_hold");
    for bad in 0..=2 {
        let want = run(&dir, &["--tier=interp"], bad, &[]);
        assert!(want.0.contains("total "), "{want:?}");
        assert_eq!(want.2 == Some(0), bad == 0, "bad {bad}: {want:?}");
        for flags in [
            vec!["--tier=clift"],
            vec!["--tier=auto", "--tier-heat=1,0,0"],
            vec!["--tier=auto", "--tier-heat=1,0,0", "--threads=4"],
        ] {
            let got = run(&dir, &flags, bad, &[]);
            assert_eq!(got, want, "bad {bad} {flags:?}");
        }
    }
}

#[test]
fn accesses_through_held_regions_are_checked_in_debug_builds() {
    if !cfg!(debug_assertions) {
        return;
    }
    let dir = analyzed("clift_hold_count");
    let o = yavs(
        &dir,
        &[
            "run",
            "--std=2008",
            "--work=work:work",
            "--tier=clift",
            "hold",
        ],
        &[("YAVS_COUNTERS", "1")],
    );
    let err = String::from_utf8_lossy(&o.stderr);
    let n: u64 = err
        .lines()
        .find_map(|l| l.strip_prefix("counters: hold_checked="))
        .and_then(|n| n.trim().parse().ok())
        .unwrap_or(0);
    assert!(n > 1000, "held accesses checked: {n}");
}

/// The ways a native frame runs an instruction or a call through a helper and comes back (no resume
/// anywhere): a caller compiled before its callees (`--tier-heat=1,0,0`), calls through the table,
/// and the count rule at its tightest. The bad cases free an object under a parameter in the nested
/// interpreter's loop and fail through a parameter's bounds, with the trace of the native frames.
type Way = (
    &'static [&'static str],
    &'static [(&'static str, &'static str)],
);
const HELPER_WAYS: &[Way] = &[
    (&["--tier=clift-auto", "--tier-heat=1,0,0"], &[]),
    (
        &["--tier=clift-auto", "--tier-heat=1,0,0", "--threads=4"],
        &[],
    ),
    (&["--tier=clift"], &[("YAVS_CLIFT_NTAB", "1")]),
    (
        &["--tier=auto"],
        &[
            ("YAVS_AUTO_COUNT", "1"),
            ("YAVS_AUTO_GATE", "0"),
            ("YAVS_AUTO_SYNC", "1"),
            ("YAVS_SPEC_ALL", "1"),
        ],
    ),
    (
        &["--tier=auto", "--tier-heat=1,0,0"],
        &[("YAVS_AUTO_POLICY", "sampled")],
    ),
];

#[test]
fn held_regions_and_bounds_over_helpers_match_the_interpreter() {
    let dir = analyzed("clift_hold_helpers");
    for bad in 0..=4 {
        let want = run(&dir, &["--tier=interp"], bad, &[]);
        assert!(want.0.contains("total "), "{want:?}");
        assert_eq!(want.2 == Some(0), bad == 0, "bad {bad}: {want:?}");
        for (flags, env) in HELPER_WAYS {
            let mut env = env.to_vec();
            env.push(("YAVS_CLIFT_HOLD_CHECK", "1"));
            let got = run(&dir, flags, bad, &env);
            assert_eq!(got, want, "bad {bad} {flags:?} {env:?}");
        }
    }
    // Each way ran compiled code with held regions, through the helpers.
    for (flags, env) in HELPER_WAYS {
        let mut env = env.to_vec();
        env.push(("YAVS_CLIFT_HOLD_CHECK", "1"));
        env.push(("YAVS_COUNTERS", "1"));
        let mut args = vec!["run", "--std=2008", "--work=work:work", "--no-cache"];
        args.extend_from_slice(flags);
        args.push("hold");
        let o = yavs(&dir, &args, &env);
        let e = String::from_utf8_lossy(&o.stderr);
        let at = |name: &str| -> u64 {
            e.lines()
                .find_map(|l| l.strip_prefix(&format!("counters: {name}=")))
                .and_then(|n| n.trim().parse().ok())
                .unwrap_or(0)
        };
        assert!(at("hold_checked") > 1000, "{flags:?} {env:?}: {e}");
        if cfg!(debug_assertions) {
            assert!(at("cb_xstep") > 100, "{flags:?} {env:?}: {e}");
        }
        if env.iter().any(|(k, _)| *k == "YAVS_AUTO_COUNT") {
            assert!(at("count_compiled") >= 1, "{env:?}: {e}");
        }
    }
}

/// The nested loop frees under a held parameter and the frame goes on: only `heap_gen`, refreshed
/// by the helper, says so. Without the recheck the check catches it.
#[test]
fn a_hold_over_a_helper_not_asked_again_is_caught() {
    if !cfg!(debug_assertions) || cfg!(feature = "gc-stress") {
        return;
    }
    let dir = analyzed("clift_hold_helper_mutant");
    let got = run(
        &dir,
        &["--tier=clift-auto", "--tier-heat=1,0,0"],
        1,
        &[("YAVS_CLIFT_HOLD_DROP", "1")],
    );
    assert!(got.1.contains("is stale"), "{got:?}");
}

/// The mutation test: without the recheck the object freed under a parameter is read through the
/// stale pointer, and the check says so.
#[test]
fn a_hold_not_asked_again_is_caught() {
    // (Under `gc-stress` the frame materialises at the free: no hold to catch.)
    if !cfg!(debug_assertions) || cfg!(feature = "gc-stress") {
        return;
    }
    let dir = analyzed("clift_hold_mutant");
    for bad in [1, 2] {
        let got = run(
            &dir,
            &["--tier=clift"],
            bad,
            &[("YAVS_CLIFT_HOLD_DROP", "1")],
        );
        assert!(got.1.contains("is stale"), "bad {bad}: {got:?}");
    }
}
