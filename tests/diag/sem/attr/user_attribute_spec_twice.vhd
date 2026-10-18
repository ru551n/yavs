-- rsim-test: std=2008
-- expect: E1101 8:20 attribute `tag` is already specified for `s`
-- LRM 7.2
package p is
  signal s : bit;
  attribute tag : integer;
  attribute tag of s : signal is 1;
  attribute tag of s : signal is 2;
end package;
