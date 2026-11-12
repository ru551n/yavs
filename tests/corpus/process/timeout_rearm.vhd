-- difftest: top=timeout_rearm std=2008 ignore=order
-- Timed waits that events end or wake without ending: `wait on s until c for T` measures T from
-- the execution of the wait statement, not from the last wake (LRM 10.2); a timeout and an event
-- in one cycle resume the process once, as a timeout (the condition is not asked); an event one
-- delta after the timeout comes too late; a zero timeout and a zero-delay event share the next
-- delta; a process that leaves such a wait for another timed wait with the same, an earlier or a
-- later deadline, or for an untimed wait on the same signal, is resumed by the new wait only;
-- `wait on s for T` starts T again at every execution; postponed processes and forced signals.
-- `ignore=order`: processes resumed by a timeout and by an event report in one cycle (30 and
-- 60 ns), whose order the LRM leaves open (yavs: `docs/decisions.md` "Process order within a cycle").
entity timeout_rearm is end;
architecture t of timeout_rearm is
  type counter_t is protected
    procedure inc;
  end protected;
  type counter_t is protected body
    variable n : natural := 0;
    procedure inc is begin n := n + 1; end;
  end protected body;
  shared variable pv : counter_t;
  signal w, x : integer := 0;
  signal s : integer := 0;
  signal u, v, z, f : integer := 0;
begin
  -- Events of s in delta 0 of 10, 20, 30, 40 and 60 ns.
  s <= 1 after 10 ns, 2 after 20 ns, 3 after 30 ns, 4 after 40 ns, 5 after 60 ns;

  -- Woken at 10 and 20 ns with the condition false; at 30 ns the event and the timeout fall in
  -- one cycle: resumed at 30 ns delta 0 by the timeout, not again one delta later.
  tie : process begin
    wait on s until s = 99 for 30 ns;
    report "tie: s=" & integer'image(s) & " event=" & boolean'image(s'event) & " at " & time'image(now);
    wait;
  end process;

  -- Four wakes with the condition false, then the timeout at 45 ns (not 45 ns after the last).
  late : process begin
    wait on s until s = 99 for 45 ns;
    report "late: s=" & integer'image(s) & " event=" & boolean'image(s'event) & " at " & time'image(now);
    wait;
  end process;

  -- u changes one delta after the 50 ns timeout: the timeout resumes first, and the next wait
  -- (deadline 60 ns) ends by its condition at 50 ns delta 1.
  ustim : process begin
    wait for 50 ns;
    u <= 1;
    wait for 180 ns;
    v <= 1;
    wait;
  end process;
  next_delta : process begin
    wait on u until u = 99 for 50 ns;
    report "next_delta: timeout u=" & integer'image(u) & " at " & time'image(now);
    wait on u until u = 1 for 10 ns;
    report "next_delta: cond u=" & integer'image(u) & " at " & time'image(now);
    wait;
  end process;

  -- Zero timeouts: the event of z and the timeout both in the next delta (one resumption),
  -- then a condition that holds in that delta, then a zero timeout with no event.
  zero : process begin
    wait for 5 ns;
    z <= 1;
    wait on z until z = 99 for 0 ns;
    report "zero: z=" & integer'image(z) & " event=" & boolean'image(z'event) & " at " & time'image(now);
    z <= 2;
    wait on z until z = 2 for 0 ns;
    report "zero: z=" & integer'image(z) & " event=" & boolean'image(z'event) & " at " & time'image(now);
    wait on z until z = 99 for 0 ns;
    report "zero: z=" & integer'image(z) & " event=" & boolean'image(z'event) & " at " & time'image(now);
    wait;
  end process;

  -- Leaves a timed wait by its condition at 20 ns (deadline 100 ns), then waits with the same
  -- deadline (100 ns), then with a later one (200 ns), then leaves a wait with a deadline of
  -- 400 ns at 230 ns for an earlier one (270 ns) and a later one (470 ns): no resumption at 400 ns.
  leave : process begin
    wait on s until s = 2 for 100 ns;
    report "leave: condition at " & time'image(now);
    wait on u until u = 99 for 80 ns;
    report "leave: same deadline at " & time'image(now);
    wait on s until s = 99 for 100 ns;
    report "leave: later deadline at " & time'image(now);
    wait on v until v = 1 for 200 ns;
    report "leave: condition at " & time'image(now);
    wait for 40 ns;
    report "leave: earlier deadline at " & time'image(now);
    wait on u for 200 ns;
    report "leave: past the left one at " & time'image(now);
    wait;
  end process;

  -- Leaves a timed wait by its condition at 20 ns (deadline 50 ns) for untimed waits on the same
  -- signal: resumed at 30, 40 and 60 ns, never at 50 ns.
  plain : process begin
    wait on s until s = 2 for 50 ns;
    report "plain: condition at " & time'image(now);
    for i in 1 to 3 loop
      wait on s;
      report "plain: s=" & integer'image(s) & " at " & time'image(now);
    end loop;
    wait;
  end process;

  -- The same, with nothing else in the activation that ends the timed wait (no report, no
  -- assignment), so the commit of the untimed wait could take a short way that knows nothing of
  -- timeouts: on `s` with many processes in the batch (`plain2`), alone on `w` (a batch of one,
  -- `lone`), two on `x` (a pooled batch with `YAVS_POOL_ALL`), one of them serial (`pair2` calls a
  -- protected method). Each waits again once with its condition false (resident from then on),
  -- leaves at the next event and reports at the fourth after (60, 61, 62 ns), never at 50 ns.
  w <= 9 after 11 ns, 1 after 21 ns, 2 after 31 ns, 3 after 41 ns, 4 after 61 ns;
  x <= 9 after 12 ns, 1 after 22 ns, 2 after 32 ns, 3 after 42 ns, 4 after 62 ns;
  plain2 : process begin
    wait on s until s = 2 for 50 ns;
    wait on s;
    wait on s;
    wait on s;
    report "plain2: s=" & integer'image(s) & " at " & time'image(now);
    wait;
  end process;
  lone : process begin
    wait on w until w = 1 for 50 ns;
    wait on w;
    wait on w;
    wait on w;
    report "lone: w=" & integer'image(w) & " at " & time'image(now);
    wait;
  end process;
  pair1 : process begin
    wait on x until x = 1 for 50 ns;
    wait on x;
    wait on x;
    wait on x;
    report "pair1: x=" & integer'image(x) & " at " & time'image(now);
    wait;
  end process;
  pair2 : process begin
    pv.inc;
    wait on x until x = 1 for 50 ns;
    wait on x;
    wait on x;
    wait on x;
    report "pair2: x=" & integer'image(x) & " at " & time'image(now);
    wait;
  end process;

  -- Without a condition the timeout restarts at every execution: 10, 20, 30, 40, 55, 60 ns.
  wfor : process
    variable n : natural := 0;
  begin
    while n < 6 loop
      wait on s for 15 ns;
      n := n + 1;
      report "wfor: " & integer'image(n) & " s=" & integer'image(s) & " event=" & boolean'image(s'event) & " at " & time'image(now);
    end loop;
    wait;
  end process;

  -- Postponed: wakes with the condition false, the timeout at 35 ns, a condition at 40 ns, and a
  -- timeout due with the event of s at 60 ns.
  post : postponed process begin
    wait on s until s = 99 for 35 ns;
    report "post: s=" & integer'image(s) & " at " & time'image(now);
    wait on s until s = 4 for 100 ns;
    report "post: s=" & integer'image(s) & " at " & time'image(now);
    wait on s until s = 99 for 20 ns;
    report "post: s=" & integer'image(s) & " at " & time'image(now);
    wait;
  end process;

  -- A force and a release wake the wait with its condition false; the timeout ends it at 30 ns.
  forcer : process begin
    wait for 12 ns;
    f <= force 3;
    wait for 10 ns;
    f <= release;
    wait;
  end process;
  fwait : process begin
    wait on f until f = 99 for 30 ns;
    report "fwait: f=" & integer'image(f) & " at " & time'image(now);
    wait;
  end process;
end;
