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

const BELOW: &str = r#"entity below is end;
architecture a of below is
  -- Nothing here moves a frame: a stable activation holds what its parameter points into.
  procedure leaf (variable i : inout integer; k : integer) is
  begin
    for t in 1 to 3 loop
      if i mod 2 = 0 then i := (i * 7 + k) mod 10007; else i := (i + t) mod 10007; end if;
      if i mod 3 = 0 then i := (i * 3 + k) mod 10009; else i := (i + t) mod 10009; end if;
      if i mod 4 = 0 then i := (i * 5 + k) mod 10037; else i := (i + t) mod 10037; end if;
      if i mod 5 = 0 then i := (i * 11 + k) mod 10039; else i := (i + t) mod 10039; end if;
      if i mod 6 = 0 then i := (i * 13 + k) mod 10061; else i := (i + t) mod 10061; end if;
      if i mod 7 = 0 then i := (i * 17 + k) mod 10067; else i := (i + t) mod 10067; end if;
      if i mod 8 = 0 then i := (i * 19 + k) mod 10069; else i := (i + t) mod 10069; end if;
      if i mod 9 = 0 then i := (i * 23 + k) mod 10079; else i := (i + t) mod 10079; end if;
      if i mod 10 = 0 then i := (i * 29 + k) mod 10091; else i := (i + t) mod 10091; end if;
      if i mod 11 = 0 then i := (i * 31 + k) mod 10093; else i := (i + t) mod 10093; end if;
      if i mod 12 = 0 then i := (i * 37 + k) mod 10099; else i := (i + t) mod 10099; end if;
      if i mod 13 = 0 then i := (i * 41 + k) mod 10103; else i := (i + t) mod 10103; end if;
      if i mod 14 = 0 then i := (i * 43 + k) mod 10111; else i := (i + t) mod 10111; end if;
      if i mod 15 = 0 then i := (i * 47 + k) mod 10133; else i := (i + t) mod 10133; end if;
      if i mod 16 = 0 then i := (i * 53 + k) mod 10139; else i := (i + t) mod 10139; end if;
      if i mod 17 = 0 then i := (i * 59 + k) mod 10141; else i := (i + t) mod 10141; end if;
    end loop;
  end procedure;
  -- Waits, so these two run on interpreter frames: `i` points below the top frame.
  procedure middle (variable i : inout integer; k : integer) is
  begin
    wait for 1 ns;
    leaf(i, k);
  end procedure;
  procedure outer (variable s : inout integer; k : integer) is
    variable v : integer := k;
  begin
    wait for 1 ns;
    middle(v, k + 1);
    s := (s + v) mod 100003;
  end procedure;
begin
  process
    variable s : integer := 0;
  begin
    for i in 1 to 20 loop
      outer(s, i);
    end loop;
    report "sum " & integer'image(s);
    wait;
  end process;
end;
"#;

/// An address parameter of a stable activation may point into an interpreter frame below the top
/// one, which the table finds and nothing moves. The check holds it against the same lookup (it
/// called such a hold stale, on a call chain materialised from native frames).
#[test]
fn a_hold_in_a_frame_below_the_top_is_no_stale_hold() {
    let dir = tempdir("clift_hold_below");
    std::fs::write(dir.join("below.vhd"), BELOW).expect("write");
    let common = ["--std=2008", "--work=work:work"];
    let a = yavs(
        &dir,
        &[&["analyze"], &common[..], &["below.vhd"]].concat(),
        &[],
    );
    assert!(a.status.success(), "{a:?}");
    let go = |tier: &'static str| {
        let o = yavs(
            &dir,
            &[&["run"], &common[..], &["--no-cache", tier, "below"]].concat(),
            &[("YAVS_CLIFT_HOLD_CHECK", "1"), ("YAVS_COUNTERS", "1")],
        );
        let e = String::from_utf8_lossy(&o.stderr).into_owned();
        assert!(o.status.success(), "{tier}: {e}");
        let counted = e
            .lines()
            .find_map(|l| l.strip_prefix("counters: hold_checked="))
            .and_then(|n| n.trim().parse::<u64>().ok())
            .unwrap_or(0);
        let out = String::from_utf8_lossy(&o.stdout);
        let sum = out.lines().find(|l| l.contains("sum ")).map(str::to_string);
        (sum, counted)
    };
    let (want, _) = go("--tier=interp");
    assert!(want.is_some());
    let (got, counted) = go("--tier=clift");
    assert_eq!(got, want);
    assert!(counted > 0, "no held access was checked");
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
