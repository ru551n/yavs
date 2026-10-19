-- difftest: top=generic_map_signature_2019 std=2019
-- 2019 signatures in the formal part of a generic map (LCS-2016-I03, #363): overloaded formal
-- subprograms of a package told apart, a formal of a subprogram instance with a signature
-- naming the list's own generic type, and an entity instance.
package generic_map_signature_2019_gp is
  generic (function f (x : integer) return integer; function f (x : real) return real);
  function both (x : integer) return integer;
end package;
package body generic_map_signature_2019_gp is
  function both (x : integer) return integer is
  begin
    return f(x) + integer(f(real(x)));
  end function;
end package body;

entity generic_map_signature_2019_sub is
  generic (function g (x : integer) return integer);
end entity;
architecture a of generic_map_signature_2019_sub is
begin
  process
  begin
    report "sub " & integer'image(g(10));
    wait;
  end process;
end architecture;

entity generic_map_signature_2019 is end entity;
architecture t of generic_map_signature_2019 is
  function inc (x : integer) return integer is begin return x + 1; end function;
  function inc (x : real) return real is begin return x + 1.0; end function;
  function dbl (x : integer) return integer is begin return x * 2; end function;
  function dbl (x : real) return real is begin return x * 2.0; end function;
  package p is new work.generic_map_signature_2019_gp
    generic map (f [real return real] => dbl, f [integer return integer] => inc);
  function ap generic (type t; function f (x : t) return t) parameter (x : t) return t is
  begin
    return f(x);
  end function;
  function api is new ap generic map (t => integer, f [t return t] => inc);
begin
  u : entity work.generic_map_signature_2019_sub
    generic map (g [integer return integer] => dbl);
  process
  begin
    wait for 1 ns;
    report "p " & integer'image(p.both(3)) & " api " & integer'image(api(1));
    assert p.both(3) = 10 and api(1) = 2 severity failure;
    wait;
  end process;
end architecture;
