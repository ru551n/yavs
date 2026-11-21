-- difftest: top=range_msg_enum std=2008 exit=1
-- A failed range check prints values and bounds in VHDL form: literals, units, the real in
-- full, the direction of the subtype (#1178).
entity range_msg_enum is end entity;
architecture a of range_msg_enum is
  type color is (red, green, blue, white);
  subtype rgb is color range red to blue;
begin
  process
    variable q : rgb := red;
    variable c : color := green;
  begin
    q := c;
    report "green ok";
    c := white;
    q := c;
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
