-- A74: under `--two-state` a metavalue literal that is compared keeps its meaning (a two-state
-- value never equals it) and its process joins the region: `d = (others => 'X')`, a `case` choice
-- "0X", `std_match` with '-'.
-- Widths 1, 63, 64, 65 and 128.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity mc_dut is
  generic (W : positive);
  port (
    clk : in std_logic;
    d : in std_logic_vector(W - 1 downto 0);
    sel : in std_logic_vector(1 downto 0);
    hit, c, m : out std_logic);
end entity;

architecture rtl of mc_dut is
begin
  process (clk) begin
    if rising_edge(clk) then
      if d = (d'range => 'X') then
        hit <= '1';
      else
        hit <= '0';
      end if;
      case sel is
        when "0X" => c <= '1';
        when others => c <= '0';
      end case;
      if std_match(d, (d'range => '-')) then
        m <= '1';
      else
        m <= '0';
      end if;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity metacmp is
  generic (ONES : boolean := false);
end entity;

architecture tb of metacmp is
  type widths_t is array (0 to 4) of positive;
  constant WIDTHS : widths_t := (1, 63, 64, 65, 128);
  signal clk : std_logic := '0';
  signal sel : std_logic_vector(1 downto 0) := "00";
begin
  clk <= not clk after 5 ns;
  sel <= "01" when ONES else "00";
  g : for i in WIDTHS'range generate
    constant W : positive := WIDTHS(i);
    signal d : std_logic_vector(W - 1 downto 0);
    signal hit, c, m : std_logic;
  begin
    d <= (others => '1') when ONES else (others => '0');
    dut : entity work.mc_dut generic map (W) port map (clk, d, sel, hit, c, m);
    process begin
      wait until rising_edge(clk);
      wait for 1 ns;
      report "W=" & integer'image(W) & " hit=" & std_logic'image(hit) & " c="
        & std_logic'image(c) & " m=" & std_logic'image(m);
      wait;
    end process;
  end generate;
  process begin
    wait for 30 ns;
    std.env.finish;
  end process;
end architecture;
