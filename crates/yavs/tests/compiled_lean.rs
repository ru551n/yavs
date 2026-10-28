//! Lean activations (#766, `yavs_llvm::regs::no_table`): a compiled process that calls nothing and
//! reaches no heap, package or shared object skips the object table's tags at every activation,
//! under each code generator and every clock-edge form; a process that calls a subprogram does
//! not. A debug build checks that a lean activation never reaches the table (it panics). Needs
//! the `llvm` or the `clift` feature.
#![cfg(any(feature = "llvm", feature = "clift"))]

mod common;
use common::tempdir;

use std::path::Path;
use std::process::Command;

/// A design of one clocked process (`BODY`, which counts into `n`) and a clock of 20 edges.
fn design(body: &str) -> String {
    format!(
        r#"library ieee; use ieee.std_logic_1164.all;
entity lean is end;
architecture a of lean is
  signal clk : std_logic := '0';
  signal n : integer := 0;
  signal r : std_logic_vector(15 downto 0) := x"ACE1";
  type ptr is access integer;
  procedure bump(signal s : inout integer) is
    variable p : ptr := new integer'(1);
  begin
    s <= s + p.all;
    deallocate(p);
  end;
begin
  process begin
    for i in 1 to 20 loop clk <= '1'; wait for 5 ns; clk <= '0'; wait for 5 ns; end loop;
    report "n = " & integer'image(n) & " r = " & to_hstring(r);
    wait;
  end process;
  {body}
end;
"#
    )
}

/// The processes, each a design of its own: lean ones (every edge form, a vector shift through a
/// constant) and one that calls a procedure (not lean).
const LEAN: [&str; 5] = [
    "process(clk) begin if rising_edge(clk) then n <= n + 1; end if; end process;",
    "process(clk) begin if clk'event and clk = '1' then n <= n + 1; end if; end process;",
    "process(clk) begin if not clk'stable and clk = '1' then n <= n + 1; end if; end process;",
    "process begin wait until rising_edge(clk); n <= n + 1; end process;",
    "process(clk) begin if rising_edge(clk) then \
       if r(0) = 'U' then r <= x\"ACE1\"; else r <= r(14 downto 0) & (r(15) xor r(13)); end if; \
       n <= n + 1; end if; end process;",
];
const CALLS: &str = "process(clk) begin if rising_edge(clk) then bump(n); end if; end process;";

/// The report line of a run and its count of lean activations (`cb_lean`, debug builds).
fn run(dir: &Path, flags: &[&str]) -> (String, u64) {
    let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(["run", "--work=work:work"])
        .args(flags)
        .arg("lean")
        .env("YAVS_COUNTERS", "1")
        .current_dir(dir)
        .output()
        .expect("run yavs");
    assert!(o.status.success(), "{flags:?}: {o:?}");
    let out = String::from_utf8_lossy(&o.stdout)
        .lines()
        .find(|l| l.contains("n = "))
        .unwrap_or_default()
        .to_string();
    let lean = String::from_utf8_lossy(&o.stderr)
        .lines()
        .find_map(|l| l.strip_prefix("counters: cb_lean="))
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(0);
    (out, lean)
}

fn tiers() -> Vec<Vec<&'static str>> {
    vec![
        #[cfg(feature = "llvm")]
        vec!["--tier=llvm"],
        #[cfg(feature = "llvm")]
        vec!["--tier=auto", "--tier-heat=1,0,0"],
        #[cfg(feature = "clift")]
        vec!["--tier=clift"],
        #[cfg(feature = "clift")]
        vec!["--tier=clift-auto", "--tier-heat=1,0,0"],
    ]
}

#[test]
fn clocked_processes_without_calls_or_heap_run_lean() {
    for (k, body) in LEAN.iter().chain([&CALLS]).enumerate() {
        let dir = tempdir(&format!("compiled_lean_{k}"));
        std::fs::write(dir.join("lean.vhd"), design(body)).expect("write");
        let a = Command::new(env!("CARGO_BIN_EXE_yavs"))
            .args(["analyze", "--std=2008", "--work=work:work", "lean.vhd"])
            .current_dir(&dir)
            .output()
            .expect("run yavs");
        assert!(a.status.success(), "{a:?}");
        let (want, _) = run(&dir, &["--tier=interp"]);
        assert!(want.contains("n = 20"), "{body}: {want}");
        for flags in tiers() {
            let (got, lean) = run(&dir, &flags);
            assert_eq!(got, want, "{body} {flags:?}");
            if cfg!(debug_assertions) {
                if k < LEAN.len() {
                    assert!(lean > 0, "{body} {flags:?}: no lean activation");
                } else {
                    assert_eq!(lean, 0, "{body} {flags:?}: a process with a call ran lean");
                }
            }
        }
    }
}
