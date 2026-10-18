-- yavs-test: std=2008
-- expect: E1300 5:27 expected `integer`, found `universal_real`
-- LRM 9.3.2
package p is
  constant c : integer := 1.5;
end package;
