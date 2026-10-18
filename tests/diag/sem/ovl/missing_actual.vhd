-- yavs-test: std=2008
-- expect: E1311 14:10 formal `b` has no actual and no default
-- LRM 6.5.7.1
package p is
  function f (a, b : integer) return integer;
end package;
entity e is end entity;
use work.p.all;
architecture r of e is
begin
  process
    variable v : integer;
  begin
    v := f(a => 1);
    wait;
  end process;
end architecture;
