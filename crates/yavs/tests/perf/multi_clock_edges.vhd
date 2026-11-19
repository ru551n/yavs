-- Processes on two clocks: a dual-port RAM model `process (wclk, rclk)` made of two edge `if`s,
-- the same on two elements of one vector (abutting waits), and `wait until rising_edge(a) or
-- rising_edge(b)`. Each resumes only on one of its edges, not for the clocks' other edges.
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity multi_clock_edges is end;
architecture a of multi_clock_edges is
  type mem_t is array (0 to 15) of std_ulogic_vector(7 downto 0);
  signal wclk, rclk : std_ulogic := '0';
  signal cv : std_ulogic_vector(1 downto 0) := "00";
  signal wa, ra : unsigned(3 downto 0) := (others => '0');
  signal wd, rd, rd2 : std_ulogic_vector(7 downto 0) := (others => '0');
  signal n, m, m2 : natural := 0;
  signal done : boolean := false;
begin
  wclk <= not wclk after 5 ns when not done;
  rclk <= not rclk after 7 ns when not done;
  cv <= (wclk, rclk);
  ram : process (wclk, rclk)
    variable mem : mem_t := (others => (others => '0'));
  begin
    if rising_edge(wclk) then
      mem(to_integer(wa)) := wd;
      wa <= wa + 1;
      wd <= std_ulogic_vector(unsigned(wd) + 3);
    end if;
    if rising_edge(rclk) then
      rd <= mem(to_integer(ra));
      ra <= ra + 1;
    end if;
  end process;
  vec : process (cv(1), cv(0))
  begin
    if rising_edge(cv(1)) then
      rd2 <= wd;
    end if;
    if falling_edge(cv(0)) then
      rd2 <= rd;
    end if;
  end process;
  -- `cv(0)` is plain, `cv(1)` held: waiters of abutting elements, the plain one first.
  cnt2 : process
  begin
    wait on cv(0), cv(1), rclk until rising_edge(cv(1)) or falling_edge(rclk);
    m2 <= m2 + 1;
  end process;
  cnt : process
  begin
    wait until rising_edge(wclk) or rising_edge(rclk);
    m <= m + 1;
  end process;
  process
  begin
    for i in 1 to 300 loop
      wait until rising_edge(wclk) or rising_edge(rclk);
      n <= n + 1;
    end loop;
    report "edges=" & integer'image(n) & " rd=" & to_hstring(rd) & " rd2=" & to_hstring(rd2) & " m=" & integer'image(m);
    done <= true;
    wait;
  end process;
end;
