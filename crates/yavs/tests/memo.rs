//! Values that depend only on generics and generate parameters are computed once per instance
//! by a process (`yavs-ir` `opt/memo.rs`): the run is the same as without it, and a failing
//! check fails at the same time with the same text.

mod common;
use common::tempdir as tmp;

use std::path::Path;
use std::process::Command;

const GOOD: &str = "
entity good is generic (G : natural := 5); end;
architecture a of good is
  signal clk : bit := '0';
  signal v : bit_vector(0 to 3) := \"1010\";
  signal o : bit_vector(0 to 3);
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;
  gen: for i in 0 to 3 generate
    process (clk) begin
      if clk = '1' then o(i) <= v((i + G) mod 4) xor v((i + 1) mod 4); end if;
    end process;
  end generate;
  process begin
    wait for 100 ns;
    report \"o=\" & to_string(o);
    done <= true; wait;
  end process;
end;
";

const BAD: &str = "
entity bad is generic (G : natural := 5); end;
architecture a of bad is
  signal clk : bit := '0';
  signal v : bit_vector(0 to 2) := \"101\";
  signal o : bit_vector(0 to 2);
begin
  clk <= not clk after 5 ns;
  gen: for i in 0 to 2 generate
    process (clk)
      variable n : natural := 0;
    begin
      if clk = '1' then
        n := n + 1;
        if n = 3 then o(i) <= v((i + G) mod 4); end if;
      end if;
    end process;
  end generate;
end;
";

fn run(dir: &Path, top: &str, src: &str, no_memo: bool) -> (i32, String, String) {
    std::fs::write(dir.join(format!("{top}.vhd")), src).expect("write");
    let work = format!("work:{}", dir.join("lib").display());
    let yavs = env!("CARGO_BIN_EXE_yavs");
    let a = Command::new(yavs)
        .current_dir(dir)
        .args(["analyze", "--work", &work, &format!("{top}.vhd")])
        .output()
        .expect("analyze");
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    let mut c = Command::new(yavs);
    c.current_dir(dir)
        .args(["--color", "never", "run", "--no-cache", "--work", &work])
        .args(["--stop-time=200ns", top]);
    if no_memo {
        c.env("YAVS_NO_MEMO", "1");
    }
    let o = c.output().expect("run");
    (
        o.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

#[test]
fn instance_constants_do_not_change_a_run() {
    let d = tmp("good");
    let with = run(&d, "good", GOOD, false);
    let without = run(&d, "good", GOOD, true);
    assert_eq!(with.0, 0, "{with:?}");
    assert!(with.1.contains("o=") || with.2.contains("o="), "{with:?}");
    assert_eq!(with, without);
    let _ = std::fs::remove_dir_all(&d);
}

#[test]
fn a_failing_instance_check_fails_at_the_same_time() {
    let d = tmp("bad");
    let with = run(&d, "bad", BAD, false);
    let without = run(&d, "bad", BAD, true);
    assert_ne!(with.0, 0, "{with:?}");
    assert!(
        with.2.contains("25 ns") || with.2.contains("25000000fs"),
        "{with:?}"
    );
    assert_eq!(with, without);
    let _ = std::fs::remove_dir_all(&d);
}
