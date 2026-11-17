-- Forces and releases keep out only the drivers of what they can force (E4128): a force in a
-- unit the design never instantiates, or in a package procedure no process calls, keeps nothing
-- out; one through an external name (in a process or a package procedure) keeps the drivers of
-- its target out while members read the forced value. A force of a signal parameter keeps every
-- process out: `forcescopef.vhd`, in a library of its own (it keeps every port of the library's
-- designs apart). W is the width.
library ieee;
use ieee.std_logic_1164.all;
package fsc_pkg is
  procedure poke (v : std_ulogic_vector);
  procedure unpoke;
  procedure never_called;
end package;

package body fsc_pkg is
  procedure poke (v : std_ulogic_vector) is
  begin
    <<signal .fsc_pkg_tb.dut.mid : std_ulogic_vector(W - 1 downto 0)>> <= force v;
  end procedure;

  procedure unpoke is
  begin
    <<signal .fsc_pkg_tb.dut.mid : std_ulogic_vector(W - 1 downto 0)>> <= release;
  end procedure;

  procedure never_called is
  begin
    <<signal .fsc_nowhere.q : std_ulogic_vector(W - 1 downto 0)>> <= force (others => '1');
  end procedure;
end package body;

library ieee;
use ieee.std_logic_1164.all;
-- Analysed, never instantiated.
entity fsc_unused is
end entity;
architecture a of fsc_unused is
  signal s : std_ulogic := '0';
begin
  f : process
  begin
    s <= force '1';
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity fsc_dut is
  port (clk : in std_ulogic; a : in std_ulogic_vector(W - 1 downto 0);
        q, r : out std_ulogic_vector(W - 1 downto 0) := (others => '0'); mo, m2o : out std_ulogic_vector(W - 1 downto 0));
end entity;
architecture rtl of fsc_dut is
  signal mid, mid2 : std_ulogic_vector(W - 1 downto 0) := (others => '0');
  signal clk2 : std_ulogic;
begin
  -- The forced register: its driver stays out.
  p_mid : process (clk)
  begin
    if rising_edge(clk) then
      mid <= std_ulogic_vector(unsigned(mid) + unsigned(a) + 1);
    end if;
  end process;
  -- Members reading the forced signal.
  mid2 <= mid xor a;
  p_q : process (clk)
  begin
    if rising_edge(clk) then
      q <= mid2;
    end if;
  end process;
  -- A clock copy the testbench forces: its register stays on the copy.
  clk2 <= clk;
  p_r : process (clk2)
  begin
    if rising_edge(clk2) then
      r <= a;
    end if;
  end process;
  mo <= mid;
  m2o <= mid2;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.fsc_pkg.all;
-- The forces of one testbench, by `how`: 0 an external name in the process, 1 the package
-- procedures, another value none. Forced in the delta of the edges
-- at 20 and 50 ns, between edges at 82 and 112 ns.
entity fsc_bench is
  generic (how : natural);
  port (q, r, mo, m2o : in std_ulogic_vector(W - 1 downto 0);
        clk : out std_ulogic; a : out std_ulogic_vector(W - 1 downto 0);
        hook : inout std_ulogic_vector(W - 1 downto 0));
end entity;
architecture tb of fsc_bench is
  constant ones : std_ulogic_vector(W - 1 downto 0) := (others => '1');
  function thirds return std_ulogic_vector is
    variable v : std_ulogic_vector(W - 1 downto 0);
  begin
    for j in v'range loop
      v(j) := '1' when j mod 3 = 0 else '0';
    end loop;
    return v;
  end function;
begin
  g0 : if how = 0 generate
    forcer : process
    begin
      wait for 20 ns;
      <<signal ^.^.dut.mid : std_ulogic_vector(W - 1 downto 0)>> <= force ones;
      wait for 30 ns;
      <<signal ^.^.dut.mid : std_ulogic_vector(W - 1 downto 0)>> <= release;
      wait for 32 ns;
      <<signal ^.^.dut.mid : std_ulogic_vector(W - 1 downto 0)>> <= force thirds;
      wait for 30 ns;
      <<signal ^.^.dut.mid : std_ulogic_vector(W - 1 downto 0)>> <= release;
      wait;
    end process;
  end generate;
  g1 : if how = 1 generate
    forcer : process
    begin
      wait for 20 ns;
      poke(ones);
      wait for 30 ns;
      unpoke;
      wait for 32 ns;
      poke(thirds);
      wait for 30 ns;
      unpoke;
      wait;
    end process;
  end generate;
  main : process
    variable x : unsigned(63 downto 0) := x"9E3779B97F4A7C15";
    variable va : std_ulogic_vector(W - 1 downto 0);
  begin
    for c in 1 to 16 loop
      for j in 0 to W - 1 loop
        va(j) := x(j mod 64) xor x((j * 7 + 3) mod 64);
      end loop;
      a <= va;
      x := (x(62 downto 0) & '0') xor (x(63 downto 0) and x"000000000000001B" and (63 downto 0 => x(63)));
      x := x + 16#5851F42D#;
      clk <= '1';
      wait for 2 ns;
      -- An extra edge of the forced clock copy.
      if c = 14 then
        a <= not va;
        <<signal ^.dut.clk2 : std_ulogic>> <= force '0';
        wait for 1 ns;
        <<signal ^.dut.clk2 : std_ulogic>> <= force '1';
        wait for 1 ns;
        <<signal ^.dut.clk2 : std_ulogic>> <= release;
      else
        wait for 2 ns;
      end if;
      wait for 1 ns;
      clk <= '0';
      wait for 5 ns;
      report "c=" & integer'image(c) & " q=" & to_hstring(q) & " r=" & to_hstring(r)
        & " mid=" & to_hstring(mo) & " mid2=" & to_hstring(m2o);
    end loop;
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
entity fsc_ext_tb is
end entity;
architecture tb of fsc_ext_tb is
  signal clk : std_ulogic := '0';
  signal a, q, r, mo, m2o, hook : std_ulogic_vector(W - 1 downto 0) := (others => '0');
begin
  dut : entity work.fsc_dut port map (clk, a, q, r, mo, m2o);
  bench : entity work.fsc_bench generic map (0) port map (q, r, mo, m2o, clk, a, hook);
end architecture;

library ieee;
use ieee.std_logic_1164.all;
entity fsc_pkg_tb is
end entity;
architecture tb of fsc_pkg_tb is
  signal clk : std_ulogic := '0';
  signal a, q, r, mo, m2o, hook : std_ulogic_vector(W - 1 downto 0) := (others => '0');
begin
  dut : entity work.fsc_dut port map (clk, a, q, r, mo, m2o);
  bench : entity work.fsc_bench generic map (1) port map (q, r, mo, m2o, clk, a, hook);
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
-- Two clocks whose edges come in the first delta; a force gives `clk_b` an edge in the second
-- (at 25 ns, with an edge of `clk_a`): the logic between them stays out, so `p_rb` samples `mid`
-- before it follows the new `ra`.
entity fsc_dclk_tb is
end entity;
architecture tb of fsc_dclk_tb is
  signal clk_a : std_ulogic := '0';
  signal clk_b : std_ulogic := '1';
  signal ra, mid, rb : std_ulogic_vector(W - 1 downto 0) := (others => '0');
begin
  clk_a <= not clk_a after 5 ns;
  clk_b <= not clk_b after 5 ns;
  p_ra : process (clk_a)
  begin
    if rising_edge(clk_a) then
      ra <= std_ulogic_vector(unsigned(ra) + 3);
    end if;
  end process;
  mid <= not ra;
  p_rb : process (clk_b)
  begin
    if rising_edge(clk_b) then
      rb <= mid;
    end if;
  end process;
  forcer : process
  begin
    wait for 25 ns;
    clk_b <= force '1';
    wait for 2 ns;
    clk_b <= release;
    wait;
  end process;
  mon : process
  begin
    wait for 2 ns;
    for i in 1 to 6 loop
      report "i=" & integer'image(i) & " ra=" & to_hstring(ra) & " rb=" & to_hstring(rb);
      wait for 5 ns;
    end loop;
    wait;
  end process;
end architecture;
