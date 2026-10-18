-- yavs-test: std=2008
-- expect: none
-- LRM 13.2
package p is
  type boolean is (no, yes);
  constant b : boolean := yes;
end package;
