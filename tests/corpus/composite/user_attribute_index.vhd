-- difftest: top=user_attribute_index std=2008
-- A user-defined attribute of an array type, indexed and sliced: x'a(1), x'a(2 to 3). The
-- attribute value is a constant of the entity it decorates (LRM 7.2).
entity user_attribute_index is end entity;

architecture a of user_attribute_index is
  signal sg : bit;
  attribute tag : bit_vector;
  attribute tag of sg : signal is "1011";
  attribute names : string;
  attribute names of a : architecture is "wxyz";
begin
  process
    variable i : integer;
    attribute w : bit_vector;
    attribute w of i : variable is "0110";
  begin
    assert i'w(1) = '1' report "variable" severity failure;
    assert i'w(0) = '0' report "variable 0" severity failure;
    assert sg'tag(3) = '1' report "signal" severity failure;
    assert sg'tag(2 to 3) = "11" report "slice" severity failure;
    assert a'names(2) = 'x' report "architecture" severity failure;
    report "ok " & to_string(sg'tag) & " " & string'(a'names);
    wait;
  end process;
end architecture;
