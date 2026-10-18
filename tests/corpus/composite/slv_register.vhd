-- difftest: top=slv_register std=2008
library ieee; use ieee.std_logic_1164.all;
entity regs is port (clk : in std_logic; d : in std_logic_vector(7 downto 0); q : out std_logic_vector(7 downto 0)); end;
architecture a of regs is begin
  process (clk) begin if rising_edge(clk) then q <= d; end if; end process;
end;
library ieee; use ieee.std_logic_1164.all, ieee.numeric_std.all;
entity slv_register is end;
architecture t of slv_register is
  signal clk : std_logic := '0';
  signal d, q : std_logic_vector(7 downto 0) := (others => '0');
begin
  u : entity work.regs port map (clk, d, q);
  process begin
    for i in 1 to 3 loop
      d <= std_logic_vector(to_unsigned(i, 8)); wait for 1 ns;
      clk <= '1'; wait for 1 ns; clk <= '0';
      report "q=" & to_string(q);
    end loop;
    wait;
  end process;
end;
