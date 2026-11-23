-- A clocked checker of the top-level unit that compares a member's register with metavalue
-- literals (`q /= 'X'`, `v = (v'range => 'U')`) checks as in the normal mode: it never fails on
-- the region's 0 and 1. Widths 1 to 128.
library ieee;
use ieee.std_logic_1164.all;

entity tm_dut is
  generic (W : positive);
  port (clk : in std_logic; q : out std_logic; v : out std_logic_vector(W - 1 downto 0));
end entity;

architecture rtl of tm_dut is
  signal r : std_logic := '0';
  signal t : std_logic_vector(W - 1 downto 0) := (others => '0');
begin
  process (clk) begin
    if rising_edge(clk) then
      r <= not r;
      t <= not t;
    end if;
  end process;
  q <= r;
  v <= t;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity topmeta is
end entity;

architecture tb of topmeta is
  type widths_t is array (0 to 4) of positive;
  constant WIDTHS : widths_t := (1, 63, 64, 65, 128);
  signal clk : std_logic := '0';
begin
  clk <= not clk after 5 ns;
  g : for i in WIDTHS'range generate
    signal q : std_logic;
    signal v : std_logic_vector(WIDTHS(i) - 1 downto 0);
  begin
    dut : entity work.tm_dut generic map (WIDTHS(i)) port map (clk, q, v);
    process (clk) begin
      if rising_edge(clk) then
        assert q /= 'X' report "q is X" severity error;
        assert v /= (v'range => 'U') report "v is U" severity error;
      end if;
    end process;
  end generate;
  process begin
    wait for 100 ns;
    report "done";
    std.env.finish;
  end process;
end architecture;
