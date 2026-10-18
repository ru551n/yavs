-- yavs-test: std=2008
-- expect: E1360 14:8 `v` is a variable and cannot be the actual for signal parameter `x`
-- LRM 4.2.2
package p is
  procedure pr (signal x : in bit);
end package;
entity e is end entity;
use work.p.all;
architecture r of e is
begin
  process
    variable v : bit;
  begin
    pr(v);
    wait;
  end process;
end architecture;
