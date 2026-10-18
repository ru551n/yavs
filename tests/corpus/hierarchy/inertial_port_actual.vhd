-- difftest: top=inertial_port_actual std=2008 stop=20ns
entity mon is generic (tag : string); port (i : in bit); end;
architecture a of mon is
begin
  process (i) begin report tag & " i=" & bit'image(i) & " @" & time'image(now); end process;
end;
entity inertial_port_actual is end;
architecture a of inertial_port_actual is
  signal s : bit := '0';
begin
  s <= '1' after 5 ns, '0' after 10 ns;
  plain : entity work.mon generic map ("plain") port map (i => s);
  inert : entity work.mon generic map ("inertial") port map (i => inertial s);
end;
