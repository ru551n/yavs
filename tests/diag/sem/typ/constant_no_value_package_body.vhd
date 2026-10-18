-- yavs-test: std=2008
-- expect: E1224 7:12 constant `c` needs an initial value
-- LRM 6.4.2.2
package p is
end package;
package body p is
  constant c : integer;
end package body;
