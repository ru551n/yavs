-- difftest: top=event_attrs std=2008
entity event_attrs is end;
architecture t of event_attrs is
  signal s : integer := 0;
  signal same : integer := 0;
begin
  process begin
    wait for 5 ns; s <= 1; same <= 0;
    wait for 3 ns; s <= 2;
    wait;
  end process;
  process (s, same) begin
    report "ev=" & boolean'image(s'event) & " act=" & boolean'image(s'active) & " same.ev=" & boolean'image(same'event)
      & " same.act=" & boolean'image(same'active) & " last_event=" & time'image(s'last_event)
      & " last_value=" & integer'image(s'last_value) & " @" & time'image(now);
  end process;
end;
