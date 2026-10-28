-- LRM 14.4.2.1: the bounds of subtype s are evaluated once, when its declaration is
-- elaborated; subtype s2 names s without a constraint of its own, so its elaboration does not
-- evaluate the bounds of s again: the impure function f runs once (#612).
package cnt is
  type counter is protected
    procedure inc;
    impure function get return natural;
  end protected;
end package;
package body cnt is
  type counter is protected body
    variable v : natural := 0;
    procedure inc is begin v := v + 1; end procedure;
    impure function get return natural is begin return v; end function;
  end protected body;
end package body;
use work.cnt.all;
package fpk is
  shared variable c : counter;
  impure function f (x : integer) return integer;
end package;
package body fpk is
  impure function f (x : integer) return integer is begin c.inc; return x - 1; end function;
end package body;
use work.fpk.all;
entity top is end entity;
architecture a of top is
  subtype s is bit_vector(f(4) downto 0);
  subtype s2 is s;
begin
  process
    variable v : s2;
  begin
    report "RES calls " & integer'image(c.get) & ", length " & integer'image(v'length);
    wait;
  end process;
end architecture;
