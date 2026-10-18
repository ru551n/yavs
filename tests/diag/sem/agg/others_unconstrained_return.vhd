-- yavs-test: std=2008
-- expect: E1325 13:12 the result subtype `bit_vector` is not constrained
-- expect: E1325 18:10 the formal `x` is not constrained
-- LRM 9.3.3.3
package p is
  function f return bit_vector;
  procedure take (x : bit_vector);
  procedure call;
end package;
package body p is
  function f return bit_vector is
  begin
    return (others => '0');
  end function;
  procedure take (x : bit_vector) is begin end procedure;
  procedure call is
  begin
    take((others => '1'));
  end procedure;
end package body;
