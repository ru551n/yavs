-- Cycle mode: a register reset asynchronously by another register changes twice in one time step
-- in the normal mode (at the edge, then when the reset register's new value arrives). A process
-- below the top that waits on it and reads logic following it (`list`) sees the value between,
-- so that register stays in the event kernel with the logic, as the logic alone did; the register
-- making the reset and the one sampling `y` join. Found by the fuzzer (cyclefuzz.py).
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_runreset_dut is
  port (
    clk, rst : in std_ulogic;
    d : in unsigned(7 downto 0);
    q, y, s : out unsigned(7 downto 0) := (others => '0')
  );
end entity;
architecture rtl of cycle_runreset_dut is
  signal r : std_ulogic := '0';
  signal b : unsigned(7 downto 0) := (others => '0');
begin
  rreg : process (clk)
  begin
    if rising_edge(clk) then
      r <= rst;
    end if;
  end process;

  qreg : process (clk, r)
  begin
    if r = '1' then
      q <= (others => '0');
    elsif rising_edge(clk) then
      q <= d;
    end if;
  end process;

  b <= q + 6;

  list : process (q)
  begin
    y <= b + q;
  end process;

  sreg : process (clk)
  begin
    if rising_edge(clk) then
      s <= y;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_runreset is
end entity;
architecture a of cycle_runreset is
  signal clk, rst : std_ulogic := '0';
  signal d, q, y, s : unsigned(7 downto 0) := (others => '0');
begin
  clk <= not clk after 5 ns;
  dut : entity work.cycle_runreset_dut port map (clk, rst, d, q, y, s);

  stim : process
  begin
    for i in 0 to 11 loop
      wait until falling_edge(clk);
      report "y=" & to_hstring(y) & " s=" & to_hstring(s) & " q=" & to_hstring(q);
      d <= d + 37;
      rst <= '1' when i mod 4 = 1 else '0';
    end loop;
    std.env.finish;
  end process;
end architecture;
