-- Cycle mode: asynchronous resets asserted and released between clock edges, beside a clocked
-- process without one, which must not run on a reset wake. With COMB the region has a
-- combinational input and waits on everything; without, it has the kernel's edge trigger with
-- the reset. A reset made inside the region (`rin`, a register) keeps its process out.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_reset is
  generic (COMB : boolean := false);
end entity;
architecture a of cycle_reset is
  signal clk, rst, din, rin : std_ulogic := '0';
  signal n, m, k : unsigned(7 downto 0) := x"00";
  signal c : std_ulogic := '0';
begin
  free : process (clk)
  begin
    if rising_edge(clk) then
      n <= n + 1;
    end if;
  end process;
  ar : process (clk, rst)
  begin
    if rst = '1' then
      m <= x"40";
    elsif rising_edge(clk) then
      m <= m + 3;
    end if;
  end process;
  g : if COMB generate
    c <= din xor n(0);
  end generate;
  rreg : process (clk)
  begin
    if rising_edge(clk) then
      rin <= din;
    end if;
  end process;
  ar2 : process (clk, rin)
  begin
    if rin = '1' then
      k <= x"00";
    elsif rising_edge(clk) then
      k <= k + 1;
    end if;
  end process;
  main : process
  begin
    for i in 1 to 20 loop
      din <= '1' when i mod 7 = 0 else '0';
      if i = 5 or i = 13 then
        rst <= '1';
        wait for 2 ns;
        rst <= '0';
        wait for 1 ns;
      elsif i = 9 then
        rst <= '1';
        wait for 3 ns;
      else
        rst <= '0';
        wait for 3 ns;
      end if;
      clk <= '1';
      wait for 5 ns;
      clk <= '0';
      wait for 2 ns;
      report "i=" & integer'image(i) & " n=" & to_hstring(n) & " m=" & to_hstring(m)
        & " k=" & to_hstring(k) & " c=" & std_ulogic'image(c);
    end loop;
    wait;
  end process;
end architecture;
