//! Cranelift inlines small native-frame callees into their callers (`yavs-clift` `Inl`). A callee
//! that reads a constant of its own (a string literal) must read that constant, not one of its
//! caller's (#765: the inliner kept the callee's symbol reference, which names another symbol in
//! the caller). Needs the `clift` feature.
#![cfg(feature = "clift")]

mod common;
use common::tempdir;

use std::process::Command;

const SRC: &str = "entity clift_inline is end;
architecture t of clift_inline is
  function enc(b : boolean) return string is
  begin
    if b then
      return \"T\";
    else
      return \"F\";
    end if;
  end;
  function hdr(x : integer; b : boolean) return string is
  begin
    return \"<\" & integer'image(x) & \":\" & enc(b) & enc(not b) & \">\";
  end;
begin
  process
  begin
    for i in 0 to 5 loop
      report hdr(i, i mod 2 = 0);
    end loop;
    wait;
  end process;
end;
";

#[test]
fn inlined_callee_reads_its_own_constants() {
    let d = tempdir("clift_inline");
    std::fs::write(d.join("inl.vhd"), SRC).expect("write");
    let work = format!("work:{}", d.join("lib").display());
    let yavs = env!("CARGO_BIN_EXE_yavs");
    let a = Command::new(yavs)
        .current_dir(&d)
        .args(["analyze", "--work", &work, "inl.vhd"])
        .output()
        .expect("analyze");
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    let run = |tier: &str| {
        let o = Command::new(yavs)
            .current_dir(&d)
            .args(["--color", "never", "run", "--no-cache", "--work", &work])
            .arg(format!("--tier={tier}"))
            .arg("clift_inline")
            // Every callee of this design is small enough to be inlined.
            .env("YAVS_CLIFT_INLINE", "600")
            .output()
            .expect("run");
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        String::from_utf8_lossy(&o.stdout).into_owned() + &String::from_utf8_lossy(&o.stderr)
    };
    let want = run("interp");
    assert!(want.contains("<0:TF>") && want.contains("<5:FT>"), "{want}");
    assert_eq!(run("clift"), want);
}
