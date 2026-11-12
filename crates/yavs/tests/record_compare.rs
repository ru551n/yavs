//! A whole record drive compared with the record's image (`gather.rs`, `docs/decisions.md` "Record
//! drive compare") changes nothing. Each design runs without images (the kernel decides every
//! leaf), with the compare after its warm-up, and from the first scatter with every skip held
//! against the kernel's full test, always in debug builds. Output and waves must be the same, and
//! the compare must fire: `scatter_spared` has a floor.

mod common;
use common::tempdir;

use std::path::Path;
use std::process::{Command, Output};

const OFF: &[(&str, &str)] = &[("YAVS_NO_IMAGE", "1")];
const CHECK: &[(&str, &str)] = &[("YAVS_IMAGE_AFTER", "1"), ("YAVS_IMAGE_CHECK", "1")];

fn yavs(dir: &Path, env: &[(&str, &str)], args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_yavs"))
        .current_dir(dir)
        .envs(env.iter().copied())
        .args(["--color", "never"])
        .args(args)
        .output()
        .expect("run yavs")
}

fn spared(err: &str) -> u64 {
    let key = "counters: scatter_spared=";
    err.lines()
        .find_map(|l| l.strip_prefix(key)?.parse().ok())
        .unwrap_or_else(|| panic!("no {key} in {err}"))
}

/// Analysed `file`, its library argument, and runs of `top` as (status, stdout, leaves spared).
struct Design {
    dir: common::Tmp,
    work: String,
    top: String,
}

impl Design {
    fn new(tag: &str, file: &Path, top: &str) -> Design {
        Design::new_in(tempdir(tag), file, top)
    }

    fn new_in(dir: common::Tmp, file: &Path, top: &str) -> Design {
        let work = format!("work:{}", dir.join("lib").display());
        let a = yavs(
            &dir,
            OFF,
            &[
                "analyze",
                "--std=2008",
                "--work",
                &work,
                file.to_str().unwrap(),
            ],
        );
        assert!(a.status.success(), "analyze: {a:?}");
        Design {
            dir,
            work,
            top: top.to_string(),
        }
    }

    fn run(&self, env: &[(&str, &str)], extra: &[&str]) -> (Option<i32>, String, u64) {
        let mut args = vec!["run", "--std=2008", "--work", &self.work];
        args.extend(extra);
        args.push(&self.top);
        let mut e = vec![("YAVS_COUNTERS", "1")];
        e.extend(env);
        let r = yavs(&self.dir, &e, &args);
        let err = String::from_utf8_lossy(&r.stderr);
        (
            r.status.code(),
            String::from_utf8_lossy(&r.stdout).into_owned(),
            spared(&err),
        )
    }
}

fn corpus(name: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../tests/corpus/held/{name}.vhd"))
}

/// Force, release, entries and waveforms of the run, delays, parts, resolved records, ports, a
/// procedure's drive, the deltas of a run that only holds: `held/held_record`.
#[test]
fn the_compare_changes_nothing_around_what_must_see_a_drive() {
    let d = Design::new("record_compare", &corpus("held_record"), "held_record");
    let interp = ["--tier=interp", "--no-cache"];
    let off = d.run(OFF, &interp);
    assert_eq!(off.0, Some(0), "{}", off.1);
    assert_eq!(off.2, 0, "no images, no compare");
    let warm = d.run(&[], &interp);
    assert_eq!((&off.0, &off.1), (&warm.0, &warm.1), "the default warm-up");
    assert!(warm.2 > 10_000, "only {} leaves spared", warm.2);
    let first = d.run(CHECK, &interp);
    assert_eq!(
        (&off.0, &off.1),
        (&first.0, &first.1),
        "from the first scatter"
    );
    assert!(first.2 > warm.2, "{} against {}", first.2, warm.2);
    for threads in ["--threads=4", "--threads=4 --shuffle=11"] {
        let mut args = interp.to_vec();
        args.extend(threads.split(' '));
        let got = d.run(CHECK, &args);
        // A shuffled run reports processes of one cycle in another order: compare sorted.
        let sorted = |s: &str| {
            let mut v: Vec<_> = s.lines().map(str::to_owned).collect();
            v.sort();
            v
        };
        assert_eq!(off.0, got.0, "{threads}");
        assert_eq!(sorted(&off.1), sorted(&got.1), "{threads}");
    }
}

#[test]
fn the_compare_changes_no_wave() {
    let d = Design::new("record_compare_w", &corpus("held_record"), "held_record");
    let (a, b) = (d.dir.join("off.fst"), d.dir.join("on.fst"));
    for (env, f) in [(OFF, &a), (CHECK, &b)] {
        let wave = format!("--wave={}", f.display());
        let r = d.run(env, &["--tier=interp", "--no-cache", &wave]);
        assert_eq!(r.0, Some(0), "{}", r.1);
    }
    let on = std::fs::read(&b).unwrap();
    assert!(on.len() > 1000);
    assert_eq!(std::fs::read(&a).unwrap(), on, "waves differ");
}

/// In process copies with facts, where a field's drive is kept in the batch of compiled code or
/// compared inline before the whole record's: both compares at once, both checked.
#[cfg(feature = "clift")]
#[test]
fn the_compare_and_the_compare_in_compiled_code_work_together() {
    let d = Design::new("record_compare_c", &corpus("held_record"), "held_record");
    let interp = d.run(OFF, &["--tier=interp", "--no-cache"]);
    assert_eq!(interp.0, Some(0), "{}", interp.1);
    let env = [
        ("YAVS_IMAGE_AFTER", "1"),
        ("YAVS_IMAGE_CHECK", "1"),
        ("YAVS_INLCMP", "check"),
        ("YAVS_SPEC_ALL", "1"),
        ("YAVS_SPEC_AFTER", "2"),
    ];
    let heat = ["--tier=clift-auto", "--tier-heat=1,0,0", "--no-cache"];
    let got = d.run(&env, &heat);
    assert_eq!((&interp.0, &interp.1), (&got.0, &got.1), "compiled");
    assert!(got.2 > 10_000, "only {} leaves spared", got.2);
    let mut args = vec!["run", "--std=2008", "--work", &d.work];
    args.extend(heat);
    args.push(&d.top);
    let mut e = env.to_vec();
    e.push(("YAVS_COUNTERS", "1"));
    let r = yavs(&d.dir, &e, &args);
    let err = String::from_utf8_lossy(&r.stderr);
    assert!(
        err.contains("counters: gate_checked="),
        "no inline compare: {err}"
    );
    let forced = d.run(CHECK, &["--tier=clift", "--no-cache"]);
    assert_eq!(
        (&interp.0, &interp.1),
        (&forced.0, &forced.1),
        "forced compiled"
    );
    assert!(forced.2 > 10_000, "forced: only {} leaves spared", forced.2);
}

/// A counter of the run in `err`, 0 when it is not printed.
#[cfg(feature = "clift")]
fn counter(err: &str, name: &str) -> u64 {
    let key = format!("counters: {name}=");
    err.lines()
        .find_map(|l| l.strip_prefix(key.as_str())?.parse().ok())
        .unwrap_or(0)
}

/// A design run with the counters on: status, output, the error stream.
#[cfg(feature = "clift")]
fn counted(d: &Design, env: &[(&str, &str)], extra: &[&str]) -> (Option<i32>, String, String) {
    let mut args = vec!["run", "--std=2008", "--work", &d.work];
    args.extend(extra);
    args.push(&d.top);
    let mut e = vec![("YAVS_COUNTERS", "1")];
    e.extend(env);
    let r = yavs(&d.dir, &e, &args);
    (
        r.status.code(),
        String::from_utf8_lossy(&r.stdout).into_owned(),
        String::from_utf8_lossy(&r.stderr).into_owned(),
    )
}

/// `--tier=clift` compiles everything before the run, so what compiled code takes is the same on
/// every host: no sampler decides. `scatter_checked` and `gather_checked` count each use of the
/// cells, each held against the kernel's full test.
#[cfg(feature = "clift")]
const FORCED: &[&str] = &["--tier=clift", "--no-cache"];

/// A design in a file of its own: the interpreter's output against compiled code with the cells,
/// without and checked; the floors of what the cells took, all leaves counted alike.
#[cfg(feature = "clift")]
fn compiled_cells_change_nothing(d: &Design, skips: u64, copies: u64) {
    let interp = d.run(OFF, &["--tier=interp", "--no-cache"]);
    assert_eq!(interp.0, Some(0), "{}", interp.1);
    let off = counted(d, &[("YAVS_IMAGE_INLINE", "off")], FORCED);
    assert_eq!((&interp.0, &interp.1), (&off.0, &off.1), "cells off");
    let on = counted(d, &[], FORCED);
    assert_eq!((&interp.0, &interp.1), (&on.0, &on.1), "cells on");
    for name in [
        "scatter_spared",
        "gathers_copied",
        "gathers_read",
        "images",
        "cycles",
    ] {
        assert_eq!(
            counter(&off.2, name),
            counter(&on.2, name),
            "{name}: cells off against cells on"
        );
    }
    // A review comes at the next look of the host, a few scatters after its count.
    let (w0, w1) = (
        counter(&off.2, "image_writes"),
        counter(&on.2, "image_writes"),
    );
    assert!(
        w1.abs_diff(w0) * 50 <= w0 + 50,
        "image writes {w0} off, {w1} on"
    );
    let check = counted(d, &[("YAVS_IMAGE_CHECK", "1")], FORCED);
    assert_eq!(
        (&interp.0, &interp.1),
        (&check.0, &check.1),
        "checked: {}",
        check.2
    );
    let first = counted(d, CHECK, FORCED);
    assert_eq!(
        (&interp.0, &interp.1),
        (&first.0, &first.1),
        "from the first: {}",
        first.2
    );
    let (s, c) = (
        counter(&first.2, "scatter_checked"),
        counter(&first.2, "gather_checked"),
    );
    assert!(s >= skips, "only {s} skips in compiled code");
    assert!(c >= copies, "only {c} copies in compiled code");
    let mut args = FORCED.to_vec();
    args.extend(["--threads=4", "--shuffle=11"]);
    let par = counted(d, CHECK, &args);
    let sorted = |s: &str| {
        let mut v: Vec<_> = s.lines().map(str::to_owned).collect();
        v.sort();
        v
    };
    assert_eq!(interp.0, par.0, "4 threads: {}", par.2);
    assert_eq!(sorted(&interp.1), sorted(&par.1), "4 threads");
}

/// Force, entries, waveforms, delays, parts, resolved records around what compiled code skips.
#[cfg(feature = "clift")]
#[test]
fn the_cells_of_compiled_code_change_nothing_around_what_must_see_a_drive() {
    let d = Design::new("record_cells", &corpus("held_record"), "held_record");
    compiled_cells_change_nothing(&d, 100, 100);
}

#[cfg(feature = "clift")]
fn inline_design(tag: &str, text: &str, top: &str) -> Design {
    let dir = tempdir(tag);
    let file = dir.join("d.vhd");
    std::fs::write(&file, text).unwrap();
    Design::new_in(dir, &file, top)
}

/// Two instances of one entity share their leaf table and driver slot numbers but not their
/// images and gates: one holds its record, the other changes it, and both read it.
#[cfg(feature = "clift")]
#[test]
fn the_cells_are_the_machine_s() {
    let text = r#"
library ieee; use ieee.std_logic_1164.all;
package p is
  type rec is record a : std_ulogic; b : std_ulogic_vector(7 downto 0); end record;
end package;
library ieee; use ieee.std_logic_1164.all; use work.p.all;
entity inst is
  generic (step : natural);
  port (clk : in std_ulogic; n : in natural; o : out natural);
end;
architecture a of inst is
  signal r : rec := ('0', x"00");
begin
  process (clk)
    variable v : rec := ('0', x"00");
    variable c : natural := 0;
    variable x : rec;
  begin
    if rising_edge(clk) then
      if step /= 0 and n mod step = 0 then v.b := not v.b; v.a := not v.a; end if;
      r <= v;
      x := r;
      if x.b(0) = '1' then c := c + 1; end if;
      if x.a = '1' then c := c + 1000; end if;
      o <= c;
    end if;
  end process;
end;
library ieee; use ieee.std_logic_1164.all;
entity top is end;
architecture a of top is
  signal clk : std_ulogic := '0';
  signal n, o0, o1, o2 : natural := 0;
begin
  clk <= not clk after 5 ns when n < 300 else '0';
  process (clk) begin if rising_edge(clk) then n <= n + 1; end if; end process;
  i0 : entity work.inst generic map (0) port map (clk, n, o0);
  i1 : entity work.inst generic map (2) port map (clk, n, o1);
  i2 : entity work.inst generic map (7) port map (clk, n, o2);
  process (n) begin
    if n = 299 then
      report integer'image(o0) & " " & integer'image(o1) & " " & integer'image(o2);
    end if;
  end process;
end;
"#;
    let d = inline_design("record_cells_two", text, "top");
    compiled_cells_change_nothing(&d, 200, 200);
}

/// More plans in one process than the plan gate has bits, and a record whose field is sized by a
/// generic, so its value type has a descriptor.
#[cfg(feature = "clift")]
#[test]
fn the_cells_past_the_plan_gate_and_with_a_descriptor() {
    let n = 70;
    let mut sigs = String::new();
    let mut drives = String::new();
    let mut sum = String::new();
    for i in 0..n {
        sigs += &format!("  signal s{i} : rec := ('0', x\"00\");\n");
        drives += &format!(
            "      s{i} <= (v.a, v.b xor x\"{i:02x}\"); x := s{i}; if x.b(6) = '1' then c := c + {}; end if;\n",
            i + 1
        );
        sum += &format!(
            "      if n = {} then v.a := not v.a; v.b := not v.b; end if;\n",
            i * 3 + 10
        );
    }
    let text = format!(
        r#"
library ieee; use ieee.std_logic_1164.all;
package p is
  type rec is record a : std_ulogic; b : std_ulogic_vector(7 downto 0); end record;
  type dyn is record a : std_ulogic; b : std_ulogic_vector; end record;
end package;
library ieee; use ieee.std_logic_1164.all; use work.p.all;
entity sized is
  generic (w : natural);
  port (clk : in std_ulogic; n : in natural; o : out natural);
end;
architecture a of sized is
  subtype t is dyn(b(w - 1 downto 0));
  signal r : t := ('0', (others => '0'));
begin
  process (clk)
    variable v : t := ('0', (others => '0'));
    variable x : t;
    variable c : natural := 0;
  begin
    if rising_edge(clk) then
      if n mod 5 = 0 then v.b(0) := not v.b(0); end if;
      r <= v;
      x := r;
      if x.b(0) = '1' then c := c + w; end if;
      o <= c;
    end if;
  end process;
end;
library ieee; use ieee.std_logic_1164.all; use work.p.all;
entity top is end;
architecture a of top is
  signal clk : std_ulogic := '0';
  signal n, o0, o1 : natural := 0;
  signal total : natural := 0;
{sigs}begin
  clk <= not clk after 5 ns when n < 400 else '0';
  process (clk) begin if rising_edge(clk) then n <= n + 1; end if; end process;
  i0 : entity work.sized generic map (9) port map (clk, n, o0);
  i1 : entity work.sized generic map (70) port map (clk, n, o1);
  process (clk)
    variable v : rec := ('0', x"00");
    variable x : rec;
    variable c : natural := 0;
  begin
    if rising_edge(clk) then
{sum}{drives}      total <= c;
    end if;
  end process;
  process (n) begin
    if n = 399 then
      report integer'image(total) & " " & integer'image(o0) & " " & integer'image(o1);
    end if;
  end process;
end;
"#
    );
    let d = inline_design("record_cells_many", &text, "top");
    compiled_cells_change_nothing(&d, 1000, 1000);
}

/// What must see a record drive, in a design with no force anywhere, so the plan gate is not shut
/// for good and only its bits stand between the compiled compare and a wrong skip: entries of the
/// run made through a procedure (the optimiser drops a drive a later one overwrites, and a call
/// stops it), a whole drive of another value before the one that holds, entries before the first
/// compare of a plan, waveforms the zero-delay drive must take away, delays, parts, a sub-record
/// on the same driver slot, two sources, a last leaf that alone changes, a run that only holds.
/// Every event of every signal reports, with its delta.
#[cfg(feature = "clift")]
#[test]
fn the_cells_stand_aside_for_what_must_see_a_drive() {
    let text = r#"
library ieee; use ieee.std_logic_1164.all;
package p is
  type inner is record x : std_ulogic; y : std_ulogic_vector(3 downto 0); end record;
  type rec is record s : inner; b : std_ulogic_vector(7 downto 0); i : integer; c : std_ulogic; end record;
  constant init : rec := (('0', x"0"), x"00", 0, '0');
  type recs is array (0 to 3) of rec;
  function image (r : rec) return string;
  type two is record a : std_logic; b : std_logic_vector(3 downto 0); end record;
end package;
package body p is
  function image (r : rec) return string is
  begin
    return std_ulogic'image(r.s.x) & to_hstring(r.s.y) & " " & to_hstring(r.b) & " " & integer'image(r.i)
      & " " & std_ulogic'image(r.c);
  end function;
end package body;
library ieee; use ieee.std_logic_1164.all; use work.p.all;
entity top is end;
architecture a of top is
  signal clk : std_ulogic := '0';
  signal n : natural := 0;
  signal entries, whole, first, firsts, waved, delayed, parted, last, only, sub : rec := init;
  signal parts : recs := (others => init);
  signal res : two := ('Z', "ZZZZ");
  signal got : integer := 0;
begin
  clk <= not clk after 5 ns when n < 200 else '0';
  process (clk) begin if rising_edge(clk) then n <= n + 1; end if; end process;

  entries_p : process (clk)
    procedure set_b is begin entries.b <= x"aa"; end procedure;
    procedure set_c is begin entries.c <= '1'; end procedure;
    variable v : rec := init;
  begin
    if rising_edge(clk) then
      if n mod 7 = 3 then set_b; end if;
      if n mod 7 = 5 then set_c; end if;
      entries <= v;
      if n mod 11 = 4 then set_b; end if;
    end if;
  end process;

  whole_p : process (clk)
    procedure other is
      variable w : rec := (('1', x"f"), x"55", 3, '1');
    begin
      whole <= w;
    end procedure;
    variable v : rec := init;
  begin
    if rising_edge(clk) then
      if n mod 5 = 2 then other; end if;
      whole <= v;
    end if;
  end process;

  first_p : process (clk)
    procedure other is
      variable w : rec := (('1', x"3"), x"11", 1, '0');
    begin
      first <= w;
    end procedure;
    variable v : rec := init;
  begin
    if rising_edge(clk) then
      -- A plan of its own first: the machine has its cells when this plan first asks, with entries
      -- of the run on its driver.
      firsts <= v;
      if n >= 30 then
        if n < 70 then other; end if;
        first <= v;
      end if;
    end if;
  end process;

  waved_p : process (clk)
    variable v : rec := init;
    variable w : rec := (('1', x"9"), x"77", 2, '1');
  begin
    if rising_edge(clk) then
      case n mod 41 is
        when 30 => waved <= w after 25 ns;
        when 35 => waved.b <= x"66" after 25 ns;
        when others => waved <= v;
      end case;
    end if;
  end process;

  delayed_p : process (clk)
    variable v : rec := init;
    variable w : rec := (('1', x"9"), x"77", 2, '1');
  begin
    if rising_edge(clk) then
      case n mod 43 is
        when 30 => delayed <= transport w after 22 ns;
        when 31 => delayed <= transport v after 3 ns;
        when 36 => delayed <= w after 22 ns;
        when 37 => delayed <= reject 1 ns inertial v after 3 ns;
        when others => delayed <= v after 2 ns;
      end case;
    end if;
  end process;

  parts_p : process (clk)
    variable v : recs := (others => init);
    variable x : recs := (others => init);
    variable y : rec;
  begin
    if rising_edge(clk) then
      if n mod 13 = 6 then v(2).b := not v(2).b; end if;
      if n mod 9 = 1 then
        parts <= v;
      elsif n mod 9 = 4 then
        parts(1) <= v(2);
      elsif n mod 9 = 7 then
        parts(2 to 3) <= v(0 to 1);
      else
        parts(2) <= v(2);
      end if;
      x := parts;
      y := parts(n mod 4);
      x(1 to 2) := parts(2 to 3);
      x(0 to 1) := parts(2 to 3);
      got <= x(1).i + y.i + x(2).i;
    end if;
  end process;

  last_p : process (clk)
    variable v : rec := init;
    variable x : rec;
  begin
    if rising_edge(clk) then
      if n mod 17 = 9 then v.c := not v.c; end if;
      if n mod 23 = 9 then v.i := v.i + 1; end if;
      last <= v;
      x := last;
      assert x.c = last.c severity failure;
    end if;
  end process;

  sub_p : process (clk)
    variable v : rec := init;
    variable w : inner := ('1', x"c");
  begin
    if rising_edge(clk) then
      if n mod 3 = 0 then sub.s <= w; elsif n mod 3 = 1 then sub <= v; else sub.s <= v.s; end if;
    end if;
  end process;

  res_a : process (clk)
    variable v : two := ('1', "1010");
  begin
    if rising_edge(clk) then
      if n < 100 then res <= v; else res <= ('Z', "ZZZZ"); end if;
    end if;
  end process;
  res_b : process (clk)
    variable v : two := ('1', "1010");
  begin
    if rising_edge(clk) then
      if n >= 90 then res <= v; else res <= ('Z', "ZZZZ"); end if;
    end if;
  end process;

  only_p : process (clk)
    variable v : rec := init;
  begin
    if falling_edge(clk) then
      only <= v;
    end if;
  end process;
  postponed process (clk)
  begin
    report "clock " & std_ulogic'image(clk);
  end postponed process;

  watch : process (entries, whole, first, waved, delayed, parted, last, only, sub, parts, res, got)
  begin
    report image(entries) & " | " & image(whole) & " | " & image(first) & " | " & image(waved)
      & " | " & image(delayed) & " | " & image(last) & " | " & image(sub) & " | " & image(parts(1))
      & " | " & image(parts(2)) & " | " & std_logic'image(res.a) & to_hstring(res.b) & " | "
      & integer'image(got);
  end process;
end;
"#;
    let d = inline_design("record_cells_see", text, "top");
    compiled_cells_change_nothing(&d, 500, 300);
}

/// Force and release of a record compiled code skips: a force shuts every plan gate for good, so
/// the value driven is the one forced and only the driving value tells a skip. Every event and
/// the driving value report.
#[cfg(feature = "clift")]
#[test]
fn the_cells_shut_at_a_force() {
    let text = r#"
library ieee; use ieee.std_logic_1164.all;
entity top is end;
architecture a of top is
  type rec is record a : std_ulogic; b : std_ulogic_vector(7 downto 0); end record;
  signal clk : std_ulogic := '0';
  signal n : natural := 0;
  signal s : rec := ('0', x"00");
begin
  clk <= not clk after 5 ns when n < 80 else '0';
  process (clk) begin if rising_edge(clk) then n <= n + 1; end if; end process;
  process (clk)
    variable v : rec := ('0', x"00");
  begin
    if rising_edge(clk) then
      if n = 34 then v := ('1', x"5a"); end if;
      s <= v;
      report "driving " & std_ulogic'image(s.a'driving_value) & to_hstring(s.b'driving_value);
    end if;
  end process;
  process begin
    wait until n = 30;
    s.a <= force '1';
    s.b <= force x"5a";
    wait until n = 38;
    s.a <= release;
    s.b <= release;
    wait until n = 50;
    s.b <= force x"11";
    wait until n = 55;
    s.b <= release;
    wait;
  end process;
  process (s) begin
    report std_ulogic'image(s.a) & to_hstring(s.b);
  end process;
end;
"#;
    let d = inline_design("record_cells_force", text, "top");
    compiled_cells_change_nothing(&d, 20, 0);
}

/// Long enough for reviews: a plan that holds through its cell keeps its image and the counts of
/// the review are the host's; a value that changes in every leaf at every clock has its image let
/// go, for the compare and for a reader, and what compiled code reads after that is not the image.
#[cfg(feature = "clift")]
#[test]
fn the_cells_go_with_the_review() {
    let text = r#"
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity top is end;
architecture a of top is
  type rec is record
    a, b, c, d, e, f, g, h : std_ulogic;
    u : unsigned(15 downto 0);
    i : integer;
  end record;
  constant init : rec := ('0', '0', '0', '0', '0', '0', '0', '0', x"0000", 0);
  signal clk : std_ulogic := '0';
  signal n : natural := 0;
  signal held, often, gone, read : rec := init;
  type pack is record a, c, d, e, f : std_ulogic; u : unsigned(15 downto 0); b : std_ulogic; end record;
  signal kept : pack := ('0', '0', '0', '0', '0', x"0000", '0');
  signal count : natural := 0;
begin
  clk <= not clk after 5 ns when n < 3200 else '0';
  -- One run of leaves with a bit that changes at every clock: the compare spares nothing and lets
  -- go, the reader keeps the image, and then the value holds.
  kept_p : process (clk)
    variable v : pack := ('0', '0', '0', '0', '0', x"0000", '0');
  begin
    if rising_edge(clk) then
      if n < 1500 then v.b := not v.b; end if;
      kept <= v;
    end if;
  end process;
  kept_r : process (clk)
    variable x : pack;
    variable c : natural := 0;
  begin
    if rising_edge(clk) then
      x := kept;
      if x.b = '1' then c := c + 1; end if;
      count <= c;
    end if;
  end process;
  process (clk) begin if rising_edge(clk) then n <= n + 1; end if; end process;
  held_p : process (clk)
    variable v : rec := init;
  begin
    if rising_edge(clk) then
      if n mod 40 = 1 then v.u := v.u + 1; end if;
      held <= v;
    end if;
  end process;
  often_p : process (clk)
    variable v : rec := init;
  begin
    if rising_edge(clk) then
      if n mod 2 = 1 then v.a := not v.a; v.b := not v.b; v.i := v.i + 1; v.h := not v.h; end if;
      often <= v;
    end if;
  end process;
  gone_p : process (clk)
    variable v : rec := init;
  begin
    if rising_edge(clk) then
      if n < 2200 then
        v := (not v.a, not v.b, not v.c, not v.d, not v.e, not v.f, not v.g, not v.h, v.u + 1, v.i + 1);
      end if;
      gone <= v;
    end if;
  end process;
  read_p : process (clk)
    variable x : rec;
    variable c : natural := 0;
  begin
    if rising_edge(clk) then
      x := gone;
      c := (c + x.i + to_integer(x.u)) mod 65536;
      read <= (x.a, x.b, x.c, x.d, x.e, x.f, x.g, x.h, x.u, c);
    end if;
  end process;
  process (n) begin
    if n mod 100 = 0 then
      report integer'image(read.i) & " " & integer'image(held.i) & " " & to_hstring(held.u)
        & " " & integer'image(often.i) & " " & integer'image(count);
    end if;
  end process;
end;
"#;
    let d = inline_design("record_cells_review", text, "top");
    compiled_cells_change_nothing(&d, 3000, 1000);
}

/// Records that change in every leaf at every clock: past the review the plans have no image, and
/// compiled code reads and drives them on the kernel itself (`yavs_rt_gather`, `yavs_rt_scatter`,
/// `docs/decisions.md` "Record drives in compiled code"). Nested records, an array field, integer,
/// enumeration, real and null leaves, a field sized by a generic, transport, a delay, a guarded
/// target, a port of resolved fields with two drivers, `'event`, `'active`, `'last_value`. Output,
/// kernel counters and waves are the interpreter's, also at 4 threads.
#[cfg(feature = "clift")]
#[test]
fn the_direct_records_change_nothing() {
    let d = inline_design("record_direct", DIRECT, "top");
    let interp = counted(&d, &[], &["--tier=interp", "--no-cache"]);
    assert_eq!(interp.0, Some(0), "{}", interp.2);
    let nodirect = counted(&d, &[("YAVS_IMAGE_INLINE", "nodirect")], FORCED);
    let direct = counted(&d, &[], FORCED);
    for (what, r) in [("nodirect", &nodirect), ("direct", &direct)] {
        assert_eq!((&interp.0, &interp.1), (&r.0, &r.1), "{what}: {}", r.2);
        for name in [
            "cycles",
            "process_runs",
            "wakes",
            "events",
            "update_fast",
            "update_timed",
            "update_general",
            "direct_codes",
            "direct_planes",
            "direct_bytes",
            "general_assigns",
            "gathers_copied",
            "gathers_read",
            "scatter_spared",
            "images",
        ] {
            assert_eq!(
                counter(&interp.2, name),
                counter(&r.2, name),
                "{name}: the interpreter against {what}"
            );
        }
    }
    // The callbacks are counted in debug builds.
    if cfg!(debug_assertions) {
        let n = counter(&direct.2, "cb_record");
        assert!(n >= 40_000, "only {n} direct reads and drives");
        assert_eq!(counter(&nodirect.2, "cb_record"), 0, "nodirect");
    }
    let waves = |tier: &str, more: &[&str]| {
        let f = d.dir.join(format!("{}.fst", more.len()));
        let wave = format!("--wave={}", f.display());
        let mut args = vec![tier, "--no-cache", &wave];
        args.extend(more);
        let r = counted(&d, &[], &args);
        assert_eq!(r.0, Some(0), "{tier} {more:?}: {}", r.2);
        (r.1, std::fs::read(&f).unwrap())
    };
    let one = waves("--tier=interp", &[]);
    assert!(one.1.len() > 1000);
    assert!(waves("--tier=clift", &[]) == one, "waves or output differ");
    assert!(
        waves("--tier=clift", &["--threads=4", "--shuffle=11"]) == one,
        "waves or output differ at 4 threads"
    );
}

#[cfg(feature = "clift")]
const DIRECT: &str = r#"
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
package p is
  type words is array (0 to 1) of std_ulogic_vector(7 downto 0);
  type inner is record
    w : words;
    k : integer;
    s : std_ulogic;
  end record;
  type state is (idle, busy, held);
  type rec is record
    re, im : signed(17 downto 0);
    n : inner;
    st : state;
    z : std_ulogic_vector(0 downto 1);
    x : real;
  end record;
  constant init : rec := (to_signed(5, 18), to_signed(-6, 18), ((x"01", x"02"), 3, '0'), idle, "", 0.5);
  type bus_t is record
    a : std_logic;
    v : std_logic_vector(5 downto 0);
  end record;
  function step (r : rec; i : integer) return rec;
  function sum (r : rec) return integer;
end;
package body p is
  function step (r : rec; i : integer) return rec is
    variable v : rec := r;
  begin
    v.re := r.im + 1;
    v.im := r.re - i;
    v.n.w := (std_ulogic_vector(unsigned(r.n.w(1)) + 3), not r.n.w(0));
    v.n.k := (r.n.k * 5 + i) mod 1000;
    v.n.s := not r.n.s;
    v.st := state'val((state'pos(r.st) + 1) mod 3);
    v.x := r.x + 0.25;
    return v;
  end;
  function sum (r : rec) return integer is
  begin
    return to_integer(r.re) - to_integer(r.im) + to_integer(unsigned(r.n.w(0))) + 3 * to_integer(unsigned(r.n.w(1)))
      + r.n.k + state'pos(r.st) * 7 + integer(r.x * 4.0) + r.z'length + std_ulogic'pos(r.n.s);
  end;
end;

library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
use work.p.all;
entity unit is
  generic (seed : integer; w : positive);
  port (clk : in std_ulogic; q : out integer; b : inout bus_t);
end;
architecture a of unit is
  signal s0 : rec := init;
  signal s1, s2, s3, t, d, g, l : rec := init;
  signal e1, a2, ev : natural := 0;
  type sized is record v : std_ulogic_vector(w - 1 downto 0); c : integer; end record;
  signal z0, z1 : sized := ((others => '0'), 0);
begin
  process (clk)
    variable i : integer := seed;
  begin
    if rising_edge(clk) then
      i := i + 1;
      s0 <= step(s0, i);
      s1 <= s0;
      s2 <= (re => s1.im, im => s1.re, n => s1.n, st => s1.st, z => s1.z, x => s1.x);
      t <= transport s1;
      d <= s2 after 3 ns;
      if i mod 3 /= 0 then
        s3 <= t;
      end if;
      l <= s2'last_value;
      z0 <= (v => not z0.v(0) & z0.v(w - 1 downto 1), c => z0.c + i);
      z1 <= z0;
    end if;
  end process;
  blk : block (clk = '1') is
  begin
    g <= guarded s3;
  end block;
  -- Two drivers of a record of resolved fields, each 'Z' half of the time.
  process (clk)
    variable n : natural := 0;
  begin
    if rising_edge(clk) then
      n := n + 1;
      if (n + seed) mod 2 = 0 then
        b <= (a => '1', v => std_logic_vector(to_unsigned((n + seed) mod 64, 6)));
      else
        b <= (a => 'Z', v => (others => 'Z'));
      end if;
    end if;
  end process;
  process (s2)
  begin
    if s2'event then
      e1 <= e1 + 1;
    end if;
  end process;
  process (s3, t)
  begin
    if s3'active then
      a2 <= a2 + 1;
    end if;
    if t.n'event then
      ev <= ev + 1;
    end if;
  end process;
  q <= sum(s3) + sum(d) - sum(g) + sum(l) + e1 + 2 * a2 + 3 * ev + z1.c mod 1000
    + to_integer(unsigned(z1.v(w - 1 downto w - 2)));
end;

library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
use work.p.all;
entity top is end;
architecture a of top is
  signal clk : std_ulogic := '0';
  signal n : natural := 0;
  type qs is array (0 to 1) of integer;
  signal q : qs;
  signal b : bus_t := ('Z', (others => 'Z'));
begin
  clk <= not clk after 5 ns when n < 1300 else '0';
  process (clk) begin if rising_edge(clk) then n <= n + 1; end if; end process;
  u0 : entity work.unit generic map (seed => 7, w => 5) port map (clk, q(0), b);
  u1 : entity work.unit generic map (seed => 1000, w => 9) port map (clk, q(1), b);
  process (n)
  begin
    if n mod 50 = 0 then
      report integer'image(n) & " " & integer'image(q(0)) & " " & integer'image(q(1)) & " " & std_logic'image(b.a)
        & " " & to_hstring(b.v);
    end if;
  end process;
end;
"#;

/// Whole record drives that change in some runs and hold in the rest (`held/held_record_runs`):
/// compiled code's compare misses and `compared_scatter` compares run by run with no host
/// (`scatters_compared`). The same output and the same counts as with the misses sent to the host
/// (`YAVS_IMAGE_INLINE=nomiss`) and with no cells; checked from the first scatter at 1 and 4
/// threads.
#[cfg(feature = "clift")]
#[test]
fn compiled_misses_compare_run_by_run() {
    let d = Design::new(
        "record_compare_runs",
        &corpus("held_record_runs"),
        "held_record_runs",
    );
    compiled_cells_change_nothing(&d, 1, 500);
    let on = counted(&d, &[], FORCED);
    let host = counted(&d, &[("YAVS_IMAGE_INLINE", "nomiss")], FORCED);
    assert_eq!((&on.0, &on.1), (&host.0, &host.1), "misses to the host");
    for name in [
        "scatter_spared",
        "image_writes",
        "images",
        "cycles",
        "events",
    ] {
        assert_eq!(
            counter(&on.2, name),
            counter(&host.2, name),
            "{name}: compiled misses against the host's"
        );
    }
    assert_eq!(counter(&host.2, "scatters_compared"), 0);
    let n = counter(&on.2, "scatters_compared");
    assert!(n > 500, "only {n} scatters compared run by run");
    let first = counted(&d, CHECK, FORCED);
    assert_eq!((&on.0, &on.1), (&first.0, &first.1), "checked");
    assert!(counter(&first.2, "scatters_compared") > n);
}

/// Parts of record signals driven at delay 0 from compiled code (`held/held_record`: one record
/// of an array, two, the whole array, around force and release) go through `direct_part`, with
/// no host (`part_scatters`): the same output and counts as through the signal callback
/// (`YAVS_IMAGE_INLINE=nopart`), at 1 and 4 threads.
#[cfg(feature = "clift")]
#[test]
fn compiled_parts_drive_with_no_host() {
    let d = Design::new(
        "record_compare_parts",
        &corpus("held_record"),
        "held_record",
    );
    let interp = d.run(OFF, &["--tier=interp", "--no-cache"]);
    let host = counted(&d, &[("YAVS_IMAGE_INLINE", "nopart")], FORCED);
    assert_eq!(counter(&host.2, "part_scatters"), 0);
    let mut par = FORCED.to_vec();
    par.extend(["--threads=4", "--shuffle=5"]);
    // A shuffled run reports processes of one cycle in another order: compare sorted.
    let sorted = |s: &str| {
        let mut v: Vec<_> = s.lines().map(str::to_owned).collect();
        v.sort();
        v
    };
    for args in [FORCED, &par[..]] {
        let on = counted(&d, CHECK, args);
        assert_eq!(interp.0, on.0, "{args:?}");
        assert_eq!(sorted(&interp.1), sorted(&on.1), "{args:?}");
        let n = counter(&on.2, "part_scatters");
        assert!(n > 100, "only {n} parts driven with no host");
    }
    let on = counted(&d, &[], FORCED);
    for name in ["scatter_spared", "events", "cycles", "general_assigns"] {
        assert_eq!(
            counter(&on.2, name),
            counter(&host.2, name),
            "{name}: parts with no host against the callback"
        );
    }
}
