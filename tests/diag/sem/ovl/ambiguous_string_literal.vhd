-- yavs-test: std=2008
-- expect: E1302 13:8 string literal is ambiguous
-- LRM 9.3.2
package p is
  procedure pr (x : string);
  procedure pr (x : bit_vector);
end package;
entity e is end entity;
use work.p.all;
architecture r of e is
begin
  process begin
    pr("01");
    wait;
  end process;
end architecture;
