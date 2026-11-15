-- Cycle mode: asynchronous resets made inside the region. A synchroniser on c1 (itself reset from
-- outside, held at time 0) makes r2; processes on c1 and on c2 (twice the rate, the same clock
-- process) use it; one reset branch reads a signal that changes while the reset holds; one reset
-- is logic of r2 and a register; one is a register of c2 used on c1; a reset member's register is
-- read by a register of the other clock. The outside reset is pulsed between edges at the end.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_inreset is
  generic (N : natural := 30);
end entity;
architecture a of cycle_inreset is
  signal c1 : std_ulogic := '0';
  signal c2 : std_ulogic := '1';
  signal arst : std_ulogic := '1';
  signal r1, r2, soft, rc, r5 : std_ulogic := '0';
  signal din, k2, n1, n2, n3, n4, n5 : unsigned(7 downto 0) := x"00";
  signal done : boolean := false;
begin
  sync : process (c1, arst)
  begin
    if arst = '1' then
      r1 <= '1';
      r2 <= '1';
    elsif rising_edge(c1) then
      r1 <= '0';
      r2 <= r1;
    end if;
  end process;
  q1 : process (c1, r2)
  begin
    if r2 = '1' then
      n1 <= x"10";
    elsif rising_edge(c1) then
      n1 <= n1 + 1;
      soft <= '1' when n1 = x"18" else '0';
    end if;
  end process;
  -- The other clock, the same reset: reads q1's register.
  q2 : process (c2, r2)
  begin
    if r2 = '1' then
      n2 <= x"20";
    elsif rising_edge(c2) then
      n2 <= n2 + n1;
    end if;
  end process;
  -- While the reset holds, n3 follows din.
  q3 : process (c1, r2, din)
  begin
    if r2 = '1' then
      n3 <= din;
    elsif rising_edge(c1) then
      n3 <= n3 + 2;
    end if;
  end process;
  rc <= r2 or soft;
  q4 : process (c1, rc)
  begin
    if rc = '1' then
      n4 <= x"00";
    elsif rising_edge(c1) then
      n4 <= n4 + 1;
    end if;
  end process;
  -- A reset made by a register of the other clock.
  c2reg : process (c2)
  begin
    if rising_edge(c2) then
      k2 <= k2 + 1;
      r5 <= k2(2);
    end if;
  end process;
  q5 : process (c1, r5)
  begin
    if r5 = '0' then
      n5 <= x"05";
    elsif rising_edge(c1) then
      n5 <= n5 + 1;
    end if;
  end process;
  clocks : process
  begin
    for k in 0 to 4 * N - 1 loop
      wait for 2.5 ns;
      c2 <= not c2;
      if k mod 2 = 1 then
        c1 <= not c1;
      end if;
    end loop;
    done <= true;
    wait;
  end process;
  main : process
  begin
    for i in 1 to 6 loop
      wait for 3.1 ns;
      din <= din + 9;
    end loop;
    arst <= '0';
    wait for 200 ns;
    arst <= '1';
    wait for 1.5 ns;
    din <= din + 1;
    wait for 1.5 ns;
    arst <= '0';
    wait;
  end process;
  mon : process
  begin
    wait for 1.2 ns;
    while not done loop
      wait for 10 ns;
      report "r2=" & std_ulogic'image(r2) & " n1=" & to_hstring(n1) & " n2=" & to_hstring(n2)
        & " n3=" & to_hstring(n3) & " n4=" & to_hstring(n4) & " n5=" & to_hstring(n5)
        & " soft=" & std_ulogic'image(soft);
    end loop;
    wait;
  end process;
end architecture;
