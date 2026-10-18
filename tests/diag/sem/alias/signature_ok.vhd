-- yavs-test: std=2008
-- expect: none
-- LRM 6.6.3
package p is
  function f (x : integer) return integer;
  function f (x : bit) return integer;
  alias g is f [integer return integer];
end package;
