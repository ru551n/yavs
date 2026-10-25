//! A large `std_logic` array (default `U`, code 0) is a zeroed allocation: untouched pages cost
//! no memory (#486). Linux only: the peak resident size is read from `/proc` while the run is busy.
#![cfg(target_os = "linux")]

mod common;
use common::tempdir as tmp;

use std::process::{Command, Stdio};

const SRC: &str = "
library ieee; use ieee.std_logic_1164.all;
entity big is end;
architecture a of big is
  type bv_ptr is access std_logic_vector;
begin
  process
    variable p : bv_ptr := new std_logic_vector(0 to 2 ** 29);
    variable n : natural := 0;
  begin
    p(12345) := '1';
    for i in 1 to 20000000 loop
      n := (n + i) mod 1000;
    end loop;
    assert p(12345) = '1' and p(777) = 'U' and n < 1000;
    report \"done\";
    wait;
  end process;
end;
";

fn rss_kb(pid: u32) -> Option<u64> {
    let s = std::fs::read_to_string(format!("/proc/{pid}/status")).ok()?;
    let l = s.lines().find(|l| l.starts_with("VmHWM:"))?;
    l.split_whitespace().nth(1)?.parse().ok()
}

#[test]
fn a_zero_filled_array_costs_only_the_pages_touched() {
    let d = tmp("rss");
    std::fs::write(d.join("big.vhd"), SRC).expect("write");
    let work = format!("work:{}", d.join("lib").display());
    let yavs = env!("CARGO_BIN_EXE_yavs");
    let a = Command::new(yavs)
        .current_dir(&*d)
        .args(["analyze", "--work", &work, "big.vhd"])
        .output()
        .expect("analyze");
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    let mut c = Command::new(yavs)
        .current_dir(&*d)
        .args([
            "--color",
            "never",
            "run",
            "--no-cache",
            "--work",
            &work,
            "big",
        ])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("run");
    let mut peak = 0;
    while c.try_wait().expect("wait").is_none() {
        peak = peak.max(rss_kb(c.id()).unwrap_or(0));
        std::thread::sleep(std::time::Duration::from_millis(2));
    }
    assert!(c.wait().expect("wait").success());
    assert!(peak > 0, "peak resident size never read");
    assert!(
        peak < 200_000,
        "peak resident size {peak} kB for a 512 MiB array"
    );
}

/// A process without a wait lists an entry per assignment for its whole activation unless it is
/// trimmed (#761): 12 million assignments of `a <= not a` stay within a few tens of MB.
#[test]
fn a_loop_without_a_wait_keeps_memory_flat() {
    let src = "
entity flat is end;
architecture a of flat is
  signal a : bit := '0';
begin
  process
    variable n : natural := 0;
  begin
    a <= not a;
    n := n + 1;
    assert n < 12000000 report \"done\" severity failure;
  end process;
end;
";
    let tiers: &[&str] = if cfg!(feature = "llvm") {
        &["--tier=interp", "--tier=llvm"]
    } else {
        &["--tier=interp"]
    };
    for tier in tiers {
        let d = tmp("flat");
        std::fs::write(d.join("flat.vhd"), src).expect("write");
        let work = format!("work:{}", d.join("lib").display());
        let yavs = env!("CARGO_BIN_EXE_yavs");
        let a = Command::new(yavs)
            .current_dir(&*d)
            .args(["analyze", "--work", &work, "flat.vhd"])
            .output()
            .expect("analyze");
        assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
        let mut c = Command::new(yavs)
            .current_dir(&*d)
            .args([
                "--color",
                "never",
                "run",
                "--no-cache",
                tier,
                "--work",
                &work,
                "flat",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("run");
        let mut peak = 0;
        while c.try_wait().expect("wait").is_none() {
            peak = peak.max(rss_kb(c.id()).unwrap_or(0));
            std::thread::sleep(std::time::Duration::from_millis(2));
        }
        c.wait().expect("wait");
        assert!(peak > 0, "{tier}: peak resident size never read");
        // Untrimmed, the entries alone add 290 MB.
        assert!(peak < 220_000, "{tier}: peak resident size {peak} kB");
    }
}
