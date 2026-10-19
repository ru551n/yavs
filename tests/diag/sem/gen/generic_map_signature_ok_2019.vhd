-- yavs-test: std=2019
-- expect: none
-- LCS-2016-I03: a signature in the formal part of a generic map picks one of overloaded
-- formal subprograms; its type marks may name a generic type of the same list
package gp is
  generic (function f (x : integer) return integer; function f (x : real) return real);
end package;
entity sub is
  generic (function g (x : integer) return integer);
end entity;
architecture a of sub is
begin
end architecture;
entity e is end entity;
architecture a of e is
  function inc (x : integer) return integer is begin return x + 1; end function;
  function inc (x : real) return real is begin return x + 1.0; end function;
  package p is new work.gp
    generic map (f [real return real] => inc, f [integer return integer] => inc);
  function ap generic (type t; function f (x : t) return t) parameter (x : t) return t is
  begin
    return f(x);
  end function;
  function api is new ap generic map (t => integer, f [t return t] => inc);
begin
  u : entity work.sub generic map (g [integer return integer] => inc);
end architecture;
