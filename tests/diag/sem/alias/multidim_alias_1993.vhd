-- yavs-test: std=1993
-- expect: E1007 8:22 an alias of a multidimensional array object is not allowed in VHDL-1993
-- LRM 1993 4.3.3.1: the type of an object alias is not a multidimensional array type (#814)
package p is
  type m is array (0 to 1, 0 to 1) of bit;
  constant c : m := (others => (others => '0'));
  alias row_ok : bit is c(0, 0);
  alias whole : m is c;
end package;
