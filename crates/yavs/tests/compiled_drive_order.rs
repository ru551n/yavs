//! Kept scalar drives (`Batch`) and vector drives of driver slots that elaboration merged into one
//! kernel driver (`a(i)`, `a(7 downto 4)`, `a(3 downto 0)` of one process) keep their order, so
//! the last assignment of an element wins; slots of different kernel drivers (a byte and a bit
//! per lane) are driven alike. Both against the interpreter under each code generator (#766: a
//! batch kept across such a vector drive was measured and not kept, decisions.md "Compiled
//! clocked RTL processes, round 11"). Needs the `llvm` or the `clift` feature.
#![cfg(any(feature = "llvm", feature = "clift"))]

mod common;
use common::tempdir;

use std::path::Path;
use std::process::Command;

/// `merged`: `a(i)`, `a(7 downto 4)` and `a(3 downto 0)` are three slots of one kernel driver
/// (elaboration merges adjacent ranges of a process), and a vector assignment between two element
/// assignments must overwrite the first. `apart`: the vector
/// `w` and the elements of `v` are drivers of their own, a byte and a bit per lane as a
/// repacking stage drives them.
const SRC: &str = r#"library ieee; use ieee.std_logic_1164.all;
entity merged is
  port (clk : in std_logic; a, b : out std_logic_vector(7 downto 0));
end;
architecture rtl of merged is
begin
  process (clk)
    variable k : natural := 0;
  begin
    if rising_edge(clk) then
      for i in 0 to 7 loop
        b(i) <= '1';
        if (i + k) mod 3 = 0 then a(i) <= 'H'; end if;
        a(7 downto 4) <= "0000";
        a(3 downto 0) <= "0000";
        if (i + k) mod 5 = 0 then a(i) <= 'L'; end if;
      end loop;
      k := k + 1;
    end if;
  end process;
end;
library ieee; use ieee.std_logic_1164.all;
entity apart is
  port (clk : in std_logic; v : out std_logic_vector(7 downto 0); w : out std_logic_vector(63 downto 0));
end;
architecture rtl of apart is
begin
  process (clk)
    variable k : natural := 0;
  begin
    if rising_edge(clk) then
      for i in 0 to 7 loop
        w(8 * i + 7 downto 8 * i) <= (others => '0') when (i + k) mod 2 = 0 else x"A5";
        v(i) <= '1' when (i + k) mod 3 = 0 else '0';
      end loop;
      k := k + 1;
    end if;
  end process;
end;
library ieee; use ieee.std_logic_1164.all;
entity top is end;
architecture t of top is
  signal clk : std_logic := '0';
  signal a, b, v : std_logic_vector(7 downto 0);
  signal w : std_logic_vector(63 downto 0);
begin
  m: entity work.merged port map (clk, a, b);
  p: entity work.apart port map (clk, v, w);
  process begin
    for c in 1 to 12 loop
      clk <= '1'; wait for 5 ns; clk <= '0'; wait for 5 ns;
      report "a=" & to_string(a) & " b=" & to_string(b) & " v=" & to_string(v) & " w=" & to_hstring(w);
    end loop;
    wait;
  end process;
end;
"#;

fn run(dir: &Path, flags: &[&str]) -> String {
    let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(["run", "--work=work:work"])
        .args(flags)
        .arg("top")
        .current_dir(dir)
        .output()
        .expect("run yavs");
    assert!(o.status.success(), "{flags:?}: {o:?}");
    String::from_utf8_lossy(&o.stdout).into_owned()
}

#[test]
fn kept_scalar_drives_and_vector_drives_of_merged_slots_keep_their_order() {
    let dir = tempdir("compiled_drive_order");
    std::fs::write(dir.join("t.vhd"), SRC).expect("write");
    let a = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .args(["analyze", "--std=2008", "--work=work:work", "t.vhd"])
        .current_dir(&dir)
        .output()
        .expect("run yavs");
    assert!(a.status.success(), "{a:?}");
    let want = run(&dir, &["--tier=interp"]);
    assert!(want.contains('L') && !want.contains('H'), "{want}");
    for flags in [
        #[cfg(feature = "llvm")]
        vec!["--tier=llvm"],
        #[cfg(feature = "llvm")]
        vec!["--tier=auto", "--tier-heat=1,0,0"],
        #[cfg(feature = "clift")]
        vec!["--tier=clift"],
        #[cfg(feature = "clift")]
        vec!["--tier=clift-auto", "--tier-heat=1,0,0"],
    ] {
        assert_eq!(run(&dir, &flags), want, "{flags:?}");
    }
}
