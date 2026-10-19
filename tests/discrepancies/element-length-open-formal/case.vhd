-- LRM 5.3.2.2, 6.5.6.2: a formal ow(open)(7 downto 0) has constrained elements, so the
-- elements of the actual must have length 8; here they have length 4.
package pkg is
  type ow is array (natural range <>) of bit_vector;
end package;

use work.pkg.all;

entity top is end entity;
architecture a of top is
  function show(x : ow(open)(7 downto 0)) return integer is
  begin
    return x'element'left;
  end function;

  function pass(x : ow) return integer is
  begin
    return show(x);
  end function;
begin
  process
    variable v : ow(0 to 1)(5 downto 2) := ("0000", "1111");
  begin
    report "RES element left " & integer'image(pass(v));
    wait;
  end process;
end architecture;
