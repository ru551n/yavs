-- difftest: top=agg_targets std=2008
library ieee;
use ieee.std_logic_1164.all;
entity agg_targets is end;
architecture t of agg_targets is
  signal x, y : std_ulogic := '0';
  signal p, q : bit := '0';
  type ivec is array (1 to 2) of integer;
  signal i, j : integer := 0;
begin
  process
  begin
    (x, y) <= std_ulogic_vector'("10");
    (p, q) <= bit_vector'("01") after 1 ns, bit_vector'("11") after 2 ns;
    (i, j) <= ivec'(5, 6);
    wait for 0 ns;
    report "x=" & std_ulogic'image(x) & " y=" & std_ulogic'image(y) & " i=" & integer'image(i)
      & " j=" & integer'image(j);
    wait for 1 ns;
    report "p=" & bit'image(p) & bit'image(q);
    wait for 1 ns;
    report "p=" & bit'image(p) & bit'image(q);
    wait;
  end process;
end;
