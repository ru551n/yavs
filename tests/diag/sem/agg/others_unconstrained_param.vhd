-- yavs-test: std=2008
-- expect: E1325 10:10 the target subtype of `v` is not constrained
-- LRM 9.3.3.3
package p is
  procedure clr (variable v : inout bit_vector);
end package;
package body p is
  procedure clr (variable v : inout bit_vector) is
  begin
    v := (others => '0');
  end procedure;
end package body;
