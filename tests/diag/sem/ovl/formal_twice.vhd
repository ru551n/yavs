-- yavs-test: std=2008
-- expect: E1310 14:20 formal `a` is associated twice
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
    v := f(a => 1, a => 2);
    wait;
  end process;
end architecture;
