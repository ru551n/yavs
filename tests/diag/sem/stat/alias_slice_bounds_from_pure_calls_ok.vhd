-- yavs-test: std=2008
-- expect: none
-- LRM 9.4.3, 6.6.2 (OSVVM WishboneRegisterSubordinate: a pure call with globally static actuals is globally static; sim1, sim2 and sim3 accept)
library ieee; use ieee.std_logic_1164.all; use ieee.math_real.all;
entity e is
  generic (DATA_NUM_BYTES : positive := 4);
  port (adr : in std_logic_vector(31 downto 0));
end entity;
architecture a of e is
  constant BYTE_ADDR_WIDTH : integer := integer(ceil(log2(real(DATA_NUM_BYTES))));
begin
  process
    variable Local : std_logic_vector(31 downto 0);
    alias LocalByteAdr is Local(BYTE_ADDR_WIDTH - 1 downto 0);
  begin
    LocalByteAdr := (others => '0');
    wait;
  end process;
end architecture;
