-- yavs-test: std=2008
-- expect: E1301 12:11 procedure `pr` takes 1 argument, found 2
-- LRM 6.5.7.1
package p is
  procedure pr (a : integer);
end package;
entity e is end entity;
use work.p.all;
architecture r of e is
begin
  process begin
    pr(1, 2);
    wait;
  end process;
end architecture;
