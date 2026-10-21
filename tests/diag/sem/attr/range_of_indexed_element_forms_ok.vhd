-- yavs-test: std=2008
-- expect: none
-- LRM 16.2.3, 9.3.3.3 (hdl-modules math_pkg, addr_pkg, axi_lite_register_file, saturate_signed; sim1 accepts)
package p is
  type slv_vec is array (natural range <>) of bit_vector;
  type rr is record data : bit_vector(31 downto 0); end record;
  type s2m_t is record read : rr; end record;
end package;
use work.p.all;
entity e is end entity;
architecture a of e is
  signal axi : s2m_t;
  signal result : bit_vector(7 downto 0);
  signal g : bit_vector(3 downto 0);
begin
  process
    variable values : slv_vec(0 to 3)(7 downto 0);
    variable addrs : slv_vec(0 to 1)(15 downto 0);
    variable n : integer;
  begin
    for k in values(0)'range loop
      n := k;
    end loop;
    for k in addrs(0)'reverse_range loop
      n := k;
    end loop;
    axi.read.data(values(0)'range) <= (others => '0');
    result <= (others => not g(g'high));
    result <= axi.read.data(addrs(1)'low + 7 downto 0);
    n := addrs(1)'length + values(2)'high;
    wait;
  end process;
end architecture;
