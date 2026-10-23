-- difftest: top=attribute_value_subtype std=2008
-- The value of a user attribute takes the attribute's subtype (LRM 7.2): a string literal
-- given to `string (3 downto 1)` has those bounds, so index 1 is the last character.
entity attribute_value_subtype is
  subtype s3 is string (3 downto 1);
  attribute a : s3;
  subtype b4 is bit_vector(3 downto 0);
  attribute v : b4;
end entity;

architecture a of attribute_value_subtype is
  constant c : integer := 0;
  attribute a of c : constant is "ABC";
  signal sg : bit;
  attribute v of sg : signal is "1011";
begin
  process
    variable n : integer;
  begin
    assert c'a(1) = 'C' report "a(1)" severity failure;
    assert c'a(3) = 'A' report "a(3)" severity failure;
    assert c'a'left = 3 and c'a'right = 1 and c'a'length = 3 report "bounds" severity failure;
    assert sg'v(3) = '1' and sg'v(0) = '1' and sg'v(2) = '0' report "vector" severity failure;
    report "ok " & c'a & " " & integer'image(c'a'left);
    wait;
  end process;
end architecture;
