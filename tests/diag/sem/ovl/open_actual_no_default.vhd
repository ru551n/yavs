-- yavs-test: std=2008
-- expect: E1311 14:10 formal `a` is open and has no default
-- LRM 6.5.6.2
package p is
  function f (a : integer; b : integer := 7) return integer;
end package;
entity e is end entity;
use work.p.all;
architecture r of e is
begin
  process
    variable v : integer;
  begin
    v := f(open, 1);
    wait;
  end process;
end architecture;
