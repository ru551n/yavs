-- yavs-test: std=2008
-- expect: none
-- LRM 4.3 (a subprogram body in a package body completes the declaration in the package)
library ieee;
package p is
  function f (b : bit_vector) return integer;
end package;
package body p is
  function f (b : bit_vector) return integer is
  begin
    return ieee.numeric_bit.to_integer(ieee.numeric_bit.unsigned(b));
  end function;
end package body;
