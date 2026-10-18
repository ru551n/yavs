-- yavs-test: std=2008
-- expect: E1303 5:12 operator `*` cannot take 3 parameters
-- LRM 4.5.2
package p is
  function "*" (a, b, c : integer) return integer;
end package;
