-- yavs-test: std=2019
-- expect: none
-- LRM 2019 5.7, 12.5 (UVVM rand_pkg: a user to_string through a use clause wins over the implicit INTEGER_VECTOR TO_STRING; sim1, sim2 and sim3 accept)
package sm is
  type t_radix is (DEC, HEX);
  impure function to_string (val : integer_vector; radix : t_radix := DEC) return string;
end package;
package body sm is
  impure function to_string (val : integer_vector; radix : t_radix := DEC) return string is
  begin
    return "x";
  end function;
end package body;
use work.sm.all;
package u is
  procedure p;
end package;
package body u is
  procedure p is
    constant v : integer_vector := (1, 2);
    constant s : string := to_string(v);
  begin
  end procedure;
end package body;
