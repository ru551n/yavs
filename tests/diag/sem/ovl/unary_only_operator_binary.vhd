-- yavs-test: std=2008
-- expect: E1303 5:12 operator `not` cannot take 2 parameters
-- LRM 4.5.2
package p is
  function "not" (a, b : integer) return integer;
end package;
