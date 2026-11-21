-- difftest: top=copy_back_range_wait std=2008 exit=1 ref=hand
-- ref=hand: sim1 copies the value back without a check (discrepancy `copy-back-range`); sim2
-- agrees with this output. A procedure that waits with 10 in its out formal: the actual is
-- checked when the procedure returns, at 2 ns (LRM 4.2.2.2, #1175).
entity copy_back_range_wait is end entity;
architecture a of copy_back_range_wait is
  procedure set (variable o : out integer; v : integer) is
  begin
    o := v;
    wait for 2 ns;
  end procedure;
begin
  process
    variable q : integer range 0 to 9 := 5;
  begin
    set(q, 7);
    report "q=" & integer'image(q);
    set(q, 10);
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
