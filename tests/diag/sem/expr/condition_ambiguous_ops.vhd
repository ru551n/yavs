-- yavs-test: std=2008
-- expect: E1341 19:8 condition `??` is ambiguous
-- LRM 9.2.9
package p1 is
  type t is (x, y);
  function "??" (a : t) return boolean;
end package;
package p2 is
  function "??" (a : work.p1.t) return boolean;
end package;
use work.p1.all;
use work.p2.all;
entity e is end entity;
architecture a of e is
begin
  process
    variable v : t;
  begin
    if v then null; end if;
    wait;
  end process;
end architecture;
