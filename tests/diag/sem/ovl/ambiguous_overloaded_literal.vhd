-- yavs-test: std=2008
-- expect: E1302 16:8 enumeration literal `a` is ambiguous
-- help: qualify the literal: `t1'(a)`
-- LRM 12.5
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
    pr(a);
    wait;
  end process;
end architecture;
