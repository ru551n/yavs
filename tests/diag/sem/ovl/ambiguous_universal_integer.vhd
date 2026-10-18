-- yavs-test: std=2008
-- expect: E1302 15:8 integer literal is ambiguous
-- LRM 9.3.6
package p is
  type t1 is range 0 to 9;
  type t2 is range 0 to 9;
  procedure pr (x : t1);
  procedure pr (x : t2);
end package;
entity e is end entity;
use work.p.all;
architecture r of e is
begin
  process begin
    pr(3);
    wait;
  end process;
end architecture;
