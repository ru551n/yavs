-- yavs-test: std=1993
-- expect: E1303 5:12 operator `and` cannot take 1 parameter
-- LRM 1993 7.2: the logical operators are binary; unary reductions are VHDL-2008 (#808)
package p is
  function "and" (a1 : real) return integer;
  function "or" (a1, a2 : real) return integer;
end package;
