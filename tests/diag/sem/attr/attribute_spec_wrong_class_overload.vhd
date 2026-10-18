-- rsim-test: std=2008
-- expect: E1611 8:20 `f` is overloaded; the attribute specification needs a signature
-- LRM 7.2
package p is
  attribute tag : integer;
  function f (x : integer) return integer;
  function f (x : bit) return integer;
  attribute tag of f : function is 1;
end package;
