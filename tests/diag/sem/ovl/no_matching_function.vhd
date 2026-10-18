-- yavs-test: std=2008
-- expect: E1301 14:10 no function `f` matches the call
-- LRM 12.5
package p is
  function f (x : integer) return integer;
end package;
entity e is end entity;
use work.p.all;
architecture r of e is
begin
  process
    variable v : integer;
  begin
    v := f(true);
    wait;
  end process;
end architecture;
