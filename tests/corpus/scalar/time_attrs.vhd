-- difftest: top=time_attrs std=2008
entity time_attrs is end;
architecture t of time_attrs is
begin
  process
    variable t : time := 1500 ps;
  begin
    assert time'low = -9223372036854775807 fs - 1 fs severity failure;
    assert 1 ns = 1000 ps and 1 us = 1000 ns and 1 ms = 1000 us and 1 sec = 1000 ms severity failure;
    assert t = 1.5 ns and t / 3 = 500 ps and t * 2 = 3 ns and t / 500 ps = 1 + 2 severity failure;
    assert abs(-t) = t and t mod 1 ns = 500 ps and -t < 0 fs severity failure;
    report "t=" & time'image(t) & " " & time'image(2 us) & " " & time'image(-7 ns);
    report "pos=" & integer'image(time'pos(5 ps)) & " val=" & time'image(time'val(77));
    report "value=" & time'image(time'value("3 ns"));
    wait for t;
    report "now=" & time'image(now);
    assert now = 1500 ps severity failure;
    assert to_string(10 ns) = time'image(10 ns) severity failure;
    wait;
  end process;
end;
