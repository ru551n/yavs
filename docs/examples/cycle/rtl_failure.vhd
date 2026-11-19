library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity accumulator is
  port (
    clk, rst : in  std_logic;
    d        : in  unsigned(7 downto 0);
    q        : out unsigned(7 downto 0)
  );
end entity;

architecture rtl of accumulator is
  signal acc : unsigned(7 downto 0) := (others => '0');
begin
  process (clk)
  begin
    if rising_edge(clk) then
      if rst = '1' then
        acc <= (others => '0');
      else
        acc <= acc + d;
      end if;
      assert acc /= 200 report "acc reached 200" severity warning;
      if acc = 250 then
        report "acc reached 250, the design ends the test" severity failure;
      end if;
    end if;
  end process;
  q <= acc;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity tb_rtl_failure is
end entity;

architecture tb of tb_rtl_failure is
  signal clk, rst : std_logic := '0';
  signal d, q     : unsigned(7 downto 0) := (others => '0');
begin
  clk <= not clk after 5 ns;

  dut : entity work.accumulator port map (clk => clk, rst => rst, d => d, q => q);

  main : process
  begin
    rst <= '1';
    wait for 22 ns;
    rst <= '0';
    d <= to_unsigned(10, 8);
    wait for 1 us;
    report "testbench finished, q=" & to_hstring(q);
    std.env.finish;
  end process;
end architecture;
