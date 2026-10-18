-- yavs-test: std=2008
-- expect: E1611 7:14 `a` is overloaded; the alias needs a signature
-- LRM 6.6.3
package p is
  type t1 is (a, b);
  type t2 is (a, c);
  alias x is a;
end package;
