-- yavs-test: std=2008
-- expect: none
-- LRM 4.5.2
package p is
  type t is range 0 to 3;
  function "+" (a, b : t) return t;
  function "-" (a : t) return t;
end package;
entity e is end entity;
use work.p.all;
architecture r of e is
begin
  process
    variable x : t;
  begin
    x := 1 + 2;
    x := -x;
    wait;
  end process;
end architecture;
