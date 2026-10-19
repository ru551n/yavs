-- difftest: top=generic_instance std=2019 ref=hand
-- ref=hand: the first reference simulator does not parse a protected type generic clause;
-- the expected output is the one reference that runs it, which matches the LRM.
-- 2019 generic protected types (5.6.2): `type t is new g generic map (...)`, two instances
-- with different actuals, generics visible in the body, a shared variable of an instance.
package gp is
  type box is protected
    generic (type t; init : t; constant depth : positive);
    procedure put(x : t);
    impure function get return t;
    impure function count return natural;
  end protected box;
end package;

package body gp is
  type box is protected body
    variable v : t := init;
    variable n : natural := 0;
    procedure put(x : t) is
    begin
      v := x;
      n := (n + 1) mod depth;
    end procedure;
    impure function get return t is
    begin
      return v;
    end function;
    impure function count return natural is
    begin
      return n;
    end function;
  end protected body box;
end package body;

use work.gp.all;

entity generic_instance is
end entity;

architecture a of generic_instance is
  type ibox is new work.gp.box generic map (t => integer, init => 7, depth => 3);
  type sbox is new box generic map (t => string(1 to 2), init => "ab", depth => 10);
  shared variable sh : ibox;
begin
  process
    variable i : ibox;
    variable s : sbox;
  begin
    assert i.get = 7 severity failure;
    i.put(5);
    i.put(6);
    i.put(9);
    assert i.get = 9 and i.count = 0 severity failure;
    report "i " & integer'image(i.get) & " " & integer'image(i.count);
    assert s.get = "ab" severity failure;
    s.put("xy");
    assert s.get = "xy" and s.count = 1 severity failure;
    report "s " & s.get & " " & integer'image(s.count);
    sh.put(42);
    assert sh.get = 42 and sh.count = 1 severity failure;
    report "generic_instance ok";
    wait;
  end process;
end architecture;
