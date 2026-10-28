-- yavs-test: std=2019
-- expect: none
-- LRM 2019 4.2.2.1: an impure function may have variable parameters (#808)
package p is
  impure function f (variable c : inout integer) return integer;
end package;
