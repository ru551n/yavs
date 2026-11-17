-- With `forcescope.vhd`: a force of a signal parameter, whose target cannot be known, keeps
-- every process out (E4128). W is the width.
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
