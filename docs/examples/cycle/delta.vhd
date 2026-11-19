library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity tb_delta is
end entity;

architecture tb of tb_delta is
  signal clk : std_logic := '0';
  signal q   : unsigned(7 downto 0) := (others => '0');
  signal r   : unsigned(7 downto 0);
begin
  clk <= not clk after 5 ns;

  reg : process (clk)
  begin
    if rising_edge(clk) then
      q <= q + 1;
    end if;
  end process;

  r <= q + 100;  -- combinational: one delta after q

  main : process
  begin
    wait until rising_edge(clk) and q = 2;
    wait for 0 ns;  -- one delta after the edge
    report "a delta after the edge: q=" & integer'image(to_integer(q)) & " r=" & integer'image(to_integer(r));
    wait for 1 ns;  -- sample later
    report "1 ns after the edge:    q=" & integer'image(to_integer(q)) & " r=" & integer'image(to_integer(r));
    std.env.finish;
  end process;
end architecture;
