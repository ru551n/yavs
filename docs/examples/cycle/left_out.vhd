library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity status is
  port (
    clk, en : in  std_logic;
    d       : in  unsigned(7 downto 0);
    q       : out unsigned(7 downto 0);
    held    : out unsigned(7 downto 0);
    slow    : out unsigned(7 downto 0);
    sum     : out unsigned(7 downto 0)
  );
end entity;

architecture rtl of status is
  signal reg : unsigned(7 downto 0) := (others => '0');
  signal b   : unsigned(7 downto 0) := x"01";
begin
  p_reg : process (clk)
  begin
    if rising_edge(clk) then
      reg <= d;
    end if;
  end process;
  q <= reg;

  -- A latch: held keeps its value while en is 0.
  p_latch : process (en, d)
  begin
    if en = '1' then
      held <= d;
    end if;
  end process;

  -- A delay inside a clock cycle.
  p_slow : process (clk)
  begin
    if rising_edge(clk) then
      slow <= d after 1 ns;
    end if;
  end process;

  -- b is read but not in the sensitivity list.
  p_sum : process (d)
  begin
    sum <= d + b;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity tb_left_out is
end entity;

architecture tb of tb_left_out is
  signal clk, en : std_logic := '0';
  signal d, q, held, slow, sum : unsigned(7 downto 0) := (others => '0');
begin
  clk <= not clk after 5 ns;

  dut : entity work.status
    port map (clk => clk, en => en, d => d, q => q, held => held, slow => slow, sum => sum);

  main : process
  begin
    d <= x"11";
    wait for 12 ns;
    en <= '1';
    wait for 10 ns;
    d <= x"22";
    en <= '0';
    wait for 10 ns;
    report "q=" & to_hstring(q) & " held=" & to_hstring(held) & " sum=" & to_hstring(sum);
    report "q last changed " & time'image(q'last_event) & " ago";
    std.env.finish;
  end process;
end architecture;
