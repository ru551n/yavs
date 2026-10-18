-- yavs-test: std=2008
-- expect: E1626 8:20 attribute `tag` is given to `s` twice
-- LRM 7.2
package p is
  signal s : bit;
  attribute tag : integer;
  attribute tag of s : signal is 1;
  attribute tag of s : signal is 2;
end package;
