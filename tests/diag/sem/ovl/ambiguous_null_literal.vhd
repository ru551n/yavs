-- yavs-test: std=2008
-- expect: E1302 15:8 `null` is ambiguous
-- LRM 9.3.2
package p is
  type pa is access integer;
  type pb is access bit;
  procedure pr (variable x : in pa);
  procedure pr (variable x : in pb);
end package;
entity e is end entity;
use work.p.all;
architecture r of e is
begin
  process begin
    pr(null);
    wait;
  end process;
end architecture;
