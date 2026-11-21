-- difftest: top=range_msg_phys_time std=2008 exit=1
-- A failed range check prints values and bounds in VHDL form: literals, units, the real in
-- full, the direction of the subtype (#1178).
entity range_msg_phys_time is end entity;
architecture a of range_msg_phys_time is
  subtype short is time range 1 ns to 10 ns;
begin
  process
    variable ts : short := 5 ns;
    variable t : time := 20 us;
  begin
    ts := 10 ns;
    report "in range";
    ts := t;
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
