-- Waiters with a long timeout that a signal ends first (`wait on net until net = i for 1 hr`, the
-- shape of a message-passing wait): each wake leaves the timeout of the wait it ended in the time
-- heap. The stale ones are dropped as the heap fills, so its size follows the live timeouts, not
-- events times waiters (`heap_peak`).
entity timeout_stale is end;
architecture a of timeout_stale is
  constant K : natural := 16;
  constant E : natural := 200;
  signal net : integer := -1;
  type iv is array (0 to K - 1) of natural;
  signal hits : iv := (others => 0);
  signal fin : boolean := false;
begin
  gw : for i in 0 to K - 1 generate
    process
      variable h : natural := 0;
    begin
      loop
        wait on net until net = i or fin for 1 hr;
        exit when fin;
        h := h + 1;
        hits(i) <= h;
      end loop;
      wait;
    end process;
  end generate;
  process
    variable s : natural := 0;
  begin
    for e in 1 to E loop
      net <= (e * 7) mod K;
      wait for 10 ns;
      net <= -1;
      wait for 10 ns;
    end loop;
    fin <= true;
    wait for 10 ns;
    for i in 0 to K - 1 loop
      s := s + hits(i) * (i + 1);
    end loop;
    report "s=" & integer'image(s) & " t=" & time'image(now);
    wait;
  end process;
end;
