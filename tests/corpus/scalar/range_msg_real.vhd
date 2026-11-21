-- difftest: top=range_msg_real std=2008 exit=1
-- A failed range check prints values and bounds in VHDL form: literals, units, the real in
-- full, the direction of the subtype (#1178).
entity range_msg_real is end entity;
architecture a of range_msg_real is
  subtype unit is real range 0.0 to 0.5;
begin
  process
    variable u : unit := 0.25;
    variable r : real := 0.5;
  begin
    u := r;
    report "in range";
    r := 2.5;
    u := r;
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
