//! Shape folded from what an instance fixes: temporaries of a process with nested subprograms in
//! its frame, a length check that folds true or fails with its own text, and stored copies that
//! never serve an instance with other generics.
#![cfg(feature = "clift")]

mod common;
use common::tempdir;

use std::path::Path;
use std::process::Command;

const DESIGN: &str = r#"
entity sub is
  generic (w : natural := 9; bad : boolean := false);
  port (clk : in bit; q : out bit_vector(w - 1 downto 0));
end entity;

architecture a of sub is
  signal chk : bit_vector(w - 1 downto 0);
begin
  q <= chk;
  process (clk)
    variable col : bit_vector(w - 1 downto 0) := (others => '0');
    variable n : natural := 0;
    variable sum : natural := 0;
    procedure bump (v : in bit_vector) is
    begin
      -- Reads and writes the frame of the process while its temporaries are alive.
      n := n + 1;
      for i in v'range loop
        if v(i) = '1' then
          sum := (sum * 3 + i + 1) mod 1000003;
        end if;
      end loop;
    end procedure;
    function width_of (v : bit_vector) return natural is
    begin
      return v'length;
    end function;
    function make return bit_vector is
      variable r : bit_vector(w downto 0) := (others => '1');
    begin
      return r;
    end function;
  begin
    if clk = '1' then
      bump(col & '1');
      bump(not col);
      col(n mod w) := not col(n mod w);
      chk <= col;
      if bad and n = 8 then
        col := make;
      end if;
      if n mod 16 = 0 then
        report integer'image(w) & " " & integer'image(sum) & " " & integer'image(width_of(col));
      end if;
    end if;
  end process;
end architecture;

entity top is
  generic (wa : natural := 9; wb : natural := 5; bad : boolean := false);
end entity;

architecture a of top is
  signal clk : bit := '0';
  signal qa : bit_vector(wa - 1 downto 0);
  signal qb : bit_vector(wb - 1 downto 0);
begin
  ua : entity work.sub generic map (wa, bad) port map (clk, qa);
  ub : entity work.sub generic map (wb, false) port map (clk, qb);
  clk <= not clk after 5 ns;
  process
  begin
    wait for 1 us;
    report "end " & integer'image(wa) & " " & integer'image(wb);
    std.env.finish;
  end process;
end architecture;
"#;

fn yavs(dir: &Path, env: &[(&str, &str)], args: &[&str]) -> (i32, String, String) {
    let o = Command::new(env!("CARGO_BIN_EXE_yavs"))
        .current_dir(dir)
        .envs(env.iter().copied())
        .args(["--color", "never"])
        .args(args)
        .output()
        .expect("run yavs");
    (
        o.status.code().unwrap_or(-1),
        String::from_utf8_lossy(&o.stdout).into_owned(),
        String::from_utf8_lossy(&o.stderr).into_owned(),
    )
}

const SPEC: (&str, &str) = ("YAVS_SPEC_AFTER", "2");
const EAGER: [&str; 2] = ["--tier=auto", "--tier-heat=1,0,0"];

fn prepared(name: &str) -> (common::Tmp, String) {
    let d = tempdir(name);
    std::fs::write(d.join("t.vhd"), DESIGN).expect("write");
    let work = format!("work:{}", d.join("lib").display());
    let a = yavs(
        &d,
        &[],
        &["analyze", "--std=2008", "--work", &work, "t.vhd"],
    );
    assert_eq!(a.0, 0, "{}", a.2);
    (d, work)
}

fn run(
    d: &Path,
    work: &str,
    env: &[(&str, &str)],
    tier: &[&str],
    extra: &[&str],
    gens: &[&str],
) -> (i32, String, String) {
    let mut args = vec!["run", "--std=2008", "--work", work];
    args.extend(tier);
    args.extend(extra);
    args.extend(["--threads", "1", "top"]);
    args.extend(gens);
    yavs(d, env, &args)
}

/// A stored copy is made for its instance data: a warm run with other generics gets the results
/// of a cold run with those generics, not those of the instance the store was filled for.
#[test]
fn a_warm_run_with_other_generics_is_a_cold_run_with_them() {
    let (d, work) = prepared("shape_facts_warm");
    let other = ["-gwa=11", "-gwb=4"];
    let expected = run(&d, &work, &[], &["--tier=interp"], &["--no-cache"], &other);
    assert_eq!(expected.0, 0, "{}", expected.2);
    assert!(expected.2.contains("end 11 4") || expected.1.contains("end 11 4"));
    let first = run(&d, &work, &[SPEC], &EAGER, &[], &[]);
    assert_eq!(first.0, 0, "{}", first.2);
    let warm = run(&d, &work, &[SPEC], &EAGER, &[], &other);
    assert_eq!(
        (&expected.0, &expected.1, &expected.2),
        (&warm.0, &warm.1, &warm.2)
    );
    // And back: the first entry still serves its own generics.
    let base = run(&d, &work, &[], &["--tier=interp"], &["--no-cache"], &[]);
    let again = run(&d, &work, &[SPEC], &EAGER, &[], &[]);
    assert_eq!(
        (&base.0, &base.1, &base.2),
        (&again.0, &again.1, &again.2)
    );
}

/// Every tier gives what the interpreter gives, and a length that does not match fails at its
/// own assignment with its own text.
#[test]
fn folded_shapes_change_no_result_and_no_error_text() {
    let (d, work) = prepared("shape_facts_tiers");
    let interp = run(&d, &work, &[], &["--tier=interp"], &["--no-cache"], &[]);
    assert_eq!(interp.0, 0, "{}", interp.2);
    let eager = run(&d, &work, &[SPEC], &EAGER, &["--no-cache"], &[]);
    assert_eq!(
        (&interp.0, &interp.1, &interp.2),
        (&eager.0, &eager.1, &eager.2)
    );
    let g = ["-gbad=true"];
    let interp = run(&d, &work, &[], &["--tier=interp"], &["--no-cache"], &g);
    assert_ne!(interp.0, 0, "the length must fail: {}", interp.2);
    assert!(interp.2.contains("length"), "{}", interp.2);
    let eager = run(&d, &work, &[SPEC], &EAGER, &["--no-cache"], &g);
    assert_eq!(
        (&interp.0, &interp.1, &interp.2),
        (&eager.0, &eager.1, &eager.2)
    );
}
