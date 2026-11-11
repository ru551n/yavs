//! The compare of a drive with the current value in Cranelift code (`TierFrame::gate`): a skip made
//! with no call gives what the interpreter gives, around everything that makes the kernel not skip.
//! Debug builds ask the kernel's full test at every such skip (`yavs_rt_gate_check`) and abort on
//! a difference; a stale gate would otherwise only show as a wrong value later.
#![cfg(feature = "clift")]

mod common;
use common::tempdir;

use std::path::Path;
use std::process::Command;

const DESIGN: &str = r#"
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity unit is
  generic (g : natural);
end entity;

architecture a of unit is
  type color is (red, green, blue);
  signal clk : std_logic := '0';
  signal n : natural := 0;

  -- Held values of each form, with a change now and then.
  signal s1 : std_logic := '0';
  signal v8 : std_logic_vector(7 downto 0) := x"00";
  signal v70 : std_logic_vector(69 downto 0) := (others => '0');
  signal w100 : std_logic_vector(99 downto 0) := (others => '0');
  signal i1 : integer := 0;
  signal c1 : color := red;
  signal r1 : real := 0.0;
  signal e8 : unsigned(7 downto 0) := x"00";

  -- The order of drives in one run.
  signal o1, o2, o3 : std_logic := '0';
  signal ov : std_logic_vector(7 downto 0) := x"00";

  -- Delays on a direct driver.
  signal d1, d2, d3, d4 : std_logic := '0';
  signal dv : std_logic_vector(3 downto 0) := x"0";
  signal kp : std_logic_vector(7 downto 0) := x"00";

  -- The fields of a record: one kernel driver, a driver of the process each.
  type pair is record
    a, b : std_logic;
    c : std_logic_vector(3 downto 0);
  end record;
  signal rc : pair := ('0', '0', x"0");
  signal rw : pair := ('0', '0', x"0");
  signal hv : std_logic_vector(7 downto 0) := x"00";
  signal hi : integer := 0;
  signal kq, kr, ks, kt : std_logic_vector(7 downto 0) := x"00";
  signal di : integer := 0;

  -- Force and release of a signal with a direct driver.
  signal f1 : std_logic := '0';
  signal fv : std_logic_vector(7 downto 0) := x"00";
  signal fi : integer := 0;

  -- Resolved, several sources.
  signal rs : std_logic := 'Z';
  signal rv : std_logic_vector(3 downto 0) := "ZZZZ";

  -- Disjoint parts of a vector, one process each.
  signal part : std_logic_vector(7 downto 0) := x"00";

  -- Elements and slices at a run-time index, held and changed.
  type ints is array (0 to 7) of integer;
  signal at1 : std_logic_vector(15 downto 0) := x"0000";
  signal at2 : std_logic_vector(31 downto 0) := x"00000000";
  signal at3 : ints := (others => 0);
  signal atx : std_logic := '0';
  -- Whole values out of a function or an operator, held and changed, at a run-time element.
  type vec4 is array (0 to 3) of unsigned(7 downto 0);
  signal at4 : vec4 := (others => x"00");
  signal at5 : vec4 := (others => x"00");
  signal hf : std_logic_vector(15 downto 0) := x"0000";
  function slow(k : natural) return std_logic_vector is
  begin
    return std_logic_vector(to_unsigned(k / 16, 16));
  end function;
begin
  clk <= not clk after 5 ns;

  count : process (clk)
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      n <= n + 1;
    end if;
  end process;

  held : process (clk)
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      if n mod 16 = 3 then
        s1 <= not s1;
        v8 <= std_logic_vector(unsigned(v8) + 1);
        v70(69 downto 66) <= v8(3 downto 0);
        w100 <= w100(98 downto 0) & not w100(99);
        i1 <= i1 + 1;
        c1 <= color'val((color'pos(c1) + 1) mod 3);
        r1 <= r1 + 0.5;
        e8 <= e8 + 1;
      else
        s1 <= s1;
        v8 <= v8;
        v70(69 downto 66) <= v70(69 downto 66);
        w100 <= w100;
        i1 <= i1;
        c1 <= c1;
        r1 <= r1;
        e8 <= e8;
      end if;
    end if;
  end process;

  orders : process (clk)
    variable k : natural := 0;
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      k := n mod 8;
      -- The value held, then another: the second wins.
      o1 <= o1;
      if n mod 5 = 1 then
        o1 <= not o1;
      end if;
      -- Another value, then the one held: the second wins, nothing changes.
      if n mod 7 = 2 then
        o2 <= not o2;
      end if;
      o2 <= o2;
      -- An element at a run-time position, then a constant one of the same driver.
      ov(k) <= '1';
      ov(3) <= ov(3);
      ov(k) <= not ov(k);
      if n mod 9 = 4 then
        ov(3) <= not ov(3);
      end if;
      -- A held value after a change of another driver of the process.
      o3 <= '1' when n mod 11 = 5 else '0';
    end if;
  end process;

  -- One way of giving a direct driver a waveform per process: each shuts the gate for good, so a
  -- site that forgets to is seen by the zero-delay assignment of the held value after it.
  quick_delay : process (clk)
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      if n mod 13 = 1 then
        d1 <= not d1 after 12 ns;
      elsif n mod 13 = 2 then
        d1 <= d1;
      end if;
    end if;
  end process;

  transport_delay : process (clk)
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      if n mod 6 = 1 then
        d2 <= transport not d2 after 17 ns;
      elsif n mod 6 = 2 then
        d2 <= transport d2;
      end if;
    end if;
  end process;

  reject_limit : process (clk)
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      if n mod 17 = 3 then
        d3 <= reject 2 ns inertial not d3 after 25 ns;
      elsif n mod 17 = 4 then
        d3 <= d3;
      end if;
    end if;
  end process;

  two_elements : process (clk)
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      if n mod 23 = 5 then
        d4 <= '1' after 13 ns, '0' after 27 ns;
      elsif n mod 23 = 6 then
        d4 <= d4;
      end if;
    end if;
  end process;

  vector_delay : process (clk)
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      if n mod 19 = 7 then
        dv <= not dv after 23 ns;
      elsif n mod 19 = 8 then
        dv <= dv;
      end if;
    end if;
  end process;

  byte_delay : process (clk)
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      if n mod 29 = 9 then
        di <= di + 1 after 23 ns;
      elsif n mod 29 = 10 then
        di <= di;
      end if;
    end if;
  end process;

  -- A field that changes, then the held value of another field of the same kernel driver.
  record_fields : process (clk)
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      if n mod 3 = 0 then
        rc.c <= not rc.c;
      end if;
      rc.a <= rc.a;
      rc.b <= rc.b;
      rc.c <= rc.c;
    end if;
  end process;

  -- The whole record, then a field held, in one run.
  whole_record : process (clk)
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      if n mod 4 = 1 then
        rw <= (not rw.a, rw.b, not rw.c);
      end if;
      rw.b <= rw.b;
      rw.a <= rw.a;
    end if;
  end process;

  -- Two drivers of the process on one kernel driver: an entry through one closes the other.
  one_kernel_driver : process (clk)
    variable k : natural := 0;
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      k := n mod 8;
      if n mod 5 = 9 then
        kq(3) <= '0';
      end if;
      kq(k) <= not kq(k);
      kq(3) <= kq(3);
      kr(k) <= not kr(k);
      kr(3) <= kr(3);
      if n mod 5 = 9 then
        kr(k) <= '0';
      end if;
      ks(3 downto 2) <= not ks(3 downto 2);
      ks(5 downto 4) <= ks(5 downto 4);
      kt(5 downto 4) <= not kt(5 downto 4);
      kt(3 downto 2) <= kt(3 downto 2);
      if n mod 5 = 9 then
        ks(k) <= '0';
        kt(k) <= '0';
      end if;
    end if;
  end process;

  -- A change through the host, then the value held of the same driver: the entry stays first.
  host_entries : process (clk)
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      if n mod 7 = 2 then
        hv <= not hv;
        hi <= hi + 1;
      end if;
      hv <= hv;
      hi <= hi;
    end if;
  end process;

  -- A kept drive at a run-time position, then the held value of a constant element of the same
  -- driver: at position 3 the second assignment wins, though the kernel has not seen the first.
  kept_position : process (clk)
    variable k : natural := 0;
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      k := n mod 8;
      kp(k) <= '1';
      kp(3) <= kp(3);
    end if;
  end process;

  owner : process (clk)
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      -- Held by the driver; `cur` differs from the driving value while forced below.
      if n = 45 then
        -- The forced value: `cur` holds it, the driver does not. Must not be skipped.
        f1 <= '1';
        fv <= x"5a";
        fi <= 77;
      elsif n = 60 then
        f1 <= '0';
        fv <= x"00";
        fi <= 0;
      else
        f1 <= f1;
        fv <= fv;
        fi <= fi;
      end if;
    end if;
  end process;

  forcer : process
  begin
    wait until n = 40;
    f1 <= force '1';
    fv <= force x"5a";
    fi <= force 77;
    wait until n = 50;
    f1 <= release;
    fv <= release;
    fi <= release;
    wait until n = 55;
    f1 <= force '1';
    fv <= force x"5a";
    fi <= force 77;
    wait until n = 57;
    f1 <= release;
    fv <= release;
    fi <= release;
    wait;
  end process;

  res1 : process (clk)
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      rs <= 'Z';
      rv <= "ZZZZ";
      if n mod 4 = 0 then
        rs <= '1';
        rv <= "01ZZ";
      end if;
    end if;
  end process;

  res2 : process (clk)
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      rs <= '0';
      rv <= "ZZ10";
      if n mod 4 = 1 then
        rs <= 'Z';
        rv <= "ZZZZ";
      end if;
    end if;
  end process;

  at_run : process (clk)
    variable k : natural := 0;
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      k := n mod 16;
      at1(k) <= at1(k);
      if n mod 5 = 1 then
        at1(k) <= not at1((k + 3) mod 16);
      end if;
      at2(2 * k + 1 downto 2 * k) <= at2(2 * k + 1 downto 2 * k);
      if n mod 7 = 2 then
        at2(2 * k + 1 downto 2 * k) <= at2(2 * k + 1 downto 2 * k) xor "01";
      end if;
      at3(k mod 8) <= at3(k mod 8);
      if n mod 11 = 3 then
        at3(k mod 8) <= at3((k + 1) mod 8) + 1;
      end if;
      atx <= at1(k) xor at2(k);
      at4(k mod 4) <= at4(k mod 4) + 0;
      if n mod 13 = 4 then
        at4(k mod 4) <= at4((k + 1) mod 4) + 1;
      end if;
      hf <= slow(n);
      for i in at5'range loop
        if n mod 13 = 4 + i then
          at5(i) <= at5(i) + 1;
        else
          at5(i) <= at5(i) + 0;
        end if;
      end loop;
    end if;
  end process;

  part_lo : process (clk)
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      part(3 downto 0) <= part(3 downto 0);
      if n mod 10 = 6 then
        part(3 downto 0) <= std_logic_vector(unsigned(part(3 downto 0)) + 1);
      end if;
    end if;
  end process;

  part_hi : process (clk)
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      part(7) <= part(7);
      part(6 downto 4) <= part(6 downto 4);
      if n mod 10 = 8 then
        part(7) <= not part(7);
      end if;
    end if;
  end process;

  -- Every value at every event, as the results to compare.
  watch : process (clk'delayed(1 ns))
  begin
    if clk'delayed(1 ns) = '1' then
      report integer'image(g) & " " & integer'image(n) & " " & std_logic'image(s1) & to_hstring(v8) & to_hstring(v70)
        & to_hstring(w100) & integer'image(i1) & color'image(c1) & real'image(r1)
        & to_hstring(e8) & " " & std_logic'image(o1) & std_logic'image(o2) & std_logic'image(o3)
        & to_hstring(ov) & " " & std_logic'image(d1) & std_logic'image(d2)
        & std_logic'image(d3) & std_logic'image(d4) & to_hstring(dv) & integer'image(di) & to_hstring(kp) & std_logic'image(rc.a)
        & std_logic'image(rc.b) & to_hstring(rc.c) & to_hstring(hv) & integer'image(hi) & to_hstring(kq) & to_hstring(kr) & to_hstring(ks) & to_hstring(kt) & std_logic'image(rw.a) & std_logic'image(rw.b)
        & to_hstring(rw.c)
        & " " & std_logic'image(f1) & to_hstring(fv) & integer'image(fi)
        & " " & std_logic'image(rs) & to_hstring(rv) & " " & to_hstring(part)
        & " " & to_hstring(at1) & to_hstring(at2) & " " & integer'image(at3(0) + 3 * at3(5))
        & std_logic'image(atx) & to_hstring(at4(0)) & to_hstring(at4(3)) & to_hstring(hf)
        & to_hstring(at5(1)) & to_hstring(at5(2));
      if n = 150 then
        std.env.finish;
      end if;
    end if;
  end process;
end architecture;

entity top is
end entity;

architecture a of top is
begin
  -- A process reads a generic: it has a copy per instance, and the facts of the copy.
  u0 : entity work.unit generic map (0);
  u1 : entity work.unit generic map (1);
end architecture;
"#;

/// A run that only assigns values held keeps its delta: the report of the postponed process names
/// it, so a skip the kernel does not hear of moves the number.
/// An adder tree written as loops over elements at a run-time position, in a copy per instance:
/// whole values from `+` and from reads of an element, some held, driven through the compare.
const TREE: &str = r#"
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity unit is
  generic (g : natural);
end entity;

architecture a of unit is
  type t_arr is array (natural range <>) of signed(15 downto 0);
  signal clk : std_logic := '0';
  signal n : natural := 0;
  signal t : t_arr(0 to 12) := (others => (others => '0'));
  signal l1 : t_arr(0 to 6) := (others => (others => '0'));
  signal l2 : t_arr(0 to 3) := (others => (others => '0'));
  signal l3 : t_arr(0 to 1) := (others => (others => '0'));
  signal l4 : signed(15 downto 0) := (others => '0');
begin
  clk <= not clk after 5 ns;
  process (clk) is
  begin
    if rising_edge(clk) then
      n <= n + 1;
      for k in t'range loop
        if (n + k) mod 4 = g then
          t(k) <= t(k) + to_signed(g + 3 * k + 1, 16);
        else
          t(k) <= t(k);
        end if;
      end loop;
      for k in l1'range loop
        if 2 * k + 1 <= t'high then l1(k) <= t(2 * k) + t(2 * k + 1); else l1(k) <= t(2 * k); end if;
      end loop;
      for k in l2'range loop
        if 2 * k + 1 <= l1'high then l2(k) <= l1(2 * k) + l1(2 * k + 1); else l2(k) <= l1(2 * k); end if;
      end loop;
      for k in l3'range loop
        if 2 * k + 1 <= l2'high then l3(k) <= l2(2 * k) + l2(2 * k + 1); else l3(k) <= l2(2 * k); end if;
      end loop;
      l4 <= l3(0) + l3(1);
    end if;
  end process;
  process (clk) is
  begin
    if falling_edge(clk) then
      report integer'image(g) & " " & integer'image(n) & " " & to_hstring(l4) & " " & to_hstring(l3(1))
        & to_hstring(l2(3)) & to_hstring(l1(6)) & to_hstring(t(12));
      if n = 60 then
        std.env.finish;
      end if;
    end if;
  end process;
end architecture;

entity top is
end entity;

architecture a of top is
begin
  u0 : entity work.unit generic map (0);
  u1 : entity work.unit generic map (1);
end architecture;
"#;

const DELTAS: &str = r#"
library ieee;
use ieee.std_logic_1164.all;
entity unit is
  generic (g : natural);
end entity;
architecture a of unit is
  signal clk, x : std_logic := '0';
  signal v : std_logic_vector(3 downto 0) := x"0";
  signal t : natural := 0;
begin
  clk <= not clk after 5 ns;
  process (clk)
  begin
    if rising_edge(clk) then
      t <= t + 1;
      if t = 60 then
        std.env.finish;
      end if;
    end if;
  end process;
  -- Reads what changes, so the idle skip leaves it alone, and only assigns what is held.
  process (clk)
  begin
    if falling_edge(clk) then
      if g > 1000 or t < 0 then
        report "g";
      end if;
      x <= x;
      v <= v;
    end if;
  end process;
  postponed process (clk)
  begin
    if clk = '0' then
      report "falling " & integer'image(t);
    end if;
  end postponed process;
end architecture;
entity top is
end entity;
architecture a of top is
begin
  u0 : entity work.unit generic map (0);
end architecture;
"#;

/// A run-time slice through the driver of the whole signal, then the held value of a constant part
/// through its own driver of the process: where they overlap the second wins. Both orders of the
/// two drivers in the machine, and a part of a byte array.
const SHARED: &str = r#"
library ieee;
use ieee.std_logic_1164.all;
entity unit is
  generic (g : natural);
end entity;
architecture a of unit is
  type ints is array (0 to 3) of integer;
  type pair is record
    a : std_logic;
    c : std_logic_vector(7 downto 0);
  end record;
  signal clk : std_logic := '0';
  signal n : natural := 0;
  signal kv, kw : std_logic_vector(7 downto 0) := x"00";
  signal ka, kb : ints := (others => 0);
  signal kr : pair := ('0', x"00");
  type counts is record
    x, y : integer;
    z : boolean;
  end record;
  signal rb : counts := (0, 0, false);
begin
  clk <= not clk after 5 ns;
  process (clk) begin if rising_edge(clk) then n <= n + 1; end if; end process;
  const_first : process (clk)
    variable k : natural;
  begin
    if rising_edge(clk) then
      if g > 1000 then
        kv(3 downto 2) <= "00";
        ka(2) <= 0;
        kr.c(3 downto 2) <= "00";
      end if;
      k := n mod 7;
      if n mod 3 = 0 then
        kv(k + 1 downto k) <= not kv(k + 1 downto k);
        ka(k mod 4) <= ka(k mod 4) + 1;
        kr.c(k + 1 downto k) <= not kr.c(k + 1 downto k);
      end if;
      kv(3 downto 2) <= kv(3 downto 2);
      ka(2) <= ka(2);
      kr.c(3 downto 2) <= kr.c(3 downto 2);
    end if;
  end process;
  record_scatter : process (clk) begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      if n mod 3 = 2 then
        rb <= (rb.x + 1, rb.y + 1, not rb.z);
      end if;
      rb.y <= rb.y;
      rb.z <= rb.z;
    end if;
  end process;
  whole_first : process (clk)
    variable k : natural;
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      k := n mod 7;
      if n mod 3 = 1 then
        kw(k + 1 downto k) <= not kw(k + 1 downto k);
        kb(k mod 4) <= kb(k mod 4) + 1;
      end if;
      kw(3 downto 2) <= kw(3 downto 2);
      kb(2) <= kb(2);
    end if;
  end process;
  process (clk) begin
    if falling_edge(clk) then
      report integer'image(n) & " " & to_hstring(kv) & to_hstring(kw) & to_hstring(kr.c)
        & integer'image(ka(0)) & " " & integer'image(ka(1)) & " " & integer'image(ka(2)) & " " & integer'image(ka(3))
        & " " & integer'image(kb(0)) & " " & integer'image(kb(2))
        & " " & integer'image(rb.x) & " " & integer'image(rb.y) & " " & boolean'image(rb.z);
      if n = 60 then std.env.finish; end if;
    end if;
  end process;
end architecture;
entity top is
end entity;
architecture a of top is
begin
  u0 : entity work.unit generic map (0);
  u1 : entity work.unit generic map (1);
end architecture;
"#;

/// Assignments of byte arrays as a whole take the general path of `Ctx::assign`: a change leaves a
/// transaction pending and a held value a phantom, and neither may be passed by a skip of a part.
const GENERAL: &str = r#"
library ieee;
use ieee.std_logic_1164.all;
entity unit is
  generic (g : natural);
end entity;
architecture a of unit is
  type ints is array (0 to 3) of integer;
  type reals is array (0 to 2) of real;
  type color is (red, green, blue);
  type colors is array (0 to 2) of color;
  signal clk : std_logic := '0';
  signal n : natural := 0;
  signal pa, ph : ints := (others => 0);
  signal pr : reals := (others => 0.0);
  signal pc : colors := (others => red);
  signal after_many : std_logic := '0';
  signal many_v : std_logic_vector(3 downto 0) := x"0";
  signal many_i : integer := 0;
begin
  clk <= not clk after 5 ns;
  process (clk) begin if rising_edge(clk) then n <= n + 1; end if; end process;
  -- A change, then what a part holds: the second wins for that part. Fresh at each n of a process.
  pending : process (clk) begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      pa(1) <= pa(1);
      if n mod 5 = 2 then
        pa <= (pa(0) + 1, pa(1) + 1, pa(2) + 1, pa(3) + 1);
        pa(2) <= pa(2);
      end if;
    end if;
  end process;
  pending_real : process (clk) begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      pr(0) <= pr(0);
      if n mod 7 = 3 then
        pr <= (pr(0) + 1.0, pr(1) + 1.0, pr(2) + 1.0);
        pr(1) <= pr(1);
      end if;
    end if;
  end process;
  pending_enum : process (clk) begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      pc(0) <= pc(0);
      if n mod 3 = 1 then
        pc <= (blue, blue, blue);
        pc(1) <= pc(1);
        pc(2) <= green;
      end if;
    end if;
  end process;
  -- The whole held, a part held, a part changed.
  phantom : process (clk) begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      ph(1) <= ph(1);
      if n mod 5 = 1 then
        ph <= ph;
        ph(2) <= ph(2);
        ph(3) <= ph(3) + 1;
      end if;
    end if;
  end process;
  -- A waveform of several elements on a direct driver, then what is held, in a later run and in
  -- the same run: the held value takes the pending ones away.
  several : process (clk) begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      after_many <= after_many;
      many_v <= many_v;
      many_i <= many_i;
      if n mod 11 = 3 then
        after_many <= '1' after 12 ns, '0' after 27 ns;
        many_v <= x"5" after 3 ns, x"a" after 17 ns;
        many_i <= many_i + 1 after 13 ns, many_i + 2 after 25 ns;
      elsif n mod 11 = 8 then
        after_many <= '1' after 2 ns, '0' after 3 ns;
        many_v <= x"3" after 2 ns, x"c" after 4 ns;
        many_v(1) <= many_v(1);
        many_i <= many_i + 4 after 2 ns, many_i + 8 after 4 ns;
        many_i <= many_i;
        after_many <= after_many;
      end if;
    end if;
  end process;
  process (clk) begin
    if falling_edge(clk) then
      report integer'image(n) & " " & integer'image(pa(0)) & " " & integer'image(pa(1)) & " " & integer'image(pa(2)) & " " & integer'image(pa(3))
        & " " & integer'image(integer(pr(0))) & " " & integer'image(integer(pr(1))) & " " & integer'image(integer(pr(2)))
        & " " & color'image(pc(0)) & " " & color'image(pc(1)) & " " & color'image(pc(2))
        & " " & integer'image(ph(0)) & " " & integer'image(ph(2)) & " " & integer'image(ph(3))
        & " " & std_logic'image(after_many) & to_hstring(many_v) & integer'image(many_i);
      if n = 60 then std.env.finish; end if;
    end if;
  end process;
end architecture;
entity top is
end entity;
architecture a of top is
begin
  u0 : entity work.unit generic map (0);
  u1 : entity work.unit generic map (1);
end architecture;
"#;

/// Drives kept for drivers with no bit (a signal of two sources, a resolution function of the
/// design), then a skip of a direct driver: the kept ones are of other kernel drivers, so the skip
/// is right and the check must not object.
const KEPT_OTHER: &str = r#"
library ieee;
use ieee.std_logic_1164.all;
entity unit is
  generic (g : natural);
end entity;
architecture a of unit is
  function res(v : std_ulogic_vector) return std_ulogic is
    variable r : std_ulogic := 'Z';
  begin
    for k in v'range loop
      if v(k) /= 'Z' then r := v(k); end if;
    end loop;
    return r;
  end function;
  subtype rl is res std_ulogic;
  signal clk : std_logic := '0';
  signal n : natural := 0;
  signal two : std_logic := 'Z';
  signal ur : rl := 'Z';
  signal one, more : std_logic := '0';
begin
  clk <= not clk after 5 ns;
  process (clk) begin if rising_edge(clk) then n <= n + 1; end if; end process;
  process (clk) begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      two <= two;
      ur <= ur;
      one <= one;
      two <= '1' when n mod 3 = 0 else 'Z';
      more <= more;
      if n mod 5 = 0 then more <= not more; end if;
    end if;
  end process;
  process (clk) begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      two <= 'Z';
    end if;
  end process;
  process (clk) begin
    if falling_edge(clk) then
      report integer'image(n) & std_logic'image(two) & std_ulogic'image(ur) & std_logic'image(one) & std_logic'image(more);
      if n = 40 then std.env.finish; end if;
    end if;
  end process;
end architecture;
entity top is
end entity;
architecture a of top is
begin
  u0 : entity work.unit generic map (0);
  u1 : entity work.unit generic map (1);
end architecture;
"#;

/// What the compare itself must tell apart: values that differ in one plane or only in the last
/// element, elements of one signal at other places, a wide vector.
const FORMS: &str = r#"
library ieee;
use ieee.std_logic_1164.all;
entity unit is
  generic (g : natural);
end entity;
architecture a of unit is
  type ints is array (0 to 3) of integer;
  signal clk : std_logic := '0';
  signal n : natural := 0;
  -- Codes that share planes, in a vector.
  signal pl : std_logic_vector(3 downto 0) := "0000";
  -- Only the last element of the part differs.
  signal la : std_logic_vector(7 downto 0) := x"00";
  -- An element that differs from the first of its signal.
  signal e1 : std_logic_vector(7 downto 0) := x"01";
  signal e2 : std_logic_vector(99 downto 0) := (0 => '1', others => '0');
  signal ea : ints := (5, 0, 0, 0);
  -- A wide vector as a whole, then what a part held.
  signal w1 : std_logic_vector(99 downto 0) := (others => '0');
begin
  clk <= not clk after 5 ns;
  process (clk) begin if rising_edge(clk) then n <= n + 1; end if; end process;
  planes : process (clk) begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      case n mod 5 is
        when 0 => pl <= "0000";
        when 1 => pl <= "ZZZZ";
        when 2 => pl <= "LLLL";
        when 3 => pl <= "UUUU";
        when others => pl <= "0000";
      end case;
    end if;
  end process;
  last_element : process (clk) begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      if n mod 4 = 1 then
        la(3 downto 0) <= "1000";
      else
        la(3 downto 0) <= "0000";
      end if;
    end if;
  end process;
  other_element : process (clk) begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      if n mod 4 = 2 then
        e1(5) <= '1';
        e2(70) <= '1';
      else
        e1(5) <= '0';
        e2(70) <= '0';
      end if;
    end if;
  end process;
  other_byte : process (clk) begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      if n mod 4 = 2 then
        ea(2) <= 5;
      else
        ea(2) <= 0;
      end if;
    end if;
  end process;
  wide : process (clk) begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      if n mod 5 = 2 then
        w1 <= not w1;
        w1(3) <= w1(3);
        w1(70 downto 66) <= w1(70 downto 66);
      end if;
      w1(4) <= w1(4);
    end if;
  end process;
  process (clk) begin
    if falling_edge(clk) then
      report integer'image(n) & " " & to_string(pl)
        & to_hstring(la) & to_hstring(e1) & to_hstring(e2) & integer'image(ea(2)) & " " & to_hstring(w1);
      if n = 60 and g = 0 then std.env.finish; end if;
    end if;
  end process;
end architecture;
entity top is
end entity;
architecture a of top is
begin
  u0 : entity work.unit generic map (0);
  u1 : entity work.unit generic map (1);
end architecture;
"#;

/// A delay on the value held is a transaction later and no delta now: alone in its time step, so
/// a skip that should not be there moves the delta number the postponed process reports. The
/// count changes on the other edge. The delay shuts the process's gate, so each process has one
/// chance, late enough for it to be compiled.
const DELAYED: &str = r#"
library ieee;
use ieee.std_logic_1164.all;
entity unit is
  generic (g : natural);
end entity;
architecture a of unit is
  signal clk : std_logic := '0';
  signal n : natural := 0;
  signal ds : std_logic := '0';
  signal dv : std_logic_vector(3 downto 0) := x"0";
  signal di : integer := 0;
  signal k : std_logic := '0';
begin
  clk <= not clk after 5 ns;
  process (clk) begin if falling_edge(clk) then n <= n + 1; end if; end process;
  held_on_the_other_edge : process (clk) begin
    if falling_edge(clk) then
      if g > 1000 or n < 0 then report "g"; end if;
      k <= k;
    end if;
  end process;
  delay_scalar : process (clk) begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      if n mod 6 = 1 and n > 12 then ds <= ds after 3 ns; end if;
    end if;
  end process;
  delay_vector : process (clk) begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      if n mod 6 = 2 and n > 12 then dv <= dv after 3 ns; end if;
    end if;
  end process;
  delay_bytes : process (clk) begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      if n mod 6 = 3 and n > 12 then di <= di after 3 ns; end if;
    end if;
  end process;
  postponed process (clk) begin
    report integer'image(n) & " pp " & std_logic'image(clk);
    if n = 40 and g = 0 then std.env.finish; end if;
  end postponed process;
end architecture;
entity top is
end entity;
architecture a of top is
begin
  u0 : entity work.unit generic map (0);
  u1 : entity work.unit generic map (1);
end architecture;
"#;

/// A drive in a procedure of the process, which is not the process's own compiled code, then the
/// held value in the process.
const NESTED: &str = r#"
library ieee;
use ieee.std_logic_1164.all;
entity unit is
  generic (g : natural);
end entity;
architecture a of unit is
  signal clk : std_logic := '0';
  signal n : natural := 0;
  signal v : std_logic_vector(7 downto 0) := x"00";
  signal w : std_logic_vector(7 downto 0) := x"00";
  signal i : integer := 0;
  signal u : std_logic := '0';
begin
  clk <= not clk after 5 ns;
  process (clk) begin if rising_edge(clk) then n <= n + 1; end if; end process;
  process (clk)
    procedure flip is
    begin
      v(1) <= not v(1);
    end procedure;
    procedure count is
    begin
      i <= i + 1;
    end procedure;
    procedure flip_part is
    begin
      w(3 downto 2) <= not w(3 downto 2);
    end procedure;
    procedure put (signal t : out std_logic; x : std_logic) is
    begin
      t <= x;
    end procedure;
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      if n mod 3 = 0 then
        flip;
      end if;
      v(1) <= v(1);
      if n mod 7 = 3 then
        put(u, not u);
      end if;
      u <= u;
      if n mod 4 = 1 then
        count;
      end if;
      i <= i;
      if n mod 5 = 2 then
        flip_part;
      end if;
      w(3 downto 2) <= w(3 downto 2);
    end if;
  end process;
  process (clk) begin
    if falling_edge(clk) then
      report integer'image(n) & " " & to_hstring(v) & to_hstring(w) & integer'image(i) & std_logic'image(u);
      if n = 60 and g = 0 then std.env.finish; end if;
    end if;
  end process;
end architecture;
entity top is
end entity;
architecture a of top is
begin
  u0 : entity work.unit generic map (0);
  u1 : entity work.unit generic map (1);
end architecture;
"#;

/// A signal whose activity is read (`'transaction`) stays out of the compare; another of the same
/// process is in it.
const ACTIVITY: &str = r#"
library ieee;
use ieee.std_logic_1164.all;
entity unit is
  generic (g : natural);
end entity;
architecture a of unit is
  signal clk : std_logic := '0';
  signal n : natural := 0;
  signal s, q, o : std_logic := '0';
  signal i : integer := 0;
  signal v : std_logic_vector(3 downto 0) := x"0";
begin
  clk <= not clk after 5 ns;
  process (clk) begin if rising_edge(clk) then n <= n + 1; end if; end process;
  process (clk) begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      s <= s;
      q <= q;
      i <= i;
      v(2) <= v(2);
      o <= o;
    end if;
  end process;
  process begin wait on s'transaction; report "s"; end process;
  process begin wait on i'transaction; report "i"; end process;
  process begin wait on v(2)'transaction; report "v"; end process;
  process (q'quiet) begin report "q " & boolean'image(q'quiet); end process;
  process (clk) begin
    if falling_edge(clk) and n = 30 and g = 0 then std.env.finish; end if;
  end process;
end architecture;
entity top is
end entity;
architecture a of top is
begin
  u0 : entity work.unit generic map (0);
  u1 : entity work.unit generic map (1);
end architecture;
"#;

/// A part of a slice whose place comes from a generic: another generic is another body, also when
/// the code of the first is in the cache.
const SLICE_OF_GENERIC: &str = r#"
library ieee;
use ieee.std_logic_1164.all;
entity top is
  generic (g : natural := 0);
end entity;
architecture a of top is
  signal clk : std_logic := '0';
  signal n : natural := 0;
  signal v : std_logic_vector(11 downto 0) := x"5a5";
begin
  clk <= not clk after 5 ns;
  process (clk) begin
    if rising_edge(clk) then
      if n < 0 then report "n"; end if;
      n <= n + 1;
      v(g + 3 downto g) <= v(g + 3 downto g);
      v(g + 5) <= v(g + 5);
      if n mod 4 = 1 then
        v(g) <= not v(g);
      end if;
    end if;
  end process;
  process (clk) begin
    if falling_edge(clk) then
      report integer'image(n) & " " & to_string(v);
      if n = 30 then std.env.finish; end if;
    end if;
  end process;
end architecture;
"#;

/// Rows and slices the optimiser reads in place (`yavs_ir` `opt/place.rs`) as the values of drives
/// that mostly hold, with one element or plane differing now and then.
const IN_PLACE: &str = r#"
library ieee;
use ieee.std_logic_1164.all;
entity unit is
  generic (g : natural);
end entity;
architecture a of unit is
  subtype word is std_logic_vector(31 downto 0);
  type words is array (0 to 3) of word;
  type rec is record
    tag : integer;
    cfg : words;
  end record;
  signal clk : std_logic := '0';
  signal n : natural := 0;
  signal rows : words := (x"000000A5", x"0000FFA5", x"00A500A5", x"A50000A5");
  signal q32 : word := x"000000A5";
  signal q8 : std_logic_vector(7 downto 0) := x"A5";
  signal b1 : std_logic := '1';
  signal hi : word := x"00000000";
  signal lo4 : std_logic_vector(3 downto 0) := x"5";
begin
  clk <= not clk after 5 ns;
  process (clk)
    variable r : rec := (0, (x"000000A5", x"0000FFA5", x"00A500A5", x"A50000A5"));
    variable k : integer := 0;
    variable w : word;
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      -- Rows and slices through a temporary the pass forwards, into a waveform: mostly what the
      -- target holds, one element or plane different now and then.
      if n mod 16 = 3 then
        r.cfg(0)(0) := not r.cfg(0)(0);
      elsif n mod 16 = 9 then
        r.cfg(0)(31) := 'X';
      elsif n mod 16 = 13 then
        r.cfg(0)(31) := '0';
      end if;
      k := (n / 4) mod 4;
      q32 <= r.cfg(k);
      q8 <= r.cfg(k)(7 downto 0);
      lo4 <= r.cfg(k)(3 downto 0);
      b1 <= r.cfg(k)(5);
      w := r.cfg(k);
      hi <= w;
      if rows(0)(7 downto 0) = x"A5" then
        q32 <= rows(k);
        q8 <= rows(k)(7 downto 0);
      end if;
      n <= n + 1;
    end if;
  end process;
  process (clk)
  begin
    if falling_edge(clk) then
      report integer'image(n) & " " & to_hstring(q32) & " " & to_hstring(q8) & " " & to_hstring(lo4) & " " & std_logic'image(b1) & " " & to_hstring(hi);
      if n = 60 then std.env.finish; end if;
    end if;
  end process;
end architecture;
entity top is
end entity;
architecture a of top is
begin
  u0 : entity work.unit generic map (0);
  u1 : entity work.unit generic map (1);
end architecture;
"#;

/// Held whole values in code that is not a copy, which has no compare of its own: offset 0, an
/// offset inside a plane word, a slice across two plane words, exactly 64 elements, values that
/// differ in their first or last element only, a record field.
const SHARED_FORMS: &str = r#"
library ieee;
use ieee.std_logic_1164.all;
entity top is
end entity;
architecture a of top is
  type rec is record
    a : std_logic_vector(3 downto 0);
    b : std_logic;
  end record;
  signal clk : std_logic := '0';
  signal n : natural := 0;
  signal v8 : std_logic_vector(7 downto 0) := x"5a";
  signal big : std_logic_vector(99 downto 0) := (others => '0');
  signal v64 : std_logic_vector(63 downto 0) := x"0123456789abcdef";
  signal c8, d8 : std_logic_vector(7 downto 0) := x"00";
  signal r : rec := (x"3", '1');
begin
  clk <= not clk after 5 ns;
  process (clk)
  begin
    if rising_edge(clk) then
      v8 <= v8;
      big(9 downto 2) <= big(9 downto 2);
      big(70 downto 60) <= big(70 downto 60);
      if n mod 4 = 1 then
        big(70 downto 60) <= not big(70 downto 60);
      end if;
      v64 <= v64;
      if n mod 5 = 0 then
        v64 <= not v64;
      end if;
      if n mod 3 = 0 then
        c8 <= c8(7 downto 1) & not c8(0);
        d8 <= not d8(7) & d8(6 downto 0);
      else
        c8 <= c8;
        d8 <= d8;
      end if;
      r.a <= r.a;
      if n mod 6 = 2 then
        r.a <= not r.a;
      end if;
      n <= n + 1;
    end if;
  end process;
  process (clk)
  begin
    if falling_edge(clk) then
      report integer'image(n) & " " & to_hstring(v8) & " " & to_hstring(big) & " " & to_hstring(v64) & " " & to_hstring(c8) & " " & to_hstring(d8) & " " & to_hstring(r.a) & std_logic'image(r.b);
      if n = 40 then std.env.finish; end if;
    end if;
  end process;
end architecture;
"#;

/// Kept scalar drives that change, handed over before a whole-value drive: the held vectors after
/// them are taken first by the runtime, but not a later drive of a kernel driver that has an entry
/// of the run (the last drives of `v(3)` and of `v` win: `v` holds).
const KEPT_THEN_HELD: &str = r#"
library ieee;
use ieee.std_logic_1164.all;
entity unit is
  generic (g : natural);
end entity;
architecture a of unit is
  signal clk : std_logic := '0';
  signal n : natural := 0;
  signal f, h : std_logic := '0';
  signal v : std_logic_vector(7 downto 0) := x"00";
  signal w : std_logic_vector(15 downto 0) := x"1234";
  signal q : std_logic_vector(3 downto 0) := x"5";
begin
  clk <= not clk after 5 ns;
  process (clk)
  begin
    if rising_edge(clk) then
      if g > 1000 then report "g"; end if;
      f <= not f;
      v(3) <= not v(3);
      v(2) <= not v(2);
      w <= w;
      h <= h;
      v(3) <= v(3);
      v <= v;
      q <= q;
      if n mod 7 = 0 then w <= not w; end if;
      n <= n + 1;
    end if;
  end process;
  process (clk)
  begin
    if falling_edge(clk) then
      report integer'image(n) & " " & std_logic'image(f) & std_logic'image(h) & " " & to_hstring(v) & " " & to_hstring(w) & " " & to_hstring(q);
      if n = 40 then std.env.finish; end if;
    end if;
  end process;
end architecture;
entity top is
end entity;
architecture a of top is
begin
  u0 : entity work.unit generic map (0);
  u1 : entity work.unit generic map (1);
end architecture;
"#;

/// A process of 70 drivers: those past the gate's 64 bits take the host's way, and a kept drive of
/// one stops a skip of none of the others wrongly.
fn many_drivers() -> String {
    let n = 70;
    let mut s = String::from(
        "library ieee;\nuse ieee.std_logic_1164.all;\nentity unit is\n  generic (g : natural);\nend entity;\narchitecture a of unit is\n  signal clk : std_logic := '0';\n  signal t : natural := 0;\n",
    );
    for k in 0..n {
        s += &format!("  signal x{k} : std_logic := '0';\n");
    }
    s += "begin\n  clk <= not clk after 5 ns;\n  process (clk)\n  begin\n    if rising_edge(clk) then\n      if g > 1000 then report \"g\"; end if;\n";
    for k in 0..n {
        s += &format!(
            "      x{k} <= x{k};\n      if t mod {} = 3 then\n        x{k} <= not x{k};\n      end if;\n",
            k + 2
        );
    }
    s += "      t <= t + 1;\n    end if;\n  end process;\n  process (clk'delayed(1 ns))\n  begin\n    if clk'delayed(1 ns) = '1' then\n      report integer'image(t)";
    for k in 0..n {
        s += &format!(" & std_logic'image(x{k})");
    }
    s += ";\n      if t = 80 then\n        std.env.finish;\n      end if;\n    end if;\n  end process;\nend architecture;\nentity top is\nend entity;\narchitecture a of top is\nbegin\n  u0 : entity work.unit generic map (0);\n  u1 : entity work.unit generic map (1);\nend architecture;\n";
    s
}

/// A kept drive of a driver past the gate's 64 bits, then the held value of another driver of the
/// same kernel driver: the kept one has no bit to close, so it closes the whole gate.
fn kept_past_the_bits() -> String {
    let n = 70;
    let mut s = String::from(
        "library ieee;\nuse ieee.std_logic_1164.all;\nentity unit is\n  generic (g : natural);\nend entity;\narchitecture a of unit is\n  signal clk : std_logic := '0';\n  signal t : natural := 0;\n  signal v : std_logic_vector(7 downto 0) := x\"00\";\n",
    );
    for k in 0..n {
        s += &format!("  signal x{k} : std_logic := '0';\n");
    }
    s += "begin\n  clk <= not clk after 5 ns;\n  process (clk)\n  begin\n    if rising_edge(clk) then\n      if g > 1000 then v(2) <= '0'; end if;\n";
    for k in 0..n {
        s += &format!("      x{k} <= x{k};\n");
    }
    s += "      v(3) <= not v(3);\n      v(2) <= v(2);\n      v(4) <= v(4);\n      t <= t + 1;\n    end if;\n  end process;\n  process (clk)\n  begin\n    if falling_edge(clk) then\n      report integer'image(t) & \" \" & to_hstring(v);\n      if t = 40 then\n        std.env.finish;\n      end if;\n    end if;\n  end process;\nend architecture;\nentity top is\nend entity;\narchitecture a of top is\nbegin\n  u0 : entity work.unit generic map (0);\n  u1 : entity work.unit generic map (1);\nend architecture;\n";
    s
}

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

/// Runs `text` at every tier against the interpreter; the number of skips the check saw on one thread.
fn against_interp(name: &str, text: &str) -> u64 {
    against_interp_after(name, text, "2")
}

/// [`against_interp`] with processes switching to their copies after `after` activations.
fn against_interp_after(name: &str, text: &str, after: &str) -> u64 {
    let d = tempdir(name);
    std::fs::write(d.join("t.vhd"), text).expect("write");
    let work = format!("work:{}", d.join("lib").display());
    let a = yavs(
        &d,
        &[],
        &["analyze", "--std=2008", "--work", &work, "t.vhd"],
    );
    assert_eq!(a.0, 0, "{}", a.2);
    let run = |env: &[(&str, &str)], tier: &[&str], threads: &str| {
        let mut args = vec!["run", "--std=2008", "--work", &work];
        args.extend(tier);
        args.extend(["--threads", threads, "top"]);
        yavs(&d, env, &args)
    };
    let interp = run(&[], &["--tier=interp", "--no-cache"], "1");
    assert_eq!(interp.0, 0, "{}", interp.2);
    let spec = ("YAVS_SPEC_AFTER", after);
    let check = ("YAVS_INLCMP", "check");
    let heat = ["--tier=clift-auto", "--tier-heat=1,0,0"];
    let cold = ["--tier=clift-auto", "--tier-heat=1,0,0", "--no-cache"];
    let auto = ["--tier=clift-auto", "--no-cache"];
    for (tier, threads) in [
        (&cold[..], "1"),
        (&cold[..], "4"),
        (&auto[..], "1"),
        // Written to the cache, then linked from it: the code holds no address of this run.
        (&heat[..], "1"),
        (&heat[..], "1"),
        (&heat[..], "4"),
    ] {
        let got = run(&[spec, check], tier, threads);
        assert_eq!(
            (&interp.0, &interp.1, &interp.2),
            (&got.0, &got.1, &got.2),
            "{tier:?} --threads {threads}"
        );
    }
    // The runs above stored the code in the work library: this one links it and still skips.
    let counters = ("YAVS_COUNTERS", "1");
    let (_, _, e) = run(&[spec, check, counters], &heat, "1");
    assert!(
        e.contains("counters: gate_checked=") || e.contains("counters: drive_src_held="),
        "a warm run skipped nothing: {e}"
    );
    let (_, _, e) = run(&[spec, check, counters], &cold, "1");
    e.lines()
        .find_map(|l| l.strip_prefix("counters: gate_checked="))
        .map_or(0, |n| n.parse().expect("a number"))
}

#[test]
fn skips_in_compiled_code_change_no_result() {
    let hits = against_interp("compiled_gate", DESIGN);
    assert!(hits > 100, "the compare took only {hits} skips");
}

#[test]
fn whole_values_at_run_time_elements_of_copies() {
    against_interp("compiled_gate_tree", TREE);
}

#[test]
fn a_run_of_skips_keeps_its_delta() {
    let hits = against_interp("compiled_gate_deltas", DELTAS);
    assert!(hits > 50, "the compare took only {hits} skips");
}

#[test]
fn a_process_of_more_drivers_than_the_gate_has_bits() {
    let hits = against_interp("compiled_gate_many", &many_drivers());
    assert!(hits > 100, "the compare took only {hits} skips");
}

#[test]
fn a_skip_through_one_driver_after_an_entry_through_another_of_its_kernel_driver() {
    let hits = against_interp("compiled_gate_shared", SHARED);
    assert!(hits > 50, "the compare took only {hits} skips");
}

#[test]
fn the_general_assignment_of_a_direct_driver_stops_the_skips() {
    let hits = against_interp("compiled_gate_general", GENERAL);
    assert!(hits > 20, "the compare took only {hits} skips");
}

#[test]
fn drives_kept_for_other_kernel_drivers_do_not_stop_a_skip() {
    let hits = against_interp("compiled_gate_kept_other", KEPT_OTHER);
    assert!(hits > 20, "the compare took only {hits} skips");
}

#[test]
fn held_values_in_code_that_is_no_copy() {
    against_interp_after("compiled_gate_shared", SHARED_FORMS, "1000000");
    let held = held_first("compiled_gate_shared_n", SHARED_FORMS, "1000000");
    assert!(held > 100, "only {held} held drives taken first");
}

/// The fused whole-value drives of `text` on `--tier=clift` that the runtime found held before the
/// kernel's look-ups (`drive_src_held`), copies made after `after` activations.
fn held_first(name: &str, text: &str, after: &str) -> u64 {
    let d = tempdir(name);
    std::fs::write(d.join("t.vhd"), text).expect("write");
    let work = format!("work:{}", d.join("lib").display());
    let a = yavs(
        &d,
        &[],
        &["analyze", "--std=2008", "--work", &work, "t.vhd"],
    );
    assert_eq!(a.0, 0, "{}", a.2);
    let (rc, _, e) = yavs(
        &d,
        &[("YAVS_SPEC_AFTER", after), ("YAVS_COUNTERS", "1")],
        &[
            "run",
            "--std=2008",
            "--work",
            &work,
            "--tier=clift",
            "--threads",
            "1",
            "top",
        ],
    );
    assert_eq!(rc, 0, "{e}");
    e.lines()
        .find_map(|l| l.strip_prefix("counters: drive_src_held="))
        .map_or(0, |n| n.parse().expect("a number"))
}

#[test]
fn drives_kept_and_handed_over_leave_the_others_open() {
    against_interp("compiled_gate_kept_then_held", KEPT_THEN_HELD);
    let held = held_first("compiled_gate_kept_then_held_n", KEPT_THEN_HELD, "2");
    assert!(held > 100, "only {held} held drives taken first");
}

#[test]
fn a_kept_drive_past_the_bits_stops_the_skips_of_its_kernel_driver() {
    let hits = against_interp("compiled_gate_kept_past", &kept_past_the_bits());
    assert!(hits > 50, "the compare took only {hits} skips");
}

/// The body of each generic in turn, the second run with the first's code in the cache.
#[test]
fn another_generic_in_a_warm_cache() {
    let d = tempdir("compiled_gate_generic");
    std::fs::write(d.join("t.vhd"), SLICE_OF_GENERIC).expect("write");
    let work = format!("work:{}", d.join("lib").display());
    let a = yavs(
        &d,
        &[],
        &["analyze", "--std=2008", "--work", &work, "t.vhd"],
    );
    assert_eq!(a.0, 0, "{}", a.2);
    let run = |env: &[(&str, &str)], tier: &[&str], g: &str| {
        let mut args = vec!["run", "--std=2008", "--work", &work];
        args.extend(tier);
        args.extend([g, "top"]);
        yavs(&d, env, &args)
    };
    let env = [
        ("YAVS_SPEC_AFTER", "2"),
        ("YAVS_INLCMP", "check"),
        ("YAVS_COUNTERS", "1"),
    ];
    let heat = ["--tier=clift-auto", "--tier-heat=1,0,0"];
    for g in ["-gG=0", "-gG=2", "-gG=6", "-gG=0"] {
        let interp = run(&[], &["--tier=interp", "--no-cache"], g);
        assert_eq!(interp.0, 0, "{}", interp.2);
        let got = run(&env, &heat, g);
        assert_eq!((interp.0, &interp.1), (got.0, &got.1), "{g}");
        assert!(got.2.contains("counters: gate_checked="), "{g}: {}", got.2);
    }
}

#[test]
fn the_compare_tells_values_and_delays_apart() {
    let hits = against_interp("compiled_gate_forms", FORMS);
    assert!(hits > 100, "the compare took only {hits} skips");
}

#[test]
fn a_drive_in_a_procedure_then_the_held_value_in_the_process() {
    let hits = against_interp("compiled_gate_nested", NESTED);
    assert!(hits > 20, "the compare took only {hits} skips");
}

#[test]
fn a_signal_whose_activity_is_read_is_not_skipped() {
    let hits = against_interp("compiled_gate_activity", ACTIVITY);
    assert!(hits > 20, "the compare took only {hits} skips");
}

#[test]
fn values_read_in_place_are_compared_as_they_are() {
    let hits = against_interp("compiled_gate_in_place", IN_PLACE);
    assert!(hits > 100, "the compare took only {hits} skips");
}

#[test]
fn a_delay_on_the_held_value_is_no_skip() {
    let hits = against_interp("compiled_gate_delayed", DELAYED);
    assert!(hits > 20, "the compare took only {hits} skips");
}
