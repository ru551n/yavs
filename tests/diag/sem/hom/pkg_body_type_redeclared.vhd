-- yavs-test: std=2008
-- expect: E1101 8:3 `t` is already declared as a declaration of package `p`
-- LRM 4.8, 12.1
package p is
  type t is (a, b);
end package;
package body p is
  type t is (c, d);
end package body;
