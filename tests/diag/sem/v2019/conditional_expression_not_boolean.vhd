-- yavs-test: std=2019
-- expect: E1340 5:34 condition must be BOOLEAN, found `integer`
-- LRM 2019 9.1
package p is
  constant c : integer := 1 when 3 else 2;
end package;
