-- A metavalue aggregate inside an expression (not an `--init-x` site) in a two-state region: under
-- `--init-x=random` each element reads as a bit of its own (A62), not one bit spread. Widths 1 to 128.
library ieee;
use ieee.std_logic_1164.all;

entity sx_dut is
  generic (W : positive);
  port (
    clk : in std_logic;
    d : in std_logic_vector(W - 1 downto 0);
    q : out std_logic_vector(W - 1 downto 0));
end entity;

architecture rtl of sx_dut is
begin
  process (clk) begin
    if rising_edge(clk) then
      q <= d xor (d'range => 'X');
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity splatxor is
end entity;

architecture tb of splatxor is
  type widths_t is array (0 to 4) of positive;
  constant WIDTHS : widths_t := (1, 63, 64, 65, 128);
  signal clk : std_logic := '0';
begin
  clk <= not clk after 5 ns;
  g : for i in WIDTHS'range generate
    constant W : positive := WIDTHS(i);
    signal d : std_logic_vector(W - 1 downto 0) := (others => '0');
    signal q : std_logic_vector(W - 1 downto 0);
  begin
    dut : entity work.sx_dut generic map (W) port map (clk, d, q);
    process begin
      wait for 12 ns;
      report "W=" & integer'image(W) & " q=" & to_hstring(q);
      wait;
    end process;
  end generate;
  process begin
    wait for 20 ns;
    std.env.finish;
  end process;
end architecture;
