-- difftest: top=subtype_attr_conversion std=2008
-- x'subtype as the type mark of a type conversion (9.3.6): a closely related array type, the
-- result takes the bounds of the object's subtype, as an argument and in an assignment.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;

entity subtype_attr_conversion is end entity;

architecture a of subtype_attr_conversion is
  signal s : unsigned(11 downto 4) := x"ab";
  function lo (arg : unsigned) return integer is
  begin
    return arg'right;
  end function;
begin
  process
    variable v : std_logic_vector(7 downto 0) := x"cd";
    variable u : unsigned(0 to 7);
    variable i : integer;
  begin
    u := u'subtype(v);
    assert u = x"cd" report "assignment" severity failure;
    i := lo(s'subtype(v));
    assert i = 4 report "bounds of the subtype" severity failure;
    assert s'subtype(v) = x"cd" report "comparison" severity failure;
    s <= s'subtype(not v);
    wait for 1 ns;
    assert s = x"32" report "signal" severity failure;
    report "ok";
    wait;
  end process;
end architecture;
