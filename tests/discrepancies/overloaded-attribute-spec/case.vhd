-- yavs-test: std=2008
-- expect: none
-- LRM 7.2 (an overloaded entity tag without a signature decorates every match)
package p is
  attribute tag : integer;
  function f (x : integer) return integer;
  function f (x : bit) return integer;
  attribute tag of f : function is 1;
end package;
