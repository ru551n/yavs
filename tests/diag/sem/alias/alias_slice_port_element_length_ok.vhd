-- yavs-test: std=2008
-- expect: none
-- LRM 6.6.2, 9.4.3: 'length of an element of a port record is globally static like that of the port
-- (OSVVM WishboneRegisterSubordinate; sim1, sim2 and sim3 accept)
package p is
  type rec_t is record
    adr : bit_vector;
    dat : bit_vector;
  end record;
end package;
use work.p.all;
entity e is
  port (b : inout rec_t);
end entity;
architecture a of e is
  constant dw : integer := b.dat'length;
  constant nb : integer := dw / 8;
begin
  process
    variable l : b'subtype;
    alias la : bit_vector(l.adr'length - 1 downto 0) is l.adr;
    alias lb is la(nb - 1 downto 0);
  begin
    wait;
  end process;
end architecture;
