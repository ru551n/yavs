-- Clocked processes whose inputs rarely change, one of each spelling of the clock edge: the kernel
-- leaves them suspended at an edge no input had an event for (idle skip) and the result is
-- the same. 600 cycles; `d` changes every 40 edges, `en` is high from edge 100 to 400.
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity idle_regs is end;
architecture a of idle_regs is
  signal clk : std_logic := '0';
  signal d : unsigned(7 downto 0) := (others => '0');
  signal en, rst : std_logic := '0';
  signal q1, q2, q3, q4, q5, q6 : unsigned(7 downto 0) := (others => '0');
  signal b, bq, ev : bit := '0';
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;
  p1 : process (clk) begin
    if rising_edge(clk) then q1 <= d; end if;
  end process;
  p2 : process (clk) begin
    if clk'event and clk = '1' then
      if en = '1' then q2 <= d + 1; end if;
    end if;
  end process;
  p3 : process begin
    wait until rising_edge(clk);
    q3 <= q1 + q2;
  end process;
  p4 : process (clk) begin
    if rising_edge(clk) and en = '1' then q4 <= d xor q1; end if;
  end process;
  p5 : process (clk, rst) begin
    if rst = '1' then q5 <= (others => '0');
    elsif rising_edge(clk) then q5 <= q3; end if;
  end process;
  p6 : process (clk) begin
    if rising_edge(clk) then bq <= b; end if;
  end process;
  p7 : process begin
    wait until clk = '1';
    ev <= bq;
  end process;
  p8 : process (clk) begin
    if rising_edge(clk) then
      if en = '1' then q6 <= q6 + 1; end if;
    end if;
  end process;
  stim : process begin
    for i in 1 to 600 loop
      wait until rising_edge(clk);
      if i mod 40 = 0 then d <= d + 3; end if;
      if i mod 150 = 0 then b <= not b; end if;
      if i = 100 then en <= '1'; end if;
      if i = 400 then en <= '0'; end if;
      if i = 500 then rst <= '1'; end if;
      if i = 520 then rst <= '0'; end if;
    end loop;
    report "q=" & integer'image(to_integer(q1)) & "," & integer'image(to_integer(q2)) & ","
      & integer'image(to_integer(q3)) & "," & integer'image(to_integer(q4)) & ","
      & integer'image(to_integer(q5)) & "," & integer'image(to_integer(q6)) & " b="
      & bit'image(bq) & bit'image(ev);
    done <= true; wait;
  end process;
end;
