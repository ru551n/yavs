-- A scan of 100-element words against zero and against a fill, compiled: equality of logic
-- vectors of more than 64 elements is computed by compiled code (the blocks before the last by
-- one byte compare), not handed to the interpreter once per compare.
library ieee;
use ieee.std_logic_1164.all;
entity zero_wide is generic (W : positive := 100); end;
architecture a of zero_wide is
  type mem_t is array (natural range <>) of std_logic_vector;
  subtype word_t is std_logic_vector(W - 1 downto 0);
  constant ZW : word_t := (others => '0');
  signal mem : mem_t(0 to 15)(W - 1 downto 0) :=
    (3 => (99 => '1', others => '0'), 9 => (64 => 'U', others => '0'), others => ZW);
  signal clk : std_logic := '0';
begin
  clk <= not clk after 5 ns when now < 200 ns;
  process (clk)
    variable n : natural := 0;
  begin
    if rising_edge(clk) then
      for i in mem'range loop
        if mem(i) /= ZW then n := n + 1; end if;
        if mem(i) = (mem(i)'range => '0') then n := n + 2; end if;
      end loop;
      if now > 190 ns then report "n=" & integer'image(n); end if;
    end if;
  end process;
end;
