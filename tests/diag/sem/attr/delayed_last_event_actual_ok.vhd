-- yavs-test: std=2008
-- expect: none
-- LRM 16.2.4
entity delayed_last_event_actual_ok is end entity;
architecture a of delayed_last_event_actual_ok is
  signal s : bit;
  procedure chk(v : time; e : time; msg : string) is
  begin
    assert v = e report msg severity note;
  end procedure;
begin
  process
    variable b : boolean;
  begin
    -- 's'delayed(t)' is a signal, so the signal attributes apply to it.
    chk(s'delayed(0 ns)'last_event, 10 ns, "delayed last_event");
    b := s'delayed(1 ns)'event or s'delayed'active or s'stable(1 ns)'quiet;
    wait;
  end process;
end architecture;
