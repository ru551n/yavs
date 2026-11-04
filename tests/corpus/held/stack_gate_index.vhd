-- difftest: top=stack_gate_index ignore=order
-- ignore=order: several processes report in one delta cycle, in an order the LRM leaves open.
-- A whole record held at zero delay against element drives of it at a run-time index in one process
-- run: kept before the whole drive (which replaces it), after it (which takes effect), from a nested
-- procedure, with a delay the whole drive preempts, resolved with a second driver. Whole reads and
-- reads of one element at a run-time index see every event.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

package stack_gate_p is
  type rec is record
    a : std_logic_vector(15 downto 0);
    b : std_logic;
    c : std_logic_vector(0 to 5);
    n : integer range 0 to 255;
  end record;
  type rrec is record
    a : std_logic_vector(15 downto 0);
    b : std_logic;
  end record;
  constant rec0 : rec := (x"00f0", '0', "010101", 3);
  function sum(r : rec) return natural;
end package;

package body stack_gate_p is
  function sum(r : rec) return natural is
    variable t : natural := r.n;
  begin
    for i in r.a'range loop
      if r.a(i) = '1' then t := t + i + 1; end if;
    end loop;
    for i in r.c'range loop
      if r.c(i) = '1' then t := t + 100 * (i + 1); end if;
    end loop;
    if r.b = '1' then t := t + 5000; end if;
    return t;
  end function;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use work.stack_gate_p.all;

entity stack_gate_inst is
  generic (K : natural; W : positive);
  port (clk : in std_logic; n : in natural; o : out natural; r2 : inout rrec);
end entity;

architecture a of stack_gate_inst is
  signal r : rec := rec0;
  signal v : std_logic_vector(W - 1 downto 0) := (others => '0');
begin
  main : process (clk)
    variable held : rec := rec0;
    variable x : rec;
    variable i : natural := 0;
    variable t : natural := 0;
    procedure touch(j : natural; val : std_logic) is
    begin
      r.a(j) <= val;
    end procedure;
    procedure hold_all is
    begin
      r <= held;
    end procedure;
  begin
    if rising_edge(clk) then
      i := (n * (K + 3)) mod 16;
      case n mod 23 is
        when 3 =>
          -- Kept at a run-time index, then replaced: nothing happens.
          r.a(i) <= not r.a(i);
          if n > 4 then hold_all; else r <= held; end if;
        when 5 =>
          -- The whole first, the element takes effect.
          hold_all;
          r.a(i) <= not held.a(i);
        when 7 =>
          -- From a nested procedure, then replaced.
          touch(i, not r.a(i));
          r <= held;
        when 9 =>
          -- From a nested procedure, last: takes effect.
          r <= held;
          touch(i, not held.a(i));
        when 11 =>
          -- Delayed past the next edges, preempted there by a held whole drive.
          -- (One instance only: a waveform on a direct driver stops the skips of its process.)
          if K = 2 then
            r.a(i) <= not r.a(i) after 15 ns;
          else
            r <= held;
          end if;
          held.b := r.b;
        when 13 =>
          -- A delayed element that is left alone.
          if K = 2 then
            r.c(i mod 6) <= not r.c(i mod 6) after 15 ns;
          else
            r.c(i mod 6) <= not r.c(i mod 6);
          end if;
        when 14 | 15 | 16 =>
          -- Nothing drives: the delayed element lands.
          null;
        when 17 =>
          held := r;
          r <= held;
        when 19 =>
          held.a(i) := not held.a(i);
          held.n := (held.n + K + 1) mod 256;
          r <= held;
        when 21 =>
          -- A plain element drive holding its value at a run-time index, then the whole.
          r.c(i mod 6) <= r.c(i mod 6);
          r <= held;
        when others =>
          r <= held;
      end case;
      v(i mod W) <= not v(i mod W);
      r2.a(i) <= held.a(i);
      if n mod 11 = K then
        r2 <= (held.a, held.b);
      end if;
      x := r;
      if x.a(i) = '1' then t := t + i + 1; end if;
      if r.a(i) = '1' then t := t + 100; end if;
      if x /= r then report "whole read differs from the signal" severity error; end if;
      t := (t + sum(x)) mod 1000003;
      o <= t;
    end if;
  end process;

  watch : process (r)
  begin
    report integer'image(K) & " r " & to_hstring(r.a) & " " & std_logic'image(r.b) & " "
      & to_hstring(r.c) & " " & integer'image(r.n);
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use work.stack_gate_p.all;

entity stack_gate_index is
end entity;

architecture a of stack_gate_index is
  signal clk : std_logic := '0';
  signal n : natural := 0;
  signal o0, o1, o2 : natural := 0;
  signal r2 : rrec := (rec0.a, rec0.b);
begin
  clk <= not clk after 5 ns when n < 400 else '0';
  count : process (clk)
  begin
    if rising_edge(clk) then n <= n + 1; end if;
  end process;
  i0 : entity work.stack_gate_inst generic map (0, 8) port map (clk, n, o0, r2);
  i1 : entity work.stack_gate_inst generic map (1, 32) port map (clk, n, o1, r2);
  i2 : entity work.stack_gate_inst generic map (2, 70) port map (clk, n, o2, r2);
  done : process (n)
  begin
    if n = 399 then
      report integer'image(o0) & " " & integer'image(o1) & " " & integer'image(o2) & " " & to_hstring(r2.a);
    end if;
  end process;
end architecture;
