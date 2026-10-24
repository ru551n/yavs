//! Logic vector operations in compiled code against the interpreter: `and`/`or`/`xor` and their
//! negations, `not`, `to_x01`, `to_ux01`, fills, slices (null ones too) and concatenations of
//! every length class (1 element, the plane block of 64, 65, several blocks), with every
//! std_ulogic code among the elements. Output and exit status are the same in every tier.
#![cfg(any(feature = "llvm", feature = "clift"))]

mod common;
use common::tempdir;

use std::path::Path;
use std::process::{Command, Output};

const SRC: &str = r#"library ieee;
use ieee.std_logic_1164.all;
entity v is end;
architecture a of v is
  subtype slv is std_logic_vector;
  function mk(n : natural; k : natural) return slv is
    variable r : slv(n - 1 downto 0);
  begin
    for i in r'range loop
      r(i) := std_logic'val((i * k + i / 3 + k) mod 9);
    end loop;
    return r;
  end;
  function f_and(a, b : slv) return slv is begin return a and b; end;
  function f_or(a, b : slv) return slv is begin return a or b; end;
  function f_xor(a, b : slv) return slv is begin return a xor b; end;
  function f_nand(a, b : slv) return slv is begin return a nand b; end;
  function f_nor(a, b : slv) return slv is begin return a nor b; end;
  function f_xnor(a, b : slv) return slv is begin return a xnor b; end;
  function f_not(a : slv) return slv is begin return not a; end;
  function f_x01(a : slv) return slv is begin return to_x01(a); end;
  function f_ux01(a : slv) return slv is begin return to_ux01(a); end;
  function f_x01z(a : slv) return slv is begin return to_x01z(a); end;
  function fl(n : natural; c : std_logic) return slv is
    variable r : slv(n - 1 downto 0) := (others => c);
  begin
    return r;
  end;
  function part(a : slv; l, r : integer) return slv is
    variable s : slv(l downto r);
  begin
    s := a(l downto r);
    return s;
  end;
  function cat(a, b : slv) return slv is begin return a & '1' & b; end;
  function mid(a : slv; l, r : integer) return natural is
  begin
    return a(l downto r)'length;
  end;
  procedure all_ops(n, k : natural) is
    variable a, b : slv(n - 1 downto 0);
  begin
    a := mk(n, k);
    b := mk(n, k + 2);
    report integer'image(n) & " " & integer'image(k) & " " & to_string(a) & " " & to_string(b);
    report to_string(f_and(a, b)) & " " & to_string(f_or(a, b)) & " " & to_string(f_xor(a, b));
    report to_string(f_nand(a, b)) & " " & to_string(f_nor(a, b)) & " " & to_string(f_xnor(a, b));
    report to_string(f_not(a)) & " " & to_string(f_x01(a)) & " " & to_string(f_ux01(a)) & " " & to_string(f_x01z(b));
    report to_string(fl(n, 'Z')) & " " & to_string(fl(n, '-')) & " " & to_string(fl(n, '1'));
    report to_string(cat(a, b)) & " " & integer'image(cat(a, b)'length);
    if n > 2 then
      report to_string(part(a, n - 1, 1)) & "|" & to_string(part(a, n - 2, 0)) & "|" & to_string(part(a, 0, 1));
      report integer'image(mid(a, n - 1, 1)) & " " & integer'image(mid(b, 1, n - 1));
    end if;
  end;
begin
  process
    constant lens : integer_vector := (1, 2, 7, 63, 64, 65, 100, 128, 129, 200);
  begin
    for li in lens'range loop
      for k in 1 to 3 loop
        all_ops(lens(li), k);
      end loop;
    end loop;
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

fn run(dir: &Path, flags: &[&str]) -> (Option<i32>, String) {
    let mut args = vec!["run", "--std=2008", "--work=work:work"];
    args.extend_from_slice(flags);
    args.push("v");
    let o = yavs(dir, &args);
    (
        o.status.code(),
        String::from_utf8_lossy(&o.stdout).into_owned(),
    )
}

#[test]
fn vector_operations_in_compiled_code_match_the_interpreter() {
    let dir = tempdir("compiled_vectors");
    std::fs::write(dir.join("v.vhd"), SRC).expect("write");
    let a = yavs(
        &dir,
        &["analyze", "--std=2008", "--work=work:work", "v.vhd"],
    );
    assert!(a.status.success(), "{a:?}");
    let want = run(&dir, &["--tier=interp"]);
    assert_eq!(want.0, Some(0), "{want:?}");
    assert!(want.1.lines().count() >= 300, "{}", want.1);
    for flags in [
        #[cfg(feature = "llvm")]
        vec!["--tier=llvm"],
        #[cfg(feature = "llvm")]
        vec!["--tier=auto", "--tier-heat=1,0,0"],
        #[cfg(feature = "clift")]
        vec!["--tier=clift"],
        #[cfg(feature = "clift")]
        vec!["--tier=clift-auto", "--tier-heat=1,0,0"],
        #[cfg(all(feature = "llvm", feature = "clift"))]
        vec!["--tier=auto3", "--tier-heat=1,0,0"],
    ] {
        assert_eq!(run(&dir, &flags), want, "{flags:?}");
    }
}
