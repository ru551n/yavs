-- yavs-test: std=2008
-- expect: E1201 6:14 `c` denotes a constant, not a type
-- LRM 6.3
package p is
  constant c : integer := 1;
  signal s : c;
end package;
