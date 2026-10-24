//! The wait of a process with a sensitivity list in compiled code (`--tier=llvm`, `--tier=auto`
//! compiling everything at once; a build with the `llvm` feature): the output is the
//! interpreter's, and a wait that names the same signals at every activation is skipped when the
//! kernel still holds it, also when the optimiser gave the process two copies of that wait (one
//! that keeps the signal references for the other, `ir::opt::memo`).
#![cfg(feature = "llvm")]

mod common;
use common::tempdir;

use std::path::Path;
use std::process::{Command, Output};

/// A sensitivity list of array slices and scalars, a variable of generic width in the body.
const SRC: &str = r#"library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity waits is generic (N : natural := 8); end;
architecture a of waits is
  signal a, b, c : std_logic_vector(N-1 downto 0) := (others => '0');
  signal d, e : std_logic := '0';
  signal q : std_logic_vector(N-1 downto 0) := (others => '0');
  signal cnt : natural := 0;
begin
  process (a(N-1 downto 0), b(N-1 downto 1), c, d, e)
    variable x : std_logic_vector(N-1 downto 0);
  begin
    x := a xor b xor c;
    if d = '1' then x := not x; end if;
    q <= x;
    if e = '1' then x := x and a; end if;
    q <= q xor x;
  end process;
  process begin
    for i in 1 to 2000 loop
      a <= std_logic_vector(to_unsigned(i, N)); wait for 1 ns;
      b <= std_logic_vector(to_unsigned(i*3, N)); c <= not a; d <= not d;  e <= not e; wait for 1 ns;
    end loop;
    report "q=" & to_hstring(q);
    wait;
  end process;
end;
"#;

fn run(dir: &Path, flags: &[&str]) -> (Output, Vec<String>) {
    let mut args = vec!["run", "--work=work:work"];
    args.extend_from_slice(flags);
    args.push("waits");
    let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(&args)
        .env("YAVS_COUNTERS", "kinds")
        .current_dir(dir)
        .output()
        .expect("run yavs");
    assert!(o.status.success(), "{o:?}");
    let census = String::from_utf8_lossy(&o.stderr)
        .lines()
        .filter(|l| l.starts_with("counters: cb_") && l.contains('.'))
        .map(str::to_string)
        .collect();
    (o, census)
}

#[test]
fn sensitivity_lists_wait_without_naming_their_signals() {
    let dir = tempdir("compiled_waits");
    std::fs::write(dir.join("waits.vhd"), SRC).expect("write");
    let a = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(["analyze", "--std=2008", "--work=work:work", "waits.vhd"])
        .current_dir(&dir)
        .output()
        .expect("analyze");
    assert!(a.status.success(), "{a:?}");
    let (want, _) = run(&dir, &["--tier=interp"]);
    let want = String::from_utf8_lossy(&want.stdout).into_owned();
    assert!(want.contains("q="), "{want}");
    for flags in [
        vec!["--tier=llvm"],
        vec!["--tier=auto", "--tier-heat=1,0,0"],
        vec!["--tier=auto", "--tier-heat=1,0,0", "--threads=4"],
        #[cfg(feature = "clift")]
        vec!["--tier=clift"],
        #[cfg(feature = "clift")]
        vec!["--tier=auto3", "--tier-heat=1,0,0"],
    ] {
        let (o, census) = run(&dir, &flags);
        assert_eq!(String::from_utf8_lossy(&o.stdout), want, "{flags:?}");
        // 4000 activations of the process, 5 signals in its list: the waits are not named again
        // (the first ones, before the kernel holds a wait, are).
        let waits: u64 = census
            .iter()
            .filter(|l| l.contains(".WaitOn="))
            .filter_map(|l| l.split_once('=')?.1.parse::<u64>().ok())
            .sum();
        assert!(waits < 100, "{flags:?}: {census:?}");
    }
}
