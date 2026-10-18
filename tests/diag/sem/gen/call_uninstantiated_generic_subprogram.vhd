-- yavs-test: std=2008
-- expect: E1505 14:10 `id` is a generic subprogram and must be instantiated before use
-- LRM 4.4
package p is
  function id generic (type t) parameter (x : t) return t;
end package;
entity e is end entity;
use work.p.all;
architecture r of e is
begin
  process
    variable v : integer;
  begin
    v := id(1);
    wait;
  end process;
end architecture;
