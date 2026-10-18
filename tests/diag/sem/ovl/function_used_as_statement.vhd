-- yavs-test: std=2008
-- expect: E1300 12:5 function `f` cannot be called as a statement
-- LRM 10.7
package p is
  function f return integer;
end package;
entity e is end entity;
use work.p.all;
architecture r of e is
begin
  process begin
    f;
    wait;
  end process;
end architecture;
