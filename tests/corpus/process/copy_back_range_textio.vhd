-- difftest: top=copy_back_range_textio std=2008 exit=1 ref=hand
-- ref=hand: sim1 copies the value back without a check (discrepancy `copy-back-range`); sim2
-- agrees with this output. READ of a line into a variable of `integer range 0 to 9`: the
-- value read is copied back through the out formal VALUE (LRM 4.2.2.2, 16.4, #1175).
use std.textio.all;
entity copy_back_range_textio is end entity;
architecture a of copy_back_range_textio is
begin
  process
    variable l : line;
    variable q : integer range 0 to 9 := 0;
  begin
    l := new string'("7");
    read(l, q);
    report "q=" & integer'image(q);
    l := new string'("10");
    read(l, q);
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
