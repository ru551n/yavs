-- difftest: top=call_generic_map_2019 std=2019
-- 2019 generic maps in calls of uninstantiated subprograms (LCS-2016-049, #362): functions
-- and procedures, positional and named maps, defaults (`open`),
-- a deferred constant in a package body and a call inside a generic body (the concurrent and
-- expanded-name forms are in call_generic_map_2019_hand).
package call_generic_map_2019_pkg is
  function ident generic (type t) parameter (x : t) return t;
  constant c : integer;
end package;
package body call_generic_map_2019_pkg is
  function ident generic (type t) parameter (x : t) return t is begin return x; end function;
  constant c : integer := ident generic map (integer) (7);
end package body;

use work.call_generic_map_2019_pkg.all;
entity call_generic_map_2019 is end entity;
architecture t of call_generic_map_2019 is
  procedure hello generic (n : integer := 3) is
  begin
    report "hello " & integer'image(n);
  end procedure;
  procedure tick generic (type t; function im (x : t) return string) parameter (x : t) is
  begin
    report "tick " & im(x);
  end procedure;
  function twice generic (type t; function "+" (a, b : t) return t is <>)
    parameter (x : t) return t is
  begin
    return x + x;
  end function;
  function img (x : integer) return string is begin return integer'image(x); end function;
  function outer generic (type u) parameter (x : u) return u is
  begin
    return ident generic map (t => u) (x);
  end function;
  function oi is new outer generic map (u => integer);
  function ob is new outer generic map (u => boolean);
begin
  process
  begin
    wait for 1 ns;
    report "c " & integer'image(c);
    assert c = 7 severity failure;
    hello generic map (n => 5);
    hello generic map (open);
    report "i " & integer'image(twice generic map (integer) (21));
    assert twice generic map (t => real) (1.5) = 3.0 severity failure;
    tick generic map (t => integer, im => img) (9);
    report "o " & integer'image(oi(3)) & " " & boolean'image(ob(true));
    wait;
  end process;
end architecture;
