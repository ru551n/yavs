//! `YAVS_CLIFT_LIST` (`yavs-clift` `blockcov.rs`): a coverage run (`write`) stores the blocks that
//! never ran, a later run (`on`) lowers them cheaply; a function whose listed blocks then run too
//! much (`YAVS_CLIFT_AGAIN` stepped instructions per IR instruction) is made again in full and leaves the list, so
//! the run after links its full code. The output is the interpreter's throughout. The coverage
//! run stops before `late` takes its first branch, so the list is wrong for it on purpose; the run
//! goes on long after the trip, which wakes the compile thread.
#![cfg(feature = "clift")]

mod common;
use common::tempdir;

use std::path::Path;
use std::process::{Command, Output};

const SRC: &str = "entity cl is end;
architecture a of cl is
  signal clk : bit := '0';
  signal cnt : integer := 0;
  function late(x : integer) return integer is
    variable s : integer := 0;
  begin
    for i in 0 to 63 loop
      if x > 600 then
        s := s + (i * x) mod 1000;
      elsif x < 0 then
        s := s - 1;
      else
        s := s + i;
      end if;
    end loop;
    return s;
  end function;
begin
  clk <= not clk after 5 ns when now < 100 us else clk;
  process (clk)
    variable acc : integer := 0;
  begin
    if clk = '1' then
      acc := (acc + late(cnt)) mod 1000003;
      cnt <= cnt + 1;
      if cnt = 9999 then
        report \"acc \" & integer'image(acc);
      end if;
    end if;
  end process;
end;
";

fn yavs(dir: &Path, list: &str, args: &[&str]) -> Output {
    yavs_with(dir, list, args, &[])
}

fn yavs_with(dir: &Path, list: &str, args: &[&str], env: &[(&str, &str)]) -> Output {
    let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(["--color", "never", "run", "--work=work:work", "--threads=1"])
        .args(args)
        .arg("cl")
        .env("YAVS_COUNTERS", "1")
        .env("YAVS_CLIFT_LIST", list)
        .env("YAVS_CLIFT_AGAIN", "1")
        .env("YAVS_AUTO_COUNT", "1")
        .env("YAVS_AUTO_GATE", "0")
        .env("YAVS_AUTO_SYNC", "1")
        .env_remove("YAVS_AUTO_POLICY")
        .envs(env.iter().copied())
        .current_dir(dir)
        .output()
        .expect("run yavs");
    assert!(o.status.success(), "{o:?}");
    o
}

fn counter(o: &Output, name: &str) -> u64 {
    String::from_utf8_lossy(&o.stderr)
        .lines()
        .find_map(|l| l.strip_prefix(&format!("counters: {name}=")))
        .and_then(|v| v.parse().ok())
        .unwrap_or_else(|| panic!("no counter {name}"))
}

#[test]
fn a_wrong_list_is_made_again_in_full_and_leaves_the_list() {
    let d = tempdir("clift_list");
    std::fs::write(d.join("t.vhd"), SRC).expect("write");
    let a = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(["analyze", "--work=work:work", "t.vhd"])
        .current_dir(&d)
        .output()
        .expect("analyze");
    assert!(a.status.success(), "{a:?}");
    let want = yavs(&d, "off", &["--tier=interp"]).stdout;
    assert!(String::from_utf8_lossy(&want).contains("acc "));
    // The coverage run ends before `late` takes its first branch.
    let w = yavs(&d, "write", &["--tier=auto", "--stop-time=4us"]);
    assert_eq!(counter(&w, "clift_listed"), 0);
    let on = yavs(&d, "on", &["--tier=auto"]);
    assert_eq!(on.stdout, want);
    assert!(counter(&on, "clift_listed") > 0, "{on:?}");
    assert!(counter(&on, "clift_again") > 0, "{on:?}");
    // `late` left the list: its full code is linked, nothing trips.
    let next = yavs(&d, "on", &["--tier=auto"]);
    assert_eq!(next.stdout, want);
    assert_eq!(counter(&next, "clift_tripped"), 0, "{next:?}");
    // Every block listed: whatever trips is made again, the output stays.
    let all = yavs(&d, "all", &["--tier=auto", "--threads=4"]);
    assert_eq!(all.stdout, want);
    assert!(counter(&all, "clift_again") > 0, "{all:?}");
    // Native-frame variants that leave at every listed block.
    let leave = yavs_with(
        &d,
        "all",
        &["--tier=auto", "--threads=4"],
        &[("YAVS_CLIFT_LIST_NATIVE", "cheap")],
    );
    assert_eq!(leave.stdout, want);
}

const COPIES: &str = "entity cp_unit is
  generic (k : integer);
  port (clk : in bit);
end;
architecture a of cp_unit is
begin
  process (clk)
    variable n : integer := 0;
    variable acc : integer := 0;
  begin
    if clk = '1' then
      n := n + 1;
      if n < 5 then
        for i in 0 to 199 loop
          acc := (acc + i * k) mod 1009;
        end loop;
      else
        acc := (acc + k) mod 1009;
      end if;
      if n = 3000 then
        report \"acc \" & integer'image(acc);
      end if;
    end if;
  end process;
end;
entity cp is end;
architecture a of cp is
  signal clk : bit := '0';
begin
  clk <= not clk after 5 ns when now < 31 us else clk;
  u1 : entity work.cp_unit generic map (k => 3) port map (clk);
  u2 : entity work.cp_unit generic map (k => 7) port map (clk);
end;
";

/// A process copy runs from its switch, which moves between runs: here the coverage run switches
/// after the first branch is done with, the next run before it. Copies get no list, so nothing
/// trips; with a list the copies would step the first branch and be made again.
#[test]
fn process_copies_get_no_list() {
    let d = tempdir("clift_list_process_copies");
    std::fs::write(d.join("t.vhd"), COPIES).expect("write");
    let a = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(["analyze", "--work=work:work", "t.vhd"])
        .current_dir(&d)
        .output()
        .expect("analyze");
    assert!(a.status.success(), "{a:?}");
    let run = |list: &str, tier: &str, after: &str| {
        let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
            .args(["--color", "never", "run", "--work=work:work", "--threads=1"])
            .arg(tier)
            .arg("cp")
            .env("YAVS_COUNTERS", "1")
            .env("YAVS_CLIFT_LIST", list)
            .env("YAVS_CLIFT_AGAIN", "1")
            .env("YAVS_AUTO_COUNT", "1")
            .env("YAVS_AUTO_GATE", "0")
            .env("YAVS_AUTO_SYNC", "1")
            .env("YAVS_SPEC_ALL", "1")
            .env("YAVS_SPEC_AFTER", after)
            .env_remove("YAVS_AUTO_POLICY")
            .current_dir(&d)
            .output()
            .expect("run yavs");
        assert!(o.status.success(), "{o:?}");
        o
    };
    let want = run("off", "--tier=interp", "2").stdout;
    assert!(String::from_utf8_lossy(&want).contains("acc "));
    let w = run("write", "--tier=auto", "40");
    assert_eq!(w.stdout, want);
    let on = run("on", "--tier=auto", "2");
    assert_eq!(on.stdout, want);
    assert!(counter(&on, "clift_copy_unlisted") >= 2, "{on:?}");
    assert_eq!(counter(&on, "clift_tripped"), 0, "{on:?}");
}

/// Copies of one process with every block listed: process copies get no list
/// (`process_copies_get_no_list`), copies of subprograms do and name their id slot, so no two
/// copies share a pre-key. Debug builds check every pre-key hit against the lowering.
#[test]
fn copies_with_a_list_do_not_share_a_pre_key() {
    for t in ["held/held_active", "process/edge_alias"] {
        let d = tempdir(&format!("clift_list_copies_{}", t.replace('/', "_")));
        let src = Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../tests/corpus/{t}.vhd"));
        let top = t.rsplit('/').next().unwrap();
        let a = Command::new(env!("CARGO_BIN_EXE_yavs"))
            .args(["analyze", "--std=2008", "--work=work:work"])
            .arg(&src)
            .current_dir(&d)
            .output()
            .expect("analyze");
        assert!(a.status.success(), "{a:?}");
        let run = |list: &str, args: &[&str]| {
            let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
                .args([
                    "--color",
                    "never",
                    "run",
                    "--std=2008",
                    "--work=work:work",
                    "--threads=1",
                ])
                .args(args)
                .arg(top)
                .env("YAVS_COUNTERS", "1")
                .env("YAVS_CLIFT_LIST", list)
                .env("YAVS_CLIFT_AGAIN", "1")
                .env("YAVS_AUTO_COUNT", "1")
                .env("YAVS_AUTO_GATE", "0")
                .env("YAVS_AUTO_SYNC", "1")
                .env("YAVS_SPEC_ALL", "1")
                .env_remove("YAVS_AUTO_POLICY")
                .current_dir(&d)
                .output()
                .expect("run yavs");
            assert!(o.status.success(), "{t}: {o:?}");
            o.stdout
        };
        let want = run("off", &["--tier=interp", "--stop-time=120ns"]);
        assert_eq!(
            run("all", &["--tier=clift-auto", "--stop-time=120ns"]),
            want,
            "{t}"
        );
    }
}
