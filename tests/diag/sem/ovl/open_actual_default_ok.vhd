-- yavs-test: std=2008
-- expect: none
-- LRM 6.5.6.2, 6.5.7.1 (open for a formal with a default; the reference simulator accepts)
package p is
  function f (a : integer := 5; b : integer := 7) return integer;
  procedure q (a : integer := 5; b : integer := 7);
end package;
entity e is end entity;
use work.p.all;
architecture r of e is
begin
  process
    variable v : integer;
  begin
    v := f(a => open, b => 1);
    v := f(2, open);
    q(open, 1);
    q(b => open);
    wait;
  end process;
end architecture;
