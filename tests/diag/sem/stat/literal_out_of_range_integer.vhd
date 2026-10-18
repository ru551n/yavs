-- yavs-test: std=2008
-- expect: E1402 5:27 literal `99999999999999999999` is out of range
-- LRM 15.5
package p is
  constant c : integer := 99999999999999999999;
end package;
