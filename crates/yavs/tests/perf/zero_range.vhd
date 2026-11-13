-- Bounds attributes of an element of an array-of-vectors signal (`mem(i)'length`, `'left`,
-- `'low`, `'ascending`), its width a generic: the bounds come from the descriptor, so a scan reads
-- no element value for them (each read of one was four operations and the element's bits).
library ieee;
use ieee.std_logic_1164.all;
entity zero_range is generic (W : positive := 48); end;
architecture a of zero_range is
  type mem_t is array (natural range <>) of std_logic_vector;
  signal mem : mem_t(0 to 15)(W - 1 downto 0) := (others => (others => '0'));
  signal clk : std_logic := '0';
begin
  clk <= not clk after 5 ns when now < 200 ns;
  process (clk)
    variable n : natural := 0;
  begin
    if rising_edge(clk) then
      for i in mem'range loop
        n := n + mem(i)'length + mem(i)'left - mem(i)'low + boolean'pos(mem(i)'ascending);
      end loop;
      if now > 190 ns then report "n=" & integer'image(n); end if;
    end if;
  end process;
end;
