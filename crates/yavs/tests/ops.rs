//! Operations of the interpreter's fast loop for instructions that used to be generic steps: the
//! address of a variable of an enclosing subprogram (`AddrOf` up the static chain, in a register
//! that is defined more than once) and the comparison of reals (`FCmp`).

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
    for want in [" AddrUp {", " FCmp {"] {
        let in_ops = err.lines().any(|l| l.contains(" pc") && l.contains(want));
        assert!(in_ops, "no op{want}\n{err}");
    }
    let _ = std::fs::remove_dir_all(&d);
}
