-- Two copies of one clock (`W` replaced by a width): a register on the inner copy moves onto the
-- source (A32); one on the outer copy goes back to that copy, which a register outside waits on.
-- It reads the moved register, which must then go back too, or it reads the new value a delta
-- early (fuzzer, seed 327 of the default contract). The direct register on the source stays.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_copyback_sub is
  generic (G_B : boolean := false);
  port (ca, cb : in std_ulogic; q : out unsigned(W - 1 downto 0));
end entity;
architecture a of cycle_copyback_sub is
  signal kb : std_ulogic := '0';
  signal n, m : unsigned(W - 1 downto 0) := (others => '0');
begin
  kb <= ca when G_B else cb;
  cnt : process (kb) begin
    if rising_edge(kb) then
      n <= n + 1;
    end if;
  end process;
  direct : process (cb) begin
    if rising_edge(cb) then
      m <= m + 1;
    end if;
  end process;
  smp : process (ca) begin
    if rising_edge(ca) then
      q <= n;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_copyback_top is
  generic (G_B : boolean := false);
  port (clk0, clk1 : in std_ulogic; dk : out std_ulogic := '0'; q : buffer unsigned(W - 1 downto 0));
end entity;
architecture a of cycle_copyback_top is
  signal t : unsigned(W - 1 downto 0);
begin
  dk <= clk0 when G_B else clk1;
  xo : process (dk) begin
    if rising_edge(dk) then
      t <= q after 1 ns;
    end if;
  end process;
  u : entity work.cycle_copyback_sub generic map (G_B => false) port map (ca => dk, cb => clk1, q => q);
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_copyback is
end entity;
architecture t of cycle_copyback is
  signal clk0, clk1, dk : std_ulogic := '0';
  signal q : unsigned(W - 1 downto 0);
begin
  dut : entity work.cycle_copyback_top generic map (G_B => false) port map (clk0, clk1, dk, q);
  main : process
  begin
    for k in 0 to 5 loop
      clk1 <= '1';
      wait for 5 ns;
      clk1 <= '0';
      wait for 5 ns;
      report "k=" & integer'image(k) & " q=" & to_hstring(q);
    end loop;
    wait;
  end process;
end architecture;
