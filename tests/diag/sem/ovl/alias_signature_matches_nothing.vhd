-- yavs-test: std=2008
-- expect: E1630 6:16 no overload of `f` has the signature [boolean return integer]
-- LRM 6.6.3
package p is
  function f (x : integer) return integer;
  alias g is f [boolean return integer];
end package;
