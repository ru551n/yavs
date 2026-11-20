-- difftest: top=copy_back_range std=2008 exit=1 ref=hand
-- ref=hand: sim1 copies the value back without a check (discrepancy `copy-back-range`); sim2
-- agrees with this output. The value of an out or inout formal copied back belongs to the
-- subtype of the actual (LRM 4.2.2.2, #1175): copies in range go back, 10 does not fit
-- `integer range 0 to 9`.
entity copy_back_range is end entity;
architecture a of copy_back_range is
  procedure set (variable o : out integer; v : integer) is begin o := v; end procedure;
  procedure inc (variable o : inout integer) is begin o := o + 1; end procedure;
begin
  process
    variable q : integer range 0 to 9 := 0;
  begin
    set(q, 8);
    inc(q);
    report "q=" & integer'image(q);
    set(q, 10);
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
