-- Aggregates of one range choice filling a logic vector of a generic width (the RAM check
-- `mem(i) /= (mem(i)'range => '0')`, a fill with a variable's value, a range choice under a
-- constrained context): each is one plane fill, so the operations per element do not grow with
-- the width (an element loop cost five per bit).
library ieee;
use ieee.std_logic_1164.all;
entity zero_fill is generic (W : positive := 48); end;
architecture a of zero_fill is
  type mem_t is array (natural range <>) of std_logic_vector;
  subtype word_t is std_logic_vector(W - 1 downto 0);
  signal mem : mem_t(0 to 15)(W - 1 downto 0) := (5 => (7 => '1', others => '0'), others => (others => '0'));
  signal clk : std_logic := '0';
begin
  clk <= not clk after 5 ns when now < 200 ns;
  process (clk)
    variable n : natural := 0;
    variable b : std_logic := '0';
    variable x : word_t;
  begin
    if rising_edge(clk) then
      for i in mem'range loop
        if mem(i) /= (mem(i)'range => '0') then n := n + 1; end if;
        x := (x'range => b);
        if x = word_t'(W - 1 downto 0 => '0') then n := n + 2; end if;
      end loop;
      b := not b;
      if now > 190 ns then report "n=" & integer'image(n); end if;
    end if;
  end process;
end;
