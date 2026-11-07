library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity counter_tb is
end entity;

architecture sim of counter_tb is
  -- The component has the module's name, generics and ports.
  component counter is
    generic (WIDTH : integer := 4);
    port (clk, rst : in std_logic; q : out std_logic_vector(WIDTH - 1 downto 0));
  end component;
  signal clk, rst : std_logic := '0';
  signal q : std_logic_vector(7 downto 0);
begin
  dut : counter generic map (WIDTH => 8) port map (clk => clk, rst => rst, q => q);

  process
  begin
    rst <= '1';
    wait for 5 ns;
    rst <= '0';
    for i in 1 to 3 loop
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 5 ns;
      report "q = " & integer'image(to_integer(unsigned(q)));
    end loop;
    assert unsigned(q) = 3 severity failure;
    wait;
  end process;
end architecture;
