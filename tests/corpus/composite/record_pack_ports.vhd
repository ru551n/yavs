-- difftest: top=record_pack_ports std=2008
-- Record ports that stay ports (not collapsed): inout records of resolved fields driven from
-- both sides, an out record whose actual has a second source, and a record port associated
-- field by field, all on records whose logic fields share kernel signals (#686).
library ieee;
use ieee.std_logic_1164.all;

package record_pack_ports_p is
  type bus_t is record
    req : std_logic;
    ack : std_logic;
    data : std_logic_vector(7 downto 0);
    sel : std_logic_vector(1 downto 0);
  end record;
end package;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.record_pack_ports_p.all;

entity record_pack_ports_dev is
  generic (id : natural);
  port (b : inout bus_t; o : out bus_t; f : in std_logic_vector(7 downto 0));
end entity;

architecture a of record_pack_ports_dev is
begin
  process
  begin
    b <= ('Z', 'Z', (others => 'Z'), "ZZ");
    o <= ('0', '0', x"00", "00");
    wait until b.req = '1';
    b.ack <= '1';
    b.data <= f;
    o <= ('1', '0', f, std_logic_vector(to_unsigned(id, 2)));
    wait until b.req = '0';
    b <= ('Z', 'Z', (others => 'Z'), "ZZ");
    wait;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
use work.record_pack_ports_p.all;

entity record_pack_ports is
end entity;

architecture a of record_pack_ports is
  signal b : bus_t;
  signal o1, o2 : bus_t;
  signal f : std_logic_vector(7 downto 0) := x"3C";
  signal req, ack : std_logic;
  signal data : std_logic_vector(7 downto 0);
  signal sel : std_logic_vector(1 downto 0);
begin
  d1 : entity work.record_pack_ports_dev generic map (1) port map (b => b, o => o1, f => f);
  -- The out record's actual has a second source here: both resolve.
  d2 : entity work.record_pack_ports_dev generic map (2)
    port map (b.req => req, b.ack => ack, b.data => data, b.sel => sel, o => o2, f => x"0F");
  o2.sel <= "LL";

  process
  begin
    b <= ('0', 'L', (others => 'L'), "LL");
    req <= '0';
    ack <= 'L';
    data <= (others => 'L');
    sel <= "LL";
    wait for 1 ns;
    b.req <= '1';
    req <= '1';
    wait for 1 ns;
    report "b.ack=" & std_logic'image(b.ack) & " b.data=" & to_string(b.data)
      & " ack=" & std_logic'image(ack) & " data=" & to_string(data)
      & " o1=" & std_logic'image(o1.req) & to_string(o1.data) & to_string(o1.sel)
      & " o2=" & std_logic'image(o2.req) & to_string(o2.data) & to_string(o2.sel);
    b.req <= '0';
    req <= '0';
    wait for 1 ns;
    report "b.ack=" & std_logic'image(b.ack) & " b.data=" & to_string(b.data)
      & " ack=" & std_logic'image(ack) & " data=" & to_string(data);
    assert b.data = "LLLLLLLL" report "released" severity failure;
    wait;
  end process;
end architecture;
