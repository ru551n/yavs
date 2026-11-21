-- difftest: top=range_msg_index_enum std=2008 exit=1 ref=hand
-- ref=hand: the expected text is yavs's wording of the error, which names the array where the
-- references name the index type (`index 'z' outside of CHARACTER range 'a' to 'c'`); the
-- index shows as a literal in both (#1178).
entity range_msg_index_enum is end entity;
architecture a of range_msg_index_enum is
  type color is (red, green, blue, white);
  subtype rgb is color range red to blue;
  type tab is array (rgb) of integer;
  type ctab is array (character range 'a' to 'c') of integer;
begin
  process
    variable v : tab := (others => 0);
    variable cv : ctab := (others => 0);
    variable c : color := blue;
    variable ch : character := 'b';
  begin
    v(c) := 1;
    cv(ch) := 2;
    report "in range";
    ch := 'z';
    cv(ch) := 3;
    report "not reached" severity failure;
    wait;
  end process;
end architecture;
