-- difftest: top=external_port_actual std=2008 stop=20ns ref=hand
-- ref=hand: an external name actual is a signal name actual (LRM 8.7; #186, user decision
-- 2026-10-19): the port follows `hidden` in the same delta, as two of the three reference
-- simulators do. The default reference simulator reports it one delta later.
entity leaf is port (o : out bit); end;
architecture a of leaf is
  signal hidden : bit := '0';
begin
  hidden <= '1' after 5 ns, '0' after 10 ns;
  o <= hidden;
end;
entity mon is port (i : in bit); end;
architecture a of mon is
begin
  process (i) begin report "i=" & bit'image(i) & " @" & time'image(now); end process;
end;
entity external_port_actual is end;
architecture a of external_port_actual is
  signal x : bit;
begin
  l : entity work.leaf port map (o => x);
  m : entity work.mon port map (i => << signal .external_port_actual.l.hidden : bit >>);
end;
