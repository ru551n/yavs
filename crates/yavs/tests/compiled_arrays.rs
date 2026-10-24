//! Array element access in compiled code (`--tier=llvm`, `--tier=auto` compiling everything at
//! once; a build with the `llvm` feature) against the interpreter: ascending and descending
//! ranges, a null range, records with a stride that is no power of two, and the errors of an
//! index outside the range (the same text and exit status in every tier).
#![cfg(feature = "llvm")]

mod common;
use common::tempdir;

use std::path::Path;
use std::process::{Command, Output};

const SRC: &str = r#"entity arrays is
  generic (bad : integer := 0);
end;
architecture a of arrays is
  type rec_t is record
    x, y : integer;
    c    : character;
  end record;
  type rec_vec_t is array (integer range <>) of rec_t;
  type rec_ptr_t is access rec_vec_t;
  type ints_t is array (integer range <>) of integer;
  type int_ptr_t is access ints_t;
  type tab_t is array (integer range <>) of int_ptr_t;
  type tab_ptr_t is access tab_t;

  procedure sum_up (variable p : inout int_ptr_t; variable s : inout integer) is
  begin
    for i in p'range loop s := s + p(i) * (i + 3); end loop;
  end procedure;

  procedure sum_rec (variable p : inout rec_ptr_t; variable s : inout integer) is
  begin
    for i in p'range loop s := s + p(i).x * 7 + p(i).y - character'pos(p(i).c); end loop;
  end procedure;

  procedure at_index (variable p : inout int_ptr_t; i : integer; variable s : inout integer) is
  begin
    s := s + p(i);
  end procedure;

  procedure via_table (variable t : inout tab_ptr_t; r, i : integer; variable s : inout integer) is
  begin
    s := s - t(r)(i);
  end procedure;
begin
  process
    variable up    : int_ptr_t := new ints_t(5 to 14);
    variable down  : int_ptr_t := new ints_t(9 downto 0);
    variable lone  : int_ptr_t := new ints_t(-3 to -3);
    variable empty : int_ptr_t := new ints_t(1 to 0);
    variable emptd : int_ptr_t := new ints_t(0 downto 3);
    variable recs  : rec_ptr_t := new rec_vec_t(-2 to 6);
    variable rdown : rec_ptr_t := new rec_vec_t(4 downto 1);
    variable t     : tab_ptr_t := new tab_t(0 to 2);
    variable acc   : integer := 0;
  begin
    for i in up'range loop up(i) := i * 5 - 20; end loop;
    for i in down'range loop down(i) := 100 - i * i; end loop;
    lone(-3) := 42;
    for i in recs'range loop recs(i) := (x => i, y => i * i, c => character'val(65 + (i + 2))); end loop;
    for i in rdown'range loop rdown(i) := (x => -i, y => 3 * i, c => character'val(97 + i)); end loop;
    t(0) := up; t(1) := down; t(2) := lone;
    for round in 1 to 40 loop
      sum_up(up, acc); sum_up(down, acc); sum_up(lone, acc); sum_up(empty, acc); sum_up(emptd, acc);
      sum_rec(recs, acc); sum_rec(rdown, acc);
      for i in 5 to 14 loop at_index(up, i, acc); via_table(t, 0, i, acc); end loop;
      for i in 0 to 9 loop at_index(down, i, acc); via_table(t, 1, i, acc); end loop;
      at_index(lone, -3, acc); via_table(t, 2, -3, acc);
    end loop;
    report "sum " & integer'image(acc);
    if bad = 1 then at_index(up, 15, acc); end if;
    if bad = 2 then at_index(down, -1, acc); end if;
    if bad = 3 then at_index(empty, 1, acc); end if;
    if bad = 4 then at_index(emptd, 3, acc); end if;
    if bad = 5 then sum_rec(recs, acc); at_index(up, rdown(5).x, acc); end if;
    if bad = 6 then via_table(t, 3, 0, acc); end if;
    if bad = 7 then at_index(up, 2**30, acc); end if;
    if bad = 8 then at_index(up, -(2**30), acc); end if;
    report "end " & integer'image(acc);
    wait;
  end process;
end;
"#;

fn yavs(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run yavs")
}

/// Standard output, standard error and the exit status of one run.
fn run(dir: &Path, flags: &[&str], bad: u32) -> (String, String, Option<i32>) {
    let g = format!("-gbad={bad}");
    let mut args = vec!["run", "--std=2008", "--work=work:work"];
    args.extend_from_slice(flags);
    args.extend(["arrays", &g]);
    let o = yavs(dir, &args);
    (
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
        o.status.code(),
    )
}

#[test]
fn element_access_in_compiled_code_matches_the_interpreter() {
    let dir = tempdir("compiled_arrays");
    std::fs::write(dir.join("arrays.vhd"), SRC).expect("write");
    let a = yavs(
        &dir,
        &["analyze", "--std=2008", "--work=work:work", "arrays.vhd"],
    );
    assert!(a.status.success(), "{a:?}");
    for bad in 0..=8 {
        let want = run(&dir, &["--tier=interp"], bad);
        assert!(want.0.contains("sum "), "{want:?}");
        assert_eq!(want.2 == Some(0), bad == 0, "bad {bad}: {want:?}");
        for flags in [
            vec!["--tier=llvm"],
            vec!["--tier=auto", "--tier-heat=1,0,0"],
            vec!["--tier=auto", "--tier-heat=1,0,0", "--threads=4"],
        ] {
            let got = run(&dir, &flags, bad);
            assert_eq!(got, want, "bad {bad} {flags:?}");
        }
    }
}
