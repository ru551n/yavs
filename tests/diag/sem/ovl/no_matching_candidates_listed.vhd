-- yavs-test: std=2008
-- expect: E1301 16:10 no function `f` matches the call
-- help: candidates: `f (x: integer) return integer`, `f (x: boolean) return integer`
-- LRM 12.5
package p is
  function f (x : integer) return integer;
  function f (x : boolean) return integer;
end package;
entity e is end entity;
use work.p.all;
architecture r of e is
begin
  process
    variable v : integer;
  begin
    v := f("abc");
    wait;
  end process;
end architecture;
