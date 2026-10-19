-- difftest: top=aggregate_target_range_length std=2008 exit=1 ref=hand
-- A piece under a range choice of an aggregate target must have as many elements as its
-- choice (#244, LRM 10.6.2.1); with a piece whose length is known only at run time this is a
-- runtime error, raised before the piece is assigned. ref=hand: the reference simulators that
-- accept range choices in targets do not check this at run time (one checks it at analysis
-- when the lengths are static); the LRM requires the lengths to match (9.3.3.3).
entity aggregate_target_range_length is end;
architecture t of aggregate_target_range_length is
  type iv is array (natural range <>) of integer;
  procedure split(x : inout iv; y : inout iv; v : iv) is
  begin
    (5 downto 2 => x, 1 downto 0 => y) := v;
  end procedure;
begin
  process
    variable vd : iv(5 downto 0) := (50, 40, 30, 20, 10, 0);
    variable hi : iv(3 downto 0);
    variable lo : iv(1 downto 0);
    variable h3 : iv(2 downto 0);
    variable l3 : iv(2 downto 0);
  begin
    split(hi, lo, vd);
    report "hi(3)=" & integer'image(hi(3)) & " lo(0)=" & integer'image(lo(0));
    split(h3, l3, vd);
    report "not reached";
    wait;
  end process;
end;
