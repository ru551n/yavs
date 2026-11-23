-- Asserts below the top that compare with metavalue literals (A74, amending A62): a two-state
-- value never equals a metavalue, so `d /= (d'range => 'X')` holds with every `--init-x`, in
-- every mode. Widths 1 to 128.
library ieee;
use ieee.std_logic_1164.all;

entity xa_dut is
  generic (W : positive);
  port (
    clk : in std_logic;
    d : in std_logic_vector(W - 1 downto 0);
    q : out std_logic_vector(W - 1 downto 0));
end entity;

architecture rtl of xa_dut is
begin
  process (clk) begin
    if rising_edge(clk) then
      assert d /= (d'range => 'X') report "W=" & integer'image(W) & " d is X" severity warning;
      assert d(0) /= 'U' report "W=" & integer'image(W) & " d(0) is U" severity warning;
      q <= d;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;

entity initxassert is
end entity;

architecture tb of initxassert is
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
    dut : entity work.xa_dut generic map (W) port map (clk, d, q);
    -- d is all 0 at the edges at 5 and 25 ns, all 1 at 15 and 35 ns.
    process begin
      for k in 0 to 3 loop
        wait until rising_edge(clk);
        wait for 1 ns;
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
