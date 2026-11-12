-- difftest: top=timeout_cancel std=2008
-- Timeouts that a signal event beats, so the timeout of the ended wait is stale: it must never
-- resume its process later, a timeout due with an event in the same cycle resumes once, a later
-- wait with an earlier or later deadline gets its own, a postponed process times out, and live
-- timeouts, pending `after` transactions and 'delayed keep working while churning waiters leave
-- several thousand stale timeouts behind.
entity timeout_cancel is end;
architecture t of timeout_cancel is
  signal s, x : integer := 0;
  signal tick : integer := 0;
  signal z : integer := 0;
  signal d : integer;
  signal u : integer := 0;
  signal stop_churn : boolean := false;
begin
  d <= x'delayed(700 ns);

  stim : process begin
    wait for 10 ns; s <= 1;
    wait for 10 ns; s <= 2;           -- 20 ns: same cycle as the 20 ns timeout of `same`
    wait for 30 ns; s <= 3;           -- 50 ns
    wait for 30 ns; s <= 4;           -- 80 ns
    wait;
  end process;

  -- A 1 hr timeout beaten at 10 ns, then an earlier deadline, then a later one: the stale 1 hr
  -- timeout must not resume it at 1 hr.
  rearm : process begin
    wait on s for 1 hr;
    assert now = 10 ns and s'event severity failure;
    report "rearm: s=" & integer'image(s) & " at " & time'image(now);
    wait for 5 ns;
    report "rearm: short timeout at " & time'image(now);
    wait until s = 99 for 2 hr;
    report "rearm: long timeout at " & time'image(now) & " s=" & integer'image(s);
    wait;
  end process;

  -- The timeout and the event of `u` (assigned 20 ns ahead) fall in one cycle: one resumption.
  same : process
    variable n : natural := 0;
  begin
    u <= 1 after 20 ns;
    loop
      wait on u until u = 1 for 20 ns;
      n := n + 1;
      report "same: wake " & integer'image(n) & " u=" & integer'image(u) & " event=" & boolean'image(u'event) & " at " & time'image(now);
      exit when n = 2;
    end loop;
    wait;
  end process;

  -- A transaction 2500 ns ahead and a timeout of 3 us live across the churn.
  live : process begin
    z <= 7 after 2500 ns;
    x <= 5 after 100 ns, 6 after 900 ns;
    wait for 3 us;
    report "live: timeout at " & time'image(now) & " z=" & integer'image(z) & " d=" & integer'image(d);
    stop_churn <= true;
    wait;
  end process;

  watch_z : process begin
    wait on z;
    report "z=" & integer'image(z) & " at " & time'image(now);
    wait;
  end process;

  -- From 750 ns: the initial value of 'delayed is a known discrepancy (`delayed-initial-value`).
  watch_d : process begin
    wait for 750 ns;
    wait on d;
    report "d=" & integer'image(d) & " at " & time'image(now);
    wait on d;
    report "d=" & integer'image(d) & " at " & time'image(now);
    wait;
  end process;

  ticker : process begin
    for i in 1 to 200 loop
      wait for 7 ns;
      tick <= i;
    end loop;
    wait;
  end process;

  -- Waiters whose 1 hr timeouts every tick beats: 16 x 200 stale timeouts.
  churn : for k in 0 to 15 generate
    process
      variable n : natural := 0;
    begin
      loop
        wait on tick, stop_churn until tick mod 16 /= k or stop_churn for 1 hr;
        exit when stop_churn;
        n := n + 1;
      end loop;
      if k = 3 then
        report "churn: n=" & integer'image(n) & " at " & time'image(now);
      end if;
      wait;
    end process;
  end generate;

  post : postponed process
    variable n : natural := 0;
  begin
    wait on s for 15 ns;
    n := n + 1;
    report "post: wake " & integer'image(n) & " s=" & integer'image(s) & " at " & time'image(now);
    if n = 6 then wait; end if;
  end process;
end;
