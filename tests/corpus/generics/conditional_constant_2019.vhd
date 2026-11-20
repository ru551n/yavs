-- difftest: top=conditional_constant_2019 std=2019
-- 2019 conditional expressions in constants that size signals, so elaboration evaluates them:
-- two alternatives and three alternatives (#445 found them unevaluated at elaboration).
entity conditional_constant_2019 is
  generic (wide : boolean := true; mode : integer := 2);
end entity;
architecture t of conditional_constant_2019 is
  constant width : integer := 4 when wide else 2;
  constant depth : integer := 1 when mode = 0 else 3 when mode = 1 else 5;
  signal a : bit_vector(width - 1 downto 0);
  signal b : bit_vector(0 to depth - 1);
  signal c : bit_vector(a'range);
  constant up : boolean := c'ascending;
begin
  process
  begin
    report "a " & integer'image(a'length) & " b " & integer'image(b'length) & " c " & integer'image(c'left) & " " & boolean'image(up);
    wait;
  end process;
end architecture;
