-- yavs-test: std=2019
-- expect: none
-- LRM 2019 9.1
package p is
  constant c : integer := 1 when true else 2;
end package;
