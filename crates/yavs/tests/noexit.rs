//! Compiled code that never hands a running function to the interpreter (decisions.md "No resume
//! anywhere in Cranelift code"): every way an instruction can fail, in a process body (frame-mode
//! code) and in a chain of functions (native frames), gives the interpreter's exit status, output
//! and error text with its call chain. So do the rare cases a native frame calls a helper for and
//! goes on after. Each case runs on the interpreter and on each of `WAYS`. Needs the `clift`
//! feature.
#![cfg(feature = "clift")]

mod common;
use common::tempdir;

use std::process::{Command, Output};

/// A design with one failing statement, in `body` of a process and of the innermost function of
/// three: `{decl}` declares its variables, `{stmt}` runs on `x` and `y` and returns or reports.
fn design(decl: &str, stmt: &str, bad: (&str, &str), native: bool) -> String {
    let (x, y) = bad;
    let inner = format!(
        "  impure function op (x, y : integer) return integer is
    {decl}
  begin
    {stmt}
    return 1;
  end;
  impure function mid (x, y : integer) return integer is
  begin
    return op(x + 0, y) + 1;
  end;
  impure function top (x, y : integer) return integer is
  begin
    return mid(x, y + 0) + 1;
  end;
"
    );
    let run = if native {
        format!(
            "    for i in 1 to 300 loop
      acc := (acc + top(i mod 7 + 1, i mod 5 + 1)) mod 1000;
    end loop;
    report \"before \" & integer'image(acc);
    acc := top({x}, {y});
"
        )
    } else {
        format!(
            "    for i in 1 to 300 loop
      x := i mod 7 + 1;
      y := i mod 5 + 1;
      {stmt}
      acc := (acc + 1) mod 1000;
    end loop;
    report \"before \" & integer'image(acc);
    x := {x};
    y := {y};
    {stmt}
"
        )
    };
    let vars = if native {
        String::new()
    } else {
        format!("    variable x, y : integer;\n    {decl}\n")
    };
    format!(
        "library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use std.env.all;
entity t is end;
architecture a of t is
  type ints is array (natural range <>) of integer;
  type acc_int is access integer;
{inner}
begin
  process
    variable acc : integer := 0;
{vars}  begin
{run}    report \"after \" & integer'image(acc);
    wait;
  end process;
end;
"
    )
}

/// A body in a process has no `return`: a statement is written to work in both.
struct Case {
    name: &'static str,
    decl: &'static str,
    stmt: &'static str,
    bad: (&'static str, &'static str),
}

const CASES: &[Case] = &[
    Case {
        name: "add_range",
        decl: "variable v : integer;",
        stmt: "v := x + y;",
        bad: ("integer'high", "1"),
    },
    Case {
        name: "sub_range",
        decl: "variable v : integer;",
        stmt: "v := x - y;",
        bad: ("integer'low", "1"),
    },
    Case {
        name: "mul_range",
        decl: "variable v : integer;",
        stmt: "v := x * y;",
        bad: ("integer'high", "2"),
    },
    Case {
        name: "time_overflow",
        decl: "variable v : time := time'high - 100 ns;",
        stmt: "v := v + x * 1 ns; assert x < 1000 report \"far\" severity note; v := v + (y * 1 ms);",
        bad: ("5", "2000000"),
    },
    Case {
        name: "div_zero",
        decl: "variable v : integer;",
        stmt: "v := x / y;",
        bad: ("5", "0"),
    },
    Case {
        name: "mod_zero",
        decl: "variable v : integer;",
        stmt: "v := x mod y;",
        bad: ("5", "0"),
    },
    Case {
        name: "rem_zero",
        decl: "variable v : integer;",
        stmt: "v := x rem y;",
        bad: ("5", "0"),
    },
    Case {
        name: "neg_range",
        decl: "variable v : integer;",
        stmt: "v := -x;",
        bad: ("integer'low", "0"),
    },
    Case {
        name: "abs_range",
        decl: "variable v : integer;",
        stmt: "v := abs x;",
        bad: ("integer'low", "0"),
    },
    Case {
        name: "subtype_range",
        decl: "variable v : integer range 0 to 100;",
        stmt: "v := x + y;",
        bad: ("100", "1"),
    },
    Case {
        name: "real_div_zero",
        decl: "variable v : real;",
        stmt: "v := real(x) / real(y);",
        bad: ("5", "0"),
    },
    Case {
        name: "real_overflow",
        decl: "variable v : real := 1.0e308;",
        stmt: "v := 1.0e308 * real(y) * real(x);",
        bad: ("100", "100"),
    },
    Case {
        name: "real_range",
        decl: "variable v : real range 0.0 to 10.0;",
        stmt: "v := real(x) + real(y);",
        bad: ("100", "1"),
    },
    Case {
        name: "index",
        decl: "variable v : ints(0 to 9) := (others => 7); variable w : integer;",
        stmt: "w := v(x);",
        bad: ("10", "0"),
    },
    Case {
        name: "index_write",
        decl: "variable v : ints(0 to 9) := (others => 7);",
        stmt: "v(y) := x;",
        bad: ("1", "11"),
    },
    Case {
        name: "slice",
        decl: "variable v : ints(0 to 9) := (others => 7); variable w : ints(0 to 1);",
        stmt: "w := v(x to x + 1);",
        bad: ("9", "0"),
    },
    Case {
        name: "length",
        decl: "variable v : ints(0 to 2); variable w : ints(0 to 9) := (others => 1);",
        stmt: "if x > 50 then v := w(0 to y); end if;",
        bad: ("51", "4"),
    },
    Case {
        name: "null_access",
        decl: "variable p : acc_int; variable w : integer;",
        stmt: "if x > 50 then w := p.all; end if;",
        bad: ("51", "0"),
    },
    Case {
        name: "freed_access",
        decl: "variable p, q : acc_int; variable w : integer;",
        stmt: "p := new integer'(x); q := p; deallocate(p); if x > 50 then w := q.all; end if;",
        bad: ("51", "0"),
    },
    Case {
        name: "assert_error",
        decl: "",
        stmt: "assert x < 50 report \"too big\" severity error;",
        bad: ("51", "0"),
    },
    Case {
        name: "assert_failure",
        decl: "",
        stmt: "assert x < 50 report \"too big\" severity failure;",
        bad: ("51", "0"),
    },
    Case {
        name: "report_note",
        decl: "",
        stmt: "if x > 50 then report \"seen \" & integer'image(y) severity note; end if;",
        bad: ("51", "0"),
    },
    Case {
        name: "finish",
        decl: "",
        stmt: "if x > 50 then finish; end if;",
        bad: ("51", "0"),
    },
    Case {
        name: "stop",
        decl: "",
        stmt: "if x > 50 then stop; end if;",
        bad: ("51", "0"),
    },
    Case {
        name: "wide_add",
        decl: "variable v : unsigned(64 downto 0);",
        stmt: "v := resize(to_unsigned(x, 64), 65) + resize(to_unsigned(y, 64), 65) + '1'; assert v(0) = '1' or v(0) = '0'; v := v + to_unsigned(1 / (y + 100 - x), 3);",
        bad: ("100", "0"),
    },
    Case {
        name: "power",
        decl: "variable v : integer;",
        stmt: "v := x ** y;",
        bad: ("1000", "5"),
    },
    Case {
        name: "new_and_grow",
        decl: "variable p : acc_int; variable w : integer := 0;",
        stmt: "for k in 1 to 40 loop p := new integer'(k + x); w := w + p.all; deallocate(p); end loop;",
        bad: ("9", "0"),
    },
    Case {
        name: "string_temp",
        decl: "variable l : integer := 0;",
        stmt: "for k in 1 to 3 loop l := l + integer'image(x * k)'length; end loop;",
        bad: ("70000", "0"),
    },
];

/// Recursion deep enough to pass the native stack allowance, and past the depth limit.
const RECURSION: &str = "entity t is generic (N : natural := 9000); end;
architecture a of t is
  function depth (n : integer; pad0, pad1, pad2, pad3 : integer) return integer is
    variable w : integer := pad0 + pad1;
  begin
    if n = 0 then
      return w;
    end if;
    return depth(n - 1, pad1, pad2, pad3, pad0) + 1 + w - w;
  end;
begin
  process
  begin
    for i in 1 to 3 loop
      report integer'image(depth(N, 1, 2, 3, 4));
    end loop;
    wait;
  end process;
end;
";

/// The same past the allowance with what a callee running in the interpreter reaches: an array in
/// a native caller's frame through a parameter, a variable of an enclosing function, a failure at
/// the bottom that names every frame.
const RECURSION_SLOTS: &str = "entity t is generic (N : natural := 9000; BAD : natural := 0); end;
architecture a of t is
  type ints is array (natural range <>) of integer;
  impure function outer (n : integer) return integer is
    variable base : integer := n * 2;
    impure function inner (k : integer; a : ints) return integer is
      variable b : ints(0 to 3) := (k, k + 1, k + 2, a(a'left));
    begin
      if k = BAD then
        return a(k + 100);
      end if;
      if k = 0 then
        return a(a'left) + base;
      end if;
      return inner(k - 1, b) + 1 - b(0) + k;
    end;
  begin
    return inner(n, (1, 2, 3, 4));
  end;
begin
  process
  begin
    for i in 1 to 3 loop
      report integer'image(outer(N));
    end loop;
    wait;
  end process;
end;
";

/// A process that never suspends: the host polls and counts its jumps back while it stays compiled,
/// and at the limit it words the error.
const NO_WAIT: &str = "entity t is end;
architecture a of t is
  signal s : integer := 0;
begin
  process (s)
    variable n, m : integer := 0;
  begin
    loop
      n := (n + 1) mod 1000;
      if n = 0 then
        m := m + 1;
      end if;
    end loop;
  end process;
end;
";

fn yavs(dir: &std::path::Path, args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut c = Command::new(env!("CARGO_BIN_EXE_yavs"));
    c.args(args).current_dir(dir);
    c.env("YAVS_SPIN_LIMIT", "2000000");
    for (k, v) in env {
        c.env(k, v);
    }
    c.output().expect("run yavs")
}

/// The ways to run compiled code: flags and environment.
type Way = (
    &'static [&'static str],
    &'static [(&'static str, &'static str)],
);
const WAYS: &[Way] = &[
    (&["--tier=clift"], &[]),
    (&["--tier=clift"], &[("YAVS_CLIFT_NTAB", "1")]),
    (&["--tier=clift"], &[("YAVS_CLIFT_NATIVE", "0")]),
    (&["--tier=clift"], &[("YAVS_CLIFT_EXITS", "old")]),
    // Compiled as they are first entered: a compiled caller meets callees without code.
    (
        &["--tier=clift-auto", "--tier-heat=1,0,0"],
        &[("YAVS_SPEC_AFTER", "2")],
    ),
];

fn show(o: &Output) -> String {
    format!(
        "{:?}\n{}\n{}",
        o.status.code(),
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

/// Run `src` on the interpreter and on every compiled way, and hold their results equal.
fn same(name: &str, src: &str, gens: &[&str]) {
    let dir = tempdir("noexit");
    std::fs::write(dir.join("t.vhd"), src).expect("write");
    let a = yavs(
        &dir,
        &["analyze", "--std=2008", "--work=work:work", "t.vhd"],
        &[],
    );
    assert!(a.status.success(), "{name}: {a:?}");
    let args = |tier: &[&'static str]| {
        let mut v = vec!["run", "--std=2008", "--work=work:work", "--no-cache"];
        v.extend_from_slice(tier);
        v.extend_from_slice(gens);
        v.push("t");
        v
    };
    let i = yavs(&dir, &args(&["--tier=interp"]), &[]);
    for (flags, env) in WAYS {
        let c = yavs(&dir, &args(flags), env);
        assert_eq!(show(&c), show(&i), "{name} under {flags:?} {env:?}");
    }
}

#[test]
fn every_failure_in_a_process_body() {
    for c in CASES {
        same(c.name, &design(c.decl, c.stmt, c.bad, false), &[]);
    }
}

#[test]
fn every_failure_in_a_chain_of_native_frames() {
    for c in CASES {
        same(c.name, &design(c.decl, c.stmt, c.bad, true), &[]);
    }
}

#[test]
fn recursion_past_the_native_stack_allowance_and_the_depth_limit() {
    same("deep", RECURSION, &[]);
    same("over the limit", RECURSION, &["-gN=20000"]);
    same("slots", RECURSION_SLOTS, &[]);
    same("failing at the bottom", RECURSION_SLOTS, &["-gBAD=5"]);
}

/// A call chain whose innermost callee has no code when the caller runs: the callers stay compiled
/// and nothing comes back to the interpreter's loop (the census of `YAVS_COUNTERS=exits`).
#[test]
fn nothing_leaves_a_running_function() {
    let dir = tempdir("noexit_census");
    let src = design(
        "variable v : ints(0 to 9) := (others => 7); variable p : acc_int;",
        "p := new integer'(v(x)); deallocate(p); assert v(y) = 7 report \"seen\" severity note;",
        ("1", "2"),
        true,
    );
    std::fs::write(dir.join("t.vhd"), src).expect("write");
    let a = yavs(
        &dir,
        &["analyze", "--std=2008", "--work=work:work", "t.vhd"],
        &[],
    );
    assert!(a.status.success(), "{a:?}");
    for (flags, env) in [WAYS[0], WAYS[1], WAYS[4]] {
        let mut args = vec!["run", "--std=2008", "--work=work:work", "--no-cache"];
        args.extend_from_slice(flags);
        args.push("t");
        let mut env = env.to_vec();
        env.push(("YAVS_COUNTERS", "exits"));
        let o = yavs(&dir, &args, &env);
        assert!(o.status.success(), "{o:?}");
        let err = String::from_utf8_lossy(&o.stderr);
        for line in err.lines().filter(|l| l.starts_with("counters: cb_")) {
            assert!(
                !line.starts_with("counters: cb_exit.") && !line.starts_with("counters: cb_deopt."),
                "{line} under {flags:?} {env:?}:\n{err}"
            );
        }
    }
}

#[test]
fn a_process_that_never_suspends_is_polled_and_bounded() {
    same("no wait", NO_WAIT, &[]);
}
