-- yavs-test: std=2008
-- expect: E1300 14:10 procedure `pr` does not return a value
-- LRM 9.3.4
package p is
  procedure pr (a : integer);
end package;
entity e is end entity;
use work.p.all;
architecture r of e is
begin
  process
    variable v : integer;
  begin
    v := pr(1);
    wait;
  end process;
end architecture;
