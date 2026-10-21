-- yavs-test: std=2019
-- expect: none
-- LRM 2019 5.7, 12.5 (OSVVM RandomBasePkg: the explicit to_string wins over the implicit composite TO_STRING; sim1, sim2 and sim3 accept)
use std.textio.all;
package p is
  type rst is array (1 to 2) of integer;
  function to_string (A : rst; sep : string := " ") return string;
  procedure w (variable L : inout line; A : rst);
end package;
package body p is
  function to_string (A : rst; sep : string := " ") return string is
  begin
    return integer'image(A(1)) & sep & integer'image(A(2));
  end function;
  procedure w (variable L : inout line; A : rst) is
  begin
    write(L, to_string(A));
  end procedure;
end package body;
