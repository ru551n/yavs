-- yavs-test: std=2008
-- expect: E1302 13:8 aggregate is ambiguous
-- LRM 9.3.3
package p is
  procedure pr (x : bit_vector);
  procedure pr (x : string);
end package;
entity e is end entity;
use work.p.all;
architecture r of e is
begin
  process begin
    pr((others => '0'));
    wait;
  end process;
end architecture;
