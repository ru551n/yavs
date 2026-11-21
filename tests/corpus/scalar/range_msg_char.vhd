-- difftest: top=range_msg_char std=2008 exit=1
-- A failed range check prints values and bounds in VHDL form: literals, units, the real in
-- full, the direction of the subtype (#1178).
entity range_msg_char is end entity;
architecture a of range_msg_char is
  type letters is (a, 'b', 'c', d);
  subtype mid is letters range 'b' to 'c';
begin
  process
    variable m : mid := 'b';
    variable l : letters := 'c';
  begin
    m := l;
    report "c ok";
    l := d;
    m := l;
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
