-- difftest: top=range_msg_downto std=2008 exit=1
-- A failed range check prints values and bounds in VHDL form: literals, units, the real in
-- full, the direction of the subtype (#1178).
entity range_msg_downto is end entity;
architecture a of range_msg_downto is
  subtype dn is integer range 9 downto 0;
begin
  process
    variable d : dn := 3;
    variable i : integer := 12;
  begin
    d := 9;
    report "in range";
    d := i;
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
