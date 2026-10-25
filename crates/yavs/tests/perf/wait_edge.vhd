-- `wait until rising_edge(clk) and v = '1'` in a loop: the wait resumes its process only in a
-- cycle with the edge, not for the falling edge or for the change of `v` that follows each
-- rising edge. 300 clock cycles; a second process waits for the falling edge, so the two
-- edges of the wait change from wait to wait.
library ieee; use ieee.std_logic_1164.all;
entity wait_edge is end;
architecture a of wait_edge is
  signal clk : std_logic := '0';
  signal v : std_logic := '0';
  signal cnt : natural := 0;
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;
  process (clk)
  begin
    if rising_edge(clk) then
      cnt <= cnt + 1;
      v <= not v;
      if cnt = 300 then done <= true; end if;
    end if;
  end process;
  process
    variable hits, falls : natural := 0;
  begin
    for i in 1 to 100 loop
      wait until rising_edge(clk) and v = '1';
      hits := hits + 1;
      wait until falling_edge(clk) and v = '0';
      falls := falls + 1;
    end loop;
    report "hits=" & integer'image(hits) & " falls=" & integer'image(falls);
    wait;
  end process;
end;
