//! The position of an index in a descriptor of the constant pool is worked out when the Cranelift
//! code is made (`yavs-clift` `Gen::const_dim`): ascending and descending ranges give the
//! interpreter's sums, and an index outside the range its error. Needs the `clift` feature.
#![cfg(feature = "clift")]

mod common;
use common::tempdir;

use std::process::Command;

const SRC: &str = "entity clift_const_dims is end;
architecture t of clift_const_dims is
  type iv is array (integer range <>) of integer;
  constant dn : iv(7 downto 2) := (70, 60, 50, 40, 30, 20);
  constant up : iv(-1 to 3) := (1, 2, 3, 4, 5);
  signal lim : integer := 3;
begin
  process
    variable s : integer := 0;
  begin
    wait for 1 ns;
    for k in 0 to 9 loop
      for i in -1 to lim loop
        s := s + up(i);
      end loop;
      for i in 2 to 7 loop
        s := s + dn(i);
      end loop;
    end loop;
    report \"sum \" & integer'image(s);
    lim <= 4;
    wait for 1 ns;
    for i in -1 to lim loop
      s := s + up(i);
    end loop;
    report \"not reached\";
    wait;
  end process;
end;
";

#[test]
fn positions_in_constant_descriptors_match_the_interpreter() {
    let d = tempdir("clift_const_dims");
    std::fs::write(d.join("cd.vhd"), SRC).expect("write");
    let work = format!("work:{}", d.join("lib").display());
    let yavs = env!("CARGO_BIN_EXE_yavs");
    let a = Command::new(yavs)
        .current_dir(&d)
        .args(["analyze", "--work", &work, "cd.vhd"])
        .output()
        .expect("analyze");
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    let run = |flags: &[&str]| {
        let o = Command::new(yavs)
            .current_dir(&d)
            .args(["--color", "never", "run", "--no-cache", "--work", &work])
            .args(flags)
            .arg("clift_const_dims")
            .output()
            .expect("run");
        (
            String::from_utf8_lossy(&o.stdout).into_owned() + &String::from_utf8_lossy(&o.stderr),
            o.status.code(),
        )
    };
    let want = run(&["--tier=interp"]);
    assert!(
        want.0.contains("sum 2850") && want.0.contains("index 4 outside of range -1 to 3"),
        "{want:?}"
    );
    assert_eq!(run(&["--tier=clift"]), want);
    assert_eq!(run(&["--tier=clift-auto", "--tier-heat=0,0,0"]), want);
}
