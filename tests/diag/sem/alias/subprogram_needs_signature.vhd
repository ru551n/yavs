-- yavs-test: std=2008
-- expect: E1611 7:14 `f` is overloaded; the alias needs a signature
-- LRM 6.6.3
package p is
  function f (x : integer) return integer;
  function f (x : bit) return integer;
  alias g is f;
end package;
