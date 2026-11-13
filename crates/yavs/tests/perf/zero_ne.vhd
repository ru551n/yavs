-- A RAM scan with `/=` against a zero word, on the signal and on a variable copy: predefined `/=`
-- of logic vectors is one word-wise compare, so the operations per element do not grow with
-- the width (a bit-by-bit loop made it 6 per bit).
library ieee;
use ieee.std_logic_1164.all;
entity zero_ne is end;
architecture a of zero_ne is
  type mem_t is array (0 to 15) of std_logic_vector(47 downto 0);
  constant ZW : std_logic_vector(47 downto 0) := (others => '0');
  signal mem : mem_t := (3 => (40 => '1', others => '0'), 9 => (others => 'U'), others => ZW);
  signal clk : std_logic := '0';
begin
  clk <= not clk after 5 ns when now < 200 ns;
  process (clk)
    variable n : natural := 0;
    variable v : mem_t;
  begin
    if rising_edge(clk) then
      v := mem;
      for i in mem'range loop
        if mem(i) /= ZW then n := n + 1; end if;
        if v(i) /= ZW then n := n + 1; end if;
      end loop;
      if now > 190 ns then report "n=" & integer'image(n); end if;
    end if;
  end process;
end;
