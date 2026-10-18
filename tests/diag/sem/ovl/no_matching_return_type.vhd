-- yavs-test: std=2008
-- expect: E1301 14:10 no function `f` returns `bit`
-- LRM 12.5
package p is
  function f return integer;
end package;
entity e is end entity;
use work.p.all;
architecture r of e is
begin
  process
    variable v : bit;
  begin
    v := f;
    wait;
  end process;
end architecture;
