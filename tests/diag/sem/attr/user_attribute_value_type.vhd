-- yavs-test: std=2008
-- expect: E1300 7:34 expected `integer`, found a string literal
-- LRM 7.2
package p is
  attribute tag : integer;
  signal s : bit;
  attribute tag of s : signal is "x";
end package;
