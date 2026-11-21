-- difftest: top=range_msg_phys std=2008 exit=1
-- A failed range check prints values and bounds in VHDL form: literals, units, the real in
-- full, the direction of the subtype (#1178).
entity range_msg_phys is end entity;
architecture a of range_msg_phys is
  subtype short is time range 1 ns to 10 ns;
  type len is range 0 to 1000000000 units mm; cm = 10 mm; m = 1000 mm; end units;
  subtype slen is len range 5 mm to 50 cm;
begin
  process
    variable ts : short := 5 ns;
    variable t : time := 7 ns;
    variable sl : slen := 5 mm;
    variable l : len := 20 cm;
  begin
    ts := t;
    sl := l;
    report "in range";
    l := 6 m;
    sl := l;
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
