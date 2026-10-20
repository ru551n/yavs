-- difftest: top=skip_unchanged std=2008 stop=300ns
-- Assignments that change nothing: the delta cycle they cause stays (postponed processes see it).
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity skip_unchanged is end;
architecture a of skip_unchanged is
  signal clk : std_logic := '0';
  signal rst : std_logic := '1';
  signal q : std_logic_vector(3 downto 0) := (others => '0');
  signal y : std_logic_vector(3 downto 0);
  signal x, z : std_logic := '0';
  signal b : boolean := false;
  signal n : integer := 0;
  signal bus1 : std_logic_vector(1 downto 0) := "00";
begin
  clk <= not clk after 5 ns when now < 200 ns;
  rst <= '0' after 23 ns;
  process (clk, rst) begin
    if rst = '1' then q <= (others => '0');
    elsif rising_edge(clk) then q <= std_logic_vector(unsigned(q) + 1);
    end if;
  end process;
  process (q) begin y <= q; x <= x; b <= b; end process;
  process (clk) begin if rising_edge(clk) then z <= z; n <= n; end if; end process;
  process (clk) begin if rising_edge(clk) then z <= not z; z <= z; end if; end process;
  process begin
    wait for 12 ns; x <= x; bus1(0) <= '0'; wait; end process;
  process begin wait for 31 ns; bus1(1) <= '0'; bus1(1) <= '1'; bus1(1) <= '0'; wait; end process;
  postponed process begin
    wait for 12 ns; report "pp at " & time'image(now) & " y=" & to_string(y);
    wait for 19 ns; report "pp2 at " & time'image(now);
    wait for 40 ns;
    report "pp3" ; wait;
  end process;
  process begin wait for 50 ns; x <= x after 0 ns; wait for 0 ns; report "w0"; x <= x after 1 ns; x <= x after 5 ns;
    wait for 20 ns; x <= x; x <= '1' after 3 ns; x <= x after 3 ns; wait;
  end process;
  process (x) begin report "x event " & std_logic'image(x); end process;
end;
