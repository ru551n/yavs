-- yavs-test: std=2008
-- expect: none
-- LRM 16.2.3 (hdl-modules axi_lite_simple_read_crossbar; sim1 and sim2 accept)
package p is
  type rr is record data : bit_vector(7 downto 0); end record;
  type port_t is record r : rr; end record;
  type ports_t is array (natural range <>) of port_t;
end package;
use work.p.all;
entity e is
  generic (n : positive := 2);
  port (o : out ports_t(0 to n - 1); i : in ports_t(0 to n - 1));
end entity;
architecture a of e is
  signal b : bit_vector(7 downto 0);
begin
  g : for k in o'range generate
    o(k).r.data <= i(k).r.data(o(k).r.data'range);
  end generate;
  process
    variable v : integer;
  begin
    b <= b(i(0).r.data'range);
    for j in i(0).r.data'range loop
      v := j;
    end loop;
    wait;
  end process;
end architecture;
