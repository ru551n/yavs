-- yavs-test: std=2008
-- expect: none
-- LRM 6.5.7.1
package p is
  function f (a : integer; b : integer := 3) return integer;
end package;
entity e is end entity;
use work.p.all;
architecture r of e is
begin
  process
    variable v : integer;
  begin
    v := f(1);
    v := f(b => 2, a => 1);
    wait;
  end process;
end architecture;
