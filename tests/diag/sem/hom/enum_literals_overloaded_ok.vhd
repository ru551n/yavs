-- yavs-test: std=2008
-- expect: none
-- LRM 5.2.2.1
package p is
  type t1 is (a, b);
  type t2 is (b, c);
  constant k : t1 := a;
  constant m : t2 := c;
end package;
