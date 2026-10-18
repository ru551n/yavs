-- yavs-test: std=2008
-- expect: E1101 5:20 enumeration literal `a` is declared twice
-- LRM 5.2.2.1
package p is
  type t is (a, b, a);
end package;
