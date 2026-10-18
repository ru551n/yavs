-- yavs-test: std=2008
-- expect: E1100 6:17 `nothing` is not declared
-- LRM 4.5.3
package p is
  function f (x : integer) return integer;
  alias g is f [nothing return integer];
end package;
