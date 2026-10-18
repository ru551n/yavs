-- yavs-test: std=2019
-- expect: E1300 5:44 expected `integer`, found `bit`
-- LRM 2019 9.1
package p is
  constant c : integer := 1 when true else '1';
end package;
