-- A process that waits for a signal nobody drives, next to a running clock.
entity stuck_forever is end entity;
architecture a of stuck_forever is
  signal clk, go, done : bit := '0';
begin
  clk <= not clk after 5 ns;
  p : process
  begin
    wait until go = '1';
    done <= '1';
    wait;
  end process;
  g : process
  begin
    wait for 100 us;
    wait;
  end process;
end architecture;
