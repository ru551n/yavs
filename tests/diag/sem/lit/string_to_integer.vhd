-- yavs-test: std=2008
-- expect: E1300 5:27 expected `integer`, found a string literal
-- LRM 15.7
package p is
  constant c : integer := "12";
end package;
