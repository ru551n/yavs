-- yavs-test: std=2008
-- expect: E1100 14:12 function `f` has no parameter `zz`
-- LRM 6.5.7.1
package p is
  function f (a : integer) return integer;
end package;
entity e is end entity;
use work.p.all;
architecture r of e is
begin
  process
    variable v : integer;
  begin
    v := f(zz => 1);
    wait;
  end process;
end architecture;
