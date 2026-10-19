-- (from the differential corpus)
-- ref=hand: an external name as an in-port actual is a signal name actual (LRM 8.7; #186,
-- user decision 2026-10-19): the formal starts at the actual's '0', not at its own default
-- '1', and follows it in the same delta, as two of the three reference simulators do. The
-- default reference simulator treats it as an expression actual, one delta later.
entity leaf is port (o : out bit); end;
architecture a of leaf is
  signal hidden : bit := '0';
begin
  hidden <= '1' after 5 ns;
  o <= hidden;
end;
entity mon is port (i : in bit := '1'); end;
architecture a of mon is
begin
  process (i) begin report "i=" & bit'image(i) & " @" & time'image(now); end process;
end;
entity external_port_init is end;
architecture a of external_port_init is
  signal x : bit;
begin
  l : entity work.leaf port map (o => x);
  m : entity work.mon port map (i => << signal .external_port_init.l.hidden : bit >>);
end;
