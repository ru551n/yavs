-- difftest: top=external_port_init std=2008 stop=20ns
-- An external name as an in-port actual is an expression actual (#186): the formal starts
-- at its own default ('1'), and the equivalent process `i <= actual` run at initialisation
-- gives it the actual's '0' in delta 1.
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
