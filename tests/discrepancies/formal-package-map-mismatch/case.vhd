-- yavs-test: std=2008
-- expect: E1503 14:43 `x` was not instantiated with the `n` the formal package requires
-- LRM 6.5.7.2
package a is
  generic (n : integer := 1; type t);
  constant c : integer := n;
  subtype s is t;
end package;
package b is
  generic (package p is new work.a generic map (n => 3, t => bit));
  constant k : integer := p.c;
end package;
package x is new work.a generic map (n => 4, t => bit);
package y is new work.b generic map (p => work.x);
