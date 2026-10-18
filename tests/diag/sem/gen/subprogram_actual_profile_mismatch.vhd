-- yavs-test: std=2008
-- expect: E1300 10:43 profile of `h` does not match generic subprogram `f`
-- LRM 6.5.6.3
package g is
  generic (function f (a : integer) return integer);
end package;
package s is
  function h (a : boolean) return integer;
end package;
package i is new work.g generic map (f => work.s.h);
