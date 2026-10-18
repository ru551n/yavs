-- difftest: top=waveforms std=2008
entity waveforms is end;
architecture t of waveforms is
  signal clk : bit := '0';
  signal w : integer := 0;
begin
  clk <= '1' after 5 ns, '0' after 10 ns, '1' after 15 ns, '0' after 20 ns;
  w <= 1, 2 after 3 ns, 3 after 6 ns;
  process (clk, w) begin report "clk=" & bit'image(clk) & " w=" & integer'image(w) & " @" & time'image(now); end process;
end;
