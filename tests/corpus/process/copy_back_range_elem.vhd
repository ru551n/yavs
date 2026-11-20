-- difftest: top=copy_back_range_elem std=2008 exit=1 ref=hand
-- ref=hand: sim1 copies the value back without a check (discrepancy `copy-back-range`); sim2
-- agrees with this output. An element of an array of a real subtype named with a variable
-- index is checked against the element subtype when copied back (LRM 4.2.2.2, #1175).
entity copy_back_range_elem is end entity;
architecture a of copy_back_range_elem is
  subtype unit is real range 0.0 to 1.0;
  type rvec is array (0 to 3) of unit;
  procedure set_r (variable o : out real; v : real) is begin o := v; end procedure;
begin
  process
    variable u : rvec := (others => 0.0);
    variable i : integer := 2;
  begin
    set_r(u(i), 0.5);
    assert u(2) = 0.5 severity failure;
    report "u(2) set";
    i := 3;
    set_r(u(i), 2.0);
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
