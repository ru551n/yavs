-- difftest: top=async_triggers std=2008 stop=400ns
-- Clocked processes with an asynchronous reset or the 'event form: events that cannot do anything are skipped.
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity async_triggers is end;
architecture a of async_triggers is
  signal clk : std_logic := '0';
  signal rst, rstn : std_logic := '1';
  signal en : std_logic := '0';
  signal q1, q2, q3, q4, q5 : unsigned(3 downto 0) := (others => '0');
  signal cnt : integer := 0;
  signal cv : std_logic_vector(1 downto 0) := "00";
begin
  clk <= not clk after 5 ns when now < 400 ns;
  rst <= '0' after 23 ns, '1' after 103 ns, '0' after 113 ns, 'X' after 150 ns, '0' after 155 ns;
  rstn <= '0' after 3 ns, '1' after 37 ns, '0' after 205 ns, '1' after 225 ns;
  en <= '1' after 45 ns, '0' after 95 ns, '1' after 120 ns;
  cv(1) <= 'H' after 60 ns, '1' after 70 ns, 'X' after 80 ns, '1' after 90 ns, '0' after 100 ns, 'L' after 101 ns;
  cv(0) <= clk;
  p1: process (clk, rst) begin
    if rst = '1' then q1 <= (others => '0');
    elsif rising_edge(clk) then q1 <= q1 + 1; end if;
  end process;
  p2: process (clk, rstn) begin
    if rstn = '0' then q2 <= x"A";
    elsif clk'event and clk = '1' then q2 <= q2 + 1; end if;
  end process;
  p3: process (clk, rst) begin
    if rst = '1' then q3 <= (others => '1');
    elsif falling_edge(clk) and en = '1' then q3 <= q3 - 1; end if;
  end process;
  p4: process (cv, rstn) begin
    if rstn = '0' then q4 <= (others => '0');
    elsif cv(1)'event and cv(1) = '1' then q4 <= q4 + 1; end if;
  end process;
  p5: process (cv, rstn) begin
    if '0' = rstn then q5 <= (others => '0');
    elsif cv(1) = '0' and cv(1)'event then q5 <= q5 + 1; end if;
  end process;
  process begin
    wait until rising_edge(clk) and en = '1'; cnt <= cnt + 1;
  end process;
  process (q1, q2, q3, q4, q5, cnt) begin
    report "q1=" & to_hstring(std_logic_vector(q1)) & " q2=" & to_hstring(std_logic_vector(q2)) & " q3=" & to_hstring(std_logic_vector(q3))
      & " q4=" & to_hstring(std_logic_vector(q4)) & " q5=" & to_hstring(std_logic_vector(q5)) & " cnt=" & integer'image(cnt);
  end process;
  postponed process begin
    wait for 7 ns; report "pp " & to_hstring(std_logic_vector(q1));
  end process;
end;
