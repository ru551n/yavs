-- yavs-test: std=2008
-- expect: E1101 8:10 `s` is already declared as a declaration of package `p`
-- LRM 4.8, 12.1
package p is
  signal s : bit;
end package;
package body p is
  signal s : bit;
end package body;
