-- difftest: top=guard_initial_value std=2008
-- The implicit GUARD signal starts as the value of the guard condition on the initial values of
-- the signals it reads (LRM 14.7.3.3, #464), also for a condition on a port and in a block
-- inside a block: a process that reads it at time 0 sees that value, and no event on GUARD
-- wakes a process sensitive to it.
entity guard_initial_value is end entity;

architecture test of guard_initial_value is
  signal v : natural := 5;
  signal on_s, off_s : boolean := false;
  signal hot : boolean := true;
begin
  on_s <= true;
  a : block (v < 10) is
  begin
    process (guard) begin
      report "a guard=" & boolean'image(guard);
    end process;
    inner : block (guard and hot) is
    begin
      process begin
        wait for 1 ns;
        report "inner guard=" & boolean'image(guard);
        wait;
      end process;
    end block;
  end block;
  b : block (v > 10) is
  begin
    process begin
      report "b guard=" & boolean'image(guard);
      wait;
    end process;
  end block;
  c : block (off_s = on_s) is
  begin
    process begin
      report "c guard=" & boolean'image(guard);
      wait;
    end process;
  end block;
end architecture;
