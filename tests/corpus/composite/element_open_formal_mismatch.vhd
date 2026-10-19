-- difftest: top=element_open_formal_mismatch std=2008 exit=1 ref=hand
-- ref=hand: the first reference simulator accepts the call; the others and the LRM
-- (5.3.2.2, 6.5.6.2) reject it, so .expected is the runtime error of the majority (#257).
-- An element of length 4 does not match the formal ow(open)(7 downto 0).
package element_open_formal_mismatch_pkg is
  type ow is array (natural range <>) of bit_vector;
end;

use work.element_open_formal_mismatch_pkg.all;

entity element_open_formal_mismatch is end;

architecture t of element_open_formal_mismatch is
  function show(x : ow(open)(7 downto 0)) return integer is
  begin
    return x'element'left;
  end;

  function pass(x : ow) return integer is
  begin
    return show(x);
  end;
begin
  process
    variable v : ow(0 to 1)(5 downto 2) := ("0000", "1111");
  begin
    report "before";
    report integer'image(pass(v));
    report "not reached";
    wait;
  end process;
end;
