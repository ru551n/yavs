//! Operations of the interpreter's fast loop for instructions that used to be generic steps: the
//! address of a variable of an enclosing subprogram (`AddrOf` up the static chain, in a register
//! that is defined more than once; with its load or store it is `LoadUp` or `StoreUp`) and the
//! comparison of reals (`FCmp`).

mod common;
use common::tempdir as tmp;

use std::process::Command;

const OPS: &str = "
entity ops is end;
architecture a of ops is
begin
  process
    variable acc : integer := 0;
    variable x : real := 0.5;
    procedure bump(n : integer) is
      variable t : integer := 0;
      procedure inner is
      begin
        t := t + n;
        acc := acc + n;
      end procedure;
    begin
      inner;
      inner;
      t := t + acc;
      acc := t mod 1000;
    end procedure;
  begin
    for i in 1 to 4 loop
      bump(i);
      if x < 2.0 then x := x * 2.0; end if;
      if x /= 4.0 and x >= 1.0 then acc := acc + 100; end if;
    end loop;
    report \"acc=\" & integer'image(acc) & \" x=\" & real'image(x);
    wait;
  end process;
end;
";

#[test]
fn static_chain_addresses_and_real_compares_are_ops() {
    let d = tmp("ops");
    std::fs::write(d.join("ops.vhd"), OPS).expect("write");
    let work = format!("work:{}", d.join("lib").display());
    let yavs = env!("CARGO_BIN_EXE_yavs");
    let a = Command::new(yavs)
        .current_dir(&d)
        .args(["analyze", "--work", &work, "ops.vhd"])
        .output()
        .expect("analyze");
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    let o = Command::new(yavs)
        .current_dir(&d)
        .env("YAVS_DUMPIR", "1")
        .args(["--color", "never", "run", "--no-cache", "--work", &work])
        .args(["--tier=interp", "ops"])
        .output()
        .expect("run");
    assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
    let err = String::from_utf8_lossy(&o.stderr);
    // Worked out by hand: acc = 440 and x = 2.
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(
        out.contains("acc=440 x=2") || err.contains("acc=440 x=2"),
        "{out}\n{err}"
    );
    for want in [" LoadUp {", " StoreUp {", " FCmp {"] {
        let in_ops = err.lines().any(|l| l.contains(" pc") && l.contains(want));
        assert!(in_ops, "no op{want}\n{err}");
    }
    let _ = std::fs::remove_dir_all(&d);
}

/// `uniform` of three variables runs in place (`Op::Uniform`); a seed out of range is reported
/// as the native call reports it and leaves the seeds and the real as it does.
const UNIFORM: &str = "
library ieee;
use ieee.math_real.all;
entity uni is end;
architecture a of uni is
begin
  process
    variable s1 : positive := 123456;
    variable s2 : positive := 987654;
    variable x  : real := 0.0;
    variable acc : real := 0.0;
    variable acc2 : real := 0.0;
    procedure draw(variable a, b : inout positive) is
    begin
      uniform(a, b, x);
      acc := acc + x;
    end procedure;
    -- The real is in the frame of the procedure, the seeds in the process's: the three
    -- variables are not in one frame.
    procedure draw2(variable a, b : inout positive) is
      variable y : real := 0.0;
    begin
      uniform(a, b, y);
      acc2 := acc2 + y;
    end procedure;
  begin
    for i in 1 to 1000 loop
      draw(s1, s2);
    end loop;
    report \"acc=\" & real'image(acc) & \" s1=\" & integer'image(s1) & \" s2=\" & integer'image(s2);
    s1 := 123456;
    s2 := 987654;
    for i in 1 to 1000 loop
      draw2(s1, s2);
    end loop;
    report \"acc2=\" & real'image(acc2) & \" s1=\" & integer'image(s1);
    s1 := positive'high;
    draw(s1, s2);
    report \"bad seed1 x=\" & real'image(x) & \" s1=\" & integer'image(s1);
    s1 := 5;
    s2 := positive'high;
    draw(s1, s2);
    report \"bad seed2 x=\" & real'image(x) & \" s2=\" & integer'image(s2);
    wait;
  end process;
end;
";

#[test]
fn uniform_is_an_op_and_reports_bad_seeds() {
    let d = tmp("uniform");
    std::fs::write(d.join("uni.vhd"), UNIFORM).expect("write");
    let work = format!("work:{}", d.join("lib").display());
    let yavs = env!("CARGO_BIN_EXE_yavs");
    let a = Command::new(yavs)
        .current_dir(&d)
        .args(["analyze", "--std=2008", "--work", &work, "uni.vhd"])
        .output()
        .expect("analyze");
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    let o = Command::new(yavs)
        .current_dir(&d)
        .env("YAVS_DUMPIR", "1")
        .args([
            "--color",
            "never",
            "run",
            "--no-cache",
            "--std=2008",
            "--work",
            &work,
        ])
        .args(["--tier=interp", "uni"])
        .output()
        .expect("run");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    // The generator's own numbers (the body's L'Ecuyer steps), and the two reports.
    for want in [
        "acc=501.184307579734 s1=1444183576 s2=208212704",
        "acc2=501.184307579734 s1=1444183576",
        "SEED1 > 2147483562 in UNIFORM",
        "bad seed1 x=0 s1=2147483647",
        "SEED2 > 2147483398 in UNIFORM",
        "bad seed2 x=0 s2=2147483647",
    ] {
        assert!(text.contains(want), "missing `{want}`:\n{text}");
    }
    assert!(
        text.lines()
            .any(|l| l.contains(" pc") && l.contains(" Uniform {")),
        "no op Uniform\n{text}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// A constant record assigned to a record signal (`r <= C`) drives all fields with one instruction
/// (`SigScatter`, #677), whatever the length of a field (one block of planes or more), the delay and the
/// values (`U`, `X`, `Z`, `-` ...). So does a part of one that is a record (`q.i`), and a field
/// of a signal that is no record is one op (`CopyDrive`).
const FIELD_DRIVE: &str = "
library ieee;
use ieee.std_logic_1164.all;
entity fd is end;
architecture a of fd is
  type rec_t is record
    a : std_ulogic_vector(31 downto 0);
    b : std_ulogic_vector(0 downto 0);
    c : std_ulogic_vector(63 downto 0);
    d : std_ulogic_vector(69 downto 0);
    e : std_ulogic_vector(0 to 6);
  end record;
  constant C : rec_t := (
    a => \"AAAA\",
    b => \"BBBB\",
    c => \"CCCC\",
    d => \"DDDD\",
    e => \"EEEE\");
  signal da : std_ulogic_vector(31 downto 0);
  signal db : std_ulogic_vector(0 downto 0);
  signal dc : std_ulogic_vector(63 downto 0);
  signal dd : std_ulogic_vector(69 downto 0);
  signal de : std_ulogic_vector(0 to 6);
  signal dt : std_ulogic_vector(31 downto 0);
  signal r, r2, r3 : rec_t;
  type inner_t is record
    f : std_ulogic_vector(69 downto 0);
    g : std_ulogic_vector(3 downto 0);
  end record;
  type outer_t is record
    i : inner_t;
    z : std_ulogic;
  end record;
  constant CI : inner_t := (f => \"DDDD\", g => \"0101\");
  signal q : outer_t;
begin
  process
    variable bad : integer := 0;
    procedure same(what : string; got, want : std_ulogic_vector) is
    begin
      if got /= want then
        bad := bad + 1;
        report what & \" differs\";
      end if;
    end procedure;
  begin
    wait for 1 ns;
    da <= C.a;
    db <= C.b;
    dc <= C.c;
    dd <= C.d;
    de <= C.e;
    dt <= C.a after 2 ns;
    r <= C;
    r2 <= C after 2 ns;
    r3 <= transport C after 1 ns;
    q.i <= CI;
    q.z <= '1';
    wait for 1 ns;
    same(\"a\", da, C.a);
    same(\"b\", db, C.b);
    same(\"c\", dc, C.c);
    same(\"d\", dd, C.d);
    same(\"e\", de, C.e);
    same(\"r.a\", r.a, C.a);
    same(\"r.b\", r.b, C.b);
    same(\"r.c\", r.c, C.c);
    same(\"r.d\", r.d, C.d);
    same(\"r.e\", r.e, C.e);
    same(\"q.i.f\", q.i.f, CI.f);
    same(\"q.i.g\", q.i.g, CI.g);
    same(\"r3.a\", r3.a, C.a);
    same(\"r3.d\", r3.d, C.d);
    wait for 2 ns;
    same(\"t\", dt, C.a);
    same(\"r2.a\", r2.a, C.a);
    same(\"r2.d\", r2.d, C.d);
    same(\"r2.e\", r2.e, C.e);
    report \"fields bad=\" & integer'image(bad);
    wait;
  end process;
end;
";

fn pattern(n: usize, seed: usize) -> String {
    (0..n)
        .map(|i| char::from(b"UX01ZWLH-"[(i * 7 + i / 3 + seed) % 9]))
        .collect()
}

#[test]
fn a_constant_field_driven_onto_a_signal_is_one_op() {
    let d = tmp("field_drive");
    let src = FIELD_DRIVE
        .replace("AAAA", &pattern(32, 1))
        .replace("BBBB", &pattern(1, 2))
        .replace("CCCC", &pattern(64, 3))
        .replace("DDDD", &pattern(70, 4))
        .replace("EEEE", &pattern(7, 5));
    std::fs::write(d.join("fd.vhd"), src).expect("write");
    let work = format!("work:{}", d.join("lib").display());
    let yavs = env!("CARGO_BIN_EXE_yavs");
    let a = Command::new(yavs)
        .current_dir(&d)
        .args(["analyze", "--std=2008", "--work", &work, "fd.vhd"])
        .output()
        .expect("analyze");
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    let o = Command::new(yavs)
        .current_dir(&d)
        .env("YAVS_DUMPIR", "1")
        .args([
            "--color",
            "never",
            "run",
            "--no-cache",
            "--std=2008",
            "--work",
            &work,
        ])
        .args(["--tier=interp", "fd"])
        .output()
        .expect("run");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(text.contains("fields bad=0"), "{text}");
    assert!(
        text.lines()
            .any(|l| l.contains(" pc") && l.contains(" CopyDrive {")),
        "no op CopyDrive\n{text}"
    );
    assert!(
        text.lines().filter(|l| l.contains(" SigScatter {")).count() >= 4,
        "no SigScatter\n{text}"
    );
    let _ = std::fs::remove_dir_all(&d);
}

/// `=` of two logic vectors is one op (`LEq`): the same answer as an element-by-element compare
/// for lengths on both sides of the 64-element blocks and for every std_ulogic value.
const VECTOR_EQ: &str = "
library ieee;
use ieee.std_logic_1164.all;
entity ve is end;
architecture a of ve is
  procedure check(n : natural; seed : natural; bad : inout integer) is
    variable a, b : std_ulogic_vector(0 to 199);
    variable same : boolean;
    variable eq : boolean;
  begin
    for i in a'range loop
      a(i) := std_ulogic'val((i * 7 + seed) mod 9);
      b(i) := a(i);
    end loop;
    for round in 0 to 3 loop
      -- Equal in the first n elements, different after them: only the first n count.
      if n < 200 then
        b(n) := std_ulogic'val((std_ulogic'pos(a(n)) + 1) mod 9);
      end if;
      same := true;
      for i in 0 to n - 1 loop
        if a(i) /= b(i) then same := false; end if;
      end loop;
      eq := a(0 to n - 1) = b(0 to n - 1);
      if eq /= same then bad := bad + 1; end if;
      -- Now one element inside differs.
      if n > 0 then
        b(n - 1) := std_ulogic'val((std_ulogic'pos(a(n - 1)) + 1 + round) mod 9);
        same := true;
        for i in 0 to n - 1 loop
          if a(i) /= b(i) then same := false; end if;
        end loop;
        eq := a(0 to n - 1) = b(0 to n - 1);
        if eq /= same then bad := bad + 1; end if;
        b(n - 1) := a(n - 1);
      end if;
    end loop;
  end procedure;
begin
  process
    variable bad : integer := 0;
  begin
    for n in 0 to 140 loop
      check(n, n, bad);
    end loop;
    check(63, 5, bad);
    check(64, 5, bad);
    check(65, 5, bad);
    check(128, 3, bad);
    check(129, 3, bad);
    report \"vector eq bad=\" & integer'image(bad);
    wait;
  end process;
end;
";

#[test]
fn vector_equality_is_one_op() {
    let d = tmp("vector_eq");
    std::fs::write(d.join("ve.vhd"), VECTOR_EQ).expect("write");
    let work = format!("work:{}", d.join("lib").display());
    let yavs = env!("CARGO_BIN_EXE_yavs");
    let a = Command::new(yavs)
        .current_dir(&d)
        .args(["analyze", "--std=2008", "--work", &work, "ve.vhd"])
        .output()
        .expect("analyze");
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    let o = Command::new(yavs)
        .current_dir(&d)
        .env("YAVS_DUMPIR", "1")
        .args([
            "--color",
            "never",
            "run",
            "--no-cache",
            "--std=2008",
            "--work",
            &work,
        ])
        .args(["--tier=interp", "ve"])
        .output()
        .expect("run");
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(text.contains("vector eq bad=0"), "{text}");
    assert!(
        text.lines()
            .any(|l| l.contains(" pc") && l.contains(" LEq {")),
        "no op LEq\n{text}"
    );
    let _ = std::fs::remove_dir_all(&d);
}
