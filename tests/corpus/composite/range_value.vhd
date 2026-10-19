-- difftest: top=range_value std=2019 ref=hand
-- ref=hand: the first reference simulator returns the unreversed range for
-- R'reverse_range'value (16.2.6: left and right swap, the direction flips); the others
-- do not support range values. Expected output written to the LRM.
-- 2019 R'value (16.2.6): the range record of a range attribute, also reversed, per
-- dimension, of a type mark, and through a generic array type.
entity range_value is
end entity;

architecture a of range_value is
  type mat_t is array (natural range <>, integer range <>) of bit;
  subtype word_t is integer range 7 downto 0;

  function span_of
    generic (type array_t is array (natural range <>) of type is private)
    parameter (x : array_t) return natural'range'record is
  begin
    return x'range'value when x'ascending else x'reverse_range'value;
  end function;

  function span is new span_of generic map (array_t => bit_vector);

  signal d : bit_vector(7 downto 2);
  signal u : bit_vector(3 to 9);
  signal m : mat_t(1 to 2, 5 downto 3);
begin
  process
    constant r1 : natural'range'record := d'range'value;
    constant r2 : natural'range'record := d'reverse_range'value;
    constant r3 : integer'range'record := m'range(2)'value;
    constant r4 : integer'range'record := m'reverse_range(2)'value;
    constant r5 : integer'range'record := word_t'range'value;
    constant r6 : natural'range'record := span(d);
    constant r7 : natural'range'record := span(u);
  begin
    assert r1.left = 7 and r1.right = 2 and r1.direction = descending severity failure;
    assert r2.left = 2 and r2.right = 7 and r2.direction = ascending severity failure;
    assert r3.left = 5 and r3.right = 3 and r3.direction = descending severity failure;
    assert r4.left = 3 and r4.right = 5 and r4.direction = ascending severity failure;
    assert r5.left = 7 and r5.right = 0 and r5.direction = descending severity failure;
    assert r6.left = 2 and r6.right = 7 and r6.direction = ascending severity failure;
    assert r7.left = 3 and r7.right = 9 and r7.direction = ascending severity failure;
    report "range_value ok";
    wait;
  end process;
end architecture;
