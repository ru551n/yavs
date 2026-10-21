-- yavs-test: std=2008
-- expect: none
-- LRM 6.3, 9.3.3.3 (OSVVM InterruptGeneratorBit: TransRec.DataFromModel <= (others => '0'); sim1 accepts)
package p is
  type srec is record d : bit_vector; n : integer; end record;
  subtype s8 is srec(d(7 downto 0));
  type arr is array (natural range <>) of bit_vector;
  type mrec is record m : arr(open)(7 downto 0); end record;
  subtype m2 is mrec(m(0 to 1));
end package;
use work.p.all;
entity e is
  port (t : inout s8);
end entity;
architecture a of e is
  signal x : s8;
  signal y, z : m2;
begin
  y.m(0) <= (others => '0');
  z.m <= (others => (others => '1'));
  process
    variable v : s8;
  begin
    t.d <= (others => '0');
    x.d <= (others => '1');
    v.d := (others => '0');
    wait;
  end process;
end architecture;
