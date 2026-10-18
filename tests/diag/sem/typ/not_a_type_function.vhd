-- yavs-test: std=2008
-- expect: E1201 6:17 `f` denotes a function, not a type
-- LRM 6.3
package p is
  function f return integer;
  subtype st is f;
end package;
