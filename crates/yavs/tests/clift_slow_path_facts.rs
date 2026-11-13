//! A native frame's slow path that hands its instruction to the interpreter (`Gen::native_step`,
//! generated after the body) passes the registers as they are at its own pc, not the constants the
//! last instruction of the function knew (#1089). Needs the `clift` feature.
#![cfg(feature = "clift")]

mod common;
use common::tempdir;

use std::process::Command;

// The register sharing gives `c` and `t` one cell, the last block of the code ends with `t := 7`
// (the rest of the inlined `g`), and the fill of `e`, null at run time, has no fast path (a fill of
// 100 elements has one since 2026-11-13).
const SRC: &str = "library ieee; use ieee.std_logic_1164.all;
entity clift_slow_path_facts is end;
architecture a of clift_slow_path_facts is
  function g(a : integer) return integer is
  begin
    if a > 0 then return a + 2; else return 1; end if;
  end;
  function f(c : std_logic; x, y, z, w : integer) return integer is
    variable v : std_logic_vector(0 to 99);
    variable e : std_logic_vector(1 to w - 4);
    variable k, t : integer := 0;
    variable a1, a2, a3, a4, a5, a6, a7, a8 : integer;
  begin
    a1 := x + y; a2 := a1 * z; a3 := a2 - w; a4 := a3 + x;
    a5 := a4 * y; a6 := a5 - z; a7 := a6 + w; a8 := a7 + a1 * a3;
    v := (others => c);
    e := (others => c);
    if c = '1' then
      k := g(a8);
      t := 7;
    else
      t := a7 + 1;
    end if;
    for i in v'range loop
      if v(i) = '1' then k := k + 1; end if;
    end loop;
    return k + t + a2 + a4 + a5;
  end;
begin
  process begin
    for n in 1 to 4 loop
      report \"sum \" & integer'image(f('1', n, 2, 3, 4));
    end loop;
    wait;
  end process;
end;
";

#[test]
fn a_slow_path_reads_the_registers_of_its_own_pc() {
    let d = tempdir("clift_slow_path_facts");
    std::fs::write(d.join("f.vhd"), SRC).expect("write");
    let work = format!("work:{}", d.join("lib").display());
    let yavs = env!("CARGO_BIN_EXE_yavs");
    let a = Command::new(yavs)
        .current_dir(&d)
        .args(["analyze", "--std=2008", "--work", &work, "f.vhd"])
        .output()
        .expect("analyze");
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    let run = |flags: &[&str], counters: bool| {
        let mut c = Command::new(yavs);
        c.current_dir(&d)
            .args(["--color", "never", "run", "--no-cache", "--threads", "1"])
            .args(["--work", &work])
            .args(flags)
            .arg("clift_slow_path_facts");
        if counters {
            c.env("YAVS_COUNTERS", "pc");
        }
        let o = c.output().expect("run");
        (
            String::from_utf8_lossy(&o.stdout).into_owned() + &String::from_utf8_lossy(&o.stderr),
            o.status.code(),
        )
    };
    let want = run(&["--tier=interp"], false);
    assert_eq!(want.1, Some(0), "{want:?}");
    for s in ["sum 164", "sum 204", "sum 250", "sum 302"] {
        assert!(want.0.contains(s), "{s}: {want:?}");
    }
    assert_eq!(run(&["--tier=clift"], false), want);
    assert_eq!(
        run(&["--tier=clift-auto", "--tier-heat=1,0,0"], false),
        want
    );
    // The test means nothing unless the fill runs through that slow path (counted in debug builds).
    if cfg!(debug_assertions) {
        let (out, _) = run(&["--tier=clift"], true);
        assert!(out.contains("cb_xstep.LFill.f."), "{out}");
    }
}
