-- yavs-test: std=2008
-- expect: E1302 16:8 call of `f` is ambiguous
-- help: qualify the call: `integer'(f)`
-- LRM 12.5
package p is
  function f return integer;
  function f return bit;
  procedure pr (x : integer);
  procedure pr (x : bit);
end package;
entity e is end entity;
use work.p.all;
architecture r of e is
begin
  process begin
    pr(f);
    wait;
  end process;
end architecture;
