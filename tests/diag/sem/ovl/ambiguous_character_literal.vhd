-- yavs-test: std=2008
-- expect: E1302 13:8 character literal `'0'` is ambiguous
-- LRM 9.3.2
package p is
  procedure pr (x : bit);
  procedure pr (x : character);
end package;
entity e is end entity;
use work.p.all;
architecture r of e is
begin
  process begin
    pr('0');
    wait;
  end process;
end architecture;
