-- Cycle mode: a register `q` reset asynchronously by another register (`r`) changes twice in one
-- time step in the normal mode when `r` rises at an edge that also changes `q`: for one delta it
-- holds the edge's value, then the reset's. That pulse resets `t` and clocks `t2`. The region
-- settles the edge and the reset together and showed no pulse, also to `t` and `t2` outside it,
-- so `q` stays in the event kernel; `r` and `t` join. Found by probing the fuzzer's reset finding.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_pulse_dut is
  port (
    clk, rst : in std_ulogic;
    q, r : buffer std_ulogic := '0';
    t, t2 : out unsigned(7 downto 0) := (others => '0')
  );
end entity;
architecture rtl of cycle_pulse_dut is
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
      q <= '0';
    elsif rising_edge(clk) then
      q <= not q;
    end if;
  end process;

  treg : process (clk, q)
  begin
    if q = '1' then
      t <= (others => '0');
    elsif rising_edge(clk) then
      t <= t + 1;
    end if;
  end process;

  t2reg : process (q)
  begin
    if rising_edge(q) then
      t2 <= t2 + 1;
    end if;
  end process;
end architecture;

library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_pulse is
end entity;
architecture a of cycle_pulse is
  signal clk, rst, q, r : std_ulogic := '0';
  signal t, t2 : unsigned(7 downto 0) := (others => '0');
begin
  clk <= not clk after 5 ns;
  dut : entity work.cycle_pulse_dut port map (clk, rst, q, r, t, t2);

  stim : process
  begin
    for i in 0 to 11 loop
      wait until falling_edge(clk);
      report "q=" & std_ulogic'image(q) & " r=" & std_ulogic'image(r) & " t=" & to_hstring(t)
        & " t2=" & to_hstring(t2);
      rst <= '1' when i mod 5 = 1 or i mod 7 = 3 else '0';
    end loop;
    std.env.finish;
  end process;
end architecture;
