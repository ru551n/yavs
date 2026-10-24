//! Clock-edge idioms in compiled code (`--tier=llvm`, `--tier=auto` compiling everything at once;
//! a build with the `llvm` feature) against the interpreter: every form counts the same edges,
//! and the forms that mean different things still do (`rising_edge` is not `'event and = '1'`:
//! it needs a `0` or `L` before the `1`).
#![cfg(feature = "llvm")]

mod common;
use common::tempdir;

use std::path::Path;
use std::process::{Command, Output};

/// One process per idiom, counting its edges in `n`; the stimulus drives the clock through
/// `X`, `U`, `L` and `H` as well, and resets and a second clock in between. Slot of each form:
/// 1 `rising_edge`, 2 `clk'event and clk = '1'`, 3 `falling_edge`, 4 and 5 the same behind an
/// asynchronous reset, 6 and 7 on a `std_ulogic`, 8 on a `bit`, 9 two edges ored, 10 `wait until
/// rising_edge`, 11 `wait until clk = '1'`, 12 `clk'event and clk = '0'`, 13 `not clk'stable and clk = '1'`, 14 and 15 `rising_edge` and `falling_edge` of a BIT, 16 a process of two signals that tests `clk = '1'` alone (no edge).
const SRC: &str = r#"library ieee; use ieee.std_logic_1164.all;
entity edges is end;
architecture a of edges is
  signal clk : std_logic := '0';
  signal cu : std_ulogic := 'U';
  signal cb : bit := '0';
  signal rst : std_logic := '1';
  type cnt_t is array (1 to 16) of integer;
  signal n : cnt_t := (others => 0);
begin
  process begin
    wait for 3 ns; rst <= '0';
    for i in 1 to 40 loop
      clk <= '1'; cb <= '1'; wait for 5 ns;
      case i mod 5 is
        when 0 => clk <= 'X'; wait for 1 ns; clk <= '1'; wait for 4 ns; clk <= 'L'; cu <= 'H'; wait for 5 ns; clk <= 'H'; cu <= '0'; wait for 5 ns;
        when 1 => clk <= 'U'; wait for 1 ns; clk <= '0'; wait for 4 ns;
        when 2 => rst <= '1'; clk <= '0'; cb <= '0'; wait for 5 ns; rst <= '0'; wait for 1 ns;
        when others => clk <= '0'; cb <= '0'; wait for 5 ns;
      end case;
      cu <= '1'; wait for 1 ns; cu <= '0';
    end loop;
    wait for 10 ns;
    for k in n'range loop report integer'image(k) & " = " & integer'image(n(k)); end loop;
    wait;
  end process;
  p1: process(clk) begin if rising_edge(clk) then n(1) <= n(1) + 1; end if; end process;
  p2: process(clk) begin if clk'event and clk = '1' then n(2) <= n(2) + 1; end if; end process;
  p3: process(clk) begin if falling_edge(clk) then n(3) <= n(3) + 1; end if; end process;
  p4: process(clk, rst) begin if rst = '1' then n(4) <= 0; elsif rising_edge(clk) then n(4) <= n(4) + 1; end if; end process;
  p5: process(clk, rst) begin if rst = '1' then n(5) <= 100; elsif falling_edge(clk) then n(5) <= n(5) + 1; end if; end process;
  p6: process(cu) begin if rising_edge(cu) then n(6) <= n(6) + 1; end if; end process;
  p7: process(cu) begin if falling_edge(cu) then n(7) <= n(7) + 1; end if; end process;
  p8: process(cb) begin if cb'event and cb = '1' then n(8) <= n(8) + 1; end if; end process;
  p9: process(clk, cu) begin if rising_edge(clk) or falling_edge(cu) then n(9) <= n(9) + 1; end if; end process;
  p10: process begin wait until rising_edge(clk); n(10) <= n(10) + 1; end process;
  p11: process begin wait until clk = '1'; n(11) <= n(11) + 1; end process;
  p12: process(clk) begin if clk'event and clk = '0' then n(12) <= n(12) + 1; end if; end process;
  p13: process(clk) begin if not clk'stable and clk = '1' then n(13) <= n(13) + 1; end if; end process;
  p14: process(cb) begin if rising_edge(cb) then n(14) <= n(14) + 1; end if; end process;
  p15: process(cb) begin if falling_edge(cb) then n(15) <= n(15) + 1; end if; end process;
  p16: process(clk, cu) begin if clk = '1' then n(16) <= n(16) + 1; end if; end process;
end;
"#;

fn yavs(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(args)
        .current_dir(dir)
        .output()
        .expect("run yavs")
}

/// The edge counts of a run and its `cb_<entry>.<Kind>` counter lines (the calls compiled code made
/// into the host, `YAVS_COUNTERS=kinds`).
fn counts(dir: &Path, flags: &[&str]) -> (Vec<(u32, u32)>, Vec<String>) {
    let mut args = vec!["run", "--work=work:work"];
    args.extend_from_slice(flags);
    args.push("edges");
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
    let counts = String::from_utf8_lossy(&o.stdout)
        .lines()
        .filter_map(|l| {
            let (k, n) = l.strip_prefix("** Note: ")?.split_once(" = ")?;
            Some((k.rsplit(' ').next()?.parse().ok()?, n.trim().parse().ok()?))
        })
        .collect();
    (counts, census)
}

#[test]
fn clock_edge_forms_in_compiled_code_match_the_interpreter() {
    let dir = tempdir("compiled_edges");
    std::fs::write(dir.join("edges.vhd"), SRC).expect("write");
    let a = yavs(
        &dir,
        &["analyze", "--std=2008", "--work=work:work", "edges.vhd"],
    );
    assert!(a.status.success(), "{a:?}");
    let (want, _) = counts(&dir, &["--tier=interp"]);
    assert_eq!(want.len(), 16, "{want:?}");
    let n = |k: usize| want[k - 1].1;
    // The forms that mean the same count the same; `rising_edge` is stricter than the event
    // form (an `X` or `U` before the `1` is no rising edge).
    assert_eq!(n(1), n(10), "{want:?}");
    assert_eq!(n(2), n(11), "{want:?}");
    // `not clk'stable` is `clk'event`; `rising_edge` of a BIT is the event ending in '1'.
    assert_eq!(n(2), n(13), "{want:?}");
    assert_eq!(n(8), n(14), "{want:?}");
    assert!(n(1) < n(2) && n(1) > 0 && n(3) > 0 && n(15) > 0, "{want:?}");
    for flags in [
        vec!["--tier=llvm"],
        vec!["--tier=auto", "--tier-heat=1,0,0"],
        vec!["--tier=auto", "--tier-heat=1,0,0", "--threads=4"],
        #[cfg(feature = "clift")]
        vec!["--tier=clift"],
        #[cfg(feature = "clift")]
        vec!["--tier=auto3", "--tier-heat=1,0,0"],
    ] {
        let (got, census) = counts(&dir, &flags);
        assert_eq!(got, want, "{flags:?}");
        // The edge tests and `'event` are answered in compiled code (the `Drive` and `SigReadScalar`
        // calls left are the integer counters).
        for l in &census {
            assert!(
                !l.contains(".SigAttr=") && !l.contains(".CallNative=") && !l.contains(".Edge="),
                "{flags:?}: {census:?}"
            );
        }
    }
}
