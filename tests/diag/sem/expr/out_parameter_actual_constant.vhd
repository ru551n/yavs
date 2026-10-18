-- yavs-test: std=2008
-- expect: E1360 14:8 constant `k` cannot be the actual for `out` parameter `x`
-- LRM 4.2.2
package p is
  procedure pr (x : out integer);
end package;
entity e is end entity;
use work.p.all;
architecture r of e is
begin
  process
    constant k : integer := 1;
  begin
    pr(k);
    wait;
  end process;
end architecture;
