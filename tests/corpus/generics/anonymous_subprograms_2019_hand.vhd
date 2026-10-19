-- difftest: top=anonymous_subprograms_2019_hand std=2019 ref=hand
-- ref=hand: no reference simulator binds anonymous parameter types (`x : type is ...`, LRM 2019
-- 6.5.2, 6.5.3) in formal subprograms, in subprograms with a generic list or in user operators;
-- the expected output is the LRM result.
package q is
  function pick generic (type t) parameter (x : type is private; y : t) return t;
  function scale generic (type t) parameter (a : type is range <>; b : a'subtype; c : t) return a'subtype;
  function "+" (a : type is private; b : a'subtype) return integer;
  function "-" (a : type is private) return integer;
end package;
package gp is
  generic (function code (v : type is private; w : v'subtype) return integer);
  function run return integer;
end package;
package body q is
  function pick generic (type t) parameter (x : type is private; y : t) return t is
  begin
    return y;
  end function;
  function scale generic (type t) parameter (a : type is range <>; b : a'subtype; c : t) return a'subtype is
  begin
    return a * b;
  end function;
  function "+" (a : type is private; b : a'subtype) return integer is
  begin
    return 42;
  end function;
  function "-" (a : type is private) return integer is
  begin
    return 99;
  end function;
end package body;
package body gp is
  function run return integer is
  begin
    return 5;
  end function;
end package body;

entity anonymous_subprograms_2019_hand is end;
architecture a of anonymous_subprograms_2019_hand is
  type rec is record x : integer; end record;
  type color is (red, green);
  function pi is new work.q.pick generic map (t => integer);
  function pc is new work.q.pick generic map (t => color);
  function sm is new work.q.scale generic map (t => boolean);
  function same (l, r : integer) return integer is begin return l * 10 + r; end function;
  function two (l, r : color) return integer is begin return color'pos(l) + color'pos(r); end function;
  package p1 is new work.gp generic map (code => same);
  package p2 is new work.gp generic map (code => two);
  use work.q."+";
  use work.q."-";
begin
  process
    variable r : rec;
  begin
    report "pick " & integer'image(pi(true, 7)) & " " & integer'image(pi(1.5, 8)) & " " & color'image(pc(red, green));
    report "scale " & integer'image(sm(3, 4, true));
    report "plus " & integer'image(r + r);
    report "plus " & integer'image(red + green) & " " & integer'image(color'pos(red) + 1);
    report "neg " & integer'image(-r) & " " & integer'image(-green);
    report "run " & integer'image(p1.run) & " " & integer'image(p2.run);
    wait;
  end process;
end architecture;
