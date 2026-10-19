-- (from the differential corpus)
-- ref=hand: the reference simulator crashes when lowering the call of a formal
-- subprogram reached through a formal package (`r.total` with the default "+");
-- the third reference simulator agrees with the expected output.
-- Nested generic packages: instances inside generic packages and their bodies,
-- using the enclosing package's generics, and a chain of formal packages.
package scale_g is
  generic (k : integer);
  function scale(x : integer) return integer;
  constant unit_k : integer := k;
end package;
package body scale_g is
  function scale(x : integer) return integer is
  begin
    return k * x;
  end function;
end package body;

-- Instances of a generic package inside a generic package, using its generics.
package outer_g is
  generic (k : integer);
  package twice is new work.scale_g generic map (k => 2 * k);
  function both(x : integer) return integer;
end package;
package body outer_g is
  package thrice is new work.scale_g generic map (k => 3 * k);
  function both(x : integer) return integer is
  begin
    return twice.scale(x) + thrice.scale(x) + twice.unit_k;
  end function;
end package body;

package outer_2 is new work.outer_g generic map (k => 2);
package outer_7 is new work.outer_g generic map (k => 7);

package ring_g is
  generic (type elem; zero : elem; function "+"(a, b : elem) return elem is <>);
  type arr is array (natural range <>) of elem;
  function total(x : arr) return elem;
end package;
package body ring_g is
  function total(x : arr) return elem is
    variable r : elem := zero;
  begin
    for i in x'range loop
      r := r + x(i);
    end loop;
    return r;
  end function;
end package body;

package stats_g is
  generic (package r is new work.ring_g generic map (<>));
  function twice(x : r.arr) return r.elem;
end package;
package body stats_g is
  use r.all;
  function twice(x : r.arr) return r.elem is
  begin
    return r.total(x) + r.total(x);
  end function;
end package body;

package int_ring is new work.ring_g generic map (elem => integer, zero => 0);
package int_stats is new work.stats_g generic map (r => work.int_ring);

entity nested_generic_packages is end entity;
architecture t of nested_generic_packages is
  package outer_local is new work.outer_g generic map (k => 11);
begin
  process
    constant v : work.int_ring.arr := (1, 2, 3, 4);
  begin
    report "outer_2=" & integer'image(work.outer_2.both(1))
      & " twice=" & integer'image(work.outer_2.twice.scale(10));
    report "outer_7=" & integer'image(work.outer_7.both(1))
      & " twice=" & integer'image(work.outer_7.twice.unit_k);
    report "outer_local=" & integer'image(outer_local.both(2))
      & " twice=" & integer'image(outer_local.twice.scale(1));
    report "total=" & integer'image(work.int_ring.total(v))
      & " twice=" & integer'image(work.int_stats.twice(v));
    wait;
  end process;
end architecture;
