-- difftest: top=copy_back_range_captured std=2008 exit=1 ref=hand
-- ref=hand: sim1 copies the value back without a check and reads the bound again
-- (discrepancies `copy-back-range`, `object-range-bound-variable`); sim2 agrees with this
-- output. The subtype of `q` is `0 to 9`, fixed at its declaration (LRM 14.4.2.1, #1174):
-- after `k := 20` a value of 10 copied back from a procedure that waits does not belong to it
-- (LRM 4.2.2.2, #1175).
entity copy_back_range_captured is end entity;
architecture a of copy_back_range_captured is
  procedure set_later (variable o : out integer; v : integer) is
  begin
    wait for 1 ns;
    o := v;
  end procedure;
begin
  process
    variable k : integer := 9;
    variable q : integer range 0 to k := 0;
  begin
    k := 20;
    set_later(q, 9);
    report "q=" & integer'image(q);
    set_later(q, 10);
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
