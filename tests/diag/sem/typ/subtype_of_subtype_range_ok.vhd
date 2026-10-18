-- yavs-test: std=2008
-- expect: none
-- LRM 6.3
package p is
  subtype a is integer range 0 to 9;
  subtype b is a range 2 to 5;
end package;
