-- difftest: top=copy_back_range_shared std=2008 exit=1 relaxed ref=hand
-- ref=hand: sim1 copies the value back without a check (discrepancy `copy-back-range`); sim2
-- agrees with this output. The actual is a shared variable that another process reads while the
-- procedure waits with 10 in its out formal: it still has its value from before the call, which
-- is checked after it (LRM 4.2.2.2, #1175).
entity copy_back_range_shared is end entity;
architecture a of copy_back_range_shared is
  shared variable sv : integer range 0 to 9 := 5;
  procedure set (variable o : out integer; v : integer) is
  begin
    o := v;
    wait for 2 ns;
  end procedure;
begin
  process
  begin
    set(sv, 10);
    report "not reached" severity failure;
    wait;
  end process;
  process
  begin
    wait for 1 ns;
    report "sv=" & integer'image(sv);
    wait;
  end process;
end architecture;
