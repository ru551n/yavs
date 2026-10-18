-- yavs-test: std=2008
-- expect: none
-- LRM 9.3.5
package p is
  type t1 is (a, b);
  type t2 is (a, c);
  procedure pr (x : t1);
  procedure pr (x : t2);
end package;
entity e is end entity;
use work.p.all;
architecture r of e is
begin
  process begin
    pr(t1'(a));
    pr(c);
    wait;
  end process;
end architecture;
