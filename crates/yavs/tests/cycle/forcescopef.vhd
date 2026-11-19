-- With `forcescope.vhd`: a force of a signal parameter, whose target cannot be known, keeps
-- every process out (E4128); one no process reaches keeps nothing out. W is the width.
library ieee;
use ieee.std_logic_1164.all;
entity fsc_formal_tb is
end entity;
architecture tb of fsc_formal_tb is
  signal clk : std_ulogic := '0';
  signal a, q, r, mo, m2o, hook, hb : std_ulogic_vector(W - 1 downto 0) := (others => '0');
begin
  dut : entity work.fsc_dut port map (clk, a, q, r, mo, m2o);
  bench : entity work.fsc_bench generic map (2) port map (q, r, mo, m2o, clk, a, hb);
  forcer : process
    procedure poke_formal (signal s : inout std_ulogic_vector; v : std_ulogic_vector) is
    begin
      s <= force v;
    end procedure;
  begin
    wait for 20 ns;
    poke_formal(hook, (hook'range => '1'));
    wait for 30 ns;
    hook <= release;
    wait;
  end process;
end architecture;

-- A package procedure forcing a signal parameter that the design below never calls: it is in the
-- analysed closure, but no process reaches it, so ports collapse and nothing stays out for it.
library ieee;
use ieee.std_logic_1164.all;
package fsc_fpkg is
  procedure poke (signal s : inout std_ulogic_vector; v : std_ulogic_vector);
end package;
package body fsc_fpkg is
  procedure poke (signal s : inout std_ulogic_vector; v : std_ulogic_vector) is
  begin
    s <= force v;
  end procedure;
end package body;

library ieee;
use ieee.std_logic_1164.all;
use work.fsc_fpkg.all;
entity fsc_unreached_tb is
end entity;
architecture tb of fsc_unreached_tb is
  signal clk : std_ulogic := '0';
  signal a, q, r, mo, m2o, hook : std_ulogic_vector(W - 1 downto 0) := (others => '0');
begin
  dut : entity work.fsc_dut port map (clk, a, q, r, mo, m2o);
  bench : entity work.fsc_bench generic map (0) port map (q, r, mo, m2o, clk, a, hook);
end architecture;
