-- difftest: top=alias_constant
-- An alias of a locally static constant in a subprogram (LRM 6.6.2): scalar, array, slice.
package alias_constant_pkg is
  constant C : natural := 8;
  constant V : bit_vector(1 to 3) := "101";
end package;

use work.alias_constant_pkg.all;

entity alias_constant is
end entity;

architecture a of alias_constant is
  function g return natural is
    alias m is C;
    alias w is V;
    alias s is V(2 to 3);
  begin
    assert w(2) = '0' severity failure;
    assert s'length = 2 and s(3) = '1' severity failure;
    return m + w'length;
  end function;
begin
  process
  begin
    assert g = 11 severity failure;
    report "g = " & integer'image(g);
    wait;
  end process;
end architecture;
