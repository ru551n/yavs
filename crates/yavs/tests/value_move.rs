//! Copies of composite values: between two objects of one heap (access values), a logic vector
//! range between two designated vectors, and whole records through designated objects (#677).

mod common;
use common::tempdir as tmp;

use std::process::Command;

const COPIES: &str = "
library ieee;
use ieee.std_logic_1164.all;
entity copies is end;
architecture a of copies is
  type slv_ptr is access std_logic_vector;
  type rec is record
    a : integer;
    v : std_logic_vector(0 to 299);
    s : string(1 to 5);
  end record;
  type rec_ptr is access rec;
begin
  process
    variable p, q : slv_ptr;
    variable r, t : rec_ptr;
    variable ok : boolean := true;
  begin
    p := new std_logic_vector(0 to 999);
    q := new std_logic_vector(0 to 999);
    for i in p'range loop
      if i mod 3 = 0 then p(i) := '1'; elsif i mod 3 = 1 then p(i) := 'Z'; else p(i) := 'X'; end if;
      q(i) := '0';
    end loop;
    -- aligned, unaligned and short ranges from one designated vector to another
    q(64 to 575) := p(64 to 575);
    q(3 to 12) := p(500 to 509);
    q(700 to 999) := p(701 to 1000 - 1) & '1';
    for i in q'range loop
      if i >= 64 and i <= 575 then ok := ok and q(i) = p(i);
      elsif i >= 3 and i <= 12 then ok := ok and q(i) = p(i + 497);
      elsif i >= 700 and i <= 998 then ok := ok and q(i) = p(i + 1);
      elsif i = 999 then ok := ok and q(i) = '1';
      else ok := ok and q(i) = '0';
      end if;
    end loop;
    assert ok report \"vector ranges differ\" severity failure;
    -- whole records between two designated objects
    r := new rec;
    t := new rec;
    r.a := 42;
    r.v := (others => '1');
    r.v(100 to 199) := (others => 'H');
    r.s := \"hello\";
    t.all := r.all;
    assert t.a = 42 and t.v(150) = 'H' and t.v(250) = '1' and t.s = \"hello\" report \"record copy\" severity failure;
    r.a := 7;
    assert t.a = 42 report \"copy shares storage\" severity failure;
    t.all := t.all;
    assert t.a = 42 and t.s = \"hello\" report \"self copy\" severity failure;
    report \"copies ok\";
    wait;
  end process;
end;
";

#[test]
fn copies_between_designated_objects() {
    let d = tmp("value_move");
    std::fs::write(d.join("copies.vhd"), COPIES).expect("write");
    let work = format!("work:{}", d.join("lib").display());
    let yavs = env!("CARGO_BIN_EXE_yavs");
    let a = Command::new(yavs)
        .current_dir(&d)
        .args(["analyze", "--std=2008", "--work", &work, "copies.vhd"])
        .output()
        .expect("analyze");
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    let tiers: &[&str] = if cfg!(feature = "llvm") {
        &["interp", "auto"]
    } else {
        &["interp"]
    };
    for tier in tiers {
        let o = Command::new(yavs)
            .current_dir(&d)
            .args([
                "--color",
                "never",
                "run",
                "--std=2008",
                "--no-cache",
                "--work",
                &work,
            ])
            .arg(format!("--tier={tier}"))
            .arg("copies")
            .output()
            .expect("run");
        let (out, err) = (
            String::from_utf8_lossy(&o.stdout),
            String::from_utf8_lossy(&o.stderr),
        );
        assert!(o.status.success(), "{tier}: {out}\n{err}");
        assert!(
            out.contains("copies ok") || err.contains("copies ok"),
            "{tier}: {out}\n{err}"
        );
    }
    let _ = std::fs::remove_dir_all(&d);
}
