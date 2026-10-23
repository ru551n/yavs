-- difftest: top=guard_same_cycle std=2008
-- GUARD is updated in the cycle where a signal of its condition has an event (LRM 11.2, 14.7.5):
-- a process that reads it sees the new value in the delta of the event, and a guard whose condition
-- reads another GUARD follows in the same cycle. Every change is reported with its delta.
entity guard_same_cycle is end;

architecture t of guard_same_cycle is
  signal v, w : natural := 0;
  signal s : bit := '0';
begin
  outer : block (v < 10) is
  begin
    inner : block (guard and w < 5) is
    begin
      process (w, guard) begin
        report "inner w=" & integer'image(w) & " guard=" & boolean'image(guard);
      end process;
    end block;
    process (guard) begin
      report "outer v=" & integer'image(v) & " guard=" & boolean'image(guard);
    end process;
    -- A guarded assignment, and a signal read in the cycle of the change.
    s <= guarded '1' when v = 3 else '0';
    process (s) begin
      report "s=" & bit'image(s);
    end process;
  end block;

  stim : process begin
    v <= 3;
    wait for 1 ns;
    w <= 7;
    wait for 1 ns;
    v <= 10;
    wait for 1 ns;
    w <= 1;
    v <= 2;
    wait for 1 ns;
    w <= 2;
    wait for 1 ns;
    wait;
  end process;
end architecture;
