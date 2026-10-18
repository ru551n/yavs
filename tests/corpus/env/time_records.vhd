-- difftest: top=time_records std=2019
-- std.env TIME_RECORD functions (LRM 2019 16.5) on fixed epoch values: gmtime, epoch,
-- to_string with fraction digits, the + and - operators. The current time is only checked
-- for plausibility.
use std.env.all;
entity time_records is end entity;

architecture test of time_records is
begin
  process
    variable t, u : time_record;
  begin
    t := gmtime(1000000000.25);
    report to_string(t) & " " & to_string(t, 3) & " " & dayofweek'image(t.weekday)
      & " day of year " & integer'image(t.dayofyear);
    assert t.month = 8 and t.day = 9 and t.microsecond = 250000 severity failure;
    report "epoch " & integer'image(integer(epoch(t)));
    u := t + 86400.0;
    report to_string(u) & ", difference " & integer'image(integer(u - t));
    u := 3600.0 + t;
    report to_string(u);
    u := t - 60.0;
    report to_string(u, 6);
    t := gmtime(0.0);
    report to_string(t) & " " & dayofweek'image(t.weekday);
    assert gmtime.year >= 2024 and localtime.year >= 2024 severity failure;
    assert epoch > 1.7e9 severity failure;
    wait;
  end process;
end architecture;
