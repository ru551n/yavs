-- difftest: top=timeout_resident std=2008
-- Many processes in one `wait on x until c for T` loop on a clock and on a message signal: they
-- wait again after every event that leaves the condition false, time out while the clock stops
-- and again while it runs, and must not see a timeout at the next event after one. A wait that
-- completes with the condition false must be a timeout, exactly T after the wait began.
entity timeout_resident is end;
architecture t of timeout_resident is
  constant K : natural := 20;
  constant M : natural := 4;
  signal clk, en : bit := '0';
  signal net : integer := -1;
  type nv is array (natural range <>) of natural;
  signal outs : nv(0 to K - 1) := (others => 0);
  signal msgs : nv(0 to M - 1) := (others => 0);
begin
  clock : process begin
    for i in 1 to 40 loop
      wait for 5 ns;
      clk <= not clk;
    end loop;
    wait for 200 ns;
    for i in 1 to 40 loop
      wait for 5 ns;
      clk <= not clk;
    end loop;
    wait;
  end process;

  enable : process begin
    wait for 452 ns;
    en <= '1';
    wait for 6 ns;
    en <= '0';
    wait;
  end process;

  g : for i in 0 to K - 1 generate
    process
      variable t0 : time;
      variable n, m : natural := 0;
    begin
      loop
        t0 := now;
        wait on clk until en = '1' for 95 ns;
        if en = '1' then
          n := n + 1;
        else
          assert now = t0 + 95 ns
            report "clock waiter " & integer'image(i) & " resumed early at " & time'image(now)
            severity failure;
          m := m + 1;
        end if;
        exit when now >= 700 ns;
      end loop;
      outs(i) <= 100 * n + m;
      wait;
    end process;
  end generate;

  -- A message to one receiver at a time; the others wake with the condition false.
  sender : process begin
    for k in 1 to 24 loop
      wait for 7 ns;
      net <= k mod M;
      wait for 7 ns;
      net <= -1;
    end loop;
    wait;
  end process;

  r : for i in 0 to M - 1 generate
    process
      variable t0 : time;
      variable n, m : natural := 0;
    begin
      loop
        t0 := now;
        wait on net until net = i for 40 ns;
        if net = i then
          n := n + 1;
        else
          assert now = t0 + 40 ns
            report "receiver " & integer'image(i) & " resumed early at " & time'image(now)
            severity failure;
          m := m + 1;
        end if;
        exit when now >= 500 ns;
      end loop;
      msgs(i) <= 100 * n + m;
      wait;
    end process;
  end generate;

  check : process begin
    wait for 800 ns;
    for i in 0 to K - 1 loop
      assert outs(i) = outs(0) report "clock waiter " & integer'image(i) & " differs" severity failure;
    end loop;
    report "clock waiters: " & integer'image(outs(0));
    for i in 0 to M - 1 loop
      report "receiver " & integer'image(i) & ": " & integer'image(msgs(i));
    end loop;
    wait;
  end process;
end;
