-- yavs-test: std=2008
-- expect: none
-- LRM 12.3
package p is
  type t is range 0 to 9;
  function "=" (a, b : t) return boolean;
end package;
