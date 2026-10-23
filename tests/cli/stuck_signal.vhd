-- A long run with a stuck process, for the SIGUSR1 report.
entity stuck_signal is end entity;
architecture a of stuck_signal is
  signal clk, go : bit := '0';
begin
  clk <= not clk after 5 ns;
  p : process
  begin
    report "started";
    wait until go = '1';
    wait;
  end process;
end architecture;
