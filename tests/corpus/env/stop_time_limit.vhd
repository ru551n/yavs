-- difftest: top=stop_time_limit std=2008 stop=55ns
entity stop_time_limit is end;
architecture t of stop_time_limit is
  signal clk : bit := '0';
begin
  clk <= not clk after 10 ns;
  process (clk) begin
    report "clk=" & bit'image(clk) & " @" & time'image(now);
  end process;
end;
