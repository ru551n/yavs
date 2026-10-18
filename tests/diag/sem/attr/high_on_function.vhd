-- yavs-test: std=2008
-- expect: E1621 14:12 attribute `high` is not defined for function `f`
-- LRM 16.2
package p is
  function f return integer;
end package;
entity e is end entity;
use work.p.all;
architecture r of e is
begin
  process
    variable v : integer;
  begin
    v := f'high;
    wait;
  end process;
end architecture;
