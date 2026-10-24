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
    procedure draw(variable a, b : inout positive) is
    begin
      uniform(a, b, x);
      acc := acc + x;
    end procedure;
  begin
    for i in 1 to 1000 loop
      draw(s1, s2);
    end loop;
    report \"acc=\" & real'image(acc) & \" s1=\" & integer'image(s1) & \" s2=\" & integer'image(s2);
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

/// A constant record assigned to a record signal (`r <= C`) drives each field with one op (`LCopyDrive`), whatever
/// the length of the field (one block of planes or more), the delay and the values (`U`, `X`, `Z`,
/// `-` ...).
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
            .any(|l| l.contains(" pc") && l.contains(" LCopyDrive {")),
        "no op LCopyDrive\n{text}"
    );
    let _ = std::fs::remove_dir_all(&d);
}
