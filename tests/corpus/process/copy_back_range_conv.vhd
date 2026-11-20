-- difftest: top=copy_back_range_conv std=2008 exit=1 ref=hand
-- ref=hand: sim1 does not support a conversion in the formal part of a variable parameter;
-- sim2 agrees with this output. The value of the formal after the conversion of the formal part
-- belongs to the subtype of the actual (LRM 4.2.2.2, 6.5.7.1, #1175).
entity copy_back_range_conv is end entity;
architecture a of copy_back_range_conv is
  type small is range 0 to 100;
  procedure set (variable o : out small; v : small) is begin o := v; end procedure;
begin
  process
    variable q : integer range 0 to 9 := 0;
  begin
    set(integer(o) => q, v => 9);
    report "q=" & integer'image(q);
    set(integer(o) => q, v => 10);
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
