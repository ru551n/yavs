library ieee;
use ieee.numeric_std.all;
use std.textio.all;

entity t is
end entity;

architecture a of t is
begin
  process
    variable l : line;
    variable s : unsigned(8 downto 0);
  begin
    s := resize(to_unsigned(200, 8), 9) + to_unsigned(100, 9);
    write(l, string'("distro-ok sum="));
    write(l, to_integer(s));
    writeline(output, l);
    wait;
  end process;
end architecture;
