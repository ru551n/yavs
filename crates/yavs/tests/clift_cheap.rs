//! `YAVS_CLIFT_CHEAP`: code lowered as if it never ran (frame-mode blocks stepped by the
//! interpreter through `yavs_rt_steps`, native frames leaving at a block start) gives the
//! interpreter's output. `conv`'s first block reads its parameter through the slot address, a
//! register the interpreter does not preload. Needs the `clift` feature.
#![cfg(feature = "clift")]

mod common;
use common::tempdir;

use std::process::Command;

const SRC: &str = "entity clift_cheap is end;
architecture t of clift_cheap is
  type ivec is array (integer range <>) of bit;
  function conv(lo : integer) return string is
    variable v : ivec(lo + 2 downto lo) := \"101\";
  begin
    return to_string(bit_vector(v));
  end function;
begin
  process
  begin
    report \"conv(0) \" & conv(0);
    if now = 0 ns then
      report \"now \" & conv(5);
    else
      report \"later \" & conv(7);
    end if;
    wait;
  end process;
end;
";

#[test]
fn cheap_blocks_give_the_interpreters_output() {
    let d = tempdir("clift_cheap");
    std::fs::write(d.join("c.vhd"), SRC).expect("write");
    let work = format!("work:{}", d.join("lib").display());
    let yavs = env!("CARGO_BIN_EXE_yavs");
    let a = Command::new(yavs)
        .current_dir(&d)
        .args(["analyze", "--work", &work, "c.vhd"])
        .output()
        .expect("analyze");
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    let run = |tier: &str, env: &[(&str, &str)]| {
        let o = Command::new(yavs)
            .current_dir(&d)
            .args(["--color", "never", "run", "--no-cache", "--work", &work])
            .arg(format!("--tier={tier}"))
            .arg("clift_cheap")
            .env("YAVS_COUNTERS", "1")
            .envs(env.iter().copied())
            .output()
            .expect("run");
        assert!(o.status.success(), "{}", String::from_utf8_lossy(&o.stderr));
        let err = String::from_utf8_lossy(&o.stderr).into_owned();
        let steps = err
            .lines()
            .find_map(|l| l.strip_prefix("counters: cb_steps="))
            .map_or(0, |v| v.parse::<u64>().expect("count"));
        let out: String = err
            .lines()
            .filter(|l| !l.starts_with("counters:"))
            .collect();
        (
            String::from_utf8_lossy(&o.stdout).into_owned() + &out,
            steps,
        )
    };
    let (want, _) = run("interp", &[]);
    assert!(want.contains("now 101"), "{want}");
    for mode in ["all", "alt"] {
        for native in ["0", "1"] {
            let (got, steps) = run(
                "clift",
                &[("YAVS_CLIFT_CHEAP", mode), ("YAVS_CLIFT_NATIVE", native)],
            );
            assert_eq!(got, want, "{mode}, native {native}");
            // Counted in debug builds only.
            assert!(
                cfg!(not(debug_assertions)) || steps > 0,
                "{mode}, native {native}"
            );
        }
    }
}
