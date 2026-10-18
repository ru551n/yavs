-- yavs-test: std=2008
-- expect: E1101 8:12 `c` is already declared as a declaration of package `p`
-- LRM 4.8, 12.1
package p is
  constant c : integer := 1;
end package;
package body p is
  constant c : integer := 1;
end package body;
