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

/// Peak resident size (kB) of running `top` from `src` on `tier`.
fn peak_of(src: &str, top: &str, tier: &str) -> u64 {
    let d = tmp(top);
    std::fs::write(d.join(format!("{top}.vhd")), src).expect("write");
    let work = format!("work:{}", d.join("lib").display());
    let yavs = env!("CARGO_BIN_EXE_yavs");
    let a = Command::new(yavs)
        .current_dir(&*d)
        .args(["analyze", "--work", &work, &format!("{top}.vhd")])
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
            top,
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
    assert!(c.wait().expect("wait").success(), "{tier}: run failed");
    assert!(peak > 0, "{tier}: peak resident size never read");
    peak
}

/// Compiled code frees a statement's temporaries at the next marker, except where a bounded few
/// wait for the suspension (`stmt_trims`): a loop with a 1 kB temporary per round runs 500 000
/// rounds, and a process whose generic-sized vectors make wave objects wakes a million times
/// (a few hundred bytes per activation wait for its suspension, which frees them).
#[cfg(feature = "clift")]
#[test]
fn compiled_temporaries_are_freed_in_loops_and_at_suspensions() {
    let looped = "
entity tloop is end;
architecture a of tloop is
  function mk(n : natural) return bit_vector is
    variable r : bit_vector(1 to 1000) := (others => '0');
  begin
    r(1) := bit'val(n mod 2);
    return r;
  end;
begin
  process
    variable n : natural := 0;
  begin
    for i in 1 to 500000 loop
      if mk(i)(1) = '1' then
        n := n + 1;
      end if;
    end loop;
    assert n = 250000 report \"count\" severity failure;
    wait;
  end process;
end;
";
    let waking = "
entity twake is generic (w : natural := 9); end;
architecture a of twake is
  signal c1, c2, c3, c4 : bit_vector(w - 1 downto 0) := (others => '0');
begin
  process
    variable col : bit_vector(w - 1 downto 0) := (others => '0');
    variable n : natural := 0;
  begin
    col(0) := not col(0);
    c1 <= col;
    c2 <= col;
    c3 <= col;
    c4 <= col;
    n := n + 1;
    if n >= 1000000 then
      wait;
    end if;
    wait for 1 ns;
  end process;
end;
";
    for (src, top) in [(looped, "tloop"), (waking, "twake")] {
        for tier in ["--tier=clift", "--tier=interp"] {
            let peak = peak_of(src, top, tier);
            eprintln!("{top} {tier}: peak {peak} kB");
            // Untrimmed, either would hold 300 MB or more.
            assert!(peak < 200_000, "{top} {tier}: peak resident size {peak} kB");
        }
    }
}
