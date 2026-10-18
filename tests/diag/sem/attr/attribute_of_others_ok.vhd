-- yavs-test: std=2008
-- expect: none
-- LRM 7.2
package p is
  attribute tag : integer;
  signal a, b : bit;
  attribute tag of others : signal is 1;
end package;
