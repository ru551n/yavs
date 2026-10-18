-- yavs-test: std=2008
-- expect: E1302 14:10 call of `to_integer` is ambiguous
-- help: qualify the operand or use a selected name (`ieee.numeric_std.to_integer`)
-- LRM 12.4
library ieee;
use ieee.numeric_std.all;
use ieee.numeric_bit.all;
entity e is end entity;
architecture r of e is
begin
  process
    variable x : integer;
  begin
    x := to_integer("0101");
    wait;
  end process;
end architecture;
