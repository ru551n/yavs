-- difftest: top=element_open_formal std=2008
-- A formal with open dimensions and a constrained element subtype, ow(open)(7 downto 0)
-- (#257, LRM 5.3.2.2): its 'element bounds are the formal's, whatever the actual's element
-- bounds, as long as the element lengths match; the dimensions come from the actual.
package element_open_formal_pkg is
  type ow is array (natural range <>) of bit_vector;
  type ow2 is array (natural range <>) of ow;
end;

use work.element_open_formal_pkg.all;

entity element_open_formal is end;

architecture t of element_open_formal is
  signal s : ow(3 to 4)(15 downto 8);
  signal u : ow(0 to 1)(0 to 7);
  signal w : ow2(1 to 2)(5 to 6)(11 downto 8);
  constant c : ow(0 to 2)(15 downto 8) := ("00000001", "11111111", "10101010");

  function show(x : ow(open)(7 downto 0)) return string is
  begin
    return integer'image(x'left) & " " & integer'image(x'length) & " "
      & integer'image(x'element'left) & " " & integer'image(x'element'right) & " "
      & integer'image(x'element'length) & " " & integer'image(x(x'left)'left) & " "
      & boolean'image(x(x'left)'ascending);
  end;

  function pass(x : ow) return string is
  begin
    return show(x);
  end;

  function show2(x : ow2(open)(open)(3 downto 0)) return string is
  begin
    return integer'image(x'left) & " " & integer'image(x'element'left) & " "
      & integer'image(x'element'element'left) & " " & integer'image(x'element'element'right);
  end;

  procedure psig(signal x : in ow(open)(7 downto 0)) is
  begin
    report "signal " & integer'image(x'left) & " " & integer'image(x'element'left) & " "
      & integer'image(x'element'right) & " " & integer'image(x'element'length);
  end;
begin
  process
    variable v : ow(1 to 2)(5 downto 2) := (others => "1111");
    variable n : natural := 0;
  begin
    psig(s);
    psig(u);
    report "variable " & show(ow'(("10101010", "01010101")));
    report "constant " & show(c);
    report "pass " & pass(c);
    report "deep " & show2(w);
    for i in c'element'range loop n := n + 1; end loop;
    report "count " & integer'image(n);
    wait;
  end process;
end;
