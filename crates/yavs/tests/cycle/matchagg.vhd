-- A `'-'` mask given as an aggregate whose length a generic sets, `std_match(d, (d'range => '-'))`
-- and `?=`: it matches anything in the region too (the region once compared with 0 there). An
-- aggregate of a `'-'` and other values keeps a matching comparison out. Widths 1 to 128.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity ma_dut is
  generic (W : positive);
  port (
    clk : in std_logic;
    d : in std_logic_vector(W - 1 downto 0);
    m, q, x : out std_logic);
end entity;

architecture rtl of ma_dut is
  subtype word_t is std_logic_vector(W - 1 downto 0);
begin
  process (clk) begin
    if rising_edge(clk) then
      if std_match(d, (d'range => '-')) then
        m <= '1';
      else
        m <= '0';
      end if;
      q <= d ?= (d'range => '-');
    end if;
  end process;
  process (clk) begin
    if rising_edge(clk) then
      if std_match(d, word_t'(0 => '-', others => '1')) then
        x <= '1';
      else
        x <= '0';
      end if;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity matchagg is
end entity;

architecture tb of matchagg is
  type widths_t is array (0 to 4) of positive;
  constant WIDTHS : widths_t := (1, 63, 64, 65, 128);
  signal clk : std_logic := '0';
begin
  clk <= not clk after 5 ns;
  g : for i in WIDTHS'range generate
    constant W : positive := WIDTHS(i);
    signal d : std_logic_vector(W - 1 downto 0) := (others => '1');
    signal m, q, x : std_logic;
  begin
    dut : entity work.ma_dut generic map (W) port map (clk, d, m, q, x);
    process begin
      for k in 0 to 2 loop
        wait until rising_edge(clk);
        wait for 1 ns;
        report "W=" & integer'image(W) & " m=" & std_logic'image(m) & " q="
          & std_logic'image(q) & " x=" & std_logic'image(x);
        d <= not d;
      end loop;
      wait;
    end process;
  end generate;
  process begin
    wait for 40 ns;
    std.env.finish;
  end process;
end architecture;
