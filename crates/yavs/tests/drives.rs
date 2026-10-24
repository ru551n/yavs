//! Repeated drives of one driver in an activation (`opt/drives.rs`, #667): a combinational
//! process that drives its outputs with defaults and overrides some of them drives the same
//! driver twice, and the first drive is done only where no second one follows. Every design runs
//! with the optimisation and without it (`YAVS_NO_DRIVES`) and must print the same; the first
//! one also drives less.

mod common;
use common::tempdir as tmp;

use std::process::Command;

/// (what the process does, its statements, its wait: sensitivity list or empty for `wait on`)
const CASES: &[(&str, &str)] = &[
    (
        "a default and an if",
        "x <= '0'; y <= '1'; if c = '1' then x <= d; end if; if d = '1' then y <= '0'; end if;",
    ),
    (
        "a default and a case with a null arm",
        "x <= '0'; y <= '1'; case sel is when 0 => x <= d; when 1 => y <= '0'; x <= '1'; when 2 => null; when others => y <= d; end case;",
    ),
    (
        "a vector default and an override of a slice",
        "v <= \"0000\"; if c = '1' then v(1 downto 0) <= \"11\"; end if; if d = '1' then v <= \"1010\"; end if;",
    ),
    (
        "a vector default and an override of the whole vector",
        "v <= \"0000\"; if d = '1' then v <= \"1010\"; end if; if c = '1' and sel = 2 then v <= \"0110\"; end if;",
    ),
    (
        "a default and a delayed override",
        "x <= '0'; if c = '1' then x <= d after 2 ns; end if; y <= '1'; if d = '1' then y <= '0' after 1 ns; end if;",
    ),
    (
        "a default and a transport override with a delay",
        "x <= '0'; if c = '1' then x <= transport d after 1500 ps; end if;",
    ),
    (
        "a default and an inertial override with a reject limit",
        "x <= '1'; if c = '1' then x <= reject 500 ps inertial d after 1 ns; end if;",
    ),
    (
        "a default, a report and an override",
        "x <= '0'; report \"x\" & std_logic'image(c); if c = '1' then x <= d; end if;",
    ),
    (
        "a default, a call of a procedure that drives it and an override",
        "x <= '0'; push(x, d); if c = '1' then x <= '1'; end if;",
    ),
    (
        "a default, a call of a procedure that drives another signal and an override",
        "x <= '0'; push(y, d); if c = '1' then x <= '1'; end if;",
    ),
    (
        "a default and 'driving_value read between",
        "x <= '0'; z <= x'driving_value; if c = '1' then x <= d; end if;",
    ),
    (
        "a default, two overrides in a row and a loop",
        "x <= '0'; for i in 0 to 3 loop if sel = i then x <= '1'; x <= d; end if; end loop;",
    ),
    (
        "a record of fields set one by one",
        "r <= (a => '0', b => \"00\"); if c = '1' then r.a <= d; end if; if d = '1' then r.b <= \"11\"; end if; case sel is when 0 => r <= (a => '1', b => \"01\"); when others => null; end case;",
    ),
    (
        "defaults in a loop over an array of records, some enabled",
        "for i in 0 to 3 loop q(i) <= (a => '0', b => \"00\"); if en(i) then q(i) <= (a => d, b => \"10\"); q(i).a <= c and d; end if; end loop;",
    ),
    (
        "a default and an override in a nested if with an elsif",
        "x <= '0'; if c = '1' then if d = '1' then x <= '1'; else y <= '0'; end if; elsif sel = 2 then x <= 'Z'; end if; y <= '1';",
    ),
];

const HEAD: &str = "
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity t is end;
architecture a of t is
  type rec_t is record
    a : std_logic;
    b : std_logic_vector(1 downto 0);
  end record;
  type rec_arr_t is array (0 to 3) of rec_t;
  type en_t is array (0 to 3) of boolean;
  constant en : en_t := (true, false, true, true);
  signal sel : integer range 0 to 3 := 0;
  signal c, d, x, y, z : std_logic := '0';
  signal v : std_logic_vector(3 downto 0) := \"0000\";
  signal r : rec_t := (a => '0', b => \"00\");
  signal q : rec_arr_t;
  procedure push(signal s : out std_logic; value : std_logic) is
  begin
    s <= value after 1 ns;
  end procedure;
begin
  comb : process (c, d, sel)
  begin
    BODY
  end process;
  stim : process
    function img(q : rec_arr_t) return string is
    begin
      return std_logic'image(q(0).a) & std_logic'image(q(1).a) & std_logic'image(q(2).a) & std_logic'image(q(3).a)
        & std_logic'image(q(0).b(1)) & std_logic'image(q(1).b(0)) & std_logic'image(q(2).b(1)) & std_logic'image(q(3).b(0));
    end function;
  begin
    for i in 0 to 31 loop
      sel <= (i / 2) mod 4;
      c <= '1' when (i / 3) mod 2 = 1 else '0';
      d <= '1' when i mod 5 < 2 else '0';
      for k in 1 to 4 loop
        wait for 500 ps;
        report integer'image(i) & \".\" & integer'image(k)
          & \" x=\" & std_logic'image(x) & \" y=\" & std_logic'image(y) & \" z=\" & std_logic'image(z)
          & \" v=\" & std_logic'image(v(3)) & std_logic'image(v(2)) & std_logic'image(v(1)) & std_logic'image(v(0))
          & \" r=\" & std_logic'image(r.a) & std_logic'image(r.b(1)) & std_logic'image(r.b(0))
          & \" q=\" & img(q);
      end loop;
    end loop;
    wait;
  end process;
end;
";

fn run(dir: &std::path::Path, env: &[(&str, &str)]) -> (i32, String, Option<u64>) {
    let yavs = env!("CARGO_BIN_EXE_yavs");
    let work = format!("work:{}", dir.join("lib").display());
    let a = Command::new(yavs)
        .current_dir(dir)
        .envs(env.iter().copied())
        .args(["analyze", "--std=2008", "--work", &work, "t.vhd"])
        .output()
        .expect("analyze");
    assert!(a.status.success(), "{}", String::from_utf8_lossy(&a.stderr));
    let o = Command::new(yavs)
        .current_dir(dir)
        .envs(env.iter().copied())
        .env("YAVS_COUNTERS", "1")
        .args([
            "--color",
            "never",
            "run",
            "--no-cache",
            "--std=2008",
            "--work",
            &work,
        ])
        .args(["--tier=interp", "--threads", "1", "t"])
        .output()
        .expect("run");
    let err = String::from_utf8_lossy(&o.stderr);
    // The drives of scalar logic and of vectors that took the kernel's direct paths.
    let drives = ["direct_codes", "direct_planes", "direct_bytes"]
        .iter()
        .map(|k| {
            err.lines()
                .find_map(|l| l.strip_prefix(&format!("counters: {k}=")))
                .and_then(|n| n.trim().parse::<u64>().ok())
        })
        .sum::<Option<u64>>();
    (
        o.status.code().unwrap_or(-1),
        format!(
            "{}{}",
            String::from_utf8_lossy(&o.stdout),
            err.lines()
                .filter(|l| !l.starts_with("counters:"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
        drives,
    )
}

#[test]
fn a_default_drive_that_is_overridden_changes_nothing_anyone_sees() {
    let mut fewer = 0;
    for (what, body) in CASES {
        let d = tmp("drives");
        std::fs::write(d.join("t.vhd"), HEAD.replace("BODY", body)).expect("write");
        // One kernel signal per leaf: record packs (#686) merge drives of fields by themselves.
        let with = run(&d, &[("YAVS_PACKS", "off")]);
        let without = run(&d, &[("YAVS_NO_DRIVES", "1"), ("YAVS_PACKS", "off")]);
        assert_eq!((&with.0, &with.1), (&without.0, &without.1), "{what}");
        assert_eq!(with.0, 0, "{what}: {}", with.1);
        if let (Some(a), Some(b)) = (with.2, without.2) {
            assert!(a <= b, "{what}: {a} drives with the pass, {b} without");
            fewer += usize::from(a < b);
        }
    }
    // Counters are printed by release and debug builds alike.
    assert!(
        // The designs that drive a whole record, or an element of an array of records, do it with
        // one instruction (`SigScatter`), which the pass leaves alone.
        fewer >= 6,
        "only {fewer} of {} designs drive less",
        CASES.len()
    );
}
