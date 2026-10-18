-- yavs-test: std=2008
-- expect: E1402 5:27 literal `16#FFFFFFFFFFFFFFFFF#` is out of range
-- LRM 15.5
package p is
  constant c : integer := 16#FFFFFFFFFFFFFFFFF#;
end package;
