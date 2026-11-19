library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity tb_counter is
end entity;

architecture tb of tb_counter is
  signal clk, rst, en : std_logic := '0';
  signal count        : unsigned(7 downto 0);
  signal wrapped      : std_logic;
begin
  clk <= not clk after 5 ns;

  dut : entity work.counter
    port map (clk => clk, rst => rst, en => en, count => count, wrapped => wrapped);

  main : process
  begin
    rst <= '1';
    wait for 20 ns;
    rst <= '0';
    en <= '1';
    for i in 1 to 300 loop
      wait until rising_edge(clk);
    end loop;
    wait for 1 ns;
    assert count = 44 report "expected 44, got " & integer'image(to_integer(count)) severity failure;
    report "300 edges counted: " & integer'image(to_integer(count));
    std.env.finish;
  end process;
end architecture;
