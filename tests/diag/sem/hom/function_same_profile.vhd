-- yavs-test: std=2008
-- expect: E1101 6:12 `f` with the same profile is already declared
-- LRM 4.5.3
package p is
  function f (x : integer) return integer;
  function f (y : integer) return integer;
end package;
