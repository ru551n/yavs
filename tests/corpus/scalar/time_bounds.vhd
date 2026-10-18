-- difftest: top=time_bounds std=2008
-- TIME is a 64-bit physical type: its range is the full i64 range in femtoseconds.
entity time_bounds is end;
architecture t of time_bounds is
begin
  process
    variable lo : time := time'low;
    variable hi : time := time'high;
  begin
    report "low=" & time'image(lo);
    report "high=" & time'image(hi);
    assert lo < 0 fs and hi > 0 fs severity failure;
    assert lo + hi = -1 fs severity failure;
    assert lo + 1 fs > time'low severity failure;
    wait;
  end process;
end;
