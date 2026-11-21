-- difftest: top=range_msg_enum_downto std=2008 exit=1
-- A failed range check prints values and bounds in VHDL form: literals, units, the real in
-- full, the direction of the subtype (#1178).
entity range_msg_enum_downto is end entity;
architecture a of range_msg_enum_downto is
  type color is (red, green, blue, white);
  subtype dc is color range blue downto green;
begin
  process
    variable d : dc := green;
    variable c : color := white;
  begin
    d := blue;
    report "in range";
    d := c;
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
